//! Synthetic catalogues only; never inspect player settings, game or network.
use super::*;
use fuze_data::{Locale, Root, Source};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(crate) struct Fixture(pub PathBuf);
impl Fixture {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fuze-locale-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn write(&self, path: &str, document: Value) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
    }
    pub fn dataset(&self, locale: &str, version: &str, full: bool) {
        let prefix = if locale == "zh-CN" {
            format!("versions/{version}")
        } else {
            format!("versions/{locale}/{version}")
        };
        let pointer = if locale == "zh-CN" {
            "current.json".into()
        } else {
            format!("current.{locale}.json")
        };
        self.write(
            &pointer,
            json!({"schemaVersion":3,"locale":locale,"dataVersion":version}),
        );
        let files = [
            "augments.json",
            "champions.json",
            "items.json",
            "champion-shards/index.json",
            "champion-shards/fixture.json",
        ];
        self.write(
            &format!("{prefix}/manifest.json"),
            json!({"locale":locale,"dataVersion":version,
            "files":files.iter().map(|path|json!({"path":path})).collect::<Vec<_>>()}),
        );
        for file in files {
            if !full && file == "items.json" {
                continue;
            }
            let document = match file {
                "augments.json" => json!({"data":[{"id":1,"name":format!("{locale} fixture")}]}),
                "champions.json" => json!({"data":[{"id":1,"name":"fixture"}]}),
                "champion-shards/index.json" => {
                    json!({"shards":[{"path":"champion-shards/fixture.json","championIds":[1]}]})
                }
                "champion-shards/fixture.json" => json!({"champions":{"1":{"augments":[]}}}),
                _ => json!({"data":[]}),
            };
            let mut document = document;
            document["locale"] = locale.into();
            document["dataVersion"] = version.into();
            self.write(&format!("{prefix}/{file}"), document);
        }
    }
    pub fn config(&self) -> Config {
        Config {
            roots: vec![Root {
                path: self.0.clone(),
                source: Source::Bundled,
            }],
            locale: Some(Locale::ZhCn),
            icon_downloads: false,
            level_gate: true,
            augment_overlay: false,
            worker: Default::default(),
            ort: Default::default(),
            model: Default::default(),
            dictionary: Default::default(),
            backend: "cpu".into(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if fs::canonicalize(&self.0).ok().as_ref() == Some(&self.0)
            && self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("fuze-locale-")
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
#[test]
fn locale_activation_preserves_memory_and_disk_until_complete_and_saved() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    fixture.dataset("zh-CN", "1.0", true);
    fixture.dataset("en-US", "1.0", false);
    let mut config = fixture.config();
    let mut current = Preferences {
        locale: "zh-CN".into(),
        backend: "cpu".into(),
        ..Default::default()
    };
    let original = current.clone();
    let path = fixture.0.join("settings.json");
    save(&path, &current).unwrap();
    let candidate = Preferences {
        locale: "en-US".into(),
        backend: "dml".into(),
        ..current.clone()
    };
    assert!(commit_change(&mut current, &mut config, candidate.clone(), Some(&path)).is_err());
    assert_eq!(current, original);
    assert_eq!(read(&path).unwrap(), original);
    assert_eq!(config.locale, Some(Locale::ZhCn));
    assert_eq!(config.backend, "cpu");
    fixture.dataset("en-US", "1.0", true);
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    assert!(commit_change(&mut current, &mut config, candidate.clone(), Some(&path)).is_err());
    assert_eq!(current, original);
    assert_eq!(read(&path).unwrap(), original);
    assert_eq!(config.locale, Some(Locale::ZhCn));
    drop(lock);
    assert!(commit_change(&mut current, &mut config, candidate.clone(), Some(&path)).unwrap());
    assert_eq!(current, candidate);
    assert_eq!(read(&path).unwrap(), candidate);
    assert_eq!(config.locale, Some(Locale::EnUs));
    assert_eq!(config.backend, "dml");
    // UI-only preference changes must not load or validate catalogues again.
    config.roots.clear();
    let candidate = Preferences {
        champ_topmost: !current.champ_topmost,
        ..current.clone()
    };
    assert!(!commit_change(&mut current, &mut config, candidate, Some(&path)).unwrap());
}
#[test]
fn locale_activation_rejects_newer_partial_cache_and_unlisted_shards() {
    let bundle = Fixture::new();
    let cache = Fixture::new();
    bundle.dataset("en-US", "1.0", true);
    cache.dataset("en-US", "2.0", false);
    let mut config = bundle.config();
    config.roots.push(Root {
        path: cache.0.clone(),
        source: Source::Cache,
    });
    let mut current = Preferences::default();
    let candidate = Preferences {
        locale: "en-US".into(),
        ..current.clone()
    };
    assert_eq!(
        commit_change(&mut current, &mut config, candidate.clone(), None),
        Err("preferences-locale-incomplete")
    );
    cache.dataset("en-US", "2.0", true);
    cache.write(
        "versions/en-US/2.0/champion-shards/index.json",
        json!({"locale":"en-US","dataVersion":"2.0",
        "shards":[{"path":"champion-shards/missing.json","championIds":[1]}]}),
    );
    assert_eq!(
        commit_change(&mut current, &mut config, candidate, None),
        Err("preferences-locale-incomplete")
    );
    assert_eq!(current.locale, "auto");
    assert_eq!(config.locale, Some(Locale::ZhCn));
}
