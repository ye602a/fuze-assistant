//! Toolkit-independent access to the existing desktop engine. No second OCR/LCU loop.
//! The offline constructor never reads preferences/accounts or starts discovery.
use super::*;
use std::sync::atomic::AtomicBool;

pub struct Session {
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
    offline: bool,
    maintenance: AtomicBool,
    last_probe: Mutex<Option<Instant>>,
}

impl Session {
    pub fn start(config: Config, offline: bool) -> Result<Self> {
        Self::start_with_recovery(config, offline, false)
    }
    /// A recovered UI must not resume capture before the player explicitly continues.
    pub fn start_with_recovery(config: Config, offline: bool, start_paused: bool) -> Result<Self> {
        let shared = Arc::new(Shared {
            external_overlays: true,
            start_paused,
            automatic_change: AtomicU8::new(0),
            resume_apply: AtomicU8::new(PAUSED),
            wake: Condvar::new(),
            wait: Mutex::new(()),
            desired: AtomicU8::new(PAUSED),
            status: Mutex::new(Status::default()),
            monitor: Mutex::new(None),
            pending: Mutex::new(None),
            import_path: Mutex::new(None),
            import_game: Mutex::new(None),
            import_selection: Mutex::new(None),
            export: Mutex::new(None),
        });
        let owned = shared.clone();
        let worker = thread::Builder::new()
            .name("fuze-frontend-engine".into())
            .spawn(move || engine(config, owned, offline))
            .map_err(|_| "frontend-thread")?;
        Ok(Self {
            shared,
            worker: Some(worker),
            offline,
            maintenance: AtomicBool::new(false),
            last_probe: Mutex::new(None),
        })
    }
    /// Account keys never need to be logged or serialized by a frontend.
    pub fn snapshot(&self) -> Status {
        self.shared.status()
    }
    pub fn diagnostic_context(&self) -> (u64, bool) {
        self.shared
            .monitor
            .lock()
            .ok()
            .and_then(|m| m.clone())
            .map(|m| m.diagnostic_context())
            .unwrap_or((0, false))
    }
    /// Only an explicit player action can start this bounded, sanitized read-only probe.
    pub fn probe_claim(&self) -> Result<std::sync::mpsc::Receiver<fuze_lcu::stage_probe::Report>> {
        if self.offline {
            return Err("frontend-offline-operation");
        }
        if self.busy() || self.shared.desired.load(Ordering::Acquire) == EXIT {
            return Err("frontend-busy");
        }
        let monitor = self
            .shared
            .monitor
            .lock()
            .map_err(|_| "frontend-lock")?
            .clone()
            .ok_or("probe-context")?;
        if !monitor.diagnostic_context().1 {
            return Err("probe-context");
        }
        let mut last = self.last_probe.lock().map_err(|_| "frontend-lock")?;
        if last.is_some_and(|at| at.elapsed() < Duration::from_secs(30)) {
            return Err("probe-cooldown");
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        thread::Builder::new()
            .name("fuze-explicit-claim-probe".into())
            .spawn(move || {
                let report = fuze_lcu::stage_probe::inspect(|| monitor.diagnostic_context());
                let _ = tx.send(report);
            })
            .map_err(|_| "probe-thread")?;
        *last = Some(Instant::now());
        Ok(rx)
    }
    pub fn busy(&self) -> bool {
        self.maintenance.load(Ordering::Acquire)
            || self.shared.desired.load(Ordering::Acquire) > EXIT
    }
    pub fn set_running(&self, enabled: bool) -> Result<()> {
        if self.busy() || self.shared.desired.load(Ordering::Acquire) == EXIT {
            return Err("frontend-busy");
        }
        if !self.offline {
            self.shared
                .automatic_change
                .store(if enabled { 2 } else { 1 }, Ordering::Release);
        }
        self.shared.request(if enabled { RUNNING } else { PAUSED });
        Ok(())
    }
    /// Pending values are committed by the existing engine only after validation succeeds.
    pub fn apply(&self, value: crate::preferences::Preferences) -> Result<()> {
        if self.maintenance.load(Ordering::Acquire) {
            return Err("frontend-busy");
        }
        value.validate()?;
        let guard = self.shared.wait.lock().map_err(|_| "frontend-lock")?;
        let mode = self.shared.desired.load(Ordering::Acquire);
        if ![PAUSED, RUNNING].contains(&mode) {
            return Err("frontend-busy");
        }
        *self.shared.pending.lock().map_err(|_| "frontend-lock")? = Some(value);
        self.shared.resume_apply.store(mode, Ordering::Release);
        self.shared.desired.store(APPLY, Ordering::Release);
        self.shared.wake.notify_one();
        drop(guard);
        Ok(())
    }
    pub fn refresh_history(&self, more: bool) -> Result<()> {
        self.request_idle(if more { HISTORY_MORE } else { HISTORY })
    }
    pub fn cached_history(&self) -> Result<()> {
        self.request_idle(HISTORY_OFFLINE)
    }
    pub fn update_data(&self) -> Result<()> {
        self.request_idle(DATA)
    }
    pub fn preview_history(&self, path: std::path::PathBuf, game: Option<u64>) -> Result<()> {
        self.history_request(
            if game.is_some() {
                JOURNAL_PREVIEW
            } else {
                HISTORY_PREVIEW
            },
            Some(path),
            game,
            None,
        )
    }
    pub fn confirm_history(&self, games: Vec<u64>) -> Result<()> {
        if self.snapshot().history.pending == 0 {
            return Err("history-import-missing");
        }
        self.history_request(HISTORY_CONFIRM, None, None, Some(games))
    }
    pub fn cancel_history(&self) -> Result<()> {
        if self.snapshot().history.pending == 0 {
            return Err("history-import-missing");
        }
        self.request_idle(HISTORY_CANCEL)
    }
    fn history_request(
        &self,
        mode: u8,
        path: Option<std::path::PathBuf>,
        game: Option<u64>,
        selection: Option<Vec<u64>>,
    ) -> Result<()> {
        let _guard = self.shared.wait.lock().map_err(|_| "frontend-lock")?;
        if self.busy() {
            return Err("frontend-busy");
        }
        if self.shared.desired.load(Ordering::Acquire) != PAUSED || self.snapshot().mode != PAUSED {
            return Err("frontend-pause-required");
        }
        if self.offline {
            return Err("frontend-offline-operation");
        }
        *self
            .shared
            .import_path
            .lock()
            .map_err(|_| "frontend-lock")? = path;
        *self
            .shared
            .import_game
            .lock()
            .map_err(|_| "frontend-lock")? = game;
        *self
            .shared
            .import_selection
            .lock()
            .map_err(|_| "frontend-lock")? = selection;
        self.shared.desired.store(mode, Ordering::Release);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn begin_maintenance(&self) -> Result<()> {
        let status = self.snapshot();
        if !status.initialized
            || status.mode != PAUSED
            || self.shared.desired.load(Ordering::Acquire) != PAUSED
        {
            return Err("frontend-pause-required");
        }
        if self.maintenance.swap(true, Ordering::AcqRel) {
            return Err("frontend-busy");
        }
        Ok(())
    }
    pub fn end_maintenance(&self) {
        self.maintenance.store(false, Ordering::Release);
    }
    fn request_idle(&self, mode: u8) -> Result<()> {
        if self.maintenance.load(Ordering::Acquire) {
            return Err("frontend-busy");
        }
        let _guard = self.shared.wait.lock().map_err(|_| "frontend-lock")?;
        if self.shared.desired.load(Ordering::Acquire) != PAUSED {
            return Err("frontend-pause-required");
        }
        // Offline previews must never pretend to have queried or downloaded data.
        if self.offline {
            return Err("frontend-offline-operation");
        }
        self.shared.desired.store(mode, Ordering::Release);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn stop(&self) {
        self.shared.request(EXIT);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
