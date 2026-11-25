//! TFL film header round-trip and network new-game data tests.

use mandrake_common::FourCC;
use mandrake_rec::tfl::{
    TflBlockSizes, TflFilmHeader, TflGameParameters, TflNetPlayer, TflNewGame, TflRecording,
    tfl_film, tfl_film_tag_flags, tfl_recording_header, tfl_tag_header,
};

/// 58cm's one player as the net layer held him, by what the film kept.
fn steamuser() -> TflNetPlayer {
    TflNetPlayer {
        team: 0,
        field_02: 0,
        flags: 1,
        field_06: 0,
        captain: -1,
        net_id: 0,
        field_10: [0; 2],
        metaserver_id: -1,
        field_1c: [0; 2],
        field_1e: 0,
        field_20: [0; 8],
        appearance_00: 0,
        appearance_02: -1,
        name: "steamuser".into(),
        team_name: "steamuser\u{2019}s Team".into(),
        primary: [0x4ccc, 0x4d7f, 0xffff, 0],
        secondary: [0xffff, 0x5399, 0x3332, 0],
    }
}

fn player(name: &str, net_id: i32, captain: i32) -> TflNetPlayer {
    TflNetPlayer {
        net_id,
        captain,
        name: name.into(),
        team_name: format!("{name}\u{2019}s Team"),
        team: 7,
        field_02: 3,
        ..steamuser()
    }
}

fn network_parameters(options: u32) -> TflGameParameters {
    TflGameParameters {
        mode: 1,
        game_type: 0,
        options,
        time_limit: 18000,
        mesh: FourCC(*b"58cm"),
        difficulty: 2,
        max_players: 16,
        field_14: 0x31c9,
        team_size: -1,
        seed: 0x0091_a6a8,
        planning_time: 0,
        field_20: [0; 8],
    }
}

/// A whole film of a header and one empty last block.
fn film_of(game: &TflNewGame) -> Vec<u8> {
    let data_size = TflFilmHeader::SIZE + 8;
    let mut out = tfl_tag_header(
        b"Last Recording",
        FourCC(*b"lare"),
        3,
        0,
        [0, 4, 0x80, 4],
        data_size as u32,
        0,
    )
    .to_vec();
    out.extend_from_slice(&tfl_recording_header(
        0,
        game,
        628,
        TflBlockSizes::default(),
        [0x1841_8fcf, 0x22cb_50e1],
    ));
    out.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]);
    out
}

#[test]
fn a_two_player_network_header_reads_back() {
    // The joiner (net id 1) on the host's team, the host (net id 0): sorted by net id.
    let players = [player("joiner", 1, 0), player("host", 0, 0)];
    let game = TflNewGame::network(&network_parameters(0xe024), &players, 1);
    assert_eq!(game.local_player, 1);
    let bytes = film_of(&game);
    let rec = TflRecording::from_bytes(&bytes).unwrap();
    let h = &rec.header;
    assert_eq!(h.to_bytes()[..], bytes[..0x40 + TflFilmHeader::SIZE]);
    assert_eq!((h.mode, h.local_player, h.end_tick), (1, 1, 628));
    let names: Vec<_> = h.players.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["host", "joiner"]);
    for (i, p) in h.players.iter().enumerate() {
        assert_eq!((p.slot, p.team, p.field_02), (i, -1, 0));
        assert_eq!((p.net_id, p.captain), (i as i16, 0));
    }
    assert_eq!(h.new_game(), game);
    assert!(rec.tick_commands.iter().all(|t| t.commands.is_empty()));
}

#[test]
fn teams_are_grouped_and_orphans_lose_their_captain() {
    // Players 0 and 2 are captains; 3 joins 2, 1 joins a player who isn't there (9).
    let players = [
        player("p3", 3, 2),
        player("p1", 1, 9),
        player("p2", 2, 2),
        player("p0", 0, -1),
    ];
    let game = TflNewGame::network(&network_parameters(0), &players, 2);
    let got: Vec<_> = game
        .players
        .iter()
        .map(|p| (p.name.as_str(), p.captain, p.flags))
        .collect();
    // Second pass promotes the orphan (flag 0x20) to a captain in its place.
    assert_eq!(
        got,
        [("p0", -1, 1), ("p1", -1, 0x21), ("p2", 2, 1), ("p3", 2, 1)]
    );
    assert_eq!(game.local_player, 2);
}

#[test]
fn a_cooperative_game_adds_the_computer() {
    let players = [player("host", 0, -1), player("joiner", 1, 0)];
    let game = TflNewGame::network(&network_parameters(0x80), &players, 0);
    let p = &game.parameters;
    assert_eq!((p.options, p.time_limit, p.team_size), (0xa0, 0, -1));
    let got: Vec<_> = game
        .players
        .iter()
        .map(|p| (p.name.as_str(), p.team, p.field_02, p.captain, p.net_id))
        .collect();
    assert_eq!(
        got,
        [
            ("host", 0, 0, 0, 0),
            ("joiner", 0, 0, 0, 1),
            ("Dark", 1, 1, -1, -1),
            ("Neutral", 2, 1, -1, -1),
        ]
    );
    assert_eq!(game.players[2].primary, [0, 0, 0xffff, 0]);
    assert_eq!(game.players[3].secondary, [0x4000, 0, 0, 0]);
}

#[test]
fn a_player_record_round_trips() {
    let p = player("someone", 5, 0);
    assert_eq!(TflNetPlayer::from_bytes(&p.to_bytes()), p);
    let q = network_parameters(0xe0bc);
    assert_eq!(TflGameParameters::from_bytes(&q.to_bytes()), q);
}

#[test]
fn tfl_command_blocks_skips_null_and_oversized_without_empty_blocks() {
    use mandrake_rec::command::{
        CommandPacket, CommandPayload, CommandType, RawCommand, TickCommands, read_command_stream,
    };
    use mandrake_rec::tfl::{TFL_MAX_COMMAND_BLOCK, tfl_command_blocks};

    let null_cmd = CommandPacket {
        tick: 1,
        player_index: 0,
        command_type: CommandType::Null,
        payload: CommandPayload::Null,
        raw_payload: Vec::new(),
    };
    let oversized_cmd = CommandPacket {
        tick: 2,
        player_index: 0,
        command_type: CommandType::Unknown(99),
        payload: CommandPayload::Unknown(RawCommand {
            command_type: 99,
            data: vec![0xaa; TFL_MAX_COMMAND_BLOCK],
        }),
        raw_payload: vec![0xaa; TFL_MAX_COMMAND_BLOCK],
    };
    let valid_cmd = CommandPacket::parse(3, 0, CommandType::Sync.to_u8(), &[0u8; 12]);

    // Only Null and oversized: single empty last block, no spurious empty non-last block.
    let empty_blocks = tfl_command_blocks(&[TickCommands {
        tick: 1,
        commands: vec![null_cmd.clone(), oversized_cmd.clone()],
    }]);
    assert_eq!(empty_blocks, [0, 0, 0, 1, 0, 0, 0, 0]);

    // Mixed stream: only the valid command survives and round-trips through read_command_stream.
    let blocks = tfl_command_blocks(&[TickCommands {
        tick: 1,
        commands: vec![null_cmd, oversized_cmd, valid_cmd.clone()],
    }]);
    let mut cursor = std::io::Cursor::new(&blocks);
    let parsed = read_command_stream(&mut cursor, 0).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].tick, 3);
    assert_eq!(parsed[0].commands, vec![valid_cmd]);

    // Whole tfl_film write count remains 4 (1 block) when Null/oversized are skipped.
    let game = TflNewGame::network(&network_parameters(0), &[steamuser()], 0);
    let film = tfl_film(
        &game,
        [0, 0],
        b"GuardTest",
        FourCC(*b"grdt"),
        10,
        &[],
        &[TickCommands {
            tick: 1,
            commands: vec![
                CommandPacket {
                    tick: 1,
                    player_index: 0,
                    command_type: CommandType::Null,
                    payload: CommandPayload::Null,
                    raw_payload: Vec::new(),
                },
                CommandPacket {
                    tick: 2,
                    player_index: 0,
                    command_type: CommandType::Unknown(99),
                    payload: CommandPayload::Unknown(RawCommand {
                        command_type: 99,
                        data: vec![0xbb; TFL_MAX_COMMAND_BLOCK],
                    }),
                    raw_payload: vec![0xbb; TFL_MAX_COMMAND_BLOCK],
                },
            ],
        }],
    );
    assert_eq!(&film[0x2c..0x30], &tfl_film_tag_flags(4));
    let parsed_film = TflRecording::from_bytes(&film).unwrap();
    assert!(parsed_film.tick_commands.is_empty());
}
