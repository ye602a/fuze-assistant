//! Small settings adapter. Match reporting neither pauses nor shares the OCR worker.
use crate::{DiagnosticState, Workbench};
use fuze_worker_host::{
    desktop::Status,
    github_feedback::{Client, Tracker, View},
    runtime_log::Snapshot,
};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, sync::mpsc, time::Instant};
pub struct Binding {
    client: Option<Client>,
    tracker: Option<Tracker>,
    last: Option<View>,
    epoch: Instant,
    credential: Option<mpsc::Receiver<Result<(), &'static str>>>,
    pending: Vec<fuze_worker_host::github_feedback::Report>,
    finished: bool,
}
impl Binding {
    pub fn poll(&mut self, ui: &Workbench, status: &Status) {
        let state = ui.global::<DiagnosticState>();
        if let Some(rx) = &self.credential {
            let result = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("credential-worker")),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.credential = None;
                state.set_upload_busy(false);
                state.set_upload_notice(
                    match result {
                        Ok(()) => "上传凭据已保存；请开启每局自动反馈。",
                        Err("credential-cancelled") => "已取消，现有上传凭据保持不变。",
                        _ => "上传凭据未能保存，请重试。",
                    }
                    .into(),
                );
                if let Some(c) = &self.client {
                    let _ = c.retry();
                }
            }
        }
        let Some(client) = &self.client else {
            return;
        };
        let view = client.view();
        if view != self.last.clone().unwrap_or_default() {
            state.set_upload_ready(view.ready);
            state.set_upload_enabled(view.enabled);
            state.set_upload_has_report(view.issue.is_some());
            let message = match view.status {
                "disabled" => "已关闭，不收集新的自动报告；待传摘要保留在本机。",
                "watching" => "已开启：每局赛后上传；首次故障提前上传，赛后更新同一报告。",
                "queued" => "脱敏摘要已进入上传队列。",
                "uploading" => "正在后台上传，识别继续运行。",
                "credential-unavailable" | "credential-format" | "feedback-auth" => {
                    "上传令牌缺失或失效，请配置独立的 Issues 读写令牌。"
                }
                "feedback-permission" => "仓库不可访问或 Issues 权限不足；请检查上传令牌。",
                "feedback-uncertain" => {
                    "上次上传结果未确认，正在查找已有报告以避免重复；本地摘要保留。"
                }
                "feedback-queue-full" => {
                    "待传队列已满（32 局），本局未能加入；请处理上传失败并手动导出反馈。"
                }
                "feedback-store" => "自动反馈文件无法读写，上传未完成；不会覆盖损坏的旧文件。",
                "feedback-rate-limit" => "GitHub 暂时限流，摘要保留，稍后重试。",
                "feedback-network" | "feedback-response" | "feedback-not-sent" => {
                    "GitHub 暂时不可达，摘要保留，稍后重试。"
                }
                "feedback-rejected" => "GitHub 拒绝了报告，请检查仓库 Issues 设置。",
                "" => "正在读取自动反馈设置…",
                _ => "自动反馈未完成，可重试；识别不受此操作阻塞。",
            };
            state.set_upload_status(
                format!(
                    "{message}\n待传 {} 份{}",
                    view.pending,
                    view.issue
                        .map(|n| format!(" · 最近报告 #{n}"))
                        .unwrap_or_default()
                )
                .into(),
            );
            self.last = Some(view.clone());
        }
        if view.enabled && view.ready {
            if self.tracker.is_none() {
                match Tracker::new() {
                    Ok(tracker) => self.tracker = Some(tracker),
                    Err(_) => {
                        state.set_upload_notice(
                            "报告标识未能初始化，自动反馈未开始；本地日志仍保留。".into(),
                        );
                        let _ = client.enable(false);
                        return;
                    }
                }
            }
            if let Some(tracker) = &mut self.tracker {
                if let Some(report) = tracker.observe(
                    Snapshot::from_status(status),
                    self.epoch.elapsed().as_millis() as u64,
                ) {
                    self.pending.push(report);
                }
            }
            while !self.pending.is_empty() {
                if client.submit(self.pending[0].clone()).is_err() {
                    break;
                }
                self.pending.remove(0);
            }
            if self.pending.len() > 4 {
                self.pending.truncate(4);
                state
                    .set_upload_notice("反馈工作线程拥堵，部分报告未排队；可使用手动导出。".into());
            }
        } else {
            self.tracker = None;
            self.pending.clear();
        }
    }
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(client) = &mut self.client {
            if let Some(report) = self.tracker.as_mut().and_then(Tracker::finish) {
                let _ = client.submit(report);
            }
            client.stop();
        }
    }
}
pub fn attach(ui: &Workbench, offline: bool) -> Rc<RefCell<Binding>> {
    let client = if offline { None } else { Client::start().ok() };
    ui.global::<DiagnosticState>().set_upload_status(
        if offline {
            "隔离界面验证：不读取凭据，不上传。"
        } else if client.is_none() {
            "自动反馈服务未能启动；本地日志仍独立保存。"
        } else {
            "正在读取自动反馈设置…"
        }
        .into(),
    );
    let binding = Rc::new(RefCell::new(Binding {
        client,
        tracker: None,
        last: None,
        epoch: Instant::now(),
        credential: None,
        pending: Vec::new(),
        finished: false,
    }));
    let weak = ui.as_weak();
    let model = binding.clone();
    ui.global::<DiagnosticState>()
        .on_upload_action(move |action| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if offline {
                ui.global::<DiagnosticState>()
                    .set_upload_notice("隔离验证禁止网络和凭据操作。".into());
                return;
            }
            let mut b = model.borrow_mut();
            if b.credential.is_some() {
                return;
            }
            let Some(client) = &b.client else {
                return;
            };
            let result = match action.as_str() {
                "toggle" => client.enable(!client.view().enabled),
                "retry" => client.retry(),
                "configure" => {
                    let (tx, rx) = mpsc::channel();
                    match std::thread::Builder::new()
                        .name("fuze-feedback-credential".into())
                        .spawn(move || {
                            let _ =
                                tx.send(fuze_worker_host::github_feedback::configure_credential());
                        }) {
                        Ok(_) => {
                            b.credential = Some(rx);
                            ui.global::<DiagnosticState>().set_upload_busy(true);
                            Ok(())
                        }
                        Err(_) => Err("credential-dialog"),
                    }
                }
                "remove" => client
                    .enable(false)
                    .and_then(|_| fuze_worker_host::github_feedback::remove_credential()),
                "open" => client
                    .view()
                    .issue
                    .ok_or("feedback-no-report")
                    .and_then(|n| {
                        use windows_sys::Win32::UI::Shell::ShellExecuteW;
                        let url: Vec<u16> =
                            format!("https://github.com/ye602a/fuze-assistant/issues/{n}\0")
                                .encode_utf16()
                                .collect();
                        let verb: Vec<u16> = "open\0".encode_utf16().collect();
                        let code = unsafe {
                            ShellExecuteW(
                                std::ptr::null_mut(),
                                verb.as_ptr(),
                                url.as_ptr(),
                                std::ptr::null(),
                                std::ptr::null(),
                                1,
                            )
                        };
                        if code as isize > 32 {
                            Ok(())
                        } else {
                            Err("feedback-open")
                        }
                    }),
                _ => return,
            };
            if action != "configure" || result.is_err() {
                ui.global::<DiagnosticState>().set_upload_notice(
                    match (action.as_str(), result) {
                        ("remove", Ok(())) => {
                            "已请求关闭自动上传并删除独立上传令牌；待传摘要保留。"
                        }
                        ("open", Ok(())) => "已请求在浏览器打开最近报告。",
                        (_, Ok(())) => "请求已提交，结果以上方状态为准。",
                        (_, Err(_)) => "操作未完成，请稍后重试；本地日志保留。",
                    }
                    .into(),
                );
            }
        });
    binding
}
