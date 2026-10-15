//! Native desktop and tray. UI never performs LCU, capture, model or filesystem IO.
use crate::{
    controller::{Config, Controller, Monitor, MonitorControl, PipelineStatus},
    Result,
};
use fuze_lcu::reconnect::State as Connection;
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_SETLIMITTEXT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, RegisterHotKey, UnregisterHotKey, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_F8,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
};
const PAUSED: u8 = 0;
const RUNNING: u8 = 1;
const EXIT: u8 = 2;
const DATA: u8 = 3;
const APPLY: u8 = 4;
const SETTINGS: usize = 107;
const SAVE_SETTINGS: usize = 209;
const UPDATE_DATA: usize = 105;
const UPDATE_APP: usize = 106;
const TOGGLE: usize = 101;
const HIDE: usize = 102;
const QUIT: usize = 103;
const OPEN: usize = 104;
const TRAY: u32 = WM_APP + 1;
#[derive(Clone, Debug, PartialEq)]
struct Status {
    mode: u8,
    connection: Option<Connection>,
    pipeline: PipelineStatus,
    done: bool,
    available: Option<String>,
    notice: Option<&'static str>,
    update_error: Option<&'static str>,
    preferences: crate::preferences::Preferences,
    selection: Option<crate::champ_select::View>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            mode: PAUSED,
            connection: None,
            pipeline: PipelineStatus::default(),
            done: false,
            available: None,
            notice: None,
            update_error: None,
            preferences: Default::default(),
            selection: None,
        }
    }
}
struct Shared {
    wake: Condvar,
    wait: Mutex<()>,
    desired: AtomicU8,
    status: Mutex<Status>,
    monitor: Mutex<Option<MonitorControl>>,
    pending: Mutex<Option<crate::preferences::Preferences>>,
}
impl Shared {
    fn request(&self, mode: u8) {
        let _guard = self.wait.lock().ok();
        // Exit is terminal, including queued button/tray commands.
        if self.desired.load(Ordering::Acquire) == EXIT {
            return;
        }
        self.desired.store(mode, Ordering::Release);
        self.wake.notify_one();
        if let Ok(control) = self.monitor.lock() {
            if let Some(control) = control.as_ref() {
                control.enable(mode == RUNNING);
            }
        }
    }
    fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}
fn engine(mut config: Config, shared: Arc<Shared>, fixture: bool) {
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(|p| std::path::PathBuf::from(p).join("FuzeRustPreview/client-data"));
    if !fixture {
        if let Some(path) = cache.as_ref() {
            config.roots.push(fuze_data::Root {
                path: path.clone(),
                source: fuze_data::Source::Cache,
            });
        }
    }
    let mut preferences = crate::preferences::Preferences::default();
    let preference_path = if fixture {
        None
    } else {
        crate::preferences::path().ok()
    };
    if let Some(path) = preference_path.as_ref() {
        match crate::preferences::read(path).and_then(|p| {
            p.apply(&mut config)?;
            Ok(p)
        }) {
            Ok(p) => preferences = p,
            Err(e) => {
                if let Ok(mut s) = shared.status.lock() {
                    s.update_error = Some(e);
                }
            }
        }
    }
    let roots: Vec<_> = config
        .roots
        .iter()
        .map(|r| fuze_data::Root {
            path: r.path.clone(),
            source: r.source,
        })
        .collect();
    let available = |locale| {
        fuze_data::prepare(&roots, locale, fuze_data::Scope::Ocr)
            .ok()
            .map(|d| format!("可用 {}", d.identity()))
    };
    let initial_available = available(config.locale.unwrap_or(fuze_data::Locale::ZhCn));
    if let Ok(mut s) = shared.status.lock() {
        s.available = initial_available;
        s.preferences = preferences.clone();
    }
    let mut controller = Controller::new(config.clone());
    let mut champion_catalog = None;
    let mut champion_locale = None;
    let mut champion_retry = Instant::now();
    let mut last_selection = None;
    let mut selection_view = None;
    let mut monitor = None;
    let mut last_mode = PAUSED;
    loop {
        let desired = shared.desired.load(Ordering::Acquire);
        if desired == EXIT {
            break;
        }
        if desired == PAUSED {
            last_selection = None;
            selection_view = None;
            if last_mode != PAUSED {
                controller.suspend();
            }
            if let Ok(mut status) = shared.status.lock() {
                status.mode = PAUSED;
                status.connection = None;
                status.pipeline = PipelineStatus::default();
                status.selection = None;
            }
        } else if desired == APPLY {
            controller.suspend();
            let candidate = shared.pending.lock().ok().and_then(|mut p| p.take());
            let result = (|| {
                let p = candidate.ok_or("preferences-missing")?;
                let mut next = config.clone();
                p.apply(&mut next)?;
                if let Some(path) = preference_path.as_ref() {
                    crate::preferences::save(path, &p)?;
                }
                config = next;
                preferences = p;
                if let Ok(mut control) = shared.monitor.lock() {
                    *control = None;
                }
                monitor = None;
                controller = Controller::new(config.clone());
                champion_catalog = None;
                champion_locale = None;
                champion_retry = Instant::now();
                Ok::<(), &'static str>(())
            })();
            let data = available(config.locale.unwrap_or(fuze_data::Locale::ZhCn));
            if let Ok(mut s) = shared.status.lock() {
                s.preferences = preferences.clone();
                s.available = data;
                s.selection = None;
                s.update_error = result.err();
                s.notice = Some(if s.update_error.is_none() {
                    "偏好已保存；下次开始识别生效。"
                } else {
                    "保存未完成；仍使用原设置。"
                });
            }
            let _ =
                shared
                    .desired
                    .compare_exchange(APPLY, PAUSED, Ordering::AcqRel, Ordering::Acquire);
        } else if desired == DATA {
            controller.suspend();
            if let Ok(mut s) = shared.status.lock() {
                s.mode = DATA;
                s.notice = Some("正在下载并校验；旧数据继续保留。");
            }
            let result = if fixture {
                Ok(Vec::new())
            } else {
                cache.as_ref().ok_or("data-update-cache").and_then(|path| {
                    crate::data_update::refresh(path, &|| {
                        shared.desired.load(Ordering::Acquire) != DATA
                    })
                })
            };
            let refreshed_available = available(config.locale.unwrap_or(fuze_data::Locale::ZhCn));
            champion_catalog = None;
            champion_locale = None;
            champion_retry = Instant::now();
            if let Ok(mut s) = shared.status.lock() {
                s.available = refreshed_available;
                s.update_error = result.as_ref().err().copied();
                s.notice = Some(if result.is_ok() {
                    "推荐数据已校验；下次开始识别时使用最新完整版本。"
                } else {
                    "更新未完成；保留可用数据，可稍后重试。"
                });
            }
            let _ =
                shared
                    .desired
                    .compare_exchange(DATA, PAUSED, Ordering::AcqRel, Ordering::Acquire);
        } else if fixture {
            // Explicit offline entry only. No discovery or game access exists in this branch.
            if champion_catalog.is_none() {
                champion_catalog =
                    crate::champ_select::Catalog::load(&roots, fuze_data::Locale::ZhCn).ok();
            }
            if let Ok(mut status) = shared.status.lock() {
                status.mode = RUNNING;
                status.connection = Some(Connection::Connected(
                    fuze_runtime::session::Phase::ClientReady,
                ));
                if let Some(c) = champion_catalog.as_ref() {
                    let mut selection = fuze_lcu::context::Selection {
                        team: [1, 4, 17, 51, 157],
                        local_slot: Some(1),
                        bench: [0; 20],
                        count: 12,
                    };
                    selection.bench[..12]
                        .copy_from_slice(&[2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
                    status.selection = Some(c.view(selection));
                }
            }
        } else {
            if last_mode != RUNNING {
                if let Ok(mut s) = shared.status.lock() {
                    s.notice = None;
                    s.update_error = None;
                }
            }
            if monitor.is_none() {
                match Monitor::start_with_fallback(
                    false,
                    if preferences.manual_directory.is_empty() {
                        None
                    } else {
                        Some(std::path::PathBuf::from(&preferences.manual_directory))
                    },
                ) {
                    Ok(m) => {
                        if let Ok(mut control) = shared.monitor.lock() {
                            let c = m.control();
                            c.enable(shared.desired.load(Ordering::Acquire) == RUNNING);
                            *control = Some(c);
                        }
                        monitor = Some(m);
                    }
                    Err(error) => {
                        if let Ok(mut s) = shared.status.lock() {
                            s.mode = RUNNING;
                            s.pipeline.error = Some(error);
                        }
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                }
            }
            if let Some(m) = monitor.as_ref() {
                let _ = controller.tick(m);
                let selection = m.selection().filter(|_| preferences.champ_select);
                let selected_locale = config.locale.or_else(|| m.data_locale());
                if selection.is_some() && champion_locale != selected_locale {
                    champion_catalog = None;
                    last_selection = None;
                    selection_view = None;
                    champion_retry = Instant::now();
                    champion_locale = selected_locale;
                }
                if selection.is_some()
                    && champion_catalog.is_none()
                    && Instant::now() >= champion_retry
                {
                    champion_catalog = selected_locale
                        .and_then(|l| crate::champ_select::Catalog::load(&roots, l).ok());
                    champion_retry = Instant::now() + Duration::from_secs(30);
                }
                if selection != last_selection || selection_view.is_none() {
                    selection_view =
                        selection.and_then(|s| champion_catalog.as_ref().map(|c| c.view(s)));
                    last_selection = selection;
                }
                if let Ok(mut s) = shared.status.lock() {
                    s.mode = RUNNING;
                    s.connection = m.state();
                    s.pipeline = controller.status();
                    s.selection = selection_view.clone();
                    if selection.is_some() && selected_locale.is_none() {
                        s.notice = Some("选人参考等待客户端语言，暂不混用不同语言数据。");
                    } else if selection.is_some() && champion_catalog.is_none() {
                        s.notice = Some("选人数据暂不可用，30 秒后重试；可暂停后更新推荐数据。");
                    } else {
                        s.notice = None;
                    }
                }
            }
        }
        last_mode = desired;
        if let Ok(guard) = shared.wait.lock() {
            if shared.desired.load(Ordering::Acquire) == desired {
                let _wait = shared.wake.wait_timeout(
                    guard,
                    if desired == RUNNING && !fixture {
                        controller.idle_wait()
                    } else {
                        Duration::from_secs(60)
                    },
                );
            }
        }
    }
    if let Ok(control) = shared.monitor.lock() {
        if let Some(c) = control.as_ref() {
            c.enable(false);
        }
    }
    controller.suspend();
    drop(monitor);
    if let Ok(mut s) = shared.status.lock() {
        s.done = true;
        s.mode = EXIT;
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe fn app_icon() -> Option<HICON> {
    let bytes = include_bytes!("../../../../public/fuze.ico");
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    for i in 0..count {
        let e = bytes.get(6 + i * 16..22 + i * 16)?;
        if e[0] != 32 {
            continue;
        }
        let length = u32::from_le_bytes(e[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(e[12..16].try_into().ok()?) as usize;
        let data = bytes.get(offset..offset.checked_add(length)?)?;
        let icon = CreateIconFromResourceEx(
            data.as_ptr(),
            length as u32,
            1,
            0x30000,
            32,
            32,
            LR_DEFAULTCOLOR,
        );
        if !icon.is_null() {
            return Some(icon);
        }
    }
    None
}
// Only called by the explicit offline smoke entry; captures this exact owned HWND.
unsafe fn snapshot(hwnd: HWND, path: &str, ui: &mut Ui) -> Result<()> {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let dc = GetDC(hwnd);
    let memory = CreateCompatibleDC(dc);
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = r.right;
    info.bmiHeader.biHeight = -r.bottom;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || memory.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !memory.is_null() {
            DeleteDC(memory);
        }
        ReleaseDC(hwnd, dc);
        return Err("desktop-image");
    }
    let old = SelectObject(memory, bitmap);
    draw(hwnd, memory, ui);
    // Native hidden-window render; never captures the desktop or another application.
    for id in [
        TOGGLE,
        UPDATE_DATA,
        UPDATE_APP,
        SETTINGS,
        HIDE,
        QUIT,
        201,
        202,
        203,
        204,
        205,
        206,
        207,
        208,
        209,
        210,
    ] {
        let child = GetDlgItem(hwnd, id as i32);
        if GetWindowLongW(child, GWL_STYLE) as u32 & WS_VISIBLE == 0 {
            continue;
        }
        let mut bounds: RECT = zeroed();
        GetWindowRect(child, &mut bounds);
        let mut point = POINT {
            x: bounds.left,
            y: bounds.top,
        };
        ScreenToClient(hwnd, &mut point);
        SetViewportOrgEx(memory, point.x, point.y, null_mut());
        SendMessageW(
            child,
            WM_PRINT,
            memory as usize,
            (PRF_CLIENT | PRF_ERASEBKGND | PRF_NONCLIENT) as isize,
        );
        SetViewportOrgEx(memory, 0, 0, null_mut());
    }
    let mut pixels =
        std::slice::from_raw_parts(bits as *const u8, (r.right * r.bottom * 4) as usize).to_vec();
    SelectObject(memory, old);
    DeleteObject(bitmap);
    DeleteDC(memory);
    ReleaseDC(hwnd, dc);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    image::save_buffer(
        path,
        &pixels,
        r.right as u32,
        r.bottom as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|_| "desktop-image")
}
pub(crate) fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
pub(crate) unsafe fn fill(dc: HDC, r: RECT, c: u32) -> () {
    let b = CreateSolidBrush(c);
    FillRect(dc, &r, b);
    DeleteObject(b);
}
pub(crate) unsafe fn text(dc: HDC, s: &str, r: RECT, size: i32, c: u32) {
    let f = CreateFontW(
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
    let old = SelectObject(dc, f);
    SetTextColor(dc, c);
    SetBkMode(dc, TRANSPARENT as i32);
    let s = wide(s);
    let mut r = r;
    DrawTextW(
        dc,
        s.as_ptr(),
        (s.len() - 1) as i32,
        &mut r,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    SelectObject(dc, old);
    DeleteObject(f);
}
fn connection(s: Option<Connection>) -> &'static str {
    use fuze_runtime::session::Phase::*;
    match s {
        Some(Connection::Connected(ClientReady)) => "已连接 · 大厅 / 匹配中",
        Some(Connection::Connected(ChampSelect)) => "已连接 · 选择英雄",
        Some(Connection::Connected(Loading)) => "已连接 · 正在载入",
        Some(Connection::Connected(InProgress)) => "已连接 · 对局中",
        Some(Connection::Connected(PostGame)) => "已连接 · 结算中",
        Some(Connection::Retry(fuze_lcu::Error::Unauthorized)) => "认证失败 · 等待重新发现",
        Some(Connection::Retry(_)) => "连接异常 · 后台重试",
        Some(Connection::Paused) => "已暂停连接检测",
        _ => "等待英雄联盟客户端",
    }
}
fn capture(p: &PipelineStatus) -> &'static str {
    match p.capture {
        "receiving" if p.frame_age.is_some_and(|v| v < 5) => "画面已处理",
        "receiving" => "等待新的可用画面",
        "suspended" => "游戏切出 / 最小化 · 已暂停截图",
        "recovering" => "截图异常 · 等待重新建流",
        "resizing" => "游戏分辨率变化 · 正在重建",
        "gpu-busy" => "显卡忙碌 · 已跳过本帧",
        "waiting-frame" => "等待新画面 · 不重复识别",
        _ => "等待支持的对局与游戏窗口",
    }
}
fn explain(error: Option<&str>) -> &'static str {
    match error {
        None => "仅推荐，不操作游戏。领取轮次未确认，使用整体胜率。",
        Some("locale-unavailable") => "暂未取得支持的客户端语言；自动重试，不猜测语言。",
        Some("game-window-waiting") => "尚未找到匹配的游戏窗口；客户端连接不等于已进入游戏。",
        Some("worker-recovered-use-next-frame") => "OCR 已回退到 CPU，等待下一帧继续识别。",
        Some(s) if s.starts_with("capture") || s.starts_with("game-window") => {
            "截图链路暂不可用，正在退避重试；不会操作游戏。"
        }
        Some(s) if s.starts_with("worker") => "OCR 进程异常；正在清理并尝试恢复。",
        Some(s) if s.starts_with("data") => "本地推荐数据未就绪；请检查开发包是否完整。",
        _ => "模块暂不可用，正在重试；下方仅显示脱敏错误分类。",
    }
}
struct Ui {
    hotkey_ok: bool,
    settings: bool,
    draft: crate::preferences::Preferences,
    champions: Option<crate::champ_select::Window>,
    champion_preview_ok: bool,
    shared: Arc<Shared>,
    shown: Status,
    desired: u8,
    tray: bool,
    tray_registered: bool,
    tray_recovered: bool,
    tray_removed: bool,
    preview_ok: bool,
    icon: HICON,
    icon_owned: bool,
    taskbar: u32,
    exiting: Option<Instant>,
    paints: u32,
    smoke: bool,
    smoke_step: u8,
    smoke_pass: bool,
}
unsafe fn tray(hwnd: HWND, icon: HICON, add: bool) -> bool {
    let mut n: NOTIFYICONDATAW = zeroed();
    n.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd;
    n.uID = 1;
    n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    n.uCallbackMessage = TRAY;
    n.hIcon = icon;
    let tip = wide("符择 FUZE · 双击打开，右键暂停或退出");
    n.szTip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(if add { NIM_ADD } else { NIM_DELETE }, &n) != 0
}
unsafe fn show(hwnd: HWND) {
    ShowWindow(hwnd, SW_RESTORE);
    SetForegroundWindow(hwnd);
}
fn cycle<'a>(current: &str, values: &[&'a str]) -> &'a str {
    values[(values.iter().position(|s| *s == current).unwrap_or(0) + 1) % values.len()]
}
unsafe fn settings_controls(hwnd: HWND, u: &Ui) {
    ShowWindow(
        GetDlgItem(hwnd, 210),
        if u.settings { SW_SHOWNA } else { SW_HIDE },
    );
    EnableWindow(
        GetDlgItem(hwnd, 210),
        i32::from(u.desired == PAUSED && u.shown.mode == PAUSED),
    );
    let yes = |v| if v { "开启" } else { "关闭" };
    let labels = [
        format!(
            "{}  · 点击切换",
            match u.draft.backend.as_str() {
                "cpu" => "CPU",
                "dml" => "DirectML",
                _ => "自动",
            }
        ),
        format!(
            "{}  · 点击切换",
            if u.draft.locale == "auto" {
                "跟随客户端"
            } else {
                &u.draft.locale
            }
        ),
        yes(u.draft.champ_select).into(),
        yes(u.draft.champ_topmost).into(),
        if u.draft.close_to_tray {
            "收起到托盘"
        } else {
            "退出程序"
        }
        .into(),
        yes(u.draft.icon_downloads).into(),
        if u.draft.private_updates {
            "私有频道（凭据管理器）"
        } else {
            "公开频道（无令牌）"
        }
        .into(),
        yes(u.draft.pause_hotkey).into(),
        "保存设置".into(),
    ];
    for (i, label) in labels.iter().enumerate() {
        let child = GetDlgItem(hwnd, 201 + i as i32);
        SetWindowTextW(child, wide(label).as_ptr());
        ShowWindow(child, if u.settings { SW_SHOWNA } else { SW_HIDE });
        EnableWindow(
            child,
            i32::from(u.desired == PAUSED && u.shown.mode == PAUSED),
        );
    }
    SetWindowTextW(
        GetDlgItem(hwnd, SETTINGS as i32),
        wide(if u.settings {
            "返回工作台"
        } else {
            "偏好设置"
        })
        .as_ptr(),
    );
}
unsafe fn draw(hwnd: HWND, dc: HDC, u: &mut Ui) {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let scale = GetDpiForWindow(hwnd).max(96) as i32;
    let x = |v: i32| v * scale / 96;
    let y = |v: i32| v * r.bottom.min(582 * scale / 96) / 582;
    let rect = |l, t, rr, b| RECT {
        left: x(l),
        top: y(t),
        right: rr,
        bottom: y(b),
    };
    fill(dc, r, rgb(6, 17, 26));
    let gold = rgb(205, 174, 108);
    let pale = rgb(221, 232, 233);
    let dim = rgb(134, 162, 176);
    text(
        dc,
        "符择  /  F U Z E",
        rect(28, 18, r.right - x(28), 62),
        x(27),
        gold,
    );
    text(
        dc,
        "对局工作台 · Rust 开发预览",
        rect(28, 65, r.right - x(28), 96),
        x(16),
        dim,
    );
    if u.settings {
        fill(dc, rect(27, 476, r.right - x(27), 506), rgb(76, 94, 100));
        fill(dc, rect(28, 477, r.right - x(28), 505), rgb(15, 30, 41));
        text(
            dc,
            "偏好设置 · 暂停识别后保存",
            rect(28, 105, r.right - x(28), 150),
            x(24),
            gold,
        );
        for (i, label) in [
            "OCR 后端",
            "识别与推荐数据语言",
            "显示选人参考",
            "选人窗口置顶",
            "关闭主界面",
            "后台下载公共图标",
            "软件更新频道",
            "暂停快捷键 Ctrl+Shift+F8",
        ]
        .iter()
        .enumerate()
        {
            let t = 148 + (i / 2) as i32 * 67;
            let l = x(28) + (i % 2) as i32 * (r.right / 2);
            text(
                dc,
                label,
                RECT {
                    left: l,
                    top: y(t),
                    right: l + r.right / 2 - x(40),
                    bottom: y(t + 24),
                },
                x(14),
                pale,
            );
        }
        text(
            dc,
            "高级目录回退（可留空，仅自动发现失败后使用）",
            rect(28, 449, r.right - x(28), 474),
            x(13),
            dim,
        );
        text(dc, "", rect(28, 475, r.right - x(28), 500), x(13), dim);
        text(
            dc,
            if !u.hotkey_ok {
                "快捷键已被占用；仍可使用按钮或托盘暂停。"
            } else {
                u.shown.notice.unwrap_or("更改在下次开始识别时生效。")
            },
            rect(28, 418, r.right / 2 - x(12), 444),
            x(13),
            dim,
        );
        return;
    }
    let error = u.shown.pipeline.error.is_some()
        || u.shown.update_error.is_some()
        || matches!(u.shown.connection, Some(Connection::Retry(_)));
    let (title, color) = if u.desired == DATA {
        ("正在更新推荐数据", rgb(22, 47, 66))
    } else if u.desired == EXIT {
        ("正在退出 · 释放识别资源", rgb(72, 54, 32))
    } else if u.shown.mode == DATA && u.desired != DATA {
        ("正在取消更新 · 等待当前请求结束", rgb(72, 54, 32))
    } else if u.desired == PAUSED && u.shown.mode != PAUSED {
        ("正在暂停 · 清理画面与 OCR", rgb(72, 54, 32))
    } else if u.desired == PAUSED && u.shown.update_error.is_some() {
        ("更新未完成 · 已保留本地数据", rgb(72, 34, 40))
    } else if u.desired == PAUSED {
        ("已暂停 · 点击开始识别", rgb(45, 49, 60))
    } else if error {
        ("需要关注 · 正在恢复", rgb(72, 34, 40))
    } else if u.shown.pipeline.capture == "suspended" {
        ("游戏未在前台 · 截图已暂停", rgb(22, 47, 66))
    } else if u.shown.pipeline.frame_age.is_some_and(|v| v < 5) {
        ("识别运行中", rgb(24, 64, 57))
    } else {
        ("等待就绪 · 按对局状态自动识别", rgb(22, 47, 66))
    };
    fill(dc, rect(28, 112, r.right - x(28), 205), color);
    text(dc, title, rect(46, 123, r.right - x(46), 164), x(25), pale);
    text(
        dc,
        u.shown
            .notice
            .unwrap_or_else(|| explain(u.shown.pipeline.error)),
        rect(46, 167, r.right - x(46), 195),
        x(14),
        pale,
    );
    let backend = match u.shown.pipeline.backend {
        Some(fuze_contract::wire::ActiveBackend::Cpu) => "CPU · 实际运行后端",
        Some(fuze_contract::wire::ActiveBackend::DirectMl) => "DirectML · 实际运行后端",
        None => "待初始化 · 进入对局后启动",
    };
    let values = [
        (
            "客户端连接",
            if u.desired == PAUSED {
                "检测已暂停"
            } else {
                connection(u.shown.connection)
            },
        ),
        (
            "游戏画面",
            if u.desired == PAUSED {
                "未截图"
            } else {
                capture(&u.shown.pipeline)
            },
        ),
        ("OCR 引擎", backend),
        (
            "推荐数据",
            u.shown
                .pipeline
                .dataset
                .as_deref()
                .or(u.shown.available.as_deref())
                .unwrap_or("待选择客户端语言 · 未加载"),
        ),
    ];
    let width = (r.right - x(68)) / 2;
    for (i, (name, value)) in values.iter().enumerate() {
        let l = x(28) + (i % 2) as i32 * (width + x(12));
        let top = 225 + (i / 2) as i32 * 98;
        let rr = RECT {
            left: l,
            top: y(top),
            right: l + width,
            bottom: y(top + 85),
        };
        fill(dc, rr, rgb(14, 31, 42));
        text(
            dc,
            name,
            RECT {
                left: l + x(16),
                top: y(top + 8),
                right: l + width - x(12),
                bottom: y(top + 32),
            },
            x(14),
            gold,
        );
        text(
            dc,
            value,
            RECT {
                left: l + x(16),
                top: y(top + 36),
                right: l + width - x(12),
                bottom: y(top + 73),
            },
            x(16),
            pale,
        );
    }
    let detail = if let Some(e) = u.shown.pipeline.error.or(u.shown.update_error) {
        format!("错误分类：{e}")
    } else if let Some(Connection::Retry(e)) = u.shown.connection {
        format!("连接错误分类：{e:?}")
    } else if let Some(age) = u.shown.pipeline.frame_age {
        format!("距上次完成识别：{age} 秒 · 当前推荐使用整体胜率")
    } else {
        "尚无本次运行的识别结果；等待阶段不代表故障。".into()
    };
    text(dc, &detail, rect(28, 428, r.right - x(28), 461), x(14), dim);
    text(
        dc,
        if u.tray {
            "关闭窗口后留在托盘；结束运行请点“退出”。"
        } else {
            "托盘不可用：关闭窗口将退出，避免留下不可见进程。"
        },
        rect(28, 465, r.right - x(28), 495),
        x(13),
        dim,
    );
    u.paints += 1;
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let u = &mut *pointer;
    if u.taskbar != 0 && msg == u.taskbar {
        if u.tray {
            tray(hwnd, u.icon, false);
        }
        u.tray = tray(hwnd, u.icon, true);
        u.tray_recovered = u.tray;
        EnableWindow(GetDlgItem(hwnd, HIDE as i32), i32::from(u.tray));
        InvalidateRect(hwnd, null(), 0);
        return 0;
    }
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            draw(hwnd, dc, u);
            EndPaint(hwnd, &ps);
            0
        }
        WM_DRAWITEM => {
            let d = &*(l as *const DRAWITEMSTRUCT);
            fill(
                d.hDC,
                d.rcItem,
                if d.itemState & ODS_DISABLED != 0 {
                    rgb(36, 42, 48)
                } else if d.itemState & ODS_SELECTED != 0 {
                    rgb(112, 89, 47)
                } else {
                    rgb(185, 154, 91)
                },
            );
            let mut caption = [0u16; 64];
            let n = GetWindowTextW(d.hwndItem, caption.as_mut_ptr(), 64);
            let mut r = d.rcItem;
            r.left += 14;
            text(
                d.hDC,
                &String::from_utf16_lossy(&caption[..n as usize]),
                r,
                16 * GetDpiForWindow(hwnd).max(96) as i32 / 96,
                if d.itemState & ODS_DISABLED != 0 {
                    rgb(145, 157, 165)
                } else {
                    rgb(7, 20, 29)
                },
            );
            if d.itemState & ODS_FOCUS != 0 {
                DrawFocusRect(d.hDC, &d.rcItem);
            }
            1
        }
        WM_PRINTCLIENT => {
            draw(hwnd, w as HDC, u);
            0
        }
        WM_ERASEBKGND => 1,
        WM_GETMINMAXINFO => {
            let m = &mut *(l as *mut MINMAXINFO);
            let d = GetDpiForWindow(hwnd).max(96) as i32;
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info);
            m.ptMinTrackSize = POINT {
                x: (860 * d / 96).min(info.rcWork.right - info.rcWork.left),
                y: (620 * d / 96).min(info.rcWork.bottom - info.rcWork.top),
            };
            0
        }
        WM_SIZE => {
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r);
            let d = GetDpiForWindow(hwnd).max(96) as i32;
            for (i, id) in [TOGGLE, UPDATE_DATA, UPDATE_APP, SETTINGS, HIDE, QUIT]
                .iter()
                .enumerate()
            {
                MoveWindow(
                    GetDlgItem(hwnd, *id as i32),
                    28 * d / 96 + i as i32 * ((r.right - 56 * d / 96) / 6),
                    r.bottom - 60 * d / 96,
                    ((r.right - 56 * d / 96) / 6 - 10 * d / 96).max(1),
                    38 * d / 96,
                    1,
                );
            }
            for i in 0..9 {
                MoveWindow(
                    GetDlgItem(hwnd, 201 + i),
                    if i == 8 {
                        r.right / 2
                    } else {
                        28 * d / 96 + (i % 2) * (r.right / 2)
                    },
                    (if i == 8 { 418 } else { 174 + (i / 2) * 67 }) * d / 96,
                    r.right / 2 - 56 * d / 96,
                    38 * d / 96,
                    1,
                );
            }
            MoveWindow(
                GetDlgItem(hwnd, 210),
                28 * d / 96,
                477 * d / 96,
                r.right - 56 * d / 96,
                28 * d / 96,
                1,
            );
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_DPICHANGED => {
            let r = &*(l as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            0
        }
        WM_HOTKEY if w == 1 => {
            if !u.smoke && [PAUSED, RUNNING].contains(&u.desired) {
                PostMessageW(hwnd, WM_COMMAND, TOGGLE, 0);
            }
            0
        }
        WM_COMMAND => {
            match w & 0xffff {
                SETTINGS => {
                    u.settings = !u.settings;
                    if u.settings {
                        u.draft = u.shown.preferences.clone();
                        SetWindowTextW(
                            GetDlgItem(hwnd, 210),
                            wide(&u.draft.manual_directory).as_ptr(),
                        );
                    }
                    settings_controls(hwnd, u);
                }
                201..=208 if u.settings && u.desired == PAUSED && u.shown.mode == PAUSED => {
                    match w & 0xffff {
                        201 => {
                            u.draft.backend =
                                cycle(&u.draft.backend, &["auto", "cpu", "dml"]).into()
                        }
                        202 => {
                            u.draft.locale =
                                cycle(&u.draft.locale, &["auto", "zh-CN", "zh-TW", "en-US"]).into()
                        }
                        203 => u.draft.champ_select = !u.draft.champ_select,
                        204 => u.draft.champ_topmost = !u.draft.champ_topmost,
                        205 => u.draft.close_to_tray = !u.draft.close_to_tray,
                        206 => u.draft.icon_downloads = !u.draft.icon_downloads,
                        207 => u.draft.private_updates = !u.draft.private_updates,
                        208 => u.draft.pause_hotkey = !u.draft.pause_hotkey,
                        _ => {}
                    }
                    settings_controls(hwnd, u);
                }
                SAVE_SETTINGS if u.settings => {
                    if u.shared.desired.load(Ordering::Acquire) == PAUSED && u.shown.mode == PAUSED
                    {
                        let mut directory = [0u16; 513];
                        let n = GetWindowTextW(GetDlgItem(hwnd, 210), directory.as_mut_ptr(), 513);
                        u.draft.manual_directory =
                            String::from_utf16_lossy(&directory[..n.max(0) as usize])
                                .trim()
                                .into();
                        if let Ok(mut pending) = u.shared.pending.lock() {
                            *pending = Some(u.draft.clone());
                        }
                        u.shared.request(APPLY);
                    }
                }
                TOGGLE => {
                    u.shared
                        .request(if u.shared.desired.load(Ordering::Acquire) == RUNNING {
                            PAUSED
                        } else {
                            RUNNING
                        })
                }
                UPDATE_DATA => {
                    if u.shared.desired.load(Ordering::Acquire) == PAUSED {
                        u.shared.request(DATA);
                    }
                }
                UPDATE_APP => {
                    if !u.smoke
                        && u.shared.desired.load(Ordering::Acquire) == PAUSED
                        && u.shown.mode == PAUSED
                    {
                        use std::os::windows::process::CommandExt;
                        if let Ok(exe) = std::env::current_exe() {
                            if std::process::Command::new(exe.with_file_name("fuze-update.exe"))
                                .arg(if u.shown.preferences.private_updates {
                                    "--update-private"
                                } else {
                                    "--update"
                                })
                                .creation_flags(0x08000000)
                                .spawn()
                                .is_ok()
                            {
                                u.shared.request(EXIT);
                            }
                        }
                    }
                }
                HIDE => {
                    if u.tray {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                }
                QUIT => u.shared.request(EXIT),
                OPEN => {
                    if !u.smoke {
                        show(hwnd);
                    }
                }
                _ => {}
            }
            // The timer must observe this transition even if background cleanup is blocked.
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            if u.tray && u.shown.preferences.close_to_tray {
                ShowWindow(hwnd, SW_HIDE);
            } else {
                u.shared.request(EXIT);
            }
            0
        }
        TRAY => {
            match l as u32 {
                WM_LBUTTONDBLCLK => {
                    if !u.smoke {
                        show(hwnd);
                    }
                }
                WM_RBUTTONUP => {
                    let menu = CreatePopupMenu();
                    AppendMenuW(menu, MF_STRING, OPEN, wide("打开符择").as_ptr());
                    AppendMenuW(
                        menu,
                        MF_STRING,
                        TOGGLE,
                        wide(if u.shared.desired.load(Ordering::Acquire) == RUNNING {
                            "暂停识别"
                        } else {
                            "开始识别"
                        })
                        .as_ptr(),
                    );
                    AppendMenuW(menu, MF_STRING, QUIT, wide("退出").as_ptr());
                    let mut p: POINT = zeroed();
                    GetCursorPos(&mut p);
                    SetForegroundWindow(hwnd);
                    let selected = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        p.x,
                        p.y,
                        0,
                        hwnd,
                        null(),
                    );
                    DestroyMenu(menu);
                    if selected != 0 {
                        PostMessageW(hwnd, WM_COMMAND, selected as usize, 0);
                    }
                    PostMessageW(hwnd, WM_NULL, 0, 0);
                }
                _ => {}
            }
            0
        }
        WM_TIMER => {
            let next = u.shared.status();
            let desired = u.shared.desired.load(Ordering::Acquire);
            if next != u.shown || desired != u.desired {
                if !u.smoke && next.preferences.pause_hotkey != u.shown.preferences.pause_hotkey {
                    UnregisterHotKey(hwnd, 1);
                    u.hotkey_ok = true;
                    if next.preferences.pause_hotkey {
                        u.hotkey_ok = RegisterHotKey(
                            hwnd,
                            1,
                            MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                            VK_F8 as u32,
                        ) != 0;
                    }
                }
                u.shown = next;
                if let Some(window) = u.champions.as_mut() {
                    window.update(
                        u.shown.selection.clone().filter(|_| desired == RUNNING),
                        u.shown.preferences.champ_topmost,
                        u.smoke,
                        u.shown.preferences.icon_downloads,
                    );
                }
                u.desired = desired;
                settings_controls(hwnd, u);
                SetWindowTextW(
                    GetDlgItem(hwnd, TOGGLE as i32),
                    wide(if desired == RUNNING {
                        "暂停识别"
                    } else {
                        "开始识别"
                    })
                    .as_ptr(),
                );
                EnableWindow(
                    GetDlgItem(hwnd, TOGGLE as i32),
                    i32::from(desired != EXIT && desired != APPLY),
                );
                EnableWindow(
                    GetDlgItem(hwnd, UPDATE_DATA as i32),
                    i32::from(desired == PAUSED),
                );
                EnableWindow(
                    GetDlgItem(hwnd, UPDATE_APP as i32),
                    i32::from(desired == PAUSED && u.shown.mode == PAUSED),
                );
                InvalidateRect(hwnd, null(), 0);
            }
            if u.smoke {
                match u.smoke_step {
                    0 => {
                        if u.tray && u.taskbar != 0 {
                            PostMessageW(hwnd, u.taskbar, 0, 0);
                        }
                        PostMessageW(hwnd, WM_COMMAND, TOGGLE, 0);
                        u.smoke_step = 1;
                    }
                    1 if u.shown.mode == RUNNING
                        && u.champions.as_ref().is_some_and(|w| w.detail_ready()) =>
                    {
                        if let (Some(window), Ok(path)) =
                            (u.champions.as_ref(), std::env::var("FUZE_CHAMPION_PREVIEW"))
                        {
                            u.champion_preview_ok = u.shown.selection.is_some()
                                && window.render(std::path::Path::new(&path)).is_ok();
                        }
                        if let Ok(path) = std::env::var("FUZE_DESKTOP_PREVIEW") {
                            u.preview_ok = snapshot(hwnd, &path, u).is_ok();
                        }
                        PostMessageW(hwnd, WM_COMMAND, TOGGLE, 0);
                        u.smoke_step = 2;
                    }
                    2 if u.shown.mode == PAUSED => {
                        if let Ok(path) = std::env::var("FUZE_SETTINGS_PREVIEW") {
                            u.settings = true;
                            u.draft = u.shown.preferences.clone();
                            settings_controls(hwnd, u);
                            if snapshot(hwnd, &path, u).is_err() {
                                u.preview_ok = false;
                            }
                            u.settings = false;
                            settings_controls(hwnd, u);
                        }
                        u.smoke_pass = true;
                        PostMessageW(hwnd, WM_COMMAND, QUIT, 0);
                        u.smoke_step = 3;
                    }
                    _ => {}
                }
            }
            if desired == EXIT {
                let started = u.exiting.get_or_insert_with(Instant::now);
                if u.shown.done || started.elapsed() > Duration::from_secs(5) {
                    DestroyWindow(hwnd);
                }
            }
            0
        }
        WM_DESTROY => {
            u.shared.request(EXIT);
            KillTimer(hwnd, 1);
            if !u.smoke {
                UnregisterHotKey(hwnd, 1);
            }
            if u.tray {
                u.tray_removed = tray(hwnd, u.icon, false);
                u.tray = false;
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
/// UI startup is paused. Only an explicit user command starts live discovery.
pub fn startup_error(error: &str) {
    let message = if error == "desktop-already-running" {
        "符择 Rust 已在运行，请从托盘打开。".into()
    } else {
        format!("符择 Rust 无法启动。请确认开发包已完整解压。\n错误分类：{error}")
    };
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(&message).as_ptr(),
            wide("符择 FUZE").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn run(config: Config, smoke: bool, managed: bool) -> Result<()> {
    struct Instance(HANDLE);
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let _instance = if smoke {
        None
    } else {
        unsafe {
            let h = windows_sys::Win32::System::Threading::CreateMutexW(
                null(),
                0,
                wide("Local\\FuzeRustDesktopPreview").as_ptr(),
            );
            if h.is_null() {
                return Err("desktop-instance");
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(h);
                return Err("desktop-already-running");
            }
            Some(Instance(h))
        }
    };
    let shared = Arc::new(Shared {
        wake: Condvar::new(),
        wait: Mutex::new(()),
        desired: AtomicU8::new(PAUSED),
        status: Mutex::new(Status::default()),
        monitor: Mutex::new(None),
        pending: Mutex::new(None),
    });
    let mut champion_roots = config.roots.clone();
    if !smoke {
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            champion_roots.push(fuze_data::Root {
                path: std::path::PathBuf::from(p).join("FuzeRustPreview/client-data"),
                source: fuze_data::Source::Cache,
            });
        }
    }
    let worker_shared = shared.clone();
    let worker = thread::Builder::new()
        .name("fuze-desktop-control".into())
        .spawn(move || engine(config, worker_shared, smoke))
        .map_err(|_| "desktop-thread")?;
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let class = wide("FuzeRustDesktop");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            shared.request(EXIT);
            let _ = worker.join();
            return Err("desktop-class");
        }
        let icon = app_icon();
        let mut ui = Box::new(Ui {
            hotkey_ok: true,
            settings: false,
            draft: Default::default(),
            champions: crate::champ_select::Window::new(champion_roots, smoke).ok(),
            champion_preview_ok: false,
            icon: icon.unwrap_or_else(|| LoadIconW(null_mut(), IDI_APPLICATION)),
            icon_owned: icon.is_some(),
            shared: shared.clone(),
            shown: Status::default(),
            desired: PAUSED,
            tray: false,
            tray_registered: false,
            tray_recovered: false,
            tray_removed: false,
            preview_ok: false,
            taskbar: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            exiting: None,
            paints: 0,
            smoke,
            smoke_step: 0,
            smoke_pass: false,
        });
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("符择 FUZE · Rust").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            960,
            660,
            null_mut(),
            null_mut(),
            instance,
            (&mut *ui as *mut Ui).cast(),
        );
        if hwnd.is_null() {
            shared.request(EXIT);
            let _ = worker.join();
            UnregisterClassW(class.as_ptr(), instance);
            return Err("desktop-window");
        }
        for (id, label) in [
            (TOGGLE, "开始识别"),
            (UPDATE_DATA, "更新推荐数据"),
            (UPDATE_APP, "退出并检查更新"),
            (SETTINGS, "偏好设置"),
            (HIDE, "收起到托盘"),
            (QUIT, "退出"),
        ] {
            CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide(label).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                0,
                0,
                0,
                0,
                hwnd,
                id as HMENU,
                instance,
                null(),
            );
        }
        for id in 201..=209 {
            CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_TABSTOP | BS_OWNERDRAW as u32,
                0,
                0,
                0,
                0,
                hwnd,
                id as HMENU,
                instance,
                null(),
            );
        }
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            0,
            0,
            0,
            0,
            hwnd,
            210 as HMENU,
            instance,
            null(),
        );
        if edit.is_null() {
            shared.request(EXIT);
            DestroyWindow(hwnd);
            return Err("desktop-directory-control");
        }
        SendMessageW(edit, EM_SETLIMITTEXT, 512, 0);
        SendMessageW(
            edit,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            0,
        );
        settings_controls(hwnd, &ui);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, ui.icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, ui.icon as isize);
        ui.tray = if smoke && std::env::var("FUZE_RUST_CI_TRAY").as_deref() != Ok("1") {
            false
        } else {
            tray(hwnd, ui.icon, true)
        };
        ui.tray_registered = ui.tray;
        EnableWindow(GetDlgItem(hwnd, HIDE as i32), i32::from(ui.tray));
        SendMessageW(hwnd, WM_SIZE, 0, 0);
        SetTimer(hwnd, 1, 250, None);
        if !smoke {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        } else {
            SendMessageW(hwnd, WM_PAINT, 0, 0);
        }
        if managed {
            use std::io::Write;
            println!("FUZE_DESKTOP_READY_1");
            let _ = std::io::stdout().flush();
        }
        let mut msg: MSG = zeroed();
        let failed = loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result <= 0 {
                break result < 0;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        };
        shared.request(EXIT);
        if worker.is_finished() {
            let _ = worker.join();
        } // OS closes our Job handle on bounded process exit if IO is stuck.
        UnregisterClassW(class.as_ptr(), instance);
        if ui.icon_owned {
            DestroyIcon(ui.icon);
        }
        if failed {
            return Err("desktop-message-loop");
        }
        if smoke {
            if !ui.smoke_pass
                || !ui.shown.done
                || (std::env::var_os("FUZE_DESKTOP_PREVIEW").is_some() && !ui.preview_ok)
                || (ui.tray_registered && (!ui.tray_recovered || !ui.tray_removed))
                || (std::env::var_os("FUZE_CHAMPION_PREVIEW").is_some() && !ui.champion_preview_ok)
            {
                return Err("desktop-smoke-lifecycle");
            }
            println!(
                "{}",
                serde_json::json!({"desktop_ready":true,"pause_resume_exit":true,"game_access":false,"tray_registered":ui.tray_registered,"tray_recovered":ui.tray_recovered,"tray_removed":ui.tray_removed,"icon_loaded":ui.icon_owned,"preview_rendered":ui.preview_ok,"champion_preview":ui.champion_preview_ok})
            );
        }
    }
    Ok(())
}
