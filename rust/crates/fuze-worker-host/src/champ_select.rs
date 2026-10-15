//! Read-only champion overview; no LCU requests, account data, pick actions or per-paint IO.
use crate::Result;
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
    placement: crate::window_memory::Memory,
    view: Option<View>,
    dismissed: bool,
    selected: u32,
    downloads: bool,
    scroll: usize,
    assets: crate::champion_assets::Service,
    reply: crate::champion_assets::Reply,
}
pub struct Window {
    hwnd: HWND,
    state: Box<State>,
    topmost: bool,
}
impl Window {
    pub fn new(roots: Vec<fuze_data::Root>, offline: bool) -> Result<Self> {
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
                placement: crate::window_memory::Memory::new("champions", offline),
                view: None,
                dismissed: false,
                selected: 0,
                downloads: true,
                scroll: 0,
                assets: crate::champion_assets::Service::new(roots, offline)?,
                reply: Default::default(),
            });
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                wide("符择 · 选人参考").as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1280,
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
                    (1280 * d / 96).min(monitor.rcWork.right - monitor.rcWork.left),
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
            let changed = self.state.view != view || self.state.downloads != downloads;
            self.state.downloads = downloads;
            if entering {
                self.state.dismissed = false;
            }
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
                if self.state.view.is_some() && !self.state.dismissed {
                    SetTimer(self.hwnd, 2, 250, None);
                } else {
                    KillTimer(self.hwnd, 2);
                }
                InvalidateRect(self.hwnd, null(), 0);
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
    if let Some(v) = (*p).view.as_ref() {
        crate::i18n::set(v.version.split('@').next().unwrap_or("zh-CN"));
    }
    match msg {
        WM_TIMER if w == 2 => {
            if let Some(next) = (*p).assets.changed(&(*p).reply) {
                (*p).reply = next;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let px = (l as u16 as i16) as i32;
            let py = ((l >> 16) as u16 as i16) as i32;
            let d = GetDpiForWindow(hwnd).max(96) as i32;
            let x = |v| v * d / 96;
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r);
            let right = r.right * 3 / 5;
            if px >= right + x(12) && py >= x(92) && py < x(124) {
                if let Some(detail) = &(*p).reply.detail {
                    let tab =
                        ((px - right - x(12)) / ((r.right - right - x(28)) / 4).max(1)).min(3);
                    let prefixes = match tab {
                        1 => ["海克斯强化", "海克斯增幅", "Augments ·"],
                        2 => ["1–18", "1–18", "1–18"],
                        3 => ["符文页", "符文頁", "Runes:"],
                        _ => ["路线", "路線", "Build "],
                    };
                    (*p).scroll = detail
                        .lines
                        .iter()
                        .position(|l| prefixes.iter().any(|prefix| l.text.starts_with(prefix)))
                        .unwrap_or(0);
                    InvalidateRect(hwnd, null(), 0);
                }
                return 0;
            }
            if let Some(v) = &(*p).view {
                let hit = if py >= x(52) && py < x(154) && px >= x(20) && px < right - x(20) {
                    v.team
                        .get(((px - x(20)) / ((right - x(40)) / 5).max(1)) as usize)
                } else if py >= x(214) && py < r.bottom - x(46) && px >= x(20) && px < right - x(20)
                {
                    let rh = ((r.bottom - x(260)) / 5).max(x(62));
                    v.bench.get(
                        (((py - x(214)) / rh) * 4 + (px - x(20)) / ((right - x(40)) / 4).max(1))
                            as usize,
                    )
                } else {
                    None
                };
                if let Some(h) = hit.filter(|h| h.id > 0) {
                    (*p).selected = h.id;
                    (*p).scroll = 0;
                    (*p).request();
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((w >> 16) as u16 as i16) as i32;
            let count = (*p)
                .reply
                .detail
                .as_ref()
                .map(|d| d.lines.len())
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
            KillTimer(hwnd, 2);
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_ERASEBKGND => 1,
        WM_SIZE => {
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
                x: (1080 * d / 96).min(info.rcWork.right - info.rcWork.left),
                y: (650 * d / 96).min(info.rcWork.bottom - info.rcWork.top),
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
unsafe fn draw(hwnd: HWND, dc: HDC, state: &State) {
    use crate::desktop::{fill, rgb, text};
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let d = GetDpiForWindow(hwnd).max(96) as i32;
    let x = |v| v * d / 96;
    fill(dc, r, rgb(6, 17, 26));
    let full_right = r.right;
    r.right = r.right * 3 / 5;
    let Some(v) = &state.view else { return };
    let label = |s: &str, l, t, rr, b, size, color| {
        text(
            dc,
            s,
            RECT {
                left: l,
                top: t,
                right: rr,
                bottom: b,
            },
            size,
            color,
        )
    };
    let gold = rgb(205, 174, 108);
    let pale = rgb(222, 233, 234);
    let dim = rgb(138, 167, 180);
    label(
        "我方阵容",
        x(20),
        x(12),
        r.right - x(20),
        x(44),
        x(22),
        gold,
    );
    let width = (r.right - x(40)) / 5;
    for (i, h) in v.team.iter().enumerate() {
        let l = x(20) + i as i32 * width;
        fill(
            dc,
            RECT {
                left: l,
                top: x(52),
                right: l + width - x(8),
                bottom: x(154),
            },
            if v.local_slot == Some(i) {
                rgb(37, 69, 67)
            } else {
                rgb(24, 42, 54)
            },
        );
        label(
            if v.local_slot == Some(i) {
                "你 · 已选择"
            } else {
                "队友 · 已选择"
            },
            l + x(10),
            x(59),
            l + width - x(12),
            x(85),
            x(12),
            gold,
        );
        icon(dc, state, &h.icon, l + x(8), x(88), x(28));
        label(
            &h.name,
            l + x(42),
            x(86),
            l + width - x(12),
            x(118),
            x(18),
            pale,
        );
        label(
            &rate(h.win, "胜率"),
            l + x(10),
            x(119),
            l + width - x(12),
            x(145),
            x(13),
            dim,
        );
    }
    label(
        "备选英雄 · 按整体胜率排序",
        x(20),
        x(168),
        r.right - x(20),
        x(202),
        x(20),
        gold,
    );
    if v.bench.is_empty() {
        label(
            "当前没有可选备选英雄；客户端变化后自动刷新",
            x(20),
            x(214),
            r.right - x(20),
            x(250),
            x(15),
            dim,
        );
    }
    let width = (r.right - x(40)) / 4;
    let row_height = ((r.bottom - x(260)) / 5).max(x(62));
    for (i, h) in v.bench.iter().enumerate() {
        let l = x(20) + (i % 4) as i32 * width;
        let t = x(214) + (i / 4) as i32 * row_height;
        fill(
            dc,
            RECT {
                left: l,
                top: t,
                right: l + width - x(8),
                bottom: t + row_height - x(8),
            },
            rgb(13, 30, 41),
        );
        icon(dc, state, &h.icon, l + x(8), t + x(6), x(26));
        label(
            &h.name,
            l + x(39),
            t + x(5),
            l + width - x(12),
            t + x(31),
            x(17),
            pale,
        );
        label(
            &format!("{} · {}", rate(h.win, "胜"), rate(h.pick, "选")),
            l + x(10),
            t + x(31),
            l + width - x(12),
            t + x(55),
            x(12),
            rgb(120, 214, 199),
        );
    }
    let left = r.right + x(12);
    label(
        "英雄详情 · 点击左侧卡片查看",
        left,
        x(12),
        full_right - x(16),
        x(44),
        x(20),
        gold,
    );
    if let Some(detail) = &state.reply.detail {
        label(
            &detail.title,
            left,
            x(52),
            full_right - x(16),
            x(89),
            x(22),
            pale,
        );
        let tab_width = (full_right - left - x(16)) / 4;
        for (i, name) in ["出装", "海克斯", "加点", "符文页"].iter().enumerate() {
            let l = left + i as i32 * tab_width;
            fill(
                dc,
                RECT {
                    left: l,
                    top: x(92),
                    right: l + tab_width - x(4),
                    bottom: x(124),
                },
                rgb(24, 42, 54),
            );
            label(
                name,
                l + x(8),
                x(94),
                l + tab_width - x(4),
                x(122),
                x(14),
                gold,
            );
        }
        let mut top = x(136);
        let mut index = state.scroll;
        while let Some(line) = detail.lines.get(index) {
            let width = (full_right - left - x(16)).max(1);
            let chars = (width
                / x(if v.version.starts_with("en-US@") {
                    8
                } else {
                    14
                })
                .max(1))
            .max(12) as usize;
            let wrapped = wrap(&line.text, chars);
            let icons = !line.icons.is_empty();
            let height = (wrapped.len() as i32) * 23 + if icons { 37 } else { 7 };
            if top + x(height) > r.bottom - x(44) {
                break;
            }
            if icons {
                for (i, url) in line.icons.iter().take(6).enumerate() {
                    icon(dc, state, url, left + i as i32 * x(30), top, x(26));
                }
                top += x(30);
            }
            for chunk in wrapped {
                label(
                    &chunk,
                    left,
                    top,
                    full_right - x(16),
                    top + x(23),
                    x(14),
                    if icons { pale } else { dim },
                );
                top += x(23);
            }
            top += x(7);
            index += 1;
        }
        label(
            &format!(
                "{} · {} / {}",
                crate::i18n::text("滚轮浏览详情"),
                state.scroll + 1,
                detail.lines.len()
            ),
            left,
            r.bottom - x(37),
            full_right - x(16),
            r.bottom - x(8),
            x(12),
            gold,
        );
    } else {
        label(
            if state.selected == 0 {
                "等待选择英雄"
            } else if state.reply.error.is_some() {
                "本地详情不可用；可暂停后更新数据"
            } else {
                "正在读取本地详情…"
            },
            left,
            x(105),
            full_right - x(16),
            x(140),
            x(15),
            dim,
        );
    }
    label(
        &format!(
            "{} · {}",
            v.version,
            crate::i18n::text("仅供参考，请在游戏中选择")
        ),
        x(20),
        r.bottom - x(37),
        r.right - x(20),
        r.bottom - x(8),
        x(12),
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
    fn request(&mut self) {
        self.reply = Default::default();
        self.assets.request(
            self.view
                .as_ref()
                .filter(|_| self.selected > 0 && !self.dismissed)
                .map(|v| crate::champion_assets::Request {
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
    if let Some(pic) = state.reply.icons.get(url) {
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = 48;
        info.bmiHeader.biHeight = -48;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
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
