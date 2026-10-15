//! Bounded, read-only presentation of one versioned champion shard. Never invent missing runes.
use crate::{Prepared, Result};
use serde_json::Value;
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub icons: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub title: String,
    pub lines: Vec<Line>,
}
fn clean(v: &Value) -> String {
    v.as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect()
}
fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten().take(1000)
}
fn rate(v: &Value) -> String {
    v.as_f64()
        .filter(|x| x.is_finite() && (0.0..=1.0).contains(x))
        .map(|x| format!("{:.1}%", x * 100.))
        .unwrap_or("--".into())
}
fn stats(v: &Value) -> String {
    // Some source item rows use zero as a missing-rate sentinel despite positive wins.
    let empirical = v["games"]
        .as_u64()
        .zip(v["wins"].as_u64())
        .filter(|(g, w)| *g > 0 && *w > 0 && w <= g);
    let win = if v["winRate"].as_f64() == Some(0.) && empirical.is_some() {
        let (g, w) = empirical.unwrap();
        format!("{:.1}%（按胜场计算）", w as f64 / g as f64 * 100.)
    } else {
        rate(&v["winRate"])
    };
    format!(
        "胜 {} · 选 {} · 样本 {}",
        win,
        rate(&v["pickRate"]),
        v["games"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or("--".into())
    )
}
impl Prepared {
    pub fn champion_detail(&self, id: u32) -> Result<Detail> {
        let key = id.to_string();
        let mut matches = self
            .documents
            .iter()
            .filter(|(p, _)| p.starts_with("champion-shards/"))
            .filter_map(|(_, d)| d["champions"].get(&key));
        let row = matches.next().ok_or("detail-champion-missing")?;
        if matches.next().is_some() {
            return Err("detail-champion-duplicate");
        }
        let items = self.document("items.json").ok_or("detail-items-missing")?;
        Ok(parse(row, items))
    }
}
fn parse(row: &Value, items: &Value) -> Detail {
    let mut lines = Vec::new();
    let mut add = |text: String, icons: Vec<String>| {
        if lines.len() < 320 {
            lines.push(Line { text, icons });
        }
    };
    let c = &row["champion"];
    add(format!("英雄整体 · {}", stats(&c["stats"])), vec![]);
    add(
        format!(
            "来源 {} / {} · 补丁 {}",
            clean(&c["stats"]["source"]),
            clean(&c["stats"]["region"]),
            clean(&c["stats"]["gamePatch"])
        ),
        vec![],
    );
    let builds: Vec<_> = if row["builds"].as_array().is_some_and(|a| !a.is_empty()) {
        rows(&row["builds"]).take(6).collect()
    } else {
        row.get("build")
            .filter(|v| v.is_object())
            .into_iter()
            .collect()
    };
    if builds.is_empty() {
        add("出装、加点与召唤师技能：数据源暂未提供".into(), vec![]);
    }
    for (i, b) in builds.iter().enumerate() {
        let mode: String = match b["queueId"].as_u64() {
            Some(450) => "普通大乱斗参考（非海克斯统计）".into(),
            Some(2400) => "海克斯大乱斗".into(),
            _ => "模式未标明，仅供参考".into(),
        };
        add(
            format!("路线 {} · {} · 补丁 {}", i + 1, mode, clean(&b["patch"])),
            vec![],
        );
        add(stats(&b["stats"]), vec![]);
        for (field, label) in [
            ("startingItems", "出门"),
            ("coreItems", "核心"),
            ("fullItems", "成装"),
            ("itemExtensions", "后续装备"),
            ("situationalItems", "备选装备"),
        ] {
            let mut entries: Vec<_> = rows(&b[field]).collect();
            entries.sort_by_key(|r| std::cmp::Reverse(r["games"].as_u64().unwrap_or(0)));
            for r in entries.into_iter().take(3) {
                if field == "itemExtensions" {
                    let names = rows(&r["coreItemIds"])
                        .take(6)
                        .filter_map(Value::as_u64)
                        .map(|id| {
                            items["data"]
                                .as_array()
                                .and_then(|a| a.iter().find(|i| i["id"].as_u64() == Some(id)))
                                .map(|i| clean(&i["name"]))
                                .unwrap_or(format!("装备 {id}"))
                        })
                        .collect::<Vec<_>>();
                    add(
                        format!(
                            "仅适用核心：{} · 后续第 {} 步",
                            names.join(" / "),
                            r["step"].as_u64().unwrap_or(0)
                        ),
                        vec![],
                    );
                }
                let ids: Vec<_> = if let Some(ids) = r["itemIds"].as_array() {
                    ids.iter().take(6).filter_map(Value::as_u64).collect()
                } else {
                    r["id"].as_u64().into_iter().collect()
                };
                let mut names = Vec::new();
                let mut icons = Vec::new();
                for id in ids {
                    let item = rows(&items["data"])
                        .find(|v| v["id"].as_u64() == Some(id))
                        .or_else(|| {
                            items["data"]
                                .as_array()?
                                .iter()
                                .find(|v| v["id"].as_u64() == Some(id))
                        });
                    names.push(
                        item.map(|v| clean(&v["name"]))
                            .filter(|s| !s.is_empty())
                            .unwrap_or(format!("装备 {id}")),
                    );
                    if let Some(item) = item {
                        icons.push(clean(&item["iconUrl"]));
                    }
                }
                if !names.is_empty() {
                    add(format!("{label} · {}", names.join(" → ")), icons);
                    add(stats(r), vec![]);
                }
            }
        }
        for r in rows(&b["skillOrders"]).take(1) {
            let keys = rows(&r["skillKeys"])
                .take(18)
                .map(clean)
                .collect::<Vec<_>>()
                .join(" ");
            add(format!("1–18 级加点 · {keys}"), vec![]);
        }
        for r in rows(&b["summonerSpells"]).take(1) {
            let names = rows(&r["summonerSpellIds"])
                .filter_map(Value::as_u64)
                .map(|id| match id {
                    4 => "闪现".into(),
                    32 => "标记".into(),
                    7 => "治疗".into(),
                    14 => "点燃".into(),
                    3 => "虚弱".into(),
                    6 => "疾跑".into(),
                    21 => "屏障".into(),
                    _ => format!("技能 {id}"),
                })
                .collect::<Vec<String>>();
            add(format!("召唤师技能 · {}", names.join(" / ")), vec![]);
        }
    }
    add(
        "符文页：当前客户端数据接口未提供，未自动配置游戏符文".into(),
        vec![],
    );
    add(
        "海克斯强化 · 整体胜率参考（实际领取轮次未确认）".into(),
        vec![],
    );
    let mut augments: Vec<_> = rows(&row["augments"]).collect();
    augments.sort_by(|a, b| {
        b["stats"]["winRate"]
            .as_f64()
            .partial_cmp(&a["stats"]["winRate"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for a in augments.into_iter().take(20) {
        let s = &a["stats"];
        add(
            format!("{} · {}", clean(&a["name"]), stats(s)),
            vec![clean(&a["iconUrl"])],
        );
        add(
            format!(
                "胜率来源 {} / {} · 补丁 {}",
                clean(s.get("winRateSource").unwrap_or(&s["source"])),
                clean(s.get("winRateRegion").unwrap_or(&s["region"])),
                clean(s.get("winRateGamePatch").unwrap_or(&s["gamePatch"]))
            ),
            vec![],
        );
    }
    Detail {
        title: format!("{} · {}", clean(&c["name"]), clean(&c["title"])),
        lines,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_data_is_not_a_zero_rate_and_modes_remain_explicit() {
        assert!(
            stats(&serde_json::json!({"winRate":0,"games":100,"wins":55}))
                .contains("55.0%（按胜场计算）")
        );
        let d = parse(
            &serde_json::json!({"champion":{"name":"Test"},"build":{"queueId":450,"coreItems":[{"itemIds":[99],"games":2,"winRate":null}]} }),
            &serde_json::json!({"data":[]}),
        );
        assert!(d.lines.iter().any(|l| l.text.contains("非海克斯统计")));
        assert!(d.lines.iter().any(|l| l.text.contains("装备 99")));
        assert!(d.lines.iter().any(|l| l.text.contains("胜 --")));
        assert!(d.lines.iter().any(|l| l.text.contains("未提供")));
    }
}
