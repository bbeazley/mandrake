use mandrake_common::FourCC;
use mandrake_tag::mesh::MeshTag;

pub const GAME_SPEEDS: [i16; 12] = [0, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384];
pub const NORMAL_INDEX: i16 = 5;
pub const NORMAL_SPEED: i16 = GAME_SPEEDS[NORMAL_INDEX as usize];
pub const MAX_INDEX: i16 = 11;
const SOLO_FLOOR: i16 = 3;
const MAX_TICKS_PER_FRAME: i32 = 4;
pub const NET_MAX_TICKS_PER_FRAME: i32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Film,
    Solo,
    Network,
}

pub fn parse_speed(text: &str) -> Result<i16, String> {
    let wanted = text.strip_suffix('x').unwrap_or(text);
    let index = match wanted {
        "0" => 0,
        "1/16" => 1,
        "1/8" => 2,
        "1/4" => 3,
        "1/2" => 4,
        "1" => 5,
        "2" => 6,
        "4" => 7,
        "8" => 8,
        "16" => 9,
        "32" => 10,
        "64" => 11,
        _ => {
            return Err(format!(
                "`{text}` is not a game speed: use 0, 1/16, 1/8, 1/4, 1/2, 1, 2, 4, 8, 16, 32 or 64"
            ));
        }
    };
    Ok(index)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameSpeed {
    mode: GameMode,
    keys_locked: bool,
    index: i16,
    requested: i16,
    remainder: i32,
    buffered: i32,
    polls: i32,
    pending: Option<i16>,
}

impl Default for GameSpeed {
    fn default() -> Self {
        Self::new(GameMode::Solo, false, NORMAL_INDEX)
    }
}

impl GameSpeed {
    pub fn new(mode: GameMode, keys_locked: bool, index: i16) -> Self {
        let index = index.clamp(0, MAX_INDEX);
        Self {
            mode,
            keys_locked,
            index,
            requested: index,
            remainder: 0,
            buffered: 0,
            polls: 0,
            pending: None,
        }
    }

    pub fn set_network_index(&mut self, index: i16) {
        self.index = index.clamp(0, MAX_INDEX);
        self.pending = None;
    }

    pub fn pend_network_index(&mut self, index: i16) {
        if index != self.index {
            self.pending = Some(index.clamp(0, MAX_INDEX));
        }
    }

    pub fn for_map(mode: GameMode, mesh_id: FourCC, mesh_data: &[u8], index: i16) -> Self {
        let locked =
            mesh_id == FourCC(*b"00tm") || MeshTag::blocks_speed_keys_from_bytes(mesh_data);
        Self::new(mode, locked, index)
    }

    pub fn speed(&self) -> i16 {
        GAME_SPEEDS[self.index as usize]
    }

    pub fn polls(&self) -> i32 {
        self.polls
    }

    pub fn adjust(&mut self, faster: bool, shift: bool) -> Option<i16> {
        if self.keys_locked || self.mode == GameMode::Network {
            return None;
        }
        let current = self.index;
        let floor = match self.mode {
            GameMode::Film => 0,
            GameMode::Solo | GameMode::Network => SOLO_FLOOR,
        };
        let step = |to: i16| {
            if to >= floor {
                to.min(MAX_INDEX)
            } else {
                floor
            }
        };
        let new = if faster {
            if shift { MAX_INDEX } else { step(current + 1) }
        } else {
            let target = match (shift, self.mode) {
                (true, GameMode::Solo | GameMode::Network) => NORMAL_INDEX,
                (true, GameMode::Film) => floor,
                (false, _) => step(current - 1),
            };
            if current > floor { target } else { 0 }
        };
        if new == current {
            return None;
        }
        self.index = new;
        self.requested = new;
        Some(new)
    }

    pub fn adjust_as_coop_host(&mut self, faster: bool, shift: bool) -> Option<i16> {
        if self.keys_locked || self.mode != GameMode::Network {
            return None;
        }
        let mut solo = Self::new(GameMode::Solo, false, self.requested);
        let new = solo.adjust(faster, shift)?;
        self.requested = new;
        Some(new)
    }

    pub fn advance(&mut self, polls: u32, paused: bool) -> u32 {
        if polls == 0 {
            return 0;
        }
        self.polls = self.polls.wrapping_add(polls as i32);
        let total = i32::from(self.speed()) * polls as i32 + self.remainder;
        self.remainder = total & 0xff;
        let available = total >> 8;
        if paused || self.index == 0 || available < 1 {
            return 0;
        }
        let cap = if self.index > NORMAL_INDEX {
            (i32::from(self.speed()) * MAX_TICKS_PER_FRAME) >> 8
        } else {
            MAX_TICKS_PER_FRAME
        };
        let run = self.buffered.min(cap);
        self.buffered += available.min(cap) - run;
        run as u32
    }

    pub fn advance_network(
        &mut self,
        gate: &mut NetGate,
        polls: u32,
        paused: bool,
        tick: u32,
        horizon: u32,
    ) -> Option<GateStep> {
        if polls == 0 {
            if let Some(index) = self.pending.take() {
                self.index = index;
            }
            return None;
        }
        self.polls = self.polls.wrapping_add(polls as i32);
        let total = i32::from(self.speed()) * polls as i32 + self.remainder;
        self.remainder = total & 0xff;
        if let Some(index) = self.pending.take() {
            self.index = index;
        }
        let requested = i32::from((total >> 8) as i16);
        if requested < 1 {
            return None;
        }
        let available = horizon as i32 - tick as i32;
        let lead = i32::from(gate.lead);
        let mut bodies = if gate.fill && available < lead {
            0
        } else {
            gate.fill = false;
            if available <= requested {
                available
            } else if available < lead {
                requested
            } else {
                requested.max(available - lead)
            }
        };
        if !paused && self.index != 0 && bodies == 0 && !gate.first_body_pending && !gate.fill {
            gate.starve(self.polls);
        }
        let cap = if self.index > NORMAL_INDEX {
            (i32::from(self.speed()) * NET_MAX_TICKS_PER_FRAME) >> 8
        } else {
            NET_MAX_TICKS_PER_FRAME
        };
        if paused {
            return Some(GateStep {
                bodies: 0,
                requested: 0,
            });
        }
        bodies = bodies.clamp(0, cap);
        let requested = requested.min(cap) as u32;
        if self.index == 0 {
            bodies = 0;
        }
        if bodies > 0 {
            gate.first_body_pending = false;
        }
        Some(GateStep {
            bodies: bodies as u32,
            requested,
        })
    }
}

pub fn coop_host_may_send(server: bool, options: u32, gameplay_version: i32, team: i16) -> bool {
    server
        && options & mandrake_common::game_options::COOPERATIVE != 0
        && gameplay_version > mandrake_common::game_options::GAMEPLAY_1_8_4
        && team != -1
}

pub const NET_LEAD: i16 = 5;
pub const NET_LEAD_STEP: i16 = 2;
pub const NET_STARVATIONS: i16 = 12;
pub const NET_WINDOW: i32 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetGate {
    pub fill: bool,
    pub lead: i16,
    window_start: i16,
    starvations: i16,
    first_body_pending: bool,
}

impl Default for NetGate {
    fn default() -> Self {
        Self::new_map()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateStep {
    pub bodies: u32,
    pub requested: u32,
}

impl NetGate {
    pub fn new_map() -> Self {
        Self {
            fill: false,
            lead: NET_LEAD,
            window_start: 0,
            starvations: 0,
            first_body_pending: true,
        }
    }

    pub fn starvations(&self) -> i16 {
        self.starvations
    }

    fn starve(&mut self, local_time: i32) {
        if local_time > i32::from(self.window_start) + NET_WINDOW {
            self.starvations = 1;
            self.window_start = local_time as i16;
        } else {
            self.starvations += 1;
            if self.starvations >= NET_STARVATIONS {
                self.lead += NET_LEAD_STEP;
                self.starvations = 0;
                self.window_start = local_time as i16;
            }
        }
        self.fill = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(mode: GameMode, index: i16) -> GameSpeed {
        GameSpeed::new(mode, false, index)
    }

    #[test]
    fn steps_match_the_engine() {
        const FILM_UP: [i16; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 11];
        const FILM_DOWN: [i16; 12] = [0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        const SOLO_UP: [i16; 12] = [3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 11];
        const SOLO_DOWN: [i16; 12] = [0, 0, 0, 0, 3, 4, 5, 6, 7, 8, 9, 10];
        for i in 0..12i16 {
            let step = |mode, faster, shift| {
                let mut s = at(mode, i);
                s.adjust(faster, shift);
                s.index
            };
            let u = i as usize;
            assert_eq!(step(GameMode::Film, true, false), FILM_UP[u], "film up {i}");
            assert_eq!(
                step(GameMode::Film, false, false),
                FILM_DOWN[u],
                "film down {i}"
            );
            assert_eq!(step(GameMode::Solo, true, false), SOLO_UP[u], "solo up {i}");
            assert_eq!(
                step(GameMode::Solo, false, false),
                SOLO_DOWN[u],
                "solo down {i}"
            );
            assert_eq!(step(GameMode::Film, true, true), MAX_INDEX);
            assert_eq!(step(GameMode::Solo, true, true), MAX_INDEX);
            assert_eq!(step(GameMode::Film, false, true), 0);
            let solo_shift_down = if i > SOLO_FLOOR { NORMAL_INDEX } else { 0 };
            assert_eq!(step(GameMode::Solo, false, true), solo_shift_down);
        }
    }

    #[test]
    fn a_change_reports_the_index_and_no_change_reports_nothing() {
        let mut s = at(GameMode::Film, MAX_INDEX);
        assert_eq!(s.adjust(true, false), None);
        assert_eq!(s.adjust(false, false), Some(10));
    }

    #[test]
    fn the_training_map_and_the_mesh_flag_lock_the_keys() {
        let mut data = vec![0u8; 0x50];
        let mut plain = GameSpeed::for_map(GameMode::Solo, FourCC(*b"le3e"), &data, 5);
        assert_eq!(plain.adjust(true, false), Some(6));
        let mut training = GameSpeed::for_map(GameMode::Solo, FourCC(*b"00tm"), &data, 5);
        assert_eq!(training.adjust(true, false), None);
        data[0x4d] = 0x80;
        assert!(MeshTag::blocks_speed_keys_from_bytes(&data));
        let mut flagged = GameSpeed::for_map(GameMode::Film, FourCC(*b"le3e"), &data, 5);
        assert_eq!(flagged.adjust(false, false), None);
        assert_eq!(flagged.index, 5);
    }

    #[test]
    fn ticks_match_the_engine() {
        let mut s = at(GameMode::Film, 1);
        s.buffered = 1;
        let run = |s: &mut GameSpeed, index: i16, polls: u32, n: usize| {
            s.index = index;
            s.remainder = 0;
            (0..n).map(|_| s.advance(polls, false)).collect::<Vec<_>>()
        };
        assert_eq!(run(&mut s, 1, 1, 8), [0; 8]);
        assert_eq!(run(&mut s, 2, 1, 8), [0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(run(&mut s, 3, 1, 8), [0, 0, 0, 1, 0, 0, 0, 1]);
        assert_eq!(run(&mut s, 4, 1, 8), [0, 1, 0, 1, 0, 1, 0, 1]);
        assert_eq!(run(&mut s, 5, 1, 8), [1; 8]);
        assert_eq!(run(&mut s, 6, 1, 8), [1, 2, 2, 2, 2, 2, 2, 2]);
        assert_eq!(run(&mut s, 7, 1, 8), [2, 4, 4, 4, 4, 4, 4, 4]);
        assert_eq!(run(&mut s, 8, 1, 8), [4, 8, 8, 8, 8, 8, 8, 8]);
        assert_eq!(run(&mut s, 9, 1, 8), [8, 16, 16, 16, 16, 16, 16, 16]);
        assert_eq!(run(&mut s, 10, 1, 8), [16, 32, 32, 32, 32, 32, 32, 32]);
        assert_eq!(run(&mut s, 11, 1, 8), [32, 64, 64, 64, 64, 64, 64, 64]);
        assert_eq!(run(&mut s, 0, 1, 8), [0; 8]);
        assert_eq!(run(&mut s, 5, 8, 1), [4]);
        assert_eq!(run(&mut s, 6, 8, 1), [8]);
        assert_eq!(run(&mut s, 7, 8, 1), [16]);
        assert_eq!(run(&mut s, 10, 8, 1), [64]);
        assert_eq!(run(&mut s, 11, 8, 1), [128]);
        assert_eq!(run(&mut s, 4, 8, 1), [4]);
    }

    #[test]
    fn a_pause_runs_nothing_and_keeps_the_buffer() {
        let mut s = at(GameMode::Film, NORMAL_INDEX);
        s.advance(1, false);
        assert_eq!(s.advance(1, true), 0);
        assert_eq!(s.advance(1, false), 1);
        assert_eq!(s.polls(), 3);
    }

    #[test]
    fn thirty_two_x_runs_thirty_two_ticks_a_poll() {
        let mut s = at(GameMode::Film, parse_speed("32").unwrap());
        s.advance(1, false);
        assert_eq!(s.advance(1, false), 32);
    }

    struct FakeNet {
        speed: GameSpeed,
        gate: NetGate,
        tick: u32,
        horizon: u32,
    }

    impl FakeNet {
        fn new() -> Self {
            Self {
                speed: GameSpeed::new(GameMode::Network, false, NORMAL_INDEX),
                gate: NetGate::new_map(),
                tick: 0,
                horizon: 0,
            }
        }

        fn frame(&mut self, polls: u32) -> (u32, u32) {
            let step =
                self.speed
                    .advance_network(&mut self.gate, polls, false, self.tick, self.horizon);
            let step = step.unwrap_or(GateStep {
                bodies: 0,
                requested: 0,
            });
            self.tick += step.bodies;
            assert!(self.tick <= self.horizon, "never past the horizon");
            (step.bodies, step.requested)
        }

        fn starve(&mut self) {
            self.horizon = self.tick;
            assert_eq!(self.frame(1).0, 0);
        }
    }

    #[test]
    fn a_network_map_starts_with_a_lead_of_five_and_no_fill() {
        let gate = NetGate::new_map();
        assert_eq!((gate.fill, gate.lead, gate.starvations()), (false, 5, 0));
        let mut s = GameSpeed::new(GameMode::Network, false, NORMAL_INDEX);
        assert_eq!(s.adjust(true, false), None, "no speed keys in mode 1");
    }

    #[test]
    fn only_a_coop_host_above_0x731_and_not_an_observer_may_pause() {
        assert!(coop_host_may_send(true, 0x80, 0x732, 0));
        assert!(coop_host_may_send(true, 0x80 | 0x20, 0x8a2, 3));
        assert!(!coop_host_may_send(false, 0x80, 0x8a2, 0), "a joiner");
        assert!(!coop_host_may_send(true, 0x20, 0x8a2, 0), "no coop option");
        assert!(!coop_host_may_send(true, 0x80, 0x731, 0), "gameplay 0x731");
        assert!(!coop_host_may_send(true, 0x80, 0x8a2, -1), "an observer");
    }

    #[test]
    fn a_coop_hosts_speed_steps_floor_at_three_and_a_step_below_it_is_stopped() {
        const UP: [i16; 12] = [3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 11];
        const DOWN: [i16; 12] = [0, 0, 0, 0, 3, 4, 5, 6, 7, 8, 9, 10];
        for i in 0..12i16 {
            let u = i as usize;
            let step = |faster, shift| {
                let mut s = GameSpeed::new(GameMode::Network, false, i);
                let changed = s.adjust_as_coop_host(faster, shift);
                assert_eq!(s.index, i, "the running index waits for the command");
                assert_eq!(changed.is_some(), s.requested != i);
                s.requested
            };
            assert_eq!(step(true, false), UP[u], "up {i}");
            assert_eq!(step(false, false), DOWN[u], "down {i}");
            assert_eq!(step(true, true), MAX_INDEX, "shift up {i}");
            let shift_down = if i > SOLO_FLOOR { NORMAL_INDEX } else { 0 };
            assert_eq!(step(false, true), shift_down, "shift down {i}");
        }
        let mut s = GameSpeed::new(GameMode::Network, false, NORMAL_INDEX);
        assert_eq!(s.adjust_as_coop_host(false, false), Some(4));
        assert_eq!(s.adjust_as_coop_host(false, false), Some(3));
        assert_eq!(
            s.adjust_as_coop_host(false, false),
            Some(0),
            "below the floor: Stopped"
        );
        assert_eq!(s.adjust_as_coop_host(false, false), None);
        assert_eq!(s.adjust_as_coop_host(true, false), Some(3));
        assert_eq!(s.speed(), NORMAL_SPEED);
        s.set_network_index(3);
        assert_eq!(s.index, 3);
        assert_eq!(
            GameSpeed::new(GameMode::Network, true, 5).adjust_as_coop_host(true, false),
            None
        );
        assert_eq!(
            GameSpeed::new(GameMode::Solo, false, 5).adjust_as_coop_host(true, false),
            None
        );
    }

    #[test]
    fn the_gate_fills_to_the_lead_then_runs_what_the_polls_ask() {
        let mut n = FakeNet::new();
        assert_eq!(n.frame(1), (0, 1));
        assert!(!n.gate.fill);
        n.horizon = 3;
        assert_eq!(n.frame(1), (1, 1));
        assert_eq!(n.frame(2), (2, 2));
        assert_eq!(n.frame(1), (0, 1));
        assert!(n.gate.fill);
        assert_eq!(n.gate.starvations(), 1);
        n.horizon = 7;
        assert_eq!(n.frame(1), (0, 1));
        assert!(n.gate.fill, "a fill is no starvation");
        assert_eq!(n.gate.starvations(), 1);
        n.horizon = 8;
        assert_eq!(n.frame(1), (1, 1));
        assert!(!n.gate.fill);
        assert_eq!(n.frame(1), (1, 1));
        assert_eq!(n.tick, 5);
    }

    #[test]
    fn the_gate_catches_up_keeping_the_lead_at_most_thirty_a_frame() {
        let mut n = FakeNet::new();
        n.horizon = 100;
        assert_eq!(n.frame(1), (30, 1));
        assert_eq!(n.frame(1), (30, 1));
        assert_eq!(n.frame(1), (30, 1));
        assert_eq!(n.frame(1), (5, 1));
        assert_eq!(n.tick, 95);
        n.horizon = 103;
        assert_eq!(n.frame(6), (6, 6));
        assert_eq!(n.frame(6), (2, 6), "no more than the horizon");
        n.horizon = 200;
        assert_eq!(n.frame(40), (30, 30));
    }

    #[test]
    fn the_twelfth_starvation_in_a_window_raises_the_lead_by_two() {
        let mut n = FakeNet::new();
        n.horizon = 1;
        assert_eq!(n.frame(1), (1, 1));
        for count in 1..=11 {
            n.starve();
            assert_eq!(n.gate.starvations(), count);
            assert_eq!(n.gate.lead, 5);
            n.horizon = n.tick + u32::from(n.gate.lead as u16);
            assert_eq!(n.frame(1), (1, 1));
        }
        n.starve();
        assert_eq!((n.gate.lead, n.gate.starvations()), (7, 0));
        n.horizon = n.tick + 6;
        assert_eq!(n.frame(1), (0, 1));
        n.horizon = n.tick + 7;
        assert_eq!(n.frame(1), (1, 1));
        n.starve();
        assert_eq!((n.gate.lead, n.gate.starvations()), (7, 1));
    }

    #[test]
    fn a_starvation_after_six_hundred_polls_starts_a_new_window() {
        let mut n = FakeNet::new();
        n.horizon = 1;
        n.frame(1);
        for _ in 0..10 {
            n.starve();
            n.horizon = n.tick + 5;
            n.frame(1);
        }
        assert_eq!(n.gate.starvations(), 10);
        for _ in 0..600 {
            n.horizon = n.tick + 5;
            assert_eq!(n.frame(1), (1, 1));
        }
        assert!(n.speed.polls() > NET_WINDOW);
        n.starve();
        assert_eq!((n.gate.lead, n.gate.starvations()), (5, 1));
    }

    #[test]
    fn a_speed_from_the_head_counts_the_frames_polls_at_the_old_speed() {
        let mut n = FakeNet::new();
        n.speed.set_network_index(0);
        n.horizon = 40;
        assert_eq!(
            n.speed
                .advance_network(&mut n.gate, 3, false, n.tick, n.horizon),
            None
        );
        n.speed.pend_network_index(NORMAL_INDEX);
        assert_eq!(
            n.speed
                .advance_network(&mut n.gate, 3, false, n.tick, n.horizon),
            None
        );
        assert_eq!(n.speed.speed(), NORMAL_SPEED);
        assert_eq!(n.frame(1), (30, 1));
        n.speed.set_network_index(7);
        n.horizon = n.tick + 20;
        assert_eq!(
            n.frame(1),
            (15, 4),
            "4x: four requested, fifteen to keep the lead"
        );
    }

    #[test]
    fn a_pause_runs_no_tick_and_costs_the_sync_timer_nothing() {
        let mut s = GameSpeed::new(GameMode::Network, false, NORMAL_INDEX);
        let mut gate = NetGate::new_map();
        let step = s.advance_network(&mut gate, 1, true, 0, 50);
        assert_eq!(
            step,
            Some(GateStep {
                bodies: 0,
                requested: 0
            })
        );
        assert_eq!(s.advance_network(&mut gate, 0, false, 0, 50), None);
        assert_eq!(s.polls(), 1);
    }

    #[test]
    fn parse_speed_takes_the_multiples() {
        assert_eq!(parse_speed("32x"), Ok(10));
        assert_eq!(parse_speed("1"), Ok(NORMAL_INDEX));
        assert_eq!(parse_speed("1/16"), Ok(1));
        assert_eq!(parse_speed("64"), Ok(MAX_INDEX));
        assert!(parse_speed("3").is_err());
    }

    #[test]
    fn new_clamps_out_of_range_speed_index() {
        let low = GameSpeed::new(GameMode::Network, false, -5);
        assert_eq!(low.speed(), GAME_SPEEDS[0]);
        let high = GameSpeed::new(GameMode::Network, false, 99);
        assert_eq!(high.speed(), GAME_SPEEDS[MAX_INDEX as usize]);
    }
}
