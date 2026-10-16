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
/// Initializes the real Windows backend without ever calling show/run on the component.
/// Snapshot rendering is rasterized and MUST NOT be presented as GPU timing.
pub fn native_review(
    renderer: &str,
    output: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use slint::winit_030::WinitWindowAccessor;
    use std::cell::RefCell;
    if !matches!(renderer, "software" | "skia") || (renderer == "skia" && !cfg!(feature = "gpu")) {
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
            let foreground_unchanged =
                before == unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
            Ok(
                serde_json::json!({"requested_backend":name,"native_window_created":true,"visible":false,"foreground_unchanged_observed":foreground_unchanged,"elapsed_including_400ms_timer":start.elapsed().as_secs_f64()*1000.,"first_raster_snapshot_ms":snapshot_ms,"warm_raster_snapshot":crate::telemetry::samples(&timings),"warm_process_cpu_ms":cpu_ms,"gpu_performance_verified":false,"real_ime_verified":false}),
            )
        })();
        *returned.borrow_mut() = Some(checked);
        let _ = slint::quit_event_loop();
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
    let weak = ui.as_weak();
    ui.on_cycle_health(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_health((ui.get_health() + 1) % 4);
        }
    });
    // Text remains in the widget model only; no files, telemetry or network.
    ui.on_query_edited(|_| {});
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
    ui.invoke_cycle_health();
    if ui.get_health() != 1 {
        return Err("state callback failed".into());
    }
    ui.hide()?;
    Ok(
        serde_json::json!({"renderer":"software_offscreen","startup_ms":startup_ms,"cases":cases,"chinese_model_roundtrip":true,"real_ime_verified":false,"native_window_verified":false,"hardware_dpi_verified":false}),
    )
}
