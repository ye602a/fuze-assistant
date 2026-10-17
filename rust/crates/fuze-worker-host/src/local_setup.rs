//! Explicit per-user preview installation. No registry, shortcuts, credentials or game access.
use crate::{managed_install as install, Result};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const START: &[u8] =
    b"@echo off\r\ncd /d \"%~dp0\"\r\nstart \"\" fuze-launcher.exe --launch\r\n";

/// Caller holds the installation lock. Verification runs only against a copied bundle.
pub fn install_with(
    root: &Path,
    source: &Path,
    verify: &dyn Fn(&Path) -> Result<()>,
) -> Result<bool> {
    install::safe_dir(root)?;
    if !source.is_dir() {
        return Err("setup-source");
    }
    for p in source.ancestors() {
        use std::os::windows::fs::MetadataExt;
        if fs::symlink_metadata(p)
            .map_err(|_| "setup-source")?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err("install-reparse");
        }
    }
    let version = install::bundled_version(source)?;
    let mut state = install::read(root)?;
    if state.pending {
        return Err("install-validation-pending");
    }
    if state.current.as_ref() == Some(&version) {
        launcher(root, &root.join("versions").join(&version.revision))?;
        return Ok(false);
    }
    if state
        .current
        .as_ref()
        .is_some_and(|v| version.sequence <= v.sequence)
        || state.rejected.as_ref() == Some(&version.revision)
    {
        return Err("install-old-or-rejected");
    }
    let versions = root.join("versions");
    install::safe_dir(&versions)?;
    let target = versions.join(&version.revision);
    if target.exists() {
        return Err("setup-version-exists");
    }
    let stage = root.join(format!(
        "setup-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "install-clock")?
            .as_nanos()
    ));
    fs::create_dir(&stage).map_err(|_| "setup-stage")?;
    // Incomplete stages are preserved for diagnosis; no recursive deletion of user directories.
    install::copy_bundle(source, &stage)?;
    if install::bundled_version(&stage)? != version {
        return Err("setup-source-changed");
    }
    verify(&stage)?;
    // Existing entry points are never overwritten. Fail before changing the active version.
    launcher(root, &stage)?;
    fs::rename(&stage, &target).map_err(|_| "setup-version")?;
    state.previous = state.current.take();
    state.current = Some(version);
    state.pending = true;
    install::save(root, &state)?;
    Ok(true)
}

fn launcher(root: &Path, bundle: &Path) -> Result<()> {
    let entry = root.join("Start-Fuze-Rust.cmd");
    if entry.exists() {
        regular(&entry)?;
        if crate::update_channel::bounded(&entry, 512)? != START {
            return Err("setup-entry-conflict");
        }
    }
    let exe = root.join("fuze-launcher.exe");
    if exe.exists() {
        regular(&exe)?;
        regular(&root.join("launcher.sha256"))?;
        // A receipt identifies only our own previously installed launcher. No publisher claim.
        let expected = crate::update_channel::bounded(&root.join("launcher.sha256"), 128)?;
        let bytes = crate::update_channel::bounded(&exe, 32 * 1024 * 1024)?;
        if expected != crate::update_io::digest(&bytes, false)? {
            return Err("setup-launcher-conflict");
        }
    } else {
        let source = bundle.join("fuze-update.exe");
        regular(&source)?;
        let bytes = crate::update_channel::bounded(&source, 32 * 1024 * 1024)?;
        let digest = crate::update_io::digest(&bytes, false)?;
        // Write both with create_new; a partial installation remains explicit and recoverable.
        write_new(&root.join("launcher.sha256"), &digest)?;
        write_new(&exe, &bytes)?;
    }
    if !entry.exists() {
        write_new(&entry, START)?;
    }
    Ok(())
}
pub(crate) fn regular(path: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    let m = fs::symlink_metadata(path).map_err(|_| "setup-file")?;
    if !m.is_file() || m.file_attributes() & 0x400 != 0 {
        return Err("install-reparse");
    }
    Ok(())
}
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "setup-create")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "setup-write")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_install_preserves_old_state_and_never_copies_private_files() {
        let base = std::env::temp_dir().join(format!(
            "fuze-setup-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&base).unwrap();
        let source = base.join("source");
        fs::create_dir(&source).unwrap();
        let root = base.join("install");
        let build = |revision: &str, sequence: u64| {
            fs::write(source.join("build.json"),serde_json::json!({"revision":revision,"sequence":sequence,"channel":"rust-development-preview"}).to_string()).unwrap()
        };
        build("111111111111", 1);
        fs::write(
            source.join("fuze-update.exe"),
            b"fixture-launcher-not-executed",
        )
        .unwrap();
        fs::write(source.join("preferences.json"), b"private-fixture").unwrap();
        assert_eq!(
            install_with(&root, &source, &|_| Err("fixture-check")),
            Err("fixture-check")
        );
        assert_eq!(install::read(&root).unwrap(), install::State::default());
        assert!(install_with(&root, &source, &|_| Ok(())).unwrap());
        assert!(!root.join("versions/111111111111/preferences.json").exists());
        assert_eq!(
            install_with(&root, &source, &|_| Ok(())),
            Err("install-validation-pending")
        );
        let mut state = install::read(&root).unwrap();
        state.pending = false;
        install::save(&root, &state).unwrap();
        assert!(!install_with(&root, &source, &|_| panic!(
            "same version should not repeat OCR check"
        ))
        .unwrap());
        build("222222222222", 2);
        assert!(install_with(&root, &source, &|_| Ok(())).unwrap());
        assert_eq!(install::read(&root).unwrap().previous, state.current);
        install::rollback(&root).unwrap();
        assert_eq!(install::read(&root).unwrap().current, state.current);
        build("333333333333", 3);
        fs::write(root.join("Start-Fuze-Rust.cmd"), b"user-owned").unwrap();
        let before = install::read(&root).unwrap();
        assert_eq!(
            install_with(&root, &source, &|_| Ok(())),
            Err("setup-entry-conflict")
        );
        assert_eq!(install::read(&root).unwrap(), before);
        assert_eq!(
            fs::read(root.join("Start-Fuze-Rust.cmd")).unwrap(),
            b"user-owned"
        );
        assert!(
            base.starts_with(std::env::temp_dir())
                && base
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("fuze-setup-test-")
        );
        fs::remove_dir_all(base).unwrap();
    }
}
