//! Non-activating strip anchored to the foreground League client. Only painted buttons take clicks.
use fuze_lcu::bench::Lobby;
use fuze_worker_host::desktop::frontend::Session;
use std::{
    cell::{Cell, RefCell},
    mem::zeroed,
    ptr::{null, null_mut},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const STOP_KEY: i32 = 1;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn scaled(n: i32, width: i32) -> i32 {
    (n * width + 640) / 1280
}
// 1280x720 client CSS: ten 50px portraits spaced across a 578px bench (top 10px).
fn button(slot: usize, width: i32) -> RECT {
    let center = 378 + (slot as i32 * 528 + 4) / 9;
    RECT {
        left: scaled(center - 18, width),
        right: scaled(center + 18, width),
        top: 0,
        bottom: scaled(18, width),
    }
}

pub fn sync(
    strip: &mut Option<Strip>,
    session: &Rc<Session>,
    status: &fuze_worker_host::desktop::Status,
) -> Result<(), &'static str> {
    use fuze_lcu::reconnect::State::Connected;
    use fuze_runtime::session::Phase::ChampSelect;
    let lobby = status.bench_lobby.filter(|l| {
        status.mode == 1
            && status.preferences.champ_select
            && matches!(status.connection, Some(Connected(ChampSelect)))
            && l.slots.iter().any(|id| *id != 0)
    });
    let Some(lobby) = lobby else {
        *strip = None;
        return Ok(());
    };
    if let Some(current) = strip.as_ref().filter(|s| s.same_session(lobby)) {
        current.update(lobby);
    } else {
        *strip = None;
        *strip = Some(Strip::new(session.clone(), lobby)?);
    }
    Ok(())
}

pub struct Strip {
    hwnd: HWND,
    state: Box<State>,
}
struct State {
    session: Rc<Session>,
    lobby: Cell<Lobby>,
    foreground: Cell<HWND>,
    anchor: Cell<HWND>,
    geometry: Cell<Option<(i32, i32, i32)>>,
    active: RefCell<Option<(u32, Arc<AtomicBool>)>>,
    hint: Cell<&'static str>,
}
impl Strip {
    fn new(session: Rc<Session>, lobby: Lobby) -> Result<Self, &'static str> {
        let state = Box::new(State {
            session,
            lobby: Cell::new(lobby),
            anchor: Cell::default(),
            foreground: Cell::default(),
            geometry: Cell::default(),
            active: RefCell::default(),
            hint: Cell::new(""),
        });
        unsafe {
            let class = wide("FuzeBenchGrabStrip");
            let module = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                hInstance: module,
                lpfnWndProc: Some(proc),
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), IDC_HAND),
                ..zeroed()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TOPMOST,
                class.as_ptr(),
                wide("符择 · 公共英雄池").as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                module,
                (&*state as *const State).cast(),
            );
            if hwnd.is_null() {
                return Err("无法显示抢英雄按钮");
            }
            let strip = Self { hwnd, state };
            if SetLayeredWindowAttributes(hwnd, 0, 255, LWA_COLORKEY) == 0
                || SetTimer(hwnd, 1, 32, None) == 0
            {
                return Err("无法跟随选人窗口");
            }
            strip.state.place(hwnd);
            Ok(strip)
        }
    }
    fn same_session(&self, l: Lobby) -> bool {
        self.state.lobby.get().game == l.game && self.state.lobby.get().ux_pid == l.ux_pid
    }
    fn update(&self, l: Lobby) {
        if self.state.lobby.replace(l) != l {
            let gone = self
                .state
                .active
                .borrow()
                .as_ref()
                .is_some_and(|(id, _)| !l.offers(l.game, *id));
            unsafe {
                if gone {
                    self.state.stop(self.hwnd);
                }
                InvalidateRect(self.hwnd, null(), 0);
                UpdateWindow(self.hwnd);
            }
        }
    }
}
impl Drop for Strip {
    fn drop(&mut self) {
        unsafe {
            self.state.stop(self.hwnd);
            DestroyWindow(self.hwnd);
        }
    }
}
impl State {
    unsafe fn stop(&self, hwnd: HWND) {
        if let Some((_, flag)) = self.active.borrow_mut().take() {
            flag.store(false, Ordering::Release);
            UnregisterHotKey(hwnd, STOP_KEY);
        }
        self.hint.set("");
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn start(&self, hwnd: HWND, hero: u32) {
        self.stop(hwnd);
        if RegisterHotKey(hwnd, STOP_KEY, MOD_NOREPEAT, VK_F9.into()) == 0 {
            self.hint.set("F9 已被占用");
        } else if let Some(flag) = self.session.grab_bench(self.lobby.get().game, hero) {
            *self.active.borrow_mut() = Some((hero, flag));
            self.hint.set("按 F9 停止");
        } else {
            UnregisterHotKey(hwnd, STOP_KEY);
            self.hint.set("英雄池已变化");
        }
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn place(&self, hwnd: HWND) {
        let fg = GetForegroundWindow();
        if fg != self.foreground.replace(fg) {
            let mut pid = 0;
            GetWindowThreadProcessId(fg, &mut pid);
            self.anchor
                .set(if pid != 0 && pid == self.lobby.get().ux_pid {
                    fg
                } else {
                    null_mut()
                });
        }
        let mut rect: RECT = zeroed();
        let mut origin: POINT = zeroed();
        if self.anchor.get().is_null()
            || GetClientRect(fg, &mut rect) == 0
            || ClientToScreen(fg, &mut origin) == 0
            || IsIconic(fg) != 0
            || rect.right < 640
            || (rect.right * 9 - rect.bottom * 16).abs() > rect.right / 8
        {
            if self.geometry.take().is_some() {
                self.stop(hwnd);
                ShowWindow(hwnd, SW_HIDE);
            }
            return;
        }
        let pos = (origin.x, origin.y + scaled(61, rect.right), rect.right);
        if self.geometry.replace(Some(pos)) != Some(pos) {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                pos.0,
                pos.1,
                pos.2,
                scaled(18, pos.2),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(hwnd, null(), 0);
        }
        let finished = self
            .active
            .borrow()
            .as_ref()
            .is_some_and(|(_, f)| !f.load(Ordering::Acquire));
        if finished {
            self.stop(hwnd);
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
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
    let Some(s) = p.as_ref() else {
        return DefWindowProcW(hwnd, msg, w, l);
    };
    match msg {
        WM_TIMER => {
            s.place(hwnd);
            0
        }
        WM_HOTKEY if w == STOP_KEY as usize => {
            s.stop(hwnd);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_NCHITTEST => {
            let mut point = POINT {
                x: l as i16 as i32,
                y: (l >> 16) as i16 as i32,
            };
            ScreenToClient(hwnd, &mut point);
            if let Some((_, _, width)) = s.geometry.get() {
                if s.lobby
                    .get()
                    .slots
                    .iter()
                    .enumerate()
                    .any(|(i, id)| *id != 0 && PtInRect(&button(i, width), point) != 0)
                {
                    return HTCLIENT as isize;
                }
            }
            HTTRANSPARENT as isize
        }
        WM_LBUTTONDOWN => {
            if let Some((_, _, width)) = s.geometry.get() {
                if !s.anchor.get().is_null() && GetForegroundWindow() == s.anchor.get() {
                    let x = (l as u16) as i32;
                    for (i, id) in s.lobby.get().slots.into_iter().enumerate() {
                        let r = button(i, width);
                        if id != 0 && x >= r.left && x < r.right {
                            s.start(hwnd, id);
                            break;
                        }
                    }
                }
            }
            0
        }
        WM_PAINT => {
            let mut ps = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            FillRect(dc, &ps.rcPaint, GetStockObject(BLACK_BRUSH).cast());
            if let Some((_, _, width)) = s.geometry.get() {
                let active = s.active.borrow().as_ref().map(|(hero, _)| *hero);
                draw(dc, width, s.lobby.get().slots, active, s.hint.get());
            }
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

unsafe fn draw(dc: HDC, width: i32, slots: [u32; 10], active_hero: Option<u32>, hint: &str) {
    let font = CreateFontW(
        -scaled(12, width),
        0,
        0,
        0,
        600,
        0,
        0,
        0,
        DEFAULT_CHARSET.into(),
        0,
        0,
        ANTIALIASED_QUALITY.into(),
        0,
        wide("Microsoft YaHei UI").as_ptr(),
    );
    let old = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    let dark = CreateSolidBrush(0x00231a10);
    let gold = CreateSolidBrush(0x0085a8c8);
    for (slot, hero) in slots.into_iter().enumerate().filter(|(_, h)| *h != 0) {
        let mut r = button(slot, width);
        let active = active_hero == Some(hero);
        FillRect(dc, &r, if active { gold } else { dark });
        FrameRect(dc, &r, gold);
        SetTextColor(dc, if active { 0x00151008 } else { 0x00bbdafa });
        let label = wide(if active { "抢…" } else { "抢" });
        DrawTextW(
            dc,
            label.as_ptr(),
            -1,
            &mut r,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
    if !hint.is_empty() {
        let mut r = RECT {
            left: scaled(936, width),
            right: scaled(1040, width),
            top: 0,
            bottom: scaled(18, width),
        };
        FillRect(dc, &r, dark);
        SetTextColor(dc, 0x00bbdafa);
        DrawTextW(
            dc,
            wide(hint).as_ptr(),
            -1,
            &mut r,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
    SelectObject(dc, old);
    DeleteObject(font);
    DeleteObject(dark);
    DeleteObject(gold);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bench_strip_draws_at_portraits_with_clear_gaps_and_stop_hint() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = 1920;
            info.bmiHeader.biHeight = -32;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            for width in [1024, 1280, 1600, 1920] {
                PatBlt(dc, 0, 0, 1920, 32, BLACKNESS);
                draw(
                    dc,
                    width,
                    [11, 0, 33, 0, 0, 0, 0, 0, 0, 44],
                    Some(33),
                    "按 F9 停止",
                );
                // Independent reference positions from the supplied 1920px screen.
                let pixel = |x: i32| GetPixel(dc, x * width / 1920, scaled(3, width));
                assert_eq!(pixel(546), 0x00231a10);
                assert_eq!(pixel(655), 0); // empty second slot
                assert_eq!(pixel(724), 0x0085a8c8); // locked target
                assert_eq!(pixel(960), 0); // timer in the middle
                assert_eq!(pixel(1340), 0x00231a10); // last slot
                assert_ne!(pixel(1412), 0); // stop hint beyond the bench
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }
}
