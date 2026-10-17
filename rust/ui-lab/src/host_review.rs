//! Bounded hidden-window integration check. Component pointer events, never system input.
use crate::{HostState, Workbench};
use slint::{winit_030::WinitWindowAccessor, ComponentHandle, Model};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
// Exercise Slint hit testing, not direct page/callback assignment. Events stay in
// this hidden component; no Windows messages, cursor movement or system input.
pub fn close_only(
    data: &Path,
    output: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use slint::platform::{PointerEventButton, WindowEvent};
    std::fs::create_dir_all(output)?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|a| a.with_visible(false).with_active(false))
        .select()?;
    let ui = Workbench::new()?;
    ui.window().set_size(slint::LogicalSize::new(1120., 860.));
    crate::slint_probe::connect(&ui);
    let binding = crate::host_bridge::attach(&ui, crate::host_bridge::offline_config(data), true)?;
    let _lifecycle = crate::lifecycle::attach(&ui, binding.session.clone(), true, false)?;
    let click = |x, y| -> Result<(), Box<dyn std::error::Error>> {
        let _ = ui.window().take_snapshot()?;
        let position = slint::LogicalPosition::new(x, y);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        Ok(())
    };
    for (action, x) in [("cancel", 495.), ("minimize", 605.), ("exit", 713.)] {
        click(1097., 20.)?;
        if !ui.get_quit_open() {
            return Err("close-choice-not-open".into());
        }
        if action == "cancel" {
            let pixels = ui.window().take_snapshot()?;
            image::save_buffer(
                output.join("close-choice.png"),
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                image::ColorType::Rgba8,
            )?;
        }
        click(x, 479.)?;
        if action != "exit" && (ui.get_quit_open() || binding.session.snapshot().done) {
            return Err("close-choice-action".into());
        }
    }
    let started = Instant::now();
    while !binding.session.snapshot().done {
        if started.elapsed() > Duration::from_secs(3) {
            return Err("close-choice-exit-timeout".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if ui.window().is_visible() {
        return Err("close-review-visible".into());
    }
    Ok(
        serde_json::json!({"close_choice":true,"cancel_keeps_running":true,"minimize_keeps_running":true,"exit_stops_session":true,"system_input":false,"game_access":false,"native_tray_interaction_tested":false}),
    )
}
fn control(
    ui: &Workbench,
    label: &str,
    fallback: impl FnOnce(),
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "ui-audit")]
    {
        let _ = fallback;
        crate::control_review::hit(ui, label)?;
    }
    #[cfg(not(feature = "ui-audit"))]
    {
        let _ = (ui, label);
        fallback();
    }
    Ok(())
}
fn navigation_input(ui: &Workbench) -> Result<(), Box<dyn std::error::Error>> {
    use slint::platform::{PointerEventButton, WindowEvent};
    let click = |x, y| {
        let position = slint::LogicalPosition::new(x, y);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    };
    for width in [1120., 920.] {
        ui.window().set_size(slint::LogicalSize::new(width, 860.));
        for health in [0, 3] {
            ui.set_health(health);
            for (page, y) in [(1, 210.), (2, 277.), (0, 142.)] {
                let _ = ui.window().take_snapshot()?;
                click(90., y);
                if ui.get_page() != page {
                    return Err(format!(
                        "navigation-hit-test-failed: expected={page} actual={}",
                        ui.get_page()
                    )
                    .into());
                }
            }
        }
    }
    ui.window().set_size(slint::LogicalSize::new(1120., 860.));
    ui.set_health(0);
    // A confirmation still blocks navigation, and its first cancel click works.
    ui.set_quit_open(true);
    let _ = ui.window().take_snapshot()?;
    click(90., 210.);
    if ui.get_page() != 0 {
        return Err("modal-navigation-not-blocked".into());
    }
    ui.set_quit_open(true); // outside click may dismiss, reopen for the button check
    let _ = ui.window().take_snapshot()?;
    click(495., 479.);
    if ui.get_quit_open() {
        return Err("modal-cancel-hit-test-failed".into());
    }
    let _ = ui.window().take_snapshot()?;
    click(90., 277.);
    if ui.get_page() != 2 {
        return Err("navigation-after-modal-failed".into());
    }
    // Keyboard activation remains available on the now-focused settings button.
    ui.set_page(0);
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    if ui.get_page() != 2 {
        return Err("navigation-keyboard-failed".into());
    }
    click(90., 142.);
    Ok(())
}
pub fn run(data: &Path, output: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(output)?;
    if !crate::native_shell::review_menu() {
        return Err("tray-menu-command-map".into());
    }
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
    let records = binding.records.clone();
    let closure = RefCell::new(crate::closure_review::Review::new(data, session.clone())?);
    let settings_fixture = output.join("legacy-settings-fixture.json");
    std::fs::write(&settings_fixture, br#"{"championInsight":{"pinDuringChampSelect":true},"app":{"locale":"en-US"},"autoScreenshotGameflowControl":true}"#)?;
    let maintenance_root = crate::maintenance_review::fixture(output)?;
    let _maintenance = crate::maintenance_bridge::attach(
        &ui,
        session.clone(),
        true,
        Some(maintenance_root.clone()),
    )?;
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
                        navigation_input(&ui)?;
                        control(&ui, "偏好设置", || ui.set_page(2))?;
                        control(&ui, "预览选人详情", || {
                            state.invoke_command("preview-champion".into())
                        })?;
                        if !state.get_notice().contains("已打开只读选人预览") {
                            return Err("champion-preview-command".into());
                        }
                        control(&ui, "预览三卡推荐", || {
                            state.invoke_command("preview-cards".into())
                        })?;
                        if !state.get_notice().contains("已打开三卡公开样例") {
                            return Err("cards-preview-command".into());
                        }
                        control(&ui, "选人阶段窗口置顶", || {
                            state.invoke_preference("champ_topmost".into(), false)
                        })?;
                        if !state.get_busy() || !state.get_notice().contains("正在保存") {
                            return Err("settings-no-immediate-feedback".into());
                        }
                        step.set(1);
                    }
                    1 if !session.busy()
                        && !state.get_busy()
                        && !session.snapshot().preferences.champ_topmost =>
                    {
                        #[cfg(feature = "ui-audit")]
                        crate::control_review::select_next(&ui, "识别后端")?;
                        #[cfg(not(feature = "ui-audit"))]
                        state.invoke_backend_changed(1);
                        step.set(2);
                    }
                    2 if !session.busy() && !state.get_busy() && state.get_backend() == 1 => {
                        ui.set_page(2);
                        save("settings")?;
                        ui.set_scroll_y(-660.);
                        save("settings-controls")?;
                        #[cfg(feature = "ui-audit")]
                        {
                            crate::control_review::hit(
                                &ui,
                                "高级回退：自动连接失败时手动指定目录",
                            )?;
                            crate::control_review::type_text(
                                &ui,
                                "客户端目录",
                                "relative/invalid",
                            )?;
                        }
                        control(&ui, "保存目录", || {
                            state.invoke_directory_save("relative/invalid".into())
                        })?;
                        if !session.snapshot().preferences.manual_directory.is_empty() {
                            return Err("invalid-settings-committed".into());
                        }
                        step.set(3);
                    }
                    3 => {
                        if !state.get_notice().contains("目录无效")
                            || !ui.get_feedback().contains("目录无效")
                        {
                            return Err("directory-error-not-visible".into());
                        }
                        save("settings-error")?;
                        ui.set_page(1);
                        ui.set_scroll_y(0.);
                        step.set(4);
                    }
                    4 => {
                        if state.get_history().row_count() != 10 {
                            return Err("history-first-page".into());
                        }
                        state.set_history_selected(0);
                        ui.set_scroll_y(-420.);
                        save("history-detail")?;
                        control(&ui, "下一页", || state.invoke_history_turn(1))?;
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
                        if state.get_history().row_count() != 10 {
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
                        ui.set_quit_open(false);
                        ui.set_page(2);
                        ui.set_scroll_y(0.);
                        control(&ui, "扫描本地占用", || {
                            ui.global::<crate::MaintenanceState>()
                                .invoke_action("scan".into())
                        })?;
                        step.set(10);
                    }
                    10 if !ui.global::<crate::MaintenanceState>().get_busy() => {
                        let maintenance = ui.global::<crate::MaintenanceState>();
                        if !maintenance.get_can_clean() {
                            return Err("storage-scan-missing-candidates".into());
                        }
                        save("maintenance-update")?;
                        ui.set_scroll_y(-460.);
                        save("maintenance-storage")?;
                        control(&ui, "清理可回收文件", || {
                            maintenance.invoke_action("clean".into())
                        })?;
                        step.set(11);
                    }
                    11 => {
                        let maintenance = ui.global::<crate::MaintenanceState>();
                        if maintenance.get_confirmation() != "clean"
                            || !ui.get_maintenance_cancel_focused()
                        {
                            return Err("storage-confirmation-focus".into());
                        }
                        save("maintenance-confirmation")?;
                        control(&ui, "取消", || maintenance.invoke_action("cancel".into()))?;
                        if !maintenance_root.join("client-data/versions/1.0.0").exists() {
                            return Err("storage-cancel-deleted".into());
                        }
                        control(&ui, "清理可回收文件", || {
                            maintenance.invoke_action("clean".into())
                        })?;
                        control(&ui, "确认", || {
                            maintenance.invoke_action("confirm".into())
                        })?;
                        if !session.busy() || session.set_running(true).is_ok() {
                            return Err("maintenance-session-reservation".into());
                        }
                        step.set(12);
                    }
                    12 if !ui.global::<crate::MaintenanceState>().get_busy() => {
                        if !crate::maintenance_review::preserved(&maintenance_root) {
                            return Err("storage-preservation".into());
                        }
                        session.set_running(true)?;
                        ui.global::<crate::MaintenanceState>()
                            .invoke_action("check".into());
                        if session.busy()
                            || !ui
                                .global::<crate::MaintenanceState>()
                                .get_update_status()
                                .contains("正在检查")
                        {
                            return Err("update-check-blocked-recognition".into());
                        }
                        step.set(13);
                    }
                    13 if !ui.global::<crate::MaintenanceState>().get_busy() => {
                        let maintenance = ui.global::<crate::MaintenanceState>();
                        if maintenance.get_installable()
                            || !maintenance.get_notice().contains("离线验证")
                            || !maintenance.get_update_status().contains("离线验证")
                        {
                            return Err("maintenance-offline-check".into());
                        }
                        session.set_running(false)?;
                        save("maintenance-complete")?;
                        ui.window().set_size(slint::LogicalSize::new(920., 760.));
                        ui.set_scroll_y(0.);
                        step.set(14);
                    }
                    14 if ui.get_content_width() < 800. => {
                        save("maintenance-narrow")?;
                        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }
                            != before
                        {
                            return Err("maintenance-foreground".into());
                        }
                        ui.set_page(1);
                        ui.set_scroll_y(0.);
                        state.invoke_history_filter("".into(), 0);
                        step.set(15);
                    }
                    15 if records.idle() => {
                        let row = state.get_history().row_data(0).ok_or("record-row")?;
                        if row.icon.size().width == 0 || row.items.row_count() == 0 {
                            return Err("record-icons-missing".into());
                        }
                        let record = ui.global::<crate::RecordState>();
                        control(&ui, "查看详情", || {
                            record.invoke_action("select".into(), 0)
                        })?;
                        if record.get_observations().row_count() != 5
                            || !record
                                .get_observations()
                                .row_data(0)
                                .unwrap()
                                .cards
                                .contains("此位置未识别")
                        {
                            return Err("record-observation-boundary".into());
                        }
                        ui.set_scroll_y(-360.);
                        save("records-detail")?;
                        control(&ui, "下一组", || {
                            record.invoke_action("observation-page".into(), 1)
                        })?;
                        if record.get_observations().row_count() != 3 {
                            return Err("observation-pagination".into());
                        }
                        ui.set_scroll_y(-610.);
                        save("records-observations")?;
                        control(&ui, "生成匿名分享预览", || {
                            record.invoke_action("share".into(), 0)
                        })?;
                        if !session.busy() || session.set_running(true).is_ok() {
                            return Err("share-reservation".into());
                        }
                        step.set(16);
                    }
                    16 if records.idle() => {
                        let record = ui.global::<crate::RecordState>();
                        if !record.get_share_ready() {
                            return Err("share-preview-missing".into());
                        }
                        let (png, json) = records.share_bytes();
                        let value: serde_json::Value = serde_json::from_slice(&json)?;
                        let text = String::from_utf8(json)?;
                        if !value["actualClaimRound"].is_null()
                            || value["recommendationScope"] != "overall"
                            || value["candidateObservations"].as_array().map(|a| a.len()) != Some(8)
                            || [
                                "demo0001",
                                "HN1",
                                "game_id",
                                "observed_at",
                                "created",
                                "platform",
                            ]
                            .iter()
                            .any(|s| text.contains(s))
                        {
                            return Err("share-privacy-boundary".into());
                        }
                        std::fs::write(destination.join("records-anonymous-share.png"), png)?;
                        ui.set_scroll_y(-1010.);
                        save("records-share")?;
                        record.invoke_action("copy".into(), 0);
                        if !record.get_notice().contains("离线验证") {
                            return Err("clipboard-offline-boundary".into());
                        }
                        record.invoke_action("select".into(), 1);
                        if record.get_share_ready()
                            || record.get_observations().row_count() != 0
                            || !records.share_bytes().0.is_empty()
                        {
                            return Err("record-selection-stale".into());
                        }
                        record.set_notice("".into());
                        ui.set_page(2);
                        ui.set_scroll_y(0.);
                        records.preview_settings_fixture(settings_fixture.clone())?;
                        step.set(17);
                    }
                    17 if records.idle() => {
                        step.set(18);
                    }
                    18 => {
                        let record = ui.global::<crate::RecordState>();
                        if record.get_confirmation() != "settings"
                            || !ui.get_record_cancel_focused()
                        {
                            return Err("settings-preview-focus".into());
                        }
                        save("records-settings-confirmation")?;
                        control(&ui, "取消", || {
                            record.invoke_action("cancel-confirm".into(), 0)
                        })?;
                        if session.snapshot().preferences.champ_topmost {
                            return Err("settings-cancel-applied".into());
                        }
                        records.preview_settings_fixture(settings_fixture.clone())?;
                        step.set(19);
                    }
                    19 if records.idle() => {
                        control(&ui, "确认导入", || {
                            ui.global::<crate::RecordState>()
                                .invoke_action("confirm".into(), 0)
                        })?;
                        step.set(20);
                    }
                    20 if !session.busy() => {
                        let p = session.snapshot().preferences;
                        if !p.champ_topmost || p.locale == "en-US" || state.get_running() {
                            return Err("settings-import-policy".into());
                        }
                        state.invoke_preference("private_updates".into(), false);
                        step.set(21);
                    }
                    21 if !session.busy()
                        && !ui.global::<crate::MaintenanceState>().get_private_channel() =>
                    {
                        if session.snapshot().preferences.private_updates {
                            return Err("update-channel-not-persisted".into());
                        }
                        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }
                            != before
                        {
                            return Err("record-foreground-changed".into());
                        }
                        step.set(22);
                    }
                    22 if closure.borrow_mut().poll(&destination)? => {
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
    let report = serde_json::json!({"hidden_window":true,"foreground_unchanged":true,"game_access":false,"system_input":false,"personal_data_read":false,"host_preferences_apply":true,"invalid_preferences_preserved":true,"history_detail_and_paging":true,"offline_network_command_rejected":true,"pause_resume":true,"champion_latest_request":true,"augment_filter":true,"public_build_details":true,"bench_replace_and_clear":true,"detail_idle_timer_stopped":true,"history_filter":true,"close_fallback_and_cancel":true,"close_default_focus":true,"tray_quit_confirmation_callback":true,"confirm_stops_session":true,"native_shell_hidden":true,"tray_registration_tested":false,"global_hotkey_registered":false,"disk_save_verified":false,"production_parity_complete":false,"storage_scan_clean_preservation":true,"maintenance_offline_network_rejected":true,"maintenance_confirmation_cancel_focus":true});
    let mut report = report;
    report["record_icons_and_observation_paging"] = true.into();
    report["service_actions_via_component_hits"] = cfg!(feature = "ui-audit").into();
    report["tray_menu_ids_map_to_each_action"] = true.into();
    report["component_pointer_navigation_two_widths"] = true.into();
    report["component_pointer_modal_cancel"] = true.into();
    report["component_navigation_keyboard"] = true.into();
    report["immediate_pending_and_fixed_feedback"] = true.into();
    report["anonymous_share_preview"] = true.into();
    report["record_selection_clears_previous"] = true.into();
    report["clipboard_offline_rejected"] = true.into();
    report["settings_import_cancel_confirm"] = true.into();
    report["update_channel_preference_synced"] = true.into();
    std::fs::write(
        output.join("host-review.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
