use mandrake_common::FourCC;
use mandrake_tag::{MarkerPaletteEntry, MeshTag};

const H: usize = 320;

fn be16(d: &[u8], o: usize) -> u16 {
    d.get(o..o + 2)
        .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], o: usize) -> i32 {
    d.get(o..o + 4)
        .map_or(0, |b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn fourcc(d: &[u8], o: usize) -> FourCC {
    FourCC::new([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

pub fn rgb_to_hsv(rgb: [u16; 3]) -> [u16; 3] {
    let k = 65535.0_f32;
    let (r, g, b) = (rgb[0] as f32 / k, rgb[1] as f32 / k, rgb[2] as f32 / k);
    let mx = b.max(g).max(r);
    let mn = b.min(g).min(r);
    if mx == 0.0 {
        return [0, 0, (k * mx) as i32 as u16];
    }
    let delta = mx - mn;
    let s = delta / mx;
    if s == 0.0 {
        return [0, (s * k) as i32 as u16, (k * mx) as i32 as u16];
    }
    let mut h = if mx == r {
        (g - b) / delta + 0.0
    } else if mx == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h /= 6.0;
    if 0.0 > h {
        h += 1.0;
    }
    h *= 65536.0;
    [
        h as i32 as u16,
        (s * k) as i32 as u16,
        (k * mx) as i32 as u16,
    ]
}

pub fn hsv_to_rgb(hsv: [u16; 3]) -> [u16; 3] {
    let k = 65535.0_f32;
    let s = hsv[1] as f32 / k;
    let v = hsv[2] as f32 / k;
    if s == 0.0 {
        let c = (v * k) as i32 as u16;
        return [c, c, c];
    }
    let h = hsv[0] as f32 * (1.0 / 65536.0) * 6.0;
    let mut i = h as i32;
    let mut fi = i as f32;
    if fi > h {
        i -= 1;
        fi = i as f32;
    }
    let f = h - fi;
    let (sd, vd, fd) = (f64::from(s), f64::from(v), f64::from(f));
    let p = ((1.0 - sd) * vd) as f32;
    let q = ((1.0 - f64::from(s * f)) * vd) as f32;
    let t = (vd * (1.0 - sd * (1.0 - fd))) as f32;
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [
        (r * k) as i32 as u16,
        (g * k) as i32 as u16,
        (b * k) as i32 as u16,
    ]
}

#[derive(Debug, Clone)]
pub struct HueChange {
    pub name: String,
    pub hue_range: (u16, u16),
    pub mask: [u8; 32],
    pub min_saturation: u16,
    pub base_hue: u16,
    pub flags: u16,
}

impl HueChange {
    pub fn affects(&self, i: usize) -> bool {
        (self.mask[i >> 3] >> (i & 7)) & 1 != 0
    }
}

pub fn color_table(coll: &[u8]) -> Vec<[u16; 4]> {
    let o = H + be32(coll, 0x44).max(0) as usize;
    let n = be32(coll, o).clamp(0, 256) as usize;
    (0..n)
        .map(|i| {
            let e = o + 0x20 + 8 * i;
            [
                be16(coll, e),
                be16(coll, e + 2),
                be16(coll, e + 4),
                be16(coll, e + 6),
            ]
        })
        .collect()
}

pub fn hue_changes(coll: &[u8]) -> Vec<HueChange> {
    let n = be32(coll, 0x50).max(0) as usize;
    let o = H + be32(coll, 0x54).max(0) as usize;
    (0..n)
        .filter_map(|j| {
            let r = coll.get(o + 0x80 * j..o + 0x80 * (j + 1))?;
            let end = r[..32].iter().position(|&c| c == 0).unwrap_or(32);
            let mut mask = [0u8; 32];
            mask.copy_from_slice(&r[0x44..0x64]);
            Some(HueChange {
                name: String::from_utf8_lossy(&r[..end]).into_owned(),
                hue_range: (be16(r, 0x40), be16(r, 0x42)),
                mask,
                min_saturation: be16(r, 0x64),
                base_hue: be16(r, 0x66),
                flags: be16(r, 0x68),
            })
        })
        .collect()
}

pub fn permutation_colors(core: &[u8], p: usize, hue_changes: usize) -> Vec<[u16; 4]> {
    (0..hue_changes)
        .map(|j| {
            let o = 8 + 0x40 * p + 8 * j;
            [
                be16(core, o),
                be16(core, o + 2),
                be16(core, o + 4),
                be16(core, o + 6),
            ]
        })
        .collect()
}

pub fn colorize(table: &[[u16; 4]], changes: &[HueChange], colors: &[[u16; 4]]) -> Vec<[u16; 3]> {
    let mut out: Vec<[u16; 3]> = table.iter().map(|c| [c[0], c[1], c[2]]).collect();
    for (hc, c) in changes.iter().zip(colors) {
        if c[3] & 4 != 0 {
            continue;
        }
        let [ch, cs, cv] = rgb_to_hsv([c[0], c[1], c[2]]);
        for (i, e) in out.iter_mut().enumerate() {
            if !hc.affects(i) {
                continue;
            }
            let [ih, is, iv] = rgb_to_hsv(*e);
            let h = if c[3] & 1 != 0 {
                ch.wrapping_add(hc.base_hue).wrapping_sub(ih)
            } else {
                ch.wrapping_add(ih).wrapping_sub(hc.base_hue)
            };
            *e = hsv_to_rgb([
                h,
                ((u32::from(is) * u32::from(cs)) >> 16) as u16,
                ((u32::from(iv) * u32::from(cv)) >> 16) as u16,
            ]);
        }
    }
    out
}

pub fn table_hash(table: &[[u16; 3]]) -> u32 {
    let mut h = 0x811c_9dc5_u32;
    for c in table.iter().skip(1) {
        for &b in c {
            h ^= u32::from((b >> 8) as u8);
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    h
}

fn rgb8(c: [u16; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0] >> 8, c[1] >> 8, c[2] >> 8)
}

pub fn colperm_report(
    mesh_id: FourCC,
    tags: &dyn Fn([u8; 4], FourCC) -> Option<Vec<u8>>,
) -> anyhow::Result<()> {
    let md = tags(*b"mesh", mesh_id).ok_or_else(|| anyhow::anyhow!("no mesh {mesh_id}"))?;
    let mesh = MeshTag::from_bytes(&md)?;
    println!(
        "# mesh {mesh_id}: colour permutations of its scenery (type 1) and unit (type 3) palette entries"
    );
    let mut seen = std::collections::BTreeSet::new();
    for (i, p) in mesh.palettes.iter().enumerate() {
        let kind = match p.marker_type {
            MarkerPaletteEntry::TYPE_SCENERY => "scen",
            MarkerPaletteEntry::TYPE_UNIT => "unit",
            _ => continue,
        };
        let group = if kind == "scen" { *b"scen" } else { *b"unit" };
        let Some(def) = tags(group, p.tag_id) else {
            println!("\n## palette {i}: {kind} {} (missing)", p.tag_id);
            continue;
        };
        let core_id = fourcc(&def, 4);
        let markers = mesh
            .markers
            .iter()
            .filter(|m| m.marker_type == p.marker_type && mesh.palette_for_marker(m) == Some(p))
            .count();
        println!(
            "\n## palette {i}: {kind} {} (flags {:#x}, {markers} markers) -> core {core_id}",
            p.tag_id, p.flags
        );
        if !seen.insert(core_id.to_string()) {
            println!("   (core listed above)");
            continue;
        }
        let Some(core) = tags(*b"core", core_id) else {
            println!("   core {core_id} missing");
            continue;
        };
        let coll_id = fourcc(&core, 0);
        let count = be16(&core, 4) as i16;
        println!(
            "   core +0x00 collection {coll_id}, +0x04 permutations {count}, +0x06 tint fraction {}, +0x148 tint colour {:04x} {:04x} {:04x} flags {:04x}, +0x150 flags {:#x}",
            be16(&core, 6) as i16,
            be16(&core, 0x148),
            be16(&core, 0x14a),
            be16(&core, 0x14c),
            be16(&core, 0x14e),
            be32(&core, 0x150)
        );
        let Some(coll) = tags(*b".256", coll_id) else {
            println!("   collection {coll_id} missing");
            continue;
        };
        let table = color_table(&coll);
        let changes = hue_changes(&coll);
        println!(
            "   collection {coll_id}: {} colour tables, {} colours in table 0, {} hue changes",
            be32(&coll, 0x40),
            table.len(),
            changes.len()
        );
        for (j, hc) in changes.iter().enumerate() {
            let idx: Vec<usize> = (0..table.len()).filter(|&i| hc.affects(i)).collect();
            println!(
                "   hue change {j} '{}': hue {}..{} base {} min saturation {} flags {:#x} ({}), {} colours: {:?}",
                hc.name,
                hc.hue_range.0,
                hc.hue_range.1,
                hc.base_hue,
                hc.min_saturation,
                hc.flags,
                match hc.flags & 3 {
                    1 => "primary",
                    2 => "secondary",
                    3 => "primary+secondary",
                    _ => "none",
                },
                idx.len(),
                idx
            );
        }
        let raw: Vec<[u16; 3]> = table.iter().map(|c| [c[0], c[1], c[2]]).collect();
        println!("   raw table 0: hash {:08x}", table_hash(&raw));
        for perm in 0..count.max(0) as usize {
            let colors = permutation_colors(&core, perm, changes.len());
            let out = colorize(&table, &changes, &colors);
            let desc: Vec<String> = colors
                .iter()
                .map(|c| {
                    let [h, s, v] = rgb_to_hsv([c[0], c[1], c[2]]);
                    format!(
                        "{:04x} {:04x} {:04x} flags {:04x} (hsv {h} {s} {v}{})",
                        c[0],
                        c[1],
                        c[2],
                        c[3],
                        if c[3] & 4 != 0 { ", off" } else { "" }
                    )
                })
                .collect();
            println!(
                "   perm {perm}: hash {:08x}; colours [{}]",
                table_hash(&out),
                desc.join("; ")
            );
            for (j, hc) in changes.iter().enumerate() {
                let sample: Vec<String> = (0..table.len())
                    .filter(|&i| hc.affects(i))
                    .take(6)
                    .map(|i| format!("[{i}] {}->{}", rgb8(raw[i]), rgb8(out[i])))
                    .collect();
                println!("      hue change {j}: {}", sample.join(" "));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip_and_grey() {
        assert_eq!(rgb_to_hsv([0, 0, 0]), [0, 0, 0]);
        assert_eq!(rgb_to_hsv([0x8000, 0x8000, 0x8000])[1], 0);
        assert_eq!(hsv_to_rgb([0, 0, 0x8000]), [0x8000, 0x8000, 0x8000]);
        assert_eq!(rgb_to_hsv([0xffff, 0, 0]), [0, 0xffff, 0xffff]);
        assert_eq!(rgb_to_hsv([0, 0xffff, 0])[0], 21845);
        assert_eq!(rgb_to_hsv([0, 0, 0xffff])[0], 43690);
    }

    #[test]
    fn colorize_masks_and_disabled_colours() {
        let table = vec![[0, 0, 0, 0], [0, 0xffff, 0, 0], [0, 0xffff, 0, 0]];
        let mut mask = [0u8; 32];
        mask[0] = 0b010;
        let hc = HueChange {
            name: "Hue Change 1".into(),
            hue_range: (16384, 27307),
            mask,
            min_saturation: 0,
            base_hue: 21845,
            flags: 1,
        };
        let out = colorize(&table, std::slice::from_ref(&hc), &[[0x451e, 0, 0x0378, 0]]);
        assert_eq!(out[2], [0, 0xffff, 0]);
        assert!(
            out[1][0] > out[1][1] && out[1][0] > out[1][2],
            "{:?}",
            out[1]
        );
        let off = colorize(&table, &[hc], &[[0x451e, 0, 0x0378, 4]]);
        assert_eq!(off[1], [0, 0xffff, 0]);
    }
}
