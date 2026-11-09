//! Background downloads into an isolated cache. Stage, validate, then atomically point.
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
    refresh_with(root, cancel, &|p, n, c| update_io::aramkit_get(p, n, c))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activated_aramkit_reference_is_available_offline_without_a_second_download() {
        use crate::stage_reference::{Access, Request, Service, Store};
        let dir = std::env::temp_dir().join(format!(
            ".pending-aramkit-offline-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let dir = Stage(fs::canonicalize(dir).unwrap(), false);
        refresh_with(&dir.0, &|| false, &|p, _, _| {
            let mut bytes = fixture(p, "16.22.1", false)?;
            if p.ends_with("versions.json") {
                let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                value["versions"][0]["dataPath"] = json!("data/16.22-20261106-abcdef123456");
                bytes = serde_json::to_vec(&value).unwrap();
            }
            Ok(bytes)
        })
        .unwrap();
        let prepared = fuze_data::prepare(
            &[Root {
                path: dir.0.clone(),
                source: Source::Cache,
            }],
            Locale::ZhCn,
            Scope::Full,
        )
        .unwrap();
        let identity = prepared.identity();
        assert!(prepared.display_version().contains("2026-11-06"));
        let mut details = fuze_data::detail_cache::Cache::prepare(
            &[Root {
                path: dir.0.clone(),
                source: Source::Cache,
            }],
            &identity,
        )
        .unwrap();
        let hero = details.get(145).unwrap();
        assert_eq!(hero.version, identity);
        assert_eq!(hero.display_version, prepared.display_version());
        let store = std::sync::Arc::new(Store::default());
        store.allow_network(false);
        let service = Service::with_store(
            vec![Root {
                path: dir.0.clone(),
                source: Source::Cache,
            }],
            true,
            store.clone(),
        )
        .unwrap();
        service.request(Some(Request {
            id: 145,
            locale: Locale::ZhCn,
            access: Access::CacheOnly,
        }));
        let start = Instant::now();
        loop {
            if let Some(reply) = service.changed(0).filter(|r| r.snapshot.is_some()) {
                assert_eq!(reply.snapshot.unwrap().rows[0][0].id, 1001);
                assert_eq!(reply.names[&1001], "符文甲");
                assert!(reply.issue.is_none());
                assert!(store
                    .peek(
                        145,
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs()
                    )
                    .is_some());
                break;
            }
            assert!(
                start.elapsed().as_secs() < 5,
                "offline reference was not published"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore = "explicit offline public-data review directory"]
    fn public_snapshot_activation_and_all_consumer_paths() {
        let review =
            PathBuf::from(std::env::var_os("FUZE_ARAMKIT_REVIEW").expect("review directory"));
        let dir = std::env::temp_dir().join(format!(
            ".pending-aramkit-review-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let dir = Stage(fs::canonicalize(dir).unwrap(), false);
        let fetch = |path: &str, _: usize, _: &dyn Fn() -> bool| {
            let tail = path.rsplit('/').next().unwrap();
            let name = if path.contains("champion-details/") {
                format!("aramkit-detail-{tail}")
            } else {
                format!("aramkit-{tail}")
            };
            fs::read(review.join(name)).map_err(|_| "review-missing")
        };
        refresh_with(&dir.0, &|| false, &fetch).unwrap();
        let roots = [Root {
            path: dir.0.clone(),
            source: Source::Cache,
        }];
        let mut data = fuze_data::prepare(&roots, Locale::ZhCn, Scope::Full).unwrap();
        let identity = data.identity();
        let names = data.catalog().clone();
        data.apply_current_augments().unwrap();
        data.supplement_recognition(&fuze_data::recognition::Catalog::bundled().unwrap());
        assert_eq!(identity, data.identity());
        assert_eq!(names, *data.catalog());
        assert!(!names.contains_key("力量"));
        assert!(!names.contains_key("掷骰狂人"));
        let champions: Vec<u32> = data.document("champions.json").unwrap()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_u64().unwrap() as u32)
            .collect();
        let mut cache = fuze_data::detail_cache::Cache::prepare(&roots, &identity).unwrap();
        let mut combos = 0;
        for id in &champions {
            let detail = cache.get(*id).unwrap();
            assert!(!detail.detail.augments.is_empty(), "{id}");
            assert!(!detail.detail.item_sets.is_empty(), "{id}");
            assert!(data
                .recommendations(*id)
                .unwrap()
                .values()
                .any(|r| r.win_rate.is_some()));
            let reference =
                crate::stage_reference::local_snapshot(&roots, Locale::ZhCn, *id, 1794001361)
                    .unwrap()
                    .unwrap();
            assert!(reference.patch_matches(&identity));
            combos += reference.combinations.as_ref().unwrap().len();
            for set in &detail.detail.item_sets {
                assert_eq!(set["associatedMaps"], json!([12]));
                for block in set["blocks"].as_array().unwrap() {
                    for item in block["items"].as_array().unwrap() {
                        let id = item["id"].as_str().unwrap().parse::<u64>().unwrap();
                        let row = data.document("items.json").unwrap()["data"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|r| r["id"] == id)
                            .unwrap();
                        assert_eq!(row["acquisition"], "shop");
                        assert_eq!(row["purchasable"], true);
                    }
                }
            }
        }
        assert!(!cache.warming());
        assert!(combos > 0);
        eprintln!("{} heroes: atomic activation, OCR pool, recommendations, UI detail, store item sets and {combos} local combinations passed", champions.len());
    }
    fn fixture(path: &str, version: &str, broken: bool) -> Result<Vec<u8>> {
        let pieces: Vec<_> = version.split('.').collect();
        let patch = format!("{}.{}", pieces[0], pieces[1]);
        if path.ends_with("versions.json") {
            return Ok(json!({"latest":patch,"versions":[{"version":patch,"dataPath":format!("data/{patch}-20261106-test"),"resourcePath":format!("resources/{patch}-test"),"dataDate":"2026-11-06","buildTimeUnixMs":pieces[2].parse::<u64>().unwrap()}]}).to_string().into_bytes());
        }
        if broken {
            return Ok(b"corrupt".to_vec());
        }
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/aramkit/fixture.json"
        ))
        .unwrap();
        let name = if path.contains("champion-details/") {
            "detail"
        } else {
            path.rsplit('/').next().unwrap().trim_end_matches(".json")
        };
        Ok(serde_json::to_vec(&fixture[name]).unwrap())
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
        assert_eq!(ok.len(), 1);
        let before = fs::read(test_dir.0.join("current.json")).unwrap();
        let ready = std::cell::Cell::new(false);
        assert_eq!(
            refresh_transaction(
                &test_dir.0,
                &[Locale::ZhCn],
                &|| false,
                &|p, _, _| fixture(p, "1.0.1", false),
                &|_| {},
                &|| {
                    ready.set(true);
                    assert_eq!(fs::read(test_dir.0.join("current.json")).unwrap(), before);
                    Err("data-update-cancelled")
                }
            ),
            Err("data-update-cancelled")
        );
        assert!(ready.get());
        assert_eq!(fs::read(test_dir.0.join("current.json")).unwrap(), before);
        assert_eq!(
            refresh_with(&test_dir.0, &|| false, &|p, _, _| fixture(p, "1.0.1", true)),
            Err("data-update-json")
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
            Locale::ZhCn,
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
        assert_eq!(counts.first(), Some(&(0, 5)));
        assert_eq!(counts.last(), Some(&(5, 5)));
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
                        p.ends_with("/versions.json"),
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
        assert!(compatibility(Some("zh-CN@16.20.4"), "16.22.1").1);
        assert!(!compatibility(Some("zh-CN@16.21.1-augments16.21"), "16.21.1").1);
        assert!(compatibility(None, "16.22.1").1);
        assert!(compatibility(Some("zh-CN@16.21.5"), "16.22.1")
            .0
            .contains("落后"));
        assert!(compatibility(Some("zh-CN@16.22.1"), "").1);
        assert!(!compatibility(Some("zh-CN@16.22.1"), "26.22").1);
        let (label, mismatch) = compatibility(Some("zh-CN@16.22.1794001361404"), "26.22");
        assert!(!mismatch);
        assert!(label.starts_with("统计 16.22 ·"));
        assert!(!label.contains("1794001361404"));

        // A stopped transfer must keep both the previous pointer and honest partial progress.
        let before = fs::read(dir.0.join("current.json")).unwrap();
        events.borrow_mut().clear();
        let interrupted = refresh_current_with(
            &dir.0,
            &roots,
            Locale::ZhCn,
            &|| false,
            &|p| events.borrow_mut().push(p),
            &|p, _, _| {
                if p.ends_with("/items.json") {
                    Err("update-network")
                } else {
                    fixture(p, "1.0.1", false)
                }
            },
        );
        assert_eq!(interrupted, Err("update-network"));
        assert_eq!(fs::read(dir.0.join("current.json")).unwrap(), before);
        let events = events.borrow();
        let last = events.last().unwrap();
        assert_eq!((last.completed, last.total), (0, 0));
        assert!(last.message.contains("装备资料"));
        assert!(result_message(&interrupted, None).contains("未能连接数据站"));
        assert!(result_message(&Err("update-cancelled"), None).contains("已取消"));
        assert!(result_message(&Err("data-update-busy"), None).contains("正在进行"));
    }
}
type Fetch<'a> = dyn Fn(&str, usize, &dyn Fn() -> bool) -> Result<Vec<u8>> + 'a;
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
        Ok(false) => "没有更新的推荐数据，无需下载。",
        Err("data-update-cancelled" | "update-cancelled") => "更新已取消，继续使用现有数据。",
        Err("update-network") => "未能连接数据站，继续使用现有数据；可稍后重试。",
        Err("update-http-status" | "update-access-or-release" | "update-redirect") => {
            "数据站暂未提供可下载的数据，继续使用现有数据；可稍后重试。"
        }
        Err("data-update-busy") => "另一次数据更新正在进行，请等待完成。",
        Err("data-update-hash") => "下载校验失败，继续使用现有数据；请重试。",
        Err("data-update-write" | "data-update-directory" | "data-update-activate") => {
            "数据写入失败，继续使用现有数据；请检查磁盘空间和目录权限。"
        }
        Err(e) if e.starts_with("aramkit-") || *e == "data-update-json" => {
            "数据站返回的资料格式无法读取，继续使用现有数据；请稍后重试。"
        }
        Err(_) => "数据更新未完成，继续使用现有数据；可稍后重试。",
    };
    if result.is_ok() && supplemental_error.is_some() {
        format!("{main} 补充识别词库更新失败，可稍后重试。")
    } else {
        main.into()
    }
}
/// Public patch reference is separate from both local data and the user's game client.
pub fn compatibility(identity: Option<&str>, reference: &str) -> (String, bool) {
    let version = identity.unwrap_or("").rsplit('@').next().unwrap_or("");
    let version = version
        .strip_suffix(fuze_data::current_augments::IDENTITY_SUFFIX)
        .unwrap_or(version);
    // Build timestamps identify immutable snapshots internally, not player-facing patches.
    let version = version
        .rsplit_once('.')
        .filter(|(_, r)| r.parse::<u64>().is_ok_and(|n| n >= 1_000_000_000_000))
        .map_or(version, |(p, _)| p);
    let patch = fuze_data::recognition::patch;
    let Some(stats) = patch(version) else {
        return ("尚无可用统计数据".into(), true);
    };
    let Some(latest) = patch(reference) else {
        return (
            format!("统计 {version} · 最新补丁尚未确认，请检查数据更新"),
            true,
        );
    };
    let status = if stats == latest {
        "统计与公开补丁一致"
    } else if stats < latest {
        "统计落后于公开补丁，等待数据站更新"
    } else {
        "统计与公开补丁不一致"
    };
    (
        format!("统计 {version} · 公开补丁 {reference}\n{status}"),
        stats != latest,
    )
}

fn phase(report: &Report<'_>, message: &str) {
    report(Progress {
        message: message.into(),
        ..Default::default()
    });
}
fn refresh_with(root: &Path, cancel: &dyn Fn() -> bool, fetch: &Fetch<'_>) -> Result<Vec<String>> {
    refresh_for(root, &[Locale::ZhCn], cancel, fetch)
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
        update_io::aramkit_get(p, n, c)
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
        update_io::aramkit_get(p, n, c)
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
    refresh_guarded(root, roots, locale, cancel, report, fetch, &|| Ok(()))
}
pub(crate) fn refresh_guarded(
    root: &Path,
    roots: &[Root],
    locale: Locale,
    cancel: &dyn Fn() -> bool,
    report: &Report<'_>,
    fetch: &Fetch<'_>,
    before_activate: &dyn Fn() -> Result<()>,
) -> Result<bool> {
    phase(report, "正在检查数据站版本…");
    if cancel() {
        return Err("data-update-cancelled");
    }
    let path = "/data/versions.json";
    let config = fetch(path, 65536, cancel)?;
    let value: Value = serde_json::from_slice(&config).map_err(|_| "data-update-config")?;
    let release = fuze_data::aramkit::Release::parse(&value)?;
    let version = release.version.as_str();
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
    refresh_transaction(
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
        before_activate,
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
    refresh_transaction(root, locales, cancel, fetch, report, &|| Ok(()))
}
fn refresh_transaction(
    root: &Path,
    locales: &[Locale],
    cancel: &dyn Fn() -> bool,
    fetch: &Fetch<'_>,
    report: &Report<'_>,
    before_activate: &dyn Fn() -> Result<()>,
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
        let config: Value = serde_json::from_slice(&fetch("/data/versions.json", 65536, cancel)?)
            .map_err(|_| "data-update-config")?;
        let release = fuze_data::aramkit::Release::parse(&config)?;
        let version = release.version.as_str();
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
        phase(report, "正在下载 ARAMKit 资料与统计…");
        let read = |path: &str| -> Result<Value> {
            check()?;
            serde_json::from_slice(&fetch(path, 8 * 1024 * 1024, cancel)?)
                .map_err(|_| "data-update-json")
        };
        let heroes = read(&release.resource(locale, "champions"))?;
        let augments = read(&release.resource(locale, "augments"))?;
        phase(report, "正在下载装备资料…");
        let items = read(&release.resource(locale, "items"))?;
        let spells = read(&release.resource(locale, "summoner-spells"))?;
        let hero_ranks = read(&release.stats("champion-rankings"))?;
        let augment_ranks = read(&release.stats("augment-rankings"))?;
        let catalog = fuze_data::aramkit::Catalog::new(
            release.clone(),
            locale,
            heroes,
            augments,
            items,
            spells,
            hero_ranks,
            augment_ranks,
        )?;
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
        let count = 4 + catalog.champions.len();
        let mut completed = 0;
        let mut total = 0usize;
        let mut files = Vec::new();
        let mut save = |name: &str, value: Value| -> Result<()> {
            check()?;
            let bytes = serde_json::to_vec(&value).map_err(|_| "data-update-json")?;
            total += bytes.len();
            if bytes.len() > 32 * 1024 * 1024 || total > 128 * 1024 * 1024 {
                return Err("data-update-size");
            }
            write(&destination.join(name), &bytes)?;
            files.push(json!({"path":name,"bytes":bytes.len()}));
            completed += 1;
            report(Progress {
                message: format!("已准备 · {completed}/{count} 个文件"),
                completed,
                total: count,
            });
            Ok(())
        };
        report(Progress {
            message: "正在整理英雄、海克斯与装备资料…".into(),
            completed: 0,
            total: count,
        });
        for (name, value) in catalog.documents() {
            save(&name, value)?;
        }
        for id in catalog.champions.keys() {
            let value = read(&release.stats(&format!("champion-details/{id}")))?;
            save(
                &format!("champion-shards/{id}.json"),
                catalog.detail(*id, &value)?,
            )?;
        }
        let manifest = json!({"source":fuze_data::aramkit::SOURCE,"locale":locale.code(),"dataVersion":version,"gamePatch":release.patch,"dataDate":release.date,"dataPath":release.data,"resourcePath":release.resources,"files":files});
        write(
            &destination.join("manifest.json"),
            &serde_json::to_vec(&manifest).map_err(|_| "data-update-json")?,
        )?;
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
        before_activate()?;
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
