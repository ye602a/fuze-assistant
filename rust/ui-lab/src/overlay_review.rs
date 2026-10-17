//! Synthetic host transitions; hidden owned windows, no game/network/input access.
use fuze_contract::{ActualStage, Slot, Snapshot};
use fuze_runtime::session::Phase;
use fuze_worker_host::{desktop::Status, overlay_frame::Publisher};
use slint::{ComponentHandle, Model};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

pub fn run(data: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let roots = [fuze_data::Root {
        path: data.into(),
        source: fuze_data::Source::Bundled,
    }];
    let prepared =
        fuze_data::prepare(&roots, fuze_data::Locale::ZhCn, fuze_data::Scope::Detail(4))?;
    let detail = prepared.champion_detail(4)?;
    let cards = detail.augments;
    if cards.len() < 3 {
        return Err("overlay-fixture-incomplete".into());
    }
    let known = |i: usize| Slot::Known {
        id: cards[i].id,
        name: cards[i].name.clone(),
        confidence: 0.95,
    };
    let mut snapshot = Snapshot {
        schema_version: 1,
        generation: 1,
        sequence: 1,
        dataset_version: prepared.identity(),
        selection_confirmed: false,
        actual_stage: ActualStage::Unknown,
        slots: [known(0), Slot::Unreadable, known(2)],
    };
    let mut publisher = Publisher::new(
        cards.clone(),
        prepared
            .recommendations(4)?
            .into_iter()
            .filter_map(|(id, r)| r.score.map(|score| (id, score)))
            .collect(),
    );
    publisher.publish(snapshot.clone(), true, false, None);
    let first = publisher.current().ok_or("overlay-frame-missing")?;
    let mut manager = crate::game_windows::Manager::new(roots.to_vec(), true, false);
    manager.preview(None)?;
    if !manager
        .sample_champion
        .as_ref()
        .ok_or("preview-champion-missing")?
        .ui
        .get_offline()
    {
        return Err("preview-client-write-enabled".into());
    }
    manager.preview_cards()?;
    if !manager
        .sample_cards
        .as_ref()
        .ok_or("preview-cards-missing")?
        .hidden()
    {
        return Err("preview-window-visible".into());
    }
    manager
        .sample_cards
        .as_ref()
        .unwrap()
        .render(&output.join("cards-preview.png"))?;
    let mut status = Status::default();
    if manager.reference(&status).is_ok() {
        return Err("reference-allowed-without-game".into());
    }
    status.mode = 1;
    status.connection = Some(fuze_lcu::reconnect::State::Connected(Phase::ChampSelect));
    status.preferences.champ_select = true;
    status.preferences.augment_side_panel = true;
    status.preferences.augment_overlay = true;
    let catalog = fuze_worker_host::champ_select::Catalog::load(&roots, fuze_data::Locale::ZhCn)?;
    let mut view = catalog.playing(4);
    view.bench = view.team.iter().take(1).cloned().collect();
    status.selection = Some(view.clone());
    manager.update(&status)?;
    if manager.sample_champion.is_some() || manager.sample_cards.is_some() {
        return Err("preview-replaced-live-context".into());
    }
    let panel = manager
        .champion
        .as_ref()
        .ok_or("selection-window-missing")?;
    if panel.ui.get_bench().row_count() != 1 || panel.ui.get_side_mode() {
        return Err("selection-view-mismatch".into());
    }
    status.connection = Some(fuze_lcu::reconnect::State::Connected(Phase::InProgress));
    status.playing = Some(view);
    status.preferences.augment_side_panel = false;
    manager.reference(&status)?;
    status.pipeline.overlay = Some(first.clone());
    manager.update(&status)?;
    if manager
        .champion
        .as_ref()
        .unwrap()
        .ui
        .get_bench()
        .row_count()
        != 0
        || !manager
            .side
            .as_ref()
            .ok_or("side-window-missing")?
            .ui
            .get_side_mode()
    {
        return Err("phase-panel-transition".into());
    }
    let overlay = manager.cards.as_mut().ok_or("cards-window-missing")?;
    if overlay.ids() != [Some(cards[0].id), None, Some(cards[2].id)] || !overlay.hidden() {
        return Err("partial-slot-or-hidden-invariant".into());
    }
    overlay.render(&output.join("host-live-cards-partial.png"))?;
    let revisions = overlay.revisions();
    overlay.dismiss();
    snapshot.sequence += 1;
    publisher.publish(snapshot.clone(), true, false, None);
    overlay.update(publisher.current(), false)?;
    if overlay.revisions() != revisions || !overlay.dismissed() {
        return Err("duplicate-repaint-or-reopen".into());
    }
    snapshot.sequence += 1;
    snapshot.slots[1] = known(1);
    publisher.publish(snapshot.clone(), true, false, None);
    let complete = publisher.current().unwrap();
    overlay.update(Some(complete.clone()), false)?;
    if overlay.dismissed() || overlay.ids()[1] != Some(cards[1].id) {
        return Err("new-choice-not-updated".into());
    }
    overlay.render(&output.join("host-live-cards-complete.png"))?;
    overlay.review_description(output)?;
    overlay.review_reference(output)?;
    if overlay.update(Some(first), false)? {
        return Err("stale-result-accepted".into());
    }
    overlay.update(None, false)?;
    if overlay.ids() != [None; 3] || !overlay.hidden() {
        return Err("clear-not-hidden".into());
    }
    // A preference toggle can re-use the exact last frame without weakening sequence rejection.
    if !overlay.update(Some(complete.clone()), false)? || overlay.ids()[1].is_none() {
        return Err("last-frame-restore".into());
    }
    let mut expired = (*complete).clone();
    expired.snapshot.sequence += 1;
    expired.deadline = Some(Instant::now() - Duration::from_millis(1));
    expired.retained = true;
    overlay.update(Some(Arc::new(expired)), false)?;
    overlay.render(&output.join("host-live-cards-retained.png"))?;
    status.connection = Some(fuze_lcu::reconnect::State::Connected(Phase::ClientReady));
    manager.update(&status)?;
    if manager.cards.as_ref().unwrap().ids() != [None; 3]
        || manager.side.as_ref().unwrap().ui.get_augments().row_count() != 0
    {
        return Err("phase-exit-stale-content".into());
    }
    std::fs::write(
        output.join("overlay-review.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "synthetic_host_transitions": true, "hidden_owned_windows": true,
            "public_data_rates": true, "partial_slots_preserved": true,
            "duplicate_model_rebuilds": 0, "dismissal_retained": true,
            "stale_sequence_rejected": true, "preference_restore": true,
            "description_long_missing_scroll_rendered": true,
            "stage_reference_four_columns_without_score_or_slot_changes": true,
            "stage_duplicate_update_does_not_repaint": true,
            "three_card_click_collapse_gap_scroll_close": true,
            "preview_without_lcu_is_separate_and_read_only": true,
            "explicit_side_panel_without_auto_show": true,
            "phase_exit_clears": true, "game_access": false, "system_input": false,
            "live_game_verified": false, "automatic_stage": "unknown_overall_only"
        }))?,
    )?;
    Ok(())
}

pub fn stage(ui: &crate::ChampionWindow, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use fuze_worker_host::stage_reference::{Reply, Row, Snapshot};
    let mut rows: [Vec<Row>; 5] = Default::default();
    rows[1] = vec![
        Row {
            id: 1,
            win: 0.58,
            pick: 0.12,
            samples: 531,
        },
        Row {
            id: 2,
            win: 0.99,
            pick: 0.1,
            samples: 199,
        },
    ];
    let reply = Reply {
        snapshot: Some(Arc::new(Snapshot {
            schema: 1,
            champion: ui.get_inspected_id() as u32,
            patch: "16.20".into(),
            date: "2026-10-15".into(),
            data_path: String::new(),
            fetched: fuze_worker_host::stage_reference::now(),
            rows,
        })),
        names: Arc::new([(1, "巨像的勇气".into()), (2, "尖端发明家".into())].into()),
        cached: true,
        issue: Some("stage-network"),
        serial: 1,
        retry_after: None,
    };
    ui.set_stage_index(1);
    crate::stage_bridge::present(ui, &reply);
    if ui.get_stage_rows().row_count() != 1
        || !ui.get_stage_status().contains("保留缓存")
        || !ui.get_stage_status().contains("地区未确认")
    {
        return Err("stage-samples-or-cache-status".into());
    }
    ui.set_stage_index(2);
    crate::stage_bridge::present(ui, &reply);
    if ui.get_stage_rows().row_count() != 0 || !ui.get_stage_status().contains("保留缓存") {
        return Err("stage-empty-no-overall-fallback".into());
    }
    ui.set_stage_index(1);
    crate::stage_bridge::present(ui, &reply);
    ui.set_stage_open(true);
    let _ = ui.window().take_snapshot()?;
    ui.set_scroll_y(-ui.get_scroll_limit());
    let pixels = ui.window().take_snapshot()?;
    image::save_buffer(
        output.join("host-stage-reference.png"),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )?;
    let size = ui.window().size();
    ui.window().set_size(slint::LogicalSize::new(380., 600.));
    ui.set_detail_version("zh-CN@16.19.3".into());
    crate::stage_bridge::present(ui, &reply);
    if !ui.get_stage_status().contains("与本地数据补丁不同") {
        return Err("stage-patch-context-missing".into());
    }
    let _ = ui.window().take_snapshot()?;
    ui.set_scroll_y(-ui.get_scroll_limit());
    let pixels = ui.window().take_snapshot()?;
    image::save_buffer(
        output.join("host-stage-reference-narrow.png"),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )?;
    ui.window().set_size(size);
    ui.invoke_stage_reset();
    if ui.get_stage_rows().row_count() != 0 || ui.get_stage_open() || ui.get_stage_busy() {
        return Err("stage-reset".into());
    }
    Ok(())
}
