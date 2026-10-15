//! Explicit, pause-only downloads into an isolated cache. Stage, validate, then atomically point.
use crate::{update_io, Result};
use fuze_data::{Locale, Root, Scope, Source};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
fn numeric(v: &str) -> bool {
    v.len() < 64
        && !v.is_empty()
        && v.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
fn pointer(locale: Locale) -> String {
    if locale == Locale::ZhCn {
        "current.json".into()
    } else {
        format!("current.{}.json", locale.code())
    }
}
fn prefix(locale: Locale, version: &str) -> String {
    if locale == Locale::ZhCn {
        format!("versions/{version}")
    } else {
        format!("versions/{}/{version}", locale.code())
    }
}
fn api(locale: Locale, version: &str) -> String {
    if locale == Locale::ZhCn {
        format!("/api/client/v1/data/{version}")
    } else {
        format!("/api/client/v1/data/{version}/{}", locale.code())
    }
}
fn clean_dir(path: &Path) -> Result<()> {
    // Reject reparse ancestors before creating any file in the user-owned cache.
    for p in path.ancestors() {
        if let Ok(m) = fs::symlink_metadata(p) {
            if m.file_attributes() & 0x400 != 0 {
                return Err("data-update-reparse");
            }
        }
    }
    fs::create_dir_all(path).map_err(|_| "data-update-directory")
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    clean_dir(path.parent().ok_or("data-update-path")?)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "data-update-write")?;
    f.write_all(bytes).map_err(|_| "data-update-write")?;
    f.sync_all().map_err(|_| "data-update-write")
}
struct Stage(PathBuf, bool);
impl Drop for Stage {
    fn drop(&mut self) {
        if self.1 {
            return;
        }
        // Only our uniquely created staging directory, never a remote or user-supplied child.
        if fs::canonicalize(&self.0).ok().as_ref() == Some(&self.0)
            && self
                .0
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".pending-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
pub fn refresh(root: &Path, cancel: &dyn Fn() -> bool) -> Result<Vec<String>> {
    refresh_with(root, cancel, &|p, n, c| update_io::data_get(p, n, c))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(path: &str, version: &str, broken: bool) -> Result<Vec<u8>> {
        let locale = if path.contains("zh-TW") {
            "zh-TW"
        } else if path.contains("en-US") {
            "en-US"
        } else {
            "zh-CN"
        };
        if path.contains("/config") {
            return Ok(json!({"locale":locale,"dataVersion":version})
                .to_string()
                .into_bytes());
        }
        let document = |name: &str| {
            if name == "augments.json" {
                json!({"locale":locale,"dataVersion":version,"data":[{"id":1,"name":"Fixture"}]})
            } else {
                json!({"locale":locale,"dataVersion":version,"shards":[]})
            }
            .to_string()
            .into_bytes()
        };
        if path.ends_with("manifest.json") {
            let files:Vec<_>=["augments.json","champions.json","items.json","champion-shards/index.json"].iter().map(|name|{let data=document(name);json!({"path":name,"bytes":data.len(),"hash":format!("sha256-{}",update_io::hex(&update_io::digest(&data,false).unwrap()))})}).collect();
            return Ok(json!({"locale":locale,"dataVersion":version,"files":files})
                .to_string()
                .into_bytes());
        }
        if broken {
            return Ok(b"corrupt".to_vec());
        }
        Ok(document(path.rsplit('/').next().unwrap()))
    }
    #[test]
    fn complete_locale_activation_rejects_corruption_and_preserves_old_pointer() {
        let root = std::env::temp_dir().join(format!(
            "fuze-rust-update-fixture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = Stage(root, false); // Unique test-owned directory, renamed to the checked staging prefix below.
        let test_dir = root.0.with_file_name(format!(
            ".pending-fuze-test-{}",
            root.0.file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir(&test_dir).unwrap();
        let test_dir = Stage(fs::canonicalize(test_dir).unwrap(), false);
        let ok = refresh_with(&test_dir.0, &|| false, &|p, _, _| {
            fixture(p, "1.0.0", false)
        })
        .unwrap();
        assert_eq!(ok.len(), 3);
        let before = fs::read(test_dir.0.join("current.json")).unwrap();
        assert_eq!(
            refresh_with(&test_dir.0, &|| false, &|p, _, _| fixture(p, "1.0.1", true)),
            Err("data-update-hash")
        );
        assert_eq!(fs::read(test_dir.0.join("current.json")).unwrap(), before);
        let orphan = test_dir.0.join("versions/1.0.1");
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("sentinel"), b"preserve").unwrap();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(test_dir.0.join("current.json"))
            .unwrap();
        assert_eq!(
            refresh_with(&test_dir.0, &|| false, &|p, _, _| fixture(
                p, "1.0.1", false
            )),
            Err("data-update-activate")
        );
        assert_eq!(fs::read(orphan.join("sentinel")).unwrap(), b"preserve");
        drop(locked);
        assert_eq!(fs::read(test_dir.0.join("current.json")).unwrap(), before);
        assert!(refresh_with(&test_dir.0, &|| false, &|p, _, _| fixture(
            p, "1.0.1", false
        ))
        .is_ok());
        assert_eq!(
            refresh_with(&test_dir.0, &|| true, &|_, _, _| panic!(
                "cancelled must not fetch"
            )),
            Err("data-update-cancelled")
        );
        assert!(fuze_data::prepare(
            &[Root {
                path: test_dir.0.clone(),
                source: Source::Cache
            }],
            Locale::ZhTw,
            Scope::Full
        )
        .is_ok());
    }
}
type Fetch<'a> = dyn Fn(&str, usize, &dyn Fn() -> bool) -> Result<Vec<u8>> + 'a;
fn refresh_with(root: &Path, cancel: &dyn Fn() -> bool, fetch: &Fetch<'_>) -> Result<Vec<String>> {
    clean_dir(root)?;
    let canonical = fs::canonicalize(root).map_err(|_| "data-update-directory")?;
    let root = canonical.as_path();
    let _lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(root.join("update.lock"))
        .map_err(|_| "data-update-busy")?;
    let started = Instant::now();
    let check = || {
        if cancel() || started.elapsed().as_secs() > 300 {
            Err("data-update-cancelled")
        } else {
            Ok(())
        }
    };
    let mut versions = Vec::new();
    for locale in [Locale::ZhCn, Locale::ZhTw, Locale::EnUs] {
        check()?;
        let config_path = if locale == Locale::ZhCn {
            "/api/client/v1/config".into()
        } else {
            format!("/api/client/v1/config?locale={}", locale.code())
        };
        let config: Value = serde_json::from_slice(&fetch(&config_path, 65536, cancel)?)
            .map_err(|_| "data-update-config")?;
        let version = config["dataVersion"]
            .as_str()
            .filter(|v| numeric(v))
            .ok_or("data-update-version")?;
        if locale != Locale::ZhCn && config["locale"].as_str() != Some(locale.code()) {
            return Err("data-update-locale");
        }
        let roots = [Root {
            path: root.to_path_buf(),
            source: Source::Cache,
        }];
        if let Ok(existing) = fuze_data::prepare(&roots, locale, Scope::Full) {
            let order =
                |s: &str| -> Option<Vec<u64>> { s.split('.').map(|p| p.parse().ok()).collect() };
            if order(existing.version()) >= order(version) {
                versions.push(existing.identity());
                continue;
            }
        }
        let remote = api(locale, version);
        let manifest_bytes = fetch(&format!("{remote}/manifest.json"), 2 * 1024 * 1024, cancel)?;
        let manifest: Value =
            serde_json::from_slice(&manifest_bytes).map_err(|_| "data-update-manifest")?;
        if manifest["dataVersion"].as_str() != Some(version)
            || manifest["locale"].as_str() != Some(locale.code())
        {
            return Err("data-update-locale");
        }
        let entries = manifest["files"]
            .as_array()
            .filter(|a| a.len() <= 1024)
            .ok_or("data-update-manifest")?;
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "data-update-clock")?
            .as_nanos();
        let mut stage = Stage(
            root.join(format!(".pending-{}-{unique}", std::process::id())),
            false,
        );
        fs::create_dir(&stage.0).map_err(|_| "data-update-stage")?;
        let relative = prefix(locale, version);
        let destination = stage.0.join(&relative);
        write(&destination.join("manifest.json"), &manifest_bytes)?;
        let mut total = 0usize;
        let mut seen = std::collections::HashSet::new();
        for e in entries {
            check()?;
            let name = e["path"].as_str().ok_or("data-update-manifest")?;
            fuze_data::safe_path(name)?;
            if !["augments.json", "champions.json", "items.json"].contains(&name)
                && !name.starts_with("champion-shards/")
            {
                continue;
            }
            if !name.ends_with(".json") || !seen.insert(name) {
                return Err("data-update-manifest");
            }
            let size = e["bytes"]
                .as_u64()
                .filter(|v| *v > 0 && *v <= 32 * 1024 * 1024)
                .ok_or("data-update-size")? as usize;
            total = total.checked_add(size).ok_or("data-update-size")?;
            if total > 128 * 1024 * 1024 {
                return Err("data-update-size");
            }
            let expected = e["hash"]
                .as_str()
                .and_then(|s| s.strip_prefix("sha256-"))
                .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or("data-update-hash")?;
            let bytes = fetch(&format!("{remote}/{name}"), size, cancel)?;
            if bytes.len() != size
                || update_io::hex(&update_io::digest(&bytes, false)?)
                    != expected.to_ascii_lowercase()
            {
                return Err("data-update-hash");
            }
            write(&destination.join(name), &bytes)?;
        }
        let pointer_bytes = serde_json::to_vec(
            &json!({"schemaVersion":3,"locale":locale.code(),"dataVersion":version}),
        )
        .map_err(|_| "data-update-pointer")?;
        let name = pointer(locale);
        write(&stage.0.join(&name), &pointer_bytes)?;
        fuze_data::prepare(
            &[Root {
                path: stage.0.clone(),
                source: Source::Cache,
            }],
            locale,
            Scope::Full,
        )?;
        check()?;
        let final_dir = root.join(&relative);
        clean_dir(final_dir.parent().ok_or("data-update-path")?)?;
        // A crash may have moved the complete version before swapping the pointer.
        // Keep that directory inside our transaction until the new pointer commits.
        let backup = stage.0.join("previous-version");
        let had_previous = final_dir.exists();
        if had_previous {
            clean_dir(&final_dir)?;
            fs::rename(&final_dir, &backup).map_err(|_| "data-update-activate")?;
            stage.1 = true;
        }
        if fs::rename(&destination, &final_dir).is_err() {
            if had_previous {
                stage.1 = fs::rename(&backup, &final_dir).is_err();
            }
            return Err("data-update-activate");
        }
        let from: Vec<u16> = stage
            .0
            .join(&name)
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let to: Vec<u16> = root
            .join(&name)
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            // Restore the old immutable directory as well as retaining the unchanged pointer.
            if fs::rename(&final_dir, &destination).is_ok() && had_previous {
                stage.1 = fs::rename(&backup, &final_dir).is_err();
            }
            return Err("data-update-activate");
        }
        stage.1 = false;
        versions.push(format!("{}@{version}", locale.code()));
    }
    Ok(versions)
}
