use mandrake_tag::Rect;
use mandrake_tag::font::{Bitmap32, RgbColor};

use super::list::fill_rect;
use super::{ControlProc, DrawContext, ItemState};
use crate::resources::Resources;

const TRANSPARENCY: u32 = 0x8000;

pub const TFL_COLOR: RgbColor = [0xffff, 0, 0];

pub struct ProgressBar {
    fraction: i16,
    color: RgbColor,
}

impl ProgressBar {
    pub fn new(res: &mut Resources) -> Self {
        Self {
            fraction: 0,
            color: res.dialog_color(8),
        }
    }

    pub fn set_fraction(&mut self, item: &mut ItemState, fraction: i16) {
        let f = fraction.clamp(0, 0x100);
        if self.fraction != f {
            self.fraction = f;
            item.dirty = true;
        }
    }

    pub fn set_color(&mut self, item: &mut ItemState, color: RgbColor) {
        if self.color != color {
            self.color = color;
            item.dirty = true;
        }
    }

    pub const fn fraction(&self) -> i16 {
        self.fraction
    }
}

impl ControlProc for ProgressBar {
    fn draw(&self, item: &ItemState, _ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let r = item.rect;
        let width = (r.right - r.left) * i32::from(self.fraction);
        let right = r.left + (width / 256);
        fill_rect(
            bitmap,
            self.color,
            Rect::new(r.top, r.left, r.bottom, right),
            TRANSPARENCY,
        );
    }

    fn click(&mut self, _item: &mut ItemState, _click: &super::Click) -> bool {
        false
    }
}
