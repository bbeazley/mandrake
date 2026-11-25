use std::cell::RefCell;

use mandrake_math::engine::{arctangent, isqrt};
use mandrake_tag::Rect;
use mandrake_tag::collection::{hsv_to_rgb, rgb_to_hsv};
use mandrake_tag::font::Bitmap32;

use super::list::{fill_rect, frame_rect};
use super::{Click, ControlProc, DrawContext, ItemState};

pub struct ColorPicker {
    color: [u16; 4],
    wheel: Rect,
    slider: Rect,
    swatch: Rect,
    wheel_cache: RefCell<Option<(u16, Vec<u32>)>>,
}

impl ColorPicker {
    pub fn new(item: &mut ItemState) -> Self {
        let r = item.rect;
        let side = r.height().max(r.width());
        item.rect.bottom = r.top + side;
        item.rect.right = r.left + side;
        let s = if side - 3 >= 0 { side - 3 } else { side };
        let q = s >> 2;
        let at = |top, left, bottom, right| {
            Rect::new(top + r.top, left + r.left, bottom + r.top, right + r.left)
        };
        Self {
            color: [0; 4],
            wheel: at(0, 0, 3 * q, 3 * q),
            slider: at(0, 3 * q + 3, 3 * q, 4 * q + 3),
            swatch: at(3 * q + 3, 0, 4 * q + 3, 4 * q + 3),
            wheel_cache: RefCell::new(None),
        }
    }

    pub const fn color(&self) -> [u16; 4] {
        self.color
    }

    pub fn set_color(&mut self, item: &mut ItemState, color: [u16; 4]) {
        self.color = color;
        item.dirty = true;
    }

    fn hsv(&self) -> [u16; 3] {
        rgb_to_hsv([self.color[0], self.color[1], self.color[2]])
    }
}

fn click_wheel(rect: Rect, point: (i32, i32)) -> Option<(u16, u16)> {
    let r = rect.height().min(rect.width()) >> 1;
    let dx = point.0 - rect.left - r;
    let dy = point.1 - rect.top - r;
    let distance = f64::from(dx * dx + dy * dy).sqrt() as i32;
    if r <= distance {
        return None;
    }
    Some((arctangent(dx, dy), ((distance << 16) / r) as u16))
}

fn click_slider(rect: Rect, point: (i32, i32)) -> u16 {
    let (w, h) = (rect.width(), rect.height());
    let v = if h < w {
        (point.0 - rect.left) * 0xffff / w
    } else {
        (point.1 - rect.top) * 0xffff / h
    };
    v as u16
}

fn pixel(rgb: [u16; 3]) -> u32 {
    let [r, g, b] = rgb.map(|v| u32::from(v >> 8));
    0xff00_0000 | r << 16 | g << 8 | b
}

fn put(bitmap: &mut Bitmap32, x: i32, y: i32, c: u32) {
    if (0..bitmap.width).contains(&x) && (0..bitmap.height).contains(&y) {
        bitmap.set(x, y, c);
    }
}

fn rasterize_wheel(rect: Rect, value: u16) -> Vec<u32> {
    let d = rect.width().min(rect.height()).max(0);
    let r = d >> 1;
    let mut pixels = vec![0u32; (d * d) as usize];
    for row in 0..d {
        let dy = row - r;
        let row_base = (row * d) as usize;
        for dx in -r..d - r {
            let distance = isqrt((dx * dx + dy * dy) as u32) as i32;
            if r <= distance {
                continue;
            }
            let hue = arctangent(dx, dy);
            let saturation = ((distance << 16) / r) as u16;
            let rgb = hsv_to_rgb([hue, saturation, value]);
            pixels[row_base + (dx + r) as usize] = pixel(rgb);
        }
    }
    pixels
}

fn blit_wheel(bitmap: &mut Bitmap32, rect: Rect, pixels: &[u32]) {
    let d = rect.width().min(rect.height()).max(0);
    for row in 0..d {
        let y = rect.top + row;
        if !(0..bitmap.height).contains(&y) {
            continue;
        }
        let row_base = (row * d) as usize;
        for col in 0..d {
            let c = pixels[row_base + col as usize];
            if c != 0 {
                let x = rect.left + col;
                if (0..bitmap.width).contains(&x) {
                    bitmap.set(x, y, c);
                }
            }
        }
    }
}

#[cfg(test)]
fn draw_wheel(bitmap: &mut Bitmap32, rect: Rect, value: u16) {
    let pixels = rasterize_wheel(rect, value);
    blit_wheel(bitmap, rect, &pixels);
}

fn draw_slider(bitmap: &mut Bitmap32, rect: Rect, hue: u16, saturation: u16) {
    let (w, h) = (rect.width(), rect.height());
    let n = w.max(h);
    for i in 0..n {
        let value = (i64::from(i) * 0xffff / i64::from(n)) as u16;
        let c = pixel(hsv_to_rgb([hue, saturation, value]));
        if h < w {
            for y in rect.top..=rect.bottom {
                put(bitmap, rect.left + i, y, c);
            }
        } else {
            for x in rect.left..=rect.right {
                put(bitmap, x, rect.top + i, c);
            }
        }
    }
}

impl ControlProc for ColorPicker {
    fn draw(&self, _item: &ItemState, _ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let [hue, saturation, value] = self.hsv();
        let mut cache = self.wheel_cache.borrow_mut();
        let pixels = match cache.as_ref() {
            Some((cached_value, pixels)) if *cached_value == value => pixels,
            _ => {
                *cache = Some((value, rasterize_wheel(self.wheel, value)));
                &cache.as_ref().unwrap().1
            }
        };
        blit_wheel(bitmap, self.wheel, pixels);
        draw_slider(bitmap, self.slider, hue, saturation);
        let rgb = [self.color[0], self.color[1], self.color[2]];
        fill_rect(bitmap, rgb, self.swatch, 0);
        frame_rect(bitmap, [0; 3], self.swatch, 0);
    }

    fn click(&mut self, item: &mut ItemState, click: &Click) -> bool {
        if !click.held {
            return false;
        }
        let [mut hue, mut saturation, mut value] = self.hsv();
        let changed = if self.wheel.contains(click.point.0, click.point.1) {
            click_wheel(self.wheel, click.point).map(|(h, s)| {
                hue = h;
                saturation = s;
            })
        } else if self.slider.contains(click.point.0, click.point.1) {
            value = click_slider(self.slider, click.point);
            Some(())
        } else {
            None
        };
        if changed.is_none() {
            return false;
        }
        let [r, g, b] = hsv_to_rgb([hue, saturation, value]);
        self.color = [r, g, b, self.color[3]];
        item.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_common::FourCC;

    fn pre_myth_951_isqrt(v: u32) -> i32 {
        let v = u64::from(v);
        let mut r = (v as f64).sqrt() as u64;
        while r * r > v {
            r -= 1;
        }
        while (r + 1) * (r + 1) <= v {
            r += 1;
        }
        if v - r * r > r {
            r += 1;
        }
        r as i32
    }

    #[test]
    fn isqrt_matches_the_old_copy() {
        for k in 0u64..=0x1_0000 {
            for v in [k * k, k * k + k, k * k + k + 1]
                .into_iter()
                .flat_map(|v| [v.wrapping_sub(1), v, v + 1])
            {
                if let Ok(v) = u32::try_from(v) {
                    assert_eq!(isqrt(v) as i32, pre_myth_951_isqrt(v), "isqrt({v})");
                }
            }
        }
    }

    #[test]
    #[ignore = "exhaustive, 2^32 inputs"]
    fn isqrt_matches_the_old_copy_everywhere() {
        let threads = std::thread::available_parallelism().map_or(8, |n| n.get()) as u64;
        std::thread::scope(|s| {
            for t in 0..threads {
                s.spawn(move || {
                    for v in (t..=u64::from(u32::MAX)).step_by(threads as usize) {
                        let v = v as u32;
                        assert_eq!(isqrt(v) as i32, pre_myth_951_isqrt(v), "isqrt({v})");
                    }
                });
            }
        });
    }

    #[test]
    fn the_click_truncates_the_distance_where_the_drawing_rounds_it() {
        let rect = Rect::new(0, 0, 20, 20);
        assert_eq!(isqrt(9 * 9 + 4 * 4), 10);
        assert_eq!(
            click_wheel(rect, (10 + 9, 10 + 4)),
            Some((arctangent(9, 4), ((9 << 16) / 10) as u16))
        );
        let mut bitmap = Bitmap32::new(20, 20, 0);
        draw_wheel(&mut bitmap, rect, 0xffff);
        assert_eq!(bitmap.get(10 + 9, 10 + 4), 0, "drawn outside the circle");
        assert_ne!(bitmap.get(10 + 8, 10 + 4), 0);
        assert_eq!(click_wheel(rect, (10 + 6, 10 + 8)), None);
    }

    fn item(rect: Rect) -> ItemState {
        ItemState {
            id: 6,
            kind: FourCC(*b"colr"),
            rect,
            flags: 0,
            pressed: false,
            dirty: false,
        }
    }

    #[test]
    fn nesp_primary_splits_in_quarters() {
        let mut it = item(Rect::new(149, 37, 232, 120));
        let p = ColorPicker::new(&mut it);
        assert_eq!(it.rect, Rect::new(149, 37, 232, 120));
        assert_eq!(p.wheel, Rect::new(149, 37, 209, 97));
        assert_eq!(p.slider, Rect::new(149, 100, 209, 120));
        assert_eq!(p.swatch, Rect::new(212, 37, 232, 120));
    }

    #[test]
    fn the_wheel_centre_is_grey_and_the_slider_sets_the_value() {
        let mut it = item(Rect::new(0, 0, 83, 83));
        let mut p = ColorPicker::new(&mut it);
        p.color = [0xffff, 0, 0, 0];
        let press = |point| Click {
            point,
            held: true,
            shift: false,
            double: false,
            now_tick: 0,
        };
        assert!(p.click(&mut it, &press((30, 30))));
        assert_eq!(p.color, [0xffff, 0xffff, 0xffff, 0]);
        assert!(p.click(&mut it, &press((70, 0))));
        assert_eq!(&p.color[..3], &[0, 0, 0]);
        assert!(!p.click(&mut it, &press((0, 0))));
    }

    #[test]
    fn wheel_cache_matches_uncached_draw_and_tracks_value() {
        let mut it = item(Rect::new(0, 0, 83, 83));
        let mut p = ColorPicker::new(&mut it);
        let res = crate::resources::Resources::from_source(|_, _| None);
        let ctx = DrawContext {
            res: &res,
            now_tick: 0,
            focused: false,
        };
        p.set_color(&mut it, [0xffff, 0x8000, 0x4000, 0]);
        let mut a = Bitmap32::new(100, 100, 0xff00_0000);
        let mut b = Bitmap32::new(100, 100, 0xff00_0000);
        p.draw(&it, &ctx, &mut a);
        assert_eq!(
            p.wheel_cache.borrow().as_ref().map(|(v, _)| *v),
            Some(0xffff)
        );
        p.draw(&it, &ctx, &mut b);
        assert_eq!(a.pixels, b.pixels);
        p.set_color(&mut it, [0x4000, 0x2000, 0x1000, 0]);
        let mut c = Bitmap32::new(100, 100, 0xff00_0000);
        p.draw(&it, &ctx, &mut c);
        assert_eq!(
            p.wheel_cache.borrow().as_ref().map(|(v, _)| *v),
            Some(0x4000)
        );
        assert_ne!(a.pixels, c.pixels);
    }
}
