use mandrake_common::FourCC;
use mandrake_session::{Action, Films, Screen};

use super::save_film::{self, Prompt};
use super::{OpenScreen, ScreenFrame, film_manage, main_menu};
use crate::controls::list::{List, list_flags};
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const SERD: FourCC = FourCC(*b"serd");
const BACKGROUND: FourCC = FourCC(*b"invs");
pub const VIEW: i16 = 1;
pub const CANCEL: i16 = 2;
pub const LIST: i16 = 3;
pub const GAME: i16 = 5;
pub const PLAYERS: i16 = 6;
pub const SAVE_LAST: i16 = 7;
pub const DELETE: i16 = 8;
pub const REPLAY_LAST: i16 = 9;
pub const RENAME: i16 = 13;
const FIND_PLUGINS: &[u8] = b"Find Plugins";

fn list_of(dialog: &mut Dialog, id: i16) -> Option<&mut List> {
    dialog.control_mut(id)?.list_mut()
}

pub fn open(
    data: &Films,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let def = res.dialog(SERD)?;
    main_menu::push_behind(res, stack);
    let mut dialog = Dialog::new(&def, SERD, dialog_flags::MODAL, res);
    let view_name = def
        .items
        .iter()
        .find(|i| i.id == VIEW as u32)
        .map(|i| i.name.clone())
        .unwrap_or_default();
    dialog.set_default_item(VIEW);
    dialog.set_cancel_item(CANCEL);
    if let Some(list) = list_of(&mut dialog, LIST) {
        list.set_double_click_item(VIEW);
        list.set_flag(list_flags::FOCUS_RING, false);
        list.set_flag(list_flags::FRAME, false);
    }
    if let Some(list) = list_of(&mut dialog, PLAYERS) {
        list.set_flag(list_flags::FRAME, false);
    }
    dialog.set_background(res.art(BACKGROUND), 0);
    if let Some((item, game)) = dialog.control_mut(GAME).and_then(|c| c.game_mut()) {
        game.set_is_replay(item, true);
    }
    let mut screen = Open {
        shown: None,
        view_name,
        prompt: None,
        manage: film_manage::Open::default(),
    };
    screen.update(&mut dialog, data, res);
    stack.push(dialog);
    Some(Box::new(screen))
}

struct Shown {
    names: Vec<mandrake_session::Text>,
    selected: Option<usize>,
    details: Option<mandrake_session::FilmDetails>,
    can_view: bool,
    find_plugins: bool,
    last_film: bool,
    save_last: bool,
    delete: bool,
    rename: bool,
}

impl Shown {
    fn matches(&self, data: &Films) -> bool {
        self.selected == data.selected
            && self.can_view == data.can_view
            && self.find_plugins == data.find_plugins
            && self.last_film == data.last_film
            && self.save_last == data.save_last
            && self.delete == data.delete
            && self.rename == data.rename
            && self.details == data.details
            && self.names == data.names
    }
}

struct Open {
    shown: Option<Shown>,
    view_name: Vec<u8>,
    prompt: Option<Prompt>,
    manage: film_manage::Open,
}

impl Open {
    fn update(&mut self, dialog: &mut Dialog, data: &Films, res: &mut Resources) {
        let names_changed = self
            .shown
            .as_ref()
            .is_none_or(|old| old.names != data.names);
        if names_changed && let Some(list) = list_of(dialog, LIST) {
            list.clear();
            for (row, name) in data.names.iter().enumerate() {
                list.add_row(&name.0, row as i32, true);
            }
        }
        if let Some(control) = dialog.control_mut(LIST) {
            control.item.dirty = true;
            if let Some(list) = control.list_mut()
                && list.selected() != data.selected
            {
                match data.selected {
                    Some(row) => {
                        list.select(row);
                        list.show_selected();
                    }
                    None => list.clear_selection(),
                }
            }
        }
        let chosen = data.selected.is_some();
        dialog.set_enabled(REPLAY_LAST, data.last_film);
        dialog.set_enabled(SAVE_LAST, data.last_film && data.save_last);
        dialog.set_visible(GAME, chosen);
        dialog.set_visible(PLAYERS, chosen);
        dialog.set_enabled(DELETE, chosen && data.delete);
        dialog.set_enabled(RENAME, chosen && data.rename);
        dialog.set_enabled(VIEW, data.can_view);
        let name = if data.find_plugins {
            FIND_PLUGINS
        } else {
            self.view_name.as_slice()
        };
        dialog.set_button_caption(VIEW, name);
        let details_changed = self
            .shown
            .as_ref()
            .is_none_or(|old| old.details != data.details);
        if details_changed {
            super::lobby::show_game_details(dialog, GAME, data.details.clone(), res);
            if let Some(control) = dialog.control_mut(PLAYERS) {
                control.item.dirty = true;
                if let Some(list) = control.list_mut() {
                    list.clear();
                }
            }
        }
        self.shown = Some(Shown {
            names: data.names.clone(),
            selected: data.selected,
            details: data.details.clone(),
            can_view: data.can_view,
            find_plugins: data.find_plugins,
            last_film: data.last_film,
            save_last: data.save_last,
            delete: data.delete,
            rename: data.rename,
        });
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Films(data) = screen else {
            return;
        };
        let prompt_open = save_film::follow(&mut self.prompt, data.save_film.as_ref(), frame);
        let managing =
            self.manage
                .follow(data.delete_film.as_ref(), data.rename_film.as_ref(), frame);
        let prompt_open = prompt_open || managing;
        let Some(dialog) = frame.stack.find_mut(SERD) else {
            return;
        };
        let hits = frame
            .hits
            .iter()
            .filter(|h| h.dialog == SERD && !prompt_open);
        for hit in hits {
            let action = match hit.item {
                VIEW => Some(Action::ViewFilm),
                CANCEL => Some(Action::Cancel),
                REPLAY_LAST => Some(Action::ReplayLastFilm),
                SAVE_LAST => Some(Action::SaveFilm),
                DELETE => Some(Action::DeleteFilm),
                RENAME => Some(Action::RenameFilm),
                LIST => list_of(dialog, LIST)
                    .and_then(|list| list.selected())
                    .filter(|&row| Some(row) != data.selected)
                    .map(Action::ChooseFilm),
                item => {
                    tracing::info!(item, "saved films: not built yet");
                    None
                }
            };
            frame.out.actions.extend(action);
        }
        if self.shown.as_ref().is_none_or(|s| !s.matches(data)) {
            self.update(dialog, data, frame.res);
        }
    }
}
