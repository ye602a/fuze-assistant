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
pub fn run(config: &Path, smoke: bool, managed: bool) -> Result<(), Box<dyn std::error::Error>> {
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
        "Local\\FuzeRustDesktopPreview".into()
    };
    let instance = Instance::acquire(&instance_name)?;
    if smoke && Instance::acquire(&instance_name).is_ok() {
        return Err("desktop-smoke-instance".into());
    }
    let v2 = managed && std::env::var("FUZE_MANAGED_HEALTH_PROTOCOL").as_deref() == Ok("2");
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(
            if cfg!(feature = "gpu") {
                if smoke {
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
        .select()?;
    let ui = Workbench::new()?;
    crate::slint_probe::connect(&ui);
    let binding = host_bridge::attach(&ui, config, smoke)?;
    let session = binding.session.clone();
    let lifecycle = lifecycle::attach(&ui, session.clone(), smoke, !smoke)?;
    let maintenance = crate::maintenance_bridge::attach(&ui, session.clone(), smoke, None)?;
    if !smoke {
        ui.show()?;
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
            if ready.get().is_none() && status.initialized && status.available.is_some() {
                // Render the real component through its selected backend before acknowledging UI readiness.
                if ui.window().take_snapshot().is_err() {
                    *error.borrow_mut() = Some("desktop-first-render");
                    engine.stop();
                    let _ = slint::quit_event_loop();
                    return;
                }
                ready.set(Some(Instant::now()));
                if smoke {
                    let _ = engine.set_running(true);
                }
                if managed && !smoke {
                    emit(if v2 {
                        "FUZE_DESKTOP_READY_2"
                    } else {
                        "FUZE_DESKTOP_READY_1"
                    });
                }
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
                if v2 && !smoke {
                    emit("FUZE_DESKTOP_HEALTHY_2");
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
    if v2 && !smoke && !healthy.get() {
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
    result?;
    if let Some(error) = *failure.borrow() {
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
    if let Some(handoff) = handoff {
        handoff.launch()?;
    }
    Ok(())
}
