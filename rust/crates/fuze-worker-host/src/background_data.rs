//! One cancellable idle update, polled by the existing host tick. No screenshot/OCR work.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub updated: bool,
    pub supplemental_error: Option<&'static str>,
}
pub struct Refresh {
    progress: Arc<Mutex<crate::data_update::Progress>>,
    due: Instant,
    manual: bool,
    activation: Arc<AtomicU8>,
    job: Option<(Arc<AtomicBool>, mpsc::Receiver<crate::Result<Outcome>>)>,
}
impl Refresh {
    pub fn new() -> Self {
        Self {
            progress: Default::default(),
            due: Instant::now() + Duration::from_secs(20),
            manual: false,
            activation: Default::default(),
            job: None,
        }
    }
    pub fn request(&mut self) {
        self.manual = true;
        self.due = Instant::now();
    }
    pub fn needs_activation(&self) -> bool {
        self.activation.load(Ordering::Acquire) == 1
    }
    pub fn allow_activation(&self) {
        self.activation.store(2, Ordering::Release);
    }
    pub fn applying(&self) -> bool {
        self.activation.load(Ordering::Acquire) == 2
    }
    pub fn cancel(&self) {
        if let Some((cancel, _)) = &self.job {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn progress(&self) -> crate::data_update::Progress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_default()
    }
    pub fn tick(
        &mut self,
        idle: bool,
        root: Option<&PathBuf>,
        roots: &[fuze_data::Root],
        locale: fuze_data::Locale,
    ) -> Option<crate::Result<Outcome>> {
        if !idle && !self.manual {
            self.cancel();
        }
        let result = self.job.as_ref().and_then(|(_, rx)| match rx.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("data-update-worker")),
            _ => None,
        });
        if let Some(result) = result {
            self.job = None;
            self.manual = false;
            self.activation.store(0, Ordering::Release);
            self.due =
                Instant::now() + Duration::from_secs(if result.is_ok() { 1800 } else { 300 });
            return Some(result);
        }
        if (idle || self.manual) && self.job.is_none() && Instant::now() >= self.due {
            let Some(root) = root.cloned() else {
                self.manual = false;
                return Some(Err("data-update-cache"));
            };
            let roots = roots.to_vec();
            let progress = self.progress.clone();
            let activation = self.activation.clone();
            let manual = self.manual;
            if let Ok(mut p) = progress.lock() {
                *p = crate::data_update::Progress {
                    message: "正在自动检查数据…".into(),
                    ..Default::default()
                };
            }
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            if std::thread::Builder::new()
                .name("fuze-idle-data".into())
                .spawn(move || {
                    let report = |p| {
                        if let Ok(mut current) = progress.lock() {
                            *current = p;
                        }
                    };
                    let before_activate = || {
                        activation.store(1, Ordering::Release);
                        while activation.load(Ordering::Acquire) != 2 {
                            if flag.load(Ordering::Acquire) {
                                return Err("data-update-cancelled");
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        Ok(())
                    };
                    report(crate::data_update::Progress {
                        message: "正在检查补充识别词库…".into(),
                        ..Default::default()
                    });
                    let prepared = crate::recognition_sources::prepare_refresh(&root, &|| {
                        flag.load(Ordering::Acquire)
                    });
                    let result = crate::data_update::refresh_guarded(
                        &root,
                        &roots,
                        if manual {
                            fuze_data::Locale::ZhCn
                        } else {
                            locale
                        },
                        &|| flag.load(Ordering::Acquire),
                        &report,
                        &|p, n, c| crate::update_io::data_get(p, n, c),
                        &before_activate,
                    );
                    let supplemental = if result.is_ok() {
                        prepared.and_then(|prepared| {
                            if prepared.changed() && activation.load(Ordering::Acquire) != 2 {
                                before_activate()?;
                            }
                            if flag.load(Ordering::Acquire) {
                                return Err("data-update-cancelled");
                            }
                            prepared.apply(&root)
                        })
                    } else {
                        Ok(())
                    };
                    let _ = tx.send(result.map(|changed| Outcome {
                        updated: changed,
                        supplemental_error: supplemental.err(),
                    }));
                })
                .is_err()
            {
                self.manual = false;
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
    fn manual_download_survives_play_and_activation_requires_host_ack() {
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        let mut updater = Refresh::new();
        updater.job = Some((flag.clone(), rx));
        updater.request();
        assert_eq!(
            updater.tick(false, None, &[], fuze_data::Locale::ZhCn),
            None
        );
        assert!(!flag.load(Ordering::Acquire));
        updater.activation.store(1, Ordering::Release);
        assert!(updater.needs_activation());
        assert!(!updater.applying());
        updater.allow_activation();
        assert!(updater.applying());
        tx.send(Ok(Outcome {
            updated: true,
            supplemental_error: None,
        }))
        .unwrap();
        assert!(
            updater
                .tick(false, None, &[], fuze_data::Locale::ZhCn)
                .unwrap()
                .unwrap()
                .updated
        );
        assert!(!updater.busy());
        assert!(!updater.applying());
    }
    #[test]
    fn leaving_idle_cancels_the_only_job_and_failure_backs_off() {
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        let mut updater = Refresh {
            progress: Default::default(),
            due: Instant::now(),
            manual: false,
            activation: Default::default(),
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
