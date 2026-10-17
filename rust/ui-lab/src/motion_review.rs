//! Deterministic component transitions; no OS window, wall-clock sleeps or game access.
use crate::{MotionReview, UiMotion};
use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, WindowAdapter,
    },
    ComponentHandle, Rgb8Pixel,
};
use std::{cell::Cell, path::Path, rc::Rc, time::Duration};
struct Backend {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<u64>>,
}
impl Platform for Backend {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        Duration::from_millis(self.clock.get())
    }
}
pub fn run(output: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(output)?;
    let clock = Rc::new(Cell::new(0));
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(Backend {
        window: window.clone(),
        clock: clock.clone(),
    }))?;
    let ui = MotionReview::new()?;
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(640, 320));
    let mut pixels = vec![Rgb8Pixel::default(); 640 * 320];
    let mut step = |ms: u64, name: Option<&str>| -> Result<bool, Box<dyn std::error::Error>> {
        clock.set(ms);
        slint::platform::update_timers_and_animations();
        let drew = window.draw_if_needed(|r| {
            r.render(&mut pixels, 640);
        });
        if let Some(name) = name {
            if name == "sweep-middle" {
                let left = pixels[100 * 640 + 24];
                if left.g <= left.b {
                    return Err("sweep-did-not-start-at-left-edge".into());
                }
            }
            let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
            image::save_buffer(
                output.join(format!("{name}.png")),
                &bytes,
                640,
                320,
                image::ColorType::Rgb8,
            )?;
        }
        Ok(drew)
    };
    step(0, Some("waiting"))?;
    let motion = ui.global::<UiMotion>();
    motion.set_enabled(true);
    step(1, None)?;
    ui.set_health(1);
    step(10, Some("sweep-start"))?;
    step(26, None)?;
    step(96, Some("sweep-middle"))?;
    let middle = ui.get_sweep_progress();
    if !(0.55..0.90).contains(&middle) {
        return Err(format!("sweep-not-progressive:{middle}").into());
    }
    ui.set_health(1);
    step(166, Some("sweep-tail"))?;
    if ui.get_sweep_progress() <= middle {
        return Err("duplicate-restarted-sweep".into());
    }
    ui.set_health(3);
    step(167, Some("interrupted-error"))?;
    if ui.get_sweep_progress() != 1. || !ui.get_status_text().starts_with("连接异常") {
        return Err("latest-state-not-immediate".into());
    }
    ui.set_health(2);
    step(180, None)?;
    step(196, None)?;
    step(210, None)?;
    motion.set_enabled(false);
    step(211, None)?;
    if ui.get_sweep_progress() != 1.
        || ui.get_entrance_progress() != 1.
        || ui.window().has_active_animations()
    {
        return Err("disabled-motion-keeps-running".into());
    }
    motion.set_enabled(true);
    step(220, None)?;
    if ui.get_entrance_progress() != 1. {
        return Err("focus-replayed-page".into());
    }
    ui.set_page(1);
    step(230, Some("page-enter"))?;
    step(246, None)?;
    step(306, Some("page-middle"))?;
    let entrance = ui.get_entrance_progress();
    if !(0.05..0.95).contains(&entrance) {
        return Err(format!("page-not-progressive:{entrance}").into());
    }
    step(520, None)?;
    ui.set_page(0);
    step(521, None)?;
    if ui.get_entrance_progress() != 1. {
        return Err("return-replayed-page".into());
    }
    step(800, Some("settled"))?;
    let mut idle_redraws = 0;
    for i in 1..=20 {
        if step(800 + i * 20, None)? {
            idle_redraws += 1;
        }
    }
    if idle_redraws != 0 || ui.window().has_active_animations() {
        return Err("motion-idle-repaint".into());
    }
    // Disable motion while navigation/entrance are both in flight.
    ui.set_page(2);
    step(1210, None)?;
    step(1226, None)?;
    step(1266, None)?;
    motion.set_enabled(false);
    step(1267, None)?;
    if ui.window().has_active_animations() || ui.get_entrance_progress() != 1. {
        return Err("navigation-keeps-animating-while-disabled".into());
    }
    ui.hide()?;
    let report = serde_json::json!({"sweep_middle_progress":middle,"new_page_progress":entrance,"duplicate_does_not_restart":true,"interruption_latest_immediate":true,"disabled_motion_settles_immediately":true,"focus_does_not_replay":true,"return_page_does_not_replay":true,"idle_redraws":idle_redraws,"clock":"deterministic","renderer":"software_offscreen","native_gpu_performance":false,"system_input":false,"game_access":false});
    std::fs::write(
        output.join("motion.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
