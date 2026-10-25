//! Updates replace the selected application directory; no version copies or rollback pointer.
use crate::{managed_install as install, Result};
use std::{
    fs::{self, OpenOptions},
    os::windows::{fs::OpenOptionsExt, process::CommandExt},
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// The updater must run outside the application so its own EXE/DLLs can be replaced.
/// Launch the GUI executable directly; no shell participates in the update lifecycle.
pub fn handoff(private: bool, confirmed: bool) -> Result<()> {
    let executable = std::env::current_exe().map_err(|_| "update-location")?;
    launch_helper(&executable, private, confirmed, None).map(|_| ())
}

/// Run the existing updater outside the install directory while the desktop keeps running.
pub fn launch_background(
    directory: &Path,
    signal: &str,
    revision: &str,
) -> Result<std::process::Child> {
    launch_helper(
        &directory.join("fuze-update.exe"),
        false,
        true,
        Some((signal, revision)),
    )
}
fn launch_helper(
    executable: &Path,
    private: bool,
    confirmed: bool,
    background: Option<(&str, &str)>,
) -> Result<std::process::Child> {
    let directory = install::application_directory(executable.parent().ok_or("update-location")?)?;
    let temporary = std::env::temp_dir().join(format!(
        "fuze-update-helper-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "install-clock")?
            .as_nanos()
    ));
    fs::create_dir(&temporary).map_err(|_| "update-helper-directory")?;
    let result = (|| {
        let helper = temporary.join("fuze-update.exe");
        fs::copy(&executable, &helper).map_err(|_| "update-helper-copy")?;
        crate::runtime_prerequisites::supply_app_local(&temporary, &directory)?;
        let mut command = Command::new(&helper);
        command
            .arg(if private {
                "--update-directory-private"
            } else {
                "--update-directory"
            })
            .arg(&directory);
        if confirmed {
            command.arg("--confirmed");
        }
        if let Some((signal, revision)) = background {
            command
                .env("FUZE_UPDATE_READY", signal)
                .env("FUZE_UPDATE_PARENT", std::process::id().to_string())
                .env("FUZE_UPDATE_REVISION", revision);
        }
        command
            .current_dir(&temporary)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| "update-helper-start")
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

/// The newly installed EXE waits for the temporary GUI to exit, removes that exact helper,
/// after the new client is healthy. This avoids deleting a running Windows image.
pub fn finish_update(directory: &Path) -> Result<()> {
    let helper = std::env::current_exe().map_err(|_| "update-location")?;
    let temporary = helper.parent().ok_or("update-location")?;
    checked_helper_directory(temporary)?;
    Command::new(directory.join("fuze-update.exe"))
        .arg("--cleanup-update")
        .arg(temporary)
        .arg(std::process::id().to_string())
        .env_remove("FUZE_UPDATE_READY")
        .env_remove("FUZE_UPDATE_PARENT")
        .env_remove("FUZE_UPDATE_REVISION")
        .env_remove("FUZE_DESKTOP_SUPERVISOR")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| "update-helper-start")?;
    Ok(())
}
fn checked_helper_directory(path: &Path) -> Result<std::path::PathBuf> {
    let root = fs::canonicalize(std::env::temp_dir()).map_err(|_| "update-helper-directory")?;
    let resolved = fs::canonicalize(path).map_err(|_| "update-helper-directory")?;
    if resolved.parent() != Some(root.as_path())
        || !resolved
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("fuze-update-helper-"))
    {
        return Err("update-helper-directory");
    }
    install::safe_dir(&resolved)?;
    Ok(resolved)
}
pub fn cleanup_after_update(temporary: &Path, pid: u32) -> Result<()> {
    use windows_sys::Win32::{Foundation::*, System::Threading::*};
    let temporary = checked_helper_directory(temporary)?;
    if pid == 0 || pid == std::process::id() {
        return Err("update-parent");
    }
    let expected = fs::canonicalize(temporary.join("fuze-update.exe"))
        .map_err(|_| "update-helper-directory")?;
    unsafe {
        let process = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if !process.is_null() {
            let mut image = [0u16; 32768];
            let mut size = image.len() as u32;
            let matches = QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut size)
                != 0
                && fs::canonicalize(std::path::PathBuf::from(String::from_utf16_lossy(
                    &image[..size as usize],
                )))
                .ok()
                    == Some(expected);
            // An exited process may no longer expose its image name. No process action is
            // needed in that case; cleanup still targets the validated helper directory.
            let exited = WaitForSingleObject(process, 0) == WAIT_OBJECT_0
                || (matches && WaitForSingleObject(process, 30000) == WAIT_OBJECT_0);
            CloseHandle(process);
            if !exited {
                return Err("update-parent-timeout");
            }
        } else if GetLastError() != ERROR_INVALID_PARAMETER {
            return Err("update-parent");
        }
    }
    fs::remove_dir_all(temporary).map_err(|_| "update-helper-cleanup")
}

/// A payload checked by the downloader is copied directly over the current files.
/// Check locked files first; write build.json last so an incomplete copy is not announced as new.
pub fn replace(directory: &Path, payload: &Path) -> Result<()> {
    let current = install::bundled_version(directory)?;
    let next = install::bundled_version(payload)?;
    if next.sequence <= current.sequence || next.revision == current.revision {
        return Err("install-old-or-rejected");
    }
    let mut files = Vec::new();
    collect(payload, payload, &mut files)?;
    files.sort_by_key(|p| p == Path::new("build.json"));
    for relative in &files {
        let target = directory.join(
            if relative == Path::new("fuze-ui-lab.exe") && directory.join("FUZE.exe").exists() {
                Path::new("FUZE.exe")
            } else {
                relative
            },
        );
        install::safe_dir(target.parent().ok_or("install-path")?)?;
        if target.exists() {
            crate::local_setup::regular(&target)?;
            OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&target)
                .map_err(|_| "update-file-in-use")?;
        }
    }
    crate::update_progress::begin_files(files.len());
    for relative in files {
        crate::update_progress::file_started(&relative);
        fs::copy(payload.join(&relative), directory.join(&relative))
            .map_err(|_| "update-replace-file")?;
        crate::update_progress::file_finished(&relative);
    }
    install::finalize_player_layout(directory)?;
    Ok(())
}

fn collect(root: &Path, directory: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<()> {
    install::safe_dir(directory)?;
    for entry in fs::read_dir(directory).map_err(|_| "install-read")? {
        let entry = entry.map_err(|_| "install-read")?;
        let path = entry.path();
        if entry.file_type().map_err(|_| "install-read")?.is_dir() {
            collect(root, &path, files)?;
        } else {
            crate::local_setup::regular(&path)?;
            let relative = path.strip_prefix(root).map_err(|_| "install-path")?;
            let logical = relative.to_string_lossy().replace('\\', "/");
            if !install::allowed(&logical)
                && !["package.fuz", "package.blockmap"].contains(&logical.as_str())
            {
                return Err("install-entry");
            }
            files.push(relative.to_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle(path: &Path, revision: &str, sequence: u64) {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("build.json"), serde_json::json!({"revision":revision,"sequence":sequence,"channel":"rust-development-preview"}).to_string()).unwrap();
        fs::write(path.join("fuze-update.exe"), revision).unwrap();
        fs::write(path.join("fuze-ui-lab.exe"), revision).unwrap();
    }
    #[test]
    fn occupied_program_aborts_before_changing_any_files() {
        let root =
            std::env::temp_dir().join(format!("fuze-occupied-update-{}", std::process::id()));
        let current = root.join("current");
        let payload = root.join("payload");
        bundle(&current, "111111111111", 1);
        bundle(&payload, "222222222222", 2);
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(current.join("fuze-ui-lab.exe"))
            .unwrap();
        assert_eq!(replace(&current, &payload), Err("update-file-in-use"));
        assert_eq!(
            install::bundled_version(&current).unwrap().revision,
            "111111111111"
        );
        drop(held);
        assert_eq!(
            fs::read_to_string(current.join("fuze-ui-lab.exe")).unwrap(),
            "111111111111"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn replacement_keeps_the_launch_directory_without_backups_or_user_file_changes() {
        let root = std::env::temp_dir().join(format!("fuze-in-place-{}", std::process::id()));
        let directory = root.join("application");
        let payload = root.join("download");
        bundle(&directory, "111111111111", 1);
        bundle(&payload, "222222222222", 2);
        // An unrelated/stale managed pointer must never redirect this bundle's shortcut.
        fs::write(directory.join("current.json"), "invalid old pointer").unwrap();
        assert_eq!(
            install::application_directory(&directory).unwrap(),
            directory
        );
        fs::remove_file(directory.join("current.json")).unwrap();
        fs::write(directory.join("preferences.json"), "user settings").unwrap();
        fs::write(directory.join("unins000.exe"), "existing uninstaller").unwrap();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(directory.join("fuze-ui-lab.exe"))
            .unwrap();
        assert_eq!(replace(&directory, &payload), Err("update-file-in-use"));
        assert_eq!(
            install::bundled_version(&directory).unwrap().revision,
            "111111111111"
        );
        assert_eq!(
            fs::read_to_string(directory.join("fuze-update.exe")).unwrap(),
            "111111111111"
        );
        drop(locked);
        replace(&directory, &payload).unwrap();
        assert_eq!(
            install::bundled_version(&directory).unwrap().revision,
            "222222222222"
        );
        assert_eq!(
            fs::read_to_string(directory.join("fuze-ui-lab.exe")).unwrap(),
            "222222222222"
        );
        assert_eq!(
            fs::read_to_string(directory.join("preferences.json")).unwrap(),
            "user settings"
        );
        assert_eq!(
            fs::read_to_string(directory.join("unins000.exe")).unwrap(),
            "existing uninstaller"
        );
        assert!(!directory.join("versions").exists());
        assert!(!directory.join("current.json").exists());
        assert_eq!(
            replace(&directory, &payload),
            Err("install-old-or-rejected")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_helper_receives_unicode_arguments_without_a_shell() {
        let directory =
            std::env::temp_dir().join(format!("fuze-native-符择's-$-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("build.json"),
            r#"{"revision":"0123456789ab","sequence":1,"channel":"rust-development-preview"}"#,
        )
        .unwrap();
        let source = directory.join("stub.rs");
        fs::write(&source, r#"fn main() {
            let args: Vec<_> = std::env::args().skip(1).collect();
            assert_eq!(args.len(),3); assert_eq!(args[0],"--update-directory"); assert_eq!(args[2],"--confirmed");
            let root = std::path::PathBuf::from(&args[1]);
            std::fs::write(root.join("helper-path"), std::env::current_exe().unwrap().to_string_lossy().as_bytes()).unwrap();
        }"#).unwrap();
        assert!(Command::new("rustc")
            .arg(&source)
            .args(["--crate-name", "fuze_stub", "-o"])
            .arg(directory.join("fuze-update.exe"))
            .creation_flags(0x08000000)
            .status()
            .unwrap()
            .success());
        let mut child = launch_background(&directory, "fixture-signal", "0123456789ab").unwrap();
        assert!(child.wait().unwrap().success());
        let helper =
            std::path::PathBuf::from(fs::read_to_string(directory.join("helper-path")).unwrap());
        assert_ne!(helper.parent(), Some(directory.as_path()));
        assert!(!helper.with_file_name("run.ps1").exists());
        cleanup_after_update(helper.parent().unwrap(), child.id()).unwrap();
        assert!(!helper.exists());
        assert!(directory.join("fuze-update.exe").exists());
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn native_cleanup_rejects_non_helper_directories() {
        let path = std::env::temp_dir().join(format!("fuze-keep-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "untouched").unwrap();
        assert!(cleanup_after_update(&path, u32::MAX).is_err());
        assert!(path.join("keep").exists());
        fs::remove_dir_all(path).unwrap();
    }
}
