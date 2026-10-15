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
    s.encode_utf16().chain(Some(0)).collect()
}
struct State {
    view: Option<View>,
    dismissed: bool,
}
pub struct Window {
    hwnd: HWND,
    state: Box<State>,
    topmost: bool,
}
impl Window {
    pub fn new() -> Result<Self> {
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
                view: None,
                dismissed: false,
            });
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                wide("符择 · 选人参考").as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                820,
                720,
                null_mut(),
                null_mut(),
                instance,
                (&mut *state as *mut State).cast(),
            );
            if hwnd.is_null() {
                return Err("champions-window");
            }
            Ok(Self {
                hwnd,
                state,
                topmost: false,
            })
        }
    }
    pub fn update(&mut self, view: Option<View>, topmost: bool, hidden: bool) {
        unsafe {
            let entering = self.state.view.is_none() && view.is_some();
            let changed = self.state.view != view;
            if entering {
                self.state.dismissed = false;
            }
            if changed {
                self.state.view = view;
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
        WM_CLOSE => {
            (*p).dismissed = true;
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
            m.ptMinTrackSize = POINT {
                x: 760 * d / 96,
                y: 650 * d / 96,
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
        label(
            &h.name,
            l + x(10),
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
        label(
            &h.name,
            l + x(10),
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
    label(
        &format!("数据 {} · 仅供参考，请在游戏中选择", v.version),
        x(20),
        r.bottom - x(37),
        r.right - x(20),
        r.bottom - x(8),
        x(12),
        dim,
    );
}
fn rate(value: Option<f64>, name: &str) -> String {
    value
        .map(|v| format!("{name} {:.1}%", v * 100.0))
        .unwrap_or(format!("{name} --"))
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
