use std::path::{Path, PathBuf};

use mandrake_net::endpoint::{Endpoint, HostOptions};
use mandrake_net::game_kind::GameKind;
use mandrake_net::player_data::{JoinRequest, until_nul};
use mandrake_session::net::{NetEvent, NetRequest, Networking};
use mandrake_session::net_game::NetLoop;
use mandrake_session::{GameEvent, Request, Session, Text};
use tracing::{info, warn};

use super::{TflNetPlay, TflNetWindow};
use crate::args::Args;
use crate::clock::Instant;

const CONNECT_FAILED: i32 = -4994;

#[cfg(not(target_arch = "wasm32"))]
struct Connecting {
    result: std::sync::mpsc::Receiver<std::io::Result<Endpoint>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Connecting {
    fn start(host: std::net::SocketAddr, request: JoinRequest) -> Self {
        let (send, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(Endpoint::join_as(GameKind::Tfl, host, request, 0));
        });
        Self { result }
    }

    fn poll(&mut self) -> Option<std::io::Result<Endpoint>> {
        use std::sync::mpsc::TryRecvError;
        match self.result.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(std::io::Error::other(
                "the connect's thread ended with no answer",
            ))),
        }
    }
}

#[cfg(target_arch = "wasm32")]
type Connecting = crate::web::Connecting;

#[cfg(target_arch = "wasm32")]
fn web_tfl_address(host: &str) -> String {
    if host.contains('/') {
        host.to_owned()
    } else {
        format!("{host}{WEB_TFL_PATH}")
    }
}

#[cfg(target_arch = "wasm32")]
const WEB_TFL_PATH: &str = "/myth2-ws/tfl";

enum Held {
    None,
    Lobby(Box<NetLoop>),
    Game(Box<TflNetWindow>),
}

pub(crate) struct TflMenuNet {
    offered: bool,
    announced: bool,
    connecting: Option<Connecting>,
    clock: Instant,
    held: Held,
    sync_counter: i32,
    survivors: Vec<mandrake_tfl_sim::MonsterInitializer>,
    carry_serial: u32,
    internal: Vec<Vec<u8>>,
    settings_path: Option<PathBuf>,
    archive: std::sync::Arc<mandrake_tag::TagArchive>,
}

impl TflMenuNet {
    pub(crate) fn new(
        archive: impl Into<std::sync::Arc<mandrake_tag::TagArchive>>,
        offered: bool,
        settings_path: Option<PathBuf>,
    ) -> Self {
        let archive = archive.into();
        let id = mandrake_common::FourCC(*b"inns");
        let internal = mandrake_session::tfl::tfl_tag(&archive, b"stli", id)
            .and_then(|(v, d)| mandrake_tag::tfl::menus::TflStringList::from_tag(id, v, &d).ok())
            .map(|l| l.strings)
            .unwrap_or_default();
        Self {
            offered,
            announced: false,
            connecting: None,
            clock: Instant::now(),
            held: Held::None,
            sync_counter: 0,
            survivors: Vec::new(),
            carry_serial: 0,
            internal,
            settings_path,
            archive,
        }
    }

    fn take_solo_survivors(&mut self, session: &Session) {
        let serial = session.tfl_carry_serial();
        if serial == self.carry_serial {
            return;
        }
        self.carry_serial = serial;
        self.survivors = session
            .tfl_carry()
            .and_then(|c| c.as_any().downcast_ref::<crate::tfl_live::TflCarry>())
            .map(|c| c.veterans.clone())
            .unwrap_or_default();
        info!(
            "TFL net: {} survivors of the last solo game for the next network film",
            self.survivors.len()
        );
    }

    fn core(&mut self) -> Option<&mut NetLoop> {
        match &mut self.held {
            Held::None => None,
            Held::Lobby(core) => Some(core),
            Held::Game(w) => Some(w.play.core_mut()),
        }
    }

    pub(crate) fn step(&mut self, session: &mut Session) {
        self.take_solo_survivors(session);
        let mut events = Vec::new();
        if !self.announced && self.offered {
            self.announced = true;
            let settings = self
                .settings_path
                .as_deref()
                .and_then(|p| crate::settings::load_tfl_net_settings(p).ok())
                .unwrap_or_else(crate::settings::net_defaults);
            info!(
                "TFL net: networking offered, the player {:?} from [net.tfl]",
                settings.name.to_plain_utf8()
            );
            events.push(NetEvent::Available(Box::new(Networking {
                settings,
                plugins: Vec::new(),
                foundation_checksum: 0,
                allow_downloads: false,
                #[cfg(target_arch = "wasm32")]
                server: Some(Text(
                    web_tfl_address(&crate::web::site_address()).into_bytes(),
                )),
                #[cfg(not(target_arch = "wasm32"))]
                server: None,
            })));
        }
        if let Some(answer) = self.connecting.as_mut().and_then(Connecting::poll) {
            self.connecting = None;
            match answer {
                Ok(mut endpoint) => {
                    info!(
                        "TFL net: connected in {} ms",
                        self.clock.elapsed().as_millis()
                    );
                    endpoint.set_internal_messages(self.internal.clone());
                    self.held = Held::Lobby(Box::new(self.new_loop(endpoint)));
                }
                Err(e) => {
                    warn!("TFL net: the connect: {e}");
                    events.push(NetEvent::Unreachable {
                        error: CONNECT_FAILED,
                    });
                }
            }
        }
        events.extend(self.poll());
        for e in events {
            session.game_event(GameEvent::Net(e));
        }
    }

    fn poll(&mut self) -> Vec<NetEvent> {
        match &mut self.held {
            Held::None => Vec::new(),
            Held::Lobby(core) => {
                let (_, events) = core.poll_events(None, |_| {});
                if let Some(host) = core.endpoint.as_host_mut() {
                    for e in host.take_events() {
                        mandrake_session::net_game::log_host_event("TFL net: host: ", &e, None);
                    }
                }
                core.end_poll(None);
                for e in &events {
                    tracing::debug!("TFL net: {e:?}");
                }
                events.iter().filter_map(NetEvent::from_endpoint).collect()
            }
            Held::Game(w) => {
                let tick = w.tick();
                w.play.poll(tick);
                w.play.take_session_events()
            }
        }
    }

    fn new_loop(&self, endpoint: Endpoint) -> NetLoop {
        let clock = self.clock;
        NetLoop::new(endpoint, Box::new(move || clock.elapsed()), "net")
    }

    pub(crate) fn perform(
        &mut self,
        request: Request,
        game: Option<&mut TflNetWindow>,
    ) -> Vec<NetEvent> {
        let mut out = Vec::new();
        let request = match request {
            Request::SaveNetPlayer(settings) => {
                if let Some(path) = &self.settings_path
                    && let Err(e) = crate::settings::save_tfl_net_settings(path, &settings)
                {
                    warn!(
                        "settings: {}: the TFL player wasn't saved: {e}",
                        path.display()
                    );
                }
                return out;
            }
            Request::Net(NetRequest::Join { address, request }) => {
                #[allow(unused_mut)]
                let mut text = String::from_utf8_lossy(&address.0).trim().to_owned();
                self.leave();
                #[cfg(target_arch = "wasm32")]
                {
                    let url = web_tfl_address(&text);
                    self.clock = Instant::now();
                    match Connecting::start_as(GameKind::Tfl, &url, *request) {
                        Ok(c) => self.connecting = Some(c),
                        Err(e) => {
                            warn!("TFL net: {url}: {e}");
                            out.push(NetEvent::Unreachable {
                                error: CONNECT_FAILED,
                            });
                        }
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if !text.contains(':') {
                        text = format!("{text}:{}", mandrake_session::net::DEFAULT_PORT);
                    }
                    match crate::net_game::resolve(&text) {
                        Ok(host) => {
                            info!("TFL net: connecting to {host}");
                            self.clock = Instant::now();
                            self.connecting = Some(Connecting::start(host, *request));
                        }
                        Err(e) => {
                            warn!("TFL net: {e}");
                            out.push(NetEvent::Unresolved);
                        }
                    }
                }
                return out;
            }
            Request::Net(NetRequest::Host {
                port,
                password,
                player,
                description,
            }) => {
                self.leave();
                let options = HostOptions {
                    player: JoinRequest {
                        name: until_nul(&player.appearance.name).to_vec(),
                        password: Vec::new(),
                        player: *player,
                    },
                    password: password.0,
                    description: *description,
                };
                self.clock = Instant::now();
                match Endpoint::host(port, options, 0) {
                    Ok(mut endpoint) => {
                        info!("TFL net: hosting on {:?}", endpoint.listen_addr());
                        endpoint.set_internal_messages(self.internal.clone());
                        self.held = Held::Lobby(Box::new(self.new_loop(endpoint)));
                        out.push(NetEvent::Listening);
                    }
                    Err(e) => {
                        warn!("TFL net: listening on port {port}: {e}");
                        out.push(NetEvent::HostFailed(Text(e.to_string().into_bytes())));
                    }
                }
                return out;
            }
            Request::Net(NetRequest::Leave) => {
                self.connecting = None;
                self.leave();
                return out;
            }
            Request::Net(NetRequest::Standings) => {
                out.push(NetEvent::Standings(Box::default()));
                return out;
            }
            Request::Net(NetRequest::ReloadPlugins) => {
                out.push(NetEvent::Plugins(Vec::new()));
                return out;
            }
            Request::DescribeNetGame(mut game) => {
                mandrake_session::tfl::describe_net_game(&self.archive, &mut game);
                out.push(NetEvent::GameDescribed(game));
                return out;
            }
            Request::Net(NetRequest::TradeUnits(counts)) => {
                match game {
                    Some(w) => w.trade_units(&counts),
                    None => warn!("TFL net: untr's trade {counts:?} with no map"),
                }
                return out;
            }
            r @ (Request::Net(_) | Request::NetChatTo(..)) => r,
            other => {
                tracing::debug!(?other, "TFL net: not a network request");
                return out;
            }
        };
        let core = match game {
            Some(w) => w.play.core_mut(),
            None => match self.core() {
                Some(core) => core,
                None => {
                    warn!("TFL net: {request:?} with no game");
                    return out;
                }
            },
        };
        match request {
            Request::Net(r) if owner_request(core, &r) => {}
            Request::Net(NetRequest::Loaded) => core.map_loaded(),
            Request::Net(NetRequest::MapOver) => core.map_over(),
            Request::Net(r) => endpoint_request(core, r),
            Request::NetChatTo(to, text) => {
                let sent = core.endpoint.joiner_mut().chat(to, &text.0);
                core.endpoint.flush();
                if !sent {
                    warn!("TFL net: the chat line to {to:?} wasn't sent");
                }
            }
            _ => {}
        }
        out
    }

    pub(crate) fn take_game(
        &mut self,
        args: &Args,
        game_dir: &Path,
    ) -> anyhow::Result<(
        Box<TflNetWindow>,
        crate::tfl_map::TflMap,
        Option<crate::tfl_game::TflReplay>,
    )> {
        match std::mem::replace(&mut self.held, Held::None) {
            Held::Lobby(core) => {
                let mut play = match TflNetPlay::try_from_loop(
                    *core,
                    self.sync_counter,
                    self.survivors.clone(),
                ) {
                    Ok(play) => play,
                    Err((core, err)) => {
                        self.held = Held::Lobby(core);
                        return Err(err);
                    }
                };
                play.forward_to_session();
                info!(
                    "TFL net: the game starts: net id {}, local player {} of {}, seed {:#x}, map {}",
                    play.local_id,
                    play.new_game.local_player,
                    play.new_game.players.len(),
                    play.new_game.parameters.seed,
                    play.new_game.parameters.mesh
                );
                let mut window = Box::new(TflNetWindow::new(args, play, self.clock));
                match window.load(game_dir) {
                    Ok((map, replay)) => Ok((window, map, replay)),
                    Err(err) => {
                        self.held = Held::Game(window);
                        Err(err)
                    }
                }
            }
            Held::Game(mut window) => {
                info!("TFL net: endi's Restart: the next map");
                match window.next_map(game_dir) {
                    Ok((map, replay)) => Ok((window, map, replay)),
                    Err(err) => {
                        self.held = Held::Game(window);
                        Err(err)
                    }
                }
            }
            Held::None => anyhow::bail!("TFL net: a game started with no endpoint"),
        }
    }

    pub(crate) fn hold(&mut self, window: Box<TflNetWindow>) {
        self.sync_counter = window.play.net.sync_counter();
        self.survivors = window.play.survivors().to_vec();
        self.held = Held::Game(window);
    }

    pub(crate) fn leave(&mut self) {
        match std::mem::replace(&mut self.held, Held::None) {
            Held::None => {}
            Held::Lobby(core) => {
                info!("TFL net: leaving the lobby");
                core.endpoint.leave();
            }
            Held::Game(window) => {
                info!("TFL net: leaving the game");
                window.leave();
            }
        }
    }
}

pub(crate) fn pump(
    session: &mut Session,
    progress: Option<&crate::tfl_menu::ProgressFile>,
    net: &mut TflMenuNet,
    mut game: Option<&mut TflNetWindow>,
) -> Option<crate::tfl_menu::TflLaunch> {
    let mut launch = None;
    for _ in 0..16 {
        let (l, others) = crate::tfl_menu::take_launch_and_others(session, progress);
        launch = l.or(launch);
        if others.is_empty() {
            break;
        }
        for request in others {
            for event in net.perform(request, game.as_deref_mut()) {
                session.game_event(GameEvent::Net(event));
            }
        }
    }
    launch
}

fn owner_request(core: &mut NetLoop, request: &NetRequest) -> bool {
    use mandrake_net::lobby::code;
    use mandrake_net::ours::Ours;
    if core.endpoint.as_host_mut().is_some() || !core.endpoint.joiner().is_owner() {
        return false;
    }
    let j = core.endpoint.joiner_mut();
    let sent = match request {
        NetRequest::Description(d) | NetRequest::Options(d) => {
            j.send_ours(Ours::Description(d.to_bytes()))
        }
        NetRequest::Start { .. } | NetRequest::Restart => j.owner_configure(code::START, 0),
        NetRequest::Kick(id) => match u16::try_from(*id) {
            Ok(id) => j.send_ours(Ours::Boot { id }),
            Err(_) => false,
        },
        NetRequest::Lock(locked) => j.send_ours(Ours::Lock { locked: *locked }),
        NetRequest::Password(p) => j.send_ours(Ours::Password(p.0.clone())),
        _ => false,
    };
    core.endpoint.flush();
    if sent {
        info!("TFL net: owner: {request:?} to our server");
    }
    sent
}

fn endpoint_request(core: &mut NetLoop, request: NetRequest) {
    let endpoint = &mut core.endpoint;
    match request {
        NetRequest::Ready(ready) => {
            endpoint.joiner_mut().set_ready(ready);
        }
        NetRequest::TeamButton(chosen) => {
            endpoint.joiner_mut().team_button(chosen);
        }
        NetRequest::TeamKick(id) => {
            endpoint.joiner_mut().team_kick(id);
        }
        NetRequest::TeamLock => {
            endpoint.joiner_mut().team_lock();
        }
        NetRequest::Chat(text) => {
            if !endpoint
                .joiner_mut()
                .chat(mandrake_net::lobby::ChatTo::All, &text.0)
            {
                warn!("TFL net: the chat line wasn't sent");
            }
        }
        NetRequest::Description(d) => {
            if let Some(host) = endpoint.as_host_mut() {
                host.set_description(*d);
            }
        }
        NetRequest::Options(d) => {
            if let Some(host) = endpoint.as_host_mut() {
                info!("TFL net: host: the new game options; every Ready cleared");
                host.set_options(*d);
            }
        }
        NetRequest::Start { vets } => {
            if let Some(host) = endpoint.as_host_mut() {
                info!("TFL net: host: Start");
                host.set_after_game(vets.is_some());
                host.set_vets(None);
                host.start(crate::net_game::machine_ticks());
            }
        }
        NetRequest::Restart => {
            if let Some(host) = endpoint.as_host_mut() {
                info!("TFL net: host: Restart Game");
                host.restart_game();
            }
        }
        NetRequest::Kick(id) => {
            if let (Some(host), Ok(id)) = (endpoint.as_host_mut(), u8::try_from(id)) {
                info!("TFL net: host: boot player {id}");
                host.boot(id);
            }
        }
        NetRequest::Lock(locked) => {
            if let Some(host) = endpoint.as_host_mut() {
                host.set_accepting(!locked);
            }
        }
        NetRequest::Password(password) => {
            if let Some(host) = endpoint.as_host_mut() {
                host.set_password(password.0);
            }
        }
        other => warn!("TFL net: {other:?} isn't TFL's"),
    }
    endpoint.flush();
}
