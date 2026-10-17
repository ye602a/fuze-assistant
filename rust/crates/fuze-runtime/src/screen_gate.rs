//! Selection UI lifetime, independent of reward ownership/rounds and OCR success.
use fuze_contract::selection_pixels::Evidence;
#[derive(Default)]
pub struct ScreenGate {
    positive: u8,
    open: bool,
    missing_since: Option<u64>,
}
pub struct Decision {
    pub recognize: bool,
    pub closed: bool,
}
impl ScreenGate {
    pub fn observe(&mut self, evidence: Evidence, now: u64) -> Decision {
        match evidence {
            Evidence::Visible => {
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
