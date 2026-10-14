use fuze_contract::Slot;
use fuze_ocr::{Recognizer, Result};
use serde::Deserialize;
use std::{collections::HashMap, fs, path::PathBuf, time::Instant};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sample {
    file: String,
    expected_ids: Vec<u32>,
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err(
            "usage: fuze-ocr ORT_DLL MODEL DICTIONARY_JSON CATALOG_JSON FIXTURE_DIRECTORY".into(),
        );
    }
    let dll = fs::canonicalize(&args[0])?;
    ort::init_from(dll.to_string_lossy())
        .with_name("fuze-offline-prototype")
        .commit()?;
    let dictionary: Vec<String> = serde_json::from_slice(&fs::read(&args[2])?)?;
    let catalog: HashMap<String, u32> = serde_json::from_slice(&fs::read(&args[3])?)?;
    let started = Instant::now();
    let mut engine = Recognizer::new(&PathBuf::from(&args[1]), dictionary, catalog)?;
    let init_ms = started.elapsed().as_secs_f64() * 1000.0;
    let warm = Instant::now();
    engine.warm_up()?;
    let warm_ms = warm.elapsed().as_secs_f64() * 1000.0;
    let root = fs::canonicalize(&args[4])?;
    let samples: Vec<Sample> = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    if samples.len() > 32 {
        return Err("too-many-fixtures".into());
    }
    let mut results = Vec::new();
    let mut passed = true;
    for (index, sample) in samples.iter().enumerate() {
        let file = fs::canonicalize(root.join(&sample.file))?;
        if !file.starts_with(&root) || fs::metadata(&file)?.len() > 32 * 1024 * 1024 {
            return Err("invalid-fixture".into());
        }
        let mut reader = image::ImageReader::open(file)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(2160);
        limits.max_alloc = Some(96 * 1024 * 1024);
        reader.limits(limits);
        let frame = reader.decode()?.to_rgb8();
        let mut runs = Vec::new();
        let mut ids = Vec::new();
        for _ in 0..3 {
            let result = engine.recognize(&frame)?;
            let current: Vec<Option<u32>> = result
                .slots
                .iter()
                .map(|s| {
                    if let Slot::Known { id, .. } = s {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect();
            let expected: Vec<Option<u32>> = (0..3)
                .map(|i| sample.expected_ids.get(i).copied())
                .collect();
            passed &= current == expected;
            ids = current;
            runs.push(result.elapsed_ms);
        }
        results.push(serde_json::json!({"sample":index,"matched_ids":ids,"runs_ms":runs,"matches_expected":ids==(0..3).map(|i|sample.expected_ids.get(i).copied()).collect::<Vec<_>>()}));
    }
    println!(
        "{}",
        serde_json::json!({"backend":"cpu","init_ms":init_ms,"warm_ms":warm_ms,"samples":results,"passed":passed,"actual_stage":null,"selection_confirmed":false})
    );
    if !passed {
        return Err("fixture-mismatch".into());
    }
    Ok(())
}
fn main() {
    if run().is_err() {
        eprintln!("fuze-ocr: offline-validation-failed");
        std::process::exit(1);
    }
}
