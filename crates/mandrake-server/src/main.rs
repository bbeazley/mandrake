use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail};
use mandrake_server::config::Config;
use mandrake_server::game::Server;
use tracing::{info, warn};

const USAGE: &str = "usage: mandrake-server CONFIG.toml [--games N] [--lobby-timeout SECONDS]";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stdout)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let mut config: Option<PathBuf> = None;
    let mut games: Option<u32> = None;
    let mut lobby_timeout: Option<Duration> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--games" => {
                games = Some(
                    args.next()
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| anyhow!("--games N\n{USAGE}"))?,
                )
            }
            "--lobby-timeout" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--lobby-timeout SECONDS\n{USAGE}"))?;
                lobby_timeout = Some(parse_lobby_timeout(&raw)?);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if config.is_none() && !a.starts_with('-') => config = Some(PathBuf::from(a)),
            _ => bail!("{a}?\n{USAGE}"),
        }
    }
    let path = config.ok_or_else(|| anyhow!("{USAGE}"))?;
    let config = Config::load(&path)?;
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, config.port));
    let mut server = Server::new(config)?;
    server.max_games = games;
    if let Some(tick) = std::env::var("M2_TEST_STOP_AT_TICK")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        warn!("mandrake-server: M2_TEST_STOP_AT_TICK: every game stops at tick {tick}");
        server.stop_at_tick = Some(tick);
    }
    info!(
        "mandrake-server: game folder {}, films in {}",
        server.game_dir.display(),
        server.config.films_dir.display()
    );
    let mut hosted = 0;
    let mut out_of_sync = 0;
    let mut stopped = 0;
    while games.is_none_or(|n| hosted < n) {
        let lobby = server.host_one(addr, lobby_timeout)?;
        for r in &lobby.games {
            hosted += 1;
            out_of_sync += r.out_of_sync;
            stopped += usize::from(r.stopped.is_some());
            info!(
                "mandrake-server: game {hosted} ended at tick {}{}{}, {} out of sync, film {:?}",
                r.end_tick,
                if r.owner_left {
                    " (the owner left)"
                } else {
                    ""
                },
                r.stopped
                    .as_ref()
                    .map_or(String::new(), |why| format!(" (stopped: {why})")),
                r.out_of_sync,
                r.film
            );
        }
        for why in &lobby.refused {
            stopped += 1;
            info!("mandrake-server: a start refused, the players kept in the lobby: {why}");
        }
        if lobby.timed_out {
            break;
        }
    }
    if out_of_sync > 0 {
        warn!("mandrake-server: {out_of_sync} out of sync in {hosted} games");
        bail!("{out_of_sync} out of sync");
    }
    if stopped > 0 {
        warn!("mandrake-server: {stopped} of {hosted} games stopped or were refused");
        bail!("{stopped} stopped");
    }
    Ok(())
}

fn parse_lobby_timeout(raw: &str) -> anyhow::Result<Duration> {
    let secs: f64 = raw
        .parse()
        .map_err(|_| anyhow!("--lobby-timeout SECONDS\n{USAGE}"))?;
    Duration::try_from_secs_f64(secs).map_err(|_| anyhow!("--lobby-timeout SECONDS\n{USAGE}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_timeout_rejects_negative_nan_and_infinity() {
        assert_eq!(
            parse_lobby_timeout("1.5").unwrap(),
            Duration::from_millis(1500)
        );
        assert_eq!(parse_lobby_timeout("0").unwrap(), Duration::ZERO);
        for bad in ["-1", "nan", "NaN", "inf", "-inf", "1e400", "abc"] {
            assert!(parse_lobby_timeout(bad).is_err(), "{bad}");
        }
    }
}
