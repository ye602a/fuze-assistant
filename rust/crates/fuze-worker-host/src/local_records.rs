//! Device-local OCR candidates. Verified account bindings use a digest; no identity strings or claims.
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: String,
    pub platform: String,
    pub game: u64,
}
impl Binding {
    pub fn from_owner(owner: &fuze_lcu::history::Owner, game: u64) -> Result<Self> {
        if game == 0 || game > 9_007_199_254_740_991 {
            return Err("records-game");
        }
        let mut material = owner.cache_material();
        let hash = crate::update_io::digest(&material, false);
        for b in &mut material {
            unsafe {
                std::ptr::write_volatile(b, 0);
            }
        }
        Ok(Self {
            key: crate::update_io::hex(&hash?),
            platform: owner.platform().into(),
            game,
        })
    }
    fn valid(&self) -> bool {
        self.key.len() == 64
            && self.key.bytes().all(|b| b.is_ascii_hexdigit())
            && fuze_lcu::history::platform(&serde_json::json!(self.platform)).as_deref()
                == Some(&self.platform)
            && self.game > 0
            && self.game <= 9_007_199_254_740_991
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    #[serde(default)]
    pub recognition: fuze_lcu::history::Recognition,
    #[serde(default = "interrupted_default")]
    pub interrupted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<Binding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<fuze_lcu::history::ObservedCards>,
    pub champion: u32,
    pub observations: Vec<[Option<u32>; 3]>,
    pub capped: bool,
}
fn interrupted_default() -> bool {
    true
}
#[derive(Clone, Default, Serialize, Deserialize)]
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
    recognition: fuze_lcu::history::Recognition,
    binding: Option<Binding>,
    visible: bool,
    level: Option<u8>,
    last_write: Option<std::time::Instant>,
    writer: Option<std::thread::JoinHandle<Result<()>>>,
    saved: Saved,
    active: Option<Segment>,
    path: Option<PathBuf>,
    pub error: Option<&'static str>,
}
impl Journal {
    pub fn verified_end(&mut self) {
        if let Some(s) = self.active.as_mut() {
            s.interrupted = false;
        }
    }
    pub fn bind(&mut self, binding: Option<Binding>) {
        if self.binding != binding {
            self.finish();
            self.binding = binding;
            self.recognition = Default::default();
        }
    }
    pub fn metrics(&mut self, total_ms: f64, ocr_ms: f64, partial: bool) {
        if !total_ms.is_finite() || !ocr_ms.is_finite() || total_ms < 0.0 || ocr_ms < 0.0 {
            return;
        }
        let r = &mut self.recognition;
        r.analyses = r.analyses.saturating_add(1).min(1_000_000);
        r.partial = (r.partial + u64::from(partial)).min(r.analyses);
        r.total_ms = (r.total_ms + total_ms).min(1e12);
        r.ocr_ms = (r.ocr_ms + ocr_ms).min(1e12);
        if let Some(s) = self.active.as_mut() {
            s.recognition = r.clone();
        }
    }
    pub fn visibility(&mut self, visible: bool, level: Option<u8>) {
        if !visible {
            self.visible = false;
        }
        self.level = level;
    }
    pub fn open(path: PathBuf) -> Self {
        let legacy = path.with_file_name("observations.json");
        let source = if !path.exists()
            && path
                .file_name()
                .is_some_and(|n| n == "observations-v2.json")
            && legacy.exists()
        {
            &legacy
        } else {
            &path
        };
        let read = || -> Result<Saved> {
            if !source.exists() {
                return Ok(Saved::default());
            }
            let data: Saved =
                serde_json::from_slice(&crate::update_channel::bounded(source, 2 * 1024 * 1024)?)
                    .map_err(|_| "records-invalid")?;
            if data.segments.len() > 100
                || data.segments.iter().any(|s| {
                    s.champion == 0
                        || s.binding.as_ref().is_some_and(|b| !b.valid())
                        || s.events.len() > 80
                        || !fuze_lcu::history::Observation {
                            recognition: Some(s.recognition.clone()),
                            game_id: 1,
                            interrupted: true,
                            capped: false,
                            events: s.events.clone(),
                        }
                        .valid()
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
                recognition: Default::default(),
                binding: None,
                visible: false,
                level: None,
                last_write: None,
                writer: None,
            },
            // Preserve unreadable data rather than overwriting it on the next match.
            Err(error) => Self {
                error: Some(error),
                saved: Saved::default(),
                active: None,
                path: None,
                recognition: Default::default(),
                binding: None,
                visible: false,
                level: None,
                last_write: None,
                writer: None,
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
            recognition: self.recognition.clone(),
            interrupted: true,
            binding: self.binding.clone(),
            events: Vec::new(),
            champion,
            observations: Vec::new(),
            capped: false,
        });
        if self.visible && segment.observations.last() == Some(&cards) {
            return;
        }
        if segment.observations.len() == 80 {
            segment.capped = true;
            return;
        }
        let kind = if segment.observations.is_empty() {
            "first"
        } else if self.visible {
            "changed"
        } else {
            "reappeared"
        };
        segment.events.push(fuze_lcu::history::ObservedCards {
            observed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|t| t.as_millis() as u64)
                .filter(|t| *t > 0),
            cards,
            level: self.level.map(u32::from),
            partial: cards.iter().any(Option::is_none),
            kind: kind.into(),
        });
        segment.observations.push(cards);
        self.visible = true;
        if self
            .last_write
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(2))
        {
            if self.writer.as_ref().is_some_and(|w| w.is_finished()) {
                if self
                    .writer
                    .take()
                    .unwrap()
                    .join()
                    .unwrap_or(Err("records-writer"))
                    .is_err()
                {
                    self.error = Some("records-write");
                }
            }
            if self.writer.is_none() {
                let mut saved = self.saved.clone();
                saved.segments.push(segment.clone());
                if saved.segments.len() > 100 {
                    saved.segments.remove(0);
                }
                let path = self.path.clone().unwrap();
                self.last_write = Some(std::time::Instant::now());
                self.writer = std::thread::Builder::new()
                    .name("fuze-record-checkpoint".into())
                    .spawn(move || crate::preferences::save_json(&path, &saved))
                    .ok();
            }
        }
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
        if let Some(writer) = self.writer.take() {
            if writer.join().unwrap_or(Err("records-writer")).is_err() {
                self.error = Some("records-write");
            }
        }
        self.visible = false;
        let Some(segment) = self.active.take() else {
            return;
        };
        self.recognition = Default::default();
        self.saved.segments.push(segment);
        if self.saved.segments.len() > 100 {
            self.saved.segments.remove(0);
        }
        if let Some(path) = &self.path {
            self.error = crate::preferences::save_json(path, &self.saved)
                .err()
                .map(|_| "records-write");
        }
    }
}
/// Read only during an explicit history/cache view. Never infer account or match from timestamps.
pub fn bound_observations(
    path: &std::path::Path,
    key: &str,
    platform: &str,
) -> Result<Vec<fuze_lcu::history::Observation>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let saved: Saved =
        serde_json::from_slice(&crate::update_channel::bounded(path, 2 * 1024 * 1024)?)
            .map_err(|_| "records-invalid")?;
    if saved.segments.len() > 100 {
        return Err("records-invalid");
    }
    let mut out = Vec::<fuze_lcu::history::Observation>::new();
    for s in saved.segments {
        let Some(b) = s.binding else {
            continue;
        };
        if !b.valid() {
            return Err("records-invalid");
        }
        if b.key != key || b.platform != platform {
            continue;
        }
        let mut record = fuze_lcu::history::Observation {
            recognition: Some(s.recognition),
            game_id: b.game,
            interrupted: s.interrupted,
            capped: s.capped,
            events: s.events,
        };
        if !record.valid() {
            return Err("records-invalid");
        }
        if let Some(previous) = out.iter_mut().find(|v| v.game_id == record.game_id) {
            if let (Some(a), Some(b)) = (&mut previous.recognition, &record.recognition) {
                a.analyses = (a.analyses + b.analyses).min(1_000_000);
                a.partial = (a.partial + b.partial).min(a.analyses);
                a.total_ms = (a.total_ms + b.total_ms).min(1e12);
                a.ocr_ms = (a.ocr_ms + b.ocr_ms).min(1e12);
            }
            previous.interrupted |= record.interrupted;
            for e in record.events.drain(..) {
                if previous.events.last() != Some(&e) {
                    if previous.events.len() < 80 {
                        previous.events.push(e);
                    } else {
                        previous.capped = true;
                    }
                }
            }
            previous.capped |= record.capped;
        } else {
            out.push(record);
        }
    }
    Ok(out)
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
    fn verified_binding_isolated_reappearance_preserved_and_unknown_never_attached() {
        let root = std::env::temp_dir().join(format!("fuze-bound-records-{}", std::process::id()));
        let path = root.join("observations.json");
        let owner = fuze_lcu::history::Owner::from_values(
            &serde_json::json!({"puuid":"fixture-account-A","platformId":"HN1"}),
            &[],
        )
        .unwrap();
        let binding = Binding::from_owner(&owner, 123).unwrap();
        let slots = [
            fuze_contract::Slot::Known {
                id: 7,
                name: "never-persist-name".into(),
                confidence: 1.0,
            },
            fuze_contract::Slot::Unreadable,
            fuze_contract::Slot::Unreadable,
        ];
        let mut journal = Journal::open(path.clone());
        journal.bind(Some(binding.clone()));
        journal.observe(1, &slots);
        journal.observe(1, &slots);
        journal.visibility(false, Some(11));
        journal.observe(1, &slots);
        journal.bind(None);
        journal.observe(1, &slots);
        journal.finish();
        drop(journal);
        let rows = bound_observations(&path, &binding.key, "HN1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].game_id, 123);
        assert_eq!(rows[0].events.len(), 2);
        assert_eq!(rows[0].events[1].kind, "reappeared");
        assert!(rows[0].interrupted);
        assert!(bound_observations(&path, &binding.key, "HN2")
            .unwrap()
            .is_empty());
        assert!(bound_observations(&path, &"0".repeat(64), "HN1")
            .unwrap()
            .is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("fixture-account-A") && !text.contains("never-persist-name"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
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
