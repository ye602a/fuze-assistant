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
    let _lock = install::lock(root)?;
    install::bootstrap(root, baseline)?;
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
    let executable = target.join("fuze-rust-host.exe");
    let held = target.join("fuze-rust-host.held");
    fs::rename(&executable, &held).map_err(|_| "validation-fault")?;
    let startup = install::launch_with(
        root,
        baseline,
        &install::check_bundle,
        &install::start_smoke,
    );
    fs::rename(&held, &executable).map_err(|_| "validation-restore")?;
    startup?;
    verify_rollback(root, &initial, manifest)
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
    fs::create_dir(root).map_err(|_| "validation-root-must-be-new")?;
    install::safe_dir(root)?;
    let release = session.check(&|| false)?;
    let map = session.bytes(&release.map, 8 * 1024 * 1024, &|| false)?;
    let old_map = update_channel::bounded(old_map, 8 * 1024 * 1024)?;
    let range = |offset, length| {
        let mut bytes = Vec::new();
        session.download(
            &release.archive,
            length as usize,
            Some((offset, offset + length - 1, release.manifest.size)),
            &mut bytes,
            &|| false,
        )?;
        Ok(bytes)
    };
    let full = |out: &mut fs::File| {
        session.download(
            &release.archive,
            release.manifest.size as usize,
            None,
            out,
            &|| false,
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
        &|| false,
    )?;
    if delta.mode != "delta" || delta.downloaded == 0 || delta.downloaded >= release.manifest.size {
        return Err("validation-real-range-required");
    }
    let full_path = root.join("full.fuz");
    let fallback = update_channel::fetch_with(
        &release.manifest,
        &full_path,
        Some((old_archive, b"invalid-map")),
        &map,
        &range,
        &full,
        &|| false,
    )?;
    if !fallback.fallback || fallback.mode != "full" {
        return Err("validation-full-fallback");
    }
    lifecycle(
        &root.join("installation"),
        baseline,
        &delta_path,
        &release.manifest,
    )?;
    Ok(serde_json::json!({
        "private_download":true, "real_range_bytes":delta.downloaded, "full_bytes":fallback.downloaded,
        "full_fallback":true, "installed":true, "repeat_launch":true,
        "health_failure_rollback":true, "process_start_failure_rollback":true,
        "hidden_native_window":true, "game_access":false, "credential_persisted":false
    }))
}
