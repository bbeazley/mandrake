use mandrake_common::FourCC;
use mandrake_rec::{CommandPacket, CommandPayload, CommandType, RawCommand};
use mandrake_tag::CollectionReference;

use crate::draw_site;
use crate::engine_rng::{EngineRng, EngineWorld, TagSource, Unmodelled};
use crate::game_state::{FLAG_OUT_OF_SYNC, FLAG_SAVE_LOADED};
use crate::players::{PlayerColor, PlayerName, player_name};

pub const SAVE_LOADED: u8 = 0x19;
pub const REPLACE_PLAYER: u8 = 0x1a;
pub const SET_TEAM_CAPTAIN: u8 = 0x1b;
pub const ADD_PLAYER: u8 = 0x16;
const DROP_PLAYER: u8 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerEntry {
    pub team: i16,
    pub field_02: i16,
    pub flags: u16,
    pub field_08: i32,
    pub id: i32,
    pub metaserver_id: i32,
    pub version_id: i16,
    pub name: PlayerName,
    pub colors: [PlayerColor; 2],
}

impl PlayerEntry {
    pub fn from_record(r: &[u8]) -> Option<PlayerEntry> {
        if r.len() < 0x7c {
            return None;
        }
        let i16_at = |o: usize| i16::from_be_bytes([r[o], r[o + 1]]);
        let mut name: PlayerName = r[0x2c..0x4c].try_into().ok()?;
        name[31] = 0;
        Some(PlayerEntry {
            team: i16_at(0),
            field_02: i16_at(2),
            flags: i16_at(4) as u16,
            field_08: be32(r, 8),
            id: be32(r, 0x0c),
            metaserver_id: be32(r, 0x18),
            version_id: i16_at(0x1e),
            name,
            colors: get_colors(r, 0x6c),
        })
    }
}

pub fn solo_restore_entries(player_count: i16) -> [PlayerEntry; 3] {
    let entry = |team: i16, field_02: i16, id: i32, name: &[u8], color: PlayerColor| PlayerEntry {
        team,
        field_02,
        flags: 0,
        field_08: -1,
        id,
        metaserver_id: 0,
        version_id: 0,
        name: player_name(name),
        colors: [color, color],
    };
    let mut light = entry(
        0,
        0,
        i32::from(player_count),
        b"Light",
        [0xffff, 0xffff, 0xffff, 0],
    );
    light.version_id = 0x889;
    [
        light,
        entry(1, 1, -1, b"Dark", [0x4000, 0x4000, 0x4000, 0]),
        entry(2, 1, -1, b"Neutral", [0x4000, 0, 0, 0]),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplacePlayer {
    pub new_id: i32,
    pub old_id: i32,
    pub field_02: i16,
    pub field_04: i32,
    pub name: PlayerName,
    pub colors: [PlayerColor; 2],
}

const REPLACE_PLAYER_SIZE: usize = 100;
const ADD_PLAYER_SIZE: usize = 0x60;

fn put_colors(d: &mut [u8], at: usize, colors: &[PlayerColor; 2]) {
    for (k, c) in colors.iter().enumerate() {
        for (j, v) in c.iter().enumerate() {
            let o = at + k * 8 + j * 2;
            d[o..o + 2].copy_from_slice(&v.to_be_bytes());
        }
    }
}

fn get_colors(d: &[u8], at: usize) -> [PlayerColor; 2] {
    let w = |o: usize| u16::from_be_bytes([d[at + o], d[at + o + 1]]);
    [[w(0), w(2), w(4), w(6)], [w(8), w(10), w(12), w(14)]]
}

fn be32(d: &[u8], at: usize) -> i32 {
    i32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

impl ReplacePlayer {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut d = vec![0u8; REPLACE_PLAYER_SIZE];
        d[0..4].copy_from_slice(&self.new_id.to_be_bytes());
        d[4..8].copy_from_slice(&self.old_id.to_be_bytes());
        d[8..10].copy_from_slice(&self.field_02.to_be_bytes());
        d[0x0c..0x10].copy_from_slice(&self.field_04.to_be_bytes());
        d[0x14..0x34].copy_from_slice(&self.name);
        put_colors(&mut d, 0x54, &self.colors);
        d
    }

    pub fn from_bytes(d: &[u8]) -> Option<Self> {
        if d.len() < REPLACE_PLAYER_SIZE {
            return None;
        }
        let mut name: PlayerName = d[0x14..0x34].try_into().ok()?;
        name[31] = 0;
        Some(Self {
            new_id: be32(d, 0),
            old_id: be32(d, 4),
            field_02: i16::from_be_bytes([d[8], d[9]]),
            field_04: be32(d, 0x0c),
            name,
            colors: get_colors(d, 0x54),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaveLoaded {
    pub x: i32,
    pub y: i32,
    pub facing: i16,
    pub zoom: i32,
}

impl SaveLoaded {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut d = vec![0u8; 16];
        d[0..4].copy_from_slice(&self.x.to_be_bytes());
        d[4..8].copy_from_slice(&self.y.to_be_bytes());
        d[8..10].copy_from_slice(&self.facing.to_be_bytes());
        d[12..16].copy_from_slice(&self.zoom.to_be_bytes());
        d
    }

    pub fn from_bytes(d: &[u8]) -> Option<Self> {
        (d.len() >= 16).then(|| Self {
            x: be32(d, 0),
            y: be32(d, 4),
            facing: i16::from_be_bytes([d[8], d[9]]),
            zoom: be32(d, 12),
        })
    }
}

fn packet(tick: u32, player: u8, kind: u8, data: Vec<u8>) -> CommandPacket {
    CommandPacket {
        tick,
        player_index: player,
        command_type: CommandType::from_u8(kind),
        payload: CommandPayload::Unknown(RawCommand {
            command_type: kind,
            data: data.clone(),
        }),
        raw_payload: data,
    }
}

pub fn save_loaded_command(tick: u32, observer: SaveLoaded) -> CommandPacket {
    packet(tick, 0xff, SAVE_LOADED, observer.to_bytes())
}

fn names_equal(a: &PlayerName, b: &PlayerName) -> bool {
    let end = |n: &PlayerName| n.iter().position(|&c| c == 0).unwrap_or(n.len());
    a[..end(a)] == b[..end(b)]
}

fn arbitrate_color(own: &mut PlayerColor, id: i32, others: &[PlayerColor]) {
    crate::players::arbitrate_color(own, id, others);
}

fn add_player_packet(e: &PlayerEntry, tick: u32) -> CommandPacket {
    let mut d = vec![0u8; ADD_PLAYER_SIZE];
    d[0..4].copy_from_slice(&e.id.to_be_bytes());
    d[4..6].copy_from_slice(&e.team.to_be_bytes());
    d[6..8].copy_from_slice(&e.version_id.to_be_bytes());
    d[8..12].copy_from_slice(&e.metaserver_id.to_be_bytes());
    d[0x10..0x30].copy_from_slice(&e.name);
    put_colors(&mut d, 0x50, &e.colors);
    packet(tick, 0xff, ADD_PLAYER, d)
}

#[derive(Clone, Copy)]
enum RestoreMode {
    Solo { tick: u32 },
    Network,
}

impl EngineWorld {
    pub fn restore_player_commands(
        &self,
        entries: &mut [PlayerEntry],
        tick: u32,
    ) -> Vec<CommandPacket> {
        self.restore_player_commands_inner(entries, RestoreMode::Solo { tick })
    }

    pub fn network_restore_player_commands(&self, entries: &[PlayerEntry]) -> Vec<CommandPacket> {
        let mut copy = entries.to_vec();
        self.restore_player_commands_inner(&mut copy, RestoreMode::Network)
    }

    fn restore_player_commands_inner(
        &self,
        entries: &mut [PlayerEntry],
        mode: RestoreMode,
    ) -> Vec<CommandPacket> {
        let tick = match mode {
            RestoreMode::Solo { tick } => tick,
            RestoreMode::Network => 0,
        };
        let players = &self.players;
        let n = players.players.len();
        let name_of = |i: usize| players.names.get(i).copied().unwrap_or([0; 32]);
        let colors_of = |i: usize| players.colors.get(i).copied().unwrap_or([[0; 4]; 2]);
        let mut out = Vec::new();
        for (i, p) in players.players.iter().enumerate() {
            if p.field_09 != 0 || p.id == -1 {
                continue;
            }
            let c = ReplacePlayer {
                new_id: p.id.wrapping_add(0x10),
                old_id: p.id,
                field_02: 0,
                field_04: 0,
                name: name_of(i),
                colors: colors_of(i),
            };
            out.push(packet(tick, 0xff, REPLACE_PLAYER, c.to_bytes()));
        }
        let mut matched = vec![false; n];
        for e in entries.iter_mut() {
            if e.field_02 != 0 {
                continue;
            }
            let found = (0..n).find(|&i| {
                let p = &players.players[i];
                !matched[i]
                    && p.field_09 == 0
                    && p.flags & 4 == 0
                    && (e.flags & 0x40 != 0) == (p.flags & 0x40 != 0)
                    && names_equal(&name_of(i), &e.name)
            });
            match (found, mode) {
                (Some(i), RestoreMode::Solo { .. }) => {
                    matched[i] = true;
                    let id = players.players[i].id;
                    if id == -0x11 {
                        continue;
                    }
                    e.id = id;
                    let c = ReplacePlayer {
                        new_id: id,
                        old_id: id.wrapping_add(0x10),
                        field_02: e.version_id,
                        field_04: e.metaserver_id,
                        name: e.name,
                        colors: e.colors,
                    };
                    out.push(packet(tick, 0xff, REPLACE_PLAYER, c.to_bytes()));
                }
                (Some(i), RestoreMode::Network) if players.players[i].id != -0x11 => {
                    matched[i] = true;
                    let c = ReplacePlayer {
                        new_id: e.id,
                        old_id: players.players[i].id.wrapping_add(0x10),
                        field_02: e.version_id,
                        field_04: e.metaserver_id,
                        name: e.name,
                        colors: e.colors,
                    };
                    out.push(packet(tick, 0xff, REPLACE_PLAYER, c.to_bytes()));
                }
                (Some(i), RestoreMode::Network) => {
                    matched[i] = true;
                    out.push(add_player_packet(e, tick));
                }
                (None, _) => out.push(add_player_packet(e, tick)),
            }
        }
        for (i, p) in players.players.iter().enumerate() {
            if matched[i] || p.field_09 != 0 || p.flags & 4 != 0 {
                continue;
            }
            out.push(packet(
                tick,
                p.id.wrapping_add(0x10) as u8,
                DROP_PLAYER,
                Vec::new(),
            ));
        }
        for e in entries.iter() {
            if e.field_02 == 0 && e.team != -1 && e.field_08 == e.id {
                let mut d = vec![0u8; 8];
                d[0..4].copy_from_slice(&e.id.to_be_bytes());
                d[4..6].copy_from_slice(&e.team.to_be_bytes());
                out.push(packet(tick, 0xff, SET_TEAM_CAPTAIN, d));
            }
        }
        out
    }

    pub fn replace_player(
        &mut self,
        c: &ReplacePlayer,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        if self.game_options & mandrake_common::game_options::COOPERATIVE == 0 {
            self.game.flags |= FLAG_OUT_OF_SYNC;
            return Ok(());
        }
        let Some(p) = self.players.players.iter().position(|pl| pl.id == c.old_id) else {
            self.game.flags |= FLAG_OUT_OF_SYNC;
            return Ok(());
        };
        {
            let pl = &mut self.players.players[p];
            pl.flags = 1;
            pl.field_09 = 0;
            pl.id = c.new_id;
        }
        let team = self.players.players[p].team;
        if team == -1 {
            self.players.players[p].flags |= 0x40;
            return Ok(());
        }
        let captain = self
            .players
            .teams
            .get(team as u8 as usize)
            .map(|t| t.player)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::REPLACE_PLAYER_TEAM,
                    format!("replace_player: team {team}"),
                )
            })?;
        let teammates: Vec<PlayerColor> = (0..self.players.players.len())
            .filter(|&j| j != p && self.players.players[j].team == team)
            .map(|j| self.players.colors.get(j).map_or([0; 4], |c| c[1]))
            .collect();
        let mut own = c.colors[0];
        arbitrate_color(&mut own, c.new_id, &teammates);
        if !self.players.colors.is_empty() {
            let primary = usize::try_from(captain)
                .ok()
                .and_then(|k| self.players.colors.get(k))
                .map(|c| c[0])
                .ok_or_else(|| {
                    Unmodelled::new(
                        draw_site::REPLACE_PLAYER_CAPTAIN,
                        format!("replace_player: team {team}'s player {captain}"),
                    )
                })?;
            if let Some(colors) = self.players.colors.get_mut(p) {
                *colors = [primary, own];
            }
        }
        let p16 = p as i16;
        let creation = self.marker_creation.clone();
        if let Some(creation) = &creation {
            let tags: Vec<FourCC> = creation
                .mesh
                .palettes
                .iter()
                .filter(|e| e.marker_type == 3)
                .filter(|e| e.team_index() != -1 && e.team_index() == i16::from(team))
                .map(|e| e.tag_id)
                .collect();
            for tag in tags {
                self.update_custom_unit_definition_for_player(creation.as_ref(), tag, p16, rng)?;
            }
            let artifacts: Vec<FourCC> = self
                .catalog
                .artifacts
                .iter()
                .filter_map(|(_, d)| artifact_unit(d))
                .collect();
            for tag in artifacts {
                self.update_custom_unit_definition_for_player(creation.as_ref(), tag, p16, rng)?;
            }
        }
        Ok(())
    }

    pub fn update_custom_unit_definition_for_player(
        &mut self,
        tags: &dyn TagSource,
        unit: FourCC,
        p: i16,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let key = (u32::from_be_bytes(unit.0), p);
        let Some(row) = self.unit_rows.get(&key).copied() else {
            return Ok(());
        };
        if row.owner != p || !row.custom_core {
            return Ok(());
        }
        let colors = usize::try_from(p)
            .ok()
            .filter(|&i| {
                self.players
                    .players
                    .get(i)
                    .is_some_and(|pl| pl.flags & 1 != 0)
            })
            .and_then(|i| self.players.colors.get(i))
            .copied();
        let custom = match colors {
            None => false,
            Some(colors) => {
                let data = tags
                    .tag(*b"unit", unit)
                    .filter(|d| d.len() >= 8)
                    .ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::CUSTOM_UNIT_MISSING,
                            format!("unit {unit} missing"),
                        )
                    })?;
                let core_tag = FourCC::new([data[4], data[5], data[6], data[7]]);
                let core = tags.tag(*b"core", core_tag).ok_or_else(|| {
                    Unmodelled::new(
                        draw_site::CUSTOM_CORE_NO_COLLECTION,
                        format!("core {core_tag} missing"),
                    )
                })?;
                let collection = CollectionReference::from_bytes(&core)
                    .ok()
                    .and_then(|c| tags.tag(*b".256", c.collection_tag))
                    .ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::CUSTOM_CORE_NO_COLLECTION,
                            format!("core {core_tag}: no collection"),
                        )
                    })?;
                let hue_flags = crate::players::collection_hue_flags(&collection);
                crate::players::colors_replace_core(&core, &hue_flags, &colors)
            }
        };
        if custom {
            rng.draw(draw_site::CUSTOM_CORE);
        }
        if let Some(r) = self.unit_rows.get_mut(&key) {
            r.custom_core = custom;
        }
        Ok(())
    }

    pub fn set_team_captain(&mut self, data: &[u8]) {
        if self.game_options & mandrake_common::game_options::COOPERATIVE == 0 || data.len() < 6 {
            self.game.flags |= FLAG_OUT_OF_SYNC;
            return;
        }
        let id = be32(data, 0);
        let team = i16::from_be_bytes([data[4], data[5]]);
        if team < 0 {
            self.game.flags |= FLAG_OUT_OF_SYNC;
            return;
        }
        let Some(p) = self.players.players.iter().position(|pl| pl.id == id) else {
            self.game.flags |= FLAG_OUT_OF_SYNC;
            return;
        };
        match self.players.teams.get_mut(team as usize) {
            Some(t) => t.player = p as i16,
            None => self.game.flags |= FLAG_OUT_OF_SYNC,
        }
    }

    pub fn save_loaded(&mut self, observer: SaveLoaded) -> SaveLoaded {
        self.game.flags |= FLAG_SAVE_LOADED;
        if let Some(i) = self
            .players
            .players
            .iter()
            .position(|pl| pl.field_09 == 0 && pl.flags & 4 == 0 && pl.id != -1)
        {
            self.local_player = i as i16;
        }
        observer
    }
}

fn artifact_unit(d: &[u8]) -> Option<FourCC> {
    let flags = u32::from_be_bytes(d.get(0..4)?.try_into().ok()?);
    let unit = d.get(0x98..0x9c)?;
    let tag = u32::from_be_bytes(unit.try_into().ok()?);
    (flags & 4 != 0 && tag != u32::MAX && tag != 0)
        .then(|| FourCC::new([unit[0], unit[1], unit[2], unit[3]]))
}
