//! Explicit isolated chain review. No LCU/game access; desktop pixels only on CI with an owned full-screen fixture.
use crate::{
    controller::Config,
    pipeline::{Delivery, Pipeline},
};
use fuze_capture::{
    native::{CaptureFeed, FrameStatus, GpuFixture, GpuFixtureReader, Result},
    screen::{NativeScreen, ScreenCapture, ScreenDriver},
};
use fuze_contract::{titles::Atlas, Slot};
use fuze_runtime::session::Phase;
use std::time::{Duration, Instant};

struct Offline {
    reader: GpuFixtureReader,
    texture: GpuFixture,
    bytes: Vec<u8>,
    size: (u32, u32),
    mode: String,
    reads: u32,
    blank: GpuFixture,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Offline {
    fn read(&mut self) -> Result<()> {
        let atlas =
            self.reader
                .read(if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
                    &self.blank
                } else {
                    &self.texture
                })?;
        self.size = (atlas.title_width, atlas.title_height);
        self.bytes.clear();
        self.bytes.extend_from_slice(atlas.bytes());
        Ok(())
    }
}
impl ScreenDriver for Offline {
    fn selection(&self, _: bool) -> Option<fuze_contract::selection_pixels::Evidence> {
        self.reader.selection()
    }
    fn start(&mut self) -> Result<()> {
        if self.mode == "offline-bitmap" {
            Err("fixture-start".into())
        } else {
            Ok(())
        }
    }
    fn stop(&mut self) {}
    fn stream(&mut self, _: Duration) -> Result<FrameStatus> {
        self.reads += 1;
        if self.mode == "offline-drop" && self.reads > 1 {
            return Err("fixture-stream-failed".into());
        }
        self.read()?;
        Ok(FrameStatus::Ready)
    }
    fn bitmap(&mut self) -> Result<()> {
        self.read()
    }
    fn titles(&self, _: bool) -> Result<Atlas<'_>> {
        Ok(Atlas::new(&self.bytes, self.size.0, self.size.1)?)
    }
}
struct BitmapOnly(NativeScreen);
impl ScreenDriver for BitmapOnly {
    fn selection(&self, _: bool) -> Option<fuze_contract::selection_pixels::Evidence> {
        self.0.selection(true)
    }
    fn start(&mut self) -> Result<()> {
        Err("fixture-force-native-bitmap".into())
    }
    fn stop(&mut self) {
        self.0.stop();
    }
    fn stream(&mut self, _: Duration) -> Result<FrameStatus> {
        Err("fixture-no-stream".into())
    }
    fn bitmap(&mut self) -> Result<()> {
        self.0.bitmap()
    }
    fn titles(&self, _: bool) -> Result<Atlas<'_>> {
        self.0.titles(true)
    }
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sample {
    file: String,
    expected_names: Vec<String>,
}

/// args: worker, ORT, model, dictionary, fixtures, bundled data, capture mode, OCR backend.
pub fn run(
    args: &[String],
    mut present: impl FnMut(&Pipeline, Option<&Delivery>, usize) -> Result<()>,
) -> Result<serde_json::Value> {
    if args.len() != 8 {
        return Err("capture-review-arguments".into());
    }
    let mode = args[6].as_str();
    let ci = matches!(mode, "screen" | "bitmap");
    if !ci && !matches!(mode, "offline-stream" | "offline-bitmap" | "offline-drop") {
        return Err("capture-review-mode".into());
    }
    if ci
        && !(std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("FUZE_RUST_CI_VISIBLE").as_deref() == Ok("1"))
    {
        return Err("screen-review-ci-only".into());
    }
    let root = std::fs::canonicalize(&args[4])?;
    let manifest = root.join("manifest.json");
    if std::fs::metadata(&manifest)?.len() > 65536 {
        return Err("capture-review-limit".into());
    }
    let samples: Vec<Sample> = serde_json::from_slice(&std::fs::read(manifest)?)?;
    if samples.is_empty() || samples.len() > 16 {
        return Err("capture-review-limit".into());
    }
    let config = Config {
        roots: vec![fuze_data::Root {
            path: args[5].clone().into(),
            source: fuze_data::Source::Bundled,
        }],
        worker: args[0].clone().into(),
        ort: args[1].clone().into(),
        model: args[2].clone().into(),
        dictionary: args[3].clone().into(),
        backend: args[7].clone(),
        locale: Some(fuze_data::Locale::ZhCn),
        augment_overlay: false,
        icon_downloads: false,
        level_gate: true,
    };
    let data = config.prepare_data(8, fuze_data::Locale::ZhCn)?;
    let details = data.champion_detail(8)?;
    // Exact production preparation, including the independent recognition catalog.
    let mut pipeline = config.prepare(8, fuze_data::Locale::ZhCn)?;
    if pipeline.has_capture() || pipeline.wake_at().is_some() {
        return Err("loading-captured".into());
    }
    let mut frames = Vec::new();
    let mut last_snapshot = None;
    let mut closes = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let path = root.join(&sample.file).canonicalize()?;
        if !path.starts_with(&root) || std::fs::metadata(&path)?.len() > 32 * 1024 * 1024 {
            return Err("capture-review-limit".into());
        }
        let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(2160);
        limits.max_alloc = Some(96 * 1024 * 1024);
        reader.limits(limits);
        let mut image = reader.decode()?.to_rgb8();
        if ci {
            let (w, h) = fuze_native_overlay::native::fixture_screen_size();
            fuze_contract::titles::regions(w as u32, h as u32)?;
            image = image::imageops::resize(
                &image,
                w as u32,
                h as u32,
                image::imageops::FilterType::Lanczos3,
            );
        }
        let pixels: Vec<u8> = image
            .pixels()
            .flat_map(|p| [p[2], p[1], p[0], 255])
            .collect();
        let owned = if ci {
            Some(fuze_native_overlay::native::HiddenOverlay::fixture_source(
                fuze_native_overlay::native::SourceImage {
                    bytes: pixels.clone(),
                    width: image.width(),
                    height: image.height(),
                },
            )?)
        } else {
            None
        };
        pipeline.phase(Phase::InProgress)?;
        if mode == "screen" {
            pipeline.capture_screen()?; // Exact production entry; no game window/path dependency.
        } else {
            let feed: Box<dyn CaptureFeed> = if mode == "bitmap" {
                Box::new(ScreenCapture::with_driver(BitmapOnly(
                    NativeScreen::default(),
                )))
            } else {
                let reader = GpuFixtureReader::new()?;
                let texture = reader.upload(&pixels, image.width(), image.height())?;
                let blank = reader.upload(&vec![0; pixels.len()], image.width(), image.height())?;
                Box::new(ScreenCapture::with_driver(Offline {
                    reader,
                    texture,
                    bytes: Vec::new(),
                    size: (0, 0),
                    mode: mode.into(),
                    reads: 0,
                    blank,
                    closed: closed.clone(),
                }))
            };
            pipeline.capture_feed(feed)?;
        }
        for repeat in 0..3 {
            pipeline.level(3);
            let deadline = Instant::now() + Duration::from_secs(12);
            let delivery = loop {
                if let Some(source) = &owned {
                    source.pulse();
                }
                if let Some(wake) = pipeline.wake_at() {
                    std::thread::sleep(Duration::from_millis(
                        wake.saturating_sub(pipeline.now()).min(500),
                    ));
                }
                if let Some(delivery) = pipeline.tick_wgc()? {
                    break Some(delivery);
                }
                if sample.expected_names.is_empty()
                    && pipeline.capture_state == "waiting-selection"
                    && Instant::now() + Duration::from_secs(10) >= deadline
                {
                    break None;
                }
                if Instant::now() >= deadline {
                    // Fixed/numeric fixture diagnostics only, never desktop pixels or paths.
                    println!(
                        "{}",
                        serde_json::json!({
                            "passed":false,"error":"capture-review-timeout",
                            "sample":index,"repeat":repeat,"fixture_width":image.width(),
                            "fixture_height":image.height(),"capture_state":pipeline.capture_state,
                            "source":pipeline.capture_source(),"fallback":pipeline.capture_fallback(),
                            "recent_frame_ms":pipeline.last_capture.map(|at|at.elapsed().as_millis()),
                            "skipped_frames":pipeline.skipped_frames,"busy_frames":pipeline.busy_frames,
                            "interval_ms":pipeline.scheduling().1
                        })
                    );
                    return Err("capture-review-timeout".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let Some(delivery) = delivery else {
                if pipeline.frames.current().is_some() {
                    return Err("capture-review-unexpected-popup".into());
                }
                frames.push(serde_json::json!({"sample":index,"repeat":repeat,"screen_gate_skipped_ocr":true}));
                continue;
            };
            let actual: [Option<u32>; 3] = delivery.snapshot.slots.each_ref().map(|s| match s {
                Slot::Known { id, .. } => Some(*id),
                _ => None,
            });
            let expected: [Option<u32>; 3] = std::array::from_fn(|i| {
                sample
                    .expected_names
                    .get(i)
                    .and_then(|name| data.catalog().get(name).copied())
            });
            if actual != expected {
                return Err("capture-review-ocr-mismatch".into());
            }
            if delivery.snapshot.selection_confirmed
                || delivery.snapshot.actual_stage != fuze_contract::ActualStage::Unknown
            {
                return Err("capture-review-claim-inferred".into());
            }
            if expected.iter().any(Option::is_some) {
                let published = pipeline
                    .frames
                    .current()
                    .ok_or("capture-review-frame-missing")?;
                for i in 0..3 {
                    let expected_card =
                        expected[i].and_then(|id| details.augments.iter().find(|a| a.id == id));
                    if published.cards[i].as_deref() != expected_card {
                        return Err("capture-review-card-data".into());
                    }
                }
            }
            if mode.ends_with("bitmap") && pipeline.capture_source() != "screen-bitmap" {
                return Err("capture-review-no-bitmap".into());
            }
            if mode == "offline-drop" && repeat > 0 && pipeline.capture_source() != "screen-bitmap"
            {
                return Err("capture-review-no-recovery".into());
            }
            present(&pipeline, Some(&delivery), frames.len())?;
            frames.push(serde_json::json!({"sample":index,"repeat":repeat,"source":pipeline.capture_source(),
                "fallback":pipeline.capture_fallback(),"matched":true,"slots":actual.map(|s|s.is_some()),
                "statistics_slots":pipeline.frames.current().map(|f|f.cards.each_ref().map(|c|c.as_ref().is_some_and(|c|c.win.is_some() && c.pick.is_some()))),
                "ocr_ms":delivery.ocr_ms,"total_ms":delivery.total_ms,"roi_bytes":delivery.roi_bytes}));
            last_snapshot = Some(delivery.snapshot);
        }
        if !ci && pipeline.frames.current().is_some() {
            closed.store(true, std::sync::atomic::Ordering::Relaxed);
            let start = Instant::now();
            while pipeline.frames.current().is_some() {
                if pipeline.tick_wgc()?.is_some() {
                    return Err("capture-review-ocr-on-absent-ui".into());
                }
                if start.elapsed() > Duration::from_millis(1600) {
                    return Err("capture-review-close-delay".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            closes.push(start.elapsed().as_secs_f64() * 1000.);
            present(&pipeline, None, frames.len())?;
        }
        pipeline.phase(Phase::Loading)?;
        if pipeline.has_capture() || pipeline.frames.current().is_some() {
            return Err("capture-review-stale-source".into());
        }
        drop(owned);
    }
    pipeline.phase(Phase::PostGame)?;
    if mode == "screen" && !frames.iter().any(|f| f["source"] == "screen-stream") {
        return Err("capture-review-no-screen-stream".into());
    }
    present(&pipeline, None, frames.len())?;
    if pipeline.wake_at().is_some()
        || pipeline.has_capture()
        || !pipeline.worker.reaped()
        || pipeline
            .overlay
            .update(last_snapshot.ok_or("capture-review-empty")?)?
    {
        return Err("capture-review-cleanup".into());
    }
    Ok(
        serde_json::json!({"passed":true,"mode":mode,"backend":args[7],"frames":frames,
        "loading_capture_started":false,"worker_launches":pipeline.worker.launches(),"worker_reaped":true,
        "data_version":data.version(),"statistics_scope":"overall","selection_confirmed":false,
        "actual_stage":null,"game_access":false,"system_input":false,
        "absent_ui_close_ms":closes,"native_screen_tested":ci,"real_game_verified":false}),
    )
}
