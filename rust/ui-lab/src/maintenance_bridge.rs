//! Startup check and explicit maintenance. Network work stays off the UI thread.
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
    revision: String,
    notes: String,
    error: Option<&'static str>,
    description: Option<update_channel::Release>,
}
#[derive(Default)]
struct Work {
    next: Option<Action>,
    reply: Option<Reply>,
    details: Option<update_channel::Release>,
    details_pending: bool,
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
            // Compare the running bundle, not a different installation's old pointer.
            let current = fuze_worker_host::managed_install::bundled_version(&dir)?;
            reply.revision = release.manifest.revision.clone();
            reply.notes = release.notes.clone();
            reply.installable = release.manifest.sequence > current.sequence
                && release.manifest.revision != current.revision;
            reply.update = Some(if reply.installable {
                format!(
                    "发现新版本 · 下载大小约 {} MB",
                    release.manifest.size.div_ceil(1024 * 1024)
                )
            } else if release.manifest.sequence < current.sequence {
                "当前版本比已发布版本更新。".into()
            } else {
                "已是最新版本。".into()
            });
            if reply.installable {
                reply.notes = "正在获取更新说明…".into();
                reply.description = Some(release);
            }
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
                    let mut description = None;
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
                        description = reply.description.take();
                        s.details_pending = description.is_some();
                        s.reply = Some(reply);
                    }
                    if let Some(mut release) = description {
                        update_channel::Session::public()
                            .describe(&mut release, &|| cancel.load(Ordering::Acquire));
                        if let Ok(mut s) = task.0.lock() {
                            s.details = Some(release);
                            s.details_pending = false;
                        }
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
            s.details = None;
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
            .arg("--confirmed")
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
        "maintenance-offline" => "离线模式下无法执行此操作。",
        "credential-unavailable" => "尚未保存私有频道令牌，请先设置授权。",
        "credential-cancelled" => "已取消授权输入，原授权保持不变。",
        "credential-format" => "授权格式无效，未保存。请使用 GitHub 访问令牌。",
        "credential-save" | "credential-delete" => "Windows 安全存储操作未完成，请稍后重试。",
        "update-access-or-release" => "无法访问更新文件，请稍后重试。",
        "update-cancelled-or-timeout" => "更新请求超时，请稍后重试。",
        "update-cancelled" => "已取消更新。",
        "update-network" => "无法连接更新服务器，请检查网络后重试。",
        "update-unauthorized" | "update-forbidden" => "更新服务器拒绝访问，请稍后重试。",
        "update-not-found" => "更新文件暂时不可用，请稍后重试。",
        "update-proxy-auth" => "网络代理要求身份验证，请检查代理设置。",
        "update-rate-limited" => "检查更新暂时受限，请稍后重试。",
        "update-http-status" => "更新服务器响应异常，请稍后重试。",
        "update-release-assets"
        | "update-release-channel"
        | "update-manifest-policy"
        | "update-manifest-format"
        | "update-digest-format" => "更新文件不完整或格式不正确，请稍后重试。",
        "storage-preview-required" | "storage-preview-expired" | "storage-changed" => {
            "清理预览已失效，请重新扫描后确认。"
        }
        "storage-reparse" => "目录包含链接，已停止清理；请重新扫描确认结果。",
        "update-helper-missing" => "缺少更新程序，请重新下载完整安装包。",
        "install-build" => "无法读取当前版本信息，请重新安装 FUZE。",
        _ => "操作未完成，请稍后重试。",
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
    let active = Rc::new(RefCell::new(
        None::<(
            fuze_worker_host::update_handoff::Signal,
            std::process::Child,
        )>,
    ));
    let target = Rc::new(RefCell::new(String::new()));
    let startup_check = Rc::new(std::cell::Cell::new(!offline));
    let state = ui.global::<MaintenanceState>();
    state.set_attached(true);
    state.set_offline(offline);
    if !offline {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(ToOwned::to_owned))
        {
            state.set_release_label(fuze_worker_host::build_identity::release_label(&dir).into());
            if let Some(version) = fuze_worker_host::build_identity::package_version(&dir) {
                state.set_current_version(version.into());
            }
        }
    }
    let weak = ui.as_weak();
    let service = worker.clone();
    let engine = session.clone();
    let poll = timer.clone();
    let active_start = active.clone();
    let target_start = target.clone();
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
        if matches!(name.as_str(), "clean" | "forget") {
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
        let read_only = operation == "check" || operation == "install";
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
            if !state.get_installable() || active_start.borrow().is_some() {
                return;
            }
            let result = (|| {
                let helper = Handoff::prepare(false)?;
                let (signal, name) = fuze_worker_host::update_handoff::Signal::create()?;
                let child = fuze_worker_host::in_place_update::launch_background(
                    helper.helper.parent().ok_or("update-location")?,
                    &name,
                    &target_start.borrow(),
                )?;
                // The user explicitly requested this updater window.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                        child.id(),
                    );
                }
                Ok::<_, &'static str>((signal, child))
            })();
            match result {
                Ok(update) => {
                    *active_start.borrow_mut() = Some(update);
                    state.set_busy(true);
                    state.set_confirmation("".into());
                    state.set_notice("正在后台下载，完成后自动更新并重启；识别继续运行。".into());
                    poll.restart();
                }
                Err(e) => state.set_notice(message(e).into()),
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
            state.set_update_status("正在检查更新…".into());
        }
        state.set_notice(
            match action {
                Action::Scan => "正在扫描本地占用，不会删除文件…",
                Action::Clean => "正在清理已确认的可回收文件…",
                Action::Check(_) => "正在检查更新…",
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
            {
                let mut slot = active.borrow_mut();
                if let Some((signal, child)) = slot.as_mut() {
                    if signal.ready() {
                        // Normal shutdown already joins capture/OCR and releases the instance lock.
                        // Arm the supervisor's bounded shutdown before dropping UI services.
                        use std::io::Write;
                        println!("FUZE_RENDER_STOPPING");
                        let _ = std::io::stdout().flush();
                        engine.stop();
                        let _ = slint::quit_event_loop();
                        return;
                    }
                    if child.try_wait().ok().flatten().is_some() {
                        *slot = None;
                        if let Some(ui) = weak.upgrade() {
                            ui.global::<MaintenanceState>().set_busy(false);
                            ui.global::<MaintenanceState>().set_notice(
                                "更新已结束，当前 FUZE 继续运行；可重新检查更新。".into(),
                            );
                        }
                        if let Some(timer) = tick.upgrade() {
                            timer.stop();
                        }
                    }
                    return;
                }
            }
            let reply = service.work.0.lock().ok().and_then(|mut s| s.reply.take());
            let Some(reply) = reply else {
                let (details, pending) = service
                    .work
                    .0
                    .lock()
                    .map(|mut s| (s.details.take(), s.details_pending))
                    .unwrap_or_default();
                let completed = details.is_some();
                if let Some(details) = details {
                    if *target.borrow() == details.manifest.revision {
                        if let Some(ui) = weak.upgrade() {
                            let state = ui.global::<MaintenanceState>();
                            state.set_update_notes(if details.notes.is_empty() {
                                "更新说明暂时不可用，可直接更新。".into()
                            } else {
                                details.notes.into()
                            });
                            if let Some(version) = details.version {
                                let status = format!(
                                    "发现版本 {version} · 下载大小约 {} MB",
                                    details.manifest.size.div_ceil(1024 * 1024)
                                );
                                if state.get_notice() == state.get_update_status() {
                                    state.set_notice(status.clone().into());
                                }
                                state.set_update_status(status.into());
                            }
                        }
                    }
                }
                if completed
                    && !pending
                    && weak
                        .upgrade()
                        .is_some_and(|ui| !ui.global::<MaintenanceState>().get_busy())
                {
                    if let Some(timer) = tick.upgrade() {
                        timer.stop();
                    }
                }
                return;
            };
            if !reply.read_only {
                engine.end_maintenance();
            }
            if !reply.installable {
                if let Some(timer) = tick.upgrade() {
                    timer.stop();
                }
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
                *target.borrow_mut() = reply.revision;
                state.set_update_notes(reply.notes.into());
                if startup_check.replace(false) && reply.installable {
                    state.set_confirmation("available".into());
                }
                state.set_update_status(update.into());
                state.set_installable(reply.installable);
            }
        },
    );
    if offline {
        timer.stop();
    } else {
        state.set_busy(true);
        worker.request(Action::Check(false));
    }
    Ok(Binding {
        _worker: worker,
        _timer: timer,
        handoff,
        session,
    })
}
