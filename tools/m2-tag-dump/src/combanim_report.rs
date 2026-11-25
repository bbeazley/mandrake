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
    (b != [0xff; 4]).then(|| FourCC::new([b[0], b[1], b[2], b[3]]))
}

fn show(t: Option<FourCC>) -> String {
    t.map_or_else(|| "-".to_string(), |t| t.to_string())
}

fn seq_text(coll: Option<&CollectionTag>, seq: i16) -> String {
    if seq < 0 {
        return "-".to_string();
    }
    match coll.and_then(|c| c.sequence(seq as usize)) {
        Some(s) => format!(
            "{seq} '{}' ({} views, {} frames x {} ticks = {} ticks, key {}, loop {}, flags {:#x})",
            s.name,
            s.num_views,
            s.frames_per_view,
            s.ticks_per_frame,
            i32::from(s.frames_per_view) * i32::from(s.ticks_per_frame.max(1)),
            s.key_frame,
            s.loop_frame,
            s.flags
        ),
        None => format!("{seq} (not in the collection)"),
    }
}

fn collection(tags: Tags, id: Option<FourCC>) -> Option<CollectionTag> {
    CollectionTag::from_bytes(&tags(*b".256", id?)?).ok()
}

const MODE_SLOTS: [(&str, usize); 8] = [
    ("mode 1 idle (+0x08)", 0x08),
    ("mode 2 celebrate (+0x1a)", 0x1a),
    ("mode 3 taunt (+0x28)", 0x28),
    ("mode 6 hold (+0x26)", 0x26),
    ("mode 9 block (+0x12)", 0x12),
    ("mode 10 (+0x16)", 0x16),
    ("modes 11/12/14/15 hit reaction (+0x14)", 0x14),
    ("mode 16 (+0x2c, else +0x14)", 0x2c),
];

const GROUPS: [(&str, usize); 6] = [
    ("death code 0x26 (+0x1a4 -> +0x3ec)", 0x1a4),
    ("death code 0x27 (+0x194 -> +0x3e8)", 0x194),
    ("death code 0x29 (+0x3c -> +0x40)", 0x3c),
    ("impact group (+0x1a0 -> +0x3ea)", 0x1a0),
    ("+0x1e4 -> +0x3ee", 0x1e4),
    ("+0x1e8 -> +0x3f0", 0x1e8),
];

pub fn report_mons(tags: Tags, mons_id: FourCC) -> anyhow::Result<()> {
    let m = tags(*b"mons", mons_id).ok_or_else(|| anyhow::anyhow!("no mons {mons_id}"))?;
    let coll_id = tag_ref(&m, 4);
    let coll = collection(tags, coll_id);
    println!(
        "mons {mons_id}: collection {} flags {:#010x}, +0x19c death-code threshold {:#06x}, +0x19e hit-reaction threshold {:#06x}",
        show(coll_id),
        be32(&m, 0),
        be16(&m, 0x19c) as u16,
        be16(&m, 0x19e) as u16
    );
    for (label, off) in MODE_SLOTS {
        println!("  {label:42} {}", seq_text(coll.as_ref(), be16(&m, off)));
    }
    let n = be16(&m, 0x88).max(0) as usize;
    for a in 0..n {
        let o = 0x8c + 0x40 * a;
        println!(
            "  attack {a}: flags {:#06x} projectile {} range {}..{} repetitions {}",
            be16(&m, o) as u16,
            show(tag_ref(&m, o + 4)),
            be16(&m, o + 8),
            be16(&m, o + 0xa),
            be16(&m, o + 0x2c)
        );
        for k in 0..4 {
            let flags = be16(&m, o + 0xc + 8 * k) as u16;
            let seq = be16(&m, o + 0xe + 8 * k);
            if seq < 0 {
                continue;
            }
            let mask = match flags & 7 {
                1 => "level (elevation < 0x1000 or >= 0xfc00)",
                2 => "down (0x8000..=0xfbff)",
                4 => "up (0x1000..=0x7fff)",
                _ => "mixed",
            };
            println!(
                "    pair {k}: flags {flags:#06x} [{mask}] {}",
                seq_text(coll.as_ref(), seq)
            );
        }
    }
    for (label, off) in GROUPS {
        let g = tag_ref(&m, off);
        println!("  {label:36} {}", show(g));
        if off != 0x1a0 && g.is_some() {
            report_group(tags, g, 2, 0)?;
        }
    }
    Ok(())
}

fn report_group(tags: Tags, id: Option<FourCC>, indent: usize, depth: usize) -> anyhow::Result<()> {
    let pad = " ".repeat(indent);
    let Some(id) = id else { return Ok(()) };
    let Some(d) = tags(*b"prgr", id) else {
        println!("{pad}  prgr {id} missing");
        return Ok(());
    };
    let g = ProjectileGroupDefinition::from_bytes(id, &d)?;
    println!(
        "{pad}  prgr {id}: flags {:#x}, {} parts, mesh effect {}, sound {}, local group {}",
        g.flags,
        g.parts.len(),
        show(g.mesh_effect),
        show(g.sound),
        show(g.local_group)
    );
    for (i, part) in g.parts.iter().enumerate() {
        let raw: Vec<String> = part.raw.iter().map(|b| format!("{b:02x}")).collect();
        println!("{pad}    part {i}: raw {}", raw.join(""));
        report_proj(tags, part.projectile, indent + 6, depth)?;
    }
    Ok(())
}

fn report_proj(tags: Tags, id: Option<FourCC>, indent: usize, depth: usize) -> anyhow::Result<()> {
    let pad = " ".repeat(indent);
    let Some(id) = id else { return Ok(()) };
    let Some(p) = tags(*b"proj", id) else {
        println!("{pad}proj {id} missing");
        return Ok(());
    };
    let flags = be32(&p, 0);
    let coll_id = tag_ref(&p, 4);
    let coll = collection(tags, coll_id);
    let mut traits = Vec::new();
    if flags & 1 != 0 {
        traits.push("inherits owner shading");
    }
    if flags & 8 != 0 {
        traits.push("detonates when its sequence wraps");
    }
    if flags & 0x100 != 0 {
        traits.push("dormant object at rest");
    }
    let lifetime = (be16(&p, 0x50), be16(&p, 0x52));
    println!(
        "{pad}proj {id}: flags {flags:#010x} [{}] collection {} sequence {}",
        traits.join(", "),
        show(coll_id),
        seq_text(coll.as_ref(), be16(&p, 8))
    );
    println!(
        "{pad}  speed {:#x} + {:#x}, lifetime {} + {} ticks{}, impact frequency {:#06x}, detonation chance {:#06x}, detonation group {}, replacement (+0x5c) {}",
        be16(&p, 0x4c),
        be16(&p, 0x4e),
        lifetime.0,
        lifetime.1,
        if lifetime == (0, 0) { " (none)" } else { "" },
        be16(&p, 0x2a) as u16,
        be16(&p, 0x2c) as u16,
        show(tag_ref(&p, 0x10)),
        show(tag_ref(&p, 0x5c))
    );
    if depth < 3 {
        if let Some(r) = tag_ref(&p, 0x5c) {
            println!("{pad}  replaced by:");
            report_proj(tags, Some(r), indent + 4, depth + 1)?;
        }
        if flags & 8 != 0
            && let Some(gid) = tag_ref(&p, 0x10)
        {
            println!("{pad}  detonation group:");
            report_group(tags, Some(gid), indent + 2, depth + 1)?;
        }
    }
    Ok(())
}

pub fn combanim_report(mesh_id: FourCC, tags: Tags) -> anyhow::Result<()> {
    let md = tags(*b"mesh", mesh_id).ok_or_else(|| anyhow::anyhow!("no mesh {mesh_id}"))?;
    let mesh = MeshTag::from_bytes(&md)?;
    println!("# mesh {mesh_id}: combat sequences and corpses of its unit (type 3) palette entries");
    let mut seen = std::collections::BTreeSet::new();
    for (i, p) in mesh.palettes.iter().enumerate() {
        if p.marker_type != MarkerPaletteEntry::TYPE_UNIT {
            continue;
        }
        let Some(unit) = tags(*b"unit", p.tag_id) else {
            println!("\n## palette {i}: unit {} (missing)", p.tag_id);
            continue;
        };
        let Some(mons) = tag_ref(&unit, 0) else {
            continue;
        };
        println!("\n## palette {i}: unit {} -> mons {mons}", p.tag_id);
        if !seen.insert(mons.to_string()) {
            println!("   (mons listed above)");
            continue;
        }
        report_mons(tags, mons)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_refs_and_words() {
        let d = [0xff, 0xff, 0xff, 0xff, b'w', b'a', b'm', b'c', 0x01, 0x00];
        assert_eq!(tag_ref(&d, 0), None);
        assert_eq!(tag_ref(&d, 4).map(|t| t.to_string()), Some("wamc".into()));
        assert_eq!(be16(&d, 8), 0x100);
        assert_eq!(be16(&d, 9), -1, "short read gives -1");
    }

    #[test]
    fn sequence_text_without_collection() {
        assert_eq!(seq_text(None, -1), "-");
        assert_eq!(seq_text(None, 4), "4 (not in the collection)");
    }
}
