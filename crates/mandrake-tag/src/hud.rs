use crate::collection::CollectionTag;
use crate::dialog::{DialogDefinition, Rect};
use crate::font::RgbColor;
use mandrake_common::FourCC;

pub const STATUS_BAR_ID: FourCC = FourCC(*b"!sba");
pub const CONTROL_BAR_ID: FourCC = FourCC(*b"!cba");
pub const EXPERIENCE_ICONS_ID: FourCC = FourCC(*b"isb6");
pub const DIFFICULTY_NAMES_ID: FourCC = FourCC(*b"indl");
pub const CONTROL_BAR_STRINGS_ID: FourCC = FourCC(*b"incb");
pub const OVERHEAD_MAP_POPUP_ID: FourCC = FourCC(*b"inom");

pub const STATUS_BAR_BASE_WIDTH: i32 = 640;

pub const STATUS_BAR_TITLE_FORMAT: &str = "|b%s|p |i(%s)|p";
pub const STATUS_BAR_SCORED_TITLE_FORMAT: &str = "|b%s|p %s |i(%s)|p";
pub const GAME_SCORING_NAMES_ID: FourCC = FourCC(*b"innn");

pub mod status_item {
    pub const PAUSE: u32 = 1;
    pub const MONSTER_STATUS: u32 = 2;
    pub const DESCRIPTION: u32 = 3;
    pub const PORTRAIT: u32 = 4;
    pub const OVERHEAD_MAP: u32 = 5;
    pub const VERSION: u32 = 100;
    pub const ERASER: u32 = 6;
    pub const SKETCH: u32 = 7;
    pub const TITLE: u32 = 8;
    pub const RULES: u32 = 9;
    pub const HELP: u32 = 10;
}

pub mod status_sequence {
    pub const MAP_ICON: usize = 0;
    pub const EXPERIENCE_ICONS: usize = 2;
    pub const PAUSE: usize = 3;
    pub const HELP: usize = 4;
    pub const SKETCH: usize = 5;
    pub const ERASER: usize = 6;
    pub const PORTRAIT_FRAME: usize = 8;
    pub const COLOR_SWATCH: usize = 9;
    pub const BACKGROUND: usize = 10;
}

pub const DEFAULT_SEQUENCE_COLOR: RgbColor = [0xffff, 0xffff, 0xffff];

pub fn rgb_color_from_sequence(
    raw: &[u8],
    coll: &CollectionTag,
    sequence: i32,
    entry: i32,
) -> RgbColor {
    use crate::parse::{be_i16, be_i32, nonneg_i32};

    let h = CollectionTag::HEADER_SIZE;
    let get = || -> Option<RgbColor> {
        let seq_count = be_i32(raw, 0x80)?;
        let seq_off = nonneg_i32(raw, 0x84)?;
        if sequence < 0 || sequence >= seq_count {
            return None;
        }
        let sref = h
            .checked_add(seq_off)?
            .checked_add((sequence as usize).checked_mul(0x80)?)?;
        let seq_rel = nonneg_i32(raw, sref.checked_add(0x40)?)?;
        let seq = h.checked_add(seq_rel)?;
        if be_i16(raw, seq.checked_add(0x0a)?)? <= 0 || be_i16(raw, seq.checked_add(8)?)? <= 0 {
            return None;
        }
        let inst = be_i16(raw, seq.checked_add(0x6e)?)? as i32;
        let inst_count = be_i32(raw, 0x70)?;
        let inst_off = nonneg_i32(raw, 0x74)?;
        let inst = if inst >= 0 && inst_count > 0 && inst < inst_count {
            inst as usize
        } else {
            0
        };
        let inst_pos = h
            .checked_add(inst_off)?
            .checked_add(inst.checked_mul(0x40)?)?;
        let bitmap = be_i16(raw, inst_pos.checked_add(0x1c)?)?;
        if bitmap < 0 || bitmap as i32 >= be_i32(raw, 0x60)? {
            return None;
        }
        let b = coll.frame(bitmap as usize)?;
        if entry < 0 || entry >= b.width as i32 {
            return None;
        }
        let c = coll
            .color_table
            .get(*b.pixels.get(entry as usize)? as usize)?;
        Some([c[0], c[1], c[2]])
    };
    get().unwrap_or(DEFAULT_SEQUENCE_COLOR)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceInstance {
    pub bitmap: usize,
    pub origin: (i16, i16),
}

pub fn sequence_instance(
    raw: &[u8],
    sequence: i32,
    view: i32,
    frame: i32,
) -> Option<SequenceInstance> {
    use crate::parse::{be_i16, be_i32, nonneg_i32};

    let h = CollectionTag::HEADER_SIZE;
    let seq_count = be_i32(raw, 0x80)?;
    let seq_off = nonneg_i32(raw, 0x84)?;
    if sequence < 0 || sequence >= seq_count {
        return None;
    }
    let sref = h
        .checked_add(seq_off)?
        .checked_add((sequence as usize).checked_mul(0x80)?)?;
    let seq_rel = nonneg_i32(raw, sref.checked_add(0x40)?)?;
    let seq = h.checked_add(seq_rel)?;
    let views = be_i16(raw, seq.checked_add(8)?)? as i32;
    let frames = be_i16(raw, seq.checked_add(0x0a)?)? as i32;
    if frames <= 0 || views <= 0 {
        return None;
    }
    let (f, v) = (frame % frames, view % views);
    if f < 0 || v < 0 {
        return None;
    }
    let stride = (views as usize).checked_mul(2)?.checked_add(0x2e)?;
    let inst_slot = seq
        .checked_add(0x6e)?
        .checked_add((v as usize).checked_mul(2)?)?
        .checked_add((f as usize).checked_mul(stride)?)?;
    let inst = be_i16(raw, inst_slot)? as i32;
    let inst_count = be_i32(raw, 0x70)?;
    let inst_off = nonneg_i32(raw, 0x74)?;
    if inst < 0 || inst_count <= 0 || inst >= inst_count {
        return None;
    }
    let base = h
        .checked_add(inst_off)?
        .checked_add((inst as usize).checked_mul(0x40)?)?;
    let bitmap = be_i16(raw, base.checked_add(0x1c)?)?;
    if bitmap < 0 || bitmap as i32 >= be_i32(raw, 0x60)? {
        return None;
    }
    Some(SequenceInstance {
        bitmap: bitmap as usize,
        origin: (
            be_i16(raw, base.checked_add(0x0c)?)?,
            be_i16(raw, base.checked_add(0x0e)?)?,
        ),
    })
}

pub type StoredPixel = Option<(u8, u8)>;

pub fn stored_bitmap_pixels(raw: &[u8], index: usize) -> Option<(usize, usize, Vec<StoredPixel>)> {
    use crate::parse::{be_i16, nonneg_i32};

    let h0 = CollectionTag::HEADER_SIZE;
    let count = nonneg_i32(raw, 0x60)?;
    let off = nonneg_i32(raw, 0x64)?;
    if index >= count {
        return None;
    }
    let r = h0.checked_add(off)?.checked_add(index.checked_mul(128)?)?;
    let boff = nonneg_i32(raw, r.checked_add(64)?)?;
    let bsize = nonneg_i32(raw, r.checked_add(68)?)?;
    let b = h0.checked_add(boff)?;
    let w = usize::try_from(be_i16(raw, b)?).ok()?;
    let h = usize::try_from(be_i16(raw, b.checked_add(2)?)?).ok()?;
    let bpr = be_i16(raw, b.checked_add(4)?)?;
    let flags = be_i16(raw, b.checked_add(6)?)? as u16;
    let pixel_count = w.checked_mul(h)?;
    if pixel_count > crate::collection::MAX_BITMAP_PIXELS {
        return None;
    }
    let start = b.checked_add(48)?.checked_add(h.checked_mul(4)?)?;
    let end = b.checked_add(bsize)?.min(raw.len());
    if start > end {
        return None;
    }
    let pixel_slice = &raw[start..end];
    let shift: u32 = if (flags & 0x10) != 0 { 1 } else { 0 };
    if pixel_count > 0
        && (flags & 0x04) == 0
        && pixel_count.checked_shl(shift)? > end.saturating_sub(b.checked_add(48)?)
    {
        return None;
    }
    let mut out = vec![None; pixel_count];
    crate::collection::decode_bitmap_pixels(pixel_slice, w, h, bpr, flags, |x, y, idx, alpha| {
        out[y * w + x] = Some((idx, alpha));
    });
    Some((w, h, out))
}

pub const CONTROL_BAR_BACKGROUND_SEQUENCE: usize = 12;

pub const STATUS_BAR_ITEM_FLAGS: [(u32, u8); 9] = [
    (1, 3),
    (2, 4),
    (3, 4),
    (5, 3),
    (6, 3),
    (7, 3),
    (8, 4),
    (9, 4),
    (10, 3),
];

pub fn status_bar_rect(dialog: &DialogDefinition, screen_w: i32) -> Rect {
    Rect::new(0, 0, dialog.height(), screen_w)
}

pub fn status_bar_item_rect(id: u32, r: Rect, screen_w: i32) -> Rect {
    let dx = screen_w - STATUS_BAR_BASE_WIDTH;
    match STATUS_BAR_ITEM_FLAGS.iter().find(|e| e.0 == id) {
        Some(&(_, flags)) => Rect {
            left: if flags & 2 != 0 { r.left + dx } else { r.left },
            right: r.right + dx,
            ..r
        },
        None => r,
    }
}

pub fn status_bar_layout(dialog: &DialogDefinition, screen_w: i32) -> Vec<(u32, Rect)> {
    dialog
        .items
        .iter()
        .map(|it| (it.id, status_bar_item_rect(it.id, it.rect, screen_w)))
        .collect()
}

pub fn status_bar_item_visible(id: u32, has_selection: bool, network_game: bool) -> bool {
    use status_item::*;
    match id {
        PORTRAIT | MONSTER_STATUS | DESCRIPTION => has_selection,
        TITLE | VERSION => !has_selection,
        RULES | ERASER | SKETCH => network_game,
        _ => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlBarEntry {
    pub level: u8,
    pub item: u8,
    pub kind: u8,
    pub sequence: u8,
}

const fn cb(level: u8, item: u8, kind: u8, sequence: u8) -> ControlBarEntry {
    ControlBarEntry {
        level,
        item,
        kind,
        sequence,
    }
}

pub const CONTROL_BAR_ENTRIES: [ControlBarEntry; 17] = [
    cb(0, 1, 0, 1),
    cb(0, 2, 0, 1),
    cb(0, 3, 3, 1),
    cb(0, 4, 4, 1),
    cb(0, 5, 5, 1),
    cb(0, 6, 6, 1),
    cb(0, 7, 7, 1),
    cb(1, 8, 8, 2),
    cb(1, 9, 9, 3),
    cb(2, 10, 10, 4),
    cb(3, 11, 11, 5),
    cb(1, 12, 12, 6),
    cb(3, 13, 13, 7),
    cb(3, 14, 14, 8),
    cb(2, 15, 15, 9),
    cb(2, 16, 16, 10),
    cb(1, 17, 17, 11),
];

pub fn control_bar_entries(network_game: bool) -> [ControlBarEntry; 17] {
    let mut e = CONTROL_BAR_ENTRIES;
    if network_game {
        e[0].kind = 1;
        e[1].kind = 2;
    }
    e
}

pub fn control_bar_row_width(entries: &[ControlBarEntry], widths: &[i32], level: u8) -> i32 {
    let mut w = 8;
    for (e, &bw) in entries.iter().zip(widths) {
        if level >= e.level && e.kind != 0 && bw != 0 {
            w += bw + 8;
        }
        if e.kind == 2 || e.kind == 7 {
            w += 16;
        }
    }
    w
}

pub fn control_bar_level(entries: &[ControlBarEntry], widths: &[i32], screen_w: i32) -> u8 {
    (1..=3u8)
        .rev()
        .find(|&l| control_bar_row_width(entries, widths, l) < screen_w)
        .unwrap_or(0)
}

pub fn control_bar_rect(dialog: &DialogDefinition, screen_w: i32, screen_h: i32) -> Rect {
    Rect::new(screen_h - dialog.height(), 0, screen_h, screen_w)
}

pub fn control_bar_layout(
    entries: &[ControlBarEntry],
    sizes: &[(i32, i32)],
    screen_w: i32,
    bar_h: i32,
) -> Vec<(u8, Rect)> {
    let widths: Vec<i32> = sizes.iter().map(|s| s.0).collect();
    let level = control_bar_level(entries, &widths, screen_w);
    let x0 = (screen_w - control_bar_row_width(entries, &widths, level)) / 2;
    let mut acc = 8;
    let mut out = Vec::new();
    for (e, &(bw, bh)) in entries.iter().zip(sizes) {
        if level >= e.level && e.kind != 0 {
            let top = (bar_h - bh) / 2 + 1;
            out.push((e.item, Rect::new(top, x0 + acc, top + bh, x0 + acc + bw)));
            if bw != 0 {
                acc += bw + 8;
            }
        }
        if e.kind == 2 || e.kind == 7 {
            acc += 16;
        }
    }
    out
}

pub fn control_bar_screen_layout(
    entries: &[ControlBarEntry],
    sizes: &[(i32, i32)],
    screen_w: i32,
    screen_h: i32,
    bar_h: i32,
) -> Vec<(u8, Rect)> {
    control_bar_layout(entries, sizes, screen_w, bar_h)
        .into_iter()
        .map(|(i, r)| (i, r.offset(0, screen_h - bar_h)))
        .collect()
}

pub fn minimap_base_scale(map_w: i32, map_h: i32, screen_w: i32, screen_h: i32) -> f32 {
    let sx = 154.0 / map_w as f32 * (screen_w as f32 * 0.000_976_562_5);
    let sy = 154.0 / map_h as f32 * (screen_h as f32 / 768.0);
    sx.min(sy)
}

pub const MINIMAP_MARGIN: i32 = 5;

pub fn minimap_rect(map_w: i32, map_h: i32, screen_w: i32, screen_h: i32, status_h: i32) -> Rect {
    let s = minimap_base_scale(map_w, map_h, screen_w, screen_h);
    let (w, h) = ((map_w as f32 * s) as i32, (map_h as f32 * s) as i32);
    Rect::new(
        status_h + MINIMAP_MARGIN,
        screen_w - MINIMAP_MARGIN - w,
        status_h + MINIMAP_MARGIN + h,
        screen_w - MINIMAP_MARGIN,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn sba() -> DialogDefinition {
        let items: [(u32, &[u8; 4], Rect); 10] = [
            (1, b"butt", Rect::new(0, 544, 64, 594)),
            (2, b"STAT", Rect::new(7, 95, 21, 488)),
            (3, b"stat", Rect::new(21, 95, 61, 488)),
            (4, b"butt", Rect::new(0, 14, 64, 93)),
            (5, b"butt", Rect::new(41, 619, 58, 636)),
            (6, b"butt", Rect::new(29, 600, 43, 622)),
            (7, b"butt", Rect::new(13, 600, 27, 622)),
            (8, b"stat", Rect::new(7, 30, 21, 488)),
            (9, b"stat", Rect::new(21, 30, 61, 485)),
            (10, b"butt", Rect::new(0, 494, 64, 544)),
        ];
        DialogDefinition {
            version: 1,
            rect: Rect::new(0, 0, 64, 640),
            items: items
                .iter()
                .map(|&(id, k, rect)| crate::dialog::DialogItem {
                    kind: FourCC(*k),
                    id,
                    field_04: 0,
                    fields_08: [-1, -1, 0, 0],
                    rect,
                    name: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn status_bar_items_at_1024_and_3840() {
        let d = sba();
        let at_1024 = [
            (1, Rect::new(0, 928, 64, 978)),
            (2, Rect::new(7, 95, 21, 872)),
            (3, Rect::new(21, 95, 61, 872)),
            (4, Rect::new(0, 14, 64, 93)),
            (5, Rect::new(41, 1003, 58, 1020)),
            (6, Rect::new(29, 984, 43, 1006)),
            (7, Rect::new(13, 984, 27, 1006)),
            (8, Rect::new(7, 30, 21, 872)),
            (9, Rect::new(21, 30, 61, 869)),
            (10, Rect::new(0, 878, 64, 928)),
        ];
        let at_3840 = [
            (1, Rect::new(0, 3744, 64, 3794)),
            (2, Rect::new(7, 95, 21, 3688)),
            (3, Rect::new(21, 95, 61, 3688)),
            (4, Rect::new(0, 14, 64, 93)),
            (5, Rect::new(41, 3819, 58, 3836)),
            (6, Rect::new(29, 3800, 43, 3822)),
            (7, Rect::new(13, 3800, 27, 3822)),
            (8, Rect::new(7, 30, 21, 3688)),
            (9, Rect::new(21, 30, 61, 3685)),
            (10, Rect::new(0, 3694, 64, 3744)),
        ];
        assert_eq!(status_bar_layout(&d, 1024), at_1024);
        assert_eq!(status_bar_layout(&d, 3840), at_3840);
        assert_eq!(status_bar_rect(&d, 3840), Rect::new(0, 0, 64, 3840));
        for (id, r) in status_bar_layout(&d, 640) {
            assert_eq!(Some(r), d.item(id).map(|i| i.rect));
        }
    }

    #[test]
    fn status_bar_visibility() {
        use status_item::*;
        assert!(status_bar_item_visible(TITLE, false, false));
        assert!(!status_bar_item_visible(TITLE, true, false));
        assert!(status_bar_item_visible(PORTRAIT, true, false));
        assert!(!status_bar_item_visible(DESCRIPTION, false, false));
        assert!(!status_bar_item_visible(SKETCH, false, false));
        assert!(status_bar_item_visible(PAUSE, false, false));
    }

    fn cba_sizes() -> Vec<(i32, i32)> {
        CONTROL_BAR_ENTRIES
            .iter()
            .map(|e| if e.sequence == 1 { (56, 21) } else { (28, 21) })
            .collect()
    }

    #[test]
    fn control_bar_buttons_at_1024_and_3840() {
        let sizes = cba_sizes();
        let widths: Vec<i32> = sizes.iter().map(|s| s.0).collect();
        assert_eq!(control_bar_row_width(&CONTROL_BAR_ENTRIES, &widths, 3), 704);
        assert_eq!(control_bar_level(&CONTROL_BAR_ENTRIES, &widths, 1024), 3);
        assert_eq!(control_bar_level(&CONTROL_BAR_ENTRIES, &widths, 3840), 3);
        assert_eq!(control_bar_level(&CONTROL_BAR_ENTRIES, &widths, 640), 2);
        assert_eq!(control_bar_level(&CONTROL_BAR_ENTRIES, &widths, 560), 1);
        let lefts_1024 = [
            168, 232, 296, 360, 424, 504, 540, 576, 612, 648, 684, 720, 756, 792, 828,
        ];
        let lefts_3840 = [
            1576, 1640, 1704, 1768, 1832, 1912, 1948, 1984, 2020, 2056, 2092, 2128, 2164, 2200,
            2236,
        ];
        for (w, h, lefts) in [(1024, 768, lefts_1024), (3840, 2160, lefts_3840)] {
            let l = control_bar_screen_layout(&CONTROL_BAR_ENTRIES, &sizes, w, h, 32);
            assert_eq!(l.len(), 15);
            for (k, ((item, r), left)) in l.iter().zip(lefts).enumerate() {
                assert_eq!(*item as usize, k + 3);
                let bw = if k < 5 { 56 } else { 28 };
                assert_eq!(*r, Rect::new(h - 26, left, h - 5, left + bw), "item {item}");
            }
        }
        let net = control_bar_entries(true);
        let l = control_bar_layout(&net, &sizes, 1024, 32);
        assert_eq!(l.len(), 17);
        assert_eq!(l[0].0, 1);
    }

    #[test]
    fn control_bar_rect_at_the_bottom() {
        let d = DialogDefinition {
            version: 1,
            rect: Rect::new(736, 0, 768, 1024),
            items: Vec::new(),
        };
        assert_eq!(
            control_bar_rect(&d, 3840, 2160),
            Rect::new(2128, 0, 2160, 3840)
        );
    }

    #[test]
    fn minimap_rects_00to_le3o() {
        let s = minimap_base_scale(150, 150, 1024, 768);
        assert!((s - 1.026_666_7).abs() < 1e-6);
        assert!((minimap_base_scale(150, 150, 3840, 2160) - 2.8875).abs() < 1e-6);
        assert!((minimap_base_scale(154, 154, 1024, 768) - 1.0).abs() < 1e-6);
        assert!((minimap_base_scale(154, 154, 3840, 2160) - 2.8125).abs() < 1e-6);
        for map in [150, 154] {
            assert_eq!(
                minimap_rect(map, map, 1024, 768, 64),
                Rect::new(69, 865, 223, 1019)
            );
            assert_eq!(
                minimap_rect(map, map, 3840, 2160, 64),
                Rect::new(69, 3402, 502, 3835)
            );
        }
    }
}
