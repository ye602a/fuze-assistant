use fuze_contract::{
    wire::{self, ActiveBackend, Identity, Reply, Request},
    Slot,
};
use fuze_runtime::{
    gate::Selection,
    session::{FrameKey, Phase, Session},
};
use fuze_worker_host::Worker;
use std::{
    fs, io,
    path::Path,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
// Test child modes are explicit and never discover processes, open windows, or load user data.
fn probe(mode: &str) -> Result<()> {
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
    if mode == "crash" {
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
fn fixtures(args: &[String]) -> Result<()> {
    if args.len() != 7 {
        return Err("fixture-arguments".into());
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
    let mut session = Session::new("zh-CN@fixture-v1".into())?;
    session.update_phase(Phase::Loading)?;
    let startup = Instant::now();
    let mut worker = Worker::start(
        Path::new(&args[0]),
        &[
            args[1].clone(),
            args[2].clone(),
            args[3].clone(),
            args[4].clone(),
            args[6].clone(),
        ],
        Duration::from_secs(30),
    )?;
    let startup_ms = startup.elapsed().as_secs_f64() * 1000.;
    session.update_phase(Phase::InProgress)?;
    let epoch = Instant::now();
    let now = || epoch.elapsed().as_millis() as u64;
    let mut reader = fuze_capture::native::GpuFixtureReader::new()?;
    let mut sequence = 0;
    let mut reports = Vec::new();
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
        let uploaded = reader.upload(&bytes, image.width(), image.height())?;
        for _ in 0..3 {
            session.update_level(3, now());
            if let Some(wake) = session.wake_at(now()) {
                std::thread::sleep(Duration::from_millis(wake.saturating_sub(now())));
            }
            sequence += 1;
            let ticket = session
                .begin(FrameKey { epoch: 1, sequence }, now())
                .ok_or("session-job-rejected")?;
            let started = Instant::now();
            let atlas = reader.read(&uploaded)?;
            let request = Request::from_atlas(
                Identity {
                    generation: ticket.generation,
                    sequence: ticket.sequence,
                },
                &atlas,
            );
            let roi_bytes = request.pixels.len();
            let reply = match worker.run(request, Duration::from_secs(3)) {
                Ok(reply) => reply,
                Err(error) => {
                    session.fail(ticket, now());
                    return Err(error.into());
                }
            };
            let total_ms = started.elapsed().as_secs_f64() * 1000.;
            let Reply::Result { slots, ocr_ms, .. } = reply else {
                return Err("unexpected-reply".into());
            };
            let snapshot = session
                .finish(ticket, slots, Selection::Unknown, now())
                .ok_or("session-result-rejected")?;
            let actual: Vec<Option<u32>> = snapshot
                .slots
                .iter()
                .map(|slot| match slot {
                    Slot::Known { id, .. } => Some(*id),
                    _ => None,
                })
                .collect();
            let expected: Vec<Option<u32>> = (0..3)
                .map(|i| sample.expected_ids.get(i).copied())
                .collect();
            if actual != expected {
                return Err("fixture-mismatch".into());
            }
            reports.push(serde_json::json!({"sample":sample_index,"sequence":sequence,"roi_bytes":roi_bytes,"total_ms":total_ms,"ocr_ms":ocr_ms,"matched":true}));
        }
    }
    // A context reset clears pending output; a real UI must consume Cleanup.
    let cleanup = session
        .update_phase(Phase::PostGame)?
        .ok_or("cleanup-missing")?;
    worker.stop()?;
    if !cleanup.clear_overlay || !cleanup.stop_capture || session.wake_at(now()).is_some() {
        return Err("postgame-not-idle".into());
    }
    println!(
        "{}",
        serde_json::json!({"passed":true,"startup_ms":startup_ms,"backend":worker.backend,"fallback_count":worker.fallback_count,"worker_launches":1,"worker_reaped":worker.stopped(),"frames":reports,"actual_stage":null,"selection_confirmed":false,"game_access":false})
    );
    Ok(())
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--probe") if args.len() == 2 => probe(&args[1]),
        Some("--lifecycle") if args.len() == 1 => lifecycle(),
        #[cfg(windows)]
        Some("--fixtures") => fixtures(&args[1..]),
        _ => Err("host-arguments".into()),
    }
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    if run().is_err() {
        eprintln!("fuze-worker-host: failed");
        std::process::exit(1);
    }
}
