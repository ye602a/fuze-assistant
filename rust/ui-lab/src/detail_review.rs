//! Public bundled data through the real detail callback. Hidden own window, offline only.
use crate::{champion_bridge, ChampionWindow};
use slint::{ComponentHandle, Model};
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

pub fn run(data: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let roots = [fuze_data::Root {
        path: data.into(),
        source: fuze_data::Source::Bundled,
    }];
    let prepared =
        fuze_data::prepare(&roots, fuze_data::Locale::ZhCn, fuze_data::Scope::Champions)?;
    let ids: Vec<u32> = prepared.document("champions.json").ok_or("champions")?["data"]
        .as_array()
        .ok_or("champions")?
        .iter()
        .filter_map(|v| v["id"].as_u64().map(|n| n as u32))
        .take(20)
        .collect();
    if ids.len() < 12 {
        return Err("detail-review-needs-twelve-public-heroes".into());
    }
    let catalog = fuze_worker_host::champ_select::Catalog::load(&roots, fuze_data::Locale::ZhCn)?;
    let selection = fuze_lcu::context::Selection {
        team: std::array::from_fn(|i| ids[i]),
        local_slot: Some(0),
        bench: std::array::from_fn(|i| ids.get(i + 5).copied().unwrap_or(0)),
        count: ids.len() - 5,
    };
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
    let ui = ChampionWindow::new()?;
    ui.set_offline(true);
    ui.set_inspected_id(0);
    let binding = champion_bridge::attach(&ui, data)?;
    let mut order: Vec<_> = ids.into_iter().take(12).collect();
    order.push(order[0]); // Revisit after more than eight different heroes.
    let mut index = 0;
    let mut began = Instant::now();
    let start = began;
    binding.selection(&ui, Some(&catalog.view(selection)));
    let result = Rc::new(RefCell::new(None));
    let returned = result.clone();
    let mut timings = Vec::new();
    let mut paint_ms = Vec::new();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(10),
        move || {
            let checked = (|| -> Result<bool, Box<dyn std::error::Error>> {
                if start.elapsed() > Duration::from_secs(30) {
                    return Err("details-timeout".into());
                }
                if ui.get_detail_champion() != order[index] as i32 {
                    return Ok(false);
                }
                if ui.get_build_lines().row_count() == 0 || !binding.idle() {
                    return Err("details-incomplete".into());
                }
                timings.push(began.elapsed().as_secs_f64() * 1000.);
                if index == 0 || index == order.len() - 1 {
                    ui.set_detail_tab(1);
                    let paint = Instant::now();
                    let _ = ui.window().take_snapshot()?;
                    paint_ms.push(paint.elapsed().as_secs_f64() * 1000.);
                }
                index += 1;
                if index == order.len() {
                    return Ok(true);
                }
                began = Instant::now();
                ui.invoke_inspect(order[index] as i32);
                Ok(false)
            })();
            match checked {
                Ok(false) => (),
                Ok(true) => {
                    *result.borrow_mut() = Some(Ok(serde_json::json!({
                        "offline":true,"hidden":true,"network":false,"selections":timings,
                        "offscreen_raster_ms":paint_ms,"returned_after_twelve":true
                    })));
                    let _ = slint::quit_event_loop();
                }
                Err(error) => {
                    *result.borrow_mut() = Some(Err(error));
                    let _ = slint::quit_event_loop();
                }
            }
        },
    );
    slint::run_event_loop_until_quit()?;
    if before != unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } {
        return Err("foreground-changed".into());
    }
    let output = returned
        .borrow_mut()
        .take()
        .ok_or("details-not-completed")??;
    Ok(output)
}
