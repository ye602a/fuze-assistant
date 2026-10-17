//! Explicit, rate-limited diagnostics. Only sanitized field shapes reach the UI.
use crate::{ClaimState, Workbench};
use fuze_lcu::stage_probe::{compare, Report};
use fuze_worker_host::desktop::frontend::Session;
use slint::{ComponentHandle, ModelRc, VecModel};
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
    // Retain only a sanitized snapshot for explicit export after pausing.
    last_feedback: Option<(u64, serde_json::Value)>,
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
pub fn details(report: &Report) -> Vec<String> {
    let value = fuze_lcu::stage_probe::feedback(report, None, 0);
    let mut rows = vec![format!(
        "结构声明字段：{}",
        value["schemaFields"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("、")
    )];
    for field in value["playerFields"].as_array().unwrap() {
        let kind = match field["kind"].as_str().unwrap_or("") {
            "null" => "空值",
            "boolean" => "布尔",
            "number" => "数值",
            "string" => "文本",
            "array" => "数组",
            "object" => "对象",
            _ => "未知类型",
        };
        let suffix = if let Some(n) = field["count"].as_u64() {
            format!(" · 数量 {n}（最多 32）")
        } else if let Some(n) = field["smallInteger"].as_u64() {
            format!(" · 小整数 {n}（不是确认轮次）")
        } else {
            String::new()
        };
        rows.push(format!(
            "{}：{kind}{suffix}",
            field["field"].as_str().unwrap_or("")
        ));
    }
    if rows.len() == 1 {
        rows.push("当前响应没有已知玩家强化字段；缺失字段不代表未领取。".into());
    }
    rows
}
impl Binding {
    pub fn feedback(&self) -> serde_json::Value {
        let context = self.session.diagnostic_context();
        match &self.last_feedback {
            Some((revision, value)) => serde_json::json!({
                "snapshot":value,"contextCurrent":context.1 && context.0==*revision,
                "note":"sanitized-snapshot-at-manual-check-not-current-claim"
            }),
            None => serde_json::json!({"status":"not-checked","actualClaimRound":null}),
        }
    }
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
                    let snapshot = fuze_lcu::stage_probe::feedback(
                        &report,
                        self.baseline.as_ref(),
                        state.get_scenario().clamp(0, 5) as u8,
                    );
                    let mut field_details = details(&report);
                    if let Some(before) = &self.baseline {
                        let result = compare(before, &report, report.checked_at);
                        for field in &result.changed_fields {
                            field_details.push(format!("前后变化字段：{field}（不代表领取）"));
                        }
                        state.set_comparison(if result.status == "comparable" {
                            format!(
                                "前后对比：{} 项字段变化，相关事件变化 {} 条。字段名见详情；线索不代表领取。",
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
                    state.set_details(ModelRc::new(VecModel::from(
                        field_details
                            .into_iter()
                            .map(Into::into)
                            .collect::<Vec<_>>(),
                    )));
                    self.last_feedback = Some((report.revision, snapshot));
                    state.set_notice(
                        "检查完成；可查看字段详情，暂停后可导出脱敏反馈。不自动保存或上传。".into(),
                    );
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
            state.set_notice("对局上下文已变化，当前检查与基线已清除；脱敏快照仅供手动导出，不再作为当前对局依据。".into());
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
        state.set_details(Default::default());
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
        last_feedback: None,
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
                b.last_feedback = None;
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
                } else {
                    state.set_notice(
                        "当前没有同一对局的有效检查，请先检查一次；未建立基线。".into(),
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
