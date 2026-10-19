//! Explicitly enabled GitHub feedback. A typed allowlist, never a raw log uploader.
use crate::{
    runtime_log::{Capture, Issue, Ocr, Phase, Snapshot},
    Result,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
#[path = "github_feedback_io.rs"]
mod io;
#[cfg(test)]
#[path = "github_feedback_tests.rs"]
mod tests;
pub const REPOSITORY: &str = "ye602a/fuze-assistant";
const TARGET: &str = "Fuze/Rust/GitHubFeedback/ye602a/fuze-assistant";
const MAX_EVENTS: usize = 48;
const MAX_QUEUE: usize = 32;
pub fn configure_credential() -> Result<()> {
    crate::update_credential::prompt_target(TARGET, "符择 · 自动诊断上传凭据", "密码栏填写仅 ye602a/fuze-assistant 的 Issues 读写令牌。独立于更新令牌；不需要 Contents 写入权限。令牌只保存在本机 Windows 凭据管理器。")
}
pub fn remove_credential() -> Result<()> {
    crate::update_credential::remove_target(TARGET)
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum End {
    Fault,
    PostGame,
    LeftMatch,
    AppStopped,
    RecoveredPartial,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    elapsed_ms: u64,
    state: Snapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    #[serde(default)]
    recovered: bool,
    #[serde(default)]
    incomplete_log: bool,
    id: String,
    end: End,
    started_in_progress: bool,
    duration_ms: u64,
    analysed: u64,
    partial_observations: u64,
    peak_analysis_ms: Option<f64>,
    peak_cpu_percent: Option<f64>,
    peak_memory_mb: Option<f64>,
    first_fault: Option<Event>,
    dropped_events: u64,
    events: VecDeque<Event>,
    #[serde(
        default,
        rename = "packageRevision",
        skip_serializing_if = "Option::is_none"
    )]
    package_revision: Option<crate::build_identity::Revision>,
    #[serde(
        default,
        rename = "runtimeRevision",
        skip_serializing_if = "Option::is_none"
    )]
    runtime_revision: Option<crate::build_identity::Revision>,
}
impl Report {
    fn title(&self) -> String {
        format!("FUZE 诊断 · {}", self.id)
    }
    fn valid(&self) -> bool {
        self.id.len() == 32
            && self.id.bytes().all(|b| b.is_ascii_hexdigit())
            && self.events.len() <= MAX_EVENTS
            && self.duration_ms <= 86_400_000
            && self.analysed <= 10_000_000
            && self.partial_observations <= 10_000_000
            && self
                .peak_analysis_ms
                .is_none_or(|n| n.is_finite() && (0. ..=3_600_000.).contains(&n))
            && self
                .peak_cpu_percent
                .is_none_or(|n| n.is_finite() && (0. ..=100.).contains(&n))
            && self
                .peak_memory_mb
                .is_none_or(|n| n.is_finite() && (0. ..=1_048_576.).contains(&n))
            && self.events.iter().chain(self.first_fault.iter()).all(|e| {
                e.elapsed_ms <= 86_400_000
                    && crate::runtime_log::Record {
                        schema: 1,
                        runtime_revision: None,
                        event: crate::runtime_log::Event::State,
                        elapsed_ms: e.elapsed_ms,
                        state: e.state.clone(),
                        dropped: 0,
                    }
                    .valid()
            })
    }
    fn payload(&self) -> Result<Vec<u8>> {
        if !self.valid() {
            return Err("feedback-invalid");
        }
        let safe = serde_json::to_string_pretty(self).map_err(|_| "feedback-invalid")?;
        let status = match self.end {
            End::Fault => "局中故障快照，赛后会更新",
            End::PostGame => "已观察到赛后阶段",
            End::LeftMatch => "已离开对局，未观察到结算阶段",
            End::AppStopped => "助手退出，记录不完整",
            End::RecoveredPartial => "本地记录未包含明确结束阶段，摘要不完整",
        };
        let source = if self.recovered {
            "手动补传：从有限本地日志恢复，可能缺失早期记录；不是实时上传或实际领取证明。"
        } else {
            "自动跟踪：开启后的对局记录。"
        };
        let body = format!("符择诊断 · {status}\n{source}\n\n反馈格式：rust-public-feedback-2；截图来源和回退状态以记录字段为准。\npackageRevision 是报告生成包；runtimeRevision 是记录对应的运行包，缺失表示未知，不能从生成包推断历史版本。\n仅受限构建标识、固定状态、相对耗时、计数及三个卡位的识别分类。没有账号、对局 ID、截图、路径、OCR 原文或凭据。\n整体胜率推荐；实际领取轮次未知。分析耗时不代表弹窗出现后的等待时间。\n\n```json\n{safe}\n```\n");
        let bytes = serde_json::to_vec(&serde_json::json!({"title": self.title(), "body": body}))
            .map_err(|_| "feedback-invalid")?;
        if bytes.len() > 60 * 1024 {
            return Err("feedback-limit");
        }
        Ok(bytes)
    }
}
/// In-memory match tracking. A disconnect or pause never masquerades as a finished match.
pub struct Tracker {
    revision: Option<crate::build_identity::Revision>,
    nonce: u64,
    serial: u64,
    active: Option<Report>,
    start: u64,
    previous_analyses: u64,
    fault_sent: bool,
}
impl Tracker {
    pub fn new() -> Result<Self> {
        let mut bytes = [0u8; 8];
        unsafe {
            use windows_sys::Win32::Security::Cryptography::*;
            if BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                8,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            ) < 0
            {
                return Err("feedback-random");
            }
        }
        Ok(Self::with_revision(
            u64::from_le_bytes(bytes),
            crate::build_identity::current(),
        ))
    }
    fn with_nonce(nonce: u64) -> Self {
        Self::with_revision(nonce, None)
    }
    fn with_revision(nonce: u64, revision: Option<crate::build_identity::Revision>) -> Self {
        Self {
            revision,
            nonce,
            serial: 0,
            active: None,
            start: 0,
            previous_analyses: 0,
            fault_sent: false,
        }
    }
    pub fn observe(&mut self, snapshot: Snapshot, now: u64) -> Option<Report> {
        if self.active.is_none() && matches!(snapshot.phase, Phase::Loading | Phase::Playing) {
            self.serial += 1;
            self.start = now;
            // Controller's counter spans matches. Count only increments observed in this report.
            self.previous_analyses = snapshot.analyses;
            self.fault_sent = false;
            self.active = Some(Report {
                recovered: false,
                incomplete_log: false,
                id: format!("{:016x}{:016x}", self.nonce, self.serial),
                end: End::Fault,
                started_in_progress: snapshot.phase == Phase::Playing,
                duration_ms: 0,
                analysed: 0,
                partial_observations: 0,
                peak_analysis_ms: None,
                peak_cpu_percent: None,
                peak_memory_mb: None,
                first_fault: None,
                dropped_events: 0,
                events: VecDeque::new(),
                package_revision: self.revision.clone(),
                runtime_revision: self.revision.clone(),
            });
        }
        let r = self.active.as_mut()?;
        r.duration_ms = now.saturating_sub(self.start).min(86_400_000);
        if let Some(n) = snapshot.process_cpu_percent {
            r.peak_cpu_percent = Some(r.peak_cpu_percent.unwrap_or(0.).max(n));
        }
        if let Some(n) = snapshot.process_memory_mb {
            r.peak_memory_mb = Some(r.peak_memory_mb.unwrap_or(0.).max(n));
        }
        let new_analysis = snapshot.analyses != self.previous_analyses;
        if new_analysis {
            r.analysed = (r.analysed
                + if snapshot.analyses >= self.previous_analyses {
                    snapshot.analyses - self.previous_analyses
                } else {
                    snapshot.analyses
                })
            .min(10_000_000);
            self.previous_analyses = snapshot.analyses;
            if let Some(ms) = snapshot.analysis_ms {
                r.peak_analysis_ms = Some(r.peak_analysis_ms.unwrap_or(0.).max(ms));
            }
        }
        let reads = snapshot.title_reads;
        use fuze_contract::wire::TitleRead::*;
        let known = reads
            .iter()
            .filter(|r| matches!(r, Exact | Alias | Fuzzy))
            .count();
        let partial = new_analysis && (1..3).contains(&known);
        if partial {
            r.partial_observations = (r.partial_observations + 1).min(10_000_000);
        }
        let fault = snapshot.capture == Capture::Failed
            || snapshot.ocr == Ocr::Failed
            || matches!(snapshot.issue, Issue::Window | Issue::Pipeline)
            || partial;
        if fault && r.first_fault.is_none() {
            r.first_fault = Some(Event {
                elapsed_ms: r.duration_ms,
                state: snapshot.clone(),
            });
        }
        let changed = r.events.back().is_none_or(|e| {
            !e.state.same_state(&snapshot) || r.duration_ms.saturating_sub(e.elapsed_ms) >= 30_000
        });
        let ending = match snapshot.phase {
            Phase::PostGame => Some(End::PostGame),
            Phase::Lobby | Phase::Selection => Some(End::LeftMatch),
            _ => None,
        };
        if changed || ending.is_some() {
            if r.events.len() == MAX_EVENTS {
                r.events.pop_front();
                r.dropped_events += 1;
            }
            r.events.push_back(Event {
                elapsed_ms: r.duration_ms,
                state: snapshot,
            });
        }
        if let Some(end) = ending {
            r.end = end;
            return self.active.take();
        }
        if fault && !self.fault_sent {
            self.fault_sent = true;
            return Some(r.clone());
        }
        None
    }
    pub fn finish(&mut self) -> Option<Report> {
        let mut r = self.active.take()?;
        r.end = End::AppStopped;
        Some(r)
    }
    pub fn recording(&self) -> bool {
        self.active.is_some()
    }
}
/// Explicit recovery only, on the feedback worker. Typed allowlist is applied again.
fn recover_recent(value: serde_json::Value) -> Result<Report> {
    recover_with_revision(value, crate::build_identity::current())
}
fn recover_with_revision(
    value: serde_json::Value,
    exporter: Option<crate::build_identity::Revision>,
) -> Result<Report> {
    let rows: Vec<crate::runtime_log::Record> =
        serde_json::from_value(value["records"].clone()).map_err(|_| "feedback-no-local-match")?;
    let mut tracker = Tracker::with_nonce(0);
    let mut latest = None;
    let mut previous = 0;
    let mut previous_revision = None;
    let mut seen = false;
    for row in rows {
        if !row.valid() {
            return Err("feedback-invalid");
        }
        // A version boundary is also a session boundary when rotation removed its start line.
        if !seen
            || row.event == crate::runtime_log::Event::Start
            || row.elapsed_ms < previous
            || row.runtime_revision != previous_revision
        {
            if let Some(r) = tracker.finish() {
                latest = Some(r);
            }
            tracker = Tracker::with_revision(0, row.runtime_revision.clone());
        }
        seen = true;
        previous_revision = row.runtime_revision;
        previous = row.elapsed_ms;
        if let Some(report) = tracker.observe(row.state, row.elapsed_ms) {
            if report.end != End::Fault {
                latest = Some(report);
            }
        }
    }
    let mut report = tracker
        .finish()
        .or(latest)
        .ok_or("feedback-no-local-match")?;
    report.recovered = true;
    report.incomplete_log = value["status"].as_str() != Some("ok")
        || report.started_in_progress
        || report.end == End::AppStopped;
    if report.end == End::AppStopped {
        report.end = End::RecoveredPartial;
    }
    // Once per explicit recovery, never per frame. Stable identity prevents repeated manual POSTs.
    report.id.clear();
    // Preserve the exact legacy hash when runtime identity is absent. Exporting the
    // same logs from a newer package must not create a second issue or bypass a receipt.
    report.package_revision = None;
    let bytes = serde_json::to_vec(&report).map_err(|_| "feedback-invalid")?;
    report.id = crate::update_io::hex(&crate::update_io::digest(&bytes, false)?)[..32].into();
    report.package_revision = exporter;
    if !report.valid() {
        return Err("feedback-invalid");
    }
    Ok(report)
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub uploaded_this_run: u64,
    pub authorization: &'static str,
    pub enabled: bool,
    pub ready: bool,
    pub pending: usize,
    pub issue: Option<u64>,
    pub status: &'static str,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    report: Report,
    issue: Option<u64>,
    uncertain: bool,
    sent: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    #[serde(default)]
    recovered_receipt: Option<(String, u64)>,
    enabled: bool,
    entries: VecDeque<Entry>,
    latest_issue: Option<u64>,
}
impl Store {
    fn load(path: &Path) -> Result<Self> {
        regular(path, 2 * 1024 * 1024)?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let value: Self =
            serde_json::from_slice(&crate::update_channel::bounded(path, 2 * 1024 * 1024)?)
                .map_err(|_| "feedback-store")?;
        if value.entries.len() > MAX_QUEUE
            || value.recovered_receipt.as_ref().is_some_and(|(id, issue)| {
                id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) || *issue == 0
            })
            || value
                .entries
                .iter()
                .any(|e| !e.report.valid() || e.issue == Some(0))
        {
            return Err("feedback-store");
        }
        Ok(value)
    }
    fn enqueue(&mut self, report: Report) -> Result<()> {
        if !report.valid() {
            return Err("feedback-invalid");
        }
        if report.recovered {
            if let Some((id, issue)) = &self.recovered_receipt {
                if id == &report.id {
                    self.latest_issue = Some(*issue);
                    return Err("feedback-already-sent");
                }
            }
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.report.id == report.id) {
            entry.report = report;
            entry.sent = false;
        } else {
            if self.entries.len() >= MAX_QUEUE {
                return Err("feedback-queue-full");
            }
            self.entries.push_back(Entry {
                report,
                issue: None,
                uncertain: false,
                sent: false,
            });
        }
        Ok(())
    }
    fn save(&self, path: &Path) -> Result<()> {
        regular(path, 2 * 1024 * 1024)?;
        crate::preferences::save_json(path, self).map_err(|_| "feedback-store")
    }
}
enum Command {
    Enable(bool),
    Report(Report),
    Retry,
    Verify,
    Recover(Option<mpsc::Receiver<Result<serde_json::Value>>>),
    Stop(Vec<Report>),
}
trait Api {
    fn lookup(&mut self, id: &str, title: &str) -> Result<Option<u64>>;
    fn send(&mut self, issue: Option<u64>, payload: &[u8]) -> Result<u64>;
}
struct Network<'a> {
    secret: &'a crate::update_credential::Secret,
    cancel: &'a std::sync::atomic::AtomicBool,
}
impl Api for Network<'_> {
    fn lookup(&mut self, id: &str, title: &str) -> Result<Option<u64>> {
        io::lookup(id, title, self.secret, self.cancel)
    }
    fn send(&mut self, issue: Option<u64>, payload: &[u8]) -> Result<u64> {
        io::send(issue, payload, self.secret, self.cancel)
    }
}
fn upload(
    store: &mut Store,
    index: usize,
    save: impl Fn(&Store) -> Result<()>,
    api: &mut impl Api,
) -> Result<()> {
    if store.entries[index].uncertain && store.entries[index].issue.is_none() {
        let report = &store.entries[index].report;
        let found = api.lookup(&report.id, &report.title())?;
        store.entries[index].issue = found;
        if found.is_none() {
            return Err("feedback-uncertain");
        }
    }
    let payload = store.entries[index].report.payload()?;
    let previous = store.entries[index].uncertain;
    store.entries[index].uncertain = true;
    if let Err(error) = save(store) {
        store.entries[index].uncertain = previous;
        return Err(error);
    }
    match api.send(store.entries[index].issue, &payload) {
        Ok(number) => {
            let e = &mut store.entries[index];
            e.issue = Some(number);
            e.uncertain = false;
            e.sent = true;
            if e.report.recovered {
                store.recovered_receipt = Some((e.report.id.clone(), number));
            }
            store.latest_issue = Some(number);
            if e.report.end != End::Fault {
                store.entries.remove(index);
            }
            save(store)?;
        }
        Err(e) => {
            if matches!(
                e,
                "feedback-not-sent"
                    | "feedback-auth"
                    | "feedback-permission"
                    | "feedback-rate-limit"
                    | "feedback-rejected"
            ) {
                store.entries[index].uncertain = false;
                save(store)?;
            }
            return Err(e);
        }
    }
    Ok(())
}
pub struct Client {
    tx: mpsc::SyncSender<Command>,
    view: Arc<Mutex<View>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Client {
    pub fn start() -> Result<Self> {
        Self::at(crate::preferences::path()?.with_file_name("github-feedback-v1.json"))
    }
    fn at(path: PathBuf) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel(16);
        let view = Arc::new(Mutex::new(View::default()));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (v, c) = (view.clone(), cancel.clone());
        let thread = std::thread::Builder::new()
            .name("fuze-github-feedback".into())
            .spawn(move || worker(path, rx, v, c))
            .map_err(|_| "feedback-worker")?;
        Ok(Self {
            tx,
            view,
            cancel,
            thread: Some(thread),
        })
    }
    pub fn view(&self) -> View {
        self.view.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn enable(&self, enabled: bool) -> Result<()> {
        if !enabled {
            self.cancel
                .store(true, std::sync::atomic::Ordering::Release);
        }
        self.tx
            .try_send(Command::Enable(enabled))
            .map_err(|_| "feedback-busy")
    }
    pub fn submit(&self, report: Report) -> Result<()> {
        self.tx
            .try_send(Command::Report(report))
            .map_err(|_| "feedback-busy")
    }
    pub fn retry(&self) -> Result<()> {
        self.tx
            .try_send(Command::Retry)
            .map_err(|_| "feedback-busy")
    }
    pub fn verify(&self) -> Result<()> {
        self.tx
            .try_send(Command::Verify)
            .map_err(|_| "feedback-busy")
    }
    pub fn recover(
        &self,
        flushed: Option<mpsc::Receiver<Result<serde_json::Value>>>,
    ) -> Result<()> {
        self.tx
            .try_send(Command::Recover(flushed))
            .map_err(|_| "feedback-busy")
    }
    pub fn stop(&mut self) {
        self.finish(Vec::new());
    }
    /// Shutdown cancels network first, then durably drains final summaries on the
    /// worker. A full UI command queue must not silently drop the last match.
    pub fn finish(&mut self, reports: Vec<Report>) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = self.tx.send(Command::Stop(reports));
            let _ = thread.join();
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
    }
}
fn worker(
    path: PathBuf,
    rx: mpsc::Receiver<Command>,
    view: Arc<Mutex<View>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;
    let _lock = match lock(&path) {
        Ok(lock) => lock,
        Err(_) => {
            *view.lock().unwrap() = View {
                status: "feedback-store",
                ..Default::default()
            };
            return;
        }
    };
    let mut store = match Store::load(&path) {
        Ok(store) => store,
        Err(_) => {
            *view.lock().unwrap() = View {
                ready: false,
                status: "feedback-store",
                ..Default::default()
            };
            return;
        }
    };
    // Keep uncertain prior uploads for reconciliation. An unfinished match is explicitly incomplete.
    for entry in &mut store.entries {
        if entry.report.end == End::Fault {
            entry.report.end = End::AppStopped;
            entry.sent = false;
        }
    }
    let mut next = Instant::now();
    let mut snoozed = std::collections::HashMap::<String, Instant>::new();
    let mut error = None;
    let mut authorization = "unchecked";
    let mut verify = store.enabled;
    let mut manual = false;
    let mut last_recovery: Option<Instant> = None;
    let mut uploaded_this_run = 0u64;
    loop {
        {
            let mut v = view.lock().unwrap();
            v.ready = true;
            v.uploaded_this_run = uploaded_this_run;
            v.authorization = authorization;
            v.enabled = store.enabled;
            v.pending = store.entries.iter().filter(|e| !e.sent).count();
            v.issue = store.latest_issue;
            v.status = error.unwrap_or(if !store.enabled {
                "disabled"
            } else if v.pending == 0 {
                "watching"
            } else {
                "queued"
            });
        }
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Command::Stop(reports)) => {
                if store.enabled {
                    for report in reports {
                        let _ = store.enqueue(report);
                    }
                }
                let _ = store.save(&path);
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = store.save(&path);
                return;
            }
            Ok(Command::Enable(enabled)) => {
                if !enabled {
                    manual = false;
                    authorization = "unchecked";
                }
                let old = store.enabled;
                store.enabled = enabled;
                if let Err(e) = store.save(&path) {
                    store.enabled = old;
                    error = Some(e);
                } else {
                    cancel.store(!enabled, Ordering::Release);
                    error = None;
                    next = Instant::now();
                    verify = enabled;
                }
            }
            Ok(Command::Report(report)) if store.enabled => {
                if let Err(e) = store.enqueue(report).and_then(|_| store.save(&path)) {
                    error = Some(e);
                } else {
                    next = Instant::now();
                }
            }
            Ok(Command::Retry) => {
                error = None;
                snoozed.clear();
                next = Instant::now();
                verify = true;
                manual = true;
                cancel.store(false, Ordering::Release);
            }
            Ok(Command::Verify) => {
                verify = true;
            }
            Ok(Command::Recover(flushed)) => {
                if last_recovery.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
                    error = Some("feedback-recovery-cooldown");
                } else {
                    last_recovery = Some(Instant::now());
                    let result = match flushed {
                        Some(rx) => rx
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(|_| "feedback-log-busy")
                            .and_then(|r| r),
                        None => {
                            crate::runtime_log::root().and_then(|p| crate::runtime_log::read(&p))
                        }
                    }
                    .and_then(recover_recent);
                    match result {
                        Ok(report) => {
                            error = store.enqueue(report).and_then(|_| store.save(&path)).err();
                            manual = error.is_none();
                            if manual {
                                cancel.store(false, Ordering::Release);
                                verify = true;
                                next = Instant::now();
                            }
                        }
                        Err(e) => error = Some(e),
                    }
                }
            }
            _ => (),
        }
        if verify {
            verify = false;
            view.lock().unwrap().authorization = "checking";
            let result = crate::update_credential::read_target(TARGET)
                .and_then(|s| io::verify(&s, &std::sync::atomic::AtomicBool::new(false)));
            authorization = match result {
                Ok(()) => "readable",
                Err(e) => e,
            };
        }
        if (!store.enabled && !manual) || cancel.load(Ordering::Acquire) || Instant::now() < next {
            continue;
        }
        let Some(index) = store.entries.iter().position(|e| {
            !e.sent
                && (store.enabled || e.report.recovered)
                && snoozed
                    .get(&e.report.id)
                    .is_none_or(|at| Instant::now() >= *at)
        }) else {
            continue;
        };
        let id = store.entries[index].report.id.clone();
        // All IO is on this independent worker. Persist before POST so a crash cannot duplicate it.
        view.lock().unwrap().status = "uploading";
        let result = (|| {
            let secret = crate::update_credential::read_target(TARGET)?;
            upload(
                &mut store,
                index,
                |store| store.save(&path),
                &mut Network {
                    secret: &secret,
                    cancel: &cancel,
                },
            )
        })();
        error = result.err();
        if error.is_none() {
            authorization = "write-confirmed";
            uploaded_this_run = uploaded_this_run.saturating_add(1);
        } else if matches!(
            error,
            Some(
                "credential-unavailable"
                    | "credential-format"
                    | "feedback-auth"
                    | "feedback-permission"
            )
        ) {
            authorization = error.unwrap();
        }
        if error.is_some() {
            snoozed.insert(id, Instant::now() + Duration::from_secs(300));
        } else {
            snoozed.remove(&id);
        }
        next = Instant::now() + Duration::from_secs(3);
    }
}
fn lock(path: &Path) -> Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    crate::managed_install::safe_dir(path.parent().ok_or("feedback-store")?)?;
    let target = path.with_extension("lock");
    regular(&target, 0)?;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .share_mode(0)
        .open(target)
        .map_err(|_| "feedback-store")
}
fn regular(path: &Path, limit: u64) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && m.file_attributes() & 0x400 == 0 && m.len() <= limit => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err("feedback-store"),
    }
}
