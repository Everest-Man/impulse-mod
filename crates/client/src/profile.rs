//! `games.json`: what the user keeps about a game, the star and the play time, and about its trainer, the mods a
//! run starts with, their collections, the pins and the folded sections.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::LazyLock,
};

use model::Ref;
use serde::{Deserialize, Serialize};

use crate::{
    library::GameId,
    storage::{Paths, Stored},
};

/// Cheat values by cheat id.
pub type Values = BTreeMap<String, f64>;
/// A collection name's most characters.
pub const NAME_LENGTH: usize = 24;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    /// A key of `collections::ICONS`.
    pub icon: String,
    pub mods: Values,
}

impl Collection {
    /// `mods` named `name`, trimmed and cut at `NAME_LENGTH`; `None` without a name or a mod.
    pub fn new(name: &str, icon: &str, mods: Values) -> Option<Self> {
        let name: String = name.trim().chars().take(NAME_LENGTH).collect();
        (!name.is_empty() && !mods.is_empty()).then(|| Self { name, icon: icon.to_owned(), mods })
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GameProfile {
    pub favorite: bool,
    /// Runs that went live.
    pub sessions: u32,
    /// Seconds live: in all runs, and in the last one.
    pub played: u64,
    pub last: u64,
    /// The overlay drawn inside this game (IN-GAME) or not (OVER THE GAME); `None`: the app's choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_in_game: Option<bool>,
    /// Game units added to the height of a map's teleports (TELEPORT HEIGHT); `None` is 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teleport_lift: Option<f64>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainerProfile {
    /// A run starts with `values` set.
    pub save_mods: bool,
    /// What the last run left on.
    pub values: Values,
    pub collections: Vec<Collection>,
    /// Cheat ids; `None` until the user pins or unpins: the source's pins.
    pub pinned: Option<BTreeSet<String>>,
    /// Section ids.
    pub folded: BTreeSet<String>,
    /// v1's folded sections, by printed title: a sheet turns the ones it has into ids.
    pub folded_labels: BTreeSet<String>,
}

impl TrainerProfile {
    /// A trainer's before the user keeps anything.
    pub fn blank() -> &'static Self {
        &BLANK_TRAINER
    }
}

static BLANK_GAME: LazyLock<GameProfile> = LazyLock::new(GameProfile::default);

static BLANK_TRAINER: TrainerProfile = TrainerProfile {
    save_mods: false,
    values: Values::new(),
    collections: Vec::new(),
    pinned: None,
    folded: BTreeSet::new(),
    folded_labels: BTreeSet::new(),
};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Kept {
    games: HashMap<GameId, GameProfile>,
    #[serde(with = "crate::storage::by_ref")]
    trainers: HashMap<Ref, TrainerProfile>,
}

pub struct Profiles {
    kept: Stored<Kept>,
}

impl Profiles {
    pub fn load(paths: &Paths) -> Self {
        Self { kept: Stored::load(paths.games()) }
    }

    pub fn game(&self, game: &GameId) -> &GameProfile {
        self.kept.games.get(game).unwrap_or(&BLANK_GAME)
    }

    pub fn trainer(&self, trainer: &Ref) -> &TrainerProfile {
        self.kept.trainers.get(trainer).unwrap_or(&BLANK_TRAINER)
    }

    /// The games the user starred, installed or not, in no order.
    pub fn favorites(&self) -> impl Iterator<Item = &GameId> {
        self.kept.games.iter().filter(|(_, profile)| profile.favorite).map(|(id, _)| id)
    }

    /// Changes a game's profile and saves.
    pub fn edit_game<R>(&mut self, game: &GameId, change: impl FnOnce(&mut GameProfile) -> R) -> R {
        self.kept.edit(|kept| change(kept.games.entry(game.clone()).or_default()))
    }

    /// Changes a trainer's profile and saves.
    pub fn edit_trainer<R>(&mut self, trainer: &Ref, change: impl FnOnce(&mut TrainerProfile) -> R) -> R {
        self.kept.edit(|kept| change(kept.trainers.entry(trainer.clone()).or_default()))
    }

    /// Changes both, as a run does, and saves once.
    pub fn edit_run<R>(&mut self, game: &GameId, trainer: &Ref, change: impl FnOnce(&mut GameProfile, &mut TrainerProfile) -> R) -> R {
        self.kept.edit(|kept| change(kept.games.entry(game.clone()).or_default(), kept.trainers.entry(trainer.clone()).or_default()))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use model::SourceId;

    use super::*;
    use crate::storage::migrate;

    #[test]
    fn a_v1_profile_splits_into_its_game_and_trainer_and_survives_a_reload() {
        let dir = std::env::temp_dir().join(format!("impulse-profiles-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let paths = Paths::new(dir.clone());
        fs::write(paths.games(), br#"{"old":{"favorite":true,"save_mods":true,"folded":["PLAYER"]}}"#).unwrap();
        migrate(&paths, &SourceId::new("s").unwrap());
        let (old, trainer) = (GameId::of(&Ref::parse("s:old").unwrap()), Ref::parse("s:old").unwrap());
        let mut profiles = Profiles::load(&paths);
        assert!(profiles.game(&old).favorite && profiles.trainer(&trainer).save_mods);
        assert!(profiles.trainer(&trainer).folded_labels.contains("PLAYER") && profiles.trainer(&trainer).pinned.is_none());
        let new = GameId::of(&Ref::parse("s:new").unwrap());
        assert!(!profiles.game(&new).favorite);
        profiles.edit_run(&new, &trainer, |game, trainer| {
            game.sessions += 1;
            trainer.collections.push(Collection {
                name: "Boss".to_owned(),
                icon: "skull".to_owned(),
                mods: Values::from([("a".to_owned(), 1.)]),
            });
        });
        let back = Profiles::load(&paths);
        assert!(back.game(&old).favorite && back.game(&new).sessions == 1);
        assert_eq!(back.trainer(&trainer).collections[0].mods["a"], 1.);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_game_keeps_its_own_overlay_mode_and_writes_none_without_one() {
        let earlier: GameProfile = serde_json::from_str(r#"{"favorite":true}"#).unwrap();
        assert_eq!(earlier.overlay_in_game, None);
        assert!(!serde_json::to_string(&earlier).unwrap().contains("overlay"));
        let over = GameProfile { overlay_in_game: Some(false), ..earlier };
        let back: GameProfile = serde_json::from_str(&serde_json::to_string(&over).unwrap()).unwrap();
        assert_eq!(back.overlay_in_game, Some(false));
    }
}
