use fuze_contract::{
    wire::{self, ActiveBackend, Identity, Reply, Request},
    Slot,
};
use fuze_runtime::session::{FrameKey, Phase};
use fuze_worker_host::{recovery::Supervisor, Worker};
use std::{
    fs, io,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
// Test child modes are explicit and never discover processes, open windows, or load user data.
fn probe(mode: &str, backend: &str) -> Result<()> {
    let mut gate = [0u8; 4];
    io::stdin().lock().read_exact(&mut gate)?;
    if &gate != b"FZH1" {
        return Err("probe-gate".into());
    }
    if mode == "startup-crash" && backend == "dml" {
        std::process::exit(23);
    }
    if mode == "startup-stall" {
        std::thread::sleep(Duration::from_secs(30));
        return Ok(());
    }
    wire::write_reply(
        &mut io::stdout().lock(),
        &Reply::Ready {
            schema: 1,
            backend: ActiveBackend::Cpu,
            fallback_count: 0,
        },
    )?;
    let request = Request::read(&mut io::stdin().lock())?.ok_or("probe-input")?;
    if mode == "crash" || mode == "recover" && backend == "dml" {
        std::process::exit(23);
    }
    if mode == "stall" {
        std::thread::sleep(Duration::from_secs(30));
        return Ok(());
    }
    let mut id = request.id;
    if mode == "stale" {
        id.generation += 1;
    }
    wire::write_reply(
        &mut io::stdout().lock(),
        &Reply::Result {
            title_kind: fuze_contract::wire::TitleKind::Unknown,
            id,
            slots: std::array::from_fn(|_| Slot::Unreadable),
            ocr_ms: 0.,
            backend: ActiveBackend::Cpu,
            fallback_count: 0,
        },
    )?;
    Ok(())
}
fn lifecycle() -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut cases = Vec::new();
    for mode in ["ok", "crash", "stall", "stale"] {
        let mut child = Worker::start(
            &exe,
            &["--probe".into(), mode.into()],
            Duration::from_secs(5),
        )?;
        let req = Request {
            id: Identity {
                generation: 1,
                sequence: 1,
            },
            width: 1,
            height: 1,
            pixels: vec![0; 12],
        };
        let result = child.run(req, Duration::from_millis(250));
        let expected = if mode == "ok" {
            result.is_ok()
        } else {
            result.is_err() && child.stopped()
        };
        child.stop()?;
        if !expected {
            return Err("lifecycle-mismatch".into());
        }
        cases.push(serde_json::json!({"case":mode,"passed":true,"reaped":child.stopped()}));
    }
    if Worker::start(
        &exe,
        &["--probe".into(), "startup-stall".into()],
        Duration::from_millis(250),
    )
    .is_ok()
    {
        return Err("startup-deadline-missing".into());
    }
    #[cfg(windows)]
    {
        let mut child = Worker::start(
            &exe,
            &["--probe".into(), "stall".into()],
            Duration::from_secs(5),
        )?;
        child.verify_job_close()?;
    }
    let request = |sequence| Request {
        id: Identity {
            generation: 1,
            sequence,
        },
        width: 1,
        height: 1,
        pixels: vec![0; 12],
    };
    for mode in ["recover", "crash", "startup-crash"] {
        let mut supervisor = Supervisor::start(
            &exe,
            &["--probe".into(), mode.into(), "auto".into()],
            Duration::from_secs(5),
        )?;
        if mode != "startup-crash"
            && supervisor.run(request(1), Duration::from_millis(250)).err()
                != Some("worker-recovered-use-next-frame")
        {
            return Err("recovery-not-once".into());
        }
        let next = supervisor.run(request(2), Duration::from_millis(250));
        if (mode == "crash") == next.is_ok() {
            return Err("recovery-result".into());
        }
        if mode == "crash"
            && supervisor.run(request(3), Duration::from_millis(250)).err()
                != Some("worker-stopped")
        {
            return Err("recovery-loop".into());
        }
        supervisor.stop()?;
        if supervisor.launches() != 2 || !supervisor.reaped() {
            return Err("recovery-lifecycle".into());
        }
        cases.push(serde_json::json!({"case":mode,"passed":true,"launches":supervisor.launches(),"reaped":true}));
    }
    let mut explicit = Supervisor::start(
        &exe,
        &["--probe".into(), "crash".into(), "dml".into()],
        Duration::from_secs(5),
    )?;
    if explicit.run(request(1), Duration::from_millis(250)).is_ok()
        || explicit.launches() != 1
        || !explicit.reaped()
    {
        return Err("explicit-mode-restarted".into());
    }
    println!(
        "{}",
        serde_json::json!({"passed":true,"cases":cases,"startup_deadline":true,"game_access":false})
    );
    Ok(())
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sample {
    file: String,
    expected_ids: Vec<u32>,
}

#[cfg(windows)]
fn fixtures(args: &[String], wgc: bool) -> Result<()> {
    use fuze_native_overlay::native::{HiddenOverlay, SourceImage};
    use fuze_worker_host::pipeline::Pipeline;
    if args.len() != 8 {
        return Err("fixture-arguments".into());
    }
    if wgc
        && !(std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("FUZE_RUST_CI_VISIBLE").as_deref() == Ok("1"))
    {
        return Err("wgc-ci-only".into());
    }
    let root = fs::canonicalize(&args[5])?;
    let manifest = root.join("manifest.json");
    if fs::metadata(&manifest)?.len() > 65536 {
        return Err("manifest-limit".into());
    }
    let samples: Vec<Sample> = serde_json::from_slice(&fs::read(manifest)?)?;
    if samples.is_empty() || samples.len() > 32 {
        return Err("sample-limit".into());
    }
    let prepared = fuze_data::prepare(
        &[fuze_data::Root {
            path: args[7].clone().into(),
            source: fuze_data::Source::Bundled,
        }],
        fuze_data::Locale::ZhCn,
        fuze_data::Scope::Detail(8),
    )?;
    let stats = prepared.recommendations(8)?;
    if stats
        .values()
        .any(|s| s.win_rate != Some(0.6) || s.pick_rate != Some(0.12))
    {
        return Err("statistics-fixture-mismatch".into());
    }
    let startup = Instant::now();
    let worker = Supervisor::start_with_catalog(
        Path::new(&args[0]),
        &[
            args[1].clone(),
            args[2].clone(),
            args[3].clone(),
            "-".into(),
            args[6].clone(),
        ],
        Duration::from_secs(30),
        Some(std::sync::Arc::new(wire::encode_catalog(
            prepared.catalog(),
        )?)),
    )?;
    let startup_ms = startup.elapsed().as_secs_f64() * 1000.;
    let mut pipeline = Pipeline::new(worker, &prepared, 8)?;
    if pipeline.has_capture() || pipeline.wake_at().is_some() {
        return Err("loading-started-capture".into());
    }
    let warmed_launches = pipeline.worker.launches();
    pipeline.phase(Phase::InProgress)?;
    if pipeline.worker.launches() != warmed_launches || pipeline.worker.backend().is_none() {
        return Err("loading-worker-not-reused".into());
    }
    let mut reader = fuze_capture::native::GpuFixtureReader::new()?;
    let mut reports = Vec::new();
    let mut last = None;
    let mut sequence = 0;
    let mut capture_start_retries = 0;
    for (sample_index, sample) in samples.iter().enumerate() {
        let path = fs::canonicalize(root.join(&sample.file))?;
        if !path.starts_with(&root) || fs::metadata(&path)?.len() > 32 * 1024 * 1024 {
            return Err("fixture-invalid".into());
        }
        let mut image = image::ImageReader::open(path)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(2160);
        limits.max_alloc = Some(96 * 1024 * 1024);
        image.limits(limits);
        let image = image.decode()?.to_rgb8();
        let bytes: Vec<u8> = image
            .pixels()
            .flat_map(|p| [p[2], p[1], p[0], 255])
            .collect();
        let source = if wgc {
            Some(HiddenOverlay::fixture_source(SourceImage {
                bytes: bytes.clone(),
                width: image.width(),
                height: image.height(),
            })?)
        } else {
            None
        };
        if let Some(source) = &source {
            for attempt in 0..4 {
                match pipeline.capture_owned(source.owned_handle(), true) {
                    Ok(()) => break,
                    Err(error) => {
                        let startup_race = pipeline.capture_failure().is_some_and(|d| {
                            d["operation"] == "create_owned_window_item"
                                && d["hresult"] == "0x80070057"
                        });
                        if !startup_race || attempt == 3 {
                            println!(
                                "{}",
                                serde_json::json!({"passed":false,"error":error,
                                "capture_failure":pipeline.capture_failure(),"capture_start_retries":capture_start_retries})
                            );
                            return Err("reported-capture-start".into());
                        }
                        // Same retained CI-owned HWND, before the first frame only. Production
                        // already retries capture startup through Controller; never retry OCR here.
                        capture_start_retries += 1;
                        source.pulse();
                        std::thread::sleep(Duration::from_millis([100, 250, 500][attempt]));
                    }
                }
            }
        }
        let uploaded = if wgc {
            None
        } else {
            Some(reader.upload(&bytes, image.width(), image.height())?)
        };
        for repeat in 0..3 {
            pipeline.level(3);
            if let Some(wake) = pipeline.wake_at() {
                std::thread::sleep(Duration::from_millis(wake.saturating_sub(pipeline.now())));
            }
            sequence += 1;
            let delivery = if let Some(source) = &source {
                let deadline = Instant::now() + Duration::from_secs(8);
                loop {
                    source.pulse();
                    if let Some(d) = pipeline.tick_wgc()? {
                        break d;
                    }
                    if Instant::now() > deadline {
                        return Err("wgc-delivery-timeout".into());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            } else {
                let atlas = reader.read(uploaded.as_ref().ok_or("fixture-texture")?)?;
                pipeline
                    .recognize(FrameKey { epoch: 1, sequence }, &atlas)?
                    .ok_or("session-job-rejected")?
            };
            let actual: Vec<Option<u32>> = delivery
                .snapshot
                .slots
                .iter()
                .map(|s| match s {
                    Slot::Known { id, .. } => Some(*id),
                    _ => None,
                })
                .collect();
            let expected: Vec<Option<u32>> = (0..3)
                .map(|i| sample.expected_ids.get(i).copied())
                .collect();
            let expected_kind = if expected.iter().any(Option::is_some) {
                wire::TitleKind::Candidate
            } else {
                wire::TitleKind::Unknown
            };
            if delivery.title_kind != expected_kind || delivery.snapshot.selection_confirmed {
                return Err("fixture-selection-kind".into());
            }
            if actual != expected {
                println!(
                    "{}",
                    serde_json::json!({"passed":false,"error":"fixture-mismatch","source":if wgc {"owned_ci_wgc"} else {"offline_gpu_texture"},"sample":sample_index,"known_slots":actual.iter().filter(|s|s.is_some()).count(),"slot_matches":actual.iter().zip(&expected).map(|(a,b)|a==b).collect::<Vec<_>>(),"roi_bytes":delivery.roi_bytes})
                );
                return Err("fixture-mismatch".into());
            }
            if pipeline.overlay.update(delivery.snapshot.clone())? {
                return Err("duplicate-accepted".into());
            }
            if !wgc && repeat > 0 && delivery.ocr_ms != 0.0 {
                return Err("unchanged-title-inferred-again".into());
            }
            reports.push(serde_json::json!({"sample":sample_index,"sequence":sequence,"title_kind":delivery.title_kind,"retained":delivery.retained,"display_visible":delivery.display_visible,"roi_bytes":delivery.roi_bytes,"ocr_ms":delivery.ocr_ms,"total_ms":delivery.total_ms,"matched":true}));
            last = Some(delivery.snapshot);
        }
        // Replacing source closes the old stream before the window is destroyed.
        if wgc {
            pipeline.phase(Phase::Loading)?;
        }
        drop(source);
        if wgc {
            pipeline.phase(Phase::InProgress)?;
        }
    }
    let previous = last.ok_or("missing-result")?;
    if wgc {
        // This branch is CI-visible-only, just like the owned source window above.
        // The UI timer must expire independently of host polling or the OCR worker.
        let timed = HiddenOverlay::start(1, previous.dataset_version.clone())?;
        let mut snapshot = previous.clone();
        snapshot.generation = 1;
        snapshot.sequence = 1;
        if !timed.update_retained(snapshot, Some(Instant::now() + Duration::from_millis(2500)))?
            || !timed.visible(true)?
        {
            return Err("retention-timer-start".into());
        }
        let wait = Instant::now();
        std::thread::sleep(Duration::from_millis(2700));
        let still_visible = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(
                timed.owned_handle() as windows_sys::Win32::Foundation::HWND
            )
        } != 0;
        let reopened = timed.visible(true)?;
        if still_visible || reopened {
            println!(
                "{}",
                serde_json::json!({"error":"retention-timer-expiry","passed":false,
                "still_visible":still_visible,"reopen_accepted":reopened,"wait_ms":wait.elapsed().as_millis(),"game_access":false})
            );
            return Err("retention-timer-expiry".into());
        }
    }
    let mut partial = previous.clone();
    partial.sequence += 1;
    partial.slots[1] = Slot::Unreadable;
    // WGC changes phase between source windows, so its old snapshot must already be rejected.
    if !wgc {
        let mut bad = partial.clone();
        bad.dataset_version = "zh-CN@0.0.0".into();
        if pipeline.overlay.update(bad)? || !pipeline.overlay.update(partial)? {
            return Err("partial-or-context".into());
        }
    }
    let cancellation = pipeline.cancellation();
    cancellation.cancel();
    let mut late = previous.clone();
    late.sequence += 10;
    if pipeline.overlay.update(late)? {
        return Err("cancelled-update-accepted".into());
    }
    pipeline.phase(Phase::PostGame)?;
    if pipeline.wake_at().is_some() || pipeline.overlay.update(previous)? {
        return Err("postgame-not-idle".into());
    }
    println!(
        "{}",
        serde_json::json!({"passed":true,"source":if wgc {"owned_ci_wgc"} else {"offline_gpu_texture"},"frames":reports,"capture_start_retries":capture_start_retries,"overlay_updates":sequence+u64::from(!wgc),"partial_slot_updated":!wgc,"statistics_loaded":stats.len(),"statistics_scope":"overall","dataset_version":prepared.version(),"overlay_cleared":true,"stale_and_duplicate_rejected":true,"worker_reaped":pipeline.worker.reaped(),"worker_launches":pipeline.worker.launches(),"backend":pipeline.worker.backend(),"startup_ms":startup_ms,"retention_timer_checked":wgc,"loading_worker_reused":true,"loading_capture_started":false,"actual_stage":null,"selection_confirmed":false,"game_access":false})
    );
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--probe") if (2..=3).contains(&args.len()) => {
            probe(&args[1], args.get(2).map(String::as_str).unwrap_or("cpu"))
        }
        Some("--lifecycle") if args.len() == 1 => lifecycle(),
        #[cfg(windows)]
        Some("--fixtures") => fixtures(&args[1..], false),
        #[cfg(windows)]
        Some("--wgc-fixtures") => fixtures(&args[1..], true),
        _ => Err("host-arguments".into()),
    }
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    if let Err(error) = run() {
        let text = error.to_string();
        let code = match text.as_str() {
            "fixture-mismatch"
            | "capture-phase"
            | "capture-start"
            | "capture-frame"
            | "capture-atlas"
            | "capture-missing"
            | "wgc-delivery-timeout"
            | "overlay-start"
            | "overlay-reset"
            | "overlay-delivery"
            | "overlay-timeout"
            | "worker-timeout"
            | "worker-not-ready"
            | "postgame-not-idle"
            | "source-dimensions"
            | "statistics-fixture-mismatch"
            | "loading-started-capture"
            | "loading-worker-not-reused"
            | "fixture-selection-kind"
            | "duplicate-accepted"
            | "unchanged-title-inferred-again"
            | "retention-timer-start"
            | "retention-timer-expiry"
            | "partial-or-context"
            | "cancelled-update-accepted" => text.as_str(),
            _ => "host-failed",
        };
        if code != "fixture-mismatch"
            && code != "retention-timer-expiry"
            && text != "reported-capture-start"
        {
            println!("{}", serde_json::json!({"passed":false,"error":code}));
        }
        eprintln!("fuze-worker-host: failed");
        std::process::exit(1);
    }
}
