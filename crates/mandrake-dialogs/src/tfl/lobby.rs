use mandrake_common::FourCC;
use mandrake_session::net::{ChatTo, Lobby, LobbyScreen, NetScreen, PlayerData};
use mandrake_session::{Action, Screen, Text};

use crate::controls::item_flags;
use crate::controls::lobby_rows::PlayerRow;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;
use crate::screens::lobby::{
    LobbyRowsCache, TEAM_ROW, chat_color, selected_player, selected_row, show_game_details,
    until_nul,
};
use crate::screens::{OpenScreen, ScreenFrame};

pub const NE1: FourCC = FourCC(*b"ne#1");
const MEGR: FourCC = FourCC(*b"megr");
const NEPI: FourCC = FourCC(*b"nepi");
const INMN: FourCC = FourCC(*b"inmn");

const BANNER: i16 = 1;
pub const GAME: i16 = 2;
pub const OPTIONS: i16 = 3;
pub const LEAVE: i16 = 4;
pub const START: i16 = 5;
pub const PLAYERS: i16 = 6;
const STAT: i16 = 7;
pub const CHAT: i16 = 8;
pub const CHAT_EDIT: i16 = 9;
pub const SEND: i16 = 10;
pub const TEAM: i16 = 11;
pub const BOOT: i16 = 12;
pub const CLOSE: i16 = 13;

const EDIT_FLAG_ON: u32 = 0x10;
const EDIT_FLAG_OFF: u32 = 0x20;
const CHAT_LENGTH: usize = 0xff;
const STRING_JOIN: usize = 0;
const STRING_CLOSE: usize = 15;
const STRING_OPEN: usize = 16;

const CLIENTS: [(u16, u16, &[u8]); 7] = [
    (1000, 1999, b"M1"),
    (2000, 2999, b"M2"),
    (3000, 3999, b"M3"),
    (4000, 4999, b"M4"),
    (5000, 5999, b"A1"),
    (6000, 6999, b"MC"),
    (10000, 10999, b"ML"),
];

pub fn client_label(version: u16) -> Vec<u8> {
    if version == 0 {
        return Vec::new();
    }
    CLIENTS
        .iter()
        .find(|(low, high, _)| (*low..=*high).contains(&version))
        .map_or_else(Vec::new, |(_, _, name)| name.to_vec())
}

fn row_of(p: &PlayerData) -> PlayerRow {
    let c = p.appearance.colors;
    PlayerRow {
        id: p.net_id,
        icon: p.appearance.field_00 as i16,
        name: until_nul(&p.appearance.name).to_vec(),
        colors: [[c[0], c[1], c[2], c[3]], [c[4], c[5], c[6], c[7]]],
        ready: false,
        host: false,
        in_team: p.team != -1,
        heading: false,
        right: client_label(p.version),
        right_under_bar: true,
    }
}

pub(crate) fn player_rows(lobby: &LobbyScreen<'_>) -> Vec<PlayerRow> {
    let mut rows = Vec::new();
    for p in lobby.listed() {
        if p.team != -1 && p.team == p.net_id {
            rows.push(PlayerRow {
                id: TEAM_ROW | p.net_id,
                name: until_nul(&p.appearance.team_name).to_vec(),
                heading: true,
                right: Vec::new(),
                ..row_of(&p)
            });
        }
        rows.push(row_of(&p));
    }
    rows
}

fn edit_text(dialog: &Dialog, id: i16) -> Vec<u8> {
    let text = dialog.edit_text_slice(id);
    text[..text.len().min(CHAT_LENGTH)].to_vec()
}

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let mut screens = Screens {
        lobby: None,
        options: None,
    };
    match screen {
        NetScreen::Lobby(lobby) => {
            let dialog = dialog(lobby, res)?;
            let mut open = Open::new(lobby, res);
            stack.push(dialog);
            if let Some(d) = stack.find_mut(NE1) {
                open.update(lobby, d, res);
            }
            screens.lobby = Some(open);
        }
        NetScreen::GameOptions(options) => {
            screens.options = Some(super::options::Open::push(options, res, stack)?);
        }
        _ => return None,
    }
    Some(Box::new(screens))
}

struct Screens {
    lobby: Option<Open>,
    options: Option<super::options::Open>,
}

impl OpenScreen for Screens {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(net) = screen else { return };
        match net {
            NetScreen::GameOptions(options) => {
                if self.options.is_none() {
                    self.options = super::options::Open::push(options, frame.res, frame.stack);
                }
                if let Some(o) = self.options.as_mut() {
                    o.frame(options, frame);
                }
            }
            NetScreen::Lobby(lobby) => {
                if self.options.take().is_some()
                    && frame.stack.find_mut(super::options::NEPD).is_some()
                {
                    frame.stack.pop();
                }
                if self.lobby.is_none() {
                    let Some(dialog) = dialog(lobby, frame.res) else {
                        return;
                    };
                    frame.stack.push(dialog);
                    self.lobby = Some(Open::new(lobby, frame.res));
                }
                let Some(open) = self.lobby.as_mut() else {
                    return;
                };
                open.hits(lobby, frame);
                if let Some(dialog) = frame.stack.find_mut(NE1) {
                    open.update(lobby, dialog, frame.res);
                }
            }
            _ => {}
        }
    }
}

pub(super) fn dialog(lobby: &Lobby, res: &mut Resources) -> Option<Dialog> {
    let def = res.dialog(NE1)?;
    let mut dialog = Dialog::new(&def, NE1, dialog_flags::MODAL, res);
    dialog.set_background(res.art(MEGR), 0);
    dialog.set_visible(STAT, false);
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
    dialog.set_keyboard_equivalent(OPTIONS, 'O', true);
    dialog.set_keyboard_equivalent(START, 'N', true);
    dialog.set_keyboard_equivalent(CLOSE, 'W', true);
    dialog.set_enabled(SEND, false);
    let host = lobby.host_controls;
    dialog.set_enabled(START, host && lobby.can_start);
    dialog.set_enabled(OPTIONS, host);
    dialog.set_visible(BOOT, host);
    dialog.set_visible(CLOSE, host);
    dialog.set_visible(BANNER, false);
    dialog.set_enabled(BOOT, false);
    Some(dialog)
}

pub(super) struct Open {
    nepi: Option<std::sync::Arc<mandrake_view::interface_bitmap::InterfaceArt>>,
    rows: LobbyRowsCache,
    chat_lines: u64,
    details: Option<mandrake_session::FilmDetails>,
}

fn update_boot_enabled(dialog: &mut Dialog, lobby: &Lobby) {
    let heading = selected_row(dialog).is_some_and(|v| v & TEAM_ROW != 0);
    let boot = !heading && selected_player(dialog).is_some_and(|id| id != lobby.local_id);
    dialog.set_enabled_if(BOOT, boot);
}

impl Open {
    pub(super) fn new(_lobby: &Lobby, res: &mut Resources) -> Self {
        Self {
            nepi: res.art(NEPI),
            rows: LobbyRowsCache::default(),
            chat_lines: 0,
            details: None,
        }
    }

    pub(super) fn update(
        &mut self,
        lobby: &LobbyScreen<'_>,
        dialog: &mut Dialog,
        res: &mut Resources,
    ) {
        if self
            .rows
            .sync(lobby, false, player_rows, dialog, &self.nepi)
            && selected_player(dialog).is_none()
        {
            dialog.set_enabled_if(BOOT, false);
        }
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
        if lobby.details != self.details {
            self.details = lobby.details.clone();
            show_game_details(dialog, GAME, self.details.clone(), res);
        }
        let host = lobby.host_controls;
        let team = lobby.team_button(selected_player(dialog));
        dialog.set_enabled_if(TEAM, team.enabled);
        if let Some(name) = team.name {
            dialog.set_button_caption_if_changed(TEAM, res.string_line(INMN, STRING_JOIN + name));
        }
        dialog.set_enabled_if(START, host && lobby.can_start);
        dialog.set_enabled_if(OPTIONS, host);
        dialog.set_visible_if(CLOSE, host);
        dialog.set_visible_if(BOOT, host);
        let close = if lobby.locked {
            STRING_OPEN
        } else {
            STRING_CLOSE
        };
        dialog.set_button_caption_if_changed(CLOSE, res.string_line(INMN, close));
        let typed = !dialog.edit_text_slice(CHAT_EDIT).is_empty();
        dialog.set_enabled_if(SEND, typed);
    }

    pub(super) fn hits(&mut self, lobby: &Lobby, frame: &mut ScreenFrame<'_>) {
        let Some(dialog) = frame.stack.find_mut(NE1) else {
            return;
        };
        let hits: Vec<i16> = frame
            .hits
            .iter()
            .filter(|h| h.dialog == NE1)
            .map(|h| h.item)
            .collect();
        for item in hits {
            match item {
                PLAYERS => update_boot_enabled(dialog, lobby),
                OPTIONS if lobby.host_controls => frame.out.actions.push(Action::OpenOptions),
                LEAVE => frame.out.actions.push(Action::Leave),
                START if lobby.can_start => frame.out.actions.push(Action::StartNetGame),
                CLOSE if lobby.host_controls => frame.out.actions.push(Action::Lock(!lobby.locked)),
                TEAM => {
                    frame
                        .out
                        .actions
                        .push(Action::TeamButton(selected_player(dialog)));
                    if let Some(list) = dialog.list_mut(PLAYERS) {
                        list.clear_selection();
                    }
                    if let Some(c) = dialog.control_mut(PLAYERS) {
                        c.item.dirty = true;
                    }
                    dialog.set_enabled_if(BOOT, false);
                }
                BOOT if lobby.host_controls => {
                    if let Some(id) = selected_player(dialog) {
                        frame.out.actions.push(Action::Kick(id));
                        dialog.set_enabled(BOOT, false);
                    }
                }
                SEND => send_chat(dialog, &mut frame.out.actions),
                _ => {}
            }
        }
    }
}

pub(super) fn show_details(
    dialog: &mut Dialog,
    id: i16,
    details: Option<mandrake_session::FilmDetails>,
    res: &mut Resources,
) {
    show_game_details(dialog, id, details, res);
}

fn send_chat(dialog: &mut Dialog, actions: &mut Vec<Action>) {
    let text = edit_text(dialog, CHAT_EDIT);
    if text.is_empty() {
        return;
    }
    match selected_row(dialog) {
        None => actions.push(Action::LobbyChat(Text(text))),
        Some(row) if row & TEAM_ROW != 0 => {
            let leader = u16::try_from(row & !TEAM_ROW).unwrap_or(u16::MAX);
            actions.push(Action::LobbyChatTo(ChatTo::Team(leader), Text(text)));
        }
        Some(id) => {
            let to = ChatTo::Player(u16::try_from(id).unwrap_or(u16::MAX));
            actions.push(Action::LobbyChatTo(to, Text(text)));
            if let Some(list) = dialog.list_mut(PLAYERS) {
                list.clear_selection();
            }
            if let Some(c) = dialog.control_mut(PLAYERS) {
                c.item.dirty = true;
            }
            dialog.set_enabled(BOOT, false);
        }
    }
    dialog.set_edit_text(CHAT_EDIT, b"");
    dialog.set_enabled(SEND, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_right_column_names_the_records_client() {
        assert_eq!(client_label(0), b"");
        assert_eq!(client_label(1500), b"M1");
        assert_eq!(client_label(0x889), b"M2");
        assert_eq!(client_label(10500), b"ML");
        assert_eq!(client_label(7000), b"");
    }
}
