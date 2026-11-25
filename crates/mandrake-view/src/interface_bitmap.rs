use mandrake_tag::font::{Bitmap32, transparency_table_entry};
use mandrake_tag::hud::{SequenceInstance, sequence_instance, stored_bitmap_pixels};
use mandrake_tag::{CollectionTag, Rect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBitmap {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<Option<(u8, u8)>>,
    pub eight_bit: bool,
}

pub struct InterfaceArt {
    pub raw: Vec<u8>,
    pub coll: CollectionTag,
    pub bitmaps: Vec<Option<StoredBitmap>>,
}

impl InterfaceArt {
    pub fn from_bytes(raw: Vec<u8>) -> Option<Self> {
        let coll = CollectionTag::from_bytes(&raw).ok()?;
        let bitmaps = (0..coll.frame_count())
            .map(|i| {
                let f = coll.frame(i)?;
                let (w, h, px) = stored_bitmap_pixels(&raw, i)?;
                let eight_bit = f.flags & 0x10 == 0;
                Some(StoredBitmap {
                    width: w as i32,
                    height: h as i32,
                    pixels: px
                        .into_iter()
                        .map(|p| p.map(|(i, a)| (i, if eight_bit { 0 } else { a & 0xf })))
                        .collect(),
                    eight_bit,
                })
            })
            .collect();
        Some(Self { raw, coll, bitmaps })
    }

    pub fn bitmap(
        &self,
        sequence: i32,
        view: i32,
        frame: i32,
    ) -> Option<(&StoredBitmap, SequenceInstance)> {
        let (view, frame) = if view == -1 {
            let (views, frames) = self.raw_views_frames(sequence)?;
            if frames <= 0 || views <= 0 {
                return None;
            }
            let n = frame.rem_euclid(frames * views);
            (n / frames, n % frames)
        } else {
            (view, frame)
        };
        let inst = sequence_instance(&self.raw, sequence, view, frame)?;
        Some((self.bitmaps.get(inst.bitmap)?.as_ref()?, inst))
    }

    fn raw_views_frames(&self, sequence: i32) -> Option<(i32, i32)> {
        let be32 = |o: usize| {
            let end = o.checked_add(4)?;
            self.raw
                .get(o..end)
                .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        let be16 = |o: usize| {
            let end = o.checked_add(2)?;
            self.raw
                .get(o..end)
                .map(|b| i16::from_be_bytes([b[0], b[1]]) as i32)
        };
        let h = CollectionTag::HEADER_SIZE;
        let (count, off) = (be32(0x80)?, be32(0x84)?);
        if sequence < 0 || sequence >= count {
            return None;
        }
        let off = usize::try_from(off).ok()?;
        let seq_entry = h
            .checked_add(off)?
            .checked_add((sequence as usize).checked_mul(0x80)?)?
            .checked_add(0x40)?;
        let seq_off = usize::try_from(be32(seq_entry)?).ok()?;
        let seq = h.checked_add(seq_off)?;
        Some((be16(seq.checked_add(8)?)?, be16(seq.checked_add(0x0a)?)?))
    }

    pub fn frames(&self, sequence: i32) -> i32 {
        self.coll
            .sequence(sequence.max(0) as usize)
            .filter(|_| sequence >= 0)
            .map_or(0, |s| s.frames_per_view as i32)
    }

    pub fn views(&self, sequence: i32) -> i32 {
        self.coll
            .sequence(sequence.max(0) as usize)
            .filter(|_| sequence >= 0)
            .map_or(0, |s| s.num_views as i32)
    }

    pub fn palette(&self) -> &[[u8; 4]] {
        self.coll.palette()
    }
}

fn intersect(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.top.max(b.top),
        a.left.max(b.left),
        a.bottom.min(b.bottom),
        a.right.min(b.right),
    )
}

pub fn palette_pixel(c: [u8; 4]) -> u32 {
    0xff00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

pub fn blend_level(dst: u32, src: u32, level: u32) -> u32 {
    let ch = |s: u32| {
        transparency_table_entry(level, (dst >> s & 0xff) >> 3, (src >> s & 0xff) >> 3) << 3
    };
    0xff00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

pub fn copy_bitmap(
    dst: &mut Bitmap32,
    bitmap: &StoredBitmap,
    palette: &[[u8; 4]],
    x: i32,
    y: i32,
    clip: Rect,
    transparency: u32,
) {
    let (w, h) = (bitmap.width, bitmap.height);
    let area = intersect(
        intersect(clip, Rect::new(0, 0, dst.height, dst.width)),
        Rect::new(y, x, y + h, x + w),
    );
    let call_level = (transparency >> 12) & 0xf;
    for py in area.top..area.bottom {
        for px in area.left..area.right {
            let Some(&Some((i, level))) = bitmap.pixels.get(((py - y) * w + px - x) as usize)
            else {
                continue;
            };
            if bitmap.eight_bit && i == 0 {
                continue;
            }
            let mut level = level as u32;
            let combined = !bitmap.eight_bit && transparency >> 4 & 0xfff != 0;
            if combined {
                level = level.max(call_level);
            } else if level >= 15 {
                continue;
            }
            let d = dst.get(px, py);
            let src = if i == 0 {
                0xff00_0000
            } else {
                palette_pixel(palette.get(i as usize).copied().unwrap_or([0; 4]))
            };
            let mut v = if level == 0 {
                src
            } else {
                blend_level(d, src, level)
            };
            if call_level != 0 && !combined {
                v = blend_level(d, v, call_level);
            }
            dst.set(px, py, v);
        }
    }
}

pub mod placement {
    pub const LEFT: u32 = 0x01;
    pub const RIGHT: u32 = 0x02;
    pub const TOP: u32 = 0x04;
    pub const BOTTOM: u32 = 0x08;
    pub const REGISTRATION: u32 = 0x10;
    pub const SHADED: u32 = 0x20;
    pub const RETURN_RECT: u32 = 0x40;
}

#[allow(clippy::too_many_arguments)]
pub fn draw_bitmap_from_sequence(
    dst: &mut Bitmap32,
    art: &InterfaceArt,
    sequence: i32,
    view: i32,
    frame: i32,
    rect: Rect,
    flags: u32,
    offset: i32,
    transparency: u32,
) -> Option<Rect> {
    draw_bitmap_from_sequence_shaded(
        dst,
        art,
        art.palette(),
        sequence,
        view,
        frame,
        rect,
        flags,
        offset,
        transparency,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn draw_bitmap_from_sequence_shaded(
    dst: &mut Bitmap32,
    art: &InterfaceArt,
    palette: &[[u8; 4]],
    sequence: i32,
    view: i32,
    frame: i32,
    rect: Rect,
    flags: u32,
    offset: i32,
    transparency: u32,
) -> Option<Rect> {
    let (bitmap, inst) = art.bitmap(sequence, view, frame)?;
    let (w, h) = (bitmap.width, bitmap.height);
    let mut x = if flags & placement::RIGHT != 0 {
        rect.right - w - offset
    } else if flags & placement::LEFT != 0 {
        offset + rect.left
    } else {
        rect.left + (rect.right - rect.left) / 2 - w / 2
    };
    let mut y = if flags & placement::TOP != 0 {
        offset + rect.top
    } else if flags & placement::BOTTOM != 0 {
        rect.bottom - h - offset
    } else {
        rect.top + (rect.bottom - rect.top) / 2 - h / 2
    };
    if flags & placement::REGISTRATION != 0 {
        x += w / 2 - inst.origin.0 as i32;
        y += h / 2 - inst.origin.1 as i32;
    }
    copy_bitmap(
        dst,
        bitmap,
        palette,
        x,
        y,
        Rect::new(0, 0, dst.height, dst.width),
        transparency,
    );
    Some(Rect::new(y, x, y + h, x + w))
}

pub const BEVEL_PLACEMENT: [u32; 9] = [0, 0x35, 0x36, 0x39, 0x3a, 0x24, 0x21, 0x28, 0x22];

pub const BACKGROUND_ORDER: [i32; 9] = [0, 5, 6, 7, 8, 1, 2, 3, 4];

pub fn tile_and_bevel_rectangle(
    dst: &mut Bitmap32,
    art: &InterfaceArt,
    sequence: i32,
    view: i32,
    rect: Rect,
    clip: Option<Rect>,
    order: Option<&[i32; 9]>,
) {
    let target = match clip {
        Some(c) => {
            let r = intersect(rect, c);
            if r.left >= r.right || r.top >= r.bottom {
                return;
            }
            r
        }
        None => rect,
    };
    let count = art.frames(sequence).min(9);
    let (mut top_l, mut top_r) = (rect.left, rect.right);
    let (mut bottom_l, mut bottom_r) = (rect.left, rect.right);
    let (mut left_t, mut left_b) = (rect.top, rect.bottom);
    let (mut right_t, mut right_b) = (rect.top, rect.bottom);
    for step in 0..count {
        let frame = order.map_or(step, |o| o[step as usize]);
        if !(0..9).contains(&frame) {
            continue;
        }
        let Some((bitmap, _)) = art.bitmap(sequence, view, frame) else {
            continue;
        };
        let (w, h) = (bitmap.width, bitmap.height);
        let word = BEVEL_PLACEMENT[frame as usize];
        let (mut y0, mut y1) = (rect.top, rect.bottom);
        let (mut x0, mut x1) = (rect.left, rect.right);
        if word & 4 != 0 {
            y1 = rect.top + h;
        }
        if word & 8 != 0 {
            y0 = rect.bottom - h;
        }
        if word & 1 != 0 {
            x1 = rect.left + w;
        }
        if word & 2 != 0 {
            x0 = rect.right - w;
        }
        if word & 0x20 != 0 {
            if word & 4 != 0 {
                x0 = x0.max(top_l);
                x1 = x1.min(top_r);
            }
            if word & 8 != 0 {
                x0 = x0.max(bottom_l);
                x1 = x1.min(bottom_r);
            }
            if word & 1 != 0 {
                y0 = y0.max(left_t);
                y1 = y1.min(left_b);
            }
            if word & 2 != 0 {
                y0 = y0.max(right_t);
                y1 = y1.min(right_b);
            }
        }
        if word & 0x10 != 0 {
            if word & 5 == 5 {
                left_t = y1;
                top_l = x1;
            }
            if word & 9 == 9 {
                left_b = y0;
                bottom_l = x1;
            }
            if word & 6 == 6 {
                top_r = x0;
                right_t = y1;
            }
            if word & 0xa == 0xa {
                bottom_r = x0;
                right_b = y0;
            }
        }
        if w <= 0 || h <= 0 {
            continue;
        }
        let cols = (w + (x1 - x0) - 1).div_euclid(w);
        let rows = (h + (y1 - y0) - 1).div_euclid(h);
        for row in 0..rows.max(0) {
            for col in 0..cols.max(0) {
                copy_bitmap(
                    dst,
                    bitmap,
                    art.palette(),
                    x0 + col * w,
                    y0 + row * h,
                    target,
                    0,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: i32, h: i32, px: Vec<u8>) -> StoredBitmap {
        StoredBitmap {
            width: w,
            height: h,
            pixels: px.into_iter().map(|i| Some((i, 0))).collect(),
            eight_bit: true,
        }
    }

    #[test]
    fn copy_skips_index_zero_and_clips() {
        let pal = vec![[0, 0, 0, 255], [10, 20, 30, 255], [200, 100, 50, 255]];
        let b = frame(3, 2, vec![1, 0, 2, 2, 1, 0]);
        let mut dst = Bitmap32::new(4, 3, 0xff00_0000 | 0x0a0b0c);
        copy_bitmap(&mut dst, &b, &pal, 1, 1, Rect::new(0, 0, 3, 3), 0);
        assert_eq!(dst.get(1, 1), 0xff0a141e);
        assert_eq!(dst.get(2, 1), 0xff0a0b0c, "index 0 is transparent");
        assert_eq!(dst.get(1, 2), 0xffc86432);
        assert_eq!(
            dst.get(3, 1),
            0xff0a0b0c,
            "clipped at the clip's right edge"
        );
    }

    #[test]
    fn a_16_bit_bitmap_under_a_call_transparency_blends_once() {
        let pal = vec![[0u8; 4], [0xff, 0xff, 0xff, 0xff]];
        let draw = |level: u8, transparency: u32| {
            let b = StoredBitmap {
                width: 1,
                height: 1,
                pixels: vec![Some((1, level))],
                eight_bit: false,
            };
            let mut dst = Bitmap32::new(1, 1, 0xff00_0000);
            copy_bitmap(
                &mut dst,
                &b,
                &pal,
                0,
                0,
                Rect::new(0, 0, 1, 1),
                transparency,
            );
            dst.get(0, 0)
        };
        let white = 0xffff_ffff;
        let at = |level| blend_level(0xff00_0000, white, level);
        assert_eq!(draw(0, 0), white);
        assert_eq!(draw(3, 0), at(3));
        assert_eq!(draw(0, 0x8000), at(8));
        assert_eq!(draw(3, 0x8000), at(8), "the call's is higher");
        assert_eq!(draw(11, 0x8000), at(11), "the pixel's is higher");
        assert_eq!(
            draw(15, 0),
            0xff00_0000,
            "skipped with no call transparency"
        );
    }

    #[test]
    fn transparent_copy_blends_on_five_bit_channels() {
        let pal = vec![[0, 0, 0, 255], [0xf8, 0xf8, 0xf8, 255]];
        let b = frame(1, 1, vec![1]);
        let mut dst = Bitmap32::new(1, 1, 0xff00_0000);
        copy_bitmap(&mut dst, &b, &pal, 0, 0, Rect::new(0, 0, 1, 1), 0xaaaa);
        assert_eq!(dst.get(0, 0), 0xff505050);
    }

    fn empty_coll() -> CollectionTag {
        CollectionTag {
            bitmap_count: 0,
            pixels_to_world: 0,
            palette: Vec::new(),
            frames: Vec::new(),
            sequences: Vec::new(),
            shadow_descriptors: Vec::new(),
            color_table: Vec::new(),
            hue_changes: Vec::new(),
            sequence_instance_flags: Vec::new(),
            sequence_missing_bitmaps: Vec::new(),
        }
    }

    #[test]
    fn raw_views_frames_rejects_negative_offsets_without_panicking() {
        let mut raw = vec![0u8; 0x200];
        raw[0x80..0x84].copy_from_slice(&1i32.to_be_bytes());
        raw[0x84..0x88].copy_from_slice(&(-16i32).to_be_bytes());
        let art = InterfaceArt {
            raw,
            coll: empty_coll(),
            bitmaps: Vec::new(),
        };
        assert_eq!(art.raw_views_frames(0), None);

        let mut raw2 = vec![0u8; 0x300];
        raw2[0x80..0x84].copy_from_slice(&1i32.to_be_bytes());
        raw2[0x84..0x88].copy_from_slice(&0i32.to_be_bytes());
        let entry = CollectionTag::HEADER_SIZE + 0x40;
        raw2[entry..entry + 4].copy_from_slice(&(-4i32).to_be_bytes());
        let art2 = InterfaceArt {
            raw: raw2,
            coll: empty_coll(),
            bitmaps: Vec::new(),
        };
        assert_eq!(art2.raw_views_frames(0), None);
    }

    #[test]
    fn view_neg1_wraps_negative_frame_with_rem_euclid() {
        let mut raw = vec![0u8; 0x800];
        let h = CollectionTag::HEADER_SIZE;
        raw[0x60..0x64].copy_from_slice(&6i32.to_be_bytes());
        raw[0x70..0x74].copy_from_slice(&6i32.to_be_bytes());
        let inst_off: i32 = 0x300;
        raw[0x74..0x78].copy_from_slice(&inst_off.to_be_bytes());
        raw[0x80..0x84].copy_from_slice(&1i32.to_be_bytes());
        raw[0x84..0x88].copy_from_slice(&0i32.to_be_bytes());
        let seq_off: i32 = 0x100;
        raw[h + 0x40..h + 0x44].copy_from_slice(&seq_off.to_be_bytes());
        let seq = h + seq_off as usize;
        raw[seq + 0x08..seq + 0x0a].copy_from_slice(&2i16.to_be_bytes());
        raw[seq + 0x0a..seq + 0x0c].copy_from_slice(&3i16.to_be_bytes());
        let stride = 2 * 2 + 0x2e;
        for v in 0..2usize {
            for f in 0..3usize {
                let slot = (v * 3 + f) as i16;
                let o = seq + 0x6e + v * 2 + f * stride;
                raw[o..o + 2].copy_from_slice(&slot.to_be_bytes());
                let base = h + inst_off as usize + slot as usize * 0x40;
                raw[base + 0x1c..base + 0x1e].copy_from_slice(&slot.to_be_bytes());
            }
        }
        let art = InterfaceArt {
            raw,
            coll: empty_coll(),
            bitmaps: (0..6)
                .map(|i| Some(frame(1, 1, vec![i as u8 + 1])))
                .collect(),
        };
        assert_eq!(art.raw_views_frames(0), Some((2, 3)));
        let wrapped = art
            .bitmap(0, -1, -1)
            .expect("negative frame wraps to slot 5");
        assert_eq!(wrapped.1.bitmap, 5);
        assert_eq!(Some(wrapped), art.bitmap(0, -1, 5));
    }
}
