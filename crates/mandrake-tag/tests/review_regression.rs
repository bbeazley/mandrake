use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use mandrake_common::{FourCC, SeekableRead};
use mandrake_tag::TagArchive;
use mandrake_tag::collection::{CollectionTag, TerrainTexture};
use mandrake_tag::connector::MeshConnectorList;
use mandrake_tag::detail_map::{DetailMap, rle_expand};
use mandrake_tag::dialog::Rect;
use mandrake_tag::font::{Bitmap32, FontFamily, FontTag, TextClip, TextLocale, draw_string};
use mandrake_tag::header::{SBMonoHeader, SBTagHeader, decode_c_string};
use mandrake_tag::hud::{rgb_color_from_sequence, sequence_instance, stored_bitmap_pixels};
use mandrake_tag::map_action::MeshMapActions;
use mandrake_tag::mesh::MeshTag;
use mandrake_tag::model::{GeometryTag, ModelTag};
use mandrake_tag::model_anim::ModelAnimationShadowMaps;
use mandrake_tag::sequence_header::{sequence_headers, sequence_key_points};
use mandrake_tag::sound::SoundTag;
use mandrake_tag::sound_manager::SoundManager;
use mandrake_tag::tag_files::{TYPE_PATCH, TYPE_PLUGIN, TagFileHead, order_engine_tag_files};
use mandrake_tag::tagset::plugin_entries;
use mandrake_tag::tfl::loose::read_loose_tag;
use mandrake_tag::tfl::menus::{TFL_COLLECTION_VERSION, tfl_menu_collection};
use mandrake_tag::tfl::mesh::TflMesh;
use mandrake_tag::tfl::model::{TFL_GEOMETRY_VERSION, TFL_MODEL_VERSION, tfl_geometry, tfl_model};

fn minimal_collection_with_one_bitmap(w: u16, h: u16, flags: u16, pixels: &[u8]) -> Vec<u8> {
    let header = CollectionTag::HEADER_SIZE;
    let bref_off = 0usize;
    let bmp_off = 128usize;
    let bmp_hdr = 48 + (h as usize) * 4;
    let bmp_size = bmp_hdr + pixels.len();
    let mut data = vec![0u8; header + bmp_off + bmp_size];

    data[0x60..0x64].copy_from_slice(&1i32.to_be_bytes());
    data[0x64..0x68].copy_from_slice(&(bref_off as i32).to_be_bytes());

    let ro = header + bref_off;
    data[ro..ro + 4].copy_from_slice(b"bmp0");
    data[ro + 64..ro + 68].copy_from_slice(&(bmp_off as i32).to_be_bytes());
    data[ro + 68..ro + 72].copy_from_slice(&(bmp_size as i32).to_be_bytes());

    let bo = header + bmp_off;
    data[bo..bo + 2].copy_from_slice(&(w as i16).to_be_bytes());
    data[bo + 2..bo + 4].copy_from_slice(&(h as i16).to_be_bytes());
    data[bo + 6..bo + 8].copy_from_slice(&flags.to_be_bytes());
    data[bo + bmp_hdr..bo + bmp_hdr + pixels.len()].copy_from_slice(pixels);
    data
}

#[test]
fn b1_collection_rejects_huge_counts_and_dimensions_without_oom() {
    let mut data = minimal_collection_with_one_bitmap(2, 2, 0, &[1, 2, 3, 4]);
    data[0x70..0x74].copy_from_slice(&i32::MAX.to_be_bytes());
    data[0x80..0x84].copy_from_slice(&i32::MAX.to_be_bytes());
    data[0x90..0x94].copy_from_slice(&u32::MAX.to_be_bytes());
    let coll = CollectionTag::from_bytes(&data).unwrap();
    assert!(coll.sequences.is_empty());
    assert!(coll.shadow_descriptors.is_empty());

    let mut huge_bmp = minimal_collection_with_one_bitmap(2, 2, 0, &[1, 2, 3, 4]);
    let bo = CollectionTag::HEADER_SIZE + 128;
    huge_bmp[bo..bo + 2].copy_from_slice(&i16::MAX.to_be_bytes());
    huge_bmp[bo + 2..bo + 4].copy_from_slice(&i16::MAX.to_be_bytes());
    assert!(CollectionTag::from_bytes(&huge_bmp).is_err());

    let mut trunc_uncomp = minimal_collection_with_one_bitmap(64, 64, 0, &[1, 2, 3, 4]);
    let bo = CollectionTag::HEADER_SIZE + 128;
    trunc_uncomp[bo..bo + 2].copy_from_slice(&64i16.to_be_bytes());
    trunc_uncomp[bo + 2..bo + 4].copy_from_slice(&64i16.to_be_bytes());
    assert!(CollectionTag::from_bytes(&trunc_uncomp).is_err());

    let header = CollectionTag::HEADER_SIZE;
    let mut seq_oom = vec![0u8; header + 128 + 48 + 4 + 128 + 64];
    seq_oom[0x60..0x64].copy_from_slice(&1i32.to_be_bytes());
    seq_oom[0x64..0x68].copy_from_slice(&0i32.to_be_bytes());
    seq_oom[header + 64..header + 68].copy_from_slice(&128i32.to_be_bytes());
    seq_oom[header + 68..header + 72].copy_from_slice(&52i32.to_be_bytes());
    seq_oom[0x80..0x84].copy_from_slice(&1i32.to_be_bytes());
    let sref_off = 128 + 52;
    seq_oom[0x84..0x88].copy_from_slice(&(sref_off as i32).to_be_bytes());
    let shdr_off = sref_off + 128;
    seq_oom[header + sref_off + 64..header + sref_off + 68]
        .copy_from_slice(&(shdr_off as i32).to_be_bytes());
    let shdr = header + shdr_off;
    seq_oom[shdr + 8..shdr + 10].copy_from_slice(&i16::MAX.to_be_bytes());
    seq_oom[shdr + 10..shdr + 12].copy_from_slice(&i16::MAX.to_be_bytes());
    assert!(CollectionTag::from_bytes(&seq_oom).is_err());
}

#[test]
fn b1_mesh_and_sequence_header_and_detail_map_and_sound_reject_overflows() {
    let mut mesh_ovf = vec![0u8; 1024];
    mesh_ovf[8..10].copy_from_slice(&2048u16.to_be_bytes());
    mesh_ovf[10..12].copy_from_slice(&1u16.to_be_bytes());
    assert!(MeshTag::from_bytes(&mesh_ovf).is_err());

    let mut mesh_zero_size = vec![0u8; 1024];
    mesh_zero_size[8..10].copy_from_slice(&1000u16.to_be_bytes());
    mesh_zero_size[10..12].copy_from_slice(&1000u16.to_be_bytes());
    assert!(MeshTag::from_bytes(&mesh_zero_size).is_err());

    let mut mesh_huge_tables = vec![0u8; 1024];
    mesh_huge_tables[36..40].copy_from_slice(&u32::MAX.to_be_bytes());
    mesh_huge_tables[52..56].copy_from_slice(&u32::MAX.to_be_bytes());
    let parsed = MeshTag::from_bytes(&mesh_huge_tables).unwrap();
    assert!(parsed.palettes.is_empty());
    assert!(parsed.markers.is_empty());

    let mut seq_bad = vec![0u8; 320];
    seq_bad[0x80..0x84].copy_from_slice(&i32::MAX.to_be_bytes());
    assert!(sequence_headers(&seq_bad).is_err());
    assert!(sequence_key_points(&seq_bad).is_err());

    assert!(rle_expand(&[0, 0], 0, usize::MAX).is_err());
    let mut dmap = vec![0u8; 0x100 + 128 * 64];
    dmap[0..4].copy_from_slice(&i32::MAX.to_be_bytes());
    dmap[4..8].copy_from_slice(&i32::MAX.to_be_bytes());
    dmap[24..28].copy_from_slice(&0x100i32.to_be_bytes());
    assert!(DetailMap::from_bytes(&dmap).is_err());

    let mut soun = vec![0u8; 64];
    soun[0x24..0x28].copy_from_slice(&0x0800_0000u32.to_be_bytes());
    assert!(SoundTag::from_bytes(&soun).is_err());
    soun[0x24..0x28].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(SoundTag::from_bytes(&soun).is_err());
}

#[test]
fn b1_misc_tag_parsers_handle_overflowing_offsets_without_panic() {
    let mut ma = vec![0u8; 1024];
    ma[0x80..0x84].copy_from_slice(&1u32.to_be_bytes());
    ma[0x84..0x88].copy_from_slice(&u32::MAX.to_be_bytes());
    ma[0x88..0x8c].copy_from_slice(&64u32.to_be_bytes());
    assert!(MeshMapActions::from_mesh_bytes(&ma).is_err());

    let mut tfl_m = vec![0u8; 1024];
    tfl_m[8..10].copy_from_slice(&4096u16.to_be_bytes());
    tfl_m[10..12].copy_from_slice(&4096u16.to_be_bytes());
    assert!(TflMesh::from_bytes(&tfl_m).is_err());

    let mut geom = vec![0u8; 128];
    geom[0x4c..0x50].copy_from_slice(&96u32.to_be_bytes());
    geom[0x50..0x54].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(tfl_geometry(&geom, TFL_GEOMETRY_VERSION).is_err());

    let mut tfl_mode = vec![0u8; 64];
    tfl_mode[0x34..0x38].copy_from_slice(&64u32.to_be_bytes());
    tfl_mode[0x38..0x3c].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(tfl_model(&tfl_mode, TFL_MODEL_VERSION).is_err());

    let mut loose = vec![0u8; 64];
    loose[0x30..0x34].copy_from_slice(&u32::MAX.to_be_bytes());
    loose[0x34..0x38].copy_from_slice(&16u32.to_be_bytes());
    loose[0x3c..0x40].copy_from_slice(b"myth");
    assert!(read_loose_tag(&loose).is_err());

    let mut m2_geom = vec![0u8; 128];
    m2_geom[8..10].copy_from_slice(&u16::MAX.to_be_bytes());
    m2_geom[28..32].copy_from_slice(&u32::MAX.to_be_bytes());
    let g = GeometryTag::from_bytes(&m2_geom).unwrap();
    assert!(g.materials.is_empty());

    let mut m2_mode = vec![0u8; 64];
    m2_mode[12..14].copy_from_slice(&10u16.to_be_bytes());
    m2_mode[0x1c..0x20].copy_from_slice(&u32::MAX.to_be_bytes());
    m2_mode[0x34..0x38].copy_from_slice(&u32::MAX.to_be_bytes());
    let m = ModelTag::from_bytes(&m2_mode).unwrap();
    assert!(m.permutations.is_empty());

    let mut conn = vec![0u8; 1024];
    conn[0x190..0x194].copy_from_slice(&10u32.to_be_bytes());
    conn[0x194..0x198].copy_from_slice(&u32::MAX.to_be_bytes());
    let cl = MeshConnectorList::from_mesh_bytes(&conn).unwrap();
    assert!(cl.connectors.is_empty());
}

#[test]
fn b2_negative_offsets_in_hud_and_terrain_texture_do_not_panic() {
    let mut raw = minimal_collection_with_one_bitmap(2, 2, 0, &[1, 2, 3, 4]);
    let coll = CollectionTag::from_bytes(&raw).unwrap();
    raw[0x80..0x84].copy_from_slice(&1i32.to_be_bytes());
    raw[0x84..0x88].copy_from_slice(&(-1i32).to_be_bytes());
    assert_eq!(
        rgb_color_from_sequence(&raw, &coll, 0, 0),
        mandrake_tag::hud::DEFAULT_SEQUENCE_COLOR
    );
    assert!(sequence_instance(&raw, 0, 0, 0).is_none());

    let mut raw_neg_inst = vec![0u8; CollectionTag::HEADER_SIZE + 128 + 128];
    let h = CollectionTag::HEADER_SIZE;
    raw_neg_inst[0x70..0x74].copy_from_slice(&(-1i32).to_be_bytes());
    raw_neg_inst[0x74..0x78].copy_from_slice(&0i32.to_be_bytes());
    raw_neg_inst[0x80..0x84].copy_from_slice(&1i32.to_be_bytes());
    raw_neg_inst[0x84..0x88].copy_from_slice(&0i32.to_be_bytes());
    raw_neg_inst[h + 0x40..h + 0x44].copy_from_slice(&128i32.to_be_bytes());
    let seq = h + 128;
    raw_neg_inst[seq + 8..seq + 10].copy_from_slice(&1i16.to_be_bytes());
    raw_neg_inst[seq + 0x0a..seq + 0x0c].copy_from_slice(&1i16.to_be_bytes());
    raw_neg_inst[seq + 0x6e..seq + 0x70].copy_from_slice(&(-2i16).to_be_bytes());
    assert!(sequence_instance(&raw_neg_inst, 0, 0, 0).is_none());

    let mut raw_bmp = minimal_collection_with_one_bitmap(2, 2, 0, &[1, 2, 3, 4]);
    raw_bmp[0x64..0x68].copy_from_slice(&(-1i32).to_be_bytes());
    assert!(stored_bitmap_pixels(&raw_bmp, 0).is_none());

    let mut terrain = vec![0u8; TerrainTexture::HDR + 4096];
    terrain[96..100].copy_from_slice(&2i32.to_be_bytes());
    terrain[100..104].copy_from_slice(&(-100i32).to_be_bytes());
    assert!(TerrainTexture::from_dot256(&terrain, 1, 1).is_err());

    terrain[100..104].copy_from_slice(&0i32.to_be_bytes());
    terrain[0xb0..0xb4].copy_from_slice(&1i32.to_be_bytes());
    terrain[0xb4..0xb8].copy_from_slice(&(-4i32).to_be_bytes());
    let ok = TerrainTexture::from_dot256(&terrain, 1, 1).unwrap();
    assert!(ok.blood_remap.is_none());

    assert!(TerrainTexture::from_dot256(&terrain, u32::MAX, 1).is_err());

    let mut terrain_mut = vec![0u8; TerrainTexture::HDR + 4096];
    terrain_mut[96..100].copy_from_slice(&(-1i32).to_be_bytes());
    let color = vec![0u8; 256 * 256];
    assert!(TerrainTexture::write_color_index(&mut terrain_mut, 1, 1, &color).is_err());
}

#[test]
fn b3_font_draw_line_sets_opaque_alpha_and_clamps_transparency() {
    let mut font_data = vec![0u8; 0x40 + 16 + 2];
    font_data[4..6].copy_from_slice(&4i16.to_be_bytes());
    font_data[6..8].copy_from_slice(&1i16.to_be_bytes());
    font_data[0x0c..0x10].copy_from_slice(&1u32.to_be_bytes());
    font_data[0x10..0x14].copy_from_slice(&0x40u32.to_be_bytes());
    font_data[0x14..0x18].copy_from_slice(&18u32.to_be_bytes());
    font_data[0x18..0x38].fill(0xff);
    let g = 0x40;
    font_data[g..g + 2].copy_from_slice(&(b'a' as u16).to_be_bytes());
    font_data[g + 2..g + 4].copy_from_slice(&3i16.to_be_bytes());
    font_data[g + 4..g + 6].copy_from_slice(&1i16.to_be_bytes());
    font_data[g + 6..g + 8].copy_from_slice(&1i16.to_be_bytes());
    font_data[g + 10..g + 12].copy_from_slice(&1i16.to_be_bytes());
    font_data[g + 16] = 0;

    let fam = FontFamily::from_font(FourCC(*b"test"), FontTag::from_bytes(&font_data).unwrap());
    let loc = TextLocale::from_stli_bytes(&[]);
    let mut bmp = Bitmap32::new(8, 8, 0);
    draw_string(
        &mut bmp,
        Rect::new(0, 0, 8, 8),
        TextClip::SameAsRect,
        &fam,
        -1,
        b"a",
        [0x1200, 0x3400, 0x5600],
        0,
        0,
        &loc,
    );
    assert_eq!(bmp.get(1, 3), 0xff12_3456);

    let mut bmp_high_t = Bitmap32::new(8, 8, 0xff20_2020);
    draw_string(
        &mut bmp_high_t,
        Rect::new(0, 0, 8, 8),
        TextClip::SameAsRect,
        &fam,
        -1,
        b"a",
        [0xffff, 0xffff, 0xffff],
        0,
        0x1000_0000,
        &loc,
    );
    assert_eq!(bmp_high_t.get(1, 3) >> 24, 0xff);
}

#[test]
fn b4_tfl_menu_collection_keys_raw_zero() {
    let data = minimal_collection_with_one_bitmap(2, 2, 0, &[0, 5, 6, 7]);
    let m2 = CollectionTag::from_bytes(&data).unwrap();
    assert_eq!(m2.frames[0].flags & 0x0002, 0);
    assert_eq!(m2.frames[0].pixels, vec![1, 5, 6, 7]);

    let tfl = tfl_menu_collection(FourCC(*b"test"), TFL_COLLECTION_VERSION, &data).unwrap();
    assert_eq!(tfl.frames[0].flags & 0x0002, 0x0002);
    assert_eq!(tfl.frames[0].pixels, vec![0, 5, 6, 7]);
}

#[test]
fn b5_tagset_ignores_non_monolith_files() {
    let dir = std::env::temp_dir().join(format!("mandrake-tag-b5-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut fake = vec![0u8; SBMonoHeader::SIZE];
    fake[0..2].copy_from_slice(&TYPE_PLUGIN.to_be_bytes());
    fake[124..128].copy_from_slice(b"junk");
    std::fs::write(dir.join("not_a_plugin"), &fake).unwrap();

    let mut valid = Vec::new();
    SBMonoHeader {
        r#type: TYPE_PLUGIN,
        version: 1,
        name: [0; 32],
        description: [0; 64],
        entry_tag_count: 0,
        tag_list_count: 0,
        checksum: [0x12, 0x34, 0x56, 0x78],
        flags: 0,
        size: 0,
        header_checksum: [0; 4],
        padding: [0; 4],
        signature: SBMonoHeader::SIG_MTH2,
    }
    .write(&mut valid)
    .unwrap();
    std::fs::write(dir.join("real_plugin"), &valid).unwrap();

    let entries = plugin_entries(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, b"real_plugin");
}

#[test]
fn b6_collection_preserves_frame_indices_when_bitmap_ref_is_invalid() {
    let header = CollectionTag::HEADER_SIZE;
    let bref_off = 0usize;
    let binst_off = 256usize;
    let bmp1_off = binst_off + 64;
    let bmp1_hdr = 48 + 2 * 4;
    let bmp1_size = bmp1_hdr + 4;
    let mut data = vec![0u8; header + bmp1_off + bmp1_size];

    data[0x60..0x64].copy_from_slice(&2i32.to_be_bytes());
    data[0x64..0x68].copy_from_slice(&(bref_off as i32).to_be_bytes());
    data[0x70..0x74].copy_from_slice(&1i32.to_be_bytes());
    data[0x74..0x78].copy_from_slice(&(binst_off as i32).to_be_bytes());

    let r0 = header;
    data[r0..r0 + 3].copy_from_slice(b"bad");
    data[r0 + 64..r0 + 68].copy_from_slice(&999_999i32.to_be_bytes());
    data[r0 + 68..r0 + 72].copy_from_slice(&64i32.to_be_bytes());

    let r1 = header + 128;
    data[r1..r1 + 4].copy_from_slice(b"good");
    data[r1 + 64..r1 + 68].copy_from_slice(&(bmp1_off as i32).to_be_bytes());
    data[r1 + 68..r1 + 72].copy_from_slice(&(bmp1_size as i32).to_be_bytes());

    let io = header + binst_off;
    data[io + 12..io + 14].copy_from_slice(&7i16.to_be_bytes());
    data[io + 14..io + 16].copy_from_slice(&9i16.to_be_bytes());
    data[io + 28..io + 30].copy_from_slice(&1i16.to_be_bytes());

    let bo = header + bmp1_off;
    data[bo..bo + 2].copy_from_slice(&2i16.to_be_bytes());
    data[bo + 2..bo + 4].copy_from_slice(&2i16.to_be_bytes());
    data[bo + bmp1_hdr..bo + bmp1_hdr + 4].copy_from_slice(&[5, 6, 7, 8]);

    let coll = CollectionTag::from_bytes(&data).unwrap();
    assert_eq!(coll.frames.len(), 2);
    assert_eq!((coll.frames[0].width, coll.frames[0].height), (0, 0));
    assert_eq!(coll.frames[1].name, "good");
    assert_eq!((coll.frames[1].width, coll.frames[1].height), (2, 2));
    assert_eq!((coll.frames[1].offset_x, coll.frames[1].offset_y), (7, 9));
}

#[test]
fn b7_negative_dimensions_rejected_in_font_and_shadow_maps() {
    let mut font_data = vec![0u8; 0x40 + 16 + 16];
    font_data[0x0c..0x10].copy_from_slice(&1u32.to_be_bytes());
    font_data[0x10..0x14].copy_from_slice(&0x40u32.to_be_bytes());
    font_data[0x14..0x18].copy_from_slice(&32u32.to_be_bytes());
    font_data[0x40 + 4..0x40 + 6].copy_from_slice(&(-4i16).to_be_bytes());
    font_data[0x40 + 6..0x40 + 8].copy_from_slice(&(-4i16).to_be_bytes());
    assert!(FontTag::from_bytes(&font_data).is_err());

    let maps = ModelAnimationShadowMaps {
        width: 4,
        height: -4,
        row_bytes: -4,
        offset: [0, 0],
        pixels: vec![0u8; 32],
    };
    assert!(maps.frame(0).is_none());

    let valid_maps = ModelAnimationShadowMaps {
        width: 4,
        height: 4,
        row_bytes: 4,
        offset: [0, 0],
        pixels: vec![0u8; 32],
    };
    assert!(valid_maps.frame(usize::MAX).is_none());
    assert_eq!(valid_maps.frame(1).map(<[u8]>::len), Some(16));
}

fn make_monolith_bytes(tags: &[(&[u8; 4], &[u8; 4], &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    SBMonoHeader {
        r#type: 1,
        version: 1,
        name: [0; 32],
        description: [0; 64],
        entry_tag_count: 0,
        tag_list_count: tags.len() as u16,
        checksum: [0; 4],
        flags: 0,
        size: 0,
        header_checksum: [0; 4],
        padding: [0; 4],
        signature: SBMonoHeader::SIG_MTH2,
    }
    .write(&mut out)
    .unwrap();
    let mut offset = SBMonoHeader::SIZE + tags.len() * SBTagHeader::SIZE;
    for (g, id, data) in tags {
        SBTagHeader {
            identifier: 0,
            flags: 0,
            tag_type_idx: 0,
            name: [0; 32],
            tag_type: FourCC::new(**g),
            tag_id: FourCC::new(**id),
            tag_data_offset: offset as i32,
            tag_data_size: data.len() as i32,
            user_data: 0,
            version: 1,
            destination: 0,
            owner_index: 0,
            signature: SBTagHeader::SIGNATURE,
        }
        .write(&mut out)
        .unwrap();
        offset += data.len();
    }
    for (_, _, data) in tags {
        out.extend_from_slice(data);
    }
    out
}

fn make_archive(tags: &[(&[u8; 4], &[u8; 4], &[u8])]) -> TagArchive {
    TagArchive::from_bytes(make_monolith_bytes(tags)).unwrap()
}

#[test]
fn b8_p2_p4_archive_negative_sizes_nested_overrides_index_and_no_end_seek() {
    let mut bad = Vec::new();
    SBMonoHeader {
        r#type: 1,
        version: 1,
        name: [0; 32],
        description: [0; 64],
        entry_tag_count: 0,
        tag_list_count: 1,
        checksum: [0; 4],
        flags: 0,
        size: 0,
        header_checksum: [0; 4],
        padding: [0; 4],
        signature: SBMonoHeader::SIG_MTH2,
    }
    .write(&mut bad)
    .unwrap();
    SBTagHeader {
        identifier: 0,
        flags: 0,
        tag_type_idx: 0,
        name: [0; 32],
        tag_type: FourCC(*b"stli"),
        tag_id: FourCC(*b"test"),
        tag_data_offset: 192,
        tag_data_size: -1,
        user_data: 0,
        version: 1,
        destination: 0,
        owner_index: 0,
        signature: SBTagHeader::SIGNATURE,
    }
    .write(&mut bad)
    .unwrap();
    bad.extend_from_slice(&[0u8; 64]);
    assert!(TagArchive::from_bytes(bad).is_err());

    let base = make_archive(&[
        (b"stli", b"aaaa", b"base"),
        (b"stli", b"bbbb", b"only_base"),
    ]);
    let patch = make_archive(&[(b"stli", b"aaaa", b"patch"), (b"stli", b"cccc", b"patch_c")]);
    let inner = make_archive(&[(b"stli", b"aaaa", b"inner"), (b"stli", b"cccc", b"inner_c")]);
    let nested = patch.with_overrides(vec![inner]);
    let merged = base.with_overrides(vec![nested]);

    let read = |id: &[u8; 4]| {
        let e = merged
            .find_tag_by_id(FourCC(*b"stli"), FourCC(*id))
            .unwrap();
        merged.get_tag_data(e).unwrap()
    };
    assert_eq!(read(b"aaaa"), b"inner");
    assert_eq!(read(b"bbbb"), b"only_base");
    assert_eq!(read(b"cccc"), b"inner_c");

    let e = merged
        .find_tag_by_id(FourCC(*b"stli"), FourCC(*b"aaaa"))
        .unwrap();
    assert_eq!(merged.read_tag_prefix(e, usize::MAX).unwrap(), b"inner");

    struct CountingReader {
        inner: Cursor<Vec<u8>>,
        end_seeks: Arc<AtomicUsize>,
    }
    impl Read for CountingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }
    impl Seek for CountingReader {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            if matches!(pos, SeekFrom::End(_)) {
                self.end_seeks.fetch_add(1, Ordering::Relaxed);
            }
            self.inner.seek(pos)
        }
    }
    let bytes = make_monolith_bytes(&[(b"stli", b"aaaa", b"hello world")]);
    let end_seeks = Arc::new(AtomicUsize::new(0));
    let reader: Box<dyn SeekableRead + Send> = Box::new(CountingReader {
        inner: Cursor::new(bytes),
        end_seeks: Arc::clone(&end_seeks),
    });
    let arc = TagArchive::from_reader(reader).unwrap();
    let seeks_after_open = end_seeks.load(Ordering::Relaxed);
    let e = arc
        .find_tag_by_id(FourCC(*b"stli"), FourCC(*b"aaaa"))
        .unwrap();
    assert_eq!(arc.get_tag_data(e).unwrap(), b"hello world");
    assert_eq!(arc.read_tag_prefix(e, 5).unwrap(), b"hello");
    assert_eq!(end_seeks.load(Ordering::Relaxed), seeks_after_open);
}

#[test]
fn b9_missing_patches_requires_patch_type() {
    let fake_patch_as_plugin = TagFileHead {
        path: "downloads/Patch 1.2".into(),
        name: "Patch 1.2".to_string(),
        header: SBMonoHeader {
            r#type: TYPE_PLUGIN,
            version: 2,
            name: [0; 32],
            description: [0; 64],
            entry_tag_count: 0,
            tag_list_count: 0,
            checksum: [0; 4],
            flags: 0,
            size: 0,
            header_checksum: [0; 4],
            padding: [0; 4],
            signature: SBMonoHeader::SIG_MTH2,
        },
    };
    let out = order_engine_tag_files(&|_| None, &[fake_patch_as_plugin], &[], "");
    assert!(out.missing_patches.iter().any(|p| p == "Patch 1.2"));

    let real_patch = TagFileHead {
        path: "plugins/Patch 1.2".into(),
        name: "Patch 1.2".to_string(),
        header: SBMonoHeader {
            r#type: TYPE_PATCH,
            version: 2,
            name: [0; 32],
            description: [0; 64],
            entry_tag_count: 0,
            tag_list_count: 0,
            checksum: [0; 4],
            flags: 0,
            size: 0,
            header_checksum: [0; 4],
            padding: [0; 4],
            signature: SBMonoHeader::SIG_MTH2,
        },
    };
    let out = order_engine_tag_files(&|_| None, &[real_patch], &[], "");
    assert!(!out.missing_patches.iter().any(|p| p == "Patch 1.2"));
}

#[test]
fn b10_decode_c_string_uses_mac_roman() {
    assert_eq!(
        decode_c_string(b"a\x8e\xc9\xd2\xd3\xd4\xd5\0junk"),
        "aé…“”‘’"
    );
    assert_eq!(decode_c_string(&[0xc3, 0xa9]), "√©");
}

#[test]
fn p1_engine_ambient_index_deduplicates_across_stack_and_spill() {
    let mut mgr = SoundManager::new(0);
    mgr.stereo_background = Some(100);
    mgr.particle_definition = Some(101);
    mgr.palette_definitions = (0..40).flat_map(|i| [i, i]).collect();
    mgr.fire_definition = Some(102);

    assert_eq!(mgr.engine_ambient_index(100), Some(0));
    assert_eq!(mgr.engine_ambient_index(101), Some(1));
    assert_eq!(mgr.engine_ambient_index(0), Some(2));
    assert_eq!(mgr.engine_ambient_index(39), Some(41));
    assert_eq!(mgr.engine_ambient_index(102), Some(42));
    assert_eq!(mgr.engine_ambient_index(999), None);
}

#[test]
fn s3_corrupt_collection_returns_error_instead_of_legacy_fallback() {
    let bad = vec![0u8; 4096];
    assert!(CollectionTag::from_bytes(&bad).is_err());
}
