mod support;

use mandrake_net::endpoint::{Host, HostOptions, Joiner};
use mandrake_net::player_data::{JoinRequest, PlayerData};
use mandrake_rec::writer::NewGameData;
use mandrake_server::config::Config;
use mandrake_sim::engine_rng::{EngineRng, SiteCounts};
use mandrake_sim::players::{GameSetup, Players, initialize_players, set_random_seed};

fn request(name: &[u8]) -> JoinRequest {
    let mut player = PlayerData::new(name, name, [0; 8]);
    if name == b"Local test server" {
        player.field_22 = mandrake_net::ours::SERVER_MARK;
    }
    JoinRequest {
        name: name.to_vec(),
        password: Vec::new(),
        player,
    }
}

fn game(cooperative: bool) -> Vec<NewGameData> {
    game_at(cooperative, None)
}

fn game_at(cooperative: bool, seed: Option<u32>) -> Vec<NewGameData> {
    let config = Config::parse(
        &format!(
            "data_dir = \"/d\"\nname = \"Local test server\"\n[game]\nmap = \"85gi\"\n\
             cooperative = {cooperative}\n"
        ),
        std::path::Path::new("/"),
    )
    .unwrap();
    let mut host = Host::new(
        HostOptions {
            player: request(b"Local test server"),
            password: Vec::new(),
            description: config.description().unwrap(),
        },
        0xa635,
        0,
    );
    host.poll(0);
    let mut joiners: Vec<(u8, Joiner)> = Vec::new();
    for (k, name) in [&b"Alpha"[..], b"Beta"].into_iter().enumerate() {
        let id = host.accept().unwrap();
        joiners.push((id, Joiner::new(request(name), 0x1000 + k as u16, 0)));
    }
    for t in 0..8 {
        host.poll(10 * t);
        for (id, j) in &mut joiners {
            if t == 3 {
                assert!(j.set_ready(true));
            }
            host.receive(*id, &j.take_outgoing());
            host.poll(10 * t + 1);
            j.receive(&host.take_outgoing(*id));
            j.poll(10 * t + 2);
        }
    }
    if let Some(seed) = seed {
        host.start(seed);
        for t in 8..10 {
            host.poll(10 * t);
            for (id, j) in &mut joiners {
                j.receive(&host.take_outgoing(*id));
                j.poll(10 * t + 2);
            }
        }
    }
    let mut out = vec![host.local().new_game_data().unwrap().unwrap()];
    for (_, j) in &joiners {
        assert_eq!(j.players().len(), 3, "the server and both joiners");
        out.push(j.new_game_data().unwrap().unwrap());
    }
    out
}

fn players(data: &NewGameData) -> (Vec<u8>, Players, i16) {
    let film = mandrake_rec::writer::film_file(
        b"Test",
        mandrake_rec::writer::subgroup_for_name(b"Test", &[]),
        &mandrake_rec::writer::Recorder::new(data.clone(), &[]).finish(0),
    );
    let header = mandrake_session::films::read_header(&film)
        .unwrap()
        .to_vec();
    let gameplay = mandrake_session::films::parse_game(&header).gameplay_version;
    let mut setup = GameSetup::from_film(&film).unwrap();
    let mut rng = set_random_seed(0x1234_5678);
    let mut sites = SiteCounts::default();
    let mut rng = EngineRng {
        rng: &mut rng,
        sites: &mut sites,
    };
    let players = initialize_players(&mut setup, 4, (0, 0), i32::from(gameplay), &mut rng).unwrap();
    (header, players, setup.local_player)
}

const ALPHA_CAPTAIN_SEED: u32 = 0xd199_876f;
const BETA_CAPTAIN_SEED: u32 = 0xd19c_852f;

fn name(n: &[u8]) -> &[u8] {
    &n[..n.iter().position(|&b| b == 0).unwrap_or(n.len())]
}

#[test]
fn every_peer_names_the_players_as_the_sim_orders_them() {
    for cooperative in [false, true] {
        for (peer, data) in game(cooperative).iter().enumerate() {
            let (header, players, local) = players(data);
            let names = mandrake_session::films::player_names(&header);
            let sim: Vec<&[u8]> = players.names.iter().map(|n| name(n)).collect();
            let ours: Vec<&[u8]> = names.iter().map(|(n, _)| n.0.as_slice()).collect();
            assert_eq!(ours, sim, "peer {peer}, cooperative {cooperative}");
            assert_eq!(ours[0], b"Alpha", "the first player is a human");
            let last = players.players.last().unwrap();
            assert_eq!(name(players.names.last().unwrap()), b"Local test server");
            assert_ne!(last.flags & 0x40, 0, "the server is an observer");
            assert_eq!(last.team, -1, "the server has no team");
            match peer {
                0 => assert_eq!(local, -1),
                p => assert_eq!(
                    ours[local as usize],
                    [&b"Alpha"[..], b"Beta"][p - 1],
                    "peer {peer}'s own name"
                ),
            }
        }
    }
}

#[test]
fn a_cooperative_game_deals_the_units_to_one_human_captain() {
    for data in game(true) {
        let (_, players, _) = players(&data);
        let (alpha, beta) = (players.players[0], players.players[1]);
        assert_ne!(alpha.team, -1);
        assert_eq!(alpha.team, beta.team, "one team");
        let team = players.teams[alpha.team as usize];
        assert_eq!(team.players, 2);
        assert!(
            team.player == 0 || team.player == 1,
            "the captain is a human"
        );
    }
}

#[test]
fn the_seed_draws_the_captain() {
    for (seed, captain) in [
        (ALPHA_CAPTAIN_SEED, 0),
        (BETA_CAPTAIN_SEED, 1),
        (0xd196_14f3, 1),
        (0xd183_f8fb, 1),
        (0xd1b3_ce99, 0),
    ] {
        for data in game_at(true, Some(seed)) {
            let film = mandrake_net::client::film(data, &[], 0, b"Network Game");
            let mut setup = GameSetup::from_film(&film).unwrap();
            let mut rng = set_random_seed(seed);
            let mut sites = SiteCounts::default();
            let mut rng = EngineRng {
                rng: &mut rng,
                sites: &mut sites,
            };
            let players = initialize_players(&mut setup, 4, (0, 0), 0x73c, &mut rng).unwrap();
            let team = players.teams[players.players[0].team as usize];
            assert_eq!(team.player, captain, "seed {seed:#x}");
        }
    }
}
