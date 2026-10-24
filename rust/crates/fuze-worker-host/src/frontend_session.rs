//! Toolkit-independent access to the existing desktop engine. No second OCR/LCU loop.
//! The offline constructor never reads preferences/accounts or starts discovery.
use super::*;

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
            screenshots: Default::default(),
            external_overlays: true,
            start_paused,
            automatic_change: AtomicU8::new(0),
            resume_apply: AtomicU8::new(PAUSED),
            history_read: Mutex::new(None),
            history_busy: AtomicBool::new(false),
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
        shared
            .screenshots
            .start_session(offline, crate::recognition_log::root);
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
    pub fn screenshot_status(&self) -> (bool, bool, String) {
        self.shared.screenshots.status()
    }
    pub fn screenshot_logging(&self, enabled: bool) -> Result<()> {
        if self.offline {
            return Err("frontend-offline-operation");
        }
        self.shared.screenshots.enable(enabled)?;
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn mark_screenshot(&self) -> Result<()> {
        if self.offline {
            return Err("frontend-offline-operation");
        }
        self.shared.screenshots.mark_recent()
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
        if self.maintenance.load(Ordering::Acquire) || self.history_busy() {
            return Err("frontend-busy");
        }
        value.validate()?;
        let guard = self.shared.wait.lock().map_err(|_| "frontend-lock")?;
        if self.history_busy() || self.maintenance.load(Ordering::Acquire) {
            return Err("frontend-busy");
        }
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
        self.request_history_read(if more {
            crate::post_game_history::ReadRequest::More
        } else {
            crate::post_game_history::ReadRequest::Refresh
        })
    }
    pub fn cached_history(&self) -> Result<()> {
        self.request_history_read(crate::post_game_history::ReadRequest::Cached)
    }
    pub fn history_busy(&self) -> bool {
        self.shared.history_busy.load(Ordering::Acquire)
    }
    fn request_history_read(&self, request: crate::post_game_history::ReadRequest) -> Result<()> {
        let _guard = self.shared.wait.lock().map_err(|_| "frontend-lock")?;
        if self.offline {
            return Err("frontend-offline-operation");
        }
        if self.busy()
            || ![PAUSED, RUNNING].contains(&self.shared.desired.load(Ordering::Acquire))
            || self.history_busy()
        {
            return Err("frontend-busy");
        }
        let status = self.snapshot();
        if !status.initialized {
            return Err("frontend-busy");
        }
        if status.history.pending > 0 {
            return Err("history-import-pending");
        }
        if matches!(request, crate::post_game_history::ReadRequest::More)
            && status.history.next_start.is_none()
        {
            return Err("history-page-limit");
        }
        let mut pending = self
            .shared
            .history_read
            .lock()
            .map_err(|_| "frontend-lock")?;
        // Publish busy before releasing the queue: a fast completion must not
        // clear the flag before the requesting thread has set it.
        self.shared.history_busy.store(true, Ordering::Release);
        *pending = Some(request);
        drop(pending);
        self.shared.wake.notify_one();
        Ok(())
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
        if self.busy() || self.history_busy() {
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
        let _guard = self.shared.wait.lock().map_err(|_| "frontend-lock")?;
        if self.history_busy() || self.maintenance.load(Ordering::Acquire) {
            return Err("frontend-busy");
        }
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
        if self.maintenance.load(Ordering::Acquire) || self.history_busy() {
            return Err("frontend-busy");
        }
        let _guard = self.shared.wait.lock().map_err(|_| "frontend-lock")?;
        if self.shared.desired.load(Ordering::Acquire) != PAUSED {
            return Err("frontend-pause-required");
        }
        if self.history_busy() || self.maintenance.load(Ordering::Acquire) {
            return Err("frontend-busy");
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
        self.shared.screenshots.finish();
    }
}
#[cfg(test)]
mod locale_tests {
    use super::*;
    #[test]
    fn history_reads_keep_recognition_mode_and_pause_exit_available() {
        // No engine, client discovery, disk access or worker: exercise the real command gate.
        let shared = Arc::new(Shared {
            screenshots: Default::default(),
            external_overlays: true,
            start_paused: false,
            automatic_change: AtomicU8::new(0),
            resume_apply: AtomicU8::new(PAUSED),
            history_read: Mutex::new(None),
            history_busy: AtomicBool::new(false),
            wake: Condvar::new(),
            wait: Mutex::new(()),
            desired: AtomicU8::new(RUNNING),
            status: Mutex::new(Status {
                initialized: true,
                mode: RUNNING,
                ..Default::default()
            }),
            monitor: Mutex::new(None),
            pending: Mutex::new(None),
            import_path: Mutex::new(None),
            import_game: Mutex::new(None),
            import_selection: Mutex::new(None),
            export: Mutex::new(None),
        });
        let session = Session {
            shared: shared.clone(),
            worker: None,
            offline: false,
            maintenance: AtomicBool::new(false),
            last_probe: Mutex::new(None),
        };
        session.refresh_history(false).unwrap();
        assert!(session.history_busy());
        assert!(!session.busy());
        assert_eq!(shared.desired.load(Ordering::Acquire), RUNNING);
        assert_eq!(session.cached_history(), Err("frontend-busy"));
        assert_eq!(session.apply(Default::default()), Err("frontend-busy"));
        assert_eq!(session.begin_maintenance(), Err("frontend-busy"));
        session.set_running(false).unwrap();
        assert_eq!(shared.desired.load(Ordering::Acquire), PAUSED);
        assert_eq!(session.update_data(), Err("frontend-busy"));
        shared.history_busy.store(false, Ordering::Release);
        shared.history_read.lock().unwrap().take();
        shared.status.lock().unwrap().history.pending = 1;
        assert_eq!(session.cached_history(), Err("history-import-pending"));
        shared.status.lock().unwrap().history.pending = 0;
        assert_eq!(session.refresh_history(true), Err("history-page-limit"));
        session.cached_history().unwrap();
        session.stop();
        assert_eq!(shared.desired.load(Ordering::Acquire), EXIT);
    }
    #[test]
    fn locale_activation_publishes_only_committed_state_and_clears_old_pipeline() {
        let fixture = crate::preferences::locale_tests::Fixture::new();
        fixture.dataset("zh-CN", "1.0", true);
        fixture.dataset("en-US", "1.0", false);
        let session = Session::start(fixture.config(), true).unwrap();
        assert_eq!(
            session.screenshot_logging(true),
            Err("frontend-offline-operation")
        );
        assert_eq!(session.mark_screenshot(), Err("frontend-offline-operation"));
        assert!(!session.screenshot_status().0);
        let wait = |ready: &dyn Fn(&Status) -> bool| {
            let start = Instant::now();
            loop {
                let status = session.snapshot();
                if ready(&status) {
                    return status;
                }
                assert!(start.elapsed() < Duration::from_secs(5));
                thread::sleep(Duration::from_millis(10));
            }
        };
        let before = wait(&|s| s.initialized);
        {
            let mut s = session.shared.status.lock().unwrap();
            s.pipeline.error = Some("old-language-fixture");
        }
        let p = crate::preferences::Preferences {
            locale: "en-US".into(),
            ..before.preferences.clone()
        };
        session.apply(p.clone()).unwrap();
        let failed = wait(&|s| !session.busy() && s.update_error.is_some());
        assert_eq!(failed.preferences, before.preferences);
        assert_eq!(failed.ui_locale, before.ui_locale);
        assert_eq!(failed.available, before.available);
        fixture.dataset("en-US", "1.0", true);
        session.apply(p).unwrap();
        let applied = wait(&|s| !session.busy() && s.preferences.locale == "en-US");
        assert_eq!(applied.ui_locale, "en-US");
        assert_eq!(applied.available.as_deref(), Some("en-US@1.0"));
        assert!(applied.pipeline.overlay.is_none());
        assert!(applied.pipeline.recent.is_none());
        assert!(applied.pipeline.dataset.is_none());
        assert!(applied.pipeline.error.is_none());
        assert_eq!(applied.pipeline.analyses, 0);
        assert!(applied.selection.is_none());
        assert!(applied.playing.is_none());
        assert!(applied.connection.is_none());
        assert!(applied.verified_time.is_none());
        assert_eq!(applied.mode, PAUSED);
        assert!(applied.update_error.is_none());
        session.stop();
        wait(&|s| s.done);
    }
}
