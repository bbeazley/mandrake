use std::sync::{Arc, Mutex, OnceLock};

use mandrake_tag::{BitmapFrame, CollectionSequence};

use crate::unit_anim::{FrameVisual, ShadowSprite};

const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);
const SUN_ELEVATION_DEGREES: f32 = 35.0;
const SUN_FACING: u16 = 0x2100;
const ONE_OVER_FIXED: f32 = 1.525_878_9e-5;
const TWO_PI: f32 = 6.283_185_5;
const SEQUENCE_FLAG_STORED_VIEW: u32 = 4;
const INSTANCE_FLAG_NO_DETAIL_SHADOW: u32 = 0x10;
const OBJECT_FLAG_FLOATING: u32 = 0x800;
const OBJECT_FLAGS_KEEP_CENTRED: u32 = 0x2800;
const OBJECT_FLAG_MIRROR_X: u32 = 0x2_0000;
const OBJECT_FLAG_MIRROR_Y: u32 = 0x4_0000;

#[derive(Debug)]
pub struct ShadowSource {
    alpha: Arc<[u8]>,
    width: i32,
    height: i32,
    top: i16,
    left: i16,
    right: i16,
    bottom: i16,
    mirror_x: bool,
    mirror_y: bool,
    cache: Mutex<Vec<(u16, Option<ShadowSprite>)>>,
}

#[derive(Clone, Copy, Debug)]
pub struct ShadowObject {
    pub facing: u16,
    pub stored_view: usize,
    pub scale: u16,
    pub flags: u32,
}

pub fn coverage(frame: &BitmapFrame) -> Arc<[u8]> {
    if frame.flags & 0x10 != 0 {
        frame
            .wide_pixels
            .iter()
            .map(|&p| {
                let n = ((!u32::from(p)) >> 8 & 0xf) as u8;
                n + (n << 4)
            })
            .collect()
    } else {
        frame
            .pixels
            .iter()
            .map(|&p| if p == 0 { 0 } else { 0xff })
            .collect()
    }
}

pub fn source_for_view(
    seq: &CollectionSequence,
    frame: &BitmapFrame,
    alpha: Arc<[u8]>,
    instance_flags: u32,
    mirrored_view: bool,
) -> Option<ShadowSource> {
    if instance_flags & INSTANCE_FLAG_NO_DETAIL_SHADOW != 0 {
        return None;
    }
    let k = seq.bounds.world_units_per_pixel_fixed();
    let (w, h) = (i32::from(frame.width), i32::from(frame.height));
    let (key_x, key_y) = (i32::from(frame.offset_x), i32::from(frame.offset_y));
    let to_right = (w - key_x).wrapping_mul(k) >> 16;
    let to_left = k.wrapping_mul(key_x) >> 16;
    let (right, to_left) = if mirrored_view || instance_flags & 0x20 != 0 {
        (to_left, to_right)
    } else {
        (to_right, to_left)
    };
    Some(ShadowSource {
        alpha,
        width: w,
        height: h,
        top: (k.wrapping_mul(key_y) >> 16) as i16,
        left: -(to_left as i16),
        right: right as i16,
        bottom: -(((h - key_y).wrapping_mul(k) >> 16) as i16),
        mirror_x: (instance_flags & 1 != 0) != mirrored_view,
        mirror_y: instance_flags & 2 != 0,
        cache: Mutex::new(Vec::new()),
    })
}

pub fn sun_view(seq: &CollectionSequence, object: &ShadowObject) -> usize {
    let n = i32::from(seq.num_views).max(1);
    if seq.flags & SEQUENCE_FLAG_STORED_VIEW != 0 {
        return object.stored_view % n as usize;
    }
    let angle = if object.flags & OBJECT_FLAG_MIRROR_X != 0 {
        object.facing.wrapping_sub(SUN_FACING)
    } else {
        SUN_FACING.wrapping_sub(object.facing)
    };
    ((n / 2 + ((i32::from(angle) * n + 0x8000) >> 16)) % n) as usize
}

pub fn drawn_shadow(
    seq: &CollectionSequence,
    visual: &FrameVisual,
    object: &ShadowObject,
) -> Option<ShadowSprite> {
    let tag = visual.shadow.as_ref()?;
    let generated = visual
        .shadow_sources
        .get(sun_view(seq, object))
        .and_then(Option::as_ref)
        .and_then(|source| source.fetch(tag, object.scale, object.flags));
    Some(generated.unwrap_or_else(|| tag.clone()))
}

fn sun_azimuth() -> (f32, f32) {
    static AZIMUTH: OnceLock<(f32, f32)> = OnceLock::new();
    *AZIMUTH.get_or_init(|| {
        let turn = mandrake_math::engine::arctangent(0x5f, 100);
        (ONE_OVER_FIXED * f32::from(turn) * TWO_PI).sin_cos()
    })
}

impl ShadowSource {
    pub fn fetch(&self, tag: &ShadowSprite, scale: u16, flags: u32) -> Option<ShadowSprite> {
        let scale = scale as i16;
        let bucket = scale >> 4;
        if scale <= 0 || scale >= 0x1000 || bucket == 0 || tag.flags & 1 != 0 {
            return None;
        }
        let centred = |offset: i32, size: u16| {
            let f = offset as f32 * ONE_OVER_FIXED / f32::from(size);
            (f64::from(f) - 0.5).abs() < 0.1
        };
        if flags & OBJECT_FLAGS_KEEP_CENTRED != 0
            && centred(tag.offset_x, tag.width)
            && centred(tag.offset_y, tag.height)
        {
            return None;
        }
        let key = bucket as u16
            | u16::from(flags & OBJECT_FLAG_MIRROR_X != 0) << 8
            | u16::from(flags & OBJECT_FLAG_MIRROR_Y != 0) << 9;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, found)) = cache.iter().find(|(k, _)| *k == key) {
            return found.clone();
        }
        let made = self.generate(i32::from(bucket), flags);
        cache.push((key, made.clone()));
        made
    }

    fn generate(&self, bucket: i32, flags: u32) -> Option<ShadowSprite> {
        let (src_w, src_h) = (self.width, self.height);
        if src_w == 0 || src_h == 0 {
            return None;
        }
        let scale = (bucket as f32 + 0.5) * 0.0625;
        let scaled_bottom = f32::from(self.bottom) * scale;
        let lowered = if self.bottom < 0x40 && flags & OBJECT_FLAG_FLOATING == 0 {
            let by = -64.0 - scaled_bottom;
            if by <= 0.0 { by } else { 0.0 }
        } else {
            0.0
        };
        let per_pixel = 0.015_625_f32;
        let a = f32::from(self.left) * scale * per_pixel;
        let b = scale * f32::from(self.right) * per_pixel;
        let c = (lowered + scaled_bottom) * per_pixel;
        let d = (f32::from(self.top) * scale + lowered) * per_pixel;

        let mirror_y = self.mirror_y != (flags & OBJECT_FLAG_MIRROR_Y != 0);
        let mirror_x = self.mirror_x != (flags & OBJECT_FLAG_MIRROR_X != 0);
        let (left, right) = if flags & OBJECT_FLAG_MIRROR_X != 0 {
            (-b, -a)
        } else {
            (a, b)
        };
        let (bottom, top) = if mirror_y { (-d, -c) } else { (c, d) };

        let (sin_e, cos_e) = (DEG_TO_RAD * SUN_ELEVATION_DEGREES).sin_cos();
        let (sin_a, cos_a) = sun_azimuth();
        let height = top / sin_e;
        let width = right - left;
        let length = cos_e * height;
        let across_y = width * (0.0 - cos_a);
        let origin_y = (0.0 - cos_a) * left + sin_a * length;
        let pixels_per_unit = src_w as f32 / width;
        let across_x = width * sin_a;
        let taper = -1.0 / (top / (top - bottom));
        let origin_x = left * sin_a + cos_a * length;
        let along_y = sin_a * length * taper;
        let along_x = taper * (cos_a * length);
        let det = across_x * along_y - across_y * along_x;
        let du_dy = (-along_x) / det;
        let du_dx = along_y / det;
        let dv_dx = (-across_y) / det;
        let dv_dy = across_x / det;

        let mut min_x = if f32::MAX < origin_x {
            f32::MAX
        } else {
            origin_x
        };
        let mut min_y = if f32::MAX < origin_y {
            f32::MAX
        } else {
            origin_y
        };
        let mut max_x = if f32::MIN_POSITIVE <= origin_x {
            origin_x
        } else {
            f32::MIN_POSITIVE
        };
        let mut max_y = if f32::MIN_POSITIVE <= origin_y {
            origin_y
        } else {
            f32::MIN_POSITIVE
        };
        for (x, y) in [
            (
                along_x * 0.0 + across_x + origin_x,
                along_y * 0.0 + across_y + origin_y,
            ),
            (
                across_x * 0.0 + along_x + origin_x,
                across_y * 0.0 + along_y + origin_y,
            ),
            (origin_x + across_x + along_x, across_y + along_y + origin_y),
        ] {
            if x <= min_x {
                min_x = x;
            }
            if max_x <= x {
                max_x = x;
            }
            if y <= min_y {
                min_y = y;
            }
            if max_y <= y {
                max_y = y;
            }
        }
        let x0 = min_x.floor() as i32;
        let x1 = max_x.ceil() as i32;
        let y0 = min_y.floor() as i32;
        let y1 = max_y.ceil() as i32;
        let stride = x1 + 1 - x0;
        let rows = y1 + 1 - y0;
        if stride <= 0 || rows <= 0 {
            return None;
        }

        let (w, h) = (src_w as usize, src_h as usize);
        let alpha = self.alpha.get(..w * h)?;
        let mut sat = vec![0u32; w * h];
        for row in 0..h {
            let mut run = 0u32;
            for col in 0..w {
                run += u32::from(alpha[row * w + col]);
                sat[row * w + col] = run + if row > 0 { sat[(row - 1) * w + col] } else { 0 };
            }
        }

        let mut out = vec![0u8; stride as usize * rows as usize];
        let (mut left_u, mut left_v) = (
            x0 as f32 * du_dx + du_dy * y0 as f32 + (origin_y * along_x - origin_x * along_y) / det,
            x0 as f32 * dv_dx
                + y0 as f32 * dv_dy
                + (origin_x * across_y - origin_y * across_x) / det,
        );
        let (mut lo_x, mut hi_x, mut lo_y, mut hi_y) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        let mut at = 0;
        for y in y0..=y1 {
            let (mut u, mut v) = (left_u, left_v);
            for x in x0..=x1 {
                let value = pixel(
                    alpha_table(&sat, src_w, src_h),
                    u,
                    v,
                    (length, height, pixels_per_unit),
                    (mirror_x, mirror_y),
                );
                u += du_dx;
                v += dv_dx;
                out[at] = value;
                at += 1;
                if value != 0 {
                    lo_x = lo_x.min(x);
                    hi_x = hi_x.max(x);
                    lo_y = lo_y.min(y);
                    hi_y = hi_y.max(y);
                }
            }
            left_u += du_dy;
            left_v += dv_dy;
        }
        if lo_x > hi_x {
            return None;
        }

        let (out_w, out_h) = ((hi_x - lo_x + 1) as usize, (hi_y - lo_y + 1) as usize);
        let mut pixels = Vec::with_capacity(out_w * out_h);
        for row in 0..out_h {
            let start = (lo_y - y0) as usize * stride as usize
                + row * stride as usize
                + (lo_x - x0) as usize;
            pixels.extend_from_slice(&out[start..start + out_w]);
        }
        Some(ShadowSprite {
            flags: 0,
            offset_x: (-lo_x as f32 * 65536.0) as i32,
            offset_y: (-lo_y as f32 * 65536.0) as i32,
            width: out_w as u16,
            height: out_h as u16,
            pixels: pixels.into(),
        })
    }
}

struct AlphaTable<'a> {
    sat: &'a [u32],
    w: i32,
    h: i32,
}

fn alpha_table(sat: &[u32], w: i32, h: i32) -> AlphaTable<'_> {
    AlphaTable { sat, w, h }
}

fn pixel(
    table: AlphaTable<'_>,
    u: f32,
    v: f32,
    (length, height, pixels_per_unit): (f32, f32, f32),
    (mirror_x, mirror_y): (bool, bool),
) -> u8 {
    let AlphaTable { sat, w, h } = table;
    let t = (length * (1.0 - v)) / 12.0;
    let (blur, darkness) = if t >= 0.0 {
        if t <= 1.0 {
            (height * t, t * -0.4 + 1.0)
        } else {
            (height, 0.6)
        }
    } else {
        (height * 0.0, 1.0)
    };
    let blur = blur * 0.0625;
    let clamped = if 0.5 <= blur {
        if 1.5 < blur { 1.5 } else { blur }
    } else {
        0.5
    };
    let radius = ((f64::from(clamped * pixels_per_unit) + 0.5) as i32).max(1);
    let side = radius * 2 + 1;
    let su = if mirror_x { 1.0 - u } else { u };
    let sv = if mirror_y { 1.0 - v } else { v };

    let cx = (su * w as f32) as i32;
    let from_x = cx - radius;
    let lo_x = from_x.max(0);
    let hi_x = (cx + radius + 1).min(w);
    let mut covered = 0.0;
    if lo_x < hi_x {
        let cy = (sv * h as f32) as i32;
        let from_y = cy - radius;
        let lo_y = from_y.max(0);
        let hi_y = (cy + radius + 1).min(h);
        if lo_y < hi_y {
            let at = |row: i32, col: i32| sat[(row * w + col) as usize];
            let mut sum = at(hi_y - 1, hi_x - 1);
            if from_y > 0 {
                sum = sum.wrapping_sub(at(lo_y - 1, hi_x - 1));
            }
            if from_x > 0 {
                sum = sum.wrapping_sub(at(hi_y - 1, lo_x - 1));
                if from_y > 0 {
                    sum = sum.wrapping_add(at(lo_y - 1, lo_x - 1));
                }
            }
            covered = sum as f32;
        }
    }
    let f = covered / (side * side * 0xff) as f32;
    let shaded = if f >= 0.0 {
        if f <= 1.0 { f * darkness } else { darkness }
    } else {
        darkness * 0.0
    };
    ((darkness * shaded * 0.8 + 0.0) * 255.0) as i32 as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sequence(views: u16, flags: u32) -> CollectionSequence {
        CollectionSequence {
            index: 0,
            name: "s".into(),
            flags,
            pixels_to_world: 1,
            num_views: views,
            frames_per_view: 1,
            ticks_per_frame: 1,
            key_frame: 0,
            loop_frame: 0,
            bitmap_indices: vec![0; views as usize],
            shadow_indices: vec![0],
            bounds: mandrake_tag::SequenceBounds {
                pixels_to_world: 0x10000,
                ..Default::default()
            },
        }
    }

    fn frame(w: u16, h: u16, key: (i16, i16)) -> BitmapFrame {
        BitmapFrame {
            index: 0,
            name: String::new(),
            width: w,
            height: h,
            offset_x: key.0,
            offset_y: key.1,
            flags: 0,
            pixel_data_offset: 0,
            pixel_data_size: 0,
            pixels: vec![1; usize::from(w) * usize::from(h)],
            wide_pixels: Vec::new(),
        }
    }

    fn tag(offset: i32) -> ShadowSprite {
        ShadowSprite {
            flags: 0,
            offset_x: offset,
            offset_y: offset,
            width: 4,
            height: 4,
            pixels: vec![0x80; 16].into(),
        }
    }

    fn source(f: &BitmapFrame) -> ShadowSource {
        source_for_view(&sequence(1, 0), f, coverage(f), 0, false).unwrap()
    }

    #[test]
    fn the_rectangle_is_the_engines() {
        let f = frame(58, 51, (29, 37));
        let s = source(&f);
        assert_eq!((s.top, s.left, s.right, s.bottom), (370, -290, 290, -140));
        let f = frame(60, 51, (29, 37));
        let m = source_for_view(&sequence(1, 0), &f, coverage(&f), 0, true).unwrap();
        assert_eq!((m.left, m.right), (-310, 290), "a mirrored view swaps them");
        assert!(m.mirror_x);
    }

    #[test]
    fn coverage_of_16_bit_pixels_is_the_inverted_nibble() {
        let mut f = frame(3, 1, (0, 0));
        f.flags = 0x10;
        f.wide_pixels = vec![
            0x0012,
            0x0f34,
            mandrake_tag::collection::WIDE_PIXEL_OUTSIDE_RUN,
        ];
        assert_eq!(&*coverage(&f), &[0xff, 0, 0]);
    }

    #[test]
    fn the_sun_view_is_half_a_turn_from_the_angle() {
        let object = |facing, flags| ShadowObject {
            facing,
            stored_view: 3,
            scale: 0x100,
            flags,
        };
        let seq = sequence(16, 0);
        assert_eq!(sun_view(&seq, &object(0x2100, 0)), 8);
        assert_eq!(sun_view(&seq, &object(0x3100, 0)), 7);
        assert_eq!(sun_view(&seq, &object(0x3100, OBJECT_FLAG_MIRROR_X)), 9);
        assert_eq!(sun_view(&sequence(16, 4), &object(0x2100, 0)), 3);
    }

    #[test]
    fn it_needs_a_scale_and_a_dark_descriptor() {
        let f = frame(16, 32, (8, 30));
        let s = source(&f);
        assert!(s.fetch(&tag(0), 0x100, 0).is_some());
        assert!(s.fetch(&tag(0), 0, 0).is_none());
        assert!(s.fetch(&tag(0), 0xf, 0).is_none(), "scale >> 4 is 0");
        assert!(s.fetch(&tag(0), 0x1000, 0).is_none());
        let mut light = tag(0);
        light.flags = 1;
        assert!(s.fetch(&light, 0x100, 0).is_none());
    }

    #[test]
    fn a_centred_bitmap_stays_only_for_the_flagged_objects() {
        let f = frame(16, 32, (8, 30));
        let s = source(&f);
        let centred = tag(2 << 16);
        assert!(s.fetch(&centred, 0x100, 0).is_some());
        assert!(s.fetch(&centred, 0x100, 0x800).is_none());
        assert!(s.fetch(&centred, 0x100, 0x2000).is_none());
        assert!(s.fetch(&tag(0), 0x100, 0x2000).is_some(), "off centre");
    }

    #[test]
    fn the_cache_key_is_the_engines() {
        let f = frame(16, 32, (8, 30));
        let s = source(&f);
        let a = s.fetch(&tag(0), 0x100, 0).unwrap();
        assert_eq!(s.fetch(&tag(0), 0x10f, 0x800).unwrap(), a);
        assert_eq!(s.cache.lock().unwrap().len(), 1);
        let b = s.fetch(&tag(0), 0x110, 0).unwrap();
        assert_ne!(a, b);
        assert_eq!(s.cache.lock().unwrap().len(), 2);
    }

    #[test]
    fn an_empty_sprite_keeps_the_descriptors_bitmap() {
        let mut f = frame(16, 32, (8, 30));
        f.pixels.fill(0);
        let s = source(&f);
        assert!(s.fetch(&tag(0), 0x100, 0).is_none());
        assert_eq!(s.cache.lock().unwrap().len(), 1);

        let mut visual = FrameVisual {
            shadow: Some(tag(0)),
            ..Default::default()
        };
        visual.shadow_sources.push(Some(Arc::new(s)));
        let object = ShadowObject {
            facing: 0,
            stored_view: 0,
            scale: 0x100,
            flags: 0,
        };
        assert_eq!(
            drawn_shadow(&sequence(1, 0), &visual, &object),
            Some(tag(0))
        );
        visual.shadow = None;
        assert_eq!(drawn_shadow(&sequence(1, 0), &visual, &object), None);
    }
}
