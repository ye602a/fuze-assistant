//! Explicit, rate-limited diagnostics. Only sanitized field shapes reach the UI.
use crate::{ClaimState, Workbench};
use fuze_lcu::stage_probe::{compare, Report};
use fuze_worker_host::desktop::frontend::Session;
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

pub struct Binding {
    session: Rc<Session>,
    report: Option<Report>,
    baseline: Option<Report>,
    pending: Option<(u64, Receiver<Report>)>,
    generation: u64,
    last: Option<Instant>,
}
pub fn summary(report: &Report) -> String {
    let status = |s: &str| match s {
        "ok" => "响应成功",
        "too-large" => "响应超过安全上限",
        "invalid-json" => "响应格式异常",
        "http-error" => "接口返回错误",
        _ => "暂不可用",
    };
    format!("接口结构：{}　当前字段：{}　对局事件：{}\n等级：{} · 相关结构字段 {} 项 · 玩家字段 {} 项 · 相关事件 {} 条{}\n{}；实际领取轮次仍未知，自动推荐继续整体胜率。",
        status(report.endpoints[0]), status(report.endpoints[1]), status(report.endpoints[2]),
        report.level.map(|n|n.to_string()).unwrap_or_else(||"未知".into()), report.schema_fields.len(), report.player_fields.len(), report.augment_event_count,
        if report.truncated { "（已限制读取范围）" } else { "" },
        if report.stable { "检查期间对局上下文未变化" } else { "对局上下文未确认或已经变化，不可用于前后对比" })
}
impl Binding {
    pub fn poll(&mut self, ui: &Workbench) {
        let state = ui.global::<ClaimState>();
        let remaining = self
            .last
            .map(|at| {
                Duration::from_secs(30)
                    .saturating_sub(at.elapsed())
                    .as_secs()
                    .saturating_add(u64::from(at.elapsed() < Duration::from_secs(30)))
            })
            .unwrap_or(0) as i32;
        if state.get_cooldown() != remaining {
            state.set_cooldown(remaining);
        }
        let reply = self
            .pending
            .as_ref()
            .and_then(|(generation, rx)| match rx.try_recv() {
                Ok(report) => Some((*generation, Some(report))),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((*generation, None)),
                Err(_) => None,
            });
        if let Some((generation, report)) = reply {
            self.pending = None;
            state.set_busy(false);
            if generation == self.generation {
                if let Some(mut report) = report {
                    let context = self.session.diagnostic_context();
                    report.stable &= context.1 && context.0 == report.revision;
                    if let Some(before) = &self.baseline {
                        let result = compare(before, &report, report.checked_at);
                        state.set_comparison(if result.status == "comparable" {
                            format!(
                                "前后对比：{} 项字段变化，相关事件变化 {} 条。线索不代表领取。",
                                result.changed_fields.len(),
                                result
                                    .event_delta
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "未知".into())
                            )
                            .into()
                        } else {
                            "两次检查不可对比，已清除基线；请在同一对局重新记录。".into()
                        });
                        if result.status != "comparable" {
                            self.baseline = None;
                        }
                    }
                    state.set_summary(summary(&report).into());
                    state.set_notice("检查完成；仅在本次会话显示，不自动保存或上传。".into());
                    state.set_can_mark(report.stable);
                    self.report = Some(report);
                } else {
                    state.set_notice("检查未完成，冷却结束后可重试。".into());
                    self.clear(ui);
                }
            }
        }
        if self.report.as_ref().is_some_and(|r| {
            let c = self.session.diagnostic_context();
            !c.1 || c.0 != r.revision
        }) {
            self.clear(ui);
            state.set_notice("对局上下文已变化，旧检查与基线已清除。".into());
        }
    }
    fn clear(&mut self, ui: &Workbench) {
        self.generation = self.generation.wrapping_add(1);
        self.report = None;
        self.baseline = None;
        let state = ui.global::<ClaimState>();
        state.set_summary("尚未检查；这项检查不会确认领取轮次。".into());
        state.set_comparison("".into());
        state.set_can_mark(false);
    }
}
pub fn attach(ui: &Workbench, session: Rc<Session>) -> Rc<RefCell<Binding>> {
    let binding = Rc::new(RefCell::new(Binding {
        session,
        report: None,
        baseline: None,
        pending: None,
        generation: 0,
        last: None,
    }));
    let handle = binding.clone();
    let weak = ui.as_weak();
    ui.global::<ClaimState>().on_action(move |action| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let state = ui.global::<ClaimState>();
        let mut b = handle.borrow_mut();
        match action.as_str() {
            "clear" => {
                b.clear(&ui);
                state.set_notice("已清除当前检查和基线。".into());
            }
            "mark" => {
                let c = b.session.diagnostic_context();
                if let Some(report) = b
                    .report
                    .as_ref()
                    .filter(|r| r.stable && c.1 && c.0 == r.revision)
                {
                    b.baseline = Some(report.clone());
                    state.set_comparison(
                        "已标记操作前基线。自行完成游戏操作后，再点检查一次。".into(),
                    );
                }
            }
            "inspect" if b.pending.is_none() => match b.session.probe_claim() {
                Ok(rx) => {
                    b.pending = Some((b.generation, rx));
                    b.last = Some(Instant::now());
                    state.set_busy(true);
                    state.set_cooldown(30);
                    state.set_notice("正在读取本地接口的字段类型与数量……".into());
                }
                Err(error) => state.set_notice(
                    match error {
                        "frontend-offline-operation" => "离线验证不会访问游戏接口。",
                        "probe-context" => "请在已确认连接的海克斯对局中检查；当前未发起请求。",
                        "probe-cooldown" => "两次检查至少间隔 30 秒，请稍候。",
                        "frontend-busy" => "当前操作尚未结束，请稍候。",
                        _ => "检查未能启动，现有推荐不受影响。",
                    }
                    .into(),
                ),
            },
            _ => (),
        }
    });
    binding
}
