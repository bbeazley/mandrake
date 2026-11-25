#![allow(dead_code)]

#[path = "../../../mandrake-common/tests/support/require_data.rs"]
pub mod test_data;

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mandrake_net::endpoint::Endpoint;
use mandrake_net::player_data::{JoinRequest, PlayerData};
use mandrake_server::config::Config;
use mandrake_server::game::{Hosted, Server};

pub fn data_dir() -> Option<PathBuf> {
    let dir = test_data::require_game()?;
    test_data::require_data(dir.join("tags")).then_some(dir)
}

pub fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn init_log() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::INFO)
        .try_init();
}

pub fn fresh_films(name: &str) -> PathBuf {
    let films = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&films);
    films
}

pub fn config(data: &Path, films: &Path, title: &str, map: &str) -> Config {
    Config::parse(
        &format!(
            "data_dir = \"{}\"\nname = \"{title}\"\nfilms_dir = \"{}\"\n[game]\n\
             map = \"{map}\"\ntime_limit_minutes = 5\n",
            data.display(),
            films.display()
        ),
        Path::new("/"),
    )
    .unwrap()
}

pub fn serve(
    config: Config,
    stop_at_tick: Option<u32>,
) -> (SocketAddr, std::thread::JoinHandle<Hosted>) {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, free_port()));
    let mut server = Server::new(config).unwrap();
    server.stop_at_tick = stop_at_tick;
    let handle = std::thread::spawn(move || {
        server
            .host_one(addr, Some(Duration::from_secs(120)))
            .expect("the server's lobby (a stop is not its error)")
    });
    (addr, handle)
}

pub fn request(name: &[u8], team: &[u8]) -> JoinRequest {
    JoinRequest {
        name: name.to_vec(),
        password: Vec::new(),
        player: PlayerData::new(name, team, [0; 8]),
    }
}

pub fn join(addr: SocketAddr, request: JoinRequest) -> (Endpoint, Instant) {
    let clock = Instant::now();
    let endpoint = loop {
        match Endpoint::join(addr, request.clone(), 0) {
            Ok(e) => break e,
            Err(_) if clock.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(e) => panic!("no host at {addr}: {e}"),
        }
    };
    (endpoint, clock)
}
