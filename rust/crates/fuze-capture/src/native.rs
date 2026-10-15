//! Bounded WGC for owned test windows or validated game targets, plus GPU ROI readback.
//! No capture picker, desktop capture, image encoder or input simulation.
use std::{
    sync::mpsc::sync_channel,
    time::{Duration, Instant},
};
use windows::{
    core::{factory, Interface},
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
        System::{
            Threading::GetCurrentProcessId,
            WinRT::{
                Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop, RoInitialize,
                RoUninitialize, RO_INIT_MULTITHREADED,
            },
        },
        UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible},
    },
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
pub struct ReadbackBusy;
impl std::fmt::Display for ReadbackBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("gpu-readback-busy")
    }
}
impl std::error::Error for ReadbackBusy {}

#[path = "fixture.rs"]
mod fixture;
pub use fixture::{GpuFixture, GpuFixtureReader};
#[path = "stream.rs"]
mod stream;
pub use stream::{failure_details, FrameStatus, OwnedWindowStream};

struct Apartment(std::marker::PhantomData<std::rc::Rc<()>>);
impl Apartment {
    fn new() -> Result<Self> {
        pin_capture_runtime()?;
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        Ok(Self(std::marker::PhantomData))
    }
}

// WGC can retain internal callbacks after Close/RoUninitialize on affected Windows builds.
// Keep only the system DLL code mapped for this process lifetime; frames/devices still close.
// Once per process, not a per-session LoadLibrary leak or a timing-dependent shutdown sleep.
fn pin_capture_runtime() -> Result<()> {
    use windows::{
        core::w,
        Win32::{Foundation::FreeLibrary, System::LibraryLoader::*},
    };
    static PIN: std::sync::OnceLock<std::result::Result<(), &'static str>> =
        std::sync::OnceLock::new();
    let result = PIN.get_or_init(|| unsafe {
        let loaded = LoadLibraryExW(
            w!("GraphicsCapture.dll"),
            None,
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
        .map_err(|_| "capture-runtime-load")?;
        let mut pinned = HMODULE::default();
        let result = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_PIN,
            w!("GraphicsCapture.dll"),
            &mut pinned,
        )
        .map_err(|_| "capture-runtime-pin");
        let _ = FreeLibrary(loaded);
        result
    });
    result.map_err(|e| e.into())
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() }
    }
}

struct Device {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}
impl Device {
    fn new() -> Result<Self> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let adapter = factory.EnumAdapters(0)?;
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            Ok(Self {
                device: device.ok_or("device-missing")?,
                context: context.ok_or("context-missing")?,
            })
        }
    }
    fn winrt(&self) -> Result<IDirect3DDevice> {
        unsafe {
            Ok(
                CreateDirect3D11DeviceFromDXGIDevice(&self.device.cast::<IDXGIDevice>()?)?
                    .cast()?,
            )
        }
    }
}

pub use fuze_contract::titles::Region;
fn validate_regions(width: u32, height: u32, regions: &[Region; 3]) -> Result<(u32, u32)> {
    if width == 0 || height == 0 || width > 4096 || height > 2160 {
        return Err("frame-size".into());
    }
    let mut atlas_width = 0u32;
    let mut atlas_height = 0;
    for region in regions {
        if region.width == 0
            || region.height == 0
            || region.x.checked_add(region.width).is_none_or(|v| v > width)
            || region
                .y
                .checked_add(region.height)
                .is_none_or(|v| v > height)
        {
            return Err("roi-bounds".into());
        }
        atlas_width = atlas_width
            .checked_add(region.width)
            .ok_or("atlas-overflow")?;
        atlas_height = atlas_height.max(region.height);
    }
    if atlas_width > 4096 || atlas_height > 256 {
        return Err("roi-budget".into());
    }
    Ok((atlas_width, atlas_height))
}

/// Reuses one staging atlas and one CPU allocation. Copies only the three title ROIs.
pub struct RoiReader {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    staging: Option<ID3D11Texture2D>,
    size: (u32, u32),
    pub bytes: Vec<u8>,
}
impl RoiReader {
    fn new(device: &Device) -> Self {
        Self {
            device: device.device.clone(),
            context: device.context.clone(),
            staging: None,
            size: (0, 0),
            bytes: Vec::new(),
        }
    }
    fn read(&mut self, source: &ID3D11Texture2D, regions: &[Region; 3]) -> Result<()> {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut desc);
            if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM || desc.SampleDesc.Count != 1 {
                return Err("frame-format".into());
            }
            let size = validate_regions(desc.Width, desc.Height, regions)?;
            if self.size != size {
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: size.0,
                    Height: size.1,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    ..Default::default()
                };
                let mut staging = None;
                self.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
                self.staging = staging;
                self.size = size;
                self.bytes.resize((size.0 * size.1 * 4) as usize, 0);
            }
            let staging = self.staging.as_ref().ok_or("staging-missing")?;
            let mut x = 0;
            for region in regions {
                let rect = D3D11_BOX {
                    left: region.x,
                    top: region.y,
                    right: region.x + region.width,
                    bottom: region.y + region.height,
                    front: 0,
                    back: 1,
                };
                self.context
                    .CopySubresourceRegion(staging, 0, x, 0, 0, source, 0, Some(&rect));
                x += region.width;
            }
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            // Submit once, then wait briefly without blocking inside Map or busy-spinning.
            self.context.Flush();
            let deadline = Instant::now() + Duration::from_millis(20);
            loop {
                match self.context.Map(
                    staging,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                ) {
                    Ok(()) => break,
                    Err(error) if error.code() == DXGI_ERROR_WAS_STILL_DRAWING => {
                        if Instant::now() >= deadline {
                            return Err(Box::new(ReadbackBusy));
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            // Map success always reaches Unmap, including invalid-stride handling.
            let stride = size.0 as usize * 4;
            let valid = !mapped.pData.is_null() && mapped.RowPitch as usize >= stride;
            if valid {
                // Copy only initialized ROI rows; no uninitialized atlas padding is exposed.
                self.bytes.fill(0);
                let mut x = 0usize;
                for region in regions {
                    for y in 0..region.height as usize {
                        let src = std::slice::from_raw_parts(
                            (mapped.pData as *const u8).add(y * mapped.RowPitch as usize + x * 4),
                            region.width as usize * 4,
                        );
                        let start = y * stride + x * 4;
                        self.bytes[start..start + src.len()].copy_from_slice(src);
                    }
                    x += region.width as usize;
                }
            }
            self.context.Unmap(staging, 0);
            if !valid {
                return Err("mapped-stride".into());
            }
            Ok(())
        }
    }
}

struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    token: Option<i64>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            let _ = self.pool.RemoveFrameArrived(token);
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// Caller must own this hidden HWND. The function checks ownership before creating a WGC item.
/// No raw frame leaves this probe. A hidden window may legitimately produce no frames.
pub fn probe_hidden_own_window(handle: isize) -> serde_json::Value {
    fn run(handle: isize, phase: &std::cell::Cell<&'static str>) -> Result<serde_json::Value> {
        let _apartment = Apartment::new()?;
        phase.set("window_ownership");
        let hwnd = HWND(handle as *mut _);
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != GetCurrentProcessId() || IsWindowVisible(hwnd).as_bool() {
                return Err("not-hidden-owned-window".into());
            }
        }
        phase.set("wgc_support");
        if !GraphicsCaptureSession::IsSupported()? {
            return Ok(serde_json::json!({"status":"unsupported"}));
        }
        phase.set("d3d_device");
        let device = Device::new()?;
        phase.set("capture_interop");
        let interop: IGraphicsCaptureItemInterop =
            factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        phase.set("create_hidden_window_item");
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        phase.set("item_size");
        let size = item.Size()?;
        if size.Width <= 0 || size.Height <= 0 || size.Width > 4096 || size.Height > 2160 {
            return Err("item-size".into());
        }
        phase.set("frame_pool");
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &device.winrt()?,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        phase.set("capture_session");
        let session = match pool.CreateCaptureSession(&item) {
            Ok(value) => value,
            Err(error) => {
                let _ = pool.Close();
                return Err(error.into());
            }
        };
        let mut capture = Capture {
            pool,
            session,
            token: None,
        };
        phase.set("cursor_disable");
        capture.session.SetIsCursorCaptureEnabled(false)?;
        let (send, receive) = sync_channel::<()>(1);
        phase.set("frame_handler");
        capture.token = Some(
            capture
                .pool
                .FrameArrived(&TypedEventHandler::new(move |_, _| {
                    let _ = send.try_send(());
                    Ok(())
                }))?,
        );
        let started = Instant::now();
        phase.set("start_capture");
        capture.session.StartCapture()?;
        if receive.recv_timeout(Duration::from_millis(750)).is_err() {
            return Ok(
                serde_json::json!({"status":"no_frame_hidden_window","elapsed_ms":started.elapsed().as_secs_f64()*1000.,"buffers":2,"notification_capacity":1}),
            );
        }
        phase.set("read_frame");
        let mut frame = capture.pool.TryGetNextFrame()?;
        // Bounded drain: prefer latest frame without accumulating a FIFO of stale work.
        if let Ok(newer) = capture.pool.TryGetNextFrame() {
            frame.Close()?;
            frame = newer;
        }
        let result = (|| -> Result<serde_json::Value> {
            let content = frame.ContentSize()?;
            if content != size {
                return Ok(serde_json::json!({"status":"resize_requires_recreate"}));
            }
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
            let regions = [
                Region {
                    x: 0,
                    y: 0,
                    width: 16,
                    height: 16,
                },
                Region {
                    x: 16,
                    y: 0,
                    width: 16,
                    height: 16,
                },
                Region {
                    x: 32,
                    y: 0,
                    width: 16,
                    height: 16,
                },
            ];
            let mut reader = RoiReader::new(&device);
            reader.read(&texture, &regions)?;
            Ok(
                serde_json::json!({"status":"own_frame_read","roi_bytes":reader.bytes.len(),"encoded_images":0,"elapsed_ms":started.elapsed().as_secs_f64()*1000.}),
            )
        })();
        let _ = frame.Close();
        result
    }
    let phase = std::cell::Cell::new("winrt_initialize");
    run(handle, &phase)
        .unwrap_or_else(|_| serde_json::json!({"status":"unavailable","phase":phase.get()}))
}

/// GPU texture created entirely from generated pixels. Does not capture any window or screen.
pub fn synthetic_readback() -> Result<serde_json::Value> {
    let device = Device::new()?;
    let (width, height) = (1920u32, 1080u32);
    let pixels: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).flat_map(move |x| [x as u8, y as u8, 73, 255]))
        .collect();
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        ..Default::default()
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: width * 4,
        ..Default::default()
    };
    let mut texture = None;
    unsafe {
        device
            .device
            .CreateTexture2D(&desc, Some(&initial), Some(&mut texture))?;
    }
    let texture = texture.ok_or("texture-missing")?;
    let regions = [
        Region {
            x: 468,
            y: 416,
            width: 271,
            height: 32,
        },
        Region {
            x: 824,
            y: 416,
            width: 271,
            height: 32,
        },
        Region {
            x: 1181,
            y: 416,
            width: 271,
            height: 32,
        },
    ];
    let mut reader = RoiReader::new(&device);
    let mut times = Vec::new();
    for _ in 0..4 {
        let start = Instant::now();
        reader.read(&texture, &regions)?;
        times.push(start.elapsed().as_secs_f64() * 1000.);
        for (i, region) in regions.iter().enumerate() {
            for y in 0..32usize {
                for x in 0..271usize {
                    let offset = (y * 813 + i * 271 + x) * 4;
                    if reader.bytes[offset..offset + 4]
                        != [
                            (region.x + x as u32) as u8,
                            (region.y + y as u32) as u8,
                            73,
                            255,
                        ]
                    {
                        return Err("readback-mismatch".into());
                    }
                }
            }
        }
    }
    Ok(
        serde_json::json!({"passed":true,"source":"generated_gpu_texture","runs_ms":times,"roi_bytes":reader.bytes.len(),"full_frame_bytes":pixels.len(),"maps_per_frame":1,"staging_textures":1,"encoded_images":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_roi_before_gpu_work() {
        let valid = Region {
            x: 1,
            y: 1,
            width: 16,
            height: 16,
        };
        assert_eq!(validate_regions(1920, 1080, &[valid; 3]).unwrap(), (48, 16));
        assert!(validate_regions(
            1920,
            1080,
            &[Region {
                x: u32::MAX,
                ..valid
            }; 3]
        )
        .is_err());
        assert!(validate_regions(
            1920,
            1080,
            &[Region {
                height: 300,
                ..valid
            }; 3]
        )
        .is_err());
    }
}
