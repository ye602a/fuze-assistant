//! Opt-in host controller. LCU IO is isolated from capture/OCR and the window thread.
use crate::{pipeline::Pipeline, recovery::Supervisor, Result};
use fuze_lcu::{context::Context, discovery::AutoClient};
use fuze_runtime::session::Phase;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

fn loading_handoff(old: Context, new: Context) -> bool {
    old.phase == Phase::Loading
        && new.phase == Phase::InProgress
        && old.supported
        && new.supported
        && old.champion.is_some()
        && old.champion == new.champion
        && old.same_game(&new)
}

struct Observation {
    journal: Option<crate::local_records::Binding>,
    locale: Option<fuze_lcu::locale::Locale>,
    state: fuze_lcu::reconnect::State,
    context: Option<Context>,
    root: Option<PathBuf>,
    revision: u64,
    updated: Instant,
    cancellation: Option<fuze_native_overlay::native::Cancellation>,
}
impl Observation {
    fn set_locale(&mut self, locale: Option<fuze_lcu::locale::Locale>) {
        if self.locale != locale {
            self.locale = locale;
            if let Some(c) = self.cancellation.take() {
                c.cancel();
            }
            self.revision = self.revision.saturating_add(1);
        }
    }
    fn publish(&mut self, context: Option<Context>, root: Option<PathBuf>) {
        if self.context != context || self.root != root {
            // Only the forward loading -> playing handoff may retain the warmed worker.
            let handoff = self.root == root
                && self
                    .context
                    .zip(context)
                    .is_some_and(|(old, new)| loading_handoff(old, new));
            if !handoff {
                if let Some(c) = self.cancellation.take() {
                    c.cancel();
                }
                self.revision = self.revision.saturating_add(1);
            }
            self.context = context;
            self.root = root;
        }
        self.updated = Instant::now();
    }
}
/// No live discovery happens until start is explicitly called by a host entry point.
#[derive(Clone)]
pub struct MonitorControl {
    wake: Option<thread::Thread>,
    enabled: Arc<AtomicBool>,
    latest: Arc<Mutex<Observation>>,
}
impl MonitorControl {
    pub(crate) fn diagnostic_context(&self) -> (u64, bool) {
        self.latest
            .lock()
            .map(|s| {
                (
                    s.revision,
                    s.updated.elapsed() < Duration::from_secs(8)
                        && s.context
                            .is_some_and(|c| c.supported && c.phase == Phase::InProgress),
                )
            })
            .unwrap_or((0, false))
    }

    pub fn enable(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
        if let Some(wake) = self.wake.as_ref() {
            wake.unpark();
        }
        if !enabled {
            if let Ok(mut latest) = self.latest.lock() {
                if let Some(c) = latest.cancellation.take() {
                    c.cancel();
                }
                latest.journal = None;
                latest.publish(None, None);
                latest.state = fuze_lcu::reconnect::State::Paused;
            }
        }
    }
}
pub struct Monitor {
    control: MonitorControl,
    latest: Arc<Mutex<Observation>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Monitor {
    pub fn playing_champion(&self) -> Option<u32> {
        let latest = self.latest.lock().ok()?;
        if latest.updated.elapsed() >= Duration::from_secs(8)
            || !matches!(
                latest.state,
                fuze_lcu::reconnect::State::Connected(Phase::InProgress)
            )
        {
            return None;
        }
        let context = latest.context?;
        context.supported.then_some(context.champion).flatten()
    }
    pub fn data_locale(&self) -> Option<fuze_data::Locale> {
        let latest = self.latest.lock().ok()?;
        if latest.updated.elapsed() >= Duration::from_secs(8)
            || !matches!(latest.state, fuze_lcu::reconnect::State::Connected(_))
        {
            return None;
        }
        Some(match latest.locale? {
            fuze_lcu::locale::Locale::ZhCn => fuze_data::Locale::ZhCn,
            fuze_lcu::locale::Locale::ZhTw => fuze_data::Locale::ZhTw,
            fuze_lcu::locale::Locale::EnUs => fuze_data::Locale::EnUs,
        })
    }
    pub fn selection(&self) -> Option<fuze_lcu::context::Selection> {
        let latest = self.latest.lock().ok()?;
        if latest.updated.elapsed() >= Duration::from_secs(8)
            || !matches!(
                latest.state,
                fuze_lcu::reconnect::State::Connected(Phase::ChampSelect)
            )
        {
            return None;
        }
        latest.context?.selection
    }
    pub fn control(&self) -> MonitorControl {
        self.control.clone()
    }
    pub fn verified_age(&self) -> Option<u64> {
        let s = self.latest.lock().ok()?;
        matches!(s.state, fuze_lcu::reconnect::State::Connected(_))
            .then(|| s.updated.elapsed().as_secs())
    }
    pub fn state(&self) -> Option<fuze_lcu::reconnect::State> {
        self.latest.lock().ok().map(|s| {
            if matches!(s.state, fuze_lcu::reconnect::State::Connected(_))
                && s.updated.elapsed() >= Duration::from_secs(8)
            {
                fuze_lcu::reconnect::State::Retry(fuze_lcu::Error::Transport)
            } else {
                s.state
            }
        })
    }
    pub fn start() -> Result<Self> {
        Self::start_enabled(true)
    }
    pub fn start_enabled(initial: bool) -> Result<Self> {
        Self::start_with_fallback(initial, None)
    }
    pub fn start_with_fallback(initial: bool, manual: Option<PathBuf>) -> Result<Self> {
        Self::start_with_records(initial, manual, false)
    }
    pub(crate) fn start_with_records(
        initial: bool,
        manual: Option<PathBuf>,
        records: bool,
    ) -> Result<Self> {
        let latest = Arc::new(Mutex::new(Observation {
            journal: None,
            locale: None,
            state: fuze_lcu::reconnect::State::Waiting,
            context: None,
            root: None,
            revision: 0,
            updated: Instant::now(),
            cancellation: None,
        }));
        let enabled = Arc::new(AtomicBool::new(initial));
        let mut control = MonitorControl {
            wake: None,
            enabled: enabled.clone(),
            latest: latest.clone(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopped) = (latest.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("fuze-lcu-control".into())
            .spawn(move || {
                let mut client = AutoClient::with_fallback(manual);
                let mut was_enabled = true;
                let mut journal: Option<crate::local_records::Binding> = None;
                let mut journal_retry = Instant::now();
                while !stopped.load(Ordering::Acquire) {
                    if !enabled.load(Ordering::Acquire) {
                        journal = None;
                        if was_enabled {
                            client.pause();
                        }
                        was_enabled = false;
                        thread::park_timeout(Duration::from_secs(1));
                        continue;
                    }
                    if !was_enabled {
                        client.resume();
                        was_enabled = true;
                    }
                    {
                        let state = client.poll();
                        let context = client.context();
                        let root = client.game_root().map(PathBuf::from);
                        let locale = client.locale();
                        let game = context.and_then(|c| c.observed_game_id());
                        if journal.as_ref().map(|j| j.game) != game {
                            journal = None;
                        }
                        if records
                            && game.is_some()
                            && journal.is_none()
                            && Instant::now() >= journal_retry
                        {
                            journal_retry = Instant::now() + Duration::from_secs(30);
                            journal = client.journal_owner().ok().and_then(|(owner, id)| {
                                crate::local_records::Binding::from_owner(&owner, id).ok()
                            });
                        }
                        if let Ok(mut latest) = shared.lock() {
                            if !enabled.load(Ordering::Acquire) {
                                continue;
                            }
                            latest.journal = journal.clone();
                            latest.set_locale(locale);
                            latest.publish(context, root);
                            if let Some(verified) = client.verified_at() {
                                latest.updated = verified;
                            }
                            latest.state = state;
                        }
                    }
                    let wait = client
                        .wake_at()
                        .unwrap_or(client.now_ms().saturating_add(1000))
                        .saturating_sub(client.now_ms())
                        .clamp(20, 1000);
                    thread::park_timeout(Duration::from_millis(wait));
                }
                client.pause();
            })
            .map_err(|_| "lcu-thread-start")?;
        control.wake = Some(thread.thread().clone());
        Ok(Self {
            control,
            latest,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut latest) = self.latest.lock() {
            if let Some(c) = latest.cancellation.take() {
                c.cancel();
            }
        }
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub icon_downloads: bool,
    pub level_gate: bool,
    pub augment_overlay: bool,
    pub roots: Vec<fuze_data::Root>,
    pub locale: Option<fuze_data::Locale>,
    pub worker: PathBuf,
    pub ort: PathBuf,
    pub model: PathBuf,
    pub dictionary: PathBuf,
    pub backend: String,
}
impl Config {
    /// Packaged-path check only. No discovery, LCU, windows, capture, or user cache reads.
    pub fn check_bundle(&self) -> Result<()> {
        for file in [&self.worker, &self.ort, &self.model, &self.dictionary] {
            let size = std::fs::metadata(file)
                .map_err(|_| "bundle-file-missing")?
                .len();
            if size == 0 || size > 128 * 1024 * 1024 {
                return Err("bundle-file-limit");
            }
        }
        let roots: Vec<_> = self
            .roots
            .iter()
            .filter(|r| r.source == fuze_data::Source::Bundled)
            .map(|r| fuze_data::Root {
                path: r.path.clone(),
                source: r.source,
            })
            .collect();
        for locale in [
            fuze_data::Locale::ZhCn,
            fuze_data::Locale::ZhTw,
            fuze_data::Locale::EnUs,
        ] {
            let data = fuze_data::prepare(&roots, locale, fuze_data::Scope::Full)?;
            if locale == fuze_data::Locale::ZhCn {
                let catalog = fuze_contract::wire::encode_catalog(data.catalog())
                    .map_err(|_| "catalog-encode")?;
                let args = vec![
                    self.ort.to_string_lossy().into(),
                    self.model.to_string_lossy().into(),
                    self.dictionary.to_string_lossy().into(),
                    "-".into(),
                    "cpu".into(),
                ];
                let mut worker = Supervisor::start_with_catalog(
                    &self.worker,
                    &args,
                    Duration::from_secs(30),
                    Some(Arc::new(catalog)),
                )?;
                worker.stop()?;
                if !worker.reaped() {
                    return Err("bundle-worker-not-reaped");
                }
            }
        }
        Ok(())
    }
    fn prepare(&self, champion: u32, locale: fuze_data::Locale) -> Result<Pipeline> {
        let mut data =
            fuze_data::prepare(&self.roots, locale, fuze_data::Scope::Champion(champion))?;
        let (supplement, _) = crate::recognition_sources::load(&self.roots)?;
        data.supplement_recognition(&supplement);
        let catalog =
            fuze_contract::wire::encode_catalog(data.catalog()).map_err(|_| "catalog-encode")?;
        let args = vec![
            self.ort.to_string_lossy().into(),
            self.model.to_string_lossy().into(),
            self.dictionary.to_string_lossy().into(),
            "-".into(),
            self.backend.clone(),
        ];
        let worker = Supervisor::start_with_catalog(
            &self.worker,
            &args,
            Duration::from_secs(30),
            Some(Arc::new(catalog)),
        )?;
        let mut pipeline = if self.augment_overlay {
            Pipeline::managed(worker, &data, champion)?
        } else {
            Pipeline::windowless(worker, &data, champion)?
        };
        if self.augment_overlay {
            pipeline.art = crate::overlay_art::Art::new(&data, false).ok();
        }
        pipeline.overlay.locale(locale.code())?;
        Ok(pipeline)
    }
}
#[derive(Default)]
struct Retry {
    failures: u8,
    next: u64,
}
impl Retry {
    fn fail(&mut self, now: u64) {
        self.failures = self.failures.saturating_add(1);
        self.next = now.saturating_add(
            [1000, 2000, 5000, 10000, 30000][usize::from(self.failures - 1).min(4)],
        );
    }
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PipelineStatus {
    pub overlay: Option<std::sync::Arc<crate::overlay_frame::Frame>>,
    pub schedule: Option<(fuze_runtime::gate::Mode, u64, Option<u8>)>,
    pub recent_tiers: [Option<u64>; 3],
    pub recent: Option<std::sync::Arc<[fuze_contract::Slot; 3]>>,
    pub skipped_frames: u64,
    pub analyses: u64,
    pub analysis_ms: Option<f64>,
    pub capture: &'static str,
    pub backend: Option<fuze_contract::wire::ActiveBackend>,
    pub dataset: Option<String>,
    pub error: Option<&'static str>,
    pub frame_age: Option<u64>,
    pub level: Option<u8>,
}
pub struct Controller {
    recent_tiers: [Option<u64>; 3],
    analyses: u64,
    analysis_ms: Option<f64>,
    recent: Option<std::sync::Arc<[fuze_contract::Slot; 3]>>,
    level_reader: Option<crate::live_level::Reader>,
    records: crate::local_records::Journal,
    last_frame: Option<Instant>,
    last_error: Option<&'static str>,
    config: Config,
    pipeline: Option<Pipeline>,
    revision: u64,
    retry: Retry,
    epoch: Instant,
}
impl Controller {
    pub(crate) fn usage(&self) -> Option<(u32, u64, u64)> {
        self.pipeline.as_ref().and_then(|p| p.worker.usage())
    }
    pub fn enable_local_records(&mut self) {
        if let Some(root) = std::env::var_os("LOCALAPPDATA") {
            self.records = crate::local_records::Journal::open(
                std::path::PathBuf::from(root).join("FuzeRustPreview/observations-v2.json"),
            );
        }
    }
    pub fn records(&self) -> (Vec<crate::local_records::Row>, Option<&'static str>) {
        (self.records.recent(), self.records.error)
    }
    pub fn unbound_records(&self) -> crate::local_records::UnboundSnapshot {
        self.records.unbound_recent()
    }
    pub fn status(&self) -> PipelineStatus {
        PipelineStatus {
            overlay: self.pipeline.as_ref().and_then(|p| p.frames.current()),
            skipped_frames: self
                .pipeline
                .as_ref()
                .map(|p| p.skipped_frames)
                .unwrap_or(0),
            schedule: self.pipeline.as_ref().map(|p| p.scheduling()),
            recent_tiers: self.recent_tiers,
            analyses: self.analyses,
            analysis_ms: self.analysis_ms,
            recent: self.recent.clone(),
            capture: self
                .pipeline
                .as_ref()
                .map(|p| p.capture_state)
                .unwrap_or("waiting"),
            backend: self.pipeline.as_ref().and_then(|p| p.worker.backend()),
            dataset: self.pipeline.as_ref().map(|p| p.dataset().to_owned()),
            error: self.last_error,
            frame_age: self.last_frame.map(|t| t.elapsed().as_secs()),
            level: self
                .level_reader
                .as_ref()
                .and_then(|r| r.sample())
                .map(|s| s.0),
        }
    }
    pub fn suspend(&mut self) {
        self.clear();
        self.last_error = None;
        self.retry = Retry::default();
    }
    pub fn idle_wait(&self) -> Duration {
        let now = self.epoch.elapsed().as_millis() as u64;
        if self.pipeline.is_none() || now < self.retry.next {
            Duration::from_millis(1000)
        } else {
            // Respect the OCR gate, while checking LCU phase/level changes at least once a second.
            let delay = self
                .pipeline
                .as_ref()
                .and_then(|p| p.wake_at().map(|at| at.saturating_sub(p.now())))
                .unwrap_or(1000);
            Duration::from_millis(delay.clamp(20, 1000))
        }
    }
    pub fn new(config: Config) -> Self {
        Self {
            recent_tiers: [None; 3],
            analyses: 0,
            analysis_ms: None,
            recent: None,
            level_reader: None,
            records: Default::default(),
            config,
            last_frame: None,
            last_error: None,
            pipeline: None,
            revision: u64::MAX,
            retry: Retry::default(),
            epoch: Instant::now(),
        }
    }
    fn clear(&mut self) {
        self.recent_tiers = [None; 3];
        self.recent = None;
        if let Some(reader) = &self.level_reader {
            reader.enable(false);
        }
        self.records.finish();
        self.last_frame = None;
        if let Some(p) = self.pipeline.take() {
            p.cancellation().cancel();
            drop(p);
        }
    }
    /// One bounded unit of work; caller yields between ticks and never runs this on UI.
    pub fn tick(&mut self, monitor: &Monitor) -> Result<()> {
        let (context, root, revision, fresh, detected_locale, journal) = {
            let latest = monitor.latest.lock().map_err(|_| "lcu-observation")?;
            (
                latest.context,
                latest.root.clone(),
                latest.revision,
                latest.updated.elapsed() < Duration::from_secs(8),
                latest.locale,
                latest.journal.clone(),
            )
        };
        if revision != self.revision {
            if fresh && context.is_some_and(|c| c.phase == Phase::PostGame) {
                self.records.verified_end();
            }
            self.clear();
            self.revision = revision;
            self.last_error = None;
            self.retry = Retry::default();
        }
        self.records.bind(if fresh { journal } else { None });
        let active = context.filter(|c| {
            fresh
                && matches!(c.phase, Phase::Loading | Phase::InProgress)
                && c.supported
                && c.champion.is_some()
        });
        let (Some(context), Some(root)) = (active, root) else {
            self.clear();
            return Ok(());
        };
        let now = self.epoch.elapsed().as_millis() as u64;
        let locale = self.config.locale.or_else(|| {
            detected_locale.map(|l| match l {
                fuze_lcu::locale::Locale::ZhCn => fuze_data::Locale::ZhCn,
                fuze_lcu::locale::Locale::ZhTw => fuze_data::Locale::ZhTw,
                fuze_lcu::locale::Locale::EnUs => fuze_data::Locale::EnUs,
            })
        });
        if now < self.retry.next {
            return Ok(());
        }
        let result: Result<()> = (|| {
            if self.pipeline.is_none() {
                let pipeline = self.config.prepare(
                    context.champion.ok_or("champion-missing")?,
                    locale.ok_or("locale-unavailable")?,
                )?;
                let mut latest = monitor.latest.lock().map_err(|_| "lcu-observation")?;
                // Loading model/data may overlap a phase or hero change; never attach that stale host.
                if latest.revision != revision || latest.updated.elapsed() >= Duration::from_secs(8)
                {
                    pipeline.cancellation().cancel();
                    return Ok(());
                }
                latest.cancellation = Some(pipeline.cancellation());
                self.pipeline = Some(pipeline);
            }
            // Re-read after model warm-up: the phase may have advanced without replacing the worker.
            let phase = {
                let latest = monitor.latest.lock().map_err(|_| "lcu-observation")?;
                if latest.revision != revision || latest.updated.elapsed() >= Duration::from_secs(8)
                {
                    self.clear();
                    return Ok(());
                }
                latest
                    .context
                    .map(|c| c.phase)
                    .ok_or("lcu-context-missing")?
            };
            let pipeline = self.pipeline.as_mut().ok_or("pipeline-missing")?;
            if let Some(art) = pipeline.art.as_mut() {
                art.download = self.config.icon_downloads;
                art.poll(&pipeline.overlay);
            }
            pipeline.phase(phase)?;
            if phase == Phase::Loading {
                pipeline.capture_state = "prewarmed";
                return Ok(()); // No window discovery, capture or level requests during loading.
            }
            if phase != Phase::InProgress {
                return Ok(());
            }
            if self.config.level_gate && self.level_reader.is_none() {
                self.level_reader = crate::live_level::Reader::start().ok();
            }
            let sample = self.level_reader.as_ref().and_then(|r| {
                r.enable(self.config.level_gate);
                r.sample()
            });
            let pipeline = self.pipeline.as_mut().ok_or("pipeline-missing")?;
            if !pipeline.has_capture() {
                let target = fuze_capture::target::discover(&root)?.ok_or("game-window-waiting")?;
                pipeline.capture_game(target)?;
            }
            if let Some((level, at)) = sample {
                pipeline.level_sample(level, at.elapsed().as_millis() as u64);
            } else {
                pipeline.clear_level();
            }
            if let Some(delivery) = pipeline.tick_wgc()? {
                self.analyses = self.analyses.saturating_add(1);
                self.records.metrics(
                    delivery.total_ms,
                    delivery.ocr_ms,
                    delivery
                        .snapshot
                        .slots
                        .iter()
                        .any(|s| matches!(s, fuze_contract::Slot::Unreadable)),
                );
                self.analysis_ms = Some(delivery.total_ms);
                if delivery.title_kind == fuze_contract::wire::TitleKind::Candidate
                    && delivery
                        .snapshot
                        .slots
                        .iter()
                        .any(|s| matches!(s, fuze_contract::Slot::Known { .. }))
                {
                    self.recent = Some(std::sync::Arc::new(delivery.snapshot.slots.clone()));
                    self.recent_tiers = std::array::from_fn(|i| match delivery.snapshot.slots[i] {
                        fuze_contract::Slot::Known { id, .. } => pipeline.tier(id),
                        _ => None,
                    });
                }
                self.records
                    .visibility(delivery.display_visible, sample.map(|v| v.0));
                if delivery.title_kind == fuze_contract::wire::TitleKind::Candidate
                    && !delivery.retained
                {
                    self.records
                        .observe(context.champion.unwrap_or(0), &delivery.snapshot.slots);
                }
                pipeline
                    .overlay
                    .visible(delivery.display_visible && self.config.augment_overlay)?;
                self.last_frame = Some(Instant::now());
                self.last_error = None;
                self.retry.failures = 0;
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.last_error = Some(error);
                self.retry.fail(self.epoch.elapsed().as_millis() as u64);
                if !matches!(
                    error,
                    "capture-frame"
                        | "capture-atlas"
                        | "capture-start"
                        | "game-window-waiting"
                        | "game-window-ambiguous"
                        | "game-window-discovery"
                        | "worker-recovered-use-next-frame"
                ) {
                    self.clear();
                }
                Err(error)
            }
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_context_requires_fresh_connected_detection() {
        let latest = Arc::new(Mutex::new(Observation {
            journal: None,
            locale: Some(fuze_lcu::locale::Locale::EnUs),
            state: fuze_lcu::reconnect::State::Connected(Phase::ChampSelect),
            context: None,
            root: None,
            revision: 0,
            updated: Instant::now(),
            cancellation: None,
        }));
        let monitor = Monitor {
            control: MonitorControl {
                wake: None,
                enabled: Arc::new(AtomicBool::new(false)),
                latest: latest.clone(),
            },
            latest: latest.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        assert_eq!(monitor.data_locale(), Some(fuze_data::Locale::EnUs));
        latest.lock().unwrap().updated = Instant::now() - Duration::from_secs(9);
        assert_eq!(monitor.data_locale(), None);
        {
            let mut s = latest.lock().unwrap();
            s.updated = Instant::now();
            s.state = fuze_lcu::reconnect::State::Waiting;
        }
        assert_eq!(monitor.data_locale(), None);
        {
            let mut s = latest.lock().unwrap();
            s.state = fuze_lcu::reconnect::State::Connected(Phase::ChampSelect);
            s.set_locale(Some(fuze_lcu::locale::Locale::ZhTw));
        }
        assert_eq!(monitor.data_locale(), Some(fuze_data::Locale::ZhTw));
        latest.lock().unwrap().set_locale(None);
        assert_eq!(monitor.data_locale(), None);
    }
    #[test]
    fn loading_handoff_reuses_only_same_game_hero_root_and_locale() {
        let context = |phase, hero, game| {
            let name = if phase == Phase::Loading {
                "GameStart"
            } else {
                "InProgress"
            };
            let flow = format!(
                r#"{{"phase":"{name}","gameData":{{"queue":{{"id":2400}},"gameId":{game},"teamOne":[{{"summonerId":7,"championId":{hero}}}],"teamTwo":[]}}}}"#
            );
            fuze_lcu::context::parse(phase, flow.as_bytes(), br#"{"summonerId":7}"#).unwrap()
        };
        let loading = context(Phase::Loading, 8, 1);
        let playing = context(Phase::InProgress, 8, 1);
        assert!(loading_handoff(loading, playing));
        assert!(!loading_handoff(playing, loading));
        assert!(!loading_handoff(loading, context(Phase::InProgress, 9, 1)));
        assert!(!loading_handoff(loading, context(Phase::InProgress, 8, 2)));
        let window =
            fuze_native_overlay::native::HiddenOverlay::start(1, "zh-CN@fixture-v1".into())
                .unwrap();
        let cancellation = window.cancellation();
        let mut observation = Observation {
            journal: None,
            locale: None,
            state: fuze_lcu::reconnect::State::Waiting,
            context: Some(loading),
            root: Some(PathBuf::from("fixture-root")),
            revision: 1,
            updated: Instant::now(),
            cancellation: Some(cancellation.clone()),
        };
        observation.publish(Some(playing), Some(PathBuf::from("fixture-root")));
        assert_eq!(observation.revision, 1);
        assert!(!cancellation.cancelled());
        observation.publish(Some(playing), Some(PathBuf::from("changed-root")));
        assert_eq!(observation.revision, 2);
        assert!(cancellation.cancelled());
    }
    #[test]
    fn hero_game_and_disconnect_cancel_old_window_without_live_discovery() {
        use fuze_contract::{ActualStage, Slot, Snapshot};
        use fuze_native_overlay::native::HiddenOverlay;
        let context = |hero, game| {
            let flow = format!(
                r#"{{"phase":"InProgress","gameData":{{"queue":{{"id":2400}},"gameId":{game},"teamOne":[{{"summonerId":7,"championId":{hero}}}],"teamTwo":[]}}}}"#
            );
            fuze_lcu::context::parse(Phase::InProgress, flow.as_bytes(), br#"{"summonerId":7}"#)
                .unwrap()
        };
        let first = context(8, 1);
        let mut observation = Observation {
            journal: None,
            locale: None,
            state: fuze_lcu::reconnect::State::Waiting,
            context: Some(first),
            root: None,
            revision: 1,
            updated: Instant::now(),
            cancellation: None,
        };
        for next in [Some(context(9, 1)), Some(context(9, 2)), None] {
            let window = HiddenOverlay::start(1, "zh-CN@fixture-v1".into()).unwrap();
            let snapshot = |sequence| Snapshot {
                schema_version: 1,
                generation: 1,
                sequence,
                dataset_version: "zh-CN@fixture-v1".into(),
                selection_confirmed: false,
                actual_stage: ActualStage::Unknown,
                slots: std::array::from_fn(|_| Slot::Unreadable),
            };
            assert!(window.update(snapshot(1)).unwrap());
            let cancellation = window.cancellation();
            observation.cancellation = Some(cancellation.clone());
            observation.publish(observation.context, None);
            assert!(!cancellation.cancelled());
            observation.publish(next, None);
            assert!(cancellation.cancelled());
            assert!(!window.update(snapshot(2)).unwrap());
        }
        assert_eq!(observation.revision, 4);
        let window = HiddenOverlay::start(1, "zh-CN@fixture-v1".into()).unwrap();
        let cancellation = window.cancellation();
        observation.cancellation = Some(cancellation.clone());
        observation.set_locale(None);
        assert!(!cancellation.cancelled());
        observation.set_locale(Some(fuze_lcu::locale::Locale::EnUs));
        assert!(cancellation.cancelled());
        assert_eq!(observation.revision, 5);
        observation.set_locale(Some(fuze_lcu::locale::Locale::EnUs));
        assert_eq!(observation.revision, 5);
        observation.set_locale(None);
        assert_eq!(observation.revision, 6);
    }
    #[test]
    fn latest_observation_drops_old_context_and_backoff_is_bounded() {
        let mut observation = Observation {
            journal: None,
            locale: None,
            state: fuze_lcu::reconnect::State::Waiting,
            context: None,
            root: None,
            revision: 0,
            updated: Instant::now(),
            cancellation: None,
        };
        observation.publish(Some(Context::idle(Phase::ClientReady)), None);
        assert_eq!(observation.revision, 1);
        observation.publish(Some(Context::idle(Phase::ClientReady)), None);
        assert_eq!(observation.revision, 1);
        observation.publish(None, None);
        assert_eq!(observation.revision, 2);
        assert!(observation.context.is_none());
        let mut retry = Retry::default();
        retry.fail(10);
        assert_eq!(retry.next, 1010);
        for _ in 0..50 {
            retry.fail(10);
        }
        assert_eq!(retry.next, 30010);
    }
}
