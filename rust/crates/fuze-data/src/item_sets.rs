//! Public build data -> bounded managed item sets. No client access or account data.
use serde_json::{json, Value};
fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten().take(1000)
}
fn rate(v: &Value) -> f64 {
    v.as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .unwrap_or(0.0)
}
fn games(v: &Value) -> u64 {
    v["games"].as_u64().unwrap_or(0)
}
fn pick(v: &Value) -> f64 {
    rate(
        v.get("pickRate")
            .or_else(|| v.get("pick_rate"))
            .unwrap_or(&Value::Null),
    )
}
fn trusted(v: &Value) -> Vec<&Value> {
    let mut r: Vec<_> = rows(v).filter(|r| games(r) >= 2 || pick(r) > 0.0).collect();
    r.sort_by(|a, b| {
        games(b)
            .cmp(&games(a))
            .then(pick(b).total_cmp(&pick(a)))
            .then(rate(&b["winRate"]).total_cmp(&rate(&a["winRate"])))
    });
    r
}
fn ids(v: &Value) -> Vec<String> {
    let values: Vec<_> = v
        .get("itemIds")
        .or_else(|| v.get("items"))
        .and_then(Value::as_array)
        .map(|a| a.iter().take(12).collect())
        .unwrap_or_else(|| {
            v.get("itemId")
                .or_else(|| v.get("id"))
                .into_iter()
                .collect()
        });
    values
        .into_iter()
        .filter_map(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|n| *n > 0 && *n <= u32::MAX as u64)
        .map(|n| n.to_string())
        .collect()
}
pub fn create(champion: u32, name: &str, builds: &[&Value]) -> Vec<Value> {
    create_indexed(champion, name, builds)
        .into_iter()
        .map(|(_, set)| set)
        .collect()
}
pub fn create_indexed(champion: u32, name: &str, builds: &[&Value]) -> Vec<(usize, Value)> {
    if champion == 0 {
        return vec![];
    }
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (route, build) in builds.iter().take(6).enumerate() {
        if build["queueId"].as_u64() != Some(2400) {
            continue;
        }
        let core = build
            .get("coreItems")
            .or_else(|| build.get("recommended"))
            .unwrap_or(&Value::Null);
        let key = serde_json::to_string(build).unwrap_or_default();
        if !seen.insert(key) {
            continue;
        }
        let rate_based = rows(core).any(|r| pick(r) > 0.0);
        if !rate_based
            && (build["games"]
                .as_u64()
                .or_else(|| build["stats"]["games"].as_u64())
                .unwrap_or(0)
                < 300
                || rows(core).map(games).max().unwrap_or(0) < 20)
        {
            continue;
        }
        let mut blocks = Vec::new();
        let mut blocked = std::collections::HashSet::new();
        for (values, label, limit) in [
            (&build["startingItems"], "出门装备", 3),
            (core, "核心装备", 5),
            (&build["fullItems"], "完整出装", 3),
        ] {
            for (i, row) in trusted(values).into_iter().take(limit).enumerate() {
                let items = ids(row);
                if label == "核心装备" {
                    blocked.extend(items.iter().cloned());
                }
                if !items.is_empty() {
                    blocks.push(json!({"type":format!("FUZE {label} {}",i+1),"items":items.into_iter().map(|id|json!({"id":id,"count":1})).collect::<Vec<_>>()}));
                }
            }
        }
        for row in trusted(&build["itemExtensions"]).into_iter().take(12) {
            let items = ids(row);
            let core = row["coreItemIds"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_u64())
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_default();
            if !items.is_empty() {
                blocks.push(json!({"type":format!("FUZE 后续 · 核心 {core} · 第 {} 步",row["step"].as_u64().unwrap_or(1)),"items":items.into_iter().map(|id|json!({"id":id,"count":1})).collect::<Vec<_>>()}));
            }
        }
        for (field, label, limit) in [("situationalItems", "备选装备", 18)] {
            let mut unique = std::collections::HashSet::new();
            let items: Vec<_> = trusted(&build[field])
                .into_iter()
                .filter_map(|r| ids(r).first().cloned())
                .filter(|id| {
                    (field != "situationalItems" || !blocked.contains(id))
                        && unique.insert(id.clone())
                })
                .take(limit)
                .map(|id| json!({"id":id,"count":1}))
                .collect();
            if !items.is_empty() {
                blocks.push(json!({"type":format!("FUZE {label}"),"items":items}));
            }
        }
        if blocks.is_empty() {
            continue;
        }
        let i = result.len();
        let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
        let mode = match build["queueId"].as_u64() {
            Some(2400) => "海克斯大乱斗",
            Some(450) => "普通大乱斗参考",
            _ => "模式未知参考",
        };
        let tags = build["tags"]
            .as_object()
            .map(|o| {
                o.values()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        let patch = build["patch"].as_str().unwrap_or("补丁未知");
        let title: String = format!("FUZE {name} · 路线 {} {tags} · {mode} · {patch}", route + 1)
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect();
        result.push((route,json!({"uid":if i==0{"aramgg-aram-current".into()}else{format!("aramgg-aram-current-{}",i+1)},"title":title,"associatedMaps":[11,12],"associatedChampions":[champion],"blocks":blocks,"map":"any","mode":"any","preferredItemSlots":[],"sortrank":0,"sortRank":0,"startedFrom":"blank","type":"custom"})));
        if result.len() == 4 {
            break;
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extensions_keep_full_sequence_core_step_and_route_provenance() {
        let low = json!({"queueId":2400,"games":1});
        let b = json!({"queueId":2400,"patch":"16.20","tags":{"style":"AP"},"games":1000,"coreItems":[{"itemIds":[3001,3002],"games":40}],"itemExtensions":[{"coreItemIds":[3001,3002],"step":2,"itemIds":[3089,3135],"games":40}]});
        let sets = create_indexed(21, "英雄", &[&low, &b]);
        assert_eq!(sets[0].0, 1);
        let set = &sets[0].1;
        assert!(set["title"]
            .as_str()
            .unwrap()
            .contains("路线 2 AP · 海克斯大乱斗 · 16.20"));
        assert_eq!(
            set["blocks"][1]["items"],
            json!([{"id":"3089","count":1},{"id":"3135","count":1}])
        );
        assert!(set["blocks"][1]["type"]
            .as_str()
            .unwrap()
            .contains("3001/3002 · 第 2 步"));
    }
    #[test]
    fn ordinary_and_unknown_modes_cannot_be_imported_even_with_large_samples() {
        for queue in [serde_json::json!(450), serde_json::Value::Null] {
            let build = json!({"queueId":queue,"games":100000,"coreItems":[{"itemIds":[3001],"games":10000,"pickRate":0.9}]});
            assert!(create(21, "Test", &[&build]).is_empty());
        }
    }
    #[test]
    fn no_low_sample_or_empty_sets_and_four_managed_only() {
        let low = json!({"queueId":2400,"games":2,"coreItems":[{"itemIds":[3001,3002],"games":2}]});
        assert!(create(21, "测试", &[&low]).is_empty());
        let valid = json!({"queueId":2400,"games":1000,"coreItems":[{"itemIds":[3001,3002],"games":40}],"situationalItems":[{"id":3001,"games":30},{"id":3003,"games":3}]});
        let sets = create(21, "测试", &[&valid, &valid]);
        assert_eq!(sets.len(), 1);
        assert_eq!(
            sets[0]["blocks"][1]["items"],
            json!([{"id":"3003","count":1}])
        );
        assert_eq!(sets[0]["associatedChampions"], json!([21]));
    }
}
