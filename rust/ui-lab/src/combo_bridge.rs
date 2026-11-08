//! Manual local selection, shared with the three-card reference. No game actions.
use crate::{ChampionWindow, ComboPick};
use fuze_worker_host::stage_reference::{now, Store};
use slint::{ComponentHandle, Model, VecModel};
use std::{rc::Rc, sync::Arc};
pub fn refresh(ui: &ChampionWindow, store: &Store) {
    let champion = ui.get_inspected_id() as u32;
    let Ok(selection) = store.combos.lock() else {
        return;
    };
    let active = ui.get_side_mode()
        && champion > 0
        && ui.get_inspected_id() == ui.get_local_id()
        && selection.champion == champion;
    let picked = if active {
        selection.picked.as_slice()
    } else {
        &[]
    };
    ui.set_combo_picked(
        Rc::new(VecModel::from(
            picked
                .iter()
                .map(|(id, name)| ComboPick {
                    id: *id as i32,
                    name: name.clone().into(),
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    let candidate = usize::try_from(ui.get_expanded_augment())
        .ok()
        .and_then(|i| ui.get_augments().row_data(i));
    ui.set_combo_current_picked(
        candidate
            .as_ref()
            .is_some_and(|a| picked.iter().any(|(id, _)| *id == a.id as u32)),
    );
    let detail = if active {
        candidate
            .map(|a| selection.detail(store.peek(champion, now()).as_deref(), a.id as u32, now()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    ui.set_combo_detail(detail.into());
}
pub fn attach(ui: &ChampionWindow, store: Arc<Store>) {
    let weak = ui.as_weak();
    let shared = store.clone();
    ui.on_combo_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &shared);
        }
    });
    let weak = ui.as_weak();
    let shared = store.clone();
    ui.on_combo_toggle(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        if !ui.get_side_mode() || ui.get_inspected_id() != ui.get_local_id() {
            return;
        }
        let name = ui
            .get_augments()
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.title.to_string())
            .or_else(|| {
                shared
                    .combos
                    .lock()
                    .ok()?
                    .picked
                    .iter()
                    .find(|(a, _)| *a == id as u32)
                    .map(|(_, n)| n.clone())
            });
        if let Some(name) = name {
            if let Ok(mut s) = shared.combos.lock() {
                s.toggle(ui.get_inspected_id() as u32, id as u32, &name);
            }
        }
        refresh(&ui, &shared);
    });
    let weak = ui.as_weak();
    ui.on_combo_clear(move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_side_mode() {
                if let Ok(mut s) = store.combos.lock() {
                    s.clear();
                }
            }
            refresh(&ui, &store);
        }
    });
}
