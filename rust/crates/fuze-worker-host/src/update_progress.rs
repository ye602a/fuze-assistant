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
static PHASE_START: AtomicU64 = AtomicU64::new(0);
pub static ERROR_SHOWN: AtomicBool = AtomicBool::new(false);
static FILES: Mutex<(usize, usize, String, Vec<String>)> =
    Mutex::new((0, 0, String::new(), Vec::new()));
pub fn begin_files(total: usize) {
    if let Ok(mut f) = FILES.lock() {
        *f = (0, total, String::new(), Vec::new());
    }
}
pub fn file_started(path: &std::path::Path) {
    if let Ok(mut f) = FILES.lock() {
        f.2 = path.to_string_lossy().into_owned();
    }
}
pub fn file_finished(path: &std::path::Path) {
    if let Ok(mut f) = FILES.lock() {
        f.0 += 1;
        f.3.push(format!("已完成 {}", path.display()));
    }
}
static SIZE: AtomicU64 = AtomicU64::new(0);
static REUSE: AtomicU64 = AtomicU64::new(0);
static MODE: AtomicU8 = AtomicU8::new(0);
static FULL_MODE: AtomicU8 = AtomicU8::new(2);
pub fn reset_transfer() {
    BYTES.store(0, Ordering::Relaxed);
    PHASE_START.store(0, Ordering::Relaxed);
    SIZE.store(0, Ordering::Relaxed);
    REUSE.store(0, Ordering::Relaxed);
    FULL_MODE.store(2, Ordering::Relaxed);
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
            PHASE_START.store(BYTES.load(Ordering::Relaxed), Ordering::Relaxed);
            SIZE.store(size, Ordering::Relaxed);
            REUSE.store(0, Ordering::Relaxed);
            FULL_MODE.store(if fallback { 3 } else { 2 }, Ordering::Relaxed);
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
pub fn resume_prefix(bytes: u64) {
    REUSE.store(bytes, Ordering::Relaxed);
    // Full fallback is still a full-package path; only its transport resumes.
    MODE.store(
        if bytes > 0 {
            5
        } else {
            FULL_MODE.load(Ordering::Relaxed)
        },
        Ordering::Release,
    );
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
        received: BYTES
            .load(Ordering::Relaxed)
            .saturating_sub(PHASE_START.load(Ordering::Relaxed)),
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
            1 => "正在检查更新",
            3 => "正在校验并准备安装文件 · FUZE 继续运行",
            4 => "正在替换当前程序文件 · 请稍候",
            5 => "正在停止识别并关闭 FUZE…",
            6 => "文件替换完成 · 正在清理并启动新版…",
            _ => "正在准备更新 · 下载期间可以取消",
        })
        .into();
    }
    let title = crate::i18n::text(match view.mode {
        1 => "差分下载 · 复用旧包内容",
        2 => "全量下载 · 没有可用差分基线",
        3 => "已回退全量下载 · 差分未成功",
        4 => "正在校验完整包 · 校验通过前不会安装",
        5 => "继续上次下载 · 保留片段仍须整包校验",
        _ => "正在准备下载方案",
    });
    format!(
        "{}\n{}",
        title,
        translated(
            "完整包 {} MiB · 已复用 {} MiB\n本次已下载 {} MiB",
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
        WM_CLOSE | WM_COMMAND if msg == WM_CLOSE || w & 0xffff == 2 => {
            // Once copying starts, finish this short operation instead of leaving mixed files.
            if STAGE.load(Ordering::Acquire) >= 4 && !ERROR_SHOWN.load(Ordering::Acquire) {
                return 0;
            }
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
            wide("FUZE · 软件更新").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            720,
            480,
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
            wide("正在准备更新 · 下载完成后自动更新并重启").as_ptr(),
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
            390,
            110,
            30,
            hwnd,
            2usize as HMENU,
            instance,
            null(),
        );
        use windows_sys::Win32::UI::Controls::*;
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        });
        let bar = CreateWindowExW(
            0,
            wide("msctls_progress32").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE,
            20,
            145,
            660,
            18,
            hwnd,
            3usize as HMENU,
            instance,
            null(),
        );
        SendMessageW(bar, PBM_SETRANGE32, 0, 1000);
        CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32,
            20,
            180,
            660,
            200,
            hwnd,
            4usize as HMENU,
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
        let mut failed = None;
        let mut speed_at = std::time::Instant::now();
        let mut speed_bytes = 0;
        let mut speed = 0.;
        let mut last_detail = String::new();
        loop {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if let Some(value) = result.lock().map_err(|_| "update-worker-failed")?.take() {
                if let Err(error) = value {
                    failed = Some(error);
                    if !cancelled() {
                        ERROR_SHOWN.store(true, Ordering::Release);
                        SetWindowTextW(
                            GetDlgItem(hwnd, 1),
                            wide(&format!(
                                "更新未完成。{}\n{}",
                                match error {
                                    "update-network" => "无法连接更新服务器，请检查网络。",
                                    "update-cancelled-or-timeout" => "请求超时，请稍后重试。",
                                    "update-forbidden" | "update-unauthorized" => "更新服务器拒绝访问，请稍后重试。",
                                    "update-not-found" => "更新文件暂时不可用，请重新检查更新。",
                                    "update-proxy-auth" => "网络代理要求身份验证，请检查代理设置。",
                                    "update-rate-limited" => "请求暂时受限，请稍后重试。",
                                    _ => "",
                                },
                                match error {
                                    "update-file-in-use" | "install-close-preview-first" => "文件仍被占用，尚未替换。请关闭占用该安装目录的程序后重试。",
                                    "update-parent-timeout" => "等待 FUZE 退出超时，尚未替换文件。请稍后重试。",
                                    "update-release-changed" => "线上版本已变化，请在 FUZE 中重新检查更新。",
                                    _ if STAGE.load(Ordering::Acquire) < 4 => "程序文件未替换，可以关闭窗口后重试。",
                                    _ => "请重新下载安装包覆盖修复，不要启动未完成的程序。",
                                }
                            ))
                            .as_ptr(),
                        );
                        SetWindowTextW(GetDlgItem(hwnd, 2), wide("关闭").as_ptr());
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                            GetDlgItem(hwnd, 2),
                            1,
                        );
                    }
                } else {
                    OWNER.store(0, Ordering::Release);
                    DestroyWindow(hwnd);
                    UnregisterClassW(class.as_ptr(), instance);
                    return value;
                }
            }
            if let Some(error) = failed {
                if cancelled() {
                    OWNER.store(0, Ordering::Release);
                    DestroyWindow(hwnd);
                    UnregisterClassW(class.as_ptr(), instance);
                    return Err(error);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            let view = snapshot();
            if speed_at.elapsed().as_millis() >= 500 {
                let bytes = received_bytes();
                speed = bytes.saturating_sub(speed_bytes) as f64 / speed_at.elapsed().as_secs_f64();
                speed_bytes = bytes;
                speed_at = std::time::Instant::now();
            }
            let (done, total, current, recent) =
                FILES.lock().map(|f| f.clone()).unwrap_or_default();
            let fraction = if view.stage == 4 || view.stage == 6 {
                if total == 0 {
                    0.
                } else {
                    done as f64 / total as f64
                }
            } else if view.stage == 2 && view.size > 0 {
                (view.received.saturating_add(view.reuse) as f64 / view.size as f64).min(1.)
            } else {
                0.
            };
            SendMessageW(bar, PBM_SETPOS, (fraction * 1000.) as usize, 0);
            let detail = if view.stage == 4 || view.stage == 6 {
                format!(
                    "已替换 {done} / {total} 个文件\n{current}\n{}",
                    recent.join("\n")
                )
            } else if view.stage == 2 {
                format!(
                    "{:.1}% · {:.1} MB/s\n下载完成后将自动停止识别、覆盖并重启 FUZE。",
                    fraction * 100.,
                    speed / 1048576.
                )
            } else {
                "下载 → 校验 → 替换文件 → 启动新版".into()
            };
            if detail != last_detail {
                SetWindowTextW(
                    GetDlgItem(hwnd, 4),
                    wide(&detail.replace("\n", "\r\n")).as_ptr(),
                );
                last_detail = detail;
            }
            windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
                GetDlgItem(hwnd, 2),
                if view.stage >= 4 { 0 } else { 1 },
            );
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
                for mode in 1..=5 {
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
        assert_eq!(
            snapshot().received,
            0,
            "failed delta traffic must not inflate full-download progress"
        );
        resume_prefix(1048576);
        assert_eq!(snapshot().mode, 5);
        assert_eq!(snapshot().reuse, 1048576);
        assert_eq!(received_bytes(), 4);
        resume_prefix(0);
        assert_eq!(snapshot().mode, 3);
        Counted(&mut output).write_all(b"AAAACCCC").unwrap();
        assert_eq!(received_bytes(), 12);
        assert_eq!(snapshot().received, 8);
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
