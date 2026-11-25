use mandrake_common::FourCC;
use mandrake_tag::{CollectionTag, MarkerPaletteEntry, MeshTag, ProjectileGroupDefinition};

type Tags<'a> = &'a dyn Fn([u8; 4], FourCC) -> Option<Vec<u8>>;

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

fn show(t: Option<FourCC>) -> String {
    t.map_or_else(|| "-".to_string(), |t| t.to_string())
}

const FLAG_NAMES: [(u32, &str); 18] = [
    (0x1, "inherit-shading"),
    (0x2, "homing"),
    (0x4, "gore"),
    (0x8, "detonate-on-wrap"),
    (0x10, "detonate-on-event-8"),
    (0x40, "wind"),
    (0x100, "dormant-at-rest"),
    (0x200, "random-0x80000"),
    (0x800, "random-mirror"),
    (0x1000, "replace-on-lifetime-end"),
    (0x2000, "melee"),
    (0x8000, "bloody"),
    (0x10000, "fuse"),
    (0x80000, "no-y-mirror"),
    (0x100000, "obj-0x1000"),
    (0x200000, "on-media-surface"),
    (0x1000000, "lightning"),
    (0x2000000, "flag-0x2000000"),
];

fn flag_text(flags: u32) -> String {
    let names: Vec<&str> = FLAG_NAMES
        .iter()
        .filter(|(b, _)| flags & b != 0)
        .map(|(_, n)| *n)
        .collect();
    names.join(" ")
}

fn instance_flags(data: &[u8], seq: usize) -> Option<Vec<u32>> {
    let h = CollectionTag::HEADER_SIZE;
    let rd = |o: usize| be32(data, o) as usize;
    let (binst_count, binst_off) = (rd(0x70), rd(0x74));
    let (seq_count, seq_off) = (rd(0x80), rd(0x84));
    if seq >= seq_count {
        return None;
    }
    let sref = h + seq_off + seq * 128;
    let shdr = h + rd(sref + 64);
    let views = be16(data, shdr + 8).max(0) as usize;
    let frames = be16(data, shdr + 10).max(0) as usize;
    let stride = 46 + 2 * views;
    let mut out = Vec::new();
    for f in 0..frames {
        for v in 0..views {
            let inst = be16(data, shdr + 64 + f * stride + 46 + 2 * v);
            if inst >= 0 && (inst as usize) < binst_count {
                out.push(be32(data, h + binst_off + 64 * inst as usize));
            }
        }
    }
    Some(out)
}

fn seq_transfer(data: &[u8], seq: usize) -> Option<(i16, i16)> {
    let h = CollectionTag::HEADER_SIZE;
    let rd = |o: usize| be32(data, o) as usize;
    if seq >= rd(0x80) {
        return None;
    }
    let shdr = h + rd(h + rd(0x84) + seq * 128 + 64);
    Some((be16(data, shdr + 0x24), be16(data, shdr + 0x26)))
}

fn seq_line(tags: Tags, coll_id: Option<FourCC>, seq: i16) -> String {
    if seq < 0 {
        return "-".to_string();
    }
    let Some(raw) = coll_id.and_then(|c| tags(*b".256", c)) else {
        return format!("{seq} (no collection)");
    };
    let Ok(coll) = CollectionTag::from_bytes(&raw) else {
        return format!("{seq} (collection does not parse)");
    };
    let Some(s) = coll.sequence(seq as usize) else {
        return format!("{seq} (not in the collection)");
    };
    let inst = instance_flags(&raw, seq as usize).unwrap_or_default();
    let quads = inst.iter().filter(|f| *f & 0x10 != 0).count();
    let alpha = s
        .bitmap_indices
        .iter()
        .filter(|&&b| coll.frame(b).is_some_and(|f| f.flags & 0x10 != 0))
        .count();
    let (mode, period) = seq_transfer(&raw, seq as usize).unwrap_or((-1, -1));
    let view_rule = if s.flags & 4 != 0 {
        "stored view"
    } else {
        "camera view"
    };
    format!(
        "{seq} '{}' ({} views, {} frames x {} ticks, key {}, loop {}, flags {:#x}: {view_rule}{}, transfer mode +0x24 {mode} period +0x26 {period}) instances {}: {quads} velocity-aligned (+0x00 & 0x10); bitmaps with 16-bit alpha (bitmap flags 0x10): {alpha} of {}",
        s.name,
        s.num_views,
        s.frames_per_view,
        s.ticks_per_frame,
        s.key_frame,
        s.loop_frame,
        s.flags,
        if s.flags & 1 != 0 { ", mirrored" } else { "" },
        inst.len(),
        s.bitmap_indices.len()
    )
}

struct Walker<'a> {
    tags: Tags<'a>,
    seen: std::collections::BTreeSet<String>,
}

impl Walker<'_> {
    fn proj(&mut self, id: Option<FourCC>, pad: usize, why: &str) {
        let Some(id) = id else { return };
        let p = " ".repeat(pad);
        if !self.seen.insert(format!("proj {id}")) {
            println!("{p}proj {id} ({why}): listed above");
            return;
        }
        let Some(d) = (self.tags)(*b"proj", id) else {
            println!("{p}proj {id} ({why}): missing");
            return;
        };
        let flags = be32(&d, 0);
        let coll = tag_ref(&d, 4);
        println!(
            "{p}proj {id} ({why}): flags {flags:#010x} [{}] collection {}",
            flag_text(flags),
            show(coll)
        );
        println!(
            "{p}  flight sequence +0x08: {}",
            seq_line(self.tags, coll, be16(&d, 8))
        );
        for (o, label) in [
            (0x0a, "detonation stain +0x0a"),
            (0x0c, "rolling stain +0x0c"),
        ] {
            if be16(&d, o) >= 0 {
                println!("{p}  {label}: {}", seq_line(self.tags, coll, be16(&d, o)));
            }
        }
        let obje = tag_ref(&d, 0x1c);
        let (scale, scale_d, gravity, elasticity, terminal, oflags) = obje
            .and_then(|o| (self.tags)(*b"obje", o))
            .map_or((-1, -1, 0, 0, 0, 0), |o| {
                (
                    be16(&o, 0xc),
                    be16(&o, 0xe),
                    be16(&o, 2),
                    be16(&o, 4),
                    be16(&o, 6),
                    be16(&o, 0) as u16,
                )
            });
        println!(
            "{p}  obje {}: scale {scale} + {scale_d} (8.8), gravity {gravity}, elasticity {elasticity}, terminal {terminal}, flags {oflags:#06x}",
            show(obje)
        );
        println!(
            "{p}  animation speed {:#x} + {:#x}, lifetime {} + {}, countdown +0x84 {} + {}, +0x32 {}{}, media group type +0x0e {}, +0x28 {}, impact frequency {:#06x}",
            be16(&d, 0x4c),
            be16(&d, 0x4e),
            be16(&d, 0x50),
            be16(&d, 0x52),
            be16(&d, 0x84),
            be16(&d, 0x86),
            be16(&d, 0x32),
            if matches!(be16(&d, 0x32), 8 | 9) {
                " (no media clip / submerged copy)"
            } else {
                ""
            },
            be16(&d, 0x0e),
            be16(&d, 0x28),
            be16(&d, 0x2a) as u16
        );
        let refs = [
            (0x10, "detonation group +0x10"),
            (0x60, "group +0x60"),
            (0x7c, "group +0x7c"),
            (0x80, "group +0x80 (-> +0xe2 -> object +0x3c)"),
        ];
        for (o, label) in refs {
            if let Some(g) = tag_ref(&d, o) {
                println!("{p}  {label}: prgr {g}");
            }
        }
        for (o, label) in [
            (0x34, "lightning +0x34"),
            (0x88, "local group +0x88"),
            (0x78, "artifact +0x78"),
        ] {
            if let Some(t) = tag_ref(&d, o) {
                println!("{p}  {label}: {t}");
            }
        }
        if let Some(t) = tag_ref(&d, 0x14) {
            println!(
                "{p}  trail +0x14 every {} ticks while in flight (+0x18 / +0x1a {} / {}):",
                be16(&d, 0x18),
                be16(&d, 0x18),
                be16(&d, 0x1a)
            );
            self.proj(Some(t), pad + 4, "trail");
        }
        if let Some(t) = tag_ref(&d, 0x5c) {
            self.proj(Some(t), pad + 2, "replacement +0x5c");
        }
        if let Some(g) = tag_ref(&d, 0x10) {
            self.group(Some(g), pad + 2, "detonation group");
        }
        if let Some(g) = tag_ref(&d, 0x60) {
            self.group(Some(g), pad + 2, "group +0x60");
        }
    }

    fn group(&mut self, id: Option<FourCC>, pad: usize, why: &str) {
        let Some(id) = id else { return };
        let p = " ".repeat(pad);
        if !self.seen.insert(format!("prgr {id}")) {
            println!("{p}prgr {id} ({why}): listed above");
            return;
        }
        let Some(d) = (self.tags)(*b"prgr", id) else {
            println!("{p}prgr {id} ({why}): missing");
            return;
        };
        let Ok(g) = ProjectileGroupDefinition::from_bytes(id, &d) else {
            println!("{p}prgr {id}: does not parse");
            return;
        };
        println!(
            "{p}prgr {id} ({why}): flags {:#x}, {} parts, mesh effect {}, sound {}, local group {}",
            g.flags,
            g.parts.len(),
            show(g.mesh_effect),
            show(g.sound),
            show(g.local_group)
        );
        for (i, part) in g.parts.iter().enumerate() {
            let r = &part.raw;
            println!(
                "{p}  part {i}: flags {:#x} (part +0x04), count {} + {} (part +0x08/+0x0a), raw {}",
                be32(r, 4),
                be16(r, 8),
                be16(r, 0xa),
                r.iter().map(|b| format!("{b:02x}")).collect::<String>()
            );
            self.proj(part.projectile, pad + 4, "part");
        }
    }
}

pub fn projfx_report(
    tags: Tags,
    meshes: &[FourCC],
    medias: &[FourCC],
    extra: &[FourCC],
) -> anyhow::Result<()> {
    let mut w = Walker {
        tags,
        seen: Default::default(),
    };
    for &mesh_id in meshes {
        let md = tags(*b"mesh", mesh_id).ok_or_else(|| anyhow::anyhow!("no mesh {mesh_id}"))?;
        let mesh = MeshTag::from_bytes(&md)?;
        println!("# mesh {mesh_id}: attack projectiles of its unit palette entries");
        for p in &mesh.palettes {
            if p.marker_type != MarkerPaletteEntry::TYPE_UNIT {
                continue;
            }
            let Some(mons) = tags(*b"unit", p.tag_id).and_then(|u| tag_ref(&u, 0)) else {
                continue;
            };
            if !w.seen.insert(format!("mons {mons}")) {
                continue;
            }
            let Some(m) = tags(*b"mons", mons) else {
                continue;
            };
            let n = be16(&m, 0x88).max(0) as usize;
            println!("\n## unit {} -> mons {mons}: {n} attacks", p.tag_id);
            for a in 0..n {
                let o = 0x8c + 0x40 * a;
                w.proj(tag_ref(&m, o + 4), 2, &format!("attack {a}"));
            }
        }
        println!();
    }
    for &medi in medias {
        let Some(d) = tags(*b"medi", medi) else {
            println!("# medi {medi}: missing");
            continue;
        };
        println!("# medi {medi}: projectile groups (+0x18 + 4 * type)");
        for t in 0..16 {
            if let Some(g) = tag_ref(&d, 0x18 + 4 * t) {
                w.group(Some(g), 2, &format!("media type {t}"));
            }
        }
        println!();
    }
    for &p in extra {
        println!("# extra proj {p}");
        w.proj(Some(p), 2, "extra");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_names() {
        assert_eq!(flag_text(0x21), "inherit-shading");
        assert_eq!(flag_text(0x10a00), "random-0x80000 random-mirror fuse");
    }

    #[test]
    fn refs_none_for_zero_and_minus_one() {
        let d = [0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, b'a', b'r', b'a', b'r'];
        assert_eq!(tag_ref(&d, 0), None);
        assert_eq!(tag_ref(&d, 4), None);
        assert_eq!(tag_ref(&d, 8).map(|t| t.to_string()), Some("arar".into()));
        assert_eq!(instance_flags(&[], 0), None);
    }
}
