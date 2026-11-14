//! Small, version-bound mechanism notices; no scores or statistical comparisons.
//!
//! Reviewed source: ARAMKit 16.22, manifest dataDate 2026-11-12.
//! https://data.aramkit.cn/resources/16.22-8a495cc12b20/zh-CN/resources/augments.json
//! SHA-256: 6b09a7bb91184e119e10be349bd54907b2eda0211492598872506b7f97a1a118
//! These notices infer relationships from explicit resource descriptions. They do
//! not establish offer-pool rules, special champion behavior, or outcome changes.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub heading: String,
    pub summary: String,
    pub detail: String,
    pub related: Vec<u32>,
}

const PANDORAS_BOX: u32 = 12317;
const BACK_TO_BASICS: u32 = 1004;
const ULTIMATE_TRIGGERS: [(u32, &str, &str); 3] = [
    (
        1011,
        "你摸不到",
        "施放你的终极技能会使你进入持续2秒的免疫伤害状态。",
    ),
    (1088, "终极刷新", "在施放终极技能后刷新你的终极技能。"),
    (
        1112,
        "终极不可阻挡",
        "使用终极技能后，获得持续3秒的控制免疫。",
    ),
];

/// `owned` must contain only confirmed, still-valid selections, not offered cards.
/// The caller supplies the reviewed resource patch, exactly `16.22`; unknown or
/// different versions cannot inherit these hand-reviewed rules.
pub fn notice(candidate: u32, owned: &[(u32, String)], patch: &str) -> Option<Notice> {
    if patch != "16.22" || candidate == 0 || owned.iter().any(|(id, _)| *id == candidate) {
        return None;
    }
    let mut held: Vec<_> = owned
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| *id > 0)
        .collect();
    held.sort_unstable();
    held.dedup();
    if candidate == PANDORAS_BOX {
        return Some(Notice {
            heading: "转换提示".into(),
            summary: "现有强化将随机变为棱彩阶".into(),
            detail: "ARAMKit 16.22 说明：将你的所有强化符文变为随机的棱彩阶强化符文。确认选择后，原有组合前置不再确定；随机新结果未知。此提示不预测转换结果，也不调整推荐分。".into(),
            related: held,
        });
    }
    let triggers: Vec<_> = ULTIMATE_TRIGGERS
        .iter()
        .filter(|(id, _, _)| {
            if candidate == BACK_TO_BASICS {
                held.contains(id)
            } else {
                candidate == *id && held.contains(&BACK_TO_BASICS)
            }
        })
        .collect();
    if triggers.is_empty() {
        return None;
    }
    let descriptions = triggers
        .iter()
        .map(|(_, name, description)| format!("{name}：{description}"))
        .collect::<Vec<_>>()
        .join("\n");
    Some(Notice {
        heading: "机制冲突提示".into(),
        summary: "终极技能触发条件冲突（说明推论）".into(),
        detail: format!(
            "依据 ARAMKit 16.22 资源说明推论：回归基本功会封印终极技能，而以下效果依赖施放或使用终极技能。\n{descriptions}\n这是触发条件冲突提示，不作负分或胜率比较；未据此判断游戏是否允许同时获得，也未确认自动施放等特殊机制的交互。"
        ),
        related: if candidate == BACK_TO_BASICS {
            triggers.iter().map(|(id, _, _)| *id).collect()
        } else {
            vec![BACK_TO_BASICS]
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transformation_notice_does_not_require_or_invent_known_results() {
        let empty = notice(PANDORAS_BOX, &[], "16.22").unwrap();
        assert_eq!(empty.heading, "转换提示");
        assert!(empty.related.is_empty());
        assert!(empty.detail.contains("随机新结果未知"));
        let owned = vec![
            (3086, "小蜜蜂！！！".into()),
            (0, "未知".into()),
            (1004, "回归基本功".into()),
            (3086, "重复记录".into()),
        ];
        let before = owned.clone();
        let notice = notice(PANDORAS_BOX, &owned, "16.22").unwrap();
        assert_eq!(notice.related, [1004, 3086]);
        assert_eq!(owned, before);
        assert!(!notice.related.contains(&PANDORAS_BOX));
    }

    #[test]
    fn ultimate_trigger_conflicts_require_confirmed_counterpart_in_both_directions() {
        for (id, name, _) in ULTIMATE_TRIGGERS {
            assert!(notice(id, &[], "16.22").is_none());
            assert!(notice(BACK_TO_BASICS, &[], "16.22").is_none());
            let forward = notice(id, &[(BACK_TO_BASICS, String::new())], "16.22").unwrap();
            assert_eq!(forward.related, [BACK_TO_BASICS]);
            assert!(forward.detail.contains(name));
            let reverse = notice(BACK_TO_BASICS, &[(id, name.into())], "16.22").unwrap();
            assert_eq!(reverse.related, [id]);
            assert!(reverse.detail.contains("说明推论"));
            assert!(reverse.detail.contains("不作负分或胜率比较"));
        }
        let owned = [
            (1112, "终极不可阻挡".into()),
            (1011, "你摸不到".into()),
            (0, "终极刷新".into()),
            (1011, "重复".into()),
        ];
        let combined = notice(BACK_TO_BASICS, &owned, "16.22").unwrap();
        assert_eq!(combined.related, [1011, 1112]);
        assert_eq!(combined.detail.matches("你摸不到：").count(), 1);
        assert!(!combined.detail.contains("终极刷新："));
    }

    #[test]
    fn unknowns_names_versions_and_already_owned_candidates_do_not_create_rules() {
        let basic = [(BACK_TO_BASICS, "回归基本功".into())];
        for candidate in [0, 99999, 1421, 1214, 2078, 2129, 2141, 1029, 3086] {
            assert!(notice(candidate, &basic, "16.22").is_none());
        }
        // A name or an unrelated ID cannot substitute for a confirmed numeric identity.
        assert!(notice(1011, &[(0, "回归基本功".into())], "16.22").is_none());
        assert!(notice(1011, &[(99999, "回归基本功".into())], "16.22").is_none());
        assert!(notice(BACK_TO_BASICS, &basic, "16.22").is_none());
        assert!(notice(PANDORAS_BOX, &[(PANDORAS_BOX, String::new())], "16.22").is_none());
        for patch in ["", "16.21", "16.23", "26.22.1", "16.22.1", "unknown"] {
            assert!(notice(1011, &basic, patch).is_none());
            assert!(notice(PANDORAS_BOX, &[], patch).is_none());
        }
    }
}
