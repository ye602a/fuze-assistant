//! Existing client-data champion shard schema. No network, per-frame IO or stage inference.
use crate::{Prepared, Result};
use serde_json::Value;
use std::collections::HashMap;
#[derive(Clone, Debug)]
pub struct Recommendation {
    pub evidence: Evidence,
    pub win_rate: Option<f64>,
    pub pick_rate: Option<f64>,
    pub score: Option<f64>,
    pub origin: String,
    pub region: String,
    pub patch: String,
}
/// Display evidence only. It never changes ordering or borrows ranking sample counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Evidence {
    pub games: Option<u64>,
    pub wins: Option<u64>,
    pub rank: Option<u64>,
    pub official: bool,
    pub win_date: String,
}
impl Evidence {
    pub fn from_stats(s: &Value) -> Self {
        let games = s["games"].as_u64().filter(|n| *n > 0);
        let explicit = s["winRateSource"].as_str().is_some_and(|s| !s.is_empty());
        let date = text(s, if explicit { "winRateDate" } else { "date" });
        Self {
            games,
            wins: s["wins"]
                .as_u64()
                .filter(|w| games.is_some_and(|n| *w <= n)),
            rank: s["rank"].as_u64().filter(|n| *n > 0),
            official: s["rankSource"] == crate::tencent::SOURCE,
            win_date: if date == "unknown" {
                String::new()
            } else {
                date
            },
        }
    }
    pub fn sample_label(&self, has_win: bool) -> String {
        if !has_win {
            return "暂无胜率统计".into();
        }
        self.games
            .map(|n| format!("胜率样本 · {n} 场"))
            .unwrap_or_else(|| "胜率样本数未提供".into())
    }
    pub fn rank_label(&self) -> String {
        self.rank
            .map(|n| {
                format!(
                    "{} · 第 {n}",
                    if self.official {
                        "腾讯101推荐"
                    } else {
                        "来源排名"
                    }
                )
            })
            .unwrap_or_default()
    }
}
fn number(v: &Value, a: &str, b: &str) -> Option<f64> {
    v.get(a)
        .or_else(|| v.get(b))
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|v| v.is_finite())
}
/// A source rank/tier or a valid win rate can support ordering. Catalogue rarity,
/// popularity and sample counts alone cannot turn missing statistics into advice.
pub(crate) fn has_ordering_evidence(stats: &Value) -> bool {
    number(stats, "winRate", "win_rate").is_some_and(|v| (0.0..=1.0).contains(&v))
        || ["rank", "tier"]
            .iter()
            .any(|key| number(stats, key, key).is_some_and(|v| v >= 1. && v.fract() == 0.))
}
fn text(v: &Value, name: &str) -> String {
    v.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 80 && !s.chars().any(char::is_control))
        .unwrap_or("unknown")
        .into()
}
#[cfg(test)]
mod evidence_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn separates_win_samples_and_date_from_official_ranking() {
        let evidence = Evidence::from_stats(&json!({"games":423,"wins":233,
            "winRateSource":"client-upload","winRateDate":"2026-10-24",
            "date":"2026-10-28","rankDate":"20261028","rank":3,
            "rankSource":crate::tencent::SOURCE,"total":127}));
        assert_eq!(evidence.games, Some(423));
        assert_eq!(evidence.wins, Some(233));
        assert_eq!(evidence.win_date, "2026-10-24");
        assert_eq!(evidence.rank_label(), "腾讯101推荐 · 第 3");
        assert_eq!(evidence.sample_label(true), "胜率样本 · 423 场");
        assert_eq!(evidence.sample_label(false), "暂无胜率统计");
        let missing = Evidence::from_stats(&json!({"rank":1,"total":127,
            "winRateSource":"client-upload","date":"2026-10-28"}));
        assert_eq!(missing.games, None);
        assert!(missing.win_date.is_empty());
        assert_eq!(missing.sample_label(true), "胜率样本数未提供");
    }
    #[test]
    fn rejects_invalid_counts_without_inventing_a_sample() {
        for games in [json!(null), json!(-1), json!(0), json!(2.5)] {
            let e = Evidence::from_stats(&json!({"games":games,"wins":5}));
            assert_eq!(e.games, None);
            assert_eq!(e.wins, None);
        }
        assert_eq!(
            Evidence::from_stats(&json!({"games":10,"wins":11})).wins,
            None
        );
    }
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
                // Keep catalogue-only rows available for recognition/display, but
                // do not let them invalidate source ranks or acquire a score.
                if has_ordering_evidence(s) {
                    ordering.push((
                        id,
                        rank,
                        total,
                        number(s, "tier", "tier").filter(|v| *v >= 1. && v.fract() == 0.),
                        pick_rate,
                    ));
                }
                let row = Recommendation {
                    evidence: Evidence::from_stats(s),
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
        // A filtered source may retain non-contiguous ranks. Do not collapse
        // those ranks to zero merely because fewer rows have usable evidence.
        let rank_total = ordering
            .iter()
            .filter_map(|r| r.1)
            .fold(ordering.len() as f64, f64::max);
        if !official.is_empty() {
            for row in &ordering {
                let total = row.2.unwrap_or(rank_total);
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
            for row in &ordering {
                let total = row.2.unwrap_or(rank_total);
                result.get_mut(&row.0).unwrap().score = Some(if total <= 1. {
                    1.
                } else {
                    ((total - row.1.unwrap()) / (total - 1.)).clamp(0., 1.)
                });
            }
        }
        // Preserve the legacy fallback when genuine statistical rows lack ranks;
        // missing catalogue entries are excluded from this pool and its denominator.
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

#[cfg(test)]
mod ordering_tests {
    use super::*;
    use serde_json::json;

    fn prepared(augments: Value) -> Prepared {
        Prepared {
            locale: crate::Locale::ZhCn,
            version: "16.22.1".into(),
            source: crate::Source::Bundled,
            scope: crate::Scope::Champion(145),
            documents: std::collections::BTreeMap::from([(
                "champion-shards/145.json".into(),
                json!({"champions":{"145":{"augments":augments}}}),
            )]),
            catalog: Default::default(),
            current_augments: false,
        }
    }

    #[test]
    fn source_ranks_survive_missing_catalogue_stats_and_opposing_popularity() {
        let known = json!([
            {"id":1,"rarity":1,"stats":{"rank":1,"tier":1,"winRate":0.60,"pickRate":0.1}},
            {"id":2,"rarity":1,"stats":{"rank":2,"tier":1,"winRate":0.53,"pickRate":0.9}}
        ]);
        let before = prepared(known.clone()).recommendations(145).unwrap();
        let mut extended = known;
        extended.as_array_mut().unwrap().extend([
            json!({"id":3,"rarity":2,"stats":null}),
            json!({"id":4,"stats":{}}),
            json!({"id":5,"stats":{"pickRate":1.0,"games":999999}}),
        ]);
        let after = prepared(extended).recommendations(145).unwrap();
        assert_eq!(after[&1].score, Some(1.));
        assert_eq!(after[&2].score, Some(0.));
        for id in [1, 2] {
            assert_eq!(after[&id].score, before[&id].score);
            assert_eq!(after[&id].win_rate, before[&id].win_rate);
            assert_eq!(after[&id].evidence.rank, Some(id as u64));
        }
        for id in [3, 4, 5] {
            assert_eq!(after[&id].score, None);
        }
    }

    #[test]
    fn no_statistics_or_source_ranks_means_no_recommendation() {
        let data = prepared(json!([
            {"id":1,"rarity":2,"stats":null},
            {"id":2,"rarity":1,"stats":{}},
            {"id":3,"stats":{"pickRate":1.0,"games":999999}},
            {"id":4,"stats":{"winRate":1.1,"rank":0,"tier":0}}
        ]));
        assert!(data
            .recommendations(145)
            .unwrap()
            .values()
            .all(|r| r.score.is_none()));
    }

    #[test]
    fn genuine_unranked_statistics_keep_legacy_tier_and_popularity_fallback() {
        let data = prepared(json!([
            {"id":1,"stats":{"rank":1,"tier":1,"winRate":0.6,"pickRate":0.1}},
            {"id":2,"stats":{"tier":1,"winRate":0.55,"pickRate":0.9}},
            {"id":3,"stats":{"tier":2}},
            {"id":4,"stats":{"pickRate":1.0,"games":999999}}
        ]));
        let rows = data.recommendations(145).unwrap();
        assert_eq!(rows[&2].score, Some(1.));
        assert_eq!(rows[&1].score, Some(0.5));
        assert_eq!(rows[&3].score, Some(0.));
        assert_eq!(rows[&3].win_rate, None);
        assert_eq!(rows[&4].score, None);
    }

    #[test]
    fn legacy_rate_weighting_is_unchanged_without_source_ranks_or_tiers() {
        let data = prepared(json!([
            {"id":1,"stats":{"winRate":0.6,"pickRate":0.12}},
            {"id":2,"stats":{"winRate":0.7,"pickRate":0.1,"games":1000}},
            {"id":3,"stats":null}
        ]));
        let rows = data.recommendations(145).unwrap();
        assert!((rows[&1].score.unwrap() - 0.384).abs() < 1e-9);
        assert!((rows[&2].score.unwrap() - 0.64).abs() < 1e-9);
        assert_eq!(rows[&3].score, None);
    }

    #[test]
    fn official_ranks_still_exclude_other_ranking_sources() {
        let data = prepared(json!([
            {"id":1,"stats":{"rank":2,"total":5,"rankSource":crate::tencent::SOURCE}},
            {"id":2,"stats":{"rank":4,"total":5,"rankSource":crate::tencent::SOURCE}},
            {"id":3,"stats":{"rank":1,"tier":1,"winRate":0.99,"pickRate":1.0}},
            {"id":4,"stats":null}
        ]));
        let rows = data.recommendations(145).unwrap();
        assert_eq!(rows[&1].score, Some(0.75));
        assert_eq!(rows[&2].score, Some(0.25));
        assert_eq!(rows[&3].score, None);
        assert_eq!(rows[&3].win_rate, Some(0.99));
        assert_eq!(rows[&4].score, None);
    }

    #[test]
    fn sparse_source_ranks_remain_distinct_without_an_explicit_total() {
        let rows = prepared(json!([
            {"id":1,"stats":{"rank":5}},
            {"id":2,"stats":{"rank":8}},
            {"id":3,"stats":null}
        ]))
        .recommendations(145)
        .unwrap();
        assert!(rows[&1].score > rows[&2].score);
        assert_eq!(rows[&2].score, Some(0.));
        assert_eq!(rows[&3].score, None);
    }
}
