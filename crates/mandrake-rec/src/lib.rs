//! Myth II recording (`.rec` / `.m2rec`) parser and command deserializer, and TFL's films
//! beside it ([`tfl`]).

pub(crate) mod binary;
pub mod command;
pub mod header;
#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;
pub use mandrake_common::mac_roman;
pub mod saved_game;
pub mod tfl;
pub mod writer;

pub use command::{
    AllyCommand, AttackLocationCommand, ChatCommand, CommandPacket, CommandPayload, CommandType,
    DetachCommand, DropPlayerCommand, GeneralCommand, InventoryCommand, InventoryDropCommand,
    MAX_WAYPOINTS, MOVEMENT_FLAG_MATCHED, MovementCommand, PickUpCommand, PresetSelectionCommand,
    RawCommand, RenameCommand, RotationCommand, SketchCommand, SketchPoint, SyncCommand,
    SyncFormationCommand, TARGET_FLAG_MATCHED, TargetCommand, TickCommands, UnitAdjustmentCommand,
    WORLD_UNITS_PER_CELL, Waypoint, movement_formation_centre,
};
pub use header::{MonsterInitializer, PlayerInfo, PluginInfo, RecordingHeader};

use mandrake_common::M2Error;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek};

/// Complete parsed Myth II recording file containing header metadata and command stream.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Recording {
    pub header: RecordingHeader,
    pub tick_commands: Vec<TickCommands>,
}

impl Recording {
    /// Parse a Myth II recording from any seekable reader.
    pub fn open<R: Read + Seek>(reader: &mut R) -> Result<Self, M2Error> {
        let (header, commands_offset) = RecordingHeader::read(reader)?;
        let mut tick_commands = command::read_command_stream(reader, commands_offset)?;
        if header.is_saved_game() {
            // The original engine stops replaying a saved game's film at its end tick.
            tick_commands.retain(|tc| tc.tick <= header.end_tick);
        }

        Ok(Self {
            header,
            tick_commands,
        })
    }

    /// Parse a recording from an in-memory byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, M2Error> {
        let mut cursor = Cursor::new(bytes);
        Self::open(&mut cursor)
    }

    /// Total number of commands across all simulation ticks.
    pub fn total_command_count(&self) -> usize {
        self.tick_commands.iter().map(|tc| tc.commands.len()).sum()
    }

    /// Breakdown of command counts grouped by command type.
    pub fn command_count_by_type(&self) -> BTreeMap<CommandType, usize> {
        let mut counts = BTreeMap::new();
        for tc in &self.tick_commands {
            for cmd in &tc.commands {
                *counts.entry(cmd.command_type).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Iterator over every individual command packet in tick order.
    pub fn iter_commands(&self) -> impl Iterator<Item = &CommandPacket> {
        self.tick_commands.iter().flat_map(|tc| &tc.commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_common::FourCC;

    /// A film: tag header, recording header, `block`, then a block of `commands`, each 8 bytes
    /// plus its payload. The last block's end flag is set.
    fn film(block: &[u8], commands: &[(u32, u8, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (tick, kind, payload) in commands {
            body.extend_from_slice(&(8 + payload.len() as i16).to_be_bytes());
            body.extend_from_slice(&[*kind, 0]);
            body.extend_from_slice(&tick.to_be_bytes());
            body.extend_from_slice(payload);
        }
        let mut f = vec![0u8; 64 + 2600];
        f[36..40].copy_from_slice(b"reco");
        f[44..48].copy_from_slice(&64u32.to_be_bytes());
        f[64..66].copy_from_slice(&3u16.to_be_bytes());
        f[68..72].copy_from_slice(&(2600 + block.len() as u32).to_be_bytes());
        f[64 + 0x14..64 + 0x18].copy_from_slice(&2600u32.to_be_bytes());
        f[64 + 0x18..64 + 0x1c].copy_from_slice(&(block.len() as u32).to_be_bytes());
        f.extend_from_slice(block);
        f.extend_from_slice(&1u32.to_be_bytes());
        f.extend_from_slice(&(body.len() as i16).to_be_bytes());
        f.extend_from_slice(&(commands.len() as i16).to_be_bytes());
        f.extend_from_slice(&body);
        f
    }

    fn initializer_record() -> [u8; 40] {
        let mut r = [0u8; 40];
        r[0..4].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
        r[4..8].copy_from_slice(&13u32.to_be_bytes());
        r[8..12].copy_from_slice(b"enfg");
        r[12..16].copy_from_slice(b"ent ");
        r[16..20].copy_from_slice(&[4, 1, 2, 7]);
        r[22..26].copy_from_slice(b"Xeno");
        r
    }

    /// The block before the commands is skipped by the command offset and parsed as records.
    #[test]
    fn the_block_before_the_commands_is_monster_initializers() {
        let record = initializer_record();
        let f = film(&[record, record].concat(), &[(7, 1, &[])]);
        let rec = Recording::from_bytes(&f).unwrap();
        assert_eq!(rec.tick_commands.len(), 1);
        assert_eq!(rec.tick_commands[0].tick, 7);
        assert_eq!(rec.header.monster_initializers.len(), 2);
        assert_eq!(
            rec.header.monster_initializers[0],
            MonsterInitializer {
                player_id: 13,
                unit_tag: FourCC(*b"enfg"),
                monster_tag: FourCC(*b"ent "),
                experience: 4,
                field_8f_plus_one: 1,
                permutation: 2,
                name_index: 7,
                name: "Xeno".to_string(),
            }
        );
        let plain = Recording::from_bytes(&film(&[], &[(7, 1, &[])])).unwrap();
        assert!(plain.header.monster_initializers.is_empty());
        assert_eq!(plain.tick_commands, rec.tick_commands);
    }

    fn replay_error(f: &[u8]) -> String {
        match Recording::from_bytes(f) {
            Err(M2Error::ReplayParse(why)) => why,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    /// Malformed films return errors rather than panicking.
    #[test]
    fn a_malformed_film_is_an_error() {
        let good = film(&initializer_record(), &[(7, 1, &[0; 4])]);
        assert!(Recording::from_bytes(&good).is_ok());
        let set = |at: usize, bytes: &[u8]| {
            let mut f = good.clone();
            f[at..at + bytes.len()].copy_from_slice(bytes);
            f
        };
        let (size_word, commands_word) = (64 + 0x18, 64 + 0x04);
        let first_block = 64 + 2600 + 40;

        // The block: not whole records, past the end of the film, larger than the engine's table.
        assert!(replay_error(&set(size_word, &39u32.to_be_bytes())).contains("initializer"));
        assert!(replay_error(&set(size_word, &40_000u32.to_be_bytes())).contains("initializer"));
        assert!(
            replay_error(&set(size_word, &0xffff_ffd8u32.to_be_bytes())).contains("initializer")
        );
        let mut far = set(size_word, &40u32.to_be_bytes());
        far[64 + 0x14..64 + 0x18].copy_from_slice(&0xffff_ff00u32.to_be_bytes());
        assert!(replay_error(&far).contains("initializer"));

        // The command offset: inside the header, past the end of the film.
        assert!(replay_error(&set(commands_word, &0u32.to_be_bytes())).contains("offset"));
        assert!(
            replay_error(&set(commands_word, &0xffff_ffffu32.to_be_bytes())).contains("offset")
        );

        // A command block's size: negative (the capacity overflow), or over the engine's buffer.
        assert!(replay_error(&set(first_block + 4, &(-1i16).to_be_bytes())).contains("size -1"));
        assert!(replay_error(&set(first_block + 4, &0x801i16.to_be_bytes())).contains("size 2049"));

        // A command's size: negative (the add overflow), too short, past its block. Or too many.
        for size in [-1i16, 0, 7, 13] {
            let f = set(first_block + 8, &size.to_be_bytes());
            assert!(replay_error(&f).contains("bad command"), "size {size}");
        }
        assert!(replay_error(&set(first_block + 6, &2i16.to_be_bytes())).contains("bad command"));
    }

    /// A plugin name with no NUL that runs to the list's end leaves no room for its URL.
    #[test]
    fn a_plugin_name_up_to_the_list_end_reads() {
        let mut f = film(&[], &[(7, 1, &[])]);
        f[64 + 0x50..64 + 0x52].copy_from_slice(&16u16.to_be_bytes());
        f[64 + 0x52..64 + 0x250].fill(b'a');
        // A shorter first URL, so that the sixth record's name meets the end of the list.
        f[64 + 0x52 + 4 + 33 + 54] = 0;
        let rec = Recording::from_bytes(&f).unwrap();
        assert_eq!(rec.header.active_plugins.len(), 6);
        assert_eq!(rec.header.active_plugins[5].name, "aaaaaa");
    }

    /// Negative command counts are treated as 0, and bit 0 of the first word marks the last block.
    #[test]
    fn block_count_and_end_flag_read_as_the_engine_does() {
        let good = film(&[], &[(7, 1, &[])]);
        let first_block = 64 + 2600;
        let mut none = good.clone();
        none[first_block + 6..first_block + 8].copy_from_slice(&(-1i16).to_be_bytes());
        assert!(
            Recording::from_bytes(&none)
                .unwrap()
                .tick_commands
                .is_empty()
        );

        // Not the last block: the one after it is read too, and nothing after the flagged one.
        let mut two = good.clone();
        two[first_block + 3] = 0;
        two.extend_from_slice(&[0, 0, 0, 1, 0, 8, 0, 1, 0, 8, 1, 0, 0, 0, 0, 9]);
        two.extend_from_slice(&[0xff; 12]);
        let ticks = Recording::from_bytes(&two).unwrap();
        let ticks: Vec<u32> = ticks.tick_commands.iter().map(|t| t.tick).collect();
        assert_eq!(ticks, vec![7, 9]);
    }

    /// Film files are untrusted: any bytes give a recording or an error, never a panic. Each
    /// command kind gets random payloads, then the whole film gets random damage.
    #[test]
    fn damaged_films_never_panic() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let record = initializer_record();
        for round in 0..6000 {
            let payloads: Vec<Vec<u8>> = (0..8)
                .map(|_| (0..next() % 48).map(|_| next() as u8).collect())
                .collect();
            let commands: Vec<(u32, u8, &[u8])> = payloads
                .iter()
                .enumerate()
                .map(|(i, p)| (i as u32, (round + i) as u8 % 27, p.as_slice()))
                .collect();
            let mut f = film(&record, &commands);
            let _ = Recording::from_bytes(&f);
            for _ in 0..next() % 16 {
                let at = next() as usize % f.len();
                // Half the damage goes to the words the readers trust.
                let at = if next() % 2 == 0 { at % 800 } else { at };
                f[at] = [0, 0xff, 0x7f, 0x80, next() as u8][next() as usize % 5];
            }
            let _ = Recording::from_bytes(&f);
            let _ = Recording::from_bytes(&f[..next() as usize % f.len()]);
        }
    }

    #[test]
    fn test_recording_tag_type() {
        assert_eq!(RecordingHeader::TAG_TYPE.to_string(), "reco");
    }

    #[test]
    fn test_command_type_roundtrip() {
        for code in 0..=18 {
            let cmd_type = CommandType::from_u8(code);
            assert_eq!(cmd_type.to_u8(), code);
            assert_ne!(cmd_type.name(), "UNKNOWN");
        }
    }

    #[test]
    fn test_invalid_magic() {
        let bad_data = vec![0u8; 128];
        let mut cursor = Cursor::new(bad_data);
        let res = Recording::open(&mut cursor);
        assert!(res.is_err());
    }

    #[test]
    fn test_synthetic_command_parsing() {
        // Build synthetic chat packet
        let mut chat_payload = Vec::new();
        chat_payload.extend_from_slice(&1u16.to_be_bytes()); // recipient = 1
        let msg = b"Attack now!";
        chat_payload.extend_from_slice(&(msg.len() as u16).to_be_bytes());
        chat_payload.extend_from_slice(msg);

        let packet = CommandPacket::parse(100, 0, CommandType::Chat.to_u8(), &chat_payload);
        assert_eq!(packet.tick, 100);
        assert_eq!(packet.player_index, 0);
        assert_eq!(packet.command_type, CommandType::Chat);
        if let CommandPayload::Chat(chat) = packet.payload {
            assert_eq!(chat.recipient, 1);
            assert_eq!(chat.message, "Attack now!");
        } else {
            panic!("Expected Chat payload");
        }

        // Build synthetic rotation packet
        let mut rot_payload = Vec::new();
        rot_payload.extend_from_slice(&0u16.to_be_bytes()); // flags = 0
        rot_payload.extend_from_slice(&256i16.to_be_bytes()); // angle = 256
        rot_payload.extend_from_slice(&2u16.to_be_bytes()); // unit count = 2
        rot_payload.extend_from_slice(&101u16.to_be_bytes()); // unit 1
        rot_payload.extend_from_slice(&102u16.to_be_bytes()); // unit 2

        let packet = CommandPacket::parse(200, 1, CommandType::Rotation.to_u8(), &rot_payload);
        assert_eq!(packet.tick, 200);
        assert_eq!(packet.player_index, 1);
        assert_eq!(packet.command_type, CommandType::Rotation);
        if let CommandPayload::Rotation(rot) = packet.payload {
            assert_eq!(rot.angle, 256);
            assert_eq!(rot.unit_ids, vec![101, 102]);
        } else {
            panic!("Expected Rotation payload");
        }
    }
}
