use std::collections::{BTreeMap, BTreeSet};

use mandrake_common::game_options;
use mandrake_rec::writer::NewGameData;

use super::joiner::{Event, Joiner};
use crate::game_description::{GameDescription, NewGameError};
use crate::game_kind::GameKind;
use crate::lobby::{
    BarrierKind, ChatTo, FLAG_NO_TEAM_CHANGE, HostLobby, boot_unready, chat_message,
    configuration_broadcast, host_options_records, leave_every_team, restart_records,
};
use crate::midgame::{self, Catchup, Midgame};
use crate::netsave::{Progress, SavedGame};
use crate::ours::{CLIENT_MARK, Ours, SERVER_MARK};
use crate::player_data::{JoinRequest, PlayerData};
use crate::server::Server;
use crate::topology::{Denial, HOST_NET_ID, HostTopology, NET_LAYER_SENDER, Outgoing, Player};
use crate::wire::{
    Body, Chat, Command, Frame, Message, Splitter, TO_ALL, TO_HOST, WireError, encode_greeting,
};

pub const JOIN_REQUEST_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    Accepted { id: u8 },
    Refused,
    Joined { id: u8, name: Vec<u8> },
    Denied { id: u8, denial: Denial },
    PlayerChanged { id: u8 },
    PlayerLeft { id: u8, joined: bool, arrived: bool },
    ConfigurationRequest { from: u8, code: u16, value: u32 },
    Arrived { id: u8 },
    CaughtUp { id: u8, tick: i32 },
    Ours { from: u8, ours: Ours },
    BarrierDone { kind: BarrierKind },
    ServerStarted,
    ServerStopped,
    Chat { from: u8, chat: Chat },
    RecordRefused { from: u8, id: u16 },
    NotRelayed { from: u8, to: u8, kind: u16 },
    Undecoded { id: u8, error: WireError },
    StreamError { id: u8, error: WireError },
}

#[derive(Debug, Clone)]
pub struct HostOptions {
    pub player: JoinRequest,
    pub password: Vec<u8>,
    pub description: GameDescription,
}

const MAX_UNDECODED_PER_PEER: usize = 32;

#[derive(Debug, Clone)]
struct Peer {
    splitter: Splitter,
    out: Vec<u8>,
    udp_port: Option<u16>,
    arrived_ms: Option<u64>,
    accepted_ms: Option<u64>,
    undecoded: usize,
    closing: bool,
}

impl Peer {
    fn new(greeting: Option<u16>) -> Peer {
        Peer {
            splitter: Splitter::with_greeting(),
            out: greeting
                .map(|p| encode_greeting(p).to_vec())
                .unwrap_or_default(),
            udp_port: None,
            arrived_ms: None,
            accepted_ms: None,
            undecoded: 0,
            closing: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Host {
    topology: HostTopology,
    lobby: HostLobby,
    server: Server,
    peers: BTreeMap<u8, Peer>,
    local: Joiner,
    udp_port: u16,
    events: Vec<HostEvent>,
    messages: Vec<Vec<u8>>,
    advertising: bool,
    after_game_ids: Option<Vec<u8>>,
    dedicated: bool,
    closed: bool,
    midgame: Midgame,
    saved: Option<SavedGame>,
    progress: Progress,
    server_hidden_from: BTreeSet<u8>,
}

fn about_server(m: &Message) -> bool {
    let host = u16::from(HOST_NET_ID);
    match &m.body {
        Body::NewPlayer { id, .. } | Body::PlayerDeleted { id } => *id == host,
        Body::PlayerSyncData { id, .. } => *id == host,
        _ => false,
    }
}

const DESTINATION_AT: usize = 4;

fn dedicated_refuses(m: &Message) -> bool {
    let to_server_player = m.to == HOST_NET_ID;
    match &m.body {
        Body::Command(_) | Body::BeginSyncData => true,
        Body::PlayerSyncData { .. } => !to_server_player,
        Body::Configuration { flags, .. } => !to_server_player || flags & 1 == 0,
        _ => false,
    }
}

impl Host {
    pub fn new(options: HostOptions, udp_port: u16, now_ms: u64) -> Host {
        let HostOptions {
            mut player,
            password,
            description,
        } = options;
        let max = description.parameters.max_players.max(1) as usize;
        let dedicated = player.player.field_22 == SERVER_MARK;
        player.password = password.clone();
        if description.parameters.options & game_options::HOST_OBSERVER != 0 {
            player.player.flags |= FLAG_NO_TEAM_CHANGE;
        } else {
            player.player.flags &= !FLAG_NO_TEAM_CHANGE;
        }
        let kind = GameKind::of(&description);
        let mut local = Joiner::new_as(kind, player, udp_port, now_ms);
        local.server = true;
        local.receive(&encode_greeting(udp_port));
        let mut peers = BTreeMap::new();
        peers.insert(HOST_NET_ID, Peer::new(None));
        Host {
            topology: HostTopology::new(max, password),
            lobby: HostLobby::new(description),
            server: Server::new(kind),
            peers,
            local,
            udp_port,
            events: Vec::new(),
            messages: Vec::new(),
            advertising: true,
            after_game_ids: None,
            dedicated,
            closed: false,
            midgame: Midgame::default(),
            saved: None,
            progress: Progress::default(),
            server_hidden_from: BTreeSet::new(),
        }
    }

    pub fn is_dedicated(&self) -> bool {
        self.dedicated
    }

    fn hides_server_from(&self, id: u8) -> bool {
        self.dedicated
            && self.kind() == GameKind::Tfl
            && id != HOST_NET_ID
            && self
                .topology
                .player(id)
                .is_some_and(|p| p.data.field_22 != CLIENT_MARK)
    }

    pub fn server_hidden_from(&self, id: u8) -> bool {
        self.server_hidden_from.contains(&id)
    }

    pub fn advertising(&self) -> bool {
        self.advertising
    }

    pub fn lan_answer(&self, at: std::net::SocketAddrV4) -> Option<crate::lan::Answer> {
        self.advertising.then(|| {
            crate::lan::Answer::new(*at.ip(), at.port(), &self.lobby.description().game_name)
        })
    }

    pub fn set_internal_messages(&mut self, messages: Vec<Vec<u8>>) {
        self.local.set_internal_messages(messages.clone());
        self.messages = messages;
    }

    pub fn accept(&mut self) -> Option<u8> {
        match self.topology.accept() {
            Some(id) => {
                self.peers.insert(id, Peer::new(Some(self.udp_port)));
                self.events.push(HostEvent::Accepted { id });
                Some(id)
            }
            None => {
                self.events.push(HostEvent::Refused);
                None
            }
        }
    }

    pub fn receive(&mut self, id: u8, bytes: &[u8]) {
        if id != HOST_NET_ID
            && let Some(p) = self.peers.get_mut(&id)
            && !p.closing
        {
            p.splitter.push(bytes);
        }
    }

    pub fn receive_at(&mut self, id: u8, bytes: &[u8], arrived_ms: u64) {
        self.receive(id, bytes);
        if id != HOST_NET_ID
            && let Some(p) = self.peers.get_mut(&id)
            && !p.closing
        {
            p.arrived_ms = Some(arrived_ms);
        }
    }

    pub fn take_outgoing(&mut self, id: u8) -> Vec<u8> {
        match self.peers.get_mut(&id) {
            Some(p) if id != HOST_NET_ID => std::mem::take(&mut p.out),
            _ => Vec::new(),
        }
    }

    pub fn has_connection(&self, id: u8) -> bool {
        id != HOST_NET_ID && self.peers.get(&id).is_some_and(|p| !p.closing)
    }

    pub fn connection_lost(&mut self, id: u8) {
        if id == HOST_NET_ID {
            return;
        }
        if let Some(p) = self.peers.get_mut(&id)
            && p.splitter.pending() > 0
        {
            p.closing = true;
            return;
        }
        self.remove(id);
    }

    pub fn take_events(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn poll(&mut self, now_ms: u64) -> Vec<Event> {
        let up = self.local.take_outgoing();
        if let Some(p) = self.peers.get_mut(&HOST_NET_ID) {
            p.splitter.push(&up);
        }
        let ids: Vec<u8> = self.peers.keys().copied().collect();
        for id in ids {
            self.read_peer(id, now_ms);
            if let Some(p) = self.peers.get_mut(&id) {
                p.arrived_ms = None;
            }
        }
        let closing: Vec<u8> = self
            .peers
            .iter()
            .filter_map(|(&id, p)| p.closing.then_some(id))
            .collect();
        for id in closing {
            self.remove(id);
        }
        self.drop_unjoined(now_ms);
        self.barrier_after_leaving(now_ms);
        if let Some(c) = self.server.idle(now_ms) {
            self.send_command(c);
        }
        self.ping(now_ms);
        self.stream_history(now_ms);
        let down = self
            .peers
            .get_mut(&HOST_NET_ID)
            .map(|p| std::mem::take(&mut p.out))
            .unwrap_or_default();
        self.local.receive(&down);
        let mut events = self.local.poll(now_ms);
        let latencies = self
            .topology
            .joined_ids()
            .into_iter()
            .filter_map(|id| Some((u16::from(id), self.server.latency(id)?)))
            .collect();
        self.local.set_latencies(latencies, &mut events);
        events
    }

    fn drop_unjoined(&mut self, now_ms: u64) {
        let mut late = Vec::new();
        for (&id, p) in self.peers.iter_mut() {
            if id == HOST_NET_ID || self.topology.player(id).is_some() {
                continue;
            }
            let since = *p.accepted_ms.get_or_insert(now_ms);
            if now_ms.saturating_sub(since) >= JOIN_REQUEST_TIMEOUT_MS {
                late.push(id);
            }
        }
        for id in late {
            self.remove(id);
        }
    }

    fn read_peer(&mut self, id: u8, now_ms: u64) {
        loop {
            let Some(peer) = self.peers.get_mut(&id) else {
                return;
            };
            let bytes = match peer.splitter.next_frame() {
                Ok(Some(Frame::Greeting(port))) => {
                    peer.udp_port = Some(port);
                    continue;
                }
                Ok(Some(Frame::Message(bytes))) => bytes,
                Ok(None) => return,
                Err(error) => {
                    self.events.push(HostEvent::StreamError { id, error });
                    if id != HOST_NET_ID {
                        self.remove(id);
                    }
                    return;
                }
            };
            let kind = self.kind();
            match Message::decode(&bytes).map(|m| kind.decoded(m)) {
                Ok(mut m) => {
                    m.from = id;
                    self.route(id, m, now_ms);
                }
                Err(error) => {
                    self.events.push(HostEvent::Undecoded { id, error });
                    if id != HOST_NET_ID
                        && let Some(peer) = self.peers.get_mut(&id)
                    {
                        peer.undecoded += 1;
                        if peer.undecoded >= MAX_UNDECODED_PER_PEER {
                            self.remove(id);
                            return;
                        }
                    }
                }
            }
        }
    }

    fn route(&mut self, from: u8, m: Message, now_ms: u64) {
        if self.dedicated && from != HOST_NET_ID && m.to != TO_HOST && dedicated_refuses(&m) {
            if self.topology.player(from).is_some() {
                self.events.push(HostEvent::NotRelayed {
                    from,
                    to: m.to,
                    kind: m.body.kind(),
                });
            }
            return;
        }
        if m.to != TO_HOST && self.topology.player(from).is_none() {
            return;
        }
        match m.to {
            TO_HOST => self.for_host(from, m, now_ms),
            TO_ALL => self.broadcast(&m),
            HOST_NET_ID => match &m.body {
                Body::PlayerSyncData { value, .. } => self.sync_data(from, *value, now_ms),
                Body::RequestMoreCommands => self.more_commands(from),
                Body::Chat(chat) => {
                    if from != HOST_NET_ID {
                        self.events.push(HostEvent::Chat {
                            from,
                            chat: chat.clone(),
                        });
                    }
                    self.queue(HOST_NET_ID, &m);
                }
                _ => self.queue(HOST_NET_ID, &m),
            },
            to => {
                if self.topology.player(to).is_some() {
                    self.queue(to, &m);
                }
            }
        }
    }

    fn for_host(&mut self, from: u8, m: Message, now_ms: u64) {
        let joined = self.topology.player(from).is_some();
        match m.body {
            Body::JoinRequest { .. } if !joined => self.handle_join_request(from, &m.body),
            Body::PlayerDataChanged { id, player_data } if joined => {
                let own = id == u16::from(from);
                let changed = match u8::try_from(id) {
                    Ok(net_id) if self.dedicated && from != HOST_NET_ID && !own => self
                        .topology
                        .team_ended(from, net_id, &player_data)
                        .map(|out| (net_id, out)),
                    Ok(net_id) => self
                        .topology
                        .player_changed(net_id, &player_data)
                        .map(|out| (net_id, out)),
                    Err(_) => None,
                };
                if let Some((net_id, out)) = changed {
                    self.send_all(out);
                    self.events.push(HostEvent::PlayerChanged { id: net_id });
                } else if self.dedicated && from != HOST_NET_ID && !own {
                    self.events.push(HostEvent::RecordRefused { from, id });
                }
            }
            Body::Command(c) if joined => {
                let c = self.server.stamp(from, c, now_ms);
                self.send_command(c);
            }
            Body::ServerCommand(0) if from == HOST_NET_ID => {
                self.server.start(now_ms);
                self.events.push(HostEvent::ServerStarted);
            }
            Body::ServerCommand(1) if from == HOST_NET_ID => {
                self.server.stop();
                self.events.push(HostEvent::ServerStopped);
            }
            Body::Pong { sdl_ticks } if joined && from != HOST_NET_ID => {
                let arrived = self.peers.get(&from).and_then(|p| p.arrived_ms);
                self.server
                    .pong(from, sdl_ticks, arrived.map_or(now_ms, |at| at.min(now_ms)))
            }
            Body::Configuration { code, value, .. } if joined => self
                .events
                .push(HostEvent::ConfigurationRequest { from, code, value }),
            Body::PlayerSyncData { value, .. } if joined => self.sync_data(from, value, now_ms),
            Body::SavedGameProgress { tick } if joined => self.progress.set(from, tick),
            Body::Ours(ours) if joined && from != HOST_NET_ID => {
                self.events.push(HostEvent::Ours { from, ours })
            }
            _ => {}
        }
    }

    fn handle_join_request(&mut self, from: u8, body: &Body) {
        let in_play = self.in_play();
        let gameplay = self.midgame.gameplay;
        let not_arrived = self
            .topology
            .joined_ids()
            .into_iter()
            .filter(|id| !self.lobby.arrived().contains(id))
            .count();
        let sim_players = self.midgame.sim_players();
        let checked = self.topology.join_request_with(from, body, |record| {
            if !in_play {
                return Ok(());
            }
            midgame::check_join(record, gameplay, not_arrived, sim_players).map_err(Denial::Midgame)
        });
        match checked {
            Ok(out) => {
                if self.hides_server_from(from) {
                    self.server_hidden_from.insert(from);
                }
                self.send_all(out);
                let name = self
                    .topology
                    .player(from)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.events.push(HostEvent::Joined { id: from, name });
                if in_play {
                    self.begin_midgame(from);
                } else {
                    let d = self.lobby.description_message();
                    self.broadcast(&d);
                    self.broadcast_saved_game();
                }
            }
            Err(denial) => {
                let body = denial.body_with(&self.messages);
                self.queue(from, &Message::new(from, NET_LAYER_SENDER, body));
                self.events.push(HostEvent::Denied { id: from, denial });
            }
        }
    }

    fn sync_data(&mut self, from: u8, value: u16, now_ms: u64) {
        if self.topology.player(from).is_none() {
            return;
        }
        if value & 1 != 0 && self.server.is_running() && self.midgame.start.is_some() {
            self.midgame_sync_data(from);
            return;
        }
        self.events.push(HostEvent::Arrived { id: from });
        let ids = self.topology.joined_ids();
        let done = self.lobby.sync_data(from, &ids);
        self.barrier_done(done, now_ms);
    }

    fn barrier_after_leaving(&mut self, now_ms: u64) {
        let ids = self.topology.joined_ids();
        let done = self.lobby.barrier_done(&ids);
        self.barrier_done(done, now_ms);
    }

    fn barrier_done(&mut self, done: Option<(BarrierKind, Vec<Message>)>, now_ms: u64) {
        if let Some((kind, entries)) = done {
            for e in &entries {
                self.broadcast(e);
            }
            self.events.push(HostEvent::BarrierDone { kind });
            if kind == BarrierKind::Start {
                self.server.start(now_ms);
                self.events.push(HostEvent::ServerStarted);
                self.enable_midgame_joining();
            }
        }
    }

    fn send_command(&mut self, c: Command) {
        let Ok(bytes) = Message::new(TO_ALL, HOST_NET_ID, Body::Command(c)).encode() else {
            return;
        };
        let arrived: Vec<u8> = self.lobby.arrived().iter().copied().collect();
        for to in arrived {
            if self.topology.player(to).is_some() {
                self.queue_encoded(to, &bytes);
            }
        }
    }

    fn queue_encoded(&mut self, to: u8, bytes: &[u8]) {
        if let Some(p) = self.peers.get_mut(&to) {
            let start = p.out.len();
            p.out.extend_from_slice(bytes);
            p.out[start + DESTINATION_AT] = to;
        }
    }

    fn ping(&mut self, now_ms: u64) {
        let ids = self.table_order();
        let Some((sdl_ticks, latencies)) = self.server.ping(now_ms, &ids, HOST_NET_ID) else {
            return;
        };
        let Ok(bytes) = Message::new(
            TO_ALL,
            HOST_NET_ID,
            Body::Ping {
                sdl_ticks,
                latencies,
            },
        )
        .encode() else {
            return;
        };
        for to in ids {
            let version = self.topology.player(to).map_or(0, |p| p.data.version);
            if to != HOST_NET_ID && version > game_options::VERSION_ID_NO_PINGS {
                self.queue_encoded(to, &bytes);
            }
        }
    }

    fn remove(&mut self, id: u8) {
        if self.peers.remove(&id).is_none() {
            return;
        }
        self.server_hidden_from.remove(&id);
        let arrived = self.lobby.arrived().contains(&id);
        self.lobby.remove(id);
        self.midgame.joining.remove(&id);
        let (player, out) = self.topology.remove(id);
        self.send_all(out);
        self.events.push(HostEvent::PlayerLeft {
            id,
            joined: player.is_some(),
            arrived,
        });
    }

    fn queue(&mut self, to: u8, m: &Message) {
        if self.server_hidden_from.contains(&to) && about_server(m) {
            return;
        }
        let bytes = self.kind().encode(m);
        if let (Some(p), Ok(bytes)) = (self.peers.get_mut(&to), bytes) {
            p.out.extend_from_slice(&bytes);
        }
    }

    fn send_all(&mut self, out: Vec<Outgoing>) {
        for (to, m) in out {
            self.queue(to, &m);
        }
    }

    pub(super) fn broadcast(&mut self, m: &Message) {
        let Ok(bytes) = self.kind().encode(m) else {
            return;
        };
        let hidden = about_server(m);
        for to in self.topology.joined_ids() {
            if hidden && self.server_hidden_from.contains(&to) {
                continue;
            }
            if let Some(p) = self.peers.get_mut(&to) {
                p.out.extend_from_slice(&bytes);
            }
        }
    }

    pub fn local(&self) -> &Joiner {
        &self.local
    }

    pub fn local_mut(&mut self) -> &mut Joiner {
        &mut self.local
    }

    pub fn topology(&self) -> &HostTopology {
        &self.topology
    }

    pub fn lobby(&self) -> &HostLobby {
        &self.lobby
    }

    pub fn take_round_trips(&mut self) -> Vec<(u8, u32, u32, u16)> {
        self.server.take_round_trips()
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    pub fn kind(&self) -> GameKind {
        self.server.kind()
    }

    pub fn tick(&self, now_ms: u64) -> i32 {
        self.server.tick(now_ms)
    }

    pub fn set_description(&mut self, description: GameDescription) {
        self.topology
            .set_max_players(description.parameters.max_players.max(1) as usize);
        self.lobby.set_description(description);
        let d = self.lobby.description_message();
        self.broadcast(&d);
        self.broadcast_saved_game();
    }

    pub fn send_description_to(&mut self, to: u8) {
        let mut d = self.lobby.description_message();
        d.to = to;
        self.queue(to, &d);
        if let Some(save) = &self.saved {
            let m = save.message(HOST_NET_ID);
            self.queue(to, &m);
        }
    }

    pub fn send_ours(&mut self, to: u8, ours: Ours) {
        if to != HOST_NET_ID {
            self.queue(to, &Message::new(to, HOST_NET_ID, Body::Ours(ours)));
        }
    }

    pub fn restart_game(&mut self) {
        if self.kind() == GameKind::Tfl {
            self.after_game_ids = Some(self.topology.joined_ids());
        }
        self.clear_ready();
        self.configure(crate::lobby::code::START, 0);
        self.lobby_opened();
    }

    fn lobby_opened(&mut self) {
        if self.lobby.kind() == GameKind::Tfl {
            return;
        }
        let d = self.lobby.reopen();
        self.broadcast(&d);
        let Some(me) = self.topology.player(HOST_NET_ID) else {
            return;
        };
        let mut record = me.data.clone();
        record.net_id = i32::from(HOST_NET_ID);
        if self.lobby.description().parameters.options & game_options::HOST_OBSERVER != 0 {
            record.flags |= FLAG_NO_TEAM_CHANGE;
        } else {
            record.flags &= !FLAG_NO_TEAM_CHANGE;
        }
        self.host_records(vec![record]);
    }

    pub fn clear_ready(&mut self) {
        let players = self.table_players();
        self.host_records(restart_records(&players));
    }

    fn table_order(&self) -> Vec<u8> {
        let joined = self.topology.joined_ids();
        let mut ids: Vec<u8> = self
            .local
            .topology
            .list_order()
            .iter()
            .filter_map(|&id| u8::try_from(id).ok())
            .filter(|id| joined.contains(id))
            .collect();
        for id in joined {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    fn table_players(&self) -> Vec<Player> {
        self.table_order()
            .into_iter()
            .filter_map(|id| self.topology.player(id).cloned())
            .collect()
    }

    fn host_records(&mut self, records: Vec<PlayerData>) {
        for r in records {
            let id = r.net_id as u8;
            if let Some(out) = self.topology.player_changed(id, &r.to_bytes()) {
                self.send_all(out);
                self.events.push(HostEvent::PlayerChanged { id });
            }
        }
    }

    pub fn set_options(&mut self, description: GameDescription) {
        let old = self.lobby.description().parameters.options;
        let new = description.parameters.options;
        let players = self.table_players();
        self.host_records(host_options_records(
            &players,
            u16::from(HOST_NET_ID),
            old,
            new,
        ));
        self.set_description(description);
        self.clear_ready();
        if new & game_options::TEAMS == 0 {
            let players = self.table_players();
            self.host_records(leave_every_team(&players));
        }
    }

    pub fn configure(&mut self, code: u16, value: u32) {
        if code == crate::lobby::code::START && !self.server.is_running() {
            self.advertising = true;
            self.topology.set_accepting(true);
        }
        self.broadcast(&configuration_broadcast(code, value));
    }

    pub fn set_password(&mut self, password: Vec<u8>) {
        self.topology.set_password(password);
    }

    pub fn set_accepting(&mut self, accepting: bool) {
        self.topology.set_accepting(accepting);
        self.closed = !accepting;
    }

    pub fn start(&mut self, machine_ticks: u32) {
        if let Some(v) = self.lobby.start_vets() {
            self.broadcast(&v);
        }
        let [description, start] = self.lobby.start(machine_ticks);
        self.broadcast(&description);
        self.broadcast_saved_game();
        self.broadcast(&start);
        self.server.new_game();
        self.progress.clear();
        if self.saved.is_some() {
            self.server.set_tick_override(Some(0));
        }
        self.midgame = self.start_midgame();
        let keep_history = self.midgame.start.is_some()
            && !self.closed
            && midgame::reopens(&self.lobby.description().parameters, self.midgame.gameplay);
        self.server.set_keep_history(keep_history);
        self.topology.set_accepting(false);
        self.advertising = false;
    }

    fn broadcast_saved_game(&mut self) {
        if let Some(save) = &self.saved {
            let m = save.message(HOST_NET_ID);
            self.broadcast(&m);
        }
    }

    pub fn set_saved_game(&mut self, saved: Option<SavedGame>) {
        let mut d = self.lobby.description().clone();
        if saved.is_some() {
            d.parameters.flags |= 1;
        } else {
            d.parameters.flags &= !1;
        }
        self.local.saved = saved.clone();
        self.saved = saved;
        self.set_description(d);
    }

    pub fn saved_game(&self) -> Option<&SavedGame> {
        self.saved.as_ref()
    }

    pub fn all_players_reached_game_time(&self, tick: i64) -> bool {
        self.progress
            .all_reached(&self.topology.joined_ids(), HOST_NET_ID, tick)
    }

    pub fn saved_game_progress(&self, id: u8) -> u32 {
        self.progress.of(id)
    }

    pub fn send_spoofed(&mut self, command: Command, tick: u32, now_ms: u64) {
        self.server.set_tick_override(Some(tick));
        let c = self.server.send_own(command, now_ms);
        self.send_command(c);
    }

    pub fn set_tick_override(&mut self, tick: Option<u32>) {
        self.server.set_tick_override(tick);
    }

    pub fn resume_clock_at(&mut self, tick: u32, now_ms: u64) {
        self.server.resume_at(tick, now_ms);
    }

    pub fn begin_barrier(&mut self, kind: BarrierKind) {
        if kind == BarrierKind::End && self.server.is_running() {
            self.server.stop();
            self.events.push(HostEvent::ServerStopped);
        }
        if kind == BarrierKind::End && self.midgame.start.is_some() {
            self.topology.set_accepting(false);
            self.advertising = false;
        }
        let m = self.lobby.begin_barrier(kind);
        self.broadcast(&m);
    }

    fn start_records(&self) -> Vec<PlayerData> {
        self.topology
            .joined()
            .map(|p| {
                let mut r = p.data.clone();
                r.ready = 1;
                r
            })
            .collect()
    }

    fn start_midgame(&self) -> Midgame {
        if self.lobby.kind() == GameKind::Tfl {
            return Midgame::default();
        }
        let Some(local) = self.local.local_id() else {
            return Midgame::default();
        };
        let Ok(start) = crate::game_description::new_game_data(
            self.lobby.description(),
            &self.start_records(),
            i32::from(local),
        ) else {
            return Midgame::default();
        };
        let gameplay = midgame::game_gameplay(
            &self.lobby.description().parameters,
            &midgame::start_records(&start),
        );
        Midgame {
            start: Some(start),
            gameplay,
            ..Midgame::default()
        }
    }

    fn in_play(&self) -> bool {
        self.server.is_running() && self.midgame.start.is_some()
    }

    fn enable_midgame_joining(&mut self) {
        if self.midgame.start.is_none() || self.closed {
            return;
        }
        if midgame::reopens(&self.lobby.description().parameters, self.midgame.gameplay) {
            self.topology.set_accepting(true);
            self.advertising = true;
        }
    }

    fn begin_midgame(&mut self, id: u8) {
        if let Some(vets) = self.lobby.kept_vets() {
            let m = Message::new(id, HOST_NET_ID, Body::Vets(vets.to_vec()));
            self.queue(id, &m);
        }
        let mut d = self.lobby.description_message();
        d.to = id;
        self.queue(id, &d);
        if let Some(save) = &self.saved {
            let mut m = save.message(HOST_NET_ID);
            m.to = id;
            self.queue(id, &m);
        }
        if let Some(start) = &self.midgame.start {
            let m = midgame::begin_midgame(id, HOST_NET_ID, start);
            self.queue(id, &m);
        }
        self.queue(id, &Message::new(id, HOST_NET_ID, Body::BeginSyncData));
        self.midgame.joining.insert(id, None);
    }

    fn midgame_sync_data(&mut self, from: u8) {
        if self.lobby.arrived().contains(&from) {
            self.remove(from);
            return;
        }
        for id in self.table_order() {
            let m = Message::new(
                from,
                HOST_NET_ID,
                Body::PlayerSyncData {
                    id: u16::from(id),
                    value: 0,
                },
            );
            self.queue(from, &m);
        }
        let cursor = (!self.server.history().is_empty()).then_some(0);
        self.midgame.joining.insert(
            from,
            Some(Catchup {
                cursor,
                wants_more: true,
            }),
        );
    }

    fn more_commands(&mut self, from: u8) {
        if let Some(Some(c)) = self.midgame.joining.get_mut(&from)
            && c.cursor.is_some()
        {
            c.wants_more = true;
        }
    }

    fn stream_history(&mut self, now_ms: u64) {
        if !self.in_play() {
            return;
        }
        let order = self.table_order();
        for id in order {
            let Some(Some(c)) = self.midgame.joining.get(&id).copied() else {
                continue;
            };
            let (Some(cursor), true) = (c.cursor, c.wants_more) else {
                continue;
            };
            let (out, cursor) = midgame::batch(self.server.history(), cursor, id, HOST_NET_ID);
            for m in &out {
                self.queue(id, m);
            }
            self.midgame.joining.insert(
                id,
                Some(Catchup {
                    cursor,
                    wants_more: false,
                }),
            );
            if cursor.is_some() {
                continue;
            }
            self.midgame.joining.remove(&id);
            self.lobby.mark_arrived(id);
            let Some(record) = self.topology.player(id).map(|p| p.data.clone()) else {
                continue;
            };
            let options = self.lobby.description().parameters.options;
            let c = self
                .server
                .send_own(midgame::add_player_command(id, &record, options), now_ms);
            let tick = c.tick;
            self.send_command(c);
            self.midgame.added += 1;
            self.events.push(HostEvent::CaughtUp { id, tick });
        }
    }

    pub fn catching_up(&self) -> Vec<u8> {
        self.midgame.joining.keys().copied().collect()
    }

    fn preview_lobby(&self, machine_ticks: u32) -> Option<(HostLobby, i32)> {
        let local = self.local.local_id()?;
        let mut lobby = self.lobby.clone();
        let _ = lobby.start_vets();
        let _ = lobby.start(machine_ticks);
        Some((lobby, i32::from(local)))
    }

    pub fn start_preview(&self, machine_ticks: u32) -> Option<Result<NewGameData, NewGameError>> {
        let (lobby, local) = self.preview_lobby(machine_ticks)?;
        Some(crate::game_description::new_game_data(
            lobby.description(),
            &self.start_records(),
            local,
        ))
    }

    pub fn tfl_start_preview(
        &self,
        machine_ticks: u32,
    ) -> Option<Result<mandrake_rec::tfl::TflNewGame, NewGameError>> {
        let (lobby, local) = self.preview_lobby(machine_ticks)?;
        Some(crate::tfl::new_game(
            lobby.description(),
            &self.start_records(),
            local,
        ))
    }

    pub fn set_vets(&mut self, initializers: Option<Vec<u8>>) {
        self.lobby.set_vets(initializers);
    }

    pub fn set_after_game(&mut self, after_game: bool) {
        let changed = self
            .after_game_ids
            .as_ref()
            .is_some_and(|ids| *ids != self.topology.joined_ids());
        let after_game = after_game && !(self.kind() == GameKind::Tfl && changed);
        let mut d = self.lobby.description().clone();
        if after_game {
            d.parameters.options |= game_options::CARRY_VETERANS;
        } else {
            d.parameters.options &= !game_options::CARRY_VETERANS;
        }
        self.lobby.set_description(d);
    }

    pub fn chat(&mut self, to: ChatTo, text: &[u8]) -> bool {
        self.local.chat(to, text)
    }

    pub fn chat_as(&mut self, name: &[u8], text: &[u8]) {
        self.chat_as_to(name, ChatTo::All, text);
    }

    pub fn chat_as_to(&mut self, name: &[u8], to: ChatTo, text: &[u8]) {
        let mut n = [0u8; 32];
        let len = name.len().min(31);
        n[..len].copy_from_slice(&name[..len]);
        self.local
            .send(&chat_message(HOST_NET_ID, &n, [0; 4], to, text));
    }

    pub fn kick(&mut self, id: u8) {
        if id != HOST_NET_ID {
            self.remove(id);
        }
    }

    pub fn boot(&mut self, id: u8) {
        let players = self.local.players();
        let lowered = boot_unready(&players, u16::from(HOST_NET_ID), i32::from(id));
        self.local.topology.lower_ready(&lowered);
        self.kick(id);
    }
}
