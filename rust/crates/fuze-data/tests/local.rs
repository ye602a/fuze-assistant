use fuze_data::{prepare, safe_path, Active, Locale, Root, Scope, Source};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fuze-rust-data-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn root(&self, source: Source) -> Root {
        Root {
            path: self.0.clone(),
            source,
        }
    }
    fn write(&self, relative: &str, value: serde_json::Value) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn dataset(&self, locale: &str, ver: &str, full: bool) {
        let pointer = if locale == "zh-CN" {
            "current.json".into()
        } else {
            format!("current.{locale}.json")
        };
        self.write(
            &pointer,
            json!({"schemaVersion":3,"locale":locale,"dataVersion":ver}),
        );
        let prefix = if locale == "zh-CN" {
            format!("versions/{ver}")
        } else {
            format!("versions/{locale}/{ver}")
        };
        self.write(
            &format!("{prefix}/manifest.json"),
            json!({"locale":locale,"dataVersion":ver,"files":[{"path":"augments.json"}]}),
        );
        self.write(&format!("{prefix}/augments.json"),json!({"meta":{"locale":locale,"dataVersion":ver},"data":[{"id":100,"name":"测试 符文"}]}));
        if full {
            for file in ["champions.json", "items.json", "champion-shards/index.json"] {
                self.write(&format!("{prefix}/{file}"), json!({"data":[]}));
            }
        }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn all_hero_details_survive_file_removal_and_more_than_eight_selections() {
    use fuze_data::detail_cache::Cache;
    use std::sync::Arc;
    let root = Temp::new();
    root.dataset("zh-CN", "16.20.1", true);
    let ids: Vec<_> = (1..=20).collect();
    root.write(
        "versions/16.20.1/manifest.json",
        json!({"files":{
        "augments.json":{},"champions.json":{},"items.json":{},
        "champion-shards/index.json":{},"champion-shards/1.json":{}}}),
    );
    root.write(
        "versions/16.20.1/champion-shards/index.json",
        json!({"shards":[{
        "path":"champion-shards/1.json","championIds":ids}]}),
    );
    let heroes: serde_json::Map<_, _> = (1..=20)
        .map(|id| {
            (
                id.to_string(),
                json!({
        "champion":{"name":format!("fixture-{id}")},
        "augments":[{"id":100,"name":"测试符文"}]}),
            )
        })
        .collect();
    root.write(
        "versions/16.20.1/champion-shards/1.json",
        json!({"champions":heroes}),
    );
    let roots = [root.root(Source::Bundled)];
    assert!(Cache::prepare(&roots, "zh-CN@16.19.1").is_err());
    let mut cache = Cache::prepare(&roots, "zh-CN@16.20.1").unwrap();
    // Remove only our synthetic fixture, proving clicks/warm-up cannot read it again.
    fs::remove_dir_all(root.0.join("versions")).unwrap();
    let first = cache.get(1).unwrap();
    assert_eq!(first.id, 1);
    for id in 2..=20 {
        assert_eq!(cache.get(id).unwrap().id, id);
    }
    assert!(!cache.warming());
    assert!(Arc::ptr_eq(&first, &cache.get(1).unwrap()));
    assert!(cache.get(99).is_err());
    cache.warm_next();
    assert_eq!(cache.identity(), "zh-CN@16.20.1");
}
#[test]
fn newer_complete_bundle_beats_cache_without_mixing_partial_versions() {
    let cache = Temp::new();
    let bundled = Temp::new();
    cache.dataset("zh-CN", "16.9.3", true);
    bundled.dataset("zh-CN", "16.10.1", true);
    let roots = [cache.root(Source::Cache), bundled.root(Source::Bundled)];
    let prepared = prepare(&roots, Locale::ZhCn, Scope::Full).unwrap();
    assert_eq!(prepared.version(), "16.10.1");
    assert_eq!(prepared.source(), Source::Bundled);
    cache.dataset("zh-CN", "16.20.1", false);
    assert_eq!(
        prepare(&roots, Locale::ZhCn, Scope::Full)
            .unwrap()
            .version(),
        "16.10.1"
    );
    assert_eq!(
        prepare(&roots, Locale::ZhCn, Scope::Ocr).unwrap().version(),
        "16.20.1"
    );
}
#[test]
fn details_pin_displayed_version_across_pointer_updates_and_incomplete_downloads() {
    let bundled = Temp::new();
    let cache = Temp::new();
    for ver in ["16.20.1", "16.20.2"] {
        bundled.dataset("zh-CN", ver, true);
        bundled.write(
            &format!("versions/{ver}/manifest.json"),
            json!({"files":{
            "augments.json":{},"champions.json":{},"items.json":{},
            "champion-shards/index.json":{},"champion-shards/1.json":{}}}),
        );
        bundled.write(
            &format!("versions/{ver}/champion-shards/index.json"),
            json!({"shards":[{"path":"champion-shards/1.json","championIds":[25]}]}),
        );
        bundled.write(&format!("versions/{ver}/champion-shards/1.json"),
            json!({"champions":{"25":{"champion":{"name":"fixture"},"augments":[{"id":100,"name":"测试符文"}]}}}));
    }
    cache.dataset("zh-CN", "16.20.3", false);
    cache.write("versions/16.20.3/champions.json", json!({"data":[]}));
    let roots = [cache.root(Source::Cache), bundled.root(Source::Bundled)];
    assert_eq!(
        prepare(&roots, Locale::ZhCn, Scope::Champions)
            .unwrap()
            .version(),
        "16.20.3"
    );
    assert_eq!(
        prepare(&roots, Locale::ZhCn, Scope::Full)
            .unwrap()
            .version(),
        "16.20.2"
    );
    let mut old = fuze_data::detail_cache::Cache::prepare(&roots, "zh-CN@16.20.1").unwrap();
    assert_eq!(old.get(25).unwrap().version, "zh-CN@16.20.1");
    assert!(fuze_data::detail_cache::Cache::prepare(&roots, "zh-CN@../16.20.1").is_err());
}
#[test]
fn locale_identity_and_failed_prepare_cannot_replace_active_snapshot() {
    let cache = Temp::new();
    cache.dataset("zh-CN", "16.20.1", false);
    let roots = [cache.root(Source::Cache)];
    let mut active = Active::default();
    active
        .activate(prepare(&roots, Locale::ZhCn, Scope::Ocr).unwrap())
        .unwrap();
    let retained = active.snapshot().unwrap();
    assert_eq!(retained.catalog().get("测试符文"), Some(&100));
    assert!(prepare(&roots, Locale::EnUs, Scope::Ocr).is_err());
    assert_eq!(active.snapshot().unwrap().identity(), retained.identity());
    cache.dataset("en-US", "16.20.2", false);
    active
        .activate(prepare(&roots, Locale::EnUs, Scope::Ocr).unwrap())
        .unwrap();
    assert_eq!(active.snapshot().unwrap().identity(), "en-US@16.20.2");
    assert_eq!(retained.identity(), "zh-CN@16.20.1");
    cache.write(
        "versions/en-US/16.20.2/manifest.json",
        json!({"locale":"zh-CN","files":[]}),
    );
    assert!(prepare(&roots, Locale::EnUs, Scope::Ocr).is_err());
}
#[test]
fn rejects_windows_path_aliases_bad_json_sizes_and_ambiguous_catalog() {
    for bad in [
        "../secret",
        "/absolute",
        "C:/data",
        "a\\b",
        "a/CON.json",
        "a/lpt1.txt",
        "a. /b",
        "a:/b",
        "a/../b",
        "a\0b",
    ] {
        assert!(safe_path(bad).is_err(), "{bad:?}");
    }
    let cache = Temp::new();
    cache.dataset("zh-CN", "16.20.1", false);
    let roots = [cache.root(Source::Cache)];
    cache.write(
        "versions/16.20.1/manifest.json",
        json!({"files":[{"path":"augments.json","bytes":1}]}),
    );
    assert!(prepare(&roots, Locale::ZhCn, Scope::Ocr).is_err());
    cache.dataset("zh-CN", "16.20.1", false);
    cache.write(
        "versions/16.20.1/augments.json",
        json!([{"id":1,"name":"same name"},{"id":2,"name":"samename"}]),
    );
    assert!(prepare(&roots, Locale::ZhCn, Scope::Ocr).is_err());
}

#[test]
fn champion_scope_reads_only_selected_shard_and_preserves_missing_rates_and_origin() {
    let root = Temp::new();
    root.dataset("zh-CN", "16.20.1", false);
    root.write("versions/16.20.1/champion-shards/index.json",json!({"dataVersion":"16.20.1","shards":[{"path":"champion-shards/1.json","championIds":[8]},{"path":"champion-shards/missing.json","championIds":[9]}]}));
    root.write("versions/16.20.1/champion-shards/1.json",json!({"champions":{"8":{"augments":[
        {"id":100,"stats":{"winRate":0.6,"pickRate":0.12,"rank":1,"total":2,"source":"tencent","region":"CN","winRateSource":"client","winRateRegion":"WORLD","winRateGamePatch":"16.19"}},
        {"id":101,"stats":{"winRate":null,"pickRate":0.2,"rank":2,"total":2}},
        {"id":102,"stats":{"winRate":60,"pickRate":-1}}
    ]}}}));
    let roots = [root.root(Source::Bundled)];
    let data = prepare(&roots, Locale::ZhCn, Scope::Champion(8)).unwrap();
    let stats = data.recommendations(8).unwrap();
    assert_eq!(stats[&100].win_rate, Some(0.6));
    assert!((stats[&100].score.unwrap() - 0.384).abs() < 1e-9);
    assert_eq!(stats[&100].region, "WORLD");
    assert_eq!(stats[&100].patch, "16.19");
    assert_eq!(stats[&101].win_rate, None);
    assert_eq!(stats[&102].win_rate, None);
    assert_eq!(stats[&102].pick_rate, None);
    assert!(data.recommendations(9).is_err());
    assert!(prepare(&roots, Locale::ZhCn, Scope::Champion(9)).is_err());
    root.write(
        "versions/16.20.1/champion-shards/1.json",
        json!({"dataVersion":"16.19.1","champions":{}}),
    );
    assert!(prepare(&roots, Locale::ZhCn, Scope::Champion(8)).is_err());
}
