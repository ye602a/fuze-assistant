//! Device-local OCR observations, never claims, wins, accounts or online match history.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub champion: u32,
    pub observations: Vec<[Option<u32>; 3]>,
    pub capped: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    segments: Vec<Segment>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub champion: u32,
    pub count: usize,
    pub last: [Option<u32>; 3],
    pub capped: bool,
}
#[derive(Default)]
pub struct Journal {
    saved: Saved,
    active: Option<Segment>,
    path: Option<PathBuf>,
    pub error: Option<&'static str>,
}
impl Journal {
    pub fn open(path: PathBuf) -> Self {
        let read = || -> Result<Saved> {
            if !path.exists() {
                return Ok(Saved::default());
            }
            let data: Saved =
                serde_json::from_slice(&crate::update_channel::bounded(&path, 1024 * 1024)?)
                    .map_err(|_| "records-invalid")?;
            if data.segments.len() > 100
                || data.segments.iter().any(|s| {
                    s.champion == 0
                        || s.observations.len() > 80
                        || s.observations
                            .iter()
                            .any(|o| o.iter().flatten().any(|id| *id == 0))
                })
            {
                return Err("records-invalid");
            }
            Ok(data)
        };
        match read() {
            Ok(saved) => Self {
                saved,
                path: Some(path),
                active: None,
                error: None,
            },
            // Preserve unreadable data rather than overwriting it on the next match.
            Err(error) => Self {
                error: Some(error),
                saved: Saved::default(),
                active: None,
                path: None,
            },
        }
    }
    pub fn observe(&mut self, champion: u32, slots: &[fuze_contract::Slot; 3]) {
        if self.path.is_none() || champion == 0 {
            return;
        }
        let cards = std::array::from_fn(|i| match slots[i] {
            fuze_contract::Slot::Known { id, .. } if id > 0 => Some(id),
            _ => None,
        });
        if cards.iter().all(Option::is_none) {
            return;
        }
        if self.active.as_ref().is_some_and(|s| s.champion != champion) {
            self.finish();
        }
        let segment = self.active.get_or_insert_with(|| Segment {
            champion,
            observations: Vec::new(),
            capped: false,
        });
        if segment.observations.last() == Some(&cards) {
            return;
        }
        if segment.observations.len() == 80 {
            segment.capped = true;
            return;
        }
        segment.observations.push(cards);
    }
    pub fn recent(&self) -> Vec<Row> {
        self.active
            .iter()
            .chain(self.saved.segments.iter().rev())
            .take(8)
            .map(|s| Row {
                champion: s.champion,
                count: s.observations.len(),
                last: s.observations.last().copied().unwrap_or([None; 3]),
                capped: s.capped,
            })
            .collect()
    }
    pub fn finish(&mut self) {
        let Some(segment) = self.active.take() else {
            return;
        };
        self.saved.segments.push(segment);
        if self.saved.segments.len() > 100 {
            self.saved.segments.remove(0);
        }
        if let Some(path) = &self.path {
            if crate::preferences::save_json(path, &self.saved).is_err() {
                self.error = Some("records-write");
            }
        }
    }
}
impl Drop for Journal {
    fn drop(&mut self) {
        self.finish();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_positions_dedup_caps_and_privacy() {
        let root = std::env::temp_dir().join(format!("fuze-local-records-{}", std::process::id()));
        let path = root.join("records.json");
        let mut j = Journal::open(path.clone());
        let known = |id| fuze_contract::Slot::Known {
            id,
            name: "fixture-not-persisted".into(),
            confidence: 0.9,
        };
        for _ in 0..20 {
            j.observe(1, &[known(2), fuze_contract::Slot::Unreadable, known(3)]);
        }
        assert_eq!(j.recent()[0].last, [Some(2), None, Some(3)]);
        assert_eq!(j.recent()[0].count, 1);
        for id in 4..100 {
            j.observe(1, &[known(id), known(2), known(3)]);
        }
        assert!(j.recent()[0].capped);
        assert_eq!(j.recent()[0].count, 80);
        j.finish();
        drop(j);
        let bytes = std::fs::read_to_string(&path).unwrap();
        for forbidden in [
            "fixture",
            "name",
            "token",
            "puuid",
            "win",
            "stage",
            "selection",
        ] {
            assert!(!bytes.contains(forbidden));
        }
        let j = Journal::open(path.clone());
        assert_eq!(j.recent().len(), 1);
        drop(j);
        std::fs::write(&path, b"invalid").unwrap();
        let mut j = Journal::open(path.clone());
        j.observe(1, &[known(1), known(2), known(3)]);
        j.finish();
        assert!(j.error.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        drop(j);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
