//! Separate updater process UI; all network and install work stays on its worker thread.
use std::{
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
};
use windows_sys::Win32::{
    Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*,
};
pub static CANCEL: AtomicBool = AtomicBool::new(false);
pub static STAGE: AtomicU8 = AtomicU8::new(0);
pub static OWNER: AtomicIsize = AtomicIsize::new(0);
// Only the separate updater uses these counters. No timer or IO in the game loop.
static BYTES: AtomicU64 = AtomicU64::new(0);
static SIZE: AtomicU64 = AtomicU64::new(0);
static REUSE: AtomicU64 = AtomicU64::new(0);
static MODE: AtomicU8 = AtomicU8::new(0);
pub fn reset_transfer() {
    BYTES.store(0, Ordering::Relaxed);
    SIZE.store(0, Ordering::Relaxed);
    REUSE.store(0, Ordering::Relaxed);
    MODE.store(0, Ordering::Release);
}
pub fn transfer_event(event: crate::update_channel::TransferEvent) {
    use crate::update_channel::TransferEvent::*;
    match event {
        Delta { size, reuse } => {
            SIZE.store(size, Ordering::Relaxed);
            REUSE.store(reuse, Ordering::Relaxed);
            MODE.store(1, Ordering::Release);
        }
        Full { size, fallback } => {
            SIZE.store(size, Ordering::Relaxed);
            REUSE.store(0, Ordering::Relaxed);
            MODE.store(if fallback { 3 } else { 2 }, Ordering::Release);
        }
        Verify => MODE.store(4, Ordering::Release),
    }
}
/// Count accepted package bytes, including a failed delta attempt before fallback.
/// This deliberately excludes response headers, TLS overhead and the small blockmap.
pub struct Counted<'a>(pub &'a mut dyn std::io::Write);
pub fn received(count: u64) {
    BYTES.fetch_add(count, Ordering::Relaxed);
}
impl std::io::Write for Counted<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = self.0.write(bytes)?;
        received(count as u64);
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct View {
    stage: u8,
    mode: u8,
    received: u64,
    size: u64,
    reuse: u64,
}
fn snapshot() -> View {
    let mode = MODE.load(Ordering::Acquire);
    View {
        stage: STAGE.load(Ordering::Acquire),
        mode,
        received: BYTES.load(Ordering::Relaxed),
        size: SIZE.load(Ordering::Relaxed),
        reuse: REUSE.load(Ordering::Relaxed),
    }
}
fn mib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1048576.0)
}
fn status_text(view: View) -> String {
    if view.stage != 2 {
        return crate::i18n::text(match view.stage {
            1 => "正在检查固定 Rust 频道",
            3 => "正在解包并验证 OCR · 不连接游戏",
            4 => "正在切换版本指针 · 保留上一版本",
            _ => "等待确认 · 随时可以取消，旧版本保留",
        })
        .into();
    }
    let title = crate::i18n::text(match view.mode {
        1 => "差分下载 · 复用旧包内容",
        2 => "全量下载 · 没有可用差分基线",
        3 => "已回退全量下载 · 差分未成功",
        4 => "正在校验完整包 · 校验通过前不会安装",
        _ => "正在准备下载方案",
    });
    format!(
        "{}\n{}",
        title,
        translated(
            "完整包 {} MiB · 预计复用 {} MiB\n已接收包数据 {} MiB（含失败尝试，不含协议开销）",
            &[mib(view.size), mib(view.reuse), mib(view.received)]
        )
    )
}
pub fn transfer_summary(mode: &str, received: u64) -> String {
    translated(
        "{} · 已接收包数据 {} MiB",
        &[
            crate::i18n::text(if mode == "delta" {
                "差分更新"
            } else {
                "全量更新"
            })
            .into(),
            mib(received),
        ],
    )
}
pub fn received_bytes() -> u64 {
    BYTES.load(Ordering::Relaxed)
}
pub fn cancelled() -> bool {
    CANCEL.load(Ordering::Acquire)
}
fn wide(s: &str) -> Vec<u16> {
    crate::i18n::text(s).encode_utf16().chain(Some(0)).collect()
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
    let locale = crate::i18n::locale();
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
            720,
            270,
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
            660,
            120,
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
            560,
            170,
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
            crate::i18n::set(locale);
            let r = std::panic::catch_unwind(work).unwrap_or(Err("update-worker-failed"));
            if let Ok(mut slot) = output.lock() {
                *slot = Some(r);
            }
        });
        let mut previous = None;
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
            let view = snapshot();
            if previous != Some(view) && !cancelled() {
                SetWindowTextW(GetDlgItem(hwnd, 1), wide(&status_text(view)).as_ptr());
                previous = Some(view);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

/// The launching desktop supplies its resolved language. Standalone commands use saved settings.
pub fn init_locale() {
    let locale = std::env::var("FUZE_UPDATE_LOCALE")
        .ok()
        .filter(|l| ["zh-CN", "zh-TW", "en-US"].contains(&l.as_str()))
        .or_else(|| {
            crate::preferences::path()
                .ok()
                .and_then(|p| crate::preferences::read(&p).ok())
                .map(|p| p.locale)
        })
        .unwrap_or_else(|| "zh-CN".into());
    crate::i18n::set(&locale);
}
pub fn translated(template: &str, values: &[String]) -> String {
    let mut parts = crate::i18n::text(template).split("{}");
    let mut output = parts.next().unwrap_or_default().to_owned();
    for (part, value) in parts.zip(values) {
        output.push_str(value);
        output.push_str(part);
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn hidden_native_progress_text_fits_without_input() {
        use super::*;
        use windows_sys::Win32::Graphics::Gdi::*;
        unsafe {
            // Own hidden STATIC window only; no screen capture, show, focus or input calls.
            let hwnd = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("").as_ptr(),
                WS_POPUP,
                0,
                0,
                660,
                120,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!hwnd.is_null());
            let dc = GetDC(hwnd);
            assert!(!dc.is_null());
            let mut fits = true;
            for locale in ["zh-CN", "zh-TW", "en-US"] {
                crate::i18n::set(locale);
                for mode in 1..=4 {
                    let text = status_text(View {
                        stage: 2,
                        mode,
                        received: 1024 * 1048576,
                        size: 512 * 1048576,
                        reuse: 511 * 1048576,
                    });
                    let mut value = wide(&text);
                    SetWindowTextW(hwnd, value.as_ptr());
                    let mut actual = vec![0u16; value.len()];
                    let length = GetWindowTextW(hwnd, actual.as_mut_ptr(), actual.len() as i32);
                    fits &= length as usize == value.len() - 1
                        && actual[..length as usize] == value[..value.len() - 1];
                    let mut rect = RECT {
                        left: 0,
                        top: 0,
                        right: 660,
                        bottom: 120,
                    };
                    let height = DrawTextW(
                        dc,
                        value.as_mut_ptr(),
                        (value.len() - 1) as i32,
                        &mut rect,
                        DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
                    );
                    fits &= height > 0 && rect.bottom <= 120 && rect.right <= 660;
                }
            }
            let hidden = IsWindowVisible(hwnd) == 0;
            ReleaseDC(hwnd, dc);
            DestroyWindow(hwnd);
            crate::i18n::set("zh-CN");
            assert!(
                hidden && fits,
                "native text layout must fit the hidden progress control"
            );
        }
    }
    #[test]
    fn transfer_status_counts_partial_writes_and_keeps_failed_attempts() {
        use super::*;
        use std::io::Write;
        reset_transfer();
        STAGE.store(2, Ordering::Release);
        transfer_event(crate::update_channel::TransferEvent::Delta { size: 8, reuse: 4 });
        let mut output = Vec::new();
        Counted(&mut output).write_all(b"CCCC").unwrap();
        transfer_event(crate::update_channel::TransferEvent::Full {
            size: 8,
            fallback: true,
        });
        assert_eq!(received_bytes(), 4);
        assert_eq!(snapshot().reuse, 0);
        Counted(&mut output).write_all(b"AAAACCCC").unwrap();
        assert_eq!(received_bytes(), 12);
        struct Partial;
        impl Write for Partial {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Ok(1)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        Counted(&mut Partial).write(b"four").unwrap();
        assert_eq!(received_bytes(), 13);
        for locale in ["zh-CN", "zh-TW", "en-US"] {
            crate::i18n::set(locale);
            let text = status_text(View {
                stage: 2,
                mode: 3,
                received: 3 * 1048576,
                size: 2 * 1048576,
                reuse: 0,
            });
            assert!(text.contains("3.0"));
            assert!(!text.contains("{}"));
            assert!(!transfer_summary("delta", 1048576).contains("{}"));
        }
        crate::i18n::set("zh-CN");
        reset_transfer();
    }
    #[test]
    fn templates_preserve_values_without_recursive_substitution() {
        crate::i18n::set("en-US");
        assert_eq!(
            super::translated("A {} B {}", &["{}".into(), "value".into()]),
            "A {} B value"
        );
        crate::i18n::set("zh-CN");
    }
}
