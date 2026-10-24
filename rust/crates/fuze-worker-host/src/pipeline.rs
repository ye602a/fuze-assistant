//! Session-scoped control-thread host shared by offline GPU input and real owned-window WGC.
//! Game attachment is explicit; no automatic startup. UI has its own message thread.
use crate::{recovery::Supervisor, Result};
use fuze_capture::native::{CaptureFeed, FrameStatus, OwnedWindowStream};
use fuze_contract::{
    titles::Atlas,
    wire::{Identity, Reply, Request, TitleKind},
    Snapshot,
};
use fuze_data::Prepared;
use fuze_native_overlay::native::HiddenOverlay;
use fuze_runtime::{
    gate::Selection,
    session::{FrameKey, Phase, Session, Ticket},
};
use std::time::{Duration, Instant};
pub struct Pipeline {
    pub(crate) diagnostics: crate::recognition_log::Probe,
    diagnostic_level: Option<u8>,
    diagnostic_ocr: Option<serde_json::Value>,
    diagnostic_active: bool,
    pub title_reads: [fuze_contract::wire::TitleRead; 3],
    pub frames: crate::overlay_frame::Publisher,
    tiers: std::collections::HashMap<u32, u64>,
    pub(crate) art: Option<crate::overlay_art::Art>,
    retention: crate::retention::Retention,
    screen_gate: fuze_runtime::screen_gate::ScreenGate,
    pub capture_state: &'static str,
    pub last_capture: Option<Instant>,
    session: Session,
    pub worker: Supervisor,
    pub overlay: HiddenOverlay,
    capture: Option<Box<dyn CaptureFeed>>,
    dataset: String,
    epoch: Instant,
    closed: bool,
    capture_failure: Option<serde_json::Value>,
    next_capture_at: u64,
    pub skipped_frames: u64,
    pub busy_frames: u64,
    busy_streak: u8,
}
pub struct Delivery {
    pub title_kind: TitleKind,
    pub display_visible: bool,
    pub retained: bool,
    pub snapshot: Snapshot,
    pub roi_bytes: usize,
    pub ocr_ms: f64,
    pub total_ms: f64,
}
fn finish_session(
    session: &mut Session,
    ticket: Ticket,
    slots: [fuze_contract::Slot; 3],
    selection: Selection,
    now: u64,
) -> Result<Option<Snapshot>> {
    let snapshot = session.finish(ticket, slots, selection, now);
    // A completed but expired reply has no transport error. Signal exhaustion to Controller
    // so its bounded backoff replaces the worker instead of leaving a timerless session.
    if snapshot.is_none() && session.faulted() {
        Err("worker-result-expired")
    } else {
        Ok(snapshot)
    }
}
impl Pipeline {
    pub fn scheduling(&self) -> (fuze_runtime::gate::Mode, u64, Option<u8>) {
        let (d, next) = self.session.level_schedule(self.now());
        (d.mode, d.interval_ms, next)
    }
    pub fn new(worker: Supervisor, data: &Prepared, champion: u32) -> Result<Self> {
        Self::new_inner(worker, data, champion, false, false)
    }
    pub fn managed(worker: Supervisor, data: &Prepared, champion: u32) -> Result<Self> {
        Self::new_inner(worker, data, champion, true, false)
    }
    pub fn windowless(worker: Supervisor, data: &Prepared, champion: u32) -> Result<Self> {
        Self::new_inner(worker, data, champion, false, true)
    }
    fn new_inner(
        worker: Supervisor,
        data: &Prepared,
        champion: u32,
        persist: bool,
        headless: bool,
    ) -> Result<Self> {
        let dataset = data.identity();
        let recognition_only = data.scope() == fuze_data::Scope::Ocr;
        let detail = if recognition_only {
            fuze_data::champion_detail::Detail {
                augments: data.recognition_cards(),
                ..Default::default()
            }
        } else {
            data.champion_detail(champion)?
        };
        let tiers = detail
            .augments
            .iter()
            .filter_map(|a| a.tier.filter(|t| *t > 0).map(|t| (a.id, t)))
            .collect();
        let mut session = Session::new(dataset.clone())?;
        session.update_phase(Phase::Loading)?;
        let mut stats = if recognition_only {
            Default::default()
        } else {
            data.recommendations(champion)?
        };
        stats.retain(|id, _| fuze_contract::skill_choice::get(*id).is_none());
        let frames = crate::overlay_frame::Publisher::with_catalog(
            detail.augments,
            data.recognition_cards(),
            stats
                .iter()
                .filter_map(|(id, r)| r.score.map(|s| (*id, s)))
                .collect(),
        );
        let overlay = if headless {
            HiddenOverlay::headless(session.generation(), dataset.clone())
        } else if persist {
            HiddenOverlay::with_placement(
                session.generation(),
                dataset.clone(),
                stats,
                Box::new(crate::window_memory::Memory::new("augments", false)),
            )?
        } else {
            HiddenOverlay::with_statistics(session.generation(), dataset.clone(), stats)?
        };
        Ok(Self {
            diagnostics: Default::default(),
            diagnostic_level: None,
            diagnostic_ocr: None,
            diagnostic_active: false,
            title_reads: Default::default(),
            frames,
            tiers,
            art: None,
            retention: Default::default(),
            screen_gate: Default::default(),
            capture_state: "waiting",
            last_capture: None,
            session,
            worker,
            overlay,
            capture: None,
            dataset,
            epoch: Instant::now(),
            closed: false,
            capture_failure: None,
            next_capture_at: 0,
            skipped_frames: 0,
            busy_frames: 0,
            busy_streak: 0,
        })
    }
    pub fn cancellation(&self) -> fuze_native_overlay::native::Cancellation {
        self.overlay.cancellation()
    }
    fn cancel_if_requested(&mut self) -> Result<bool> {
        if !self.overlay.cancellation().cancelled() {
            return Ok(false);
        }
        self.capture = None;
        self.worker.stop()?;
        self.closed = true;
        self.frames.clear();
        Ok(true)
    }
    pub fn dataset(&self) -> &str {
        &self.dataset
    }
    pub fn tier(&self, id: u32) -> Option<u64> {
        self.tiers.get(&id).copied()
    }
    pub fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
    pub fn wake_at(&self) -> Option<u64> {
        if self.closed {
            None
        } else {
            self.session.wake_at(self.now()).map(|at| {
                let at = at.max(self.next_capture_at);
                self.retention
                    .deadline()
                    .map_or(at, |deadline| at.min(deadline))
            })
        }
    }
    pub fn clear_level(&mut self) {
        self.diagnostic_level = None;
        self.session.clear_level();
    }
    pub(crate) fn closed_window(&self) -> Option<fuze_runtime::gate::ClosedWindow> {
        self.session.closed_window()
    }
    pub(crate) fn restore_closed_window(&mut self, state: fuze_runtime::gate::ClosedWindow) {
        self.session.restore_closed_window(state);
    }
    pub fn level_gate(&mut self, enabled: bool) {
        self.session.level_gate(enabled);
    }
    pub fn level_sample(&mut self, level: u8, age: u64) {
        self.diagnostic_level = (age <= 10_000).then_some(level);
        self.session
            .update_level(level, self.now().saturating_sub(age));
    }
    pub fn level(&mut self, level: u8) {
        self.diagnostic_level = Some(level);
        self.session.update_level(level, self.now());
    }
    pub fn life_sample(&mut self, dead: Option<bool>) {
        self.session.update_dead(dead, self.now());
    }
    pub fn phase(&mut self, phase: Phase) -> Result<()> {
        if let Some(cleanup) = self.session.update_phase(phase)? {
            self.retention = Default::default();
            self.screen_gate = Default::default();
            self.frames.clear();
            if cleanup.stop_capture {
                self.capture = None;
            }
            if !self
                .overlay
                .reset(cleanup.generation, self.dataset.clone())?
            {
                return Err("overlay-reset");
            }
            if matches!(phase, Phase::Disconnected | Phase::PostGame) {
                self.worker.stop()?;
                self.closed = true;
                self.frames.clear();
            }
        }
        Ok(())
    }
    pub fn capture_failure(&self) -> Option<&serde_json::Value> {
        self.capture_failure.as_ref()
    }
    pub fn has_capture(&self) -> bool {
        self.capture.is_some()
    }
    pub fn capture_source(&self) -> &'static str {
        self.capture.as_ref().map_or("waiting", |c| c.source())
    }
    pub fn capture_fallback(&self) -> Option<&'static str> {
        self.capture.as_ref().and_then(|c| c.fallback())
    }
    pub fn capture_screen(&mut self) -> Result<()> {
        self.capture_feed(Box::new(fuze_capture::screen::ScreenCapture::default()))
    }
    /// Shared installation path for production screen capture and isolated transport fixtures.
    pub fn capture_feed(&mut self, feed: Box<dyn CaptureFeed>) -> Result<()> {
        if self.closed || self.overlay.cancellation().cancelled() || self.wake_at().is_none() {
            return Err("capture-phase");
        }
        self.capture = None;
        self.retention = Default::default();
        self.screen_gate = Default::default();
        self.frames.clear();
        let cleanup = self.session.capture_replaced()?;
        if !self
            .overlay
            .reset(cleanup.generation, self.dataset.clone())?
        {
            return Err("overlay-reset");
        }
        self.capture_failure = None;
        self.capture = Some(feed);
        self.capture_state = "waiting-frame";
        self.next_capture_at = 0;
        Ok(())
    }
    pub fn capture_game(&mut self, target: fuze_capture::target::GameWindow) -> Result<()> {
        if self.closed || self.overlay.cancellation().cancelled() || self.wake_at().is_none() {
            return Err("capture-phase");
        }
        self.capture = None;
        self.retention = Default::default();
        self.screen_gate = Default::default();
        self.frames.clear();
        let cleanup = self.session.capture_replaced()?;
        if !self
            .overlay
            .reset(cleanup.generation, self.dataset.clone())?
        {
            return Err("overlay-reset");
        }
        self.capture_failure = None;
        match OwnedWindowStream::start_game(target) {
            Ok(stream) => {
                self.capture = Some(Box::new(stream));
                self.capture_state = "waiting-frame";
            }
            Err(error) => {
                self.capture_failure = Some(fuze_capture::native::failure_details(&*error));
                return Err("capture-start");
            }
        }
        Ok(())
    }
    pub fn capture_owned(&mut self, hwnd: isize, visible_ci: bool) -> Result<()> {
        if self.closed || self.wake_at().is_none() {
            return Err("capture-phase");
        }
        self.capture = None;
        self.retention = Default::default();
        self.screen_gate = Default::default();
        self.frames.clear();
        let cleanup = self.session.capture_replaced()?;
        if !self
            .overlay
            .reset(cleanup.generation, self.dataset.clone())?
        {
            return Err("overlay-reset");
        }
        self.capture_failure = None;
        match OwnedWindowStream::start(hwnd, visible_ci) {
            Ok(stream) => {
                self.capture = Some(Box::new(stream));
                self.capture_state = "waiting-frame";
            }
            Err(error) => {
                self.capture_failure = Some(fuze_capture::native::failure_details(&*error));
                return Err("capture-start");
            }
        }
        Ok(())
    }
    pub fn recognize(&mut self, key: FrameKey, atlas: &Atlas<'_>) -> Result<Option<Delivery>> {
        if self.closed || self.cancel_if_requested()? {
            return Ok(None);
        }
        let Some(ticket) = self.session.begin(key, self.now()) else {
            return Ok(None);
        };
        let started = Instant::now();
        let request = Request::from_atlas(
            Identity {
                generation: ticket.generation,
                sequence: ticket.sequence,
            },
            atlas,
        );
        let roi_bytes = request.pixels.len();
        if self.diagnostic_active {
            self.diagnostic_ocr = Some(serde_json::json!({"attempted":true}));
        }
        let reply = match self.worker.run(request, Duration::from_secs(3)) {
            Ok(reply) => reply,
            Err(error) => {
                self.session.fail(ticket, self.now());
                return Err(error);
            }
        };
        if self.cancel_if_requested()? {
            self.session.fail(ticket, self.now());
            return Ok(None);
        }
        let Reply::Result {
            mut slots,
            mut title_kind,
            title_reads,
            ocr_ms,
            ..
        } = reply
        else {
            self.session.fail(ticket, self.now());
            return Err("worker-result");
        };
        let accepted = self.screen_gate.accept_titles(&slots, title_kind);
        if self.diagnostic_active {
            self.diagnostic_ocr = Some(serde_json::json!({
                "attempted": true, "slots": slots, "titleKind": title_kind, "titleReads": title_reads,
                "accepted": accepted, "ocrMs": ocr_ms,
            }));
        }
        if !accepted {
            slots = std::array::from_fn(|_| fuze_contract::Slot::Unreadable);
            title_kind = TitleKind::Unknown;
        }
        let selection = match title_kind {
            TitleKind::StatShard => Selection::StatShard,
            TitleKind::SkillChoice => Selection::SkillChoice,
            TitleKind::Candidate => Selection::Candidate,
            TitleKind::Unknown => Selection::Unknown,
        };
        let now = self.now();
        let Some(snapshot) = finish_session(&mut self.session, ticket, slots, selection, now)?
        else {
            return Ok(None);
        };
        self.title_reads = title_reads;
        let display = self
            .retention
            .apply(&snapshot.slots, title_kind, self.now());
        let mut shown = snapshot.clone();
        self.capture_state = if title_kind == TitleKind::StatShard {
            "stat-shards"
        } else if display.retained {
            "retaining"
        } else {
            "receiving"
        };
        shown.slots = display.slots;
        self.frames.publish(
            shown.clone(),
            display.visible,
            display.retained,
            self.retention
                .deadline()
                .map(|ms| self.epoch + Duration::from_millis(ms)),
        );
        if let Some(art) = self.art.as_mut() {
            art.request(&shown.slots);
            art.poll(&self.overlay);
        }
        if !self.overlay.update_retained(
            shown,
            self.retention
                .deadline()
                .map(|ms| self.epoch + Duration::from_millis(ms)),
        )? {
            return Err("overlay-delivery");
        }
        Ok(Some(Delivery {
            title_kind,
            display_visible: display.visible,
            retained: display.retained,
            snapshot,
            roi_bytes,
            ocr_ms,
            total_ms: started.elapsed().as_secs_f64() * 1000.,
        }))
    }
    /// At most one capture/readback/inference. Caller sleeps until wake_at or control event.
    pub fn tick_wgc(&mut self) -> Result<Option<Delivery>> {
        if self.cancel_if_requested()? {
            return Ok(None);
        }
        let saving = self.scheduling().0 == fuze_runtime::gate::Mode::Saving;
        if let Some(capture) = self.capture.as_mut() {
            capture.saving(saving);
        }
        let recognition_due =
            self.now() >= self.next_capture_at && self.wake_at().is_some_and(|at| self.now() >= at);
        let diagnostic_due = self.diagnostics.begin();
        if !recognition_due && !diagnostic_due {
            self.expire_retained()?;
            self.skipped_frames = self.skipped_frames.saturating_add(1);
            return Ok(None);
        }
        // Separate GDI probes cannot drain the production DXGI stream, enter a gate or publish.
        if !recognition_due {
            if let Some(capture) = self.capture.as_ref() {
                let metadata = self.diagnostic_metadata("level-or-schedule-gate", &**capture);
                self.diagnostics.gated(metadata);
            }
            self.expire_retained()?;
            return Ok(None);
        }
        let Some(mut capture) = self.capture.take() else {
            return Err("capture-missing");
        };
        self.diagnostic_active = diagnostic_due;
        self.diagnostic_ocr = None;
        let mut diagnostic_sample = None;
        let result = (|| match capture.next(Duration::from_millis(100)).map_err(|e| {
            if e.to_string() == "capture-bitmap" {
                "capture-bitmap"
            } else {
                "capture-frame"
            }
        })? {
            FrameStatus::Ready => {
                if diagnostic_due && self.diagnostics.visible() {
                    diagnostic_sample = Some(crate::recognition_log::Sample::frame(
                        &*capture,
                        self.diagnostic_metadata("frame-ready", &*capture),
                    ));
                }
                self.last_capture = Some(Instant::now());
                self.capture_state = "receiving";
                self.busy_streak = 0;
                if capture.source().starts_with("screen-") {
                    self.next_capture_at = self.now().saturating_add(500);
                }
                let key = FrameKey {
                    epoch: capture.key().0,
                    sequence: capture.key().1,
                };
                let evidence = capture.selection();
                if let Some(evidence) = evidence {
                    let now = self.now();
                    let decision = self.screen_gate.observe(evidence, now);
                    if !decision.recognize {
                        if let Some(sample) = &mut diagnostic_sample {
                            sample.metadata["stage"] = serde_json::json!("pixel-gate");
                        }
                        self.session.screen_probe(
                            evidence == fuze_contract::selection_pixels::Evidence::Visible,
                            // Occlusion may hide cards without ending this level's window.
                            decision.closed
                                && evidence == fuze_contract::selection_pixels::Evidence::Absent,
                            now,
                        );
                        if decision.closed {
                            self.retention = Default::default();
                            self.screen_gate = Default::default();
                            self.frames.end_selection();
                            self.overlay.visible(false)?;
                        }
                        self.capture_state = "waiting-selection";
                        return Ok(None);
                    }
                }
                let atlas = capture.titles().map_err(|_| "capture-atlas")?;
                if let Some(sample) = &mut diagnostic_sample {
                    sample.metadata["stage"] = serde_json::json!("ocr");
                }
                let delivery = self.recognize(key, &atlas)?;
                if evidence == Some(fuze_contract::selection_pixels::Evidence::Visible)
                    || delivery
                        .as_ref()
                        .is_some_and(|d| d.title_kind == TitleKind::SkillChoice)
                {
                    self.session.screen_visible(self.now());
                }
                Ok(delivery)
            }
            FrameStatus::Resized => {
                self.capture_state = "resizing";
                self.retention = Default::default();
                self.screen_gate = Default::default();
                self.frames.clear();
                let cleanup = self.session.capture_replaced()?;
                if !self
                    .overlay
                    .reset(cleanup.generation, self.dataset.clone())?
                {
                    return Err("overlay-reset");
                }
                Ok(None)
            }
            FrameStatus::Busy => {
                self.capture_state = "gpu-busy";
                self.busy_frames = self.busy_frames.saturating_add(1);
                self.busy_streak = self.busy_streak.saturating_add(1);
                self.next_capture_at = self.now().saturating_add(100);
                if self.busy_streak >= 5 {
                    self.busy_streak = 0;
                    return Err("capture-frame");
                }
                Ok(None)
            }
            FrameStatus::Suspended => {
                self.capture_state = "suspended";
                self.retention = Default::default();
                self.screen_gate = Default::default();
                self.frames.clear();
                self.overlay.visible(false)?;
                self.next_capture_at = self.now().saturating_add(500);
                Ok(None)
            }
            FrameStatus::Timeout | FrameStatus::Duplicate => {
                self.capture_state = "waiting-frame";
                self.next_capture_at = self.now().saturating_add(500);
                Ok(None)
            }
        })();
        if diagnostic_due && self.diagnostics.visible() {
            let mut sample = diagnostic_sample.unwrap_or_else(|| {
                crate::recognition_log::Sample::without_image(
                    self.diagnostic_metadata("capture-no-frame", &*capture),
                )
            });
            sample.metadata["ocr"] = serde_json::json!(self.diagnostic_ocr.take());
            sample.metadata["error"] = serde_json::json!(result.as_ref().err());
            if let Ok(Some(delivery)) = &result {
                sample.metadata["displayVisible"] = serde_json::json!(delivery.display_visible);
                sample.metadata["retained"] = serde_json::json!(delivery.retained);
                sample.metadata["totalMs"] = serde_json::json!(delivery.total_ms);
            }
            self.diagnostics.control.record(sample);
        }
        self.diagnostic_active = false;
        // Returned inference failures retain the capture stream; a capture failure stops it.
        if self.closed {
            return result;
        }
        let discard = matches!(result, Err("capture-frame" | "capture-atlas"));
        if discard || matches!(result, Err("capture-bitmap")) {
            self.capture_state = "recovering";
            self.retention = Default::default();
            self.screen_gate = Default::default();
            self.frames.clear();
            let cleanup = self.session.capture_replaced()?;
            self.overlay
                .reset(cleanup.generation, self.dataset.clone())?;
        }
        if !discard {
            self.capture = Some(capture);
        }
        // Let a fresh frame replace the previous display before expiring stale retention.
        // A timeout still expires normally, without waiting for another captured frame.
        self.expire_retained()?;
        result
    }
    fn expire_retained(&mut self) -> Result<()> {
        if self.retention.expire(self.now()) {
            self.frames.clear();
            self.overlay.visible(false)?;
            self.capture_state = "waiting-frame";
        }
        Ok(())
    }
    fn diagnostic_metadata(&self, stage: &str, capture: &dyn CaptureFeed) -> serde_json::Value {
        serde_json::json!({
            "stage": stage, "frame": [capture.key().0, capture.key().1],
            "source": capture.source(), "fallback": capture.fallback(),
            "dataset": self.dataset, "level": self.diagnostic_level,
            "schedule": format!("{:?}", self.scheduling().0), "nextLevel": self.scheduling().2,
            "evidence": capture.selection().map(|e| format!("{e:?}")),
        })
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.capture.take();
        let _ = self.worker.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_completed_jobs_report_exhaustion_instead_of_silently_stopping() {
        let mut session = Session::new("zh-CN@fixture".into()).unwrap();
        session.update_phase(Phase::InProgress).unwrap();
        for i in 0..3 {
            let at = i * 10_000;
            let ticket = session
                .begin(
                    FrameKey {
                        epoch: 1,
                        sequence: i + 1,
                    },
                    at,
                )
                .unwrap();
            let result = finish_session(
                &mut session,
                ticket,
                std::array::from_fn(|_| fuze_contract::Slot::Unreadable),
                Selection::Unknown,
                at + 3000,
            );
            if i < 2 {
                assert!(result.unwrap().is_none());
            } else {
                assert_eq!(result.unwrap_err(), "worker-result-expired");
            }
        }
        assert!(session.faulted());
        assert!(session.wake_at(100_000).is_none());
    }
}
