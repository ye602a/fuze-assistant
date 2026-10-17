//! Event-driven own-window tray/hotkey bridge. Never sends input to another process.
use std::{
    cell::Cell,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const TRAY: u32 = WM_APP + 72;
const HOTKEY: i32 = 0x4655;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Open,
    Hide,
    Toggle,
    Pause,
    Quit,
    Availability,
    Reference,
}
fn menu_action(id: i32) -> Option<Action> {
    match id {
        1 => Some(Action::Open),
        2 => Some(Action::Hide),
        3 => Some(Action::Pause),
        4 => Some(Action::Quit),
        5 => Some(Action::Reference),
        _ => None,
    }
}
pub fn review_menu() -> bool {
    [
        Action::Open,
        Action::Hide,
        Action::Pause,
        Action::Quit,
        Action::Reference,
    ]
    .into_iter()
    .enumerate()
    .all(|(i, action)| menu_action(i as i32 + 1) == Some(action))
        && menu_action(0).is_none()
}
struct State {
    emit: Box<dyn Fn(Action)>,
    enabled: bool,
    icon: HICON,
    registered: Cell<bool>,
    hotkey: Cell<bool>,
    desired_hotkey: Cell<Option<bool>>,
    running: Cell<bool>,
    taskbar: u32,
    suppress_release: Cell<bool>,
}
pub struct Shell {
    hwnd: HWND,
    state: Box<State>,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe fn icon() -> HICON {
    let bytes = include_bytes!("../../../public/fuze.ico");
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    for i in 0..count {
        let Some(e) = bytes.get(6 + i * 16..22 + i * 16) else {
            break;
        };
        if e[0] != 64 {
            continue;
        }
        let length = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
        if let Some(data) = offset
            .checked_add(length)
            .and_then(|end| bytes.get(offset..end))
        {
            return CreateIconFromResourceEx(
                data.as_ptr(),
                length as u32,
                1,
                0x30000,
                64,
                64,
                LR_DEFAULTCOLOR,
            );
        }
    }
    null_mut()
}
unsafe fn notify(hwnd: HWND, s: &State, operation: u32) -> bool {
    let mut n: NOTIFYICONDATAW = zeroed();
    n.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd;
    n.uID = 1;
    n.hIcon = s.icon;
    n.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    n.uCallbackMessage = TRAY;
    let tip = wide(if s.running.get() {
        "符择 FUZE · 自动识别中"
    } else {
        "符择 FUZE · 识别已暂停"
    });
    n.szTip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(operation, &n) != 0
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(l as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let s = &*ptr;
    if s.taskbar != 0 && msg == s.taskbar && s.enabled {
        s.registered.set(notify(hwnd, s, NIM_ADD));
        (s.emit)(Action::Availability);
        return 0;
    }
    match msg {
        WM_TIMER if w == 1 => {
            KillTimer(hwnd, 1);
            (s.emit)(Action::Toggle);
            0
        }
        WM_HOTKEY if w == HOTKEY as usize && s.hotkey.get() => {
            (s.emit)(Action::Pause);
            0
        }
        TRAY if s.registered.get() => {
            match l as u32 {
                WM_LBUTTONUP => {
                    if !s.suppress_release.replace(false) {
                        SetTimer(hwnd, 1, GetDoubleClickTime(), None);
                    }
                }
                WM_LBUTTONDBLCLK => {
                    KillTimer(hwnd, 1);
                    s.suppress_release.set(true);
                    (s.emit)(Action::Open);
                }
                WM_RBUTTONUP => {
                    let menu = CreatePopupMenu();
                    if !menu.is_null() {
                        for (id, label) in [
                            (1, "打开主窗口"),
                            (2, "隐藏主窗口"),
                            (
                                3,
                                if s.running.get() {
                                    "暂停识别"
                                } else {
                                    "继续识别"
                                },
                            ),
                            (4, "退出软件"),
                            (5, "局内符文资料"),
                        ] {
                            AppendMenuW(menu, MF_STRING, id, wide(label).as_ptr());
                        }
                        let mut point: POINT = zeroed();
                        if GetCursorPos(&mut point) != 0 {
                            // Only the user's own tray click may activate this menu owner.
                            SetForegroundWindow(hwnd);
                            let selected = TrackPopupMenu(
                                menu,
                                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                                point.x,
                                point.y,
                                0,
                                hwnd,
                                null(),
                            );
                            if let Some(action) = menu_action(selected) {
                                (s.emit)(action);
                            }
                            PostMessageW(hwnd, WM_NULL, 0, 0);
                        }
                        DestroyMenu(menu);
                    }
                }
                _ => {}
            }
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, w, l)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
impl Shell {
    /// Tests use enabled=false: native message owner exists but no tray or global hotkey is registered.
    pub fn new(enabled: bool, emit: impl Fn(Action) + 'static) -> Result<Rc<Self>, &'static str> {
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = wide("FuzeSlintShell");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..zeroed()
            };
            RegisterClassW(&wc);
            let state = Box::new(State {
                emit: Box::new(emit),
                enabled,
                icon: if enabled { icon() } else { null_mut() },
                registered: Cell::new(false),
                hotkey: Cell::new(false),
                desired_hotkey: Cell::new(None),
                running: Cell::new(false),
                taskbar: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                suppress_release: Cell::new(false),
            });
            // Hidden top-level owner receives Explorer's TaskbarCreated broadcast; HWND_MESSAGE does not.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                wide("符择后台消息").as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                (&*state as *const State).cast(),
            );
            if hwnd.is_null() {
                if !state.icon.is_null() {
                    DestroyIcon(state.icon);
                }
                return Err("shell-window");
            }
            if enabled && !state.icon.is_null() {
                state.registered.set(notify(hwnd, &state, NIM_ADD));
            }
            Ok(Rc::new(Self { hwnd, state }))
        }
    }
    pub fn tray_available(&self) -> bool {
        self.state.registered.get()
    }
    pub fn hotkey_available(&self) -> bool {
        self.state.hotkey.get()
    }
    pub fn hotkey_enabled(&self) -> bool {
        self.state.desired_hotkey.get() == Some(true)
    }
    pub fn sync(&self, hotkey: bool, running: bool) {
        unsafe {
            if self.state.desired_hotkey.replace(Some(hotkey)) != Some(hotkey) {
                if self.state.hotkey.replace(false) {
                    UnregisterHotKey(self.hwnd, HOTKEY);
                }
                if hotkey && self.state.enabled {
                    self.state.hotkey.set(
                        RegisterHotKey(
                            self.hwnd,
                            HOTKEY,
                            MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                            VK_F8 as u32,
                        ) != 0,
                    );
                }
            }
            if self.state.running.replace(running) != running && self.tray_available() {
                if !notify(self.hwnd, &self.state, NIM_MODIFY) {
                    self.state.registered.set(false);
                    (self.state.emit)(Action::Availability);
                }
            }
        }
    }
    pub fn hidden(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd) == 0 }
    }
}
impl Drop for Shell {
    fn drop(&mut self) {
        unsafe {
            KillTimer(self.hwnd, 1);
            if self.state.hotkey.get() {
                UnregisterHotKey(self.hwnd, HOTKEY);
            }
            if self.tray_available() {
                notify(self.hwnd, &self.state, NIM_DELETE);
            }
            DestroyWindow(self.hwnd);
            if !self.state.icon.is_null() {
                DestroyIcon(self.state.icon);
            }
        }
    }
}
