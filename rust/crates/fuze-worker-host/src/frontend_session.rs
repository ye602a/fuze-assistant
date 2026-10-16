//! Toolkit-independent access to the existing desktop engine. No second OCR/LCU loop.
//! The offline constructor never reads preferences/accounts or starts discovery.
use super::*;
use std::sync::atomic::AtomicBool;

pub struct Session {
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
    offline: bool,
    maintenance: AtomicBool,
}

impl Session {
    pub fn start(config: Config, offline: bool) -> Result<Self> {
        let shared = Arc::new(Shared {
            external_overlays: true,
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
        })
    }
    /// Account keys never need to be logged or serialized by a frontend.
    pub fn snapshot(&self) -> Status {
        self.shared.status()
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
