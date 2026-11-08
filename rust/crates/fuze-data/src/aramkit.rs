//! Public ARAMKit snapshots -> the existing FUZE data model. No network or game access.
use crate::{Locale, Result};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
pub const SOURCE: &str = "ARAMKit";
pub const HOST: &str = "data.aramkit.cn";
#[derive(Clone, Debug)]
pub struct Release {
    pub patch: String,
    pub date: String,
    pub data: String,
    pub resources: String,
    pub version: String,
}
impl Release {
    pub fn parse(v: &Value) -> Result<Self> {
        let patch = v["latest"]
            .as_str()
            .filter(|s| {
                s.split('.').count() == 2
                    && s.split('.')
                        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            })
            .ok_or("aramkit-version")?;
        let versions = v["versions"]
            .as_array()
            .filter(|a| a.len() <= 100)
            .ok_or("aramkit-version")?;
        let mut found = versions.iter().filter(|r| r["version"] == patch);
        let r = found.next().ok_or("aramkit-version")?;
        if found.next().is_some() {
            return Err("aramkit-version");
        }
        let path = |field: &str, prefix: &str| -> Result<String> {
            let s = r[field].as_str().ok_or("aramkit-path")?;
            crate::safe_path(s)?;
            if !s.starts_with(&format!("{prefix}/{patch}-"))
                || s.split('/').count() != 2
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/.-".contains(&b))
            {
                return Err("aramkit-path");
            }
            Ok(s.into())
        };
        let date = r["dataDate"]
            .as_str()
            .filter(|s| {
                s.len() == 10
                    && s.bytes().enumerate().all(|(i, b)| {
                        if i == 4 || i == 7 {
                            b == b'-'
                        } else {
                            b.is_ascii_digit()
                        }
                    })
            })
            .ok_or("aramkit-date")?;
        let revision = r["buildTimeUnixMs"].as_u64().ok_or("aramkit-version")?;
        Ok(Self {
            patch: patch.into(),
            date: date.into(),
            data: path("dataPath", "data")?,
            resources: path("resourcePath", "resources")?,
            version: format!("{patch}.{revision}"),
        })
    }
    pub fn resource(&self, locale: Locale, file: &str) -> String {
        format!(
            "/{}/{}/resources/{file}.json",
            self.resources,
            locale.code()
        )
    }
    pub fn stats(&self, file: &str) -> String {
        format!("/{}/stats/all/{file}.json", self.data)
    }
    pub fn wrap(&self, locale: Locale, data: Value) -> Value {
        json!({"source":SOURCE,"locale":locale.code(),"gamePatch":self.patch,"dataVersion":self.version,"dataDate":self.date,"data":data})
    }
    pub fn stats_row(&self, r: &Value) -> Result<Value> {
        for k in ["winRate", "pickRate"] {
            if !r[k].is_null()
                && !r[k]
                    .as_f64()
                    .is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v))
            {
                return Err("aramkit-rate");
            }
        }
        let games = r["sampleCount"].as_u64().ok_or("aramkit-samples")?;
        Ok(
            json!({"source":SOURCE,"region":"地区未标明","gamePatch":self.patch,"date":self.date,"games":games,"winRate":r["winRate"],"pickRate":r["pickRate"],"rank":r["rank"],"tier":match r["tier"].as_str(){Some("S")=>Some(1),Some("A")=>Some(2),Some("B")=>Some(3),Some("C")=>Some(4),Some("D")=>Some(5),_=>None}}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Release, Value) {
        let release=Release::parse(&json!({"latest":"16.22","versions":[{"version":"16.22","dataPath":"data/16.22-20261106-test","resourcePath":"resources/16.22-test","dataDate":"2026-11-06","buildTimeUnixMs":1}]})).unwrap();
        (
            release,
            serde_json::from_str(include_str!(
                "../../../../tests/fixtures/aramkit/fixture.json"
            ))
            .unwrap(),
        )
    }
    #[test]
    fn identity_names_unknown_statistics_and_purchase_routes() {
        let (release, mut f) = fixture();
        f["augments"]["3171"] = f["augments"]["1002"].clone();
        f["augments"]["3171"]["name"] = json!("力量");
        let c = Catalog::new(
            release,
            Locale::ZhCn,
            f["champions"].clone(),
            f["augments"].clone(),
            f["items"].clone(),
            f["summoner-spells"].clone(),
            f["champion-rankings"].clone(),
            f["augment-rankings"].clone(),
        )
        .unwrap();
        assert_eq!(c.champions[&145]["name"], "卡莎");
        assert!(c.augments[&1002]["stats"].is_null());
        assert!(!c.augments.contains_key(&3171));
        let d = c.detail(145, &f["detail"]).unwrap();
        let b = &d["champions"]["145"]["builds"][0];
        assert_eq!(b["coreItems"][0]["itemIds"], json!([3040, 1001]));
        assert_eq!(b["coreItems"][0]["importItemIds"], json!([3003, 1001]));
        assert_eq!(b["boots"], json!([]));
        assert_eq!(b["summonerSpells"][0]["summonerSpellIds"], json!([4]));
        let sets = crate::item_sets::create(145, "卡莎", &[b]);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0]["associatedMaps"], json!([12]));
        assert_eq!(
            sets[0]["blocks"][1]["items"],
            json!([{"id":"3003","count":1},{"id":"1001","count":1}])
        );
        assert!(icon("assets/champion/../145.webp").is_err());
        assert!(c.detail(17, &f["detail"]).is_err());
        let mut wrong = f["detail"].clone();
        wrong["augments"]["all"][0]["winRate"] = json!(1.1);
        assert!(c.detail(145, &wrong).is_err());
    }
    #[test]
    #[ignore = "explicit offline public-data review directory"]
    fn public_snapshot_all_champions() {
        let root = std::path::PathBuf::from(
            std::env::var_os("FUZE_ARAMKIT_REVIEW").expect("review directory"),
        );
        let read = |name: &str| -> Value {
            serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap())
                .unwrap()
        };
        let c = Catalog::new(
            Release::parse(&read("aramkit-versions")).unwrap(),
            Locale::ZhCn,
            read("aramkit-champions"),
            read("aramkit-augments"),
            read("aramkit-items"),
            read("aramkit-summoner-spells"),
            read("aramkit-champion-rankings"),
            read("aramkit-augment-rankings"),
        )
        .unwrap();
        let mut imported = 0;
        for id in c.champions.keys() {
            let d = c
                .detail(*id, &read(&format!("aramkit-detail-{id}")))
                .unwrap();
            let builds = d["champions"][id.to_string()]["builds"]
                .as_array()
                .unwrap()
                .iter()
                .collect::<Vec<_>>();
            imported += !crate::item_sets::create(*id, "fixture", &builds).is_empty() as usize;
        }
        eprintln!(
            "{} champion details converted, {imported} have importable routes",
            c.champions.len()
        );
        assert!(imported > 0);
    }
}
pub fn icon(path: &str) -> Result<String> {
    let mut parts = path.split('/');
    if parts.next() != Some("assets")
        || !matches!(
            parts.next(),
            Some("champion" | "augment" | "item" | "summoner" | "tooltip")
        )
    {
        return Err("aramkit-icon");
    }
    let file = parts.next().ok_or("aramkit-icon")?;
    if parts.next().is_some()
        || !file.ends_with(".webp")
        || file.len() > 100
        || !file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_- .".contains(&b))
        || file.contains("..")
        || file.contains(' ')
    {
        return Err("aramkit-icon");
    }
    Ok(format!("https://{HOST}/{path}"))
}
fn text(v: &Value) -> String {
    v["blocks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| {
            b["spans"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|s| s["type"] == "text")
                .filter_map(|s| s["text"].as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn resources(v: &Value) -> Result<&Map<String, Value>> {
    let m = v
        .as_object()
        .filter(|m| !m.is_empty() && m.len() <= 2000)
        .ok_or("aramkit-resources")?;
    for (id, row) in m {
        if id.parse::<u32>().is_err()
            || !row["name"]
                .as_str()
                .is_some_and(|n| !n.trim().is_empty() && n.chars().count() <= 160)
        {
            return Err("aramkit-resources");
        }
        icon(row["icon"].as_str().ok_or("aramkit-icon")?)?;
    }
    Ok(m)
}
fn ranks(v: &Value) -> Result<BTreeMap<u32, &Value>> {
    let mut map = BTreeMap::new();
    for r in v
        .as_array()
        .filter(|a| a.len() <= 2000)
        .ok_or("aramkit-ranks")?
    {
        let id = r["id"]
            .as_u64()
            .filter(|id| *id > 0 && *id <= u32::MAX as u64)
            .ok_or("aramkit-id")? as u32;
        if map.insert(id, r).is_some() {
            return Err("aramkit-duplicate");
        }
    }
    Ok(map)
}
pub struct Catalog {
    pub release: Release,
    pub locale: Locale,
    pub champions: BTreeMap<u32, Value>,
    pub augments: BTreeMap<u32, Value>,
    pub items: BTreeMap<u32, Value>,
    pub spells: BTreeMap<u32, String>,
}
impl Catalog {
    pub fn new(
        release: Release,
        locale: Locale,
        heroes: Value,
        augments: Value,
        items: Value,
        spells: Value,
        hero_ranks: Value,
        augment_ranks: Value,
    ) -> Result<Self> {
        let mut out = Self {
            release,
            locale,
            champions: BTreeMap::new(),
            augments: BTreeMap::new(),
            items: BTreeMap::new(),
            spells: BTreeMap::new(),
        };
        let hr = ranks(&hero_ranks["rows"])?;
        let ar = ranks(&augment_ranks["all"])?;
        for (id, r) in resources(&heroes)? {
            let id = id.parse::<u32>().unwrap();
            let name = r["title"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("aramkit-champion-name")?;
            out.champions.insert(id,json!({"id":id,"alias":r["riotId"],"name":name,"title":r["name"],"roles":r["roles"],"iconUrl":icon(r["icon"].as_str().unwrap())?,"stats":hr.get(&id).filter(|s|s["hasData"]!=false).map(|s|out.release.stats_row(s)).transpose()?}));
        }
        for (id, r) in resources(&augments)? {
            let id = id.parse::<u32>().unwrap();
            // Pentakill ability choices have their own recognition and display path
            // (fuze-contract::skill_choice), not ordinary augment statistics.
            if (3170..=3173).contains(&id) {
                continue;
            }
            let rarity = match r["rarity"].as_str() {
                Some("silver") => 0,
                Some("gold") => 1,
                Some("prismatic") => 2,
                _ => return Err("aramkit-rarity"),
            };
            out.augments.insert(id,json!({"id":id,"name":r["name"],"rarity":rarity,"rarityName":r["rarity"],"iconUrl":icon(r["icon"].as_str().unwrap())?,"description":text(&r["tooltip"]),"enabled":true,"stats":ar.get(&id).filter(|s|s["hasData"]!=false).map(|s|out.release.stats_row(s)).transpose()?}));
        }
        for id in hr.keys() {
            if !out.champions.contains_key(id) {
                return Err("aramkit-champion-resource");
            }
        }
        for id in ar.keys() {
            if !(3170..=3173).contains(id) && !out.augments.contains_key(id) {
                return Err("aramkit-augment-resource");
            }
        }
        for (id, r) in resources(&items)? {
            let id = id.parse::<u32>().unwrap();
            let mut item = r.clone();
            item["id"] = json!(id);
            item["iconUrl"] = json!(icon(r["icon"].as_str().unwrap())?);
            item["description"] = json!(text(&r["tooltip"]));
            item["inStore"] = r["purchasable"].clone();
            out.items.insert(id, item);
        }
        for (id, r) in resources(&spells)? {
            out.spells
                .insert(id.parse().unwrap(), icon(r["icon"].as_str().unwrap())?);
        }
        Ok(out)
    }
    pub fn documents(&self) -> BTreeMap<String, Value> {
        let mut d = BTreeMap::new();
        for (name, rows) in [
            ("champions.json", &self.champions),
            ("augments.json", &self.augments),
            ("items.json", &self.items),
        ] {
            d.insert(
                name.into(),
                self.release
                    .wrap(self.locale, json!(rows.values().collect::<Vec<_>>())),
            );
        }
        d.insert("champion-shards/index.json".into(),json!({"dataVersion":self.release.version,"shards":self.champions.keys().map(|id|json!({"path":format!("champion-shards/{id}.json"),"championIds":[id]})).collect::<Vec<_>>()}));
        d
    }
    pub fn detail(&self, id: u32, v: &Value) -> Result<Value> {
        if v["champion"]["id"] != id {
            return Err("aramkit-champion");
        }
        let mut hero = self
            .champions
            .get(&id)
            .ok_or("aramkit-champion-resource")?
            .clone();
        hero["stats"] = if v["champion"]["stats"]["hasData"] == false {
            Value::Null
        } else {
            self.release.stats_row(&v["champion"]["stats"])?
        };
        let stats = ranks(&v["augments"]["all"])?;
        for id in stats.keys() {
            if !(3170..=3173).contains(id) && !self.augments.contains_key(id) {
                return Err("aramkit-augment-resource");
            }
        }
        let augments = self
            .augments
            .iter()
            .map(|(id, a)| {
                let mut a = a.clone();
                a["stats"] = stats
                    .get(id)
                    .filter(|r| r["hasData"] != false)
                    .map(|r| self.release.stats_row(r))
                    .transpose()?
                    .unwrap_or(Value::Null);
                Ok(a)
            })
            .collect::<Result<Vec<_>>>()?;
        let builds = self.builds(v)?;
        Ok(
            json!({"dataVersion":self.release.version,"champions":{id.to_string():{"champion":hero,"augments":augments,"builds":builds,"aramkitReference":{"patch":self.release.patch,"date":self.release.date,"dataPath":self.release.data,"augments":v["augments"],"augmentCombinations":v["augmentCombinations"]}}}}),
        )
    }
    fn purchase(&self, id: u32, visited: &mut HashSet<u32>) -> Option<u32> {
        if id == 0 || !visited.insert(id) {
            return None;
        }
        let i = self.items.get(&id)?;
        match i["acquisition"].as_str() {
            Some("shop") if i["purchasable"] == true => Some(id),
            Some("transform") => {
                let from = i["from"].as_array()?;
                if from.len() != 1 {
                    return None;
                }
                self.purchase(from[0].as_str()?.parse().ok()?, visited)
            }
            _ => None,
        }
    }
    fn item_row(&self, r: &Value, ids: &Value) -> Result<Option<Value>> {
        let list = ids.as_array().ok_or("aramkit-build-items")?;
        let mut original = Vec::new();
        let mut imported = Vec::new();
        for item in list {
            let id = item["id"]
                .as_u64()
                .filter(|id| *id <= u32::MAX as u64)
                .ok_or("aramkit-item")? as u32;
            if !self.items.contains_key(&id) {
                return Err("aramkit-item-resource");
            }
            let Some(purchase) = self.purchase(id, &mut HashSet::new()) else {
                return Ok(None);
            };
            original.push(id);
            imported.push(purchase);
        }
        if original.is_empty() {
            return Ok(None);
        }
        let mut row = self.release.stats_row(r)?;
        row["itemIds"] = json!(original);
        row["importItemIds"] = json!(imported);
        Ok(Some(row))
    }
    fn builds(&self, v: &Value) -> Result<Vec<Value>> {
        let Some(archetypes) = v["builds"]["filtered"]["archetypes"].as_array() else {
            return Ok(vec![]);
        };
        let mut result = Vec::new();
        // Use the provider's normal (augment-independent) builds, never combine with unfiltered routes.
        for archetype in archetypes.iter().take(6) {
            for profile in archetype["profiles"]
                .as_array()
                .into_iter()
                .flatten()
                .take(6)
            {
                let Some(routes) = profile["routes"].as_array() else {
                    continue;
                };
                let mut routes: Vec<_> = routes.iter().collect();
                routes.sort_by_key(|r| std::cmp::Reverse(r["sampleCount"].as_u64().unwrap_or(0)));
                for route in routes {
                    let Some(core) = self.item_row(route, &route["purchaseOrder"])? else {
                        continue;
                    };
                    let mut build = json!({"source":SOURCE,"queueId":2400,"patch":self.release.patch,"date":self.release.date,"stats":self.release.stats_row(route)?,"tags":{"style":archetype["key"]},"coreItems":[core],"startingItems":[],"boots":[],"situationalItems":[]});
                    for (field, target, single) in [
                        ("starters", "startingItems", false),
                        ("boots", "boots", true),
                        ("laterItems", "situationalItems", true),
                    ] {
                        for r in route[field].as_array().into_iter().flatten().take(30) {
                            let ids = if single {
                                json!([r["item"]])
                            } else {
                                r["items"].clone()
                            };
                            if let Some(row) = self.item_row(r, &ids)? {
                                build[target].as_array_mut().unwrap().push(row);
                            }
                        }
                    }
                    build["summonerSpells"] = json!(v["summoners"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .take(3)
                        .map(|r| {
                            let mut row = self.release.stats_row(r)?;
                            row["summonerSpellIds"] = json!(r["spells"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|s| s["id"].as_u64())
                                .collect::<Vec<_>>());
                            row["icons"] = json!(r["spells"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|s| s["id"].as_u64())
                                .filter_map(|id| self.spells.get(&(id as u32)))
                                .collect::<Vec<_>>());
                            Ok(row)
                        })
                        .collect::<Result<Vec<_>>>()?);
                    build["skillPriorities"] = v["skills"].clone();
                    result.push(build);
                    break;
                }
                if result.len() >= 4 {
                    return Ok(result);
                }
            }
        }
        Ok(result)
    }
}
