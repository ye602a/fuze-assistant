//! Display-only frame after the existing retention/classification gate. No claim inference.
use fuze_contract::Snapshot;
use fuze_data::champion_detail::Augment;
use std::{collections::HashMap, sync::Arc, time::Instant};
pub fn skill_cards() -> Vec<Augment> {
    fuze_contract::skill_choice::CHOICES.iter().map(|choice| Augment {
        evidence: Default::default(),
        id: choice.id,
        name: format!("{} · {}", choice.key, choice.title),
        icon: fuze_contract::skill_choice::ICON_URL.into(),
        rarity: None,
        tier: None,
        win: None,
        pick: None,
        description: format!("五杀摇滚 · 强化 {} 技能\n该技能获得 20 技能急速、10% 额外伤害，以及 10% 治疗和护盾效果。\n开场强化基础技能；18 级可强化另一个技能，包括终极技能。\n暂无技能选择统计，请结合本局玩法选择。", choice.key),
        provenance: String::new(),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuze_contract::{skill_choice, wire::TitleKind, ActualStage, Slot};
    #[test]
    fn background_rank_refresh_keeps_slots_claim_stage_and_retention_deadline() {
        let mut card = skill_cards()[0].clone();
        card.id = 3051;
        card.win = Some(0.55);
        card.pick = Some(0.1);
        let mut publisher = Publisher::new(vec![card.clone()], HashMap::new());
        let deadline = Some(Instant::now() + std::time::Duration::from_secs(2));
        publisher.publish(
            Snapshot {
                schema_version: 1,
                generation: 1,
                sequence: 7,
                dataset_version: "test".into(),
                selection_confirmed: false,
                actual_stage: ActualStage::Unknown,
                slots: [
                    Slot::Known {
                        id: 3051,
                        name: "蓝色屏障".into(),
                        confidence: 1.,
                    },
                    Slot::Unreadable,
                    Slot::Unreadable,
                ],
            },
            true,
            true,
            deadline,
        );
        let before = publisher.current().unwrap();
        card.pick = Some(0.2);
        publisher.supplement(vec![card], HashMap::from([(3051, 0.8), (3170, 1.)]));
        let after = publisher.current().unwrap();
        assert_eq!(before.snapshot, after.snapshot);
        assert_eq!(after.deadline, deadline);
        assert!(after.retained);
        assert_eq!(after.cards[0].as_ref().unwrap().win, Some(0.55));
        assert_eq!(after.scores, [Some(0.8), None, None]);
        assert_eq!(after.context, before.context);
        assert_eq!(after.selection_cycle, before.selection_cycle);
        publisher.clear();
        publisher.supplement(vec![], HashMap::new());
        assert!(publisher.current().is_none());
    }
    #[test]
    fn skill_to_ordinary_and_reroll_frames_keep_slots_without_statistics_leaks() {
        let mut polluted = skill_cards();
        polluted[0].win = Some(0.99);
        polluted[0].tier = Some(1);
        let mut ordinary = polluted[0].clone();
        ordinary.id = 3051;
        ordinary.name = "蓝色屏障".into();
        ordinary.rarity = Some(0);
        ordinary.win = None;
        ordinary.tier = None;
        polluted.push(ordinary.clone());
        ordinary.id = 3060;
        ordinary.name = "巫妖之脑".into();
        ordinary.rarity = Some(1);
        polluted.push(ordinary);
        let mut publisher = Publisher::new(polluted, HashMap::from([(3170, 1.)]));
        let mut retention = crate::retention::Retention::default();
        let skills = ["重型", "力量", "鞭挞"].map(|n| skill_choice::read(n, 1.).unwrap());
        let snapshot = |slots, sequence| Snapshot {
            schema_version: 1,
            generation: 1,
            sequence,
            dataset_version: "zh-CN@16.20.4-augments16.21".into(),
            selection_confirmed: false,
            actual_stage: ActualStage::Unknown,
            slots,
        };
        let display = retention.apply(&skills, TitleKind::SkillChoice, 0);
        publisher.publish(
            snapshot(display.slots, 1),
            display.visible,
            display.retained,
            None,
        );
        let frame = publisher.current().unwrap();
        assert!(frame.scores.iter().all(Option::is_none));
        for card in frame.cards.iter().flatten() {
            assert!(card.win.is_none() && card.pick.is_none() && card.tier.is_none());
            assert!(card.description.contains("20 技能急速"));
        }
        let regular = |id| Slot::Known {
            id,
            name: "ordinary".into(),
            confidence: 1.,
        };
        let cards = [regular(3051), regular(3060), Slot::Unreadable];
        let display = retention.apply(&cards, TitleKind::Candidate, 250);
        assert!(!display.retained);
        publisher.publish(snapshot(display.slots, 2), display.visible, false, None);
        let frame = publisher.current().unwrap();
        assert_eq!(frame.cards[0].as_ref().unwrap().rarity, Some(0));
        assert!(frame.cards[2].is_none());
        // COMMAND LINE upgrade replaces the old card/rarity in the same slot immediately.
        let rerolled = [regular(3060), regular(3051), Slot::Unreadable];
        let display = retention.apply(&rerolled, TitleKind::Candidate, 500);
        assert!(!display.retained);
        publisher.publish(snapshot(display.slots, 3), display.visible, false, None);
        let frame = publisher.current().unwrap();
        assert_eq!(frame.cards[0].as_ref().unwrap().rarity, Some(1));
        assert!(frame.cards[0].as_ref().unwrap().win.is_none());
        assert_eq!(frame.snapshot.actual_stage, ActualStage::Unknown);
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// Process-local pipeline identity prevents a restarted game accepting an old generation.
    pub context: u64,
    pub selection_cycle: u64,
    pub snapshot: Snapshot,
    pub retained: bool,
    pub deadline: Option<Instant>,
    pub cards: [Option<Arc<Augment>>; 3],
    pub scores: [Option<f64>; 3],
}
pub struct Publisher {
    context: u64,
    selection_cycle: u64,
    augments: HashMap<u32, Arc<Augment>>,
    scores: HashMap<u32, f64>,
    current: Option<Arc<Frame>>,
}
impl Publisher {
    /// A valid OCR match may have no statistics for this champion. Keep its global
    /// artwork/description while leaving rates and scores absent, rather than losing the card.
    pub fn with_catalog(
        augments: Vec<Augment>,
        mut catalog: Vec<Augment>,
        scores: HashMap<u32, f64>,
    ) -> Self {
        for card in &mut catalog {
            card.win = None;
            card.pick = None;
            card.tier = None;
            card.provenance.clear();
            card.evidence = Default::default();
        }
        catalog.extend(augments);
        Self::new(catalog, scores)
    }
    pub fn new(mut augments: Vec<Augment>, mut scores: HashMap<u32, f64>) -> Self {
        // Skill choices have no ordinary augment statistics, even if an upstream row reuses the ID.
        augments.extend(skill_cards());
        scores.retain(|id, _| fuze_contract::skill_choice::get(*id).is_none());
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            context: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            selection_cycle: 0,
            augments: augments.into_iter().map(|a| (a.id, Arc::new(a))).collect(),
            scores,
            current: None,
        }
    }
    pub fn clear(&mut self) {
        self.current = None;
    }
    pub fn supplement(&mut self, augments: Vec<Augment>, scores: HashMap<u32, f64>) {
        self.augments.extend(
            augments
                .into_iter()
                .filter(|a| fuze_contract::skill_choice::get(a.id).is_none())
                .map(|a| (a.id, Arc::new(a))),
        );
        self.scores = scores
            .into_iter()
            .filter(|(id, _)| fuze_contract::skill_choice::get(*id).is_none())
            .collect();
        if let Some(frame) = self.current.clone() {
            self.publish(frame.snapshot.clone(), true, frame.retained, frame.deadline);
        }
    }
    pub fn end_selection(&mut self) {
        self.clear();
        self.selection_cycle = self.selection_cycle.saturating_add(1);
    }
    pub fn current(&self) -> Option<Arc<Frame>> {
        self.current.clone()
    }
    pub fn publish(
        &mut self,
        snapshot: Snapshot,
        visible: bool,
        retained: bool,
        deadline: Option<Instant>,
    ) {
        self.current = visible.then(|| {
            let ids = snapshot.slots.each_ref().map(|s| match s {
                fuze_contract::Slot::Known { id, .. } => Some(*id),
                _ => None,
            });
            Arc::new(Frame {
                context: self.context,
                selection_cycle: self.selection_cycle,
                cards: ids.map(|id| id.and_then(|id| self.augments.get(&id).cloned())),
                scores: ids.map(|id| id.and_then(|id| self.scores.get(&id).copied())),
                snapshot,
                retained,
                deadline,
            })
        });
    }
}
