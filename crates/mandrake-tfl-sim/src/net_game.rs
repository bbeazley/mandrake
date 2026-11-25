use std::collections::VecDeque;
use std::path::Path;

use mandrake_rec::{CommandPacket, TickCommands};

use crate::{Stop, TflSim};

pub const GAME_SPEEDS: [i16; 10] = [0, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];
pub const NORMAL_SPEED_INDEX: i16 = 5;
pub const MAX_TICKS_PER_FRAME: [i16; 4] = [4, 30, 4, 4];
pub const NET_LEAD: i16 = 5;
pub const NET_LEAD_STEP: i16 = 2;
pub const NET_STARVATIONS: i16 = 12;
pub const NET_WINDOW: i32 = 600;
pub const SYNC_RESET: i32 = 0xaa;
pub const INCOMING_QUEUE_SIZE: i32 = 0x3000;
const COMMAND_HEADER: i32 = 8;
pub const SYNC: u8 = 1;
pub const CHAT: u8 = 9;
pub const DROP_PLAYER: u8 = 8;
pub const UNIT_ADJUSTMENT: u8 = 11;
pub const NOBODY: u8 = 0xff;

pub const MYTH_C: &str = ".\\Myth1Code\\myth.c";
pub const MONSTER_COMMANDS_C: &str = ".\\Myth1Code\\monster_commands.c";
pub const COMMANDS_C: &str = ".\\CoreLibrary\\myth\\commands.c";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfSync {
    pub file: &'static str,
    pub line: u32,
    pub tick: u32,
}

impl OutOfSync {
    pub fn line_text(&self) -> String {
        format!("OOS occurred at line {} in {}", self.line, self.file)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub player: i8,
    pub team_only: bool,
    pub text: Vec<u8>,
}

impl Chat {
    pub fn of_payload(player: i8, p: &[u8]) -> Self {
        Self {
            player,
            team_only: p.get(1).is_some_and(|f| f & 1 != 0),
            text: p
                .iter()
                .skip(2)
                .take(0x31)
                .take_while(|&&b| b != 0)
                .copied()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateStep {
    pub bodies: i32,
    pub requested: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetGate {
    pub mode: i16,
    pub local_time: i32,
    pub speed_index: i16,
    pub max_ticks: i16,
    pub remainder: i16,
    pub fill: bool,
    pub lead: i16,
    pub window_start: i16,
    pub starvations: i16,
}

impl NetGate {
    pub fn new_map(mode: i16) -> Self {
        Self {
            mode,
            local_time: 0,
            speed_index: NORMAL_SPEED_INDEX,
            max_ticks: MAX_TICKS_PER_FRAME[mode.clamp(0, 3) as usize],
            remainder: 0,
            fill: false,
            lead: NET_LEAD,
            window_start: 0,
            starvations: 0,
        }
    }

    fn speed(&self) -> i32 {
        i32::from(GAME_SPEEDS[self.speed_index.clamp(0, 9) as usize])
    }

    pub fn frame(&mut self, polls: i16, tick: u32, horizon: u32, paused: bool) -> Option<GateStep> {
        self.local_time += i32::from(polls);
        let total = self.speed() * i32::from(polls) + i32::from(self.remainder);
        let requested = i32::from((total >> 8) as i16);
        self.remainder = (total & 0xff) as i16;
        if requested < 1 {
            return None;
        }
        let mut available = horizon as i32 - tick as i32;
        let lead = i32::from(self.lead);
        if self.fill {
            if available < lead {
                available = 0;
            } else {
                self.fill = false;
            }
        }
        let bodies = if requested < available && lead <= available {
            if requested < available - lead {
                available - lead
            } else {
                requested
            }
        } else if available < requested {
            available
        } else {
            requested
        };
        if bodies == 0 && self.mode != 0 && !self.fill {
            if i32::from(self.window_start) + NET_WINDOW < self.local_time {
                self.window_start = self.local_time as i16;
                self.starvations = 0;
            }
            self.starvations += 1;
            if self.starvations >= NET_STARVATIONS {
                self.lead += NET_LEAD_STEP;
                self.window_start = self.local_time as i16;
                self.starvations = 0;
            }
            self.fill = true;
        }
        let cap = if self.speed_index < 6 {
            i32::from(self.max_ticks)
        } else {
            i32::from(((self.speed() * i32::from(self.max_ticks)) >> 8) as i16)
        };
        let mut bodies = bodies.min(cap);
        let mut requested = requested.min(cap);
        if paused {
            bodies = 0;
            requested = 0;
        }
        Some(GateStep { bodies, requested })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandQueue {
    size: i32,
    read: i32,
    write: i32,
    wrap: i32,
    records: VecDeque<CommandPacket>,
}

fn record_size(p: &CommandPacket) -> i32 {
    COMMAND_HEADER + p.raw_payload.len() as i32
}

impl CommandQueue {
    pub fn new(size: i32) -> Self {
        Self {
            size,
            read: 0,
            write: 0,
            wrap: 0,
            records: VecDeque::new(),
        }
    }

    pub fn push(&mut self, p: CommandPacket) -> bool {
        let len = record_size(&p);
        if self.read <= self.write && self.size < len + self.write {
            self.wrap = self.write;
            self.write = 0;
        }
        let free = if self.write < self.read {
            -self.write
        } else {
            self.size - self.write
        };
        if len < free + self.read {
            self.write += len;
            self.records.push_back(p);
            true
        } else {
            false
        }
    }

    pub fn pop(&mut self, tick: u32) -> Option<CommandPacket> {
        if self.write < self.read && self.wrap <= self.read {
            self.read = 0;
        }
        if self.read == self.write {
            return None;
        }
        let front = self.records.front()?;
        if front.tick as i32 > tick as i32 {
            return None;
        }
        self.read += record_size(front);
        self.records.pop_front()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetGame {
    pub local_net_id: i16,
    pub horizon: u32,
    pub sync_counter: i32,
    pub gate: NetGate,
    pub incoming: CommandQueue,
    pub outgoing: Vec<CommandPacket>,
    pub recorded: Vec<CommandPacket>,
    pub out_of_sync_flag: bool,
    pub out_of_sync: Vec<OutOfSync>,
    pub console_error: Option<i16>,
    pub syncs_checked: u32,
    pub chats: Vec<Chat>,
}

impl NetGame {
    pub fn new(local_net_id: i16, sync_counter: i32) -> Self {
        Self {
            local_net_id,
            horizon: 0,
            sync_counter,
            gate: NetGate::new_map(1),
            incoming: CommandQueue::new(INCOMING_QUEUE_SIZE),
            outgoing: Vec::new(),
            recorded: Vec::new(),
            out_of_sync_flag: false,
            out_of_sync: Vec::new(),
            console_error: None,
            syncs_checked: 0,
            chats: Vec::new(),
        }
    }

    pub fn take_chats(&mut self) -> Vec<Chat> {
        std::mem::take(&mut self.chats)
    }

    pub fn is_host(&self) -> bool {
        self.local_net_id == 0
    }

    pub fn sync_counter(&self) -> i32 {
        self.sync_counter
    }

    pub fn game_is_out_of_sync(&mut self, file: &'static str, line: u32, tick: u32) {
        const MAX_OUT_OF_SYNC_REPORTS: usize = 64;
        if self.console_error.is_none() {
            self.console_error = Some(0);
        }
        self.out_of_sync_flag = true;
        let report = OutOfSync { file, line, tick };
        tracing::error!("{} (game time {tick})", report.line_text());
        if self.out_of_sync.len() < MAX_OUT_OF_SYNC_REPORTS {
            self.out_of_sync.push(report);
        }
    }

    pub fn queue_incoming_command(&mut self, packet: CommandPacket) {
        let tick = packet.tick;
        if packet.command_type.to_u8() != 0 {
            self.recorded.push(packet.clone());
        }
        if !self.incoming.push(packet) && !self.out_of_sync_flag {
            self.game_is_out_of_sync(COMMANDS_C, 0xb4, tick);
        }
        if self.horizon as i32 <= tick as i32 {
            self.horizon = tick;
        }
    }

    pub fn queue_outgoing_command(&mut self, command_type: u8, raw_payload: &[u8]) {
        self.outgoing
            .push(CommandPacket::parse(0, NOBODY, command_type, raw_payload));
    }

    pub fn queue_unit_adjustment(&mut self, counts: &[i16]) {
        let mut p = vec![0u8, 0];
        p.extend_from_slice(&(counts.len() as i16).to_be_bytes());
        for c in counts {
            p.extend_from_slice(&c.to_be_bytes());
        }
        self.queue_outgoing_command(UNIT_ADJUSTMENT, &p);
    }

    pub fn take_outgoing_commands(&mut self) -> Vec<CommandPacket> {
        std::mem::take(&mut self.outgoing)
    }

    pub fn count_down_sync(&mut self, requested: i32, sample: (u32, u32)) {
        self.sync_counter -= requested;
        if self.sync_counter < 0 {
            if self.is_host() {
                let (tick, state) = sample;
                let mut p = [0u8; 12];
                p[4..8].copy_from_slice(&state.to_be_bytes());
                p[8..12].copy_from_slice(&tick.to_be_bytes());
                self.queue_outgoing_command(SYNC, &p);
            }
            self.sync_counter = SYNC_RESET;
        }
    }

    pub fn frame(
        &mut self,
        sim: &mut TflSim,
        polls: i16,
        paused: bool,
        poll: impl FnOnce(&mut NetGame),
    ) -> Result<Option<GateStep>, Stop> {
        let Some(step) = self.gate.frame(polls, sim.tick, self.horizon, paused) else {
            return Ok(None);
        };
        let sample = sample_of(sim);
        self.count_down_sync(step.requested, sample);
        poll(self);
        if (step.bodies < 0 || (self.horizon as i64) < sim.tick as i64 + i64::from(step.bodies))
            && !self.out_of_sync_flag
        {
            self.game_is_out_of_sync(MYTH_C, 0x333, sim.tick);
        }
        for _ in 0..step.bodies.max(0) {
            if sim.game_over() {
                break;
            }
            self.body(sim)?;
        }
        Ok(Some(step))
    }

    pub fn body(&mut self, sim: &mut TflSim) -> Result<(), Stop> {
        let t = sim.tick;
        if t & 0x1ff == 0 && t != 0 {
            sim.map_actions.game.sync_sample = Some((t, sim.rng.state()));
        }
        let players = sim.data.film.header.players.len();
        let mut commands = Vec::new();
        while let Some(c) = self.incoming.pop(t) {
            if c.tick != t {
                self.game_is_out_of_sync(MONSTER_COMMANDS_C, 0x5c, t);
            }
            let player = c.player_index as i8;
            let from_a_player = player == -1 || (player >= 0 && (player as usize) < players);
            match c.command_type.to_u8() {
                0 => {}
                SYNC if from_a_player => self.sync_command(sim, &c.raw_payload)?,
                CHAT if from_a_player => self.chats.push(Chat::of_payload(player, &c.raw_payload)),
                SYNC | CHAT => {}
                _ => commands.push(c),
            }
        }
        sim.data.film.tick_commands.clear();
        if !commands.is_empty() {
            sim.data
                .film
                .tick_commands
                .push(TickCommands { tick: t, commands });
        }
        sim.step()
    }

    fn sync_command(&mut self, sim: &TflSim, p: &[u8]) -> Result<(), Stop> {
        let (Some(state), Some(tick)) = (crate::be::u32_at(p, 4), crate::be::u32_at(p, 8)) else {
            return Err(Stop::new(
                crate::draw_site::SYNC_COMMAND_SHORT,
                "a short sync command",
            ));
        };
        self.syncs_checked += 1;
        let (game_tick, game_state) = sample_of(sim);
        if tick == game_tick && state != game_state && !self.out_of_sync_flag {
            self.game_is_out_of_sync(MONSTER_COMMANDS_C, 0x2d1, sim.tick);
        }
        Ok(())
    }
}

fn sample_of(sim: &TflSim) -> (u32, u32) {
    sim.map_actions
        .game
        .sync_sample
        .unwrap_or((0, sim.data.film.header.initial_seed))
}

impl TflSim {
    pub fn network_game(game_dir: &Path, film: &[u8]) -> Result<Self, Stop> {
        let mut sim = Self::set_up_as(game_dir, film, 1).map_err(|b| b.1)?;
        sim.data.film.tick_commands.clear();
        Ok(sim)
    }
}
