use std::sync::Arc;

use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, text_flags};
use mandrake_tag::{DialogItem, Rect};
use mandrake_view::interface_bitmap::{InterfaceArt, draw_bitmap_from_sequence};

use super::{Click, ControlProc, DrawContext, ItemState, default_click};
use crate::resources::{BAIE, Resources};

const SEQUENCE: i32 = 0x10;
const OVERLAP: i32 = 8;
const FRAME_SELECTED: i32 = 0;
const FRAME_BEHIND: i32 = 1;
const DISABLED: u32 = 0x8000;
const NAME_BYTES: usize = 32;

#[derive(Debug, Clone, Default)]
struct Tab {
    name: Vec<u8>,
    enabled: bool,
}

pub struct Tabs {
    bounds: Rect,
    art: Option<Arc<InterfaceArt>>,
    views: i32,
    width: i32,
    tabs: Vec<Tab>,
    selected: usize,
    color: RgbColor,
    family: Option<Arc<FontFamily>>,
}

impl Tabs {
    pub fn new(item: &DialogItem, res: &mut Resources) -> Self {
        let art = res.art(BAIE);
        let views = art.as_ref().map_or(0, |a| a.views(SEQUENCE));
        let width = art.as_ref().map_or(0, |a| {
            (0..a.frames(SEQUENCE))
                .filter_map(|n| a.bitmap(SEQUENCE, -1, n))
                .next_back()
                .map_or(0, |(bitmap, _)| bitmap.width)
        });
        Self {
            bounds: item.rect,
            art,
            views,
            width,
            tabs: Vec::new(),
            selected: 0,
            color: res.dialog_color(2),
            family: res.font(0).cloned(),
        }
    }

    pub fn set_count(&mut self, item: &mut ItemState, count: usize) {
        if self.tabs.len() != count {
            self.tabs = vec![Tab::default(); count];
            if self.selected >= count {
                self.selected = 0;
            }
            item.dirty = true;
        }
    }

    pub fn set_name(&mut self, item: &mut ItemState, tab: usize, name: &[u8]) {
        let name = &name[..name.len().min(NAME_BYTES)];
        if let Some(t) = self.tabs.get_mut(tab)
            && t.name != name
        {
            t.name = name.to_vec();
            t.enabled = true;
            item.dirty = true;
        }
    }

    pub fn set_enabled(&mut self, tab: usize, enabled: bool) {
        if let Some(t) = self.tabs.get_mut(tab) {
            t.enabled = enabled;
        }
    }

    pub const fn selected(&self) -> usize {
        self.selected
    }

    pub fn set_selected(&mut self, item: &mut ItemState, tab: usize) {
        self.selected = tab;
        item.dirty = true;
    }

    fn rect(&self, tab: usize) -> Rect {
        let left = self.bounds.left + (self.width - OVERLAP) * tab as i32;
        Rect::new(self.bounds.top, left, self.bounds.bottom, left + self.width)
    }

    fn tab_at(&self, x: i32) -> Option<usize> {
        (0..self.tabs.len()).find(|&tab| {
            let r = self.rect(tab);
            let (from, to) = if tab > self.selected {
                (r.left + OVERLAP, r.right)
            } else if tab < self.selected {
                (r.left, r.right - OVERLAP)
            } else {
                (r.left, r.right)
            };
            (from..to).contains(&x)
        })
    }

    fn draw_tab(
        &self,
        tab: usize,
        frame: i32,
        art: &InterfaceArt,
        ctx: &DrawContext<'_>,
        bitmap: &mut Bitmap32,
    ) {
        let t = &self.tabs[tab];
        let transparency = if t.enabled { 0 } else { DISABLED };
        let rect = self.rect(tab);
        draw_bitmap_from_sequence(bitmap, art, SEQUENCE, 0, frame, rect, 0, 0, transparency);
        if let Some(family) = &self.family {
            font::draw_string(
                bitmap,
                rect,
                TextClip::None,
                family,
                font::STYLE_PLAIN,
                &t.name,
                self.color,
                text_flags::CENTER | text_flags::MIDDLE | text_flags::SHADOW,
                transparency,
                ctx.res.locale(),
            );
        }
    }
}

impl ControlProc for Tabs {
    fn click(&mut self, item: &mut ItemState, click: &Click) -> bool {
        if !default_click(item, click) {
            return false;
        }
        if let Some(tab) = self.tab_at(click.point.0)
            && self.tabs[tab].enabled
        {
            self.selected = tab;
        }
        item.dirty = true;
        true
    }

    fn draw(&self, _item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let Some(art) = self.art.as_ref().filter(|_| self.views != 0) else {
            return;
        };
        if self.selected >= self.tabs.len() {
            return;
        }
        for tab in (self.selected + 1..self.tabs.len()).rev() {
            self.draw_tab(tab, FRAME_BEHIND, art, ctx, bitmap);
        }
        for tab in 0..self.selected {
            self.draw_tab(tab, FRAME_BEHIND, art, ctx, bitmap);
        }
        self.draw_tab(self.selected, FRAME_SELECTED, art, ctx, bitmap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip() -> Tabs {
        Tabs {
            bounds: Rect::new(152, 30, 174, 182),
            art: None,
            views: 0,
            width: 68,
            tabs: vec![
                Tab {
                    name: b"General".to_vec(),
                    enabled: true,
                },
                Tab {
                    name: b"Level".to_vec(),
                    enabled: true,
                },
            ],
            selected: 0,
            color: [0; 3],
            family: None,
        }
    }

    #[test]
    fn the_overlap_goes_to_the_selected_side() {
        let mut t = strip();
        assert_eq!(t.tab_at(30), Some(0));
        assert_eq!(t.tab_at(97), Some(0));
        assert_eq!(t.tab_at(98), Some(1));
        assert_eq!(t.tab_at(157), Some(1));
        assert_eq!(t.tab_at(158), None);
        assert_eq!(t.tab_at(29), None);
        t.selected = 1;
        assert_eq!(t.tab_at(89), Some(0));
        assert_eq!(t.tab_at(90), Some(1));
    }

    fn state() -> ItemState {
        ItemState {
            id: 6,
            kind: mandrake_common::FourCC(*b"tabc"),
            rect: Rect::new(152, 30, 174, 182),
            flags: 0,
            pressed: false,
            dirty: false,
        }
    }

    fn release_at(x: i32) -> Click {
        Click {
            point: (x, 160),
            held: false,
            shift: false,
            double: false,
            now_tick: 0,
        }
    }

    #[test]
    fn a_disabled_tab_doesnt_select() {
        let mut t = strip();
        t.set_enabled(1, false);
        let mut item = state();
        assert!(t.click(&mut item, &release_at(120)));
        assert_eq!(t.selected(), 0);
        t.set_enabled(1, true);
        assert!(t.click(&mut item, &release_at(120)));
        assert_eq!(t.selected(), 1);
    }

    #[test]
    fn a_new_name_enables_a_tab_and_the_same_name_doesnt() {
        let mut t = strip();
        let mut item = state();
        t.set_enabled(1, false);
        t.set_name(&mut item, 1, b"Level");
        assert!(!t.tabs[1].enabled);
        t.set_name(&mut item, 1, b"Other");
        assert!(t.tabs[1].enabled);
    }

    #[test]
    fn set_count_resets_out_of_range_selection() {
        let mut t = strip();
        let mut item = state();
        t.set_selected(&mut item, 1);
        assert_eq!(t.selected(), 1);
        t.set_count(&mut item, 1);
        assert_eq!(t.selected(), 0);
    }
}
