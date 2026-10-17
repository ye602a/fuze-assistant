//! Bounded state transitions from the existing host poll. No files, network or raw payloads.
use fuze_worker_host::desktop::Status;
use serde_json::{json, Value};
use std::{collections::VecDeque, time::Instant};

const CAPACITY: usize = 128;
#[derive(Clone, PartialEq)]
struct State {
    connection: &'static str,
    phase: &'static str,
    capture: &'static str,
    ocr: &'static str,
    issue: &'static str,
}
struct Event {
    seconds: u64,
    state: State,
}
pub struct Recorder {
    started: Instant,
    events: VecDeque<Event>,
    dropped: u64,
    metrics: (u64, u64, Option<f64>, Option<u8>),
    last_analysis: Option<(u64, u64, f64)>,
}
impl Default for Recorder {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            events: VecDeque::with_capacity(CAPACITY),
            dropped: 0,
            metrics: (0, 0, None, None),
            last_analysis: None,
        }
    }
}
impl Recorder {
    pub fn observe(&mut self, status: &Status, offline: bool) -> bool {
        let view = crate::status_view::present(status, offline);
        // Free-form error text never crosses this boundary, including future errors.
        let issue = match status.pipeline.error {
            None => "none",
            Some("game-window-waiting") => "game-window-waiting",
            Some(e) if e.starts_with("capture") => "capture-failure",
            Some(e) if e.starts_with("game-window") => "game-window-failure",
            Some(e) if e.starts_with("ocr") || e.starts_with("worker") => "ocr-worker-failure",
            Some(_) => "pipeline-failure",
        };
        self.metrics = (
            status.pipeline.analyses,
            status.pipeline.skipped_frames,
            status
                .pipeline
                .analysis_ms
                .filter(|v| v.is_finite() && *v >= 0. && *v <= 3_600_000.),
            status.pipeline.level.filter(|n| (1..=30).contains(n)),
        );
        if let Some(ms) = self.metrics.2.filter(|_| self.metrics.0 > 0) {
            if self
                .last_analysis
                .is_none_or(|(_, count, old)| count != self.metrics.0 || old != ms)
            {
                self.last_analysis = Some((self.started.elapsed().as_secs(), self.metrics.0, ms));
            }
        }
        let state = State {
            connection: view.connection,
            phase: view.phase,
            capture: view.capture,
            ocr: view.ocr,
            issue,
        };
        if self.events.back().is_some_and(|event| event.state == state) {
            return false;
        }
        if self.events.len() == CAPACITY {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(Event {
            seconds: self.started.elapsed().as_secs(),
            state,
        });
        true
    }
    pub fn summary(&self) -> String {
        format!(
            "本次启动保留 {} 条状态变化（最多 128 条，已淘汰 {} 条）。仅存内存，退出后清空。",
            self.events.len(),
            self.dropped
        )
    }
    pub fn rows(&self) -> Vec<String> {
        self.events
            .iter()
            .rev()
            .take(8)
            .map(|e| {
                format!(
                    "+{}分{:02}秒 · {} · {}\n画面：{} · 识别：{} · {}",
                    e.seconds / 60,
                    e.seconds % 60,
                    e.state.connection,
                    e.state.phase,
                    e.state.capture,
                    e.state.ocr,
                    match e.state.issue {
                        "none" => "无链路错误",
                        "game-window-waiting" => "等待游戏窗口",
                        "capture-failure" => "截图异常",
                        "game-window-failure" => "窗口获取异常",
                        "ocr-worker-failure" => "识别引擎异常",
                        _ => "识别链路异常",
                    }
                )
            })
            .collect()
    }
    pub fn encode(&self, claim: Value) -> Result<Vec<u8>, &'static str> {
        let events = self.events.iter().map(|e|json!({
            "secondsSinceStart":e.seconds,"connection":e.state.connection,"phase":e.state.phase,
            "capture":e.state.capture,"ocr":e.state.ocr,"issue":e.state.issue
        })).collect::<Vec<_>>();
        let value = json!({
            "schema":1,"kind":"fuze-runtime-safe-feedback",
            "events":events,"droppedEvents":self.dropped,"latestMetrics":{
                "completedAnalyses":self.metrics.0,"skippedFrames":self.metrics.1,
                "lastAnalysisMs":self.metrics.2,"level":self.metrics.3,
                "scope":"latest-host-context-not-session-total"
            },
            "claimProbe":claim,"actualClaimRound":null,"recommendationScope":"overall",
            "lastObservedAnalysis":self.last_analysis.map(|(seconds,count,ms)|json!({
                "secondsSinceStart":seconds,"completedAnalysesInThatContext":count,"analysisMs":ms,
                "scope":"last-observed-completion-may-belong-to-previous-game"
            })),
            "limitations":["bounded-memory-state-changes","not-popup-latency",
                "not-cpu-gpu-measurement","not-claim-confirmation"]
        });
        let bytes = serde_json::to_vec_pretty(&value).map_err(|_| "export-encode")?;
        if bytes.len() > 128 * 1024 {
            return Err("export-limit");
        }
        Ok(bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_changes_are_bounded_and_private_host_fields_never_export() {
        let mut r = Recorder::default();
        let mut s = Status::default();
        s.available = Some("private-account-path".into());
        s.pipeline.error = Some("worker-private-token");
        assert!(r.observe(&s, false));
        s.pipeline.analyses = 77;
        assert!(!r.observe(&s, false));
        for i in 0..150 {
            s.mode = (i % 2) as u8;
            r.observe(&s, false);
        }
        assert_eq!(r.events.len(), 128);
        assert!(r.dropped > 0);
        assert_eq!(r.rows().len(), 8);
        s.pipeline.analysis_ms = Some(f64::NAN);
        r.observe(&s, false);
        let bytes = r.encode(Value::Null).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("private"));
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["latestMetrics"]["completedAnalyses"], 77);
        assert!(v["latestMetrics"]["lastAnalysisMs"].is_null());
        assert!(v["actualClaimRound"].is_null());
        s.pipeline.analysis_ms = Some(250.);
        r.observe(&s, false);
        s.pipeline = Default::default();
        r.observe(&s, false);
        let v: Value = serde_json::from_slice(&r.encode(Value::Null).unwrap()).unwrap();
        assert_eq!(v["lastObservedAnalysis"]["analysisMs"], 250.);
        assert!(v["latestMetrics"]["lastAnalysisMs"].is_null());
    }
}
