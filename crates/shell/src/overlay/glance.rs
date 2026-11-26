//! The HUD over the game while Impulse is behind it: one small window with the minimap at its dock's place on the game's
//! monitor. Shown while the game is in front and its live map is up, gone with the run.
//!
//! In the IN-GAME mode, drawn inside the game's own frame when its canvas (`InGame`) is ready: each frame the window
//! draws is mirrored into the game, and once one got there the window cloaks. Otherwise a topmost window that never
//! takes the input.

use std::{cell::Cell, mem, rc::Rc};

use anyhow::Result;
use futures::{StreamExt, channel::mpsc};
use gpui_kit::{
    App, AsyncApp, Context, Entity, IntoElement, Render, Task, WeakEntity, Window, WindowHandle, WindowKind, WindowOptions, div,
    prelude::*, px, size,
};
use model::CanvasEvent;
use topmost::Area;

use super::{
    Layer, Playing,
    docks::{self, Docks},
    hud::MINIMAP_DOCK,
    in_game::{GLANCE_LAYER, InGame, InGameEvent},
};
use crate::{
    map::{LiveMap, Minimap},
    settings::Settings,
    ui::{theme::*, widgets::brackets},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// A topmost window over the game.
    Topmost,
    /// Cloaked and out of the topmost band: its frames are drawn inside the game.
    InGame,
}

/// A window call to make between updates: on our own window it runs GPUI's callbacks at once.
enum Call {
    Open,
    /// The window's frames go into `game` at the dock's screen `rect`, as a place in its client area.
    Mirror {
        game: isize,
        rect: Area,
    },
    /// Cloaked at `rect`, out of the topmost band: its frames reach the game.
    Embed {
        hwnd: isize,
        rect: Area,
    },
    Place {
        hwnd: isize,
        rect: Area,
        uncloak: bool,
    },
    Hide {
        hwnd: isize,
        uncloak: bool,
    },
}

struct Popup {
    window: WindowHandle<Card>,
    hwnd: isize,
    placed: Option<(Mode, Area)>,
}

pub(super) struct Glance {
    docks: Entity<Docks>,
    live: Entity<LiveMap>,
    playing: Playing,
    // The live map follows the player while the HUD could show, so it shows once they are placed again.
    following: bool,
    // From screen capture.
    hidden: bool,
    hud: Option<Popup>,
    in_game: Entity<InGame>,
    // A frame of the window reached the canvas: the mirror works.
    delivered: Rc<Cell<bool>>,
    // The screen rect the window's frames go into the game for.
    mirrored: Option<Area>,
    // The game's client area may have moved or changed size: place the layer again.
    refit: bool,
    wake: mpsc::UnboundedSender<()>,
    _calls: Task<()>,
}

impl Glance {
    pub fn new(
        layer: &Entity<Layer>,
        in_game: Entity<InGame>,
        docks: Entity<Docks>,
        live: Entity<LiveMap>,
        settings: &Entity<Settings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(settings, |glance, settings, cx| {
            let hidden = settings.read(cx).prefs().hide_from_capture;
            if hidden != glance.hidden {
                glance.set_hidden(hidden);
            }
        })
        .detach();
        let hidden = settings.read(cx).prefs().hide_from_capture;
        let (wake, woken) = mpsc::unbounded();
        cx.subscribe(layer, |glance, _, playing: &Playing, _| {
            glance.playing = *playing;
            glance.refit = true;
            let _ = glance.wake.unbounded_send(());
        })
        .detach();
        // The live map comes up or goes after the game is in front.
        cx.observe(&live, |glance, _, _| {
            let _ = glance.wake.unbounded_send(());
        })
        .detach();
        cx.subscribe(&in_game, |glance, _, event: &InGameEvent, cx| {
            match event {
                // A new canvas mirrors anew; a gone one's frame callback draws nothing.
                InGameEvent::Gone => {
                    glance.delivered.set(false);
                    glance.unmirror(cx);
                }
                InGameEvent::Canvas(CanvasEvent::Ready) => {}
                InGameEvent::Canvas(CanvasEvent::Resized { .. }) => glance.refit = true,
                InGameEvent::Canvas(CanvasEvent::Input { .. }) => return,
            }
            let _ = glance.wake.unbounded_send(());
        })
        .detach();
        // The app quits once its last window closes.
        cx.on_release(|glance, cx| glance.close(cx)).detach();
        let calls = cx.spawn(async move |glance, cx| {
            let _ = calls(glance, woken, cx).await;
        });
        Self {
            docks,
            live,
            playing: Playing::Idle,
            following: false,
            hidden,
            hud: None,
            in_game,
            delivered: Rc::default(),
            mirrored: None,
            refit: false,
            wake,
            _calls: calls,
        }
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
        if let Some(hud) = &self.hud {
            topmost::exclude_from_capture(hud.hwnd, hidden);
        }
    }

    /// Settles what an update can; the rest is the next window call. The mirror first, then the mode, the rect and the
    /// visibility together: a mode switch at the same rect goes through. The window stays topmost until a frame of it
    /// reached the game.
    fn next(&mut self, cx: &mut App) -> Option<Call> {
        let due = self.docks.read(cx).is_shown(&MINIMAP_DOCK) && self.live.read(cx).ready(cx);
        let game = match self.playing {
            Playing::Idle => {
                self.close(cx);
                return None;
            }
            Playing::Front(game) if due => Some(game),
            Playing::Front(_) | Playing::Away => None,
        };
        self.follow(game.is_some(), cx);
        let want = game.filter(|_| self.live.read(cx).placed()).and_then(|game| {
            let monitor = topmost::monitor_of(game)?;
            let screen = size(px(monitor.size.0 as f32 / monitor.scale), px(monitor.size.1 as f32 / monitor.scale));
            let bounds = docks::below_header(self.docks.read(cx).bounds(&MINIMAP_DOCK, screen)).to_device_pixels(monitor.scale);
            let (left, top) = (monitor.origin.0 + bounds.origin.x.0, monitor.origin.1 + bounds.origin.y.0);
            Some((Area { left, top, width: bounds.size.width.0, height: bounds.size.height.0 }, game))
        });
        let refit = mem::take(&mut self.refit);
        let Some(hud) = &self.hud else { return want.map(|_| Call::Open) };
        let (window, hwnd, was) = (hud.window, hud.hwnd, hud.placed);
        let inside = want.filter(|_| self.in_game.read(cx).ready());
        match inside {
            Some((rect, game)) if refit || self.mirrored != Some(rect) => {
                self.mirrored = Some(rect);
                return Some(Call::Mirror { game, rect });
            }
            None if self.mirrored.is_some() => self.unmirror(cx),
            _ => {}
        }
        let mode = if inside.is_some() && self.delivered.get() { Mode::InGame } else { Mode::Topmost };
        let now = want.map(|(rect, _)| (mode, rect));
        if was == now {
            return None;
        }
        if let Some(hud) = &mut self.hud {
            hud.placed = now;
        }
        let uncloak = matches!(was, Some((Mode::InGame, _))) && mode != Mode::InGame;
        let Some((mode, rect)) = now else {
            // Its few tiles stay while the minimap is due again (the overlay closing): drawn from scratch, it blinks.
            if !(due && self.live.read(cx).placed()) {
                window.update(cx, |card, window, cx| card.minimap.update(cx, |minimap, cx| minimap.hide(window, cx))).ok();
            }
            return Some(Call::Hide { hwnd, uncloak });
        };
        Some(match mode {
            Mode::InGame => Call::Embed { hwnd, rect },
            Mode::Topmost => Call::Place { hwnd, rect, uncloak },
        })
    }

    /// The frames of the window go into the game at `at`, a place in its client area.
    fn mirror(&mut self, at: (i32, i32), cx: &mut App) {
        let (delivered, wake) = (self.delivered.clone(), self.wake.clone());
        let first = move || {
            delivered.set(true);
            let _ = wake.unbounded_send(());
        };
        let (Some(hud), Some(frames)) = (&self.hud, self.in_game.read(cx).frames(GLANCE_LAYER, at, first)) else { return };
        hud.window
            .update(cx, |_, window, _| {
                window.mirror_frames(Some(frames));
                // A first frame, so the layer shows or moves even when nothing else redraws.
                window.refresh();
            })
            .ok();
    }

    fn unmirror(&mut self, cx: &mut App) {
        self.mirrored = None;
        if let Some(hud) = &self.hud {
            hud.window.update(cx, |_, window, _| window.mirror_frames(None)).ok();
        }
        if let Some(layers) = self.in_game.read(cx).layers() {
            layers.hide(GLANCE_LAYER);
        }
    }

    fn follow(&mut self, follow: bool, cx: &mut App) {
        if follow != self.following {
            self.following = follow;
            self.live.update(cx, |live, cx| live.set_shown(follow, cx));
        }
    }

    fn close(&mut self, cx: &mut App) {
        self.follow(false, cx);
        if self.hud.is_some() {
            self.unmirror(cx);
        }
        if let Some(hud) = self.hud.take() {
            hud.window.update(cx, |_, window, _| window.remove_window()).ok();
        }
    }
}

async fn calls(glance: WeakEntity<Glance>, mut woken: mpsc::UnboundedReceiver<()>, cx: &mut AsyncApp) -> Result<()> {
    while woken.next().await.is_some() {
        while woken.try_recv().is_ok() {}
        while let Some(call) = glance.update(cx, |glance, cx| glance.next(cx))? {
            match call {
                Call::Open => {
                    let (live, hidden) = glance.read_with(cx, |glance, _| (glance.live.clone(), glance.hidden))?;
                    let hud = open(live, cx)?;
                    // Before the first show, so it never takes the input.
                    topmost::pass_through(hud.hwnd);
                    topmost::exclude_from_capture(hud.hwnd, hidden);
                    glance.update(cx, |glance, _| glance.hud = Some(hud))?;
                }
                Call::Mirror { game, rect } => {
                    // The game is gone: the window stays topmost until the next change of what plays.
                    let Some(area) = topmost::client_area(game) else { continue };
                    glance.update(cx, |glance, cx| glance.mirror(fit(rect, area), cx))?;
                }
                Call::Embed { hwnd, rect } => {
                    topmost::cloak(hwnd, true);
                    topmost::show(hwnd, rect);
                }
                Call::Place { hwnd, rect, uncloak } => {
                    topmost::place(hwnd, rect);
                    if uncloak {
                        topmost::cloak(hwnd, false);
                    }
                }
                Call::Hide { hwnd, uncloak } => {
                    topmost::hide(hwnd);
                    if uncloak {
                        topmost::cloak(hwnd, false);
                    }
                }
            }
        }
    }
    Ok(())
}

fn open(live: Entity<LiveMap>, cx: &mut AsyncApp) -> Result<Popup> {
    let options = WindowOptions {
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let window = cx.update(|cx| {
        cx.open_window(options, |_, cx| {
            let minimap = cx.new(|cx| Minimap::new(live.clone(), cx));
            cx.new(|_| Card { minimap })
        })
    })?;
    let hwnd = window.update(cx, |_, window, _| crate::hwnd(window))?.ok_or_else(|| anyhow::anyhow!("the HUD has no window handle"))?;
    Ok(Popup { window, hwnd, placed: None })
}

struct Card {
    minimap: Entity<Minimap>,
}

impl Render for Card {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().relative().size_full().border_1().border_color(line(0.14)).child(self.minimap.clone()).children(brackets(cx))
    }
}

/// Where a layer of size `rect` goes in the game's client area: the screen `rect`'s place relative to the client area's
/// origin, shifted to fit inside it (at the origin when the area is smaller).
fn fit(rect: Area, area: Area) -> (i32, i32) {
    ((rect.left - area.left).min(area.width - rect.width).max(0), (rect.top - area.top).min(area.height - rect.height).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(left: i32, top: i32, width: i32, height: i32) -> Area {
        Area { left, top, width, height }
    }

    #[test]
    fn a_layer_keeps_its_place_relative_to_the_client_area() {
        assert_eq!(fit(area(1500, 700, 300, 300), area(100, 50, 1920, 1080)), (1400, 650));
    }

    #[test]
    fn a_layer_is_shifted_back_inside_the_client_area() {
        assert_eq!(fit(area(1900, 900, 300, 300), area(0, 0, 1280, 720)), (980, 420));
        assert_eq!(fit(area(-40, -20, 300, 300), area(0, 0, 1280, 720)), (0, 0));
    }

    #[test]
    fn a_layer_larger_than_the_client_area_sits_at_its_origin() {
        assert_eq!(fit(area(500, 500, 300, 300), area(0, 0, 200, 200)), (0, 0));
    }
}
