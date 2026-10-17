//! Explicit maintenance only. IO stays off the UI thread; tokens never enter Slint models.
use crate::{HostState, MaintenanceState, Workbench};
use fuze_worker_host::{desktop::frontend::Session, storage, update_channel};
use slint::{ComponentHandle, VecModel};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};
#[derive(Clone, Copy)]
enum Action {
    Scan,
    Clean,
    Check(bool),
    Credentials,
    Forget,
}
#[derive(Default)]
struct Reply {
    read_only: bool,
    notice: String,
    storage: Option<storage::Preview>,
    update: Option<String>,
    installable: bool,
    error: Option<&'static str>,
}
#[derive(Default)]
struct Work {
    next: Option<Action>,
    reply: Option<Reply>,
}
fn perform(
    action: Action,
    offline: bool,
    fixture_root: &Option<PathBuf>,
    preview: &mut Option<storage::Preview>,
    cancel: &AtomicBool,
) -> Result<Reply, &'static str> {
    if offline && !(fixture_root.is_some() && matches!(action, Action::Scan | Action::Clean)) {
        return Err("maintenance-offline");
    }
    let mut reply = Reply::default();
    match action {
        Action::Scan | Action::Clean => {
            let root = match fixture_root {
                Some(p) => p.clone(),
                None => fuze_worker_host::preferences::path()?
                    .parent()
                    .ok_or("storage-root")?
                    .to_path_buf(),
            };
            let scanned = if matches!(action, Action::Clean) {
                storage::clean(preview.take().ok_or("storage-preview-required")?)?
            } else {
                storage::scan(&root)?
            };
            *preview = Some(scanned.clone());
            reply.notice = if matches!(action, Action::Clean) {
                "清理完成，已重新扫描；当前数据和个人记录保留。".into()
            } else if scanned.removable == 0 {
                "扫描完成，目前没有符合安全清理条件的文件。".into()
            } else {
                format!(
                    "扫描完成，可回收 {:.1} MB；确认清理后才会删除。",
                    scanned.reclaimable as f64 / 1048576.
                )
            };
            reply.storage = Some(scanned);
        }
        Action::Check(private) => {
            let channel = if private {
                update_channel::Session::private()?
            } else {
                update_channel::Session::public()
            };
            let release = channel.check(&|| cancel.load(Ordering::Acquire))?;
            let dir = std::env::current_exe()
                .map_err(|_| "update-location")?
                .parent()
                .ok_or("update-location")?
                .to_owned();
            let bundled = fuze_worker_host::managed_install::bundled_version(&dir)?;
            let installed = fuze_worker_host::managed_install::read(
                &fuze_worker_host::managed_install::root()?,
            )?;
            // A portable test build may be newer than a previously managed installation.
            let current = installed
                .current
                .as_ref()
                .filter(|v| v.sequence > bundled.sequence)
                .unwrap_or(&bundled);
            let rejected = installed.rejected.as_ref() == Some(&release.manifest.revision);
            reply.installable = !installed.pending
                && !rejected
                && release.manifest.sequence > current.sequence
                && release.manifest.revision != current.revision;
            reply.update = Some(if installed.pending {
                "上一版本仍等待启动验证，暂不能安装更新。".into()
            } else if rejected {
                "该版本曾启动失败，已保留当前版本；等待后续修复版。".into()
            } else if reply.installable {
                format!(
                    "发现版本 {} · 完整包约 {} MB\n更新程序将优先差分下载，失败时回退全量。",
                    release.manifest.revision,
                    release.manifest.size.div_ceil(1024 * 1024)
                )
            } else if release.manifest.sequence < current.sequence {
                format!(
                    "本机版本较新；线上测试频道仍为 {}，不会降级。",
                    release.manifest.revision
                )
            } else {
                format!("已是当前测试频道版本 {}。", release.manifest.revision)
            });
            reply.notice = reply.update.clone().unwrap_or_default();
        }
        Action::Credentials => {
            fuze_worker_host::update_credential::prompt()?;
            reply.update = Some("授权已安全保存；请再次检查新版确认访问权限。".into());
            reply.notice = reply.update.clone().unwrap_or_default();
        }
        Action::Forget => {
            fuze_worker_host::update_credential::remove()?;
            reply.update = Some("已移除私有频道令牌。".into());
            reply.notice = reply.update.clone().unwrap_or_default();
        }
    }
    Ok(reply)
}
struct Worker {
    work: Arc<(Mutex<Work>, Condvar)>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    fn new(offline: bool, fixture_root: Option<PathBuf>) -> Result<Self, &'static str> {
        let work = Arc::new((Mutex::new(Work::default()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let task = work.clone();
        let cancel = stop.clone();
        let thread = std::thread::Builder::new()
            .name("fuze-slint-maintenance".into())
            .spawn(move || {
                let mut preview = None;
                loop {
                    let mut s = match task.0.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    while s.next.is_none() && !cancel.load(Ordering::Acquire) {
                        s = match task.1.wait(s) {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                    if cancel.load(Ordering::Acquire) {
                        return;
                    }
                    let action = s.next.take().unwrap();
                    drop(s);
                    let result = perform(action, offline, &fixture_root, &mut preview, &cancel);
                    if let Ok(mut s) = task.0.lock() {
                        let mut reply = result.unwrap_or_else(|e| Reply {
                            error: Some(e),
                            update: matches!(
                                action,
                                Action::Check(_) | Action::Credentials | Action::Forget
                            )
                            .then(|| message(e).to_owned()),
                            ..Default::default()
                        });
                        reply.read_only = matches!(action, Action::Check(_));
                        s.reply = Some(reply);
                    }
                }
            })
            .map_err(|_| "maintenance-worker")?;
        Ok(Self {
            work,
            stop,
            thread: Some(thread),
        })
    }
    fn request(&self, action: Action) {
        if let Ok(mut s) = self.work.0.lock() {
            s.reply = None;
            s.next = Some(action);
            self.work.1.notify_one();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Hold the predicate lock to avoid a missed wake between the worker's check and wait.
        let guard = self.work.0.lock().ok();
        self.stop.store(true, Ordering::Release);
        self.work.1.notify_one();
        drop(guard);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub struct Handoff {
    helper: PathBuf,
    private: bool,
}
impl Handoff {
    pub(crate) fn prepare(private: bool) -> Result<Self, &'static str> {
        let helper = std::env::current_exe()
            .map_err(|_| "update-location")?
            .with_file_name("fuze-update.exe");
        let meta = std::fs::symlink_metadata(&helper).map_err(|_| "update-helper-missing")?;
        use std::os::windows::fs::MetadataExt;
        if !meta.is_file() || meta.len() == 0 || meta.file_attributes() & 0x400 != 0 {
            return Err("update-helper-missing");
        }
        Ok(Self { helper, private })
    }
    pub fn launch(self) -> Result<(), &'static str> {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(self.helper)
            .arg(if self.private {
                "--update-private"
            } else {
                "--update"
            })
            .env("FUZE_UPDATE_LOCALE", "zh-CN")
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| "update-helper-start")?;
        Ok(())
    }
    pub(crate) fn private_channel(&self) -> bool {
        self.private
    }
}
pub struct Binding {
    _worker: Rc<Worker>,
    _timer: Rc<slint::Timer>,
    handoff: Rc<RefCell<Option<Handoff>>>,
    session: Rc<Session>,
}
impl Binding {
    pub fn take_handoff(&self) -> Option<Handoff> {
        self.handoff.borrow_mut().take()
    }
}
impl Drop for Binding {
    fn drop(&mut self) {
        self._timer.stop();
        self.session.end_maintenance();
    }
}
fn message(error: &str) -> &'static str {
    match error {
        "frontend-pause-required" => "请先暂停识别，再执行维护操作。",
        "frontend-busy" => "当前操作尚未结束，请稍候。",
        "maintenance-offline" => "离线验证不会访问网络、令牌或个人目录。",
        "credential-unavailable" => "尚未保存私有频道令牌，请先设置授权。",
        "credential-cancelled" => "已取消授权输入，原授权保持不变。",
        "credential-format" => "授权格式无效，未保存。请使用 GitHub 访问令牌。",
        "credential-save" | "credential-delete" => "Windows 安全存储操作未完成，请稍后重试。",
        "update-access-or-release" => "无法读取公开测试频道，请稍后重试；下载更新不需要上传令牌。",
        "update-cancelled-or-timeout" | "update-http-status" => {
            "更新检查未完成，请检查网络后重试；当前版本仍可使用。"
        }
        "update-release-assets" | "update-release-channel" | "update-manifest-policy" => {
            "频道尚未提供适用的完整 Rust 更新包，当前版本继续保留。"
        }
        "storage-preview-required" | "storage-preview-expired" | "storage-changed" => {
            "清理预览已失效，请重新扫描后确认。"
        }
        "storage-reparse" => "目录包含链接，已停止清理；请重新扫描确认结果。",
        "update-helper-missing" => "缺少配套更新程序；当前程序保持运行，请使用完整阶段测试包。",
        "install-build" => "当前为未打包开发构建，不能判定可安装版本。",
        _ => "操作未完成。请稍后重试；更新未安装，清理结果请重新扫描确认。",
    }
}
pub fn attach(
    ui: &Workbench,
    session: Rc<Session>,
    offline: bool,
    fixture_root: Option<PathBuf>,
) -> Result<Binding, &'static str> {
    let worker = Rc::new(Worker::new(offline, fixture_root)?);
    let timer = Rc::new(slint::Timer::default());
    let handoff = Rc::new(RefCell::new(None));
    let state = ui.global::<MaintenanceState>();
    state.set_attached(true);
    state.set_offline(offline);
    if !offline {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(ToOwned::to_owned))
        {
            if let Ok(version) = fuze_worker_host::managed_install::bundled_version(&dir) {
                state.set_current_version(version.revision.into());
            }
        }
    }
    let weak = ui.as_weak();
    let service = worker.clone();
    let engine = session.clone();
    let poll = timer.clone();
    let pending = handoff.clone();
    state.on_action(move |name| {
        let Some(ui) = weak.upgrade() else { return };
        let state = ui.global::<MaintenanceState>();
        if name == "cancel" {
            state.set_confirmation("".into());
            state.set_notice("已取消本次操作。".into());
            return;
        }
        if state.get_busy() {
            return;
        }
        if matches!(name.as_str(), "clean" | "install" | "forget") {
            if engine.snapshot().mode != 0 {
                state.set_notice(message("frontend-pause-required").into());
                return;
            }
            if name == "clean" && !state.get_can_clean() {
                return;
            }
            if name == "install" && !state.get_installable() {
                return;
            }
            state.set_confirmation(name);
            return;
        }
        let operation = if name == "confirm" {
            let s = state.get_confirmation();
            state.set_confirmation("".into());
            s
        } else {
            name
        };
        if operation.is_empty() {
            return;
        }
        let read_only = operation == "check";
        if !read_only {
            if let Err(e) = engine.begin_maintenance() {
                state.set_notice(message(e).into());
                return;
            }
        }
        if operation == "install" {
            if offline {
                engine.end_maintenance();
                state.set_notice(message("maintenance-offline").into());
                return;
            }
            match Handoff::prepare(false) {
                Ok(next) => {
                    *pending.borrow_mut() = Some(next);
                    engine.stop();
                    let _ = slint::quit_event_loop();
                }
                Err(e) => {
                    engine.end_maintenance();
                    state.set_notice(message(e).into());
                }
            }
            return;
        }
        let action = match operation.as_str() {
            "scan" => Action::Scan,
            "clean" => Action::Clean,
            "check" => Action::Check(false),
            "credentials" => Action::Credentials,
            "forget" => Action::Forget,
            _ => {
                engine.end_maintenance();
                return;
            }
        };
        state.set_busy(true);
        if !read_only {
            ui.global::<HostState>().set_busy(true);
        }
        if read_only {
            state.set_update_status("正在检查公开测试频道，识别继续运行…".into());
        }
        state.set_notice(
            match action {
                Action::Scan => "正在扫描本地占用，不会删除文件…",
                Action::Clean => "正在清理已确认的可回收文件…",
                Action::Check(_) => "正在检查更新频道，不会下载或安装…",
                Action::Credentials => "请在 Windows 授权窗口填写令牌；取消会保留原授权。",
                Action::Forget => "正在从 Windows 安全存储移除授权…",
            }
            .into(),
        );
        if matches!(action, Action::Scan | Action::Clean) {
            state.set_can_clean(false);
        }
        if matches!(
            action,
            Action::Check(_) | Action::Credentials | Action::Forget
        ) {
            state.set_installable(false);
        }
        service.request(action);
        poll.restart();
    });
    let weak = ui.as_weak();
    let service = worker.clone();
    let engine = session.clone();
    let tick = Rc::downgrade(&timer);
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let reply = service.work.0.lock().ok().and_then(|mut s| s.reply.take());
            let Some(reply) = reply else { return };
            if !reply.read_only {
                engine.end_maintenance();
            }
            if let Some(timer) = tick.upgrade() {
                timer.stop();
            }
            let Some(ui) = weak.upgrade() else { return };
            let state = ui.global::<MaintenanceState>();
            state.set_busy(false);
            ui.global::<HostState>().set_busy(engine.busy());
            state.set_notice(reply.error.map(message).unwrap_or(&reply.notice).into());
            if let Some(preview) = reply.storage {
                state.set_storage_summary(
                    format!(
                        "已扫描 {} 个文件 · 共 {:.1} MB\n可清理 {} 项 · {:.1} MB{}",
                        preview.files,
                        preview.bytes as f64 / 1048576.,
                        preview.removable,
                        preview.reclaimable as f64 / 1048576.,
                        if preview.protection_unknown {
                            " · 当前版本保护信息不完整，相关数据不清理"
                        } else {
                            ""
                        }
                    )
                    .into(),
                );
                state.set_storage_rows(
                    Rc::new(VecModel::from(
                        preview
                            .categories
                            .iter()
                            .map(|(name, files, bytes)| {
                                format!(
                                    "{name} · {files} 个文件 · {:.1} MB",
                                    *bytes as f64 / 1048576.
                                )
                                .into()
                            })
                            .collect::<Vec<_>>(),
                    ))
                    .into(),
                );
                state.set_can_clean(preview.removable > 0);
            }
            if let Some(update) = reply.update {
                state.set_update_status(update.into());
                state.set_installable(reply.installable);
            }
        },
    );
    timer.stop();
    Ok(Binding {
        _worker: worker,
        _timer: timer,
        handoff,
        session,
    })
}
