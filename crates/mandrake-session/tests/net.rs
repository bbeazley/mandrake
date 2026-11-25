mod common;

use std::collections::VecDeque;

use common::net_pipe::{ADDRESS, NetPipe, settings};
use common::{catalog, report, text};
use mandrake_common::game_options;
use mandrake_net::game_description::{GameDescription, Plugin, new_game_data};
use mandrake_net::lobby::OPTION_VETS;

const VETERANS_BOTH: u32 = game_options::VETERANS | game_options::CARRY_VETERANS;
use mandrake_net::player_data::{JoinRequest, PlayerData, until_nul};
use mandrake_net::wire::{Body, Chat, Message, TO_ALL, TO_HOST};
use mandrake_rec::header::MonsterInitializer;
use mandrake_session::net::{
    CODE_START, ChatKind, ChatLine, JoinFailure, NetEvent, NetNotice, NetRequest, NetScreen,
    NetSettings, Networking,
};
use mandrake_session::{
    Action, GameEvent, GameStart, Overlay, Progress, Request, Screen, Session, Text,
};

const PLUGIN: &str = "A plugin map";
const PLUGIN_CHECKSUM: u32 = 0x1234_5678;
const MESH: [u8; 4] = *b"85gi";

fn plugin() -> Plugin {
    Plugin {
        checksum: PLUGIN_CHECKSUM,
        name: PLUGIN.as_bytes().to_vec(),
        url: Vec::new(),
    }
}

struct Peer {
    session: Session,
    id: Option<i32>,
    connecting_until: Option<u64>,
    inbox: VecDeque<Vec<u8>>,
    known: Vec<i32>,
    barrier_open: bool,
    barrier_answers: usize,
    answer_wanted: bool,
    load_pending: bool,
    hold_load: bool,
    loads: Vec<GameStart>,
    playing: bool,
    saved_settings: Vec<NetSettings>,
    films: Vec<Text>,
    veterans_files: Vec<(Vec<u8>, Vec<u8>)>,
    requests: Vec<Request>,
    local: VecDeque<NetEvent>,
}

impl Peer {
    fn new(name: &str, plugins: Vec<Plugin>) -> Peer {
        let mut session = Session::new(catalog(), Progress::default());
        session.game_event(GameEvent::Net(NetEvent::Available(Box::new(Networking {
            settings: settings(name),
            plugins,
            foundation_checksum: 0x0bad_f00d,
            allow_downloads: false,
            server: None,
        }))));
        Peer {
            session,
            id: None,
            connecting_until: None,
            inbox: VecDeque::new(),
            known: Vec::new(),
            barrier_open: false,
            barrier_answers: 0,
            answer_wanted: false,
            load_pending: false,
            hold_load: false,
            loads: Vec::new(),
            playing: false,
            saved_settings: Vec::new(),
            veterans_files: Vec::new(),
            films: Vec::new(),
            requests: Vec::new(),
            local: VecDeque::new(),
        }
    }
}

#[derive(Default)]
struct Hub {
    host: Option<usize>,
    password: Vec<u8>,
    locked: bool,
    started: bool,
    start_list: Vec<u8>,
    description: Option<GameDescription>,
    vets: Option<Vec<u8>>,
    players: Vec<(usize, PlayerData)>,
    next_id: i32,
}

#[derive(Default)]
struct Pipe {
    peers: Vec<Peer>,
    hub: Hub,
    now_ms: u64,
}

fn bytes(to: u8, from: u8, body: Body) -> Vec<u8> {
    Message::new(to, from, body).encode().expect("fits")
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
                for request in requests {
                    moved = true;
                    self.peers[i].requests.push(request.clone());
                    self.perform(i, request);
                }
                while let Some(event) = self.peers[i].local.pop_front() {
                    moved = true;
                    self.peers[i].session.game_event(GameEvent::Net(event));
                }
                while let Some(message) = self.peers[i].inbox.pop_front() {
                    moved = true;
                    self.receive(i, &message);
                }
                if !self.peers[i].hold_load && std::mem::take(&mut self.peers[i].load_pending) {
                    moved = true;
                    self.peers[i].session.game_event(GameEvent::Loaded);
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
    fn add(&mut self, name: &str, plugins: Vec<Plugin>) -> usize {
        self.peers.push(Peer::new(name, plugins));
        self.peers.len() - 1
    }

    fn advance(&mut self, ms: u64) {
        self.now_ms += ms;
        for peer in &mut self.peers {
            if peer.connecting_until.is_some_and(|t| t <= self.now_ms) {
                peer.connecting_until = None;
                peer.local.push_back(NetEvent::TimedOut);
            }
        }
        self.pump();
    }

    fn send(&mut self, peer: usize, message: Vec<u8>) {
        self.peers[peer].inbox.push_back(message);
    }

    fn broadcast(&mut self, body: &Body) {
        let to: Vec<usize> = self.hub.players.iter().map(|&(p, _)| p).collect();
        for peer in to {
            self.send(peer, bytes(TO_ALL, 0, body.clone()));
        }
    }

    fn player_body(id: i32, flags: u16, player: &PlayerData) -> Body {
        Body::NewPlayer {
            id: id as u16,
            flags,
            name: until_nul(&player.appearance.name).to_vec(),
            player_data: player.to_bytes().to_vec(),
        }
    }

    fn perform(&mut self, i: usize, request: Request) {
        match request {
            Request::Net(request) => self.net_request(i, request),
            Request::LoadGame(start) => {
                self.peers[i].loads.push(start);
                self.peers[i].load_pending = true;
            }
            Request::BeginPlay => self.peers[i].playing = true,
            Request::DropGame => self.peers[i].playing = false,
            Request::SaveNetPlayer(settings) => self.peers[i].saved_settings.push(settings),
            Request::SaveFilm { name } => {
                self.peers[i].films.push(name.clone());
                self.peers[i]
                    .session
                    .game_event(GameEvent::FilmSaved(Some(name.0)));
            }
            _ => {}
        }
    }

    fn net_request(&mut self, i: usize, request: NetRequest) {
        let host = self.hub.host == Some(i);
        match request {
            NetRequest::Host {
                port,
                password,
                player,
                description,
            } => {
                if port == 0 {
                    self.peers[i]
                        .local
                        .push_back(NetEvent::HostFailed(text("Couldn't listen.")));
                    return;
                }
                self.hub = Hub {
                    host: Some(i),
                    password: password.0,
                    description: Some(*description),
                    players: vec![(i, (*player).clone())],
                    next_id: 1,
                    ..Hub::default()
                };
                self.peers[i].id = Some(0);
                self.peers[i].local.push_back(NetEvent::Listening);
                self.send(i, bytes(TO_ALL, 0, Self::player_body(0, 1, &player)));
            }
            NetRequest::Join { address, request } => {
                if address.0 != ADDRESS.as_bytes() || self.hub.host.is_none() {
                    self.peers[i].connecting_until = Some(self.now_ms + 30_000);
                    return;
                }
                let message = bytes(TO_HOST, 0xff, request.to_body());
                self.hub_receive(i, &message);
            }
            NetRequest::Ready(ready) => {
                let id = self.peers[i].id.expect("in the game");
                let mut me = self.hub_player(i).clone();
                me.ready = u32::from(ready);
                let message = bytes(
                    TO_HOST,
                    id as u8,
                    Body::PlayerDataChanged {
                        id: id as u16,
                        player_data: me.to_bytes().to_vec(),
                    },
                );
                self.hub_receive(i, &message);
            }
            NetRequest::Description(description) | NetRequest::Options(description) => {
                assert!(host, "only the host sends the description");
                self.broadcast(&description.to_body());
                self.hub.description = Some(*description);
            }
            NetRequest::Start { vets } => {
                assert!(host, "only the host starts");
                let mut description = self.hub.description.clone().expect("described");
                if vets.is_some() {
                    description.parameters.options |= OPTION_VETS;
                } else {
                    description.parameters.options &= !OPTION_VETS;
                }
                self.hub.vets = vets;
                description
                    .parameters
                    .reset_seeds(self.now_ms as u32 + 0x130df);
                self.hub.description = Some(description.clone());
                self.hub.started = true;
                self.hub.start_list = self
                    .hub
                    .players
                    .iter()
                    .flat_map(|(_, p)| {
                        PlayerData {
                            ready: 1,
                            ..p.clone()
                        }
                        .to_bytes()
                    })
                    .collect();
                self.broadcast(&description.to_body());
                self.broadcast(&Body::Configuration {
                    flags: 0,
                    code: CODE_START,
                    value: 0,
                });
                self.broadcast(&Body::BeginSyncData);
            }
            NetRequest::Restart => {
                assert!(host, "only the host restarts");
                self.broadcast(&Body::Configuration {
                    flags: 0,
                    code: CODE_START,
                    value: 0,
                });
            }
            NetRequest::SaveVeterans { name, table } => {
                let saved = (!table.is_empty()).then(|| {
                    let files = &mut self.peers[i].veterans_files;
                    files.retain(|(n, _)| *n != name.0);
                    files.push((name.0.clone(), table));
                    true
                });
                self.peers[i]
                    .local
                    .push_back(NetEvent::VeteransSaved { name, saved });
            }
            NetRequest::LoadVeterans { name } => {
                let table = self.peers[i]
                    .veterans_files
                    .iter()
                    .find(|(n, _)| *n == name.0)
                    .and_then(|(_, b)| mandrake_session::chat_commands::parse_veterans(b));
                self.peers[i]
                    .local
                    .push_back(NetEvent::VeteransLoaded { name, table });
            }
            NetRequest::Chat(text) => {
                let name = self.hub_player(i).appearance.name;
                let mut words = [0; 8];
                words[5] = mandrake_net::lobby::chat::ALL;
                let chat = Body::Chat(Chat {
                    name,
                    words,
                    text: text.0,
                });
                if host {
                    self.broadcast(&chat);
                } else {
                    let message = bytes(TO_HOST, self.peers[i].id.unwrap() as u8, chat);
                    self.hub_receive(i, &message);
                }
            }
            NetRequest::TeamButton(_)
            | NetRequest::TeamKick(_)
            | NetRequest::TeamLock
            | NetRequest::ReloadPlugins
            | NetRequest::DownloadPlugins { .. }
            | NetRequest::CancelDownload => {}
            NetRequest::Kick(id) => {
                assert!(host, "only the host kicks");
                let kicked = self
                    .hub
                    .players
                    .iter()
                    .find(|(_, p)| p.net_id == id)
                    .map(|&(p, _)| p)
                    .expect("a player");
                self.drop_peer(kicked);
            }
            NetRequest::Lock(locked) => {
                assert!(host, "only the host locks");
                self.hub.locked = locked;
            }
            NetRequest::Password(password) => {
                assert!(host, "only the host sets the password");
                self.hub.password = password.0;
            }
            NetRequest::Leave => {
                self.peers[i].connecting_until = None;
                if host {
                    let others: Vec<usize> = self
                        .hub
                        .players
                        .iter()
                        .map(|&(p, _)| p)
                        .filter(|&p| p != i)
                        .collect();
                    for p in others {
                        self.peers[p].local.push_back(NetEvent::HostLost);
                        self.peers[p].id = None;
                    }
                    self.hub = Hub::default();
                } else if self.hub.players.iter().any(|&(p, _)| p == i) {
                    self.leave_peer(i);
                }
                self.peers[i].id = None;
                self.peers[i].known.clear();
            }
            NetRequest::Standings => {
                self.peers[i]
                    .local
                    .push_back(NetEvent::Standings(Box::new(report(0))));
            }
            NetRequest::Loaded => {
                self.peers[i].answer_wanted = true;
                self.answer_barrier(i);
            }
            NetRequest::MapOver => {
                self.peers[i].answer_wanted = true;
                if host {
                    self.broadcast(&Body::BeginSyncData);
                }
                self.answer_barrier(i);
            }
            NetRequest::ForceEnd(_) => {}
            NetRequest::TradeUnits(_) | NetRequest::NetSave(_) => {}
        }
    }

    fn hub_player(&self, peer: usize) -> &PlayerData {
        &self
            .hub
            .players
            .iter()
            .find(|&&(p, _)| p == peer)
            .expect("in the game")
            .1
    }

    fn leave_peer(&mut self, peer: usize) {
        let id = self.hub_player(peer).net_id;
        self.hub.players.retain(|&(p, _)| p != peer);
        self.broadcast(&Body::PlayerDeleted { id: id as u16 });
    }

    fn drop_peer(&mut self, peer: usize) {
        self.leave_peer(peer);
        self.peers[peer].local.push_back(NetEvent::HostLost);
        self.peers[peer].id = None;
    }

    fn answer_barrier(&mut self, i: usize) {
        let peer = &mut self.peers[i];
        if !(peer.barrier_open && peer.answer_wanted) {
            return;
        }
        peer.answer_wanted = false;
        let id = peer.id.expect("in the game");
        let message = bytes(
            TO_HOST,
            id as u8,
            Body::PlayerSyncData {
                id: id as u16,
                value: 1,
            },
        );
        self.hub_receive(i, &message);
    }

    fn hub_receive(&mut self, from: usize, message: &[u8]) {
        let message = Message::decode(message).expect("decodes");
        match message.body {
            body @ Body::JoinRequest { .. } => {
                let request = JoinRequest::from_body(&body).expect("a join request");
                let deny = |reason: &str| {
                    bytes(
                        0xff,
                        0,
                        Body::JoinDenied {
                            reason: reason.as_bytes().to_vec(),
                            error: None,
                        },
                    )
                };
                if self.hub.locked {
                    self.send(from, deny("The game is closed."));
                    return;
                }
                if request.password != self.hub.password {
                    self.send(from, deny("Wrong password."));
                    return;
                }
                if self.hub.players.len() >= 16 {
                    self.send(from, deny("The game is full."));
                    return;
                }
                let id = self.hub.next_id;
                self.hub.next_id += 1;
                let mut player = request.player;
                player.net_id = id;
                let others = self.hub.players.clone();
                for (p, other) in &others {
                    self.send(
                        from,
                        bytes(id as u8, 0, Self::player_body(other.net_id, 0, other)),
                    );
                    self.send(*p, bytes(*p as u8, 0, Self::player_body(id, 0, &player)));
                }
                self.send(from, bytes(id as u8, 0, Self::player_body(id, 1, &player)));
                self.hub.players.push((from, player));
                let description = self.hub.description.clone().expect("described");
                self.send(from, bytes(id as u8, 0, description.to_body()));
                if self.hub.started {
                    let count = (self.hub.start_list.len() / 0x7c) as u16;
                    let mut raw = count.to_be_bytes().to_vec();
                    raw.extend_from_slice(&self.hub.start_list);
                    self.send(from, bytes(id as u8, 0, Body::BeginGameMidgame(raw)));
                    self.send(from, bytes(id as u8, 0, Body::BeginSyncData));
                }
            }
            Body::PlayerDataChanged { id, player_data } => {
                let player = PlayerData::read(&player_data).expect("a record");
                if let Some((_, p)) = self.hub.players.iter_mut().find(|(p, _)| *p == from) {
                    *p = player;
                }
                self.broadcast(&Body::PlayerDataChanged { id, player_data });
            }
            body @ Body::Chat(_) => self.broadcast(&body),
            body @ Body::PlayerSyncData { .. } => self.broadcast(&body),
            body => panic!("the hub doesn't expect {body:?}"),
        }
    }

    fn receive(&mut self, i: usize, message: &[u8]) {
        let message = Message::decode(message).expect("decodes");
        let peer = &mut self.peers[i];
        let mut events = Vec::new();
        match message.body {
            Body::NewPlayer {
                id,
                flags,
                player_data,
                ..
            } => {
                let player = PlayerData::read(&player_data).expect("a record");
                let id = i32::from(id);
                if flags & 1 != 0 {
                    peer.id = Some(id);
                    events.push(NetEvent::Joined { id });
                }
                if !peer.known.contains(&id) {
                    peer.known.push(id);
                }
                events.push(NetEvent::Player(Box::new(player)));
            }
            Body::PlayerDataChanged { player_data, .. } => {
                let player = PlayerData::read(&player_data).expect("a record");
                events.push(NetEvent::Player(Box::new(player)));
            }
            Body::JoinDenied { reason, .. } => events.push(NetEvent::Denied(Text(reason))),
            Body::PlayerDeleted { id } => {
                peer.known.retain(|&k| k != i32::from(id));
                events.push(NetEvent::PlayerLeft { id: i32::from(id) });
            }
            body @ Body::GameDescription(_) => {
                events.push(match GameDescription::from_body(&body) {
                    Ok(description) => NetEvent::Description(Box::new(description)),
                    Err(_) => NetEvent::BadDescription,
                })
            }
            Body::Configuration { code, .. } => events.push(NetEvent::Configuration(code)),
            Body::BeginGameMidgame(raw) => events.push(NetEvent::Midgame(raw[2..].to_vec())),
            Body::Chat(chat) => events.push(NetEvent::Chat {
                name: Text(until_nul(&chat.name).to_vec()),
                kind: mandrake_session::net::ChatKind::of_mode(chat.words[5]),
                text: Text(chat.text),
            }),
            Body::BeginSyncData => {
                peer.barrier_open = true;
                peer.barrier_answers = 0;
                events.push(NetEvent::BeginSync);
            }
            Body::PlayerSyncData { .. } => {
                peer.barrier_answers += 1;
                if peer.barrier_answers == peer.known.len() {
                    peer.barrier_open = false;
                    events.push(NetEvent::AllSynced);
                }
            }
            body => panic!("the client doesn't expect {body:?}"),
        }
        for event in events {
            self.peers[i].session.game_event(GameEvent::Net(event));
        }
        self.answer_barrier(i);
    }

    fn net_requests(&self, peer: usize) -> Vec<NetRequest> {
        self.peers[peer]
            .requests
            .iter()
            .filter_map(|r| match r {
                Request::Net(r) => Some(r.clone()),
                _ => None,
            })
            .collect()
    }

    fn clear_requests(&mut self) {
        for peer in &mut self.peers {
            peer.requests.clear();
        }
    }

    fn set_map(&mut self, host: usize, with_plugin: bool) {
        self.act(host, Action::OpenOptions);
        let NetScreen::GameOptions(options) = self.net(host) else {
            panic!("no options");
        };
        let mut description = options.description.clone();
        description.parameters.mesh = MESH;
        description.parameters.time_limit = 1800;
        if with_plugin {
            description.plugins.0.push(plugin());
        }
        self.act(host, Action::SetGameOptions(Box::new(description)));
        self.act(host, Action::Ok);
    }
}

fn lobby_of_two() -> (Pipe, usize, usize) {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.host(host, "");
    pipe.set_map(host, false);
    pipe.join(joiner, "");
    pipe.lobby(joiner);
    pipe.clear_requests();
    (pipe, host, joiner)
}

#[test]
fn multiplayer_is_enabled_once_networking_is_available() {
    let mut s = Session::new(catalog(), Progress::default());
    let Screen::MainMenu(menu) = s.screen() else {
        panic!("not the menu");
    };
    assert!(!menu.multiplayer);
    s.act(Action::Multiplayer);
    assert!(matches!(s.screen(), Screen::MainMenu(_)));

    let mut pipe = Pipe::default();
    let peer = pipe.add("Host", Vec::new());
    let Screen::MainMenu(menu) = pipe.screen(peer) else {
        panic!("not the menu");
    };
    assert!(menu.multiplayer);
    pipe.act(peer, Action::Multiplayer);
    let NetScreen::Player(player) = pipe.net(peer) else {
        panic!("not nesp");
    };
    assert_eq!(player.name, text("Host"));
    assert!(!player.metaserver, "the metaserver is drawn disabled");
}

#[test]
fn cancel_on_the_protocols_goes_to_the_main_menu() {
    let mut pipe = Pipe::default();
    let peer = pipe.add("Host", Vec::new());
    pipe.act(peer, Action::Multiplayer);
    pipe.act(
        peer,
        Action::SetNetPlayer {
            name: text("Alric"),
            team: text("Legion"),
            icon: 3,
            colors: [9; 8],
        },
    );
    pipe.act(peer, Action::ChooseOther);
    assert!(matches!(pipe.net(peer), NetScreen::Protocol(_)));
    pipe.act(peer, Action::Cancel);
    assert!(matches!(pipe.screen(peer), Screen::MainMenu(_)));
    assert!(pipe.peers[peer].saved_settings.is_empty());
    pipe.act(peer, Action::Multiplayer);
    let NetScreen::Player(player) = pipe.net(peer) else {
        panic!("not nesp");
    };
    assert_eq!((&player.name, player.icon), (&text("Host"), 0));
}

#[test]
fn the_team_goes_over_the_old_field() {
    let mut s = NetSettings {
        team: Text(b"\xd5s Team".to_vec()),
        ..NetSettings::default()
    };
    s.set_team(&text("Blue"));
    assert_eq!(s.team, Text(b"Blue\0am".to_vec()));
    assert_eq!(s.team_text(), text("Blue"));
    s.icon = 5;
    let a = s.player_data().appearance;
    assert_eq!(&a.team_name[..8], b"Blue\0am\0");
    assert_eq!(a.field_00, 5);
    s.set_team(&text("A longer team name"));
    s.set_team(&text("Red"));
    assert_eq!(s.team, Text(b"Red\0nger team name".to_vec()));
}

#[test]
fn the_name_goes_over_the_old_field() {
    let mut s = NetSettings {
        name: text("Soulblighter"),
        ..NetSettings::default()
    };
    s.set_name(&text("Alric"));
    assert_eq!(s.name, Text(b"Alric\0ighter".to_vec()));
    assert_eq!(s.name_text(), text("Alric"));
    assert_eq!(&s.player_data().appearance.name[..13], b"Alric\0ighter\0");
}

#[test]
fn hosting_opens_the_lobby_on_listening_or_reports_the_failure() {
    let mut pipe = Pipe::default();
    let peer = pipe.add("Host", Vec::new());
    pipe.open_host_or_join(peer);
    pipe.act(
        peer,
        Action::HostGame {
            name: text("A game"),
            password: Text::default(),
            port: 0,
        },
    );
    let NetScreen::HostOrJoin(dialog) = pipe.net(peer) else {
        panic!("not Inet");
    };
    assert_eq!(dialog.host_failed, Some(text("Couldn't listen.")));
    assert!(!dialog.hosting);
    pipe.act(peer, Action::Ok);
    assert!(matches!(pipe.net(peer), NetScreen::HostOrJoin(d) if d.host_failed.is_none()));

    pipe.act(
        peer,
        Action::HostGame {
            name: text("A game"),
            password: Text::default(),
            port: 3453,
        },
    );
    let lobby = pipe.lobby(peer);
    assert!(lobby.host && lobby.host_controls);
    assert_eq!(lobby.local_id, 0);
    assert_eq!(
        lobby.players.len(),
        1,
        "the loopback's 0x8003 is our own record"
    );
    assert_eq!(lobby.description.game_name, b"A game");
    assert_eq!(lobby.description.foundation_checksum, 0x0bad_f00d);
    assert!(matches!(
        pipe.net_requests(peer).last(),
        Some(NetRequest::Host { port: 3453, .. })
    ));
    let ports: Vec<u16> = pipe.peers[peer]
        .saved_settings
        .iter()
        .map(|s| s.port)
        .collect();
    assert_eq!(ports[1..], [0, 3453]);
}

#[test]
fn a_join_reaches_the_lobby_with_the_host_and_its_description() {
    let (pipe, host, joiner) = lobby_of_two();
    let lobby = pipe.lobby(joiner);
    assert!(!lobby.host && !lobby.host_controls && !lobby.can_start);
    assert_eq!(lobby.local_id, 1);
    let names: Vec<&[u8]> = lobby
        .players
        .iter()
        .map(|p| until_nul(&p.appearance.name))
        .collect();
    assert_eq!(names, [&b"Host"[..], b"Joiner"]);
    assert_eq!(lobby.description.parameters.mesh, MESH);
    assert_eq!(pipe.lobby(host).players.len(), 2);
    let saved = pipe.peers[joiner].saved_settings.last().expect("saved");
    assert_eq!(saved.address, text(ADDRESS), "the address is kept");
}

#[test]
fn a_denied_join_shows_the_hosts_reason_and_ok_goes_back() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.host(host, "secret");
    pipe.join(joiner, "wrong");
    let NetScreen::JoinFailed(failed) = pipe.net(joiner) else {
        panic!("not jogf");
    };
    assert_eq!(failed.reason, JoinFailure::Denied(text("Wrong password.")));
    assert_eq!(pipe.net_requests(joiner).last(), Some(&NetRequest::Leave));
    pipe.act(joiner, Action::Ok);
    assert!(matches!(pipe.net(joiner), NetScreen::HostOrJoin(_)));

    pipe.act(
        joiner,
        Action::JoinByAddress {
            address: text(ADDRESS),
            password: text("secret"),
        },
    );
    pipe.lobby(joiner);
}

#[test]
fn a_lan_game_joins_at_its_address_and_keeps_the_last_address() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.host(host, "");
    pipe.open_host_or_join(joiner);
    let saved = pipe.peers[joiner].saved_settings.len();
    pipe.act(
        joiner,
        Action::JoinLanGame {
            address: text(ADDRESS),
        },
    );
    assert!(pipe.net_requests(joiner).iter().any(|r| matches!(
        r,
        NetRequest::Join { address, request } if *address == text(ADDRESS) && request.password.is_empty()
    )));
    pipe.lobby(joiner);
    assert_eq!(
        pipe.peers[joiner].saved_settings.len(),
        saved,
        "nothing saved"
    );
}

#[test]
fn a_locked_game_denies_a_join() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.host(host, "");
    pipe.act(host, Action::Lock(true));
    assert!(pipe.lobby(host).locked);
    pipe.join(joiner, "");
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::JoinFailed(f) if f.reason == JoinFailure::Denied(text("The game is closed."))
    ));
}

#[test]
fn a_join_with_nobody_there_times_out() {
    let mut pipe = Pipe::default();
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.join(joiner, "");
    assert!(matches!(pipe.net(joiner), NetScreen::Joining(j) if j.address == text(ADDRESS)));
    pipe.advance(29_999);
    assert!(matches!(pipe.net(joiner), NetScreen::Joining(_)));
    pipe.advance(1);
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::JoinFailed(f) if f.reason == JoinFailure::TimedOut
    ));
}

#[test]
fn a_join_with_no_connection_carries_the_error() {
    let mut pipe = Pipe::default();
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.join(joiner, "");
    pipe.session_mut(joiner)
        .game_event(GameEvent::Net(NetEvent::Unreachable { error: -4994 }));
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::JoinFailed(f) if f.reason == JoinFailure::Unreachable(-4994)
    ));
}

#[test]
fn an_unresolved_address_goes_back_to_inet_with_no_notice() {
    let mut pipe = Pipe::default();
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.join(joiner, "");
    pipe.session_mut(joiner)
        .game_event(GameEvent::Net(NetEvent::Unresolved));
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::HostOrJoin(h) if h.address == text(ADDRESS) && !h.hosting
    ));
}

#[test]
fn joining_can_be_cancelled() {
    let mut pipe = Pipe::default();
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.join(joiner, "");
    pipe.act(joiner, Action::Cancel);
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::JoinFailed(f) if f.reason == JoinFailure::Cancelled
    ));
    assert_eq!(pipe.net_requests(joiner).last(), Some(&NetRequest::Leave));
    pipe.act(joiner, Action::Ok);
    assert!(matches!(pipe.net(joiner), NetScreen::HostOrJoin(_)));
}

#[test]
fn missing_plugins_show_over_the_lobby_then_leave_for_the_main_menu() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", vec![plugin()]);
    let without = pipe.add("Without", Vec::new());
    let with = pipe.add("With", vec![plugin()]);
    pipe.host(host, "");
    pipe.set_map(host, true);
    pipe.join(without, "");
    let NetScreen::MissingPlugins(lobby, missing) = pipe.net(without) else {
        panic!("not srsw: {:?}", pipe.net(without));
    };
    assert_eq!(missing, [plugin()]);
    assert!(!lobby.host);
    assert_eq!(lobby.players.len(), 2, "in the host's lobby");
    assert_eq!(pipe.lobby(host).players.len(), 2);
    assert!(!pipe.net_requests(without).contains(&NetRequest::Leave));

    pipe.act(without, Action::Ok);
    assert!(matches!(pipe.screen(without), Screen::MainMenu(_)));
    assert_eq!(pipe.net_requests(without).last(), Some(&NetRequest::Leave));
    assert_eq!(pipe.lobby(host).players.len(), 1, "the joiner left");

    pipe.join(with, "");
    pipe.lobby(with);
}

#[test]
fn a_plugin_of_another_checksum_is_missing() {
    let mut other = plugin();
    other.checksum ^= 1;
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", vec![plugin()]);
    let joiner = pipe.add("Joiner", vec![other]);
    pipe.host(host, "");
    pipe.set_map(host, true);
    pipe.join(joiner, "");
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::MissingPlugins(_, missing) if missing == [plugin()]
    ));
}

#[test]
fn a_new_map_with_a_plugin_we_lack_shows_srsw() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.set_map(host, true);
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::MissingPlugins(_, missing) if missing == [plugin()]
    ));
    assert!(
        matches!(pipe.net(host), NetScreen::Lobby(_)),
        "not the host"
    );
    pipe.act(joiner, Action::Ok);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
}

#[test]
fn srsws_reload_reads_the_plugins_again() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.set_map(host, true);
    pipe.clear_requests();
    pipe.act(joiner, Action::ReloadPlugins);
    assert_eq!(pipe.net_requests(joiner), [NetRequest::ReloadPlugins]);
    pipe.event(joiner, GameEvent::Net(NetEvent::Plugins(Vec::new())));
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::MissingPlugins(_, missing) if missing == [plugin()]
    ));
    pipe.event(joiner, GameEvent::Net(NetEvent::Plugins(vec![plugin()])));
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::MissingPlugins(_, missing) if missing.is_empty()
    ));
    pipe.act(joiner, Action::Ok);
    assert!(matches!(pipe.net(joiner), NetScreen::Lobby(_)));
    assert_eq!(pipe.lobby(host).players.len(), 2, "still in the game");
}

fn downloading(joiner: usize, pipe: &mut Pipe, allow: bool) {
    pipe.event(
        joiner,
        GameEvent::Net(NetEvent::Available(Box::new(Networking {
            settings: settings("Joiner"),
            plugins: Vec::new(),
            foundation_checksum: 0,
            allow_downloads: allow,
            server: None,
        }))),
    );
}

const URL: &str = "http://127.0.0.1:8080/plugins/";

#[test]
fn a_missing_plugin_is_downloaded_from_our_server() {
    let (mut pipe, host, joiner) = lobby_of_two();
    downloading(joiner, &mut pipe, true);
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadsFrom(text(URL))));
    pipe.set_map(host, true);
    assert_eq!(
        pipe.net_requests(joiner),
        [NetRequest::DownloadPlugins {
            url: text(URL),
            plugins: vec![plugin()],
        }]
    );
    let progress = mandrake_session::net::DownloadProgress {
        index: 0,
        received: 512 * 1024,
        total: 1024 * 1024,
    };
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadProgress(progress)));
    let NetScreen::Downloading(lobby, d) = pipe.net(joiner) else {
        panic!("not downloading: {:?}", pipe.net(joiner));
    };
    assert_eq!(lobby.players.len(), 2);
    assert_eq!(d.fraction(), 0x80);
    assert_eq!(
        String::from_utf8_lossy(&d.text().0),
        format!("Downloading {PLUGIN} (1 of 1): 0.5 of 1.0 MB")
    );
    pipe.event(joiner, GameEvent::Net(NetEvent::Plugins(vec![plugin()])));
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadDone(None)));
    assert!(matches!(pipe.net(joiner), NetScreen::Lobby(_)));
    assert_eq!(pipe.lobby(host).players.len(), 2, "still in the game");
}

#[test]
fn a_failed_or_cancelled_or_refused_download_shows_srsw_or_leaves() {
    let (mut pipe, host, joiner) = lobby_of_two();
    downloading(joiner, &mut pipe, true);
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadsFrom(text(URL))));
    pipe.set_map(host, true);
    pipe.event(joiner, GameEvent::Net(NetEvent::Plugins(Vec::new())));
    pipe.event(
        joiner,
        GameEvent::Net(NetEvent::DownloadDone(Some(text("checksum")))),
    );
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::MissingPlugins(_, missing) if missing == [plugin()]
    ));
    pipe.clear_requests();
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadsFrom(text(URL))));
    assert!(pipe.net_requests(joiner).is_empty(), "tried once");

    let (mut pipe, host, joiner) = lobby_of_two();
    downloading(joiner, &mut pipe, true);
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadsFrom(text(URL))));
    pipe.set_map(host, true);
    pipe.clear_requests();
    pipe.act(joiner, Action::Cancel);
    assert_eq!(
        pipe.net_requests(joiner),
        [NetRequest::CancelDownload, NetRequest::Leave]
    );
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));

    let (mut pipe, host, joiner) = lobby_of_two();
    downloading(joiner, &mut pipe, false);
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadsFrom(text(URL))));
    pipe.set_map(host, true);
    assert!(matches!(pipe.net(joiner), NetScreen::MissingPlugins(..)));
    assert!(pipe.net_requests(joiner).is_empty());

    let (mut pipe, host, joiner) = lobby_of_two();
    downloading(joiner, &mut pipe, true);
    pipe.set_map(host, true);
    assert!(matches!(pipe.net(joiner), NetScreen::MissingPlugins(..)));
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadsFrom(text(URL))));
    assert!(matches!(pipe.net(joiner), NetScreen::Downloading(..)));
}

#[test]
fn an_invalid_description_fails_the_join() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.host(host, "");
    pipe.act(host, Action::OpenOptions);
    let NetScreen::GameOptions(options) = pipe.net(host) else {
        panic!("no options");
    };
    let mut description = options.description.clone();
    description.parameters.version = 0x866;
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.act(host, Action::Ok);
    pipe.join(joiner, "");
    let NetScreen::LobbyFailed(lobby, f) = pipe.net(joiner) else {
        panic!("not jogf over the lobby");
    };
    assert_eq!(f.reason, JoinFailure::InvalidGame);
    assert!(
        f.text.0.starts_with(b"Game versions don't match."),
        "{:?}",
        f.text
    );
    assert_eq!(lobby.lobby.players.len(), 2, "in the host's lobby");
    assert_eq!(pipe.lobby(host).players.len(), 2);
    pipe.act(joiner, Action::Ok);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    assert_eq!(pipe.lobby(host).players.len(), 1, "left the host's lobby");

    let mut s = Peer::new("Alone", Vec::new()).session;
    s.act(Action::Multiplayer);
    s.act(Action::ChooseOther);
    s.act(Action::Ok);
    s.act(Action::JoinByAddress {
        address: text(ADDRESS),
        password: Text::default(),
    });
    s.game_event(GameEvent::Net(NetEvent::BadDescription));
    assert!(matches!(
        s.screen(),
        Screen::Net(NetScreen::JoinFailed(f)) if f.reason == JoinFailure::InvalidGame
    ));
}

#[test]
fn a_game_in_progress_is_joined_from_its_start_list() {
    let (mut pipe, host, _) = lobby_of_two();
    let late = pipe.add("Late", Vec::new());
    pipe.act(1, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    pipe.join(late, "");
    let Some(start) = pipe.peers[late].loads.last() else {
        panic!("no load: {:?}", pipe.net(late));
    };
    let data = &start.net.as_ref().expect("a network game").new_game_data.0;
    assert_eq!(data[0x22c..0x230], [0, 2, 0xff, 0xff]);
    let host_load = pipe.peers[host].loads.last().unwrap();
    let host_data = &host_load.net.as_ref().unwrap().new_game_data.0;
    assert_eq!(data[..0x22c], host_data[..0x22c]);
    assert_eq!(
        data[0x230..0x230 + 2 * 0x7c],
        host_data[0x230..0x230 + 2 * 0x7c]
    );
}

#[test]
fn only_the_host_starts_and_only_with_everyone_else_ready() {
    let (mut pipe, host, joiner) = lobby_of_two();
    assert!(!pipe.lobby(host).can_start, "the joiner isn't ready");
    pipe.act(host, Action::StartNetGame);
    assert!(pipe.net_requests(host).is_empty());
    assert!(matches!(pipe.net(host), NetScreen::Lobby(_)));

    pipe.act(joiner, Action::SetReady(true));
    assert_eq!(pipe.net_requests(joiner), [NetRequest::Ready(true)]);
    assert!(pipe.lobby(joiner).ready());
    assert!(pipe.lobby(host).player(1).is_some_and(|p| p.ready == 1));
    assert!(pipe.lobby(host).can_start);
    assert!(!pipe.lobby(joiner).can_start, "a joiner never starts");
    pipe.act(joiner, Action::StartNetGame);
    assert!(!started(&pipe, joiner));

    pipe.act(joiner, Action::SetReady(false));
    assert!(!pipe.lobby(host).can_start);
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    assert!(started(&pipe, host));
    assert!(
        pipe.net_requests(host)
            .contains(&NetRequest::Start { vets: None }),
        "the first lobby's start clears 0x40 and has no vets"
    );
}

fn started(pipe: &Pipe, peer: usize) -> bool {
    pipe.net_requests(peer)
        .iter()
        .any(|r| matches!(r, NetRequest::Start { .. }))
}

#[test]
fn the_owner_of_our_servers_game_has_the_hosts_controls() {
    let (mut pipe, _host, joiner) = lobby_of_two();
    assert!(!pipe.lobby(joiner).host_controls);
    pipe.peers[joiner].local.push_back(NetEvent::Owner);
    pipe.pump();
    let lobby = pipe.lobby(joiner);
    assert!(lobby.owner && !lobby.host && lobby.host_controls);
    assert!(lobby.can_start, "player 0 is the server's observer");
    assert!(!mandrake_session::net::may_kick(lobby, 0));
    assert!(!mandrake_session::net::may_kick(lobby, 1));
    pipe.act(joiner, Action::Kick(0));
    assert!(pipe.net_requests(joiner).is_empty());
    pipe.act(joiner, Action::OpenOptions);
    assert!(matches!(pipe.net(joiner), NetScreen::GameOptions(_)));
}

#[test]
fn only_the_host_kicks_and_never_itself() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::Kick(0));
    assert!(pipe.net_requests(joiner).is_empty());
    pipe.act(host, Action::Kick(0));
    pipe.act(host, Action::Kick(7));
    assert!(pipe.net_requests(host).is_empty());
    assert_eq!(pipe.lobby(host).players.len(), 2);

    pipe.act(host, Action::Kick(1));
    assert_eq!(pipe.net_requests(host), [NetRequest::Kick(1)]);
    assert_eq!(pipe.lobby(host).players.len(), 1);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
}

#[test]
fn only_the_host_changes_the_options_and_locks() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::OpenOptions);
    pipe.act(joiner, Action::Lock(true));
    assert!(matches!(pipe.net(joiner), NetScreen::Lobby(l) if !l.locked));
    assert!(pipe.net_requests(joiner).is_empty());

    pipe.act(host, Action::OpenOptions);
    let NetScreen::GameOptions(options) = pipe.net(host) else {
        panic!("no options");
    };
    let mut description = options.description.clone();
    description.parameters.time_limit = 99;
    pipe.act(host, Action::SetGameOptions(Box::new(description.clone())));
    pipe.act(host, Action::Cancel);
    assert_eq!(pipe.lobby(host).description.parameters.time_limit, 1800);
    assert!(pipe.net_requests(host).is_empty());

    pipe.act(host, Action::OpenOptions);
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.act(host, Action::Ok);
    assert!(matches!(
        &pipe.net_requests(host)[..],
        [NetRequest::Password(p), NetRequest::Options(_)] if p.0.is_empty()
    ));
    assert_eq!(pipe.lobby(joiner).description.parameters.time_limit, 99);
    assert_eq!(pipe.lobby(host).description.player_count, 2);

    pipe.act(host, Action::Lock(true));
    assert_eq!(
        pipe.net_requests(host).last(),
        Some(&NetRequest::Lock(true))
    );
}

#[test]
fn the_options_password_is_the_one_a_join_needs_after_ok() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.host(host, "");
    pipe.clear_requests();
    pipe.act(host, Action::OpenOptions);
    pipe.act(host, Action::SetGamePassword(text("dropped")));
    pipe.act(host, Action::Cancel);
    assert!(pipe.net_requests(host).is_empty());
    pipe.act(host, Action::OpenOptions);
    let NetScreen::GameOptions(options) = pipe.net(host) else {
        panic!("no options");
    };
    assert_eq!(options.password, text(""), "Cancel kept the old one");
    pipe.act(host, Action::SetGamePassword(text("secret")));
    pipe.act(host, Action::Ok);
    assert_eq!(pipe.hub.password, b"secret");
    pipe.act(host, Action::OpenOptions);
    let NetScreen::GameOptions(options) = pipe.net(host) else {
        panic!("no options");
    };
    assert_eq!(
        options.password,
        text("secret"),
        "the dialog shows the new one"
    );
    pipe.act(host, Action::Cancel);
    pipe.join(joiner, "");
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::JoinFailed(f) if f.reason == JoinFailure::Denied(text("Wrong password."))
    ));
    pipe.act(joiner, Action::Ok);
    pipe.act(
        joiner,
        Action::JoinByAddress {
            address: text(ADDRESS),
            password: text("secret"),
        },
    );
    pipe.lobby(joiner);
}

#[test]
fn a_private_line_is_kept_as_private() {
    assert_eq!(
        ChatKind::of_mode(mandrake_net::lobby::chat::ALL),
        ChatKind::Line
    );
    assert_eq!(
        ChatKind::of_mode(mandrake_net::lobby::chat::PLAYER),
        ChatKind::Private
    );
    assert_eq!(
        ChatKind::of_mode(mandrake_net::lobby::chat::TEAM),
        ChatKind::Team
    );
    assert_eq!(ChatKind::Team.code(), 4);
}

#[test]
fn lobby_chat_goes_both_ways() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::LobbyChat(text("hello")));
    pipe.act(host, Action::LobbyChat(text("hi")));
    let lines = [
        ChatLine::said(text("Joiner"), text("hello")),
        ChatLine::said(text("Host"), text("hi")),
    ];
    assert!(pipe.lobby(host).chat.lines().eq(&lines));
    assert!(pipe.lobby(joiner).chat.lines().eq(&lines));
}

#[test]
fn the_chat_back_log_keeps_the_last_63_lines() {
    let (mut pipe, host, _joiner) = lobby_of_two();
    for n in 0..100 {
        pipe.act(host, Action::LobbyChat(text(&format!("line {n}"))));
    }
    let log = &pipe.lobby(host).chat;
    assert_eq!(log.added(), 100);
    let kept: Vec<_> = log.lines().map(|l| l.text.clone()).collect();
    let want: Vec<_> = (37..100).map(|n| text(&format!("line {n}"))).collect();
    assert_eq!(kept, want);
    let since: Vec<_> = log.since(98).map(|l| l.text.clone()).collect();
    assert_eq!(since, [text("line 98"), text("line 99")]);
    assert_eq!(log.since(0).count(), 63);
}

#[test]
fn leaving_the_lobby_goes_to_the_main_menu() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::Leave);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    assert_eq!(pipe.net_requests(joiner), [NetRequest::Leave]);
    assert_eq!(pipe.lobby(host).players.len(), 1);
}

#[test]
fn a_lost_host_leaves_the_lobby_for_the_menu_with_no_notice() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(host, Action::Leave);
    assert!(matches!(pipe.screen(host), Screen::MainMenu(_)));
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    assert_eq!(pipe.net_requests(joiner), [NetRequest::Leave]);
}

fn ticks_into_play(pipe: &mut Pipe, host: usize, joiner: usize) {
    for peer in [host, joiner] {
        assert!(matches!(
            pipe.screen(peer),
            Screen::Playing { overlay: None }
        ));
        assert!(pipe.peers[peer].playing);
    }
}

#[test]
fn host_join_ready_start_barrier_play_end_restart_and_leave() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));

    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    let (h, j) = (&pipe.peers[host], &pipe.peers[joiner]);
    for peer in [h, j] {
        assert_eq!(peer.loads.len(), 1);
        let start = &peer.loads[0];
        assert_eq!(start.mesh.0, MESH);
        assert!(start.film.is_none() && start.plugins.is_empty());
        let order: Vec<&Request> = peer
            .requests
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    Request::LoadGame(_) | Request::BeginPlay | Request::Net(NetRequest::Loaded)
                )
            })
            .collect();
        assert!(matches!(
            order[..],
            [
                Request::LoadGame(_),
                Request::Net(NetRequest::Loaded),
                Request::BeginPlay
            ]
        ));
    }
    let hd = h.loads[0].net.as_ref().expect("a network game");
    let jd = j.loads[0].net.as_ref().expect("a network game");
    assert!(hd.host && !jd.host);
    assert_eq!((hd.local_id, jd.local_id), (0, 1));
    let (a, b) = (&hd.new_game_data.0, &jd.new_game_data.0);
    assert_eq!(a.len(), b.len());
    let differ: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    assert_eq!(differ, [0x22f]);
    let description = pipe.hub.description.clone().unwrap();
    assert_eq!(
        pipe.hub.players[0].1.ready, 0,
        "the host's Ready on the wire"
    );
    let players: Vec<PlayerData> = pipe
        .hub
        .players
        .iter()
        .map(|(_, p)| PlayerData {
            ready: 1,
            ..p.clone()
        })
        .collect();
    assert_eq!(
        &new_game_data(&description, &players, 1).unwrap(),
        &jd.new_game_data
    );
    assert_eq!(
        &a[0x240..0x244],
        [0, 0, 0, 1],
        "the host's record (+0x230) is ready"
    );
    assert_ne!(description.parameters.seed, 0, "the start's seed");

    pipe.act(joiner, Action::OpenPause);
    let Screen::Playing {
        overlay: Some(mandrake_session::Overlay::Paused(paused)),
    } = pipe.screen(joiner)
    else {
        panic!("not paused");
    };
    assert!(!paused.can_save && !paused.can_open && !paused.can_revert && !paused.can_restart);
    pipe.act(joiner, Action::Resume);
    pipe.act(joiner, Action::Quicksave);
    assert!(!pipe.peers[joiner].requests.contains(&Request::Quicksave));

    pipe.clear_requests();
    pipe.event(
        joiner,
        GameEvent::MapOver {
            report: report(1),
            veterans: Vec::new(),
        },
    );
    assert_eq!(pipe.net(joiner), NetScreen::EndBarrier { waiting: None });
    assert_eq!(pipe.net_requests(joiner), [NetRequest::MapOver]);
    pipe.event(
        host,
        GameEvent::MapOver {
            report: report(3),
            veterans: Vec::new(),
        },
    );
    let NetScreen::Postgame(end, _) = pipe.net(host) else {
        panic!("not endi");
    };
    assert_eq!(end.report, report(3), "each peer's own statistics");
    assert!(end.can_restart && end.can_chat);
    let NetScreen::Postgame(end, lobby) = pipe.net(joiner) else {
        panic!("not endi");
    };
    assert_eq!(end.report, report(1));
    assert!(!end.can_restart, "Restart Game is the host's");
    assert_eq!(
        lobby.chat.lines().cloned().collect::<Vec<_>>(),
        [
            ChatLine::notice(b"Game Started"),
            ChatLine::notice(b"Game Over")
        ]
    );

    pipe.act(joiner, Action::LobbyChat(text("gg")));
    assert!(matches!(pipe.net(host), NetScreen::Postgame(_, l) if l.chat.len() == 3));
    pipe.act(joiner, Action::SaveReplay { name: text("run") });
    assert_eq!(pipe.peers[joiner].films, [text("Netgame- run")]);
    assert!(matches!(pipe.net(joiner), NetScreen::Postgame(..)));
    pipe.act(joiner, Action::RestartNetGame);
    assert!(matches!(pipe.net(joiner), NetScreen::Postgame(..)));
    pipe.act(host, Action::RestartNetGame);
    for peer in [host, joiner] {
        assert_eq!(pipe.lobby(peer).chat.len(), 3);
    }
    assert_eq!(pipe.peers[joiner].loads.len(), 1);
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    assert_eq!(pipe.peers[joiner].loads.len(), 2);

    for (peer, code) in [(host, 1), (joiner, 3)] {
        pipe.event(
            peer,
            GameEvent::MapOver {
                report: report(code),
                veterans: Vec::new(),
            },
        );
    }
    assert!(matches!(pipe.net(joiner), NetScreen::Postgame(..)));
    pipe.act(joiner, Action::Ok);
    assert_eq!(pipe.lobby(joiner).players.len(), 2);
    pipe.act(host, Action::Leave);
    assert!(matches!(pipe.screen(host), Screen::MainMenu(_)));
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
}

#[test]
fn play_waits_for_the_barrier_after_the_load() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.peers[joiner].hold_load = true;
    pipe.act(host, Action::StartNetGame);
    for peer in [host, joiner] {
        assert_eq!(pipe.screen(peer), Screen::Loading);
        assert!(!pipe.peers[peer].playing);
    }
    assert!(pipe.net_requests(host).contains(&NetRequest::Loaded));
    pipe.peers[joiner].hold_load = false;
    pipe.pump();
    ticks_into_play(&mut pipe, host, joiner);

    let mut s = Peer::new("Alone", Vec::new()).session;
    s.game_event(GameEvent::Net(NetEvent::AllSynced));
    assert!(matches!(s.screen(), Screen::MainMenu(_)));
}

#[test]
fn a_player_dropped_in_play_leaves_the_game_running() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let a = pipe.add("A", Vec::new());
    let b = pipe.add("B", Vec::new());
    pipe.host(host, "");
    pipe.set_map(host, false);
    pipe.join(a, "");
    pipe.join(b, "");
    pipe.act(a, Action::SetReady(true));
    pipe.act(b, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    for peer in [host, a, b] {
        assert!(matches!(
            pipe.screen(peer),
            Screen::Playing { overlay: None }
        ));
    }
    pipe.act(b, Action::OpenPause);
    pipe.act(b, Action::QuitToMenu);
    assert!(matches!(pipe.screen(b), Screen::MainMenu(_)));
    assert!(pipe.net_requests(b).contains(&NetRequest::Leave));
    for peer in [host, a] {
        assert!(matches!(
            pipe.screen(peer),
            Screen::Playing { overlay: None }
        ));
        assert!(pipe.peers[peer].playing);
    }
}

#[test]
fn a_lost_host_in_play_leaves_the_game_stalled_until_quit() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    pipe.peers[host].session.act(Action::OpenPause);
    pipe.act(host, Action::QuitToMenu);
    assert!(matches!(
        pipe.screen(joiner),
        Screen::Playing { overlay: None }
    ));
    assert!(pipe.peers[joiner].playing, "the game stays");
    assert!(!pipe.net_requests(joiner).contains(&NetRequest::Leave));
    pipe.act(joiner, Action::OpenPause);
    pipe.act(joiner, Action::QuitToMenu);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    assert!(!pipe.peers[joiner].playing);
    assert!(pipe.net_requests(joiner).contains(&NetRequest::Leave));
}

fn map_over(pipe: &mut Pipe, peer: usize) {
    pipe.event(
        peer,
        GameEvent::MapOver {
            report: report(3),
            veterans: Vec::new(),
        },
    );
}

#[test]
fn a_host_lost_after_the_end_barrier_ends_at_the_main_menu() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.peers[joiner].local.push_back(NetEvent::AllSynced);
    pipe.peers[joiner].local.push_back(NetEvent::HostLost);
    pipe.pump();
    assert!(matches!(
        pipe.screen(joiner),
        Screen::Playing { overlay: None }
    ));
    map_over(&mut pipe, joiner);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    let requests = pipe.net_requests(joiner);
    assert!(requests.ends_with(&[NetRequest::MapOver, NetRequest::Leave]));
}

#[test]
fn a_host_lost_before_the_end_barrier_ended_opens_endi() {
    for in_barrier in [false, true] {
        let (mut pipe, host, joiner) = lobby_of_two();
        pipe.act(joiner, Action::SetReady(true));
        pipe.act(host, Action::StartNetGame);
        ticks_into_play(&mut pipe, host, joiner);
        if in_barrier {
            map_over(&mut pipe, joiner);
            assert_eq!(pipe.net(joiner), NetScreen::EndBarrier { waiting: None });
        }
        pipe.peers[joiner].local.push_back(NetEvent::HostLost);
        pipe.pump();
        if !in_barrier {
            map_over(&mut pipe, joiner);
        }
        assert!(matches!(pipe.net(joiner), NetScreen::Postgame(..)));
        pipe.act(joiner, Action::Leave);
        assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    }
}

#[test]
fn a_host_lost_in_endi_goes_to_the_main_menu() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    for peer in [host, joiner] {
        map_over(&mut pipe, peer);
    }
    assert!(matches!(pipe.net(joiner), NetScreen::Postgame(..)));
    pipe.act(host, Action::Leave);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    assert_eq!(pipe.net_requests(joiner).last(), Some(&NetRequest::Leave));
}

#[test]
fn a_host_lost_in_the_load_opens_endi_disconnected() {
    for leave in [false, true] {
        let (mut pipe, host, joiner) = lobby_of_two();
        pipe.act(joiner, Action::SetReady(true));
        pipe.peers[joiner].hold_load = true;
        pipe.act(host, Action::StartNetGame);
        assert!(matches!(pipe.screen(joiner), Screen::Loading));
        pipe.peers[joiner]
            .local
            .push_back(NetEvent::Latencies(vec![(0, 0xffff), (1, 22)]));
        pipe.peers[joiner].local.push_back(NetEvent::HostLost);
        pipe.pump();
        assert_eq!(pipe.screen(joiner), Screen::Loading);
        assert!(!pipe.peers[joiner].requests.contains(&Request::DropGame));
        pipe.peers[joiner].hold_load = false;
        pipe.pump();
        let NetScreen::Postgame(end, lobby) = pipe.net(joiner) else {
            panic!("endi");
        };
        assert!(!end.can_chat && !end.can_restart);
        let lines: Vec<_> = lobby.lobby.chat.lines().map(|l| l.text.clone()).collect();
        assert!(lines.ends_with(&[text("Game Started"), text("Game Over")]));
        assert_eq!(end.report, report(0));
        assert_eq!(lobby.latency(0), 0);
        assert_eq!(lobby.latency(1), 0);
        let requests = pipe.net_requests(joiner);
        assert!(!requests.contains(&NetRequest::Loaded));
        let standings = requests.iter().position(|r| *r == NetRequest::Standings);
        let left = requests.iter().position(|r| *r == NetRequest::Leave);
        assert!(standings.is_some() && standings < left);
        let all = &pipe.peers[joiner].requests;
        let asked = all
            .iter()
            .position(|r| *r == Request::Net(NetRequest::Standings));
        let dropped = all.iter().position(|r| *r == Request::DropGame);
        assert!(asked < dropped);
        pipe.act(joiner, if leave { Action::Leave } else { Action::Ok });
        assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    }
}

#[test]
fn the_end_barriers_button_gives_it_up_for_endi() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    map_over(&mut pipe, joiner);
    assert_eq!(pipe.net(joiner), NetScreen::EndBarrier { waiting: None });
    pipe.act(joiner, Action::Cancel);
    assert!(matches!(pipe.net(joiner), NetScreen::Postgame(end, _) if end.can_chat));
}

#[test]
fn the_hosts_end_barrier_names_the_player_it_waits_for() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    map_over(&mut pipe, host);
    assert_eq!(pipe.net(host), NetScreen::EndBarrier { waiting: None });
    let id = pipe.peers[joiner].id.expect("joined");
    pipe.peers[host]
        .local
        .push_back(NetEvent::BarrierWaiting(Some(id)));
    pipe.pump();
    let name = text("Joiner");
    assert_eq!(
        pipe.net(host),
        NetScreen::EndBarrier {
            waiting: Some(&name)
        }
    );
    pipe.peers[host]
        .local
        .push_back(NetEvent::BarrierWaiting(None));
    pipe.pump();
    assert_eq!(pipe.net(host), NetScreen::EndBarrier { waiting: None });
}

#[test]
fn a_failed_load_leaves_the_game() {
    let mut s = Peer::new("Alone", Vec::new()).session;
    s.act(Action::Multiplayer);
    s.act(Action::ChooseOther);
    s.act(Action::Ok);
    s.act(Action::HostGame {
        name: text("A game"),
        password: Text::default(),
        port: 3453,
    });
    s.game_event(GameEvent::Net(NetEvent::Listening));
    s.act(Action::StartNetGame);
    s.game_event(GameEvent::Net(NetEvent::Configuration(CODE_START)));
    assert_eq!(s.screen(), Screen::Loading);
    s.take_requests();
    s.game_event(GameEvent::LoadFailed("no map".to_owned()));
    assert_eq!(
        s.screen(),
        Screen::Net(NetScreen::Notice(NetNotice::LoadFailed))
    );
    assert_eq!(s.take_requests(), [Request::Net(NetRequest::Leave)]);
    s.act(Action::Ok);
    assert!(matches!(s.screen(), Screen::MainMenu(_)));
}

#[test]
fn the_options_describe_each_change_for_their_game_control() {
    let (mut pipe, host, _joiner) = lobby_of_two();
    pipe.act(host, Action::OpenOptions);
    let NetScreen::GameOptions(options) = pipe.net(host) else {
        panic!("no options");
    };
    let mut description = options.description.clone();
    description.parameters.game_type = 0;
    pipe.peers[host].requests.clear();
    pipe.act(host, Action::SetGameOptions(Box::new(description.clone())));
    let described: Vec<_> = pipe.peers[host]
        .requests
        .iter()
        .filter_map(|r| match r {
            Request::DescribeNetGame(g) => Some(g.game_type),
            _ => None,
        })
        .collect();
    assert_eq!(described, [0]);
    pipe.peers[host].requests.clear();
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    assert!(pipe.peers[host].requests.is_empty());

    let game = mandrake_session::net::film_game(&pipe_options(&pipe, host).description);
    pipe.peers[host]
        .session
        .game_event(GameEvent::Net(NetEvent::GameDescribed(Box::new(game))));
    let details = pipe_options(&pipe, host)
        .details
        .clone()
        .expect("described");
    assert_eq!(details.game.game_type, 0);
    pipe.act(host, Action::Ok);
    assert_eq!(pipe.lobby(host).description.parameters.game_type, 0);
}

fn pipe_options(pipe: &Pipe, peer: usize) -> &mandrake_session::net::GameOptions {
    match pipe.net(peer) {
        NetScreen::GameOptions(o) => o,
        other => panic!("not the options: {other:?}"),
    }
}

#[test]
fn an_end_barrier_done_before_the_maps_end_reaches_black_opens_endi() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.event(joiner, GameEvent::Net(NetEvent::AllSynced));
    assert!(matches!(pipe.screen(joiner), Screen::Playing { .. }));
    pipe.event(
        joiner,
        GameEvent::MapOver {
            report: report(1),
            veterans: Vec::new(),
        },
    );
    let NetScreen::Postgame(end, lobby) = pipe.net(joiner) else {
        panic!("not endi");
    };
    assert_eq!(end.report, report(1));
    assert_eq!(
        lobby.chat.lines().last(),
        Some(&ChatLine::notice(b"Game Over"))
    );
}

fn veteran(n: u8) -> MonsterInitializer {
    MonsterInitializer {
        player_id: 1,
        unit_tag: mandrake_common::FourCC(*b"mysm"),
        monster_tag: mandrake_common::FourCC(*b"mysn"),
        experience: n,
        field_8f_plus_one: 1,
        permutation: 0,
        name_index: n,
        name: String::new(),
    }
}

fn forced_end_runs(pipe: &mut Pipe, host: usize, joiner: usize, kind: i16) {
    for peer in [host, joiner] {
        pipe.event(peer, GameEvent::Net(NetEvent::ForcedEnd(kind)));
    }
}

#[test]
fn our_servers_owner_has_rehost_and_restart_in_its_pause_dialog() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.peers[joiner].local.push_back(NetEvent::Owner);
    pipe.pump();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.act(joiner, Action::OpenPause);
    let Screen::Playing {
        overlay: Some(mandrake_session::Overlay::Paused(paused)),
    } = pipe.screen(joiner)
    else {
        panic!("not paused");
    };
    assert!(
        paused
            .network
            .is_some_and(|n| n.originator && n.rehost_items && n.can_rehost)
    );
    pipe.clear_requests();
    pipe.act(joiner, Action::RestartLevel);
    assert_eq!(
        pipe.peers[joiner].requests,
        [
            Request::SetPaused(false),
            Request::Net(NetRequest::ForceEnd(3))
        ]
    );
}

#[test]
fn the_hosts_rehost_game_ends_the_game_on_every_peer_into_endi() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);

    pipe.act(joiner, Action::OpenPause);
    let Screen::Playing {
        overlay: Some(mandrake_session::Overlay::Paused(paused)),
    } = pipe.screen(joiner)
    else {
        panic!("not paused");
    };
    assert!(paused.network.is_some_and(|n| !n.rehost_items));
    pipe.clear_requests();
    pipe.act(joiner, Action::LastSavedGame);
    assert!(pipe.net_requests(joiner).is_empty());
    pipe.act(joiner, Action::Resume);

    pipe.act(host, Action::OpenPause);
    let Screen::Playing {
        overlay: Some(mandrake_session::Overlay::Paused(paused)),
    } = pipe.screen(host)
    else {
        panic!("not paused");
    };
    assert!(
        paused
            .network
            .is_some_and(|n| n.rehost_items && n.can_rehost)
    );
    pipe.clear_requests();
    pipe.act(host, Action::LastSavedGame);
    assert!(matches!(
        pipe.screen(host),
        Screen::Playing { overlay: None }
    ));
    assert_eq!(
        pipe.peers[host].requests,
        [
            Request::SetPaused(false),
            Request::Net(NetRequest::ForceEnd(2))
        ]
    );

    forced_end_runs(&mut pipe, host, joiner, 2);
    for peer in [joiner, host] {
        pipe.event(
            peer,
            GameEvent::MapOver {
                report: report(3),
                veterans: vec![veteran(1)],
            },
        );
    }
    for peer in [host, joiner] {
        let NetScreen::Postgame(end, lobby) = pipe.net(peer) else {
            panic!("not endi");
        };
        assert!(end.can_chat);
        assert_eq!(end.can_restart, peer == host);
        assert_eq!(
            lobby.chat.lines().cloned().collect::<Vec<_>>(),
            [
                ChatLine::notice(b"Game Started"),
                ChatLine::notice(b"Rehosting..."),
                ChatLine::notice(b"Game Over")
            ]
        );
    }
    pipe.act(host, Action::RestartNetGame);
    pipe.clear_requests();
    pipe.act(host, Action::StartNetGame);
    assert_eq!(
        pipe.hub.vets,
        Some(mandrake_rec::writer::initializer_block(&[veteran(1)]))
    );
    let options = pipe.hub.description.as_ref().unwrap().parameters.options;
    assert_ne!(options & OPTION_VETS, 0);
    ticks_into_play(&mut pipe, host, joiner);
}

#[test]
fn the_hosts_restart_game_starts_the_next_game_with_no_endi() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.act(host, Action::OpenPause);
    pipe.clear_requests();
    pipe.act(host, Action::RestartLevel);
    assert_eq!(
        pipe.net_requests(host),
        [NetRequest::ForceEnd(3)],
        "Restart Game, not the solo game's restart"
    );
    assert!(!pipe.peers[host].requests.contains(&Request::DropGame));

    forced_end_runs(&mut pipe, host, joiner, 3);
    pipe.event(
        host,
        GameEvent::MapOver {
            report: report(3),
            veterans: vec![veteran(2)],
        },
    );
    pipe.event(joiner, GameEvent::Net(NetEvent::AllSynced));
    assert!(matches!(pipe.screen(joiner), Screen::Playing { .. }));
    pipe.event(
        joiner,
        GameEvent::MapOver {
            report: report(3),
            veterans: vec![veteran(2)],
        },
    );
    ticks_into_play(&mut pipe, host, joiner);
    for peer in [host, joiner] {
        assert_eq!(pipe.peers[peer].loads.len(), 2);
    }
    assert_eq!(
        pipe.hub.vets,
        Some(mandrake_rec::writer::initializer_block(&[veteran(2)]))
    );
    let NetScreen::Postgame(..) = ({
        for peer in [host, joiner] {
            pipe.event(
                peer,
                GameEvent::MapOver {
                    report: report(1),
                    veterans: Vec::new(),
                },
            );
        }
        pipe.net(joiner)
    }) else {
        panic!("not endi");
    };
    let NetScreen::Postgame(_, lobby) = pipe.net(host) else {
        panic!("not endi");
    };
    assert_eq!(
        lobby.chat.lines().cloned().collect::<Vec<_>>(),
        [
            ChatLine::notice(b"Game Started"),
            ChatLine::notice(b"Restarting..."),
            ChatLine::notice(b"Game Started"),
            ChatLine::notice(b"Game Over")
        ]
    );
}

#[test]
fn endi_shows_the_players_latencies() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.event(
        host,
        GameEvent::Net(NetEvent::Latencies(vec![(0, 0), (1, 32)])),
    );
    for peer in [host, joiner] {
        pipe.event(
            peer,
            GameEvent::MapOver {
                report: report(1),
                veterans: Vec::new(),
            },
        );
    }
    let NetScreen::Postgame(_, lobby) = pipe.net(host) else {
        panic!("not endi");
    };
    assert_eq!(lobby.latency(1), 32);
    assert_eq!(lobby.latency(0), 0);
}

#[test]
fn a_restarts_code_100_before_our_maps_end_reaches_black_starts_the_next_game() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.event(joiner, GameEvent::Net(NetEvent::ForcedEnd(3)));
    pipe.event(joiner, GameEvent::Net(NetEvent::AllSynced));
    pipe.event(joiner, GameEvent::Net(NetEvent::Configuration(CODE_START)));
    assert!(matches!(pipe.screen(joiner), Screen::Playing { .. }));
    pipe.event(
        joiner,
        GameEvent::MapOver {
            report: report(3),
            veterans: Vec::new(),
        },
    );
    assert_eq!(pipe.peers[joiner].loads.len(), 2);
    assert!(matches!(
        pipe.screen(joiner),
        Screen::Loading { .. } | Screen::Playing { .. }
    ));
}

#[test]
fn unit_trading_sends_the_counts_on_ok_or_at_the_planning_times_end() {
    use mandrake_session::screen::{TradeOffer, UnitTrading};
    let trading = || {
        let row = |placed, cost| TradeOffer {
            name: mandrake_session::Text(b"Soulless".to_vec()),
            adjustable: true,
            minimum: 0,
            maximum: 16,
            placed,
            cost,
        };
        Box::new(UnitTrading {
            rows: vec![row(12, 3), row(22, 2)],
            counts: vec![12, 22],
            top: 62,
        })
    };
    let trades = |pipe: &Pipe, peer| -> Vec<Vec<i16>> {
        pipe.net_requests(peer)
            .into_iter()
            .filter_map(|r| match r {
                NetRequest::TradeUnits(c) => Some(c),
                _ => None,
            })
            .collect()
    };
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);

    pipe.event(joiner, GameEvent::Net(NetEvent::UnitTrading(trading())));
    let Screen::Playing {
        overlay: Some(Overlay::UnitTrading(open)),
    } = pipe.screen(joiner)
    else {
        panic!("untr isn't open");
    };
    assert_eq!(open.counts, [12, 22]);
    pipe.act(joiner, Action::SetTradeCounts(vec![10, 25]));
    pipe.act(joiner, Action::SetTradeCounts(vec![1]));
    pipe.act(joiner, Action::Ok);
    assert!(matches!(
        pipe.screen(joiner),
        Screen::Playing { overlay: None }
    ));
    assert_eq!(trades(&pipe, joiner), [vec![10, 25]]);
    pipe.event(joiner, GameEvent::Net(NetEvent::TradingOver));
    assert_eq!(trades(&pipe, joiner).len(), 1);

    pipe.event(host, GameEvent::Net(NetEvent::UnitTrading(trading())));
    pipe.act(host, Action::SetTradeCounts(vec![13, 21]));
    pipe.event(host, GameEvent::Net(NetEvent::TradingOver));
    assert!(matches!(
        pipe.screen(host),
        Screen::Playing { overlay: None }
    ));
    assert_eq!(trades(&pipe, host), [vec![13, 21]]);

    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.event(joiner, GameEvent::Net(NetEvent::UnitTrading(trading())));
    pipe.act(joiner, Action::Cancel);
    pipe.event(joiner, GameEvent::Net(NetEvent::TradingOver));
    assert!(matches!(
        pipe.screen(joiner),
        Screen::Playing { overlay: None }
    ));
    assert!(trades(&pipe, joiner).is_empty());
}

#[test]
fn the_hosts_pause_dialog_after_game_over_has_no_rehost_items() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    let rehost_items = |pipe: &Pipe| {
        let Screen::Playing {
            overlay: Some(mandrake_session::Overlay::Paused(paused)),
        } = pipe.screen(host)
        else {
            panic!("not paused");
        };
        paused.network.is_some_and(|n| n.rehost_items)
    };
    pipe.act(host, Action::OpenPause);
    assert!(rehost_items(&pipe), "before game over");
    pipe.act(host, Action::Resume);
    pipe.event(host, GameEvent::Net(NetEvent::GameOver));
    pipe.act(host, Action::OpenPause);
    assert!(!rehost_items(&pipe), "after game over");
}

use mandrake_common::game_options::{COOPERATIVE as COOP, NARRATION};

fn coop_game(options: u32) -> (Pipe, usize, usize) {
    coop_game_at(options, 0x889)
}

fn coop_game_at(options: u32, version: u32) -> (Pipe, usize, usize) {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.host(host, "");
    pipe.act(host, Action::OpenOptions);
    let NetScreen::GameOptions(o) = pipe.net(host) else {
        panic!("no options");
    };
    let mut description = o.description.clone();
    description.parameters.mesh = common::WILLOW.0;
    description.parameters.options |= options;
    description.parameters.version = version;
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.act(host, Action::Ok);
    pipe.join(joiner, "");
    pipe.lobby(joiner);
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    prologues(&mut pipe, options & NARRATION != 0);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.clear_requests();
    (pipe, host, joiner)
}

fn prologues(pipe: &mut Pipe, shown: bool) {
    for peer in 0..pipe.peers.len() {
        if !shown {
            assert!(!matches!(pipe.screen(peer), Screen::Briefing(_)));
            continue;
        }
        let Screen::Briefing(b) = pipe.screen(peer) else {
            panic!("no prologue: {:?}", pipe.screen(peer));
        };
        assert_eq!(b.title, Text(common::WILLOW.0.to_vec()));
        assert!(!pipe.peers[peer].playing);
        pipe.act(peer, Action::BriefingDone);
    }
}

fn ends(pipe: &mut Pipe, code: i16) {
    for peer in 0..pipe.peers.len() {
        pipe.event(
            peer,
            GameEvent::MapOver {
                report: report(code),
                veterans: Vec::new(),
            },
        );
    }
}

fn epilogue(pipe: &Pipe, peer: usize) -> mandrake_session::PostgameExits {
    let Screen::Postgame(p) = pipe.screen(peer) else {
        panic!("no epilogue: {:?}", pipe.screen(peer));
    };
    p.exits.clone()
}

#[test]
fn a_coop_win_with_narration_shows_the_epilogue_whose_next_level_forces_the_end() {
    let (mut pipe, host, joiner) = coop_game(COOP | NARRATION);
    ends(&mut pipe, common::WIN);
    let h = epilogue(&pipe, host);
    assert!(h.next && h.quit && h.save_film && !h.restart && !h.last_saved);
    let j = epilogue(&pipe, joiner);
    assert!(!j.next && !j.restart && !j.skip && j.quit && j.save_film);
    assert!(pipe.net_requests(host).is_empty(), "no end barrier yet");

    pipe.act(joiner, Action::NextLevel);
    assert!(pipe.net_requests(joiner).is_empty());
    pipe.act(host, Action::NextLevel);
    assert_eq!(pipe.net_requests(host), [NetRequest::ForceEnd(2)]);
    pipe.act(host, Action::NextLevel);
    assert_eq!(pipe.net_requests(host), [NetRequest::ForceEnd(2)], "once");
    assert!(matches!(pipe.screen(host), Screen::Postgame(_)), "it waits");

    for peer in [host, joiner] {
        pipe.event(peer, GameEvent::Net(NetEvent::ForcedEnd(2)));
    }
    assert!(pipe.net_requests(joiner).contains(&NetRequest::MapOver));
    let NetScreen::Postgame(_, lobby) = pipe.net(host) else {
        panic!("not endi");
    };
    assert_eq!(
        lobby.chat.lines().cloned().collect::<Vec<_>>(),
        [
            ChatLine::notice(b"Game Started"),
            ChatLine::notice(b"Rehosting..."),
            ChatLine::notice(b"Game Over")
        ]
    );
    pipe.act(host, Action::RestartNetGame);
    for peer in [host, joiner] {
        let mesh = pipe.lobby(peer).description.parameters.mesh;
        assert_eq!(mesh, common::SALVATION.0, "the next level");
    }
}

#[test]
fn a_coop_loss_with_narration_restarts_from_the_epilogue() {
    let (mut pipe, host, joiner) = coop_game(COOP | NARRATION);
    ends(&mut pipe, common::LOSS);
    let h = epilogue(&pipe, host);
    assert!(h.restart && !h.next && !h.last_saved);
    assert!(!epilogue(&pipe, joiner).restart);
    pipe.act(host, Action::RestartLevel);
    assert_eq!(pipe.net_requests(host), [NetRequest::ForceEnd(3)]);
    for peer in [host, joiner] {
        pipe.event(peer, GameEvent::Net(NetEvent::ForcedEnd(3)));
    }
    ticks_into_play(&mut pipe, host, joiner);
    for peer in [host, joiner] {
        assert_eq!(pipe.peers[peer].loads.len(), 2);
        assert_eq!(pipe.peers[peer].loads[1].mesh, common::WILLOW);
    }
}

#[test]
fn the_epilogues_quit_goes_on_for_a_joiner_and_forces_the_end_for_the_host() {
    let (mut pipe, host, joiner) = coop_game(COOP | NARRATION);
    ends(&mut pipe, common::WIN);
    pipe.act(joiner, Action::QuitToMenu);
    assert_eq!(pipe.net_requests(joiner), [NetRequest::MapOver]);
    assert!(matches!(pipe.net(joiner), NetScreen::EndBarrier { .. }));
    pipe.act(host, Action::QuitToMenu);
    assert_eq!(pipe.net_requests(host), [NetRequest::ForceEnd(2)]);
}

#[test]
fn no_epilogue_after_a_forced_end_or_without_narration() {
    let (mut pipe, host, joiner) = coop_game(COOP | NARRATION);
    forced_end_runs(&mut pipe, host, joiner, 2);
    ends(&mut pipe, 0);
    assert!(matches!(pipe.net(host), NetScreen::Postgame(..)), "endi");

    let (mut pipe, host, joiner) = coop_game(COOP);
    ends(&mut pipe, common::WIN);
    assert!(matches!(pipe.net(joiner), NetScreen::Postgame(..)), "endi");
    pipe.act(host, Action::RestartNetGame);
    assert_eq!(
        pipe.lobby(joiner).description.parameters.mesh,
        common::SALVATION.0
    );
}

#[test]
fn an_old_gameplays_epilogue_takes_its_default_item_with_no_dialog() {
    let (mut pipe, host, joiner) = coop_game_at(COOP | NARRATION, 0x888);
    ends(&mut pipe, common::WIN);
    for peer in [host, joiner] {
        assert!(matches!(pipe.screen(peer), Screen::Loading), "no dialog");
        assert_eq!(pipe.net_requests(peer), [NetRequest::ForceEnd(2)]);
    }
    pipe.clear_requests();
    pipe.act(host, Action::NextLevel);
    assert!(pipe.net_requests(host).is_empty(), "nothing to press");
    forced_end_runs(&mut pipe, host, joiner, 2);
    for peer in [host, joiner] {
        assert!(pipe.net_requests(peer).contains(&NetRequest::MapOver));
    }

    let (mut pipe, host, joiner) = coop_game_at(COOP | NARRATION, 0x888);
    ends(&mut pipe, common::LOSS);
    for peer in [host, joiner] {
        assert_eq!(
            pipe.net_requests(peer),
            [NetRequest::MapOver],
            "no forced end"
        );
        assert!(matches!(
            pipe.net(peer),
            NetScreen::EndBarrier { .. } | NetScreen::Postgame(..)
        ));
    }

    let (mut pipe, host, joiner) = coop_game(COOP | NARRATION);
    ends(&mut pipe, 0);
    for peer in [host, joiner] {
        assert_eq!(
            pipe.net_requests(peer),
            [NetRequest::MapOver],
            "no forced end"
        );
        assert!(!matches!(pipe.screen(peer), Screen::Postgame(_)));
    }
}

#[test]
fn a_forced_end_skips_the_epilogues_dialog_and_a_forced_restart_the_prologue() {
    let (mut pipe, host, joiner) = coop_game(COOP | NARRATION);
    forced_end_runs(&mut pipe, host, joiner, 2);
    ends(&mut pipe, common::WIN);
    for peer in [host, joiner] {
        assert!(pipe.net_requests(peer).contains(&NetRequest::MapOver));
        assert!(!pipe.net_requests(peer).contains(&NetRequest::ForceEnd(2)));
    }
    let (mut pipe, host, joiner) = coop_game(COOP | NARRATION);
    ends(&mut pipe, common::LOSS);
    pipe.act(host, Action::RestartLevel);
    forced_end_runs(&mut pipe, host, joiner, 3);
    prologues(&mut pipe, false);
}

fn replies(pipe: &Pipe, peer: usize) -> Vec<Vec<u8>> {
    let log = match pipe.net(peer) {
        NetScreen::Lobby(l) | NetScreen::Postgame(_, l) => l.replies,
        other => panic!("no chat on {other:?}"),
    };
    log.lines()
        .map(|l| {
            assert_eq!((l.kind, &l.name), (ChatKind::Notice, &Text::default()));
            l.text.0.clone()
        })
        .collect()
}

fn dot(pipe: &mut Pipe, peer: usize, line: &str) -> Vec<Vec<u8>> {
    let before = replies(pipe, peer).len();
    let command = mandrake_session::chat_commands::parse(line.as_bytes()).expect("a dot command");
    pipe.act(peer, Action::ChatCommand(command));
    replies(pipe, peer).split_off(before)
}

fn lines(want: &[&str]) -> Vec<Vec<u8>> {
    want.iter().map(|l| l.as_bytes().to_vec()).collect()
}

#[test]
fn info_shows_the_password_and_the_plugins() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", vec![plugin()]);
    let joiner = pipe.add("Joiner", vec![plugin()]);
    pipe.host(host, "secret");
    pipe.set_map(host, true);
    pipe.join(joiner, "secret");
    let got = dot(&mut pipe, joiner, ".info");
    assert_eq!(got[2], b"Game password has been set.");
    assert_eq!(got[4], b"Active Plugins:");
    assert_eq!(
        got[5],
        [b"  \xc6 ".as_slice(), PLUGIN.as_bytes(), b"\r"].concat()
    );
}

#[test]
fn the_veterans_commands_save_and_load_the_table() {
    use mandrake_net::game_description::OPTION_COOPERATIVE;
    let (mut pipe, host, joiner) = lobby_of_two();
    assert_eq!(
        dot(&mut pipe, host, ".savevets"),
        lines(&["Save Veterans |Kvets|0: No veterans to save."])
    );
    assert_eq!(
        dot(&mut pipe, host, ".loadvets"),
        lines(&[
            "Load Veterans |Kvets|0: You must be the game host of a co-op game to use this command."
        ])
    );
    assert_eq!(
        dot(&mut pipe, joiner, ".loadvets mine"),
        lines(&[
            "Load Veterans |Kmine|0: You must be the game host of a co-op game to use this command."
        ])
    );
    pipe.act(host, Action::OpenOptions);
    let NetScreen::GameOptions(options) = pipe.net(host) else {
        panic!("no options");
    };
    let mut description = options.description.clone();
    description.parameters.options =
        (description.parameters.options | OPTION_COOPERATIVE) & !VETERANS_BOTH;
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.act(host, Action::Ok);
    assert_eq!(
        dot(&mut pipe, host, ".vets"),
        lines(&["Veterans disabled."])
    );
    assert_eq!(
        dot(&mut pipe, host, ".loadvets"),
        lines(&["Load Veterans |Kvets|0: Error reading file."])
    );
    let table = vec![MonsterInitializer {
        player_id: 0,
        unit_tag: mandrake_common::FourCC(*b"mysm"),
        monster_tag: mandrake_common::FourCC(*b"mysn"),
        experience: 3,
        field_8f_plus_one: 1,
        permutation: 0,
        name_index: 2,
        name: String::new(),
    }];
    let block = mandrake_rec::writer::initializer_block(&table);
    pipe.peers[host]
        .veterans_files
        .push((b"vets".to_vec(), block.clone()));
    pipe.clear_requests();
    assert_eq!(
        dot(&mut pipe, host, ".loadvets"),
        lines(&["Load Veterans |Kvets|0: OK."])
    );
    assert!(
        pipe.net_requests(host)
            .iter()
            .any(|r| matches!(r, NetRequest::Description(d)
                if d.parameters.options & VETERANS_BOTH == VETERANS_BOTH))
    );
    assert_eq!(
        pipe.lobby(joiner).description.parameters.options & VETERANS_BOTH,
        VETERANS_BOTH
    );
    assert_eq!(
        dot(&mut pipe, host, ".vets"),
        lines(&["Veterans enabled and present."])
    );
    assert_eq!(
        dot(&mut pipe, host, ".savevets copy"),
        lines(&["Save Veterans |Kcopy|0: OK."])
    );
    let saved = &pipe.peers[host].veterans_files;
    assert_eq!(saved.iter().find(|(n, _)| n == b"copy").unwrap().1, block);
    pipe.clear_requests();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    assert!(
        pipe.net_requests(host)
            .contains(&NetRequest::Start { vets: Some(block) })
    );
}

#[test]
fn endi_takes_the_dot_commands_too() {
    let (mut pipe, host, joiner) = lobby_of_two();
    dot(&mut pipe, joiner, ".host");
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    for peer in [host, joiner] {
        pipe.event(
            peer,
            GameEvent::MapOver {
                report: report(1),
                veterans: Vec::new(),
            },
        );
    }
    assert!(matches!(pipe.net(joiner), NetScreen::Postgame(..)));
    assert_eq!(
        dot(&mut pipe, joiner, ".count").last().unwrap(),
        b"Players in game: 2 / 16"
    );
}

fn direct(peer: usize, pipe: &mut Pipe) {
    pipe.event(
        peer,
        GameEvent::Net(NetEvent::Available(Box::new(Networking {
            settings: settings("Joiner"),
            plugins: Vec::new(),
            foundation_checksum: 0,
            allow_downloads: false,
            server: Some(text(ADDRESS)),
        }))),
    );
}

#[test]
fn the_browser_joins_its_server_from_the_main_menu() {
    let mut pipe = Pipe::default();
    let host = pipe.add("Host", Vec::new());
    let joiner = pipe.add("Joiner", Vec::new());
    pipe.host(host, "");
    direct(joiner, &mut pipe);
    pipe.peers[joiner].session.act(Action::Multiplayer);
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::Joining(j) if j.direct && j.address == text(ADDRESS)
    ));
    pipe.pump();
    assert_eq!(pipe.lobby(joiner).players.len(), 2);
    let players = &pipe.lobby(host).players;
    assert_eq!(players.len(), 2);
    let me = players.iter().find(|p| p.net_id != 0).expect("the joiner");
    assert_eq!(&me.appearance.name[..6], b"Joiner");
    assert_eq!(me.appearance.colors, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(
        pipe.peers[joiner].saved_settings.is_empty(),
        "the player isn't saved again"
    );
}

#[test]
fn the_browsers_failed_join_returns_to_the_main_menu() {
    let mut pipe = Pipe::default();
    let joiner = pipe.add("Joiner", Vec::new());
    direct(joiner, &mut pipe);
    pipe.act(joiner, Action::Multiplayer);
    assert!(matches!(pipe.net(joiner), NetScreen::Joining(j) if j.direct));
    pipe.advance(30_000);
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::JoinFailed(f) if f.reason == JoinFailure::TimedOut
    ));
    pipe.act(joiner, Action::Ok);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
    pipe.act(joiner, Action::Multiplayer);
    assert!(matches!(pipe.net(joiner), NetScreen::Joining(j) if j.direct));
    pipe.act(joiner, Action::Cancel);
    pipe.act(joiner, Action::Ok);
    assert!(matches!(pipe.screen(joiner), Screen::MainMenu(_)));
}

fn coop_save() -> mandrake_session::saves::SaveFolder {
    let game = mandrake_session::films::FilmGame {
        mode: 1,
        game_type: 13,
        options: 0x0080_0089,
        time_limit: -1,
        mesh: mandrake_common::FourCC(*b"01hp"),
        difficulty: 3,
        captain: -1,
        flags: 2,
        build_number: 471,
        version_id: 0x889,
        plugins_present: true,
        map_name: Some(text("The Gate of Storms")),
        version_of_id: 0x73c,
        ..Default::default()
    };
    mandrake_session::saves::SaveFolder {
        saves: vec![mandrake_session::saves::SaveFile {
            path: "/saves/Co-op".into(),
            name: text("Co-op"),
            modified: 0,
            date: text("Nov 21, 2026"),
            game,
            end_tick: 901,
            veterans: Vec::new(),
            snapshot: None,
            subgroup: mandrake_common::FourCC(*b"0001"),
        }],
        any: true,
        new_name: text("Saved Game"),
    }
}

#[test]
fn a_coop_hosts_options_load_a_saved_game_and_ok_plays_it() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(host, Action::OpenOptions);
    pipe.act(host, Action::LoadNetSave);
    assert!(pipe_options(&pipe, host).picker.is_none());
    let mut description = pipe_options(&pipe, host).description.clone();
    description.parameters.options |= game_options::COOPERATIVE;
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.clear_requests();
    pipe.act(host, Action::LoadNetSave);
    assert!(pipe.peers[host].requests.contains(&Request::ListSaves));
    pipe.event(host, GameEvent::SavesListed(coop_save()));
    let picker = pipe_options(&pipe, host)
        .picker
        .clone()
        .expect("the picker");
    assert_eq!(picker.selected, Some(0));
    assert!(picker.can_open);
    pipe.act(host, Action::OpenSave);
    let options = pipe_options(&pipe, host);
    assert!(options.picker.is_none());
    assert_eq!(
        options.saved_game.as_ref().map(|s| s.name.clone()),
        Some(text("Co-op"))
    );
    let p = &options.description.parameters;
    assert_eq!((p.mode, p.mesh, p.difficulty), (1, *b"01hp", 3));
    assert_eq!(p.flags, 3);
    assert_eq!(p.options, 0x89 & !8 | 0x80);
    pipe.clear_requests();
    pipe.act(host, Action::Ok);
    let requests = pipe.net_requests(host);
    assert_eq!(
        requests.first(),
        Some(&NetRequest::NetSave(Some("/saves/Co-op".into())))
    );
    assert!(matches!(requests.last(), Some(NetRequest::Options(d)) if d.parameters.flags & 1 != 0));
    assert!(mandrake_net::netsave::loaded_from_network_save(
        &pipe.lobby(joiner).description.parameters.to_bytes()
    ));
    let game = mandrake_session::net::film_game(&pipe.lobby(joiner).description);
    assert!(game.flags & 1 != 0 && i32::from(game.version_of_id) > game_options::GAMEPLAY_1_8_4);

    pipe.act(host, Action::OpenOptions);
    assert!(pipe_options(&pipe, host).saved_game.is_some());
    pipe.act(host, Action::ClearNetSave);
    assert!(pipe_options(&pipe, host).saved_game.is_none());
    assert_eq!(
        pipe_options(&pipe, host).description.parameters.flags & 1,
        0
    );
    pipe.clear_requests();
    pipe.act(host, Action::Ok);
    assert_eq!(
        pipe.net_requests(host).first(),
        Some(&NetRequest::NetSave(None))
    );
    assert_eq!(pipe.lobby(joiner).description.parameters.flags & 1, 0);

    pipe.act(host, Action::OpenOptions);
    pipe.act(host, Action::LoadNetSave);
    pipe.event(host, GameEvent::SavesListed(coop_save()));
    pipe.act(host, Action::Cancel);
    let options = pipe_options(&pipe, host);
    assert!(options.picker.is_none() && options.saved_game.is_none());
}

#[test]
fn save_game_in_a_network_game_is_the_co_op_hosts_when_the_client_allows_it() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    let can_save = |pipe: &Pipe, peer: usize| match pipe.screen(peer) {
        Screen::Playing {
            overlay: Some(Overlay::Paused(p)),
        } => p.can_save,
        other => panic!("not paused: {other:?}"),
    };
    pipe.act(host, Action::OpenPause);
    assert!(!can_save(&pipe, host));
    pipe.act(host, Action::SaveGame);
    assert!(matches!(
        pipe.screen(host),
        Screen::Playing {
            overlay: Some(Overlay::Paused(_))
        }
    ));
    pipe.act(host, Action::Resume);
    pipe.event(host, GameEvent::Net(NetEvent::SaveAllowed(true)));
    pipe.event(joiner, GameEvent::Net(NetEvent::SaveAllowed(false)));
    pipe.act(host, Action::OpenPause);
    assert!(can_save(&pipe, host));
    pipe.act(joiner, Action::OpenPause);
    assert!(!can_save(&pipe, joiner));
    pipe.clear_requests();
    pipe.act(host, Action::SaveGame);
    assert!(matches!(
        pipe.screen(host),
        Screen::Playing {
            overlay: Some(Overlay::SaveGame(_))
        }
    ));
    assert!(pipe.peers[host].requests.contains(&Request::TakeSnapshot));
}

#[test]
fn a_network_saves_restore_shows_its_progress_until_it_is_restored() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    pipe.event(
        joiner,
        GameEvent::Net(NetEvent::SaveRestoring {
            ticks: 1800,
            end: 3700,
        }),
    );
    match pipe.screen(joiner) {
        Screen::Restoring(r) => {
            assert_eq!(r.name, text("Network Save Game"));
            assert!(r.network);
            assert_eq!(r.fraction, 256 / 3);
        }
        other => panic!("no progress: {other:?}"),
    }
    assert!(pipe.peers[joiner].session.front_end_has_input());
    pipe.event(joiner, GameEvent::Net(NetEvent::SaveRestored));
    assert!(matches!(
        pipe.screen(joiner),
        Screen::Playing { overlay: None }
    ));
}

#[test]
fn coop_next_map_clears_loaded_network_save() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(host, Action::OpenOptions);
    let mut description = pipe_options(&pipe, host).description.clone();
    description.parameters.options |= game_options::COOPERATIVE;
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.act(host, Action::LoadNetSave);
    let mut folder = coop_save();
    folder.saves[0].game.mesh = common::WILLOW;
    pipe.event(host, GameEvent::SavesListed(folder));
    pipe.act(host, Action::OpenSave);
    pipe.act(host, Action::Ok);
    assert_ne!(pipe.lobby(host).description.parameters.flags & 1, 0);

    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    ends(&mut pipe, 1);
    assert!(matches!(pipe.net(host), NetScreen::Postgame(..)));

    pipe.clear_requests();
    pipe.act(host, Action::RestartNetGame);
    assert!(matches!(pipe.net(host), NetScreen::Lobby(_)));
    let host_reqs = pipe.net_requests(host);
    let clear_pos = host_reqs
        .iter()
        .position(|r| matches!(r, NetRequest::NetSave(None)))
        .expect("host must clear NetSave when advancing to next map");
    let desc_pos = host_reqs
        .iter()
        .position(|r| matches!(r, NetRequest::Description(_)))
        .expect("host must broadcast updated description");
    assert!(
        clear_pos < desc_pos,
        "NetSave(None) must precede Description"
    );
    assert_eq!(
        pipe.lobby(host).description.parameters.mesh,
        common::SALVATION.0
    );
    assert_eq!(pipe.lobby(host).description.parameters.flags & 1, 0);
    assert_eq!(pipe.lobby(joiner).description.parameters.flags & 1, 0);

    pipe.act(host, Action::OpenOptions);
    assert!(pipe_options(&pipe, host).saved_game.is_none());
}

#[test]
fn coop_network_plugin_reusing_base_mesh_id_uses_plugin_level() {
    use mandrake_session::catalog::Level;

    const PLUGIN_CSUM: u32 = 0x55aa_1234;
    const PLUGIN_NEXT: mandrake_common::FourCC = mandrake_common::FourCC(*b"px02");

    let mut base_willow = Level::new(common::WILLOW);
    base_willow.title = text("Base Willow");
    base_willow.post_art = Some(mandrake_common::FourCC(*b"01po"));
    base_willow.next = Some(common::SALVATION);

    let mut plugin_willow = Level::new(common::WILLOW);
    plugin_willow.plugin_checksum = PLUGIN_CSUM as i32;
    plugin_willow.title = text("Plugin Willow");
    plugin_willow.post_art = Some(mandrake_common::FourCC(*b"p1po"));
    plugin_willow.next = Some(PLUGIN_NEXT);

    let mut plugin_next = Level::new(PLUGIN_NEXT);
    plugin_next.plugin_checksum = PLUGIN_CSUM as i32;
    plugin_next.title = text("Plugin Next");

    let catalog = mandrake_session::Catalog {
        levels: vec![
            base_willow,
            Level::new(common::SALVATION),
            plugin_willow,
            plugin_next,
        ],
        ..mandrake_session::Catalog::default()
    };

    let mut session =
        mandrake_session::Session::new(catalog, mandrake_session::Progress::default());
    let plugin = Plugin {
        checksum: PLUGIN_CSUM,
        name: b"Coop Plugin".to_vec(),
        url: Vec::new(),
    };
    session.game_event(GameEvent::Net(NetEvent::Available(Box::new(Networking {
        settings: settings("Host"),
        plugins: vec![plugin.clone()],
        foundation_checksum: 0,
        allow_downloads: false,
        server: None,
    }))));
    session.act(Action::Multiplayer);
    session.act(Action::ChooseOther);
    session.act(Action::Ok);
    session.act(Action::HostGame {
        name: text("Coop"),
        password: Text::default(),
        port: 3453,
    });
    session.game_event(GameEvent::Net(NetEvent::Listening));
    session.act(Action::OpenOptions);
    let Screen::Net(NetScreen::GameOptions(o)) = session.screen() else {
        panic!("expected game options");
    };
    let mut description = o.description.clone();
    description.parameters.mesh = common::WILLOW.0;
    description.parameters.options |= game_options::COOPERATIVE;
    description.plugins = mandrake_net::game_description::PluginList(vec![plugin]);
    session.act(Action::SetGameOptions(Box::new(description)));
    session.act(Action::Ok);
    session.game_event(GameEvent::Net(NetEvent::Configuration(
        mandrake_session::net::CODE_START,
    )));
    session.game_event(GameEvent::Loaded);
    session.game_event(GameEvent::Net(NetEvent::AllSynced));
    session.take_requests();

    session.game_event(GameEvent::MapOver {
        report: report(1),
        veterans: Vec::new(),
    });
    let reqs = session.take_requests();
    let saved_progress = reqs
        .iter()
        .find_map(|r| match r {
            Request::SaveProgress(p) => Some(p),
            _ => None,
        })
        .expect("next level visited progress saved");
    assert!(
        saved_progress
            .visited
            .iter()
            .any(|v| v.mesh == PLUGIN_NEXT && v.plugin_checksum == PLUGIN_CSUM as i32),
        "expected plugin next level visited, got {:?}",
        saved_progress.visited
    );

    session.game_event(GameEvent::Net(NetEvent::AllSynced));
    session.game_event(GameEvent::Net(NetEvent::Configuration(
        mandrake_session::net::CODE_START,
    )));
    let Screen::Net(NetScreen::Lobby(lobby)) = session.screen() else {
        panic!("expected lobby, got {:?}", session.screen());
    };
    assert_eq!(lobby.description.parameters.mesh, PLUGIN_NEXT.0);
    assert_eq!(lobby.description.map_name, b"Plugin Next");
}

#[test]
fn missing_plugins_ok_after_reload_refreshes_details_and_checks_version() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(host, Action::OpenOptions);
    let mut description = pipe_options(&pipe, host).description.clone();
    description.plugins = mandrake_net::game_description::PluginList(vec![plugin()]);
    description.parameters.version = 0x500;
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.act(host, Action::Ok);

    assert!(matches!(
        pipe.net(joiner),
        NetScreen::MissingPlugins(_, missing) if missing == [plugin()]
    ));
    pipe.event(joiner, GameEvent::Net(NetEvent::Plugins(vec![plugin()])));
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::MissingPlugins(_, missing) if missing.is_empty()
    ));

    pipe.clear_requests();
    pipe.act(joiner, Action::Ok);
    assert!(
        pipe.peers[joiner]
            .requests
            .iter()
            .any(|r| matches!(r, Request::DescribeNetGame(_))),
        "OK after reload must refresh lobby details"
    );
    assert!(
        matches!(pipe.net(joiner), NetScreen::LobbyFailed(..)),
        "OK after reload must run version check, got {:?}",
        pipe.net(joiner)
    );
}

#[test]
fn host_description_changes_update_missing_plugins_and_trigger_download_after_done() {
    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.set_map(host, true);
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::MissingPlugins(_, missing) if missing == [plugin()]
    ));

    pipe.act(host, Action::OpenOptions);
    let mut description = pipe_options(&pipe, host).description.clone();
    description.plugins.0.retain(|p| p != &plugin());
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.act(host, Action::Ok);
    assert!(
        matches!(pipe.net(joiner), NetScreen::Lobby(_)),
        "expected lobby when host removes missing plugin, got {:?}",
        pipe.net(joiner)
    );

    let plugin_b = Plugin {
        checksum: 0x2222_3333,
        name: b"Other Plugin".to_vec(),
        url: Vec::new(),
    };
    downloading(joiner, &mut pipe, true);
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadsFrom(text(URL))));
    pipe.set_map(host, true);
    assert!(matches!(pipe.net(joiner), NetScreen::Downloading(..)));

    pipe.act(host, Action::OpenOptions);
    let mut description = pipe_options(&pipe, host).description.clone();
    description.plugins = mandrake_net::game_description::PluginList(vec![plugin_b.clone()]);
    pipe.act(host, Action::SetGameOptions(Box::new(description)));
    pipe.act(host, Action::Ok);
    pipe.clear_requests();

    pipe.event(joiner, GameEvent::Net(NetEvent::Plugins(vec![plugin()])));
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadDone(None)));
    assert_eq!(
        pipe.net_requests(joiner),
        [NetRequest::DownloadPlugins {
            url: text(URL),
            plugins: vec![plugin_b.clone()],
        }]
    );
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::Downloading(_, d) if d.plugins == [plugin_b]
    ));
}

#[test]
fn host_lost_during_download_cancels_download_and_owner_updates_postgame_restart() {
    let (mut pipe, host, joiner) = lobby_of_two();
    downloading(joiner, &mut pipe, true);
    pipe.event(joiner, GameEvent::Net(NetEvent::DownloadsFrom(text(URL))));
    pipe.set_map(host, true);
    assert!(matches!(pipe.net(joiner), NetScreen::Downloading(..)));

    pipe.clear_requests();
    pipe.event(joiner, GameEvent::Net(NetEvent::HostLost));
    assert_eq!(
        pipe.net_requests(joiner),
        [NetRequest::CancelDownload, NetRequest::Leave]
    );
    assert!(matches!(
        pipe.net(joiner),
        NetScreen::Notice(NetNotice::HostLost)
    ));

    let (mut pipe, host, joiner) = lobby_of_two();
    pipe.act(joiner, Action::SetReady(true));
    pipe.act(host, Action::StartNetGame);
    ticks_into_play(&mut pipe, host, joiner);
    ends(&mut pipe, 1);
    let NetScreen::Postgame(p, _) = pipe.net(joiner) else {
        panic!("expected postgame");
    };
    assert!(!p.can_restart);
    pipe.event(joiner, GameEvent::Net(NetEvent::Owner));
    let NetScreen::Postgame(p, _) = pipe.net(joiner) else {
        panic!("expected postgame");
    };
    assert!(p.can_restart);
}
