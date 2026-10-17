//! Public bundled data through the real detail callback. Hidden own window, offline only.
use crate::{champion_bridge, ChampionWindow};
use slint::{ComponentHandle, Model};
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

pub fn run(
    data: &Path,
    output: Option<&Path>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let output = output.map(Path::to_path_buf);
    if let Some(p) = &output {
        std::fs::create_dir_all(p)?;
    }
    let roots = [fuze_data::Root {
        path: data.into(),
        source: fuze_data::Source::Bundled,
    }];
    let prepared =
        fuze_data::prepare(&roots, fuze_data::Locale::ZhCn, fuze_data::Scope::Champions)?;
    let mut ids: Vec<u32> = prepared.document("champions.json").ok_or("champions")?["data"]
        .as_array()
        .ok_or("champions")?
        .iter()
        .filter_map(|v| v["id"].as_u64().map(|n| n as u32))
        .collect();
    let mut all = fuze_data::detail_cache::Cache::prepare(&roots, &prepared.identity())?;
    let hero_count = ids.len();
    for id in &ids {
        let _ = all.get(*id)?;
    }
    // The reported empty hero must traverse the real callback, not just a JSON check.
    if let Some(index) = ids.iter().position(|id| *id == 25) {
        ids.swap(0, index);
    }
    ids.truncate(20);
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
    ui.window().set_size(slint::LogicalSize::new(420., 800.));
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
                if order[index] == 25 && ui.get_augments().row_count()==0 {
                    return Err("morgana-augments-empty".into());
                }
                timings.push(began.elapsed().as_secs_f64() * 1000.);
                if index == 0 || index == order.len() - 1 {
                    ui.set_detail_tab(1);
                    let paint = Instant::now();
                    let _=ui.window().take_snapshot()?; // resolve layout before scrolling
                    ui.set_scroll_y(-ui.get_detail_offset());
                    let pixels = ui.window().take_snapshot()?;
                    if index == 0 {
                        if let Some(output)=&output {
                            image::save_buffer(output.join("morgana-build.png"),pixels.as_bytes(),pixels.width(),pixels.height(),image::ColorType::Rgba8)?;
                            ui.set_detail_tab(0);
                            let pixels=ui.window().take_snapshot()?;
                            image::save_buffer(output.join("morgana-augments.png"),pixels.as_bytes(),pixels.width(),pixels.height(),image::ColorType::Rgba8)?;
                        }
                    }
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
                        "offscreen_raster_ms":paint_ms,"returned_after_twelve":true,"all_hero_details":hero_count,
                        "first_hero":order[0]
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
