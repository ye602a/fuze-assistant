//! Formatting of provider combination statistics. No ownership inference or UI interaction.
use crate::stage_reference::Snapshot;

/// `owned` must come from an actual selected-augment source, never the offered cards.
/// Production has no such source yet, so this formatter is not wired to live recommendations.
pub fn detail(
    snapshot: Option<&Snapshot>,
    champion: u32,
    patch: &str,
    owned: &[(u32, String)],
    candidate: u32,
    at: u64,
) -> String {
    let Some(s) = snapshot.filter(|s| s.usable(champion, at)) else {
        return String::new();
    };
    let patch = fuze_data::recognition::patch(patch);
    if patch.is_none()
        || patch != fuze_data::recognition::patch(&s.patch)
        || owned.iter().any(|(id, _)| *id == candidate)
    {
        return String::new();
    }
    let mut lines = Vec::new();
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
        return String::new();
    }
    format!(
        "搭配历史胜率 · ARAMKit {} · {}\n{}",
        s.patch,
        s.date,
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_reference::Combination;
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
