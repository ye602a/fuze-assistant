//! Hit-test every player-facing control in hidden real components. No host/session
//! is attached: external effects are recorded at the callback boundary, not executed.
//! This report proves event routing, not network/installer/game success.
use crate::{
    ChampionWindow, DiagnosticState, HostState, MaintenanceState, RecordState, ShellState,
    Workbench,
};
use i_slint_core::{
    accessibility::AccessibleStringProperty as Prop, items::ItemRc, window::WindowInner,
};
use slint::{
    platform::{PointerEventButton, WindowEvent},
    ComponentHandle, ModelRc, VecModel,
};
use std::{cell::RefCell, path::Path, rc::Rc};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Events = Rc<RefCell<Vec<String>>>;

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(v)).into()
}
pub(crate) trait Surface: ComponentHandle {
    fn bottom(&self) -> f32;
    fn offset(&self) -> f32;
    fn scroll(&self, y: f32);
}
impl Surface for Workbench {
    fn bottom(&self) -> f32 {
        self.get_content_bottom()
    }
    fn offset(&self) -> f32 {
        self.get_scroll_y()
    }
    fn scroll(&self, y: f32) {
        self.set_scroll_y(y);
    }
}
impl Surface for ChampionWindow {
    fn bottom(&self) -> f32 {
        self.get_content_bottom()
    }
    fn offset(&self) -> f32 {
        self.get_scroll_y()
    }
    fn scroll(&self, y: f32) {
        self.set_scroll_y(y);
    }
}
fn descendants(item: ItemRc, out: &mut Vec<ItemRc>) {
    let mut child = item.first_child();
    while let Some(node) = child {
        child = node.next_sibling();
        descendants(node.clone(), out);
        if node.is_accessible() {
            out.push(node);
        }
    }
}
fn items(ui: &impl ComponentHandle) -> Vec<ItemRc> {
    let mut out = Vec::new();
    descendants(
        ItemRc::new(WindowInner::from_pub(ui.window()).component(), 0),
        &mut out,
    );
    out
}
fn point(item: &ItemRc) -> (f32, f32) {
    let r = item.geometry();
    let mut center = r.center();
    if item
        .accessible_string_property(Prop::Label)
        .is_some_and(|s| s == "数据来源与版本" || s.starts_with("等级调度"))
    {
        center.y = r.origin.y + 23.;
    }
    let p = item.map_to_window(center);
    (p.x, p.y)
}
pub(crate) fn hit(ui: &impl Surface, label: &str) -> Result<()> {
    let (x, y) = target(ui, label)?;
    click_at(ui.window(), x, y);
    Ok(())
}
fn key(ui: &impl ComponentHandle, text: slint::SharedString) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
pub(crate) fn type_text(ui: &impl Surface, label: &str, text: &str) -> Result<()> {
    hit(ui, label)?;
    for ch in text.chars() {
        key(ui, ch.to_string().into());
    }
    Ok(())
}
pub(crate) fn select_next(ui: &impl Surface, label: &str) -> Result<()> {
    target(ui, label)?;
    let item = items(ui)
        .into_iter()
        .find(|i| {
            matches!(
                i.accessible_role(),
                i_slint_core::items::AccessibleRole::Combobox
            ) && i
                .accessible_string_property(Prop::Label)
                .is_some_and(|v| v == label)
        })
        .ok_or("combobox-not-found")?;
    // Focus only this component. Never open a native popup or change OS focus.
    WindowInner::from_pub(ui.window()).set_focus_item(
        &item,
        true,
        i_slint_core::input::FocusReason::Programmatic,
    );
    key(ui, slint::platform::Key::DownArrow.into());
    Ok(())
}
fn click_at(window: &slint::Window, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
fn target(ui: &impl Surface, label: &str) -> Result<(f32, f32)> {
    for _ in 0..4 {
        i_slint_core::properties::ChangeTracker::run_change_handlers();
        let _ = ui.window().take_snapshot()?;
        let h = ui
            .window()
            .size()
            .to_logical(ui.window().scale_factor())
            .height;
        let item = items(ui)
            .into_iter()
            .find(|i| {
                let y = point(i).1;
                !matches!(
                    i.accessible_role(),
                    i_slint_core::items::AccessibleRole::Text
                ) && i
                    .accessible_string_property(Prop::Label)
                    .is_some_and(|v| v == label)
                    && (i.is_visible() || y < 42. || y > ui.bottom() - 6.)
            })
            .ok_or_else(|| format!("control-not-found:{label}"))?;
        let (x, y) = point(&item);
        let fixed = matches!(label, "最小化窗口" | "关闭窗口" | "关闭英雄详情")
            || label.contains("套装备方案")
            || matches!(label, "关闭操作反馈" | "局内资料 · 只读参考");
        if (45. ..ui.bottom() - 6.).contains(&y) || (fixed && y >= 0. && y < h) {
            if !item.is_visible() {
                return Err(format!("control-clipped:{label}").into());
            }
            // Flickable may clamp a programmatic offset when input arrives. Flush
            // that transition before measuring, as a real user moving to it would.
            ui.window().dispatch_event(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(x, y),
            });
            let _ = ui.window().take_snapshot()?;
            let fresh = point(&item);
            if (fresh.1 - y).abs() > 1. {
                continue;
            }
            return Ok(fresh);
        }
        // Reveal using the page's real scroll position; the activation itself always
        // goes through WindowEvent hit testing, never invoke_* or accessibility_action.
        ui.scroll((ui.offset() + h * 0.45 - y).min(0.));
    }
    Err(format!("control-not-reachable:{label}").into())
}
#[derive(Default)]
struct Audit {
    rows: Vec<serde_json::Value>,
    events: Events,
    output: std::path::PathBuf,
}
impl Audit {
    fn check(&mut self, name: &str, ok: bool, detail: impl ToString) {
        self.rows
            .push(serde_json::json!({"control":name,"passed":ok,"detail":detail.to_string()}));
    }
    fn click(&mut self, ui: &impl Surface, label: &str, expected: Option<&str>) -> bool {
        self.activate(ui, label, expected, true)
    }
    fn click_once(&mut self, ui: &impl Surface, label: &str, expected: Option<&str>) -> bool {
        self.activate(ui, label, expected, false)
    }
    fn activate(
        &mut self,
        ui: &impl Surface,
        label: &str,
        expected: Option<&str>,
        keyboard: bool,
    ) -> bool {
        self.events.borrow_mut().clear();
        match target(ui, label) {
            Ok((x, y)) => {
                click_at(ui.window(), x, y);
                let events = self.events.borrow().clone();
                let ok = expected.is_none_or(|e| events == [e]);
                self.check(
                    label,
                    ok,
                    if let Some(e) = expected {
                        format!("expected={e}; observed={events:?}; at={x:.1},{y:.1}")
                    } else {
                        "component state checked separately".into()
                    },
                );
                if !ok && expected != Some("__must_not_dispatch__") {
                    if let Ok(p) = ui.window().take_snapshot() {
                        let _ = image::save_buffer(
                            self.output.join(format!("failure-{}.png", self.rows.len())),
                            p.as_bytes(),
                            p.width(),
                            p.height(),
                            image::ColorType::Rgba8,
                        );
                    }
                }
                if ok
                    && keyboard
                    && expected.is_some_and(|e| {
                        e != "__must_not_dispatch__" && e != "stage:false" && e != "stage-reset"
                    })
                {
                    self.events.borrow_mut().clear();
                    key(ui, " ".into());
                    let observed = self.events.borrow().clone();
                    self.check(
                        &format!("{label} · 键盘"),
                        observed == [expected.unwrap()],
                        format!("observed={observed:?}"),
                    );
                }
                ok
            }
            Err(e) => {
                self.check(label, false, e);
                false
            }
        }
    }
    fn disabled(&mut self, ui: &impl Surface, label: &str) {
        self.click(ui, label, Some("__must_not_dispatch__"));
        if let Some(row) = self.rows.last_mut() {
            if !row["detail"].as_str().unwrap_or("").contains("control-") {
                row["passed"] = self.events.borrow().is_empty().into();
                row["detail"] = "disabled control dispatches no action".into();
            }
        }
    }
}
fn bind(ui: &Workbench, events: &Events) {
    let s = ui.global::<HostState>();
    let e = events.clone();
    s.on_command(move |v| e.borrow_mut().push(format!("host:{v}")));
    let e = events.clone();
    s.on_preference(move |k, v| e.borrow_mut().push(format!("pref:{k}:{v}")));
    let e = events.clone();
    s.on_backend_changed(move |v| e.borrow_mut().push(format!("backend:{v}")));
    let e = events.clone();
    s.on_directory_save(move |v| e.borrow_mut().push(format!("directory:{v}")));
    let e = events.clone();
    s.on_history_turn(move |v| e.borrow_mut().push(format!("page:{v}")));
    let e = events.clone();
    s.on_history_filter(move |q, v| e.borrow_mut().push(format!("filter:{q}:{v}")));
    let e = events.clone();
    ui.global::<MaintenanceState>()
        .on_action(move |v| e.borrow_mut().push(format!("maintenance:{v}")));
    let e = events.clone();
    ui.global::<DiagnosticState>()
        .on_action(move |v| e.borrow_mut().push(format!("diagnostic:{v}")));
    let e = events.clone();
    ui.global::<DiagnosticState>()
        .on_upload_action(move |v| e.borrow_mut().push(format!("upload:{v}")));
    let e = events.clone();
    ui.global::<RecordState>()
        .on_action(move |v, i| e.borrow_mut().push(format!("record:{v}:{i}")));
    let e = events.clone();
    ui.global::<RecordState>()
        .on_unbound_action(move |v, i| e.borrow_mut().push(format!("unbound:{v}:{i}")));
    let e = events.clone();
    ui.on_minimize(move || e.borrow_mut().push("shell:minimize".into()));
    let e = events.clone();
    ui.on_request_close(move || e.borrow_mut().push("shell:close".into()));
    let e = events.clone();
    ui.on_close_requested(move || e.borrow_mut().push("shell:quit".into()));
    let e = events.clone();
    ui.on_drag_window(move || e.borrow_mut().push("shell:drag".into()));
}
fn workbench(a: &mut Audit, ui: &Workbench) -> Result<()> {
    bind(ui, &a.events);
    let s = ui.global::<HostState>();
    let m = ui.global::<MaintenanceState>();
    let r = ui.global::<RecordState>();
    let d = ui.global::<DiagnosticState>();
    s.set_connected(true);
    s.set_offline(false);
    s.set_running(false);
    ui.set_paused(true);
    m.set_attached(true);
    m.set_installable(true);
    m.set_can_clean(true);
    let prefs = [
        ("automatic", "自动识别强化符文"),
        ("level_gate", "按强化等级节省识别开销"),
        ("augment_overlay", "顶部三卡推荐"),
        ("augment_side_panel", "局内海克斯侧栏"),
        ("champ_select", "选人英雄详情"),
        ("champ_topmost", "选人阶段窗口置顶"),
        ("close_to_tray", "最小化时收入托盘"),
        ("icon_downloads", "补充缺失图标"),
        ("auto_item_sets", "自动导入装备方案（本次运行）"),
        ("pause_hotkey", "暂停快捷键"),
        ("local_records", "保存本地观察记录"),
    ];
    s.set_preferences(model(
        prefs
            .iter()
            .map(|(k, t)| crate::PreferenceRow {
                key: (*k).into(),
                title: (*t).into(),
                checked: true,
                hint: "隔离检查".into(),
            })
            .collect(),
    ));
    for (label, page) in [("战绩查询", 1), ("偏好设置", 2), ("对局助手", 0)] {
        a.click(ui, label, None);
        a.check(label, ui.get_page() == page, "page changed via hit test");
    }
    a.click(ui, "最小化窗口", Some("shell:minimize"));
    a.click(ui, "关闭窗口", Some("shell:close"));
    let _ = ui.window().take_snapshot()?;
    click_at(ui.window(), 300., 20.);
    let dragged = a.events.borrow().last().is_some_and(|v| v == "shell:drag");
    a.check("主窗口拖动入口", dragged, "callback only; no OS drag");
    for (label, value) in [("继续识别", true), ("暂停识别", false)] {
        ui.set_paused(value);
        a.click(ui, label, Some("host:toggle-running"));
    }
    let content_bottom = ui.get_content_bottom();
    s.set_notice("测试提示".into());
    i_slint_core::properties::ChangeTracker::run_change_handlers();
    let _ = ui.window().take_snapshot()?;
    a.check(
        "反馈不挤动内容区",
        ui.get_content_bottom() == content_bottom,
        "reserved footer geometry",
    );
    a.click(ui, "关闭操作反馈", None);
    a.check("关闭提示结果", s.get_notice().is_empty(), "notice cleared");
    ui.global::<ShellState>().set_notice("测试提示".into());
    a.click(ui, "关闭操作反馈", None);
    a.check(
        "关闭窗口提示结果",
        ui.global::<ShellState>().get_notice().is_empty(),
        "notice cleared",
    );
    r.set_notice("测试提示".into());
    a.click(ui, "关闭操作反馈", None);
    a.check(
        "关闭记录提示结果",
        r.get_notice().is_empty(),
        "notice cleared",
    );
    // Feedback remains in the viewport even when the triggering control is far down a page.
    ui.set_page(2);
    ui.set_scroll_y(-2000.);
    m.set_notice("扫描完成，目前没有符合安全清理条件的文件。".into());
    i_slint_core::properties::ChangeTracker::run_change_handlers();
    let _ = ui.window().take_snapshot()?;
    a.check(
        "滚动后维护反馈",
        ui.get_feedback().contains("扫描完成"),
        "fixed feedback, no scroll-to-top",
    );
    a.click(ui, "关闭操作反馈", None);
    a.check(
        "关闭维护反馈",
        m.get_notice().is_empty() && ui.get_feedback().is_empty(),
        "dismissed",
    );
    s.set_running(true);
    ui.set_page(1);
    a.click(ui, "刷新战绩", Some("host:history-refresh"));
    a.click(ui, "本地缓存", Some("host:history-cached"));
    a.disabled(ui, "更多战绩");
    s.set_history_busy(true);
    a.disabled(ui, "正在读取…");
    a.disabled(ui, "本地缓存");
    a.click(ui, "暂停并管理", Some("host:toggle-running"));
    s.set_history_busy(false);
    s.set_running(false);
    ui.set_page(0);
    ui.set_scroll_y(0.);
    a.check(
        "运行详情默认收起",
        !ui.get_runtime_open(),
        "normal overview stays compact",
    );
    a.click(ui, "查看运行详情", None);
    a.check(
        "展开运行详情",
        ui.get_runtime_open(),
        "details revealed via hit test",
    );
    for label in ["等级调度 · 等待对局", "数据来源与版本"] {
        a.click(ui, label, None);
        a.check(
            label,
            if label.starts_with("等级") {
                ui.get_level_open()
            } else {
                ui.get_sources_open()
            },
            "expanded",
        );
        a.click(ui, label, None);
        a.check(
            label,
            if label.starts_with("等级") {
                !ui.get_level_open()
            } else {
                !ui.get_sources_open()
            },
            "collapsed",
        );
    }
    a.click(ui, "收起运行详情", None);
    a.check(
        "收起运行详情结果",
        !ui.get_runtime_open(),
        "details collapsed",
    );
    ui.set_page(2);
    ui.set_scroll_y(0.);
    for (label, section) in [("数据与更新", 1), ("帮助与反馈", 2), ("常用设置", 0)] {
        a.click(ui, label, None);
        a.check(
            label,
            ui.get_settings_section() == section,
            "settings section changed via hit test",
        );
        for removed in [
            "查看运行状态",
            "预览选人详情",
            "预览三卡推荐",
            "检查一次",
            "标记操作前",
            "清除对比",
            "领取验证场景",
            "查看字段详情",
            "移除旧更新令牌",
            "界面语言",
        ] {
            a.check(
                &format!("普通设置移除 {removed}"),
                !items(ui).iter().any(|i| {
                    i.accessible_string_property(Prop::Label)
                        .is_some_and(|v| v == removed)
                }),
                "development or obsolete control absent",
            );
        }
    }
    a.click(ui, "数据与更新", None);
    for (label, command) in [
        ("检查新版", "check"),
        ("退出并打开更新程序", "install"),
        ("扫描本地占用", "scan"),
        ("清理可回收文件", "clean"),
    ] {
        a.click(ui, label, Some(&format!("maintenance:{command}")));
    }
    a.check(
        "公开更新不要求私有授权",
        target(ui, "设置私有授权").is_err() && target(ui, "使用私有更新频道").is_err(),
        "public channel has no credential entry or private toggle",
    );
    a.click(ui, "选择旧设置并预览", Some("record:import-settings:0"));
    a.click(ui, "更新推荐数据与字典", Some("host:data-update"));
    a.click(ui, "常用设置", None);
    for (key, title) in prefs {
        a.click(ui, title, Some(&format!("pref:{key}:false")));
    }
    for index in 1..=2 {
        a.events.borrow_mut().clear();
        select_next(ui, "识别后端")?;
        let ev = a.events.borrow().clone();
        a.check(
            "识别后端 · 下拉键盘选择",
            s.get_backend() == index && ev == [format!("backend:{index}")],
            format!("index={index}; {ev:?}"),
        );
    }
    a.click(ui, "高级回退：自动连接失败时手动指定目录", None);
    s.set_directory("".into());
    type_text(ui, "客户端目录", "fixture-client")?;
    a.check(
        "客户端目录输入",
        s.get_directory() == "fixture-client",
        "own component text events",
    );
    a.click(ui, "保存目录", Some("directory:fixture-client"));
    a.click(ui, "清除手动回退", Some("directory:"));
    a.click(ui, "高级回退：自动连接失败时手动指定目录", None);
    a.click(ui, "重新打开局内资料", Some("host:show-reference"));
    a.click(ui, "帮助与反馈", None);
    a.click(ui, "查看最近状态", None);
    a.check(
        "最近状态展开",
        ui.global::<crate::DiagnosticState>().get_open(),
        "local diagnostic disclosure",
    );
    a.click(ui, "收起最近状态", None);
    a.click(ui, "导出脱敏反馈", Some("diagnostic:export"));
    a.click(ui, "打开日志目录", Some("diagnostic:open-logs"));
    a.disabled(ui, "重试日志");
    ui.global::<crate::DiagnosticState>().set_log_retry(true);
    a.click(ui, "重试日志", Some("diagnostic:retry-log"));
    ui.global::<crate::DiagnosticState>().set_log_retry(false);
    a.check(
        "公开反馈默认收起",
        target(ui, "配置上传令牌").is_err(),
        "advanced feedback controls hidden",
    );
    a.click(ui, "展开公开诊断反馈", None);
    d.set_upload_ready(true);
    d.set_upload_has_report(true);
    for (label, action) in [
        ("每局自动上传到公开 Issues", "toggle"),
        ("获取上传令牌", "help"),
        ("配置上传令牌", "configure"),
        ("验证授权", "verify"),
        ("重试待传报告", "retry"),
        ("查看报告", "open"),
        ("删除上传令牌", "remove"),
    ] {
        a.click(ui, label, Some(&format!("upload:{action}")));
    }
    a.click(ui, "补传最近对局", None);
    a.check(
        "补传必须确认",
        d.get_upload_confirm() && a.events.borrow().is_empty(),
        "opening confirmation performs no upload",
    );
    a.click(ui, "取消", None);
    a.check(
        "取消补传不上传",
        !d.get_upload_confirm() && a.events.borrow().is_empty(),
        "cancel closes confirmation only",
    );
    a.click(ui, "补传最近对局", None);
    a.click_once(ui, "确认公开补传", Some("upload:recover"));
    a.check(
        "补传确认关闭",
        !d.get_upload_confirm(),
        "callback recorded only; no host or network attached",
    );
    d.set_upload_busy(true);
    for label in [
        "每局自动上传到公开 Issues",
        "获取上传令牌",
        "配置上传令牌",
        "正在验证…",
        "补传最近对局",
        "重试待传报告",
        "删除上传令牌",
    ] {
        a.disabled(ui, label);
    }
    d.set_upload_busy(false);
    s.set_running(true);
    a.disabled(ui, "补传最近对局");
    s.set_running(false);
    a.click(ui, "收起公开诊断反馈", None);
    a.click(ui, "数据与更新", None);
    for kind in ["clean", "install"] {
        m.set_confirmation(kind.into());
        a.click(ui, "取消", Some("maintenance:cancel"));
        a.click(ui, "确认", Some("maintenance:confirm"));
        a.events.borrow_mut().clear();
        key(ui, slint::platform::Key::Escape.into());
        let ev = a.events.borrow().clone();
        a.check("维护弹窗 Escape", ev == ["maintenance:cancel"], kind);
        let page = ui.get_page();
        click_at(ui.window(), 90., 210.);
        a.check("维护弹窗阻止穿透", ui.get_page() == page, kind);
        m.set_confirmation("".into());
    }
    s.set_running(true);
    a.click(ui, "帮助与反馈", None);
    a.disabled(ui, "导出脱敏反馈");
    a.click(ui, "数据与更新", None);
    a.click(ui, "检查新版", Some("maintenance:check"));
    a.disabled(ui, "扫描本地占用");
    a.disabled(ui, "更新推荐数据与字典");
    a.click(ui, "暂停并管理", Some("host:toggle-running"));
    s.set_running(false);
    for (mode, offline, busy) in [("服务忙", false, true), ("离线", true, false)] {
        s.set_offline(offline);
        s.set_busy(busy);
        for label in ["更新推荐数据与字典", "选择旧设置并预览"] {
            a.disabled(ui, label);
        }
        a.check(
            mode,
            true,
            "disabled state covered separately from enabled actions",
        );
    }
    s.set_offline(false);
    s.set_busy(false);
    ui.set_page(1);
    ui.set_scroll_y(0.);
    s.set_history(model(vec![crate::MatchRow {
        title: "测试英雄".into(),
        won: true,
        ..Default::default()
    }]));
    type_text(ui, "筛选英雄", "格温")?;
    a.check(
        "英雄筛选输入",
        s.get_history_query() == "格温",
        "own component Unicode events, not OS IME",
    );
    for index in 1..4 {
        a.events.borrow_mut().clear();
        select_next(ui, "战绩结果")?;
        let ev = a.events.borrow().clone();
        a.check(
            "战绩结果",
            s.get_history_result() == index && ev == [format!("filter:格温:{index}")],
            format!("index={index}"),
        );
    }
    for (label, cmd) in [
        ("刷新战绩", "history-refresh"),
        ("本地缓存", "history-cached"),
        ("更多战绩", "history-more"),
    ] {
        s.set_history_can_more(true);
        a.click(ui, label, Some(&format!("host:{cmd}")));
    }
    a.check(
        "战绩管理默认收起",
        !ui.get_history_tools_open(),
        "import and unbound observations hidden initially",
    );
    a.click(ui, "战绩管理", None);
    a.check(
        "展开战绩管理",
        ui.get_history_tools_open(),
        "management controls revealed via hit test",
    );
    a.click(ui, "选择旧战绩并预览", Some("record:import-history:0"));
    let history_fixture = s.get_history();
    s.set_history(Default::default());
    a.click(ui, "查看未绑定本地观察", Some("unbound:open:1"));
    r.set_unbound_open(true);
    r.set_unbound(model(vec![crate::ObservationRow {
        title: "测试观察".into(),
        ..Default::default()
    }]));
    a.click(ui, "查看完整事件", Some("unbound:select:0"));
    r.set_unbound_selected(0);
    r.set_unbound_pages(3);
    r.set_unbound_page(1);
    a.click(ui, "上一页", Some("unbound:page:-1"));
    a.click(ui, "下一页", Some("unbound:page:1"));
    a.click(ui, "收起完整事件", Some("unbound:select:0"));
    a.click(ui, "收起未绑定本地观察", Some("unbound:open:0"));
    r.set_unbound_open(false);
    a.click(ui, "收起战绩管理", None);
    a.check(
        "收起战绩管理结果",
        !ui.get_history_tools_open(),
        "management controls collapsed",
    );
    s.set_history(history_fixture);
    a.click(ui, "查看详情", Some("record:select:0"));
    s.set_history_selected(0);
    a.click(ui, "收起详情", Some("record:select:0"));
    a.click(ui, "生成匿名分享预览", Some("record:share:0"));
    a.click(ui, "导入本场旧观察记录", Some("record:import-journal:0"));
    r.set_observation_pages(3);
    r.set_observation_page(1);
    a.click(ui, "上一组", Some("record:observation-page:-1"));
    a.click(ui, "下一组", Some("record:observation-page:1"));
    r.set_share_ready(true);
    a.click(ui, "保存图片 / JSON", Some("record:save:0"));
    a.click(ui, "复制图片", Some("record:copy:0"));
    r.set_share_ready(false);
    s.set_history_selected(-1);
    s.set_history_pages(3);
    s.set_history_page(1);
    a.click(ui, "上一页", Some("page:-1"));
    a.click(ui, "下一页", Some("page:1"));
    r.set_import_count(1);
    r.set_chosen_count(1);
    a.click(ui, "选择导入", Some("record:choose:0"));
    a.click(ui, "取消导入", Some("record:cancel-import:0"));
    a.click(ui, "确认导入所选", Some("record:confirm-import:0"));
    r.set_import_count(0);
    for kind in ["settings", "journal", "history"] {
        r.set_confirmation(kind.into());
        a.click(ui, "取消", Some("record:cancel-confirm:0"));
        a.click(ui, "确认导入", Some("record:confirm:0"));
        a.events.borrow_mut().clear();
        key(ui, slint::platform::Key::Escape.into());
        let ev = a.events.borrow().clone();
        a.check("记录弹窗 Escape", ev == ["record:cancel-confirm:0"], kind);
        r.set_confirmation("".into());
    }
    ui.set_quit_open(true);
    a.click(ui, "取消", None);
    a.check("退出确认取消", !ui.get_quit_open(), "dialog closed");
    ui.set_quit_open(true);
    a.click(ui, "退出软件", Some("shell:quit"));
    ui.set_quit_open(false);
    ui.set_quit_open(true);
    let _ = ui.window().take_snapshot()?;
    key(ui, slint::platform::Key::Escape.into());
    a.check(
        "退出弹窗 Escape",
        !ui.get_quit_open(),
        "cancel without exit",
    );
    ui.set_page(2);
    ui.set_settings_section(0);
    ui.set_scroll_y(0.);
    // A real tab click settles its scroll reset before a later wheel event.
    i_slint_core::properties::ChangeTracker::run_change_handlers();
    let _ = ui.window().take_snapshot()?;
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position: slint::LogicalPosition::new(650., 350.),
        delta_x: 0.,
        delta_y: -300.,
    });
    let _ = ui.window().take_snapshot()?;
    a.check(
        "设置页滚轮",
        ui.get_scroll_y() < 0.,
        "own component scroll event",
    );
    ui.set_scroll_y(0.);
    let _ = ui.window().take_snapshot()?;
    let p = slint::LogicalPosition::new(
        ui.window()
            .size()
            .to_logical(ui.window().scale_factor())
            .width
            - 7.,
        120.,
    );
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position: p });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button: PointerEventButton::Left,
    });
    let moved = slint::LogicalPosition::new(p.x, p.y + 180.);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position: moved });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: moved,
        button: PointerEventButton::Left,
    });
    let _ = ui.window().take_snapshot()?;
    a.check(
        "设置页滚动条拖动",
        ui.get_scroll_y() < 0.,
        "own component drag, no OS cursor",
    );
    ui.set_page(2);
    ui.set_scroll_y(0.);
    let pix = ui.window().take_snapshot()?;
    image::save_buffer(
        a.output.join(format!("settings-{}.png", pix.width())),
        pix.as_bytes(),
        pix.width(),
        pix.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
fn champion(a: &mut Audit, ui: &ChampionWindow) -> Result<()> {
    let e = a.events.clone();
    ui.on_inspect(move |v| e.borrow_mut().push(format!("inspect:{v}")));
    let e = a.events.clone();
    ui.on_player_action(move |v, i| e.borrow_mut().push(format!("player:{v}:{i}")));
    let e = a.events.clone();
    ui.on_filter(move |q, v| e.borrow_mut().push(format!("augment-filter:{q}:{v}")));
    let e = a.events.clone();
    ui.on_stage_request(move |v| e.borrow_mut().push(format!("stage:{v}")));
    let e = a.events.clone();
    ui.on_stage_select(move |v| e.borrow_mut().push(format!("stage-select:{v}")));
    let e = a.events.clone();
    let weak = ui.as_weak();
    ui.on_stage_reset(move || {
        e.borrow_mut().push("stage-reset".into());
        if let Some(ui) = weak.upgrade() {
            ui.set_stage_open(false);
            ui.set_stage_busy(false);
            ui.set_stage_rows(Default::default());
            ui.set_stage_index(0);
        }
    });
    let e = a.events.clone();
    ui.on_route_changed(move |v| e.borrow_mut().push(format!("route:{v}")));
    let e = a.events.clone();
    ui.on_close_requested(move || e.borrow_mut().push("champion:close".into()));
    let e = a.events.clone();
    ui.on_retry_portraits(move || e.borrow_mut().push("champion:retry-portraits".into()));
    ui.set_offline(false);
    ui.set_item_count(4);
    ui.set_local_id(4);
    ui.set_inspected_id(4);
    ui.set_detail_champion(4);
    ui.set_guides(model(vec!["测试攻略一".into(), "测试攻略二".into()]));
    ui.set_portraits_missing(true);
    a.click(ui, "重试头像", Some("champion:retry-portraits"));
    ui.set_portraits_missing(false);
    a.check(
        "头像完整时隐藏重试",
        target(ui, "重试头像").is_err(),
        "retry action is only available for missing portraits",
    );
    ui.set_build_routes(model(vec![
        "出装路线 1".into(),
        "出装路线 2".into(),
        "出装路线 3".into(),
        "出装路线 4".into(),
    ]));
    ui.set_augments(model(vec![crate::DetailRow {
        title: "测试符文".into(),
        description: "仅供界面验证。长说明应能完整滚动阅读，且不会覆盖底部反馈与装备操作。"
            .repeat(24)
            .into(),
        ..Default::default()
    }]));
    use slint::Model;
    for hero in ui.get_team().iter() {
        a.click(
            ui,
            &format!("查看{}", hero.name),
            Some(&format!("inspect:{}", hero.id)),
        );
    }
    for hero in ui.get_bench().iter() {
        a.click(
            ui,
            &format!("{}，胜率 {}", hero.name, hero.win),
            Some(&format!("inspect:{}", hero.id)),
        );
    }
    a.click(ui, "出装与技能", None);
    a.check("出装页签", ui.get_detail_tab() == 1, "tab changed");
    for i in 0..4 {
        a.click(
            ui,
            &format!("出装路线 {}", i + 1),
            Some(&format!("route:{i}")),
        );
        a.check(
            &format!("当前出装路线 {}", i + 1),
            ui.get_build_route() == i,
            "route selection updates the import target",
        );
        a.click(
            ui,
            &format!("导入路线 {} 到游戏商店", i + 1),
            Some("player:prepare-items:0"),
        );
    }
    a.click(ui, "海克斯", None);
    a.check("海克斯页签", ui.get_detail_tab() == 0, "tab changed");
    type_text(ui, "搜索强化符文", "强化")?;
    a.check(
        "符文搜索输入",
        ui.get_augment_query() == "强化",
        "own component Unicode events",
    );
    ui.set_augment_query("".into());
    for (i, title) in ["全部", "白银", "黄金", "棱彩"].iter().enumerate() {
        a.click(
            ui,
            title,
            Some(&format!("augment-filter::{}", i as i32 - 1)),
        );
    }
    a.click(ui, "查看测试符文的说明", None);
    a.check(
        "展开符文说明",
        ui.get_expanded_augment() == 0,
        "description selected",
    );
    key(ui, " ".into());
    a.check(
        "符文说明键盘重复收起",
        ui.get_expanded_augment() == -1,
        "Space matches pointer toggle",
    );
    key(ui, "\n".into());
    a.check(
        "符文说明 Enter 展开",
        ui.get_expanded_augment() == 0,
        "focus remains on originating augment",
    );
    key(ui, slint::platform::Key::Escape.into());
    a.check(
        "符文说明 Escape 关闭",
        ui.get_expanded_augment() == -1,
        "Escape dismisses without changing hero",
    );
    key(ui, "\n".into());
    a.check(
        "符文说明关闭后焦点保留",
        ui.get_expanded_augment() == 0,
        "Enter reopens the same focused augment",
    );
    ui.set_action_notice(
        "装备配置失败，请稍后重试。长反馈保留在独立底栏，不遮住当前符文说明。"
            .repeat(4)
            .into(),
    );
    let pixels = ui.window().take_snapshot()?;
    a.check(
        "符文说明避让反馈区",
        ui.get_description_bottom() <= ui.get_feedback_top(),
        format!(
            "description-bottom={}; feedback-top={}",
            ui.get_description_bottom(),
            ui.get_feedback_top()
        ),
    );
    image::save_buffer(
        a.output.join(format!(
            "champion-description-feedback-{}.png",
            pixels.width()
        )),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )?;
    a.click(ui, "关闭符文说明", None);
    a.check(
        "收起符文说明",
        ui.get_expanded_augment() == -1,
        "description cleared",
    );
    ui.set_action_notice("".into());
    let augment_fixture = ui.get_augments();
    ui.set_augments(Default::default());
    a.click(ui, "棱彩", Some("augment-filter::2"));
    let pixels = ui.window().take_snapshot()?;
    let position = |label: &str| {
        items(ui)
            .iter()
            .find(|i| {
                i.accessible_string_property(Prop::Label)
                    .is_some_and(|v| v == label)
            })
            .map(point)
    };
    let compact_empty = position("棱彩")
        .zip(position("查看分阶段胜率参考"))
        .is_some_and(|(filter, next)| next.1 - filter.1 < 160. && next.1 > filter.1);
    a.check(
        "空符文结果不留空白列表",
        compact_empty,
        "next section stays directly after filters and empty state",
    );
    image::save_buffer(
        a.output
            .join(format!("champion-empty-{}.png", pixels.width())),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )?;
    ui.set_augments(augment_fixture);
    for i in 0..2 {
        a.click(
            ui,
            &format!(
                "在默认浏览器打开攻略：测试攻略{}",
                if i == 0 { "一" } else { "二" }
            ),
            Some(&format!("player:guide:{i}")),
        );
    }
    a.click(ui, "查看分阶段胜率参考", Some("stage:false"));
    for (i, label) in ["整体", "第 1 次", "第 2 次", "第 3 次", "第 4 次"]
        .iter()
        .enumerate()
    {
        a.click(ui, label, Some(&format!("stage-select:{i}")));
    }
    a.click(ui, "刷新公开统计", Some("stage:true"));
    a.click(ui, "收起分阶段胜率参考", Some("stage-reset"));
    a.check(
        "阶段参考收起",
        !ui.get_stage_open(),
        "cancel and clear route",
    );
    // This toggle changes its next action after closing; test keyboard close from
    // an open fixture instead of expecting a second activation to close it again.
    ui.set_stage_open(true);
    let _ = ui.window().take_snapshot()?;
    a.events.borrow_mut().clear();
    key(ui, " ".into());
    let closed = a.events.borrow().as_slice() == ["stage-reset"] && !ui.get_stage_open();
    a.check(
        "阶段参考键盘收起",
        closed,
        "keyboard cancel and clear route",
    );
    let import_label = "导入路线 4 到游戏商店";
    a.click(ui, import_label, Some("player:prepare-items:0"));
    ui.set_action_busy(true);
    a.disabled(ui, "正在配置装备方案…");
    ui.set_action_busy(false);
    ui.set_item_confirm(true);
    a.click(ui, "取消", Some("player:cancel:0"));
    a.click(ui, "确认配置", Some("player:confirm-items:0"));
    ui.set_item_confirm(false);
    ui.set_item_confirm(true);
    let _ = ui.window().take_snapshot()?;
    a.events.borrow_mut().clear();
    key(ui, slint::platform::Key::Escape.into());
    let ev = a.events.borrow().clone();
    a.check(
        "装备确认 Escape",
        ev == ["player:cancel:0"],
        "no client write",
    );
    ui.set_item_confirm(false);
    ui.set_side_mode(true);
    a.check(
        "局内资料不显示配置按钮",
        target(ui, import_label).is_err(),
        "read-only mode uses explanatory text",
    );
    ui.set_side_mode(false);
    ui.set_offline(true);
    a.check(
        "离线预览不显示配置按钮",
        target(ui, import_label).is_err(),
        "preview cannot configure equipment",
    );
    ui.set_offline(false);
    ui.set_inspected_id(8);
    a.check(
        "候选英雄不显示配置按钮",
        target(ui, import_label).is_err(),
        "only the player's selected champion may configure equipment",
    );
    ui.set_inspected_id(4);
    ui.set_item_count(0);
    a.check(
        "无装备方案时隐藏配置按钮",
        target(ui, import_label).is_err(),
        "missing data cannot dispatch equipment configuration",
    );
    ui.set_item_count(4);
    a.click(ui, import_label, Some("player:prepare-items:0"));
    a.click(ui, "关闭英雄详情", Some("champion:close"));
    Ok(())
}
pub fn run(output: &Path) -> Result<serde_json::Value> {
    std::fs::create_dir_all(output)?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(
            if cfg!(feature = "gpu") {
                "skia-software"
            } else {
                "software"
            }
            .into(),
        )
        .with_winit_window_attributes_hook(|a| a.with_visible(false).with_active(false))
        .select()?;
    let mut audit = Audit {
        output: output.into(),
        ..Default::default()
    };
    for (w, h, cw, ch) in [(1120., 860., 420., 640.), (920., 760., 380., 600.)] {
        let first = audit.rows.len();
        let ui = Workbench::new()?;
        ui.window().set_size(slint::LogicalSize::new(w, h));
        let champion_ui = ChampionWindow::new()?;
        champion_ui
            .window()
            .set_size(slint::LogicalSize::new(cw, ch));
        // Public fixtures only. Capture the overview, not a scrolled detail tab.
        use slint::Model;
        let hero = champion_ui
            .get_bench()
            .row_data(1)
            .ok_or("missing-public-fixture")?;
        champion_ui.set_inspected_id(hero.id);
        champion_ui.set_inspected_name(hero.name);
        champion_ui.set_inspected_icon(hero.icon);
        champion_ui.set_inspected_win(hero.win);
        champion_ui.set_inspected_pick(hero.pick);
        for (name, window) in [
            (format!("home-{w:.0}.png"), ui.window()),
            (format!("champion-{cw:.0}.png"), champion_ui.window()),
        ] {
            let p = window.take_snapshot()?;
            image::save_buffer(
                output.join(name),
                p.as_bytes(),
                p.width(),
                p.height(),
                image::ColorType::Rgba8,
            )?;
        }
        workbench(&mut audit, &ui)?;
        champion(&mut audit, &champion_ui)?;
        use slint::winit_030::WinitWindowAccessor;
        for window in [ui.window(), champion_ui.window()] {
            if window.with_winit_window(|w| w.is_visible()).flatten() == Some(true) {
                return Err("audit-window-visible".into());
            }
        }
        for row in &mut audit.rows[first..] {
            row["size"] = format!("main={w}x{h}; champion={cw}x{ch}").into();
        }
    }
    let failures = audit.rows.iter().filter(|r| r["passed"] != true).count();
    for row in audit.rows.iter().filter(|r| r["passed"] != true) {
        eprintln!("control-audit-failure: {row}");
    }
    let report = serde_json::json!({"checks":audit.rows.len(),"failures":failures,"system_input":false,"game_access":false,"external_effects_executed":false,"scope":"real component hit testing; external callback targets recorded only","rows":audit.rows});
    std::fs::write(
        output.join("controls.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if failures > 0 {
        return Err(format!("control-audit-failed:{failures}; see controls.json").into());
    }
    Ok(
        serde_json::json!({"checks":report["checks"],"failures":0,"system_input":false,"game_access":false}),
    )
}
