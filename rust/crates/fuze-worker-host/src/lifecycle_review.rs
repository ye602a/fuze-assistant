//! Explicit offline replay through the production controller, real OCR, and UI publisher.
//! Inputs are committed fixtures/synthetic text. No discovery, game API, or desktop capture.
use crate::{
    controller::{Config, Controller, Monitor, ReviewInputs},
    desktop::Status,
};
use fuze_capture::{
    native::{FrameStatus, GpuFixtureReader, Result},
    screen::{ScreenCapture, ScreenDriver},
};
use fuze_contract::{titles::Atlas, ActualStage, Slot};
use fuze_lcu::context::Context;
use fuze_runtime::{gate::Mode, session::Phase};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Pixels {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}
#[derive(Default)]
struct Input {
    image: usize,
    stalled: bool,
    reads: u64,
    starts: u64,
    stops: u64,
}
struct Feed {
    frames: Arc<Vec<Pixels>>,
    input: Arc<Mutex<Input>>,
    selected: usize,
}
impl ScreenDriver for Feed {
    fn start(&mut self) -> Result<()> {
        self.input.lock().unwrap().starts += 1;
        Ok(())
    }
    fn stop(&mut self) {
        self.input.lock().unwrap().stops += 1;
    }
    fn stream(&mut self, _: Duration) -> Result<FrameStatus> {
        let mut input = self.input.lock().unwrap();
        input.reads += 1;
        if input.stalled {
            return Ok(FrameStatus::Timeout);
        }
        self.selected = input.image;
        Ok(FrameStatus::Ready)
    }
    fn bitmap(&mut self) -> Result<()> {
        let mut input = self.input.lock().unwrap();
        input.reads += 1;
        self.selected = input.image;
        Ok(())
    }
    fn titles(&self, _: bool) -> Result<Atlas<'_>> {
        let p = &self.frames[self.selected];
        Ok(Atlas::new(&p.bytes, p.width, p.height)?)
    }
}

fn fixtures(root: &str) -> Result<Vec<Pixels>> {
    let root = std::fs::canonicalize(root)?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
    let mut gpu = GpuFixtureReader::new()?;
    let mut result = Vec::new();
    for row in manifest.as_array().ok_or("capture-review-manifest")? {
        let path = root
            .join(row["file"].as_str().ok_or("capture-review-manifest")?)
            .canonicalize()?;
        if !path.starts_with(&root) || std::fs::metadata(&path)?.len() > 32 * 1024 * 1024 {
            return Err("capture-review-limit".into());
        }
        let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(2160);
        limits.max_alloc = Some(96 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode()?.to_rgb8();
        let bytes: Vec<u8> = image
            .pixels()
            .flat_map(|p| [p[2], p[1], p[0], 255])
            .collect();
        let texture = gpu.upload(&bytes, image.width(), image.height())?;
        let atlas = gpu.read(&texture)?;
        result.push(Pixels {
            bytes: atlas.bytes().to_vec(),
            width: atlas.title_width,
            height: atlas.title_height,
        });
    }
    if result.len() != 3 {
        return Err("capture-review-manifest".into());
    }
    // Same first title, center/right obscured: display grace must not combine new selections.
    let p = &result[0];
    let mut bytes = p.bytes.clone();
    for row in bytes.chunks_exact_mut(p.width as usize * 3 * 4) {
        row[p.width as usize * 4..].fill(0);
    }
    result.push(Pixels {
        bytes,
        width: p.width,
        height: p.height,
    });
    let image = image::open(root.join("synthetic-shard-titles.png"))?.to_rgb8();
    require(
        image.dimensions() == (900, 40),
        "capture-review-shard-fixture",
    )?;
    result.push(Pixels {
        bytes: image
            .pixels()
            .flat_map(|p| [p[2], p[1], p[0], 255])
            .collect(),
        width: 300,
        height: 40,
    });
    Ok(result)
}
fn context(phase: Phase, champion: u32, game: u64) -> Result<Context> {
    let phase_text = if phase == Phase::Loading {
        "GameStart"
    } else {
        "InProgress"
    };
    // Synthetic identities exist only in this fixed parser fixture, never output.
    let flow = serde_json::json!({"phase":phase_text,"gameData":{"queue":{"id":2400},"gameId":game,
        "teamOne":[{"summonerId":7,"championId":champion}],"teamTwo":[]}});
    Ok(
        fuze_lcu::context::parse(phase, &serde_json::to_vec(&flow)?, br#"{"summonerId":7}"#)
            .map_err(|_| "capture-review-context")?,
    )
}
fn require(ok: bool, error: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(error.into())
    }
}
fn ids(status: &Status) -> [Option<u32>; 3] {
    status.pipeline.overlay.as_ref().map_or([None; 3], |f| {
        f.snapshot.slots.each_ref().map(|s| match s {
            Slot::Known { id, .. } => Some(*id),
            _ => None,
        })
    })
}
struct Replay<F> {
    controller: Controller,
    monitor: Monitor,
    inputs: ReviewInputs,
    frames: Arc<Vec<Pixels>>,
    input: Arc<Mutex<Input>>,
    present: F,
    updates: usize,
    workers: Vec<(u32, WorkerExit)>,
}
struct WorkerExit(windows_sys::Win32::Foundation::HANDLE);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
impl<F: FnMut(Status, usize) -> Result<()>> Replay<F> {
    fn tick(&mut self, context: Context, level: Option<u8>) -> Result<Status> {
        self.monitor.review_publish(context);
        self.inputs.level = level.map(|v| (v, Instant::now()));
        if self.inputs.capture.is_none() {
            self.inputs.capture = Some(Box::new(ScreenCapture::with_driver(Feed {
                frames: self.frames.clone(),
                input: self.input.clone(),
                selected: 0,
            })));
        }
        self.controller
            .tick_inputs(&self.monitor, Some(&mut self.inputs))?;
        if let Some((pid, _, _)) = self.controller.usage() {
            if !self.workers.iter().any(|(id, _)| *id == pid) {
                // Synchronize-only handle to the OCR child this review just launched.
                let handle = unsafe {
                    windows_sys::Win32::System::Threading::OpenProcess(0x0010_0000, 0, pid)
                };
                require(!handle.is_null(), "capture-review-worker-handle")?;
                self.workers.push((pid, WorkerExit(handle)));
            }
        }
        let mut status = Status::default();
        status.mode = 1;
        status.connection = Some(fuze_lcu::reconnect::State::Connected(context.phase));
        status.preferences.augment_overlay = true;
        status.preferences.icon_downloads = false;
        status.pipeline = self.controller.status();
        if let Some(frame) = &status.pipeline.overlay {
            require(
                !frame.snapshot.selection_confirmed
                    && frame.snapshot.actual_stage == ActualStage::Unknown,
                "capture-review-claim-inferred",
            )?;
        }
        (self.present)(status.clone(), self.updates)?;
        self.updates += 1;
        Ok(status)
    }
    fn next(&mut self, context: Context, level: Option<u8>, image: usize) -> Result<Status> {
        self.input.lock().unwrap().image = image;
        let before = self.controller.status().analyses;
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            let status = self.tick(context, level)?;
            if status.pipeline.analyses > before {
                return Ok(status);
            }
            require(Instant::now() < until, "capture-review-lifecycle-timeout")?;
            std::thread::sleep(self.controller.idle_wait().min(Duration::from_millis(250)));
        }
    }
}
pub fn run(
    args: &[String],
    present: impl FnMut(Status, usize) -> Result<()>,
) -> Result<serde_json::Value> {
    if args.len() != 8 || !matches!(args[6].as_str(), "lifecycle" | "lifecycle-minimal") {
        return Err("capture-review-arguments".into());
    }
    let minimal = args[6] == "lifecycle-minimal";
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
    let mut r = Replay {
        controller: Controller::new(config),
        monitor: Monitor::review(),
        inputs: ReviewInputs {
            capture: None,
            level: None,
        },
        frames: Arc::new(fixtures(&args[4])?),
        input: Arc::new(Mutex::new(Input {
            image: 2,
            ..Default::default()
        })),
        present,
        updates: 0,
        workers: Vec::new(),
    };
    let loading = context(Phase::Loading, 0, 1)?;
    let playing = context(Phase::InProgress, 0, 1)?;
    let warmed = r.tick(loading, None)?;
    let token = r
        .controller
        .review_pipeline()
        .ok_or("capture-review-prewarm")?
        .cancellation();
    require(
        warmed.pipeline.capture == "prewarmed"
            && r.input.lock().unwrap().reads == 0
            && warmed.pipeline.analyses == 0,
        "capture-review-loading-captured",
    )?;
    let first = r.next(playing, Some(1), 2)?;
    require(
        !token.cancelled()
            && first
                .pipeline
                .schedule
                .is_some_and(|s| s.0 == Mode::Saving && s.1 == 8000),
        "capture-review-loading-handoff",
    )?;
    require(
        r.input.lock().unwrap().starts == 0,
        "capture-review-saving-stream",
    )?;
    let at = Instant::now();
    let full = r.next(playing, Some(3), 0)?;
    let first_ms = at.elapsed().as_millis();
    require(
        first_ms < 3000 && ids(&full) == [Some(1032), Some(1390), Some(1129)],
        "capture-review-level-wakeup",
    )?;
    require(
        r.input.lock().unwrap().starts == 1 && full.pipeline.capture_source == "screen-stream",
        "capture-review-stream-resume",
    )?;
    let frame = full.pipeline.overlay.as_ref().unwrap();
    require(
        frame.cards.iter().all(|c| {
            c.as_ref()
                .is_some_and(|a| !a.name.is_empty() && a.win.is_none() && a.pick.is_none())
        }) && frame.scores == [None; 3],
        "capture-review-unknown-hero-stats",
    )?;
    let first_context = frame.context;
    let partial = r.next(playing, Some(3), 3)?;
    require(
        ids(&partial) == ids(&full) && partial.pipeline.overlay.as_ref().unwrap().retained,
        "capture-review-retention",
    )?;
    r.input.lock().unwrap().stalled = true;
    let until = Instant::now() + Duration::from_secs(4);
    while r.tick(playing, None)?.pipeline.overlay.is_some() {
        require(Instant::now() < until, "capture-review-expiry")?;
        std::thread::sleep(Duration::from_millis(100));
    }
    r.input.lock().unwrap().stalled = false;
    require(
        ids(&r.next(playing, None, 0)?) == ids(&full),
        "capture-review-level-unavailable",
    )?;
    let reroll = r.next(playing, Some(3), 1)?;
    require(
        ids(&reroll) == [Some(1332), None, None]
            && !reroll.pipeline.overlay.as_ref().unwrap().retained,
        "capture-review-reroll-mixed",
    )?;
    let shard = r.next(playing, Some(3), 4)?;
    require(
        shard.pipeline.capture == "stat-shards" && shard.pipeline.overlay.is_none(),
        "capture-review-shards",
    )?;
    require(
        ids(&r.next(playing, Some(3), 0)?) == ids(&full),
        "capture-review-after-shards",
    )?;
    let known = context(Phase::InProgress, 8, 1)?;
    let enriched = r.next(known, Some(3), 0)?;
    let frame = enriched
        .pipeline
        .overlay
        .as_ref()
        .ok_or("capture-review-hero-refresh")?;
    require(
        token.cancelled() && frame.context != first_context && ids(&enriched) == ids(&full),
        "capture-review-hero-refresh",
    )?;
    require(
        frame.cards.iter().all(|c| {
            c.as_ref().is_some_and(|c| {
                if minimal {
                    c.win.is_none() && c.pick.is_none()
                } else {
                    c.win.is_some() && c.pick.is_some()
                }
            })
        }),
        "capture-review-statistics",
    )?;
    for level in [7, 11, 15] {
        require(
            ids(&r.next(known, Some(level), 0)?) == ids(&full),
            "capture-review-later-level",
        )?;
    }
    let reads = r.input.lock().unwrap().reads;
    let cancellation = r.controller.review_pipeline().unwrap().cancellation();
    let ended = r.tick(Context::idle(Phase::PostGame), None)?;
    require(
        cancellation.cancelled()
            && r.controller.review_pipeline().is_none()
            && ended.pipeline.overlay.is_none()
            && ended.pipeline.recent.is_none()
            && r.input.lock().unwrap().reads == reads,
        "capture-review-end-cleanup",
    )?;
    let next = r.next(context(Phase::InProgress, 8, 2)?, None, 0)?;
    require(
        ids(&next) == ids(&full)
            && next.pipeline.overlay.as_ref().unwrap().context != frame.context,
        "capture-review-new-match",
    )?;
    let cancellation = r.controller.review_pipeline().unwrap().cancellation();
    r.controller.suspend();
    r.tick(Context::idle(Phase::Disconnected), None)?;
    require(
        cancellation.cancelled() && r.controller.review_pipeline().is_none(),
        "capture-review-pause-cleanup",
    )?;
    require(
        !r.workers.is_empty()
            && r.workers.iter().all(|(_, handle)| unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(handle.0, 0) == 0
            }),
        "capture-review-worker-reap",
    )?;
    Ok(
        serde_json::json!({"passed":true,"mode":args[6],"controller_lifecycle":true,
        "first_threshold_to_delivery_ms":first_ms,"ui_updates":r.updates,"worker_reaped":true,
        "loading_capture_started":false,"unknown_hero_recognized":true,"statistics_available":!minimal,
        "occlusion_expired_without_frames":true,"reroll_preserved_positions":true,"shards_hidden":true,
        "formal_after_shards_recognized":true,"new_match_isolated":true,"statistics_scope":"overall",
        "selection_confirmed":false,"actual_stage":null,"game_access":false,"system_input":false,
        "native_screen_tested":false,"real_game_verified":false}),
    )
}
