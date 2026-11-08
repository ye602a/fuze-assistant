//! Passive game clicks followed by a definite visual close. Never supplies a claim stage.
use fuze_contract::{selection_pixels::Evidence, wire::TitleKind, Slot};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

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
    at: u64,
    absent: Option<u64>,
}
#[derive(Default)]
pub(crate) struct Tracker {
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
        // Incomplete readings remain useful for recommendations, but cannot identify a click.
        if cards.iter().any(Option::is_none)
            || cards.iter().enumerate().any(|(i, c)| {
                cards[..i]
                    .iter()
                    .any(|p| p.as_ref().map(|a| a.0) == c.as_ref().map(|a| a.0))
            })
        {
            self.cancel();
            return;
        }
        if self
            .offer
            .as_ref()
            .is_none_or(|o| o.cards != cards || o.screen != screen)
        {
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
            .position(|r| x >= r.x as i32 && x < (r.x + r.width) as i32)
    }
    fn pointer(&mut self, down: bool, point: Option<(i32, i32)>, at: u64) {
        let slot = point.and_then(|(x, y)| self.slot(x, y, at));
        if down {
            self.pending = None;
            self.down = slot.map(|slot| (slot, at));
            // A reroll or hide click invalidates the old offer immediately, before the next OCR.
            if slot.is_none() {
                self.invalidated_at = at;
                self.cancel();
            }
        } else {
            let previous = self.down.take();
            if let (Some(slot), Some((pressed, since))) = (slot, previous) {
                if pressed == slot && at.saturating_sub(since) <= FRESH_MS {
                    self.pending =
                        self.offer
                            .as_ref()
                            .and_then(|o| o.cards[slot].clone())
                            .map(|card| Pending {
                                card,
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
    fn frame(&mut self, evidence: Option<Evidence>, at: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| at.saturating_sub(p.at) > CLOSE_MS)
        {
            self.cancel();
        }
        match evidence {
            Some(Evidence::Visible) => {
                if let Some(p) = &mut self.pending {
                    p.absent = None;
                }
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
                        }
                    }
                }
            }
            _ => self.cancel(),
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
    pub fn owned(&self) -> Owned {
        self.shared.lock().unwrap().owned.clone()
    }
    pub fn restore(&mut self, owned: Owned) {
        self.shared.lock().unwrap().owned = owned;
    }
    pub fn cancel(&mut self) {
        self.shared.lock().unwrap().cancel();
    }
    pub fn frame(&mut self, evidence: Option<Evidence>) {
        let visible = self
            .target
            .as_ref()
            .and_then(|w| w.primary_client())
            .is_some();
        let mut s = self.shared.lock().unwrap();
        s.lost_input(self.input_lost.load(Ordering::Acquire));
        s.frame(if visible { evidence } else { None }, now());
    }
    pub fn offer(&mut self, slots: &[Slot; 3], kind: TitleKind, captured: u64, screen: (u32, u32)) {
        let at = now();
        if self.root.is_none() || at.saturating_sub(captured) > FRESH_MS {
            self.cancel();
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
        assert_eq!(combo.statistic, "组合 65.0% · 6819 场");
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
    fn occlusion_alt_tab_shards_skills_and_unreadable_cards_never_confirm_a_pick() {
        for evidence in [
            None,
            Some(Evidence::Partial),
            Some(Evidence::Uncertain),
            Some(Evidence::TitlesOnly),
        ] {
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
        click(&mut s, 0, 300);
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
