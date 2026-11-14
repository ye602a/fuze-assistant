//! Immutable, descriptive evidence for a single champion and data batch.
//! Hero totals and augment subsets are not independent treatment/control samples.
//! These comparisons never alter source recommendation scores.
use crate::{
    recommendations::{Evidence, Recommendation},
    Prepared, Result,
};
use serde_json::Value;
use std::collections::HashMap;

/// Display guidance only; it does not hide a reported rate or gate ordering.
pub const LOW_SAMPLE_GAMES: u64 = 200;
/// A descriptive display band, not a statistical equivalence test.
pub const NEAR_MARGIN: f64 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub value: f64,
    pub games: Option<u64>,
    pub wins: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub lower: f64,
    pub upper: f64,
}

/// Approximate 95% Wilson sampling interval for one reported proportion.
/// Rounded provider rates remain rounded; no integer win count is invented.
/// This does not account for cohort selection, repeated players, or source bias.
/// Formula: https://www.itl.nist.gov/div898/handbook/prc/section2/prc241.htm
pub fn interval95(value: f64, games: u64) -> Option<Interval> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) || games == 0 {
        return None;
    }
    let n = games as f64;
    let z = 1.959_963_984_540_054;
    let z2 = z * z;
    let denominator = 1. + z2 / n;
    let center = (value + z2 / (2. * n)) / denominator;
    let half = z * ((value * (1. - value) + z2 / (4. * n)) / n).sqrt() / denominator;
    Some(Interval {
        lower: (center - half).clamp(0., 1.),
        upper: (center + half).clamp(0., 1.),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    pub source: String,
    pub patch: String,
    pub date: String,
    pub region: String,
}

impl Scope {
    /// ARAMKit's stage/pair snapshots carry a patch and date but no region.
    /// Matching them establishes the provider batch, not identical populations.
    pub fn matches_aramkit(&self, patch: &str, date: &str) -> bool {
        self.source == crate::aramkit::SOURCE
            && reported(&self.patch)
            && reported(&self.date)
            && self.patch == patch
            && self.date == date
    }

    fn same_batch(&self, other: &Self) -> bool {
        reported(&self.source)
            && reported(&self.patch)
            && reported(&self.date)
            && self == other
            && (known_region(&self.region)
                || (self.source == crate::aramkit::SOURCE && self.region == "地区未标明"))
    }

    fn from_stats(stats: &Value) -> Self {
        let explicit = stats["winRateSource"]
            .as_str()
            .is_some_and(|s| !s.is_empty());
        Self {
            source: text(stats, if explicit { "winRateSource" } else { "source" }),
            patch: text(
                stats,
                if explicit {
                    "winRateGamePatch"
                } else {
                    "gamePatch"
                },
            ),
            date: Evidence::from_stats(stats).win_date,
            region: text(stats, if explicit { "winRateRegion" } else { "region" }),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeroReference {
    pub champion: u32,
    pub rate: Rate,
    pub scope: Scope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Higher,
    Near,
    Lower,
}

#[derive(Clone, Debug, PartialEq)]
pub struct History {
    /// Difference of reported proportions; multiply by 100 for percentage points.
    pub delta: f64,
    /// The augment's own sampling interval, not an interval for the difference.
    pub interval: Option<Interval>,
    pub relation: Relation,
    pub low_sample: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AugmentHistory {
    pub rate: Option<Rate>,
    pub scope: Scope,
    pub comparison: Option<History>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryContext {
    pub dataset: String,
    pub champion: u32,
    pub baseline: Option<HeroReference>,
    pub by_augment: HashMap<u32, AugmentHistory>,
}

impl HistoryContext {
    /// All inputs must come from this Prepared champion shard. Metadata checks
    /// prevent mixed source/date rates from borrowing the hero's reference.
    /// Missing hero evidence leaves each augment's own rate and scope available.
    pub fn new(
        dataset: String,
        champion: u32,
        baseline: Option<HeroReference>,
        recommendations: &HashMap<u32, Recommendation>,
    ) -> Self {
        let baseline = baseline.filter(|b| b.champion == champion && valid_rate(b.rate.value));
        let by_augment = recommendations
            .iter()
            .map(|(id, row)| {
                let rate = rate(row.win_rate, &row.evidence);
                let scope = Scope {
                    source: row.origin.clone(),
                    patch: row.patch.clone(),
                    date: row.evidence.win_date.clone(),
                    region: row.region.clone(),
                };
                let comparison = rate.and_then(|r| {
                    let baseline = baseline.as_ref()?;
                    if dataset.is_empty() || !scope.same_batch(&baseline.scope) {
                        return None;
                    }
                    let delta = r.value - baseline.rate.value;
                    Some(History {
                        delta,
                        interval: r.games.and_then(|n| interval95(r.value, n)),
                        relation: if delta.abs() <= NEAR_MARGIN + 1e-12 {
                            Relation::Near
                        } else if delta > 0. {
                            Relation::Higher
                        } else {
                            Relation::Lower
                        },
                        low_sample: r.games.is_some_and(|n| n < LOW_SAMPLE_GAMES),
                    })
                });
                (
                    *id,
                    AugmentHistory {
                        rate,
                        scope,
                        comparison,
                    },
                )
            })
            .collect();
        Self {
            dataset,
            champion,
            baseline,
            by_augment,
        }
    }
}

fn valid_rate(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn rate(value: Option<f64>, evidence: &Evidence) -> Option<Rate> {
    let value = value.filter(|v| valid_rate(*v))?;
    let games = evidence.games.filter(|n| *n > 0);
    Some(Rate {
        value,
        games,
        wins: evidence.wins.filter(|w| games.is_some_and(|n| *w <= n)),
    })
}

fn reported(s: &str) -> bool {
    !s.is_empty() && !s.eq_ignore_ascii_case("unknown")
}

fn known_region(s: &str) -> bool {
    reported(s) && s != "地区未标明"
}

fn text(stats: &Value, name: &str) -> String {
    stats[name]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 80 && !s.chars().any(char::is_control))
        .unwrap_or("unknown")
        .into()
}

impl Prepared {
    /// Read the hero total from the same detail shard as its augment statistics.
    /// No global-catalogue fallback or aggregate of augment subsets is substituted.
    pub fn hero_reference(&self, champion: u32) -> Result<Option<HeroReference>> {
        if champion == 0 {
            return Err("data-champion-missing");
        }
        let key = champion.to_string();
        let mut rows = self
            .documents
            .iter()
            .filter(|(p, _)| p.starts_with("champion-shards/") && !p.ends_with("/index.json"))
            .filter_map(|(_, document)| document.get("champions")?.get(&key));
        let row = rows.next().ok_or("data-champion-missing")?;
        if rows.next().is_some() {
            return Err("data-duplicate-champion");
        }
        let stats = &row["champion"]["stats"];
        let value = stats
            .get("winRate")
            .or_else(|| stats.get("win_rate"))
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()));
        Ok(
            rate(value, &Evidence::from_stats(stats)).map(|rate| HeroReference {
                champion,
                rate,
                scope: Scope::from_stats(stats),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stats(value: f64, games: u64) -> Value {
        json!({"source": crate::aramkit::SOURCE, "region":"地区未标明",
            "gamePatch":"16.22", "date":"2026-11-12", "winRate":value, "games":games})
    }

    fn prepared(hero: Value, augments: Value) -> Prepared {
        Prepared {
            locale: crate::Locale::ZhCn,
            version: "16.22.1".into(),
            source: crate::Source::Bundled,
            scope: crate::Scope::Champion(145),
            documents: std::collections::BTreeMap::from([
                (
                    "champions.json".into(),
                    json!({"data":[{"id":145,"stats":stats(0.99, 99999)}]}),
                ),
                (
                    "champion-shards/145.json".into(),
                    json!({"champions":{"145":{
                    "champion":{"id":145,"stats":hero},"augments":augments}}}),
                ),
            ]),
            catalog: Default::default(),
            current_augments: false,
        }
    }

    fn context(data: &Prepared) -> HistoryContext {
        HistoryContext::new(
            data.identity(),
            145,
            data.hero_reference(145).unwrap(),
            &data.recommendations(145).unwrap(),
        )
    }

    #[test]
    fn wilson_interval_handles_small_samples_and_boundaries_without_fake_wins() {
        let middle = interval95(0.5, 100).unwrap();
        assert!((middle.lower - 0.4038315303659957).abs() < 1e-12);
        assert!((middle.upper - 0.5961684696340044).abs() < 1e-12);
        let zero = interval95(0., 10).unwrap();
        let one = interval95(1., 10).unwrap();
        assert!(zero.lower >= 0. && zero.lower < 1e-15);
        assert!((zero.upper - 0.2775327998628892).abs() < 1e-12);
        assert!((one.lower - (1. - zero.upper)).abs() < 1e-12);
        assert!(one.upper <= 1. && one.upper > 1. - 1e-15);
        assert!(interval95(0.5, 0).is_none());
        for p in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
            assert!(interval95(p, 200).is_none());
        }
        let data = prepared(
            stats(0.5, 10000),
            json!([{"id":1,"stats":stats(0.5344, 123)}]),
        );
        let h = context(&data);
        assert_eq!(h.by_augment[&1].rate.unwrap().wins, None);
        assert!(h.by_augment[&1].comparison.as_ref().unwrap().low_sample);
    }

    #[test]
    fn uses_detail_baseline_and_keeps_all_below_baseline_source_ranks() {
        let mut augments = json!([]);
        for (i, p) in [0.58, 0.56, 0.54].into_iter().enumerate() {
            let mut s = stats(p, 2000);
            s["rank"] = json!(i + 1);
            augments
                .as_array_mut()
                .unwrap()
                .push(json!({"id":i + 1,"stats":s}));
        }
        let data = prepared(stats(0.60, 10000), augments);
        let recommendations = data.recommendations(145).unwrap();
        let h = HistoryContext::new(
            data.identity(),
            145,
            data.hero_reference(145).unwrap(),
            &recommendations,
        );
        assert_eq!(h.baseline.as_ref().unwrap().rate.value, 0.60);
        for id in 1..=3 {
            assert_eq!(
                h.by_augment[&id].comparison.as_ref().unwrap().relation,
                Relation::Lower
            );
        }
        assert_eq!(recommendations[&1].score, Some(1.));
        assert_eq!(recommendations[&2].score, Some(0.5));
        assert_eq!(recommendations[&3].score, Some(0.));
    }

    #[test]
    fn missing_baseline_or_counts_does_not_remove_rows_or_invent_evidence() {
        let mut own_stats = stats(0.55, 1000);
        own_stats.as_object_mut().unwrap().remove("games");
        let data = prepared(
            Value::Null,
            json!([
                {"id":1,"stats":own_stats}, {"id":2,"stats":null},
                {"id":3,"stats":{"games":10000,"pickRate":0.5,"rank":1}}
            ]),
        );
        let h = context(&data);
        assert!(h.baseline.is_none());
        assert_eq!(h.by_augment.len(), 3);
        assert_eq!(
            h.by_augment[&1].rate,
            Some(Rate {
                value: 0.55,
                games: None,
                wins: None
            })
        );
        assert!(h.by_augment[&1]
            .scope
            .matches_aramkit("16.22", "2026-11-12"));
        assert!(h.by_augment[&1].comparison.is_none());
        assert!(h.by_augment[&2].rate.is_none());
        assert!(h.by_augment[&3].rate.is_none());
        let mut baseline = stats(0.50, 1000);
        baseline["wins"] = json!(500);
        let data = prepared(baseline, json!([{"id":1,"stats":own_stats}]));
        let h = context(&data);
        assert_eq!(h.baseline.as_ref().unwrap().rate.wins, Some(500));
        let comparison = h.by_augment[&1].comparison.as_ref().unwrap();
        assert!(comparison.interval.is_none());
        assert!(!comparison.low_sample);
    }

    #[test]
    fn incompatible_scope_cannot_borrow_baseline_and_rank_metadata_is_separate() {
        for (field, value) in [
            ("source", "other"),
            ("gamePatch", "16.21"),
            ("date", "2026-11-11"),
            ("region", "WORLD"),
        ] {
            let mut s = stats(0.6, 1000);
            s[field] = json!(value);
            let data = prepared(stats(0.5, 10000), json!([{"id":1,"stats":s}]));
            let h = context(&data);
            assert!(h.by_augment[&1].rate.is_some());
            assert!(h.by_augment[&1].comparison.is_none(), "{field}");
        }
        let mut s = stats(0.6, 1000);
        s["rankSource"] = json!(crate::tencent::SOURCE);
        s["rank"] = json!(1);
        s["date"] = json!("20261113");
        s["winRateSource"] = json!(crate::aramkit::SOURCE);
        s["winRateDate"] = json!("2026-11-12");
        s["winRateGamePatch"] = json!("16.22");
        s["winRateRegion"] = json!("地区未标明");
        let data = prepared(stats(0.5, 10000), json!([{"id":1,"stats":s}]));
        let h = context(&data);
        assert!(h.by_augment[&1].comparison.is_some());
        assert!(h.by_augment[&1]
            .scope
            .matches_aramkit("16.22", "2026-11-12"));
        assert!(!h.by_augment[&1]
            .scope
            .matches_aramkit("16.22", "2026-11-13"));
    }

    #[test]
    fn near_is_an_inclusive_display_band_and_missing_metadata_is_not_a_match() {
        for p in [0.49, 0.5, 0.51] {
            let data = prepared(stats(0.5, 10000), json!([{"id":1,"stats":stats(p, 200)}]));
            let h = context(&data);
            let comparison = h.by_augment[&1].comparison.as_ref().unwrap();
            assert_eq!(comparison.relation, Relation::Near);
            assert!(!comparison.low_sample);
        }
        let data = prepared(
            json!({"winRate":0.5}),
            json!([{"id":1,"stats":{"winRate":0.6}}]),
        );
        assert!(context(&data).by_augment[&1].comparison.is_none());
        assert!(!Scope::default().matches_aramkit("", ""));
        let data = prepared(
            stats(0.5, 10000),
            json!([{"id":1,"stats":stats(0.6, 1000)}]),
        );
        let mut wrong_hero = data.hero_reference(145).unwrap().unwrap();
        wrong_hero.champion = 17;
        let h = HistoryContext::new(
            data.identity(),
            145,
            Some(wrong_hero),
            &data.recommendations(145).unwrap(),
        );
        assert!(h.baseline.is_none());
        assert!(h.by_augment[&1].comparison.is_none());
    }
}
