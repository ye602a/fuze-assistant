//! Fixed title-region evidence only. Never infer a claim or popup absence from OCR misses.
use fuze_contract::{wire::TitleKind, Slot};

pub fn shard_title(text: &str, confidence: f32) -> bool {
    if !confidence.is_finite() || confidence < 0.9 || text.chars().count() > 80 {
        return false;
    }
    let text: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let base = text
        .strip_suffix("属性碎片")
        .or_else(|| text.strip_suffix("屬性碎片"))
        .or_else(|| text.strip_suffix("碎片"))
        .or_else(|| text.strip_suffix("shard"));
    matches!(
        base,
        Some(
            "攻击力"
                | "攻擊力"
                | "法术强度"
                | "法術強度"
                | "生命值"
                | "护甲"
                | "護甲"
                | "魔法抗性"
                | "攻击速度"
                | "攻擊速度"
                | "技能急速"
                | "移动速度"
                | "移動速度"
                | "暴击几率"
                | "暴擊機率"
                | "适应之力"
                | "適應之力"
                | "attackdamage"
                | "abilitypower"
                | "health"
                | "armor"
                | "magicresist"
                | "attackspeed"
                | "abilityhaste"
                | "movementspeed"
                | "criticalstrikechance"
                | "adaptiveforce"
        )
    )
}

pub fn classify(slots: &[Slot; 3], shards: [bool; 3]) -> TitleKind {
    if slots.iter().any(fuze_contract::skill_choice::is_slot) {
        // Never merge a skill screen with an ordinary candidate or anvil result.
        if !shards.into_iter().any(|v| v)
            && slots
                .iter()
                .all(|s| *s == Slot::Unreadable || fuze_contract::skill_choice::is_slot(s))
        {
            TitleKind::SkillChoice
        } else {
            TitleKind::Unknown
        }
    } else if shards.into_iter().all(|v| v) && slots.iter().all(|s| *s == Slot::Unreadable) {
        TitleKind::StatShard
    } else if slots.iter().any(|s| matches!(s, Slot::Known { .. })) {
        TitleKind::Candidate
    } else {
        TitleKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pentakill_choices_never_mix_with_ordinary_augments_or_shards() {
        let mut slots = ["重型", "力量", "鞭挞"]
            .map(|name| fuze_contract::skill_choice::read(name, 0.95).unwrap());
        assert_eq!(classify(&slots, [false; 3]), TitleKind::SkillChoice);
        slots[0] = fuze_contract::skill_choice::read("厄运", 0.95).unwrap();
        assert_eq!(classify(&slots, [false; 3]), TitleKind::SkillChoice);
        slots[1] = Slot::Unreadable;
        assert_eq!(classify(&slots, [false; 3]), TitleKind::SkillChoice);
        assert_eq!(classify(&slots, [false, true, false]), TitleKind::Unknown);
        slots[1] = Slot::Known {
            id: 3051,
            name: "蓝色屏障".into(),
            confidence: 1.,
        };
        assert_eq!(classify(&slots, [false; 3]), TitleKind::Unknown);
    }
    #[test]
    fn exact_three_shards_only_and_no_claim_inference() {
        for text in [
            "攻击力碎片",
            "法術強度屬性碎片",
            "Attack Damage Shard",
            "生命值属性碎片",
        ] {
            assert!(shard_title(text, 0.95));
        }
        for text in [
            "攻击力",
            "攻击力碎片!",
            "未知碎片",
            "购买属性锻造器",
            "health shard bonus",
        ] {
            assert!(!shard_title(text, 1.0));
        }
        assert!(!shard_title("攻击力碎片", 0.8));
        assert!(!shard_title("攻击力碎片", f32::NAN));
        let mut slots = std::array::from_fn(|_| Slot::Unreadable);
        assert_eq!(classify(&slots, [true; 3]), TitleKind::StatShard);
        assert_eq!(classify(&slots, [true, false, true]), TitleKind::Unknown);
        slots[1] = Slot::Known {
            id: 1,
            name: "fixture".into(),
            confidence: 1.0,
        };
        assert_eq!(classify(&slots, [true; 3]), TitleKind::Candidate);
    }
}
