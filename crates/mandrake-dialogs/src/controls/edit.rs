use std::borrow::Cow;
use std::sync::Arc;

use mandrake_session::UiKey;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, text_flags};
use mandrake_tag::{DialogItem, Rect};

use super::list::{fill_rect, frame_rect, inset};
use super::{Click, ControlProc, DrawContext, ItemState, Key, item_flags};
use crate::resources::Resources;

const MAX_LENGTH: usize = 0xff;
const TEXT_FLAGS: u32 = text_flags::MULTILINE | text_flags::SHADOW | text_flags::MIDDLE;
pub mod edit_flags {
    pub const NO_WIDTH_LIMIT: u32 = 0x100;
    pub const NO_WIDTH_LIMIT_TOO: u32 = 0x80;
    pub const PASSWORD: u32 = 0x08;
    pub const FOCUS_RING: u32 = 0x20;
    pub const NO_BOX: u32 = 0x10;
}

const BLINK_TICKS: u64 = 15;
const WHITE: RgbColor = [0xffff; 3];
const BLACK: RgbColor = [0; 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastClick {
    None,
    TookFocus,
    Selecting,
}

pub struct Edit {
    rect: Rect,
    text: Vec<u8>,
    start: usize,
    end: usize,
    scroll: i32,
    anchor: usize,
    last_click: LastClick,
    focused: bool,
    caret_on: bool,
    typed_tick: u64,
    color: RgbColor,
    ring: RgbColor,
    family: Option<Arc<FontFamily>>,
    wants_focus: bool,
    flags: u32,
    placeholder: Option<Vec<u8>>,
}

impl Edit {
    pub fn new(item: &DialogItem, res: &mut Resources) -> Self {
        Self {
            rect: item.rect,
            text: item.name.clone(),
            start: 0,
            end: 0,
            scroll: 0,
            anchor: 0,
            last_click: LastClick::None,
            focused: false,
            caret_on: false,
            typed_tick: 0,
            color: res.dialog_color(1),
            ring: res.edit_ring(),
            family: res.font(0).cloned(),
            wants_focus: false,
            flags: edit_flags::FOCUS_RING,
            placeholder: None,
        }
    }

    pub fn new_password(item: &DialogItem, res: &mut Resources) -> Self {
        let mut edit = Self::new(item, res);
        edit.flags |= edit_flags::PASSWORD;
        edit
    }

    pub fn set_placeholder(&mut self, item: &mut ItemState, text: &[u8]) {
        self.placeholder = Some(text.to_vec());
        item.dirty = true;
    }

    pub fn set_flag(&mut self, item: &mut ItemState, flag: u32, on: bool) {
        if on {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
        item.dirty = true;
    }

    fn follow(&mut self, at: usize) {
        let at = at.min(self.text.len());
        let x = self.measure(&self.text[..at]) + 1;
        if x < self.scroll {
            self.scroll = x;
        } else if self.scroll < x + 3 - self.rect.width() {
            self.scroll = x + 3 - self.rect.width();
        }
    }

    const fn displayed_byte(&self, byte: u8) -> u8 {
        if self.flags & edit_flags::PASSWORD != 0 {
            0xa5
        } else {
            byte
        }
    }

    fn measure(&self, text: &[u8]) -> i32 {
        self.family.as_ref().map_or(0, |family| {
            text.iter()
                .filter_map(|&c| family.glyph(font::STYLE_PLAIN, u16::from(self.displayed_byte(c))))
                .map(|g| i32::from(g.advance))
                .sum::<i32>()
        })
    }

    pub fn text(&self) -> &[u8] {
        &self.text
    }

    pub fn set_text(&mut self, item: &mut ItemState, text: &[u8]) {
        self.text = text[..text.len().min(MAX_LENGTH)].to_vec();
        self.start = 0;
        self.end = 0;
        self.scroll = 0;
        item.dirty = true;
    }

    pub const fn prepare(item: &mut ItemState) {
        item.flags |= item_flags::WANTS_IDLE;
    }

    fn width(&self, text: &[u8], res: &Resources) -> i32 {
        self.family.as_ref().map_or(0, |f| {
            font::string_width(f, font::STYLE_PLAIN, text, res.locale())
        })
    }

    pub fn select_range(&mut self, item: &mut ItemState, from: usize, to: usize) {
        self.select(from, to);
        item.dirty = true;
    }

    fn select(&mut self, from: usize, to: usize) {
        let to = to.min(self.text.len());
        self.end = to;
        self.start = from.min(to);
    }

    fn delete_selection(&mut self) {
        self.text.drain(self.start..self.end);
        self.end = self.start;
    }

    fn click_point(&self, x: i32) -> usize {
        let Some(family) = &self.family else { return 0 };
        let x = x + 1 - self.rect.left;
        let mut width = 0;
        for (i, &byte) in self.text.iter().enumerate() {
            let advance = family
                .glyph(font::STYLE_PLAIN, u16::from(self.displayed_byte(byte)))
                .map_or(0, |g| i32::from(g.advance));
            if x < width + advance / 2 {
                return i;
            }
            width += advance;
        }
        self.text.len()
    }

    const fn restart_blink(&mut self, tick: u64) {
        self.typed_tick = tick;
        self.caret_on = true;
    }
}

impl ControlProc for Edit {
    fn click(&mut self, item: &mut ItemState, click: &Click) -> bool {
        if !click.held {
            self.last_click = LastClick::None;
            return false;
        }
        match self.last_click {
            LastClick::None => {
                if !self.focused {
                    self.wants_focus = true;
                    self.last_click = LastClick::TookFocus;
                    return false;
                }
                let at = self.click_point(click.point.0);
                if click.shift {
                    let (from, to) = (self.start.min(at), self.end.max(at));
                    self.select(from, to);
                } else {
                    self.anchor = at;
                    self.select(at, at);
                    self.last_click = LastClick::Selecting;
                    self.restart_blink(click.now_tick);
                }
            }
            LastClick::Selecting => {
                let at = self.click_point(click.point.0);
                let (from, to) = (self.anchor.min(at), self.anchor.max(at));
                self.select(from, to);
            }
            LastClick::TookFocus => return false,
        }
        item.dirty = true;
        false
    }

    fn key(&mut self, item: &mut ItemState, key: &Key) -> bool {
        match key.key {
            UiKey::Backspace => {
                if self.start != self.end {
                    self.delete_selection();
                } else if self.start > 0 {
                    self.start -= 1;
                    self.end = self.start;
                    self.text.remove(self.start);
                } else {
                    return false;
                }
                self.follow(self.start);
            }
            UiKey::Delete => {
                if self.start != self.end {
                    self.delete_selection();
                } else if self.start < self.text.len() {
                    self.text.remove(self.start);
                } else {
                    return false;
                }
                self.follow(self.start);
            }
            UiKey::Up | UiKey::Home => {
                self.start = 0;
                if !key.shift {
                    self.end = 0;
                }
                self.follow(self.start);
            }
            UiKey::Down | UiKey::End => {
                self.end = self.text.len();
                if !key.shift {
                    self.start = self.end;
                }
                self.follow(self.end);
            }
            UiKey::Left => {
                self.start = self.start.saturating_sub(1);
                if !key.shift {
                    self.end = self.start;
                }
                self.follow(self.start);
            }
            UiKey::Right => {
                self.end = (self.end + 1).min(self.text.len());
                if !key.shift {
                    self.start = self.end;
                }
                self.follow(self.end);
            }
            _ => return false,
        }
        self.restart_blink(key.now_tick);
        item.dirty = true;
        true
    }

    fn text(&mut self, item: &mut ItemState, ch: char) -> bool {
        if !ch.is_ascii() || self.start > self.end {
            return false;
        }
        let selected = self.end - self.start;
        if self.text.len() + 1 - selected > MAX_LENGTH {
            return false;
        }
        let byte = ch as u8;
        let unlimited =
            self.flags & (edit_flags::NO_WIDTH_LIMIT | edit_flags::NO_WIDTH_LIMIT_TOO) != 0;
        if self.family.is_some() && !unlimited {
            let grown = self.measure(&self.text) + self.measure(&[byte]);
            let cut = self.measure(&self.text[self.start..self.end]);
            if grown - cut >= self.rect.width() - 2 {
                return false;
            }
        }
        self.delete_selection();
        self.text.insert(self.start, byte);
        self.start += 1;
        self.end = self.start;
        self.follow(self.start);
        item.dirty = true;
        true
    }

    fn can_focus(&self, item: &ItemState) -> bool {
        item.live()
    }

    fn focus(&mut self, item: &mut ItemState, focused: bool) -> bool {
        if focused {
            if !item.live() {
                return false;
            }
            self.focused = true;
            self.select(0, usize::MAX);
        } else {
            self.focused = false;
            self.caret_on = false;
            self.select(usize::MAX, usize::MAX);
            self.start = self.end;
        }
        item.dirty = true;
        true
    }

    fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.wants_focus)
    }

    fn idle(&mut self, _item: &mut ItemState, now_tick: u64) -> bool {
        if !self.focused || self.start != self.end {
            return false;
        }
        let on = (now_tick.saturating_sub(self.typed_tick)) % (BLINK_TICKS * 2) < BLINK_TICKS;
        let changed = on != self.caret_on;
        self.caret_on = on;
        changed
    }

    fn draw(&self, _item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        if ctx.focused && self.flags & edit_flags::FOCUS_RING != 0 {
            let ring = inset(self.rect, -4, -4);
            frame_rect(bitmap, self.ring, ring, 0x8000);
            frame_rect(bitmap, self.ring, inset(ring, 1, 1), 0);
        }
        if self.flags & edit_flags::NO_BOX == 0 {
            frame_rect(bitmap, BLACK, self.rect, 0x8000);
            fill_rect(bitmap, BLACK, inset(self.rect, 1, 1), 0x5555);
        }
        let inner = inset(self.rect, 1, 1);
        let Some(family) = &self.family else { return };
        let locale = ctx.res.locale();
        let (shown, transparency): (Cow<'_, [u8]>, u32) = match &self.placeholder {
            Some(p) if self.text.is_empty() => (Cow::Borrowed(p.as_slice()), 0x8000),
            _ if self.flags & edit_flags::PASSWORD != 0 => {
                (Cow::Owned(vec![0xa5; self.text.len()]), 0)
            }
            _ => (Cow::Borrowed(self.text.as_slice()), 0),
        };
        let text: &[u8] = &shown;
        let draw = |bitmap: &mut Bitmap32, pen, text: &[u8], color, flags| {
            font::draw_string_at(
                bitmap,
                pen,
                inner,
                TextClip::None,
                family,
                font::STYLE_PLAIN,
                text,
                color,
                flags,
                transparency,
                locale,
            )
        };
        let (start, end) = (self.start.min(text.len()), self.end.min(text.len()));
        let mut pen = (-self.scroll, 0);
        pen = draw(bitmap, pen, &text[..start], self.color, TEXT_FLAGS);
        if ctx.focused && start == end && self.caret_on {
            let x = inner.left + pen.0 + 1;
            let height = family.font(font::STYLE_PLAIN).line_height();
            let top = inner.top + (inner.height() - height) / 2;
            let caret = Rect::new(top, x, top + height, x + 1);
            let caret = Rect::new(
                caret.top.max(inner.top),
                caret.left,
                caret.bottom.min(inner.bottom),
                caret.right.min(inner.right),
            );
            fill_rect(bitmap, self.color, caret, 0);
        }
        if start != end {
            let x = inner.left + pen.0 + 1;
            let right = if end < text.len() {
                (x + self.width(&text[start..end], ctx.res)).min(inner.right)
            } else {
                inner.right
            };
            let height = family.font(font::STYLE_PLAIN).line_height();
            let bar = Rect::new(inner.top, x.max(inner.left), inner.top + height, right);
            fill_rect(bitmap, WHITE, bar, 0x4000);
            let flags = TEXT_FLAGS & !(text_flags::WRAP | text_flags::SHADOW);
            pen = draw(bitmap, pen, &text[start..end], BLACK, flags);
        }
        if end < text.len() {
            draw(bitmap, pen, &text[end..], self.color, TEXT_FLAGS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_common::FourCC;

    #[test]
    fn password_mode_maps_all_bytes_to_bullet_for_metrics() {
        let mut res = Resources::from_source(|_, _| None);
        let def = DialogItem {
            kind: FourCC(*b"pass"),
            field_04: 0,
            id: 1,
            fields_08: [0; 4],
            rect: Rect::new(0, 0, 20, 100),
            name: b"abc".to_vec(),
        };
        let edit = Edit::new_password(&def, &mut res);
        assert_eq!(edit.displayed_byte(b'i'), 0xa5);
        assert_eq!(edit.displayed_byte(b'W'), 0xa5);
        let plain = Edit::new(&def, &mut res);
        assert_eq!(plain.displayed_byte(b'i'), b'i');
    }

    #[test]
    fn backspace_and_delete_follow_caret_when_scrolled() {
        let mut res = Resources::from_source(|_, _| None);
        let def = DialogItem {
            kind: FourCC(*b"edit"),
            field_04: 0,
            id: 1,
            fields_08: [0; 4],
            rect: Rect::new(0, 0, 20, 40),
            name: b"abcdef".to_vec(),
        };
        let mut item = ItemState {
            id: 1,
            kind: FourCC(*b"edit"),
            rect: def.rect,
            flags: 0,
            pressed: false,
            dirty: false,
        };
        let mut edit = Edit::new(&def, &mut res);
        edit.select(3, 3);
        edit.scroll = 50;
        assert!(edit.key(
            &mut item,
            &Key {
                key: UiKey::Backspace,
                repeat: false,
                shift: false,
                alt: false,
                now_tick: 1,
            }
        ));
        assert!(edit.scroll <= 1);
        edit.scroll = 50;
        assert!(edit.key(
            &mut item,
            &Key {
                key: UiKey::Delete,
                repeat: false,
                shift: false,
                alt: false,
                now_tick: 2,
            }
        ));
        assert!(edit.scroll <= 1);
    }
}
