//! Public names + Riot public patch reference, loaded once per preparation, never per OCR frame.
use crate::Result;
use std::path::Path;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub patch: String,
    pub date: String,
    pub count: usize,
    pub reference: String,
}
pub fn load(roots: &[fuze_data::Root]) -> Result<(fuze_data::recognition::Catalog, Summary)> {
    let mut chosen = fuze_data::recognition::Catalog::bundled()?;
    let mut reference = String::new();
    for root in roots
        .iter()
        .filter(|r| r.source == fuze_data::Source::Cache)
    {
        let Some(parent) = root.path.parent() else {
            continue;
        };
        let dir = parent.join("recognition-data");
        if let Ok(c) = crate::update_channel::bounded(&dir.join("catalog.json"), 2 * 1024 * 1024)
            .and_then(|b| fuze_data::recognition::Catalog::parse(&b))
        {
            if c.order() >= chosen.order() {
                chosen = c;
            }
        }
        if let Ok(bytes) = crate::update_channel::bounded(&dir.join("reference.json"), 4096) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if v["source"] != "riot-ddragon" {
                    continue;
                }
                if let Some(p) = v["patch"]
                    .as_str()
                    .filter(|p| fuze_data::recognition::patch(p).is_some())
                {
                    reference = p.into();
                }
            }
        }
    }
    let summary = Summary {
        patch: chosen.patch.clone(),
        date: chosen.date.clone(),
        count: chosen.count(),
        reference,
    };
    Ok((chosen, summary))
}
pub fn refresh(cache: &Path, cancel: &dyn Fn() -> bool) -> Result<()> {
    prepare_refresh(cache, cancel)?.apply(cache)
}
pub(crate) struct Prepared {
    catalog: Result<Option<serde_json::Value>>,
    reference: Result<Option<serde_json::Value>>,
}
impl Prepared {
    pub fn changed(&self) -> bool {
        self.catalog.as_ref().is_ok_and(|v| v.is_some())
            || self.reference.as_ref().is_ok_and(|v| v.is_some())
    }
    pub fn apply(self, cache: &Path) -> Result<()> {
        let dir = cache
            .parent()
            .ok_or("recognition-root")?
            .join("recognition-data");
        let save = |result: Result<Option<serde_json::Value>>, name: &str| {
            result.and_then(|value| {
                value.map_or(Ok(()), |v| {
                    crate::preferences::save_json(&dir.join(name), &v)
                })
            })
        };
        let catalog = save(self.catalog, "catalog.json");
        let reference = save(self.reference, "reference.json");
        catalog.and(reference)
    }
}
pub(crate) fn prepare_refresh(cache: &Path, cancel: &dyn Fn() -> bool) -> Result<Prepared> {
    let dir = cache
        .parent()
        .ok_or("recognition-root")?
        .join("recognition-data");
    let catalog = (|| {
        let bytes = crate::update_io::get(
            "arammayhem.com",
            "/data/v1/latest/augments.json",
            2 * 1024 * 1024,
            None,
            None,
            cancel,
        )?;
        let candidate = fuze_data::recognition::Catalog::parse(&bytes)?;
        let old = load(&[fuze_data::Root {
            path: cache.to_owned(),
            source: fuze_data::Source::Cache,
        }])?
        .0;
        if candidate.order() < old.order() {
            return Err("recognition-older");
        }
        if cancel() {
            return Err("recognition-cancelled");
        }
        if candidate == old {
            return Ok(None);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "recognition-format")?;
        Ok(Some(value))
    })();
    if cancel() {
        return Err("recognition-cancelled");
    }
    let reference = (|| {
        let bytes = crate::update_io::get(
            "ddragon.leagueoflegends.com",
            "/api/versions.json",
            2 * 1024 * 1024,
            None,
            None,
            cancel,
        )?;
        let v: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "recognition-reference")?;
        let patch = v[0]
            .as_str()
            .filter(|p| fuze_data::recognition::patch(p).is_some())
            .ok_or("recognition-reference")?;
        if cancel() {
            return Err("recognition-cancelled");
        }
        let value = serde_json::json!({"patch":patch,"source":"riot-ddragon"});
        let old = crate::update_channel::bounded(&dir.join("reference.json"), 4096)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        Ok((old.as_ref() != Some(&value)).then_some(value))
    })();
    Ok(Prepared { catalog, reference })
}

#[cfg(test)]
mod tests {
    #[test]
    fn legacy_reference_is_not_relabelled_as_a_riot_patch() {
        let root =
            std::env::temp_dir().join(format!("fuze-reference-source-{}", std::process::id()));
        let dir = root.join("recognition-data");
        std::fs::create_dir_all(&dir).unwrap();
        let roots = [fuze_data::Root {
            path: root.join("client-data"),
            source: fuze_data::Source::Cache,
        }];
        std::fs::write(dir.join("reference.json"), r#"{"patch":"16.21"}"#).unwrap();
        assert!(super::load(&roots).unwrap().1.reference.is_empty());
        std::fs::write(
            dir.join("reference.json"),
            r#"{"patch":"16.22.1","source":"riot-ddragon"}"#,
        )
        .unwrap();
        assert_eq!(super::load(&roots).unwrap().1.reference, "16.22.1");
        std::fs::remove_dir_all(root).unwrap();
    }
}
