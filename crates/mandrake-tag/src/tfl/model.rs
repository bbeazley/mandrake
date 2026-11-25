use mandrake_common::M2Error;

use crate::model::{GeometryTag, ModelTag};

pub const TFL_GEOMETRY_VERSION: u16 = 1;
pub const TFL_MODEL_VERSION: u16 = 4;
pub const TFL_GEOMETRY_HEADER_SIZE: usize = 0x60;
pub const TFL_MODEL_HEADER_SIZE: usize = 0x40;

fn be_u16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}

fn be_u32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn bad(what: &str, detail: String) -> M2Error {
    M2Error::TagParse(format!("TFL {what}: {detail}"))
}

fn check_table(
    what: &str,
    d: &[u8],
    at: usize,
    count: usize,
    record: usize,
    data_len: usize,
) -> Result<(), M2Error> {
    let (offset, size) = (be_u32(d, at) as usize, be_u32(d, at + 4) as usize);
    if count.checked_mul(record) != Some(size) {
        return Err(bad(
            what,
            format!("table at +{at:#x}: {size} bytes for {count} records of {record}"),
        ));
    }
    if offset.checked_add(size).is_none_or(|end| end > data_len) {
        return Err(bad(
            what,
            format!("table at +{at:#x} ({offset:#x}+{size:#x}) past the data ({data_len:#x})"),
        ));
    }
    Ok(())
}

pub fn tfl_geometry(data: &[u8], version: u16) -> Result<GeometryTag, M2Error> {
    if version != TFL_GEOMETRY_VERSION {
        return Err(bad(
            "geom",
            format!("version {version}, this reader knows {TFL_GEOMETRY_VERSION}"),
        ));
    }
    let h = TFL_GEOMETRY_HEADER_SIZE;
    if data.len() < h {
        return Err(bad(
            "geom",
            format!("{} bytes, the header is {h}", data.len()),
        ));
    }
    let (data_at, data_size) = (be_u32(data, 0x4c) as usize, be_u32(data, 0x50) as usize);
    if data_at != h || data_at.checked_add(data_size) != Some(data.len()) {
        return Err(bad(
            "geom",
            format!(
                "data at {data_at:#x}, {data_size:#x} bytes, in a {:#x}-byte tag",
                data.len()
            ),
        ));
    }
    let count = |o| be_u16(data, o) as usize;
    check_table(
        "geom materials",
        data,
        0x1c,
        count(0x08),
        GeometryTag::MATERIAL_SIZE,
        data_size,
    )?;
    check_table(
        "geom vertices",
        data,
        0x28,
        count(0x0a),
        GeometryTag::VERTEX_SIZE,
        data_size,
    )?;
    check_table(
        "geom surfaces",
        data,
        0x34,
        count(0x0c),
        GeometryTag::SURFACE_SIZE,
        data_size,
    )?;
    check_table(
        "geom draw-after words",
        data,
        0x40,
        count(0x0e),
        2,
        data_size,
    )?;
    let mut myth2 = Vec::with_capacity(GeometryTag::HEADER_SIZE + data_size);
    myth2.extend_from_slice(&data[..0x58]);
    myth2.resize(GeometryTag::HEADER_SIZE, 0);
    myth2.extend_from_slice(&data[h..]);
    let geom = GeometryTag::from_bytes(&myth2)?;
    if geom.vertices.len() != count(0x0a) || geom.surfaces.len() != count(0x0c) {
        return Err(bad("geom", "records not read".to_string()));
    }
    for (i, s) in geom.surfaces.iter().enumerate() {
        if let Some(v) = s
            .vertices
            .iter()
            .find(|v| usize::from(v.vertex_index) >= geom.vertices.len())
        {
            return Err(bad(
                "geom",
                format!("surface {i} names vertex {}", v.vertex_index),
            ));
        }
        if s.material_index >= 0 && s.material_index as usize >= geom.materials.len() {
            return Err(bad(
                "geom",
                format!("surface {i} names material {}", s.material_index),
            ));
        }
    }
    Ok(geom)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TflModelCell {
    pub x: i16,
    pub y: i16,
    pub flags: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflModel {
    pub model: ModelTag,
    pub cells: Vec<TflModelCell>,
}

pub fn tfl_model(data: &[u8], version: u16) -> Result<TflModel, M2Error> {
    if version != TFL_MODEL_VERSION {
        return Err(bad(
            "mode",
            format!("version {version}, this reader knows {TFL_MODEL_VERSION}"),
        ));
    }
    let h = TFL_MODEL_HEADER_SIZE;
    if data.len() < h {
        return Err(bad(
            "mode",
            format!("{} bytes, the header is {h}", data.len()),
        ));
    }
    let (data_at, data_size) = (be_u32(data, 0x34) as usize, be_u32(data, 0x38) as usize);
    if data_at != h || data_at.checked_add(data_size) != Some(data.len()) {
        return Err(bad(
            "mode",
            format!(
                "data at {data_at:#x}, {data_size:#x} bytes, in a {:#x}-byte tag",
                data.len()
            ),
        ));
    }
    let count = |o| be_u16(data, o) as usize;
    check_table("mode vertex flags", data, 0x10, count(0x0a), 4, data_size)?;
    check_table("mode permutations", data, 0x1c, count(0x0c), 64, data_size)?;
    check_table("mode cells", data, 0x28, count(0x0e), 8, data_size)?;
    let model = ModelTag::from_bytes(data)?;
    if model.permutations.len() != count(0x0c) {
        return Err(bad("mode", "permutations not read".to_string()));
    }
    let at = h + be_u32(data, 0x28) as usize;
    let cells = (0..count(0x0e))
        .map(|k| {
            let e = at + 8 * k;
            TflModelCell {
                x: be_u16(data, e) as i16,
                y: be_u16(data, e + 2) as i16,
                flags: be_u16(data, e + 4) as i16,
            }
        })
        .collect();
    Ok(TflModel { model, cells })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Vec<u8> {
        let mut d = vec![0u8; TFL_GEOMETRY_HEADER_SIZE];
        d[0x04..0x08].copy_from_slice(b"crbt");
        d[0x08..0x0a].copy_from_slice(&1u16.to_be_bytes());
        d[0x0a..0x0c].copy_from_slice(&3u16.to_be_bytes());
        d[0x0c..0x0e].copy_from_slice(&1u16.to_be_bytes());
        d[0x10..0x16].copy_from_slice(&[0x08, 0, 0x08, 0, 0x02, 0]);
        let tables = [
            (0x1c, 0, 64),
            (0x28, 64, 18),
            (0x34, 82, 64),
            (0x40, 146, 0),
        ];
        for (at, off, size) in tables {
            d[at..at + 4].copy_from_slice(&(off as u32).to_be_bytes());
            d[at + 4..at + 8].copy_from_slice(&(size as u32).to_be_bytes());
        }
        let mut body = vec![0u8; 146];
        body[..4].copy_from_slice(b"top\0");
        body[0x20..0x22].copy_from_slice(&2i16.to_be_bytes());
        for (k, v) in [[0i16, 0, 0], [1024, 0, 0], [0, 1024, 512]]
            .iter()
            .enumerate()
        {
            for (j, c) in v.iter().enumerate() {
                let o = 64 + 6 * k + 2 * j;
                body[o..o + 2].copy_from_slice(&c.to_be_bytes());
            }
        }
        let s = 82;
        body[s + 0x1a..s + 0x1c].copy_from_slice(&0i16.to_be_bytes());
        for k in 0..3u16 {
            let o = s + 0x1c + 8 * k as usize + 4;
            body[o..o + 2].copy_from_slice(&k.to_be_bytes());
        }
        body[s + 0x3c..s + 0x3e].copy_from_slice(&(-1i16).to_be_bytes());
        d[0x4c..0x50].copy_from_slice(&(TFL_GEOMETRY_HEADER_SIZE as u32).to_be_bytes());
        d[0x50..0x54].copy_from_slice(&(body.len() as u32).to_be_bytes());
        d.extend_from_slice(&body);
        d
    }

    #[test]
    fn a_tfl_geometry_reads_from_its_96_byte_header() {
        let g = tfl_geometry(&triangle(), 1).unwrap();
        assert_eq!(g.texture_tag, "crbt");
        assert_eq!(g.origin, (0x800, 0x800, 0x200));
        assert_eq!(g.materials[0].name, "top");
        assert_eq!(g.materials[0].texture_index, 2);
        assert_eq!(g.vertices[2].z, 512);
        assert_eq!(g.surfaces[0].vertices[2].vertex_index, 2);
        assert_eq!(g.bounds, [[0; 3]; 2]);
    }

    #[test]
    fn an_unknown_version_or_layout_fails() {
        assert!(tfl_geometry(&triangle(), 2).is_err());
        let mut d = triangle();
        d[0x0a..0x0c].copy_from_slice(&4u16.to_be_bytes());
        assert!(
            tfl_geometry(&d, 1).is_err(),
            "vertex count against its table"
        );
        let mut d = triangle();
        d.push(0);
        assert!(tfl_geometry(&d, 1).is_err(), "data size against the tag");
        assert!(tfl_model(&[0; 0x40], 3).is_err());
    }
}
