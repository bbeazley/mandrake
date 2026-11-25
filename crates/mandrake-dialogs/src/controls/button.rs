use std::sync::Arc;

use mandrake_tag::Rect;
use mandrake_tag::font::{self, Bitmap32, RgbColor, TextClip, text_flags};
use mandrake_view::interface_bitmap::{InterfaceArt, draw_bitmap_from_sequence_shaded, placement};

use super::{Click, ControlProc, DrawContext, ItemState, default_click, item_flags};
use crate::resources::{BAIE, Resources};

pub mod button_flags {
    pub const SHAPE_LEFT: u32 = 0x01;
    pub const STICKS: u32 = 0x02;
    pub const ICON: u32 = 0x04;
    pub const STATE_WHEN_DISABLED: u32 = 0x08;
}

const CHECKBOX_SEQUENCE: i32 = 1;
const RADIO_SEQUENCE: i32 = 2;
const fn default_sequence(width: i32) -> i32 {
    if width <= 0x41 {
        8
    } else if width <= 0x5e {
        3
    } else {
        11
    }
}

pub struct Button {
    original: Rect,
    text_rect: Rect,
    art: Option<Arc<InterfaceArt>>,
    sequence: i32,
    states: i32,
    state: i32,
    transparency: i32,
    flags: u32,
    replacement_colors: Option<Vec<[u16; 4]>>,
    overlay: Option<(Arc<InterfaceArt>, i32)>,
    caption: Vec<u8>,
    color: RgbColor,
    family: Option<Arc<font::FontFamily>>,
}

impl Button {
    pub fn new(item: &mut ItemState, name: &[u8], res: &mut Resources) -> Self {
        let mut button = Self {
            original: item.rect,
            text_rect: item.rect,
            art: None,
            sequence: -1,
            states: 0,
            state: 0,
            transparency: 0,
            flags: 0,
            replacement_colors: None,
            overlay: None,
            caption: name.to_vec(),
            color: res.dialog_color(2),
            family: res.font(0).cloned(),
        };
        let art = res.art(BAIE);
        button.set_shape(item, art, default_sequence(item.rect.width()));
        button
    }

    pub fn new_checkbox(item: &mut ItemState, name: &[u8], res: &mut Resources) -> Self {
        let mut button = Self::new(item, name, res);
        button.flags = button_flags::SHAPE_LEFT;
        let art = res.art(BAIE);
        button.set_shape(item, art, CHECKBOX_SEQUENCE);
        button.color = res.dialog_color(10);
        button
    }

    pub fn new_radio(item: &mut ItemState, name: &[u8], res: &mut Resources) -> Self {
        let mut button = Self::new(item, name, res);
        button.flags = button_flags::SHAPE_LEFT | button_flags::STICKS;
        let art = res.art(BAIE);
        button.set_shape(item, art, RADIO_SEQUENCE);
        button.color = res.dialog_color(11);
        button
    }

    pub fn set_shape(
        &mut self,
        item: &mut ItemState,
        art: Option<Arc<InterfaceArt>>,
        sequence: i32,
    ) {
        item.dirty = true;
        self.sequence = sequence;
        self.art = art;
        let Some(art) = self
            .art
            .as_ref()
            .filter(|a| a.coll.sequence(sequence as usize).is_some())
        else {
            self.states = 0;
            item.flags &= !item_flags::SINGLE_STATE;
            return;
        };
        self.states = art.views(sequence);
        if self.states == 1 {
            item.flags |= item_flags::SINGLE_STATE;
        } else {
            item.flags &= !item_flags::SINGLE_STATE;
        }
        self.text_rect = self.original;
        if self.flags & button_flags::ICON != 0 {
            self.text_rect.left += 0x2d;
        } else if self.flags & button_flags::SHAPE_LEFT != 0
            && let Some((bitmap, _)) = art.bitmap(sequence, 0, 0)
        {
            self.text_rect.left += 8 + bitmap.width;
        }
        let o = self.original;
        let (cx, cy) = (o.left + o.width() / 2, o.top + o.height() / 2);
        let mut rect = o;
        for n in 0..art.frames(sequence) * self.states {
            let Some((bitmap, inst)) = art.bitmap(sequence, -1, n) else {
                continue;
            };
            let (x, y) = (cx - inst.origin.0 as i32, cy - inst.origin.1 as i32);
            rect.left = rect.left.min(x);
            rect.top = rect.top.min(y);
            rect.right = rect.right.max(x + bitmap.width);
            rect.bottom = rect.bottom.max(y + bitmap.height);
        }
        item.rect = rect;
    }

    pub const fn state(&self) -> i32 {
        self.state
    }

    pub fn set_state(&mut self, item: &mut ItemState, state: i32) {
        if self.state != state {
            self.state = state;
            item.dirty = true;
        }
    }

    pub fn set_transparency(&mut self, item: &mut ItemState, transparency: i32) {
        if self.transparency != transparency {
            self.transparency = transparency;
            item.dirty = true;
        }
    }

    pub fn set_replacement_colors(&mut self, item: &mut ItemState, colors: Option<Vec<[u16; 4]>>) {
        self.replacement_colors = colors;
        item.dirty = true;
    }

    pub fn replacement_colors(&self) -> Option<&[[u16; 4]]> {
        self.replacement_colors.as_deref()
    }

    pub fn set_overlay_shape(
        &mut self,
        item: &mut ItemState,
        art: Option<Arc<InterfaceArt>>,
        sequence: i32,
    ) {
        self.overlay = art.filter(|_| sequence != -1).map(|a| (a, sequence));
        item.dirty = true;
    }

    pub fn overlay_shape(&self) -> Option<(&Arc<InterfaceArt>, i32)> {
        self.overlay.as_ref().map(|(a, s)| (a, *s))
    }

    pub const fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }

    pub fn caption(&self) -> &[u8] {
        &self.caption
    }

    pub fn set_caption(&mut self, item: &mut ItemState, caption: &[u8]) {
        self.caption = caption.to_vec();
        item.dirty = true;
    }

    pub fn set_text_color(&mut self, item: &mut ItemState, color: RgbColor) {
        self.color = color;
        item.dirty = true;
    }
}

impl ControlProc for Button {
    fn click(&mut self, item: &mut ItemState, click: &Click) -> bool {
        if !default_click(item, click) {
            return false;
        }
        if self.state == 0 || self.flags & button_flags::STICKS == 0 {
            self.state = if click.shift {
                (if self.state == 0 {
                    self.states
                } else {
                    self.state
                }) - 1
            } else if self.state == self.states - 1 {
                0
            } else {
                self.state + 1
            };
        }
        true
    }

    fn draw(&self, item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        if self.states == 0 {
            return;
        }
        let transparency = if item.enabled() {
            self.transparency
        } else {
            self.transparency.wrapping_mul(-0x7fff) + 0x8000
        };
        if transparency >= 0xffff {
            return;
        }
        let transparency = transparency.max(0) as u32;
        let Some(art) = &self.art else { return };
        let mut rect = self.original;
        let (mut draw_flags, mut offset) = (placement::REGISTRATION, 0);
        let text_flags = if self.flags & button_flags::ICON != 0 {
            rect.right = self.text_rect.left;
            text_flags::SHADOW
        } else if self.flags & button_flags::SHAPE_LEFT != 0 {
            if art.bitmap(self.sequence, 0, 0).is_some() {
                draw_flags |= placement::LEFT;
                offset = 4;
            }
            text_flags::MIDDLE | text_flags::SHADOW
        } else {
            text_flags::CENTER | text_flags::MIDDLE | text_flags::SHADOW
        };
        let view = if item.enabled() || self.flags & button_flags::STATE_WHEN_DISABLED != 0 {
            self.state
        } else {
            0
        };
        let recoloured = self
            .replacement_colors
            .as_deref()
            .map(|c| art.coll.palette_of_colors(c));
        draw_bitmap_from_sequence_shaded(
            bitmap,
            art,
            recoloured.as_deref().unwrap_or(art.palette()),
            self.sequence,
            view,
            item.pressed as i32,
            rect,
            draw_flags,
            offset,
            transparency,
        );
        if let Some((overlay, sequence)) = &self.overlay {
            draw_bitmap_from_sequence_shaded(
                bitmap,
                overlay,
                overlay.palette(),
                *sequence,
                0,
                0,
                rect,
                draw_flags,
                offset,
                transparency,
            );
        }
        if let Some(family) = self.family.as_ref().filter(|_| !self.caption.is_empty()) {
            font::draw_string(
                bitmap,
                self.text_rect,
                TextClip::None,
                family,
                font::STYLE_PLAIN,
                &self.caption,
                self.color,
                text_flags,
                transparency,
                ctx.res.locale(),
            );
        }
    }
}
