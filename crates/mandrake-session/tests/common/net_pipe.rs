use mandrake_session::net::{Lobby, NetScreen, NetSettings};
use mandrake_session::{Action, GameEvent, Screen, Session};

use super::text;

pub const ADDRESS: &str = "127.0.0.1:3453";

pub fn settings(name: &str) -> NetSettings {
    NetSettings {
        name: text(name),
        team: text("Team"),
        colors: [1, 2, 3, 4, 5, 6, 7, 8],
        ..NetSettings::default()
    }
}

pub trait NetPipe {
    fn session(&self, peer: usize) -> &Session;
    fn session_mut(&mut self, peer: usize) -> &mut Session;
    fn pump(&mut self);

    fn act(&mut self, peer: usize, action: Action) {
        self.session_mut(peer).act(action);
        self.pump();
    }

    fn event(&mut self, peer: usize, event: GameEvent) {
        self.session_mut(peer).game_event(event);
        self.pump();
    }

    fn screen(&self, peer: usize) -> Screen<'_> {
        self.session(peer).screen()
    }

    fn net(&self, peer: usize) -> NetScreen<'_> {
        match self.screen(peer) {
            Screen::Net(net) => net,
            other => panic!("peer {peer} isn't on a multiplayer screen: {other:?}"),
        }
    }

    fn lobby(&self, peer: usize) -> &Lobby {
        match self.net(peer) {
            NetScreen::Lobby(lobby) => lobby.lobby,
            other => panic!("peer {peer} isn't in the lobby: {other:?}"),
        }
    }

    fn playing(&self, peer: usize) -> bool {
        matches!(self.screen(peer), Screen::Playing { .. })
    }

    fn open_host_or_join(&mut self, peer: usize) {
        self.act(peer, Action::Multiplayer);
        self.act(peer, Action::ChooseOther);
        self.act(peer, Action::Ok);
        assert!(matches!(self.net(peer), NetScreen::HostOrJoin(_)));
    }

    fn host(&mut self, peer: usize, password: &str) {
        self.open_host_or_join(peer);
        self.act(
            peer,
            Action::HostGame {
                name: text("A game"),
                password: text(password),
                port: 3453,
            },
        );
        self.lobby(peer);
    }

    fn join(&mut self, peer: usize, password: &str) {
        self.open_host_or_join(peer);
        self.act(
            peer,
            Action::JoinByAddress {
                address: text(ADDRESS),
                password: text(password),
            },
        );
    }
}
