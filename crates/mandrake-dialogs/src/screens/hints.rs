use mandrake_common::FourCC;
use mandrake_session::{Action, Hints, Screen};

use super::{OpenScreen, ScreenFrame};
use crate::controls::list::list_flags;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const GATI: FourCC = FourCC(*b"gati");
const INGH: FourCC = FourCC(*b"ingh");

pub const OK: i16 = 1;
pub const DONT_SHOW: i16 = 2;
const HINT_TEXT: i16 = 3;
const OBJECTIVES: i16 = 4;
pub const NEXT_TIP: i16 = 5;
pub const TABS: i16 = 6;
const HINT_COUNT: i16 = 7;

const BULLET: &[u8] = b"\xa5 ";

const GENERAL_TAB: usize = 0;
const LEVEL_TAB: usize = 1;

const TAB_COUNT: usize = 2;

fn objective_rows(objectives: &[u8]) -> impl Iterator<Item = Vec<u8>> + '_ {
    objectives
        .split(|b| *b == b'\r')
        .filter(|line| !line.is_empty())
        .map(|line| [BULLET, line].concat())
}

fn set_text(dialog: &mut Dialog, id: i16, text: &[u8]) {
    if let Some((item, stat)) = dialog.control_mut(id).and_then(|c| c.static_text_mut()) {
        stat.set_text(item, text);
    }
}

pub fn open(
    data: &Hints,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let def = res.dialog(GATI)?;
    let art = res.art(INGH);
    let mut dialog = Dialog::new(&def, GATI, dialog_flags::MODAL, res);
    dialog.set_background(art, 0);
    dialog.set_list_unfocusable(OBJECTIVES);
    if let Some(list) = dialog.control_mut(OBJECTIVES).and_then(|c| c.list_mut()) {
        list.set_flag(list_flags::ROW_STYLE, true);
        list.set_flag(list_flags::NO_ROW_CLICKS, true);
        for (i, row) in objective_rows(&data.objectives.0).enumerate() {
            let i = i as i32;
            list.add_row(&row, i, data.first_open_objective <= i);
        }
    }
    if let Some((item, tabs)) = dialog.control_mut(TABS).and_then(|c| c.tabs_mut()) {
        tabs.set_count(item, TAB_COUNT);
        for (i, name) in data.tab_names.iter().enumerate().take(TAB_COUNT) {
            tabs.set_name(item, i, &name.0);
        }
        tabs.set_selected(item, GENERAL_TAB);
        if data.level.is_empty() {
            tabs.set_enabled(LEVEL_TAB, false);
        }
    }
    let screen = HintsScreen {
        general: data.first_general,
        level: 0,
    };
    screen.show(&mut dialog, data);
    if let Some((item, check)) = dialog.control_mut(DONT_SHOW).and_then(|c| c.button_mut()) {
        check.set_state(item, i32::from(!data.show_at_start));
    }
    dialog.set_default_item(OK);
    dialog.set_cancel_item(OK);
    stack.push(dialog);
    Some(Box::new(screen))
}

struct HintsScreen {
    general: i32,
    level: i32,
}

impl HintsScreen {
    fn selected_tab(dialog: &mut Dialog) -> usize {
        dialog
            .control_mut(TABS)
            .and_then(|c| c.tabs_mut())
            .map_or(GENERAL_TAB, |(_, tabs)| tabs.selected())
    }

    fn show(&self, dialog: &mut Dialog, data: &Hints) {
        let (hints, index) = if Self::selected_tab(dialog) == GENERAL_TAB {
            (&data.general, self.general)
        } else {
            (&data.level, self.level)
        };
        let text = usize::try_from(index).ok().and_then(|i| hints.get(i));
        let count = if hints.is_empty() {
            "0 / 0".to_owned()
        } else {
            format!("{} / {}", index + 1, hints.len())
        };
        set_text(dialog, HINT_COUNT, count.as_bytes());
        set_text(
            dialog,
            HINT_TEXT,
            text.map_or(&[][..], |text| text.0.as_slice()),
        );
    }

    fn step(&mut self, tab: usize, data: &Hints, back: bool) {
        let by = if back { -1 } else { 1 };
        let (index, count) = if tab == GENERAL_TAB {
            (&mut self.general, data.general.len())
        } else {
            (&mut self.level, data.level.len())
        };
        if count != 0 {
            *index = (*index + by).rem_euclid(count as i32);
        }
    }
}

impl OpenScreen for HintsScreen {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Playing {
            overlay: Some(mandrake_session::Overlay::Hints(data)),
        } = screen
        else {
            return;
        };
        let Some(dialog) = frame.stack.find_mut(GATI) else {
            return;
        };
        for hit in frame.hits {
            match hit.item {
                OK => {
                    let checked = dialog
                        .control_mut(DONT_SHOW)
                        .and_then(|c| c.button_mut())
                        .is_some_and(|(_, check)| check.state() != 0);
                    frame.out.actions.extend([
                        Action::SetHintIndex(self.general as i16),
                        Action::SetShowHints(!checked),
                        Action::CloseHints,
                    ]);
                    return;
                }
                NEXT_TIP => {
                    let tab = Self::selected_tab(dialog);
                    self.step(tab, data, frame.input.shift_held);
                }
                _ => {}
            }
            self.show(dialog, data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objectives_are_a_bulleted_row_each() {
        let rows: Vec<_> = objective_rows(b"One.\rTwo.").collect();
        assert_eq!(rows, [b"\xa5 One.".to_vec(), b"\xa5 Two.".to_vec()]);
        assert_eq!(objective_rows(b"").count(), 0);
    }
}
