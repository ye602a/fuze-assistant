//! Immutable per-user Rust versions. Atomic pointer switching; no in-place executable replacement.
use crate::{
    update_channel::{self, Manifest},
    Result,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
pub fn safe_dir(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        if let Ok(m) = fs::symlink_metadata(p) {
            if m.file_attributes() & 0x400 != 0 {
                return Err("install-reparse");
            }
        }
    }
    fs::create_dir_all(path).map_err(|_| "install-directory")
}
pub fn root() -> Result<PathBuf> {
    let p = std::env::var_os("LOCALAPPDATA").ok_or("install-root")?;
    let p = PathBuf::from(p).join("FuzeRustPreview/releases");
    safe_dir(&p)?;
    fs::canonicalize(p).map_err(|_| "install-root")
}
pub fn lock(root: &Path) -> Result<File> {
    safe_dir(root)?;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(root.join("update.lock"))
        .map_err(|_| "update-busy")
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    pub revision: String,
    pub sequence: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub current: Option<Installed>,
    pub previous: Option<Installed>,
    pub pending: bool,
    pub rejected: Option<String>,
}
fn valid(s: &State) -> bool {
    [s.current.as_ref(), s.previous.as_ref()]
        .into_iter()
        .flatten()
        .all(|v| update_channel::revision(&v.revision))
        && s.rejected
            .as_ref()
            .is_none_or(|v| update_channel::revision(v))
}
pub fn read(root: &Path) -> Result<State> {
    if !root.join("current.json").exists() {
        return Ok(State::default());
    }
    let s = serde_json::from_slice(&update_channel::bounded(&root.join("current.json"), 16384)?)
        .map_err(|_| "install-pointer")?;
    if !valid(&s) {
        return Err("install-pointer");
    }
    Ok(s)
}
pub fn save(root: &Path, state: &State) -> Result<()> {
    if !valid(state) {
        return Err("install-pointer");
    }
    safe_dir(root)?;
    let tmp = root.join(format!(
        "pointer-{}-{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "install-clock")?
            .as_nanos()
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|_| "install-pointer")?;
    f.write_all(&serde_json::to_vec(state).map_err(|_| "install-pointer")?)
        .map_err(|_| "install-pointer")?;
    f.sync_all().map_err(|_| "install-pointer")?;
    drop(f);
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    use std::os::windows::ffi::OsStrExt;
    if unsafe {
        MoveFileExW(
            wide(&tmp).as_ptr(),
            wide(&root.join("current.json")).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        let _ = fs::remove_file(tmp);
        return Err("install-pointer");
    }
    Ok(())
}
pub fn rollback(root: &Path) -> Result<()> {
    let mut s = read(root)?;
    let old = s.previous.take().ok_or("rollback-unavailable")?;
    s.rejected = s.current.as_ref().map(|v| v.revision.clone());
    s.current = Some(old);
    s.pending = false;
    save(root, &s)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    size: u64,
    compressed: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: u32,
    revision: String,
    files: Vec<Entry>,
}
fn allowed(path: &str) -> bool {
    if path.len() > 240
        || path.contains('\\')
        || path.split('/').any(|s| {
            s.is_empty()
                || s == "."
                || s == ".."
                || s.ends_with('.')
                || s.starts_with('.')
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || ["con", "prn", "aux", "nul", "com1", "lpt1"].contains(
                    &s.split('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str(),
                )
        })
    {
        return false;
    }
    [
        "fuze-rust-host.exe",
        "fuze-ocr-worker.exe",
        "fuze-update.exe",
        "onnxruntime.dll",
        "DirectML.dll",
        "dxcompiler.dll",
        "dxil.dll",
        "preview.json",
        "build.json",
        "THANKS.md",
        "README.txt",
        "Start-preview.cmd",
        "Check-preview.cmd",
        "Update.cmd",
        "Private-channel.cmd",
        "Forget-private-token.cmd",
        "Rollback.cmd",
        "Install-local.cmd",
    ]
    .contains(&path)
        || path == "resources/inference.onnx"
        || path == "resources/dictionary.json"
        || path.starts_with("resources/client-data/") && path.ends_with(".json")
}
pub fn unpack(archive: &Path, destination: &Path, revision: &str) -> Result<()> {
    if destination.exists() {
        return Err("install-stage-exists");
    }
    safe_dir(destination)?;
    let mut input = File::open(archive).map_err(|_| "install-archive")?;
    let mut header = [0u8; 12];
    input
        .read_exact(&mut header)
        .map_err(|_| "install-archive")?;
    if &header[..8] != b"FUZEUP01" {
        return Err("install-format");
    }
    let size = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    if size > 512 * 1024 {
        return Err("install-index-limit");
    }
    let mut data = vec![0; size];
    input.read_exact(&mut data).map_err(|_| "install-index")?;
    let index: Index = serde_json::from_slice(&data).map_err(|_| "install-index")?;
    if index.schema != 1
        || index.revision != revision
        || index.files.is_empty()
        || index.files.len() > 1024
    {
        return Err("install-index");
    }
    let mut total = 0u64;
    let mut names = HashSet::new();
    for e in &index.files {
        if !allowed(&e.path)
            || !names.insert(e.path.to_ascii_lowercase())
            || e.size > 128 * 1024 * 1024
            || e.compressed > 128 * 1024 * 1024
        {
            return Err("install-entry");
        }
        total = total
            .checked_add(e.size)
            .filter(|v| *v <= 512 * 1024 * 1024)
            .ok_or("install-expanded-limit")?;
    }
    for required in [
        "fuze-rust-host.exe",
        "fuze-ocr-worker.exe",
        "fuze-update.exe",
        "preview.json",
        "build.json",
        "onnxruntime.dll",
    ] {
        if !names.contains(required) {
            return Err("install-missing-file");
        }
    }
    for e in index.files {
        let path = destination.join(e.path);
        safe_dir(path.parent().ok_or("install-path")?)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| "install-write")?;
        let mut compressed = (&mut input).take(e.compressed);
        let mut decoder = flate2::read::DeflateDecoder::new(&mut compressed).take(e.size + 1);
        if std::io::copy(&mut decoder, &mut output).map_err(|_| "install-deflate")? != e.size {
            return Err("install-file-size");
        }
        drop(decoder);
        if compressed.limit() != 0 {
            return Err("install-compressed-size");
        }
        output.sync_all().map_err(|_| "install-write")?;
    }
    if input.read(&mut [0]).map_err(|_| "install-archive")? != 0 {
        return Err("install-trailing-data");
    }
    let config: serde_json::Value = serde_json::from_slice(&update_channel::bounded(
        &destination.join("preview.json"),
        16384,
    )?)
    .map_err(|_| "install-config")?;
    for (key, value) in [
        ("bundled_data", "resources/client-data"),
        ("worker", "fuze-ocr-worker.exe"),
        ("ort", "onnxruntime.dll"),
        ("model", "resources/inference.onnx"),
        ("dictionary", "resources/dictionary.json"),
        ("locale", "auto"),
        ("backend", "auto"),
    ] {
        if config[key] != value {
            return Err("install-config-policy");
        }
    }
    if config.get("cache_data").is_some() {
        return Err("install-config-policy");
    }
    let build: serde_json::Value = serde_json::from_slice(&update_channel::bounded(
        &destination.join("build.json"),
        16384,
    )?)
    .map_err(|_| "install-build")?;
    if build["revision"] != revision || build["channel"] != "rust-development-preview" {
        return Err("install-channel");
    }
    Ok(())
}
pub fn check_bundle(directory: &Path) -> Result<()> {
    let mut child = Command::new(directory.join("fuze-rust-host.exe"))
        .arg("--check-bundle")
        .arg(directory.join("preview.json"))
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| "install-check-start")?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                return if s.success() {
                    Ok(())
                } else {
                    Err("install-check-failed")
                }
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("install-check-wait");
            }
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("install-check-timeout");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub fn activate(
    root: &Path,
    manifest: &Manifest,
    verify: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    manifest.validate()?;
    let mut state = read(root)?;
    if state.pending {
        return Err("install-validation-pending");
    }
    if state
        .current
        .as_ref()
        .is_some_and(|s| manifest.sequence <= s.sequence || manifest.revision == s.revision)
        || state.rejected.as_ref() == Some(&manifest.revision)
    {
        return Err("install-old-or-rejected");
    }
    let target = root.join("versions").join(&manifest.revision);
    verify(&target)?;
    state.previous = state.current.take();
    state.current = Some(Installed {
        revision: manifest.revision.clone(),
        sequence: manifest.sequence,
    });
    state.pending = true;
    save(root, &state)
}
pub(crate) fn copy_bundle(bundled: &Path, target: &Path) -> Result<()> {
    let mut count = 0;
    let mut total = 0;
    fn copy(
        from: &Path,
        to: &Path,
        relative: &str,
        count: &mut usize,
        total: &mut u64,
    ) -> Result<()> {
        for entry in fs::read_dir(from).map_err(|_| "install-baseline")? {
            let entry = entry.map_err(|_| "install-baseline")?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "install-baseline")?;
            let logical = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let meta = fs::symlink_metadata(entry.path()).map_err(|_| "install-baseline")?;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("install-reparse");
            }
            if meta.is_dir() {
                if ["resources", "resources/client-data"].contains(&logical.as_str())
                    || logical.starts_with("resources/client-data/")
                {
                    safe_dir(&to.join(entry.file_name()))?;
                    copy(
                        &entry.path(),
                        &to.join(entry.file_name()),
                        &logical,
                        count,
                        total,
                    )?;
                }
            } else if allowed(&logical) {
                *count += 1;
                *total += meta.len();
                if *count > 1024 || *total > 512 * 1024 * 1024 {
                    return Err("install-baseline-limit");
                }
                fs::copy(entry.path(), to.join(entry.file_name()))
                    .map_err(|_| "install-baseline")?;
            }
        }
        Ok(())
    }
    copy(bundled, target, "", &mut count, &mut total)
}
pub fn bootstrap(root: &Path, bundled: &Path) -> Result<()> {
    if read(root)?.current.is_some() {
        return Ok(());
    }
    let installed = bundled_version(bundled)?;
    let revision = installed.revision.as_str();
    let target = root.join("versions").join(revision);
    if !target.exists() {
        safe_dir(&target)?;
        copy_bundle(bundled, &target)?;
    }
    check_bundle(&target)?;
    save(
        root,
        &State {
            current: Some(installed),
            ..Default::default()
        },
    )
}
pub fn bundled_version(bundled: &Path) -> Result<Installed> {
    let build: serde_json::Value = serde_json::from_slice(&update_channel::bounded(
        &bundled.join("build.json"),
        16384,
    )?)
    .map_err(|_| "install-build")?;
    let revision = build["revision"]
        .as_str()
        .filter(|r| update_channel::revision(r))
        .ok_or("install-build")?;
    if build["channel"] != "rust-development-preview" {
        return Err("install-channel");
    }
    let sequence = match build.get("sequence") {
        None => 0,
        Some(v) => v.as_u64().filter(|n| *n > 0).ok_or("install-build")?,
    };
    Ok(Installed {
        revision: revision.into(),
        sequence,
    })
}
/// Uses only the fixed native preview mutex, never enumerates or terminates other applications.
pub fn desktop_running() -> bool {
    unsafe {
        use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::OpenMutexW};
        let n: Vec<u16> = "Local\\FuzeRustDesktopPreview"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let h = OpenMutexW(0x00100000, 0, n.as_ptr());
        if h.is_null() {
            false
        } else {
            CloseHandle(h);
            true
        }
    }
}
pub fn launch(root: &Path, bundled: &Path) -> Result<()> {
    if desktop_running() {
        return Err("install-close-preview-first");
    }
    let _lock = lock(root)?;
    launch_with(root, bundled, &check_bundle, &start)?;
    // A healthy launch retains both rollback versions; maintenance is best effort.
    prune_versions(root, bundled);
    Ok(())
}
fn prune_versions(root: &Path, bundled: &Path) {
    let Ok(state) = read(root) else { return };
    if state.pending {
        return;
    }
    let versions = root.join("versions");
    let protected = fs::canonicalize(bundled).ok();
    for path in crate::cache_maintenance::children(&versions) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !update_channel::revision(name)
            || state.current.as_ref().is_some_and(|v| v.revision == name)
            || state.previous.as_ref().is_some_and(|v| v.revision == name)
            || fs::canonicalize(&path).ok() == protected
        {
            continue;
        }
        if bundled_version(&path).is_ok_and(|v| v.revision == name) {
            let _ = crate::cache_maintenance::retire(&versions, &path, &|s| {
                allowed(s) || ["package.fuz", "package.blockmap"].contains(&s)
            });
        }
    }
}
pub(crate) fn launch_with(
    root: &Path,
    bundled: &Path,
    check: &dyn Fn(&Path) -> Result<()>,
    start: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    let mut state = read(root)?;
    let dir = state
        .current
        .as_ref()
        .map(|v| root.join("versions").join(&v.revision))
        .unwrap_or_else(|| bundled.to_path_buf());
    if state.pending {
        if check(&dir).is_err() {
            if state.previous.is_some() {
                rollback(root)?;
                state = read(root)?;
            } else {
                state.rejected = state.current.take().map(|v| v.revision);
                state.pending = false;
                save(root, &state)?;
            }
        }
    }
    let directory = state
        .current
        .as_ref()
        .map(|v| root.join("versions").join(&v.revision))
        .unwrap_or_else(|| bundled.to_path_buf());
    let launched = start(&directory);
    if launched == Err("install-launch-cancelled") {
        return launched;
    }
    if launched.is_err() {
        if state.previous.is_some() {
            rollback(root)?;
            let old = read(root)?.current.ok_or("rollback-unavailable")?;
            start(&root.join("versions").join(old.revision))?;
        } else {
            return Err("install-launch-readiness");
        }
    } else if state.pending {
        // Do not commit health before the launched UI has acknowledged readiness.
        state.pending = false;
        save(root, &state)?;
    }
    Ok(())
}
fn start(directory: &Path) -> Result<()> {
    start_mode(directory, false)
}
/// Runs the real native window hidden, with live monitoring disabled, and waits for clean exit.
pub(crate) fn start_smoke(directory: &Path) -> Result<()> {
    start_mode(directory, true)
}
fn start_mode(directory: &Path, smoke: bool) -> Result<()> {
    let mut child = Command::new(directory.join("fuze-rust-host.exe"))
        .arg(if smoke {
            "--desktop-smoke"
        } else {
            "--managed-desktop"
        })
        .arg(directory.join("preview.json"))
        .current_dir(directory)
        .env("FUZE_MANAGED_HEALTH_PROTOCOL", "2")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| "install-launch")?;
    let stdout = child.stdout.take().ok_or("install-launch")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        use std::io::BufRead;
        let mut line = String::new();
        let mut input = std::io::BufReader::new(stdout.take(1024));
        let _ = input.read_line(&mut line);
        let ready = if smoke {
            serde_json::from_str::<serde_json::Value>(&line).is_ok_and(|r| {
                r["desktop_ready"] == true
                    && r["pause_resume_exit"] == true
                    && r["game_access"] == false
            })
        } else {
            if line.trim() == "FUZE_DESKTOP_READY_2" {
                line.clear();
                let _ = input.read_line(&mut line);
                if line.trim() == "FUZE_DESKTOP_CLOSED_2" {
                    let _ = tx.send(Err("install-launch-cancelled"));
                    return;
                }
                line.trim() == "FUZE_DESKTOP_HEALTHY_2"
            } else {
                // Existing preview packages predate the two-step health protocol.
                line.trim() == "FUZE_DESKTOP_READY_1"
            }
        };
        let _ = tx.send(if ready {
            Ok(())
        } else {
            Err("install-launch-readiness")
        });
    });
    let acknowledgement = rx
        .recv_timeout(Duration::from_secs(15))
        .unwrap_or(Err("install-launch-readiness"));
    if acknowledgement == Err("install-launch-cancelled") {
        return acknowledgement;
    }
    if acknowledgement.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("install-launch-readiness");
    }
    if smoke {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(s)) if s.success() => break,
                Ok(Some(_)) | Err(_) => return Err("install-smoke-exit"),
                _ if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("install-smoke-timeout");
                }
                _ => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_keeps_current_previous_and_unrecognized_files() {
        let root = std::env::temp_dir().join(format!("fuze-retention-{}", std::process::id()));
        safe_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        for revision in [
            "111111111111",
            "222222222222",
            "333333333333",
            "444444444444",
        ] {
            let dir = root.join("versions").join(revision);
            safe_dir(&dir).unwrap();
            fs::write(dir.join("build.json"), serde_json::json!({"revision":revision,"channel":"rust-development-preview","sequence":1}).to_string()).unwrap();
        }
        fs::write(root.join("versions/444444444444/personal.txt"), b"fixture").unwrap();
        save(
            &root,
            &State {
                current: Some(Installed {
                    revision: "111111111111".into(),
                    sequence: 3,
                }),
                previous: Some(Installed {
                    revision: "222222222222".into(),
                    sequence: 2,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        prune_versions(&root, &root);
        assert!(root.join("versions/111111111111").exists());
        assert!(root.join("versions/222222222222").exists());
        assert!(!root.join("versions/333333333333").exists());
        assert!(root.join("versions/444444444444/personal.txt").exists());
        for revision in ["111111111111", "222222222222", "444444444444"] {
            let dir = root.join("versions").join(revision);
            if revision == "444444444444" {
                fs::remove_file(dir.join("personal.txt")).unwrap();
            }
            crate::cache_maintenance::retire(&root.join("versions"), &dir, &|s| s == "build.json")
                .unwrap();
        }
        fs::remove_file(root.join("current.json")).unwrap();
        fs::remove_dir(root.join("versions")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn bundle_sequence_is_read_before_first_update_and_same_revision_cannot_activate() {
        let root = std::env::temp_dir().join(format!("fuze-version-test-{}", std::process::id()));
        safe_dir(&root).unwrap();
        fs::write(
            root.join("build.json"),
            br#"{"revision":"111111111111","channel":"rust-development-preview","sequence":500}"#,
        )
        .unwrap();
        let version = bundled_version(&root).unwrap();
        assert_eq!(version.sequence, 500);
        save(
            &root,
            &State {
                current: Some(version),
                ..Default::default()
            },
        )
        .unwrap();
        let manifest = Manifest {
            schema: 1,
            channel: "rust-preview".into(),
            revision: "111111111111".into(),
            sequence: 501,
            archive: "Fuze-Rust-111111111111.fuz".into(),
            size: 1,
            sha512: "00".repeat(64),
            blockmap_sha256: "00".repeat(32),
        };
        assert_eq!(
            activate(&root, &manifest, &|_| panic!(
                "same version must not be verified"
            )),
            Err("install-old-or-rejected")
        );
        fs::write(
            root.join("build.json"),
            br#"{"revision":"111111111111","channel":"rust-development-preview","sequence":-1}"#,
        )
        .unwrap();
        assert!(bundled_version(&root).is_err());
        fs::remove_file(root.join("build.json")).unwrap();
        fs::remove_file(root.join("current.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn pointer_commit_failure_and_health_failure_preserve_rollback() {
        let root = std::env::temp_dir().join(format!("fuze-pointer-test-{}", std::process::id()));
        safe_dir(&root).unwrap();
        let initial = State {
            current: Some(Installed {
                revision: "111111111111".into(),
                sequence: 1,
            }),
            ..Default::default()
        };
        save(&root, &initial).unwrap();
        let manifest = Manifest {
            schema: 1,
            channel: "rust-preview".into(),
            revision: "222222222222".into(),
            sequence: 2,
            archive: "Fuze-Rust-222222222222.fuz".into(),
            size: 1,
            sha512: "00".repeat(64),
            blockmap_sha256: "00".repeat(32),
        };
        assert_eq!(
            activate(&root, &manifest, &|_| Err("fixture-health-failed")),
            Err("fixture-health-failed")
        );
        assert_eq!(read(&root).unwrap(), initial);
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(root.join("current.json"))
            .unwrap();
        assert!(activate(&root, &manifest, &|_| Ok(())).is_err());
        drop(locked);
        assert_eq!(read(&root).unwrap(), initial);
        // Failed readiness must switch to the previous version, then start only that version.
        save(&root, &initial).unwrap();
        activate(&root, &manifest, &|_| Ok(())).unwrap();
        let starts = std::cell::RefCell::new(Vec::new());
        launch_with(&root, &root, &|_| Ok(()), &|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            starts.borrow_mut().push(name.clone());
            if name == manifest.revision {
                assert!(
                    read(&root).unwrap().pending,
                    "readiness must precede pointer commit"
                );
                Err("fixture-not-ready")
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(
            *starts.borrow(),
            vec![manifest.revision.clone(), "111111111111".into()]
        );
        assert_eq!(read(&root).unwrap().current, initial.current);
        save(&root, &initial).unwrap();
        activate(&root, &manifest, &|_| Ok(())).unwrap();
        launch_with(&root, &root, &|_| Err("fixture-unhealthy"), &|p| {
            assert_eq!(p.file_name().unwrap(), "111111111111");
            Ok(())
        })
        .unwrap();
        assert_eq!(read(&root).unwrap().current, initial.current);
        save(&root, &initial).unwrap();
        assert_eq!(read(&root).unwrap(), initial);
        activate(&root, &manifest, &|_| Ok(())).unwrap();
        assert!(read(&root).unwrap().pending);
        rollback(&root).unwrap();
        let rolled = read(&root).unwrap();
        assert_eq!(rolled.current, initial.current);
        assert_eq!(rolled.rejected, Some(manifest.revision.clone()));
        assert!(activate(&root, &manifest, &|_| Ok(())).is_err());
        for invalid in [
            "../fuze-rust-host.exe",
            "C:/evil.exe",
            "resources/client-data/../x.json",
            "resources/client-data/CON.json",
            "resources/client-data/a.json:stream",
            "secret.txt",
        ] {
            assert!(!allowed(invalid));
        }
        assert!(allowed(
            "resources/client-data/versions/16.20.1/augments.json"
        ));
        std::fs::remove_file(root.join("current.json")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
