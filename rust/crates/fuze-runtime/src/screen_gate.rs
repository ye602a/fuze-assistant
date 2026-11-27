//! Selection UI lifetime, independent of reward ownership/rounds and OCR success.
use fuze_contract::{selection_pixels::Evidence, CardSlots, Layout};
#[derive(Default)]
pub struct ScreenGate {
    positive: u8,
    open: bool,
    missing_since: Option<u64>,
    titles: CardSlots<Option<u32>>,
    skill_only: bool,
}
pub struct Decision {
    pub recognize: bool,
    pub closed: bool,
}
impl ScreenGate {
    pub fn active_layout(&self) -> Layout {
        if self.open {
            self.titles.layout()
        } else {
            Layout::Three
        }
    }
    pub fn set_layout(&mut self, layout: Layout) -> bool {
        if self.titles.layout() == layout {
            return false;
        }
        *self = Self {
            titles: CardSlots::from_fn(layout, |_| None),
            ..Self::default()
        };
        true
    }
    /// A lone shop/tooltip title cannot open a recommendation. Partial OCR may
    /// only retain already admitted cards in the same slots; a reroll needs two.
    pub fn accept_titles(
        &mut self,
        slots: &CardSlots<fuze_contract::Slot>,
        kind: fuze_contract::wire::TitleKind,
    ) -> bool {
        use fuze_contract::{wire::TitleKind, Slot};
        if (self.skill_only && kind != TitleKind::SkillChoice) || kind == TitleKind::Unknown {
            return false;
        }
        if kind == TitleKind::StatShard {
            self.titles.fill(None);
            return true;
        }
        let ids = slots.each_ref().map(|slot| match slot {
            Slot::Known { id, .. } => Some(*id),
            _ => None,
        });
        let mut known = ids.iter().flatten();
        if known
            .next()
            .is_some_and(|first| known.any(|id| id != first))
        {
            self.titles = ids;
            return true;
        }
        ids.layout() == self.titles.layout()
            && ids
                .iter()
                .zip(self.titles.iter().copied())
                .all(|(new, old)| new.is_none() || *new == old)
    }
    pub fn observe(&mut self, evidence: Evidence, now: u64) -> Decision {
        match evidence {
            Evidence::Visible | Evidence::TitlesOnly => {
                // Button changes/rerolls are still the same open selection screen.
                // Keep its lifetime so a following disappearance can close it.
                self.skill_only = evidence == Evidence::TitlesOnly;
                self.positive = self.positive.saturating_add(1);
                self.missing_since = None;
                self.open |= self.positive >= 2;
                Decision {
                    recognize: self.open,
                    closed: false,
                }
            }
            Evidence::Partial if self.open => {
                self.missing_since = None;
                Decision {
                    recognize: true,
                    closed: false,
                }
            }
            Evidence::Absent | Evidence::Uncertain | Evidence::Partial => {
                self.positive = 0;
                let since = *self.missing_since.get_or_insert(now);
                // Definite disappearance closes quickly. Occlusion/reroll gets the
                // same bounded 2.5 s grace as the displayed OCR cards.
                let grace = if evidence == Evidence::Absent {
                    500
                } else {
                    2500
                };
                let closed = self.open && now.saturating_sub(since) >= grace;
                if closed {
                    self.open = false;
                    self.titles.fill(None);
                }
                Decision {
                    recognize: false,
                    closed,
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn four_layout_only_stays_active_until_the_shared_screen_gate_closes() {
        let mut gate = ScreenGate::default();
        assert!(gate.set_layout(Layout::Four));
        assert!(!gate.observe(Evidence::Visible, 0).recognize);
        assert_eq!(gate.active_layout(), Layout::Three);
        assert!(gate.observe(Evidence::Visible, 500).recognize);
        assert!(gate.observe(Evidence::Partial, 750).recognize);
        assert_eq!(gate.active_layout(), Layout::Four);
        let missing = gate.observe(Evidence::Uncertain, 1_000);
        assert!(!missing.recognize && !missing.closed);
        assert!(gate.observe(Evidence::Uncertain, 3_500).closed);
        assert_eq!(gate.active_layout(), Layout::Three);
        assert!(gate.set_layout(Layout::Three));
        assert!(!gate.observe(Evidence::Visible, 4_000).recognize);
        assert!(gate.observe(Evidence::Visible, 4_500).recognize);
    }

    #[test]
    fn no_button_probe_cannot_admit_shop_or_ordinary_titles() {
        use fuze_contract::{skill_choice, wire::TitleKind, Slot};
        let mut g = ScreenGate::default();
        assert!(!g.observe(Evidence::TitlesOnly, 0).recognize);
        assert!(g.observe(Evidence::TitlesOnly, 250).recognize);
        let ordinary = CardSlots::Three([1, 2, 3].map(|id| Slot::Known {
            id,
            name: "ordinary".into(),
            confidence: 1.,
        }));
        assert!(!g.accept_titles(&ordinary, TitleKind::Candidate));
        let mut skills =
            CardSlots::Three(["重型", "力量", "鞭挞"].map(|n| skill_choice::read(n, 1.).unwrap()));
        assert!(!g.accept_titles(
            &[skills[0].clone(), Slot::Unreadable, Slot::Unreadable].into(),
            TitleKind::SkillChoice
        ));
        assert!(!g.accept_titles(
            &[skills[0].clone(), skills[0].clone(), Slot::Unreadable].into(),
            TitleKind::SkillChoice
        ));
        assert!(g.accept_titles(&skills, TitleKind::SkillChoice));
        skills[1] = Slot::Unreadable;
        assert!(g.accept_titles(&skills, TitleKind::SkillChoice));
        assert!(g.observe(Evidence::Visible, 500).recognize);
        assert!(g.observe(Evidence::Visible, 750).recognize);
        assert!(g.accept_titles(&ordinary, TitleKind::Candidate));
    }
    #[test]
    fn changing_button_evidence_preserves_closure_and_same_level_reopen() {
        use crate::gate::{LevelGate, Mode, Selection};
        for (first, next) in [
            (Evidence::Visible, Evidence::TitlesOnly),
            (Evidence::TitlesOnly, Evidence::Visible),
        ] {
            let mut screen = ScreenGate::default();
            let mut level = LevelGate::default();
            level.update_level(3, 0);
            assert!(!screen.observe(first, 0).recognize);
            assert!(screen.observe(first, 500).recognize);
            level.observe(
                if first == Evidence::Visible {
                    Selection::Candidate
                } else {
                    Selection::SkillChoice
                },
                500,
            );
            // A reroll/animation changes button pixels just before the panel closes.
            assert!(screen.observe(next, 1000).recognize);
            if next == Evidence::Visible {
                level.observe(Selection::Candidate, 1000);
            }
            assert!(!screen.observe(Evidence::Absent, 1500).closed);
            assert!(screen.observe(Evidence::Absent, 2000).closed);
            level.record_capture(2000);
            level.screen_closed();
            level.update_level(6, 3000);
            assert_eq!(level.decision(3000).mode, Mode::Saving);
            assert_eq!(level.wake_at(3000), 3500);
            assert!(!screen.observe(Evidence::Visible, 3500).recognize);
            level.record_capture(3500);
            level.screen_visible(3500);
            assert_eq!(level.wake_at(3500), 3750);
            assert!(screen.observe(Evidence::Visible, 3750).recognize);
            level.update_level(7, 4000);
            assert!(level.decision(4000).capture);
        }
    }
    #[test]
    fn isolated_shop_title_cannot_open_or_replace_a_confirmed_card() {
        use fuze_contract::{wire::TitleKind, Slot};
        let slot = |id| Slot::Known {
            id,
            name: format!("fixture-{id}"),
            confidence: 1.,
        };
        let mut g = ScreenGate::default();
        assert!(!g.accept_titles(
            &[slot(1), Slot::Unreadable, Slot::Unreadable].into(),
            TitleKind::Candidate
        ));
        assert!(!g.accept_titles(
            &[slot(1), slot(1), Slot::Unreadable].into(),
            TitleKind::Candidate
        ));
        assert!(g.accept_titles(&[slot(1), slot(2), slot(3)].into(), TitleKind::Candidate));
        assert!(g.accept_titles(
            &[slot(1), Slot::Unreadable, Slot::Unreadable].into(),
            TitleKind::Candidate
        ));
        assert!(!g.accept_titles(
            &[slot(9), Slot::Unreadable, Slot::Unreadable].into(),
            TitleKind::Candidate
        ));
        assert!(g.accept_titles(
            &[slot(9), slot(8), Slot::Unreadable].into(),
            TitleKind::Candidate
        ));
    }
    #[test]
    fn first_choice_reroll_occlusion_and_close_are_distinct() {
        let mut g = ScreenGate::default();
        assert!(!g.observe(Evidence::Visible, 0).recognize);
        assert!(!g.observe(Evidence::Absent, 500).closed); // one noisy frame never opens
        assert!(!g.observe(Evidence::Visible, 1000).recognize);
        assert!(g.observe(Evidence::Visible, 1500).recognize);
        assert!(g.observe(Evidence::Partial, 1700).recognize); // occluded slots retain their positions
        assert!(!g.observe(Evidence::Uncertain, 2000).closed);
        assert!(g.observe(Evidence::Visible, 2500).recognize); // do not re-confirm a reroll
        assert!(!g.observe(Evidence::Absent, 3000).closed);
        assert!(g.observe(Evidence::Absent, 3500).closed);
        assert!(!g.observe(Evidence::Visible, 4000).recognize);
        assert!(g.observe(Evidence::Visible, 4500).recognize);
    }
}
