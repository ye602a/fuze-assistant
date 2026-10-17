//! Own-window geometry only. Startup read, coalesced background writes, no per-frame IO.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    dpi: u32,
}
impl Geometry {
    fn valid(self) -> bool {
        self.x.abs_diff(0) <= 100_000
            && self.y.abs_diff(0) <= 100_000
            && (100..=32_768).contains(&self.width)
            && (100..=32_768).contains(&self.height)
            && (48..=768).contains(&self.dpi)
    }
    fn fit(self, work: RECT, dpi: u32) -> Self {
        let dpi = dpi.clamp(48, 768);
        let width = (self.width as i64 * dpi as i64 / self.dpi.max(1) as i64)
            .clamp(1, (work.right - work.left).max(1) as i64) as i32;
        let height = (self.height as i64 * dpi as i64 / self.dpi.max(1) as i64)
            .clamp(1, (work.bottom - work.top).max(1) as i64) as i32;
        Self {
            x: self.x.clamp(work.left, work.right - width),
            y: self.y.clamp(work.top, work.bottom - height),
            width,
            height,
            dpi,
        }
    }
}
#[derive(Default)]
struct Pending {
    next: Option<Geometry>,
    stop: bool,
}
pub struct Memory {
    failed: Arc<AtomicBool>,
    initial: Option<Geometry>,
    last: Option<Geometry>,
    shared: Arc<(Mutex<Pending>, Condvar)>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Memory {
    pub fn new(name: &str, offline: bool) -> Self {
        let path = if offline {
            None
        } else {
            std::env::var_os("LOCALAPPDATA").map(|p| {
                PathBuf::from(p)
                    .join("FuzeRustPreview")
                    .join(format!("window-{name}.json"))
            })
        };
        let initial = path
            .as_ref()
            .and_then(|p| crate::update_channel::bounded(p, 1024).ok())
            .and_then(|b| serde_json::from_slice::<Geometry>(&b).ok())
            .filter(|v| v.valid());
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let failed = Arc::new(AtomicBool::new(false));
        let worker = path.and_then(|path| {
            let shared = shared.clone();
            let failed = failed.clone();
            thread::Builder::new()
                .name("fuze-window-save".into())
                .spawn(move || loop {
                    let (lock, wake) = &*shared;
                    let Ok(mut state) = lock.lock() else { break };
                    while state.next.is_none() && !state.stop {
                        state = match wake.wait(state) {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                    let next = state.next.take();
                    let stop = state.stop;
                    drop(state);
                    if let Some(next) = next {
                        failed.store(save(&path, next).is_err(), Ordering::Release);
                    }
                    if stop {
                        break;
                    }
                })
                .ok()
        });
        Self {
            failed,
            initial,
            last: initial,
            shared,
            worker,
        }
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub unsafe fn restore(&self, hwnd: HWND) {
        if let Some(g) = self.initial {
            let r = RECT {
                left: g.x,
                top: g.y,
                right: g.x + g.width,
                bottom: g.y + g.height,
            };
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST), &mut info) != 0 {
                // Move to the intended monitor first; then use its effective window DPI.
                let first = g.fit(info.rcWork, g.dpi);
                position(hwnd, first);
                position(hwnd, g.fit(info.rcWork, GetDpiForWindow(hwnd).max(96)));
            }
        } else {
            self.recover(hwnd);
        }
    }
    pub unsafe fn recover(&self, hwnd: HWND) {
        if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
            return;
        }
        if let Some(g) = read_window(hwnd) {
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) != 0 {
                position(hwnd, g.fit(info.rcWork, g.dpi));
            }
        }
    }
    pub unsafe fn remember(&mut self, hwnd: HWND) {
        if self.worker.is_none() || IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
            return;
        }
        let next = read_window(hwnd);
        self.queue(next);
    }
    /// A disclosure changes height temporarily; persist the collapsed card dimensions instead.
    pub unsafe fn remember_fixed(&mut self, hwnd: HWND, width: u32, height: u32) {
        if self.worker.is_none() || IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
            return;
        }
        let next = read_window(hwnd)
            .map(|mut g| {
                g.width = (width as u64 * g.dpi as u64 / 96) as i32;
                g.height = (height as u64 * g.dpi as u64 / 96) as i32;
                g
            })
            .filter(|g| g.valid());
        self.queue(next);
    }
    fn queue(&mut self, next: Option<Geometry>) {
        if next.is_none() || self.last == next && !self.failed() {
            return;
        }
        if let Ok(mut p) = self.shared.0.lock() {
            p.next = next;
            self.last = next;
            self.shared.1.notify_one();
        }
    }
}
impl Drop for Memory {
    fn drop(&mut self) {
        if let Ok(mut p) = self.shared.0.lock() {
            p.stop = true;
            self.shared.1.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
unsafe fn read_window(hwnd: HWND) -> Option<Geometry> {
    let mut r: RECT = zeroed();
    if GetWindowRect(hwnd, &mut r) == 0 {
        return None;
    }
    let g = Geometry {
        x: r.left,
        y: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
        dpi: GetDpiForWindow(hwnd).max(96),
    };
    g.valid().then_some(g)
}
unsafe fn position(hwnd: HWND, g: Geometry) {
    SetWindowPos(
        hwnd,
        null_mut(),
        g.x,
        g.y,
        g.width,
        g.height,
        SWP_NOACTIVATE | SWP_NOZORDER,
    );
}
fn save(path: &std::path::Path, g: Geometry) -> Result<()> {
    crate::preferences::save_json(path, &g)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removed_monitor_negative_origin_and_dpi_changes() {
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        let g = Geometry {
            x: 4000,
            y: 2000,
            width: 1200,
            height: 800,
            dpi: 96,
        }
        .fit(work, 144);
        assert_eq!((g.x, g.y, g.width, g.height), (-1800, 0, 1800, 1040));
        let g = Geometry {
            x: -1800,
            y: 40,
            width: 900,
            height: 600,
            dpi: 144,
        }
        .fit(work, 96);
        assert_eq!((g.x, g.y, g.width, g.height), (-1800, 40, 600, 400));
        assert!(!Geometry {
            x: i32::MIN,
            y: 0,
            width: 900,
            height: 600,
            dpi: 96
        }
        .valid());
    }
}

impl fuze_native_overlay::native::Placement for Memory {
    fn restore(&mut self, hwnd: isize) {
        unsafe { Memory::restore(self, hwnd as HWND) }
    }
    fn remember(&mut self, hwnd: isize) {
        unsafe { Memory::remember(self, hwnd as HWND) }
    }
    fn recover(&mut self, hwnd: isize) {
        unsafe { Memory::recover(self, hwnd as HWND) }
    }
}
