use mandrake_common::FourCC;
use mandrake_session::{Action, DeleteFilm, RenameFilm, Text};

use super::ScreenFrame;
use crate::controls::edit::edit_flags;
use crate::controls::list::List;
use crate::core::{Dialog, DialogStack, dialog_flags, format_one};
use crate::resources::Resources;

pub const SU01: FourCC = FourCC(*b"su01");
const INNB: FourCC = FourCC(*b"innb");
pub const DELETE: i16 = 1;
pub const KEEP: i16 = 2;
pub const QUESTION: i16 = 3;
const QUESTION_STRINGS: FourCC = FourCC(*b"irp!");
pub const FILM_QUESTION: usize = 3;
pub const SAVE_QUESTION: usize = 1;

pub const STPT: FourCC = FourCC(*b"stpt");
const INSH: FourCC = FourCC(*b"insh");
pub const RENAME: i16 = 1;
const RENAME_NAME: &[u8] = b"Rename";
pub const CANCEL: i16 = 2;
pub const NAME: i16 = 3;
pub const LIST: i16 = 4;
pub const PROMPT: i16 = 5;

pub const PRDN: FourCC = FourCC(*b"prdn");
pub const NOTICE: i16 = 1;
pub const NOTICE_OK: i16 = 2;
const EXISTS: &[u8] = b"A file with that name already exists.";
const OK_NAME: &[u8] = b"OK";

fn format_name(format: &[u8], name: &[u8]) -> Vec<u8> {
    format_one(format, name)
}

pub(crate) fn pop_if_top(stack: &mut DialogStack, id: FourCC) {
    if stack.top().is_some_and(|d| d.id == id) {
        stack.pop();
    }
}

fn open_question(data: &DeleteFilm, res: &mut Resources, stack: &mut DialogStack) -> bool {
    open_delete_question(&data.name.0, FILM_QUESTION, res, stack)
}

pub(crate) fn open_delete_question(
    name: &[u8],
    string: usize,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> bool {
    let Some(def) = res.dialog(SU01) else {
        tracing::warn!("dialogs: the delete question (su01) isn't in the tags");
        return false;
    };
    let format = res.string_line(QUESTION_STRINGS, string).to_vec();
    let mut dialog = Dialog::new(&def, SU01, dialog_flags::MODAL, res);
    let mut line = format_name(&format, name);
    line.truncate(99);
    dialog.set_static_text(QUESTION, &line);
    dialog.set_background(res.art(INNB), 0);
    stack.push(dialog);
    true
}

fn open_prompt(data: &RenameFilm, res: &mut Resources, stack: &mut DialogStack) -> bool {
    let Some(def) = res.dialog(STPT) else {
        tracing::warn!("dialogs: Rename Film's prompt (stpt) isn't in the tags");
        return false;
    };
    let mut dialog = Dialog::new(&def, STPT, dialog_flags::MODAL, res);
    dialog.set_static_text(PROMPT, b"");
    if let Some(list) = dialog.list_mut(LIST) {
        list.set_double_click_item(RENAME);
    }
    if let Some((item, edit)) = dialog.control_mut(NAME).and_then(|c| c.edit_mut()) {
        edit.set_flag(item, edit_flags::NO_WIDTH_LIMIT, true);
        edit.set_text(item, &data.name.0);
    }
    let ok = mandrake_session::films::rename_name_ok(dialog.edit_text_slice(NAME));
    dialog.set_enabled(RENAME, ok);
    dialog.set_background(res.art(INSH), 0);
    dialog.set_button_caption(RENAME, RENAME_NAME);
    dialog.set_default_item(RENAME);
    dialog.set_cancel_item(CANCEL);
    if let Some(list) = list_of(&mut dialog) {
        list.clear();
        for (row, name) in data.names.iter().enumerate() {
            list.add_row(&name.0, row as i32, false);
        }
        list.select(0);
    }
    dialog.set_focus(NAME);
    stack.push(dialog);
    true
}

fn list_of(dialog: &mut Dialog) -> Option<&mut List> {
    dialog.control_mut(LIST)?.list_mut()
}

fn open_notice(res: &mut Resources, stack: &mut DialogStack) -> bool {
    let Some(def) = res.dialog(PRDN) else {
        tracing::warn!("dialogs: the notice (prdn) isn't in the tags");
        return false;
    };
    let mut dialog = Dialog::new(&def, PRDN, dialog_flags::MODAL, res);
    dialog.set_background(res.art(INNB), 0);
    if let Some((item, text)) = dialog.control_mut(NOTICE).and_then(|c| c.static_text_mut()) {
        text.set_text(item, EXISTS);
    }
    if let Some((item, button)) = dialog.control_mut(NOTICE_OK).and_then(|c| c.button_mut()) {
        button.set_caption(item, OK_NAME);
    }
    dialog.set_default_item(NOTICE_OK);
    stack.push(dialog);
    true
}

#[derive(Default)]
pub struct Open {
    question: bool,
    prompt: bool,
    notice: bool,
}

impl Open {
    pub fn follow(
        &mut self,
        delete: Option<&DeleteFilm>,
        rename: Option<&RenameFilm>,
        frame: &mut ScreenFrame<'_>,
    ) -> bool {
        let exists = rename.is_some_and(|r| r.exists);
        if self.notice && !exists {
            pop_if_top(frame.stack, PRDN);
            self.notice = false;
        }
        if self.prompt && rename.is_none() {
            pop_if_top(frame.stack, STPT);
            self.prompt = false;
        }
        if self.question && delete.is_none() {
            pop_if_top(frame.stack, SU01);
            self.question = false;
        }

        if let Some(data) = delete {
            if !self.question {
                self.question = open_question(data, frame.res, frame.stack);
            } else {
                for hit in frame.hits.iter().filter(|h| h.dialog == SU01) {
                    match hit.item {
                        DELETE => frame.out.actions.push(Action::ConfirmDeleteFilm),
                        KEEP => frame.out.actions.push(Action::Cancel),
                        _ => {}
                    }
                }
            }
        }
        if let Some(data) = rename {
            if !self.prompt {
                self.prompt = open_prompt(data, frame.res, frame.stack);
            } else if self.notice {
                for hit in frame.hits.iter().filter(|h| h.dialog == PRDN) {
                    if hit.item == NOTICE_OK {
                        frame.out.actions.push(Action::CloseNotice);
                    }
                }
            } else {
                self.prompt_frame(frame);
            }
            if data.exists && !self.notice && self.prompt {
                self.notice = open_notice(frame.res, frame.stack);
            }
        }
        self.question || self.prompt || self.notice
    }

    fn prompt_frame(&mut self, frame: &mut ScreenFrame<'_>) {
        let Some(dialog) = frame.stack.find_mut(STPT) else {
            return;
        };
        for hit in frame.hits.iter().filter(|h| h.dialog == STPT) {
            match hit.item {
                RENAME => {
                    let name = dialog.edit_text_slice(NAME);
                    if mandrake_session::films::rename_name_ok(name) {
                        frame
                            .out
                            .actions
                            .push(Action::RenameFilmAs(Text(name.to_vec())));
                    }
                }
                CANCEL => frame.out.actions.push(Action::Cancel),
                _ => {}
            }
        }
        let ok = mandrake_session::films::rename_name_ok(dialog.edit_text_slice(NAME));
        dialog.set_enabled(RENAME, ok);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_goes_in_for_its_s() {
        assert_eq!(
            format_name(
                b"Are you sure you want to delete the film \xd2%s\xd3?",
                b"MDL.m2rec"
            ),
            b"Are you sure you want to delete the film \xd2MDL.m2rec\xd3?"
        );
    }
}
