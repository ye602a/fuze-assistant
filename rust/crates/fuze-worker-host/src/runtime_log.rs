//! Typed, bounded local diagnostics. No free-form strings or raw payloads enter the queue.
use crate::{desktop::Status, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
pub const FILE_BYTES: u64 = 256 * 1024;
pub const NAMES: [&str; 3] = [
    "runtime-v1.jsonl",
    "runtime-v1.1.jsonl",
    "runtime-v1.2.jsonl",
];
const FLUSH: Duration = Duration::from_secs(2);
macro_rules! codes {
    ($name:ident {$($item:ident),+}) => {
        #[derive(Clone,Copy,Debug,PartialEq,Serialize,Deserialize)]
        #[serde(rename_all="kebab-case")]
        pub enum $name {$($item),+}
    }
}
codes!(Event {
    Start,
    State,
    Metrics,
    QueueOverflow,
    Stop
});
codes!(Connection {
    Waiting,
    Connected,
    Paused,
    Discovery,
    Credentials,
    Multiple,
    Auth,
    NotFound,
    Http,
    Phase,
    Limit,
    Transport
});
codes!(Phase {
    Unknown,
    Lobby,
    Selection,
    Loading,
    Playing,
    PostGame
});
codes!(Capture {
    Waiting,
    Paused,
    Fresh,
    WindowWaiting,
    Failed
});
codes!(Ocr {
    Waiting,
    Paused,
    Warming,
    Cpu,
    Directml,
    Failed
});
codes!(Issue {
    None,
    WindowWaiting,
    Capture,
    Window,
    Ocr,
    Pipeline
});
codes!(Failure {
    None,
    Other,
    LocaleUnavailable,
    ChampionMissing,
    LcuContextMissing,
    LcuObservation,
    PipelineMissing,
    WindowWaiting,
    WindowRoot,
    WindowLayout,
    WindowPathEscape,
    WindowAccess,
    WindowImageUnavailable,
    WindowPathMismatch,
    WindowFiltered,
    WindowAmbiguous,
    WindowDiscovery,
    CapturePhase,
    CaptureStart,
    CaptureMissing,
    CaptureFrame,
    CaptureAtlas,
    CaptureBitmap,
    WorkerResult,
    Worker,
    OverlayReset,
    OverlayDelivery
});
codes!(Stream {
    Waiting,
    WaitingFrame,
    WaitingSelection,
    Receiving,
    Resizing,
    GpuBusy,
    Suspended,
    Recovering,
    StatShards,
    Retaining,
    Unknown
});
codes!(CaptureSource {
    ScreenStream,
    ScreenBitmap,
    WindowStream
});
codes!(CaptureFallback {
    Start,
    Timeout,
    BlackFrame,
    Readback,
    Frame
});
// Preparing is a historical on-disk value; the current scheduler no longer emits it.
codes!(Gate {
    Unknown,
    Fallback,
    Selection,
    Pending,
    Preparing,
    Saving
});
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    #[serde(default)]
    pub process_cpu_percent: Option<f64>,
    #[serde(default)]
    pub process_memory_mb: Option<f64>,
    #[serde(default)]
    pub title_reads: [fuze_contract::wire::TitleRead; 3],
    pub connection: Connection,
    pub phase: Phase,
    pub capture: Capture,
    pub ocr: Ocr,
    pub issue: Issue,
    pub failure: Failure,
    pub analyses: u64,
    pub skipped: u64,
    pub analysis_ms: Option<f64>,
    pub level: Option<u8>,
    pub stream: Stream,
    #[serde(default)]
    pub capture_source: Option<CaptureSource>,
    #[serde(default)]
    pub capture_fallback: Option<CaptureFallback>,
    pub gate: Gate,
    pub interval_ms: Option<u64>,
    pub readable_slots: Option<[bool; 3]>,
    pub overlay_prepared: bool,
}
impl Snapshot {
    pub fn from_status(s: &Status) -> Self {
        use fuze_lcu::{reconnect::State, Error};
        use fuze_runtime::session::Phase as P;
        let connection = match s.connection {
            Some(State::Connected(_)) => Connection::Connected,
            Some(State::Paused) => Connection::Paused,
            Some(State::Retry(e)) => match e {
                Error::Discovery => Connection::Discovery,
                Error::AuthInput => Connection::Credentials,
                Error::AmbiguousClient => Connection::Multiple,
                Error::Unauthorized => Connection::Auth,
                Error::NotFound => Connection::NotFound,
                Error::HttpStatus => Connection::Http,
                Error::InvalidPhase => Connection::Phase,
                Error::ResponseLimit => Connection::Limit,
                Error::Transport => Connection::Transport,
            },
            _ => Connection::Waiting,
        };
        let phase = match s.connection {
            Some(State::Connected(p)) => match p {
                P::ClientReady => Phase::Lobby,
                P::ChampSelect => Phase::Selection,
                P::Loading => Phase::Loading,
                P::InProgress => Phase::Playing,
                P::PostGame => Phase::PostGame,
                _ => Phase::Unknown,
            },
            _ => Phase::Unknown,
        };
        let issue = match s.pipeline.error {
            None => Issue::None,
            Some("game-window-waiting") => Issue::WindowWaiting,
            Some(e) if e.starts_with("capture") => Issue::Capture,
            Some(e) if e.starts_with("game-window") => Issue::Window,
            Some(e) if e.starts_with("ocr") || e.starts_with("worker") => Issue::Ocr,
            Some(_) => Issue::Pipeline,
        };
        let failure = match s.pipeline.error {
            None => Failure::None,
            Some("locale-unavailable") => Failure::LocaleUnavailable,
            Some("champion-missing") => Failure::ChampionMissing,
            Some("lcu-context-missing") => Failure::LcuContextMissing,
            Some("lcu-observation") => Failure::LcuObservation,
            Some("pipeline-missing") => Failure::PipelineMissing,
            Some("game-window-waiting") => Failure::WindowWaiting,
            Some("game-window-root") => Failure::WindowRoot,
            Some("game-window-layout") => Failure::WindowLayout,
            Some("game-window-path-escape") => Failure::WindowPathEscape,
            Some("game-window-access") => Failure::WindowAccess,
            Some("game-window-image-unavailable") => Failure::WindowImageUnavailable,
            Some("game-window-path-mismatch") => Failure::WindowPathMismatch,
            Some("game-window-filtered") => Failure::WindowFiltered,
            Some("game-window-ambiguous") => Failure::WindowAmbiguous,
            Some("game-window-discovery") => Failure::WindowDiscovery,
            Some("capture-phase") => Failure::CapturePhase,
            Some("capture-start") => Failure::CaptureStart,
            Some("capture-missing") => Failure::CaptureMissing,
            Some("capture-frame") => Failure::CaptureFrame,
            Some("capture-atlas") => Failure::CaptureAtlas,
            Some("capture-bitmap") => Failure::CaptureBitmap,
            Some("worker-result") => Failure::WorkerResult,
            Some("overlay-reset") => Failure::OverlayReset,
            Some("overlay-delivery") => Failure::OverlayDelivery,
            Some(e) if e.starts_with("worker") => Failure::Worker,
            Some(_) => Failure::Other,
        };
        let capture = if s.mode != 1 {
            Capture::Paused
        } else {
            match issue {
                Issue::WindowWaiting => Capture::WindowWaiting,
                Issue::Capture | Issue::Window => Capture::Failed,
                _ if s.pipeline.frame_age.is_some_and(|age| age <= 5) => Capture::Fresh,
                _ => Capture::Waiting,
            }
        };
        let ocr = if s.mode != 1 {
            Ocr::Paused
        } else if matches!(issue, Issue::Ocr | Issue::Pipeline) {
            Ocr::Failed
        } else if let Some(backend) = s.pipeline.backend {
            if backend == fuze_contract::wire::ActiveBackend::Cpu {
                Ocr::Cpu
            } else {
                Ocr::Directml
            }
        } else if phase == Phase::Loading {
            Ocr::Warming
        } else {
            Ocr::Waiting
        };
        let stream = match s.pipeline.capture {
            "waiting" => Stream::Waiting,
            "waiting-frame" => Stream::WaitingFrame,
            "waiting-selection" => Stream::WaitingSelection,
            "receiving" => Stream::Receiving,
            "resizing" => Stream::Resizing,
            "gpu-busy" => Stream::GpuBusy,
            "suspended" => Stream::Suspended,
            "recovering" => Stream::Recovering,
            "stat-shards" => Stream::StatShards,
            "retaining" => Stream::Retaining,
            _ => Stream::Unknown,
        };
        let gate = s
            .pipeline
            .schedule
            .map(|(mode, _, _)| match mode {
                fuze_runtime::gate::Mode::Fallback => Gate::Fallback,
                fuze_runtime::gate::Mode::Selection => Gate::Selection,
                fuze_runtime::gate::Mode::Pending => Gate::Pending,
                fuze_runtime::gate::Mode::Saving => Gate::Saving,
            })
            .unwrap_or(Gate::Unknown);
        Self {
            process_cpu_percent: s
                .resources
                .cpu
                .filter(|n| n.is_finite() && (0. ..=100.).contains(n)),
            process_memory_mb: s
                .resources
                .memory_mb
                .filter(|n| n.is_finite() && (0. ..=1_048_576.).contains(n)),
            title_reads: s.pipeline.title_reads,
            connection,
            phase,
            capture,
            ocr,
            issue,
            failure,
            stream,
            capture_source: match s.pipeline.capture_source {
                "screen-stream" => Some(CaptureSource::ScreenStream),
                "screen-bitmap" => Some(CaptureSource::ScreenBitmap),
                "window-stream" => Some(CaptureSource::WindowStream),
                _ => None,
            },
            capture_fallback: match s.pipeline.capture_fallback {
                Some("screen-start") => Some(CaptureFallback::Start),
                Some("screen-timeout") => Some(CaptureFallback::Timeout),
                Some("screen-black-frame") => Some(CaptureFallback::BlackFrame),
                Some("screen-readback") => Some(CaptureFallback::Readback),
                Some("screen-frame") => Some(CaptureFallback::Frame),
                _ => None,
            },
            gate,
            interval_ms: s
                .pipeline
                .schedule
                .map(|(_, interval, _)| interval.min(3_600_000)),
            readable_slots: s.pipeline.recent.as_ref().map(|slots| {
                slots
                    .each_ref()
                    .map(|slot| matches!(slot, fuze_contract::Slot::Known { .. }))
            }),
            overlay_prepared: s.pipeline.overlay.is_some(),
            analyses: s.pipeline.analyses.min(1_000_000_000_000),
            skipped: s.pipeline.skipped_frames.min(1_000_000_000_000),
            analysis_ms: s
                .pipeline
                .analysis_ms
                .filter(|n| n.is_finite() && *n >= 0. && *n <= 3_600_000.),
            level: s.pipeline.level.filter(|n| (1..=30).contains(n)),
        }
    }
    pub(crate) fn same_state(&self, other: &Self) -> bool {
        (
            self.connection,
            self.phase,
            self.capture,
            self.ocr,
            self.issue,
            self.failure,
            self.capture_source,
            self.capture_fallback,
        ) == (
            other.connection,
            other.phase,
            other.capture,
            other.ocr,
            other.issue,
            other.failure,
            other.capture_source,
            other.capture_fallback,
        ) && (
            self.stream,
            self.gate,
            self.readable_slots,
            self.overlay_prepared,
            self.title_reads,
        ) == (
            other.stream,
            other.gate,
            other.readable_slots,
            other.overlay_prepared,
            other.title_reads,
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: u8,
    #[serde(
        default,
        rename = "runtimeRevision",
        skip_serializing_if = "Option::is_none"
    )]
    pub runtime_revision: Option<crate::build_identity::Revision>,
    pub event: Event,
    pub elapsed_ms: u64,
    pub state: Snapshot,
    pub dropped: u64,
}
impl Record {
    pub(crate) fn valid(&self) -> bool {
        self.schema == 1
            && self
                .state
                .process_cpu_percent
                .is_none_or(|n| n.is_finite() && (0. ..=100.).contains(&n))
            && self
                .state
                .process_memory_mb
                .is_none_or(|n| n.is_finite() && (0. ..=1_048_576.).contains(&n))
            && self.elapsed_ms <= 1_000_000_000_000
            && self.dropped <= 1_000_000_000_000
            && self.state.analyses <= 1_000_000_000_000
            && self.state.skipped <= 1_000_000_000_000
            && self.state.level.is_none_or(|n| (1..=30).contains(&n))
            && self
                .state
                .analysis_ms
                .is_none_or(|n| n.is_finite() && (0. ..=3_600_000.).contains(&n))
            && self.state.interval_ms.is_none_or(|n| n <= 3_600_000)
    }
}
pub fn root() -> Result<PathBuf> {
    Ok(crate::preferences::path()?.with_file_name("logs"))
}
pub(crate) fn directory(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.to_string_lossy().starts_with("\\\\") {
        return Err("log-path");
    }
    for p in path.ancestors() {
        if let Ok(m) = fs::symlink_metadata(p) {
            if m.file_attributes() & 0x400 != 0 || !m.is_dir() {
                return Err("log-path");
            }
        }
    }
    Ok(())
}
fn regular(path: &Path, max: u64) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() || m.file_attributes() & 0x400 != 0 || m.len() > max => {
            Err("log-file")
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("log-file"),
    }
}
struct Store {
    dir: PathBuf,
    file: Option<BufWriter<File>>,
    size: u64,
    limit: u64,
    _lock: File,
}
impl Store {
    fn open(dir: &Path, limit: u64) -> Result<Self> {
        directory(dir)?;
        fs::create_dir_all(dir).map_err(|_| "log-path")?;
        regular(&dir.join("runtime-v1.lock"), 0)?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(dir.join("runtime-v1.lock"))
            .map_err(|_| "log-busy")?;
        for name in NAMES {
            regular(&dir.join(name), FILE_BYTES)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .share_mode(1)
            .open(dir.join(NAMES[0]))
            .map_err(|_| "log-write")?;
        let size = file.metadata().map_err(|_| "log-write")?.len();
        let mut store = Self {
            dir: dir.into(),
            file: Some(BufWriter::with_capacity(8192, file)),
            size,
            limit,
            _lock: lock,
        };
        // A previous abrupt exit can leave a partial last line; never join it to the next record.
        if size > 0 {
            store.write_bytes(b"\n")?;
        }
        Ok(store)
    }
    fn flush(&mut self, durable: bool) -> Result<()> {
        if let Some(file) = &mut self.file {
            file.flush().map_err(|_| "log-write")?;
            if durable {
                file.get_ref().sync_data().map_err(|_| "log-write")?;
            }
        }
        Ok(())
    }
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        if self.size + bytes.len() as u64 > self.limit {
            self.flush(true)?;
            self.file.take();
            directory(&self.dir)?;
            for name in NAMES {
                regular(&self.dir.join(name), FILE_BYTES)?;
            }
            if self.dir.join(NAMES[2]).exists() {
                fs::remove_file(self.dir.join(NAMES[2])).map_err(|_| "log-rotate")?;
            }
            for i in (0..2).rev() {
                if self.dir.join(NAMES[i]).exists() {
                    fs::rename(self.dir.join(NAMES[i]), self.dir.join(NAMES[i + 1]))
                        .map_err(|_| "log-rotate")?;
                }
            }
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(1)
                .open(self.dir.join(NAMES[0]))
                .map_err(|_| "log-write")?;
            self.file = Some(BufWriter::with_capacity(8192, file));
            self.size = 0;
        }
        self.file
            .as_mut()
            .ok_or("log-write")?
            .write_all(bytes)
            .map_err(|_| "log-write")?;
        self.size += bytes.len() as u64;
        Ok(())
    }
    fn append(&mut self, r: &Record) -> Result<()> {
        let mut bytes = serde_json::to_vec(r).map_err(|_| "log-encode")?;
        if bytes.len() > 2048 {
            return Err("log-limit");
        }
        bytes.push(b'\n');
        self.write_bytes(&bytes)
    }
}
/// Re-parse typed entries on export; corrupted/tampered fields never escape as raw text.
pub fn read(dir: &Path) -> Result<serde_json::Value> {
    directory(dir)?;
    let mut rows = Vec::new();
    let mut skipped = 0u64;
    let mut unavailable = 0u8;
    let mut found = false;
    for name in NAMES.iter().rev() {
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        found = true;
        let result = (|| {
            regular(&path, FILE_BYTES)?;
            let file = File::open(&path).map_err(|_| "log-read")?;
            let mut bytes = Vec::new();
            file.take(FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "log-read")?;
            if bytes.len() as u64 > FILE_BYTES {
                return Err("log-limit");
            }
            for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
                if line.len() > 2048 {
                    skipped += 1;
                    continue;
                }
                match serde_json::from_slice::<Record>(line) {
                    Ok(row) if row.valid() => rows.push(row),
                    _ => skipped += 1,
                }
            }
            Ok(())
        })();
        if result.is_err() {
            unavailable += 1;
        }
    }
    Ok(
        serde_json::json!({"schema":1,"status":if unavailable>0 || skipped>0 {"partial"} else if found {"ok"} else {"missing"},
        "records":rows,"skippedLines":skipped,"unavailableFiles":unavailable,
        "scope":"last-three-rotating-files-not-match-count","actualClaimRound":null}),
    )
}
enum Command {
    Record(Record),
    Export(mpsc::Sender<Result<serde_json::Value>>),
}
pub struct Journal {
    revision: Option<crate::build_identity::Revision>,
    tx: Option<SyncSender<Command>>,
    worker: Option<JoinHandle<()>>,
    status: Arc<AtomicU8>,
    dropped: Arc<AtomicU64>,
    started: Instant,
    previous: Option<Snapshot>,
    sample: Instant,
    latest: Option<Snapshot>,
}
impl Journal {
    pub fn start(dir: PathBuf) -> Self {
        Self::start_with_revision(dir, crate::build_identity::current())
    }
    fn start_with_revision(
        dir: PathBuf,
        revision: Option<crate::build_identity::Revision>,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel(64);
        let status = Arc::new(AtomicU8::new(0));
        let state = status.clone();
        let dropped = Arc::new(AtomicU64::new(0));
        let loss = dropped.clone();
        let started = Instant::now();
        let worker_revision = revision.clone();
        let worker = std::thread::Builder::new()
            .name("fuze-runtime-log".into())
            .spawn(move || {
                let result = write_loop(&dir, rx, &state, &loss, started, worker_revision);
                if let Err(e) = result {
                    state.store(if e == "log-busy" { 3 } else { 2 }, Ordering::Release);
                }
            })
            .ok();
        if worker.is_none() {
            status.store(2, Ordering::Release);
        }
        Self {
            revision,
            tx: Some(tx),
            worker,
            status,
            dropped,
            started,
            previous: None,
            sample: started,
            latest: None,
        }
    }
    /// 0 preparing, 1 writing, 2 failed, 3 another writer owns the files.
    pub fn status(&self) -> u8 {
        self.status.load(Ordering::Acquire)
    }
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    pub fn observe(&mut self, s: &Status) {
        if self.status() >= 2 {
            return;
        }
        let next = Snapshot::from_status(s);
        self.latest = Some(next.clone());
        let changed = self
            .previous
            .as_ref()
            .is_none_or(|old| !old.same_state(&next));
        let metrics = self.previous.as_ref() != Some(&next)
            && self.sample.elapsed() >= Duration::from_secs(10);
        if !changed && !metrics {
            return;
        }
        let record = Record {
            schema: 1,
            runtime_revision: self.revision.clone(),
            event: if changed {
                Event::State
            } else {
                Event::Metrics
            },
            elapsed_ms: self.started.elapsed().as_millis().min(1_000_000_000_000) as u64,
            state: next.clone(),
            dropped: 0,
        };
        if self
            .tx
            .as_ref()
            .is_some_and(|tx| tx.try_send(Command::Record(record)).is_ok())
        {
            self.previous = Some(next);
            self.sample = Instant::now();
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn export(&self) -> Result<Receiver<Result<serde_json::Value>>> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .as_ref()
            .ok_or("log-unavailable")?
            .try_send(Command::Export(tx))
            .map_err(|_| "log-busy")?;
        Ok(rx)
    }
    pub fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            if let Some(state) = self.latest.take() {
                let _ = tx.send(Command::Record(Record {
                    schema: 1,
                    runtime_revision: self.revision.clone(),
                    event: Event::Metrics,
                    elapsed_ms: self.started.elapsed().as_millis() as u64,
                    state,
                    dropped: 0,
                }));
            }
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Journal {
    fn drop(&mut self) {
        self.stop();
    }
}
fn write_loop(
    dir: &Path,
    rx: Receiver<Command>,
    status: &AtomicU8,
    dropped: &AtomicU64,
    started: Instant,
    revision: Option<crate::build_identity::Revision>,
) -> Result<()> {
    let mut store = Store::open(dir, FILE_BYTES)?;
    let mut last = Record {
        schema: 1,
        runtime_revision: revision,
        event: Event::Start,
        elapsed_ms: 0,
        state: Snapshot::from_status(&Status::default()),
        dropped: 0,
    };
    store.append(&last)?;
    store.flush(false)?;
    status.store(1, Ordering::Release);
    let mut flushed = Instant::now();
    let mut dirty = false;
    let mut lost = 0;
    loop {
        match rx.recv_timeout(FLUSH.saturating_sub(flushed.elapsed())) {
            Ok(Command::Record(record)) => {
                store.append(&record)?;
                last = record;
                dirty = true;
            }
            Ok(Command::Export(reply)) => {
                store.flush(false)?;
                dirty = false;
                flushed = Instant::now();
                let _ = reply.send(read(dir));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                last.event = Event::Stop;
                last.elapsed_ms = started.elapsed().as_millis() as u64;
                last.dropped = dropped.load(Ordering::Relaxed).saturating_sub(lost);
                store.append(&last)?;
                return store.flush(true);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if flushed.elapsed() >= FLUSH {
            let total = dropped.load(Ordering::Relaxed);
            if total > lost {
                let mut gap = last.clone();
                gap.event = Event::QueueOverflow;
                gap.dropped = total - lost;
                gap.elapsed_ms = started.elapsed().as_millis() as u64;
                store.append(&gap)?;
                lost = total;
                dirty = true;
            }
            if dirty {
                store.flush(false)?;
                dirty = false;
            }
            flushed = Instant::now();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "fuze-typed-log-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            assert!(self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("fuze-typed-log-"));
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn record() -> Record {
        let mut status = Status::default();
        status.mode = 1;
        status.available = Some("private-do-not-export".into());
        status.pipeline.error = Some("worker-private-do-not-export");
        status.pipeline.dataset = Some("private-do-not-export".into());
        Record {
            schema: 1,
            runtime_revision: None,
            event: Event::State,
            elapsed_ms: 1,
            state: Snapshot::from_status(&status),
            dropped: 0,
        }
    }
    #[test]
    fn discovery_failures_export_distinct_fixed_codes_without_mislabeling_ocr() {
        let mut status = Status::default();
        status.mode = 1;
        status.pipeline.backend = Some(fuze_contract::wire::ActiveBackend::DirectMl);
        for (error, code) in [
            ("game-window-root", "window-root"),
            ("game-window-layout", "window-layout"),
            ("game-window-path-escape", "window-path-escape"),
            ("game-window-access", "window-access"),
            ("game-window-image-unavailable", "window-image-unavailable"),
            ("game-window-path-mismatch", "window-path-mismatch"),
            ("game-window-filtered", "window-filtered"),
            ("game-window-ambiguous", "window-ambiguous"),
            ("game-window-discovery", "window-discovery"),
        ] {
            status.pipeline.error = Some(error);
            let snapshot = Snapshot::from_status(&status);
            assert_eq!(snapshot.capture, Capture::Failed);
            assert_eq!(snapshot.issue, Issue::Window);
            assert_eq!(snapshot.ocr, Ocr::Directml);
            let value = serde_json::to_value(&snapshot).unwrap();
            assert_eq!(value["failure"], code);
            assert_eq!(serde_json::from_value::<Snapshot>(value).unwrap(), snapshot);
        }
    }
    #[test]
    fn rotation_reopen_and_corruption_remain_bounded_and_private() {
        let fixture = Fixture::new();
        let sentinel = fixture.0.join("unrelated-user-file.txt");
        fs::write(&sentinel, "keep").unwrap();
        let mut store = Store::open(&fixture.0, 4096).unwrap();
        let mut row = record();
        row.runtime_revision = Some("111111111111".to_owned().try_into().unwrap());
        for i in 0..80 {
            row.elapsed_ms = i;
            store.append(&row).unwrap();
        }
        store.flush(true).unwrap();
        drop(store);
        for name in NAMES {
            assert!(fs::metadata(fixture.0.join(name)).unwrap().len() <= 4096);
        }
        let before = read(&fixture.0).unwrap();
        assert!(before["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["runtimeRevision"] == "111111111111"));
        assert!(before["records"].as_array().unwrap().len() < 80);
        assert!(!before.to_string().contains("private-do-not-export"));
        let mut store = Store::open(&fixture.0, 4096).unwrap();
        row.elapsed_ms = 999;
        store.append(&row).unwrap();
        store.flush(true).unwrap();
        drop(store);
        let mut file = OpenOptions::new()
            .append(true)
            .open(fixture.0.join(NAMES[0]))
            .unwrap();
        file.write_all(b"{\"private-do-not-export\":\"secret\"}\n{\"partial\":")
            .unwrap();
        drop(file);
        let after = read(&fixture.0).unwrap();
        assert_eq!(after["status"], "partial");
        assert_eq!(after["skippedLines"], 2);
        assert_eq!(
            after["records"].as_array().unwrap().last().unwrap()["elapsed_ms"],
            999
        );
        assert!(!after.to_string().contains("secret"));
        // Starting after a truncated final line must preserve older entries and produce valid new ones.
        let mut store = Store::open(&fixture.0, 4096).unwrap();
        row.elapsed_ms = 1000;
        store.append(&row).unwrap();
        store.flush(true).unwrap();
        drop(store);
        assert_eq!(
            read(&fixture.0).unwrap()["records"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["elapsed_ms"],
            1000
        );
        assert_eq!(fs::read_to_string(sentinel).unwrap(), "keep");
    }
    #[test]
    fn background_writer_deduplicates_flushes_exports_and_preserves_across_runs() {
        let fixture = Fixture::new();
        let mut journal = Journal::start(fixture.0.clone());
        let mut status = Status::default();
        journal.observe(&status);
        for _ in 0..100 {
            journal.observe(&status);
        }
        status.pipeline.analyses = 5;
        status.pipeline.analysis_ms = Some(275.);
        journal.observe(&status); // Metrics wait for the interval; they are not per-frame writes.
        let report = journal
            .export()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(report["records"].as_array().unwrap().len(), 2); // start + one state
        journal.sample = Instant::now() - Duration::from_secs(11);
        journal.observe(&status);
        let report = journal
            .export()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(
            report["records"].as_array().unwrap().last().unwrap()["event"],
            "metrics"
        );
        status.pipeline.analyses = 6;
        status.pipeline.analysis_ms = Some(300.);
        journal.observe(&status);
        journal.stop();
        let report = read(&fixture.0).unwrap();
        let last = report["records"].as_array().unwrap().last().unwrap();
        assert_eq!(last["event"], "stop");
        assert_eq!(last["state"]["analysis_ms"], 300.);
        let before = report["records"].as_array().unwrap().len();
        let mut next = Journal::start(fixture.0.clone());
        next.stop();
        assert_eq!(
            read(&fixture.0).unwrap()["records"]
                .as_array()
                .unwrap()
                .len(),
            before + 2
        );
    }
    #[test]
    fn journal_keeps_each_build_and_never_relabels_legacy_records() {
        assert_eq!(
            serde_json::from_str::<Gate>(r#""preparing""#).unwrap(),
            Gate::Preparing
        );
        let fixture = Fixture::new();
        let mut store = Store::open(&fixture.0, FILE_BYTES).unwrap();
        store.append(&record()).unwrap();
        store.flush(true).unwrap();
        drop(store);
        let mut journal = Journal::start_with_revision(
            fixture.0.clone(),
            Some("111111111111".to_owned().try_into().unwrap()),
        );
        journal.observe(&Status::default());
        journal.stop();
        let mut next = Journal::start_with_revision(
            fixture.0.clone(),
            Some("222222222222".to_owned().try_into().unwrap()),
        );
        next.observe(&Status::default());
        next.stop();
        let report = read(&fixture.0).unwrap();
        let rows = report["records"].as_array().unwrap();
        assert!(rows[0].get("runtimeRevision").is_none());
        for (offset, revision) in [(1, "111111111111"), (5, "222222222222")] {
            assert_eq!(rows[offset]["event"], "start");
            assert_eq!(rows[offset + 3]["event"], "stop");
            assert!(rows[offset..offset + 4]
                .iter()
                .all(|r| r["runtimeRevision"] == revision));
        }
        let mut invalid = serde_json::to_value(record()).unwrap();
        invalid["runtimeRevision"] = serde_json::json!("private-do-not-export");
        assert!(serde_json::from_value::<Record>(invalid).is_err());
    }
    #[test]
    fn lock_and_write_failure_do_not_touch_other_files_or_pretend_success() {
        let fixture = Fixture::new();
        let store = Store::open(&fixture.0, FILE_BYTES).unwrap();
        let mut other = Journal::start(fixture.0.clone());
        other.stop();
        assert_eq!(other.status(), 3);
        drop(store);
        let blocked = fixture.0.join("file-not-directory");
        fs::write(&blocked, "keep").unwrap();
        let mut failed = Journal::start(blocked.clone());
        failed.stop();
        assert_eq!(failed.status(), 2);
        assert_eq!(fs::read_to_string(blocked).unwrap(), "keep");
    }
}
