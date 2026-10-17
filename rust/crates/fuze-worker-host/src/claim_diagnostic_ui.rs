//! Explicit bounded, in-memory before/after clues. Never runs at startup or during smoke.
use super::*;
use fuze_lcu::stage_probe::{Comparison, Report};
pub(super) const INSPECT: usize = 234;
pub(super) const BEFORE: usize = 235;
pub(super) const AFTER: usize = 236;
pub(super) const SCENARIO: usize = 237;
const SCENARIOS: [&str; 6] = [
    "普通海克斯",
    "刷新",
    "属性碎片",
    "锻造器",
    "额外选择",
    "重新连接",
];
#[derive(Default)]
pub(super) struct State {
    ready: bool,
    scenario: usize,
    last: Option<Instant>,
    job: Option<(usize, std::sync::mpsc::Receiver<Report>)>,
    report: Option<Report>,
    baseline: Option<Report>,
    comparison: Option<Comparison>,
    message: Option<&'static str>,
}
impl State {
    pub(super) fn sanitized(&self) -> serde_json::Value {
        serde_json::json!({"scenario":(["normal","reroll","shard","anvil","extra","reconnect"][self.scenario]),"report":self.report,"before":self.baseline,"comparison":self.comparison})
    }
}
pub(super) unsafe fn controls(hwnd: HWND, u: &Ui) {
    let s = &u.maintenance.claim;
    for (id, label) in [
        (INSPECT, "检查领取信号"),
        (BEFORE, "记录操作前"),
        (AFTER, "对比操作后"),
        (SCENARIO, SCENARIOS[s.scenario]),
    ] {
        let c = GetDlgItem(hwnd, id as i32);
        ShowWindow(c, if u.settings { SW_SHOWNA } else { SW_HIDE });
        SetWindowTextW(c, wide(label).as_ptr());
        EnableWindow(
            c,
            i32::from(
                !u.smoke
                    && s.job.is_none()
                    && (id == SCENARIO
                        || (matches!(u.desired, RUNNING | PAUSED)
                            && s.last
                                .is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
                            && (id != AFTER || s.baseline.is_some()))),
            ),
        );
    }
}
pub(super) unsafe fn layout(hwnd: HWND, u: &Ui) {
    if !u.settings {
        return;
    }
    let d = workbench::dpi(hwnd);
    let x = |v| v * d / 96;
    for (i, id) in [INSPECT, BEFORE, AFTER, SCENARIO].iter().enumerate() {
        workbench::place(
            hwnd,
            GetDlgItem(hwnd, *id as i32),
            x(48 + i as i32 * 183),
            x(1250),
            x(169),
            x(32),
            1,
        );
    }
}
pub(super) unsafe fn command(hwnd: HWND, u: &mut Ui, id: usize) {
    if u.smoke || !u.settings {
        return;
    }
    let s = &mut u.maintenance.claim;
    if s.job.is_some() {
        return;
    }
    if id == SCENARIO {
        s.scenario = (s.scenario + 1) % 6;
        s.baseline = None;
        s.comparison = None;
        controls(hwnd, u);
        InvalidateRect(hwnd, null(), 0);
        return;
    }
    if !matches!(u.desired, RUNNING | PAUSED)
        || s.last
            .is_some_and(|t| t.elapsed() < Duration::from_secs(30))
        || (id == AFTER && s.baseline.is_none())
    {
        return;
    }
    let control = u.shared.monitor.lock().ok().and_then(|v| v.clone());
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    if std::thread::Builder::new()
        .name("fuze-manual-claim-probe".into())
        .spawn(move || {
            let report = fuze_lcu::stage_probe::inspect(|| {
                control
                    .as_ref()
                    .map(|c| c.diagnostic_context())
                    .unwrap_or((0, false))
            });
            let _ = tx.send(report);
        })
        .is_ok()
    {
        s.job = Some((id, rx));
        s.last = Some(Instant::now());
        s.message = Some("正在检查；仅收集字段类型与数量。");
    }
    controls(hwnd, u);
    InvalidateRect(hwnd, null(), 0);
}
pub(super) unsafe fn poll(hwnd: HWND, u: &mut Ui) {
    let s = &mut u.maintenance.claim;
    let result = s.job.as_ref().and_then(|(id, rx)| match rx.try_recv() {
        Ok(v) => Some((*id, Some(v))),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((*id, None)),
        _ => None,
    });
    let completed = result.is_some();
    if let Some((id, value)) = result {
        s.job = None;
        if let Some(r) = value {
            if id == BEFORE {
                s.baseline = Some(r.clone());
                s.comparison = None;
            }
            if id == AFTER {
                if let Some(a) = &s.baseline {
                    s.comparison = Some(fuze_lcu::stage_probe::compare(a, &r, r.checked_at));
                }
            }
            s.report = Some(r);
            s.message = Some("检查完成；线索不代表实际领取，继续使用整体胜率。");
        } else {
            s.message = Some("检查未完成，可稍后重试。");
        }
        InvalidateRect(hwnd, null(), 0);
    }
    let ready = u.maintenance.claim.job.is_none()
        && u.maintenance
            .claim
            .last
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(30));
    if ready != u.maintenance.claim.ready || completed {
        u.maintenance.claim.ready = ready;
        controls(hwnd, u);
    }
}
pub(super) unsafe fn paint(dc: HDC, u: &Ui, right: i32, d: i32) {
    let x = |n| n * d / 96;
    let s = &u.maintenance.claim;
    text(
        dc,
        s.message
            .unwrap_or("手动检查有 30 秒间隔；前后对比需在同一局的 2 分钟内完成。"),
        RECT {
            left: x(48),
            top: x(workbench::settings_y(1290, u)),
            right: right - x(20),
            bottom: x(workbench::settings_y(1318, u)),
        },
        x(12),
        rgb(158, 184, 195),
    );
    if let Some(r) = &s.report {
        let line = format!(
            "{} {}/3 · {} {} · {} {}",
            crate::i18n::text("可用接口"),
            r.endpoints.iter().filter(|s| **s == "ok").count(),
            crate::i18n::text("候选字段"),
            r.player_fields.len(),
            crate::i18n::text("事件线索"),
            r.augment_event_count
        );
        text(
            dc,
            &line,
            RECT {
                left: x(48),
                top: x(workbench::settings_y(1320, u)),
                right: right - x(20),
                bottom: x(workbench::settings_y(1348, u)),
            },
            x(13),
            rgb(221, 232, 233),
        );
    }
    if let Some(c) = &s.comparison {
        let label = match c.status {
            "comparable" => "已对比；变化不等于领取确认",
            "expired" => "对比过期，请重新记录操作前",
            "context" => "对局连接未确认或已变化，不能对比",
            _ => "接口结果不完整，不能对比",
        };
        text(
            dc,
            label,
            RECT {
                left: x(48),
                top: x(workbench::settings_y(1350, u)),
                right: right - x(20),
                bottom: x(workbench::settings_y(1380, u)),
            },
            x(12),
            rgb(200, 170, 110),
        );
    }
}
