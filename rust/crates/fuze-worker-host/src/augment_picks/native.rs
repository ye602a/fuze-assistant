//! Observe only real mouse presses/releases; always pass every input through unchanged.
use super::{now, Tracker};
use std::{
    cell::RefCell,
    ptr::null_mut,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{HiDpi::*, WindowsAndMessaging::*},
};
thread_local! { static STATE: RefCell<Option<(Arc<Mutex<Tracker>>, Arc<AtomicU64>)>> = const { RefCell::new(None) }; }
unsafe extern "system" fn hook(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && matches!(message as u32, WM_LBUTTONDOWN | WM_LBUTTONUP) {
        let event = &*(data as *const MSLLHOOKSTRUCT);
        STATE.with(|slot| {
            let state = slot.borrow();
            if let Some((shared, lost)) = state.as_ref() {
                // Never block the OS input path on OCR or UI work.
                if let Ok(mut s) = shared.try_lock() {
                    s.lost_input(lost.load(Ordering::Acquire));
                    let hwnd = s.hwnd as HWND;
                    let valid = event.flags & LLMHF_INJECTED == 0
                        && !hwnd.is_null()
                        && GetForegroundWindow() == hwnd
                        && GetAncestor(WindowFromPoint(event.pt), GA_ROOT) == hwnd;
                    s.pointer(
                        message as u32 == WM_LBUTTONDOWN,
                        valid.then_some((event.pt.x, event.pt.y)),
                        now(),
                    );
                } else {
                    // A dropped notification invalidates the click sequence; never fill its gap.
                    lost.store(now(), Ordering::Release);
                }
            }
        });
    }
    CallNextHookEx(null_mut(), code, message, data)
}
pub(super) struct Observer {
    thread: Option<thread::JoinHandle<()>>,
    id: u32,
}
impl Observer {
    pub fn start(shared: Arc<Mutex<Tracker>>, lost: Arc<AtomicU64>) -> Result<Self, &'static str> {
        let (tx, rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("augment-clicks".into())
            .spawn(move || unsafe {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                STATE.with(|s| *s.borrow_mut() = Some((shared, lost)));
                let hook = SetWindowsHookExW(
                    WH_MOUSE_LL,
                    Some(hook),
                    GetModuleHandleW(std::ptr::null()),
                    0,
                );
                if hook.is_null() {
                    let _ = tx.send(Err("augment-click-observer"));
                    return;
                }
                let mut msg: MSG = std::mem::zeroed();
                PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
                let _ = tx.send(Ok(GetCurrentThreadId()));
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                UnhookWindowsHookEx(hook);
                STATE.with(|s| *s.borrow_mut() = None);
            })
            .map_err(|_| "augment-click-thread")?;
        match rx.recv().unwrap_or(Err("augment-click-thread")) {
            Ok(id) => Ok(Self {
                thread: Some(thread),
                id,
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        unsafe {
            PostThreadMessageW(self.id, WM_QUIT, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
