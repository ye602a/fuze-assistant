//! Separate updater process UI; all network and install work stays on its worker thread.
use std::{
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
use windows_sys::Win32::{
    Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*,
};
pub static CANCEL: AtomicBool = AtomicBool::new(false);
pub static STAGE: AtomicU8 = AtomicU8::new(0);
pub static OWNER: AtomicIsize = AtomicIsize::new(0);
pub fn cancelled() -> bool {
    CANCEL.load(Ordering::Acquire)
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE | WM_COMMAND => {
            CANCEL.store(true, Ordering::Release);
            SetWindowTextW(
                GetDlgItem(hwnd, 1),
                wide("正在取消 · 等待当前请求结束，旧版本保留").as_ptr(),
            );
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
pub fn run(work: fn() -> crate::Result<()>) -> crate::Result<()> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide("FuzeRustUpdateProgress");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hbrBackground: (16usize) as _,
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return Err("update-window");
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("符择 · Rust 更新").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            540,
            170,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return Err("update-window");
        }
        OWNER.store(hwnd as isize, Ordering::Release);
        CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("等待确认 · 随时可以取消，旧版本保留").as_ptr(),
            WS_CHILD | WS_VISIBLE,
            20,
            20,
            490,
            40,
            hwnd,
            1usize as HMENU,
            instance,
            null(),
        );
        CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("取消").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            380,
            70,
            110,
            30,
            hwnd,
            2usize as HMENU,
            instance,
            null(),
        );
        let result = Arc::new(Mutex::new(None));
        let output = result.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(work).unwrap_or(Err("update-worker-failed"));
            if let Ok(mut slot) = output.lock() {
                *slot = Some(r);
            }
        });
        let mut previous = 255;
        loop {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if let Some(value) = result.lock().map_err(|_| "update-worker-failed")?.take() {
                OWNER.store(0, Ordering::Release);
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return value;
            }
            let stage = STAGE.load(Ordering::Acquire);
            if stage != previous && !cancelled() {
                let text = match stage {
                    1 => "正在检查固定 Rust 频道",
                    2 => "正在下载 · 优先复用旧包，失败一次全量回退",
                    3 => "正在解包并验证 OCR · 不连接游戏",
                    4 => "正在切换版本指针 · 保留上一版本",
                    _ => "等待确认 · 随时可以取消，旧版本保留",
                };
                SetWindowTextW(GetDlgItem(hwnd, 1), wide(text).as_ptr());
                previous = stage;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}
