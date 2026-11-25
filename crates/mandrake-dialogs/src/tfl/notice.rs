use mandrake_net::game_description::Plugin;
use mandrake_session::net::{NetNotice, NetScreen};
use mandrake_session::{Action, Screen};

use super::lobby;
use crate::core::DialogStack;
use crate::resources::Resources;
use crate::screens::join_failed::{self, Notice};
use crate::screens::{OpenScreen, ScreenFrame};

const INMN: mandrake_common::FourCC = mandrake_common::FourCC(*b"inmn");
const STRING_JOIN_FAILED: usize = 3;
const STRING_HOST_LOST: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    HostLost,
    LoadFailed,
    LobbyFailed,
    MissingPlugins,
    Downloading,
}

pub fn kind(screen: &NetScreen<'_>) -> Option<Kind> {
    match screen {
        NetScreen::Notice(NetNotice::HostLost) => Some(Kind::HostLost),
        NetScreen::Notice(NetNotice::LoadFailed) => Some(Kind::LoadFailed),
        NetScreen::LobbyFailed(..) => Some(Kind::LobbyFailed),
        NetScreen::MissingPlugins(..) => Some(Kind::MissingPlugins),
        NetScreen::Downloading(..) => Some(Kind::Downloading),
        _ => None,
    }
}

pub fn is_notice(screen: &NetScreen<'_>) -> bool {
    kind(screen).is_some()
}

fn lobby_under<'a>(screen: &NetScreen<'a>) -> Option<NetScreen<'a>> {
    match screen {
        NetScreen::LobbyFailed(l, _)
        | NetScreen::MissingPlugins(l, _)
        | NetScreen::Downloading(l, _) => Some(NetScreen::Lobby(*l)),
        _ => None,
    }
}

fn plugin_names(plugins: &[Plugin]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, p) in plugins.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b", ");
        }
        out.extend_from_slice(mandrake_net::player_data::until_nul(&p.name));
    }
    out
}

pub fn text(screen: &NetScreen<'_>, res: &mut Resources) -> Vec<u8> {
    match screen {
        NetScreen::Notice(NetNotice::HostLost) => res.string_line(INMN, STRING_HOST_LOST).to_vec(),
        NetScreen::Notice(NetNotice::LoadFailed) => crate::core::format_one(
            res.string_line(INMN, STRING_JOIN_FAILED),
            b"the map didn't load.",
        ),
        NetScreen::LobbyFailed(_, f) => f.text.0.clone(),
        NetScreen::MissingPlugins(_, plugins) => {
            let mut t = b"This game needs plugins this install doesn't have: ".to_vec();
            t.extend_from_slice(&plugin_names(plugins));
            t
        }
        NetScreen::Downloading(_, d) => {
            let mut t = b"Downloading plugins: ".to_vec();
            t.extend_from_slice(&plugin_names(&d.plugins));
            t
        }
        _ => Vec::new(),
    }
}

pub(super) fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let under = lobby_under(screen).and_then(|l| lobby::open(&l, res, stack));
    let notice = Notice::Error(text(screen, res));
    join_failed::push(&notice, res, stack);
    Some(Box::new(Open { under, notice }))
}

struct Open {
    under: Option<Box<dyn OpenScreen>>,
    notice: Notice,
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(net) = screen else {
            return;
        };
        if let (Some(under), Some(l)) = (&mut self.under, lobby_under(net)) {
            under.frame(&Screen::Net(l), frame);
        }
        let wanted = Notice::Error(text(net, frame.res));
        if wanted != self.notice {
            if let Notice::Error(t) = &wanted
                && let Some(dialog) = frame.stack.find_mut(join_failed::PRDN)
            {
                dialog.set_static_text(1, t);
            }
            self.notice = wanted;
        }
        let start = frame.out.actions.len();
        join_failed::frame(&self.notice, &[], 0, frame);
        if matches!(net, NetScreen::Downloading(..)) {
            for a in &mut frame.out.actions[start..] {
                if *a == Action::Ok {
                    *a = Action::Cancel;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_kinds_are_distinct_for_backdrop_transitions() {
        let description = mandrake_session::net::GameDescription::read(&[0u8; 0x23e]).unwrap();
        let lobby = mandrake_session::net::Lobby {
            host: false,
            owner: false,
            local_id: 1,
            players: Vec::new(),
            description,
            chat: Default::default(),
            locked: false,
            can_start: false,
            host_controls: false,
            details: None,
        };
        let lobby_screen = mandrake_session::net::LobbyScreen {
            lobby: &lobby,
            latencies: &[],
            replies: &Default::default(),
            show_ping: false,
        };
        let download = mandrake_session::net::PluginDownload {
            plugins: Vec::new(),
            progress: Default::default(),
        };
        assert_eq!(
            kind(&NetScreen::Notice(NetNotice::HostLost)),
            Some(Kind::HostLost)
        );
        assert_eq!(
            kind(&NetScreen::Notice(NetNotice::LoadFailed)),
            Some(Kind::LoadFailed)
        );
        assert_eq!(
            kind(&NetScreen::MissingPlugins(lobby_screen, &[])),
            Some(Kind::MissingPlugins)
        );
        assert_eq!(
            kind(&NetScreen::Downloading(lobby_screen, &download)),
            Some(Kind::Downloading)
        );
        assert_ne!(
            kind(&NetScreen::MissingPlugins(lobby_screen, &[])),
            kind(&NetScreen::Notice(NetNotice::HostLost))
        );
    }
}
