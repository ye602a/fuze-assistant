//! Local-only recognition evidence. Normal sessions enable it; offline sessions never do.
use crate::Result;
use fuze_capture::{native::CaptureFeed, target::GameWindow};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Cursor, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 120;
pub fn root() -> Result<PathBuf> {
    Ok(crate::preferences::path()?.with_file_name("recognition-captures"))
}
fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Default, Clone)]
pub struct Control(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    enabled: bool,
    sender: Option<SyncSender<Job>>,
    worker: Option<std::thread::JoinHandle<()>>,
    latest: Option<Arc<Sample>>,
    signature: Option<(String, Instant)>,
    saved: usize,
    dropped: usize,
    error: Option<&'static str>,
}
struct Job {
    sample: Arc<Sample>,
    marked: bool,
}
pub struct Sample {
    at: Instant,
    pub(crate) metadata: Value,
    pixels: Option<image::RgbImage>,
}
impl Sample {
    /// Copies only the exact atlases already read by this feed. Never captures another frame.
    pub fn frame(capture: &dyn CaptureFeed, mut metadata: Value) -> Self {
        let pixels = (|| {
            let titles = capture.titles().ok()?;
            let controls = capture.diagnostic_pixels()?;
            let buttons = controls.buttons;
            let width = (titles.title_width.max(buttons.title_width)) * 3;
            let height = titles.title_height + buttons.title_height;
            let mut image = image::RgbImage::new(width, height);
            for (atlas, top) in [(&titles, 0), (&buttons, titles.title_height)] {
                for (index, bgra) in atlas.bytes().chunks_exact(4).enumerate() {
                    let x = index as u32 % (atlas.title_width * 3);
                    let y = index as u32 / (atlas.title_width * 3);
                    image.put_pixel(x, top + y, image::Rgb([bgra[2], bgra[1], bgra[0]]));
                }
            }
            metadata["imageLayout"] = json!({
                "screen": [controls.screen.0, controls.screen.1],
                "titles": {"slotWidth": titles.title_width, "height": titles.title_height, "top": 0},
                "buttons": {"slotWidth": buttons.title_width, "height": buttons.title_height, "top": titles.title_height},
                "titleRegions": regions(fuze_contract::titles::regions(controls.screen.0, controls.screen.1).ok()?),
                "buttonRegions": regions(fuze_contract::selection_pixels::buttons(controls.screen.0, controls.screen.1).ok()?),
            });
            Some(image)
        })();
        Self::new(metadata, pixels)
    }
    pub fn without_image(mut metadata: Value) -> Self {
        metadata["frame"] = Value::Null;
        metadata["evidence"] = Value::Null;
        Self::new(metadata, None)
    }
    fn new(mut metadata: Value, pixels: Option<image::RgbImage>) -> Self {
        metadata["schema"] = json!(1);
        metadata["capturedAtUnixMs"] = json!(timestamp());
        metadata["appVersion"] = json!(crate::build_identity::APP_VERSION.trim());
        metadata["runtimeRevision"] = json!(crate::build_identity::current());
        metadata["hasImage"] = json!(pixels.is_some());
        Self {
            at: Instant::now(),
            metadata,
            pixels,
        }
    }
}
fn regions(regions: [fuze_contract::titles::Region; 3]) -> Value {
    json!(regions.map(|r| [r.x, r.y, r.width, r.height]))
}
impl Control {
    pub(crate) fn start_session(&self, offline: bool, directory: impl FnOnce() -> Result<PathBuf>) {
        if !offline {
            if let Err(error) = directory().and_then(|path| self.enable_at(path)) {
                if let Ok(mut s) = self.0.lock() {
                    s.error = Some(error);
                }
            }
        }
    }
    pub fn enabled(&self) -> bool {
        self.0.lock().is_ok_and(|s| s.enabled)
    }
    pub fn enable(&self, enabled: bool) -> Result<()> {
        if enabled {
            self.enable_at(root()?)
        } else {
            let mut s = self.0.lock().map_err(|_| "capture-log-lock")?;
            s.enabled = false;
            s.latest = None;
            s.signature = None;
            Ok(())
        }
    }
    fn enable_at(&self, directory: PathBuf) -> Result<()> {
        let mut s = self.0.lock().map_err(|_| "capture-log-lock")?;
        if s.sender.is_none() {
            let (tx, rx) = mpsc::sync_channel::<Job>(2);
            let status = Arc::downgrade(&self.0);
            let worker = std::thread::Builder::new()
                .name("fuze-local-captures".into())
                .spawn(move || {
                    let mut writer = Writer::open(directory);
                    if let Err(error) = &writer {
                        if let Some(status) = status.upgrade() {
                            if let Ok(mut s) = status.lock() {
                                s.error = Some(error);
                                s.enabled = false;
                            }
                        }
                    }
                    while let Ok(job) = rx.recv() {
                        let result = match writer.as_mut() {
                            Ok(writer) => writer.save(&job),
                            Err(error) => Err(*error),
                        };
                        if let Some(status) = status.upgrade() {
                            if let Ok(mut s) = status.lock() {
                                match result {
                                    Ok(()) => s.saved += 1,
                                    Err(e) => s.error = Some(e),
                                }
                                if result.is_err() {
                                    s.enabled = false;
                                    s.latest = None;
                                }
                            }
                        }
                    }
                })
                .map_err(|_| "capture-log-thread")?;
            s.sender = Some(tx);
            s.worker = Some(worker);
        }
        s.enabled = true;
        s.error = None;
        Ok(())
    }
    pub fn record(&self, sample: Sample) {
        let Ok(mut s) = self.0.lock() else {
            return;
        };
        if !s.enabled {
            return;
        }
        let signature = json!([
            sample.metadata["stage"],
            sample.metadata["level"],
            sample.metadata["evidence"],
            sample.metadata["ocr"]["titleKind"],
            sample.metadata["ocr"]["accepted"],
            sample.metadata["displayVisible"],
            sample.metadata["retained"],
            sample.metadata["error"],
            sample.metadata["source"],
            sample.metadata["fallback"],
            sample.metadata["ocr"]["workerCause"],
            sample.metadata["ocr"]["slots"]
                .as_array()
                .map(|slots| slots.iter().map(|s| s.get("id")).collect::<Vec<_>>()),
        ])
        .to_string();
        let failed = sample.metadata["error"].is_string();
        let sample = Arc::new(sample);
        // A capture failure replaces, rather than inherits, the previous image.
        s.latest = Some(sample.clone());
        let changed = s.signature.as_ref().is_none_or(|(old, at)| {
            (old != &signature && (failed || at.elapsed() >= Duration::from_secs(2)))
                || at.elapsed() >= Duration::from_secs(30)
        });
        if !changed {
            return;
        }
        if s.sender.as_ref().is_some_and(|tx| {
            tx.try_send(Job {
                sample,
                marked: false,
            })
            .is_ok()
        }) {
            s.signature = Some((signature, Instant::now()));
        } else {
            s.dropped += 1;
        }
    }
    pub fn mark_recent(&self) -> Result<()> {
        let s = self.0.lock().map_err(|_| "capture-log-lock")?;
        let sample = s
            .latest
            .as_ref()
            .filter(|r| s.enabled && r.at.elapsed() <= Duration::from_secs(30))
            .ok_or("capture-log-no-recent")?
            .clone();
        s.sender
            .as_ref()
            .ok_or("capture-log-disabled")?
            .try_send(Job {
                sample,
                marked: true,
            })
            .map_err(|_| "capture-log-busy")
    }
    pub fn status(&self) -> (bool, bool, String) {
        let Ok(s) = self.0.lock() else {
            return (false, false, "截图日志暂不可用".into());
        };
        let hint = match s.error {
            Some("capture-log-limit") => {
                "已到 120 条或 64 MiB 上限，请先移走目录内的旧记录，再重启后开启。"
            }
            Some(_) => "截图日志写入失败，请检查目录是否可写或被另一实例占用。",
            None if s.enabled => "已开启，等待前台主屏游戏画面；关闭或退出后停止采样。",
            None => "本次运行已关闭，已有文件保留；下次启动默认开启。",
        };
        (
            s.enabled,
            s.enabled
                && s.latest
                    .as_ref()
                    .is_some_and(|r| r.at.elapsed() <= Duration::from_secs(30)),
            format!(
                "{hint} 已保存 {} 条，队列繁忙跳过 {} 条。",
                s.saved, s.dropped
            ),
        )
    }
    /// Flush accepted work after the engine stops, without blocking the render loop per frame.
    pub fn finish(&self) {
        let worker = if let Ok(mut s) = self.0.lock() {
            s.enabled = false;
            s.latest = None;
            s.sender.take();
            s.worker.take()
        } else {
            None
        };
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

/// Foreground checks are metadata-only and occur only while local diagnostics are enabled.
#[derive(Default)]
pub struct Probe {
    pub control: Control,
    bitmap: Option<fuze_capture::screen::ScreenCapture>,
    root: Option<PathBuf>,
    target: Option<GameWindow>,
    next_discovery: Option<Instant>,
    next_sample: Option<Instant>,
}
impl Probe {
    pub fn configure(&mut self, control: Control, root: PathBuf) {
        self.control = control;
        self.root = Some(root);
    }
    pub fn due(&self) -> bool {
        self.control.enabled() && self.next_sample.is_none_or(|at| Instant::now() >= at)
    }
    pub fn begin(&mut self) -> bool {
        if !self.control.enabled() {
            self.bitmap = None;
            self.target = None;
            self.next_discovery = None;
            self.next_sample = None;
            return false;
        }
        if !self.due() {
            return false;
        }
        self.next_sample = Some(Instant::now() + Duration::from_secs(1));
        if self.target.as_ref().is_some_and(|w| !w.valid()) {
            self.target = None;
        }
        if self.target.is_none() && self.next_discovery.is_none_or(|at| Instant::now() >= at) {
            self.next_discovery = Some(Instant::now() + Duration::from_secs(5));
            self.target = self
                .root
                .as_ref()
                .and_then(|r| fuze_capture::target::discover(r).ok().flatten());
        }
        self.visible()
    }
    pub fn visible(&self) -> bool {
        self.control.enabled()
            && self
                .target
                .as_ref()
                .is_some_and(GameWindow::covers_primary_selection)
    }
    /// A gated diagnostic must not consume a DXGI frame that the OCR loop still needs.
    pub fn gated(&mut self, mut metadata: Value) {
        let capture = self.bitmap.get_or_insert_with(Default::default);
        capture.saving(true); // existing primary-screen GDI ROI reader, no continuous stream
        let result = capture.next(Duration::ZERO);
        metadata["source"] = json!("diagnostic-bitmap");
        metadata["frame"] = json!([capture.key().0, capture.key().1]);
        metadata["fallback"] = Value::Null;
        metadata["evidence"] = json!(capture.selection().map(|e| format!("{e:?}")));
        let sample = if matches!(result, Ok(fuze_capture::native::FrameStatus::Ready)) {
            Sample::frame(capture, metadata)
        } else {
            metadata["stage"] = json!("diagnostic-no-frame");
            Sample::without_image(metadata)
        };
        if self.visible() {
            self.control.record(sample);
        }
    }
}

struct Writer {
    directory: PathBuf,
    _lock: File,
    bytes: u64,
    count: usize,
    sequence: u64,
    session: u128,
}
impl Writer {
    fn open(directory: PathBuf) -> Result<Self> {
        crate::runtime_log::directory(&directory)?;
        fs::create_dir_all(&directory).map_err(|_| "capture-log-directory")?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(directory.join("captures-v1.lock"))
            .map_err(|_| "capture-log-busy")?;
        let (mut bytes, mut count) = (0, 0);
        for entry in fs::read_dir(&directory).map_err(|_| "capture-log-read")? {
            let entry = entry.map_err(|_| "capture-log-read")?;
            let meta = entry.metadata().map_err(|_| "capture-log-read")?;
            if meta.is_file() {
                bytes += meta.len();
                count += usize::from(entry.path().extension().is_some_and(|x| x == "json"));
            }
        }
        Ok(Self {
            directory,
            _lock: lock,
            bytes,
            count,
            sequence: 0,
            session: timestamp(),
        })
    }
    fn save(&mut self, job: &Job) -> Result<()> {
        let mut png = Cursor::new(Vec::new());
        if let Some(pixels) = &job.sample.pixels {
            pixels
                .write_to(&mut png, image::ImageFormat::Png)
                .map_err(|_| "capture-log-png")?;
        }
        let mut metadata = job.sample.metadata.clone();
        metadata["markedByPlayer"] = json!(job.marked);
        self.sequence += 1;
        let name = format!(
            "capture-v1-{}-{}-{}",
            self.session,
            std::process::id(),
            self.sequence
        );
        metadata["imageFile"] = if png.get_ref().is_empty() {
            Value::Null
        } else {
            json!(format!("{name}.png"))
        };
        let json = serde_json::to_vec_pretty(&metadata).map_err(|_| "capture-log-json")?;
        let size = (json.len() + png.get_ref().len()) as u64;
        if self.count >= MAX_RECORDS || self.bytes + size > MAX_BYTES {
            return Err("capture-log-limit");
        }
        // create_new protects prior sessions and any manually placed files. JSON is the completion marker.
        if !png.get_ref().is_empty() {
            write_new(&self.directory.join(format!("{name}.png")), png.get_ref())?;
        }
        write_new(&self.directory.join(format!("{name}.json")), &json)?;
        self.bytes += size;
        self.count += 1;
        Ok(())
    }
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|_| "capture-log-write")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuze_capture::native::{DiagnosticPixels, FrameStatus};
    use fuze_contract::titles::Atlas;
    struct Feed {
        titles: Vec<u8>,
        buttons: Vec<u8>,
    }
    impl Feed {
        fn new() -> Self {
            Self {
                titles: [11, 22, 33, 255].repeat(12),
                buttons: [44, 55, 66, 0].repeat(3),
            }
        }
    }
    impl CaptureFeed for Feed {
        fn next(&mut self, _: Duration) -> fuze_capture::native::Result<FrameStatus> {
            panic!("diagnostics must not recapture")
        }
        fn titles(&self) -> fuze_capture::native::Result<Atlas<'_>> {
            Ok(Atlas::new(&self.titles, 2, 2)?)
        }
        fn diagnostic_pixels(&self) -> Option<DiagnosticPixels<'_>> {
            Some(DiagnosticPixels {
                buttons: Atlas::new(&self.buttons, 1, 1).unwrap(),
                screen: (1920, 1080),
            })
        }
        fn key(&self) -> (u64, u64) {
            (4, 12)
        }
        fn source(&self) -> &'static str {
            "fixture"
        }
    }
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            Self(std::env::temp_dir().join(format!(
                "fuze-capture-test-{}-{}-{}",
                timestamp(),
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn frame(feed: &Feed) -> Sample {
        Sample::frame(
            feed,
            json!({"frame":[4,12], "stage":"ocr", "level":7, "ocr":{"accepted":false,"slots":[{"id":1},null,null]}}),
        )
    }
    fn queued() -> (Control, mpsc::Receiver<Job>) {
        let (tx, rx) = mpsc::sync_channel(2);
        let control = Control::default();
        {
            let mut s = control.0.lock().unwrap();
            s.enabled = true;
            s.sender = Some(tx);
        }
        (control, rx)
    }
    #[test]
    fn normal_session_starts_local_writer_but_offline_never_resolves_personal_directory() {
        let offline = Control::default();
        offline.start_session(true, || panic!("offline must not touch personal paths"));
        assert!(!offline.enabled());
        let directory = Directory::new();
        let control = Control::default();
        control.start_session(false, || Ok(directory.0.clone()));
        assert!(control.enabled());
        control.record(frame(&Feed::new()));
        control.finish();
        assert!(fs::read_dir(&directory.0).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|x| x == "json")));
    }
    #[test]
    fn new_failure_is_saved_even_immediately_after_success() {
        let (control, rx) = queued();
        control.record(frame(&Feed::new()));
        rx.recv().unwrap();
        control.record(Sample::without_image(
            json!({"stage":"ocr","error":"worker-timeout"}),
        ));
        assert_eq!(
            rx.try_recv().unwrap().sample.metadata["error"],
            "worker-timeout"
        );
        control.record(Sample::without_image(
            json!({"stage":"ocr","error":"worker-timeout"}),
        ));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn same_frame_pixels_survive_png_and_manual_marker_keeps_capture_identity() {
        let directory = Directory::new();
        let mut writer = Writer::open(directory.0.clone()).unwrap();
        let mut feed = Feed::new();
        let sample = Arc::new(frame(&feed));
        feed.titles.fill(0); // the original feed has already advanced
        writer
            .save(&Job {
                sample: sample.clone(),
                marked: true,
            })
            .unwrap();
        let entry = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|x| x == "json"))
            .unwrap();
        let metadata: Value = serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
        assert_eq!(metadata["frame"], json!([4, 12]));
        assert_eq!(
            metadata["capturedAtUnixMs"],
            sample.metadata["capturedAtUnixMs"]
        );
        assert_eq!(
            metadata["runtimeRevision"],
            sample.metadata["runtimeRevision"]
        );
        assert_eq!(metadata["markedByPlayer"], true);
        let png = image::open(directory.0.join(metadata["imageFile"].as_str().unwrap()))
            .unwrap()
            .to_rgb8();
        assert_eq!(png.dimensions(), (6, 3));
        assert_eq!(png.get_pixel(5, 1).0, [33, 22, 11]);
        assert_eq!(png.get_pixel(2, 2).0, [66, 55, 44]);
        assert_eq!(png.get_pixel(3, 2).0, [0, 0, 0]);
    }
    #[test]
    fn opt_in_deduplication_manual_mark_and_failure_never_reuse_stale_pixels() {
        let disabled = Control::default();
        disabled.record(frame(&Feed::new()));
        assert!(!disabled.enabled());
        assert!(disabled.0.lock().unwrap().latest.is_none());
        let (control, rx) = queued();
        control.record(frame(&Feed::new()));
        assert!(rx.recv().unwrap().sample.pixels.is_some());
        control.record(frame(&Feed::new()));
        assert!(rx.try_recv().is_err());
        control.record(Sample::without_image(
            json!({"stage":"capture-no-frame","error":"capture-frame"}),
        ));
        assert!(!rx.recv().unwrap().marked); // new failure bypasses the normal sample cooldown
        control.mark_recent().unwrap();
        let marked = rx.recv().unwrap();
        assert!(marked.marked);
        assert!(marked.sample.pixels.is_none());
        assert_eq!(marked.sample.metadata["hasImage"], false);
        control.enable(false).unwrap();
        assert!(control.mark_recent().is_err());
        assert!(control.0.lock().unwrap().latest.is_none());
    }
    #[test]
    fn changed_and_periodic_states_are_retained_and_full_queue_does_not_block() {
        let (control, rx) = queued();
        control.record(frame(&Feed::new()));
        {
            control.0.lock().unwrap().signature.as_mut().unwrap().1 -= Duration::from_secs(3);
        }
        control.record(Sample::without_image(
            json!({"stage":"pixel-gate","level":7}),
        ));
        {
            control.0.lock().unwrap().signature.as_mut().unwrap().1 -= Duration::from_secs(31);
        }
        control.record(Sample::without_image(
            json!({"stage":"pixel-gate","level":7}),
        ));
        assert_eq!(control.0.lock().unwrap().dropped, 1);
        assert_eq!(rx.recv().unwrap().sample.metadata["stage"], "ocr");
        assert_eq!(rx.recv().unwrap().sample.metadata["stage"], "pixel-gate");
        control.record(Sample::without_image(
            json!({"stage":"pixel-gate","level":7}),
        ));
        assert!(rx.try_recv().is_ok());
    }
    #[test]
    fn writer_limits_lock_and_missing_image_preserve_existing_files() {
        let directory = Directory::new();
        let mut writer = Writer::open(directory.0.clone()).unwrap();
        assert!(Writer::open(directory.0.clone()).is_err());
        fs::write(directory.0.join("keep.txt"), "keep").unwrap();
        let job = Job {
            sample: Arc::new(Sample::without_image(json!({"stage":"capture-no-frame"}))),
            marked: false,
        };
        writer.save(&job).unwrap();
        assert!(!fs::read_dir(&directory.0).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|x| x == "png")));
        writer.count = MAX_RECORDS;
        assert_eq!(writer.save(&job), Err("capture-log-limit"));
        writer.count = 0;
        writer.bytes = MAX_BYTES;
        assert_eq!(writer.save(&job), Err("capture-log-limit"));
        assert_eq!(
            fs::read_to_string(directory.0.join("keep.txt")).unwrap(),
            "keep"
        );
    }
    #[test]
    fn disabled_probe_never_discovers_or_captures_game() {
        let mut probe = Probe::default();
        probe.root = Some(PathBuf::from("does-not-exist"));
        assert!(!probe.begin());
        assert!(probe.next_discovery.is_none());
        assert!(probe.next_sample.is_none());
        assert!(!probe.visible());
    }
    #[test]
    fn background_writer_flushes_accepted_samples_on_shutdown() {
        let directory = Directory::new();
        let control = Control::default();
        control.enable_at(directory.0.clone()).unwrap();
        control.record(frame(&Feed::new()));
        control.finish();
        assert!(!control.enabled());
        assert_eq!(control.0.lock().unwrap().saved, 1);
        assert!(control.0.lock().unwrap().error.is_none());
        assert!(!Control::default().enabled());
        assert!(Writer::open(directory.0.clone()).is_ok());
    }
}
