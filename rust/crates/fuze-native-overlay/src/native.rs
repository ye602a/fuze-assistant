use fuze_contract::{ContextGate, Slot, Snapshot};
use std::{
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    Storage::Xps::{PrintWindow, PW_CLIENTONLY},
    System::{
        LibraryLoader::GetModuleHandleW,
        ProcessStatus::*,
        Threading::{
            GetCurrentProcess, GetCurrentThreadId, GetProcessHandleCount, GetProcessTimes,
        },
    },
    UI::{HiDpi::*, WindowsAndMessaging::*},
};
static PAINTS: AtomicUsize = AtomicUsize::new(0);
thread_local! { static SNAPSHOT: std::cell::RefCell<Option<Snapshot>> = const { std::cell::RefCell::new(None) }; }
thread_local! {
    static STATS: std::cell::RefCell<std::collections::HashMap<u32, fuze_data::recommendations::Recommendation>> = std::cell::RefCell::new(std::collections::HashMap::new());
    static SOURCE: std::cell::RefCell<Option<SourceImage>> = const { std::cell::RefCell::new(None) };
    static STATISTICS_MODE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PULSE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}
pub struct SourceImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    let brush = CreateSolidBrush(color);
    FillRect(dc, &rect, brush);
    DeleteObject(brush);
}
unsafe fn label(dc: HDC, value: &str, rect: RECT, size: i32, color: u32) {
    let font = CreateFontW(
        -size,
        0,
        0,
        0,
        500,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        wide("Microsoft YaHei UI").as_ptr(),
    );
    let previous = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    let text = wide(value);
    let mut rect = rect;
    DrawTextW(
        dc,
        text.as_ptr(),
        (text.len() - 1) as i32,
        &mut rect,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    SelectObject(dc, previous);
    DeleteObject(font);
}
unsafe fn draw(dc: HDC, w: i32, h: i32) {
    if SOURCE.with(|s| {
        let source = s.borrow();
        let Some(source) = source.as_ref() else {
            return false;
        };
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: source.width as i32,
            biHeight: -(source.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        StretchDIBits(
            dc,
            0,
            0,
            w,
            h,
            0,
            0,
            source.width as i32,
            source.height as i32,
            source.bytes.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        fill(
            dc,
            RECT {
                left: 0,
                top: 0,
                right: 8,
                bottom: 8,
            },
            PULSE.with(|p| p.get()).wrapping_mul(0x010101) & 0xffffff,
        );
        true
    }) {
        return;
    }
    let snapshot = SNAPSHOT.with(|s| s.borrow().clone());
    let slots = snapshot
        .as_ref()
        .map(|s| s.slots.clone())
        .unwrap_or_else(|| std::array::from_fn(|_| Slot::Unreadable));
    let version = snapshot
        .as_ref()
        .map(|s| s.dataset_version.as_str())
        .unwrap_or("等待数据");
    let sx = w as f64 / 760.0;
    let sy = h as f64 / 170.0;
    let rect = |l: i32, t: i32, r: i32, b: i32| RECT {
        left: (l as f64 * sx) as i32,
        top: (t as f64 * sy) as i32,
        right: (r as f64 * sx) as i32,
        bottom: (b as f64 * sy) as i32,
    };
    fill(dc, rect(0, 0, 760, 170), rgb(5, 16, 25));
    label(
        dc,
        "FUZE · Rust 原型 · 离线识别 / 未确认选择界面",
        rect(12, 3, 580, 25),
        (12.0 * sy) as i32,
        rgb(198, 173, 112),
    );
    label(
        dc,
        &format!("数据 {version}"),
        rect(600, 3, 715, 25),
        (11.0 * sy) as i32,
        rgb(145, 174, 184),
    );
    label(
        dc,
        "×",
        rect(730, 2, 754, 25),
        (16.0 * sy) as i32,
        rgb(198, 173, 112),
    );
    for i in 0..3 {
        let x = 12 + i * 248;
        fill(dc, rect(x, 30, x + 240, 160), rgb(101, 85, 51));
        fill(dc, rect(x + 1, 31, x + 239, 159), rgb(13, 32, 44));
        fill(dc, rect(x + 10, 46, x + 57, 93), rgb(33, 64, 77));
        label(
            dc,
            "◇",
            rect(x + 20, 48, x + 53, 88),
            (26.0 * sy) as i32,
            rgb(221, 190, 118),
        );
        let (name, rate, pick, progress) = match &slots[i as usize] {
            Slot::Known {
                name, confidence, ..
            } => (
                name.as_str(),
                "已识别",
                "识别置信度 · 胜率待接入",
                *confidence as f64,
            ),
            Slot::Unreadable => ("此位置未识别", "等待补全", "可能被遮挡或正在刷新", 0.0),
        };
        let recommendation = match &slots[i as usize] {
            Slot::Known { id, .. } => STATS.with(|s| s.borrow().get(id).cloned()),
            _ => None,
        };
        let rate_text = recommendation.as_ref().map(|r| {
            r.win_rate
                .map(|v| format!("\u{80dc}\u{7387} {:.1}%", v * 100.))
                .unwrap_or("\u{6682}\u{65e0}\u{80dc}\u{7387}".into())
        });
        let pick_text = recommendation.as_ref().map(|r| {
            r.pick_rate
                .map(|v| format!("\u{9009}\u{53d6} {:.1}% / \u{6574}\u{4f53}", v * 100.))
                .unwrap_or("\u{9009}\u{53d6} -- / \u{6574}\u{4f53}".into())
        });
        let missing_statistics = STATISTICS_MODE.with(|s| s.get()) && recommendation.is_none();
        let rate_text = if missing_statistics {
            Some("\u{6682}\u{65e0}\u{80dc}\u{7387}".into())
        } else {
            rate_text
        };
        let pick_text = if missing_statistics {
            Some("\u{6682}\u{65e0}\u{7edf}\u{8ba1}".into())
        } else {
            pick_text
        };
        let progress = if missing_statistics { 0. } else { progress };
        let progress = recommendation
            .as_ref()
            .map(|r| r.score.unwrap_or(0.))
            .unwrap_or(progress);
        let rate = rate_text.as_deref().unwrap_or(rate);
        let pick = pick_text.as_deref().unwrap_or(pick);
        if let Some(r) = &recommendation {
            label(
                dc,
                &format!("{} / {} / {}", r.origin, r.region, r.patch),
                rect(x + 10, 120, x + 230, 132),
                (9.0 * sy) as i32,
                rgb(145, 174, 184),
            );
        }
        label(
            dc,
            name,
            rect(x + 65, 40, x + 230, 65),
            (16.0 * sy) as i32,
            rgb(240, 224, 187),
        );
        label(
            dc,
            rate,
            rect(x + 65, 66, x + 232, 94),
            (19.0 * sy) as i32,
            rgb(126, 211, 207),
        );
        label(
            dc,
            pick,
            rect(x + 10, 99, x + 230, 121),
            (12.0 * sy) as i32,
            rgb(166, 185, 194),
        );
        fill(dc, rect(x + 10, 133, x + 230, 144), rgb(32, 51, 62));
        if progress > 0.0 {
            fill(
                dc,
                rect(x + 10, 133, x + 10 + (220.0 * progress) as i32, 144),
                rgb(195, 164, 94),
            );
        }
    }
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_LBUTTONUP => {
            let mut bounds: RECT = zeroed();
            GetClientRect(hwnd, &mut bounds);
            let x = (lparam as u16) as i16 as i32;
            let y = ((lparam >> 16) as u16) as i16 as i32;
            if x >= bounds.right * 730 / 760 && y >= 0 && y <= bounds.bottom * 25 / 170 {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PRINT | WM_PRINTCLIENT => {
            let mut bounds: RECT = zeroed();
            GetClientRect(hwnd, &mut bounds);
            draw(wparam as HDC, bounds.right, bounds.bottom);
            0
        }
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut bounds: RECT = zeroed();
            GetClientRect(hwnd, &mut bounds);
            draw(dc, bounds.right, bounds.bottom);
            EndPaint(hwnd, &paint);
            PAINTS.fetch_add(1, Ordering::Relaxed);
            0
        }
        WM_DPICHANGED => {
            let bounds = &*(lparam as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
unsafe fn save_own_surface(path: PathBuf, w: i32, h: i32) -> Result<()> {
    let dc = CreateCompatibleDC(null_mut());
    if dc.is_null() {
        return Err("create-dc".into());
    }
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..zeroed()
    };
    let mut pixels = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
    if bitmap.is_null() {
        DeleteDC(dc);
        return Err("create-dib".into());
    }
    let previous = SelectObject(dc, bitmap);
    draw(dc, w, h);
    GdiFlush();
    let bytes = std::slice::from_raw_parts(pixels as *const u8, (w * h * 4) as usize);
    let mut rgba = Vec::with_capacity(bytes.len());
    for p in bytes.chunks_exact(4) {
        rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    SelectObject(dc, previous);
    DeleteObject(bitmap);
    DeleteDC(dc);
    image::save_buffer_with_format(
        path,
        &rgba,
        w as u32,
        h as u32,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    Ok(())
}
// Private function: the only caller supplies the HWND created in this process.
// No window enumeration, desktop DC, process discovery, or external HWND argument.
unsafe fn capture_own_window(hwnd: HWND) -> Result<serde_json::Value> {
    let mut bounds: RECT = zeroed();
    if GetClientRect(hwnd, &mut bounds) == 0 {
        return Err("capture-size-failed".into());
    }
    let (w, h) = (bounds.right, bounds.bottom);
    if w <= 0 || h <= 0 || w > 4096 || h > 2160 {
        return Err("capture-size-invalid".into());
    }
    let dc = CreateCompatibleDC(null_mut());
    if dc.is_null() {
        return Err("capture-dc-failed".into());
    }
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..zeroed()
    };
    let mut pixels = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
    if bitmap.is_null() {
        DeleteDC(dc);
        return Err("capture-dib-failed".into());
    }
    let previous = SelectObject(dc, bitmap);
    let started = std::time::Instant::now();
    let printed = PrintWindow(hwnd, dc, PW_CLIENTONLY) != 0;
    GdiFlush();
    // Sample inside the surface, away from compositor-rounded corner pixels.
    let valid_pixels = || {
        let bytes = std::slice::from_raw_parts(pixels as *const u8, (w * h * 4) as usize);
        fuze_contract::frame::BgraFrame::new(bytes, w as usize, h as usize, w as usize * 4)
            .and_then(|frame| frame.rgb_roi(w as usize / 2, h as usize - 4, 1, 1))
            .is_ok_and(|rgb| rgb == [5, 16, 25])
    };
    let print_window_verified = printed && valid_pixels();
    // Hidden compositor surfaces may be blank. This is ONLY an owned-HWND contract
    // test; explicitly report the fallback instead of pretending PrintWindow worked.
    if !print_window_verified {
        SendMessageW(hwnd, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
        GdiFlush();
    }
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let valid = valid_pixels();
    SelectObject(dc, previous);
    DeleteObject(bitmap);
    DeleteDC(dc);
    if !valid {
        println!(
            "{}",
            serde_json::json!({"error":"own-window-capture-invalid","print_succeeded":printed,"pixel_valid":valid})
        );
        return Err("own-window-capture-invalid".into());
    }
    Ok(
        serde_json::json!({"source":"owned_hidden_hwnd","capture_method":if print_window_verified {"print_window"} else {"wm_printclient"},"print_window_verified":print_window_verified,"width":w,"height":h,"stride":w*4,"capture_ms":elapsed,"bgra_valid":valid,"encoded_image_count":0}),
    )
}
fn ticks(t: FILETIME) -> u64 {
    (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64
}
unsafe fn cpu_ticks() -> Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (zeroed(), zeroed(), zeroed(), zeroed());
    let ok = GetProcessTimes(
        GetCurrentProcess(),
        &mut created,
        &mut exited,
        &mut kernel,
        &mut user,
    );
    if ok == 0 {
        return Err("cpu-metric-unavailable".into());
    }
    Ok(ticks(kernel) + ticks(user))
}
unsafe fn visible_capture_probe(hwnd: HWND) -> serde_json::Value {
    use fuze_capture::native::{FrameStatus, OwnedWindowStream};
    let mut handles_before = 0;
    GetProcessHandleCount(GetCurrentProcess(), &mut handles_before);
    let started = std::time::Instant::now();
    let mut frames = 0u32;
    let mut rebuilds = 0u32;
    let mut nonblack = false;
    let mut resized = false;
    let mut failure = None;
    let mut failure_details = None;
    let mut tick = 0u32;
    match OwnedWindowStream::start(hwnd as isize, true) {
        Err(error) => {
            failure = Some("wgc_start_unavailable");
            failure_details = Some(fuze_capture::native::failure_details(&*error));
        }
        Ok(mut stream) => {
            while started.elapsed() < std::time::Duration::from_secs(4) {
                let mut message: MSG = zeroed();
                while PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                if frames > 0 && !resized {
                    SetWindowPos(hwnd, HWND_TOPMOST, 40, 40, 960, 540, SWP_NOACTIVATE);
                    resized = true;
                }
                InvalidateRect(hwnd, null(), 0);
                UpdateWindow(hwnd);
                // A static repaint can legitimately be coalesced by DWM. Change a tiny area
                // of this CI-owned source so frame/recreate progress doesn't depend on that.
                tick += 1;
                let dc = GetDC(hwnd);
                if !dc.is_null() {
                    // Two-color alternation can alias when DWM samples every second paint.
                    let brush = CreateSolidBrush(tick.wrapping_mul(0x00010101) & 0x00ffffff);
                    let pulse = RECT {
                        left: 20,
                        top: 20,
                        right: 36,
                        bottom: 36,
                    };
                    FillRect(dc, &pulse, brush);
                    DeleteObject(brush);
                    ReleaseDC(hwnd, dc);
                }
                match stream.next(std::time::Duration::from_millis(100)) {
                    Ok(FrameStatus::Ready) => {
                        frames += 1;
                        if let Ok(atlas) = stream.titles() {
                            if let Ok(pixels) = atlas.rgb_slot(0) {
                                nonblack |= pixels.iter().any(|v| *v > 0);
                            }
                        }
                    }
                    Ok(FrameStatus::Resized) => rebuilds += 1,
                    Ok(_) => {}
                    Err(error) => {
                        failure = Some("wgc_frame_failed");
                        failure_details = Some(fuze_capture::native::failure_details(&*error));
                        break;
                    }
                }
                if frames >= 3 && rebuilds > 0 {
                    break;
                }
            }
        }
    } // Stream/session/frame pool are dropped before checking owned-process handles.
    let mut handles_after = 0;
    GetProcessHandleCount(GetCurrentProcess(), &mut handles_after);
    serde_json::json!({"frames":frames,"rebuilds":rebuilds,"nonblack":nonblack,
        "verified":frames>=3&&rebuilds>0&&nonblack&&failure.is_none(),"failure":failure,"failure_details":failure_details,
        "elapsed_ms":started.elapsed().as_secs_f64()*1000.,
        "handle_delta":handles_after as i64-handles_before as i64})
}
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.first().is_some_and(|v| v == "--smoke");
    let ci_capture = args.first().is_some_and(|v| v == "--wgc-ci");
    if ci_capture
        && !(std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("FUZE_RUST_CI_VISIBLE").as_deref() == Ok("1"))
    {
        return Err("visible-test-ci-only".into());
    }
    if !(smoke && args.len() == 3 || args.len() == 2 && (args[0] == "--show" || ci_capture)) {
        return Err("explicit-mode-required".into());
    }
    let snapshot_path = args.last().unwrap();
    if std::fs::metadata(snapshot_path)?.len() > 16 * 1024 {
        return Err("snapshot-too-large".into());
    }
    let snapshot: Snapshot = serde_json::from_slice(&std::fs::read(snapshot_path)?)?;
    let mut gate = ContextGate::new(1, "fixture-v1".into());
    if !gate.accept(&snapshot) || snapshot.selection_confirmed {
        return Err("invalid-offline-context".into());
    }
    SNAPSHOT.with(|s| *s.borrow_mut() = Some(snapshot));
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let foreground = GetForegroundWindow();
        let instance = GetModuleHandleW(null());
        let class = wide("FuzeRustIsolatedPrototype");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return Err("register-window".into());
        }
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE
                | if ci_capture {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW
                },
            class.as_ptr(),
            wide("FUZE Rust prototype").as_ptr(),
            if ci_capture {
                WS_OVERLAPPEDWINDOW
            } else {
                WS_POPUP
            },
            40,
            40,
            760,
            170,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            UnregisterClassW(class.as_ptr(), instance);
            return Err("create-window".into());
        }
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        if ci_capture {
            SetWindowPos(hwnd, HWND_TOPMOST, 40, 40, 1280, 720, SWP_NOACTIVATE);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            UpdateWindow(hwnd);
            let report = visible_capture_probe(hwnd);
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
            let unchanged = foreground == GetForegroundWindow();
            println!(
                "{}",
                serde_json::json!({"wgc_stream":report,"foreground_unchanged":unchanged,"game_access":false,"source":"owned_ci_window"})
            );
            if !unchanged {
                return Err("foreground-changed".into());
            }
        } else if smoke {
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            let no_activate = style & WS_EX_NOACTIVATE != 0;
            let tool_window = style & WS_EX_TOOLWINDOW != 0;
            let hidden = IsWindowVisible(hwnd) == 0;
            let topmost = style & WS_EX_TOPMOST != 0;
            let render = save_own_surface(PathBuf::from(&args[1]), 760, 170);
            let capture = capture_own_window(hwnd);
            let wgc = if std::env::var_os("FUZE_RUST_WGC_TEST").as_deref()
                == Some(std::ffi::OsStr::new("1"))
            {
                Some(fuze_capture::native::probe_hidden_own_window(hwnd as isize))
            } else {
                None
            };
            let cpu_start = cpu_ticks();
            std::thread::sleep(std::time::Duration::from_millis(500));
            let cpu_end = cpu_ticks();
            let mut memory: PROCESS_MEMORY_COUNTERS = zeroed();
            memory.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let memory_ok = GetProcessMemoryInfo(GetCurrentProcess(), &mut memory, memory.cb) != 0;
            let unchanged = foreground == GetForegroundWindow();
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
            render?;
            let capture = capture?;
            let idle_cpu_ms = (cpu_end? - cpu_start?) as f64 / 10000.0;
            if !memory_ok {
                return Err("memory-metric-unavailable".into());
            }
            println!(
                "{}",
                serde_json::json!({"hidden":hidden,"no_activate":no_activate,"tool_window":tool_window,"topmost_style":topmost,"foreground_unchanged":unchanged,"paint_events":PAINTS.load(Ordering::Relaxed),"idle_cpu_ms_500ms":idle_cpu_ms,"working_set_bytes":memory.WorkingSetSize,"capture_screen":false,"game_access":false,"own_window_capture":capture,"wgc":wgc})
            );
            if !hidden || !no_activate || !tool_window || !topmost || !unchanged {
                return Err("window-invariant".into());
            }
        } else {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            UpdateWindow(hwnd);
            let mut message: MSG = zeroed();
            loop {
                let result = GetMessageW(&mut message, null_mut(), 0, 0);
                if result <= 0 {
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            UnregisterClassW(class.as_ptr(), instance);
        }
    }
    Ok(())
}

/// Event-driven hidden overlay used by the integrated host. No timers, foreign HWNDs or input.
/// OCR waits stay on the caller's control thread, never on this window's message thread.
#[derive(Clone)]
pub struct Cancellation {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    hwnd: isize,
}
impl Cancellation {
    pub fn cancel(&self) {
        if !self.flag.swap(true, Ordering::AcqRel) {
            unsafe {
                PostMessageW(self.hwnd as HWND, WM_APP + 3, 0, 0);
            }
        }
    }
    pub fn cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}
pub struct HiddenOverlay {
    cancellation: Cancellation,
    hwnd: isize,
    tx: std::sync::mpsc::SyncSender<OverlayCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}
enum OverlayCommand {
    Update(Snapshot, std::sync::mpsc::SyncSender<bool>),
    Reset(u64, String, std::sync::mpsc::SyncSender<bool>),
}
impl HiddenOverlay {
    pub fn start(generation: u64, dataset: String) -> std::result::Result<Self, &'static str> {
        Self::start_inner(
            generation,
            dataset,
            std::collections::HashMap::new(),
            None,
            false,
        )
    }
    pub fn with_statistics(
        generation: u64,
        dataset: String,
        stats: std::collections::HashMap<u32, fuze_data::recommendations::Recommendation>,
    ) -> std::result::Result<Self, &'static str> {
        Self::start_inner(generation, dataset, stats, None, true)
    }
    pub fn fixture_source(image: SourceImage) -> std::result::Result<Self, &'static str> {
        if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
            || std::env::var("FUZE_RUST_CI_VISIBLE").as_deref() != Ok("1")
        {
            return Err("visible-test-ci-only");
        }
        if image.width == 0
            || image.height == 0
            || image.width > 4096
            || image.height > 2160
            || image.bytes.len() != image.width as usize * image.height as usize * 4
        {
            return Err("source-dimensions");
        }
        Self::start_inner(
            1,
            "fixture-v1".into(),
            std::collections::HashMap::new(),
            Some(image),
            false,
        )
    }
    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }
    pub fn owned_handle(&self) -> isize {
        self.hwnd
    }
    pub fn pulse(&self) {
        unsafe {
            PostMessageW(self.hwnd as HWND, WM_APP + 2, 0, 0);
        }
    }
    fn start_inner(
        generation: u64,
        dataset: String,
        stats: std::collections::HashMap<u32, fuze_data::recommendations::Recommendation>,
        source: Option<SourceImage>,
        statistics_mode: bool,
    ) -> std::result::Result<Self, &'static str> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ui_flag = flag.clone();
        let thread = std::thread::spawn(move || unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let visible = source.is_some();
            let (width, height) = source
                .as_ref()
                .map(|s| (s.width as i32, s.height as i32))
                .unwrap_or((760, 170));
            STATS.with(|s| *s.borrow_mut() = stats);
            STATISTICS_MODE.with(|s| s.set(statistics_mode));
            SOURCE.with(|s| *s.borrow_mut() = source);
            let instance = GetModuleHandleW(null());
            let class = wide(&format!("FuzeIntegrated-{}", GetCurrentThreadId()));
            let wc = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..zeroed()
            };
            if RegisterClassW(&wc) == 0 {
                let _ = ready_tx.send(0isize);
                return;
            }
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                wide("FUZE Rust integrated hidden overlay").as_ptr(),
                WS_POPUP,
                0,
                0,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                let _ = ready_tx.send(0isize);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let Some(surface) = Surface::new() else {
                let _ = ready_tx.send(0isize);
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            };
            if visible {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                UpdateWindow(hwnd);
            }
            if ready_tx.send(hwnd as isize).is_err() {
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
            let mut current_generation = generation;
            let mut current_dataset = dataset.clone();
            let mut gate = ContextGate::new(generation, dataset);
            let mut message: MSG = zeroed();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                if message.message == WM_APP + 3 {
                    SNAPSHOT.with(|s| *s.borrow_mut() = None);
                    InvalidateRect(hwnd, null(), 0);
                } else if message.message == WM_APP + 2 {
                    PULSE.with(|p| p.set(p.get().wrapping_add(1)));
                    InvalidateRect(hwnd, null(), 0);
                    UpdateWindow(hwnd);
                } else if message.message == WM_APP + 1 {
                    if let Ok(command) = rx.try_recv() {
                        let (accepted, ack) = match command {
                            OverlayCommand::Update(snapshot, ack) => {
                                let accepted = !ui_flag.load(Ordering::Acquire)
                                    && !snapshot.selection_confirmed
                                    && gate.accept(&snapshot);
                                if accepted {
                                    SNAPSHOT.with(|s| *s.borrow_mut() = Some(snapshot));
                                }
                                (accepted, ack)
                            }
                            OverlayCommand::Reset(next, dataset, ack) => {
                                let accepted = next > current_generation;
                                if accepted {
                                    current_generation = next;
                                    gate = ContextGate::new(next, dataset.clone());
                                    if current_dataset != dataset {
                                        STATS.with(|s| s.borrow_mut().clear());
                                    }
                                    current_dataset = dataset.clone();
                                    SNAPSHOT.with(|s| *s.borrow_mut() = None);
                                }
                                (accepted, ack)
                            }
                        };
                        if accepted {
                            InvalidateRect(hwnd, null(), 0);
                            if !surface.paint() {
                                let _ = ack.send(false);
                                continue;
                            }
                        }
                        let _ = ack.send(accepted);
                    }
                } else {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
            SNAPSHOT.with(|s| *s.borrow_mut() = None);
            UnregisterClassW(class.as_ptr(), instance);
        });
        let hwnd = ready_rx.recv().map_err(|_| "overlay-start")?;
        if hwnd == 0 {
            let _ = thread.join();
            return Err("overlay-start");
        }
        Ok(Self {
            cancellation: Cancellation { flag, hwnd },
            hwnd,
            tx,
            thread: Some(thread),
        })
    }
    fn send(
        &self,
        command: OverlayCommand,
        ack: std::sync::mpsc::Receiver<bool>,
    ) -> std::result::Result<bool, &'static str> {
        self.tx.try_send(command).map_err(|_| "overlay-busy")?;
        if unsafe { PostMessageW(self.hwnd as HWND, WM_APP + 1, 0, 0) } == 0 {
            return Err("overlay-closed");
        }
        ack.recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| "overlay-timeout")
    }
    pub fn update(&self, snapshot: Snapshot) -> std::result::Result<bool, &'static str> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.send(OverlayCommand::Update(snapshot, tx), rx)
    }
    pub fn reset(
        &self,
        generation: u64,
        dataset: String,
    ) -> std::result::Result<bool, &'static str> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.send(OverlayCommand::Reset(generation, dataset, tx), rx)
    }
}
impl Drop for HiddenOverlay {
    fn drop(&mut self) {
        self.cancellation.flag.store(true, Ordering::Release);
        unsafe {
            PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// One reusable owned bitmap per UI thread. This is drawing, never desktop capture.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u8,
}
impl Surface {
    unsafe fn new() -> Option<Self> {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 760,
            biHeight: -170,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        let mut pixels = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(dc);
            return None;
        }
        let previous = SelectObject(dc, bitmap);
        Some(Self {
            dc,
            bitmap,
            previous,
            pixels: pixels.cast(),
        })
    }
    unsafe fn paint(&self) -> bool {
        draw(self.dc, 760, 170);
        GdiFlush();
        let bytes = std::slice::from_raw_parts(self.pixels.add((169 * 760 + 380) * 4), 3);
        bytes == [25, 16, 5]
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
