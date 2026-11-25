use mandrake_common::FourCC;
use mandrake_session::chat_commands as dot;
use mandrake_session::net::{ChatTo, HOST_ID, Lobby, LobbyScreen, NetScreen, PlayerData};
use mandrake_session::{Action, Screen, Text};

use super::join_failed::INMN;
use super::net_join::edit_text;
use super::{OpenScreen, ScreenFrame};
use crate::controls::item_flags;
use crate::controls::list::List;
use crate::controls::lobby_rows::PlayerRow;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::{BAIE, Resources};

pub const NE1: FourCC = FourCC(*b"ne#1");
pub(super) const INND: FourCC = FourCC(*b"innd");
pub(super) const NEPI: FourCC = FourCC(*b"nepi");
pub(super) const INMS: (FourCC, FourCC) = (FourCC(*b"stli"), FourCC(*b"inms"));

pub const GAME: i16 = 2;
pub const OPTIONS: i16 = 3;
pub const LEAVE: i16 = 4;
pub const START: i16 = 5;
pub const PLAYERS: i16 = 6;
const STAT: i16 = 7;
pub const CHAT: i16 = 8;
pub const CHAT_EDIT: i16 = 9;
pub const SEND: i16 = 10;
pub const TEAM_JOIN: i16 = 11;
pub const BOOT: i16 = 12;
pub const CLOSE: i16 = 13;
pub const TEAM_KICK: i16 = 14;
pub const TEAM_LOCK: i16 = 15;
pub const READY: i16 = 16;
pub const TABS: i16 = 0x7d;

pub(super) const EDIT_FLAG_ON: u32 = 0x10;
pub(super) const EDIT_FLAG_OFF: u32 = 0x20;
const SMALL_SHAPE: i32 = 0x14;
const TABS_HEIGHT: i32 = 0x16;
const TABS_WIDTH: i32 = 0x82;
const STRING_JOIN: usize = 0;
const STRING_CLOSE: usize = 15;
const STRING_OPEN: usize = 16;
const STRING_LOCK: usize = 32;
const STRING_UNREADY: usize = 35;
const STRING_READY: usize = 36;
const HOST_TEXT: &[u8] = b"host";
const UNKNOWN_LATENCY: &[u8] = b"---";
const LATENCY_SHOWN_BELOW: u16 = 1000;

const TAB_GAME: &[u8] = b"Game";
const TAB_PLUGINS: &[u8] = b"Plugins";

pub(crate) fn list_of(dialog: &mut Dialog, id: i16) -> Option<&mut List> {
    dialog.list_mut(id)
}

pub(crate) const TEAM_ROW: i32 = 0x1_0000;

pub(crate) fn selected_player(dialog: &Dialog) -> Option<i32> {
    selected_row(dialog).map(|v| v & !TEAM_ROW)
}

pub(crate) fn selected_row(dialog: &Dialog) -> Option<i32> {
    dialog.selected_list_value(PLAYERS)
}

pub(crate) fn show_game_details(
    dialog: &mut Dialog,
    id: i16,
    details: Option<mandrake_session::FilmDetails>,
    res: &mut Resources,
) {
    let art = details.as_ref().and_then(|d| {
        d.game
            .art
            .and_then(|art_id| res.art_of_plugins(&d.game.art_plugins, art_id))
            .filter(|art| {
                art.bitmap(crate::controls::game::ART_SEQUENCE, -1, 0)
                    .is_some()
            })
    });
    let fallback = res.art(crate::controls::game::NO_ART);
    if let Some((item, game)) = dialog.control_mut(id).and_then(|c| c.game_mut()) {
        game.set_description(item, details, art.or(fallback));
    }
}

#[derive(Default)]
pub(crate) struct LobbyRowsCache {
    initialized: bool,
    players: Vec<PlayerData>,
    show_ping: bool,
    latencies: Vec<(i32, u16)>,
    pub(crate) rows: Vec<PlayerRow>,
}

impl LobbyRowsCache {
    fn unchanged(&self, lobby: &LobbyScreen<'_>, check_ping: bool) -> bool {
        if !self.initialized || self.players != lobby.players {
            return false;
        }
        if !check_ping {
            return true;
        }
        if self.show_ping != lobby.show_ping {
            return false;
        }
        if !lobby.show_ping {
            return true;
        }
        self.latencies.len() == lobby.players.len()
            && lobby
                .players
                .iter()
                .zip(&self.latencies)
                .all(|(p, &(id, ms))| p.net_id == id && lobby.latency(p.net_id) == ms)
    }

    pub(crate) fn sync(
        &mut self,
        lobby: &LobbyScreen<'_>,
        check_ping: bool,
        build: impl FnOnce(&LobbyScreen<'_>) -> Vec<PlayerRow>,
        dialog: &mut Dialog,
        nepi: &Option<std::sync::Arc<mandrake_view::interface_bitmap::InterfaceArt>>,
    ) -> bool {
        if self.unchanged(lobby, check_ping) {
            return false;
        }
        self.initialized = true;
        self.players.clone_from(&lobby.players);
        self.show_ping = lobby.show_ping;
        self.latencies.clear();
        if check_ping && lobby.show_ping {
            self.latencies.extend(
                lobby
                    .players
                    .iter()
                    .map(|p| (p.net_id, lobby.latency(p.net_id))),
            );
        }
        let rows = build(lobby);
        if rows != self.rows {
            if let Some(list) = dialog.list_mut(PLAYERS) {
                list.set_player_rows(rows.clone(), nepi.clone());
            }
            if let Some(c) = dialog.control_mut(PLAYERS) {
                c.item.dirty = true;
            }
            self.rows = rows;
            return true;
        }
        false
    }
}

pub(super) fn player_rows(lobby: &LobbyScreen<'_>) -> Vec<PlayerRow> {
    let ping = lobby.show_ping
        && lobby
            .player(HOST_ID)
            .is_some_and(|h| h.version > mandrake_common::game_options::VERSION_ID_NO_PINGS);
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
        rows.push(PlayerRow {
            right: right_column(&p, ping.then(|| lobby.latency(p.net_id))),
            ..row_of(&p)
        });
    }
    rows
}

fn right_column(p: &PlayerData, latency: Option<u16>) -> Vec<u8> {
    match latency {
        None => Vec::new(),
        Some(_) if p.net_id == HOST_ID => HOST_TEXT.to_vec(),
        Some(ms) if ms < LATENCY_SHOWN_BELOW => format!("{ms:3}ms").into_bytes(),
        Some(_) => UNKNOWN_LATENCY.to_vec(),
    }
}

pub(crate) fn until_nul(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

pub(super) fn row_of(p: &PlayerData) -> PlayerRow {
    let c = p.appearance.colors;
    let host = p.net_id == HOST_ID;
    PlayerRow {
        id: p.net_id,
        icon: p.appearance.field_00 as i16,
        name: until_nul(&p.appearance.name).to_vec(),
        colors: [[c[0], c[1], c[2], c[3]], [c[4], c[5], c[6], c[7]]],
        ready: p.ready != 0,
        host,
        in_team: p.team != -1,
        heading: false,
        right: right_column(p, Some(u16::MAX)),
        right_under_bar: false,
    }
}

pub(crate) fn chat_color(rows: &[PlayerRow], name: &[u8]) -> [u16; 3] {
    rows.iter()
        .find(|r| r.name == name)
        .map_or([0; 3], |r| [r.colors[0][0], r.colors[0][1], r.colors[0][2]])
}

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let NetScreen::Lobby(lobby) = screen else {
        return None;
    };
    let def = res.dialog(NE1)?;
    let mut dialog = Dialog::new(&def, NE1, dialog_flags::MODAL, res);
    dialog.set_background(res.art(INND), 0);
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
    dialog.set_enabled(SEND, false);
    dialog.set_keyboard_equivalent(OPTIONS, 'O', true);
    dialog.set_keyboard_equivalent(START, 'N', true);
    dialog.set_keyboard_equivalent(CLOSE, 'W', true);
    let host = lobby.host_controls;
    dialog.set_enabled(START, host && lobby.can_start);
    dialog.set_enabled(OPTIONS, host);
    dialog.set_enabled(CLOSE, host);
    dialog.set_visible(CLOSE, host);
    dialog.set_visible(BOOT, host);
    dialog.set_enabled(BOOT, false);
    dialog.set_enabled(READY, !lobby.host);
    dialog.set_enabled(TEAM_KICK, false);
    dialog.set_enabled(TEAM_LOCK, true);
    let baie = res.art(BAIE);
    for id in [READY, TEAM_KICK, TEAM_LOCK, TEAM_JOIN] {
        if let Some((item, b)) = dialog.control_mut(id).and_then(|c| c.button_mut()) {
            b.set_shape(item, baie.clone(), SMALL_SHAPE);
        }
    }
    if let Some(game) = dialog.control(GAME).map(|c| c.item.rect) {
        let (left, top) = (game.left + 2, game.top - TABS_HEIGHT);
        let rect = mandrake_tag::Rect::new(top, left, top + TABS_HEIGHT, left + TABS_WIDTH);
        dialog.add_control(FourCC(*b"tabc"), TABS, rect, b"", res);
        if let Some((item, tabs)) = dialog.control_mut(TABS).and_then(|c| c.tabs_mut()) {
            tabs.set_count(item, 2);
            tabs.set_name(item, 0, TAB_GAME);
            tabs.set_name(item, 1, TAB_PLUGINS);
            tabs.set_enabled(0, true);
            tabs.set_enabled(1, lobby.description.parameters.plugin_count != 0);
            tabs.set_selected(item, 0);
        }
    }
    let mut screen = Open {
        nepi: res.art_beside(NEPI, INMS),
        rows: LobbyRowsCache::default(),
        chat_lines: 0,
        reply_lines: lobby.replies.added(),
        details: None,
    };
    screen.update(lobby, &mut dialog, res);
    stack.push(dialog);
    Some(Box::new(screen))
}

struct Open {
    nepi: Option<std::sync::Arc<mandrake_view::interface_bitmap::InterfaceArt>>,
    rows: LobbyRowsCache,
    chat_lines: u64,
    reply_lines: u64,
    details: Option<mandrake_session::FilmDetails>,
}

fn update_selection_buttons(dialog: &mut Dialog, lobby: &Lobby) {
    let chosen = selected_player(dialog);
    let heading = selected_row(dialog).is_some_and(|v| v & TEAM_ROW != 0);
    let boot = !heading && chosen.is_some_and(|id| id != lobby.local_id);
    dialog.set_enabled_if(BOOT, boot);
    dialog.set_enabled_if(TEAM_KICK, lobby.team_kick_enabled(chosen));
}

impl Open {
    fn update(&mut self, lobby: &LobbyScreen<'_>, dialog: &mut Dialog, res: &mut Resources) {
        if self.rows.sync(lobby, true, player_rows, dialog, &self.nepi)
            && selected_player(dialog).is_none()
        {
            dialog.set_enabled_if(BOOT, false);
            dialog.set_enabled_if(TEAM_KICK, false);
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
        add_replies(lobby, dialog, &self.rows.rows, &mut self.reply_lines);
        if lobby.details != self.details {
            self.details = lobby.details.clone();
            show_game_details(dialog, GAME, self.details.clone(), res);
        }
        let host = lobby.host_controls;
        let me = lobby.player(lobby.local_id);
        let chosen = selected_player(dialog);
        let team = lobby.team_button(chosen);
        dialog.set_enabled_if(TEAM_JOIN, team.enabled);
        if let Some(name) = team.name {
            dialog.set_button_caption_if_changed(
                TEAM_JOIN,
                res.string_line(INMN, STRING_JOIN + name),
            );
        }
        let locked_team = me.is_some_and(|p| p.team_locked != 0);
        dialog.set_button_caption_if_changed(
            TEAM_LOCK,
            res.string_line(INMN, STRING_LOCK + usize::from(locked_team)),
        );
        let leader = me.is_some_and(|p| p.team != -1 && p.team == p.net_id);
        dialog.set_enabled_if(TEAM_LOCK, leader);
        let ready = me.is_some_and(|p| p.ready != 0) || lobby.host;
        let name = if ready { STRING_UNREADY } else { STRING_READY };
        dialog.set_button_caption_if_changed(READY, res.string_line(INMN, name));
        dialog.set_enabled_if(START, host && lobby.can_start);
        dialog.set_enabled_if(OPTIONS, host);
        dialog.set_visible_if(CLOSE, host);
        dialog.set_enabled_if(CLOSE, host);
        let close = if lobby.locked {
            STRING_OPEN
        } else {
            STRING_CLOSE
        };
        dialog.set_button_caption_if_changed(CLOSE, res.string_line(INMN, close));
        dialog.set_visible_if(BOOT, host);
        let typed = !dialog.edit_text_slice(CHAT_EDIT).is_empty();
        dialog.set_enabled_if(SEND, typed);
    }

    fn hits(&mut self, lobby: &Lobby, frame: &mut ScreenFrame<'_>) {
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
                PLAYERS => update_selection_buttons(dialog, lobby),
                OPTIONS if lobby.host_controls => frame.out.actions.push(Action::OpenOptions),
                LEAVE => frame.out.actions.push(Action::Leave),
                START if lobby.can_start => frame.out.actions.push(Action::StartNetGame),
                READY if !lobby.host => frame.out.actions.push(Action::SetReady(!lobby.ready())),
                CLOSE if lobby.host_controls => frame.out.actions.push(Action::Lock(!lobby.locked)),
                TEAM_JOIN => {
                    frame
                        .out
                        .actions
                        .push(Action::TeamButton(selected_player(dialog)));
                    if let Some(list) = list_of(dialog, PLAYERS) {
                        list.clear_selection();
                    }
                    if let Some(c) = dialog.control_mut(PLAYERS) {
                        c.item.dirty = true;
                    }
                    dialog.set_enabled(BOOT, false);
                    dialog.set_enabled(TEAM_KICK, false);
                }
                TEAM_KICK => {
                    if let Some(id) = selected_player(dialog) {
                        frame.out.actions.push(Action::TeamKick(id));
                    }
                    dialog.set_enabled(TEAM_KICK, false);
                }
                TEAM_LOCK => frame.out.actions.push(Action::TeamLock),
                BOOT if lobby.host_controls => {
                    if let Some(id) = selected_player(dialog) {
                        frame.out.actions.push(Action::Kick(id));
                        dialog.set_enabled(BOOT, false);
                    }
                }
                SEND => send_chat(dialog, &mut frame.out.actions, true),
                _ => {}
            }
        }
    }
}

pub(super) fn send_chat(dialog: &mut Dialog, actions: &mut Vec<Action>, lobby: bool) {
    let text = edit_text(dialog, CHAT_EDIT);
    if text.is_empty() {
        return;
    }
    if let Some(command) = dot::parse(&text) {
        if command == dot::DotCommand::Ping
            && let Some(c) = dialog.control_mut(PLAYERS)
        {
            c.item.dirty = true;
        }
        actions.push(Action::ChatCommand(command));
    } else {
        match selected_row(dialog) {
            None if dot::goes_nowhere(&text) => {}
            None => actions.push(Action::LobbyChat(Text(text))),
            Some(row) if row & TEAM_ROW != 0 => {
                let leader = u16::try_from(row & !TEAM_ROW).unwrap_or(u16::MAX);
                actions.push(Action::LobbyChatTo(ChatTo::Team(leader), Text(text)));
            }
            Some(id) => {
                let to = ChatTo::Player(u16::try_from(id).unwrap_or(u16::MAX));
                actions.push(Action::LobbyChatTo(to, Text(text)));
                if let Some(list) = list_of(dialog, PLAYERS) {
                    list.clear_selection();
                }
                if let Some(c) = dialog.control_mut(PLAYERS) {
                    c.item.dirty = true;
                }
                if lobby {
                    dialog.set_enabled(BOOT, false);
                    dialog.set_enabled(TEAM_KICK, false);
                }
            }
        }
    }
    if let Some((item, e)) = dialog.control_mut(CHAT_EDIT).and_then(|c| c.edit_mut()) {
        e.set_text(item, b"");
    }
    dialog.set_enabled(SEND, false);
}

pub(super) fn add_replies(
    lobby: &LobbyScreen<'_>,
    dialog: &mut Dialog,
    rows: &[PlayerRow],
    seen: &mut u64,
) {
    if lobby.replies.added() < *seen {
        *seen = 0;
    }
    for line in lobby.replies.since(*seen) {
        let color = chat_color(rows, &line.name.0);
        if let Some(list) = list_of(dialog, CHAT) {
            list.add_chat_of_kind(line.kind.code(), &line.name.0, color, &line.text.0);
        }
        if let Some(c) = dialog.control_mut(CHAT) {
            c.item.dirty = true;
        }
    }
    *seen = lobby.replies.added();
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(NetScreen::Lobby(lobby)) = screen else {
            return;
        };
        self.hits(lobby, frame);
        if let Some(dialog) = frame.stack.find_mut(NE1) {
            self.update(lobby, dialog, frame.res);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_line_takes_its_senders_list_colour() {
        let row = PlayerRow {
            id: 1,
            icon: 0,
            name: b"Ours".to_vec(),
            colors: [[1, 2, 3, 0], [4, 5, 6, 0]],
            ready: false,
            host: false,
            in_team: false,
            heading: false,
            right: Vec::new(),
            right_under_bar: false,
        };
        assert_eq!(chat_color(std::slice::from_ref(&row), b"Ours"), [1, 2, 3]);
        assert_eq!(chat_color(&[row], b"Gone"), [0; 3]);
    }
}
