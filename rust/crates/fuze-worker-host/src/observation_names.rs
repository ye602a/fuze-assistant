//! Display names from the active locale/version only. No inference about reward ownership.
use crate::{history_detail, Result};
use fuze_data::{Locale, Prepared, Root, Scope};
use std::collections::HashMap;
pub struct Names {
    pub version: String,
    pub champions: HashMap<u32, String>,
    pub augments: HashMap<u32, String>,
}
impl Names {
    pub fn load(roots: &[Root], expected: &str) -> Result<Self> {
        let locale = match expected.split_once('@') {
            Some(("zh-CN", version)) if !version.is_empty() => Locale::ZhCn,
            Some(("zh-TW", version)) if !version.is_empty() => Locale::ZhTw,
            Some(("en-US", version)) if !version.is_empty() => Locale::EnUs,
            _ => return Err("observation-names-locale"),
        };
        let data = fuze_data::prepare(roots, locale, Scope::Champions)?;
        if data.identity() != expected {
            return Err("observation-names-version");
        }
        Ok(Self {
            version: data.identity(),
            champions: labels(&data, "champions.json"),
            augments: labels(&data, "augments.json"),
        })
    }
}
fn labels(data: &Prepared, file: &str) -> HashMap<u32, String> {
    let Some(doc) = data.document(file) else {
        return Default::default();
    };
    let doc = doc.get("data").unwrap_or(doc);
    let mut out = HashMap::new();
    if let Some(rows) = doc.as_array() {
        for row in rows.iter().take(10000) {
            let id = row["id"]
                .as_u64()
                .and_then(|id| u32::try_from(id).ok())
                .or_else(|| row["id"].as_str().and_then(|id| id.parse().ok()));
            if let Some(id) = id.filter(|id| *id > 0) {
                out.insert(id, history_detail::name(row, id));
            }
        }
    } else if let Some(rows) = doc.as_object() {
        for (id, row) in rows.iter().take(10000) {
            if let Ok(id) = id.parse::<u32>() {
                if id > 0 {
                    out.insert(id, history_detail::name(row, id));
                }
            }
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_context_names_require_exact_identity_without_unrelated_details() {
        let fixture = crate::preferences::locale_tests::Fixture::new();
        // Names require only augments/champions: missing equipment must not block them.
        for locale in ["zh-CN", "zh-TW", "en-US"] {
            fixture.dataset(locale, "1.0", false);
        }
        let roots = fixture.config().roots;
        for locale in ["zh-CN", "zh-TW", "en-US"] {
            let names = Names::load(&roots, &format!("{locale}@1.0")).unwrap();
            assert_eq!(names.augments[&1], format!("{locale} fixture"));
        }
        assert!(Names::load(&roots, "auto@1.0").is_err());
        assert!(Names::load(&roots, "en-US@").is_err());
        assert!(Names::load(&roots, "en-US@2.0").is_err());
        fixture.dataset("en-US", "2.0", false);
        assert!(Names::load(&roots, "en-US@1.0").is_err());
        assert!(Names::load(&roots, "en-US@2.0").is_ok());
    }
}
