//! Rounded outer bounds for owned HWNDs only. No transparency, extra window or repaint loop.
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{
        Dwm::{
            DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWMWCP_ROUNDSMALL,
        },
        Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn},
    },
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{GetWindowRect, IsIconic, IsZoomed},
    },
};

pub struct Corners {
    native: bool,
    radius: i32,
    last: Option<(i32, i32, u32, bool)>,
}
impl Corners {
    pub fn for_window(window: &slint::Window, small: bool) -> Option<Self> {
        crate::lifecycle::hwnd(window).map(|handle| Self::new(handle, small))
    }
    pub fn new(hwnd: HWND, small: bool) -> Self {
        let preference = if small {
            DWMWCP_ROUNDSMALL
        } else {
            DWMWCP_ROUND
        };
        let native = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&preference as *const i32).cast(),
                std::mem::size_of_val(&preference) as u32,
            ) >= 0
        };
        let mut result = Self {
            native,
            radius: if small { 4 } else { 8 },
            last: None,
        };
        result.resize(hwnd);
        result
    }
    pub fn resize(&mut self, hwnd: HWND) {
        // DWM owns its geometry. The region fallback is updated only on resize/DPI events.
        if self.native {
            return;
        }
        unsafe {
            if IsIconic(hwnd) != 0 {
                return;
            }
            let mut bounds = RECT::default();
            if GetWindowRect(hwnd, &mut bounds) == 0 {
                return;
            }
            let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
            if width <= 0 || height <= 0 {
                return;
            }
            let dpi = GetDpiForWindow(hwnd).max(96);
            let maximized = IsZoomed(hwnd) != 0;
            let key = (width, height, dpi, maximized);
            if self.last == Some(key) {
                return;
            }
            if maximized {
                if SetWindowRgn(hwnd, std::ptr::null_mut(), 1) != 0 {
                    self.last = Some(key);
                }
                return;
            }
            let diameter = (self.radius * 2 * dpi as i32 + 48) / 96;
            let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter);
            if region.is_null() {
                return;
            }
            if SetWindowRgn(hwnd, region, 1) != 0 {
                // Ownership transfers to Windows on success.
                self.last = Some(key);
            } else {
                DeleteObject(region);
            }
        }
    }
}

/// Exercise the Windows 10 fallback on an isolated, always-hidden review window.
pub fn review(hwnd: HWND) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use windows_sys::Win32::{
        Graphics::{
            Dwm::DwmGetWindowAttribute,
            Gdi::{CreateRectRgn, GetWindowRgn, PtInRegion},
        },
        UI::WindowsAndMessaging::*,
    };
    unsafe {
        if IsWindowVisible(hwnd) != 0 {
            return Err("corner-review-window-visible".into());
        }
        let native = Corners::new(hwnd, true);
        let mut preference = -1i32;
        let readback = DwmGetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&mut preference as *mut i32).cast(),
            4,
        ) >= 0;
        if native.native && (!readback || preference != DWMWCP_ROUNDSMALL) {
            return Err("corner-native-preference".into());
        }
        let mut original = RECT::default();
        if GetWindowRect(hwnd, &mut original) == 0 {
            return Err("corner-window-bounds".into());
        }
        let (width, height) = (
            original.right - original.left,
            original.bottom - original.top,
        );
        let mut fallback = Corners {
            native: false,
            radius: 4,
            last: None,
        };
        for (w, h) in [(width, height), (width + 40, height + 24)] {
            if SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                w,
                h,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
            ) == 0
            {
                return Err("corner-review-resize".into());
            }
            fallback.resize(hwnd);
            let region = CreateRectRgn(0, 0, 0, 0);
            if region.is_null() {
                return Err("corner-review-region".into());
            }
            let valid = GetWindowRgn(hwnd, region) > 0
                && PtInRegion(region, 0, 0) == 0
                && PtInRegion(region, w / 2, h / 2) != 0
                && PtInRegion(region, w - 1, h / 2) != 0;
            DeleteObject(region);
            if !valid {
                return Err("corner-region-geometry".into());
            }
        }
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
        );
        SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
        let _ = Corners::new(hwnd, true);
        Ok(
            serde_json::json!({"backend":if native.native {"dwm-preference"}else{"window-region"},"native_preference_readback":readback,"fallback_resize_geometry":true,"visible_dwm_edge_verified":false}),
        )
    }
}
