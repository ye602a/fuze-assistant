//! Formatting of provider combination statistics. No ownership inference or UI interaction.
use crate::stage_reference::Snapshot;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reference {
    pub ids: Vec<u32>,
    pub partners: String,
    pub statistic: String,
    pub detail: String,
}

/// `owned` must come from an actual selected-augment source, never the offered cards.
pub fn reference(
    snapshot: Option<&Snapshot>,
    champion: u32,
    patch: &str,
    owned: &[(u32, String)],
    candidate: u32,
    at: u64,
) -> Reference {
    let Some(s) = snapshot.filter(|s| s.usable(champion, at)) else {
        return Reference::default();
    };
    let patch = fuze_data::recognition::patch(patch);
    if patch.is_none()
        || patch != fuze_data::recognition::patch(&s.patch)
        || owned.iter().any(|(id, _)| *id == candidate)
    {
        return Reference::default();
    }
    let mut lines = Vec::new();
    let mut best = (0, 0);
    let mut view = Reference::default();
    for row in s.combinations.iter().flatten() {
        if !row.ids.contains(&candidate) {
            continue;
        }
        let partners: Option<Vec<_>> = row
            .ids
            .iter()
            .filter(|id| **id != candidate)
            .map(|id| {
                owned
                    .iter()
                    .find(|(p, _)| p == id)
                    .map(|(_, name)| name.as_str())
            })
            .collect();
        if let Some(names) = partners {
            // Prefer the most specific observed combination, then the larger sample, not the highest win rate.
            if (names.len(), row.samples) > best {
                best = (names.len(), row.samples);
                view.partners = names.join(" + ");
                view.ids = row
                    .ids
                    .iter()
                    .copied()
                    .filter(|id| *id != candidate)
                    .collect();
                view.statistic = format!(
                    "{}\n{:.1}% · {} 场{}",
                    if row.ids.len() == 2 {
                        "两符文搭配胜率"
                    } else {
                        "多符文搭配胜率"
                    },
                    row.win * 100.,
                    row.samples,
                    if row.samples < 200 {
                        " · 少样本"
                    } else {
                        ""
                    }
                );
            }
            lines.push(format!(
                "与 {}：{:.1}% · {} 场{}",
                names.join(" + "),
                row.win * 100.,
                row.samples,
                if row.samples < 200 {
                    " · 样本较少"
                } else {
                    ""
                }
            ));
        }
    }
    if lines.is_empty() {
        return view;
    }
    view.detail = format!(
        "搭配历史胜率 · ARAMKit {} · {}\n{}",
        s.patch,
        s.date,
        lines.join("\n")
    );
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_reference::Combination;
    fn detail(
        s: Option<&Snapshot>,
        hero: u32,
        patch: &str,
        owned: &[(u32, String)],
        candidate: u32,
        at: u64,
    ) -> String {
        reference(s, hero, patch, owned, candidate, at).detail
    }
    #[test]
    fn explicit_owned_set_matches_complete_rows_without_combining_their_rates() {
        let snapshot = Snapshot {
            schema: 1,
            champion: 145,
            patch: "16.22".into(),
            date: "2026-11-06".into(),
            data_path: String::new(),
            fetched: 100,
            rows: Default::default(),
            combinations: Some(vec![
                Combination {
                    ids: vec![1004, 3086],
                    win: 0.6495,
                    samples: 6819,
                },
                Combination {
                    ids: vec![2018, 3086],
                    win: 0.42,
                    samples: 120,
                },
                Combination {
                    ids: vec![1004, 2018, 3086],
                    win: 0.71,
                    samples: 400,
                },
                Combination {
                    ids: vec![1004, 9999, 3086],
                    win: 0.99,
                    samples: 999,
                },
            ]),
        };
        assert!(detail(Some(&snapshot), 145, "26.22.1", &[], 3086, 101).is_empty());
        let mut owned = vec![(1004, "符文甲".into())];
        let one = detail(Some(&snapshot), 145, "26.22.1", &owned, 3086, 101);
        assert!(one.contains("与 符文甲：65.0% · 6819 场"));
        assert!(!one.contains("71.0%") && !one.contains("42.0%") && !one.contains("99.0%"));
        owned.push((2018, "符文乙".into()));
        let two = detail(Some(&snapshot), 145, "16.22", &owned, 3086, 101);
        let view = reference(Some(&snapshot), 145, "16.22", &owned, 3086, 101);
        assert_eq!(view.ids, vec![1004, 2018]);
        assert_eq!(view.statistic, "多符文搭配胜率\n71.0% · 400 场");
        assert!(two.contains("与 符文甲：65.0% · 6819 场"));
        assert!(two.contains("与 符文乙：42.0% · 120 场 · 样本较少"));
        assert!(two.contains("与 符文甲 + 符文乙：71.0% · 400 场"));
        assert_eq!(two.lines().count(), 4);
        assert!(detail(Some(&snapshot), 145, "16.22", &owned, 1004, 101).is_empty());
        assert!(detail(Some(&snapshot), 145, "16.22", &owned, 999, 101).is_empty());
        assert!(detail(None, 145, "16.22", &owned, 3086, 101).is_empty());
        assert!(detail(
            Some(&snapshot),
            145,
            "16.22",
            &owned,
            3086,
            100 + 7 * 86400 + 1
        )
        .is_empty());
        assert!(detail(Some(&snapshot), 145, "16.21", &owned, 3086, 101).is_empty());
        assert!(detail(Some(&snapshot), 145, "", &owned, 3086, 101).is_empty());
        assert!(detail(Some(&snapshot), 17, "16.22", &owned, 3086, 101).is_empty());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataState {
    Unavailable,
    PatchMismatch,
    Ready,
}
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostics {
    pub data: DataState,
    pub rows: u16,
    pub matched: u8,
    pub delivered: bool,
}
