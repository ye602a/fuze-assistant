//! Public local catalogue presentation of private result IDs; no network or account lookup.
use fuze_data::champion_detail::{Detail, Line};
use serde_json::Value;
fn clean(v: &Value) -> String {
    v.as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect()
}
fn asset(doc: &Value, id: u32) -> &Value {
    let d = doc.get("data").unwrap_or(doc);
    let key = id.to_string();
    if let Some(a) = d.as_array() {
        a.iter()
            .take(10000)
            .find(|r| r["id"].as_u64() == Some(id as u64) || r["id"].as_str() == Some(&key))
            .unwrap_or(&Value::Null)
    } else {
        d.get(key).unwrap_or(&Value::Null)
    }
}
pub fn name(v: &Value, id: u32) -> String {
    let n = clean(&v["name"]);
    if n.is_empty() {
        format!("#{id}")
    } else {
        n
    }
}
pub fn populate(
    view: &mut crate::account_history::View,
    roots: &[fuze_data::Root],
    locale: fuze_data::Locale,
) {
    view.details = Default::default();
    view.asset_version.clear();
    view.augment_names = Default::default();
    if view.rows.is_empty() && view.observations.is_empty() {
        return;
    }
    let Ok(data) = fuze_data::prepare(roots, locale, fuze_data::Scope::History) else {
        return;
    };
    view.asset_version = data.identity();
    let empty = Value::Null;
    let augments = data.document("augments.json").unwrap_or(&empty);
    let ids: std::collections::HashSet<u32> = view
        .observations
        .iter()
        .flat_map(|o| o.events.iter())
        .flat_map(|e| e.cards.iter().flatten().copied())
        .collect();
    view.augment_names = ids
        .into_iter()
        .map(|id| (id, name(asset(augments, id), id)))
        .collect::<std::collections::HashMap<_, _>>()
        .into();
    view.details = view
        .rows
        .iter()
        .map(|row| std::sync::Arc::new(present(row, &data)))
        .collect::<Vec<_>>()
        .into();
}
fn present(row: &fuze_lcu::history::Match, data: &fuze_data::Prepared) -> Detail {
    let empty = Value::Null;
    let hero = asset(
        data.document("champions.json").unwrap_or(&empty),
        row.champion,
    );
    let mut lines = vec![Line {
        text: name(hero, row.champion),
        icons: vec![clean(&hero["iconUrl"])],
    }];
    for (ids, file) in [(&row.items, "items.json"), (&row.augments, "augments.json")] {
        let doc = data.document(file).unwrap_or(&empty);
        lines.push(Line {
            text: ids
                .iter()
                .map(|id| name(asset(doc, *id), *id))
                .collect::<Vec<_>>()
                .join(" · "),
            icons: ids
                .iter()
                .map(|id| clean(&asset(doc, *id)["iconUrl"]))
                .collect(),
        });
    }
    Detail {
        title: name(hero, row.champion),
        lines,
        ..Default::default()
    }
}
/// UTC explicitly labelled; does not imply the time zone from the old machine.
pub fn date(ms: u64) -> String {
    if ms > 253402300799999 {
        return "--".into();
    }
    let seconds = ms / 1000;
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        seconds % 86400 / 3600,
        seconds % 3600 / 60
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn date_and_unknown_ids_are_explicit() {
        assert_eq!(date(0), "1970-01-01 00:00 UTC");
        assert_eq!(date(1709164800000), "2024-02-29 00:00 UTC");
        assert_eq!(name(asset(&serde_json::json!({"data":[]}), 99), 99), "#99");
        assert_eq!(
            name(
                asset(&serde_json::json!({"data":{"1001":{"name":"Boots"}}}), 1001),
                1001
            ),
            "Boots"
        );
        assert_eq!(
            name(
                asset(&serde_json::json!([{"id":7,"name":"Fixture augment"}]), 7),
                7
            ),
            "Fixture augment"
        );
    }
}
