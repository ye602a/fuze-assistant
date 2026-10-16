//! Bounded hidden-window integration check. Calls component callbacks, never system input.
use crate::{HostState, Workbench};
use slint::{winit_030::WinitWindowAccessor, ComponentHandle, Model};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
pub fn run(data: &Path, output: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(output)?;
    let before = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(
            if cfg!(feature = "gpu") {
                "skia-software"
            } else {
                "software"
            }
            .into(),
        )
        .with_winit_window_attributes_hook(|a| a.with_visible(false).with_active(false))
        .select()?;
    let ui = Workbench::new()?;
    ui.window().set_size(slint::LogicalSize::new(1120., 860.));
    crate::slint_probe::connect(&ui);
    let binding = crate::host_bridge::attach(&ui, crate::host_bridge::offline_config(data), true)?;
    let session = binding.session.clone();
    let lifecycle = crate::lifecycle::attach(&ui, session.clone(), true, false)?;
    if lifecycle.shell.tray_available()
        || lifecycle.shell.hotkey_available()
        || !lifecycle.shell.hidden()
    {
        return Err("review-shell-isolation".into());
    }
    let champion = crate::ChampionWindow::new()?;
    champion
        .window()
        .set_size(slint::LogicalSize::new(420., 640.));
    let champion_binding = crate::champion_bridge::attach(&champion, data)?;
    let weak = ui.as_weak();
    let destination = output.to_owned();
    let data_root = data.to_owned();
    let result = Rc::new(RefCell::new(None));
    let returned = result.clone();
    let step = Cell::new(0);
    let start = Instant::now();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let checked = (|| -> Result<bool, Box<dyn std::error::Error>> {
                if start.elapsed() > Duration::from_secs(30) {
                    return Err("host-review-timeout".into());
                }
                let ui = weak.upgrade().ok_or("window-gone")?;
                let state = ui.global::<HostState>();
                if !ui
                    .window()
                    .with_winit_window(|w| w.is_visible() == Some(false))
                    .unwrap_or(false)
                {
                    return Err("hidden-window-invariant".into());
                }
                let save = |name: &str| -> Result<(), Box<dyn std::error::Error>> {
                    let pixels = ui.window().take_snapshot()?;
                    image::save_buffer(
                        destination.join(format!("host-{name}.png")),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                        image::ColorType::Rgba8,
                    )?;
                    Ok(())
                };
                match step.get() {
                    0 if state.get_preferences().row_count() > 0
                        && state.get_data_version() != "—" =>
                    {
                        save("home-paused")?;
                        state.invoke_preference("champ_topmost".into(), false);
                        step.set(1);
                    }
                    1 if !session.busy() && !session.snapshot().preferences.champ_topmost => {
                        state.invoke_backend_changed(1);
                        step.set(2);
                    }
                    2 if !session.busy() && state.get_backend() == 1 => {
                        ui.set_page(2);
                        save("settings")?;
                        ui.set_scroll_y(-660.);
                        save("settings-controls")?;
                        state.invoke_directory_save("relative/invalid".into());
                        if !session.snapshot().preferences.manual_directory.is_empty() {
                            return Err("invalid-settings-committed".into());
                        }
                        step.set(3);
                    }
                    3 => {
                        save("settings-error")?;
                        ui.set_page(1);
                        ui.set_scroll_y(0.);
                        step.set(4);
                    }
                    4 => {
                        if state.get_history().row_count() != 6 {
                            return Err("history-first-page".into());
                        }
                        state.set_history_selected(0);
                        ui.set_scroll_y(-420.);
                        save("history-detail")?;
                        state.invoke_history_turn(1);
                        step.set(5);
                    }
                    5 => {
                        if state.get_history_page() != 1
                            || state.get_history().row_count() != 2
                            || state.get_history_selected() != -1
                        {
                            return Err("history-pagination".into());
                        }
                        ui.set_scroll_y(-420.);
                        save("history-page2")?;
                        state.invoke_history_filter("没有这个英雄".into(), 0);
                        if state.get_history().row_count() != 0 || state.get_history_page() != 0 {
                            return Err("history-filter".into());
                        }
                        state.invoke_history_filter("".into(), 0);
                        if state.get_history().row_count() != 6 {
                            return Err("history-filter-restore".into());
                        }
                        state.invoke_command("history-refresh".into());
                        if session.busy() {
                            return Err("offline-network-command-accepted".into());
                        }
                        state.invoke_command("toggle-running".into());
                        step.set(6);
                    }
                    6 if state.get_running() => {
                        ui.set_page(0);
                        ui.set_scroll_y(0.);
                        save("home-running")?;
                        state.invoke_command("toggle-running".into());
                        step.set(7);
                    }
                    7 if !state.get_running() => {
                        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }
                            != before
                        {
                            return Err("foreground-changed".into());
                        }
                        if champion.get_augments().row_count() == 0
                            || champion.get_build_lines().row_count() == 0
                        {
                            return Ok(false);
                        }
                        champion.set_scroll_y(-480.);
                        champion.set_expanded_augment(0);
                        let pixels = champion.window().take_snapshot()?;
                        image::save_buffer(
                            destination.join("host-champion-augments.png"),
                            pixels.as_bytes(),
                            pixels.width(),
                            pixels.height(),
                            image::ColorType::Rgba8,
                        )?;
                        champion.invoke_inspect(10);
                        champion.invoke_inspect(11);
                        champion.invoke_filter("不存在的测试强化".into(), -1);
                        if champion.get_augments().row_count() != 0 {
                            return Err("old-hero-resurfaced".into());
                        }
                        champion.invoke_filter("".into(), -1);
                        step.set(8);
                    }
                    8 if champion.get_augments().row_count() > 0 => {
                        if champion.get_inspected_id() != 11 || champion.get_detail_champion() != 11
                        {
                            return Err("latest-hero-mismatch".into());
                        }
                        champion.set_detail_tab(1);
                        let pixels = champion.window().take_snapshot()?;
                        image::save_buffer(
                            destination.join("host-champion-build.png"),
                            pixels.as_bytes(),
                            pixels.width(),
                            pixels.height(),
                            image::ColorType::Rgba8,
                        )?;
                        champion.invoke_filter("不存在的测试强化".into(), -1);
                        if champion.get_augments().row_count() != 0 {
                            return Err("augment-search-failed".into());
                        }
                        if !champion_binding.idle() {
                            return Err("detail-idle-timer".into());
                        }
                        let c = fuze_worker_host::champ_select::Catalog::load(
                            &[fuze_data::Root {
                                path: data_root.clone(),
                                source: fuze_data::Source::Bundled,
                            }],
                            fuze_data::Locale::ZhCn,
                        )?;
                        let mut view = c.playing(4);
                        view.bench = view.team.iter().take(1).cloned().collect();
                        champion_binding.selection(&champion, Some(&view));
                        if champion.get_bench().row_count() != 1 {
                            return Err("bench-not-replaced".into());
                        }
                        champion_binding.selection(&champion, None);
                        if champion.get_bench().row_count() != 0
                            || champion.get_augments().row_count() != 0
                        {
                            return Err("selection-not-cleared".into());
                        }
                        ui.set_page(0);
                        ui.set_scroll_y(0.);
                        ui.invoke_request_close();
                        champion.set_inspected_id(4);
                        champion.set_detail_champion(4);
                        champion.invoke_stage_request(false);
                        step.set(9);
                    }
                    9 if !champion.get_stage_busy() => {
                        if !champion.get_stage_status().contains("离线验证") {
                            return Err("stage-offline-isolation".into());
                        }
                        crate::overlay_review::stage(&champion, &destination)?;
                        crate::overlay_review::run(&data_root, &destination)?;
                        save("close-confirmation")?;
                        if !ui.get_quit_open() || !ui.get_quit_cancel_focused() {
                            return Err("close-fallback-focus".into());
                        }
                        ui.set_quit_open(false);
                        if ui.get_quit_open() {
                            return Err("close-cancel".into());
                        }
                        ui.global::<crate::ShellState>()
                            .invoke_action("quit".into());
                        if !ui.get_quit_open() {
                            return Err("tray-quit-confirmation".into());
                        }
                        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }
                            != before
                        {
                            return Err("foreground-changed".into());
                        }
                        // The same confirm callback stops this isolated engine before ending the loop.
                        ui.invoke_close_requested();
                        return Ok(true);
                    }
                    _ => (),
                }
                Ok(false)
            })();
            match checked {
                Ok(false) => (),
                other => {
                    *returned.borrow_mut() = Some(other);
                    let _ = slint::quit_event_loop();
                }
            }
        },
    );
    slint::run_event_loop_until_quit()?;
    timer.stop();
    result.borrow_mut().take().ok_or("review-incomplete")??;
    let stopped = Instant::now();
    while !binding.session.snapshot().done {
        if stopped.elapsed() > Duration::from_secs(3) {
            return Err("session-exit-timeout".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(binding);
    let report = serde_json::json!({"hidden_window":true,"foreground_unchanged":true,"game_access":false,"system_input":false,"personal_data_read":false,"host_preferences_apply":true,"invalid_preferences_preserved":true,"history_detail_and_paging":true,"offline_network_command_rejected":true,"pause_resume":true,"champion_latest_request":true,"augment_filter":true,"public_build_details":true,"bench_replace_and_clear":true,"detail_idle_timer_stopped":true,"history_filter":true,"close_fallback_and_cancel":true,"close_default_focus":true,"tray_quit_confirmation_callback":true,"confirm_stops_session":true,"native_shell_hidden":true,"tray_registration_tested":false,"global_hotkey_registered":false,"disk_save_verified":false,"production_parity_complete":false});
    std::fs::write(
        output.join("host-review.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
