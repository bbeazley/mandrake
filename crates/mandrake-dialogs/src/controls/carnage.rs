use std::sync::Arc;

use mandrake_common::FourCC;
use mandrake_session::{FilmPages, Graph, RankRow};
use mandrake_tag::Rect;
use mandrake_tag::font::string_width;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, TextLocale};
use mandrake_tag::mesh_links::string_list_lines;

use super::list::{fill_rect, inset};
use super::{ControlProc, DrawContext, ItemState};
use crate::resources::Resources;

const STRINGS: FourCC = FourCC(*b"stli");
const RANK_STRINGS: FourCC = FourCC(*b"in02");
const PERM_STRINGS: FourCC = FourCC(*b"in01");
const GRAPH_TITLES: FourCC = FourCC(*b"ingn");
const WHITE: RgbColor = [0xffff; 3];
const GAP: i32 = 5;
const NAME_FLAGS: u32 = font::text_flags::SHADOW | font::text_flags::MIDDLE;
const NUMBER_FLAGS: u32 = NAME_FLAGS | font::text_flags::CENTER;
const ROW_TRANSPARENCY: u32 = 0x8000;
const HEADER_TRANSPARENCY: u32 = 0x2aaa;
const GRAPH_TRANSPARENCY: u32 = 0x5555;
const GRAPH_TOP: i32 = 0x12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Stats(i16),
    Graph(i16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Column {
    cell: usize,
    heading: Vec<u8>,
    width: i32,
    advance: i32,
    name: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatsRows {
    rows: Vec<RankRow>,
    columns: Vec<Column>,
    in_team_indent: i32,
}

impl StatsRows {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(super) fn draw_row(
        &self,
        index: usize,
        rect: Rect,
        clip: Rect,
        family: &FontFamily,
        locale: &TextLocale,
        bitmap: &mut Bitmap32,
    ) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let rect = inset(rect, 0, 2);
        let fill = Rect::new(
            rect.top.max(clip.top),
            rect.left.max(clip.left),
            rect.bottom.min(clip.bottom),
            rect.right.min(clip.right),
        );
        fill_rect(bitmap, row.color, fill, ROW_TRANSPARENCY);
        let mut x = rect.left;
        for column in &self.columns {
            if column.cell == 0 && row.in_team && !row.team {
                x += column.advance;
                continue;
            }
            let indent = if column.name && row.in_team {
                self.in_team_indent
            } else {
                0
            };
            let cell = Rect::new(rect.top, x + indent, rect.bottom, x + column.width);
            let cell_clip = Rect::new(
                clip.top,
                x + indent,
                clip.bottom,
                (x + column.width).min(clip.right),
            );
            let text = row
                .cells
                .get(column.cell)
                .map_or(&[][..], |t| t.0.as_slice());
            font::draw_string(
                bitmap,
                cell,
                TextClip::Rect(cell_clip),
                family,
                if column.name && row.team {
                    font::STYLE_ITALIC
                } else {
                    font::STYLE_PLAIN
                },
                text,
                WHITE,
                if column.name {
                    NAME_FLAGS
                } else {
                    NUMBER_FLAGS
                },
                0,
                locale,
            );
            x += column.advance;
        }
    }
}

#[derive(Clone)]
pub struct CarnageText {
    rank: Vec<Vec<u8>>,
    perm: Vec<Vec<u8>>,
    graphs: Vec<Vec<u8>>,
    family: Option<Arc<FontFamily>>,
    locale: Arc<TextLocale>,
}

fn strings(res: &Resources, id: FourCC) -> Vec<Vec<u8>> {
    res.tag(STRINGS, id)
        .map(|d| string_list_lines(&d))
        .unwrap_or_default()
}

impl CarnageText {
    pub fn new(res: &Resources) -> Self {
        Self {
            rank: strings(res, RANK_STRINGS),
            perm: strings(res, PERM_STRINGS),
            graphs: strings(res, GRAPH_TITLES),
            family: res.font(0).cloned(),
            locale: Arc::new(res.locale().clone()),
        }
    }

    fn width(&self, text: &[u8]) -> i32 {
        self.family.as_ref().map_or(0, |f| {
            string_width(f, font::STYLE_PLAIN, text, &self.locale)
        })
    }

    fn rank_string(&self, index: usize) -> Vec<u8> {
        self.rank.get(index).cloned().unwrap_or_default()
    }

    fn perm_string(&self, index: usize) -> Vec<u8> {
        self.perm.get(index).cloned().unwrap_or_default()
    }

    fn first_column(&self, game_type: i16) -> (Vec<u8>, i32) {
        let title = match game_type {
            0 => self.rank_string(5),
            3 => self.rank_string(10),
            4 => self.rank_string(9),
            9 | 14 | 15 => self.rank_string(8),
            _ => Vec::new(),
        };
        let width = match game_type {
            0 => 0x32,
            1 | 2 => 0xd,
            3 | 4 => 0x5a,
            5..=15 => {
                let bit = 1u32 << game_type;
                if bit & 0x1d80 != 0 {
                    0x19
                } else if bit & 0x2060 != 0 {
                    0xd
                } else if bit & 0xc200 != 0 {
                    0x32
                } else {
                    0
                }
            }
            _ => 0,
        };
        (title, width)
    }

    fn rank_columns(&self, pages: &FilmPages, bounds: Rect) -> Vec<Column> {
        let mask = pages.rank_columns;
        let has = |bit: u8| mask & (1 << bit) != 0;
        let (first_title, first_width) = self.first_column(pages.game_type);
        let widths: Vec<i32> = (1..7)
            .map(|i| self.width(&self.rank_string(i)) + GAP)
            .collect();
        let mut name = bounds.width() - 0x14 - first_width;
        for bit in 2u8..8 {
            if has(bit) {
                name -= widths[usize::from(bit) - 2] + GAP;
            }
        }
        let name = name.max(0x26);
        (0u8..8)
            .filter(|&bit| has(bit))
            .map(|bit| {
                let (heading, width) = match bit {
                    0 => (first_title.clone(), first_width),
                    1 => (self.rank_string(0), name),
                    _ => (
                        self.rank_string(usize::from(bit) - 1),
                        widths[usize::from(bit) - 2],
                    ),
                };
                Column {
                    cell: usize::from(bit),
                    heading,
                    width,
                    advance: width + GAP,
                    name: bit == 1,
                }
            })
            .collect()
    }

    fn perm_columns(&self, rows: &[RankRow], bounds: Rect) -> Vec<Column> {
        let widest = |cell: usize, string: usize| {
            rows.iter()
                .filter_map(|r| r.cells.get(cell))
                .map(|t| self.width(&t.0))
                .fold(self.width(&self.perm_string(string)), i32::max)
                + GAP
        };
        let places: Vec<i32> = (0..3).map(|i| widest(2 + i, 5 + i)).collect();
        let (kills, damage, losses) = (widest(5, 2), widest(6, 3), widest(7, 4));
        let name = (bounds.width() - 0x5a - kills - damage - losses - places.iter().sum::<i32>())
            .max(0x5a);
        let column = |cell, string, width, advance, name| Column {
            cell,
            heading: self.perm_string(string),
            width,
            advance,
            name,
        };
        let mut columns = vec![
            column(0, 1, 0x28, 0x2d, false),
            column(1, 0, name, name + GAP, true),
        ];
        for (i, &w) in places.iter().enumerate() {
            columns.push(column(2 + i, 5 + i, w, w + GAP, false));
        }
        columns.push(column(5, 2, kills, kills + GAP, false));
        columns.push(column(6, 3, damage, damage + GAP, false));
        columns.push(column(7, 4, losses, losses + GAP, false));
        columns
    }

    pub fn stats_rows(&self, pages: &FilmPages, page: i16, bounds: Rect) -> StatsRows {
        match page {
            0 => StatsRows {
                rows: pages.rank.clone(),
                columns: self.rank_columns(pages, bounds),
                in_team_indent: 10,
            },
            _ => {
                let rows = pages.perm.clone().unwrap_or_default();
                StatsRows {
                    columns: self.perm_columns(&rows, bounds),
                    rows,
                    in_team_indent: 0,
                }
            }
        }
    }
}

pub struct CarnageHeader {
    text: CarnageText,
    page: Page,
    columns: Vec<Column>,
}

impl CarnageHeader {
    pub fn new(res: &Resources) -> Self {
        Self {
            text: CarnageText::new(res),
            page: Page::Stats(0),
            columns: Vec::new(),
        }
    }

    pub fn set_page(&mut self, item: &mut ItemState, page: Page, rows: Option<&StatsRows>) {
        self.page = page;
        self.columns = rows.map(|r| r.columns.clone()).unwrap_or_default();
        item.dirty = true;
    }
}

impl ControlProc for CarnageHeader {
    fn draw(&self, item: &ItemState, _ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let b = item.rect;
        fill_rect(bitmap, [0; 3], b, HEADER_TRANSPARENCY);
        let Some(family) = &self.text.family else {
            return;
        };
        let locale = &self.text.locale;
        match self.page {
            Page::Stats(_) => {
                let mut x = b.left;
                for column in &self.columns {
                    let rect = Rect::new(b.top, x, b.bottom, x + column.width);
                    let clip = Rect::new(b.top, x, b.bottom, (x + column.width).min(b.right));
                    font::draw_string(
                        bitmap,
                        rect,
                        TextClip::Rect(clip),
                        family,
                        font::STYLE_PLAIN,
                        &column.heading,
                        WHITE,
                        if column.name {
                            NAME_FLAGS
                        } else {
                            NUMBER_FLAGS
                        },
                        0,
                        locale,
                    );
                    x += column.advance;
                }
            }
            Page::Graph(graph) => {
                let title = usize::try_from(graph)
                    .ok()
                    .and_then(|g| self.text.graphs.get(g))
                    .map_or(&[][..], Vec::as_slice);
                font::draw_string(
                    bitmap,
                    b,
                    TextClip::None,
                    family,
                    font::STYLE_BOLD,
                    title,
                    WHITE,
                    font::text_flags::CENTER | font::text_flags::MIDDLE,
                    0,
                    locale,
                );
            }
        }
    }
}

#[derive(Default)]
pub struct CarnageGraph {
    graph: Option<Graph>,
    kind: i16,
}

impl CarnageGraph {
    pub fn set_graph(&mut self, item: &mut ItemState, kind: i16, graph: Option<Graph>) {
        self.kind = kind;
        self.graph = graph;
        item.dirty = true;
    }
}

fn pixel(color: [u16; 3]) -> u32 {
    0xff00_0000
        | u32::from(color[0] >> 8) << 16
        | u32::from(color[1] >> 8) << 8
        | u32::from(color[2] >> 8)
}

fn draw_line(bitmap: &mut Bitmap32, color: [u16; 3], clip: Rect, a: (i16, i16), b: (i16, i16)) {
    let (top, bottom) = if b.1 < a.1 { (b, a) } else { (a, b) };
    let right = (clip.right as i16).min(bitmap.width as i16);
    let lower = (clip.bottom as i16).min(bitmap.height as i16);
    let (left, upper) = (clip.left.max(0) as i16, clip.top.max(0) as i16);
    let (mut x, mut y) = top;
    let (x1, y1) = bottom;
    let dx = x1.wrapping_sub(x);
    let dy = y1.wrapping_sub(y);
    let (ax, ay) = (dx.wrapping_abs(), dy.wrapping_abs());
    let (ax2, ay2) = (ax.wrapping_mul(2), ay.wrapping_mul(2));
    let sx = if dx == 0 { 0 } else { dx >> 15 | 1 };
    let sy = if dy == 0 { 0 } else { dy >> 15 | 1 };
    let mut d = if ay2 < ax2 {
        ay2.wrapping_sub(ax2 >> 1)
    } else {
        ax2.wrapping_sub(ay2 >> 1)
    };
    let value = pixel(color);
    loop {
        if x < right && left <= x && y < lower && upper <= y {
            bitmap.set(i32::from(x), i32::from(y), value);
        }
        if ay2 < ax2 {
            if x == x1 {
                return;
            }
            if d >= 0 {
                y = y.wrapping_add(sy);
                d = d.wrapping_sub(ax2);
            }
            x = x.wrapping_add(sx);
            d = d.wrapping_add(ay2);
        } else {
            if y == y1 {
                return;
            }
            if d >= 0 {
                x = x.wrapping_add(sx);
                d = d.wrapping_sub(ay2);
            }
            y = y.wrapping_add(sy);
            d = d.wrapping_add(ax2);
        }
    }
}

impl ControlProc for CarnageGraph {
    fn draw(&self, item: &ItemState, _ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let b = item.rect;
        fill_rect(bitmap, [0; 3], b, GRAPH_TRANSPARENCY);
        let Some(graph) = self.graph.as_ref().filter(|g| g.count >= 1) else {
            return;
        };
        let area = Rect::new(b.top + GRAPH_TOP, b.left, b.bottom, b.right);
        let ticks = graph
            .count
            .wrapping_mul(graph.period)
            .wrapping_add(graph.tick);
        if graph.lines.is_empty() {
            return;
        }
        let (mut low, mut high) = (i32::MAX, i32::MIN);
        let mut any = false;
        for line in &graph.lines {
            let mut sum = 0i32;
            for &v in line.values.iter().take(graph.count as usize) {
                any = true;
                sum = sum.wrapping_add(i32::from(v));
                high = high.max(sum);
                match self.kind {
                    0 => low = low.min(sum),
                    1 | 2 | 4 => low = 1,
                    _ => {}
                }
            }
        }
        if !any || low == i32::MAX {
            return;
        }
        let base = i64::from(low) - 1;
        let high = i64::from(high) + 1;
        let x_scale = if ticks != 0 {
            (area.right - area.left) as f32 / ticks as f32
        } else {
            0.0
        };
        let y_scale = if high != base {
            (area.bottom - area.top) as f32 / (high - base) as f32
        } else {
            0.0
        };
        let mut zero = area.bottom;
        if base < 0 {
            zero = (area.bottom as f32 - (1 - i64::from(low)) as f32 * y_scale) as i32;
            draw_line(
                bitmap,
                [0; 3],
                area,
                (area.left as i16, zero as i16),
                (area.right as i16, zero as i16),
            );
        }
        for line in &graph.lines {
            let mut from = (area.left as i16, zero as i16);
            let mut sum = 0i32;
            for (i, &v) in line.values.iter().take(graph.count as usize).enumerate() {
                let x = ((graph.period.wrapping_mul(i as i32)) as f32 * x_scale + area.left as f32)
                    as i32;
                sum = sum.wrapping_add(i32::from(v));
                let y = (area.bottom as f32 - (i64::from(sum) - base) as f32 * y_scale) as i32;
                let to = (x as i16, y as i16);
                draw_line(bitmap, line.color, area, from, to);
                from = to;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_the_engines_bresenham_from_its_upper_end() {
        let mut b = Bitmap32::new(8, 8, 0);
        let clip = Rect::new(0, 0, 8, 8);
        draw_line(&mut b, [0xffff; 3], clip, (6, 1), (0, 4));
        let lit: Vec<(i32, i32)> = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter(|&(x, y)| b.get(x, y) != 0)
            .collect();
        assert_eq!(
            lit,
            [(6, 1), (4, 2), (5, 2), (2, 3), (3, 3), (0, 4), (1, 4)]
        );
        let mut c = Bitmap32::new(4, 4, 0);
        draw_line(&mut c, [0xffff; 3], Rect::new(0, 0, 2, 4), (0, 0), (0, 3));
        assert_eq!((0..4).filter(|&y| c.get(0, y) != 0).count(), 2, "clipped");
    }

    #[test]
    fn graph_draw_guards_empty_values_and_invalid_kind() {
        use mandrake_session::GraphLine;
        let item = ItemState {
            id: 1,
            kind: mandrake_common::FourCC(*b"cgrf"),
            rect: Rect::new(0, 0, 40, 40),
            flags: 0,
            pressed: false,
            dirty: false,
        };
        let res = crate::resources::Resources::from_source(|_, _| None);
        let ctx = DrawContext {
            res: &res,
            now_tick: 0,
            focused: false,
        };
        let mut bmp = Bitmap32::new(40, 40, 0);
        let empty = CarnageGraph {
            kind: 0,
            graph: Some(Graph {
                count: 5,
                period: 10,
                tick: 0,
                lines: vec![GraphLine {
                    color: [0xffff; 3],
                    values: Vec::new(),
                }],
            }),
        };
        empty.draw(&item, &ctx, &mut bmp);

        let unknown_kind = CarnageGraph {
            kind: 3,
            graph: Some(Graph {
                count: 2,
                period: 10,
                tick: 0,
                lines: vec![GraphLine {
                    color: [0xffff; 3],
                    values: vec![1, 2],
                }],
            }),
        };
        unknown_kind.draw(&item, &ctx, &mut bmp);
    }
}
