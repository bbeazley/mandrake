use mandrake_tag::{MeshTag, TagArchive, TagEntry};
use std::collections::BTreeMap;

fn be16(d: &[u8], o: usize) -> u16 {
    d.get(o..o + 2)
        .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandscapeShading {
    pub dark_weight: i16,
    pub light_weight: i16,
    pub dark_color: [u16; 3],
    pub light_color: [u16; 3],
    pub transition: u32,
}

impl LandscapeShading {
    pub fn from_mesh_bytes(d: &[u8]) -> Self {
        Self {
            dark_weight: be16(d, 0x58) as i16,
            light_weight: be16(d, 0x5a) as i16,
            dark_color: [be16(d, 0x5c), be16(d, 0x5e), be16(d, 0x60)],
            light_color: [be16(d, 0x64), be16(d, 0x66), be16(d, 0x68)],
            transition: be32(d, 0x6c),
        }
    }

    pub fn transition_level(&self, n: u32) -> u32 {
        ((self.transition as u64 * (n as u64 - 1)) >> 16) as u32
    }
}

#[derive(Debug, Default)]
struct ShadeStats {
    min: u8,
    max: u8,
    mean: f64,
    at_transition: f64,
    above: f64,
}

fn shade_stats(tex: &[u8], submeshes: usize, m: u32) -> Option<ShadeStats> {
    let secs = be32(tex, 96) as usize;
    let so = be32(tex, 100) as usize;
    let mut hist = [0u64; 256];
    for k in 0..submeshes {
        let i = 2 * k + 1;
        if i >= secs {
            return None;
        }
        let base = 0x140 + be32(tex, 0x140 + so + i * 128 + 64) as usize + 52;
        for &s in tex.get(base..base + 65536)? {
            hist[s as usize] += 1;
        }
    }
    let n: u64 = hist.iter().sum();
    if n == 0 {
        return None;
    }
    let min = hist.iter().position(|&c| c > 0)? as u8;
    let max = 255 - hist.iter().rev().position(|&c| c > 0)? as u8;
    let mean = hist
        .iter()
        .enumerate()
        .map(|(v, &c)| v as u64 * c)
        .sum::<u64>() as f64
        / n as f64;
    let at = hist.get(m as usize).map_or(0.0, |&c| c as f64 / n as f64);
    let above = hist
        .get(m as usize + 1..)
        .map_or(0, |h| h.iter().sum::<u64>()) as f64
        / n as f64;
    Some(ShadeStats {
        min,
        max,
        mean,
        at_transition: at,
        above,
    })
}

fn collect<'a>(
    archives: &'a [(String, TagArchive)],
    ty: &str,
) -> BTreeMap<String, (&'a str, &'a TagArchive, &'a TagEntry)> {
    let mut out = BTreeMap::new();
    for (name, a) in archives {
        for e in a.list_tags() {
            if e.tag_type.as_str().eq_ignore_ascii_case(ty) {
                out.insert(e.tag_id.as_str().trim().to_string(), (name.as_str(), a, e));
            }
        }
    }
    out
}

pub fn lighting_report(archives: &[(String, TagArchive)]) -> anyhow::Result<()> {
    let meshes = collect(archives, "mesh");
    let textures = collect(archives, ".256");
    println!("# mesh header landscape shading (initialize_cmap_shading_tables_for_new_map,");
    println!("# build_colored_shading_table); m = transition level of the 256-level table;");
    println!(
        "# shade = landscape shading-map bytes (.256 odd sections): min max mean, share == m,"
    );
    println!("# share > m (toward the light colour). '-' = landscape .256 not in these archives.");
    println!(
        "{:4} | {:28} | {:4} | {:>4} {:>4} | {:>17} | {:>17} | {:>8} | {:>3} | shade min max mean =m >m",
        "mesh", "name (archive)", "land", "+58", "+5a", "+5c dark", "+64 light", "+6c", "m"
    );
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, (src, a, e)) in &meshes {
        let d = a.get_tag_data(e)?;
        let mesh = MeshTag::from_bytes(&d)?;
        let s = LandscapeShading::from_mesh_bytes(&d);
        let m = s.transition_level(256);
        let land = mesh.landscape_tag.to_string();
        let stats = textures.get(land.trim()).and_then(|(_, ta, te)| {
            let t = ta.get_tag_data(te).ok()?;
            shade_stats(&t, mesh.submesh_w as usize * mesh.submesh_h as usize, m)
        });
        let st = stats.map_or("-".to_string(), |x| {
            format!(
                "{:3} {:3} {:6.1} {:5.1}% {:5.1}%",
                x.min,
                x.max,
                x.mean,
                100.0 * x.at_transition,
                100.0 * x.above
            )
        });
        let key = format!(
            "{} {} {:?} {:?} {:#x}",
            s.dark_weight, s.light_weight, s.dark_color, s.light_color, s.transition
        );
        groups.entry(key).or_default().push(id.clone());
        println!(
            "{:4} | {:28} | {:4} | {:4} {:4} | {:5},{:5},{:5} | {:5},{:5},{:5} | {:8x} | {:3} | {}",
            id,
            format!("{} ({})", e.name, src)
                .chars()
                .take(28)
                .collect::<String>(),
            land,
            s.dark_weight,
            s.light_weight,
            s.dark_color[0],
            s.dark_color[1],
            s.dark_color[2],
            s.light_color[0],
            s.light_color[1],
            s.light_color[2],
            s.transition,
            m,
            st
        );
    }
    println!();
    println!("# fog (mesh +0x1f0 colour, +0x1f8 density): meshes with a non-zero density");
    for (id, (src, a, e)) in &meshes {
        let d = a.get_tag_data(e)?;
        let fog = mandrake_tag::MeshFog::from_mesh_bytes(&d);
        if fog.density() != 0.0 {
            println!(
                "fog {:4} | {:28} | {:5},{:5},{:5} | {:?}",
                id,
                format!("{} ({})", e.name, src)
                    .chars()
                    .take(28)
                    .collect::<String>(),
                fog.color[0],
                fog.color[1],
                fog.color[2],
                fog.density()
            );
        }
    }
    println!();
    println!("# distinct parameter sets (a b dark light k): meshes");
    for (k, v) in &groups {
        println!("{k}: {} ({})", v.join(" "), v.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_fields_and_transition_level() {
        let mut d = vec![0u8; 0x70];
        d[0x58..0x70].copy_from_slice(&[
            0x00, 0xe5, 0x00, 0x19, 0, 0, 0, 0, 0x0f, 0x5c, 0, 0, 0xff, 0xff, 0xa1, 0x47, 0, 0, 0,
            0, 0, 0, 0xf3, 0x33,
        ]);
        let s = LandscapeShading::from_mesh_bytes(&d);
        assert_eq!((s.dark_weight, s.light_weight), (229, 25));
        assert_eq!(s.dark_color, [0, 0, 3932]);
        assert_eq!(s.light_color, [65535, 41287, 0]);
        assert_eq!(s.transition, 0xf333);
        assert_eq!(s.transition_level(256), 242);
        assert_eq!(s.transition_level(64), 59);
    }
}
