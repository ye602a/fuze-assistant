//! Passive game clicks followed by a definite visual close. Never supplies a claim stage.
use fuze_contract::{selection_pixels::Evidence, wire::TitleKind, Slot};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    #[default]
    None,
    IncompleteCards,
    StaleOffer,
    WindowUnavailable,
    Interrupted,
    CloseTimeout,
    AmbiguousFrame,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostics {
    pub owned: u16,
    pub clicks: u16,
    pub cancelled: u16,
    pub reason: Reason,
    pub observer_failed: bool,
}

const FRESH_MS: u64 = 900;
const CLOSE_MS: u64 = 2000;
pub type Owned = Arc<Vec<(u32, String)>>;

#[derive(Clone, PartialEq)]
struct Offer {
    cards: [Option<(u32, String)>; 3],
    screen: (u32, u32),
    at: u64,
}
struct Pending {
    card: (u32, String),
    slot: usize,
    screen: (u32, u32),
    at: u64,
    absent: Option<u64>,
}
#[derive(Default)]
pub(crate) struct Tracker {
    diagnostics: Diagnostics,
    offer: Option<Offer>,
    down: Option<(usize, u64)>,
    pending: Option<Pending>,
    owned: Owned,
    hwnd: isize,
    invalidated_at: u64,
}
impl Tracker {
    fn lost_input(&mut self, at: u64) {
        if at > self.invalidated_at {
            self.cancel();
            self.invalidated_at = at;
        }
    }
    fn cancel(&mut self) {
        if self.pending.is_some() || self.down.is_some() {
            self.diagnostics.cancelled = self.diagnostics.cancelled.saturating_add(1);
            self.diagnostics.reason = Reason::Interrupted;
        }
        self.offer = None;
        self.down = None;
        self.pending = None;
    }
    fn show(&mut self, slots: &[Slot; 3], kind: TitleKind, screen: (u32, u32), at: u64) {
        // A reroll can be clicked while an older capture is still in the OCR worker.
        if self.invalidated_at != 0 && at <= self.invalidated_at {
            return;
        }
        if kind != TitleKind::Candidate {
            self.cancel();
            return;
        }
        let cards = slots.each_ref().map(|s| match s {
            Slot::Known { id, name, .. } if fuze_contract::skill_choice::get(*id).is_none() => {
                Some((*id, name.clone()))
            }
            _ => None,
        });
        // A missing unrelated title must not invalidate a known clicked slot.
        // Duplicate known IDs are ambiguous and still reject this offer.
        if cards.iter().all(Option::is_none)
            || cards.iter().enumerate().any(|(i, c)| {
                c.as_ref()
                    .is_some_and(|c| cards[..i].iter().flatten().any(|p| p.0 == c.0))
            })
        {
            self.cancel();
            self.diagnostics.reason = Reason::IncompleteCards;
            return;
        }
        // OCR changes in another slot do not change the card the player clicked.
        let changed_pending = self.pending.as_ref().is_some_and(|p| {
            p.absent.is_some()
                || p.screen != screen
                || cards[p.slot].as_ref().map(|c| c.0) != Some(p.card.0)
        });
        let changed_press = self.down.is_some_and(|(slot, _)| {
            self.offer
                .as_ref()
                .is_none_or(|o| o.screen != screen || o.cards[slot] != cards[slot])
        });
        if changed_pending || changed_press {
            self.cancel();
        }
        self.offer = Some(Offer { cards, screen, at });
    }
    fn slot(&self, x: i32, y: i32, at: u64) -> Option<usize> {
        let o = self
            .offer
            .as_ref()
            .filter(|o| at.saturating_sub(o.at) <= FRESH_MS)?;
        // Same primary-screen card columns as OCR. Exclude the reroll controls below the cards.
        let regions = fuze_contract::titles::regions(o.screen.0, o.screen.1).ok()?;
        if y < (o.screen.1 as f64 * 0.19) as i32 || y >= (o.screen.1 as f64 * 0.61) as i32 {
            return None;
        }
        regions
            .iter()
            .enumerate()
            .find(|(i, r)| o.cards[*i].is_some() && x >= r.x as i32 && x < (r.x + r.width) as i32)
            .map(|(i, _)| i)
    }
    fn pointer(&mut self, down: bool, point: Option<(i32, i32)>, at: u64) {
        // After a definite close, ordinary game clicks belong to gameplay. Keep
        // waiting for the second absent frame; foreign input still cancels below.
        if point.is_some() && self.pending.as_ref().is_some_and(|p| p.absent.is_some()) {
            return;
        }
        let stale = self
            .offer
            .as_ref()
            .is_some_and(|o| at.saturating_sub(o.at) > FRESH_MS);
        let slot = point.and_then(|(x, y)| self.slot(x, y, at));
        if down {
            if self.pending.take().is_some() {
                self.diagnostics.cancelled = self.diagnostics.cancelled.saturating_add(1);
                self.diagnostics.reason = Reason::Interrupted;
            }
            self.down = slot.map(|slot| (slot, at));
            // A reroll or hide click invalidates the old offer immediately, before the next OCR.
            if slot.is_none() {
                self.invalidated_at = at;
                self.cancel();
                if stale {
                    self.diagnostics.reason = Reason::StaleOffer;
                }
            }
        } else {
            let previous = self.down.take();
            if let (Some(slot), Some((pressed, since))) = (slot, previous) {
                if pressed == slot && at.saturating_sub(since) <= FRESH_MS {
                    self.diagnostics.clicks = self.diagnostics.clicks.saturating_add(1);
                    self.pending =
                        self.offer
                            .as_ref()
                            .and_then(|o| o.cards[slot].clone())
                            .map(|card| Pending {
                                card,
                                slot,
                                screen: self.offer.as_ref().unwrap().screen,
                                at,
                                absent: None,
                            });
                    return;
                }
            }
            self.invalidated_at = at;
            self.cancel();
        }
    }
    fn expire(&mut self, at: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| at.saturating_sub(p.at) > CLOSE_MS)
        {
            self.cancel();
            self.diagnostics.reason = Reason::CloseTimeout;
        }
    }
    fn capture_at(&mut self, scheduled: Option<u64>, next_capture: u64, at: u64) -> Option<u64> {
        // Also expire on duplicate/timeout frames, which never reach frame().
        self.expire(at);
        scheduled.map(|at| {
            if self.pending.is_some() {
                at.min(next_capture)
            } else {
                at
            }
        })
    }
    fn frame(&mut self, evidence: Option<Evidence>, at: u64) {
        // A capture already in flight when the click arrived cannot confirm its result.
        if self.pending.as_ref().is_some_and(|p| at <= p.at) {
            return;
        }
        self.expire(at);
        match evidence {
            Some(Evidence::Visible) => {
                if self.pending.as_ref().is_some_and(|p| p.absent.is_some()) {
                    self.cancel();
                }
            }
            Some(Evidence::Partial | Evidence::Uncertain)
                if self.pending.as_ref().is_some_and(|p| p.absent.is_none()) =>
            {
                // Fade-out is not confirmation. Keep only the actual click until
                // fresh absent frames arrive; do not accept clicks on stale cards.
                self.offer = None;
                self.down = None;
            }
            Some(Evidence::Absent) => {
                self.offer = None;
                self.down = None;
                if let Some(p) = &mut self.pending {
                    let since = *p.absent.get_or_insert(at);
                    // Two fresh captures spanning the existing screen-close grace, not OCR misses.
                    if at.saturating_sub(since) >= 500 {
                        let p = self.pending.take().unwrap();
                        if !self.owned.iter().any(|c| c.0 == p.card.0) {
                            Arc::make_mut(&mut self.owned).push(p.card);
                            self.diagnostics.reason = Reason::None;
                        }
                    }
                }
            }
            _ => {
                let pending = self.pending.is_some();
                self.cancel();
                if pending {
                    self.diagnostics.reason = Reason::AmbiguousFrame;
                }
            }
        }
    }
}

#[derive(Default)]
pub struct Recorder {
    shared: Arc<Mutex<Tracker>>,
    input_lost: Arc<AtomicU64>,
    root: Option<std::path::PathBuf>,
    target: Option<fuze_capture::target::GameWindow>,
    next_discovery: u64,
    input: Option<native::Observer>,
    pub error: Option<&'static str>,
}
fn now() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}
impl Recorder {
    pub fn configure(&mut self, root: std::path::PathBuf) {
        self.root = Some(root);
    }
    pub fn diagnostics(&self) -> Diagnostics {
        let s = self.shared.lock().unwrap();
        Diagnostics {
            owned: s.owned.len().min(u16::MAX as usize) as u16,
            observer_failed: self.error.is_some(),
            ..s.diagnostics
        }
    }
    pub fn owned(&self) -> Owned {
        self.shared.lock().unwrap().owned.clone()
    }
    pub fn restore(&mut self, owned: Owned) {
        self.shared.lock().unwrap().owned = owned;
    }
    pub fn cancel(&mut self) {
        self.shared.lock().unwrap().cancel();
    }
    /// Finish the observed click even if closing the popup already slowed recognition.
    /// None still means paused/disconnected/faulted; this never starts recognition.
    pub fn capture_at(&self, scheduled: Option<u64>, next_capture: u64) -> Option<u64> {
        self.shared
            .lock()
            .unwrap()
            .capture_at(scheduled, next_capture, now())
    }
    pub fn frame(&mut self, evidence: Option<Evidence>, captured: u64) {
        let visible = self
            .target
            .as_ref()
            .and_then(|w| w.primary_client())
            .is_some();
        let mut s = self.shared.lock().unwrap();
        s.lost_input(self.input_lost.load(Ordering::Acquire));
        if visible {
            s.frame(evidence, captured);
        } else {
            s.cancel();
        }
    }
    pub fn offer(&mut self, slots: &[Slot; 3], kind: TitleKind, captured: u64, screen: (u32, u32)) {
        let at = now();
        if self.root.is_none() || at.saturating_sub(captured) > FRESH_MS {
            self.cancel();
            self.shared.lock().unwrap().diagnostics.reason = Reason::StaleOffer;
            return;
        }
        {
            let mut s = self.shared.lock().unwrap();
            s.show(slots, kind, screen, captured);
            if s.offer.is_none() {
                return;
            }
        }
        if self.target.as_ref().is_some_and(|w| !w.valid()) {
            self.target = None;
        }
        if self.target.is_none() && at >= self.next_discovery {
            self.next_discovery = at + 5000;
            self.target = self
                .root
                .as_ref()
                .and_then(|r| fuze_capture::target::discover(r).ok().flatten());
        }
        let Some((hwnd, width, height)) = self.target.as_ref().and_then(|w| w.primary_client())
        else {
            self.cancel();
            self.shared.lock().unwrap().diagnostics.reason = Reason::WindowUnavailable;
            return;
        };
        if screen != (width, height) {
            self.cancel();
            return;
        }
        if self.input.is_none() && self.error.is_none() {
            match native::Observer::start(self.shared.clone(), self.input_lost.clone()) {
                Ok(input) => self.input = Some(input),
                Err(error) => self.error = Some(error),
            }
        }
        if self.input.is_some() {
            let mut s = self.shared.lock().unwrap();
            s.hwnd = hwnd;
        }
    }
    pub(crate) fn timestamp() -> u64 {
        now()
    }
}

mod native;

#[cfg(test)]
mod tests {
    use super::*;
    fn cards(ids: [u32; 3]) -> [Slot; 3] {
        ids.map(|id| Slot::Known {
            id,
            name: format!("符文{id}"),
            confidence: 1.,
        })
    }
    fn offer() -> Tracker {
        let mut s = Tracker::default();
        s.show(
            &cards([1004, 2018, 3060]),
            TitleKind::Candidate,
            (1920, 1080),
            100,
        );
        s
    }
    fn click(s: &mut Tracker, slot: usize, at: u64) {
        let r = fuze_contract::titles::regions(1920, 1080).unwrap()[slot];
        let p = Some(((r.x + r.width / 2) as i32, 400));
        s.pointer(true, p, at);
        s.pointer(false, p, at + 20);
    }
    fn close(s: &mut Tracker, at: u64) {
        s.frame(Some(Evidence::Absent), at);
        s.frame(Some(Evidence::Absent), at + 500);
    }
    #[test]
    fn candidates_and_disappearance_alone_never_become_owned() {
        let mut s = offer();
        assert!(s.owned.is_empty());
        close(&mut s, 300);
        assert!(s.owned.is_empty());
        s = offer();
        click(&mut s, 1, 200);
        s.frame(Some(Evidence::Visible), 500);
        assert!(s.owned.is_empty());
        s.frame(Some(Evidence::Visible), 2500);
        assert_eq!(s.diagnostics.reason, Reason::CloseTimeout);
        assert_eq!(s.diagnostics.cancelled, 1);
        close(&mut s, 2600);
        assert!(s.owned.is_empty());
    }
    #[test]
    fn real_card_click_then_two_absent_frames_records_only_that_id_and_feeds_combos() {
        let mut s = offer();
        click(&mut s, 0, 200);
        s.frame(Some(Evidence::Absent), 300);
        assert!(s.owned.is_empty());
        s.frame(Some(Evidence::Absent), 800);
        assert_eq!(&*s.owned, &[(1004, "符文1004".into())]);
        assert_eq!(s.diagnostics.clicks, 1);
        assert_eq!(s.diagnostics.cancelled, 0);
        assert_eq!(s.diagnostics.reason, Reason::None);
        let previous = s.owned.clone();
        s.show(
            &cards([1004, 2018, 3060]),
            TitleKind::Candidate,
            (1920, 1080),
            1100,
        );
        click(&mut s, 0, 1200);
        close(&mut s, 1300);
        assert!(Arc::ptr_eq(&previous, &s.owned));
        let snapshot = crate::stage_reference::Snapshot {
            schema: 1,
            champion: 145,
            patch: "16.22".into(),
            date: "2026-11-06".into(),
            data_path: String::new(),
            fetched: 100,
            rows: Default::default(),
            combinations: Some(vec![crate::stage_reference::Combination {
                ids: vec![1004, 3086],
                win: 0.6495,
                samples: 6819,
            }]),
        };
        let combo =
            crate::augment_combos::reference(Some(&snapshot), 145, "16.22", &s.owned, 3086, 101);
        assert_eq!(combo.partners, "符文1004");
        assert_eq!(combo.statistic, "两符文搭配胜率\n65.0% · 6819 场");
    }
    #[test]
    fn reroll_invalidates_inflight_ocr_and_does_not_record_replacement_in_old_slot() {
        let mut s = offer();
        s.pointer(true, Some((1000, 700)), 200); // below card: reroll, not choice
        s.pointer(false, Some((1000, 700)), 220);
        s.show(
            &cards([1004, 2018, 3060]),
            TitleKind::Candidate,
            (1920, 1080),
            180,
        );
        click(&mut s, 0, 250);
        close(&mut s, 300);
        assert!(s.owned.is_empty());
        s.show(
            &cards([9999, 2018, 3060]),
            TitleKind::Candidate,
            (1920, 1080),
            900,
        );
        click(&mut s, 0, 1000);
        close(&mut s, 1100);
        assert_eq!(s.owned[0].0, 9999);
    }
    #[test]
    fn black_frame_shards_skills_and_unreadable_clicked_card_never_confirm_a_pick() {
        for evidence in [None, Some(Evidence::TitlesOnly)] {
            let mut s = offer();
            click(&mut s, 1, 200);
            s.frame(evidence, 300);
            close(&mut s, 500);
            assert!(s.owned.is_empty());
        }
        for kind in [
            TitleKind::Unknown,
            TitleKind::StatShard,
            TitleKind::SkillChoice,
        ] {
            let mut s = offer();
            s.show(&cards([1004, 2018, 3060]), kind, (1920, 1080), 200);
            click(&mut s, 0, 300);
            close(&mut s, 400);
            assert!(s.owned.is_empty());
        }
        let mut s = offer();
        let mut partial = cards([1004, 2018, 3060]);
        partial[1] = Slot::Unreadable;
        s.show(&partial, TitleKind::Candidate, (1920, 1080), 200);
        click(&mut s, 1, 300);
        close(&mut s, 400);
        assert!(s.owned.is_empty());
    }
    #[test]
    fn stale_drag_outside_and_untrusted_window_clicks_are_ignored() {
        let mut s = offer();
        click(&mut s, 0, 1100);
        close(&mut s, 1200);
        assert!(s.owned.is_empty());
        s = offer();
        click(&mut s, 0, 200);
        s.lost_input(250);
        close(&mut s, 300);
        assert!(s.owned.is_empty());
        s = offer();
        s.pointer(true, Some((550, 400)), 200);
        s.pointer(false, Some((950, 400)), 220);
        close(&mut s, 300);
        assert!(s.owned.is_empty());
        s = offer();
        click(&mut s, 0, 200);
        s.pointer(true, None, 250); // FUZE/another app/injected input
        close(&mut s, 300);
        assert!(s.owned.is_empty());
    }
    #[test]
    fn changed_cards_or_reopened_offer_cancel_pending_without_touching_owned() {
        for changed in [false, true] {
            let mut s = offer();
            click(&mut s, 1, 200);
            if !changed {
                s.frame(Some(Evidence::Absent), 300);
            }
            s.show(
                &cards(if changed {
                    [1004, 9999, 3060]
                } else {
                    [1004, 2018, 3060]
                }),
                TitleKind::Candidate,
                (1920, 1080),
                400,
            );
            close(&mut s, 600);
            assert!(s.owned.is_empty());
        }
    }
    #[test]
    fn hit_regions_follow_ocr_columns_and_exclude_reroll_buttons_at_each_resolution() {
        for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440)] {
            let mut s = Tracker::default();
            s.show(&cards([1, 2, 3]), TitleKind::Candidate, (w, h), 100);
            for (i, r) in fuze_contract::titles::regions(w, h)
                .unwrap()
                .iter()
                .enumerate()
            {
                assert_eq!(
                    s.slot((r.x + r.width / 2) as i32, (h as f64 * 0.4) as i32, 200),
                    Some(i)
                );
            }
            for b in fuze_contract::selection_pixels::buttons(w, h).unwrap() {
                assert_eq!(
                    s.slot((b.x + b.width / 2) as i32, (b.y + b.height / 2) as i32, 200),
                    None
                );
            }
        }
    }
}

#[cfg(test)]
mod gameplay_regressions {
    use super::*;
    fn cards() -> [Slot; 3] {
        [1004, 2018, 3060].map(|id| Slot::Known {
            id,
            name: format!("rune{id}"),
            confidence: 1.0,
        })
    }
    fn click(s: &mut Tracker, at: u64) {
        let p = Some((550, 400));
        s.pointer(true, p, at);
        s.pointer(false, p, at + 20);
    }
    #[test]
    fn closed_level_gate_allows_only_the_pending_close_confirmation_then_sleeps() {
        use fuze_runtime::{
            screen_gate::ScreenGate,
            session::{FrameKey, Phase, Session},
        };
        let mut session = Session::new("zh-CN@fixture".into()).unwrap();
        session.update_phase(Phase::InProgress).unwrap();
        session.update_level(3, 0);
        let ticket = session
            .begin(
                FrameKey {
                    epoch: 1,
                    sequence: 1,
                },
                0,
            )
            .unwrap();
        assert!(session
            .finish(
                ticket,
                cards(),
                fuze_runtime::gate::Selection::Candidate,
                100
            )
            .is_some());
        session.screen_visible(100);
        let mut gate = ScreenGate::default();
        gate.observe(Evidence::Visible, 0);
        gate.observe(Evidence::Visible, 100);
        let mut s = Tracker::default();
        s.show(&cards(), TitleKind::Candidate, (1920, 1080), 100);
        click(&mut s, 200);
        s.frame(Some(Evidence::Uncertain), 600);
        gate.observe(Evidence::Uncertain, 600);
        s.frame(Some(Evidence::Absent), 1100);
        let decision = gate.observe(Evidence::Absent, 1100);
        assert!(decision.closed);
        session.screen_probe(false, decision.closed, 1100);
        assert!(session.wake_at(1100).unwrap() > 2220);
        assert_eq!(s.capture_at(session.wake_at(1100), 1600, 1100), Some(1600));
        assert_eq!(s.capture_at(None, 1600, 1100), None);
        assert!(session
            .begin(
                FrameKey {
                    epoch: 1,
                    sequence: 2
                },
                1600
            )
            .is_none());
        s.frame(Some(Evidence::Absent), 1600);
        assert_eq!(s.owned[0].0, 1004);
        session.update_level(6, 1600);
        assert_eq!(
            s.capture_at(session.wake_at(1600), 2100, 1600),
            session.wake_at(1600)
        );
        assert_eq!(
            session.level_schedule(1600).0.mode,
            fuze_runtime::gate::Mode::Saving
        );
    }
    #[test]
    fn missing_capture_cannot_keep_confirmation_polling_after_timeout() {
        let mut s = Tracker::default();
        s.show(&cards(), TitleKind::Candidate, (1920, 1080), 100);
        click(&mut s, 200);
        assert_eq!(s.capture_at(Some(9000), 800, 300), Some(800));
        // No usable capture ever arrives; the scheduling boundary must expire the click.
        assert_eq!(s.capture_at(Some(9000), 2800, 2500), Some(9000));
        assert_eq!(s.diagnostics.reason, Reason::CloseTimeout);
        assert!(s.owned.is_empty());
    }
    #[test]
    fn unrelated_ocr_change_during_press_and_after_click_keeps_clicked_identity() {
        let mut s = Tracker::default();
        let mut shown = cards();
        s.show(&shown, TitleKind::Candidate, (1920, 1080), 100);
        s.pointer(true, Some((550, 400)), 200);
        shown[2] = Slot::Unreadable;
        s.show(&shown, TitleKind::Candidate, (1920, 1080), 210);
        s.pointer(false, Some((550, 400)), 220);
        shown[1] = Slot::Unreadable;
        s.show(&shown, TitleKind::Candidate, (1920, 1080), 250);
        s.frame(Some(Evidence::Absent), 300);
        s.frame(Some(Evidence::Absent), 800);
        assert_eq!(s.owned.iter().map(|c| c.0).collect::<Vec<_>>(), [1004]);
        assert_eq!(s.diagnostics.clicks, 1);
        assert_eq!(s.diagnostics.cancelled, 0);
    }
    #[test]
    fn fade_only_waits_for_two_fresh_absent_frames_and_does_not_extend_deadline() {
        for late in [false, true] {
            let mut s = Tracker::default();
            s.show(&cards(), TitleKind::Candidate, (1920, 1080), 100);
            click(&mut s, 200);
            s.frame(Some(Evidence::Partial), 300);
            s.frame(Some(Evidence::Uncertain), 600);
            assert!(s.owned.is_empty());
            assert_eq!(s.slot(550, 400, 650), None);
            let first = if late { 2000 } else { 800 };
            s.frame(Some(Evidence::Absent), first);
            assert!(s.owned.is_empty());
            s.frame(Some(Evidence::Absent), first + 500);
            assert_eq!(s.owned.len(), usize::from(!late));
            if late {
                assert_eq!(s.diagnostics.reason, Reason::CloseTimeout);
            }
        }
    }
    #[test]
    fn pre_click_capture_cannot_start_close_confirmation() {
        let mut s = Tracker::default();
        s.show(&cards(), TitleKind::Candidate, (1920, 1080), 100);
        click(&mut s, 200);
        // Capture started before mouse-up; its delivery must not count as a post-click close.
        s.frame(Some(Evidence::Absent), 210);
        s.frame(Some(Evidence::Absent), 750);
        assert!(s.owned.is_empty());
        s.frame(Some(Evidence::Absent), 1250);
        assert_eq!(s.owned[0].0, 1004);
    }
    #[test]
    fn fade_does_not_survive_black_frame_foreign_click_or_changed_target() {
        for kind in 0..4 {
            let mut s = Tracker::default();
            s.show(&cards(), TitleKind::Candidate, (1920, 1080), 100);
            click(&mut s, 200);
            s.frame(Some(Evidence::Partial), 300);
            match kind {
                0 => s.frame(None, 400),
                1 => s.pointer(true, None, 400),
                2 => {
                    let mut next = cards();
                    next[0] = Slot::Unreadable;
                    s.show(&next, TitleKind::Candidate, (1920, 1080), 400);
                }
                _ => s.show(&cards(), TitleKind::Candidate, (1280, 720), 400),
            }
            s.frame(Some(Evidence::Absent), 500);
            s.frame(Some(Evidence::Absent), 1000);
            assert!(s.owned.is_empty());
        }
    }
    #[test]
    fn known_clicked_card_survives_unreadable_other_card() {
        let mut s = Tracker::default();
        let mut offer = cards();
        offer[2] = Slot::Unreadable;
        s.show(&offer, TitleKind::Candidate, (1920, 1080), 100);
        click(&mut s, 200);
        s.frame(Some(Evidence::Absent), 300);
        s.frame(Some(Evidence::Absent), 800);
        assert_eq!(s.owned[0].0, 1004);
        assert_eq!(s.diagnostics.clicks, 1);
        assert_eq!(s.diagnostics.reason, Reason::None);
    }
    #[test]
    fn post_close_foreign_input_reopen_and_ambiguity_still_cancel() {
        for kind in 0..4 {
            let mut s = Tracker::default();
            s.show(&cards(), TitleKind::Candidate, (1920, 1080), 100);
            click(&mut s, 200);
            s.frame(Some(Evidence::Absent), 300);
            match kind {
                0 => s.pointer(true, None, 400),
                1 => {
                    s.frame(Some(Evidence::Visible), 400);
                    s.show(&cards(), TitleKind::Candidate, (1920, 1080), 400);
                }
                2 => s.frame(None, 400),
                _ => s.frame(Some(Evidence::Uncertain), 400),
            }
            s.frame(Some(Evidence::Absent), 500);
            s.frame(Some(Evidence::Absent), 1000);
            assert!(s.owned.is_empty());
        }
    }
    #[test]
    fn game_click_after_first_absent_frame_preserves_pending_selection() {
        let mut s = Tracker::default();
        s.show(&cards(), TitleKind::Candidate, (1920, 1080), 100);
        click(&mut s, 200);
        s.frame(Some(Evidence::Absent), 300);
        // Another genuine left click after the choice closes, before confirmation completes.
        s.pointer(true, Some((1000, 800)), 400);
        s.pointer(false, Some((1000, 800)), 420);
        s.frame(Some(Evidence::Absent), 800);
        assert_eq!(s.owned[0].0, 1004);
        assert_eq!(s.diagnostics.clicks, 1);
        assert_eq!(s.diagnostics.cancelled, 0);
    }
}
