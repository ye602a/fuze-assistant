//! Public 101 Mayhem detail adapter. Rankings never supply champion win rates.
use crate::{Prepared, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub const SOURCE: &str = "tencent-101";
pub const CATALOG_PATCH: &str = "16.21";
pub const DETAIL_PATH: &str = "/go/battle_info/odp_proxy/fuwen_hero_rank?championid=";
pub const LIMIT: usize = 512 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rank {
    pub id: u32,
    pub rank: u32,
    pub pick: f64,
    pub tier: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub champion: u32,
    pub date: String,
    pub ranks: Vec<Rank>,
    pub build: Value,
}
pub fn catalog() -> Value {
    serde_json::from_str(include_str!(
        "../../../../resources/tencent-mayhem-items-16.21.json"
    ))
    .expect("checked-in item catalog")
}
fn probability(s: &str, scale: f64) -> Result<f64> {
    s.parse::<f64>()
        .ok()
        .map(|n| n / scale)
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
        .ok_or("tencent-rate")
}
fn number(s: &str) -> Result<u32> {
    s.parse::<u32>()
        .ok()
        .filter(|n| *n > 0 && *n < 1_000_000)
        .ok_or("tencent-id")
}
fn date(s: &str) -> bool {
    if s.len() != 8 || !s.bytes().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let y = s[..4].parse::<u32>().unwrap_or(0);
    let m = s[4..6].parse::<usize>().unwrap_or(0);
    let d = s[6..].parse::<u32>().unwrap_or(0);
    let days = [
        0,
        31,
        if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    y >= 2026 && m > 0 && m <= 12 && d > 0 && d <= days[m]
}
/// The caller uses only DETAIL_PATH on the fixed public host, never the ordinary ARAM endpoint.
pub fn parse(bytes: &[u8], champion: u32) -> Result<Snapshot> {
    if bytes.len() > LIMIT || champion == 0 || champion > 10000 {
        return Err("tencent-request");
    }
    let wrapper: Value = serde_json::from_slice(bytes).map_err(|_| "tencent-json")?;
    if wrapper["code"] != 0 {
        return Err("tencent-response");
    }
    let fields = wrapper["data"]["_fieldValues"]
        .as_object()
        .filter(|v| v.len() == 1)
        .ok_or("tencent-envelope")?;
    let raw: Value = serde_json::from_str(
        fields
            .values()
            .next()
            .and_then(Value::as_str)
            .ok_or("tencent-envelope")?,
    )
    .map_err(|_| "tencent-json")?;
    let stamp = raw["dtstatdate"]
        .as_str()
        .filter(|s| date(s))
        .ok_or("tencent-date")?;
    let mut ranks = Vec::new();
    let mut ids = HashSet::new();
    let mut positions = HashSet::new();
    let groups = raw["augment_json_irank"].as_str().ok_or("tencent-ranks")?;
    let pools: Vec<_> = groups
        .split('&')
        .filter_map(|s| s.strip_prefix("255:"))
        .collect();
    if pools.len() != 1 {
        return Err("tencent-ranks");
    }
    for row in pools[0].split('#').filter(|s| !s.is_empty()) {
        let r: Vec<_> = row.split('|').collect();
        if r.len() != 6 || number(r[1])? != champion {
            return Err("tencent-champion");
        }
        let id = number(r[2])?;
        let rank = number(r[0])?;
        let tier = match r[5] {
            "S" => 1,
            "A" => 2,
            "B" => 3,
            "C" => 4,
            "D" => 5,
            _ => return Err("tencent-tier"),
        };
        if !ids.insert(id) || !positions.insert(rank) || rank > 512 {
            return Err("tencent-ranks");
        }
        ranks.push(Rank {
            id,
            rank,
            pick: probability(r[4], 1.)?,
            tier,
        });
    }
    if ranks.is_empty() || ranks.len() > 512 {
        return Err("tencent-ranks");
    }
    ranks.sort_by_key(|r| r.rank);
    let items = catalog();
    let items: BTreeMap<_, _> = items["data"]
        .as_array()
        .ok_or("tencent-items")?
        .iter()
        .filter_map(|v| Some((v["id"].as_u64()? as u32, v)))
        .collect();
    let mut excluded = 0;
    let mut conditions = BTreeMap::new();
    let mut build = json!({"queueId":2400,"source":SOURCE,"date":stamp,"catalogPatch":CATALOG_PATCH,"patch":null,"stats":{},"tags":{"source":"腾讯101"}});
    let mut add = |field: &str, raw_ids: &str, pick: f64, win: f64| -> Result<()> {
        let ids: Vec<u32> = raw_ids
            .replace('&', ",")
            .split(',')
            .map(number)
            .collect::<Result<_>>()?;
        if ids.is_empty() || ids.len() > 6 {
            return Err("tencent-items");
        }
        let mut imports = Vec::new();
        let mut importable = true;
        for id in &ids {
            let Some(item) = items.get(id) else {
                excluded += 1;
                return Ok(());
            };
            if let Some(buy) = item["purchaseId"].as_u64() {
                imports.push(buy);
            } else {
                importable = false;
                conditions.insert(*id, json!({"id":id,"reason":item["reason"]}));
            }
        }
        if !importable {
            excluded += 1;
            return Ok(());
        }
        let row = json!({"itemIds":ids,"importItemIds":imports,"pickRate":pick,"winRate":win});
        if build.get(field).is_none() {
            build[field] = json!([]);
        }
        build[field].as_array_mut().unwrap().push(row);
        Ok(())
    };
    for (source, target) in [("itemout", "startingItems"), ("itemshoes", "boots")] {
        for row in raw[source]
            .as_str()
            .ok_or("tencent-items")?
            .split('#')
            .filter(|s| !s.is_empty())
            .take(32)
        {
            let r: Vec<_> = row.split('$').collect();
            if r.len() != 3 {
                return Err("tencent-items");
            }
            add(target, r[0], probability(r[1], 1.)?, probability(r[2], 1.)?)?;
        }
    }
    for (source, field, target) in [
        ("itemcore_json", "itemcore", "coreItems"),
        ("itemone_json", "itemone", "situationalItems"),
    ] {
        let rows: Value = serde_json::from_str(raw[source].as_str().ok_or("tencent-items")?)
            .map_err(|_| "tencent-items")?;
        for row in rows
            .as_object()
            .filter(|r| r.len() <= 64)
            .ok_or("tencent-items")?
            .values()
        {
            let val = |k: &str| -> Result<String> {
                match &row[k] {
                    Value::String(s) => Ok(s.clone()),
                    Value::Number(n) => Ok(n.to_string()),
                    _ => Err("tencent-items"),
                }
            };
            add(
                target,
                row[field].as_str().ok_or("tencent-items")?,
                probability(&val("showrate")?, 10000.)?,
                probability(&val("winrate")?, 10000.)?,
            )?;
        }
    }
    for row in raw["itemover_rec"]
        .as_str()
        .ok_or("tencent-items")?
        .split(';')
        .filter(|s| !s.is_empty())
        .take(16)
    {
        let r: Vec<_> = row.split('_').collect();
        if r.len() != 4 {
            return Err("tencent-items");
        }
        add(
            "fullItems",
            r[1],
            probability(r[2], 1.)?,
            probability(r[3], 1.)?,
        )?;
    }
    for field in [
        "startingItems",
        "boots",
        "coreItems",
        "situationalItems",
        "fullItems",
    ] {
        if let Some(rows) = build[field].as_array_mut() {
            rows.sort_by(|a, b| {
                b["pickRate"]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&a["pickRate"].as_f64().unwrap())
                    .then_with(|| a["itemIds"].to_string().cmp(&b["itemIds"].to_string()))
            });
        }
    }
    build["excludedRows"] = json!(excluded);
    build["conditionalItems"] = json!(conditions.values().collect::<Vec<_>>());
    Ok(Snapshot {
        champion,
        date: stamp.into(),
        ranks,
        build,
    })
}
impl Prepared {
    /// Apply after native metadata filtering. Keep win/source/patch/sample and stage fields intact.
    pub fn apply_tencent(&mut self, snapshot: &Snapshot) -> Result<()> {
        if self.locale != crate::Locale::ZhCn
            || crate::recognition::patch(&self.version) != crate::recognition::patch(CATALOG_PATCH)
        {
            return Err("tencent-version");
        }
        let bases = self
            .documents
            .get("augments.json")
            .and_then(|v| v["data"].as_array())
            .cloned()
            .ok_or("tencent-catalog")?;
        let byid: BTreeMap<_, _> = bases
            .iter()
            .filter_map(|a| Some((a["id"].as_u64()? as u32, a)))
            .collect();
        let key = snapshot.champion.to_string();
        let row = self
            .documents
            .iter_mut()
            .filter(|(p, _)| p.starts_with("champion-shards/"))
            .find_map(|(_, d)| d["champions"].get_mut(&key))
            .ok_or("tencent-champion")?;
        let rows = row["augments"].as_array_mut().ok_or("tencent-ranks")?;
        for a in rows.iter_mut() {
            if let Some(s) = a["stats"].as_object_mut() {
                s.remove("rank");
                s.remove("total");
                s.remove("tier");
                s.remove("rankSource");
                s.remove("rankDate");
            }
        }
        for rank in &snapshot.ranks {
            let Some(base) = byid.get(&rank.id) else {
                continue;
            };
            let index = rows
                .iter()
                .position(|a| a["id"].as_u64() == Some(rank.id as u64))
                .unwrap_or_else(|| {
                    let mut a = (*base).clone();
                    a["stats"] = json!({});
                    rows.push(a);
                    rows.len() - 1
                });
            if !rows[index]["stats"].is_object() {
                rows[index]["stats"] = json!({});
            }
            let s = &mut rows[index]["stats"];
            s["rank"] = json!(rank.rank);
            s["total"] = json!(snapshot.ranks.iter().map(|r| r.rank).max().unwrap_or(1));
            s["tier"] = json!(rank.tier);
            s["pickRate"] = json!(rank.pick);
            s["rankSource"] = json!(SOURCE);
            s["rankDate"] = json!(snapshot.date);
        }
        row["builds"] = json!([snapshot.build]);
        let catalog = catalog();
        let items = self
            .documents
            .get_mut("items.json")
            .and_then(|v| v["data"].as_array_mut())
            .ok_or("tencent-items")?;
        for item in catalog["data"].as_array().unwrap() {
            if let Some(i) = items.iter().position(|v| v["id"] == item["id"]) {
                items[i] = item.clone();
            } else {
                items.push(item.clone());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    pub const ANNIE: &[u8] = include_bytes!("../../../../tests/fixtures/tencent101/hero-1.json");
    fn edit(f: impl FnOnce(&mut Value)) -> Vec<u8> {
        let mut v: Value = serde_json::from_slice(ANNIE).unwrap();
        let raw = v["data"]["_fieldValues"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        let mut body: Value = serde_json::from_str(raw.as_str().unwrap()).unwrap();
        f(&mut body);
        *raw = json!(body.to_string());
        serde_json::to_vec(&v).unwrap()
    }
    #[test]
    fn real_mayhem_rates_recipes_and_conditional_items() {
        let s = parse(ANNIE, 1).unwrap();
        assert_eq!(s.date, "20261024");
        assert_eq!(s.build["queueId"], 2400);
        assert!(s.build["patch"].is_null());
        assert!(s.build["coreItems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["winRate"] == json!(0.5217) && r["pickRate"] == json!(0.0742)));
        assert!(s.build["conditionalItems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == 228002));
        let ryze = parse(
            include_bytes!("../../../../tests/fixtures/tencent101/hero-13.json"),
            13,
        )
        .unwrap();
        let sets = crate::item_sets::create(13, "瑞兹", &[&ryze.build]);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0]["associatedMaps"], json!([12]));
        let ids: Vec<_> = sets[0]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|b| b["items"].as_array().unwrap())
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        for id in ["228002", "994403", "3121", "3040", "2530"] {
            assert!(!ids.contains(&id), "{id}");
        }
        assert!(ids.contains(&"3119"));
        assert!(ids.contains(&"3003"));
        let catalog = catalog();
        assert_eq!(
            catalog["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == 2530)
                .unwrap()["purchaseId"],
            2526
        );
        for id in [124011, 123430] {
            assert_eq!(
                catalog["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|i| i["id"] == id)
                    .unwrap()["purchaseId"],
                id
            );
        }
    }
    #[test]
    fn reject_wrong_hero_schema_rate_duplicates_and_skip_whole_unknown_combination() {
        assert!(parse(ANNIE, 2).is_err());
        assert!(parse(b"{\"code\":0,\"data\":[]}", 1).is_err());
        for bytes in [
            edit(|v| v["dtstatdate"] = json!("20260230")),
            edit(|v| v["itemout"] = json!("1001$1.1$0.5")),
            edit(|v| {
                v["augment_json_irank"] = json!("255:1|1|1001|kGold|0.1|A#1|1|1001|kGold|0.1|A")
            }),
        ] {
            assert!(parse(&bytes, 1).is_err());
        }
        let s = parse(&edit(|v| v["itemout"] = json!("1001,999999$0.9$0.5")), 1).unwrap();
        assert!(s.build["startingItems"].is_null()); // no truncated combination, nor unknown ID
    }
    #[test]
    fn ranks_preserve_win_samples_stages_and_client_rarity_without_importing_aram() {
        let snap = parse(ANNIE, 1).unwrap();
        let id = snap.ranks[0].id;
        let mut data = Prepared {
            locale: crate::Locale::ZhCn,
            version: "16.21.2".into(),
            source: crate::Source::Cache,
            scope: crate::Scope::Detail(1),
            current_augments: false,
            catalog: Default::default(),
            documents: BTreeMap::from([
                (
                    "augments.json".into(),
                    json!({"data":[{"id":id,"name":"测试","rarity":1}]}),
                ),
                ("items.json".into(), json!({"data":[]})),
                (
                    "champion-shards/0.json".into(),
                    json!({"champions":{"1":{"champion":{"id":1,"name":"安妮"},"builds":[{"queueId":450}],"stages":[{"winRate":0.6}],"augments":[{"id":id,"name":"测试","rarity":1,"stats":{"winRate":0.55,"wins":550,"games":1000,"winRateSource":"aramgg-client-upload","winRateGamePatch":"16.21","winRateRegion":"WORLD"}}]}}}),
                ),
            ]),
        };
        data.apply_tencent(&snap).unwrap();
        let r = &data.documents["champion-shards/0.json"]["champions"]["1"];
        assert_eq!(r["stages"], json!([{"winRate":0.6}]));
        assert_eq!(r["augments"][0]["stats"]["wins"], 550);
        assert_eq!(r["augments"][0]["stats"]["games"], 1000);
        assert_eq!(r["augments"][0]["rarity"], 1);
        assert_eq!(r["augments"][0]["stats"]["winRate"], 0.55);
        assert_eq!(r["augments"][0]["stats"]["rankSource"], SOURCE);
        let rec = data.recommendations(1).unwrap();
        assert_eq!(rec[&id].win_rate, Some(0.55));
        assert_eq!(rec[&id].origin, "aramgg-client-upload");
        assert_eq!(rec[&id].score, Some(1.));
        let detail = data.champion_detail(1).unwrap();
        assert!(!detail.item_sets.is_empty());
        assert!(detail.augments[0].provenance.contains("腾讯101推荐"));
        data.version = "16.22.1".into();
        assert!(data.apply_tencent(&snap).is_err());
    }
}
