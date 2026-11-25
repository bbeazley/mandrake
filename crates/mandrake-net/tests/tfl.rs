use std::collections::BTreeMap;

use mandrake_net::endpoint::{Event, Host, HostOptions, Joiner};
use mandrake_net::game_description::{GameDescription, GameParameters, PARAMETERS_SIZE};
use mandrake_net::game_kind::{GameKind, MYTH2_COMMANDS, TFL_COMMANDS, TFL_GAMEPLAY_VERSION};
use mandrake_net::lobby::{BarrierKind, InvalidGame, check, check_as};
use mandrake_net::player_data::{JoinRequest, PlayerData};
use mandrake_net::tfl::{self, DataMismatch, NO_PLUGIN};
use mandrake_net::wire::{Body, Command, Frame, Message, Splitter};

const ARCHIVE: u32 = 0x1357_9bdf;
const PLUGIN: u32 = 0x22cb_50e1;
const OURS: [u32; 2] = [ARCHIVE, PLUGIN];

fn tfl_parameters() -> GameParameters {
    let mut p = GameParameters::read(&[0; PARAMETERS_SIZE]);
    p.mode = 1;
    p.game_type = 0;
    p.options = 0xe024;
    p.time_limit = 18_000;
    p.mesh = *b"58cm";
    p.difficulty = 2;
    p.max_players = 16;
    p.seed_low = 0x31c9;
    p.field_16 = -1;
    p.seed = 0x0091_a6a8;
    p
}

fn tfl_description() -> GameDescription {
    tfl::description(
        &tfl_parameters(),
        OURS,
        b"TFL game",
        b"For Carnage Apply Within",
        false,
    )
}

fn myth2_description() -> GameDescription {
    let mut p = GameParameters::read(&[0; PARAMETERS_SIZE]);
    p.mode = 1;
    p.game_type = 2;
    p.max_players = 16;
    p.mesh = *b"85gi";
    p.version = 0x889;
    p.build = 0x1d7;
    GameDescription {
        parameters: p,
        plugins: Default::default(),
        foundation_checksum: 0,
        field_230: 0,
        flags: 0,
        player_count: 1,
        game_name: b"Myth II game".to_vec(),
        map_name: Vec::new(),
    }
}

fn request(name: &[u8], rank: i16) -> JoinRequest {
    let mut player = PlayerData::new(name, b"Team", [0; 8]);
    player.appearance.rank = rank;
    JoinRequest {
        name: name.to_vec(),
        password: Vec::new(),
        player,
    }
}

struct Game {
    host: Host,
    joiners: Vec<(u8, Joiner)>,
    seen: BTreeMap<u8, Vec<Event>>,
    wire: BTreeMap<u8, Vec<Message>>,
    splitters: BTreeMap<u8, Splitter>,
}

impl Game {
    fn new(description: GameDescription, host_rank: i16) -> Game {
        let mut g = Game {
            host: Host::new(
                HostOptions {
                    player: request(b"Host", host_rank),
                    password: Vec::new(),
                    description,
                },
                0xa635,
                0,
            ),
            joiners: Vec::new(),
            seen: BTreeMap::new(),
            wire: BTreeMap::new(),
            splitters: BTreeMap::new(),
        };
        g.pump(0);
        assert_eq!(g.host.local().local_id(), Some(0));
        g
    }

    fn connect(&mut self, kind: GameKind, name: &[u8], rank: i16, now: u64) -> u8 {
        let id = self.host.accept().unwrap();
        let j = Joiner::new_as(kind, request(name, rank), 0x1000 + u16::from(id), now);
        self.joiners.push((id, j));
        id
    }

    fn joiner(&mut self, id: u8) -> &mut Joiner {
        &mut self.joiners.iter_mut().find(|(i, _)| *i == id).unwrap().1
    }

    fn pump(&mut self, now: u64) {
        for (id, j) in &mut self.joiners {
            self.host.receive(*id, &j.take_outgoing());
        }
        let local = self.host.poll(now);
        self.seen.entry(0).or_default().extend(local);
        for (id, j) in &mut self.joiners {
            let bytes = self.host.take_outgoing(*id);
            let s = self
                .splitters
                .entry(*id)
                .or_insert_with(Splitter::with_greeting);
            s.push(&bytes);
            let wire = self.wire.entry(*id).or_default();
            while let Some(f) = s.next_frame().unwrap() {
                if let Frame::Message(m) = f {
                    wire.push(Message::decode(&m).unwrap());
                }
            }
            j.receive(&bytes);
            let events = j.poll(now);
            self.seen.entry(*id).or_default().extend(events);
        }
    }

    fn start(&mut self, now: u64) {
        let ids: Vec<u8> = self.joiners.iter().map(|(i, _)| *i).collect();
        for id in &ids {
            assert!(self.joiner(*id).set_ready(true));
        }
        self.pump(now);
        self.host.start(0x0091_a6a8);
        self.host.begin_barrier(BarrierKind::Start);
        self.pump(now);
        for id in &ids {
            assert!(self.joiner(*id).loaded());
        }
        assert!(self.host.local_mut().loaded());
        self.pump(now);
        self.pump(now);
        assert!(self.host.server().is_running());
    }

    fn heard(&self, id: u8) -> Vec<Command> {
        self.seen[&id]
            .iter()
            .flat_map(|e| match e {
                Event::Commands { commands, .. } => commands.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn wired(&self, id: u8) -> Vec<Command> {
        self.wire[&id]
            .iter()
            .filter_map(|m| match &m.body {
                Body::Command(c) => Some(c.clone()),
                _ => None,
            })
            .collect()
    }

    fn clear(&mut self) {
        self.seen.clear();
        self.wire.clear();
    }
}

#[test]
fn the_tfl_codes_and_the_marker() {
    assert_eq!(TFL_GAMEPLAY_VERSION, 0x5446_4c35);
    assert!(!mandrake_net::lobby::VERSION_RANGE.contains(&TFL_GAMEPLAY_VERSION));
    let t = GameKind::Tfl.commands();
    assert_eq!(t, TFL_COMMANDS);
    assert_eq!(
        (t.null, t.sync, t.drop_player, t.chat, t.unit_adjustment),
        (0, 1, 8, 9, 11)
    );
    assert_eq!((t.forced_end, t.pause, t.game_speed), (None, None, None));
    assert!(t.has(13) && !t.has(14));
    let m = GameKind::Myth2.commands();
    assert_eq!(m, MYTH2_COMMANDS);
    assert_eq!((m.drop_player, m.chat, m.unit_adjustment), (9, 10, 12));
    assert_eq!(
        (m.forced_end, m.pause, m.game_speed),
        (Some(0x15), Some(0x17), Some(0x18))
    );
    assert_eq!(m.drop_player, mandrake_net::server::COMMAND_DROP_PLAYER);
}

#[test]
fn a_tfl_1_5_hosts_description_is_tfls_to_a_tfl_client() {
    let mut d = tfl_description();
    d.parameters.version = 0;
    let body = d.to_bytes();
    assert_eq!(&body[0x20..0x2c], &[0; 12], "as TFL 1.5 sends them");
    let m = Message::new(0xff, 0, Body::GameDescription(body));
    let tfl = GameDescription::from_body(&GameKind::Tfl.decoded(m.clone()).body).unwrap();
    assert_eq!(GameKind::of(&tfl), GameKind::Tfl);
    assert_eq!(check_as(GameKind::Tfl, &tfl), Ok(()));
    let myth2 = GameDescription::from_body(&GameKind::Myth2.decoded(m).body).unwrap();
    assert_eq!(
        check_as(GameKind::Myth2, &myth2),
        Err(InvalidGame::Version(0))
    );
}

#[test]
fn a_tfl_description_carries_the_marker_and_tfls_data_check() {
    let d = tfl_description();
    assert_eq!(GameKind::of(&d), GameKind::Tfl);
    assert!(d.plugins.0.is_empty());
    let body = d.to_bytes();
    assert_eq!(&body[0x24..0x28], b"TFL5");
    assert_eq!(&body[0x20..0x24], &[0; 4], "flags and build as TFL 1.5's");
    assert_eq!(body[0x22c..0x230], (!ARCHIVE).to_be_bytes());
    assert_eq!(body[0x230..0x234], PLUGIN.to_be_bytes());
    assert_eq!(GameDescription::read(&body).unwrap(), d);
    assert_eq!(tfl::archive_checksum(&d), ARCHIVE);
    assert_eq!(tfl::plugin_checksum(&d), Some(PLUGIN));
    assert_eq!(tfl::check_data(&d, OURS), Ok(()));
    assert_eq!(
        tfl::check_data(&d, [7, PLUGIN]),
        Err(DataMismatch::Archive {
            host: ARCHIVE,
            ours: 7
        })
    );
    assert_eq!(
        tfl::check_data(&d, [ARCHIVE, NO_PLUGIN]),
        Err(DataMismatch::Plugin {
            host: PLUGIN,
            ours: NO_PLUGIN
        })
    );
    let none = tfl::description(&tfl_parameters(), [ARCHIVE, NO_PLUGIN], b"g", b"m", true);
    assert_eq!(tfl::plugin_checksum(&none), None);
    assert_eq!(none.flags, mandrake_net::game_description::FLAG_PASSWORD);
    assert_eq!(tfl::parameters(&d).field_20, [0; 8]);
    assert_eq!(tfl::film_checksums(&d), OURS);
}

#[test]
fn a_joiner_whose_record_says_net_id_0_is_heard_as_its_own_index() {
    let mut g = Game::new(tfl_description(), -1);
    let a = g.connect(GameKind::Tfl, b"A", 0, 0);
    g.pump(10);
    let b = g.connect(GameKind::Tfl, b"B", 0, 0);
    g.pump(20);
    g.pump(30);
    g.host.start(0x0091_a6a8);
    g.host.begin_barrier(BarrierKind::Start);
    g.pump(1_000);
    for id in [a, b] {
        assert!(g.joiner(id).loaded());
    }
    assert!(g.host.local_mut().loaded());
    g.pump(1_000);
    g.pump(1_000);
    let order = g.joiner(b).game_players().to_vec();
    assert_eq!(order, [1, 2, 0], "every net id listed, each once");
    let new_game = g.joiner(b).tfl_new_game().unwrap().unwrap();
    let ids: Vec<i32> = new_game.players.iter().map(|p| p.net_id as i32).collect();
    assert_eq!(ids, order, "the new-game data in the same order");
    assert_eq!(
        new_game.players[new_game.local_player as usize].net_id as i32,
        i32::from(b)
    );
    g.clear();
    assert!(g.joiner(b).send_command(3, vec![1, 2]));
    g.pump(1_100);
    g.pump(1_110);
    for id in [0, a, b] {
        let heard: Vec<(u8, u8)> = g
            .heard(id)
            .iter()
            .filter(|c| c.kind != 0)
            .map(|c| (c.kind, c.player))
            .collect();
        assert_eq!(
            heard,
            [(3, 1)],
            "player {id} hears B's move as its index, 1"
        );
    }
}

#[test]
fn a_tfl_host_and_a_tfl_joiner_play_and_the_other_games_joiner_is_refused() {
    let mut g = Game::new(tfl_description(), -1);
    assert_eq!(g.host.kind(), GameKind::Tfl);
    assert_eq!(g.host.local().kind(), GameKind::Tfl);
    let a = g.connect(GameKind::Tfl, b"A", 5, 0);
    let m = g.connect(GameKind::Myth2, b"M", -1, 0);
    g.pump(10);
    g.pump(20);
    assert_eq!(g.joiner(a).description_check(), Some(Ok(())));
    let seen = g.joiner(a).description().cloned().unwrap();
    assert_eq!(GameKind::of(&seen), GameKind::Tfl);
    assert_eq!(tfl::check_data(&seen, OURS), Ok(()));
    assert_eq!(
        g.joiner(m).description_check(),
        Some(Err(InvalidGame::OtherGame(GameKind::Tfl)))
    );
    assert_eq!(
        check(&seen),
        Err(InvalidGame::Version(TFL_GAMEPLAY_VERSION))
    );
    g.host.connection_lost(m);
    g.joiners.retain(|(i, _)| *i != m);
    g.pump(30);

    g.start(1_000);
    assert_eq!(g.joiner(a).game_players(), [1, 0]);
    assert_eq!(g.host.local().game_players(), [1, 0]);
    let new_game = g.joiner(a).tfl_new_game().unwrap().unwrap();
    assert_eq!(new_game.local_player, 0);
    assert_eq!(new_game.players[0].name, "A");
    assert_eq!(new_game.parameters.seed, 0x0091_a6a8, "the start's seed");
    assert_eq!(new_game.parameters.field_14, 0x31c9, "TFL's +0x14 stays");
    assert_eq!(new_game.parameters.field_20, [0; 8]);
    assert_eq!(
        g.host.local().tfl_new_game().unwrap().unwrap().local_player,
        1
    );

    g.clear();
    assert!(g.joiner(a).send_command(3, vec![1, 2]));
    assert!(g.host.local_mut().send_command(4, vec![3, 4]));
    g.pump(1_100);
    g.pump(1_110);
    let wired: Vec<(u8, u8)> = g
        .wired(a)
        .iter()
        .filter(|c| c.kind != 0)
        .map(|c| (c.kind, c.player))
        .collect();
    assert_eq!(wired, [(4, 0), (3, 1)], "the server stamps net ids");
    for id in [0, a] {
        let heard: Vec<(u8, u8)> = g.heard(id).iter().map(|c| (c.kind, c.player)).collect();
        assert_eq!(heard, [(4, 1), (3, 0)], "player {id} maps them to indices");
    }
    for kind in [0x15, 0x17, 0x18, 14] {
        assert!(!g.joiner(a).send_command(kind, vec![0; 4]), "{kind}");
    }
    assert!(g.joiner(a).send_command(13, Vec::new()));
}

#[test]
fn a_tfl_joiner_is_refused_a_myth_ii_game() {
    let mut g = Game::new(myth2_description(), -1);
    assert_eq!(g.host.kind(), GameKind::Myth2);
    let t = g.connect(GameKind::Tfl, b"T", -1, 0);
    let m = g.connect(GameKind::Myth2, b"M", -1, 0);
    g.pump(10);
    g.pump(20);
    assert_eq!(
        g.joiner(t).description_check(),
        Some(Err(InvalidGame::OtherGame(GameKind::Myth2)))
    );
    assert_eq!(g.joiner(m).description_check(), Some(Ok(())));
    assert_eq!(check_as(GameKind::Myth2, &myth2_description()), Ok(()));
}

#[test]
fn a_tfl_hosts_drop_player_is_8_with_the_dropped_net_id_and_heard_as_its_index() {
    let mut g = Game::new(tfl_description(), -1);
    let a = g.connect(GameKind::Tfl, b"A", -1, 0);
    let b = g.connect(GameKind::Tfl, b"B", -1, 0);
    g.pump(10);
    g.start(1_000);
    assert_eq!(g.joiner(a).game_players(), [0, 1, 2]);
    g.clear();
    g.host.connection_lost(b);
    g.joiners.retain(|(i, _)| *i != b);
    g.pump(2_000);
    g.pump(2_010);
    let drop = g
        .wired(a)
        .into_iter()
        .find(|c| c.kind != 0)
        .expect("the host's DROP_PLAYER");
    assert_eq!((drop.kind, drop.player), (8, b), "type 8, its net id kept");
    assert!(drop.payload.is_empty());
    let heard: Vec<(u8, u8)> = g.heard(a).iter().map(|c| (c.kind, c.player)).collect();
    assert_eq!(heard, [(8, 2)]);
    assert!(
        g.host.server().history().is_empty(),
        "a TFL game has no mid-game join to keep it for"
    );
}

#[test]
fn a_tfl_game_heartbeats_after_more_than_3_ticks() {
    let mut g = Game::new(tfl_description(), -1);
    let a = g.connect(GameKind::Tfl, b"A", -1, 0);
    g.pump(10);
    g.start(1_000);
    g.clear();
    g.pump(1_133);
    assert!(g.wired(a).is_empty());
    g.pump(1_134);
    let nulls: Vec<(u8, u8, i32)> = g
        .wired(a)
        .iter()
        .map(|c| (c.kind, c.player, c.tick))
        .collect();
    assert_eq!(nulls, [(0, 0xff, 4)]);
    assert!(g.heard(a).is_empty(), "NULLs aren't kept");
    assert_eq!(g.joiner(a).horizon(), 4);
}

#[test]
fn the_player_byte_maps_as_0x451570() {
    let order = [3, 1, -1];
    assert_eq!(tfl::player_index(&order, 3), 0);
    assert_eq!(tfl::player_index(&order, 1), 1);
    assert_eq!(tfl::player_index(&order, 0xff), 0xff, "never mapped");
    assert_eq!(tfl::player_index(&order, 7), 7, "nobody: stays");
    assert_eq!(tfl::player_index(&[], 2), 2);
}

#[test]
fn our_servers_own_player_is_in_no_tfl_game() {
    let d = tfl_description();
    let mut server = PlayerData::new(b"Server", b"Server", [0; 8]);
    server.net_id = 0;
    server.field_22 = mandrake_net::ours::SERVER_MARK;
    assert!(tfl::is_our_server(&server));
    let mut a = request(b"alpha", 0).player;
    a.net_id = 1;
    let mut b = request(b"beta", 0).player;
    b.net_id = 2;
    let players = [server.clone(), a.clone(), b.clone()];
    let order = tfl::player_order(&d, &players);
    assert_eq!(order.len(), 2, "the server isn't listed: {order:?}");
    assert!(!order.contains(&0));
    let game = tfl::new_game(&d, &players, 2).unwrap();
    assert_eq!(game.players.len(), 2);
    assert!(game.players.iter().all(|p| p.net_id != 0));
    assert_eq!(game.players[game.local_player as usize].net_id, 2);
    assert_eq!(tfl::new_game(&d, &players, 0).unwrap().local_player, 0);
    assert!(tfl::new_game(&d, &[server], 0).is_err());
    let mut host = request(b"host", 0).player;
    host.net_id = 0;
    assert_eq!(tfl::player_order(&d, &[host, a]).len(), 2);
}
