use std::borrow::Cow;

use mandrake_common::FourCC;
use mandrake_common::game_options::VERSION_ID_1_8_5;

use crate::binary::{
    be_u16, flush_command_block, push_command_block, put_u16, put_u32, write_command_block,
};
use crate::command::{CommandPacket, CommandPayload};
use crate::header::{MonsterInitializer, PluginInfo};

pub const HEADER_SIZE: usize = 0xa28;
pub const NEW_GAME_DATA_OFFSET: usize = 0x24;
pub const NEW_GAME_DATA_SIZE: usize = 0x9fc;
pub const TAG_HEADER_SIZE: usize = 0x40;
pub const MAX_BLOCK_BYTES: usize = 0x800;
pub const RECORDING_VERSION: u16 = 3;
pub const RECORDING_GROUP: FourCC = FourCC(*b"reco");
pub const SAVE_GROUP: FourCC = FourCC(*b"save");
pub const SAVED_GAME_SUB_VERSION: u16 = 1;
pub const FILM_EXTENSION: &str = ".m2rec";

const NEW_GAME_FLAGS: usize = 0x20;
const PLAYER_COUNT: usize = 0x22c;
const PLAYERS: usize = 0x230;
const PLAYER_SIZE: usize = 0x7c;
const MAX_PLAYERS: usize = 16;
const PLUGIN_COUNT: usize = 0x2a;
const PLUGIN_LIST: usize = 0x2c;
const PLUGIN_LIST_SIZE: usize = 0x200;
const PLUGIN_NAME_MAX: usize = 0x1f;
const PLUGIN_URL_MAX: usize = 0x3f;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGameData(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoloNewGame {
    pub mesh: FourCC,
    pub difficulty: i16,
    pub seed: u32,
    pub options: u32,
    pub plugins: Vec<PluginInfo>,
}

struct SoloPlayer {
    team: i16,
    field_02: i16,
    id: i32,
    version: Option<(u32, u16)>,
    name: &'static [u8],
    colors: [[u16; 4]; 2],
}

const SOLO_MODE: i16 = 2;
const SOLO_GAME_TYPE: i16 = 13;
const SOLO_FIELD_12: i16 = 0x10;
const SOLO_FIELD_16: i16 = -1;
const SOLO_BUILD_NUMBER: u16 = 471;
const GAMEPLAY_VERSION_ID: u32 = VERSION_ID_1_8_5 as u32;

const SOLO_PLAYERS: [SoloPlayer; 3] = [
    SoloPlayer {
        team: 0,
        field_02: 0,
        id: 0,
        version: Some((GAMEPLAY_VERSION_ID, SOLO_BUILD_NUMBER)),
        name: b"Light",
        colors: [[0xffff, 0xffff, 0xffff, 0], [0xffff, 0xffff, 0xffff, 0]],
    },
    SoloPlayer {
        team: 1,
        field_02: 1,
        id: -1,
        version: None,
        name: b"Dark",
        colors: [[0x4000, 0x4000, 0x4000, 0], [0x4000, 0x4000, 0x4000, 0]],
    },
    SoloPlayer {
        team: 2,
        field_02: 1,
        id: -1,
        version: None,
        name: b"Neutral",
        colors: [[0x4000, 0, 0, 0], [0x4000, 0, 0, 0]],
    },
];

impl NewGameData {
    pub fn from_header(header: &[u8]) -> Option<Self> {
        header
            .get(NEW_GAME_DATA_OFFSET..NEW_GAME_DATA_OFFSET + NEW_GAME_DATA_SIZE)
            .map(|b| Self(b.to_vec()))
    }

    pub fn solo(game: &SoloNewGame) -> Self {
        let mut d = vec![0u8; NEW_GAME_DATA_SIZE];
        put_u16(&mut d, 0x00, SOLO_MODE as u16);
        put_u16(&mut d, 0x02, SOLO_GAME_TYPE as u16);
        put_u32(&mut d, 0x04, game.options);
        d[0x0c..0x10].copy_from_slice(&game.mesh.0);
        put_u16(&mut d, 0x10, game.difficulty as u16);
        put_u16(&mut d, 0x12, SOLO_FIELD_12 as u16);
        put_u16(&mut d, 0x14, game.seed as u16);
        put_u16(&mut d, 0x16, SOLO_FIELD_16 as u16);
        put_u32(&mut d, 0x18, game.seed);
        put_u16(&mut d, 0x22, SOLO_BUILD_NUMBER);
        put_u32(&mut d, 0x24, GAMEPLAY_VERSION_ID);
        if !game.plugins.is_empty() {
            let (written, list) = encode_plugins(&game.plugins);
            put_u16(&mut d, PLUGIN_COUNT, written);
            d[PLUGIN_LIST..PLUGIN_LIST + PLUGIN_LIST_SIZE].copy_from_slice(&list);
        }
        put_u16(&mut d, PLAYER_COUNT, SOLO_PLAYERS.len() as u16);
        for (i, p) in SOLO_PLAYERS.iter().enumerate() {
            let e = PLAYERS + i * PLAYER_SIZE;
            put_u16(&mut d, e, p.team as u16);
            put_u16(&mut d, e + 2, p.field_02 as u16);
            put_u32(&mut d, e + 8, u32::MAX);
            put_u32(&mut d, e + 0x0c, p.id as u32);
            if let Some((version, build)) = p.version {
                put_u32(&mut d, e + 0x1c, version);
                put_u16(&mut d, e + 0x20, build);
            }
            d[e + 0x2c..e + 0x2c + p.name.len()].copy_from_slice(p.name);
            for (k, color) in p.colors.iter().enumerate() {
                for (j, &c) in color.iter().enumerate() {
                    put_u16(&mut d, e + 0x6c + k * 8 + j * 2, c);
                }
            }
        }
        Self(d)
    }

    pub fn seed(&self) -> u32 {
        u32::from_be_bytes([self.0[0x18], self.0[0x19], self.0[0x1a], self.0[0x1b]])
    }
}

fn encode_plugins(plugins: &[PluginInfo]) -> (u16, [u8; PLUGIN_LIST_SIZE]) {
    let mut list = [0u8; PLUGIN_LIST_SIZE];
    let mut at = 2;
    let mut written = 0u16;
    for p in plugins {
        let (name, url) = (
            crate::mac_roman::encode(&p.name),
            crate::mac_roman::encode(&p.url),
        );
        let name = &name[..name.len().min(PLUGIN_NAME_MAX)];
        let url = &url[..url.len().min(PLUGIN_URL_MAX)];
        let size = 4 + name.len() + 1 + url.len() + 1;
        if at + size > PLUGIN_LIST_SIZE {
            break;
        }
        put_u32(&mut list, at, p.checksum);
        list[at + 4..at + 4 + name.len()].copy_from_slice(name);
        let u = at + 4 + name.len() + 1;
        list[u..u + url.len()].copy_from_slice(url);
        at += size;
        written += 1;
    }
    put_u16(&mut list, 0, written);
    (written, list)
}

pub fn recording_header(
    sub_version: u16,
    game: &NewGameData,
    end_tick: u32,
    sizes: BlockSizes,
    saved_game_commands: bool,
) -> [u8; HEADER_SIZE] {
    let mut h = [0u8; HEADER_SIZE];
    let saved = sizes.saved_game_header as u32;
    let initializers = sizes.initializers as u32;
    let snapshot = sizes.snapshot as u32;
    let base = HEADER_SIZE as u32;
    put_u16(&mut h, 0x00, RECORDING_VERSION);
    put_u16(&mut h, 0x02, sub_version);
    put_u32(&mut h, 0x04, base + saved + initializers + snapshot);
    put_u32(&mut h, 0x08, end_tick);
    put_u32(&mut h, 0x0c, base);
    put_u32(&mut h, 0x10, saved);
    put_u32(&mut h, 0x14, base + saved);
    put_u32(&mut h, 0x18, initializers);
    put_u32(&mut h, 0x1c, base + saved + initializers);
    put_u32(&mut h, 0x20, snapshot);
    let g = &game.0;
    let n = NEW_GAME_DATA_OFFSET;
    h[n..n + PLAYERS].copy_from_slice(&g[..PLAYERS]);
    let count = (be_u16(g, PLAYER_COUNT) as i16).clamp(0, MAX_PLAYERS as i16) as usize;
    let players = count * PLAYER_SIZE;
    h[n + PLAYERS..n + PLAYERS + players].copy_from_slice(&g[PLAYERS..PLAYERS + players]);
    h[n + 0x9f4..n + NEW_GAME_DATA_SIZE].copy_from_slice(&g[0x9f4..NEW_GAME_DATA_SIZE]);
    let flags = be_u16(g, NEW_GAME_FLAGS);
    let flags = if saved_game_commands {
        flags & 0xfffc
    } else {
        (flags & 0xfffe) | 2
    };
    put_u16(&mut h, n + NEW_GAME_FLAGS, flags);
    h
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockSizes {
    pub saved_game_header: usize,
    pub initializers: usize,
    pub snapshot: usize,
}

impl MonsterInitializer {
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut r = [0u8; Self::SIZE];
        put_u32(&mut r, 0x04, self.player_id);
        r[0x08..0x0c].copy_from_slice(&self.unit_tag.0);
        r[0x0c..0x10].copy_from_slice(&self.monster_tag.0);
        r[0x10] = self.experience;
        r[0x11] = self.field_8f_plus_one;
        r[0x12] = self.permutation;
        r[0x13] = self.name_index;
        let name = crate::mac_roman::encode(&self.name);
        let len = name.len().min(Self::SIZE - 0x16 - 1);
        r[0x16..0x16 + len].copy_from_slice(&name[..len]);
        r
    }
}

pub fn initializer_block(table: &[MonsterInitializer]) -> Vec<u8> {
    table
        .iter()
        .flat_map(MonsterInitializer::to_bytes)
        .collect()
}

impl CommandPayload {
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let ids = |out: &mut Vec<u8>, ids: &[u16]| {
            out.extend_from_slice(&(ids.len() as u16).to_be_bytes());
            for id in ids {
                out.extend_from_slice(&id.to_be_bytes());
            }
        };
        match self {
            Self::Null => {}
            Self::Sync(s) => {
                out.extend_from_slice(&s.flags.to_be_bytes());
                out.extend_from_slice(&[0, 0]);
                out.extend_from_slice(&s.random_seed.to_be_bytes());
                out.extend_from_slice(&s.snapshot_tick.to_be_bytes());
            }
            Self::General(g) => {
                out.extend_from_slice(&[0, 0]);
                out.extend_from_slice(&g.action.to_be_bytes());
                ids(&mut out, &g.unit_ids);
            }
            Self::Movement(m) => {
                out.extend_from_slice(&m.flags.to_be_bytes());
                out.extend_from_slice(&m.formation.to_be_bytes());
                out.extend_from_slice(&m.facing.to_be_bytes());
                out.extend_from_slice(&[0, 0]);
                ids(&mut out, &m.unit_ids);
                out.extend_from_slice(&(m.waypoints.len() as u16).to_be_bytes());
                for w in &m.waypoints {
                    out.extend_from_slice(&w.x.to_be_bytes());
                    out.extend_from_slice(&w.y.to_be_bytes());
                }
            }
            Self::Target(t) => {
                out.extend_from_slice(&t.flags.to_be_bytes());
                ids(&mut out, &t.unit_ids);
                ids(&mut out, &t.target_ids);
            }
            Self::AttackLocation(a) => {
                out.extend_from_slice(&a.flags.to_be_bytes());
                out.extend_from_slice(&[0, 0]);
                out.extend_from_slice(&a.target_x.to_be_bytes());
                out.extend_from_slice(&a.target_y.to_be_bytes());
                out.extend_from_slice(&a.target_z.to_be_bytes());
                ids(&mut out, &a.unit_ids);
            }
            Self::SyncFormation(f) => {
                for p in &f.parameters {
                    out.extend_from_slice(&p.to_be_bytes());
                }
            }
            Self::Unknown(raw) => out.extend_from_slice(&raw.data),
            _ => return None,
        }
        Some(out)
    }
}

impl CommandPacket {
    pub fn to_bytes(&self) -> Vec<u8> {
        let payload: Cow<'_, [u8]> = if self.raw_payload.is_empty() {
            Cow::Owned(self.payload.encode().unwrap_or_default())
        } else {
            Cow::Borrowed(&self.raw_payload)
        };
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&((8 + payload.len()) as i16).to_be_bytes());
        out.push(self.command_type.to_u8());
        out.push(self.player_index);
        out.extend_from_slice(&self.tick.to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }
}

pub fn sync_command(seed: u32, snapshot_tick: u32) -> CommandPayload {
    CommandPayload::Sync(crate::command::SyncCommand {
        flags: 0,
        random_seed: seed,
        snapshot_tick,
    })
}

#[derive(Debug, Clone)]
pub struct Recorder {
    sub_version: u16,
    game: NewGameData,
    sizes: BlockSizes,
    data: Vec<u8>,
    buffer: Vec<u8>,
    count: u16,
    saved_game_commands: bool,
}

impl Recorder {
    pub fn new(game: NewGameData, initializers: &[MonsterInitializer]) -> Self {
        let block = initializer_block(initializers);
        let sizes = BlockSizes {
            initializers: block.len(),
            ..BlockSizes::default()
        };
        let mut data = recording_header(0, &game, 0, sizes, false).to_vec();
        data.extend_from_slice(&block);
        Self {
            sub_version: 0,
            game,
            sizes,
            data,
            buffer: Vec::with_capacity(MAX_BLOCK_BYTES),
            count: 0,
            saved_game_commands: false,
        }
    }

    pub fn record(&mut self, command: &[u8]) {
        let Some(&kind) = command.get(2) else {
            return;
        };
        if kind.wrapping_sub(0x17) < 3 {
            self.saved_game_commands = true;
        }
        push_command_block(&mut self.data, &mut self.buffer, &mut self.count, command);
    }

    pub fn record_packet(&mut self, command: &CommandPacket) {
        self.record(&command.to_bytes());
    }

    pub fn finish(mut self, end_tick: u32) -> Vec<u8> {
        flush_command_block(&mut self.data, &mut self.buffer, &mut self.count, true);
        let header = recording_header(
            self.sub_version,
            &NewGameData::from_header(&self.data).unwrap_or(self.game),
            end_tick,
            self.sizes,
            self.saved_game_commands,
        );
        self.data[..HEADER_SIZE].copy_from_slice(&header);
        self.data
    }

    pub fn saved_game(
        &self,
        end_tick: u32,
        header: &[u8; crate::saved_game::SAVED_GAME_HEADER_SIZE],
        snapshot: &[u8],
    ) -> Vec<u8> {
        let game = NewGameData::from_header(&self.data).unwrap_or_else(|| self.game.clone());
        let sizes = BlockSizes {
            saved_game_header: header.len() + snapshot.len(),
            ..self.sizes
        };
        let mut out = recording_header(
            SAVED_GAME_SUB_VERSION,
            &game,
            end_tick,
            sizes,
            self.saved_game_commands,
        )
        .to_vec();
        out.extend_from_slice(header);
        out.extend_from_slice(snapshot);
        out.extend_from_slice(&self.data[HEADER_SIZE..]);
        write_command_block(&mut out, &self.buffer, self.count, true);
        out
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty() && self.data.len() == HEADER_SIZE + self.sizes.initializers
    }
}

pub fn tag_header(
    name: &[u8],
    group: FourCC,
    subgroup: FourCC,
    version: u16,
) -> [u8; TAG_HEADER_SIZE] {
    let mut h = [0u8; TAG_HEADER_SIZE];
    let len = name.len().min(31);
    h[4..4 + len].copy_from_slice(&name[..len]);
    h[0x24..0x28].copy_from_slice(&group.0);
    h[0x28..0x2c].copy_from_slice(&subgroup.0);
    put_u32(&mut h, 0x2c, TAG_HEADER_SIZE as u32);
    put_u16(&mut h, 0x38, version);
    h[0x3b] = 0xff;
    h[0x3c..0x40].copy_from_slice(b"mth2");
    h
}

pub fn film_file(name: &[u8], subgroup: FourCC, data: &[u8]) -> Vec<u8> {
    let mut out = tag_header(name, RECORDING_GROUP, subgroup, RECORDING_VERSION).to_vec();
    out.extend_from_slice(data);
    out
}

pub fn save_file(name: &[u8], subgroup: FourCC, data: &[u8]) -> Vec<u8> {
    let mut out = tag_header(name, SAVE_GROUP, subgroup, RECORDING_VERSION).to_vec();
    out.extend_from_slice(data);
    out
}

pub fn sanitize_name(name: &mut Vec<u8>, max: usize) -> bool {
    let mut changed = false;
    let mut seen_other = false;
    let mut trailing: Option<usize> = None;
    let mut i = 0;
    while i < name.len() {
        let c = name[i];
        if c == b' ' || c == b'.' {
            trailing.get_or_insert(i);
            if c == b'.' && !seen_other {
                name[i] = b'_';
                changed = true;
            }
        } else {
            seen_other = true;
            trailing = None;
            if b"\\/|\"<>*?:".contains(&c) {
                name[i] = b'_';
                changed = true;
            }
        }
        if i >= max {
            name.truncate(i);
            changed = true;
            break;
        }
        i += 1;
    }
    if let Some(at) = trailing {
        name.truncate(at);
        changed = true;
    }
    changed
}

pub fn film_names(name: &[u8]) -> Option<(Vec<u8>, Vec<u8>, bool)> {
    tag_names(name, Some(FILM_EXTENSION))
}

pub fn save_names(name: &[u8]) -> Option<(Vec<u8>, Vec<u8>, bool)> {
    tag_names(name, None)
}

fn tag_names(name: &[u8], extension: Option<&str>) -> Option<(Vec<u8>, Vec<u8>, bool)> {
    let mut tag_name = name[..name.len().min(0x1f)].to_vec();
    sanitize_name(&mut tag_name, 0x1f);
    if tag_name.is_empty() {
        return None;
    }
    let mut file = name[..name.len().min(0x3ff)].to_vec();
    let changed = sanitize_name(&mut file, 0x3f9);
    if let Some(ext) = extension.map(str::as_bytes)
        && !(file.len() > ext.len() && file[file.len() - ext.len()..].eq_ignore_ascii_case(ext))
    {
        file.extend_from_slice(ext);
    }
    Some((tag_name, file, changed))
}

pub fn subgroup_for_name(tag_name: &[u8], taken: &[FourCC]) -> FourCC {
    let lower: Vec<u8> = tag_name.iter().map(u8::to_ascii_lowercase).collect();
    let mut tag = *b"    ";
    let mut place = 0usize;
    for word in lower
        .split(|c| b" \t\n()[]{}?!:;-".contains(c))
        .filter(|w| !w.is_empty())
    {
        if place >= 4 {
            break;
        }
        let n = word.len().min(4 - place);
        tag[place..place + n].copy_from_slice(&word[..n]);
        place = (place + 1).max(2);
    }
    let mut value = u32::from_be_bytes(tag);
    while taken.contains(&FourCC(value.to_be_bytes())) {
        value = value.wrapping_add(1);
    }
    FourCC(value.to_be_bytes())
}

pub fn random_subgroup(mut next: impl FnMut() -> u16, taken: &[FourCC]) -> FourCC {
    loop {
        let mut tag = [0u8; 4];
        for c in &mut tag {
            *c = (next() % 0x5f) as u8 + 0x20;
        }
        let tag = FourCC(tag);
        if !taken.contains(&tag) {
            return tag;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Recording;

    #[test]
    fn names_and_subgroups_are_the_engines() {
        let taken = [FourCC(*b"salv"), FourCC(*b"salw"), FourCC(*b"salx")];
        assert_eq!(
            subgroup_for_name(b"salvation_vic_new", &taken),
            FourCC(*b"saly")
        );
        assert_eq!(subgroup_for_name(b"MDL", &[]), FourCC(*b"mdl "));
        assert_eq!(
            subgroup_for_name(b"breach_vic_185", &[FourCC(*b"brea")]),
            FourCC(*b"breb")
        );
        assert_eq!(subgroup_for_name(b"Untitled Replay", &[]), FourCC(*b"unre"));
        assert_eq!(subgroup_for_name(b"a b c d", &[]), FourCC(*b"a bc"));
        assert_eq!(subgroup_for_name(b"", &[]), FourCC(*b"    "));

        let (tag, file, changed) = film_names(b"My Film").unwrap();
        assert_eq!(
            (tag.as_slice(), file.as_slice(), changed),
            (&b"My Film"[..], &b"My Film.m2rec"[..], false)
        );
        let (_, file, _) = film_names(b"x.M2REC").unwrap();
        assert_eq!(file, b"x.M2REC");
        let (tag, _, changed) = film_names(b"a/b?. ").unwrap();
        assert_eq!((tag.as_slice(), changed), (&b"a_b_"[..], true));
        assert!(film_names(b" . ").is_none());
        let mut n = b"..x".to_vec();
        assert!(sanitize_name(&mut n, 0x1f));
        assert_eq!(n, b"__x");
        let mut n = b"abc  d".to_vec();
        assert!(sanitize_name(&mut n, 5));
        assert_eq!(n, b"abc  ");
        let (tag, _, _) = film_names(&[b'z'; 40]).unwrap();
        assert_eq!(tag.len(), 31);
    }

    #[test]
    fn a_block_is_flushed_before_a_command_that_wont_fit() {
        let game = NewGameData(vec![0; NEW_GAME_DATA_SIZE]);
        let mut r = Recorder::new(game, &[]);
        let command = |kind: u8, len: usize| {
            let mut c = vec![0u8; len];
            c[..2].copy_from_slice(&(len as i16).to_be_bytes());
            c[2] = kind;
            c
        };
        r.record(&command(3, 0x400));
        r.record(&command(0, 8));
        r.record(&command(3, 0x400));
        r.record(&command(1, 20));
        let data = r.finish(99);
        let blocks = &data[HEADER_SIZE..];
        assert_eq!(&blocks[..8], &[0, 0, 0, 0, 0x08, 0x00, 0, 2]);
        let second = &blocks[8 + 0x800..];
        assert_eq!(&second[..8], &[0, 0, 0, 1, 0, 20, 0, 1]);
        assert_eq!(second.len(), 8 + 20);
        assert_eq!(&data[8..12], &99u32.to_be_bytes());
        assert_eq!(be_u16(&data, NEW_GAME_DATA_OFFSET + NEW_GAME_FLAGS), 2);
    }

    #[test]
    fn encode_plugins_writes_clipped_count_when_buffer_overflows() {
        let plugins: Vec<PluginInfo> = (0..8)
            .map(|i| PluginInfo {
                name: format!("plugin_{i:02}_{}", "x".repeat(21)),
                url: format!("http://example.com/{}", "y".repeat(44)),
                checksum: 0x1000 + i,
            })
            .collect();
        let game = NewGameData::solo(&SoloNewGame {
            mesh: FourCC(*b"lots"),
            difficulty: 2,
            seed: 42,
            options: 0,
            plugins,
        });
        let header_count = be_u16(&game.0, PLUGIN_COUNT);
        let list_count = be_u16(&game.0, PLUGIN_LIST);
        assert_eq!(header_count, 5);
        assert_eq!(list_count, 5);

        let film_data = Recorder::new(game, &[]).finish(10);
        let bytes = film_file(b"ClipTest", FourCC(*b"clip"), &film_data);
        let rec = Recording::from_bytes(&bytes).unwrap();
        assert_eq!(rec.header.active_plugins.len(), 5);
        for (i, p) in rec.header.active_plugins.iter().enumerate() {
            assert_eq!(p.checksum, 0x1000 + i as u32);
        }
    }
}
