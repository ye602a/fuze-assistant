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

struct Observation {
    state: fuze_lcu::reconnect::State,
    context: Option<Context>,
    root: Option<PathBuf>,
    revision: u64,
    updated: Instant,
    cancellation: Option<fuze_native_overlay::native::Cancellation>,
}
impl Observation {
    fn publish(&mut self, context: Option<Context>, root: Option<PathBuf>) {
        if self.context != context || self.root != root {
            if let Some(c) = self.cancellation.take() {
                c.cancel();
            }
            self.revision = self.revision.saturating_add(1);
            self.context = context;
            self.root = root;
        }
        self.updated = Instant::now();
    }
}
/// No live discovery happens until start is explicitly called by a host entry point.
pub struct Monitor {
    latest: Arc<Mutex<Observation>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Monitor {
    pub fn state(&self) -> Option<fuze_lcu::reconnect::State> {
        self.latest.lock().ok().map(|s| s.state)
    }
    pub fn start() -> Result<Self> {
        let latest = Arc::new(Mutex::new(Observation {
            state: fuze_lcu::reconnect::State::Waiting,
            context: None,
            root: None,
            revision: 0,
            updated: Instant::now(),
            cancellation: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopped) = (latest.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("fuze-lcu-control".into())
            .spawn(move || {
                let mut client = AutoClient::default();
                while !stopped.load(Ordering::Acquire) {
                    if client.wake_at().is_some_and(|at| client.now_ms() >= at) {
                        let state = client.poll();
                        let context = client.context();
                        let root = client.game_root().map(PathBuf::from);
                        if let Ok(mut latest) = shared.lock() {
                            latest.publish(context, root);
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
        Ok(Self {
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

pub struct Config {
    pub roots: Vec<fuze_data::Root>,
    pub locale: fuze_data::Locale,
    pub worker: PathBuf,
    pub ort: PathBuf,
    pub model: PathBuf,
    pub dictionary: PathBuf,
    pub backend: String,
}
impl Config {
    fn prepare(&self, champion: u32) -> Result<Pipeline> {
        let data = fuze_data::prepare(
            &self.roots,
            self.locale,
            fuze_data::Scope::Champion(champion),
        )?;
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
        let mut pipeline = Pipeline::new(worker, &data, champion)?;
        pipeline.phase(Phase::InProgress)?;
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
pub struct Controller {
    config: Config,
    pipeline: Option<Pipeline>,
    revision: u64,
    retry: Retry,
    epoch: Instant,
}
impl Controller {
    pub fn idle_wait(&self) -> Duration {
        let now = self.epoch.elapsed().as_millis() as u64;
        if self.pipeline.is_none() || now < self.retry.next {
            Duration::from_millis(1000)
        } else {
            Duration::from_millis(100)
        }
    }
    pub fn new(config: Config) -> Self {
        Self {
            config,
            pipeline: None,
            revision: u64::MAX,
            retry: Retry::default(),
            epoch: Instant::now(),
        }
    }
    fn clear(&mut self) {
        if let Some(p) = self.pipeline.take() {
            p.cancellation().cancel();
            drop(p);
        }
    }
    /// One bounded unit of work; caller yields between ticks and never runs this on UI.
    pub fn tick(&mut self, monitor: &Monitor) -> Result<()> {
        let (context, root, revision, fresh) = {
            let latest = monitor.latest.lock().map_err(|_| "lcu-observation")?;
            (
                latest.context,
                latest.root.clone(),
                latest.revision,
                latest.updated.elapsed() < Duration::from_secs(8),
            )
        };
        if revision != self.revision {
            self.clear();
            self.revision = revision;
            self.retry = Retry::default();
        }
        let active = context.filter(|c| {
            fresh && c.phase == Phase::InProgress && c.supported && c.champion.is_some()
        });
        let (Some(context), Some(root)) = (active, root) else {
            self.clear();
            return Ok(());
        };
        let now = self.epoch.elapsed().as_millis() as u64;
        if now < self.retry.next {
            return Ok(());
        }
        let result: Result<()> = (|| {
            if self.pipeline.is_none() {
                let pipeline = self
                    .config
                    .prepare(context.champion.ok_or("champion-missing")?)?;
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
            let pipeline = self.pipeline.as_mut().ok_or("pipeline-missing")?;
            if !pipeline.has_capture() {
                let target = fuze_capture::target::discover(&root)?.ok_or("game-window-waiting")?;
                pipeline.capture_game(target)?;
            }
            if let Some(delivery) = pipeline.tick_wgc()? {
                let known = delivery
                    .snapshot
                    .slots
                    .iter()
                    .any(|s| matches!(s, fuze_contract::Slot::Known { .. }));
                pipeline.overlay.visible(known)?;
                self.retry.failures = 0;
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
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
    }
    #[test]
    fn latest_observation_drops_old_context_and_backoff_is_bounded() {
        let mut observation = Observation {
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
