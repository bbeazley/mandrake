use std::collections::BTreeMap;

use crate::client::{COMMAND_NULL, UNSTAMPED_PLAYER};
use crate::game_kind::GameKind;
use crate::wire::Command;

pub const TICKS_PER_SECOND: u32 = 30;
pub const HEARTBEAT_MS: u32 = 0x65;
pub const PING_INTERVAL_MS: u32 = 1000;
pub const PONG_SILENCE_MS: u32 = 4000;

pub const COMMAND_DROP_PLAYER: u8 = 9;
pub const TFL_HEARTBEAT_TICKS: i32 = 3;
pub const NO_LATENCY: u16 = 0xffff;

fn ms32(now_ms: u64) -> u32 {
    now_ms as u32
}

#[derive(Debug, Clone, Copy, Default)]
struct Latency {
    shown: u16,
    last_rtt: u32,
    most_rtt: u32,
    last_pong: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Server {
    running: bool,
    start: u32,
    last_sent: u32,
    tick_override: Option<u32>,
    history: Vec<Command>,
    history_unused: bool,
    last_ping: u32,
    latency: BTreeMap<u8, Latency>,
    kind: GameKind,
    last_tick: i32,
}

impl Server {
    pub fn new(kind: GameKind) -> Server {
        Server {
            kind,
            ..Server::default()
        }
    }

    pub fn kind(&self) -> GameKind {
        self.kind
    }

    pub fn start(&mut self, now_ms: u64) {
        self.running = true;
        self.start = ms32(now_ms);
        self.last_tick = 0;
    }

    pub fn new_game(&mut self) {
        self.history = Vec::new();
    }

    pub fn set_keep_history(&mut self, keep: bool) {
        self.history_unused = !keep;
        if !keep {
            self.history = Vec::new();
        }
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn set_tick_override(&mut self, tick: Option<u32>) {
        self.tick_override = tick;
    }

    pub fn resume_at(&mut self, tick: u32, now_ms: u64) {
        self.running = true;
        self.tick_override = None;
        let back = (u64::from(tick) * 1000).div_ceil(u64::from(TICKS_PER_SECOND));
        self.start = ms32(now_ms).wrapping_sub(back as u32);
    }

    pub fn tick(&self, now_ms: u64) -> i32 {
        match self.tick_override {
            Some(t) => t as i32,
            None => {
                (ms32(now_ms)
                    .wrapping_sub(self.start)
                    .wrapping_mul(TICKS_PER_SECOND)
                    / 1000) as i32
            }
        }
    }

    pub fn history(&self) -> &[Command] {
        &self.history
    }

    pub fn stamp(&mut self, sender: u8, mut command: Command, now_ms: u64) -> Command {
        if command.kind != self.kind.commands().drop_player || sender != 0 {
            command.player = sender;
        }
        self.send(command, now_ms)
    }

    pub fn send_own(&mut self, command: Command, now_ms: u64) -> Command {
        self.send(command, now_ms)
    }

    fn send(&mut self, mut command: Command, now_ms: u64) -> Command {
        command.tick = self.tick(now_ms);
        self.last_sent = ms32(now_ms);
        self.last_tick = command.tick;
        if command.kind != COMMAND_NULL && !self.history_unused {
            self.history.push(command.clone());
        }
        command
    }

    pub fn idle(&mut self, now_ms: u64) -> Option<Command> {
        if !self.running {
            return None;
        }
        let quiet = match self.kind {
            GameKind::Myth2 => ms32(now_ms).wrapping_sub(self.last_sent) >= HEARTBEAT_MS,
            GameKind::Tfl => self.tick(now_ms).wrapping_sub(self.last_tick) > TFL_HEARTBEAT_TICKS,
        };
        if !quiet {
            return None;
        }
        let null = Command {
            kind: COMMAND_NULL,
            player: UNSTAMPED_PLAYER,
            tick: 0,
            payload: Vec::new(),
        };
        Some(self.send(null, now_ms))
    }

    pub fn ping(&mut self, now_ms: u64, players: &[u8], local: u8) -> Option<(u32, Vec<i16>)> {
        let now = ms32(now_ms);
        if now.wrapping_sub(self.last_ping) <= PING_INTERVAL_MS {
            return None;
        }
        self.last_ping = now;
        self.latency.retain(|id, _| players.contains(id));
        let latencies = players
            .iter()
            .map(|&id| {
                let l = self.latency.entry(id).or_default();
                if id == local {
                    l.last_pong = Some(0);
                    l.shown = 0;
                } else if l
                    .last_pong
                    .is_none_or(|t| now.wrapping_sub(t) >= PONG_SILENCE_MS)
                {
                    l.shown = NO_LATENCY;
                }
                l.shown as i16
            })
            .collect();
        Some((now, latencies))
    }

    pub fn pong(&mut self, from: u8, ticks: u32, now_ms: u64) {
        let now = ms32(now_ms);
        let last_ping = self.last_ping;
        let l = self.latency.entry(from).or_default();
        if ticks <= now && ticks <= last_ping && l.last_pong.is_none_or(|t| t <= ticks) {
            let rtt = now - ticks;
            let smoothed = if l.shown == NO_LATENCY {
                rtt
            } else {
                (rtt + 3 * u32::from(l.shown)) >> 2
            };
            l.last_pong = Some(now);
            l.shown = smoothed.min(u32::from(NO_LATENCY)) as u16;
            l.last_rtt = rtt;
            l.most_rtt = l.most_rtt.max(rtt);
        }
    }

    pub fn take_round_trips(&mut self) -> Vec<(u8, u32, u32, u16)> {
        self.latency
            .iter_mut()
            .filter(|(_, l)| l.last_pong.is_some_and(|t| t != 0))
            .map(|(id, l)| (*id, l.last_rtt, std::mem::take(&mut l.most_rtt), l.shown))
            .collect()
    }

    pub fn latency(&self, id: u8) -> Option<u16> {
        self.latency.get(&id).map(|l| l.shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(kind: u8) -> Command {
        Command {
            kind,
            player: UNSTAMPED_PLAYER,
            tick: 0,
            payload: vec![1, 2],
        }
    }

    #[test]
    fn the_tick_is_thirty_a_second_of_the_servers_clock() {
        let mut s = Server::default();
        s.start(5_000);
        for (ms, tick) in [
            (0, 0),
            (33, 0),
            (34, 1),
            (100, 3),
            (1000, 30),
            (60_000, 1800),
        ] {
            assert_eq!(s.tick(5_000 + ms), tick, "{ms} ms");
        }
        let c = s.stamp(2, cmd(16), 6_000);
        assert_eq!((c.player, c.tick), (2, 30));
        assert_eq!(s.history().len(), 1);
    }

    #[test]
    fn a_drop_player_from_the_host_keeps_its_player_byte() {
        let mut s = Server::default();
        s.start(0);
        let mut drop = cmd(COMMAND_DROP_PLAYER);
        drop.player = 3;
        assert_eq!(s.stamp(0, drop.clone(), 10).player, 3);
        assert_eq!(s.stamp(1, drop, 10).player, 1);
    }

    #[test]
    fn a_tfl_drop_player_is_8_and_from_the_host_keeps_its_player_byte() {
        let mut s = Server::new(GameKind::Tfl);
        s.start(0);
        let mut drop = cmd(8);
        drop.player = 3;
        assert_eq!(s.stamp(0, drop.clone(), 10).player, 3);
        assert_eq!(s.stamp(1, drop, 10).player, 1);
        let mut nine = cmd(COMMAND_DROP_PLAYER);
        nine.player = 3;
        assert_eq!(s.stamp(0, nine, 10).player, 0);
    }

    #[test]
    fn a_tfl_heartbeat_follows_more_than_3_ticks_with_nothing_sent() {
        let mut s = Server::new(GameKind::Tfl);
        assert!(s.idle(1_000).is_none(), "the clock isn't running");
        s.start(1_000);
        assert!(s.idle(1_000 + 133).is_none());
        let null = s.idle(1_000 + 134).expect("tick 4");
        assert_eq!((null.kind, null.player, null.tick), (COMMAND_NULL, 0xff, 4));
        assert!(s.history().is_empty(), "NULLs aren't kept");
        let c = s.stamp(1, cmd(3), 1_000 + 200);
        assert_eq!(c.tick, 6);
        assert!(s.idle(1_000 + 333).is_none(), "tick 9");
        assert_eq!(s.idle(1_000 + 334).map(|c| c.tick), Some(10));
        s.stop();
        assert!(s.idle(1_000 + 1_000).is_none());
    }

    #[test]
    fn the_myth_ii_heartbeat_is_101_ms_whatever_the_ticks() {
        let mut s = Server::default();
        assert_eq!(s.kind(), GameKind::Myth2);
        s.start(0);
        assert!(s.idle(100).is_none(), "tick 3, but 100 ms");
        assert_eq!(s.idle(101).map(|c| c.tick), Some(3));
    }

    #[test]
    fn the_latency_is_smoothed_and_unknown_after_4_s() {
        let mut s = Server::default();
        let (t, l) = s.ping(1_001, &[0, 1], 0).unwrap();
        assert_eq!(l, [0, -1]);
        s.pong(1, t, 1_041);
        assert_eq!(s.latency(1), Some(40));
        let (t, _) = s.ping(2_002, &[0, 1], 0).unwrap();
        s.pong(1, t, 2_022);
        assert_eq!(s.latency(1), Some((20 + 120) / 4));
        assert!(s.ping(3_002, &[0, 1], 0).is_none());
        let (_, l) = s.ping(3_003, &[0, 1], 0).unwrap();
        assert_eq!(l, [0, 35]);
        s.ping(4_004, &[0, 1], 0);
        s.ping(5_005, &[0, 1], 0);
        let (_, l) = s.ping(6_006, &[0, 1], 0).unwrap();
        assert_eq!(l, [0, 35], "3984 ms since the pong");
        let (_, l) = s.ping(7_007, &[0, 1], 0).unwrap();
        assert_eq!(l, [0, -1]);
    }
}
