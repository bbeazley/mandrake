//! Writing TFL film headers, network game new-game data, and command blocks.

use mandrake_common::FourCC;
use mandrake_common::game_options::{COOPERATIVE, VETERANS};

use super::{TflFilmHeader, TflFilmPlayer};
use crate::binary::{CommandBlockPacker, be_u16, be_u32, c_string, put_u16, put_u32};
use crate::command::{CommandType, TickCommands};

/// TFL film new-game parameters (`0x28` bytes, big-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TflGameParameters {
    /// 0 replay, 1 network, 2 solo.
    pub mode: i16,
    pub game_type: i16,
    pub options: u32,
    pub time_limit: u32,
    pub mesh: FourCC,
    pub difficulty: i16,
    pub max_players: i16,
    pub field_14: i16,
    pub team_size: i16,
    pub seed: u32,
    pub planning_time: u32,
    pub field_20: [u8; 8],
}

impl TflGameParameters {
    pub const SIZE: usize = 0x28;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Self {
        Self {
            mode: be_u16(b, 0) as i16,
            game_type: be_u16(b, 2) as i16,
            options: be_u32(b, 4),
            time_limit: be_u32(b, 8),
            mesh: FourCC(b[0xc..0x10].try_into().expect("4 bytes")),
            difficulty: be_u16(b, 0x10) as i16,
            max_players: be_u16(b, 0x12) as i16,
            field_14: be_u16(b, 0x14) as i16,
            team_size: be_u16(b, 0x16) as i16,
            seed: be_u32(b, 0x18),
            planning_time: be_u32(b, 0x1c),
            field_20: b[0x20..0x28].try_into().expect("8 bytes"),
        }
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put_u16(&mut b, 0, self.mode as u16);
        put_u16(&mut b, 2, self.game_type as u16);
        put_u32(&mut b, 4, self.options);
        put_u32(&mut b, 8, self.time_limit);
        b[0xc..0x10].copy_from_slice(&self.mesh.0);
        put_u16(&mut b, 0x10, self.difficulty as u16);
        put_u16(&mut b, 0x12, self.max_players as u16);
        put_u16(&mut b, 0x14, self.field_14 as u16);
        put_u16(&mut b, 0x16, self.team_size as u16);
        put_u32(&mut b, 0x18, self.seed);
        put_u32(&mut b, 0x1c, self.planning_time);
        b[0x20..0x28].copy_from_slice(&self.field_20);
        b
    }
}

/// A player's `0x7c`-byte record as carried on the network and in new-game data.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TflNetPlayer {
    pub team: i16,
    pub field_02: i16,
    pub flags: u16,
    pub field_06: u16,
    pub captain: i32,
    pub net_id: i32,
    pub field_10: [u32; 2],
    pub metaserver_id: i32,
    pub field_1c: [u8; 2],
    pub field_1e: u16,
    pub field_20: [u8; 8],
    pub appearance_00: u16,
    pub appearance_02: i16,
    pub name: String,
    pub team_name: String,
    pub primary: [u16; 4],
    pub secondary: [u16; 4],
}

impl TflNetPlayer {
    pub const SIZE: usize = 0x7c;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Self {
        Self {
            team: be_u16(b, 0) as i16,
            field_02: be_u16(b, 2) as i16,
            flags: be_u16(b, 4),
            field_06: be_u16(b, 6),
            captain: be_u32(b, 8) as i32,
            net_id: be_u32(b, 0xc) as i32,
            field_10: [be_u32(b, 0x10), be_u32(b, 0x14)],
            metaserver_id: be_u32(b, 0x18) as i32,
            field_1c: [b[0x1c], b[0x1d]],
            field_1e: be_u16(b, 0x1e),
            field_20: b[0x20..0x28].try_into().expect("8 bytes"),
            appearance_00: be_u16(b, 0x28),
            appearance_02: be_u16(b, 0x2a) as i16,
            name: c_string(&b[0x2c..0x4c]),
            team_name: c_string(&b[0x4c..0x6c]),
            primary: std::array::from_fn(|i| be_u16(b, 0x6c + 2 * i)),
            secondary: std::array::from_fn(|i| be_u16(b, 0x74 + 2 * i)),
        }
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put_u16(&mut b, 0, self.team as u16);
        put_u16(&mut b, 2, self.field_02 as u16);
        put_u16(&mut b, 4, self.flags);
        put_u16(&mut b, 6, self.field_06);
        put_u32(&mut b, 8, self.captain as u32);
        put_u32(&mut b, 0xc, self.net_id as u32);
        put_u32(&mut b, 0x10, self.field_10[0]);
        put_u32(&mut b, 0x14, self.field_10[1]);
        put_u32(&mut b, 0x18, self.metaserver_id as u32);
        b[0x1c..0x1e].copy_from_slice(&self.field_1c);
        put_u16(&mut b, 0x1e, self.field_1e);
        b[0x20..0x28].copy_from_slice(&self.field_20);
        put_u16(&mut b, 0x28, self.appearance_00);
        put_u16(&mut b, 0x2a, self.appearance_02 as u16);
        put_string(&mut b[0x2c..0x4c], &self.name);
        put_string(&mut b[0x4c..0x6c], &self.team_name);
        put_color(&mut b, 0x6c, self.primary);
        put_color(&mut b, 0x74, self.secondary);
        b
    }

    fn computer(team: i16, name: &str, color: [u16; 3]) -> Self {
        let color = [color[0], color[1], color[2], 0];
        Self {
            team,
            field_02: 1,
            flags: 0,
            field_06: 0,
            captain: -1,
            net_id: -1,
            field_10: [0; 2],
            metaserver_id: -1,
            field_1c: [0; 2],
            field_1e: 0,
            field_20: [0; 8],
            appearance_00: 0,
            appearance_02: 0,
            name: name.to_string(),
            team_name: String::new(),
            primary: color,
            secondary: color,
        }
    }

    fn film_player(&self, slot: usize) -> TflFilmPlayer {
        TflFilmPlayer {
            slot,
            name: self.name.clone(),
            team_name: self.team_name.clone(),
            primary: self.primary,
            secondary: self.secondary,
            team: self.team,
            field_02: self.field_02,
            flags: self.flags,
            captain: self.captain as i16,
            net_id: self.net_id as i16,
            field_0a: 0,
            metaserver_id: self.metaserver_id as u32,
            field_10: [0; 8],
            appearance_00: self.appearance_00,
            appearance_02: self.appearance_02,
        }
    }
}

/// A TFL film's new-game data (`0x6f8` bytes).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TflNewGame {
    pub parameters: TflGameParameters,
    pub local_player: i16,
    pub players: Vec<TflFilmPlayer>,
}

impl TflNewGame {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut d = vec![0u8; TflFilmHeader::SIZE - 0x24 - 8];
        d[..TflGameParameters::SIZE].copy_from_slice(&self.parameters.to_bytes());
        let count = self.players.len().min(TflFilmHeader::PLAYER_SLOTS);
        put_u16(&mut d, 0x28, count as u16);
        put_u16(&mut d, 0x2a, self.local_player as u16);
        for (i, p) in self.players.iter().take(count).enumerate() {
            let s = &mut d[TflFilmHeader::PLAYERS - 0x24 + i * TflFilmPlayer::SIZE..]
                [..TflFilmPlayer::SIZE];
            put_u16(s, 0, p.team as u16);
            put_u16(s, 2, p.field_02 as u16);
            put_u16(s, 4, p.flags);
            put_u16(s, 6, p.captain as u16);
            put_u16(s, 8, p.net_id as u16);
            put_u16(s, 0xa, p.field_0a as u16);
            put_u32(s, 0xc, p.metaserver_id);
            s[0x10..0x18].copy_from_slice(&p.field_10);
            put_u16(s, 0x18, p.appearance_00);
            put_u16(s, 0x1a, p.appearance_02 as u16);
            put_string(&mut s[0x1c..0x3c], &p.name);
            put_string(&mut s[0x3c..0x5c], &p.team_name);
            put_color(s, 0x5c, p.primary);
            put_color(s, 0x64, p.secondary);
        }
        d
    }

    /// Build a network game's new-game data matching TFL's captain grouping and co-op rules.
    pub fn network(
        parameters: &TflGameParameters,
        players: &[TflNetPlayer],
        local_net_id: i32,
    ) -> Self {
        let mut parameters = *parameters;
        let cooperative = parameters.options & COOPERATIVE != 0;
        // In co-op, the shared team id is the first player's net id, or the last differing captain.
        let mut coop_team = players.first().map_or(-1, |p| p.net_id as i16);
        if cooperative {
            for p in players {
                if p.captain != -1 && p.captain != i32::from(coop_team) {
                    coop_team = p.captain as i16;
                }
            }
            parameters.team_size = -1;
            parameters.time_limit = 0;
        }
        let mut sorted = sort_new_player_data(players, true);
        let mut local_player = 0;
        for (i, p) in sorted.iter_mut().enumerate() {
            p.team = -1;
            if cooperative {
                p.team = 0;
                p.captain = i32::from(coop_team);
            }
            p.field_02 = 0;
            if p.net_id == local_net_id {
                local_player = i as i16;
            }
        }
        if cooperative {
            parameters.options |= VETERANS;
            sorted.push(TflNetPlayer::computer(1, "Dark", [0, 0, 0xffff]));
            sorted.push(TflNetPlayer::computer(2, "Neutral", [0x4000, 0, 0]));
        }
        Self {
            parameters,
            local_player,
            players: sorted
                .iter()
                .take(TflFilmHeader::PLAYER_SLOTS)
                .enumerate()
                .map(|(i, p)| p.film_player(i))
                .collect(),
        }
    }
}

/// Sort players by appearance rank descending then net id ascending, grouping teammates behind
/// their captain; players whose captain is missing are promoted on a second pass with flag `0x20`.
fn sort_new_player_data(players: &[TflNetPlayer], clear: bool) -> Vec<TflNetPlayer> {
    let mut input = players.to_vec();
    input.sort_by(|a, b| {
        b.appearance_02
            .cmp(&a.appearance_02)
            .then(a.net_id.cmp(&b.net_id))
    });
    for p in &mut input {
        p.flags &= !0x20;
    }
    loop {
        // Each pass starts with flag 0x08 clear; copies are pushed before orphans are flagged.
        for p in &mut input {
            p.flags &= !0x08;
        }
        let mut out = Vec::with_capacity(input.len());
        let mut placed = vec![false; input.len()];
        for i in 0..input.len() {
            let p = &input[i];
            if placed[i] || !(p.net_id == p.captain || p.captain == -1 || p.flags & 0x20 != 0) {
                continue;
            }
            out.push(p.clone());
            placed[i] = true;
            let captain = p.net_id;
            for j in 0..input.len() {
                if !placed[j] && input[j].captain == captain {
                    out.push(input[j].clone());
                    placed[j] = true;
                }
            }
        }
        let mut done = true;
        for (i, p) in input.iter_mut().enumerate() {
            if !placed[i] {
                p.flags |= 0x28;
                if clear {
                    p.captain = -1;
                }
                done = false;
            }
        }
        if done {
            return out;
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TflBlockSizes {
    pub saved_game: u32,
    pub initializers: u32,
    pub snapshot: u32,
}

pub fn tfl_recording_header(
    sub_version: u16,
    game: &TflNewGame,
    end_tick: u32,
    sizes: TflBlockSizes,
    checksums: [u32; 2],
) -> [u8; TflFilmHeader::SIZE] {
    let mut h = [0u8; TflFilmHeader::SIZE];
    let base = TflFilmHeader::SIZE as u32;
    let initializers = base + sizes.saved_game;
    let snapshot = initializers + sizes.initializers;
    put_u16(&mut h, 0, 3);
    put_u16(&mut h, 2, sub_version);
    put_u32(&mut h, 4, snapshot + sizes.snapshot);
    put_u32(&mut h, 8, end_tick);
    put_u32(&mut h, 0xc, base);
    put_u32(&mut h, 0x10, sizes.saved_game);
    put_u32(&mut h, 0x14, initializers);
    put_u32(&mut h, 0x18, sizes.initializers);
    put_u32(&mut h, 0x1c, snapshot);
    put_u32(&mut h, 0x20, sizes.snapshot);
    h[0x24..0x71c].copy_from_slice(&game.to_bytes());
    put_u32(&mut h, 0x71c, checksums[0]);
    put_u32(&mut h, 0x720, checksums[1]);
    h
}

pub fn tfl_tag_header(
    name: &[u8],
    tag_id: FourCC,
    version: u16,
    word_28: u16,
    flags: [u8; 4],
    data_size: u32,
    user_data: u32,
) -> [u8; TflFilmHeader::TAG_HEADER_SIZE] {
    let mut h = [0u8; TflFilmHeader::TAG_HEADER_SIZE];
    let len = name.len().min(31);
    h[..len].copy_from_slice(&name[..len]);
    h[0x20..0x24].copy_from_slice(&TflFilmHeader::TAG_TYPE.0);
    h[0x24..0x28].copy_from_slice(&tag_id.0);
    put_u16(&mut h, 0x28, word_28);
    put_u16(&mut h, 0x2a, version);
    h[0x2c..0x30].copy_from_slice(&flags);
    put_u32(&mut h, 0x30, TflFilmHeader::TAG_HEADER_SIZE as u32);
    put_u32(&mut h, 0x34, data_size);
    put_u32(&mut h, 0x38, user_data);
    h[0x3c..0x40].copy_from_slice(&TflFilmHeader::SIGNATURE.0);
    h
}

pub const TFL_MAX_COMMAND_BLOCK: usize = crate::writer::MAX_BLOCK_BYTES;
pub const TFL_FILM_TAG_FLAGS: [u8; 4] = tfl_film_tag_flags(1);

pub const fn tfl_film_tag_flags(writes: u16) -> [u8; 4] {
    let [hi, lo] = writes.to_be_bytes();
    [hi, lo, 0x80, 0x04]
}

/// Tag write count for a TFL last-recording film: initial header, optional veterans block, two
/// writes per command block (header + body), and final header with end tick.
pub fn tfl_last_recording_writes(veterans: bool, blocks: usize) -> u16 {
    (2 + usize::from(veterans) + 2 * blocks) as u16
}

/// Pack commands into TFL film command blocks (same framing as Myth II, skipping Null and
/// oversized commands).
pub fn tfl_command_blocks(commands: &[TickCommands]) -> Vec<u8> {
    command_blocks(commands).0
}

fn command_blocks(commands: &[TickCommands]) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut packer = CommandBlockPacker::new();
    for c in commands.iter().flat_map(|t| &t.commands) {
        if c.command_type == CommandType::Null {
            continue;
        }
        packer.push(&mut out, &c.to_bytes());
    }
    packer.flush(&mut out, true);
    (out, packer.blocks_flushed())
}

/// Assemble a complete TFL `reco` tag film.
pub fn tfl_film(
    game: &TflNewGame,
    checksums: [u32; 2],
    name: &[u8],
    id: FourCC,
    end_tick: u32,
    veterans: &[u8],
    commands: &[TickCommands],
) -> Vec<u8> {
    let mut data = tfl_recording_header(
        0,
        game,
        end_tick,
        TflBlockSizes {
            initializers: veterans.len() as u32,
            ..TflBlockSizes::default()
        },
        checksums,
    )
    .to_vec();
    data.extend_from_slice(veterans);
    let (blocks, count) = command_blocks(commands);
    data.extend(blocks);
    let flags = tfl_film_tag_flags(tfl_last_recording_writes(!veterans.is_empty(), count));
    let mut film = tfl_tag_header(name, id, 3, 0, flags, data.len() as u32, 0).to_vec();
    film.extend(data);
    film
}

fn put_color(d: &mut [u8], at: usize, c: [u16; 4]) {
    for (i, &v) in c.iter().enumerate() {
        put_u16(d, at + 2 * i, v);
    }
}

fn put_string(field: &mut [u8], s: &str) {
    let b = crate::mac_roman::encode(s);
    let len = b.len().min(field.len() - 1);
    field[..len].copy_from_slice(&b[..len]);
}
