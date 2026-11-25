use mandrake_common::FourCC;

pub const PARAMETER_TYPE_WIDTHS: [usize; 21] = [
    1, 1, 2, 2, 2, 4, 4, 4, 4, 4, 4, 4, 4, 8, 16, 2, 2, 2, 12, 2, 2,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapActionParameter {
    pub kind: i16,
    pub count: i16,
    pub name: FourCC,
    pub values: Vec<i32>,
}

impl MapActionParameter {
    pub fn scalars_per_element(kind: i16) -> usize {
        match PARAMETER_TYPE_WIDTHS.get(kind as usize) {
            Some(&w) if w > 4 => w / 4,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapActionRecord {
    pub identifier: u16,
    pub kind: i16,
    pub tag: u32,
    pub flags: u32,
    pub delay: i32,
    pub delay_range: i32,
    pub field_1c: u16,
    pub time: i32,
    pub parameters: Vec<MapActionParameter>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeshMapActions {
    pub actions: Vec<MapActionRecord>,
}

fn be_i16(d: &[u8], o: usize) -> Option<i16> {
    Some(i16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn be_u32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

impl MeshMapActions {
    pub const HEADER_OFFSET: usize = 0x80;

    pub fn from_mesh_bytes(data: &[u8]) -> Result<Self, String> {
        let short = || "mesh map actions truncated".to_string();
        let count = be_u32(data, Self::HEADER_OFFSET).ok_or_else(short)? as i32;
        let offset = be_u32(data, Self::HEADER_OFFSET + 4).ok_or_else(short)? as usize;
        let size = be_u32(data, Self::HEADER_OFFSET + 8).ok_or_else(short)? as usize;
        if count <= 0 {
            return Ok(Self::default());
        }
        let base = 1024usize.checked_add(offset).ok_or_else(short)?;
        let records_end = base.checked_add(size).ok_or_else(short)?;
        let records = data.get(base..records_end).ok_or_else(short)?;
        let n = count as usize;
        let records_bytes = n.checked_mul(0x40).ok_or_else(short)?;
        let buffer = records.get(records_bytes..).ok_or_else(short)?;
        let mut actions = Vec::with_capacity(n);
        for i in 0..n {
            let r = &records[i * 0x40..(i + 1) * 0x40];
            let pcount = be_i16(r, 0x14).ok_or_else(short)?;
            let psize = be_i16(r, 0x16).ok_or_else(short)? as u16 as usize;
            let poff = be_u32(r, 0x18).ok_or_else(short)? as usize;
            let p_limit = poff.checked_add(psize).ok_or_else(short)?;
            let mut parameters = Vec::new();
            let mut o = poff;
            for _ in 0..pcount.max(0) {
                let kind = be_i16(buffer, o).ok_or_else(short)?;
                let pc = be_i16(buffer, o.checked_add(2).ok_or_else(short)?).ok_or_else(short)?;
                let name_start = o.checked_add(4).ok_or_else(short)?;
                let name_end = o.checked_add(8).ok_or_else(short)?;
                let name = FourCC::new(
                    buffer
                        .get(name_start..name_end)
                        .ok_or_else(short)?
                        .try_into()
                        .unwrap(),
                );
                let width = usize::try_from(kind)
                    .ok()
                    .and_then(|k| PARAMETER_TYPE_WIDTHS.get(k).copied())
                    .ok_or_else(|| format!("map action {i}: parameter type {kind}"))?;
                let bytes = width.checked_mul(pc.max(0) as usize).ok_or_else(short)?;
                let raw_end = name_end.checked_add(bytes).ok_or_else(short)?;
                let raw = buffer.get(name_end..raw_end).ok_or_else(short)?;
                let values = match width {
                    1 => raw.iter().map(|&b| i32::from(b)).collect(),
                    2 => raw
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| i32::from(i16::from_be_bytes(*c)))
                        .collect(),
                    _ => raw
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| i32::from_be_bytes(*c))
                        .collect(),
                };
                parameters.push(MapActionParameter {
                    kind,
                    count: pc,
                    name,
                    values,
                });
                let padded = bytes.div_ceil(4).checked_mul(4).ok_or_else(short)?;
                o = name_end.checked_add(padded).ok_or_else(short)?;
            }
            if o > p_limit {
                return Err(format!("map action {i}: parameters overrun their size"));
            }
            actions.push(MapActionRecord {
                identifier: be_i16(r, 0).ok_or_else(short)? as u16,
                kind: be_i16(r, 2).ok_or_else(short)?,
                tag: be_u32(r, 4).ok_or_else(short)?,
                flags: be_u32(r, 8).ok_or_else(short)?,
                delay: be_u32(r, 0x0c).ok_or_else(short)? as i32,
                delay_range: be_u32(r, 0x10).ok_or_else(short)? as i32,
                field_1c: be_i16(r, 0x1c).ok_or_else(short)? as u16,
                time: be_u32(r, 0x3c).ok_or_else(short)? as i32,
                parameters,
            });
        }
        Ok(Self { actions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records_and_padded_parameters() {
        let mut d = vec![0u8; 1024];
        d[0x80..0x84].copy_from_slice(&1u32.to_be_bytes());
        d[0x84..0x88].copy_from_slice(&0u32.to_be_bytes());
        let mut rec = vec![0u8; 0x40];
        rec[0..2].copy_from_slice(&35000u16.to_be_bytes());
        rec[2..4].copy_from_slice(&3i16.to_be_bytes());
        rec[4..8].copy_from_slice(b"wand");
        rec[8..12].copy_from_slice(&5u32.to_be_bytes());
        rec[0x0c..0x10].copy_from_slice(&150i32.to_be_bytes());
        rec[0x14..0x16].copy_from_slice(&2i16.to_be_bytes());
        rec[0x16..0x18].copy_from_slice(&32i16.to_be_bytes());
        let mut params = Vec::new();
        params.extend_from_slice(&2i16.to_be_bytes());
        params.extend_from_slice(&3i16.to_be_bytes());
        params.extend_from_slice(b"subj");
        for v in [10001i16, 10002, -2] {
            params.extend_from_slice(&v.to_be_bytes());
        }
        params.extend_from_slice(&[0, 0]);
        params.extend_from_slice(&13i16.to_be_bytes());
        params.extend_from_slice(&1i16.to_be_bytes());
        params.extend_from_slice(b"cent");
        params.extend_from_slice(&81969i32.to_be_bytes());
        params.extend_from_slice(&84117i32.to_be_bytes());
        let size = (rec.len() + params.len()) as u32;
        d[0x88..0x8c].copy_from_slice(&size.to_be_bytes());
        d.extend_from_slice(&rec);
        d.extend_from_slice(&params);
        let a = MeshMapActions::from_mesh_bytes(&d).unwrap();
        assert_eq!(a.actions.len(), 1);
        let r = &a.actions[0];
        assert_eq!((r.identifier, r.kind, r.flags, r.delay), (35000, 3, 5, 150));
        assert_eq!(&r.tag.to_be_bytes(), b"wand");
        assert_eq!(r.parameters[0].values, vec![10001, 10002, -2]);
        assert_eq!(r.parameters[1].name, FourCC::new(*b"cent"));
        assert_eq!(r.parameters[1].values, vec![81969, 84117]);
    }
}
