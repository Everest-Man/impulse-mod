//! The user's preferences in `settings.json`, kept here alone, and the SETTINGS dialog. The accent and
//! reduce motion apply to every window.

mod folder_rows;
mod folders;
mod view;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui_kit::{App, Context, Entity};
use i18n::Language;
use serde::{Deserialize, Serialize};
use sound::Cue;

use client::{
    Registry,
    library::GameId,
    storage::{Paths, Stored},
};
pub(super) use folders::Folders;
use folders::Shown;
pub(super) use view::{Looked, SettingsView, Tabs, label, row};

use super::{
    sources::{Sources, SourcesChanged},
    ui::{
        text::Texts,
        theme::{ACCENTS, set_look},
    },
};

/// The leading item of a sidebar row.
#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lead {
    Number,
    #[default]
    Thumbnail,
}

/// Where the map and session panels sit.
#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Panels {
    /// In a row over the cheat list.
    #[default]
    Top,
    /// In a column beside it.
    Side,
}

/// The cues that play, and the WAV the user chose for a cue in place of its tone. A slider and a number fire
/// on every step of a drag, so they start off.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Sounds {
    on: BTreeSet<Cue>,
    files: BTreeMap<Cue, PathBuf>,
}

impl Default for Sounds {
    fn default() -> Self {
        Self { on: BTreeSet::from([Cue::ToggleOn, Cue::ToggleOff, Cue::Button, Cue::Selection]), files: BTreeMap::new() }
    }
}

impl Sounds {
    pub fn plays(&self, cue: Cue) -> bool {
        self.on.contains(&cue)
    }

    pub fn set(&mut self, cue: Cue, on: bool) {
        if on {
            self.on.insert(cue);
        } else {
            self.on.remove(&cue);
        }
    }

    pub fn file(&self, cue: Cue) -> Option<&Path> {
        self.files.get(&cue).map(PathBuf::as_path)
    }

    pub fn set_file(&mut self, cue: Cue, file: Option<PathBuf>) {
        match file {
            Some(file) => self.files.insert(cue, file),
            None => self.files.remove(&cue),
        };
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// A name of `ACCENTS`; an unknown one means the first.
    accent: String,
    pub lead: Lead,
    pub panels: Panels,
    pub reduce_motion: bool,
    pub game: Option<GameId>,
    /// Off, the default: the close button hides Impulse in the tray.
    pub quit_on_close: bool,
    /// Screenshots, recordings and streams show no Impulse window.
    pub hide_from_capture: bool,
    /// A shipped language's tag; `None` or an unknown one means the system's.
    pub language: Option<String>,
    /// The remote panel's server listens; off by default.
    pub remote: bool,
    /// The first-launch notice about the account data was accepted.
    pub disclaimer_seen: bool,
    /// The overlay no longer asks about an exclusive fullscreen game.
    pub skip_exclusive_warning: bool,
    /// LAUNCH no longer asks about a build newer than the trainer's.
    pub skip_unconfirmed_warning: bool,
    /// IN-GAME: the HUD draws inside the game's frame where its source can, else over the game. `None` until chosen:
    /// IN-GAME, a new install's default; a mode saved before stays.
    #[serde(skip_serializing_if = "Option::is_none")]
    overlay_in_game: Option<bool>,
    /// On, Impulse does not look for a new version by itself; CHECK NOW still does.
    pub no_update_check: bool,
    pub update_channel: updater::Channel,
    /// A release feed in place of the project's own: whoever runs it publishes what Impulse installs.
    pub update_feed: Option<String>,
    pub cues: Sounds,
}

impl Prefs {
    pub fn accent(&self) -> u32 {
        ACCENTS.iter().find(|(name, ..)| *name == self.accent).unwrap_or(&ACCENTS[0]).1
    }

    pub fn set_accent(&mut self, color: u32) {
        self.accent = ACCENTS.iter().find(|(_, other, _)| *other == color).map_or_else(String::new, |(name, ..)| (*name).to_owned());
    }

    pub fn overlay_in_game(&self) -> bool {
        self.overlay_in_game.unwrap_or(true)
    }

    pub fn set_overlay_in_game(&mut self, on: bool) {
        self.overlay_in_game = Some(on);
    }

    /// The language the user picked, if it ships.
    pub fn chosen(&self) -> Option<Language> {
        self.language.as_deref().and_then(Language::find)
    }

    pub fn language(&self) -> Language {
        self.chosen().unwrap_or_else(Language::system)
    }
}

pub struct Settings {
    prefs: Stored<Prefs>,
    /// The sources' catalogs, for each language's `Texts`.
    registry: Arc<Registry>,
}

impl Settings {
    pub fn new(paths: &Paths, registry: Arc<Registry>, sources: &Entity<Sources>, cx: &mut Context<Self>) -> Self {
        // A source's catalogs come and go with it; the language stays.
        cx.subscribe(sources, |settings, _, _: &SourcesChanged, cx| {
            cx.set_global(Texts::new(settings.prefs.language(), &settings.registry.texts()));
            cx.refresh_windows();
        })
        .detach();
        let settings = Self { prefs: Stored::load(paths.settings()), registry };
        settings.apply(cx);
        settings
    }

    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }

    /// Changes the prefs and saves them.
    pub fn edit(&mut self, change: impl FnOnce(&mut Prefs), cx: &mut Context<Self>) {
        self.prefs.edit(change);
        self.apply(cx);
        cx.notify();
    }

    fn apply(&self, cx: &mut App) {
        let (accent, language) = (self.prefs.accent(), self.prefs.language());
        let look = set_look(cx, accent, language);
        let texts = cx.try_global::<Texts>().is_none_or(|texts| texts.language() != language);
        if texts {
            cx.set_global(Texts::new(language, &self.registry.texts()));
            gpui_kit::component::set_locale(language.tag);
        }
        if look || texts {
            // Cached views draw again only when refreshed.
            cx.refresh_windows();
        }
        cx.set_reduce_motion(self.prefs.reduce_motion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_and_unknown_accents_mean_the_defaults() {
        let prefs: Prefs = serde_json::from_str(r#"{"accent":"PINK","lead":"number"}"#).unwrap();
        assert!(prefs.accent() == ACCENTS[0].1 && prefs.lead == Lead::Number && prefs.panels == Panels::Top);
        let mut cyan: Prefs = serde_json::from_str(r#"{"accent":"CYAN"}"#).unwrap();
        assert!(cyan.accent() == ACCENTS[2].1 && cyan.lead == Lead::Thumbnail);
        cyan.set_accent(ACCENTS[1].1);
        assert_eq!(cyan.accent, "AMBER");
    }

    #[test]
    fn sounds_start_on_for_the_cues_that_do_not_repeat_and_use_their_own_tone() {
        let prefs: Prefs = serde_json::from_str("{}").unwrap();
        let plays = Cue::ALL.map(|cue| prefs.cues.plays(cue));
        assert_eq!(plays, [true, true, false, false, true, true]);
        assert!(Cue::ALL.iter().all(|&cue| prefs.cues.file(cue).is_none()));
        assert!(!prefs.disclaimer_seen && !prefs.skip_exclusive_warning);
    }

    #[test]
    fn a_saved_choice_of_cues_replaces_the_defaults_and_keeps_its_files() {
        let json = r#"{"cues":{"on":["slider"],"files":{"slider":"C:\\clicks\\tick.wav"}}}"#;
        let mut prefs: Prefs = serde_json::from_str(json).unwrap();
        assert!(prefs.cues.plays(Cue::Slider) && !prefs.cues.plays(Cue::ToggleOn));
        assert_eq!(prefs.cues.file(Cue::Slider), Some(Path::new(r"C:\clicks\tick.wav")));
        prefs.cues.set_file(Cue::Slider, None);
        prefs.cues.set(Cue::Slider, false);
        assert_eq!(serde_json::to_string(&prefs.cues).unwrap(), r#"{"on":[],"files":{}}"#);
    }

    #[test]
    fn a_new_install_draws_the_overlay_in_the_game_and_a_saved_mode_stays() {
        assert!(Prefs::default().overlay_in_game());
        assert!(!serde_json::to_string(&Prefs::default()).unwrap().contains("overlay_in_game"));
        let mut over: Prefs = serde_json::from_str(r#"{"overlay_in_game":false}"#).unwrap();
        assert!(!over.overlay_in_game());
        over.set_overlay_in_game(true);
        assert!(serde_json::to_string(&over).unwrap().contains(r#""overlay_in_game":true"#));
    }

    #[test]
    fn the_array_an_older_build_saved_as_sounds_is_ignored() {
        let prefs: Prefs = serde_json::from_str(r#"{"sounds":["toggle"]}"#).unwrap();
        assert!(prefs.cues.plays(Cue::Button));
    }
}
