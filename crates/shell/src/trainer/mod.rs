//! The trainers of the games opened in this run: the open game's loading, which copy shows while a run holds it,
//! and what the user keeps about a game (`games.json`).

mod title;

use std::{collections::HashSet, sync::Arc};

use client::{
    Load, Registry,
    library::{GameId, Support},
    profile::{Collection, GameProfile, Profiles, TrainerProfile, Values},
    run,
    storage::Paths,
    trainer::{Sheet, Shelf},
};
use futures::{StreamExt, channel::oneshot};
use gpui_kit::{App, Context, Entity, EventEmitter};
use model::{Chord, Locale, Ref, Snapshot};

use crate::{
    account::{Account, SignedIn},
    library::{Library, LibraryEvent},
    ui::text::{Texts, content_locale},
};
pub use title::title;

pub enum TrainerEvent {
    /// A copy of the game's trainer shows: the first, a newer one, or the one that waited.
    Installed(GameId),
    Rebound(GameId),
    Set {
        game: GameId,
        index: usize,
        value: f64,
    },
    /// Anything else of the game's trainer that shows: its load, pins, collections, star, values at rest.
    Changed(GameId),
}

/// While it lives the game's shown copy stays; dropping it shows the newer one that waited.
pub struct Hold {
    _release: oneshot::Sender<()>,
}

pub struct Trainers {
    registry: Arc<Registry>,
    library: Entity<Library>,
    profiles: Profiles,
    shelf: Shelf,
    /// The games' loads, by the trainer each asks for.
    loading: HashSet<(GameId, Ref)>,
    // What the copies were asked in: a new `Texts` in the same language (a source's catalogs) reloads nothing.
    locale: Locale,
}

impl EventEmitter<TrainerEvent> for Trainers {}

impl Trainers {
    pub fn new(
        registry: Arc<Registry>,
        paths: &Paths,
        library: Entity<Library>,
        account: &Entity<Account>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&library, |trainers, _, event: &LibraryEvent, cx| match event {
            // A map link is not the trainer's.
            LibraryEvent::Linked => {}
            LibraryEvent::Retrained(game) => {
                trainers.shelf.forget(game);
                trainers.open(cx);
                cx.notify();
            }
            _ => trainers.open(cx),
        })
        .detach();
        // The session is back: the open game's trainer that failed for want of it asks again.
        cx.subscribe(account, |trainers, _, _: &SignedIn, cx| trainers.open(cx)).detach();
        // Copies in the old language show until their next open asks again; the open game's asks now.
        cx.observe_global::<Texts>(|trainers, cx| {
            let locale = content_locale(cx);
            if locale != trainers.locale {
                trainers.locale = locale;
                trainers.shelf.outdate(None);
                trainers.open(cx);
            }
        })
        .detach();
        let (profiles, locale) = (Profiles::load(paths), content_locale(cx));
        Self { registry, library, profiles, shelf: Shelf::default(), loading: HashSet::new(), locale }
    }

    pub fn get(&self, game: &GameId) -> Option<&Load<Sheet>> {
        self.shelf.get(game)
    }

    pub fn sheet(&self, game: &GameId) -> Option<&Sheet> {
        self.shelf.sheet(game)
    }

    pub fn sheets(&self) -> impl Iterator<Item = &Sheet> {
        self.shelf.sheets()
    }

    pub fn is_loading(&self, game: &GameId) -> bool {
        self.loading.iter().any(|(loading, _)| loading == game)
    }

    pub fn game_profile(&self, game: &GameId) -> &GameProfile {
        self.profiles.game(game)
    }

    pub fn favorites(&self) -> impl Iterator<Item = &GameId> {
        self.profiles.favorites()
    }

    /// The shown copy's trainer profile; a blank one without a copy.
    pub fn trainer_profile(&self, game: &GameId) -> &TrainerProfile {
        self.shelf.sheet(game).map_or(TrainerProfile::blank(), |sheet| self.profiles.trainer(&sheet.trainer))
    }

    /// Changes the shown copy's trainer profile; nothing without a copy.
    fn edit_trainer(&mut self, game: &GameId, change: impl FnOnce(&mut TrainerProfile), cx: &mut Context<Self>) {
        let Some(trainer) = self.shelf.sheet(game).map(|sheet| sheet.trainer.clone()) else { return };
        self.profiles.edit_trainer(&trainer, change);
        cx.emit(TrainerEvent::Changed(game.clone()));
        cx.notify();
    }

    /// Keeps the game's shown copy until the `Hold` drops.
    pub fn hold(&mut self, game: GameId, cx: &mut Context<Self>) -> Hold {
        let (release, released) = oneshot::channel::<()>();
        self.shelf.hold(game.clone());
        cx.spawn(async move |trainers, cx| {
            let _ = released.await;
            trainers
                .update(cx, |trainers, cx| {
                    if trainers.shelf.release(&game, &trainers.profiles) {
                        trainers.installed(game.clone(), cx);
                    }
                    // A trainer chosen while held may have had no copy waiting.
                    if trainers.drop_other(&game, cx) && trainers.library.read(cx).open_id() == Some(&game) {
                        trainers.open(cx);
                    }
                })
                .ok();
        })
        .detach();
        Hold { _release: release }
    }

    /// The caches were cleared: the copies go, and the open game's loads with the next list.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.shelf.clear();
        cx.notify();
    }

    /// The hotkeys in effect, by cheat: the user's choices over the source's.
    pub fn bind(&mut self, game: &GameId, hotkeys: Vec<Vec<Option<Chord>>>, cx: &mut Context<Self>) {
        let Some(sheet) = self.shelf.sheet_mut(game) else { return };
        sheet.hotkeys = hotkeys;
        cx.emit(TrainerEvent::Rebound(game.clone()));
        cx.notify();
    }

    pub fn set(&mut self, game: &GameId, index: usize, value: f64, cx: &mut Context<Self>) {
        let Some(sheet) = self.shelf.sheet_mut(game) else { return };
        sheet.values[index] = value;
        cx.emit(TrainerEvent::Set { game: game.clone(), index, value });
        cx.notify();
    }

    /// Every cheat at rest, as a trainer starts.
    pub fn reset(&mut self, game: &GameId, cx: &mut Context<Self>) {
        if let Some(sheet) = self.shelf.sheet_mut(game) {
            sheet.reset();
            cx.emit(TrainerEvent::Changed(game.clone()));
            cx.notify();
        }
    }

    pub fn toggle_pin(&mut self, game: &GameId, index: usize, cx: &mut Context<Self>) {
        let Some(sheet) = self.shelf.sheet_mut(game) else { return };
        let trainer = sheet.trainer.clone();
        self.profiles.edit_trainer(&trainer, |profile| sheet.toggle_pin(index, profile));
        cx.emit(TrainerEvent::Changed(game.clone()));
        cx.notify();
    }

    pub fn toggle_fold(&mut self, game: &GameId, section: &str, cx: &mut Context<Self>) {
        let Some(sheet) = self.shelf.sheet_mut(game) else { return };
        let trainer = sheet.trainer.clone();
        self.profiles.edit_trainer(&trainer, |profile| sheet.toggle_fold(section, profile));
        cx.notify();
    }

    pub fn toggle_favorite(&mut self, game: &GameId, cx: &mut Context<Self>) {
        self.profiles.edit_game(game, |profile| profile.favorite = !profile.favorite);
        cx.emit(TrainerEvent::Changed(game.clone()));
        cx.notify();
    }

    /// The map's TELEPORT HEIGHT for the game, as kept (not a number is 0). Only the map shows it: no `TrainerEvent`.
    pub fn set_teleport_lift(&mut self, game: &GameId, lift: f64) -> f64 {
        let lift = if lift.is_finite() { lift } else { 0. };
        self.profiles.edit_game(game, |profile| profile.teleport_lift = (lift != 0.).then_some(lift));
        lift
    }

    /// The game's own overlay mode, IN-GAME or not; `None`: the OVERLAY tab's.
    pub fn set_overlay_in_game(&mut self, game: &GameId, in_game: Option<bool>, cx: &mut Context<Self>) {
        self.profiles.edit_game(game, |profile| profile.overlay_in_game = in_game);
        cx.emit(TrainerEvent::Changed(game.clone()));
        cx.notify();
    }

    /// Whether a run's mods come back at the next one; what was kept goes either way.
    pub fn set_save_mods(&mut self, game: &GameId, on: bool, cx: &mut Context<Self>) {
        self.edit_trainer(
            game,
            |profile| {
                profile.save_mods = on;
                profile.values.clear();
            },
            cx,
        );
    }

    pub fn forget_kept(&mut self, game: &GameId, cx: &mut Context<Self>) {
        self.edit_trainer(game, |profile| profile.values.clear(), cx);
    }

    /// Renames `was` and changes its icon, or adds `collection`. Found by value: a device may have deleted one meanwhile.
    pub fn keep_collection(&mut self, game: &GameId, was: Option<&Collection>, collection: Collection, cx: &mut Context<Self>) {
        let change = |profile: &mut TrainerProfile| match was {
            Some(was) => {
                if let Some(kept) = profile.collections.iter_mut().find(|kept| *kept == was) {
                    (kept.name, kept.icon) = (collection.name, collection.icon);
                }
            }
            None => profile.collections.push(collection),
        };
        self.edit_trainer(game, change, cx);
    }

    pub fn delete_collection(&mut self, game: &GameId, at: usize, cx: &mut Context<Self>) {
        let change = |profile: &mut TrainerProfile| {
            if at < profile.collections.len() {
                profile.collections.remove(at);
            }
        };
        self.edit_trainer(game, change, cx);
    }

    /// Counts a run that went live; the values it starts with.
    pub fn run_opened(&mut self, game: &GameId, cx: &mut Context<Self>) -> Option<Values> {
        cx.notify();
        let trainer = self.shelf.sheet(game)?.trainer.clone();
        self.profiles.edit_run(game, &trainer, run::opened)
    }

    /// Keeps a run's time and what it leaves on.
    pub fn run_closed(&mut self, game: &GameId, secs: u64, cx: &mut Context<Self>) {
        match self.shelf.sheet(game).map(|sheet| (sheet.trainer.clone(), sheet.live_values())) {
            Some((trainer, left)) => self.profiles.edit_run(game, &trainer, |game, trainer| run::closed(game, trainer, secs, Some(left))),
            // The caches were cleared: only the time is kept.
            None => self.profiles.edit_game(game, |game| run::closed(game, &mut TrainerProfile::default(), secs, None)),
        }
        cx.notify();
    }

    /// The open game's trainer. Nothing is downloaded for a game the user did not open.
    fn open(&mut self, cx: &mut Context<Self>) {
        let Some(game) = self.library.read(cx).opened().cloned() else { return };
        self.drop_other(&game.id, cx);
        let Some(offer) = game.offer() else { return };
        if game.support == Support::NoTrainer || self.shelf.is_current(&game.id, &game.stamp) {
            return;
        }
        // One task per game and trainer: a second open while it runs changes nothing.
        if !self.loading.insert((game.id.clone(), offer.id.clone())) {
            return;
        }
        let (id, stamp, trainer_id, locale) = (game.id.clone(), game.stamp.clone(), offer.id.clone(), content_locale(cx));
        let mut snapshots = self.registry.trainer(offer, game.install.as_ref().and_then(|install| install.build), &locale);
        cx.spawn(async move |trainers, cx| {
            while let Some(snapshot) = snapshots.next().await {
                trainers
                    .update(cx, |trainers, cx| {
                        // Its copies would show over the trainer chosen meanwhile.
                        if trainers.superseded(&id, &trainer_id, cx) {
                            return;
                        }
                        match snapshot {
                            Snapshot::Cached(Some(trainer)) | Snapshot::Fresh(Ok(trainer)) => {
                                let sheet = Sheet::new(trainer, trainer_id.clone(), stamp.clone(), trainers.profiles.trainer(&trainer_id));
                                if trainers.shelf.put(id.clone(), sheet) {
                                    trainers.installed(id.clone(), cx);
                                }
                            }
                            // The network is next.
                            Snapshot::Cached(None) => {
                                trainers.shelf.pending(id.clone());
                                cx.emit(TrainerEvent::Changed(id.clone()));
                            }
                            Snapshot::Fresh(Err(e)) => {
                                trainers.shelf.failed(id.clone(), e);
                                cx.emit(TrainerEvent::Changed(id.clone()));
                            }
                        }
                        cx.notify();
                    })
                    .ok();
            }
            trainers
                .update(cx, |trainers, cx| {
                    trainers.loading.remove(&(id.clone(), trainer_id.clone()));
                    // The trainer chosen meanwhile has its own load.
                    if trainers.superseded(&id, &trainer_id, cx) {
                        return;
                    }
                    cx.emit(TrainerEvent::Changed(id.clone()));
                    // The language changed while it loaded: this copy is in the old one.
                    let relocalized = content_locale(cx) != locale;
                    if relocalized {
                        trainers.shelf.outdate(Some(&id));
                    }
                    // Or the source changed: this trainer is for the old build.
                    if trainers.library.read(cx).opened().is_some_and(|game| game.id == id && (game.stamp != stamp || relocalized)) {
                        trainers.open(cx);
                    }
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    /// Another trainer than `trainer` is in use for `game` now.
    fn superseded(&self, game: &GameId, trainer: &Ref, cx: &App) -> bool {
        let library = self.library.read(cx);
        let current = library.opened().filter(|open| open.id == *game).or_else(|| library.game(game));
        current.is_some_and(|game| game.stamp.trainer.as_ref() != Some(trainer))
    }

    /// Drops `game`'s copy of a trainer it no longer uses, unless a run holds it; whether a shown one went.
    fn drop_other(&mut self, game: &GameId, cx: &mut Context<Self>) -> bool {
        let other = self.shelf.trainer(game).is_some_and(|trainer| self.superseded(game, trainer, cx));
        let dropped = other && self.shelf.forget(game);
        if dropped {
            cx.emit(TrainerEvent::Changed(game.clone()));
        }
        dropped
    }

    fn installed(&mut self, game: GameId, cx: &mut Context<Self>) {
        cx.emit(TrainerEvent::Installed(game));
        cx.notify();
    }
}
