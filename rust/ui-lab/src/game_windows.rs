//! Own-window lifecycle driven by verified host state. No discovery, capture or LCU requests here.
use crate::{champion_bridge, direct::HostOverlay, lifecycle::hwnd, ChampionWindow};
use fuze_lcu::reconnect::State;
use fuze_runtime::session::Phase;
use fuze_worker_host::{champ_select::View, desktop::Status, window_memory::Memory};
use slint::{
    winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor},
    ComponentHandle,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
pub struct Panel {
    pub ui: ChampionWindow,
    details: champion_bridge::Binding,
    dismissed: Rc<Cell<bool>>,
    initial: bool,
    topmost: Option<bool>,
    previous: Option<View>,
    memory: Rc<RefCell<Memory>>,
    _debounce: Rc<slint::Timer>,
}
impl Panel {
    fn new(
        roots: &[fuze_data::Root],
        side: bool,
        offline: bool,
        store: std::sync::Arc<fuze_worker_host::stage_reference::Store>,
    ) -> Result<Self, &'static str> {
        let ui = ChampionWindow::new().map_err(|_| "champion-window")?;
        ui.set_side_mode(side);
        ui.set_offline(offline);
        ui.set_inspected_id(0);
        let details = champion_bridge::attach_shared(&ui, roots.to_vec(), store)?;
        let dismissed = Rc::new(Cell::new(false));
        let weak = ui.as_weak();
        let closed = dismissed.clone();
        ui.on_close_requested(move || {
            closed.set(true);
            if let Some(ui) = weak.upgrade() {
                ui.invoke_stage_reset();
                ui.invoke_player_action("cancel".into(), 0);
                crate::motion::refresh(ui.global::<crate::UiMotion>(), false);
                if let Some(h) = hwnd(ui.window()) {
                    unsafe {
                        ShowWindow(h, SW_HIDE);
                    }
                }
            }
        });
        let weak = ui.as_weak();
        ui.window().on_close_requested(move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_close_requested();
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
        let weak = ui.as_weak();
        ui.on_drag_window(move || {
            if let Some(ui) = weak.upgrade() {
                ui.window().with_winit_window(|w| {
                    let _ = w.drag_window();
                });
            }
        });
        let memory = Rc::new(RefCell::new(Memory::new(
            if side { "slint-side" } else { "slint-champion" },
            offline,
        )));
        let debounce = Rc::new(slint::Timer::default());
        let timer = debounce.clone();
        let saved = memory.clone();
        let weak = ui.as_weak();
        let corners = RefCell::new(crate::window_style::Corners::for_window(ui.window(), true));
        ui.window().on_winit_window_event(move |_, event| {
            if matches!(
                event,
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
            ) {
                if let Some(ui) = weak.upgrade() {
                    if let Some(handle) = hwnd(ui.window()) {
                        let mut style = corners.borrow_mut();
                        if let Some(style) = style.as_mut() {
                            style.resize(handle);
                        } else {
                            *style = Some(crate::window_style::Corners::new(handle, true));
                        }
                    }
                }
            }
            if let WindowEvent::Focused(focused) = event {
                if let Some(ui) = weak.upgrade() {
                    crate::motion::refresh(ui.global::<crate::UiMotion>(), *focused);
                }
            }
            if matches!(event, WindowEvent::Occluded(true)) {
                if let Some(ui) = weak.upgrade() {
                    crate::motion::refresh(ui.global::<crate::UiMotion>(), false);
                }
            }
            if matches!(
                event,
                WindowEvent::Moved(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. }
            ) {
                let saved = saved.clone();
                let weak = weak.clone();
                timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_millis(500),
                    move || {
                        if let Some(ui) = weak.upgrade() {
                            if let Some(h) = hwnd(ui.window()) {
                                unsafe {
                                    saved.borrow_mut().remember(h);
                                }
                            }
                        }
                    },
                );
            }
            EventResult::Propagate
        });
        Ok(Self {
            ui,
            details,
            dismissed,
            initial: true,
            topmost: None,
            previous: None,
            memory,
            _debounce: debounce,
        })
    }
    fn update(
        &mut self,
        view: Option<&View>,
        topmost: bool,
        visible: bool,
    ) -> Result<(), &'static str> {
        if view.is_none() {
            crate::motion::refresh(self.ui.global::<crate::UiMotion>(), false);
            if self.previous.take().is_some() {
                self.details.selection(&self.ui, None);
            }
            self.dismissed.set(false);
            if let Some(h) = hwnd(self.ui.window()) {
                unsafe {
                    ShowWindow(h, SW_HIDE);
                }
            }
            return Ok(());
        }
        if self.previous.as_ref() != view {
            self.details.selection(&self.ui, view);
            self.previous = view.cloned();
        }
        if self.dismissed.get() || !visible {
            return Ok(());
        }
        if self.initial {
            // Host entry sets with_active(false) for initial creation. Later shows use Win32
            // SW_SHOWNOACTIVATE directly: Winit repeat set_visible(true) can steal focus.
            self.ui.show().map_err(|_| "champion-window")?;
            if let Some(h) = hwnd(self.ui.window()) {
                unsafe {
                    self.memory.borrow().restore(h);
                }
            }
            self.initial = false;
        }
        if let Some(h) = hwnd(self.ui.window()) {
            unsafe {
                if self.topmost != Some(topmost) {
                    SetWindowPos(
                        h,
                        if topmost {
                            HWND_TOPMOST
                        } else {
                            HWND_NOTOPMOST
                        },
                        0,
                        0,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                    );
                    self.topmost = Some(topmost);
                }
                if IsWindowVisible(h) == 0 {
                    ShowWindow(h, SW_SHOWNOACTIVATE);
                }
            }
        }
        Ok(())
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        self._debounce.stop();
        if let Some(h) = hwnd(self.ui.window()) {
            unsafe {
                self.memory.borrow_mut().remember(h);
            }
        }
        let _ = self.ui.hide();
    }
}
pub struct Manager {
    stages: crate::stage_cards::Reference,
    roots: Vec<fuze_data::Root>,
    offline: bool,
    visible: bool,
    manual_side: bool,
    pub sample_champion: Option<Panel>,
    pub sample_cards: Option<HostOverlay>,
    pub champion: Option<Panel>,
    pub side: Option<Panel>,
    pub cards: Option<HostOverlay>,
}
impl Manager {
    pub fn placement(&mut self) -> Result<(), &'static str> {
        if self.cards.as_mut().is_some_and(|cards| !cards.placement()) {
            return Err("overlay-position-memory");
        }
        Ok(())
    }
    pub fn new(roots: Vec<fuze_data::Root>, offline: bool, visible: bool) -> Self {
        Self {
            stages: crate::stage_cards::Reference::new(offline),
            roots,
            offline,
            visible,
            manual_side: false,
            sample_champion: None,
            sample_cards: None,
            champion: None,
            side: None,
            cards: None,
        }
    }
    fn panel(&mut self, side: bool) -> Result<&mut Panel, &'static str> {
        let target = if side {
            &mut self.side
        } else {
            &mut self.champion
        };
        if target.is_none() {
            *target = Some(Panel::new(
                &self.roots,
                side,
                self.offline,
                self.stages.store.clone(),
            )?);
        }
        Ok(target.as_mut().unwrap())
    }
    pub fn preview(&mut self, view: Option<&View>) -> Result<(), &'static str> {
        let fallback;
        let view = if let Some(view) = view {
            view
        } else {
            fallback = fuze_worker_host::champ_select::Catalog::load(
                &self.roots,
                fuze_data::Locale::ZhCn,
            )?
            .playing(4);
            &fallback
        };
        // Separate read-only public-data window; never reuse or replace the actual
        // selection context, save its placement, or enable client writes in a preview.
        if self.sample_champion.is_none() {
            self.sample_champion = Some(Panel::new(
                &self.roots,
                false,
                true,
                std::sync::Arc::new(Default::default()),
            )?);
        }
        let p = self.sample_champion.as_mut().unwrap();
        p.dismissed.set(false);
        p.update(Some(view), true, self.visible)
    }
    pub fn preview_cards(&mut self) -> Result<(), &'static str> {
        use fuze_contract::{ActualStage, Slot, Snapshot};
        let prepared = fuze_data::prepare(
            &self.roots,
            fuze_data::Locale::ZhCn,
            fuze_data::Scope::Detail(4),
        )?;
        let cards = prepared.champion_detail(4)?.augments;
        if cards.len() < 3 {
            return Err("preview-data-unavailable");
        }
        let slots = std::array::from_fn(|i| Slot::Known {
            id: cards[i].id,
            name: cards[i].name.clone(),
            confidence: 1.,
        });
        let mut publisher = fuze_worker_host::overlay_frame::Publisher::new(
            cards,
            prepared
                .recommendations(4)?
                .into_iter()
                .filter_map(|(id, r)| r.score.map(|s| (id, s)))
                .collect(),
        );
        publisher.publish(
            Snapshot {
                schema_version: 1,
                generation: 1,
                sequence: 1,
                dataset_version: prepared.identity(),
                selection_confirmed: false,
                actual_stage: ActualStage::Unknown,
                slots,
            },
            true,
            false,
            None,
        );
        let mut overlay = HostOverlay::new(true).map_err(|_| "overlay-window")?;
        overlay
            .preview(
                publisher.current().ok_or("preview-data-unavailable")?,
                self.visible,
            )
            .map_err(|_| "overlay-window")?;
        self.sample_cards = Some(overlay);
        Ok(())
    }
    pub fn reference(&mut self, status: &Status) -> Result<(), &'static str> {
        if status.mode != 1
            || !matches!(status.connection, Some(State::Connected(Phase::InProgress)))
        {
            return Err("reference-unavailable");
        }
        let view = status.playing.as_ref().ok_or("reference-unavailable")?;
        self.manual_side = true;
        let visible = self.visible;
        let panel = self.panel(true)?;
        panel.dismissed.set(false);
        panel.update(Some(view), true, visible)
    }
    pub fn update(&mut self, status: &Status) -> Result<(), &'static str> {
        self.stages.update(status);
        let active = status.mode == 1;
        let phase = match status.connection {
            Some(State::Connected(p)) => p,
            _ => Phase::Disconnected,
        };
        let choose = (active && phase == Phase::ChampSelect && status.preferences.champ_select)
            .then_some(status.selection.as_ref())
            .flatten();
        if !active || phase != Phase::InProgress {
            self.manual_side = false;
        }
        let side = (active
            && phase == Phase::InProgress
            && (status.preferences.augment_side_panel || self.manual_side))
            .then_some(status.playing.as_ref())
            .flatten();
        // Live game surfaces always supersede previews, including after reconnect.
        if active && matches!(phase, Phase::ChampSelect | Phase::InProgress) {
            self.sample_champion = None;
            self.sample_cards = None;
        }
        if choose.is_some() || self.champion.is_some() {
            let visible = self.visible;
            let panel = self.panel(false)?;
            panel
                .details
                .options(&panel.ui, status.preferences.icon_downloads);
            panel.update(choose, status.preferences.champ_topmost, visible)?;
        }
        if side.is_some() || self.side.is_some() {
            let visible = self.visible;
            let panel = self.panel(true)?;
            panel
                .details
                .options(&panel.ui, status.preferences.icon_downloads);
            panel.update(side, true, visible)?;
        }
        let frame = (active && phase == Phase::InProgress && status.preferences.augment_overlay)
            .then_some(status.pipeline.overlay.clone())
            .flatten();
        if frame.is_some() && self.cards.is_none() {
            self.cards = Some(HostOverlay::new(self.offline).map_err(|_| "overlay-window")?);
        }
        if let Some(cards) = self.cards.as_mut() {
            cards
                .update(frame, self.visible)
                .map_err(|_| "overlay-window")?;
            let dataset = status
                .pipeline
                .overlay
                .as_ref()
                .map_or("", |f| f.snapshot.dataset_version.as_str());
            let reference = self.stages.cards(cards.ids(), dataset);
            cards.reference(reference).map_err(|_| "overlay-window")?;
        }
        Ok(())
    }
}
