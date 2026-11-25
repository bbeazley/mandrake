use mandrake_common::FourCC;
use mandrake_session::net::{NetPlayer, NetProtocol, NetScreen, Protocol};
use mandrake_session::{Action, Screen, Text};
use mandrake_tag::Rect;
use mandrake_tag::font::{STYLE_PLAIN, text_flags};

use super::{OpenScreen, ScreenFrame, main_menu};
use crate::core::{Dialog, DialogStack, dialog_flags, format_one};
use crate::resources::Resources;

pub const NESP: FourCC = FourCC(*b"nesp");
const INMG: FourCC = FourCC(*b"inmg");
const NEPI: FourCC = FourCC(*b"nepi");
const INMS: (FourCC, FourCC) = (FourCC(*b"stli"), FourCC(*b"inms"));
const INMN: FourCC = FourCC(*b"inmn");
const FLAGS: u16 = dialog_flags::MODAL;

pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const NAME: i16 = 3;
pub const TEAM: i16 = 4;
pub const ICON: i16 = 5;
pub const PRIMARY: i16 = 6;
pub const SECONDARY: i16 = 7;
pub const METASERVER: i16 = 8;
pub const OTHER: i16 = 9;
const THIRD: i16 = 10;
const SAVE_PASSWORD: i16 = 11;
pub const PROTOCOLS: i16 = 12;
const LOGIN: i16 = 13;
const PASSWORD: i16 = 14;
pub const INFO: i16 = 15;
const UPDATE_APPEARANCE: i16 = 16;
const PROTOCOLS_LABEL: i16 = 17;
pub const SERVER: i16 = 0x32;

const RIGHT_LABELS: [i16; 5] = [25, 24, 23, 18, 19];
const CENTRED_LABELS: [i16; 2] = [21, 20];
const METASERVER_ITEMS: [i16; 6] = [18, 13, 19, 14, 16, 11];

const GUEST_PLACEHOLDER: &[u8] = b"\xdc guest \xdd";
const SERVER_NAME: &[u8] = b"Server";
const STRING_PROTOCOL_INFO: usize = 10;
const STRING_GUEST: usize = 12;
const STRING_OK: usize = 14;
const STRING_VISIT: usize = 17;
const TCP_IP_NAME: &[u8] = b"TCP/IP (Internet)";
const TCP_IP_PLAYERS: i32 = 16;
const ICON_SEQUENCE: i32 = 0;
const ICON_HIGHLIGHT: i32 = 3;
const ICON_COLUMNS: i32 = 9;
const ICON_ROWS: i32 = 7;
const NAME_LENGTH: usize = 0x1f;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Half {
    Metaserver,
    Protocols,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Appearance {
    pub(crate) name: Vec<u8>,
    pub(crate) team: Vec<u8>,
    pub(crate) icon: u16,
    pub(crate) colors: [u16; 8],
}

impl Appearance {
    pub(crate) fn of(p: &NetPlayer) -> Self {
        Self {
            name: p.name.0.clone(),
            team: p.team.0.clone(),
            icon: p.icon,
            colors: p.colors,
        }
    }

    pub(crate) fn action(&self) -> Action {
        Action::SetNetPlayer {
            name: Text(self.name.clone()),
            team: Text(self.team.clone()),
            icon: self.icon,
            colors: self.colors,
        }
    }

    pub(crate) fn matches_dialog(
        &self,
        dialog: &Dialog,
        name_id: i16,
        team_id: i16,
        icon_id: i16,
        primary_id: i16,
        secondary_id: i16,
    ) -> bool {
        let name = dialog.edit_text_slice(name_id);
        let name = &name[..name.len().min(NAME_LENGTH)];
        let team = dialog.edit_text_slice(team_id);
        let team = &team[..team.len().min(NAME_LENGTH)];
        let icon = dialog
            .control(icon_id)
            .and_then(|c| c.icon_popup())
            .map_or(0, |p| p.value() as u16);
        let (p, s) = (color_of(dialog, primary_id), color_of(dialog, secondary_id));
        self.icon == icon
            && self.colors[..4] == p
            && self.colors[4..] == s
            && self.name.as_slice() == name
            && self.team.as_slice() == team
    }

    pub(crate) fn read_dialog(
        dialog: &Dialog,
        name_id: i16,
        team_id: i16,
        icon_id: i16,
        primary_id: i16,
        secondary_id: i16,
    ) -> Self {
        let (p, s) = (color_of(dialog, primary_id), color_of(dialog, secondary_id));
        let mut colors = [0; 8];
        colors[..4].copy_from_slice(&p);
        colors[4..].copy_from_slice(&s);
        let name = dialog.edit_text_slice(name_id);
        let team = dialog.edit_text_slice(team_id);
        Self {
            name: name[..name.len().min(NAME_LENGTH)].to_vec(),
            team: team[..team.len().min(NAME_LENGTH)].to_vec(),
            icon: dialog
                .control(icon_id)
                .and_then(|c| c.icon_popup())
                .map_or(0, |p| p.value() as u16),
            colors,
        }
    }
}

pub(crate) fn protocol_info(format: &[u8], players: i32, name: &[u8]) -> Vec<u8> {
    let with_players = format_one(format, players.to_string().as_bytes());
    format_one(&with_players, name)
}

fn color_of(dialog: &Dialog, id: i16) -> [u16; 4] {
    dialog
        .control(id)
        .and_then(|c| c.color_picker())
        .map_or([0; 4], |p| p.color())
}

fn appearance(dialog: &Dialog) -> Appearance {
    Appearance::read_dialog(dialog, NAME, TEAM, ICON, PRIMARY, SECONDARY)
}

pub(crate) fn split_colors(colors: [u16; 8]) -> [[u16; 4]; 2] {
    [
        [colors[0], colors[1], colors[2], colors[3]],
        [colors[4], colors[5], colors[6], colors[7]],
    ]
}

fn set_radio_family(dialog: &mut Dialog, chosen: i16) {
    for id in METASERVER..=THIRD {
        dialog.set_checked(id, id == chosen);
    }
}

fn show_half(dialog: &mut Dialog, half: Half, res: &mut Resources) {
    let chosen = match half {
        Half::Metaserver => {
            dialog.set_static_text(INFO, res.string_line(INMN, STRING_VISIT));
            dialog.set_enabled(SAVE_PASSWORD, false);
            dialog.set_button_caption(OK, res.string_line(INMN, STRING_GUEST));
            for id in [METASERVER, LOGIN, PASSWORD, UPDATE_APPEARANCE, SERVER, OK] {
                dialog.set_enabled(id, false);
            }
            METASERVER
        }
        Half::Protocols => {
            let info = protocol_info(
                res.string_line(INMN, STRING_PROTOCOL_INFO),
                TCP_IP_PLAYERS,
                TCP_IP_NAME,
            );
            dialog.set_static_text(INFO, &info);
            dialog.set_button_caption(OK, res.string_line(INMN, STRING_OK));
            dialog.set_enabled(OK, true);
            dialog.set_enabled(SERVER, false);
            OTHER
        }
    };
    for id in METASERVER_ITEMS {
        dialog.set_visible(id, half == Half::Metaserver);
    }
    dialog.set_visible(PROTOCOLS, half == Half::Protocols);
    dialog.set_visible(PROTOCOLS_LABEL, half == Half::Protocols);
    set_radio_family(dialog, chosen);
}

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let (player, half) = match screen {
        NetScreen::Player(p) => (Some(*p), Half::Metaserver),
        NetScreen::Protocol(_) => (None, Half::Protocols),
        _ => return None,
    };
    let def = res.dialog(NESP)?;
    main_menu::push_behind(res, stack);
    let inmg = res.art(INMG);
    let mut dialog = Dialog::new(&def, NESP, FLAGS, res);
    dialog.set_background(inmg.clone(), 0);
    for (id, sequence) in [(METASERVER, 1), (OTHER, 2), (THIRD, 3)] {
        if let Some((item, b)) = dialog.control_mut(id).and_then(|c| c.button_mut()) {
            b.set_shape(item, inmg.clone(), sequence);
            b.set_flags(crate::controls::button::button_flags::STICKS);
            b.set_caption(item, b"");
        }
    }
    if let Some((item, e)) = dialog.control_mut(PASSWORD).and_then(|c| c.edit_mut()) {
        e.set_placeholder(item, GUEST_PLACEHOLDER);
    }
    for (ids, flags) in [
        (&RIGHT_LABELS[..], text_flags::RIGHT | text_flags::SHADOW),
        (&CENTRED_LABELS[..], text_flags::CENTER | text_flags::SHADOW),
    ] {
        for &id in ids {
            if let Some((item, t)) = dialog.control_mut(id).and_then(|c| c.static_text_mut()) {
                t.set_appearance(item, None, STYLE_PLAIN, None, Some(flags));
            }
        }
    }
    dialog.set_default_item(OK);
    dialog.set_cancel_item(CANCEL);
    if let Some(cancel) = def.item(CANCEL as u32) {
        let r = cancel.rect;
        let dx = -r.width() - 12;
        let rect = Rect::new(r.top, r.left + dx, r.bottom, r.right + dx);
        dialog.add_control(FourCC(*b"butt"), SERVER, rect, SERVER_NAME, res);
        dialog.set_enabled(SERVER, false);
    }
    let shown = player.map(Appearance::of);
    if let Some(a) = &shown {
        for (id, text) in [(NAME, &a.name), (TEAM, &a.team)] {
            if let Some((item, e)) = dialog.control_mut(id).and_then(|c| c.edit_mut()) {
                e.set_text(item, text);
            }
        }
    }
    let colors = split_colors(shown.as_ref().map_or([0; 8], |a| a.colors));
    let nepi = res.art_beside(NEPI, INMS);
    if let Some((item, popup)) = dialog.control_mut(ICON).and_then(|c| c.icon_popup_mut()) {
        let icon = shown.as_ref().map_or(0, |a| a.icon as i16);
        popup.set_information(item, nepi, ICON_SEQUENCE, ICON_HIGHLIGHT, icon, 0);
        popup.set_colors(item, colors);
        popup.set_columns(item, ICON_COLUMNS);
        popup.set_rows(item, ICON_ROWS);
    }
    for (id, color) in [(PRIMARY, colors[0]), (SECONDARY, colors[1])] {
        if let Some((item, p)) = dialog.control_mut(id).and_then(|c| c.color_picker_mut()) {
            p.set_color(item, color);
        }
    }
    if let Some(list) = dialog.control_mut(PROTOCOLS).and_then(|c| c.list_mut()) {
        list.add_row(TCP_IP_NAME, 0, true);
        list.select(0);
    }
    dialog.set_visible(THIRD, false);
    show_half(&mut dialog, half, res);
    dialog.set_focus(NAME);
    let original = appearance(&dialog);
    stack.push(dialog);
    Some(Box::new(Open {
        half,
        original: original.clone(),
        sent: original,
        appearance_changed: false,
    }))
}

struct Open {
    half: Half,
    original: Appearance,
    sent: Appearance,
    appearance_changed: bool,
}

impl Open {
    fn changed(&mut self, dialog: &mut Dialog, now: &Appearance) {
        if !self.appearance_changed && *now != self.original {
            dialog.set_checked(UPDATE_APPEARANCE, true);
            self.appearance_changed = true;
        }
        if let Some((item, popup)) = dialog.control_mut(ICON).and_then(|c| c.icon_popup_mut()) {
            popup.set_colors(item, split_colors(now.colors));
        }
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(net) = screen else { return };
        let half = match net {
            NetScreen::Player(_) => Half::Metaserver,
            NetScreen::Protocol(NetProtocol { .. }) => Half::Protocols,
            _ => return,
        };
        let Some(dialog) = frame.stack.find_mut(NESP) else {
            return;
        };
        if half != self.half {
            show_half(dialog, half, frame.res);
            self.half = half;
        }
        if !self
            .sent
            .matches_dialog(dialog, NAME, TEAM, ICON, PRIMARY, SECONDARY)
        {
            let now = appearance(dialog);
            self.changed(dialog, &now);
            frame.out.actions.push(now.action());
            self.sent = now;
        }
        for hit in frame.hits.iter().filter(|h| h.dialog == NESP) {
            match (hit.item, self.half) {
                (OTHER, Half::Metaserver) => frame.out.actions.push(Action::ChooseOther),
                (OK, Half::Protocols) => {
                    let protocol = match net {
                        NetScreen::Protocol(p) => p.selected,
                        _ => Protocol::TcpIp,
                    };
                    tracing::debug!(?protocol, "nesp: OK");
                    frame.out.actions.push(Action::Ok);
                }
                (CANCEL, _) => frame.out.actions.push(Action::Cancel),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_protocol_line_takes_the_players_and_the_name() {
        assert_eq!(
            protocol_info(
                b"Supports up to %d players and requires %s.",
                16,
                TCP_IP_NAME
            ),
            b"Supports up to 16 players and requires TCP/IP (Internet)."
        );
    }
}
