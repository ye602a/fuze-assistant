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
    route: usize,
    automatic: bool,
}
#[derive(Default)]
struct State {
    epoch: u64,
    current: Option<Context>,
    pending: Option<Context>,
    result: Option<mpsc::Receiver<(u64, Result<(), &'static str>)>>,
    auto_attempt: Option<(u32, String, usize)>,
    auto_pending: Option<(u32, String, usize)>,
    auto_due: Option<std::time::Instant>,
}
impl State {
    fn auto_ready(&mut self, key: Option<(u32, String, usize)>, now: std::time::Instant) -> bool {
        let Some(key) = key else {
            self.auto_due = None;
            return false;
        };
        if self.auto_attempt.as_ref() == Some(&key) {
            return false;
        }
        if self.auto_pending.as_ref() != Some(&key) {
            self.auto_pending = Some(key.clone());
            self.auto_due = None;
        }
        let at = *self.auto_due.get_or_insert(now);
        if now.saturating_duration_since(at) < Duration::from_secs(1) {
            return false;
        }
        self.auto_attempt = Some(key);
        self.auto_due = None;
        true
    }
}
pub struct Binding {
    state: Rc<RefCell<State>>,
    timer: Rc<slint::Timer>,
}
impl Binding {
    pub fn automatic(&self, ui: &ChampionWindow, enabled: bool) {
        ui.set_auto_items(enabled);
        let mut s = self.state.borrow_mut();
        let ready = enabled
            && !ui.get_offline()
            && !ui.get_side_mode()
            && !ui.get_action_busy()
            && !ui.get_item_confirm();
        let context = s.current.clone().filter(|c| {
            c.id as i32 == ui.get_local_id()
                && c.id as i32 == ui.get_inspected_id()
                && !c.detail.item_sets.is_empty()
        });
        let key = context.filter(|_| ready).map(|c| {
            (
                c.id,
                c.version.clone(),
                ui.get_build_route().max(0) as usize,
            )
        });
        if !s.auto_ready(key, std::time::Instant::now()) {
            return;
        }
        drop(s);
        ui.invoke_player_action("auto-items".into(), 0);
    }
    pub fn context(&self, ui: &ChampionWindow, value: Option<(u32, String, Arc<Detail>)>) {
        let mut s = self.state.borrow_mut();
        s.epoch = s.epoch.wrapping_add(1);
        let epoch = s.epoch;
        s.auto_due = None;
        if value.is_none() {
            s.auto_attempt = None;
        }
        s.current = value.map(|(id, version, detail)| Context {
            epoch,
            id,
            version,
            detail,
            route: 0,
            automatic: false,
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
        if action=="cancel" { shared.borrow_mut().pending=None;ui.set_item_confirm(false);ui.set_action_notice("已取消配置，未修改客户端装备方案。".into());return; }
        if ui.get_action_busy() {return;}
        let result=(|| -> Result<(), &'static str> {
            // The offline harness cannot launch a browser or discover/connect to a client.
            if ui.get_offline() {return Err("offline");}
            let mut current=shared.borrow().current.clone().ok_or("missing")?;
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
            if action=="prepare-items" || action=="auto-items" {
                current.route=ui.get_build_route().max(0) as usize;
                if !current.detail.item_routes.contains(&current.route) {return Err("route-unavailable");}
                current.automatic=action=="auto-items";
                if current.automatic && !ui.get_auto_items() {return Err("changed");}
                ui.set_item_summary(format!("为 {} 导入路线 {}。\n仅替换助手管理的装备方案，保留你自己的方案。写入后核对客户端保存结果；商店显示需在游戏内确认。",current.detail.title,current.route+1).into());
                shared.borrow_mut().pending=Some(current.clone());
                if !current.automatic {ui.set_item_confirm(true);return Ok(());}
            }
            if action!="confirm-items" && action!="auto-items" {return Err("action");}
            ui.set_item_confirm(false);
            let pending=shared.borrow_mut().pending.take().ok_or("changed")?;
            if pending.epoch!=current.epoch || pending.id!=current.id || pending.version!=current.version || pending.route!=ui.get_build_route().max(0) as usize {return Err("changed");}
            shared.borrow_mut().auto_attempt=Some((pending.id,pending.version.clone(),pending.route));
            let (tx,rx)=mpsc::sync_channel(1);
            std::thread::Builder::new().name("fuze-slint-item-sets".into()).spawn(move || {
                let result=(|| {
                    let prefs=fuze_worker_host::preferences::read(&fuze_worker_host::preferences::path()?)?;
                    if pending.automatic && !prefs.auto_item_sets {return Err("item-sets-auto-disabled");}
                    let manual=(!prefs.manual_directory.is_empty()).then(||std::path::Path::new(&prefs.manual_directory));
                    let client=fuze_lcu::discovery::HistorySession::open(manual)?;
                    // Existing transaction revalidates phase, current hero and account before writing.
                    let index=pending.detail.item_routes.iter().position(|r|*r==pending.route).ok_or("item-sets-route-unavailable")?;
                    client.install_item_sets(pending.id,&[pending.detail.item_sets[index].clone()])
                })();
                let _=tx.send((pending.epoch,result));
            }).map_err(|_|"worker")?;
            shared.borrow_mut().result=Some(rx);ui.set_action_busy(true);
            ui.set_action_notice("正在核对当前选人并配置装备方案…".into());poll.restart();Ok(())
        })();
        if let Err(e)=result {
            ui.set_action_notice(match e {"offline"=>"离线预览不会打开网页或操作客户端。","route-unavailable"=>"当前路线样本不足，未导入装备方案。","changed"=>"当前英雄或选人状态已变化，请重新查看后操作。","guide"=>"无法打开攻略，请稍后重试。",_=>"操作未完成，请重新加载英雄详情。"}.into());
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
                    Ok(()) => {
                        "客户端装备方案已保存并回读核对。进入游戏后在商店装备方案中选择 FUZE 路线。"
                    }
                    Err("item-sets-selection-changed") => "选人已变化，未提交装备方案。",
                    Err("item-sets-account-changed") => "账号已变化，未提交装备方案。",
                    Err("item-sets-readback-failed" | "item-sets-readback-mismatch") => {
                        "写入请求已提交，但保存结果未核实。请重试导入，暂不视为成功。"
                    }
                    Err("item-sets-read-failed") => "无法读取客户端原装备方案，未提交修改。",
                    Err("item-sets-auto-disabled") => "自动导入已关闭，未提交装备方案。",
                    Err(_) => "装备方案配置未成功，请确认客户端连接后重试。",
                }
                .into(),
            );
        },
    );
    timer.stop();
    Binding { state, timer }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_import_debounces_context_and_never_retries_a_failed_attempt_in_a_loop() {
        let mut state = State::default();
        let now = std::time::Instant::now();
        let a = Some((115, "fixture-v1".into(), 0));
        let b = Some((115, "fixture-v1".into(), 1));
        assert!(!state.auto_ready(a.clone(), now));
        assert!(!state.auto_ready(a.clone(), now + Duration::from_millis(999)));
        assert!(!state.auto_ready(None, now + Duration::from_secs(1)));
        assert!(!state.auto_ready(a.clone(), now + Duration::from_secs(2)));
        assert!(state.auto_ready(a.clone(), now + Duration::from_secs(3)));
        assert!(!state.auto_ready(a, now + Duration::from_secs(60)));
        assert!(!state.auto_ready(b.clone(), now + Duration::from_secs(61)));
        assert!(state.auto_ready(b, now + Duration::from_secs(62)));
    }
}
