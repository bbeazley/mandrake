use mandrake_session::net::NetScreen;
use mandrake_session::{Action, Screen};

use super::join_failed::{INMN, PRDN};
use super::{OpenScreen, ScreenFrame};
use crate::core::{Dialog, DialogStack, dialog_flags, format_one};
use crate::resources::Resources;
use mandrake_common::FourCC;
use mandrake_session::Text;

const INNB: FourCC = FourCC(*b"innb");
pub const STRING_END_BARRIER: usize = 5;
pub const STRING_WAITING_FOR: usize = 4;
pub const BOX_MS: u64 = 1000;
const TEXT: i16 = 1;
const BUTTON: i16 = 2;

pub fn open(_res: &mut Resources, _stack: &mut DialogStack) -> Option<Box<dyn OpenScreen>> {
    Some(Box::new(Open::default()))
}

#[derive(Default)]
struct Open {
    since: Option<u64>,
    shown: bool,
    named: Option<Text>,
}

fn push(res: &mut Resources, stack: &mut DialogStack) -> bool {
    let Some(def) = res.dialog(PRDN) else {
        return false;
    };
    let mut dialog = Dialog::new(&def, PRDN, dialog_flags::MODAL, res);
    dialog.set_background(res.art(INNB), 0);
    dialog.set_static_text(TEXT, res.string_line(INMN, STRING_END_BARRIER));
    stack.push(dialog);
    true
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(NetScreen::EndBarrier { waiting }) = screen else {
            return;
        };
        let now = frame.input.now_ms;
        let since = *self.since.get_or_insert(now);
        if !self.shown && now.saturating_sub(since) > BOX_MS {
            self.shown = push(frame.res, frame.stack);
            self.named = None;
        }
        if self.shown
            && let Some(name) = waiting
            && self.named.as_ref() != Some(*name)
        {
            let line = format_one(frame.res.string_line(INMN, STRING_WAITING_FOR), &name.0);
            if let Some(dialog) = frame.stack.find_mut(PRDN) {
                dialog.set_static_text(TEXT, &line);
            }
            self.named = Some((*name).clone());
        }
        if frame
            .hits
            .iter()
            .any(|h| h.dialog == PRDN && h.item == BUTTON)
        {
            frame.out.actions.push(Action::Cancel);
        }
    }
}
