//! Player-requested confirmation, using the workbench palette. No background exit prompts.
use super::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
struct Dialog {
    done: bool,
    confirmed: bool,
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let c = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, c.lpCreateParams as isize);
    }
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dialog;
    if p.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    match msg {
        WM_COMMAND if [1, 2].contains(&(w & 0xffff)) => {
            (*p).confirmed = w & 0xffff == 1;
            (*p).done = true;
            0
        }
        WM_CLOSE => {
            (*p).done = true;
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT | WM_PRINTCLIENT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = if msg == WM_PAINT {
                BeginPaint(hwnd, &mut ps)
            } else {
                w as HDC
            };
            let d = GetDpiForWindow(hwnd).max(96) as i32;
            let x = |n| n * d / 96;
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r);
            fill(dc, r, rgb(16, 31, 45));
            let b = CreateSolidBrush(rgb(167, 140, 81));
            FrameRect(dc, &r, b);
            DeleteObject(b);
            text(
                dc,
                "退出软件",
                RECT {
                    left: x(26),
                    top: x(20),
                    right: r.right - x(26),
                    bottom: x(52),
                },
                x(22),
                rgb(240, 230, 210),
            );
            text(
                dc,
                "退出后将停止自动识别和浮窗更新。",
                RECT {
                    left: x(26),
                    top: x(64),
                    right: r.right - x(20),
                    bottom: x(92),
                },
                x(13),
                rgb(183, 211, 219),
            );
            if msg == WM_PAINT {
                EndPaint(hwnd, &ps);
            }
            0
        }
        WM_DRAWITEM => {
            let item = &*(l as *const DRAWITEMSTRUCT);
            let primary = item.CtlID == 1;
            fill(
                item.hDC,
                item.rcItem,
                if primary {
                    rgb(200, 170, 110)
                } else {
                    rgb(12, 26, 37)
                },
            );
            let b = CreateSolidBrush(rgb(167, 140, 81));
            FrameRect(item.hDC, &item.rcItem, b);
            DeleteObject(b);
            let mut r = item.rcItem;
            r.left += 14;
            r.right -= 8;
            text(
                item.hDC,
                if primary { "退出软件" } else { "取消" },
                r,
                14 * GetDpiForWindow(hwnd).max(96) as i32 / 96,
                if primary {
                    rgb(6, 16, 26)
                } else {
                    rgb(240, 230, 210)
                },
            );
            if item.itemState & ODS_FOCUS != 0 {
                let mut r = item.rcItem;
                r.left += 3;
                r.top += 3;
                r.right -= 3;
                r.bottom -= 3;
                DrawFocusRect(item.hDC, &r);
            }
            1
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
pub(super) unsafe fn confirm(owner: HWND) -> bool {
    let instance = GetModuleHandleW(null());
    let class = wide("FuzeQuitConfirmation");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        ..zeroed()
    };
    RegisterClassW(&wc);
    let d = GetDpiForWindow(owner).max(96) as i32;
    let mut parent: RECT = zeroed();
    GetWindowRect(owner, &mut parent);
    let width = 380 * d / 96;
    let height = 184 * d / 96;
    let mut state = Dialog {
        done: false,
        confirmed: false,
    };
    let window = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        class.as_ptr(),
        wide("退出软件").as_ptr(),
        WS_POPUP | WS_CLIPCHILDREN,
        (parent.left + parent.right - width) / 2,
        (parent.top + parent.bottom - height) / 2,
        width,
        height,
        owner,
        null_mut(),
        instance,
        &mut state as *mut _ as *mut _,
    );
    if window.is_null() {
        return false;
    }
    for (id, label, left, width) in [(2, "取消", 166, 72), (1, "退出软件", 248, 106)] {
        CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(label).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            left * d / 96,
            122 * d / 96,
            width * d / 96,
            36 * d / 96,
            window,
            id as _,
            instance,
            null(),
        );
    }
    let was_enabled = windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(owner) != 0;
    EnableWindow(owner, 0);
    ShowWindow(window, SW_SHOW);
    SetFocus(GetDlgItem(window, 2));
    let mut msg: MSG = zeroed();
    while !state.done {
        let read = GetMessageW(&mut msg, null_mut(), 0, 0);
        if read <= 0 {
            if read == 0 {
                PostQuitMessage(msg.wParam as i32);
            }
            break;
        }
        if IsDialogMessageW(window, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    EnableWindow(owner, i32::from(was_enabled));
    DestroyWindow(window);
    state.confirmed
}
