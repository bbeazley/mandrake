use mandrake_sim::engine_rng::EngineWorld;
use mandrake_sim::players::{Player, Players, Team, player_name};
use mandrake_sim::save_restore::{ReplacePlayer, solo_restore_entries};

fn players(names: &[(&[u8], i8, i32)]) -> Players {
    Players {
        players: names
            .iter()
            .map(|&(_, team, id)| Player {
                flags: 0,
                field_09: 0,
                team,
                field_0b: team,
                id,
            })
            .collect(),
        colors: vec![[[0x1000, 0x1000, 0x1000, 0]; 2]; names.len()],
        names: names.iter().map(|&(n, _, _)| player_name(n)).collect(),
        teams: vec![
            Team {
                flags: 0,
                players: 1,
                player: 0,
            },
            Team::default(),
        ],
        ..Players::default()
    }
}

#[test]
fn an_unmatched_player_is_dropped_and_the_entry_added() {
    let mut w = EngineWorld::default();
    w.players = players(&[(b"Someone", 0, 0)]);
    let mut entries = solo_restore_entries(1);
    let commands = w.restore_player_commands(&mut entries, 7);
    let kinds: Vec<(u8, u8, u32)> = commands
        .iter()
        .map(|c| (c.command_type.to_u8(), c.player_index, c.tick))
        .collect();
    assert_eq!(kinds, vec![(0x1a, 0xff, 7), (0x16, 0xff, 7), (9, 0x10, 7)]);
    let rename = ReplacePlayer::from_bytes(&commands[0].raw_payload).unwrap();
    assert_eq!((rename.new_id, rename.old_id), (0x10, 0));
    assert_eq!(&commands[1].raw_payload[0..6], &[0, 0, 0, 1, 0, 0]);

    w.players.names[0] = player_name(b"Light");
    let mut entries = solo_restore_entries(1);
    let commands = w.restore_player_commands(&mut entries, 7);
    assert_eq!(commands.len(), 2);
    let back = ReplacePlayer::from_bytes(&commands[1].raw_payload).unwrap();
    assert_eq!((back.new_id, back.old_id, back.field_02), (0, 0x10, 0x889));
    assert_eq!(back.colors, [[0xffff, 0xffff, 0xffff, 0]; 2]);
    assert_eq!(entries[0].id, 0);
}
