//! Original rarity controls and description interaction. Painting never does IO.
use super::*;
use crate::desktop::{fill, rgb, text};
use fuze_data::champion_detail::Augment;

pub(super) fn cards(state: &State) -> Vec<&Augment> {
    state
        .reply
        .detail
        .iter()
        .flat_map(|d| &d.augments)
        .filter(|a| state.rarity == 0 || a.rarity == Some((state.rarity - 1) as u8))
        .collect()
}
pub(super) fn height(state: &State) -> i32 {
    80 + (cards(state).len() as i32 * 76).max(50)
}
fn hit(state: &State, l: &presentation::Layout, x: i32, y: i32) -> Option<u32> {
    let start = presentation::body_top(state, l) + 80;
    if state.active_tab != 0 || x < 16 || x >= l.width - 16 || y < start {
        return None;
    }
    let row = (y - start) / 76;
    if (y - start) % 76 >= 68 {
        return None;
    }
    cards(state).get(row as usize).map(|a| a.id)
}
pub(super) unsafe fn click(
    hwnd: HWND,
    state: &mut State,
    l: &presentation::Layout,
    x: i32,
    y: i32,
) -> bool {
    if state.active_tab != 0 {
        return false;
    }
    let top = presentation::body_top(state, l);
    if x >= 16 && x < l.width - 16 && y >= top + 30 && y < top + 62 {
        state.hover = None;
        state.inspected = None;
        state.rarity = ((x - 16) / ((l.width - 32) / 4).max(1)).min(3) as usize;
        presentation::scroll(hwnd, state, 0);
        return true;
    }
    if let Some(id) = hit(state, l, x, y) {
        state.inspected = (state.inspected != Some(id)).then_some(id);
        state.description_scroll = 0;
        state.hover = None;
        InvalidateRect(hwnd, null(), 0);
        return true;
    }
    if state.inspected.take().is_some() {
        InvalidateRect(hwnd, null(), 0);
    }
    false
}
pub(super) unsafe fn hover(hwnd: HWND, state: &mut State, px: i32, py: i32) {
    let l = presentation::layout(hwnd, state);
    let y = py * 96 / l.dpi;
    let next = if y >= 40 && y < l.height - 22 {
        hit(state, &l, px * 96 / l.dpi, y - 40 + state.page_scroll)
    } else {
        None
    };
    if state.hover != next {
        state.hover = next;
        if state.inspected.is_none() {
            state.description_scroll = 0;
        }
        InvalidateRect(hwnd, null(), 0);
    }
    let mut track = windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT>(
        ) as u32,
        dwFlags: windows_sys::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    windows_sys::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut track);
}
pub(super) unsafe fn draw(dc: HDC, state: &State, l: &presentation::Layout) {
    let x = |n| n * l.dpi / 96;
    let r = |a, b, c, d| RECT {
        left: x(a),
        top: x(b),
        right: x(c),
        bottom: x(d),
    };
    let label = |s: &str, a, b, c, d, size, color| text(dc, s, r(a, b, c, d), x(size), color);
    let top = presentation::body_top(state, l);
    let all = cards(state);
    label(
        "核心海克斯",
        16,
        top,
        l.width - 80,
        top + 26,
        14,
        rgb(230, 238, 248),
    );
    label(
        &format!("{}", all.len()),
        l.width - 65,
        top,
        l.width - 16,
        top + 26,
        12,
        rgb(160, 179, 202),
    );
    let colors = [
        rgb(125, 223, 205),
        rgb(193, 207, 224),
        rgb(222, 187, 109),
        rgb(190, 161, 242),
    ];
    for (i, name) in ["全部", "白银", "黄金", "棱彩"].iter().enumerate() {
        let a = 16 + i as i32 * ((l.width - 32) / 4);
        fill(
            dc,
            r(a, top + 30, a + (l.width - 32) / 4 - 5, top + 62),
            if state.rarity == i {
                rgb(40, 73, 82)
            } else {
                rgb(23, 42, 59)
            },
        );
        label(
            name,
            a + 8,
            top + 30,
            a + (l.width - 32) / 4 - 8,
            top + 62,
            12,
            colors[i],
        );
    }
    if all.is_empty() {
        label(
            "暂无符合条件的海克斯",
            24,
            top + 80,
            l.width - 24,
            top + 120,
            13,
            rgb(160, 179, 202),
        );
    }
    for (i, a) in all.iter().enumerate() {
        let y = top + 80 + i as i32 * 76;
        if y + 68 < state.page_scroll || y - state.page_scroll > l.height {
            continue;
        }
        fill(dc, r(16, y, l.width - 16, y + 68), rgb(43, 64, 87));
        fill(dc, r(17, y + 1, l.width - 17, y + 67), rgb(26, 43, 62));
        icon(dc, state, &a.icon, x(25), x(y + 14), x(40));
        label(
            &a.name,
            76,
            y + 8,
            l.width - 100,
            y + 35,
            14,
            rgb(230, 238, 248),
        );
        let rarity = a
            .rarity
            .map(|v| ["白银", "黄金", "棱彩"][v as usize])
            .unwrap_or("—");
        label(
            rarity,
            76,
            y + 37,
            l.width - 100,
            y + 59,
            11,
            colors[a.rarity.map(|v| v as usize + 1).unwrap_or(0)],
        );
        label(
            "胜率",
            l.width - 88,
            y + 9,
            l.width - 24,
            y + 27,
            10,
            rgb(160, 179, 202),
        );
        label(
            &a.win
                .map(|v| format!("{:.1}%", v * 100.))
                .unwrap_or("—".into()),
            l.width - 88,
            y + 29,
            l.width - 24,
            y + 59,
            20,
            rgb(125, 223, 205),
        );
    }
}
pub(super) unsafe fn tooltip(dc: HDC, state: &State, l: &presentation::Layout) {
    let Some(id) = state.inspected.or(state.hover) else {
        return;
    };
    let Some(a) = state
        .reply
        .detail
        .iter()
        .flat_map(|d| &d.augments)
        .find(|a| a.id == id)
    else {
        return;
    };
    let x = |n| n * l.dpi / 96;
    let r = |a, b, c, d| RECT {
        left: x(a),
        top: x(b),
        right: x(c),
        bottom: x(d),
    };
    let chars = ((l.width - 64)
        / if crate::i18n::locale() == "en-US" {
            7
        } else {
            14
        })
    .max(12) as usize;
    let description = if a.description.is_empty() {
        crate::i18n::text("数据源暂未提供说明")
    } else {
        &a.description
    };
    let parts: Vec<char> = description.chars().collect();
    let max_rows = ((l.height - 190) / 22).clamp(2, 14) as usize;
    let rows: Vec<String> = parts
        .chunks(chars)
        .skip(
            state
                .description_scroll
                .min(parts.len().div_ceil(chars).saturating_sub(max_rows)),
        )
        .take(max_rows)
        .map(|c| c.iter().collect())
        .collect();
    let height = 100 + rows.len() as i32 * 22;
    let top = (l.height - height - 30).max(42);
    fill(
        dc,
        r(20, top, l.width - 20, top + height),
        rgb(106, 139, 157),
    );
    fill(
        dc,
        r(21, top + 1, l.width - 21, top + height - 1),
        rgb(12, 27, 41),
    );
    text(
        dc,
        &a.name,
        r(32, top + 8, l.width - 32, top + 36),
        x(16),
        rgb(125, 223, 205),
    );
    for (i, s) in rows.iter().enumerate() {
        text(
            dc,
            s,
            r(
                32,
                top + 40 + i as i32 * 22,
                l.width - 32,
                top + 62 + i as i32 * 22,
            ),
            x(13),
            rgb(230, 238, 248),
        );
    }
    text(
        dc,
        &a.provenance,
        r(32, top + height - 52, l.width - 32, top + height - 30),
        x(10),
        rgb(160, 179, 202),
    );
    text(
        dc,
        "点击卡片固定说明 · 滚轮查看长文本",
        r(32, top + height - 29, l.width - 32, top + height - 7),
        x(11),
        rgb(160, 179, 202),
    );
}

pub(super) unsafe fn scroll_description(
    hwnd: HWND,
    state: &mut State,
    l: &presentation::Layout,
    x: i32,
    y: i32,
    delta: i32,
) -> bool {
    let Some(id) = state.inspected else {
        return false;
    };
    let Some(a) = state
        .reply
        .detail
        .as_ref()
        .and_then(|d| d.augments.iter().find(|a| a.id == id))
    else {
        return false;
    };
    let chars = ((l.width - 64)
        / if crate::i18n::locale() == "en-US" {
            7
        } else {
            14
        })
    .max(12) as usize;
    let count = a.description.chars().count().div_ceil(chars).max(1);
    let max_rows = ((l.height - 190) / 22).clamp(2, 14) as usize;
    let height = 100 + count.min(max_rows) as i32 * 22;
    let top = (l.height - height - 30).max(42);
    if x < 20 || x >= l.width - 20 || y < top || y >= top + height {
        return false;
    }
    state.description_scroll = if delta > 0 {
        state.description_scroll.saturating_sub(3)
    } else {
        (state.description_scroll + 3).min(count.saturating_sub(max_rows))
    };
    InvalidateRect(hwnd, null(), 0);
    true
}
