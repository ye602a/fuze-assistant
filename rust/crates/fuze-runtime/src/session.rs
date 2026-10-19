use crate::gate::{LevelGate, Selection};
use fuze_contract::{ActualStage, ContextGate, Slot, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Disconnected,
    ClientReady,
    ChampSelect,
    Loading,
    InProgress,
    PostGame,
}
impl Phase {
    pub fn from_lcu(value: Option<&str>) -> Self {
        match value {
            None | Some("") => Self::Disconnected,
            Some("ChampSelect") => Self::ChampSelect,
            Some("GameStart") => Self::Loading,
            Some("InProgress") => Self::InProgress,
            Some("WaitingForStats" | "PreEndOfGame" | "EndOfGame") => Self::PostGame,
            Some("None" | "Lobby" | "Matchmaking" | "ReadyCheck") => Self::ClientReady,
            _ => Self::Disconnected,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Ticket {
    pub generation: u64,
    pub sequence: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameKey {
    pub epoch: u64,
    pub sequence: u64,
}
#[derive(Clone, Copy, Debug)]
enum Worker {
    Idle,
    Running(Ticket, u64),
    Cancelling(Ticket),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cleanup {
    pub generation: u64,
    pub clear_overlay: bool,
    pub stop_capture: bool,
}

/// All methods are called by one host event loop with monotonic millisecond timestamps.
/// It never spawns/kills a worker or calls the game; cancellation requires host acknowledgement.
pub struct Session {
    phase: Phase,
    paused: bool,
    generation: u64,
    sequence: u64,
    dataset: String,
    gate: LevelGate,
    worker: Worker,
    last_frame: Option<FrameKey>,
    failures: u8,
    retry_at: u64,
}
fn valid_dataset_key(value: &str) -> bool {
    value.len() <= 128
        && value.split_once('@').is_some_and(|(locale, version)| {
            ["zh-CN", "zh-TW", "en-US"].contains(&locale)
                && !version.is_empty()
                && version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        })
}
impl Session {
    pub fn new(dataset: String) -> Result<Self, &'static str> {
        if !valid_dataset_key(&dataset) {
            return Err("invalid-dataset-version");
        }
        Ok(Self {
            phase: Phase::Disconnected,
            paused: false,
            generation: 1,
            sequence: 0,
            dataset,
            gate: LevelGate::default(),
            worker: Worker::Idle,
            last_frame: None,
            failures: 0,
            retry_at: 0,
        })
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn update_phase(&mut self, phase: Phase) -> Result<Option<Cleanup>, &'static str> {
        if phase == self.phase {
            return Ok(None);
        }
        self.phase = phase;
        self.gate.reset();
        self.invalidate().map(Some)
    }
    pub fn pause(&mut self, paused: bool) -> Result<Option<Cleanup>, &'static str> {
        if self.paused == paused {
            return Ok(None);
        }
        self.paused = paused;
        self.invalidate().map(Some)
    }
    pub fn dataset_changed(&mut self, dataset: String) -> Result<Option<Cleanup>, &'static str> {
        if !valid_dataset_key(&dataset) {
            return Err("invalid-dataset-version");
        }
        if dataset == self.dataset {
            return Ok(None);
        }
        self.dataset = dataset;
        self.invalidate().map(Some)
    }
    pub fn capture_replaced(&mut self) -> Result<Cleanup, &'static str> {
        self.invalidate()
    }
    fn invalidate(&mut self) -> Result<Cleanup, &'static str> {
        self.gate.invalidate_capture();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("generation-overflow")?;
        self.last_frame = None;
        self.failures = 0;
        self.retry_at = 0;
        // Keep outstanding work until completion or cancellation acknowledgement; never overlap it.
        Ok(Cleanup {
            generation: self.generation,
            clear_overlay: true,
            stop_capture: self.paused || self.phase != Phase::InProgress,
        })
    }
    pub fn should_prewarm(&self) -> bool {
        !self.paused && matches!(self.phase, Phase::Loading | Phase::InProgress)
    }
    /// Host can sleep until this instant or an external event; idle/paused needs no timer.
    pub fn wake_at(&self, now: u64) -> Option<u64> {
        match self.worker {
            Worker::Running(ticket, started) => Some(if ticket.generation != self.generation {
                now
            } else {
                started.saturating_add(3000).max(now)
            }),
            Worker::Cancelling(_) => None,
            Worker::Idle if !self.paused && self.phase == Phase::InProgress && !self.faulted() => {
                Some(self.gate.wake_at(now).max(self.retry_at))
            }
            _ => None,
        }
    }
    pub fn clear_level(&mut self) {
        self.gate.clear_level();
    }
    pub fn closed_window(&self) -> Option<crate::gate::ClosedWindow> {
        self.gate.closed_window()
    }
    pub fn restore_closed_window(&mut self, state: crate::gate::ClosedWindow) {
        self.gate.restore_closed_window(state);
    }
    pub fn level_gate(&mut self, enabled: bool) {
        self.gate.enable(enabled);
    }
    /// Pixel-only probe consumes scheduling time without creating an OCR ticket.
    pub fn screen_probe(&mut self, visible: bool, closed: bool, now: u64) {
        self.gate.record_capture(now);
        if visible {
            self.screen_visible(now);
        } else if closed {
            self.gate.screen_closed();
        }
    }
    /// Visible selection pixels keep capture fast even when OCR cannot read a title.
    /// The completed OCR ticket already recorded this capture.
    pub fn screen_visible(&mut self, now: u64) {
        self.gate.screen_visible(now);
    }
    pub fn level_schedule(&self, now: u64) -> (crate::gate::Decision, Option<u8>) {
        (self.gate.decision(now), self.gate.next_level())
    }
    pub fn update_level(&mut self, level: u8, now: u64) {
        if self.phase == Phase::InProgress && !self.paused {
            self.gate.update_level(level, now);
        }
    }
    pub fn update_dead(&mut self, dead: Option<bool>, now: u64) {
        if self.phase == Phase::InProgress && !self.paused {
            self.gate.update_dead(dead, now);
        }
    }
    pub fn poll_cancel(&mut self, now: u64) -> Option<Ticket> {
        if let Worker::Running(ticket, started) = self.worker {
            if ticket.generation != self.generation || now.saturating_sub(started) >= 3000 {
                self.worker = Worker::Cancelling(ticket);
                return Some(ticket);
            }
        }
        None
    }
    pub fn worker_stopped(&mut self, ticket: Ticket, now: u64) -> bool {
        if let Worker::Cancelling(active) = self.worker {
            if active == ticket {
                self.worker = Worker::Idle;
                if ticket.generation == self.generation {
                    self.note_failure(now);
                }
                return true;
            }
        }
        false
    }
    fn note_failure(&mut self, now: u64) {
        // At most three consecutive failed jobs per context; success ends a failure streak.
        self.failures = self.failures.saturating_add(1);
        let delay = match self.failures {
            1 => 500,
            2 => 2000,
            _ => 8000,
        };
        self.retry_at = now.saturating_add(delay);
    }
    pub fn faulted(&self) -> bool {
        self.failures >= 3
    }
    pub fn begin(&mut self, key: FrameKey, now: u64) -> Option<Ticket> {
        if self.paused
            || self.phase != Phase::InProgress
            || !matches!(self.worker, Worker::Idle)
            || self.faulted()
            || now < self.retry_at
            || !self.gate.decision(now).capture
        {
            return None;
        }
        if self.last_frame.is_some_and(|old| {
            key.epoch < old.epoch || (key.epoch == old.epoch && key.sequence <= old.sequence)
        }) {
            return None;
        }
        self.sequence = self.sequence.checked_add(1)?;
        let ticket = Ticket {
            generation: self.generation,
            sequence: self.sequence,
        };
        self.last_frame = Some(key);
        self.gate.record_capture(now);
        self.worker = Worker::Running(ticket, now);
        Some(ticket)
    }
    pub fn fail(&mut self, ticket: Ticket, now: u64) -> bool {
        if let Worker::Running(active, _) = self.worker {
            if ticket == active {
                self.worker = Worker::Idle;
                if ticket.generation == self.generation {
                    self.note_failure(now);
                }
                return true;
            }
        }
        false
    }
    pub fn finish(
        &mut self,
        ticket: Ticket,
        slots: [Slot; 3],
        selection: Selection,
        now: u64,
    ) -> Option<Snapshot> {
        match self.worker {
            Worker::Running(active, started) if active == ticket => {
                self.worker = Worker::Idle;
                if now < started || now.saturating_sub(started) >= 3000 {
                    if ticket.generation == self.generation {
                        self.note_failure(now);
                    }
                    return None;
                }
            }
            Worker::Cancelling(active) if active == ticket => {
                self.worker = Worker::Idle;
                if ticket.generation == self.generation {
                    self.note_failure(now);
                }
                return None;
            }
            _ => return None,
        }
        if ticket.generation != self.generation || self.paused || self.phase != Phase::InProgress {
            return None;
        }
        let snapshot = Snapshot {
            schema_version: 1,
            generation: self.generation,
            sequence: ticket.sequence,
            dataset_version: self.dataset.clone(),
            selection_confirmed: false,
            actual_stage: ActualStage::Unknown,
            slots,
        };
        if !ContextGate::new(self.generation, self.dataset.clone()).accept(&snapshot) {
            self.note_failure(now);
            return None;
        }
        self.gate.observe(selection, now);
        self.failures = 0;
        self.retry_at = 0;
        Some(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_window_survives_pause_dataset_and_capture_changes_until_next_threshold() {
        let mut s = Session::new("zh-CN@v1".into()).unwrap();
        s.update_phase(Phase::InProgress).unwrap();
        s.update_level(3, 0);
        let t = s.begin(key(1), 0).unwrap();
        s.finish(t, blank(), Selection::Candidate, 1).unwrap();
        s.screen_probe(false, true, 1000);
        s.pause(true).unwrap();
        s.pause(false).unwrap();
        s.dataset_changed("zh-CN@v2".into()).unwrap();
        s.capture_replaced().unwrap();
        s.update_level(6, 2000);
        assert!(s.begin(key(2), 50000).is_none());
        s.update_level(7, 50001);
        let t = s.begin(key(3), 50001).unwrap();
        s.finish(t, blank(), Selection::Candidate, 50002).unwrap();
        s.screen_probe(false, true, 51000);
        assert!(s.begin(key(4), 52000).is_none());
        s.update_phase(Phase::PostGame).unwrap();
        s.update_phase(Phase::InProgress).unwrap();
        s.update_level(3, 53000);
        assert!(s.begin(key(5), 53000).is_some());
    }
    #[test]
    fn visible_selection_retries_unreadable_titles_at_fast_interval() {
        let mut s = Session::new("zh-CN@v1".into()).unwrap();
        s.update_phase(Phase::InProgress).unwrap();
        s.update_level(1, 0);
        s.screen_probe(true, false, 0);
        for sequence in 1..=30 {
            let now = sequence * 250;
            s.update_level(1, now);
            let ticket = s.begin(key(sequence), now).unwrap();
            assert!(s
                .finish(ticket, blank(), Selection::Unknown, now + 1)
                .is_some());
            s.screen_visible(now + 1);
            assert_eq!(s.level_schedule(now + 1).0.interval_ms, 250);
        }
        s.update_level(1, 14000);
        assert_eq!(s.level_schedule(14000).0.interval_ms, 8000);
    }
    #[test]
    fn a_success_resets_intermittent_failures_but_consecutive_failures_still_stop() {
        let mut s = Session::new("zh-CN@v1".into()).unwrap();
        s.update_phase(Phase::InProgress).unwrap();
        for round in 0..4 {
            let now = round * 5000;
            let failed = s.begin(key(round * 2 + 1), now).unwrap();
            assert!(s.fail(failed, now));
            let recovered = s.begin(key(round * 2 + 2), now + 1500).unwrap();
            assert!(s
                .finish(recovered, blank(), Selection::Unknown, now + 1501)
                .is_some());
            assert!(!s.faulted());
        }
        for (sequence, now) in [(9, 20000), (10, 22000), (11, 25000)] {
            let failed = s.begin(key(sequence), now).unwrap();
            assert!(s.fail(failed, now));
        }
        assert!(s.faulted());
        assert!(s.wake_at(40000).is_none());
    }
    #[test]
    fn locale_is_part_of_context_and_unrecognized_phase_cannot_enable_capture() {
        assert!(Session::new("v1".into()).is_err());
        assert_eq!(Phase::from_lcu(Some("Unauthorized")), Phase::Disconnected);
        let mut s = Session::new("zh-CN@v1".into()).unwrap();
        s.update_phase(Phase::InProgress).unwrap();
        let t = s.begin(key(1), 0).unwrap();
        s.dataset_changed("en-US@v1".into()).unwrap();
        assert!(s.finish(t, blank(), Selection::Candidate, 1).is_none());
        let t = s.begin(key(2), 2).unwrap();
        assert_eq!(
            s.finish(t, blank(), Selection::Unknown, 3)
                .unwrap()
                .dataset_version,
            "en-US@v1"
        );
    }
    #[test]
    fn late_completion_is_rejected_even_without_timer_poll_and_idle_has_no_timer() {
        let mut s = Session::new("zh-CN@v1".into()).unwrap();
        assert_eq!(s.wake_at(0), None);
        s.update_phase(Phase::InProgress).unwrap();
        let t = s.begin(key(1), 0).unwrap();
        assert_eq!(s.wake_at(0), Some(3000));
        assert!(s.finish(t, blank(), Selection::Candidate, 3001).is_none());
        assert_eq!(s.wake_at(3001), Some(3501));
        s.pause(true).unwrap();
        assert_eq!(s.wake_at(4000), None);
    }
    fn blank() -> [Slot; 3] {
        [Slot::Unreadable, Slot::Unreadable, Slot::Unreadable]
    }
    fn key(sequence: u64) -> FrameKey {
        FrameKey { epoch: 1, sequence }
    }
    #[test]
    fn timeout_never_starts_an_overlapping_worker_and_old_completion_is_rejected() {
        let mut s = Session::new("zh-CN@v1".into()).unwrap();
        s.update_phase(Phase::InProgress).unwrap();
        let old = s.begin(key(1), 0).unwrap();
        assert_eq!(s.poll_cancel(3000), Some(old));
        assert_eq!(s.poll_cancel(3001), None);
        assert!(s.begin(key(2), 4000).is_none());
        assert!(s.worker_stopped(old, 4000));
        assert!(s.begin(key(2), 4499).is_none());
        let new = s.begin(key(2), 4500).unwrap();
        assert!(s.finish(old, blank(), Selection::Unknown, 4501).is_none());
        assert!(s.finish(new, blank(), Selection::Unknown, 4502).is_some());
        assert!(s.begin(key(2), 9000).is_none());
    }
    #[test]
    fn pause_disconnect_dataset_and_capture_reset_reject_old_results() {
        let mut s = Session::new("zh-CN@v1".into()).unwrap();
        assert!(s.begin(key(1), 0).is_none());
        s.update_phase(Phase::Loading).unwrap();
        assert!(s.should_prewarm());
        s.update_phase(Phase::InProgress).unwrap();
        let old = s.begin(key(1), 0).unwrap();
        assert!(s.pause(true).unwrap().unwrap().stop_capture);
        assert!(s.finish(old, blank(), Selection::Candidate, 1).is_none());
        s.pause(false).unwrap();
        let old = s.begin(key(2), 2).unwrap();
        s.dataset_changed("zh-CN@v2".into()).unwrap();
        assert_eq!(s.poll_cancel(2), Some(old));
        assert!(s.worker_stopped(old, 3));
        let t = s.begin(key(3), 4).unwrap();
        let snapshot = s.finish(t, blank(), Selection::StatShard, 5).unwrap();
        assert_eq!(snapshot.dataset_version, "zh-CN@v2");
        assert_eq!(snapshot.actual_stage, ActualStage::Unknown);
        assert!(!snapshot.selection_confirmed);
        let t = s.begin(key(4), 2000).unwrap();
        s.capture_replaced().unwrap();
        assert!(s.finish(t, blank(), Selection::Candidate, 2001).is_none());
        let t = s.begin(key(5), 2002).unwrap();
        s.update_phase(Phase::Disconnected).unwrap();
        assert!(s.finish(t, blank(), Selection::Candidate, 2003).is_none());
    }
    #[test]
    fn repeated_failures_stop_until_context_is_explicitly_reset() {
        let mut s = Session::new("zh-CN@v1".into()).unwrap();
        s.update_phase(Phase::InProgress).unwrap();
        for i in 0..3 {
            let t = s.begin(key(i + 1), i * 10000).unwrap();
            assert!(s.fail(t, i * 10000 + 1));
        }
        assert!(s.faulted());
        assert!(s.begin(key(4), 100000).is_none());
        s.pause(true).unwrap();
        s.pause(false).unwrap();
        assert!(!s.faulted());
        assert!(s.begin(key(1), 100001).is_some());
    }
}
