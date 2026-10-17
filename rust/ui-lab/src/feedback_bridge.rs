//! Existing UI polling feeds a bounded background journal; export stays explicit.
use crate::{DiagnosticState, HostState, Workbench};
use fuze_worker_host::{
    desktop::{frontend::Session, Status},
    record_export, runtime_log,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
};

pub struct Binding {
    upload: Rc<RefCell<crate::feedback_upload_bridge::Binding>>,
    recorder: crate::feedback::Recorder,
    pending: Option<(Receiver<Result<(), &'static str>>, JoinHandle<()>)>,
    session: Rc<Session>,
    journal: Option<runtime_log::Journal>,
    log_status: (u8, u64),
}
impl Binding {
    pub fn poll(&mut self, ui: &Workbench, status: &Status, offline: bool) {
        self.upload.borrow_mut().poll(ui, status);
        let state = ui.global::<DiagnosticState>();
        if let Some(journal) = &mut self.journal {
            journal.observe(status);
        }
        let marker = self
            .journal
            .as_ref()
            .map(|j| (j.status(), j.dropped()))
            .unwrap_or((if offline { 4 } else { 5 }, 0));
        if self.log_status != marker {
            self.log_status = marker;
            let hint = match marker.0 {
                0 => "日志：正在准备本地文件。".to_owned(),
                1 => format!(
                    "日志：自动保存中 · 最多 3 × 256 KB · 队列丢弃 {} 条",
                    marker.1
                ),
                3 => "日志未写入：另一助手实例正在使用日志文件。".into(),
                4 => "日志：隔离验证不写入个人目录。".into(),
                5 => "日志不可用：未取得本地数据目录。可重试日志。".into(),
                _ => "日志写入失败：请检查本地目录权限或剩余空间，再点重试日志。".into(),
            };
            state.set_log_status(hint.into());
            state.set_log_retry(!offline && self.journal.as_ref().is_none_or(|j| j.status() >= 2));
        }
        if self.recorder.observe(status, offline) {
            state.set_summary(self.recorder.summary().into());
            state.set_rows(ModelRc::new(VecModel::from(
                self.recorder
                    .rows()
                    .into_iter()
                    .map(Into::into)
                    .collect::<Vec<_>>(),
            )));
        }
        let result = self
            .pending
            .as_ref()
            .and_then(|(rx, _)| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("export-worker")),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            if let Some((_, worker)) = self.pending.take() {
                let _ = worker.join();
            }
            self.session.end_maintenance();
            state.set_busy(false);
            notice(ui, result);
        }
    }
    pub fn finish(&mut self) {
        self.upload.borrow_mut().finish();
        if let Some(journal) = &mut self.journal {
            journal.stop();
        }
    }
}
impl Drop for Binding {
    fn drop(&mut self) {
        self.finish();
        if let Some((_, worker)) = self.pending.take() {
            let _ = worker.join();
            self.session.end_maintenance();
        }
    }
}
fn notice(ui: &Workbench, result: Result<(), &'static str>) {
    ui.global::<HostState>().set_notice(
        match result {
            Ok(()) => "脱敏反馈已保存到你选择的位置；没有自动上传。",
            Err("export-cancelled") => "已取消导出，没有写入文件。",
            Err("frontend-pause-required") => "请先暂停识别，再导出本次运行反馈。",
            Err("frontend-busy") => "当前操作尚未结束，请稍候再导出。",
            Err("export-create-new") => "该位置无法新建文件，可能已存在同名文件；请换一个文件名。",
            Err("export-dialog") => "保存窗口未能打开；没有写入文件，请稍后重试。",
            Err("export-path") => "请选择可写的本地文件夹，使用 .json 文件名。",
            Err("frontend-offline-operation") => "隔离验证不会打开系统保存窗口。",
            Err("export-log-partial") => {
                "反馈已保存，但持久化日志有缺失或损坏，文件已注明；可检查日志状态后再导出。"
            }
            Err("log-open") => "日志目录未能打开；可以使用包内反馈脚本导出已有日志。",
            Err("log-busy" | "log-unavailable") => {
                "持久化日志暂不可读取；请检查日志状态后重试，已有日志保留。"
            }
            _ => "反馈未能保存；现有文件未被覆盖，可换一个本地目录重试。",
        }
        .into(),
    );
}
pub fn attach(
    ui: &Workbench,
    session: Rc<Session>,
    diagnostics: Rc<RefCell<crate::diagnostic_bridge::Binding>>,
    offline: bool,
) -> Rc<RefCell<Binding>> {
    let binding = Rc::new(RefCell::new(Binding {
        upload: crate::feedback_upload_bridge::attach(ui, offline),
        recorder: Default::default(),
        pending: None,
        session: session.clone(),
        journal: if offline {
            None
        } else {
            runtime_log::root().ok().map(runtime_log::Journal::start)
        },
        log_status: (u8::MAX, 0),
    }));
    let model = binding.clone();
    let weak = ui.as_weak();
    ui.global::<DiagnosticState>().on_action(move |action| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if offline {
            notice(&ui, Err("frontend-offline-operation"));
            return;
        }
        if action == "open-logs" {
            let result = (|| {
                let path = runtime_log::root().map_err(|_| "log-open")?;
                if !path.is_dir() {
                    return Err("log-open");
                }
                let hwnd = crate::lifecycle::hwnd(ui.window()).ok_or("log-open")?;
                let verb: Vec<u16> = "open\0".encode_utf16().collect();
                use std::os::windows::ffi::OsStrExt;
                let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                let code = unsafe {
                    windows_sys::Win32::UI::Shell::ShellExecuteW(
                        hwnd,
                        verb.as_ptr(),
                        target.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        1,
                    )
                };
                if code as isize <= 32 {
                    Err("log-open")
                } else {
                    Ok(())
                }
            })();
            if let Err(e) = result {
                notice(&ui, Err(e));
            } else {
                ui.global::<HostState>()
                    .set_notice("已请求打开日志目录；反馈时请发送导出的脱敏 JSON。".into());
            }
            return;
        }
        if action == "retry-log" {
            let mut binding = model.borrow_mut();
            if binding.journal.as_ref().is_none_or(|j| j.status() >= 2) {
                binding.journal = runtime_log::root().ok().map(runtime_log::Journal::start);
                ui.global::<HostState>()
                    .set_notice("正在重试本地日志；结果以日志状态为准。".into());
            }
            return;
        }
        if action != "export" {
            return;
        }
        if let Err(error) = session.begin_maintenance() {
            notice(&ui, Err(error));
            return;
        }
        ui.global::<DiagnosticState>().set_busy(true);
        let result = (|| {
            let claim = diagnostics.borrow().feedback();
            let bytes = model.borrow().recorder.encode(claim)?;
            let hwnd = crate::lifecycle::hwnd(ui.window()).ok_or("export-dialog")?;
            // No RefCell borrow is held while the owned native dialog runs its message loop.
            let path =
                unsafe { record_export::choose_feedback(hwnd) }?.ok_or("export-cancelled")?;
            let log = model
                .borrow()
                .journal
                .as_ref()
                .and_then(|journal| journal.export().ok());
            let log_root = runtime_log::root().ok();
            let (tx, rx) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("fuze-feedback".into())
                .spawn(move || {
                    let result = (|| {
                        let logs = if let Some(rx) = log {
                            rx.recv_timeout(std::time::Duration::from_secs(5))
                                .unwrap_or(Err("log-unavailable"))
                        } else {
                            log_root
                                .as_ref()
                                .ok_or("log-unavailable")
                                .and_then(|root| runtime_log::read(root))
                        };
                        let mut report: serde_json::Value =
                            serde_json::from_slice(&bytes).map_err(|_| "export-encode")?;
                        report["persistentLog"] = logs.unwrap_or_else(
                            |_| serde_json::json!({"status":"unavailable","records":[]}),
                        );
                        let bytes =
                            serde_json::to_vec_pretty(&report).map_err(|_| "export-encode")?;
                        record_export::save(record_export::Request { path, bytes })?;
                        if report["persistentLog"]["status"] == "ok" {
                            Ok(())
                        } else {
                            Err("export-log-partial")
                        }
                    })();
                    let _ = tx.send(result);
                })
                .map_err(|_| "export-worker")?;
            model.borrow_mut().pending = Some((rx, worker));
            Ok(())
        })();
        if let Err(error) = result {
            session.end_maintenance();
            ui.global::<DiagnosticState>().set_busy(false);
            notice(&ui, Err(error));
        } else {
            ui.global::<HostState>()
                .set_notice("正在保存脱敏反馈…".into());
        }
    });
    binding
}
