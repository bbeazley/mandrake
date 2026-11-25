use mandrake_common::FourCC;
use mandrake_session::{Action, SaveFilm, Text};

use super::ScreenFrame;
use crate::controls::list::List;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const STPT: FourCC = FourCC(*b"stpt");
const BACKGROUND: FourCC = FourCC(*b"insh");
pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const NAME: i16 = 3;
pub const LIST: i16 = 4;
pub const PROMPT: i16 = 5;

fn list_of(dialog: &mut Dialog) -> Option<&mut List> {
    dialog.list_mut(LIST)
}

pub struct Prompt;

impl Prompt {
    pub fn open(data: &SaveFilm, res: &mut Resources, stack: &mut DialogStack) -> Option<Self> {
        let def = res.dialog(STPT)?;
        let mut dialog = Dialog::new(&def, STPT, dialog_flags::MODAL, res);
        dialog.set_static_text(PROMPT, b"");
        dialog.set_edit_text(NAME, &data.name.0);
        dialog.set_background(res.art(BACKGROUND), 0);
        dialog.set_default_item(OK);
        dialog.set_cancel_item(CANCEL);
        if let Some(list) = list_of(&mut dialog) {
            list.clear();
            for (row, name) in data.names.iter().enumerate() {
                list.add_row(&name.0, row as i32, true);
            }
            if data.names.is_empty() {
                list.clear_selection();
            } else {
                list.select(0);
            }
            list.set_double_click_item(OK);
        }
        dialog.set_focus(NAME);
        dialog.set_enabled(OK, ok_enabled(&data.name.0, data));
        stack.push(dialog);
        Some(Self)
    }

    pub fn frame(&mut self, data: &SaveFilm, frame: &mut ScreenFrame<'_>) {
        let Some(dialog) = frame.stack.find_mut(STPT) else {
            return;
        };
        for hit in frame.hits.iter().filter(|h| h.dialog == STPT) {
            match hit.item {
                OK if ok_enabled(dialog.edit_text_slice(NAME), data) => {
                    frame.out.actions.push(Action::SaveFilmAs(Text(
                        dialog.edit_text_slice(NAME).to_vec(),
                    )));
                }
                CANCEL => frame.out.actions.push(Action::Cancel),
                LIST => {
                    let row = list_of(dialog).and_then(|list| list.selected());
                    if let Some(name) = row.and_then(|row| data.names.get(row)) {
                        dialog.set_edit_text(NAME, &name.0);
                    }
                }
                _ => {}
            }
        }
        let enabled = ok_enabled(dialog.edit_text_slice(NAME), data);
        dialog.set_enabled(OK, enabled);
    }

    pub fn close(self, stack: &mut DialogStack) {
        if stack.top().is_some_and(|d| d.id == STPT) {
            stack.pop();
        }
    }
}

fn ok_enabled(name: &[u8], data: &SaveFilm) -> bool {
    mandrake_session::films::film_name_ok(name, &data.last_name.0)
}

pub fn follow(
    prompt: &mut Option<Prompt>,
    data: Option<&SaveFilm>,
    frame: &mut ScreenFrame<'_>,
) -> bool {
    match (prompt.as_mut(), data) {
        (None, Some(data)) => {
            *prompt = Prompt::open(data, frame.res, frame.stack);
            if prompt.is_none() {
                tracing::warn!("dialogs: Save Film's prompt (stpt) isn't in the tags");
            }
        }
        (Some(open), Some(data)) => open.frame(data, frame),
        (Some(_), None) => {
            if let Some(open) = prompt.take() {
                open.close(frame.stack);
            }
        }
        (None, None) => {}
    }
    prompt.is_some()
}
