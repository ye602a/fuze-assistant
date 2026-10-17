//! Device-only observations. No identity lookup, personal-file read or claim inference here.
use crate::{ObservationRow, RecordState, Workbench};
use fuze_worker_host::{
    desktop::Status, history_detail, local_records::UnboundSnapshot, observation_names::Names,
};
use slint::{ComponentHandle, VecModel};
use std::{cell::RefCell, rc::Rc, thread::JoinHandle};

type NamesJob = JoinHandle<Option<Names>>;
pub struct Binding {
    roots: Vec<fuze_data::Root>,
    snapshot: UnboundSnapshot,
    version: Option<String>,
    names: Option<Names>,
    job: Option<(Option<String>, NamesJob)>,
    attempted: bool,
    open: bool,
}
impl Binding {
    pub fn update(&mut self, ui: &Workbench, status: &Status, offline: bool) {
        let state = ui.global::<RecordState>();
        let changed = self.snapshot != status.unbound_records;
        if changed {
            self.snapshot = status.unbound_records.clone();
            // Indices can move when another segment arrives; never silently show a different one.
            state.set_unbound_selected(-1);
            state.set_unbound_page(0);
        }
        let version_changed = self.version != status.available;
        if version_changed {
            self.version = status.available.clone();
            self.names = None;
            self.attempted = false;
        }
        let mut refresh = changed || version_changed;
        if self.job.as_ref().is_some_and(|(_, job)| job.is_finished()) {
            refresh = true;
            let (version, job) = self.job.take().unwrap();
            if let Ok(names) = job.join() {
                if version == self.version {
                    self.names = names;
                    refresh = true;
                }
            }
        }
        if self.open
            && !self.attempted
            && self.job.is_none()
            && !self.snapshot.0.is_empty()
            && self.version.is_some()
        {
            self.attempted = true;
            let roots = self.roots.clone();
            let expected = self.version.clone().unwrap();
            self.job = std::thread::Builder::new()
                .name("fuze-observation-names".into())
                .spawn(move || Names::load(&roots, &expected).ok())
                .ok()
                .map(|job| (self.version.clone(), job));
            refresh = true;
        }
        if refresh && self.open {
            self.present(ui);
        }
        let note = if offline {
            "隔离预览不读取个人记录。"
        } else if status.record_error.is_some() {
            "记录读取或保存异常，原文件保留。"
        } else if !status.preferences.local_records {
            "保存已关闭；仅查看会话中已加载的观察。"
        } else {
            "最近 8 段本机观察，每段最多 80 次。账号与对局未确认，不自动并入战绩。"
        };
        if state.get_unbound_note() != note {
            state.set_unbound_note(note.into());
        }
    }
    fn name(&self, file: &str, id: u32) -> String {
        let name = self
            .names
            .as_ref()
            .and_then(|d| {
                if file == "champions.json" {
                    d.champions.get(&id)
                } else {
                    d.augments.get(&id)
                }
            })
            .cloned()
            .unwrap_or_else(|| format!("#{id}"));
        if name == format!("#{id}") {
            format!("#{id}（名称暂缺）")
        } else {
            name
        }
    }
    fn cards(&self, cards: &[Option<u32>; 3]) -> String {
        cards
            .iter()
            .enumerate()
            .map(|(i, id)| {
                format!(
                    "位置 {}：{}",
                    i + 1,
                    id.map(|id| self.name("augments.json", id))
                        .unwrap_or_else(|| "暂未识别，可能被遮挡或刷新".into())
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn present(&self, ui: &Workbench) {
        let state = ui.global::<RecordState>();
        state.set_unbound(
            Rc::new(VecModel::from(
                self.snapshot
                    .0
                    .iter()
                    .enumerate()
                    .map(|(i, s)| ObservationRow {
                        title: format!(
                            "记录段 {} · {} · {} 次观察",
                            i + 1,
                            self.name("champions.json", s.champion),
                            s.events.len().max(s.legacy.len())
                        )
                        .into(),
                        cards: String::new().into(),
                        hint: format!(
                            "{}{} · 非领取记录",
                            if s.interrupted {
                                "未确认完整结束"
                            } else {
                                "已记录结束"
                            },
                            if s.capped {
                                " · 已达 80 次上限"
                            } else {
                                ""
                            }
                        )
                        .into(),
                    })
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
        state.set_unbound_names(if self.version.is_none() {
            "等待名称数据版本，事件编号可先查看。".into()
        } else if self.job.is_some() {
            "正在加载对应版本的名称，事件可先查看。".into()
        } else if let Some(names) = &self.names {
            format!("名称数据：{} · 观察顺序不等于领取轮次", names.version).into()
        } else {
            "对应版本名称暂不可用，保留原始编号；可收起后重新打开重试。".into()
        });
        let Some(segment) = self.snapshot.0.get(state.get_unbound_selected() as usize) else {
            state.set_unbound_events(Default::default());
            state.set_unbound_pages(1);
            return;
        };
        let count = segment.events.len().max(segment.legacy.len());
        let pages = count.div_ceil(5).max(1);
        let page = (state.get_unbound_page().max(0) as usize).min(pages - 1);
        state.set_unbound_page(page as i32);
        state.set_unbound_pages(pages as i32);
        let rows = (page * 5..count.min(page * 5 + 5))
            .map(|index| {
                let event = segment.events.get(index);
                let cards = event
                    .map(|e| &e.cards)
                    .unwrap_or_else(|| &segment.legacy[index]);
                let kind = event
                    .map(|e| match e.kind.as_str() {
                        "first" => "首次观察",
                        "changed" => "候选变化",
                        "reappeared" => "重新出现",
                        _ => "旧记录",
                    })
                    .unwrap_or("旧记录");
                let time = event
                    .and_then(|e| e.observed_at)
                    .filter(|ms| *ms <= 253402300799999)
                    .map(|ms| {
                        format!(
                            "{}:{:02}.{:03} UTC",
                            history_detail::date(ms).trim_end_matches(" UTC"),
                            ms / 1000 % 60,
                            ms % 1000
                        )
                    })
                    .unwrap_or_else(|| "时间未记录".into());
                let level = event
                    .and_then(|e| e.level)
                    .map(|v| format!("等级 {v}"))
                    .unwrap_or_else(|| "等级未记录".into());
                ObservationRow {
                    title: format!("观察 {} · {kind} · {time}", index + 1).into(),
                    cards: self.cards(cards).into(),
                    hint: format!(
                        "{level} · {} · 不代表已领取",
                        if event.is_some_and(|e| e.partial) || cards.iter().any(Option::is_none) {
                            "部分识别"
                        } else {
                            "三卡可读"
                        }
                    )
                    .into(),
                }
            })
            .collect::<Vec<_>>();
        state.set_unbound_events(Rc::new(VecModel::from(rows)).into());
    }
}
pub fn attach(ui: &Workbench, roots: Vec<fuze_data::Root>) -> Rc<RefCell<Binding>> {
    let binding = Rc::new(RefCell::new(Binding {
        roots,
        snapshot: Default::default(),
        version: None,
        names: None,
        job: None,
        attempted: false,
        open: false,
    }));
    let handle = binding.clone();
    let weak = ui.as_weak();
    ui.global::<RecordState>()
        .on_unbound_action(move |action, value| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let state = ui.global::<RecordState>();
            let mut binding = handle.borrow_mut();
            match action.as_str() {
                "open" => {
                    binding.open = value != 0;
                    state.set_unbound_open(binding.open);
                    if binding.names.is_none() {
                        binding.attempted = false;
                    }
                }
                "select" if value >= 0 && (value as usize) < binding.snapshot.0.len() => {
                    state.set_unbound_selected(if state.get_unbound_selected() == value {
                        -1
                    } else {
                        value
                    });
                    state.set_unbound_page(0);
                }
                "page" => {
                    state.set_unbound_page((state.get_unbound_page() + value.clamp(-1, 1)).max(0))
                }
                _ => return,
            }
            binding.present(&ui);
        });
    binding
}
