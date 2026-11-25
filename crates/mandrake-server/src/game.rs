use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use mandrake_common::game_options::COOPERATIVE;
use mandrake_common::{FourCC, NativeVfs};
use mandrake_net::endpoint::{Endpoint, Event, HostEvent, HostOptions};
use mandrake_net::game_description::GameDescription;
use mandrake_net::game_kind::GameKind;
use mandrake_net::lobby::code;
use mandrake_net::ours::Ours;
use mandrake_net::player_data::{JoinRequest, PlayerData};
use mandrake_rec::Recording;
use mandrake_rec::writer::NewGameData;
use mandrake_session::net_game::{FrameStep, NetLoop, PollClock, log_host_event, machine_ticks};
use mandrake_sim::engine_rng::GameParameters;
use mandrake_sim::game_speed::{GameMode, GameSpeed, NORMAL_INDEX, NetGate};
use mandrake_sim::net_game::NetRole;
use mandrake_sim::{MapSource, Simulation};
use mandrake_tag::{MeshTag, TagArchive, engine_tag_files};
use tracing::{info, warn};

use crate::autohost::{self, Action, Autohost, Seat};
use crate::config::Config;
use crate::owner::{Owner, Policy};

const COLORS: [u16; 8] = [0; 8];
const POSTGAME: Duration = Duration::from_secs(2);
const OWNER_POSTGAME: Duration = Duration::from_secs(30 * 60);
pub const AUTOHOST_RESTART: Duration = Duration::from_secs(30);
pub(crate) const AUTOHOST_RESTART_LINE: &[u8] = b"Game will restart in 30 seconds.";
const ENDI_LINE_DELAY: Duration = Duration::from_millis(250);
const AUTOHOST_LINE_WAIT: Duration = Duration::from_millis(500);
const FORCED_RESTART: i16 = 3;
pub(crate) const FRAME: Duration = Duration::from_micros(4167);
const LOBBY_POLL: Duration = Duration::from_millis(5);
const POSTGAME_POLL: Duration = Duration::from_millis(10);
const LATENCY_LOG: Duration = Duration::from_secs(10);
const STOP_END_WAIT: Duration = Duration::from_millis(500);
const STOP_FORCED_END_WAIT: Duration = Duration::from_secs(5);
const STOP_FORCED_END_KIND: i16 = 2;

pub struct Server {
    pub config: Config,
    pub game_dir: PathBuf,
    pub description: GameDescription,
    pub games: u32,
    pub stop_at_tick: Option<u32>,
    pub max_games: Option<u32>,
    pub autohost_restart: Duration,
    pub ownerless: bool,
    table: Vec<mandrake_rec::header::MonsterInitializer>,
    pub(crate) tfl_sync_counter: i32,
    pub(crate) tfl_survivors: Vec<mandrake_tfl_sim::MonsterInitializer>,
    pub(crate) tfl_last_players: Vec<i16>,
}

pub(crate) enum Next {
    Leave,
    Lobby {
        endpoint: Endpoint,
        vets: Vec<u8>,
        start_now: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameResult {
    pub film: Option<PathBuf>,
    pub end_tick: u32,
    pub out_of_sync: usize,
    pub owner_left: bool,
    pub stopped: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hosted {
    pub games: Vec<GameResult>,
    pub timed_out: bool,
    pub refused: Vec<String>,
}

enum LobbyEnd {
    Start(NewGameData, Option<Box<Simulation>>),
    StartTfl(mandrake_rec::tfl::TflNewGame),
    TimedOut,
    OwnerLeft,
    Refused(String),
}

#[derive(Default)]
struct LobbyCarry {
    vets: Option<Vec<u8>>,
    start_now: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayPhase {
    Active,
    ForcedEnding,
    Barrier { deserted: bool },
}

impl Server {
    pub fn new(config: Config) -> anyhow::Result<Server> {
        let game_dir = config.game_dir()?;
        let mut description = config.description()?;
        match config.kind {
            GameKind::Tfl => {
                let name = crate::tfl_game::check_description_with_name(
                    &game_dir,
                    config.tfl_data_words,
                    &description,
                )
                .map_err(|e| anyhow!("[game]: {e} ({})", game_dir.display()))?;
                description.map_name = name;
            }
            GameKind::Myth2 => {
                crate::owner::check_description(&game_dir, &description)
                    .map_err(|e| anyhow!("[game]: {e} ({})", game_dir.display()))?;
            }
        }
        std::fs::create_dir_all(&config.films_dir)
            .with_context(|| format!("films_dir {}", config.films_dir.display()))?;
        Ok(Server {
            config,
            game_dir,
            description,
            games: 0,
            stop_at_tick: None,
            max_games: None,
            autohost_restart: AUTOHOST_RESTART,
            ownerless: false,
            table: Vec::new(),
            tfl_sync_counter: 0,
            tfl_survivors: Vec::new(),
            tfl_last_players: Vec::new(),
        })
    }

    fn policy(&self) -> Policy {
        Policy {
            password: self.config.game.password.as_bytes().to_vec(),
            max_players: self.description.parameters.max_players,
            tfl: (self.config.kind == GameKind::Tfl).then_some(self.config.tfl_data_words),
        }
    }

    fn options(&self) -> HostOptions {
        let name = self.config.name.as_bytes();
        let mut player = PlayerData::new(name, name, COLORS);
        player.field_22 = mandrake_net::ours::SERVER_MARK;
        if self.config.kind == GameKind::Tfl {
            player.version = mandrake_net::player_data::TFL_PLAYER_VERSION;
        }
        HostOptions {
            player: JoinRequest {
                name: name.to_vec(),
                password: Vec::new(),
                player,
            },
            password: self.config.game.password.as_bytes().to_vec(),
            description: self.description.clone(),
        }
    }

    pub fn host_one(
        &mut self,
        addr: SocketAddr,
        lobby_timeout: Option<Duration>,
    ) -> anyhow::Result<Hosted> {
        let clock = Instant::now();
        let mut endpoint = Endpoint::host_at(addr, self.options(), 0)
            .map_err(|e| anyhow!("listening on {addr}: {e}"))?;
        anyhow::ensure!(
            endpoint.as_host().is_some_and(|h| h.is_dedicated()),
            "the host isn't a dedicated server's"
        );
        let d = &self.description;
        info!(
            "lobby: listening on {:?}: \"{}\", map {:?}, type {}, time limit {}, at most {} players",
            endpoint.listen_addr(),
            String::from_utf8_lossy(&d.game_name),
            String::from_utf8_lossy(&d.parameters.mesh),
            d.parameters.game_type,
            d.parameters.time_limit,
            d.parameters.max_players
        );
        let options = d.parameters.options;
        let own = endpoint.joiner().local_record().flags;
        info!("lobby: the server is player 0, record flags {own:#x}, game options {options:#x}");
        let mut owner = Owner::default();
        let mut hosted = Hosted::default();
        let mut carry = LobbyCarry::default();
        loop {
            let end = self.lobby(
                &mut endpoint,
                clock,
                lobby_timeout,
                &mut owner,
                &carry,
                &mut hosted.refused,
            )?;
            let (game, preloaded) = match end {
                LobbyEnd::Start(game, sim) => (game, sim),
                LobbyEnd::StartTfl(game) => {
                    self.games += 1;
                    let (result, next) = self.play_tfl(endpoint, clock, game, &mut owner)?;
                    hosted.games.push(result);
                    match next {
                        Next::Lobby { endpoint: e, .. } => {
                            endpoint = e;
                            carry = LobbyCarry {
                                vets: Some(Vec::new()),
                                start_now: false,
                            };
                            continue;
                        }
                        Next::Leave => return Ok(hosted),
                    }
                }
                LobbyEnd::TimedOut => {
                    info!("lobby: no start in {lobby_timeout:?}");
                    hosted.timed_out = true;
                    endpoint.leave();
                    return Ok(hosted);
                }
                LobbyEnd::OwnerLeft => {
                    info!("lobby: the owner left: the game ends for everyone");
                    endpoint.leave();
                    return Ok(hosted);
                }
                LobbyEnd::Refused(why) => {
                    endpoint.leave();
                    hosted.games.push(GameResult::refused(why));
                    return Ok(hosted);
                }
            };
            self.games += 1;
            let (result, next) = self.play(endpoint, clock, game, preloaded, &mut owner)?;
            hosted.games.push(result);
            match next {
                Next::Lobby {
                    endpoint: e,
                    vets: v,
                    start_now: now,
                } => {
                    endpoint = e;
                    carry = LobbyCarry {
                        vets: Some(v),
                        start_now: now,
                    };
                }
                Next::Leave => return Ok(hosted),
            }
        }
    }

    fn lobby(
        &mut self,
        endpoint: &mut Endpoint,
        clock: Instant,
        lobby_timeout: Option<Duration>,
        owner: &mut Owner,
        carry: &LobbyCarry,
        refused: &mut Vec<String>,
    ) -> anyhow::Result<LobbyEnd> {
        let vets = &carry.vets;
        let opened = clock.elapsed();
        let policy = self.policy();
        let cooperative = endpoint
            .as_host()
            .is_some_and(|h| h.lobby().description().parameters.options & COOPERATIVE != 0);
        let mut rule = Autohost::new(opened.as_millis() as u64);
        let mut started = false;
        let mut preloaded: Option<(NewGameData, Box<Simulation>)> = None;
        let tfl = self.config.kind == GameKind::Tfl;
        if carry.start_now {
            let host = endpoint.as_host_mut().expect("a host");
            info!(
                "lobby: Restart: the next game starts at once (vets {} bytes)",
                vets.as_ref().map_or(0, Vec::len)
            );
            match self.start_loaded(host, vets) {
                Ok(loaded) => {
                    preloaded = loaded;
                    started = true;
                }
                Err(why) => refuse_start(host, owner, why, refused),
            }
            endpoint.flush();
        }
        let mut autohost_start: Option<(Vec<i32>, Instant, bool)> = None;
        let mut latencies_logged = Instant::now();
        loop {
            let now = clock.elapsed().as_millis() as u64;
            if lobby_timeout.is_some_and(|t| clock.elapsed() - opened > t) {
                return Ok(LobbyEnd::TimedOut);
            }
            for event in endpoint.poll(now) {
                match event {
                    Event::Configuration {
                        flags: 0,
                        code: code::START,
                        ..
                    } if started && tfl => {
                        return Ok(match endpoint.joiner().tfl_new_game() {
                            Some(Ok(g)) => LobbyEnd::StartTfl(g),
                            other => {
                                let why = format!("no TFL new-game data at the start: {other:?}");
                                warn!(
                                    "lobby: the start is refused, every player is dropped: {why}"
                                );
                                LobbyEnd::Refused(why)
                            }
                        });
                    }
                    Event::Configuration {
                        flags: 0,
                        code: code::START,
                        ..
                    } if started => {
                        let game = match endpoint.joiner().new_game_data() {
                            Some(Ok(g)) => g,
                            other => {
                                let why = format!("no new-game data at the start: {other:?}");
                                warn!(
                                    "lobby: the start is refused, every player is dropped: {why}"
                                );
                                return Ok(LobbyEnd::Refused(why));
                            }
                        };
                        endpoint.joiner_mut().new_map();
                        info!("game: starts, seed {:#x}", game.seed());
                        let sim = match preloaded.take() {
                            Some((g, sim)) if g == game => Some(sim),
                            Some(_) => {
                                warn!(
                                    "game: the start's new-game data isn't the one the map was \
                                     loaded from: it is loaded again"
                                );
                                None
                            }
                            None => None,
                        };
                        return Ok(LobbyEnd::Start(game, sim));
                    }
                    Event::Chat { ref chat, .. }
                        if mandrake_net::player_data::until_nul(&chat.name) == autohost::NAME =>
                    {
                        if let Some((_, _, back)) = &mut autohost_start {
                            *back = true;
                        }
                        log_event(&event);
                    }
                    other => log_event(&other),
                }
            }
            let host = endpoint.as_host_mut().expect("a host");
            for e in host.take_events() {
                match e {
                    HostEvent::Ours {
                        from,
                        ours: Ours::Game { kind },
                    } => owner.game(from, kind),
                    HostEvent::Ours {
                        from,
                        ours: Ours::Hello { version },
                    } => {
                        let in_lobby = !started && autohost_start.is_none() && !self.ownerless;
                        owner.hello(host, from, version, in_lobby, self.config.kind);
                        if owner.id().is_some()
                            && let Some(Action::Say(text)) = rule.abort()
                        {
                            info!("lobby: Autohost: {text}");
                            host.chat_as(autohost::NAME, text.as_bytes());
                        }
                        if let Some(url) = &self.config.download_url {
                            host.send_ours(from, Ours::Downloads(url.as_bytes().to_vec()));
                        }
                    }
                    HostEvent::Ours { from, ours } => {
                        owner.request(host, &self.game_dir, &policy, from, ours)
                    }
                    HostEvent::ConfigurationRequest {
                        from,
                        code: code::START,
                        ..
                    } if owner.is(from) && !started => {
                        let kind = self.config.kind;
                        let unready: Vec<u8> = host
                            .topology()
                            .joined()
                            .filter(|p| {
                                p.id != 0
                                    && p.id != u16::from(from)
                                    && !autohost::counts_ready(kind, p.data.ready)
                            })
                            .map(|p| p.id as u8)
                            .collect();
                        if unready.is_empty() {
                            info!(
                                "lobby: the owner's Start (after a game: {}, vets {} bytes)",
                                vets.is_some(),
                                vets.as_ref().map_or(0, Vec::len)
                            );
                            match self.start_loaded(host, vets) {
                                Ok(loaded) => {
                                    preloaded = loaded;
                                    started = true;
                                }
                                Err(why) => refuse_start(host, owner, why, refused),
                            }
                        } else {
                            warn!("lobby: the owner's Start refused: {unready:?} aren't ready");
                        }
                    }
                    HostEvent::PlayerLeft { id, .. } => {
                        let was_owner = owner.left(id);
                        log_host_event("", &e, None);
                        if was_owner {
                            return Ok(LobbyEnd::OwnerLeft);
                        }
                    }
                    other => log_host_event("", &other, None),
                }
            }
            let line_passed_on = autohost_start
                .as_ref()
                .is_some_and(|(_, said, back)| *back || said.elapsed() >= AUTOHOST_LINE_WAIT);
            if line_passed_on && let Some((unready, _, _)) = autohost_start.take() {
                for id in unready {
                    info!("lobby: player {id} isn't ready: removed");
                    host.kick(id as u8);
                }
                match self.start_loaded(host, vets) {
                    Ok(loaded) => {
                        preloaded = loaded;
                        started = true;
                        info!("lobby: Start");
                    }
                    Err(why) => {
                        refuse_start(host, owner, why, refused);
                        rule = Autohost::new(clock.elapsed().as_millis() as u64);
                    }
                }
            } else if owner.id().is_none() && !started && autohost_start.is_none() {
                let seats: Vec<Seat> = host
                    .topology()
                    .joined()
                    .map(|p| Seat {
                        id: i32::from(p.id),
                        team: p.data.team,
                        ready: autohost::counts_ready(self.config.kind, p.data.ready),
                    })
                    .collect();
                for action in rule.poll(now, &seats, 0, cooperative) {
                    match action {
                        Action::Say(text) => {
                            info!("lobby: Autohost: {text}");
                            host.chat_as(autohost::NAME, text.as_bytes());
                        }
                        Action::Start { unready } => {
                            autohost_start = Some((unready, Instant::now(), false))
                        }
                    }
                }
            }
            endpoint.flush();
            if latencies_logged.elapsed() >= LATENCY_LOG {
                latencies_logged = Instant::now();
                log_round_trips(endpoint);
            }
            endpoint.wait(LOBBY_POLL);
        }
    }

    fn play(
        &mut self,
        endpoint: Endpoint,
        clock: Instant,
        game: NewGameData,
        preloaded: Option<Box<Simulation>>,
        owner: &mut Owner,
    ) -> anyhow::Result<(GameResult, Next)> {
        let mut sim = match preloaded {
            Some(sim) => *sim,
            None => {
                let header = mandrake_net::client::film(game.clone(), &[], 0, b"Network Game");
                let loading = Instant::now();
                match start_sim_with(&header, &self.game_dir, &self.table) {
                    Ok(sim) => {
                        info!(
                            "game: the map is loaded in {} ms",
                            loading.elapsed().as_millis()
                        );
                        sim
                    }
                    Err(e) => {
                        let why = format!("{e:#}");
                        warn!("game: the start is refused, every player is dropped: {why}");
                        endpoint.leave();
                        return Ok((GameResult::refused(why), Next::Leave));
                    }
                }
            }
        };
        sim.recorded_commands = Some(Vec::new());
        let mut net = Net::new(endpoint, clock);
        net.core.map_loaded();
        let mut speed = GameSpeed::new(GameMode::Network, false, NORMAL_INDEX);
        let mut polls_clock = PollClock::new(net.core.elapsed());
        let mut film: Option<PathBuf> = None;
        let mut phase = PlayPhase::Active;
        let mut stop = Stop::default();
        let mut owner_notified = false;
        let make_result = |sim: &Simulation,
                           film: Option<PathBuf>,
                           owner_left: bool,
                           stopped: Option<String>| GameResult {
            film,
            end_tick: sim.current_tick,
            out_of_sync: sim.out_of_sync.len(),
            owner_left,
            stopped,
        };
        loop {
            let begun = Instant::now();
            if net.poll(&mut sim, owner) {
                info!("game: begins");
                polls_clock = PollClock::new(net.core.elapsed());
            }
            if let Some(kind) = net.force_end.take() {
                net.owner_force_end(&mut sim, owner, kind, stop.why.is_some());
            }
            let polls = if net.core.started {
                polls_clock.polls(net.core.elapsed())
            } else {
                0
            };
            let mut frame = net.core.frame(
                &mut SimNet(&mut sim),
                &mut Pace {
                    speed: &mut speed,
                    gate: &mut net.gate,
                },
                polls,
            );
            while frame.next_body(&mut SimNet(&mut sim)) {
                log_tick_commands(&sim);
                sim.step();
            }
            let step = frame.step;
            net.after_batch(&mut sim, step);
            if self.stop_at_tick.is_some_and(|t| sim.current_tick >= t) && sim.engine_stop.is_none()
            {
                sim.engine_stop = Some(mandrake_sim::engine_rng::Unmodelled::new(
                    0,
                    "a stop given by the test (stop_at_tick)",
                ));
            }
            let game_over = sim.engine.game.game_over;
            let in_barrier = matches!(phase, PlayPhase::Barrier { .. });
            if let (Some(why), None, false) = (&sim.engine_stop, &stop.why, game_over || in_barrier)
            {
                let why = why.to_string();
                warn!(
                    "game: the sim stopped at {why}, tick {}: the game ends for everyone",
                    sim.current_tick
                );
                let sent = phase == PlayPhase::ForcedEnding
                    || (net.core.started && stop.send_forced_end(&mut net));
                stop.why = Some(why);
                if sent {
                    phase = PlayPhase::ForcedEnding;
                } else {
                    warn!("game: no forced end can be sent: every player is dropped");
                    let end = sim.current_tick;
                    film = self.write_film(&game, &mut sim, end);
                    log_out_of_sync(&sim);
                    net.core.endpoint.leave();
                    return Ok((make_result(&sim, film, false, stop.why), Next::Leave));
                }
            }
            if phase == PlayPhase::ForcedEnding
                && stop.due(net.forced_end_stamped)
                && !net.core.is_over()
            {
                let end = net
                    .forced_end_tick
                    .map_or(sim.current_tick, |t| t.max(sim.current_tick));
                film = self.write_film(&game, &mut sim, end);
                net.core.map_over();
                phase = PlayPhase::Barrier { deserted: false };
            }
            if net.owner_left && !owner_notified {
                owner_notified = true;
                info!(
                    "game: the owner left at tick {}: the game ends for everyone",
                    sim.current_tick
                );
                if !game_over
                    && phase == PlayPhase::Active
                    && net.core.started
                    && stop.send_forced_end(&mut net)
                {
                    info!("game: the forced end goes out for the owner's leaving");
                    phase = PlayPhase::ForcedEnding;
                }
            }
            if owner_notified && !game_over && phase == PlayPhase::Active {
                let end = sim.current_tick;
                film = self.write_film(&game, &mut sim, end);
                log_out_of_sync(&sim);
                net.core.endpoint.leave();
                return Ok((make_result(&sim, film, true, stop.why), Next::Leave));
            }
            if !matches!(phase, PlayPhase::Barrier { .. }) {
                let deserted =
                    phase == PlayPhase::Active && net.core.started && alone(&net.core.endpoint);
                if game_over || deserted {
                    if deserted {
                        info!(
                            "game: every player has left at tick {}: the game ends",
                            sim.current_tick
                        );
                    }
                    let end = sim.current_tick;
                    film = self.write_film(&game, &mut sim, end);
                    net.core.map_over();
                    phase = PlayPhase::Barrier { deserted };
                }
            }
            let done = net.core.ended
                || matches!(
                    phase,
                    PlayPhase::Barrier { deserted }
                        if net.core.end_given_up()
                            || deserted
                            || (stop.sent.is_some() && alone(&net.core.endpoint))
                );
            if done {
                let oos = sim.out_of_sync.len();
                info!(
                    "game: done at tick {}: {} commands received, {} sent, {} out of sync",
                    sim.current_tick, net.core.received, net.core.sent, oos
                );
                log_out_of_sync(&sim);
                let owner_left = net.owner_left;
                let result = make_result(&sim, film, owner_left, stop.why);
                let forced = sim.net.as_ref().map_or(0, |n| n.forced_end);
                let more = self.max_games.is_none_or(|n| self.games < n);
                let (restart, start_now) = if !owner_left
                    && !alone(&net.core.endpoint)
                    && more
                    && forced == FORCED_RESTART
                {
                    info!("game: Restart: no endi, the next game starts at once");
                    (true, true)
                } else {
                    (
                        run_postgame(
                            &mut net,
                            owner,
                            owner_left,
                            more,
                            self.games,
                            self.autohost_restart,
                        ),
                        false,
                    )
                };
                self.table = sim.monster_initializers();
                if restart {
                    let vets = mandrake_rec::writer::initializer_block(&self.table);
                    return Ok((
                        result,
                        Next::Lobby {
                            endpoint: net.core.endpoint,
                            vets,
                            start_now,
                        },
                    ));
                }
                net.core.endpoint.leave();
                return Ok((result, Next::Leave));
            }
            if let Some(rest) = FRAME.checked_sub(begun.elapsed()) {
                net.core.endpoint.wait(rest);
            }
        }
    }

    fn start_loaded(
        &self,
        host: &mut mandrake_net::endpoint::Host,
        vets: &Option<Vec<u8>>,
    ) -> Result<Option<(NewGameData, Box<Simulation>)>, String> {
        let mut after_game = vets.is_some();
        if self.config.kind == GameKind::Tfl && after_game {
            let mut now: Vec<i16> = host
                .topology()
                .joined()
                .filter(|p| p.id != 0)
                .map(|p| p.id as i16)
                .collect();
            let mut last = self.tfl_last_players.clone();
            now.sort_unstable();
            last.sort_unstable();
            if now != last {
                info!(
                    "lobby: the players aren't the last game's ({last:?}, now {now:?}): no veterans"
                );
                after_game = false;
            }
        }
        host.set_after_game(after_game);
        host.set_vets(vets.clone());
        let ticks = machine_ticks(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default(),
        );
        if self.config.kind == GameKind::Tfl {
            crate::tfl_game::check_description(
                &self.game_dir,
                self.config.tfl_data_words,
                host.lobby().description(),
            )?;
            host.start(ticks);
            return Ok(None);
        }
        let game = match host.start_preview(ticks) {
            Some(Ok(g)) => g,
            other => return Err(format!("no new-game data for the start: {other:?}")),
        };
        let header = mandrake_net::client::film(game.clone(), &[], 0, b"Network Game");
        let loading = Instant::now();
        let sim =
            start_sim_with(&header, &self.game_dir, &self.table).map_err(|e| format!("{e:#}"))?;
        info!(
            "lobby: the map is loaded in {} ms, before the Start",
            loading.elapsed().as_millis()
        );
        host.start(ticks);
        Ok(Some((game, Box::new(sim))))
    }

    fn write_film(
        &self,
        game: &NewGameData,
        sim: &mut Simulation,
        end_tick: u32,
    ) -> Option<PathBuf> {
        let commands = sim.recorded_commands.clone().unwrap_or_default();
        let mut recorder = mandrake_rec::writer::Recorder::new(game.clone(), &self.table);
        for c in &commands {
            recorder.record_packet(c);
        }
        let data = recorder.finish(end_tick);
        let name = format!("Server Game {}", self.games);
        let file = mandrake_rec::writer::film_file(
            name.as_bytes(),
            mandrake_rec::writer::subgroup_for_name(name.as_bytes(), &[]),
            &data,
        );
        self.save_film("m2rec", &file, end_tick, commands.len())
    }

    pub(crate) fn save_film(
        &self,
        ext: &str,
        data: &[u8],
        end_tick: u32,
        commands: usize,
    ) -> Option<PathBuf> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = self
            .config
            .films_dir
            .join(format!("{:03}-{stamp}.{ext}", self.games));
        match std::fs::write(&path, data) {
            Ok(()) => {
                info!(
                    "game: the film ends at tick {end_tick} with {commands} commands: {}",
                    path.display()
                );
                Some(path)
            }
            Err(e) => {
                warn!("game: could not write film {}: {e}", path.display());
                None
            }
        }
    }
}

impl GameResult {
    pub(crate) fn refused(why: String) -> GameResult {
        GameResult {
            film: None,
            end_tick: 0,
            out_of_sync: 0,
            owner_left: false,
            stopped: Some(why),
        }
    }
}

fn refuse_start(
    host: &mut mandrake_net::endpoint::Host,
    owner: &Owner,
    why: String,
    refused: &mut Vec<String>,
) {
    warn!("lobby: the start is refused, the players stay in the lobby: {why}");
    const LINE: &[u8] = b"Cannot start: the map doesn't load on the server.";
    match owner.id() {
        Some(id) => host.chat_as_to(
            crate::owner::SERVER_NAME,
            mandrake_net::lobby::ChatTo::Player(u16::from(id)),
            LINE,
        ),
        None => host.chat_as(autohost::NAME, LINE),
    }
    refused.push(why);
}

#[derive(Default)]
struct Stop {
    why: Option<String>,
    sent: Option<Instant>,
}

impl Stop {
    fn send_forced_end(&mut self, net: &mut Net) -> bool {
        let [hi, lo] = STOP_FORCED_END_KIND.to_be_bytes();
        let sent = net
            .core
            .endpoint
            .joiner_mut()
            .send_command(mandrake_sim::net_game::FORCED_ENDGAME, vec![hi, lo, 0, 0]);
        if sent {
            self.sent = Some(Instant::now());
            net.core.endpoint.flush();
        }
        sent
    }

    fn due(&self, stamped: Option<Instant>) -> bool {
        match (self.sent, stamped) {
            (Some(_), Some(at)) => at.elapsed() >= STOP_END_WAIT,
            (Some(sent), None) => sent.elapsed() >= STOP_FORCED_END_WAIT,
            (None, _) => false,
        }
    }
}

mandrake_session::net_sim_adapters!(pub, Simulation, GameSpeed, NetGate);

struct Net {
    core: NetLoop,
    gate: NetGate,
    owner_left: bool,
    forced_end_stamped: Option<Instant>,
    forced_end_tick: Option<u32>,
    force_end: Option<i16>,
}

impl Net {
    fn new(endpoint: Endpoint, clock: Instant) -> Net {
        Net {
            core: NetLoop::new(endpoint, Box::new(move || clock.elapsed()), "game"),
            gate: NetGate::new_map(),
            owner_left: false,
            forced_end_stamped: None,
            forced_end_tick: None,
            force_end: None,
        }
    }

    fn host_events(&mut self, owner: &mut Owner, tick: Option<u32>) {
        for e in self.host().take_events() {
            match e {
                HostEvent::PlayerLeft { id, .. } => {
                    if owner.left(id) {
                        self.owner_left = true;
                    }
                }
                HostEvent::Ours {
                    from,
                    ours: Ours::ForceEnd { kind },
                } if tick.is_some() => {
                    if owner.is(from) && matches!(kind, 2 | 3) {
                        info!("game: the owner's forced end, kind {kind}, at tick {tick:?}");
                        self.force_end = Some(kind);
                    } else {
                        warn!("game: player {from}'s forced end, kind {kind}, ignored");
                    }
                    continue;
                }
                _ => {}
            }
            log_host_event("", &e, tick);
        }
    }

    fn owner_force_end(&mut self, sim: &mut Simulation, owner: &Owner, kind: i16, stopped: bool) {
        let over = sim.engine.game.game_over;
        let queued = self.core.started && !over && !stopped && sim.force_network_endgame(kind);
        info!(
            "game: the owner's forced end, kind {kind}, at tick {} (queued: {queued})",
            sim.current_tick
        );
        if !queued && let Some(id) = owner.id() {
            let line: &[u8] = if kind == 2 {
                b"Cannot rehost at this time."
            } else {
                b"Cannot restart at this time."
            };
            self.host().chat_as_to(
                crate::owner::SERVER_NAME,
                mandrake_net::lobby::ChatTo::Player(u16::from(id)),
                line,
            );
        }
    }

    fn host(&mut self) -> &mut mandrake_net::endpoint::Host {
        self.core.endpoint.as_host_mut().expect("a host")
    }

    fn poll(&mut self, sim: &mut Simulation, owner: &mut Owner) -> bool {
        let (forced_stamped, forced_tick) =
            (&mut self.forced_end_stamped, &mut self.forced_end_tick);
        let (news, events) = self.core.poll_events(Some(&mut SimNet(sim)), |c| {
            if c.kind == mandrake_sim::net_game::FORCED_ENDGAME {
                info!(
                    "game: forced end from player {} stamped tick {}",
                    c.player, c.tick
                );
                forced_stamped.get_or_insert_with(Instant::now);
                forced_tick.get_or_insert(c.tick.max(0) as u32);
            }
        });
        for event in &events {
            if !matches!(event, Event::Commands { .. } | Event::AllSynced { .. }) {
                log_event(event);
            }
        }
        self.host_events(owner, Some(sim.current_tick));
        self.core.end_poll(Some(&mut SimNet(sim)));
        news.started
    }

    fn after_batch(&mut self, sim: &mut Simulation, step: FrameStep) {
        sim.net_end_of_batch(step.requested);
        let tick = sim.current_tick;
        self.core.send_outgoing(&mut SimNet(sim), Vec::new(), |c| {
            tracing::debug!("game: sent {} at tick {tick}", c.command_type.name())
        });
        if self.core.lead_log_due(tick) {
            info!(
                "game: tick {tick} horizon {} lead {} fill {} starvations {}",
                sim.net_horizon().unwrap_or(0),
                self.gate.lead,
                u8::from(self.gate.fill),
                self.gate.starvations()
            );
        }
    }
}

impl Postgame for Net {
    fn poll(&mut self) -> Vec<HostEvent> {
        let (_, events) = self.core.poll_events(None, |_| {});
        for event in &events {
            log_event(event);
        }
        let host_events = self.host().take_events();
        self.core.end_poll(None);
        host_events
    }

    fn endpoint(&self) -> &Endpoint {
        &self.core.endpoint
    }

    fn endpoint_mut(&mut self) -> &mut Endpoint {
        &mut self.core.endpoint
    }
}

pub(crate) trait Postgame {
    fn poll(&mut self) -> Vec<HostEvent>;
    fn endpoint(&self) -> &Endpoint;
    fn endpoint_mut(&mut self) -> &mut Endpoint;
}

pub(crate) fn alone(endpoint: &Endpoint) -> bool {
    endpoint
        .as_host()
        .is_some_and(|h| h.topology().joined().all(|p| p.id == 0))
}

pub(crate) fn run_postgame(
    net: &mut impl Postgame,
    owner: &mut Owner,
    owner_left: bool,
    more_games: bool,
    games: u32,
    autohost_restart: Duration,
) -> bool {
    if owner_left {
        info!("game: the owner has left: the server leaves");
        return false;
    }
    if alone(net.endpoint()) || !more_games {
        if !more_games {
            info!("game: the last of {games} games: the server leaves");
        }
        let until = Instant::now() + POSTGAME;
        while Instant::now() < until {
            for e in net.poll() {
                if let HostEvent::PlayerLeft { id, .. } = e {
                    owner.left(id);
                }
                log_host_event("", &e, None);
            }
            net.endpoint_mut().wait(POSTGAME_POLL);
        }
        return false;
    }
    if owner.id().is_some() {
        postgame_with_owner(net, owner)
    } else {
        postgame_autohost(net, owner, autohost_restart)
    }
}

fn postgame_autohost(net: &mut impl Postgame, owner: &mut Owner, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    let say_at = Instant::now() + ENDI_LINE_DELAY.min(wait);
    let mut said = false;
    while Instant::now() < until {
        if !said && Instant::now() >= say_at {
            said = true;
            info!(
                "lobby: Autohost: {}",
                String::from_utf8_lossy(AUTOHOST_RESTART_LINE)
            );
            net.endpoint_mut()
                .as_host_mut()
                .expect("a host")
                .chat_as(autohost::NAME, AUTOHOST_RESTART_LINE);
        }
        for e in net.poll() {
            if let HostEvent::PlayerLeft { id, .. } = e {
                owner.left(id);
            }
            log_host_event("", &e, None);
        }
        if alone(net.endpoint()) {
            info!("game: every player has left endi");
            return false;
        }
        net.endpoint_mut().wait(POSTGAME_POLL);
    }
    info!("game: the autohost's Restart Game: everyone back to the lobby");
    net.endpoint_mut()
        .as_host_mut()
        .expect("a host")
        .restart_game();
    net.endpoint_mut().flush();
    true
}

fn postgame_with_owner(net: &mut impl Postgame, owner: &mut Owner) -> bool {
    info!("game: endi: waiting for the owner's Restart Game");
    let until = Instant::now() + OWNER_POSTGAME;
    while Instant::now() < until {
        for e in net.poll() {
            match e {
                HostEvent::ConfigurationRequest {
                    from,
                    code: code::START,
                    ..
                } if owner.is(from) => {
                    info!("game: the owner's Restart Game: everyone back to the lobby");
                    net.endpoint_mut()
                        .as_host_mut()
                        .expect("a host")
                        .restart_game();
                    net.endpoint_mut().flush();
                    return true;
                }
                HostEvent::PlayerLeft { id, .. } => {
                    let was_owner = owner.left(id);
                    log_host_event("", &e, None);
                    if was_owner {
                        info!("game: the owner left endi: the game ends for everyone");
                        return false;
                    }
                }
                other => log_host_event("", &other, None),
            }
        }
        if alone(net.endpoint()) {
            info!("game: every player has left endi");
            return false;
        }
        net.endpoint_mut().wait(POSTGAME_POLL);
    }
    info!("game: no Restart Game in {OWNER_POSTGAME:?}");
    false
}

pub fn start_sim(film: &[u8], game_dir: &Path) -> anyhow::Result<Simulation> {
    start_sim_with(film, game_dir, &[])
}

pub fn start_sim_with(
    film: &[u8],
    game_dir: &Path,
    carried: &[mandrake_rec::header::MonsterInitializer],
) -> anyhow::Result<Simulation> {
    start_sim_as(film, game_dir, carried, NetRole::Host)
}

pub fn start_sim_as(
    film: &[u8],
    game_dir: &Path,
    carried: &[mandrake_rec::header::MonsterInitializer],
    role: NetRole,
) -> anyhow::Result<Simulation> {
    let rec = Recording::from_bytes(film).map_err(|e| anyhow!("the film header: {e}"))?;
    let plugins: Vec<&str> = rec
        .header
        .active_plugins
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    let files = engine_tag_files(game_dir, &plugins);
    if !files.missing_plugins.is_empty() {
        bail!(
            "the game's plugins {:?} aren't in {}",
            files.missing_plugins,
            game_dir.display()
        );
    }
    if !files.complete() {
        warn!(
            "not the engine's tag set: missing patches {:?}",
            files.missing_patches
        );
    }
    let vfs = NativeVfs::new(".");
    let archives: Vec<TagArchive> = files
        .paths
        .iter()
        .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
        .collect();
    let refs: Vec<&TagArchive> = archives.iter().collect();
    let tags = |g: [u8; 4], id: FourCC| {
        refs.iter().rev().find_map(|a| {
            a.find_tag_by_id(FourCC::new(g), id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    };
    let mesh_id = rec.header.mesh_tag_id;
    let mesh_data = tags(*b"mesh", mesh_id)
        .ok_or_else(|| anyhow!("mesh {mesh_id} isn't in {}", game_dir.display()))?;
    let mesh = MeshTag::from_bytes(&mesh_data).map_err(|e| anyhow!("mesh {mesh_id}: {e}"))?;
    let params = GameParameters::from_film(film).unwrap_or(GameParameters::COMMAND_LINE);
    let map = MapSource {
        mesh_data: &mesh_data,
        mesh: &mesh,
        tags: &tags,
        params,
    };
    let (sim, set_up) = Simulation::start_network_game(film, role, carried, &map, &refs)
        .map_err(|e| anyhow!("{e}"))?;
    let draws = set_up.load.as_ref().expect("checked by the start");
    info!(
        "game: map {mesh_id} loaded: {} monsters, {} draws",
        set_up.monsters.as_ref().map_or(0, Vec::len),
        draws.total()
    );
    for (id, reason) in set_up.population.iter().flat_map(|p| &p.movement.missing) {
        warn!("unit {id} keeps the placeholder locomotion: {reason}");
    }
    Ok(sim)
}

pub(crate) fn log_event(event: &Event) {
    match event {
        Event::Player { player, new } => info!(
            "lobby: {} player {} \"{}\" ready {}",
            if *new { "new" } else { "changed" },
            player.id,
            String::from_utf8_lossy(&player.name),
            player.data.ready
        ),
        Event::PlayerLeft { id } => info!("player {id} left"),
        Event::Chat { from, chat } => info!(
            "chat from {from} \"{}\": {}",
            String::from_utf8_lossy(mandrake_net::player_data::until_nul(&chat.name)),
            String::from_utf8_lossy(&chat.text)
        ),
        Event::BeginSync { barrier } => info!("barrier {barrier} begins"),
        Event::Ignored(_) | Event::Malformed(_) | Event::Undecoded(_) | Event::StreamError(_) => {
            warn!("{event:?}")
        }
        Event::Commands { .. } | Event::Latencies(_) => {}
        other => tracing::debug!("{other:?}"),
    }
}

fn log_out_of_sync(sim: &Simulation) {
    for o in &sim.out_of_sync {
        warn!(
            "game: OUT OF SYNC at tick {} ({}:{}, seed {:#x})",
            o.tick, o.file, o.line, o.seed
        );
    }
}

fn log_round_trips(endpoint: &mut Endpoint) {
    let Some(host) = endpoint.as_host_mut() else {
        return;
    };
    let trips = host.take_round_trips();
    if !trips.is_empty() {
        let line: Vec<String> = trips
            .iter()
            .map(|(id, last, most, shown)| {
                format!("player {id}: last {last} ms, most {most} ms, shown {shown} ms")
            })
            .collect();
        info!("lobby: round trips: {}", line.join("; "));
    }
}

fn log_tick_commands(sim: &Simulation) {
    let tick = sim.current_tick;
    for c in sim.commands_by_tick.get(&tick).into_iter().flatten() {
        match c.command_type {
            mandrake_rec::CommandType::DropPlayer => {
                info!("game: tick {tick}: DROP_PLAYER {}", c.player_index)
            }
            mandrake_rec::CommandType::Chat => info!(
                "game: tick {tick}: chat from player {}",
                c.player_index as i8
            ),
            _ => {}
        }
    }
}
