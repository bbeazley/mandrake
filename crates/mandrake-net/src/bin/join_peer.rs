use std::collections::BTreeMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use mandrake_net::client::{COMMAND_SYNC_FORMATION, DEFAULT_FORMATION, END_GAME_COUNTDOWN, film};
use mandrake_net::endpoint::{Endpoint, Event};
use mandrake_net::lobby::{ChatTo, code};
use mandrake_net::player_data::{JoinRequest, PlayerData};
use mandrake_net::wire::{self, Command, Frame, Message, Splitter};

const TEAM_NAME: &[u8] = b"\xd5s Team";
const COLORS: [u16; 8] = [
    0x5998, 0x08f3, 0x2f4b, 0x0000, 0xc424, 0x8cb2, 0x1a27, 0x0000,
];

struct Args {
    join: String,
    name: String,
    film: String,
    tag_name: String,
    ready_after: f64,
    timeout: f64,
    check_pcap: Option<String>,
    chat: Option<String>,
    chat_at: f64,
}

fn args() -> Args {
    let mut a = Args {
        join: format!("127.0.0.1:{}", wire::DEFAULT_PORT),
        name: "Guest".into(),
        film: "Last Recording".into(),
        tag_name: "Last Recording".into(),
        ready_after: 2.0,
        timeout: 300.0,
        check_pcap: None,
        chat: None,
        chat_at: 5.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| usage(&flag));
        match flag.as_str() {
            "--join" => a.join = value(),
            "--name" => a.name = value(),
            "--film" => a.film = value(),
            "--tag-name" => a.tag_name = value(),
            "--ready-after" => a.ready_after = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--timeout" => a.timeout = value().parse().unwrap_or_else(|_| usage(&flag)),
            "--check-pcap" => a.check_pcap = Some(value()),
            "--chat" => a.chat = Some(value()),
            "--chat-at" => a.chat_at = value().parse().unwrap_or_else(|_| usage(&flag)),
            _ => usage(&flag),
        }
    }
    a
}

fn usage(flag: &str) -> ! {
    eprintln!(
        "bad argument {flag}\nusage: join_peer [--join ADDR] [--name NAME] [--film PATH] \
         [--tag-name NAME] [--ready-after S] [--timeout S] [--chat TEXT --chat-at S] \
         | --check-pcap FILE"
    );
    std::process::exit(64)
}

fn main() {
    let a = args();
    if let Some(p) = &a.check_pcap {
        std::process::exit(check_pcap(p));
    }
    std::process::exit(play(&a));
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

fn play(a: &Args) -> i32 {
    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let log = |what: String| println!("{:8} {what}", start.elapsed().as_millis());
    let Some(host): Option<SocketAddr> = a.join.to_socket_addrs().ok().and_then(|mut i| i.next())
    else {
        log(format!("bad address {}", a.join));
        return 2;
    };
    let request = JoinRequest {
        name: a.name.as_bytes().to_vec(),
        password: Vec::new(),
        player: PlayerData::new(a.name.as_bytes(), TEAM_NAME, COLORS),
    };
    let mut ep = match Endpoint::join(host, request, now()) {
        Ok(e) => e,
        Err(e) => {
            log(format!("connect {host}: {e}"));
            return 2;
        }
    };
    log(format!("connected to {host}"));
    let mut joined_at = None;
    let mut ready_sent = false;
    let mut chat_sent = false;
    let mut game = None;
    let mut load_at = None;
    let mut in_game = false;
    let mut formation_sent = false;
    let mut recorded: Vec<Command> = Vec::new();
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    let deadline = Duration::from_secs_f64(a.timeout);
    while start.elapsed() < deadline {
        let t = now();
        for e in ep.poll(t) {
            match &e {
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
                            *counts.entry(c.kind).or_default() += 1;
                            recorded.push(c.clone());
                        }
                    }
                }
                Event::Joined { id } => {
                    joined_at = Some(t);
                    log(format!("joined as player {id}"));
                }
                Event::Player { player, new } => log(format!(
                    "{} player {} \"{}\" ready {}",
                    if *new { "new" } else { "changed" },
                    player.id,
                    String::from_utf8_lossy(&player.name),
                    player.data.ready
                )),
                Event::Description(d) => log(format!(
                    "description: map {:?} type {} limit {} seed {:#x}, {} plugins, game \"{}\"",
                    String::from_utf8_lossy(&d.parameters.mesh),
                    d.parameters.game_type,
                    d.parameters.time_limit,
                    d.parameters.seed,
                    d.plugins.0.len(),
                    String::from_utf8_lossy(&d.game_name)
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
                        log("the game starts: loading".into());
                    }
                }
                Event::BeginSync { barrier } => {
                    log(format!("begin sync, barrier {barrier}"));
                    load_at = Some(t + if in_game { 0 } else { 300 });
                }
                Event::AllSynced { barrier } => {
                    log(format!("all synced, barrier {barrier}"));
                    if !in_game {
                        in_game = true;
                    } else {
                        let Some(g) = game.take() else { return 1 };
                        return finish(&mut ep, a, g, &recorded, &counts, &log);
                    }
                }
                Event::Chat { from, chat } => log(format!(
                    "chat from {from} \"{}\", words {:04x?}: {}",
                    String::from_utf8_lossy(chat.name.split(|&b| b == 0).next().unwrap_or(&[])),
                    chat.words,
                    String::from_utf8_lossy(&chat.text)
                )),
                Event::Denied { reason } => {
                    log(format!("denied: {}", String::from_utf8_lossy(reason)));
                    return 2;
                }
                Event::JoinTimedOut => {
                    log("no answer in 5 s".into());
                    return 2;
                }
                Event::HostLost => {
                    log("host lost".into());
                    return 1;
                }
                Event::Greeting(_)
                | Event::PlayerLeft { .. }
                | Event::Vets(_)
                | Event::Owner
                | Event::Downloads(_)
                | Event::BeginMidgame { .. }
                | Event::SavedGame { .. }
                | Event::Latencies(_) => log(format!("{e:?}")),
                Event::Ignored(_)
                | Event::Malformed(_)
                | Event::Undecoded(_)
                | Event::StreamError(_) => log(format!("PROBLEM {e:?}")),
            }
        }
        let j = ep.joiner_mut();
        if let Some(text) = &a.chat
            && !chat_sent
            && t >= (a.chat_at * 1000.0) as u64
            && j.local_id().is_some()
        {
            chat_sent = j.chat(ChatTo::All, text.as_bytes());
            log(format!("chat sent: {text}"));
        }
        if !ready_sent
            && j.description().is_some()
            && joined_at.is_some_and(|at| t >= at + (a.ready_after * 1000.0) as u64)
        {
            ready_sent = j.set_ready(true);
            log("ready".into());
        }
        if load_at.is_some_and(|at| t >= at) {
            load_at = None;
            if j.loaded() {
                log("barrier answered".into());
            }
        }
        if in_game && !formation_sent && j.horizon() > 0 {
            formation_sent = j.send_command(COMMAND_SYNC_FORMATION, DEFAULT_FORMATION.to_vec());
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
    counts: &BTreeMap<u8, usize>,
    log: &dyn Fn(String),
) -> i32 {
    let horizon = ep.joiner().horizon();
    let limit = ep
        .joiner()
        .description()
        .map_or(0, |d| d.parameters.time_limit);
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
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        ep.poll(0);
        std::thread::sleep(Duration::from_millis(10));
    }
    0
}

fn check_pcap(path: &str) -> i32 {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{path}: {e}");
            return 1;
        }
    };
    if data.len() < 24 {
        eprintln!("{path}: not a pcap");
        return 1;
    }
    let le = data[0] == 0xd4 || data[0] == 0x4d;
    let u32_at = |at: usize| {
        let b: [u8; 4] = data[at..at + 4].try_into().unwrap();
        if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let skip = match u32_at(20) {
        1 => 14,
        113 => 16,
        0 => 4,
        other => {
            eprintln!("link type {other}");
            return 1;
        }
    };
    let port = wire::DEFAULT_PORT;
    let mut streams: BTreeMap<(u16, u16), (Option<u32>, Vec<u8>)> = BTreeMap::new();
    let mut udp = 0;
    let mut at = 24;
    while at + 16 <= data.len() {
        let incl = u32_at(at + 8) as usize;
        let Some(frame) = data.get(at + 16..at + 16 + incl) else {
            break;
        };
        at += 16 + incl;
        let Some(ip) = frame.get(skip..) else {
            continue;
        };
        if ip.len() < 20 || ip[0] >> 4 != 4 {
            continue;
        }
        let ihl = (ip[0] & 15) as usize * 4;
        let total = (u16::from_be_bytes([ip[2], ip[3]]) as usize).min(ip.len());
        let body = &ip[ihl..total];
        if body.len() < 8 {
            continue;
        }
        let (sport, dport) = (
            u16::from_be_bytes([body[0], body[1]]),
            u16::from_be_bytes([body[2], body[3]]),
        );
        if sport != port && dport != port {
            continue;
        }
        if ip[9] == 17 {
            udp += 1;
            continue;
        }
        if ip[9] != 6 || body.len() < 20 {
            continue;
        }
        let seq = u32::from_be_bytes(body[4..8].try_into().unwrap());
        let payload = &body[((body[12] >> 4) as usize * 4).min(body.len())..];
        let (next, bytes) = streams.entry((sport, dport)).or_default();
        if body[13] & 0x02 != 0 {
            *next = Some(seq.wrapping_add(1));
            continue;
        }
        let Some(n) = *next else { continue };
        let old = n.wrapping_sub(seq) as usize;
        if !payload.is_empty() && old < payload.len() {
            bytes.extend_from_slice(&payload[old..]);
            *next = Some(n.wrapping_add((payload.len() - old) as u32));
        }
    }
    let mut bad = 0;
    let mut counts: BTreeMap<(&str, u16), usize> = BTreeMap::new();
    for ((sport, _), (_, bytes)) in &streams {
        let dir = if *sport == port { "H>J" } else { "J>H" };
        let mut s = Splitter::with_greeting();
        s.push(bytes);
        loop {
            match s.next_frame() {
                Ok(Some(Frame::Greeting(p))) => println!("{dir} greeting {p}"),
                Ok(Some(Frame::Message(m))) => match Message::decode(&m) {
                    Ok(msg) => {
                        let back = msg.encode().ok();
                        if back.as_deref() != Some(&m[..]) {
                            bad += 1;
                            println!("{dir} does not encode back: {:02x?}", &m[..m.len().min(32)]);
                        }
                        *counts.entry((dir, msg.body.kind())).or_default() += 1;
                    }
                    Err(e) => {
                        bad += 1;
                        println!("{dir} UNDECODED {e}: {:02x?}", &m[..m.len().min(32)]);
                    }
                },
                Ok(None) => break,
                Err(e) => {
                    bad += 1;
                    println!("{dir} STREAM ERROR {e}");
                    break;
                }
            }
        }
        if s.pending() != 0 {
            println!("{dir} {} bytes left over at the end", s.pending());
        }
    }
    for ((dir, kind), n) in &counts {
        println!("{dir} type {kind:#06x}: {n}");
    }
    println!("udp datagrams: {udp}");
    println!(
        "{} messages, {bad} that don't decode",
        counts.values().sum::<usize>()
    );
    i32::from(bad != 0)
}
