//! Comparable historical evidence for the displayed offer. Never infers a claim stage.
use fuze_data::{
    contextual_advice::{self, Candidate, Choice, Decision, Reason, StagePattern},
    decision_evidence::{self, AugmentHistory, HeroReference, HistoryContext, Rate},
};
use fuze_lcu::reconnect::State;
use fuze_runtime::session::Phase;
use fuze_worker_host::{
    desktop::Status,
    overlay_frame::Frame,
    stage_reference::{self, Access, Request, Service, Snapshot, Store},
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryDisplay {
    pub relation: String,
    pub delta: String,
    pub samples: String,
    pub interval: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub decisions: [Decision; 3],
    pub reasons: [String; 3],
    pub stage_summary: [String; 3],
    pub history: [HistoryDisplay; 3],
    pub mechanisms: [Option<fuze_data::mechanism_reference::Notice>; 3],
    pub combos: [fuze_worker_host::augment_combos::Reference; 3],
    pub current_combos: [bool; 3],
    pub combo_alternatives: [usize; 3],
    pub values: [[String; 4]; 3],
    pub samples: [[String; 4]; 3],
    pub details: [String; 3],
    pub source: String,
}
impl Default for Comparison {
    fn default() -> Self {
        Self {
            decisions: Default::default(),
            reasons: std::array::from_fn(|_| "尚缺可比较的历史证据".into()),
            stage_summary: std::array::from_fn(|_| "阶段资料未就绪".into()),
            history: std::array::from_fn(|_| HistoryDisplay {
                relation: "较英雄均值".into(),
                delta: "不可比较".into(),
                samples: "样本量未提供 · 英雄均值未就绪".into(),
                interval: "约95%区间：样本量未提供".into(),
            }),
            mechanisms: Default::default(),
            combos: Default::default(),
            current_combos: Default::default(),
            combo_alternatives: Default::default(),
            values: std::array::from_fn(|_| std::array::from_fn(|_| "—".into())),
            samples: std::array::from_fn(|_| std::array::from_fn(|_| "—".into())),
            details: Default::default(),
            source: "阶段参考暂无缓存 · 选人及加载时准备，局内不联网".into(),
        }
    }
}
impl Comparison {
    pub fn decision(&self, index: usize, id: Option<u32>) -> Option<&Decision> {
        self.decisions
            .get(index)
            .filter(|d| id.is_some() && d.candidate_id == id)
    }
    pub fn preferred(&self, index: usize, id: Option<u32>) -> bool {
        self.decision(index, id)
            .is_some_and(|d| d.choice == Choice::Preferred)
    }
}
pub fn compare(
    snapshot: Option<&Snapshot>,
    champion: u32,
    ids: [Option<u32>; 3],
    dataset: &str,
    at: u64,
) -> Comparison {
    let mut view = Comparison::default();
    let Some(s) = snapshot.filter(|s| s.usable(champion, at)) else {
        if snapshot.is_some_and(|s| s.champion == champion && s.fetched <= at) {
            view.source = "阶段缓存已过期 · 暂不参与情境判断".into();
        }
        return view;
    };
    view.source = format!(
        "阶段参考 · ARAMKit {} · {} · 地区未确认{}{}",
        s.patch,
        s.date,
        if s.patch_matches(dataset) {
            ""
        } else {
            " · 与本地补丁不同"
        },
        if s.stale(at) {
            " · 缓存待刷新"
        } else {
            ""
        }
    );
    for (i, id) in ids.into_iter().enumerate() {
        let Some(id) = id else { continue };
        let mut details = Vec::new();
        for j in 0..4 {
            let row = s.rows[j + 1].iter().find(|r| r.id == id);
            let value = match row {
                Some(r) => format!("{:.1}%", r.win * 100.),
                None => "—".into(),
            };
            view.samples[i][j] = row.map_or_else(
                || "—".into(),
                |r| format!("{}{}", r.samples, if r.samples < 200 { "*" } else { "" }),
            );
            details.push(match row {
                Some(r) if r.samples >= 200 => {
                    format!("第 {} 次：{} · {} 场", j + 1, value, r.samples)
                }
                Some(r) => format!("第 {} 次：{} · {} 场 · 少样本", j + 1, value, r.samples),
                None => format!("第 {} 次：暂无统计", j + 1),
            });
            view.values[i][j] = value;
        }
        view.details[i] = format!(
            "\n\n{}\n{}\n四次正式选择的历史参考，当前轮次未确认；缺失阶段不算优势证据。",
            view.source,
            details.join("\n")
        );
    }
    view
}

fn historical(
    row: Option<&AugmentHistory>,
    baseline: Option<&HeroReference>,
    fallback: Option<Rate>,
) -> HistoryDisplay {
    let rate = row.and_then(|r| r.rate).or(fallback);
    let comparison = row.and_then(|r| r.comparison.as_ref());
    let delta = comparison.map_or_else(
        || "不可比较".into(),
        |h| format!("{:+.1} pp", h.delta * 100.),
    );
    let relation = comparison
        .map_or("较英雄均值", |h| match h.relation {
            decision_evidence::Relation::Higher => "高于英雄均值",
            decision_evidence::Relation::Near => "接近英雄均值",
            decision_evidence::Relation::Lower => "低于英雄均值",
        })
        .into();
    let samples = rate.and_then(|r| r.games).map_or_else(
        || "样本量未提供".into(),
        |n| format!("n={n}{}", if n < 200 { " · 少样本" } else { "" }),
    );
    let hero = baseline.filter(|_| comparison.is_some()).map_or_else(
        || "均值不可比较".into(),
        |b| format!("英雄 {:.1}%", b.rate.value * 100.),
    );
    let interval = rate
        .and_then(|r| decision_evidence::interval95(r.value, r.games?))
        .map_or_else(
            || "约95%区间：样本量未提供".into(),
            |r| format!("约95%区间 {:.1}–{:.1}%", r.lower * 100., r.upper * 100.),
        );
    HistoryDisplay {
        relation,
        delta,
        samples: format!("{samples} · {hero}"),
        interval,
    }
}

fn short_name(id: u32, names: &HashMap<u32, String>) -> String {
    let name = names.get(&id).filter(|n| !n.trim().is_empty());
    match name {
        Some(name) if name.chars().count() > 12 => {
            format!("{}…", name.chars().take(11).collect::<String>())
        }
        Some(name) => name.clone(),
        None => format!("强化 #{id}"),
    }
}

pub(crate) fn reason_text(decision: &Decision, names: &HashMap<u32, String>) -> String {
    match &decision.reason {
        Reason::OverallRank if decision.choice == Choice::Preferred => {
            "本组三项整体参考中居前\n英雄均值差不作为否决条件".into()
        }
        Reason::OverallRank => "整体历史表现可参考\n暂无明确情境优势".into(),
        Reason::StageAdvantage { other } => format!(
            "四阶段均优于 {}\n同批次、完整阶段证据",
            short_name(*other, names)
        ),
        Reason::StageAlternative { other } => format!(
            "四阶段均弱于 {}\n本轮可优先考虑该卡",
            short_name(*other, names)
        ),
        Reason::ComboAdvantage { other, required } => format!(
            "同{}项前置优于 {}\n依据下方已成立的搭配",
            required.len(),
            short_name(*other, names)
        ),
        Reason::ComboAlternative { other, required } => format!(
            "同{}项前置弱于 {}\n本轮可优先考虑该卡",
            required.len(),
            short_name(*other, names)
        ),
        Reason::FutureOnly => "只有尚未成立的后续路线\n未来是否出现尚未确认".into(),
        Reason::PartialOffer => "候选未完整识别\n暂不指定本组优先".into(),
        Reason::ComparableTie => "本组证据未拉开差异\n暂不指定唯一优先".into(),
        Reason::ConflictingEvidence => "情境证据相互冲突\n仍按整体参考顺序".into(),
        Reason::MissingEvidence => "尚缺可比较的历史证据\n缺失记录不等于不搭配".into(),
    }
}

pub(crate) fn stage_text(decision: &Decision) -> String {
    match decision.stage.pattern {
        StagePattern::Unavailable => "阶段资料未就绪".into(),
        StagePattern::Partial => format!("阶段资料不全 · 已报 {}/4", decision.stage.reported),
        StagePattern::LowSample => format!("阶段支持不足 · ≥500场 {}/4", decision.stage.supported),
        StagePattern::NoClearOrder => "四阶段齐全 · 未分出明确顺序".into(),
        StagePattern::Consistent => "四阶段一致 · 当前轮次未确认".into(),
        StagePattern::Reversal => "阶段表现反转 · 不按当前轮次猜测".into(),
    }
}

fn same_arc<T: PartialEq>(a: Option<&Arc<T>>, b: Option<&Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || **a == **b,
        (None, None) => true,
        _ => false,
    }
}

struct Rendered {
    champion: u32,
    dataset: String,
    ids: [Option<u32>; 3],
    cards: [Option<Arc<fuze_data::champion_detail::Augment>>; 3],
    scores: [Option<f64>; 3],
    history: Option<Arc<HistoryContext>>,
    snapshot: Option<Arc<Snapshot>>,
    stale: bool,
    usable: bool,
    view: Arc<Comparison>,
}

#[derive(Clone, PartialEq)]
struct Context {
    champion: u32,
    dataset: String,
    prepare: bool,
}
pub struct Reference {
    roots: Vec<fuze_data::Root>,
    owned: fuze_worker_host::augment_picks::Owned,
    patch: String,
    pub store: Arc<Store>,
    offline: bool,
    service: Option<Service>,
    context: Option<Context>,
    since: Instant,
    requested: bool,
    retry: Option<u64>,
    serial: u64,
    snapshot: Option<Arc<Snapshot>>,
    names: Arc<HashMap<u32, String>>,
    rendered: Option<Rendered>,
}
impl Reference {
    /// The public preview has no active game or confirmed ownership. Reuse the
    /// same relative-choice adapter without starting a service or inventing stages.
    pub fn preview(frame: &Frame, champion: u32) -> Arc<Comparison> {
        let mut reference = Self::new(true);
        reference.context = Some(Context {
            champion,
            dataset: frame.snapshot.dataset_version.clone(),
            prepare: false,
        });
        reference.cards(frame)
    }
    pub fn new(offline: bool) -> Self {
        Self::with_roots(offline, Vec::new())
    }
    pub fn with_roots(offline: bool, roots: Vec<fuze_data::Root>) -> Self {
        let store = Arc::new(Store::default());
        store.allow_network(false);
        Self {
            roots,
            owned: Default::default(),
            patch: String::new(),
            store,
            offline,
            service: None,
            context: None,
            since: Instant::now(),
            requested: false,
            retry: None,
            serial: 0,
            snapshot: None,
            names: Default::default(),
            rendered: None,
        }
    }
    pub fn update(&mut self, status: &Status) {
        if *self.owned != *status.pipeline.picked || self.patch != status.recognition.reference {
            self.owned = status.pipeline.picked.clone();
            self.patch = status.recognition.reference.clone();
            self.rendered = None;
        }
        let prepare = status.mode == 1
            && matches!(
                status.connection,
                Some(State::Connected(Phase::ChampSelect | Phase::Loading))
            );
        self.store.allow_network(prepare);
        let view = if status.mode != 1 || !status.preferences.augment_overlay {
            None
        } else if prepare {
            status.selection.as_ref()
        } else if matches!(status.connection, Some(State::Connected(Phase::InProgress))) {
            status.playing.as_ref()
        } else {
            None
        };
        let mut next = view
            .and_then(|v| {
                Some(Context {
                    champion: v.team.get(v.local_slot?)?.id,
                    dataset: v.version.clone(),
                    prepare,
                })
            })
            .filter(|c| c.champion > 0);
        // Carry only the last verified selection into Loading; gameplay checks the actual
        // champion again. No extra LCU query and never use this to infer a reward stage.
        if next.is_none()
            && prepare
            && status.preferences.augment_overlay
            && matches!(status.connection, Some(State::Connected(Phase::Loading)))
        {
            next = self.context.clone().filter(|c| c.prepare);
        }
        if next != self.context {
            if let Some(service) = &self.service {
                service.request(None);
            }
            self.context = next;
            self.since = Instant::now();
            self.requested = false;
            self.retry = None;
            self.snapshot = None;
            self.names = Default::default();
            self.rendered = None;
        }
        let Some(context) = &self.context else { return };
        let at = stage_reference::now();
        if let Some(s) = self.store.peek(context.champion, at) {
            self.snapshot = Some(s);
        }
        // Keep an observed expired snapshot so the UI can distinguish expiry from
        // data never loaded. Neither compare() nor the adapter uses expired rows.
        if let Some(reply) = self.service.as_ref().and_then(|s| s.changed(self.serial)) {
            self.serial = reply.serial;
            if !same_arc(Some(&self.names), Some(&reply.names)) {
                self.names = reply.names.clone();
                self.rendered = None;
            }
            if let Some(s) = reply.snapshot.filter(|s| s.usable(context.champion, at)) {
                self.snapshot = Some(s);
            }
            self.retry = if context.prepare && reply.issue == Some("stage-busy") {
                reply.retry_after
            } else {
                None
            };
        }
        if self.requested && !self.retry.is_some_and(|t| at >= t) {
            return;
        }
        if context.prepare && self.since.elapsed() < Duration::from_millis(750) {
            return;
        }
        if self.service.is_none() {
            // Reuse the same local data roots as the hero details and recognition service.
            self.service =
                Service::with_store(self.roots.clone(), self.offline, self.store.clone()).ok();
        }
        self.requested = true;
        self.retry = None;
        if let Some(service) = &self.service {
            service.request(Some(Request {
                id: context.champion,
                locale: fuze_data::Locale::ZhCn,
                access: if context.prepare {
                    Access::Prepare
                } else {
                    Access::CacheOnly
                },
            }));
        }
    }
    pub fn diagnostics(&self) -> fuze_worker_host::augment_combos::Diagnostics {
        use fuze_worker_host::augment_combos::{DataState, Diagnostics};
        let champion = self.context.as_ref().map_or(0, |c| c.champion);
        let source = self
            .snapshot
            .as_ref()
            .filter(|s| s.usable(champion, stage_reference::now()));
        let data = match source {
            None => DataState::Unavailable,
            Some(s)
                if fuze_data::recognition::patch(&self.patch).is_none()
                    || fuze_data::recognition::patch(&self.patch)
                        != fuze_data::recognition::patch(&s.patch) =>
            {
                DataState::PatchMismatch
            }
            Some(_) => DataState::Ready,
        };
        Diagnostics {
            data,
            rows: source
                .and_then(|s| s.combinations.as_ref())
                .map_or(0, |r| r.len().min(u16::MAX as usize) as u16),
            matched: 0,
            delivered: false,
        }
    }
    pub fn cards(&mut self, frame: &Frame) -> Arc<Comparison> {
        self.cards_at(frame, stage_reference::now())
    }
    fn cards_at(&mut self, frame: &Frame, at: u64) -> Arc<Comparison> {
        let ids = frame.snapshot.slots.each_ref().map(|slot| match slot {
            fuze_contract::Slot::Known { id, .. } => Some(*id),
            _ => None,
        });
        let dataset = frame.snapshot.dataset_version.as_str();
        let champion = self.context.as_ref().map_or(0, |c| c.champion);
        let stale = self.snapshot.as_ref().is_some_and(|s| s.stale(at));
        let usable = self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.usable(champion, at));
        let same = self.rendered.as_ref().is_some_and(|r| {
            r.champion == champion
                && stale == r.stale
                && usable == r.usable
                && r.dataset == dataset
                && r.ids == ids
                && r.scores == frame.scores
                && same_arc(r.history.as_ref(), frame.history.as_ref())
                && r.cards
                    .iter()
                    .zip(&frame.cards)
                    .all(|(a, b)| same_arc(a.as_ref(), b.as_ref()))
                && match (&r.snapshot, &self.snapshot) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        });
        if same {
            return self.rendered.as_ref().unwrap().view.clone();
        }
        let mut view = compare(self.snapshot.as_deref(), champion, ids, dataset, at);
        let history = frame
            .history
            .as_deref()
            .filter(|h| h.champion == champion && h.dataset == dataset);
        let mut names = (*self.names).clone();
        names.extend(self.owned.iter().cloned());
        for slot in &frame.snapshot.slots {
            if let fuze_contract::Slot::Known { id, name, .. } = slot {
                names.insert(*id, name.clone());
            }
        }
        let mut evidence: [Option<fuze_worker_host::augment_combos::CandidateEvidence>; 3] =
            Default::default();
        let mut eligible = [false; 3];
        let candidates: [Candidate; 3] = std::array::from_fn(|i| {
            let Some(id) = ids[i].filter(|id| fuze_contract::skill_choice::get(*id).is_none())
            else {
                return Candidate::default();
            };
            let row = history.and_then(|h| h.by_augment.get(&id));
            let overall = row.and_then(|r| r.rate).or_else(|| {
                let card = frame.cards[i].as_ref().filter(|a| a.id == id)?;
                Some(Rate {
                    value: card.win?,
                    games: card.evidence.games,
                    wins: card.evidence.wins,
                })
            });
            view.history[i] = historical(row, history.and_then(|h| h.baseline.as_ref()), overall);
            let source = self.snapshot.as_deref().filter(|s| s.usable(champion, at));
            eligible[i] = source.is_some_and(|s| {
                s.patch_matches(dataset)
                    && fuze_data::recognition::patch(&self.patch).is_some()
                    && fuze_data::recognition::patch(&self.patch)
                        == fuze_data::recognition::patch(&s.patch)
                    && row.is_some_and(|r| r.scope.matches_aramkit(&s.patch, &s.date))
            });
            let e = fuze_worker_host::augment_combos::evaluate(
                self.snapshot.as_deref(),
                champion,
                &self.patch,
                &self.owned,
                id,
                at,
            );
            let to_combo = |r: &fuze_worker_host::augment_combos::ComboEvidence| {
                contextual_advice::Combination {
                    required: r.required.clone(),
                    rate: Rate {
                        value: r.win,
                        games: Some(r.samples),
                        wins: None,
                    },
                }
            };
            let candidate = Candidate {
                id: Some(id),
                score: frame.scores[i],
                overall,
                stages: std::array::from_fn(|stage| {
                    let row = source.filter(|_| eligible[i])?.rows[stage + 1]
                        .iter()
                        .find(|r| r.id == id)?;
                    Some(Rate {
                        value: row.win,
                        games: Some(row.samples),
                        wins: None,
                    })
                }),
                current: if eligible[i] {
                    e.current.iter().map(to_combo).collect()
                } else {
                    Vec::new()
                },
                future: if eligible[i] {
                    e.future.iter().map(to_combo).collect()
                } else {
                    Vec::new()
                },
            };
            evidence[i] = Some(e);
            candidate
        });
        view.decisions = contextual_advice::decide(&candidates);
        let dataset_patch =
            |value: &str| fuze_data::recognition::patch(value.rsplit('@').next().unwrap_or(value));
        // recognition::patch normalizes both 16.x and 26.x to the 2026 family;
        // the reviewed resource snapshot uses the exact provider label 16.22.
        let mechanism_patch = fuze_data::recognition::patch(&self.patch)
            .filter(|patch| {
                Some(*patch) == dataset_patch(dataset)
                    && self
                        .context
                        .as_ref()
                        .is_some_and(|c| Some(*patch) == dataset_patch(&c.dataset))
            })
            .filter(|patch| *patch == (2026, 22))
            .map(|_| "16.22");
        let mut withheld = false;
        for i in 0..3 {
            let Some(e) = evidence[i].as_ref() else {
                continue;
            };
            let decision = &view.decisions[i];
            view.reasons[i] = reason_text(decision, &names);
            view.mechanisms[i] = ids[i].and_then(|id| {
                fuze_data::mechanism_reference::notice(id, &self.owned, mechanism_patch?)
            });
            view.stage_summary[i] = if usable && !eligible[i] {
                withheld = true;
                "异批次/版本 · 仅作历史参考".into()
            } else {
                stage_text(decision)
            };
            view.current_combos[i] = !e.current.is_empty();
            view.combo_alternatives[i] = (e.current.len() + e.future.len()).saturating_sub(1);
            view.combos[i] =
                fuze_worker_host::augment_combos::present(e, &names, decision.reason.required());
            let other = match decision.reason {
                Reason::StageAdvantage { other }
                | Reason::StageAlternative { other }
                | Reason::ComboAdvantage { other, .. }
                | Reason::ComboAlternative { other, .. } => format!(
                    "\n比较对象：{}",
                    names
                        .get(&other)
                        .cloned()
                        .unwrap_or_else(|| format!("强化 #{other}"))
                ),
                _ => String::new(),
            };
            view.details[i].push_str(&format!("\n\n{}\n{}{}\n英雄均值差只描述历史，不是选择该符文的因果收益。约95%区间为该符文单项胜率的近似抽样区间。",
                decision.choice.label(), view.reasons[i], other));
            if let Some(notice) = &view.mechanisms[i] {
                view.details[i].push_str(&format!("\n\n{}\n{}", notice.heading, notice.detail));
            }
        }
        if withheld {
            view.source
                .push_str(" · 异批次/版本的阶段与搭配不参与本组判断");
        }
        let view = Arc::new(view);
        self.rendered = Some(Rendered {
            champion,
            dataset: dataset.into(),
            ids,
            cards: frame.cards.clone(),
            scores: frame.scores,
            history: frame.history.clone(),
            snapshot: self.snapshot.clone(),
            stale,
            usable,
            view: view.clone(),
        });
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stage_reference::Row;
    fn frame(ids: [Option<u32>; 3], dataset: &str) -> Frame {
        Frame {
            context: 1,
            selection_cycle: 1,
            snapshot: fuze_contract::Snapshot {
                schema_version: 1,
                generation: 1,
                sequence: 1,
                dataset_version: dataset.into(),
                selection_confirmed: false,
                actual_stage: fuze_contract::ActualStage::Unknown,
                slots: ids.map(|id| {
                    id.map_or(fuze_contract::Slot::Unreadable, |id| {
                        fuze_contract::Slot::Known {
                            id,
                            name: format!("符文{id}"),
                            confidence: 1.,
                        }
                    })
                }),
            },
            retained: false,
            deadline: None,
            cards: Default::default(),
            scores: [None; 3],
            history: None,
        }
    }
    #[test]
    fn automatic_status_updates_combo_cards_without_actions_or_changing_single_rates() {
        use fuze_worker_host::{
            champ_select::{Hero, View},
            stage_reference::Combination,
        };
        let mut state = Status::default();
        state.mode = 1;
        state.preferences.augment_overlay = true;
        state.connection = Some(State::Connected(Phase::InProgress));
        state.recognition.reference = "16.22.1".into();
        state.playing = Some(View {
            team: vec![Hero {
                id: 145,
                name: "合成英雄".into(),
                icon: String::new(),
                win: None,
                pick: None,
                games: None,
                tier: None,
            }],
            local_slot: Some(0),
            bench: vec![],
            choices: None,
            version: "16.22".into(),
        });
        let mut r = Reference::new(true);
        r.update(&state);
        r.snapshot = Some(Arc::new(Snapshot {
            schema: 1,
            champion: 145,
            patch: "16.22".into(),
            date: "2026-11-06".into(),
            data_path: String::new(),
            fetched: stage_reference::now(),
            rows: Default::default(),
            combinations: Some(vec![Combination {
                ids: vec![1004, 3086],
                win: 0.6495,
                samples: 6819,
            }]),
        }));
        let ids = [Some(3086), Some(9999), None];
        let frame = frame(ids, "16.22");
        assert_eq!(
            r.diagnostics().data,
            fuze_worker_host::augment_combos::DataState::Ready
        );
        assert_eq!(r.diagnostics().rows, 1);
        let single = r.cards(&frame);
        assert!(single.combos[0].heading.starts_with("后续留意"));
        assert!(!single.current_combos[0]);
        assert!(single.combos[1].heading.contains("组合前置"));
        state.pipeline.picked = Arc::new(vec![(1004, "秘术冲拳".into())]);
        r.update(&state);
        let combo = r.cards(&frame);
        assert_eq!(combo.combos[0].partners, "秘术冲拳");
        assert_eq!(combo.combos[0].statistic, "两符文搭配胜率\n65.0% · 6819 场");
        assert!(combo.current_combos[0]);
        assert!(
            combo.combos[1].heading.contains("暂无对应组合记录")
                && combo.combos[2].detail.is_empty()
        );
        assert_eq!(combo.values, single.values);
        assert!(Arc::ptr_eq(&combo, &r.cards(&frame)));
        state.pipeline.picked = Arc::new(Vec::new());
        r.update(&state);
        let cleared = r.cards(&frame);
        assert!(cleared.current_combos.iter().all(|current| !current));
        assert!(cleared.combos[0].heading.starts_with("后续留意"));
        assert!(!Arc::ptr_eq(&combo, &cleared));
        assert_eq!(cleared.values, single.values);
        assert!(Arc::ptr_eq(&cleared, &r.cards(&frame)));
        state.pipeline.picked = Arc::new(vec![(1004, "秘术冲拳".into())]);
        r.update(&state);
        assert!(r.cards(&frame).current_combos[0]);
        state.recognition.reference = "16.23".into();
        r.update(&state);
        assert!(r.cards(&frame).combos[0].summary.contains("补丁不一致"));
        assert_eq!(
            r.diagnostics().data,
            fuze_worker_host::augment_combos::DataState::PatchMismatch
        );
        state.mode = 2;
        r.update(&state);
        assert_eq!(
            r.diagnostics().data,
            fuze_worker_host::augment_combos::DataState::Unavailable
        );
        assert!(r.cards(&frame).combos[0].summary.contains("尚未就绪"));
    }
    #[test]
    fn loading_carries_verified_champion_but_gameplay_rechecks_and_pause_clears_it() {
        use fuze_worker_host::champ_select::{Hero, View};
        let hero = |id| View {
            team: vec![Hero {
                id,
                name: "合成英雄".into(),
                icon: String::new(),
                win: None,
                pick: None,
                games: None,
                tier: None,
            }],
            local_slot: Some(0),
            bench: Vec::new(),
            choices: None,
            version: "zh-CN@16.20.1".into(),
        };
        let mut state = Status::default();
        state.mode = 1;
        state.preferences.augment_overlay = true;
        state.connection = Some(State::Connected(Phase::ChampSelect));
        state.selection = Some(hero(21));
        let mut r = Reference::new(true);
        r.update(&state);
        assert_eq!(r.context.as_ref().unwrap().champion, 21);
        assert!(!r.requested); // short selection debounce, no worker/network
        state.selection = None;
        state.connection = Some(State::Connected(Phase::Loading));
        r.update(&state);
        assert_eq!(r.context.as_ref().unwrap().champion, 21);
        state.connection = Some(State::Connected(Phase::InProgress));
        state.playing = Some(hero(22));
        r.update(&state);
        assert_eq!(r.context.as_ref().unwrap().champion, 22);
        assert!(!r.context.as_ref().unwrap().prepare);
        let frame = frame([Some(1), None, Some(3)], "16.20");
        let a = r.cards(&frame);
        let b = r.cards(&frame);
        assert!(Arc::ptr_eq(&a, &b));
        state.mode = 2;
        r.update(&state);
        assert!(r.context.is_none() && r.snapshot.is_none());
    }
    #[test]
    fn fixed_slots_missing_samples_expiry_and_context_never_borrow_overall_rates() {
        let mut s = Snapshot {
            combinations: Some(Vec::new()),
            schema: 1,
            champion: 21,
            patch: "16.20".into(),
            date: "2026-10-16".into(),
            data_path: String::new(),
            fetched: 100,
            rows: Default::default(),
        };
        s.rows[0] = vec![Row {
            id: 1,
            win: 0.99,
            pick: 0.2,
            samples: 1000,
        }];
        s.rows[1] = vec![Row {
            id: 1,
            win: 0.55,
            pick: 0.2,
            samples: 200,
        }];
        s.rows[2] = vec![Row {
            id: 1,
            win: 0.98,
            pick: 0.2,
            samples: 199,
        }];
        let ids = [Some(1), None, Some(9)];
        let v = compare(Some(&s), 21, ids, "zh-CN@16.19.3", 101);
        assert_eq!(v.values[0], ["55.0%", "98.0%", "—", "—"]);
        assert_eq!(v.samples[0], ["200", "199*", "—", "—"]);
        assert_eq!(v.values[1], ["—"; 4]);
        assert_eq!(v.samples[1], ["—"; 4]);
        assert_eq!(v.values[2], ["—"; 4]);
        assert_eq!(v.samples[2], ["—"; 4]);
        assert!(v.source.contains("地区未确认") && v.source.contains("与本地补丁不同"));
        assert!(v.details[0].contains("200 场") && !v.details[0].contains("99.0%"));
        assert_eq!(
            compare(Some(&s), 22, ids, "16.20", 101),
            Comparison::default()
        );
        let expired = compare(Some(&s), 21, ids, "16.20", 100 + 7 * 86400 + 1);
        assert!(expired.source.contains("已过期"));
        assert_eq!(expired.values, Comparison::default().values);
    }

    fn contextual_fixture() -> (Reference, Frame, u64) {
        use decision_evidence::{History, Relation, Scope};
        let at = 1_000_000;
        let mut reference = Reference::new(true);
        reference.context = Some(Context {
            champion: 145,
            dataset: "16.22".into(),
            prepare: false,
        });
        reference.patch = "16.22.1".into();
        let scope = Scope {
            source: fuze_data::aramkit::SOURCE.into(),
            patch: "16.22".into(),
            date: "2026-11-06".into(),
            region: "地区未标明".into(),
        };
        let rate = |value| Rate {
            value,
            games: Some(10_000),
            wins: None,
        };
        let mut frame = frame([Some(1), Some(2), Some(3)], "16.22");
        frame.scores = [Some(0.9), Some(0.8), Some(0.7)];
        frame.history = Some(Arc::new(HistoryContext {
            dataset: "16.22".into(),
            champion: 145,
            baseline: Some(HeroReference {
                champion: 145,
                rate: rate(0.7),
                scope: scope.clone(),
            }),
            by_augment: [1, 2, 3]
                .into_iter()
                .map(|id| {
                    (
                        id,
                        AugmentHistory {
                            rate: Some(rate(0.55)),
                            scope: scope.clone(),
                            comparison: Some(History {
                                delta: -0.15,
                                interval: decision_evidence::interval95(0.55, 10_000),
                                relation: Relation::Lower,
                                low_sample: false,
                            }),
                        },
                    )
                })
                .collect(),
        }));
        reference.snapshot = Some(Arc::new(Snapshot {
            schema: 1,
            champion: 145,
            patch: "16.22".into(),
            date: "2026-11-06".into(),
            data_path: String::new(),
            fetched: at,
            rows: std::array::from_fn(|_| {
                [0.6, 0.8, 0.4]
                    .into_iter()
                    .enumerate()
                    .map(|(i, win)| Row {
                        id: i as u32 + 1,
                        win,
                        pick: 0.1,
                        samples: 10_000,
                    })
                    .collect()
            }),
            combinations: Some(Vec::new()),
        }));
        (reference, frame, at)
    }

    #[test]
    fn comparable_stages_drive_one_decision_while_negative_baselines_remain_descriptive() {
        let (mut reference, frame, at) = contextual_fixture();
        let view = reference.cards_at(&frame, at);
        assert!(view.preferred(1, Some(2)));
        assert!(!view.preferred(0, Some(1)) && !view.preferred(1, Some(1)));
        assert_eq!(view.decisions[0].choice, Choice::NotAdvised);
        assert!(matches!(
            view.decisions[1].reason,
            Reason::StageAdvantage { .. }
        ));
        assert!(view.history.iter().all(|h| h.delta == "-15.0 pp"));
        assert!(view.history.iter().all(|h| h.relation == "低于英雄均值"));
        assert!(
            view.history[1].samples.contains("n=10000")
                && view.history[1].samples.contains("70.0%")
        );
        assert!(view.history[1].interval.contains("约95%"));
        assert!(view.reasons[1].contains("符文") && view.stage_summary[1].contains("四阶段一致"));
        assert_eq!(
            frame.snapshot.actual_stage,
            fuze_contract::ActualStage::Unknown
        );
        assert_eq!(frame.scores, [Some(0.9), Some(0.8), Some(0.7)]);

        // Without contextual rows the original relative ordering remains available,
        // even though all three augment cohorts are below the hero's broad mean.
        reference.snapshot = None;
        let overall = reference.cards_at(&frame, at);
        assert!(overall.preferred(0, Some(1)));
        assert_eq!(overall.decisions[0].reason, Reason::OverallRank);
        assert_eq!(overall.history[0].delta, "-15.0 pp");
        let preview = Reference::preview(&frame, 145);
        assert!(preview.preferred(0, Some(1)) && preview.current_combos == [false; 3]);
        assert_eq!(preview.history[0].relation, "低于英雄均值");

        let mut near = frame.clone();
        let row = Arc::make_mut(near.history.as_mut().unwrap())
            .by_augment
            .get_mut(&1)
            .unwrap();
        row.rate.as_mut().unwrap().value = 0.695;
        let comparison = row.comparison.as_mut().unwrap();
        comparison.delta = -0.005;
        comparison.relation = decision_evidence::Relation::Near;
        comparison.interval = decision_evidence::interval95(0.695, 10_000);
        let near = reference.cards_at(&near, at);
        assert_eq!(near.history[0].relation, "接近英雄均值");
        assert_eq!(near.history[0].delta, "-0.5 pp");
        assert!(near.preferred(0, Some(1)));
    }

    #[test]
    fn cache_tracks_evidence_but_not_ocr_sequence_and_withholds_mixed_batches() {
        let (mut reference, mut frame, at) = contextual_fixture();
        let first = reference.cards_at(&frame, at);
        frame.snapshot.sequence += 1;
        assert!(Arc::ptr_eq(&first, &reference.cards_at(&frame, at)));
        frame.history = frame.history.as_ref().map(|h| Arc::new((**h).clone()));
        assert!(Arc::ptr_eq(&first, &reference.cards_at(&frame, at)));
        let history = Arc::make_mut(frame.history.as_mut().unwrap());
        for row in history.by_augment.values_mut() {
            row.scope.date = "2026-11-05".into();
            row.comparison = None;
        }
        let mixed = reference.cards_at(&frame, at);
        assert!(!Arc::ptr_eq(&first, &mixed));
        assert!(mixed.preferred(0, Some(1)) && !mixed.preferred(1, Some(2)));
        assert_eq!(mixed.values, first.values); // Original reference remains inspectable.
        assert_eq!(mixed.history[0].delta, "不可比较");
        assert!(mixed.source.contains("不参与本组判断"));
        assert!(mixed.stage_summary[0].contains("异批次/版本"));
        assert!(Arc::ptr_eq(&mixed, &reference.cards_at(&frame, at)));
        frame.scores[2] = Some(1.);
        let reordered = reference.cards_at(&frame, at);
        assert!(!Arc::ptr_eq(&mixed, &reordered) && reordered.preferred(2, Some(3)));

        let expired = reference.cards_at(&frame, at + 7 * 86400 + 1);
        assert!(!Arc::ptr_eq(&reordered, &expired));
        assert!(expired.source.contains("已过期"));
        assert!(expired.combos[0].summary.contains("已过期"));
        assert_eq!(expired.values, Comparison::default().values);
    }

    #[test]
    fn partial_evidence_and_future_routes_do_not_create_contextual_priority() {
        let (mut reference, mut frame, at) = contextual_fixture();
        frame.scores = [None; 3];
        let source = Arc::make_mut(reference.snapshot.as_mut().unwrap());
        source.rows[4].clear();
        source.combinations = Some(vec![stage_reference::Combination {
            ids: vec![1, 1004],
            win: 0.99,
            samples: 100_000,
        }]);
        reference.names = Arc::new(HashMap::from([(1004, "秘术冲拳".into())]));
        let view = reference.cards_at(&frame, at);
        assert!(view.decisions.iter().all(|d| d.choice != Choice::Preferred));
        assert!(view.stage_summary[0].contains("已报 3/4"));
        assert_eq!(view.combos[0].heading, "后续留意 · 秘术冲拳");
        assert!(view.combos[0].summary.contains("99.0%") && !view.current_combos[0]);
        assert!(view.combos[1].summary.contains("暂无此符文"));
        frame.snapshot.slots[2] = fuze_contract::Slot::Unreadable;
        frame.scores = [Some(0.9), Some(0.8), None];
        let partial = reference.cards_at(&frame, at);
        assert!(partial
            .decisions
            .iter()
            .all(|d| d.choice != Choice::Preferred));
        assert_eq!(partial.decisions[0].reason, Reason::PartialOffer);
    }

    #[test]
    fn mechanism_notices_require_matching_reviewed_patch_without_changing_choices() {
        let (mut reference, _, at) = contextual_fixture();
        let mut frame = frame([Some(1011), Some(12317), Some(3)], "zh-CN@16.22.1");
        frame.scores = [Some(0.9), Some(0.8), Some(0.7)];
        reference.owned = Arc::new(vec![(1004, "回归基本功".into())]);
        reference.patch = "16.22.1".into();
        let shown = reference.cards_at(&frame, at);
        assert_eq!(shown.mechanisms[0].as_ref().unwrap().related, [1004]);
        assert!(shown.mechanisms[0]
            .as_ref()
            .unwrap()
            .summary
            .contains("说明推论"));
        assert!(shown.mechanisms[1]
            .as_ref()
            .unwrap()
            .summary
            .contains("随机"));
        assert!(shown.preferred(0, Some(1011))); // A notice never creates a statistical penalty.
        reference.patch = "26.22.1".into();
        reference.rendered = None;
        assert!(reference.cards_at(&frame, at).mechanisms[0].is_some());
        reference.patch = "16.23.1".into();
        reference.rendered = None;
        let disabled = reference.cards_at(&frame, at);
        assert!(disabled.mechanisms.iter().all(Option::is_none));
        assert_eq!(shown.decisions, disabled.decisions);
        reference.patch = "16.22.1".into();
        reference.context.as_mut().unwrap().dataset = "zh-CN@16.23.1".into();
        reference.rendered = None;
        assert!(reference
            .cards_at(&frame, at)
            .mechanisms
            .iter()
            .all(Option::is_none));
    }
}
