#[path = "tfl_live_film.rs"]
pub(crate) mod film;
#[path = "tfl_live_save.rs"]
pub(crate) mod save;

use crate::tfl_game::TflReplay;
use crate::tfl_map::TflMap;
use film::SoloGame;
use mandrake_common::FourCC;
use mandrake_rec::{CommandPacket, TickCommands};
use mandrake_tfl_sim::Stop;
use std::path::{Path, PathBuf};
use tracing::info;

pub(crate) const LOCAL_PLAYER: u8 = 0;
pub(crate) const LAST_RECORDING_NAME: &[u8] = b"Last Recording";
pub(crate) const LAST_RECORDING_ID: FourCC = FourCC(*b"lare");

pub(crate) fn our_films_dir() -> Option<PathBuf> {
    films_dir_in(crate::user_dirs::config_dir())
}

pub(crate) fn films_dir_in(config: Option<PathBuf>) -> Option<PathBuf> {
    config.map(|d| d.join("tfl").join("RECO"))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TflCarry {
    pub(crate) veterans: Vec<mandrake_tfl_sim::MonsterInitializer>,
    pub(crate) permanent: Vec<mandrake_tfl_sim::statistics::PermanentRanking>,
}

impl TflCarry {
    pub(crate) fn at_map_end(sim: &mut mandrake_tfl_sim::TflSim) -> Result<Self, Stop> {
        sim.finish_map()?;
        Ok(Self {
            veterans: sim.monster_initializers().to_vec(),
            permanent: sim.statistics.permanent.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LiveScript {
    pub(crate) mesh: FourCC,
    pub(crate) difficulty: i16,
    pub(crate) seed: u32,
    commands: Vec<CommandPacket>,
}

impl LiveScript {
    pub(crate) fn of_film(bytes: &[u8]) -> Option<Self> {
        let rec = mandrake_rec::tfl::TflRecording::from_bytes(bytes).ok()?;
        let d = u32::from_be_bytes(bytes.get(0x30..0x34)?.try_into().ok()?) as usize;
        let at = |o: usize| bytes.get(d + 0x24 + o..d + 0x24 + o + 4);
        Some(Self {
            mesh: FourCC(at(0xc)?.try_into().ok()?),
            difficulty: mandrake_tfl_sim::film_difficulty(bytes).ok()?,
            seed: u32::from_be_bytes(at(0x18)?.try_into().ok()?),
            commands: rec
                .tick_commands
                .into_iter()
                .flat_map(|t| t.commands)
                .collect(),
        })
    }

    pub(crate) fn from_env(mesh: FourCC) -> Option<Self> {
        let path = std::env::var_os("M2_TFL_LIVE_SCRIPT")?;
        let script = Self::of_film(&std::fs::read(&path).ok()?)?;
        (script.mesh == mesh).then(|| {
            println!(
                "TFL game: M2_TFL_LIVE_SCRIPT: {} commands of {} given live",
                script.commands.len(),
                PathBuf::from(&path).display()
            );
            script
        })
    }
}

pub(crate) struct TflLive {
    script: std::collections::VecDeque<CommandPacket>,
    pub(crate) game: SoloGame,
    veterans: Vec<u8>,
    permanent: Vec<mandrake_tfl_sim::statistics::PermanentRanking>,
    tick: u32,
    stop: Option<Stop>,
    outgoing: Vec<CommandPacket>,
    recorded: Vec<TickCommands>,
    said: Vec<u8>,
    obmo_modes: std::collections::VecDeque<(u32, i32)>,
    films: Option<PathBuf>,
    written_at: Option<u32>,
    restored_ambient_volumes: Vec<(i16, i16)>,
}

pub(crate) const CHECKPOINT_TICKS: u32 = crate::app::recording::CHECKPOINT_TICKS;

pub(crate) fn obmo_modes_from_env() -> Vec<(u32, i32)> {
    std::env::var("M2_TFL_OBMO_MODES")
        .map(|v| parse_obmo_modes(&v))
        .unwrap_or_default()
}

fn parse_obmo_modes(v: &str) -> Vec<(u32, i32)> {
    let mut out: Vec<(u32, i32)> = v
        .split(',')
        .filter_map(|p| {
            let (t, m) = p.trim().split_once(':')?;
            Some((t.parse().ok()?, m.parse().ok()?))
        })
        .collect();
    out.sort_by_key(|&(t, _)| t);
    out
}

pub(crate) fn trigger_obmo_with_mode(sim: &mut mandrake_tfl_sim::TflSim, mode: i32) -> Option<i16> {
    let actions = &mut sim.map_actions.actions;
    let i = actions
        .iter()
        .position(|a| a.identifier != 0 && &a.tag.to_be_bytes() == b"obmo")?;
    let p = actions[i]
        .parameters
        .iter_mut()
        .find(|p| &p.name.0 == b"mode" && p.kind == 5 && p.count > 0)?;
    let own = std::mem::replace(&mut p.data, mode.to_be_bytes().to_vec());
    let id = actions[i].identifier;
    let r = sim.force_trigger(i);
    if let Some(p) = sim.map_actions.actions[i]
        .parameters
        .iter_mut()
        .find(|p| &p.name.0 == b"mode" && p.kind == 5)
    {
        p.data = own;
    }
    tracing::debug!(
        "TFL game: M2_TFL_OBMO_MODES: obmo {id} mode {mode} at tick {}: {r:?}, camera control \
         flags {:#x}",
        sim.tick,
        sim.camera_control_flags
    );
    Some(id)
}

pub(crate) const SIM_COMMANDS: [u8; 13] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];

fn sim_executes(command: &CommandPacket) -> bool {
    SIM_COMMANDS.contains(&command.command_type.to_u8())
}

fn command_name(ty: u8) -> &'static str {
    match ty {
        film::TARGET_COMMAND => "the target command (an attack)",
        film::ATTACK_LOCATION_COMMAND => "the attack location command",
        film::PICK_UP_COMMAND => "the pick-up command",
        film::DETACH_COMMAND => "the detach command",
        film::RENAME_COMMAND => "the rename command",
        _ => "this command",
    }
}

impl TflLive {
    pub(crate) fn start_with(
        game_dir: &Path,
        map: &mut TflMap,
        game: SoloGame,
        carry: &TflCarry,
    ) -> (Self, Option<TflReplay>) {
        let mut live = Self {
            script: Default::default(),
            game,
            veterans: mandrake_tfl_sim::statistics::encode_monster_initializers(&carry.veterans),
            permanent: carry.permanent.clone(),
            tick: 0,
            stop: None,
            outgoing: Vec::new(),
            recorded: Vec::new(),
            said: Vec::new(),
            obmo_modes: obmo_modes_from_env().into(),
            films: None,
            written_at: None,
            restored_ambient_volumes: Vec::new(),
        };
        let bytes = live.film(0);
        map.film = mandrake_rec::tfl::TflRecording::from_bytes(&bytes)
            .ok()
            .map(|r| crate::tfl_map::with_game_colours(r.header));
        map.film_bytes = Some(bytes);
        let mode = map.film.as_ref().map_or(film::SOLO_GAME as i16, |h| h.mode);
        let mut replay = TflReplay::new_as(game_dir, map, mode);
        if let Some(sim) = replay.as_mut().and_then(TflReplay::sim_mut) {
            sim.set_previous_permanent_rankings(&carry.permanent);
            live.veterans = mandrake_tfl_sim::statistics::encode_monster_initializers(
                sim.monster_initializers(),
            );
        }
        match replay.as_ref().and_then(TflReplay::sim) {
            Some(sim) => {
                for v in sim.monster_initializers().iter().filter(|v| v.used) {
                    println!(
                        "TFL game: veteran {} of {} taken: name {:?} (list {}), {} kills, {} \
                         battles",
                        v.mons,
                        v.unit,
                        String::from_utf8_lossy(&v.name),
                        v.field_13,
                        v.experience,
                        v.field_11
                    );
                }
                println!(
                    "TFL game: {} at difficulty {}, seed {:#010x}: {} objects, {} monsters, {} \
                 veterans carried ({} taken)",
                    game.mesh,
                    game.difficulty,
                    game.seed,
                    sim.objects.len(),
                    sim.monsters.len(),
                    carry.veterans.len(),
                    sim.monster_initializers().iter().filter(|v| v.used).count()
                )
            }
            None => {
                let stop = replay
                    .as_ref()
                    .and_then(|r| r.stop().cloned())
                    .unwrap_or_else(|| Stop::new(0, "no TFL data"));
                tracing::error!("TFL game stopped in its set-up: {stop}");
                live.stop = Some(stop);
            }
        }
        (live, replay)
    }

    pub(crate) fn film(&self, end_tick: u32) -> Vec<u8> {
        film::solo_film(
            &self.game,
            LAST_RECORDING_NAME,
            LAST_RECORDING_ID,
            end_tick,
            &self.veterans,
            &self.recorded,
        )
    }

    pub(crate) fn tick(&self) -> u32 {
        self.tick
    }

    pub(crate) fn saved_game(
        &self,
        name: &[u8],
        snapshot: Option<mandrake_rec::saved_game::Snapshot>,
    ) -> save::TflSave {
        save::TflSave::of_film(
            film::solo_film(
                &self.game,
                name,
                save::SAVE_ID,
                self.tick,
                &self.veterans,
                &self.recorded,
            ),
            self.permanent.clone(),
            snapshot,
        )
    }

    pub(crate) fn begin_restore(
        game_dir: &Path,
        map: &mut TflMap,
        saved: &save::TflSave,
    ) -> Result<(Self, Option<TflReplay>, u32), String> {
        let game = saved.game().ok_or("the save's film doesn't read")?;
        let script = LiveScript::of_film(&saved.film).ok_or("the save's film doesn't read")?;
        let (mut live, replay) = Self::start_with(game_dir, map, game, &saved.carry());
        live.follow(script);
        Ok((live, replay, saved.end_tick()))
    }

    pub(crate) fn restore_step(
        &mut self,
        replay: &mut Option<TflReplay>,
        end: u32,
        budget: std::time::Duration,
    ) -> Result<bool, String> {
        let started = crate::clock::Instant::now();
        while self.tick() < end {
            if !self.step(replay) {
                return Err(format!(
                    "the save stopped at tick {} of {end}: {:?}",
                    self.tick(),
                    self.stop()
                ));
            }
            if started.elapsed() >= budget {
                break;
            }
        }
        if let Some(sim) = replay.as_mut().and_then(TflReplay::sim_mut) {
            let _ = sim.take_camera_movements();
        }
        if let Some(events) = replay.as_mut().map(TflReplay::take_sound_events) {
            for event in events {
                if let mandrake_tfl_sim::SoundKind::AmbientVolume {
                    identifier, volume, ..
                } = event.kind
                {
                    if let Some(slot) = self
                        .restored_ambient_volumes
                        .iter_mut()
                        .find(|(id, _)| *id == identifier)
                    {
                        slot.1 = volume;
                    } else {
                        self.restored_ambient_volumes.push((identifier, volume));
                    }
                }
            }
        }
        let over = self.tick() >= end;
        if over {
            info!("TFL game: restored to tick {end}");
        }
        Ok(over)
    }

    pub(crate) fn take_restored_ambient_volumes(&mut self) -> Vec<(i16, i16)> {
        std::mem::take(&mut self.restored_ambient_volumes)
    }

    pub(crate) fn follow(&mut self, script: LiveScript) {
        self.script = script.commands.into();
    }

    pub(crate) fn stop(&self) -> Option<&Stop> {
        self.stop.as_ref()
    }

    pub(crate) fn queue(&mut self, command: CommandPacket) {
        self.outgoing.push(command);
    }

    pub(crate) fn give(&mut self, command: CommandPacket) -> bool {
        let ty = command.command_type.to_u8();
        if sim_executes(&command) {
            self.queue(command);
            return true;
        }
        if !self.said.contains(&ty) {
            self.said.push(ty);
            tracing::error!(
                "TFL game: {} (type {ty}, {} bytes) isn't built in the sim yet: the order isn't \
                 sent",
                command_name(ty),
                command.raw_payload.len()
            );
        }
        false
    }

    pub(crate) fn step(&mut self, replay: &mut Option<TflReplay>) -> bool {
        if self.stop.is_some() {
            return false;
        }
        let Some(replay) = replay.as_mut().filter(|r| r.sim().is_some()) else {
            return false;
        };
        let t = replay.tick();
        while self.script.front().is_some_and(|c| c.tick <= t) {
            let c = self.script.pop_front().expect("checked");
            if c.tick == t {
                self.outgoing.push(c);
            }
        }
        let commands: Vec<CommandPacket> = std::mem::take(&mut self.outgoing)
            .into_iter()
            .map(|mut c| {
                c.tick = t;
                c.player_index = LOCAL_PLAYER;
                c
            })
            .filter(|c| replay.sniff_command(c))
            .collect();
        if !commands.is_empty() {
            let group = TickCommands { tick: t, commands };
            replay.queue(group.clone());
            self.recorded.push(group);
        }
        while self.obmo_modes.front().is_some_and(|&(at, _)| at <= t) {
            let (at, mode) = self.obmo_modes.pop_front().expect("checked");
            if let (true, Some(sim)) = (at == t, replay.sim_mut()) {
                trigger_obmo_with_mode(sim, mode);
            }
        }
        let ran = replay.step();
        self.tick = replay.tick();
        if ran {
            if self.checkpoint_due() {
                let _ = self.write_film();
            }
            return true;
        }
        let stop = replay
            .stop()
            .cloned()
            .unwrap_or_else(|| Stop::new(0, "the sim stopped"));
        let film = match self.write_film() {
            Some(Ok(path)) => format!("its film is {}", path.display()),
            Some(Err(e)) => format!("its film wasn't written: {e}"),
            None => "its film isn't written to a folder".to_string(),
        };
        tracing::error!(
            "TFL game stopped at tick {}: {stop}; {film} (the sim is still being built; the game \
             holds here)",
            self.tick
        );
        self.stop = Some(stop);
        false
    }

    fn checkpoint_due(&self) -> bool {
        let last = self.written_at.unwrap_or(0);
        self.films.is_some() && self.tick >= last.saturating_add(CHECKPOINT_TICKS)
    }

    pub(crate) fn record_to(&mut self, dir: Option<PathBuf>) {
        self.films = dir;
    }

    pub(crate) fn write_film(&mut self) -> Option<std::io::Result<PathBuf>> {
        let dir = self.films.clone()?;
        let written = self.save(&dir);
        if written.is_ok() {
            self.written_at = Some(self.tick);
        }
        Some(written)
    }

    pub(crate) fn save(&self, dir: &Path) -> std::io::Result<PathBuf> {
        let path = dir.join("LAST RECORDING");
        crate::config_store::write_bytes_replacing(&path, ".tmp", &self.film(self.tick()))?;
        info!(
            "TFL game: {} commands in {} ticks recorded",
            self.recorded
                .iter()
                .map(|t| t.commands.len())
                .sum::<usize>(),
            self.tick()
        );
        Ok(path)
    }
}

impl Drop for TflLive {
    fn drop(&mut self) {
        if self.films.is_some() && self.written_at != Some(self.tick) {
            println!(
                "TFL game: went at tick {} without its end: the film is written now",
                self.tick
            );
            let _ = self.write_film();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn obmo_modes_parse_by_tick() {
        assert_eq!(
            parse_obmo_modes("300:-1, 60:0,bad,400:2"),
            vec![(60, 0), (300, -1), (400, 2)]
        );
        assert!(parse_obmo_modes("").is_empty());
    }
}
