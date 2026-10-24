//! Selection UI lifetime, independent of reward ownership/rounds and OCR success.
use fuze_contract::selection_pixels::Evidence;
#[derive(Default)]
pub struct ScreenGate {
    positive: u8,
    open: bool,
    missing_since: Option<u64>,
    titles: [Option<u32>; 3],
    skill_only: bool,
}
pub struct Decision {
    pub recognize: bool,
    pub closed: bool,
}
impl ScreenGate {
    /// A lone shop/tooltip title cannot open a recommendation. Partial OCR may
    /// only retain already admitted cards in the same slots; a reroll needs two.
    pub fn accept_titles(
        &mut self,
        slots: &[fuze_contract::Slot; 3],
        kind: fuze_contract::wire::TitleKind,
    ) -> bool {
        use fuze_contract::{wire::TitleKind, Slot};
        if (self.skill_only && kind != TitleKind::SkillChoice) || kind == TitleKind::Unknown {
            return false;
        }
        if kind == TitleKind::StatShard {
            self.titles = [None; 3];
            return true;
        }
        let ids = slots.each_ref().map(|s| match s {
            Slot::Known { id, .. } => Some(*id),
            _ => None,
        });
        let unique: std::collections::HashSet<_> = ids.iter().flatten().collect();
        if unique.len() >= 2 {
            self.titles = ids;
            return true;
        }
        ids.iter()
            .zip(self.titles)
            .all(|(new, old)| new.is_none() || *new == old)
    }
    pub fn observe(&mut self, evidence: Evidence, now: u64) -> Decision {
        match evidence {
            Evidence::Visible | Evidence::TitlesOnly => {
                let skill_only = evidence == Evidence::TitlesOnly;
                if self.skill_only != skill_only {
                    self.positive = 0;
                    self.open = false;
                }
                self.skill_only = skill_only;
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
                    self.titles = [None; 3];
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
    fn no_button_probe_cannot_admit_shop_or_ordinary_titles() {
        use fuze_contract::{skill_choice, wire::TitleKind, Slot};
        let mut g = ScreenGate::default();
        assert!(!g.observe(Evidence::TitlesOnly, 0).recognize);
        assert!(g.observe(Evidence::TitlesOnly, 250).recognize);
        let ordinary = [1, 2, 3].map(|id| Slot::Known {
            id,
            name: "ordinary".into(),
            confidence: 1.,
        });
        assert!(!g.accept_titles(&ordinary, TitleKind::Candidate));
        let mut skills = ["重型", "力量", "鞭挞"].map(|n| skill_choice::read(n, 1.).unwrap());
        assert!(!g.accept_titles(
            &[skills[0].clone(), Slot::Unreadable, Slot::Unreadable],
            TitleKind::SkillChoice
        ));
        assert!(!g.accept_titles(
            &[skills[0].clone(), skills[0].clone(), Slot::Unreadable],
            TitleKind::SkillChoice
        ));
        assert!(g.accept_titles(&skills, TitleKind::SkillChoice));
        skills[1] = Slot::Unreadable;
        assert!(g.accept_titles(&skills, TitleKind::SkillChoice));
        assert!(!g.observe(Evidence::Visible, 500).recognize);
        assert!(g.observe(Evidence::Visible, 750).recognize);
        assert!(g.accept_titles(&ordinary, TitleKind::Candidate));
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
            &[slot(1), Slot::Unreadable, Slot::Unreadable],
            TitleKind::Candidate
        ));
        assert!(!g.accept_titles(&[slot(1), slot(1), Slot::Unreadable], TitleKind::Candidate));
        assert!(g.accept_titles(&[slot(1), slot(2), slot(3)], TitleKind::Candidate));
        assert!(g.accept_titles(
            &[slot(1), Slot::Unreadable, Slot::Unreadable],
            TitleKind::Candidate
        ));
        assert!(!g.accept_titles(
            &[slot(9), Slot::Unreadable, Slot::Unreadable],
            TitleKind::Candidate
        ));
        assert!(g.accept_titles(&[slot(9), slot(8), Slot::Unreadable], TitleKind::Candidate));
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
