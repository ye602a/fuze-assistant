//! Bounded, read-only presentation of one versioned champion shard. Never invent missing runes.
use crate::{Locale, Prepared, Result};
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
fn word(locale: Locale, key: &str) -> &str {
    let i = match locale {
        Locale::ZhCn => 0,
        Locale::ZhTw => 1,
        Locale::EnUs => 2,
    };
    const WORDS: &[[&str; 3]] = &[
        ["胜", "勝", "Win"],
        ["选", "選", "Pick"],
        ["样本", "樣本", "Games"],
        ["按胜场计算", "依勝場計算", "derived from wins"],
        ["英雄整体", "英雄整體", "Overall champion"],
        ["来源", "來源", "Source"],
        ["补丁", "版本", "Patch"],
        [
            "出装、加点与召唤师技能：数据源暂未提供",
            "出裝、技能點法與召喚師技能：資料源尚未提供",
            "Builds, skill order and spells: unavailable from source",
        ],
        [
            "普通大乱斗参考（非海克斯统计）",
            "一般大亂鬥參考（非海克斯統計）",
            "Standard ARAM reference (not Mayhem statistics)",
        ],
        ["海克斯大乱斗", "海克斯大亂鬥", "ARAM Mayhem"],
        [
            "模式未标明，仅供参考",
            "模式未標明，僅供參考",
            "Unspecified mode; reference only",
        ],
        ["路线", "路線", "Build"],
        ["出门", "起始裝備", "Starting"],
        ["核心", "核心", "Core"],
        ["成装", "完整裝備", "Full build"],
        ["后续装备", "後續裝備", "Extensions"],
        ["备选装备", "備選裝備", "Situational"],
        ["装备", "裝備", "Item"],
        ["仅适用核心", "僅適用核心", "Applicable core only"],
        ["后续步骤", "後續步驟", "Extension step"],
        ["1–18 级加点", "1–18 級技能點法", "1–18 skill order"],
        ["召唤师技能", "召喚師技能", "Summoner spells"],
        ["闪现", "閃現", "Flash"],
        ["标记", "標記", "Mark"],
        ["治疗", "治癒", "Heal"],
        ["点燃", "點燃", "Ignite"],
        ["虚弱", "虛弱", "Exhaust"],
        ["疾跑", "鬼步", "Ghost"],
        ["屏障", "光盾", "Barrier"],
        ["技能", "技能", "Spell"],
        [
            "符文页：当前客户端数据接口未提供，未自动配置游戏符文",
            "符文頁：目前資料接口未提供，不會自動設定遊戲符文",
            "Runes: unavailable from data source; game runes not modified",
        ],
        [
            "海克斯强化 · 整体胜率参考（实际领取轮次未确认）",
            "海克斯增幅 · 整體勝率參考（實際領取輪次未確認）",
            "Augments · Overall win rates (claim round unconfirmed)",
        ],
        ["胜率来源", "勝率來源", "Win-rate source"],
    ];
    WORDS
        .iter()
        .find(|r| r[0] == key)
        .map(|r| r[i])
        .unwrap_or(key)
}
fn stats(v: &Value, locale: Locale) -> String {
    // Some source item rows use zero as a missing-rate sentinel despite positive wins.
    let empirical = v["games"]
        .as_u64()
        .zip(v["wins"].as_u64())
        .filter(|(g, w)| *g > 0 && *w > 0 && w <= g);
    let win = if v["winRate"].as_f64() == Some(0.) && empirical.is_some() {
        let (g, w) = empirical.unwrap();
        format!(
            "{:.1}%（{}）",
            w as f64 / g as f64 * 100.,
            word(locale, "按胜场计算")
        )
    } else {
        rate(&v["winRate"])
    };
    format!(
        "{} {} · {} {} · {} {}",
        word(locale, "胜"),
        win,
        word(locale, "选"),
        rate(&v["pickRate"]),
        word(locale, "样本"),
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
        Ok(parse(row, items, self.locale))
    }
}
fn parse(row: &Value, items: &Value, locale: Locale) -> Detail {
    let t = |s| word(locale, s);
    let stats = |v: &Value| stats(v, locale);
    let mut lines = Vec::new();
    let mut add = |text: String, icons: Vec<String>| {
        if lines.len() < 320 {
            lines.push(Line { text, icons });
        }
    };
    let c = &row["champion"];
    add(
        format!("{} · {}", t("英雄整体"), stats(&c["stats"])),
        vec![],
    );
    add(
        format!(
            "{} {} / {} · {} {}",
            t("来源"),
            clean(&c["stats"]["source"]),
            clean(&c["stats"]["region"]),
            t("补丁"),
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
        add(t("出装、加点与召唤师技能：数据源暂未提供").into(), vec![]);
    }
    for (i, b) in builds.iter().enumerate() {
        let mode: String = match b["queueId"].as_u64() {
            Some(450) => t("普通大乱斗参考（非海克斯统计）").into(),
            Some(2400) => t("海克斯大乱斗").into(),
            _ => t("模式未标明，仅供参考").into(),
        };
        add(
            format!(
                "{} {} · {} · {} {}",
                t("路线"),
                i + 1,
                mode,
                t("补丁"),
                clean(&b["patch"])
            ),
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
                                .unwrap_or(format!("{} {id}", t("装备")))
                        })
                        .collect::<Vec<_>>();
                    add(
                        format!(
                            "{}: {} · {} {}",
                            t("仅适用核心"),
                            names.join(" / "),
                            t("后续步骤"),
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
                            .unwrap_or(format!("{} {id}", t("装备"))),
                    );
                    if let Some(item) = item {
                        icons.push(clean(&item["iconUrl"]));
                    }
                }
                if !names.is_empty() {
                    add(format!("{} · {}", t(label), names.join(" → ")), icons);
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
            add(format!("{} · {keys}", t("1–18 级加点")), vec![]);
        }
        for r in rows(&b["summonerSpells"]).take(1) {
            let names = rows(&r["summonerSpellIds"])
                .filter_map(Value::as_u64)
                .map(|id| match id {
                    4 => t("闪现").into(),
                    32 => t("标记").into(),
                    7 => t("治疗").into(),
                    14 => t("点燃").into(),
                    3 => t("虚弱").into(),
                    6 => t("疾跑").into(),
                    21 => t("屏障").into(),
                    _ => format!("{} {id}", t("技能")),
                })
                .collect::<Vec<String>>();
            add(
                format!("{} · {}", t("召唤师技能"), names.join(" / ")),
                vec![],
            );
        }
    }
    add(
        t("符文页：当前客户端数据接口未提供，未自动配置游戏符文").into(),
        vec![],
    );
    add(
        t("海克斯强化 · 整体胜率参考（实际领取轮次未确认）").into(),
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
                "{} {} / {} · {} {}",
                t("胜率来源"),
                clean(s.get("winRateSource").unwrap_or(&s["source"])),
                clean(s.get("winRateRegion").unwrap_or(&s["region"])),
                t("补丁"),
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
    fn details_follow_locale_without_translating_data_names() {
        let row = serde_json::json!({"champion":{"name":"fixture-name"},"build":{"queueId":450}});
        let items = serde_json::json!({"data":[]});
        let en = parse(&row, &items, Locale::EnUs);
        assert!(en.title.starts_with("fixture-name"));
        assert!(en
            .lines
            .iter()
            .any(|l| l.text.contains("not Mayhem statistics")));
        assert!(en.lines.iter().all(|l| !l
            .text
            .chars()
            .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))));
        let tw = parse(&row, &items, Locale::ZhTw);
        assert!(tw.lines.iter().any(|l| l.text.starts_with("路線")));
    }
    #[test]
    fn absent_data_is_not_a_zero_rate_and_modes_remain_explicit() {
        assert!(stats(
            &serde_json::json!({"winRate":0,"games":100,"wins":55}),
            Locale::ZhCn
        )
        .contains("55.0%（按胜场计算）"));
        let d = parse(
            &serde_json::json!({"champion":{"name":"Test"},"build":{"queueId":450,"coreItems":[{"itemIds":[99],"games":2,"winRate":null}]} }),
            &serde_json::json!({"data":[]}),
            Locale::ZhCn,
        );
        assert!(d.lines.iter().any(|l| l.text.contains("非海克斯统计")));
        assert!(d.lines.iter().any(|l| l.text.contains("装备 99")));
        assert!(d.lines.iter().any(|l| l.text.contains("胜 --")));
        assert!(d.lines.iter().any(|l| l.text.contains("未提供")));
    }
}
