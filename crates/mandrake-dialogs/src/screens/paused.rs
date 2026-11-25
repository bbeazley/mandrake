use mandrake_common::FourCC;
use mandrake_session::{Action, Paused, Screen, UiEvent, UiKey};

use super::{OpenScreen, ScreenFrame};
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const QUDI: FourCC = FourCC(*b"qudi");
const INGP: FourCC = FourCC(*b"ingp");

pub const QUIT: i16 = 1;
pub const RESUME: i16 = 2;
pub const REVERT: i16 = 3;
pub const RESTART: i16 = 4;
pub const SAVE: i16 = 5;
pub const OPEN: i16 = 7;
pub const HOST_TEXT: i16 = 6;
pub const PEER_TEXT: i16 = 8;
const TITLE: i16 = 9;

const SOLO_ITEMS: u32 = 0x2be;
const NETWORK_ITEMS: u32 = 0x206;
const REHOST_NAME: &[u8] = b"Rehost\r\nGame";
const RESTART_NAME: &[u8] = b"Restart\r\nGame";
const BIG_BUTTON_FLAGS: u32 =
    crate::controls::button::button_flags::SHAPE_LEFT | crate::controls::button::button_flags::ICON;
const BIG_BUTTONS: [(i16, i32); 4] = [(REVERT, 6), (RESTART, 4), (SAVE, 3), (OPEN, 5)];
fn title_sequence(data: &Paused) -> i32 {
    if data.network.is_some() || data.replay {
        2
    } else {
        1
    }
}
const TITLE_DRAWING_FLAGS: u32 = mandrake_view::interface_bitmap::placement::LEFT;

fn visible_items(data: &Paused) -> u32 {
    let Some(net) = data.network.as_ref() else {
        return if data.replay {
            NETWORK_ITEMS
        } else {
            SOLO_ITEMS
        };
    };
    let extra = if net.rehost_items {
        (1 << REVERT) | (1 << RESTART) | (1 << SAVE) | (1 << OPEN)
    } else if net.originator {
        1 << HOST_TEXT
    } else {
        1 << PEER_TEXT
    };
    NETWORK_ITEMS | extra
}

fn set_enabled(dialog: &mut Dialog, data: &Paused) {
    if let Some(net) = &data.network {
        dialog.set_enabled(REVERT, net.can_rehost);
        dialog.set_enabled(RESTART, net.can_rehost);
        dialog.set_enabled(SAVE, data.can_save);
        dialog.set_enabled(OPEN, data.can_save);
        return;
    }
    dialog.set_enabled(OPEN, data.can_open);
    dialog.set_enabled(SAVE, data.can_save);
    dialog.set_enabled(REVERT, data.can_revert);
    dialog.set_enabled(RESTART, data.can_restart);
}

pub fn open(
    data: &Paused,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let def = res.dialog(QUDI)?;
    let art = res.art(INGP);
    let mut dialog = Dialog::new(&def, QUDI, dialog_flags::MODAL, res);
    dialog.set_background(art.clone(), 0);
    for (id, sequence) in BIG_BUTTONS {
        if let Some((item, button)) = dialog.control_mut(id).and_then(|c| c.button_mut()) {
            button.set_flags(BIG_BUTTON_FLAGS);
            button.set_shape(item, art.clone(), sequence);
        }
    }
    if let Some((item, title)) = dialog.control_mut(TITLE).and_then(|c| c.bitmap_mut()) {
        title.set_art(item, art, title_sequence(data), 0);
        title.set_drawing_flags(item, TITLE_DRAWING_FLAGS, 0);
    }
    let visible = visible_items(data);
    for id in 1..10 {
        dialog.set_visible(id, visible & (1 << id) != 0);
    }
    if data.network.is_some_and(|net| net.rehost_items) {
        for (id, name) in [(REVERT, REHOST_NAME), (RESTART, RESTART_NAME)] {
            if let Some((item, button)) = dialog.control_mut(id).and_then(|c| c.button_mut()) {
                button.set_caption(item, name);
            }
        }
    }
    set_enabled(&mut dialog, data);
    stack.push(dialog);
    Some(Box::new(PausedScreen))
}

struct PausedScreen;

fn escape_pressed(events: &[UiEvent]) -> bool {
    events.iter().any(|e| {
        matches!(
            e,
            UiEvent::Key {
                key: UiKey::Escape,
                pressed: true,
                repeat: false,
                ..
            }
        )
    })
}

impl OpenScreen for PausedScreen {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Playing {
            overlay: Some(mandrake_session::Overlay::Paused(data)),
        } = screen
        else {
            return;
        };
        if let Some(dialog) = frame.stack.find_mut(QUDI) {
            set_enabled(dialog, data);
        }
        if escape_pressed(frame.input.events) {
            frame.out.actions.push(Action::Resume);
            return;
        }
        let network = data.network.is_some();
        for hit in frame.hits {
            match hit.item {
                REVERT | RESTART if network && !data.network.is_some_and(|n| n.rehost_items) => {}
                QUIT => frame.out.actions.push(Action::QuitToMenu),
                RESUME => frame.out.actions.push(Action::Resume),
                RESTART => frame.out.actions.push(Action::RestartLevel),
                SAVE => frame.out.actions.push(Action::SaveGame),
                REVERT => frame.out.actions.push(Action::LastSavedGame),
                OPEN => frame.out.actions.push(Action::OpenSavedGames),
                item => tracing::info!(item, "paused: not built yet"),
            }
        }
    }
}
