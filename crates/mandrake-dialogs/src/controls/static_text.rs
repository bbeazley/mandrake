use std::sync::Arc;

use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, text_flags};

use super::{ControlProc, DrawContext, ItemState};
use crate::resources::Resources;

const DEFAULT_FLAGS: u32 = text_flags::WRAP | text_flags::SHADOW | text_flags::COLOR_CODES;

pub struct StaticText {
    text: Vec<u8>,
    flags: u32,
    color: RgbColor,
    family: Option<Arc<FontFamily>>,
    style: i16,
}

impl StaticText {
    pub fn new(name: &[u8], res: &mut Resources) -> Self {
        Self {
            text: name.to_vec(),
            flags: DEFAULT_FLAGS,
            color: res.dialog_color(0),
            family: res.font(0).cloned(),
            style: font::STYLE_PLAIN,
        }
    }

    pub fn text(&self) -> &[u8] {
        &self.text
    }

    pub fn set_text(&mut self, item: &mut ItemState, text: &[u8]) {
        if self.text != text {
            self.text = text.to_vec();
            item.dirty = true;
        }
    }

    pub fn set_appearance(
        &mut self,
        item: &mut ItemState,
        family: Option<Arc<FontFamily>>,
        style: i16,
        color: Option<RgbColor>,
        flags: Option<u32>,
    ) {
        if family.is_some() {
            self.family = family;
        }
        if let Some(color) = color {
            self.color = color;
        }
        if let Some(flags) = flags {
            self.flags = flags;
        }
        self.style = style;
        item.dirty = true;
    }
}

impl ControlProc for StaticText {
    fn draw(&self, item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let Some(family) = &self.family else { return };
        font::draw_string(
            bitmap,
            item.rect,
            TextClip::SameAsRect,
            family,
            self.style,
            &self.text,
            self.color,
            self.flags,
            if item.enabled() { 0 } else { 0x8000 },
            ctx.res.locale(),
        );
    }
}
