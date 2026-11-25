use crate::clock::Instant;
use std::net::{SocketAddr, ToSocketAddrs};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
use std::time::Duration;

use mandrake_net::endpoint::{Endpoint, Event, HostEvent, HostOptions};
use mandrake_net::game_description::GameDescription;
use mandrake_net::game_kind::GameKind;
use mandrake_net::lobby::{BarrierKind, code};
use mandrake_net::player_data::JoinRequest;
use mandrake_rec::CommandPacket;
use mandrake_rec::writer::NewGameData;
use mandrake_sim::Simulation;
use mandrake_sim::net_game::NetRole;
use tracing::{info, warn};

use crate::args::Args;

#[path = "net_game_download.rs"]
pub(crate) mod download;
#[path = "net_game_orders.rs"]
mod orders;
#[path = "net_game_save.rs"]
pub(crate) mod save;
use crate::game_speed::{GameMode, GameSpeed, NORMAL_INDEX, NetGate};
use mandrake_session::net_game::PollClock;

const LOBBY_LOG: Duration = Duration::from_secs(1);
pub(crate) const NETWORKING_STRINGS: mandrake_common::FourCC = mandrake_common::FourCC(*b"inmn");
pub(crate) const STRING_PLAYER_LEFT: usize = 7;
pub(crate) const INTERNAL_STRINGS: mandrake_common::FourCC = mandrake_common::FourCC(*b"inns");
pub(crate) const STRING_HOST_LOST: usize = 20;
pub(crate) const CATCH_UP_TICKS: u32 = 30;
const READY_AFTER: Duration = Duration::from_millis(250);
const HOST_POSTGAME: Duration = Duration::from_secs(2);

pub(crate) fn join_request(args: &Args) -> JoinRequest {
    let mut settings = crate::settings::user_settings_path()
        .and_then(|path| match crate::settings::load_net_settings(&path) {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(
                    "settings: {}: {e}; the [net] table is ignored",
                    path.display()
                );
                None
            }
        })
        .unwrap_or_else(crate::settings::net_defaults);
    if let Some(name) = &args.net_name {
        settings.set_name(&mandrake_session::Text(name.as_bytes().to_vec()));
    }
    JoinRequest {
        name: settings.name_text().0,
        password: args
            .net_password
            .as_ref()
            .map(|p| p.as_bytes().to_vec())
            .unwrap_or_default(),
        player: settings.player_data(),
    }
}

pub(crate) fn resolve(address: &str) -> anyhow::Result<SocketAddr> {
    address
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("{address}: {e}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("{address}: no address"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Connecting {
    result: mpsc::Receiver<std::io::Result<Endpoint>>,
    pub(crate) host_name: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl Connecting {
    pub(crate) fn resolve_and_start(address: String, request: JoinRequest, now_ms: u64) -> Self {
        let (send, result) = mpsc::channel();
        let host_name = download::host_of(&address).to_string();
        std::thread::spawn(move || {
            let joined = resolve(&address)
                .map_err(std::io::Error::other)
                .and_then(|host| {
                    info!("net: connecting to {host}");
                    Endpoint::join(host, request, now_ms)
                });
            let _ = send.send(joined);
        });
        Self { result, host_name }
    }

    pub(crate) fn poll(&self) -> Option<std::io::Result<Endpoint>> {
        match self.result.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(std::io::Error::other(
                "the connect's thread ended with no answer",
            ))),
        }
    }
}

pub(crate) use mandrake_session::net_game::NetNews;
use mandrake_session::net_game::{FrameStep, NetFrame, NetLoop};

mandrake_session::net_sim_adapters!(pub(crate), Simulation, GameSpeed, NetGate);

#[derive(Default)]
struct MapStats {
    cost_frames: u32,
    cost_sum: Duration,
    cost_max: Duration,
    frame_cost: Duration,
    in_flight: std::collections::VecDeque<Instant>,
    stamped_own: std::collections::VecDeque<(u32, Instant, Instant)>,
    rtt_count: u32,
    rtt_back_sum: u128,
    rtt_back_max: u128,
    rtt_ran_sum: u128,
    rtt_ran_max: u128,
    last_frame: Option<Instant>,
    frame_count: u32,
    frame_sum: Duration,
    frame_max: Duration,
}

impl MapStats {
    fn record_frame(&mut self, end: Instant) {
        if let Some(last) = self.last_frame {
            let d = end - last;
            self.frame_count += 1;
            self.frame_sum += d;
            self.frame_max = self.frame_max.max(d);
        }
        self.last_frame = Some(end);
    }

    fn take_frames(&mut self) -> (u32, Duration, Duration) {
        let out = (self.frame_count, self.frame_sum, self.frame_max);
        self.frame_count = 0;
        self.frame_sum = Duration::ZERO;
        self.frame_max = Duration::ZERO;
        out
    }

    fn take_cost(&mut self) -> (u128, u128) {
        let frames = std::mem::take(&mut self.cost_frames);
        let sum = std::mem::take(&mut self.cost_sum);
        let most = std::mem::take(&mut self.cost_max);
        (
            sum.as_micros() / u128::from(frames.max(1)),
            most.as_micros(),
        )
    }

    fn take_rtt(&mut self) -> (u32, u128, u128, u128, u128) {
        (
            std::mem::take(&mut self.rtt_count),
            std::mem::take(&mut self.rtt_back_sum),
            std::mem::take(&mut self.rtt_back_max),
            std::mem::take(&mut self.rtt_ran_sum),
            std::mem::take(&mut self.rtt_ran_max),
        )
    }
}

fn game_save_info(game: Option<&NewGameData>) -> (bool, [u8; 4]) {
    let coop = game.is_some_and(|g| g.0.get(7).is_some_and(|o| o & 0x80 != 0));
    let mesh = game
        .and_then(|g| g.0.get(0x0c..0x10)?.try_into().ok())
        .unwrap_or_default();
    (coop, mesh)
}

fn find_local_player(w: &mandrake_sim::engine_rng::EngineWorld, local_id: u8) -> Option<usize> {
    let id = i32::from(local_id);
    w.players
        .players
        .iter()
        .rposition(|p| p.id == id && p.flags & 1 != 0 && p.field_09 == 0)
}

pub(crate) struct NetPlay {
    core: NetLoop,
    pub(crate) gate: NetGate,
    pub(crate) game: Option<NewGameData>,
    pub(crate) local_id: u8,
    pub(crate) role: NetRole,
    forward: bool,
    session_events: Vec<mandrake_session::net::NetEvent>,
    stats: MapStats,
    orders: Option<orders::Orders>,
    forced_end_told: bool,
    game_over_told: bool,
    pub(crate) trading: (bool, bool),
    lobby_logged: Option<Instant>,
    pub(crate) window_occluded: Option<bool>,
    pub(crate) joined_host: String,
    pub(crate) download: Option<download::Download>,
    waker_set: bool,
    barrier_waiting: Option<i32>,
    pub(crate) postgame: Option<Box<Simulation>>,
    pub(crate) joining: bool,
    asked_at: u32,
    pub(crate) restore: Option<save::Restore>,
    pub(crate) host_save: Option<save::HostRestore>,
    save_told: (Option<bool>, Option<u32>),
}

impl std::ops::Deref for NetPlay {
    type Target = NetLoop;
    fn deref(&self) -> &NetLoop {
        &self.core
    }
}

impl NetPlay {
    pub(crate) fn new(endpoint: Endpoint, clock: Instant, forward: bool) -> Self {
        let local_id = endpoint.joiner().local_id().unwrap_or(0);
        let role = if endpoint.as_host().is_some() {
            NetRole::Host
        } else {
            NetRole::Joiner
        };
        Self {
            core: NetLoop::new(endpoint, Box::new(move || clock.elapsed()), "net"),
            gate: NetGate::new_map(),
            game: None,
            local_id,
            role,
            forward,
            session_events: Vec::new(),
            stats: MapStats::default(),
            orders: None,
            forced_end_told: false,
            game_over_told: false,
            trading: (false, false),
            lobby_logged: None,
            window_occluded: None,
            joined_host: String::new(),
            download: None,
            waker_set: false,
            barrier_waiting: None,
            postgame: None,
            joining: false,
            asked_at: 0,
            restore: None,
            host_save: None,
            save_told: (None, None),
        }
    }

    pub(crate) fn is_host(&self) -> bool {
        self.role == NetRole::Host
    }

    pub(crate) fn ensure_waker(&mut self, waker: &mandrake_net::transport::Waker) {
        if !self.waker_set {
            self.core.endpoint.set_waker(Some(waker.clone()));
            self.waker_set = true;
        }
    }

    pub(crate) fn host_mut(&mut self) -> Option<&mut mandrake_net::endpoint::Host> {
        self.core.endpoint.as_host_mut()
    }

    pub(crate) fn as_owner(
        &mut self,
        send: impl FnOnce(&mut mandrake_net::endpoint::Joiner) -> bool,
    ) -> bool {
        if self.is_host() || !self.core.endpoint.joiner().is_owner() {
            return false;
        }
        let sent = send(self.core.endpoint.joiner_mut());
        self.core.endpoint.flush();
        sent
    }

    pub(crate) fn set_internal_messages(&mut self, lines: Vec<Vec<u8>>) {
        self.core.endpoint.set_internal_messages(lines);
    }

    pub(crate) fn chat(&mut self, to: mandrake_net::lobby::ChatTo, text: &[u8]) -> bool {
        let sent = self.core.endpoint.joiner_mut().chat(to, text);
        self.core.endpoint.flush();
        sent
    }

    fn host_events(&mut self, tick: Option<u32>) {
        let Some(host) = self.core.endpoint.as_host_mut() else {
            return;
        };
        for e in host.take_events() {
            mandrake_session::net_game::log_host_event("net: host: ", &e, tick);
        }
        self.tell_barrier_waiting();
    }

    fn tell_barrier_waiting(&mut self) {
        let Some(host) = self.core.endpoint.as_host() else {
            return;
        };
        let lobby = host.lobby();
        let waiting = (lobby.barrier() == Some(BarrierKind::End))
            .then(|| {
                host.topology()
                    .joined_ids()
                    .into_iter()
                    .find(|id| !lobby.arrived().contains(id))
            })
            .flatten()
            .map(i32::from);
        if waiting != self.barrier_waiting {
            self.barrier_waiting = waiting;
            if self.forward {
                self.session_events
                    .push(mandrake_session::net::NetEvent::BarrierWaiting(waiting));
            }
        }
    }

    pub(crate) fn begin_map(&mut self, game: Option<NewGameData>) {
        self.game = game.or_else(|| {
            self.core
                .endpoint
                .joiner()
                .new_game_data()
                .and_then(Result::ok)
        });
        self.local_id = self.core.endpoint.joiner().local_id().unwrap_or(0);
        self.core.begin_map();
        self.gate = NetGate::new_map();
        self.stats = MapStats::default();
        self.barrier_waiting = None;
        self.asked_at = 0;
        self.forced_end_told = false;
        self.game_over_told = false;
        self.trading = (false, false);
        self.postgame = None;
        self.joining = self.core.endpoint.joiner().joining_mid_game();
        if self.joining {
            info!("net: joining the game in progress: the history first, muted");
        }
        self.restore = None;
        self.save_told.0 = None;
        let joiner = self.core.endpoint.joiner();
        if joiner.plays_network_save()
            && let Some(saved) = joiner.saved_game()
        {
            let end = saved.ticks;
            self.restore = if self.is_host() {
                self.host_save.clone().map(save::Restore::Host)
            } else {
                Some(save::Restore::Peer { end, told: 0 })
            };
            info!(
                "net: a network save restores to tick {end} ({}), muted",
                if self.is_host() {
                    "we replay it"
                } else {
                    "the host replays it"
                }
            );
        }
    }

    fn tell_save_state(&mut self, sim: &Simulation) {
        use mandrake_session::net::NetEvent;
        let (coop, mesh) = game_save_info(self.game.as_ref());
        let allowed = self.restore.is_none() && save::may_save(sim, self.is_host() && coop, mesh);
        if self.save_told.0 != Some(allowed) {
            self.save_told.0 = Some(allowed);
            self.session_events.push(NetEvent::SaveAllowed(allowed));
        }
        let tick = sim.current_tick;
        match &self.restore {
            Some(restore) if self.save_told.1 != Some(tick) => {
                self.save_told.1 = Some(tick);
                self.session_events.push(NetEvent::SaveRestoring {
                    ticks: tick,
                    end: restore.end(),
                });
            }
            None if self.save_told.1.is_some() => {
                self.save_told.1 = None;
                self.session_events.push(NetEvent::SaveRestored);
            }
            _ => {}
        }
    }

    pub(crate) fn restoring(&self) -> bool {
        self.restore.is_some()
    }

    fn restore_step(&mut self, sim: &Simulation) {
        let tick = sim.current_tick;
        let now = self.core.elapsed().as_millis() as u64;
        let started = self.core.started;
        let entries = match &self.restore {
            Some(save::Restore::Host(_)) => self
                .core
                .endpoint
                .joiner()
                .lobby_new_game_data()
                .and_then(Result::ok)
                .map(|g| save::entries(&g))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        match &mut self.restore {
            Some(save::Restore::Peer { told, .. }) => {
                if tick > *told
                    && self
                        .core
                        .endpoint
                        .joiner_mut()
                        .send_saved_game_progress(tick)
                {
                    *told = tick;
                }
            }
            Some(save::Restore::Host(h)) if started => {
                let Some(host) = self.core.endpoint.as_host_mut() else {
                    return;
                };
                match h.step(sim, host, &entries, now) {
                    save::HostStep::Sent { batch, last } => tracing::debug!(
                        "net: restore: {batch} of the save's commands sent, to tick {last}"
                    ),
                    save::HostStep::ToEnd => info!(
                        "net: restore: the save's {} commands sent in {} batches; to the end, tick {}",
                        h.sent.0, h.sent.1, h.end
                    ),
                    save::HostStep::Queued { commands } => info!(
                        "net: restore: at tick {tick}, {commands} restore commands and SAVE_LOADED sent for {} players",
                        entries.len()
                    ),
                    save::HostStep::Waiting => {}
                }
            }
            _ => {}
        }
        self.core.endpoint.flush();
    }

    pub(crate) fn catch_up_polls(&self, sim: &Simulation) -> Option<u32> {
        if self.restore.is_some() {
            return Some(
                sim.net_horizon()
                    .unwrap_or(0)
                    .saturating_sub(sim.current_tick)
                    .min(save::RESTORE_TICKS),
            );
        }
        self.joining.then(|| {
            sim.net_horizon()
                .unwrap_or(0)
                .saturating_sub(sim.current_tick)
                .min(CATCH_UP_TICKS)
        })
    }

    pub(crate) fn after_body(&mut self, sim: &mut Simulation) -> bool {
        if self.restore.is_some() {
            return self.after_restore_body(sim);
        }
        if !self.joining {
            return false;
        }
        let w = &mut sim.engine;
        let Some(index) = find_local_player(w, self.local_id) else {
            return false;
        };
        w.local_player = index as i16;
        self.joining = false;
        info!(
            "net: joined the game in progress at tick {}: game player {index}, team {}",
            sim.current_tick,
            w.local_team()
        );
        true
    }

    fn after_restore_body(&mut self, sim: &mut Simulation) -> bool {
        if !sim.save_loaded() {
            return false;
        }
        let Some(restore) = self.restore.take() else {
            return false;
        };
        let tick = sim.current_tick;
        {
            let w = &mut sim.engine;
            if let Some(index) = find_local_player(w, self.local_id) {
                w.local_player = index as i16;
            }
            info!(
                "net: the network save is restored at tick {tick} (its end {}): game player {}, \
                 team {}",
                restore.end(),
                w.local_player,
                w.local_team()
            );
        }
        true
    }

    fn ask_for_more(&mut self, sim: &Simulation) {
        if !self.joining || !self.core.is_loaded() {
            return;
        }
        let (tick, horizon) = (sim.current_tick, sim.net_horizon().unwrap_or(0));
        let due = (horizon <= tick + 1 && tick != self.asked_at) || sim.ticks_stopped();
        if due && self.core.endpoint.joiner_mut().request_more_commands() {
            info!("net: more of the history asked for at tick {tick} (horizon {horizon})");
            self.asked_at = tick;
        }
    }

    pub(crate) fn endpoint_game(&self) -> Option<NewGameData> {
        self.core
            .endpoint
            .joiner()
            .new_game_data()
            .and_then(Result::ok)
    }

    pub(crate) fn is_loaded(&self) -> bool {
        self.core.is_loaded()
    }

    pub(crate) fn set_ready(&mut self, ready: bool) {
        self.core.endpoint.joiner_mut().set_ready(ready);
        self.core.endpoint.flush();
    }

    pub(crate) fn team(&mut self, request: &mandrake_session::net::NetRequest) -> bool {
        use mandrake_session::net::NetRequest;
        let joiner = self.core.endpoint.joiner_mut();
        let sent = match *request {
            NetRequest::TeamButton(chosen) => joiner.team_button(chosen),
            NetRequest::TeamKick(id) => joiner.team_kick(id),
            NetRequest::TeamLock => joiner.team_lock(),
            _ => false,
        };
        self.core.endpoint.flush();
        sent
    }

    pub(crate) fn take_session_events(&mut self) -> Vec<mandrake_session::net::NetEvent> {
        let events = std::mem::take(&mut self.session_events);
        if self.core.ended {
            hosts_loss_in_barrier(events)
        } else {
            events
        }
    }

    pub(crate) fn poll_lobby(&mut self) {
        if self.postgame.is_some() {
            self.poll_postgame();
            return;
        }
        let (_, events) = self.core.poll_events(None, |_| {});
        for event in &events {
            self.note(event);
            if let Event::Commands { commands, .. } = event {
                warn!("net: {} stamped commands with no map", commands.len());
            }
        }
        self.host_events(None);
        self.core.end_poll(None);
        self.log_lobby_polls();
    }

    fn poll_postgame(&mut self) {
        let Some(mut sim) = self.postgame.take() else {
            return;
        };
        let (_, events) = self.core.poll_events(Some(&mut SimNet(&mut sim)), |_| {});
        for event in &events {
            self.note(event);
        }
        self.host_events(None);
        self.core.heartbeat(&mut SimNet(&mut sim));
        let kind = sim.net_postgame_system_commands();
        self.core
            .send_outgoing(&mut SimNet(&mut sim), Vec::new(), |c| {
                info!("net: sent {} in the postgame", c.command_type.name())
            });
        if kind != 0 && !self.forced_end_told {
            self.forced_end_told = true;
            info!("net: the forced end, kind {kind}, came in the postgame");
            if self.forward {
                self.session_events
                    .push(mandrake_session::net::NetEvent::ForcedEnd(kind));
            }
        }
        self.postgame = Some(sim);
        self.core.answer_barriers();
        self.log_lobby_polls();
    }

    pub(crate) fn force_postgame_end(&mut self, kind: i16) -> bool {
        let Some(sim) = self.postgame.as_mut() else {
            return false;
        };
        let queued = sim.force_network_endgame(kind);
        info!("net: the epilogue's forced end, kind {kind} (queued: {queued})");
        true
    }

    pub(crate) fn forwards(&self) -> bool {
        self.forward
    }

    fn log_lobby_polls(&mut self) {
        let end = Instant::now();
        self.stats.record_frame(end);
        if self.lobby_logged.is_some_and(|at| end - at < LOBBY_LOG) {
            return;
        }
        let first = self.lobby_logged.is_none();
        self.lobby_logged = Some(end);
        if first {
            return;
        }
        let (n, sum, most) = self.stats.take_frames();
        let occluded = match self.window_occluded {
            Some(true) => "yes",
            Some(false) => "no",
            None => "no window",
        };
        let pings = match self.core.endpoint.as_host_mut() {
            Some(host) => format!(
                "round trips (player, last, most, shown; ms) {:?}",
                host.take_round_trips()
            ),
            None => {
                let times = self.core.endpoint.take_pong_times();
                format!(
                    "pings answered by the socket in {:?} us, seen by this loop after {:?} us",
                    times.answered_us, times.seen_us
                )
            }
        };
        info!(
            "net: lobby: window hidden: {occluded}; polls {n}, {} us apart, at most {} us; \
             {pings}; latencies {:?}",
            sum.as_micros() / u128::from(n.max(1)),
            most.as_micros(),
            self.core.endpoint.joiner().latencies()
        );
    }

    fn note(&mut self, event: &Event) {
        match event {
            Event::BeginSync { barrier } => info!("net: barrier {barrier} begins"),
            Event::AllSynced { .. } | Event::HostLost => {}
            Event::PlayerLeft { id } => info!("net: player {id} left"),
            Event::Chat { from, chat } => info!(
                "net: chat from {from}: {}",
                String::from_utf8_lossy(&chat.text)
            ),
            Event::Ignored(_)
            | Event::Malformed(_)
            | Event::Undecoded(_)
            | Event::StreamError(_) => {
                warn!("net: {event:?}")
            }
            Event::Commands { .. } => {}
            other => tracing::debug!("net: {other:?}"),
        }
        if self.forward
            && let Some(e) = mandrake_session::net::NetEvent::from_endpoint(event)
        {
            self.session_events.push(e);
        }
    }

    pub(crate) fn film_header(&self) -> Vec<u8> {
        self.game
            .clone()
            .map(|g| mandrake_net::client::film(g, &[], 0, b"Network Game"))
            .unwrap_or_default()
    }

    pub(crate) fn map_loaded(&mut self) {
        self.core.map_loaded();
    }

    pub(crate) fn map_over(&mut self) {
        self.postgame = None;
        self.core.map_over();
    }

    pub(crate) fn end_given_up(&self) -> bool {
        self.core.end_given_up()
    }

    pub(crate) fn poll(&mut self, sim: &mut Simulation) -> NetNews {
        let begun = Instant::now();
        let (local_id, in_flight, stamped_own) = (
            self.local_id,
            &mut self.stats.in_flight,
            &mut self.stats.stamped_own,
        );
        let (news, events) = self.core.poll_events(Some(&mut SimNet(sim)), |c| {
            if c.player == local_id
                && c.kind != mandrake_net::client::COMMAND_NULL
                && let Some(sent) = in_flight.pop_front()
            {
                stamped_own.push_back((c.tick.max(0) as u32, sent, Instant::now()));
            }
        });
        for event in &events {
            self.note(event);
        }
        self.host_events(Some(sim.current_tick));
        self.core.end_poll(Some(&mut SimNet(sim)));
        sim.set_loaded_from_network_save(self.restore.is_some());
        self.stats.frame_cost += begun.elapsed();
        news
    }

    pub(crate) fn frame(
        &mut self,
        sim: &mut Simulation,
        speed: &mut GameSpeed,
        polls: u32,
    ) -> NetFrame {
        self.core.frame(
            &mut SimNet(sim),
            &mut Pace {
                speed,
                gate: &mut self.gate,
            },
            polls,
        )
    }

    pub(crate) fn after_batch(&mut self, sim: &mut Simulation, step: FrameStep) -> bool {
        let begun = Instant::now();
        sim.net_end_of_batch(step.requested);
        let forced = sim.net.as_ref().map_or(0, |n| n.forced_end);
        if forced != 0 && !self.forced_end_told {
            self.forced_end_told = true;
            info!(
                "net: the forced end, kind {forced}, ran at tick {}",
                sim.current_tick
            );
            if self.forward {
                self.session_events
                    .push(mandrake_session::net::NetEvent::ForcedEnd(forced));
            }
        }
        let over = sim.engine.game.flags & mandrake_sim::game_state::FLAG_GAME_OVER != 0;
        if over && !self.game_over_told {
            self.game_over_told = true;
            if self.forward {
                self.session_events
                    .push(mandrake_session::net::NetEvent::GameOver);
            }
        }
        if self.forward {
            self.tell_save_state(sim);
        }
        let tick = sim.current_tick;
        if self.restore.is_some() {
            self.restore_step(sim);
        } else if self.joining {
            self.ask_for_more(sim);
            self.core.endpoint.flush();
        } else {
            let mut orders = Vec::new();
            if let Some(o) = self.orders.as_mut() {
                for (kind, payload) in o.due(sim, self.local_id) {
                    orders.push(CommandPacket::parse(0, 0xff, kind, &payload));
                }
            }
            let in_flight = &mut self.stats.in_flight;
            self.core.send_outgoing(&mut SimNet(sim), orders, |c| {
                in_flight.push_back(Instant::now());
                info!("net: sent {} at tick {tick}", c.command_type.name());
            });
        }
        let frame = std::mem::take(&mut self.stats.frame_cost) + begun.elapsed();
        self.stats.cost_frames += 1;
        self.stats.cost_sum += frame;
        self.stats.cost_max = self.stats.cost_max.max(frame);
        let end = Instant::now();
        while let Some(&(at, sent, back)) = self.stats.stamped_own.front() {
            if at >= tick {
                break;
            }
            self.stats.stamped_own.pop_front();
            let (b, r) = ((back - sent).as_millis(), (end - sent).as_millis());
            self.stats.rtt_count += 1;
            self.stats.rtt_back_sum += b;
            self.stats.rtt_back_max = self.stats.rtt_back_max.max(b);
            self.stats.rtt_ran_sum += r;
            self.stats.rtt_ran_max = self.stats.rtt_ran_max.max(r);
        }
        self.stats.record_frame(end);
        if self.core.lead_log_due(tick) {
            let (mean, most) = self.stats.take_cost();
            let (n, bs, bm, rs, rm) = self.stats.take_rtt();
            let n1 = u128::from(n.max(1));
            let (fc, fs, fm) = self.stats.take_frames();
            info!(
                "net: LEAD tick {tick} horizon {} fill {} lead {} starvations {}; the polls: \
                 {mean} us a frame, at most {most} us; frames {fc}, {} us apart, at most {} us; \
                 own commands {n}: back in {} ms (at most {bm}), ran in {} ms (at most {rm})",
                sim.net_horizon().unwrap_or(0),
                u8::from(self.gate.fill),
                self.gate.lead,
                self.gate.starvations(),
                fs.as_micros() / u128::from(fc.max(1)),
                fm.as_micros(),
                bs / n1,
                rs / n1,
            );
            return true;
        }
        false
    }

    pub(crate) fn load_orders(&mut self, args: &Args) -> anyhow::Result<()> {
        if let Some(film) = &args.net_orders {
            self.orders = Some(orders::Orders::load(
                film,
                args.net_orders_host.as_deref(),
                self.is_host(),
            )?);
        }
        Ok(())
    }

    pub(crate) fn log_orders(&self) {
        if let Some(o) = &self.orders {
            info!(
                "net-orders: {} of ours sent, {} of the other side's written, all out: {}",
                o.ours,
                o.host,
                o.done()
            );
        }
    }

    pub(crate) fn leave(self) {
        self.core.endpoint.leave();
    }
}

pub(crate) fn join_and_wait(args: &Args) -> anyhow::Result<NetPlay> {
    let Lobbied {
        mut endpoint,
        clock,
        midgame,
    } = join_lobby(args, GameKind::Myth2)?;
    let game = match endpoint.joiner().new_game_data() {
        Some(Ok(g)) => g,
        other => anyhow::bail!("net: no new-game data at the start: {other:?}"),
    };
    let local_id = endpoint.joiner().local_id().unwrap_or(0);
    endpoint.joiner_mut().new_map();
    info!(
        "net: the host starts the game: player {local_id}, seed {:#x}",
        game.seed()
    );
    let mut net = NetPlay::new(endpoint, clock, false);
    if !midgame {
        net.load_orders(args)?;
    }
    net.begin_map(Some(game));
    Ok(net)
}

pub(crate) struct Lobbied {
    pub(crate) endpoint: Endpoint,
    pub(crate) clock: Instant,
    pub(crate) midgame: bool,
}

pub(crate) fn join_lobby(args: &Args, kind: GameKind) -> anyhow::Result<Lobbied> {
    let address = args
        .net_join
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no --net-join"))?;
    let host = resolve(address)?;
    let request = join_request(args);
    info!(
        "net: joining {host} as \"{}\"",
        String::from_utf8_lossy(&request.name)
    );
    let clock = Instant::now();
    let mut endpoint = Endpoint::join_as(kind, host, request, 0)
        .map_err(|e| anyhow::anyhow!("net: the connect to {host}: {e}"))?;
    endpoint.set_internal_messages(internal_strings(&args.archive));
    info!(
        "net: connected to {host} in {} ms",
        clock.elapsed().as_millis()
    );
    let deadline = Duration::from_secs_f64(args.net_timeout);
    let mut ready = false;
    let game_dir = args
        .archive
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let downloads = crate::settings::download_settings();
    if downloads.allow {
        mandrake_tag::tag_files::set_downloads_dir(crate::settings::downloads_dir());
    }
    let mut download_url: Option<String> = None;
    let mut download: Option<download::Download> = None;
    let mut downloaded = false;
    let mut described_at: Option<Instant> = None;
    const URL_WAIT: Duration = Duration::from_secs(3);
    let mut all_ready_since: Option<Instant> = None;
    let mut start_sent = false;
    while clock.elapsed() < deadline {
        let now = clock.elapsed().as_millis() as u64;
        for event in endpoint.poll(now) {
            match event {
                Event::Joined { id } => info!("net: joined as player {id}"),
                Event::Description(d) => info!(
                    "net: the game \"{}\": map {:?}, type {}, time limit {}",
                    String::from_utf8_lossy(&d.game_name),
                    String::from_utf8_lossy(&d.parameters.mesh),
                    d.parameters.game_type,
                    d.parameters.time_limit
                ),
                Event::Player { player, new } => info!(
                    "net: {} player {} \"{}\" ready {}",
                    if new { "new" } else { "changed" },
                    player.id,
                    String::from_utf8_lossy(&player.name),
                    player.data.ready
                ),
                Event::Configuration {
                    flags: 0,
                    code: code::START,
                    ..
                } => {
                    return Ok(Lobbied {
                        endpoint,
                        clock,
                        midgame: false,
                    });
                }
                Event::BeginMidgame { start } => {
                    info!(
                        "net: the game is in play: joining it as player {:?} with a start list \
                         of {} players",
                        endpoint.joiner().local_id(),
                        start.len() / mandrake_net::player_data::PLAYER_DATA_SIZE,
                    );
                    return Ok(Lobbied {
                        endpoint,
                        clock,
                        midgame: true,
                    });
                }
                Event::Denied { reason } => {
                    anyhow::bail!(
                        "net: the host said no: {}",
                        String::from_utf8_lossy(&reason)
                    )
                }
                Event::Owner => info!("net: we own this game on our server"),
                Event::Downloads(url) => {
                    let url = String::from_utf8_lossy(&url).into_owned();
                    info!("net: our server's plugins are at {url}");
                    download_url = Some(url);
                }
                Event::JoinTimedOut => anyhow::bail!("net: no answer from the host in 5 s"),
                Event::HostLost => anyhow::bail!("net: the host is gone"),
                other => tracing::debug!("net: {other:?}"),
            }
        }
        if let Some(d) = download.as_mut() {
            if let Some(done) = d.poll().done {
                download = None;
                match done {
                    Ok(()) => info!("download: done; reloading plugins from disk..."),
                    Err(e) => anyhow::bail!("download: failed: {e}"),
                }
            }
        } else if let Some(description) = endpoint.joiner().description()
            && let Err(why) = game_check(kind, description)
        {
            endpoint.leave();
            anyhow::bail!("net: can't join: {why}");
        } else if kind == GameKind::Myth2
            && !ready
            && !downloaded
            && let Some(description) = endpoint.joiner().description()
        {
            let missing = missing_plugins(&game_dir, description);
            let ours = endpoint.joiner().server_is_ours();
            let since = *described_at.get_or_insert_with(Instant::now);
            match (
                &download_url,
                downloads.allow,
                crate::settings::downloads_dir(),
            ) {
                _ if missing.is_empty() => {}
                (None, true, Some(_)) if ours && since.elapsed() < URL_WAIT => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                (Some(url), true, Some(dir)) => {
                    info!(
                        "download: {} plugins from {url}: {:?}",
                        missing.len(),
                        missing
                            .iter()
                            .map(|p| String::from_utf8_lossy(&p.name).into_owned())
                            .collect::<Vec<_>>()
                    );
                    download = Some(download::Download::start(
                        url,
                        download::host_of(address),
                        missing,
                        dir,
                        downloads.max_bytes,
                    ));
                    downloaded = true;
                    continue;
                }
                _ => warn!("net: {} of the game's plugins are missing", missing.len()),
            }
        }
        if download.is_some() {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let j = endpoint.joiner_mut();
        if !ready
            && j.local_id().is_some()
            && j.description().is_some()
            && described_at.get_or_insert_with(Instant::now).elapsed() >= READY_AFTER
        {
            if downloaded {
                let missing = missing_plugins(&game_dir, j.description().expect("checked"));
                anyhow::ensure!(
                    missing.is_empty(),
                    "download: {} plugins still missing",
                    missing.len()
                );
                info!("download: the game's plugins are all here now");
            }
            ready = j.set_ready(true);
            info!("net: ready");
        }
        if j.is_owner() && !start_sent {
            let me = j.local_id().map(u16::from);
            let players: Vec<_> = j.players().into_iter().filter(|p| p.id != 0).collect();
            let all = players.len() >= args.net_players
                && players
                    .iter()
                    .all(|p| Some(p.id) == me || p.data.ready != 0 || kind == GameKind::Tfl);
            match (all, all_ready_since) {
                (false, _) => all_ready_since = None,
                (true, None) => all_ready_since = Some(Instant::now()),
                (true, Some(at)) if at.elapsed().as_secs_f64() >= args.net_start_after => {
                    start_sent = j.owner_configure(code::START, 0);
                    info!("net: owner: Start (code 100) to our server");
                }
                (true, Some(_)) => {}
            }
        }
        endpoint.flush();
        endpoint.wait(Duration::from_millis(5));
    }
    anyhow::bail!("net: the host didn't start the game in {deadline:?}")
}

fn game_check(kind: GameKind, description: &GameDescription) -> Result<(), String> {
    let theirs = GameKind::of(description);
    if theirs != kind {
        return Err(
            String::from_utf8_lossy(&mandrake_session::net::other_game_text(theirs).0).into_owned(),
        );
    }
    if kind == GameKind::Tfl {
        mandrake_net::tfl::check_data(description, crate::tfl_live::film::CHECKSUMS).map_err(
            |m| String::from_utf8_lossy(&mandrake_session::net::tfl_data_text(m).0).into_owned(),
        )?;
    }
    Ok(())
}

pub(crate) fn host_game_description(args: &Args) -> anyhow::Result<GameDescription> {
    let name = args
        .net_name
        .clone()
        .unwrap_or_else(|| "Network Game".into());
    let password = args.net_password.clone().unwrap_or_default();
    let mut d = match &args.net_description {
        Some(path) => {
            let bytes = std::fs::read(path)
                .map_err(|e| anyhow::anyhow!("--net-description {}: {e}", path.display()))?;
            let m = mandrake_net::wire::Message::decode(&bytes)
                .map_err(|e| anyhow::anyhow!("--net-description {}: {e}", path.display()))?;
            GameDescription::from_body(&m.body)
                .map_err(|e| anyhow::anyhow!("--net-description {}: {e}", path.display()))?
        }
        None => mandrake_session::net::host_description(
            &mandrake_session::Text(name.into_bytes()),
            &mandrake_session::Text(password.as_bytes().to_vec()),
            0,
        ),
    };
    host_overrides(args, &mut d)?;
    Ok(d)
}

pub(crate) fn host_overrides(args: &Args, d: &mut GameDescription) -> anyhow::Result<()> {
    let password = args.net_password.clone().unwrap_or_default();
    if let Some(mesh) = &args.net_map {
        let b = mesh.as_bytes();
        anyhow::ensure!(
            b.len() == 4,
            "--net-map {mesh}: a mesh tag is four characters"
        );
        d.parameters.mesh.copy_from_slice(b);
    }
    if let Some(t) = args.net_game_type {
        d.parameters.game_type = t;
    }
    if let Some(t) = args.net_time_limit {
        d.parameters.time_limit = t;
    }
    if args.net_coop {
        d.parameters.options |= mandrake_net::game_description::OPTION_COOPERATIVE;
    }
    if password.is_empty() {
        d.flags &= !mandrake_net::game_description::FLAG_PASSWORD;
    } else {
        d.flags |= mandrake_net::game_description::FLAG_PASSWORD;
    }
    anyhow::ensure!(
        d.parameters.mesh != [0; 4],
        "--net-host: no map (--net-description or --net-map)"
    );
    Ok(())
}

pub(crate) fn machine_ticks() -> u32 {
    mandrake_session::net_game::machine_ticks(
        crate::clock::SystemTime::now()
            .duration_since(crate::clock::UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

pub(crate) fn host_and_wait(args: &Args) -> anyhow::Result<NetPlay> {
    let description = host_game_description(args)?;
    let mut host_save = None;
    let Lobbied {
        mut endpoint,
        clock,
        ..
    } = host_lobby(args, description, |endpoint| {
        let Some(path) = &args.net_load_save else {
            return Ok(());
        };
        let (saved, restore) = save::load_save(path)?;
        info!(
            "net: the lobby plays the network save {}: its end tick {}",
            path.display(),
            saved.ticks
        );
        let host = endpoint.as_host_mut().expect("a host");
        host.set_saved_game(Some(saved));
        host_save = Some(restore);
        Ok(())
    })?;
    let game = match endpoint.joiner().new_game_data() {
        Some(Ok(g)) => g,
        other => anyhow::bail!("net: no new-game data at the start: {other:?}"),
    };
    endpoint.joiner_mut().new_map();
    info!("net: the game starts: seed {:#x}", game.seed());
    let mut net = NetPlay::new(endpoint, clock, false);
    net.load_orders(args)?;
    net.host_save = host_save;
    net.begin_map(Some(game));
    Ok(net)
}

pub(crate) fn host_lobby(
    args: &Args,
    description: GameDescription,
    prepare: impl FnOnce(&mut Endpoint) -> anyhow::Result<()>,
) -> anyhow::Result<Lobbied> {
    let port = args
        .net_host
        .ok_or_else(|| anyhow::anyhow!("no --net-host"))?;
    let request = join_request(args);
    info!(
        "net: hosting on port {port} as \"{}\": map {:?}, type {}, time limit {}, {} players",
        String::from_utf8_lossy(&request.name),
        String::from_utf8_lossy(&description.parameters.mesh),
        description.parameters.game_type,
        description.parameters.time_limit,
        args.net_players
    );
    let options = HostOptions {
        player: request,
        password: args
            .net_password
            .as_ref()
            .map(|p| p.as_bytes().to_vec())
            .unwrap_or_default(),
        description,
    };
    let clock = Instant::now();
    let mut endpoint = Endpoint::host(port, options, 0)
        .map_err(|e| anyhow::anyhow!("net: listening on port {port}: {e}"))?;
    endpoint.set_internal_messages(internal_strings(&args.archive));
    info!("net: listening on {:?}", endpoint.listen_addr());
    prepare(&mut endpoint)?;
    let deadline = Duration::from_secs_f64(args.net_timeout);
    let mut ready_since: Option<Instant> = None;
    let mut started = false;
    while clock.elapsed() < deadline {
        let now = clock.elapsed().as_millis() as u64;
        for event in endpoint.poll(now) {
            match event {
                Event::Player { player, new } => info!(
                    "net: {} player {} \"{}\" ready {}",
                    if new { "new" } else { "changed" },
                    player.id,
                    String::from_utf8_lossy(&player.name),
                    player.data.ready
                ),
                Event::Configuration {
                    flags: 0,
                    code: code::START,
                    ..
                } => {
                    return Ok(Lobbied {
                        endpoint,
                        clock,
                        midgame: false,
                    });
                }
                Event::Chat { from, chat } => info!(
                    "net: chat from {from}: {}",
                    String::from_utf8_lossy(&chat.text)
                ),
                other => tracing::debug!("net: {other:?}"),
            }
        }
        let host = endpoint.as_host_mut().expect("a host");
        for e in host.take_events() {
            match e {
                HostEvent::Arrived { .. } | HostEvent::PlayerChanged { .. } => {}
                other => info!("net: host: {other:?}"),
            }
        }
        if !started {
            let players: Vec<_> = host.topology().joined().cloned().collect();
            let ready = players.len() >= args.net_players
                && players.iter().all(|p| p.id == 0 || p.data.ready != 0);
            match (ready, ready_since) {
                (true, None) => {
                    info!("net: {} players in and ready", players.len());
                    ready_since = Some(Instant::now());
                }
                (false, Some(_)) => ready_since = None,
                _ => {}
            }
            if ready_since.is_some_and(|at| at.elapsed().as_secs_f64() >= args.net_start_after) {
                host.start(machine_ticks());
                started = true;
                info!("net: host: Start");
            }
        }
        endpoint.flush();
        endpoint.wait(Duration::from_millis(5));
    }
    anyhow::bail!("net: the players weren't in and ready in {deadline:?}")
}

fn archive_string_list(archive: &std::path::Path, id: mandrake_common::FourCC) -> Vec<Vec<u8>> {
    archive
        .parent()
        .and_then(std::path::Path::parent)
        .map(|game_dir| string_list(game_dir, id))
        .unwrap_or_default()
}

pub(crate) fn networking_strings(archive: &std::path::Path) -> Vec<Vec<u8>> {
    archive_string_list(archive, NETWORKING_STRINGS)
}

pub(crate) fn internal_strings(archive: &std::path::Path) -> Vec<Vec<u8>> {
    archive_string_list(archive, INTERNAL_STRINGS)
}

fn missing_plugins(
    game_dir: &std::path::Path,
    description: &GameDescription,
) -> Vec<mandrake_net::game_description::Plugin> {
    let names: Vec<String> = description
        .plugins
        .0
        .iter()
        .map(|p| String::from_utf8_lossy(&p.name).into_owned())
        .collect();
    let installed: Vec<_> = crate::app::recording::plugin_infos(game_dir, &names)
        .into_iter()
        .map(|p| mandrake_net::game_description::Plugin {
            checksum: p.checksum,
            name: p.name.into_bytes(),
            url: p.url.into_bytes(),
        })
        .collect();
    mandrake_session::net::missing_plugins(description, &installed)
}

pub(crate) fn string_list(game_dir: &std::path::Path, id: mandrake_common::FourCC) -> Vec<Vec<u8>> {
    let tags = mandrake_tag::mesh_links::CampaignTags::open(game_dir);
    tags.base_search()
        .get(mandrake_common::FourCC(*b"stli"), id)
        .map(|raw| crate::unit_info::string_list(&raw))
        .unwrap_or_default()
}

pub(crate) fn networking_string(strings: &[Vec<u8>], index: usize, arg: &[u8]) -> Vec<u8> {
    let text = strings.get(index).cloned().unwrap_or_default();
    match text.windows(2).position(|w| w == b"%s") {
        Some(at) => [&text[..at], arg, &text[at + 2..]].concat(),
        None => text,
    }
}

pub(crate) fn run_headless_join(args: Args) -> anyhow::Result<()> {
    let started = Instant::now();
    let net = join_and_wait(&args)?;
    run_headless_game(args, net, started)
}

pub(crate) fn run_headless_host(args: Args) -> anyhow::Result<()> {
    let started = Instant::now();
    let net = host_and_wait(&args)?;
    run_headless_game(args, net, started)
}

fn hosts_loss_in_barrier(
    mut events: Vec<mandrake_session::net::NetEvent>,
) -> Vec<mandrake_session::net::NetEvent> {
    use mandrake_session::net::NetEvent;
    let synced = events.iter().position(|e| *e == NetEvent::AllSynced);
    let lost = events.iter().rposition(|e| *e == NetEvent::HostLost);
    if let (Some(synced), Some(lost)) = (synced, lost)
        && lost > synced
    {
        let loss = events.remove(lost);
        events.insert(synced, loss);
    }
    events
}

fn run_headless_game(args: Args, mut net: NetPlay, started: Instant) -> anyhow::Result<()> {
    let interface = crate::interface_config::load(args.conf.as_deref(), true);
    let start = crate::startup::start_network_level(&args, interface, net.film_header(), net.role)?;
    let names = start.film_names.clone();
    let strings = networking_strings(&args.archive);
    let mut sim = start.sim;
    info!(
        "net: the players by game player: {:?}, local player {}",
        names
            .iter()
            .map(|(n, _)| String::from_utf8_lossy(&n.0).into_owned())
            .collect::<Vec<_>>(),
        sim.local_player()
    );
    info!(
        "net: the map {} is loaded in {} ms",
        start.mesh_name,
        started.elapsed().as_millis()
    );
    sim.recorded_commands = Some(Vec::new());
    net.map_loaded();
    let mut speed = GameSpeed::new(GameMode::Network, false, NORMAL_INDEX);
    let mut clock = PollClock::new(net.elapsed());
    let deadline = Duration::from_secs_f64(args.net_timeout);
    let frame = Duration::from_micros(4167);
    let mut film_written = false;
    while started.elapsed() < deadline {
        let begun = Instant::now();
        let news = net.poll(&mut sim);
        if news.started {
            info!("net: the game begins");
            clock = PollClock::new(net.elapsed());
        }
        if news.host_lost && !net.ended {
            let line = networking_string(&strings, STRING_HOST_LOST, b"");
            println!("{}", String::from_utf8_lossy(&line));
            write_film(&args, &net, &mut sim)?;
            anyhow::bail!("net: the host is gone");
        }
        let clock_polls = if net.started {
            clock.polls(net.elapsed())
        } else {
            0
        };
        let polls = net.catch_up_polls(&sim).unwrap_or(clock_polls);
        let mut bodies = net.frame(&mut sim, &mut speed, polls);
        let step = bodies.step;
        while bodies.next_body(&mut SimNet(&mut sim)) {
            let dropped = dropped_players(&sim);
            let chats = chats(&sim);
            let tick = sim.current_tick;
            crate::film_report::step(&mut sim);
            net.after_body(&mut sim);
            if args.net_save_at == Some(tick) {
                write_net_save(&args, &net, &sim)?;
            }
            if let Some(at) = args.net_detach_at {
                if tick == at {
                    detach_own_units(&mut sim);
                }
                if tick == at || tick == at + 150 {
                    log_owners(&sim);
                }
            }
            for (sender, flags, text) in chats {
                let (shown, _) = chat_shown(&sim, sender, flags);
                info!(
                    "chat from player {sender} (flags {flags}) at tick {tick}: {}{}",
                    String::from_utf8_lossy(&text),
                    if shown {
                        ""
                    } else {
                        " (not shown: another team's)"
                    }
                );
            }
            for player in dropped {
                let name = names.get(player).map(|n| n.0.0.clone()).unwrap_or_default();
                let line = networking_string(&strings, STRING_PLAYER_LEFT, &name);
                info!("console: {}", String::from_utf8_lossy(&line));
            }
        }
        if sim.game_over() {
            sim.end_recording();
        }
        net.after_batch(&mut sim, step);
        if let Some(stop) = &sim.engine_stop {
            warn!("net: the sim stopped at tick {}: {stop}", sim.current_tick);
        }
        let over = sim.engine.game.game_over;
        if over && !film_written {
            film_written = true;
            write_film(&args, &net, &mut sim)?;
            net.map_over();
        }
        if net.ended || (film_written && net.end_given_up()) {
            info!(
                "net: done at tick {}: {} commands received, {} sent, {} out of sync, lead {}",
                sim.current_tick,
                net.received,
                net.sent,
                sim.out_of_sync.len(),
                net.gate.lead
            );
            net.log_orders();
            let oos = sim.out_of_sync.len();
            if net.is_host() {
                let until = Instant::now() + HOST_POSTGAME;
                while Instant::now() < until {
                    net.poll_lobby();
                    net.endpoint.wait(Duration::from_millis(10));
                }
            }
            net.leave();
            if oos > 0 {
                anyhow::bail!("net: {oos} out of sync");
            }
            return Ok(());
        }
        if let Some(rest) = frame.checked_sub(begun.elapsed()) {
            net.endpoint.wait(rest);
        }
    }
    if net.started && !film_written {
        write_film(&args, &net, &mut sim)?;
    }
    anyhow::bail!("net: --net-timeout ran out at tick {}", sim.current_tick)
}

fn detach_own_units(sim: &mut Simulation) {
    let w = &sim.engine;
    let local = w.local_player;
    let selection: Vec<usize> = (0..w.monsters.len())
        .filter(|&i| w.monsters[i].owner == local && w.monsters[i].identifier != -1)
        .collect();
    let target = match sim.detach_units(&selection) {
        mandrake_sim::net_game::DetachUnits::To(t) => Some(t),
        mandrake_sim::net_game::DetachUnits::Choose(rows) => rows.first().copied(),
        mandrake_sim::net_game::DetachUnits::Nothing => None,
    };
    info!(
        "net-detach: player {local} gives {} units at tick {} to {target:?} (teammates {:?})",
        selection.len(),
        sim.current_tick,
        sim.teammates()
    );
    if let Some(t) = target {
        sim.queue_detach(&selection, t, false);
    }
}

fn log_owners(sim: &Simulation) {
    let w = &sim.engine;
    let mut owners = std::collections::BTreeMap::<i16, usize>::new();
    for m in &w.monsters {
        if m.identifier != -1 {
            *owners.entry(m.owner).or_default() += 1;
        }
    }
    let captain = usize::try_from(w.local_player)
        .ok()
        .and_then(|p| w.players.players.get(p))
        .and_then(|p| usize::try_from(p.team).ok())
        .and_then(|t| w.players.teams.get(t))
        .map(|t| t.player);
    info!(
        "net-detach: monsters by owner at tick {}: {owners:?} (local player {}, its team's captain {captain:?})",
        sim.current_tick, w.local_player
    );
}

pub(crate) fn dropped_players(sim: &Simulation) -> Vec<usize> {
    sim.commands_by_tick
        .get(&sim.current_tick)
        .into_iter()
        .flatten()
        .filter(|c| c.command_type == mandrake_rec::CommandType::DropPlayer)
        .map(|c| usize::from(sim.command_player(c.player_index)))
        .collect()
}

pub(crate) fn chats(sim: &Simulation) -> Vec<(i16, u16, Vec<u8>)> {
    sim.commands_by_tick
        .get(&sim.current_tick)
        .into_iter()
        .flatten()
        .filter(|c| c.command_type == mandrake_rec::CommandType::Chat)
        .filter_map(|c| {
            let (flags, text) = crate::command_bar::chat_command_parts(&c.raw_payload)?;
            let player = sim.command_player(c.player_index);
            Some((i16::from(player as i8), flags, text.to_vec()))
        })
        .collect()
}

pub(crate) fn chat_shown(sim: &Simulation, sender: i16, flags: u16) -> (bool, Option<u16>) {
    let mode = sim.engine.mode;
    let local_team = sim.engine.local_team();
    let sender_team = usize::try_from(sender)
        .ok()
        .and_then(|s| sim.engine.players.players.get(s))
        .map_or(-1, |p| i16::from(p.team));

    let team = flags & 1 != 0;
    let shown = !team || mode == 0 || (mode == 1 && local_team == -1) || sender_team == local_team;
    if !shown {
        return (false, None);
    }
    let options = sim.game_setup.as_ref().map_or(0, |g| g.options);
    let sound = (mode != 0).then(|| {
        let tfl_co_op =
            mandrake_common::game_options::TFL | mandrake_common::game_options::COOPERATIVE;
        if options & tfl_co_op == tfl_co_op && mode == 1 {
            0x15
        } else {
            0x15 + u16::from(team)
        }
    });
    (true, sound)
}

fn write_net_save(args: &Args, net: &NetPlay, sim: &Simulation) -> anyhow::Result<()> {
    let Some(path) = &args.net_save else {
        return Ok(());
    };
    let Some(game) = &net.game else {
        anyhow::bail!("net: no new-game data for the save");
    };
    let (is_coop, mesh) = game_save_info(Some(game));
    let coop = net.is_host() && is_coop;
    if !save::may_save(sim, coop, mesh) {
        warn!(
            "net: no save at tick {}: only the co-op host saves (can_save_game)",
            sim.current_tick
        );
        return Ok(());
    }
    let commands = sim.recorded_commands.clone().unwrap_or_default();
    let header = mandrake_rec::saved_game::SavedGameHeader {
        snapshot_width: mandrake_rec::saved_game::SNAPSHOT_WIDTH as i16,
        snapshot_height: mandrake_rec::saved_game::SNAPSHOT_HEIGHT as i16,
        snapshot_format: mandrake_rec::saved_game::SNAPSHOT_FORMAT_32_BIT,
        observer_x: 0,
        observer_y: 0,
        observer_facing: 0,
        observer_zoom: 0x10000,
        console: vec![
            mandrake_rec::saved_game::ConsoleLine {
                kind: 0,
                text: Vec::new(),
            };
            mandrake_rec::saved_game::CONSOLE_LINES
        ],
        console_count: 0,
    };
    let data = save::saved_game_file(game, &commands, sim.current_tick, &header);
    let name = b"Network Save";
    let file = mandrake_rec::writer::save_file(
        name,
        mandrake_rec::writer::subgroup_for_name(name, &[]),
        &data,
    );
    std::fs::write(path, file)?;
    info!(
        "net: saved the game at tick {} with {} commands: {}",
        sim.current_tick,
        commands.len(),
        path.display()
    );
    Ok(())
}

fn write_film(args: &Args, net: &NetPlay, sim: &mut Simulation) -> anyhow::Result<()> {
    let commands = sim.recorded_commands.clone().unwrap_or_default();
    let Some(game) = &net.game else {
        anyhow::bail!("net: no new-game data for the film");
    };
    let data = crate::app::recording::network_film(game, &commands, sim.current_tick);
    info!(
        "net: the film ends at tick {} with {} commands",
        sim.current_tick,
        commands.len()
    );
    if let Some(mut file) = crate::app::recording::FilmFile::for_archive(&args.archive) {
        file.write(&data);
    }
    if let Some(path) = &args.net_film {
        let name = b"Last Recording";
        let file = mandrake_rec::writer::film_file(
            name,
            mandrake_rec::writer::subgroup_for_name(name, &[]),
            &data,
        );
        std::fs::write(path, file)?;
        info!("net: wrote {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_session::net::NetEvent;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_join_looks_up_its_host_on_the_connect_thread() {
        let request = JoinRequest {
            name: b"Joiner\0".to_vec(),
            password: vec![0],
            player: crate::settings::net_defaults().player_data(),
        };
        let connecting = Connecting::resolve_and_start("no port here".to_owned(), request, 0);
        assert_eq!(connecting.host_name, "no port here");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let answer = loop {
            if let Some(answer) = connecting.poll() {
                break answer;
            }
            assert!(std::time::Instant::now() < deadline, "no answer");
            std::thread::yield_now();
        };
        let error = answer.expect_err("an address with no port can't connect");
        assert!(error.to_string().contains("no port here"), "{error}");
    }

    #[test]
    fn a_team_line_has_its_own_sound() {
        let mut sim = Simulation::new(1);
        let mut w = mandrake_sim::engine_rng::EngineWorld::default();
        w.mode = 1;
        w.players.players = vec![mandrake_sim::players::Player {
            flags: 0,
            field_09: 0,
            team: 0,
            field_0b: 0,
            id: 0,
        }];
        w.local_player = 0;
        sim.set_engine_world(w);
        let setup = |options| mandrake_sim::players::GameSetup {
            options,
            team_seed: 0,
            maximum_teams: 0,
            local_player: 0,
            players: Vec::new(),
            complete: true,
        };
        sim.game_setup = Some(setup(0x4000));
        assert_eq!(chat_shown(&sim, 0, 0), (true, Some(0x15)));
        assert_eq!(chat_shown(&sim, 0, 1), (true, Some(0x16)));
        let tfl_co_op =
            mandrake_common::game_options::TFL | mandrake_common::game_options::COOPERATIVE;
        assert_eq!(tfl_co_op, 0x200080);
        sim.game_setup = Some(setup(tfl_co_op));
        assert_eq!(chat_shown(&sim, 0, 0), (true, Some(0x15)));
        assert_eq!(chat_shown(&sim, 0, 1), (true, Some(0x15)));
        sim.engine.mode = 0;
        sim.game_setup = Some(setup(0x4000));
        assert_eq!(chat_shown(&sim, 0, 1), (true, None));
    }

    #[test]
    fn a_loss_with_the_barriers_end_goes_before_it() {
        let lost = NetEvent::HostLost;
        let left = NetEvent::PlayerLeft { id: 1 };
        assert_eq!(
            hosts_loss_in_barrier(vec![left.clone(), NetEvent::AllSynced, lost.clone()]),
            [left.clone(), lost.clone(), NetEvent::AllSynced]
        );
        let kept = vec![lost.clone(), NetEvent::AllSynced];
        assert_eq!(hosts_loss_in_barrier(kept.clone()), kept);
        assert_eq!(
            hosts_loss_in_barrier(vec![left.clone(), lost.clone()]),
            [left, lost]
        );
    }

    #[test]
    fn a_networking_string_takes_the_name() {
        let strings = vec![b"zero".to_vec(), b"\xd2%s\xd3 has left the game.".to_vec()];
        assert_eq!(
            networking_string(&strings, 1, b"Host"),
            b"\xd2Host\xd3 has left the game."
        );
        assert_eq!(networking_string(&strings, 0, b"Host"), b"zero");
        assert!(networking_string(&strings, STRING_HOST_LOST, b"").is_empty());
    }

    #[test]
    fn game_save_info_handles_short_new_game_data() {
        let short = NewGameData(vec![0; 4]);
        assert_eq!(game_save_info(Some(&short)), (false, [0; 4]));
        assert_eq!(game_save_info(None), (false, [0; 4]));
        let mut full = vec![0u8; 16];
        full[7] = 0x80;
        full[0x0c..0x10].copy_from_slice(b"le3e");
        assert_eq!(game_save_info(Some(&NewGameData(full))), (true, *b"le3e"));
    }
}
