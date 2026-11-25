use mandrake_common::FourCC;
use mandrake_session::net::{NetScreen, Plugin};
use mandrake_session::{Action, Screen};
use mandrake_tag::Rect;

use super::{OpenScreen, ScreenFrame, lobby};
use crate::controls::mpll::MissingPluginRow;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const SRSW: FourCC = FourCC(*b"srsw");
const MPLL: FourCC = FourCC(*b"mpll");
const BUTT: FourCC = FourCC(*b"butt");
pub const OK: i16 = 1;
const SECOND_BUTTON: i16 = 2;
const LIST_SLOT: i16 = 3;
const TITLE: i16 = 4;
pub const LIST: i16 = 5;
pub const RELOAD: i16 = 6;
const LIST_HEIGHT: i32 = 0xcc;
const TITLE_DOWN: i32 = 8;
const RELOAD_RECT: Rect = Rect::new(0xfd, 0x1a, 0xfd + 0x16, 0x1a + 0x3c);
const TITLE_TEXT: &[u8] = b"Missing Plugins:";
const LIST_NAME: &[u8] = b"Plugins";
const RELOAD_NAME: &[u8] = b"Reload";

pub fn rows_of(plugins: &[Plugin]) -> Vec<MissingPluginRow> {
    let until_nul = |b: &[u8]| b.split(|&c| c == 0).next().unwrap_or_default().to_vec();
    plugins
        .iter()
        .map(|p| MissingPluginRow::new(&until_nul(&p.name), &until_nul(&p.url), p.checksum))
        .collect()
}

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let NetScreen::MissingPlugins(lobby, plugins) = screen else {
        return None;
    };
    let under = lobby::open(&NetScreen::Lobby(*lobby), res, stack);
    let def = res.dialog(SRSW)?;
    let mut dialog = Dialog::new(&def, SRSW, dialog_flags::MODAL, res);
    dialog.set_default_item(OK);
    dialog.set_visible(SECOND_BUTTON, false);
    let slot = dialog.control(LIST_SLOT).map(|c| c.item.rect)?;
    dialog.set_visible(LIST_SLOT, false);
    let ok = dialog.control(OK).map(|c| c.item.rect)?;
    let rect = Rect::new(slot.top, slot.left, slot.top + LIST_HEIGHT, ok.right);
    dialog.add_control(MPLL, LIST, rect, LIST_NAME, res);
    if let Some(c) = dialog.control_mut(TITLE) {
        c.item.rect = Rect::new(
            c.item.rect.top + TITLE_DOWN,
            c.item.rect.left,
            c.item.rect.bottom + TITLE_DOWN,
            c.item.rect.right,
        );
    }
    if let Some((item, t)) = dialog.control_mut(TITLE).and_then(|c| c.static_text_mut()) {
        t.set_text(item, TITLE_TEXT);
    }
    dialog.add_control(BUTT, RELOAD, RELOAD_RECT, RELOAD_NAME, res);
    let rows = rows_of(plugins);
    if let Some(list) = dialog.control_mut(LIST).and_then(|c| c.list_mut()) {
        list.set_missing_plugin_rows(rows.clone());
    }
    stack.push(dialog);
    Some(Box::new(Open { under, rows }))
}

struct Open {
    under: Option<Box<dyn OpenScreen>>,
    rows: Vec<MissingPluginRow>,
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(NetScreen::MissingPlugins(lobby, plugins)) = screen else {
            return;
        };
        if let Some(under) = &mut self.under {
            under.frame(&Screen::Net(NetScreen::Lobby(*lobby)), frame);
        }
        let rows = rows_of(plugins);
        if rows != self.rows
            && let Some(c) = frame.stack.find_mut(SRSW).and_then(|d| d.control_mut(LIST))
        {
            if let Some(list) = c.list_mut() {
                list.set_missing_plugin_rows(rows.clone());
            }
            c.item.dirty = true;
            self.rows = rows;
        }
        let hits: Vec<i16> = frame
            .hits
            .iter()
            .filter(|h| h.dialog == SRSW)
            .map(|h| h.item)
            .collect();
        for item in hits {
            match item {
                OK => frame.out.actions.push(Action::Ok),
                RELOAD => frame.out.actions.push(Action::ReloadPlugins),
                _ => {}
            }
        }
    }
}
