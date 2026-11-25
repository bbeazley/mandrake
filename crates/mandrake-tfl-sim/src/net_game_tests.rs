use mandrake_rec::CommandPacket;

use crate::net_game::*;

struct Clock {
    gate: NetGate,
    tick: u32,
    horizon: u32,
}

impl Clock {
    fn new(mode: i16) -> Self {
        Self {
            gate: NetGate::new_map(mode),
            tick: 0,
            horizon: 0,
        }
    }

    fn frame(&mut self) -> Option<GateStep> {
        let s = self.gate.frame(1, self.tick, self.horizon, false);
        if let Some(s) = s {
            self.tick += s.bodies.max(0) as u32;
        }
        s
    }
}

fn step(bodies: i32, requested: i32) -> Option<GateStep> {
    Some(GateStep { bodies, requested })
}

#[test]
fn a_map_starts_with_lead_5_and_30_ticks_a_frame_in_mode_1() {
    let g = NetGate::new_map(1);
    assert_eq!((g.lead, g.max_ticks, g.speed_index), (5, 30, 5));
    assert!(!g.fill);
    assert_eq!(
        [0, 1, 2, 3].map(|m| NetGate::new_map(m).max_ticks),
        [4, 30, 4, 4]
    );
}

#[test]
fn a_starvation_fills_to_the_lead() {
    let mut c = Clock::new(1);
    assert_eq!(c.frame(), step(0, 1));
    assert!(c.gate.fill);
    assert_eq!(c.gate.starvations, 1);
    for h in 1..5 {
        c.horizon = h;
        assert_eq!(c.frame(), step(0, 1), "horizon {h}");
        assert_eq!(c.gate.starvations, 1);
    }
    c.horizon = 5;
    assert_eq!(c.frame(), step(1, 1));
    assert!(!c.gate.fill);
    assert_eq!(c.tick, 1);
    for _ in 0..50 {
        c.horizon += 1;
        assert_eq!(c.frame(), step(1, 1));
    }
    assert_eq!(c.horizon - c.tick, 4);
    c.horizon = c.tick + 2;
    assert_eq!(c.frame(), step(1, 1));
    assert!(!c.gate.fill);
}

#[test]
fn catching_up_keeps_the_lead_and_the_frame_cap() {
    let mut c = Clock::new(1);
    c.horizon = 20;
    assert_eq!(c.frame(), step(15, 1));
    assert_eq!(c.tick, 15);
    c.horizon = 200;
    assert_eq!(c.frame(), step(30, 1));
    let mut solo = Clock::new(2);
    solo.horizon = 200;
    assert_eq!(solo.frame(), step(4, 1));
}

#[test]
fn the_twelfth_starvation_raises_the_lead() {
    let mut c = Clock::new(1);
    for n in 1..=12 {
        c.horizon = c.tick;
        assert_eq!(c.frame(), step(0, 1), "starvation {n}");
        if n < 12 {
            assert_eq!(c.gate.starvations, n);
            c.horizon = c.tick + 5;
            assert_eq!(c.frame(), step(1, 1));
        }
    }
    assert_eq!(c.gate.lead, 7);
    assert_eq!(c.gate.starvations, 0);
    assert_eq!(i32::from(c.gate.window_start), c.gate.local_time);
    c.horizon = c.tick + 6;
    assert_eq!(c.frame(), step(0, 1));
    c.horizon = c.tick + 7;
    assert_eq!(c.frame(), step(1, 1));
}

#[test]
fn a_window_of_600_polls() {
    let mut c = Clock::new(1);
    for _ in 0..11 {
        c.horizon = c.tick;
        c.frame();
        c.horizon = c.tick + 5;
        c.frame();
    }
    assert_eq!(c.gate.starvations, 11);
    let start = c.gate.window_start;
    while c.gate.local_time <= i32::from(start) + NET_WINDOW {
        c.horizon += 1;
        assert_eq!(c.frame(), step(1, 1));
    }
    c.horizon = c.tick;
    assert_eq!(c.frame(), step(0, 1));
    assert_eq!(c.gate.lead, 5);
    assert_eq!(c.gate.starvations, 1);
    assert_eq!(i32::from(c.gate.window_start), c.gate.local_time);
}

#[test]
fn a_replay_counts_no_starvation() {
    let mut c = Clock::new(0);
    assert_eq!(c.frame(), step(0, 1));
    assert!(!c.gate.fill);
    assert_eq!(c.gate.starvations, 0);
}

#[test]
fn the_speed_scales_the_requested_ticks_and_the_cap() {
    let mut g = NetGate::new_map(1);
    g.speed_index = 4;
    assert_eq!(g.frame(1, 0, 100, false), None);
    assert_eq!(g.remainder, 128);
    assert_eq!(g.frame(1, 0, 100, false), step(30, 1));
    g.speed_index = 6;
    assert_eq!(g.frame(1, 0, 100, false), step(60, 2));
    assert_eq!(g.frame(40, 0, 1000, false), step(60, 60));
}

#[test]
fn paused_runs_nothing() {
    let mut g = NetGate::new_map(1);
    assert_eq!(g.frame(1, 0, 0, true), step(0, 0));
    assert!(g.fill);
    assert_eq!(g.frame(1, 0, 50, true), step(0, 0));
}

fn syncs(n: &mut NetGame) -> Vec<CommandPacket> {
    n.take_outgoing_commands()
        .into_iter()
        .filter(|p| p.command_type.to_u8() == SYNC)
        .collect()
}

#[test]
fn the_host_syncs_every_171_ticks() {
    let mut host = NetGame::new(0, 0);
    let mut at = Vec::new();
    for t in 1..=1000 {
        host.count_down_sync(1, (512, 0x1234_5678));
        let s = syncs(&mut host);
        assert!(s.len() <= 1);
        if let Some(p) = s.first() {
            assert_eq!(p.player_index, NOBODY);
            assert_eq!(p.tick, 0);
            assert_eq!(
                p.raw_payload,
                [0, 0, 0, 0, 0x12, 0x34, 0x56, 0x78, 0, 0, 2, 0]
            );
            at.push(t);
        }
    }
    assert_eq!(at, [1, 172, 343, 514, 685, 856]);
    let mut joiner = NetGame::new(1, 0);
    for _ in 0..1000 {
        joiner.count_down_sync(1, (0, 0));
        assert!(joiner.take_outgoing_commands().is_empty());
    }
    assert_eq!(joiner.sync_counter(), host.sync_counter());
    let mut next = NetGame::new(0, 23);
    let first = (1..).find(|_| {
        next.count_down_sync(1, (0, 0));
        !syncs(&mut next).is_empty()
    });
    assert_eq!(first, Some(24));
    let mut big = NetGame::new(0, 10);
    big.count_down_sync(30, (0, 0));
    assert_eq!(syncs(&mut big).len(), 1);
    assert_eq!(big.sync_counter(), SYNC_RESET);
}

#[test]
fn out_of_sync_flags_and_goes_on() {
    let mut n = NetGame::new(1, 0);
    n.game_is_out_of_sync(MONSTER_COMMANDS_C, 0x2d1, 600);
    assert!(n.out_of_sync_flag);
    assert_eq!(n.console_error, Some(0));
    assert_eq!(
        n.out_of_sync[0].line_text(),
        "OOS occurred at line 721 in .\\Myth1Code\\monster_commands.c"
    );
}

#[test]
fn incoming_commands_raise_the_horizon() {
    let mut n = NetGame::new(1, 0);
    n.queue_incoming_command(CommandPacket::parse(3, 0, 0, &[]));
    n.queue_incoming_command(CommandPacket::parse(3, 0, CHAT, &[0, 0, b'h', b'i', 0]));
    n.queue_incoming_command(CommandPacket::parse(7, 0, 0, &[]));
    assert_eq!(n.horizon, 7);
    assert_eq!(n.recorded.len(), 1);
    assert_eq!(n.incoming.pop(2), None);
    assert_eq!(n.incoming.pop(3).map(|p| p.tick), Some(3));
    assert_eq!(n.incoming.pop(3).map(|p| p.tick), Some(3));
    assert_eq!(n.incoming.pop(6), None);
    assert_eq!(n.incoming.pop(9).map(|p| p.tick), Some(7));
    assert!(n.incoming.is_empty());
}

#[test]
fn a_full_queue_is_out_of_sync() {
    let mut n = NetGame::new(1, 0);
    let mut t = 0;
    while !n.out_of_sync_flag {
        n.queue_incoming_command(CommandPacket::parse(t, 0, 0, &[0; 56]));
        t += 1;
    }
    assert!(n.incoming.len() < (INCOMING_QUEUE_SIZE / 64) as usize);
    n.queue_incoming_command(CommandPacket::parse(t, 0, 0, &[0; 56]));
    assert_eq!(n.out_of_sync.len(), 1);
    assert_eq!(
        (n.out_of_sync[0].file, n.out_of_sync[0].line),
        (COMMANDS_C, 0xb4)
    );
}
