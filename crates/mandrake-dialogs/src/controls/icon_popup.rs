use std::sync::Arc;

use mandrake_tag::Rect;
use mandrake_tag::font::Bitmap32;
use mandrake_view::interface_bitmap::{
    InterfaceArt, copy_bitmap, draw_bitmap_from_sequence_shaded, placement,
    tile_and_bevel_rectangle,
};

use super::list::{fill_rect, inset};
use super::{Click, ControlProc, DrawContext, ItemState};
use crate::resources::{BAIE, Resources};

const BOX_SEQUENCE: i32 = 6;
const ARROW_SEQUENCE: i32 = 7;
const BOX_ORDER: [i32; 9] = [0, 1, 2, -1, 4, 5, 6, 7, 8];
const ARROW_FLAGS: u32 = placement::RIGHT | placement::REGISTRATION;

pub struct IconPopup {
    baie: Option<Arc<InterfaceArt>>,
    art: Option<Arc<InterfaceArt>>,
    sequence: i32,
    highlight: i32,
    cell: (i32, i32),
    spacing: i32,
    columns: i32,
    rows: i32,
    value: i16,
    hot: i16,
    count: i16,
    closed: Rect,
    open_rect: Rect,
    open: bool,
    colors: [[u16; 4]; 2],
}

impl IconPopup {
    pub fn new(item: &ItemState, res: &mut Resources) -> Self {
        Self {
            baie: res.art(BAIE),
            art: None,
            sequence: 0,
            highlight: -1,
            cell: (0, 0),
            spacing: 0,
            columns: 1,
            rows: 0,
            value: 0,
            hot: -1,
            count: 0,
            closed: item.rect,
            open_rect: item.rect,
            open: false,
            colors: [[0; 4]; 2],
        }
    }

    pub fn set_information(
        &mut self,
        item: &mut ItemState,
        art: Option<Arc<InterfaceArt>>,
        sequence: i32,
        highlight: i32,
        value: i16,
        spacing: i32,
    ) {
        self.art = art;
        self.sequence = sequence;
        self.highlight = highlight;
        self.value = value;
        self.spacing = spacing;
        self.hot = -1;
        self.open = false;
        let Some(art) = self.art.clone() else {
            return;
        };
        if let Some((b, _)) = art
            .bitmap(highlight, -1, 0)
            .or_else(|| art.bitmap(sequence, -1, 0))
        {
            self.cell = (b.width, b.height);
            let r = &mut self.closed;
            if r.width() < b.width + 4 || r.height() < b.height + 4 {
                r.right = r.left + b.width + 4;
                r.bottom = r.top + b.height + 4;
            }
        }
        item.rect = self.closed;
        self.count = (art.frames(sequence) * art.views(sequence).max(1)) as i16;
        let root = (f64::from(self.count).sqrt() + 0.5) as i32;
        self.columns = if root == 0 { 1 } else { root.min(8) };
        self.rows = self.default_rows();
        self.layout();
        item.dirty = true;
    }

    pub fn set_columns(&mut self, item: &mut ItemState, columns: i32) {
        self.columns = columns.max(1);
        self.rows = self.default_rows();
        self.layout();
        item.dirty = true;
    }

    pub fn set_rows(&mut self, item: &mut ItemState, rows: i32) {
        self.rows = rows;
        self.layout();
        item.dirty = true;
    }

    const fn default_rows(&self) -> i32 {
        if self.count <= 0 {
            0
        } else {
            (self.count as i32 + self.columns - 1) / self.columns
        }
    }

    pub(crate) fn close(&mut self, item: &mut ItemState) {
        if self.open {
            self.hot = -1;
            item.rect = self.closed;
            self.open = false;
            item.dirty = true;
        }
    }

    fn layout(&mut self) {
        let (w, h) = self.cell;
        let s = self.spacing;
        self.open_rect = Rect::new(
            self.closed.top,
            self.closed.left,
            self.closed.top + self.rows * (h + s) + 4 - s,
            self.closed.left + self.columns * (w + s) + 4 - s,
        );
    }

    pub fn set_colors(&mut self, item: &mut ItemState, colors: [[u16; 4]; 2]) {
        if self.colors != colors {
            self.colors = colors;
            item.dirty = true;
        }
    }

    pub const fn value(&self) -> i16 {
        self.value
    }

    const fn icon_at(&self, n: i16) -> i16 {
        if n == 0 {
            self.value
        } else if n == self.value {
            0
        } else {
            n
        }
    }

    fn cell_rect(&self, n: i16) -> Rect {
        let area = inset(
            if self.open {
                self.open_rect
            } else {
                self.closed
            },
            2,
            2,
        );
        let (w, h) = self.cell;
        let (col, row) = (i32::from(n) % self.columns, i32::from(n) / self.columns);
        let (x, y) = (
            area.left + col * (w + self.spacing),
            area.top + row * (h + self.spacing),
        );
        Rect::new(y, x, y + h, x + w)
    }

    fn palette(art: &InterfaceArt, colors: &[[u16; 4]; 2]) -> Vec<[u8; 4]> {
        art.coll.palette_of_colors(colors)
    }
}

fn draw_icon(
    bitmap: &mut Bitmap32,
    art: &InterfaceArt,
    palette: &[[u8; 4]],
    sequence: i32,
    icon: i32,
    cell: Rect,
    transparency: u32,
) {
    let Some((b, inst)) = art.bitmap(sequence, -1, icon) else {
        return;
    };
    let x = cell.left + cell.width() / 2 - i32::from(inst.origin.0);
    let y = cell.top + cell.height() / 2 - i32::from(inst.origin.1);
    copy_bitmap(bitmap, b, palette, x, y, cell, transparency);
}

impl ControlProc for IconPopup {
    fn draw(&self, item: &ItemState, _ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let rect = if self.open {
            self.open_rect
        } else {
            self.closed
        };
        let transparency = if item.enabled() { 0 } else { 0x8000 };
        if let Some(baie) = &self.baie {
            tile_and_bevel_rectangle(bitmap, baie, BOX_SEQUENCE, -1, rect, None, Some(&BOX_ORDER));
            if !self.open {
                let palette = Self::palette(baie, &self.colors);
                draw_bitmap_from_sequence_shaded(
                    bitmap,
                    baie,
                    &palette,
                    ARROW_SEQUENCE,
                    -1,
                    0,
                    rect,
                    ARROW_FLAGS,
                    0,
                    transparency,
                );
            }
        }
        let Some(art) = &self.art else { return };
        let palette = Self::palette(art, &self.colors);
        let shown = if self.open {
            self.count
        } else {
            1.min(self.count)
        };
        for n in 0..shown {
            let cell = self.cell_rect(n);
            if self.hot == n {
                if self.highlight == -1 {
                    fill_rect(bitmap, [0xffff; 3], cell, 0);
                } else {
                    draw_bitmap_from_sequence_shaded(
                        bitmap,
                        art,
                        &palette,
                        self.highlight,
                        -1,
                        0,
                        cell,
                        0,
                        0,
                        transparency,
                    );
                }
            }
            draw_icon(
                bitmap,
                art,
                &palette,
                self.sequence,
                i32::from(self.icon_at(n)),
                cell,
                transparency,
            );
        }
    }

    fn click(&mut self, item: &mut ItemState, click: &Click) -> bool {
        let before = self.value;
        if self.art.is_none() {
            return false;
        }
        if click.held {
            let area = if self.open {
                self.open_rect
            } else {
                self.closed
            };
            let hot = if area.contains(click.point.0, click.point.1) {
                (0..self.count)
                    .find(|&n| self.cell_rect(n).contains(click.point.0, click.point.1))
                    .unwrap_or(self.hot)
            } else {
                -1
            };
            if hot != self.hot {
                self.hot = hot;
                item.dirty = true;
            }
            if !self.open {
                self.open = true;
                item.rect = self.open_rect;
                item.dirty = true;
            }
        } else if self.open {
            if self.hot != -1 {
                self.value = self.icon_at(self.hot);
            }
            self.hot = -1;
            item.rect = self.closed;
            self.open = false;
            item.dirty = true;
        }
        self.value != before
    }
}
