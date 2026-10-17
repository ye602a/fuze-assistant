//! Tiny inert archives exercise native extraction/copy rules without starting executables.
use super::*;

fn archive(path: &Path, extra: Option<&str>) {
    let mut entries = Vec::new();
    let mut bodies = Vec::new();
    for name in include_str!("../../../bundle-files.txt")
        .lines()
        .chain(["resources/inference.onnx", "resources/dictionary.json"])
        .chain(extra)
    {
        let bytes = match name {
            "build.json" => br#"{"revision":"111111111111","sequence":1,"channel":"rust-development-preview","uiFrontend":"slint"}"#.to_vec(),
            "preview.json" => br#"{"bundled_data":"resources/client-data","worker":"fuze-ocr-worker.exe","ort":"onnxruntime.dll","model":"resources/inference.onnx","dictionary":"resources/dictionary.json","locale":"auto","backend":"auto"}"#.to_vec(),
            _ => b"inert public fixture, never executed".to_vec(),
        };
        let mut compressor =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        compressor.write_all(&bytes).unwrap();
        let packed = compressor.finish().unwrap();
        entries.push(serde_json::json!({"path":name,"size":bytes.len(),"compressed":packed.len()}));
        bodies.push(packed);
    }
    let index = serde_json::to_vec(
        &serde_json::json!({"schema":1,"revision":"111111111111","files":entries}),
    )
    .unwrap();
    let mut out = File::create(path).unwrap();
    out.write_all(b"FUZEUP01").unwrap();
    out.write_all(&(index.len() as u32).to_le_bytes()).unwrap();
    out.write_all(&index).unwrap();
    for body in bodies {
        out.write_all(&body).unwrap();
    }
}

#[test]
fn managed_bundle_contract_extracts_current_files_and_excludes_portable_private_paths() {
    let root = std::env::temp_dir().join(format!(
        "fuze-bundle-contract-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    safe_dir(&root).unwrap();
    let packed = root.join("fixture.fuz");
    archive(&packed, None);
    let extracted = root.join("extracted");
    unpack(&packed, &extracted, "111111111111").unwrap();
    assert_eq!(
        desktop_program(&extracted).unwrap(),
        extracted.join("fuze-ui-lab.exe")
    );
    for name in include_str!("../../../bundle-files.txt").lines() {
        assert!(extracted.join(name).is_file(), "{name}");
    }
    for name in [
        "Start-this-build.cmd",
        "Start-this-build.ps1",
        "preferences.json",
        "observations-v2.json",
    ] {
        fs::write(extracted.join(name), b"fixture omitted from managed copy").unwrap();
        assert!(!allowed(name));
    }
    let copied = root.join("copied");
    safe_dir(&copied).unwrap();
    copy_bundle(&extracted, &copied).unwrap();
    assert!(copied.join("Export-feedback.ps1").is_file());
    for name in [
        "Start-this-build.cmd",
        "Start-this-build.ps1",
        "preferences.json",
        "observations-v2.json",
    ] {
        assert!(!copied.join(name).exists());
    }
    for (i, path) in [
        "Start-this-build.cmd",
        "Start-this-build.ps1",
        "preferences.json",
        "../escape.exe",
        "resources/client-data/CON.json",
    ]
    .into_iter()
    .enumerate()
    {
        archive(&packed, Some(path));
        let target = root.join(format!("rejected-{i}"));
        assert_eq!(
            unpack(&packed, &target, "111111111111"),
            Err("install-entry")
        );
        assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    }
    assert!(
        root.starts_with(std::env::temp_dir())
            && root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("fuze-bundle-contract-")
    );
    fs::remove_dir_all(root).unwrap();
}
