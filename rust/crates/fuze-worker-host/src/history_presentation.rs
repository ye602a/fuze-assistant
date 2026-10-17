//! Account-isolated cached match list. Filters and pagination never issue a network request.
use super::*;
pub(super) const CHAMPION: usize = 226;
pub(super) const RESULT: usize = 227;
pub(super) const COPY: usize = 228;
pub(super) const MORE: usize = 233;
pub(super) const PAGE: usize = 6;
pub(super) fn indices(u: &Ui) -> Vec<usize> {
    u.shown
        .history
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            (u.history_champion == 0 || r.champion == u.history_champion)
                && (u.history_result == 0
                    || r.result == ["", "win", "loss", "remake"][u.history_result.min(3)])
        })
        .map(|(i, _)| i)
        .collect()
}
fn heroes(u: &Ui) -> Vec<(u32, String)> {
    let mut seen = std::collections::BTreeMap::new();
    for (i, r) in u.shown.history.rows.iter().enumerate() {
        seen.entry(r.champion).or_insert_with(|| {
            u.shown
                .history
                .details
                .get(i)
                .map(|d| d.title.clone())
                .unwrap_or_else(|| format!("#{}", r.champion))
        });
    }
    seen.into_iter().collect()
}
pub(super) fn height(u: &Ui) -> i32 {
    424 + indices(u).len().saturating_sub(u.history_page).min(PAGE) as i32 * 154
}
pub(super) unsafe fn controls(hwnd: HWND, u: &Ui) {
    let more = GetDlgItem(hwnd, MORE as i32);
    ShowWindow(
        more,
        if u.records && u.history && !u.journal {
            SW_SHOWNA
        } else {
            SW_HIDE
        },
    );
    SetWindowTextW(more, wide("加载更早战绩").as_ptr());
    EnableWindow(
        more,
        i32::from(
            !u.smoke
                && u.desired == PAUSED
                && u.shown.mode == PAUSED
                && u.shown.history.next_start.is_some(),
        ),
    );
    let copy = GetDlgItem(hwnd, COPY as i32);
    ShowWindow(
        copy,
        if u.records && u.history && !u.journal {
            SW_SHOWNA
        } else {
            SW_HIDE
        },
    );
    SetWindowTextW(copy, wide("复制匿名分享图片").as_ptr());
    EnableWindow(
        copy,
        i32::from(
            !u.smoke && u.desired == PAUSED && u.shown.history.rows.get(u.history_offset).is_some(),
        ),
    );
    for (id, label) in [
        (
            CHAMPION,
            if u.history_champion == 0 {
                crate::i18n::text("全部英雄").into()
            } else {
                heroes(u)
                    .into_iter()
                    .find(|(id, _)| *id == u.history_champion)
                    .map(|(_, n)| n)
                    .unwrap_or("—".into())
            },
        ),
        (
            RESULT,
            crate::i18n::text(["全部结果", "胜利", "失败", "重开"][u.history_result.min(3)])
                .to_owned(),
        ),
    ] {
        let child = GetDlgItem(hwnd, id as i32);
        ShowWindow(
            child,
            if u.records && u.history && !u.journal {
                SW_SHOWNA
            } else {
                SW_HIDE
            },
        );
        SetWindowTextW(child, wide(&label).as_ptr());
    }
}
pub(super) unsafe fn layout(hwnd: HWND, u: &Ui) {
    if !u.records || !u.history || u.journal {
        return;
    }
    let d = workbench::dpi(hwnd);
    let x = |n| n * d / 96;
    let width = workbench::bounds(hwnd, u).right * 96 / d;
    for (id, left, top, w) in [
        (CHAMPION, 28, 246, 240),
        (RESULT, 280, 246, 150),
        (HISTORY_REFRESH, width - 188, 246, 160),
        (EXPORT_MATCH, 28, 290, 230),
        (JOURNAL_VIEW, 270, 290, 230),
        (COPY, 512, 290, 230),
        (HISTORY_PAGE, 28, 328, 170),
        (HISTORY_IMPORT, 208, 328, 170),
        (HISTORY_ACCEPT, 388, 328, 170),
        (JOURNAL_IMPORT, 568, 328, 190),
        (HISTORY_SAVED, 438, 246, 190),
        (HISTORY_PREV, 28, height(u) - 46, 140),
        (HISTORY_NEXT, 184, height(u) - 46, 140),
        (MORE, 340, height(u) - 46, 200),
    ] {
        workbench::place(
            hwnd,
            GetDlgItem(hwnd, id as i32),
            x(left),
            x(top),
            x(w),
            x(30),
            1,
        );
    }
}
pub(super) unsafe fn filter(hwnd: HWND, u: &mut Ui, id: usize) {
    if u.smoke {
        return;
    }
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let options = if id == CHAMPION {
        let mut v = vec![(0, crate::i18n::text("全部英雄").into())];
        v.extend(heroes(u));
        v
    } else {
        ["全部结果", "胜利", "失败", "重开"]
            .into_iter()
            .enumerate()
            .map(|(i, s)| (i as u32, crate::i18n::text(s).into()))
            .collect()
    };
    for (i, (_, name)) in options.iter().enumerate() {
        AppendMenuW(menu, MF_STRING, i + 1, wide(name).as_ptr());
    }
    let mut r: RECT = zeroed();
    GetWindowRect(GetDlgItem(hwnd, id as i32), &mut r);
    let result = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_NONOTIFY,
        r.left,
        r.bottom,
        0,
        hwnd,
        null(),
    );
    DestroyMenu(menu);
    if result > 0 {
        if let Some((value, _)) = options.get(result as usize - 1) {
            if id == CHAMPION {
                u.history_champion = *value;
            } else {
                u.history_result = *value as usize;
            }
            u.history_page = 0;
            u.history_offset = indices(u).first().copied().unwrap_or(usize::MAX);
            u.scroll = 0;
            settings_controls(hwnd, u);
            SendMessageW(hwnd, WM_SIZE, 0, 0);
        }
    }
}
pub(super) unsafe fn click(hwnd: HWND, u: &mut Ui, px: i32, py: i32) {
    if !u.records || !u.history || u.journal {
        return;
    }
    let d = workbench::dpi(hwnd);
    let x = (px - workbench::rail(hwnd)) * 96 / d;
    let y = (py + u.scroll) * 96 / d;
    let width = workbench::bounds(hwnd, u).right * 96 / d;
    if x < 28 || x >= width - 28 || y < 374 {
        return;
    }
    let slot = ((y - 374) / 154) as usize;
    if slot >= PAGE || (y - 374) % 154 >= 142 {
        return;
    }
    if let Some(index) = indices(u).get(u.history_page + slot) {
        u.history_offset = *index;
        settings_controls(hwnd, u);
        InvalidateRect(hwnd, null(), 0);
    }
}
pub(super) unsafe fn draw(dc: HDC, hwnd: HWND, u: &Ui) {
    let d = workbench::dpi(hwnd);
    let x = |n| n * d / 96;
    let width = workbench::bounds(hwnd, u).right * 96 / d;
    let rect = |a, b, c, e| RECT {
        left: x(a),
        top: x(b),
        right: x(c),
        bottom: x(e),
    };
    let list = indices(u);
    if list.is_empty() {
        text(
            dc,
            "暂无符合条件的战绩",
            rect(28, 374, width - 28, 420),
            x(15),
            rgb(160, 180, 190),
        );
        return;
    }
    for (slot, &index) in list.iter().skip(u.history_page).take(PAGE).enumerate() {
        let top = 374 + slot as i32 * 154;
        let row = &u.shown.history.rows[index];
        let detail = u.shown.history.details.get(index);
        let color = match row.result.as_str() {
            "win" => rgb(113, 207, 173),
            "loss" => rgb(221, 139, 137),
            _ => rgb(191, 177, 139),
        };
        fill(
            dc,
            rect(28, top, width - 28, top + 142),
            if index == u.history_offset {
                rgb(200, 170, 110)
            } else {
                rgb(55, 75, 85)
            },
        );
        fill(
            dc,
            rect(29, top + 1, width - 29, top + 141),
            rgb(16, 32, 45),
        );
        let title = detail
            .map(|d| d.title.clone())
            .unwrap_or_else(|| format!("#{}", row.champion));
        if let Some(url) = detail
            .and_then(|d| d.lines.first())
            .and_then(|l| l.icons.first())
        {
            crate::champ_select::draw_icon(dc, &u.history_reply, url, x(42), x(top + 12), x(42));
        }
        text(
            dc,
            &title,
            rect(96, top + 8, width - 250, top + 34),
            x(16),
            rgb(230, 238, 248),
        );
        text(
            dc,
            &format!(
                "{} · {}:{:02}",
                crate::history_detail::date(row.created),
                row.duration / 60,
                row.duration % 60
            ),
            rect(96, top + 34, width - 250, top + 56),
            x(11),
            rgb(160, 180, 190),
        );
        text(
            dc,
            match row.result.as_str() {
                "win" => "胜利",
                "loss" => "失败",
                _ => "重开",
            },
            rect(width - 238, top + 12, width - 165, top + 40),
            x(17),
            color,
        );
        text(
            dc,
            &format!("{}/{}/{}", row.kills, row.deaths, row.assists),
            rect(width - 155, top + 12, width - 44, top + 40),
            x(20),
            rgb(230, 238, 248),
        );
        if let Some(detail) = detail {
            for (kind, label) in [(1, "装备"), (2, "战绩强化")] {
                let y = top + 62 + (kind as i32 - 1) * 36;
                text(
                    dc,
                    label,
                    rect(42, y, 112, y + 28),
                    x(11),
                    rgb(160, 180, 190),
                );
                if let Some(line) = detail.lines.get(kind) {
                    for (i, url) in line.icons.iter().take(12).enumerate() {
                        crate::champ_select::draw_icon(
                            dc,
                            &u.history_reply,
                            url,
                            x(116 + i as i32 * 34),
                            x(y),
                            x(28),
                        );
                    }
                    text(
                        dc,
                        &line.text,
                        rect(540, y, width - 42, y + 28),
                        x(10),
                        rgb(160, 180, 190),
                    );
                }
            }
        }
    }
}

pub(super) unsafe fn copy(hwnd: HWND, u: &mut Ui) {
    if u.smoke || u.desired != PAUSED || !u.records || !u.history || u.journal {
        return;
    }
    let result = (|| {
        let h = &u.shown.history;
        let row = h.rows.get(u.history_offset).ok_or("share-no-match")?;
        let data = crate::record_export::encode(row, u.observation(), h.cached)?;
        let png = crate::record_export::png(
            &data,
            h.details.get(u.history_offset).map(|d| d.as_ref()),
            Some(&u.history_reply),
        )?;
        crate::record_export::copy_image(hwnd, &png)
    })();
    if let Ok(mut status) = u.shared.status.lock() {
        status.notice = Some(if result.is_ok() {
            "匿名分享图片已复制"
        } else {
            "图片复制未完成，请稍后重试"
        });
    }
}
