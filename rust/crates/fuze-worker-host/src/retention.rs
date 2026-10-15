//! Display-only grace period. Observations and scheduling always use the fresh result.
use fuze_contract::{wire::TitleKind, Slot};
const GRACE_MS: u64 = 2500;
#[derive(Default)]
pub struct Retention {
    previous: Option<[Slot; 3]>,
    until: Option<u64>,
}
pub struct Display {
    pub slots: [Slot; 3],
    pub retained: bool,
    pub visible: bool,
}
impl Retention {
    pub fn deadline(&self) -> Option<u64> {
        self.until
    }
    pub fn expire(&mut self, now: u64) -> bool {
        if self.until.is_some_and(|at| now >= at) {
            *self = Self::default();
            true
        } else {
            false
        }
    }
    pub fn apply(&mut self, slots: &[Slot; 3], kind: TitleKind, now: u64) -> Display {
        self.expire(now);
        let known = slots.iter().any(|s| matches!(s, Slot::Known { .. }));
        if kind == TitleKind::StatShard {
            *self = Self::default();
            return Display {
                slots: std::array::from_fn(|_| Slot::Unreadable),
                retained: false,
                visible: false,
            };
        }
        // Never combine cards from different frames or carry results over a changed known title.
        let subset = self.previous.as_ref().is_some_and(|old| {
            slots.iter().zip(old).all(|(new, old)| match (new, old) {
                (Slot::Unreadable, _) => true,
                (Slot::Known { id: a, .. }, Slot::Known { id: b, .. }) => a == b,
                _ => false,
            })
        }) && slots
            .iter()
            .filter(|s| matches!(s, Slot::Known { .. }))
            .count()
            < self
                .previous
                .as_ref()
                .unwrap()
                .iter()
                .filter(|s| matches!(s, Slot::Known { .. }))
                .count();
        if subset {
            self.until.get_or_insert(now.saturating_add(GRACE_MS));
            return Display {
                slots: self.previous.as_ref().unwrap().clone(),
                retained: true,
                visible: true,
            };
        }
        self.until = None;
        self.previous = known.then(|| slots.clone());
        Display {
            slots: slots.clone(),
            retained: false,
            visible: known,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn cards(ids: [u32; 3]) -> [Slot; 3] {
        ids.map(|id| {
            if id == 0 {
                Slot::Unreadable
            } else {
                Slot::Known {
                    id,
                    name: format!("fixture-{id}"),
                    confidence: 1.0,
                }
            }
        })
    }
    #[test]
    fn animation_retention_expires_without_mixing_or_counting_as_claims() {
        let mut r = Retention::default();
        r.apply(&cards([1, 2, 3]), TitleKind::Candidate, 0);
        assert!(
            r.apply(&cards([1, 0, 3]), TitleKind::Candidate, 100)
                .retained
        );
        assert_eq!(
            r.apply(&cards([0, 0, 0]), TitleKind::Unknown, 200).slots,
            cards([1, 2, 3])
        );
        assert_eq!(r.deadline(), Some(2600));
        assert!(!r.apply(&cards([0, 0, 0]), TitleKind::Unknown, 2600).visible);
        r.apply(&cards([1, 2, 3]), TitleKind::Candidate, 3000);
        let new = r.apply(&cards([1, 9, 0]), TitleKind::Candidate, 3100);
        assert!(!new.retained);
        assert_eq!(new.slots, cards([1, 9, 0]));
        assert!(
            !r.apply(&cards([0, 0, 0]), TitleKind::StatShard, 3200)
                .visible
        );
        assert!(!r.apply(&cards([0, 0, 0]), TitleKind::Unknown, 3300).visible);
        r.apply(&cards([1, 2, 3]), TitleKind::Candidate, 4000);
        r.apply(&cards([0, 0, 0]), TitleKind::Unknown, 4100);
        assert!(r.expire(6600)); // No new capture is required to expire the display.
        assert!(r.previous.is_none());
    }
}
