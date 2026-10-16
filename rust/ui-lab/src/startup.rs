//! Explicit production entry; smoke keeps discovery, input, credentials and user storage disabled.
use crate::{host_bridge, lifecycle, Workbench};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    io::Write,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, System::Threading::CreateMutexW};
pub struct Instance(HANDLE);
impl Instance {
    pub fn acquire(name: &str) -> Result<Self, &'static str> {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err("desktop-instance");
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return Err("desktop-already-running");
            }
            Ok(Self(handle))
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn emit(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}
fn owns_foreground() -> bool {
    unsafe {
        let window = windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        let mut process = 0;
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(window, &mut process);
        process == std::process::id()
    }
}
pub fn run_session(
    config: &Path,
    smoke: bool,
    software: bool,
    start_paused: bool,
    fault: &str,
) -> Result<Option<crate::maintenance_bridge::Handoff>, Box<dyn std::error::Error>> {
    let foreground = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let config = fuze_worker_host::host_config::load(config)?;
    if !smoke {
        for path in [
            &config.worker,
            &config.ort,
            &config.model,
            &config.dictionary,
        ] {
            let meta = std::fs::metadata(path).map_err(|_| "bundle-file-missing")?;
            if !meta.is_file() || meta.len() == 0 {
                return Err("bundle-file-missing".into());
            }
        }
    }
    let instance_name = if smoke {
        format!("Local\\FuzeSlintSmoke-{}", std::process::id())
    } else {
        "Local\\FuzeRustDesktopSession".into()
    };
    let instance = Instance::acquire(&instance_name)?;
    if smoke && Instance::acquire(&instance_name).is_ok() {
        return Err("desktop-smoke-instance".into());
    }
    if smoke && fault == "ordinary" && !software {
        return Err("desktop-fixture-ordinary".into());
    }
    if smoke && fault == "exit" {
        return Ok(None);
    }
    if smoke && (fault == "both" || fault == "gpu-start" && !software) {
        return Err(Box::new(crate::desktop_runner::GraphicsFault));
    }
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(
            if cfg!(feature = "gpu") {
                if software || smoke && fault == "gpu-runtime" {
                    "skia-software"
                } else {
                    "skia"
                }
            } else {
                "software"
            }
            .into(),
        )
        .with_winit_window_attributes_hook(move |a| a.with_active(false).with_visible(!smoke))
        .select()
        .map_err(crate::desktop_runner::graphics)?;
    let ui = Workbench::new().map_err(crate::desktop_runner::graphics)?;
    crate::slint_probe::connect(&ui);
    let binding = host_bridge::attach_recovered(&ui, config, smoke, start_paused)?;
    let session = binding.session.clone();
    let lifecycle = lifecycle::attach(&ui, session.clone(), smoke, !smoke)?;
    // RenderingNotifier disables Skia partial rendering and is absent on some surfaces.
    // Use the existing native event hook; a draw error exits the loop before health commits.
    let redraw = lifecycle.redraw_observed.clone();
    let maintenance = crate::maintenance_bridge::attach(&ui, session.clone(), smoke, None)?;
    if software {
        ui.global::<crate::ShellState>().set_notice(if start_paused {
            "界面渲染异常后已切换兼容模式，自动识别保持暂停。请确认界面正常后再继续识别；OCR 后端设置未改变。"
        } else { "界面使用软件兼容模式；OCR 后端设置未改变。" }.into());
    }
    if !smoke {
        ui.show().map_err(crate::desktop_runner::graphics)?;
    }
    let started = Instant::now();
    let resumed = Rc::new(Cell::new(false));
    let paused = Rc::new(Cell::new(false));
    let smoke_resumed = resumed.clone();
    let smoke_paused = paused.clone();
    let ready_at = Rc::new(Cell::new(None::<Instant>));
    let healthy = Rc::new(Cell::new(false));
    let failure = Rc::new(RefCell::new(None::<&'static str>));
    let ready = ready_at.clone();
    let health = healthy.clone();
    let error = failure.clone();
    let weak = ui.as_weak();
    let engine = session.clone();
    let timer = Rc::new(slint::Timer::default());
    let ready_timer = Rc::downgrade(&timer);
    let injected_runtime_failure = smoke && fault == "gpu-runtime" && !software;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if smoke && owns_foreground() {
                *error.borrow_mut() = Some("desktop-smoke-owned-foreground");
                engine.stop();
                let _ = slint::quit_event_loop();
                return;
            }
            let status = engine.snapshot();
            if !smoke && !redraw.get() {
                if started.elapsed() > Duration::from_secs(10) {
                    *error.borrow_mut() = Some("desktop-first-render");
                    engine.stop();
                    let _ = slint::quit_event_loop();
                }
                return;
            }
            if ready.get().is_none() && status.initialized && status.available.is_some() {
                // Offline snapshots are raster evidence; live readiness also requires a native redraw and healthy event loop.
                if smoke && ui.window().take_snapshot().is_err() {
                    *error.borrow_mut() = Some("desktop-first-render");
                    engine.stop();
                    let _ = slint::quit_event_loop();
                    return;
                }
                ready.set(Some(Instant::now()));
                if smoke {
                    let _ = engine.set_running(true);
                }
                emit("FUZE_DESKTOP_READY_2");
            }
            if smoke && status.mode == 1 && !smoke_resumed.get() {
                smoke_resumed.set(true);
                let _ = engine.set_running(false);
            }
            if smoke && smoke_resumed.get() && status.mode == 0 {
                smoke_paused.set(true);
            }
            if !health.get()
                && ready
                    .get()
                    .is_some_and(|at| at.elapsed() >= Duration::from_secs(3))
                && !status.done
                && (!smoke || smoke_paused.get())
            {
                health.set(true);
                if let Some(timer) = ready_timer.upgrade() {
                    timer.stop();
                }
                emit("FUZE_DESKTOP_HEALTHY_2");
                if injected_runtime_failure {
                    *error.borrow_mut() = Some("desktop-first-render");
                    engine.stop();
                    let _ = slint::quit_event_loop();
                    return;
                }
                if smoke {
                    engine.stop();
                    let _ = slint::quit_event_loop();
                }
            }
            if !health.get() && started.elapsed() > Duration::from_secs(10) {
                *error.borrow_mut() = Some("desktop-startup-timeout");
                engine.stop();
                let _ = slint::quit_event_loop();
            }
        },
    );
    let result = slint::run_event_loop_until_quit();
    timer.stop();
    drop(timer);
    if !healthy.get() {
        emit("FUZE_DESKTOP_CLOSED_2");
    }
    let handoff = maintenance.take_handoff();
    drop(maintenance);
    drop(lifecycle);
    drop(binding);
    session.stop();
    let stopping = Instant::now();
    while !session.snapshot().done {
        if stopping.elapsed() > Duration::from_secs(5) {
            return Err("desktop-stop-timeout".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(session);
    if smoke
        && (ui.window().is_visible()
            || lifecycle::hwnd(ui.window()).is_some_and(|h| unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(h) != 0
            }))
    {
        return Err("desktop-smoke-visible".into());
    }
    let owned_foreground = smoke && owns_foreground();
    drop(ui);
    drop(instance);
    result.map_err(crate::desktop_runner::graphics)?;
    if let Some(error) = *failure.borrow() {
        if error == "desktop-first-render" {
            return Err(Box::new(crate::desktop_runner::GraphicsFault));
        }
        return Err(error.into());
    }
    if smoke {
        drop(Instance::acquire(&instance_name)?);
        if !healthy.get() {
            return Err("desktop-smoke-incomplete".into());
        }
        if owned_foreground || owns_foreground() {
            return Err("desktop-smoke-owned-foreground".into());
        }
        // Another application's focus can change while the player uses their computer.
        // Never restore focus or reject a healthy hidden UI for that unrelated change.
        let external_changed = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() != foreground
        };
        emit(
            &serde_json::json!({"desktop_ready":true,"pause_resume_exit":true,"game_access":false,
            "slint_rendered":true,"hidden":true,"owned_foreground_observed":false,
            "external_foreground_changed":external_changed})
            .to_string(),
        );
    }
    // No updater can start while this session still owns the host mutex or worker.
    Ok(handoff)
}
