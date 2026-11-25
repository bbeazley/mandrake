use mandrake_common::game_options;
use mandrake_rec::{CommandPacket, CommandType, Recording};

use crate::Simulation;
use crate::game_state::FLAG_OUT_OF_SYNC;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetRole {
    Host,
    Joiner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfSync {
    pub file: &'static str,
    pub line: u32,
    pub tick: u32,
    pub seed: u32,
}

const SYSTEM_COMMAND_TYPES: u32 = 0x1a0_0001;
pub const FORCED_ENDGAME: u8 = 0x15;
pub const PAUSE: u8 = 0x17;
pub const GAME_SPEED: u8 = 0x18;
pub const UNIT_ADJUSTMENT: u8 = 12;

pub const DETACH: u8 = 11;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetachUnits {
    Nothing,
    To(i16),
    Choose(Vec<i16>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeRow {
    pub adjustable: bool,
    pub unit: mandrake_common::FourCC,
    pub placed: i16,
    pub minimum: i16,
    pub maximum: i16,
    pub cost: i16,
}

pub const NORMAL_SPEED_INDEX: i16 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    Paused { paused: bool, originator: bool },
    Speed { index: i16, originator: bool },
}

pub fn forced_end_line(kind: i16) -> Option<&'static str> {
    match kind {
        2 => Some("Rehosting..."),
        3 => Some("Restarting..."),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetGame {
    pub role: NetRole,
    pub horizon: u32,
    pub system_commands: std::collections::VecDeque<CommandPacket>,
    pub forced_end: i16,
    pub forced_end_received: bool,
    pub forced_end_sent: bool,
    pub game_speed: i16,
    pub events: Vec<SystemEvent>,
    pub loaded_from_network_save: bool,
}

impl NetGame {
    pub fn new(role: NetRole) -> Self {
        Self {
            role,
            horizon: 0,
            system_commands: std::collections::VecDeque::new(),
            forced_end: 0,
            forced_end_received: false,
            forced_end_sent: false,
            game_speed: NORMAL_SPEED_INDEX,
            events: Vec::new(),
            loaded_from_network_save: false,
        }
    }
}

impl Simulation {
    pub fn network_game(film: &[u8], role: NetRole) -> Result<Self, mandrake_common::M2Error> {
        let recording = Recording::from_bytes(film)?;
        if recording.header.mode != 1 {
            return Err(mandrake_common::M2Error::ReplayParse(format!(
                "a network game's block has mode 1, not {}",
                recording.header.mode
            )));
        }
        let (mut sim, _) = Self::live_game_of_film(film)?;
        sim.net = Some(NetGame::new(role));
        sim.sync_timer = crate::SYNC_TIMER_RESET;
        Ok(sim)
    }

    pub fn is_network_game(&self) -> bool {
        self.engine.mode == 1
    }

    pub fn net_horizon(&self) -> Option<u32> {
        self.net.as_ref().map(|n| n.horizon)
    }

    pub fn queue_incoming_command(&mut self, packet: CommandPacket) {
        let Some(net) = self.net.as_mut() else {
            return;
        };
        let kind = packet.command_type.to_u8();
        let tick = packet.tick;
        if kind != 0
            && !self.recording_ended
            && let Some(recorded) = self.recorded_commands.as_mut()
        {
            recorded.push(packet.clone());
        }
        if net.horizon <= tick {
            net.horizon = tick.wrapping_add(u32::from(kind == FORCED_ENDGAME));
        }
        if kind == FORCED_ENDGAME {
            net.forced_end_received = true;
        }
        if is_system_command(packet.command_type) {
            if kind != 0 {
                net.system_commands.push_back(packet);
            }
        } else {
            self.commands_by_tick.entry(tick).or_default().push(packet);
        }
    }

    pub fn end_recording(&mut self) {
        self.recording_ended = true;
    }

    pub fn command_player(&self, player: u8) -> u8 {
        self.map_command_player_index(player)
    }

    pub fn take_outgoing_commands(&mut self) -> Vec<CommandPacket> {
        if !self.is_network_game() {
            return Vec::new();
        }
        std::mem::take(&mut self.local_commands)
            .into_iter()
            .map(|c| crate::outgoing_packet(c).1)
            .collect()
    }

    pub fn net_end_of_batch(&mut self, requested: u32) {
        let Some(net) = self.net.as_ref() else {
            return;
        };
        let host = net.role == NetRole::Host;
        if !self.is_network_game() {
            return;
        }
        self.sync_timer -= requested as i32;
        if self.sync_timer < 0 {
            if host {
                self.queue_sync_command();
            }
            self.sync_timer = crate::SYNC_TIMER_RESET;
        }
    }

    pub fn game_is_out_of_sync(&mut self, file: &'static str, line: u32) {
        let report = OutOfSync {
            file,
            line,
            tick: self.current_tick,
            seed: self.rng.state,
        };
        tracing::error!(
            "OUT OF SYNC: OOS occurred at line {line} in {file} - game time: {} random seed: {}",
            report.tick,
            report.seed
        );
        {
            let w = &mut self.engine;
            w.game.flags |= FLAG_OUT_OF_SYNC;
        }
        self.out_of_sync.push(report);
    }

    pub fn is_out_of_sync(&self) -> bool {
        !self.out_of_sync.is_empty() || (self.engine.game.flags & FLAG_OUT_OF_SYNC != 0)
    }

    pub(crate) fn net_before_body(&mut self, t: u32) {
        let Some(net) = self.net.as_ref() else {
            return;
        };
        if !self.is_network_game() {
            return;
        }
        if t >= net.horizon && !self.is_out_of_sync() {
            self.game_is_out_of_sync("Myth2Code/myth.c", 0x433);
        }
    }

    pub fn net_paused(&self) -> bool {
        self.net.is_some() && (self.engine.game.flags & crate::game_state::FLAG_PAUSED != 0)
    }

    pub fn net_game_speed(&self) -> Option<i16> {
        self.net.as_ref().map(|n| n.game_speed)
    }

    pub fn force_network_endgame(&mut self, kind: i16) -> bool {
        let gameplay = self.engine.formation_globals.gameplay_version;
        let Some(net) = self.net.as_mut() else {
            return false;
        };
        if net.forced_end_sent {
            return false;
        }
        net.forced_end_sent = true;
        if gameplay <= game_options::GAMEPLAY_1_7_2 {
            return false;
        }
        let [hi, lo] = kind.to_be_bytes();
        let packet = CommandPacket::parse(0, 0xff, FORCED_ENDGAME, &[hi, lo, 0, 0]);
        self.local_commands
            .push(crate::QueuedCommand::Packet(packet));
        true
    }

    pub fn unit_trading(&mut self) -> Option<Vec<TradeRow>> {
        let w = &mut self.engine;
        let team = w.local_team();
        let captain = usize::try_from(team)
            .ok()
            .and_then(|t| w.players.teams.get(t))
            .map(|t| t.player);
        if w.game_options & mandrake_common::game_options::UNIT_TRADING == 0
            || w.game.state != 0
            || captain != Some(w.local_player)
        {
            return None;
        }
        let rows = w.placement_rows(team)?;
        let rows: Vec<TradeRow> = rows
            .into_iter()
            .map(|r| TradeRow {
                adjustable: r.adjustable,
                unit: mandrake_common::FourCC(r.row.0.to_be_bytes()),
                placed: r.placed,
                minimum: r.minimum,
                maximum: r.maximum,
                cost: r.cost,
            })
            .collect();
        (!rows.is_empty()).then_some(rows)
    }

    pub fn trading_time(&self) -> bool {
        self.engine.game.state == 0
    }

    pub fn queue_unit_adjustment(&mut self, counts: &[i16]) {
        let mut payload = vec![0, 0];
        payload.extend_from_slice(&(counts.len() as i16).to_be_bytes());
        for c in counts {
            payload.extend_from_slice(&c.to_be_bytes());
        }
        let packet = CommandPacket::parse(0, 0xff, UNIT_ADJUSTMENT, &payload);
        self.local_commands
            .push(crate::QueuedCommand::Packet(packet));
    }

    pub fn detach_units(&self, selection: &[usize]) -> DetachUnits {
        let w = &self.engine;
        let local = w.local_player;
        let owned = |m: &crate::engine_rng::EngineMonster| {
            m.owner == local && w.teams_are_allied(w.local_team(), m.team)
        };
        if selection
            .iter()
            .any(|&m| w.monsters.get(m).is_none_or(|m| !owned(m)))
        {
            return DetachUnits::To(local);
        }
        let team = w.local_team();
        let Some(t) = usize::try_from(team)
            .ok()
            .and_then(|t| w.players.teams.get(t))
        else {
            return DetachUnits::Nothing;
        };
        match t.players {
            1 => DetachUnits::Nothing,
            2 => DetachUnits::To(
                w.players
                    .players
                    .iter()
                    .enumerate()
                    .rfind(|&(i, p)| {
                        i as i16 != local && i16::from(p.team) == team && p.flags & 4 == 0
                    })
                    .map_or(-1, |(i, _)| i as i16),
            ),
            _ => DetachUnits::Choose(self.teammates()),
        }
    }

    pub fn teammates(&self) -> Vec<i16> {
        let w = &self.engine;
        let team = w.local_team();
        w.players
            .players
            .iter()
            .enumerate()
            .filter(|&(i, p)| i as i16 != w.local_player && i16::from(p.team) == team)
            .map(|(i, _)| i as i16)
            .collect()
    }

    pub fn queue_detach(&mut self, selection: &[usize], target: i16, look: bool) -> bool {
        let w = &self.engine;
        if selection.is_empty() || look {
            return false;
        }
        let max = if w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3 {
            0x7e
        } else {
            0x1e
        };
        let ids: Vec<i16> = selection
            .iter()
            .take(max)
            .filter_map(|&m| w.monsters.get(m).map(|m| m.identifier))
            .collect();
        let mut payload = vec![0, 0];
        payload.extend_from_slice(&target.to_be_bytes());
        payload.extend_from_slice(&(ids.len() as i16).to_be_bytes());
        for id in ids {
            payload.extend_from_slice(&id.to_be_bytes());
        }
        let packet = CommandPacket::parse(0, 0xff, DETACH, &payload);
        self.local_commands
            .push(crate::QueuedCommand::Packet(packet));
        true
    }

    pub fn take_system_events(&mut self) -> Vec<SystemEvent> {
        self.net
            .as_mut()
            .map(|n| std::mem::take(&mut n.events))
            .unwrap_or_default()
    }

    pub fn net_head(&mut self, filling: bool) {
        let Some(net) = self.net.as_ref() else {
            return;
        };
        if !self.is_network_game() || (self.engine.game.game_over) {
            return;
        }
        let horizon = net.horizon;
        if self.net_paused() || net.game_speed == 0 {
            self.execute_system_commands(horizon, true);
        } else if filling {
            self.execute_system_commands(self.current_tick, false);
        }
    }

    pub fn net_body_may_run(&mut self) -> bool {
        if !self.is_network_game() || self.net.is_none() {
            return true;
        }
        self.execute_system_commands(self.current_tick, false);
        !self.net_paused() && self.net_game_speed() != Some(0)
    }

    pub fn net_postgame_system_commands(&mut self) -> i16 {
        let Some(horizon) = self.net.as_ref().map(|n| n.horizon) else {
            return 0;
        };
        self.execute_system_commands(horizon, false);
        self.engine.game.command_reason
    }

    pub fn set_loaded_from_network_save(&mut self, on: bool) {
        if let Some(net) = self.net.as_mut() {
            net.loaded_from_network_save = on;
        }
    }

    pub fn restoring_from_saved_game(&self) -> bool {
        self.net
            .as_ref()
            .is_some_and(|n| n.loaded_from_network_save)
            && !self.save_loaded()
    }

    fn execute_system_commands(&mut self, t: u32, until_running: bool) {
        loop {
            let Some(net) = self.net.as_mut() else {
                return;
            };
            if net.system_commands.front().is_none_or(|c| c.tick > t) {
                return;
            }
            let Some(c) = net.system_commands.pop_front() else {
                return;
            };
            self.execute_system_command(&c);
            if until_running && !self.net_paused() && self.net_game_speed() != Some(0) {
                return;
            }
        }
    }

    fn execute_system_command(&mut self, c: &CommandPacket) {
        let kind = c.command_type.to_u8();
        if matches!(kind, PAUSE | GAME_SPEED) && self.restoring_from_saved_game() {
            return;
        }
        let word = c
            .raw_payload
            .get(..2)
            .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]));
        let originator = self.net.as_ref().is_some_and(|n| n.role == NetRole::Host);
        match kind {
            FORCED_ENDGAME => {
                let tick = self.current_tick;
                if let Some(net) = self.net.as_mut() {
                    if net.game_speed == 0 {
                        net.game_speed = NORMAL_SPEED_INDEX;
                    }
                    net.horizon = tick.wrapping_add(0x1e);
                }
                if word == 0 {
                    return;
                }
                if let Some(net) = self.net.as_mut() {
                    net.forced_end = word;
                }
                {
                    let w = &mut self.engine;
                    w.game.command_reason = word;
                    if w.game.shell_state == crate::game_state::SHELL_STATE_POSTGAME {
                        return;
                    }
                    w.game.suspend_end_game(false);
                    let r = w.game.end_game(1, tick as i32, &mut w.game_options);
                    if let Err(e) = r {
                        self.engine_stop.get_or_insert(e);
                    }
                }
            }
            GAME_SPEED => {
                if let Some(net) = self.net.as_mut() {
                    net.game_speed = word;
                    net.events.push(SystemEvent::Speed {
                        index: word.clamp(0, crate::game_speed::MAX_INDEX),
                        originator,
                    });
                }
            }
            PAUSE => {
                let paused = word != 0;
                {
                    let w = &mut self.engine;
                    if paused {
                        w.game.flags |= crate::game_state::FLAG_PAUSED;
                    } else {
                        w.game.flags &= !crate::game_state::FLAG_PAUSED;
                    }
                }
                if let Some(net) = self.net.as_mut() {
                    net.events.push(SystemEvent::Paused { paused, originator });
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn sync_differs(
    sync: &mandrake_rec::command::SyncCommand,
    snapshot: (u32, u32),
) -> bool {
    sync.snapshot_tick == snapshot.1 && sync.random_seed != snapshot.0
}

pub fn is_system_command(kind: CommandType) -> bool {
    let k = kind.to_u8();
    k < 0x19 && (SYSTEM_COMMAND_TYPES >> k) & 1 != 0
}

#[cfg(test)]
mod detach_tests {
    use super::*;
    use crate::engine_rng::{EngineMonster, EngineObject, EngineWorld, MonsterFormation};
    use crate::players::{Player, Team};
    use mandrake_common::FourCC;

    fn monster(index: usize, identifier: i16, owner: i16) -> EngineMonster {
        EngineMonster {
            index,
            identifier,
            first_draw: 0,
            definition: 0,
            object: EngineObject::new(index as i32, FourCC::new(*b"test")),
            mode: 1,
            anim_state: 0x1a,
            speed: 0,
            position: (0, 0),
            flags: 0,
            flags_8: 0,
            neighbours: 0,
            impassable: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team: 0,
            owner,
            facing: 0,
            formation: MonsterFormation::default(),
            cell_link: 0,
            flying_height: 0,
        }
    }

    fn game(players: usize, flags: &[u8], local: i16) -> Simulation {
        let mut w = EngineWorld::default();
        w.mode = 1;
        w.local_player = local;
        w.formation_globals.gameplay_version = 0x73c;
        w.players.players = (0..players)
            .map(|p| Player {
                flags: flags.get(p).copied().unwrap_or(0),
                field_09: 0,
                team: 0,
                field_0b: 0,
                id: p as i32,
            })
            .collect();
        w.players.teams = vec![Team {
            players: players as u8,
            player: 0,
            ..Default::default()
        }];
        w.monsters = vec![monster(0, 7, 0), monster(1, 9, 0), monster(2, 11, local)];
        let mut sim = Simulation::new(1);
        sim.set_engine_world(w);
        sim
    }

    #[test]
    fn a_team_of_one_does_nothing_and_two_give_to_the_teammate() {
        assert_eq!(game(1, &[], 0).detach_units(&[0, 1]), DetachUnits::Nothing);
        assert_eq!(game(2, &[], 0).detach_units(&[0, 1]), DetachUnits::To(1));
        assert_eq!(game(2, &[], 1).detach_units(&[2]), DetachUnits::To(0));
        assert_eq!(
            game(2, &[0, 4], 0).detach_units(&[0, 1]),
            DetachUnits::To(-1)
        );
    }

    #[test]
    fn three_or_more_ask_which_teammate_in_player_order() {
        let sim = game(3, &[0, 0, 4], 1);
        assert_eq!(sim.detach_units(&[2]), DetachUnits::Choose(vec![0, 2]));
        assert_eq!(sim.teammates(), vec![0, 2]);
        assert_eq!(
            game(4, &[], 2).detach_units(&[2]),
            DetachUnits::Choose(vec![0, 1, 3])
        );
    }

    #[test]
    fn a_unit_we_do_not_own_takes_the_selection_back() {
        let sim = game(3, &[], 1);
        assert_eq!(sim.detach_units(&[2, 0]), DetachUnits::To(1));
        let mut sim = game(3, &[], 1);
        sim.engine.monsters[2].team = 1;
        assert_eq!(sim.detach_units(&[2]), DetachUnits::To(1));
    }

    #[test]
    fn the_command_names_at_most_30_before_gameplay_0x515() {
        let mut sim = game(2, &[], 0);
        let w = &mut sim.engine;
        w.monsters = (0..0x90).map(|i| monster(i, i as i16 + 1, 0)).collect();
        let all: Vec<usize> = (0..0x90).collect();
        assert!(sim.queue_detach(&all, 1, false));
        sim.engine.formation_globals.gameplay_version = 0x514;
        assert!(sim.queue_detach(&all, 1, false));
        let out = sim.take_outgoing_commands();
        let count = |c: &CommandPacket| i16::from_be_bytes([c.raw_payload[4], c.raw_payload[5]]);
        assert_eq!((count(&out[0]), count(&out[1])), (0x7e, 0x1e));
        assert_eq!(out[0].raw_payload.len(), 6 + 2 * 0x7e);
    }
}

#[cfg(test)]
mod forced_end_tests {
    use super::*;
    use crate::engine_rng::EngineWorld;
    use crate::game_state::{FLAG_GAME_OVER, GameState, SHELL_STATE_IN_GAME, SHELL_STATE_POSTGAME};

    fn game(shell: i16, kind: i16) -> Simulation {
        let mut w = EngineWorld::default();
        w.mode = 1;
        w.game.shell_state = shell;
        let mut sim = Simulation::new(1);
        sim.set_engine_world(w);
        sim.current_tick = 500;
        sim.net = Some(NetGame::new(NetRole::Joiner));
        let [hi, lo] = kind.to_be_bytes();
        sim.queue_incoming_command(CommandPacket::parse(
            500,
            0,
            FORCED_ENDGAME,
            &[hi, lo, 0, 0],
        ));
        assert_eq!(sim.net_horizon(), Some(501));
        sim
    }

    #[test]
    fn in_game_the_forced_end_ends_the_game() {
        let mut sim = game(SHELL_STATE_IN_GAME, 2);
        sim.net_body_may_run();
        let g = &sim.engine.game;
        assert_eq!((g.command_reason, g.winner, g.end_code), (2, -1, 3));
        assert_eq!((g.state, g.timer, g.end_tick), (5, 1, 500));
        assert_ne!(g.flags & FLAG_GAME_OVER, 0);
        let net = sim.net.as_ref().unwrap();
        assert_eq!((net.forced_end, net.horizon), (2, 500 + 0x1e));
        assert!(sim.engine_stop.is_none());
    }

    #[test]
    fn in_the_postgame_the_forced_end_keeps_its_kind_and_ends_nothing() {
        let mut sim = game(SHELL_STATE_POSTGAME, 3);
        let before = sim.engine.game.clone();
        assert_eq!(sim.net_postgame_system_commands(), 3);
        let g = &sim.engine.game;
        assert_eq!(g.command_reason, 3);
        assert_eq!(
            GameState {
                command_reason: before.command_reason,
                ..g.clone()
            },
            before,
            "nothing else of the game block changes"
        );
        let net = sim.net.as_ref().unwrap();
        assert_eq!((net.forced_end, net.horizon), (3, 500 + 0x1e));
        assert!(net.system_commands.is_empty());
        let mut sim = game(SHELL_STATE_POSTGAME, 3);
        sim.net.as_mut().unwrap().system_commands.clear();
        assert_eq!(sim.net_postgame_system_commands(), 0);
    }

    #[test]
    fn the_game_over_puts_the_shell_in_the_postgame() {
        let mut g = GameState::new(0, 0);
        assert_eq!(g.shell_state, SHELL_STATE_IN_GAME);
        g.winner = 0;
        g.end_game(1, 100, &mut 0).unwrap();
        for t in 101..110 {
            if g.game_over {
                break;
            }
            let _ = g.end_of_tick_body(t, &mut 0);
        }
        assert!(g.game_over);
        assert_eq!(g.shell_state, SHELL_STATE_POSTGAME);
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;
    use crate::engine_rng::EngineWorld;
    use crate::game_state::{FLAG_PAUSED, FLAG_SAVE_LOADED};

    fn game(loaded_from_save: bool, save_loaded: bool) -> Simulation {
        let mut w = EngineWorld::default();
        w.mode = 1;
        if save_loaded {
            w.game.flags |= FLAG_SAVE_LOADED;
        }
        let mut sim = Simulation::new(1);
        sim.set_engine_world(w);
        sim.current_tick = 1327;
        sim.net = Some(NetGame::new(NetRole::Joiner));
        sim.set_loaded_from_network_save(loaded_from_save);
        sim.queue_incoming_command(CommandPacket::parse(1327, 0xff, PAUSE, &[0, 1, 0, 0]));
        sim.queue_incoming_command(CommandPacket::parse(1327, 0xff, GAME_SPEED, &[0, 0, 0, 0]));
        sim.net_body_may_run();
        sim
    }

    fn paused_and_speed(sim: &Simulation) -> (bool, i16) {
        let paused = sim.engine.game.flags & FLAG_PAUSED != 0;
        (paused, sim.net.as_ref().unwrap().game_speed)
    }

    #[test]
    fn a_saves_pause_and_speed_do_nothing_until_save_loaded_has_run() {
        let mut restoring = game(true, false);
        assert!(restoring.restoring_from_saved_game());
        assert_eq!(paused_and_speed(&restoring), (false, NORMAL_SPEED_INDEX));
        assert!(restoring.net_body_may_run());
        for (from_save, loaded) in [(false, false), (true, true)] {
            let sim = game(from_save, loaded);
            assert!(!sim.restoring_from_saved_game());
            assert_eq!(paused_and_speed(&sim), (true, 0));
        }
    }

    #[test]
    fn forced_end_clears_limited_visibility_in_engine_world() {
        let mut w = EngineWorld::default();
        w.mode = 1;
        w.game_options = mandrake_common::game_options::LIMITED_VISIBILITY;
        let mut sim = Simulation::new(1);
        sim.set_engine_world(w);
        sim.net = Some(NetGame::new(NetRole::Joiner));
        sim.queue_incoming_command(CommandPacket::parse(0, 0xff, FORCED_ENDGAME, &[0, 1, 0, 0]));
        sim.net_body_may_run();
        assert_eq!(
            sim.engine.game_options & mandrake_common::game_options::LIMITED_VISIBILITY,
            0
        );
        assert_eq!(sim.game_option_flags(), sim.engine.game_options);
    }

    #[test]
    fn incoming_game_speed_event_is_clamped_and_max_tick_wraps() {
        let mut w = EngineWorld::default();
        w.mode = 1;
        let mut sim = Simulation::new(1);
        sim.set_engine_world(w.clone());
        sim.net = Some(NetGame::new(NetRole::Joiner));
        sim.queue_incoming_command(CommandPacket::parse(
            u32::MAX,
            0xff,
            FORCED_ENDGAME,
            &[0, 0, 0, 0],
        ));
        assert_eq!(sim.net_horizon(), Some(0));

        let mut sim = Simulation::new(1);
        sim.set_engine_world(w);
        sim.net = Some(NetGame::new(NetRole::Joiner));
        sim.queue_incoming_command(CommandPacket::parse(
            0,
            0xff,
            GAME_SPEED,
            &99i16
                .to_be_bytes()
                .into_iter()
                .chain([0, 0])
                .collect::<Vec<_>>(),
        ));
        sim.net_body_may_run();
        let events = sim.take_system_events();
        assert!(events.iter().any(|e| matches!(
            e,
            SystemEvent::Speed { index, .. } if *index == crate::game_speed::MAX_INDEX
        )));
    }
}
