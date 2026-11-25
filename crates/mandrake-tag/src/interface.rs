use crate::collection::CollectionTag;
use mandrake_common::{FourCC, M2Error};

pub const STANDARD_INTERFACE_COLLECTION: FourCC = FourCC(*b"stin");

pub const STANDARD_INTERFACE_DEFINITION: FourCC = FourCC(*b"stin");

pub const INTERFACE_FONT_COUNT: usize = 6;
pub const FONT_SYSTEM: usize = 0;
pub const FONT_SCREEN_HIGH_RES: usize = 1;
pub const FONT_SCREEN_LOW_RES: usize = 2;
pub const FONT_NARRATION_HEADER: usize = 3;
pub const FONT_NARRATION_BODY: usize = 4;
pub const FONT_NARRATION_CAPTION: usize = 5;

pub const COLOR_CONSOLE_TEXT: usize = 0x38;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceDefinition {
    pub collection: Option<FourCC>,
    pub fonts: [Option<FourCC>; INTERFACE_FONT_COUNT],
}

impl InterfaceDefinition {
    pub const SIZE: usize = 0x1c;

    pub fn from_bytes(d: &[u8]) -> Result<Self, M2Error> {
        if d.len() < Self::SIZE {
            return Err(M2Error::TagParse(format!(
                "inte: {} bytes, need {}",
                d.len(),
                Self::SIZE
            )));
        }
        Ok(Self {
            collection: crate::parse::optional_fourcc(d, 0),
            fonts: std::array::from_fn(|i| crate::parse::optional_fourcc(d, 4 + 4 * i)),
        })
    }

    pub fn font(&self, index: usize) -> Option<FourCC> {
        self.fonts.get(index).copied().flatten()
    }
}

pub const INTERFACE_COLOR_COUNT: usize = 0x39;

pub const COLOR_FACING_ARROW: usize = 1;
pub const COLOR_SELECTION_OWNED: usize = 2;
pub const COLOR_SELECTION_ENEMY: usize = 3;
pub const COLOR_SELECTION_ALLY: usize = 4;
pub const COLOR_SELECTION_OBJECT: usize = 5;
pub const COLOR_VITALITY_FILL_BASE: usize = 0x0c;
pub const COLOR_VITALITY_BACKGROUND_BASE: usize = 0x22;
pub const VITALITY_COLOR_STEPS: usize = 0x15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceColorTable {
    colors: Vec<[u8; 4]>,
    fallback: [u8; 4],
}

impl InterfaceColorTable {
    pub fn from_collection(coll: &CollectionTag) -> Option<Self> {
        let palette = coll.palette();
        if palette.is_empty() {
            return None;
        }
        let entry = |idx: usize| {
            let c = palette.get(idx).copied().unwrap_or([0, 0, 0, 255]);
            [c[0], c[1], c[2], 255]
        };
        let colors = coll.frame(0).map_or_else(Vec::new, |bitmap| {
            let count = (bitmap.width as usize * bitmap.height as usize)
                .min(INTERFACE_COLOR_COUNT)
                .min(bitmap.pixels.len());
            bitmap.pixels[..count]
                .iter()
                .map(|&idx| entry(idx as usize))
                .collect()
        });
        Some(Self {
            colors,
            fallback: entry(0),
        })
    }

    pub fn from_colors(colors: Vec<[u8; 4]>, palette_entry_0: [u8; 4]) -> Self {
        Self {
            colors,
            fallback: palette_entry_0,
        }
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    pub fn color(&self, index: usize) -> [u8; 4] {
        self.colors.get(index).copied().unwrap_or(self.fallback)
    }

    pub fn fallback(&self) -> [u8; 4] {
        self.fallback
    }

    pub fn color_f32(&self, index: usize) -> [f32; 4] {
        let c = self.color(index);
        [
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
            1.0,
        ]
    }

    pub fn vitality_step(fraction: f32) -> usize {
        let f = (fraction.clamp(0.0, 1.0) * 65536.0) as i32;
        ((f * VITALITY_COLOR_STEPS as i32) >> 16) as usize
    }

    pub fn vitality_colors(&self, fraction: f32) -> ([f32; 4], [f32; 4]) {
        let step = Self::vitality_step(fraction);
        (
            self.color_f32(COLOR_VITALITY_FILL_BASE + step),
            self.color_f32(COLOR_VITALITY_BACKGROUND_BASE + step),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> InterfaceColorTable {
        InterfaceColorTable {
            colors: (0..INTERFACE_COLOR_COUNT)
                .map(|i| [i as u8, 0, 0, 255])
                .collect(),
            fallback: [7, 8, 9, 255],
        }
    }

    #[test]
    fn test_vitality_step_quantization() {
        assert_eq!(InterfaceColorTable::vitality_step(0.0), 0);
        assert_eq!(InterfaceColorTable::vitality_step(0.5), 10);
        assert_eq!(InterfaceColorTable::vitality_step(1.0), 21);
        assert_eq!(InterfaceColorTable::vitality_step(2.0), 21);
    }

    #[test]
    fn test_vitality_color_indices() {
        let t = table();
        let (fill, bg) = t.vitality_colors(1.0);
        assert_eq!(fill[0], (COLOR_VITALITY_FILL_BASE + 21) as f32 / 255.0);
        assert_eq!(bg[0], (COLOR_VITALITY_BACKGROUND_BASE + 21) as f32 / 255.0);
        assert_eq!(t.color(1000), [7, 8, 9, 255]);
    }

    #[test]
    fn interface_definition_refs() {
        let mut d = b"stinti1pco1bco#1go2bgo1b".to_vec();
        d.extend_from_slice(&[0xff; 4]);
        let i = InterfaceDefinition::from_bytes(&d).unwrap();
        assert_eq!(i.collection, Some(STANDARD_INTERFACE_COLLECTION));
        assert_eq!(i.font(FONT_SCREEN_LOW_RES), Some(FourCC(*b"co#1")));
        assert_eq!(i.font(FONT_NARRATION_CAPTION), None);
        assert!(InterfaceDefinition::from_bytes(&d[..8]).is_err());
    }

    #[test]
    fn test_missing_index_falls_back_to_palette_entry_0() {
        let t = InterfaceColorTable::from_colors(vec![[1, 2, 3, 255]], [40, 50, 60, 255]);
        assert_eq!(t.color(0), [1, 2, 3, 255]);
        assert_eq!(t.color(1), [40, 50, 60, 255]);
        assert_eq!(t.color(INTERFACE_COLOR_COUNT), [40, 50, 60, 255]);
        assert_eq!(
            t.color_f32(5),
            [40.0 / 255.0, 50.0 / 255.0, 60.0 / 255.0, 1.0]
        );
    }
}
