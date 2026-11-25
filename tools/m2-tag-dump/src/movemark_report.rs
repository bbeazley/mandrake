use mandrake_common::FourCC;
use mandrake_tag::unit::MonsterTerrain;
use mandrake_tag::{CollectionTag, MarkerPaletteEntry, MeshTag};

type Tags<'a> = &'a dyn Fn([u8; 4], FourCC) -> Option<Vec<u8>>;

const GAMEPLAY_VERSION: i32 = 0x726;

fn be16(d: &[u8], o: usize) -> i16 {
    d.get(o..o + 2)
        .map_or(-1, |b| i16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn tag_ref(d: &[u8], o: usize) -> Option<FourCC> {
    let b = d.get(o..o + 4)?;
    (b != [0xff; 4] && b != [0; 4]).then(|| FourCC::new([b[0], b[1], b[2], b[3]]))
}

pub fn terrain_type_at(flags_at: &dyn Fn(i32, i32) -> u16, w: i32, h: i32, x: i32, y: i32) -> u8 {
    let cx = (x >> 9).clamp(0, w - 1);
    let cy = (y >> 9).clamp(0, h - 1);
    let (fx, fy) = (x & 0x1ff, y & 0x1ff);
    let low = if (cx ^ cy) & 1 == 0 {
        fx >= fy
    } else {
        fy >= 0x200 - fx
    };
    let f = flags_at(cx, cy);
    if low {
        (f & 0xf) as u8
    } else {
        ((f >> 4) & 0xf) as u8
    }
}

pub fn marker_sequence(impassable_mask: u16, terrain_type: u8, last_point: bool) -> i16 {
    if (impassable_mask >> (terrain_type & 0xf)) & 1 != 0 {
        4
    } else if last_point {
        0
    } else {
        1
    }
}

pub fn instance_world_rect(s: i32, kx: i32, ky: i32, w: i32, h: i32) -> (i32, i32, i32, i32) {
    (
        (s.wrapping_mul(ky)) >> 16,
        -((s.wrapping_mul(kx)) >> 16),
        ((w - kx).wrapping_mul(s)) >> 16,
        -(((h - ky).wrapping_mul(s)) >> 16),
    )
}

fn describe_sequence(raw: &[u8], coll: &CollectionTag, seq: usize) {
    let h = CollectionTag::HEADER_SIZE;
    let rd = |o: usize| be32(raw, o) as usize;
    let Some(s) = coll.sequence(seq) else {
        println!("    sequence {seq}: not in the collection");
        return;
    };
    let shdr = h + rd(h + rd(0x84) + seq * 128 + 64);
    println!(
        "    sequence {seq} '{}': flags {:#x}, {} views, {} frames x {} ticks, key {}, loop {}, transfer mode +0x24 {} period +0x26 {}, frame shadow descriptors {:?} (-1 = none)",
        s.name,
        s.flags,
        s.num_views,
        s.frames_per_view,
        s.ticks_per_frame,
        s.key_frame,
        s.loop_frame,
        be16(raw, shdr + 0x24),
        be16(raw, shdr + 0x26),
        s.shadow_indices,
    );
    let (binst_count, binst_off) = (rd(0x70), rd(0x74));
    let views = s.num_views.max(1) as usize;
    let stride = 46 + 2 * views;
    for f in 0..s.frames_per_view as usize {
        for v in 0..views {
            let inst = be16(raw, shdr + 64 + f * stride + 46 + 2 * v);
            if inst < 0 || inst as usize >= binst_count {
                continue;
            }
            let io = h + binst_off + 64 * inst as usize;
            let iflags = be32(raw, io);
            let bref = be16(raw, io + 28);
            let (scale, kx, ky) = if iflags & 0x20 != 0 {
                (
                    be32(raw, io + 0x20) as i32,
                    be16(raw, io + 0x24) as i32,
                    be16(raw, io + 0x26) as i32,
                )
            } else {
                (
                    (be32(raw, shdr + 4) as i32).wrapping_mul(10),
                    be16(raw, io + 0x0c) as i32,
                    be16(raw, io + 0x0e) as i32,
                )
            };
            print!(
                "      frame {f} view {v}: instance {inst} flags {iflags:#x}, bitmap {bref}, key {kx},{ky}, scale {scale:#x} (16.16 world units per pixel)",
            );
            if let Some(b) = coll.frame(bref.max(0) as usize) {
                let r = instance_world_rect(scale, kx, ky, b.width as i32, b.height as i32);
                println!(
                    "; bitmap {}x{} flags {:#x}; world rect top {} left {} right {} bottom {}",
                    b.width, b.height, b.flags, r.0, r.1, r.2, r.3
                );
                print_colours(coll, b.index);
            } else {
                println!();
            }
        }
    }
}

fn print_colours(coll: &CollectionTag, bitmap: usize) {
    let Some(b) = coll.frame(bitmap) else { return };
    let mut counts = std::collections::BTreeMap::<u8, usize>::new();
    let n = b.width as usize * b.height as usize;
    for &p in b.pixels.iter().take(n) {
        if p != 0 {
            *counts.entry(p).or_default() += 1;
        }
    }
    let mut v: Vec<(u8, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let total: usize = v.iter().map(|x| x.1).sum();
    let top: Vec<String> = v
        .iter()
        .take(6)
        .map(|&(i, c)| {
            let rgb = coll.palette.get(i as usize).copied().unwrap_or([0; 4]);
            format!(
                "#{:02x}{:02x}{:02x} (index {i}) x{c}",
                rgb[0], rgb[1], rgb[2]
            )
        })
        .collect();
    println!(
        "        {total} opaque pixels of {n}; most common: {}",
        top.join(", ")
    );
    for y in 0..b.height as usize {
        let row: String = (0..b.width as usize)
            .map(|x| {
                let p = b.pixels.get(y * b.width as usize + x).copied().unwrap_or(0);
                let c = coll.palette.get(p as usize).copied().unwrap_or([0; 4]);
                if p == 0 {
                    '.'
                } else if u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2]) < 96 {
                    ':'
                } else {
                    '#'
                }
            })
            .collect();
        println!("          {row}");
    }
}

fn closed_types(mask: u16) -> String {
    (0..16)
        .filter(|t| mask >> t & 1 != 0)
        .map(|t: i32| t.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn movemark_report(
    tags: Tags,
    inte: &[FourCC],
    meshes: &[FourCC],
    points: &[(i32, i32)],
    radius: i32,
    grid: bool,
) -> anyhow::Result<()> {
    for &id in inte {
        let Some(d) = tags(*b"inte", id) else {
            println!("# inte {id}: missing");
            continue;
        };
        let coll_id = tag_ref(&d, 0);
        println!(
            "# inte {id}: collection +0x00 {} (-> [0x860870] +0xfc, get_interface_collection)",
            coll_id.map_or("-".to_string(), |c| c.to_string())
        );
        let Some(raw) = coll_id.and_then(|c| tags(*b".256", c)) else {
            println!("  collection missing");
            continue;
        };
        let coll = CollectionTag::from_bytes(&raw)?;
        println!(
            "  {} bitmaps, {} sequences; marker sequences the engine uses: 0 (last point), 1 (earlier waypoint), 4 (impassable)",
            coll.frame_count(),
            coll.sequences.len()
        );
        for s in 0..coll.sequences.len() {
            describe_sequence(&raw, &coll, s);
        }
        println!();
    }
    for &mesh_id in meshes {
        let md = tags(*b"mesh", mesh_id).ok_or_else(|| anyhow::anyhow!("no mesh {mesh_id}"))?;
        let mesh = MeshTag::from_bytes(&md)?;
        println!(
            "# mesh {mesh_id}: unit types and their impassable masks (monster +0x1a, gameplay {GAMEPLAY_VERSION:#x})"
        );
        let mut seen = std::collections::BTreeSet::new();
        let mut masks: Vec<(FourCC, u16)> = Vec::new();
        for p in &mesh.palettes {
            if p.marker_type != MarkerPaletteEntry::TYPE_UNIT {
                continue;
            }
            let Some(mons) = tags(*b"unit", p.tag_id).and_then(|u| tag_ref(&u, 0)) else {
                continue;
            };
            if !seen.insert(mons) {
                continue;
            }
            let Some(m) = tags(*b"mons", mons) else {
                continue;
            };
            let t = MonsterTerrain::from_mons_bytes(&m)?;
            let mask = t.impassable_mask(GAMEPLAY_VERSION);
            masks.push((mons, mask));
            println!(
                "  unit {} -> mons {mons}: mask {mask:#06x}, closed types {}",
                p.tag_id,
                closed_types(mask)
            );
        }
        let (w, h) = mesh.dimensions();
        let (w, h) = (w as i32, h as i32);
        let flags_at = |cx: i32, cy: i32| {
            mesh.cell_at(cx as usize, cy as usize)
                .map_or(0, |c| c.flags)
        };
        for &(px, py) in points {
            let t = terrain_type_at(&flags_at, w, h, px, py);
            let seqs: Vec<String> = masks
                .iter()
                .map(|(m, mask)| format!("{m} {}", marker_sequence(*mask, t, true)))
                .collect();
            println!(
                "  point ({px}, {py}): cell ({}, {}), terrain type {t}; marker sequence as the last point: {}",
                px >> 9,
                py >> 9,
                seqs.join(", ")
            );
            let mut best: [Option<(i64, i32, i32)>; 16] = [None; 16];
            let (c0x, c0y) = (px >> 9, py >> 9);
            let rc = radius >> 9;
            for cy in (c0y - rc).max(0)..=(c0y + rc).min(h - 1) {
                for cx in (c0x - rc).max(0)..=(c0x + rc).min(w - 1) {
                    let f = flags_at(cx, cy);
                    let (a, b) = (((f >> 4) & 0xf) as usize, (f & 0xf) as usize);
                    if a != b {
                        continue;
                    }
                    let (wx, wy) = (cx * 512 + 256, cy * 512 + 256);
                    let d =
                        (wx - px) as i64 * (wx - px) as i64 + (wy - py) as i64 * (wy - py) as i64;
                    if best[a].is_none_or(|(bd, _, _)| d < bd) {
                        best[a] = Some((d, wx, wy));
                    }
                }
            }
            if grid {
                println!(
                    "    cells x {}..={} (columns), y {}..={} (rows):",
                    (c0x - rc).max(0),
                    (c0x + rc).min(w - 1),
                    (c0y - rc).max(0),
                    (c0y + rc).min(h - 1)
                );
                for cy in (c0y - rc).max(0)..=(c0y + rc).min(h - 1) {
                    let row: String = ((c0x - rc).max(0)..=(c0x + rc).min(w - 1))
                        .map(|cx| {
                            let f = flags_at(cx, cy);
                            if (cx, cy) == (c0x, c0y) {
                                '*'
                            } else if (f >> 4) & 0xf == f & 0xf {
                                char::from_digit(u32::from(f & 0xf), 16).unwrap_or('?')
                            } else {
                                '/'
                            }
                        })
                        .collect();
                    println!("      {cy:4} {row}");
                }
            }
            for (t, b) in best.iter().enumerate() {
                if let Some((d, wx, wy)) = b {
                    println!(
                        "    type {t:2}: nearest whole cell centre ({wx}, {wy}), cell ({}, {}), distance {}",
                        wx >> 9,
                        wy >> 9,
                        (*d as f64).sqrt() as i64
                    );
                }
            }
        }
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_choice_follows_the_mask_then_the_point() {
        assert_eq!(marker_sequence(0xf83c, 1, true), 0);
        assert_eq!(marker_sequence(0xf83e, 1, true), 4);
        assert_eq!(marker_sequence(0xf83e, 1, false), 4);
        assert_eq!(marker_sequence(0xf83c, 0, false), 1);
        assert_eq!(marker_sequence(0xf83c, 3, true), 4);
    }

    #[test]
    fn terrain_triangle_by_parity() {
        let f = |cx: i32, _cy: i32| if cx == 0 { 0x21 } else { 0x43 };
        assert_eq!(terrain_type_at(&f, 2, 1, 300, 100), 1);
        assert_eq!(terrain_type_at(&f, 2, 1, 100, 300), 2);
        assert_eq!(terrain_type_at(&f, 2, 1, 512 + 400, 300), 3);
        assert_eq!(terrain_type_at(&f, 2, 1, 512 + 10, 10), 4);
    }

    #[test]
    fn world_rect_of_a_centred_key_point() {
        assert_eq!(
            instance_world_rect(10 << 16, 16, 7, 32, 15),
            (70, -160, 160, -80)
        );
    }
}
