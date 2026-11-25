//! Myth II recording command deserializer and packet types.

use mandrake_common::M2Error;
use std::io::{Read, Seek, SeekFrom};

use crate::binary::{SliceReader, be_i16, be_u32};

/// Myth II network & replay command types.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[repr(u8)]
pub enum CommandType {
    Null = 0,
    Sync = 1,
    General = 2,
    Movement = 3,
    Target = 4,
    AttackLocation = 5,
    PickUp = 6,
    Rename = 7,
    Rotation = 8,
    DropPlayer = 9,
    Chat = 10,
    Detach = 11,
    UnitAdjustment = 12,
    Sketch = 13,
    Ally = 14,
    PresetSelection = 15,
    SyncFormation = 16,
    Inventory = 17,
    InventoryDrop = 18,
    Unknown(u8),
}

impl CommandType {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Null,
            1 => Self::Sync,
            2 => Self::General,
            3 => Self::Movement,
            4 => Self::Target,
            5 => Self::AttackLocation,
            6 => Self::PickUp,
            7 => Self::Rename,
            8 => Self::Rotation,
            9 => Self::DropPlayer,
            10 => Self::Chat,
            11 => Self::Detach,
            12 => Self::UnitAdjustment,
            13 => Self::Sketch,
            14 => Self::Ally,
            15 => Self::PresetSelection,
            16 => Self::SyncFormation,
            17 => Self::Inventory,
            18 => Self::InventoryDrop,
            other => Self::Unknown(other),
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Sync => 1,
            Self::General => 2,
            Self::Movement => 3,
            Self::Target => 4,
            Self::AttackLocation => 5,
            Self::PickUp => 6,
            Self::Rename => 7,
            Self::Rotation => 8,
            Self::DropPlayer => 9,
            Self::Chat => 10,
            Self::Detach => 11,
            Self::UnitAdjustment => 12,
            Self::Sketch => 13,
            Self::Ally => 14,
            Self::PresetSelection => 15,
            Self::SyncFormation => 16,
            Self::Inventory => 17,
            Self::InventoryDrop => 18,
            Self::Unknown(v) => v,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Null => "NULL",
            Self::Sync => "SYNC",
            Self::General => "GENERAL",
            Self::Movement => "MOVEMENT",
            Self::Target => "TARGET",
            Self::AttackLocation => "ATTACK_LOCATION",
            Self::PickUp => "PICK_UP",
            Self::Rename => "RENAME",
            Self::Rotation => "ROTATION",
            Self::DropPlayer => "DROP_PLAYER",
            Self::Chat => "CHAT",
            Self::Detach => "DETACH",
            Self::UnitAdjustment => "UNIT_ADJUSTMENT",
            Self::Sketch => "SKETCH",
            Self::Ally => "ALLY",
            Self::PresetSelection => "PRESET_SELECTION",
            Self::SyncFormation => "SYNC_FORMATION",
            Self::Inventory => "INVENTORY",
            Self::InventoryDrop => "INVENTORY_DROP",
            Self::Unknown(_) => "UNKNOWN",
        }
    }
}

/// Periodic synchronization packet: the sender's held snapshot `{random seed, tick}` (the tick is
/// a multiple of 512, `mandrake_sim::SYNC_SNAPSHOT_TICKS`). It carries no state hash.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SyncCommand {
    pub flags: u16,
    pub random_seed: u32,
    /// The snapshot's tick (0, 512, 1024, ...).
    #[serde(rename = "state_hash")]
    pub snapshot_tick: u32,
}

/// General game or squad command (e.g. stop, guard, scatter).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GeneralCommand {
    /// 0 Stop, 1 Scatter, 2 Retreat, 4 Hold, ...
    pub action: u16,
    pub unit_ids: Vec<u16>,
}

/// Engine world units per mesh cell: command coordinates, object positions and markers all use
/// this scale.
pub const WORLD_UNITS_PER_CELL: i32 = 512;

/// Maximum waypoints the original engine keeps for a MOVEMENT command.
pub const MAX_WAYPOINTS: usize = 4;

/// MOVEMENT header flag indicating the issuer pre-matched monsters to destinations and appended
/// the formation centre after the waypoint array.
pub const MOVEMENT_FLAG_MATCHED: u16 = 0x80;

/// TARGET header flag indicating the issuer pre-matched attackers to targets in list order.
pub const TARGET_FLAG_MATCHED: u16 = 0x200;

/// Trailing formation centre carried by a MOVEMENT command with [`MOVEMENT_FLAG_MATCHED`],
/// located after the full wire waypoint array (even when the count exceeds [`MAX_WAYPOINTS`]).
pub fn movement_formation_centre(payload: &[u8]) -> Option<(i32, i32)> {
    let mut r = SliceReader::new(payload);
    if r.u16()? & MOVEMENT_FLAG_MATCHED == 0 {
        return None;
    }
    r.skip(6)?;
    let units = r.i16()?.max(0) as usize;
    r.skip(2usize.checked_mul(units)?)?;
    let waypoints = r.i16()?.max(0) as usize;
    r.skip(8usize.checked_mul(waypoints)?)?;
    Some((r.i32()?, r.i32()?))
}

/// One waypoint of a MOVEMENT command (8-byte record in world units, 512 per mesh cell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Waypoint {
    pub x: i32,
    pub y: i32,
    /// Not part of the wire format; always 0 when decoded.
    pub facing_x: i32,
    /// Not part of the wire format; always 0 when decoded.
    pub facing_y: i32,
}

/// Unit movement command containing formation, facing, selected units, and waypoints.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MovementCommand {
    pub flags: u16,
    pub formation: i16,
    pub facing: i16,
    pub unit_ids: Vec<u16>,
    pub waypoints: Vec<Waypoint>,
}

/// Unit attack or target selection command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TargetCommand {
    pub flags: u16,
    pub unit_ids: Vec<u16>,
    pub target_ids: Vec<u16>,
}

/// Ground or location targeted attack command (coordinates in world units, 512 per cell).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttackLocationCommand {
    pub flags: u16,
    pub target_x: i32,
    pub target_y: i32,
    pub target_z: i32,
    pub unit_ids: Vec<u16>,
}

/// Item or projectile pick up command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PickUpCommand {
    pub flags: u16,
    pub object_index: i16,
    pub object_identifier: i16,
    pub unit_ids: Vec<u16>,
}

/// Unit rename command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RenameCommand {
    pub unit_id: u16,
    /// Mac Roman ([`crate::mac_roman`]).
    pub name: String,
}

/// Unit formation rotation command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RotationCommand {
    pub flags: u16,
    pub angle: i16,
    pub unit_ids: Vec<u16>,
}

/// Player drop/disconnect notification command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DropPlayerCommand {
    pub player_index: u8,
    pub reason: u8,
}

/// In-game chat message command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChatCommand {
    pub recipient: u16,
    /// Mac Roman ([`crate::mac_roman`]).
    pub message: String,
}

/// Unit detach / disengage command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DetachCommand {
    pub flags: u16,
    pub unit_ids: Vec<u16>,
}

/// Unit parameter or state adjustment command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnitAdjustmentCommand {
    pub unit_id: u16,
    pub adjustment_type: u16,
    pub value: i32,
}

/// Sketch point on overhead map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SketchPoint {
    pub x: i16,
    pub y: i16,
}

/// Tactical overhead map sketch command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SketchCommand {
    pub color: u16,
    pub points: Vec<SketchPoint>,
}

/// Alliance toggle between two teams.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AllyCommand {
    pub team: i16,
    pub other_team: i16,
}

/// Preset unit group selection shortcut command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PresetSelectionCommand {
    pub preset_index: u16,
    pub unit_ids: Vec<u16>,
}

/// Formation parameters synchronization command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SyncFormationCommand {
    pub parameters: Vec<i16>,
}

/// Unit inventory action command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InventoryCommand {
    pub action: u16,
    pub unit_ids: Vec<u16>,
}

/// Unit inventory drop item command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InventoryDropCommand {
    pub unit_id: u16,
    pub slot: u16,
}

/// Unparsed or fallback command payload.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawCommand {
    pub command_type: u8,
    pub data: Vec<u8>,
}

/// Strongly-typed command payload enum.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CommandPayload {
    Null,
    Sync(SyncCommand),
    General(GeneralCommand),
    Movement(MovementCommand),
    Target(TargetCommand),
    AttackLocation(AttackLocationCommand),
    PickUp(PickUpCommand),
    Rename(RenameCommand),
    Rotation(RotationCommand),
    DropPlayer(DropPlayerCommand),
    Chat(ChatCommand),
    Detach(DetachCommand),
    UnitAdjustment(UnitAdjustmentCommand),
    Sketch(SketchCommand),
    Ally(AllyCommand),
    PresetSelection(PresetSelectionCommand),
    SyncFormation(SyncFormationCommand),
    Inventory(InventoryCommand),
    InventoryDrop(InventoryDropCommand),
    Unknown(RawCommand),
}

/// Single command packet occurring at a specific tick timestamp.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommandPacket {
    pub tick: u32,
    pub player_index: u8,
    pub command_type: CommandType,
    pub payload: CommandPayload,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub raw_payload: Vec<u8>,
}

/// Group of commands occurring at the exact same simulation tick.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TickCommands {
    pub tick: u32,
    pub commands: Vec<CommandPacket>,
}

impl TickCommands {
    /// A stream's commands grouped by tick in their order, as a film's blocks keep them (a tick
    /// that comes back after another is a group of its own).
    pub fn group(commands: &[CommandPacket]) -> Vec<TickCommands> {
        let mut out: Vec<TickCommands> = Vec::new();
        for c in commands {
            match out.last_mut() {
                Some(t) if t.tick == c.tick => t.commands.push(c.clone()),
                _ => out.push(TickCommands {
                    tick: c.tick,
                    commands: vec![c.clone()],
                }),
            }
        }
        out
    }
}

fn parse_payload(command_type: CommandType, payload_bytes: &[u8]) -> Option<CommandPayload> {
    let mut r = SliceReader::new(payload_bytes);
    Some(match command_type {
        CommandType::Null => CommandPayload::Null,
        CommandType::Sync => {
            let flags = r.u16()?;
            r.skip(2)?;
            CommandPayload::Sync(SyncCommand {
                flags,
                random_seed: r.u32()?,
                snapshot_tick: r.u32()?,
            })
        }
        CommandType::General => {
            r.skip(2)?;
            let action = r.u16()?;
            let count = r.i16()?;
            CommandPayload::General(GeneralCommand {
                action,
                unit_ids: r.u16_vec(count),
            })
        }
        CommandType::Movement => {
            let flags = r.u16()?;
            let formation = r.i16()?;
            let facing = r.i16()?;
            r.skip(2)?;
            let unit_ids = r.u16_vec_opt();
            let wp_count = r.bounded_count(8).unwrap_or(0).min(MAX_WAYPOINTS);
            let mut waypoints = Vec::with_capacity(wp_count);
            for _ in 0..wp_count {
                waypoints.push(Waypoint {
                    x: r.i32()?,
                    y: r.i32()?,
                    facing_x: 0,
                    facing_y: 0,
                });
            }
            CommandPayload::Movement(MovementCommand {
                flags,
                formation,
                facing,
                unit_ids,
                waypoints,
            })
        }
        CommandType::Target => {
            let flags = r.u16()?;
            let unit_count = r.i16()?;
            let unit_ids = r.u16_vec(unit_count);
            let target_ids = r.u16_vec_opt();
            CommandPayload::Target(TargetCommand {
                flags,
                unit_ids,
                target_ids,
            })
        }
        CommandType::AttackLocation => {
            let flags = r.u16()?;
            r.skip(2)?;
            let target_x = r.i32()?;
            let target_y = r.i32()?;
            let target_z = r.i32()?;
            let count = r.i16()?;
            CommandPayload::AttackLocation(AttackLocationCommand {
                flags,
                target_x,
                target_y,
                target_z,
                unit_ids: r.u16_vec(count),
            })
        }
        CommandType::PickUp => {
            let flags = r.u16()?;
            r.skip(2)?;
            let object_index = r.i16()?;
            let object_identifier = r.i16()?;
            let count = r.i16()?;
            CommandPayload::PickUp(PickUpCommand {
                flags,
                object_index,
                object_identifier,
                unit_ids: r.u16_vec(count),
            })
        }
        CommandType::Rename => {
            let unit_id = r.u16()?;
            let len = (r.u16()? as usize).min(r.remaining());
            let name = crate::mac_roman::decode(r.take(len)?);
            CommandPayload::Rename(RenameCommand { unit_id, name })
        }
        CommandType::Rotation => {
            let flags = r.u16()?;
            let angle = r.i16()?;
            let count = r.i16()?;
            CommandPayload::Rotation(RotationCommand {
                flags,
                angle,
                unit_ids: r.u16_vec(count),
            })
        }
        CommandType::DropPlayer => CommandPayload::DropPlayer(DropPlayerCommand {
            player_index: r.u8()?,
            reason: r.u8()?,
        }),
        CommandType::Chat => {
            let recipient = r.u16()?;
            let len = (r.u16()? as usize).min(r.remaining());
            let message = crate::mac_roman::decode(r.take(len)?);
            CommandPayload::Chat(ChatCommand { recipient, message })
        }
        CommandType::Detach => {
            let flags = r.u16()?;
            let count = r.i16()?;
            CommandPayload::Detach(DetachCommand {
                flags,
                unit_ids: r.u16_vec(count),
            })
        }
        CommandType::UnitAdjustment => CommandPayload::UnitAdjustment(UnitAdjustmentCommand {
            unit_id: r.u16()?,
            adjustment_type: r.u16()?,
            value: r.i32()?,
        }),
        CommandType::Sketch => {
            let color = r.u16()?;
            let count = r.bounded_count(4)?;
            let mut points = Vec::with_capacity(count);
            for _ in 0..count {
                points.push(SketchPoint {
                    x: r.i16()?,
                    y: r.i16()?,
                });
            }
            CommandPayload::Sketch(SketchCommand { color, points })
        }
        CommandType::Ally => {
            r.skip(4)?;
            CommandPayload::Ally(AllyCommand {
                team: r.i16()?,
                other_team: r.i16()?,
            })
        }
        CommandType::PresetSelection => {
            let preset_index = r.u16()?;
            let count = r.i16()?;
            CommandPayload::PresetSelection(PresetSelectionCommand {
                preset_index,
                unit_ids: r.u16_vec(count),
            })
        }
        CommandType::SyncFormation => {
            let count = r.remaining() / 2;
            let mut parameters = Vec::with_capacity(count);
            for _ in 0..count {
                parameters.push(r.i16()?);
            }
            CommandPayload::SyncFormation(SyncFormationCommand { parameters })
        }
        CommandType::Inventory => {
            let action = r.u16()?;
            let count = r.i16()?;
            CommandPayload::Inventory(InventoryCommand {
                action,
                unit_ids: r.u16_vec(count),
            })
        }
        CommandType::InventoryDrop => CommandPayload::InventoryDrop(InventoryDropCommand {
            unit_id: r.u16()?,
            slot: r.u16()?,
        }),
        CommandType::Unknown(_) => return None,
    })
}

impl CommandPacket {
    /// Deserialize a command packet from a byte slice.
    pub fn parse(tick: u32, player_index: u8, cmd_type_byte: u8, payload_bytes: &[u8]) -> Self {
        let command_type = CommandType::from_u8(cmd_type_byte);
        let payload = parse_payload(command_type, payload_bytes).unwrap_or_else(|| {
            CommandPayload::Unknown(RawCommand {
                command_type: cmd_type_byte,
                data: payload_bytes.to_vec(),
            })
        });

        Self {
            tick,
            player_index,
            command_type,
            payload,
            raw_payload: payload_bytes.to_vec(),
        }
    }
}

/// Maximum size of a single command block in the film stream.
const MAX_COMMAND_BLOCK_BYTES: i16 = 0x800;

/// Read all command packets from stream starting at given offset and group by tick.
pub fn read_command_stream<R: Read + Seek>(
    reader: &mut R,
    start_offset: u64,
) -> Result<Vec<TickCommands>, M2Error> {
    reader.seek(SeekFrom::Start(start_offset))?;

    let mut all_packets = Vec::new();
    let mut chunk_storage = [0u8; MAX_COMMAND_BLOCK_BYTES as usize];

    loop {
        let block_offset = reader.stream_position()?;
        let mut chunk_header = [0u8; 8];
        if let Err(e) = reader.read_exact(&mut chunk_header) {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                break;
            }
            return Err(M2Error::Io(e));
        }

        let flags = be_u32(&chunk_header, 0);
        let chunk_len = be_i16(&chunk_header, 4);
        let cmd_count = be_i16(&chunk_header, 6);

        if !(0..=MAX_COMMAND_BLOCK_BYTES).contains(&chunk_len) {
            return Err(M2Error::ReplayParse(format!(
                "command block at offset {block_offset} has size {chunk_len}"
            )));
        }

        let chunk_buf = &mut chunk_storage[..chunk_len as usize];
        if let Err(e) = reader.read_exact(chunk_buf) {
            // Preserve earlier blocks on mid-block EOF, matching the original engine.
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                break;
            }
            return Err(M2Error::Io(e));
        }

        let mut cpos = 0;
        for _ in 0..cmd_count {
            let cmd_len = chunk_buf
                .get(cpos..cpos + 2)
                .map(|size| i16::from_be_bytes([size[0], size[1]]) as usize);
            let Some(cmd_len) = cmd_len
                .filter(|&len| (8..=MAX_COMMAND_BLOCK_BYTES as usize).contains(&len))
                .filter(|&len| cpos + len <= chunk_buf.len())
            else {
                return Err(M2Error::ReplayParse(format!(
                    "command block at offset {block_offset}: bad command at byte {cpos}"
                )));
            };

            let cmd_type_byte = chunk_buf[cpos + 2];
            let player_index = chunk_buf[cpos + 3];
            let tick = be_u32(chunk_buf, cpos + 4);
            let payload_bytes = &chunk_buf[cpos + 8..cpos + cmd_len];

            let packet = CommandPacket::parse(tick, player_index, cmd_type_byte, payload_bytes);
            all_packets.push(packet);

            cpos += cmd_len;
        }

        // Bit 0 of the block's first word marks the film's last block.
        if (flags & 1) != 0 {
            break;
        }
    }

    // Group packets by simulation tick
    all_packets.sort_by_key(|p| p.tick);

    let mut tick_commands = Vec::new();
    let mut current_tick: Option<u32> = None;
    let mut current_group = Vec::new();

    for packet in all_packets {
        if let Some(t) = current_tick {
            if t == packet.tick {
                current_group.push(packet);
            } else {
                tick_commands.push(TickCommands {
                    tick: t,
                    commands: std::mem::take(&mut current_group),
                });
                current_tick = Some(packet.tick);
                current_group.push(packet);
            }
        } else {
            current_tick = Some(packet.tick);
            current_group.push(packet);
        }
    }

    if let Some(t) = current_tick
        && !current_group.is_empty()
    {
        tick_commands.push(TickCommands {
            tick: t,
            commands: current_group,
        });
    }

    Ok(tick_commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ally_has_the_two_teams_at_4_and_6() {
        let p = CommandPacket::parse(729, 2, 14, &[0, 0, 0, 0, 0, 2, 0, 1]);
        assert_eq!(
            p.payload,
            CommandPayload::Ally(AllyCommand {
                team: 2,
                other_team: 1
            })
        );
        let short = CommandPacket::parse(729, 2, 14, &[0, 0, 0, 0, 0, 2]);
        assert!(matches!(short.payload, CommandPayload::Unknown(_)));
    }

    #[test]
    fn pick_up_has_the_object_and_the_list() {
        let raw = [0, 0, 0, 0, 0x03, 0xa4, 0x55, 0xce, 0, 1, 0x27, 0x1c];
        assert_eq!(
            CommandPacket::parse(40913, 0, 6, &raw).payload,
            CommandPayload::PickUp(PickUpCommand {
                flags: 0,
                object_index: 932,
                object_identifier: 21966,
                unit_ids: vec![10012],
            })
        );
    }

    #[test]
    fn general_has_two_words_then_the_list() {
        let parse = |p: &[u8]| CommandPacket::parse(3307, 0, 2, p).payload;
        let stop = [0, 0, 0, 0, 0, 2, 0x27, 0x11, 0x27, 0x18];
        assert_eq!(
            parse(&stop),
            CommandPayload::General(GeneralCommand {
                action: 0,
                unit_ids: vec![10001, 10008]
            })
        );
        let kind3 = [0, 0, 0, 3, 0, 2, 0x27, 0x65, 0x27, 0x64];
        assert_eq!(
            parse(&kind3),
            CommandPayload::General(GeneralCommand {
                action: 3,
                unit_ids: vec![10085, 10084]
            })
        );
        // Negative count decodes to no units.
        let negative = [0, 0, 0, 0, 0xff, 0xff, 0x27, 0x11];
        assert_eq!(
            parse(&negative),
            CommandPayload::General(GeneralCommand {
                action: 0,
                unit_ids: vec![]
            })
        );
    }

    /// flags, formation, facing, pad; a list of 2; `count` points (their values 1..); a centre.
    fn movement(flags: u16, count: i16) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&flags.to_be_bytes());
        p.extend_from_slice(&[0; 6]);
        p.extend_from_slice(&2i16.to_be_bytes());
        p.extend_from_slice(&[0x27, 0x10, 0x27, 0x11]);
        p.extend_from_slice(&count.to_be_bytes());
        for i in 0..count {
            p.extend_from_slice(&i32::from(i).to_be_bytes());
            p.extend_from_slice(&i32::from(i).to_be_bytes());
        }
        p.extend_from_slice(&1000i32.to_be_bytes());
        p.extend_from_slice(&2000i32.to_be_bytes());
        p
    }

    #[test]
    fn the_formation_centre_follows_the_whole_array() {
        assert_eq!(
            movement_formation_centre(&movement(0x88, 1)),
            Some((1000, 2000))
        );
        // Even with more waypoints than MAX_WAYPOINTS, the trailing centre follows the full array.
        assert_eq!(
            movement_formation_centre(&movement(0x88, 6)),
            Some((1000, 2000))
        );
        assert_eq!(movement_formation_centre(&movement(0x08, 1)), None);
        let mut short = movement(0x88, 1);
        short.truncate(short.len() - 4);
        assert_eq!(movement_formation_centre(&short), None);
    }

    #[test]
    fn huge_and_negative_counts_are_bounded_without_over_allocation() {
        // Sketch with count = 0x7fff and 0xffff on a 4-byte payload.
        for count in [0x7fffu16, 0xffffu16] {
            let mut raw = vec![0, 1];
            raw.extend_from_slice(&count.to_be_bytes());
            let pkt = CommandPacket::parse(1, 0, CommandType::Sketch.to_u8(), &raw);
            let CommandPayload::Sketch(s) = pkt.payload else {
                panic!("expected Sketch");
            };
            assert!(s.points.is_empty());
            assert_eq!(s.points.capacity(), 0);
        }

        // Target with huge positive counts on minimal payload.
        let pkt = CommandPacket::parse(
            1,
            0,
            CommandType::Target.to_u8(),
            &[0, 0, 0x7f, 0xff, 0x7f, 0xff],
        );
        let CommandPayload::Target(t) = pkt.payload else {
            panic!("expected Target");
        };
        assert!(t.unit_ids.capacity() <= 2);
        assert!(t.target_ids.capacity() <= 2);

        // Negative unit count (-1) in Target must not swallow target_ids.
        let pkt = CommandPacket::parse(
            1,
            0,
            CommandType::Target.to_u8(),
            &[0, 0, 0xff, 0xff, 0, 1, 0x27, 0x10],
        );
        let CommandPayload::Target(t) = pkt.payload else {
            panic!("expected Target");
        };
        assert!(t.unit_ids.is_empty());
        assert_eq!(t.target_ids, vec![0x2710]);

        // Negative unit count (-1) in Movement must not swallow waypoints.
        let mut mv = vec![0u8; 8];
        mv.extend_from_slice(&(-1i16).to_be_bytes());
        mv.extend_from_slice(&1i16.to_be_bytes());
        mv.extend_from_slice(&1234i32.to_be_bytes());
        mv.extend_from_slice(&5678i32.to_be_bytes());
        let pkt = CommandPacket::parse(1, 0, CommandType::Movement.to_u8(), &mv);
        let CommandPayload::Movement(m) = pkt.payload else {
            panic!("expected Movement");
        };
        assert!(m.unit_ids.is_empty());
        assert_eq!(m.waypoints.len(), 1);
        assert_eq!((m.waypoints[0].x, m.waypoints[0].y), (1234, 5678));

        // All u16-list commands bound capacity to remaining bytes.
        let cases: &[(CommandType, usize)] = &[
            (CommandType::General, 4),
            (CommandType::AttackLocation, 16),
            (CommandType::PickUp, 8),
            (CommandType::Rotation, 4),
            (CommandType::Detach, 2),
            (CommandType::PresetSelection, 2),
            (CommandType::Inventory, 2),
        ];
        for &(kind, prefix_len) in cases {
            let mut raw = vec![0u8; prefix_len];
            raw.extend_from_slice(&0x7fffu16.to_be_bytes());
            raw.extend_from_slice(&0x1234u16.to_be_bytes());
            let pkt = CommandPacket::parse(1, 0, kind.to_u8(), &raw);
            let ids = match pkt.payload {
                CommandPayload::General(c) => c.unit_ids,
                CommandPayload::AttackLocation(c) => c.unit_ids,
                CommandPayload::PickUp(c) => c.unit_ids,
                CommandPayload::Rotation(c) => c.unit_ids,
                CommandPayload::Detach(c) => c.unit_ids,
                CommandPayload::PresetSelection(c) => c.unit_ids,
                CommandPayload::Inventory(c) => c.unit_ids,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(ids, vec![0x1234], "{kind:?}");
            assert!(ids.capacity() <= 4, "{kind:?}: cap {}", ids.capacity());
        }
    }
}
