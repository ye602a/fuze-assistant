//! Non-activating strip anchored to the foreground League client. Only painted buttons take clicks.
use fuze_lcu::bench::Lobby;
use fuze_worker_host::{
    controller::{BenchGrab, GrabOutcome},
    desktop::frontend::Session,
};
use std::{
    cell::{Cell, RefCell},
    mem::zeroed,
    ptr::{null, null_mut},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::WM_MOUSELEAVE, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
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
        bottom: scaled(22, width),
    }
}

// The transparent window includes the portrait row; only the buttons at y=61 take input.
fn arrow_bounds(slots: [u32; 10], hero: u32, width: i32) -> Option<RECT> {
    let slot = slots.iter().position(|id| *id == hero)?;
    let r = button(slot, width);
    let center = (r.left + r.right) / 2;
    Some(RECT {
        left: center - scaled(12, width),
        right: center + scaled(12, width) + 1,
        top: scaled(9, width),
        bottom: scaled(41, width),
    })
}
fn bob(elapsed: Duration) -> i32 {
    (3.0 - 3.0 * (elapsed.as_secs_f64() * std::f64::consts::TAU / 0.9).cos()).round() as i32
}
unsafe fn draw_arrow(dc: HDC, bounds: RECT, width: i32, offset: i32) {
    let x = (bounds.left + bounds.right) / 2;
    let y = scaled(12 + offset, width);
    let points = [
        (-4i32, 0),
        (4, 0),
        (4, 8),
        (10, 8),
        (0, 18),
        (-10, 8),
        (-4, 8),
    ]
    .map(|(dx, dy)| POINT {
        x: x + scaled(dx.abs(), width) * dx.signum(),
        y: y + scaled(dy, width),
    });
    let fill = CreateSolidBrush(0x0085a8c8);
    let border = CreatePen(PS_SOLID, scaled(2, width).max(1), 0x00231a10);
    let old_fill = SelectObject(dc, fill);
    let old_border = SelectObject(dc, border);
    Polygon(dc, points.as_ptr(), points.len() as i32);
    SelectObject(dc, old_border);
    SelectObject(dc, old_fill);
    DeleteObject(border);
    DeleteObject(fill);
}
fn hint_rect(width: i32) -> RECT {
    RECT {
        left: scaled(936, width),
        right: scaled(1264, width),
        top: 0,
        bottom: scaled(22, width),
    }
}
fn hit(slots: [u32; 10], width: i32, x: i32, y: i32) -> Option<u32> {
    slots.into_iter().enumerate().find_map(|(i, id)| {
        let r = button(i, width);
        (id != 0 && x >= r.left && x < r.right && y >= r.top && y < r.bottom).then_some(id)
    })
}
fn window_hit(slots: [u32; 10], width: i32, x: i32, y: i32) -> Option<u32> {
    hit(slots, width, x, y - scaled(61, width))
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Pointer {
    hover: Option<u32>,
    pressed: Option<u32>,
}
impl Pointer {
    fn release(&mut self, hero: Option<u32>) -> Option<u32> {
        self.pressed.take().filter(|id| Some(*id) == hero)
    }
}
// Accepted requests remain pending; only the observed champion confirms success.
fn observed_outcome(hero: u32, own: Option<u32>, lobby: Lobby, task: GrabOutcome) -> GrabOutcome {
    if own == Some(hero) {
        GrabOutcome::Acquired
    } else if !lobby.offers(lobby.game, hero) {
        GrabOutcome::Unavailable
    } else {
        task
    }
}
fn outcome_hint(outcome: GrabOutcome) -> &'static str {
    match outcome {
        GrabOutcome::Running => "按 F9 停止",
        GrabOutcome::Requested => "正在确认换人 · 按 F9 停止",
        GrabOutcome::Acquired => "已抢到目标英雄",
        GrabOutcome::Unavailable => "目标英雄已不可用",
        GrabOutcome::Failed => "换人请求失败，请重试",
        GrabOutcome::Stopped => "已停止抢取",
    }
}

pub fn sync(
    strip: &mut Option<Strip>,
    session: &Rc<Session>,
    status: &fuze_worker_host::desktop::Status,
) -> Result<(), &'static str> {
    use fuze_lcu::reconnect::State::Connected;
    use fuze_runtime::session::Phase::ChampSelect;
    let lobby = status.bench_lobby.filter(|_| {
        status.mode == 1
            && status.preferences.champ_select
            && matches!(status.connection, Some(Connected(ChampSelect)))
    });
    let Some(lobby) = lobby else {
        *strip = None;
        return Ok(());
    };
    let own = status
        .selection
        .as_ref()
        .and_then(|v| v.team.get(v.local_slot?))
        .map(|h| h.id);
    if let Some(current) = strip.as_ref().filter(|s| s.same_session(lobby)) {
        current.update(lobby, own);
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
    active: RefCell<Option<(u32, Arc<BenchGrab>)>>,
    pointer: Cell<Pointer>,
    own: Cell<Option<u32>>,
    requested: Cell<Option<u32>>,
    hint: Cell<&'static str>,
    hint_until: Cell<Option<Instant>>,
    motion: Cell<Option<Instant>>,
    arrow_offset: Cell<i32>,
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
            pointer: Cell::default(),
            own: Cell::default(),
            requested: Cell::default(),
            hint: Cell::new(""),
            hint_until: Cell::default(),
            motion: Cell::default(),
            arrow_offset: Cell::new(0),
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
    fn update(&self, l: Lobby, own: Option<u32>) {
        self.state.own.set(own);
        unsafe {
            if self.state.lobby.replace(l) != l {
                self.state.pointer.set(Pointer::default());
                InvalidateRect(self.hwnd, null(), 0);
            }
            self.state.feedback(self.hwnd);
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
    unsafe fn repaint(&self, hwnd: HWND) {
        InvalidateRect(hwnd, null(), 0);
        UpdateWindow(hwnd);
    }
    unsafe fn stop(&self, hwnd: HWND) {
        if let Some((_, task)) = self.active.borrow_mut().take() {
            task.cancel();
        }
        UnregisterHotKey(hwnd, STOP_KEY);
        if GetCapture() == hwnd {
            ReleaseCapture();
        }
        self.pointer.set(Pointer::default());
        self.motion.set(None);
        self.arrow_offset.set(0);
        self.requested.set(None);
        self.hint.set("");
        self.hint_until.set(None);
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn notice(&self, hwnd: HWND, text: &'static str) {
        self.hint.set(text);
        self.hint_until
            .set(Some(Instant::now() + Duration::from_secs(3)));
        self.repaint(hwnd);
    }
    unsafe fn feedback(&self, hwnd: HWND) {
        let finished = self.active.borrow().as_ref().and_then(|(hero, task)| {
            let outcome = observed_outcome(*hero, self.own.get(), self.lobby.get(), task.outcome());
            if outcome == GrabOutcome::Running {
                return None;
            }
            Some((*hero, outcome, task.failure_hint()))
        });
        if let Some((hero, outcome, failure)) = finished {
            if outcome == GrabOutcome::Requested {
                // Keep the target and its stop control visible until the read-only
                // champion state confirms the accepted request.
                if self.requested.get() != Some(hero) {
                    self.requested.set(Some(hero));
                    self.notice(hwnd, outcome_hint(outcome));
                }
            } else {
                self.stop(hwnd);
                self.notice(
                    hwnd,
                    if outcome == GrabOutcome::Failed {
                        failure
                    } else {
                        outcome_hint(outcome)
                    },
                );
            }
        }
        if self
            .hint_until
            .get()
            .is_some_and(|until| Instant::now() >= until)
        {
            if self.requested.get().is_some() {
                self.stop(hwnd);
                self.notice(hwnd, "未确认换人结果，请查看当前英雄");
            } else {
                self.hint.set("");
                self.hint_until.set(None);
                self.repaint(hwnd);
            }
        }
    }
    unsafe fn start(&self, hwnd: HWND, hero: u32) {
        let same = self
            .active
            .borrow()
            .as_ref()
            .is_some_and(|(id, _)| *id == hero);
        if same {
            self.stop(hwnd);
            self.notice(hwnd, outcome_hint(GrabOutcome::Stopped));
            return;
        }
        // Cancel the old target before queuing the next one; never wait for its retry loop.
        self.stop(hwnd);
        if RegisterHotKey(hwnd, STOP_KEY, MOD_NOREPEAT, VK_F9.into()) == 0 {
            self.notice(hwnd, "F9 被其他程序占用，无法开始抢取");
        } else if let Some(task) = self.session.grab_bench(self.lobby.get().game, hero) {
            *self.active.borrow_mut() = Some((hero, task));
            self.hint.set(outcome_hint(GrabOutcome::Running));
            let mut animate = 0i32;
            if GetSystemMetrics(SM_REMOTESESSION) == 0
                && SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    (&mut animate as *mut i32).cast(),
                    0,
                ) != 0
                && animate != 0
            {
                self.motion.set(Some(Instant::now()));
            }
            self.repaint(hwnd);
        } else {
            UnregisterHotKey(hwnd, STOP_KEY);
            self.notice(hwnd, "未能开始抢取，请重试");
        }
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
        let pos = (origin.x, origin.y, rect.right);
        if self.geometry.replace(Some(pos)) != Some(pos) {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                pos.0,
                pos.1,
                pos.2,
                scaled(83, pos.2),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(hwnd, null(), 0);
        }
        self.feedback(hwnd);
        if let Some(start) = self.motion.get() {
            let offset = bob(start.elapsed());
            if self.arrow_offset.replace(offset) != offset {
                if let Some((hero, _)) = self.active.borrow().as_ref() {
                    if let Some(bounds) = arrow_bounds(self.lobby.get().slots, *hero, rect.right) {
                        InvalidateRect(hwnd, &bounds, 0);
                    }
                }
            }
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
            s.notice(hwnd, outcome_hint(GrabOutcome::Stopped));
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
                if window_hit(s.lobby.get().slots, width, point.x, point.y).is_some() {
                    return HTCLIENT as isize;
                }
            }
            HTTRANSPARENT as isize
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
            let hero = s.geometry.get().and_then(|(_, _, width)| {
                window_hit(
                    s.lobby.get().slots,
                    width,
                    l as i16 as i32,
                    (l >> 16) as i16 as i32,
                )
            });
            let mut pointer = s.pointer.get();
            pointer.hover = hero;
            if msg == WM_MOUSEMOVE {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
            }
            let mut clicked = None;
            if msg == WM_LBUTTONDOWN {
                pointer.pressed = hero;
                if hero.is_some() {
                    SetCapture(hwnd);
                }
            } else if msg == WM_LBUTTONUP {
                clicked = pointer.release(hero);
                ReleaseCapture();
            }
            if s.pointer.replace(pointer) != pointer {
                s.repaint(hwnd);
            }
            if let Some(id) = clicked {
                if !s.anchor.get().is_null() && GetForegroundWindow() == s.anchor.get() {
                    s.start(hwnd, id);
                }
            }
            0
        }
        WM_MOUSELEAVE | WM_CAPTURECHANGED | WM_CANCELMODE => {
            s.pointer.set(Pointer::default());
            s.repaint(hwnd);
            0
        }
        WM_PAINT => {
            let mut ps = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            FillRect(dc, &ps.rcPaint, GetStockObject(BLACK_BRUSH).cast());
            if let Some((_, _, width)) = s.geometry.get() {
                let active = s.active.borrow().as_ref().map(|(hero, _)| *hero);
                if let Some(bounds) =
                    active.and_then(|id| arrow_bounds(s.lobby.get().slots, id, width))
                {
                    draw_arrow(dc, bounds, width, s.arrow_offset.get());
                }
                let top = scaled(61, width);
                if ps.rcPaint.bottom > top {
                    SetViewportOrgEx(dc, 0, top, null_mut());
                    draw(
                        dc,
                        width,
                        s.lobby.get().slots,
                        active,
                        s.hint.get(),
                        s.pointer.get(),
                    );
                    SetViewportOrgEx(dc, 0, 0, null_mut());
                }
            }
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

unsafe fn draw(
    dc: HDC,
    width: i32,
    slots: [u32; 10],
    active_hero: Option<u32>,
    hint: &str,
    pointer: Pointer,
) {
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
    let hover = CreateSolidBrush(0x00514323);
    let pressed = CreateSolidBrush(0x00b1d4ed);
    for (slot, hero) in slots.into_iter().enumerate().filter(|(_, h)| *h != 0) {
        let mut r = button(slot, width);
        let active = active_hero == Some(hero);
        let down = pointer.pressed == Some(hero) && pointer.hover == Some(hero);
        FillRect(
            dc,
            &r,
            if down {
                pressed
            } else if active {
                gold
            } else if pointer.hover == Some(hero) {
                hover
            } else {
                dark
            },
        );
        FrameRect(dc, &r, gold);
        SetTextColor(
            dc,
            if active || down {
                0x00151008
            } else {
                0x00bbdafa
            },
        );
        let label = wide(if active { "停" } else { "抢" });
        DrawTextW(
            dc,
            label.as_ptr(),
            -1,
            &mut r,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
    if !hint.is_empty() {
        let mut r = hint_rect(width);
        FillRect(dc, &r, dark);
        SetTextColor(dc, 0x00bbdafa);
        DrawTextW(
            dc,
            wide(hint).as_ptr(),
            -1,
            &mut r,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
    SelectObject(dc, old);
    DeleteObject(font);
    DeleteObject(dark);
    DeleteObject(gold);
    DeleteObject(hover);
    DeleteObject(pressed);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_request_waits_for_observed_champion_and_detects_target_loss() {
        let mut lobby = Lobby {
            slots: [11, 22, 0, 0, 0, 0, 0, 0, 0, 0],
            game: 1,
            ux_pid: 1,
        };
        assert_eq!(
            observed_outcome(11, Some(9), lobby, GrabOutcome::Requested),
            GrabOutcome::Requested
        );
        assert_eq!(
            observed_outcome(11, Some(9), lobby, GrabOutcome::Running),
            GrabOutcome::Running
        );
        lobby.slots[0] = 33;
        assert_eq!(
            observed_outcome(11, Some(9), lobby, GrabOutcome::Requested),
            GrabOutcome::Unavailable
        );
        assert_eq!(
            observed_outcome(11, Some(11), lobby, GrabOutcome::Requested),
            GrabOutcome::Acquired
        );
        assert_eq!(
            observed_outcome(22, Some(9), lobby, GrabOutcome::Failed),
            GrabOutcome::Failed
        );
    }
    #[test]
    fn arrow_tracks_target_id_and_portraits_remain_click_through() {
        let slots = [11, 22, 33, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(bob(Duration::ZERO), 0);
        assert_eq!(bob(Duration::from_millis(450)), 6);
        assert_eq!(bob(Duration::from_millis(900)), 0);
        for width in [640, 1280, 1920, 2560] {
            let a = arrow_bounds(slots, 11, width).unwrap();
            let b = arrow_bounds(slots, 22, width).unwrap();
            assert!(a.right < b.left);
            let moved = arrow_bounds([22, 11, 33, 0, 0, 0, 0, 0, 0, 0], 11, width).unwrap();
            assert_eq!(moved.left, b.left);
            assert!(arrow_bounds(slots, 99, width).is_none());
            assert_eq!(
                window_hit(slots, width, (a.left + a.right) / 2, scaled(20, width)),
                None
            );
            assert_eq!(
                window_hit(slots, width, (a.left + a.right) / 2, scaled(65, width)),
                Some(11)
            );
        }
    }
    #[test]
    fn arrow_render_moves_vertically_and_switches_to_the_new_portrait() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = 1280;
            info.bmiHeader.biHeight = -270;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!bitmap.is_null());
            let old = SelectObject(dc, bitmap);
            PatBlt(dc, 0, 0, 1280, 270, BLACKNESS);
            let slots = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            for (row, (hero, offset)) in [(3, 0), (3, 6), (6, 0)].into_iter().enumerate() {
                SetViewportOrgEx(dc, 0, row as i32 * 90, null_mut());
                draw_arrow(dc, arrow_bounds(slots, hero, 1280).unwrap(), 1280, offset);
                SetViewportOrgEx(dc, 0, row as i32 * 90 + 61, null_mut());
                draw(
                    dc,
                    1280,
                    slots,
                    Some(hero),
                    outcome_hint(GrabOutcome::Running),
                    Pointer::default(),
                );
            }
            SetViewportOrgEx(dc, 0, 0, null_mut());
            let a = arrow_bounds(slots, 3, 1280).unwrap();
            let x = (a.left + a.right) / 2;
            assert_ne!(GetPixel(dc, x, 13), 0);
            assert_eq!(GetPixel(dc, x, 103), 0);
            assert_ne!(GetPixel(dc, x, 119), 0);
            assert_eq!(GetPixel(dc, x, 193), 0);
            let b = arrow_bounds(slots, 6, 1280).unwrap();
            assert_ne!(GetPixel(dc, (b.left + b.right) / 2, 193), 0);
            if let Some(path) = std::env::var_os("FUZE_BENCH_ARROW_PNG") {
                let mut rgba =
                    std::slice::from_raw_parts(bits as *const u8, 1280 * 270 * 4).to_vec();
                for p in rgba.chunks_exact_mut(4) {
                    p.swap(0, 2);
                    p[3] = 255;
                }
                image::RgbaImage::from_raw(1280, 270, rgba)
                    .unwrap()
                    .save(path)
                    .unwrap();
            }
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }
    #[test]
    fn clicks_require_the_same_hero_on_release_and_ignore_hint_and_empty_slots() {
        for width in [640, 1024, 1280, 1600, 1920, 2560] {
            let mut slots = [0; 10];
            slots[0] = 11;
            slots[9] = 99;
            let r = button(0, width);
            let mut pointer = Pointer {
                hover: Some(11),
                pressed: hit(slots, width, r.left + 2, 2),
            };
            assert_eq!(pointer.release(Some(11)), Some(11));
            pointer.pressed = Some(11);
            slots[0] = 22;
            assert_eq!(pointer.release(hit(slots, width, r.left + 2, 2)), None);
            pointer.pressed = Some(11);
            assert_eq!(pointer.release(None), None);
            assert_eq!(hit(slots, width, r.left + 2, -1), None);
            assert_eq!(hit(slots, width, r.left + 2, r.bottom), None);
            assert_eq!(hit(slots, width, button(1, width).left + 2, 2), None);
            assert_eq!(hit(slots, width, hint_rect(width).left + 2, 2), None);
            assert!(button(9, width).right < hint_rect(width).left);
            assert!(hint_rect(width).right <= width);
        }
    }
    #[test]
    fn feedback_states_render_separately_from_buttons() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = 1280;
            info.bmiHeader.biHeight = -240;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!bitmap.is_null());
            let old = SelectObject(dc, bitmap);
            PatBlt(dc, 0, 0, 1280, 240, BLACKNESS);
            let states = [
                (None, "", Pointer::default()),
                (
                    None,
                    "",
                    Pointer {
                        hover: Some(3),
                        pressed: None,
                    },
                ),
                (
                    None,
                    "",
                    Pointer {
                        hover: Some(3),
                        pressed: Some(3),
                    },
                ),
                (
                    Some(3),
                    outcome_hint(GrabOutcome::Running),
                    Pointer::default(),
                ),
                (None, "F9 被其他程序占用，无法开始抢取", Pointer::default()),
                (
                    None,
                    outcome_hint(GrabOutcome::Acquired),
                    Pointer::default(),
                ),
            ];
            for (row, (active, hint, pointer)) in states.into_iter().enumerate() {
                SetViewportOrgEx(dc, 0, row as i32 * 40, null_mut());
                draw(
                    dc,
                    1280,
                    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                    active,
                    hint,
                    pointer,
                );
            }
            SetViewportOrgEx(dc, 0, 0, null_mut());
            assert_ne!(GetPixel(dc, 480, 3), GetPixel(dc, 480, 43));
            assert_ne!(GetPixel(dc, 480, 43), GetPixel(dc, 480, 83));
            let bytes = std::slice::from_raw_parts(bits as *const u8, 1280 * 240 * 4);
            for y in 0..240usize {
                for x in 0..1280usize {
                    let painted = (0..10).any(|i| {
                        let r = button(i, 1280);
                        x >= r.left as usize && x < r.right as usize && y % 40 < r.bottom as usize
                    }) || x >= 936 && x < 1264 && y % 40 < 22;
                    if !painted {
                        assert_eq!(
                            &bytes[(y * 1280 + x) * 4..(y * 1280 + x) * 4 + 3],
                            &[0, 0, 0]
                        );
                    }
                }
            }
            if let Some(path) = std::env::var_os("FUZE_BENCH_REVIEW_PNG") {
                let mut rgba = bytes.to_vec();
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                    pixel[3] = 255;
                }
                image::RgbaImage::from_raw(1280, 240, rgba)
                    .unwrap()
                    .save(path)
                    .unwrap();
            }
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }
    #[test]
    fn bench_strip_draws_at_portraits_with_clear_gaps_and_stop_hint() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = 1920;
            info.bmiHeader.biHeight = -40;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            for width in [1024, 1280, 1600, 1920] {
                PatBlt(dc, 0, 0, 1920, 40, BLACKNESS);
                draw(
                    dc,
                    width,
                    [11, 0, 33, 0, 0, 0, 0, 0, 0, 44],
                    Some(33),
                    "正在抢取 · 按 F9 停止",
                    Pointer::default(),
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
