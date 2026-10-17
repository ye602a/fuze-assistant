//! Capture scheduling only. A burst ending never establishes a formal reward or claim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Selection {
    Candidate,
    StatShard,
    Unknown,
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
const BURST_MS: u64 = 20_000;
const EVIDENCE_HOLD_MS: u64 = 6_000;
const LEVEL_TTL_MS: u64 = 10_000;
#[derive(Default)]
pub struct LevelGate {
    level: Option<(u8, u64)>,
    band: u8,
    burst_started: Option<u64>,
    last_evidence: Option<u64>,
    unreadable_since: Option<u64>,
    last_probe: Option<u64>,
}
// Only approaching/crossing a threshold starts a burst; ordinary telemetry cannot renew it.
fn band(level: u8) -> u8 {
    match level {
        0..=1 => 0,
        2 | 6 | 10 | 14 => level,
        3..=5 => 3,
        7..=9 => 7,
        11..=13 => 11,
        _ => 15,
    }
}
impl LevelGate {
    /// Next level threshold, not an unclaimed reward or actual selection stage.
    pub fn next_level(&self) -> Option<u8> {
        let level = self.level.map_or(0, |(v, _)| v);
        [3, 7, 11, 15].into_iter().find(|n| *n > level)
    }
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
        let next = band(level);
        if next != self.band {
            self.band = next;
            self.burst_started = (next != 0).then_some(now);
            if self
                .last_evidence
                .is_some_and(|at| now.saturating_sub(at) >= EVIDENCE_HOLD_MS)
            {
                self.last_evidence = None;
                self.unreadable_since = None;
            }
        }
        self.level = Some((level, now));
    }
    pub fn observe(&mut self, selection: Selection, now: u64) {
        if selection != Selection::Unknown {
            self.last_evidence = Some(now);
            self.unreadable_since = None;
        } else if self.last_evidence.is_some() {
            let since = *self.unreadable_since.get_or_insert(now);
            if now.saturating_sub(since) >= EVIDENCE_HOLD_MS {
                // Occlusion/rerolls can resemble closure. End only fast capture; a watchdog
                // continues to find delayed formal choices after shards. No claim counter.
                self.last_evidence = None;
                self.burst_started = None;
            }
        }
    }
    pub fn decision(&self, now: u64) -> Decision {
        let level = self
            .level
            .filter(|(_, at)| now >= *at && now - *at <= LEVEL_TTL_MS)
            .map(|(v, _)| v);
        let evidence = self
            .last_evidence
            .is_some_and(|at| now >= at && now - at < EVIDENCE_HOLD_MS);
        let burst = self
            .burst_started
            .is_some_and(|at| now >= at && now - at < BURST_MS);
        let mode = if evidence {
            Mode::Selection
        } else if level.is_none() {
            Mode::Fallback
        } else if level == Some(1) {
            Mode::Saving
        } else if burst && self.last_evidence.is_none() {
            if matches!(self.band, 2 | 6 | 10 | 14) {
                Mode::Preparing
            } else {
                Mode::Pending
            }
        } else {
            Mode::Fallback
        };
        let interval_ms = match mode {
            Mode::Saving => 8000,
            Mode::Fallback => {
                if level.is_some() {
                    1000
                } else {
                    1500
                }
            }
            _ => 250,
        };
        Decision {
            mode,
            capture: self
                .last_probe
                .is_none_or(|at| now >= at && now - at >= interval_ms),
            prepare: matches!(mode, Mode::Preparing | Mode::Pending | Mode::Selection),
            interval_ms,
        }
    }
    pub fn record_capture(&mut self, now: u64) {
        self.last_probe = Some(now);
    }
    pub fn wake_at(&self, now: u64) -> u64 {
        let due = self.last_probe.map_or(now, |last| {
            last.saturating_add(self.decision(now).interval_ms).max(now)
        });
        self.level.map_or(due, |(_, at)| {
            let stale = at.saturating_add(LEVEL_TTL_MS + 1);
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
    fn production_candidate_and_unknown_end_burst_without_claim_counting() {
        let mut g = LevelGate::default();
        g.update_level(3, 0);
        g.observe(Selection::Candidate, 0);
        g.observe(Selection::Unknown, 100);
        assert_eq!(g.decision(100).mode, Mode::Selection);
        g.observe(Selection::Unknown, 6100);
        g.update_level(3, 6100);
        assert_eq!(g.decision(6100).interval_ms, 1000);
        assert_eq!(g.next_level(), Some(7));
        g.observe(Selection::Candidate, 7100);
        assert_eq!(g.decision(7100).interval_ms, 250);
    }
    #[test]
    fn no_popup_or_midgame_start_never_keeps_high_frequency_indefinitely() {
        for level in [2, 3, 7, 11, 15, 18] {
            let mut g = LevelGate::default();
            g.update_level(level, 0);
            assert_eq!(g.decision(0).interval_ms, 250);
            for t in [8000, 16000, 24000, 32000] {
                g.update_level(level, t);
                g.observe(Selection::Unknown, t);
            }
            assert_eq!(g.decision(32000).interval_ms, 1000);
        }
    }
    #[test]
    fn rerolls_shards_and_delayed_choices_restore_fast_detection_without_advancing_stage() {
        let mut g = LevelGate::default();
        g.update_level(7, 0);
        for (at, selection) in [
            (0, Selection::Candidate),
            (100, Selection::Unknown),
            (400, Selection::Candidate),
            (500, Selection::StatShard),
            (700, Selection::Unknown),
            (6700, Selection::Unknown),
        ] {
            g.observe(selection, at);
        }
        assert_eq!(g.decision(6700).interval_ms, 1000);
        assert_eq!(g.next_level(), Some(11));
        g.observe(Selection::Candidate, 7700);
        assert_eq!(g.decision(7700).mode, Mode::Selection);
        assert_eq!(g.next_level(), Some(11));
    }
    #[test]
    fn lost_telemetry_and_missing_frames_expire_without_freezing_or_rearming_burst() {
        let mut g = LevelGate::default();
        g.update_level(1, 0);
        g.record_capture(8000);
        assert_eq!(g.wake_at(8000), 10001);
        g.clear_level();
        assert_eq!(g.decision(9500).interval_ms, 1500);
        g.update_level(3, 10000);
        g.observe(Selection::Candidate, 10000);
        g.update_level(3, 17000);
        assert_eq!(g.decision(17000).interval_ms, 1000);
        g.update_level(7, 18000);
        assert_eq!(g.decision(18000).mode, Mode::Pending);
        g.update_level(1, 25000);
        assert_eq!(g.decision(25000).mode, Mode::Saving);
        g.update_level(15, 24000);
        assert_eq!(g.next_level(), Some(3));
    }
}
