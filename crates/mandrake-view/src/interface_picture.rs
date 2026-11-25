use mandrake_tag::Rect;
use mandrake_tag::font::{Bitmap32, TextMask};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InterfaceFilter {
    #[default]
    Sharp,
    Nearest,
    Linear,
    Lanczos2,
    Fsr1,
}

pub const EDGE_ROWS: i32 = 20;
pub const TEXTURE_PAD_ROWS: i32 = 3;

#[derive(Debug, Clone, Copy)]
pub struct PrologueGpuLayer<'a> {
    pub generation: u64,
    pub mask: &'a TextMask,
    pub color: [u8; 3],
    pub scroll: f32,
    pub strength: i32,
}

impl PrologueGpuLayer<'_> {
    pub fn is_in_view(&self, view_height: i32) -> bool {
        self.strength > 0
            && self.scroll + (view_height.max(0) as f32) > 0.0
            && self.scroll < self.mask.height as f32
    }

    pub fn to_padded_premultiplied_rgba(&self, pad_rows: u32) -> (u32, u32, Vec<u8>) {
        let w = self.mask.width.max(0) as u32;
        let h = self.mask.height.max(0) as u32;
        let total_h = h + pad_rows * 2;
        let [r, g, b] = self.color;
        let mut rgba = vec![0u8; (w * total_h * 4) as usize];
        let pad_bytes = (w * pad_rows * 4) as usize;
        for (dst, &level) in rgba[pad_bytes..pad_bytes + (w * h * 4) as usize]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&self.mask.levels)
        {
            let alpha = match level {
                0xff => 0,
                0 | 1 => 0xff,
                l => 0xff - l,
            };
            if alpha != 0 {
                *dst = [
                    ((r as u16 * alpha as u16 + 127) / 255) as u8,
                    ((g as u16 * alpha as u16 + 127) / 255) as u8,
                    ((b as u16 * alpha as u16 + 127) / 255) as u8,
                    alpha,
                ];
            }
        }
        (w, total_h, rgba)
    }
}

pub struct MenuCanvas<'a> {
    pub canvas: &'a Bitmap32,
    pub revision: u64,
    pub size: (u32, u32),
    pub prologue: Option<PrologueGpuLayer<'a>>,
    pub scroll: Option<(Rect, f32, i32, bool)>,
}

#[derive(Debug, Clone, Copy)]
pub struct InterfaceLayer<'a> {
    pub bitmap: &'a Bitmap32,
    pub dst: [i32; 4],
}

pub enum InterfacePicture<'a> {
    Menu(MenuCanvas<'a>),
    Layers {
        layers: Vec<InterfaceLayer<'a>>,
        revision: u64,
    },
}
