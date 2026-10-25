//! Existing client-data champion shard schema. No network, per-frame IO or stage inference.
use crate::{Prepared, Result};
use serde_json::Value;
use std::collections::HashMap;
#[derive(Clone, Debug)]
pub struct Recommendation {
    pub win_rate: Option<f64>,
    pub pick_rate: Option<f64>,
    pub score: Option<f64>,
    pub origin: String,
    pub region: String,
    pub patch: String,
}
fn number(v: &Value, a: &str, b: &str) -> Option<f64> {
    v.get(a)
        .or_else(|| v.get(b))
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|v| v.is_finite())
}
fn text(v: &Value, name: &str) -> String {
    v.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 80 && !s.chars().any(char::is_control))
        .unwrap_or("unknown")
        .into()
}
impl Prepared {
    pub fn recommendations(&self, champion: u32) -> Result<HashMap<u32, Recommendation>> {
        if champion == 0 {
            return Err("data-champion-missing");
        }
        let key = champion.to_string();
        let mut result = HashMap::new();
        let mut found = false;
        let mut ordering = Vec::new();
        let mut official = std::collections::HashSet::new();
        for (path, document) in &self.documents {
            if !path.starts_with("champion-shards/") || path.ends_with("/index.json") {
                continue;
            }
            let Some(value) = document.get("champions").and_then(|v| v.get(&key)) else {
                continue;
            };
            if found {
                return Err("data-duplicate-champion");
            };
            found = true;
            let entries = value
                .get("augments")
                .and_then(Value::as_array)
                .ok_or("data-champion-augments")?;
            if entries.len() > 10000 {
                return Err("data-augment-count");
            }
            for entry in entries {
                let id = entry
                    .get("id")
                    .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
                    .filter(|id| *id > 0 && *id <= u32::MAX as u64)
                    .ok_or("data-augment-id")? as u32;
                let s = entry.get("stats").unwrap_or(entry);
                let rate = |a, b| number(s, a, b).filter(|v| (0.0..=1.0).contains(v));
                let win_rate = rate("winRate", "win_rate");
                let pick_rate = rate("pickRate", "pick_rate");
                let rank = number(s, "rank", "rank").filter(|v| *v >= 1. && v.fract() == 0.);
                let total = number(s, "total", "total").filter(|v| *v >= 1. && v.fract() == 0.);
                let score = win_rate.map(|w| {
                    w * 0.6
                        + pick_rate.unwrap_or(0.) * 0.2
                        + (number(s, "games", "num_games").unwrap_or(0.).max(0.) / 1000.).min(1.)
                            * 0.2
                });
                let explicit = s
                    .get("winRateSource")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty());
                if s["rankSource"] == crate::tencent::SOURCE && rank.is_some() {
                    official.insert(id);
                }
                ordering.push((
                    id,
                    rank,
                    total,
                    number(s, "tier", "tier").filter(|v| *v >= 1. && v.fract() == 0.),
                    pick_rate,
                ));
                let row = Recommendation {
                    win_rate,
                    pick_rate,
                    score,
                    origin: text(s, if explicit { "winRateSource" } else { "source" }),
                    region: text(s, if explicit { "winRateRegion" } else { "region" }),
                    patch: text(
                        s,
                        if explicit {
                            "winRateGamePatch"
                        } else {
                            "gamePatch"
                        },
                    ),
                };
                if result.insert(id, row).is_some() {
                    return Err("data-duplicate-augment");
                }
            }
        }
        if !found {
            return Err("data-champion-missing");
        }
        if !official.is_empty() {
            for row in &ordering {
                let total = row.2.unwrap_or(ordering.len() as f64);
                result.get_mut(&row.0).unwrap().score = official.contains(&row.0).then(|| {
                    if total <= 1. {
                        1.
                    } else {
                        ((total - row.1.unwrap()) / (total - 1.)).clamp(0., 1.)
                    }
                });
            }
            return Ok(result);
        }
        if !ordering.is_empty() && ordering.iter().all(|r| r.1.is_some()) {
            let count = ordering.len() as f64;
            for row in &ordering {
                let total = row.2.unwrap_or(count);
                result.get_mut(&row.0).unwrap().score = Some(if total <= 1. {
                    1.
                } else {
                    ((total - row.1.unwrap()) / (total - 1.)).clamp(0., 1.)
                });
            }
        }
        // Match the existing tier-priority path when the complete pool has no authoritative ranks.
        if !ordering.is_empty()
            && !ordering.iter().all(|r| r.1.is_some())
            && ordering.iter().any(|r| r.3.is_some())
        {
            ordering.sort_by(|a, b| {
                a.3.unwrap_or(f64::INFINITY)
                    .total_cmp(&b.3.unwrap_or(f64::INFINITY))
                    .then_with(|| b.4.unwrap_or(-1.).total_cmp(&a.4.unwrap_or(-1.)))
                    .then_with(|| a.0.cmp(&b.0))
            });
            let count = ordering.len();
            for (index, row) in ordering.iter().enumerate() {
                result.get_mut(&row.0).unwrap().score = Some(if count == 1 {
                    1.
                } else {
                    (count - 1 - index) as f64 / (count - 1) as f64
                });
            }
        }
        Ok(result)
    }
}
