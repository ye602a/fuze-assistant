//! Explicit manual reference. Never sends stage statistics into the recommendation pipeline.
use crate::{ChampionWindow, StageRow};
use fuze_worker_host::stage_reference::{self, Access, Reply, Request, Service, Store};
use slint::{ComponentHandle, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub struct Binding {
    _service: Rc<RefCell<Option<Service>>>,
    _timer: Rc<slint::Timer>,
}
pub fn present(ui: &ChampionWindow, reply: &Reply) {
    let rows = reply
        .snapshot
        .as_ref()
        .filter(|s| s.usable(ui.get_inspected_id() as u32, stage_reference::now()));
    let values = rows
        .map(|s| {
            s.visible(ui.get_stage_index().clamp(0, 4) as usize)
                .into_iter()
                .map(|r| StageRow {
                    name: reply
                        .names
                        .get(&r.id)
                        .cloned()
                        .unwrap_or_else(|| format!("强化 #{}", r.id))
                        .into(),
                    win: format!("{:.1}%", r.win * 100.).into(),
                    pick: format!("{:.1}%", r.pick * 100.).into(),
                    samples: r.samples.to_string().into(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let empty = values.is_empty();
    ui.set_stage_rows(Rc::new(VecModel::from(values)).into());
    let message = if let Some(s) = rows {
        format!(
            "ARAMKit · {} · {}{}{}\n地区未确认{}\n{}",
            s.patch,
            s.date,
            if reply.cached { " · 缓存" } else { "" },
            if s.stale(stage_reference::now()) {
                " · 待刷新"
            } else {
                ""
            },
            if s.patch_matches(&ui.get_detail_version()) {
                ""
            } else {
                " · 与本地数据补丁不同"
            },
            if reply.issue.is_some() && reply.issue != Some("stage-loading") {
                "刷新失败，保留缓存；仅显示样本不少于 200 的统计。"
            } else if empty {
                "此阶段暂无足够样本；不会用整体统计冒充阶段胜率。"
            } else {
                "仅显示样本不少于 200 的统计；来源与三卡整体数据独立。"
            }
        )
    } else if reply.snapshot.is_some() {
        "阶段缓存已超过保留期限，请在选人阶段重新准备。".into()
    } else if reply.issue == Some("stage-cache-missing") {
        "暂无此英雄的阶段缓存；对局中不下载，请在选人阶段准备。".into()
    } else if reply.issue == Some("stage-offline") {
        "离线验证不查询网络，也不读取个人缓存。".into()
    } else if reply.issue.is_some() {
        "阶段统计暂不可用，可稍后手动重试。".into()
    } else {
        "正在读取公开阶段统计…".into()
    };
    ui.set_stage_status(message.into());
}
pub fn attach(
    ui: &ChampionWindow,
    roots: Vec<fuze_data::Root>,
    offline: bool,
    store: std::sync::Arc<Store>,
) -> Binding {
    let service = Rc::new(RefCell::new(None::<Service>));
    let timer = Rc::new(slint::Timer::default());
    let reply = Rc::new(RefCell::new(Reply::default()));
    let serial = Rc::new(Cell::new(0));
    let selected = Rc::new(Cell::new(0));
    let weak = ui.as_weak();
    let s = service.clone();
    let poll = timer.clone();
    let seen = serial.clone();
    let current = reply.clone();
    let hero = selected.clone();
    ui.on_stage_request(move |refresh| {
        let Some(ui) = weak.upgrade() else { return };
        let id = ui.get_inspected_id() as u32;
        if id == 0 || ui.get_stage_busy() {
            return;
        }
        if !refresh
            && hero.get() == id
            && current
                .borrow()
                .snapshot
                .as_ref()
                .is_some_and(|s| s.usable(id, stage_reference::now()))
        {
            present(&ui, &current.borrow());
            return;
        }
        if s.borrow().is_none() {
            match Service::with_store(roots.clone(), offline, store.clone()) {
                Ok(worker) => *s.borrow_mut() = Some(worker),
                Err(_) => {
                    ui.set_stage_status("阶段统计服务暂不可用。".into());
                    return;
                }
            }
        }
        if ui.get_detail_champion() != id as i32 {
            ui.set_stage_status("英雄资料尚未就绪，请稍后重试。".into());
            return;
        }
        hero.set(id);
        let locale = match ui.get_detail_version().split('@').next() {
            Some("zh-TW") => fuze_data::Locale::ZhTw,
            Some("en-US") => fuze_data::Locale::EnUs,
            _ => fuze_data::Locale::ZhCn,
        };
        let service = s.borrow();
        let service = service.as_ref().unwrap();
        service.request(Some(Request {
            id,
            locale,
            access: if ui.get_side_mode() {
                Access::CacheOnly
            } else if refresh {
                Access::Refresh
            } else {
                Access::Cached
            },
        }));
        if let Some(next) = service.changed(seen.get()) {
            seen.set(next.serial);
            *current.borrow_mut() = next;
        }
        let complete = current
            .borrow()
            .issue
            .is_some_and(|issue| issue != "stage-loading")
            || (current.borrow().snapshot.is_some() && current.borrow().issue.is_none());
        ui.set_stage_busy(!complete);
        present(&ui, &current.borrow());
        if complete {
            poll.stop();
        } else {
            poll.restart();
        }
    });
    let weak = ui.as_weak();
    let current = reply.clone();
    ui.on_stage_select(move |index| {
        if let Some(ui) = weak.upgrade() {
            ui.set_stage_index(index.clamp(0, 4));
            present(&ui, &current.borrow());
        }
    });
    let weak = ui.as_weak();
    let s = service.clone();
    let poll = timer.clone();
    let current = reply.clone();
    let hero = selected.clone();
    ui.on_stage_reset(move || {
        poll.stop();
        hero.set(0);
        *current.borrow_mut() = Reply::default();
        if let Some(s) = s.borrow().as_ref() {
            s.request(None);
        }
        if let Some(ui) = weak.upgrade() {
            ui.set_stage_busy(false);
            ui.set_stage_open(false);
            ui.set_stage_rows(Default::default());
            ui.set_stage_index(0);
            ui.set_stage_status("手动参考，不改变三卡推荐".into());
        }
    });
    let weak = ui.as_weak();
    let s = service.clone();
    let poll = Rc::downgrade(&timer);
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let next = s.borrow().as_ref().and_then(|s| s.changed(serial.get()));
            let Some(next) = next else { return };
            serial.set(next.serial);
            let complete = next.snapshot.is_some() || next.issue.is_some();
            *reply.borrow_mut() = next;
            present(&ui, &reply.borrow());
            if complete {
                ui.set_stage_busy(false);
                if let Some(p) = poll.upgrade() {
                    p.stop();
                }
            }
        },
    );
    timer.stop();
    Binding {
        _service: service,
        _timer: timer,
    }
}
