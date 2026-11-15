//! Routes observed in other heroes' finite ARAMKit combination lists.
//!
//! This index contains membership and provenance only. A listing is neither an
//! offer probability nor evidence of performance on a different hero. Keep it
//! separate from hero-specific combination statistics and recommendation scores.
use crate::{aramkit, Prepared, Scope};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub members: Vec<u32>,
    pub champions: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub dataset: String,
    pub patch: String,
    pub date: String,
    pub data_path: String,
    pub rows: Vec<Route>,
    /// Distinct heroes with an explicit, same-batch combination array, including
    /// empty arrays. This is source coverage, never a popularity or pick rate.
    pub champions: usize,
}

fn positive_id(value: &Value) -> Option<u32> {
    u32::try_from(value.as_u64()?).ok().filter(|id| *id > 0)
}

fn members(row: &Value, ordinary: &BTreeSet<u32>) -> Option<Vec<u32>> {
    let ids = row["augmentIds"].as_array()?;
    if !(2..=4).contains(&ids.len())
        || !row["sampleCount"]
            .as_u64()
            .is_some_and(|n| (200..=9_007_199_254_740_991).contains(&n))
        || !row["winRate"]
            .as_f64()
            .is_some_and(|r| r.is_finite() && (0.0..=1.0).contains(&r))
    {
        return None;
    }
    let mut ids = ids.iter().map(positive_id).collect::<Option<Vec<_>>>()?;
    ids.sort_unstable();
    (ids.iter().all(|id| ordinary.contains(id)) && ids.windows(2).all(|pair| pair[0] != pair[1]))
        .then_some(ids)
}

/// Build once from an already loaded full snapshot; performs no IO or requests.
///
/// Missing/legacy sources return `None`. A same-batch explicit array can produce
/// an empty index when no rows meet the display eligibility checks. A malformed
/// or mismatched hero does not supply routes or count toward source coverage.
pub fn from_prepared(data: &Prepared) -> Option<Index> {
    if data.scope() != Scope::Full {
        return None;
    }
    let manifest = data.document("manifest.json")?;
    if manifest["source"] != aramkit::SOURCE {
        return None;
    }
    let patch = manifest["gamePatch"].as_str()?;
    let date = manifest["dataDate"].as_str()?;
    let data_path = manifest["dataPath"].as_str()?;
    if patch.split('.').count() != 2
        || !patch
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        || !(data.version() == patch || data.version().starts_with(&format!("{patch}.")))
        || date.len() != 10
        || !date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
        || crate::safe_path(data_path).is_err()
        || !data_path.starts_with(&format!("data/{patch}-"))
        || data_path.split('/').count() != 2
    {
        return None;
    }
    // Use the same snapshot's ordinary resource rows, not a recognition catalogue
    // that a caller may have supplemented with older aliases or special choices.
    let ordinary: BTreeSet<_> = data.document("augments.json")?["data"]
        .as_array()?
        .iter()
        .filter(|row| row["enabled"] != false && row["rarity"].as_u64().is_some_and(|r| r <= 2))
        .filter_map(|row| positive_id(&row["id"]))
        .filter(|id| !(3170..=3173).contains(id))
        .collect();
    if ordinary.is_empty() {
        return None;
    }
    let shards = data.document("champion-shards/index.json")?["shards"].as_array()?;
    if shards.len() > 512 {
        return None;
    }
    let mut routes: BTreeMap<Vec<u32>, BTreeSet<u32>> = BTreeMap::new();
    let mut champions = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for shard in shards {
        let Some(path) = shard["path"].as_str().filter(|p| {
            p.starts_with("champion-shards/")
                && *p != "champion-shards/index.json"
                && crate::safe_path(p).is_ok()
        }) else {
            continue;
        };
        let Some(ids) = shard["championIds"]
            .as_array()
            .filter(|ids| ids.len() <= 512)
        else {
            continue;
        };
        let Some(rows) = data.document(path).and_then(|v| v["champions"].as_object()) else {
            continue;
        };
        for id in ids.iter().filter_map(positive_id) {
            if !visited.insert((path, id)) {
                continue;
            }
            let Some(hero) = rows.get(&id.to_string()) else {
                continue;
            };
            let reference = &hero["aramkitReference"];
            if positive_id(&hero["champion"]["id"]) != Some(id)
                || reference["patch"] != patch
                || reference["date"] != date
                || reference["dataPath"] != data_path
            {
                continue;
            }
            let Some(combinations) = reference["augmentCombinations"]
                .as_array()
                .filter(|rows| rows.len() <= 512)
            else {
                continue;
            };
            champions.insert(id);
            for row in combinations {
                if let Some(members) = members(row, &ordinary) {
                    routes.entry(members).or_default().insert(id);
                }
            }
        }
    }
    if champions.is_empty() {
        return None;
    }
    Some(Index {
        dataset: data.identity(),
        patch: patch.into(),
        date: date.into(),
        data_path: data_path.into(),
        rows: routes
            .into_iter()
            .map(|(members, champions)| Route {
                members,
                champions: champions.into_iter().collect(),
            })
            .collect(),
        champions: champions.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Locale, Source};
    use serde_json::json;
    use std::collections::HashMap;

    fn fixture() -> Prepared {
        let value: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/aramkit-16.22-cross-hero-routes.json"
        ))
        .unwrap();
        let source = &value["source"];
        let version = format!(
            "{}.{}",
            source["patch"].as_str().unwrap(),
            source["buildTimeUnixMs"]
        );
        let mut documents = BTreeMap::from([
            (
                "manifest.json".into(),
                json!({"source":source["provider"],"gamePatch":source["patch"],
                    "dataDate":source["dataDate"],"dataPath":source["dataPath"],
                    "dataVersion":version}),
            ),
            (
                "augments.json".into(),
                json!({"data":value["resources"].as_array().unwrap().iter().map(|row| {
                    let rarity = match row["rarity"].as_str().unwrap() {
                        "silver" => 0, "gold" => 1, "prismatic" => 2, _ => unreachable!(),
                    };
                    json!({"id":row["id"],"name":row["name"],"rarity":rarity,"enabled":true})
                }).collect::<Vec<_>>()}),
            ),
        ]);
        let mut shards = Vec::new();
        for hero in value["heroes"].as_array().unwrap() {
            let id = hero["championId"].as_u64().unwrap();
            let path = format!("champion-shards/{id}.json");
            shards.push(json!({"path":path,"championIds":[id]}));
            documents.insert(path, json!({"dataVersion":version,"champions":{id.to_string():{
                "champion":{"id":id},"aramkitReference":{
                    "patch":source["patch"],"date":source["dataDate"],"dataPath":source["dataPath"],
                    "augmentCombinations":hero["combinationRows"]
                }
            }}}));
        }
        documents.insert(
            "champion-shards/index.json".into(),
            json!({"shards":shards}),
        );
        Prepared {
            locale: Locale::ZhCn,
            version,
            source: Source::Cache,
            scope: Scope::Full,
            documents,
            catalog: HashMap::new(),
            current_augments: false,
        }
    }

    fn reference(data: &mut Prepared, id: u32) -> &mut Value {
        &mut data
            .documents
            .get_mut(&format!("champion-shards/{id}.json"))
            .unwrap()["champions"][id.to_string()]["aramkitReference"]
    }

    #[test]
    fn real_subset_preserves_members_and_exact_source_heroes_without_rates() {
        let data = fixture();
        let index = from_prepared(&data).unwrap();
        assert_eq!(index.dataset, "zh-CN@16.22.1794482205522");
        assert_eq!(index.patch, "16.22");
        assert_eq!(index.date, "2026-11-12");
        assert_eq!(index.data_path, "data/16.22-20261112-bfae7c1ce948");
        assert_eq!(index.champions, 3);
        assert_eq!(
            index.rows,
            vec![
                Route {
                    members: vec![1004, 3048],
                    champions: vec![157]
                }, // Real n=383 is eligible for a route.
                Route {
                    members: vec![1004, 3086],
                    champions: vec![145]
                },
                Route {
                    members: vec![1029, 3086],
                    champions: vec![17, 145]
                },
                Route {
                    members: vec![1058, 3048],
                    champions: vec![157]
                },
                Route {
                    members: vec![1388, 3060],
                    champions: vec![17]
                },
                Route {
                    members: vec![1388, 3086],
                    champions: vec![17, 145]
                },
            ]
        );
    }

    #[test]
    fn legacy_partial_and_absent_sources_do_not_become_empty_global_knowledge() {
        for scope in [
            Scope::Ocr,
            Scope::Champion(145),
            Scope::Detail(145),
            Scope::History,
        ] {
            let mut data = fixture();
            data.scope = scope;
            assert!(from_prepared(&data).is_none());
        }
        let mut data = fixture();
        data.documents.get_mut("manifest.json").unwrap()["source"] = json!("Tencent");
        assert!(from_prepared(&data).is_none());
        data.documents.remove("manifest.json");
        assert!(from_prepared(&data).is_none());
        for payload in [Value::Null, json!({})] {
            let mut data = fixture();
            for id in [145, 17, 157] {
                reference(&mut data, id)["augmentCombinations"] = payload.clone();
            }
            assert!(from_prepared(&data).is_none());
        }
        let mut data = fixture();
        for id in [145, 17, 157] {
            reference(&mut data, id)["augmentCombinations"] = json!([]);
        }
        let index = from_prepared(&data).unwrap();
        assert!(index.rows.is_empty());
        assert_eq!(index.champions, 3);
    }

    #[test]
    fn mismatched_batches_and_hero_identity_never_contribute_routes() {
        for (field, wrong) in [
            ("patch", "16.21"),
            ("date", "2026-11-11"),
            ("dataPath", "data/16.22-20261112-other"),
        ] {
            let mut data = fixture();
            reference(&mut data, 145)[field] = json!(wrong);
            let index = from_prepared(&data).unwrap();
            assert_eq!(index.champions, 2);
            assert!(index.rows.iter().all(|r| !r.champions.contains(&145)));
            for id in [17, 157] {
                reference(&mut data, id)[field] = json!(wrong);
            }
            assert!(from_prepared(&data).is_none());
        }
        let mut data = fixture();
        data.documents.get_mut("champion-shards/145.json").unwrap()["champions"]["145"]
            ["champion"]["id"] = json!(17);
        assert_eq!(from_prepared(&data).unwrap().champions, 2);
        for field in ["gamePatch", "dataDate", "dataPath"] {
            let mut data = fixture();
            data.documents
                .get_mut("manifest.json")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(from_prepared(&data).is_none());
        }
    }

    #[test]
    fn only_eligible_ordinary_routes_survive_and_duplicates_do_not_add_weight() {
        let mut data = fixture();
        let valid = json!({"augmentIds":[3086,1004],"sampleCount":200,"winRate":0.5});
        let mut rows = vec![valid.clone(), valid.clone()];
        for ids in [
            json!([1004]),
            json!([1004, 1004]),
            json!([1004, 0]),
            json!([1004, 99999]),
            json!([1004, 3170]),
            json!([1004, 1029, 1058, 1388, 3048]),
            json!([1004, "3086"]),
        ] {
            let mut bad = valid.clone();
            bad["augmentIds"] = ids;
            rows.push(bad);
        }
        for (field, value) in [
            ("sampleCount", json!(199)),
            ("sampleCount", Value::Null),
            ("sampleCount", json!(-1)),
            ("sampleCount", json!(9_007_199_254_740_992_u64)),
            ("winRate", json!(-0.1)),
            ("winRate", json!(1.1)),
            ("winRate", Value::Null),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            rows.push(bad);
        }
        reference(&mut data, 145)["augmentCombinations"] = json!(rows);
        for id in [17, 157] {
            reference(&mut data, id)["augmentCombinations"] = json!([]);
        }
        data.documents.get_mut("augments.json").unwrap()["data"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":3170,"rarity":0,"enabled":true}));
        // A duplicated shard entry and hero ID must not duplicate provenance.
        data.documents
            .get_mut("champion-shards/index.json")
            .unwrap()["shards"]
            .as_array_mut()
            .unwrap()
            .push(json!({"path":"champion-shards/145.json","championIds":[145,145]}));
        let index = from_prepared(&data).unwrap();
        assert_eq!(index.champions, 3);
        assert_eq!(
            index.rows,
            vec![Route {
                members: vec![1004, 3086],
                champions: vec![145]
            }]
        );
        for row in data.documents.get_mut("augments.json").unwrap()["data"]
            .as_array_mut()
            .unwrap()
        {
            if row["id"] == 1004 {
                row["enabled"] = json!(false);
            }
        }
        assert!(from_prepared(&data).unwrap().rows.is_empty());
    }
}
