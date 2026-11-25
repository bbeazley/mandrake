use mandrake_common::FourCC;
use mandrake_session::{Action, Restoring, Screen};

use super::{OpenScreen, ScreenFrame};
use crate::core::{Dialog, DialogStack, format_one};
use crate::resources::Resources;

pub const PRDB: FourCC = FourCC(*b"prdb");
const BACKGROUND: FourCC = FourCC(*b"inbp");
const FLAGS: u16 = 0x58;
const STRINGS: FourCC = FourCC(*b"in05");
const TEXT_STRING: usize = 1;
pub const TEXT: i16 = 1;
pub const CANCEL: i16 = 2;
pub const BAR: i16 = 3;

fn format_name(format: &[u8], name: &[u8]) -> Vec<u8> {
    format_one(format, name)
}

pub fn open(
    data: &Restoring,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let def = res.dialog(PRDB)?;
    let format = res.string_line(STRINGS, TEXT_STRING).to_vec();
    let mut dialog = Dialog::new(&def, PRDB, FLAGS, res);
    dialog.set_static_text(TEXT, &format_name(&format, &data.name.0));
    dialog.set_background(res.art(BACKGROUND), 0);
    dialog.set_visible(CANCEL, data.network);
    if data.network {
        dialog.set_cancel_item(CANCEL);
    }
    let mut screen = Open { fraction: -1 };
    screen.update(&mut dialog, data);
    stack.push(dialog);
    Some(Box::new(screen))
}

struct Open {
    fraction: i16,
}

impl Open {
    fn update(&mut self, dialog: &mut Dialog, data: &Restoring) {
        if self.fraction == data.fraction {
            return;
        }
        self.fraction = data.fraction;
        if let Some((item, bar)) = dialog.control_mut(BAR).and_then(|c| c.progress_mut()) {
            bar.set_fraction(item, data.fraction);
        }
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Restoring(data) = screen else {
            return;
        };
        if data.network
            && frame
                .hits
                .iter()
                .any(|h| h.dialog == PRDB && h.item == CANCEL)
        {
            frame.out.actions.push(Action::Cancel);
        }
        if let Some(dialog) = frame.stack.find_mut(PRDB) {
            self.update(dialog, data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_goes_in_for_its_s() {
        assert_eq!(
            format_name(b"Restoring \"%s\"...", b"Quicksave 2"),
            b"Restoring \"Quicksave 2\"..."
        );
        assert_eq!(format_name(b"No name", b"x"), b"No name");
    }

    #[test]
    fn network_restore_cancel_emits_cancel_action() {
        use mandrake_session::{FrameInput, FrameOutput, Text};
        let mut open = Open { fraction: 0 };
        let data = Restoring {
            name: Text(b"NetSave".to_vec()),
            fraction: 64,
            network: true,
        };
        let mut stack = DialogStack::new();
        let mut res = Resources::from_source(|_, _| None);
        let input = FrameInput {
            now_ms: 0,
            size: (800, 600),
            events: &[],
            sound_busy: false,
            shift_held: false,
        };
        let mut out = FrameOutput::default();
        let hits = [crate::core::Hit {
            dialog: PRDB,
            item: CANCEL,
        }];
        open.frame(
            &Screen::Restoring(&data),
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
        assert_eq!(out.actions, [Action::Cancel]);
    }
}
