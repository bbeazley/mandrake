use std::time::{Duration, Instant};

use mandrake_net::client::{COMMAND_SYNC_FORMATION, DEFAULT_FORMATION, END_GAME_COUNTDOWN, film};
use mandrake_net::endpoint::{Endpoint, Event, HostEvent, HostOptions};
use mandrake_net::game_description::GameDescription;
use mandrake_net::lobby::{BarrierKind, ChatTo, code};
use mandrake_net::player_data::{JoinRequest, PlayerData};
use mandrake_net::wire::{self, Command, Message};

const TEAM_NAME: &[u8] = b"\xd5s Team";
const COLORS: [u16; 8] = [
    0x0000, 0x5998, 0x08f3, 0x2f4b, 0x8cb2, 0x1a27, 0xc424, 0x0000,
];

struct Args {
    description: String,
    port: u16,
    name: String,
    joiners: usize,
    start_after: f64,
    load_ms: u64,
    end_margin: i32,
    film: String,
    tag_name: String,
    postgame: f64,
    timeout: f64,
    chat: Option<String>,
    chat_after: f64,
}

fn args() -> Args {
    let mut a = Args {
        description: String::new(),
        port: wire::DEFAULT_PORT,
        name: "Host".into(),
        joiners: 1,
        start_after: 3.0,
        load_ms: 700,
        end_margin: 15,
        film: "Last Recording".into(),
        tag_name: "Last Recording".into(),
        postgame: 8.0,
        timeout: 300.0,
        chat: None,
        chat_after: 2.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| usage(&flag));
        match flag.as_str() {
            "--description" => a.description = value(),
            "--port" => a.port = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--name" => a.name = value(),
            "--joiners" => a.joiners = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--start-after" => a.start_after = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--load-ms" => a.load_ms = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--end-margin" => a.end_margin = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--film" => a.film = value(),
            "--tag-name" => a.tag_name = value(),
            "--postgame" => a.postgame = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--timeout" => a.timeout = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--chat" => a.chat = Some(value()),
            "--chat-after" => a.chat_after = value().parse().unwrap_or_else(|_| usage(&flag)),
            _ => usage(&flag),
        }
    }
    if a.description.is_empty() {
        usage("--description");
    }
    a
}

fn usage(flag: &str) -> ! {
    eprintln!(
        "bad or missing argument {flag}\nusage: host_peer --description FILE [--port P] \
         [--name NAME] [--joiners N] [--start-after S] [--load-ms MS] [--end-margin TICKS] \
         [--film PATH] [--tag-name NAME] [--postgame S] [--timeout S] [--chat TEXT] \
         [--chat-after S]"
    );
    std::process::exit(64)
}

fn main() {
    std::process::exit(play(&args()));
}

fn kind_name(kind: u8) -> String {
    match kind {
        1 => "SYNC".into(),
        2 => "GENERAL".into(),
        9 => "DROP_PLAYER".into(),
        10 => "CHAT".into(),
        16 => "SYNC_FORMATION".into(),
        k => format!("type {k}"),
    }
}

fn description(path: &str) -> Result<GameDescription, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let m = Message::decode(&bytes).map_err(|e| format!("{path}: {e}"))?;
    GameDescription::from_body(&m.body).map_err(|e| format!("{path}: {e}"))
}

fn play(a: &Args) -> i32 {
    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let log = |what: String| println!("{:8} {what}", start.elapsed().as_millis());
    let d = match description(&a.description) {
        Ok(d) => d,
        Err(e) => {
            log(e);
            return 2;
        }
    };
    let limit = d.parameters.time_limit;
    log(format!(
        "description: map {:?} type {} limit {limit} max {} players, {} plugins, game \"{}\"",
        String::from_utf8_lossy(&d.parameters.mesh),
        d.parameters.game_type,
        d.parameters.max_players,
        d.plugins.0.len(),
        String::from_utf8_lossy(&d.game_name)
    ));
    let options = HostOptions {
        player: JoinRequest {
            name: a.name.as_bytes().to_vec(),
            password: Vec::new(),
            player: PlayerData::new(a.name.as_bytes(), TEAM_NAME, COLORS),
        },
        password: Vec::new(),
        description: d,
    };
    let mut ep = match Endpoint::host(a.port, options, now()) {
        Ok(e) => e,
        Err(e) => {
            log(format!("listen on {}: {e}", a.port));
            return 2;
        }
    };
    log(format!("listening on {:?}", ep.listen_addr()));
    let mut all_ready_at = None;
    let mut started = false;
    let mut game = None;
    let mut barrier_at = None;
    let mut in_game = false;
    let mut formation_sent = false;
    let mut end_begun = false;
    let mut recorded: Vec<Command> = Vec::new();
    let mut all_joined_at = None;
    let mut chat_sent = false;
    let deadline = Duration::from_secs_f64(a.timeout);
    while start.elapsed() < deadline {
        let t = now();
        let events = ep.poll(t);
        let host_events = ep
            .as_host_mut()
            .map(|h| h.take_events())
            .unwrap_or_default();
        for e in &host_events {
            match e {
                HostEvent::Joined { id, name } => log(format!(
                    "host: player {id} \"{}\" joined, colours {:04x?}",
                    String::from_utf8_lossy(name),
                    ep.as_host()
                        .and_then(|h| h.topology().player(*id))
                        .map(|p| p.data.appearance.colors)
                        .unwrap_or_default()
                )),
                HostEvent::Chat { from, chat } => log(format!(
                    "host: chat from {from} for passing on, words {:04x?}: {}",
                    chat.words,
                    String::from_utf8_lossy(&chat.text)
                )),
                HostEvent::Arrived { .. } | HostEvent::PlayerChanged { .. } => {}
                other => log(format!("host: {other:?}")),
            }
        }
        for e in &events {
            match e {
                Event::Commands { commands, .. } => {
                    for c in commands {
                        log(format!(
                            "stamped {} player {} tick {} ({} bytes)",
                            kind_name(c.kind),
                            c.player,
                            c.tick,
                            8 + c.payload.len()
                        ));
                        if game.is_some() {
                            recorded.push(c.clone());
                        }
                    }
                }
                Event::Player { player, new } => log(format!(
                    "{} player {} \"{}\" ready {}",
                    if *new { "new" } else { "changed" },
                    player.id,
                    String::from_utf8_lossy(&player.name),
                    player.data.ready
                )),
                Event::Description(d) => log(format!(
                    "description out: seed {:#x}, {} plugins",
                    d.parameters.seed,
                    d.plugins.0.len()
                )),
                Event::Configuration { flags, code, value } => {
                    log(format!(
                        "configuration flags {flags} code {code} value {value}"
                    ));
                    if *flags == 0 && *code == code::START {
                        match ep.joiner().new_game_data() {
                            Some(Ok(g)) => game = Some(g),
                            other => {
                                log(format!("no new-game data: {other:?}"));
                                return 1;
                            }
                        }
                        ep.joiner_mut().new_map();
                        recorded.clear();
                        barrier_at = Some(t + a.load_ms);
                    }
                }
                Event::BeginSync { barrier } => {
                    log(format!("begin sync, barrier {barrier}"));
                    if ep.joiner_mut().loaded() {
                        log("our own barrier answer".into());
                    }
                }
                Event::AllSynced { barrier } => {
                    log(format!("all synced, barrier {barrier}"));
                    if !in_game {
                        in_game = true;
                    } else {
                        let Some(g) = game.take() else { return 1 };
                        return finish(&mut ep, a, g, &recorded, limit, &log, start);
                    }
                }
                Event::Chat { from, chat } => log(format!(
                    "chat from {from} \"{}\", words {:04x?}: {}",
                    String::from_utf8_lossy(chat.name.split(|&b| b == 0).next().unwrap_or(&[])),
                    chat.words,
                    String::from_utf8_lossy(&chat.text)
                )),
                Event::Joined { id } => log(format!("our own player joined as {id}")),
                Event::PlayerLeft { id } => log(format!("player {id} left")),
                Event::Greeting(_) => {}
                other => log(format!("PROBLEM {other:?}")),
            }
        }
        let host = ep.as_host_mut().expect("a host");
        if all_joined_at.is_none() && host.topology().joined().count() == a.joiners + 1 {
            all_joined_at = Some(t);
        }
        if let (Some(text), Some(at), false) = (&a.chat, all_joined_at, chat_sent)
            && t >= at + (a.chat_after * 1000.0) as u64
        {
            chat_sent = host.chat(ChatTo::All, text.as_bytes());
            log(format!("our own chat sent: {text}"));
        }
        if !started {
            let players: Vec<_> = host.topology().joined().cloned().collect();
            let ready = players.len() == a.joiners + 1
                && players.iter().all(|p| p.id == 0 || p.data.ready != 0);
            match (ready, all_ready_at) {
                (true, None) => {
                    log(format!("{} joiners ready", a.joiners));
                    all_ready_at = Some(t);
                }
                (false, Some(_)) => all_ready_at = None,
                _ => {}
            }
            if all_ready_at.is_some_and(|at| t >= at + (a.start_after * 1000.0) as u64) {
                host.start(t as u32 ^ 0x0001_30df);
                started = true;
                log("start".into());
            }
        }
        if barrier_at.is_some_and(|at| t >= at) {
            barrier_at = None;
            host.begin_barrier(BarrierKind::Start);
            log("begin the pre-game barrier".into());
        }
        if in_game && !end_begun && limit > 0 {
            let tick = host.tick(t);
            if tick >= limit + END_GAME_COUNTDOWN as i32 + a.end_margin {
                end_begun = true;
                host.begin_barrier(BarrierKind::End);
                log(format!(
                    "tick {tick}: the server stops, the end barrier begins"
                ));
            }
        }
        if in_game && !formation_sent && ep.joiner().horizon() > 0 {
            formation_sent = ep
                .joiner_mut()
                .send_command(COMMAND_SYNC_FORMATION, DEFAULT_FORMATION.to_vec());
            log("sent SYNC_FORMATION".into());
        }
        ep.flush();
        std::thread::sleep(Duration::from_millis(2));
    }
    log("timeout".into());
    3
}

fn finish(
    ep: &mut Endpoint,
    a: &Args,
    game: mandrake_rec::writer::NewGameData,
    recorded: &[Command],
    limit: i32,
    log: &dyn Fn(String),
    start: Instant,
) -> i32 {
    let horizon = ep.joiner().horizon();
    let end_tick = if limit > 0 {
        limit as u32 + END_GAME_COUNTDOWN
    } else {
        horizon.max(0) as u32
    };
    let bytes = film(game, recorded, end_tick, a.tag_name.as_bytes());
    if let Err(e) = std::fs::write(&a.film, &bytes) {
        log(format!("writing {}: {e}", a.film));
        return 1;
    }
    let mut counts = std::collections::BTreeMap::<u8, usize>::new();
    for c in recorded {
        *counts.entry(c.kind).or_default() += 1;
    }
    let summary: Vec<String> = counts
        .iter()
        .map(|(k, n)| format!("{} {n}", kind_name(*k)))
        .collect();
    log(format!(
        "film {} written: {} bytes, {} commands ({}), end tick {end_tick}, horizon {horizon}",
        a.film,
        bytes.len(),
        recorded.len(),
        summary.join(", ")
    ));
    let until = Instant::now() + Duration::from_secs_f64(a.postgame);
    while Instant::now() < until {
        ep.poll(start.elapsed().as_millis() as u64);
        std::thread::sleep(Duration::from_millis(10));
    }
    0
}
