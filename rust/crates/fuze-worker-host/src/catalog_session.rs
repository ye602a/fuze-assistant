//! Host-owned selection catalogue. Unknown language invalidates old content; no per-tick IO.
use crate::champ_select::{Catalog, View};
use fuze_data::{Locale, Root};
use fuze_lcu::context::Selection;
use std::time::{Duration, Instant};

pub(crate) struct Session {
    locale: Option<Locale>,
    catalog: Option<Catalog>,
    retry: Instant,
    selection: Option<Selection>,
    view: Option<View>,
}
impl Session {
    pub fn new() -> Self {
        Self {
            locale: None,
            catalog: None,
            retry: Instant::now(),
            selection: None,
            view: None,
        }
    }
    pub fn invalidate(&mut self) {
        *self = Self::new();
    }
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.view = None;
    }
    pub fn refresh(
        &mut self,
        roots: &[Root],
        locale: Option<Locale>,
        selection: Option<Selection>,
    ) {
        if self.locale != locale {
            self.invalidate();
            self.locale = locale;
        }
        if let Some(locale) = locale {
            if self.catalog.is_none() && Instant::now() >= self.retry {
                self.catalog = Catalog::load(roots, locale).ok();
                self.retry = Instant::now() + Duration::from_secs(30);
            }
        }
        if self.selection != selection || self.view.is_none() {
            self.view = selection.and_then(|s| self.catalog.as_ref().map(|c| c.view(s)));
            self.selection = selection;
        }
    }
    pub fn display_version(&self) -> Option<&str> {
        self.catalog.as_ref().map(Catalog::display_version)
    }
    pub fn identity(&self) -> Option<&str> {
        self.catalog.as_ref().map(Catalog::identity)
    }
    pub fn selection(&self) -> Option<View> {
        self.view.clone()
    }
    pub fn refresh_playing(&self, playing: &mut Option<View>, champion: Option<u32>) {
        let (Some(catalog), Some(id)) = (&self.catalog, champion) else {
            *playing = None;
            return;
        };
        // Catalog::playing puts the local hero in slot zero. The catalogue identity includes the locale.
        if playing
            .as_ref()
            .is_some_and(|view| view.team[0].id == id && view.version == catalog.identity())
        {
            return;
        }
        *playing = Some(catalog.playing(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_context_invalidates_unknown_reloads_and_bounds_io() {
        let fixture = crate::preferences::locale_tests::Fixture::new();
        fixture.dataset("zh-CN", "1.0", true);
        fixture.dataset("en-US", "2.0", true);
        let roots = fixture.config().roots;
        let selection = Selection {
            team: [1, 0, 0, 0, 0],
            local_slot: Some(0),
            bench: [0; 20],
            count: 0,
            choices: None,
        };
        let mut state = Session::new();
        state.refresh(&roots, Some(Locale::ZhCn), Some(selection));
        assert_eq!(state.selection().unwrap().version, "zh-CN@1.0");
        // Stable context and pause/resume reuse the in-memory catalogue, even without roots.
        state.clear_selection();
        state.refresh(&[], Some(Locale::ZhCn), Some(selection));
        let mut playing = None;
        state.refresh_playing(&mut playing, Some(1));
        assert_eq!(playing.as_ref().unwrap().version, "zh-CN@1.0");
        state.refresh(&roots, None, Some(selection));
        assert!(state.identity().is_none());
        assert!(state.selection().is_none());
        state.refresh_playing(&mut playing, Some(1));
        assert!(playing.is_none());
        state.refresh(&roots, Some(Locale::EnUs), Some(selection));
        assert_eq!(state.selection().unwrap().version, "en-US@2.0");
        fixture.dataset("en-US", "3.0", true);
        state.invalidate();
        state.refresh(&roots, Some(Locale::EnUs), Some(selection));
        assert_eq!(state.selection().unwrap().version, "en-US@3.0");
        state.refresh(&[], Some(Locale::ZhTw), Some(selection));
        fixture.dataset("zh-TW", "4.0", true);
        state.refresh(&roots, Some(Locale::ZhTw), Some(selection));
        assert!(state.identity().is_none()); // Failed load is not retried every host tick.
        state.retry = Instant::now();
        state.refresh(&roots, Some(Locale::ZhTw), Some(selection));
        assert_eq!(state.identity(), Some("zh-TW@4.0"));
    }

    #[test]
    fn playing_view_is_reused_until_the_hero_or_catalogue_changes() {
        let fixture = crate::preferences::locale_tests::Fixture::new();
        fixture.dataset("zh-CN", "1.0", true);
        let roots = fixture.config().roots;
        let mut session = Session::new();
        let mut playing = None;
        session.refresh(&roots, Some(Locale::ZhCn), None);
        session.refresh_playing(&mut playing, Some(1));
        let first_team = playing.as_ref().unwrap().team.as_ptr();
        session.refresh(&roots, Some(Locale::ZhCn), None);
        session.refresh_playing(&mut playing, Some(1));
        assert_eq!(playing.as_ref().unwrap().team.as_ptr(), first_team);

        session.refresh_playing(&mut playing, Some(2));
        assert_eq!(playing.as_ref().unwrap().team[0].id, 2);
        fixture.dataset("zh-CN", "2.0", true);
        session.invalidate();
        session.refresh(&roots, Some(Locale::ZhCn), None);
        session.refresh_playing(&mut playing, Some(2));
        assert_eq!(playing.as_ref().unwrap().version, "zh-CN@2.0");

        fixture.dataset("en-US", "2.0", true);
        session.refresh(&roots, Some(Locale::EnUs), None);
        session.refresh_playing(&mut playing, Some(2));
        assert_eq!(playing.as_ref().unwrap().version, "en-US@2.0");
        session.refresh_playing(&mut playing, None);
        assert!(playing.is_none());
        session.refresh_playing(&mut playing, Some(1));
        assert_eq!(playing.unwrap().team[0].id, 1);
    }
}
