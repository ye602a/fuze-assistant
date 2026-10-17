//! Public stage comparison only. It cannot supply a claim stage or replace overall scores.
use fuze_lcu::reconnect::State;
use fuze_runtime::session::Phase;
use fuze_worker_host::{
    desktop::Status,
    stage_reference::{self, Access, Request, Service, Snapshot, Store},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub values: [[String; 4]; 3],
    pub details: [String; 3],
    pub source: String,
}
impl Default for Comparison {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|_| std::array::from_fn(|_| "—".into())),
            details: Default::default(),
            source: "阶段参考暂无缓存 · 选人及加载时准备，局内不联网".into(),
        }
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
                Some(r) if r.samples >= 200 => format!("{:.1}%", r.win * 100.),
                Some(_) => "少样本".into(),
                None => "—".into(),
            };
            details.push(match row {
                Some(r) if r.samples >= 200 => {
                    format!("第 {} 次：{} · {} 场", j + 1, value, r.samples)
                }
                Some(r) => format!("第 {} 次：仅 {} 场，未展示胜率", j + 1, r.samples),
                None => format!("第 {} 次：暂无统计", j + 1),
            });
            view.values[i][j] = value;
        }
        view.details[i] = format!(
            "\n\n{}\n{}\n四次正式选择的统计参考，未确认当前轮次，不改变整体推荐。",
            view.source,
            details.join("\n")
        );
    }
    view
}

#[derive(Clone, PartialEq)]
struct Context {
    champion: u32,
    dataset: String,
    prepare: bool,
}
pub struct Reference {
    pub store: Arc<Store>,
    offline: bool,
    service: Option<Service>,
    context: Option<Context>,
    since: Instant,
    requested: bool,
    retry: Option<u64>,
    serial: u64,
    snapshot: Option<Arc<Snapshot>>,
    rendered: Option<(
        u32,
        String,
        [Option<u32>; 3],
        Option<Arc<Snapshot>>,
        bool,
        Arc<Comparison>,
    )>,
}
impl Reference {
    pub fn new(offline: bool) -> Self {
        let store = Arc::new(Store::default());
        store.allow_network(false);
        Self {
            store,
            offline,
            service: None,
            context: None,
            since: Instant::now(),
            requested: false,
            retry: None,
            serial: 0,
            snapshot: None,
            rendered: None,
        }
    }
    pub fn update(&mut self, status: &Status) {
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
            self.rendered = None;
        }
        let Some(context) = &self.context else { return };
        let at = stage_reference::now();
        if let Some(s) = self.store.peek(context.champion, at) {
            self.snapshot = Some(s);
        }
        if self
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.usable(context.champion, at))
        {
            self.snapshot = None;
        }
        if let Some(reply) = self.service.as_ref().and_then(|s| s.changed(self.serial)) {
            self.serial = reply.serial;
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
            // No name catalog needed here; card titles already come from OCR/catalog.
            self.service = Service::with_store(Vec::new(), self.offline, self.store.clone()).ok();
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
    pub fn cards(&mut self, ids: [Option<u32>; 3], dataset: &str) -> Arc<Comparison> {
        let champion = self.context.as_ref().map_or(0, |c| c.champion);
        let stale = self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.stale(stage_reference::now()));
        let same =
            self.rendered
                .as_ref()
                .is_some_and(|(id, version, slots, source, old_stale, _)| {
                    *id == champion
                        && stale == *old_stale
                        && version == dataset
                        && *slots == ids
                        && match (source, &self.snapshot) {
                            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                            (None, None) => true,
                            _ => false,
                        }
                });
        if same {
            return self.rendered.as_ref().unwrap().5.clone();
        }
        let view = Arc::new(compare(
            self.snapshot.as_deref(),
            champion,
            ids,
            dataset,
            stage_reference::now(),
        ));
        self.rendered = Some((
            champion,
            dataset.into(),
            ids,
            self.snapshot.clone(),
            stale,
            view.clone(),
        ));
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stage_reference::Row;
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
        let a = r.cards([Some(1), None, Some(3)], "16.20");
        let b = r.cards([Some(1), None, Some(3)], "16.20");
        assert!(Arc::ptr_eq(&a, &b));
        state.mode = 2;
        r.update(&state);
        assert!(r.context.is_none() && r.snapshot.is_none());
    }
    #[test]
    fn fixed_slots_missing_samples_expiry_and_context_never_borrow_overall_rates() {
        let mut s = Snapshot {
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
        assert_eq!(v.values[0], ["55.0%", "少样本", "—", "—"]);
        assert_eq!(v.values[1], ["—"; 4]);
        assert_eq!(v.values[2], ["—"; 4]);
        assert!(v.source.contains("地区未确认") && v.source.contains("与本地补丁不同"));
        assert!(v.details[0].contains("200 场") && !v.details[0].contains("99.0%"));
        assert_eq!(
            compare(Some(&s), 22, ids, "16.20", 101),
            Comparison::default()
        );
        assert_eq!(
            compare(Some(&s), 21, ids, "16.20", 100 + 7 * 86400 + 1),
            Comparison::default()
        );
    }
}
