//! Reusable GDI title atlas; no PNG, full-frame CPU allocation, or disk writes.
use crate::native::Result;
use fuze_contract::titles::{regions, Atlas};
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
}
#[derive(Default)]
pub(super) struct Reader {
    dib: Option<Dib>,
    size: (u32, u32, u32, u32),
    bytes: Vec<u8>,
    controls: Vec<u8>,
    pub evidence: Option<fuze_contract::selection_pixels::Evidence>,
    screen: (u32, u32),
}
impl Reader {
    pub fn read(&mut self) -> Result<()> {
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
            let slots = regions(
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
            )?;
            let buttons = fuze_contract::selection_pixels::buttons(
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
            )?;
            let size = (
                slots[0].width,
                slots[0].height,
                slots[2].x + slots[2].width - slots[0].x,
                buttons[0].y + buttons[0].height - slots[0].y,
            );
            let dc = DisplayDc(GetDC(null_mut()));
            if dc.0.is_null() {
                return Err("capture-bitmap-source".into());
            }
            if self.size != size || self.dib.is_none() {
                self.dib = Some(Dib::new(dc.0, size.2, size.3)?);
                self.size = size;
                self.bytes.resize((size.0 * size.1 * 12) as usize, 0);
            }
            let dib = self.dib.as_ref().ok_or("capture-bitmap-missing")?;
            // One coherent band contains titles and reroll controls. No second screenshot,
            // encoder, full-screen CPU buffer or image file is required for the UI gate.
            if BitBlt(
                dib.dc,
                0,
                0,
                size.2 as i32,
                size.3 as i32,
                dc.0,
                rect.left + slots[0].x as i32,
                rect.top + slots[0].y as i32,
                SRCCOPY | CAPTUREBLT,
            ) == 0
            {
                return Err("capture-bitmap-copy".into());
            }
            if GdiFlush() == 0 {
                return Err("capture-bitmap-flush".into());
            }
            let source_stride = size.2 as usize * 4;
            let title_bytes = size.0 as usize * 4;
            for (i, slot) in slots.iter().enumerate() {
                let x = (slot.x - slots[0].x) as usize * 4;
                for y in 0..size.1 as usize {
                    let start = y * title_bytes * 3 + i * title_bytes;
                    self.bytes[start..start + title_bytes].copy_from_slice(
                        std::slice::from_raw_parts(
                            dib.pixels.add(y * source_stride + x),
                            title_bytes,
                        ),
                    );
                }
            }
            let button_bytes = buttons[0].width as usize * 4;
            self.controls
                .resize(button_bytes * 3 * buttons[0].height as usize, 0);
            for (i, slot) in buttons.iter().enumerate() {
                let x = (slot.x - slots[0].x) as usize * 4;
                let top = (slot.y - slots[0].y) as usize;
                for y in 0..slot.height as usize {
                    let start = (y * 3 + i) * button_bytes;
                    self.controls[start..start + button_bytes].copy_from_slice(
                        std::slice::from_raw_parts(
                            dib.pixels.add((top + y) * source_stride + x),
                            button_bytes,
                        ),
                    );
                }
            }
            self.evidence = Some(fuze_contract::selection_pixels::classify(
                &self.titles()?,
                &Atlas::new(&self.controls, buttons[0].width, buttons[0].height)?,
            ));
            Ok(())
        }
    }
    pub fn titles(&self) -> Result<Atlas<'_>> {
        Ok(Atlas::new(&self.bytes, self.size.0, self.size.1)?)
    }
    pub fn diagnostic_pixels(&self) -> Option<crate::native::DiagnosticPixels<'_>> {
        let buttons =
            fuze_contract::selection_pixels::buttons(self.screen.0, self.screen.1).ok()?;
        Some(crate::native::DiagnosticPixels {
            buttons: Atlas::new(&self.controls, buttons[0].width, buttons[0].height).ok()?,
            screen: self.screen,
        })
    }
}
