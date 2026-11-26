//! TELEPORT HEIGHT: what a teleport adds to the height the map gives, kept for the open game as typed.

use gpui_kit::{
    App, Context, Entity, Focusable, IntoElement, Window,
    component::{
        h_flex,
        input::{InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px, rgb,
};

use super::{Board, Sidebar};
use crate::{
    Load,
    ui::{
        key,
        text::t,
        theme::*,
        widgets::{mono, number_field},
    },
};

/// The field, showing `lift`. A number is kept as typed: a teleport may come before the field loses the focus.
pub(super) fn lift_field(lift: f64, window: &mut Window, cx: &mut Context<Sidebar>) -> Entity<InputState> {
    let field = cx.new(|cx| InputState::new(window, cx).default_value(lift.to_string()));
    cx.subscribe_in(&field, window, |sidebar, _, event: &InputEvent, window, cx| match event {
        InputEvent::Change => sidebar.keep_lift(cx),
        InputEvent::PressEnter { .. } | InputEvent::Blur => sidebar.show_lift(window, cx),
        _ => {}
    })
    .detach();
    field
}

/// The field's row is drawn: a map that teleports, no search showing its results.
pub(super) fn lift_shown(board: &Board) -> bool {
    matches!(&board.map, Load::Ready(map) if map.teleport) && !board.searching()
}

impl Sidebar {
    /// What is not a number, a blank field too, is 0: a teleport never adds a lift the field does not show.
    fn keep_lift(&mut self, cx: &mut Context<Self>) {
        let typed = self.lift.read(cx).value().trim().replace(',', ".").parse::<f64>().ok().filter(|lift| lift.is_finite());
        let lift = typed.unwrap_or(0.);
        if lift != self.board.read(cx).lift() {
            self.maps.update(cx, |maps, cx| maps.set_lift(lift, cx));
        }
    }

    /// The board's lift in the field, unless it shows that already.
    fn show_lift(&self, window: &mut Window, cx: &mut Context<Self>) {
        let lift = self.board.read(cx).lift().to_string();
        if *self.lift.read(cx).value() != lift {
            self.lift.update(cx, |field, cx| field.set_value(lift, window, cx));
        }
    }

    /// On a board change: another game's lift shows, and the focus leaves the field once its row is gone (the map's
    /// keys would not reach a field no longer drawn).
    pub(super) fn follow_lift(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.lift.focus_handle(cx).is_focused(window) {
            self.show_lift(window, cx);
        } else if !lift_shown(self.board.read(cx)) {
            self.focus_search(window, cx);
        }
    }

    pub(super) fn lift_row(&self, cx: &App) -> impl IntoElement {
        let title = mono(t(cx, key::MAP_TELEPORT_HEIGHT), TYPE_CAPTION).text_color(rgb(DIM));
        v_flex()
            .gap(px(4.))
            .px(px(14.))
            .pb(px(12.))
            .child(h_flex().justify_between().child(title).child(number_field(&self.lift, 72., false)))
            .child(div().text_size(px(TYPE_SMALL)).text_color(rgb(FAINT)).child(t(cx, key::MAP_TELEPORT_HEIGHT_HINT)))
    }
}
