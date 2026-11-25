#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Myth II 3D Client, the native binary: the logger and the command line, then [`mandrake_client::run`].

fn main() -> anyhow::Result<()> {
    mandrake_client::init_audio_env();
    // Stderr: a map's end prints the carnage report on stdout, which holds nothing else.
    tracing_subscriber::fmt()
        .with_env_filter(mandrake_client::log_filter())
        .with_writer(std::io::stderr)
        .init();
    // `--help` and a bad command line exit here, as clap's `parse` does.
    let args = mandrake_client::parse_command_line(std::env::args_os().collect())
        .unwrap_or_else(|e| e.exit());
    mandrake_client::run(args)
}
