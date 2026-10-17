//! Independent public recognition names. Popularity never becomes win rate or recommendation rank.
use crate::{Locale, Prepared, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[derive(PartialEq)]
pub struct Catalog {
    pub patch: String,
    pub date: String,
    rows: Vec<(String, [String; 3])>,
}
pub fn patch(value: &str) -> Option<(u32, u32)> {
    let mut v = value.split('.');
    let year = v.next()?.parse::<u32>().ok()?;
    let minor = v.next()?.parse::<u32>().ok()?;
    matches!(year, 16 | 26)
        .then_some((2026, minor))
        .filter(|(_, m)| *m < 100)
}
impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("recognition-limit");
        }
        let envelope: Value = serde_json::from_slice(bytes).map_err(|_| "recognition-format")?;
        let value = envelope.get("payload").unwrap_or(&envelope);
        if !value["license"]
            .as_str()
            .is_some_and(|s| s.starts_with("CC BY 4.0"))
            || value["licenseUrl"] != "https://creativecommons.org/licenses/by/4.0/"
        {
            return Err("recognition-license");
        }
        let p = value["meta"]["patch"]
            .as_str()
            .filter(|p| patch(p).is_some())
            .ok_or("recognition-patch")?;
        let date = value["meta"]["statsDate"]
            .as_str()
            .filter(|s| {
                s.len() == 10
                    && s.bytes().enumerate().all(|(i, c)| {
                        if i == 4 || i == 7 {
                            c == b'-'
                        } else {
                            c.is_ascii_digit()
                        }
                    })
            })
            .ok_or("recognition-date")?;
        let mut rows = Vec::new();
        let mut ids = HashSet::new();
        for row in value["rows"]
            .as_array()
            .filter(|r| (50..=2000).contains(&r.len()))
            .ok_or("recognition-rows")?
        {
            let id = row["augmentId"]
                .as_str()
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 100
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_!-".contains(&b))
                })
                .ok_or("recognition-id")?;
            if !ids.insert(id) {
                return Err("recognition-duplicate");
            }
            if !matches!(
                row["rarity"].as_str(),
                Some("silver" | "gold" | "prismatic")
            ) {
                return Err("recognition-rarity");
            }
            let mut names: [String; 3] = Default::default();
            for (i, locale) in ["zh-CN", "zh-TW", "en"].iter().enumerate() {
                names[i] = row["name"][locale]
                    .as_str()
                    .filter(|n| {
                        !n.trim().is_empty()
                            && n.chars().count() <= 160
                            && !n.chars().any(|c| c.is_control() || "<>".contains(c))
                    })
                    .ok_or("recognition-name")?
                    .trim()
                    .into();
            }
            rows.push((id.into(), names));
        }
        Ok(Self {
            patch: p.into(),
            date: date.into(),
            rows,
        })
    }
    pub fn bundled() -> Result<Self> {
        Self::parse(include_bytes!(
            "../../../../resources/recognition-catalog.json"
        ))
    }
    pub fn count(&self) -> usize {
        self.rows.len()
    }
    pub fn order(&self) -> ((u32, u32), &str) {
        (patch(&self.patch).unwrap_or_default(), &self.date)
    }
    pub fn merge(&self, base: &HashMap<String, u32>, locale: Locale) -> HashMap<String, u32> {
        let mut result = base.clone();
        let mut by_name: HashMap<String, HashSet<u32>> = HashMap::new();
        for (name, id) in base {
            by_name.entry(key(name)).or_default().insert(*id);
        }
        let i = match locale {
            Locale::ZhCn => 0,
            Locale::ZhTw => 1,
            Locale::EnUs => 2,
        };
        let mut occupied: HashSet<u32> = base.values().copied().collect();
        let mut provider_ids = HashMap::<u32, usize>::new();
        for (source, _) in &self.rows {
            *provider_ids.entry(private_id(source)).or_default() += 1;
        }
        let mut aliases = HashMap::<String, usize>::new();
        for (_, names) in &self.rows {
            *aliases.entry(key(&names[i])).or_default() += 1;
        }
        for (source, names) in &self.rows {
            let name = &names[i];
            if aliases[&key(name)] != 1 {
                continue;
            }
            let matches = by_name.get(&key(name));
            let id = match matches {
                Some(ids) if ids.len() == 1 => *ids.iter().next().unwrap(),
                Some(_) => continue,
                None => {
                    let id = private_id(source);
                    if provider_ids[&id] != 1 || !occupied.insert(id) {
                        continue;
                    }
                    id
                }
            };
            result.entry(name.clone()).or_insert(id);
        }
        result
    }
}
fn key(name: &str) -> String {
    name.chars()
        .filter_map(|c| {
            let c = if ('\u{ff01}'..='\u{ff5e}').contains(&c) {
                char::from_u32(c as u32 - 0xfee0).unwrap()
            } else {
                c
            };
            (!c.is_whitespace() && !"：:·.,，。！!？?-".contains(c)).then_some(c)
        })
        .flat_map(char::to_lowercase)
        .collect()
}
// A stable, local-only namespace, never a provider/statistics ID. Collisions are rejected above.
fn private_id(source: &str) -> u32 {
    source
        .bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
        | 0x80000000
}
impl Prepared {
    pub fn supplement_recognition(&mut self, catalog: &Catalog) {
        self.catalog = catalog.merge(&self.catalog, self.locale);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supplemental_names_never_reuse_unrelated_statistics_or_popularity() {
        let c = Catalog::bundled().unwrap();
        assert!(c.count() >= 50);
        let first = &c.rows[0];
        let base = HashMap::from([(first.1[0].clone(), 7), ("unrelated".into(), 9)]);
        let merged = c.merge(&base, Locale::ZhCn);
        assert_eq!(merged[&first.1[0]], 7);
        let second = &c.rows[1];
        assert!(merged[&second.1[0]] >= 0x80000000);
        assert_ne!(merged[&second.1[0]], 9);
        assert_eq!(merged, c.merge(&base, Locale::ZhCn));
        assert!(Catalog::parse(b"{}").is_err());
    }
}
