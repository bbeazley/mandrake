//! Myth II monolithic archive and tag deserializers, and TFL's beside them ([`tfl`]).

pub mod archive;
pub mod archive_writer;
pub mod collection;
pub mod connector;
pub mod detail_map;
pub mod dialog;
pub mod font;
pub mod header;
pub mod hud;
pub mod interface;
pub mod map_action;
pub mod media;
pub mod mesh;
pub mod mesh_links;
pub mod model;
pub mod model_anim;
pub mod model_hit;
pub mod object;
pub(crate) mod parse;
pub mod pix0r1;
pub mod projectile;
pub mod scenery;
pub mod sequence_anim;
pub mod sequence_header;
pub mod sound;
pub mod sound_manager;
pub mod stli;
pub mod tag_files;
pub mod tagset;
#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;
pub mod tfl;
pub mod unit;

pub use archive::{ArchiveFormat, ArchiveSource, TagArchive, TagEntry};
pub use archive_writer::{MonolithTag, write_monolith};
pub use collection::{
    BITMAP_FLAG_RAW_COLOR_TABLE, BitmapFrame, CollectionSequence, CollectionTag, CorePermutations,
    HueChange, LandscapeShadingTable, SequenceBounds, SequenceWorldBounds, ShadowDescriptor,
    TerrainTexture, displayed_views, hsv_to_rgb, mirror_view, rgb_to_hsv, view_for_relative_angle,
};
pub use connector::{ConnectorTag, MeshConnector, MeshConnectorList};
pub use detail_map::{DetailMap, DetailMapEntry, DetailTexture};
pub use dialog::{DialogDefinition, DialogItem, Rect, parse_ditl};
pub use font::{
    Bitmap32, CharClass, FontFamily, FontTag, Glyph, LineEnd, RgbColor, TextClip, TextCursor,
    TextLocale, WrappedLine, draw_string, string_width, text_flags, wrap_string,
};
pub use header::{SBEntryTag, SBMonoHeader, SBTagHeader, decode_c_string, encode_c_string};
pub use interface::{InterfaceColorTable, InterfaceDefinition};
pub use map_action::{MapActionParameter, MapActionRecord, MeshMapActions};
pub use mesh::{
    GameMesh, LandscapeShading, MESH_HEIGHT_VOID, MYTH2_MESH_VERSION, MarkerEntry,
    MarkerPaletteEntry, MeshFog, MeshHeightCorner, MeshHeightMode, MeshTag, Myth2Cell,
    UnitPlacement, get_mesh_height,
};
pub use model::{
    GeometryMaterial, GeometrySurface, GeometryTag, GeometryVertex, ModelPermutation, ModelTag,
    SurfaceVertex,
};
pub use model_anim::{ModelAnimationFrame, ModelAnimationPlayback, ModelAnimationTag};
pub use object::ObjectDefinition;
pub use pix0r1::Pix0r1Image;
pub use projectile::{
    DamageDefinition, ProjectileDefinition, ProjectileGroupDefinition, ProjectileGroupPart,
};
pub use scenery::SceneryTag;
pub use sequence_anim::{NORMAL_SPEED, SequenceEvents, SequencePlayback};
pub use sequence_header::{CollectionReference, SequenceHeader, get_sequence, sequence_headers};
pub use sound::{
    AmbientSoundTag, DecodedSound, PermutationHistory, SampledSoundHeader, SoundPermutation,
    SoundTag,
};
pub use sound_manager::{
    AmbientDefinition, AmbientSoundSource, ChannelStatus, SoundChannelBackend, SoundDefinition,
    SoundListener, SoundManager, SoundVolumes, WorldPoint,
};
pub use stli::StringListTag;
pub use tag_files::{
    EngineTagFiles, TagLookup, engine_interface_tag_files, engine_tag_files, open_tfl_tags,
    tfl_tag_files,
};
pub use tfl::Game;
pub use unit::{
    MONSTER_ATTACK_SLOTS, MONSTER_SEQUENCE_SLOTS, MONSTER_SOUND_SLOTS, MonsterSequenceSlot,
    MonsterSoundType, UnitTag, monster_group_sound,
};

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{BigEndian, WriteBytesExt};
    use mandrake_common::{FourCC, MemoryVfs};
    use std::io::Cursor;

    #[test]
    fn test_mono_header_roundtrip() {
        let original = SBMonoHeader {
            r#type: 3,
            version: 0,
            name: encode_c_string("test install"),
            description: encode_c_string("Myth II test archive"),
            entry_tag_count: 2,
            tag_list_count: 5,
            checksum: [1, 2, 3, 4],
            flags: 1,
            size: 1024,
            header_checksum: [5, 6, 7, 8],
            padding: [0; 4],
            signature: SBMonoHeader::SIG_DNG2,
        };

        let mut buf = Vec::new();
        original.write(&mut buf).unwrap();
        assert_eq!(buf.len(), SBMonoHeader::SIZE);

        let mut cursor = Cursor::new(&buf);
        let decoded = SBMonoHeader::read(&mut cursor).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.name_as_str(), "test install");
        assert_eq!(decoded.description_as_str(), "Myth II test archive");
    }

    #[test]
    fn test_tag_header_roundtrip() {
        let original = SBTagHeader {
            identifier: -1,
            flags: 0,
            tag_type_idx: 0,
            name: encode_c_string("amb boat"),
            tag_type: FourCC::new(*b"amso"),
            tag_id: FourCC::new(*b"ambo"),
            tag_data_offset: 214288,
            tag_data_size: 64,
            user_data: 0,
            version: 4,
            destination: -1,
            owner_index: -1,
            signature: SBTagHeader::SIGNATURE,
        };

        let mut buf = Vec::new();
        original.write(&mut buf).unwrap();
        assert_eq!(buf.len(), SBTagHeader::SIZE);

        let mut cursor = Cursor::new(&buf);
        let decoded = SBTagHeader::read(&mut cursor).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.name_as_str(), "amb boat");
        assert_eq!(decoded.tag_type, "amso");
        assert_eq!(decoded.tag_id, "ambo");
    }

    #[test]
    fn test_entry_tag_roundtrip() {
        let original = SBEntryTag {
            tag_id: FourCC::new(*b"00tm"),
            reserved1: [0; 12],
            internal_name: encode_c_string("00 Training Map"),
            display_name: encode_c_string("Tutorial"),
            description: [0; 32],
        };

        let mut buf = Vec::new();
        original.write(&mut buf).unwrap();
        assert_eq!(buf.len(), SBEntryTag::SIZE);

        let mut cursor = Cursor::new(&buf);
        let decoded = SBEntryTag::read(&mut cursor).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.internal_name_as_str(), "00 Training Map");
        assert_eq!(decoded.display_name_as_str(), "Tutorial");
    }

    #[test]
    fn test_synthetic_tag_archive() {
        let tag0_data = b"Hello from Myth II Tag 0 Data Payload!";
        let tag1_data = b"Another chunk of Tag 1 Mesh Geometry Data.";

        let header_size = SBMonoHeader::SIZE; // 128
        let entry_count = 1;
        let entry_size = SBEntryTag::SIZE; // 112
        let tag_count = 2;
        let tag_headers_size = tag_count * SBTagHeader::SIZE; // 128

        let data_start = header_size + entry_count * entry_size + tag_headers_size;
        let tag0_offset = data_start as i32;
        let tag0_size = tag0_data.len() as i32;
        let tag1_offset = tag0_offset + tag0_size;
        let tag1_size = tag1_data.len() as i32;
        let total_size = (tag1_offset + tag1_size) as u32;

        let mono_header = SBMonoHeader {
            r#type: 3,
            version: 0,
            name: encode_c_string("synthetic install"),
            description: encode_c_string("Synthetic test archive"),
            entry_tag_count: entry_count as u16,
            tag_list_count: tag_count as u16,
            checksum: [0; 4],
            flags: 1,
            size: total_size,
            header_checksum: [0; 4],
            padding: [0; 4],
            signature: SBMonoHeader::SIG_DNG2,
        };

        let entry_tag = SBEntryTag {
            tag_id: FourCC::new(*b"00tm"),
            reserved1: [0; 12],
            internal_name: encode_c_string("00 Training Map"),
            display_name: encode_c_string("Tutorial"),
            description: [0; 32],
        };

        let tag0 = SBTagHeader {
            identifier: 0,
            flags: 0,
            tag_type_idx: 0,
            name: encode_c_string("amb boat"),
            tag_type: FourCC::new(*b"amso"),
            tag_id: FourCC::new(*b"ambo"),
            tag_data_offset: tag0_offset,
            tag_data_size: tag0_size,
            user_data: 0,
            version: 1,
            destination: 0,
            owner_index: 0,
            signature: SBTagHeader::SIGNATURE,
        };

        let tag1 = SBTagHeader {
            identifier: 1,
            flags: 0,
            tag_type_idx: 0,
            name: encode_c_string("Willow Creek Mesh"),
            tag_type: FourCC::new(*b"mesh"),
            tag_id: FourCC::new(*b"l1  "),
            tag_data_offset: tag1_offset,
            tag_data_size: tag1_size,
            user_data: 0,
            version: 1,
            destination: 0,
            owner_index: 0,
            signature: SBTagHeader::SIGNATURE,
        };

        let mut archive_bytes = Vec::new();
        mono_header.write(&mut archive_bytes).unwrap();
        entry_tag.write(&mut archive_bytes).unwrap();
        tag0.write(&mut archive_bytes).unwrap();
        tag1.write(&mut archive_bytes).unwrap();
        archive_bytes.extend_from_slice(tag0_data);
        archive_bytes.extend_from_slice(tag1_data);

        let archive = TagArchive::from_bytes(archive_bytes.clone()).unwrap();
        assert_eq!(archive.header().name_as_str(), "synthetic install");
        assert_eq!(archive.tag_count(), 2);
        assert_eq!(archive.entry_tags().len(), 1);

        let found0 = archive.find_tag(FourCC::new(*b"amso"), "amb boat").unwrap();
        assert_eq!(found0.index, 0);

        let found_ci = archive
            .find_tag(FourCC::new(*b"mesh"), "willow creek mesh")
            .unwrap();
        assert_eq!(found_ci.index, 1);

        let data0 = archive.get_tag_data(found0).unwrap();
        assert_eq!(data0, tag0_data);

        let vfs = MemoryVfs::new();
        vfs.insert("tags/small install", archive_bytes);
        let vfs_archive = TagArchive::open(&vfs, "tags/small install").unwrap();
        assert_eq!(vfs_archive.tag_count(), 2);
    }

    #[test]
    fn test_stli_deserializers() {
        // Delimited string list
        let delimited_data = b"Welcome to Myth II\rWillow Creek\rSalvation\r";
        let stli_delim = StringListTag::from_bytes(delimited_data).unwrap();
        assert_eq!(stli_delim.len(), 3);
        assert_eq!(stli_delim.get(0), Some("Welcome to Myth II"));
        assert_eq!(stli_delim.get(1), Some("Willow Creek"));
        assert_eq!(stli_delim.get(2), Some("Salvation"));

        // Structured offset table format (count + offsets + null-terminated strings)
        let mut structured = Vec::new();
        let count = 2u16;
        let s0 = b"First String\0";
        let s1 = b"Second String\0";
        let off0 = 2 + 2 * 2; // header size
        let off1 = off0 + s0.len();

        structured.write_u16::<BigEndian>(count).unwrap();
        structured.write_u16::<BigEndian>(off0 as u16).unwrap();
        structured.write_u16::<BigEndian>(off1 as u16).unwrap();
        structured.extend_from_slice(s0);
        structured.extend_from_slice(s1);

        let stli_struct = StringListTag::from_bytes(&structured).unwrap();
        assert_eq!(stli_struct.len(), 2);
        assert_eq!(stli_struct.get(0), Some("First String"));
        assert_eq!(stli_struct.get(1), Some("Second String"));
    }

    #[test]
    fn test_mesh_deserializer() {
        let mut mesh_data = vec![0u8; 1024 + 32 * 32 * 12];
        mesh_data[0..4].copy_from_slice(b"00tm");
        mesh_data[4..8].copy_from_slice(b"wate");
        // submesh_w = 1, submesh_h = 1 -> 32x32 cells
        mesh_data[8..10].copy_from_slice(&1u16.to_be_bytes());
        mesh_data[10..12].copy_from_slice(&1u16.to_be_bytes());
        mesh_data[12..16].copy_from_slice(&0u32.to_be_bytes());
        mesh_data[16..20].copy_from_slice(&(32 * 32 * 12u32).to_be_bytes());

        let mesh = MeshTag::from_bytes(&mesh_data).unwrap();
        assert_eq!(mesh.mesh_id, "00tm");
        assert_eq!(mesh.media_tag, "wate");
        assert_eq!(mesh.dimensions(), (32, 32));
        assert_eq!(mesh.total_cells, 1024);
    }

    #[test]
    fn test_collection_deserializer() {
        let header = CollectionTag::HEADER_SIZE;
        let bref_off = 0usize;
        let bmp_off = 128usize;
        let (w, h) = (64usize, 128usize);
        let bmp_hdr = 48 + h * 4;
        let bmp_size = bmp_hdr + w * h;
        let mut coll_data = vec![0u8; header + bmp_off + bmp_size];

        coll_data[0x60..0x64].copy_from_slice(&1i32.to_be_bytes());
        coll_data[0x64..0x68].copy_from_slice(&(bref_off as i32).to_be_bytes());

        let ro = header + bref_off;
        coll_data[ro..ro + 8].copy_from_slice(b"frame #0");
        coll_data[ro + 64..ro + 68].copy_from_slice(&(bmp_off as i32).to_be_bytes());
        coll_data[ro + 68..ro + 72].copy_from_slice(&(bmp_size as i32).to_be_bytes());

        let bo = header + bmp_off;
        coll_data[bo..bo + 2].copy_from_slice(&(w as i16).to_be_bytes());
        coll_data[bo + 2..bo + 4].copy_from_slice(&(h as i16).to_be_bytes());

        let coll = CollectionTag::from_bytes(&coll_data).unwrap();
        assert_eq!(coll.bitmap_count, 1);
        let frame = coll.frame(0).unwrap();
        assert_eq!(frame.name, "frame #0");
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 128);
    }

    #[test]
    fn test_unit_deserializer() {
        // Test unit record (128 bytes)
        let mut unit_data = vec![0u8; 128];
        unit_data[0..4].copy_from_slice(b"arch");
        unit_data[4..8].copy_from_slice(b"arch");

        let unit = UnitTag::from_bytes(&unit_data).unwrap();
        assert_eq!(unit.tag_id, "arch");
        assert_eq!(unit.monster_tag_id, "arch");

        // Test monster record (1024 bytes)
        let mut mons_data = vec![0u8; 1024];
        mons_data[4..8].copy_from_slice(b"wam2");
        let mut cursor = Cursor::new(&mut mons_data[0x6C..0x80]);
        cursor.write_u32::<BigEndian>(2048).unwrap(); // vitality
        cursor.write_u32::<BigEndian>(350).unwrap(); // movement speed
        cursor.write_u32::<BigEndian>(0).unwrap();
        cursor.write_u32::<BigEndian>(2).unwrap(); // size

        mons_data[0x84..0x88].copy_from_slice(b"wamo");
        mons_data[0x90..0x94].copy_from_slice(b"wamt");

        let mons = UnitTag::from_bytes(&mons_data).unwrap();
        assert_eq!(mons.animation_collection, "wam2");
        assert_eq!(mons.vitality, 2048);
        assert_eq!(mons.movement_speed, 350);
        assert_eq!(mons.object_tag, Some(FourCC::new(*b"wamo")));
        assert_eq!(mons.projectile_tag, Some(FourCC::new(*b"wamt")));
    }
}
