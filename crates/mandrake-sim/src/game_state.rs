use crate::draw_site;
use crate::engine_rng::{EngineRng, EngineWorld, Unmodelled};

pub fn clear_limited_visibility_if_applicable(options: &mut u32) {
    if *options & mandrake_common::game_options::COOPERATIVE == 0 {
        *options &= !mandrake_common::game_options::LIMITED_VISIBILITY;
    }
}

pub const FLAG_END_REQUESTED: u32 = 1;
pub const FLAG_GAME_OVER: u32 = 2;
pub const FLAG_END_SUSPENDED: u32 = 4;
pub const FLAG_OUT_OF_SYNC: u32 = 8;
pub const FLAG_SAVE_LOADED: u32 = 0x80;
pub const FLAG_PAUSED: u32 = 0x20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameEnd {
    pub tick: i32,
    pub winner: i16,
    pub code: i16,
}

pub const GAME_SOUND_5_MINUTES: u16 = 0;
pub const GAME_SOUND_1_MINUTE: u16 = 1;
pub const GAME_SOUND_30_SECONDS: u16 = 2;
pub const GAME_SOUND_ABOUT_TO_WIN: u16 = 3;
pub const GAME_SOUND_END_REQUESTED: u16 = 5;
pub const GAME_SOUND_GAME_OVER: u16 = 6;
pub const GAME_SOUND_CASUALTY: u16 = 7;
pub const GAME_SOUND_PLAYER_ELIMINATED: u16 = 9;
pub const GAME_SOUND_UNIT_LOST: u16 = 0x13;
pub const GAME_SOUND_UNIT_GAINED: u16 = 0x14;
pub const GAME_SOUND_PREGAME_OVER: u16 = 0x19;

pub const SHELL_STATE_IN_GAME: i16 = 3;
pub const SHELL_STATE_POSTGAME: i16 = 0xb;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameState {
    pub shell_state: i16,
    pub state: i16,
    pub timer: i32,
    pub game_time: i32,
    pub flags: u32,
    pub winner: i16,
    pub end_code: i16,
    pub end_tick: i32,
    pub command_reason: i16,
    pub final_statistics: bool,
    pub carnage_report_open: bool,
    pub game_over: bool,
    pub cached_end_code: Option<i16>,
    pub standings_leader: Option<i16>,
    pub game_sound_requests: Vec<u16>,
}

impl Default for GameState {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl GameState {
    pub fn new(time_limit: i32, planning_time: i32) -> Self {
        let (state, timer, game_time) = if time_limit <= 0 {
            (1, 0, 0)
        } else if planning_time <= 0 {
            (2, time_limit, time_limit)
        } else {
            (0, planning_time, time_limit)
        };
        Self {
            shell_state: SHELL_STATE_IN_GAME,
            state,
            timer,
            game_time,
            flags: 0,
            winner: -1,
            end_code: 0,
            end_tick: 0,
            command_reason: 0,
            final_statistics: false,
            carnage_report_open: false,
            game_over: false,
            cached_end_code: None,
            standings_leader: None,
            game_sound_requests: Vec::new(),
        }
    }

    pub fn suspend_end_game(&mut self, suspend: bool) {
        if suspend {
            self.flags |= FLAG_END_SUSPENDED;
        } else {
            self.flags &= !FLAG_END_SUSPENDED;
        }
    }

    pub fn end_game(&mut self, edi: i32, tick: i32, options: &mut u32) -> Result<(), Unmodelled> {
        if self.flags & FLAG_END_SUSPENDED != 0 {
            self.flags |= FLAG_END_REQUESTED;
            if self.state < 4 {
                self.state = 4;
                self.timer = 1;
                self.game_sound_requests.push(GAME_SOUND_END_REQUESTED);
            }
            return Ok(());
        }
        let r = self.finalize_winning_team();
        clear_limited_visibility_if_applicable(options);
        self.end_code = 3;
        self.flags |= FLAG_GAME_OVER;
        self.end_tick = tick;
        if self.state < 5 {
            self.state = 5;
            self.timer = if edi == 0 { 300 } else { 1 };
            self.game_sound_requests.push(GAME_SOUND_GAME_OVER);
        }
        r
    }

    pub fn finalize_winning_team(&mut self) -> Result<(), Unmodelled> {
        if self.command_reason != 0 {
            self.winner = -1;
            return Ok(());
        }
        if self.winner != -1 {
            return Ok(());
        }
        if let Some(leader) = self.standings_leader {
            self.winner = leader;
        }
        Ok(())
    }

    pub fn update_player_statistics(&mut self, game_mode: i16) {
        if self.flags & FLAG_GAME_OVER == 0 || self.final_statistics {
            return;
        }
        self.final_statistics = true;
        if game_mode == 0 {
            self.carnage_report_open = true;
            self.flags |= FLAG_PAUSED;
        }
    }

    pub fn close_carnage_report(&mut self) {
        if self.carnage_report_open {
            self.carnage_report_open = false;
            self.flags &= !FLAG_PAUSED;
        }
    }

    pub fn end_of_tick_body(
        &mut self,
        tick: i32,
        options: &mut u32,
    ) -> (TimerEvent, Result<(), Unmodelled>) {
        self.timer = self.timer.wrapping_sub(1);
        if self.timer != 0 {
            if self.state == 2 {
                let warning = match self.timer {
                    9000 => Some(GAME_SOUND_5_MINUTES),
                    0x708 => Some(GAME_SOUND_1_MINUTE),
                    900 => Some(GAME_SOUND_30_SECONDS),
                    _ => None,
                };
                self.game_sound_requests.extend(warning);
            }
            return (TimerEvent::None, Ok(()));
        }
        match self.state {
            0 => {
                self.state = 2;
                self.timer = self.game_time;
                self.game_sound_requests.push(GAME_SOUND_PREGAME_OVER);
                (TimerEvent::PreGameEnded, Ok(()))
            }
            2 | 3 => (TimerEvent::None, self.end_game(0, tick, options)),
            4 => {
                self.timer = 1;
                (TimerEvent::None, self.end_game(0, tick, options))
            }
            5 => {
                self.cached_end_code = Some(self.end_code);
                self.game_over = true;
                self.shell_state = SHELL_STATE_POSTGAME;
                (TimerEvent::None, Ok(()))
            }
            _ => (TimerEvent::None, Ok(())),
        }
    }

    pub fn ticks_stopped(&self) -> bool {
        self.game_over || self.flags & FLAG_PAUSED != 0
    }

    pub fn end(&self) -> Option<GameEnd> {
        (self.flags & FLAG_GAME_OVER != 0).then_some(GameEnd {
            tick: self.end_tick,
            winner: self.winner,
            code: self.end_code,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerEvent {
    None,
    PreGameEnded,
}

impl EngineWorld {
    pub fn eliminate_team(
        &mut self,
        team: i16,
        rng: Option<&mut EngineRng>,
    ) -> Result<(), Unmodelled> {
        if self.game.flags & FLAG_GAME_OVER != 0 {
            return Ok(());
        }
        let Some(record) = usize::try_from(team)
            .ok()
            .and_then(|t| self.players.teams.get_mut(t))
        else {
            return Ok(());
        };
        if record.flags & 1 != 0 || record.players == 0 {
            return Ok(());
        }
        record.flags |= 1;
        let sound = GAME_SOUND_PLAYER_ELIMINATED + u16::from(record.players != 1);
        self.game.game_sound_requests.push(sound);
        let units = self.eliminate_units_of_losing_team(team, rng);
        if team == self.local_team() {
            clear_limited_visibility_if_applicable(&mut self.game_options);
        }
        units
    }

    fn eliminate_units_of_losing_team(
        &mut self,
        team: i16,
        rng: Option<&mut EngineRng>,
    ) -> Result<(), Unmodelled> {
        let Some(creation) = self.marker_creation.clone() else {
            return Ok(());
        };
        let mesh = &creation.mesh;
        let to_eliminate: Vec<usize> = (0..self.monsters.len())
            .filter(|&i| {
                let m = &self.monsters[i];
                if m.identifier == 0 || m.team != team {
                    return false;
                }
                let palette = mesh
                    .markers
                    .iter()
                    .find(|k| k.marker_type == 3 && k.marker_id == m.identifier)
                    .and_then(|k| mesh.palette_for_marker(k));
                palette.is_some_and(|p| p.flags & 0x10 == 0)
            })
            .collect();

        if to_eliminate.is_empty() {
            return Ok(());
        }
        let Some(rng) = rng else {
            return Err(Unmodelled::new(
                draw_site::ELIMINATE_WITHOUT_RNG,
                format!("eliminate_team {team}: living units without RNG"),
            ));
        };

        let mut actions = self.map_actions.take();
        for m in to_eliminate {
            if self.monsters[m].identifier == 0 {
                continue;
            }
            crate::engine_rng::WorldHooks {
                world: self,
                actions: &mut actions,
                rng,
                monster: m,
                time: 0,
                host: &mut crate::engine_attack::NoHost,
            }
            .kill_monster_code_28()?;
        }
        self.map_actions = actions;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_warnings_and_the_planning_time_ask_for_sounds() {
        let mut g = GameState::new(0, 0);
        g.state = 2;
        for (timer, id) in [(9001, 0), (0x709, 1), (901, 2)] {
            g.timer = timer;
            g.end_of_tick_body(0, &mut 0).1.unwrap();
            assert_eq!(std::mem::take(&mut g.game_sound_requests), [id]);
            g.end_of_tick_body(0, &mut 0).1.unwrap();
            assert!(g.game_sound_requests.is_empty());
        }
        g.state = 0;
        g.timer = 1;
        assert_eq!(g.end_of_tick_body(0, &mut 0).0, TimerEvent::PreGameEnded);
        assert_eq!(g.game_sound_requests, [GAME_SOUND_PREGAME_OVER]);
    }

    #[test]
    fn the_end_asks_once_for_each_sound() {
        let mut g = GameState::new(0, 0);
        g.winner = 1;
        g.suspend_end_game(true);
        g.end_game(0, 10, &mut 0).unwrap();
        g.end_game(0, 11, &mut 0).unwrap();
        assert_eq!(g.game_sound_requests, [GAME_SOUND_END_REQUESTED]);
        g.suspend_end_game(false);
        g.end_game(0, 12, &mut 0).unwrap();
        g.end_game(0, 13, &mut 0).unwrap();
        assert_eq!(
            g.game_sound_requests,
            [GAME_SOUND_END_REQUESTED, GAME_SOUND_GAME_OVER]
        );
    }

    #[test]
    fn end_game_sets_state_5_and_keeps_the_winner() {
        let mut g = GameState::new(0, 0);
        assert_eq!((g.state, g.timer, g.winner, g.end_code), (1, 0, -1, 0));
        g.winner = 0;
        g.end_game(0, 23712, &mut 0).unwrap();
        assert_eq!((g.state, g.timer, g.flags), (5, 300, FLAG_GAME_OVER));
        assert_eq!((g.winner, g.end_code, g.end_tick), (0, 3, 23712));
    }

    #[test]
    fn edi_1_ends_with_timer_1() {
        let mut g = GameState::new(0, 0);
        g.winner = 1;
        g.end_game(1, 5, &mut 0).unwrap();
        assert_eq!((g.state, g.timer), (5, 1));
    }

    #[test]
    fn a_suspended_end_waits() {
        let mut g = GameState::new(0, 0);
        g.winner = 1;
        g.suspend_end_game(true);
        g.end_game(0, 10, &mut 0).unwrap();
        assert_eq!((g.state, g.timer), (4, 1));
        assert_eq!(g.flags, FLAG_END_SUSPENDED | FLAG_END_REQUESTED);
        assert_eq!(g.end_of_tick_body(11, &mut 0).0, TimerEvent::None);
        assert_eq!((g.state, g.timer), (4, 1));
        g.suspend_end_game(false);
        g.end_of_tick_body(12, &mut 0).1.unwrap();
        assert_eq!((g.state, g.timer, g.end_tick), (5, 300, 12));
    }

    #[test]
    fn no_rebuild_yet_ends_without_a_winner() {
        let mut g = GameState::new(0, 0);
        g.end_game(0, 3, &mut 0).unwrap();
        assert_eq!((g.state, g.timer, g.winner), (5, 300, -1));
    }

    #[test]
    fn finalize_winning_team_branches() {
        let mut g = GameState::new(0, 0);
        g.finalize_winning_team().unwrap();
        assert_eq!(g.winner, -1);
        g.standings_leader = Some(-1);
        g.finalize_winning_team().unwrap();
        assert_eq!(g.winner, -1);
        g.standings_leader = Some(2);
        g.finalize_winning_team().unwrap();
        assert_eq!(g.winner, 2);
        g.standings_leader = Some(3);
        g.finalize_winning_team().unwrap();
        assert_eq!(g.winner, 2);
        g.command_reason = 1;
        g.finalize_winning_team().unwrap();
        assert_eq!(g.winner, -1);
    }

    #[test]
    fn a_command_reason_clears_the_winner() {
        let mut g = GameState::new(0, 0);
        g.winner = 1;
        g.command_reason = 2;
        g.end_game(1, 3, &mut 0).unwrap();
        assert_eq!(g.winner, -1);
    }

    #[test]
    fn timed_game_states() {
        assert_eq!(GameState::new(900, 0).state, 2);
        assert_eq!(GameState::new(900, 0).timer, 900);
        let mut g = GameState::new(900, 30);
        assert_eq!((g.state, g.timer), (0, 30));
        for t in 1..30 {
            assert_eq!(g.end_of_tick_body(t, &mut 0).0, TimerEvent::None);
        }
        assert_eq!(g.end_of_tick_body(30, &mut 0).0, TimerEvent::PreGameEnded);
        assert_eq!((g.state, g.timer), (2, 900));
    }

    #[test]
    fn game_over_comes_300_tick_bodies_later() {
        let mut g = GameState::new(0, 0);
        g.timer = -12608;
        g.winner = 1;
        g.end_game(0, 12608, &mut 0).unwrap();
        for tick in 12609..12908 {
            g.update_player_statistics(2);
            g.end_of_tick_body(tick, &mut 0).1.unwrap();
            assert!(!g.ticks_stopped(), "{tick}");
        }
        g.end_of_tick_body(12908, &mut 0).1.unwrap();
        assert!(g.game_over);
        assert_eq!(g.cached_end_code, Some(3));
    }

    #[test]
    fn a_replay_pauses_in_the_tick_body_of_the_end() {
        let mut g = GameState::new(0, 0);
        g.winner = 0;
        g.end_game(0, 23712, &mut 0).unwrap();
        g.update_player_statistics(0);
        assert_eq!(g.flags, 0x22);
        assert!(g.carnage_report_open);
        g.end_of_tick_body(23713, &mut 0).1.unwrap();
        assert_eq!(g.timer, 299);
        assert!(g.ticks_stopped());
        g.close_carnage_report();
        assert!(!g.ticks_stopped());
        assert_eq!(g.flags, 2);
        g.update_player_statistics(0);
        assert!(!g.carnage_report_open);
    }

    #[test]
    fn a_solo_game_does_not_pause() {
        let mut g = GameState::new(0, 0);
        g.winner = 1;
        g.end_game(0, 300, &mut 0).unwrap();
        g.update_player_statistics(2);
        assert_eq!(g.flags, FLAG_GAME_OVER);
        assert!(!g.carnage_report_open);
    }

    #[test]
    fn end_game_clears_option_4_unless_option_0x80() {
        for (before, after) in [(0x200c_40fc, 0x200c_40fc), (0x200c_407c, 0x200c_4078)] {
            let mut g = GameState::new(0, 0);
            g.winner = 0;
            let mut options = before;
            g.end_game(0, 61, &mut options).unwrap();
            assert_eq!(options, after);
        }
    }

    #[test]
    fn a_suspended_end_keeps_option_4() {
        let mut g = GameState::new(0, 0);
        g.winner = 0;
        g.suspend_end_game(true);
        let mut options = 0x7c;
        g.end_game(0, 10, &mut options).unwrap();
        assert_eq!(options, 0x7c);
        g.suspend_end_game(false);
        g.end_of_tick_body(11, &mut options).1.unwrap();
        assert_eq!(options, 0x78);
    }

    #[test]
    fn no_winner_still_clears_option_4() {
        let mut g = GameState::new(0, 0);
        let mut options = 4;
        g.end_game(0, 10, &mut options).unwrap();
        assert_eq!(options, 0);
    }
}
