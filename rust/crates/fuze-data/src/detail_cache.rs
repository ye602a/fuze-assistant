//! One local version per UI worker. No network or file reads on a hero selection.
use crate::{champion_detail::Detail, Locale, Prepared, Result, Root, Scope};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

pub struct Loaded {
    pub id: u32,
    pub detail: Detail,
    pub version: String,
}

pub struct Cache {
    identity: String,
    source: Option<Prepared>,
    pending: VecDeque<u32>,
    details: HashMap<u32, Result<Arc<Loaded>>>,
}

impl Cache {
    /// Called once in the background at startup, and again only for a new data identity.
    pub fn prepare(roots: &[Root], expected: &str) -> Result<Self> {
        let locale = match expected.split('@').next() {
            Some("zh-TW") => Locale::ZhTw,
            Some("en-US") => Locale::EnUs,
            _ => Locale::ZhCn,
        };
        let data = if expected.is_empty() {
            crate::prepare(roots, locale, Scope::Full)?
        } else {
            crate::current_augments::prepare_identity(roots, locale, Scope::Full, expected)?
        };
        let identity = data.identity();
        if !expected.is_empty() && expected != identity {
            return Err("detail-version-mismatch");
        }
        let entries = data
            .document("champion-shards/index.json")
            .and_then(|v| v["shards"].as_array())
            .ok_or("data-shard-index")?;
        let mut ids = BTreeSet::new();
        for entry in entries {
            let path = entry["path"].as_str().ok_or("data-shard-path")?;
            let rows = data
                .document(path)
                .and_then(|v| v["champions"].as_object())
                .ok_or("data-champion-missing")?;
            for id in entry["championIds"].as_array().ok_or("data-shard-index")? {
                let id = id
                    .as_u64()
                    .filter(|n| *n > 0 && *n <= u32::MAX as u64)
                    .ok_or("data-champion-id")? as u32;
                if !ids.insert(id) || ids.len() > 512 || !rows.contains_key(&id.to_string()) {
                    return Err("data-champion-index");
                }
            }
        }
        if ids.is_empty() {
            return Err("data-champion-missing");
        }
        Ok(Self {
            identity,
            source: Some(data),
            pending: ids.into_iter().collect(),
            details: HashMap::new(),
        })
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn warming(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn get(&mut self, id: u32) -> Result<Arc<Loaded>> {
        if let Some(detail) = self.details.get(&id) {
            return detail.clone();
        }
        // An unknown ID cannot grow the cache or cause a disk/network fallback.
        if !self.pending.contains(&id) {
            return Err("detail-champion-missing");
        }
        let result = self
            .source
            .as_ref()
            .ok_or("detail-source-missing")?
            .champion_detail(id)
            .map(|detail| {
                Arc::new(Loaded {
                    id,
                    detail,
                    version: self.identity.clone(),
                })
            });
        self.details.insert(id, result.clone());
        self.pending.retain(|n| *n != id);
        if self.pending.is_empty() {
            self.source = None; // Keep compact display data, release all raw JSON after warm-up.
        }
        result
    }
    /// The caller checks for a click/cancellation between heroes; no extra polling thread.
    pub fn warm_next(&mut self) {
        if let Some(id) = self.pending.front().copied() {
            let _ = self.get(id);
        }
    }
}
