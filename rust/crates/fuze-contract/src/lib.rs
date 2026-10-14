use serde::{Deserialize, Serialize};
pub mod frame;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "state", deny_unknown_fields)]
pub enum Slot {
    Known {
        id: u32,
        name: String,
        confidence: f32,
    },
    Unreadable,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActualStage {
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u16,
    pub generation: u64,
    pub sequence: u64,
    pub dataset_version: String,
    pub selection_confirmed: bool,
    pub actual_stage: ActualStage,
    pub slots: [Slot; 3],
}

pub struct ContextGate {
    generation: u64,
    dataset_version: String,
    last_sequence: Option<u64>,
}
impl ContextGate {
    pub fn new(generation: u64, dataset_version: String) -> Self {
        Self {
            generation,
            dataset_version,
            last_sequence: None,
        }
    }
    pub fn accept(&mut self, snapshot: &Snapshot) -> bool {
        let valid_slots = snapshot.slots.iter().all(|slot| match slot {
            Slot::Known {
                id,
                name,
                confidence,
            } => {
                *id > 0
                    && !name.is_empty()
                    && name.chars().count() <= 80
                    && confidence.is_finite()
                    && (0.0..=1.0).contains(confidence)
            }
            Slot::Unreadable => true,
        });
        if snapshot.schema_version != 1
            || snapshot.generation != self.generation
            || snapshot.dataset_version != self.dataset_version
            || !valid_slots
            || self
                .last_sequence
                .is_some_and(|last| snapshot.sequence <= last)
        {
            return false;
        }
        self.last_sequence = Some(snapshot.sequence);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot {
            schema_version: 1,
            generation: 4,
            sequence: 2,
            dataset_version: "fixture-v1".into(),
            selection_confirmed: false,
            actual_stage: ActualStage::Unknown,
            slots: [Slot::Unreadable, Slot::Unreadable, Slot::Unreadable],
        }
    }
    #[test]
    fn rejects_stale_duplicate_and_cross_dataset_without_consuming_sequence() {
        let mut gate = ContextGate::new(4, "fixture-v1".into());
        let mut value = snapshot();
        value.generation = 3;
        assert!(!gate.accept(&value));
        value.generation = 4;
        value.dataset_version = "old".into();
        assert!(!gate.accept(&value));
        value.dataset_version = "fixture-v1".into();
        assert!(gate.accept(&value));
        assert!(!gate.accept(&value));
        value.sequence = 1;
        assert!(!gate.accept(&value));
        value.sequence = 3;
        assert!(gate.accept(&value));
    }
    #[test]
    fn rejects_invalid_confidence_without_shifting_slots() {
        let mut value = snapshot();
        value.slots[1] = Slot::Known {
            id: 1,
            name: "fixture".into(),
            confidence: f32::NAN,
        };
        let mut gate = ContextGate::new(4, "fixture-v1".into());
        assert!(!gate.accept(&value));
        assert_eq!(value.slots[0], Slot::Unreadable);
        assert_eq!(value.slots[2], Slot::Unreadable);
    }
}
