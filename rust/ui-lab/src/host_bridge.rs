//! Adapt existing host state, not a parallel implementation of capture, discovery or storage.
use crate::{HostState, MatchRow, PreferenceRow, Workbench};
use fuze_worker_host::{
    controller::Config,
    desktop::{frontend::Session, Status},
    preferences::Preferences,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, path::Path, rc::Rc, time::Duration};

pub struct Binding {
    pub session: Rc<Session>,
    _timer: slint::Timer,
}
pub fn offline_config(root: &Path) -> Config {
    Config {
        roots: vec![fuze_data::Root {
            path: root.into(),
            source: fuze_data::Source::Bundled,
        }],
        locale: Some(fuze_data::Locale::ZhCn),
        icon_downloads: false,
        level_gate: true,
        augment_overlay: false,
        worker: Default::default(),
        ort: Default::default(),
        model: Default::default(),
        dictionary: Default::default(),
        backend: "auto".into(),
    }
}
fn notice(ui: &Workbench, result: Result<(), &'static str>) {
    if let Err(e) = result {
        ui.global::<HostState>().set_notice(
            match e {
                "frontend-busy" => "当前操作尚未结束，请稍候。",
                "frontend-pause-required" => "请先暂停识别，再执行此操作。",
                "frontend-offline-operation" => "离线验证不会查询账号或下载数据。",
                "preferences-directory" | "preferences-directory-client-missing" => {
                    "目录无效；请填写包含 LeagueClient.exe 的本地目录，原设置已保留。"
                }
                _ => "操作未完成，原设置和缓存已保留。",
            }
            .into(),
        );
    }
}
fn rows(p: &Preferences) -> ModelRc<PreferenceRow> {
    let values = [
        (
            "automatic",
            "自动识别强化符文",
            "仅在对局阶段检测，空闲时停止截图。",
            p.automatic,
        ),
        (
            "level_gate",
            "按强化等级节省识别开销",
            "等级不可读时恢复基础检测，不据此推断领取轮次。",
            p.level_gate,
        ),
        (
            "augment_overlay",
            "顶部三卡推荐",
            "按实际卡位显示，暂不可读时保留位置和提示。",
            p.augment_overlay,
        ),
        (
            "augment_side_panel",
            "局内海克斯侧栏",
            "按需查看强化符文池，不自动操作游戏。",
            p.augment_side_panel,
        ),
        (
            "champ_select",
            "选人英雄详情",
            "五人阵容在顶部，可选英雄按胜率网格一览。",
            p.champ_select,
        ),
        (
            "champ_topmost",
            "选人阶段窗口置顶",
            "仅影响助手窗口。",
            p.champ_topmost,
        ),
        (
            "close_to_tray",
            "关闭后保留托盘",
            "从托盘可再次打开主窗口。",
            p.close_to_tray,
        ),
        (
            "icon_downloads",
            "补充缺失图标",
            "优先本地缓存，按需下载公开图片。",
            p.icon_downloads,
        ),
        (
            "pause_hotkey",
            "暂停快捷键",
            "Ctrl + Shift + F8；不向游戏发送按键。",
            p.pause_hotkey,
        ),
        (
            "local_records",
            "保存本地观察记录",
            "观察到的卡片不作为已领取，按账号隔离保存。",
            p.local_records,
        ),
        (
            "private_updates",
            "私有更新频道",
            "令牌由系统安全存储管理，不写入偏好文件。",
            p.private_updates,
        ),
    ];
    Rc::new(VecModel::from(
        values
            .into_iter()
            .map(|(key, title, hint, checked)| PreferenceRow {
                key: key.into(),
                title: title.into(),
                hint: hint.into(),
                checked,
            })
            .collect::<Vec<_>>(),
    ))
    .into()
}
fn change(p: &mut Preferences, key: &str, value: bool) -> Result<(), &'static str> {
    let target = match key {
        "level_gate" => &mut p.level_gate,
        "augment_overlay" => &mut p.augment_overlay,
        "augment_side_panel" => &mut p.augment_side_panel,
        "champ_select" => &mut p.champ_select,
        "champ_topmost" => &mut p.champ_topmost,
        "close_to_tray" => &mut p.close_to_tray,
        "icon_downloads" => &mut p.icon_downloads,
        "pause_hotkey" => &mut p.pause_hotkey,
        "local_records" => &mut p.local_records,
        "private_updates" => &mut p.private_updates,
        _ => return Err("preferences-key"),
    };
    *target = value;
    Ok(())
}
fn history(ui: &Workbench, view: &fuze_worker_host::account_history::View, page: usize) {
    let state = ui.global::<HostState>();
    let query = state.get_history_query().to_lowercase();
    let result = state.get_history_result();
    let filtered = view
        .rows
        .iter()
        .enumerate()
        .filter(|(i, r)| {
            let title = view
                .details
                .get(*i)
                .map(|d| d.title.to_lowercase())
                .unwrap_or_else(|| format!("英雄 #{}", r.champion));
            (query.is_empty() || title.contains(&query))
                && (result == 0
                    || ["", "win", "loss", "remake"].get(result as usize)
                        == Some(&r.result.as_str()))
        })
        .collect::<Vec<_>>();
    let pages = filtered.len().div_ceil(6).max(1);
    let page = page.min(pages - 1);
    state.set_history_page(page as i32);
    state.set_history_pages(pages as i32);
    state.set_history_selected(-1);
    state.set_history_note(if state.get_offline() {
        "隔离固定样例 · 非个人战绩".into()
    } else if view.error.is_some() {
        fuze_worker_host::account_history::error_label(view.error).into()
    } else if view.cached {
        "显示本地缓存，未据此确认当前登录账号。".into()
    } else {
        "已读取战绩；观察记录不等于实际领取。".into()
    });
    state.set_history(
        Rc::new(VecModel::from(
            filtered
                .into_iter()
                .skip(page * 6)
                .take(6)
                .map(|(i, r)| {
                    let d = view.details.get(i);
                    MatchRow {
                        title: d
                            .map(|d| d.title.clone())
                            .unwrap_or_else(|| format!("英雄 #{}", r.champion))
                            .into(),
                        result: if r.result == "win" {
                            "胜利"
                        } else if r.result == "loss" {
                            "失败"
                        } else if r.result == "remake" {
                            "重开"
                        } else {
                            "未确认"
                        }
                        .into(),
                        won: r.result == "win",
                        kda: format!("{} / {} / {}", r.kills, r.deaths, r.assists).into(),
                        duration: format!("{}分{}秒", r.duration / 60, r.duration % 60).into(),
                        detail: d
                            .map(|d| {
                                d.lines
                                    .iter()
                                    .map(|l| l.text.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_else(|| "详情数据暂不可用".into())
                            .into(),
                    }
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
}
pub fn attach(ui: &Workbench, config: Config, offline: bool) -> Result<Binding, &'static str> {
    let data_root = config.roots.first().map(|r| r.path.clone());
    let popup = Rc::new(RefCell::new(
        None::<(crate::ChampionWindow, crate::champion_bridge::Binding)>,
    ));
    let session = Rc::new(Session::start(config, offline)?);
    let state = ui.global::<HostState>();
    state.set_connected(true);
    state.set_offline(offline);
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_preference(move |key, value| {
        if let Some(ui) = weak.upgrade() {
            let result = if key == "automatic" {
                service.set_running(value)
            } else {
                let mut p = service.snapshot().preferences;
                change(&mut p, &key, value).and_then(|_| service.apply(p))
            };
            notice(&ui, result);
            ui.global::<HostState>().set_busy(service.busy());
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_backend_changed(move |index| {
        if let Some(ui) = weak.upgrade() {
            let mut p = service.snapshot().preferences;
            if let Some(backend) = ["auto", "cpu", "dml"].get(index as usize) {
                p.backend = (*backend).into();
                notice(&ui, service.apply(p));
                ui.global::<HostState>().set_busy(service.busy());
            }
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_directory_save(move |directory| {
        if let Some(ui) = weak.upgrade() {
            let mut p = service.snapshot().preferences;
            p.manual_directory = directory.trim().into();
            notice(&ui, service.apply(p));
            ui.global::<HostState>().set_busy(service.busy());
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    let popup_action = popup.clone();
    state.on_command(move |command| {
        if let Some(ui) = weak.upgrade() {
            if command == "preview-champion" {
                let opened = (|| -> Result<(), &'static str> {
                    if popup_action.borrow().is_none() {
                        let window = crate::ChampionWindow::new().map_err(|_| "champion-window")?;
                        let details = crate::champion_bridge::attach(
                            &window,
                            data_root.as_deref().ok_or("data-root")?,
                        )?;
                        let weak = window.as_weak();
                        window.on_close_requested(move || {
                            if let Some(w) = weak.upgrade() {
                                let _ = w.hide();
                            }
                        });
                        if let Some(view) = service.snapshot().selection.as_ref() {
                            details.selection(&window, Some(view));
                        }
                        *popup_action.borrow_mut() = Some((window, details));
                    }
                    popup_action
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .0
                        .show()
                        .map_err(|_| "champion-window")?;
                    Ok(())
                })();
                notice(&ui, opened);
                return;
            }
            let result = match command.as_str() {
                "toggle-running" => service.set_running(ui.get_paused()),
                "history-refresh" => service.refresh_history(false),
                "history-more" => service.refresh_history(true),
                "history-cached" => service.cached_history(),
                "data-update" => service.update_data(),
                _ => Err("frontend-command"),
            };
            notice(&ui, result);
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_history_turn(move |direction| {
        if let Some(ui) = weak.upgrade() {
            let page = (ui.global::<HostState>().get_history_page() + direction).max(0) as usize;
            history(&ui, &service.snapshot().history, page);
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_history_filter(move |query, result| {
        if let Some(ui) = weak.upgrade() {
            let state = ui.global::<HostState>();
            state.set_history_query(query);
            state.set_history_result(result);
            history(&ui, &service.snapshot().history, 0);
        }
    });
    let previous = RefCell::new(None::<Status>);
    let weak = ui.as_weak();
    let service = session.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(250),
        move || {
            let Some(ui) = weak.upgrade() else {
                service.stop();
                return;
            };
            let next = service.snapshot();
            let state = ui.global::<HostState>();
            let completed = state.get_busy() && !service.busy();
            state.set_busy(service.busy());
            if previous.borrow().as_ref() == Some(&next) && !completed {
                return;
            }
            let old = previous.borrow();
            if old.as_ref().is_none_or(|s|s.selection!=next.selection) {
                if let Some((window, details))=popup.borrow().as_ref(){details.selection(window,next.selection.as_ref());}
            }
            if old
                .as_ref()
                .is_none_or(|s| s.preferences != next.preferences || s.mode != next.mode || s.update_error != next.update_error || completed)
            {
                let mut prefs = next.preferences.clone();
                prefs.automatic = next.mode == 1;
                state.set_preferences(rows(&prefs));
                state.set_backend(
                    ["auto", "cpu", "dml"]
                        .iter()
                        .position(|s| *s == prefs.backend)
                        .unwrap_or(0) as i32,
                );
                state.set_locale(
                    ["auto", "zh-CN", "zh-TW", "en-US"]
                        .iter()
                        .position(|s| *s == prefs.locale)
                        .unwrap_or(0) as i32,
                );
                if old
                    .as_ref()
                    .is_none_or(|s| s.preferences.manual_directory != prefs.manual_directory)
                {
                    state.set_directory(prefs.manual_directory.into());
                }
            }
            if old.as_ref().is_none_or(|s| s.history != next.history) {
                history(&ui, &next.history, 0);
            }
            if old
                .as_ref()
                .is_none_or(|s| s.notice != next.notice || s.update_error != next.update_error)
            {
                state.set_notice(
                    if next.update_error.is_some() {
                        "操作未完成，原设置和缓存已保留。"
                    } else if offline && next.notice.is_some() {
                        "设置已在隔离会话中应用；未写入稳定版设置。"
                    } else {
                        next.notice.unwrap_or("")
                    }
                    .into(),
                );
            }
            ui.set_paused(next.mode == 0);
            state.set_running(next.mode == 1);
            state.set_data_version(next.available.as_deref().unwrap_or("—").rsplit('@').next().unwrap_or("—").into());
            if old.as_ref().is_none_or(|s| s.pipeline.recent != next.pipeline.recent || s.pipeline.recent_tiers != next.pipeline.recent_tiers) {
                state.set_slots(Rc::new(VecModel::from((0..3).map(|i| {
                    let (name, hint) = match next.pipeline.recent.as_ref().map(|slots| &slots[i]) {
                        Some(fuze_contract::Slot::Known { name, .. }) => (name.clone(), next.pipeline.recent_tiers[i].map(|v| format!("梯队 T{v} · 整体胜率参考")).unwrap_or_else(|| "暂无评级 · 整体胜率参考".into())),
                        Some(fuze_contract::Slot::Unreadable) => ("此位置暂不可读".into(), "可能遮挡或正在刷新，保留卡位".into()),
                        None => ("等待海克斯出现".into(), "未进入强化选择时，这里为空是正常现象。".into()),
                    }; crate::SlotRow { name: name.into(), hint: hint.into() }
                }).collect::<Vec<_>>())).into());
            }
            state.set_resources(Rc::new(VecModel::from(vec![
                next.resources.cpu.map(|v| format!("{v:.1}%")).unwrap_or("—".into()).into(),
                next.resources.memory_mb.map(|v| format!("{v:.0} MB")).unwrap_or("—".into()).into(),
                next.pipeline.analyses.to_string().into(),
                next.pipeline.analysis_ms.map(|v| format!("{v:.0} ms")).unwrap_or("—".into()).into(),
            ])).into());
            state.set_sampled(next.resources.sampled.as_deref().unwrap_or("等待首次采样").into());
            state.set_schedule(format!("当前等级 {} · 跳过截图 {} 次\n等级不可读时恢复基础检测；等级不是实际领取轮次。",next.pipeline.level.map(|n|n.to_string()).unwrap_or("—".into()),next.pipeline.skipped_frames).into());
            let view=crate::status_view::present(&next,offline);
            state.set_connection_value(view.connection.into());
            state.set_connection_hint(view.phase.into());
            state.set_connection_tone(view.connection_tone);
            state.set_capture_value(view.capture.into());
            state.set_capture_hint(view.capture_hint.into());
            state.set_capture_tone(view.capture_tone);
            state.set_ocr_value(view.ocr.into());
            state.set_ocr_hint(view.ocr_hint.into());
            state.set_ocr_tone(view.ocr_tone);
            state.set_verified(if offline { "隔离验证，未连接游戏".into() } else { next.verified_time.as_deref().unwrap_or("尚未验证").into() });
            ui.set_health(view.health);
            state.set_status_title(view.title.into());
            state.set_status_hint(view.hint.into());
            ui.global::<crate::ShellState>().invoke_sync(next.preferences.pause_hotkey,next.mode==1);
            drop(old);
            *previous.borrow_mut() = Some(next);
        },
    );
    Ok(Binding {
        session,
        _timer: timer,
    })
}
