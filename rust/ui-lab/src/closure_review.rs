//! Synthetic-only checks added to the single hidden host acceptance pass.
use crate::{ClaimState, HostState, RecordState, Workbench};
use fuze_worker_host::{
    desktop::{frontend::Session, Status},
    local_records::{Unbound, UnboundSnapshot},
};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};
pub struct Review {
    ui: Workbench,
    binding: Rc<RefCell<crate::unbound_bridge::Binding>>,
    _diagnostic: Rc<RefCell<crate::diagnostic_bridge::Binding>>,
    feedback: Rc<RefCell<crate::feedback_bridge::Binding>>,
    status: Status,
    started: bool,
}
impl Review {
    pub fn new(data: &Path, session: Rc<Session>) -> Result<Self, Box<dyn std::error::Error>> {
        let ui = Workbench::new()?;
        ui.window().set_size(slint::LogicalSize::new(1120., 860.));
        ui.set_page(1);
        ui.global::<HostState>().set_connected(true);
        ui.global::<HostState>().set_offline(true);
        let binding =
            crate::unbound_bridge::attach(&ui, crate::host_bridge::offline_config(data).roots);
        let diagnostic = crate::diagnostic_bridge::attach(&ui, session.clone());
        let feedback = crate::feedback_bridge::attach(&ui, session, diagnostic.clone(), true);
        let mut status = Status::default();
        status.available = Some("zh-CN@16.20.1".into());
        status.unbound_records = UnboundSnapshot(Arc::new(vec![
            Unbound {
                champion: 4,
                capped: true,
                interrupted: true,
                legacy: vec![],
                events: (0..80)
                    .map(|i| fuze_lcu::history::ObservedCards {
                        observed_at: Some(1709164800000 + i * 1000),
                        cards: [Some(1), None, Some(3)],
                        level: Some(6),
                        partial: true,
                        kind: if i == 0 {
                            "first"
                        } else if i % 2 == 0 {
                            "reappeared"
                        } else {
                            "changed"
                        }
                        .into(),
                    })
                    .collect(),
            },
            Unbound {
                champion: 4,
                capped: false,
                interrupted: true,
                events: vec![],
                legacy: vec![[Some(99999), None, Some(3)]],
            },
        ]));
        Ok(Self {
            ui,
            binding,
            _diagnostic: diagnostic,
            feedback,
            status,
            started: false,
        })
    }
    pub fn poll(&mut self, output: &Path) -> Result<bool, Box<dyn std::error::Error>> {
        let ui = &self.ui;
        self.binding.borrow_mut().update(ui, &self.status, true);
        let state = ui.global::<RecordState>();
        if !self.started {
            state.invoke_unbound_action("open".into(), 1);
            self.started = true;
            return Ok(false);
        }
        if !state.get_unbound_names().starts_with("名称数据：") {
            return Ok(false);
        }
        state.invoke_unbound_action("select".into(), 0);
        let event = state
            .get_unbound_events()
            .row_data(0)
            .ok_or("unbound-event-missing")?;
        if state.get_unbound_pages() != 16
            || state.get_unbound_events().row_count() != 5
            || !event.cards.contains("巨像的勇气")
            || !event.cards.contains("位置 2：暂未识别")
            || !event.hint.contains("不代表已领取")
        {
            return Err("unbound-name-slot-pages".into());
        }
        save(ui, &output.join("closure-unbound-events.png"))?;
        for _ in 0..30 {
            state.invoke_unbound_action("page".into(), 1);
        }
        if state.get_unbound_page() != 15
            || !state
                .get_unbound_events()
                .row_data(4)
                .unwrap()
                .title
                .contains("观察 80")
        {
            return Err("unbound-page-cap".into());
        }
        state.invoke_unbound_action("select".into(), 1);
        let event = state
            .get_unbound_events()
            .row_data(0)
            .ok_or("unbound-legacy-missing")?;
        if !event.title.contains("时间未记录")
            || !event.hint.contains("等级未记录")
            || !event.cards.contains("名称暂缺")
        {
            return Err("unbound-legacy-fabrication".into());
        }
        save(ui, &output.join("closure-unbound-legacy.png"))?;
        self.status.available = None;
        self.binding.borrow_mut().update(ui, &self.status, true);
        let unknown = state
            .get_unbound_events()
            .row_data(0)
            .ok_or("unbound-context-missing")?;
        if !state.get_unbound_names().starts_with("等待名称数据版本")
            || unknown.cards.contains("回归基本功")
            || !unknown.cards.contains("#3（名称暂缺）")
        {
            return Err("unbound-context-old-names".into());
        }
        save(ui, &output.join("closure-unbound-unknown-version.png"))?;
        self.status.available = Some("en-US@missing-fixture".into());
        self.binding.borrow_mut().update(ui, &self.status, true);
        if state
            .get_unbound_events()
            .row_data(0)
            .unwrap()
            .cards
            .contains("回归基本功")
        {
            return Err("unbound-context-language-fallback".into());
        }
        self.status.unbound_records = Default::default();
        self.binding.borrow_mut().update(ui, &self.status, true);
        if state.get_unbound_selected() != -1 || state.get_unbound_events().row_count() != 0 {
            return Err("unbound-stale-detail".into());
        }
        let claim = ui.global::<ClaimState>();
        claim.invoke_action("inspect".into());
        if !claim.get_notice().contains("离线验证") || claim.get_busy() {
            return Err("probe-offline-boundary".into());
        }
        let report = fuze_lcu::stage_probe::summarize(
            [
                Ok(serde_json::json!({"properties":{"augments":{}}})),
                Ok(
                    serde_json::json!({"level":6,"augments":["private-fixture-value"],"name":"private-fixture-name"}),
                ),
                Err("unavailable"),
            ],
            1709164800000,
        );
        let summary = crate::diagnostic_bridge::summary(&report);
        if summary.contains("private-fixture") || !summary.contains("领取轮次仍未知") {
            return Err("probe-privacy".into());
        }
        claim.set_summary(summary.into());
        claim.set_details(slint::ModelRc::new(slint::VecModel::from(
            crate::diagnostic_bridge::details(&report)
                .into_iter()
                .map(Into::into)
                .collect::<Vec<_>>(),
        )));
        let mut recorder = crate::feedback::Recorder::default();
        recorder.observe(&self.status, true);
        let bytes = recorder.encode(fuze_lcu::stage_probe::feedback(&report, None, 0))?;
        if String::from_utf8_lossy(&bytes).contains("private-fixture") {
            return Err("feedback-privacy".into());
        }
        let path = std::path::absolute(output)?.join("safe-feedback.json");
        fuze_worker_host::record_export::save(fuze_worker_host::record_export::Request {
            path: path.clone(),
            bytes: bytes.clone(),
        })?;
        if fuze_worker_host::record_export::save(fuze_worker_host::record_export::Request {
            path: path.clone(),
            bytes: vec![],
        })
        .is_ok()
            || std::fs::read(path)? != bytes
        {
            return Err("feedback-overwrite".into());
        }
        self.feedback.borrow_mut().poll(ui, &self.status, true);
        ui.global::<crate::DiagnosticState>()
            .invoke_action("export".into());
        if !ui.global::<HostState>().get_notice().contains("隔离验证") {
            return Err("feedback-offline-boundary".into());
        }
        ui.global::<HostState>().set_notice("".into());
        ui.set_feedback("".into());
        ui.set_page(2);
        #[cfg(feature = "ui-audit")]
        crate::control_review::hit(ui, "查看最近状态")?;
        #[cfg(not(feature = "ui-audit"))]
        ui.global::<crate::DiagnosticState>().set_open(true);
        save(ui, &output.join("closure-feedback.png"))?;
        #[cfg(feature = "ui-audit")]
        crate::control_review::hit(ui, "查看字段详情")?;
        #[cfg(not(feature = "ui-audit"))]
        claim.set_details_open(true);
        save(ui, &output.join("closure-diagnostics.png"))?;
        claim.invoke_action("clear".into());
        if claim.get_can_mark()
            || !claim.get_comparison().is_empty()
            || claim.get_details().row_count() != 0
        {
            return Err("probe-clear".into());
        }
        std::fs::write(
            output.join("closure-review.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "synthetic_only":true,"unbound_names_and_partial_slots":true,"unbound_80_events_paged":true,
                "legacy_metadata_not_invented":true,"stale_detail_cleared":true,"names_cleared_on_unknown_or_changed_identity":true,"probe_offline_rejected":true,
                "probe_summary_sanitized":true,"feedback_export_sanitized_no_overwrite":true,"feedback_offline_dialog_rejected":true,"native_visible_interaction_verified":false,"production_parity_complete":false
            }))?,
        )?;
        Ok(true)
    }
}
fn save(ui: &Workbench, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let pixels = ui.window().take_snapshot()?;
    image::save_buffer(
        path,
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
