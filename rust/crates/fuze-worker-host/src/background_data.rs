//! One cancellable idle update, polled by the existing host tick. No screenshot/OCR work.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub updated: bool,
    pub supplemental_error: Option<&'static str>,
}
pub struct Refresh {
    due: Instant,
    job: Option<(Arc<AtomicBool>, mpsc::Receiver<crate::Result<Outcome>>)>,
}
impl Refresh {
    pub fn new() -> Self {
        Self {
            due: Instant::now() + Duration::from_secs(20),
            job: None,
        }
    }
    pub fn cancel(&self) {
        if let Some((cancel, _)) = &self.job {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn tick(
        &mut self,
        idle: bool,
        root: Option<&PathBuf>,
        roots: &[fuze_data::Root],
        locale: fuze_data::Locale,
    ) -> Option<crate::Result<Outcome>> {
        if !idle {
            self.cancel();
        }
        let result = self.job.as_ref().and_then(|(_, rx)| match rx.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("data-update-worker")),
            _ => None,
        });
        if let Some(result) = result {
            self.job = None;
            self.due =
                Instant::now() + Duration::from_secs(if result.is_ok() { 1800 } else { 300 });
            return Some(result);
        }
        if idle && self.job.is_none() && Instant::now() >= self.due {
            let Some(root) = root.cloned() else {
                return None;
            };
            let roots = roots.to_vec();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            if std::thread::Builder::new()
                .name("fuze-idle-data".into())
                .spawn(move || {
                    let supplemental = crate::recognition_sources::refresh(&root, &|| {
                        flag.load(Ordering::Acquire)
                    });
                    let result =
                        crate::data_update::refresh_current(&root, &roots, locale, &|| {
                            flag.load(Ordering::Acquire)
                        });
                    let _ = tx.send(result.map(|changed| Outcome {
                        updated: changed,
                        supplemental_error: supplemental.err(),
                    }));
                })
                .is_err()
            {
                self.due = Instant::now() + Duration::from_secs(300);
                return Some(Err("data-update-worker"));
            }
            self.job = Some((cancel, rx));
        }
        None
    }
}
impl Drop for Refresh {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leaving_idle_cancels_the_only_job_and_failure_backs_off() {
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        let mut updater = Refresh {
            due: Instant::now(),
            job: Some((flag.clone(), rx)),
        };
        assert_eq!(
            updater.tick(false, None, &[], fuze_data::Locale::ZhCn),
            None
        );
        assert!(flag.load(Ordering::Acquire));
        assert!(updater.busy());
        tx.send(Err("data-update-cancelled")).unwrap();
        assert_eq!(
            updater.tick(false, None, &[], fuze_data::Locale::ZhCn),
            Some(Err("data-update-cancelled"))
        );
        assert!(!updater.busy());
        assert!(updater.due > Instant::now() + Duration::from_secs(290));
    }
}
