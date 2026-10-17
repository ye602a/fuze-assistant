use super::*;
use crate::desktop::Status;
fn state(phase: Phase) -> Snapshot {
    let mut s = Snapshot::from_status(&Status::default());
    s.phase = phase;
    s.capture = Capture::Waiting;
    s.ocr = Ocr::Waiting;
    s
}
#[test]
fn every_game_including_zero_analyses_uploads_once_and_disconnect_is_not_completion() {
    let mut t = Tracker::with_nonce(1);
    assert!(t.observe(state(Phase::Lobby), 0).is_none());
    assert!(t.observe(state(Phase::Loading), 1).is_none());
    assert!(t.observe(state(Phase::Playing), 2).is_none());
    assert!(t.observe(state(Phase::Unknown), 3).is_none());
    assert!(t.observe(state(Phase::Playing), 4).is_none());
    let first = t.observe(state(Phase::PostGame), 5).unwrap();
    assert_eq!(first.end, End::PostGame);
    assert_eq!(first.analysed, 0);
    assert!(t.observe(state(Phase::PostGame), 6).is_none());
    t.observe(state(Phase::Playing), 7);
    let second = t.observe(state(Phase::Lobby), 8).unwrap();
    assert_eq!(second.end, End::LeftMatch);
    assert!(second.started_in_progress);
    assert_ne!(first.id, second.id);
}
#[test]
fn fault_is_immediate_and_final_report_updates_same_issue_after_restart() {
    let mut t = Tracker::with_nonce(2);
    let mut s = state(Phase::Playing);
    t.observe(s.clone(), 0);
    s.capture = Capture::Failed;
    let early = t.observe(s.clone(), 100).unwrap();
    assert!(t.observe(s, 200).is_none());
    let final_report = t.observe(state(Phase::PostGame), 300).unwrap();
    assert_eq!(early.id, final_report.id);
    let mut store = Store::default();
    store.enqueue(early).unwrap();
    store.entries[0].issue = Some(42);
    store.entries[0].sent = true;
    let bytes = serde_json::to_vec(&store).unwrap();
    let mut restored: Store = serde_json::from_slice(&bytes).unwrap();
    restored.enqueue(final_report).unwrap();
    assert_eq!(restored.entries.len(), 1);
    assert_eq!(restored.entries[0].issue, Some(42));
    assert!(!restored.entries[0].sent);
    assert_eq!(restored.entries[0].report.end, End::PostGame);
}
#[test]
fn partial_not_full_unknown_or_stat_shards_is_an_actionable_read_failure() {
    use fuze_contract::wire::TitleRead::*;
    let mut t = Tracker::with_nonce(3);
    let mut s = state(Phase::Playing);
    s.analyses = 1;
    s.title_reads = [Empty; 3];
    assert!(t.observe(s.clone(), 1).is_none());
    s.analyses = 2;
    s.title_reads = [StatShard; 3];
    assert!(t.observe(s.clone(), 2).is_none());
    s.analyses = 3;
    s.title_reads = [Exact, NoCatalog, LowConfidence];
    let r = t.observe(s, 3).unwrap();
    assert_eq!(r.partial_observations, 1);
}
#[test]
fn report_is_bounded_and_does_not_copy_free_form_status_or_account_data() {
    let mut status = Status::default();
    status.preferences.manual_directory = "C:\\private-name\\private-token".into();
    status.ui_locale = "private-name".into();
    status.pipeline.error = Some("private-token");
    let mut s = Snapshot::from_status(&status);
    s.phase = Phase::Playing;
    let mut t = Tracker::with_nonce(4);
    for n in 0..300 {
        s.analyses = n;
        s.analysis_ms = Some(12.);
        t.observe(s.clone(), n * 30_000);
    }
    let report = t.finish().unwrap();
    assert_eq!(report.events.len(), MAX_EVENTS);
    let payload = String::from_utf8(report.payload().unwrap()).unwrap();
    assert!(!payload.contains("private-name"));
    assert!(!payload.contains("private-token"));
    assert!(!payload.contains("manual_directory"));
    assert!(payload.len() <= 60 * 1024);
    let mut invalid = serde_json::to_value(&report).unwrap();
    invalid["token"] = "private-token".into();
    assert!(serde_json::from_value::<Report>(invalid).is_err());
}
#[test]
fn outbox_capacity_and_interrupted_match_never_silently_become_confirmed_postgame() {
    let mut t = Tracker::with_nonce(5);
    let mut store = Store::default();
    for n in 0..MAX_QUEUE {
        t.observe(state(Phase::Playing), n as u64 * 10);
        store.enqueue(t.finish().unwrap()).unwrap();
    }
    assert!(store
        .entries
        .iter()
        .all(|e| e.report.end == End::AppStopped));
    t.observe(state(Phase::Playing), 400);
    assert_eq!(
        store.enqueue(t.finish().unwrap()),
        Err("feedback-queue-full")
    );
}
#[derive(Default)]
struct FakeApi {
    calls: Vec<Option<u64>>,
    found: Option<u64>,
    fail: Option<&'static str>,
}
impl Api for FakeApi {
    fn lookup(&mut self, _: &str, _: &str) -> Result<Option<u64>> {
        Ok(self.found)
    }
    fn send(&mut self, number: Option<u64>, payload: &[u8]) -> Result<u64> {
        let value: serde_json::Value = serde_json::from_slice(payload).unwrap();
        assert!(value["body"].as_str().unwrap().contains("实际领取轮次未知"));
        self.calls.push(number);
        if let Some(e) = self.fail {
            Err(e)
        } else {
            Ok(number.unwrap_or(42))
        }
    }
}
#[test]
fn whole_report_flow_creates_then_patches_and_uncertain_post_never_creates_twice() {
    let mut tracker = Tracker::with_nonce(6);
    let mut s = state(Phase::Playing);
    tracker.observe(s.clone(), 0);
    s.capture = Capture::Failed;
    let early = tracker.observe(s, 1).unwrap();
    let final_report = tracker.observe(state(Phase::PostGame), 2).unwrap();
    let mut store = Store::default();
    store.enqueue(early).unwrap();
    let mut api = FakeApi {
        fail: Some("feedback-network"),
        ..Default::default()
    };
    let saves = std::cell::RefCell::new(Vec::new());
    let save = |store: &Store| {
        saves.borrow_mut().push(serde_json::to_vec(store).unwrap());
        Ok(())
    };
    assert_eq!(
        upload(&mut store, 0, save, &mut api),
        Err("feedback-network")
    );
    assert!(store.entries[0].uncertain);
    // Simulate process restart after GitHub accepted a POST but its response was lost.
    store = serde_json::from_slice(saves.borrow().last().unwrap()).unwrap();
    api.fail = None;
    assert_eq!(
        upload(&mut store, 0, |_| Ok(()), &mut api),
        Err("feedback-uncertain")
    );
    assert_eq!(api.calls, vec![None]);
    api.found = Some(42);
    upload(&mut store, 0, |_| Ok(()), &mut api).unwrap();
    store.enqueue(final_report).unwrap();
    upload(&mut store, 0, |_| Ok(()), &mut api).unwrap();
    assert_eq!(api.calls, vec![None, Some(42), Some(42)]);
    assert!(store.entries.is_empty());
    assert_eq!(store.latest_issue, Some(42));
}
#[test]
fn refused_or_unsent_requests_can_retry_and_unwritable_queue_never_posts() {
    let mut t = Tracker::with_nonce(7);
    t.observe(state(Phase::Playing), 0);
    let r = t.finish().unwrap();
    let mut store = Store::default();
    store.enqueue(r).unwrap();
    let mut api = FakeApi::default();
    assert_eq!(
        upload(&mut store, 0, |_| Err("feedback-store"), &mut api),
        Err("feedback-store")
    );
    assert!(api.calls.is_empty());
    assert!(!store.entries[0].uncertain);
    api.fail = Some("feedback-not-sent");
    assert_eq!(
        upload(&mut store, 0, |_| Ok(()), &mut api),
        Err("feedback-not-sent")
    );
    assert!(!store.entries[0].uncertain);
    api.fail = None;
    upload(&mut store, 0, |_| Ok(()), &mut api).unwrap();
    assert!(store.entries.is_empty());
}
#[test]
fn counters_are_per_observed_match_not_cumulative_host_lifetime() {
    let mut t = Tracker::with_nonce(8);
    let mut s = state(Phase::Loading);
    s.analyses = 200;
    t.observe(s.clone(), 0);
    s.phase = Phase::Playing;
    s.analyses = 203;
    t.observe(s.clone(), 1);
    s.phase = Phase::PostGame;
    assert_eq!(t.observe(s.clone(), 2).unwrap().analysed, 3);
    s.phase = Phase::Loading;
    t.observe(s.clone(), 3);
    s.phase = Phase::Playing;
    s.analyses = 205;
    t.observe(s, 4);
    assert_eq!(t.finish().unwrap().analysed, 2);
}
#[test]
fn persistent_outbox_is_exclusive_and_corrupt_existing_data_is_preserved() {
    let parent = std::env::temp_dir();
    let dir = parent.join(format!(
        "fuze-feedback-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = dir.join("github-feedback-v1.json");
    let guard = lock(&path).unwrap();
    assert!(lock(&path).is_err());
    let mut tracker = Tracker::with_nonce(9);
    tracker.observe(state(Phase::Playing), 0);
    let mut store = Store::default();
    store.enabled = true;
    store.enqueue(tracker.finish().unwrap()).unwrap();
    store.save(&path).unwrap();
    let restored = Store::load(&path).unwrap();
    assert!(restored.enabled);
    assert_eq!(restored.entries.len(), 1);
    std::fs::write(&path, b"not-a-report").unwrap();
    assert!(Store::load(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"not-a-report");
    drop(guard);
    assert_eq!(dir.parent(), Some(parent.as_path()));
    assert!(dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("fuze-feedback-fixture-"));
    std::fs::remove_dir_all(dir).unwrap();
}
