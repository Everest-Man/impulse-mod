//! The HUD inside the game's frame (the IN-GAME mode): Impulse's window, cloaked and never in front, takes the size the
//! overlay draws in and is mirrored into the canvas as its input layer. The game keeps the focus: the mouse the overlay
//! takes over it is posted to the window, the keys come from our hook (`Input`) as keystrokes.

use anyhow::Result;
use futures::{StreamExt, channel::mpsc};
use gpui_kit::{AsyncWindowContext, Modifiers, WeakEntity};
use model::CanvasEvent;
use topmost::{Area, Away};

use super::{
    Ask, Layer, Mode, fullscreen,
    in_game::{HUD_LAYER, InGameEvent},
    reveal,
};
use crate::input::HudKey;

impl Layer {
    pub(super) fn is_inside(&self) -> bool {
        matches!(self.mode, Mode::Over { inside: Some(_), .. })
    }

    /// The game the HUD takes keys in, while it is inside and ready.
    fn typing_into(&self) -> Option<isize> {
        match self.mode {
            Mode::Over { game, inside: Some(_) } if self.typing => Some(game),
            _ => None,
        }
    }

    /// While inside: the HUD layer's mouse goes to the window; a new size or the canvas's end is an ask.
    pub(super) fn in_game_event(&self, event: InGameEvent) {
        if !self.is_inside() {
            return;
        }
        let ask = match event {
            InGameEvent::Canvas(CanvasEvent::Input { layer: HUD_LAYER, message, wparam, lparam }) => {
                if let Some(hwnd) = self.hwnd {
                    topmost::post_input(hwnd, message, wparam, lparam);
                }
                return;
            }
            InGameEvent::Canvas(CanvasEvent::Resized { .. }) => Ask::Refit,
            InGameEvent::Gone => Ask::Lost,
            InGameEvent::Canvas(_) => return,
        };
        let _ = self.asks.unbounded_send(ask);
    }
}

pub(super) async fn enter(
    layer: &WeakEntity<Layer>,
    hwnd: isize,
    game: isize,
    size: (u32, u32),
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    topmost::cloak(hwnd, true);
    // A hidden or minimized window neither draws nor turns fullscreen; it is put back on the way out.
    let away = topmost::restore(hwnd);
    layer.update_in(cx, |layer, window, cx| {
        layer.exclusive = false;
        layer.turn(Mode::Over { game, inside: Some(away) }, window, cx);
    })?;
    // Frameless first: GPUI's switch saves the window's rect for the way back.
    fullscreen(true, cx).await?;
    fit(hwnd, game, Some(size));
    let drawn = layer.update_in(cx, |layer, window, cx| {
        let in_game = layer.in_game.read(cx);
        let Some(layers) = in_game.layers() else { return false };
        // Before the first frame, so the layer is described with the input; the block waits for the layer.
        layers.interact(HUD_LAYER, true);
        layers.block(true);
        window.mirror_frames(in_game.frames(HUD_LAYER, (0, 0), || {}));
        window.mirror_cursor(in_game.cursors(HUD_LAYER));
        window.refresh();
        layer.typing = true;
        layer.publish(cx);
        true
    })?;
    // The canvas went before the mode was set: nothing asks to leave.
    if !drawn {
        return leave(layer, hwnd, game, Some(away), cx).await;
    }
    // Outside the layer's update: its own observer hears this. GPUI's inputs draw the caret in an active window only.
    cx.update(|window, cx| window.hold_active(true, cx))
}

/// Out of the game's frame: put back as it was, behind the game, or in front where the user asked for it (`away`: `None`).
pub(super) async fn leave(
    layer: &WeakEntity<Layer>,
    hwnd: isize,
    game: isize,
    away: Option<Away>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    layer.update_in(cx, |layer, window, cx| {
        // First, so no key typed from now on lands in the window on its way out.
        layer.typing = false;
        window.mirror_frames(None);
        window.mirror_cursor(None);
        if let Some(layers) = layer.in_game.read(cx).layers() {
            layers.hide(HUD_LAYER);
            layers.block(false);
        }
        layer.turn(Mode::Window { armed: None }, window, cx);
        // The game keeps the focus unless the window was asked for: the HUD's minimap is due as the layer goes.
        if away.is_some() {
            layer.seen(Some(game), cx);
        }
    })?;
    // A capture or a question made inside ends here, as with any window going inactive.
    cx.update(|window, cx| window.hold_active(false, cx))?;
    fullscreen(false, cx).await?;
    match away {
        Some(Away::Shown) => topmost::lower(hwnd, Some(game)),
        Some(away) => {
            topmost::put_away(hwnd, away);
            topmost::cloak(hwnd, false);
            return Ok(());
        }
        None => {}
    }
    reveal(hwnd, cx).await
}

/// The keys our hook takes for the HUD, as keystrokes in the game's layout. Dispatched outside any entity's update:
/// the HUD's handlers read the layer.
pub(super) async fn type_keys(
    layer: WeakEntity<Layer>,
    mut keys: mpsc::UnboundedReceiver<HudKey>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    while let Some(HudKey { vk, scan, held }) = keys.next().await {
        // A key queued before the HUD was ready or after it began to leave goes nowhere.
        let Some(game) = layer.read_with(cx, |layer, _| layer.typing_into())? else { continue };
        let modifiers = Modifiers { control: held.control, alt: held.alt, shift: held.shift, ..Modifiers::default() };
        let keystroke = gpui_windows::keystroke_in(vk.into(), scan, modifiers, topmost::key_state(held), topmost::layout(game));
        let Some(keystroke) = keystroke else { continue };
        cx.update(|window, cx| window.dispatch_keystroke(keystroke, cx))?;
    }
    Ok(())
}

/// The window at the game's client area with the size the overlay draws in, right behind the game.
pub(super) fn fit(hwnd: isize, game: isize, size: Option<(u32, u32)>) {
    let (Some(Area { left, top, .. }), Some((width, height))) = (topmost::client_area(game), size) else { return };
    let area = Area { left, top, width: width as i32, height: height as i32 };
    topmost::lower(hwnd, Some(game));
    topmost::resize(hwnd, area);
    // Moved onto a monitor of another scale, GPUI's fullscreen fills that monitor: once more there.
    if topmost::client_area(hwnd).is_some_and(|now| (now.width, now.height) != (area.width, area.height)) {
        topmost::resize(hwnd, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_typed_in_the_game_is_named_as_gpui_names_it_and_a_modifier_is_no_key() {
        let key = |vk: u16| gpui_windows::keystroke_in(vk, 0, Modifiers::default(), [0; 256], topmost::layout(0)).map(|stroke| stroke.key);
        assert_eq!((key(0x0D).as_deref(), key(0x7B).as_deref()), (Some("enter"), Some("f12")));
        // Sided too: the hook reports them so.
        for vk in [0x10, 0xA0, 0xA1, 0x11, 0xA2, 0xA3, 0x12, 0xA4, 0xA5, 0x5B, 0x5C, 0x14, 0xE7] {
            assert_eq!(key(vk), None, "{vk:#x}");
        }
    }
}
