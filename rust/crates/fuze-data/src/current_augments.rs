//! Versioned client metadata, never a source of win rates. Historical pinned
//! preparations stay untouched; current Chinese views opt in explicitly.
use crate::{recognition::patch, Locale, Prepared, Result, Scope};
use serde_json::Value;
use std::collections::HashMap;

pub const PATCH: &str = "16.21";
pub const IDENTITY_SUFFIX: &str = "-augments16.21";
/// Reload exactly the same statistics source and metadata view in UI workers.
pub fn prepare_identity(
    roots: &[crate::Root],
    locale: Locale,
    scope: Scope,
    identity: &str,
) -> Result<Prepared> {
    let version = identity.split_once('@').ok_or("detail-version-mismatch")?.1;
    let (version, current) = version
        .strip_suffix(IDENTITY_SUFFIX)
        .map_or((version, false), |v| (v, true));
    let mut data = crate::prepare_version(roots, locale, scope, Some(version))?;
    if current {
        data.apply_current_augments()?;
    }
    if data.identity() != identity {
        return Err("detail-version-mismatch");
    }
    Ok(data)
}
pub fn metadata() -> Result<Value> {
    serde_json::from_slice(include_bytes!(
        "../../../../resources/augments-16.21-zh-CN.json"
    ))
    .map_err(|_| "augment-metadata")
}
impl Prepared {
    pub fn apply_current_augments(&mut self) -> Result<()> {
        if self.current_augments || self.locale != Locale::ZhCn || self.scope == Scope::History {
            return Ok(());
        }
        let Some(version) = patch(&self.version) else {
            return Ok(());
        };
        if version > patch(PATCH).unwrap() {
            return Ok(());
        }
        let data = metadata()?;
        let rows = data["data"].as_array().ok_or("augment-metadata")?;
        let active: HashMap<u64, &Value> = rows
            .iter()
            .filter_map(|r| r["id"].as_u64().map(|id| (id, r)))
            .collect();
        let same_patch = version == patch(PATCH).unwrap();
        for (path, document) in &mut self.documents {
            if path.starts_with("champion-shards/") {
                if let Some(champions) = document["champions"].as_object_mut() {
                    for champion in champions.values_mut() {
                        if let Some(augments) = champion["augments"].as_array_mut() {
                            augments.retain(|a| {
                                a["enabled"] != false
                                    && a["id"].as_u64().is_some_and(|id| active.contains_key(&id))
                            });
                            for a in augments {
                                let id = a["id"].as_u64().unwrap();
                                // Require an explicit matching statistics patch, too.
                                let stats_match = same_patch
                                    && a["stats"]["gamePatch"].as_str().and_then(patch)
                                        == patch(PATCH)
                                    && a["stats"]
                                        .get("winRateGamePatch")
                                        .is_none_or(|p| p.as_str().and_then(patch) == patch(PATCH));
                                let stats = if stats_match {
                                    a["stats"].clone()
                                } else {
                                    Value::Null
                                };
                                *a = active[&id].clone();
                                a["stats"] = stats;
                            }
                        }
                    }
                }
            }
        }
        // Metadata contains no global or champion statistics. Keep the original
        // source version; identity separately records this presentation overlay.
        self.documents.insert("augments.json".into(), data);
        self.catalog = super::catalog(&self.documents["augments.json"])?;
        self.current_augments = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn fixture(version: &str) -> Prepared {
        Prepared {
            locale: Locale::ZhCn,
            version: version.into(),
            source: crate::Source::Cache,
            scope: Scope::Detail(1),
            current_augments: false,
            catalog: HashMap::new(),
            documents: BTreeMap::from([(
                "champion-shards/test.json".into(),
                serde_json::json!({
                    "champions":{"1":{"augments":[
                        {"id":3051,"stats":{"gamePatch":version,"winRate":0.99,"tier":1}},
                        {"id":2095,"stats":{"gamePatch":version,"winRate":0.99}}
                    ]}}
                }),
            )]),
        }
    }
    #[test]
    fn current_pool_has_new_metadata_without_old_statistics_or_retired_names() {
        let mut d = fixture("16.20.4");
        d.apply_current_augments().unwrap();
        assert_eq!(d.version(), "16.20.4");
        assert!(d.identity().ends_with(IDENTITY_SUFFIX));
        assert_eq!(d.catalog["蓝色屏障"], 3051);
        assert!(!d.catalog.contains_key("掷骰狂人"));
        assert!(!d.catalog.contains_key("力量"));
        for id in [
            2128, 2024, 1356, 2063, 2031, 2006, 1220, 2042, 2095, 2082, 2046, 1345, 2118,
        ] {
            assert!(!d.catalog.values().any(|v| *v == id), "retired {id}");
        }
        let a = &d.documents["champion-shards/test.json"]["champions"]["1"]["augments"];
        assert_eq!(a.as_array().unwrap().len(), 1);
        assert!(a[0]["stats"].is_null());
        assert!(d
            .recommendations(1)
            .unwrap()
            .values()
            .all(|r| r.score.is_none()));
        let before = d.catalog.clone();
        d.supplement_recognition(&crate::recognition::Catalog::bundled().unwrap());
        assert_eq!(before, d.catalog);
        assert!(d
            .recognition_cards()
            .iter()
            .all(|a| !a.icon.is_empty() && !a.description.is_empty()));
    }
    #[test]
    fn same_patch_statistics_survive_and_future_or_history_is_not_rewritten() {
        let mut d = fixture("16.21");
        d.apply_current_augments().unwrap();
        assert_eq!(
            d.documents["champion-shards/test.json"]["champions"]["1"]["augments"][0]["stats"]
                ["winRate"],
            0.99
        );
        for (version, scope) in [("16.22", Scope::Detail(1)), ("16.20.4", Scope::History)] {
            let mut d = fixture(version);
            d.scope = scope;
            d.apply_current_augments().unwrap();
            assert!(!d.current_augments);
        }
    }
    #[test]
    fn missing_or_conflicting_statistics_patch_never_inherits_current_patch() {
        for stats in [
            serde_json::json!({"winRate":0.9,"tier":1}),
            serde_json::json!({"gamePatch":"16.21","winRateGamePatch":"16.20","winRate":0.9,"rank":1}),
        ] {
            let mut d = fixture("16.21.1");
            d.documents.get_mut("champion-shards/test.json").unwrap()["champions"]["1"]
                ["augments"][0]["stats"] = stats;
            d.apply_current_augments().unwrap();
            let r = d.recommendations(1).unwrap();
            assert!(r[&3051].win_rate.is_none() && r[&3051].score.is_none());
        }
    }
}
