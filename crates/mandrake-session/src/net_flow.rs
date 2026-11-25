use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_net::game_description::new_game_data;

use super::{GameSetup, Session, State};
use crate::action::{Action, GameEvent, GameStart, LevelStart, Request};
use crate::catalog::ENTRY_NET_MASK;
use crate::net::{
    CODE_MIDGAME, CODE_START, ChatLine, ChatTo, FORCED_RESTART, GAME_OVER, GAME_STARTED,
    GameDescription, GameKind, GameOptions, HostOrJoin, JoinFailed, JoinFailure, JoinRequest,
    Joining, Lobby, LobbyScreen, NETGAME_FILM_PREFIX, NetEvent, NetGameStart, NetNotice, NetPlayer,
    NetPostgame, NetProtocol, NetRequest, NetScreen, NetSettings, PlayerData, Plugin,
    PluginDownload, Protocol, description_valid, film_game, game_plugins, host_description,
    invalid_game_text, may_change_options, may_kick, missing_plugins, net_mapsets, other_game_text,
    tfl_data_text, tfl_host_description,
};
use crate::report::PostgameReport;
use crate::text::Text;
use mandrake_net::lobby::InvalidGame;
use mandrake_rec::header::MonsterInitializer;

#[derive(Debug)]
pub(super) enum NetState {
    Player(NetPlayer),
    Protocol(NetProtocol, NetPlayer),
    HostOrJoin(HostOrJoin),
    Joining(Joining, Box<JoinWait>),
    JoinFailed(JoinFailed),
    Lobby,
    MissingPlugins(Vec<Plugin>),
    Downloading(PluginDownload),
    LobbyFailed(JoinFailed),
    GameOptions(Box<GameOptions>),
    EndBarrier,
    Postgame(NetPostgame),
    Notice(NetNotice),
}

const NETWORK_SAVE_NAME: &[u8] = b"Network Save Game";

const OPTION_COOP_NARRATION: u32 = game_options::COOPERATIVE | game_options::NARRATION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NetEpilogue {
    originator: bool,
    waiting: bool,
    undrawn: Option<bool>,
    hidden: bool,
}

impl NetEpilogue {
    pub(super) fn hidden(&self) -> bool {
        self.hidden
    }
}

#[derive(Debug, Default)]
pub(super) struct JoinWait {
    players: Vec<PlayerData>,
    description: Option<GameDescription>,
    owner: bool,
    downloads: Option<Text>,
}

#[derive(Debug)]
pub(super) struct NetGame {
    lobby: Lobby,
    loaded: bool,
    report: PostgameReport,
    end_synced: bool,
    host_lost: bool,
    password: Text,
    latencies: Vec<(i32, u16)>,
    veterans: Option<Vec<MonsterInitializer>>,
    replies: crate::net::BackLog,
    forced_end: i16,
    game_over: bool,
    start_heard: bool,
    next_map: Option<FourCC>,
    next_map_name: Option<Text>,
    end_code: i16,
    download_url: Option<Text>,
    download_tried: Vec<Plugin>,
    disconnected: bool,
    barrier_waiting: Option<Text>,
    saved_game: Option<crate::net::NetSavedGame>,
    save_allowed: bool,
    restoring: Option<crate::screen::Restoring>,
}

impl NetGame {
    fn new(lobby: Lobby, password: Text, download_url: Option<Text>) -> Self {
        Self {
            lobby,
            loaded: false,
            report: PostgameReport::default(),
            end_synced: false,
            host_lost: false,
            password,
            latencies: Vec::new(),
            veterans: None,
            replies: Default::default(),
            forced_end: 0,
            game_over: false,
            start_heard: false,
            next_map: None,
            next_map_name: None,
            end_code: 0,
            download_url,
            download_tried: Vec::new(),
            disconnected: false,
            barrier_waiting: None,
            saved_game: None,
            save_allowed: false,
            restoring: None,
        }
    }

    fn lobby_screen(&self, show_ping: bool) -> LobbyScreen<'_> {
        LobbyScreen {
            lobby: &self.lobby,
            latencies: &self.latencies,
            replies: &self.replies,
            show_ping,
        }
    }

    pub(super) fn restoring(&self) -> Option<&crate::screen::Restoring> {
        self.restoring.as_ref()
    }

    pub(super) fn save_allowed(&self) -> bool {
        self.save_allowed
    }

    pub(super) fn game_over(&self) -> bool {
        self.game_over
    }

    pub(super) fn owner(&self) -> bool {
        self.lobby.owner
    }
}

pub(super) fn screen<'a>(
    state: &'a NetState,
    net: Option<&'a NetGame>,
    show_ping: bool,
) -> NetScreen<'a> {
    match state {
        NetState::Player(s) => NetScreen::Player(s),
        NetState::Protocol(s, _) => NetScreen::Protocol(s),
        NetState::HostOrJoin(s) => NetScreen::HostOrJoin(s),
        NetState::Joining(s, _) => NetScreen::Joining(s),
        NetState::JoinFailed(s) => NetScreen::JoinFailed(s),
        NetState::Lobby => {
            let net = net.expect("the lobby has its game");
            NetScreen::Lobby(net.lobby_screen(show_ping))
        }
        NetState::MissingPlugins(plugins) => {
            let net = net.expect("missing plugins dialog is over the lobby");
            NetScreen::MissingPlugins(net.lobby_screen(show_ping), plugins)
        }
        NetState::Downloading(download) => {
            let net = net.expect("the download is over the lobby");
            NetScreen::Downloading(net.lobby_screen(show_ping), download)
        }
        NetState::LobbyFailed(failed) => {
            let net = net.expect("join failure dialog is over the lobby");
            NetScreen::LobbyFailed(net.lobby_screen(show_ping), failed)
        }
        NetState::GameOptions(s) => NetScreen::GameOptions(s),
        NetState::EndBarrier => NetScreen::EndBarrier {
            waiting: net.and_then(|n| n.barrier_waiting.as_ref()),
        },
        NetState::Postgame(s) => {
            let net = net.expect("net postgame has its game");
            NetScreen::Postgame(s, net.lobby_screen(show_ping))
        }
        NetState::Notice(n) => NetScreen::Notice(*n),
    }
}

pub(super) fn name(state: &NetState) -> &'static str {
    match state {
        NetState::Player(_) => "net player",
        NetState::Protocol(..) => "net protocol",
        NetState::HostOrJoin(_) => "host or join",
        NetState::Joining(..) => "joining",
        NetState::JoinFailed(_) => "join failed",
        NetState::Lobby => "lobby",
        NetState::MissingPlugins(_) => "missing plugins",
        NetState::Downloading(_) => "downloading plugins",
        NetState::LobbyFailed(_) => "lobby join failed",
        NetState::GameOptions(_) => "game options",
        NetState::EndBarrier => "end barrier",
        NetState::Postgame(_) => "net postgame",
        NetState::Notice(_) => "net notice",
    }
}

fn back_log_notices(kind: GameKind) -> bool {
    kind != GameKind::Tfl
}

impl Session {
    fn net_settings(&self) -> NetSettings {
        self.networking
            .as_ref()
            .map(|n| n.settings.clone())
            .unwrap_or_default()
    }

    fn net_settings_mut(&mut self) -> &mut NetSettings {
        &mut self
            .networking
            .get_or_insert_with(|| {
                Box::new(crate::net::Networking {
                    settings: NetSettings::default(),
                    plugins: Vec::new(),
                    foundation_checksum: 0,
                    allow_downloads: false,
                    server: None,
                })
            })
            .settings
    }

    fn host_or_join(&self) -> NetState {
        let settings = self.net_settings();
        NetState::HostOrJoin(HostOrJoin {
            address: settings.address,
            port: settings.port,
            lan_broadcast: (!cfg!(target_arch = "wasm32")).then_some(crate::net::LAN_BROADCAST),
            hosting: false,
            host_failed: None,
        })
    }

    fn keep_net_player(&mut self, p: &NetPlayer) {
        let settings = self.net_settings_mut();
        settings.set_name(&p.name);
        settings.set_team(&p.team);
        settings.icon = p.icon;
        settings.colors = p.colors;
        let settings = settings.clone();
        self.requests.push(Request::SaveNetPlayer(settings));
        self.state = State::Net(self.host_or_join());
    }

    fn net_player(&self) -> NetState {
        let settings = self.net_settings();
        NetState::Player(NetPlayer {
            team: settings.team_text(),
            name: settings.name_text(),
            icon: settings.icon,
            colors: settings.colors,
            metaserver: false,
        })
    }

    fn leave_net_game(&mut self) {
        self.net = None;
        self.game.net = None;
        self.net_load = None;
    }

    fn net_push(&mut self, request: NetRequest) {
        self.requests.push(Request::Net(request));
    }

    pub(super) fn net_act(&mut self, action: Action) -> Option<Action> {
        if matches!(self.state, State::MainMenu) && action == Action::Multiplayer {
            if let Some(server) = self.direct_server().filter(|_| self.main_menu.multiplayer) {
                self.join_direct(server, Text::default());
            } else if self.main_menu.multiplayer {
                self.state = State::Net(self.net_player());
            } else {
                tracing::debug!("session: no networking here");
            }
            return None;
        }
        if matches!(self.state, State::MainMenu)
            && let Action::JoinByAddress { address, password } = &action
            && let Some(server) = self.direct_server().filter(|s| s == address)
        {
            self.join_direct(server, password.clone());
            return None;
        }
        if self.net.is_some()
            && matches!(
                self.state,
                State::Playing(Some(super::OpenOverlay::Paused(_)))
            )
            && action == Action::QuitToMenu
        {
            self.net_push(NetRequest::Leave);
            self.requests.push(Request::DropGame);
            self.leave_net_game();
            self.state = State::MainMenu;
            return None;
        }
        if let State::Playing(overlay @ Some(super::OpenOverlay::UnitTrading(_))) = &mut self.state
        {
            let Some(super::OpenOverlay::UnitTrading(trading)) = overlay.as_mut() else {
                return Some(action);
            };
            match action {
                Action::SetTradeCounts(counts) => {
                    if counts.len() == trading.counts.len() {
                        trading.counts = counts;
                    }
                }
                Action::Ok => {
                    let counts = std::mem::take(&mut trading.counts);
                    *overlay = None;
                    self.net_push(NetRequest::TradeUnits(counts));
                }
                Action::Cancel => *overlay = None,
                action => return Some(action),
            }
            return None;
        }
        if self.net.is_some() && matches!(self.state, State::Postgame(_)) {
            return self.net_epilogue_act(action);
        }
        let direct = self.networking.as_ref().is_some_and(|n| n.server.is_some());
        let tfl = self.game_kind == GameKind::Tfl;
        let State::Net(state) = &mut self.state else {
            return Some(action);
        };
        match (state, action) {
            (NetState::Player(_) | NetState::Protocol(..), action) => {
                self.net_player_act(action, tfl);
            }
            (NetState::HostOrJoin(_), action) => self.net_host_or_join_act(action),
            (NetState::Joining(..), Action::Cancel) => {
                self.join_failed(JoinFailure::Cancelled, Text::default());
            }
            (NetState::JoinFailed(_), Action::Ok) if direct => {
                self.state = State::MainMenu;
            }
            (NetState::JoinFailed(_), Action::Ok) => self.state = State::Net(self.host_or_join()),
            (NetState::MissingPlugins(missing), Action::Ok) if missing.is_empty() => {
                tracing::info!("session: the game's plugins are all here now");
                self.state = State::Net(NetState::Lobby);
                self.describe_net_game();
                self.check_lobby_plugins();
            }
            (NetState::MissingPlugins(_), Action::ReloadPlugins) => {
                self.net_push(NetRequest::ReloadPlugins);
            }
            (NetState::Downloading(_), Action::Cancel) => {
                tracing::info!("session: the download was cancelled");
                self.net_push(NetRequest::CancelDownload);
                self.net_push(NetRequest::Leave);
                self.leave_net_game();
                self.state = State::MainMenu;
            }
            (NetState::MissingPlugins(_) | NetState::LobbyFailed(_), Action::Ok) => {
                self.net_push(NetRequest::Leave);
                self.leave_net_game();
                self.state = State::MainMenu;
            }

            (NetState::Lobby | NetState::GameOptions(_), Action::ChatCommand(command)) => {
                self.chat_command(command);
            }
            (NetState::Postgame(p), Action::ChatCommand(command)) if p.can_chat => {
                self.chat_command(command);
            }
            (NetState::Lobby | NetState::GameOptions(_), Action::LobbyChat(text)) => {
                self.net_push(NetRequest::Chat(text));
            }
            (NetState::Postgame(p), Action::LobbyChat(text)) if p.can_chat => {
                self.net_push(NetRequest::Chat(text));
            }
            (NetState::Postgame(p), Action::LobbyChatTo(to, text)) if p.can_chat => {
                self.send_lobby_chat_to(to, text);
            }
            (NetState::Lobby | NetState::GameOptions(_), Action::LobbyChatTo(to, text)) => {
                self.send_lobby_chat_to(to, text);
            }
            (NetState::Lobby, action) => self.lobby_act(action),
            (NetState::GameOptions(_), action) => self.net_game_options_act(action),
            (NetState::Postgame(_), action) => self.net_postgame_act(action),

            (NetState::Notice(_), Action::Ok) => self.state = State::MainMenu,
            (NetState::EndBarrier, Action::Cancel) => {
                tracing::info!("session: the end barrier given up");
                self.end_barrier_done();
            }

            (state, action) => {
                tracing::debug!(?action, screen = name(state), "session: ignored");
            }
        }
        None
    }

    fn send_lobby_chat_to(&mut self, to: ChatTo, text: Text) {
        if to == ChatTo::All {
            self.net_push(NetRequest::Chat(text));
        } else {
            self.requests.push(Request::NetChatTo(to, text));
        }
    }

    fn net_player_act(&mut self, action: Action, tfl: bool) {
        let State::Net(state) = &mut self.state else {
            return;
        };
        match (state, action) {
            (
                NetState::Player(p),
                Action::SetNetPlayer {
                    name,
                    team,
                    icon,
                    colors,
                },
            ) => {
                p.name = name;
                p.team = team;
                p.icon = icon;
                p.colors = colors;
            }
            (NetState::Player(p), Action::ChooseOther) => {
                let player = p.clone();
                self.state = State::Net(NetState::Protocol(
                    NetProtocol {
                        protocols: vec![Protocol::TcpIp],
                        selected: Protocol::TcpIp,
                    },
                    player,
                ));
            }
            (NetState::Player(_), Action::Cancel) => self.state = State::MainMenu,
            (NetState::Player(p), Action::Ok) if tfl => {
                let p = p.clone();
                self.keep_net_player(&p);
            }
            (NetState::Protocol(_, p), Action::Ok) => {
                let p = p.clone();
                self.keep_net_player(&p);
            }
            (NetState::Protocol(..), Action::Cancel) => self.state = State::MainMenu,
            (state, action) => {
                tracing::debug!(?action, screen = name(state), "session: ignored");
            }
        }
    }

    fn net_host_or_join_act(&mut self, action: Action) {
        let State::Net(NetState::HostOrJoin(h)) = &mut self.state else {
            return;
        };
        match action {
            Action::Ok if h.host_failed.is_some() => {
                h.host_failed = None;
            }
            Action::Cancel if h.hosting => {
                h.hosting = false;
                self.net = None;
                self.net_push(NetRequest::Leave);
            }
            action if h.hosting || h.host_failed.is_some() => {
                tracing::debug!(?action, "session: waiting for the host's game");
            }
            Action::JoinByAddress { address, password } if !address.0.is_empty() => {
                self.join_by_address(address, password);
            }
            Action::JoinLanGame { address } => self.join_lan_game(address),
            Action::HostGame {
                name,
                password,
                port,
            } => self.host_game(name, password, port),
            Action::Cancel => self.state = State::Net(self.net_player()),
            action => tracing::debug!(?action, screen = "host or join", "session: ignored"),
        }
    }

    fn net_game_options_act(&mut self, action: Action) {
        let State::Net(NetState::GameOptions(o)) = &mut self.state else {
            return;
        };
        if o.picker.is_some() {
            self.net_picker_act(action);
            return;
        }
        match action {
            Action::LoadNetSave
                if o.description.parameters.options & game_options::COOPERATIVE != 0 =>
            {
                o.picker = Some(Box::default());
                self.requests.push(Request::ListSaves);
            }
            Action::ClearNetSave => {
                o.saved_game = None;
                o.description.parameters.flags &= !1;
            }
            Action::SetGameOptions(description) => {
                if o.description != *description {
                    o.description = *description;
                    let game = film_game(&o.description);
                    self.requests.push(Request::DescribeNetGame(Box::new(game)));
                }
            }
            Action::SetGamePassword(password) => o.password = password,
            Action::Ok => {
                let description = o.description.clone();
                let password = o.password.clone();
                let saved_game = o.saved_game.clone();
                if let Some(net) = &mut self.net {
                    let path = saved_game.as_ref().map(|s| s.path.clone());
                    if net.saved_game.is_some() || path.is_some() {
                        self.requests.push(Request::Net(NetRequest::NetSave(path)));
                    }
                    net.saved_game = saved_game;
                    net.password = password.clone();
                    self.requests
                        .push(Request::Net(NetRequest::Password(password)));
                    net.lobby.description = description.clone();
                    net.lobby.refresh();
                    let description = net.lobby.description.clone();
                    self.net_push(NetRequest::Options(Box::new(description)));
                }
                self.describe_net_game();
                self.state = State::Net(NetState::Lobby);
            }
            Action::Cancel => self.state = State::Net(NetState::Lobby),
            action => tracing::debug!(?action, screen = "game options", "session: ignored"),
        }
    }

    fn net_postgame_act(&mut self, action: Action) {
        let disconnected = self.net.as_ref().is_some_and(|n| n.disconnected);
        let State::Net(NetState::Postgame(p)) = &self.state else {
            return;
        };
        let can_restart = p.can_restart;
        match action {
            Action::SaveReplay { name } => {
                let mut film = NETGAME_FILM_PREFIX.to_vec();
                film.extend_from_slice(&name.0);
                self.requests.push(Request::SaveFilm { name: Text(film) });
            }
            Action::RestartNetGame if can_restart => {
                self.net_push(NetRequest::Restart);
            }
            Action::Ok if disconnected => {
                self.leave_net_game();
                self.state = State::MainMenu;
            }
            Action::Ok => self.state = State::Net(NetState::Lobby),
            Action::Leave => {
                self.net_push(NetRequest::Leave);
                self.leave_net_game();
                self.state = State::MainMenu;
            }
            action => tracing::debug!(?action, screen = "net postgame", "session: ignored"),
        }
    }

    fn net_picker_act(&mut self, action: Action) {
        let asking = self
            .saved_games_mut()
            .is_some_and(|s| s.delete_save.is_some());
        match action {
            Action::ConfirmDeleteSave if asking => self.delete_save(),
            Action::Cancel if asking => {
                if let Some(s) = self.saved_games_mut() {
                    s.delete_save = None;
                }
            }
            _ if asking => tracing::debug!("session: Delete's question is open"),
            Action::DeleteSave => self.ask_delete_save(),
            Action::ChooseSave(row) => self.choose_save(Some(row)),
            Action::OpenSave => {
                let chosen = self
                    .saved_games_mut()
                    .filter(|s| s.can_open)
                    .and_then(|s| s.selected)
                    .and_then(|row| self.saves.saves.get(row))
                    .cloned();
                let none = self.saved_games_mut().is_some_and(|s| s.selected.is_none());
                if let State::Net(NetState::GameOptions(o)) = &mut self.state {
                    if let Some(save) = chosen {
                        o.description =
                            crate::net::description_from_save(&o.description, &save.game);
                        o.saved_game = Some(crate::net::NetSavedGame {
                            name: save.name.clone(),
                            snapshot: save.snapshot.clone(),
                            path: save.path.clone(),
                        });
                        o.picker = None;
                        let game = film_game(&o.description);
                        self.requests.push(Request::DescribeNetGame(Box::new(game)));
                    } else if none {
                        o.picker = None;
                    }
                }
            }
            Action::Cancel => {
                if let State::Net(NetState::GameOptions(o)) = &mut self.state {
                    o.picker = None;
                }
            }
            action => tracing::debug!(?action, "session: Continue Saved Game is open over !ngp"),
        }
    }

    fn describe_net_game(&mut self) {
        if let Some(net) = &self.net {
            let game = film_game(&net.lobby.description);
            self.requests.push(Request::DescribeNetGame(Box::new(game)));
        }
    }

    fn lobby_act(&mut self, action: Action) {
        let Some(net) = &mut self.net else {
            return;
        };
        let lobby = &mut net.lobby;
        match action {
            Action::SetReady(ready) => {
                let id = lobby.local_id;
                if let Some(mut me) = lobby.player(id).cloned() {
                    me.ready = u32::from(ready);
                    lobby.set_player(me);
                }
                self.net_push(NetRequest::Ready(ready));
            }
            Action::OpenOptions if may_change_options(lobby) => {
                let installed = self
                    .networking
                    .as_ref()
                    .map(|n| n.plugins.as_slice())
                    .unwrap_or_default();
                let options = GameOptions {
                    description: lobby.description.clone(),
                    password: net.password.clone(),
                    mapsets: net_mapsets(
                        &self.catalog.net_mapsets,
                        installed,
                        ENTRY_NET_MASK,
                        &self.catalog.net_max_teams,
                    ),
                    cooperative_mapsets: match self.tfl.as_deref() {
                        Some(tfl) => vec![crate::tfl::coop_mapset(&tfl.archive, &tfl.prefs)],
                        None => net_mapsets(
                            &self.catalog.campaigns,
                            installed,
                            crate::rules::ENTRY_SOLO,
                            &self.catalog.net_max_teams,
                        ),
                    },
                    difficulties: self.catalog.difficulties.clone(),
                    details: lobby.details.clone(),
                    saved_game: net.saved_game.clone(),
                    picker: None,
                };
                self.state = State::Net(NetState::GameOptions(Box::new(options)));
            }
            Action::StartNetGame if lobby.can_start => self.start_net_game(),
            Action::Kick(id) if may_kick(lobby, id) => {
                if lobby.host {
                    let lowered = mandrake_net::lobby::boot_unready(
                        &lobby.net_players(),
                        lobby.local_id as u16,
                        id,
                    );
                    for p in lobby.players.iter_mut() {
                        if lowered.iter().any(|&l| i32::from(l) == p.net_id) {
                            p.ready = 0;
                        }
                    }
                }
                lobby.remove_player(id);
                self.net_push(NetRequest::Kick(id));
            }
            Action::TeamButton(chosen) => self.net_push(NetRequest::TeamButton(chosen)),
            Action::TeamKick(id) => {
                if let Ok(id) = u16::try_from(id) {
                    self.net_push(NetRequest::TeamKick(id));
                }
            }
            Action::TeamLock => self.net_push(NetRequest::TeamLock),
            Action::Lock(locked) if may_change_options(lobby) => {
                lobby.locked = locked;
                self.net_push(NetRequest::Lock(locked));
            }
            Action::Leave => {
                self.net_push(NetRequest::Leave);
                self.leave_net_game();
                self.state = State::MainMenu;
            }
            action => tracing::debug!(?action, "session: not in this lobby"),
        }
    }

    fn join_by_address(&mut self, address: Text, password: Text) {
        let settings = self.net_settings_mut();
        if settings.address != address {
            settings.address = address.clone();
            let settings = settings.clone();
            self.requests.push(Request::SaveNetPlayer(settings));
        }
        self.join(address, password);
    }

    fn join_lan_game(&mut self, address: Text) {
        self.join(address, Text::default());
    }

    fn direct_server(&self) -> Option<Text> {
        self.networking.as_ref().and_then(|n| n.server.clone())
    }

    fn join_direct(&mut self, server: Text, password: Text) {
        tracing::info!(
            server = %String::from_utf8_lossy(&server.0),
            "session: joining the site's server"
        );
        self.join(server, password);
        if let State::Net(NetState::Joining(joining, _)) = &mut self.state {
            joining.direct = true;
        }
    }

    fn our_player_data(&self) -> PlayerData {
        let mut p = self.net_settings().player_data();
        if self.game_kind == GameKind::Tfl {
            p.version = mandrake_net::player_data::TFL_PLAYER_VERSION;
        }
        p
    }

    fn join(&mut self, address: Text, password: Text) {
        let settings = self.net_settings();
        self.net_push(NetRequest::Join {
            address: address.clone(),
            request: Box::new(JoinRequest {
                name: settings.name_text().0,
                password: password.0,
                player: self.our_player_data(),
            }),
        });
        self.state = State::Net(NetState::Joining(
            Joining {
                address,
                id: None,
                direct: false,
            },
            Box::default(),
        ));
    }

    fn host_game(&mut self, name: Text, password: Text, port: u16) {
        let settings = self.net_settings_mut();
        if settings.port != port {
            settings.port = port;
            let settings = settings.clone();
            self.requests.push(Request::SaveNetPlayer(settings));
        }
        let foundation = self
            .networking
            .as_ref()
            .map_or(0, |n| n.foundation_checksum);
        let description = match self.game_kind {
            GameKind::Myth2 => host_description(&name, &password, foundation),
            GameKind::Tfl => tfl_host_description(&name, &password, self.tfl_data),
        };
        let mut me = self.our_player_data();
        me.net_id = crate::net::HOST_ID;
        let mut lobby = Lobby {
            host: true,
            owner: false,
            local_id: crate::net::HOST_ID,
            players: vec![me.clone()],
            description: description.clone(),
            chat: crate::net::BackLog::default(),
            locked: false,
            can_start: false,
            host_controls: false,
            details: None,
        };
        lobby.refresh();
        self.net = Some(NetGame::new(lobby, password.clone(), None));
        self.net_push(NetRequest::Host {
            port,
            password,
            player: Box::new(me),
            description: Box::new(description),
        });
        self.describe_net_game();
        if let State::Net(NetState::HostOrJoin(h)) = &mut self.state {
            h.hosting = true;
            h.port = port;
        }
    }

    fn join_failed(&mut self, reason: JoinFailure, text: Text) {
        tracing::info!(?reason, "session: the join failed");
        self.net_push(NetRequest::Leave);
        self.leave_net_game();
        self.state = State::Net(NetState::JoinFailed(JoinFailed { reason, text }));
    }

    fn installed_plugins(&self) -> &[Plugin] {
        self.networking
            .as_ref()
            .map(|n| n.plugins.as_slice())
            .unwrap_or_default()
    }

    fn check_lobby_plugins(&mut self) {
        let Some(net) = &self.net else { return };
        if net.lobby.host || !matches!(self.state, State::Net(NetState::Lobby)) {
            return;
        }
        let description = &net.lobby.description;
        let missing = missing_plugins(description, self.installed_plugins());
        if !missing.is_empty() {
            tracing::info!(
                count = missing.len(),
                "session: the game's plugins are missing"
            );
            self.missing_or_download(missing);
        } else if self.game_kind != GameKind::Tfl && !description_valid(description) {
            let text = invalid_game_text(description, self.installed_plugins());
            tracing::info!("session: the game's version doesn't match");
            self.state = State::Net(NetState::LobbyFailed(JoinFailed {
                reason: JoinFailure::InvalidGame,
                text,
            }));
        }
    }

    fn missing_or_download(&mut self, missing: Vec<Plugin>) {
        let allowed = self.networking.as_ref().is_some_and(|n| n.allow_downloads);
        let url = match &mut self.net {
            Some(net) if allowed && net.download_tried != missing => {
                net.download_url.clone().inspect(|_| {
                    net.download_tried = missing.clone();
                })
            }
            _ => None,
        };
        match url {
            Some(url) => {
                tracing::info!(
                    count = missing.len(),
                    url = %String::from_utf8_lossy(&url.0),
                    "session: downloading the game's plugins from our server"
                );
                self.net_push(NetRequest::DownloadPlugins {
                    url,
                    plugins: missing.clone(),
                });
                self.state = State::Net(NetState::Downloading(PluginDownload {
                    plugins: missing,
                    progress: Default::default(),
                }));
            }
            None => self.state = State::Net(NetState::MissingPlugins(missing)),
        }
    }

    fn try_enter_lobby(&mut self) {
        let State::Net(NetState::Joining(joining, wait)) = &mut self.state else {
            return;
        };
        let Some(id) = joining.id else {
            return;
        };
        let Some(description) = wait.description.take() else {
            return;
        };
        let mut players = std::mem::take(&mut wait.players);
        let owner = wait.owner;
        let download_url = wait.downloads.take();
        if let Err(InvalidGame::OtherGame(theirs)) =
            mandrake_net::lobby::check_as(self.game_kind, &description)
        {
            self.join_failed(JoinFailure::InvalidGame, other_game_text(theirs));
            return;
        }
        if self.game_kind == GameKind::Tfl
            && let Err(mismatch) = mandrake_net::tfl::check_data(&description, self.tfl_data)
        {
            self.join_failed(JoinFailure::InvalidGame, tfl_data_text(mismatch));
            return;
        }
        if !players.iter().any(|p| p.net_id == id) {
            let mut me = self.our_player_data();
            me.net_id = id;
            players.push(me);
        }
        let mut lobby = Lobby {
            host: false,
            owner,
            local_id: id,
            players,
            description,
            chat: crate::net::BackLog::default(),
            locked: false,
            can_start: false,
            host_controls: false,
            details: None,
        };
        lobby.refresh();
        self.net = Some(NetGame::new(lobby, Text::default(), download_url));
        self.state = State::Net(NetState::Lobby);
        self.describe_net_game();
        self.check_lobby_plugins();
    }

    fn start_net_load(&mut self) {
        self.start_net_load_from(None);
    }

    fn start_net_load_from(&mut self, midgame: Option<Vec<u8>>) {
        let Some(net) = &mut self.net else {
            return;
        };
        let lobby = &net.lobby;
        let players: Vec<PlayerData> = lobby
            .players
            .iter()
            .map(|p| PlayerData {
                ready: 1,
                ..p.clone()
            })
            .collect();
        let kind = GameKind::of(&lobby.description);
        let built = match (kind, &midgame) {
            (GameKind::Myth2, Some(start)) => Ok((
                mandrake_net::game_description::midgame_new_game_data(
                    &lobby.description,
                    start,
                    &lobby.players,
                ),
                None,
            )),
            (GameKind::Myth2, None) => {
                new_game_data(&lobby.description, &players, lobby.local_id).map(|data| (data, None))
            }
            (GameKind::Tfl, _) => {
                mandrake_net::tfl::new_game(&lobby.description, &players, lobby.local_id)
                    .map(|tfl| (mandrake_rec::writer::NewGameData(Vec::new()), Some(tfl)))
            }
        };
        let (new_game_data, tfl) = match built {
            Ok(built) => built,
            Err(error) => {
                tracing::error!(?error, "session: no new-game data for the network game");
                return;
            }
        };
        let parameters = &lobby.description.parameters;
        let mesh = FourCC(parameters.mesh);
        let difficulty = parameters.difficulty;
        let options = parameters.options;
        let plugins = game_plugins(&lobby.description);
        let plugin_checksums: Vec<i32> = lobby
            .description
            .plugins
            .0
            .iter()
            .rev()
            .map(|p| p.checksum as i32)
            .collect();
        let start = NetGameStart {
            new_game_data,
            local_id: lobby.local_id,
            host: lobby.host,
            kind,
            tfl,
        };
        let restarted = net.forced_end == FORCED_RESTART;
        net.loaded = false;
        net.end_synced = false;
        net.host_lost = false;
        net.barrier_waiting = None;
        net.forced_end = 0;
        net.game_over = false;
        net.save_allowed = false;
        net.restoring = None;
        net.start_heard = false;
        net.next_map = None;
        net.next_map_name = None;
        net.end_code = 0;
        let plugin_checksum = self.plugin_checksum_for_mesh(mesh, plugin_checksums, &plugins);
        self.game = GameSetup {
            mesh: Some(mesh),
            plugin_checksum,
            difficulty,
            plugins,
            film: None,
            restore: false,
            start: LevelStart::NewGame,
            net: Some(start.clone()),
        };
        let load = GameStart {
            start: LevelStart::NewGame,
            mesh,
            difficulty,
            plugins: self.game.plugins.clone(),
            veterans: Vec::new(),
            reset_rankings: false,
            film: None,
            restore: false,
            net: Some(start),
        };
        if kind == GameKind::Myth2
            && options & OPTION_COOP_NARRATION == OPTION_COOP_NARRATION
            && !restarted
            && midgame.is_none()
        {
            tracing::info!("session: the network game's prologue");
            self.net_load = Some(load);
            self.enter_level();
            return;
        }
        self.requests.push(Request::LoadGame(load));
        self.state = State::Loading;
    }

    pub(super) fn net_game_event(&mut self, event: GameEvent) -> Option<GameEvent> {
        let event = match event {
            GameEvent::Net(event) => {
                self.net_event(event);
                return None;
            }
            event => event,
        };
        let Some(net) = &mut self.net else {
            return Some(event);
        };
        match (&self.state, event) {
            (State::Loading, GameEvent::Loaded) if net.host_lost => {
                net.loaded = true;
                self.endi_disconnected();
            }
            (State::Loading, GameEvent::Loaded) => {
                net.loaded = true;
                self.net_push(NetRequest::Loaded);
            }
            (State::Loading, GameEvent::LoadFailed(error)) => {
                tracing::error!(%error, "session: the network game's map didn't load");
                self.net_push(NetRequest::Leave);
                self.leave_net_game();
                self.state = State::Net(NetState::Notice(NetNotice::LoadFailed));
            }
            (State::Playing(_), GameEvent::MapOver { report, veterans }) => {
                net.end_code = report.end_code;
                net.report = report;
                net.veterans = Some(veterans);
                if !net.host_lost && self.open_net_epilogue() {
                    return None;
                }
                self.end_network_game();
            }
            (_, event) => return Some(event),
        }
        None
    }

    fn open_net_epilogue(&mut self) -> bool {
        let Some(net) = &self.net else {
            return false;
        };
        let description = &net.lobby.description;
        let p = &description.parameters;
        if GameKind::of(description) != GameKind::Myth2 || net.forced_end == FORCED_RESTART {
            return false;
        }
        if p.options & OPTION_COOP_NARRATION != OPTION_COOP_NARRATION {
            if p.options & game_options::COOPERATIVE != 0 && net.forced_end == 0 {
                self.mark_next_level_visited();
            }
            return false;
        }
        let host = net.lobby.host;
        let report = net.report.clone();
        let mesh = FourCC(p.mesh);
        let build = net
            .lobby
            .players
            .iter()
            .map(|p| p.build as i16)
            .min()
            .unwrap_or(-1);
        let gameplay = crate::films::compute_gameplay_version(p.options, p.version as i32, build);
        let old_gameplay = i32::from(gameplay) <= game_options::GAMEPLAY_1_8_4;
        let forced = net.forced_end != 0;
        let rule = crate::rules::postgame(self.level_in_game(mesh), report.end_code, false);
        let (mut choice, art) = match rule {
            crate::rules::PostgameRule::Dialog(choice) => (choice, true),
            crate::rules::PostgameRule::Undrawn(choice) => (choice, false),
            _ => return false,
        };
        let undrawn = (forced || old_gameplay || !art).then_some(choice.won);
        if !host {
            choice.exits.next = false;
            choice.exits.restart = false;
            choice.exits.skip = false;
        }
        tracing::info!(
            end_code = report.end_code,
            host,
            ?undrawn,
            "session: the network game's epilogue"
        );
        self.open_postgame(
            report,
            choice,
            Some(NetEpilogue {
                originator: host,
                waiting: false,
                undrawn,
                hidden: undrawn.is_some(),
            }),
        );
        self.net_epilogue_opened();
        true
    }

    fn mark_next_level_visited(&mut self) {
        let Some(net) = &self.net else {
            return;
        };
        if net.end_code != 1 {
            return;
        }
        let mesh = FourCC(net.lobby.description.parameters.mesh);
        let Some(level) = self.level_in_game(mesh) else {
            return;
        };
        if let Some(next) = level.next {
            let checksum = self
                .level_in_game(next)
                .map_or(level.plugin_checksum, |l| l.plugin_checksum);
            self.progress_changed |= self.progress.mark_visited(next, checksum);
        }
    }

    fn net_epilogue_act(&mut self, action: Action) -> Option<Action> {
        let State::Postgame(after) = &mut self.state else {
            return Some(action);
        };
        let Some(epilogue) = after.network.as_mut() else {
            return Some(action);
        };
        if after.screen.save_film.is_some() {
            return Some(action);
        }
        let exits = &after.screen.exits;
        let kind = match action {
            Action::NextLevel if exits.next => 2,
            Action::SkipLevel if exits.skip => 2,
            Action::RestartLevel if exits.restart => FORCED_RESTART,
            Action::QuitToMenu if epilogue.originator => 2,
            Action::QuitToMenu => {
                if epilogue.waiting {
                    return None;
                }
                tracing::info!("session: the epilogue's Quit: no next level");
                if let Some(net) = &mut self.net {
                    net.end_code = 0;
                }
                self.end_network_game();
                return None;
            }
            Action::NextLevel
            | Action::SkipLevel
            | Action::RestartLevel
            | Action::LastSavedGame => {
                tracing::debug!(?action, "session: not in this epilogue");
                return None;
            }
            action => return Some(action),
        };
        if std::mem::replace(&mut epilogue.waiting, true) {
            return None;
        }
        tracing::info!(kind, "session: the epilogue forces the network's end");
        self.net_push(NetRequest::ForceEnd(kind));
        None
    }

    pub(super) fn net_epilogue_opened(&mut self) {
        let State::Postgame(after) = &mut self.state else {
            return;
        };
        let Some(epilogue) = after.network.as_mut() else {
            return;
        };
        if self.net.as_ref().is_some_and(|n| n.forced_end != 0) {
            self.end_network_game();
            return;
        }
        match epilogue.undrawn.take() {
            Some(true) => {
                epilogue.waiting = true;
                tracing::info!("session: the epilogue's default item, Next Level: the forced end");
                self.net_push(NetRequest::ForceEnd(2));
            }
            Some(false) => {
                tracing::info!("session: the epilogue's default item: no save, it goes on");
                self.end_network_game();
            }
            None => {}
        }
    }

    fn end_network_game(&mut self) {
        let next_choice = self.net.as_ref().and_then(|net| {
            if GameKind::of(&net.lobby.description) == GameKind::Myth2
                && (1..=2).contains(&net.end_code)
            {
                let mesh = FourCC(net.lobby.description.parameters.mesh);
                let next = self
                    .level_in_game(mesh)
                    .and_then(|l| {
                        if net.end_code == 1 {
                            l.next
                        } else {
                            l.alternate
                        }
                    })
                    .filter(|&n| n != mesh)?;
                let name = self.level_in_game(next).map(|l| l.title.clone());
                Some((next, name))
            } else {
                None
            }
        });
        let Some(net) = &mut self.net else {
            return;
        };
        if let Some((next, name)) = next_choice {
            tracing::info!(%next, "session: the co-op game's next level");
            net.next_map = Some(next);
            net.next_map_name = name;
        }
        let synced = std::mem::take(&mut net.end_synced);
        let host_lost = net.host_lost;
        self.net_push(NetRequest::MapOver);
        self.state = State::Net(NetState::EndBarrier);
        if host_lost && synced {
            tracing::info!("session: the host left after the end barrier: the main menu");
            self.host_lost_in_endi();
        } else if synced || host_lost {
            self.end_barrier_done();
        }
    }

    fn take_next_map(&mut self) {
        let Some(net) = &mut self.net else {
            return;
        };
        let Some(mesh) = net.next_map.take() else {
            return;
        };
        tracing::info!(%mesh, "session: the next level");
        net.lobby.description.parameters.mesh = mesh.0;
        if let Some(name) = net.next_map_name.take() {
            net.lobby.description.map_name = name.0;
        }
        let had_save =
            net.saved_game.take().is_some() || net.lobby.description.parameters.flags & 1 != 0;
        if had_save {
            net.lobby.description.parameters.flags &= !1;
        }
        net.lobby.refresh();
        let host_description = net.lobby.host.then(|| net.lobby.description.clone());
        if let Some(description) = host_description {
            if had_save {
                self.net_push(NetRequest::NetSave(None));
            }
            self.net_push(NetRequest::Description(Box::new(description)));
        }
        self.describe_net_game();
    }

    fn chat_command(&mut self, command: crate::chat_commands::DotCommand) {
        use crate::chat_commands::{self as dot, DotCommand, LoadRefused};
        if command == DotCommand::Ping {
            self.show_ping = !self.show_ping;
            return;
        }
        let local = self.local_plugins();
        let Some(net) = &mut self.net else {
            return;
        };
        let options = net.lobby.description.parameters.options;
        let table = net.veterans.as_deref().unwrap_or_default();
        let lines = match &command {
            DotCommand::Vets => vec![dot::vets_line(net.lobby.host, options, table.len()).to_vec()],
            DotCommand::SaveVets(name) => {
                let request = NetRequest::SaveVeterans {
                    name: name.clone(),
                    table: mandrake_rec::writer::initializer_block(table),
                };
                self.net_push(request);
                return;
            }
            DotCommand::LoadVets(name)
                if !net.lobby.host || options & game_options::COOPERATIVE == 0 =>
            {
                vec![dot::load_line(&name.0, Err(LoadRefused::NotCoopHost))]
            }
            DotCommand::LoadVets(name) => {
                self.net_push(NetRequest::LoadVeterans { name: name.clone() });
                return;
            }
            command => dot::lines(command, &net.lobby, &local),
        };
        for line in lines {
            self.chat_reply(&line);
        }
        tracing::info!(?command, "session: a chat dot command");
    }

    fn chat_reply(&mut self, line: &[u8]) {
        if let Some(net) = &mut self.net {
            net.replies.push(ChatLine::notice(line));
        }
    }

    fn veterans_loaded(&mut self, name: &Text, table: Option<Vec<MonsterInitializer>>) {
        use crate::chat_commands::{self as dot, LoadRefused};

        let Some(net) = &mut self.net else {
            return;
        };
        let Some(table) = table else {
            self.chat_reply(&dot::load_line(&name.0, Err(LoadRefused::Unreadable)));
            return;
        };
        tracing::info!(veterans = table.len(), "session: .loadvets");
        net.veterans = Some(table);
        let both = game_options::VETERANS | game_options::CARRY_VETERANS;
        if net.lobby.description.parameters.options & both != both {
            net.lobby.description.parameters.options |= both;
            net.lobby.refresh();
            let description = net.lobby.description.clone();
            self.net_push(NetRequest::Description(Box::new(description)));
        }
        self.chat_reply(&dot::load_line(&name.0, Ok(())));
    }

    fn local_plugins(&self) -> Vec<crate::chat_commands::LocalPlugin> {
        let Some(networking) = &self.networking else {
            return Vec::new();
        };
        networking
            .plugins
            .iter()
            .map(|p| crate::chat_commands::LocalPlugin {
                name: p.name.clone(),
                checksum: p.checksum,
                has_levels: self.catalog.listed_plugins.contains(&(p.checksum as i32)),
            })
            .collect()
    }

    fn start_net_game(&mut self) {
        let vets = self
            .net
            .as_ref()
            .and_then(|n| n.veterans.as_deref())
            .map(mandrake_rec::writer::initializer_block);
        self.net_push(NetRequest::Start { vets });
    }

    fn end_barrier_done(&mut self) {
        let Some(net) = &mut self.net else {
            return;
        };
        if net.forced_end == FORCED_RESTART {
            tracing::info!("session: Restart: the next game starts");
            let heard = std::mem::take(&mut net.start_heard);
            let host = net.lobby.host;
            net.report = PostgameReport::default();
            self.state = State::Net(NetState::Lobby);
            self.take_next_map();
            if host {
                self.start_net_game();
            } else if heard {
                self.start_net_load();
            }
            return;
        }
        if back_log_notices(self.game_kind) {
            net.lobby.chat.push(ChatLine::notice(GAME_OVER));
        }
        self.state = State::Net(NetState::Postgame(NetPostgame {
            report: std::mem::take(&mut net.report),
            can_restart: may_change_options(&net.lobby),
            can_chat: true,
        }));
    }

    fn host_lost_in_endi(&mut self) {
        self.net_push(NetRequest::Leave);
        self.leave_net_game();
        self.state = State::MainMenu;
    }

    fn endi_disconnected(&mut self) {
        tracing::info!("session: the host left in the load: endi, disconnected");
        self.net_push(NetRequest::Standings);
        self.requests.push(Request::DropGame);
        self.net_push(NetRequest::Leave);
        if let Some(net) = &mut self.net {
            net.disconnected = true;
            net.report = PostgameReport::default();
            net.latencies = net.lobby.players.iter().map(|p| (p.net_id, 0)).collect();
            if back_log_notices(self.game_kind) {
                net.lobby.chat.push(ChatLine::notice(GAME_STARTED));
                net.lobby.chat.push(ChatLine::notice(GAME_OVER));
            }
        }
        self.state = State::Net(NetState::Postgame(NetPostgame {
            report: PostgameReport::default(),
            can_restart: false,
            can_chat: false,
        }));
    }

    fn joining_net_event(&mut self, event: NetEvent) -> Option<NetEvent> {
        let State::Net(NetState::Joining(joining, wait)) = &mut self.state else {
            return Some(event);
        };
        match event {
            NetEvent::Joined { id } => {
                joining.id = Some(id);
                self.try_enter_lobby();
            }
            NetEvent::Description(description) => {
                wait.description = Some(*description);
                self.try_enter_lobby();
            }
            NetEvent::Player(player) => {
                match wait.players.iter_mut().find(|p| p.net_id == player.net_id) {
                    Some(p) => *p = *player,
                    None => wait.players.push(*player),
                }
            }
            NetEvent::PlayerLeft { id } => {
                wait.players.retain(|p| p.net_id != id);
            }
            NetEvent::BadDescription => {
                self.join_failed(JoinFailure::InvalidGame, Text::default());
            }
            NetEvent::Denied(reason) => {
                self.join_failed(JoinFailure::Denied(reason), Text::default());
            }
            NetEvent::TimedOut => {
                self.join_failed(JoinFailure::TimedOut, Text::default());
            }
            NetEvent::Unreachable { error } => {
                self.join_failed(JoinFailure::Unreachable(error), Text::default());
            }
            NetEvent::Unresolved => {
                tracing::info!("session: the address didn't resolve");
                let direct = joining.direct;
                self.net_push(NetRequest::Leave);
                self.leave_net_game();
                self.state = if direct {
                    State::MainMenu
                } else {
                    State::Net(self.host_or_join())
                };
            }
            NetEvent::Owner => wait.owner = true,
            NetEvent::DownloadsFrom(url) => wait.downloads = Some(url),
            NetEvent::HostLost => self.join_failed(JoinFailure::HostLost, Text::default()),
            NetEvent::Midgame(_) => self.join_failed(JoinFailure::InProgress, Text::default()),
            other => return Some(other),
        }
        None
    }

    fn host_lost_net_event(&mut self) {
        match &self.state {
            State::Playing(_) => {
                if let Some(net) = &mut self.net {
                    net.host_lost = true;
                }
            }
            State::Net(NetState::EndBarrier) => {
                if let Some(net) = &mut self.net {
                    net.host_lost = true;
                }
                self.end_barrier_done();
            }
            State::Net(NetState::Postgame(_)) => {
                tracing::info!("session: the host left endi: the main menu");
                self.host_lost_in_endi();
            }
            State::Net(NetState::Lobby | NetState::GameOptions(_)) => {
                tracing::info!("session: the host left the lobby: the main menu");
                self.net_push(NetRequest::Leave);
                self.leave_net_game();
                self.state = State::MainMenu;
            }
            State::Loading => {
                let loaded = self.net.as_mut().is_some_and(|net| {
                    net.host_lost = true;
                    net.loaded
                });
                if loaded {
                    self.endi_disconnected();
                } else {
                    tracing::info!("session: the host left in the load: endi once it's loaded");
                }
            }
            state => {
                if matches!(state, State::Net(NetState::Downloading(_))) {
                    self.net_push(NetRequest::CancelDownload);
                }
                self.net_push(NetRequest::Leave);
                self.leave_net_game();
                self.state = State::Net(NetState::Notice(NetNotice::HostLost));
            }
        }
    }

    fn net_event(&mut self, event: NetEvent) {
        let Some(event) = self.joining_net_event(event) else {
            return;
        };
        if matches!(event, NetEvent::HostLost) && self.net.is_some() {
            self.host_lost_net_event();
            return;
        }
        match (&mut self.state, event) {
            (_, NetEvent::Available(networking)) => {
                self.main_menu.multiplayer = true;
                self.networking = Some(networking);
            }
            (State::Net(NetState::HostOrJoin(h)), NetEvent::Listening) if h.hosting => {
                self.state = State::Net(NetState::Lobby);
            }
            (State::Net(NetState::HostOrJoin(h)), NetEvent::HostFailed(reason)) if h.hosting => {
                h.hosting = false;
                h.host_failed = Some(reason);
                self.net = None;
            }

            (_, NetEvent::DownloadsFrom(url)) if self.net.is_some() => {
                if let Some(net) = &mut self.net {
                    net.download_url = Some(url);
                }
                if let State::Net(NetState::MissingPlugins(missing)) = &self.state {
                    let missing = missing.clone();
                    self.missing_or_download(missing);
                }
            }
            (State::Net(NetState::Downloading(d)), NetEvent::DownloadProgress(progress)) => {
                d.progress = progress;
            }
            (State::Net(NetState::Downloading(_)), NetEvent::DownloadDone(failed)) => {
                if let Some(why) = &failed {
                    tracing::warn!(
                        "session: the download failed: {}",
                        String::from_utf8_lossy(&why.0)
                    );
                }
                let installed = self.installed_plugins().to_vec();
                let missing = self
                    .net
                    .as_ref()
                    .map(|n| missing_plugins(&n.lobby.description, &installed))
                    .unwrap_or_default();
                if missing.is_empty() {
                    tracing::info!("session: the game's plugins are all here now");
                    self.state = State::Net(NetState::Lobby);
                    self.describe_net_game();
                    self.check_lobby_plugins();
                } else {
                    self.missing_or_download(missing);
                }
            }
            (state, NetEvent::Owner) if self.net.is_some() => {
                if let Some(net) = &mut self.net {
                    tracing::info!("session: we own this game on our server");
                    net.lobby.owner = true;
                    net.lobby.refresh();
                    if let State::Net(NetState::Postgame(p)) = state
                        && !net.disconnected
                    {
                        p.can_restart = may_change_options(&net.lobby);
                    }
                }
            }
            (State::Net(NetState::Lobby), NetEvent::Midgame(start))
                if self.net.as_ref().is_some_and(|n| !n.lobby.host) =>
            {
                tracing::info!("session: state {CODE_MIDGAME:#x}: the game is in play; joining it");
                self.start_net_load_from(Some(start));
            }

            (State::Net(NetState::Lobby), NetEvent::Configuration(CODE_START)) => {
                self.start_net_load();
            }
            (State::Net(NetState::Postgame(_)), NetEvent::Configuration(CODE_START)) => {
                self.state = State::Net(NetState::Lobby);
                self.take_next_map();
            }
            (State::Playing(_) | State::Net(NetState::EndBarrier), NetEvent::NextMap(mesh))
                if self
                    .net
                    .as_ref()
                    .is_some_and(|n| GameKind::of(&n.lobby.description) == GameKind::Tfl) =>
            {
                if let Some(net) = &mut self.net {
                    net.next_map = Some(mesh);
                }
            }

            (state, NetEvent::Chat { name, text, kind }) if self.net.is_some() => match state {
                State::Net(
                    NetState::Lobby
                    | NetState::MissingPlugins(_)
                    | NetState::Downloading(_)
                    | NetState::GameOptions(_)
                    | NetState::Postgame(_),
                ) => {
                    if let Some(net) = &mut self.net {
                        net.lobby.chat.push(ChatLine { kind, name, text });
                    }
                }
                _ => tracing::debug!("session: chat out of the lobby"),
            },

            (State::Loading, NetEvent::AllSynced)
                if self.net.as_ref().is_some_and(|n| n.loaded) =>
            {
                self.mission_advances = 0;
                if let Some(net) = &mut self.net
                    && back_log_notices(self.game_kind)
                {
                    net.lobby.chat.push(ChatLine::notice(GAME_STARTED));
                }
                self.requests.push(Request::BeginPlay);
                self.state = State::Playing(None);
            }
            (State::Net(NetState::EndBarrier), NetEvent::AllSynced) => self.end_barrier_done(),
            (
                State::Postgame(_) | State::Cutscene(..) | State::CampaignEnd(..),
                NetEvent::ForcedEnd(kind),
            ) if self.net.as_ref().is_some_and(|n| n.forced_end == 0) => {
                if let Some(net) = &mut self.net {
                    tracing::info!(kind, "session: the forced end came in the postgame");
                    net.forced_end = kind;
                    if let Some(line) = crate::net::forced_end_line(kind) {
                        net.lobby.chat.push(ChatLine::notice(line));
                    }
                }
                self.net_epilogue_opened();
            }
            (State::Playing(_), NetEvent::ForcedEnd(kind)) => {
                if let Some(net) = &mut self.net
                    && net.forced_end == 0
                {
                    net.forced_end = kind;
                    if let Some(line) = crate::net::forced_end_line(kind) {
                        net.lobby.chat.push(ChatLine::notice(line));
                    }
                }
            }
            (_, NetEvent::Plugins(plugins)) => {
                tracing::info!(count = plugins.len(), "session: the plugins read again");
                if let Some(networking) = &mut self.networking {
                    networking.plugins = plugins;
                }
                let installed = self.installed_plugins().to_vec();
                if let (State::Net(NetState::MissingPlugins(missing)), Some(net)) =
                    (&mut self.state, &self.net)
                {
                    *missing = missing_plugins(&net.lobby.description, &installed);
                }
            }
            (_, NetEvent::SaveAllowed(allowed)) => {
                if let Some(net) = &mut self.net {
                    net.save_allowed = allowed;
                }
            }
            (State::Playing(_), NetEvent::SaveRestoring { ticks, end }) => {
                if let Some(net) = &mut self.net {
                    let name = match (&net.saved_game, net.lobby.host) {
                        (Some(save), true) => save.name.clone(),
                        _ => Text(NETWORK_SAVE_NAME.to_vec()),
                    };
                    let steps = i64::from(end / super::RESTORE_TICKS_PER_STEP + 1);
                    let done = i64::from(ticks.div_ceil(super::RESTORE_TICKS_PER_STEP)).min(steps);
                    net.restoring = Some(crate::screen::Restoring {
                        name,
                        fraction: ((done << 8) / steps) as i16,
                        network: true,
                    });
                }
            }
            (_, NetEvent::SaveRestored) => {
                if let Some(net) = &mut self.net {
                    net.restoring = None;
                }
            }
            (State::Playing(_), NetEvent::GameOver) => {
                if let Some(net) = &mut self.net {
                    net.game_over = true;
                }
            }
            (
                State::Playing(_) | State::Net(NetState::EndBarrier),
                NetEvent::Configuration(CODE_START),
            ) if self
                .net
                .as_ref()
                .is_some_and(|n| n.forced_end == FORCED_RESTART && !n.lobby.host) =>
            {
                if let Some(net) = &mut self.net {
                    net.start_heard = true;
                }
            }
            (State::Playing(overlay @ None), NetEvent::UnitTrading(trading)) => {
                *overlay = Some(super::OpenOverlay::UnitTrading(*trading));
            }
            (
                State::Playing(overlay @ Some(super::OpenOverlay::UnitTrading(_))),
                NetEvent::TradingOver,
            ) => {
                if let Some(super::OpenOverlay::UnitTrading(trading)) = overlay.take() {
                    self.net_push(NetRequest::TradeUnits(trading.counts));
                }
            }
            (State::Playing(_), NetEvent::AllSynced) => {
                if let Some(net) = &mut self.net {
                    net.end_synced = true;
                }
            }

            (State::Net(NetState::Postgame(p)), NetEvent::Standings(report))
                if self.net.as_ref().is_some_and(|n| n.disconnected) =>
            {
                p.report = *report;
            }
            (_, NetEvent::BarrierWaiting(id)) if self.net.is_some() => {
                if let Some(net) = &mut self.net {
                    net.barrier_waiting = id.and_then(|id| {
                        net.lobby.player(id).map(|p| {
                            Text(mandrake_net::player_data::until_nul(&p.appearance.name).to_vec())
                        })
                    });
                }
            }

            (_, NetEvent::Description(description)) if self.net.is_some() => {
                if let Some(net) = &mut self.net {
                    net.lobby.description = *description;
                    net.lobby.refresh();
                }
                self.describe_net_game();
                if matches!(self.state, State::Net(NetState::MissingPlugins(_)))
                    && let Some(net) = &self.net
                {
                    let missing = missing_plugins(&net.lobby.description, self.installed_plugins());
                    if missing.is_empty() {
                        self.state = State::Net(NetState::Lobby);
                        self.check_lobby_plugins();
                    } else {
                        self.missing_or_download(missing);
                    }
                } else {
                    self.check_lobby_plugins();
                }
            }
            (State::Net(NetState::GameOptions(o)), NetEvent::GameDescribed(game)) => {
                o.details = Some(crate::screen::FilmDetails {
                    name: Text(o.description.game_name.clone()),
                    game: *game,
                });
            }
            (_, NetEvent::GameDescribed(game)) if self.net.is_some() => {
                if let Some(net) = &mut self.net {
                    net.lobby.details = Some(crate::screen::FilmDetails {
                        name: Text(net.lobby.description.game_name.clone()),
                        game: *game,
                    });
                }
            }
            (_, NetEvent::VeteransSaved { name, saved }) => {
                self.chat_reply(&crate::chat_commands::save_line(&name.0, saved));
            }
            (_, NetEvent::VeteransLoaded { name, table }) => self.veterans_loaded(&name, table),
            (_, NetEvent::Latencies(_)) if self.net.as_ref().is_some_and(|n| n.disconnected) => {}
            (_, NetEvent::Latencies(latencies)) if self.net.is_some() => {
                if let Some(net) = &mut self.net {
                    net.latencies = latencies;
                }
            }
            (_, NetEvent::Player(player)) if self.net.is_some() => {
                if let Some(net) = &mut self.net {
                    net.lobby.set_player(*player);
                }
            }
            (state, NetEvent::PlayerLeft { id }) if self.net.is_some() => {
                if matches!(state, State::Playing(_)) {
                    tracing::info!(id, "session: a player left the game");
                }
                if let Some(net) = &mut self.net {
                    net.lobby.remove_player(id);
                }
            }

            (state, event) => {
                tracing::debug!(?event, screen = state.name(), "session: ignored");
            }
        }
    }
}
