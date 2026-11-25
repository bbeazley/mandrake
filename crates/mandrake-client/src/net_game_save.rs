use std::collections::VecDeque;
use std::path::Path;

use mandrake_net::netsave::SavedGame;
use mandrake_net::wire::Command;
use mandrake_rec::writer::NewGameData;
use mandrake_sim::Simulation;
use mandrake_sim::save_restore::{PlayerEntry, SaveLoaded};

pub(crate) const RESTORE_TICKS: u32 = 0x708;
const BATCH_BYTES: usize = 0x800;
const SAVE_LOADED: u8 = 0x19;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Replay,
    ToEnd,
    Queued,
}

#[derive(Debug, Clone)]
pub(crate) struct HostRestore {
    commands: VecDeque<Command>,
    pub(crate) end: u32,
    observer: SaveLoaded,
    last_sent: Option<u32>,
    phase: Phase,
    pub(crate) sent: (usize, usize),
}

#[derive(Debug, Clone)]
pub(crate) enum Restore {
    Host(HostRestore),
    Peer { end: u32, told: u32 },
}

impl Restore {
    pub(crate) fn end(&self) -> u32 {
        match self {
            Restore::Host(h) => h.end,
            Restore::Peer { end, .. } => *end,
        }
    }
}

fn tag_data(file: &[u8]) -> Option<&[u8]> {
    file.get(mandrake_rec::header::data_offset(file)..)
}

pub(crate) fn load_save(path: &Path) -> anyhow::Result<(SavedGame, HostRestore)> {
    let file = std::fs::read(path)?;
    let film = tag_data(&file).ok_or_else(|| anyhow::anyhow!("{path:?}: no tag data"))?;
    let save = SavedGame::from_film(film).ok_or_else(|| anyhow::anyhow!("{path:?}: short"))?;
    let saved = mandrake_rec::saved_game::SavedGame::from_bytes(&file)
        .map_err(|e| anyhow::anyhow!("{path:?}: {e}"))?;
    let recording =
        mandrake_rec::Recording::from_bytes(&file).map_err(|e| anyhow::anyhow!("{path:?}: {e}"))?;
    let end = recording.header.end_tick;
    let mut commands = VecDeque::new();
    for tc in &recording.tick_commands {
        for c in &tc.commands {
            let b = c.to_bytes();
            if b.len() < 8 {
                continue;
            }
            let kind = (b[2] as i8).unsigned_abs();
            if kind == SAVE_LOADED {
                continue;
            }
            commands.push_back(Command {
                kind,
                player: b[3],
                tick: tc.tick as i32,
                payload: b[8..].to_vec(),
            });
        }
    }
    let h = &saved.header;
    let observer = SaveLoaded {
        x: h.observer_x,
        y: h.observer_y,
        facing: h.observer_facing,
        zoom: h.observer_zoom,
    };
    Ok((
        save,
        HostRestore {
            commands,
            end,
            observer,
            last_sent: None,
            phase: Phase::Replay,
            sent: (0, 0),
        },
    ))
}

pub(crate) fn may_save(sim: &Simulation, coop_host: bool, mesh: [u8; 4]) -> bool {
    let w = &sim.engine;
    coop_host
        && w.formation_globals.gameplay_version > mandrake_common::game_options::GAMEPLAY_1_8_4
        && w.local_team() != -1
        && &mesh != b"mt00"
}

pub(crate) fn saved_game_file(
    game: &NewGameData,
    commands: &[mandrake_rec::CommandPacket],
    tick: u32,
    header: &mandrake_rec::saved_game::SavedGameHeader,
) -> Vec<u8> {
    use mandrake_rec::saved_game::{SNAPSHOT_HEIGHT, SNAPSHOT_WIDTH, Snapshot};
    let mut recorder = mandrake_rec::writer::Recorder::new(game.clone(), &[]);
    for c in commands {
        recorder.record_packet(c);
    }
    let snapshot = Snapshot {
        width: SNAPSHOT_WIDTH,
        height: SNAPSHOT_HEIGHT,
        rgba: [0, 0, 0, 0xff].repeat((SNAPSHOT_WIDTH * SNAPSHOT_HEIGHT) as usize),
    };
    recorder.saved_game(tick, &header.to_bytes(), &snapshot.to_bytes())
}

pub(crate) fn entries(game: &NewGameData) -> Vec<PlayerEntry> {
    let d = &game.0;
    let count = d.get(0x22c..0x22e).map_or(0, |c| {
        i16::from_be_bytes([c[0], c[1]]).clamp(0, 16) as usize
    });
    (0..count)
        .filter_map(|i| PlayerEntry::from_record(d.get(0x230 + i * 0x7c..0x230 + (i + 1) * 0x7c)?))
        .collect()
}

pub(crate) enum HostStep {
    Waiting,
    Sent { batch: usize, last: u32 },
    ToEnd,
    Queued { commands: usize },
}

impl HostRestore {
    pub(crate) fn step(
        &mut self,
        sim: &Simulation,
        host: &mut mandrake_net::endpoint::Host,
        entries: &[PlayerEntry],
        now_ms: u64,
    ) -> HostStep {
        let tick = sim.current_tick;
        match self.phase {
            Phase::Replay => {
                let ready = self.last_sent.is_none_or(|t| {
                    tick >= t && host.all_players_reached_game_time(i64::from(t) - 1)
                });
                if !ready {
                    return HostStep::Waiting;
                }
                if self.commands.is_empty() {
                    host.set_tick_override(Some(self.end));
                    self.phase = Phase::ToEnd;
                    return HostStep::ToEnd;
                }
                let mut bytes = 0;
                let mut batch = 0;
                let mut last = 0;
                while let Some(c) = self.commands.front() {
                    let size = 8 + c.payload.len();
                    if batch > 0 && bytes + size > BATCH_BYTES {
                        break;
                    }
                    let c = self.commands.pop_front().expect("a command");
                    bytes += size;
                    batch += 1;
                    last = c.tick.max(0) as u32;
                    host.send_spoofed(c, last, now_ms);
                }
                self.last_sent = Some(last);
                self.sent = (self.sent.0 + batch, self.sent.1 + 1);
                HostStep::Sent { batch, last }
            }
            Phase::ToEnd => {
                if tick < self.end {
                    return HostStep::Waiting;
                }
                let w = &sim.engine;
                let mut commands = w.network_restore_player_commands(entries);
                commands.push(mandrake_sim::save_restore::save_loaded_command(
                    0,
                    self.observer,
                ));
                for c in &commands {
                    let b = c.to_bytes();
                    let command = Command {
                        kind: b[2],
                        player: b[3],
                        tick: 0,
                        payload: b[8..].to_vec(),
                    };
                    host.send_spoofed(command, self.end, now_ms);
                }
                host.resume_clock_at(self.end, now_ms);
                self.phase = Phase::Queued;
                HostStep::Queued {
                    commands: commands.len(),
                }
            }
            Phase::Queued => HostStep::Waiting,
        }
    }
}
