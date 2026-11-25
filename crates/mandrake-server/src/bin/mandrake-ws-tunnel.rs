use std::net::TcpListener;

use anyhow::{anyhow, bail};
use tracing::info;

const USAGE: &str = "usage: mandrake-ws-tunnel LISTEN ws://HOST:PORT/PATH [--once]";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stdout)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let mut once = false;
    let mut pos = Vec::new();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--once" => once = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if !a.starts_with('-') => pos.push(a),
            _ => bail!("{a}?\n{USAGE}"),
        }
    }
    let [listen, url] = <[String; 2]>::try_from(pos).map_err(|_| anyhow!("{USAGE}"))?;
    let listener = TcpListener::bind(&listen).map_err(|e| anyhow!("listen {listen}: {e}"))?;
    info!("tunnel: listening on {}, to {url}", listener.local_addr()?);
    mandrake_server::proxy::tunnel(listener, &url, once)
}
