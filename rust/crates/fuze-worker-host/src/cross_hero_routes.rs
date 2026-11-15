//! Catalogue routes observed for OTHER champions. These are never champion win
//! rates, comparative recommendation evidence, or predictions of future offers.
use fuze_data::combination_routes::Index;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct RouteEvidence {
    pub members: Vec<u32>,
    pub required: Vec<u32>,
    pub missing: Vec<u32>,
    /// Champions whose published lists contain this route, excluding this hero.
    pub champions: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    pub candidate: u32,
    pub patch: String,
    pub date: String,
    pub routes: Vec<RouteEvidence>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reference {
    pub heading: String,
    pub summary: String,
    pub secondary: String,
    pub ids: Vec<u32>,
    pub partners: String,
    pub detail: String,
}

/// The caller decides whether a hero-specific record already takes precedence.
/// No row returned here may be supplied to contextual_advice::Candidate.
pub fn evaluate(
    index: Option<&Index>,
    champion: u32,
    dataset: &str,
    patch: &str,
    owned: &[(u32, String)],
    candidate: u32,
) -> Option<Evidence> {
    let index = index?;
    let current_patch = fuze_data::recognition::patch(patch)?;
    if champion == 0
        || candidate == 0
        || fuze_contract::skill_choice::get(candidate).is_some()
        || dataset.is_empty()
        || index.dataset != dataset
        || fuze_data::recognition::patch(&index.patch) != Some(current_patch)
        || fuze_data::recognition::patch(dataset.rsplit('@').next().unwrap_or(dataset))
            != Some(current_patch)
        || owned.iter().any(|(id, _)| *id == candidate)
    {
        return None;
    }
    let mut routes = Vec::new();
    for row in index.rows.iter().filter(|r| r.members.contains(&candidate)) {
        let mut members = row.members.clone();
        members.sort_unstable();
        if !(2..=4).contains(&members.len())
            || members.iter().any(|id| *id == 0 || *id > 100000)
            || members.windows(2).any(|ids| ids[0] == ids[1])
        {
            continue;
        }
        let mut champions: Vec<_> = row
            .champions
            .iter()
            .copied()
            .filter(|id| *id > 0 && *id <= 10000 && *id != champion)
            .collect();
        champions.sort_unstable();
        champions.dedup();
        if champions.is_empty() {
            continue;
        }
        let required: Vec<_> = members
            .iter()
            .copied()
            .filter(|id| *id != candidate)
            .collect();
        let missing = required
            .iter()
            .copied()
            .filter(|id| !owned.iter().any(|(held, _)| held == id))
            .collect();
        routes.push(RouteEvidence {
            members,
            required,
            missing,
            champions,
        });
    }
    // Display the closest routes first; the number of reporting heroes is not a
    // performance score and never breaks a tie. The provider lists are selective.
    routes.sort_by(|a, b| {
        a.missing
            .len()
            .cmp(&b.missing.len())
            .then(a.members.cmp(&b.members))
    });
    if routes.is_empty() {
        return None;
    }
    Some(Evidence {
        candidate,
        patch: index.patch.clone(),
        date: index.date.clone(),
        routes,
    })
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

fn targets(row: &RouteEvidence) -> &[u32] {
    if row.missing.is_empty() {
        &row.required
    } else {
        &row.missing
    }
}

/// The compact route label shares one 214px line with its source prefix. Keep
/// the entire target label within ten characters, including a multi-target suffix.
fn compact_targets(ids: &[u32], catalog: &HashMap<u32, String>) -> String {
    let Some(first) = ids.first() else {
        return String::new();
    };
    let suffix = if ids.len() > 1 {
        format!("等{}项", ids.len())
    } else {
        String::new()
    };
    let limit = 10usize.saturating_sub(suffix.chars().count());
    let name = names(&[*first], catalog);
    let compact = if name.chars().count() > limit {
        format!(
            "{}…",
            name.chars()
                .take(limit.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        name
    };
    format!("{compact}{suffix}")
}

pub fn present(evidence: &Evidence, catalog: &HashMap<u32, String>) -> Reference {
    let Some(first) = evidence.routes.first() else {
        return Reference::default();
    };
    let ids = targets(first).to_vec();
    let partners = names(&ids, catalog);
    let secondary = evidence.routes.get(1).map_or_else(String::new, |row| {
        format!("另条参考 · {}", compact_targets(targets(row), catalog))
    });
    let rows = evidence
        .routes
        .iter()
        .map(|row| {
            format!(
                "{} · {}\n来源英雄 ID：{}",
                names(&row.members, catalog),
                if row.missing.is_empty() {
                    "已有前置，仍待本英雄验证".into()
                } else {
                    format!("尚缺 {}", names(&row.missing, catalog))
                },
                row.champions
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join("、")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Reference {
        heading: format!("跨英雄参考 · {}", compact_targets(&ids, catalog)),
        summary: if first.missing.is_empty() {
            "待本英雄验证 · 已有前置".into()
        } else {
            format!("待本英雄验证 · 尚缺{}项", first.missing.len())
        },
        secondary,
        ids,
        partners,
        detail: format!(
            "跨英雄参考 · ARAMKit {} · {}\n待本英雄验证：仅记录其他英雄公开榜单中的组合成员，不提供本英雄胜率，不参与本组选择。\n这些榜单并不完整；缺少记录不表示不搭配，也不预测后续强化会出现。\n{}",
            evidence.patch, evidence.date, rows
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuze_data::combination_routes::Route;

    fn index() -> Index {
        Index {
            dataset: "zh-CN@16.22.1".into(),
            patch: "16.22".into(),
            date: "2026-11-12".into(),
            data_path: "data/16.22-20261112-bfae7c1ce948".into(),
            champions: 3,
            rows: vec![
                Route {
                    members: vec![1004, 3086],
                    champions: vec![145, 17],
                },
                Route {
                    members: vec![1004, 1029],
                    champions: vec![145],
                },
                Route {
                    members: vec![1004, 1038],
                    champions: vec![157],
                },
            ],
        }
    }

    #[test]
    fn only_other_champions_and_matching_dataset_supply_routes() {
        let index = index();
        let evaluate_for = |dataset, patch, owned: &[(u32, String)]| {
            evaluate(Some(&index), 145, dataset, patch, owned, 1004)
        };
        let e = evaluate_for(&index.dataset, "26.22.1", &[]).unwrap();
        assert_eq!(e.routes.len(), 2);
        assert!(e.routes.iter().all(|r| !r.champions.contains(&145)));
        assert!(!e.routes.iter().any(|r| r.members.contains(&1029)));
        assert!(evaluate_for("zh-CN@16.22.2", "16.22", &[]).is_none());
        assert!(evaluate_for(&index.dataset, "16.21", &[]).is_none());
        assert!(evaluate_for(&index.dataset, "unknown", &[]).is_none());
        assert!(evaluate_for(&index.dataset, "16.22", &[(1004, "已持有".into())]).is_none());
        assert!(evaluate(None, 145, &index.dataset, "16.22", &[], 1004).is_none());
    }

    #[test]
    fn confirmed_prerequisites_keep_cross_hero_label_and_visible_second_target() {
        let index = index();
        let names = HashMap::from([
            (1004, "珠光护手".into()),
            (1038, "回归基本功".into()),
            (3086, "蛋白粉奶昔".into()),
        ]);
        let e = evaluate(
            Some(&index),
            145,
            &index.dataset,
            "16.22",
            &[(3086, "蛋白粉奶昔".into())],
            1004,
        )
        .unwrap();
        assert!(e.routes[0].missing.is_empty());
        let view = present(&e, &names);
        assert_eq!(view.heading, "跨英雄参考 · 蛋白粉奶昔");
        assert!(view.summary.contains("待本英雄验证"));
        assert!(view.secondary.contains("回归基本功"));
        assert!(view.detail.contains("来源英雄 ID：17"));
        assert!(!view.detail.contains("145") && !view.detail.contains('%'));
        assert_eq!(view.ids, [3086]);
        let fallback = present(&e, &HashMap::new());
        assert!(fallback.heading.contains("强化 #3086"));
    }

    #[test]
    fn compact_routes_bound_three_long_target_names_and_keep_full_details() {
        let mut index = index();
        index.rows = vec![
            Route {
                members: vec![1004, 2001, 2002, 2003],
                champions: vec![17],
            },
            Route {
                members: vec![1004, 2004, 2005, 2006],
                champions: vec![157],
            },
        ];
        let first = "这是用于检验三项目标展示的完整长名称";
        let second = "第二条路线也有需要完整保留的很长名称";
        let names = HashMap::from([
            (2001, first.into()),
            (2002, "第二伙伴".into()),
            (2003, "第三伙伴".into()),
            (2004, second.into()),
            (2005, "第五伙伴".into()),
            (2006, "第六伙伴".into()),
        ]);
        let e = evaluate(Some(&index), 145, &index.dataset, "16.22", &[], 1004).unwrap();
        let view = present(&e, &names);
        for (line, prefix) in [
            (&view.heading, "跨英雄参考 · "),
            (&view.secondary, "另条参考 · "),
        ] {
            let label = line.strip_prefix(prefix).unwrap();
            assert!(label.chars().count() <= 10);
            assert!(label.ends_with("等3项") && label.contains('…'));
            assert!(!line.contains('\n'));
        }
        assert!(view.summary.chars().count() <= 18 && !view.summary.contains('\n'));
        assert!(view.partners.contains(first) && view.partners.contains("第三伙伴"));
        assert!(view.detail.contains(first) && view.detail.contains(second));
        assert_eq!(view.ids, [2001, 2002, 2003]);
        let single = evaluate(
            Some(&index),
            145,
            &index.dataset,
            "16.22",
            &[(2002, "已持有二".into()), (2003, "已持有三".into())],
            1004,
        )
        .unwrap();
        assert_eq!(
            present(&single, &names)
                .heading
                .strip_prefix("跨英雄参考 · ")
                .unwrap()
                .chars()
                .count(),
            10
        );
    }

    #[test]
    fn reporting_hero_counts_never_prioritize_routes_with_equal_missing_members() {
        let mut index = index();
        index.rows = vec![
            Route {
                members: vec![1004, 3086],
                champions: vec![1, 2, 3, 4, 5],
            },
            Route {
                members: vec![1004, 1038],
                champions: vec![17],
            },
            Route {
                members: vec![1004, 1029, 2001],
                champions: vec![157],
            },
        ];
        let owned = vec![(1038, "已持有一".into()), (3086, "已持有二".into())];
        let e = evaluate(Some(&index), 145, &index.dataset, "16.22", &owned, 1004).unwrap();
        assert_eq!(e.routes[0].members, [1004, 1038]);
        assert_eq!(e.routes[1].members, [1004, 3086]);
        assert!(e.routes[0].missing.is_empty() && e.routes[1].missing.is_empty());
        assert_eq!(e.routes[2].missing, [1029, 2001]);
    }
}
