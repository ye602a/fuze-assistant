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
/// Convert only for presentation. Stored/exported timestamps remain UTC milliseconds.
pub fn date(ms: u64) -> String {
    date_with(ms, |utc| {
        let mut local = unsafe { std::mem::zeroed() };
        // A null zone uses Windows' current zone and its historical daylight rules.
        let ok = unsafe {
            windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTimeEx(
                std::ptr::null(),
                utc,
                &mut local,
            )
        };
        (ok != 0).then_some(local)
    })
}
fn date_with(
    ms: u64,
    localize: impl FnOnce(
        &windows_sys::Win32::Foundation::SYSTEMTIME,
    ) -> Option<windows_sys::Win32::Foundation::SYSTEMTIME>,
) -> String {
    if ms > 253402300799999 {
        return "--".into();
    }
    let ticks = (ms + 11644473600000) * 10000;
    let file_time = windows_sys::Win32::Foundation::FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = unsafe { std::mem::zeroed() };
    if unsafe { windows_sys::Win32::System::Time::FileTimeToSystemTime(&file_time, &mut utc) } == 0
    {
        return "--".into();
    }
    let (time, zone) = match localize(&utc) {
        Some(time) if (1..=9999).contains(&time.wYear) => (time, "本地"),
        _ => (utc, "UTC"),
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} {zone}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_dates_and_conversion_failure_are_explicit() {
        assert_eq!(date(u64::MAX), "--");
        assert_eq!(date(253402300800000), "--");
        assert_eq!(date_with(0, |_| None), "1970-01-01 00:00 UTC");
        assert_eq!(date_with(1709164800000, |_| None), "2024-02-29 00:00 UTC");
        assert_eq!(date_with(253402300799999, |_| None), "9999-12-31 23:59 UTC");
    }
    fn in_zone(ms: u64, zone: windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION) -> String {
        date_with(ms, |utc| {
            let mut local = unsafe { std::mem::zeroed() };
            let ok = unsafe {
                windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTime(
                    &zone, utc, &mut local,
                )
            };
            (ok != 0).then_some(local)
        })
    }
    #[test]
    fn local_dates_cover_offsets_midnight_and_leap_day() {
        let mut zone: windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION =
            unsafe { std::mem::zeroed() };
        zone.Bias = -480;
        assert_eq!(in_zone(1709164800000, zone), "2024-02-29 08:00 本地");
        assert_eq!(in_zone(1709226000000, zone), "2024-03-01 01:00 本地");
        zone.Bias = 300;
        assert_eq!(in_zone(0, zone), "1969-12-31 19:00 本地");
    }
    #[test]
    fn local_dates_follow_daylight_rules_at_both_transitions() {
        let mut zone: windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION =
            unsafe { std::mem::zeroed() };
        zone.Bias = 300;
        zone.DaylightBias = -60;
        zone.DaylightDate.wMonth = 3;
        zone.DaylightDate.wDay = 2;
        zone.DaylightDate.wHour = 2;
        zone.StandardDate.wMonth = 11;
        zone.StandardDate.wDay = 1;
        zone.StandardDate.wHour = 2;
        assert_eq!(in_zone(1710053940000, zone), "2024-03-10 01:59 本地");
        assert_eq!(in_zone(1710054000000, zone), "2024-03-10 03:00 本地");
        assert_eq!(in_zone(1730613540000, zone), "2024-11-03 01:59 本地");
        assert_eq!(in_zone(1730613600000, zone), "2024-11-03 01:00 本地");
    }
    #[test]
    fn unknown_ids_are_explicit() {
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
