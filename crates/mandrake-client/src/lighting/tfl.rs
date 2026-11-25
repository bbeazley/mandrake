use mandrake_common::FourCC;
use mandrake_tag::TagArchive;
use std::collections::BTreeSet;

const FLAMING: u32 = 0x1_0000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TflFires {
    lit: BTreeSet<FourCC>,
}

impl TflFires {
    pub(crate) fn of(archive: &TagArchive) -> Self {
        let data = |group: &[u8; 4], id: FourCC| crate::tfl_map::tag_data(archive, group, id);
        let mut lit = BTreeSet::new();
        for entry in archive.find_tags_by_type(FourCC(*b"proj")) {
            let Ok(proj) = archive.get_tag_data(entry) else {
                continue;
            };
            if be_u32(&proj, 0) & FLAMING == 0 {
                continue;
            }
            lit.insert(entry.tag_id);
            let Some(prgr) = tag_at(&proj, 0x10).and_then(|g| data(b"prgr", g)) else {
                continue;
            };
            let parts = be_i16(&prgr, 4).unwrap_or(0).max(0) as usize;
            lit.extend((0..parts).filter_map(|k| tag_at(&prgr, 0x20 + 0x20 * k)));
        }
        Self { lit }
    }

    pub(crate) fn lights(&self, proj: FourCC) -> bool {
        self.lit.contains(&proj)
    }
}

fn be_i16(d: &[u8], o: usize) -> Option<i16> {
    d.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]))
}

fn be_u32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn tag_at(d: &[u8], o: usize) -> Option<FourCC> {
    let b: [u8; 4] = d.get(o..o + 4)?.try_into().ok()?;
    let v = u32::from_be_bytes(b);
    (v != 0xffff_ffff && v != 0).then_some(FourCC(b))
}
