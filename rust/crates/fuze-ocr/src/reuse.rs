//! One exact title atlas, in memory only. No hashing, image encoding or retained game frames.
use fuze_contract::{
    wire::{Request, TitleKind},
    CardSlots, Slot,
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct TitleReuse {
    previous: Option<(Request, CardSlots<Slot>, TitleKind, Instant)>,
}
impl TitleReuse {
    pub fn get(&self, request: &Request) -> Option<(CardSlots<Slot>, TitleKind)> {
        self.get_at(request, Instant::now())
    }
    fn get_at(&self, request: &Request, now: Instant) -> Option<(CardSlots<Slot>, TitleKind)> {
        let (old, slots, kind, until) = self.previous.as_ref()?;
        (now < *until
            && old.id.generation == request.id.generation
            && old.width == request.width
            && old.height == request.height
            && old.layout == request.layout
            && old.pixels == request.pixels)
            .then(|| (slots.clone(), *kind))
    }
    pub fn remember(&mut self, request: Request, slots: CardSlots<Slot>, kind: TitleKind) {
        self.remember_at(request, slots, kind, Instant::now());
    }
    fn remember_at(
        &mut self,
        request: Request,
        slots: CardSlots<Slot>,
        kind: TitleKind,
        now: Instant,
    ) {
        // Match the legacy bounded reuse policy: a static failed/partial read must retry.
        // Hits do not extend the deadline. Keep exact bytes rather than another hash pass.
        let seconds = if kind == TitleKind::StatShard {
            10
        } else if slots.iter().all(|s| matches!(s, Slot::Known { .. })) {
            3
        } else {
            1
        };
        // The wire decoder already bounds size; keep this boundary valid for internal callers too.
        self.previous = request
            .atlas()
            .ok()
            .map(|_| ())
            .map(|_| (request, slots, kind, now + Duration::from_secs(seconds)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuze_contract::wire::Identity;
    fn request(generation: u64, sequence: u64, width: u32, height: u32, value: u8) -> Request {
        Request {
            id: Identity {
                generation,
                sequence,
            },
            width,
            height,
            layout: fuze_contract::Layout::Three,
            pixels: vec![value; width as usize * height as usize * 12],
        }
    }
    #[test]
    fn exact_reuse_preserves_missing_slot_and_rejects_context_geometry_or_pixel_changes() {
        let mut cache = TitleReuse::default();
        let slots = CardSlots::Three([
            Slot::Known {
                id: 1,
                name: "fixture".into(),
                confidence: 1.0,
            },
            Slot::Unreadable,
            Slot::Unreadable,
        ]);
        cache.remember(request(1, 1, 2, 1, 9), slots.clone(), TitleKind::Candidate);
        assert_eq!(
            cache.get(&request(1, 2, 2, 1, 9)),
            Some((slots, TitleKind::Candidate))
        );
        assert!(cache.get(&request(2, 3, 2, 1, 9)).is_none());
        assert!(cache.get(&request(1, 3, 1, 2, 9)).is_none());
        assert!(cache.get(&request(1, 3, 2, 1, 8)).is_none());
        let empty = CardSlots::default();
        cache.remember(request(1, 4, 2, 1, 8), empty, TitleKind::StatShard);
        assert!(cache.get(&request(1, 5, 2, 1, 9)).is_none());
        assert!(cache
            .get(&request(1, 5, 2, 1, 8))
            .unwrap()
            .0
            .iter()
            .all(|s| *s == Slot::Unreadable));
        assert_eq!(
            cache.get(&request(1, 5, 2, 1, 8)).unwrap().1,
            TitleKind::StatShard
        );
        let mut bad = request(1, 6, 2, 1, 8);
        bad.pixels.clear();
        cache.remember(bad, CardSlots::default(), TitleKind::Unknown);
        assert!(cache.get(&request(1, 7, 2, 1, 8)).is_none());
    }
    #[test]
    fn identical_failed_partial_full_and_shard_reads_have_bounded_reuse() {
        let now = Instant::now();
        for (count, kind, seconds) in [
            (0, TitleKind::Unknown, 1),
            (1, TitleKind::Candidate, 1),
            (3, TitleKind::Candidate, 3),
            (0, TitleKind::StatShard, 10),
        ] {
            let mut cache = TitleReuse::default();
            let slots = CardSlots::from_fn(fuze_contract::Layout::Three, |i| {
                if i < count {
                    Slot::Known {
                        id: i as u32 + 1,
                        name: "fixture".into(),
                        confidence: 1.0,
                    }
                } else {
                    Slot::Unreadable
                }
            });
            cache.remember_at(request(1, 1, 2, 1, 9), slots, kind, now);
            let end = now + Duration::from_secs(seconds);
            assert!(cache
                .get_at(&request(1, 2, 2, 1, 9), end - Duration::from_millis(1))
                .is_some());
            assert!(cache.get_at(&request(1, 3, 2, 1, 9), end).is_none());
        }
    }
}
