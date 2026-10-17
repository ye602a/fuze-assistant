//! Explicit record actions. Private match identifiers stay in Rust; UI rows use snapshot indices.
use crate::{HostState, ObservationRow, RecordState, Workbench};
use fuze_worker_host::{
    account_history::View,
    champion_assets::{Picture, Reply as Pictures},
    desktop::frontend::Session,
    preferences::Preferences,
    record_export,
};
use slint::{ComponentHandle, Model, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
enum Task {
    Icons(View, Vec<usize>, bool),
    Share(View, usize, bool),
    Settings(PathBuf, Preferences),
    Save(record_export::Request),
}
enum Reply {
    Icons(HashMap<String, Arc<Picture>>),
    Share {
        png: Vec<u8>,
        json: Vec<u8>,
        preview: image::RgbaImage,
    },
    Settings(Preferences),
    Saved,
}
#[derive(Default)]
struct Work {
    generation: u64,
    next: Option<Task>,
    reply: Option<Result<Reply, &'static str>>,
    stop: bool,
}
struct Worker {
    shared: Arc<(Mutex<Work>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    fn new(offline: bool) -> Result<Self, &'static str> {
        let shared = Arc::new((Mutex::new(Work::default()), Condvar::new()));
        let task = shared.clone();
        let thread = std::thread::Builder::new()
            .name("fuze-slint-records".into())
            .spawn(move || {
                let mut cache = crate::art_cache::Cache::new(offline);
                loop {
                    let mut s = match task.0.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    while !s.stop && s.next.is_none() {
                        s = match task.1.wait(s) {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                    if s.stop {
                        return;
                    }
                    let generation = s.generation;
                    let action = s.next.take().unwrap();
                    drop(s);
                    let cancelled = || {
                        task.0
                            .lock()
                            .map(|s| s.stop || s.generation != generation)
                            .unwrap_or(true)
                    };
                    let result = (|| -> Result<Reply, &'static str> {
                        match action {
                            Task::Icons(view, indices, download) => Ok(Reply::Icons(art(
                                &view, &indices, download, &mut cache, &cancelled,
                            ))),
                            Task::Share(view, index, download) => {
                                let row = view.rows.get(index).ok_or("record-selection")?;
                                let json = record_export::encode(
                                    row,
                                    view.observations.iter().find(|o| o.game_id == row.game_id),
                                    view.cached,
                                )?;
                                let pictures = Pictures {
                                    icons: art(&view, &[index], download, &mut cache, &cancelled),
                                    ..Default::default()
                                };
                                if cancelled() {
                                    return Err("record-cancelled");
                                }
                                let png = record_export::png(
                                    &json,
                                    view.details.get(index).map(|d| d.as_ref()),
                                    Some(&pictures),
                                )?;
                                let preview = image::load_from_memory(&png)
                                    .map_err(|_| "share-preview")?
                                    .resize(600, 490, image::imageops::FilterType::Triangle)
                                    .to_rgba8();
                                Ok(Reply::Share { png, json, preview })
                            }
                            Task::Settings(path, current) => {
                                let mut next =
                                    fuze_worker_host::legacy_preferences::read(&path, &current)?;
                                // Slint localization remains a single pending transaction; importing must not bypass it.
                                next.locale = current.locale;
                                next.automatic = current.automatic;
                                Ok(Reply::Settings(next))
                            }
                            Task::Save(request) => {
                                record_export::save(request)?;
                                Ok(Reply::Saved)
                            }
                        }
                    })();
                    if let Ok(mut s) = task.0.lock() {
                        if !s.stop && s.generation == generation {
                            s.reply = Some(result);
                        }
                    }
                }
            })
            .map_err(|_| "record-worker")?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }
    fn request(&self, task: Task) {
        if let Ok(mut s) = self.shared.0.lock() {
            s.generation = s.generation.wrapping_add(1);
            s.next = Some(task);
            s.reply = None;
            self.shared.1.notify_one();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Ok(mut s) = self.shared.0.lock() {
            s.stop = true;
            self.shared.1.notify_one();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn art(
    view: &View,
    indices: &[usize],
    download: bool,
    cache: &mut crate::art_cache::Cache,
    cancelled: &dyn Fn() -> bool,
) -> HashMap<String, Arc<Picture>> {
    let mut icons = HashMap::new();
    for index in indices {
        if let Some(detail) = view.details.get(*index) {
            for url in detail.lines.iter().flat_map(|l| &l.icons).take(20) {
                if cancelled() {
                    return icons;
                }
                if !icons.contains_key(url) {
                    if let Some(p) = cache.get(url, &view.asset_version, download, cancelled) {
                        icons.insert(url.clone(), p);
                    }
                }
            }
        }
    }
    icons
}
#[derive(Default)]
struct Display {
    view: View,
    chosen: HashSet<u64>,
    png: Vec<u8>,
    json: Vec<u8>,
    settings: Option<Preferences>,
}
pub struct Binding {
    worker: Rc<Worker>,
    timer: Rc<slint::Timer>,
    display: Rc<RefCell<Display>>,
    lease: Rc<Cell<bool>>,
    session: Rc<Session>,
    offline: bool,
}
impl Drop for Binding {
    fn drop(&mut self) {
        self.timer.stop();
        if self.lease.replace(false) {
            self.session.end_maintenance();
        }
    }
}
impl Binding {
    pub fn idle(&self) -> bool {
        !self.timer.running()
    }
    pub fn share_bytes(&self) -> (Vec<u8>, Vec<u8>) {
        let s = self.display.borrow();
        (s.png.clone(), s.json.clone())
    }
    /// Explicit fixture path only; native file dialogs and installed settings remain untouched.
    pub fn preview_settings_fixture(&self, path: PathBuf) -> Result<(), &'static str> {
        if !self.offline {
            return Err("record-fixture-only");
        }
        self.session.begin_maintenance()?;
        self.lease.set(true);
        self.worker
            .request(Task::Settings(path, self.session.snapshot().preferences));
        self.timer.restart();
        Ok(())
    }
}
fn message(error: &str) -> &'static str {
    match error {
        "frontend-pause-required" => "请先暂停识别，再导入或分享记录。",
        "frontend-busy" => "当前操作尚未结束，请稍候。",
        "frontend-offline-operation" => "离线验证不访问账号、文件选择框或剪贴板。",
        "export-create-new" => "目标文件已存在或不可写；请选择新的文件名，不会覆盖旧文件。",
        "share-clipboard-busy" => "剪贴板正被其他程序占用，请稍后重试。",
        "history-import-selection" => "请至少选择一场预览中的战绩。",
        _ => "操作未完成。原数据保留，请重新选择并重试。",
    }
}
fn events(ui: &Workbench, view: &View, index: Option<usize>, page: usize) {
    let state = ui.global::<RecordState>();
    let observation = if view.journal_preview {
        view.observations.first()
    } else {
        index
            .and_then(|i| view.rows.get(i))
            .and_then(|r| view.observations.iter().find(|o| o.game_id == r.game_id))
    };
    let rows = observation.map(|o| o.events.as_slice()).unwrap_or(&[]);
    let pages = rows.len().div_ceil(5).max(1);
    let page = page.min(pages - 1);
    state.set_observation_page(page as i32);
    state.set_observation_pages(pages as i32);
    state.set_observation_note(match observation {
        None => "本场没有绑定的观察记录；不从相邻对局补填。".into(),
        Some(o) => format!(
            "{} 次候选观察{}{}。观察顺序不等于领取轮次，自动推荐仍为整体胜率。",
            o.events.len(),
            if o.interrupted {
                " · 记录可能中断"
            } else {
                ""
            },
            if o.capped {
                " · 已达记录上限"
            } else {
                ""
            }
        )
        .into(),
    });
    state.set_observations(
        Rc::new(VecModel::from(
            rows.iter()
                .enumerate()
                .skip(page * 5)
                .take(5)
                .map(|(i, e)| ObservationRow {
                    title: format!(
                        "观察 {} · 等级 {}",
                        i + 1,
                        e.level.map(|n| n.to_string()).unwrap_or("未知".into())
                    )
                    .into(),
                    cards: e
                        .cards
                        .iter()
                        .map(|id| {
                            id.map(|n| {
                                view.augment_names
                                    .get(&n)
                                    .cloned()
                                    .unwrap_or_else(|| format!("强化 #{n}"))
                            })
                            .unwrap_or("此位置未识别".into())
                        })
                        .collect::<Vec<_>>()
                        .join(" / ")
                        .into(),
                    hint: if e.partial {
                        "部分卡位可读 · 非实际领取"
                    } else {
                        "候选卡片 · 非实际领取"
                    }
                    .into(),
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
}
pub fn attach(
    ui: &Workbench,
    session: Rc<Session>,
    offline: bool,
) -> Result<Binding, &'static str> {
    let worker = Rc::new(Worker::new(offline)?);
    let timer = Rc::new(slint::Timer::default());
    let display = Rc::new(RefCell::new(Display::default()));
    let lease = Rc::new(Cell::new(false));
    let weak = ui.as_weak();
    let model = display.clone();
    let service = worker.clone();
    let poll = timer.clone();
    let engine = session.clone();
    ui.global::<RecordState>().on_refresh(move || {
        let Some(ui) = weak.upgrade() else { return };
        let view = engine.snapshot().history;
        let mut s = model.borrow_mut();
        if s.view != view {
            s.chosen = view.rows.iter().map(|r| r.game_id).collect();
            s.view = view;
            ui.global::<RecordState>().set_confirmation("".into());
        }
        s.png.clear();
        s.json.clear();
        let state = ui.global::<RecordState>();
        state.set_share_ready(false);
        state.set_share_image(Default::default());
        state.set_import_count(s.view.pending as i32);
        state.set_chosen_count(if s.view.journal_preview {
            1
        } else {
            s.chosen.len() as i32
        });
        state.set_journal_preview(s.view.journal_preview);
        events(&ui, &s.view, None, 0);
        let rows = ui
            .global::<HostState>()
            .get_history()
            .iter()
            .map(|mut r| {
                r.chosen = s
                    .view
                    .rows
                    .get(r.source_index as usize)
                    .is_some_and(|v| s.chosen.contains(&v.game_id));
                r
            })
            .collect::<Vec<_>>();
        let indices = rows.iter().map(|r| r.source_index as usize).collect();
        ui.global::<HostState>()
            .set_history(Rc::new(VecModel::from(rows)).into());
        service.request(Task::Icons(
            s.view.clone(),
            indices,
            engine.snapshot().preferences.icon_downloads,
        ));
        poll.restart();
    });
    let weak = ui.as_weak();
    let model = display.clone();
    let service = worker.clone();
    let poll = timer.clone();
    let engine = session.clone();
    let reserved = lease.clone();
    ui.global::<RecordState>().on_action(move |action,index| {
        let Some(ui)=weak.upgrade() else {return}; let state=ui.global::<RecordState>(); let host=ui.global::<HostState>();
        if action=="cancel-confirm" { state.set_confirmation("".into()); model.borrow_mut().settings=None; state.set_notice("已取消确认，未写入任何数据。".into()); return; }
        if state.get_busy() {return;}
        let selected=host.get_history().row_data(index.max(0) as usize).map(|r|r.source_index as usize);
        if action=="select" {
            let next=if host.get_history_selected()==index {-1}else{index}; host.set_history_selected(next);
            state.set_share_ready(false); state.set_share_image(Default::default());
            { let mut s = model.borrow_mut(); s.png.clear(); s.json.clear(); }
            events(&ui,&model.borrow().view,if next<0 {None}else{selected},0); return;
        }
        if action=="observation-page" {
            let selected=host.get_history().row_data(host.get_history_selected().max(0) as usize).map(|r|r.source_index as usize);
            events(&ui,&model.borrow().view,selected,(state.get_observation_page()+index).max(0) as usize); return;
        }
        if action=="choose" {
            let mut s=model.borrow_mut(); if let Some(row)=selected.and_then(|i|s.view.rows.get(i)).cloned() {
                if !s.chosen.remove(&row.game_id) { s.chosen.insert(row.game_id); }
                let rows=host.get_history().iter().map(|mut r|{r.chosen=s.view.rows.get(r.source_index as usize).is_some_and(|v|s.chosen.contains(&v.game_id));r}).collect::<Vec<_>>();
                host.set_history(Rc::new(VecModel::from(rows)).into()); state.set_chosen_count(s.chosen.len() as i32);
            } return;
        }
        let result=(|| -> Result<(), &'static str> {
            if action=="confirm-import" {
                if state.get_chosen_count()==0 {return Err("history-import-selection");}
                state.set_confirm_text(format!("将导入 {} 项已预览记录。确认时会重新核对当前账号与区服；不明确或已变化时拒绝导入。\n原文件保留，观察记录不作为已领取强化。",state.get_chosen_count()).into()); state.set_confirmation("import".into()); return Ok(());
            }
            if action=="confirm" {
                let kind=state.get_confirmation(); state.set_confirmation("".into());
                if kind=="settings" {
                    let p=model.borrow_mut().settings.take().ok_or("legacy-settings-missing")?;
                    crate::host_bridge::submitted(&ui,engine.apply(p),"settings"); return Ok(());
                }
                if kind=="import" { crate::host_bridge::submitted(&ui,engine.confirm_history(if state.get_journal_preview() {vec![]}else{model.borrow().chosen.iter().copied().collect()}),"history"); return Ok(()); }
                return Ok(());
            }
            if action=="cancel-import" { crate::host_bridge::submitted(&ui,engine.cancel_history(),"history"); return Ok(()); }
            if action=="share" {
                if model.borrow().view.pending>0 { return Err("history-import-selection"); }
                let i=selected.ok_or("record-selection")?; engine.begin_maintenance()?; reserved.set(true);
                service.request(Task::Share(model.borrow().view.clone(),i,engine.snapshot().preferences.icon_downloads));
            } else {
                if offline { return Err("frontend-offline-operation"); }
                engine.begin_maintenance()?; reserved.set(true);
                let hwnd=crate::lifecycle::hwnd(ui.window()).ok_or("record-window")?;
                if action=="copy" {
                    if !state.get_share_ready() {return Err("record-selection");}
                    unsafe {record_export::copy_image(hwnd,&model.borrow().png)?;}
                    engine.end_maintenance(); reserved.set(false); state.set_notice("匿名图片已复制；未上传。".into()); return Ok(());
                }
                if action=="save" {
                    if !state.get_share_ready() {return Err("record-selection");}
                    let path=unsafe {record_export::choose(hwnd)};
                    let Some(path)=path else {engine.end_maintenance();reserved.set(false);state.set_notice("已取消保存，没有创建文件。".into());return Ok(())};
                    let s=model.borrow(); let bytes=if path.extension().is_some_and(|e|e=="png") {s.png.clone()}else{s.json.clone()};
                    service.request(Task::Save(record_export::Request {path,bytes}));
                } else if matches!(action.as_str(),"import-history"|"import-journal"|"import-settings") {
                    let path=unsafe {fuze_worker_host::legacy_preferences::choose(hwnd)};
                    let Some(path)=path else {engine.end_maintenance();reserved.set(false);state.set_notice("已取消选择，没有导入或修改任何数据。".into());return Ok(())};
                    if action=="import-settings" {service.request(Task::Settings(path,engine.snapshot().preferences));}
                    else {
                        let game=if action=="import-journal" {Some(selected.and_then(|i|model.borrow().view.rows.get(i).map(|r|r.game_id)).ok_or("record-selection")?)}else{None};
                        engine.end_maintenance();reserved.set(false);crate::host_bridge::submitted(&ui,engine.preview_history(path,game),"history");return Ok(());
                    }
                } else { return Err("record-action"); }
            }
            state.set_busy(true); host.set_busy(true); state.set_notice("正在处理，请稍候…".into()); poll.restart(); Ok(())
        })();
        if let Err(e)=result { if reserved.replace(false) {engine.end_maintenance();} state.set_notice(message(e).into()); }
    });
    let weak = ui.as_weak();
    let model = display.clone();
    let service = worker.clone();
    let engine = session.clone();
    let reserved = lease.clone();
    let tick = Rc::downgrade(&timer);
    timer.start(slint::TimerMode::Repeated,Duration::from_millis(100),move || {
        let reply=service.shared.0.lock().ok().and_then(|mut s|s.reply.take()); let Some(reply)=reply else{return};
        if let Some(t)=tick.upgrade(){t.stop();} if reserved.replace(false){engine.end_maintenance();}
        let Some(ui)=weak.upgrade()else{return};let state=ui.global::<RecordState>();state.set_busy(false);ui.global::<HostState>().set_busy(engine.busy());
        match reply {
            Ok(Reply::Icons(icons))=>{
                let images=icons.iter().map(|(u,p)|(u.clone(),crate::art_cache::image(p))).collect::<HashMap<_,_>>(); let s=model.borrow();
                let rows=ui.global::<HostState>().get_history().iter().map(|mut row|{
                    if let Some(d)=s.view.details.get(row.source_index as usize){
                        row.icon=d.lines.first().and_then(|l|l.icons.first()).and_then(|u|images.get(u)).cloned().unwrap_or_default();
                        let line=|n: usize|Rc::new(VecModel::from(d.lines.get(n).map(|l|l.icons.iter().map(|u|images.get(u).cloned().unwrap_or_default()).collect::<Vec<_>>()).unwrap_or_default())).into();
                        row.items=line(1);row.augments=line(2);
                    } row
                }).collect::<Vec<_>>();ui.global::<HostState>().set_history(Rc::new(VecModel::from(rows)).into());
            }
            Ok(Reply::Share{png,json,preview})=>{
                state.set_share_image(slint::Image::from_rgba8(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(preview.as_raw(),preview.width(),preview.height())));
                let mut s=model.borrow_mut();s.png=png;s.json=json;state.set_share_ready(true);state.set_notice("匿名图片预览已生成；尚未复制、保存或上传。".into());
            }
            Ok(Reply::Settings(p))=>{model.borrow_mut().settings=Some(p);state.set_notice("旧设置已读取，等待你确认后才会应用。".into());state.set_confirm_text("仅导入识别后端、等级门禁、浮窗和目录等受支持偏好。\n保留当前语言及启动状态，不复制密钥、账号或战绩。原设置文件保持不变。".into());state.set_confirmation("settings".into());}
            Ok(Reply::Saved)=>state.set_notice("已保存到你选择的新文件；未上传。".into()),
            Err(e)=>state.set_notice(message(e).into()),
        }
    });
    timer.stop();
    Ok(Binding {
        worker,
        timer,
        display,
        lease,
        session,
        offline,
    })
}
