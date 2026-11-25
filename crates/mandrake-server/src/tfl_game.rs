use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use mandrake_common::FourCC;
use mandrake_net::endpoint::{Endpoint, Event, HostEvent};
use mandrake_net::game_description::GameDescription;
use mandrake_net::game_kind::GameKind;
use mandrake_net::ours::Ours;
use mandrake_net::tfl::DataChecksums;
use mandrake_rec::tfl::TflNewGame;
use mandrake_rec::{CommandPacket, TickCommands};
use mandrake_session::net_game::{NetLoop, NetNews, NetSim, PollClock, log_host_event};
use mandrake_tag::TagArchive;
use mandrake_tfl_sim::TflSim;
use mandrake_tfl_sim::net_game::NetGame;
use tracing::{info, warn};

use crate::game::{FRAME, GameResult, Next, Postgame, Server, alone, log_event, run_postgame};
use crate::owner::Owner;

const FILM_ID: FourCC = FourCC(*b"lare");

fn load_level(tags: &TagArchive, mesh: FourCC) -> Option<mandrake_session::tfl::TflLevelHeader> {
    let e = tags.find_tag_by_id(FourCC::new(*b"mesh"), mesh)?;
    let data = tags.get_tag_data(e).ok()?;
    mandrake_session::tfl::TflLevelHeader::from_mesh(&data)
}

pub(crate) fn check_description_with_name(
    game_dir: &Path,
    words: DataChecksums,
    d: &GameDescription,
) -> Result<Vec<u8>, String> {
    mandrake_net::lobby::check_as(GameKind::Tfl, d).map_err(|e| format!("{e:?}"))?;
    mandrake_net::tfl::check_data(d, words).map_err(|m| {
        String::from_utf8_lossy(&mandrake_session::net::tfl_data_text(m).0).into_owned()
    })?;
    let mesh = FourCC::new(d.parameters.mesh);
    mesh_name(game_dir, mesh).ok_or_else(|| format!("the map {mesh} isn't on the server"))
}

pub fn check_description(
    game_dir: &Path,
    words: DataChecksums,
    d: &GameDescription,
) -> Result<(), String> {
    check_description_with_name(game_dir, words, d).map(|_| ())
}

pub fn mesh_name(game_dir: &Path, mesh: FourCC) -> Option<Vec<u8>> {
    let tags = mandrake_tag::tag_files::open_tfl_tags(game_dir).ok()?;
    let header = load_level(&tags, mesh)?;
    Some(mandrake_session::tfl::level_name(&tags, mesh, &header))
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

fn poll_into(
    core: &mut NetLoop,
    net: &mut NetGame,
    tick: u32,
    host_events: &mut Vec<HostEvent>,
) -> (NetNews, Vec<Event>) {
    let mut sim = Incoming { net, tick };
    let (news, events) = core.poll_events(Some(&mut sim), |_| {});
    if let Some(host) = core.endpoint.as_host_mut() {
        host_events.extend(host.take_events());
    }
    core.end_poll(Some(&mut sim));
    core.send_outgoing(&mut sim, Vec::new(), |_| {});
    (news, events)
}

pub fn film(
    game: &TflNewGame,
    words: DataChecksums,
    name: &[u8],
    end_tick: u32,
    veterans: &[u8],
    commands: &[CommandPacket],
) -> Vec<u8> {
    mandrake_rec::tfl::tfl_film(
        game,
        words,
        name,
        FILM_ID,
        end_tick,
        veterans,
        &TickCommands::group(commands),
    )
}

pub fn next_level(game_dir: &Path, mesh: FourCC, end_code: i16) -> Option<FourCC> {
    let tags = mandrake_tag::tag_files::open_tfl_tags(game_dir).ok()?;
    let header = load_level(&tags, mesh)?;
    match end_code {
        1 => header.next,
        2 => header.alternative,
        _ => None,
    }
    .filter(|&n| n != mesh)
}

fn next_level_with_name(
    game_dir: &Path,
    mesh: FourCC,
    end_code: i16,
) -> Option<(FourCC, Option<Vec<u8>>)> {
    let tags = mandrake_tag::tag_files::open_tfl_tags(game_dir).ok()?;
    let header = load_level(&tags, mesh)?;
    let next = match end_code {
        1 => header.next,
        2 => header.alternative,
        _ => None,
    }
    .filter(|&n| n != mesh)?;
    let name = load_level(&tags, next).map(|h| mandrake_session::tfl::level_name(&tags, next, &h));
    Some((next, name))
}

struct TflNet {
    core: NetLoop,
    net: NetGame,
    host_events: Vec<HostEvent>,
}

impl TflNet {
    fn host(&mut self) -> &mut mandrake_net::endpoint::Host {
        self.core.endpoint.as_host_mut().expect("a host")
    }

    fn poll(&mut self, tick: u32) -> NetNews {
        let (news, events) = poll_into(&mut self.core, &mut self.net, tick, &mut self.host_events);
        log_events(&events);
        news
    }
}

struct TflPostgame<'a> {
    play: &'a mut TflNet,
    tick: u32,
}

impl Postgame for TflPostgame<'_> {
    fn poll(&mut self) -> Vec<HostEvent> {
        self.play.poll(self.tick);
        std::mem::take(&mut self.play.host_events)
    }

    fn endpoint(&self) -> &Endpoint {
        &self.play.core.endpoint
    }

    fn endpoint_mut(&mut self) -> &mut Endpoint {
        &mut self.play.core.endpoint
    }
}

impl Server {
    pub(crate) fn play_tfl(
        &mut self,
        endpoint: Endpoint,
        clock: Instant,
        game: TflNewGame,
        owner: &mut Owner,
    ) -> anyhow::Result<(GameResult, Next)> {
        let description = endpoint
            .joiner()
            .description()
            .cloned()
            .context("no game description at the start")?;
        let words = mandrake_net::tfl::film_checksums(&description);
        let mut play = TflNet {
            core: NetLoop::new(endpoint, Box::new(move || clock.elapsed()), "game"),
            net: NetGame::new(0, self.tfl_sync_counter),
            host_events: Vec::new(),
        };
        play.core.begin_map();
        let names: Vec<String> = game.players.iter().map(|p| p.name.clone()).collect();
        info!(
            "game: TFL, map {}, seed {:#x}, the players {names:?} (the server in none)",
            game.parameters.mesh, game.parameters.seed
        );
        let carried =
            mandrake_tfl_sim::statistics::encode_monster_initializers(&self.tfl_survivors);
        self.tfl_last_players = play
            .core
            .endpoint
            .as_host()
            .map(|h| {
                h.topology()
                    .joined()
                    .filter(|p| p.id != 0)
                    .map(|p| p.id as i16)
                    .collect()
            })
            .unwrap_or_default();
        let loading = Instant::now();
        let set_up =
            TflSim::network_game(&self.game_dir, &film(&game, words, b"", 0, &carried, &[]));
        let mut sim = match set_up {
            Ok(sim) => sim,
            Err(stop) => {
                let why = format!("the TFL map doesn't load: {stop}");
                warn!("game: the start is refused, every player is dropped: {why}");
                play.core.endpoint.leave();
                return Ok((GameResult::refused(why), Next::Leave));
            }
        };
        info!(
            "game: the map is loaded in {} ms",
            loading.elapsed().as_millis()
        );
        let veterans = carried;
        let taken = sim.monster_initializers().iter().filter(|v| v.used).count();
        info!(
            "game: {} veterans carried, {taken} taken",
            sim.monster_initializers().len()
        );
        let mut next_mesh: Option<(FourCC, Option<Vec<u8>>)> = None;
        play.core.map_loaded();
        let mut polls_clock = PollClock::new(play.core.elapsed());
        let mut film_path: Option<PathBuf> = None;
        let mut written = false;
        let mut stopped: Option<String> = None;
        let mut owner_left = false;
        let mut dropped: Vec<usize> = Vec::new();
        loop {
            let begun = Instant::now();
            let polls = if play.core.started && !written && stopped.is_none() {
                polls_clock.polls(play.core.elapsed())
            } else {
                0
            };
            let tick = sim.tick;
            let mut seen = None;
            let step = if polls > 0 {
                let (core, events) = (&mut play.core, &mut play.host_events);
                play.net
                    .frame(&mut sim, polls.min(i16::MAX as u32) as i16, false, |n| {
                        seen = Some(poll_into(core, n, tick, events));
                    })
            } else {
                Ok(None)
            };
            if let Err(stop) = step {
                warn!("game: the TFL sim stopped at tick {}: {stop}", sim.tick);
                stopped = Some(stop.to_string());
            }
            let news = match seen {
                Some((news, events)) => {
                    log_events(&events);
                    news
                }
                None => play.poll(tick),
            };
            if news.started {
                info!("game: begins");
                polls_clock = PollClock::new(play.core.elapsed());
            }
            for e in std::mem::take(&mut play.host_events) {
                match e {
                    HostEvent::PlayerLeft { id, .. } => {
                        let was_owner = owner.left(id);
                        log_host_event("", &e, Some(sim.tick));
                        if was_owner {
                            owner_left = true;
                        }
                    }
                    HostEvent::Ours {
                        from,
                        ours: Ours::ForceEnd { kind },
                    } => {
                        warn!("game: player {from}'s forced end, kind {kind}: TFL has none");
                        play.host().chat_as_to(
                            crate::owner::SERVER_NAME,
                            mandrake_net::lobby::ChatTo::Player(u16::from(from)),
                            b"A TFL game has no Rehost or Restart.",
                        );
                    }
                    other => log_host_event("", &other, Some(sim.tick)),
                }
            }
            if sim.tick != tick {
                for p in sim.dropped_players() {
                    if !dropped.contains(&p) {
                        dropped.push(p);
                        info!(
                            "game: tick {}: DROP_PLAYER {p} ({})",
                            sim.tick,
                            names.get(p).map_or("", String::as_str)
                        );
                    }
                }
                for chat in play.net.take_chats() {
                    info!("game: tick {}: chat from player {}", sim.tick, chat.player);
                }
            }
            let abandon = (stopped.is_some() || owner_left) && !sim.game_over();
            let deserted = play.core.started && !written && alone(&play.core.endpoint);
            if (sim.game_over() || abandon || deserted) && !written {
                written = true;
                if deserted {
                    info!(
                        "game: every player has left at tick {}: the game ends",
                        sim.tick
                    );
                }
                if sim.game_over() {
                    self.tfl_survivors = match sim.map_end_initializers() {
                        Ok(v) => v,
                        Err(stop) => {
                            warn!("game: the survivors weren't kept: {stop}");
                            Vec::new()
                        }
                    };
                    info!("game: {} survivors kept", self.tfl_survivors.len());
                    let mesh = game.parameters.mesh;
                    next_mesh =
                        next_level_with_name(&self.game_dir, mesh, sim.map_actions.game.end_reason);
                } else {
                    self.tfl_survivors.clear();
                }
                film_path =
                    self.write_tfl_film(&game, words, &veterans, &play.net.recorded, sim.tick);
                play.core.map_over();
            }
            let result = |play: &TflNet, sim: &TflSim, film: &Option<PathBuf>| GameResult {
                film: film.clone(),
                end_tick: sim.tick,
                out_of_sync: play.net.out_of_sync.len(),
                owner_left,
                stopped: stopped.clone(),
            };
            if abandon || (written && alone(&play.core.endpoint) && !play.core.ended) {
                if owner_left {
                    info!(
                        "game: the owner left at tick {}: TFL has no forced end, so the server \
                         leaves and the game ends for everyone",
                        sim.tick
                    );
                } else if stopped.is_some() {
                    warn!("game: the server leaves: the game ends for everyone");
                }
                self.log_done(&play, &sim);
                let r = result(&play, &sim, &film_path);
                self.tfl_sync_counter = play.net.sync_counter();
                play.core.endpoint.leave();
                return Ok((r, Next::Leave));
            }
            if play.core.ended || (written && play.core.end_given_up()) {
                self.log_done(&play, &sim);
                let r = result(&play, &sim, &film_path);
                self.tfl_sync_counter = play.net.sync_counter();
                let more = self.max_games.is_none_or(|n| self.games < n);
                let restart = run_postgame(
                    &mut TflPostgame {
                        play: &mut play,
                        tick: sim.tick,
                    },
                    owner,
                    owner_left,
                    more,
                    self.games,
                    self.autohost_restart,
                );
                if restart {
                    if let Some((mesh, name)) = next_mesh {
                        let host = play.host();
                        let mut d = host.lobby().description().clone();
                        d.parameters.mesh = mesh.0;
                        if let Some(name) = name {
                            d.map_name = name;
                        }
                        info!("game: the next level is {mesh}");
                        host.set_description(d);
                    }
                    return Ok((
                        r,
                        Next::Lobby {
                            endpoint: play.core.endpoint,
                            vets: Vec::new(),
                            start_now: false,
                        },
                    ));
                }
                play.core.endpoint.leave();
                return Ok((r, Next::Leave));
            }
            if let Some(rest) = FRAME.checked_sub(begun.elapsed()) {
                play.core.endpoint.wait(rest);
            }
        }
    }

    fn log_done(&self, play: &TflNet, sim: &TflSim) {
        info!(
            "game: done at tick {}: {} commands received, {} sent, {} SYNCs checked, {} out of \
             sync",
            sim.tick,
            play.core.received,
            play.core.sent,
            play.net.syncs_checked,
            play.net.out_of_sync.len()
        );
        for o in &play.net.out_of_sync {
            warn!("game: OUT OF SYNC at tick {}: {}", o.tick, o.line_text());
        }
    }

    fn write_tfl_film(
        &self,
        game: &TflNewGame,
        words: DataChecksums,
        veterans: &[u8],
        recorded: &[CommandPacket],
        end_tick: u32,
    ) -> Option<PathBuf> {
        let ran: Vec<CommandPacket> = recorded
            .iter()
            .filter(|c| c.tick < end_tick)
            .cloned()
            .collect();
        let name = format!("Server Game {}", self.games);
        let data = film(game, words, name.as_bytes(), end_tick, veterans, &ran);
        self.save_film("tflrec", &data, end_tick, ran.len())
    }
}

fn log_events(events: &[Event]) {
    for e in events {
        if !matches!(e, Event::Commands { .. } | Event::AllSynced { .. }) {
            log_event(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_film_of_no_commands_reads_back_with_its_players_and_words() {
        let d = mandrake_session::net::tfl_host_description(
            &mandrake_session::Text(b"TFL".to_vec()),
            &mandrake_session::Text::default(),
            crate::config::DEFAULT_TFL_DATA_WORDS,
        );
        let mut a = mandrake_net::player_data::PlayerData::new(b"alpha", b"A", [0; 8]);
        a.net_id = 1;
        let mut b = mandrake_net::player_data::PlayerData::new(b"beta", b"B", [0; 8]);
        b.net_id = 2;
        let game = mandrake_net::tfl::new_game(&d, &[a, b], 0).unwrap();
        let words = mandrake_net::tfl::film_checksums(&d);
        let bytes = film(&game, words, b"Server Game 1", 300, &[], &[]);
        let rec = mandrake_rec::tfl::TflRecording::from_bytes(&bytes).unwrap();
        assert_eq!(rec.header.new_game(), game);
        assert_eq!(rec.header.checksums, words);
    }
}
