//! Caller-driven backoff: no thread, timer, hot polling, or stale success after a failed request.
use crate::{Error, Result};
use fuze_runtime::session::Phase;
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    Waiting,
    Connected(Phase),
    Retry(Error),
    Paused,
}
pub struct Reconnect {
    state: State,
    failures: u8,
    next: Option<u64>,
}
impl Default for Reconnect {
    fn default() -> Self {
        Self {
            state: State::Waiting,
            failures: 0,
            next: Some(0),
        }
    }
}
impl Reconnect {
    pub fn refresh_at(&mut self, at: u64) {
        if self.state != State::Paused {
            self.next = Some(at);
        }
    }
    pub fn due(&self, now: u64) -> bool {
        self.next.is_some_and(|at| now >= at)
    }
    pub fn wake_at(&self) -> Option<u64> {
        self.next
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn pause(&mut self) {
        self.state = State::Paused;
        self.next = None;
    }
    pub fn resume(&mut self, now: u64) {
        self.state = State::Waiting;
        self.failures = 0;
        self.next = Some(now);
    }
    pub fn finish(&mut self, result: Result<Option<Phase>>, completed_at: u64) {
        if self.state == State::Paused {
            return;
        }
        let delay = match result {
            Ok(Some(phase)) if phase != Phase::Disconnected => {
                self.failures = 0;
                self.state = State::Connected(phase);
                2000
            }
            Ok(_) => {
                self.failures = 0;
                self.state = State::Waiting;
                5000
            }
            Err(error) => {
                self.state = State::Retry(error);
                self.failures = self.failures.saturating_add(1);
                [1000, 2000, 5000, 10000, 30000]
                    [usize::from(self.failures.saturating_sub(1)).min(4)]
            }
        };
        self.next = Some(completed_at.saturating_add(delay));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_clears_connected_state_and_backoff_is_bounded() {
        let mut r = Reconnect::default();
        r.finish(Ok(Some(Phase::ClientReady)), 10);
        assert_eq!(r.state(), State::Connected(Phase::ClientReady));
        assert!(!r.due(2009));
        assert!(r.due(2010));
        r.finish(Err(Error::Unauthorized), 2010);
        assert_eq!(r.state(), State::Retry(Error::Unauthorized));
        assert_eq!(r.wake_at(), Some(3010));
        for _ in 0..100 {
            r.finish(Err(Error::Transport), 5000);
        }
        assert_eq!(r.wake_at(), Some(35000));
        r.pause();
        r.finish(Ok(Some(Phase::InProgress)), 9000);
        assert_eq!(r.wake_at(), None);
        assert_eq!(r.state(), State::Paused);
        r.resume(10000);
        assert!(r.due(10000));
        r.finish(Ok(None), 10000);
        assert_eq!(r.state(), State::Waiting);
    }
}
