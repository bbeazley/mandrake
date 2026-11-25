use mandrake_common::FourCC;
use mandrake_tag::{MeshTag, TagArchive, TagEntry};
use std::collections::BTreeMap;

pub const MESH_LOD_SECTION: usize = 0xcc;
pub const MESH_HEADER_FLAGS: usize = 0x4c;

fn be16(d: &[u8], o: usize) -> i16 {
    d.get(o..o + 2)
        .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn tag(d: &[u8], o: usize) -> String {
    match d.get(o..o + 4) {
        Some(b) if b != [0xff; 4] && b != [0; 4] => {
            FourCC::new([b[0], b[1], b[2], b[3]]).to_string()
        }
        _ => "-".to_string(),
    }
}

pub fn lod_words(data: &[u8]) -> Vec<u16> {
    let off = be32(data, MESH_LOD_SECTION) as usize;
    let size = be32(data, MESH_LOD_SECTION + 4) as usize;
    let start = MeshTag::HEADER_SIZE + off;
    data.get(start..start + size)
        .map(|s| {
            s.as_chunks::<2>()
                .0
                .iter()
                .map(|w| u16::from_be_bytes(*w))
                .collect()
        })
        .unwrap_or_default()
}

pub fn media_bits_from_cells(c00: u16, c10: u16, c11: u16, c01: u16) -> u16 {
    let b = |f: u16, m: u16, bit: u16| if f & m != 0 { 1 << bit } else { 0 };
    b(c00, 0x0800, 15)
        | b(c00, 0x1000, 8)
        | b(c10, 0x0800, 9)
        | b(c10, 0x1000, 10)
        | b(c11, 0x1000, 11)
        | b(c11, 0x0800, 12)
        | b(c01, 0x1000, 13)
        | b(c01, 0x0800, 14)
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

pub fn media_report(archives: &[(String, TagArchive)]) -> anyhow::Result<()> {
    let media = collect(archives, "medi");
    let meshes = collect(archives, "mesh");

    let mut users: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut mesh_rows = Vec::new();
    for (id, (src, a, e)) in &meshes {
        let data = a.get_tag_data(e)?;
        let mesh = MeshTag::from_bytes(&data)?;
        let medi = mesh.media_tag.to_string();
        users.entry(medi.clone()).or_default().push(id.clone());
        mesh_rows.push((id.clone(), src.to_string(), e.name.clone(), data, mesh));
    }

    println!(
        "# medi definitions (big-endian tag payload = the engine's in-memory record before it is processed)"
    );
    println!(
        "{:4} | {:24} | {:>8} | {:4} | {:>6} {:16} | {:>5} | {:4} | {:>17} | {:>17} | {:>4} {:>4} | {:>6} | prgr[4,5,6] | used by",
        "id",
        "name",
        "+00 flg",
        "core",
        "+58",
        "+5a data",
        "+62",
        "lpgr",
        "+68 amp x,y,z",
        "+6e freq x,y,z",
        "+74",
        "+76",
        "+78"
    );
    for (id, (_src, a, e)) in &media {
        let d = a.get_tag_data(e)?;
        let data: String = d[0x5a..0x62].iter().map(|b| format!("{b:02x}")).collect();
        println!(
            "{:4} | {:24} | {:08x} | {:4} | {:6} {:16} | {:5} | {:4} | {:5},{:5},{:5} | {:5},{:5},{:5} | {:4} {:4} | {:6} | {},{},{} | {}",
            id,
            e.name,
            be32(&d, 0),
            tag(&d, 4),
            be16(&d, 0x58),
            data,
            be16(&d, 0x62) as u16,
            tag(&d, 0x64),
            be16(&d, 0x68),
            be16(&d, 0x6a),
            be16(&d, 0x6c),
            be16(&d, 0x6e),
            be16(&d, 0x70),
            be16(&d, 0x72),
            be16(&d, 0x74),
            be16(&d, 0x76),
            be16(&d, 0x78) as u16,
            tag(&d, 0x18 + 16),
            tag(&d, 0x18 + 20),
            tag(&d, 0x18 + 24),
            users.get(id).map_or(String::new(), |v| v.join(" ")),
        );
    }
    println!();
    println!("# medi bytes not covered above (offset: value), all other bytes zero");
    for (id, (_src, a, e)) in &media {
        let d = a.get_tag_data(e)?;
        let known = |o: usize| matches!(o, 0x00..=0x07 | 0x18..=0x79);
        let extra: Vec<String> = (0..d.len().min(0x100))
            .filter(|&o| !known(o) && d[o] != 0)
            .map(|o| format!("{o:02x}:{:02x}", d[o]))
            .collect();
        let prgr: Vec<String> = (0..16).map(|i| tag(&d, 0x18 + 4 * i)).collect();
        println!("{id:4} prgr[0..16] = {}", prgr.join(" "));
        println!(
            "{id:4} other = {}",
            if extra.is_empty() {
                "-".into()
            } else {
                extra.join(" ")
            }
        );
    }

    println!();
    println!("# meshes with media cells");
    println!(
        "{:4} | {:28} | {:4} | {:>8} | {:>6} | {:>6} | {:>6} | {:>6} | {:>7} | {:>6} | {:>6} | {:>6}",
        "mesh",
        "name (archive)",
        "medi",
        "+4c flg",
        "0x1800",
        "medVtx",
        "1800!4",
        "4!1800",
        "lodblk",
        "full",
        "fullCo",
        "lodBad"
    );
    for (id, src, name, data, mesh) in &mesh_rows {
        let flags = be32(data, MESH_HEADER_FLAGS);
        let cells = &mesh.cells;
        let media_cells = cells.iter().filter(|c| c.flags & 0x1800 != 0).count();
        if media_cells == 0 {
            continue;
        }
        let media_vtx = cells
            .iter()
            .filter(|c| c.flags & 0x0100 != 0 && c.flags & 0x0400 != 0)
            .count();
        let no_400 = cells
            .iter()
            .filter(|c| c.flags & 0x1800 != 0 && !(c.flags & 0x0100 != 0 && c.flags & 0x0400 != 0))
            .count();
        let only_400 = cells
            .iter()
            .filter(|c| c.flags & 0x1800 == 0 && c.flags & 0x0100 != 0 && c.flags & 0x0400 != 0)
            .count();
        let lod = lod_words(data);
        let (bw, bh) = (mesh.cell_width as usize / 2, mesh.cell_height as usize / 2);
        let mut full = 0;
        let mut full_coarse = 0;
        let mut mismatch = 0;
        for by in 0..bh {
            for bx in 0..bw {
                let Some(&w) = lod.get(by * bw + bx) else {
                    continue;
                };
                let f = |dx: usize, dy: usize| {
                    mesh.cell_at(2 * bx + dx, 2 * by + dy)
                        .map_or(0, |c| c.flags)
                };
                if media_bits_from_cells(f(0, 0), f(1, 0), f(1, 1), f(0, 1)) != w & 0xff00 {
                    mismatch += 1;
                }
                if w & 0xfff0 == 0xfff0 {
                    full += 1;
                    if w & 0xf == 0 {
                        full_coarse += 1;
                    }
                }
            }
        }
        println!(
            "{:4} | {:28} | {:4} | {:08x} | {:6} | {:6} | {:6} | {:6} | {:7} | {:6} | {:6} | {:6}",
            id,
            format!("{name} ({src})"),
            mesh.media_tag,
            flags,
            media_cells,
            media_vtx,
            no_400,
            only_400,
            lod.len(),
            full,
            full_coarse,
            mismatch
        );
    }
    println!();
    println!(
        "0x1800 = cells with a media triangle; medVtx = cells with 0x0100 and 0x0400 (vertex at the"
    );
    println!("media height, build_mesh_region); 1800!4 = media-triangle cells that are");
    println!(
        "not media vertices; 4!1800 = media vertices without a media triangle; lodblk = LOD words;"
    );
    println!(
        "full = blocks with all 4 cells drawn and all 8 fan triangles media (bits 4..15 set);"
    );
    println!(
        "fullCo = those with no edge subdivided (drawn as 2 triangles); lodBad = blocks whose LOD"
    );
    println!("media bits differ from the cell flags (media_bits_from_cells).");
    Ok(())
}

pub fn media_lod(
    archive: &TagArchive,
    entry: &TagEntry,
    region: Option<[usize; 4]>,
) -> anyhow::Result<()> {
    let data = archive.get_tag_data(entry)?;
    let mesh = MeshTag::from_bytes(&data)?;
    let lod = lod_words(&data);
    let (bw, bh) = (mesh.cell_width as usize / 2, mesh.cell_height as usize / 2);
    let [x0, y0, x1, y1] =
        region.unwrap_or([0, 0, mesh.cell_width as usize, mesh.cell_height as usize]);
    println!(
        "{} ({}): {}x{} cells, {} LOD words; region cells x {}..{}, y {}..{} (2 cells per char)",
        entry.tag_id,
        entry.name,
        mesh.cell_width,
        mesh.cell_height,
        lod.len(),
        x0,
        x1,
        y0,
        y1
    );
    println!(
        "legend: '#' all-media block, 2 triangles (no edge subdivided); '%' all-media, some edges"
    );
    println!(
        "        subdivided; 'w' part media; '.' dry, 2 triangles; ':' dry, subdivided; ' ' killed"
    );
    for by in (y0 / 2)..(y1 / 2).min(bh) {
        let mut line = format!("{:4} ", by * 2);
        for bx in (x0 / 2)..(x1 / 2).min(bw) {
            let w = lod.get(by * bw + bx).copied().unwrap_or(0);
            let ch = if w & 0x00f0 == 0 {
                ' '
            } else if w & 0xfff0 == 0xfff0 {
                if w & 0xf == 0 { '#' } else { '%' }
            } else if w & 0xff00 != 0 {
                'w'
            } else if w & 0xf == 0 {
                '.'
            } else {
                ':'
            };
            line.push(ch);
        }
        println!("{line}");
    }
    Ok(())
}

pub fn media_objects(archive: &TagArchive, entry: &TagEntry) -> anyhow::Result<()> {
    let data = archive.get_tag_data(entry)?;
    let mesh = MeshTag::from_bytes(&data)?;
    let coverage = mandrake_tag::mesh::MediaCoverage::from_mesh_bytes(
        &data,
        mesh.cell_width,
        mesh.cell_height,
    );
    let (w, h) = (i32::from(mesh.cell_width), i32::from(mesh.cell_height));
    let flags_at = |cx: i32, cy: i32| -> u16 {
        if (0..w).contains(&cx) && (0..h).contains(&cy) {
            mesh.cells[(cy * w + cx) as usize].flags
        } else {
            0
        }
    };
    println!(
        "{} ({}): markers on reflective (0x2000) or media (0x1800) cells, or within 2 cells of media",
        entry.tag_id, entry.name
    );
    println!("type tag  id     x      y      z | cell     flags   phys  media | refl sub cov dist");
    for m in &mesh.markers {
        if m.marker_type != 1 && m.marker_type != 3 {
            continue;
        }
        let tag = mesh
            .palettes
            .iter()
            .filter(|p| p.marker_type == m.marker_type)
            .nth(m.palette_index.max(0) as usize)
            .map_or("?".to_string(), |p| p.tag_id.to_string());
        let (cx, cy) = (m.x >> 9, m.y >> 9);
        let f = flags_at(cx, cy);
        let dist = (-2..=2)
            .flat_map(|dy| (-2..=2).map(move |dx| (dx, dy)))
            .filter(|&(dx, dy)| flags_at(cx + dx, cy + dy) & 0x1800 != 0)
            .map(|(dx, dy): (i32, i32)| dx.abs().max(dy.abs()))
            .min();
        if f & 0x3800 == 0 && dist.is_none() {
            continue;
        }
        let c = mesh.cells[(cy * w + cx) as usize];
        let cov = coverage.point_is_media(m.x, m.y);
        println!(
            "{:4} {:4} {:5} {:6} {:6} {:6} | {:3},{:3} {:#06x} {:6} {:6} | {:4} {:3} {:3} {:4}",
            if m.marker_type == 1 { "scen" } else { "unit" },
            tag,
            m.marker_id,
            m.x,
            m.y,
            m.z,
            cx,
            cy,
            f,
            c.physical_height,
            c.media_height,
            if f & 0x2000 != 0 { "yes" } else { "-" },
            if f & 0x1800 != 0 && cov { "yes" } else { "-" },
            if cov { "yes" } else { "-" },
            dist.map_or("-".to_string(), |d| d.to_string()),
        );
    }
    Ok(())
}
