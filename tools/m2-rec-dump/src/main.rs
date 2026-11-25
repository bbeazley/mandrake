use anyhow::Context;
use clap::Parser;
use mandrake_rec::{Recording, TickCommands};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "m2-rec-dump",
    about = "Disassemble Myth II recording files (.rec/.m2rec) into structured JSON"
)]
struct Args {
    #[arg(help = "Path to the recording file")]
    recording: String,
}

#[derive(Debug, serde::Serialize)]
pub struct RecDumpOutput {
    pub metadata: RecordingMetadataOutput,
    pub command_summary: BTreeMap<String, usize>,
    pub events_by_tick: Vec<TickCommands>,
}

#[derive(Debug, serde::Serialize)]
pub struct RecordingMetadataOutput {
    pub name: String,
    pub mesh: String,
    pub seed: u32,
    pub plugins: Vec<String>,
    pub teams: Vec<String>,
    pub player_count: usize,
    /// The JSON keeps the reader's old names: `game_type` is the mode, `difficulty` the game type.
    #[serde(rename = "game_type")]
    pub mode: i16,
    #[serde(rename = "difficulty")]
    pub game_type: i16,
    pub game_options_flags: u32,
    pub time_limit: u32,
}

pub fn dump_recording_json(path: &Path) -> anyhow::Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("Failed to open recording at {:?}", path))?;

    let recording = Recording::open(&mut file)
        .with_context(|| format!("Failed to parse recording at {:?}", path))?;

    let mut command_summary = BTreeMap::new();
    for (cmd_type, count) in recording.command_count_by_type() {
        command_summary.insert(cmd_type.name().to_string(), count);
    }

    let output = RecDumpOutput {
        metadata: RecordingMetadataOutput {
            name: recording.header.name,
            mesh: recording.header.map_name,
            seed: recording.header.initial_seed,
            plugins: recording
                .header
                .active_plugins
                .iter()
                .map(|p| p.name.clone())
                .collect(),
            teams: recording.header.teams,
            player_count: recording.header.players.len(),
            mode: recording.header.mode,
            game_type: recording.header.game_type,
            game_options_flags: recording.header.game_options_flags,
            time_limit: recording.header.time_limit,
        },
        command_summary,
        events_by_tick: recording.tick_commands,
    };

    serde_json::to_string_pretty(&output).context("Failed to serialize recording dump to JSON")
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    info!("Disassembling recording: {}", args.recording);

    let json = dump_recording_json(Path::new(&args.recording))?;
    println!("{json}");

    Ok(())
}
