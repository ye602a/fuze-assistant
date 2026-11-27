use fuze_capture::pick_pixels::{PickPixels, PickProbe};
use fuze_contract::{
    selection_pixels::Evidence, titles::pick_regions, wire::TitleKind, CardSlots, Slot,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

mod hud;
mod native;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    #[default]
    None,
    IncompleteCards,
    StaleOffer,
    WindowUnavailable,
    Interrupted,
    HudUnavailable,
    HudTimeout,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostics {
    pub owned: u16,
    pub clicks: u16,
    pub cancelled: u16,
    pub reason: Reason,
    pub observer_failed: bool,
}

const FRESH_MS: u64 = 900;
const HUD_MS: u64 = 2000;
const PANDORAS_BOX: u32 = 12317;
/// Confirmed augments still known to be owned, not the history of all choices.
pub type Owned = Arc<Vec<(u32, String)>>;

struct Offer {
    cards: CardSlots<Option<(u32, String)>>,
    visual: Option<hud::Offer>,
    screen: (u32, u32),
    at: u64,
}
impl Offer {
    fn slot(&self, x: i32, y: i32) -> Option<usize> {
        pick_regions(self.screen.0, self.screen.1, self.cards.layout())
            .ok()?
            .iter()
            .enumerate()
            .find(|(i, r)| {
                self.cards[*i].is_some()
                    && x >= r.x as i32
                    && x < (r.x + r.width) as i32
                    && y >= r.y as i32
                    && y < (r.y + r.height) as i32
            })
            .map(|(i, _)| i)
    }
}
struct Press {
    offer: Arc<Offer>,
    slot: usize,
    at: u64,
}
struct Pending {
    press: Press,
    at: u64,
}
impl Pending {
    fn card(&self) -> &(u32, String) {
        self.press.offer.cards[self.press.slot].as_ref().unwrap()
    }
    fn matches(&self, pixels: PickPixels<'_>) -> bool {
        self.press.offer.screen == pixels.screen
            && self.press.offer.visual.as_ref().is_some_and(|visual| {
                visual.confirms(self.press.slot, self.card().0 == PANDORAS_BOX, pixels.hud)
            })
    }
}
#[derive(Default)]
pub(crate) struct Tracker {
    diagnostics: Diagnostics,
    offer: Option<Arc<Offer>>,
    down: Option<Press>,
    pending: Option<Arc<Pending>>,
    owned: Owned,
    hwnd: isize,
    invalidated_at: u64,
}
impl Tracker {
    fn lost_input(&mut self, at: u64) {
        if at > self.invalidated_at {
            self.cancel();
            self.invalidated_at = at;
        }
    }
    fn clear_offer(&mut self) {
        self.offer = None;
    }
    fn cancel(&mut self) {
        let had_press = self.down.take().is_some();
        if self.pending.take().is_some() || had_press {
            self.interrupted(Reason::Interrupted);
        }
        self.clear_offer();
    }
    fn interrupted(&mut self, reason: Reason) {
        self.diagnostics.cancelled = self.diagnostics.cancelled.saturating_add(1);
        self.diagnostics.reason = reason;
    }
    fn show(
        &mut self,
        slots: &CardSlots<Slot>,
        kind: TitleKind,
        screen: (u32, u32),
        at: u64,
        visual: Option<hud::Offer>,
    ) {
        // A reroll can be clicked while an older capture is still in the OCR worker.
        if self.invalidated_at != 0 && at <= self.invalidated_at {
            return;
        }
        if kind != TitleKind::Candidate {
            self.clear_offer();
            return;
        }
        let cards = slots.each_ref().map(|s| match s {
            Slot::Known { id, name, .. } if fuze_contract::skill_choice::get(*id).is_none() => {
                Some((*id, name.clone()))
            }
            _ => None,
        });
        // A missing unrelated title must not invalidate a known clicked slot.
        if cards.iter().all(Option::is_none)
            || cards.iter().enumerate().any(|(i, c)| {
                c.as_ref()
                    .is_some_and(|c| cards[..i].iter().flatten().any(|p| p.0 == c.0))
            })
        {
            self.clear_offer();
            self.diagnostics.reason = Reason::IncompleteCards;
            return;
        }
        self.offer = Some(Arc::new(Offer {
            cards,
            visual,
            screen,
            at,
        }));
    }
    fn pointer(&mut self, down: bool, point: Option<(i32, i32)>, at: u64) {
        let Some((x, y)) = point else {
            self.cancel();
            self.invalidated_at = at;
            return;
        };
        if down {
            self.down = None;
            if let Some(offer) = &self.offer {
                if at.saturating_sub(offer.at) > FRESH_MS {
                    self.diagnostics.reason = Reason::StaleOffer;
                } else if let Some(slot) = offer.slot(x, y) {
                    // Freeze both the hit region and the visual baseline. The game
                    // may start hiding the card before the mouse-up reaches us.
                    self.down = Some(Press {
                        offer: offer.clone(),
                        slot,
                        at,
                    });
                    return;
                }
            }
        } else if let Some(press) = self.down.take() {
            if press.offer.slot(x, y) == Some(press.slot) && at.saturating_sub(press.at) <= FRESH_MS
            {
                self.diagnostics.clicks = self.diagnostics.clicks.saturating_add(1);
                if self.pending.take().is_some() {
                    self.interrupted(Reason::Interrupted);
                }
                let transforms = press.offer.cards[press.slot].as_ref().unwrap().0 == PANDORAS_BOX;
                if press
                    .offer
                    .visual
                    .as_ref()
                    .is_some_and(|v| v.can_follow(press.slot, transforms))
                {
                    self.pending = Some(Arc::new(Pending { press, at }));
                } else {
                    self.interrupted(Reason::HudUnavailable);
                }
            }
        }
        // A completed card click or a hide/reroll click retires the offer and
        // invalidates in-flight OCR. A later game click needs its own fresh offer.
        self.invalidated_at = at;
        self.clear_offer();
    }
    fn expire(&mut self, at: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| at.saturating_sub(p.at) > HUD_MS)
        {
            self.pending = None;
            self.interrupted(Reason::HudTimeout);
        }
    }
    fn capture_at(&mut self, scheduled: Option<u64>, next_capture: u64, at: u64) -> Option<u64> {
        // Also expire on duplicate/timeout frames, which never reach frame().
        self.expire(at);
        scheduled.map(|at| {
            if self.pending.is_some() {
                at.min(next_capture)
            } else {
                at
            }
        })
    }
    fn frame(&mut self, evidence: Option<Evidence>, at: u64) -> Option<Arc<Pending>> {
        if evidence != Some(Evidence::Visible) && at > self.invalidated_at {
            self.clear_offer();
        }
        self.expire(at);
        // A capture already in flight when the click arrived cannot confirm its result.
        self.pending.as_ref().filter(|p| at > p.at).cloned()
    }
    fn confirm(&mut self, pending: &Arc<Pending>) {
        if self
            .pending
            .as_ref()
            .is_none_or(|live| !Arc::ptr_eq(live, pending))
        {
            return;
        }
        self.pending = None;
        let card = pending.card();
        if card.0 == PANDORAS_BOX {
            self.owned = Owned::default();
        } else if !self.owned.iter().any(|c| c.0 == card.0) {
            Arc::make_mut(&mut self.owned).push(card.clone());
        }
        self.diagnostics.reason = Reason::None;
    }
}

#[derive(Default)]
pub struct Recorder {
    shared: Arc<Mutex<Tracker>>,
    input_lost: Arc<AtomicU64>,
    root: Option<std::path::PathBuf>,
    target: Option<fuze_capture::target::GameWindow>,
    next_discovery: u64,
    input: Option<native::Observer>,
    pub error: Option<&'static str>,
}
fn now() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}
impl Recorder {
    pub fn configure(&mut self, root: std::path::PathBuf) {
        self.root = Some(root);
    }
    pub fn diagnostics(&self) -> Diagnostics {
        let s = self.shared.lock().unwrap();
        Diagnostics {
            owned: s.owned.len().min(u16::MAX as usize) as u16,
            observer_failed: self.error.is_some(),
            ..s.diagnostics
        }
    }
    pub fn owned(&self) -> Owned {
        self.shared.lock().unwrap().owned.clone()
    }
    pub fn restore(&mut self, owned: Owned) {
        self.shared.lock().unwrap().owned = owned;
    }
    pub fn cancel(&mut self) {
        self.shared.lock().unwrap().cancel();
    }
    pub fn clear_offer(&mut self) {
        self.shared.lock().unwrap().clear_offer();
    }
    pub fn probe(&self) -> PickProbe {
        if self.root.is_none() || self.error.is_some() {
            PickProbe::Off
        } else if self.shared.lock().unwrap().pending.is_some() {
            PickProbe::Click
        } else {
            PickProbe::Offer
        }
    }
    /// Finish the observed click even if closing the popup already slowed recognition.
    /// None still means paused/disconnected/faulted; this never starts recognition.
    pub fn capture_at(&self, scheduled: Option<u64>, next_capture: u64) -> Option<u64> {
        self.shared
            .lock()
            .unwrap()
            .capture_at(scheduled, next_capture, now())
    }
    pub fn frame(
        &mut self,
        evidence: Option<Evidence>,
        pixels: Option<PickPixels<'_>>,
        captured: u64,
    ) {
        let pending = {
            let mut state = self.shared.lock().unwrap();
            state.lost_input(self.input_lost.load(Ordering::Acquire));
            state.frame(evidence, captured)
        };
        let Some(pending) = pending else { return };
        let client = self.target.as_ref().and_then(|w| w.primary_client());
        if client.map(|(_, w, h)| (w, h)) != Some(pending.press.offer.screen) {
            self.cancel();
            return;
        }
        // Keep pixel work outside the input lock. If another input cancelled
        // this attempt while we compared images, its result is now stale.
        if pixels.is_some_and(|pixels| pending.matches(pixels)) {
            let mut state = self.shared.lock().unwrap();
            state.lost_input(self.input_lost.load(Ordering::Acquire));
            state.confirm(&pending);
        }
    }
    pub fn offer(
        &mut self,
        slots: &CardSlots<Slot>,
        kind: TitleKind,
        captured: u64,
        screen: (u32, u32),
        pixels: Option<PickPixels<'_>>,
    ) {
        let at = now();
        if self.root.is_none() || self.error.is_some() || kind != TitleKind::Candidate {
            self.clear_offer();
            return;
        }
        if at.saturating_sub(captured) > FRESH_MS {
            self.clear_offer();
            self.shared.lock().unwrap().diagnostics.reason = Reason::StaleOffer;
            return;
        }
        if self.target.as_ref().is_some_and(|w| !w.valid()) {
            self.target = None;
        }
        if self.target.is_none() && at >= self.next_discovery {
            self.next_discovery = at + 5000;
            self.target = self
                .root
                .as_ref()
                .and_then(|r| fuze_capture::target::discover(r).ok().flatten());
        }
        let Some((hwnd, width, height)) = self.target.as_ref().and_then(|w| w.primary_client())
        else {
            self.cancel();
            self.shared.lock().unwrap().diagnostics.reason = Reason::WindowUnavailable;
            return;
        };
        if screen != (width, height) {
            self.cancel();
            return;
        }
        if self.input.is_none() {
            match native::Observer::start(self.shared.clone(), self.input_lost.clone()) {
                Ok(input) => self.input = Some(input),
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
        }
        let previous = self.shared.lock().unwrap().offer.clone();
        let previous = previous.as_ref().filter(|offer| offer.screen == screen);
        let visual = pixels.and_then(|pixels| {
            hud::Offer::read(
                pixels,
                slots.layout(),
                previous.and_then(|offer| offer.visual.as_ref()),
            )
        });
        let mut state = self.shared.lock().unwrap();
        state.lost_input(self.input_lost.load(Ordering::Acquire));
        state.hwnd = hwnd;
        state.show(slots, kind, screen, captured, visual);
    }
    pub(crate) fn timestamp() -> u64 {
        now()
    }
}
