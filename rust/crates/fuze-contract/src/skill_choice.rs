//! 16.21 KIWI Pentakill ability choices, separate from the ordinary augment pool.
//! Source: Riot client cherry-augments.json and zh_cn lol.stringtable.json, via
//! raw.communitydragon.org/16.21. The generic display entry 3176 is not a choice.
use crate::Slot;
pub const ICON_URL: &str = "https://raw.communitydragon.org/16.21/plugins/rcp-be-lol-game-data/global/default/assets/ux/kiwi/augments/icons/pentakill_small.png";

pub struct Choice {
    pub id: u32,
    pub title: &'static str,
    pub key: &'static str,
}
pub const CHOICES: [Choice; 4] = [
    Choice {
        id: 3170,
        title: "重型",
        key: "Q",
    },
    Choice {
        id: 3171,
        title: "力量",
        key: "W",
    },
    Choice {
        id: 3172,
        title: "鞭挞",
        key: "E",
    },
    Choice {
        id: 3173,
        title: "厄运",
        key: "R",
    },
];
pub fn get(id: u32) -> Option<&'static Choice> {
    CHOICES.iter().find(|choice| choice.id == id)
}
pub fn is_slot(slot: &Slot) -> bool {
    matches!(slot, Slot::Known { id, .. } if get(*id).is_some())
}
pub fn read(title: &str, confidence: f32) -> Option<Slot> {
    if !confidence.is_finite() || confidence < 0.9 || title.len() > 80 {
        return None;
    }
    let title: String = title.chars().filter(|c| !c.is_whitespace()).collect();
    CHOICES
        .iter()
        .find(|choice| choice.title == title)
        .map(|choice| Slot::Known {
            id: choice.id,
            name: format!("{} · {}", choice.key, choice.title),
            confidence,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_high_confidence_ability_titles_are_choices() {
        for choice in &CHOICES {
            assert!(
                matches!(read(choice.title, 0.95), Some(Slot::Known { id, .. }) if id == choice.id)
            );
        }
        for title in [
            "购买力量",
            "力量碎片",
            "海克斯科技核心",
            "重形",
            "GoH_PentakillAug_Generic_BlankName",
        ] {
            assert!(read(title, 1.).is_none());
        }
        assert!(read("力量", 0.89).is_none());
        assert!(read("力量", f32::NAN).is_none());
        assert!(get(3176).is_none());
    }
}
