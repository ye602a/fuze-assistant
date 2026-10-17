//! Native rendering of the established Chinese FUZE workbench (Display.vue).
//! Geometry is shared with native controls; scrolling never starts background work.
use super::*;
use windows_sys::Win32::UI::Controls::SetScrollInfo;
pub(super) const HOME: usize = 110;
const GOLD: u32 = 0x6eaac8;
const IVORY: u32 = 0xd2e6f0;
const MUTED: u32 = 0xb4ada1;
const BASE: u32 = 0x1a1006;
const PANEL: u32 = 0x2a1e10;
const EDGE: u32 = 0x484b43;
pub(super) unsafe fn dpi(hwnd: HWND) -> i32 {
    GetDpiForWindow(hwnd).max(96) as i32
}
pub(super) unsafe fn rail(hwnd: HWND) -> i32 {
    184 * dpi(hwnd) / 96
}
fn height(u: &Ui) -> i32 {
    if u.settings {
        1680 + storage_extra(u)
    } else if u.records && u.history && !u.journal {
        history_ui::height(u).max(620)
    } else if u.records {
        620
    } else {
        1010 + if u.level_details { 80 } else { 0 } + if u.data_details { 154 } else { 0 }
    }
}
fn storage_extra(u: &Ui) -> i32 {
    u.maintenance
        .storage
        .as_ref()
        .map(|p| (p.categories.len() + p.versions.len()) as i32 * 26 + 28)
        .unwrap_or(0)
}
pub(super) fn settings_y(y: i32, u: &Ui) -> i32 {
    if !u.settings {
        y
    } else if y >= 1442 {
        y - 1176
    } else if y >= 266 {
        y + 232 + storage_extra(u)
    } else {
        y
    }
}
pub(super) unsafe fn bounds(hwnd: HWND, u: &Ui) -> RECT {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    r.right -= rail(hwnd);
    r.bottom = r.bottom.max(height(u) * dpi(hwnd) / 96);
    r
}
pub(super) unsafe fn scroll(hwnd: HWND, u: &mut Ui, delta: i32) {
    u.scroll = u.scroll.saturating_add(delta);
    SendMessageW(hwnd, WM_SIZE, 0, 0);
}
pub(super) unsafe fn configure_scroll(hwnd: HWND, u: &mut Ui) {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let total = height(u) * dpi(hwnd) / 96;
    u.scroll = u.scroll.clamp(0, (total - r.bottom).max(0));
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: total - 1,
        nPage: r.bottom.max(1) as u32,
        nPos: u.scroll,
        nTrackPos: 0,
    };
    SetScrollInfo(hwnd, SB_VERT, &info, 1);
}
pub(super) unsafe fn place(hwnd: HWND, child: HWND, x: i32, y: i32, w: i32, h: i32, redraw: i32) {
    let u = &*(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Ui);
    let d = dpi(hwnd);
    let y = settings_y(y * 96 / d, u) * d / 96;
    MoveWindow(
        child,
        rail(hwnd) + x,
        y - u.scroll,
        w.max(1),
        h.max(1),
        redraw,
    );
}
unsafe fn box_rect(dc: HDC, r: RECT, surface: u32, edge: u32) {
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
// Same path vocabulary as RuneGlyph.vue, with GDI lines rather than text symbols.
unsafe fn glyph(dc: HDC, kind: usize, left: i32, top: i32, size: i32, color: u32) {
    let pen = CreatePen(PS_SOLID, (size / 20).max(1), color);
    let old = SelectObject(dc, pen);
    let paths: &[&[(i32, i32)]] = match kind {
        0 => &[
            &[
                (16, 3),
                (28, 10),
                (28, 22),
                (16, 29),
                (4, 22),
                (4, 10),
                (16, 3),
            ],
            &[(16, 9), (23, 16), (16, 23), (9, 16), (16, 9)],
        ],
        1 => &[
            &[
                (8, 5),
                (25, 5),
                (25, 28),
                (8, 28),
                (4, 24),
                (4, 9),
                (8, 5),
                (8, 28),
            ],
            &[(12, 11), (21, 11)],
            &[(12, 16), (21, 16)],
            &[(12, 21), (17, 21)],
        ],
        2 => &[
            &[(16, 3), (16, 10)],
            &[(16, 22), (16, 29)],
            &[(3, 16), (10, 16)],
            &[(22, 16), (29, 16)],
            &[(7, 7), (12, 12)],
            &[(20, 20), (25, 25)],
            &[(7, 25), (12, 20)],
            &[(20, 12), (25, 7)],
            &[(16, 9), (23, 16), (16, 23), (9, 16), (16, 9)],
        ],
        3 => &[
            &[(8, 5), (16, 13), (11, 18), (3, 10), (8, 5)],
            &[(21, 14), (29, 22), (24, 27), (16, 19), (21, 14)],
            &[(11, 21), (21, 11)],
        ],
        4 => &[
            &[(12, 5), (5, 5), (5, 12)],
            &[(20, 5), (27, 5), (27, 12)],
            &[(5, 20), (5, 27), (12, 27)],
            &[(27, 20), (27, 27), (20, 27)],
            &[(3, 16), (29, 16)],
            &[(16, 10), (22, 16), (16, 22), (10, 16), (16, 10)],
        ],
        5 => &[
            &[(8, 5), (24, 5)],
            &[(16, 3), (16, 11)],
            &[(7, 11), (25, 11), (21, 19), (16, 23), (11, 19), (7, 11)],
            &[(9, 29), (16, 23), (23, 29)],
        ],
        6 => &[
            &[(16, 3), (28, 9), (16, 15), (4, 9), (16, 3)],
            &[(4, 15), (16, 21), (28, 15)],
            &[(4, 21), (16, 27), (28, 21)],
        ],
        _ => &[
            &[
                (16, 2),
                (26, 11),
                (26, 21),
                (16, 30),
                (6, 21),
                (6, 11),
                (16, 2),
            ],
            &[(16, 7), (16, 25)],
            &[(10, 12), (16, 16), (22, 12)],
            &[(10, 20), (16, 16), (22, 20)],
        ],
    };
    for path in paths {
        if let Some((x, y)) = path.first() {
            MoveToEx(dc, left + x * size / 32, top + y * size / 32, null_mut());
            for (x, y) in &path[1..] {
                LineTo(dc, left + x * size / 32, top + y * size / 32);
            }
        }
    }
    SelectObject(dc, old);
    DeleteObject(pen);
}
pub(super) unsafe fn shell(hwnd: HWND, dc: HDC, u: &Ui) {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let d = dpi(hwnd);
    let x = |v| v * d / 96;
    fill(dc, r, BASE);
    fill(
        dc,
        RECT {
            right: rail(hwnd),
            ..r
        },
        rgb(6, 16, 26),
    );
    fill(
        dc,
        RECT {
            left: rail(hwnd) - 1,
            right: rail(hwnd),
            ..r
        },
        rgb(93, 81, 57),
    );
    DrawIconEx(
        dc,
        x(22),
        x(28),
        u.icon,
        x(43),
        x(48),
        0,
        null_mut(),
        DI_NORMAL,
    );
    text(
        dc,
        "符择",
        RECT {
            left: x(77),
            top: x(25),
            right: x(174),
            bottom: x(62),
        },
        x(26),
        GOLD,
    );
    text(
        dc,
        "F U Z E",
        RECT {
            left: x(77),
            top: x(63),
            right: x(174),
            bottom: x(84),
        },
        x(11),
        GOLD,
    );
    text(
        dc,
        "独立测试版",
        RECT {
            left: x(22),
            top: r.bottom - x(74),
            right: x(174),
            bottom: r.bottom - x(50),
        },
        x(12),
        MUTED,
    );
    text(
        dc,
        "设置与稳定版分开保存",
        RECT {
            left: x(22),
            top: r.bottom - x(49),
            right: x(174),
            bottom: r.bottom - x(27),
        },
        x(10),
        MUTED,
    );
}
pub(super) unsafe fn navigation(hwnd: HWND, u: &Ui) {
    let d = dpi(hwnd);
    let x = |v| v * d / 96;
    for (i, id) in [HOME, RECORDS, SETTINGS].iter().enumerate() {
        MoveWindow(
            GetDlgItem(hwnd, *id as i32),
            x(12),
            x(115 + i as i32 * 66),
            x(160),
            x(54),
            1,
        );
    }
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    for (i, id) in [HIDE, QUIT].iter().enumerate() {
        MoveWindow(
            GetDlgItem(hwnd, *id as i32),
            x(18),
            r.bottom - x(170 - i as i32 * 42),
            x(148),
            x(34),
            1,
        );
    }
    let content = bounds(hwnd, u);
    let right = content.right;
    place(
        hwnd,
        GetDlgItem(hwnd, TOGGLE as i32),
        right - x(176),
        x(202),
        x(124),
        x(40),
        1,
    );
    if u.settings || u.records {
        place(
            hwnd,
            GetDlgItem(hwnd, TOGGLE as i32),
            right - x(176),
            x(62),
            x(124),
            x(38),
            1,
        );
    }
    ShowWindow(GetDlgItem(hwnd, TOGGLE as i32), SW_SHOWNA);
    for (i, id) in [UPDATE_DATA, UPDATE_APP].iter().enumerate() {
        place(
            hwnd,
            GetDlgItem(hwnd, *id as i32),
            x(48 + i as i32 * 180),
            x(182),
            x(164),
            x(38),
            1,
        );
        ShowWindow(
            GetDlgItem(hwnd, *id as i32),
            if u.settings { SW_SHOWNA } else { SW_HIDE },
        );
    }
    if u.settings {
        for (i, id) in [201, 202, 203, 204, 205, 206, 207, 208, 223, 224, 225]
            .iter()
            .enumerate()
        {
            place(
                hwnd,
                GetDlgItem(hwnd, *id),
                right - x(284),
                x(288 + i as i32 * 56),
                x(236),
                x(36),
                1,
            );
        }
        place(
            hwnd,
            GetDlgItem(hwnd, IMPORT_SETTINGS as i32),
            x(48),
            x(1080),
            x(190),
            x(38),
            1,
        );
        place(
            hwnd,
            GetDlgItem(hwnd, SAVE_SETTINGS as i32),
            right - x(212),
            x(1080),
            x(164),
            x(38),
            1,
        );
        place(
            hwnd,
            GetDlgItem(hwnd, 210),
            x(48),
            x(992),
            right - x(96),
            x(32),
            1,
        );
    }
}
pub(super) unsafe fn button(hwnd: HWND, u: &Ui, item: &DRAWITEMSTRUCT) {
    let id = item.CtlID as usize;
    let selected = match id {
        HOME => !u.records && !u.settings,
        RECORDS => u.records,
        SETTINGS => u.settings,
        _ => false,
    };
    let disabled = item.itemState & ODS_DISABLED != 0;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let primary = [TOGGLE, SAVE_SETTINGS].contains(&id);
    let surface = if disabled {
        rgb(25, 35, 43)
    } else if pressed {
        rgb(65, 63, 48)
    } else if primary {
        GOLD
    } else if selected {
        rgb(31, 46, 49)
    } else {
        rgb(12, 26, 37)
    };
    let navigation = [HOME, RECORDS, SETTINGS].contains(&id);
    box_rect(
        item.hDC,
        item.rcItem,
        if navigation && !selected && !pressed {
            BASE
        } else {
            surface
        },
        if selected || primary {
            GOLD
        } else if navigation {
            BASE
        } else {
            EDGE
        },
    );
    let color = if disabled {
        rgb(143, 157, 166)
    } else if primary && !pressed {
        BASE
    } else if selected {
        IVORY
    } else {
        rgb(204, 214, 218)
    };
    let mut caption = [0u16; 256];
    let n = GetWindowTextW(item.hwndItem, caption.as_mut_ptr(), 256);
    let mut r = item.rcItem;
    let d = dpi(hwnd);
    r.left += 12 * d / 96;
    r.right -= 10 * d / 96;
    if let Some(kind) = [HOME, RECORDS, SETTINGS].iter().position(|v| *v == id) {
        glyph(
            item.hDC,
            kind,
            r.left,
            r.top + 14 * d / 96,
            25 * d / 96,
            if selected { GOLD } else { MUTED },
        );
        r.left += 38 * d / 96;
    }
    text(
        item.hDC,
        &String::from_utf16_lossy(&caption[..n.max(0) as usize]),
        r,
        14 * d / 96,
        color,
    );
    if item.itemState & ODS_FOCUS != 0 {
        let mut focus = item.rcItem;
        InflateRect(&mut focus, -3, -3);
        DrawFocusRect(item.hDC, &focus);
    }
}
pub(super) unsafe fn page(hwnd: HWND, dc: HDC, u: &Ui) {
    let r = bounds(hwnd, u);
    let d = dpi(hwnd);
    let x = |v| v * d / 96;
    let rect = |l, t, rr, b| RECT {
        left: x(l),
        top: x(settings_y(t, u)),
        right: rr,
        bottom: x(settings_y(b, u)),
    };
    let width = r.right;
    let right = width - x(28);
    text(
        dc,
        if u.settings {
            "偏好设置"
        } else if u.records {
            "战绩查询"
        } else {
            "对局助手"
        },
        rect(28, 10, right, 36),
        x(12),
        MUTED,
    );
    fill(dc, rect(0, 43, width, 44), rgb(41, 56, 65));
    if u.records {
        return;
    }
    text(
        dc,
        if u.settings {
            "偏好设置"
        } else if u.records {
            "战绩与个人记录"
        } else {
            "你的对局工作台"
        },
        rect(28, 62, right, 105),
        x(28),
        IVORY,
    );
    text(
        dc,
        if u.settings {
            "开关即时保存，目录单独保存；仅识别相关设置会重建识别链路。"
        } else {
            "连接、识别与推荐，进度一目了然。"
        },
        rect(28, 108, right, 136),
        x(13),
        MUTED,
    );
    if u.settings {
        box_rect(dc, rect(28, 154, right, 242), PANEL, EDGE);
        if let Some(e) = u.shown.update_error {
            text(
                dc,
                explain(Some(e)),
                rect(48, 222, right - x(20), 242),
                x(11),
                rgb(247, 167, 158),
            );
        }
        text(
            dc,
            "数据与软件更新",
            rect(48, 160, right - x(20), 182),
            x(14),
            GOLD,
        );
        box_rect(dc, rect(28, 266, right, 914), PANEL, EDGE);
        let labels = [
            "识别计算方式",
            "识别与推荐数据语言",
            "显示选人参考",
            "选人窗口置顶",
            "关闭主界面时",
            "后台下载公共图标",
            "软件更新频道",
            "暂停快捷键 Ctrl+Shift+F8",
            "等级辅助调度",
            "显示三卡推荐",
            "自动显示局内侧栏",
        ];
        for (i, label) in labels.iter().enumerate() {
            let top = 282 + i as i32 * 56;
            text(
                dc,
                label,
                rect(48, top, right - x(278), top + 48),
                x(14),
                IVORY,
            );
            if i < 10 {
                fill(dc, rect(48, top + 55, right - x(20), top + 56), EDGE);
            }
        }
        box_rect(dc, rect(28, 934, right, 1046), PANEL, EDGE);
        box_rect(dc, rect(47, 991, right - x(19), 1025), BASE, EDGE);
        text(
            dc,
            "连接与诊断",
            rect(48, 942, right - x(20), 967),
            x(16),
            GOLD,
        );
        text(
            dc,
            "自动发现客户端；下方目录仅在自动发现失败后使用，可留空。",
            rect(48, 966, right - x(20), 991),
            x(12),
            MUTED,
        );
        text(
            dc,
            u.shown
                .notice
                .unwrap_or("保存后自动应用；自动发现始终优先。"),
            rect(48, 1028, right - x(20), 1049),
            x(12),
            MUTED,
        );
        box_rect(dc, rect(28, 1136, right, 1424), PANEL, EDGE);
        text(
            dc,
            "浮窗与领取信号诊断",
            rect(48, 1142, right - x(20), 1167),
            x(16),
            GOLD,
        );
        text(
            dc,
            "实际领取轮次未确认，继续使用整体胜率。预览内容不是本局识别结果。",
            rect(48, 1212, right - x(20), 1242),
            x(12),
            MUTED,
        );
        claim_ui::paint(dc, u, right, d);
        box_rect(
            dc,
            rect(28, 1442, right, 1650 + storage_extra(u)),
            PANEL,
            EDGE,
        );
        text(
            dc,
            "存储管理",
            rect(48, 1452, right - x(20), 1480),
            x(16),
            GOLD,
        );
        text(
            dc,
            "保留个人记录、设置、当前与回退版本；清理预览中的过期数据及下载事务。",
            rect(48, 1534, right - x(20), 1560),
            x(12),
            MUTED,
        );
        if let Some(p) = &u.maintenance.storage {
            for (i, (label, n, bytes)) in p
                .categories
                .iter()
                .map(|(l, n, b)| (crate::i18n::text(l).to_owned(), *n, *b))
                .chain(p.versions.iter().cloned())
                .enumerate()
            {
                text(
                    dc,
                    &format!(
                        "{label} · {n} {} · {:.1} MB",
                        crate::i18n::text("文件"),
                        bytes as f64 / 1048576.0
                    ),
                    rect(
                        48,
                        1648 + i as i32 * 26,
                        right - x(20),
                        1672 + i as i32 * 26,
                    ),
                    x(12),
                    MUTED,
                );
            }
            let line = format!(
                "{} {:.1} MB · {} {} · {} {:.1} MB / {}",
                crate::i18n::text("存储占用"),
                p.bytes as f64 / 1048576.0,
                crate::i18n::text("文件"),
                p.files,
                crate::i18n::text("可回收"),
                p.reclaimable as f64 / 1048576.0,
                p.removable
            );
            text(dc, &line, rect(48, 1564, right - x(20), 1594), x(14), IVORY);
            if p.protection_unknown {
                text(
                    dc,
                    "部分数据保护状态不明，已保留。",
                    rect(48, 1594, right - x(20), 1618),
                    x(12),
                    GOLD,
                );
            }
        }
        if let Some(message) = u.maintenance.message {
            text(
                dc,
                message,
                rect(48, 1620, right - x(20), 1646),
                x(12),
                MUTED,
            );
        }
        return;
    }
    let p = &u.shown.pipeline;
    let (title, color, accent) = if u.desired == EXIT {
        ("正在退出 · 释放识别资源", rgb(59, 51, 32), GOLD)
    } else if u.desired == PAUSED && u.shown.mode != PAUSED {
        ("正在暂停 · 清理画面与 OCR", rgb(59, 51, 32), GOLD)
    } else if u.desired == DATA {
        ("正在更新推荐数据", rgb(18, 50, 75), rgb(105, 188, 230))
    } else if u.desired == PAUSED {
        ("已暂停 · 点击开始识别", rgb(59, 51, 32), GOLD)
    } else if p.error.is_some()
        || u.shown.update_error.is_some()
        || matches!(u.shown.connection, Some(Connection::Retry(_)))
    {
        ("需要关注 · 正在恢复", rgb(66, 33, 40), rgb(247, 167, 158))
    } else if p.capture == "stat-shards" {
        ("属性碎片界面 · 等待强化符文", rgb(59, 51, 32), GOLD)
    } else if p.frame_age.is_some_and(|v| v < 5) {
        ("识别运行中", rgb(18, 61, 52), rgb(121, 214, 174))
    } else if matches!(
        u.shown.connection,
        Some(Connection::Connected(
            fuze_runtime::session::Phase::ClientReady
        ))
    ) {
        ("已连接 · 等待对局", rgb(18, 50, 65), rgb(121, 214, 174))
    } else {
        (
            connection(u.shown.connection),
            rgb(18, 50, 65),
            rgb(105, 188, 230),
        )
    };
    box_rect(dc, rect(28, 154, right, 286), color, accent);
    text(
        dc,
        "实时状态",
        rect(48, 166, right - x(20), 192),
        x(12),
        GOLD,
    );
    text(dc, title, rect(48, 198, right - x(168), 236), x(23), IVORY);
    text(
        dc,
        u.shown
            .notice
            .unwrap_or_else(|| explain(p.error.or(u.shown.update_error))),
        rect(48, 245, right - x(20), 274),
        x(13),
        rgb(183, 211, 219),
    );
    let data = p.dataset.as_deref().or(u.shown.available.as_deref());
    let version = data
        .and_then(|s| s.split_once('@').map(|v| v.1))
        .or(data)
        .unwrap_or("未加载");
    let values = [
        (
            "客户端连接",
            if u.desired == PAUSED {
                "检测已暂停"
            } else {
                connection(u.shown.connection)
            },
        ),
        (
            "游戏画面",
            if u.desired == PAUSED {
                "未截图"
            } else if p
                .error
                .is_some_and(|e| e.starts_with("capture") || e.starts_with("game-window"))
            {
                "截图恢复中"
            } else if p.capture == "waiting" || p.capture.is_empty() {
                "等待对局画面"
            } else {
                capture(p)
            },
        ),
        (
            "中文识别",
            match p.backend {
                Some(fuze_contract::wire::ActiveBackend::Cpu) => "处理器识别已就绪",
                Some(fuze_contract::wire::ActiveBackend::DirectMl) => "显卡加速已就绪",
                None => "待初始化",
            },
        ),
        ("推荐数据", version),
    ];
    let tile = (width - x(56 + 36)) / 4;
    for (i, (name, value)) in values.iter().enumerate() {
        let l = x(28) + i as i32 * (tile + x(12));
        box_rect(
            dc,
            RECT {
                left: l,
                top: x(306),
                right: l + tile,
                bottom: x(438),
            },
            PANEL,
            EDGE,
        );
        glyph(dc, i + 3, l + x(16), x(321), x(25), GOLD);
        text(
            dc,
            name,
            RECT {
                left: l + x(16),
                top: x(354),
                right: l + tile - x(12),
                bottom: x(378),
            },
            x(12),
            MUTED,
        );
        text(
            dc,
            value,
            RECT {
                left: l + x(16),
                top: x(383),
                right: l + tile - x(12),
                bottom: x(411),
            },
            x(14),
            IVORY,
        );
        if i == 0 || i == 3 {
            let verification = format!(
                "{} · {}",
                crate::i18n::text("最近连接验证"),
                u.shown.verified_time.as_deref().unwrap_or("—")
            );
            text(
                dc,
                if i == 3 {
                    "数据版本"
                } else {
                    &verification
                },
                RECT {
                    left: l + x(16),
                    top: x(413),
                    right: l + tile - x(12),
                    bottom: x(431),
                },
                x(11),
                MUTED,
            );
        }
    }
    box_rect(dc, rect(28, 458, right, 686), PANEL, EDGE);
    text(
        dc,
        "本局最近识别",
        rect(48, 472, right - x(20), 501),
        x(17),
        GOLD,
    );
    text(
        dc,
        "仅展示识别到的候选，不代表实际领取；推荐继续使用整体胜率。",
        rect(48, 501, right - x(20), 527),
        x(12),
        MUTED,
    );
    let card = (width - x(96 + 24)) / 3;
    for i in 0..3 {
        let l = x(48) + i as i32 * (card + x(12));
        box_rect(
            dc,
            RECT {
                left: l,
                top: x(540),
                right: l + card,
                bottom: x(666),
            },
            BASE,
            EDGE,
        );
        let slot = p.recent.as_ref().map(|s| &s[i]);
        if let Some(tier) = p.recent_tiers[i] {
            text(
                dc,
                &format!("{} {}", crate::i18n::text("梯队"), tier),
                RECT {
                    left: l + x(64),
                    top: x(558),
                    right: l + card - x(12),
                    bottom: x(584),
                },
                x(13),
                GOLD,
            );
        }
        let (name, hint) = match slot {
            Some(fuze_contract::Slot::Known { name, .. }) => (name.as_str(), "最近识别的候选"),
            Some(fuze_contract::Slot::Unreadable) => ("此位置未识别", "可能被遮挡，等待下一次识别"),
            None => ("等待海克斯出现", "未进入强化选择时，空白是正常现象"),
        };
        glyph(
            dc,
            7,
            l + x(16),
            x(558),
            x(32),
            if slot.is_some() { GOLD } else { MUTED },
        );
        text(
            dc,
            name,
            RECT {
                left: l + x(16),
                top: x(601),
                right: l + card - x(12),
                bottom: x(628),
            },
            x(15),
            IVORY,
        );
        text(
            dc,
            hint,
            RECT {
                left: l + x(16),
                top: x(634),
                right: l + card - x(12),
                bottom: x(654),
            },
            x(11),
            MUTED,
        );
    }
    box_rect(dc, rect(28, 702, right, 868), PANEL, EDGE);
    text(
        dc,
        "运行开销",
        rect(48, 714, right - x(20), 742),
        x(17),
        GOLD,
    );
    let values = [
        ("CPU", u.shown.resources.cpu.map(|v| format!("{v:.1}%"))),
        (
            "内存",
            u.shown.resources.memory_mb.map(|v| format!("{v:.0} MB")),
        ),
        ("识别次数", Some(p.analyses.to_string())),
        ("最近识别耗时", p.analysis_ms.map(|v| format!("{v:.0} ms"))),
    ];
    let cell = (width - x(96)) / 4;
    for (i, (label, value)) in values.iter().enumerate() {
        let left = x(48) + i as i32 * cell;
        text(
            dc,
            label,
            RECT {
                left,
                top: x(756),
                right: left + cell - x(12),
                bottom: x(777),
            },
            x(12),
            MUTED,
        );
        text(
            dc,
            value.as_deref().unwrap_or("—"),
            RECT {
                left,
                top: x(780),
                right: left + cell - x(12),
                bottom: x(819),
            },
            x(23),
            IVORY,
        );
    }
    text(
        dc,
        &format!(
            "{} · {} · {} {}",
            crate::i18n::text("每 2 秒采样自身与 OCR 子进程"),
            u.shown.resources.sampled.as_deref().unwrap_or("—"),
            crate::i18n::text("调度跳过"),
            p.skipped_frames
        ),
        rect(48, 833, right - x(20), 855),
        x(12),
        MUTED,
    );
    let level_bottom = 920 + if u.level_details { 80 } else { 0 };
    box_rect(dc, rect(28, 884, right, level_bottom), PANEL, EDGE);
    text(
        dc,
        if u.level_details {
            "▾ 等级辅助调度"
        } else {
            "▸ 等级辅助调度"
        },
        rect(48, 888, right - x(20), 916),
        x(14),
        GOLD,
    );
    if u.level_details {
        let mut value = format!(
            "{} {} · {}",
            crate::i18n::text("当前等级"),
            p.level.map(|v| v.to_string()).unwrap_or("—".into()),
            crate::i18n::text(if u.shown.preferences.level_gate {
                "已开启"
            } else {
                "已关闭"
            })
        );
        if let Some((_, interval, next)) = p.schedule {
            value.push_str(&format!(
                " · {} {} · {} {interval} ms",
                crate::i18n::text("下次调度等级"),
                next.map(|v| v.to_string()).unwrap_or("—".into()),
                crate::i18n::text("检查间隔")
            ));
        }
        text(dc, &value, rect(48, 926, right - x(20), 951), x(14), IVORY);
        text(
            dc,
            "等级仅辅助安排识别；无法确认实际领取轮次。",
            rect(48, 956, right - x(20), 981),
            x(12),
            MUTED,
        );
    }
    let data_top = level_bottom + 16;
    let data_bottom = data_top + 36 + if u.data_details { 154 } else { 0 };
    box_rect(dc, rect(28, data_top, right, data_bottom), PANEL, EDGE);
    text(
        dc,
        if u.data_details {
            "▾ 数据来源与版本"
        } else {
            "▸ 数据来源与版本"
        },
        rect(48, data_top + 4, right - x(20), data_top + 32),
        x(14),
        GOLD,
    );
    if u.data_details {
        let r = &u.shown.recognition;
        for (i, value) in [
            format!(
                "{} / ARAMMayhem {} · {} · {}",
                crate::i18n::text("识别字典"),
                r.patch,
                r.count,
                r.date
            ),
            format!(
                "{} / ARAMGG {}",
                crate::i18n::text("推荐数据"),
                u.shown.available.as_deref().unwrap_or("—")
            ),
            format!(
                "{} {}",
                crate::i18n::text("腾讯版本参考（非本机实测）"),
                if r.reference.is_empty() {
                    "—"
                } else {
                    &r.reference
                }
            ),
            crate::i18n::text("多源补充名称；胜率采用单一来源，不混用其他补丁。").into(),
            "ARAMMayhem · CC BY 4.0 · arammayhem.com/zh-cn/data/".into(),
        ]
        .iter()
        .enumerate()
        {
            text(
                dc,
                value,
                rect(
                    48,
                    data_top + 42 + i as i32 * 26,
                    right - x(20),
                    data_top + 66 + i as i32 * 26,
                ),
                x(12),
                MUTED,
            );
        }
    }
    text(
        dc,
        if u.tray && u.shown.preferences.close_to_tray {
            "关闭窗口可收起到托盘；结束运行请使用左侧“退出”。"
        } else {
            "关闭窗口将退出；你也可以使用左侧“退出”。"
        },
        rect(28, data_bottom + 16, right, data_bottom + 44),
        x(12),
        MUTED,
    );
}

pub(super) unsafe fn click(hwnd: HWND, u: &mut Ui, px: i32, py: i32) {
    if u.records || u.settings {
        return;
    }
    let d = dpi(hwnd);
    let left = (px - rail(hwnd)) * 96 / d;
    let top = (py + u.scroll) * 96 / d;
    let width = bounds(hwnd, u).right * 96 / d;
    if left < 28 || left >= width - 28 {
        return;
    }
    if (884..920).contains(&top) {
        u.level_details = !u.level_details;
    } else {
        let data_top = 936 + if u.level_details { 80 } else { 0 };
        if !(data_top..data_top + 36).contains(&top) {
            return;
        }
        u.data_details = !u.data_details;
    }
    SendMessageW(hwnd, WM_SIZE, 0, 0);
    InvalidateRect(hwnd, null(), 0);
}

/// Bounded renders of this hidden fixture window only. No input, live discovery or capture.
pub(super) unsafe fn review(hwnd: HWND, u: &mut Ui, directory: &str) -> Result<()> {
    if !u.smoke {
        return Err("workbench-review-fixture-only");
    }
    let directory = std::path::Path::new(directory);
    std::fs::create_dir_all(directory).map_err(|_| "workbench-review-directory")?;
    let shown = u.shown.clone();
    let desired = u.desired;
    let mut original: RECT = zeroed();
    GetWindowRect(hwnd, &mut original);
    let render = |hwnd: HWND, u: &mut Ui, name: &str| {
        snapshot(hwnd, &directory.join(name).to_string_lossy(), u)
    };
    let result = (|| {
        u.shown.pipeline.recent = Some(Arc::new([
            fuze_contract::Slot::Known {
                id: 1,
                name: "珠光护手".into(),
                confidence: 0.99,
            },
            fuze_contract::Slot::Unreadable,
            fuze_contract::Slot::Known {
                id: 2,
                name: "万用瞄准镜".into(),
                confidence: 0.99,
            },
        ]));
        u.shown.pipeline.frame_age = Some(0);
        u.shown.pipeline.backend = Some(fuze_contract::wire::ActiveBackend::Cpu);
        u.shown.pipeline.capture = "receiving";
        u.shown.connection = Some(Connection::Connected(
            fuze_runtime::session::Phase::InProgress,
        ));
        render(hwnd, u, "partial.png")?;
        u.shown.pipeline.error = Some("capture-frame");
        render(hwnd, u, "error.png")?;
        u.shown.pipeline.error = None;
        u.desired = PAUSED;
        render(hwnd, u, "paused.png")?;
        u.desired = desired;
        u.shown = shown.clone();
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            1080,
            700,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        render(hwnd, u, "compact.png")?;
        // Run actual navigation handlers in the owned fixture; never simulate player input.
        SendMessageW(hwnd, WM_COMMAND, SETTINGS, 0);
        u.scroll = i32::MAX;
        SendMessageW(hwnd, WM_SIZE, 0, 0);
        render(hwnd, u, "settings-bottom.png")?;
        // Reveal a focused directory control through our own notification handler.
        SendMessageW(
            hwnd,
            WM_COMMAND,
            210 | ((EN_SETFOCUS as usize) << 16),
            GetDlgItem(hwnd, 210) as isize,
        );
        let mut edit: RECT = zeroed();
        GetWindowRect(GetDlgItem(hwnd, 210), &mut edit);
        let mut point = POINT {
            x: edit.left,
            y: edit.top,
        };
        ScreenToClient(hwnd, &mut point);
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        if u.scroll <= 0 || point.y < 0 || point.y + edit.bottom - edit.top > client.bottom {
            return Err("workbench-scroll-layout");
        }
        render(hwnd, u, "settings-directory.png")?;
        SendMessageW(hwnd, WM_COMMAND, RECORDS, 0);
        if !u.records || u.settings || u.scroll != 0 {
            return Err("workbench-history-navigation");
        }
        SendMessageW(hwnd, WM_COMMAND, HOME, 0);
        if u.records || u.settings || u.scroll != 0 {
            return Err("workbench-home-navigation");
        }
        u.level_details = true;
        u.data_details = true;
        u.scroll = i32::MAX;
        SendMessageW(hwnd, WM_SIZE, 0, 0);
        render(hwnd, u, "main-details.png")?;
        u.level_details = false;
        u.data_details = false;
        Ok(())
    })();
    u.shown = shown;
    u.desired = desired;
    u.settings = false;
    u.records = false;
    u.history = false;
    u.scroll = 0;
    SetWindowPos(
        hwnd,
        null_mut(),
        0,
        0,
        original.right - original.left,
        original.bottom - original.top,
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
    settings_controls(hwnd, u);
    result
}
