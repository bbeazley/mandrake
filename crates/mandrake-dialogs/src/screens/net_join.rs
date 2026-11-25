use std::net::SocketAddrV4;

use mandrake_common::FourCC;
use mandrake_net::lan::{Browser, LanEvent};
use mandrake_session::net::{HostOrJoin, NetScreen};
use mandrake_session::{Action, Screen, Text};

use super::join_failed::{self, Notice};
use super::{OpenScreen, ScreenFrame, main_menu};
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const INET: FourCC = FourCC(*b"Inet");
const INTC: FourCC = FourCC(*b"intc");
const FLAGS: u16 = dialog_flags::MODAL;

pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const NAME: i16 = 3;
pub const LAN_LIST: i16 = 4;
const NAME_LABEL: i16 = 5;
const PASSWORD_LABEL: i16 = 6;
pub const PASSWORD: i16 = 7;
pub const ADDRESS: i16 = 8;
const JOIN_TEXT: i16 = 9;
pub const PORT: i16 = 11;
const HOST_TEXT: i16 = 12;
const PORT_LABEL: i16 = 13;
pub const JOIN_BY_ADDRESS: i16 = 16;
pub const JOIN_RADIO: i16 = 17;
pub const HOST_RADIO: i16 = 18;

const PORT_FLAG: u32 = 0x40;
const INNI: FourCC = FourCC(*b"inni");
const STRING_CREATE: usize = 0;
const STRING_JOIN: usize = 1;
const STRING_GAME_NAME: usize = 24;
const TEXT_LENGTH: usize = 0x1f;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Half {
    Join,
    Host,
}

#[derive(Default)]
pub(crate) struct LanList {
    browser: Option<Browser>,
    failed: bool,
    games: Vec<Option<SocketAddrV4>>,
}

impl LanList {
    pub(crate) fn close(&mut self) {
        self.browser = None;
        self.failed = false;
    }

    pub(crate) fn address(&self, value: i32) -> Option<SocketAddrV4> {
        usize::try_from(value)
            .ok()
            .and_then(|i| self.games.get(i).copied().flatten())
    }

    pub(crate) fn apply_events(
        &mut self,
        events: &[LanEvent],
        list: &mut crate::controls::list::List,
        disabled: bool,
        tag: &str,
    ) {
        let selected = list.selected_value();
        for e in events {
            match e {
                LanEvent::Added(game) => {
                    tracing::info!(
                        "{tag}: LAN game {} at {}",
                        String::from_utf8_lossy(&game.name),
                        game.address
                    );
                    let value = self.games.len() as i32;
                    self.games.push(Some(game.address));
                    let row = list.add_row(&game.name, value, true);
                    list.set_row_enabled(row, !disabled);
                }
                LanEvent::Removed(address) => {
                    tracing::info!("{tag}: LAN game at {address} gone");
                    if let Some(idx) = self.games.iter().position(|a| *a == Some(*address)) {
                        self.games[idx] = None;
                        if let Ok(v) = i32::try_from(idx) {
                            list.remove_value(v);
                        }
                    }
                }
            }
        }
        if let Some(v) = selected {
            list.select_value(v);
        }
    }

    pub(crate) fn poll(
        &mut self,
        target: Option<SocketAddrV4>,
        now_ms: u64,
        dialog: &mut Dialog,
        list_id: i16,
        disabled: bool,
        tag: &str,
    ) {
        let Some(target) = target else {
            self.close();
            return;
        };
        if self.browser.is_none() {
            if self.failed {
                return;
            }
            match Browser::open(target) {
                Ok(b) => self.browser = Some(b),
                Err(e) => {
                    self.failed = true;
                    tracing::warn!("{tag}: no LAN list: {e}");
                    return;
                }
            }
            self.games.clear();
            if let Some(c) = dialog.control_mut(list_id)
                && let Some(list) = c.list_mut()
            {
                list.clear();
                c.item.dirty = true;
            }
        }
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        let events = browser.poll(now_ms);
        if events.is_empty() {
            return;
        }
        let Some(c) = dialog.control_mut(list_id) else {
            return;
        };
        c.item.dirty = true;
        let Some(list) = c.list_mut() else {
            return;
        };
        self.apply_events(&events, list, disabled, tag);
    }
}

pub(super) fn edit_text(dialog: &Dialog, id: i16) -> Vec<u8> {
    dialog.edit_text_slice(id).to_vec()
}

pub fn valid_game_name(name: &[u8]) -> bool {
    name.iter().any(|&c| c.is_ascii_graphic())
}

pub fn port_of(text: &[u8]) -> u16 {
    let digits: Vec<u8> = text
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits)
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .map_or(mandrake_session::net::DEFAULT_PORT, |p| p as u16)
}

fn half(dialog: &Dialog) -> Half {
    if dialog.is_checked(HOST_RADIO) {
        Half::Host
    } else {
        Half::Join
    }
}

fn ok_enabled(dialog: &Dialog) -> bool {
    match half(dialog) {
        Half::Join => {
            dialog.is_checked(JOIN_BY_ADDRESS)
                || dialog
                    .control(LAN_LIST)
                    .and_then(|c| c.list())
                    .is_some_and(|l| l.selected().is_some())
        }
        Half::Host => valid_game_name(dialog.edit_text_slice(NAME)),
    }
}

fn show_half(dialog: &mut Dialog, chosen: Half, res: &mut Resources) {
    dialog.set_checked(JOIN_RADIO, chosen == Half::Join);
    dialog.set_checked(HOST_RADIO, chosen == Half::Host);
    let host = chosen == Half::Host;
    for id in [
        NAME_LABEL,
        NAME,
        PASSWORD_LABEL,
        PASSWORD,
        HOST_TEXT,
        PORT_LABEL,
        PORT,
    ] {
        dialog.set_visible(id, host);
    }
    dialog.set_visible(JOIN_BY_ADDRESS, !host);
    dialog.set_enabled(LAN_LIST, !host);
    dialog.set_visible(JOIN_TEXT, !host);
    if host {
        dialog.set_visible(ADDRESS, false);
        dialog.set_checked(JOIN_BY_ADDRESS, false);
        if let Some(c) = dialog.control_mut(LAN_LIST) {
            if let Some(list) = c.list_mut() {
                for row in 0..list.row_count() {
                    list.set_row_enabled(row, true);
                }
            }
            c.item.dirty = true;
        }
        dialog.set_button_caption(OK, res.string_line(INNI, STRING_CREATE));
    } else {
        dialog.set_button_caption(OK, res.string_line(INNI, STRING_JOIN));
    }
}

fn join_by_address_only(dialog: &mut Dialog, address: &[u8]) {
    dialog.set_enabled(HOST_RADIO, false);
    dialog.set_checked(JOIN_BY_ADDRESS, true);
    dialog.set_edit_text(ADDRESS, address);
    dialog.set_visible(ADDRESS, true);
    dialog.set_focus(ADDRESS);
}

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let def = res.dialog(INET)?;
    main_menu::push_behind(res, stack);
    if let NetScreen::Joining(j) = screen
        && j.direct
    {
        return Some(Box::new(Open {
            address: j.address.0.clone(),
            joining_address: j.address.0.clone(),
            checked: true,
            notice: None,
            joining_since: None,
            lan: LanList::default(),
            direct: true,
        }));
    }
    let data = match screen {
        NetScreen::HostOrJoin(h) => (*h).clone(),
        _ => HostOrJoin {
            port: mandrake_session::net::DEFAULT_PORT,
            ..HostOrJoin::default()
        },
    };
    let mut dialog = Dialog::new(&def, INET, FLAGS, res);
    show_half(&mut dialog, Half::Join, res);
    dialog.set_edit_text(NAME, res.string_line(join_failed::INMN, STRING_GAME_NAME));
    dialog.set_edit_text(PASSWORD, b"");
    dialog.set_focus(NAME);
    dialog.set_default_item(OK);
    dialog.set_cancel_item(CANCEL);
    dialog.set_edit_text(ADDRESS, b"");
    dialog.set_background(res.art(INTC), 0);
    dialog.set_edit_text(PORT, data.port.to_string().as_bytes());
    if let Some((item, e)) = dialog.control_mut(PORT).and_then(|c| c.edit_mut()) {
        e.set_flag(item, PORT_FLAG, true);
    }
    dialog.set_checked(JOIN_BY_ADDRESS, false);
    dialog.set_visible(ADDRESS, false);
    if let Some(list) = dialog.list_mut(LAN_LIST) {
        list.set_double_click_item(OK);
    }
    let web = cfg!(target_arch = "wasm32");
    if web {
        join_by_address_only(&mut dialog, &data.address.0);
    }
    dialog.set_enabled(OK, ok_enabled(&dialog));
    stack.push(dialog);
    Some(Box::new(Open {
        address: data.address.0,
        joining_address: Vec::new(),
        checked: web,
        notice: None,
        joining_since: None,
        lan: LanList::default(),
        direct: false,
    }))
}

struct Open {
    address: Vec<u8>,
    joining_address: Vec<u8>,
    checked: bool,
    notice: Option<Notice>,
    joining_since: Option<u64>,
    lan: LanList,
    direct: bool,
}

impl Open {
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
            "Inet",
        );
    }

    fn inet_hits(&mut self, data: Option<&HostOrJoin>, frame: &mut ScreenFrame<'_>) {
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
        for item in hits {
            match item {
                JOIN_RADIO => show_half(dialog, Half::Join, frame.res),
                HOST_RADIO => show_half(dialog, Half::Host, frame.res),
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
                    dialog.set_edit_text(ADDRESS, &self.address);
                    dialog.set_visible(ADDRESS, checked);
                    if checked {
                        dialog.set_focus(ADDRESS);
                    }
                }
                OK if dialog.is_enabled(OK) && data.is_some_and(|h| !h.hosting) => {
                    let lan = dialog
                        .selected_list_value(LAN_LIST)
                        .and_then(|v| self.lan.address(v));
                    frame.out.actions.push(match half(dialog) {
                        Half::Join if !dialog.is_checked(JOIN_BY_ADDRESS) && lan.is_some() => {
                            let addr = lan.map(|a| a.to_string()).unwrap_or_default().into_bytes();
                            self.joining_address = addr.clone();
                            Action::JoinLanGame {
                                address: Text(addr),
                            }
                        }
                        Half::Join => {
                            let addr = edit_text(dialog, ADDRESS);
                            self.joining_address = addr.clone();
                            Action::JoinByAddress {
                                address: Text(addr),
                                password: Text::default(),
                            }
                        }
                        Half::Host => {
                            let mut name = edit_text(dialog, NAME);
                            name.truncate(TEXT_LENGTH);
                            let mut password = edit_text(dialog, PASSWORD);
                            password.truncate(TEXT_LENGTH);
                            Action::HostGame {
                                name: Text(name),
                                password: Text(password),
                                port: port_of(dialog.edit_text_slice(PORT)),
                            }
                        }
                    });
                }
                CANCEL => frame.out.actions.push(Action::Cancel),
                _ => {}
            }
        }
        self.checked = dialog.is_checked(JOIN_BY_ADDRESS);
        dialog.set_enabled_if(OK, ok_enabled(dialog));
    }
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(net) = screen else { return };
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
        let wanted = if self.direct {
            join_failed::direct_notice_for(net, &self.address, joining_ms, frame.res)
        } else {
            join_failed::notice_for(net, joining_ms, frame.res)
        };
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
            let address = if self.direct {
                self.address.clone()
            } else if !self.joining_address.is_empty() {
                self.joining_address.clone()
            } else {
                frame
                    .stack
                    .find_mut(INET)
                    .map(|d| edit_text(d, ADDRESS))
                    .unwrap_or_default()
            };
            join_failed::frame(notice, &address, joining_ms, frame);
            return;
        }
        if self.direct {
            return;
        }
        let data = match net {
            NetScreen::HostOrJoin(h) => Some(*h),
            _ => None,
        };
        self.browse(data, frame);
        self.inet_hits(data, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_needs_a_visible_character() {
        assert!(!valid_game_name(b""));
        assert!(!valid_game_name(b"   "));
        assert!(valid_game_name(b" a "));
    }

    #[test]
    fn the_port_box_reads_its_digits() {
        assert_eq!(port_of(b"3453"), 3453);
        assert_eq!(port_of(b"2000x"), 2000);
        assert_eq!(port_of(b""), 3453);
    }

    #[test]
    fn myth2_lan_password_retry_uses_lan_address_and_host_switch_reenables_rows() {
        use mandrake_net::lan::LanGame;
        use mandrake_session::net::{JoinFailed, JoinFailure};
        use mandrake_session::{FrameInput, FrameOutput};
        use mandrake_tag::{DialogDefinition, DialogItem, Rect};

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
                make_item(b"butt", OK as u32),
                make_item(b"butt", CANCEL as u32),
                make_item(b"edit", NAME as u32),
                make_item(b"list", LAN_LIST as u32),
                make_item(b"edit", PASSWORD as u32),
                make_item(b"edit", ADDRESS as u32),
                make_item(b"edit", PORT as u32),
                make_item(b"chek", JOIN_BY_ADDRESS as u32),
                make_item(b"radi", JOIN_RADIO as u32),
                make_item(b"radi", HOST_RADIO as u32),
            ],
        };
        let gapd_def = DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 200, 200),
            items: vec![
                make_item(b"butt", join_failed::GAPD_OK as u32),
                make_item(b"butt", join_failed::GAPD_CANCEL as u32),
                make_item(b"pass", join_failed::GAPD_PASSWORD as u32),
            ],
        };
        let mut res = Resources::from_source(|group, id| {
            if group == FourCC(*b"stli") && id == join_failed::INNS {
                let mut lines = [b"x".as_slice(); 10];
                lines[join_failed::STRING_PASSWORD_NEEDED] = b"needs password";
                let mut out = lines.join(&b'\r');
                out.push(0);
                return Some(out);
            }
            None
        });
        let mut stack = DialogStack::new();
        let mut dialog = Dialog::new(&inet_def, INET, FLAGS, &mut res);
        dialog.set_checked(JOIN_RADIO, true);
        dialog.set_checked(JOIN_BY_ADDRESS, false);
        dialog.set_edit_text(ADDRESS, b"");
        let addr: SocketAddrV4 = "192.168.1.50:3453".parse().unwrap();
        let mut lan = LanList::default();
        if let Some(list) = dialog.list_mut(LAN_LIST) {
            lan.apply_events(
                &[LanEvent::Added(LanGame {
                    name: b"Secret Game".to_vec(),
                    address: addr,
                })],
                list,
                false,
                "test",
            );
            list.select(0);
        }
        dialog.set_enabled(OK, true);
        stack.push(dialog);

        let mut open = Open {
            address: Vec::new(),
            joining_address: Vec::new(),
            checked: false,
            notice: None,
            joining_since: None,
            lan,
            direct: false,
        };
        {
            let d = stack.find_mut(INET).unwrap();
            d.set_checked(JOIN_BY_ADDRESS, true);
            if let Some(list) = d.list_mut(LAN_LIST) {
                list.set_row_enabled(0, false);
            }
            show_half(d, Half::Host, &mut res);
            show_half(d, Half::Join, &mut res);
            assert!(d.control(LAN_LIST).unwrap().list().unwrap().row_enabled(0));
            d.list_mut(LAN_LIST).unwrap().select(0);
        }
        let hoj = HostOrJoin::default();
        let input = FrameInput {
            now_ms: 0,
            size: (800, 600),
            events: &[],
            sound_busy: false,
            shift_held: false,
        };
        let mut out = FrameOutput::default();
        let hits = [crate::core::Hit {
            dialog: INET,
            item: OK,
        }];
        open.frame(
            &Screen::Net(NetScreen::HostOrJoin(&hoj)),
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
        assert_eq!(
            out.actions,
            [Action::JoinLanGame {
                address: Text(b"192.168.1.50:3453".to_vec()),
            }]
        );
        let failed = JoinFailed {
            reason: JoinFailure::Denied(Text(b"needs password".to_vec())),
            text: Text::default(),
        };
        let mut gapd = Dialog::new(&gapd_def, join_failed::GAPD, FLAGS, &mut res);
        gapd.set_edit_text(join_failed::GAPD_PASSWORD, b"hunter2");
        stack.push(gapd);
        open.notice = Some(Notice::Password);
        out.actions.clear();
        let gapd_hits = [crate::core::Hit {
            dialog: join_failed::GAPD,
            item: join_failed::GAPD_OK,
        }];
        open.frame(
            &Screen::Net(NetScreen::JoinFailed(&failed)),
            &mut ScreenFrame {
                stack: &mut stack,
                res: &mut res,
                input: &input,
                now_tick: 1,
                ticks: 1,
                hits: &gapd_hits,
                out: &mut out,
            },
        );
        assert_eq!(
            out.actions,
            [
                Action::Ok,
                Action::JoinByAddress {
                    address: Text(b"192.168.1.50:3453".to_vec()),
                    password: Text(b"hunter2".to_vec()),
                }
            ]
        );
    }
}
