//! v0.6.3 title normalization/aliases and bounded typo tolerance, scoped to ONE title ROI.
//! Canonical public names only; ambiguous equal-quality matches remain unreadable.
use fuze_contract::{wire::TitleRead, Slot};
use std::collections::HashMap;
pub struct Matcher(Vec<(String, Vec<char>, u32)>);
const BLACKLIST: &[&str] = &[
    "攻击", "防御", "生命", "法术", "魔法", "伤害", "护甲", "技能", "冷却", "移速", "暴击", "吸血",
    "穿透", "功能", "能力", "效果", "被动", "主动", "额外", "持续", "提供", "增加", "获得", "造成",
];
pub fn normalize(text: &str) -> Vec<char> {
    text.chars()
        .take(128)
        .filter_map(|c| {
            let c = if ('\u{ff01}'..='\u{ff5e}').contains(&c) {
                char::from_u32(c as u32 - 0xfee0).unwrap_or(c)
            } else {
                c
            };
            (!c.is_whitespace()
                && !"\"'“”‘’`.,，。:：;；!！?？、|｜/\\()[]{}<>《》【】「」『』-_=+~·•".contains(c))
            .then_some(c)
        })
        .collect()
}
// One short row, bounded to 32 characters and an early rejection threshold.
fn distance(a: &[char], b: &[char], limit: usize) -> usize {
    let mut prev = std::array::from_fn::<_, 33, _>(|i| i);
    let mut row = [0usize; 33];
    for (i, x) in a.iter().enumerate() {
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            row[j + 1] = (prev[j] + usize::from(x != y))
                .min(row[j] + 1)
                .min(prev[j + 1] + 1);
        }
        if *row[..=b.len()].iter().min().unwrap() > limit {
            return limit + 1;
        }
        std::mem::swap(&mut row, &mut prev);
    }
    prev[b.len()]
}
impl Matcher {
    pub fn new(catalog: HashMap<String, u32>) -> Self {
        let mut unique = HashMap::<Vec<char>, (String, u32)>::new();
        for (name, id) in catalog {
            let chars = normalize(&name);
            let clean: String = chars.iter().collect();
            if chars.is_empty() || chars.len() > 32 || BLACKLIST.contains(&clean.as_str()) {
                continue;
            }
            let priority = |id: u32| (id >= 1000, id);
            if unique
                .get(&chars)
                .is_none_or(|(_, old)| priority(id) > priority(*old))
            {
                unique.insert(chars, (name, id));
            }
        }
        Self(
            unique
                .into_iter()
                .map(|(chars, (name, id))| (name, chars, id))
                .collect(),
        )
    }
    pub fn read(&self, raw: &str, confidence: f32) -> (Slot, TitleRead) {
        use TitleRead::*;
        let text = normalize(raw);
        if text.is_empty() {
            return (Slot::Unreadable, Empty);
        }
        if !confidence.is_finite() || confidence < 0.75 {
            return (Slot::Unreadable, LowConfidence);
        }
        if text.len() > 40 {
            return (Slot::Unreadable, NoCatalog);
        }
        let mut best: Option<(usize, usize, usize, TitleRead)> = None;
        let mut ambiguous = false;
        for (index, (name, chars, _)) in self.0.iter().enumerate() {
            if text.len() > chars.len() + 8 {
                continue;
            }
            let candidate = if &text == chars {
                Some((0, Exact))
            } else if name == "一板一眼" && text == ['板', '一', '眼'] {
                Some((0, Alias))
            } else if chars.len() >= 3
                && text.len() == chars.len()
                && confidence >= 0.9
                && best.is_none_or(|(d, _, _, _)| d > 0)
            {
                // Keep the legacy tolerance, reject ties instead of guessing by numeric ID.
                let limit = (chars.len() / 3).max(1).min(2);
                Some(distance(&text, chars, limit))
                    .filter(|d| *d <= limit)
                    .map(|d| (d, Fuzzy))
            } else {
                None
            };
            let Some((d, reason)) = candidate else {
                continue;
            };
            let rank = (d, usize::MAX - chars.len());
            match best {
                Some((bd, bl, _, _)) if rank > (bd, bl) => (),
                Some((bd, bl, _, _)) if rank == (bd, bl) => ambiguous = true,
                _ => {
                    best = Some((d, usize::MAX - chars.len(), index, reason));
                    ambiguous = false;
                }
            }
        }
        if ambiguous {
            return (Slot::Unreadable, Ambiguous);
        }
        if let Some((_, _, index, reason)) = best {
            let (name, _, id) = &self.0[index];
            (
                Slot::Known {
                    id: *id,
                    name: name.clone(),
                    confidence: if reason == Fuzzy {
                        confidence.min(0.8)
                    } else {
                        confidence
                    },
                },
                reason,
            )
        } else {
            (Slot::Unreadable, NoCatalog)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shop_item_or_description_cannot_match_a_substring() {
        let m = matcher(&["海克斯科技", "科技核心", "量子计算"]);
        for text in ["海克斯科技核心", "购买量子计算", "量子计算装备"] {
            assert_eq!(m.read(text, 0.99).1, TitleRead::NoCatalog);
        }
        assert_eq!(m.read("量子计算", 0.99).1, TitleRead::Exact);
    }
    fn matcher(names: &[&str]) -> Matcher {
        Matcher::new(
            names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.to_string(), i as u32 + 1000))
                .collect(),
        )
    }
    #[test]
    fn legacy_punctuation_alias_and_single_typo_return_canonical_names() {
        let m = matcher(&["一板一眼", "量子计算", "超级：闪电"]);
        assert_eq!(m.read("超级闪电！", 0.99).1, TitleRead::Exact);
        assert_eq!(m.read("板一眼", 0.99).1, TitleRead::Alias);
        assert_eq!(m.read("量子计笄", 0.99).1, TitleRead::Fuzzy);
        assert!(
            matches!(m.read("量子计笄", 0.99).0, Slot::Known { name, .. } if name == "量子计算")
        );
        assert_eq!(m.read("量子计笄", 0.8).1, TitleRead::NoCatalog);
    }
    #[test]
    fn ambiguity_short_generic_text_and_long_description_are_not_guessed() {
        let m = matcher(&["量子计算", "量子计数", "攻击", "穿针"]);
        assert_eq!(m.read("量子计笄", 0.99).1, TitleRead::Ambiguous);
        assert_eq!(m.read("穿计", 0.99).1, TitleRead::NoCatalog);
        assert_eq!(m.read("攻击", 0.99).1, TitleRead::NoCatalog);
        assert_eq!(
            m.read(
                "这是一段非常长的装备描述文字量子计算请勿把描述当作标题",
                0.99
            )
            .1,
            TitleRead::NoCatalog
        );
        assert_eq!(m.read("", 1.).1, TitleRead::Empty);
        assert_eq!(m.read("量子计算", f32::NAN).1, TitleRead::LowConfidence);
    }
}
