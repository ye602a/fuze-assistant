//! Restores the vertical Chinese popup: team, bench overview, hero, stats, detail tabs.
use super::*;
use crate::desktop::{fill, rgb, text};
use windows_sys::Win32::UI::Controls::SetScrollInfo;
pub(super) struct Layout {
    pub width: i32,
    pub height: i32,
    pub dpi: i32,
    pub columns: i32,
    pub bench_top: i32,
    pub hero_top: i32,
    pub tabs_top: i32,
    pub detail_top: i32,
}
impl Layout {
    pub fn new(width: i32, height: i32, dpi: i32, count: usize) -> Self {
        let width = width * 96 / dpi;
        let height = height * 96 / dpi;
        let columns = ((width - 32) / 96).clamp(3, 8);
        let rows = ((count as i32 + columns - 1) / columns).max(1);
        let hero_top = 196 + rows * 88 + 16;
        Self {
            width,
            height,
            dpi,
            columns,
            bench_top: 196,
            hero_top,
            tabs_top: hero_top + 150,
            detail_top: hero_top + 196,
        }
    }
    fn card(&self, i: usize, team: bool) -> RECT {
        let columns = if team { 5 } else { self.columns };
        let width = (self.width - 32) / columns;
        let l = 16 + (i as i32 % columns) * width;
        let top = if team {
            42
        } else {
            self.bench_top + (i as i32 / columns) * 88
        };
        RECT {
            left: l,
            top,
            right: l + width - 6,
            bottom: top + if team { 100 } else { 80 },
        }
    }
    pub fn hit(&self, x: i32, y: i32, view: &View) -> Option<u32> {
        let inside = |r: RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        view.team
            .iter()
            .enumerate()
            .find(|(i, _)| inside(self.card(*i, true)))
            .or_else(|| {
                view.bench
                    .iter()
                    .enumerate()
                    .find(|(i, _)| inside(self.card(*i, false)))
            })
            .map(|(_, h)| h.id)
            .filter(|id| *id > 0)
    }
}
pub(super) unsafe fn layout(hwnd: HWND, state: &State) -> Layout {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let mut l = Layout::new(
        r.right,
        r.bottom,
        GetDpiForWindow(hwnd).max(96) as i32,
        state.view.as_ref().map(|v| v.bench.len()).unwrap_or(0),
    );
    let guides = state
        .reply
        .detail
        .as_ref()
        .map(|d| d.guides.len())
        .unwrap_or(0) as i32;
    if state.side {
        l.hero_top = -150;
        l.tabs_top = 8;
        l.detail_top = 54;
    }
    if guides > 0 && !state.side {
        l.tabs_top += 30 + guides * 32;
        l.detail_top += 30 + guides * 32;
    }
    l
}
fn section(text: &str) -> Option<usize> {
    if ["海克斯强化", "海克斯增幅", "Augments ·"]
        .iter()
        .any(|s| text.starts_with(s))
    {
        Some(1)
    } else if text.starts_with("1–18") {
        Some(2)
    } else if ["符文页", "符文頁", "Runes:"]
        .iter()
        .any(|s| text.starts_with(s))
    {
        Some(3)
    } else if ["路线", "路線", "Build "]
        .iter()
        .any(|s| text.starts_with(s))
    {
        Some(0)
    } else {
        None
    }
}
pub(super) fn source_line(text: &str) -> bool {
    ["胜率来源", "勝率來源", "Win-rate source"]
        .iter()
        .any(|p| text.starts_with(p))
}
fn lines(state: &State) -> Vec<&fuze_data::champion_detail::Line> {
    let mut active = 0;
    let mut route = 0;
    state
        .reply
        .detail
        .iter()
        .flat_map(|d| &d.lines)
        .filter(|l| {
            if let Some(tab) = section(&l.text) {
                active = tab;
                if tab == 0 {
                    route += 1;
                }
            }
            if state.active_tab == 0 {
                active == 1 && !source_line(&l.text)
            } else {
                active != 1 && route == state.build_route + 1
            }
        })
        .collect()
}
fn route_titles(state: &State) -> Vec<&str> {
    state
        .reply
        .detail
        .iter()
        .flat_map(|d| &d.lines)
        .filter(|l| section(&l.text) == Some(0))
        .map(|l| l.text.as_str())
        .collect()
}
fn skill_keys(line: &fuze_data::champion_detail::Line) -> Option<Vec<&str>> {
    if !line.text.starts_with("1–18") {
        return None;
    }
    let keys: Vec<_> = line.text.split('·').nth(1)?.split_whitespace().collect();
    (!keys.is_empty()
        && keys.len() <= 18
        && keys.iter().all(|k| matches!(*k, "Q" | "W" | "E" | "R")))
    .then_some(keys)
}
fn line_height(line: &fuze_data::champion_detail::Line, layout: &Layout) -> i32 {
    if skill_keys(line).is_some() {
        return 150;
    }
    let inline = line.icons.len() == 1;
    let chars = ((layout.width - if inline { 100 } else { 48 })
        / if crate::i18n::locale() == "en-US" {
            7
        } else {
            14
        })
    .max(10) as usize;
    (wrap(&line.text, chars).len() as i32 * 22
        + if line.icons.is_empty() || inline {
            14
        } else {
            52
        })
    .max(if inline { 69 } else { 0 })
}
pub(super) fn body_top(state: &State, layout: &Layout) -> i32 {
    layout.detail_top
        + if state.active_tab == 0 && !state.side {
            40 + if state.stage_panel { 480 } else { 0 }
        } else if state.active_tab == 1 {
            if route_titles(state).is_empty() {
                0
            } else {
                60
            }
        } else {
            0
        }
}
pub(super) fn total(state: &State, layout: &Layout) -> i32 {
    body_top(state, layout)
        + if state.active_tab == 0 {
            augment_ui::height(state)
        } else {
            lines(state)
                .iter()
                .map(|l| line_height(l, layout))
                .sum::<i32>()
                .max(80)
        }
        + 24
}
pub(super) fn content_bottom(state: &State, layout: &Layout) -> i32 {
    layout.height - if state.side { 22 } else { 64 }
}
pub(super) unsafe fn scroll(hwnd: HWND, state: &mut State, delta: i32) {
    if delta != 0 {
        state.hover = None;
        state.inspected = None;
    }
    let l = layout(hwnd, state);
    let content = total(state, &l);
    let viewport = (content_bottom(state, &l) - 40).max(1);
    state.page_scroll = state
        .page_scroll
        .saturating_add(delta)
        .clamp(0, (content - viewport).max(0));
    let si = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: content - 1,
        nPage: viewport as u32,
        nPos: state.page_scroll,
        nTrackPos: 0,
    };
    SetScrollInfo(hwnd, SB_VERT, &si, 1);
    if state.reply.detail.is_some() {
        let mut icons = state
            .view
            .iter()
            .flat_map(|v| v.team.iter().chain(&v.bench).map(|h| h.icon.clone()))
            .collect::<Vec<_>>();
        if state.active_tab == 0 {
            let start = body_top(state, &l) + 80;
            let first = ((state.page_scroll - start).max(0) / 76) as usize;
            icons.extend(
                augment_ui::cards(state)
                    .into_iter()
                    .skip(first)
                    .take(((l.height / 76) + 3) as usize)
                    .map(|a| a.icon.clone()),
            );
        } else {
            let mut top = body_top(state, &l);
            for line in lines(state) {
                let height = line_height(line, &l);
                if top + height >= state.page_scroll && top < state.page_scroll + l.height {
                    icons.extend(line.icons.iter().cloned());
                }
                top += height;
            }
        }
        state.assets.prioritize(icons);
    }
    InvalidateRect(hwnd, null(), 0);
}
pub(super) unsafe fn click(hwnd: HWND, state: &mut State, px: i32, py: i32) {
    let l = layout(hwnd, state);
    let x = px * 96 / l.dpi;
    let y = py * 96 / l.dpi;
    if x >= l.width - 40 && y < 40 {
        SendMessageW(hwnd, WM_CLOSE, 0, 0);
        return;
    }
    if augment_ui::description_click(hwnd, state, &l, x, y) {
        return;
    }
    if y >= l.height - 60 && y < l.height - 24 && x >= 16 && x < l.width - 16 && !state.side {
        state.install_items();
        InvalidateRect(hwnd, null(), 0);
        return;
    }
    // Only painted content can receive clicks; the fixed title/footer cover these rows.
    if y < 40 || y >= content_bottom(state, &l) {
        return;
    }
    let y = y - 40 + state.page_scroll;
    if state.active_tab == 1
        && x >= 16
        && x < l.width - 16
        && y >= l.detail_top
        && y < l.detail_top + 52
    {
        let route =
            ((x - 16) / ((l.width - 32) / route_titles(state).len().max(1) as i32).max(1)) as usize;
        if route < route_titles(state).len() {
            state.build_route = route;
            scroll(hwnd, state, 0);
            return;
        }
    }
    if augment_ui::click(hwnd, state, &l, x, y) {
        return;
    }
    if let Some(detail) = &state.reply.detail {
        let start = l.hero_top + 180;
        if !state.side
            && x >= 16
            && x < l.width - 16
            && y >= start
            && y < start + detail.guides.len() as i32 * 32
        {
            let guide = &detail.guides[((y - start) / 32) as usize];
            if !state.offline && fuze_data::champion_detail::guide_url(&guide.url) {
                windows_sys::Win32::UI::Shell::ShellExecuteW(
                    hwnd,
                    wide("open").as_ptr(),
                    wide(&guide.url).as_ptr(),
                    null(),
                    null(),
                    SW_SHOWNORMAL,
                );
            }
            return;
        }
    }
    if y >= l.tabs_top && y < l.tabs_top + 34 && x >= 16 && x < l.width - 16 {
        let tab = ((x - 16) / ((l.width - 32) / 2).max(1)).min(1) as usize;
        state.active_tab = tab;
        state.hover = None;
        state.inspected = None;
        if tab != 0 {
            state.stage_panel = false;
        }
        state.stage_scroll = 0;
        state.stage_request(false);
        scroll(hwnd, state, 0);
        return;
    }
    if state.active_tab == 0
        && !state.side
        && y >= l.detail_top
        && y < l.detail_top + 30
        && x >= 16
        && x < l.width - 16
    {
        state.stage_panel = !state.stage_panel;
        state.stage_request(false);
        scroll(hwnd, state, 0);
        return;
    }
    if state.stage_panel
        && y >= l.detail_top + 38
        && y < l.detail_top + 70
        && x >= 16
        && x < l.width - 16
    {
        let tab = ((x - 16) / ((l.width - 32) / 6).max(1)).min(5);
        if tab == 5 {
            state.stage_request(true);
        } else {
            state.stage = tab as usize;
            state.stage_scroll = 0;
        }
        InvalidateRect(hwnd, null(), 0);
        return;
    }
    if let Some(id) = state
        .view
        .as_ref()
        .filter(|_| !state.side)
        .and_then(|v| l.hit(x, y, v))
    {
        if state.selected != id {
            state.selected = id;
            state.scroll = 0;
            state.request();
        }
        InvalidateRect(hwnd, null(), 0);
    }
}
unsafe fn panel(dc: HDC, r: RECT, surface: u32, edge: u32) {
    fill(dc, r, edge);
    fill(
        dc,
        RECT {
            left: r.left + 1,
            top: r.top + 1,
            right: r.right - 1,
            bottom: r.bottom - 1,
        },
        surface,
    );
}
/// Direct callbacks on the offline fixture window, never OS input or game actions.
pub(super) unsafe fn review_interactions(
    hwnd: HWND,
    state: &mut State,
    directory: &std::path::Path,
) -> Result<()> {
    if !state.offline {
        return Err("fixture-only");
    }
    let l = layout(hwnd, state);
    let original_route = state.build_route;
    let route_count = route_titles(state).len();
    if !(1..=4).contains(&route_count) {
        return Err("fixture-routes-missing");
    }
    state.page_scroll = l.detail_top;
    for route in 0..route_count {
        let x = 16 + route as i32 * ((l.width - 32) / route_count as i32) + 8;
        click(hwnd, state, x * l.dpi / 96, 48 * l.dpi / 96);
        if state.build_route != route
            || lines(state).first().map(|v| v.text.as_str())
                != route_titles(state).get(route).copied()
        {
            return Err("fixture-route-content");
        }
    }
    // Fixed title/footer must not hit content that happens to scroll underneath them.
    let selected = state.selected;
    state.page_scroll = 100;
    click(hwnd, state, 25 * l.dpi / 96, 20 * l.dpi / 96);
    click(hwnd, state, 25 * l.dpi / 96, (l.height - 10) * l.dpi / 96);
    if state.selected != selected {
        return Err("fixture-covered-content");
    }
    state.active_tab = 0;
    state.rarity = 0;
    state.page_scroll = body_top(state, &l);
    state.inspected = augment_ui::cards(state).first().map(|a| a.id);
    let pinned = state.inspected;
    if pinned.is_none() {
        return Err("fixture-augment-missing");
    }
    // This point is inside every nonempty tooltip but above the fixed footer.
    click(
        hwnd,
        state,
        30 * l.dpi / 96,
        (content_bottom(state, &l) - 20) * l.dpi / 96,
    );
    if state.inspected != pinned || state.active_tab != 0 || state.selected != selected {
        return Err("fixture-description-click-through");
    }
    if !augment_ui::scroll_description(hwnd, state, &l, 30, content_bottom(state, &l) - 20, -120) {
        return Err("fixture-description-scroll");
    }
    // Native footer and painted content share one viewport, including the side panel.
    scroll(hwnd, state, i32::MAX);
    if state.page_scroll != (total(state, &l) - (content_bottom(state, &l) - 40)).max(0) {
        return Err("fixture-scroll-end");
    }
    state.build_route = original_route;
    state.inspected = None;
    state.hover = None;
    std::fs::write(directory.join("interaction-checks.json"), serde_json::to_vec(&serde_json::json!({
        "game_access":false,"input_simulated":false,"routes_checked":route_count,
        "description_click_isolated":true,"description_scroll":true,"fixed_regions_isolated":true,"scroll_end_reachable":true
    })).map_err(|_| "fixture-report")?).map_err(|_| "fixture-report")?;
    Ok(())
}
pub(super) unsafe fn header(hwnd: HWND, dc: HDC, state: &State) {
    let l = layout(hwnd, state);
    let x = |v| v * l.dpi / 96;
    let r = |a, b, c, d| RECT {
        left: x(a),
        top: x(b),
        right: x(c),
        bottom: x(d),
    };
    fill(dc, r(0, 0, l.width, 40), rgb(23, 42, 59));
    text(dc, "FUZE", r(14, 4, 62, 36), x(11), rgb(131, 223, 202));
    text(
        dc,
        if state.side {
            "海克斯推荐"
        } else {
            "英雄详情"
        },
        r(77, 4, l.width - 45, 36),
        x(14),
        rgb(215, 232, 247),
    );
    let pen = CreatePen(PS_SOLID, 1, rgb(160, 179, 202));
    let old = SelectObject(dc, pen);
    MoveToEx(dc, x(l.width - 28), x(15), null_mut());
    LineTo(dc, x(l.width - 18), x(25));
    MoveToEx(dc, x(l.width - 18), x(15), null_mut());
    LineTo(dc, x(l.width - 28), x(25));
    SelectObject(dc, old);
    DeleteObject(pen);
}
pub(super) unsafe fn draw(hwnd: HWND, dc: HDC, state: &State) {
    let l = layout(hwnd, state);
    let x = |v| v * l.dpi / 96;
    let r = |a, b, c, d| RECT {
        left: x(a),
        top: x(b),
        right: x(c),
        bottom: x(d),
    };
    let pale = rgb(230, 238, 248);
    let gold = rgb(200, 170, 110);
    let dim = rgb(160, 179, 202);
    let cyan = rgb(125, 223, 205);
    fill(dc, r(0, 0, l.width, l.height), rgb(17, 30, 46));
    let Some(v) = &state.view else {
        header(hwnd, dc, state);
        return;
    };
    let saved = SaveDC(dc);
    IntersectClipRect(dc, 0, x(40), x(l.width), x(content_bottom(state, &l)));
    SetViewportOrgEx(dc, 0, x(40 - state.page_scroll), null_mut());
    let label = |s: &str, a, b, c, d, size, color| text(dc, s, r(a, b, c, d), x(size), color);
    if !state.side {
        label("我方阵容", 16, 9, l.width - 16, 35, 14, gold);
        panel(
            dc,
            r(12, 38, l.width - 12, 148),
            rgb(11, 32, 48),
            rgb(53, 86, 106),
        );
        for (i, h) in v.team.iter().take(5).enumerate() {
            let c = l.card(i, true);
            let center = (c.left + c.right) / 2;
            if h.id == state.selected {
                fill(dc, r(c.left, c.bottom - 2, c.right, c.bottom), gold);
            }
            panel(
                dc,
                r(center - 20, c.top + 7, center + 20, c.top + 47),
                rgb(7, 18, 28),
                if v.local_slot == Some(i) {
                    cyan
                } else {
                    rgb(84, 123, 145)
                },
            );
            icon(dc, state, &h.icon, x(center - 18), x(c.top + 9), x(36));
            label(
                &h.name,
                c.left + 3,
                c.top + 50,
                c.right - 2,
                c.top + 74,
                12,
                pale,
            );
            label(
                &h.win
                    .map(|v| format!("{:.1}%", v * 100.))
                    .unwrap_or("—".into()),
                c.left + 3,
                c.top + 77,
                c.right - 2,
                c.top + 98,
                11,
                dim,
            );
            if v.local_slot == Some(i) {
                label(
                    "你",
                    center + 8,
                    c.top + 26,
                    center + 20,
                    c.top + 46,
                    10,
                    cyan,
                );
            }
        }
        label(
            "可选英雄 · 按胜率排序",
            16,
            156,
            l.width - 16,
            183,
            14,
            gold,
        );
        if v.bench.is_empty() {
            label(
                "暂无备选英雄，客户端变化后自动刷新",
                16,
                l.bench_top,
                l.width - 16,
                l.bench_top + 52,
                12,
                dim,
            );
        }
        for (i, h) in v.bench.iter().enumerate() {
            let c = l.card(i, false);
            let selected = h.id == state.selected;
            panel(
                dc,
                r(c.left, c.top, c.right, c.bottom),
                if i == 0 {
                    rgb(43, 45, 35)
                } else {
                    rgb(25, 36, 42)
                },
                if selected {
                    cyan
                } else if i == 0 {
                    gold
                } else {
                    rgb(98, 88, 62)
                },
            );
            icon(dc, state, &h.icon, x(c.left + 6), x(c.top + 6), x(24));
            label(
                &h.name,
                c.left + 34,
                c.top + 4,
                c.right - 4,
                c.top + 31,
                11,
                pale,
            );
            label(
                &rate(h.win, "胜率"),
                c.left + 6,
                c.top + 33,
                c.right - 4,
                c.top + 57,
                16,
                gold,
            );
            label(
                &rate(h.pick, "选取率"),
                c.left + 6,
                c.top + 60,
                c.right - 4,
                c.top + 77,
                10,
                dim,
            );
        }
        let hero = v
            .team
            .iter()
            .chain(&v.bench)
            .find(|h| h.id == state.selected);
        panel(
            dc,
            r(16, l.hero_top, l.width - 16, l.hero_top + 86),
            rgb(18, 35, 55),
            rgb(53, 74, 96),
        );
        if let Some(hero) = hero {
            icon(dc, state, &hero.icon, x(30), x(l.hero_top + 16), x(54));
            label(
                &hero.name,
                98,
                l.hero_top + 12,
                l.width - 26,
                l.hero_top + 46,
                24,
                cyan,
            );
            label(
                &hero
                    .tier
                    .map(|tier| format!("{} {tier}", crate::i18n::text("梯队")))
                    .unwrap_or_else(|| crate::i18n::text("仅供参考，请在游戏中选择").into()),
                98,
                l.hero_top + 48,
                l.width - 26,
                l.hero_top + 73,
                11,
                dim,
            );
        }
        let cell = (l.width - 32) / 3;
        for (i, (title, value)) in [
            (
                "胜率",
                hero.and_then(|h| h.win)
                    .map(|v| format!("{:.1}%", v * 100.)),
            ),
            (
                "选取率",
                hero.and_then(|h| h.pick)
                    .map(|v| format!("{:.1}%", v * 100.)),
            ),
            ("场次", hero.and_then(|h| h.games).map(|v| v.to_string())),
        ]
        .iter()
        .enumerate()
        {
            let left = 16 + i as i32 * cell;
            panel(
                dc,
                r(left, l.hero_top + 94, left + cell - 6, l.hero_top + 140),
                rgb(25, 45, 64),
                rgb(53, 74, 96),
            );
            label(
                title,
                left + 10,
                l.hero_top + 97,
                left + cell - 10,
                l.hero_top + 113,
                10,
                dim,
            );
            label(
                value.as_deref().unwrap_or("—"),
                left + 10,
                l.hero_top + 114,
                left + cell - 10,
                l.hero_top + 139,
                18,
                pale,
            );
        }
        if let Some(detail) = &state.reply.detail {
            if !detail.guides.is_empty() {
                label(
                    "相关攻略",
                    16,
                    l.hero_top + 150,
                    l.width - 16,
                    l.hero_top + 177,
                    13,
                    gold,
                );
                for (i, guide) in detail.guides.iter().enumerate() {
                    let top = l.hero_top + 180 + i as i32 * 32;
                    label(&guide.title, 24, top, l.width - 24, top + 30, 12, cyan);
                }
            }
        }
    }
    let cell = (l.width - 32) / 2;
    for (i, name) in ["海克斯", "出装与技能"].iter().enumerate() {
        let left = 16 + i as i32 * cell;
        let selected = state.active_tab == i;
        panel(
            dc,
            r(left, l.tabs_top, left + cell - 4, l.tabs_top + 34),
            if selected {
                rgb(40, 73, 82)
            } else {
                rgb(23, 42, 59)
            },
            if selected { cyan } else { rgb(49, 67, 87) },
        );
        label(
            name,
            left + 7,
            l.tabs_top + 2,
            left + cell - 5,
            l.tabs_top + 32,
            12,
            if selected { cyan } else { dim },
        );
    }
    if state.active_tab == 0 && !state.side {
        panel(
            dc,
            r(16, l.detail_top, l.width - 16, l.detail_top + 30),
            rgb(23, 42, 59),
            rgb(53, 74, 96),
        );
        label(
            if state.stage_panel {
                "收起阶段胜率参考"
            } else {
                "展开阶段胜率参考"
            },
            24,
            l.detail_top,
            l.width - 24,
            l.detail_top + 30,
            12,
            cyan,
        );
    }
    if state.stage_panel {
        draw_stage(
            dc,
            state,
            r(16, l.detail_top + 38, l.width - 16, l.detail_top + 488),
            l.dpi,
        );
    }
    if state.active_tab == 1 {
        let titles = route_titles(state);
        let cell = (l.width - 32) / titles.len().max(1) as i32;
        for (i, title) in titles.iter().enumerate() {
            let top = l.detail_top;
            let left = 16 + i as i32 * cell;
            panel(
                dc,
                r(left, top, left + cell - 6, top + 52),
                if state.build_route == i {
                    rgb(40, 73, 82)
                } else {
                    rgb(23, 42, 59)
                },
                rgb(53, 74, 96),
            );
            label(
                title.split('·').next().unwrap_or(title).trim(),
                left + 8,
                top,
                left + cell - 8,
                top + 32,
                12,
                if state.build_route == i { cyan } else { dim },
            );
            label(
                if state.build_route == i {
                    "当前路线"
                } else {
                    "查看路线"
                },
                left + 8,
                top + 28,
                left + cell - 8,
                top + 48,
                10,
                dim,
            );
        }
    }
    if state.reply.detail.is_none() {
        label(
            if state.reply.error.is_some() {
                "本地详情不可用；可暂停后更新数据"
            } else {
                "正在读取本地详情…"
            },
            16,
            body_top(state, &l),
            l.width - 16,
            body_top(state, &l) + 46,
            13,
            dim,
        );
    } else if state.active_tab == 0 {
        augment_ui::draw(dc, state, &l);
    } else {
        let mut top = body_top(state, &l);
        for row in lines(state) {
            let height = line_height(row, &l);
            if top + height < state.page_scroll - 40 {
                top += height;
                continue;
            }
            if top - state.page_scroll > l.height {
                break;
            }
            if let Some(keys) = skill_keys(row) {
                panel(
                    dc,
                    r(16, top, l.width - 16, top + height - 6),
                    rgb(26, 43, 62),
                    rgb(43, 64, 87),
                );
                label("1–18 级加点", 24, top + 4, l.width - 24, top + 28, 13, gold);
                let cell = (l.width - 48) / 9;
                for (i, key) in keys.iter().enumerate() {
                    let left = 24 + (i as i32 % 9) * cell;
                    let y = top + 32 + (i as i32 / 9) * 52;
                    label(&(i + 1).to_string(), left, y, left + cell, y + 18, 10, dim);
                    panel(
                        dc,
                        r(left, y + 18, left + cell - 3, y + 48),
                        if *key == "R" {
                            rgb(68, 55, 34)
                        } else {
                            rgb(22, 50, 61)
                        },
                        if *key == "R" { gold } else { rgb(56, 83, 94) },
                    );
                    label(
                        key,
                        left + 6,
                        y + 18,
                        left + cell - 3,
                        y + 48,
                        15,
                        if *key == "R" { gold } else { cyan },
                    );
                }
                top += height;
                continue;
            }
            let art = !row.icons.is_empty();
            if art {
                panel(
                    dc,
                    r(16, top, l.width - 16, top + height - 6),
                    rgb(26, 43, 62),
                    rgb(43, 64, 87),
                );
            }
            let mut text_top = top + 6;
            let inline = row.icons.len() == 1;
            if inline {
                icon(dc, state, &row.icons[0], x(24), x(top + 10), x(40));
            } else if art {
                for (i, url) in row.icons.iter().take(8).enumerate() {
                    icon(dc, state, url, x(26 + i as i32 * 38), x(top + 8), x(30));
                }
                text_top += 36;
            }
            let chars = ((l.width - if inline { 100 } else { 48 })
                / if crate::i18n::locale() == "en-US" {
                    7
                } else {
                    14
                })
            .max(10) as usize;
            for chunk in wrap(&row.text, chars) {
                label(
                    &chunk,
                    if inline { 76 } else { 24 },
                    text_top,
                    l.width - 24,
                    text_top + 22,
                    13,
                    if art { pale } else { dim },
                );
                text_top += 22;
            }
            top += height;
        }
    }
    RestoreDC(dc, saved);
    header(hwnd, dc, state);
    augment_ui::tooltip(dc, state, &l);
    if !state.side {
        fill(
            dc,
            r(0, l.height - 64, l.width, l.height - 22),
            rgb(12, 27, 40),
        );
        let available = state.can_install();
        panel(
            dc,
            r(16, l.height - 60, l.width - 16, l.height - 26),
            if available {
                rgb(37, 65, 72)
            } else {
                rgb(23, 42, 59)
            },
            if available { gold } else { rgb(53, 74, 96) },
        );
        let caption = if state.install.is_some() {
            "正在配置装备方案…"
        } else if state.install_status == Some((state.selected, true)) {
            "装备方案已配置"
        } else if state.install_status == Some((state.selected, false)) {
            "配置未完成，点击重试"
        } else if available {
            "配置当前英雄装备方案"
        } else {
            "选中自己的英雄后配置装备方案"
        };
        text(
            dc,
            caption,
            r(26, l.height - 60, l.width - 26, l.height - 26),
            x(12),
            if available { pale } else { dim },
        );
    }
    fill(dc, r(0, l.height - 22, l.width, l.height), rgb(23, 42, 59));
    text(
        dc,
        "滚轮浏览详情 · 推荐使用整体胜率",
        r(14, l.height - 22, l.width - 14, l.height),
        x(11),
        dim,
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vertical_layout_has_five_team_slots_and_gap_hits_are_ignored() {
        let l = Layout::new(444, 780, 96, 20);
        let hero = |id| Hero {
            id,
            name: String::new(),
            icon: String::new(),
            win: None,
            pick: None,
            games: None,
            tier: None,
        };
        let v = View {
            team: (1..=5).map(hero).collect(),
            bench: (6..=25).map(hero).collect(),
            choices: None,
            local_slot: Some(0),
            version: "zh-CN@1".into(),
        };
        assert_eq!(l.columns, 4);
        assert!(l.hero_top > l.card(19, false).bottom);
        assert_eq!(l.hit(20, 50, &v), Some(1));
        assert_eq!(l.hit(l.card(0, true).right + 1, 50, &v), None);
        assert_eq!(l.hit(20, l.bench_top + 5, &v), Some(6));
        assert!(l.tabs_top > l.hero_top + 140);
    }
}
