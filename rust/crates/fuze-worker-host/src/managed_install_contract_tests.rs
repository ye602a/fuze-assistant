//! Tiny inert archives exercise native extraction/copy rules without starting executables.
use super::*;

fn archive(path: &Path, extra: Option<&str>) {
    archive_version(path, extra, "111111111111", 1);
}

fn archive_version(path: &Path, extra: Option<&str>, revision: &str, sequence: u64) {
    let mut entries = Vec::new();
    let mut bodies = Vec::new();
    for name in include_str!("../../../bundle-files.txt")
        .lines()
        .chain(["resources/inference.onnx", "resources/dictionary.json"])
        .chain(extra)
    {
        let bytes = match name {
            "build.json" => serde_json::to_vec(&serde_json::json!({
                "revision": revision, "sequence": sequence,
                "channel": "rust-development-preview", "uiFrontend": "slint"
            })).unwrap(),
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
    let index =
        serde_json::to_vec(&serde_json::json!({"schema":1,"revision":revision,"files":entries}))
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

#[test]
fn staging_public_archives_preserves_inputs_and_never_executes_inert_programs() {
    let root = std::env::temp_dir().join(format!(
        "fuze-stage-contract-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    safe_dir(&root).unwrap();
    let old = root.join("old.fuz");
    let new = root.join("new.fuz");
    archive_version(&old, None, "111111111111", 1);
    archive_version(&new, None, "222222222222", 2);
    let manifest = |path: &Path, revision: &str, sequence| {
        let bytes = fs::read(path).unwrap();
        Manifest {
            schema: 1,
            channel: "rust-preview".into(),
            revision: revision.into(),
            sequence,
            archive: format!("Fuze-Rust-{revision}.fuz"),
            size: bytes.len() as u64,
            sha512: crate::update_io::digest(&bytes, true)
                .unwrap()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            blockmap_sha256: "0".repeat(64),
        }
    };
    let baseline = manifest(&old, "111111111111", 1);
    let target = manifest(&new, "222222222222", 2);
    let original = [fs::read(&old).unwrap(), fs::read(&new).unwrap()];
    let stage = root.join("stage");
    let report = crate::update_validation::staging(&stage, &target, &new, &baseline, &old).unwrap();
    for key in [
        "native_extracted",
        "build_identity_verified",
        "pointer_rollback",
        "pending_guard",
        "rejected_guard",
    ] {
        assert_eq!(report[key], true, "{key}");
    }
    for key in [
        "process_launch",
        "runtime_health_verified",
        "restart_verified",
        "installed",
        "registration_changed",
        "game_access",
    ] {
        assert_eq!(report[key], false, "{key}");
    }
    // All executable entries are inert text: any attempted process launch would fail this test.
    let pointer = fs::read(stage.join("current.json")).unwrap();
    assert_eq!(
        crate::update_validation::staging(&stage, &target, &new, &baseline, &old),
        Err("validation-root-must-be-new")
    );
    assert_eq!(fs::read(stage.join("current.json")).unwrap(), pointer);
    assert_eq!(fs::read(&old).unwrap(), original[0]);
    assert_eq!(fs::read(&new).unwrap(), original[1]);

    let same = root.join("same");
    assert_eq!(
        crate::update_validation::staging(&same, &baseline, &old, &baseline, &old),
        Err("validation-distinct-upgrade-required")
    );
    assert!(!same.exists());
    let mut corrupt = target.clone();
    corrupt.sha512 = "0".repeat(128);
    let bad_hash = root.join("bad-hash");
    assert_eq!(
        crate::update_validation::staging(&bad_hash, &corrupt, &new, &baseline, &old),
        Err("update-final-hash")
    );
    assert!(!bad_hash.join("versions").exists());
    let mut mismatched = target.clone();
    mismatched.sequence += 1;
    let bad_identity = root.join("bad-identity");
    assert_eq!(
        crate::update_validation::staging(&bad_identity, &mismatched, &new, &baseline, &old),
        Err("validation-build-identity")
    );
    assert!(!bad_identity.join("current.json").exists());
    assert!(
        root.starts_with(std::env::temp_dir())
            && root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("fuze-stage-contract-")
    );
    fs::remove_dir_all(root).unwrap();
}
