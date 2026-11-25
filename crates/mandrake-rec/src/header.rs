//! Myth II recording header deserializer and metadata types.

use mandrake_common::{FourCC, M2Error};
use std::io::{Read, Seek, SeekFrom};

use crate::binary::{be_i16, be_u16, be_u32, c_string, c_string_trimmed};
use crate::writer::{NEW_GAME_DATA_OFFSET, TAG_HEADER_SIZE};

/// Active plugin metadata stored in recording header.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    /// The name and URL are Mac Roman ([`crate::mac_roman`]), so they encode back to their bytes.
    pub name: String,
    pub url: String,
    pub checksum: u32,
}

/// Player roster information stored in recording header.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayerInfo {
    #[serde(rename = "id")]
    pub team: i16,
    #[serde(rename = "team")]
    pub field_02: i16,
    /// Mac Roman ([`crate::mac_roman`]): `mac_roman::encode` gives back its bytes.
    pub name: String,
}

/// One 40-byte record of a film's monster initializer block (a named or experienced unit kept
/// when a game ended for the next game). The leading word is run-time state cleared on import.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MonsterInitializer {
    pub player_id: u32,
    pub unit_tag: FourCC,
    pub monster_tag: FourCC,
    pub experience: u8,
    pub field_8f_plus_one: u8,
    pub permutation: u8,
    pub name_index: u8,
    /// Custom name (up to 18 bytes, NUL-terminated Mac Roman).
    pub name: String,
}

impl MonsterInitializer {
    pub const SIZE: usize = 40;
    pub const MAX_BLOCK_BYTES: u32 = 32000;

    fn parse(r: &[u8; Self::SIZE]) -> Self {
        let word = |at: usize| [r[at], r[at + 1], r[at + 2], r[at + 3]];
        Self {
            player_id: be_u32(r, 0x04),
            unit_tag: FourCC(word(0x08)),
            monster_tag: FourCC(word(0x0c)),
            experience: r[0x10],
            field_8f_plus_one: r[0x11],
            permutation: r[0x12],
            name_index: r[0x13],
            name: c_string(&r[0x16..Self::SIZE]),
        }
    }
}

fn read_monster_initializers<R: Read + Seek>(
    reader: &mut R,
    data_offset: u64,
    offset: u32,
    size: u32,
    file_len: u64,
) -> Result<Vec<MonsterInitializer>, M2Error> {
    if size == 0 {
        return Ok(Vec::new());
    }
    let start = data_offset + u64::from(offset);
    if size > MonsterInitializer::MAX_BLOCK_BYTES
        || !(size as usize).is_multiple_of(MonsterInitializer::SIZE)
        || start + u64::from(size) > file_len
    {
        return Err(M2Error::ReplayParse(format!(
            "bad monster initializer block: {size} bytes at offset {start} of a {file_len}-byte film"
        )));
    }
    reader.seek(SeekFrom::Start(start))?;
    let mut block = vec![0u8; size as usize];
    reader.read_exact(&mut block)?;
    let (records, _) = block.as_chunks::<{ MonsterInitializer::SIZE }>();
    Ok(records.iter().map(MonsterInitializer::parse).collect())
}

/// Start offset of the recording header within a film or saved game: the tag header's data
/// offset (at least 64) when preceded by a `reco`/`save` tag header, 4 for a bare `reco` prefix,
/// or 0 for raw header bytes.
pub fn data_offset(file: &[u8]) -> usize {
    match file.get(36..40) {
        Some(b"reco" | b"save") => file.get(44..48).map_or(TAG_HEADER_SIZE, |w| {
            (u32::from_be_bytes([w[0], w[1], w[2], w[3]]) as usize).max(TAG_HEADER_SIZE)
        }),
        _ if file.starts_with(b"reco") => 4,
        _ => 0,
    }
}

/// The new-game parameters slice of a film or saved game, to the end of `file`.
pub fn game_parameters(file: &[u8]) -> Option<&[u8]> {
    let start = data_offset(file).checked_add(NEW_GAME_DATA_OFFSET)?;
    file.get(start..)
}

/// Header metadata stored at the start of a Myth II recording file.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RecordingHeader {
    pub name: String,
    /// `reco` for a film, `save` for a saved game.
    pub tag_type: FourCC,
    pub version: u16,
    pub sub_version: u16,
    /// 0 replay, 1 network game, 2 solo game.
    #[serde(rename = "game_type")]
    pub mode: i16,
    /// Game type (13 is solo mission, 0 Body Count, ...).
    #[serde(rename = "difficulty")]
    pub game_type: i16,
    pub game_options_flags: u32,
    pub time_limit: u32,
    /// Tick the film ends at; a saved game's replay stops here and live play resumes.
    #[serde(default)]
    pub end_tick: u32,
    pub planning_time: u32,
    pub mesh_tag_id: FourCC,
    pub map_name: String,
    #[serde(default, rename = "build_id")]
    pub version_id: u32,
    pub initial_seed: u32,
    pub seed: u32,
    pub active_plugins: Vec<PluginInfo>,
    pub players: Vec<PlayerInfo>,
    pub teams: Vec<String>,
    #[serde(default)]
    pub monster_initializers: Vec<MonsterInitializer>,
}

impl RecordingHeader {
    pub const TAG_TYPE: FourCC = FourCC(*b"reco");
    pub const SAVE_TAG_TYPE: FourCC = FourCC(*b"save");

    pub fn is_saved_game(&self) -> bool {
        self.tag_type == Self::SAVE_TAG_TYPE
    }

    /// Parse header from a seekable stream, returning the header and the byte offset where
    /// command blocks begin.
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<(Self, u64), M2Error> {
        let mut initial_buf = [0u8; 64];
        if let Err(e) = reader.read_exact(&mut initial_buf) {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                return Err(M2Error::ReplayParse(
                    "Recording file is too short to contain header".to_string(),
                ));
            }
            return Err(M2Error::Io(e));
        }

        let data_offset = data_offset(&initial_buf) as u64;
        let tag_type = match &initial_buf[36..40] {
            b"save" => Self::SAVE_TAG_TYPE,
            _ => Self::TAG_TYPE,
        };
        let recording_name = if data_offset >= TAG_HEADER_SIZE as u64 {
            c_string_trimmed(&initial_buf[4..36])
        } else {
            let ver = be_u16(&initial_buf, 0);
            if data_offset == 0 && !(ver > 0 && ver <= 3) {
                return Err(M2Error::InvalidMagic {
                    expected: "reco".to_string(),
                    found: String::from_utf8_lossy(&initial_buf[36..40]).to_string(),
                });
            }
            "Recording".to_string()
        };

        let file_len = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(data_offset))?;

        let mut rec_buf = [0u8; 2600];
        reader.read_exact(&mut rec_buf)?;

        let version = be_u16(&rec_buf, 0);
        let sub_version = be_u16(&rec_buf, 2);

        if version > 3 {
            return Err(M2Error::ReplayParse(format!(
                "Recording version {version} is too new (max supported is 3)"
            )));
        }

        let commands_offset = be_u32(&rec_buf, 0x04);
        let end_tick = be_u32(&rec_buf, 0x08);
        let initializers_offset = be_u32(&rec_buf, 0x14);
        let initializers_size = be_u32(&rec_buf, 0x18);

        let commands_start_offset = data_offset + u64::from(commands_offset);
        if commands_offset < rec_buf.len() as u32 || commands_start_offset > file_len {
            return Err(M2Error::ReplayParse(format!(
                "bad command offset: {commands_offset} in a {file_len}-byte film"
            )));
        }
        let monster_initializers = read_monster_initializers(
            reader,
            data_offset,
            initializers_offset,
            initializers_size,
            file_len,
        )?;

        let gp = &rec_buf[NEW_GAME_DATA_OFFSET..NEW_GAME_DATA_OFFSET + 0x2c];
        let mode = be_i16(gp, 0);
        let game_type = be_i16(gp, 2);
        let game_options_flags = be_u32(gp, 4);
        let time_limit = be_u32(gp, 8);
        let mesh_tag_id = FourCC([gp[12], gp[13], gp[14], gp[15]]);
        let map_name = mesh_tag_id.as_str().to_string();
        let initial_seed = be_u32(gp, 24);
        let planning_time = be_u32(gp, 0x1c);
        let version_id = be_u32(gp, 0x24);

        let plugin_count = be_u16(&rec_buf, 0x50) as usize;
        let mut active_plugins = Vec::new();
        let mut p_off = 0x52;

        for _ in 0..plugin_count.min(16) {
            if p_off + 4 > 0x250 {
                break;
            }
            let checksum = be_u32(&rec_buf, p_off);
            p_off += 4;

            let name_max = (0x250 - p_off).min(32);
            let raw_pname = &rec_buf[p_off..p_off + name_max];
            let name_len = raw_pname
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(raw_pname.len());
            let plugin_name = crate::mac_roman::decode(&raw_pname[..name_len]);
            p_off += name_len + 1;

            let url_max = 0x250usize.saturating_sub(p_off).min(64);
            let raw_url = &rec_buf[p_off..p_off + url_max];
            let url_len = raw_url
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(raw_url.len());
            let plugin_url = crate::mac_roman::decode(&raw_url[..url_len]);
            p_off += url_len + 1;

            if !plugin_name.is_empty() {
                active_plugins.push(PluginInfo {
                    name: plugin_name,
                    url: plugin_url,
                    checksum,
                });
            }
        }

        let mut players = Vec::new();
        let mut teams = Vec::new();

        for p in 0..16 {
            let slot_off = 0x254 + p * 124;
            let slot = &rec_buf[slot_off..slot_off + 124];

            if slot.iter().all(|&b| b == 0) {
                continue;
            }

            let team = be_i16(slot, 0);
            let field_02 = be_i16(slot, 2);
            let name = c_string_trimmed(&slot[0x2c..0x2c + 64]);

            if !name.is_empty() {
                if !teams.contains(&name) {
                    teams.push(name.clone());
                }
                players.push(PlayerInfo {
                    team,
                    field_02,
                    name,
                });
            }
        }

        Ok((
            Self {
                name: recording_name,
                tag_type,
                version,
                sub_version,
                mode,
                game_type,
                game_options_flags,
                time_limit,
                end_tick,
                planning_time,
                mesh_tag_id,
                map_name,
                version_id,
                initial_seed,
                seed: initial_seed,
                active_plugins,
                players,
                teams,
                monster_initializers,
            },
            commands_start_offset,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_offset_is_the_tag_headers_word_or_the_bare_films_start() {
        let mut tagged = vec![0u8; 0x200];
        tagged[36..40].copy_from_slice(b"save");
        tagged[44..48].copy_from_slice(&0x80u32.to_be_bytes());
        tagged[0x80 + 0x24 + 0x10..][..2].copy_from_slice(&3i16.to_be_bytes());
        assert_eq!(data_offset(&tagged), 0x80);
        assert_eq!(game_parameters(&tagged).unwrap()[0x10..0x12], [0, 3]);
        tagged[44..48].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(data_offset(&tagged), 64, "at least the tag header");
        assert_eq!(data_offset(b"reco\0\x03"), 4);
        assert_eq!(data_offset(&[0, 3, 0, 0]), 0);
    }

    #[test]
    fn game_parameters_handles_max_data_offset() {
        let mut tagged = vec![0u8; 0x100];
        tagged[36..40].copy_from_slice(b"reco");
        tagged[44..48].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(game_parameters(&tagged).is_none());
    }

    #[test]
    fn header_read_handles_chunked_and_interrupted_reader() {
        struct TrickleReader {
            inner: std::io::Cursor<Vec<u8>>,
            interrupt_next: bool,
        }
        impl Read for TrickleReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.interrupt_next {
                    self.interrupt_next = false;
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "interrupted",
                    ));
                }
                self.interrupt_next = true;
                let max = buf.len().min(7);
                self.inner.read(&mut buf[..max])
            }
        }
        impl Seek for TrickleReader {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.inner.seek(pos)
            }
        }

        let mut film = vec![0u8; 64 + 2600];
        film[4..8].copy_from_slice(b"Test");
        film[36..40].copy_from_slice(b"reco");
        film[44..48].copy_from_slice(&64u32.to_be_bytes());
        film[64..66].copy_from_slice(&3u16.to_be_bytes());
        film[68..72].copy_from_slice(&2600u32.to_be_bytes());

        let mut reader = TrickleReader {
            inner: std::io::Cursor::new(film),
            interrupt_next: true,
        };
        let (header, cmd_off) = RecordingHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, "Test");
        assert_eq!(cmd_off, 64 + 2600);

        let mut short = TrickleReader {
            inner: std::io::Cursor::new(vec![0u8; 20]),
            interrupt_next: false,
        };
        assert!(matches!(
            RecordingHeader::read(&mut short),
            Err(M2Error::ReplayParse(_))
        ));
    }
}
