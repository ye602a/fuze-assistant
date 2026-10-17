//! Slint window lifecycle. The review path never registers a tray icon/hotkey or shows a window.
use crate::{
    native_shell::{Action, Shell},
    ShellState, Workbench,
};
use fuze_worker_host::{desktop::frontend::Session, window_memory::Memory};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{
    winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor},
    ComponentHandle,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub struct Binding {
    pub shell: Rc<Shell>,
    pub redraw_observed: Rc<Cell<bool>>,
    _geometry: Rc<RefCell<Memory>>,
    _debounce: Rc<slint::Timer>,
    window: slint::Weak<Workbench>,
}
impl Drop for Binding {
    fn drop(&mut self) {
        self._debounce.stop();
        // Flush the last position even when the player exits during the debounce interval.
        if let Some(ui) = self.window.upgrade() {
            if let Some(handle) = hwnd(ui.window()) {
                unsafe {
                    self._geometry.borrow_mut().remember(handle);
                }
            }
        }
    }
}
pub fn hwnd(window: &slint::Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    window
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as _),
            _ => None,
        })
        .flatten()
}
fn reveal(ui: &Workbench, interactive: bool) {
    if !interactive {
        return;
    }
    let _ = ui.show();
    ui.window().with_winit_window(|w| {
        w.set_minimized(false);
        w.focus_window();
    });
}
fn close(ui: &Workbench, shell: &Shell, session: &Session, interactive: bool) {
    ui.global::<crate::RecordState>()
        .invoke_action("cancel-confirm".into(), 0);
    ui.global::<crate::MaintenanceState>()
        .set_confirmation("".into());
    if session.snapshot().preferences.close_to_tray && shell.tray_available() {
        ui.set_quit_open(false);
        crate::motion::refresh(ui.global::<crate::UiMotion>(), false);
        if interactive {
            let _ = ui.hide();
        }
    } else {
        if session.snapshot().preferences.close_to_tray && !shell.tray_available() {
            ui.global::<ShellState>()
                .set_notice("系统托盘不可用，窗口会保持可见；可取消或确认退出。".into());
        }
        ui.set_quit_open(true);
    }
}
pub fn attach(
    ui: &Workbench,
    session: Rc<Session>,
    offline: bool,
    interactive: bool,
) -> Result<Binding, &'static str> {
    let weak = ui.as_weak();
    let shell = Shell::new(interactive, move |action| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.global::<ShellState>().invoke_action(
                match action {
                    Action::Open => "open",
                    Action::Hide => "hide",
                    Action::Toggle => "toggle",
                    Action::Pause => "pause",
                    Action::Quit => "quit",
                    Action::Availability => "availability",
                    Action::Reference => "reference",
                }
                .into(),
            )
        });
    })?;
    ui.global::<ShellState>().set_attached(true);
    let weak = ui.as_weak();
    let native = shell.clone();
    let engine = session.clone();
    ui.on_request_close(move || {
        if let Some(ui) = weak.upgrade() {
            close(&ui, &native, &engine, interactive);
        }
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_request_close();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let engine = session.clone();
    ui.on_close_requested(move || {
        engine.stop();
        let _ = slint::quit_event_loop();
    });
    let weak = ui.as_weak();
    let native = shell.clone();
    let engine = session.clone();
    ui.global::<ShellState>().on_action(move |action| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        match action.as_str() {
            "reference" => {
                ui.global::<crate::HostState>()
                    .invoke_command("show-reference".into());
            }
            "open" => reveal(&ui, interactive),
            "hide" => {
                if native.tray_available() && interactive {
                    crate::motion::refresh(ui.global::<crate::UiMotion>(), false);
                    let _ = ui.hide();
                }
            }
            "toggle" => {
                if ui.window().is_visible() && native.tray_available() {
                    if interactive {
                        crate::motion::refresh(ui.global::<crate::UiMotion>(), false);
                        let _ = ui.hide();
                    }
                } else {
                    reveal(&ui, interactive);
                }
            }
            "pause" => {
                if engine.set_running(engine.snapshot().mode != 1).is_err() {
                    ui.global::<ShellState>()
                        .set_notice("当前操作尚未结束，请稍后重试暂停或继续。".into());
                }
            }
            "quit" => {
                reveal(&ui, interactive);
                ui.global::<crate::RecordState>()
                    .invoke_action("cancel-confirm".into(), 0);
                ui.global::<crate::MaintenanceState>()
                    .set_confirmation("".into());
                ui.set_quit_open(true);
            }
            "availability" => {
                availability(&ui, &native, interactive);
                if !native.tray_available() {
                    // Winit set_visible(true) may activate on subsequent shows. Recover to the
                    // taskbar without activation, then synchronize Slint when the player restores it.
                    if interactive && !ui.window().is_visible() {
                        if let Some(handle) = hwnd(ui.window()) {
                            unsafe {
                                windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                                    handle,
                                    windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE,
                                );
                            }
                        }
                    }
                    ui.global::<ShellState>()
                        .set_notice("系统托盘暂不可用，可从任务栏恢复主窗口。".into());
                }
            }
            _ => {}
        }
    });
    let memory = Rc::new(RefCell::new(Memory::new("slint-main", offline)));
    let debounce = Rc::new(slint::Timer::default());
    let saved = memory.clone();
    let delay = debounce.clone();
    let weak = ui.as_weak();
    let redraw_observed = Rc::new(Cell::new(false));
    let redraw = redraw_observed.clone();
    let corners = RefCell::new(crate::window_style::Corners::for_window(ui.window(), false));
    ui.window().on_winit_window_event(move |_, event| {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            if let Some(ui) = weak.upgrade() {
                if let Some(handle) = hwnd(ui.window()) {
                    let mut style = corners.borrow_mut();
                    if let Some(style) = style.as_mut() {
                        style.resize(handle);
                    } else {
                        *style = Some(crate::window_style::Corners::new(handle, false));
                    }
                }
            }
        }
        if let WindowEvent::Focused(focused) = event {
            if let Some(ui) = weak.upgrade() {
                crate::motion::refresh(ui.global::<crate::UiMotion>(), interactive && *focused);
            }
        }
        if matches!(event, WindowEvent::Occluded(true)) {
            if let Some(ui) = weak.upgrade() {
                crate::motion::refresh(ui.global::<crate::UiMotion>(), false);
            }
        }
        if matches!(event, WindowEvent::Resized(size) if size.width == 0 || size.height == 0) {
            if let Some(ui) = weak.upgrade() {
                crate::motion::refresh(ui.global::<crate::UiMotion>(), false);
            }
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            redraw.set(true);
        }
        if interactive && matches!(event, WindowEvent::Focused(true)) {
            let weak = weak.clone();
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    if !ui.window().is_visible() {
                        let _ = ui.show();
                    }
                }
            });
        }
        if matches!(
            event,
            WindowEvent::Moved(_)
                | WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
        ) {
            let saved = saved.clone();
            let weak = weak.clone();
            delay.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(500),
                move || {
                    if let Some(ui) = weak.upgrade() {
                        if let Some(handle) = hwnd(ui.window()) {
                            unsafe {
                                saved.borrow_mut().remember(handle);
                            }
                        }
                    }
                },
            );
        }
        EventResult::Propagate
    });
    let weak = ui.as_weak();
    let native = shell.clone();
    let saved = memory.clone();
    let initialized = Cell::new(false);
    ui.global::<ShellState>().on_sync(move |hotkey, running| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if !initialized.get() {
            if let Some(handle) = hwnd(ui.window()) {
                if interactive {
                    unsafe {
                        saved.borrow().restore(handle);
                    }
                }
                initialized.set(true);
            }
        }
        native.sync(hotkey, running);
        availability(&ui, &native, interactive);
        if saved.borrow().failed() {
            ui.global::<ShellState>()
                .set_notice("窗口位置保存失败，下次启动将使用默认位置。".into());
        }
    });
    let snapshot = session.snapshot();
    ui.global::<ShellState>()
        .invoke_sync(snapshot.preferences.pause_hotkey, snapshot.mode == 1);
    Ok(Binding {
        shell,
        redraw_observed,
        _geometry: memory,
        _debounce: debounce,
        window: ui.as_weak(),
    })
}

fn availability(ui: &Workbench, native: &Shell, interactive: bool) {
    let state = ui.global::<ShellState>();
    state.set_tray_available(native.tray_available());
    state.set_hotkey_hint(
        if !interactive {
            "隔离检查未注册全局快捷键"
        } else if !native.hotkey_enabled() {
            "已关闭"
        } else if native.hotkey_available() {
            "Ctrl + Shift + F8 已就绪"
        } else {
            "快捷键已被占用；仍可使用暂停按钮"
        }
        .into(),
    );
    state.set_tray_hint(
        if !interactive {
            "隔离检查未注册系统托盘"
        } else if native.tray_available() {
            "系统托盘已就绪"
        } else {
            "系统托盘不可用，关闭时保留退出确认"
        }
        .into(),
    );
}
