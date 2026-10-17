use super::*;
use fuze_contract::titles::{regions, Atlas};
use std::sync::mpsc::Receiver;
use windows::Graphics::SizeInt32;

#[derive(Debug)]
struct OperationError {
    operation: &'static str,
    code: u32,
}
impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "capture-operation-failed")
    }
}
impl std::error::Error for OperationError {}
fn step<T>(operation: &'static str, value: windows::core::Result<T>) -> Result<T> {
    value.map_err(|e| {
        Box::new(OperationError {
            operation,
            code: e.code().0 as u32,
        }) as Box<dyn std::error::Error>
    })
}
pub fn failure_details(error: &(dyn std::error::Error + 'static)) -> serde_json::Value {
    if let Some(e) = error.downcast_ref::<OperationError>() {
        serde_json::json!({"operation":e.operation,"hresult":format!("0x{:08X}",e.code)})
    } else if let Some(e) = error.downcast_ref::<windows::core::Error>() {
        serde_json::json!({"operation":"native_setup","hresult":format!("0x{:08X}",e.code().0 as u32)})
    } else {
        serde_json::json!({"operation":"contract_validation"})
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameStatus {
    Ready,
    Timeout,
    Resized,
    Duplicate,
    Busy,
    Suspended,
}
struct Frame(Direct3D11CaptureFrame);
impl Drop for Frame {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}

/// Serialized, bounded stream. Foreign windows require an opaque validated GameWindow.
/// Visible self-window testing is allowed only by an explicit CI mode.
pub struct OwnedWindowStream {
    target: Option<crate::target::GameWindow>,
    capture: Capture,
    reader: SelectionReader,
    winrt: IDirect3DDevice,
    device: Device,
    receive: Receiver<()>,
    size: SizeInt32,
    handle: isize,
    monitor: bool,
    pub generation: u64,
    pub sequence: u64,
    last_timestamp: Option<i64>,
    ready: bool,
    _apartment: Apartment,
}
impl OwnedWindowStream {
    pub fn start(handle: isize, visible_ci: bool) -> Result<Self> {
        let hwnd = HWND(handle as *mut _);
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let visible = IsWindowVisible(hwnd).as_bool();
            let ci = std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                && std::env::var("FUZE_RUST_CI_VISIBLE").as_deref() == Ok("1");
            if pid != GetCurrentProcessId() || (visible && !(visible_ci && ci)) {
                return Err("window-ownership-or-visibility".into());
            }
        }
        Self::start_validated(handle, None, false)
    }
    pub fn start_game(target: crate::target::GameWindow) -> Result<Self> {
        if !target.valid() || target.minimized() {
            return Err("game-window-unavailable".into());
        }
        Self::start_validated(target.handle(), Some(target), false)
    }
    pub fn start_primary_screen() -> Result<Self> {
        use windows::Win32::{Foundation::POINT, Graphics::Gdi::*};
        let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        if monitor.is_invalid() {
            return Err("capture-monitor".into());
        }
        Self::start_validated(monitor.0 as isize, None, true)
    }
    fn start_validated(
        handle: isize,
        target: Option<crate::target::GameWindow>,
        monitor: bool,
    ) -> Result<Self> {
        let apartment = Apartment::new()?;
        let hwnd = HWND(handle as *mut _);
        if !step("is_supported", GraphicsCaptureSession::IsSupported())? {
            return Err("wgc-unsupported".into());
        }
        let device = Device::new()?;
        let winrt = device.winrt()?;
        let interop: IGraphicsCaptureItemInterop = step(
            "activation_factory",
            factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>(),
        )?;
        let item: GraphicsCaptureItem = unsafe {
            if monitor {
                step(
                    "create_monitor_item",
                    interop.CreateForMonitor(windows::Win32::Graphics::Gdi::HMONITOR(
                        handle as *mut _,
                    )),
                )?
            } else {
                step("create_owned_window_item", interop.CreateForWindow(hwnd))?
            }
        };
        let size = step("item_size", item.Size())?;
        regions(size.Width as u32, size.Height as u32)?;
        let pool = step(
            "create_frame_pool",
            Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            ),
        )?;
        let session = match step("create_session", pool.CreateCaptureSession(&item)) {
            Ok(s) => s,
            Err(e) => {
                let _ = pool.Close();
                return Err(e.into());
            }
        };
        let mut capture = Capture {
            pool,
            session,
            token: None,
        };
        step(
            "disable_cursor",
            capture.session.SetIsCursorCaptureEnabled(false),
        )?;
        // Optional on older Windows: host sampling is bounded even if the OS lacks this API.
        if monitor {
            let _ = capture
                .session
                .SetMinUpdateInterval(windows::Foundation::TimeSpan {
                    Duration: 5_000_000,
                });
        }
        let (send, receive) = sync_channel(1);
        capture.token = Some(
            capture
                .pool
                .FrameArrived(&TypedEventHandler::new(move |_, _| {
                    let _ = send.try_send(());
                    Ok(())
                }))?,
        );
        step("start_capture", capture.session.StartCapture())?;
        Ok(Self {
            target,
            capture,
            reader: SelectionReader::new(&device),
            winrt,
            device,
            receive,
            size,
            handle,
            monitor,
            generation: 1,
            sequence: 0,
            last_timestamp: None,
            ready: false,
            _apartment: apartment,
        })
    }
    pub fn next(&mut self, timeout: Duration) -> Result<FrameStatus> {
        self.ready = false;
        if let Some(target) = &self.target {
            if !target.valid() {
                return Err("source-window-closed".into());
            }
            if target.minimized() || !target.foreground() {
                return Ok(FrameStatus::Suspended);
            }
        }
        // Ownership is rechecked before consuming frames if the source HWND was destroyed/reused.
        if !self.monitor {
            unsafe {
                let mut pid = 0;
                GetWindowThreadProcessId(HWND(self.handle as *mut _), Some(&mut pid));
                if self.target.is_none() && pid != GetCurrentProcessId() {
                    return Err("source-window-closed".into());
                }
            }
        }
        if self
            .receive
            .recv_timeout(timeout.min(Duration::from_millis(750)))
            .is_err()
        {
            return Ok(FrameStatus::Timeout);
        }
        let mut frame = match self.capture.pool.TryGetNextFrame() {
            Ok(frame) => Frame(frame),
            Err(error)
                if error.code() == windows::Win32::Foundation::E_POINTER
                    || error.code() == windows::Win32::Foundation::E_BOUNDS =>
            {
                return Ok(FrameStatus::Timeout)
            }
            Err(error) => return Err(error.into()),
        };
        if let Ok(newer) = self.capture.pool.TryGetNextFrame() {
            frame = Frame(newer);
        }
        let size = frame.0.ContentSize()?;
        let rects = regions(size.Width as u32, size.Height as u32)?;
        if size != self.size {
            drop(frame); // No old frame may remain checked out during Recreate.
            self.capture.pool.Recreate(
                &self.winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )?;
            self.size = size;
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("capture-generation-overflow")?;
            self.last_timestamp = None;
            self.reader = SelectionReader::new(&self.device);
            return Ok(FrameStatus::Resized);
        }
        let stamp = frame.0.SystemRelativeTime()?.Duration;
        if self.last_timestamp.is_some_and(|last| stamp <= last) {
            return Ok(FrameStatus::Duplicate);
        }
        let access: IDirect3DDxgiInterfaceAccess = frame.0.Surface()?.cast()?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
        if let Err(error) =
            self.reader
                .read(&texture, &rects, (size.Width as u32, size.Height as u32))
        {
            if error.is::<ReadbackBusy>() {
                self.last_timestamp = Some(stamp);
                return Ok(FrameStatus::Busy);
            }
            return Err(error);
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("capture-sequence-overflow")?;
        self.last_timestamp = Some(stamp);
        self.ready = true;
        Ok(FrameStatus::Ready)
    }
    pub fn titles(&self) -> Result<Atlas<'_>> {
        if !self.ready {
            return Err("no-current-frame".into());
        }
        let rects = regions(self.size.Width as u32, self.size.Height as u32)?;
        Ok(Atlas::new(
            &self.reader.bytes,
            rects[0].width,
            rects[0].height,
        )?)
    }
}

impl CaptureFeed for OwnedWindowStream {
    fn selection(&self) -> Option<fuze_contract::selection_pixels::Evidence> {
        self.reader.evidence
    }
    fn next(&mut self, timeout: Duration) -> Result<FrameStatus> {
        Self::next(self, timeout)
    }
    fn titles(&self) -> Result<Atlas<'_>> {
        Self::titles(self)
    }
    fn key(&self) -> (u64, u64) {
        (self.generation, self.sequence)
    }
    fn source(&self) -> &'static str {
        if self.monitor {
            "screen-stream"
        } else {
            "window-stream"
        }
    }
}
