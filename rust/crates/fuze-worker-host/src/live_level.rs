//! One lazy IO thread, one sample, no queued requests and no UI/OCR blocking.
use std::{
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
struct State {
    generation: u64,
    active: bool,
    stop: bool,
    sample: Option<(fuze_lcu::transport::LiveState, Instant)>,
}
pub struct Reader {
    shared: Arc<(Mutex<State>, Condvar)>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Reader {
    pub fn start() -> Result<Self, &'static str> {
        Self::with_source(Box::new(|| fuze_lcu::transport::active_player_state().ok()))
    }
    fn with_source(
        read: Box<dyn Fn() -> Option<fuze_lcu::transport::LiveState> + Send>,
    ) -> Result<Self, &'static str> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("fuze-live-level".into())
            .spawn(move || {
                let (lock, wake) = &*state;
                loop {
                    let Ok(mut s) = lock.lock() else { return };
                    while !s.active && !s.stop {
                        s = match wake.wait(s) {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                    if s.stop {
                        return;
                    }
                    let generation = s.generation;
                    drop(s);
                    let value = read().filter(|v| (1..=30).contains(&v.level));
                    let Ok(mut s) = lock.lock() else { return };
                    if s.active && s.generation == generation {
                        s.sample = value.map(|v| (v, Instant::now()));
                    }
                    if s.stop {
                        return;
                    }
                    // A context change wakes this wait; ordinary polling cannot enqueue more work.
                    let _ = wake.wait_timeout_while(s, Duration::from_secs(3), |s| {
                        !s.stop && s.active && s.generation == generation
                    });
                }
            })
            .map_err(|_| "level-worker-start")?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub fn enable(&self, active: bool) {
        if let Ok(mut s) = self.shared.0.lock() {
            if s.active != active {
                s.active = active;
                s.generation = s.generation.wrapping_add(1);
                s.sample = None;
                self.shared.1.notify_one();
            }
        }
    }
    pub fn sample(&self) -> Option<(u8, Instant)> {
        self.state().map(|(s, at)| (s.level, at))
    }
    pub fn state(&self) -> Option<(fuze_lcu::transport::LiveState, Instant)> {
        self.shared
            .0
            .lock()
            .ok()
            .and_then(|s| s.sample)
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(10))
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        if let Ok(mut s) = self.shared.0.lock() {
            s.stop = true;
            self.shared.1.notify_one();
        }
        // IO has bounded per-operation timeouts. Controller replacement never blocks OCR/UI.
        self.worker.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_rejects_inflight_old_level_and_stale_samples() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = count.clone();
        let reader = Reader::with_source(Box::new(move || {
            let index = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if index == 0 {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                Some(fuze_lcu::transport::LiveState {
                    level: 3,
                    dead: None,
                })
            } else {
                Some(fuze_lcu::transport::LiveState {
                    level: 7,
                    dead: None,
                })
            }
        }))
        .unwrap();
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
        reader.enable(true);
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        reader.enable(false);
        reader.enable(true);
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while reader.sample().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(reader.sample().map(|s| s.0), Some(7));
        reader.enable(false);
        assert!(reader.sample().is_none());
        reader.shared.0.lock().unwrap().sample = Some((
            fuze_lcu::transport::LiveState {
                level: 7,
                dead: None,
            },
            Instant::now() - Duration::from_secs(11),
        ));
        assert!(reader.sample().is_none());
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
