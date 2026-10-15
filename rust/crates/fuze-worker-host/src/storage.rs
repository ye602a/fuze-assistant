//! Player-requested storage preview. Preserves records, settings, current data and one fallback.
use crate::Result;
use std::{
    fs,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
    directory: bool,
}
#[derive(Clone)]
pub struct Preview {
    pub categories: Vec<(&'static str, usize, u64)>,
    pub versions: Vec<(String, usize, u64)>,
    root: PathBuf,
    candidates: Vec<Entry>,
    checked: Instant,
    pub bytes: u64,
    pub reclaimable: u64,
    pub files: usize,
    pub removable: usize,
    pub protection_unknown: bool,
}
fn safe(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        let m = fs::symlink_metadata(p).map_err(|_| "storage-read")?;
        if m.file_attributes() & 0x400 != 0 {
            return Err("storage-reparse");
        }
    }
    Ok(())
}
fn json(path: &Path) -> Result<serde_json::Value> {
    safe(path)?;
    serde_json::from_slice(&crate::update_channel::bounded(path, 2 * 1024 * 1024)?)
        .map_err(|_| "storage-data-invalid")
}
fn version(s: &str) -> Option<Vec<u64>> {
    if s.len() > 64 {
        return None;
    }
    let v: Option<Vec<_>> = s
        .split('.')
        .map(|p| {
            if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
                p.parse().ok()
            } else {
                None
            }
        })
        .collect();
    v.filter(|v| !v.is_empty() && v.len() <= 6)
}
fn manifest_files(
    m: &serde_json::Value,
) -> Option<std::collections::BTreeMap<String, serde_json::Value>> {
    match &m["files"] {
        serde_json::Value::Object(v) if !v.is_empty() && v.len() <= 512 => {
            Some(v.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        serde_json::Value::Array(v) if !v.is_empty() && v.len() <= 512 => v
            .iter()
            .map(|v| {
                Some((
                    v.get("path")
                        .or_else(|| v.get("logicalPath"))?
                        .as_str()?
                        .to_owned(),
                    v.clone(),
                ))
            })
            .collect(),
        _ => None,
    }
}
fn tree(root: &Path) -> Result<Vec<Entry>> {
    let mut paths = vec![root.to_path_buf()];
    let mut out = Vec::new();
    let mut n = 0;
    while n < paths.len() {
        safe(&paths[n])?;
        for child in fs::read_dir(&paths[n]).map_err(|_| "storage-read")? {
            let p = child.map_err(|_| "storage-read")?.path();
            safe(&p)?;
            let m = fs::symlink_metadata(&p).map_err(|_| "storage-read")?;
            if !m.is_dir() && !m.is_file() {
                return Err("storage-kind");
            }
            if m.is_dir() {
                paths.push(p.clone());
            }
            out.push(Entry {
                path: p,
                bytes: if m.is_file() { m.len() } else { 0 },
                modified: m.modified().map_err(|_| "storage-read")?,
                directory: m.is_dir(),
            });
            if out.len() > 8192 || paths.len() > 128 {
                return Err("storage-limit");
            }
        }
        n += 1;
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
pub fn scan(root: &Path) -> Result<Preview> {
    scan_at(root, SystemTime::now())
}
fn scan_at(root: &Path, now: SystemTime) -> Result<Preview> {
    safe(root)?;
    let root = fs::canonicalize(root).map_err(|_| "storage-root")?;
    let entries = tree(&root)?;
    let data = root.join("client-data");
    let mut targets = Vec::new();
    let mut unknown = false;
    for locale in ["zh-CN", "zh-TW", "en-US"] {
        let parent = if locale == "zh-CN" {
            data.join("versions")
        } else {
            data.join("versions").join(locale)
        };
        let mut versions = Vec::new();
        for entry in entries
            .iter()
            .filter(|e| e.directory && e.path.parent() == Some(parent.as_path()))
        {
            let Some(name) = entry.path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(key) = version(name) else {
                continue;
            };
            let manifest = json(&entry.path.join("manifest.json")).ok();
            let complete = manifest.as_ref().is_some_and(|m| {
                m["dataVersion"] == name
                    && m["locale"].as_str().unwrap_or("zh-CN") == locale
                    && manifest_files(m).is_some_and(|files| {
                        !files.is_empty()
                            && files.len() <= 512
                            && files.iter().all(|(name, meta)| {
                                fuze_data::safe_path(name).is_ok()
                                    && entries.iter().any(|e| {
                                        e.path == entry.path.join(name)
                                            && !e.directory
                                            && e.bytes > 0
                                            && meta["bytes"]
                                                .as_u64()
                                                .is_none_or(|size| size == e.bytes)
                                    })
                            })
                    })
            });
            versions.push((key, entry, manifest, complete));
        }
        if versions.is_empty() {
            continue;
        }
        let pointer = if locale == "zh-CN" {
            "current.json".into()
        } else {
            format!("current.{locale}.json")
        };
        let current = json(&data.join(pointer))
            .ok()
            .and_then(|v| v["dataVersion"].as_str().and_then(version));
        let Some(current) = current.filter(|v| {
            versions
                .iter()
                .any(|(key, _, _, complete)| key == v && *complete)
        }) else {
            unknown = true;
            continue;
        };
        let fallback = versions
            .iter()
            .filter(|(key, _, _, complete)| *complete && key < &current)
            .map(|(key, _, _, _)| key)
            .max();
        for (key, entry, manifest, complete) in &versions {
            if !complete || key >= &current || Some(key) == fallback {
                continue;
            }
            let descendants: Vec<_> = entries
                .iter()
                .filter(|e| e.path.starts_with(&entry.path))
                .collect();
            let files = manifest_files(manifest.as_ref().unwrap()).unwrap();
            if descendants.iter().any(|e| {
                now.duration_since(e.modified).unwrap_or_default() < Duration::from_secs(7 * 86400)
                    || (!e.directory
                        && e.path.strip_prefix(&entry.path).ok().is_none_or(|p| {
                            let name = p.to_string_lossy().replace('\\', "/");
                            name != "manifest.json" && !files.contains_key(&name)
                        }))
            }) {
                continue;
            }
            targets.extend(descendants.into_iter().cloned());
        }
    }
    // Only marked transactions created by this version. A backup may be the sole
    // remaining complete version after interruption: never remove it as staging.
    for stage in entries
        .iter()
        .filter(|e| e.directory && e.path.parent() == Some(data.as_path()))
    {
        let Some(name) = stage.path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let owned = name
            .strip_prefix(".pending-")
            .and_then(|s| s.split_once('-'))
            .is_some_and(|(pid, nonce)| {
                !pid.is_empty()
                    && !nonce.is_empty()
                    && pid.bytes().chain(nonce.bytes()).all(|c| c.is_ascii_digit())
            });
        if !owned
            || stage.path.join("previous-version").exists()
            || crate::update_channel::bounded(&stage.path.join(".fuze-data-transaction"), 64)
                .ok()
                .as_deref()
                != Some(b"fuze-data-staging-v1")
        {
            continue;
        }
        let children: Vec<_> = entries
            .iter()
            .filter(|e| e.path.starts_with(&stage.path))
            .collect();
        if children.iter().any(|e| {
            now.duration_since(e.modified).unwrap_or_default() < Duration::from_secs(7 * 86400)
                || (!e.directory
                    && !e
                        .path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|name| {
                            [
                                ".fuze-data-transaction",
                                "manifest.json",
                                "augments.json",
                                "champions.json",
                                "items.json",
                                "index.json",
                                "current.json",
                                "current.zh-CN.json",
                                "current.zh-TW.json",
                                "current.en-US.json",
                            ]
                            .contains(&name)
                                || (e
                                    .path
                                    .parent()
                                    .and_then(|p| p.file_name())
                                    .is_some_and(|n| n == "champion-shards")
                                    && name.strip_suffix(".json").is_some_and(|id| {
                                        !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
                                    }))
                        }))
        }) {
            continue;
        }
        targets.extend(children.into_iter().cloned());
    }
    targets.sort_by(|a, b| a.path.cmp(&b.path));
    targets.dedup_by(|a, b| a.path == b.path);
    let mut categories = vec![
        ("推荐与识别数据", 0, 0),
        ("图标缓存", 0, 0),
        ("个人记录与设置（保留）", 0, 0),
        ("诊断文件（保留）", 0, 0),
        ("其他文件（保留）", 0, 0),
    ];
    for e in entries.iter().filter(|e| !e.directory) {
        let relative = e.path.strip_prefix(&root).map_err(|_| "storage-boundary")?;
        let first = relative
            .components()
            .next()
            .and_then(|v| v.as_os_str().to_str())
            .unwrap_or("");
        let group = if ["client-data", "recognition-data", "stage-reference"].contains(&first) {
            0
        } else if first.contains("icon") || first == "art" {
            1
        } else if first == "diagnostics" {
            3
        } else if [
            "observations.json",
            "observations-v2.json",
            "account-history.json",
            "account-history-v2.json",
            "account-history-v3.json",
            "preferences.json",
            "window-memory.json",
        ]
        .contains(&first)
        {
            2
        } else {
            4
        };
        categories[group].1 += 1;
        categories[group].2 += e.bytes;
    }
    let mut groups = std::collections::BTreeMap::<String, (usize, u64)>::new();
    for e in targets.iter().filter(|e| !e.directory) {
        let parts: Vec<_> = e
            .path
            .strip_prefix(&data)
            .map_err(|_| "storage-boundary")?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let label = if parts.first().is_some_and(|p| p.starts_with(".pending-")) {
            crate::i18n::text("过期下载事务").to_owned()
        } else if parts.get(1).is_some_and(|p| p == "zh-TW" || p == "en-US") {
            format!("{} / {}", parts[1], parts.get(2).ok_or("storage-version")?)
        } else {
            format!("zh-CN / {}", parts.get(1).ok_or("storage-version")?)
        };
        let entry = groups.entry(label).or_default();
        entry.0 += 1;
        entry.1 += e.bytes;
    }
    Ok(Preview {
        categories,
        versions: groups.into_iter().map(|(v, (n, b))| (v, n, b)).collect(),
        bytes: entries
            .iter()
            .filter(|e| !e.directory)
            .map(|e| e.bytes)
            .sum(),
        files: entries.iter().filter(|e| !e.directory).count(),
        reclaimable: targets.iter().map(|e| e.bytes).sum(),
        removable: targets.iter().filter(|e| !e.directory).count(),
        root,
        candidates: targets,
        checked: Instant::now(),
        protection_unknown: unknown,
    })
}
pub fn clean(preview: Preview) -> Result<Preview> {
    clean_at(preview, SystemTime::now())
}
fn clean_at(preview: Preview, now: SystemTime) -> Result<Preview> {
    if preview.checked.elapsed() > Duration::from_secs(120) {
        return Err("storage-preview-expired");
    }
    let data = preview.root.join("client-data");
    safe(&data)?;
    if data.join("update.lock").exists() {
        safe(&data.join("update.lock"))?;
    }
    let _lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(data.join("update.lock"))
        .map_err(|_| "storage-busy")?;
    let current = scan_at(&preview.root, now)?;
    if current.candidates != preview.candidates {
        return Err("storage-changed");
    }
    // Check every candidate before the first deletion; paths never come from the renderer.
    for e in &current.candidates {
        safe(&e.path)?;
        if !e.path.starts_with(&data) || fs::canonicalize(&e.path).ok().as_ref() != Some(&e.path) {
            return Err("storage-boundary");
        }
    }
    for e in current.candidates.iter().filter(|e| !e.directory) {
        safe(&e.path)?;
        fs::remove_file(&e.path).map_err(|_| "storage-remove")?;
    }
    for e in current.candidates.iter().rev().filter(|e| e.directory) {
        safe(&e.path)?;
        fs::remove_dir(&e.path).map_err(|_| "storage-remove")?;
    }
    scan(&preview.root)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn keeps_current_fallback_and_records_and_does_not_hash_payloads() {
        let root = std::env::temp_dir().join(format!("fuze-storage-{}", std::process::id()));
        assert!(!root.exists());
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        for v in ["1.0.0", "1.0.1", "1.0.2"] {
            let dir = root.join("client-data/versions").join(v);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("augments.json"), b"{}").unwrap();
            fs::write(dir.join("manifest.json"),serde_json::to_vec(&json!({"dataVersion":v,"locale":"zh-CN","files":{"augments.json":{"bytes":2}}})).unwrap()).unwrap();
        }
        fs::write(
            root.join("client-data/current.json"),
            br#"{"dataVersion":"1.0.2"}"#,
        )
        .unwrap();
        fs::write(root.join("observations.json"), b"private-fixture").unwrap();
        for (name, backup, marked) in [
            (".pending-1-100", false, true),
            (".pending-2-100", true, true),
            (".pending-3-100", false, false),
        ] {
            let dir = root.join("client-data").join(name);
            fs::create_dir_all(&dir).unwrap();
            if marked {
                fs::write(dir.join(".fuze-data-transaction"), b"fuze-data-staging-v1").unwrap();
            }
            fs::write(dir.join("manifest.json"), b"{}").unwrap();
            if backup {
                fs::create_dir(dir.join("previous-version")).unwrap();
            }
        }
        let p = scan_at(&root, SystemTime::now() + Duration::from_secs(8 * 86400)).unwrap();
        assert_eq!(p.removable, 4);
        assert!(!p.protection_unknown);
        assert!(p
            .candidates
            .iter()
            .all(|e| e.path.to_string_lossy().contains("1.0.0")
                || e.path.to_string_lossy().contains(".pending-1-100")));
        assert!(clean(p.clone()).is_err());
        let cleaned = clean_at(p, SystemTime::now() + Duration::from_secs(8 * 86400)).unwrap();
        assert_eq!(cleaned.removable, 0);
        assert!(!root.join("client-data/versions/1.0.0").exists());
        assert!(root.join("client-data/versions/1.0.1").exists());
        assert!(root.join("client-data/versions/1.0.2").exists());
        assert!(root.join("observations.json").exists());
        assert!(!root.join("client-data/.pending-1-100").exists());
        assert!(root
            .join("client-data/.pending-2-100/previous-version")
            .exists());
        assert!(root.join("client-data/.pending-3-100").exists());
        crate::cache_maintenance::retire(root.parent().unwrap(), &root, &|_| true).unwrap();
    }
}
