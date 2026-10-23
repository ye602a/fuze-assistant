//! One delayed refresh after an observed game. No startup scan, recurring timer or upload.
use fuze_runtime::session::Phase;
use std::{path::PathBuf, sync::mpsc, time::Instant};
#[derive(Clone, Copy)]
pub enum ReadRequest {
    Refresh,
    More,
    Cached,
}
pub struct Completion {
    pub view: Option<crate::account_history::View>,
    pub explicit: bool,
}
#[derive(Default)]
pub struct Schedule {
    pending: bool,
    due: Option<u64>,
}
impl Schedule {
    pub fn tick(&mut self, phase: Option<Phase>, now: u64) -> bool {
        match phase {
            Some(Phase::InProgress) => {
                self.pending = true;
                self.due = None;
                false
            }
            Some(Phase::ClientReady | Phase::PostGame) if self.pending => {
                let due = self.due.get_or_insert(now.saturating_add(10_000));
                if now >= *due {
                    self.pending = false;
                    self.due = None;
                    true
                } else {
                    false
                }
            }
            Some(Phase::ClientReady | Phase::PostGame) => false,
            _ => {
                self.pending = false;
                self.due = None;
                false
            }
        }
    }
}
pub struct Refresh {
    schedule: Schedule,
    epoch: Instant,
    job: Option<mpsc::Receiver<crate::account_history::View>>,
    explicit: bool,
    cached: bool,
}
impl Default for Refresh {
    fn default() -> Self {
        Self {
            schedule: Default::default(),
            epoch: Instant::now(),
            job: None,
            explicit: false,
            cached: false,
        }
    }
}
impl Refresh {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    /// One explicit read shares the post-game transaction slot. No capture pause or
    /// second account-cache writer; caller polls the result through its existing loop.
    pub fn request(
        &mut self,
        request: ReadRequest,
        previous: crate::account_history::View,
        manual: Option<PathBuf>,
        roots: Vec<fuze_data::Root>,
        locale: fuze_data::Locale,
        cache_index: usize,
    ) -> crate::Result<()> {
        if self.busy() {
            return Err("frontend-busy");
        }
        self.spawn(true, matches!(request, ReadRequest::Cached), move || {
            let result = (|| {
                let path = crate::account_history::path()?;
                Ok(match request {
                    ReadRequest::Cached => crate::account_history::offline(&path, cache_index)?,
                    ReadRequest::Refresh => {
                        crate::account_history::refresh(manual.as_deref(), &path)
                    }
                    ReadRequest::More => crate::account_history::refresh_page(
                        manual.as_deref(),
                        &path,
                        previous.next_start.ok_or("history-page-limit")?,
                        Some((&previous.account, &previous.platform)),
                        Some(&previous.source),
                    ),
                })
            })();
            let mut view = preserve_failure(previous, result);
            crate::history_detail::populate(&mut view, &roots, locale);
            view
        })
    }
    fn spawn(
        &mut self,
        explicit: bool,
        cached: bool,
        work: impl FnOnce() -> crate::account_history::View + Send + 'static,
    ) -> crate::Result<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("fuze-history-read".into())
            .spawn(move || {
                let _ = tx.send(work());
            })
            .map_err(|_| "history-thread")?;
        self.job = Some(rx);
        self.explicit = explicit;
        self.cached = cached;
        Ok(())
    }
    pub fn tick(
        &mut self,
        phase: Option<Phase>,
        manual: Option<PathBuf>,
        automatic: bool,
    ) -> Option<Completion> {
        let completed = self.job.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(v) => Some(Some(v)),
            Err(mpsc::TryRecvError::Disconnected) => Some(None),
            _ => None,
        });
        if let Some(v) = completed {
            self.job = None;
            let explicit = self.explicit;
            let view = if explicit {
                let mut view = v.unwrap_or_else(|| crate::account_history::View {
                    error: Some("history-thread"),
                    ..Default::default()
                });
                if !self.cached
                    && phase.is_some_and(|p| !matches!(p, Phase::ClientReady | Phase::PostGame))
                {
                    // Do not present an earlier online validation as current after entering a match.
                    view.cached = true;
                    view.error = Some("history-idle-required");
                }
                Some(view)
            } else {
                v.filter(|_| matches!(phase, Some(Phase::ClientReady | Phase::PostGame)))
            };
            return Some(Completion { view, explicit });
        }
        let scheduled_phase = if automatic { phase } else { None };
        // A user cache read must not consume the one pending post-game refresh.
        // While busy, still observe playing/disconnect so cancellation stays current.
        let defer =
            self.busy() && matches!(scheduled_phase, Some(Phase::ClientReady | Phase::PostGame));
        if !defer
            && self
                .schedule
                .tick(scheduled_phase, self.epoch.elapsed().as_millis() as u64)
            && self.job.is_none()
        {
            let _ = self.spawn(false, false, move || match crate::account_history::path() {
                Ok(path) => crate::account_history::refresh_after_game(manual.as_deref(), &path),
                Err(error) => crate::account_history::View {
                    error: Some(error),
                    ..Default::default()
                },
            });
        }
        None
    }
}
fn preserve_failure(
    previous: crate::account_history::View,
    result: crate::Result<crate::account_history::View>,
) -> crate::account_history::View {
    let view = result.unwrap_or_else(|error| crate::account_history::View {
        error: Some(error),
        ..Default::default()
    });
    // Keep an explicitly labelled cache on transport failure, but never transfer
    // old-account rows into a newly verified account or identity conflict.
    if view.error.is_some()
        && view.rows.is_empty()
        && view.account.is_empty()
        && !matches!(
            view.error,
            Some(
                "history-account-changed"
                    | "history-owner-mismatch"
                    | "history-platform-conflict"
                    | "history-platform-unknown"
                    | "history-login-required"
            )
        )
    {
        crate::account_history::View {
            error: view.error,
            cached: true,
            pending: 0,
            journal_preview: false,
            ..previous
        }
    } else {
        view
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_reads_poll_without_waiting_and_never_replace_another_account_on_failure() {
        let mut worker = Refresh::default();
        let (release, wait) = mpsc::sync_channel(1);
        worker
            .spawn(true, true, move || {
                wait.recv().unwrap();
                crate::account_history::View {
                    cached: true,
                    source: "fixture".into(),
                    ..Default::default()
                }
            })
            .unwrap();
        assert!(worker.busy());
        worker.schedule = Schedule {
            pending: true,
            due: Some(0),
        };
        assert!(worker.tick(Some(Phase::PostGame), None, true).is_none());
        assert!(worker.schedule.pending);
        assert!(worker.tick(Some(Phase::InProgress), None, false).is_none());
        assert!(worker
            .request(
                ReadRequest::Cached,
                Default::default(),
                None,
                vec![],
                fuze_data::Locale::ZhCn,
                0
            )
            .is_err());
        release.send(()).unwrap();
        let start = Instant::now();
        let complete = loop {
            if let Some(c) = worker.tick(Some(Phase::InProgress), None, false) {
                break c;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(2));
            std::thread::yield_now();
        };
        assert!(complete.explicit);
        assert_eq!(complete.view.unwrap().source, "fixture");
        assert!(!worker.busy());
        let previous = crate::account_history::View {
            account: "old-fixture".into(),
            ..Default::default()
        };
        let kept = preserve_failure(previous.clone(), Err("history-connect-failed"));
        assert_eq!(kept.account, previous.account);
        assert!(kept.cached);
        for error in [
            "history-account-changed",
            "history-platform-unknown",
            "history-login-required",
        ] {
            assert!(preserve_failure(previous.clone(), Err(error))
                .account
                .is_empty());
        }
        let new_owner = crate::account_history::View {
            account: "new-fixture".into(),
            error: Some("history-http"),
            ..Default::default()
        };
        assert_eq!(
            preserve_failure(previous, Ok(new_owner)).account,
            "new-fixture"
        );
        // An online response completed after entering selection must be labelled cached.
        worker.spawn(true, false, Default::default).unwrap();
        let start = Instant::now();
        loop {
            if let Some(c) = worker.tick(Some(Phase::ChampSelect), None, false) {
                let view = c.view.unwrap();
                assert!(view.cached);
                assert_eq!(view.error, Some("history-idle-required"));
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(2));
            std::thread::yield_now();
        }
    }
    #[test]
    fn only_one_completed_game_schedules_and_phase_change_cancels() {
        let mut s = Schedule::default();
        assert!(!s.tick(Some(Phase::ClientReady), 0));
        assert!(!s.tick(Some(Phase::InProgress), 100));
        assert!(!s.tick(Some(Phase::PostGame), 200));
        assert!(!s.tick(Some(Phase::ClientReady), 10_199));
        assert!(s.tick(Some(Phase::ClientReady), 10_200));
        assert!(!s.tick(Some(Phase::ClientReady), 50_000));
        s.tick(Some(Phase::InProgress), 60_000);
        s.tick(Some(Phase::PostGame), 61_000);
        s.tick(Some(Phase::ChampSelect), 62_000);
        assert!(!s.tick(Some(Phase::ClientReady), 80_000));
        s.tick(Some(Phase::InProgress), 90_000);
        s.tick(None, 90_100);
        assert!(!s.tick(Some(Phase::PostGame), 100_000));
    }
}
