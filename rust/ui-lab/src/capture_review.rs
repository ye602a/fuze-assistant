//! Real OCR deliveries through the production Manager and native three-card window.
use fuze_runtime::session::Phase;
use fuze_worker_host::desktop::Status;
use std::{path::Path, time::Instant};
pub fn run(args: &[String]) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if args.len() != 9 {
        return Err("capture-review-arguments".into());
    }
    let output = Path::new(&args[8]);
    std::fs::create_dir_all(output)?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|a| a.with_active(false))
        .select()?;
    let roots = vec![fuze_data::Root {
        path: args[5].clone().into(),
        source: fuze_data::Source::Bundled,
    }];
    let mut manager = crate::game_windows::Manager::new(roots, true, false);
    let foreground = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let started = Instant::now();
    let mut renders = 0;
    // Same threading boundary as production: capture/WinRT off the UI thread.
    let inputs = args[..8].to_vec();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let (ack, acknowledged) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let present = |status: Status, index| {
            send.send((status, index))
                .map_err(|_| "capture-review-ui-closed")?;
            acknowledged
                .recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|_| "capture-review-ui-timeout")?;
            Ok(())
        };
        if inputs[6].starts_with("lifecycle") {
            fuze_worker_host::lifecycle_review::run(&inputs, present)
        } else {
            fuze_worker_host::capture_review::run(&inputs, |pipeline, delivery, index| {
                let mut status = Status::default();
                status.mode = 1;
                status.preferences.augment_overlay = true;
                status.preferences.icon_downloads = false;
                status.connection = Some(fuze_lcu::reconnect::State::Connected(
                    if delivery.is_some() {
                        Phase::InProgress
                    } else {
                        Phase::PostGame
                    },
                ));
                status.pipeline.overlay = pipeline.frames.current();
                present(status, index)
            })
        }
        .map_err(|e| e.to_string())
    });
    for (status, index) in receive {
        manager.update(&status)?;
        if let Some(frame) = status.pipeline.overlay {
            let cards = manager
                .cards
                .as_mut()
                .ok_or("capture-review-window-missing")?;
            let expected = frame.snapshot.slots.each_ref().map(|s| match s {
                fuze_contract::Slot::Known { id, .. } => Some(*id),
                _ => None,
            });
            if cards.ids() != expected || !cards.hidden() {
                return Err("capture-review-ui-slots".into());
            }
            if index % 3 == 0 {
                cards.render(&output.join(format!("cards-{index}.png")))?;
                renders += 1;
            }
        } else if let Some(cards) = manager.cards.as_ref() {
            if cards.ids() != [None; 3] || !cards.hidden() {
                return Err("capture-review-ui-clear".into());
            }
        }
        ack.send(()).map_err(|_| "capture-review-worker-closed")?;
    }
    let mut report = worker
        .join()
        .map_err(|_| "capture-review-worker-panicked")??;
    if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } != foreground {
        return Err("capture-review-foreground-changed".into());
    }
    report["production_ui_publisher"] = true.into();
    report["hidden_native_popup_renders"] = renders.into();
    report["foreground_unchanged"] = true.into();
    report["elapsed_ms"] = (started.elapsed().as_secs_f64() * 1000.).into();
    std::fs::write(
        output.join("capture-review.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
