use std::sync::Arc;

use mandrake_tag::Rect;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip};
use mandrake_view::interface_bitmap::{InterfaceArt, draw_bitmap_from_sequence, placement};

use super::list::fill_rect;
use super::{ControlProc, DrawContext, ItemState, item_flags};

pub const CROSSFADE: u32 = 0x80;
const CROSSFADE_TICKS: u64 = 0x1e;

#[derive(Clone)]
struct Caption {
    family: Arc<FontFamily>,
    color: RgbColor,
    text: Vec<u8>,
    flags: u32,
}

pub struct BitmapItem {
    art: Option<Arc<InterfaceArt>>,
    sequence: i32,
    frame: i32,
    transparency: u32,
    flags: u32,
    fill: Option<RgbColor>,
    crossfade_from: Option<u64>,
    caption: Option<Caption>,
    caption_below_art: bool,
}

impl BitmapItem {
    pub const fn new() -> Self {
        Self {
            art: None,
            sequence: -1,
            frame: 0,
            transparency: 0,
            flags: placement::REGISTRATION,
            fill: None,
            crossfade_from: None,
            caption: None,
            caption_below_art: false,
        }
    }

    pub fn set_art(
        &mut self,
        item: &mut ItemState,
        art: Option<Arc<InterfaceArt>>,
        sequence: i32,
        frame: i32,
    ) {
        self.fill = None;
        self.art = art;
        self.sequence = sequence;
        self.frame = frame;
        self.crossfade_from = None;
        item.flags &= !item_flags::WANTS_IDLE;
        item.dirty = true;
    }

    pub fn set_color(&mut self, item: &mut ItemState, color: Option<RgbColor>) {
        self.art = None;
        self.fill = color;
        self.crossfade_from = None;
        item.flags &= !item_flags::WANTS_IDLE;
        item.dirty = true;
    }

    pub fn set_transparency(&mut self, item: &mut ItemState, transparency: u32) {
        self.transparency = transparency;
        item.dirty = true;
    }

    pub fn set_drawing_flags(&mut self, item: &mut ItemState, flags: u32, now_tick: u64) {
        self.flags = flags;
        if flags & CROSSFADE != 0 && self.frame != 0 {
            item.flags |= item_flags::WANTS_IDLE;
            self.crossfade_from = Some(now_tick);
        }
        item.dirty = true;
    }

    pub fn set_caption(
        &mut self,
        family: Arc<FontFamily>,
        color: RgbColor,
        text: &[u8],
        flags: u32,
    ) {
        self.caption = Some(Caption {
            family,
            color,
            text: text.to_vec(),
            flags,
        });
    }

    pub fn set_caption_below_art(&mut self, below: bool) {
        self.caption_below_art = below;
    }

    const fn placement(&self) -> u32 {
        self.flags & !CROSSFADE
    }
}

impl Default for BitmapItem {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlProc for BitmapItem {
    fn idle(&mut self, item: &mut ItemState, now_tick: u64) -> bool {
        let Some(from) = self.crossfade_from else {
            item.flags &= !item_flags::WANTS_IDLE;
            return false;
        };
        if now_tick.saturating_sub(from) > CROSSFADE_TICKS {
            self.crossfade_from = None;
            item.flags &= !item_flags::WANTS_IDLE;
        }
        true
    }

    fn draw(&self, item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let rect = item.rect;
        match &self.art {
            None => {
                if let Some(color) = self.fill {
                    fill_rect(bitmap, color, rect, self.transparency);
                }
            }
            Some(art) => {
                let mut transparency = self.transparency;
                let mut skip = false;
                if let Some(from) = self.crossfade_from.filter(|_| self.frame != 0) {
                    let elapsed = ctx.now_tick.saturating_sub(from);
                    if elapsed <= CROSSFADE_TICKS {
                        draw_bitmap_from_sequence(
                            bitmap,
                            art,
                            self.sequence,
                            -1,
                            self.frame - 1,
                            rect,
                            self.placement(),
                            0,
                            0,
                        );
                        transparency =
                            (0x10000 - elapsed * 0x10000 / CROSSFADE_TICKS).min(0x10000) as u32;
                        skip = transparency == 0x10000;
                    }
                }
                if !skip {
                    draw_bitmap_from_sequence(
                        bitmap,
                        art,
                        self.sequence,
                        -1,
                        self.frame,
                        rect,
                        self.placement(),
                        0,
                        transparency,
                    );
                }
            }
        }
        if let Some(c) = &self.caption {
            let rect = match self.art.as_ref().filter(|_| self.caption_below_art) {
                Some(art) => {
                    let below = art.bitmap(self.sequence, 0, 0).map_or(0, |(b, _)| b.height);
                    Rect::new(
                        rect.top + below,
                        rect.left,
                        rect.top + below + 40,
                        rect.right,
                    )
                }
                None => Rect::new(rect.bottom, rect.left, rect.bottom, rect.right),
            };
            font::draw_string(
                bitmap,
                rect,
                TextClip::None,
                &c.family,
                font::STYLE_PLAIN,
                &c.text,
                c.color,
                c.flags,
                self.transparency,
                ctx.res.locale(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_common::FourCC;

    #[test]
    fn set_art_clears_wants_idle_after_crossfade() {
        let mut item = ItemState {
            id: 1,
            kind: FourCC(*b"bitm"),
            rect: Rect::new(0, 0, 10, 10),
            flags: 0,
            pressed: false,
            dirty: false,
        };
        let mut bmp = BitmapItem::new();
        bmp.set_art(&mut item, None, 0, 1);
        bmp.set_drawing_flags(&mut item, CROSSFADE, 10);
        assert_ne!(item.flags & item_flags::WANTS_IDLE, 0);
        bmp.set_art(&mut item, None, 0, 2);
        assert_eq!(item.flags & item_flags::WANTS_IDLE, 0);
        assert!(!bmp.idle(&mut item, 11));
    }
}
