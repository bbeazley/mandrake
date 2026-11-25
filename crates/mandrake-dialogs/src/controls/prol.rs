use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mandrake_tag::DialogItem;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextMask};
use mandrake_view::interface_bitmap::{blend_level, palette_pixel};

use super::{ControlProc, DrawContext, ItemState};
use crate::resources::Resources;

pub use mandrake_view::interface_picture::{EDGE_ROWS, PrologueGpuLayer, TEXTURE_PAD_ROWS};

const TOP_EDGE_LAST: i32 = EDGE_ROWS - 2;
const OPAQUE: i32 = 0xff;
const RUN_ON: i32 = 0x3c;
const END_MARGIN: i32 = 0x14;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

const fn edge_strength(row: i32, height: i32) -> i32 {
    if row > height - EDGE_ROWS {
        ((height - row) << 8) / EDGE_ROWS
    } else if row <= TOP_EDGE_LAST {
        ((row + 1) << 8) / EDGE_ROWS
    } else {
        0x100
    }
}

pub struct Prologue {
    mask: Option<TextMask>,
    color: RgbColor,
    scroll: i32,
    scroll_f32: f32,
    end: i32,
    strength: i32,
    done: bool,
    generation: u64,
}

impl Prologue {
    pub fn new(item: &DialogItem, _res: &mut Resources) -> Self {
        let scroll = item.rect.top - item.rect.bottom;
        Self {
            mask: None,
            color: [0; 3],
            scroll,
            scroll_f32: scroll as f32,
            end: 0,
            strength: OPAQUE,
            done: false,
            generation: 0,
        }
    }

    pub fn set_data(
        &mut self,
        item: &mut ItemState,
        color: RgbColor,
        family: &Arc<FontFamily>,
        text: &[u8],
        res: &Resources,
    ) {
        let width = item.rect.width();
        let style = font::STYLE_PLAIN;
        let height = font::string_height(family, style, text, width, res.locale());
        let mut mask = TextMask::new(width, height);
        let (_, last_baseline) =
            font::draw_string_to_mask(&mut mask, family, style, text, res.locale());
        self.mask = Some(mask);
        self.color = color;
        self.end = item.rect.top + last_baseline + END_MARGIN - item.rect.bottom;
        self.scroll_f32 = self.scroll as f32;
        self.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        item.dirty = true;
    }

    pub fn update(&mut self, item: &mut ItemState) {
        if self.end + RUN_ON < self.scroll {
            self.done = true;
        }
        self.scroll += 1;
        self.scroll_f32 = self.scroll as f32;
        item.dirty = true;
    }

    pub const fn done(&self) -> bool {
        self.done
    }

    pub fn set_transparency(&mut self, item: &mut ItemState, transparency: u32) {
        self.strength = OPAQUE - ((transparency >> 8) & 0xff) as i32;
        item.dirty = true;
    }

    pub const fn scroll(&self) -> i32 {
        self.scroll
    }

    pub const fn scroll_f32(&self) -> f32 {
        self.scroll_f32
    }

    pub fn set_scroll_f32(&mut self, scroll: f32) {
        self.scroll_f32 = scroll;
    }

    pub fn gpu_layer(&self) -> Option<PrologueGpuLayer<'_>> {
        let mask = self.mask.as_ref()?;
        if mask.width <= 0 || mask.height <= 0 {
            return None;
        }
        Some(PrologueGpuLayer {
            generation: self.generation,
            mask,
            color: self.color.map(|v| (v >> 8) as u8),
            scroll: self.scroll_f32,
            strength: self.strength,
        })
    }
}

impl ControlProc for Prologue {
    fn can_focus(&self, _item: &ItemState) -> bool {
        true
    }

    fn focus(&mut self, _item: &mut ItemState, _focused: bool) -> bool {
        true
    }

    fn draw(&self, item: &ItemState, _ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let Some(mask) = &self.mask else { return };
        let rect = item.rect;
        let height = rect.height();
        let [r, g, b] = self.color.map(|v| (v >> 8) as u8);
        let color = palette_pixel([r, g, b, 0xff]);
        for row in 0..height {
            let source = self.scroll + row;
            let y = rect.top + row;
            if source < 0 || source >= mask.height || y < 0 || y >= bitmap.height {
                continue;
            }
            let strength = (edge_strength(row, height) * self.strength) >> 8;
            for (x, &level) in (rect.left..).zip(mask.row(source)) {
                if x < 0 || x >= bitmap.width || level == 0xff {
                    continue;
                }
                let pixel = if level == 0 {
                    color
                } else {
                    let blend = 0xf - (((0xff - level as i32) * strength) >> 12);
                    blend_level(bitmap.get(x, y), color, blend as u32)
                };
                bitmap.set(x, y, pixel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_fades_over_twenty_rows_at_each_edge() {
        let height = 200;
        assert_eq!(edge_strength(0, height), 12);
        assert_eq!(edge_strength(18, height), 243);
        assert_eq!(edge_strength(19, height), 0x100);
        assert_eq!(edge_strength(180, height), 0x100);
        assert_eq!(edge_strength(181, height), 243);
        assert_eq!(edge_strength(199, height), 12);
    }

    #[test]
    fn padded_premultiplied_rgba_tints_and_pads_the_mask() {
        let mut mask = TextMask::new(2, 1);
        mask.levels = vec![1, 0xff];
        let layer = PrologueGpuLayer {
            generation: 1,
            mask: &mask,
            color: [200, 100, 50],
            scroll: -10.0,
            strength: OPAQUE,
        };
        let (w, h, rgba) = layer.to_padded_premultiplied_rgba(2);
        assert_eq!((w, h), (2, 5));
        let pixels = rgba.as_chunks::<4>().0;
        assert!(pixels[..4].iter().all(|&p| p == [0, 0, 0, 0]));
        assert_eq!(pixels[4], [200, 100, 50, 255]);
        assert_eq!(pixels[5], [0, 0, 0, 0]);
        assert!(pixels[6..].iter().all(|&p| p == [0, 0, 0, 0]));
        assert!(!layer.is_in_view(10));
        assert!(layer.is_in_view(11));
    }
}
