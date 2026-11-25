use mandrake_common::game_options;
use mandrake_rec::writer::{NEW_GAME_DATA_SIZE, NewGameData};

use crate::player_data::{NONE, PLAYER_DATA_SIZE, PlayerData, fixed, sort_players, until_nul};
use crate::wire::{Body, WireError, kind};

pub const PARAMETERS_SIZE: usize = 0x2c;
pub const PLUGIN_LIST_SIZE: usize = 0x200;
pub const FIXED_SIZE: usize = 0x23c;
pub const GAME_NAME_MAX: usize = 0x20;
pub const MAP_NAME_MAX: usize = 0x40;
pub const MAX_PLUGINS: usize = 16;
pub const MAX_PLAYERS: usize = 16;
const PLUGIN_NAME_MAX: usize = 0x1f;
const PLUGIN_URL_MAX: usize = 0x3f;

pub use mandrake_common::game_options::COOPERATIVE as OPTION_COOPERATIVE;
pub(crate) const PLAYER_COUNT: usize = 0x22c;
pub(crate) const LOCAL_PLAYER: usize = 0x22e;
pub(crate) const PLAYERS: usize = 0x230;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameParameters {
    pub mode: i16,
    pub game_type: i16,
    pub options: u32,
    pub time_limit: i32,
    pub mesh: [u8; 4],
    pub difficulty: i16,
    pub max_players: i16,
    pub seed_low: u16,
    pub field_16: i16,
    pub seed: u32,
    pub planning_time: i32,
    pub flags: u16,
    pub build: u16,
    pub version: u32,
    pub field_28: u16,
    pub plugin_count: u16,
}

use crate::wire::{be16, be32};

fn bad(reason: &'static str) -> WireError {
    WireError::BadBody {
        kind: kind::GAME_DESCRIPTION,
        reason,
    }
}

impl GameParameters {
    pub fn read(b: &[u8; PARAMETERS_SIZE]) -> GameParameters {
        GameParameters {
            mode: be16(b, 0x00) as i16,
            game_type: be16(b, 0x02) as i16,
            options: be32(b, 0x04),
            time_limit: be32(b, 0x08) as i32,
            mesh: b[0x0c..0x10].try_into().unwrap(),
            difficulty: be16(b, 0x10) as i16,
            max_players: be16(b, 0x12) as i16,
            seed_low: be16(b, 0x14),
            field_16: be16(b, 0x16) as i16,
            seed: be32(b, 0x18),
            planning_time: be32(b, 0x1c) as i32,
            flags: be16(b, 0x20),
            build: be16(b, 0x22),
            version: be32(b, 0x24),
            field_28: be16(b, 0x28),
            plugin_count: be16(b, 0x2a),
        }
    }

    pub fn to_bytes(&self) -> [u8; PARAMETERS_SIZE] {
        let mut b = [0; PARAMETERS_SIZE];
        b[0x00..0x02].copy_from_slice(&self.mode.to_be_bytes());
        b[0x02..0x04].copy_from_slice(&self.game_type.to_be_bytes());
        b[0x04..0x08].copy_from_slice(&self.options.to_be_bytes());
        b[0x08..0x0c].copy_from_slice(&self.time_limit.to_be_bytes());
        b[0x0c..0x10].copy_from_slice(&self.mesh);
        b[0x10..0x12].copy_from_slice(&self.difficulty.to_be_bytes());
        b[0x12..0x14].copy_from_slice(&self.max_players.to_be_bytes());
        b[0x14..0x16].copy_from_slice(&self.seed_low.to_be_bytes());
        b[0x16..0x18].copy_from_slice(&self.field_16.to_be_bytes());
        b[0x18..0x1c].copy_from_slice(&self.seed.to_be_bytes());
        b[0x1c..0x20].copy_from_slice(&self.planning_time.to_be_bytes());
        b[0x20..0x22].copy_from_slice(&self.flags.to_be_bytes());
        b[0x22..0x24].copy_from_slice(&self.build.to_be_bytes());
        b[0x24..0x28].copy_from_slice(&self.version.to_be_bytes());
        b[0x28..0x2a].copy_from_slice(&self.field_28.to_be_bytes());
        b[0x2a..0x2c].copy_from_slice(&self.plugin_count.to_be_bytes());
        b
    }

    pub fn reset_seeds(&mut self, machine_ticks: u32) {
        self.seed = machine_ticks;
        self.seed_low = machine_ticks as u16;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    pub checksum: u32,
    pub name: Vec<u8>,
    pub url: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginList(pub Vec<Plugin>);

pub const GAMEPLAY_VERSION_PLUGINS: [(u32, i16); 13] = [
    (0x8a13_e09d, 1800),
    (0x4b4b_d40a, 1801),
    (0xf09a_db5b, 1805),
    (0x9b38_4660, 1810),
    (0x2e2b_9cb7, 1811),
    (0x7b2f_fe3e, 1820),
    (0x3e9e_0261, 1821),
    (0x1bc7_aaf1, 1822),
    (0xddc1_f2d4, 1830),
    (0x5738_53ba, 1840),
    (0xc9d8_bf4a, 1841),
    (0x526c_3e68, 1850),
    (0x5982_c2c9, 1852),
];
pub const GAMEPLAY_PLUGIN_URL: &[u8] = b"http://www.projectmagma.net";
const VERSION_FLAG_0: i16 = 0x73a;
const VERSION_TYPE_F: i16 = 0x70d;
const GAME_TYPE_F: i16 = 0xf;

pub fn gameplay_version_plugin(version: i16) -> Option<Plugin> {
    let &(checksum, v) = GAMEPLAY_VERSION_PLUGINS
        .iter()
        .find(|&&(_, v)| v == version)?;
    let name = format!("Myth II {}.{}.{}", v / 1000, v % 1000 / 100, v % 100 / 10);
    Some(Plugin {
        checksum,
        name: name.into_bytes(),
        url: GAMEPLAY_PLUGIN_URL.to_vec(),
    })
}

fn is_gameplay_version_plugin(p: &Plugin) -> bool {
    GAMEPLAY_VERSION_PLUGINS.iter().any(|&(checksum, v)| {
        checksum == p.checksum
            && gameplay_version_plugin(v).is_some_and(|g| g.name.eq_ignore_ascii_case(&p.name))
    })
}

impl PluginList {
    pub fn read(b: &[u8; PLUGIN_LIST_SIZE]) -> Result<PluginList, WireError> {
        let count = be16(b, 0) as usize;
        if count > MAX_PLUGINS {
            return Err(bad("more than 16 plugins"));
        }
        let mut at = 2;
        let mut out = Vec::with_capacity(count);
        let cstr = |at: &mut usize, max: usize| -> Result<Vec<u8>, WireError> {
            let rest = &b[*at..];
            let n = rest
                .iter()
                .position(|&c| c == 0)
                .ok_or_else(|| bad("plugin string without its terminator"))?;
            if n > max {
                return Err(bad("plugin string too long"));
            }
            *at += n + 1;
            Ok(rest[..n].to_vec())
        };
        for _ in 0..count {
            if at + 4 > PLUGIN_LIST_SIZE {
                return Err(bad("plugin list overruns"));
            }
            let checksum = be32(b, at);
            at += 4;
            let name = cstr(&mut at, PLUGIN_NAME_MAX)?;
            let url = cstr(&mut at, PLUGIN_URL_MAX)?;
            out.push(Plugin {
                checksum,
                name,
                url,
            });
        }
        Ok(PluginList(out))
    }

    pub fn to_bytes(&self) -> [u8; PLUGIN_LIST_SIZE] {
        let mut b = [0; PLUGIN_LIST_SIZE];
        b[0..2].copy_from_slice(&(self.0.len() as u16).to_be_bytes());
        let mut at = 2;
        for p in &self.0 {
            let name = &p.name[..p.name.len().min(PLUGIN_NAME_MAX)];
            let url = &p.url[..p.url.len().min(PLUGIN_URL_MAX)];
            let size = 4 + name.len() + 1 + url.len() + 1;
            if at + size > PLUGIN_LIST_SIZE {
                break;
            }
            b[at..at + 4].copy_from_slice(&p.checksum.to_be_bytes());
            b[at + 4..at + 4 + name.len()].copy_from_slice(name);
            let u = at + 4 + name.len() + 1;
            b[u..u + url.len()].copy_from_slice(url);
            at += size;
        }
        b
    }

    pub fn remove_gameplay_versions(&mut self) {
        self.0.retain(|p| !is_gameplay_version_plugin(p));
    }

    pub fn for_entry_point(gameplay_version: Option<i16>, plugins: &[Plugin]) -> PluginList {
        let mut list: Vec<Plugin> = gameplay_version
            .and_then(gameplay_version_plugin)
            .into_iter()
            .collect();
        for p in plugins {
            if !list.contains(p) {
                list.push(p.clone());
            }
        }
        PluginList(list)
    }

    pub fn add_gameplay_version(&mut self, params: &GameParameters, preference: i16) {
        let base = if params.flags & 1 != 0 {
            VERSION_FLAG_0
        } else if params.game_type == GAME_TYPE_F {
            VERSION_TYPE_F
        } else {
            -1
        };
        let version = if preference < -1 {
            base.max(preference.saturating_abs())
        } else {
            base
        };
        self.remove_gameplay_versions();
        if version != -1
            && let Some(p) = gameplay_version_plugin(version)
        {
            self.0.push(p);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDescription {
    pub parameters: GameParameters,
    pub plugins: PluginList,
    pub foundation_checksum: u32,
    pub field_230: u32,
    pub flags: u16,
    pub player_count: u8,
    pub game_name: Vec<u8>,
    pub map_name: Vec<u8>,
}

pub const FLAG_PASSWORD: u16 = 2;

impl GameDescription {
    pub fn read(b: &[u8]) -> Result<GameDescription, WireError> {
        if b.len() < FIXED_SIZE {
            return Err(bad("game description too short"));
        }
        let mut strings = b[FIXED_SIZE..].split(|&c| c == 0);
        let game_name = strings.next().unwrap_or_default().to_vec();
        let map_name = strings.next().unwrap_or_default().to_vec();
        if FIXED_SIZE + game_name.len() + map_name.len() + 2 != b.len() {
            return Err(bad("game description's names don't end the body"));
        }
        Ok(GameDescription {
            parameters: GameParameters::read(b[..PARAMETERS_SIZE].try_into().unwrap()),
            plugins: PluginList::read(b[PARAMETERS_SIZE..0x22c].try_into().unwrap())?,
            foundation_checksum: be32(b, 0x22c),
            field_230: be32(b, 0x230),
            flags: be16(b, 0x238),
            player_count: b[0x23a],
            game_name,
            map_name,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![0; FIXED_SIZE];
        b[..PARAMETERS_SIZE].copy_from_slice(&self.parameters.to_bytes());
        b[PARAMETERS_SIZE..0x22c].copy_from_slice(&self.plugins.to_bytes());
        b[0x22c..0x230].copy_from_slice(&self.foundation_checksum.to_be_bytes());
        b[0x230..0x234].copy_from_slice(&self.field_230.to_be_bytes());
        b[0x238..0x23a].copy_from_slice(&self.flags.to_be_bytes());
        b[0x23a] = self.player_count;
        let game = until_nul(&self.game_name);
        b.extend_from_slice(&game[..game.len().min(GAME_NAME_MAX - 1)]);
        b.push(0);
        let map = until_nul(&self.map_name);
        b.extend_from_slice(&map[..map.len().min(MAP_NAME_MAX - 1)]);
        b.push(0);
        b
    }

    pub fn from_body(body: &Body) -> Result<GameDescription, WireError> {
        match body {
            Body::GameDescription(raw) => GameDescription::read(raw),
            _ => Err(bad("not a game description")),
        }
    }

    pub fn to_body(&self) -> Body {
        Body::GameDescription(self.to_bytes())
    }

    pub fn new_game_head(&self) -> [u8; PARAMETERS_SIZE + PLUGIN_LIST_SIZE] {
        let mut b = [0; PARAMETERS_SIZE + PLUGIN_LIST_SIZE];
        b[..PARAMETERS_SIZE].copy_from_slice(&self.parameters.to_bytes());
        b[PARAMETERS_SIZE..].copy_from_slice(&self.plugins.to_bytes());
        b
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewGameError {
    PlayerCount(usize),
}

pub fn new_game_players(d: &NewGameData) -> (u16, &[u8]) {
    let raw = &d.0;
    let Some(count) = raw.get(PLAYER_COUNT..PLAYER_COUNT + 2) else {
        return (0, &[]);
    };
    let count = u16::from_be_bytes([count[0], count[1]]).min(MAX_PLAYERS as u16);
    let end = (PLAYERS + usize::from(count) * PLAYER_DATA_SIZE).min(raw.len());
    (count, raw.get(PLAYERS..end).unwrap_or(&[]))
}

pub fn midgame_new_game_data(
    description: &GameDescription,
    start: &[u8],
    lobby: &[PlayerData],
) -> NewGameData {
    let mut d = vec![0u8; NEW_GAME_DATA_SIZE];
    d[..0x22c].copy_from_slice(&description.new_game_head());
    let mut params = description.parameters.clone();
    if params.options & game_options::COOPERATIVE != 0 {
        let options = params.options;
        let leader = if params.version > u32::from(game_options::VERSION_ID_1_8_1) {
            -((options & game_options::HOST_OBSERVER == 0) as i32)
        } else {
            -1
        };
        if options & game_options::TEAMS != 0
            && lobby.iter().any(|p| p.team != NONE && p.team != leader)
        {
            params.options |= game_options::COOPERATIVE_TEAM;
        }
        params.field_16 = -1;
        params.time_limit = 0;
        params.options |= game_options::VETERANS;
    }
    let records = start.as_chunks::<PLAYER_DATA_SIZE>().0;
    let records = &records[..records.len().min(MAX_PLAYERS)];
    d[..PARAMETERS_SIZE].copy_from_slice(&params.to_bytes());
    d[PLAYER_COUNT..PLAYER_COUNT + 2].copy_from_slice(&(records.len() as u16).to_be_bytes());
    d[LOCAL_PLAYER..LOCAL_PLAYER + 2].copy_from_slice(&0xffffu16.to_be_bytes());
    for (i, r) in records.iter().enumerate() {
        let at = PLAYERS + i * PLAYER_DATA_SIZE;
        d[at..at + PLAYER_DATA_SIZE].copy_from_slice(r);
    }
    NewGameData(d)
}

pub fn new_game_data(
    description: &GameDescription,
    players: &[PlayerData],
    local_net_id: i32,
) -> Result<NewGameData, NewGameError> {
    if players.is_empty() || players.len() > MAX_PLAYERS {
        return Err(NewGameError::PlayerCount(players.len()));
    }
    let mut d = vec![0u8; NEW_GAME_DATA_SIZE];
    d[..0x22c].copy_from_slice(&description.new_game_head());
    let mut params = description.parameters.clone();
    let mut team: Option<i32> = None;
    if params.options & game_options::COOPERATIVE != 0 {
        let options = params.options;
        let (leader, mut t) = if params.version < u32::from(game_options::VERSION_ID_1_8_2) {
            (-1, 0)
        } else {
            (
                -((options & game_options::HOST_OBSERVER == 0) as i32),
                -((options & game_options::TEAMS != 0) as i32),
            )
        };
        if options & game_options::TEAMS != 0 {
            for p in players {
                if p.team != NONE && p.team != leader {
                    params.options |= game_options::COOPERATIVE_TEAM;
                    t = p.team as i16 as i32;
                }
            }
        }
        params.time_limit = 0;
        params.field_16 = -1;
        team = (t != -1).then_some(t);
    }
    let mut sorted = players.to_vec();
    sort_players(&mut sorted);
    let mut local = None;
    for (i, p) in sorted.iter_mut().enumerate() {
        p.field_00 = -1;
        if params.options & game_options::COOPERATIVE != 0 {
            p.field_00 = 0;
            if let Some(t) = team {
                p.team = t;
            }
        }
        p.field_02 = 0;
        if p.net_id == local_net_id {
            local = Some(i);
        }
    }
    if params.options & game_options::COOPERATIVE != 0 {
        params.options |= game_options::VETERANS;
        for (field_00, name, colors) in [
            (1i16, &b"Dark"[..], [0, 0, 0xffff, 0, 0, 0xffff, 0xffff, 0]),
            (2, b"Neutral", [0x4000, 0, 0, 0, 0x4000, 0, 0, 0]),
        ] {
            if sorted.len() == MAX_PLAYERS {
                break;
            }
            let mut ai = PlayerData::read(&[0; PLAYER_DATA_SIZE]).unwrap();
            ai.field_00 = field_00;
            ai.field_02 = 1;
            ai.team = NONE;
            ai.net_id = NONE;
            ai.metaserver_id = NONE;
            ai.appearance.name = fixed(name);
            ai.appearance.colors = colors;
            sorted.push(ai);
        }
    }
    let v1_5_1 = u32::from(game_options::VERSION_ID_1_5_1);
    if params.version == v1_5_1 || params.options & game_options::GAMEPLAY_1_5_1_GAME != 0 {
        params.version = v1_5_1;
        params.options |= game_options::GAMEPLAY_1_5_1_GAME;
    }
    d[..PARAMETERS_SIZE].copy_from_slice(&params.to_bytes());
    d[PLAYER_COUNT..PLAYER_COUNT + 2].copy_from_slice(&(sorted.len() as u16).to_be_bytes());
    if let Some(i) = local {
        d[LOCAL_PLAYER..LOCAL_PLAYER + 2].copy_from_slice(&(i as u16).to_be_bytes());
    }
    for (i, p) in sorted.iter().enumerate() {
        let at = PLAYERS + i * PLAYER_DATA_SIZE;
        d[at..at + PLAYER_DATA_SIZE].copy_from_slice(&p.to_bytes());
    }
    Ok(NewGameData(d))
}
