use fuze_contract::{ContextGate, Slot, Snapshot};
use std::{
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        ProcessStatus::*,
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
    UI::{HiDpi::*, WindowsAndMessaging::*},
};
static PAINTS: AtomicUsize = AtomicUsize::new(0);
static SNAPSHOT: OnceLock<Snapshot> = OnceLock::new();
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
        "数据 fixture-v1",
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
        let (name, rate, pick, progress) =
            match &SNAPSHOT.get().expect("validated snapshot").slots[i as usize] {
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
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.first().is_some_and(|v| v == "--smoke");
    if !(smoke && args.len() == 3 || args.len() == 2 && args[0] == "--show") {
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
    SNAPSHOT.set(snapshot).map_err(|_| "duplicate-context")?;
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
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("FUZE Rust prototype").as_ptr(),
            WS_POPUP,
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
        if smoke {
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            let no_activate = style & WS_EX_NOACTIVATE != 0;
            let tool_window = style & WS_EX_TOOLWINDOW != 0;
            let hidden = IsWindowVisible(hwnd) == 0;
            let topmost = style & WS_EX_TOPMOST != 0;
            let render = save_own_surface(PathBuf::from(&args[1]), 760, 170);
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
            let idle_cpu_ms = (cpu_end? - cpu_start?) as f64 / 10000.0;
            if !memory_ok {
                return Err("memory-metric-unavailable".into());
            }
            println!(
                "{}",
                serde_json::json!({"hidden":hidden,"no_activate":no_activate,"tool_window":tool_window,"topmost_style":topmost,"foreground_unchanged":unchanged,"paint_events":PAINTS.load(Ordering::Relaxed),"idle_cpu_ms_500ms":idle_cpu_ms,"working_set_bytes":memory.WorkingSetSize,"capture_screen":false,"game_access":false})
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
