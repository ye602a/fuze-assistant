use crate::Workbench;
use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, WindowAdapter,
    },
    ComponentHandle, Rgb8Pixel,
};
use std::{path::Path, rc::Rc, time::Instant};

struct Offscreen(Rc<MinimalSoftwareWindow>);
#[test]
fn champion_subset_offscreen() {
    use slint::{Model, VecModel};
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
    let ui = crate::ChampionWindow::new().unwrap();
    ui.set_offline(false);
    let mut team = ui.get_team().iter().collect::<Vec<_>>();
    for index in [1, 3, 4] {
        team[index].id = 0;
        team[index].name = "等待选择".into();
        team[index].icon = Default::default();
        team[index].win = "—".into();
    }
    let cards = ui.get_bench().iter().take(3).collect::<Vec<_>>();
    ui.set_team(Rc::new(VecModel::from(team)).into());
    ui.set_local_slot(3);
    ui.set_local_id(0);
    ui.set_inspected_id(0);
    ui.set_inspected_name("等待选择英雄".into());
    ui.set_inspected_icon(Default::default());
    ui.set_subset_active(true);
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(420, 640));
    for (name, count, active) in [
        ("three", 3, true),
        ("two", 2, true),
        ("unavailable", 0, true),
        ("ordinary", 0, false),
    ] {
        ui.set_choices(Rc::new(VecModel::from(cards[..count].to_vec())).into());
        ui.set_subset_active(active);
        let mut pixels = vec![Rgb8Pixel::default(); 420 * 640];
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut pixels, 420);
        });
        assert!(pixels.iter().filter(|p| p.g > 20).count() > 50_000);
        if let Some(output) = std::env::var_os("FUZE_CAPTURE_REVIEW") {
            image::save_buffer(
                Path::new(&output).join(format!("champion-{name}.png")),
                &pixels
                    .iter()
                    .flat_map(|p| [p.r, p.g, p.b])
                    .collect::<Vec<_>>(),
                420,
                640,
                image::ColorType::Rgb8,
            )
            .unwrap();
        }
    }
}

#[test]
fn release_label_offscreen() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
    let ui = Workbench::new().unwrap();
    ui.window().set_size(slint::LogicalSize::new(920., 760.));
    ui.global::<crate::HostState>().set_offline(false);
    ui.global::<crate::MaintenanceState>()
        .set_release_label("0.7.1 · 测试渠道".into());
    let mut pixels = vec![Rgb8Pixel::default(); 920 * 760];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut pixels, 920);
    });
    assert!(pixels.iter().filter(|p| p.g > 20).count() > 100_000);
    if let Some(output) = std::env::var_os("FUZE_CAPTURE_REVIEW") {
        image::save_buffer(
            std::path::Path::new(&output).join("release-label.png"),
            &pixels
                .iter()
                .flat_map(|p| [p.r, p.g, p.b])
                .collect::<Vec<_>>(),
            920,
            760,
            image::ColorType::Rgb8,
        )
        .unwrap();
    }
}

#[test]
fn recognition_log_settings_offscreen() {
    // Actual Slint layout, in memory only: no HWND, engine, desktop pixels or network.
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
    let ui = Workbench::new().unwrap();
    ui.set_page(2);
    ui.set_settings_section(2);
    ui.global::<crate::HostState>().set_connected(true);
    ui.global::<crate::HostState>().set_offline(false);
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(920, 760));
    for (name, enabled, message) in [
        ("disabled", false, "本次运行已关闭，已有文件保留；下次启动默认开启。 已保存 0 条，队列繁忙跳过 0 条。"),
        ("enabled", true, "已开启，等待前台主屏游戏画面；关闭或退出后停止采样。 已保存 12 条，队列繁忙跳过 0 条。"),
        ("limit", false, "已到 1200 条或 256 MiB 上限，请先移走目录内的旧记录，再重启后开启。 已保存 1200 条，队列繁忙跳过 0 条。"),
    ] {
        let state = ui.global::<crate::DiagnosticState>();
        state.set_screenshots_enabled(enabled);
        state.set_screenshots_recent(enabled);
        state.set_screenshots_status(message.into());
        ui.set_scroll_y(-360.);
        let mut pixels = vec![Rgb8Pixel::default(); 920 * 760];
        window.request_redraw();
        window.draw_if_needed(|r| { r.render(&mut pixels, 920); });
        assert!(pixels.iter().filter(|p| p.g > 20).count() > 100_000);
        if let Some(output) = std::env::var_os("FUZE_CAPTURE_REVIEW") {
            image::save_buffer(std::path::Path::new(&output).join(format!("captures-{name}.png")),
                &pixels.iter().flat_map(|p|[p.r,p.g,p.b]).collect::<Vec<_>>(), 920,760,image::ColorType::Rgb8).unwrap();
        }
    }
}
/// Initializes the real Windows backend without ever calling show/run on the component.
/// Snapshot rendering is rasterized and MUST NOT be presented as GPU timing.
pub fn native_review(
    renderer: &str,
    output: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use slint::winit_030::WinitWindowAccessor;
    use std::cell::RefCell;
    if !matches!(renderer, "software" | "skia" | "skia-software")
        || (renderer != "software" && !cfg!(feature = "gpu"))
    {
        return Err("unsupported renderer or missing gpu feature".into());
    }
    std::fs::create_dir_all(output)?;
    let before = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let start = Instant::now();
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(renderer.into())
        .with_winit_window_attributes_hook(|attributes| {
            attributes.with_visible(false).with_active(false)
        })
        .select()?;
    let ui = Workbench::new()?;
    connect(&ui);
    ui.window().set_size(slint::LogicalSize::new(1120., 860.));
    ui.set_query("格温／繁體中文 · 中文输入需手动验证".into());
    let result = Rc::new(RefCell::new(None));
    let returned = result.clone();
    let weak = ui.as_weak();
    let destination = output.to_owned();
    let name = renderer.to_string();
    slint::Timer::single_shot(std::time::Duration::from_millis(400), move || {
        let checked = (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
            let ui = weak.upgrade().ok_or("component destroyed")?;
            let visible = ui
                .window()
                .with_winit_window(|w| w.is_visible())
                .ok_or("native window not created")?;
            if visible != Some(false) {
                return Err("hidden native window invariant failed".into());
            }
            let at = Instant::now();
            let pixels = ui.window().take_snapshot()?;
            let snapshot_ms = at.elapsed().as_secs_f64() * 1000.;
            image::save_buffer(
                destination.join(format!("slint-native-{name}.png")),
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                image::ColorType::Rgba8,
            )?;
            let cpu_before = crate::telemetry::cpu_ms();
            let mut timings = Vec::new();
            for state in 0..12 {
                ui.set_health(state % 4);
                let at = Instant::now();
                let frame = ui.window().take_snapshot()?;
                std::hint::black_box(frame);
                timings.push(at.elapsed().as_secs_f64() * 1000.);
            }
            let cpu_after = crate::telemetry::cpu_ms();
            let cpu_ms = if cpu_before >= 0. && cpu_after >= cpu_before {
                Some(cpu_after - cpu_before)
            } else {
                None
            };
            let mut resize_checks = Vec::new();
            for (width, height) in [(920, 760), (1120, 860)] {
                let logical = slint::LogicalSize::new(width as f32, height as f32);
                let expected = logical.to_physical(ui.window().scale_factor());
                ui.window().set_size(logical);
                let actual = ui
                    .window()
                    .with_winit_window(|w| (w.inner_size(), w.is_visible()))
                    .ok_or("native window lost during resize")?;
                if actual.0.width != expected.width
                    || actual.0.height != expected.height
                    || actual.1 != Some(false)
                {
                    return Err("native hidden resize mismatch".into());
                }
                resize_checks.push(serde_json::json!({"logical_width":width,"logical_height":height,"physical_width":actual.0.width,"physical_height":actual.0.height,"still_hidden":true}));
            }
            let foreground_unchanged =
                before == unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
            // Fixed model transitions only, never OS input or access to another window.
            for (name, width, height, state, paused, bottom, quit) in [
                ("home", 1120, 860, 0, false, false, false),
                ("ready", 1120, 860, 1, false, false, false),
                ("partial", 1120, 860, 2, false, false, false),
                ("error", 1120, 860, 3, false, false, false),
                ("paused", 1120, 860, 0, true, false, false),
                ("home-bottom", 1120, 860, 0, false, true, false),
                ("quit", 1120, 860, 0, false, false, true),
            ] {
                ui.window()
                    .set_size(slint::LogicalSize::new(width as f32, height as f32));
                ui.set_health(state);
                ui.set_paused(paused);
                ui.set_quit_open(quit);
                ui.set_runtime_open(bottom);
                ui.set_level_open(bottom);
                ui.set_sources_open(bottom);
                ui.set_scroll_y(0.);
                // First layout determines the new scroll extent.
                let _ = ui.window().take_snapshot()?;
                if bottom {
                    ui.set_scroll_y(-520.);
                }
                let pixels = ui.window().take_snapshot()?;
                let expected = slint::LogicalSize::new(width as f32, height as f32)
                    .to_physical(ui.window().scale_factor());
                if pixels.width() != expected.width || pixels.height() != expected.height {
                    return Err("layout image size differs from requested window".into());
                }
                image::save_buffer(
                    destination.join(format!("restored-{name}.png")),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                    image::ColorType::Rgba8,
                )?;
                if ui.get_module_width() < 180. {
                    return Err("restored module width too small".into());
                }
            }
            ui.set_quit_open(false);
            for (name, page, section, bottom, sample) in [
                ("settings", 2, 0, false, false),
                ("settings-bottom", 2, 0, true, false),
                ("settings-data", 2, 1, false, false),
                ("settings-help", 2, 2, false, false),
                ("history-empty", 1, 0, false, false),
                ("history-sample", 1, 0, false, true),
            ] {
                ui.set_page(page);
                ui.set_settings_section(section);
                ui.set_history_sample(sample);
                ui.set_scroll_y(0.);
                let _ = ui.window().take_snapshot()?;
                if bottom {
                    ui.set_scroll_y(-1200.);
                }
                let pixels = ui.window().take_snapshot()?;
                image::save_buffer(
                    destination.join(format!("restored-{name}.png")),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                    image::ColorType::Rgba8,
                )?;
            }
            ui.set_page(0);
            ui.set_history_sample(false);
            let champion = crate::ChampionWindow::new()?;
            champion
                .window()
                .set_size(slint::LogicalSize::new(420., 640.));
            for (name, offset, tab) in [
                ("champion", 0., 0),
                ("champion-detail", -480., 0),
                ("champion-build", -480., 1),
            ] {
                champion.set_scroll_y(offset);
                champion.set_detail_tab(tab);
                let pixels = champion.window().take_snapshot()?;
                image::save_buffer(
                    destination.join(format!("restored-{name}.png")),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                    image::ColorType::Rgba8,
                )?;
            }
            let champion_visibility = champion
                .window()
                .with_winit_window(|w| w.is_visible())
                .flatten();
            if champion_visibility == Some(true) {
                return Err("champion preview unexpectedly visible".into());
            }
            ui.set_level_open(false);
            ui.set_sources_open(false);
            ui.set_runtime_open(false);
            ui.set_scroll_y(0.);
            Ok(
                serde_json::json!({"champion_native_visibility_observed":champion_visibility,"requested_backend":name,"native_window_created":true,"visible":false,"foreground_unchanged_observed":foreground_unchanged,"hidden_native_resize":resize_checks,"elapsed_including_400ms_timer":start.elapsed().as_secs_f64()*1000.,"first_raster_snapshot_ms":snapshot_ms,"warm_raster_snapshot":crate::telemetry::samples(&timings),"warm_process_cpu_ms":cpu_ms,"gpu_performance_verified":false,"real_ime_verified":false}),
            )
        })();
        let ok = checked.is_ok();
        *returned.borrow_mut() = Some(checked);
        if !ok {
            let _ = slint::quit_event_loop();
            return;
        }
        let Some(ui) = weak.upgrade() else {
            let _ = slint::quit_event_loop();
            return;
        };
        ui.window().set_size(slint::LogicalSize::new(920., 760.));
        // Yield to the real Winit resize event before snapshotting the narrow layout.
        slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
            let narrow = (|| -> Result<(), Box<dyn std::error::Error>> {
                let ui = weak
                    .upgrade()
                    .ok_or("component destroyed before narrow review")?;
                let pixels = ui.window().take_snapshot()?;
                let expected =
                    slint::LogicalSize::new(920., 760.).to_physical(ui.window().scale_factor());
                if pixels.width() != expected.width
                    || pixels.height() != expected.height
                    || ui.get_module_width() < 300.
                {
                    return Err("narrow window/layout did not process resize".into());
                }
                image::save_buffer(
                    destination.join("restored-home-narrow.png"),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                    image::ColorType::Rgba8,
                )?;
                Ok(())
            })();
            if let Err(e) = narrow {
                *returned.borrow_mut() = Some(Err(e));
            } else if let Some(Ok(value)) = returned.borrow_mut().as_mut() {
                value["narrow_real_event_layout_verified"] = true.into();
            }
            let _ = slint::quit_event_loop();
        });
    });
    slint::run_event_loop_until_quit()?;
    let value = result
        .borrow_mut()
        .take()
        .ok_or("native callback did not complete")??;
    std::fs::write(
        output.join(format!("native-{renderer}.json")),
        serde_json::to_vec_pretty(&value)?,
    )?;
    Ok(value)
}
impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}
pub fn connect(ui: &Workbench) {
    use slint::winit_030::WinitWindowAccessor;
    let weak = ui.as_weak();
    ui.on_request_close(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_quit_open(true);
        }
    });
    let weak = ui.as_weak();
    ui.on_minimize(move || {
        if let Some(ui) = weak.upgrade() {
            crate::motion::refresh(ui.global::<crate::UiMotion>(), false);
            ui.window()
                .with_winit_window(|window| window.set_minimized(true));
        }
    });
    let weak = ui.as_weak();
    ui.on_drag_window(move || {
        if let Some(ui) = weak.upgrade() {
            ui.window().with_winit_window(|window| {
                let _ = window.drag_window();
            });
        }
    });
    ui.on_close_requested(|| {
        let _ = slint::quit_event_loop();
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_quit_open(true);
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let weak = ui.as_weak();
    ui.on_cycle_health(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_health((ui.get_health() + 1) % 4);
        }
    });
    // Text remains in the widget model only; no files, telemetry or network.
    ui.on_query_edited(|_| {});
    let weak = ui.as_weak();
    let corners =
        std::cell::RefCell::new(crate::window_style::Corners::for_window(ui.window(), false));
    ui.window().on_winit_window_event(move |_, event| {
        use slint::winit_030::{winit::event::WindowEvent, EventResult};
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            if let Some(ui) = weak.upgrade() {
                if let Some(handle) = crate::lifecycle::hwnd(ui.window()) {
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
                crate::motion::refresh(ui.global::<crate::UiMotion>(), *focused);
            }
        }
        EventResult::Propagate
    });
}
pub fn review(output: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let start = Instant::now();
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone())))?;
    let ui = Workbench::new()?;
    connect(&ui);
    let input = "格温／深海泰坦 · 繁體中文 · Gwen 56.3%";
    ui.set_query(input.into());
    if ui.get_query() != input {
        return Err("Chinese model roundtrip failed".into());
    }
    ui.show()?;
    let startup_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut cases = Vec::new();
    for (width, height, scale) in [
        (1120u32, 860u32, 1.0f32),
        (1120, 860, 1.25),
        (1120, 860, 1.5),
        (1120, 860, 1.75),
        (1120, 860, 2.0),
        (920, 760, 1.0),
    ] {
        let size = slint::PhysicalSize::new(
            (width as f32 * scale) as u32,
            (height as f32 * scale) as u32,
        );
        window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        window.set_size(size);
        let mut pixels = vec![Rgb8Pixel::default(); (size.width * size.height) as usize];
        let mut timings = Vec::new();
        for i in 0..12 {
            ui.set_health(i % 4);
            window.request_redraw();
            let at = Instant::now();
            window.draw_if_needed(|r| {
                r.render(&mut pixels, size.width as usize);
            });
            timings.push(at.elapsed().as_secs_f64() * 1000.0);
        }
        ui.set_health(0);
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut pixels, size.width as usize);
        });
        let mut idle_redraws = 0;
        for _ in 0..20 {
            if window.draw_if_needed(|r| {
                r.render(&mut pixels, size.width as usize);
            }) {
                idle_redraws += 1;
            }
        }
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        if bytes.iter().all(|b| *b == 0) {
            return Err("empty Slint image".into());
        }
        image::save_buffer(
            output.join(format!("slint-{width}-{}.png", (scale * 100.0) as u32)),
            &bytes,
            size.width,
            size.height,
            image::ColorType::Rgb8,
        )?;
        cases.push(serde_json::json!({"logical_width":width,"scale":scale,"physical_width":size.width,"render":crate::telemetry::samples(&timings),"idle_redraws":idle_redraws,"content_width":ui.get_content_width(),"module_width":ui.get_module_width()}));
        if idle_redraws != 0 {
            return Err("unexpected continuous Slint redraw".into());
        }
    }
    // Exercise actual property transitions without an OS window or frame-rate loop in production.
    let motion = ui.global::<crate::UiMotion>();
    motion.set_enabled(true);
    ui.set_health(1);
    let mut pixels = vec![Rgb8Pixel::default(); 920 * 760];
    let mut motion_frames = 0;
    for _ in 0..15 {
        slint::platform::update_timers_and_animations();
        if window.draw_if_needed(|r| {
            r.render(&mut pixels, 920);
        }) {
            motion_frames += 1;
        }
        // A duplicate host update must not restart the finite transition.
        ui.set_health(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if ui.window().has_active_animations() {
        return Err("motion-did-not-settle".into());
    }
    motion.set_enabled(false);
    ui.set_health(0);
    slint::platform::update_timers_and_animations();
    window.draw_if_needed(|r| {
        r.render(&mut pixels, 920);
    });
    if ui.window().has_active_animations() {
        return Err("reduced-motion-not-immediate".into());
    }
    ui.invoke_cycle_health();
    if ui.get_health() != 1 {
        return Err("state callback failed".into());
    }
    ui.hide()?;
    Ok(
        serde_json::json!({"renderer":"software_offscreen","startup_ms":startup_ms,"cases":cases,"motion_frames":motion_frames,"duplicate_updates_settle":true,"reduced_motion_immediate":true,"chinese_model_roundtrip":true,"real_ime_verified":false,"native_window_verified":false,"hardware_dpi_verified":false}),
    )
}
