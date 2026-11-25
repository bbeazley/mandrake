#[path = "../../mandrake-common/tests/support/require_data.rs"]
mod data;

use mandrake_tfl_sim::net_game::{self, NetGame};

#[test]
fn a_unit_adjustment_is_queued_in_the_films_form() {
    let mut net = NetGame::new(1, 0);
    net.queue_unit_adjustment(&[2, -1, 10]);
    let out = net.take_outgoing_commands();
    assert_eq!(out.len(), 1);
    let p = &out[0];
    assert_eq!(
        (p.command_type.to_u8(), p.player_index, p.tick),
        (net_game::UNIT_ADJUSTMENT, 0xff, 0)
    );
    assert_eq!(p.raw_payload, [0, 0, 0, 3, 0, 2, 0xff, 0xff, 0, 10]);
}
