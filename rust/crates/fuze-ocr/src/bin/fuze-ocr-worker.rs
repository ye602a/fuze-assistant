use fuze_contract::wire::{self, ActiveBackend, Reply, Request};
use fuze_ocr::backend::{Backend, Preference, ResilientRecognizer};
use std::{collections::HashMap, fs, io, io::Read, path::PathBuf};
fn run() -> fuze_ocr::Result<()> {
    let mut gate = [0u8; 4];
    io::stdin().lock().read_exact(&mut gate)?;
    if &gate != b"FZH1" && &gate != b"FZH2" {
        return Err("worker-parent-gate".into());
    }
    let supplied_catalog = if &gate == b"FZH2" {
        Some(wire::read_catalog(&mut io::stdin().lock())?)
    } else {
        None
    };
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("worker-arguments".into());
    }
    // Only public model/catalog paths and backend are arguments. No credentials or account data.
    for (file, limit) in [(&args[1], 64 * 1024 * 1024), (&args[2], 2 * 1024 * 1024)] {
        if fs::metadata(file)?.len() > limit {
            return Err("worker-input-limit".into());
        }
    }
    let dll = fs::canonicalize(&args[0])?;
    ort::init_from(dll.to_string_lossy())
        .with_name("fuze-private-worker")
        .commit()?;
    let dictionary: Vec<String> = serde_json::from_slice(&fs::read(&args[2])?)?;
    let catalog: HashMap<String, u32> = if let Some(catalog) = supplied_catalog {
        catalog
    } else {
        if fs::metadata(&args[3])?.len() > 2 * 1024 * 1024 {
            return Err("worker-input-limit".into());
        }
        serde_json::from_slice(&fs::read(&args[3])?)?
    };
    let mut engine = ResilientRecognizer::new(
        &PathBuf::from(&args[1]),
        dictionary,
        catalog,
        Preference::parse(&args[4])?,
    )?;
    engine.warm_up()?;
    let backend = |e: &ResilientRecognizer| {
        if e.status().active == Backend::Cpu {
            ActiveBackend::Cpu
        } else {
            ActiveBackend::DirectMl
        }
    };
    let mut input = io::stdin().lock();
    let mut out = io::stdout().lock();
    wire::write_reply(
        &mut out,
        &Reply::Ready {
            schema: 1,
            backend: backend(&engine),
            fallback_count: engine.status().fallback_count,
        },
    )?;
    let mut last = None;
    let mut reuse = fuze_ocr::reuse::TitleReuse::default();
    let mut last_reads = [wire::TitleRead::Unknown; 3];
    while let Some(request) = Request::read(&mut input)? {
        let id = request.id;
        if last.is_some_and(|old: wire::Identity| {
            id.generation < old.generation || id.sequence <= old.sequence
        }) {
            return Err("worker-stale-request".into());
        }
        last = Some(id);
        if let Some((slots, title_kind)) = reuse.get(&request) {
            wire::write_reply(
                &mut out,
                &Reply::Result {
                    id,
                    slots,
                    title_kind,
                    title_reads: last_reads,
                    ocr_ms: 0.0,
                    backend: backend(&engine),
                    fallback_count: engine.status().fallback_count,
                },
            )?;
            continue;
        }
        let atlas = request.atlas()?;
        let result = engine.run("gpu_inference_failed", |e| e.recognize_atlas(&atlas));
        match result {
            Ok(result) => {
                last_reads = result.title_reads;
                reuse.remember(request, result.slots.clone(), result.title_kind);
                wire::write_reply(
                    &mut out,
                    &Reply::Result {
                        id,
                        slots: result.slots,
                        title_kind: result.title_kind,
                        title_reads: result.title_reads,
                        ocr_ms: result.elapsed_ms,
                        backend: backend(&engine),
                        fallback_count: engine.status().fallback_count,
                    },
                )?;
            }
            Err(_) => {
                wire::write_reply(&mut out, &Reply::Failed { id })?;
                return Err("worker-inference-failed".into());
            }
        }
    }
    Ok(())
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    if run().is_err() {
        std::process::exit(1);
    }
}
