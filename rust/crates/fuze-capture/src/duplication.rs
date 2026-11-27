//! GPU desktop duplication, without WGC's capture indicator or a permission dialog.
//! Only selection and pick ROIs are copied to CPU memory.
use super::*;
use crate::pick_pixels::{PickPixels, PickProbe};
use fuze_contract::titles::{regions, Atlas};

pub struct DesktopStream {
    duplication: IDXGIOutputDuplication,
    reader: SelectionReader,
    pick_reader: RoiReader,
    pick_regions: Option<[Region; 2]>,
    size: (u32, u32),
    ready: bool,
}
struct FrameLease<'a>(&'a IDXGIOutputDuplication);
impl Drop for FrameLease<'_> {
    fn drop(&mut self) {
        // Release even on a failed cast/readback, otherwise the next acquire is invalid.
        let _ = unsafe { self.0.ReleaseFrame() };
    }
}
impl DesktopStream {
    pub fn start_primary_screen() -> Result<Self> {
        use windows::Win32::{Foundation::POINT, Graphics::Gdi::*};
        unsafe {
            let primary = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            // A laptop's primary output need not belong to adapter zero.
            for a in 0..16 {
                let adapter = match factory.EnumAdapters(a) {
                    Ok(adapter) => adapter,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return Err(e.into()),
                };
                for o in 0..32 {
                    let output = match adapter.EnumOutputs(o) {
                        Ok(output) => output,
                        Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                        Err(e) => return Err(e.into()),
                    };
                    let desc = output.GetDesc()?;
                    if desc.Monitor != primary || !desc.AttachedToDesktop.as_bool() {
                        continue;
                    }
                    // Rotated desktops need a transform; the established bitmap path handles it.
                    if desc.Rotation != DXGI_MODE_ROTATION_IDENTITY {
                        return Err("desktop-rotation".into());
                    }
                    let size = (
                        (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left) as u32,
                        (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top) as u32,
                    );
                    regions(size.0, size.1)?;
                    let device = Device::on_adapter(&adapter)?;
                    let duplication = output
                        .cast::<IDXGIOutput1>()?
                        .DuplicateOutput(&device.device)?;
                    return Ok(Self {
                        duplication,
                        reader: SelectionReader::new(&device),
                        pick_reader: RoiReader::new(&device),
                        pick_regions: None,
                        size,
                        ready: false,
                    });
                }
            }
        }
        Err("desktop-output-missing".into())
    }
    pub fn next(
        &mut self,
        timeout: Duration,
        active_layout: fuze_contract::Layout,
        pick: PickProbe,
    ) -> Result<FrameStatus> {
        self.ready = false;
        self.pick_regions = None;
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        unsafe {
            match self.duplication.AcquireNextFrame(
                timeout.as_millis().min(750) as u32,
                &mut info,
                &mut resource,
            ) {
                Ok(()) => (),
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(FrameStatus::Timeout),
                // Access lost / mode changes are recovered by ScreenCapture's bounded fallback.
                Err(e) => return Err(e.into()),
            }
        }
        let _lease = FrameLease(&self.duplication);
        // Consume only the pixels supplied by Windows (protected content stays masked).
        // A protected/excluded window elsewhere on the desktop must not disable all three ROIs.
        let texture: ID3D11Texture2D = resource.ok_or("desktop-frame-missing")?.cast()?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        if (desc.Width, desc.Height) != self.size {
            return Err("desktop-size-changed".into());
        }
        match self.reader.read(&texture, self.size, active_layout) {
            Ok(()) => (),
            Err(e) if e.is::<ReadbackBusy>() => return Ok(FrameStatus::Busy),
            Err(e) => return Err(e),
        }
        if let Some(regions) = pick.regions(self.selection(), self.size) {
            // The supplemental data comes from the same DXGI frame lease. A busy
            // supplemental readback must not invalidate the title frame.
            if self.pick_reader.read(&texture, &regions).is_ok() {
                self.pick_regions = Some(regions);
            }
        }
        self.ready = true;
        Ok(FrameStatus::Ready)
    }
    pub fn selection(&self) -> Option<fuze_contract::selection_pixels::Evidence> {
        self.reader.frame.evidence
    }
    pub fn pick_pixels(&self) -> Option<PickPixels<'_>> {
        PickPixels::packed(&self.pick_reader.bytes, self.pick_regions?, self.size)
    }
    pub fn diagnostic_pixels(&self) -> Option<DiagnosticPixels<'_>> {
        if !self.ready {
            return None;
        }
        self.reader.frame.diagnostic_pixels()
    }
    pub fn titles(&self) -> Result<Atlas<'_>> {
        if !self.ready {
            return Err("no-current-frame".into());
        }
        self.reader.frame.titles()
    }
}
