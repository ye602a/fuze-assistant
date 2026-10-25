//! Local catalogue worker and independently scheduled public artwork.
use crate::{BuildRow, ChampionChoice, ChampionWindow, DetailRow};
use fuze_data::detail_cache::{Cache, Loaded};
use slint::{ComponentHandle, Model, VecModel};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::Path,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
#[derive(Default)]
struct Work {
    wanted: Option<(u64, u32, String)>,
    expected: String,
    generation: u64,
    stop: bool,
    invalidate: bool,
    reload: bool,
    reply: Option<(u64, Result<Arc<Loaded>, &'static str>)>,
}
struct Worker {
    shared: Arc<(Mutex<Work>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    fn new(roots: Vec<fuze_data::Root>) -> Result<Self, &'static str> {
        let shared = Arc::new((
            Mutex::new(Work {
                invalidate: true,
                ..Work::default()
            }),
            Condvar::new(),
        ));
        let task = shared.clone();
        let thread = std::thread::Builder::new()
            .name("fuze-slint-details".into())
            .spawn(move || {
                let mut cache: Option<Cache> = None;
                let mut failure = "detail-data-not-ready";
                loop {
                    let mut s = match task.0.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    while !s.stop
                        && s.wanted.is_none()
                        && !s.invalidate
                        && !cache.as_ref().is_some_and(Cache::warming)
                    {
                        s = match task.1.wait(s) {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                    if s.stop {
                        return;
                    }
                    if std::mem::take(&mut s.invalidate) {
                        let expected = s.expected.clone();
                        if std::mem::take(&mut s.reload) {
                            cache = None;
                        }
                        drop(s);
                        if !cache
                            .as_ref()
                            .is_some_and(|c| expected.is_empty() || c.identity() == expected)
                        {
                            cache = None;
                            match Cache::prepare(&roots, &expected) {
                                Ok(next) => cache = Some(next),
                                Err(error) => failure = error,
                            }
                        }
                        continue; // Check a possible version change before serving queued work.
                    }
                    let Some((generation, id, expected)) = s.wanted.take() else {
                        drop(s);
                        if let Some(cache) = cache.as_mut() {
                            cache.warm_next();
                        }
                        std::thread::yield_now();
                        continue;
                    };
                    drop(s);
                    // A previous failed preload is not permanent: a later click/retry may
                    // arrive after a data update or a temporarily unavailable local file.
                    if !cache
                        .as_ref()
                        .is_some_and(|c| expected.is_empty() || c.identity() == expected)
                    {
                        match Cache::prepare(&roots, &expected) {
                            Ok(next) => cache = Some(next),
                            Err(error) => failure = error,
                        }
                    }
                    let result = match cache.as_mut() {
                        Some(cache) if expected.is_empty() || cache.identity() == expected => {
                            cache.get(id)
                        }
                        Some(_) => Err("detail-version-mismatch"),
                        None => Err(failure),
                    };
                    let mut s = match task.0.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    if s.stop {
                        return;
                    }
                    if s.generation != generation {
                        continue;
                    }
                    s.reply = Some((generation, result));
                }
            })
            .map_err(|_| "details-thread")?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }
    fn request(&self, id: u32, reload: bool) {
        if let Ok(mut s) = self.shared.0.lock() {
            s.generation = s.generation.wrapping_add(1);
            s.wanted = Some((s.generation, id, s.expected.clone()));
            s.reply = None;
            if reload {
                s.invalidate = true;
                s.reload = true;
            }
            self.shared.1.notify_one();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Ok(mut s) = self.shared.0.lock() {
            s.stop = true;
            self.shared.1.notify_one();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub struct Binding {
    _stage: crate::stage_bridge::Binding,
    actions: Rc<crate::champion_actions::Binding>,
    art: Rc<crate::champion_art::Binding>,
    _worker: Rc<Worker>,
    _timer: Rc<slint::Timer>,
    state: Rc<RefCell<Presentation>>,
    known_icons: HashMap<i32, slint::Image>,
    version: RefCell<String>,
}
impl Binding {
    pub fn automatic(&self, ui: &ChampionWindow, enabled: bool) {
        self.actions.automatic(ui, enabled);
    }
    /// Every changed selection snapshot replaces both models; never retain an obsolete bench.
    pub fn selection(
        &self,
        ui: &ChampionWindow,
        view: Option<&fuze_worker_host::champ_select::View>,
    ) {
        let Some(view) = view else {
            self.actions.context(ui, None);
            self.art.cancel();
            self.state.borrow_mut().heroes.clear();
            ui.invoke_stage_reset();
            if let Ok(mut s) = self._worker.shared.0.lock() {
                s.generation = s.generation.wrapping_add(1);
                s.wanted = None;
                s.reply = None;
            }
            self._timer.stop();
            self.state.borrow_mut().loaded = None;
            ui.set_team(Default::default());
            ui.set_bench(Default::default());
            ui.set_choices(Default::default());
            ui.set_subset_active(false);
            ui.set_local_slot(-1);
            ui.set_expanded_augment(-1);
            ui.set_augments(Default::default());
            ui.set_build_lines(Default::default());
            ui.set_local_id(0);
            ui.set_inspected_games(Default::default());
            ui.set_inspected_id(0);
            ui.set_inspected_name("等待选择英雄".into());
            ui.set_inspected_icon(Default::default());
            ui.set_inspected_win("—".into());
            ui.set_inspected_pick("—".into());
            ui.set_detail_status("选人阶段已结束或连接暂不可用".into());
            return;
        };
        let model =
            |heroes: &[fuze_worker_host::champ_select::Hero]| -> slint::ModelRc<ChampionChoice> {
                Rc::new(VecModel::from(
                    heroes
                        .iter()
                        .map(|h| ChampionChoice {
                            id: h.id as i32,
                            name: h.name.clone().into(),
                            win: rate(h.win),
                            pick: rate(h.pick),
                            games: h.games.map(|n| n.to_string()).unwrap_or_default().into(),
                            icon: self
                                .state
                                .borrow()
                                .icons
                                .get(&h.icon)
                                .cloned()
                                .or_else(|| self.known_icons.get(&(h.id as i32)).cloned())
                                .unwrap_or_default(),
                        })
                        .collect::<Vec<_>>(),
                ))
                .into()
            };
        {
            let mut p = self.state.borrow_mut();
            p.heroes = view
                .team
                .iter()
                .chain(&view.bench)
                .chain(view.choices.iter().flatten())
                .map(|h| (h.id as i32, h.icon.clone()))
                .collect();
            if p.version != view.version {
                p.icons.clear();
            }
            p.version = view.version.clone();
        }
        let version_changed = *self.version.borrow() != view.version;
        if version_changed {
            *self.version.borrow_mut() = view.version.clone();
            if let Ok(mut s) = self._worker.shared.0.lock() {
                s.invalidate = true;
                s.expected = view.version.clone();
                self._worker.shared.1.notify_one();
            }
        }
        let local = view
            .local_slot
            .and_then(|i| view.team.get(i))
            .map(|h| h.id as i32)
            .unwrap_or(0);
        let changed = local != ui.get_local_id();
        let present = view
            .team
            .iter()
            .chain(&view.bench)
            .chain(view.choices.iter().flatten())
            .any(|h| h.id as i32 == ui.get_inspected_id());
        ui.set_team(model(&view.team));
        ui.set_bench(model(&view.bench));
        ui.set_choices(model(view.choices.as_deref().unwrap_or(&[])));
        ui.set_subset_active(view.choices.is_some());
        ui.set_local_slot(view.local_slot.map(|i| i as i32).unwrap_or(-1));
        ui.set_local_id(local);
        if let Some(hero) = view
            .team
            .iter()
            .chain(&view.bench)
            .chain(view.choices.iter().flatten())
            .find(|h| h.id as i32 == ui.get_inspected_id())
        {
            ui.set_inspected_name(hero.name.clone().into());
            ui.set_inspected_win(rate(hero.win));
            ui.set_inspected_pick(rate(hero.pick));
            ui.set_inspected_games(hero.games.map(|n| n.to_string()).unwrap_or_default().into());
        }
        if changed || !present || version_changed {
            if local > 0 {
                ui.invoke_inspect(if changed || !present {
                    local
                } else {
                    ui.get_inspected_id()
                });
                // Automatic hero/context changes start at the five-player overview.
                ui.set_scroll_y(0.);
            } else {
                if let Ok(mut s) = self._worker.shared.0.lock() {
                    s.generation = s.generation.wrapping_add(1);
                    s.wanted = None;
                    s.reply = None;
                }
                self._timer.stop();
                self.actions.context(ui, None);
                self.state.borrow_mut().loaded = None;
                ui.invoke_stage_reset();
                ui.set_inspected_id(0);
                ui.set_detail_champion(0);
                ui.set_detail_failed(false);
                ui.set_inspected_games(Default::default());
                ui.set_inspected_name("等待选择英雄".into());
                ui.set_inspected_icon(Default::default());
                ui.set_inspected_win("—".into());
                ui.set_inspected_pick("—".into());
                ui.set_augments(Default::default());
                ui.set_build_lines(Default::default());
                ui.set_build_routes(Default::default());
                ui.set_detail_status("等待确认自己的英雄".into());
            }
        }
        request_art(ui, &self.state.borrow(), &self.art);
    }
    pub fn options(&self, ui: &ChampionWindow, download: bool) {
        if self.state.borrow().download != download {
            self.state.borrow_mut().download = download;
            request_art(ui, &self.state.borrow(), &self.art);
        }
    }
    pub fn idle(&self) -> bool {
        !self._timer.running()
    }
}
struct Presentation {
    heroes: Vec<(i32, String)>,
    version: String,
    download: bool,
    loaded: Option<Arc<Loaded>>,
    icons: HashMap<String, slint::Image>,
}
fn request_art(ui: &ChampionWindow, p: &Presentation, art: &crate::champion_art::Binding) {
    ui.set_portraits_missing(p.heroes.iter().any(|(_, u)| !p.icons.contains_key(u)));
    let mut urls = p
        .heroes
        .iter()
        .map(|(_, url)| url.clone())
        .collect::<Vec<_>>();
    if let Some(loaded) = &p.loaded {
        let query = ui.get_augment_query().to_lowercase();
        let rarity = ui.get_rarity();
        urls.extend(
            loaded
                .detail
                .augments
                .iter()
                .filter(|a| {
                    (rarity < 0 || a.rarity.map(i32::from) == Some(rarity))
                        && (query.is_empty()
                            || a.name.to_lowercase().contains(&query)
                            || a.description.to_lowercase().contains(&query))
                })
                .skip(ui.get_augment_offset().max(0) as usize)
                .take(24)
                .map(|a| a.icon.clone()),
        );
        urls.extend(
            loaded
                .detail
                .lines
                .iter()
                .flat_map(|l| l.icons.clone())
                .take(64),
        );
    }
    art.request(
        if p.version.is_empty() {
            p.loaded
                .as_ref()
                .map(|l| l.version.clone())
                .unwrap_or_default()
        } else {
            p.version.clone()
        },
        urls,
        p.download,
    );
}
fn rate(value: Option<f64>) -> slint::SharedString {
    value
        .filter(|n| n.is_finite() && (0.0..=1.).contains(n))
        .map(|n| format!("{:.1}%", n * 100.))
        .unwrap_or("—".into())
        .into()
}
fn apply(ui: &ChampionWindow, p: &Presentation) {
    let Some(loaded) = &p.loaded else {
        return;
    };
    let query = ui.get_augment_query().to_lowercase();
    let rarity = ui.get_rarity();
    ui.set_augments(
        Rc::new(VecModel::from(
            loaded
                .detail
                .augments
                .iter()
                .filter(|a| {
                    (rarity < 0 || a.rarity.map(i32::from) == Some(rarity))
                        && (query.is_empty()
                            || a.name.to_lowercase().contains(&query)
                            || a.description.to_lowercase().contains(&query))
                })
                .map(|a| DetailRow {
                    title: a.name.clone().into(),
                    icon_url: a.icon.clone().into(),
                    description: if a.description.is_empty() {
                        "数据源暂未提供说明".into()
                    } else {
                        a.description.clone().into()
                    },
                    win: rate(a.win),
                    pick: rate(a.pick),
                    rarity: a.rarity.map(i32::from).unwrap_or(-1),
                    tier: a.tier.map(|v| format!("T{v}")).unwrap_or_default().into(),
                    icon: p.icons.get(&a.icon).cloned().unwrap_or_default(),
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
}
fn builds(ui: &ChampionWindow, p: &Presentation) {
    let Some(loaded) = &p.loaded else {
        return;
    };
    let routes = loaded
        .detail
        .lines
        .iter()
        .filter(|l| l.text.starts_with("路线 "))
        .map(|l| l.text.clone().into())
        .collect::<Vec<slint::SharedString>>();
    ui.set_build_routes(Rc::new(VecModel::from(routes.clone())).into());
    let chosen = ui.get_build_route().max(0) as usize;
    let mut route = None;
    let mut augments = false;
    let lines = loaded
        .detail
        .lines
        .iter()
        .filter(|l| {
            if l.text.starts_with("海克斯强化") {
                augments = true;
            }
            if l.text.starts_with("路线 ") {
                route = Some(route.map_or(0, |n: usize| n + 1));
            }
            !augments
                && (route.is_none() || route == Some(chosen.min(routes.len().saturating_sub(1))))
        })
        .map(|l| BuildRow {
            text: l.text.clone().into(),
            icons: Rc::new(VecModel::from(
                l.icons
                    .iter()
                    .map(|url| p.icons.get(url).cloned().unwrap_or_default())
                    .collect::<Vec<_>>(),
            ))
            .into(),
            skills: Rc::new(VecModel::from(if l.text.starts_with("1–18") {
                l.text
                    .split('·')
                    .nth(1)
                    .unwrap_or("")
                    .split_whitespace()
                    .filter(|k| matches!(*k, "Q" | "W" | "E" | "R"))
                    .take(18)
                    .map(Into::into)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            }))
            .into(),
        })
        .collect::<Vec<_>>();
    ui.set_build_lines(Rc::new(VecModel::from(lines)).into());
}
pub fn attach(ui: &ChampionWindow, data: &Path) -> Result<Binding, &'static str> {
    attach_roots(
        ui,
        vec![fuze_data::Root {
            path: data.into(),
            source: fuze_data::Source::Bundled,
        }],
    )
}
pub fn attach_roots(
    ui: &ChampionWindow,
    roots: Vec<fuze_data::Root>,
) -> Result<Binding, &'static str> {
    attach_shared(ui, roots, std::sync::Arc::new(Default::default()))
}
pub fn attach_shared(
    ui: &ChampionWindow,
    roots: Vec<fuze_data::Root>,
    store: std::sync::Arc<fuze_worker_host::stage_reference::Store>,
) -> Result<Binding, &'static str> {
    let worker = Rc::new(Worker::new(roots.clone())?);
    let actions = Rc::new(crate::champion_actions::attach(ui));
    let state = Rc::new(RefCell::new(Presentation {
        loaded: None,
        heroes: Vec::new(),
        version: String::new(),
        download: false,
        icons: HashMap::new(),
    }));
    let weak = ui.as_weak();
    let artwork_state = state.clone();
    let art = Rc::new(crate::champion_art::Binding::new(
        ui.get_offline(),
        move |images| {
            let Some(ui) = weak.upgrade() else { return };
            {
                let mut p = artwork_state.borrow_mut();
                if p.icons.len() > 192 {
                    p.icons.clear();
                }
                p.icons.extend(images);
            }
            let p = artwork_state.borrow();
            ui.set_portraits_missing(p.heroes.iter().any(|(_, u)| !p.icons.contains_key(u)));
            let rows = |model: slint::ModelRc<ChampionChoice>| {
                Rc::new(VecModel::from(
                    model
                        .iter()
                        .map(|mut h| {
                            if let Some(image) = p
                                .heroes
                                .iter()
                                .find(|(id, _)| *id == h.id)
                                .and_then(|(_, url)| p.icons.get(url))
                            {
                                h.icon = image.clone();
                            }
                            h
                        })
                        .collect::<Vec<_>>(),
                ))
                .into()
            };
            ui.set_team(rows(ui.get_team()));
            ui.set_bench(rows(ui.get_bench()));
            ui.set_choices(rows(ui.get_choices()));
            if let Some(hero) = ui
                .get_team()
                .iter()
                .chain(ui.get_bench().iter())
                .chain(ui.get_choices().iter())
                .find(|h| h.id == ui.get_inspected_id())
            {
                ui.set_inspected_icon(hero.icon);
            }
            let augments = ui.get_augments();
            for index in 0..augments.row_count() {
                if let Some(mut row) = augments.row_data(index) {
                    if let Some(image) = p.icons.get(row.icon_url.as_str()) {
                        row.icon = image.clone();
                        augments.set_row_data(index, row);
                    }
                }
            }
            builds(&ui, &p);
        },
    )?);
    let retry_art = art.clone();
    let retry_state = state.clone();
    let retry_ui = ui.as_weak();
    ui.on_retry_portraits(move || {
        if let Some(ui) = retry_ui.upgrade() {
            retry_art.retry();
            request_art(&ui, &retry_state.borrow(), &retry_art);
        }
    });
    let timer = Rc::new(slint::Timer::default());
    let reload = timer.clone();
    let weak = ui.as_weak();
    let service = worker.clone();
    let current = state.clone();
    let action_reset = actions.clone();
    let art_reset = art.clone();
    ui.on_inspect(move |id| {
        if id <= 0 {
            return;
        }
        if let Some(ui) = weak.upgrade() {
            let found: Option<ChampionChoice> = ui
                .get_team()
                .iter()
                .chain(ui.get_bench().iter())
                .chain(ui.get_choices().iter())
                .find(|h| h.id == id);
            if let Some(hero) = found {
                let retry = ui.get_detail_failed();
                action_reset.context(&ui, None);
                ui.invoke_stage_reset();
                current.borrow_mut().loaded = None;
                art_reset.cancel();
                ui.set_inspected_id(id);
                ui.set_build_route(0);
                ui.set_build_routes(Default::default());
                ui.set_detail_champion(0);
                ui.set_expanded_augment(-1);
                ui.set_inspected_name(hero.name);
                ui.set_inspected_win(hero.win);
                ui.set_inspected_pick(hero.pick);
                ui.set_inspected_games(hero.games);
                ui.set_inspected_icon(hero.icon);
                ui.set_augments(Default::default());
                ui.set_build_lines(Default::default());
                ui.set_detail_failed(false);
                ui.set_detail_status("正在读取本地英雄数据…".into());
                ui.set_scroll_y(-ui.get_detail_offset());
                service.request(id as u32, retry);
                reload.restart();
            }
        }
    });
    let weak = ui.as_weak();
    let current = state.clone();
    let art_filter = art.clone();
    ui.on_filter(move |query, rarity| {
        if let Some(ui) = weak.upgrade() {
            ui.set_augment_query(query);
            ui.set_rarity(rarity);
            ui.set_expanded_augment(-1);
            ui.set_augment_offset(0);
            request_art(&ui, &current.borrow(), &art_filter);
            apply(&ui, &current.borrow());
        }
    });
    let weak = ui.as_weak();
    let service = worker.clone();
    let current = state.clone();
    let route_weak = ui.as_weak();
    let route_state = state.clone();
    ui.on_route_changed(move |route| {
        if let Some(ui) = route_weak.upgrade() {
            ui.set_build_route(route);
            builds(&ui, &route_state.borrow());
        }
    });
    let art_request = art.clone();
    let art_state = state.clone();
    let weak_art = ui.as_weak();
    let debounce = Rc::new(slint::Timer::default());
    ui.on_artwork_refresh(move || {
        let weak = weak_art.clone();
        let state = art_state.clone();
        let art = art_request.clone();
        debounce.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(150),
            move || {
                if let Some(ui) = weak.upgrade() {
                    request_art(&ui, &state.borrow(), &art);
                }
            },
        );
    });
    let art_result = art.clone();
    let action_result = actions.clone();
    let tick = Rc::downgrade(&timer);
    timer.start(
        slint::TimerMode::Repeated,
        // Active only until this click's local result arrives; zero idle polling.
        Duration::from_millis(16),
        move || {
            let reply = service
                .shared
                .0
                .lock()
                .ok()
                .and_then(|mut s| s.reply.take());
            let Some((_, reply)) = reply else {
                return;
            };
            if let Some(timer) = tick.upgrade() {
                timer.stop();
            }
            let Some(ui) = weak.upgrade() else {
                return;
            };
            match reply {
                Ok(loaded) => {
                    ui.set_detail_failed(false);
                    ui.set_detail_version(loaded.version.clone().into());
                    ui.set_detail_champion(loaded.id as i32);
                    ui.set_detail_status(
                        format!(
                            "{} · 整体胜率参考\n实际领取轮次未确认，不自动使用阶段排序。",
                            loaded.version
                        )
                        .into(),
                    );
                    action_result.context(
                        &ui,
                        Some((
                            loaded.id,
                            loaded.version.clone(),
                            Arc::new(loaded.detail.clone()),
                        )),
                    );
                    current.borrow_mut().loaded = Some(loaded);
                    let p = current.borrow();
                    request_art(&ui, &p, &art_result);
                    apply(&ui, &p);
                    builds(&ui, &p);
                }
                Err(error) => {
                    current.borrow_mut().loaded = None;
                    ui.set_detail_failed(true);
                    let reason = match error {
                        "detail-version-mismatch" => "详情版本与当前英雄概览不一致",
                        "detail-champion-missing" => "本地数据暂未提供这位英雄的详情",
                        "data-no-complete-local-version" => "当前版本的本地数据不完整或暂不可读",
                        _ => "本地英雄详情加载失败",
                    };
                    ui.set_detail_status(
                        format!("{reason}。可重试读取；仍失败时请在设置中检查数据更新。").into(),
                    );
                }
            }
        },
    );
    if ui.get_inspected_id() > 0 {
        worker.request(ui.get_inspected_id() as u32, false);
    } else {
        timer.stop();
    }
    Ok(Binding {
        _stage: crate::stage_bridge::attach(ui, roots, ui.get_offline(), store),
        actions,
        art,
        _worker: worker,
        _timer: timer,
        state,
        version: RefCell::new(String::new()),
        known_icons: ui
            .get_team()
            .iter()
            .chain(ui.get_bench().iter())
            .chain(ui.get_choices().iter())
            .map(|h| (h.id, h.icon))
            .collect(),
    })
}
