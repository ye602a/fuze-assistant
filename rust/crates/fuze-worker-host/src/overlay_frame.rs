//! Display-only frame after the existing retention/classification gate. No claim inference.
use fuze_contract::Snapshot;
use fuze_data::champion_detail::Augment;
use std::{collections::HashMap, sync::Arc, time::Instant};
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// Process-local pipeline identity prevents a restarted game accepting an old generation.
    pub context: u64,
    pub snapshot: Snapshot,
    pub retained: bool,
    pub deadline: Option<Instant>,
    pub cards: [Option<Arc<Augment>>; 3],
    pub scores: [Option<f64>; 3],
}
pub struct Publisher {
    context: u64,
    augments: HashMap<u32, Arc<Augment>>,
    scores: HashMap<u32, f64>,
    current: Option<Arc<Frame>>,
}
impl Publisher {
    /// A valid OCR match may have no statistics for this champion. Keep its global
    /// artwork/description while leaving rates and scores absent, rather than losing the card.
    pub fn with_catalog(
        augments: Vec<Augment>,
        mut catalog: Vec<Augment>,
        scores: HashMap<u32, f64>,
    ) -> Self {
        for card in &mut catalog {
            card.win = None;
            card.pick = None;
            card.tier = None;
            card.provenance.clear();
        }
        catalog.extend(augments);
        Self::new(catalog, scores)
    }
    pub fn new(augments: Vec<Augment>, scores: HashMap<u32, f64>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            context: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            augments: augments.into_iter().map(|a| (a.id, Arc::new(a))).collect(),
            scores,
            current: None,
        }
    }
    pub fn clear(&mut self) {
        self.current = None;
    }
    pub fn current(&self) -> Option<Arc<Frame>> {
        self.current.clone()
    }
    pub fn publish(
        &mut self,
        snapshot: Snapshot,
        visible: bool,
        retained: bool,
        deadline: Option<Instant>,
    ) {
        self.current = visible.then(|| {
            let ids = snapshot.slots.each_ref().map(|s| match s {
                fuze_contract::Slot::Known { id, .. } => Some(*id),
                _ => None,
            });
            Arc::new(Frame {
                context: self.context,
                cards: ids.map(|id| id.and_then(|id| self.augments.get(&id).cloned())),
                scores: ids.map(|id| id.and_then(|id| self.scores.get(&id).copied())),
                snapshot,
                retained,
                deadline,
            })
        });
    }
}
