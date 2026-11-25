use std::sync::Arc;

use mandrake_tag::Rect;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, TextLocale, text_flags};
use mandrake_view::interface_bitmap::{InterfaceArt, draw_bitmap_from_sequence_shaded};

use super::list::fill_rect;

const WHITE: RgbColor = [0xffff; 3];
const ICON_HEIGHT: i32 = 0x11;
const ICON_WIDTH: i32 = 0x16;
const TEAM_INDENT: i32 = 8;
const ICON_SEQUENCE: i32 = 0;
const READY_SEQUENCE: i32 = 4;
const CENTRED: u32 = 0x10;
const NAME_FLAGS: u32 = text_flags::MIDDLE | text_flags::SHADOW;
const RIGHT_FLAGS: u32 =
    text_flags::RIGHT | text_flags::MIDDLE | text_flags::SHADOW | text_flags::COLOR_CODES;
const TAG_WIDTH: i32 = 0x5a;
const CHAT_TEXT_LEFT: i32 = 0x60;
const CHAT_TEXT_FLAGS: u32 = text_flags::MIDDLE | text_flags::SHADOW | text_flags::COLOR_CODES;
const NOTICE_TRANSPARENCY: u32 = 0x8000;
pub const CHAT_WRAP_MARGIN: i32 = 0x7a;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerRow {
    pub id: i32,
    pub icon: i16,
    pub name: Vec<u8>,
    pub colors: [[u16; 4]; 2],
    pub ready: bool,
    pub host: bool,
    pub in_team: bool,
    pub heading: bool,
    pub right: Vec<u8>,
    pub right_under_bar: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRow {
    pub kind: i16,
    pub name: Vec<u8>,
    pub color: RgbColor,
    pub text: Vec<u8>,
}

pub struct RowArt<'a> {
    pub family: &'a FontFamily,
    pub locale: &'a TextLocale,
    pub nepi: Option<&'a Arc<InterfaceArt>>,
}

fn clip_to(r: Rect, clip: Rect) -> Rect {
    Rect::new(
        r.top.max(clip.top),
        r.left.max(clip.left),
        r.bottom.min(clip.bottom),
        r.right.min(clip.right),
    )
}

pub fn draw_player(
    row: &PlayerRow,
    rect: Rect,
    clip: Rect,
    enabled: bool,
    art: &RowArt<'_>,
    bitmap: &mut Bitmap32,
) {
    let area = Rect::new(
        rect.top + 2,
        rect.left + 2,
        rect.top + 2 + ICON_HEIGHT,
        rect.right - 2,
    );
    if row.heading {
        let c = row.colors[0];
        fill_rect(bitmap, [c[0], c[1], c[2]], clip_to(area, clip), 0x4000);
        font::draw_string(
            bitmap,
            area,
            TextClip::Rect(clip),
            art.family,
            font::STYLE_ITALIC,
            &row.name,
            WHITE,
            NAME_FLAGS,
            if enabled { 0 } else { 0x8000 },
            art.locale,
        );
        return;
    }
    let indent = if row.in_team { TEAM_INDENT } else { 0 };
    let icon = Rect::new(
        area.top,
        area.left + indent,
        area.top + ICON_HEIGHT,
        area.left + indent + ICON_WIDTH,
    );
    let bar = Rect::new(area.top, icon.right + 2, area.bottom, area.right);
    let (bar_transparency, text_transparency) = if enabled {
        (0x4000, 0)
    } else {
        (0xc000, 0x8000)
    };
    let palette = art
        .nepi
        .map(|nepi| nepi.coll.palette_of_colors(&row.colors));
    if let (Some(nepi), Some(palette)) = (art.nepi, &palette) {
        draw_bitmap_from_sequence_shaded(
            bitmap,
            nepi,
            palette,
            ICON_SEQUENCE,
            -1,
            i32::from(row.icon),
            icon,
            CENTRED,
            0,
            text_transparency,
        );
    }
    let text = |bitmap: &mut Bitmap32, s: &[u8], flags: u32| {
        font::draw_string(
            bitmap,
            bar,
            TextClip::Rect(clip),
            art.family,
            font::STYLE_PLAIN,
            s,
            WHITE,
            flags,
            text_transparency,
            art.locale,
        );
    };
    if row.right_under_bar && !row.right.is_empty() {
        text(bitmap, &row.right, RIGHT_FLAGS);
    }
    let fill = if row.in_team {
        row.colors[1]
    } else {
        row.colors[0]
    };
    fill_rect(
        bitmap,
        [fill[0], fill[1], fill[2]],
        clip_to(bar, clip),
        bar_transparency,
    );
    if (row.ready || row.host)
        && let (Some(nepi), Some(palette)) = (art.nepi, &palette)
    {
        draw_bitmap_from_sequence_shaded(
            bitmap,
            nepi,
            palette,
            READY_SEQUENCE,
            -1,
            i32::from(!row.host),
            icon,
            CENTRED,
            0,
            text_transparency,
        );
    }
    text(bitmap, &row.name, NAME_FLAGS);
    if !row.right_under_bar && !row.right.is_empty() {
        text(bitmap, &row.right, RIGHT_FLAGS);
    }
}

fn fill_tag(bitmap: &mut Bitmap32, r: Rect, color: RgbColor, clip: Rect) {
    let (top, bottom, left) = (r.top, r.bottom, r.left);
    let box_right = left + TAG_WIDTH;
    let mid = (top + bottom) / 2;
    let tip = left + 0x59 + (bottom - top) / 2;
    for y in top..bottom {
        let right = if y < mid - 1 {
            let span = (mid - 1 - top).max(1);
            box_right + (tip - box_right) * (y - top) / span
        } else if y <= mid + 1 {
            tip
        } else {
            let span = (bottom - (mid + 1)).max(1);
            tip - (tip - box_right) * (y - (mid + 1)) / span
        };
        fill_rect(
            bitmap,
            color,
            clip_to(Rect::new(y, left, y + 1, right), clip),
            0,
        );
    }
}

pub fn draw_chat(row: &ChatRow, rect: Rect, clip: Rect, art: &RowArt<'_>, bitmap: &mut Bitmap32) {
    let mut r = Rect::new(rect.top, rect.left + 3, rect.bottom, rect.right - 3);
    r.top += 1;
    let bar = match row.kind {
        1 => Some([0xffff, 0, 0]),
        2 => Some([0, 0, 0xffff]),
        4 => Some([0, 0xffff, 0]),
        _ => None,
    };
    if let Some(bar) = bar {
        fill_rect(bitmap, bar, clip_to(r, clip), NOTICE_TRANSPARENCY);
    }
    if !(0..2).contains(&row.kind) && row.kind != 4 {
        font::draw_string(
            bitmap,
            r,
            TextClip::Rect(clip),
            art.family,
            font::STYLE_PLAIN,
            &row.text,
            WHITE,
            CHAT_TEXT_FLAGS,
            0,
            art.locale,
        );
        return;
    }
    fill_tag(bitmap, r, row.color, clip_to(r, clip));
    let name = Rect::new(r.top, r.left, r.bottom, r.left + TAG_WIDTH);
    font::draw_string(
        bitmap,
        name,
        TextClip::Rect(clip_to(r, clip)),
        art.family,
        font::STYLE_PLAIN,
        &row.name,
        WHITE,
        NAME_FLAGS,
        0,
        art.locale,
    );
    let text = Rect::new(r.top, r.left + CHAT_TEXT_LEFT, r.bottom, r.right);
    font::draw_string(
        bitmap,
        text,
        TextClip::Rect(clip),
        art.family,
        font::STYLE_PLAIN,
        &row.text,
        WHITE,
        CHAT_TEXT_FLAGS,
        0,
        art.locale,
    );
}

pub fn wrap_chat(
    family: &FontFamily,
    locale: &TextLocale,
    text: &[u8],
    width: i32,
) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut style = font::STYLE_PLAIN;
    loop {
        let rest = &text[at.min(text.len())..];
        let line = font::wrap_string(family, style, rest, width, font::TEXT_WRAP, locale);
        let count = if line.count == 0 {
            rest.len()
        } else {
            line.count.min(rest.len())
        };
        let mut piece = Vec::new();
        if !out.is_empty() {
            piece.extend_from_slice(match style {
                font::STYLE_BOLD => b"|b",
                font::STYLE_ITALIC => b"|i",
                _ => b"",
            });
        }
        piece.extend_from_slice(&rest[..count]);
        out.push(piece);
        if line.end == font::LineEnd::End || count >= rest.len() {
            break;
        }
        style = line.style;
        at += count;
        while text.get(at) == Some(&b' ') {
            at += 1;
        }
        if at >= text.len() {
            break;
        }
    }
    out
}
