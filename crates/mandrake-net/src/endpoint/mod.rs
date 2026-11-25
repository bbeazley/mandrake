mod host;
mod joiner;

pub use host::{Host, HostEvent, HostOptions, JOIN_REQUEST_TIMEOUT_MS};
pub use joiner::{Event, Joiner};

use std::collections::BTreeMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use crate::game_kind::GameKind;
use crate::player_data::JoinRequest;
use crate::transport::{
    ByteStream, CONNECT_TIMEOUT, Connection, Listener, Notify, PongTimes, Waker,
};

#[derive(Debug)]
pub struct Endpoint {
    role: Role,
    notify: Arc<Notify>,
}

#[derive(Debug)]
enum Role {
    Join {
        connection: Connection,
        joiner: Box<Joiner>,
        host_lost: bool,
    },
    Host {
        listener: Listener,
        host: Box<Host>,
        connections: BTreeMap<u8, Connection>,
    },
}

impl Endpoint {
    pub fn join(host: SocketAddr, request: JoinRequest, now_ms: u64) -> io::Result<Endpoint> {
        Endpoint::join_as(GameKind::Myth2, host, request, now_ms)
    }

    pub fn join_as(
        kind: GameKind,
        host: SocketAddr,
        request: JoinRequest,
        now_ms: u64,
    ) -> io::Result<Endpoint> {
        let mut connection = Connection::connect(host, CONNECT_TIMEOUT)?;
        let notify = Arc::new(Notify::default());
        connection.set_notify(notify.clone());
        let joiner = Box::new(Joiner::new_as(kind, request, connection.udp_port(), now_ms));
        let mut e = Endpoint {
            role: Role::Join {
                connection,
                joiner,
                host_lost: false,
            },
            notify,
        };
        e.flush();
        Ok(e)
    }

    pub fn join_over(stream: Box<dyn ByteStream>, request: JoinRequest, now_ms: u64) -> Endpoint {
        Endpoint::join_over_as(GameKind::Myth2, stream, request, now_ms)
    }

    pub fn join_over_as(
        kind: GameKind,
        stream: Box<dyn ByteStream>,
        request: JoinRequest,
        now_ms: u64,
    ) -> Endpoint {
        let connection = Connection::over(stream);
        let joiner = Box::new(Joiner::new_as(kind, request, connection.udp_port(), now_ms));
        let mut e = Endpoint {
            role: Role::Join {
                connection,
                joiner,
                host_lost: false,
            },
            notify: Arc::new(Notify::default()),
        };
        e.flush();
        e
    }

    pub fn host(port: u16, options: HostOptions, now_ms: u64) -> io::Result<Endpoint> {
        Endpoint::host_at(
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
            options,
            now_ms,
        )
    }

    pub fn host_at(addr: SocketAddr, options: HostOptions, now_ms: u64) -> io::Result<Endpoint> {
        let listener = Listener::bind(addr)?;
        let host = Box::new(Host::new(options, listener.udp_port(), now_ms));
        Ok(Endpoint {
            role: Role::Host {
                listener,
                host,
                connections: BTreeMap::new(),
            },
            notify: Arc::new(Notify::default()),
        })
    }

    pub fn wait(&self, timeout: std::time::Duration) -> bool {
        self.notify.wait(timeout)
    }

    pub fn set_waker(&self, waker: Option<Waker>) {
        self.notify.set_waker(waker);
    }

    pub fn take_pong_times(&mut self) -> PongTimes {
        match &mut self.role {
            Role::Join { connection, .. } => connection.take_pong_times(),
            Role::Host { .. } => PongTimes::default(),
        }
    }

    pub fn poll(&mut self, now_ms: u64) -> Vec<Event> {
        let notify = &self.notify;
        let mut events = match &mut self.role {
            Role::Join {
                connection,
                joiner,
                host_lost,
            } => {
                let mut bytes = Vec::new();
                let (_, answered) = connection.read(&mut bytes);
                joiner.receive(&bytes);
                joiner.pongs_sent(&answered);
                let events = joiner.poll(now_ms);
                for e in &events {
                    match e {
                        Event::Greeting(port) => connection.set_peer_udp_port(*port),
                        Event::StreamError(_) => {
                            connection.close();
                            *host_lost = true;
                        }
                        _ => {}
                    }
                }
                connection.set_pong_from(joiner.local_id());
                events
            }
            Role::Host {
                listener,
                host,
                connections,
            } => {
                for stream in listener.accept_all() {
                    if let Some(id) = host.accept() {
                        match Connection::accepted(stream, listener.udp_port()) {
                            Ok(mut c) => {
                                c.set_notify(notify.clone());
                                connections.insert(id, c);
                            }
                            Err(_) => host.connection_lost(id),
                        }
                    }
                }
                let answer = listener.udp_addr().and_then(|at| host.lan_answer(at));
                listener.answer_queries(answer.as_ref());
                for (id, c) in connections.iter_mut() {
                    let mut bytes = Vec::new();
                    match c.read(&mut bytes).0 {
                        Some(at) => {
                            let ago = u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX);
                            host.receive_at(*id, &bytes, now_ms.saturating_sub(ago));
                        }
                        None => host.receive(*id, &bytes),
                    }
                }
                for (id, c) in connections.iter() {
                    if c.is_lost() {
                        host.connection_lost(*id);
                    }
                }
                host.poll(now_ms)
            }
        };
        self.flush();
        if let Role::Join {
            connection,
            joiner,
            host_lost,
        } = &mut self.role
            && connection.is_lost()
            && !*host_lost
        {
            *host_lost = true;
            events.push(joiner.connection_lost());
        }
        events
    }

    pub fn flush(&mut self) {
        match &mut self.role {
            Role::Join {
                connection, joiner, ..
            } => {
                let out = joiner.take_outgoing();
                if !out.is_empty() {
                    connection.queue(&out);
                }
                connection.flush();
            }
            Role::Host {
                host, connections, ..
            } => {
                for (id, c) in connections.iter_mut() {
                    let out = host.take_outgoing(*id);
                    if !out.is_empty() {
                        c.queue(&out);
                    }
                    c.flush();
                    if c.is_lost() {
                        host.connection_lost(*id);
                    }
                }
                connections.retain(|id, _| host.has_connection(*id));
            }
        }
    }

    pub fn joiner(&self) -> &Joiner {
        match &self.role {
            Role::Join { joiner, .. } => joiner,
            Role::Host { host, .. } => host.local(),
        }
    }

    pub fn joiner_mut(&mut self) -> &mut Joiner {
        match &mut self.role {
            Role::Join { joiner, .. } => joiner,
            Role::Host { host, .. } => host.local_mut(),
        }
    }

    pub fn set_internal_messages(&mut self, messages: Vec<Vec<u8>>) {
        match &mut self.role {
            Role::Join { joiner, .. } => joiner.set_internal_messages(messages),
            Role::Host { host, .. } => host.set_internal_messages(messages),
        }
    }

    pub fn as_host(&self) -> Option<&Host> {
        match &self.role {
            Role::Host { host, .. } => Some(host),
            Role::Join { .. } => None,
        }
    }

    pub fn as_host_mut(&mut self) -> Option<&mut Host> {
        match &mut self.role {
            Role::Host { host, .. } => Some(host),
            Role::Join { .. } => None,
        }
    }

    pub fn listen_addr(&self) -> Option<SocketAddr> {
        match &self.role {
            Role::Host { listener, .. } => listener.local_addr().ok(),
            Role::Join { .. } => None,
        }
    }

    pub fn leave(mut self) {
        self.flush();
        match &mut self.role {
            Role::Join { connection, .. } => connection.close(),
            Role::Host { connections, .. } => {
                for c in connections.values_mut() {
                    c.close();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mandrake_common::game_options;

    use super::*;
    use crate::game_description::{GameDescription, GameParameters, PARAMETERS_SIZE, PluginList};
    use crate::lobby::{ChatTo, FLAG_NO_TEAM_CHANGE};
    use crate::ours::SERVER_MARK;
    use crate::player_data::PlayerData;
    use crate::topology::HOST_NET_ID;
    use crate::wire::{Body, Frame, Message, Splitter, TO_ALL, encode_greeting};

    fn description(options: u32) -> GameDescription {
        let mut parameters = GameParameters::read(&[0; PARAMETERS_SIZE]);
        parameters.options = options;
        parameters.max_players = 4;
        parameters.mesh = *b"wabe";
        GameDescription {
            parameters,
            plugins: PluginList::default(),
            foundation_checksum: 0,
            field_230: 0,
            flags: 0,
            player_count: 0,
            game_name: b"Server".to_vec(),
            map_name: Vec::new(),
        }
    }

    fn host(options: u32) -> Host {
        let player = JoinRequest {
            name: b"Server".to_vec(),
            password: Vec::new(),
            player: PlayerData::new(b"Server", b"Team", [0; 8]),
        };
        let mut h = Host::new(
            HostOptions {
                player,
                password: Vec::new(),
                description: description(options),
            },
            0xa635,
            0,
        );
        h.poll(0);
        h
    }

    #[test]
    fn server_is_observer_flags_the_hosts_own_record() {
        let h = host(game_options::HOST_OBSERVER);
        assert_eq!(h.local().local_id(), Some(0));
        let own = h.topology().joined().find(|p| p.id == 0).unwrap();
        assert_ne!(own.data.flags & FLAG_NO_TEAM_CHANGE, 0);
        let h = host(0);
        let own = h.topology().joined().find(|p| p.id == 0).unwrap();
        assert_eq!(own.data.flags & FLAG_NO_TEAM_CHANGE, 0);
    }

    #[test]
    fn a_line_in_another_name_reaches_a_joiner() {
        let mut h = host(game_options::HOST_OBSERVER);
        let id = h.accept().unwrap();
        let mut j = Joiner::new(
            JoinRequest {
                name: b"Joiner".to_vec(),
                password: Vec::new(),
                player: PlayerData::new(b"Joiner", b"Team", [0; 8]),
            },
            0x1000,
            0,
        );
        h.receive(id, &j.take_outgoing());
        h.poll(10);
        j.receive(&h.take_outgoing(id));
        j.poll(10);
        h.chat_as(b"Autohost", b"Game will begin in 20 seconds...");
        h.poll(20);
        h.poll(30);
        j.receive(&h.take_outgoing(id));
        let line = j
            .poll(30)
            .into_iter()
            .find_map(|e| match e {
                Event::Chat { chat, .. } => Some(chat),
                _ => None,
            })
            .expect("the line");
        assert_eq!(&line.name[..9], b"Autohost\0");
        assert_eq!(line.text, b"Game will begin in 20 seconds...");
        assert_eq!(line.words[..3], [0, 0, 0]);
    }

    fn tfl_host() -> Host {
        let player = JoinRequest {
            name: b"Host".to_vec(),
            password: Vec::new(),
            player: PlayerData::new(b"Host", b"Team", [0; 8]),
        };
        let mut d = description(game_options::TEAMS);
        d.parameters.version = crate::game_kind::TFL_GAMEPLAY_VERSION;
        let mut h = Host::new(
            HostOptions {
                player,
                password: Vec::new(),
                description: d,
            },
            0xa635,
            0,
        );
        h.poll(0);
        h
    }

    fn tfl_join(h: &mut Host, now: u64) -> (u8, Joiner) {
        let id = h.accept().unwrap();
        let mut j = Joiner::new_as(
            GameKind::Tfl,
            JoinRequest {
                name: b"Joiner".to_vec(),
                password: Vec::new(),
                player: PlayerData::new(b"Joiner", b"Team", [0; 8]),
            },
            0x1000,
            now,
        );
        h.receive(id, &j.take_outgoing());
        h.poll(now + 10);
        j.receive(&h.take_outgoing(id));
        j.poll(now + 10);
        (id, j)
    }

    #[test]
    fn a_tfl_lobby_after_a_game_drops_0x40_when_its_players_change() {
        let options = |h: &Host| h.lobby().description().parameters.options;
        let mut h = tfl_host();
        assert_eq!(h.kind(), GameKind::Tfl);
        h.restart_game();
        h.set_after_game(true);
        assert_ne!(options(&h) & game_options::CARRY_VETERANS, 0);
        let _joiner = tfl_join(&mut h, 100);
        h.set_after_game(true);
        assert_eq!(options(&h) & game_options::CARRY_VETERANS, 0);
        h.restart_game();
        h.set_after_game(true);
        assert_ne!(options(&h) & game_options::CARRY_VETERANS, 0);
    }

    #[test]
    fn tfl_chat_reaches_a_tfl_joiner_as_typed() {
        let mut h = tfl_host();
        let (id, mut j) = tfl_join(&mut h, 0);
        assert!(j.chat(ChatTo::All, b"Hello. This is"));
        h.receive(id, &j.take_outgoing());
        h.poll(20);
        h.poll(30);
        j.receive(&h.take_outgoing(id));
        let line = j
            .poll(30)
            .into_iter()
            .find_map(|e| match e {
                Event::Chat { chat, .. } => Some(chat),
                _ => None,
            })
            .expect("the line");
        assert_eq!(line.text, b"Hello. This is");
    }

    #[test]
    fn a_tfl_server_keeps_its_own_player_from_tfl_1_5_joiners() {
        let server = |version: u32| {
            let mut player = PlayerData::new(b"Server", b"Server", [0; 8]);
            player.field_22 = SERVER_MARK;
            let mut d = description(game_options::TEAMS);
            d.parameters.version = version;
            let mut h = Host::new(
                HostOptions {
                    player: JoinRequest {
                        name: b"Server".to_vec(),
                        password: Vec::new(),
                        player,
                    },
                    password: Vec::new(),
                    description: d,
                },
                0xa635,
                0,
            );
            h.poll(0);
            h
        };
        let join = |h: &mut Host, kind: GameKind, name: &[u8]| {
            let id = h.accept().unwrap();
            let request = JoinRequest {
                name: name.to_vec(),
                password: Vec::new(),
                player: PlayerData::new(name, b"Team", [0; 8]),
            };
            let mut j = Joiner::new_as(kind, request, 0x1000, 0);
            h.receive(id, &j.take_outgoing());
            h.poll(10);
            (id, j)
        };
        let listed = |j: &Joiner| j.players().iter().map(|p| p.id).collect::<Vec<_>>();
        let mut h = server(crate::game_kind::TFL_GAMEPLAY_VERSION);
        assert!(h.is_dedicated());
        let (ours, mut o) = join(&mut h, GameKind::Tfl, b"Ours");
        let (tfl, mut t) = join(&mut h, GameKind::Myth2, b"Tfl15");
        assert!(!h.server_hidden_from(ours));
        assert!(h.server_hidden_from(tfl));
        h.poll(20);
        o.receive(&h.take_outgoing(ours));
        t.receive(&h.take_outgoing(tfl));
        o.poll(20);
        t.poll(20);
        assert_eq!(o.local_id(), Some(ours));
        assert_eq!(t.local_id(), Some(tfl));
        assert_eq!(listed(&o), [0, u16::from(ours), u16::from(tfl)]);
        assert!(o.server_is_ours());
        assert_eq!(listed(&t), [u16::from(ours), u16::from(tfl)]);
        for id in [HOST_NET_ID, ours, tfl] {
            let entry = Message::new(
                TO_ALL,
                HOST_NET_ID,
                Body::PlayerSyncData {
                    id: u16::from(id),
                    value: 0,
                },
            );
            h.broadcast(&entry);
        }
        let entries = |bytes: Vec<u8>| {
            let mut s = Splitter::without_greeting();
            s.push(&bytes);
            std::iter::from_fn(|| match s.next_frame() {
                Ok(Some(Frame::Message(m))) => Message::decode(&m).ok(),
                _ => None,
            })
            .filter_map(|m| match m.body {
                Body::PlayerSyncData { id, .. } => Some(id),
                _ => None,
            })
            .collect::<Vec<_>>()
        };
        assert_eq!(
            entries(h.take_outgoing(ours)),
            [0, u16::from(ours), u16::from(tfl)]
        );
        assert_eq!(
            entries(h.take_outgoing(tfl)),
            [u16::from(ours), u16::from(tfl)]
        );
        let mut h = server(u32::from(crate::player_data::GAMEPLAY_VERSION));
        let (id, mut j) = join(&mut h, GameKind::Myth2, b"Myth2");
        assert!(!h.server_hidden_from(id));
        j.receive(&h.take_outgoing(id));
        j.poll(20);
        assert_eq!(listed(&j), [0, u16::from(id)]);
    }

    fn guest() -> Joiner {
        let request = JoinRequest {
            name: b"Guest".to_vec(),
            password: Vec::new(),
            player: PlayerData::new(b"Guest", b"Team", [0; 8]),
        };
        let mut j = Joiner::new(request, 1, 0);
        j.receive(&encode_greeting(2));
        j
    }

    #[test]
    fn a_joiner_keeps_no_bytes_once_its_stream_failed() {
        let mut j = guest();
        j.receive(&[0, 0x0d, 0, 3, 1, 0xfe, 0, 0]);
        assert!(
            j.poll(10)
                .iter()
                .any(|e| matches!(e, Event::StreamError(_)))
        );
        let pending = j.splitter.pending();
        j.receive(&[0; 0x1000]);
        assert_eq!(j.splitter.pending(), pending);
    }

    #[test]
    fn code_100_forgets_the_last_games_deferred_drops() {
        let mut j = guest();
        j.server = true;
        j.deferred_drops.push(3);
        let start = Message::new(
            0,
            HOST_NET_ID,
            Body::Configuration {
                flags: 0,
                code: crate::lobby::code::START,
                value: 0,
            },
        );
        j.receive(&start.encode().unwrap());
        j.poll(10);
        assert!(j.deferred_drops.is_empty());
    }
}
