//! Read-only champion overview; no LCU requests, account data, pick actions or per-paint IO.
use crate::Result;
#[path = "champ_presentation.rs"]
mod presentation;
use fuze_lcu::context::Selection;
use std::{
    collections::HashMap,
    mem::zeroed,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::*, WindowsAndMessaging::*},
};
#[derive(Clone, Debug, PartialEq)]
pub struct Hero {
    pub id: u32,
    pub name: String,
    pub icon: String,
    pub win: Option<f64>,
    pub pick: Option<f64>,
    pub games: Option<u64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub team: Vec<Hero>,
    pub local_slot: Option<usize>,
    pub bench: Vec<Hero>,
    pub version: String,
}
pub struct Catalog {
    heroes: HashMap<u32, Hero>,
    version: String,
}
impl Catalog {
    pub fn playing(&self, id: u32) -> View {
        self.view(Selection {
            team: [id, 0, 0, 0, 0],
            local_slot: Some(0),
            bench: [0; 20],
            count: 0,
        })
    }
    pub fn identity(&self) -> &str {
        &self.version
    }
    pub fn load(roots: &[fuze_data::Root], locale: fuze_data::Locale) -> Result<Self> {
        let data = fuze_data::prepare(roots, locale, fuze_data::Scope::Champions)?;
        Self::parse(
            data.document("champions.json").ok_or("champions-missing")?,
            data.identity(),
        )
    }
    fn parse(value: &serde_json::Value, version: String) -> Result<Self> {
        let rows = value["data"]
            .as_array()
            .filter(|a| a.len() <= 1000)
            .ok_or("champions-format")?;
        let mut heroes = HashMap::new();
        for r in rows {
            let id = r["id"]
                .as_u64()
                .filter(|n| *n > 0 && *n <= u32::MAX as u64)
                .ok_or("champions-id")? as u32;
            let name = r["name"]
                .as_str()
                .filter(|s| !s.is_empty() && s.chars().count() <= 64)
                .ok_or("champions-name")?;
            let rate = |key| {
                r["stats"][key]
                    .as_f64()
                    .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            };
            if heroes
                .insert(
                    id,
                    Hero {
                        id,
                        name: name.chars().filter(|c| !c.is_control()).collect(),
                        icon: r["iconUrl"]
                            .as_str()
                            .filter(|s| s.len() < 200)
                            .unwrap_or("")
                            .into(),
                        win: rate("winRate"),
                        pick: rate("pickRate"),
                        games: r["stats"]["games"].as_u64(),
                    },
                )
                .is_some()
            {
                return Err("champions-duplicate");
            }
        }
        Ok(Self { heroes, version })
    }
    pub fn view(&self, selection: Selection) -> View {
        let hero = |id| {
            self.heroes.get(&id).cloned().unwrap_or(Hero {
                id,
                name: if id == 0 {
                    "等待选择".into()
                } else {
                    format!("英雄 {id}")
                },
                icon: String::new(),
                win: None,
                pick: None,
                games: None,
            })
        };
        let mut bench: Vec<_> = selection.bench[..selection.count]
            .iter()
            .copied()
            .map(hero)
            .collect();
        bench.sort_by(|a, b| {
            b.win
                .partial_cmp(&a.win)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.id.cmp(&b.id))
        });
        View {
            team: selection.team.into_iter().map(hero).collect(),
            local_slot: selection.local_slot,
            bench,
            version: self.version.clone(),
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    crate::i18n::text(s).encode_utf16().chain(Some(0)).collect()
}
struct State {
    side: bool,
    stage_service: crate::stage_reference::Service,
    stage_reply: crate::stage_reference::Reply,
    stage_panel: bool,
    stage: usize,
    stage_scroll: usize,
    stage_allowed: bool,
    stage_key: Option<(u32, String)>,
    offline: bool,
    placement: crate::window_memory::Memory,
    view: Option<View>,
    dismissed: bool,
    selected: u32,
    downloads: bool,
    scroll: usize,
    page_scroll: i32,
    active_tab: usize,
    assets: crate::champion_assets::Service,
    reply: crate::champion_assets::Reply,
}
pub struct Window {
    hwnd: HWND,
    state: Box<State>,
    topmost: bool,
}
impl Window {
    pub fn dismissed(&self) -> bool {
        self.state.dismissed
    }
    pub fn new(roots: Vec<fuze_data::Root>, offline: bool) -> Result<Self> {
        Self::create(roots, offline, false)
    }
    pub fn side(roots: Vec<fuze_data::Root>, offline: bool) -> Result<Self> {
        Self::create(roots, offline, true)
    }
    fn create(roots: Vec<fuze_data::Root>, offline: bool, side: bool) -> Result<Self> {
        unsafe {
            let class = wide("FuzeRustChampionOverview");
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..zeroed()
            };
            RegisterClassW(&wc);
            let mut state = Box::new(State {
                side,
                stage_service: crate::stage_reference::Service::new(roots.clone(), offline)?,
                stage_reply: Default::default(),
                stage_panel: false,
                stage: 0,
                stage_scroll: 0,
                stage_allowed: false,
                stage_key: None,
                offline,
                placement: crate::window_memory::Memory::new(
                    if side {
                        "augment-reference"
                    } else {
                        "champions"
                    },
                    offline,
                ),
                view: None,
                dismissed: false,
                selected: 0,
                downloads: true,
                scroll: 0,
                page_scroll: 0,
                active_tab: 0,
                assets: crate::champion_assets::Service::new(roots, offline)?,
                reply: Default::default(),
            });
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                wide(if side {
                    "符择 · 局内海克斯参考"
                } else {
                    "符择 · 选人参考"
                })
                .as_ptr(),
                WS_OVERLAPPED | WS_SYSMENU | WS_THICKFRAME | if side { 0 } else { WS_VSCROLL },
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                if side { 460 } else { 480 },
                780,
                null_mut(),
                null_mut(),
                instance,
                (&mut *state as *mut State).cast(),
            );
            if hwnd.is_null() {
                return Err("champions-window");
            }
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            ) != 0
            {
                let d = GetDpiForWindow(hwnd).max(96) as i32;
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    monitor.rcWork.left,
                    monitor.rcWork.top,
                    ((if side { 460 } else { 480 }) * d / 96)
                        .min(monitor.rcWork.right - monitor.rcWork.left),
                    (780 * d / 96).min(monitor.rcWork.bottom - monitor.rcWork.top),
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            state.placement.restore(hwnd);
            Ok(Self {
                hwnd,
                state,
                topmost: false,
            })
        }
    }
    pub fn update(&mut self, view: Option<View>, topmost: bool, hidden: bool, downloads: bool) {
        unsafe {
            let entering = self.state.view.is_none() && view.is_some();
            if entering {
                self.state.dismissed = false;
            }
            let allowed = view.is_some() && !hidden && !self.state.dismissed;
            if self.state.stage_allowed != allowed {
                self.state.stage_allowed = allowed;
                self.state.stage_key = None;
                if !allowed {
                    self.state.stage_service.request(None);
                    self.state.stage_reply = Default::default();
                }
            }
            let changed = self.state.view != view || self.state.downloads != downloads;
            self.state.downloads = downloads;
            if changed {
                let previous = self
                    .state
                    .view
                    .as_ref()
                    .and_then(|v| v.local_slot.and_then(|i| v.team.get(i)))
                    .map(|h| h.id);
                let current = view
                    .as_ref()
                    .and_then(|v| v.local_slot.and_then(|i| v.team.get(i)))
                    .map(|h| h.id);
                if previous != current
                    || view.as_ref().is_some_and(|v| {
                        !v.team
                            .iter()
                            .chain(&v.bench)
                            .any(|h| h.id == self.state.selected)
                    })
                {
                    self.state.selected = current.unwrap_or(0);
                    self.state.scroll = 0;
                }
                self.state.view = view;
                self.state.request();
                if !self.state.side {
                    presentation::scroll(self.hwnd, &mut self.state, 0);
                }
                if self.state.view.is_some() && !self.state.dismissed {
                    SetTimer(self.hwnd, 2, 250, None);
                } else {
                    KillTimer(self.hwnd, 2);
                }
                InvalidateRect(self.hwnd, null(), 0);
            }
            if allowed && self.state.stage_panel && self.state.stage_key.is_none() {
                self.state.stage_request(false);
            }
            if self.topmost != topmost {
                SetWindowPos(
                    self.hwnd,
                    if topmost {
                        HWND_TOPMOST
                    } else {
                        HWND_NOTOPMOST
                    },
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                self.topmost = topmost;
            }
            if self.state.view.is_none() || self.state.dismissed || hidden {
                ShowWindow(self.hwnd, SW_HIDE);
            } else if entering {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }
    pub fn detail_ready(&self) -> bool {
        self.state.reply.detail.is_some()
    }
    /// Explicit offline visual review; never inspects user caches or sends game/input events.
    pub fn review(&mut self, directory: &std::path::Path) -> Result<()> {
        if !self.state.offline {
            return Err("fixture-only");
        }
        std::fs::create_dir_all(directory).map_err(|_| "fixture-directory")?;
        if let Some(root) = std::env::var_os("FUZE_POPUP_REVIEW_ART") {
            let root = std::path::PathBuf::from(root);
            let urls: Vec<_> = self
                .state
                .view
                .iter()
                .flat_map(|v| v.team.iter().chain(&v.bench).map(|h| h.icon.clone()))
                .chain(
                    self.state
                        .reply
                        .detail
                        .iter()
                        .flat_map(|d| d.lines.iter().flat_map(|l| l.icons.clone())),
                )
                .take(96)
                .collect();
            for url in urls {
                let Ok(relative) = crate::champion_assets::icon_path(&url) else {
                    continue;
                };
                let file = root.join(relative.trim_start_matches('/'));
                if std::fs::metadata(&file).is_ok_and(|m| m.len() <= 512 * 1024) {
                    if let Ok(bytes) = std::fs::read(file) {
                        if let Ok(image) = crate::champion_assets::decode(&bytes) {
                            self.state
                                .reply
                                .icons
                                .insert(url, std::sync::Arc::new(image));
                        }
                    }
                }
            }
        }
        self.render(&directory.join("champion.png"))?;
        let scroll = self.state.page_scroll;
        let tab = self.state.active_tab;
        unsafe {
            self.state.page_scroll = presentation::layout(self.hwnd, &self.state).hero_top;
            self.render(&directory.join("champion-detail.png"))?;
            // The same callback used by the window validates content coordinates after scrolling.
            let layout = presentation::layout(self.hwnd, &self.state);
            let x = (16 + (layout.width - 32) / 2 + 8) * layout.dpi / 96;
            let y = (40 + layout.tabs_top - self.state.page_scroll + 8) * layout.dpi / 96;
            presentation::click(self.hwnd, &mut self.state, x, y);
            if self.state.active_tab != 1 {
                return Err("fixture-tab-hit");
            }
            self.render(&directory.join("champion-build.png"))?;
            self.state.active_tab = tab;
            self.state.page_scroll = scroll;
            let mut bounds: RECT = zeroed();
            GetWindowRect(self.hwnd, &mut bounds);
            SetWindowPos(
                self.hwnd,
                null_mut(),
                0,
                0,
                380,
                560,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            self.render(&directory.join("champion-compact.png"))?;
            SetWindowPos(
                self.hwnd,
                null_mut(),
                0,
                0,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.render_side_fixture(&directory.join("side.png"))?;
        self.render_stage_fixture(&directory.join("stage.png"))?;
        Ok(())
    }
    pub fn render_side_fixture(&mut self, path: &std::path::Path) -> Result<()> {
        if !self.state.offline {
            return Err("fixture-only");
        }
        self.state.side = true;
        self.state.scroll = 0;
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                0,
                0,
                460,
                780,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let result = self.render(path);
        self.state.side = false;
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                0,
                0,
                480,
                780,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        result
    }
    pub fn render_stage_fixture(&mut self, path: &std::path::Path) -> Result<()> {
        if !self.state.offline {
            return Err("stage-fixture-only");
        }
        let previous = std::mem::take(&mut self.state.stage_reply);
        let panel = self.state.stage_panel;
        let active_tab = self.state.active_tab;
        let page_scroll = self.state.page_scroll;
        let stage = self.state.stage;
        let mut rows: [Vec<crate::stage_reference::Row>; 5] = Default::default();
        rows[1] = (1..=6)
            .map(|id| crate::stage_reference::Row {
                id,
                win: 0.62 - id as f64 * 0.01,
                pick: 0.13,
                samples: 400 + id as u64 * 100,
            })
            .collect();
        self.state.stage_reply = crate::stage_reference::Reply {
            snapshot: Some(std::sync::Arc::new(crate::stage_reference::Snapshot {
                schema: 1,
                champion: self.state.selected,
                patch: "16.20".into(),
                date: "2026-10-15".into(),
                data_path: "data/16.20-20261015-abcdef012345".into(),
                fetched: crate::stage_reference::now(),
                rows,
            })),
            issue: Some("stage-fixture"),
            cached: true,
            ..Default::default()
        };
        self.state.stage_panel = true;
        self.state.active_tab = 0;
        self.state.page_scroll = unsafe { presentation::layout(self.hwnd, &self.state).hero_top };
        self.state.stage = 1;
        let result = self.render(path);
        self.state.stage_reply = previous;
        self.state.stage_panel = panel;
        self.state.active_tab = active_tab;
        self.state.page_scroll = page_scroll;
        self.state.stage = stage;
        result
    }
    pub fn render(&self, path: &std::path::Path) -> Result<()> {
        unsafe {
            let mut r: RECT = zeroed();
            GetClientRect(self.hwnd, &mut r);
            let dc = GetDC(self.hwnd);
            let mem = CreateCompatibleDC(dc);
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = r.right;
            info.bmiHeader.biHeight = -r.bottom;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits = null_mut();
            let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bmp.is_null() || mem.is_null() {
                if !bmp.is_null() {
                    DeleteObject(bmp);
                }
                if !mem.is_null() {
                    DeleteDC(mem);
                }
                ReleaseDC(self.hwnd, dc);
                return Err("champions-render");
            }
            let old = SelectObject(mem, bmp);
            draw(self.hwnd, mem, &self.state);
            let pixels =
                std::slice::from_raw_parts(bits as *const u8, (r.right * r.bottom * 4) as usize);
            let mut rgba = pixels.to_vec();
            for p in rgba.chunks_exact_mut(4) {
                p.swap(0, 2);
                p[3] = 255;
            }
            let result = image::save_buffer(
                path,
                &rgba,
                r.right as u32,
                r.bottom as u32,
                image::ColorType::Rgba8,
            )
            .map_err(|_| "champions-render");
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(self.hwnd, dc);
            result
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            KillTimer(self.hwnd, 2);
            DestroyWindow(self.hwnd);
        }
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(l as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if p.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    match msg {
        WM_TIMER if w == 2 => {
            if (*p).stage_panel {
                if let Some(next) = (*p).stage_service.changed((*p).stage_reply.serial) {
                    (*p).stage_reply = next;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            if let Some(next) = (*p).assets.changed(&(*p).reply) {
                (*p).reply = next;
                if !(*p).side {
                    presentation::scroll(hwnd, &mut *p, 0);
                }
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            presentation::click(
                hwnd,
                &mut *p,
                (l as u16 as i16) as i32,
                ((l >> 16) as u16 as i16) as i32,
            );
            0
        }
        WM_VSCROLL if !(*p).side => {
            let mut info: SCROLLINFO = zeroed();
            info.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
            info.fMask = SIF_ALL;
            GetScrollInfo(hwnd, SB_VERT, &mut info);
            let delta = match w as u16 as i32 {
                SB_LINEUP => -32,
                SB_LINEDOWN => 32,
                SB_PAGEUP => -(info.nPage as i32),
                SB_PAGEDOWN => info.nPage as i32,
                SB_THUMBTRACK => info.nTrackPos - (*p).page_scroll,
                _ => 0,
            };
            presentation::scroll(hwnd, &mut *p, delta);
            0
        }
        WM_NCHITTEST => {
            let hit = DefWindowProcW(hwnd, msg, w, l);
            if hit == HTCLIENT as isize {
                let mut pt = POINT {
                    x: (l as u16 as i16) as i32,
                    y: ((l >> 16) as u16 as i16) as i32,
                };
                ScreenToClient(hwnd, &mut pt);
                let view = presentation::layout(hwnd, &*p);
                if pt.y < 40 * view.dpi / 96 && pt.x < (view.width - 40) * view.dpi / 96 {
                    return HTCAPTION as isize;
                }
            }
            hit
        }
        WM_MOUSEWHEEL => {
            let delta = ((w >> 16) as u16 as i16) as i32;
            if !(*p).side {
                let layout = presentation::layout(hwnd, &*p);
                let mut point = POINT {
                    x: (l as u16 as i16) as i32,
                    y: ((l >> 16) as u16 as i16) as i32,
                };
                ScreenToClient(hwnd, &mut point);
                let y = point.y * 96 / layout.dpi - 40 + (*p).page_scroll;
                let stage_rows =
                    (*p).stage_panel && y >= layout.detail_top + 120 && y < layout.detail_top + 488;
                if !stage_rows {
                    presentation::scroll(hwnd, &mut *p, if delta > 0 { -72 } else { 72 });
                    return 0;
                }
            }
            if (*p).stage_panel {
                let count = (*p)
                    .stage_reply
                    .snapshot
                    .as_ref()
                    .map(|s| s.visible((*p).stage).len())
                    .unwrap_or(0);
                (*p).stage_scroll = if delta > 0 {
                    (*p).stage_scroll.saturating_sub(3)
                } else {
                    ((*p).stage_scroll + 3).min(count.saturating_sub(1))
                };
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            let count = (*p)
                .reply
                .detail
                .as_ref()
                .map(|d| {
                    if (*p).side {
                        side_lines(d)
                            .iter()
                            .filter(|row| !presentation::source_line(&row.text))
                            .count()
                    } else {
                        d.lines.len()
                    }
                })
                .unwrap_or(0);
            (*p).scroll = if delta > 0 {
                (*p).scroll.saturating_sub(3)
            } else {
                ((*p).scroll + 3).min(count.saturating_sub(1))
            };
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_EXITSIZEMOVE => {
            (*p).placement.remember(hwnd);
            0
        }
        WM_DISPLAYCHANGE => {
            (*p).placement.recover(hwnd);
            (*p).placement.remember(hwnd);
            0
        }
        WM_CLOSE => {
            (*p).placement.remember(hwnd);
            (*p).dismissed = true;
            (*p).assets.request(None);
            (*p).stage_allowed = false;
            (*p).stage_key = None;
            (*p).stage_service.request(None);
            KillTimer(hwnd, 2);
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            if !(*p).side {
                presentation::scroll(hwnd, &mut *p, 0);
            }
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
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            0
        }
        WM_GETMINMAXINFO => {
            let m = &mut *(l as *mut MINMAXINFO);
            let d = GetDpiForWindow(hwnd).max(96) as i32;
            let mut info: MONITORINFO = zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info);
            m.ptMinTrackSize = POINT {
                x: ((if (*p).side { 360 } else { 380 }) * d / 96)
                    .min(info.rcWork.right - info.rcWork.left),
                y: ((if (*p).side { 400 } else { 440 }) * d / 96)
                    .min(info.rcWork.bottom - info.rcWork.top),
            };
            0
        }
        WM_PAINT => {
            let mut ps = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            draw(hwnd, dc, &*p);
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
fn side_lines(detail: &fuze_data::champion_detail::Detail) -> &[fuze_data::champion_detail::Line] {
    detail
        .lines
        .iter()
        .position(|l| {
            ["海克斯强化", "海克斯增幅", "Augments ·"]
                .iter()
                .any(|p| l.text.starts_with(p))
        })
        .map(|i| &detail.lines[i + 1..])
        .unwrap_or(&[])
}
unsafe fn draw_side(hwnd: HWND, dc: HDC, state: &State) {
    use crate::desktop::{fill, rgb, text};
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
    let x = |v| v * dpi / 96;
    fill(dc, r, rgb(17, 30, 46));
    presentation::header(hwnd, dc, state);
    let line = |s: &str, top, size, color| {
        text(
            dc,
            s,
            RECT {
                left: x(16),
                top,
                right: r.right - x(16),
                bottom: top + x(30),
            },
            x(size),
            color,
        )
    };
    line(
        "实际领取轮次未确认；与三卡识别独立",
        x(48),
        12,
        rgb(138, 167, 180),
    );
    if let Some(detail) = &state.reply.detail {
        line(&detail.title, x(82), 17, rgb(222, 233, 234));
        let mut top = x(125);
        for row in side_lines(detail)
            .iter()
            .filter(|row| !presentation::source_line(&row.text))
            .skip(state.scroll)
        {
            let chunks = wrap(
                &row.text,
                ((r.right - x(70))
                    / x(if crate::i18n::locale() == "en-US" {
                        9
                    } else {
                        16
                    }))
                .max(8) as usize,
            );
            let height = x((chunks.len() as i32 * 23 + 18).max(63));
            if top + height > r.bottom - x(52) {
                break;
            }
            fill(
                dc,
                RECT {
                    left: x(16),
                    top,
                    right: r.right - x(16),
                    bottom: top + height - x(8),
                },
                rgb(43, 64, 87),
            );
            fill(
                dc,
                RECT {
                    left: x(17),
                    top: top + 1,
                    right: r.right - x(17),
                    bottom: top + height - x(8) - 1,
                },
                rgb(26, 43, 62),
            );
            let left = if let Some(url) = row.icons.first() {
                icon(dc, state, url, x(24), top + x(8), x(36));
                x(70)
            } else {
                x(24)
            };
            for (i, chunk) in chunks.iter().enumerate() {
                text(
                    dc,
                    chunk,
                    RECT {
                        left,
                        top: top + x(i as i32 * 23 + 5),
                        right: r.right - x(24),
                        bottom: top + x(i as i32 * 23 + 28),
                    },
                    x(if row.icons.is_empty() { 12 } else { 15 }),
                    rgb(176, 211, 215),
                );
            }
            top += height;
        }
        if side_lines(detail).is_empty() {
            line(
                "本地详情不可用；可暂停后更新数据",
                x(130),
                14,
                rgb(198, 171, 100),
            );
        }
    } else {
        line(
            if state.reply.error.is_some() {
                "本地详情不可用；可暂停后更新数据"
            } else {
                "正在读取本地详情…"
            },
            x(100),
            15,
            rgb(176, 211, 215),
        );
    }
    if let Some(view) = &state.view {
        line(&view.version, r.bottom - x(50), 12, rgb(138, 167, 180));
    }
    line("滚轮浏览详情", r.bottom - x(28), 12, rgb(205, 174, 108));
}
unsafe fn draw(hwnd: HWND, dc: HDC, state: &State) {
    if state.side {
        draw_side(hwnd, dc, state);
    } else {
        presentation::draw(hwnd, dc, state);
    }
}
unsafe fn draw_stage(dc: HDC, state: &State, r: RECT, d: i32) {
    use crate::desktop::{fill, rgb, text};
    let x = |n| n * d / 96;
    let t = crate::i18n::text;
    let label = |s: &str, top: i32, size: i32, color| {
        text(
            dc,
            s,
            RECT {
                left: r.left,
                top,
                right: r.right,
                bottom: top + x(25),
            },
            x(size),
            color,
        )
    };
    let gold = rgb(205, 174, 108);
    let dim = rgb(138, 167, 180);
    let pale = rgb(222, 233, 234);
    let width = (r.right - r.left) / 6;
    for (i, name) in ["整体", "第1次", "第2次", "第3次", "第4次", "刷新"]
        .iter()
        .enumerate()
    {
        let l = r.left + i as i32 * width;
        fill(
            dc,
            RECT {
                left: l,
                top: r.top,
                right: l + width - x(4),
                bottom: r.top + x(32),
            },
            if state.stage == i {
                rgb(37, 69, 67)
            } else {
                rgb(24, 42, 54)
            },
        );
        text(
            dc,
            name,
            RECT {
                left: l + x(4),
                top: r.top + x(4),
                right: l + width - x(4),
                bottom: r.top + x(29),
            },
            x(13),
            gold,
        );
    }
    label("手动参考 · 不改变三卡整体胜率推荐", r.top + x(40), 13, gold);
    label(
        "阶段表示选择次序；不是已确认的领取轮次",
        r.top + x(65),
        12,
        dim,
    );
    let Some(s) = &state.stage_reply.snapshot else {
        label(
            if state.stage_reply.issue == Some("stage-busy") {
                "请求较频繁，请稍后刷新"
            } else if state.stage_reply.issue.is_some() {
                "参考数据暂不可用，可稍后刷新"
            } else {
                "正在读取阶段参考…"
            },
            r.top + x(105),
            14,
            dim,
        );
        return;
    };
    label(
        &format!("ARAMKit · {} · {}", s.patch, s.date),
        r.top + x(96),
        14,
        pale,
    );
    label(
        "数据区服未确认 · 仅展示样本量 ≥ 200",
        r.top + x(121),
        12,
        dim,
    );
    let local = state
        .view
        .as_ref()
        .and_then(|v| v.version.split('@').nth(1))
        .unwrap_or("");
    let reference = local.split('.').take(2).collect::<Vec<_>>().join(".");
    let notice = if state.stage_reply.issue == Some("stage-fixture") {
        "离线示例数据"
    } else if state.stage_reply.issue == Some("stage-loading") {
        "正在刷新，继续显示已有数据"
    } else if reference != s.patch {
        "与本地数据补丁不同，仅作参考"
    } else if state.stage_reply.issue.is_some() {
        "刷新未完成，保留可用数据"
    } else if s.stale(crate::stage_reference::now()) {
        "缓存已过期，可手动刷新"
    } else if state.stage_reply.cached {
        "使用本地缓存"
    } else {
        "参考数据已更新"
    };
    label(notice, r.top + x(146), 13, gold);
    let rows = s.visible(state.stage);
    let mut y = r.top + x(182);
    if rows.is_empty() {
        label("此阶段暂无足够样本", y, 14, dim);
    }
    for row in rows.iter().skip(state.stage_scroll) {
        if y + x(58) > r.bottom {
            break;
        }
        let name = state
            .stage_reply
            .names
            .get(&row.id)
            .cloned()
            .unwrap_or_else(|| format!("#{}", row.id));
        label(&name, y, 16, pale);
        label(
            &format!(
                "{} {:.1}%  ·  {} {:.1}%  ·  {} {}",
                t("胜率"),
                row.win * 100.,
                t("选取率"),
                row.pick * 100.,
                t("样本"),
                row.samples
            ),
            y + x(26),
            13,
            gold,
        );
        y += x(60);
    }
    label(
        &format!(
            "{} · {} / {}",
            t("滚轮浏览详情"),
            if rows.is_empty() {
                0
            } else {
                state.stage_scroll + 1
            },
            rows.len()
        ),
        r.bottom + x(8),
        12,
        dim,
    );
}
fn rate(value: Option<f64>, name: &str) -> String {
    let name = crate::i18n::text(name);
    value
        .map(|v| format!("{name} {:.1}%", v * 100.0))
        .unwrap_or(format!("{name} --"))
}
fn wrap(text: &str, columns: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    let mut lines = Vec::new();
    while start < chars.len() && lines.len() < 3 {
        let mut end = (start + columns).min(chars.len());
        if end < chars.len() {
            if let Some(space) = (start + columns / 2..end)
                .rev()
                .find(|i| chars[*i].is_whitespace())
            {
                end = space;
            }
        }
        let mut line: String = chars[start..end].iter().collect();
        if lines.len() == 2 && end < chars.len() {
            line.push('…');
        }
        lines.push(line);
        start = end;
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
    }
    lines
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sort_missing_last_and_keep_team_order() {
        let c=Catalog::parse(&serde_json::json!({"data":[{"id":1,"name":"A","stats":{"winRate":0.4}},{"id":2,"name":"B","stats":{"winRate":0.6}},{"id":3,"name":"C","stats":{}}]}),"test".into()).unwrap();
        let mut s = Selection {
            team: [1, 0, 0, 0, 0],
            local_slot: Some(0),
            bench: [0; 20],
            count: 3,
        };
        s.bench[..3].copy_from_slice(&[1, 3, 2]);
        let v = c.view(s);
        assert_eq!(v.team[0].id, 1);
        assert_eq!(
            v.bench.iter().map(|h| h.id).collect::<Vec<_>>(),
            vec![2, 1, 3]
        );
    }
}

impl State {
    fn stage_request(&mut self, refresh: bool) {
        if !self.stage_panel || !self.stage_allowed || self.selected == 0 {
            self.stage_service.request(None);
            self.stage_reply = Default::default();
            self.stage_key = None;
            return;
        }
        let Some(v) = &self.view else { return };
        let key = (self.selected, v.version.clone());
        if !refresh && self.stage_key.as_ref() == Some(&key) {
            return;
        }
        self.stage_key = Some(key);
        self.stage_scroll = 0;
        if refresh
            && self
                .stage_reply
                .snapshot
                .as_ref()
                .is_some_and(|s| s.champion == self.selected)
        {
            self.stage_reply.issue = Some("stage-loading");
        } else {
            self.stage_reply = Default::default();
        }
        let locale = match v.version.split('@').next() {
            Some("en-US") => fuze_data::Locale::EnUs,
            Some("zh-TW") => fuze_data::Locale::ZhTw,
            _ => fuze_data::Locale::ZhCn,
        };
        self.stage_service
            .request(Some(crate::stage_reference::Request {
                id: self.selected,
                locale,
                refresh,
            }));
    }

    fn request(&mut self) {
        self.stage_request(false);
        self.reply = Default::default();
        self.assets.request(
            self.view
                .as_ref()
                .filter(|_| self.selected > 0 && !self.dismissed)
                .map(|v| crate::champion_assets::Request {
                    presentation: None,
                    download: self.downloads,
                    version: v.version.clone(),
                    id: self.selected,
                    icons: v
                        .team
                        .iter()
                        .chain(&v.bench)
                        .map(|h| h.icon.clone())
                        .filter(|s| !s.is_empty())
                        .collect(),
                }),
        );
    }
}
unsafe fn icon(dc: HDC, state: &State, url: &str, left: i32, top: i32, size: i32) {
    draw_icon(dc, &state.reply, url, left, top, size);
}
pub(crate) unsafe fn draw_icon(
    dc: HDC,
    reply: &crate::champion_assets::Reply,
    url: &str,
    left: i32,
    top: i32,
    size: i32,
) {
    if let Some(pic) = reply.icons.get(url) {
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = 48;
        info.bmiHeader.biHeight = -48;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let stretch = SetStretchBltMode(dc, HALFTONE);
        SetBrushOrgEx(dc, 0, 0, null_mut());
        StretchDIBits(
            dc,
            left,
            top,
            size,
            size,
            0,
            0,
            48,
            48,
            pic.bgra.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        SetStretchBltMode(dc, stretch);
    } else {
        use crate::desktop::{fill, rgb, text};
        let r = RECT {
            left,
            top,
            right: left + size,
            bottom: top + size,
        };
        fill(dc, r, rgb(24, 42, 54));
        text(dc, "◇", r, size * 2 / 3, rgb(138, 167, 180));
    }
}
