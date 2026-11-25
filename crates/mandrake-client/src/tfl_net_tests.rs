use super::*;

#[test]
fn the_stream_is_grouped_by_tick() {
    let c = |tick, kind| CommandPacket::parse(tick, 0, kind, &[0; 12]);
    let groups = by_tick(&[c(3, 1), c(3, 4), c(9, 1)]);
    assert_eq!(
        groups
            .iter()
            .map(|g| (g.tick, g.commands.len()))
            .collect::<Vec<_>>(),
        [(3, 2), (9, 1)]
    );
}

#[test]
fn the_windows_orders_go_to_the_outgoing_queue() {
    let mut net = NetGame::new(1, 0);
    let payload = film::movement_payload(0, 0, 0x4000, &[3, 4], &[(0x1000, 0x2000)]);
    let movement = film::movement_command(0, 0, &payload);
    assert!(give(&mut net, &movement));
    let target = film::target_payload(0, &[3], &[9]);
    let attack = CommandPacket::parse(0, 0, film::TARGET_COMMAND, &target);
    assert!(give(&mut net, &attack));
    let other = CommandPacket::parse(0, 0, 14, &[0, 0]);
    assert!(!give(&mut net, &other), "type 14 isn't TFL's");
    let out = net.take_outgoing_commands();
    let got: Vec<(u8, Vec<u8>)> = out
        .iter()
        .map(|c| (c.command_type.to_u8(), c.raw_payload.clone()))
        .collect();
    assert_eq!(
        got,
        [
            (mandrake_rec::CommandType::Movement.to_u8(), payload),
            (film::TARGET_COMMAND, target)
        ]
    );
}
