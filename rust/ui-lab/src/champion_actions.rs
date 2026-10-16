//! Explicit player actions, separate from the read-only selection/recognition loop.
use crate::ChampionWindow;
use fuze_data::champion_detail::Detail;
use slint::{ComponentHandle, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{mpsc, Arc},
    time::Duration,
};

#[derive(Clone)]
struct Context {
    epoch: u64,
    id: u32,
    version: String,
    detail: Arc<Detail>,
}
#[derive(Default)]
struct State {
    epoch: u64,
    current: Option<Context>,
    pending: Option<Context>,
    result: Option<mpsc::Receiver<(u64, Result<(), &'static str>)>>,
}
pub struct Binding {
    state: Rc<RefCell<State>>,
    timer: Rc<slint::Timer>,
}
impl Binding {
    pub fn context(&self, ui: &ChampionWindow, value: Option<(u32, String, Arc<Detail>)>) {
        let mut s = self.state.borrow_mut();
        s.epoch = s.epoch.wrapping_add(1);
        let epoch = s.epoch;
        s.current = value.map(|(id, version, detail)| Context {
            epoch,
            id,
            version,
            detail,
        });
        s.pending = None;
        ui.set_item_confirm(false);
        ui.set_item_count(s.current.as_ref().map_or(0, |v| v.detail.item_sets.len()) as i32);
        ui.set_guides(
            Rc::new(VecModel::from(
                s.current
                    .as_ref()
                    .map(|v| {
                        v.detail
                            .guides
                            .iter()
                            .map(|g| g.title.clone().into())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            ))
            .into(),
        );
        ui.set_action_notice("".into());
    }
}
impl Drop for Binding {
    fn drop(&mut self) {
        self.timer.stop();
    }
}
pub fn attach(ui: &ChampionWindow) -> Binding {
    let state = Rc::new(RefCell::new(State::default()));
    let timer = Rc::new(slint::Timer::default());
    let weak = ui.as_weak();
    let shared = state.clone();
    let poll = timer.clone();
    ui.on_player_action(move |action,index| {
        let Some(ui)=weak.upgrade() else{return};
        if action=="cancel" { shared.borrow_mut().pending=None;ui.set_item_confirm(false);return; }
        if ui.get_action_busy() {return;}
        let result=(|| -> Result<(), &'static str> {
            // The offline harness cannot launch a browser or discover/connect to a client.
            if ui.get_offline() {return Err("offline");}
            let current=shared.borrow().current.clone().ok_or("missing")?;
            if current.id as i32!=ui.get_inspected_id() || current.id as i32!=ui.get_detail_champion() {return Err("changed");}
            if action=="guide" {
                let guide=current.detail.guides.get(index.max(0) as usize).ok_or("missing")?;
                if !fuze_data::champion_detail::guide_url(&guide.url) {return Err("guide");}
                let url:Vec<u16>=guide.url.encode_utf16().chain(Some(0)).collect();
                let verb:Vec<u16>="open".encode_utf16().chain(Some(0)).collect();
                let hwnd=crate::lifecycle::hwnd(ui.window()).ok_or("window")?;
                let code=unsafe { windows_sys::Win32::UI::Shell::ShellExecuteW(hwnd,verb.as_ptr(),url.as_ptr(),std::ptr::null(),std::ptr::null(),1) };
                if code as isize<=32 {return Err("guide");}
                ui.set_action_notice("已请求默认浏览器打开公开攻略。".into());return Ok(());
            }
            if ui.get_side_mode() || current.id as i32!=ui.get_local_id() || current.detail.item_sets.is_empty() {return Err("changed");}
            if action=="prepare-items" {
                ui.set_item_summary(format!("为 {} 配置 {} 套装备方案。\n仅替换助手管理的装备方案，保留你自己的方案。不会选择英雄、购买装备或设置符文。",current.detail.title,current.detail.item_sets.len()).into());
                shared.borrow_mut().pending=Some(current);ui.set_item_confirm(true);return Ok(());
            }
            if action!="confirm-items" {return Err("action");}
            ui.set_item_confirm(false);
            let pending=shared.borrow_mut().pending.take().ok_or("changed")?;
            if pending.epoch!=current.epoch || pending.id!=current.id || pending.version!=current.version {return Err("changed");}
            let (tx,rx)=mpsc::sync_channel(1);
            std::thread::Builder::new().name("fuze-slint-item-sets".into()).spawn(move || {
                let result=(|| {
                    let prefs=fuze_worker_host::preferences::read(&fuze_worker_host::preferences::path()?)?;
                    let manual=(!prefs.manual_directory.is_empty()).then(||std::path::Path::new(&prefs.manual_directory));
                    let client=fuze_lcu::discovery::HistorySession::open(manual)?;
                    // Existing transaction revalidates phase, current hero and account before writing.
                    client.install_item_sets(pending.id,&pending.detail.item_sets)
                })();
                let _=tx.send((pending.epoch,result));
            }).map_err(|_|"worker")?;
            shared.borrow_mut().result=Some(rx);ui.set_action_busy(true);
            ui.set_action_notice("正在核对当前选人并配置装备方案…".into());poll.restart();Ok(())
        })();
        if let Err(e)=result {
            ui.set_action_notice(match e {"offline"=>"离线预览不会打开网页或操作客户端。","changed"=>"当前英雄或选人状态已变化，请重新查看后操作。","guide"=>"无法打开攻略，请稍后重试。",_=>"操作未完成，请重新加载英雄详情。"}.into());
        }
    });
    let weak = ui.as_weak();
    let shared = state.clone();
    let tick = Rc::downgrade(&timer);
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let reply = shared.borrow().result.as_ref().map(|r| r.try_recv());
            let Some(reply) = reply else { return };
            if matches!(reply, Err(mpsc::TryRecvError::Empty)) {
                return;
            }
            shared.borrow_mut().result = None;
            if let Some(t) = tick.upgrade() {
                t.stop();
            }
            let Some(ui) = weak.upgrade() else { return };
            ui.set_action_busy(false);
            let Ok((epoch, result)) = reply else {
                ui.set_action_notice("配置任务未完成，请重试。".into());
                return;
            };
            if !shared
                .borrow()
                .current
                .as_ref()
                .is_some_and(|c| c.epoch == epoch)
            {
                return;
            }
            ui.set_action_notice(
                match result {
                    Ok(()) => "装备方案已配置。",
                    Err("item-sets-selection-changed") => "选人已变化，未提交装备方案。",
                    Err("item-sets-account-changed") => "账号已变化，未提交装备方案。",
                    Err(_) => "装备方案配置未成功，请确认客户端连接后重试。",
                }
                .into(),
            );
        },
    );
    timer.stop();
    Binding { state, timer }
}
