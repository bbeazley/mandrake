use mandrake_session::net::NetScreen;
use mandrake_session::{Action, Screen};

use super::join_failed::{INBP, PRDB, PRDB_BAR, PRDB_CANCEL, PRDB_FLAGS, PRDB_TEXT};
use super::{OpenScreen, ScreenFrame, join_failed, lobby};
use crate::core::{Dialog, DialogStack};
use crate::resources::Resources;

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let NetScreen::Downloading(lobby, download) = screen else {
        return None;
    };
    let under = lobby::open(&NetScreen::Lobby(*lobby), res, stack);
    let def = res.dialog(PRDB)?;
    let mut dialog = Dialog::new(&def, PRDB, PRDB_FLAGS, res);
    dialog.set_background(res.art(INBP), 0);
    dialog.set_cancel_item(PRDB_CANCEL);
    let text = download.text().0;
    join_failed::set_static_text(&mut dialog, PRDB_TEXT, &text);
    stack.push(dialog);
    Some(Box::new(Open {
        under,
        text,
        fraction: -1,
    }))
}

struct Open {
    under: Option<Box<dyn OpenScreen>>,
    text: Vec<u8>,
    fraction: i16,
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(NetScreen::Downloading(lobby, download)) = screen else {
            return;
        };
        if let Some(under) = &mut self.under {
            under.frame(&Screen::Net(NetScreen::Lobby(*lobby)), frame);
        }
        let text = download.text().0;
        let fraction = download.fraction();
        if let Some(dialog) = frame.stack.find_mut(PRDB) {
            if text != self.text {
                join_failed::set_static_text(dialog, PRDB_TEXT, &text);
                if let Some(c) = dialog.control_mut(PRDB_TEXT) {
                    c.item.dirty = true;
                }
                self.text = text;
            }
            if fraction != self.fraction
                && let Some((item, bar)) =
                    dialog.control_mut(PRDB_BAR).and_then(|c| c.progress_mut())
            {
                bar.set_fraction(item, fraction);
                self.fraction = fraction;
            }
        }
        if frame
            .hits
            .iter()
            .any(|h| h.dialog == PRDB && h.item == PRDB_CANCEL)
        {
            frame.out.actions.push(Action::Cancel);
        }
    }
}
