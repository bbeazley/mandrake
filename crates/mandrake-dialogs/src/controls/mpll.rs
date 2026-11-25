use std::sync::Arc;

use mandrake_tag::Rect;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, TextLocale, text_flags};
use mandrake_view::interface_bitmap::{InterfaceArt, StoredBitmap, copy_bitmap};

pub const ICON_BITMAP: usize = 0xc;
pub const ICON_INSET: i32 = 3;
pub const NO_ICON_LINE_HEIGHT: i32 = 0x40;
const BUTTON_BITMAP: usize = 0x38;
const BUTTON_HELD_BITMAP: usize = 0x3a;
const BUTTON_DOWN: i32 = 2;
const TEXT_DOWN: i32 = 2;
const URL_DOWN: i32 = 2;
const TEXT_COLOR: usize = 3;
const NOT_FOUND_COLOR: usize = 5;
const NOT_FOUND: &[u8] = b"Not Found";
const DISABLED_TRANSPARENCY: u32 = 0x8000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPluginRow {
    pub name: Vec<u8>,
    pub url: Vec<u8>,
    pub checksum: u32,
    pub pressed: bool,
    pub not_found: bool,
}

impl MissingPluginRow {
    pub fn new(name: &[u8], url: &[u8], checksum: u32) -> Self {
        Self {
            name: name.to_vec(),
            url: url.to_vec(),
            checksum,
            pressed: false,
            not_found: false,
        }
    }
}

pub struct RowArt<'a> {
    pub family: &'a FontFamily,
    pub locale: &'a TextLocale,
    pub sba: Option<&'a Arc<InterfaceArt>>,
    pub baie: Option<&'a Arc<InterfaceArt>>,
    pub colors: &'a [RgbColor; 10],
}

fn stored(art: Option<&Arc<InterfaceArt>>, index: usize) -> Option<(&InterfaceArt, &StoredBitmap)> {
    let art = art?;
    Some((art, art.bitmaps.get(index)?.as_ref()?))
}

pub fn line_height(sba: Option<&Arc<InterfaceArt>>) -> i32 {
    stored(sba, ICON_BITMAP).map_or(NO_ICON_LINE_HEIGHT, |(_, b)| b.height + ICON_INSET)
}

pub fn draw_row(
    row: &MissingPluginRow,
    rect: Rect,
    clip: Rect,
    selected: bool,
    enabled: bool,
    art: &RowArt<'_>,
    bitmap: &mut Bitmap32,
) {
    let transparency = if enabled { 0 } else { DISABLED_TRANSPARENCY };
    let shadow = if selected { 0 } else { text_flags::SHADOW };
    let button = if row.not_found {
        None
    } else {
        let index = if row.pressed {
            BUTTON_HELD_BITMAP
        } else {
            BUTTON_BITMAP
        };
        stored(art.baie, index)
    };
    if let Some((sba, icon)) = stored(art.sba, ICON_BITMAP) {
        copy_bitmap(
            bitmap,
            icon,
            sba.palette(),
            rect.left + ICON_INSET,
            rect.top + ICON_INSET,
            clip,
            transparency,
        );
    }
    let mut text = Rect::new(
        rect.top + TEXT_DOWN,
        rect.left + (rect.bottom + 2 - rect.top),
        rect.bottom,
        rect.right,
    );
    let draw = |bitmap: &mut Bitmap32, pen, r, style, s: &[u8], color, flags| {
        font::draw_string_at(
            bitmap,
            pen,
            r,
            TextClip::Rect(clip),
            art.family,
            style,
            s,
            color,
            flags | text_flags::CLIP_LINES,
            transparency,
            art.locale,
        )
    };
    match button {
        Some((_, b)) => text.right -= b.width,
        None if row.not_found => {
            draw(
                bitmap,
                (0, 0),
                text,
                font::STYLE_ITALIC,
                NOT_FOUND,
                art.colors[NOT_FOUND_COLOR],
                shadow | text_flags::RIGHT,
            );
            text.right -= font::string_width(art.family, font::STYLE_ITALIC, NOT_FOUND, art.locale);
        }
        None => {}
    }
    let mut line = row.name.clone();
    line.push(b'\r');
    let color = art.colors[TEXT_COLOR];
    let mut pen = draw(bitmap, (0, 0), text, font::STYLE_BOLD, &line, color, shadow);
    pen.1 += URL_DOWN;
    if !row.url.is_empty() {
        line = b"|i".to_vec();
        if !row.url.starts_with(b"http://") {
            line.extend_from_slice(b"http://");
        }
        line.extend_from_slice(&row.url);
        line.push(b'\r');
    }
    if row.not_found {
        text.right = rect.right;
    }
    draw(
        bitmap,
        pen,
        text,
        font::STYLE_PLAIN,
        &line,
        color,
        shadow | text_flags::WRAP,
    );
    if let Some((baie, b)) = button {
        copy_bitmap(
            bitmap,
            b,
            baie.palette(),
            clip.right - b.width,
            clip.top + BUTTON_DOWN,
            clip,
            transparency,
        );
    }
}

pub fn click(
    rows: &mut [MissingPluginRow],
    hit: Option<(usize, i32, i32)>,
    held: bool,
    baie: Option<&Arc<InterfaceArt>>,
) -> bool {
    let on_button = hit.and_then(|(row, down, from_right)| {
        let (_, b) = stored(baie, BUTTON_BITMAP)?;
        let inside = down >= BUTTON_DOWN
            && down < b.height + BUTTON_DOWN
            && (0..b.width).contains(&from_right);
        inside.then_some(row)
    });
    let mut changed = false;
    for (i, row) in rows.iter_mut().enumerate() {
        let pressed = on_button == Some(i) && held;
        if on_button == Some(i) && row.pressed && !held {
            tracing::info!(
                plugin = %String::from_utf8_lossy(&row.name),
                "dialogs: the plugin look-up isn't built"
            );
        }
        changed |= row.pressed != pressed;
        row.pressed = pressed;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_holds_only_the_button_under_it() {
        let mut rows = vec![
            MissingPluginRow::new(b"A", b"a.net", 1),
            MissingPluginRow::new(b"B", b"", 2),
        ];
        assert!(!click(&mut rows, Some((0, 4, 4)), true, None));
        assert!(rows.iter().all(|r| !r.pressed));
    }
}
