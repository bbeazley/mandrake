use std::sync::Arc;

use mandrake_tag::Rect;
use mandrake_tag::font::{self, Bitmap32, RgbColor, TextClip, text_flags};
use mandrake_view::interface_bitmap::{InterfaceArt, draw_bitmap_from_sequence, placement};

use super::list::{fill_rect, frame_rect};
use super::{Click, ControlProc, DrawContext, ItemState};
use crate::resources::{BAIE, Resources};

const ARROWS_SEQUENCE: i32 = 0xf;
const ARROWS_WIDTH: i32 = 0x14;
const REPEAT_TICKS: u64 = 0xf;
const TIME_MAX: i32 = 810_000;
const BLACK: RgbColor = [0, 0, 0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Integer,
    Time,
}

impl Format {
    pub fn text(self, value: i32) -> Vec<u8> {
        match self {
            Self::Integer => value.to_string().into_bytes(),
            Self::Time => {
                let v = value.clamp(0, TIME_MAX);
                format!("{}:{:02}", v / 1800, v % 1800 / 30).into_bytes()
            }
        }
    }
}

pub struct DoubleArrow {
    original: Rect,
    art: Option<Arc<InterfaceArt>>,
    part: i32,
    last_step: u64,
    min: i32,
    max: i32,
    value: i32,
    deltas: Vec<(i32, i32)>,
    format: Format,
    reversed: bool,
    color: RgbColor,
    family: Option<Arc<font::FontFamily>>,
}

impl DoubleArrow {
    pub fn new(item: &mut ItemState, res: &mut Resources) -> Self {
        let original = item.rect;
        let art = res
            .art(BAIE)
            .filter(|a| a.coll.sequence(ARROWS_SEQUENCE as usize).is_some());
        if let Some(art) = &art {
            let (cx, cy) = (
                original.left + original.width() / 2,
                original.top + original.height() / 2,
            );
            let mut rect = original;
            let count = art.frames(ARROWS_SEQUENCE) * art.views(ARROWS_SEQUENCE);
            for n in 0..count {
                let Some((bitmap, inst)) = art.bitmap(ARROWS_SEQUENCE, -1, n) else {
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
        Self {
            original,
            art,
            part: 0,
            last_step: 0,
            min: 0,
            max: 0,
            value: 0,
            deltas: Vec::new(),
            format: Format::Integer,
            reversed: false,
            color: res.dialog_color(1),
            family: res.font(0).cloned(),
        }
    }

    pub const fn value(&self) -> i32 {
        self.value
    }

    pub fn set_value(&mut self, item: &mut ItemState, min: i32, max: i32, value: i32) {
        let old = self.value;
        let min = if min == -1 { self.min } else { min };
        self.min = min;
        let max = (if max == -1 { self.max } else { max }).max(min);
        let value = if value == -1 { old } else { value };
        self.max = max;
        self.value = if value < min { min } else { value.min(max) };
        if self.value != old {
            item.dirty = true;
        }
    }

    pub const fn maximum(&self) -> i32 {
        self.max
    }

    pub fn set_deltas(&mut self, deltas: &[(i32, i32)]) {
        self.deltas = deltas.to_vec();
    }

    pub fn set_format(&mut self, item: &mut ItemState, format: Format) {
        self.format = format;
        item.dirty = true;
    }

    pub const fn set_reversed(&mut self, reversed: bool) {
        self.reversed = reversed;
    }

    fn step(&self, up: bool) -> i32 {
        let mut delta = 1;
        let mut negate = !up;
        for &(at, d) in &self.deltas {
            delta = d;
            if self.value < at {
                break;
            }
            if self.value == at && !up {
                negate = true;
                break;
            }
        }
        if negate { -delta } else { delta }
    }
}

impl ControlProc for DoubleArrow {
    fn draw(&self, item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let transparency = if item.enabled() { 0 } else { 0x8000 };
        let mut rect = self.original;
        rect.right -= ARROWS_WIDTH;
        frame_rect(bitmap, BLACK, rect, 0x8000);
        let inner = Rect::new(rect.top + 1, rect.left + 1, rect.bottom - 1, rect.right - 1);
        fill_rect(bitmap, BLACK, inner, 0x5555);
        if let Some(family) = &self.family {
            font::draw_string(
                bitmap,
                inner,
                TextClip::None,
                family,
                font::STYLE_PLAIN,
                &self.format.text(self.value),
                self.color,
                text_flags::MIDDLE | text_flags::RIGHT,
                transparency,
                ctx.res.locale(),
            );
        }
        if let Some(art) = &self.art {
            draw_bitmap_from_sequence(
                bitmap,
                art,
                ARROWS_SEQUENCE,
                -1,
                self.part,
                self.original,
                placement::RIGHT,
                0,
                transparency,
            );
        }
    }

    fn click(&mut self, item: &mut ItemState, click: &Click) -> bool {
        item.dirty = true;
        if !click.held {
            self.last_step = 0;
            self.part = 0;
            return item.rect.contains(click.point.0, click.point.1);
        }
        let r = item.rect;
        if !r.contains(click.point.0, click.point.1) {
            self.part = 0;
            return false;
        }
        self.part = if r.top + r.height() / 2 <= click.point.1 {
            2
        } else {
            1
        };
        if self.last_step != 0 && click.now_tick.saturating_sub(self.last_step) <= REPEAT_TICKS {
            return false;
        }
        self.last_step = click.now_tick.max(1);
        let up = (self.part == 1) != self.reversed;
        self.value = if click.shift {
            if up { self.max } else { self.min }
        } else {
            let next = self.value + self.step(up);
            if next < self.min {
                self.min
            } else if next > self.max {
                self.max
            } else {
                next
            }
        };
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLANNING: [(i32, i32); 3] = [(1800, 450), (9000, 900), (18000, 1800)];
    const TIME_LIMIT: [(i32, i32); 5] = [
        (18000, 1800),
        (36000, 3600),
        (162_000, 9000),
        (270_000, 18000),
        (810_000, 90000),
    ];

    fn arrows(min: i32, max: i32, value: i32, deltas: &[(i32, i32)]) -> DoubleArrow {
        let mut a = DoubleArrow {
            original: Rect::new(0, 0, 16, 60),
            art: None,
            part: 0,
            last_step: 0,
            min: 0,
            max: 0,
            value: 0,
            deltas: deltas.to_vec(),
            format: Format::Time,
            reversed: false,
            color: BLACK,
            family: None,
        };
        let mut item = ItemState {
            id: 1,
            kind: mandrake_common::FourCC(*b"numb"),
            rect: Rect::new(0, 0, 16, 60),
            flags: 0,
            pressed: false,
            dirty: false,
        };
        a.set_value(&mut item, min, max, value);
        a
    }

    #[test]
    fn the_time_limit_steps_by_its_table() {
        let a = arrows(1800, 810_000, 1800, &TIME_LIMIT);
        assert_eq!(a.step(true), 1800);
        assert_eq!(a.step(false), -1800);
        let a = arrows(1800, 810_000, 18000, &TIME_LIMIT);
        assert_eq!((a.step(true), a.step(false)), (3600, -1800));
        let a = arrows(1800, 810_000, 810_000, &TIME_LIMIT);
        assert_eq!(a.step(false), -90000);
        let a = arrows(450, 18000, 1800, &PLANNING);
        assert_eq!((a.step(true), a.step(false)), (900, -450));
    }

    #[test]
    fn values_hold_to_the_bounds_and_minus_one_keeps() {
        let mut item = ItemState {
            id: 1,
            kind: mandrake_common::FourCC(*b"numb"),
            rect: Rect::new(0, 0, 16, 60),
            flags: 0,
            pressed: false,
            dirty: false,
        };
        let mut a = arrows(1, 16, 20, &[]);
        assert_eq!(a.value(), 16);
        a.set_value(&mut item, 2, -1, -1);
        assert_eq!((a.min, a.max, a.value()), (2, 16, 16));
        a.set_value(&mut item, 2, 4, 0);
        assert_eq!(a.value(), 2);
    }

    #[test]
    fn times_are_minutes_and_seconds() {
        assert_eq!(Format::Time.text(1800), b"1:00");
        assert_eq!(Format::Time.text(450), b"0:15");
        assert_eq!(Format::Time.text(10 * 1800), b"10:00");
        assert_eq!(Format::Integer.text(16), b"16");
    }
}
