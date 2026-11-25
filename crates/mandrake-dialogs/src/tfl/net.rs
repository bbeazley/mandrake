use mandrake_common::FourCC;
use mandrake_session::net::{HostOrJoin, JoinFailure, NetPlayer, NetScreen};
use mandrake_session::{Action, Screen, Text};
use mandrake_tag::font::{STYLE_PLAIN, text_flags};

use crate::core::{Dialog, DialogStack, dialog_flags, format_one};
use crate::resources::Resources;
use crate::screens::join_failed::{self, Notice};
use crate::screens::net_join::LanList;
use crate::screens::net_player::{Appearance, protocol_info, split_colors};
use crate::screens::{OpenScreen, ScreenFrame};

pub const NESP: FourCC = FourCC(*b"nesp");
pub const INET: FourCC = FourCC(*b"Inet");
const INMG: FourCC = FourCC(*b"inmg");
const NEPI: FourCC = FourCC(*b"nepi");
const INMN: FourCC = FourCC(*b"inmn");
const INNS: FourCC = FourCC(*b"inns");
const FLAGS: u16 = dialog_flags::MODAL;

pub const NESP_OK: i16 = 1;
pub const NESP_CANCEL: i16 = 2;
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
const INFO: i16 = 15;
const METASERVER_ITEMS: [i16; 6] = [0x12, 0xd, 0x13, 0xe, 0x10, 0xb];
const PROTOCOL_ITEMS: [i16; 2] = [0xc, 0x11];
const RIGHT_LABELS: [i16; 5] = [0x19, 0x18, 0x17, 0x12, 0x13];
const CENTRED_LABELS: [i16; 2] = [0x15, 0x14];

const STRING_NO_PROTOCOLS: usize = 9;
const STRING_PROTOCOL_INFO: usize = 10;
const STRING_LOGIN: usize = 13;
const STRING_OK: usize = 14;
const STRING_VISIT: usize = 17;
const STRING_GAME_NAME: usize = 24;
const STRING_CREATE_FAILED: usize = 26;
const STRING_CANCELLED: usize = 1;
const STRING_TIMED_OUT: usize = 2;
const STRING_UNABLE: usize = 3;

const TCP_IP_NAME: &[u8] = b"TCP/IP (Internet)";
const TCP_IP_PLAYERS: i32 = 16;
const ICON_SEQUENCE: i32 = 0;
const ICON_HIGHLIGHT: i32 = 3;
const ICON_COLUMNS: i32 = 10;
const TEXT_LENGTH: usize = 0x1f;

pub const CREATE: i16 = 1;
pub const INET_CANCEL: i16 = 2;
pub const GAME_NAME: i16 = 3;
pub const JOIN: i16 = 4;
pub const LAN_LIST: i16 = 5;
pub const PASSWORD: i16 = 8;
pub const ADDRESS: i16 = 9;
pub const PORT: i16 = 12;
pub const JOIN_BY_ADDRESS: i16 = 17;
const ADDRESS_ITEMS: [i16; 3] = [9, 10, 11];
const PORT_ITEMS: [i16; 3] = [12, 13, 14];
const PORT_FLAG: u32 = 0x40;

const JOINING_BOX_MS: u64 = 1000;
pub const JOIN_WAIT_MS: u64 = 15_000;

pub fn is_tfl_screen(screen: &NetScreen<'_>) -> bool {
    matches!(
        screen,
        NetScreen::Player(_)
            | NetScreen::Protocol(_)
            | NetScreen::HostOrJoin(_)
            | NetScreen::Joining(_)
            | NetScreen::JoinFailed(_)
            | NetScreen::Lobby(_)
            | NetScreen::GameOptions(_)
            | NetScreen::EndBarrier { .. }
            | NetScreen::Postgame(..)
    )
}

fn edit_text(dialog: &Dialog, id: i16) -> Vec<u8> {
    let text = dialog.edit_text_slice(id);
    text[..text.len().min(TEXT_LENGTH)].to_vec()
}

fn appearance(dialog: &Dialog) -> Appearance {
    Appearance::read_dialog(dialog, NAME, TEAM, ICON, PRIMARY, SECONDARY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Half {
    Metaserver,
    Protocols,
}

fn show_half(dialog: &mut Dialog, half: Half, res: &mut Resources) {
    let chosen = match half {
        Half::Metaserver => {
            dialog.set_static_text(INFO, res.string_line(INMN, STRING_VISIT));
            dialog.set_button_caption(NESP_OK, res.string_line(INMN, STRING_LOGIN));
            dialog.set_enabled(NESP_OK, false);
            dialog.set_enabled(SAVE_PASSWORD, false);
            METASERVER
        }
        Half::Protocols => {
            let info = protocol_info(
                res.string_line(INMN, STRING_PROTOCOL_INFO),
                TCP_IP_PLAYERS,
                TCP_IP_NAME,
            );
            dialog.set_static_text(INFO, &info);
            dialog.set_button_caption(NESP_OK, res.string_line(INMN, STRING_OK));
            dialog.set_enabled(NESP_OK, true);
            OTHER
        }
    };
    for id in METASERVER_ITEMS {
        dialog.set_visible(id, half == Half::Metaserver);
    }
    for id in PROTOCOL_ITEMS {
        dialog.set_visible(id, half == Half::Protocols);
    }
    for id in METASERVER..=THIRD {
        dialog.set_checked(id, id == chosen);
    }
}

fn nesp(player: &NetPlayer, res: &mut Resources) -> Option<Dialog> {
    let def = res.dialog(NESP)?;
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
    dialog.set_default_item(NESP_OK);
    dialog.set_cancel_item(NESP_CANCEL);
    let shown = Appearance::of(player);
    dialog.set_edit_text(NAME, &shown.name);
    dialog.set_edit_text(TEAM, &shown.team);
    let colors = split_colors(shown.colors);
    let nepi = res.art(NEPI);
    if let Some((item, popup)) = dialog.control_mut(ICON).and_then(|c| c.icon_popup_mut()) {
        popup.set_information(
            item,
            nepi,
            ICON_SEQUENCE,
            ICON_HIGHLIGHT,
            shown.icon as i16,
            0,
        );
        popup.set_colors(item, colors);
        popup.set_columns(item, ICON_COLUMNS);
    }
    for (id, color) in [(PRIMARY, colors[0]), (SECONDARY, colors[1])] {
        if let Some((item, p)) = dialog.control_mut(id).and_then(|c| c.color_picker_mut()) {
            p.set_color(item, color);
        }
    }
    let mut protocols = 0;
    if let Some(list) = dialog.list_mut(PROTOCOLS) {
        list.add_row(TCP_IP_NAME, 0, true);
        list.select(0);
        protocols = list.row_count();
    }
    dialog.set_visible(THIRD, false);
    show_half(&mut dialog, Half::Protocols, res);
    if protocols == 0 {
        dialog.set_static_text(INFO, res.string_line(INMN, STRING_NO_PROTOCOLS));
        dialog.set_enabled(NESP_OK, false);
    }
    dialog.set_focus(NAME);
    Some(dialog)
}

fn create_enabled(dialog: &Dialog) -> bool {
    let text = dialog.edit_text_slice(GAME_NAME);
    crate::screens::net_join::valid_game_name(&text[..text.len().min(TEXT_LENGTH)])
}

fn join_enabled(dialog: &Dialog) -> bool {
    dialog.is_checked(JOIN_BY_ADDRESS)
        || dialog
            .control(LAN_LIST)
            .and_then(|c| c.list())
            .is_some_and(|l| l.selected().is_some())
}

fn show_join_by_address(dialog: &mut Dialog, checked: bool) {
    for id in ADDRESS_ITEMS {
        dialog.set_visible(id, checked);
    }
    for id in PORT_ITEMS {
        dialog.set_visible(id, !checked);
    }
}

fn update_buttons(dialog: &mut Dialog) {
    let (join, create) = (join_enabled(dialog), create_enabled(dialog));
    dialog.set_enabled_if(JOIN, join);
    dialog.set_enabled_if(CREATE, create);
    if join {
        dialog.set_default_item(JOIN);
    } else if create {
        dialog.set_default_item(CREATE);
    }
}

fn inet(data: &HostOrJoin, res: &mut Resources) -> Option<Dialog> {
    let def = res.dialog(INET)?;
    let mut dialog = Dialog::new(&def, INET, FLAGS, res);
    dialog.set_edit_text(GAME_NAME, res.string_line(INMN, STRING_GAME_NAME));
    dialog.set_edit_text(PASSWORD, b"");
    dialog.set_focus(GAME_NAME);
    dialog.set_cancel_item(INET_CANCEL);
    dialog.set_edit_text(ADDRESS, &data.address.0);
    dialog.set_edit_text(PORT, data.port.to_string().as_bytes());
    if let Some((item, e)) = dialog.control_mut(PORT).and_then(|c| c.edit_mut()) {
        e.set_flag(item, PORT_FLAG, true);
    }
    if let Some(list) = dialog.list_mut(LAN_LIST) {
        list.set_double_click_item(JOIN);
    }
    dialog.set_checked(JOIN_BY_ADDRESS, false);
    show_join_by_address(&mut dialog, false);
    update_buttons(&mut dialog);
    Some(dialog)
}

fn notice_for(screen: &NetScreen<'_>, joining_ms: u64, res: &mut Resources) -> Option<Notice> {
    match screen {
        NetScreen::Joining(_) => (joining_ms > JOINING_BOX_MS).then_some(Notice::Joining),
        NetScreen::HostOrJoin(h) => h.host_failed.as_ref().map(|reason| {
            Notice::Error(format_one(
                res.string_line(INMN, STRING_CREATE_FAILED),
                &reason.0,
            ))
        }),
        NetScreen::JoinFailed(f) => Some(Notice::Error(match &f.reason {
            JoinFailure::Denied(reason) => reason.0.clone(),
            JoinFailure::TimedOut | JoinFailure::HostLost => {
                res.string_line(INNS, STRING_TIMED_OUT).to_vec()
            }
            JoinFailure::Cancelled => res.string_line(INNS, STRING_CANCELLED).to_vec(),
            JoinFailure::Unreachable(error) => format_one(
                res.string_line(INNS, STRING_UNABLE),
                error.to_string().as_bytes(),
            ),
            JoinFailure::InProgress => format_one(res.string_line(INNS, STRING_UNABLE), b"0"),
            JoinFailure::InvalidGame => f.text.0.clone(),
        })),
        _ => None,
    }
}

pub(super) fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let player = match screen {
        NetScreen::Player(p) => (*p).clone(),
        _ => NetPlayer::default(),
    };
    let dialog = nesp(&player, res)?;
    let original = appearance(&dialog);
    stack.push(dialog);
    Some(Box::new(Open {
        half: Half::Protocols,
        sent: original,
        inet: None,
    }))
}

struct Open {
    half: Half,
    sent: Appearance,
    inet: Option<InetOpen>,
}

#[derive(Default)]
struct InetOpen {
    address: Vec<u8>,
    notice: Option<Notice>,
    joining_since: Option<u64>,
    lan: LanList,
}

impl Open {
    fn nesp_frame(&mut self, frame: &mut ScreenFrame<'_>) {
        let Some(dialog) = frame.stack.find_mut(NESP) else {
            return;
        };
        if !self
            .sent
            .matches_dialog(dialog, NAME, TEAM, ICON, PRIMARY, SECONDARY)
        {
            let now = appearance(dialog);
            if let Some((item, popup)) = dialog.control_mut(ICON).and_then(|c| c.icon_popup_mut()) {
                popup.set_colors(item, split_colors(now.colors));
            }
            frame.out.actions.push(now.action());
            self.sent = now;
        }
        for hit in frame.hits.iter().filter(|h| h.dialog == NESP) {
            match hit.item {
                METASERVER if self.half != Half::Metaserver => {
                    self.half = Half::Metaserver;
                    show_half(dialog, Half::Metaserver, frame.res);
                }
                OTHER if self.half != Half::Protocols => {
                    self.half = Half::Protocols;
                    show_half(dialog, Half::Protocols, frame.res);
                }
                NESP_OK if self.half == Half::Protocols && dialog.is_enabled(NESP_OK) => {
                    frame.out.actions.push(Action::Ok);
                }
                NESP_CANCEL => frame.out.actions.push(Action::Cancel),
                _ => {}
            }
        }
    }
}

impl InetOpen {
    fn browse(&mut self, data: Option<&HostOrJoin>, frame: &mut ScreenFrame<'_>) {
        let target = data.filter(|h| !h.hosting).and_then(|h| h.lan_broadcast);
        let Some(dialog) = frame.stack.find_mut(INET) else {
            if target.is_none() {
                self.lan.close();
            }
            return;
        };
        let disabled = dialog.is_checked(JOIN_BY_ADDRESS);
        self.lan.poll(
            target,
            frame.input.now_ms,
            dialog,
            LAN_LIST,
            disabled,
            "TFL Inet",
        );
    }

    fn hits(&mut self, data: Option<&HostOrJoin>, frame: &mut ScreenFrame<'_>) {
        let Some(dialog) = frame.stack.find_mut(INET) else {
            return;
        };
        if let Some(h) = data {
            self.address = h.address.0.clone();
        }
        let hits: Vec<i16> = frame
            .hits
            .iter()
            .filter(|h| h.dialog == INET)
            .map(|h| h.item)
            .collect();
        let idle = data.is_some_and(|h| !h.hosting);
        for item in hits {
            match item {
                JOIN_BY_ADDRESS => {
                    let checked = dialog.is_checked(JOIN_BY_ADDRESS);
                    if let Some(c) = dialog.control_mut(LAN_LIST) {
                        c.item.dirty = true;
                    }
                    if let Some(list) = dialog.list_mut(LAN_LIST) {
                        for row in 0..list.row_count() {
                            list.set_row_enabled(row, !checked);
                        }
                        if checked {
                            list.clear_selection();
                        } else {
                            list.select_first_enabled();
                        }
                    }
                    show_join_by_address(dialog, checked);
                    if checked {
                        dialog.set_focus(ADDRESS);
                    }
                }
                JOIN if idle && dialog.is_enabled(JOIN) => {
                    let lan = dialog
                        .selected_list_value(LAN_LIST)
                        .and_then(|v| self.lan.address(v));
                    let password = edit_text(dialog, PASSWORD);
                    frame.out.actions.push(
                        match lan.filter(|_| !dialog.is_checked(JOIN_BY_ADDRESS)) {
                            Some(a) if password.is_empty() => Action::JoinLanGame {
                                address: Text(a.to_string().into_bytes()),
                            },
                            Some(a) => Action::JoinByAddress {
                                address: Text(a.to_string().into_bytes()),
                                password: Text(password),
                            },
                            None => Action::JoinByAddress {
                                address: Text(dialog.edit_text_slice(ADDRESS).to_vec()),
                                password: Text(password),
                            },
                        },
                    );
                }
                CREATE if idle && dialog.is_enabled(CREATE) => {
                    frame.out.actions.push(Action::HostGame {
                        name: Text(edit_text(dialog, GAME_NAME)),
                        password: Text(edit_text(dialog, PASSWORD)),
                        port: crate::screens::net_join::port_of(dialog.edit_text_slice(PORT)),
                    });
                }
                INET_CANCEL => frame.out.actions.push(Action::Cancel),
                _ => {}
            }
        }
        update_buttons(dialog);
    }

    fn frame(&mut self, net: &NetScreen<'_>, frame: &mut ScreenFrame<'_>) {
        if !matches!(net, NetScreen::HostOrJoin(_)) {
            self.lan.close();
        }
        let now = frame.input.now_ms;
        let joining_ms = match net {
            NetScreen::Joining(_) => now.saturating_sub(*self.joining_since.get_or_insert(now)),
            _ => {
                self.joining_since = None;
                0
            }
        };
        let wanted = notice_for(net, joining_ms, frame.res);
        if wanted != self.notice {
            if self.notice.is_some() {
                frame.stack.pop();
            }
            if let Some(n) = &wanted {
                join_failed::push(n, frame.res, frame.stack);
            }
            self.notice = wanted;
        }
        if let Some(notice) = &self.notice {
            join_failed::frame(notice, &self.address, joining_ms, frame);
            if *notice == Notice::Joining
                && let Some((item, bar)) = frame
                    .stack
                    .find_mut(join_failed::PRDB)
                    .and_then(|d| d.control_mut(3))
                    .and_then(|c| c.progress_mut())
            {
                let fraction = i16::try_from(joining_ms * 0x100 / JOIN_WAIT_MS).unwrap_or(0x100);
                bar.set_fraction(item, fraction.min(0x100));
                bar.set_color(item, crate::controls::progress::TFL_COLOR);
            }
            return;
        }
        let data = match net {
            NetScreen::HostOrJoin(h) => Some(*h),
            _ => None,
        };
        self.browse(data, frame);
        self.hits(data, frame);
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(net) = screen else { return };
        let over_nesp = matches!(net, NetScreen::Player(_) | NetScreen::Protocol(_));
        if over_nesp {
            if let Some(inet) = self.inet.take() {
                if inet.notice.is_some() {
                    frame.stack.pop();
                }
                frame.stack.pop();
            }
            self.nesp_frame(frame);
            return;
        }
        if self.inet.is_none() {
            let data = match net {
                NetScreen::HostOrJoin(h) => (*h).clone(),
                _ => HostOrJoin {
                    port: mandrake_session::net::DEFAULT_PORT,
                    ..HostOrJoin::default()
                },
            };
            let Some(dialog) = inet(&data, frame.res) else {
                return;
            };
            frame.stack.push(dialog);
            self.inet = Some(InetOpen {
                address: data.address.0,
                ..InetOpen::default()
            });
        }
        if let Some(inet) = self.inet.as_mut() {
            inet.frame(net, frame);
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
                b"This protocol supports up to %d players and requires %s.",
                16,
                TCP_IP_NAME
            ),
            b"This protocol supports up to 16 players and requires TCP/IP (Internet)."
        );
    }

    #[test]
    fn tfl_lan_join_carries_password_and_idle_inet_stays_clean() {
        use mandrake_net::lan::{LanEvent, LanGame};
        use mandrake_session::{FrameInput, FrameOutput};
        use mandrake_tag::{DialogDefinition, DialogItem, Rect};
        use std::net::SocketAddrV4;

        let make_item = |kind: &[u8; 4], id: u32| DialogItem {
            kind: FourCC(*kind),
            field_04: 0,
            id,
            fields_08: [0; 4],
            rect: Rect::new(0, 0, 100, 100),
            name: Vec::new(),
        };
        let inet_def = DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 400, 400),
            items: vec![
                make_item(b"butt", CREATE as u32),
                make_item(b"butt", INET_CANCEL as u32),
                make_item(b"edit", GAME_NAME as u32),
                make_item(b"butt", JOIN as u32),
                make_item(b"list", LAN_LIST as u32),
                make_item(b"edit", PASSWORD as u32),
                make_item(b"edit", ADDRESS as u32),
                make_item(b"edit", PORT as u32),
                make_item(b"chek", JOIN_BY_ADDRESS as u32),
            ],
        };
        let mut res = Resources::from_source(|_, _| None);
        let mut dialog = Dialog::new(&inet_def, INET, FLAGS, &mut res);
        dialog.set_edit_text(GAME_NAME, b"My Game");
        dialog.set_checked(JOIN_BY_ADDRESS, false);
        let addr: SocketAddrV4 = "10.0.0.5:3453".parse().unwrap();
        let mut lan = LanList::default();
        if let Some(list) = dialog.list_mut(LAN_LIST) {
            lan.apply_events(
                &[LanEvent::Added(LanGame {
                    name: b"TFL Game".to_vec(),
                    address: addr,
                })],
                list,
                false,
                "test",
            );
            list.select(0);
        }
        update_buttons(&mut dialog);
        dialog.redraw(&res, 0);
        assert!(!dialog.is_dirty());
        let mut stack = DialogStack::tfl();
        stack.push(dialog);
        let mut inet = InetOpen {
            address: Vec::new(),
            notice: None,
            joining_since: None,
            lan,
        };
        let hoj = HostOrJoin::default();
        let input = FrameInput {
            now_ms: 0,
            size: (640, 480),
            events: &[],
            sound_busy: false,
            shift_held: false,
        };
        let mut out = FrameOutput::default();
        inet.frame(
            &NetScreen::HostOrJoin(&hoj),
            &mut ScreenFrame {
                stack: &mut stack,
                res: &mut res,
                input: &input,
                now_tick: 0,
                ticks: 0,
                hits: &[],
                out: &mut out,
            },
        );
        assert!(!stack.find_mut(INET).unwrap().is_dirty());

        let hits = [crate::core::Hit {
            dialog: INET,
            item: JOIN,
        }];
        inet.frame(
            &NetScreen::HostOrJoin(&hoj),
            &mut ScreenFrame {
                stack: &mut stack,
                res: &mut res,
                input: &input,
                now_tick: 1,
                ticks: 1,
                hits: &hits,
                out: &mut out,
            },
        );
        assert_eq!(
            out.actions,
            [Action::JoinLanGame {
                address: Text(b"10.0.0.5:3453".to_vec()),
            }]
        );

        stack
            .find_mut(INET)
            .unwrap()
            .set_edit_text(PASSWORD, b"secret");
        out.actions.clear();
        inet.frame(
            &NetScreen::HostOrJoin(&hoj),
            &mut ScreenFrame {
                stack: &mut stack,
                res: &mut res,
                input: &input,
                now_tick: 2,
                ticks: 1,
                hits: &hits,
                out: &mut out,
            },
        );
        assert_eq!(
            out.actions,
            [Action::JoinByAddress {
                address: Text(b"10.0.0.5:3453".to_vec()),
                password: Text(b"secret".to_vec()),
            }]
        );
    }
}
