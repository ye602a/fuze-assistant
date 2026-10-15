//! Scheduling hints are deliberately separate from proof of a formal reward claim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Selection {
    Augment,
    StatShard,
    Unknown,
    Absent,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Fallback,
    Selection,
    Pending,
    Preparing,
    Saving,
}
#[derive(Debug, Clone, Copy)]
pub struct Decision {
    pub mode: Mode,
    pub capture: bool,
    pub prepare: bool,
    pub interval_ms: u64,
}
#[derive(Default)]
pub struct LevelGate {
    level: Option<(u8, u64)>,
    // Suppresses repeated high-frequency windows only; NEVER exported as actualStage.
    finished_windows: u8,
    observed: Option<usize>,
    visible: bool,
    absent_since: Option<u64>,
    misses: u8,
    last_probe: Option<u64>,
    last_absent: bool,
}
impl LevelGate {
    pub fn clear_level(&mut self) {
        self.level = None;
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn update_level(&mut self, level: u8, now: u64) {
        if !(1..=30).contains(&level) || self.level.is_some_and(|(_, at)| now < at) {
            return;
        }
        if self.level.is_some_and(|(old, _)| level < old) {
            self.reset();
        }
        self.level = Some((level, now));
    }
    pub fn observe(&mut self, selection: Selection, now: u64) {
        self.last_absent = selection == Selection::Absent;
        if selection != Selection::Absent {
            self.visible = true;
            self.absent_since = None;
            self.misses = 0;
            if selection == Selection::Augment {
                if self.observed.is_none() {
                    self.observed = [3, 7, 11, 15]
                        .iter()
                        .enumerate()
                        .find(|(i, n)| {
                            self.level.is_some_and(|(level, _)| level >= **n)
                                && self.finished_windows & (1 << i) == 0
                        })
                        .map(|(i, _)| i);
                }
            } else {
                self.observed = None;
            }
        } else if self.visible {
            let start = *self.absent_since.get_or_insert(now);
            self.misses = self.misses.saturating_add(1);
            if self.misses >= 3 && now.saturating_sub(start) >= 6000 {
                if let Some(stage) = self.observed.take() {
                    self.finished_windows |= 1 << stage;
                }
                self.visible = false;
            }
        }
    }
    pub fn unchanged_frame(&mut self, now: u64) {
        if self.visible && self.last_absent {
            self.observe(Selection::Absent, now);
        }
    }
    pub fn decision(&self, now: u64) -> Decision {
        let level = self
            .level
            .filter(|(_, at)| now >= *at && now - *at <= 10000)
            .map(|(v, _)| v);
        let next = [3, 7, 11, 15]
            .into_iter()
            .enumerate()
            .find(|(i, _)| self.finished_windows & (1 << i) == 0)
            .map(|(_, n)| n);
        let pending = level.zip(next).is_some_and(|(l, n)| l >= n);
        let preparing = level.zip(next).is_some_and(|(l, n)| l == n - 1);
        let mode = if self.visible {
            Mode::Selection
        } else if level.is_none() {
            Mode::Fallback
        } else if pending {
            Mode::Pending
        } else if preparing {
            Mode::Preparing
        } else {
            Mode::Saving
        };
        let interval_ms = match mode {
            Mode::Saving => 8000,
            Mode::Fallback => 1500,
            _ => 250,
        };
        let capture = self
            .last_probe
            .is_none_or(|at| now >= at && now - at >= interval_ms);
        Decision {
            mode,
            capture,
            prepare: pending || preparing,
            interval_ms,
        }
    }
    pub fn record_capture(&mut self, now: u64) {
        self.last_probe = Some(now);
    }
    pub fn wake_at(&self, now: u64) -> u64 {
        let d = self.decision(now);
        let due = self
            .last_probe
            .map_or(now, |last| last.saturating_add(d.interval_ms).max(now));
        // Wake when telemetry expires, even if the saving-mode watchdog was later.
        self.level.map_or(due, |(_, at)| {
            let stale = at.saturating_add(10001);
            if stale > now {
                due.min(stale)
            } else {
                due
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lost_telemetry_returns_to_watchdog_without_changing_selection() {
        let mut g = LevelGate::default();
        g.update_level(1, 0);
        g.record_capture(0);
        assert_eq!(g.decision(100).interval_ms, 8000);
        g.clear_level();
        assert_eq!(g.decision(1500).interval_ms, 1500);
        assert!(g.decision(1500).capture);
        g.observe(Selection::StatShard, 1600);
        g.clear_level();
        assert_eq!(g.decision(1700).interval_ms, 250);
    }
    #[test]
    fn telemetry_expiry_wakes_before_later_watchdog_and_preview_does_not_consume_probe() {
        let mut g = LevelGate::default();
        g.update_level(1, 0);
        g.record_capture(8000);
        assert_eq!(g.wake_at(8000), 10001);
        assert!(g.decision(10001).capture);
        assert!(g.decision(10001).capture);
        g.record_capture(10001);
        assert!(!g.decision(10002).capture);
    }
    #[test]
    fn shards_rerolls_and_watchdog_never_establish_claims() {
        let mut g = LevelGate::default();
        g.update_level(3, 0);
        g.observe(Selection::Augment, 0);
        g.observe(Selection::Absent, 100);
        g.observe(Selection::Augment, 300);
        assert_eq!(g.decision(400).mode, Mode::Selection);
        g.observe(Selection::StatShard, 500);
        for now in [600, 2000, 7000] {
            g.observe(Selection::Absent, now);
        }
        assert_eq!(g.decision(7000).mode, Mode::Pending);
        g.observe(Selection::Augment, 7100);
        for now in [7200, 8000, 13200] {
            g.observe(Selection::Absent, now);
        }
        g.update_level(3, 13200);
        g.record_capture(13200);
        assert_eq!(g.decision(13201).mode, Mode::Saving);
        assert!(!g.decision(21200 - 1).capture);
        assert!(g.decision(21200).capture);
    }
    #[test]
    fn missing_telemetry_and_unchanged_absence_do_not_freeze_detection() {
        let mut g = LevelGate::default();
        g.update_level(2, 100);
        assert_eq!(g.decision(100).mode, Mode::Preparing);
        assert_eq!(g.decision(10200).mode, Mode::Fallback);
        g.update_level(3, 10300);
        g.observe(Selection::Augment, 10300);
        g.observe(Selection::Absent, 10400);
        g.unchanged_frame(12000);
        g.unchanged_frame(16400);
        assert_eq!(g.decision(16400).mode, Mode::Saving);
        g.update_level(1, 16500);
        assert_eq!(g.decision(16500).mode, Mode::Saving);
        g.update_level(3, 16400);
        assert_eq!(g.decision(16500).mode, Mode::Saving);
    }
}
