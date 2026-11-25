//! TFL film reader and writer: a `reco` tag with TFL's 64-byte `myth` tag header, a 0x724-byte
//! film header, and command blocks in the same framing as Myth II.

use std::io::{Cursor, Read, Seek, SeekFrom};

use mandrake_common::{FourCC, M2Error};

use crate::binary::{be_u16, be_u32, c_string};
use crate::command::{TickCommands, read_command_stream};

#[path = "tfl_write.rs"]
mod write;
pub use write::{
    TFL_FILM_TAG_FLAGS, TFL_MAX_COMMAND_BLOCK, TflBlockSizes, TflGameParameters, TflNetPlayer,
    TflNewGame, tfl_command_blocks, tfl_film, tfl_film_tag_flags, tfl_last_recording_writes,
    tfl_recording_header, tfl_tag_header,
};

/// One player slot of a TFL film (`0x6c` bytes).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TflFilmPlayer {
    pub slot: usize,
    pub name: String,
    pub team_name: String,
    #[serde(default)]
    pub primary: [u16; 4],
    #[serde(default)]
    pub secondary: [u16; 4],
    #[serde(default)]
    pub team: i16,
    #[serde(default)]
    pub field_02: i16,
    #[serde(default)]
    pub flags: u16,
    #[serde(default)]
    pub captain: i16,
    #[serde(default)]
    pub net_id: i16,
    #[serde(default)]
    pub field_0a: i16,
    #[serde(default)]
    pub metaserver_id: u32,
    #[serde(default)]
    pub field_10: [u8; 8],
    #[serde(default)]
    pub appearance_00: u16,
    #[serde(default)]
    pub appearance_02: i16,
}

/// A TFL film's header.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TflFilmHeader {
    pub name: String,
    pub tag_id: FourCC,
    pub tag_version: u16,
    #[serde(default)]
    pub tag_word_28: u16,
    #[serde(default)]
    pub tag_flags: [u8; 4],
    #[serde(default)]
    pub tag_user_data: u32,
    #[serde(default)]
    pub tag_data_size: u32,
    pub version: u16,
    pub sub_version: u16,
    pub commands_offset: u32,
    pub end_tick: u32,
    #[serde(default)]
    pub block_sizes: TflBlockSizes,
    #[serde(default)]
    pub mode: i16,
    pub game_type: i16,
    pub difficulty: i16,
    pub game_options: u32,
    pub time_limit: u32,
    pub mesh_tag_id: FourCC,
    #[serde(default)]
    pub max_players: i16,
    #[serde(default)]
    pub parameters_14: i16,
    #[serde(default)]
    pub team_size: i16,
    pub initial_seed: u32,
    #[serde(default)]
    pub planning_time: u32,
    #[serde(default)]
    pub parameters_20: [u8; 8],
    #[serde(default)]
    pub local_player: i16,
    pub players: Vec<TflFilmPlayer>,
    pub checksums: [u32; 2],
}

/// A TFL film: header and decoded command stream.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TflRecording {
    pub header: TflFilmHeader,
    pub tick_commands: Vec<TickCommands>,
}

impl TflFilmPlayer {
    pub const SIZE: usize = 0x6c;

    pub(crate) fn parse(slot: usize, p: &[u8]) -> Self {
        Self {
            slot,
            name: c_string(&p[0x1c..0x3c]),
            team_name: c_string(&p[0x3c..0x5c]),
            primary: std::array::from_fn(|i| be_u16(p, 0x5c + 2 * i)),
            secondary: std::array::from_fn(|i| be_u16(p, 0x64 + 2 * i)),
            team: be_u16(p, 0) as i16,
            field_02: be_u16(p, 2) as i16,
            flags: be_u16(p, 4),
            captain: be_u16(p, 6) as i16,
            net_id: be_u16(p, 8) as i16,
            field_0a: be_u16(p, 0xa) as i16,
            metaserver_id: be_u32(p, 0xc),
            field_10: p[0x10..0x18].try_into().expect("8 bytes"),
            appearance_00: be_u16(p, 0x18),
            appearance_02: be_u16(p, 0x1a) as i16,
        }
    }
}

impl TflFilmHeader {
    pub const SIZE: usize = 0x724;
    pub const TAG_TYPE: FourCC = FourCC(*b"reco");
    pub const SIGNATURE: FourCC = FourCC(*b"myth");
    pub const TAG_HEADER_SIZE: usize = 0x40;
    pub(crate) const PLAYERS: usize = 0x50;
    pub(crate) const PLAYER_SLOTS: usize = 16;

    pub fn is_tfl_film(bytes: &[u8]) -> bool {
        bytes.len() >= 64
            && bytes[0x20..0x24] == Self::TAG_TYPE.0
            && bytes[0x3c..0x40] == Self::SIGNATURE.0
    }

    /// Parse the tag header and film header; returns the header and the file offset where the
    /// commands start.
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<(Self, u64), M2Error> {
        let mut tag = [0u8; 64];
        reader.seek(SeekFrom::Start(0))?;
        reader.read_exact(&mut tag)?;
        if !Self::is_tfl_film(&tag) {
            return Err(M2Error::InvalidMagic {
                expected: "a TFL reco tag (myth)".to_string(),
                found: String::from_utf8_lossy(&tag[0x3c..0x40]).into_owned(),
            });
        }
        let data_offset = be_u32(&tag, 0x30) as u64;
        let file_len = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(data_offset))?;
        let mut h = [0u8; Self::SIZE];
        reader.read_exact(&mut h)?;
        let version = be_u16(&h, 0);
        if version != 3 {
            return Err(M2Error::ReplayParse(format!(
                "TFL film version {version}, expected 3"
            )));
        }
        let commands_offset = be_u32(&h, 0x04);
        let commands_start = data_offset + commands_offset as u64;
        if (commands_offset as usize) < Self::SIZE || commands_start > file_len {
            return Err(M2Error::ReplayParse(format!(
                "TFL film: bad command offset {commands_offset} in a {file_len}-byte file"
            )));
        }
        let count = (be_u16(&h, 0x4c) as i16).clamp(0, Self::PLAYER_SLOTS as i16) as usize;
        let players = (0..count)
            .map(|slot| {
                TflFilmPlayer::parse(
                    slot,
                    &h[Self::PLAYERS + slot * TflFilmPlayer::SIZE..][..TflFilmPlayer::SIZE],
                )
            })
            .collect();
        Ok((
            Self {
                name: c_string(&tag[..0x20]),
                tag_id: FourCC(tag[0x24..0x28].try_into().expect("4 bytes")),
                tag_version: be_u16(&tag, 0x2a),
                tag_word_28: be_u16(&tag, 0x28),
                tag_flags: tag[0x2c..0x30].try_into().expect("4 bytes"),
                tag_user_data: be_u32(&tag, 0x38),
                tag_data_size: be_u32(&tag, 0x34),
                version,
                sub_version: be_u16(&h, 2),
                commands_offset,
                end_tick: be_u32(&h, 0x08),
                block_sizes: TflBlockSizes {
                    saved_game: be_u32(&h, 0x10),
                    initializers: be_u32(&h, 0x18),
                    snapshot: be_u32(&h, 0x20),
                },
                mode: be_u16(&h, 0x24) as i16,
                game_type: be_u16(&h, 0x26) as i16,
                game_options: be_u32(&h, 0x28),
                time_limit: be_u32(&h, 0x2c),
                mesh_tag_id: FourCC(h[0x30..0x34].try_into().expect("4 bytes")),
                difficulty: be_u16(&h, 0x34) as i16,
                max_players: be_u16(&h, 0x36) as i16,
                parameters_14: be_u16(&h, 0x38) as i16,
                team_size: be_u16(&h, 0x3a) as i16,
                initial_seed: be_u32(&h, 0x3c),
                planning_time: be_u32(&h, 0x40),
                parameters_20: h[0x44..0x4c].try_into().expect("8 bytes"),
                local_player: be_u16(&h, 0x4e) as i16,
                players,
                checksums: [be_u32(&h, 0x71c), be_u32(&h, 0x720)],
            },
            commands_start,
        ))
    }

    pub fn parameters(&self) -> TflGameParameters {
        TflGameParameters {
            mode: self.mode,
            game_type: self.game_type,
            options: self.game_options,
            time_limit: self.time_limit,
            mesh: self.mesh_tag_id,
            difficulty: self.difficulty,
            max_players: self.max_players,
            field_14: self.parameters_14,
            team_size: self.team_size,
            seed: self.initial_seed,
            planning_time: self.planning_time,
            field_20: self.parameters_20,
        }
    }

    pub fn new_game(&self) -> TflNewGame {
        TflNewGame {
            parameters: self.parameters(),
            local_player: self.local_player,
            players: self.players.clone(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = tfl_tag_header(
            &crate::mac_roman::encode(&self.name),
            self.tag_id,
            self.tag_version,
            self.tag_word_28,
            self.tag_flags,
            self.tag_data_size,
            self.tag_user_data,
        )
        .to_vec();
        out.extend_from_slice(&tfl_recording_header(
            self.sub_version,
            &self.new_game(),
            self.end_tick,
            self.block_sizes,
            self.checksums,
        ));
        out
    }
}

impl TflRecording {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, M2Error> {
        let mut r = Cursor::new(bytes);
        let (header, start) = TflFilmHeader::read(&mut r)?;
        let tick_commands = read_command_stream(&mut r, start)?;
        Ok(Self {
            header,
            tick_commands,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_myth_ii_film_is_not_a_tfl_film() {
        let mut b = vec![0u8; 64];
        b[36..40].copy_from_slice(b"reco");
        b[60..64].copy_from_slice(b"mth2");
        assert!(!TflFilmHeader::is_tfl_film(&b));
        assert!(TflRecording::from_bytes(&b).is_err());
    }
}
