//! Bounded, read-only presentation of one versioned champion shard. Never invent missing runes.
use crate::{Locale, Prepared, Result};
use serde_json::Value;
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub icons: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub title: String,
    pub lines: Vec<Line>,
    pub augments: Vec<Augment>,
    pub guides: Vec<Guide>,
    pub item_sets: Vec<Value>,
    pub item_routes: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Augment {
    pub id: u32,
    pub name: String,
    pub icon: String,
    pub rarity: Option<u8>,
    pub tier: Option<u64>,
    pub win: Option<f64>,
    pub pick: Option<f64>,
    pub description: String,
    pub provenance: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Guide {
    pub title: String,
    pub url: String,
}
/// Plain text only: provider markup is never executed or handed to a web view.
pub fn description(value: &str) -> String {
    let mut tag = false;
    let mut output = String::new();
    for c in value.chars().take(8192) {
        match c {
            '<' => {
                tag = true;
                output.push(' ');
            }
            '>' => tag = false,
            _ if !tag && !c.is_control() => output.push(c),
            _ => (),
        }
    }
    output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(2048)
        .collect()
}
/// External links must be ordinary HTTPS articles, never credentials, files or executable schemes.
pub fn guide_url(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    value.len() <= 2048
        && host.contains('.')
        && !host.starts_with('.')
        && host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        && !value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        && !value.contains('@')
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
            "暂无海克斯出装数据",
            "暫無海克斯出裝資料",
            "No ARAM Mayhem build data available",
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
    /// Display metadata when hero statistics are unavailable. Never relabel global rates as hero rates.
    pub fn recognition_cards(&self) -> Vec<Augment> {
        self.document("augments.json")
            .into_iter()
            .flat_map(|v| rows(&v["data"]))
            .filter_map(|a| {
                Some(Augment {
                    id: u32::try_from(a["id"].as_u64()?).ok().filter(|id| *id > 0)?,
                    name: clean(&a["name"]),
                    icon: clean(&a["iconUrl"]),
                    rarity: a["rarity"].as_u64().filter(|v| *v <= 2).map(|v| v as u8),
                    description: description(a["description"].as_str().unwrap_or("")),
                    tier: None,
                    win: None,
                    pick: None,
                    provenance: String::new(),
                })
            })
            .collect()
    }
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
        let mut detail = parse(row, items, self.locale);
        if let Some(catalog) = self.document("augments.json") {
            for a in &mut detail.augments {
                if let Some(base) =
                    rows(&catalog["data"]).find(|v| v["id"].as_u64() == Some(a.id as u64))
                {
                    a.description = description(
                        base["description"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .or_else(|| base["tooltip"].as_str())
                            .unwrap_or(""),
                    );
                }
            }
        }
        Ok(detail)
    }
}
fn parse(row: &Value, items: &Value, locale: Locale) -> Detail {
    let t = |s| word(locale, s);
    let stats = |v: &Value| stats(v, locale);
    let mut lines = Vec::new();
    let mut add = |text: String, icons: Vec<String>| {
        if lines.len() < 1024 {
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
        rows(&row["builds"])
            .filter(|b| b["queueId"].as_u64() == Some(2400))
            .take(6)
            .collect()
    } else {
        row.get("build")
            .filter(|v| v.is_object() && v["queueId"].as_u64() == Some(2400))
            .into_iter()
            .collect()
    };
    if builds.is_empty() {
        add(t("暂无海克斯出装数据").into(), vec![]);
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
        if b["source"] == crate::tencent::SOURCE {
            add(
                format!(
                    "腾讯101 · 统计日 {} · 统计补丁及样本数未提供",
                    clean(&b["date"])
                ),
                vec![],
            );
            add(
                "装备组合按选取率排列；成装组合不代表购买顺序。成长装备导入其可购买前置装备。"
                    .into(),
                vec![],
            );
            for r in rows(&b["conditionalItems"]) {
                let name = rows(&items["data"])
                    .find(|i| i["id"] == r["id"])
                    .map(|i| clean(&i["name"]))
                    .unwrap_or_default();
                add(format!("未导入：{name} · {}", clean(&r["reason"])), vec![]);
            }
            if b["excludedRows"].as_u64().unwrap_or(0) > 0 {
                add("已跳过包含条件装备或未确认装备的组合。".into(), vec![]);
            }
        }
        for r in rows(&b["summonerSpells"]).take(3) {
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
                format!("{} · {} · {}", t("召唤师技能"), names.join(" / "), stats(r)),
                rows(&r["summonerSpellIds"])
                    .filter_map(Value::as_u64)
                    .take(2)
                    .map(|id| format!("https://cdn.dtodo.cn/hextech/summoner-spell-icons/{id}.png"))
                    .collect(),
            );
        }
        for r in rows(&b["skillOrders"]).take(3) {
            let keys = rows(&r["skillKeys"])
                .take(18)
                .map(clean)
                .collect::<Vec<_>>()
                .join(" ");
            add(format!("{} · {keys}", t("1–18 级加点")), vec![]);
            add(stats(r), vec![]);
        }
        for (field, label) in [
            ("startingItems", "出门"),
            ("coreItems", "核心"),
            ("boots", "鞋子"),
            ("fullItems", "成装"),
            ("itemExtensions", "后续装备"),
            ("situationalItems", "备选装备"),
        ] {
            let mut entries: Vec<_> = rows(&b[field]).collect();
            entries.sort_by(|a, b| {
                b["games"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&a["games"].as_u64().unwrap_or(0))
                    .then_with(|| {
                        b["pickRate"]
                            .as_f64()
                            .unwrap_or(0.)
                            .total_cmp(&a["pickRate"].as_f64().unwrap_or(0.))
                    })
            });
            for r in entries.into_iter().take(match field {
                "startingItems" => 2,
                "coreItems" => 4,
                "fullItems" => 3,
                _ => 12,
            }) {
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
                    add(
                        format!(
                            "{} · {}",
                            t(label),
                            names.join(if b["source"] == crate::tencent::SOURCE {
                                " / "
                            } else {
                                " → "
                            })
                        ),
                        icons,
                    );
                    add(stats(r), vec![]);
                }
            }
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
    let rank = |v: &Value| v["stats"]["rank"].as_u64().filter(|v| *v > 0);
    let tier = |v: &Value| v["stats"]["tier"].as_u64().filter(|v| *v > 0);
    let authoritative = !augments.is_empty()
        && (augments.iter().all(|v| rank(v).is_some())
            || augments
                .iter()
                .any(|v| v["stats"]["rankSource"] == crate::tencent::SOURCE));
    let tier_priority = !authoritative && augments.iter().any(|v| tier(v).is_some());
    augments.sort_by(|a, b| {
        // Same full-pool precedence as 0.6.3 augment-ranking.ts; filtering never reranks.
        if authoritative {
            return rank(a)
                .unwrap_or(u64::MAX)
                .cmp(&rank(b).unwrap_or(u64::MAX));
        }
        let primary = if tier_priority {
            tier(a)
                .unwrap_or(u64::MAX)
                .cmp(&tier(b).unwrap_or(u64::MAX))
        } else {
            b["stats"]["recommendScore"]
                .as_f64()
                .unwrap_or(-1.)
                .total_cmp(&a["stats"]["recommendScore"].as_f64().unwrap_or(-1.))
        };
        primary
            .then_with(|| {
                b["stats"]["pickRate"]
                    .as_f64()
                    .unwrap_or(-1.)
                    .total_cmp(&a["stats"]["pickRate"].as_f64().unwrap_or(-1.))
            })
            .then_with(|| a["id"].as_u64().cmp(&b["id"].as_u64()))
    });
    let mut cards = Vec::new();
    for a in augments.into_iter().take(512) {
        let s = &a["stats"];
        if let Some(id) = a["id"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|id| *id > 0)
        {
            let valid_rate = |v: &Value| {
                v.as_f64()
                    .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            };
            cards.push(Augment {
                id,
                name: clean(&a["name"]),
                icon: clean(&a["iconUrl"]),
                rarity: a["rarity"].as_u64().filter(|v| *v <= 2).map(|v| v as u8),
                tier: s["tier"].as_u64(),
                win: valid_rate(&s["winRate"]),
                pick: valid_rate(&s["pickRate"]),
                description: description(a["description"].as_str().unwrap_or("")),
                provenance: format!(
                    "{} {} / {} · {} {}",
                    t("胜率来源"),
                    clean(s.get("winRateSource").unwrap_or(&s["source"])),
                    clean(s.get("winRateRegion").unwrap_or(&s["region"])),
                    t("补丁"),
                    clean(s.get("winRateGamePatch").unwrap_or(&s["gamePatch"]))
                ) + &if s["rankSource"] == crate::tencent::SOURCE {
                    format!(
                        " · 腾讯101推荐 #{} / {}（选取率，非胜率）",
                        s["rank"],
                        clean(&s["rankDate"])
                    )
                } else {
                    String::new()
                },
            });
        }
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
    let mut guides = Vec::new();
    for record in rows(&row["relatedBlogs"]).chain(rows(&c["relatedBlogs"])) {
        let url = record
            .as_str()
            .or_else(|| record["url"].as_str())
            .or_else(|| record["href"].as_str())
            .or_else(|| record["link"].as_str())
            .unwrap_or("");
        if guides.len() < 12 && guide_url(url) && !guides.iter().any(|g: &Guide| g.url == url) {
            guides.push(Guide {
                title: record["title"]
                    .as_str()
                    .or_else(|| record["name"].as_str())
                    .map(description)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "英雄攻略".into()),
                url: url.into(),
            });
        }
    }
    let (item_routes, item_sets) = crate::item_sets::create_indexed(
        c["id"].as_u64().unwrap_or(0) as u32,
        &clean(&c["name"]),
        &builds,
    )
    .into_iter()
    .unzip();
    Detail {
        title: format!("{} · {}", clean(&c["name"]), clean(&c["title"])),
        lines,
        augments: cards,
        guides,
        item_sets,
        item_routes,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detail_order_uses_authoritative_pool_rank_then_tier_and_pick_like_v063() {
        let mut row = serde_json::json!({"champion":{},"augments":[{"id":1,"stats":{"rank":2,"tier":1,"pickRate":0.9,"winRate":0.9}},{"id":2,"stats":{"rank":1,"tier":2,"pickRate":0.1,"winRate":0.4}}]});
        let data = serde_json::json!({"data":[]});
        assert_eq!(parse(&row, &data, Locale::ZhCn).augments[0].id, 2);
        row["augments"][1]["stats"]["rank"] = Value::Null;
        assert_eq!(parse(&row, &data, Locale::ZhCn).augments[0].id, 1);
        row["augments"][1]["stats"]["tier"] = serde_json::json!(1);
        row["augments"][1]["stats"]["pickRate"] = serde_json::json!(0.95);
        assert_eq!(parse(&row, &data, Locale::ZhCn).augments[0].id, 2);
    }
    #[test]
    fn complete_rarity_list_and_safe_guides_do_not_drop_low_ranked_augments() {
        let augments: Vec<_> = (1..=120).map(|id|serde_json::json!({"id":id,"name":format!("A{id}"),"rarity":id%3,"stats":{"winRate":(121-id) as f64/121.}})).collect();
        let row = serde_json::json!({"champion":{"name":"Fixture"},"augments":augments,"relatedBlogs":[{"title":"Guide","url":"https://example.com/guide"},{"url":"javascript:alert(1)"},{"url":"https://secret@example.com/private"}]});
        let detail = parse(&row, &serde_json::json!({"data":[]}), Locale::ZhCn);
        assert_eq!(detail.augments.len(), 120);
        for rarity in 0..3 {
            assert_eq!(
                detail
                    .augments
                    .iter()
                    .filter(|a| a.rarity == Some(rarity))
                    .count(),
                40
            );
        }
        assert_eq!(detail.augments.last().unwrap().id, 120);
        assert_eq!(detail.guides.len(), 1);
        assert_eq!(
            description("<attention>伤害</attention><br>提升&nbsp;20%"),
            "伤害 提升 20%"
        );
        for url in [
            "file:///C:/test",
            "https://example.com\\bad",
            "https://127.0.0.1:1234/",
            "https://example.com/\ncmd",
        ] {
            assert!(!guide_url(url));
        }
    }
    #[test]
    fn hextech_routes_exclude_other_modes_before_display_limit_and_import() {
        let standard = serde_json::json!({"queueId":450,"games":1000,"coreItems":[{"itemIds":[99],"games":40}]});
        let hextech = serde_json::json!({"queueId":2400,"games":1000,"coreItems":[{"itemIds":[88],"games":40}]});
        let mut routes = vec![standard.clone(); 7];
        routes.push(hextech);
        let items = serde_json::json!({"data":[]});
        let mut row = serde_json::json!({"champion":{"id":21,"name":"Test"},"builds":routes});
        let d = parse(&row, &items, Locale::ZhCn);
        assert_eq!(d.item_routes, vec![0]);
        assert_eq!(d.item_sets[0]["blocks"][0]["items"][0]["id"], "88");
        assert!(!d.lines.iter().any(|l| l.text.contains("装备 99")));
        row["builds"] =
            serde_json::json!([standard, {"games":1000,"coreItems":[{"itemIds":[77],"games":40}]}]);
        let d = parse(&row, &items, Locale::ZhCn);
        assert!(d.item_sets.is_empty());
        assert!(d.lines.iter().any(|l| l.text == "暂无海克斯出装数据"));
        assert!(!d.lines.iter().any(|l| l.text.starts_with("路线 ")));
    }
    #[test]
    fn details_follow_locale_without_translating_data_names() {
        let row = serde_json::json!({"champion":{"name":"fixture-name"},"build":{"queueId":2400}});
        let items = serde_json::json!({"data":[]});
        let en = parse(&row, &items, Locale::EnUs);
        assert!(en.title.starts_with("fixture-name"));
        assert!(en.lines.iter().any(|l| l.text.contains("ARAM Mayhem")));
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
            &serde_json::json!({"champion":{"name":"Test"},"build":{"queueId":2400,"coreItems":[{"itemIds":[99],"games":2,"winRate":null}]} }),
            &serde_json::json!({"data":[]}),
            Locale::ZhCn,
        );
        assert!(d.lines.iter().any(|l| l.text.contains("海克斯大乱斗")));
        assert!(d.lines.iter().any(|l| l.text.contains("装备 99")));
        assert!(d.lines.iter().any(|l| l.text.contains("胜 --")));
        assert!(d.lines.iter().any(|l| l.text.contains("未提供")));
    }
}
