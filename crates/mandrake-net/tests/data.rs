use mandrake_net::player_data::*;

#[test]
fn the_players_sort_by_rank_then_id_and_group_by_team() {
    let p = |id: i32, team: i32, rank: i16| {
        let mut p = PlayerData::new(b"p", b"t", [0; 8]);
        p.net_id = id;
        p.team = team;
        p.appearance.rank = rank;
        p
    };
    let mut players = vec![
        p(1, 3, -1),
        p(2, 2, -1),
        p(3, 3, -1),
        p(5, 9, -1),
        p(4, -1, 0),
    ];
    sort_players(&mut players);
    let order: Vec<(i32, i32)> = players.iter().map(|p| (p.net_id, p.team)).collect();
    assert_eq!(order, [(4, -1), (2, 2), (3, 3), (1, 3), (5, -1)]);
    assert_eq!(players[4].flags, 0x11 | 0x20);
    assert!(players[..4].iter().all(|p| p.flags == 0x11));
}
