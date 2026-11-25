use crate::header::decode_c_string;
use mandrake_common::M2Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitmapKeying {
    Myth2,
    Tfl,
}

pub(crate) const MAX_BITMAP_PIXELS: usize = 4096 * 4096;
const MAX_COLLECTION_PIXELS: usize = 64 * 1024 * 1024;

fn uncompressed_has_raw_zero(pixels: &[u8], w: usize, h: usize, stride: usize) -> bool {
    (0..h).any(|y| {
        let Some(start) = y.checked_mul(stride) else {
            return false;
        };
        let Some(row) = pixels.get(start..) else {
            return false;
        };
        row[..w.min(row.len())].contains(&0)
    })
}

pub(crate) fn decode_bitmap_pixels(
    pixel_data: &[u8],
    w: usize,
    h: usize,
    bpr: i16,
    flags: u16,
    mut on_pixel: impl FnMut(usize, usize, u8, u8),
) {
    let shift: usize = if (flags & 0x0010) != 0 { 1 } else { 0 };
    let step = 1usize << shift;
    if (flags & 0x0004) != 0 {
        let mut pos = 0usize;
        for y in 0..h {
            let Some(run_count_i16) = crate::parse::be_i16(pixel_data, pos) else {
                break;
            };
            let Some(pcount_i16) = crate::parse::be_i16(pixel_data, pos + 2) else {
                break;
            };
            let run_count = run_count_i16.max(0) as usize;
            let pcount = pcount_i16.max(0) as usize;
            let Some(runs_bytes) = run_count.checked_mul(4) else {
                break;
            };
            let Some(runs_end) = pos.checked_add(4).and_then(|p| p.checked_add(runs_bytes)) else {
                break;
            };
            if runs_end <= pixel_data.len() {
                let mut raw_pos = runs_end;
                for r in 0..run_count {
                    let rh = pos + 4 + r * 4;
                    let x0 =
                        i16::from_be_bytes([pixel_data[rh], pixel_data[rh + 1]]).max(0) as usize;
                    let x1 = i16::from_be_bytes([pixel_data[rh + 2], pixel_data[rh + 3]]).max(0)
                        as usize;
                    let x_end = x1.min(w);
                    for x in x0..x_end {
                        let Some(next_raw) = raw_pos.checked_add(step) else {
                            break;
                        };
                        if next_raw <= pixel_data.len() {
                            if shift == 0 {
                                on_pixel(x, y, pixel_data[raw_pos], 0xff);
                            } else {
                                on_pixel(x, y, pixel_data[raw_pos + 1], pixel_data[raw_pos]);
                            }
                            raw_pos = next_raw;
                        }
                    }
                }
            }
            let Some(pixel_bytes) = pcount.checked_shl(shift as u32) else {
                break;
            };
            let Some(mut row_bytes) = runs_bytes
                .checked_add(4)
                .and_then(|v| v.checked_add(pixel_bytes))
            else {
                break;
            };
            if (flags & 0x0040) != 0 && (row_bytes & 1) != 0 {
                let Some(padded) = row_bytes.checked_add(1) else {
                    break;
                };
                row_bytes = padded;
            }
            let Some(next_pos) = pos.checked_add(row_bytes) else {
                break;
            };
            pos = next_pos;
        }
    } else {
        let row_stride = if bpr > 0 {
            bpr as usize
        } else {
            let Some(s) = w.checked_shl(shift as u32) else {
                return;
            };
            s
        };
        for y in 0..h {
            let Some(row_off) = y.checked_mul(row_stride) else {
                break;
            };
            if row_off >= pixel_data.len() {
                break;
            }
            for x in 0..w {
                let Some(px_off) = x
                    .checked_shl(shift as u32)
                    .and_then(|dx| row_off.checked_add(dx))
                else {
                    break;
                };
                let Some(px_end) = px_off.checked_add(step) else {
                    break;
                };
                if px_end <= pixel_data.len() {
                    if shift == 0 {
                        on_pixel(x, y, pixel_data[px_off], 0xff);
                    } else {
                        on_pixel(x, y, pixel_data[px_off + 1], pixel_data[px_off]);
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BitmapFrame {
    pub index: usize,
    pub name: String,
    pub width: u16,
    pub height: u16,
    pub offset_x: i16,
    pub offset_y: i16,
    pub flags: u16,
    pub pixel_data_offset: usize,
    pub pixel_data_size: usize,
    pub pixels: Vec<u8>,
    #[serde(default)]
    pub wide_pixels: Vec<u16>,
}

pub const WIDE_PIXEL_OUTSIDE_RUN: u16 = 0xff00;

impl BitmapFrame {
    pub fn to_rgba(&self, palette: &[[u8; 4]]) -> Vec<u8> {
        let total = (self.width as usize) * (self.height as usize);
        let mut rgba = vec![0u8; total * 4];
        for (i, &px) in self.pixels.iter().take(total).enumerate() {
            if px != 0 {
                let c = palette
                    .get(px as usize)
                    .copied()
                    .unwrap_or([px, px, px, 255]);
                let dst = i * 4;
                rgba[dst] = c[0];
                rgba[dst + 1] = c[1];
                rgba[dst + 2] = c[2];
                rgba[dst + 3] = 255;
            }
        }
        rgba
    }

    pub fn to_rgba_wide(&self, palette: &[[u8; 4]]) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.wide_pixels.len() * 4);
        for &px in &self.wide_pixels {
            let a = u32::from(!px >> 8 & 0xf) * 0x11;
            let i = usize::from(px & 0xff);
            let c = if i == 0 {
                [0; 4]
            } else {
                palette.get(i).copied().unwrap_or([0; 4])
            };
            let ch = |v: u8| ((u32::from(v) * (a + 1)) >> 8) as u8;
            rgba.extend_from_slice(&[ch(c[0]), ch(c[1]), ch(c[2]), a as u8]);
        }
        rgba
    }

    pub fn to_rgba_color_alpha(&self, palette: &[[u8; 4]], color_table: &[[u16; 4]]) -> Vec<u8> {
        let total = (self.width as usize) * (self.height as usize);
        let mut rgba = vec![0u8; total * 4];
        for (i, &px) in self.pixels.iter().take(total).enumerate() {
            if px == 0 {
                continue;
            }
            let c = palette
                .get(px as usize)
                .copied()
                .unwrap_or([px, px, px, 255]);
            let a = color_table
                .get(px as usize)
                .map_or(0xff, |e| (e[0].max(e[1]).max(e[2]) >> 8) as u8);
            rgba[i * 4..i * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], a]);
        }
        rgba
    }

    pub fn to_shadow_rgba(&self) -> Vec<u8> {
        let total = (self.width as usize) * (self.height as usize);
        let mut rgba = vec![0u8; total * 4];
        for (i, &px) in self.pixels.iter().take(total).enumerate() {
            if px > 0 {
                let dst = i * 4;
                rgba[dst] = 0;
                rgba[dst + 1] = 0;
                rgba[dst + 2] = 0;
                rgba[dst + 3] = px;
            }
        }
        rgba
    }

    pub fn world_size(&self, pixels_to_world: i16) -> (f32, f32) {
        let ptw = pixels_to_world.max(1) as f32;
        (
            (self.width as f32 * ptw) / 512.0,
            (self.height as f32 * ptw) / 512.0,
        )
    }
}

pub fn displayed_views(n: usize) -> usize {
    if n <= 2 { n } else { n / 2 + 1 }
}

pub fn mirror_view(n: usize, view: usize) -> usize {
    (2 * displayed_views(n) + (n & 1)).saturating_sub(view + 2)
}

pub fn view_for_relative_angle(relative: u16, num_views: usize, mirrored: bool) -> usize {
    if num_views <= 1 {
        return 0;
    }
    let angle = if mirrored {
        relative.wrapping_neg()
    } else {
        relative
    };
    let view = (angle as usize * num_views + 0x8000) >> 16;
    if view >= num_views { 0 } else { view }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CollectionSequence {
    pub index: usize,
    pub name: String,
    pub flags: u32,
    pub pixels_to_world: i16,
    pub num_views: u16,
    pub frames_per_view: u16,
    pub ticks_per_frame: i16,
    pub key_frame: i16,
    pub loop_frame: i16,
    pub bitmap_indices: Vec<usize>,
    pub shadow_indices: Vec<i16>,
    #[serde(default)]
    pub bounds: SequenceBounds,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SequenceBounds {
    pub pixels_to_world: i32,
    pub half_width: i32,
    pub bottom: i32,
    pub top: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SequenceWorldBounds {
    pub half_width: i16,
    pub bottom: i16,
    pub top: i16,
}

impl SequenceBounds {
    pub fn from_header(header: &[u8]) -> Self {
        let rd = |o: usize| {
            header
                .get(o..o + 4)
                .map_or(0, |b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        Self {
            pixels_to_world: rd(0x04),
            half_width: rd(0x28),
            bottom: rd(0x2c),
            top: rd(0x30),
        }
    }

    pub fn world_units_per_pixel_fixed(&self) -> i32 {
        self.pixels_to_world.wrapping_mul(10)
    }

    pub fn world(&self) -> SequenceWorldBounds {
        let scale = self.world_units_per_pixel_fixed();
        let conv = |v: i32| (((v >> 16) as i16 as i32).wrapping_mul(scale) >> 16) as i16;
        SequenceWorldBounds {
            half_width: conv(self.half_width),
            bottom: conv(self.bottom),
            top: conv(self.top),
        }
    }
}

impl CollectionSequence {
    pub fn mirrors_views(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn resolve_view(&self, view: usize) -> (usize, bool) {
        let nv = self.num_views as usize;
        if nv == 0 {
            return (0, false);
        }
        let view = view % nv;
        if self.mirrors_views() && view >= displayed_views(nv) {
            (mirror_view(nv, view), true)
        } else {
            (view, false)
        }
    }

    pub fn bitmap_index(&self, view: usize, frame: usize) -> Option<usize> {
        let nv = self.num_views as usize;
        let fpv = self.frames_per_view as usize;
        if nv == 0 || fpv == 0 {
            return None;
        }
        let v = view % nv;
        let f = frame % fpv;
        self.bitmap_indices.get(v * fpv + f).copied()
    }

    pub fn shadow_descriptor_index(&self, frame: usize) -> Option<usize> {
        let fpv = self.frames_per_view as usize;
        if fpv == 0 {
            return None;
        }
        let f = frame % fpv;
        self.shadow_indices
            .get(f)
            .and_then(|&idx| if idx >= 0 { Some(idx as usize) } else { None })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShadowDescriptor {
    pub flags: u32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub shadow_bitmap_index: i16,
}

impl ShadowDescriptor {
    pub fn anchor_pixels(&self) -> [f32; 2] {
        [
            self.offset_x as f32 / 65536.0,
            self.offset_y as f32 / 65536.0,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CollectionTag {
    pub bitmap_count: usize,
    pub pixels_to_world: i16,
    pub palette: Vec<[u8; 4]>,
    pub frames: Vec<BitmapFrame>,
    pub sequences: Vec<CollectionSequence>,
    pub shadow_descriptors: Vec<ShadowDescriptor>,
    #[serde(default)]
    pub color_table: Vec<[u16; 4]>,
    #[serde(default)]
    pub hue_changes: Vec<HueChange>,
    #[serde(default)]
    pub sequence_instance_flags: Vec<Vec<u32>>,
    #[serde(default)]
    pub sequence_missing_bitmaps: Vec<Vec<bool>>,
}

pub const INSTANCE_FLAG_VELOCITY_ALIGNED: u32 = 0x10;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HueChange {
    pub name: String,
    pub hue_range: (u16, u16),
    pub mask: [u8; 32],
    pub min_saturation: u16,
    pub base_hue: u16,
    pub flags: u16,
}

impl HueChange {
    pub fn affects(&self, i: usize) -> bool {
        i < 256 && (self.mask[i >> 3] >> (i & 7)) & 1 != 0
    }
}

pub fn rgb_to_hsv(rgb: [u16; 3]) -> [u16; 3] {
    let k = 65535.0_f32;
    let (r, g, b) = (rgb[0] as f32 / k, rgb[1] as f32 / k, rgb[2] as f32 / k);
    let mx = b.max(g).max(r);
    let mn = b.min(g).min(r);
    if mx == 0.0 {
        return [0, 0, (k * mx) as i32 as u16];
    }
    let delta = mx - mn;
    let s = delta / mx;
    if s == 0.0 {
        return [0, (s * k) as i32 as u16, (k * mx) as i32 as u16];
    }
    let mut h = if mx == r {
        (g - b) / delta + 0.0
    } else if mx == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h /= 6.0;
    if 0.0 > h {
        h += 1.0;
    }
    h *= 65536.0;
    [
        h as i32 as u16,
        (s * k) as i32 as u16,
        (k * mx) as i32 as u16,
    ]
}

pub fn hsv_to_rgb(hsv: [u16; 3]) -> [u16; 3] {
    let k = 65535.0_f32;
    let s = hsv[1] as f32 / k;
    let v = hsv[2] as f32 / k;
    if s == 0.0 {
        let c = (v * k) as i32 as u16;
        return [c, c, c];
    }
    let h = hsv[0] as f32 * (1.0 / 65536.0) * 6.0;
    let mut i = h as i32;
    let mut fi = i as f32;
    if fi > h {
        i -= 1;
        fi = i as f32;
    }
    let f = h - fi;
    let (sd, vd, fd) = (f64::from(s), f64::from(v), f64::from(f));
    let p = ((1.0 - sd) * vd) as f32;
    let q = ((1.0 - f64::from(s * f)) * vd) as f32;
    let t = (vd * (1.0 - sd * (1.0 - fd))) as f32;
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [
        (r * k) as i32 as u16,
        (g * k) as i32 as u16,
        (b * k) as i32 as u16,
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePermutations {
    pub collection_tag: mandrake_common::FourCC,
    pub count: i16,
    raw: Vec<u8>,
}

impl CorePermutations {
    pub const MAX: usize = 5;

    pub fn from_bytes(core: &[u8]) -> Result<Self, M2Error> {
        if core.len() < 8 {
            return Err(M2Error::TagParse(format!(
                "core tag too short: {} bytes",
                core.len()
            )));
        }
        Ok(Self {
            collection_tag: mandrake_common::FourCC::new([core[0], core[1], core[2], core[3]]),
            count: i16::from_be_bytes([core[4], core[5]]),
            raw: core.to_vec(),
        })
    }

    pub fn len(&self) -> usize {
        (self.count.max(0) as usize).min(Self::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn colors(&self, p: usize, hue_changes: usize) -> Vec<[u16; 4]> {
        let w = |o: usize| {
            self.raw
                .get(o..o + 2)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
        };
        (0..hue_changes)
            .map(|j| {
                let o = 8 + 0x40 * p + 8 * j;
                [w(o), w(o + 2), w(o + 4), w(o + 6)]
            })
            .collect()
    }
}

impl CollectionTag {
    pub const HEADER_SIZE: usize = 320;

    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        Self::from_bytes_keyed(data, BitmapKeying::Myth2)
    }

    pub fn from_bytes_tfl(data: &[u8]) -> Result<Self, M2Error> {
        Self::from_bytes_keyed(data, BitmapKeying::Tfl)
    }

    pub fn from_bytes_keyed(data: &[u8], keying: BitmapKeying) -> Result<Self, M2Error> {
        use crate::parse::{be_i16, be_u32, checked_table_range, nonneg_i32};

        if data.len() < Self::HEADER_SIZE {
            return Err(M2Error::TagParse(format!(
                "Collection data too short: expected >= {} bytes, got {}",
                Self::HEADER_SIZE,
                data.len()
            )));
        }

        let pixels_to_world = be_i16(data, 0x06).unwrap_or(1).max(1);

        let clut_count = be_u32(data, 0x40).unwrap_or(0) as usize;
        let clut_offset = be_u32(data, 0x44).unwrap_or(0) as usize;

        let mut palette = Vec::with_capacity(256);
        let pal_start = if clut_count > 0
            && Self::HEADER_SIZE
                .checked_add(clut_offset)
                .and_then(|base| base.checked_add(32 + 256 * 8))
                .is_some_and(|end| end <= data.len())
        {
            Some(Self::HEADER_SIZE + clut_offset + 32)
        } else if data.len() >= 0x160 + 256 * 8 {
            Some(0x160)
        } else {
            None
        };

        if let Some(start) = pal_start {
            for i in 0..256 {
                let off = start + i * 8;
                let alpha = if i == 0 { 0 } else { 255 };
                palette.push([data[off], data[off + 2], data[off + 4], alpha]);
            }
        } else if data.len() >= 0x800 {
            for i in 0..256 {
                palette.push([i as u8, i as u8, i as u8, if i == 0 { 0 } else { 255 }]);
            }
        }

        let bref_count = nonneg_i32(data, 0x60).ok_or_else(|| {
            M2Error::TagParse("Collection has negative bitmap reference count".into())
        })?;
        let bref_offset = nonneg_i32(data, 0x64).ok_or_else(|| {
            M2Error::TagParse("Collection has negative bitmap reference offset".into())
        })?;
        if bref_count == 0 {
            return Err(M2Error::TagParse(
                "Collection has no bitmap references".into(),
            ));
        }
        let bref_range =
            checked_table_range(Self::HEADER_SIZE, bref_offset, bref_count, 128, data.len())
                .ok_or_else(|| {
                    M2Error::TagParse("Collection bitmap reference table out of bounds".into())
                })?;

        let binst_count = nonneg_i32(data, 0x70).unwrap_or(0);
        let binst_offset = nonneg_i32(data, 0x74).unwrap_or(0);

        let seq_count = nonneg_i32(data, 0x80).unwrap_or(0);
        let seq_offset = nonneg_i32(data, 0x84).unwrap_or(0);

        let shadow_count = be_u32(data, 0x90).unwrap_or(0) as usize;
        let shadow_offset = be_u32(data, 0x94).unwrap_or(0) as usize;

        let mut shadow_descriptors = Vec::new();
        if shadow_count > 0
            && let Some(range) = checked_table_range(
                Self::HEADER_SIZE,
                shadow_offset,
                shadow_count,
                32,
                data.len(),
            )
        {
            shadow_descriptors.reserve_exact(shadow_count);
            for i in 0..shadow_count {
                let r = &data[range.start + i * 32..];
                let word = |o: usize| [r[o], r[o + 1], r[o + 2], r[o + 3]];
                shadow_descriptors.push(ShadowDescriptor {
                    flags: u32::from_be_bytes(word(0)),
                    offset_x: i32::from_be_bytes(word(4)),
                    offset_y: i32::from_be_bytes(word(8)),
                    shadow_bitmap_index: i16::from_be_bytes([r[12], r[13]]),
                });
            }
        }

        let mut instances: Vec<(i16, i16, Option<usize>)> = Vec::new();
        let mut instance_flag_words: Vec<u32> = Vec::new();
        if binst_count > 0
            && let Some(range) =
                checked_table_range(Self::HEADER_SIZE, binst_offset, binst_count, 64, data.len())
        {
            instances.reserve_exact(binst_count);
            instance_flag_words.reserve_exact(binst_count);
            for i in 0..binst_count {
                let io = range.start + i * 64;
                let reg_x = i16::from_be_bytes([data[io + 12], data[io + 13]]);
                let reg_y = i16::from_be_bytes([data[io + 14], data[io + 15]]);
                let bref_idx = i16::from_be_bytes([data[io + 28], data[io + 29]]);
                instances.push((reg_x, reg_y, usize::try_from(bref_idx).ok()));
                instance_flag_words.push(u32::from_be_bytes([
                    data[io],
                    data[io + 1],
                    data[io + 2],
                    data[io + 3],
                ]));
            }
        }

        let mut frames = Vec::with_capacity(bref_count);
        let mut total_decoded_pixels = 0usize;
        for bi in 0..bref_count {
            let ro = bref_range.start + bi * 128;
            let frame_name = decode_c_string(&data[ro..ro + 32]);
            let empty_frame = |name: String| BitmapFrame {
                index: bi,
                name,
                width: 0,
                height: 0,
                offset_x: 0,
                offset_y: 0,
                flags: 0,
                pixel_data_offset: 0,
                pixel_data_size: 0,
                pixels: Vec::new(),
                wide_pixels: Vec::new(),
            };

            let Some(boff) = nonneg_i32(data, ro + 64) else {
                frames.push(empty_frame(frame_name));
                continue;
            };
            let Some(bsize) = nonneg_i32(data, ro + 68) else {
                frames.push(empty_frame(frame_name));
                continue;
            };
            let Some(bmp_pos) = Self::HEADER_SIZE.checked_add(boff) else {
                frames.push(empty_frame(frame_name));
                continue;
            };
            let Some(bmp_hdr_end) = bmp_pos.checked_add(48) else {
                frames.push(empty_frame(frame_name));
                continue;
            };
            let Some(pixel_end) = bmp_pos.checked_add(bsize) else {
                frames.push(empty_frame(frame_name));
                continue;
            };
            if bmp_hdr_end > data.len() || pixel_end > data.len() {
                frames.push(empty_frame(frame_name));
                continue;
            }

            let w = i16::from_be_bytes([data[bmp_pos], data[bmp_pos + 1]]).max(0) as u16;
            let h = i16::from_be_bytes([data[bmp_pos + 2], data[bmp_pos + 3]]).max(0) as u16;
            let bpr = i16::from_be_bytes([data[bmp_pos + 4], data[bmp_pos + 5]]);
            let mut flags = u16::from_be_bytes([data[bmp_pos + 6], data[bmp_pos + 7]]);
            let shift: usize = if (flags & 0x0010) != 0 { 1 } else { 0 };

            let Some(pixel_count) = (w as usize).checked_mul(h as usize) else {
                return Err(M2Error::TagParse(format!(
                    "Collection bitmap {bi} dimensions {w}x{h} overflow"
                )));
            };
            if pixel_count > MAX_BITMAP_PIXELS
                || total_decoded_pixels
                    .checked_add(pixel_count)
                    .is_none_or(|t| t > MAX_COLLECTION_PIXELS)
            {
                return Err(M2Error::TagParse(format!(
                    "Collection bitmap {bi} dimensions {w}x{h} exceed maximum"
                )));
            }

            let Some(pixel_start) = bmp_hdr_end.checked_add((h as usize) * 4) else {
                frames.push(empty_frame(frame_name));
                continue;
            };
            let pixel_slice = if pixel_start <= pixel_end {
                &data[pixel_start..pixel_end]
            } else {
                &[]
            };

            if pixel_count > 0 && (flags & 0x0004) == 0 {
                let min_bytes = pixel_count.checked_shl(shift as u32);
                let avail = pixel_end.saturating_sub(bmp_hdr_end);
                if min_bytes.is_none_or(|n| n > avail) {
                    return Err(M2Error::TagParse(format!(
                        "Collection bitmap {bi} uncompressed payload truncated for {w}x{h}"
                    )));
                }
            }
            total_decoded_pixels += pixel_count;

            if keying == BitmapKeying::Tfl
                && flags & 0x001c == 0
                && pixel_start <= pixel_end
                && uncompressed_has_raw_zero(
                    pixel_slice,
                    w as usize,
                    h as usize,
                    if bpr > 0 { bpr as usize } else { w as usize },
                )
            {
                flags |= 0x0002;
            }
            let mut pixels = vec![0u8; pixel_count];
            let mut wide_pixels = if shift == 1 {
                vec![WIDE_PIXEL_OUTSIDE_RUN; pixel_count]
            } else {
                Vec::new()
            };

            if pixel_start <= pixel_end {
                let is_rle = (flags & 0x0004) != 0;
                let opaque_zero = (flags & 0x0002) == 0;
                let wu = w as usize;
                decode_bitmap_pixels(
                    pixel_slice,
                    wu,
                    h as usize,
                    bpr,
                    flags,
                    |x, y, idx, alpha| {
                        let out_idx = y * wu + x;
                        if shift == 1 {
                            wide_pixels[out_idx] = u16::from_be_bytes([alpha, idx]);
                            pixels[out_idx] = if idx == 0 && (is_rle || opaque_zero) {
                                1
                            } else {
                                idx
                            };
                        } else {
                            pixels[out_idx] = if !is_rle && idx == 0 && opaque_zero {
                                1
                            } else {
                                idx
                            };
                        }
                    },
                );
            }

            frames.push(BitmapFrame {
                index: bi,
                name: frame_name,
                width: w,
                height: h,
                offset_x: (w / 2) as i16,
                offset_y: h as i16,
                flags,
                pixel_data_offset: pixel_start,
                pixel_data_size: pixel_end.saturating_sub(pixel_start),
                pixels,
                wide_pixels,
            });
        }

        let bref_frame_count = frames.len();
        let mut keyed = vec![false; bref_frame_count];
        for inst in instances.iter_mut() {
            let (reg_x, reg_y, bref_idx) = *inst;
            let Some(bi) = bref_idx.filter(|&bi| bi < bref_frame_count) else {
                continue;
            };
            if !keyed[bi] {
                keyed[bi] = true;
                frames[bi].offset_x = reg_x;
                frames[bi].offset_y = reg_y;
            } else if (frames[bi].offset_x, frames[bi].offset_y) != (reg_x, reg_y) {
                let existing = frames[bref_frame_count..].iter().position(|f| {
                    f.pixel_data_offset == frames[bi].pixel_data_offset
                        && (f.offset_x, f.offset_y) == (reg_x, reg_y)
                });
                let new_idx = match existing {
                    Some(p) => bref_frame_count + p,
                    None => {
                        let mut copy = frames[bi].clone();
                        copy.index = frames.len();
                        copy.offset_x = reg_x;
                        copy.offset_y = reg_y;
                        frames.push(copy);
                        frames.len() - 1
                    }
                };
                inst.2 = Some(new_idx);
            }
        }

        let mut sequences = Vec::new();
        let mut sequence_instance_flags = Vec::new();
        let mut sequence_missing_bitmaps = Vec::new();
        if seq_count > 0
            && let Some(seq_range) =
                checked_table_range(Self::HEADER_SIZE, seq_offset, seq_count, 128, data.len())
        {
            sequences.reserve_exact(seq_count);
            sequence_instance_flags.reserve_exact(seq_count);
            sequence_missing_bitmaps.reserve_exact(seq_count);
            for si in 0..seq_count {
                let sref = seq_range.start + si * 128;
                let seq_name = decode_c_string(&data[sref..sref + 32]);
                let Some(sdata_off) = nonneg_i32(data, sref + 64) else {
                    continue;
                };
                let Some(shdr_pos) = Self::HEADER_SIZE.checked_add(sdata_off) else {
                    continue;
                };
                let Some(shdr_end) = shdr_pos.checked_add(64) else {
                    continue;
                };
                if shdr_end > data.len() {
                    continue;
                }
                let seq_ptw = be_i16(data, shdr_pos + 4).unwrap_or(1).max(1);
                let num_views = be_i16(data, shdr_pos + 8).unwrap_or(0).max(0) as u16;
                let frames_per_view = be_i16(data, shdr_pos + 10).unwrap_or(0).max(0) as u16;
                let ticks_per_frame = be_i16(data, shdr_pos + 12).unwrap_or(1).max(1);
                let flags = be_u32(data, shdr_pos).unwrap_or(0);
                let key_frame = be_i16(data, shdr_pos + 14).unwrap_or(0);
                let loop_frame = be_i16(data, shdr_pos + 16).unwrap_or(0);

                let stride = 46usize + 2usize * (num_views as usize);
                let Some(frame_table_bytes) = (frames_per_view as usize).checked_mul(stride) else {
                    return Err(M2Error::TagParse(format!(
                        "Collection sequence {si} frame table size overflow"
                    )));
                };
                let Some(frame_table_end) = shdr_end.checked_add(frame_table_bytes) else {
                    return Err(M2Error::TagParse(format!(
                        "Collection sequence {si} frame table offset overflow"
                    )));
                };
                if frame_table_end > data.len() {
                    return Err(M2Error::TagParse(format!(
                        "Collection sequence {si} frame table exceeds tag size"
                    )));
                }
                let total_slots = (num_views as usize) * (frames_per_view as usize);
                let mut bitmap_indices = Vec::with_capacity(total_slots);
                let mut shadow_indices = Vec::with_capacity(frames_per_view as usize);
                let mut instance_flags = Vec::with_capacity(total_slots);
                let mut missing = Vec::with_capacity(total_slots);
                for f in 0..(frames_per_view as usize) {
                    let fpos = shdr_end + f * stride;
                    let shd_idx = i16::from_be_bytes([data[fpos], data[fpos + 1]]);
                    shadow_indices.push(shd_idx);
                }
                for v in 0..(num_views as usize) {
                    for f in 0..(frames_per_view as usize) {
                        let inst_pos = shdr_end + f * stride + 46 + v * 2;
                        let inst_idx = i16::from_be_bytes([data[inst_pos], data[inst_pos + 1]]);
                        instance_flags.push(
                            usize::try_from(inst_idx)
                                .ok()
                                .and_then(|i| instance_flag_words.get(i).copied())
                                .unwrap_or(0),
                        );
                        let bmp_idx = if let Some(&(_, _, Some(bref_idx))) =
                            usize::try_from(inst_idx)
                                .ok()
                                .and_then(|i| instances.get(i))
                        {
                            missing.push(false);
                            bref_idx
                        } else {
                            missing.push(true);
                            0
                        };
                        bitmap_indices.push(bmp_idx);
                    }
                }

                sequence_instance_flags.push(instance_flags);
                sequence_missing_bitmaps.push(missing);
                sequences.push(CollectionSequence {
                    index: si,
                    name: seq_name,
                    flags,
                    pixels_to_world: seq_ptw,
                    num_views,
                    frames_per_view,
                    ticks_per_frame,
                    key_frame,
                    loop_frame,
                    bitmap_indices,
                    shadow_indices,
                    bounds: SequenceBounds::from_header(&data[shdr_pos..shdr_end]),
                });
            }
        }

        Ok(Self {
            bitmap_count: frames.len(),
            pixels_to_world,
            palette,
            frames,
            sequences,
            shadow_descriptors,
            color_table: parse_color_table(data),
            hue_changes: parse_hue_changes(data),
            sequence_instance_flags,
            sequence_missing_bitmaps,
        })
    }

    pub fn frame(&self, index: usize) -> Option<&BitmapFrame> {
        self.frames.get(index)
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn sequence(&self, index: usize) -> Option<&CollectionSequence> {
        self.sequences.get(index)
    }

    pub fn sequence_by_name(&self, name: &str) -> Option<&CollectionSequence> {
        self.sequences
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn sequence_frame(
        &self,
        sequence_index: usize,
        view: usize,
        frame: usize,
    ) -> Option<&BitmapFrame> {
        let seq = self.sequences.get(sequence_index)?;
        let bmp_idx = seq.bitmap_index(view, frame)?;
        self.frames.get(bmp_idx)
    }

    pub fn instance_flags(&self, sequence_index: usize, view: usize, frame: usize) -> u32 {
        let Some(seq) = self.sequences.get(sequence_index) else {
            return 0;
        };
        let (nv, nf) = (seq.num_views as usize, seq.frames_per_view as usize);
        if view >= nv || frame >= nf {
            return 0;
        }
        self.sequence_instance_flags
            .get(sequence_index)
            .and_then(|f| f.get(view * nf + frame))
            .copied()
            .unwrap_or(0)
    }

    pub fn has_bitmap(&self, sequence_index: usize, view: usize, frame: usize) -> bool {
        let Some(seq) = self.sequences.get(sequence_index) else {
            return true;
        };
        let (nv, nf) = (seq.num_views as usize, seq.frames_per_view as usize);
        if view >= nv || frame >= nf {
            return true;
        }
        !self
            .sequence_missing_bitmaps
            .get(sequence_index)
            .and_then(|m| m.get(view * nf + frame))
            .copied()
            .unwrap_or(false)
    }

    pub fn shadow_descriptors(&self) -> &[ShadowDescriptor] {
        &self.shadow_descriptors
    }

    pub fn shadow_frame_for_sequence(
        &self,
        sequence_index: usize,
        frame_index: usize,
    ) -> Option<&BitmapFrame> {
        let seq = self.sequences.get(sequence_index)?;
        let desc_idx = seq.shadow_descriptor_index(frame_index)?;
        let desc = self.shadow_descriptors.get(desc_idx)?;
        if desc.shadow_bitmap_index >= 0 {
            self.frames.get(desc.shadow_bitmap_index as usize)
        } else {
            None
        }
    }

    pub fn shadow_for_sequence(
        &self,
        sequence_index: usize,
        frame_index: usize,
    ) -> Option<(&ShadowDescriptor, &BitmapFrame)> {
        let seq = self.sequences.get(sequence_index)?;
        let desc = self
            .shadow_descriptors
            .get(seq.shadow_descriptor_index(frame_index)?)?;
        let bitmap = usize::try_from(desc.shadow_bitmap_index).ok()?;
        Some((desc, self.frames.get(bitmap)?))
    }

    pub fn palette(&self) -> &[[u8; 4]] {
        &self.palette
    }

    pub fn colorize_color_table(&self, colors: &[[u16; 4]]) -> Vec<[u16; 3]> {
        let mut out: Vec<[u16; 3]> = self
            .color_table
            .iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        for (hc, c) in self.hue_changes.iter().zip(colors) {
            if c[3] & 4 != 0 {
                continue;
            }
            let [ch, cs, cv] = rgb_to_hsv([c[0], c[1], c[2]]);
            for (i, e) in out.iter_mut().enumerate() {
                if !hc.affects(i) {
                    continue;
                }
                let [ih, is, iv] = rgb_to_hsv(*e);
                let h = if c[3] & 1 != 0 {
                    ch.wrapping_add(hc.base_hue).wrapping_sub(ih)
                } else {
                    ch.wrapping_add(ih).wrapping_sub(hc.base_hue)
                };
                *e = hsv_to_rgb([
                    h,
                    ((u32::from(is) * u32::from(cs)) >> 16) as u16,
                    ((u32::from(iv) * u32::from(cv)) >> 16) as u16,
                ]);
            }
        }
        out
    }

    pub fn permutation_palette(&self, core: &CorePermutations, p: usize) -> Vec<[u8; 4]> {
        self.palette_of_colors(&core.colors(p, self.hue_changes.len()))
    }

    pub fn team_colors(
        &self,
        base: &[[u16; 4]],
        primary: [u16; 4],
        secondary: [u16; 4],
    ) -> Vec<[u16; 4]> {
        let off = |c: &[u16; 4]| c[3] & 4 != 0;
        let secondary_or = |b: [u16; 4]| if off(&secondary) { b } else { secondary };
        self.hue_changes
            .iter()
            .take(8)
            .zip(base)
            .map(|(hc, &b)| {
                if off(&b) {
                    b
                } else if hc.flags & 1 != 0 {
                    if !off(&primary) {
                        primary
                    } else if hc.flags & 2 != 0 {
                        secondary_or(b)
                    } else {
                        b
                    }
                } else if hc.flags & 2 != 0 {
                    secondary_or(b)
                } else {
                    b
                }
            })
            .collect()
    }

    pub fn palette_of_colors(&self, colors: &[[u16; 4]]) -> Vec<[u8; 4]> {
        let colored = self.colorize_color_table(colors);
        let mut pal = self.palette.clone();
        for (i, c) in colored.iter().enumerate() {
            if let Some(e) = pal.get_mut(i) {
                *e = [
                    (c[0] >> 8) as u8,
                    (c[1] >> 8) as u8,
                    (c[2] >> 8) as u8,
                    if i == 0 { 0 } else { 255 },
                ];
            }
        }
        pal
    }

    pub fn has_hue_changes(&self) -> bool {
        !self.hue_changes.is_empty()
    }
}

pub const BITMAP_FLAG_RAW_COLOR_TABLE: u16 = 0x2000;

pub const BITMAP_FLAG_COLOR_ALPHA: u16 = 0x800;

fn be16_at(d: &[u8], o: usize) -> u16 {
    crate::parse::be_u16(d, o).unwrap_or(0)
}

fn parse_color_table(coll: &[u8]) -> Vec<[u16; 4]> {
    let Some(clut_off) = crate::parse::nonneg_i32(coll, 0x44) else {
        return Vec::new();
    };
    let Some(o) = CollectionTag::HEADER_SIZE.checked_add(clut_off) else {
        return Vec::new();
    };
    let n = crate::parse::be_i32(coll, o).unwrap_or(0).clamp(0, 256) as usize;
    (0..n)
        .map(|i| {
            let e = o + 0x20 + 8 * i;
            [
                be16_at(coll, e),
                be16_at(coll, e + 2),
                be16_at(coll, e + 4),
                be16_at(coll, e + 6),
            ]
        })
        .collect()
}

fn parse_hue_changes(coll: &[u8]) -> Vec<HueChange> {
    let Some(n) = crate::parse::nonneg_i32(coll, 0x50) else {
        return Vec::new();
    };
    let Some(off) = crate::parse::nonneg_i32(coll, 0x54) else {
        return Vec::new();
    };
    let Some(o) = CollectionTag::HEADER_SIZE.checked_add(off) else {
        return Vec::new();
    };
    let max_entries = coll.len().saturating_sub(o) / 0x80;
    (0..n.min(max_entries))
        .map_while(|j| {
            let start = o.checked_add(0x80usize.checked_mul(j)?)?;
            let end = start.checked_add(0x80)?;
            let r = coll.get(start..end)?;
            let mut mask = [0u8; 32];
            mask.copy_from_slice(&r[0x44..0x64]);
            Some(HueChange {
                name: decode_c_string(&r[..0x40]),
                hue_range: (be16_at(r, 0x40), be16_at(r, 0x42)),
                mask,
                min_saturation: be16_at(r, 0x64),
                base_hue: be16_at(r, 0x66),
                flags: be16_at(r, 0x68),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainTexture {
    pub width: u32,
    pub height: u32,
    pub color_index: Vec<u8>,
    pub shadow_map: Vec<u8>,
    pub palette16: [[u16; 3]; 256],
    pub color_count: usize,
    pub blood_remap: Option<Box<[u8; 256]>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandscapeShadingTable {
    pub levels: u32,
    pub channels: Vec<[u16; 3]>,
}

impl LandscapeShadingTable {
    pub fn build(
        levels: u32,
        palette16: &[[u16; 3]; 256],
        color_count: usize,
        shading: &crate::LandscapeShading,
    ) -> Self {
        let n1 = levels as i32 - 1;
        let m = shading.transition_level(levels);
        let (a, b) = (
            i32::from(shading.dark_weight),
            i32::from(shading.light_weight),
        );
        let mut channels = vec![[0u16; 3]; levels as usize * 256];
        for i in 0..levels as i32 {
            let (t, c) = if m >= i {
                let t = if m == 0 { 0 } else { (a * (m - i)) / m };
                (t, shading.dark_color)
            } else {
                let d = i32::from((n1 - m) as i16);
                ((b * (i - m)) / d, shading.light_color)
            };
            let t = i32::from(t as i16);
            let w = i32::from((256 - t) as i16);
            for (j, p) in palette16.iter().enumerate().take(color_count) {
                let ch = |q: usize| -> u16 {
                    let v = (i64::from(p[q]) * i64::from(w) + i64::from(c[q]) * i64::from(t)) >> 8;
                    v.min(0xffff) as u16
                };
                channels[(i as usize) << 8 | j] = [ch(0), ch(1), ch(2)];
            }
        }
        Self { levels, channels }
    }

    pub fn rgb8(&self) -> Vec<[u8; 3]> {
        self.channels
            .iter()
            .map(|c| [(c[0] >> 8) as u8, (c[1] >> 8) as u8, (c[2] >> 8) as u8])
            .collect()
    }

    pub fn rgb555(&self) -> Vec<u16> {
        self.channels
            .iter()
            .map(|c| ((c[0] >> 1) & 0x7c00) | ((c[1] >> 6) & 0x3e0) | (c[2] >> 11))
            .collect()
    }
}

impl TerrainTexture {
    pub const HDR: usize = 320;
    pub const SECSZ: usize = 128;
    pub const TEXHDR: usize = 52;

    pub fn from_dot256(data: &[u8], submesh_w: u32, submesh_h: u32) -> Result<Self, M2Error> {
        if data.len() < Self::HDR + 1024 {
            return Err(M2Error::TagParse(format!(
                ".256 tag data too small: expected >= {} bytes, got {}",
                Self::HDR + 1024,
                data.len()
            )));
        }

        let sections = crate::parse::nonneg_i32(data, 96)
            .filter(|s| (1..=4000).contains(s))
            .ok_or_else(|| M2Error::TagParse("Invalid section count in .256 tag".into()))?;
        let sec_off = crate::parse::nonneg_i32(data, 100)
            .ok_or_else(|| M2Error::TagParse("Invalid section offset in .256 tag".into()))?;

        let pal_pos = 0x160;
        if pal_pos + 256 * 8 > data.len() {
            return Err(M2Error::TagParse("Palette offset exceeds file size".into()));
        }
        let color_count = (crate::parse::be_u32(data, Self::HDR).unwrap_or(0) as usize).min(256);
        let mut palette16 = [[0u16; 3]; 256];
        for (i, entry) in palette16.iter_mut().enumerate().take(color_count) {
            let w = |o: usize| {
                u16::from_be_bytes([data[pal_pos + i * 8 + o], data[pal_pos + i * 8 + o + 1]])
            };
            *entry = [w(0), w(2), w(4)];
        }

        let sec_offsets = Self::section_pixel_offsets(data, sections, sec_off)?;

        let submeshes = (submesh_w as usize)
            .checked_mul(submesh_h as usize)
            .filter(|&n| n <= 2000)
            .ok_or_else(|| {
                M2Error::TagParse(format!(
                    "Invalid landscape submesh dimensions {submesh_w}x{submesh_h}"
                ))
            })?;
        let out_w = submesh_w.checked_mul(256).ok_or_else(|| {
            M2Error::TagParse(format!(
                "Landscape width overflow for submesh_w={submesh_w}"
            ))
        })?;
        let out_h = submesh_h.checked_mul(256).ok_or_else(|| {
            M2Error::TagParse(format!(
                "Landscape height overflow for submesh_h={submesh_h}"
            ))
        })?;
        let total_pixels = submeshes
            .checked_mul(256 * 256)
            .ok_or_else(|| M2Error::TagParse("Landscape pixel count overflow".into()))?;
        let mut color_index = vec![0u8; total_pixels];
        let mut shadow_map = vec![255u8; total_pixels];

        for r in 0..submesh_h as usize {
            for c in 0..submesh_w as usize {
                let k = r * submesh_w as usize + c;
                for (section, dst) in [(2 * k, &mut color_index), (2 * k + 1, &mut shadow_map)] {
                    let Some(&base) = sec_offsets.get(section) else {
                        continue;
                    };
                    if base
                        .checked_add(256 * 256)
                        .is_none_or(|end| end > data.len())
                    {
                        continue;
                    }
                    for y in 0..256 {
                        let src = &data[base + y * 256..base + y * 256 + 256];
                        let d0 = (r * 256 + y) * out_w as usize + c * 256;
                        dst[d0..d0 + 256].copy_from_slice(src);
                    }
                }
            }
        }

        let remap_count = crate::parse::be_i32(data, 0xb0).unwrap_or(0);
        let blood_remap = (remap_count != 0)
            .then(|| {
                let remap_offset = crate::parse::nonneg_i32(data, 0xb4)?;
                let start = Self::HDR.checked_add(remap_offset)?;
                let end = start.checked_add(256)?;
                data.get(start..end)
            })
            .flatten()
            .and_then(|table| <[u8; 256]>::try_from(table).ok())
            .map(Box::new);

        Ok(Self {
            width: out_w,
            height: out_h,
            color_index,
            shadow_map,
            palette16,
            color_count,
            blood_remap,
        })
    }

    fn section_pixel_offsets(
        data: &[u8],
        sections: usize,
        sec_off: usize,
    ) -> Result<Vec<usize>, M2Error> {
        let Some(st_pos) = Self::HDR.checked_add(sec_off) else {
            return Ok(Vec::new());
        };
        let max_sections = data.len().saturating_sub(st_pos) / Self::SECSZ;
        let count = sections.min(max_sections);
        let mut sec_offsets = Vec::with_capacity(count);
        for i in 0..count {
            let e_pos = st_pos + i * Self::SECSZ;
            let Some(rel_off) = crate::parse::be_u32(data, e_pos + 64).map(|v| v as usize) else {
                break;
            };
            let Some(px_base) = Self::HDR
                .checked_add(rel_off)
                .and_then(|p| p.checked_add(Self::TEXHDR))
            else {
                return Err(M2Error::TagParse(format!(
                    ".256 section {i} pixel offset overflow"
                )));
            };
            sec_offsets.push(px_base);
        }
        Ok(sec_offsets)
    }

    pub fn write_color_index(
        data: &mut [u8],
        submesh_w: u32,
        submesh_h: u32,
        color_index: &[u8],
    ) -> Result<(), M2Error> {
        let out_w = (submesh_w as usize)
            .checked_mul(256)
            .ok_or_else(|| M2Error::TagParse("landscape width overflow".into()))?;
        let out_h = (submesh_h as usize)
            .checked_mul(256)
            .ok_or_else(|| M2Error::TagParse("landscape height overflow".into()))?;
        let expected_pixels = out_w
            .checked_mul(out_h)
            .ok_or_else(|| M2Error::TagParse("landscape pixel count overflow".into()))?;
        if color_index.len() != expected_pixels {
            return Err(M2Error::TagParse(format!(
                "colour map of {} pixels, the landscape has {}x{}",
                color_index.len(),
                out_w,
                out_h
            )));
        }
        if data.len() < Self::HDR + 1024 {
            return Err(M2Error::TagParse(".256 tag data too small".into()));
        }
        let sections = crate::parse::nonneg_i32(data, 96)
            .filter(|s| (1..=4000).contains(s))
            .ok_or_else(|| M2Error::TagParse("Invalid section count in .256 tag".into()))?;
        let sec_off = crate::parse::nonneg_i32(data, 100)
            .ok_or_else(|| M2Error::TagParse("Invalid section offset in .256 tag".into()))?;
        let sec_offsets = Self::section_pixel_offsets(data, sections, sec_off)?;
        for r in 0..submesh_h as usize {
            for c in 0..submesh_w as usize {
                let k = r * submesh_w as usize + c;
                let base = *sec_offsets
                    .get(2 * k)
                    .filter(|&&b| {
                        b.checked_add(256 * 256)
                            .is_some_and(|end| end <= data.len())
                    })
                    .ok_or_else(|| {
                        M2Error::TagParse(format!(".256 tag has no colour section {}", 2 * k))
                    })?;
                for y in 0..256 {
                    let s0 = (r * 256 + y) * out_w + c * 256;
                    data[base + y * 256..base + y * 256 + 256]
                        .copy_from_slice(&color_index[s0..s0 + 256]);
                }
            }
        }
        Ok(())
    }

    pub fn shading_table(&self, shading: &crate::LandscapeShading) -> LandscapeShadingTable {
        LandscapeShadingTable::build(256, &self.palette16, self.color_count, shading)
    }

    pub fn texel(table: &[[u8; 3]], shade: u8, index: u8) -> [u8; 3] {
        table[(shade as usize) << 8 | index as usize]
    }

    pub fn shaded_rgba(&self, table: &[[u8; 3]]) -> Vec<u8> {
        let mut out = vec![255u8; self.color_index.len() * 4];
        for ((px, &i), &s) in out
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&self.color_index)
            .zip(&self.shadow_map)
        {
            px[..3].copy_from_slice(&Self::texel(table, s, i));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colorize_fixture(base_hue: u16) -> CollectionTag {
        let mut mask = [0u8; 32];
        mask[0] = 0b10;
        CollectionTag {
            bitmap_count: 0,
            pixels_to_world: 1,
            palette: vec![
                [0, 0, 0, 0],
                [0x02, 0xc1, 0x02, 255],
                [0x02, 0xc1, 0x02, 255],
            ],
            frames: Vec::new(),
            sequences: Vec::new(),
            shadow_descriptors: Vec::new(),
            color_table: vec![
                [0, 0, 0, 0],
                [0x02c1, 0xc1c1, 0x0202, 0],
                [0x02c1, 0xc1c1, 0x0202, 0],
            ],
            hue_changes: vec![HueChange {
                name: "Hue Change 1".into(),
                hue_range: (0, 0),
                mask,
                min_saturation: 0,
                base_hue,
                flags: 1,
            }],
            sequence_instance_flags: Vec::new(),
            sequence_missing_bitmaps: Vec::new(),
        }
    }

    #[test]
    fn team_colors_pick_by_hue_flags() {
        let mut coll = colorize_fixture(0x5555);
        let hc = coll.hue_changes[0].clone();
        coll.hue_changes = [1u16, 2, 3, 0, 1]
            .iter()
            .map(|&flags| HueChange {
                flags,
                ..hc.clone()
            })
            .collect();
        let (b, p, s) = ([1, 1, 1, 0], [2, 2, 2, 0], [3, 3, 3, 0]);
        let base = [b; 5];
        assert_eq!(coll.team_colors(&base, p, s), vec![p, s, p, b, p]);
        let p_off = [2, 2, 2, 4];
        assert_eq!(coll.team_colors(&base, p_off, s), vec![b, s, s, b, b]);
        let s_off = [3, 3, 3, 4];
        assert_eq!(coll.team_colors(&base, p_off, s_off), vec![b; 5]);
        let b_off = [1, 1, 1, 4];
        assert_eq!(coll.team_colors(&[b_off; 5], p, s), vec![b_off; 5]);
        assert_eq!(coll.team_colors(&base[..2], p, s), vec![p, s]);
    }

    #[test]
    fn colorize_off_mask_and_mirrored_hue() {
        let coll = colorize_fixture(0x5555);
        let raw: Vec<[u16; 3]> = coll
            .color_table
            .iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        let red = [0xffff, 0, 0];
        assert_eq!(
            coll.colorize_color_table(&[[red[0], red[1], red[2], 4]]),
            raw
        );

        let out = coll.colorize_color_table(&[[red[0], red[1], red[2], 0]]);
        assert_eq!(out[0], raw[0]);
        assert_eq!(out[2], raw[2], "entry 2 is outside the mask");
        let e = rgb_to_hsv(raw[1]);
        let c = rgb_to_hsv(red);
        let normal = c[0].wrapping_add(e[0]).wrapping_sub(0x5555);
        let mirrored = c[0].wrapping_add(0x5555).wrapping_sub(e[0]);
        assert_ne!(normal, mirrored);
        let sv = |x: u16, y: u16| ((u32::from(x) * u32::from(y)) >> 16) as u16;
        assert_eq!(out[1], hsv_to_rgb([normal, sv(e[1], c[1]), sv(e[2], c[2])]));
        let out = coll.colorize_color_table(&[[red[0], red[1], red[2], 1]]);
        assert_eq!(
            out[1],
            hsv_to_rgb([mirrored, sv(e[1], c[1]), sv(e[2], c[2])])
        );
        assert_eq!(out[2], raw[2]);
    }

    fn one_colour_palette(p: [u16; 3]) -> [[u16; 3]; 256] {
        let mut pal = [[0u16; 3]; 256];
        pal[0] = p;
        pal
    }

    #[test]
    fn transition_beyond_last_level_is_all_dark() {
        let shading = crate::LandscapeShading {
            dark_weight: 229,
            light_weight: 25,
            dark_color: [0, 0, 3932],
            light_color: [65535, 41287, 0],
            transition: 0x16666,
            fog: Default::default(),
        };
        assert_eq!(shading.transition_level(256), 356);
        let p = [40000u16, 30000, 20000];
        let t = LandscapeShadingTable::build(256, &one_colour_palette(p), 1, &shading);
        for i in [0usize, 100, 255] {
            let w = 229 * (356 - i as i64) / 356;
            let want = |q: usize| {
                ((i64::from(p[q]) * (256 - w) + i64::from(shading.dark_color[q]) * w) >> 8) as u16
            };
            assert_eq!(t.channels[i << 8], [want(0), want(1), want(2)], "level {i}");
        }
    }

    #[test]
    fn transition_at_last_level_has_no_light_branch() {
        let shading = crate::LandscapeShading {
            dark_weight: 229,
            light_weight: 0,
            dark_color: [0, 0, 3932],
            light_color: [0, 0, 0],
            transition: 0x10000,
            fog: Default::default(),
        };
        assert_eq!(shading.transition_level(256), 255);
        let p = [0xff00u16, 0x8000, 0x0100];
        let t = LandscapeShadingTable::build(256, &one_colour_palette(p), 1, &shading);
        assert_eq!(t.channels[255 << 8], p);
        assert_eq!(
            t.channels[0],
            [
                ((0xff00 * 27) >> 8) as u16,
                ((0x8000 * 27) >> 8) as u16,
                ((0x0100 * 27 + 3932 * 229) >> 8) as u16
            ]
        );
        assert_eq!(t.rgb8()[255 << 8], [0xff, 0x80, 0x01]);
    }

    #[test]
    fn light_branch_above_transition() {
        let shading = crate::LandscapeShading {
            dark_weight: 229,
            light_weight: 25,
            dark_color: [0, 0, 3932],
            light_color: [65535, 41287, 0],
            transition: 0xf333,
            fog: Default::default(),
        };
        let p = [0xffffu16, 0x2000, 0x4000];
        let t = LandscapeShadingTable::build(256, &one_colour_palette(p), 1, &shading);
        assert_eq!(t.channels[242 << 8], p);
        assert_eq!(
            t.channels[255 << 8],
            [
                0xffff,
                ((0x2000 * 231 + 41287 * 25) >> 8) as u16,
                ((0x4000 * 231) >> 8) as u16
            ]
        );
    }

    fn put16(d: &mut [u8], o: usize, v: i16) {
        d[o..o + 2].copy_from_slice(&v.to_be_bytes());
    }
    fn put32(d: &mut [u8], o: usize, v: i32) {
        d[o..o + 4].copy_from_slice(&v.to_be_bytes());
    }

    fn shared_bitmap_collection() -> Vec<u8> {
        const H: usize = CollectionTag::HEADER_SIZE;
        let (bref, bmp, inst, sref, sdata) = (0usize, 128usize, 188usize, 316usize, 444usize);
        let mut d = vec![0u8; H + sdata + 64 + 2 * 48];
        put32(&mut d, 0x60, 1);
        put32(&mut d, 0x64, bref as i32);
        put32(&mut d, 0x70, 2);
        put32(&mut d, 0x74, inst as i32);
        put32(&mut d, 0x80, 1);
        put32(&mut d, 0x84, sref as i32);
        put32(&mut d, H + bref + 64, bmp as i32);
        put32(&mut d, H + bref + 68, 48 + 8 + 4);
        put16(&mut d, H + bmp, 2);
        put16(&mut d, H + bmp + 2, 2);
        put16(&mut d, H + bmp + 4, 2);
        for (i, key_y) in [2i16, 3].into_iter().enumerate() {
            let io = H + inst + i * 64;
            put16(&mut d, io + 0x0c, 1);
            put16(&mut d, io + 0x0e, key_y);
            put16(&mut d, io + 0x1c, 0);
        }
        put32(&mut d, H + sref + 64, sdata as i32);
        put16(&mut d, H + sdata + 8, 1);
        put16(&mut d, H + sdata + 10, 2);
        for f in 0..2 {
            let fpos = H + sdata + 64 + f * 48;
            put16(&mut d, fpos, -1);
            put16(&mut d, fpos + 46, f as i16);
        }
        d
    }

    #[test]
    fn test_key_point_is_kept_per_bitmap_instance() {
        let coll = CollectionTag::from_bytes(&shared_bitmap_collection()).expect("collection");
        assert_eq!(coll.frames.len(), 2);
        assert_eq!((coll.frames[0].offset_x, coll.frames[0].offset_y), (1, 2));
        assert_eq!((coll.frames[1].offset_x, coll.frames[1].offset_y), (1, 3));
        assert_eq!(coll.frames[1].pixels, coll.frames[0].pixels);
        assert_eq!(coll.sequences[0].bitmap_indices, vec![0, 1]);
    }

    #[test]
    fn test_shadow_descriptor_layout() {
        let mut d = shared_bitmap_collection();
        let shd = d.len() - CollectionTag::HEADER_SIZE;
        d.resize(d.len() + 32, 0);
        put32(&mut d, 0x90, 1);
        put32(&mut d, 0x94, shd as i32);
        let o = CollectionTag::HEADER_SIZE + shd;
        put32(&mut d, o, 1);
        put32(&mut d, o + 4, 0x0001_c000);
        put32(&mut d, o + 8, 0x0002_0000);
        put16(&mut d, o + 12, 1);
        put16(&mut d, CollectionTag::HEADER_SIZE + 444 + 64 + 48, 0);
        let coll = CollectionTag::from_bytes(&d).expect("collection");
        assert!(coll.shadow_for_sequence(0, 0).is_none());
        let (desc, bitmap) = coll.shadow_for_sequence(0, 1).expect("shadow of frame 1");
        assert_eq!(desc.flags, 1);
        assert_eq!(desc.anchor_pixels(), [1.75, 2.0]);
        assert_eq!(desc.shadow_bitmap_index, 1);
        assert_eq!((bitmap.width, bitmap.height), (2, 2));
    }
}

#[cfg(test)]
mod sequence_tests {
    use super::*;

    #[test]
    fn displayed_view_counts_match_engine() {
        let got: Vec<usize> = [1, 2, 3, 4, 5, 8, 12, 32]
            .iter()
            .map(|&n| displayed_views(n))
            .collect();
        assert_eq!(got, vec![1, 2, 2, 3, 3, 5, 7, 17]);
    }

    #[test]
    fn mirror_view_matches_engine_formula() {
        assert_eq!(
            (5..8).map(|v| mirror_view(8, v)).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert_eq!(
            (7..12).map(|v| mirror_view(12, v)).collect::<Vec<_>>(),
            vec![5, 4, 3, 2, 1]
        );
        assert_eq!(mirror_view(5, 3), 2);
        assert_eq!(mirror_view(5, 4), 1);
    }

    fn seq(views: u16, flags: u32) -> CollectionSequence {
        CollectionSequence {
            index: 0,
            name: "t".into(),
            flags,
            pixels_to_world: 1,
            num_views: views,
            frames_per_view: 1,
            ticks_per_frame: 1,
            key_frame: -1,
            loop_frame: 0,
            bitmap_indices: (0..views as usize).collect(),
            shadow_indices: vec![-1],
            bounds: SequenceBounds::default(),
        }
    }

    #[test]
    fn sequence_bounds_world_conversion() {
        let mut hdr = [0u8; 64];
        hdr[0x04..0x08].copy_from_slice(&0x0001_0000_i32.to_be_bytes());
        hdr[0x28..0x2c].copy_from_slice(&0x000c_0000_i32.to_be_bytes());
        hdr[0x2c..0x30].copy_from_slice(&0xfff9_8000_u32.to_be_bytes());
        hdr[0x30..0x34].copy_from_slice(&0x003a_0000_i32.to_be_bytes());
        let b = SequenceBounds::from_header(&hdr);
        assert_eq!(b.world_units_per_pixel_fixed(), 0xa_0000);
        assert_eq!(
            b.world(),
            SequenceWorldBounds {
                half_width: 120,
                bottom: -70,
                top: 580
            }
        );
        let b = SequenceBounds {
            pixels_to_world: 0x1_8000,
            ..b
        };
        assert_eq!(b.world().top, 870);
    }

    #[test]
    fn resolve_view_mirrors_only_when_flagged() {
        let plain = seq(8, 2);
        assert_eq!(plain.resolve_view(6), (6, false));
        let mirrored = seq(8, 3);
        assert_eq!(mirrored.resolve_view(3), (3, false));
        assert_eq!(mirrored.resolve_view(6), (2, true));
        assert_eq!(mirrored.resolve_view(9), (1, false));
    }

    #[test]
    fn view_selection_rounds_to_nearest_of_n() {
        assert_eq!(view_for_relative_angle(0, 8, false), 0);
        assert_eq!(view_for_relative_angle(0x0fff, 8, false), 0);
        assert_eq!(view_for_relative_angle(0x1000, 8, false), 1);
        assert_eq!(view_for_relative_angle(0x4000, 8, false), 2);
        assert_eq!(view_for_relative_angle(0x8000, 8, false), 4);
        assert_eq!(view_for_relative_angle(0xf000, 8, false), 0);
        assert_eq!(view_for_relative_angle(0xefff, 8, false), 7);
        assert_eq!(view_for_relative_angle(0x4000, 8, true), 6);
        assert_eq!(view_for_relative_angle(0x1234, 1, false), 0);
    }

    fn synthetic_dot256(w: u32, h: u32) -> Vec<u8> {
        let n = (2 * w * h) as usize;
        let sec_off = 0x400;
        let pixels_rel = sec_off + n * TerrainTexture::SECSZ;
        let sec_len = TerrainTexture::TEXHDR + 256 * 256;
        let mut data = vec![0u8; TerrainTexture::HDR + pixels_rel + n * sec_len];
        data[96..100].copy_from_slice(&(n as i32).to_be_bytes());
        data[100..104].copy_from_slice(&(sec_off as i32).to_be_bytes());
        for i in 0..n {
            let rel = pixels_rel + i * sec_len;
            let e = TerrainTexture::HDR + sec_off + i * TerrainTexture::SECSZ + 64;
            data[e..e + 4].copy_from_slice(&(rel as u32).to_be_bytes());
            let p = TerrainTexture::HDR + rel + TerrainTexture::TEXHDR;
            let v = if i % 2 == 0 { i / 2 } else { 100 + i / 2 };
            data[p..p + 256 * 256].fill(v as u8);
        }
        data
    }

    #[test]
    fn write_color_index_round_trips_and_keeps_the_shading() {
        let mut data = synthetic_dot256(3, 2);
        let tex = TerrainTexture::from_dot256(&data, 3, 2).unwrap();
        assert_eq!(tex.color_index[0], 0);
        assert_eq!(tex.color_index[(256 * 768) + 256 * 2], 5, "submesh (2, 1)");
        let before = data.clone();
        let painted: Vec<u8> = (0..tex.color_index.len())
            .map(|i| ((i % 768) ^ (i / 768)) as u8)
            .collect();
        TerrainTexture::write_color_index(&mut data, 3, 2, &painted).unwrap();
        assert_eq!(data.len(), before.len());
        let after = TerrainTexture::from_dot256(&data, 3, 2).unwrap();
        assert_eq!(after.color_index, painted);
        assert_eq!(after.shadow_map, tex.shadow_map);
        assert_eq!(after.palette16, tex.palette16);
        assert!(TerrainTexture::write_color_index(&mut data, 3, 2, &painted[1..]).is_err());
        assert!(TerrainTexture::write_color_index(&mut data, 4, 2, &[0; 4 * 2 * 65536]).is_err());
    }
}
