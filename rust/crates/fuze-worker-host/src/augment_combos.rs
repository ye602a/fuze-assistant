//! Manually confirmed augments. Never feeds recognition gates or automatic ranking.
use crate::stage_reference::Snapshot;
#[derive(Default)]
pub struct Selection {
    pub revision: u64,
    pub champion: u32,
    pub patch: String,
    pub picked: Vec<(u32, String)>,
}

impl Selection {
    pub fn context(&mut self, champion: u32, patch: &str) {
        if self.champion != champion {
            self.champion = champion;
            self.clear();
        }
        if self.patch != patch {
            self.patch = patch.into();
            self.revision = self.revision.wrapping_add(1);
        }
    }
    pub fn clear(&mut self) {
        self.picked.clear();
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn toggle(&mut self, champion: u32, id: u32, name: &str) {
        if champion == 0 || champion != self.champion || id == 0 || id > 100000 {
            return;
        }
        if let Some(index) = self.picked.iter().position(|(a, _)| *a == id) {
            self.picked.remove(index);
        } else {
            self.picked.push((id, name.into()));
        }
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn detail(&self, snapshot: Option<&Snapshot>, candidate: u32, at: u64) -> String {
        let Some(s) = snapshot.filter(|s| s.usable(self.champion, at)) else {
            return String::new();
        };
        if fuze_data::recognition::patch(&self.patch).is_none()
            || fuze_data::recognition::patch(&self.patch) != fuze_data::recognition::patch(&s.patch)
            || self.picked.iter().any(|(id, _)| *id == candidate)
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
                    self.picked
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_reference::Combination;

    #[test]
    fn only_complete_recorded_combinations_keep_their_own_rates_and_samples() {
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
        let mut s = Selection::default();
        s.context(145, "26.22.1");
        assert!(s.detail(Some(&snapshot), 3086, 101).is_empty());
        s.toggle(145, 1004, "符文甲");
        let one = s.detail(Some(&snapshot), 3086, 101);
        assert!(one.contains("与 符文甲：65.0% · 6819 场"));
        assert!(!one.contains("71.0%") && !one.contains("42.0%") && !one.contains("99.0%"));
        s.toggle(145, 2018, "符文乙");
        let two = s.detail(Some(&snapshot), 3086, 101);
        assert!(two.contains("与 符文甲：65.0% · 6819 场"));
        assert!(two.contains("与 符文乙：42.0% · 120 场 · 样本较少"));
        assert!(two.contains("与 符文甲 + 符文乙：71.0% · 400 场"));
        assert_eq!(two.lines().count(), 4);
        assert!(s.detail(Some(&snapshot), 1004, 101).is_empty()); // Already owned.
        assert!(s.detail(Some(&snapshot), 999, 101).is_empty());
        assert!(s.detail(None, 3086, 101).is_empty());
        assert!(s
            .detail(Some(&snapshot), 3086, 100 + 7 * 86400 + 1)
            .is_empty());
        s.context(145, "16.21");
        assert!(s.detail(Some(&snapshot), 3086, 101).is_empty());
        s.context(145, "");
        assert!(s.detail(Some(&snapshot), 3086, 101).is_empty());
        assert_eq!(s.picked.len(), 2); // A late public-version response does not erase picks.
        s.context(17, "16.22");
        assert!(s.picked.is_empty());
        assert!(s.detail(Some(&snapshot), 3086, 101).is_empty());
    }

    #[test]
    fn manual_records_are_reversible_and_tied_to_the_current_champion() {
        let mut s = Selection::default();
        s.context(145, "16.22");
        s.toggle(17, 1004, "另一英雄");
        assert!(s.picked.is_empty());
        s.toggle(145, 1004, "符文甲");
        let revision = s.revision;
        s.context(145, "16.22");
        assert_eq!(s.revision, revision);
        s.toggle(145, 1004, "符文甲");
        assert!(s.picked.is_empty());
        assert_ne!(s.revision, revision);
        s.toggle(145, 3086, "符文乙");
        s.clear();
        assert!(s.picked.is_empty());
        s.toggle(145, 3086, "符文乙");
        s.context(0, "");
        assert!(s.picked.is_empty());
    }
}
