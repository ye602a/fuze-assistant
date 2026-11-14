//! Observed combinations and future routes. No ownership inference or automatic ranking.
use crate::stage_reference::Snapshot;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceState {
    Ready { stale: bool },
    Unavailable,
    Expired,
    PatchMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchState {
    CurrentMatch,
    FutureOnly,
    NoOwned,
    NoRecord,
    AlreadyOwned,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComboEvidence {
    pub members: Vec<u32>,
    pub required: Vec<u32>,
    pub missing: Vec<u32>,
    pub win: f64,
    pub samples: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateEvidence {
    pub candidate: u32,
    pub source: SourceState,
    /// Only meaningful when the source is ready. Missing data is not a negative match.
    pub match_state: MatchState,
    pub current: Vec<ComboEvidence>,
    pub future: Vec<ComboEvidence>,
}

/// `owned` contains only confirmed, still-valid selections, never the other offered cards.
/// Future routes retain observed rows, not a prediction of which augments will be offered.
pub fn evaluate(
    snapshot: Option<&Snapshot>,
    champion: u32,
    patch: &str,
    owned: &[(u32, String)],
    candidate: u32,
    at: u64,
) -> CandidateEvidence {
    let mut evidence = CandidateEvidence {
        candidate,
        source: SourceState::Unavailable,
        match_state: MatchState::NoRecord,
        current: Vec::new(),
        future: Vec::new(),
    };
    let Some(s) = snapshot.filter(|s| s.champion == champion && s.fetched <= at) else {
        return evidence;
    };
    if !s.usable(champion, at) {
        evidence.source = SourceState::Expired;
        return evidence;
    }
    let patch = fuze_data::recognition::patch(patch);
    if patch.is_none() || patch != fuze_data::recognition::patch(&s.patch) {
        evidence.source = SourceState::PatchMismatch;
        return evidence;
    }
    let Some(rows) = s.combinations.as_ref() else {
        return evidence;
    };
    evidence.source = SourceState::Ready { stale: s.stale(at) };
    if owned.iter().any(|(id, _)| *id == candidate) {
        evidence.match_state = MatchState::AlreadyOwned;
        return evidence;
    }
    for row in rows.iter().filter(|r| r.ids.contains(&candidate)) {
        let mut members = row.ids.clone();
        members.sort_unstable();
        let required: Vec<_> = members
            .iter()
            .copied()
            .filter(|id| *id != candidate)
            .collect();
        let missing: Vec<_> = required
            .iter()
            .copied()
            .filter(|id| !owned.iter().any(|(held, _)| held == id))
            .collect();
        let current = missing.is_empty();
        let row = ComboEvidence {
            members,
            required,
            missing,
            win: row.win,
            samples: row.samples,
        };
        if current {
            evidence.current.push(row);
        } else {
            evidence.future.push(row);
        }
    }
    // Use specificity and support for display, not the highest observed win rate.
    evidence.current.sort_by(|a, b| {
        b.required
            .len()
            .cmp(&a.required.len())
            .then(b.samples.cmp(&a.samples))
            .then(a.members.cmp(&b.members))
    });
    evidence.future.sort_by(|a, b| {
        a.missing
            .len()
            .cmp(&b.missing.len())
            .then(b.samples.cmp(&a.samples))
            .then(a.members.cmp(&b.members))
    });
    evidence.match_state = if !evidence.current.is_empty() {
        MatchState::CurrentMatch
    } else if owned.is_empty() {
        MatchState::NoOwned
    } else if !evidence.future.is_empty() {
        MatchState::FutureOnly
    } else {
        MatchState::NoRecord
    };
    evidence
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reference {
    pub heading: String,
    pub summary: String,
    pub ids: Vec<u32>,
    pub partners: String,
    pub statistic: String,
    pub detail: String,
}

fn names(ids: &[u32], catalog: &HashMap<u32, String>) -> String {
    ids.iter()
        .map(|id| {
            catalog
                .get(id)
                .filter(|name| !name.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("强化 #{id}"))
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

fn statistic(row: &ComboEvidence) -> String {
    format!(
        "{:.1}% · {} 场{}",
        row.win * 100.,
        row.samples,
        if row.samples < 200 {
            " · 少样本"
        } else {
            ""
        },
    )
}

/// Formats explicit states and observed rows. `preferred_required` can select the
/// exact current group used by the caller's comparison; it cannot promote a future route.
pub fn present(
    evidence: &CandidateEvidence,
    catalog: &HashMap<u32, String>,
    preferred_required: Option<&[u32]>,
) -> Reference {
    let unavailable = match evidence.source {
        SourceState::Unavailable => Some("组合数据尚未就绪"),
        SourceState::Expired => Some("组合缓存已过期"),
        SourceState::PatchMismatch => Some("组合数据与当前补丁不一致"),
        SourceState::Ready { .. } => None,
    };
    if let Some(message) = unavailable {
        return Reference {
            heading: "组合参考".into(),
            summary: message.into(),
            detail: message.into(),
            ..Default::default()
        };
    }
    let preferred = preferred_required.map(|ids| {
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        ids
    });
    let current = preferred
        .as_ref()
        .and_then(|ids| evidence.current.iter().find(|row| row.required == *ids))
        .or_else(|| evidence.current.first());
    let mut view = if let Some(row) = current {
        let partners = names(&row.required, catalog);
        let summary = statistic(row);
        Reference {
            heading: format!("当前搭配 · {partners}"),
            summary: format!("搭配历史胜率 {summary}"),
            ids: row.required.clone(),
            partners,
            statistic: format!(
                "{}\n{summary}",
                if row.members.len() == 2 {
                    "两符文搭配胜率"
                } else {
                    "多符文搭配胜率"
                },
            ),
            ..Default::default()
        }
    } else if let Some(row) = evidence.future.first() {
        // Only name the missing partners as future targets, not already-held members.
        let partners = names(&row.missing, catalog);
        let summary = statistic(row);
        Reference {
            heading: format!("后续留意 · {partners}"),
            summary: format!("该搭配历史胜率 {summary}"),
            ids: row.missing.clone(),
            partners,
            statistic: format!("后续路线历史胜率\n{summary}"),
            ..Default::default()
        }
    } else {
        let (heading, summary) = match evidence.match_state {
            MatchState::NoOwned => ("尚无已确认的组合前置", "公开榜单暂无此符文的后续路线记录"),
            MatchState::AlreadyOwned => ("此符文已确认持有", "不作为新的组合候选"),
            _ => ("暂无对应组合记录", "公开榜单有限，缺失记录不代表不搭配"),
        };
        Reference {
            heading: heading.into(),
            summary: summary.into(),
            ..Default::default()
        }
    };
    let mut lines = Vec::new();
    for row in &evidence.current {
        lines.push(format!(
            "当前搭配 · {}：{}",
            names(&row.required, catalog),
            statistic(row)
        ));
    }
    for row in &evidence.future {
        lines.push(format!(
            "后续路线 · 尚缺 {}：{}",
            names(&row.missing, catalog),
            statistic(row),
        ));
    }
    if lines.is_empty() {
        lines.push(format!("{}\n{}", view.heading, view.summary));
    }
    if !evidence.future.is_empty() {
        lines.push("后续路线尚未成立，是否出现未确认；不改变当前排序。".into());
    }
    if matches!(evidence.source, SourceState::Ready { stale: true }) {
        lines.push("组合缓存待刷新。".into());
    }
    view.detail = lines.join("\n");
    view
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
    let evidence = evaluate(snapshot, champion, patch, owned, candidate, at);
    if evidence.current.is_empty() {
        return Reference::default();
    }
    let s = snapshot.unwrap();
    let catalog = owned.iter().cloned().collect();
    let mut view = present(&evidence, &catalog, None);
    let lines: Vec<_> = evidence
        .current
        .iter()
        .map(|row| {
            format!(
                "与 {}：{:.1}% · {} 场{}",
                names(&row.required, &catalog),
                row.win * 100.,
                row.samples,
                if row.samples < 200 {
                    " · 样本较少"
                } else {
                    ""
                },
            )
        })
        .collect();
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
    fn snapshot() -> Snapshot {
        Snapshot {
            schema: 1,
            champion: 145,
            patch: "16.22".into(),
            date: "2026-11-12".into(),
            data_path: String::new(),
            fetched: 100,
            rows: Default::default(),
            combinations: Some(vec![
                Combination {
                    ids: vec![3086, 1004],
                    win: 0.6505,
                    samples: 26552,
                },
                Combination {
                    ids: vec![3086, 2018],
                    win: 0.42,
                    samples: 120,
                },
                Combination {
                    ids: vec![2018, 3086, 1004],
                    win: 0.71,
                    samples: 400,
                },
                Combination {
                    ids: vec![9999, 1004, 3086],
                    win: 0.99,
                    samples: 999,
                },
            ]),
        }
    }

    #[test]
    fn evidence_separates_current_and_future_without_inventing_ownership() {
        let source = snapshot();
        let owned = vec![(1004, "符文甲".into())];
        let e = evaluate(Some(&source), 145, "26.22.1", &owned, 3086, 101);
        assert_eq!(e.source, SourceState::Ready { stale: false });
        assert_eq!(e.match_state, MatchState::CurrentMatch);
        assert_eq!(e.current.len(), 1);
        assert_eq!(e.current[0].members, [1004, 3086]);
        assert_eq!(e.current[0].required, [1004]);
        assert!(e.current[0].missing.is_empty());
        assert_eq!(e.current[0].win, 0.6505);
        assert_eq!(e.current[0].samples, 26552);
        assert_eq!(e.future.len(), 3);
        let triple = e
            .future
            .iter()
            .find(|r| r.members.len() == 3 && r.members.contains(&2018))
            .unwrap();
        assert_eq!(triple.required, [1004, 2018]);
        assert_eq!(triple.missing, [2018]);
        assert_eq!(
            e.future.iter().find(|r| r.samples == 120).unwrap().win,
            0.42
        );
        // Removing confirmed ownership moves complete matches back to future routes.
        let cleared = evaluate(Some(&source), 145, "16.22", &[], 3086, 101);
        assert_eq!(cleared.match_state, MatchState::NoOwned);
        assert!(cleared.current.is_empty());
        assert_eq!(cleared.future.len(), 4);
        assert_eq!(cleared.future[0].required, [1004]);
        assert_eq!(cleared.future[0].missing, [1004]);
        assert_eq!(cleared.future[1].samples, 120); // Fewer missing partners, not higher win rate.
        assert_eq!(owned, [(1004, "符文甲".into())]);
        let unrelated_owned = [(77, "无关前置".into())];
        let future = evaluate(Some(&source), 145, "16.22", &unrelated_owned, 3086, 101);
        assert_eq!(future.match_state, MatchState::FutureOnly);
        assert!(future.current.is_empty());
        let already_owned = evaluate(Some(&source), 145, "16.22", &owned, 1004, 101);
        assert_eq!(already_owned.match_state, MatchState::AlreadyOwned);
        assert!(already_owned.current.is_empty() && already_owned.future.is_empty());
    }

    #[test]
    fn source_status_distinguishes_missing_expired_patch_and_empty_records() {
        let mut source = snapshot();
        let owned = [(1004, "符文甲".into())];
        let none = evaluate(None, 145, "16.22", &owned, 3086, 101);
        assert_eq!(none.source, SourceState::Unavailable);
        assert_eq!(
            evaluate(Some(&source), 17, "16.22", &owned, 3086, 101).source,
            SourceState::Unavailable
        );
        assert_eq!(
            evaluate(Some(&source), 145, "16.22", &owned, 3086, 99).source,
            SourceState::Unavailable
        );
        for patch in ["", "16.21"] {
            let e = evaluate(Some(&source), 145, patch, &owned, 3086, 101);
            assert_eq!(e.source, SourceState::PatchMismatch);
            assert!(e.current.is_empty() && e.future.is_empty());
        }
        let retained = evaluate(Some(&source), 145, "16.22", &owned, 3086, 100 + 7 * 86400);
        assert_eq!(retained.source, SourceState::Ready { stale: true });
        let expired = evaluate(Some(&source), 145, "16.22", &owned, 3086, 101 + 7 * 86400);
        assert_eq!(expired.source, SourceState::Expired);
        assert!(expired.current.is_empty() && expired.future.is_empty());
        source.combinations = None;
        assert!(source.stale(101));
        let unavailable = evaluate(Some(&source), 145, "16.22", &owned, 3086, 101);
        assert_eq!(unavailable.source, SourceState::Unavailable); // Missing combinations are not expired stages.
        source.combinations = Some(Vec::new());
        let empty = evaluate(Some(&source), 145, "16.22", &owned, 3086, 101);
        assert_eq!(empty.source, SourceState::Ready { stale: false });
        assert_eq!(empty.match_state, MatchState::NoRecord);
        assert!(empty.current.is_empty() && empty.future.is_empty());
        let names = HashMap::new();
        assert_eq!(
            present(&unavailable, &names, None).summary,
            "组合数据尚未就绪"
        );
        assert_eq!(present(&expired, &names, None).summary, "组合缓存已过期");
        assert_eq!(present(&empty, &names, None).heading, "暂无对应组合记录");
        assert!(present(&empty, &names, None)
            .summary
            .contains("不代表不搭配"));
    }

    #[test]
    fn presentation_uses_exact_compared_group_and_marks_future_targets_and_small_samples() {
        let mut source = snapshot();
        let names = HashMap::from([(1004, "符文甲".into()), (2018, "符文乙".into())]);
        let owned = [(1004, "符文甲".into()), (2018, "符文乙".into())];
        let e = evaluate(Some(&source), 145, "16.22", &owned, 3086, 101);
        assert_eq!(present(&e, &names, None).ids, [1004, 2018]);
        let compared = present(&e, &names, Some(&[1004]));
        assert_eq!(compared.ids, [1004]);
        assert_eq!(compared.heading, "当前搭配 · 符文甲");
        assert!(compared.summary.contains("65.0% · 26552 场"));
        let small = present(&e, &names, Some(&[2018]));
        assert!(small.summary.contains("42.0% · 120 场 · 少样本"));
        let future_key = present(&e, &names, Some(&[1004, 9999]));
        assert_eq!(future_key.ids, [1004, 2018]); // Future-only keys cannot override a current group.
        let unordered = present(&e, &names, Some(&[2018, 1004]));
        assert_eq!(unordered.ids, [1004, 2018]);
        // One real triple; its missing member is not fabricated into a pairwise statistic.
        source
            .combinations
            .as_mut()
            .unwrap()
            .retain(|r| r.ids.contains(&9999));
        let e = evaluate(Some(&source), 145, "16.22", &owned, 3086, 101);
        let future = present(&e, &names, None);
        assert_eq!(future.heading, "后续留意 · 强化 #9999");
        assert_eq!(future.ids, [9999]);
        assert!(future.summary.contains("99.0% · 999 场"));
        assert!(!future.heading.contains("符文甲"));
        assert!(
            future.detail.contains("是否出现未确认") && future.detail.contains("不改变当前排序")
        );
        assert!(reference(Some(&source), 145, "16.22", &owned, 3086, 101)
            .detail
            .is_empty());
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
