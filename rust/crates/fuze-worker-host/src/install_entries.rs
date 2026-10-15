//! Explicit install-time shell integration; never called by capture or OCR.
use crate::{
    local_setup::{regular, write_new},
    managed_install as install, Result,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use windows::{
    core::{Interface, HSTRING},
    Win32::{System::Com::*, UI::Shell::*},
};
const LINKS: [(&str, &str, &str); 2] = [
    ("Fuze Rust Preview.lnk", "fuze-launcher.exe", "--launch"),
    (
        "Uninstall Fuze Rust Preview.lnk",
        "fuze-maintenance.exe",
        "--uninstall-local",
    ),
];
fn shell_path(path: &Path) -> Result<HSTRING> {
    use std::os::windows::ffi::OsStrExt;
    let value: Vec<u16> = path.as_os_str().encode_wide().collect();
    let value = if value.starts_with(&[92, 92, 63, 92]) {
        &value[4..]
    } else {
        &value[..]
    };
    // Shell Link does not accept verbatim paths. Managed installs are local drive paths.
    if value.len() < 3 || value[1] != 58 || value[2] != 92 || value.len() >= 260 {
        return Err("shortcut-local-path");
    }
    Ok(HSTRING::from_wide(value))
}
struct Com;
impl Com {
    fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|_| "shortcut-com")?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}
pub fn programs() -> Result<PathBuf> {
    let _com = Com::new()?;
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None)
            .map_err(|_| "shortcut-folder")?;
        let value = p.to_string().map_err(|_| "shortcut-folder");
        CoTaskMemFree(Some(p.0 as _));
        Ok(PathBuf::from(value?))
    }
}
fn receipt(root: &Path, name: &str) -> PathBuf {
    root.join(format!("{name}.sha256"))
}
fn owned(root: &Path, path: &Path, name: &str) -> Result<()> {
    regular(path)?;
    let r = receipt(root, name);
    regular(&r)?;
    let expected = crate::update_channel::bounded(&r, 32)?;
    let bytes = crate::update_channel::bounded(path, 32 * 1024 * 1024)?;
    if expected != crate::update_io::digest(&bytes, false)? {
        return Err("setup-entry-conflict");
    }
    Ok(())
}
/// Caller holds the managed installation lock. Tests pass a temporary Programs folder.
pub fn create(root: &Path, bundle: &Path, folder: &Path) -> Result<()> {
    install::safe_dir(root)?;
    install::safe_dir(folder)?;
    let maintenance = root.join("fuze-maintenance.exe");
    if maintenance.exists() {
        owned(root, &maintenance, "maintenance")?;
    } else {
        let source = bundle.join("fuze-update.exe");
        regular(&source)?;
        let bytes = crate::update_channel::bounded(&source, 32 * 1024 * 1024)?;
        write_new(
            &receipt(root, "maintenance"),
            &crate::update_io::digest(&bytes, false)?,
        )?;
        write_new(&maintenance, &bytes)?;
    }
    let _com = Com::new()?;
    for (name, target, args) in LINKS {
        let destination = folder.join(name);
        if destination.exists() {
            owned(root, &destination, name)?;
            continue;
        }
        // COM writes only a new temporary sibling; rename never replaces a user's shortcut.
        let temporary = folder.join(format!("{name}.{}.tmp", std::process::id()));
        write_new(&temporary, b"")?;
        let result = (|| unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| "shortcut-create")?;
            link.SetPath(&shell_path(&root.join(target))?)
                .map_err(|_| "shortcut-target")?;
            link.SetArguments(&HSTRING::from(args))
                .map_err(|_| "shortcut-arguments")?;
            link.SetWorkingDirectory(&shell_path(root)?)
                .map_err(|_| "shortcut-directory")?;
            let persist: IPersistFile = link.cast().map_err(|_| "shortcut-persist")?;
            persist
                .Save(&shell_path(&temporary)?, true)
                .map_err(|_| "shortcut-save")?;
            let bytes = crate::update_channel::bounded(&temporary, 65536)?;
            write_new(
                &receipt(root, name),
                &crate::update_io::digest(&bytes, false)?,
            )?;
            fs::rename(&temporary, &destination).map_err(|_| "shortcut-rename")
        })();
        let _ = fs::remove_file(&temporary);
        result?;
    }
    Ok(())
}
/// Caller is outside the install directory, holds its lock, and checked the desktop mutex.
/// Preflight every version and owned entry before deleting anything. User data is a sibling.
pub fn uninstall(root: &Path, folder: &Path) -> Result<usize> {
    install::safe_dir(root)?;
    install::safe_dir(folder)?;
    let root = fs::canonicalize(root).map_err(|_| "uninstall-root")?;
    let versions = root.join("versions");
    let mut packages = Vec::new();
    if versions.exists() {
        install::safe_dir(&versions)?;
        for item in fs::read_dir(&versions)
            .map_err(|_| "uninstall-read")?
            .take(66)
        {
            let path = item.map_err(|_| "uninstall-read")?.path();
            let version = install::bundled_version(&path)?;
            if path.file_name().and_then(|s| s.to_str()) != Some(&version.revision) {
                return Err("uninstall-unknown-version");
            }
            crate::cache_maintenance::inspect(&versions, &path, &|s| {
                install::allowed(s) || ["package.fuz", "package.blockmap"].contains(&s)
            })?;
            packages.push(path);
        }
        if packages.len() > 64 {
            return Err("uninstall-limit");
        }
    }
    let mut files = Vec::new();
    for (name, _, _) in LINKS {
        let link = folder.join(name);
        if link.exists() {
            owned(&root, &link, name)?;
            files.push(link);
        }
        let r = receipt(&root, name);
        if r.exists() {
            regular(&r)?;
            files.push(r);
        }
    }
    for (exe, r) in [
        ("fuze-launcher.exe", "launcher"),
        ("fuze-maintenance.exe", "maintenance"),
    ] {
        let path = root.join(exe);
        if path.exists() {
            owned(&root, &path, r)?;
            files.push(path);
        }
        let path = receipt(&root, r);
        if path.exists() {
            regular(&path)?;
            files.push(path);
        }
    }
    let entry = root.join("Start-Fuze-Rust.cmd");
    if entry.exists() {
        regular(&entry)?;
        if crate::update_channel::bounded(&entry, 512)? != crate::local_setup::START {
            return Err("setup-entry-conflict");
        }
        files.push(entry);
    }
    install::read(&root)?;
    let current = root.join("current.json");
    if current.exists() {
        regular(&current)?;
        files.push(current);
    }
    for path in &packages {
        crate::cache_maintenance::retire(&versions, path, &|s| {
            install::allowed(s) || ["package.fuz", "package.blockmap"].contains(&s)
        })?;
    }
    for path in files {
        regular(&path)?;
        fs::remove_file(path).map_err(|_| "uninstall-file-in-use")?;
    }
    if versions.exists() {
        fs::remove_dir(&versions).map_err(|_| "uninstall-directory")?;
    }
    // update.lock, unknown/staging files and the parent containing private data are retained.
    Ok(packages.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_links_and_uninstall_preserve_private_and_unknown_files() {
        let root = std::env::temp_dir().join(format!(
            "fuze-entries-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("build.json"),
            br#"{"revision":"111111111111","sequence":1,"channel":"rust-development-preview"}"#,
        )
        .unwrap();
        fs::write(source.join("fuze-update.exe"), b"not-executed").unwrap();
        fs::write(root.join("private-records.json"), b"private-fixture").unwrap();
        let install = root.join("releases");
        let programs = root.join("programs");
        crate::local_setup::install_with(&install, &source, &|_| Ok(())).unwrap();
        let install = fs::canonicalize(install).unwrap();
        create(&install, &source, &programs).unwrap();
        create(&install, &source, &programs).unwrap();
        {
            let _com = Com::new().unwrap();
            unsafe {
                let link: IShellLinkW =
                    CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
                let file: IPersistFile = link.cast().unwrap();
                file.Load(
                    &HSTRING::from(programs.join(LINKS[0].0).as_os_str()),
                    STGM_READ,
                )
                .unwrap();
                let mut target = [0u16; 32768];
                link.GetPath(&mut target, std::ptr::null_mut(), 0).unwrap();
                let path = PathBuf::from(String::from_utf16_lossy(
                    &target[..target.iter().position(|c| *c == 0).unwrap()],
                ));
                assert_eq!(
                    fs::canonicalize(path).unwrap(),
                    fs::canonicalize(install.join("fuze-launcher.exe")).unwrap()
                );
            }
        }
        let link = programs.join(LINKS[0].0);
        let original = fs::read(&link).unwrap();
        fs::write(&link, b"user-modified-shortcut").unwrap();
        assert_eq!(uninstall(&install, &programs), Err("setup-entry-conflict"));
        assert!(install.join("current.json").exists());
        fs::write(link, original).unwrap();
        let unknown = install.join("versions/111111111111/personal.txt");
        fs::write(&unknown, b"keep").unwrap();
        assert_eq!(uninstall(&install, &programs), Err("cleanup-unknown-file"));
        assert!(install.join("current.json").exists());
        fs::remove_file(unknown).unwrap();
        assert_eq!(uninstall(&install, &programs).unwrap(), 1);
        assert!(root.join("private-records.json").exists());
        assert!(!programs.join(LINKS[0].0).exists());
        assert!(!install.join("fuze-launcher.exe").exists());
        assert!(
            root.starts_with(std::env::temp_dir())
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("fuze-entries-")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
