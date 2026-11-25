use mandrake_common::FourCC;
use mandrake_session::{Action, Overlay, SavedGames, Screen};
use mandrake_tag::Rect;
use mandrake_tag::font::{STYLE_PLAIN, text_flags};

use super::film_manage::{self, SAVE_QUESTION, SU01};
use super::{OpenScreen, ScreenFrame, main_menu};
use crate::controls::list::{List, list_flags};
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const OPSG: FourCC = FourCC(*b"opsg");
const BACKGROUND: FourCC = FourCC(*b"in#1");
pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const LIST: i16 = 4;
pub const SNAPSHOT: i16 = 5;
pub const DELETE: i16 = 6;
pub const DATE: i16 = 7;
const STAT: FourCC = FourCC(*b"stat");

fn list_of(dialog: &mut Dialog, id: i16) -> Option<&mut List> {
    dialog.control_mut(id)?.list_mut()
}

const fn date_rect(snapshot: Rect) -> Rect {
    Rect::new(
        snapshot.bottom + 8,
        snapshot.left,
        snapshot.bottom + 0x18,
        snapshot.left + 0xa0,
    )
}

pub fn open(
    data: &SavedGames,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    res.dialog(OPSG)?;
    main_menu::push_behind(res, stack);
    open_over_game(data, res, stack)
}

pub fn open_over_game(
    data: &SavedGames,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    Some(Box::new(push(data, res, stack)?))
}

pub(crate) struct Picker(Open);

impl Picker {
    pub(crate) fn open(
        data: &SavedGames,
        res: &mut Resources,
        stack: &mut DialogStack,
    ) -> Option<Self> {
        push(data, res, stack).map(Picker)
    }

    pub(crate) fn frame(&mut self, data: &SavedGames, frame: &mut ScreenFrame<'_>) {
        self.0.frame_with(data, frame);
    }

    pub(crate) fn close(self, stack: &mut DialogStack) {
        if self.0.question {
            film_manage::pop_if_top(stack, SU01);
        }
        film_manage::pop_if_top(stack, OPSG);
    }
}

fn push(data: &SavedGames, res: &mut Resources, stack: &mut DialogStack) -> Option<Open> {
    let def = res.dialog(OPSG)?;
    let mut dialog = Dialog::new(&def, OPSG, dialog_flags::MODAL, res);
    dialog.set_default_item(OK);
    dialog.set_cancel_item(CANCEL);
    if let Some(list) = list_of(&mut dialog, LIST) {
        list.set_double_click_item(OK);
        list.set_flag(list_flags::FOCUS_RING, false);
        list.set_flag(list_flags::FRAME, false);
    }
    dialog.set_background(res.art(BACKGROUND), 0);
    if let Some(snapshot) = dialog.control(SNAPSHOT).map(|c| c.item.rect) {
        dialog.add_control(STAT, DATE, date_rect(snapshot), &data.date.0, res);
        if let Some((item, text)) = dialog.control_mut(DATE).and_then(|c| c.static_text_mut()) {
            text.set_appearance(item, None, STYLE_PLAIN, None, Some(text_flags::CENTER));
        }
    }
    let mut screen = Open {
        shown: None,
        question: false,
    };
    screen.update(&mut dialog, data);
    stack.push(dialog);
    Some(screen)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SnapshotKey {
    ptr: usize,
    len: usize,
    width: u32,
    height: u32,
    samples: [u8; 4],
}

pub(super) fn snapshot_key(snapshot: Option<&mandrake_session::Snapshot>) -> Option<SnapshotKey> {
    snapshot.map(|s| {
        let len = s.rgba.len();
        let sample = |i: usize| s.rgba.get(i).copied().unwrap_or(0);
        SnapshotKey {
            ptr: s.rgba.as_ptr() as usize,
            len,
            width: s.width,
            height: s.height,
            samples: [
                sample(0),
                sample(len / 3),
                sample(len * 2 / 3),
                sample(len.saturating_sub(1)),
            ],
        }
    })
}

struct Shown {
    names: Vec<mandrake_session::Text>,
    selected: Option<usize>,
    snapshot: Option<SnapshotKey>,
    date: mandrake_session::Text,
    delete: bool,
}

impl Shown {
    fn matches(&self, data: &SavedGames) -> bool {
        self.selected == data.selected
            && self.delete == data.delete
            && self.date == data.date
            && self.snapshot == snapshot_key(data.snapshot.as_ref())
            && self.names == data.names
    }
}

struct Open {
    shown: Option<Shown>,
    question: bool,
}

impl Open {
    fn update(&mut self, dialog: &mut Dialog, data: &SavedGames) {
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
        dialog.set_enabled(DELETE, data.selected.is_some() && data.delete);
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
        dialog.set_static_text(DATE, &data.date.0);
        self.shown = Some(Shown {
            names: data.names.clone(),
            selected: data.selected,
            snapshot: snap_key,
            date: data.date.clone(),
            delete: data.delete,
        });
    }
}

impl Open {
    fn follow_question(&mut self, data: &SavedGames, frame: &mut ScreenFrame<'_>) -> bool {
        match (&data.delete_save, self.question) {
            (None, true) => {
                film_manage::pop_if_top(frame.stack, SU01);
                self.question = false;
            }
            (Some(question), false) => {
                self.question = film_manage::open_delete_question(
                    &question.name.0,
                    SAVE_QUESTION,
                    frame.res,
                    frame.stack,
                );
            }
            (Some(_), true) => {
                for hit in frame.hits.iter().filter(|h| h.dialog == SU01) {
                    match hit.item {
                        film_manage::DELETE => frame.out.actions.push(Action::ConfirmDeleteSave),
                        film_manage::KEEP => frame.out.actions.push(Action::Cancel),
                        _ => {}
                    }
                }
            }
            (None, false) => {}
        }
        self.question
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let (Screen::SavedGames(data)
        | Screen::Playing {
            overlay: Some(Overlay::SavedGames(data)),
        }) = screen
        else {
            return;
        };
        self.frame_with(data, frame);
    }
}

impl Open {
    fn frame_with(&mut self, data: &SavedGames, frame: &mut ScreenFrame<'_>) {
        let asking = self.follow_question(data, frame);
        let Some(dialog) = frame.stack.find_mut(OPSG) else {
            return;
        };
        for hit in frame.hits.iter().filter(|h| h.dialog == OPSG && !asking) {
            let action = match hit.item {
                OK => Some(Action::OpenSave),
                CANCEL => Some(Action::Cancel),
                DELETE => Some(Action::DeleteSave),
                LIST => list_of(dialog, LIST)
                    .and_then(|list| list.selected())
                    .filter(|&row| Some(row) != data.selected)
                    .map(Action::ChooseSave),
                item => {
                    tracing::info!(item, "saved games: not built yet");
                    None
                }
            };
            frame.out.actions.extend(action);
        }
        if self.shown.as_ref().is_none_or(|s| !s.matches(data)) {
            self.update(dialog, data);
        }
    }
}
