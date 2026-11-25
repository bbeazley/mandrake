use mandrake_common::FourCC;
use mandrake_session::net::{LobbyScreen, NetPostgame, NetScreen};
use mandrake_session::saves::{AUTOSAVE_PREFIX, LocalZone, autosave_name};
use mandrake_session::{Action, Screen, Text};

use super::film_report::{Carnage, CarnageItems};
use super::join_failed::INMN;
use super::lobby::{
    EDIT_FLAG_OFF, EDIT_FLAG_ON, INMS, INND, LobbyRowsCache, NEPI, chat_color, list_of,
};
use super::{OpenScreen, ScreenFrame};
use crate::controls::item_flags;
use crate::controls::lobby_rows::PlayerRow;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const END: FourCC = FourCC(*b"!END");
pub const ENDI: FourCC = FourCC(*b"endi");
const NPG: FourCC = FourCC(*b"!npg");

const BANNER: i16 = 1;
const STRING_NOT_CONNECTED: usize = 22;
pub const SAVE_REPLAY: i16 = 3;
pub const LEAVE: i16 = 4;
pub const RESTART: i16 = 5;
pub const PLAYERS: i16 = 6;
pub const CHAT: i16 = 8;
pub const CHAT_EDIT: i16 = 9;
pub const SEND: i16 = 10;
const ITEMS: CarnageItems = CarnageItems {
    header: 11,
    stats: 12,
    graph: 13,
    first_tab: 14,
    frame: false,
};
pub const TABS: i16 = 0x1e;
const TAB_COUNT: usize = 7;

fn recording_time_string(unix_secs: i64) -> Vec<u8> {
    autosave_name(unix_secs, &LocalZone::system())[AUTOSAVE_PREFIX.len()..].to_vec()
}

fn player_rows(lobby: &LobbyScreen<'_>) -> Vec<PlayerRow> {
    super::lobby::player_rows(lobby)
        .into_iter()
        .map(|r| PlayerRow {
            ready: false,
            host: false,
            ..r
        })
        .collect()
}

#[derive(Clone, Copy)]
pub struct Style {
    pub rows: fn(&LobbyScreen<'_>) -> Vec<PlayerRow>,
    pub back_log: bool,
}

pub const MYTH2: Style = Style {
    rows: player_rows,
    back_log: true,
};

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    open_with(screen, res, stack, MYTH2)
}

pub fn open_with(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
    style: Style,
) -> Option<Box<dyn OpenScreen>> {
    let NetScreen::Postgame(end, lobby) = screen else {
        return None;
    };
    let (id, def) = match res.dialog(END) {
        Some(def) => (END, def),
        None => (ENDI, res.dialog(ENDI)?),
    };
    let mut dialog = Dialog::new(&def, id, dialog_flags::MODAL, res);
    let background = if id == END { res.art(NPG) } else { None };
    dialog.set_background(background.or_else(|| res.art(INND)), 0);
    if let Some((item, e)) = dialog.control_mut(CHAT_EDIT).and_then(|c| c.edit_mut()) {
        e.set_flag(item, EDIT_FLAG_ON, true);
        e.set_flag(item, EDIT_FLAG_OFF, false);
    }
    dialog.set_focus(CHAT_EDIT);
    if let Some(c) = dialog.control_mut(SEND) {
        c.item.flags |= item_flags::REPORT_AT_PRESS;
    }
    dialog.set_default_item(SEND);
    dialog.set_cancel_item(LEAVE);
    dialog.set_keyboard_equivalent(SAVE_REPLAY, 'S', true);
    dialog.set_keyboard_equivalent(RESTART, 'N', true);
    dialog.set_enabled(RESTART, end.can_restart);
    dialog.set_enabled(SEND, false);
    dialog.set_enabled(CHAT_EDIT, end.can_chat);
    dialog.set_visible(BANNER, false);
    let carnage = Carnage::open(
        &mut dialog,
        ITEMS,
        end.report.pages.clone().unwrap_or_default(),
        res,
    );
    if dialog.control(TABS).is_some() {
        let tabs: Vec<(Vec<u8>, bool, bool)> = (0..TAB_COUNT as i16)
            .map(|n| {
                let control = dialog.control_mut(ITEMS.first_tab + n);
                let enabled = control.as_ref().is_some_and(|c| c.item.enabled());
                let (name, pressed) = control
                    .and_then(|c| c.button_mut())
                    .map(|(_, b)| (b.caption().to_vec(), b.state() != 0))
                    .unwrap_or_default();
                (name, enabled, pressed)
            })
            .collect();
        if let Some((item, strip)) = dialog.control_mut(TABS).and_then(|c| c.tabs_mut()) {
            strip.set_count(item, TAB_COUNT);
            for (n, (name, enabled, pressed)) in tabs.iter().enumerate() {
                strip.set_name(item, n, name);
                strip.set_enabled(n, *enabled);
                if *pressed {
                    strip.set_selected(item, n);
                }
            }
        }
        for n in 0..TAB_COUNT as i16 {
            dialog.set_visible(ITEMS.first_tab + n, false);
        }
        dialog.set_enabled(TABS, true);
    }
    let mut open = Open {
        id,
        nepi: res.art_beside(NEPI, INMS),
        rows: LobbyRowsCache::default(),
        chat_lines: if style.back_log {
            0
        } else {
            lobby.chat.added()
        },
        reply_lines: lobby.replies.added(),
        style,
        carnage,
        can_chat: end.can_chat,
    };
    open.update(lobby, &mut dialog);
    if !end.can_chat {
        let color = chat_color(&open.rows.rows, &[]);
        for n in [STRING_NOT_CONNECTED, STRING_NOT_CONNECTED + 1] {
            let line = res.string_line(INMN, n);
            if let Some(list) = list_of(&mut dialog, CHAT) {
                list.add_chat_of_kind(2, &[], color, line);
            }
        }
    }
    tracing::info!(dialog = ?id, "net postgame: endi opens");
    stack.push(dialog);
    Some(Box::new(open))
}

struct Open {
    id: FourCC,
    nepi: Option<std::sync::Arc<mandrake_view::interface_bitmap::InterfaceArt>>,
    rows: LobbyRowsCache,
    chat_lines: u64,
    reply_lines: u64,
    carnage: Carnage,
    can_chat: bool,
    style: Style,
}

impl Open {
    fn update(&mut self, lobby: &LobbyScreen<'_>, dialog: &mut Dialog) {
        let build_rows = self.style.rows;
        self.rows.sync(lobby, true, build_rows, dialog, &self.nepi);
        if lobby.chat.added() < self.chat_lines {
            self.chat_lines = 0;
        }
        for line in lobby.chat.since(self.chat_lines) {
            let color = chat_color(&self.rows.rows, &line.name.0);
            if let Some(list) = dialog.list_mut(CHAT) {
                list.add_chat_of_kind(line.kind.code(), &line.name.0, color, &line.text.0);
            }
            if let Some(c) = dialog.control_mut(CHAT) {
                c.item.dirty = true;
            }
        }
        self.chat_lines = lobby.chat.added();
        super::lobby::add_replies(lobby, dialog, &self.rows.rows, &mut self.reply_lines);
        let typed = !dialog.edit_text_slice(CHAT_EDIT).is_empty();
        dialog.set_enabled_if(SEND, typed && self.can_chat);
    }

    fn hits(&mut self, end: &NetPostgame, frame: &mut ScreenFrame<'_>) {
        let id = self.id;
        let Some(dialog) = frame.stack.find_mut(id) else {
            return;
        };
        let hits: Vec<i16> = frame
            .hits
            .iter()
            .filter(|h| h.dialog == id)
            .map(|h| h.item)
            .collect();
        for item in hits {
            match item {
                SAVE_REPLAY => {
                    let unix_secs = frame.res.unix_secs(frame.input.now_ms);
                    let name = Text(recording_time_string(unix_secs));
                    frame.out.actions.push(Action::SaveReplay { name });
                    dialog.set_enabled(SAVE_REPLAY, false);
                }
                LEAVE => frame.out.actions.push(Action::Leave),
                RESTART if end.can_restart => frame.out.actions.push(Action::RestartNetGame),
                SEND if self.can_chat => {
                    super::lobby::send_chat(dialog, &mut frame.out.actions, false);
                }
                TABS => {
                    let tab = dialog
                        .control(TABS)
                        .and_then(|c| c.tabs())
                        .map_or(0, |t| t.selected() as i16);
                    self.carnage.show(dialog, ITEMS.first_tab + tab);
                }
                tab if self.carnage.is_tab(tab) => self.carnage.show(dialog, tab),
                _ => {}
            }
        }
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(NetScreen::Postgame(end, lobby)) = screen else {
            return;
        };
        self.hits(end, frame);
        if let Some(dialog) = frame.stack.find_mut(self.id) {
            self.update(lobby, dialog);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_time_string_formats_without_system_time() {
        let s = recording_time_string(0);
        assert_eq!(s.len(), "1970-01-01 00.00.00".len());
        assert_eq!(s[4], b'-');
        assert_eq!(s[7], b'-');
        assert_eq!(s[10], b' ');
        assert_eq!(s[13], b'.');
        assert_eq!(s[16], b'.');
    }
}
