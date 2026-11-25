use std::net::TcpListener;
use std::path::PathBuf;

use anyhow::{anyhow, bail};
use mandrake_server::proxy::{self, ProxyConfig};
use tracing::info;

const USAGE: &str = "usage: mandrake-ws-proxy CONFIG.toml [--listen ADDR]";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stdout)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let mut config: Option<PathBuf> = None;
    let mut listen: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = Some(args.next().ok_or_else(|| anyhow!("--listen ADDR"))?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if config.is_none() && !a.starts_with('-') => config = Some(PathBuf::from(a)),
            _ => bail!("{a}?\n{USAGE}"),
        }
    }
    let mut config = ProxyConfig::load(&config.ok_or_else(|| anyhow!("{USAGE}"))?)?;
    if let Some(l) = listen {
        config.listen = l;
    }
    let listener =
        TcpListener::bind(&config.listen).map_err(|e| anyhow!("listen {}: {e}", config.listen))?;
    info!(
        "proxy: listening on {}, {} to {}, {} named targets",
        listener.local_addr()?,
        config.path,
        config.target,
        config.targets.len()
    );
    proxy::run(config, listener)
}
