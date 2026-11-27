//! Reusable GDI title atlas; no PNG, full-frame CPU allocation, or disk writes.
use crate::{
    native::Result,
    pick_pixels::{PickPixels, PickProbe},
    selection_frame::{CaptureRegions, SelectionFrame},
};
use fuze_contract::titles::{Atlas, Region};
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::HiDpi::*};

pub(super) struct PhysicalPixels(DPI_AWARENESS_CONTEXT);
impl PhysicalPixels {
    pub fn new() -> Self {
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}
impl Drop for PhysicalPixels {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}
struct DisplayDc(HDC);
impl Drop for DisplayDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(null_mut(), self.0);
        }
    }
}
struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u8,
}
impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
impl Dib {
    fn new(source: HDC, width: u32, height: u32) -> Result<Self> {
        unsafe {
            let dc = CreateCompatibleDC(source);
            if dc.is_null() {
                return Err("capture-bitmap-dc".into());
            }
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..zeroed()
            };
            let mut pixels = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
            if bitmap.is_null() || pixels.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(dc);
                return Err("capture-bitmap-allocation".into());
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() || previous as isize == -1 {
                DeleteObject(bitmap);
                DeleteDC(dc);
                return Err("capture-bitmap-select".into());
            }
            Ok(Self {
                dc,
                bitmap,
                previous,
                pixels: pixels.cast(),
            })
        }
    }
    fn copy(&self, source: HDC, monitor: RECT, x: u32, area: Region) -> bool {
        unsafe {
            BitBlt(
                self.dc,
                x as i32,
                0,
                area.width as i32,
                area.height as i32,
                source,
                monitor.left + area.x as i32,
                monitor.top + area.y as i32,
                SRCCOPY | CAPTUREBLT,
            ) != 0
        }
    }
}
#[derive(Default)]
pub(super) struct Reader {
    dib: Option<Dib>,
    size: (u32, u32),
    pub frame: SelectionFrame,
    pick_dib: Option<Dib>,
    pick_size: (u32, u32),
    pick_regions: Option<[Region; 2]>,
    screen: (u32, u32),
}
impl Reader {
    pub fn read(&mut self, active_layout: fuze_contract::Layout, pick: PickProbe) -> Result<()> {
        self.pick_regions = None;
        unsafe {
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if monitor.is_null() || GetMonitorInfoW(monitor, &mut info) == 0 {
                return Err("capture-monitor".into());
            }
            let rect = info.rcMonitor;
            self.screen = (
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
            );
            let regions = CaptureRegions::new(self.screen)?;
            let bounds = regions.bitmap_bounds();
            let size = (bounds.width, bounds.height);
            let dc = DisplayDc(GetDC(null_mut()));
            if dc.0.is_null() {
                return Err("capture-bitmap-source".into());
            }
            if self.size != size || self.dib.is_none() {
                self.dib = Some(Dib::new(dc.0, size.0, size.1)?);
                self.size = size;
            }
            let dib = self.dib.as_ref().ok_or("capture-bitmap-missing")?;
            if !dib.copy(dc.0, rect, 0, bounds) {
                return Err("capture-bitmap-copy".into());
            }
            if GdiFlush() == 0 {
                return Err("capture-bitmap-flush".into());
            }
            let stride = size.0 as usize * 4;
            self.frame.read(&regions, active_layout, |area, y| {
                let r = regions.areas[area];
                let x = (r.x - bounds.x) as usize * 4;
                let top = (r.y - bounds.y) as usize;
                std::slice::from_raw_parts(
                    dib.pixels.add((top + y) * stride + x),
                    r.width as usize * 4,
                )
            })?;
            if let Some([hud, cards]) = pick.regions(self.frame.evidence, self.screen) {
                let size = (hud.width + cards.width, hud.height.max(cards.height));
                if self.pick_size != size || self.pick_dib.is_none() {
                    self.pick_dib = Dib::new(dc.0, size.0, size.1).ok();
                    if let Some(dib) = &self.pick_dib {
                        // The shorter strip leaves a few rows untouched by BitBlt.
                        std::ptr::write_bytes(dib.pixels, 0, (size.0 * size.1 * 4) as usize);
                        self.pick_size = size;
                    }
                }
                if let Some(dib) = &self.pick_dib {
                    // Always flush the two copies, including a partially failed probe.
                    let hud_copied = dib.copy(dc.0, rect, 0, hud);
                    let cards_copied = dib.copy(dc.0, rect, hud.width, cards);
                    if GdiFlush() != 0 && hud_copied && cards_copied {
                        self.pick_regions = Some([hud, cards]);
                    }
                }
            }
            Ok(())
        }
    }
    pub fn titles(&self) -> Result<Atlas<'_>> {
        self.frame.titles()
    }
    pub fn diagnostic_pixels(&self) -> Option<crate::native::DiagnosticPixels<'_>> {
        self.frame.diagnostic_pixels()
    }

    pub fn pick_pixels(&self) -> Option<PickPixels<'_>> {
        let dib = self.pick_dib.as_ref()?;
        let regions = self.pick_regions?;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                dib.pixels,
                (self.pick_size.0 * self.pick_size.1 * 4) as usize,
            )
        };
        PickPixels::packed(bytes, regions, self.screen)
    }
}
