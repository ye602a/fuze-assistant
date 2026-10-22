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
/// A temporary PowerShell runner waits for this EXE to exit, then runs and removes the helper.
pub fn handoff(private: bool) -> Result<()> {
    let executable = std::env::current_exe().map_err(|_| "update-location")?;
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
        let runner = temporary.join("run.ps1");
        let script = runner_script(&directory, private, std::process::id());
        // Windows PowerShell needs a BOM to preserve non-ASCII installation paths.
        let bytes: Vec<u8> = std::iter::once(0xfeffu16)
            .chain(script.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        fs::write(&runner, bytes).map_err(|_| "update-helper-copy")?;
        Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&runner)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| "update-helper-start")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn runner_script(directory: &Path, private: bool, parent: u32) -> String {
    let target = directory.to_string_lossy().replace('\'', "''");
    let action = if private {
        "--update-directory-private"
    } else {
        "--update-directory"
    };
    format!(
        r#"$ErrorActionPreference = 'Stop'
$helperDirectory = $PSScriptRoot
try {{
    Wait-Process -Id {parent} -ErrorAction SilentlyContinue
    $worker = Start-Process -FilePath (Join-Path $helperDirectory 'fuze-update.exe') -ArgumentList '"{action}" "{target}"' -WindowStyle Hidden -Wait -PassThru
}} finally {{
    # This exact fresh helper directory was created by the updater, under the system temp folder.
    $resolved = [IO.Path]::GetFullPath($helperDirectory)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved).StartsWith('fuze-update-helper-')) {{
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }}
}}
"#
    )
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
        let target = directory.join(relative);
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
    for relative in files {
        fs::copy(payload.join(&relative), directory.join(&relative))
            .map_err(|_| "update-replace-file")?;
    }
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
    fn helper_runner_cleans_up_after_worker_failure_in_a_unicode_path() {
        let temporary = std::env::temp_dir().join(format!(
            "fuze-update-helper-符择's-$-{}",
            std::process::id()
        ));
        fs::create_dir(&temporary).unwrap();
        // This helper is the inert Rust test harness, never FUZE. It rejects the worker
        // flags without running tests, exercising cleanup after a non-zero worker exit.
        fs::copy(
            std::env::current_exe().unwrap(),
            temporary.join("fuze-update.exe"),
        )
        .unwrap();
        let directory = temporary.join("符择 user's $folder");
        fs::write(temporary.with_extension("keep"), "outside helper").unwrap();
        let script = runner_script(&directory, false, i32::MAX as u32);
        let bytes: Vec<u8> = std::iter::once(0xfeffu16)
            .chain(script.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        fs::write(temporary.join("run.ps1"), bytes).unwrap();
        let result = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(temporary.join("run.ps1"))
            .creation_flags(0x08000000)
            .status()
            .unwrap();
        assert!(result.success());
        assert!(!temporary.exists());
        assert_eq!(
            fs::read_to_string(temporary.with_extension("keep")).unwrap(),
            "outside helper"
        );
        fs::remove_file(temporary.with_extension("keep")).unwrap();
    }
}
