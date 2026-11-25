use mandrake_common::FourCC;
use mandrake_session::{Action, SaveGame, Screen, Text};

use super::saved_games::{SnapshotKey, snapshot_key};
use super::{OpenScreen, ScreenFrame};
use crate::controls::list::List;
use crate::core::{Dialog, DialogStack, dialog_flags, format_one};
use crate::resources::Resources;

pub const SAGD: FourCC = FourCC(*b"sagd");
const BACKGROUND: FourCC = FourCC(*b"insg");
pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const NAME: i16 = 3;
pub const LIST: i16 = 4;
pub const SNAPSHOT: i16 = 6;

pub const SUYW: FourCC = FourCC(*b"suyw");
const QUESTION_BACKGROUND: FourCC = FourCC(*b"innb");
pub const REPLACE: i16 = 1;
pub const KEEP: i16 = 2;
pub const QUESTION: i16 = 3;
const QUESTION_STRINGS: FourCC = FourCC(*b"irp!");
const QUESTION_STRING: usize = 0;

fn list_of(dialog: &mut Dialog) -> Option<&mut List> {
    dialog.list_mut(LIST)
}

fn format_name(format: &[u8], name: &[u8]) -> Vec<u8> {
    format_one(format, name)
}

pub fn open(
    data: &SaveGame,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let def = res.dialog(SAGD)?;
    let mut dialog = Dialog::new(&def, SAGD, dialog_flags::MODAL, res);
    dialog.set_default_item(OK);
    dialog.set_cancel_item(CANCEL);
    if let Some(list) = list_of(&mut dialog) {
        list.set_double_click_item(OK);
    }
    if let Some((item, edit)) = dialog.control_mut(NAME).and_then(|c| c.edit_mut()) {
        edit.set_text(item, &data.name.0);
    }
    dialog.set_focus(NAME);
    dialog.set_background(res.art(BACKGROUND), 0);
    let mut screen = Open {
        shown: None,
        question: false,
    };
    screen.update(&mut dialog, data);
    stack.push(dialog);
    Some(Box::new(screen))
}

struct Shown {
    names: Vec<Text>,
    snapshot: Option<SnapshotKey>,
}

impl Shown {
    fn matches(&self, data: &SaveGame) -> bool {
        self.snapshot == snapshot_key(data.snapshot.as_ref()) && self.names == data.names
    }
}

struct Open {
    shown: Option<Shown>,
    question: bool,
}

impl Open {
    fn update(&mut self, dialog: &mut Dialog, data: &SaveGame) {
        let names_changed = self
            .shown
            .as_ref()
            .is_none_or(|old| old.names != data.names);
        if names_changed && let Some(list) = list_of(dialog) {
            list.clear();
            for (row, name) in data.names.iter().enumerate() {
                list.add_row(&name.0, row as i32, true);
            }
            if data.names.is_empty() {
                list.clear_selection();
            } else {
                list.select(0);
            }
        }
        let snap_key = snapshot_key(data.snapshot.as_ref());
        let snap_changed = self
            .shown
            .as_ref()
            .is_none_or(|old| old.snapshot != snap_key);
        if snap_changed
            && let Some((item, snap)) = dialog.control_mut(SNAPSHOT).and_then(|c| c.snapshot_mut())
        {
            snap.set(item, data.snapshot.clone());
        }
        dialog.set_enabled(
            OK,
            mandrake_session::saves::save_name_ok(dialog.edit_text_slice(NAME)),
        );
        self.shown = Some(Shown {
            names: data.names.clone(),
            snapshot: snap_key,
        });
    }

    fn ask(name: &Text, frame: &mut ScreenFrame<'_>) -> bool {
        let Some(def) = frame.res.dialog(SUYW) else {
            tracing::warn!("dialogs: the replace question (suyw) isn't in the tags");
            return false;
        };
        let format = frame
            .res
            .string_line(QUESTION_STRINGS, QUESTION_STRING)
            .to_vec();
        let mut dialog = Dialog::new(&def, SUYW, dialog_flags::MODAL, frame.res);
        dialog.set_static_text(QUESTION, &format_name(&format, &name.0));
        dialog.set_background(frame.res.art(QUESTION_BACKGROUND), 0);
        frame.stack.push(dialog);
        true
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Playing {
            overlay: Some(mandrake_session::Overlay::SaveGame(data)),
        } = screen
        else {
            return;
        };
        match (&data.replace, self.question) {
            (Some(name), false) => {
                self.question = Self::ask(name, frame);
            }
            (None, true) => {
                if frame.stack.top().is_some_and(|d| d.id == SUYW) {
                    frame.stack.pop();
                }
                self.question = false;
            }
            _ => {}
        }
        if self.question {
            for hit in frame.hits.iter().filter(|h| h.dialog == SUYW) {
                match hit.item {
                    REPLACE => frame.out.actions.push(Action::ReplaceSave),
                    KEEP => frame.out.actions.push(Action::Cancel),
                    _ => {}
                }
            }
            return;
        }
        let Some(dialog) = frame.stack.find_mut(SAGD) else {
            return;
        };
        for hit in frame.hits.iter().filter(|h| h.dialog == SAGD) {
            match hit.item {
                OK if mandrake_session::saves::save_name_ok(dialog.edit_text_slice(NAME)) => {
                    frame.out.actions.push(Action::SaveGameAs(Text(
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
        if self.shown.as_ref().is_none_or(|s| !s.matches(data)) {
            self.update(dialog, data);
        }
        dialog.set_enabled(
            OK,
            mandrake_session::saves::save_name_ok(dialog.edit_text_slice(NAME)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_goes_in_for_its_s() {
        assert_eq!(
            format_name(b"Replace the saved game \xd2%s\xd3?", b"Quicksave 2"),
            b"Replace the saved game \xd2Quicksave 2\xd3?"
        );
    }

    #[test]
    fn missing_suyw_does_not_wedge_save_game() {
        use mandrake_session::{FrameInput, FrameOutput, Overlay};
        use mandrake_tag::{DialogDefinition, DialogItem, Rect};
        let def = DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 200, 200),
            items: vec![
                DialogItem {
                    kind: FourCC(*b"butt"),
                    field_04: 0,
                    id: OK as u32,
                    fields_08: [0; 4],
                    rect: Rect::new(0, 0, 20, 40),
                    name: Vec::new(),
                },
                DialogItem {
                    kind: FourCC(*b"butt"),
                    field_04: 0,
                    id: CANCEL as u32,
                    fields_08: [0; 4],
                    rect: Rect::new(30, 0, 50, 40),
                    name: Vec::new(),
                },
                DialogItem {
                    kind: FourCC(*b"edit"),
                    field_04: 0,
                    id: NAME as u32,
                    fields_08: [0; 4],
                    rect: Rect::new(60, 0, 80, 100),
                    name: Vec::new(),
                },
            ],
        };
        let mut res = Resources::from_source(|_, _| None);
        let mut stack = DialogStack::new();
        stack.push(Dialog::new(&def, SAGD, dialog_flags::MODAL, &mut res));
        let mut open = Open {
            shown: None,
            question: false,
        };
        let data = SaveGame {
            names: Vec::new(),
            name: Text(b"Test".to_vec()),
            snapshot: None,
            replace: Some(Text(b"Test".to_vec())),
        };
        let input = FrameInput {
            now_ms: 0,
            size: (800, 600),
            events: &[],
            sound_busy: false,
            shift_held: false,
        };
        let mut out = FrameOutput::default();
        let hits = [crate::core::Hit {
            dialog: SAGD,
            item: CANCEL,
        }];
        open.frame(
            &Screen::Playing {
                overlay: Some(Overlay::SaveGame(&data)),
            },
            &mut ScreenFrame {
                stack: &mut stack,
                res: &mut res,
                input: &input,
                now_tick: 0,
                ticks: 0,
                hits: &hits,
                out: &mut out,
            },
        );
        assert!(!open.question);
        assert_eq!(out.actions, [Action::Cancel]);
    }
}
