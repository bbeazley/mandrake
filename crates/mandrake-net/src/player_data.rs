use crate::wire::{Body, WireError, kind};

pub const PLAYER_DATA_SIZE: usize = 0x7c;
pub const APPEARANCE_SIZE: usize = 0x54;
pub const GAMEPLAY_VERSION: u16 = mandrake_common::game_options::VERSION_ID_1_8_5;
pub const TFL_PLAYER_VERSION: u16 = 0x47e;
pub const BUILD_NUMBER: u16 = 0x1d7;
pub const NONE: i32 = -1;

const FLAG_TAKEN: u16 = 8;
const FLAG_OWN_TEAM: u16 = 0x20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appearance {
    pub field_00: u16,
    pub rank: i16,
    pub name: [u8; 32],
    pub team_name: [u8; 32],
    pub colors: [u16; 8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerData {
    pub field_00: i16,
    pub field_02: i16,
    pub flags: u16,
    pub field_06: u16,
    pub team: i32,
    pub net_id: i32,
    pub ready: u32,
    pub team_locked: u32,
    pub metaserver_id: i32,
    pub metaserver: u8,
    pub field_1d: u8,
    pub version: u16,
    pub build: u16,
    pub field_22: [u8; 6],
    pub appearance: Appearance,
}

use crate::wire::{be16, be32};

pub fn fixed<const N: usize>(s: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    let n = s.len().min(N - 1);
    out[..n].copy_from_slice(&s[..n]);
    out
}

pub fn copy_over<const N: usize>(field: &mut [u8; N], s: &[u8]) {
    let n = until_nul(s).len().min(N - 1);
    field[..n].copy_from_slice(&s[..n]);
    field[n] = 0;
}

pub fn until_nul(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

impl Appearance {
    pub fn read(b: &[u8; APPEARANCE_SIZE]) -> Appearance {
        let mut colors = [0; 8];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = be16(b, 0x44 + i * 2);
        }
        Appearance {
            field_00: be16(b, 0),
            rank: be16(b, 2) as i16,
            name: b[0x04..0x24].try_into().unwrap(),
            team_name: b[0x24..0x44].try_into().unwrap(),
            colors,
        }
    }

    pub fn to_bytes(&self) -> [u8; APPEARANCE_SIZE] {
        let mut b = [0; APPEARANCE_SIZE];
        b[0..2].copy_from_slice(&self.field_00.to_be_bytes());
        b[2..4].copy_from_slice(&self.rank.to_be_bytes());
        b[0x04..0x24].copy_from_slice(&self.name);
        b[0x24..0x44].copy_from_slice(&self.team_name);
        for (i, c) in self.colors.iter().enumerate() {
            b[0x44 + i * 2..0x46 + i * 2].copy_from_slice(&c.to_be_bytes());
        }
        b
    }
}

impl PlayerData {
    pub fn new(name: &[u8], team_name: &[u8], colors: [u16; 8]) -> PlayerData {
        PlayerData {
            field_00: -1,
            field_02: 0,
            flags: 0x11,
            field_06: 0,
            team: NONE,
            net_id: 0,
            ready: 0,
            team_locked: 0,
            metaserver_id: NONE,
            metaserver: 0,
            field_1d: 0,
            version: GAMEPLAY_VERSION,
            build: BUILD_NUMBER,
            field_22: [0; 6],
            appearance: Appearance {
                field_00: 0,
                rank: -1,
                name: fixed(name),
                team_name: fixed(team_name),
                colors,
            },
        }
    }

    pub fn read(b: &[u8]) -> Result<PlayerData, WireError> {
        let b: &[u8; PLAYER_DATA_SIZE] = b.try_into().map_err(|_| WireError::BadBody {
            kind: kind::JOIN_REQUEST,
            reason: "player data is not 0x7c bytes",
        })?;
        Ok(PlayerData {
            field_00: be16(b, 0x00) as i16,
            field_02: be16(b, 0x02) as i16,
            flags: be16(b, 0x04),
            field_06: be16(b, 0x06),
            team: be32(b, 0x08) as i32,
            net_id: be32(b, 0x0c) as i32,
            ready: be32(b, 0x10),
            team_locked: be32(b, 0x14),
            metaserver_id: be32(b, 0x18) as i32,
            metaserver: b[0x1c],
            field_1d: b[0x1d],
            version: be16(b, 0x1e),
            build: be16(b, 0x20),
            field_22: b[0x22..0x28].try_into().unwrap(),
            appearance: Appearance::read(b[0x28..].try_into().unwrap()),
        })
    }

    pub fn to_bytes(&self) -> [u8; PLAYER_DATA_SIZE] {
        let mut b = [0; PLAYER_DATA_SIZE];
        b[0x00..0x02].copy_from_slice(&self.field_00.to_be_bytes());
        b[0x02..0x04].copy_from_slice(&self.field_02.to_be_bytes());
        b[0x04..0x06].copy_from_slice(&self.flags.to_be_bytes());
        b[0x06..0x08].copy_from_slice(&self.field_06.to_be_bytes());
        b[0x08..0x0c].copy_from_slice(&self.team.to_be_bytes());
        b[0x0c..0x10].copy_from_slice(&self.net_id.to_be_bytes());
        b[0x10..0x14].copy_from_slice(&self.ready.to_be_bytes());
        b[0x14..0x18].copy_from_slice(&self.team_locked.to_be_bytes());
        b[0x18..0x1c].copy_from_slice(&self.metaserver_id.to_be_bytes());
        b[0x1c] = self.metaserver;
        b[0x1d] = self.field_1d;
        b[0x1e..0x20].copy_from_slice(&self.version.to_be_bytes());
        b[0x20..0x22].copy_from_slice(&self.build.to_be_bytes());
        b[0x22..0x28].copy_from_slice(&self.field_22);
        b[0x28..].copy_from_slice(&self.appearance.to_bytes());
        b
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequest {
    pub name: Vec<u8>,
    pub password: Vec<u8>,
    pub player: PlayerData,
}

impl JoinRequest {
    pub fn from_body(body: &Body) -> Result<JoinRequest, WireError> {
        match body {
            Body::JoinRequest {
                name,
                password,
                player_data,
            } => Ok(JoinRequest {
                name: name.clone(),
                password: password.clone(),
                player: PlayerData::read(player_data)?,
            }),
            _ => Err(WireError::BadBody {
                kind: body.kind(),
                reason: "not a join request",
            }),
        }
    }

    pub fn to_body(&self) -> Body {
        Body::JoinRequest {
            name: self.name.clone(),
            password: self.password.clone(),
            player_data: self.player.to_bytes().to_vec(),
        }
    }
}

pub fn sort_players(players: &mut Vec<PlayerData>) {
    players.sort_by(|a, b| {
        b.appearance
            .rank
            .cmp(&a.appearance.rank)
            .then(a.net_id.cmp(&b.net_id))
    });
    for p in players.iter_mut() {
        p.flags &= !FLAG_OWN_TEAM;
    }
    loop {
        for p in players.iter_mut() {
            p.flags &= !FLAG_TAKEN;
        }
        let mut out = Vec::with_capacity(players.len());
        for i in 0..players.len() {
            let p = &players[i];
            let leads = p.flags & FLAG_TAKEN == 0
                && (p.net_id == p.team || p.team == NONE || p.flags & FLAG_OWN_TEAM != 0);
            if !leads {
                continue;
            }
            out.push(p.clone());
            players[i].flags |= FLAG_TAKEN;
            let leader = players[i].net_id;
            for m in players.iter_mut() {
                if m.flags & FLAG_TAKEN == 0 && m.team == leader {
                    out.push(m.clone());
                    m.flags |= FLAG_TAKEN;
                }
            }
        }
        let mut done = true;
        for p in players.iter_mut() {
            if p.flags & FLAG_TAKEN == 0 {
                p.flags |= FLAG_TAKEN | FLAG_OWN_TEAM;
                p.team = NONE;
                let mut q = p.clone();
                q.flags &= !FLAG_TAKEN;
                out.push(q);
                done = false;
            }
        }
        if done {
            *players = out;
            return;
        }
    }
}
