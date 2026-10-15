//! Geometry and visual roles follow the existing AugmentFloatingOverlay.vue.
use super::*;
pub(super) unsafe fn draw(dc: HDC, width: i32, height: i32, snapshot: Option<&Snapshot>) {
    let sx = width as f64 / 760.;
    let sy = height as f64 / 170.;
    let rect = |l: i32, t: i32, r: i32, b: i32| RECT {
        left: (l as f64 * sx) as i32,
        top: (t as f64 * sy) as i32,
        right: (r as f64 * sx) as i32,
        bottom: (b as f64 * sy) as i32,
    };
    let locale = UI_LOCALE.with(|l| match l.get() {
        1 => "zh-TW",
        2 => "en-US",
        _ => "zh-CN",
    });
    let t = |cn, tw, en| match locale {
        "en-US" => en,
        "zh-TW" => tw,
        _ => cn,
    };
    let label_at = |s: &str, l, top, r, b, size, color| {
        label(dc, s, rect(l, top, r, b), (size as f64 * sy) as i32, color)
    };
    let gold = rgb(200, 170, 110);
    let pale = rgb(240, 230, 210);
    let dim = rgb(168, 182, 189);
    fill(dc, rect(0, 0, 760, 170), rgb(5, 16, 25));
    label_at("FUZE", 9, 6, 55, 27, 11, gold);
    label_at(
        t("海克斯推荐", "海克斯推薦", "Augments"),
        62,
        6,
        165,
        27,
        11,
        dim,
    );
    label_at(
        if RETAINED.with(|r| r.get()) {
            t(
                "等待刷新 · 暂留上次结果 · 整体胜率",
                "等待更新 · 暫留上次結果 · 整體勝率",
                "Updating · Previous results · Overall rates",
            )
        } else {
            t(
                "整体胜率推荐 · 实际领取轮次未确认",
                "整體勝率推薦 · 實際領取輪次未確認",
                "Overall win rates · Claim round unconfirmed",
            )
        },
        175,
        6,
        720,
        27,
        11,
        gold,
    );
    // Draw the close mark as geometry, independent of the installed font.
    let pen = CreatePen(PS_SOLID, 1, gold);
    let old = SelectObject(dc, pen);
    let a = rect(738, 11, 747, 20);
    MoveToEx(dc, a.left, a.top, null_mut());
    LineTo(dc, a.right, a.bottom);
    MoveToEx(dc, a.right, a.top, null_mut());
    LineTo(dc, a.left, a.bottom);
    SelectObject(dc, old);
    DeleteObject(pen);
    let empty = std::array::from_fn(|_| Slot::Unreadable);
    let slots = snapshot.map(|s| &s.slots).unwrap_or(&empty);
    let stats: [Option<fuze_data::recommendations::Recommendation>; 3] = STATS.with(|s| {
        let map = s.borrow();
        std::array::from_fn(|i| match &slots[i] {
            Slot::Known { id, .. } => map.get(id).cloned(),
            _ => None,
        })
    });
    let best = stats
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.as_ref()?.score.filter(|s| s.is_finite()).map(|s| (i, s)))
        .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|v| v.0);
    for (i, slot) in slots.iter().enumerate() {
        let left = 8 + i as i32 * 248;
        let top = 36;
        let right = left + 240;
        let known = matches!(slot, Slot::Known { .. });
        let priority = best == Some(i);
        fill(
            dc,
            rect(left, top, right, 162),
            if priority { gold } else { rgb(71, 83, 88) },
        );
        fill(
            dc,
            rect(left + 1, top + 1, right - 1, 161),
            if priority {
                rgb(32, 43, 43)
            } else {
                rgb(14, 29, 40)
            },
        );
        fill(
            dc,
            rect(left, top, left + if priority { 34 } else { 16 }, top + 3),
            if priority {
                rgb(240, 216, 157)
            } else {
                rgb(142, 156, 170)
            },
        );
        let icon_rect = rect(left + 10, top + 20, left + 76, top + 86);
        fill(
            dc,
            icon_rect,
            if known {
                rgb(136, 112, 68)
            } else {
                rgb(46, 64, 80)
            },
        );
        fill(
            dc,
            rect(left + 13, top + 23, left + 73, top + 83),
            rgb(3, 11, 19),
        );
        let (id, name) = match slot {
            Slot::Known { id, name, .. } => (*id, name.as_str()),
            _ => (0, t("此位置未识别", "此位置未辨識", "Unreadable slot")),
        };
        let art = ICONS.with(|a| a.borrow().get(&id).cloned());
        if let Some(art) = art.filter(|a| a.len() == 48 * 48 * 4) {
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = 48;
            info.bmiHeader.biHeight = -48;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let r = rect(left + 13, top + 23, left + 73, top + 83);
            StretchDIBits(
                dc,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                0,
                0,
                48,
                48,
                art.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
        if !known {
            label_at(name, left + 86, top + 30, right - 8, top + 54, 12, dim);
            label_at(
                t("可能被遮挡", "可能被遮擋", "May be obscured"),
                left + 86,
                top + 58,
                right - 8,
                top + 79,
                11,
                dim,
            );
            label_at(
                t(
                    "等待补全或刷新",
                    "等待補全或更新",
                    "Waiting for recognition",
                ),
                left + 86,
                top + 80,
                right - 8,
                top + 101,
                11,
                dim,
            );
            continue;
        }
        if priority {
            label_at(
                t("优先推荐", "優先推薦", "Top pick"),
                right - 71,
                top - 7,
                right - 6,
                top + 8,
                10,
                gold,
            );
        }
        label_at(name, left + 86, top + 12, right - 8, top + 34, 15, pale);
        let row = stats[i].as_ref();
        let percent = |v: Option<f64>| {
            v.filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                .map(|v| format!("{:.1}%", v * 100.))
                .unwrap_or("—".into())
        };
        label_at(
            t("选取率", "選取率", "Pick"),
            left + 86,
            top + 39,
            left + 158,
            top + 55,
            10,
            dim,
        );
        label_at(
            t("胜率", "勝率", "Win"),
            left + 164,
            top + 39,
            right - 6,
            top + 55,
            10,
            dim,
        );
        label_at(
            &percent(row.and_then(|r| r.pick_rate)),
            left + 86,
            top + 56,
            left + 159,
            top + 84,
            21,
            if priority { gold } else { pale },
        );
        label_at(
            &percent(row.and_then(|r| r.win_rate)),
            left + 164,
            top + 56,
            right - 6,
            top + 84,
            21,
            if priority { gold } else { pale },
        );
        label_at(
            t("推荐度", "推薦度", "Recommendation"),
            left + 10,
            top + 91,
            left + 131,
            top + 108,
            10,
            dim,
        );
        let score = row
            .and_then(|r| r.score)
            .filter(|s| s.is_finite())
            .map(|s| s.clamp(0., 1.));
        label_at(
            &score
                .map(|s| format!("{:.0} / 100", s * 100.))
                .unwrap_or(t("暂无统计", "暫無統計", "No statistics").into()),
            left + 140,
            top + 91,
            right - 9,
            top + 108,
            10,
            dim,
        );
        fill(
            dc,
            rect(left + 10, top + 110, right - 10, top + 119),
            rgb(54, 86, 104),
        );
        fill(
            dc,
            rect(left + 11, top + 111, right - 11, top + 118),
            rgb(2, 11, 19),
        );
        if let Some(score) = score {
            fill(
                dc,
                rect(
                    left + 11,
                    top + 111,
                    left + 11 + (218. * score) as i32,
                    top + 118,
                ),
                if priority { gold } else { rgb(97, 212, 223) },
            );
        }
    }
}
