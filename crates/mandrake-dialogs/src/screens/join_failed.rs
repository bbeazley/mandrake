use mandrake_common::FourCC;
use mandrake_session::net::{JoinFailure, NetScreen};
use mandrake_session::{Action, Text};

use super::ScreenFrame;
use crate::core::{Dialog, DialogStack, dialog_flags, format_one};
use crate::resources::Resources;

pub const PRDN: FourCC = FourCC(*b"prdn");
pub const GAPD: FourCC = FourCC(*b"gapd");
pub const JOGF: FourCC = FourCC(*b"jogf");
pub const PRDB: FourCC = FourCC(*b"prdb");
pub(super) const INBP: FourCC = FourCC(*b"inbp");
pub(super) const PRDB_FLAGS: u16 = 0x58;
const INNB: FourCC = FourCC(*b"innb");
const INJG: FourCC = FourCC(*b"injg");
pub(super) const INMN: FourCC = FourCC(*b"inmn");
pub(super) const INNS: FourCC = FourCC(*b"inns");
const STRING_OK: usize = 14;
const STRING_CREATE_FAILED: usize = 26;
const STRING_JOINING: usize = 0;
const STRING_CANCELLED: usize = 1;
const STRING_TIMED_OUT: usize = 2;
const STRING_UNABLE: usize = 3;
pub const STRING_PASSWORD_NEEDED: usize = 9;

const PRDN_TEXT: i16 = 1;
const PRDN_OK: i16 = 2;
pub const GAPD_OK: i16 = 1;
pub const GAPD_CANCEL: i16 = 2;
pub const GAPD_PASSWORD: i16 = 4;
pub(super) const PRDB_TEXT: i16 = 1;
pub const PRDB_CANCEL: i16 = 2;
pub(super) const PRDB_BAR: i16 = 3;
pub const JOINING_BOX_MS: u64 = 1000;
pub const JOIN_WAIT_MS: u64 = 5000;
const JOGF_OK: i16 = 1;
const JOGF_TEXT: i16 = 2;
const PASSWORD_LENGTH: usize = 0x1f;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Error(Vec<u8>),
    Password,
    Failed(Vec<u8>),
    Joining,
}

pub fn notice_for(screen: &NetScreen<'_>, joining_ms: u64, res: &mut Resources) -> Option<Notice> {
    match screen {
        NetScreen::Joining(_) => (joining_ms > JOINING_BOX_MS).then_some(Notice::Joining),
        NetScreen::HostOrJoin(h) => h.host_failed.as_ref().map(|reason| {
            Notice::Error(format_one(
                res.string_line(INMN, STRING_CREATE_FAILED),
                &reason.0,
            ))
        }),
        NetScreen::JoinFailed(f) => Some(match &f.reason {
            JoinFailure::Denied(reason)
                if reason.0 == res.string_line(INNS, STRING_PASSWORD_NEEDED) =>
            {
                Notice::Password
            }
            JoinFailure::Denied(reason) => Notice::Error(reason.0.clone()),
            JoinFailure::TimedOut | JoinFailure::Unreachable(_) | JoinFailure::HostLost => {
                Notice::Error(res.string_line(INNS, STRING_TIMED_OUT).to_vec())
            }
            JoinFailure::Cancelled => {
                Notice::Error(res.string_line(INNS, STRING_CANCELLED).to_vec())
            }
            JoinFailure::InProgress => {
                Notice::Error(format_one(res.string_line(INNS, STRING_UNABLE), b"0"))
            }
            JoinFailure::InvalidGame => Notice::Failed(f.text.0.clone()),
        }),
        _ => None,
    }
}

pub fn direct_notice_for(
    screen: &NetScreen<'_>,
    address: &[u8],
    joining_ms: u64,
    res: &mut Resources,
) -> Option<Notice> {
    match screen {
        NetScreen::Joining(_) => Some(Notice::Joining),
        NetScreen::JoinFailed(f)
            if matches!(
                f.reason,
                JoinFailure::TimedOut | JoinFailure::Unreachable(_) | JoinFailure::HostLost
            ) =>
        {
            Some(Notice::Error(unreachable_text(address)))
        }
        _ => notice_for(screen, joining_ms, res),
    }
}

pub fn unreachable_text(address: &[u8]) -> Vec<u8> {
    let mut text = b"Could not reach the game server at ".to_vec();
    text.extend_from_slice(address);
    text.extend_from_slice(b". It may be down or full; try again later.");
    text
}

pub fn push(notice: &Notice, res: &mut Resources, stack: &mut DialogStack) {
    let (id, background, flags) = match notice {
        Notice::Error(_) => (PRDN, INNB, dialog_flags::MODAL),
        Notice::Password => (GAPD, INNB, dialog_flags::MODAL),
        Notice::Failed(_) => (JOGF, INJG, dialog_flags::CENTERED),
        Notice::Joining => (PRDB, INBP, PRDB_FLAGS),
    };
    let Some(def) = res.dialog(id) else {
        return;
    };
    let mut dialog = Dialog::new(&def, id, flags, res);
    dialog.set_background(res.art(background), 0);
    match notice {
        Notice::Error(text) => {
            dialog.set_static_text(PRDN_TEXT, text);
            dialog.set_button_caption(PRDN_OK, res.string_line(INMN, STRING_OK));
            dialog.set_default_item(PRDN_OK);
        }
        Notice::Password => {
            dialog.set_default_item(GAPD_OK);
            dialog.set_cancel_item(GAPD_CANCEL);
        }
        Notice::Failed(text) => {
            dialog.set_default_item(JOGF_OK);
            dialog.set_visible(JOGF_TEXT, true);
            for id in 3..=6 {
                dialog.set_visible(id, false);
            }
            dialog.set_static_text(JOGF_TEXT, text);
        }
        Notice::Joining => {
            dialog.set_static_text(PRDB_TEXT, res.string_line(INNS, STRING_JOINING));
        }
    }
    stack.push(dialog);
}

pub(super) fn set_static_text(dialog: &mut Dialog, id: i16, text: &[u8]) {
    dialog.set_static_text(id, text);
}

pub fn joining_fraction(joining_ms: u64) -> i16 {
    i16::try_from(joining_ms * 0x100 / JOIN_WAIT_MS).unwrap_or(i16::MAX)
}

pub fn frame(notice: &Notice, address: &[u8], joining_ms: u64, frame: &mut ScreenFrame<'_>) {
    let id = match notice {
        Notice::Error(_) => PRDN,
        Notice::Password => GAPD,
        Notice::Failed(_) => JOGF,
        Notice::Joining => PRDB,
    };
    if *notice == Notice::Joining
        && let Some((item, bar)) = frame
            .stack
            .find_mut(PRDB)
            .and_then(|d| d.control_mut(PRDB_BAR))
            .and_then(|c| c.progress_mut())
    {
        bar.set_fraction(item, joining_fraction(joining_ms));
    }
    let hits: Vec<i16> = frame
        .hits
        .iter()
        .filter(|h| h.dialog == id)
        .map(|h| h.item)
        .collect();
    for item in hits {
        match (notice, item) {
            (Notice::Error(_), PRDN_OK) | (Notice::Failed(_), JOGF_OK) => {
                frame.out.actions.push(Action::Ok);
            }
            (Notice::Password, GAPD_OK) => {
                let mut password = frame
                    .stack
                    .find_mut(GAPD)
                    .map(|d| d.edit_text_slice(GAPD_PASSWORD).to_vec())
                    .unwrap_or_default();
                password.truncate(PASSWORD_LENGTH);
                frame.out.actions.push(Action::Ok);
                frame.out.actions.push(Action::JoinByAddress {
                    address: Text(address.to_vec()),
                    password: Text(password),
                });
            }
            (Notice::Password, GAPD_CANCEL) => frame.out.actions.push(Action::Ok),
            (Notice::Joining, PRDB_CANCEL) => frame.out.actions.push(Action::Cancel),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_argument_goes_in() {
        assert_eq!(
            format_one(b"Create game failed: %s", b"no port"),
            b"Create game failed: no port"
        );
        assert_eq!(format_one(b"(Error %d).", b"0"), b"(Error 0).");
    }

    #[test]
    fn the_bar_counts_the_five_seconds_in_256ths() {
        assert_eq!(joining_fraction(0), 0);
        assert_eq!(joining_fraction(1001), 51);
        assert_eq!(joining_fraction(3000), 153);
        assert_eq!(joining_fraction(5000), 256);
    }
}
