//! An explicit save action; no automatic logging and no extra recurring timers.
use crate::{DiagnosticState, HostState, Workbench};
use fuze_worker_host::{
    desktop::{frontend::Session, Status},
    record_export,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
};

pub struct Binding {
    recorder: crate::feedback::Recorder,
    pending: Option<(Receiver<Result<(), &'static str>>, JoinHandle<()>)>,
    session: Rc<Session>,
}
impl Binding {
    pub fn poll(&mut self, ui: &Workbench, status: &Status, offline: bool) {
        let state = ui.global::<DiagnosticState>();
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
}
impl Drop for Binding {
    fn drop(&mut self) {
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
        recorder: Default::default(),
        pending: None,
        session: session.clone(),
    }));
    let model = binding.clone();
    let weak = ui.as_weak();
    ui.global::<DiagnosticState>().on_action(move |action| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if action != "export" {
            return;
        }
        if offline {
            notice(&ui, Err("frontend-offline-operation"));
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
            let (tx, rx) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("fuze-feedback".into())
                .spawn(move || {
                    let _ = tx.send(record_export::save(record_export::Request { path, bytes }));
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
