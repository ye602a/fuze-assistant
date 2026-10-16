//! Native desktop and tray. UI never performs LCU, capture, model or filesystem IO.
use crate::{
    controller::{Config, Controller, Monitor, MonitorControl, PipelineStatus},
    Result,
};
use fuze_lcu::reconnect::State as Connection;
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_SETLIMITTEXT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, RegisterHotKey, UnregisterHotKey, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_F8,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
};
#[path = "claim_diagnostic_ui.rs"]
mod claim_ui;
#[path = "frontend_session.rs"]
pub mod frontend;
#[path = "history_presentation.rs"]
mod history_ui;
#[path = "maintenance_ui.rs"]
mod maintenance;
#[path = "quit_dialog.rs"]
mod quit_dialog;
#[path = "desktop_workbench.rs"]
mod workbench;
const PAUSED: u8 = 0;
const RUNNING: u8 = 1;
const EXIT: u8 = 2;
const DATA: u8 = 3;
const APPLY: u8 = 4;
const IMPORT: u8 = 5;
const HISTORY: u8 = 6;
const HISTORY_PREVIEW: u8 = 7;
const HISTORY_CONFIRM: u8 = 8;
const HISTORY_OFFLINE: u8 = 9;
const JOURNAL_PREVIEW: u8 = 10;
const EXPORT: u8 = 11;
const HISTORY_MORE: u8 = 12;
const HISTORY_CANCEL: u8 = 13;
const EXPORT_MATCH: usize = 222;
const LEVEL_GATE: usize = 223;
const AUGMENT_OVERLAY: usize = 224;
const AUGMENT_SIDE: usize = 225;
const JOURNAL_IMPORT: usize = 220;
const JOURNAL_VIEW: usize = 221;
const HISTORY_PAGE: usize = 213;
const HISTORY_REFRESH: usize = 214;
const HISTORY_IMPORT: usize = 215;
const HISTORY_ACCEPT: usize = 216;
const HISTORY_PREV: usize = 217;
const HISTORY_NEXT: usize = 218;
const HISTORY_SAVED: usize = 219;
const IMPORT_SETTINGS: usize = 211;
const RECORDS: usize = 108;
const RECORD_TOGGLE: usize = 212;
const SETTINGS: usize = 107;
const SAVE_SETTINGS: usize = 209;
const UPDATE_DATA: usize = 105;
const UPDATE_APP: usize = 106;
const TOGGLE: usize = 101;
const HIDE: usize = 102;
const QUIT: usize = 103;
const OPEN: usize = 104;
const SIDE: usize = 109;
const TRAY: u32 = WM_APP + 1;
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub initialized: bool,
    pub history: crate::account_history::View,
    pub records: Vec<crate::local_records::Row>,
    pub unbound_records: crate::local_records::UnboundSnapshot,
    pub record_error: Option<&'static str>,
    pub ui_locale: String,
    pub mode: u8,
    pub verified_time: Option<String>,
    pub connection: Option<Connection>,
    pub pipeline: PipelineStatus,
    pub resources: crate::resources::Snapshot,
    pub recognition: crate::recognition_sources::Summary,
    pub done: bool,
    pub available: Option<String>,
    pub notice: Option<&'static str>,
    pub update_error: Option<&'static str>,
    pub preferences: crate::preferences::Preferences,
    pub selection: Option<crate::champ_select::View>,
    pub playing: Option<crate::champ_select::View>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            initialized: false,
            history: Default::default(),
            records: Vec::new(),
            unbound_records: Default::default(),
            record_error: None,
            ui_locale: "zh-CN".into(),
            mode: PAUSED,
            verified_time: None,
            connection: None,
            pipeline: PipelineStatus::default(),
            resources: Default::default(),
            recognition: Default::default(),
            done: false,
            available: None,
            notice: None,
            update_error: None,
            preferences: Default::default(),
            selection: None,
            playing: None,
        }
    }
}
struct Shared {
    external_overlays: bool,
    start_paused: bool,
    automatic_change: AtomicU8,
    resume_apply: AtomicU8,
    wake: Condvar,
    wait: Mutex<()>,
    desired: AtomicU8,
    status: Mutex<Status>,
    monitor: Mutex<Option<MonitorControl>>,
    pending: Mutex<Option<crate::preferences::Preferences>>,
    import_path: Mutex<Option<std::path::PathBuf>>,
    import_game: Mutex<Option<u64>>,
    import_selection: Mutex<Option<Vec<u64>>>,
    export: Mutex<Option<crate::record_export::Request>>,
}
impl Shared {
    fn finish_request(&self, from: u8, to: u8) {
        let _guard = self.wait.lock().ok();
        if self
            .desired
            .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            if let Ok(control) = self.monitor.lock() {
                if let Some(c) = control.as_ref() {
                    c.enable(to == RUNNING);
                }
            }
        }
    }
    fn request(&self, mode: u8) {
        let _guard = self.wait.lock().ok();
        // Exit is terminal, including queued button/tray commands.
        if self.desired.load(Ordering::Acquire) == EXIT {
            return;
        }
        self.desired.store(mode, Ordering::Release);
        self.wake.notify_one();
        if let Ok(control) = self.monitor.lock() {
            if let Some(control) = control.as_ref() {
                control.enable(
                    mode == RUNNING
                        || mode == APPLY && self.resume_apply.load(Ordering::Acquire) == RUNNING,
                );
            }
        }
    }
    fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}
fn engine(mut config: Config, shared: Arc<Shared>, fixture: bool) {
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(|p| std::path::PathBuf::from(p).join("FuzeRustPreview/client-data"));
    if !fixture {
        if let Some(path) = cache.as_ref() {
            config.roots.push(fuze_data::Root {
                path: path.clone(),
                source: fuze_data::Source::Cache,
            });
        }
    }
    let mut preferences = crate::preferences::Preferences::default();
    let mut preferences_valid = true;
    if fixture {
        if let Ok(locale) = std::env::var("FUZE_SMOKE_LOCALE") {
            if ["zh-CN", "zh-TW", "en-US"].contains(&locale.as_str()) {
                preferences.locale = locale;
                if preferences.apply(&mut config).is_err() {
                    return;
                }
            }
        }
    }
    let preference_path = if fixture {
        None
    } else {
        crate::preferences::path().ok()
    };
    if let Some(path) = preference_path.as_ref() {
        match crate::preferences::read(path).and_then(|p| {
            p.apply(&mut config)?;
            Ok(p)
        }) {
            Ok(p) => preferences = p,
            Err(e) => {
                preferences_valid = false;
                if let Ok(mut s) = shared.status.lock() {
                    s.update_error = Some(e);
                }
            }
        }
    }
    let roots: Vec<_> = config
        .roots
        .iter()
        .map(|r| fuze_data::Root {
            path: r.path.clone(),
            source: r.source,
        })
        .collect();
    let available = |locale| {
        fuze_data::prepare(&roots, locale, fuze_data::Scope::Ocr)
            .ok()
            .map(|d| d.identity())
    };
    if let Ok((_, summary)) = crate::recognition_sources::load(&roots) {
        if let Ok(mut s) = shared.status.lock() {
            s.recognition = summary;
        }
    }
    let initial_available = available(config.locale.unwrap_or(fuze_data::Locale::ZhCn));
    let mut fixture_history = crate::account_history::View::default();
    if fixture {
        fixture_history = crate::account_history::View {
            account: "demo0001".into(),
            platform: "HN1".into(),
            cached: true,
            rows: (1..=8)
                .map(|id| fuze_lcu::history::Match {
                    game_id: id,
                    created: 1709164800000,
                    duration: 900,
                    champion: 21,
                    kills: 8,
                    deaths: 3,
                    assists: 15,
                    result: "win".into(),
                    items: vec![1001, 3089, 3157],
                    augments: vec![1],
                })
                .collect::<Vec<_>>()
                .into(),
            observations: vec![fuze_lcu::history::Observation {
                recognition: None,
                game_id: 1,
                interrupted: true,
                capped: false,
                events: (0..8)
                    .map(|_| fuze_lcu::history::ObservedCards {
                        observed_at: None,
                        cards: [Some(1), None, Some(3)],
                        level: Some(11),
                        partial: true,
                        kind: "changed".into(),
                    })
                    .collect(),
            }]
            .into(),
            ..Default::default()
        };
        crate::history_detail::populate(
            &mut fixture_history,
            &roots,
            config.locale.unwrap_or(fuze_data::Locale::ZhCn),
        );
    }
    if let Ok(mut s) = shared.status.lock() {
        if fixture {
            s.history = fixture_history;
        }
        s.initialized = true;
        s.available = initial_available;
        s.preferences = preferences.clone();
        s.ui_locale = match preferences.locale.as_str() {
            "auto" => "zh-CN",
            v => v,
        }
        .into();
    }
    if !shared.start_paused && preferences.starts_automatically(fixture, preferences_valid) {
        shared.finish_request(PAUSED, RUNNING);
    }
    let mut controller = Controller::new({
        let mut c = config.clone();
        if shared.external_overlays {
            c.augment_overlay = false;
        }
        c
    });
    if preferences.local_records && !fixture {
        controller.enable_local_records();
    }
    let mut champions = crate::catalog_session::Session::new();
    let mut monitor = None;
    let mut last_mode = PAUSED;
    let mut resource_sampler = crate::resources::Sampler::default();
    let mut background = crate::background_data::Refresh::new();
    let mut history_background = crate::post_game_history::Refresh::default();
    let mut history_import: Option<crate::account_history::Import> = None;
    let mut history_before_import: Option<crate::account_history::View> = None;
    let mut history_account_index = 0;
    loop {
        let automatic = shared.automatic_change.swap(0, Ordering::AcqRel);
        if automatic != 0 && !fixture && preferences.automatic != (automatic == 2) {
            let mut next = preferences.clone();
            next.automatic = automatic == 2;
            let result = if preferences_valid {
                preference_path
                    .as_ref()
                    .ok_or("preferences-root")
                    .and_then(|p| crate::preferences::save(p, &next))
            } else {
                Err("preferences-invalid")
            };
            if result.is_ok() {
                preferences = next;
            }
            if let Ok(mut s) = shared.status.lock() {
                s.preferences = preferences.clone();
                if let Err(e) = result {
                    s.update_error = Some(e);
                    s.notice = Some("识别状态已切换，但未能保存，下次启动可能恢复旧状态。");
                }
            }
        }
        if !fixture && resource_sampler.due() {
            resource_sampler.sample(controller.usage());
            if let Ok(mut s) = shared.status.lock() {
                s.resources = resource_sampler.latest.clone();
            }
        }
        let desired = shared.desired.load(Ordering::Acquire);
        if desired == EXIT {
            break;
        }
        if !fixture {
            let phase = if desired == RUNNING && preferences.local_records {
                monitor.as_ref().and_then(|m: &Monitor| match m.state() {
                    Some(Connection::Connected(p)) => Some(p),
                    _ => None,
                })
            } else {
                None
            };
            let manual = (!preferences.manual_directory.is_empty())
                .then(|| std::path::PathBuf::from(&preferences.manual_directory));
            if let Some(mut view) = history_background.tick(phase, manual) {
                crate::history_detail::populate(
                    &mut view,
                    &roots,
                    config.locale.unwrap_or(fuze_data::Locale::ZhCn),
                );
                if let Ok(mut s) = shared.status.lock() {
                    s.history = view;
                }
            }
            // Serialize explicit cache writers behind the bounded background transaction.
            if history_background.busy()
                && [
                    HISTORY,
                    HISTORY_MORE,
                    HISTORY_CONFIRM,
                    HISTORY_PREVIEW,
                    HISTORY_OFFLINE,
                    JOURNAL_PREVIEW,
                ]
                .contains(&desired)
            {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            let idle = desired == RUNNING
                && matches!(
                    monitor.as_ref().and_then(|m: &Monitor| m.state()),
                    Some(Connection::Connected(
                        fuze_runtime::session::Phase::ClientReady
                            | fuze_runtime::session::Phase::PostGame
                    ))
                );
            if let Some(result) = background.tick(
                idle,
                cache.as_ref(),
                &roots,
                config
                    .locale
                    .or_else(|| monitor.as_ref().and_then(|m| m.data_locale()))
                    .unwrap_or(fuze_data::Locale::ZhCn),
            ) {
                if result.as_ref().is_ok_and(|o| o.updated) {
                    champions.invalidate();
                }
                let supplemental = crate::recognition_sources::load(&roots)
                    .ok()
                    .map(|(_, s)| s);
                if let Ok(mut status) = shared.status.lock() {
                    status.update_error = match result {
                        Ok(o) => o.supplemental_error,
                        Err(e) => Some(e),
                    };
                    if let Some(summary) = supplemental {
                        status.recognition = summary;
                    }
                }
            }
            if desired == DATA && background.busy() {
                // The writer owns an exclusive transaction lock. Wait outside the UI thread
                // for its cancellation before an explicit refresh can acquire it.
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        }
        if desired == PAUSED {
            champions.clear_selection();
            if last_mode != PAUSED {
                controller.suspend();
            }
            if let Ok(mut status) = shared.status.lock() {
                status.mode = PAUSED;
                status.connection = None;
                status.pipeline = controller.status();
                status.selection = None;
            }
        } else if desired == EXPORT {
            controller.suspend();
            let result = if fixture {
                Err("export-fixture-disabled")
            } else {
                shared
                    .export
                    .lock()
                    .ok()
                    .and_then(|mut r| r.take())
                    .ok_or("export-missing")
                    .and_then(crate::record_export::save)
            };
            if let Ok(mut s) = shared.status.lock() {
                s.notice = Some(if result.is_ok() {
                    "匿名导出已保存到本机。"
                } else {
                    result.unwrap_err()
                });
            }
            let _ = shared.desired.compare_exchange(
                EXPORT,
                PAUSED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        } else if [
            HISTORY,
            HISTORY_MORE,
            HISTORY_PREVIEW,
            HISTORY_CONFIRM,
            HISTORY_OFFLINE,
            JOURNAL_PREVIEW,
            HISTORY_CANCEL,
        ]
        .contains(&desired)
        {
            controller.suspend();
            if let Ok(mut s) = shared.status.lock() {
                s.mode = desired;
            }
            let manual = (!preferences.manual_directory.is_empty())
                .then(|| std::path::Path::new(&preferences.manual_directory));
            let result = (|| -> Result<crate::account_history::View> {
                if desired == HISTORY_CANCEL {
                    history_import = None;
                    let mut previous = history_before_import.take().unwrap_or_default();
                    previous.cached = true; // Cancelling does not revalidate the logged-in account.
                    return Ok(previous);
                }
                if fixture {
                    return Err("history-fixture-disabled");
                }
                let path = crate::account_history::path()?;
                match desired {
                    JOURNAL_PREVIEW => {
                        if history_before_import.is_none() {
                            history_before_import = Some(shared.status().history);
                        }
                        history_import = None;
                        let source = shared
                            .import_path
                            .lock()
                            .ok()
                            .and_then(|mut p| p.take())
                            .ok_or("journal-source-required")?;
                        let game = shared
                            .import_game
                            .lock()
                            .ok()
                            .and_then(|mut p| p.take())
                            .ok_or("journal-game-required")?;
                        let pending =
                            crate::account_history::preview_journal(manual, &source, game)?;
                        let view = pending.view();
                        history_import = Some(pending);
                        Ok(view)
                    }
                    HISTORY_OFFLINE => {
                        history_import = None;
                        history_before_import = None;
                        let view = crate::account_history::offline(&path, history_account_index)?;
                        history_account_index = history_account_index.wrapping_add(1);
                        Ok(view)
                    }
                    HISTORY | HISTORY_MORE => {
                        history_import = None;
                        history_before_import = None;
                        let previous = shared.status().history;
                        if desired == HISTORY_MORE {
                            Ok(crate::account_history::refresh_page(
                                manual,
                                &path,
                                previous.next_start.ok_or("history-page-limit")?,
                                Some((&previous.account, &previous.platform)),
                                Some(&previous.source),
                            ))
                        } else {
                            Ok(crate::account_history::refresh(manual, &path))
                        }
                    }
                    HISTORY_PREVIEW => {
                        if history_before_import.is_none() {
                            history_before_import = Some(shared.status().history);
                        }
                        history_import = None;
                        let source = shared
                            .import_path
                            .lock()
                            .ok()
                            .and_then(|mut p| p.take())
                            .ok_or("history-import-missing")?;
                        let pending = crate::account_history::preview(manual, &source)?;
                        let view = pending.view();
                        history_import = Some(pending);
                        Ok(view)
                    }
                    _ => {
                        let mut pending = history_import.take().ok_or("history-import-missing")?;
                        if let Some(selected) = shared
                            .import_selection
                            .lock()
                            .map_err(|_| "frontend-lock")?
                            .take()
                        {
                            pending = pending.select(&selected)?;
                        }
                        let confirmed = pending.confirm(manual, &path)?;
                        history_before_import = None;
                        Ok(confirmed)
                    }
                }
            })();
            let mut view = result.unwrap_or_else(|e| {
                let mut preserved = history_before_import.take().unwrap_or_default();
                preserved.error = Some(e);
                preserved.pending = 0;
                preserved.journal_preview = false;
                preserved.cached = true;
                preserved
            });
            let history_locale = match shared.status().ui_locale.as_str() {
                "en-US" => fuze_data::Locale::EnUs,
                "zh-TW" => fuze_data::Locale::ZhTw,
                _ => fuze_data::Locale::ZhCn,
            };
            crate::history_detail::populate(&mut view, &roots, history_locale);
            if let Ok(mut s) = shared.status.lock() {
                s.history = view;
            }
            let _ = shared.desired.compare_exchange(
                desired,
                PAUSED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        } else if desired == APPLY || desired == IMPORT {
            let resume = if desired == APPLY {
                shared.resume_apply.load(Ordering::Acquire)
            } else {
                PAUSED
            };
            let candidate = shared.pending.lock().ok().and_then(|mut p| p.take());
            let mut reconfigured = false;
            let result = (|| {
                let mut p = if desired == IMPORT {
                    let path = shared
                        .import_path
                        .lock()
                        .ok()
                        .and_then(|mut p| p.take())
                        .ok_or("legacy-settings-missing")?;
                    crate::legacy_preferences::read(&path, &preferences)?
                } else {
                    candidate.ok_or("preferences-missing")?
                };
                if desired == APPLY {
                    p.automatic = preferences.automatic;
                }
                let restart = crate::preferences::commit_change(
                    &mut preferences,
                    &mut config,
                    p,
                    preference_path.as_deref(),
                )?;
                preferences_valid = true;
                if restart {
                    reconfigured = true;
                    controller.suspend();
                    if let Ok(mut control) = shared.monitor.lock() {
                        *control = None;
                    }
                    monitor = None;
                    controller = Controller::new({
                        let mut c = config.clone();
                        if shared.external_overlays {
                            c.augment_overlay = false;
                        }
                        c
                    });
                    if preferences.local_records && !fixture {
                        controller.enable_local_records();
                    }
                    champions.invalidate();
                }
                Ok::<(), &'static str>(())
            })();
            let data = if reconfigured {
                available(config.locale.unwrap_or(fuze_data::Locale::ZhCn))
            } else {
                shared.status().available
            };
            let mut history = shared.status().history;
            if result.is_ok() && reconfigured {
                crate::history_detail::populate(
                    &mut history,
                    &roots,
                    config.locale.unwrap_or(fuze_data::Locale::ZhCn),
                );
            }
            if let Ok(mut s) = shared.status.lock() {
                if result.is_ok() {
                    s.history = history;
                }
                s.preferences = preferences.clone();
                s.ui_locale = match preferences.locale.as_str() {
                    "auto" => "zh-CN",
                    v => v,
                }
                .into();
                s.available = data;
                if reconfigured {
                    s.selection = None;
                    s.playing = None;
                    s.pipeline = PipelineStatus::default();
                    s.connection = None;
                    s.verified_time = None;
                }
                s.update_error = result.err();
                s.notice = Some(if s.update_error.is_none() {
                    "偏好已保存，正在应用。"
                } else {
                    "保存未完成；仍使用原设置。"
                });
            }
            shared.finish_request(desired, resume);
            shared.resume_apply.store(PAUSED, Ordering::Release);
        } else if desired == DATA {
            controller.suspend();
            if let Ok(mut s) = shared.status.lock() {
                s.mode = DATA;
                s.notice = Some("正在下载并校验；旧数据继续保留。");
            }
            let result = if fixture {
                Ok(Vec::new())
            } else {
                cache.as_ref().ok_or("data-update-cache").and_then(|path| {
                    let cancel = || shared.desired.load(Ordering::Acquire) != DATA;
                    let supplemental = crate::recognition_sources::refresh(path, &cancel);
                    let primary = crate::data_update::refresh(path, &cancel);
                    primary.and_then(|updated| supplemental.map(|_| updated))
                })
            };
            let refreshed_available = available(config.locale.unwrap_or(fuze_data::Locale::ZhCn));
            champions.invalidate();
            if let Ok(mut s) = shared.status.lock() {
                s.available = refreshed_available;
                if let Ok((_, summary)) = crate::recognition_sources::load(&roots) {
                    s.recognition = summary;
                }
                s.update_error = result.as_ref().err().copied();
                s.notice = Some(if result.is_ok() {
                    "推荐数据已校验；下次开始识别时使用最新完整版本。"
                } else {
                    "更新未完成；保留可用数据，可稍后重试。"
                });
            }
            let _ =
                shared
                    .desired
                    .compare_exchange(DATA, PAUSED, Ordering::AcqRel, Ordering::Acquire);
        } else if fixture {
            // Explicit offline entry only. No discovery or game access exists in this branch.
            let mut selection = fuze_lcu::context::Selection {
                team: [1, 4, 17, 51, 157],
                local_slot: Some(1),
                bench: [0; 20],
                count: 12,
            };
            selection.bench[..12].copy_from_slice(&[2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
            champions.refresh(
                &roots,
                Some(config.locale.unwrap_or(fuze_data::Locale::ZhCn)),
                Some(selection),
            );
            if let Ok(mut status) = shared.status.lock() {
                status.mode = RUNNING;
                status.connection = Some(Connection::Connected(
                    fuze_runtime::session::Phase::ClientReady,
                ));
                status.selection = champions.selection();
            }
        } else {
            if last_mode != RUNNING {
                if let Ok(mut s) = shared.status.lock() {
                    if s.update_error.is_none() {
                        s.notice = None;
                    }
                }
            }
            if monitor.is_none() {
                match Monitor::start_with_records(
                    false,
                    if preferences.manual_directory.is_empty() {
                        None
                    } else {
                        Some(std::path::PathBuf::from(&preferences.manual_directory))
                    },
                    preferences.local_records,
                ) {
                    Ok(m) => {
                        if let Ok(mut control) = shared.monitor.lock() {
                            let c = m.control();
                            c.enable(shared.desired.load(Ordering::Acquire) == RUNNING);
                            *control = Some(c);
                        }
                        monitor = Some(m);
                    }
                    Err(error) => {
                        if let Ok(mut s) = shared.status.lock() {
                            s.mode = RUNNING;
                            s.pipeline.error = Some(error);
                        }
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                }
            }
            if let Some(m) = monitor.as_ref() {
                let _ = controller.tick(m);
                let selection = m.selection().filter(|_| preferences.champ_select);
                let selected_locale = config.locale.or_else(|| m.data_locale());
                champions.refresh(&roots, selected_locale, selection);
                if let Ok(mut s) = shared.status.lock() {
                    s.mode = RUNNING;
                    s.connection = m.state();
                    s.verified_time = m.verified_age().map(crate::resources::clock_ago);
                    s.pipeline = controller.status();
                    s.selection = champions.selection();
                    s.playing = m.playing_champion().and_then(|id| champions.playing(id));
                    // Only change interface language after that locale's catalogue is ready.
                    if let Some(identity) = champions.identity() {
                        if let Some(locale) = identity.split('@').next() {
                            if preferences.locale != "auto" {
                                s.ui_locale = locale.into();
                            }
                        }
                        s.available = Some(identity.into());
                    }
                    if selection.is_some() && selected_locale.is_none() {
                        s.notice = Some("选人参考等待客户端语言，暂不混用不同语言数据。");
                    } else if selection.is_some() && champions.identity().is_none() {
                        s.notice = Some("选人数据暂不可用，30 秒后重试；可暂停后更新推荐数据。");
                    } else if s.update_error.is_none() {
                        s.notice = None;
                    }
                }
            }
        }
        last_mode = desired;
        if let Ok(mut s) = shared.status.lock() {
            (s.records, s.record_error) = controller.records();
            s.unbound_records = controller.unbound_records();
        }
        if let Ok(guard) = shared.wait.lock() {
            if shared.desired.load(Ordering::Acquire) == desired {
                let _wait = shared.wake.wait_timeout(
                    guard,
                    if desired == RUNNING && !fixture {
                        controller.idle_wait()
                    } else {
                        Duration::from_secs(60)
                    },
                );
            }
        }
    }
    if let Ok(control) = shared.monitor.lock() {
        if let Some(c) = control.as_ref() {
            c.enable(false);
        }
    }
    controller.suspend();
    drop(monitor);
    if let Ok(mut s) = shared.status.lock() {
        s.done = true;
        s.mode = EXIT;
    }
}
fn wide(s: &str) -> Vec<u16> {
    crate::i18n::text(s).encode_utf16().chain(Some(0)).collect()
}
unsafe fn app_icon() -> Option<HICON> {
    let bytes = include_bytes!("../../../../public/fuze.ico");
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    for i in 0..count {
        let e = bytes.get(6 + i * 16..22 + i * 16)?;
        if e[0] != 64 {
            continue;
        }
        let length = u32::from_le_bytes(e[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(e[12..16].try_into().ok()?) as usize;
        let data = bytes.get(offset..offset.checked_add(length)?)?;
        let icon = CreateIconFromResourceEx(
            data.as_ptr(),
            length as u32,
            1,
            0x30000,
            64,
            64,
            LR_DEFAULTCOLOR,
        );
        if !icon.is_null() {
            return Some(icon);
        }
    }
    None
}
// Only called by the explicit offline smoke entry; captures this exact owned HWND.
unsafe fn snapshot(hwnd: HWND, path: &str, ui: &mut Ui) -> Result<()> {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let dc = GetDC(hwnd);
    let memory = CreateCompatibleDC(dc);
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = r.right;
    info.bmiHeader.biHeight = -r.bottom;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || memory.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !memory.is_null() {
            DeleteDC(memory);
        }
        ReleaseDC(hwnd, dc);
        return Err("desktop-image");
    }
    let old = SelectObject(memory, bitmap);
    draw(hwnd, memory, ui);
    // Native hidden-window render; never captures the desktop or another application.
    for id in [
        workbench::HOME,
        TOGGLE,
        UPDATE_DATA,
        UPDATE_APP,
        SETTINGS,
        RECORDS,
        HIDE,
        QUIT,
        EXPORT_MATCH,
        LEVEL_GATE,
        AUGMENT_OVERLAY,
        AUGMENT_SIDE,
        201,
        202,
        203,
        204,
        205,
        206,
        207,
        208,
        209,
        210,
        IMPORT_SETTINGS,
        RECORD_TOGGLE,
        HISTORY_PAGE,
        HISTORY_REFRESH,
        HISTORY_IMPORT,
        HISTORY_ACCEPT,
        HISTORY_PREV,
        HISTORY_NEXT,
        HISTORY_SAVED,
        JOURNAL_IMPORT,
        JOURNAL_VIEW,
        history_ui::CHAMPION,
        history_ui::RESULT,
        history_ui::COPY,
        maintenance::PREVIEW,
        maintenance::DIAGNOSTICS,
        maintenance::SCAN,
        maintenance::CLEAN,
        claim_ui::INSPECT,
        claim_ui::BEFORE,
        claim_ui::AFTER,
        claim_ui::SCENARIO,
    ] {
        let child = GetDlgItem(hwnd, id as i32);
        if GetWindowLongW(child, GWL_STYLE) as u32 & WS_VISIBLE == 0 {
            continue;
        }
        let mut bounds: RECT = zeroed();
        GetWindowRect(child, &mut bounds);
        let mut point = POINT {
            x: bounds.left,
            y: bounds.top,
        };
        ScreenToClient(hwnd, &mut point);
        SetViewportOrgEx(memory, point.x, point.y, null_mut());
        SendMessageW(
            child,
            WM_PRINT,
            memory as usize,
            (PRF_CLIENT | PRF_ERASEBKGND | PRF_NONCLIENT) as isize,
        );
        SetViewportOrgEx(memory, 0, 0, null_mut());
    }
    let mut pixels =
        std::slice::from_raw_parts(bits as *const u8, (r.right * r.bottom * 4) as usize).to_vec();
    SelectObject(memory, old);
    DeleteObject(bitmap);
    DeleteDC(memory);
    ReleaseDC(hwnd, dc);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    image::save_buffer(
        path,
        &pixels,
        r.right as u32,
        r.bottom as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|_| "desktop-image")
}
pub(crate) fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
pub(crate) unsafe fn fill(dc: HDC, r: RECT, c: u32) -> () {
    let b = CreateSolidBrush(c);
    FillRect(dc, &r, b);
    DeleteObject(b);
}
pub(crate) unsafe fn text(dc: HDC, s: &str, r: RECT, size: i32, c: u32) {
    let s = crate::i18n::text(s);
    let f = CreateFontW(
        -size,
        0,
        0,
        0,
        500,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        wide("Microsoft YaHei UI").as_ptr(),
    );
    let old = SelectObject(dc, f);
    SetTextColor(dc, c);
    SetBkMode(dc, TRANSPARENT as i32);
    let s = wide(s);
    let mut r = r;
    DrawTextW(
        dc,
        s.as_ptr(),
        (s.len() - 1) as i32,
        &mut r,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    SelectObject(dc, old);
    DeleteObject(f);
}
fn connection(s: Option<Connection>) -> &'static str {
    use fuze_runtime::session::Phase::*;
    match s {
        Some(Connection::Connected(ClientReady)) => "已连接 · 大厅 / 匹配中",
        Some(Connection::Connected(ChampSelect)) => "已连接 · 选择英雄",
        Some(Connection::Connected(Loading)) => "已连接 · 正在载入",
        Some(Connection::Connected(InProgress)) => "已连接 · 对局中",
        Some(Connection::Connected(PostGame)) => "已连接 · 结算中",
        Some(Connection::Retry(fuze_lcu::Error::Unauthorized)) => "认证失败 · 等待重新发现",
        Some(Connection::Retry(_)) => "连接异常 · 后台重试",
        Some(Connection::Paused) => "已暂停连接检测",
        _ => "等待英雄联盟客户端",
    }
}
fn capture(p: &PipelineStatus) -> &'static str {
    match p.capture {
        "prewarmed" => "OCR 与推荐数据已预热 · 等待对局",
        "stat-shards" => "属性碎片界面 · 等待强化符文",
        "retaining" => "等待刷新 · 暂留上次结果",
        "receiving" if p.frame_age.is_some_and(|v| v < 5) => "画面已处理",
        "receiving" => "等待新的可用画面",
        "suspended" => "游戏切出 / 最小化 · 已暂停截图",
        "recovering" => "截图异常 · 等待重新建流",
        "resizing" => "游戏分辨率变化 · 正在重建",
        "gpu-busy" => "显卡忙碌 · 已跳过本帧",
        "waiting-frame" => "等待新画面 · 不重复识别",
        _ => "等待支持的对局与游戏窗口",
    }
}
fn explain(error: Option<&str>) -> &'static str {
    match error {
        None => "仅推荐，不操作游戏。领取轮次未确认，使用整体胜率。",
        Some("locale-unavailable") => "暂未取得支持的客户端语言；自动重试，不猜测语言。",
        Some("game-window-waiting") => "尚未找到匹配的游戏窗口；客户端连接不等于已进入游戏。",
        Some("worker-recovered-use-next-frame") => "OCR 已回退到 CPU，等待下一帧继续识别。",
        Some(s) if s.starts_with("capture") || s.starts_with("game-window") => {
            "截图链路暂不可用，正在退避重试；不会操作游戏。"
        }
        Some(s) if s.starts_with("worker") => "OCR 进程异常；正在清理并尝试恢复。",
        Some(s) if s.starts_with("data") => "本地推荐数据未就绪；请检查开发包是否完整。",
        Some(s) if s.starts_with("preferences") => {
            "设置读写未完成，原文件已保留；请检查目录权限后重试。"
        }
        _ => "模块暂不可用，正在重试；下方仅显示脱敏错误分类。",
    }
}
struct Ui {
    layout_page: std::cell::Cell<u8>,
    maintenance: maintenance::State,
    scroll: i32,
    health_wait: Option<Instant>,
    journal: bool,
    journal_offset: usize,
    history_assets: crate::champion_assets::Service,
    history_reply: crate::champion_assets::Reply,
    history_offset: usize,
    history_page: usize,
    level_details: bool,
    data_details: bool,
    history_champion: u32,
    history_result: usize,
    history: bool,
    records: bool,
    placement: crate::window_memory::Memory,
    hotkey_ok: bool,
    settings: bool,
    draft: crate::preferences::Preferences,
    champions: Option<crate::champ_select::Window>,
    side: Option<crate::champ_select::Window>,
    side_roots: Vec<fuze_data::Root>,
    side_open: bool,
    champion_preview_ok: bool,
    shared: Arc<Shared>,
    shown: Status,
    desired: u8,
    tray: bool,
    tray_registered: bool,
    tray_recovered: bool,
    tray_removed: bool,
    preview_ok: bool,
    icon: HICON,
    icon_owned: bool,
    taskbar: u32,
    exiting: Option<Instant>,
    paints: u32,
    smoke: bool,
    smoke_step: u8,
    smoke_pass: bool,
}
impl Ui {
    fn apply_preference(&mut self) {
        let current = self.shared.desired.load(Ordering::Acquire);
        if ![PAUSED, RUNNING].contains(&current) {
            return;
        }
        self.shared.resume_apply.store(
            if current == RUNNING { RUNNING } else { PAUSED },
            Ordering::Release,
        );
        if let Ok(mut pending) = self.shared.pending.lock() {
            *pending = Some(self.draft.clone());
        }
        self.shared.request(APPLY);
        self.desired = APPLY;
    }
    fn observation(&self) -> Option<&fuze_lcu::history::Observation> {
        let h = &self.shown.history;
        if h.journal_preview {
            return h.observations.first();
        }
        let game = h.rows.get(self.history_offset)?.game_id;
        h.observations.iter().find(|o| o.game_id == game)
    }
}
unsafe fn tray(hwnd: HWND, icon: HICON, add: bool) -> bool {
    let mut n: NOTIFYICONDATAW = zeroed();
    n.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd;
    n.uID = 1;
    n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    n.uCallbackMessage = TRAY;
    n.hIcon = icon;
    let tip = wide("符择 FUZE · 双击打开，右键暂停或退出");
    n.szTip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(if add { NIM_ADD } else { NIM_DELETE }, &n) != 0
}
unsafe fn show(hwnd: HWND) {
    ShowWindow(hwnd, SW_RESTORE);
    SetForegroundWindow(hwnd);
}
fn cycle<'a>(current: &str, values: &[&'a str]) -> &'a str {
    values[(values.iter().position(|s| *s == current).unwrap_or(0) + 1) % values.len()]
}
unsafe fn settings_controls(hwnd: HWND, u: &Ui) {
    maintenance::controls(hwnd, u);
    history_ui::controls(hwnd, u);
    let export = GetDlgItem(hwnd, EXPORT_MATCH as i32);
    ShowWindow(
        export,
        if u.records && u.history {
            SW_SHOWNA
        } else {
            SW_HIDE
        },
    );
    EnableWindow(
        export,
        i32::from(
            !u.smoke
                && u.desired == PAUSED
                && u.shown.mode == PAUSED
                && u.shown.history.pending == 0
                && u.shown.history.rows.get(u.history_offset).is_some(),
        ),
    );
    SetWindowTextW(export, wide("导出当前对局（匿名）").as_ptr());
    let view = GetDlgItem(hwnd, JOURNAL_VIEW as i32);
    ShowWindow(
        view,
        if u.records && u.history {
            SW_SHOWNA
        } else {
            SW_HIDE
        },
    );
    EnableWindow(view, i32::from(u.journal || u.observation().is_some()));
    SetWindowTextW(
        view,
        wide(if u.journal {
            "返回战绩详情"
        } else {
            "查看候选观察（非领取）"
        })
        .as_ptr(),
    );
    let journal = GetDlgItem(hwnd, JOURNAL_IMPORT as i32);
    ShowWindow(
        journal,
        if u.records && u.history {
            SW_SHOWNA
        } else {
            SW_HIDE
        },
    );
    EnableWindow(
        journal,
        i32::from(
            !u.smoke
                && u.desired == PAUSED
                && u.shown.mode == PAUSED
                && u.shown.history.rows.get(u.history_offset).is_some(),
        ),
    );
    SetWindowTextW(journal, wide("导入此局旧观察").as_ptr());
    let saved = GetDlgItem(hwnd, HISTORY_SAVED as i32);
    ShowWindow(
        saved,
        if u.records && u.history {
            SW_SHOWNA
        } else {
            SW_HIDE
        },
    );
    EnableWindow(
        saved,
        i32::from(!u.smoke && u.desired == PAUSED && u.shown.mode == PAUSED),
    );
    SetWindowTextW(saved, wide("浏览已存账号（离线）").as_ptr());
    for (id, label, enabled) in [
        (
            HISTORY_PREV,
            "上一页",
            if u.journal {
                u.journal_offset > 0
            } else {
                u.history_page > 0
            },
        ),
        (
            HISTORY_NEXT,
            "下一页",
            if u.journal {
                u.observation()
                    .is_some_and(|o| u.journal_offset + 5 < o.events.len())
            } else {
                u.history_page + history_ui::PAGE < history_ui::indices(u).len()
            },
        ),
    ] {
        let child = GetDlgItem(hwnd, id as i32);
        ShowWindow(
            child,
            if u.records && u.history {
                SW_SHOWNA
            } else {
                SW_HIDE
            },
        );
        EnableWindow(child, i32::from(enabled));
        SetWindowTextW(child, wide(label).as_ptr());
    }
    for (id, label) in [
        (
            HISTORY_PAGE,
            if u.history {
                "识别观察记录"
            } else {
                "账号战绩"
            },
        ),
        (HISTORY_REFRESH, "刷新最近 50 场"),
        (HISTORY_IMPORT, "预览旧战绩导入"),
        (HISTORY_ACCEPT, "确认导入匹配记录"),
    ] {
        let child = GetDlgItem(hwnd, id as i32);
        let visible = u.records && (id == HISTORY_PAGE || u.history);
        ShowWindow(child, if visible { SW_SHOWNA } else { SW_HIDE });
        EnableWindow(
            child,
            i32::from(
                id == HISTORY_PAGE
                    || (!u.smoke
                        && u.desired == PAUSED
                        && u.shown.mode == PAUSED
                        && (id != HISTORY_ACCEPT || u.shown.history.pending > 0)),
            ),
        );
        SetWindowTextW(child, wide(label).as_ptr());
    }
    let toggle = GetDlgItem(hwnd, RECORD_TOGGLE as i32);
    ShowWindow(
        toggle,
        if u.records && !u.history {
            SW_SHOWNA
        } else {
            SW_HIDE
        },
    );
    EnableWindow(
        toggle,
        i32::from(u.desired == PAUSED && u.shown.mode == PAUSED),
    );
    SetWindowTextW(
        toggle,
        wide(if u.shown.preferences.local_records {
            "停止保存新记录"
        } else {
            "开启本地记录"
        })
        .as_ptr(),
    );
    let import = GetDlgItem(hwnd, IMPORT_SETTINGS as i32);
    ShowWindow(import, if u.settings { SW_SHOWNA } else { SW_HIDE });
    EnableWindow(
        import,
        i32::from(u.desired == PAUSED && u.shown.mode == PAUSED),
    );
    SetWindowTextW(import, wide("导入旧版设置").as_ptr());
    ShowWindow(
        GetDlgItem(hwnd, 210),
        if u.settings { SW_SHOWNA } else { SW_HIDE },
    );
    EnableWindow(
        GetDlgItem(hwnd, 210),
        i32::from([PAUSED, RUNNING].contains(&u.desired) && u.shown.mode == u.desired),
    );
    let yes = |v| if v { "开启" } else { "关闭" };
    let labels = [
        format!(
            "{}  · {}",
            match u.draft.backend.as_str() {
                "cpu" => crate::i18n::text("处理器"),
                "dml" => crate::i18n::text("显卡加速"),
                _ => crate::i18n::text("自动"),
            },
            crate::i18n::text("点击切换")
        ),
        format!(
            "{}  · {}",
            crate::i18n::text(match u.draft.locale.as_str() {
                "zh-CN" => "简体中文",
                "zh-TW" => "繁体中文",
                "en-US" => "英语",
                _ => "跟随客户端",
            }),
            crate::i18n::text("点击切换")
        ),
        yes(u.draft.champ_select).into(),
        yes(u.draft.champ_topmost).into(),
        if u.draft.close_to_tray {
            "收起到托盘"
        } else {
            "退出程序"
        }
        .into(),
        yes(u.draft.icon_downloads).into(),
        if u.draft.private_updates {
            "私有频道（凭据管理器）"
        } else {
            "公开频道（无令牌）"
        }
        .into(),
        yes(u.draft.pause_hotkey).into(),
        yes(u.draft.level_gate).into(),
        yes(u.draft.augment_overlay).into(),
        yes(u.draft.augment_side_panel).into(),
        "保存回退目录".into(),
    ];
    for (i, label) in labels.iter().enumerate() {
        let id = [201, 202, 203, 204, 205, 206, 207, 208, 223, 224, 225, 209][i];
        let child = GetDlgItem(hwnd, id);
        SetWindowTextW(child, wide(label).as_ptr());
        ShowWindow(child, if u.settings { SW_SHOWNA } else { SW_HIDE });
        EnableWindow(
            child,
            i32::from([PAUSED, RUNNING].contains(&u.desired) && u.shown.mode == u.desired),
        );
    }
    SetWindowTextW(GetDlgItem(hwnd, SETTINGS as i32), wide("偏好设置").as_ptr());
    let page = if u.settings {
        1
    } else if u.records {
        if !u.history {
            2
        } else if u.journal {
            3
        } else {
            4
        }
    } else {
        0
    };
    if u.layout_page.replace(page) != page {
        SendMessageW(hwnd, WM_SIZE, 0, 0);
    }
}
unsafe fn draw(hwnd: HWND, dc: HDC, u: &mut Ui) {
    workbench::shell(hwnd, dc, u);
    let saved = SaveDC(dc);
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    IntersectClipRect(dc, workbench::rail(hwnd), 0, client.right, client.bottom);
    SetViewportOrgEx(dc, workbench::rail(hwnd), -u.scroll, null_mut());
    workbench::page(hwnd, dc, u);
    if u.records {
        draw_records(hwnd, dc, u);
    }
    RestoreDC(dc, saved);
    u.paints += 1;
}
unsafe fn draw_records(hwnd: HWND, dc: HDC, u: &mut Ui) {
    let r = workbench::bounds(hwnd, u);
    let scale = workbench::dpi(hwnd);
    let x = |v: i32| v * scale / 96;
    let y = |v: i32| v * scale / 96;
    let rect = |l, t, rr, b| RECT {
        left: x(l),
        top: y(t),
        right: rr,
        bottom: y(b),
    };
    let gold = rgb(205, 174, 108);
    let pale = rgb(221, 232, 233);
    let dim = rgb(151, 176, 187);
    if u.records {
        if u.history {
            let h = &u.shown.history;
            text(
                dc,
                "账号战绩",
                rect(28, 105, r.right - x(28), 145),
                x(24),
                gold,
            );
            text(
                dc,
                "暂停时手动查询客户端最近 50 场，仅保留海克斯大乱斗。",
                rect(28, 150, r.right - x(28), 178),
                x(13),
                dim,
            );
            text(
                dc,
                u.shown
                    .notice
                    .unwrap_or("分享图片不含账号、区服、时间与对局标识。"),
                rect(342, 180, r.right - x(28), 208),
                x(13),
                dim,
            );
            let state = if h.pending > 0 {
                format!(
                    "{}: {} / {}",
                    crate::i18n::text("匹配账号导入预览"),
                    h.platform,
                    h.pending
                )
            } else if [
                HISTORY,
                HISTORY_MORE,
                HISTORY_PREVIEW,
                HISTORY_CONFIRM,
                JOURNAL_PREVIEW,
            ]
            .contains(&u.desired)
            {
                crate::i18n::text("正在核实账号与战绩…").into()
            } else {
                format!(
                    "{} / {} | {} {} | {}",
                    h.platform,
                    h.account,
                    crate::i18n::text(if h.cached {
                        "缓存快照"
                    } else {
                        "最近查询快照"
                    }),
                    match h.source.as_str() {
                        "sgp" => "SGP",
                        "lcu" => "LCU",
                        _ => "",
                    },
                    crate::i18n::text(crate::account_history::error_label(h.error))
                )
            };
            text(dc, &state, rect(28, 210, r.right - x(28), 239), x(14), gold);
            if !h.asset_version.is_empty() {
                text(
                    dc,
                    &format!(
                        "{} {}",
                        crate::i18n::text("当前图标词典（非历史补丁）："),
                        h.asset_version
                    ),
                    rect(28, 180, x(330), 208),
                    x(12),
                    dim,
                );
            }
            if u.journal {
                text(
                    dc,
                    "候选观察不代表实际领取；变化或再次出现不证明刷新。",
                    rect(28, 244, r.right - x(28), 273),
                    x(15),
                    gold,
                );
                if let Some(o) = u.observation() {
                    for (i, event) in o.events.iter().skip(u.journal_offset).take(5).enumerate() {
                        let cards = event.cards.map(|c| {
                            c.map(|id| {
                                h.augment_names
                                    .get(&id)
                                    .cloned()
                                    .unwrap_or_else(|| format!("#{id}"))
                            })
                            .unwrap_or("--".into())
                        });
                        let line = format!(
                            "{} {} · Lv {} · {} | {} | {} · {}{}",
                            u.journal_offset + i + 1,
                            event
                                .observed_at
                                .map(|t| format!(
                                    "{:02}:{:02}:{:02} UTC",
                                    (t / 3_600_000) % 24,
                                    (t / 60_000) % 60,
                                    (t / 1000) % 60
                                ))
                                .unwrap_or("—".into()),
                            event.level.map(|v| v.to_string()).unwrap_or("--".into()),
                            cards[0],
                            cards[1],
                            cards[2],
                            crate::i18n::text(match event.kind.as_str() {
                                "first" => "首次观察",
                                "changed" => "候选变化",
                                _ => "再次出现",
                            }),
                            if event.partial { " *" } else { "" }
                        );
                        text(
                            dc,
                            &line,
                            rect(
                                28,
                                279 + i as i32 * 27,
                                r.right - x(28),
                                304 + i as i32 * 27,
                            ),
                            x(14),
                            pale,
                        );
                    }
                    text(
                        dc,
                        &format!(
                            "{} {} · {} {} · {}",
                            crate::i18n::text("中断"),
                            o.interrupted,
                            crate::i18n::text("达到记录上限"),
                            o.capped,
                            crate::i18n::text("* 含缺失卡位；未记录的事件不补造。")
                        ),
                        rect(28, 416, r.right - x(28), 437),
                        x(12),
                        dim,
                    );
                    if let Some(metrics) = &o.recognition {
                        let n = metrics.analyses;
                        let line = format!(
                            "{} {n} · {} {} ms · OCR {} ms",
                            crate::i18n::text("已完成识别"),
                            crate::i18n::text("平均耗时"),
                            if n > 0 {
                                format!("{:.0}", metrics.total_ms / n as f64)
                            } else {
                                "—".into()
                            },
                            if n > 0 {
                                format!("{:.0}", metrics.ocr_ms / n as f64)
                            } else {
                                "—".into()
                            }
                        );
                        text(dc, &line, rect(28, 444, r.right - x(28), 472), x(12), dim);
                    }
                }
                return;
            }
            history_ui::draw(dc, hwnd, u);
            if h.rows.is_empty() && h.pending == 0 {
                text(
                    dc,
                    "暂无已验证的战绩记录。",
                    rect(28, 250, r.right - x(28), 285),
                    x(16),
                    pale,
                );
            }
            return;
        }
        text(
            dc,
            "本机识别记录",
            rect(28, 105, r.right - x(28), 145),
            x(24),
            gold,
        );
        text(
            dc,
            "只记录出现过的卡片，不代表已领取或胜负；暂停、断线可能拆成多个片段。",
            rect(28, 150, r.right - x(28), 178),
            x(13),
            dim,
        );
        text(
            dc,
            "账号战绩与本机识别记录分开保存。记录默认关闭，不含账号、玩家名称、时间或对局编号。",
            rect(28, 180, r.right - x(28), 208),
            x(13),
            dim,
        );
        if let Some(error) = u.shown.record_error {
            text(dc, error, rect(28, 210, r.right - x(28), 239), x(14), gold);
        }
        for (i, row) in u.shown.records.iter().take(7).enumerate() {
            let cards = row
                .last
                .map(|id| id.map(|id| id.to_string()).unwrap_or("--".into()));
            let line = format!(
                "#{} · {} {} · [{} | {} | {}]{}",
                row.champion,
                row.count,
                crate::i18n::text("次观察"),
                cards[0],
                cards[1],
                cards[2],
                if row.capped { " +" } else { "" }
            );
            text(
                dc,
                &line,
                rect(
                    28,
                    240 + i as i32 * 32,
                    r.right - x(28),
                    270 + i as i32 * 32,
                ),
                x(15),
                pale,
            );
        }
        if u.shown.records.is_empty() {
            text(
                dc,
                "暂无记录；开启后从下一次识别开始保存。",
                rect(28, 240, r.right - x(28), 280),
                x(16),
                pale,
            );
        }
        return;
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let u = &mut *pointer;
    crate::i18n::set(&u.shown.ui_locale);
    if u.taskbar != 0 && msg == u.taskbar {
        if u.tray {
            tray(hwnd, u.icon, false);
        }
        u.tray = tray(hwnd, u.icon, true);
        u.tray_recovered = u.tray;
        EnableWindow(GetDlgItem(hwnd, HIDE as i32), i32::from(u.tray));
        InvalidateRect(hwnd, null(), 0);
        return 0;
    }
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            draw(hwnd, dc, u);
            EndPaint(hwnd, &ps);
            0
        }
        WM_DRAWITEM => {
            workbench::button(hwnd, u, &*(l as *const DRAWITEMSTRUCT));
            1
        }
        WM_PRINTCLIENT => {
            draw(hwnd, w as HDC, u);
            0
        }
        WM_ERASEBKGND => 1,
        WM_GETMINMAXINFO => {
            let m = &mut *(l as *mut MINMAXINFO);
            let d = GetDpiForWindow(hwnd).max(96) as i32;
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info);
            m.ptMinTrackSize = POINT {
                x: (1080 * d / 96).min(info.rcWork.right - info.rcWork.left),
                y: (700 * d / 96).min(info.rcWork.bottom - info.rcWork.top),
            };
            0
        }
        WM_SIZE => {
            workbench::configure_scroll(hwnd, u);
            let r = workbench::bounds(hwnd, u);
            let d = GetDpiForWindow(hwnd).max(96) as i32;
            workbench::place(
                hwnd,
                GetDlgItem(hwnd, EXPORT_MATCH as i32),
                28 * d / 96,
                180 * d / 96,
                300 * d / 96,
                28 * d / 96,
                1,
            );
            workbench::place(
                hwnd,
                GetDlgItem(hwnd, JOURNAL_VIEW as i32),
                28 * d / 96,
                521 * d / 96,
                300 * d / 96,
                28 * d / 96,
                1,
            );
            workbench::place(
                hwnd,
                GetDlgItem(hwnd, JOURNAL_IMPORT as i32),
                580 * d / 96,
                483 * d / 96,
                (r.right - 608 * d / 96).max(1),
                32 * d / 96,
                1,
            );
            workbench::place(
                hwnd,
                GetDlgItem(hwnd, HISTORY_SAVED as i32),
                340 * d / 96,
                483 * d / 96,
                230 * d / 96,
                32 * d / 96,
                1,
            );
            for (i, id) in [HISTORY_PREV, HISTORY_NEXT].iter().enumerate() {
                workbench::place(
                    hwnd,
                    GetDlgItem(hwnd, *id as i32),
                    (28 + i as i32 * 150) * d / 96,
                    483 * d / 96,
                    140 * d / 96,
                    32 * d / 96,
                    1,
                );
            }
            for (i, id) in [
                HISTORY_PAGE,
                HISTORY_REFRESH,
                HISTORY_IMPORT,
                HISTORY_ACCEPT,
            ]
            .iter()
            .enumerate()
            {
                workbench::place(
                    hwnd,
                    GetDlgItem(hwnd, *id as i32),
                    28 * d / 96 + i as i32 * ((r.right - 56 * d / 96) / 4),
                    443 * d / 96,
                    ((r.right - 56 * d / 96) / 4 - 8 * d / 96).max(1),
                    32 * d / 96,
                    1,
                );
            }
            for i in 0..12 {
                workbench::place(
                    hwnd,
                    GetDlgItem(
                        hwnd,
                        [201, 202, 203, 204, 205, 206, 207, 208, 223, 224, 225, 209][i as usize],
                    ),
                    if i == 11 {
                        r.right / 2
                    } else {
                        28 * d / 96 + (i % 3) * (r.right / 3)
                    },
                    (if i == 11 { 418 } else { 174 + (i / 3) * 67 }) * d / 96,
                    if i == 11 {
                        r.right / 2 - 56 * d / 96
                    } else {
                        r.right / 3 - 40 * d / 96
                    },
                    38 * d / 96,
                    1,
                );
            }
            workbench::place(
                hwnd,
                GetDlgItem(hwnd, IMPORT_SETTINGS as i32),
                28 * d / 96,
                418 * d / 96,
                r.right / 2 - 56 * d / 96,
                38 * d / 96,
                1,
            );
            workbench::place(
                hwnd,
                GetDlgItem(hwnd, RECORD_TOGGLE as i32),
                28 * d / 96,
                477 * d / 96,
                r.right / 2 - 56 * d / 96,
                32 * d / 96,
                1,
            );
            workbench::place(
                hwnd,
                GetDlgItem(hwnd, 210),
                28 * d / 96,
                477 * d / 96,
                r.right - 56 * d / 96,
                28 * d / 96,
                1,
            );
            workbench::navigation(hwnd, u);
            history_ui::layout(hwnd, u);
            maintenance::layout(hwnd, u);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            workbench::click(
                hwnd,
                u,
                l as u16 as i16 as i32,
                (l >> 16) as u16 as i16 as i32,
            );
            history_ui::click(
                hwnd,
                u,
                l as u16 as i16 as i32,
                (l >> 16) as u16 as i16 as i32,
            );
            0
        }
        WM_MOUSEWHEEL => {
            workbench::scroll(
                hwnd,
                u,
                -((w >> 16) as i16 as i32) * workbench::dpi(hwnd) / 96,
            );
            0
        }
        WM_VSCROLL => {
            let mut info: SCROLLINFO = zeroed();
            info.cbSize = size_of::<SCROLLINFO>() as u32;
            info.fMask = SIF_ALL;
            GetScrollInfo(hwnd, SB_VERT, &mut info);
            let delta = match (w & 0xffff) as i32 {
                SB_LINEUP => -40,
                SB_LINEDOWN => 40,
                SB_PAGEUP => -(info.nPage as i32),
                SB_PAGEDOWN => info.nPage as i32,
                SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos - u.scroll,
                SB_TOP => -u.scroll,
                SB_BOTTOM => info.nMax - u.scroll,
                _ => 0,
            };
            workbench::scroll(hwnd, u, delta);
            0
        }
        WM_EXITSIZEMOVE => {
            u.placement.remember(hwnd);
            0
        }
        WM_DISPLAYCHANGE => {
            u.placement.recover(hwnd);
            u.placement.remember(hwnd);
            0
        }
        WM_DPICHANGED => {
            let r = &*(l as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            0
        }
        WM_HOTKEY if w == 1 => {
            if !u.smoke && [PAUSED, RUNNING].contains(&u.desired) {
                PostMessageW(hwnd, WM_COMMAND, TOGGLE, 0);
            }
            0
        }
        WM_COMMAND => {
            // Native keyboard tabbing must reveal controls below the fold.
            if (w >> 16 == BN_SETFOCUS as usize || w >> 16 == EN_SETFOCUS as usize) && l != 0 {
                let child = l as HWND;
                let mut rect: RECT = zeroed();
                GetWindowRect(child, &mut rect);
                let mut p = POINT {
                    x: rect.left,
                    y: rect.top,
                };
                ScreenToClient(hwnd, &mut p);
                let mut client: RECT = zeroed();
                GetClientRect(hwnd, &mut client);
                if p.x >= workbench::rail(hwnd) {
                    let bottom = p.y + rect.bottom - rect.top;
                    let delta = if p.y < 0 {
                        p.y - 12
                    } else if bottom > client.bottom {
                        bottom - client.bottom + 12
                    } else {
                        0
                    };
                    if delta != 0 {
                        workbench::scroll(hwnd, u, delta);
                    }
                }
                return 0;
            }
            match w & 0xffff {
                claim_ui::INSPECT | claim_ui::BEFORE | claim_ui::AFTER | claim_ui::SCENARIO => {
                    claim_ui::command(hwnd, u, w as u16 as usize)
                }
                maintenance::PREVIEW
                | maintenance::DIAGNOSTICS
                | maintenance::SCAN
                | maintenance::CLEAN => maintenance::command(hwnd, u, w as u16 as usize),
                history_ui::MORE
                    if !u.smoke
                        && u.records
                        && u.history
                        && !u.journal
                        && u.desired == PAUSED
                        && u.shown.mode == PAUSED
                        && u.shown.history.next_start.is_some() =>
                {
                    u.shared.request(HISTORY_MORE);
                }
                history_ui::COPY => history_ui::copy(hwnd, u),
                history_ui::CHAMPION | history_ui::RESULT
                    if u.records && u.history && !u.journal =>
                {
                    history_ui::filter(hwnd, u, w & 0xffff)
                }
                JOURNAL_VIEW if u.records && u.history => {
                    u.journal = !u.journal;
                    u.journal_offset = 0;
                    settings_controls(hwnd, u);
                }
                JOURNAL_IMPORT
                    if u.records
                        && u.history
                        && !u.smoke
                        && u.desired == PAUSED
                        && u.shown.mode == PAUSED =>
                {
                    if let Some(row) = u.shown.history.rows.get(u.history_offset) {
                        let game = row.game_id;
                        if let Some(path) = crate::legacy_preferences::choose(hwnd) {
                            if let Ok(mut p) = u.shared.import_path.lock() {
                                *p = Some(path);
                            }
                            if let Ok(mut g) = u.shared.import_game.lock() {
                                *g = Some(game);
                            }
                            u.shared.request(JOURNAL_PREVIEW);
                        }
                    }
                }
                HISTORY_SAVED
                    if u.records
                        && u.history
                        && !u.smoke
                        && u.desired == PAUSED
                        && u.shown.mode == PAUSED =>
                {
                    u.shared.request(HISTORY_OFFLINE);
                }
                HISTORY_PREV | HISTORY_NEXT if u.records && u.history => {
                    if u.journal {
                        u.journal_offset = if w & 0xffff == HISTORY_PREV {
                            u.journal_offset.saturating_sub(5)
                        } else {
                            (u.journal_offset + 5).min(
                                u.observation()
                                    .map(|o| o.events.len().saturating_sub(1) / 5 * 5)
                                    .unwrap_or(0),
                            )
                        };
                        settings_controls(hwnd, u);
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                    let indices = history_ui::indices(u);
                    u.history_page = if w & 0xffff == HISTORY_PREV {
                        u.history_page.saturating_sub(history_ui::PAGE)
                    } else {
                        (u.history_page + history_ui::PAGE).min(
                            indices.len().saturating_sub(1) / history_ui::PAGE * history_ui::PAGE,
                        )
                    };
                    u.history_offset = indices.get(u.history_page).copied().unwrap_or(usize::MAX);
                    u.scroll = 0;
                    SendMessageW(hwnd, WM_SIZE, 0, 0);
                    settings_controls(hwnd, u);
                }
                HISTORY_PAGE if u.records => {
                    u.history = !u.history;
                    settings_controls(hwnd, u);
                }
                HISTORY_REFRESH | HISTORY_IMPORT | HISTORY_ACCEPT
                    if u.records
                        && u.history
                        && !u.smoke
                        && u.desired == PAUSED
                        && u.shown.mode == PAUSED =>
                {
                    let id = w & 0xffff;
                    if id == HISTORY_IMPORT {
                        if let Some(path) = crate::legacy_preferences::choose(hwnd) {
                            if let Ok(mut p) = u.shared.import_path.lock() {
                                *p = Some(path);
                            }
                            u.shared.request(HISTORY_PREVIEW);
                        }
                    } else {
                        u.shared.request(if id == HISTORY_REFRESH {
                            HISTORY
                        } else {
                            HISTORY_CONFIRM
                        });
                    }
                }
                workbench::HOME => {
                    u.records = false;
                    u.settings = false;
                    u.scroll = 0;
                    settings_controls(hwnd, u);
                }
                RECORDS => {
                    u.scroll = 0;
                    u.records = true;
                    u.history = true;
                    u.settings = false;
                    settings_controls(hwnd, u);
                }
                EXPORT_MATCH
                    if !u.smoke
                        && u.records
                        && u.history
                        && u.desired == PAUSED
                        && u.shown.mode == PAUSED
                        && u.shown.history.pending == 0 =>
                {
                    if let Some(row) = u.shown.history.rows.get(u.history_offset) {
                        if let Ok(bytes) = crate::record_export::encode(
                            row,
                            u.observation(),
                            u.shown.history.cached,
                        ) {
                            if let Some(path) = crate::record_export::choose(hwnd) {
                                let bytes =
                                    if path.extension().and_then(|s| s.to_str()) == Some("png") {
                                        match crate::record_export::png(
                                            &bytes,
                                            u.shown
                                                .history
                                                .details
                                                .get(u.history_offset)
                                                .map(|d| d.as_ref()),
                                            Some(&u.history_reply),
                                        ) {
                                            Ok(bytes) => bytes,
                                            Err(_) => return 0,
                                        }
                                    } else {
                                        bytes
                                    };
                                if let Ok(mut request) = u.shared.export.lock() {
                                    *request = Some(crate::record_export::Request { path, bytes });
                                }
                                u.shared.request(EXPORT);
                            }
                        }
                    }
                }
                RECORD_TOGGLE if u.records && u.desired == PAUSED && u.shown.mode == PAUSED => {
                    let mut p = u.shown.preferences.clone();
                    p.local_records = !p.local_records;
                    if let Ok(mut pending) = u.shared.pending.lock() {
                        *pending = Some(p);
                    }
                    u.shared.request(APPLY);
                }
                IMPORT_SETTINGS
                    if u.settings && !u.smoke && u.desired == PAUSED && u.shown.mode == PAUSED =>
                {
                    if let Some(path) = crate::legacy_preferences::choose(hwnd) {
                        if let Ok(mut pending) = u.shared.import_path.lock() {
                            *pending = Some(path);
                        }
                        u.shared.request(IMPORT);
                    }
                }
                SETTINGS => {
                    u.scroll = 0;
                    u.records = false;
                    let opening = !u.settings;
                    u.settings = true;
                    if opening {
                        u.draft = u.shown.preferences.clone();
                        SetWindowTextW(
                            GetDlgItem(hwnd, 210),
                            wide(&u.draft.manual_directory).as_ptr(),
                        );
                    }
                    settings_controls(hwnd, u);
                }
                LEVEL_GATE | AUGMENT_OVERLAY | AUGMENT_SIDE
                    if u.settings
                        && [PAUSED, RUNNING].contains(&u.desired)
                        && u.shown.mode == u.desired =>
                {
                    if w & 0xffff == LEVEL_GATE {
                        u.draft.level_gate = !u.draft.level_gate;
                    } else if w & 0xffff == AUGMENT_SIDE {
                        u.draft.augment_side_panel = !u.draft.augment_side_panel;
                    } else {
                        u.draft.augment_overlay = !u.draft.augment_overlay;
                    }
                    u.apply_preference();
                    settings_controls(hwnd, u);
                }
                201..=208
                    if u.settings
                        && [PAUSED, RUNNING].contains(&u.desired)
                        && u.shown.mode == u.desired =>
                {
                    match w & 0xffff {
                        201 => {
                            u.draft.backend =
                                cycle(&u.draft.backend, &["auto", "cpu", "dml"]).into()
                        }
                        202 => {
                            u.draft.locale =
                                cycle(&u.draft.locale, &["auto", "zh-CN", "zh-TW", "en-US"]).into()
                        }
                        203 => u.draft.champ_select = !u.draft.champ_select,
                        204 => u.draft.champ_topmost = !u.draft.champ_topmost,
                        205 => u.draft.close_to_tray = !u.draft.close_to_tray,
                        206 => u.draft.icon_downloads = !u.draft.icon_downloads,
                        207 => u.draft.private_updates = !u.draft.private_updates,
                        208 => u.draft.pause_hotkey = !u.draft.pause_hotkey,
                        _ => {}
                    }
                    u.apply_preference();
                    settings_controls(hwnd, u);
                }
                SAVE_SETTINGS if u.settings => {
                    if [PAUSED, RUNNING].contains(&u.desired) && u.shown.mode == u.desired {
                        let mut directory = [0u16; 513];
                        let n = GetWindowTextW(GetDlgItem(hwnd, 210), directory.as_mut_ptr(), 513);
                        u.draft.manual_directory =
                            String::from_utf16_lossy(&directory[..n.max(0) as usize])
                                .trim()
                                .into();
                        u.apply_preference();
                        settings_controls(hwnd, u);
                    }
                }
                TOGGLE => {
                    u.maintenance.close_preview();
                    let next = if u.shared.desired.load(Ordering::Acquire) == RUNNING {
                        PAUSED
                    } else {
                        RUNNING
                    };
                    if ![PAUSED, RUNNING].contains(&u.shared.desired.load(Ordering::Acquire)) {
                        return 0;
                    }
                    if !u.smoke {
                        u.shared
                            .automatic_change
                            .store(if next == RUNNING { 2 } else { 1 }, Ordering::Release);
                    }
                    u.shared.request(next)
                }
                UPDATE_DATA => {
                    if u.shared.desired.load(Ordering::Acquire) == PAUSED {
                        u.shared.request(DATA);
                    }
                }
                UPDATE_APP => {
                    if !u.smoke
                        && u.shared.desired.load(Ordering::Acquire) == PAUSED
                        && u.shown.mode == PAUSED
                    {
                        use std::os::windows::process::CommandExt;
                        if let Ok(exe) = std::env::current_exe() {
                            if std::process::Command::new(exe.with_file_name("fuze-update.exe"))
                                .arg(if u.shown.preferences.private_updates {
                                    "--update-private"
                                } else {
                                    "--update"
                                })
                                .env("FUZE_UPDATE_LOCALE", &u.shown.ui_locale)
                                .creation_flags(0x08000000)
                                .spawn()
                                .is_ok()
                            {
                                u.shared.request(EXIT);
                            }
                        }
                    }
                }
                HIDE => {
                    if u.tray {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                }
                QUIT => {
                    if u.smoke || quit_dialog::confirm(hwnd) {
                        u.shared.request(EXIT);
                    }
                }
                SIDE if !u.smoke && u.desired == RUNNING && u.shown.playing.is_some() => {
                    u.side_open = !u.side_open || u.side.as_ref().is_some_and(|s| s.dismissed());
                    if u.side_open {
                        // Re-create only on explicit reopening; close stays dismissed this session.
                        u.side =
                            crate::champ_select::Window::side(u.side_roots.clone(), false).ok();
                    }
                    if let Some(side) = u.side.as_mut() {
                        side.update(
                            u.shown.playing.clone().filter(|_| u.side_open),
                            true,
                            false,
                            u.shown.preferences.icon_downloads,
                        );
                    }
                }
                OPEN => {
                    if !u.smoke {
                        show(hwnd);
                    }
                }
                _ => {}
            }
            // The timer must observe this transition even if background cleanup is blocked.
            if [workbench::HOME, SETTINGS, RECORDS].contains(&(w & 0xffff)) {
                SendMessageW(hwnd, WM_SIZE, 0, 0);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            u.placement.remember(hwnd);
            if u.tray && u.shown.preferences.close_to_tray {
                ShowWindow(hwnd, SW_HIDE);
            } else if u.smoke || quit_dialog::confirm(hwnd) {
                u.shared.request(EXIT);
            }
            0
        }
        TRAY => {
            match l as u32 {
                WM_LBUTTONDBLCLK => {
                    if !u.smoke {
                        show(hwnd);
                    }
                }
                WM_RBUTTONUP => {
                    let menu = CreatePopupMenu();
                    AppendMenuW(menu, MF_STRING, OPEN, wide("打开符择").as_ptr());
                    AppendMenuW(
                        menu,
                        MF_STRING
                            | if u.desired == RUNNING && u.shown.playing.is_some() {
                                0
                            } else {
                                MF_GRAYED
                            },
                        SIDE,
                        wide("显示 / 隐藏局内海克斯参考").as_ptr(),
                    );
                    AppendMenuW(
                        menu,
                        MF_STRING,
                        TOGGLE,
                        wide(if u.shared.desired.load(Ordering::Acquire) == RUNNING {
                            "暂停识别"
                        } else {
                            "开始识别"
                        })
                        .as_ptr(),
                    );
                    AppendMenuW(menu, MF_STRING, QUIT, wide("退出").as_ptr());
                    let mut p: POINT = zeroed();
                    GetCursorPos(&mut p);
                    SetForegroundWindow(hwnd);
                    let selected = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        p.x,
                        p.y,
                        0,
                        hwnd,
                        null(),
                    );
                    DestroyMenu(menu);
                    if selected != 0 {
                        PostMessageW(hwnd, WM_COMMAND, selected as usize, 0);
                    }
                    PostMessageW(hwnd, WM_NULL, 0, 0);
                }
                _ => {}
            }
            0
        }
        WM_TIMER => {
            maintenance::poll(hwnd, u);
            let request = if u.records
                && u.history
                && !u.journal
                && u.desired == PAUSED
                && (u.smoke || IsWindowVisible(hwnd) != 0)
            {
                u.shown
                    .history
                    .rows
                    .get(u.history_offset)
                    .zip(u.shown.history.details.get(u.history_offset))
                    .map(|(row, detail)| crate::champion_assets::Request {
                        version: u.shown.history.asset_version.clone(),
                        id: row.champion,
                        icons: history_ui::indices(u)
                            .into_iter()
                            .skip(
                                u.history_page
                                    + ((u.scroll * 96 / workbench::dpi(hwnd) - 374).max(0) / 154)
                                        as usize,
                            )
                            .take(4)
                            .flat_map(|i| {
                                u.shown
                                    .history
                                    .details
                                    .get(i)
                                    .into_iter()
                                    .flat_map(|d| d.lines.iter().flat_map(|l| l.icons.clone()))
                            })
                            .collect(),
                        download: u.shown.preferences.icon_downloads,
                        presentation: Some(detail.clone()),
                    })
            } else {
                None
            };
            u.history_assets.request(request);
            if let Some(mut reply) = u.history_assets.changed(&u.history_reply) {
                for (url, picture) in &u.history_reply.icons {
                    if reply.icons.len() >= 192 {
                        break;
                    }
                    reply
                        .icons
                        .entry(url.clone())
                        .or_insert_with(|| picture.clone());
                }
                u.history_reply = reply;
                InvalidateRect(hwnd, null(), 0);
            }
            let next = u.shared.status();
            let desired = u.shared.desired.load(Ordering::Acquire);
            if next != u.shown || desired != u.desired {
                if !u.smoke && next.preferences.pause_hotkey != u.shown.preferences.pause_hotkey {
                    UnregisterHotKey(hwnd, 1);
                    u.hotkey_ok = true;
                    if next.preferences.pause_hotkey {
                        u.hotkey_ok = RegisterHotKey(
                            hwnd,
                            1,
                            MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                            VK_F8 as u32,
                        ) != 0;
                    }
                }
                if next.preferences != u.shown.preferences
                    || (u.desired == APPLY && desired != APPLY)
                {
                    u.draft = next.preferences.clone();
                    if next.preferences.manual_directory != u.shown.preferences.manual_directory {
                        SetWindowTextW(
                            GetDlgItem(hwnd, 210),
                            wide(&u.draft.manual_directory).as_ptr(),
                        );
                    }
                }
                if next.history != u.shown.history {
                    u.layout_page.set(255);
                    u.history_page = 0;
                    if next.history.account != u.shown.history.account
                        || next.history.platform != u.shown.history.platform
                    {
                        u.history_champion = 0;
                        u.history_result = 0;
                    }
                    u.history_offset = 0;
                    u.journal = next.history.journal_preview;
                    u.journal_offset = 0;
                }
                let entered_game = u.shown.playing.is_none() && next.playing.is_some();
                let active = desired == RUNNING
                    || desired == APPLY && u.shared.resume_apply.load(Ordering::Acquire) == RUNNING;
                let side_changed =
                    next.preferences.augment_side_panel != u.shown.preferences.augment_side_panel;
                if side_changed && !next.preferences.augment_side_panel {
                    u.side_open = false;
                }
                if (entered_game || side_changed)
                    && next.playing.is_some()
                    && next.preferences.augment_side_panel
                    && active
                {
                    u.side_open = true;
                    if u.side.is_none() || u.side.as_ref().is_some_and(|s| s.dismissed()) {
                        u.side =
                            crate::champ_select::Window::side(u.side_roots.clone(), u.smoke).ok();
                    }
                }
                u.shown = next;
                let filtered = history_ui::indices(u);
                if !filtered.contains(&u.history_offset) {
                    u.history_offset = filtered.first().copied().unwrap_or(usize::MAX);
                }
                crate::i18n::set(&u.shown.ui_locale);
                for (id, label) in [
                    (UPDATE_DATA, "更新推荐数据"),
                    (UPDATE_APP, "软件更新"),
                    (workbench::HOME, "对局助手"),
                    (RECORDS, "战绩查询"),
                    (HIDE, "收起到托盘"),
                    (QUIT, "退出"),
                ] {
                    SetWindowTextW(GetDlgItem(hwnd, id as i32), wide(label).as_ptr());
                }
                if let Some(window) = u.champions.as_mut() {
                    window.update(
                        u.shown.selection.clone().filter(|_| active),
                        u.shown.preferences.champ_topmost,
                        u.smoke,
                        u.shown.preferences.icon_downloads,
                    );
                }
                if u.shown.playing.is_none() || !active {
                    u.side_open = false;
                }
                if let Some(side) = u.side.as_mut() {
                    side.update(
                        u.shown.playing.clone().filter(|_| u.side_open && active),
                        true,
                        u.smoke,
                        u.shown.preferences.icon_downloads,
                    );
                }
                u.desired = desired;
                settings_controls(hwnd, u);
                SetWindowTextW(
                    GetDlgItem(hwnd, TOGGLE as i32),
                    wide(if desired == RUNNING {
                        "暂停识别"
                    } else {
                        "开始识别"
                    })
                    .as_ptr(),
                );
                EnableWindow(
                    GetDlgItem(hwnd, TOGGLE as i32),
                    i32::from([PAUSED, RUNNING].contains(&desired)),
                );
                EnableWindow(
                    GetDlgItem(hwnd, UPDATE_DATA as i32),
                    i32::from(desired == PAUSED),
                );
                EnableWindow(
                    GetDlgItem(hwnd, UPDATE_APP as i32),
                    i32::from(desired == PAUSED && u.shown.mode == PAUSED),
                );
                InvalidateRect(hwnd, null(), 0);
            }
            if u.smoke {
                match u.smoke_step {
                    0 => {
                        if u.tray && u.taskbar != 0 {
                            PostMessageW(hwnd, u.taskbar, 0, 0);
                        }
                        PostMessageW(hwnd, WM_COMMAND, TOGGLE, 0);
                        u.smoke_step = 1;
                    }
                    1 if u.shown.mode == RUNNING
                        && u.champions.as_ref().is_some_and(|w| w.detail_ready()) =>
                    {
                        if let (Some(window), Ok(path)) =
                            (u.champions.as_ref(), std::env::var("FUZE_CHAMPION_PREVIEW"))
                        {
                            u.champion_preview_ok = u.shown.selection.is_some()
                                && window.render(std::path::Path::new(&path)).is_ok();
                        }
                        if let (Some(window), Ok(path)) =
                            (u.champions.as_mut(), std::env::var("FUZE_POPUP_REVIEW"))
                        {
                            u.champion_preview_ok &=
                                window.review(std::path::Path::new(&path)).is_ok();
                        }
                        if let (Some(window), Ok(path)) =
                            (u.champions.as_mut(), std::env::var("FUZE_SIDE_PREVIEW"))
                        {
                            u.champion_preview_ok &= window
                                .render_side_fixture(std::path::Path::new(&path))
                                .is_ok();
                        }
                        if let (Some(window), Ok(path)) =
                            (u.champions.as_mut(), std::env::var("FUZE_STAGE_PREVIEW"))
                        {
                            u.champion_preview_ok &= window
                                .render_stage_fixture(std::path::Path::new(&path))
                                .is_ok();
                        }
                        if let Ok(path) = std::env::var("FUZE_DESKTOP_PREVIEW") {
                            u.preview_ok = snapshot(hwnd, &path, u).is_ok();
                        }
                        if let Ok(path) = std::env::var("FUZE_WORKBENCH_REVIEW") {
                            u.preview_ok &= workbench::review(hwnd, u, &path).is_ok();
                        }
                        PostMessageW(hwnd, WM_COMMAND, TOGGLE, 0);
                        u.smoke_step = 2;
                    }
                    2 if u.shown.mode == PAUSED
                        && u.health_wait
                            .is_none_or(|t| t.elapsed() >= Duration::from_millis(3500)) =>
                    {
                        if let Ok(path) = std::env::var("FUZE_SETTINGS_PREVIEW") {
                            u.settings = true;
                            u.draft = u.shown.preferences.clone();
                            settings_controls(hwnd, u);
                            if snapshot(hwnd, &path, u).is_err() {
                                u.preview_ok = false;
                            }
                            u.settings = false;
                            settings_controls(hwnd, u);
                        }
                        if let Ok(path) = std::env::var("FUZE_RECORDS_PREVIEW") {
                            u.records = true;
                            settings_controls(hwnd, u);
                            if snapshot(hwnd, &path, u).is_err() {
                                u.preview_ok = false;
                            }
                            u.records = false;
                            settings_controls(hwnd, u);
                        }
                        if let Ok(path) = std::env::var("FUZE_HISTORY_PREVIEW") {
                            u.records = true;
                            u.history = true;
                            settings_controls(hwnd, u);
                            if snapshot(hwnd, &path, u).is_err() {
                                u.preview_ok = false;
                            }
                            if let Ok(journal) = std::env::var("FUZE_JOURNAL_PREVIEW") {
                                u.journal = true;
                                settings_controls(hwnd, u);
                                if snapshot(hwnd, &journal, u).is_err() {
                                    u.preview_ok = false;
                                }
                                u.journal = false;
                            }
                            u.records = false;
                            u.history = false;
                            settings_controls(hwnd, u);
                        }
                        u.smoke_pass = true;
                        PostMessageW(hwnd, WM_COMMAND, QUIT, 0);
                        u.smoke_step = 3;
                    }
                    _ => {}
                }
            }
            if desired == EXIT {
                let started = u.exiting.get_or_insert_with(Instant::now);
                if u.shown.done || started.elapsed() > Duration::from_secs(5) {
                    DestroyWindow(hwnd);
                }
            }
            0
        }
        WM_DESTROY => {
            u.placement.remember(hwnd);
            u.shared.request(EXIT);
            KillTimer(hwnd, 1);
            if !u.smoke {
                UnregisterHotKey(hwnd, 1);
            }
            if u.tray {
                u.tray_removed = tray(hwnd, u.icon, false);
                u.tray = false;
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
/// Normal startup follows the player's saved automatic preference; smoke never discovers games.
pub fn startup_error(error: &str) {
    let message = if error == "desktop-already-running" {
        "符择 Rust 已在运行，请从托盘打开。".into()
    } else {
        format!("符择 Rust 无法启动。请确认开发包已完整解压。\n错误分类：{error}")
    };
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(&message).as_ptr(),
            wide("符择 FUZE").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn run(config: Config, smoke: bool, managed: bool) -> Result<()> {
    // Older launchers only understand READY_1. Upgrade health is negotiated, not forced.
    let health_v2 =
        managed && (smoke || std::env::var("FUZE_MANAGED_HEALTH_PROTOCOL").as_deref() == Ok("2"));
    struct Instance(HANDLE);
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let _instance = if smoke {
        None
    } else {
        unsafe {
            let h = windows_sys::Win32::System::Threading::CreateMutexW(
                null(),
                0,
                wide("Local\\FuzeRustDesktopPreview").as_ptr(),
            );
            if h.is_null() {
                return Err("desktop-instance");
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(h);
                return Err("desktop-already-running");
            }
            Some(Instance(h))
        }
    };
    let shared = Arc::new(Shared {
        external_overlays: false,
        start_paused: false,
        automatic_change: AtomicU8::new(0),
        resume_apply: AtomicU8::new(PAUSED),
        wake: Condvar::new(),
        wait: Mutex::new(()),
        desired: AtomicU8::new(PAUSED),
        status: Mutex::new(Status::default()),
        monitor: Mutex::new(None),
        pending: Mutex::new(None),
        import_path: Mutex::new(None),
        import_game: Mutex::new(None),
        import_selection: Mutex::new(None),
        export: Mutex::new(None),
    });
    let mut champion_roots = config.roots.clone();
    if !smoke {
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            champion_roots.push(fuze_data::Root {
                path: std::path::PathBuf::from(p).join("FuzeRustPreview/client-data"),
                source: fuze_data::Source::Cache,
            });
        }
    }
    let worker_shared = shared.clone();
    let worker = thread::Builder::new()
        .name("fuze-desktop-control".into())
        .spawn(move || engine(config, worker_shared, smoke))
        .map_err(|_| "desktop-thread")?;
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let class = wide("FuzeRustDesktop");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            shared.request(EXIT);
            let _ = worker.join();
            return Err("desktop-class");
        }
        let icon = app_icon();
        let mut ui = Box::new(Ui {
            layout_page: std::cell::Cell::new(255),
            maintenance: Default::default(),
            scroll: 0,
            health_wait: (managed && smoke).then(Instant::now),
            journal: false,
            journal_offset: 0,
            history_assets: crate::champion_assets::Service::new(champion_roots.clone(), smoke)?,
            history_reply: Default::default(),
            records: false,
            history: false,
            history_offset: 0,
            history_page: 0,
            level_details: false,
            data_details: false,
            history_champion: 0,
            history_result: 0,
            placement: crate::window_memory::Memory::new("main", smoke),
            hotkey_ok: true,
            settings: false,
            draft: Default::default(),
            side: None,
            side_roots: champion_roots.clone(),
            side_open: false,
            champions: crate::champ_select::Window::new(champion_roots, smoke).ok(),
            champion_preview_ok: false,
            icon: icon.unwrap_or_else(|| LoadIconW(null_mut(), IDI_APPLICATION)),
            icon_owned: icon.is_some(),
            shared: shared.clone(),
            shown: Status::default(),
            desired: PAUSED,
            tray: false,
            tray_registered: false,
            tray_recovered: false,
            tray_removed: false,
            preview_ok: false,
            taskbar: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            exiting: None,
            paints: 0,
            smoke,
            smoke_step: 0,
            smoke_pass: false,
        });
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("符择 FUZE · 独立测试版").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VSCROLL,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1160,
            840,
            null_mut(),
            null_mut(),
            instance,
            (&mut *ui as *mut Ui).cast(),
        );
        if hwnd.is_null() {
            shared.request(EXIT);
            let _ = worker.join();
            UnregisterClassW(class.as_ptr(), instance);
            return Err("desktop-window");
        }
        ui.placement.restore(hwnd);
        for (id, label) in [
            (workbench::HOME, "\u{5bf9}\u{5c40}\u{52a9}\u{624b}"),
            (TOGGLE, "开始识别"),
            (UPDATE_DATA, "更新推荐数据"),
            (UPDATE_APP, "软件更新"),
            (SETTINGS, "偏好设置"),
            (RECORDS, "战绩查询"),
            (HIDE, "收起到托盘"),
            (QUIT, "退出"),
        ] {
            CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide(label).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32 | BS_NOTIFY as u32,
                0,
                0,
                0,
                0,
                hwnd,
                id as HMENU,
                instance,
                null(),
            );
        }
        for id in (201..=209).chain([
            EXPORT_MATCH,
            LEVEL_GATE,
            AUGMENT_OVERLAY,
            AUGMENT_SIDE,
            IMPORT_SETTINGS,
            RECORD_TOGGLE,
            HISTORY_PAGE,
            HISTORY_REFRESH,
            HISTORY_IMPORT,
            HISTORY_ACCEPT,
            HISTORY_PREV,
            HISTORY_NEXT,
            HISTORY_SAVED,
            JOURNAL_IMPORT,
            JOURNAL_VIEW,
            history_ui::CHAMPION,
            history_ui::RESULT,
            history_ui::COPY,
            history_ui::MORE,
            maintenance::PREVIEW,
            maintenance::DIAGNOSTICS,
            maintenance::SCAN,
            maintenance::CLEAN,
            claim_ui::INSPECT,
            claim_ui::BEFORE,
            claim_ui::AFTER,
            claim_ui::SCENARIO,
        ]) {
            CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_TABSTOP | BS_OWNERDRAW as u32 | BS_NOTIFY as u32,
                0,
                0,
                0,
                0,
                hwnd,
                id as HMENU,
                instance,
                null(),
            );
        }
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            0,
            0,
            0,
            0,
            hwnd,
            210 as HMENU,
            instance,
            null(),
        );
        if edit.is_null() {
            shared.request(EXIT);
            DestroyWindow(hwnd);
            return Err("desktop-directory-control");
        }
        SendMessageW(edit, EM_SETLIMITTEXT, 512, 0);
        SendMessageW(
            edit,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            0,
        );
        settings_controls(hwnd, &ui);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, ui.icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, ui.icon as isize);
        ui.tray = if smoke && std::env::var("FUZE_RUST_CI_TRAY").as_deref() != Ok("1") {
            false
        } else {
            tray(hwnd, ui.icon, true)
        };
        ui.tray_registered = ui.tray;
        EnableWindow(GetDlgItem(hwnd, HIDE as i32), i32::from(ui.tray));
        SendMessageW(hwnd, WM_SIZE, 0, 0);
        SetTimer(hwnd, 1, 250, None);
        if !smoke {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        } else {
            SendMessageW(hwnd, WM_PAINT, 0, 0);
        }
        if managed {
            use std::io::Write;
            println!(
                "{}",
                if health_v2 {
                    "FUZE_DESKTOP_READY_2"
                } else {
                    "FUZE_DESKTOP_READY_1"
                }
            );
            let _ = std::io::stdout().flush();
        }
        let health_started = Instant::now();
        let mut health_sent = false;
        let mut msg: MSG = zeroed();
        let failed = loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result <= 0 {
                break result < 0;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if health_v2
                && !health_sent
                && health_started.elapsed() >= Duration::from_secs(3)
                && !worker.is_finished()
                && shared.desired.load(Ordering::Acquire) != EXIT
            {
                use std::io::Write;
                println!("FUZE_DESKTOP_HEALTHY_2");
                let _ = std::io::stdout().flush();
                health_sent = true;
            }
        };
        if health_v2 && !health_sent {
            use std::io::Write;
            println!("FUZE_DESKTOP_CLOSED_2");
            let _ = std::io::stdout().flush();
        }
        shared.request(EXIT);
        if worker.is_finished() {
            let _ = worker.join();
        } // OS closes our Job handle on bounded process exit if IO is stuck.
        UnregisterClassW(class.as_ptr(), instance);
        if ui.icon_owned {
            DestroyIcon(ui.icon);
        }
        if failed {
            return Err("desktop-message-loop");
        }
        if smoke {
            if !ui.smoke_pass
                || !ui.shown.done
                || (std::env::var_os("FUZE_DESKTOP_PREVIEW").is_some() && !ui.preview_ok)
                || (ui.tray_registered && (!ui.tray_recovered || !ui.tray_removed))
                || ((std::env::var_os("FUZE_CHAMPION_PREVIEW").is_some()
                    || std::env::var_os("FUZE_POPUP_REVIEW").is_some())
                    && !ui.champion_preview_ok)
            {
                return Err("desktop-smoke-lifecycle");
            }
            println!(
                "{}",
                serde_json::json!({"desktop_ready":true,"pause_resume_exit":true,"game_access":false,"tray_registered":ui.tray_registered,"tray_recovered":ui.tray_recovered,"tray_removed":ui.tray_removed,"icon_loaded":ui.icon_owned,"preview_rendered":ui.preview_ok,"champion_preview":ui.champion_preview_ok})
            );
        }
    }
    Ok(())
}
