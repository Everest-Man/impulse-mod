//! The combined page: the title bar, the installed games, the open game's pane and the status
//! bar. It makes the slices' entities and lays out their views; over a game the overlay's `Hud` takes its place.

mod disclaimer;
mod header;
mod hero;
mod signed_out;
mod status;
mod update_notice;

use std::sync::Arc;

use client::{
    Registry,
    library::{Game, GameId},
    storage::Paths,
};
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, IntoElement, KeystrokeEvent, Modifiers, Render, ScrollHandle, Window,
    component::{h_flex, scroll::ScrollableElement, v_flex},
    div,
    prelude::*,
    px, rgb,
};

use crate::{
    about::AboutView,
    account::Account,
    cheats::CheatList,
    crash,
    hotkeys::{HotkeySettings, Hotkeys, hotkeys_scope},
    input::Input,
    library::{GamePicker, Library, Marks, SIDEBAR_WIDTH, Sidebar, vacant},
    log_view::{LogEvent, LogView},
    map::{MapEvent, MapView, Maps, map_panel, paneled},
    overlay::{Entered, Hud, InGame, Layer, OverlaySettings},
    plugins::{Plugins, PluginsView},
    remote::{Remote, RemoteView, Serve},
    resources::{Resources, ResourcesEvent, ResourcesView},
    run::{Ask, Runs, session_panel},
    settings::{Folders, Looked, Panels, Settings, SettingsView, Tabs},
    sounds::{Cues, SoundsView},
    sources::Sources,
    systray::TrayIcon,
    trainer::Trainers,
    ui::{theme::*, widgets::grid},
    update::{UpdatePanel, Updates},
};
use disclaimer::Disclaimer;
use header::{header, search_box};
use hero::{Hero, banner, pinned};
use signed_out::SignedOut;
use status::status_bar;
use update_notice::{Opened, UpdateNotice};

// Below this pane width the side panels drop under the cheat list.
const WIDE_PANE: f32 = 900.;
const SIDE_WIDTH: f32 = 280.;
const SESSION_WIDTH: f32 = 300.;

pub struct Page {
    account: Entity<Account>,
    library: Entity<Library>,
    trainers: Entity<Trainers>,
    runs: Entity<Runs>,
    hotkeys: Entity<Hotkeys>,
    sidebar: Entity<Sidebar>,
    picker: Entity<GamePicker>,
    settings: Entity<Settings>,
    settings_view: Entity<SettingsView>,
    /// None in a build without the remote panel's server.
    remote_view: Option<Entity<RemoteView>>,
    /// Impulse runs as administrator.
    elevated: bool,
    _tray: Entity<TrayIcon>,
    layer: Entity<Layer>,
    // Drawn in place of the page while over a game.
    hud: Entity<Hud>,
    ask: Entity<Ask>,
    log: Entity<LogView>,
    maps: Entity<Maps>,
    map: Entity<MapView>,
    resources: Entity<Resources>,
    cheat_list: Entity<CheatList>,
    hero: Entity<Hero>,
    /// The open game's pane: the hero's bar stays over it.
    scroll: ScrollHandle,
    disclaimer: Entity<Disclaimer>,
    signed_out: Entity<SignedOut>,
    updates: Entity<Updates>,
    update_notice: Entity<UpdateNotice>,
    _cues: Entity<Cues>,
    _sources: Entity<Sources>,
}

impl Page {
    pub fn new(
        registry: Arc<Registry>,
        plugins: Arc<client::plugins::Plugins>,
        paths: Paths,
        serve: Option<Serve>,
        elevated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sources = cx.new(|cx| Sources::new(&registry, cx));
        let settings = cx.new(|cx| Settings::new(&paths, registry.clone(), &sources, cx));
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        let disclaimer = cx.new(|cx| Disclaimer::new(settings.clone(), registry.clone(), window, cx));
        cx.observe(&disclaimer, |_, _, cx| cx.notify()).detach();
        let folders = cx.new(|cx| Folders::new(registry.clone(), cx));
        let library = cx.new(|cx| Library::new(registry.clone(), &paths, settings.clone(), cx));
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        let account = cx.new(|cx| Account::new(registry.clone(), &library, cx));
        cx.observe(&account, |_, _, cx| cx.notify()).detach();
        let signed_out = cx.new(|cx| SignedOut::new(account.clone(), disclaimer.clone(), registry.clone(), window, cx));
        cx.observe(&signed_out, |_, _, cx| cx.notify()).detach();
        let trainers = cx.new(|cx| Trainers::new(registry.clone(), &paths, library.clone(), &account, cx));
        cx.observe(&trainers, |_, _, cx| cx.notify()).detach();
        let runs = cx.new(|cx| Runs::new(library.clone(), trainers.clone(), cx));
        cx.observe(&runs, |_, _, cx| cx.notify()).detach();
        crash::watch(&runs, cx);
        let log = cx.new(|cx| LogView::new(window, cx));
        cx.subscribe(&log, |_, _, _: &LogEvent, cx| cx.notify()).detach();
        let maps = cx.new(|cx| Maps::new(registry.clone(), &paths, library.clone(), runs.clone(), trainers.clone(), &sources, cx));
        // The map panel reads the open game's offers and the titles to link.
        cx.observe(&maps, |_, _, cx| cx.notify()).detach();
        let map = cx.new(|cx| MapView::new(maps.clone(), &library, &paths, window, cx));
        cx.subscribe_in(&map, window, |page, map, _: &MapEvent, window, cx| {
            // Over a game the map is a dock, and Escape's last step leaves the overlay.
            if page.layer.read(cx).is_over() {
                return page.layer.read(cx).leave();
            }
            map.update(cx, |map, cx| map.hide(window, cx));
        })
        .detach();
        let resources = cx.new(|cx| Resources::new(registry.clone(), &paths, &sources, cx));
        let plugins = cx.new(|cx| Plugins::new(plugins, cx));
        // Only a source the page reads from reloads it: a plugin's answer cache must not stop a live trainer.
        let plays = registry.clone();
        cx.subscribe(&resources, move |page, _, event: &ResourcesEvent, cx| match event {
            ResourcesEvent::Cleared(sources) if sources.iter().any(|source| plays.plays(source)) => page.reload(cx),
            ResourcesEvent::Cleared(_) => {}
        })
        .detach();
        let picker = cx.new(|cx| GamePicker::new(library.clone(), cx));
        cx.observe(&picker, |_, _, cx| cx.notify()).detach();
        let marks = {
            let (trainers, runs) = (trainers.clone(), runs.clone());
            move |game: &GameId, cx: &App| Marks {
                favorite: trainers.read(cx).game_profile(game).favorite,
                live: runs.read(cx).is_live(game),
            }
        };
        let starred = {
            let trainers = trainers.clone();
            move |cx: &App| trainers.read(cx).favorites().cloned().collect()
        };
        let sidebar = cx.new(|cx| {
            // What `marks` and `starred` read.
            cx.observe(&trainers, |_, _, cx| cx.notify()).detach();
            cx.observe(&runs, |_, _, cx| cx.notify()).detach();
            Sidebar::new(library.clone(), settings.clone(), picker.clone(), marks, starred, window, cx)
        });
        let in_game = cx.new(|cx| InGame::new(registry.clone(), &runs, &settings, &trainers, cx));
        let input = cx.new(|cx| Input::new(&runs, cx));
        let layer = cx.new(|cx| Layer::new(&paths, &runs, &settings, input.clone(), in_game.clone(), window, cx));
        let ask = cx.new(|cx| Ask::new(runs.clone(), settings.clone(), window, cx));
        cx.observe(&ask, |_, _, cx| cx.notify()).detach();
        let hero = cx.new(|cx| Hero::new(library.clone(), trainers.clone(), runs.clone(), ask.clone(), layer.clone(), picker.clone(), cx));
        let hotkeys = cx.new(|cx| Hotkeys::new(&paths, &library, trainers.clone(), runs.clone(), input, window, cx));
        cx.observe(&hotkeys, |_, _, cx| cx.notify()).detach();
        let cheat_list =
            cx.new(|cx| CheatList::new(library.clone(), trainers.clone(), runs.clone(), hotkeys.clone(), picker.clone(), window, cx));
        cx.observe(&cheat_list, |_, _, cx| cx.notify()).detach();
        let cues = cx.new(|cx| Cues::new(&runs, &trainers, &settings, cx));
        let updates = cx.new(|cx| Updates::new(settings.clone(), runs.clone(), &paths, cx));
        cx.observe(&updates, |_, _, cx| cx.notify()).detach();
        let update_notice = cx.new(|cx| UpdateNotice::new(updates.clone(), disclaimer.clone(), signed_out.clone(), window, cx));
        cx.observe(&update_notice, |_, _, cx| cx.notify()).detach();
        let update_panel = cx.new(|cx| UpdatePanel::new(updates.clone(), settings.clone(), cx));
        let tabs = Tabs {
            overlay: cx.new(|cx| OverlaySettings::new(layer.clone(), hotkeys.clone(), trainers.clone(), settings.clone(), cx)).into(),
            hotkeys: cx.new(|cx| HotkeySettings::new(hotkeys.clone(), cx)).into(),
            sounds: cx.new(|cx| SoundsView::new(settings.clone(), cues.clone(), cx)).into(),
            resources: cx.new(|cx| ResourcesView::new(resources.clone(), cx)).into(),
            plugins: cx.new(|cx| PluginsView::new(plugins.clone(), window, cx)).into(),
            about: cx.new(|_| AboutView::new(update_panel.into())).into(),
        };
        let settings_view = cx.new(|cx| SettingsView::new(settings.clone(), folders, tabs, window, cx));
        cx.observe(&settings_view, |_, _, cx| cx.notify()).detach();
        let remote_view = serve.map(|serve| {
            let owners = (library.clone(), trainers.clone(), runs.clone());
            let remote = cx.new(|cx| Remote::new(serve, settings.clone(), owners, &paths, cx));
            let view = cx.new(|cx| RemoteView::new(remote, settings.clone(), cx));
            cx.observe(&view, |_, _, cx| cx.notify()).detach();
            view
        });
        // A dialog over Settings would leave it mounted beneath.
        cx.subscribe(&update_notice, |page, _, _: &Opened, cx| page.settings_view.update(cx, |view, cx| view.close(cx))).detach();
        cx.subscribe(&settings_view, move |page, _, _: &Looked, cx| {
            page.resources.update(cx, |resources, cx| resources.measure(cx));
            plugins.update(cx, |plugins, cx| plugins.refresh(cx));
        })
        .detach();
        let hud = cx.new(|cx| {
            let (library, trainers, runs, hotkeys) = (library.clone(), trainers.clone(), runs.clone(), hotkeys.clone());
            Hud::new(
                layer.clone(),
                library,
                trainers,
                runs,
                ask.clone(),
                hotkeys,
                maps.clone(),
                map.clone(),
                cheat_list.clone(),
                picker.clone(),
                in_game,
                &settings,
                &paths,
                window,
                cx,
            )
        });
        cx.observe(&layer, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&hud, |page, _, _: &Entered, cx| {
            page.hero.update(cx, |hero, cx| hero.close(cx));
            page.picker.update(cx, |picker, cx| picker.dismiss(cx));
            page.settings_view.update(cx, |view, cx| view.close(cx));
            if let Some(remote) = &page.remote_view {
                remote.update(cx, |view, cx| view.close(cx));
            }
            page.log.update(cx, |log, cx| log.close(cx));
            cx.notify();
        })
        .detach();
        cx.observe_keystrokes(Self::search).detach();
        let tray = cx.new(|cx| TrayIcon::new(library.clone(), runs.clone(), layer.clone(), settings.clone(), window, cx));
        Self {
            hotkeys,
            sidebar,
            picker,
            account,
            library,
            trainers,
            runs,
            settings,
            settings_view,
            remote_view,
            elevated,
            _tray: tray,
            layer,
            hud,
            ask,
            log,
            maps,
            map,
            resources,
            cheat_list,
            hero,
            scroll: ScrollHandle::new(),
            disclaimer,
            signed_out,
            updates,
            update_notice,
            _cues: cues,
            _sources: sources,
        }
    }

    /// The map `Root` shows over the page.
    pub fn map(&self) -> &Entity<MapView> {
        &self.map
    }

    fn game(&self, cx: &App) -> Option<Arc<Game>> {
        self.library.read(cx).opened().cloned()
    }

    /// The open map shows over the page; over a game it is in a dock.
    pub fn map_modal(&self, cx: &App) -> bool {
        !self.layer.read(cx).is_over() && self.maps.read(cx).is_open(cx)
    }

    /// Fills the page anew after a source's caches were cleared. The session stays: it is not a cache, but it
    /// is asked for again.
    fn reload(&mut self, cx: &mut Context<Self>) {
        log::info!("caches cleared");
        self.library.update(cx, |library, cx| library.reload(cx));
        self.account.update(cx, |account, cx| account.reload(cx));
        // The run keeps what it leaves on first, from the trainers about to go.
        self.runs.update(cx, |runs, cx| runs.stop(cx));
        self.trainers.update(cx, |trainers, cx| trainers.clear(cx));
        cx.notify();
    }

    fn pane(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(game) = self.game(cx) else {
            return div().flex_1().p(px(24.)).child(vacant(self.library.read(cx), cx)).into_any_element();
        };
        let cheats = self.cheat_list.clone().into_any_element();
        let mapped = paneled(&self.map, cx);
        // A game not installed has no session.
        let installed = game.installed();
        let body = div().flex().gap(px(20.)).px(px(24.)).pt(px(20.)).pb(px(28.));
        let body = match self.settings.read(cx).prefs().panels {
            // Without a map the session takes the row alone, as a strip.
            Panels::Top if !mapped => body.flex_col().when(installed, |body| body.child(self.session(&game, true, cx))).child(cheats),
            Panels::Top => {
                let map = map_panel(&self.map, &game, cx).flex_1().min_w_0();
                let row = h_flex().items_stretch().gap(px(16.)).child(map);
                let row = row.when(installed, |row| row.child(self.session(&game, false, cx).w(px(SESSION_WIDTH)).flex_none()));
                body.flex_col().child(row).child(cheats)
            }
            Panels::Side => {
                let wide = window.viewport_size().width >= px(SIDEBAR_WIDTH + WIDE_PANE);
                let side = match wide {
                    true => v_flex().flex_none(),
                    false => h_flex().items_start(),
                };
                let side = side.gap(px(16.)).child(map_panel(&self.map, &game, cx).w(px(SIDE_WIDTH)).flex_none());
                let side = side.when(installed, |side| side.child(self.session(&game, false, cx).when(!wide, |panel| panel.flex_1())));
                body.when(!wide, |body| body.flex_col()).when(wide, |body| body.items_start()).child(cheats).child(side)
            }
        };
        let banner = banner(&game, self.trainers.read(cx), cx);
        let pane = v_flex().id("pane").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(banner).child(body.flex_none());
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(pane)
            .child(pinned(&self.hero, &self.scroll))
            .vertical_scrollbar(&self.scroll)
            .into_any_element()
    }

    /// Ctrl+K, seen whatever has the focus (a key nobody binds reaches no element then); not over the game or a
    /// dialog.
    fn search(&mut self, event: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key != "k" || keystroke.modifiers != Modifiers::control() {
            return;
        }
        let covered = self.dialogs(cx).iter().any(Option::is_some)
            || self.log.read(cx).is_open()
            || self.layer.read(cx).is_over()
            || self.map_modal(cx)
            || self.map.read(cx).linker().read(cx).is_open();
        if !covered {
            self.picker.update(cx, |picker, cx| picker.find(window, cx));
        }
    }

    /// The dialogs open over the page, bottom to top: what draws them and what they cover read this one list.
    fn dialogs(&self, cx: &App) -> [Option<AnyElement>; 8] {
        // Typed, not `AnyView`: a debug build keeps a name per `AnyView` it draws, and each editor is a new one.
        fn shown<V: Render>(view: &Entity<V>, open: bool) -> Option<AnyElement> {
            open.then(|| view.clone().into_any_element())
        }
        [
            shown(&self.settings_view, self.settings_view.read(cx).is_open()),
            self.remote_view.as_ref().and_then(|view| shown(view, view.read(cx).is_open())),
            self.cheat_list.read(cx).editor().and_then(|editor| shown(editor, true)),
            shown(&self.picker, self.picker.read(cx).is_open()),
            shown(&self.ask, self.ask.read(cx).is_open()),
            shown(&self.disclaimer, self.disclaimer.read(cx).is_open(cx)),
            shown(&self.signed_out, self.signed_out.read(cx).is_open(cx)),
            shown(&self.update_notice, self.update_notice.read(cx).is_open(cx)),
        ]
    }

    fn session(&self, game: &Game, strip: bool, cx: &App) -> Div {
        session_panel(self.runs.read(cx), self.trainers.read(cx), game, strip, hotkeys_scope(&self.hotkeys, cx), cx)
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.layer.read(cx).is_over() {
            // Entering the overlay first must not skip the disclaimer.
            return if self.disclaimer.read(cx).is_open(cx) {
                div().relative().size_full().child(self.hud.clone()).child(self.disclaimer.clone()).into_any_element()
            } else {
                self.hud.clone().into_any_element()
            };
        }
        v_flex()
            .relative()
            .size_full()
            .bg(rgb(BG))
            .child(grid(32., line(0.025)))
            .child(header(
                search_box(self.library.read(cx), &self.picker, cx),
                self.account.read(cx),
                &self.settings_view,
                self.remote_view.as_ref(),
                self.elevated,
                window,
                cx,
            ))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(self.sidebar.clone())
                    .child(self.pane(window, cx))
                    .when(self.log.read(cx).is_open(), |body| {
                        body.child(div().occlude().absolute().top_0().left_0().size_full().child(self.log.clone()))
                    }),
            )
            .child(status_bar(
                &self.log,
                &self.updates,
                self.runs.read(cx),
                self.library.read(cx),
                self.trainers.read(cx),
                self.hotkeys.read(cx),
                cx,
            ))
            .children(self.dialogs(cx).into_iter().flatten())
            .into_any_element()
    }
}
