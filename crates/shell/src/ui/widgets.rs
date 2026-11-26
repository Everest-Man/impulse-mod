use std::{cell::RefCell, time::Duration};

use gpui_kit::{
    AnyView, App, Bounds, Context, Div, ElementId, Entity, FontWeight, Hsla, InteractiveText, IntoElement, Rgba, SharedString, Stateful,
    StyledText, Subscription, Svg, Task, Window,
    base::input::{InputBaseState, InputModeKind},
    canvas,
    component::{
        h_flex,
        input::{Input, InputEvent, InputState},
        tooltip::Tooltip,
        v_flex,
    },
    div, fill, point,
    prelude::*,
    px, rgb, size, svg,
};

use rustc_hash::FxHashMap;
use unicode_segmentation::UnicodeSegmentation;

use i18n::Key;

use super::{
    assets::Icon,
    key,
    text::{Texts, t},
    theme::*,
};

const FLASH: Duration = Duration::from_millis(900);
const TIP_WIDTH: f32 = 320.;

pub fn icon(icon: Icon, size: f32, color: impl Into<Hsla>) -> Svg {
    svg().path(icon.path()).size(px(size)).flex_none().text_color(color)
}

pub fn mono(text: impl Into<SharedString>, size: f32) -> Div {
    div().font_family(MONO).font_weight(FontWeight::MEDIUM).text_size(px(size)).child(text.into())
}

/// Letter-spaced label: GPUI has no tracking, so glyphs sit in a flex row with a gap.
pub fn tracked(text: &str, size: f32, em: f32) -> Div {
    let gap = px(size * em);
    div()
        .flex()
        .flex_wrap()
        .gap_x(px(size * 0.28) + gap)
        .text_size(px(size))
        .children(text.split(' ').map(|word| div().flex().gap(gap).children(word.graphemes(true).map(letter))))
}

/// One cluster as a string with no allocation per frame: ASCII from a table, the rest made once.
fn letter(cluster: &str) -> SharedString {
    const ASCII: &[u8] = b" !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";
    thread_local!(static MADE: RefCell<FxHashMap<Box<str>, SharedString>> = RefCell::default());
    if let &[byte @ b' '..=b'~'] = cluster.as_bytes() {
        let at = usize::from(byte - b' ');
        return SharedString::new_static(str::from_utf8(&ASCII[at..=at]).unwrap_or_default());
    }
    MADE.with_borrow_mut(|made| match made.get(cluster) {
        Some(letter) => letter.clone(),
        None => made.entry(cluster.into()).or_insert_with(|| SharedString::from(cluster.to_owned())).clone(),
    })
}

/// A 38 px tall bordered button over art.
pub fn action(id: &'static str) -> Stateful<Div> {
    h_flex()
        .id(id)
        .occlude()
        .h(px(38.))
        .gap(px(10.))
        .border_1()
        .border_color(line(0.2))
        .bg(alpha(BG, 0.75))
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
        .hover(|style| style.bg(rgb(RAISED_HOVER)))
}

/// The hero's main button: 38 px tall, filled with the accent.
pub fn primary_action(id: &'static str, cx: &App) -> Stateful<Div> {
    h_flex().id(id).occlude().h(px(38.)).bg(rgb(accent(cx))).text_color(rgb(BG)).font_weight(FontWeight::BOLD).cursor_pointer()
}

pub fn square(side: f32, color: impl Into<Hsla>) -> Div {
    div().size(px(side)).flex_none().bg(color.into())
}

/// Over the corners of a `relative` box with a 1 px border.
pub fn brackets(cx: &App) -> [Div; 4] {
    let accent = accent(cx);
    let bracket = || div().absolute().size(px(10.)).border_color(rgb(accent));
    [
        bracket().left(px(-1.)).top(px(-1.)).border_l_2().border_t_2(),
        bracket().right(px(-1.)).top(px(-1.)).border_r_2().border_t_2(),
        bracket().left(px(-1.)).bottom(px(-1.)).border_l_2().border_b_2(),
        bracket().right(px(-1.)).bottom(px(-1.)).border_r_2().border_b_2(),
    ]
}

/// Shrinks with its row: the text is cut, and shown whole in a tooltip.
pub fn keycap(id: impl Into<ElementId>, text: SharedString) -> Stateful<Div> {
    div()
        .id(id)
        .min_w_0()
        .px(px(6.))
        .py(px(3.))
        .border_1()
        .border_color(line(0.16))
        .bg(rgb(RAISED))
        .text_color(rgb(MUTED))
        .font_family(MONO)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(TYPE_CAPTION))
        .truncate()
        .child(fitted("text", text))
}

/// Text for a parent that truncates: a tooltip holds the whole of it when it was cut.
pub fn fitted(id: impl Into<ElementId>, text: SharedString) -> InteractiveText {
    let styled = StyledText::new(text.clone());
    let layout = styled.layout().clone();
    InteractiveText::new(id, styled).tooltip(move |_, window, cx| (layout.text() != *text).then(|| tip(text.clone(), window, cx)))
}

/// A tooltip that wraps instead of running across the window.
pub fn tip(text: SharedString, window: &mut Window, cx: &mut App) -> AnyView {
    Tooltip::element(move |_, _| div().max_w(px(TIP_WIDTH)).child(text.clone())).build(window, cx)
}

pub fn segment(label: impl Into<SharedString>, fill: Option<u32>, color: u32) -> Div {
    mono(label, TYPE_CAPTION)
        .font_weight(FontWeight::BOLD)
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(color))
        .when_some(fill, |cell, fill| cell.bg(rgb(fill)))
}

pub fn segmented() -> Div {
    h_flex().flex_none().border_1().border_color(line(0.16))
}

/// One option of a `segmented` choice; the caller adds the click.
pub fn choice(id: impl Into<ElementId>, label: impl Into<SharedString>, current: bool, cx: &App) -> Stateful<Div> {
    let accent = accent(cx);
    div()
        .id(id)
        .h_full()
        .cursor_pointer()
        .child(segment(label, current.then_some(accent), if current { BG } else { SEGMENT_IDLE }).px(px(12.)))
}

pub fn toggle(on: bool, cx: &App) -> Div {
    let accent = accent(cx);
    let (off_color, on_color) = if on { (SEGMENT_IDLE, BG) } else { (TEXT, SEGMENT_IDLE) };
    segmented()
        .h(px(24.))
        .child(segment(t(cx, key::OFF), (!on).then_some(SEGMENT_OFF), off_color).w(px(34.)))
        .child(segment(t(cx, key::ON), on.then_some(accent), on_color).w(px(34.)))
}

/// A `toggle` that is its own click target; the caller adds the click.
pub fn switch(id: impl Into<ElementId>, on: bool, cx: &App) -> Stateful<Div> {
    div().id(id).cursor_pointer().child(toggle(on, cx))
}

/// A clickable list row: filled while `selected`, else lit on hover; the caller adds the click.
pub fn list_row(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    h_flex().id(id).cursor_pointer().map(|row| match selected {
        true => row.bg(rgb(ROW_SELECTED)),
        false => row.hover(|style| style.bg(rgb(ROW_HOVER))),
    })
}

pub fn icon_button(id: impl Into<ElementId>, glyph: Icon, side: f32, glyph_size: f32) -> Stateful<Div> {
    tinted_icon_button(id, glyph, side, glyph_size, DIM)
}

/// An `icon_button` whose glyph rests in `color` and lights on hover.
pub fn tinted_icon_button(id: impl Into<ElementId>, glyph: Icon, side: f32, glyph_size: f32, color: u32) -> Stateful<Div> {
    const GROUP: &str = "icon-button";
    div()
        .id(id)
        .group(GROUP)
        .size(px(side))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(line(0.06)))
        .child(icon(glyph, glyph_size, rgb(color)).group_hover(GROUP, |style| style.text_color(rgb(TEXT))))
}

/// A small outlined tag in `color`, for a state worth noticing.
pub fn badge(label: impl Into<SharedString>, color: u32) -> Div {
    mono(label, TYPE_TAG)
        .font_weight(FontWeight::BOLD)
        .flex_none()
        .px(px(6.))
        .py(px(2.))
        .border_1()
        .border_color(alpha(color, 0.55))
        .bg(alpha(color, 0.12))
        .text_color(rgb(color))
}

/// A dashed card that stands in for content that is not there.
pub fn notice(tag: impl Into<SharedString>, title: impl Into<SharedString>, text: impl Into<SharedString>) -> Div {
    v_flex()
        .items_center()
        .gap(px(14.))
        .px(px(24.))
        .py(px(56.))
        .border_1()
        .border_dashed()
        .border_color(line(0.2))
        .bg(alpha(BG, 0.7))
        .child(mono(tag, TYPE_LABEL).text_color(rgb(DIM)))
        .child(tracked(&title.into(), TYPE_BRAND, 0.1).justify_center().font_weight(FontWeight::BOLD))
        .child(div().max_w(px(420.)).text_center().text_color(rgb(MUTED)).child(text.into()))
}

pub fn button(label: &str) -> Div {
    button_frame().child(tracked(label, TYPE_LABEL, 0.12))
}

/// A press-only button's confirmation: `done` in the accent color while `flashed`.
pub fn flash_button(label: &str, done: &str, flashed: bool, cx: &App) -> Div {
    if !flashed {
        return button(label);
    }
    let overlay = h_flex().absolute().top_0().left_0().size_full();
    button_frame()
        .relative()
        .border_color(rgb(accent(cx)))
        .text_color(rgb(accent(cx)))
        // The hidden label keeps the button's width.
        .child(tracked(label, TYPE_LABEL, 0.12).invisible())
        .child(overlay.justify_center().child(tracked(done, TYPE_LABEL, 0.12)))
}

/// Runs `clear` once the confirmation has been shown; dropping the task cancels it.
pub fn flash<T: 'static>(cx: &mut Context<T>, clear: impl FnOnce(&mut T) + 'static) -> Task<()> {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(FLASH).await;
        this.update(cx, |this, cx| {
            clear(this);
            cx.notify();
        })
        .ok();
    })
}

fn button_frame() -> Div {
    h_flex()
        .h(px(26.))
        .px(px(12.))
        .gap(px(8.))
        .flex_none()
        .justify_center()
        .border_1()
        .border_color(line(0.2))
        .bg(rgb(RAISED))
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
        .hover(|style| style.bg(rgb(RAISED_HOVER)))
}

/// The text of a `filter`; `V` repaints as it changes (`set_value` sends no change).
pub fn filter_input<V: 'static>(placeholder: Key, window: &mut Window, cx: &mut Context<V>) -> (Entity<InputState>, Subscription) {
    let (state, relabel) = input(placeholder, window, cx);
    let typed = cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    });
    (state, Subscription::join(typed, relabel))
}

/// An input whose placeholder follows the language while the subscription lives.
pub fn input<V: 'static>(placeholder: Key, window: &mut Window, cx: &mut Context<V>) -> (Entity<InputState>, Subscription) {
    let state = cx.new(|cx| InputState::new(window, cx).placeholder(t(cx, placeholder)));
    let relabel = relabel(&state, placeholder, window, cx);
    (state, relabel)
}

/// Keeps an input's placeholder in the user's language while the subscription lives.
pub fn relabel<V: 'static, M: InputModeKind>(
    state: &Entity<InputBaseState<M>>,
    placeholder: Key,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Subscription {
    let state = state.downgrade();
    cx.observe_global_in::<Texts>(window, move |_, window, cx| {
        state.update(cx, |state, cx| state.set_placeholder(t(cx, placeholder), window, cx)).ok();
    })
}

/// A short typed number: a sunken box of `width` around `state`, centered in the mono face.
pub fn number_field(state: &Entity<InputState>, width: f32, disabled: bool) -> Div {
    mono("", TYPE_FIELD)
        .w(px(width))
        .h(px(26.))
        .flex()
        .items_center()
        .border_1()
        .border_color(line(0.14))
        .bg(rgb(SUNKEN))
        .child(Input::new(state).appearance(false).disabled(disabled).px(px(8.)).text_center().font_family(MONO).text_size(px(TYPE_FIELD)))
}

pub fn filter(state: &Entity<InputState>, cx: &App) -> Div {
    h_flex()
        .gap(px(8.))
        .bg(rgb(SUNKEN))
        .child(mono("/", TYPE_SMALL).font_weight(FontWeight::BOLD).text_color(rgb(accent(cx))))
        .child(div().flex_1().min_w_0().child(Input::new(state).appearance(false).px_0().font_family(MONO).text_size(px(TYPE_FIELD))))
}

/// 1 px background grid, painted because GPUI has no repeating backgrounds.
pub fn grid(step: f32, color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<gpui_kit::Pixels>, _, window, _| {
            let mut x = bounds.left();
            while x < bounds.right() {
                let column = Bounds::new(point(x, bounds.top()), size(px(1.), bounds.size.height));
                window.paint_quad(fill(column, color));
                x += px(step);
            }
            let mut y = bounds.top();
            while y < bounds.bottom() {
                let row = Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(1.)));
                window.paint_quad(fill(row, color));
                y += px(step);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::letter;

    #[test]
    fn a_letter_is_its_cluster() {
        for cluster in (' '..='~').map(String::from).chain(["É".into(), "e\u{301}".into()]) {
            assert_eq!(letter(&cluster).as_ref(), cluster);
        }
    }
}
