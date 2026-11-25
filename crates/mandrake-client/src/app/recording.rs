use std::path::{Path, PathBuf};

use mandrake_common::FourCC;
use mandrake_rec::header::{MonsterInitializer, PluginInfo};
use mandrake_rec::writer::{NewGameData, Recorder, SoloNewGame};
use mandrake_rec::{CommandPacket, Recording};
use tracing::{error, info, warn};

use super::ClientApp;
use crate::args::Args;

const TRAINING_MAP: FourCC = FourCC(*b"00tm");
pub(crate) const CHECKPOINT_TICKS: u32 = 900;

pub(crate) struct FilmFile {
    game_dir: PathBuf,
    name: Vec<u8>,
    subgroup: Option<FourCC>,
}

impl FilmFile {
    pub(crate) fn new(game_dir: &Path, name: Vec<u8>) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
            name,
            subgroup: None,
        }
    }

    pub(crate) fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    pub(crate) fn for_archive(archive: &Path) -> Option<Self> {
        let game_dir = archive.parent()?.parent()?;
        let tags = mandrake_tag::mesh_links::CampaignTags::open(game_dir);
        let name = mandrake_session::films::FilmTags::new(&tags).last_recording_name();
        Some(Self::new(game_dir, name))
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.game_dir
            .join("recordings")
            .join(String::from_utf8_lossy(&self.name).as_ref())
    }

    pub(crate) fn write(&mut self, data: &[u8]) -> Option<PathBuf> {
        let subgroup = *self.subgroup.get_or_insert_with(|| {
            mandrake_session::films::last_recording_subgroup(
                &self.game_dir,
                &self.name,
                local_random(),
            )
        });
        match mandrake_session::films::write_recording(&self.game_dir, &self.name, subgroup, data) {
            Ok(path) => {
                info!("recording: wrote {} ({} bytes)", path.display(), data.len());
                Some(path)
            }
            Err(e) => {
                error!(
                    "recording: the last recording wasn't written to {}: {e}",
                    self.path().display()
                );
                None
            }
        }
    }
}

fn local_random() -> impl FnMut() -> u16 {
    let mut state = crate::clock::SystemTime::now()
        .duration_since(crate::clock::UNIX_EPOCH)
        .map_or(0x2545_f491, |d| d.as_nanos() as u64)
        | 1;
    move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 16) as u16
    }
}

const fn writes_film_files() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

#[cfg(not(target_arch = "wasm32"))]
struct CheckpointWriter {
    tx: Option<std::sync::mpsc::Sender<(Recorder, u32)>>,
    handle: Option<std::thread::JoinHandle<FilmFile>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl CheckpointWriter {
    fn start(mut file: FilmFile) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<(Recorder, u32)>();
        let handle = std::thread::Builder::new()
            .name("m2-film-checkpoint".to_owned())
            .spawn(move || {
                while let Ok((recorder, tick)) = rx.recv() {
                    let data = recorder.finish(tick);
                    file.write(&data);
                }
                file
            })
            .ok();
        Self {
            tx: handle.is_some().then_some(tx),
            handle,
        }
    }

    fn send(&self, recorder: Recorder, tick: u32) -> bool {
        self.tx
            .as_ref()
            .is_some_and(|tx| tx.send((recorder, tick)).is_ok())
    }

    fn finish(mut self) -> Option<FilmFile> {
        self.tx = None;
        self.handle.take().and_then(|h| h.join().ok())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for CheckpointWriter {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(crate) struct LiveRecording {
    recorder: Recorder,
    before_count: usize,
    recorded_count: usize,
    file: Option<FilmFile>,
    #[cfg(not(target_arch = "wasm32"))]
    writer: Option<CheckpointWriter>,
    written_at: u32,
}

impl LiveRecording {
    fn new(
        game: NewGameData,
        initializers: &[MonsterInitializer],
        before: &[CommandPacket],
        file: Option<FilmFile>,
        written_at: u32,
    ) -> Self {
        let mut recorder = Recorder::new(game, initializers);
        for c in before {
            recorder.record_packet(c);
        }
        Self {
            recorder,
            before_count: before.len(),
            recorded_count: 0,
            file,
            #[cfg(not(target_arch = "wasm32"))]
            writer: None,
            written_at,
        }
    }

    fn has_file(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.file.is_some() || self.writer.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.file.is_some()
        }
    }

    fn checkpoint_due(&self, tick: u32) -> bool {
        self.has_file() && tick >= self.written_at.saturating_add(CHECKPOINT_TICKS)
    }

    fn sync_commands(&mut self, commands: &[CommandPacket]) {
        if let Some(new_commands) = commands.get(self.recorded_count..) {
            for c in new_commands {
                self.recorder.record_packet(c);
            }
            self.recorded_count = commands.len();
        }
    }

    fn film(&mut self, commands: &[CommandPacket], tick: u32) -> Vec<u8> {
        self.sync_commands(commands);
        self.recorder.clone().finish(tick)
    }

    fn flush_writer(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(writer) = self.writer.take() {
            self.file = writer.finish();
        }
    }

    fn write(&mut self, tick: u32, data: &[u8]) -> Option<PathBuf> {
        self.flush_writer();
        let path = self.file.as_mut().and_then(|file| file.write(data));
        self.written_at = tick;
        path
    }

    fn checkpoint(&mut self, commands: &[CommandPacket], tick: u32) {
        self.sync_commands(commands);
        self.written_at = tick;
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.writer.is_none()
                && let Some(file) = self.file.take()
            {
                self.writer = Some(CheckpointWriter::start(file));
            }
            if let Some(writer) = &self.writer
                && writer.send(self.recorder.clone(), tick)
            {
                return;
            }
            self.flush_writer();
        }
        let data = self.recorder.clone().finish(tick);
        if let Some(file) = self.file.as_mut() {
            file.write(&data);
        }
    }

    pub(crate) fn recorder_with(&self, commands: &[CommandPacket]) -> Recorder {
        let mut recorder = self.recorder.clone();
        if let Some(new_commands) = commands.get(self.recorded_count..) {
            for c in new_commands {
                recorder.record_packet(c);
            }
        }
        recorder
    }
}

pub(crate) fn plugin_infos(game_dir: &Path, names: &[String]) -> Vec<PluginInfo> {
    let vfs = mandrake_common::game_vfs();
    names
        .iter()
        .filter_map(|name| {
            let files = mandrake_tag::engine_tag_files(game_dir, &[name.as_str()]);
            if !files.missing_plugins.is_empty() {
                return None;
            }
            let path = files.paths.last()?;
            let file = mandrake_tag::TagArchive::open(vfs, &path.to_string_lossy()).ok()?;
            let header = file.header();
            let description = &header.description;
            let len = description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(description.len());
            Some(PluginInfo {
                name: mandrake_tag::tag_files::plugin_name(path)?,
                url: String::from_utf8_lossy(&description[..len]).into_owned(),
                checksum: u32::from_be_bytes(header.checksum),
            })
        })
        .collect()
}

pub(crate) fn network_film(game: &NewGameData, commands: &[CommandPacket], tick: u32) -> Vec<u8> {
    let mut recorder = Recorder::new(game.clone(), &[]);
    for c in commands {
        recorder.record_packet(c);
    }
    recorder.finish(tick)
}

impl ClientApp {
    pub(crate) fn start_recording(&mut self, args: &Args, game_dir: &Path, file: Option<FilmFile>) {
        let file = file.filter(|_| writes_film_files());
        let mesh = self.mesh_tag.mesh_id;
        if self.replay.is_some() || mesh == TRAINING_MAP {
            return;
        }
        let (seed, veterans, recording) =
            if let Some(game) = self.net.as_ref().and_then(|n| n.game.clone()) {
                let seed = game.seed();
                let veterans = self.sim.carried_initializers.len();
                let recording = LiveRecording::new(
                    game,
                    &self.sim.carried_initializers,
                    &[],
                    file,
                    self.sim.current_tick,
                );
                (seed, veterans, recording)
            } else if self.restore.is_some() {
                let Some(save) = args
                    .replay
                    .as_deref()
                    .and_then(|p| crate::game_files::read(p).ok())
                else {
                    warn!("recording: the saved game doesn't read again");
                    return;
                };
                let Ok(rec) = Recording::from_bytes(&save) else {
                    return;
                };
                let at = mandrake_rec::header::data_offset(&save);
                let Some(mut game) = save.get(at..).and_then(NewGameData::from_header) else {
                    return;
                };
                game.0[0..2].copy_from_slice(&2u16.to_be_bytes());
                let seed = game.seed();
                let veterans = rec.header.monster_initializers.len();
                let before: Vec<CommandPacket> = rec.iter_commands().cloned().collect();
                let recording = LiveRecording::new(
                    game,
                    &rec.header.monster_initializers,
                    &before,
                    file,
                    self.sim.current_tick,
                );
                (seed, veterans, recording)
            } else {
                let game = NewGameData::solo(&SoloNewGame {
                    mesh,
                    difficulty: args.difficulty,
                    seed: self.sim.sync_snapshot.0,
                    options: self.sim.game_option_flags(),
                    plugins: plugin_infos(game_dir, &args.plugin),
                });
                let seed = game.seed();
                let veterans = self.sim.carried_initializers.len();
                let recording = LiveRecording::new(
                    game,
                    &self.sim.carried_initializers,
                    &[],
                    file,
                    self.sim.current_tick,
                );
                (seed, veterans, recording)
            };
        info!(
            "recording: {} on {} (seed {seed:#x}, {veterans} veterans)",
            if recording.before_count == 0 {
                "a game"
            } else {
                "a restored game"
            },
            mesh.as_str(),
        );
        self.sim.recorded_commands = Some(Vec::new());
        self.sim.recording_ended = false;
        self.recording = Some(recording);
    }

    pub(crate) fn checkpoint_recording(&mut self) {
        let tick = self.sim.current_tick;
        let Some(recording) = self.recording.as_mut().filter(|r| r.checkpoint_due(tick)) else {
            return;
        };
        let commands = self.sim.recorded_commands.as_deref().unwrap_or_default();
        recording.checkpoint(commands, tick);
    }

    pub(crate) fn finish_recording(&mut self) -> Option<Vec<u8>> {
        let mut recording = self.recording.take()?;
        let commands = self.sim.recorded_commands.take().unwrap_or_default();
        let tick = self.sim.current_tick;
        info!(
            "recording: stopped at tick {tick} with {} commands",
            recording.before_count + commands.len()
        );
        let data = recording.film(&commands, tick);
        recording.write(tick, &data);
        Some(data)
    }

    fn write_recording_now(&mut self) -> Option<PathBuf> {
        let tick = self.sim.current_tick;
        let recording = self.recording.as_mut()?;
        let commands = self.sim.recorded_commands.as_deref().unwrap_or_default();
        let data = recording.film(commands, tick);
        recording.write(tick, &data)
    }

    pub(crate) fn report_stop(&mut self) {
        if self.stop_report.is_some() || self.replay.is_some() {
            return;
        }
        let Some(stop) = self.sim.engine_stop.clone() else {
            return;
        };
        let tick = self.sim.current_tick;
        let path = self.write_recording_now();
        let film = match (&path, &self.recording) {
            (Some(path), _) => format!("its film is {}", path.display()),
            (None, Some(_)) => "its film wasn't written".to_string(),
            (None, None) => "this game isn't recorded".to_string(),
        };
        let line = stop_line(&self.mesh_name, tick, &stop, &film);
        error!("{line}");
        self.stop_report = Some(match &path {
            Some(path) => format!(
                " (film: {})",
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned()
                )
            ),
            None => String::new(),
        });
    }
}

pub(crate) fn stop_line(
    mesh: &str,
    tick: u32,
    stop: &mandrake_sim::engine_rng::Unmodelled,
    film: &str,
) -> String {
    format!(
        "mandrake: the game on {mesh} stopped at tick {tick}: not ported: site {:#x}: {}; {film} \
         (replay it with --replay to reach the stop)",
        stop.site, stop.reason
    )
}

impl Drop for ClientApp {
    fn drop(&mut self) {
        if self.recording.is_some() {
            warn!("recording: the map went without its end: the film is written now");
            self.finish_recording();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{self, Args};
    use clap::Parser;

    #[test]
    fn the_desktop_writes_film_files() {
        assert!(writes_film_files());
    }

    #[test]
    fn a_window_on_a_mesh_is_recorded() {
        let parse = |extra: &[&str]| {
            let mut line = vec!["mandrake", "--archive", "/x/tags/small install"];
            line.extend_from_slice(extra);
            args::records_direct_game(&Args::parse_from(line))
        };
        assert!(parse(&["--lighting", "on", "--mesh", "11an"]));
        assert!(parse(&["--play", "cr01"]));
        assert!(!parse(&[]), "the menus record from their session");
        assert!(!parse(&["--mesh", "11an", "--headless"]));
        assert!(!parse(&["--mesh", "11an", "--screenshot", "/x/a.png"]));
        assert!(!parse(&["--replay", "MDL.m2rec"]));
        assert!(!parse(&["--mesh", "11an", "--frame-budget", "5"]));
    }
}
