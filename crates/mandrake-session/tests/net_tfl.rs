mod common;

use common::net_pipe::{ADDRESS, NetPipe, settings};
use common::{catalog, report, text};
use mandrake_common::FourCC;
use mandrake_net::endpoint::{Event, Host, HostOptions, Joiner};
use mandrake_net::game_kind::GameKind;
use mandrake_net::lobby::{BarrierKind, code};
use mandrake_net::player_data::{JoinRequest, until_nul};
use mandrake_session::net::{
    DataChecksums, JoinFailure, NetEvent, NetGameStart, NetRequest, NetScreen, Networking,
};
use mandrake_session::{Action, GameEvent, Overlay, Progress, Request, Screen, Session};

const DATA: DataChecksums = [0x1357_9bdf, 0x22cb_50e1];

struct Peer {
    session: Session,
    joiner: Option<Joiner>,
    id: Option<u8>,
    answer: bool,
    load_pending: bool,
    starts: Vec<NetGameStart>,
    requests: Vec<Request>,
}

#[derive(Default)]
struct Pipe {
    peers: Vec<Peer>,
    host: Option<(usize, Host)>,
    barriers: (bool, bool),
    now: u64,
}

impl NetPipe for Pipe {
    fn session(&self, peer: usize) -> &Session {
        &self.peers[peer].session
    }

    fn session_mut(&mut self, peer: usize) -> &mut Session {
        &mut self.peers[peer].session
    }

    fn pump(&mut self) {
        for _ in 0..100 {
            let mut moved = false;
            for i in 0..self.peers.len() {
                let requests = self.peers[i].session.take_requests();
                for r in requests {
                    moved = true;
                    self.peers[i].requests.push(r.clone());
                    self.perform(i, r);
                }
                if std::mem::take(&mut self.peers[i].load_pending) {
                    moved = true;
                    self.peers[i].session.game_event(GameEvent::Loaded);
                }
            }
            self.now += 10;
            let mut events: Vec<(usize, Vec<Event>)> = Vec::new();
            if let Some((hp, host)) = self.host.as_mut() {
                for p in self.peers.iter_mut() {
                    if let (Some(id), Some(j)) = (p.id, p.joiner.as_mut()) {
                        host.receive(id, &j.take_outgoing());
                    }
                }
                events.push((*hp, host.poll(self.now)));
                host.take_events();
                for (i, p) in self.peers.iter_mut().enumerate() {
                    if let (Some(id), Some(j)) = (p.id, p.joiner.as_mut()) {
                        j.receive(&host.take_outgoing(id));
                        events.push((i, j.poll(self.now)));
                    }
                }
            }
            for (i, list) in events {
                for e in list.iter().filter_map(NetEvent::from_endpoint) {
                    moved = true;
                    self.peers[i].session.game_event(GameEvent::Net(e));
                }
            }
            for i in 0..self.peers.len() {
                if self.peers[i].answer && self.joiner(i).is_some_and(|j| j.loaded()) {
                    self.peers[i].answer = false;
                    moved = true;
                }
            }
            if !moved {
                return;
            }
        }
        panic!("the pipe never went quiet");
    }
}

impl Pipe {
    fn add(&mut self, name: &str, kind: GameKind, data: DataChecksums) -> usize {
        let mut session = Session::new(catalog(), Progress::default());
        if kind == GameKind::Tfl {
            session.set_tfl(data);
        }
        assert_eq!(session.game_kind(), kind);
        session.game_event(GameEvent::Net(NetEvent::Available(Box::new(Networking {
            settings: settings(name),
            plugins: Vec::new(),
            foundation_checksum: 0,
            allow_downloads: false,
            server: None,
        }))));
        self.peers.push(Peer {
            session,
            joiner: None,
            id: None,
            answer: false,
            load_pending: false,
            starts: Vec::new(),
            requests: Vec::new(),
        });
        self.peers.len() - 1
    }

    fn is_host(&self, peer: usize) -> bool {
        self.host.as_ref().is_some_and(|(p, _)| *p == peer)
    }

    fn joiner(&mut self, peer: usize) -> Option<&mut Joiner> {
        if self.is_host(peer) {
            return self.host.as_mut().map(|(_, h)| h.local_mut());
        }
        self.peers[peer].joiner.as_mut()
    }

    fn perform(&mut self, peer: usize, request: Request) {
        let Request::Net(request) = request else {
            if let Request::LoadGame(game) = request {
                self.peers[peer]
                    .starts
                    .push(game.net.expect("a network game"));
                self.peers[peer].load_pending = true;
            }
            return;
        };
        match request {
            NetRequest::Host {
                password,
                player,
                description,
                ..
            } => {
                let options = HostOptions {
                    player: JoinRequest {
                        name: until_nul(&player.appearance.name).to_vec(),
                        password: Vec::new(),
                        player: *player,
                    },
                    password: password.0,
                    description: *description,
                };
                self.host = Some((peer, Host::new(options, 0xa635, self.now)));
                self.barriers = (false, false);
                self.peers[peer]
                    .session
                    .game_event(GameEvent::Net(NetEvent::Listening));
            }
            NetRequest::Join { address, request } => {
                assert_eq!(address, text(ADDRESS));
                let (_, host) = self.host.as_mut().expect("a host to join");
                let id = host.accept().expect("room");
                let kind = self.peers[peer].session.game_kind();
                self.peers[peer].joiner = Some(Joiner::new_as(kind, *request, 0x1000, self.now));
                self.peers[peer].id = Some(id);
            }
            NetRequest::Ready(ready) => {
                assert!(self.joiner(peer).unwrap().set_ready(ready));
            }
            NetRequest::Description(d) => self.host_mut().set_description(*d),
            NetRequest::Options(d) => self.host_mut().set_options(*d),
            NetRequest::Start { vets } => {
                let host = self.host_mut();
                host.set_after_game(vets.is_some());
                host.set_vets(vets);
                host.start(0x0091_a6a8);
                self.barriers = (false, false);
            }
            NetRequest::Restart => self.host_mut().configure(code::START, 0),
            NetRequest::Loaded => {
                if self.is_host(peer) && !self.barriers.0 {
                    self.barriers.0 = true;
                    self.host_mut().begin_barrier(BarrierKind::Start);
                }
                self.peers[peer].answer = true;
            }
            NetRequest::MapOver => {
                if self.is_host(peer) && !self.barriers.1 {
                    self.barriers.1 = true;
                    self.host_mut().begin_barrier(BarrierKind::End);
                }
                self.peers[peer].answer = true;
            }
            NetRequest::Leave => {
                if let Some(id) = self.peers[peer].id.take()
                    && let Some((_, host)) = self.host.as_mut()
                {
                    host.connection_lost(id);
                }
                self.peers[peer].joiner = None;
            }
            NetRequest::ForceEnd(_) | NetRequest::TradeUnits(_) => {
                panic!("not in this test: {request:?}")
            }
            _ => {}
        }
    }

    fn host_mut(&mut self) -> &mut Host {
        &mut self.host.as_mut().expect("hosting").1
    }
}

fn join_failed_text(pipe: &Pipe, peer: usize) -> (JoinFailure, String) {
    match pipe.net(peer) {
        NetScreen::JoinFailed(f) => (
            f.reason.clone(),
            String::from_utf8_lossy(&f.text.0).into_owned(),
        ),
        other => panic!("not jogf: {other:?}"),
    }
}

#[test]
fn a_tfl_host_and_joiner_play_to_endi_and_restart_on_the_next_map() {
    let mut pipe = Pipe::default();
    let h = pipe.add("Host", GameKind::Tfl, DATA);
    let j = pipe.add("Joiner", GameKind::Tfl, DATA);
    pipe.host(h, "");
    let NetScreen::Lobby(lobby) = pipe.net(h) else {
        unreachable!()
    };
    assert_eq!(GameKind::of(&lobby.description), GameKind::Tfl);
    assert_eq!(mandrake_net::tfl::film_checksums(&lobby.description), DATA);
    assert_eq!(lobby.description.parameters.mesh, *b"58cm");
    pipe.join(j, "");
    let NetScreen::Lobby(lobby) = pipe.net(j) else {
        panic!("the joiner's lobby: {:?}", pipe.net(j));
    };
    assert_eq!(lobby.players.len(), 2);

    pipe.act(j, Action::SetReady(true));
    let NetScreen::Lobby(lobby) = pipe.net(h) else {
        unreachable!()
    };
    assert!(lobby.can_start);
    pipe.act(h, Action::StartNetGame);
    assert!(pipe.playing(h) && pipe.playing(j));
    for (peer, local) in [(h, 0), (j, 1)] {
        let start = pipe.peers[peer].starts.last().unwrap().clone();
        assert_eq!(start.kind, GameKind::Tfl);
        assert!(start.new_game_data.0.is_empty(), "no Myth II new-game data");
        let tfl = start.tfl.expect("TFL's new-game data");
        assert_eq!(tfl.parameters.mode, 1);
        assert_eq!(tfl.parameters.seed, 0x0091_a6a8);
        assert_eq!(tfl.parameters.field_20, [0; 8], "no marker in the film");
        assert_eq!(tfl.players.len(), 2);
        assert_eq!(tfl.local_player, local);
        let theirs = pipe.joiner(peer).unwrap().tfl_new_game().unwrap().unwrap();
        assert_eq!(tfl, theirs, "peer {peer}");
        assert!(pipe.peers[peer].requests.contains(&Request::BeginPlay));
    }

    pipe.peers[h].requests.clear();
    pipe.act(h, Action::OpenPause);
    let Screen::Playing {
        overlay: Some(Overlay::Paused(paused)),
    } = pipe.peers[h].session.screen()
    else {
        panic!("no pause dialog");
    };
    let network = paused.network.expect("a network game's items");
    assert!(network.originator);
    assert!(!network.rehost_items && !network.can_rehost);
    pipe.act(h, Action::LastSavedGame);
    pipe.act(h, Action::RestartLevel);
    pipe.act(h, Action::Resume);
    assert!(
        !pipe.peers[h].requests.iter().any(|r| matches!(
            r,
            Request::SetPaused(_) | Request::Net(NetRequest::ForceEnd(_))
        )),
        "{:?}",
        pipe.peers[h].requests
    );
    let codes = GameKind::Tfl.commands();
    assert_eq!(
        (codes.pause, codes.game_speed, codes.forced_end),
        (None, None, None)
    );

    let next = FourCC(*b"02bt");
    for peer in [h, j] {
        pipe.event(peer, GameEvent::Net(NetEvent::NextMap(next)));
    }
    for peer in [j, h] {
        pipe.event(
            peer,
            GameEvent::MapOver {
                report: report(1),
                veterans: Vec::new(),
            },
        );
    }
    for peer in [h, j] {
        let NetScreen::Postgame(p, lobby) = pipe.net(peer) else {
            panic!("peer {peer}: {:?}", pipe.net(peer));
        };
        assert_eq!(p.can_restart, peer == h);
        assert_eq!(lobby.chat.lines().count(), 0, "peer {peer}");
    }
    pipe.act(j, Action::RestartNetGame);
    assert!(
        matches!(pipe.net(j), NetScreen::Postgame(..)),
        "the host's alone"
    );
    pipe.act(h, Action::RestartNetGame);
    for peer in [h, j] {
        let NetScreen::Lobby(lobby) = pipe.net(peer) else {
            panic!("peer {peer}: {:?}", pipe.net(peer));
        };
        assert_eq!(lobby.description.parameters.mesh, next.0, "peer {peer}");
    }
    pipe.act(j, Action::SetReady(true));
    pipe.act(h, Action::StartNetGame);
    assert!(pipe.playing(h) && pipe.playing(j));
    for peer in [h, j] {
        let tfl = pipe.peers[peer].starts.last().unwrap().tfl.clone().unwrap();
        assert_eq!(tfl.parameters.mesh, next);
    }
}

#[test]
fn the_other_games_description_fails_the_join_with_the_reason() {
    let mut pipe = Pipe::default();
    let h = pipe.add("Host", GameKind::Tfl, DATA);
    let m = pipe.add("Myth II", GameKind::Myth2, DATA);
    pipe.host(h, "");
    pipe.join(m, "");
    let (reason, text) = join_failed_text(&pipe, m);
    assert_eq!(reason, JoinFailure::InvalidGame);
    assert_eq!(
        text,
        "This is a TFL game: start the client with --game tfl.\r"
    );

    let mut pipe = Pipe::default();
    let h = pipe.add("Host", GameKind::Myth2, DATA);
    let t = pipe.add("TFL", GameKind::Tfl, DATA);
    pipe.host(h, "");
    pipe.join(t, "");
    let (reason, text) = join_failed_text(&pipe, t);
    assert_eq!(reason, JoinFailure::InvalidGame);
    assert_eq!(
        text,
        "This is a Myth II game: start the client with --game myth2.\r"
    );
}

#[test]
fn other_tfl_data_fails_the_join() {
    let mut pipe = Pipe::default();
    let h = pipe.add("Host", GameKind::Tfl, DATA);
    let a = pipe.add("Other archives", GameKind::Tfl, [7, DATA[1]]);
    let p = pipe.add(
        "No plugin",
        GameKind::Tfl,
        [DATA[0], mandrake_net::tfl::NO_PLUGIN],
    );
    let same = pipe.add("Same", GameKind::Tfl, DATA);
    pipe.host(h, "");
    pipe.join(a, "");
    let (reason, text) = join_failed_text(&pipe, a);
    assert_eq!(reason, JoinFailure::InvalidGame);
    assert_eq!(
        text,
        "The host's TFL data differs from ours (checksum 13579bdf, ours 00000007).\r"
    );
    pipe.join(p, "");
    let (_, text) = join_failed_text(&pipe, p);
    assert_eq!(
        text,
        "The host's TFL plays another plugin (checksum 22cb50e1, ours ffffffff).\r"
    );
    pipe.join(same, "");
    assert!(matches!(pipe.net(same), NetScreen::Lobby(_)));
}

#[test]
fn a_myth_ii_game_keeps_its_pause_and_forced_end() {
    let mut pipe = Pipe::default();
    let h = pipe.add("Host", GameKind::Myth2, DATA);
    let j = pipe.add("Joiner", GameKind::Myth2, DATA);
    pipe.host(h, "");
    pipe.join(j, "");
    pipe.act(j, Action::SetReady(true));
    pipe.act(h, Action::StartNetGame);
    assert!(pipe.playing(h));
    let start = pipe.peers[h].starts.last().unwrap();
    assert_eq!((start.kind, start.tfl.is_none()), (GameKind::Myth2, true));
    assert_eq!(
        start.new_game_data.0.len(),
        mandrake_rec::writer::NEW_GAME_DATA_SIZE
    );
    pipe.peers[h].requests.clear();
    pipe.act(h, Action::OpenPause);
    let Screen::Playing {
        overlay: Some(Overlay::Paused(paused)),
    } = pipe.peers[h].session.screen()
    else {
        panic!("no pause dialog");
    };
    let network = paused.network.unwrap();
    assert!(network.rehost_items && network.can_rehost);
    assert!(pipe.peers[h].requests.contains(&Request::SetPaused(true)));
    pipe.peers[h].session.act(Action::LastSavedGame);
    let requests = pipe.peers[h].session.take_requests();
    assert!(requests.contains(&Request::Net(NetRequest::ForceEnd(2))));
}
