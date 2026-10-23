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
        && v.split('.').all(|p| {
            !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u64>().is_ok()
        })
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
        refresh_with(&test_dir.0, &|| false, &|p, _, _| {
            fixture(p, "1.0.2", false)
        })
        .unwrap();
        assert!(!test_dir.0.join("versions/1.0.0").exists());
        assert!(test_dir.0.join("versions/1.0.1").exists());
        assert!(test_dir.0.join("versions/1.0.2").exists());
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
    #[test]
    fn chinese_progress_and_bundled_version_skip() {
        let dir = std::env::temp_dir().join(format!(
            ".pending-fuze-progress-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let dir = Stage(fs::canonicalize(dir).unwrap(), false);
        let events = std::cell::RefCell::new(Vec::new());
        let fetch = |p: &str, _: usize, _: &dyn Fn() -> bool| {
            assert!(!p.contains("zh-TW") && !p.contains("en-US"));
            fixture(p, "1.0.0", false)
        };
        assert_eq!(
            refresh_current_with(
                &dir.0,
                &[],
                Locale::ZhCn,
                &|| false,
                &|p| events.borrow_mut().push(p),
                &fetch
            ),
            Ok(true)
        );
        let counts: Vec<_> = events
            .borrow()
            .iter()
            .filter(|p| p.total > 0)
            .map(|p| (p.completed, p.total))
            .collect();
        assert_eq!(counts.first(), Some(&(0, 4)));
        assert_eq!(counts.last(), Some(&(4, 4)));
        assert!(counts.windows(2).all(|p| p[0].0 <= p[1].0));
        assert!(!dir.0.join("current.en-US.json").exists());
        let roots = [Root {
            path: dir.0.clone(),
            source: Source::Bundled,
        }];
        assert_eq!(
            refresh_current_with(
                &dir.0,
                &roots,
                Locale::ZhCn,
                &|| false,
                &|_| {},
                &|p, _, _| {
                    assert!(
                        p.ends_with("/config"),
                        "unchanged bundled data must not download"
                    );
                    fixture(p, "1.0.0", false)
                }
            ),
            Ok(false)
        );
        assert!(result_message(&Ok(false), None).contains("无需下载"));
        assert!(result_message(&Ok(true), Some("failed")).contains("补充识别词库更新失败"));
        assert!(result_message(&Err("data-update-hash"), None).contains("校验失败"));
        assert!(compatibility(Some("zh-CN@16.20.4")).1);
        assert!(!compatibility(Some("zh-CN@16.21.1-augments16.21")).1);
        assert!(compatibility(None).0.contains("未确认"));
    }
}
type Fetch<'a> = dyn Fn(&str, usize, &dyn Fn() -> bool) -> Result<Vec<u8>> + 'a;
fn selected(name: &str) -> bool {
    ["augments.json", "champions.json", "items.json"].contains(&name)
        || name.starts_with("champion-shards/")
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub message: String,
    pub completed: usize,
    pub total: usize,
}
type Report<'a> = dyn Fn(Progress) + 'a;
pub fn result_message(result: &Result<bool>, supplemental_error: Option<&str>) -> String {
    let main = match result {
        Ok(true) => "推荐数据已更新，下次识别使用新数据。",
        Ok(false) => "本地数据已是数据站最新版本，无需下载。",
        Err("data-update-cancelled") => "更新已取消，继续使用现有数据。",
        Err("data-update-hash") => "下载校验失败，继续使用现有数据；请重试。",
        Err("data-update-write" | "data-update-directory" | "data-update-activate") => {
            "数据写入失败，继续使用现有数据；请检查磁盘空间和目录权限。"
        }
        Err(_) => "数据更新未完成，继续使用现有数据；可稍后重试。",
    };
    if result.is_ok() && supplemental_error.is_some() {
        format!("{main} 补充识别词库更新失败，可稍后重试。")
    } else {
        main.into()
    }
}
/// Compare the two data sources, not an unobserved live client version.
pub fn compatibility(identity: Option<&str>) -> (String, bool) {
    let version = identity.unwrap_or("").rsplit('@').next().unwrap_or("");
    let version = version
        .strip_suffix(fuze_data::current_augments::IDENTITY_SUFFIX)
        .unwrap_or(version);
    let patch = |v: &str| -> Option<(u32, u32)> {
        let mut parts = v.split('.');
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    };
    let Some(stats) = patch(version) else {
        return ("版本匹配：未确认 · 尚无可用统计数据".into(), true);
    };
    let builtin = fuze_data::current_augments::PATCH;
    let metadata = if stats > patch(builtin).unwrap() {
        version
    } else {
        builtin
    };
    let matched = patch(metadata) == Some(stats);
    (
        format!(
            "资料 {metadata} · 统计 {version}\n{}",
            if matched {
                "版本匹配 · 不代表每个符文已有统计"
            } else {
                "版本不匹配 · 不匹配的符文显示暂无统计"
            }
        ),
        !matched,
    )
}
fn phase(report: &Report<'_>, message: &str) {
    report(Progress {
        message: message.into(),
        ..Default::default()
    });
}
fn refresh_with(root: &Path, cancel: &dyn Fn() -> bool, fetch: &Fetch<'_>) -> Result<Vec<String>> {
    refresh_for(
        root,
        &[Locale::ZhCn, Locale::ZhTw, Locale::EnUs],
        cancel,
        fetch,
    )
}
/// Idle background refresh compares the complete selected local bundle first. No repeated full
/// verification/download of an unchanged bundled version and no unrelated language downloads.
pub fn refresh_current(
    root: &Path,
    roots: &[Root],
    locale: Locale,
    cancel: &dyn Fn() -> bool,
) -> Result<bool> {
    refresh_current_with(root, roots, locale, cancel, &|_| {}, &|p, n, c| {
        update_io::data_get(p, n, c)
    })
}
/// The production manual action only updates the Chinese catalogue, including bundled data
/// when deciding whether a download is needed.
pub fn refresh_chinese(
    root: &Path,
    roots: &[Root],
    cancel: &dyn Fn() -> bool,
    report: &Report<'_>,
) -> Result<bool> {
    refresh_current_with(root, roots, Locale::ZhCn, cancel, report, &|p, n, c| {
        update_io::data_get(p, n, c)
    })
}
fn refresh_current_with(
    root: &Path,
    roots: &[Root],
    locale: Locale,
    cancel: &dyn Fn() -> bool,
    report: &Report<'_>,
    fetch: &Fetch<'_>,
) -> Result<bool> {
    phase(report, "正在检查数据站版本…");
    let path = if locale == Locale::ZhCn {
        "/api/client/v1/config".into()
    } else {
        format!("/api/client/v1/config?locale={}", locale.code())
    };
    let config = fetch(&path, 65536, cancel)?;
    let value: Value = serde_json::from_slice(&config).map_err(|_| "data-update-config")?;
    let version = value["dataVersion"]
        .as_str()
        .filter(|v| numeric(v))
        .ok_or("data-update-version")?;
    if locale != Locale::ZhCn && value["locale"] != locale.code() {
        return Err("data-update-locale");
    }
    let order = |v: &str| {
        v.split('.')
            .map(|p| p.parse::<u64>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .ok()
    };
    // A current overview alone cannot satisfy the detail/build cache. Re-fetch a
    // partial same-version download instead of reporting it as already up to date.
    if fuze_data::prepare(roots, locale, Scope::Full)
        .is_ok_and(|data| order(data.version()) >= order(version))
    {
        return Ok(false);
    }
    refresh_for_progress(
        root,
        &[locale],
        cancel,
        &|p, n, c| {
            if p == path {
                Ok(config.clone())
            } else {
                fetch(p, n, c)
            }
        },
        report,
    )?;
    Ok(true)
}
fn refresh_for(
    root: &Path,
    locales: &[Locale],
    cancel: &dyn Fn() -> bool,
    fetch: &Fetch<'_>,
) -> Result<Vec<String>> {
    refresh_for_progress(root, locales, cancel, fetch, &|_| {})
}
fn refresh_for_progress(
    root: &Path,
    locales: &[Locale],
    cancel: &dyn Fn() -> bool,
    fetch: &Fetch<'_>,
    report: &Report<'_>,
) -> Result<Vec<String>> {
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
    for &locale in locales {
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
        let previous = fuze_data::prepare(&roots, locale, Scope::Full)
            .ok()
            .map(|v| v.version().to_owned());
        if let Some(existing) = previous.as_deref() {
            let order =
                |s: &str| -> Option<Vec<u64>> { s.split('.').map(|p| p.parse().ok()).collect() };
            if order(existing) >= order(version) {
                versions.push(format!("{}@{existing}", locale.code()));
                continue;
            }
        }
        let remote = api(locale, version);
        phase(report, "正在读取下载清单…");
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
        write(
            &stage.0.join(".fuze-data-transaction"),
            b"fuze-data-staging-v1",
        )?;
        let relative = prefix(locale, version);
        let destination = stage.0.join(&relative);
        write(&destination.join("manifest.json"), &manifest_bytes)?;
        let mut total = 0usize;
        let mut seen = std::collections::HashSet::new();
        let count = entries
            .iter()
            .filter(|e| e["path"].as_str().is_some_and(selected))
            .count();
        let mut completed = 0;
        for e in entries {
            check()?;
            let name = e["path"].as_str().ok_or("data-update-manifest")?;
            fuze_data::safe_path(name)?;
            if !selected(name) {
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
            report(Progress {
                message: format!("正在下载并校验 · {completed}/{count} 个文件"),
                completed,
                total: count,
            });
            let bytes = fetch(&format!("{remote}/{name}"), size, cancel)?;
            if bytes.len() != size
                || update_io::hex(&update_io::digest(&bytes, false)?)
                    != expected.to_ascii_lowercase()
            {
                return Err("data-update-hash");
            }
            write(&destination.join(name), &bytes)?;
            completed += 1;
            report(Progress {
                message: format!("已下载并校验 · {completed}/{count} 个文件"),
                completed,
                total: count,
            });
        }
        phase(report, "正在校验完整数据…");
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
        phase(report, "正在启用新数据…");
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
        // Only after pointer commit; retain the previous complete locale version.
        let parent = final_dir.parent().ok_or("data-update-path")?;
        if let Some(previous) = previous.as_deref() {
            for old in crate::cache_maintenance::children(parent) {
                let Some(name) = old.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !numeric(name) || name == version || name == previous {
                    continue;
                }
                let manifest =
                    crate::update_channel::bounded(&old.join("manifest.json"), 2 * 1024 * 1024)
                        .ok()
                        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
                if manifest
                    .is_some_and(|m| m["dataVersion"] == name && m["locale"] == locale.code())
                {
                    let _ = crate::cache_maintenance::retire(parent, &old, &|s| {
                        [
                            "manifest.json",
                            "augments.json",
                            "champions.json",
                            "items.json",
                        ]
                        .contains(&s)
                            || s.starts_with("champion-shards/")
                                && s.ends_with(".json")
                                && fuze_data::safe_path(s).is_ok()
                    });
                }
            }
        }
        versions.push(format!("{}@{version}", locale.code()));
    }
    Ok(versions)
}
