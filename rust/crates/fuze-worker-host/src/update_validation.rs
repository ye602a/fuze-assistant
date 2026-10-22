//! Opt-in isolated acceptance harness. Never uses the player install root or credential store.
use crate::{
    managed_install as install,
    update_channel::{self, Manifest, Session},
    Result,
};
use std::{fs, path::Path};

pub fn lifecycle(root: &Path, baseline: &Path, archive: &Path, manifest: &Manifest) -> Result<()> {
    // A caller must provide a NEW directory; it cannot target an existing installation.
    fs::create_dir(root).map_err(|_| "validation-root-must-be-new")?;
    install::safe_dir(root)?;
    in_place_lifecycle(root, baseline, archive, manifest)?;
    let _lock = install::lock(root)?;
    crate::local_setup::install_with(root, baseline, &install::check_bundle)?;
    install::launch_with(
        root,
        &root.join("no-portable-fallback"),
        &install::check_bundle,
        &install::start_smoke,
    )?;
    if crate::local_setup::install_with(root, baseline, &install::check_bundle)? {
        return Err("validation-setup-idempotence");
    }
    let mut initial = install::read(root)?;
    // CI may use the same built package for both versions; alias only its isolated baseline.
    if initial
        .current
        .as_ref()
        .is_some_and(|v| v.revision == manifest.revision)
    {
        let old = initial.current.as_mut().ok_or("validation-baseline")?;
        fs::rename(
            root.join("versions").join(&old.revision),
            root.join("versions/000000000001"),
        )
        .map_err(|_| "validation-baseline")?;
        old.revision = "000000000001".into();
        old.sequence = manifest.sequence.saturating_sub(1);
        let build_path = root.join("versions/000000000001/build.json");
        let mut build: serde_json::Value =
            serde_json::from_slice(&update_channel::bounded(&build_path, 16384)?)
                .map_err(|_| "validation-baseline")?;
        build["revision"] = old.revision.clone().into();
        build["sequence"] = old.sequence.into();
        // Preserve uiFrontend: aliasing a Slint baseline must never silently select Win32.
        fs::write(
            build_path,
            serde_json::to_vec(&build).map_err(|_| "validation-baseline")?,
        )
        .map_err(|_| "validation-baseline")?;
        install::save(root, &initial)?;
    }
    let target = root.join("versions").join(&manifest.revision);
    install::unpack(archive, &target, &manifest.revision)?;
    install::activate(root, manifest, &install::check_bundle)?;
    install::launch_with(
        root,
        baseline,
        &install::check_bundle,
        &install::start_smoke,
    )?;
    let active = install::read(root)?;
    if active.pending || active.current.as_ref().map(|v| &v.revision) != Some(&manifest.revision) {
        return Err("validation-activation");
    }
    // Repeat launch exercises the committed (not pending) installation.
    install::launch_with(
        root,
        baseline,
        &install::check_bundle,
        &install::start_smoke,
    )?;
    // Actual missing-model failure in the owned installed copy, before startup.
    install::save(root, &initial)?;
    install::activate(root, manifest, &install::check_bundle)?;
    let model = target.join("resources/inference.onnx");
    let held = target.join("resources/inference.held");
    fs::rename(&model, &held).map_err(|_| "validation-fault")?;
    let health = install::launch_with(
        root,
        baseline,
        &install::check_bundle,
        &install::start_smoke,
    );
    fs::rename(&held, &model).map_err(|_| "validation-restore")?;
    health?;
    verify_rollback(root, &initial, manifest)?;
    // Actual CreateProcess failure after health verification, not a mocked callback.
    install::save(root, &initial)?;
    install::activate(root, manifest, &install::check_bundle)?;
    let mut checked = install::read(root)?;
    checked.pending = false;
    install::save(root, &checked)?;
    let executable = install::desktop_program(&target)?;
    let held = executable.with_extension("held");
    fs::rename(&executable, &held).map_err(|_| "validation-fault")?;
    let startup = install::launch_with(
        root,
        baseline,
        &install::check_bundle,
        &install::start_smoke,
    );
    fs::rename(&held, &executable).map_err(|_| "validation-restore")?;
    startup?;
    verify_rollback(root, &initial, manifest)?;
    let menu = root.join("fixture-programs");
    crate::install_entries::create(root, baseline, &menu)?;
    crate::install_entries::create(root, baseline, &menu)?;
    crate::install_entries::uninstall(root, &menu)?;
    if root.join("current.json").exists() || root.join("fuze-launcher.exe").exists() {
        return Err("validation-uninstall");
    }
    Ok(())
}
// This acceptance path runs only in the explicitly requested isolated package check.
// It complements the retained legacy installation/rollback checks below.
fn in_place_lifecycle(
    root: &Path,
    baseline: &Path,
    archive: &Path,
    manifest: &Manifest,
) -> Result<()> {
    let directory = root.join("in-place-application");
    let payload = root.join("in-place-payload");
    install::copy_bundle(baseline, &directory)?;
    let build_path = directory.join("build.json");
    let mut build: serde_json::Value =
        serde_json::from_slice(&update_channel::bounded(&build_path, 16384)?)
            .map_err(|_| "validation-baseline")?;
    build["revision"] = "000000000001".into();
    build["sequence"] = manifest.sequence.saturating_sub(1).into();
    fs::write(
        &build_path,
        serde_json::to_vec(&build).map_err(|_| "validation-baseline")?,
    )
    .map_err(|_| "validation-baseline")?;
    fs::write(directory.join("personal-sentinel.txt"), "preserve")
        .map_err(|_| "validation-baseline")?;
    install::unpack(archive, &payload, &manifest.revision)?;
    install::check_bundle(&payload)?;
    crate::in_place_update::replace(&directory, &payload)?;
    if install::bundled_version(&directory)?.revision != manifest.revision
        || install::application_directory(&directory)? != directory
        || directory.join("versions").exists()
        || directory.join("current.json").exists()
        || fs::read_to_string(directory.join("personal-sentinel.txt"))
            .ok()
            .as_deref()
            != Some("preserve")
    {
        return Err("validation-in-place");
    }
    install::start_smoke(&directory)
}
fn verify_rollback(root: &Path, initial: &install::State, manifest: &Manifest) -> Result<()> {
    let state = install::read(root)?;
    if state.current != initial.current
        || state.pending
        || state.rejected.as_ref() != Some(&manifest.revision)
    {
        return Err("validation-rollback");
    }
    Ok(())
}
pub fn remote(
    session: Session,
    root: &Path,
    baseline: &Path,
    old_archive: &Path,
    old_map: &Path,
) -> Result<serde_json::Value> {
    let (manifest, mut report) = transfer_check(session, root, old_archive, old_map)?;
    lifecycle(
        &root.join("installation"),
        baseline,
        &root.join("delta.fuz"),
        &manifest,
    )?;
    for key in [
        "installed",
        "repeat_launch",
        "health_failure_rollback",
        "process_start_failure_rollback",
        "hidden_native_window",
    ] {
        report[key] = true.into();
    }
    Ok(report)
}

/// Real production transport, but no extraction, process launch, installation or credential store.
pub fn transport(
    session: Session,
    root: &Path,
    old_archive: &Path,
    old_map: &Path,
) -> Result<serde_json::Value> {
    transfer_check(session, root, old_archive, old_map).map(|(_, report)| report)
}

/// Archive and pointer evidence only. Deliberately never calls check_bundle, launch_with,
/// local_setup, registry or shortcut APIs. It cannot establish runtime health or restart success.
pub fn staging(
    root: &Path,
    manifest: &Manifest,
    archive: &Path,
    baseline: &Manifest,
    old_archive: &Path,
) -> Result<serde_json::Value> {
    manifest.validate()?;
    baseline.validate()?;
    if manifest.sequence <= baseline.sequence || manifest.revision == baseline.revision {
        return Err("validation-distinct-upgrade-required");
    }
    // No existing installation (including a previously used validation root) can be modified.
    fs::create_dir(root).map_err(|_| "validation-root-must-be-new")?;
    install::safe_dir(root)?;
    let _lock = install::lock(root)?;
    for (meta, path) in [(baseline, old_archive), (manifest, archive)] {
        let bytes = update_channel::bounded(path, meta.size)?;
        if bytes.len() as u64 != meta.size {
            return Err("update-size");
        }
        if crate::update_io::digest(&bytes, true)? != update_channel::decode_hex(&meta.sha512, 64)?
        {
            return Err("update-final-hash");
        }
    }
    let expected = |meta: &Manifest| install::Installed {
        revision: meta.revision.clone(),
        sequence: meta.sequence,
    };
    // Structural validation only, never substitute this callback for production health checks.
    let inspect = |directory: &Path, meta: &Manifest| {
        if install::bundled_version(directory)? != expected(meta) {
            return Err("validation-build-identity");
        }
        install::desktop_program(directory)?;
        Ok(())
    };
    for (meta, path) in [(baseline, old_archive), (manifest, archive)] {
        let destination = root.join("versions").join(&meta.revision);
        install::unpack(path, &destination, &meta.revision)?;
        inspect(&destination, meta)?;
    }
    let initial = install::State {
        current: Some(expected(baseline)),
        ..Default::default()
    };
    install::save(root, &initial)?;
    let inspect_target = |directory: &Path| inspect(directory, manifest);
    install::activate(root, manifest, &inspect_target)?;
    let pending = install::read(root)?;
    if pending.current != Some(expected(manifest))
        || pending.previous != initial.current
        || !pending.pending
    {
        return Err("validation-activation");
    }
    if install::activate(root, manifest, &inspect_target) != Err("install-validation-pending") {
        return Err("validation-pending-guard");
    }
    install::rollback(root)?;
    verify_rollback(root, &initial, manifest)?;
    if install::activate(root, manifest, &inspect_target) != Err("install-old-or-rejected") {
        return Err("validation-rejected-guard");
    }
    Ok(serde_json::json!({
        "revision": manifest.revision, "baseline_revision": baseline.revision,
        "final_hashes_verified": true, "native_extracted": true, "build_identity_verified": true,
        "pending_pointer_verified": true, "pending_guard": true, "pointer_rollback": true,
        "rejected_guard": true, "installed": false, "process_launch": false,
        "runtime_health_verified": false, "restart_verified": false,
        "registration_changed": false, "game_access": false
    }))
}

/// Two explicit processes: seed a real prefix then reopen it in a later invocation.
/// No archive extraction, application launch or install lifecycle is reachable here.
pub fn resume_transport(session: Session, root: &Path, seed: bool) -> Result<serde_json::Value> {
    use std::{
        cell::{Cell, RefCell},
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    if seed {
        fs::create_dir(root).map_err(|_| "validation-root-must-be-new")?;
    } else if !root.is_dir() {
        return Err("validation-resume-root");
    }
    install::safe_dir(root)?;
    let _lock = install::lock(root)?;
    let marker = root.join("resume-validation.json");
    let prior = if seed {
        None
    } else {
        let manifest: Manifest = serde_json::from_slice(&update_channel::bounded(&marker, 16384)?)
            .map_err(|_| "validation-resume-root")?;
        manifest.validate()?;
        Some(manifest)
    };
    let started = Instant::now();
    let cancel = || started.elapsed() >= Duration::from_secs(600);
    let release = session.check(&cancel)?;
    if release.manifest.size <= 1024 * 1024 {
        return Err("validation-real-range-required");
    }
    let manifest = serde_json::to_vec(&release.manifest).map_err(|_| "validation-manifest")?;
    if let Some(prior) = prior {
        if serde_json::to_vec(&prior).map_err(|_| "validation-manifest")? != manifest {
            return Err("validation-resume-version-changed");
        }
    } else {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&marker)
            .map_err(|_| "validation-resume-root")?;
        file.write_all(&manifest)
            .map_err(|_| "validation-resume-root")?;
    }
    let cache = RefCell::new(crate::update_resume::Cache::open(root, &release.manifest)?);
    let before = cache.borrow().retained_bytes()?;
    if !seed && before == 0 {
        return Err("validation-resume-empty");
    }
    let received = AtomicU64::new(0);
    let prefix = Cell::new(0);
    let attempt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "validation-clock")?
        .as_nanos();
    let result = update_channel::fetch_with(
        &release.manifest,
        &root.join(format!("attempt-{attempt}.fuz")),
        None,
        &[],
        &|_, _| Err("validation-unexpected-delta"),
        &|out| {
            cache.borrow_mut().transfer(
                out,
                &|offset, output| {
                    if seed {
                        let mut bytes = Vec::new();
                        session.download(
                            &release.archive,
                            1024 * 1024,
                            Some((0, 1024 * 1024 - 1, release.manifest.size)),
                            &mut bytes,
                            &cancel,
                        )?;
                        received.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                        output.write_all(&bytes).map_err(|_| "update-write")?;
                        Err("validation-intentional-interruption")
                    } else {
                        session.archive_from_observed(
                            &release.archive,
                            release.manifest.size,
                            offset,
                            output,
                            &cancel,
                            &|bytes| {
                                received.fetch_add(bytes, Ordering::Relaxed);
                            },
                        )
                    }
                },
                &cancel,
                &|value| prefix.set(value),
            )
        },
        &cancel,
    );
    let success = if seed {
        result == Err("validation-intentional-interruption")
    } else {
        result.is_ok()
    };
    let verified = result.is_ok();
    if verified || result == Err("update-final-hash") {
        cache.borrow_mut().clear()?;
    }
    Ok(serde_json::json!({
        "success": success, "seed": seed, "final_hashes_verified": verified,
        "error": if success { None } else { result.err() },
        "package_bytes": release.manifest.size, "retained_before": before,
        "retained_after": cache.borrow().retained_bytes()?, "resume_offset": prefix.get(),
        "received_bytes": received.load(Ordering::Relaxed), "total_ms": started.elapsed().as_millis(),
        "revision": release.manifest.revision, "installed": false, "game_access": false, "credential_persisted": false,
        "public_download": !session.uses_credential(), "private_download": session.uses_credential(),
        "process_launch": false
    }))
}

fn transfer_check(
    session: Session,
    root: &Path,
    old_archive: &Path,
    old_map: &Path,
) -> Result<(Manifest, serde_json::Value)> {
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    struct Received<'a> {
        output: &'a mut Vec<u8>,
        bytes: &'a AtomicU64,
    }
    impl Write for Received<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.output.extend_from_slice(bytes);
            self.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let started = Instant::now();
    let cancel = || started.elapsed() >= Duration::from_secs(600);
    // Fail locally before opening a network connection if the baseline is missing.
    let old_map = update_channel::bounded(old_map, 8 * 1024 * 1024)?;
    let old_bytes = fs::metadata(old_archive)
        .map_err(|_| "validation-baseline")?
        .len();
    if old_bytes == 0 || old_bytes > 512 * 1024 * 1024 {
        return Err("validation-baseline");
    }
    fs::create_dir(root).map_err(|_| "validation-root-must-be-new")?;
    install::safe_dir(root)?;
    let release = session.check(&cancel)?;
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&release.manifest).map_err(|_| "validation-manifest")?,
    )
    .map_err(|_| "validation-manifest")?;
    let map = session.bytes(&release.map, 8 * 1024 * 1024, &cancel)?;
    // Avoid a redundant full transfer when the supplied baseline cannot exercise real reuse.
    let plan = crate::private_update::plan(&old_map, old_bytes, &map, release.manifest.size)?;
    let reused: u64 = plan
        .iter()
        .map(|op| match op {
            crate::private_update::Operation::Copy { length, .. } => *length,
            _ => 0,
        })
        .sum();
    if reused == 0 || reused == release.manifest.size {
        return Err("validation-real-range-required");
    }
    let received = AtomicU64::new(0);
    let requests = AtomicU64::new(0);
    let range = |offset, length| {
        requests.fetch_add(1, Ordering::Relaxed);
        let mut bytes = Vec::new();
        session.download(
            &release.archive,
            length as usize,
            Some((offset, offset + length - 1, release.manifest.size)),
            &mut Received {
                output: &mut bytes,
                bytes: &received,
            },
            &cancel,
        )?;
        Ok(bytes)
    };
    let full = |out: &mut fs::File| {
        session.archive_observed(
            &release.archive,
            release.manifest.size,
            out,
            &cancel,
            &|bytes| {
                received.fetch_add(bytes, Ordering::Relaxed);
            },
        )
    };
    let delta_path = root.join("delta.fuz");
    let delta = update_channel::fetch_with(
        &release.manifest,
        &delta_path,
        Some((old_archive, &old_map)),
        &map,
        &range,
        &full,
        &cancel,
    )?;
    if delta.mode != "delta" || delta.downloaded == 0 || delta.downloaded >= release.manifest.size {
        return Err("validation-real-range-required");
    }
    let delta_received = received.load(Ordering::Relaxed);
    let delta_requests = requests.load(Ordering::Relaxed);
    let delta_ms = started.elapsed().as_millis();
    if std::env::var_os("FUZE_UPDATE_VERIFY_TRACE").is_some() {
        eprintln!(
            "validation-delta bytes={delta_received} requests={delta_requests} ms={delta_ms}"
        );
    }
    let full_path = root.join("full.fuz");
    let fallback = update_channel::fetch_with(
        &release.manifest,
        &full_path,
        Some((old_archive, b"invalid-map")),
        &map,
        &range,
        &full,
        &cancel,
    )?;
    if !fallback.fallback || fallback.mode != "full" {
        return Err("validation-full-fallback");
    }
    let report = serde_json::json!({
        "revision":release.manifest.revision, "package_bytes":release.manifest.size,
        "private_download":session.uses_credential(), "public_download":!session.uses_credential(),
        "real_range_bytes":delta.downloaded, "full_bytes":fallback.downloaded,
        "delta_received_bytes":delta_received, "fallback_received_bytes":received.load(Ordering::Relaxed)-delta_received,
        "delta_range_requests":delta_requests, "delta_ms":delta_ms, "total_ms":started.elapsed().as_millis(),
        "final_hashes_verified":true, "full_fallback":true, "installed":false,
        "game_access":false, "credential_persisted":false, "process_launch":false
    });
    Ok((release.manifest, report))
}
