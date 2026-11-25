use std::collections::VecDeque;
use std::sync::Arc;

use mandrake_session::UiKey;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, text_flags};
use mandrake_tag::{DialogItem, Rect};
use mandrake_view::interface_bitmap::{
    InterfaceArt, blend_level, draw_bitmap_from_sequence, palette_pixel, placement,
    tile_and_bevel_rectangle,
};

use super::lobby_rows::{self, ChatRow, PlayerRow};
use super::mpll::{self, MissingPluginRow};
use super::{Click, ControlProc, DrawContext, ItemState, Key};
use crate::resources::{BAIE, Resources};

pub mod list_flags {
    pub const SCROLLABLE: u16 = 0x01;
    pub const FOCUSABLE: u16 = 0x02;
    pub const KEEP_SELECTION: u16 = 0x08;
    pub const FRAME: u16 = 0x20;
    pub const TILED_TRACK: u16 = 0x40;
    pub const TOGGLE: u16 = 0x80;
    pub const FOCUS_RING: u16 = 0x100;
    pub const SELECT_DISABLED: u16 = 0x400;
    pub const ROW_STYLE: u16 = 0x800;
    pub const NO_ROW_CLICKS: u16 = 0x1000;
    pub const CLICK_UNSELECTS: u16 = 0x4000;
    pub const NO_SCROLLBAR: u16 = 0x8000;
}
use list_flags as lf;

const STRING_ROW_MARGIN: i32 = 0x14;
const GUTTER_ACROSS: i32 = 1;
const GUTTER_DOWN: i32 = 2;
const ARROW_RATE: i32 = 4;
const PAGE_PART: i32 = 4;
const BAR_WIDTH: i32 = 16;
const ARROW_HEIGHT: i32 = 0xe;
const BAR_SEQUENCE: i32 = 0;
const TRACK_SEQUENCE: i32 = 4;
const SIZED_THUMB_FRAMES: i32 = 7;
const TYPE_AHEAD_TICKS: u64 = 30;
const CHAT_ROWS: usize = 0x400;
const PLUGIN_NAME_BYTES: usize = 31;
const MARK_OPEN: u8 = 0xc3;
const MARK_LOCKED: u8 = 0xa5;
const ORDER_COLORS: [RgbColor; 7] = [
    [0, 0, 0],
    [17664, 38144, 58624],
    [44288, 3584, 58880],
    [54528, 16384, 25856],
    [16384, 54528, 20480],
    [49152, 54528, 16384],
    [54528, 34048, 16384],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Strings,
    Entries,
    Plugins,
    Tags,
    Players,
    Chat,
    Carnage,
    MissingPlugins,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Track {
    None,
    ArrowUp,
    ArrowDown,
    Thumb,
    PageUp,
    PageDown,
    Unselected,
    Row,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntryLook {
    pub listed: bool,
    pub order: i16,
}

#[derive(Debug, Clone)]
struct Row {
    name: Vec<u8>,
    text: Vec<u8>,
    value: i32,
    look: EntryLook,
    top: i32,
    height: i32,
    selected: bool,
    enabled: bool,
    visible: bool,
}

pub struct List {
    kind: Kind,
    bounds: Rect,
    flags: u16,
    rows: Vec<Row>,
    first: usize,
    within: i32,
    tracking: Track,
    grab: i32,
    gutters: (i32, i32),
    carnage: Option<super::carnage::StatsRows>,
    players: Vec<PlayerRow>,
    chat: VecDeque<ChatRow>,
    chat_base: usize,
    missing: Vec<MissingPluginRow>,
    sba: Option<Arc<InterfaceArt>>,
    nepi: Option<Arc<InterfaceArt>>,
    last_row: Option<usize>,
    double_click_item: Option<i16>,
    line_height: i32,
    locale: std::sync::Arc<font::TextLocale>,
    family: Option<Arc<FontFamily>>,
    baie: Option<Arc<InterfaceArt>>,
    colors: [RgbColor; 10],
    typed: Vec<u8>,
    typed_tick: u64,
    wants_focus: bool,
    press_request: Option<i16>,
}

pub(super) fn fill_rect(bitmap: &mut Bitmap32, color: RgbColor, rect: Rect, transparency: u32) {
    let [r, g, b] = color.map(|v| (v >> 8) as u8);
    let c = palette_pixel([r, g, b, 0xff]);
    let level = transparency >> 12 & 0xf;
    for y in rect.top.max(0)..rect.bottom.min(bitmap.height) {
        for x in rect.left.max(0)..rect.right.min(bitmap.width) {
            let v = if level == 0 {
                c
            } else {
                blend_level(bitmap.get(x, y), c, level)
            };
            bitmap.set(x, y, v);
        }
    }
}

pub(super) fn frame_rect(bitmap: &mut Bitmap32, color: RgbColor, rect: Rect, transparency: u32) {
    let (t, l, b, r) = (rect.top, rect.left, rect.bottom - 1, rect.right - 1);
    let one = |top, left, bottom, right| Rect::new(top, left, bottom, right);
    fill_rect(bitmap, color, one(t, l, t + 1, r + 1), transparency);
    fill_rect(bitmap, color, one(t, r, b + 1, r + 1), transparency);
    fill_rect(bitmap, color, one(b, l, b + 1, r + 1), transparency);
    fill_rect(bitmap, color, one(t, l, b + 1, l + 1), transparency);
}

pub(super) const fn inset(r: Rect, across: i32, down: i32) -> Rect {
    Rect::new(
        r.top + down,
        r.left + across,
        r.bottom - down,
        r.right - across,
    )
}

fn starts_with_fuzzy(name: &[u8], typed: &[u8]) -> bool {
    let skip = |s: &[u8]| {
        s.iter()
            .position(|&c| !(1..=0x20).contains(&c))
            .unwrap_or(s.len())
    };
    let name = &name[skip(name)..];
    let typed = &typed[typed.iter().position(|&c| c > 0x20).unwrap_or(typed.len())..];
    let mut at = 0;
    for &want in typed {
        while name.get(at) == Some(&b'|') && name.get(at + 1).is_some_and(|&c| c != b'\r') {
            at += 2;
        }
        if !name.get(at).is_some_and(|c| c.eq_ignore_ascii_case(&want)) {
            return false;
        }
        at += 1;
    }
    true
}

impl List {
    pub fn new(item: &DialogItem, res: &mut Resources) -> Self {
        let kind = match &item.kind.0 {
            b"slst" => Kind::Strings,
            b"lstm" => Kind::Entries,
            b"lstp" => Kind::Plugins,
            b"lstg" => Kind::Tags,
            b"plli" => Kind::Players,
            b"chat" => Kind::Chat,
            b"cstt" => Kind::Carnage,
            b"mpll" => Kind::MissingPlugins,
            _ => Kind::Plain,
        };
        let flags = match kind {
            Kind::Plain => lf::FOCUSABLE,
            Kind::Strings | Kind::Entries | Kind::Plugins => {
                lf::FOCUSABLE | lf::KEEP_SELECTION | lf::FRAME | lf::TILED_TRACK
            }
            Kind::Tags => lf::FOCUSABLE | lf::FRAME | lf::TILED_TRACK,
            Kind::Players => lf::TILED_TRACK | lf::CLICK_UNSELECTS | lf::SELECT_DISABLED,
            Kind::Carnage => lf::TILED_TRACK,
            Kind::Chat => lf::TILED_TRACK,
            Kind::MissingPlugins => lf::FRAME | lf::TILED_TRACK | lf::SELECT_DISABLED,
        };
        let sba = if kind == Kind::MissingPlugins {
            res.art(mandrake_common::FourCC(*b"!sba"))
        } else {
            None
        };
        let family = res.font(0).cloned();
        let (line_height, gutters) = match kind {
            Kind::Carnage | Kind::Players => (0x15, (2, 2)),
            Kind::Chat => (
                family
                    .as_ref()
                    .map_or(0, |f| f.font(font::STYLE_PLAIN).line_height())
                    + 2,
                (1, 4),
            ),
            Kind::MissingPlugins => (mpll::line_height(sba.as_ref()), (2, 2)),
            _ => (
                family
                    .as_ref()
                    .map_or(0, |f| f.font(font::STYLE_PLAIN).line_height()),
                (GUTTER_ACROSS, GUTTER_DOWN),
            ),
        };
        Self {
            kind,
            gutters,
            carnage: None,
            players: Vec::new(),
            chat: VecDeque::new(),
            chat_base: 0,
            missing: Vec::new(),
            sba,
            nepi: None,
            bounds: item.rect,
            flags,
            rows: Vec::new(),
            first: 0,
            within: 0,
            tracking: Track::None,
            grab: 0,
            last_row: None,
            double_click_item: None,
            line_height,
            locale: std::sync::Arc::new(res.locale().clone()),
            family,
            baie: res.art(BAIE),
            colors: std::array::from_fn(|i| res.dialog_color(i as i32)),
            typed: Vec::new(),
            typed_tick: 0,
            wants_focus: false,
            press_request: None,
        }
    }

    pub fn set_flag(&mut self, flag: u16, on: bool) {
        if on {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }

    pub const fn set_double_click_item(&mut self, item: i16) {
        self.double_click_item = Some(item);
    }

    pub fn set_carnage_rows(&mut self, rows: super::carnage::StatsRows) {
        self.clear();
        for index in 0..rows.len() {
            self.add_row(b"", index as i32, true);
        }
        self.carnage = Some(rows);
    }

    pub fn set_player_rows(&mut self, rows: Vec<PlayerRow>, nepi: Option<Arc<InterfaceArt>>) {
        let chosen = self.selected_value();
        let (first, within) = (self.first, self.within);
        self.rows.clear();
        for row in &rows {
            self.add_row(&row.name, row.id, true);
        }
        self.players = rows;
        self.nepi = nepi;
        if let Some(id) = chosen {
            self.select_value(id);
        }
        if first < self.rows.len() {
            self.first = first;
            self.within = within;
        }
        self.layout();
    }

    pub fn set_missing_plugin_rows(&mut self, rows: Vec<MissingPluginRow>) {
        self.clear();
        for (index, row) in rows.iter().enumerate() {
            self.add_row(&row.name, index as i32, true);
        }
        self.missing = rows;
    }

    fn row_at_point(&self, point: (i32, i32)) -> Option<(usize, i32, i32)> {
        let b = self.bounds;
        let bar = Rect::new(b.top, b.right - BAR_WIDTH, b.bottom, b.right);
        if self.rows.is_empty() || bar.contains(point.0, point.1) {
            return None;
        }
        let inner = inset(b, self.gutters.0, self.gutters.1);
        let x = point.0 - inner.left;
        let y = point.1 - inner.top + self.scroll();
        let width = inner.width() - BAR_WIDTH;
        self.rows
            .iter()
            .position(|r| {
                r.visible && (0..width).contains(&x) && (r.top..r.top + r.height).contains(&y)
            })
            .map(|row| (row, y - self.rows[row].top, width - x))
    }

    pub fn player_row(&self, id: i32) -> Option<&PlayerRow> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn clear_selection(&mut self) {
        self.rows.iter_mut().for_each(|r| r.selected = false);
    }

    pub fn add_chat(&mut self, name: &[u8], color: RgbColor, text: &[u8]) {
        self.add_chat_of_kind(0, name, color, text);
    }

    pub fn add_chat_of_kind(&mut self, kind: i16, name: &[u8], color: RgbColor, text: &[u8]) {
        let Some(family) = self.family.clone() else {
            return;
        };
        let width = self.bounds.width() - lobby_rows::CHAT_WRAP_MARGIN;
        for piece in lobby_rows::wrap_chat(&family, &self.locale, text, width) {
            let value = (self.chat_base + self.chat.len()) as i32;
            self.chat.push_back(ChatRow {
                kind,
                name: name.to_vec(),
                color,
                text: piece,
            });
            self.add_row(b"", value, true);
        }
        let mut trimmed = false;
        while self.rows.len() > CHAT_ROWS {
            self.rows.remove(0);
            trimmed = true;
        }
        while self.chat.len() > CHAT_ROWS {
            self.chat.pop_front();
            self.chat_base += 1;
        }
        if trimmed {
            self.layout();
        }
        let (view, end) = (self.view_height(), self.content_end());
        if end > view {
            self.set_scroll(end - view);
        }
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.chat.clear();
        self.chat_base = 0;
        self.first = 0;
        self.within = 0;
        self.last_row = None;
        self.flags &= !lf::SCROLLABLE;
    }

    pub fn add_row(&mut self, name: &[u8], value: i32, enabled: bool) -> usize {
        let name = match self.kind {
            Kind::Plugins => &name[..name.len().min(PLUGIN_NAME_BYTES)],
            _ => name,
        };
        let height = match self.kind {
            Kind::Strings => self.string_row_height(name),
            _ => self.line_height,
        };
        let top = self
            .rows
            .last()
            .map_or(0, |r| r.top + if r.visible { r.height } else { 0 });
        self.rows.push(Row {
            name: name.to_vec(),
            text: name.to_vec(),
            value,
            look: EntryLook::default(),
            top,
            height,
            selected: false,
            enabled,
            visible: true,
        });
        if self.view_height() < top + height {
            self.flags |= lf::SCROLLABLE;
        } else {
            self.first = 0;
            self.within = 0;
            self.flags &= !lf::SCROLLABLE;
        }
        self.rows.len() - 1
    }

    fn string_row_height(&self, text: &[u8]) -> i32 {
        let Some(family) = &self.family else {
            return self.line_height;
        };
        let width = self.bounds.width() - STRING_ROW_MARGIN;
        let (mut at, mut lines, mut style) = (0, 1, font::STYLE_PLAIN);
        loop {
            let line = font::wrap_string(
                family,
                style,
                &text[at.min(text.len())..],
                width,
                font::TEXT_WRAP,
                &self.locale,
            );
            if line.end == font::LineEnd::End || line.count == 0 {
                break;
            }
            style = line.style;
            at += line.count;
            lines += 1;
        }
        let pitch = family.font(font::STYLE_PLAIN).line_pitch();
        self.line_height + (lines - 1) * pitch
    }

    pub fn add_entry(&mut self, name: &[u8], value: i32, enabled: bool, look: EntryLook) -> usize {
        let row = self.add_row(name, value, enabled);
        let mark = if enabled { MARK_OPEN } else { MARK_LOCKED };
        let r = &mut self.rows[row];
        r.look = look;
        r.text = [&[mark, b' '], name].concat();
        row
    }

    pub fn add_unmarked_entry(&mut self, name: &[u8], value: i32, look: EntryLook) -> usize {
        let row = self.add_row(name, value, true);
        self.rows[row].look = look;
        row
    }

    pub const fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn row_value(&self, row: usize) -> Option<i32> {
        self.rows.get(row).map(|r| r.value)
    }

    pub fn row_text(&self, row: usize) -> Option<&[u8]> {
        self.rows.get(row).map(|r| r.text.as_slice())
    }

    pub fn row_enabled(&self, row: usize) -> bool {
        self.rows.get(row).is_some_and(|r| r.enabled)
    }

    pub fn row_visible(&self, row: usize) -> bool {
        self.rows.get(row).is_some_and(|r| r.visible)
    }

    pub fn selected(&self) -> Option<usize> {
        self.rows.iter().position(|r| r.selected)
    }

    pub fn selected_value(&self) -> Option<i32> {
        self.selected().and_then(|row| self.row_value(row))
    }

    pub fn select(&mut self, row: usize) {
        if row < self.rows.len() {
            self.set_select(row);
        }
    }

    pub fn select_value(&mut self, value: i32) {
        if let Some(row) = self.rows.iter().position(|r| r.value == value) {
            self.select(row);
        }
    }

    pub fn show_selected(&mut self) {
        if let Some(row) = self.selected() {
            self.make_row_visible(row, true);
        }
    }

    pub fn remove_value(&mut self, value: i32) {
        if let Some(row) = self.rows.iter().position(|r| r.value == value) {
            self.rows.remove(row);
            self.last_row = None;
            self.layout();
            let (view, end) = (self.view_height(), self.content_end());
            self.set_scroll_clamped(self.scroll(), (end - view).max(0));
        }
    }

    pub fn set_row_enabled(&mut self, row: usize, enabled: bool) {
        if let Some(r) = self.rows.get_mut(row) {
            r.enabled = enabled;
        }
    }

    pub fn select_first_enabled(&mut self) {
        if let Some(row) = self.rows.iter().position(|r| r.enabled) {
            self.select(row);
        }
    }

    pub fn set_row_visible(&mut self, row: usize, visible: bool) {
        if let Some(r) = self.rows.get_mut(row) {
            r.visible = visible;
            self.layout();
            let (view, end) = (self.view_height(), self.content_end());
            if end > view {
                self.set_scroll_clamped(self.scroll(), end - view);
            }
        }
    }

    pub fn sort_by(&mut self, mut key: impl FnMut(i32) -> usize) {
        self.rows.sort_by_key(|r| key(r.value));
        self.layout();
    }

    const fn view_height(&self) -> i32 {
        self.bounds.height() - self.gutters.1 * 2
    }

    fn content_end(&self) -> i32 {
        self.rows.last().map_or(0, |r| r.top + r.height)
    }

    fn content_height(&self) -> i32 {
        self.rows
            .iter()
            .filter(|r| r.visible)
            .map(|r| r.height)
            .sum()
    }

    fn scroll(&self) -> i32 {
        self.rows.get(self.first).map_or(0, |r| r.top) + self.within
    }

    fn layout(&mut self) {
        let mut top = 0;
        for r in &mut self.rows {
            r.top = top;
            if r.visible {
                top += r.height;
            }
        }
        if self.view_height() < self.content_end() {
            self.flags |= lf::SCROLLABLE;
        } else {
            self.first = 0;
            self.within = 0;
            self.flags &= !lf::SCROLLABLE;
        }
    }

    fn set_scroll(&mut self, scroll: i32) {
        if let Some(row) = self
            .rows
            .iter()
            .position(|r| r.visible && r.top + r.height > scroll)
        {
            self.first = row;
            self.within = scroll - self.rows[row].top;
        }
    }

    fn set_scroll_clamped(&mut self, scroll: i32, max: i32) {
        let mut to = scroll.min(max);
        if scroll < 0 {
            to = 0;
        }
        self.set_scroll(to);
    }

    fn move_by(&mut self, delta: i32) {
        let (view, end) = (self.view_height(), self.content_end());
        if view < end {
            self.set_scroll_clamped(self.scroll() + delta, end - view);
        }
    }

    fn make_row_visible(&mut self, row: usize, centre: bool) {
        let Some(r) = self.rows.get(row) else { return };
        let (top, bottom) = (r.top, r.top + r.height);
        let (view_top, view) = (self.scroll(), self.view_height());
        let half = view / 2;
        let delta = if top < view_top {
            top - view_top - if centre { half } else { 0 }
        } else if bottom > view_top + view {
            bottom - (view_top + view) + if centre { half } else { 0 }
        } else {
            return;
        };
        if delta == 0 {
            return;
        }
        let scroll = view_top + delta;
        let max = self.content_end() - view;
        let to = if scroll < 0 { 0 } else { scroll.min(max) };
        self.set_scroll(to);
    }

    fn set_select(&mut self, row: usize) -> bool {
        if self.flags & lf::SELECT_DISABLED == 0 && !self.rows[row].enabled {
            if self.flags & lf::KEEP_SELECTION == 0 {
                self.rows.iter_mut().for_each(|r| r.selected = false);
            }
            return false;
        }
        self.rows.iter_mut().for_each(|r| r.selected = false);
        self.rows[row].selected = true;
        true
    }

    const fn has_bar(&self) -> bool {
        self.flags & lf::NO_SCROLLBAR == 0
    }

    const fn bar_rect(&self) -> Rect {
        let b = self.bounds;
        Rect::new(b.top, b.right - BAR_WIDTH, b.bottom, b.right)
    }

    fn sized_thumb(&self) -> bool {
        self.baie
            .as_ref()
            .is_some_and(|a| a.frames(BAR_SEQUENCE) >= SIZED_THUMB_FRAMES)
    }

    fn thumb_rect(&self, bar: Rect) -> Option<Rect> {
        let sized = self.sized_thumb();
        let track = bar.height() - if sized { ARROW_HEIGHT * 2 } else { 0x2a };
        let (content, view) = (self.content_height(), self.view_height());
        if content <= view {
            return None;
        }
        let scroll = self.scroll();
        if sized {
            let height = (view * track / content).max(ARROW_HEIGHT);
            let mut at = scroll * track / content;
            if track < at + height {
                at = track - height;
            }
            let top = at + bar.top + ARROW_HEIGHT;
            Some(Rect::new(top, bar.left, top + height, bar.right))
        } else {
            let at = scroll * track / (content - view) + bar.top;
            Some(Rect::new(
                at + ARROW_HEIGHT,
                bar.left,
                at + ARROW_HEIGHT * 2,
                bar.right,
            ))
        }
    }

    fn click_bar(&mut self, point: (i32, i32)) -> bool {
        if !self.has_bar() || self.flags & lf::SCROLLABLE == 0 {
            return false;
        }
        let (x, y) = point;
        let bar = self.bar_rect();
        if !self.rows.is_empty() {
            if self.tracking == Track::Thumb {
                self.drag_thumb(bar, y);
            } else if bar.contains(x, y) {
                self.press_bar(bar, y);
                return true;
            }
        }
        !matches!(self.tracking, Track::None | Track::Unselected | Track::Row)
    }

    fn drag_thumb(&mut self, bar: Rect, y: i32) {
        let (content, view) = (self.content_height(), self.view_height());
        let scroll = if self.sized_thumb() {
            let (low, high) = (bar.top + ARROW_HEIGHT, bar.bottom - ARROW_HEIGHT);
            if high <= low {
                return;
            }
            let at = (y + self.grab).clamp(low, high);
            let scroll = (at - low) * content / (high - low);
            scroll.min(content - view)
        } else {
            let (low, high) = (bar.top + 0x15, bar.bottom - 0x15);
            if high <= low {
                return;
            }
            let at = y.clamp(low, high);
            (at - low) * (content - view) / (high - low)
        };
        self.set_scroll(scroll);
    }

    fn press_bar(&mut self, bar: Rect, y: i32) {
        let Some(thumb) = self.thumb_rect(bar) else {
            return;
        };
        let up_arrow_end = bar.top + ARROW_HEIGHT;
        let down_arrow_top = bar.bottom - ARROW_HEIGHT;
        let page = self.view_height() / PAGE_PART;
        let (part, delta) = if y < up_arrow_end {
            (Track::ArrowUp, -ARROW_RATE)
        } else if y < thumb.top {
            (Track::PageUp, -page)
        } else if y < thumb.bottom {
            (Track::Thumb, 0)
        } else if y < down_arrow_top {
            (Track::PageDown, page)
        } else {
            (Track::ArrowDown, ARROW_RATE)
        };
        if self.tracking != Track::None && self.tracking != part {
            return;
        }
        self.tracking = part;
        if part == Track::Thumb {
            self.grab = thumb.top - y;
        } else {
            self.move_by(delta);
        }
    }

    fn click_rows(&mut self, point: (i32, i32), double: bool) -> bool {
        if self.flags & lf::NO_ROW_CLICKS != 0 {
            return false;
        }
        let inner = inset(self.bounds, self.gutters.0, self.gutters.1);
        let x = point.0 - inner.left;
        let y = point.1 - inner.top + self.scroll();
        let right = inner.width() - if self.has_bar() { BAR_WIDTH } else { 0 };
        let row = self.rows.iter().position(|r| {
            r.visible && (0..right).contains(&x) && (r.top..r.top + r.height).contains(&y)
        });
        let Some(row) = row else {
            if self.flags & lf::KEEP_SELECTION == 0 {
                self.rows.iter_mut().for_each(|r| r.selected = false);
            }
            self.last_row = None;
            return false;
        };
        if self.tracking != Track::None {
            self.last_row = Some(row);
            return false;
        }
        let selected = self.rows[row].selected;
        let unselects = selected
            && (self.flags & lf::TOGGLE != 0 || (!double && self.flags & lf::CLICK_UNSELECTS != 0));
        if unselects {
            self.rows[row].selected = false;
            self.tracking = Track::Unselected;
            self.last_row = Some(row);
            return false;
        }
        let chosen = self.set_select(row);
        self.tracking = Track::Row;
        let again = chosen && double && self.last_row == Some(row);
        self.last_row = Some(row);
        again
    }

    fn draw_bar(&self, bitmap: &mut Bitmap32, on: bool) {
        let Some(art) = self.baie.clone() else { return };
        let bar = self.bar_rect();
        if self.flags & lf::TILED_TRACK != 0 {
            let view = if on { 0 } else { 2 };
            tile_and_bevel_rectangle(bitmap, &art, TRACK_SEQUENCE, view, bar, None, None);
        }
        let part = |held: Track| {
            if !on {
                2
            } else {
                i32::from(self.tracking == held)
            }
        };
        let part_at = |bitmap: &mut Bitmap32, view, frame, rect, flags| {
            draw_bitmap_from_sequence(bitmap, &art, BAR_SEQUENCE, view, frame, rect, flags, 0, 0);
        };
        part_at(bitmap, part(Track::ArrowUp), 0, bar, placement::TOP);
        part_at(bitmap, part(Track::ArrowDown), 1, bar, placement::BOTTOM);
        let Some(thumb) = self.thumb_rect(bar) else {
            return;
        };
        let view = part(Track::Thumb);
        if !self.sized_thumb() {
            part_at(bitmap, view, 2, thumb, placement::BOTTOM << 1);
            return;
        }
        let height_of = |frame| {
            art.bitmap(BAR_SEQUENCE, 0, frame % art.frames(BAR_SEQUENCE))
                .map_or(0, |(b, _)| b.height)
        };
        let (cap_top, cap_bottom, centre, tile) =
            (height_of(3), height_of(4), height_of(5), height_of(6));
        if thumb.height() > ARROW_HEIGHT && tile > 0 {
            let x0 = thumb.left.max(0);
            let x1 = thumb.right.min(bitmap.width);
            let row_w = (x1 - x0).max(0) as usize;
            let saved_rows: Vec<i32> = if row_w > 0 {
                [thumb.top, thumb.bottom - 1]
                    .into_iter()
                    .chain(thumb.bottom..(thumb.bottom + tile).min(bitmap.height))
                    .filter(|&y| y >= 0 && y < bitmap.height)
                    .collect()
            } else {
                Vec::new()
            };
            let mut saved_pixels = Vec::with_capacity(saved_rows.len() * row_w);
            for &y in &saved_rows {
                let start = (y * bitmap.width + x0) as usize;
                saved_pixels.extend_from_slice(&bitmap.pixels[start..start + row_w]);
            }
            let mut top = thumb.top;
            while top < thumb.bottom {
                let rect = Rect::new(top, thumb.left, top + tile, thumb.right);
                part_at(bitmap, view, 6, rect, placement::TOP);
                top += tile;
            }
            for (idx, &y) in saved_rows.iter().enumerate() {
                let start = (y * bitmap.width + x0) as usize;
                let src = idx * row_w;
                bitmap.pixels[start..start + row_w]
                    .copy_from_slice(&saved_pixels[src..src + row_w]);
            }
        }
        let top = thumb.top + (thumb.height() - centre) / 2;
        let middle = Rect::new(top, thumb.left, top + centre, thumb.right);
        part_at(bitmap, view, 5, middle, placement::TOP);
        let upper = Rect::new(thumb.top, thumb.left, thumb.top + cap_top, thumb.right);
        part_at(bitmap, view, 3, upper, placement::TOP);
        let lower = Rect::new(
            thumb.bottom - cap_bottom,
            thumb.left,
            thumb.bottom,
            thumb.right,
        );
        part_at(bitmap, view, 4, lower, placement::TOP);
    }

    fn draw_row(
        &self,
        row: &Row,
        rect: Rect,
        clip: Rect,
        enabled: bool,
        ctx: &DrawContext<'_>,
        bitmap: &mut Bitmap32,
    ) {
        let Some(family) = &self.family else { return };
        let c = &self.colors;
        if row.selected {
            let area = Rect::new(
                rect.top.max(clip.top),
                rect.left.max(clip.left),
                rect.bottom.min(clip.bottom),
                rect.right.min(clip.right),
            );
            fill_rect(bitmap, c[6], area, 0x4000);
        }
        let shadow = if row.selected { 0 } else { text_flags::SHADOW };
        let (color, flags) = match self.kind {
            Kind::Plain => return,
            Kind::Players | Kind::Chat => {
                let art = lobby_rows::RowArt {
                    family,
                    locale: ctx.res.locale(),
                    nepi: self.nepi.as_ref(),
                };
                let index = usize::try_from(row.value).unwrap_or(usize::MAX);
                if self.kind == Kind::Players {
                    if let Some(p) = self.players.iter().find(|p| p.id == row.value) {
                        lobby_rows::draw_player(p, rect, clip, enabled, &art, bitmap);
                    }
                } else if let Some(c) = index
                    .checked_sub(self.chat_base)
                    .and_then(|i| self.chat.get(i))
                {
                    lobby_rows::draw_chat(c, rect, clip, &art, bitmap);
                }
                return;
            }
            Kind::MissingPlugins => {
                let art = mpll::RowArt {
                    family,
                    locale: ctx.res.locale(),
                    sba: self.sba.as_ref(),
                    baie: self.baie.as_ref(),
                    colors: &self.colors,
                };
                let index = usize::try_from(row.value).unwrap_or(usize::MAX);
                if let Some(m) = self.missing.get(index) {
                    mpll::draw_row(m, rect, clip, row.selected, enabled, &art, bitmap);
                }
                return;
            }
            Kind::Carnage => {
                if let Some(rows) = &self.carnage {
                    let index = usize::try_from(row.value).unwrap_or(usize::MAX);
                    rows.draw_row(index, rect, clip, family, ctx.res.locale(), bitmap);
                }
                return;
            }
            Kind::Tags => {
                let color = if row.selected {
                    c[7]
                } else if enabled {
                    c[3]
                } else {
                    c[4]
                };
                (color, 0)
            }
            Kind::Entries => {
                let color = if row.selected {
                    c[7]
                } else if !row.look.listed {
                    c[5]
                } else if (100..700).contains(&row.look.order) {
                    ORDER_COLORS[(row.look.order / 100) as usize]
                } else {
                    c[3]
                };
                let color = if enabled {
                    color
                } else {
                    color.map(|v| ((u32::from(v) * 0xaaaa) >> 16) as u16)
                };
                (color, shadow)
            }
            Kind::Plugins => {
                let color = if !enabled {
                    c[4]
                } else if row.selected {
                    c[7]
                } else {
                    c[3]
                };
                (color, shadow)
            }
            Kind::Strings => {
                let color = if row.selected {
                    c[7]
                } else if enabled {
                    c[3]
                } else {
                    c[4]
                };
                let flags = if self.flags & lf::ROW_STYLE != 0 {
                    text_flags::WRAP
                } else {
                    shadow
                };
                (color, flags)
            }
        };
        font::draw_string(
            bitmap,
            rect,
            TextClip::Rect(clip),
            family,
            font::STYLE_PLAIN,
            &row.text,
            color,
            flags,
            0,
            ctx.res.locale(),
        );
    }

    fn find_typed(&mut self, ch: u8, now_tick: u64) -> Option<usize> {
        if now_tick >= self.typed_tick + TYPE_AHEAD_TICKS {
            self.typed.clear();
        }
        if self.typed.len() == 0x1f {
            return None;
        }
        self.typed_tick = now_tick;
        self.typed.push(ch);
        self.rows
            .iter()
            .position(|r| r.enabled && starts_with_fuzzy(&r.name, &self.typed))
    }

    fn usable(&self, row: usize) -> bool {
        self.rows[row].enabled && self.rows[row].visible
    }

    fn row_above(&self) -> usize {
        match self.selected() {
            Some(at) => (0..at).rev().find(|&r| self.usable(r)).unwrap_or(at),
            None => (1..self.rows.len()).find(|&r| self.usable(r)).unwrap_or(0),
        }
    }

    fn row_below(&self) -> usize {
        let last = self.rows.len() - 1;
        match self.selected() {
            Some(at) => (at + 1..=last).find(|&r| self.usable(r)).unwrap_or(at),
            None => (0..last).rev().find(|&r| self.usable(r)).unwrap_or(last),
        }
    }
}

impl ControlProc for List {
    fn click(&mut self, item: &mut ItemState, click: &Click) -> bool {
        item.dirty = true;
        if self.kind == Kind::MissingPlugins {
            let hit = self
                .row_at_point(click.point)
                .and_then(|(row, down, across)| {
                    let index = usize::try_from(self.rows[row].value).ok()?;
                    Some((index, down, across))
                });
            mpll::click(&mut self.missing, hit, click.held, self.baie.as_ref());
        }
        if !click.held {
            self.tracking = Track::None;
            return false;
        }
        let before = self.selected();
        self.wants_focus = self.flags & lf::FOCUSABLE != 0;
        if !self.click_bar(click.point) && self.click_rows(click.point, click.double) {
            self.press_request = self.double_click_item;
        }
        before != self.selected() || click.shift
    }

    fn key(&mut self, item: &mut ItemState, key: &Key) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        let to = match key.key {
            UiKey::Up => self.row_above(),
            UiKey::Down => self.row_below(),
            UiKey::Home => match (0..self.rows.len()).find(|&r| self.usable(r)) {
                Some(row) => row,
                None => return true,
            },
            UiKey::End => match (0..self.rows.len()).rev().find(|&r| self.usable(r)) {
                Some(row) => row,
                None => return false,
            },
            UiKey::PageUp | UiKey::PageDown => {
                let page = self.view_height() / PAGE_PART;
                self.move_by(if key.key == UiKey::PageUp {
                    -page
                } else {
                    page
                });
                item.dirty = true;
                return true;
            }
            UiKey::Space => match self.find_typed(b' ', key.now_tick) {
                Some(row) => row,
                None => return false,
            },
            UiKey::Char(c) if c.is_ascii() => match self.find_typed(c as u8, key.now_tick) {
                Some(row) => row,
                None => return false,
            },
            _ => return false,
        };
        self.set_select(to);
        self.make_row_visible(to, false);
        item.dirty = true;
        true
    }

    fn can_focus(&self, item: &ItemState) -> bool {
        item.live() && self.flags & lf::FOCUSABLE != 0
    }

    fn focus(&mut self, item: &mut ItemState, _focused: bool) -> bool {
        item.live() && self.flags & lf::FOCUSABLE != 0
    }

    fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.wants_focus)
    }

    fn take_press_request(&mut self) -> Option<i16> {
        self.press_request.take()
    }

    fn wheel(&mut self, item: &mut ItemState, delta: i32) -> bool {
        if delta == 0 {
            return false;
        }
        self.move_by(-delta * self.line_height);
        item.dirty = true;
        true
    }

    fn draw(&self, item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let b = self.bounds;
        if self.flags & lf::FRAME != 0 {
            frame_rect(bitmap, [0; 3], b, 0x8000);
            fill_rect(bitmap, [0; 3], inset(b, 1, 1), 0x5555);
        }
        if self.has_bar() {
            self.draw_bar(bitmap, self.flags & lf::SCROLLABLE != 0);
        }
        let view = self.view_height();
        let scroll = if self.kind == Kind::Chat && self.content_end() < view {
            self.content_end() - view
        } else {
            self.scroll()
        };
        let right_cut = if self.has_bar() { BAR_WIDTH } else { 0 };
        for row in self.rows.iter().filter(|r| r.visible) {
            if row.top + row.height <= scroll || row.top >= scroll + view {
                continue;
            }
            let (across, down) = self.gutters;
            let top = b.top + down + row.top - scroll;
            let rect = Rect::new(
                top,
                b.left + across,
                top + row.height,
                b.right - across - right_cut,
            );
            let clip = Rect::new(
                top.max(b.top + down),
                rect.left,
                (top + row.height).min(b.top + down + view),
                rect.right,
            );
            self.draw_row(row, rect, clip, row.enabled && item.enabled(), ctx, bitmap);
        }
        if ctx.focused && self.flags & lf::FOCUS_RING != 0 {
            let ring = inset(b, -4, -4);
            frame_rect(bitmap, self.colors[9], ring, 0x8000);
            frame_rect(bitmap, self.colors[9], inset(ring, 1, 1), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_common::FourCC;

    fn make_list(kind: &[u8; 4], rect: Rect) -> List {
        let mut res = Resources::from_source(|_, _| None);
        let item = DialogItem {
            kind: FourCC(*kind),
            field_04: 0,
            id: 1,
            fields_08: [0; 4],
            rect,
            name: Vec::new(),
        };
        List::new(&item, &mut res)
    }

    #[test]
    fn incremental_add_row_matches_full_layout() {
        let mut list = make_list(b"list", Rect::new(0, 0, 100, 100));
        list.line_height = 14;
        for i in 0..10 {
            list.add_row(b"item", i, true);
        }
        let incremental_tops: Vec<i32> = list.rows.iter().map(|r| r.top).collect();
        let incremental_flags = list.flags;
        list.layout();
        let layout_tops: Vec<i32> = list.rows.iter().map(|r| r.top).collect();
        assert_eq!(incremental_tops, layout_tops);
        assert_eq!(incremental_flags, list.flags);
    }

    #[test]
    fn drag_thumb_on_tiny_bar_does_not_panic() {
        let mut list = make_list(b"list", Rect::new(0, 0, 20, 100));
        list.line_height = 14;
        for i in 0..5 {
            list.add_row(b"item", i, true);
        }
        list.drag_thumb(Rect::new(0, 84, 20, 100), 10);
    }
}
