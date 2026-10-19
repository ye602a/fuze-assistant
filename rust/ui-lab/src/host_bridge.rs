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
    pub records: Rc<crate::record_bridge::Binding>,
    _timer: slint::Timer,
    feedback: Rc<RefCell<crate::feedback_bridge::Binding>>,
}
impl Drop for Binding {
    fn drop(&mut self) {
        self._timer.stop();
        self.feedback.borrow_mut().finish();
    }
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
                "reference-unavailable" => {
                    "当前没有已确认英雄的对局，局内资料暂不可用。请在进入对局后重试。"
                }
                "preview-data-unavailable" => "本地预览数据不完整，请暂停识别后更新推荐数据。",
                "preview-live-phase" => {
                    "正在显示真实选人或对局内容。请先暂停识别，再打开样例预览。"
                }
                "champion-selection-unavailable" => {
                    "当前没有已确认的选人信息，请进入选人阶段后查看。"
                }
                "frontend-busy" => "当前操作尚未结束，请稍候。",
                "frontend-pause-required" => "请先暂停识别，再执行此操作。",
                "history-import-pending" => "请先确认或取消当前导入预览，再读取战绩。",
                "history-page-limit" => "当前来源没有可继续加载的战绩；可刷新重新查询。",
                "overlay-position-memory" => {
                    "三卡浮窗位置未能保存，当前显示不受影响；下次移动时会重试。"
                }
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
/// A queued operation is not a successful operation. Completion comes from the host poll.
pub(crate) fn submitted(ui: &Workbench, result: Result<(), &'static str>, action: &str) {
    if result.is_ok() {
        let state = ui.global::<HostState>();
        state.set_pending_action(action.into());
        state.set_busy(true);
        state.set_notice(
            match action {
                "pause" => "正在暂停识别…",
                "resume" => "正在恢复自动识别…",
                "directory" => "正在校验并保存目录；连接结果仍以客户端状态为准…",
                "data-update" => "正在更新推荐数据与字典；旧数据继续保留…",
                "history" => "正在读取战绩，请稍候…",
                _ => "正在保存并应用设置…",
            }
            .into(),
        );
    } else {
        notice(ui, result);
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
            "最小化时收入托盘",
            "点击 X 始终询问；关闭此项后最小化到任务栏。",
            p.close_to_tray,
        ),
        (
            "icon_downloads",
            "补充缺失图标",
            "优先本地缓存，按需下载公开图片。",
            p.icon_downloads,
        ),
        (
            "auto_item_sets",
            "自动导入装备方案（本次运行）",
            "开启即允许选人时导入本人所选路线，重启后关闭；保留个人方案。",
            p.auto_item_sets,
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
        "auto_item_sets" => &mut p.auto_item_sets,
        "pause_hotkey" => &mut p.pause_hotkey,
        "local_records" => &mut p.local_records,
        "private_updates" => &mut p.private_updates,
        _ => return Err("preferences-key"),
    };
    *target = value;
    Ok(())
}
const HISTORY_PAGE_SIZE: usize = 10;

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
    let pages = filtered.len().div_ceil(HISTORY_PAGE_SIZE).max(1);
    let page = page.min(pages - 1);
    state.set_history_page(page as i32);
    state.set_history_pages(pages as i32);
    state.set_history_selected(-1);
    state.set_history_can_more(
        view.next_start.is_some() && !view.account.is_empty() && view.pending == 0,
    );
    state.set_history_note(if state.get_offline() {
        "隔离固定样例 · 非个人战绩".into()
    } else if view.error.is_some() {
        let error = fuze_worker_host::account_history::error_label(view.error);
        if view.cached && !view.rows.is_empty() {
            format!("{error}。保留列表仅作本地参考，不代表当前账号已验证。").into()
        } else {
            error.into()
        }
    } else if view.pending > 0 {
        format!(
            "导入预览 · {} 项 · 尚未写入，请选择后确认或取消。",
            view.pending
        )
        .into()
    } else if view.cached {
        "显示本地缓存，未据此确认当前登录账号。".into()
    } else {
        let source = match view.source.as_str() {
            "sgp" => "官方战绩接口",
            "lcu" => "本地客户端接口",
            _ => "已验证来源",
        };
        format!("已读取战绩 · {source}；观察记录不等于实际领取。").into()
    });
    state.set_history(
        Rc::new(VecModel::from(
            filtered
                .into_iter()
                .skip(page * HISTORY_PAGE_SIZE)
                .take(HISTORY_PAGE_SIZE)
                .map(|(i, r)| {
                    let d = view.details.get(i);
                    MatchRow {
                        source_index: i as i32,
                        date: fuze_worker_host::history_detail::date(r.created).into(),
                        icon: Default::default(),
                        items: Default::default(),
                        augments: Default::default(),
                        chosen: false,
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
    ui.global::<crate::RecordState>().invoke_refresh();
}
pub fn attach(ui: &Workbench, config: Config, offline: bool) -> Result<Binding, &'static str> {
    attach_recovered(ui, config, offline, false)
}
pub fn attach_recovered(
    ui: &Workbench,
    config: Config,
    offline: bool,
    start_paused: bool,
) -> Result<Binding, &'static str> {
    let popup = Rc::new(RefCell::new(crate::game_windows::Manager::new(
        config.roots.clone(),
        offline,
        !offline,
    )));
    let unbound = crate::unbound_bridge::attach(ui, config.roots.clone());
    let session = Rc::new(Session::start_with_recovery(config, offline, start_paused)?);
    let diagnostics = crate::diagnostic_bridge::attach(ui, session.clone());
    let feedback =
        crate::feedback_bridge::attach(ui, session.clone(), diagnostics.clone(), offline);
    let feedback_lifetime = feedback.clone();
    let records = Rc::new(crate::record_bridge::attach(ui, session.clone(), offline)?);
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
            submitted(
                &ui,
                result,
                if key == "automatic" {
                    if value {
                        "resume"
                    } else {
                        "pause"
                    }
                } else {
                    "settings"
                },
            );
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_backend_changed(move |index| {
        if let Some(ui) = weak.upgrade() {
            let mut p = service.snapshot().preferences;
            if let Some(backend) = ["auto", "cpu", "dml"].get(index as usize) {
                p.backend = (*backend).into();
                submitted(&ui, service.apply(p), "settings");
            }
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_directory_save(move |directory| {
        if let Some(ui) = weak.upgrade() {
            let mut p = service.snapshot().preferences;
            p.manual_directory = directory.trim().into();
            submitted(&ui, service.apply(p), "directory");
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    let popup_action = popup.clone();
    state.on_command(move |command| {
        if let Some(ui) = weak.upgrade() {
            if matches!(command.as_str(), "preview-champion" | "preview-cards") {
                let status = service.snapshot();
                if status.mode == 1
                    && matches!(
                        status.connection,
                        Some(fuze_lcu::reconnect::State::Connected(
                            fuze_runtime::session::Phase::ChampSelect
                                | fuze_runtime::session::Phase::InProgress
                        ))
                    )
                {
                    notice(&ui, Err("preview-live-phase"));
                    return;
                }
            }
            if command == "preview-champion" {
                let opened = popup_action
                    .borrow_mut()
                    .preview(service.snapshot().selection.as_ref());
                if opened.is_ok() {
                    ui.global::<HostState>()
                        .set_notice("已打开只读选人预览，内容不是本局识别结果。".into());
                }
                notice(&ui, opened);
                return;
            }
            if command == "show-reference" {
                let result = popup_action.borrow_mut().reference(&service.snapshot());
                if result.is_ok() {
                    ui.global::<HostState>()
                        .set_notice("已打开本局英雄资料；关闭后可在这里重新打开。".into());
                }
                notice(&ui, result);
                return;
            }
            if command == "preview-cards" {
                let opened = popup_action.borrow_mut().preview_cards();
                if opened.is_ok() {
                    ui.global::<HostState>()
                        .set_notice("已打开三卡公开样例，内容不是本局识别结果。".into());
                }
                notice(&ui, opened);
                return;
            }
            if command == "show-status" {
                ui.set_page(0);
                ui.set_scroll_y(0.);
                return;
            }
            if matches!(
                command.as_str(),
                "history-refresh" | "history-more" | "history-cached"
            ) {
                let result = match command.as_str() {
                    "history-refresh" => service.refresh_history(false),
                    "history-more" => service.refresh_history(true),
                    _ => service.cached_history(),
                };
                if result.is_ok() {
                    let state = ui.global::<HostState>();
                    state.set_history_busy(true);
                    state.set_history_note("正在后台读取战绩；不改变识别开关，原列表保留。".into());
                    state.set_notice("正在后台读取战绩…".into());
                }
                notice(&ui, result);
                return;
            }
            let result = match command.as_str() {
                "toggle-running" => service.set_running(ui.get_paused()),
                "data-update" => service.update_data(),
                _ => Err("frontend-command"),
            };
            submitted(
                &ui,
                result,
                match command.as_str() {
                    "toggle-running" => {
                        if ui.get_paused() {
                            "resume"
                        } else {
                            "pause"
                        }
                    }
                    "data-update" => "data-update",
                    _ => "history",
                },
            );
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_history_turn(move |direction| {
        if let Some(ui) = weak.upgrade() {
            if ui.global::<crate::RecordState>().get_busy() {
                return;
            }
            let page = (ui.global::<HostState>().get_history_page() + direction).max(0) as usize;
            history(&ui, &service.snapshot().history, page);
        }
    });
    let weak = ui.as_weak();
    let service = session.clone();
    state.on_history_filter(move |query, result| {
        if let Some(ui) = weak.upgrade() {
            if ui.global::<crate::RecordState>().get_busy() {
                return;
            }
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
            // Read completion first: a completed request has published its result before this snapshot.
            let engine_busy = service.busy();
            let history_busy = service.history_busy();
            let next = service.snapshot();
            unbound.borrow_mut().update(&ui, &next, offline);
            diagnostics.borrow_mut().poll(&ui);
            feedback.borrow_mut().poll(&ui, &next, offline);
            if let Err(error) = popup.borrow_mut().placement() { notice(&ui, Err(error)); }
            let state = ui.global::<HostState>();
            let history_completed = state.get_history_busy() && !history_busy;
            state.set_history_busy(history_busy);
            let action = state.get_pending_action();
            let operation_completed = !action.is_empty() && !engine_busy && match action.as_str() {
                "pause" => next.mode == 0,
                "resume" => next.mode == 1,
                _ => true,
            };
            let completed = state.get_busy() && !engine_busy;
            state.set_busy(engine_busy || (!action.is_empty() && !operation_completed));
            if previous.borrow().as_ref() == Some(&next) && !completed && !history_completed {
                return;
            }
            let old = previous.borrow();
            if let Err(error)=popup.borrow_mut().update(&next){notice(&ui,Err(error));}
            if old
                .as_ref()
                .is_none_or(|s| s.preferences != next.preferences || s.mode != next.mode || s.update_error != next.update_error || completed)
            {
                let mut prefs = next.preferences.clone();
                prefs.automatic = next.mode == 1;
                state.set_preferences(rows(&prefs));
                let maintenance = ui.global::<crate::MaintenanceState>();
                if maintenance.get_private_channel() != prefs.private_updates {
                    maintenance.set_private_channel(prefs.private_updates);
                    maintenance.set_installable(false);
                    maintenance.set_update_status("频道已切换，请重新检查更新。".into());
                }
                state.set_backend(
                    ["auto", "cpu", "dml"]
                        .iter()
                        .position(|s| *s == prefs.backend)
                        .unwrap_or(0) as i32,
                );
                if old
                    .as_ref()
                    .is_none_or(|s| s.preferences.manual_directory != prefs.manual_directory)
                {
                    state.set_directory(prefs.manual_directory.into());
                }
            }
            if old.as_ref().is_none_or(|s| s.history != next.history) || history_completed || (operation_completed && action == "history") {
                history(&ui, &next.history, 0);
            }
            if history_completed && action.is_empty() { state.set_notice(state.get_history_note()); }
            if operation_completed {
                state.set_pending_action("".into());
                let message = match action.as_str() {
                    "pause" | "resume" if !offline && next.preferences.automatic != (action == "resume") => "识别状态已切换，但未能保存，下次启动可能恢复旧状态。",
                    "pause" => "识别已暂停，现在可以更新、导入或管理存储。",
                    "resume" => "自动识别已恢复；正在等待客户端与对局状态。",
                    "history" => "",
                    _ if offline => "设置已在隔离会话中应用；未写入稳定版设置。",
                    _ => next.notice.unwrap_or("操作已结束，请查看当前模块状态。"),
                };
                state.set_notice(if action == "history" { state.get_history_note() } else { message.into() });
            } else if old
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
                next.resources.memory_mb.map(|v| format!("{v:.0} MiB")).unwrap_or("—".into()).into(),
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
        records,
        _timer: timer,
        feedback: feedback_lifetime,
    })
}
