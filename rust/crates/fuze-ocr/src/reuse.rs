//! One exact title atlas, in memory only. No hashing, image encoding or retained game frames.
use fuze_contract::{wire::Request, Slot};

#[derive(Default)]
pub struct TitleReuse {
    previous: Option<(Request, [Slot; 3])>,
}
impl TitleReuse {
    pub fn get(&self, request: &Request) -> Option<[Slot; 3]> {
        let (old, slots) = self.previous.as_ref()?;
        (old.id.generation == request.id.generation
            && old.width == request.width
            && old.height == request.height
            && old.pixels == request.pixels)
            .then(|| slots.clone())
    }
    pub fn remember(&mut self, request: Request, slots: [Slot; 3]) {
        // The wire decoder already bounds size; keep this boundary valid for internal callers too.
        self.previous = request.atlas().ok().map(|_| ()).map(|_| (request, slots));
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
            pixels: vec![value; width as usize * height as usize * 12],
        }
    }
    #[test]
    fn exact_reuse_preserves_missing_slot_and_rejects_context_geometry_or_pixel_changes() {
        let mut cache = TitleReuse::default();
        let slots = [
            Slot::Known {
                id: 1,
                name: "fixture".into(),
                confidence: 1.0,
            },
            Slot::Unreadable,
            Slot::Unreadable,
        ];
        cache.remember(request(1, 1, 2, 1, 9), slots.clone());
        assert_eq!(cache.get(&request(1, 2, 2, 1, 9)), Some(slots));
        assert!(cache.get(&request(2, 3, 2, 1, 9)).is_none());
        assert!(cache.get(&request(1, 3, 1, 2, 9)).is_none());
        assert!(cache.get(&request(1, 3, 2, 1, 8)).is_none());
        let empty = std::array::from_fn(|_| Slot::Unreadable);
        cache.remember(request(1, 4, 2, 1, 8), empty);
        assert!(cache.get(&request(1, 5, 2, 1, 9)).is_none());
        assert!(cache
            .get(&request(1, 5, 2, 1, 8))
            .unwrap()
            .iter()
            .all(|s| *s == Slot::Unreadable));
        let mut bad = request(1, 6, 2, 1, 8);
        bad.pixels.clear();
        cache.remember(bad, std::array::from_fn(|_| Slot::Unreadable));
        assert!(cache.get(&request(1, 7, 2, 1, 8)).is_none());
    }
}
