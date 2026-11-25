use std::path::Path;
use std::time::Duration;

use mandrake_net::endpoint::Event;
use mandrake_net::game_description::GameDescription;
use mandrake_net::game_kind::GameKind;
use mandrake_rec::tfl::TflNewGame;
use mandrake_rec::{CommandPacket, TickCommands};
use mandrake_session::net_game::{NetLoop, NetNews, NetSim, PollClock};
use mandrake_tfl_sim::net_game::{Chat, GateStep, NetGame};
use mandrake_tfl_sim::{Stop, TflSim};
use tracing::{info, warn};

use crate::args::Args;
use crate::clock::Instant;
use crate::net_game::Lobbied;
use crate::tfl_live::film;

const FILM_NAME: &[u8] = crate::tfl_live::LAST_RECORDING_NAME;
const HOST_POSTGAME: Duration = Duration::from_secs(2);
pub(crate) const CHAT_SOUND: i16 = 0x15;

pub(crate) const TEAM_CHAT_SOUND: i16 = 0x116;

pub(crate) const fn chat_sound(team_only: bool) -> i16 {
    if team_only {
        TEAM_CHAT_SOUND
    } else {
        CHAT_SOUND
    }
}

struct Incoming<'a> {
    net: &'a mut NetGame,
    tick: u32,
}

impl NetSim for Incoming<'_> {
    fn current_tick(&self) -> u32 {
        self.tick
    }
    fn net_horizon(&self) -> Option<u32> {
        Some(self.net.horizon)
    }
    fn queue_incoming_command(&mut self, packet: CommandPacket) {
        self.net.queue_incoming_command(packet);
    }
    fn take_outgoing_commands(&mut self) -> Vec<CommandPacket> {
        self.net.take_outgoing_commands()
    }
    fn update_head(&mut self, _filling: bool) {}
    fn net_game_speed(&self) -> Option<i16> {
        None
    }
    fn ticks_stopped(&self) -> bool {
        false
    }
    fn net_body_may_run(&mut self) -> bool {
        true
    }
}

pub(crate) struct TflNetPlay {
    core: NetLoop,
    pub(crate) net: NetGame,
    pub(crate) new_game: TflNewGame,
    description: GameDescription,
    pub(crate) local_id: u8,
    dropped: Vec<i16>,
    forward: bool,
    session_events: Vec<mandrake_session::net::NetEvent>,
    survivors: Vec<mandrake_tfl_sim::MonsterInitializer>,
    veterans: Vec<u8>,
}

impl std::ops::Deref for TflNetPlay {
    type Target = NetLoop;
    fn deref(&self) -> &NetLoop {
        &self.core
    }
}

impl TflNetPlay {
    pub(crate) fn new(lobbied: Lobbied, sync_counter: i32) -> anyhow::Result<Self> {
        let Lobbied {
            endpoint, clock, ..
        } = lobbied;
        let core = NetLoop::new(endpoint, Box::new(move || clock.elapsed()), "net");
        Self::from_loop(core, sync_counter, Vec::new())
    }

    pub(crate) fn from_loop(
        core: NetLoop,
        sync_counter: i32,
        survivors: Vec<mandrake_tfl_sim::MonsterInitializer>,
    ) -> anyhow::Result<Self> {
        Self::try_from_loop(core, sync_counter, survivors).map_err(|(_, err)| err)
    }

    pub(crate) fn try_from_loop(
        mut core: NetLoop,
        sync_counter: i32,
        survivors: Vec<mandrake_tfl_sim::MonsterInitializer>,
    ) -> Result<Self, (Box<NetLoop>, anyhow::Error)> {
        let parsed = {
            let j = core.endpoint.joiner();
            let local_id = j.local_id().unwrap_or(0);
            match (j.description().cloned(), j.tfl_new_game()) {
                (None, _) => Err(anyhow::anyhow!("net: no game description at the start")),
                (Some(description), Some(Ok(new_game))) => Ok((local_id, description, new_game)),
                (Some(_), other) => Err(anyhow::anyhow!(
                    "net: no TFL new-game data at the start: {other:?}"
                )),
            }
        };
        let (local_id, description, new_game) = match parsed {
            Ok(v) => v,
            Err(err) => return Err((Box::new(core), err)),
        };
        core.begin_map();
        let veterans = mandrake_tfl_sim::statistics::encode_monster_initializers(&survivors);
        Ok(Self {
            core,
            net: NetGame::new(i16::from(local_id), sync_counter),
            new_game,
            description,
            local_id,
            dropped: Vec::new(),
            forward: false,
            session_events: Vec::new(),
            survivors,
            veterans,
        })
    }

    pub(crate) fn forward_to_session(&mut self) {
        self.forward = true;
    }

    pub(crate) fn take_session_events(&mut self) -> Vec<mandrake_session::net::NetEvent> {
        std::mem::take(&mut self.session_events)
    }

    pub(crate) fn core_mut(&mut self) -> &mut NetLoop {
        &mut self.core
    }

    fn note(&mut self, events: &[Event]) {
        log_events(events);
        if self.forward {
            self.session_events.extend(
                events
                    .iter()
                    .filter_map(mandrake_session::net::NetEvent::from_endpoint),
            );
        }
    }

    pub(crate) fn set_up(&mut self, game_dir: &Path) -> Result<TflSim, Stop> {
        let sim = TflSim::network_game(game_dir, &self.film(0))?;
        self.set_up_done(&sim);
        Ok(sim)
    }

    pub(crate) fn set_up_done(&mut self, sim: &TflSim) {
        if self.new_game.parameters.options & mandrake_common::game_options::CARRY_VETERANS == 0 {
            return;
        }
        let taken = sim.monster_initializers().iter().filter(|v| v.used).count();
        if taken > 0 {
            info!("net: {taken} veterans of the last game taken");
        }
    }

    pub(crate) fn survivors(&self) -> &[mandrake_tfl_sim::MonsterInitializer] {
        &self.survivors
    }

    pub(crate) fn keep_survivors(&mut self, sim: &TflSim) {
        match sim.map_end_initializers() {
            Ok(v) => {
                info!("net: {} survivors kept for the next game", v.len());
                self.survivors = v;
            }
            Err(stop) => {
                warn!("net: the survivors weren't kept: {stop}");
                self.survivors.clear();
            }
        }
    }

    pub(crate) fn film(&self, end_tick: u32) -> Vec<u8> {
        let ran: Vec<CommandPacket> = self
            .net
            .recorded
            .iter()
            .filter(|c| c.tick < end_tick)
            .cloned()
            .collect();
        mandrake_rec::tfl::tfl_film(
            &self.new_game,
            mandrake_net::tfl::film_checksums(&self.description),
            FILM_NAME,
            crate::tfl_live::LAST_RECORDING_ID,
            end_tick,
            &self.veterans,
            &TickCommands::group(&ran),
        )
    }

    fn poll_into(core: &mut NetLoop, net: &mut NetGame, tick: u32) -> (NetNews, Vec<Event>) {
        let mut sim = Incoming { net, tick };
        let (news, events) = core.poll_events(Some(&mut sim), |_| {});
        if let Some(host) = core.endpoint.as_host_mut() {
            for e in host.take_events() {
                mandrake_session::net_game::log_host_event("net: host: ", &e, Some(tick));
            }
        }
        core.end_poll(Some(&mut sim));
        core.send_outgoing(&mut sim, Vec::new(), |_| {});
        (news, events)
    }

    pub(crate) fn poll(&mut self, tick: u32) -> NetNews {
        let (news, events) = Self::poll_into(&mut self.core, &mut self.net, tick);
        self.note(&events);
        news
    }

    pub(crate) fn frame(
        &mut self,
        sim: &mut TflSim,
        polls: u32,
    ) -> Result<(NetNews, Option<GateStep>), Stop> {
        let tick = sim.tick;
        let mut seen = None;
        let core = &mut self.core;
        let step = if polls > 0 {
            self.net
                .frame(sim, polls.min(i16::MAX as u32) as i16, false, |n| {
                    seen = Some(Self::poll_into(core, n, tick));
                })?
        } else {
            None
        };
        let (news, events) = match seen {
            Some(s) => s,
            None => Self::poll_into(&mut self.core, &mut self.net, tick),
        };
        self.note(&events);
        Ok((news, step))
    }

    pub(crate) fn map_loaded(&mut self) {
        self.core.map_loaded();
    }

    pub(crate) fn map_over(&mut self) {
        self.core.map_over();
    }

    pub(crate) fn take_dropped(&mut self, sim: &TflSim) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for p in sim.dropped_players() {
            let Some(player) = self.new_game.players.get(p) else {
                continue;
            };
            if self.dropped.contains(&player.net_id) {
                continue;
            }
            self.dropped.push(player.net_id);
            out.push((p, player.name.clone()));
        }
        out
    }

    pub(crate) fn next_map(&mut self) -> anyhow::Result<()> {
        let mut game = match self.core.endpoint.joiner().tfl_new_game() {
            Some(Ok(g)) => g,
            other => anyhow::bail!("net: no TFL new-game data for the next map: {other:?}"),
        };
        trim_dropped(&mut game, &self.dropped);
        self.veterans = mandrake_tfl_sim::statistics::encode_monster_initializers(&self.survivors);
        self.new_game = game;
        self.dropped.clear();
        self.core.begin_map();
        self.net = NetGame::new(i16::from(self.local_id), self.net.sync_counter());
        Ok(())
    }

    pub(crate) fn leave(self) {
        self.core.endpoint.leave();
    }
}

#[cfg(test)]
fn by_tick(commands: &[CommandPacket]) -> Vec<TickCommands> {
    TickCommands::group(commands)
}

fn log_events(events: &[Event]) {
    for e in events {
        match e {
            Event::Commands { .. } | Event::AllSynced { .. } | Event::HostLost => {}
            Event::PlayerLeft { id } => info!("net: player {id} left"),
            Event::Chat { from, chat } => info!(
                "net: chat from {from}: {}",
                String::from_utf8_lossy(&chat.text)
            ),
            other => tracing::debug!("net: {other:?}"),
        }
    }
}

pub(crate) fn trim_dropped(game: &mut TflNewGame, dropped_net_ids: &[i16]) {
    let local = usize::try_from(game.local_player)
        .ok()
        .and_then(|i| game.players.get(i))
        .map(|p| p.net_id);
    game.players
        .retain(|p| !dropped_net_ids.contains(&p.net_id));
    for (slot, p) in game.players.iter_mut().enumerate() {
        p.slot = slot;
    }
    game.local_player = local
        .and_then(|id| game.players.iter().position(|p| p.net_id == id))
        .map_or(-1, |i| i as i16);
}

pub(crate) fn chat_shown(sim: &TflSim, chat: &Chat) -> bool {
    let local = sim.local_team().ok().flatten();
    if !chat.team_only {
        return true;
    }
    let Some(local) = local else {
        return true;
    };
    sim.statistics_teams()
        .ok()
        .and_then(|t| {
            usize::try_from(chat.player)
                .ok()
                .and_then(|p| t.player_teams.get(p).copied())
        })
        .is_some_and(|team| team == local)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sketch {
    pub(crate) flags: u16,
    pub(crate) points: Vec<u16>,
}

#[derive(Debug, Default)]
pub(crate) struct InterfaceNews {
    pub(crate) chats: Vec<Chat>,
    pub(crate) sketches: Vec<Sketch>,
}

pub(crate) fn interface_news(sim: &mut TflSim) -> InterfaceNews {
    use mandrake_tfl_sim::net_commands::InterfaceCommand as C;
    let mut out = InterfaceNews::default();
    for c in sim.take_interface_commands() {
        match c {
            C::Chat(chat) if chat_shown(sim, &chat) => out.chats.push(chat),
            C::Sketch {
                player,
                flags,
                points,
            } if sketch_shown(sim, player) => out.sketches.push(Sketch {
                flags,
                points: points.into_iter().map(|w| w as u16).collect(),
            }),
            C::Chat(_) | C::Sketch { .. } | C::OwnerChanged { .. } => {}
            C::SaveLoaded { player, header, .. } => tracing::debug!(
                "TFL: player {player}'s presets ({}) at tick {}: not kept (a film's units can't be \
                 selected)",
                header[1],
                sim.tick
            ),
        }
    }
    out
}

pub(crate) fn sketch_shown(sim: &TflSim, player: i8) -> bool {
    if i16::from(player) == sim.local_player() {
        return false;
    }
    let Some(local) = sim.local_team().ok().flatten() else {
        return true;
    };
    sim.statistics_teams()
        .ok()
        .and_then(|t| {
            usize::try_from(player)
                .ok()
                .and_then(|p| t.player_teams.get(p).copied())
        })
        .is_some_and(|team| team == local)
}

pub(crate) const SKETCH_SOUND: i16 = 0x16;

pub(crate) fn game_sound(tick: u32, site: u32, id: i16) -> mandrake_tfl_sim::SoundEvent {
    mandrake_tfl_sim::SoundEvent {
        tick,
        site,
        kind: mandrake_tfl_sim::SoundKind::GameSound(id),
        sound: None,
        object: None,
        world: None,
        velocity: [0; 3],
        volume: 0x100,
    }
}

pub(crate) const CHAT_SOUND_SITE: u32 = mandrake_tfl_sim::draw_site::CHAT_SOUND;
pub(crate) const SKETCH_SOUND_SITE: u32 = mandrake_tfl_sim::draw_site::SKETCH_SOUND;

pub(crate) fn scripted_orders(sim: &TflSim) -> Option<(u8, Vec<u8>)> {
    let local = sim.local_player();
    let team = sim.local_team().ok().flatten()?;
    let live = |m: &mandrake_tfl_sim::monsters::TflMonster| {
        m.identifier != 0 && m.flags & 0x4000_0000 == 0
    };
    let ours: Vec<&mandrake_tfl_sim::monsters::TflMonster> = sim
        .monsters
        .iter()
        .filter(|m| live(m) && i16::from(m.player) == local && m.flags & 0x1000_0000 == 0)
        .take(30)
        .collect();
    if ours.is_empty() {
        return None;
    }
    let at = |m: &mandrake_tfl_sim::monsters::TflMonster| {
        sim.objects
            .get(m.object)
            .map_or((0i64, 0i64), |o| (i64::from(o.x), i64::from(o.y)))
    };
    let n = ours.len() as i64;
    let (sx, sy) = ours
        .iter()
        .map(|m| at(m))
        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    let centre = (sx / n, sy / n);
    let mut enemies: Vec<(i64, i16)> = sim
        .monsters
        .iter()
        .filter(|m| live(m) && m.player >= 0 && i16::from(m.team as i8) != team)
        .map(|m| {
            let (x, y) = at(m);
            ((x - centre.0).pow(2) + (y - centre.1).pow(2), m.identifier)
        })
        .collect();
    if enemies.is_empty() {
        return None;
    }
    enemies.sort();
    let targets: Vec<i16> = enemies.iter().take(30).map(|e| e.1).collect();
    let attackers: Vec<i16> = ours.iter().map(|m| m.identifier).collect();
    Some((
        film::TARGET_COMMAND,
        film::target_payload(0, &attackers, &targets),
    ))
}

fn write_film(args: &Args, play: &TflNetPlay, tick: u32) -> anyhow::Result<()> {
    let data = play.film(tick);
    info!(
        "net: the film ends at tick {tick} with {} commands recorded, local player {}",
        play.net.recorded.len(),
        play.new_game.local_player
    );
    if let Some(path) = &args.net_film {
        std::fs::write(path, &data)?;
        info!("net: wrote {}", path.display());
    }
    Ok(())
}

pub(crate) fn run(args: &Args, game_dir: &Path) -> anyhow::Result<()> {
    if !args.headless {
        refuse_myth2_aids(args)?;
        let started = Instant::now();
        let play = lobby(args)?;
        let net = TflNetWindow::new(args, play, started);
        return crate::tfl_window::run_tfl_net_window(args, game_dir, net);
    }
    let played = run_peer(args, game_dir)?;
    println!(
        "TFL network game: {} at tick {}: {} SYNCs checked, {} out of sync, dropped {:?}, a film \
         of {} bytes",
        if played.left { "left" } else { "ended" },
        played.tick,
        played.syncs_checked,
        played.out_of_sync,
        played.dropped,
        played.film.len()
    );
    anyhow::ensure!(played.stop.is_none(), "net: the TFL sim stopped");
    anyhow::ensure!(
        played.out_of_sync == 0,
        "net: {} out of sync",
        played.out_of_sync
    );
    Ok(())
}

fn refuse_myth2_aids(args: &Args) -> anyhow::Result<()> {
    for (given, what) in [
        (args.net_orders.is_some(), "--net-orders"),
        (args.net_load_save.is_some(), "--net-load-save"),
        (args.net_save_at.is_some(), "--net-save-at"),
        (args.net_detach_at.is_some(), "--net-detach-at"),
    ] {
        anyhow::ensure!(
            !given,
            "{what} is Myth II's: a TFL network game has no such aid"
        );
    }
    Ok(())
}

pub(crate) fn lobby(args: &Args) -> anyhow::Result<TflNetPlay> {
    let lobbied = if args.net_host.is_some() && args.net_description.is_some() {
        let d = crate::net_game::host_game_description(args)?;
        crate::net_game::host_lobby(args, d, |_| Ok(()))?
    } else if args.net_host.is_some() {
        let mut d = mandrake_session::net::tfl_host_description(
            &mandrake_session::Text(
                args.net_name
                    .clone()
                    .unwrap_or_else(|| "Network Game".into())
                    .into_bytes(),
            ),
            &mandrake_session::Text(args.net_password.clone().unwrap_or_default().into_bytes()),
            film::CHECKSUMS,
        );
        crate::net_game::host_overrides(args, &mut d)?;
        crate::net_game::host_lobby(args, d, |_| Ok(()))?
    } else {
        crate::net_game::join_lobby(args, GameKind::Tfl)?
    };
    let play = TflNetPlay::new(lobbied, 0)?;
    info!(
        "net: the TFL game starts: net id {}, local player {} of {}, seed {:#x}, map {}",
        play.local_id,
        play.new_game.local_player,
        play.new_game.players.len(),
        play.new_game.parameters.seed,
        play.new_game.parameters.mesh
    );
    Ok(play)
}

pub(crate) fn give(net: &mut NetGame, command: &CommandPacket) -> bool {
    let ty = command.command_type.to_u8();
    if !crate::tfl_live::SIM_COMMANDS.contains(&ty) {
        tracing::error!("net: command type {ty} isn't TFL's: not sent");
        return false;
    }
    net.queue_outgoing_command(ty, &command.raw_payload);
    true
}

#[derive(Debug, Default)]
pub(crate) struct WindowFrame {
    pub(crate) stepped: u32,
    pub(crate) chats: Vec<(Chat, bool)>,
    pub(crate) dropped: Vec<(usize, String)>,
    pub(crate) host_lost: bool,
}

pub(crate) struct TflNetWindow {
    pub(crate) play: TflNetPlay,
    pub(crate) names: Vec<String>,
    film_path: Option<std::path::PathBuf>,
    film_written: bool,
    loaded: bool,
    stopped: Option<String>,
    script_every: Option<u32>,
    next_order: Option<u32>,
    leave_after: Option<Instant>,
    started: Instant,
    tick: u32,
    pub(crate) host_gone: bool,
    pub(crate) reported: bool,
    pub(crate) strings: Vec<Vec<u8>>,
    trade: Option<mandrake_session::screen::UnitTrading>,
    trading: (bool, bool),
}

pub(crate) const STRING_PLAYER_LEFT: usize = crate::net_game::STRING_PLAYER_LEFT;

impl TflNetWindow {
    pub(crate) fn new(args: &Args, play: TflNetPlay, started: Instant) -> Self {
        let names = play
            .new_game
            .players
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let film_path = args
            .net_film
            .clone()
            .or_else(|| crate::tfl_live::our_films_dir().map(|d| d.join("LAST RECORDING")));
        let script_every = args.net_script_every.filter(|&n| n > 0);
        Self {
            play,
            names,
            film_path,
            film_written: false,
            loaded: false,
            stopped: None,
            script_every,
            next_order: script_every,
            leave_after: None,
            started,
            tick: 0,
            host_gone: false,
            reported: false,
            strings: Vec::new(),
            trade: None,
            trading: (false, false),
        }
    }

    pub(crate) fn trading_events(
        &mut self,
        replay: Option<&crate::tfl_game::TflReplay>,
        may_open: bool,
    ) -> Vec<mandrake_session::net::NetEvent> {
        use mandrake_session::net::NetEvent;
        let mut out = Vec::new();
        let Some(sim) = replay.and_then(crate::tfl_game::TflReplay::sim) else {
            return out;
        };
        let (offered, over) = self.trading;
        let planning = sim.game_state().0 == 0;
        if !offered
            && planning
            && may_open
            && let Some(trade) = self.trade.take()
        {
            info!(
                "net: untr opens at tick {}: {:?}",
                sim.tick,
                trade
                    .rows
                    .iter()
                    .map(|r| (
                        r.name.to_plain_utf8(),
                        r.placed,
                        r.minimum,
                        r.maximum,
                        r.cost
                    ))
                    .collect::<Vec<_>>()
            );
            out.push(NetEvent::UnitTrading(Box::new(trade)));
            self.trading.0 = true;
        }
        if self.trading.0 && !over && !planning {
            info!("net: untr: the planning time ended at tick {}", sim.tick);
            out.push(NetEvent::TradingOver);
            self.trading.1 = true;
        }
        out
    }

    pub(crate) fn trade_units(&mut self, counts: &[i16]) {
        info!("net: untr: the trade {counts:?} at tick {}", self.tick);
        self.play.net.queue_unit_adjustment(counts);
    }

    pub(crate) fn left_line(&self, name: &str) -> Vec<u8> {
        crate::net_game::networking_string(
            &self.strings,
            STRING_PLAYER_LEFT,
            &mandrake_common::mac_roman::encode(name),
        )
    }

    pub(crate) fn load(
        &mut self,
        game_dir: &Path,
    ) -> anyhow::Result<(crate::tfl_map::TflMap, Option<crate::tfl_game::TflReplay>)> {
        let mesh = self.play.new_game.parameters.mesh;
        let mut map = crate::tfl_map::load_tfl_map(game_dir, None, Some(mesh.as_str().trim_end()))?;
        let inmn = mandrake_common::FourCC(*b"inmn");
        self.strings = mandrake_session::tfl::tfl_tag(&map.archive, b"stli", inmn)
            .and_then(|(v, d)| mandrake_tag::tfl::menus::TflStringList::from_tag(inmn, v, &d).ok())
            .map(|l| l.strings)
            .unwrap_or_default();
        let bytes = self.play.film(0);
        map.film = mandrake_rec::tfl::TflRecording::from_bytes(&bytes)
            .ok()
            .map(|r| crate::tfl_map::with_game_colours(r.header));
        map.film_bytes = Some(bytes);
        let replay = crate::tfl_game::TflReplay::new_as(game_dir, &map, NETWORK_GAME_MODE);
        self.trading = (false, false);
        let mut replay = replay;
        self.trade = replay
            .as_mut()
            .and_then(crate::tfl_game::TflReplay::sim_mut)
            .and_then(|sim| trade_offer(sim, &map.archive));
        if let Some(sim) = replay.as_ref().and_then(crate::tfl_game::TflReplay::sim) {
            self.play.set_up_done(sim);
            info!(
                "net: the players: {:?}, local player {}, local team {:?}, leads {:?}; the map is \
                 loaded in {} ms",
                self.names,
                sim.local_player(),
                sim.local_team(),
                sim.local_player_leads(),
                self.started.elapsed().as_millis()
            );
        }
        Ok((map, replay))
    }

    pub(crate) fn tick(&self) -> u32 {
        self.tick
    }

    pub(crate) fn stop(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    pub(crate) fn takes_polls(&self) -> bool {
        self.play.started && !self.film_written && self.stopped.is_none()
    }

    pub(crate) fn frame(
        &mut self,
        replay: &mut Option<crate::tfl_game::TflReplay>,
        polls: u32,
    ) -> WindowFrame {
        let mut out = WindowFrame::default();
        let Some(r) = replay.as_mut().filter(|r| r.sim().is_some()) else {
            return out;
        };
        if self.leave_after.is_some() && !self.play.is_host() {
            return out;
        }
        if !self.loaded {
            self.loaded = true;
            self.play.map_loaded();
            info!("net: the map is up: the pre-game barrier answered");
        }
        let polls = if self.takes_polls() { polls } else { 0 };
        let before = r.tick();
        let play = &mut self.play;
        let news = match r.step_with(|sim| play.frame(sim, polls)) {
            Some((news, _)) => news,
            None => {
                let stop = r
                    .stop()
                    .map_or_else(|| "stopped".into(), ToString::to_string);
                if self.stopped.is_none() {
                    warn!("net: the TFL sim stopped at tick {}: {stop}", r.tick());
                }
                self.stopped = Some(stop);
                self.play.poll(r.tick())
            }
        };
        out.stepped = r.tick().saturating_sub(before);
        if news.started {
            info!("net: the game begins");
        }
        let Some(sim) = r.sim() else {
            return out;
        };
        let tick = sim.tick;
        self.tick = tick;
        if news.host_lost && !self.play.ended {
            println!("The host is gone.");
            out.host_lost = true;
            self.host_gone = true;
            self.write_film(tick);
            return out;
        }
        if out.stepped > 0 {
            for chat in self.play.net.take_chats() {
                let shown = chat_shown(sim, &chat);
                out.chats.push((chat, shown));
            }
            out.dropped = self.play.take_dropped(sim);
            if let Some(n) = self.next_order.filter(|&n| tick >= n) {
                if let Some((kind, payload)) = scripted_orders(sim) {
                    self.play.net.queue_outgoing_command(kind, &payload);
                }
                self.next_order = Some(n + self.script_every.unwrap_or(1));
            }
        }
        if (sim.game_over() || self.stopped.is_some()) && !self.film_written {
            if sim.game_over() {
                self.play.keep_survivors(sim);
            }
            self.write_film(tick);
            self.play.map_over();
            info!(
                "net: the game is over at tick {tick}: {} SYNCs checked, {} out of sync; the end \
                 barrier answered",
                self.play.net.syncs_checked,
                self.play.net.out_of_sync.len()
            );
        }
        if self.leave_after.is_none()
            && (self.play.ended || (self.film_written && self.play.end_given_up()))
        {
            let oos = self.play.net.out_of_sync.len();
            info!(
                "net: done at tick {tick}: {} commands received, {} sent, {} SYNCs checked, {oos} \
                 out of sync, lead {}",
                self.play.received,
                self.play.sent,
                self.play.net.syncs_checked,
                self.play.net.gate.lead
            );
            println!(
                "TFL network game: ended at tick {tick}: {} SYNCs checked, {oos} out of sync",
                self.play.net.syncs_checked
            );
            let wait = if self.play.is_host() {
                HOST_POSTGAME
            } else {
                Duration::ZERO
            };
            self.leave_after = Some(Instant::now() + wait);
        }
        out
    }

    fn write_film(&mut self, tick: u32) {
        if self.film_written {
            return;
        }
        self.film_written = true;
        let data = self.play.film(tick);
        info!(
            "net: the film ends at tick {tick} with {} commands recorded, local player {}",
            self.play.net.recorded.len(),
            self.play.new_game.local_player
        );
        let Some(path) = &self.film_path else {
            return;
        };
        let written = crate::config_store::write_bytes(path, &data);
        match written {
            Ok(()) => println!("TFL network game: the film is {}", path.display()),
            Err(e) => tracing::error!("net: the film wasn't written to {}: {e}", path.display()),
        }
    }

    pub(crate) fn may_leave(&self) -> bool {
        self.leave_after.is_some_and(|t| Instant::now() >= t)
    }

    pub(crate) fn leave(mut self) {
        if !self.film_written {
            info!("net: leaving the game at tick {}", self.tick);
            self.write_film(self.tick);
        }
        self.play.leave();
    }

    pub(crate) fn next_map(
        &mut self,
        game_dir: &Path,
    ) -> anyhow::Result<(crate::tfl_map::TflMap, Option<crate::tfl_game::TflReplay>)> {
        self.play.next_map()?;
        self.names = self
            .play
            .new_game
            .players
            .iter()
            .map(|p| p.name.clone())
            .collect();
        self.film_written = false;
        self.loaded = false;
        self.stopped = None;
        self.leave_after = None;
        self.reported = false;
        self.tick = 0;
        self.next_order = self.script_every;
        self.load(game_dir)
    }
}

fn trade_offer(
    sim: &mut TflSim,
    archive: &mandrake_tag::TagArchive,
) -> Option<mandrake_session::screen::UnitTrading> {
    use mandrake_session::screen::{TradeOffer, UnitTrading};
    if sim.game_options & mandrake_common::game_options::UNIT_TRADING == 0
        || sim.game_state().0 != 0
    {
        return None;
    }
    let team = sim.local_team().ok().flatten()?;
    if !sim.local_player_leads().unwrap_or(false) {
        return None;
    }
    let rows = sim.monster_placement_data(team).ok().flatten()?;
    let rows: Vec<_> = rows
        .into_iter()
        .take(mandrake_dialogs::screens::unit_trading::ROWS)
        .collect();
    if rows.is_empty() {
        return None;
    }
    Some(UnitTrading {
        rows: rows
            .iter()
            .map(|r| TradeOffer {
                name: mandrake_session::Text(crate::tfl_menu::tfl_unit_plural(archive, r.entry.0)),
                adjustable: true,
                minimum: r.minimum,
                maximum: r.maximum,
                placed: r.placed,
                cost: r.cost,
            })
            .collect(),
        counts: rows.iter().map(|r| r.placed).collect(),
        top: 0,
    })
}

const NETWORK_GAME_MODE: i16 = 1;

#[derive(Debug)]
pub(crate) struct Played {
    pub(crate) film: Vec<u8>,
    pub(crate) tick: u32,
    pub(crate) out_of_sync: usize,
    pub(crate) syncs_checked: u32,
    pub(crate) dropped: Vec<usize>,
    pub(crate) left: bool,
    pub(crate) stop: Option<String>,
}

pub(crate) fn run_peer(args: &Args, game_dir: &Path) -> anyhow::Result<Played> {
    refuse_myth2_aids(args)?;
    let started = Instant::now();
    let play = lobby(args)?;
    play_headless(args, game_dir, play, started)
}

fn play_headless(
    args: &Args,
    game_dir: &Path,
    mut play: TflNetPlay,
    started: Instant,
) -> anyhow::Result<Played> {
    let mut sim = play
        .set_up(game_dir)
        .map_err(|s| anyhow::anyhow!("net: the TFL game's set-up stopped: {s}"))?;
    let names: Vec<String> = play
        .new_game
        .players
        .iter()
        .map(|p| p.name.clone())
        .collect();
    info!(
        "net: the players: {names:?}, local player {}, local team {:?}, leads {:?}; the map is \
         loaded in {} ms",
        sim.local_player(),
        sim.local_team(),
        sim.local_player_leads(),
        started.elapsed().as_millis()
    );
    play.map_loaded();
    let mut clock = PollClock::new(play.elapsed());
    let deadline = Duration::from_secs_f64(args.net_timeout);
    let frame = Duration::from_micros(4167);
    let mut film_written = false;
    let mut stopped: Option<String> = None;
    let mut dropped = Vec::new();
    let mut next_order = args.net_script_every.filter(|&n| n > 0);
    while started.elapsed() < deadline {
        let begun = Instant::now();
        let polls = if play.started && !film_written && stopped.is_none() {
            clock.polls(play.elapsed())
        } else {
            0
        };
        let before = sim.tick;
        let news = match play.frame(&mut sim, polls) {
            Ok((news, _)) => news,
            Err(stop) => {
                warn!("net: the TFL sim stopped at tick {}: {stop}", sim.tick);
                stopped = Some(stop.to_string());
                NetNews::default()
            }
        };
        if news.started {
            info!("net: the game begins");
            clock = PollClock::new(play.elapsed());
        }
        if news.host_lost && !play.ended {
            println!("The host is gone.");
            write_film(args, &play, sim.tick)?;
            anyhow::bail!("net: the host is gone at tick {}", sim.tick);
        }
        if sim.tick != before {
            for chat in play.net.take_chats() {
                let shown = chat_shown(&sim, &chat);
                let name = usize::try_from(chat.player)
                    .ok()
                    .and_then(|p| names.get(p))
                    .cloned()
                    .unwrap_or_default();
                info!(
                    "console: chat from player {} ({name}){} at tick {}: {}{}",
                    chat.player,
                    if chat.team_only { " to its team" } else { "" },
                    sim.tick,
                    String::from_utf8_lossy(&chat.text),
                    if shown {
                        format!(", sound {:#x}", chat_sound(chat.team_only))
                    } else {
                        " (not shown: another team's)".into()
                    }
                );
            }
            for (player, name) in play.take_dropped(&sim) {
                dropped.push(player);
                info!(
                    "console: player {player} ({name}) left the game: dropped at tick {}",
                    sim.tick
                );
            }
            if let Some(n) = next_order.filter(|&n| sim.tick >= n) {
                if let Some((kind, payload)) = scripted_orders(&sim) {
                    play.net.queue_outgoing_command(kind, &payload);
                }
                next_order = Some(n + args.net_script_every.unwrap_or(1));
            }
            if args.net_leave_at.is_some_and(|at| sim.tick >= at) && !play.is_host() {
                info!(
                    "net: leaving the game at tick {} (--net-leave-at)",
                    sim.tick
                );
                write_film(args, &play, sim.tick)?;
                let played = Played {
                    film: play.film(sim.tick),
                    tick: sim.tick,
                    out_of_sync: play.net.out_of_sync.len(),
                    syncs_checked: play.net.syncs_checked,
                    dropped,
                    left: true,
                    stop: stopped,
                };
                play.leave();
                return Ok(played);
            }
        }
        let over = sim.game_over() || stopped.is_some();
        if over && !film_written {
            film_written = true;
            write_film(args, &play, sim.tick)?;
            play.map_over();
        }
        if play.ended || (film_written && play.end_given_up()) {
            let oos = play.net.out_of_sync.len();
            info!(
                "net: done at tick {}: {} commands received, {} sent, {} SYNCs checked, {oos} out \
                 of sync, lead {}",
                sim.tick, play.received, play.sent, play.net.syncs_checked, play.net.gate.lead
            );
            if play.is_host() {
                let until = Instant::now() + HOST_POSTGAME;
                while Instant::now() < until {
                    play.poll(sim.tick);
                    play.endpoint.wait(Duration::from_millis(10));
                }
            }
            let played = Played {
                film: play.film(sim.tick),
                tick: sim.tick,
                out_of_sync: oos,
                syncs_checked: play.net.syncs_checked,
                dropped,
                left: false,
                stop: stopped,
            };
            play.leave();
            return Ok(played);
        }
        if let Some(rest) = frame.checked_sub(begun.elapsed()) {
            play.endpoint.wait(rest);
        }
    }
    if play.started && !film_written {
        write_film(args, &play, sim.tick)?;
    }
    anyhow::bail!("net: --net-timeout ran out at tick {}", sim.tick)
}

impl TflNetPlay {
    pub(crate) fn is_host(&self) -> bool {
        self.core.endpoint.as_host().is_some()
    }
}

#[cfg(test)]
#[path = "tfl_net_tests.rs"]
mod tests;

#[path = "tfl_net_menu.rs"]
pub(crate) mod menu;
