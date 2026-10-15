//! One delayed refresh after an observed game. No startup scan, recurring timer or upload.
use fuze_runtime::session::Phase;
use std::{path::PathBuf, sync::mpsc, time::Instant};
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
}
impl Default for Refresh {
    fn default() -> Self {
        Self {
            schedule: Default::default(),
            epoch: Instant::now(),
            job: None,
        }
    }
}
impl Refresh {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn tick(
        &mut self,
        phase: Option<Phase>,
        manual: Option<PathBuf>,
    ) -> Option<crate::account_history::View> {
        let completed = self.job.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(v) => Some(Some(v)),
            Err(mpsc::TryRecvError::Disconnected) => Some(None),
            _ => None,
        });
        if let Some(v) = completed {
            self.job = None;
            return v.filter(|_| matches!(phase, Some(Phase::ClientReady | Phase::PostGame)));
        }
        if self
            .schedule
            .tick(phase, self.epoch.elapsed().as_millis() as u64)
            && self.job.is_none()
        {
            let (tx, rx) = mpsc::sync_channel(1);
            if std::thread::Builder::new()
                .name("fuze-post-game-history".into())
                .spawn(move || {
                    if let Ok(path) = crate::account_history::path() {
                        let _ = tx.send(crate::account_history::refresh(manual.as_deref(), &path));
                    }
                })
                .is_ok()
            {
                self.job = Some(rx);
            }
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
