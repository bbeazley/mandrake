use mandrake_common::game_options;
use mandrake_rec::writer::NewGameData;

use crate::client::{Barrier, Client, command_message, drop_player_message_for};
use crate::game_description::{GameDescription, NewGameError};
use crate::game_kind::GameKind;
use crate::lobby::{
    ChatTo, InvalidGame, Lobby, chat, chat_message, check_as, code, configuration_request,
    list_players, pass_on_chat, team_button_value, team_kick, team_lock, team_request,
};
use crate::netsave::SavedGame;
use crate::ours::{CLIENT_MARK, Ours, SERVER_MARK};
use crate::player_data::{JoinRequest, NONE, PlayerData};
use crate::topology::{HOST_NET_ID, JoinState, Player, Topology};
use crate::wire::{
    Body, Chat, Command, Frame, Message, Splitter, TO_HOST, WireError, encode_greeting,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Greeting(u16),
    Joined {
        id: u8,
    },
    Denied {
        reason: Vec<u8>,
    },
    JoinTimedOut,
    Player {
        player: Player,
        new: bool,
    },
    PlayerLeft {
        id: u16,
    },
    Description(GameDescription),
    Configuration {
        flags: u16,
        code: u16,
        value: u32,
    },
    Chat {
        from: u8,
        chat: Chat,
    },
    Vets(Vec<u8>),
    Latencies(Vec<(u16, u16)>),
    BeginSync {
        barrier: u32,
    },
    AllSynced {
        barrier: u32,
    },
    Commands {
        commands: Vec<Command>,
        horizon: i32,
    },
    Ignored(u16),
    Malformed(u16),
    Undecoded(WireError),
    StreamError(WireError),
    HostLost,
    Owner,
    Downloads(Vec<u8>),
    BeginMidgame {
        start: Vec<u8>,
    },
    SavedGame {
        ticks: u32,
    },
}

#[derive(Debug, Clone)]
pub struct Joiner {
    pub(super) splitter: Splitter,
    out: Vec<u8>,
    pub(super) topology: Topology,
    lobby: Lobby,
    client: Client,
    host_udp_port: Option<u16>,
    lost: bool,
    pub(super) server: bool,
    latencies: Vec<(u16, u16)>,
    hello_sent: bool,
    pub(super) deferred_drops: Vec<u8>,
    owner: bool,
    kind: GameKind,
    game_players: Vec<i32>,
    midgame: Option<Vec<u8>>,
    pub(super) saved: Option<SavedGame>,
}

impl Joiner {
    pub fn new(request: JoinRequest, udp_port: u16, now_ms: u64) -> Joiner {
        Joiner::new_as(GameKind::Myth2, request, udp_port, now_ms)
    }

    pub fn new_as(kind: GameKind, mut request: JoinRequest, udp_port: u16, now_ms: u64) -> Joiner {
        if kind == GameKind::Tfl && request.player.field_22 == [0; 6] {
            request.player.field_22 = CLIENT_MARK;
        }
        let (topology, message) =
            Topology::new_waiting(request, now_ms, kind.join_answer_timeout_ms());
        let mut j = Joiner {
            splitter: Splitter::with_greeting(),
            out: encode_greeting(udp_port).to_vec(),
            topology,
            lobby: Lobby::default(),
            client: Client::default(),
            host_udp_port: None,
            lost: false,
            server: false,
            latencies: Vec::new(),
            hello_sent: false,
            deferred_drops: Vec::new(),
            owner: false,
            kind,
            game_players: Vec::new(),
            midgame: None,
            saved: None,
        };
        j.send(&message);
        j
    }

    pub fn kind(&self) -> GameKind {
        self.kind
    }

    pub fn description_check(&self) -> Option<Result<(), InvalidGame>> {
        self.lobby.description().map(|d| check_as(self.kind, d))
    }

    pub fn game_players(&self) -> &[i32] {
        &self.game_players
    }

    pub fn receive(&mut self, bytes: &[u8]) {
        if !self.lost {
            self.splitter.push(bytes);
        }
    }

    pub fn pongs_sent(&mut self, ticks: &[u32]) {
        self.client.pongs_sent(ticks);
    }

    pub fn set_internal_messages(&mut self, messages: Vec<Vec<u8>>) {
        self.topology.set_internal_messages(messages);
    }

    pub fn take_outgoing(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    pub fn connection_lost(&mut self) -> Event {
        self.lost = true;
        self.topology.lost();
        Event::HostLost
    }

    pub fn poll(&mut self, now_ms: u64) -> Vec<Event> {
        let mut events = Vec::new();
        let mut stamped = Vec::new();
        let mut out = Vec::new();
        let mut inner = Vec::new();
        while !self.lost {
            let frame = match self.splitter.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => {
                    flush_commands(&mut stamped, &self.client, &mut events);
                    events.push(Event::StreamError(e));
                    events.push(self.connection_lost());
                    break;
                }
            };
            let bytes = match frame {
                Frame::Greeting(port) => {
                    self.host_udp_port = Some(port);
                    events.push(Event::Greeting(port));
                    continue;
                }
                Frame::Message(bytes) => bytes,
            };
            let message = match Message::decode(&bytes) {
                Ok(m) => self.kind.decoded(m),
                Err(e) => {
                    flush_commands(&mut stamped, &self.client, &mut events);
                    events.push(Event::Undecoded(e));
                    continue;
                }
            };
            inner.clear();
            let before = stamped.len();
            let taken = self.ours_in(&message, &mut inner)
                || self.midgame_in(&message, &mut inner)
                || self.saved_in(&message, &mut inner)
                || self.chat_in(&message, &mut out, &mut inner)
                || self.topology.handle(&message, &mut inner)
                || self.lobby.handle(&message, &mut inner)
                || self
                    .client
                    .handle(&message, &self.topology, &mut stamped, &mut out, &mut inner);
            if !taken {
                inner.push(Event::Ignored(message.body.kind()));
            }
            if let Body::Configuration {
                flags: 0,
                code: code::START,
                ..
            } = message.body
            {
                self.midgame = None;
                self.deferred_drops.clear();
                if self.kind != GameKind::Tfl {
                    self.topology.sort_for_game();
                }
            }
            if self.kind == GameKind::Tfl {
                self.tfl_in(&message, &mut stamped[before..]);
            }
            self.team_request_in(&message, &mut out);
            if let Body::Ping { latencies, .. } = &message.body
                && !self.server
            {
                self.ping_latencies(latencies, &mut inner);
            }
            self.drop_players(&inner, &mut out);
            if !inner.is_empty() {
                flush_commands(&mut stamped, &self.client, &mut events);
                events.append(&mut inner);
            }
        }
        flush_commands(&mut stamped, &self.client, &mut events);
        self.hello(&mut out);
        for m in &out {
            self.send(m);
        }
        self.topology.poll(now_ms, &mut events);
        events
    }

    fn tfl_in(&mut self, message: &Message, stamped: &mut [Command]) {
        if let Body::Configuration {
            flags: 0,
            code: code::START,
            ..
        } = message.body
            && let Some(d) = self.lobby.description()
        {
            let records: Vec<PlayerData> =
                self.topology.players().map(crate::lobby::record).collect();
            self.game_players = crate::tfl::player_order(d, &records);
        }
        for c in stamped {
            c.player = crate::tfl::player_index(&self.game_players, c.player);
        }
    }

    fn midgame_in(&mut self, message: &Message, events: &mut Vec<Event>) -> bool {
        let Body::BeginGameMidgame(raw) = &message.body else {
            return false;
        };
        if self.kind == GameKind::Tfl {
            return false;
        }
        let Some((count, records)) = raw.split_first_chunk::<2>() else {
            events.push(Event::Malformed(message.body.kind()));
            return true;
        };
        let count = u16::from_be_bytes(*count);
        let len = (usize::from(count) * crate::player_data::PLAYER_DATA_SIZE).min(records.len());
        self.midgame = Some(records[..len].to_vec());
        events.push(Event::BeginMidgame {
            start: records[..len].to_vec(),
        });
        true
    }

    fn saved_in(&mut self, message: &Message, events: &mut Vec<Event>) -> bool {
        let Body::SavedGameHeader(raw) = &message.body else {
            return false;
        };
        if self.kind == GameKind::Tfl {
            return false;
        }
        if self.server {
            return true;
        }
        match SavedGame::from_body(raw) {
            Some(save) => {
                events.push(Event::SavedGame { ticks: save.ticks });
                self.saved = Some(save);
            }
            None => events.push(Event::Malformed(message.body.kind())),
        }
        true
    }

    pub fn saved_game(&self) -> Option<&SavedGame> {
        self.saved.as_ref()
    }

    pub fn plays_network_save(&self) -> bool {
        self.saved.is_some()
            && self
                .lobby
                .description()
                .is_some_and(|d| crate::netsave::loaded_from_network_save(&d.parameters.to_bytes()))
    }

    pub fn send_saved_game_progress(&mut self, tick: u32) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        self.send(&Message::new(TO_HOST, id, Body::SavedGameProgress { tick }));
        true
    }

    pub fn joining_mid_game(&self) -> bool {
        self.midgame.is_some()
    }

    pub fn midgame_start(&self) -> Option<&[u8]> {
        self.midgame.as_deref()
    }

    pub fn request_more_commands(&mut self) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        self.send(&Message::new(
            crate::client::HOST_ID,
            id,
            Body::RequestMoreCommands,
        ));
        true
    }

    pub fn server_is_ours(&self) -> bool {
        self.topology
            .players()
            .any(|p| p.id == u16::from(HOST_NET_ID) && p.data.field_22 == SERVER_MARK)
    }

    pub fn is_owner(&self) -> bool {
        self.owner
    }

    fn hello(&mut self, out: &mut Vec<Message>) {
        if self.server || self.hello_sent || !self.server_is_ours() {
            return;
        }
        if let Some(id) = self.local_id() {
            self.hello_sent = true;
            out.push(Message::new(
                TO_HOST,
                id,
                Body::Ours(Ours::Game { kind: self.kind }),
            ));
            out.push(Message::new(
                TO_HOST,
                id,
                Body::Ours(Ours::Hello {
                    version: crate::ours::VERSION,
                }),
            ));
        }
    }

    fn ours_in(&mut self, message: &Message, events: &mut Vec<Event>) -> bool {
        let Body::Ours(o) = &message.body else {
            return false;
        };
        if let Ours::Owner { id } = o
            && !self.owner
            && message.from == HOST_NET_ID
            && self.server_is_ours()
            && self.local_id().map(u16::from) == Some(*id)
        {
            self.owner = true;
            events.push(Event::Owner);
        }
        if let Ours::Downloads(url) = o
            && message.from == HOST_NET_ID
            && self.server_is_ours()
        {
            events.push(Event::Downloads(url.clone()));
        }
        true
    }

    pub fn send_ours(&mut self, ours: Ours) -> bool {
        match self.local_id() {
            Some(id) if self.owner => {
                self.send(&Message::new(TO_HOST, id, Body::Ours(ours)));
                true
            }
            _ => false,
        }
    }

    pub fn owner_configure(&mut self, code: u16, value: u32) -> bool {
        match self.local_id() {
            Some(id) if self.owner => {
                if code == crate::lobby::code::START
                    && self
                        .topology
                        .players()
                        .find(|p| p.id == u16::from(id))
                        .is_some_and(|p| p.data.net_id != i32::from(id))
                    && let Some(m) = self.topology.change_local(|_| {})
                {
                    self.send(&m);
                }
                let mut m = configuration_request(id, code, value);
                m.to = TO_HOST;
                self.send(&m);
                true
            }
            _ => false,
        }
    }

    fn chat_in(&self, message: &Message, out: &mut Vec<Message>, events: &mut Vec<Event>) -> bool {
        let Body::Chat(c) = &message.body else {
            return false;
        };
        if self.client.in_game() {
            return true;
        }
        if c.words[4] & chat::PASS_ON != 0 {
            if let Some(id) = self.local_id() {
                let players: Vec<Player> = self.topology.players().cloned().collect();
                out.extend(pass_on_chat(id, message.from, c, &players));
            }
        } else {
            events.push(Event::Chat {
                from: message.from,
                chat: c.clone(),
            });
        }
        true
    }

    fn team_request_in(&self, message: &Message, out: &mut Vec<Message>) {
        let Body::Configuration {
            flags: 1,
            code: code::SEAT,
            value,
        } = message.body
        else {
            return;
        };
        if !self.server || self.client.in_game() {
            return;
        }
        let target = value as i32;
        let teams = self
            .lobby
            .description()
            .is_some_and(|d| d.parameters.options & game_options::TEAMS != 0);
        if target != NONE && !teams {
            return;
        }
        let players: Vec<Player> = self.topology.players().cloned().collect();
        for r in team_request(&players, u16::from(message.from), target) {
            out.extend(self.topology.record_message(&r));
        }
    }

    fn send_records(&mut self, records: Vec<PlayerData>) -> bool {
        if self.local_id().is_none() {
            return false;
        }
        for r in records {
            if let Some(m) = self.topology.record_message(&r) {
                self.send(&m);
            }
        }
        true
    }

    pub fn team_button(&mut self, chosen: Option<i32>) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        match team_button_value(&self.players(), u16::from(id), chosen) {
            Some(value) => self.request_configuration(code::SEAT, value as u32),
            None => false,
        }
    }

    pub fn team_kick(&mut self, target: u16) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        let records = team_kick(&self.players(), u16::from(id), target);
        self.send_records(records)
    }

    pub fn team_lock(&mut self) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        let records: Vec<PlayerData> = team_lock(&self.players(), u16::from(id))
            .into_iter()
            .collect();
        self.send_records(records)
    }

    fn ping_latencies(&mut self, latencies: &[i16], events: &mut Vec<Event>) {
        let ids = self.topology.list_order().to_vec();
        let by_id = ids
            .into_iter()
            .zip(latencies.iter().map(|&l| l as u16))
            .collect();
        self.set_latencies(by_id, events);
    }

    pub(super) fn set_latencies(&mut self, latencies: Vec<(u16, u16)>, events: &mut Vec<Event>) {
        if latencies != self.latencies {
            self.latencies = latencies.clone();
            events.push(Event::Latencies(latencies));
        }
    }

    pub fn latencies(&self) -> &[(u16, u16)] {
        &self.latencies
    }

    fn drop_players(&mut self, events: &[Event], out: &mut Vec<Message>) {
        let Some(local) = self.local_id() else {
            return;
        };
        if !self.server {
            return;
        }
        for e in events {
            match e {
                Event::PlayerLeft { id } if self.in_pregame_barrier() => {
                    if let Ok(net_id) = u8::try_from(*id) {
                        self.deferred_drops.push(net_id);
                    }
                }
                Event::PlayerLeft { id }
                    if self.client.in_game() && self.client.arrived().contains(id) =>
                {
                    if let Ok(net_id) = u8::try_from(*id) {
                        out.push(drop_player_message_for(
                            self.kind,
                            local,
                            net_id,
                            self.horizon(),
                        ));
                    }
                }
                Event::AllSynced { barrier } if !barrier.is_multiple_of(2) => {
                    for id in std::mem::take(&mut self.deferred_drops) {
                        out.push(drop_player_message_for(
                            self.kind,
                            local,
                            id,
                            self.horizon(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    fn in_pregame_barrier(&self) -> bool {
        self.client.in_game()
            && !self.client.barriers_begun().is_multiple_of(2)
            && matches!(self.client.barrier(), Barrier::Waiting { .. })
    }

    pub fn chat(&mut self, to: ChatTo, text: &[u8]) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        if self.client.in_game() {
            return false;
        }
        let players: Vec<Player> = self.topology.players().cloned().collect();
        let name = players
            .iter()
            .find(|p| p.id == u16::from(id))
            .map_or(self.local_record().appearance.name, |p| {
                p.data.appearance.name
            });
        let color = list_players(&players)
            .into_iter()
            .find(|r| r.id == u16::from(id))
            .map_or_else(
                || {
                    let c = self.local_record().appearance.colors;
                    [c[0], c[1], c[2], c[3]]
                },
                |r| r.colors[0],
            );
        self.send(&chat_message(id, &name, color, to, text));
        true
    }

    pub fn in_game(&self) -> bool {
        self.client.in_game()
    }

    pub(super) fn send(&mut self, message: &Message) {
        if let Ok(bytes) = self.kind.encode(message) {
            self.out.extend_from_slice(&bytes);
        }
    }

    pub fn join_state(&self) -> JoinState {
        self.topology.state()
    }

    pub fn local_id(&self) -> Option<u8> {
        self.topology.local_id()
    }

    pub fn host_udp_port(&self) -> Option<u16> {
        self.host_udp_port
    }

    pub fn players(&self) -> Vec<Player> {
        self.topology.players().cloned().collect()
    }

    pub fn description(&self) -> Option<&GameDescription> {
        self.lobby.description()
    }

    pub fn lobby(&self) -> &Lobby {
        &self.lobby
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn horizon(&self) -> i32 {
        self.client.horizon()
    }

    pub fn local_record(&self) -> &PlayerData {
        self.topology.local_record()
    }

    pub fn set_ready(&mut self, ready: bool) -> bool {
        match self.topology.change_local(|p| p.ready = u32::from(ready)) {
            Some(m) => {
                self.send(&m);
                true
            }
            None => false,
        }
    }

    pub fn request_configuration(&mut self, code: u16, value: u32) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        self.send(&configuration_request(id, code, value));
        true
    }

    pub fn loaded(&mut self) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        match self.client.answer_barrier(id) {
            Some(m) => {
                self.send(&m);
                true
            }
            None => false,
        }
    }

    pub fn send_command(&mut self, kind: u8, payload: Vec<u8>) -> bool {
        let Some(id) = self.local_id() else {
            return false;
        };
        if self.kind == GameKind::Tfl && !self.kind.commands().has(kind) {
            return false;
        }
        self.send(&command_message(id, kind, payload));
        true
    }

    pub fn new_game_data(&self) -> Option<Result<NewGameData, NewGameError>> {
        let local = self.local_id()?;
        if self.plays_network_save()
            && let Some(save) = &self.saved
        {
            let mut g = save.new_game_data();
            if self.midgame.is_some() {
                let at = crate::game_description::LOCAL_PLAYER;
                g.0[at..at + 2].copy_from_slice(&0xffffu16.to_be_bytes());
            }
            return Some(Ok(g));
        }
        if let Some(start) = &self.midgame {
            let lobby: Vec<PlayerData> = self.topology.players().map(|p| p.data.clone()).collect();
            return Some(Ok(crate::game_description::midgame_new_game_data(
                self.lobby.description()?,
                start,
                &lobby,
            )));
        }
        self.lobby.new_game_data(&self.players(), local)
    }

    pub fn lobby_new_game_data(&self) -> Option<Result<NewGameData, NewGameError>> {
        self.lobby.new_game_data(&self.players(), self.local_id()?)
    }

    pub fn tfl_new_game(&self) -> Option<Result<mandrake_rec::tfl::TflNewGame, NewGameError>> {
        self.lobby.tfl_new_game(&self.players(), self.local_id()?)
    }

    pub fn new_map(&mut self) {
        self.client.reset_horizon();
    }
}

fn flush_commands(stamped: &mut Vec<Command>, client: &Client, events: &mut Vec<Event>) {
    if !stamped.is_empty() {
        events.push(Event::Commands {
            commands: std::mem::take(stamped),
            horizon: client.horizon(),
        });
    }
}
