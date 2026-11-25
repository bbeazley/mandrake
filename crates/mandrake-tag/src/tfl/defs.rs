use mandrake_common::{FourCC, M2Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TflTagDefinition {
    pub group: FourCC,
    pub version: u16,
    pub max_count: u16,
    pub size: u16,
    pub fixed: bool,
}

const fn def(
    group: &[u8; 4],
    version: u16,
    max_count: u16,
    size: u16,
    fixed: bool,
) -> TflTagDefinition {
    TflTagDefinition {
        group: FourCC(*group),
        version,
        max_count,
        size,
        fixed,
    }
}

pub const TFL_TAG_DEFINITIONS: &[TflTagDefinition] = &[
    def(b".256", 4, 40, 320, false),
    def(b"amso", 1, 32, 64, true),
    def(b"arti", 1, 16, 128, true),
    def(b"core", 1, 128, 384, true),
    def(b"font", 1, 16, 64, false),
    def(b"geom", 1, 128, 96, false),
    def(b"inte", 1, 1, 256, true),
    def(b"ligh", 1, 4, 64, true),
    def(b"lpgr", 1, 64, 128, true),
    def(b"medi", 1, 2, 256, true),
    def(b"meef", 1, 32, 32, true),
    def(b"mesh", 7, 1, 1024, false),
    def(b"mode", 4, 128, 64, false),
    def(b"mons", 1, 32, 512, true),
    def(b"obje", 1, 32, 16, true),
    def(b"obpc", 1, 1, 256, true),
    def(b"part", 1, 1, 128, true),
    def(b"phys", 1, 128, 64, true),
    def(b"prgr", 1, 80, 512, true),
    def(b"proj", 1, 128, 256, true),
    def(b"reva", 1, 1, 128, true),
    def(b"scen", 2, 32, 128, true),
    def(b"soli", 1, 16, 256, true),
    def(b"soun", 1, 160, 64, false),
    def(b"stli", 1, 128, 16, false),
    def(b"text", 1, 32, 8, false),
    def(b"unit", 1, 128, 128, true),
    def(b"wind", 1, 1, 128, true),
];

pub fn tfl_tag_definition(group: FourCC) -> Option<&'static TflTagDefinition> {
    TFL_TAG_DEFINITIONS.iter().find(|d| d.group == group)
}

pub const TFL_REFERENCE_FIELDS: &[(FourCC, usize, FourCC)] = &[
    (FourCC(*b"arti"), 0x04, FourCC(*b"mons")),
    (FourCC(*b"arti"), 0x10, FourCC(*b".256")),
    (FourCC(*b"part"), 0x04, FourCC(*b"core")),
    (FourCC(*b"mons"), 0x04, FourCC(*b".256")),
    (FourCC(*b"mons"), 0x90, FourCC(*b"obje")),
    (FourCC(*b"unit"), 0x00, FourCC(*b"mons")),
    (FourCC(*b"unit"), 0x04, FourCC(*b"core")),
    (FourCC(*b"scen"), 0x08, FourCC(*b"core")),
    (FourCC(*b"scen"), 0x18, FourCC(*b"obje")),
    (FourCC(*b"proj"), 0x1c, FourCC(*b"obje")),
    (FourCC(*b"lpgr"), 0x04, FourCC(*b"core")),
    (FourCC(*b"medi"), 0x04, FourCC(*b"core")),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflDefinition {
    pub group: FourCC,
    pub id: FourCC,
    pub data: Vec<u8>,
}

impl TflDefinition {
    pub fn from_bytes(group: FourCC, id: FourCC, data: &[u8]) -> Result<Self, M2Error> {
        let (size, exact) = match tfl_tag_definition(group) {
            Some(d) if d.fixed => (
                d.size as usize,
                !matches!(&group.0, b"arti" | b"core" | b"medi"),
            ),
            None if group.0 == *b"meli" => (128, true),
            _ => {
                return Err(M2Error::TagParse(format!(
                    "TFL '{group}' is not a fixed-size definition"
                )));
            }
        };
        if data.len() < size || (exact && data.len() != size) {
            return Err(M2Error::TagParse(format!(
                "TFL {group} '{id}': {} bytes, expected {}{size}",
                data.len(),
                if exact { "" } else { ">= " }
            )));
        }
        Ok(Self {
            group,
            id,
            data: data.to_vec(),
        })
    }

    pub fn u16_at(&self, o: usize) -> u16 {
        u16::from_be_bytes([self.data[o], self.data[o + 1]])
    }

    pub fn i16_at(&self, o: usize) -> i16 {
        self.u16_at(o) as i16
    }

    pub fn u32_at(&self, o: usize) -> u32 {
        u32::from_be_bytes(self.data[o..o + 4].try_into().expect("in the tag"))
    }

    pub fn tag_at(&self, o: usize) -> Option<FourCC> {
        let b: [u8; 4] = self.data[o..o + 4].try_into().expect("in the tag");
        (b != [0xff; 4] && b != [0; 4]).then_some(FourCC(b))
    }

    pub fn references(&self) -> Vec<(usize, FourCC, Option<FourCC>)> {
        TFL_REFERENCE_FIELDS
            .iter()
            .filter(|(g, _, _)| *g == self.group)
            .map(|&(_, o, target)| (o, target, self.tag_at(o)))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TflObjectDefinition {
    pub tag_id: FourCC,
    pub words: [i16; 8],
}

impl TflObjectDefinition {
    pub fn from_bytes(tag_id: FourCC, data: &[u8]) -> Result<Self, M2Error> {
        let d = TflDefinition::from_bytes(FourCC(*b"obje"), tag_id, data)?;
        Ok(Self {
            tag_id,
            words: std::array::from_fn(|i| d.i16_at(2 * i)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_the_exe_table() {
        let obje = FourCC(*b"obje");
        let o = TflObjectDefinition::from_bytes(
            FourCC(*b"baob"),
            &[0xff, 0xfe, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .unwrap();
        assert_eq!(&o.words[..2], &[-2, 0x80]);
        assert!(TflDefinition::from_bytes(obje, obje, &[0; 64]).is_err());
        let arti = FourCC(*b"arti");
        assert!(TflDefinition::from_bytes(arti, arti, &[0; 256]).is_ok());
        assert!(TflDefinition::from_bytes(arti, arti, &[0; 100]).is_err());
        assert!(TflDefinition::from_bytes(FourCC(*b"mesh"), arti, &[0; 1024]).is_err());
        assert_eq!(tfl_tag_definition(FourCC(*b"mesh")).unwrap().version, 7);
    }
}
