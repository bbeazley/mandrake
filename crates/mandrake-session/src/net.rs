use mandrake_common::game_options;
pub use mandrake_net::game_description::{
    FLAG_PASSWORD, GameDescription, GameParameters, Plugin, PluginList,
};
use mandrake_net::game_description::{GAMEPLAY_VERSION_PLUGINS, gameplay_version_plugin};
pub use mandrake_net::game_kind::GameKind;
pub use mandrake_net::lan::BROADCAST as LAN_BROADCAST;
pub use mandrake_net::lobby::ChatTo;
use mandrake_net::player_data::{BUILD_NUMBER, GAMEPLAY_VERSION, copy_over, fixed, until_nul};
pub use mandrake_net::player_data::{JoinRequest, PlayerData};
pub use mandrake_net::tfl::DataChecksums;
pub use mandrake_net::wire::DEFAULT_PORT;
use mandrake_rec::writer::NewGameData;

use crate::films::{FilmGame, FilmPlugin};
use crate::report::PostgameReport;
use crate::screen::FilmDetails;
use crate::text::Text;

pub const CODE_START: u16 = 100;
pub const CODE_MIDGAME: u16 = 0x65;
pub const VERSIONS: std::ops::RangeInclusive<u32> = mandrake_net::lobby::VERSION_RANGE;
pub const HOST_ID: i32 = 0;
const NET_MODE: i16 = 1;
const DEFAULT_GAME_TYPE: i16 = 2;
const DEFAULT_OPTIONS: u32 = 0x200c_4034;
const DEFAULT_TIME_LIMIT: i32 = 10 * 1800;
const DEFAULT_MESH: [u8; 4] = *b"85gi";
const DEFAULT_DIFFICULTY: i16 = 2;
const GAMEPLAY_PREFERENCE: i16 = -1852;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetSettings {
    pub name: Text,
    pub team: Text,
    pub icon: u16,
    pub colors: [u16; 8],
    pub port: u16,
    pub address: Text,
}

impl Default for NetSettings {
    fn default() -> Self {
        Self {
            name: Text::default(),
            team: Text::default(),
            icon: 0,
            colors: [0; 8],
            port: DEFAULT_PORT,
            address: Text::default(),
        }
    }
}

impl NetSettings {
    pub fn player_data(&self) -> PlayerData {
        let mut p = PlayerData::new(&self.name.0, &self.team.0, self.colors);
        p.appearance.field_00 = self.icon;
        p
    }

    pub fn name_text(&self) -> Text {
        Text(until_nul(&self.name.0).to_vec())
    }

    pub fn set_name(&mut self, name: &Text) {
        self.name = over_field(&self.name, name);
    }

    pub fn team_text(&self) -> Text {
        Text(until_nul(&self.team.0).to_vec())
    }

    pub fn set_team(&mut self, team: &Text) {
        self.team = over_field(&self.team, team);
    }
}

fn over_field(old: &Text, typed: &Text) -> Text {
    let mut field: [u8; 32] = fixed(&old.0);
    copy_over(&mut field, &typed.0);
    let len = field.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    Text(field[..len].to_vec())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Networking {
    pub settings: NetSettings,
    pub plugins: Vec<Plugin>,
    pub foundation_checksum: u32,
    pub allow_downloads: bool,
    pub server: Option<Text>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginDownload {
    pub plugins: Vec<Plugin>,
    pub progress: DownloadProgress,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DownloadProgress {
    pub index: usize,
    pub received: u64,
    pub total: u64,
}

impl PluginDownload {
    pub fn text(&self) -> Text {
        let name = self
            .plugins
            .get(self.progress.index)
            .map(|p| String::from_utf8_lossy(&p.name).into_owned())
            .unwrap_or_default();
        let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
        let mut line = format!(
            "Downloading {name} ({} of {})",
            (self.progress.index + 1).min(self.plugins.len()),
            self.plugins.len()
        );
        if self.progress.total > 0 {
            line.push_str(&format!(
                ": {:.1} of {:.1} MB",
                mb(self.progress.received),
                mb(self.progress.total)
            ));
        }
        Text(line.into_bytes())
    }

    pub fn fraction(&self) -> i16 {
        match self.progress.total {
            0 => 0,
            t => (u128::from(self.progress.received.min(t)) * 0x100 / u128::from(t)) as i16,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetRequest {
    Host {
        port: u16,
        password: Text,
        player: Box<PlayerData>,
        description: Box<GameDescription>,
    },
    Join {
        address: Text,
        request: Box<JoinRequest>,
    },
    Ready(bool),
    Description(Box<GameDescription>),
    Options(Box<GameDescription>),
    Start {
        vets: Option<Vec<u8>>,
    },
    Restart,
    Chat(Text),
    SaveVeterans {
        name: Text,
        table: Vec<u8>,
    },
    LoadVeterans {
        name: Text,
    },
    Password(Text),
    TeamButton(Option<i32>),
    TeamKick(u16),
    TeamLock,
    ReloadPlugins,
    DownloadPlugins {
        url: Text,
        plugins: Vec<Plugin>,
    },
    CancelDownload,
    Kick(i32),
    Lock(bool),
    Leave,
    Loaded,
    MapOver,
    Standings,
    ForceEnd(i16),
    TradeUnits(Vec<i16>),
    NetSave(Option<std::path::PathBuf>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetEvent {
    SaveAllowed(bool),
    SaveRestoring {
        ticks: u32,
        end: u32,
    },
    SaveRestored,
    Available(Box<Networking>),
    Listening,
    HostFailed(Text),
    Joined {
        id: i32,
    },
    Denied(Text),
    TimedOut,
    Unreachable {
        error: i32,
    },
    Unresolved,
    Description(Box<GameDescription>),
    BadDescription,
    Player(Box<PlayerData>),
    PlayerLeft {
        id: i32,
    },
    Configuration(u16),
    Midgame(Vec<u8>),
    Chat {
        name: Text,
        text: Text,
        kind: ChatKind,
    },
    Latencies(Vec<(i32, u16)>),
    VeteransSaved {
        name: Text,
        saved: Option<bool>,
    },
    VeteransLoaded {
        name: Text,
        table: Option<Vec<mandrake_rec::header::MonsterInitializer>>,
    },
    BeginSync,
    AllSynced,
    HostLost,
    BarrierWaiting(Option<i32>),
    Standings(Box<PostgameReport>),
    Owner,
    DownloadsFrom(Text),
    DownloadProgress(DownloadProgress),
    DownloadDone(Option<Text>),
    GameDescribed(Box<FilmGame>),
    ForcedEnd(i16),
    GameOver,
    Plugins(Vec<Plugin>),
    UnitTrading(Box<crate::screen::UnitTrading>),
    TradingOver,
    NextMap(mandrake_common::FourCC),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetGameStart {
    pub new_game_data: NewGameData,
    pub local_id: i32,
    pub host: bool,
    pub kind: GameKind,
    pub tfl: Option<mandrake_rec::tfl::TflNewGame>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NetScreen<'a> {
    Player(&'a NetPlayer),
    Protocol(&'a NetProtocol),
    HostOrJoin(&'a HostOrJoin),
    Joining(&'a Joining),
    JoinFailed(&'a JoinFailed),
    Lobby(LobbyScreen<'a>),
    MissingPlugins(LobbyScreen<'a>, &'a [Plugin]),
    Downloading(LobbyScreen<'a>, &'a PluginDownload),
    LobbyFailed(LobbyScreen<'a>, &'a JoinFailed),
    GameOptions(&'a GameOptions),
    EndBarrier { waiting: Option<&'a Text> },
    Postgame(&'a NetPostgame, LobbyScreen<'a>),
    Notice(NetNotice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetNotice {
    HostLost,
    LoadFailed,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetPlayer {
    pub name: Text,
    pub team: Text,
    pub icon: u16,
    pub colors: [u16; 8],
    pub metaserver: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    TcpIp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetProtocol {
    pub protocols: Vec<Protocol>,
    pub selected: Protocol,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostOrJoin {
    pub address: Text,
    pub port: u16,
    pub lan_broadcast: Option<std::net::SocketAddrV4>,
    pub hosting: bool,
    pub host_failed: Option<Text>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Joining {
    pub address: Text,
    pub id: Option<i32>,
    pub direct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinFailure {
    Denied(Text),
    TimedOut,
    Unreachable(i32),
    Cancelled,
    InvalidGame,
    InProgress,
    HostLost,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinFailed {
    pub reason: JoinFailure,
    pub text: Text,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatLine {
    pub kind: ChatKind,
    pub name: Text,
    pub text: Text,
}

impl ChatLine {
    pub fn said(name: Text, text: Text) -> Self {
        Self {
            kind: ChatKind::Line,
            name,
            text,
        }
    }

    pub fn notice(text: &[u8]) -> Self {
        Self {
            kind: ChatKind::Notice,
            name: Text::default(),
            text: Text(text.to_vec()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackLog {
    lines: std::collections::VecDeque<ChatLine>,
    added: u64,
}

impl BackLog {
    pub const SLOTS: usize = 64;

    pub fn push(&mut self, line: ChatLine) {
        if self.lines.len() == Self::SLOTS - 1 {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.added += 1;
    }

    pub fn lines(&self) -> impl ExactSizeIterator<Item = &ChatLine> {
        self.lines.iter()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn added(&self) -> u64 {
        self.added
    }

    pub fn since(&self, seen: u64) -> impl Iterator<Item = &ChatLine> {
        let seen = if seen > self.added { 0 } else { seen };
        let new = usize::try_from(self.added - seen)
            .map_or(self.lines.len(), |n| n.min(self.lines.len()));
        self.lines.iter().skip(self.lines.len() - new)
    }
}

impl From<Vec<ChatLine>> for BackLog {
    fn from(lines: Vec<ChatLine>) -> Self {
        let mut log = Self::default();
        for line in lines {
            log.push(line);
        }
        log
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChatKind {
    #[default]
    Line,
    Private,
    Notice,
    Team,
}

impl ChatKind {
    pub const fn code(self) -> i16 {
        match self {
            Self::Line => 0,
            Self::Private => 1,
            Self::Notice => 2,
            Self::Team => 4,
        }
    }

    pub const fn of_mode(mode: u16) -> Self {
        match mode {
            mandrake_net::lobby::chat::ALL => Self::Line,
            mandrake_net::lobby::chat::TEAM => Self::Team,
            _ => Self::Private,
        }
    }
}

pub const GAME_STARTED: &[u8] = b"Game Started";
pub const GAME_OVER: &[u8] = b"Game Over";
pub fn forced_end_line(kind: i16) -> Option<&'static [u8]> {
    match kind {
        2 => Some(b"Rehosting..."),
        3 => Some(b"Restarting..."),
        _ => None,
    }
}
pub const FORCED_RESTART: i16 = 3;
pub const NETGAME_FILM_PREFIX: &[u8] = b"Netgame- ";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LobbyScreen<'a> {
    pub lobby: &'a Lobby,
    pub latencies: &'a [(i32, u16)],
    pub replies: &'a BackLog,
    pub show_ping: bool,
}

impl LobbyScreen<'_> {
    pub fn latency(&self, id: i32) -> u16 {
        self.latencies
            .iter()
            .find(|(i, _)| *i == id)
            .map_or(u16::MAX, |&(_, ms)| ms)
    }
}

impl std::ops::Deref for LobbyScreen<'_> {
    type Target = Lobby;

    fn deref(&self) -> &Lobby {
        self.lobby
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lobby {
    pub host: bool,
    pub owner: bool,
    pub local_id: i32,
    pub players: Vec<PlayerData>,
    pub description: GameDescription,
    pub chat: BackLog,
    pub locked: bool,
    pub can_start: bool,
    pub host_controls: bool,
    pub details: Option<FilmDetails>,
}

impl Lobby {
    pub fn player(&self, id: i32) -> Option<&PlayerData> {
        self.players.iter().find(|p| p.net_id == id)
    }

    pub fn net_players(&self) -> Vec<mandrake_net::topology::Player> {
        self.players
            .iter()
            .map(|p| mandrake_net::topology::Player {
                id: u16::try_from(p.net_id).unwrap_or(u16::MAX),
                name: until_nul(&p.appearance.name).to_vec(),
                data: p.clone(),
            })
            .collect()
    }

    fn player_by_wire_id(&self, id: i32) -> Option<&PlayerData> {
        self.players
            .iter()
            .find(|p| i32::from(u16::try_from(p.net_id).unwrap_or(u16::MAX)) == id)
    }

    pub fn team_button(&self, chosen: Option<i32>) -> mandrake_net::lobby::TeamButton {
        let off = mandrake_net::lobby::TeamButton {
            enabled: false,
            name: None,
        };
        if self.description.parameters.options & game_options::TEAMS == 0 {
            return off;
        }
        let local = i32::from(self.local_u16());
        let Some(me) = self.player_by_wire_id(local) else {
            return off;
        };
        let fixed = me.flags & mandrake_net::lobby::FLAG_NO_TEAM_CHANGE != 0;
        if me.team == -1 {
            let open = chosen
                .and_then(|id| self.player_by_wire_id(id))
                .is_some_and(|p| {
                    let pid = i32::from(u16::try_from(p.net_id).unwrap_or(u16::MAX));
                    (p.team == -1 || p.team == pid) && p.team_locked == 0
                });
            return mandrake_net::lobby::TeamButton {
                enabled: open && !fixed,
                name: Some(0),
            };
        }
        mandrake_net::lobby::TeamButton {
            enabled: !fixed,
            name: Some(1 + usize::from(local != me.team)),
        }
    }

    pub fn team_kick_enabled(&self, chosen: Option<i32>) -> bool {
        let local = i32::from(self.local_u16());
        chosen.is_some_and(|id| {
            id != local && self.player_by_wire_id(id).is_some_and(|p| p.team == local)
        })
    }

    fn local_u16(&self) -> u16 {
        u16::try_from(self.local_id).unwrap_or(u16::MAX)
    }

    pub fn ready(&self) -> bool {
        self.player(self.local_id).is_some_and(|p| p.ready != 0)
    }

    pub fn listed(&self) -> Vec<PlayerData> {
        let players: Vec<mandrake_net::topology::Player> = self
            .players
            .iter()
            .map(|p| mandrake_net::topology::Player {
                id: u16::try_from(p.net_id).unwrap_or(u16::MAX),
                name: Vec::new(),
                data: p.clone(),
            })
            .collect();
        let rows = mandrake_net::lobby::list_players(&players);
        let mut pool: Vec<Option<PlayerData>> = players.into_iter().map(|p| Some(p.data)).collect();
        rows.into_iter()
            .filter_map(|row| {
                let id = i32::from(row.id);
                let mut p = pool
                    .iter_mut()
                    .find_map(|slot| {
                        slot.as_ref()
                            .is_some_and(|p| p.net_id == id)
                            .then(|| slot.take())
                            .flatten()
                    })
                    .or_else(|| self.player(id).cloned())?;
                if is_our_server(&p) {
                    return None;
                }
                let [a, b] = row.colors;
                p.appearance.colors = [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]];
                Some(p)
            })
            .collect()
    }

    pub(crate) fn set_player(&mut self, player: PlayerData) {
        match self.players.iter_mut().find(|p| p.net_id == player.net_id) {
            Some(p) => *p = player,
            None => self.players.push(player),
        }
        self.refresh();
    }

    pub(crate) fn remove_player(&mut self, id: i32) {
        self.players.retain(|p| p.net_id != id);
        self.refresh();
    }

    pub(crate) fn refresh(&mut self) {
        self.can_start = may_start(self);
        self.host_controls = may_change_options(self);
        self.description.player_count = self.players.len().min(u8::MAX as usize) as u8;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GameOptions {
    pub description: GameDescription,
    pub password: Text,
    pub mapsets: Vec<NetMapset>,
    pub cooperative_mapsets: Vec<NetMapset>,
    pub difficulties: Vec<crate::screen::Difficulty>,
    pub details: Option<FilmDetails>,
    pub saved_game: Option<NetSavedGame>,
    pub picker: Option<Box<crate::screen::SavedGames>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetSavedGame {
    pub name: Text,
    pub snapshot: Option<mandrake_rec::saved_game::Snapshot>,
    pub path: std::path::PathBuf,
}

pub fn description_from_save(description: &GameDescription, save: &FilmGame) -> GameDescription {
    let mut d = description.clone();
    let p = &mut d.parameters;
    p.mode = 1;
    p.game_type = save.game_type;
    p.options = (save.options
        & !(game_options::COOPERATIVE_TEAM | game_options::NO_AUTOMATIC_TABLE))
        | game_options::COOPERATIVE;
    p.time_limit = save.time_limit;
    p.mesh = save.mesh.0;
    p.difficulty = save.difficulty;
    p.field_16 = save.captain;
    p.planning_time = save.planning_time;
    p.flags = save.flags | 1;
    p.build = save.build_number as u16;
    p.version = save.version_id as u32;
    d.plugins.0 = save
        .plugins
        .iter()
        .map(|f| Plugin {
            checksum: f.checksum as u32,
            name: mandrake_rec::mac_roman::encode(&f.name),
            url: Vec::new(),
        })
        .collect();
    d
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetMapset {
    pub name: Text,
    pub plugin_names: Vec<String>,
    pub plugins: Vec<Plugin>,
    pub maps: Vec<NetMap>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetMap {
    pub mesh: mandrake_common::FourCC,
    pub name: Text,
    pub flags: u32,
    pub order: i16,
    pub max_teams: i16,
}

pub fn net_maps(entries: &[crate::catalog::ListEntry], mask: u32) -> Vec<NetMap> {
    let mut maps: Vec<NetMap> = entries
        .iter()
        .rev()
        .filter(|e| {
            e.flags & mask != 0
                && e.flags & crate::rules::ENTRY_LISTED != 0
                && e.flags & crate::rules::ENTRY_OPEN_HIDDEN == 0
        })
        .map(|e| NetMap {
            mesh: e.mesh,
            name: e.name.clone(),
            flags: e.flags,
            order: e.order,
            max_teams: 0,
        })
        .collect();
    maps.sort_by_key(|m| m.order);
    maps
}

pub fn net_mapsets(
    campaigns: &[crate::catalog::Campaign],
    installed: &[Plugin],
    mask: u32,
    max_teams: &[crate::catalog::MaxTeams],
) -> Vec<NetMapset> {
    let teams = |checksum: i32, mesh| {
        max_teams
            .iter()
            .find(|(key, _)| *key == (checksum, mesh))
            .map_or(0, |&(_, most)| most)
    };
    campaigns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut maps = net_maps(&c.entries, mask);
            for m in &mut maps {
                m.max_teams = teams(c.checksum, m.mesh);
            }
            (i, c, maps)
        })
        .filter(|(i, _, maps)| *i == 0 || !maps.is_empty())
        .map(|(_, c, maps)| NetMapset {
            name: c.name.clone(),
            plugin_names: c.plugins.clone(),
            plugins: c
                .plugins
                .iter()
                .filter_map(|name| {
                    installed
                        .iter()
                        .find(|p| until_nul(&p.name) == name.as_bytes())
                        .cloned()
                })
                .collect(),
            maps,
        })
        .collect()
}

pub fn choose_map(description: &mut GameDescription, mapset: &NetMapset, map: &NetMap) {
    description.parameters.mesh = map.mesh.0;
    description.map_name = map.name.0.clone();
    description.plugins =
        PluginList::for_entry_point(Some(GAMEPLAY_PREFERENCE.saturating_abs()), &mapset.plugins);
    description.parameters.plugin_count = description.plugins.0.len() as u16;
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetPostgame {
    pub report: PostgameReport,
    pub can_restart: bool,
    pub can_chat: bool,
}

pub fn is_our_server(p: &PlayerData) -> bool {
    p.net_id == HOST_ID && p.field_22 == mandrake_net::ours::SERVER_MARK
}

pub fn may_start(lobby: &Lobby) -> bool {
    if GameKind::of(&lobby.description) == GameKind::Tfl {
        return may_change_options(lobby);
    }
    may_change_options(lobby)
        && lobby
            .players
            .iter()
            .filter(|p| p.net_id != lobby.local_id && !(lobby.owner && p.net_id == HOST_ID))
            .all(|p| p.ready != 0)
}

pub fn may_change_options(lobby: &Lobby) -> bool {
    lobby.host || lobby.owner
}

pub fn may_kick(lobby: &Lobby, id: i32) -> bool {
    may_change_options(lobby)
        && id != lobby.local_id
        && !(lobby.owner && id == HOST_ID)
        && lobby.player(id).is_some()
}

pub fn description_valid(description: &GameDescription) -> bool {
    VERSIONS.contains(&description.parameters.version)
}

fn built_in(plugin: &Plugin) -> bool {
    GAMEPLAY_VERSION_PLUGINS.iter().any(|&(checksum, v)| {
        checksum == plugin.checksum
            && gameplay_version_plugin(v).is_some_and(|g| g.name.eq_ignore_ascii_case(&plugin.name))
    })
}

pub fn missing_plugins(description: &GameDescription, installed: &[Plugin]) -> Vec<Plugin> {
    description
        .plugins
        .0
        .iter()
        .filter(|p| !built_in(p))
        .filter(|p| {
            !installed
                .iter()
                .any(|i| i.checksum == p.checksum && i.name.eq_ignore_ascii_case(&p.name))
        })
        .cloned()
        .collect()
}

pub fn other_game_text(theirs: GameKind) -> Text {
    Text(
        format!(
            "This is a {} game: start the client with --game {}.\r",
            theirs.name(),
            theirs.switch()
        )
        .into_bytes(),
    )
}

pub fn tfl_data_text(mismatch: mandrake_net::tfl::DataMismatch) -> Text {
    use mandrake_net::tfl::DataMismatch;
    Text(
        match mismatch {
            DataMismatch::Plugin { host, ours } => format!(
                "The host's TFL plays another plugin (checksum {host:08x}, ours {ours:08x}).\r"
            ),
            DataMismatch::Archive { host, ours } => format!(
                "The host's TFL data differs from ours (checksum {host:08x}, ours {ours:08x}).\r"
            ),
        }
        .into_bytes(),
    )
}

pub fn invalid_game_text(description: &GameDescription, installed: &[Plugin]) -> Text {
    if !description_valid(description) {
        let v = description.parameters.version;
        let name = i16::try_from(v)
            .ok()
            .and_then(gameplay_version_plugin)
            .map_or_else(
                || v.to_string(),
                |p| String::from_utf8_lossy(&p.name).into_owned(),
            );
        return Text(
            format!("Game versions don't match. The game is being played using {name}.\r")
                .into_bytes(),
        );
    }
    let mut out = Vec::new();
    for p in description.plugins.0.iter().filter(|p| !built_in(p)) {
        let have = installed
            .iter()
            .any(|i| i.checksum == p.checksum && i.name.eq_ignore_ascii_case(&p.name));
        if have {
            continue;
        }
        let url = until_nul(&p.url);
        out.extend_from_slice(until_nul(&p.name));
        out.extend_from_slice(b" - ");
        if !url.starts_with(b"http://") {
            out.extend_from_slice(b"http://");
        }
        out.extend_from_slice(url);
        out.push(b'\r');
    }
    Text(out)
}

const MYTH2_SOLO_GAME_TYPE: i16 = 13;

pub fn film_game(description: &GameDescription) -> FilmGame {
    let p = &description.parameters;
    let version_id = p.version as i32;
    let build_number = p.build as i16;
    let options = match GameKind::of(description) {
        GameKind::Tfl => {
            (p.options | game_options::ALLIANCES | game_options::SHOW_ENEMY)
                & !(game_options::HOST_OBSERVER | game_options::DEATHMATCH)
        }
        GameKind::Myth2 => p.options,
    };
    let game_type = match GameKind::of(description) {
        GameKind::Tfl if p.options & game_options::COOPERATIVE != 0 => MYTH2_SOLO_GAME_TYPE,
        _ => p.game_type,
    };
    FilmGame {
        mode: p.mode,
        game_type,
        options,
        time_limit: p.time_limit,
        mesh: mandrake_common::FourCC(p.mesh),
        difficulty: p.difficulty,
        captain: p.field_16,
        planning_time: p.planning_time,
        flags: p.flags,
        build_number,
        version_id,
        plugins: description
            .plugins
            .0
            .iter()
            .filter(|p| !built_in(p))
            .map(|p| FilmPlugin {
                name: mandrake_rec::mac_roman::decode(until_nul(&p.name)),
                checksum: p.checksum as i32,
            })
            .collect(),
        gameplay_version: crate::films::compute_gameplay_version(p.options, version_id, -1),
        version_of_id: crate::films::gameplay_version_from_version_id(version_id, build_number),
        ..FilmGame::default()
    }
}

pub fn game_plugins(description: &GameDescription) -> Vec<String> {
    description
        .plugins
        .0
        .iter()
        .filter(|p| !built_in(p))
        .map(|p| String::from_utf8_lossy(until_nul(&p.name)).into_owned())
        .collect()
}

pub fn tfl_host_description(
    game_name: &Text,
    password: &Text,
    checksums: DataChecksums,
) -> GameDescription {
    let mut parameters =
        GameParameters::read(&[0; mandrake_net::game_description::PARAMETERS_SIZE]);
    parameters.mode = NET_MODE;
    parameters.game_type = 0;
    parameters.options = 0xe024;
    parameters.time_limit = DEFAULT_TIME_LIMIT;
    parameters.mesh = *b"58cm";
    parameters.difficulty = DEFAULT_DIFFICULTY;
    parameters.max_players = mandrake_net::game_description::MAX_PLAYERS as i16;
    parameters.field_16 = -1;
    mandrake_net::tfl::description(
        &parameters,
        checksums,
        &game_name.0,
        b"For Carnage Apply Within",
        !password.0.is_empty(),
    )
}

pub fn host_description(
    game_name: &Text,
    password: &Text,
    foundation_checksum: u32,
) -> GameDescription {
    let mut parameters =
        GameParameters::read(&[0; mandrake_net::game_description::PARAMETERS_SIZE]);
    parameters.mode = NET_MODE;
    parameters.game_type = DEFAULT_GAME_TYPE;
    parameters.options = DEFAULT_OPTIONS;
    parameters.time_limit = DEFAULT_TIME_LIMIT;
    parameters.mesh = DEFAULT_MESH;
    parameters.difficulty = DEFAULT_DIFFICULTY;
    parameters.field_16 = -1;
    parameters.max_players = mandrake_net::game_description::MAX_PLAYERS as i16;
    parameters.version = u32::from(GAMEPLAY_VERSION);
    parameters.build = BUILD_NUMBER;
    let mut plugins = PluginList::default();
    plugins.add_gameplay_version(&parameters, GAMEPLAY_PREFERENCE);
    parameters.plugin_count = plugins.0.len() as u16;
    GameDescription {
        parameters,
        plugins,
        foundation_checksum,
        field_230: 0,
        flags: if password.0.is_empty() {
            0
        } else {
            mandrake_net::game_description::FLAG_PASSWORD
        },
        player_count: 1,
        game_name: game_name.0.clone(),
        map_name: Vec::new(),
    }
}

impl NetEvent {
    pub fn from_endpoint(event: &mandrake_net::endpoint::Event) -> Option<NetEvent> {
        use mandrake_net::endpoint::Event;
        Some(match event {
            Event::Joined { id } => NetEvent::Joined { id: i32::from(*id) },
            Event::Denied { reason } => NetEvent::Denied(Text(reason.clone())),
            Event::JoinTimedOut => NetEvent::TimedOut,
            Event::Player { player, .. } => {
                let mut data = player.data.clone();
                data.net_id = i32::from(player.id);
                NetEvent::Player(Box::new(data))
            }
            Event::PlayerLeft { id } => NetEvent::PlayerLeft { id: i32::from(*id) },
            Event::Description(d) => NetEvent::Description(Box::new(d.clone())),
            Event::Configuration { flags: 0, code, .. } => NetEvent::Configuration(*code),
            Event::BeginMidgame { start } => NetEvent::Midgame(start.clone()),
            Event::Chat { chat, .. } => NetEvent::Chat {
                name: Text(until_nul(&chat.name).to_vec()),
                text: Text(chat.text.clone()),
                kind: ChatKind::of_mode(chat.words[5]),
            },
            Event::Latencies(latencies) => NetEvent::Latencies(
                latencies
                    .iter()
                    .map(|&(id, ms)| (i32::from(id), ms))
                    .collect(),
            ),
            Event::BeginSync { .. } => NetEvent::BeginSync,
            Event::AllSynced { .. } => NetEvent::AllSynced,
            Event::HostLost => NetEvent::HostLost,
            Event::Owner => NetEvent::Owner,
            Event::Downloads(url) => NetEvent::DownloadsFrom(Text(url.clone())),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod event_tests {
    use mandrake_net::endpoint::Event;
    use mandrake_net::lobby::code;

    use super::*;

    #[test]
    fn the_session_gets_the_lobby_events_and_not_the_commands() {
        let of = NetEvent::from_endpoint;
        assert_eq!(
            of(&Event::Joined { id: 1 }),
            Some(NetEvent::Joined { id: 1 })
        );
        assert_eq!(
            of(&Event::Configuration {
                flags: 0,
                code: code::START,
                value: 0
            }),
            Some(NetEvent::Configuration(100))
        );
        assert_eq!(
            of(&Event::Configuration {
                flags: 1,
                code: code::START,
                value: 0
            }),
            None,
            "a request, not the host's broadcast"
        );
        assert_eq!(of(&Event::HostLost), Some(NetEvent::HostLost));
        assert_eq!(
            of(&Event::AllSynced { barrier: 2 }),
            Some(NetEvent::AllSynced)
        );
        assert_eq!(
            of(&Event::Commands {
                commands: Vec::new(),
                horizon: 9
            }),
            None
        );
        let player = mandrake_net::topology::Player {
            id: 3,
            name: b"Ours".to_vec(),
            data: PlayerData::new(b"Ours", b"Team", [0; 8]),
        };
        let Some(NetEvent::Player(data)) = of(&Event::Player { player, new: true }) else {
            panic!("a player");
        };
        assert_eq!(data.net_id, 3);
    }
}

#[cfg(test)]
mod tfl_lobby_tests {
    use super::*;

    #[test]
    fn a_tfl_host_starts_without_ready() {
        let player = |id: i32| {
            let mut p = PlayerData::new(b"P", b"T", [0; 8]);
            p.net_id = id;
            p.ready = 0;
            p
        };
        let mut lobby = Lobby {
            host: true,
            owner: false,
            local_id: HOST_ID,
            players: vec![player(HOST_ID), player(1)],
            description: host_description(&Text(b"Game".to_vec()), &Text(Vec::new()), 0),
            chat: BackLog::default(),
            locked: false,
            can_start: false,
            host_controls: false,
            details: None,
        };
        assert!(!may_start(&lobby));
        lobby.description =
            tfl_host_description(&Text(b"Game".to_vec()), &Text(Vec::new()), [0; 2]);
        assert!(may_start(&lobby));
        lobby.host = false;
        assert!(!may_start(&lobby), "only the host");
    }

    #[test]
    fn tfls_game_control_has_no_myth_ii_lines() {
        let mut d = tfl_host_description(&Text(b"Game".to_vec()), &Text(Vec::new()), [0; 2]);
        d.parameters.options = 0x20;
        let g = film_game(&d);
        assert_eq!(
            g.options,
            0x20 | game_options::ALLIANCES | game_options::SHOW_ENEMY
        );
        assert_eq!(g.game_type, 0);
        d.parameters.options = 0x80;
        d.parameters.game_type = 11;
        assert_eq!(film_game(&d).game_type, MYTH2_SOLO_GAME_TYPE);
    }

    #[test]
    fn plugin_download_fraction_handles_large_totals_without_overflow() {
        let dl = |received, total| PluginDownload {
            plugins: Vec::new(),
            progress: DownloadProgress {
                index: 0,
                received,
                total,
            },
        };
        assert_eq!(dl(0, 0).fraction(), 0);
        assert_eq!(dl(u64::MAX / 2, u64::MAX).fraction(), 127);
        assert_eq!(dl(u64::MAX, u64::MAX).fraction(), 256);
        assert_eq!(dl(u64::MAX, u64::MAX - 1).fraction(), 256);
    }

    #[test]
    fn lobby_team_helpers_match_mandrake_net() {
        let mut host = PlayerData::new(b"Host", b"H", [0; 8]);
        host.net_id = 0;
        host.team = 0;
        let mut member = PlayerData::new(b"Member", b"H", [0; 8]);
        member.net_id = 1;
        member.team = 0;
        let mut solo = PlayerData::new(b"Solo", b"S", [0; 8]);
        solo.net_id = 2;
        solo.team = -1;
        let mut d = host_description(&Text(b"G".to_vec()), &Text::default(), 0);
        d.parameters.options |= game_options::TEAMS;
        for local_id in [0, 1, 2] {
            let lobby = Lobby {
                host: local_id == 0,
                owner: false,
                local_id,
                players: vec![host.clone(), member.clone(), solo.clone()],
                description: d.clone(),
                chat: BackLog::default(),
                locked: false,
                can_start: false,
                host_controls: false,
                details: None,
            };
            let net_players = lobby.net_players();
            for chosen in [None, Some(0), Some(1), Some(2)] {
                assert_eq!(
                    lobby.team_button(chosen),
                    mandrake_net::lobby::team_button(&net_players, local_id as u16, chosen, true)
                );
                assert_eq!(
                    lobby.team_kick_enabled(chosen),
                    mandrake_net::lobby::team_kick_enabled(&net_players, local_id as u16, chosen)
                );
            }
        }
    }
}
