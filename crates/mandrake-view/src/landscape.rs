use mandrake_tag::collection::TerrainTexture;

pub const WORLD_UNITS_PER_PIXEL_SHIFT: u32 = 6;
const SUBMESH_PIXELS: u32 = 256;

pub const DARKEN_FLOOR: i32 = 0x40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayMode {
    Darken,
    Lighten,
    Min,
}

impl OverlayMode {
    pub fn from_descriptor_flags(flags: u32) -> Self {
        if flags & 1 != 0 {
            Self::Lighten
        } else {
            Self::Darken
        }
    }

    fn drawn_last(self) -> bool {
        self == Self::Lighten
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadeOverlay<'a> {
    pub x: i32,
    pub y: i32,
    pub pixels: &'a [u8],
    pub width: i16,
    pub height: i16,
    pub row_bytes: i16,
    pub intensity: u16,
    pub mode: OverlayMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    #[cfg(test)]
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x as i32
            && y >= self.y as i32
            && x < (self.x + self.width) as i32
            && y < (self.y + self.height) as i32
    }

    fn intersects(&self, o: &PixelRect) -> bool {
        self.x < o.x + o.width
            && o.x < self.x + self.width
            && self.y < o.y + o.height
            && o.y < self.y + self.height
    }
}

impl ShadeOverlay<'_> {
    pub fn pixel_rect(&self, map_w: u32, map_h: u32) -> Option<PixelRect> {
        let x0 = self.x >> WORLD_UNITS_PER_PIXEL_SHIFT;
        let y0 = self.y >> WORLD_UNITS_PER_PIXEL_SHIFT;
        let x1 = (x0 + i32::from(self.width)).min(map_w as i32);
        let y1 = (y0 + i32::from(self.height)).min(map_h as i32);
        let (x0, y0) = (x0.max(0), y0.max(0));
        (x1 > x0 && y1 > y0).then(|| PixelRect {
            x: x0 as u32,
            y: y0 as u32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        })
    }

    pub fn rasterize(&self, buf: &mut [u8], window: PixelRect) {
        self.rasterize_with(buf, window, &mut Vec::new());
    }

    fn rasterize_with(&self, buf: &mut [u8], window: PixelRect, row_carry: &mut Vec<i32>) {
        let (w, h) = (i32::from(self.width), i32::from(self.height));
        if w <= 0 || h <= 0 {
            return;
        }
        let px = self.x >> WORLD_UNITS_PER_PIXEL_SHIFT;
        let py = self.y >> WORLD_UNITS_PER_PIXEL_SHIFT;
        let clip = |first: i32, origin: u32, size: u32, len: i32| -> (i32, i32) {
            let lo = i64::from(origin) - i64::from(first);
            let hi = lo + i64::from(size);
            (
                lo.clamp(0, i64::from(len)) as i32,
                hi.clamp(0, i64::from(len)) as i32,
            )
        };
        let (c0, c1) = clip(px, window.x, window.width, w);
        let (r0, r1) = clip(py, window.y, window.height, h);
        if c0 >= c1 || r0 >= r1 {
            return;
        }
        let row_bytes = self.row_bytes as isize;
        let src = |r: i32, c: i32| -> i32 {
            let o = r as isize * row_bytes + c as isize;
            usize::try_from(o)
                .ok()
                .and_then(|o| self.pixels.get(o))
                .map_or(0, |&p| i32::from(p))
        };
        let stride = window.width as usize;
        let row_start = |r: i32| -> usize {
            (py + r - window.y as i32) as usize * stride + (px + c0 - window.x as i32) as usize
        };
        let visible = (c1 - c0) as usize;
        match self.mode {
            OverlayMode::Min => {
                for r in r0..r1 {
                    let o = row_start(r);
                    for (c, d) in (c0..c1).zip(&mut buf[o..o + visible]) {
                        *d = i32::from(*d).min(src(r, c)) as u8;
                    }
                }
            }
            OverlayMode::Darken | OverlayMode::Lighten => {
                let fx = !self.x & 0x3f;
                let fy = !self.y & 0x3f;
                let intensity = i32::from(self.intensity);
                let lighten = self.mode == OverlayMode::Lighten;
                let (cs, rs) = ((c0 - 1).max(0), (r0 - 1).max(0));
                row_carry.clear();
                row_carry.resize((c1 - cs) as usize, 0);
                for r in rs..r1 {
                    let mut col_carry = 0i32;
                    let o = row_start(r.max(r0));
                    let out = &mut buf[o..o + visible];
                    for c in cs..c1 {
                        let a = (src(r, c) * intensity) >> 16;
                        let hs = fx * a;
                        let hv = hs + col_carry;
                        col_carry = 63 * a - hs;
                        let vs = fy * hv;
                        let carry = &mut row_carry[(c - cs) as usize];
                        let v = (*carry + vs) >> 12;
                        *carry = 63 * hv - vs;
                        if r < r0 || c < c0 {
                            continue;
                        }
                        let d = &mut out[(c - c0) as usize];
                        let shade = i32::from(*d);
                        if lighten {
                            *d = (shade + v).min(255) as u8;
                        } else if a != 0 {
                            *d = (((255 - v) * shade) >> 8).max(DARKEN_FLOOR).min(shade) as u8;
                        }
                    }
                }
            }
        }
    }
}

pub fn map_overlay_intensity(
    height: i32,
    light: bool,
    transfer_mode: i8,
    transfer_phase: i16,
) -> Option<(u16, i32)> {
    map_overlay_intensity_of(0xffff, height, light, transfer_mode, transfer_phase)
}

pub fn map_overlay_intensity_of(
    opacity: u16,
    height: i32,
    light: bool,
    transfer_mode: i8,
    transfer_phase: i16,
) -> Option<(u16, i32)> {
    let caller = i32::from(opacity);
    let (base, shift) = if height > 0 {
        let h = height << 16;
        let (fade, shift) = if light {
            let h = if h < 0 { h + 0x3ff } else { h };
            (h >> 10, 0)
        } else {
            let fade = ((i64::from(h) * 0x2aaa_aaab) >> 32) as i32;
            let fade = (fade >> 9) - (h >> 31);
            (fade, ((fade * 3) << 9) >> 16)
        };
        ((0x10000 - fade).min(0xffff), shift)
    } else if height < 0 {
        let h = (-height) << 16;
        if h <= 0x1ff {
            (0xffff, 0)
        } else {
            let h = if h < 0 { h + 0x1ff } else { h };
            (0x10000 - (h >> 9), 0)
        }
    } else {
        (0xffff, 0)
    };
    let mut intensity = (((caller >> 8) & 0xff) * base) >> 8;
    if intensity <= 0 {
        return None;
    }
    let phase = i32::from(transfer_phase);
    let hi = (intensity as u16 >> 8) as i32;
    intensity = match transfer_mode {
        1 => hi * ((!phase) >> 8),
        2 => hi * (phase >> 8),
        5 => hi * (0xff - ((0xffff - (phase >> 1)) >> 8)),
        6 => hi * (0xff - (((phase >> 1) + 0x7fff) >> 8)),
        _ => intensity,
    };
    Some((intensity as u16, shift))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TflShadowPlace {
    pub above_ground: i16,
    pub drawn_height: i32,
    pub in_water_top: Option<i32>,
    pub transfer: (u8, u16),
}

pub fn tfl_shadow_intensity(place: &TflShadowPlace) -> Option<u16> {
    let fade = match place.in_water_top {
        Some(top) if top <= 0 => return None,
        Some(_) if (place.drawn_height as i16) < 0 => {
            (i32::from(place.drawn_height as i16) + 0x200) << 7
        }
        Some(_) => (0x800 - i32::from(place.above_ground.max(0))) << 5,
        None => (0x800 - i32::from(place.above_ground)) << 5,
    };
    let intensity = if fade >= 0x10000 {
        0xffff
    } else if fade <= 0 {
        return None;
    } else {
        fade
    };
    let (mode, ratio) = place.transfer;
    let ratio = u32::from(ratio);
    let scaled = |r: u32| (((r as i32) >> 8).wrapping_mul(intensity >> 8)) as u16;
    Some(match mode {
        1 => scaled(!ratio),
        2 => scaled(ratio),
        _ => intensity as u16,
    })
}

pub const MYTH2_FOOTSTEP_SHADE: u8 = 0x50;

#[derive(Clone, Debug)]
pub struct LandscapeTexels {
    pub width: u32,
    pub height: u32,
    color_index: Vec<u8>,
    shade: Vec<u8>,
    table: Vec<[u8; 3]>,
    palette16: [[u16; 3]; 256],
    color_count: usize,
    blend: Option<Vec<u8>>,
    blood_remap: Option<Box<[u8; 256]>>,
    scratch: RegionScratch,
}

const BUCKET_SHIFT: u32 = 6;

#[derive(Clone, Debug, Default)]
struct RegionScratch {
    shade: Vec<u8>,
    rgba: Vec<u8>,
    carry: Vec<i32>,
    rects: Vec<Option<PixelRect>>,
    columns: u32,
    starts: Vec<u32>,
    members: Vec<u32>,
    hits: Vec<u32>,
}

impl RegionScratch {
    fn buckets(rect: &PixelRect) -> (std::ops::RangeInclusive<u32>, std::ops::RangeInclusive<u32>) {
        (
            rect.x >> BUCKET_SHIFT..=(rect.x + rect.width - 1) >> BUCKET_SHIFT,
            rect.y >> BUCKET_SHIFT..=(rect.y + rect.height - 1) >> BUCKET_SHIFT,
        )
    }

    fn bucket(&mut self, overlays: &[ShadeOverlay<'_>], map_w: u32, map_h: u32) {
        self.rects.clear();
        self.rects
            .extend(overlays.iter().map(|o| o.pixel_rect(map_w, map_h)));
        self.columns = (map_w >> BUCKET_SHIFT) + 1;
        let columns = self.columns;
        let count = (columns * ((map_h >> BUCKET_SHIFT) + 1)) as usize;
        self.starts.clear();
        self.starts.resize(count + 2, 0);
        for rect in self.rects.iter().flatten() {
            let (xs, ys) = Self::buckets(rect);
            for by in ys {
                for bx in xs.clone() {
                    self.starts[(by * columns + bx) as usize + 2] += 1;
                }
            }
        }
        for b in 1..self.starts.len() {
            self.starts[b] += self.starts[b - 1];
        }
        self.members.clear();
        self.members.resize(self.starts[count + 1] as usize, 0);
        for (i, rect) in self.rects.iter().enumerate() {
            let Some(rect) = rect else { continue };
            let (xs, ys) = Self::buckets(rect);
            for by in ys {
                for bx in xs.clone() {
                    let at = &mut self.starts[(by * columns + bx) as usize + 1];
                    self.members[*at as usize] = i as u32;
                    *at += 1;
                }
            }
        }
    }

    fn find_hits(&mut self, rect: &PixelRect) {
        self.hits.clear();
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let (xs, ys) = Self::buckets(rect);
        let several = xs.start() != xs.end() || ys.start() != ys.end();
        for by in ys {
            for bx in xs.clone() {
                let b = (by * self.columns + bx) as usize;
                let (from, to) = (self.starts[b] as usize, self.starts[b + 1] as usize);
                let rects = &self.rects;
                self.hits.extend(
                    self.members[from..to]
                        .iter()
                        .filter(|&&i| rects[i as usize].is_some_and(|r| r.intersects(rect))),
                );
            }
        }
        if several {
            self.hits.sort_unstable();
            self.hits.dedup();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StainBitmap {
    pub width: u16,
    pub height: u16,
    pub key_x: i16,
    pub key_y: i16,
    pub color: Vec<u8>,
    pub mask: Vec<u8>,
}

fn closest_color(table: &[[u16; 3]], color: [u16; 3]) -> u8 {
    let mut best = i32::MAX;
    let mut found = 0u8;
    for (i, c) in table.iter().enumerate() {
        let d = |k: usize| (i32::from(color[k]) - i32::from(c[k])) >> 4;
        let (r, g, b) = (d(0), d(1), d(2));
        let e = 2 * r * r + 5 * g * g + b * b;
        if i == 0 || e < best {
            best = e;
            found = i as u8;
        }
    }
    found
}

fn build_blend_table(colors: &[[u16; 3]]) -> Vec<u8> {
    let n = colors.len();
    let mut table = vec![0u8; 65536];
    for b in 0..n {
        for a in 0..n {
            table[a << 8 | b] = if a == 0 {
                b as u8
            } else if b == 0 {
                a as u8
            } else {
                let avg =
                    |k: usize| ((u32::from(colors[b][k]) + u32::from(colors[a][k])) >> 1) as u16;
                closest_color(colors, [avg(0), avg(1), avg(2)])
            };
        }
    }
    table
}

impl LandscapeTexels {
    pub fn new(texture: &TerrainTexture, shading: &mandrake_tag::LandscapeShading) -> Self {
        Self::with_table(texture, &texture.shading_table(shading))
    }

    pub fn with_table(
        texture: &TerrainTexture,
        table: &mandrake_tag::LandscapeShadingTable,
    ) -> Self {
        Self {
            width: texture.width,
            height: texture.height,
            color_index: texture.color_index.clone(),
            shade: texture.shadow_map.clone(),
            table: table.rgb8(),
            palette16: texture.palette16,
            color_count: texture.color_count,
            blend: None,
            blood_remap: texture.blood_remap.clone(),
            scratch: RegionScratch::default(),
        }
    }

    pub fn prepare_stains(&mut self) {
        let colors = &self.palette16[..self.color_count.min(256)];
        self.blend.get_or_insert_with(|| build_blend_table(colors));
    }

    pub fn set_blend_table(&mut self, table: Vec<u8>) {
        if table.len() == 1 << 16 {
            self.blend = Some(table);
        }
    }

    pub fn color_index(&self) -> &[u8] {
        &self.color_index
    }

    pub fn shade(&self) -> &[u8] {
        &self.shade
    }

    pub fn table(&self) -> &[[u8; 3]] {
        &self.table
    }

    fn submesh_pixel(&self, x: i32, y: i32) -> Option<(u32, u32)> {
        let row_submeshes = (self.width / SUBMESH_PIXELS) as i32;
        let submesh = (x >> 14) + (y >> 14) * row_submeshes;
        if submesh < 0 || submesh >= row_submeshes * (self.height / SUBMESH_PIXELS) as i32 {
            return None;
        }
        Some((
            (submesh % row_submeshes) as u32 * SUBMESH_PIXELS
                + (x >> WORLD_UNITS_PER_PIXEL_SHIFT & 0xff) as u32,
            (submesh / row_submeshes) as u32 * SUBMESH_PIXELS
                + (y >> WORLD_UNITS_PER_PIXEL_SHIFT & 0xff) as u32,
        ))
    }

    pub fn footstep(
        &mut self,
        point: (i32, i32),
        random: &mut dyn FnMut() -> u16,
    ) -> Option<PixelRect> {
        self.footstep_above(point, MYTH2_FOOTSTEP_SHADE, random)
    }

    pub fn footstep_above(
        &mut self,
        (x, y): (i32, i32),
        threshold: u8,
        random: &mut dyn FnMut() -> u16,
    ) -> Option<PixelRect> {
        let (px, py) = self.submesh_pixel(x, y)?;
        let shade = &mut self.shade[(py * self.width + px) as usize];
        if *shade <= threshold {
            return None;
        }
        *shade -= 6 + (random() & 3) as u8;
        Some(PixelRect {
            x: px,
            y: py,
            width: 1,
            height: 1,
        })
    }

    pub fn bloody_mesh(
        &mut self,
        (x, y): (i32, i32),
        is_media: &dyn Fn(i32, i32) -> bool,
        random: &mut dyn FnMut() -> u16,
    ) -> Option<PixelRect> {
        let table = self.blood_remap.as_ref()?;
        if is_media(
            x >> WORLD_UNITS_PER_PIXEL_SHIFT,
            y >> WORLD_UNITS_PER_PIXEL_SHIFT,
        ) {
            return None;
        }
        let (px, py) = self.submesh_pixel(x, y)?;
        let o = (py * self.width + px) as usize;
        let new = table[usize::from(self.color_index[o])];
        if new == self.color_index[o] {
            return None;
        }
        self.color_index[o] = new;
        if self.shade[o] > 0x28 {
            self.shade[o] -= 6 + (random() & 7) as u8;
        }
        Some(PixelRect {
            x: px,
            y: py,
            width: 1,
            height: 1,
        })
    }

    pub fn apply_stain(
        &mut self,
        stain: &StainBitmap,
        color_table: &[[u16; 4]],
        point: (i32, i32),
        mirror: (bool, bool),
        is_media: &dyn Fn(i32, i32) -> bool,
    ) -> Option<PixelRect> {
        let remap = self.stain_remap(color_table);
        self.apply_stain_remapped(stain, &remap, point, mirror, is_media)
    }

    pub fn stain_remap(&self, color_table: &[[u16; 4]]) -> [u8; 256] {
        let colors = &self.palette16[..self.color_count.min(256)];
        let mut remap = [0u8; 256];
        for (i, c) in color_table.iter().enumerate().take(256).skip(1) {
            remap[i] = closest_color(colors, [c[0], c[1], c[2]]);
        }
        remap
    }

    pub fn apply_stain_remapped(
        &mut self,
        stain: &StainBitmap,
        remap: &[u8; 256],
        (x, y): (i32, i32),
        mirror: (bool, bool),
        is_media: &dyn Fn(i32, i32) -> bool,
    ) -> Option<PixelRect> {
        let colors = &self.palette16[..self.color_count.min(256)];
        let blend = self.blend.get_or_insert_with(|| build_blend_table(colors));
        let (w, h) = (i32::from(stain.width), i32::from(stain.height));
        let x0 = (x >> WORLD_UNITS_PER_PIXEL_SHIFT) - i32::from(stain.key_x);
        let y0 = (y >> WORLD_UNITS_PER_PIXEL_SHIFT) - i32::from(stain.key_y);
        let (map_w, map_h) = (self.width as i32, self.height as i32);
        for row in 0..h {
            let py = y0 + row;
            if py < 0 || py >= map_h {
                continue;
            }
            let sy = if mirror.1 { h - 1 - row } else { row };
            for col in 0..w {
                let px = x0 + col;
                if px < 0 || px >= map_w || is_media(px, py) {
                    continue;
                }
                let sx = if mirror.0 { w - 1 - col } else { col };
                let s = (sy * w + sx) as usize;
                let o = (py * map_w + px) as usize;
                let m = i32::from(stain.mask.get(s).copied().unwrap_or(0));
                self.shade[o] = ((i32::from(self.shade[o]) * (255 - m)) >> 8) as u8;
                let c = stain.color.get(s).copied().unwrap_or(0);
                if c != 0 {
                    let dst = usize::from(self.color_index[o]);
                    self.color_index[o] = blend[dst << 8 | usize::from(remap[usize::from(c)])];
                }
            }
        }
        let (x1, y1) = ((x0 + w).min(map_w), (y0 + h).min(map_h));
        let (x0, y0) = (x0.max(0), y0.max(0));
        (x1 > x0 && y1 > y0).then(|| PixelRect {
            x: x0 as u32,
            y: y0 as u32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        })
    }

    pub fn region_rgba(&self, rect: PixelRect, overlays: &[ShadeOverlay<'_>]) -> Vec<u8> {
        let mut scratch = RegionScratch::default();
        scratch.bucket(overlays, self.width, self.height);
        self.build_region(rect, overlays, &mut scratch);
        scratch.rgba
    }

    pub fn rebuild_regions(
        &mut self,
        dirty: &mut Vec<PixelRect>,
        overlays: &[ShadeOverlay<'_>],
        mut upload: impl FnMut(PixelRect, &[u8]),
    ) {
        merge_overlapping_rects(dirty);
        if dirty.is_empty() {
            return;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.bucket(overlays, self.width, self.height);
        for &rect in dirty.iter() {
            self.build_region(rect, overlays, &mut scratch);
            upload(rect, &scratch.rgba);
        }
        self.scratch = scratch;
    }

    fn build_region(
        &self,
        rect: PixelRect,
        overlays: &[ShadeOverlay<'_>],
        scratch: &mut RegionScratch,
    ) {
        let stride = self.width as usize;
        let (x, width) = (rect.x as usize, rect.width as usize);
        let rows = rect.y as usize..(rect.y + rect.height) as usize;
        scratch.shade.clear();
        for y in rows.clone() {
            let o = y * stride + x;
            scratch.shade.extend_from_slice(&self.shade[o..o + width]);
        }
        scratch.find_hits(&rect);
        for last in [false, true] {
            for o in scratch.hits.iter().rev().map(|&i| &overlays[i as usize]) {
                if o.mode.drawn_last() == last {
                    o.rasterize_with(&mut scratch.shade, rect, &mut scratch.carry);
                }
            }
        }
        scratch.rgba.clear();
        scratch.rgba.reserve(width * rows.len() * 4);
        let mut shade = scratch.shade.iter();
        for y in rows {
            let o = y * stride + x;
            for (&index, &s) in self.color_index[o..o + width].iter().zip(&mut shade) {
                let rgb = TerrainTexture::texel(&self.table, s, index);
                scratch
                    .rgba
                    .extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    }

    pub fn rgba(&self, overlays: &[ShadeOverlay<'_>]) -> Vec<u8> {
        self.region_rgba(
            PixelRect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            overlays,
        )
    }
}

const MERGE_SLACK: u64 = 96;
const MERGE_ROWS: u32 = 4;

fn merge_overlapping_rects(rects: &mut Vec<PixelRect>) {
    rects.retain(|r| r.width != 0 && r.height != 0);
    rects.sort_unstable_by_key(|r| (r.y, r.x, r.width, r.height));
    let area = |r: &PixelRect| u64::from(r.width) * u64::from(r.height);
    let mut kept = 0;
    for i in 0..rects.len() {
        if rects[i].width == 0 {
            continue;
        }
        let mut a = rects[i];
        for b in &mut rects[i + 1..] {
            if b.y >= a.y + a.height + MERGE_ROWS {
                break;
            }
            if b.width == 0 {
                continue;
            }
            let (x, x1) = (a.x.min(b.x), (a.x + a.width).max(b.x + b.width));
            let both = PixelRect {
                x,
                y: a.y,
                width: x1 - x,
                height: (a.y + a.height).max(b.y + b.height) - a.y,
            };
            if area(&both) <= area(&a) + area(b) + MERGE_SLACK {
                a = both;
                b.width = 0;
            }
        }
        rects[kept] = a;
        kept += 1;
    }
    rects.truncate(kept);
}

pub fn changed_overlay_rects(
    old: &[ShadeOverlay<'_>],
    new: &[ShadeOverlay<'_>],
    map_w: u32,
    map_h: u32,
) -> Vec<PixelRect> {
    let same = |(a, b): &(&ShadeOverlay<'_>, &ShadeOverlay<'_>)| a == b;
    let head = old.iter().zip(new).take_while(same).count();
    let (old, new) = (&old[head..], &new[head..]);
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(same)
        .count();
    let (old, new) = (&old[..old.len() - tail], &new[..new.len() - tail]);

    let mut dirty = Vec::new();
    let mut unmatched = vec![true; new.len()];
    let mut by_place: Vec<(i32, i32, usize)> =
        new.iter().enumerate().map(|(i, o)| (o.x, o.y, i)).collect();
    by_place.sort_unstable();
    let mut next = 0;
    let mut in_order = true;
    for o in old {
        let from = by_place.partition_point(|&(x, y, _)| (x, y) < (o.x, o.y));
        let here = &by_place[from..];
        let here = &here[..here.partition_point(|&(x, y, _)| (x, y) == (o.x, o.y))];
        let ahead = here.partition_point(|&(_, _, i)| i < next);
        let found = here[ahead..]
            .iter()
            .chain(&here[..ahead])
            .map(|&(_, _, i)| i)
            .find(|&i| unmatched[i] && new[i] == *o);
        match found {
            Some(i) => {
                unmatched[i] = false;
                in_order &= i >= next;
                next = i + 1;
            }
            None => dirty.extend(o.pixel_rect(map_w, map_h)),
        }
    }
    if in_order {
        dirty.extend(
            new.iter()
                .zip(&unmatched)
                .filter(|&(_, &came)| came)
                .filter_map(|(o, _)| o.pixel_rect(map_w, map_h)),
        );
    } else {
        dirty = old
            .iter()
            .chain(new)
            .filter_map(|o| o.pixel_rect(map_w, map_h))
            .collect();
    }
    dirty.sort_by_key(|r| (r.y, r.x, r.width, r.height));
    dirty.dedup();
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(
        pixels: &[u8],
        w: i16,
        h: i16,
        x: i32,
        y: i32,
        mode: OverlayMode,
    ) -> ShadeOverlay<'_> {
        ShadeOverlay {
            x,
            y,
            pixels,
            width: w,
            height: h,
            row_bytes: w,
            intensity: 0xfeff,
            mode,
        }
    }

    fn all(w: u32, h: u32) -> PixelRect {
        PixelRect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        }
    }

    #[test]
    fn darken_aligned_single_pixel() {
        let mut buf = vec![200u8; 4 * 4];
        let src = [128u8];
        overlay(&src, 1, 1, 64, 64, OverlayMode::Darken).rasterize(&mut buf, all(4, 4));
        let a = (128 * 0xfeff) >> 16;
        let v = (63 * 63 * a) >> 12;
        assert_eq!(i32::from(buf[5]), (((255 - v) * 200) >> 8).max(0x40));
        assert_eq!(buf.iter().filter(|&&b| b != 200).count(), 1);
        let mut buf = vec![200u8, 0x30];
        overlay(&[255, 255], 2, 1, 0, 0, OverlayMode::Darken).rasterize(&mut buf, all(2, 1));
        assert_eq!(buf, [0x40, 0x30]);
    }

    #[test]
    fn darken_subpixel_carries() {
        let src = [0u8, 255, 0];
        let mut buf = vec![255u8; 3];
        overlay(&src, 3, 1, 32, 0, OverlayMode::Darken).rasterize(&mut buf, all(3, 1));
        let a = (255 * 0xfeff) >> 16;
        let h1 = 31 * a;
        let v1 = (63 * h1) >> 12;
        assert_eq!(buf[0], 255);
        assert_eq!(i32::from(buf[1]), ((255 - v1) * 255) >> 8);
        assert_eq!(buf[2], 255);
        let mut buf = vec![10u8; 3];
        overlay(&src, 3, 1, 32, 0, OverlayMode::Lighten).rasterize(&mut buf, all(3, 1));
        let h2 = 32 * a;
        assert_eq!(
            buf,
            [
                10,
                (10 + v1).min(255) as u8,
                (10 + ((63 * h2) >> 12)).min(255) as u8
            ]
        );
    }

    #[test]
    fn min_mode_and_window() {
        let src = [10u8, 250, 30, 40];
        let mut buf = vec![100u8; 2];
        let o = overlay(&src, 2, 2, 0, 0, OverlayMode::Min);
        o.rasterize(
            &mut buf,
            PixelRect {
                x: 1,
                y: 0,
                width: 1,
                height: 2,
            },
        );
        assert_eq!(buf, [100, 40]);
        assert_eq!(
            o.pixel_rect(1, 8),
            Some(PixelRect {
                x: 0,
                y: 0,
                width: 1,
                height: 2
            })
        );
    }

    #[test]
    fn the_callers_opacity_scales_the_overlay_by_its_high_byte() {
        assert_eq!(
            map_overlay_intensity_of(0x80ff, 0, true, 0, 0),
            Some((((0x80 * 0xffff) >> 8) as u16, 0))
        );
        assert_eq!(map_overlay_intensity_of(0x00ff, 0, true, 0, 0), None);
    }

    #[test]
    fn grounded_shadow_intensity() {
        assert_eq!(map_overlay_intensity(0, false, 0, 0), Some((0xfeff, 0)));
        assert_eq!(map_overlay_intensity(3072, false, 0, 0), None);
        let (i, s) = map_overlay_intensity(1536, false, 0, 0).unwrap();
        assert_eq!((i, s), (((0xff * 0x8000) >> 8) as u16, 768));
        assert_eq!(
            map_overlay_intensity(512, true, 0, 0),
            Some((((0xff * 0x8000) >> 8) as u16, 0))
        );
        assert_eq!(
            map_overlay_intensity(-256, false, 0, 0),
            Some((((0xff * 0x8000) >> 8) as u16, 0))
        );
    }

    #[test]
    fn tfl_shadow_record_intensity() {
        let place = |above_ground, drawn_height, in_water_top, transfer| TflShadowPlace {
            above_ground,
            drawn_height,
            in_water_top,
            transfer,
        };
        assert_eq!(
            tfl_shadow_intensity(&place(0, 0, None, (0, 0))),
            Some(0xffff)
        );
        assert_eq!(
            tfl_shadow_intensity(&place(-40, 0, None, (0, 0))),
            Some(0xffff)
        );
        assert_eq!(
            tfl_shadow_intensity(&place(1024, 0, None, (0, 0))),
            Some(0x8000)
        );
        assert_eq!(tfl_shadow_intensity(&place(2048, 0, None, (0, 0))), None);
        assert_eq!(tfl_shadow_intensity(&place(0, -300, Some(0), (0, 0))), None);
        assert_eq!(
            tfl_shadow_intensity(&place(0, -256, Some(40), (0, 0))),
            Some(0x8000)
        );
        assert_eq!(
            tfl_shadow_intensity(&place(0, -512, Some(40), (0, 0))),
            None
        );
        assert_eq!(
            tfl_shadow_intensity(&place(-90, 10, Some(40), (0, 0))),
            Some(0xffff)
        );
        assert_eq!(
            tfl_shadow_intensity(&place(0, 0, None, (2, 0xc000))),
            Some(0xc0 * 0xff)
        );
        assert_eq!(
            tfl_shadow_intensity(&place(0, 0, None, (1, 0x8000))),
            Some((-129_i32 * 0xff) as u16)
        );
    }

    #[test]
    fn a_stain_darkens_the_shading_and_blends_the_colours() {
        let mut texture = mandrake_tag::collection::TerrainTexture {
            width: 4,
            height: 2,
            color_index: vec![1; 8],
            shadow_map: vec![200; 8],
            palette16: [[0; 3]; 256],
            color_count: 4,
            blood_remap: None,
        };
        texture.palette16[1] = [0x8000, 0x8000, 0x8000];
        texture.palette16[2] = [0xffff, 0xffff, 0xffff];
        texture.palette16[3] = [0xc000, 0xc000, 0xc000];
        let mut texels = LandscapeTexels::new(&texture, &mandrake_tag::LandscapeShading::default());
        let stain = StainBitmap {
            width: 2,
            height: 1,
            key_x: 1,
            key_y: 0,
            color: vec![5, 0],
            mask: vec![128, 64],
        };
        let mut table = vec![[0u16; 4]; 6];
        table[5] = [0xffff, 0xffff, 0xffff, 0];
        let rect = texels
            .apply_stain(&stain, &table, (2 * 64 + 5, 64), (false, false), &|_, _| {
                false
            })
            .unwrap();
        assert_eq!(
            rect,
            PixelRect {
                x: 1,
                y: 1,
                width: 2,
                height: 1
            }
        );
        assert_eq!(texels.color_index[4 + 1], 3);
        assert_eq!(texels.shade[4 + 1], ((200 * (255 - 128)) >> 8) as u8);
        assert_eq!(texels.color_index[4 + 2], 1);
        assert_eq!(texels.shade[4 + 2], ((200 * (255 - 64)) >> 8) as u8);
        assert_eq!(texels.color_index[0], 1);
        assert_eq!(texels.shade[0], 200);
    }

    #[test]
    fn a_stain_is_the_same_with_the_blend_table_built_early() {
        let mut texture = mandrake_tag::collection::TerrainTexture {
            width: 3,
            height: 1,
            color_index: vec![1, 2, 3],
            shadow_map: vec![200; 3],
            palette16: [[0; 3]; 256],
            color_count: 4,
            blood_remap: None,
        };
        texture.palette16[1] = [0x8000, 0x8000, 0x8000];
        texture.palette16[2] = [0xffff, 0xffff, 0xffff];
        texture.palette16[3] = [0xc000, 0xc000, 0xc000];
        let mut lazy = LandscapeTexels::new(&texture, &mandrake_tag::LandscapeShading::default());
        let mut early = lazy.clone();
        early.prepare_stains();
        assert!(early.blend.is_some() && lazy.blend.is_none());
        let stain = StainBitmap {
            width: 3,
            height: 1,
            key_x: 0,
            key_y: 0,
            color: vec![1, 2, 3],
            mask: vec![128, 64, 32],
        };
        let table = vec![[0xffffu16; 4]; 4];
        let stain_on = |t: &mut LandscapeTexels| {
            t.apply_stain(&stain, &table, (0, 0), (false, false), &|_, _| false)
        };
        assert_eq!(stain_on(&mut early), stain_on(&mut lazy));
        assert_eq!(early.blend, lazy.blend);
        assert_eq!(early.color_index, lazy.color_index);
        assert_eq!(early.shade, lazy.shade);
    }

    #[test]
    fn a_stain_mirrors_and_skips_media_pixels() {
        let texture = mandrake_tag::collection::TerrainTexture {
            width: 3,
            height: 1,
            color_index: vec![0; 3],
            shadow_map: vec![255; 3],
            palette16: [[0; 3]; 256],
            color_count: 1,
            blood_remap: None,
        };
        let mut texels = LandscapeTexels::new(&texture, &mandrake_tag::LandscapeShading::default());
        let stain = StainBitmap {
            width: 3,
            height: 1,
            key_x: 0,
            key_y: 0,
            color: vec![0; 3],
            mask: vec![0, 128, 255],
        };
        texels.apply_stain(&stain, &[], (0, 0), (true, false), &|x, _| x == 1);
        assert_eq!(texels.shade[0], 0, "mask 255 lands on pixel 0");
        assert_eq!(texels.shade[1], 255, "media is skipped");
        assert_eq!(texels.shade[2], 254, "mask 0 leaves 255 * 255 >> 8");
    }

    fn blood_texels(remap: bool) -> LandscapeTexels {
        let mut table = [0u8; 256];
        for (i, t) in table.iter_mut().enumerate() {
            *t = if i >= 200 { 202 } else { 200 + (i % 2) as u8 };
        }
        let texture = mandrake_tag::collection::TerrainTexture {
            width: 512,
            height: 256,
            color_index: vec![70; 512 * 256],
            shadow_map: vec![211; 512 * 256],
            palette16: [[0; 3]; 256],
            color_count: 256,
            blood_remap: remap.then(|| Box::new(table)),
        };
        LandscapeTexels::new(&texture, &mandrake_tag::LandscapeShading::default())
    }

    #[test]
    fn a_footstep_above_a_passed_threshold_stops_there() {
        let mut t = blood_texels(true);
        let (point, o) = ((229 * 64, 66 * 64), 66 * 512 + 229);
        let mut random = || 0xfff6u16;
        t.shade[o] = 0x30;
        assert!(t.footstep(point, &mut random).is_none());
        assert!(t.footstep_above(point, 0x28, &mut random).is_some());
        assert_eq!(t.shade[o], 0x30 - 8);
        assert!(t.footstep_above(point, 0x28, &mut random).is_none());
        assert_eq!(t.shade[o], 0x28);
    }

    #[test]
    fn a_footstep_darkens_only_the_shade_of_one_texel_down_to_0x50() {
        let mut t = blood_texels(true);
        let (mut draws, point, o) = (0, (229 * 64 + 11, 66 * 64 + 59), 66 * 512 + 229);
        let mut random = || {
            draws += 1;
            0xfff6u16
        };
        assert_eq!(
            t.footstep(point, &mut random),
            Some(PixelRect {
                x: 229,
                y: 66,
                width: 1,
                height: 1
            })
        );
        assert_eq!((t.shade[o], t.color_index[o]), (203, 70));
        t.shade[o] = 0x51;
        assert!(t.footstep(point, &mut random).is_some());
        assert_eq!(t.shade[o], 0x51 - 8);
        t.shade[o] = 0x50;
        assert_eq!(t.footstep(point, &mut random), None);
        assert_eq!(t.shade[o], 0x50);
        assert_eq!(t.footstep((-1, 0), &mut random), None);
        assert_eq!(t.footstep((0, 256 * 64), &mut random), None);
        assert_eq!(draws, 2);
    }

    #[test]
    fn bloody_mesh_remaps_one_texel_and_darkens_it_only_when_it_changed() {
        let mut t = blood_texels(true);
        let mut draws = 0;
        let mut random = || {
            draws += 1;
            0xfff7u16
        };
        let point = (229 * 64 + 11, 66 * 64 + 59);
        let o = 66 * 512 + 229;
        assert_eq!(
            t.bloody_mesh(point, &|_, _| false, &mut random),
            Some(PixelRect {
                x: 229,
                y: 66,
                width: 1,
                height: 1
            })
        );
        assert_eq!((t.color_index[o], t.shade[o]), (200, 211 - 6 - 7));
        assert!(t.bloody_mesh(point, &|_, _| false, &mut random).is_some());
        assert_eq!((t.color_index[o], t.shade[o]), (202, 211 - 26));
        assert_eq!(t.bloody_mesh(point, &|_, _| false, &mut random), None);
        assert_eq!((t.color_index[o], t.shade[o]), (202, 211 - 26));
        assert_eq!(draws, 2);
    }

    #[test]
    fn bloody_mesh_leaves_a_dark_shade_but_still_changes_the_colour() {
        let mut t = blood_texels(true);
        t.shade[0] = 0x28;
        let mut random = || panic!("no local_random at or under 0x28");
        assert!(t.bloody_mesh((0, 0), &|_, _| false, &mut random).is_some());
        assert_eq!((t.color_index[0], t.shade[0]), (200, 0x28));
    }

    #[test]
    fn bloody_mesh_skips_media_no_table_and_points_off_the_map() {
        let mut random = || panic!("no draw");
        let mut t = blood_texels(true);
        let media = |x: i32, y: i32| (x, y) == (3, 4);
        assert_eq!(t.bloody_mesh((3 * 64, 4 * 64), &media, &mut random), None);
        assert_eq!(t.color_index[4 * 512 + 3], 70);
        assert_eq!(t.bloody_mesh((-1, 0), &|_, _| false, &mut random), None);
        assert_eq!(
            t.bloody_mesh((0, 1 << 14), &|_, _| false, &mut random),
            None
        );
        let mut none = blood_texels(false);
        assert_eq!(none.bloody_mesh((0, 0), &|_, _| false, &mut random), None);
        assert_eq!(none.color_index[0], 70);
    }

    #[test]
    fn bloody_mesh_finds_the_texel_through_the_submesh() {
        let mut t = blood_texels(true);
        let mut random = || 0;
        let r = t.bloody_mesh(((1 << 14) + 5 * 64, 6 * 64), &|_, _| false, &mut random);
        assert_eq!(r.map(|r| (r.x, r.y)), Some((261, 6)));
        assert_eq!(
            t.bloody_mesh((2 << 14, 0), &|_, _| false, &mut random),
            None
        );
    }

    #[test]
    fn the_blend_table_gives_the_other_for_index_0() {
        let c = [
            [0, 0, 0],
            [0x1000, 0x1000, 0x1000],
            [0x2000, 0x2000, 0x2000],
        ];
        let t = build_blend_table(&c);
        assert_eq!(t[1 << 8], 1);
        assert_eq!(t[2], 2);
        assert_eq!(t[1 << 8 | 1], 1);
        assert_eq!(
            closest_color(&c, [0x1800, 0x1800, 0x1800]),
            1,
            "the first of equals"
        );
    }

    fn moving_list<'a>(bitmap: &'a [u8], list: &[(i32, i32, usize)]) -> Vec<ShadeOverlay<'a>> {
        let modes = [OverlayMode::Darken, OverlayMode::Lighten, OverlayMode::Min];
        list.iter()
            .map(|&(x, y, n)| overlay(&bitmap[n..n + 9], 3, 3, x, y, modes[n % 3]))
            .collect()
    }

    #[test]
    fn only_the_changed_rects_rebuild_the_same_landscape() {
        let (w, h) = (16u32, 16u32);
        let bitmap: Vec<u8> = (0..16u32).map(|i| (i * 37 + 40) as u8).collect();
        let texels = LandscapeTexels {
            width: w,
            height: h,
            color_index: (0..w * h).map(|i| (i % 4) as u8).collect(),
            shade: (0..w * h).map(|i| (64 + i % 128) as u8).collect(),
            table: (0..65536u32)
                .map(|i| [(i >> 8) as u8, i as u8, (i >> 3) as u8])
                .collect(),
            palette16: [[0; 3]; 256],
            color_count: 0,
            blend: None,
            blood_remap: None,
            scratch: RegionScratch::default(),
        };
        let a = (64, 64, 0);
        let b = (128, 96, 1);
        let c = (96, 128, 2);
        let d = (576, 576, 3);
        let steps: [&[(i32, i32, usize)]; 12] = [
            &[],
            &[a, b, c, d],
            &[a, b, c, d],
            &[(a.0 + 3, a.1, a.2), b, c, d],
            &[(a.0 + 3, a.1, a.2), (b.0, b.1 + 70, b.2), c, d],
            &[a, b, c, (d.0 - 64, d.1, d.2)],
            &[a, (b.0, b.1, 4), c, d],
            &[a, c, d],
            &[b, a, c, d],
            &[a, b, c, d],
            &[b, a, c, d],
            &[a, a, d],
        ];
        let mut shown = texels.rgba(&[]);
        let mut old: Vec<ShadeOverlay<'_>> = Vec::new();
        for (step, list) in steps.iter().enumerate() {
            let new = moving_list(&bitmap, list);
            for rect in changed_overlay_rects(&old, &new, w, h) {
                let rgba = texels.region_rgba(rect, &new);
                for row in 0..rect.height as usize {
                    let to = (rect.y as usize + row) * w as usize * 4 + rect.x as usize * 4;
                    let width = rect.width as usize * 4;
                    shown[to..to + width].copy_from_slice(&rgba[row * width..(row + 1) * width]);
                }
            }
            assert_eq!(shown, texels.rgba(&new), "step {step}");
            old = new;
        }
    }

    fn rasterize_whole_bitmap(o: &ShadeOverlay<'_>, buf: &mut [u8], window: PixelRect) {
        let (w, h) = (i32::from(o.width), i32::from(o.height));
        if w <= 0 || h <= 0 {
            return;
        }
        let px = o.x >> WORLD_UNITS_PER_PIXEL_SHIFT;
        let py = o.y >> WORLD_UNITS_PER_PIXEL_SHIFT;
        let row_bytes = o.row_bytes as isize;
        let src = |r: i32, c: i32| -> i32 {
            let at = r as isize * row_bytes + c as isize;
            usize::try_from(at)
                .ok()
                .and_then(|at| o.pixels.get(at))
                .map_or(0, |&p| i32::from(p))
        };
        let mut put = |c: i32, r: i32, f: &dyn Fn(i32) -> i32| {
            let (x, y) = (px + c, py + r);
            if window.contains(x, y) {
                let at = (y as u32 - window.y) as usize * window.width as usize
                    + (x as u32 - window.x) as usize;
                let d = &mut buf[at];
                *d = f(i32::from(*d)) as u8;
            }
        };
        match o.mode {
            OverlayMode::Min => {
                for r in 0..h {
                    for c in 0..w {
                        let s = src(r, c);
                        put(c, r, &|d| d.min(s));
                    }
                }
            }
            OverlayMode::Darken | OverlayMode::Lighten => {
                let fx = !o.x & 0x3f;
                let fy = !o.y & 0x3f;
                let intensity = i32::from(o.intensity);
                let mut row_carry = vec![0i32; w as usize];
                for r in 0..h {
                    let mut col_carry = 0i32;
                    for c in 0..w {
                        let a = (src(r, c) * intensity) >> 16;
                        let hs = fx * a;
                        let hv = hs + col_carry;
                        col_carry = 63 * a - hs;
                        let vs = fy * hv;
                        let v = (row_carry[c as usize] + vs) >> 12;
                        row_carry[c as usize] = 63 * hv - vs;
                        match o.mode {
                            OverlayMode::Lighten => put(c, r, &|d| (d + v).min(255)),
                            _ if a != 0 => {
                                put(c, r, &|d| (((255 - v) * d) >> 8).max(DARKEN_FLOOR).min(d))
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    fn region_rgba_whole_list(
        texels: &LandscapeTexels,
        rect: PixelRect,
        overlays: &[ShadeOverlay<'_>],
    ) -> Vec<u8> {
        let stride = texels.width as usize;
        let mut shade = Vec::new();
        for y in rect.y..rect.y + rect.height {
            let o = y as usize * stride + rect.x as usize;
            shade.extend_from_slice(&texels.shade[o..o + rect.width as usize]);
        }
        let hit = |o: &&ShadeOverlay<'_>| {
            o.pixel_rect(texels.width, texels.height)
                .is_some_and(|r| r.intersects(&rect))
        };
        for last in [false, true] {
            for o in overlays
                .iter()
                .rev()
                .filter(|o| o.mode.drawn_last() == last)
                .filter(hit)
            {
                rasterize_whole_bitmap(o, &mut shade, rect);
            }
        }
        let mut out = Vec::new();
        let mut s = shade.iter();
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                let index = texels.color_index[y as usize * stride + x as usize];
                let rgb = TerrainTexture::texel(&texels.table, *s.next().unwrap_or(&0), index);
                out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        out
    }

    fn changed_overlay_rects_whole_list(
        old: &[ShadeOverlay<'_>],
        new: &[ShadeOverlay<'_>],
        map_w: u32,
        map_h: u32,
    ) -> Vec<PixelRect> {
        let same = |(a, b): &(&ShadeOverlay<'_>, &ShadeOverlay<'_>)| a == b;
        let head = old.iter().zip(new).take_while(same).count();
        let (old, new) = (&old[head..], &new[head..]);
        let tail = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take_while(same)
            .count();
        let (old, new) = (&old[..old.len() - tail], &new[..new.len() - tail]);

        let mut dirty = Vec::new();
        let mut unmatched = vec![true; new.len()];
        let mut next = 0;
        let mut in_order = true;
        for o in old {
            let found = (next..new.len())
                .chain(0..next)
                .find(|&i| unmatched[i] && new[i] == *o);
            match found {
                Some(i) => {
                    unmatched[i] = false;
                    in_order &= i >= next;
                    next = i + 1;
                }
                None => dirty.extend(o.pixel_rect(map_w, map_h)),
            }
        }
        if in_order {
            dirty.extend(
                new.iter()
                    .zip(&unmatched)
                    .filter(|&(_, &came)| came)
                    .filter_map(|(o, _)| o.pixel_rect(map_w, map_h)),
            );
        } else {
            dirty = old
                .iter()
                .chain(new)
                .filter_map(|o| o.pixel_rect(map_w, map_h))
                .collect();
        }
        dirty.sort_by_key(|r| (r.y, r.x, r.width, r.height));
        dirty.dedup();
        dirty
    }

    fn plain_texels(w: u32, h: u32) -> LandscapeTexels {
        LandscapeTexels {
            width: w,
            height: h,
            color_index: (0..w * h).map(|i| (i * 7 % 256) as u8).collect(),
            shade: (0..w * h).map(|i| (40 + i * 13 % 216) as u8).collect(),
            table: (0..65536u32)
                .map(|i| [(i >> 8) as u8, i as u8, (i >> 3) as u8])
                .collect(),
            palette16: [[0; 3]; 256],
            color_count: 0,
            blend: None,
            blood_remap: None,
            scratch: RegionScratch::default(),
        }
    }

    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u32) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as u32) % n
        }
    }

    fn random_overlay<'a>(rng: &mut Lcg, bitmap: &'a [u8]) -> ShadeOverlay<'a> {
        let (width, height) = (1 + rng.below(40) as i16, 1 + rng.below(40) as i16);
        let row_bytes = width + rng.below(3) as i16;
        let from = rng.below(64) as usize;
        let len = (row_bytes as usize * height as usize).saturating_sub(rng.below(2) as usize * 9);
        ShadeOverlay {
            x: rng.below(64 * 260) as i32 - 64 * 30,
            y: rng.below(64 * 210) as i32 - 64 * 30,
            pixels: &bitmap[from..from + len],
            width,
            height,
            row_bytes,
            intensity: [0xffff, 0x8000, 0x1234, 0][rng.below(4) as usize],
            mode: [OverlayMode::Darken, OverlayMode::Lighten, OverlayMode::Min]
                [rng.below(3) as usize],
        }
    }

    fn random_bitmap(rng: &mut Lcg) -> Vec<u8> {
        (0..64 + 42 * 40)
            .map(|_| match rng.below(4) {
                0 => 0,
                _ => rng.below(256) as u8,
            })
            .collect()
    }

    #[test]
    fn a_clipped_rasterize_writes_the_whole_bitmaps_values() {
        let mut rng = Lcg(691);
        let bitmap = random_bitmap(&mut rng);
        let (w, h) = (200u32, 150u32);
        for case in 0..4000 {
            let o = random_overlay(&mut rng, &bitmap);
            let x = rng.below(w);
            let y = rng.below(h);
            let window = PixelRect {
                x,
                y,
                width: 1 + rng.below(w - x).min(rng.below(60)),
                height: 1 + rng.below(h - y).min(rng.below(60)),
            };
            let before: Vec<u8> = (0..window.width * window.height)
                .map(|_| rng.below(256) as u8)
                .collect();
            let mut want = before.clone();
            rasterize_whole_bitmap(&o, &mut want, window);
            let mut got = before;
            o.rasterize(&mut got, window);
            assert_eq!(got, want, "case {case}: {o:?} in {window:?}");
        }
    }

    #[test]
    fn rebuilt_regions_are_the_texels_of_the_whole_list() {
        let mut rng = Lcg(0x691);
        let bitmap = random_bitmap(&mut rng);
        let (w, h) = (200u32, 150u32);
        let mut texels = plain_texels(w, h);
        let whole = PixelRect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let fixed: Vec<ShadeOverlay<'_>> =
            (0..60).map(|_| random_overlay(&mut rng, &bitmap)).collect();
        let mut moving: Vec<ShadeOverlay<'_>> = Vec::new();
        let mut shown = region_rgba_whole_list(&texels, whole, &fixed);
        assert_eq!(texels.rgba(&fixed), shown, "the whole map");
        for frame in 0..300 {
            let mut next = moving.clone();
            match rng.below(8) {
                0..=3 => {
                    for o in &mut next {
                        o.x += rng.below(130) as i32 - 64;
                        o.y += rng.below(130) as i32 - 64;
                    }
                }
                4 => next.extend((0..rng.below(12)).map(|_| random_overlay(&mut rng, &bitmap))),
                5 if !next.is_empty() => {
                    next.remove(rng.below(next.len() as u32) as usize);
                }
                6 if next.len() > 1 => {
                    let (a, b) = (rng.below(next.len() as u32), rng.below(next.len() as u32));
                    next.swap(a as usize, b as usize);
                }
                7 if !next.is_empty() => {
                    let twice = next[rng.below(next.len() as u32) as usize];
                    next.insert(rng.below(next.len() as u32) as usize, twice);
                }
                _ => {}
            }
            let mut dirty = changed_overlay_rects(&moving, &next, w, h);
            assert_eq!(
                dirty,
                changed_overlay_rects_whole_list(&moving, &next, w, h),
                "frame {frame}: the rectangles"
            );
            let overlays: Vec<ShadeOverlay<'_>> = fixed.iter().chain(&next).copied().collect();
            if let Some(&rect) = dirty.first() {
                assert_eq!(
                    texels.region_rgba(rect, &overlays),
                    region_rgba_whole_list(&texels, rect, &overlays),
                    "frame {frame}: {rect:?}"
                );
            }
            let asked = dirty.clone();
            texels.rebuild_regions(&mut dirty, &overlays, |rect, rgba| {
                assert_eq!(rgba.len(), (rect.width * rect.height * 4) as usize);
                for row in 0..rect.height as usize {
                    let to = ((rect.y as usize + row) * w as usize + rect.x as usize) * 4;
                    let width = rect.width as usize * 4;
                    shown[to..to + width].copy_from_slice(&rgba[row * width..(row + 1) * width]);
                }
            });
            let area = |r: &[PixelRect]| r.iter().map(|r| r.width * r.height).sum::<u32>();
            let joined = (asked.len() - dirty.len()) as u32;
            assert!(
                area(&dirty) <= area(&asked) + joined * MERGE_SLACK as u32,
                "frame {frame}: more pixels"
            );
            for a in &asked {
                assert!(
                    dirty.iter().any(|d| d.x <= a.x
                        && d.y <= a.y
                        && a.x + a.width <= d.x + d.width
                        && a.y + a.height <= d.y + d.height),
                    "frame {frame}: {a:?} left out"
                );
            }
            assert_eq!(
                shown,
                region_rgba_whole_list(&texels, whole, &overlays),
                "frame {frame}"
            );
            moving = next;
        }
        assert!(moving.len() > 20, "the list grew: {}", moving.len());
    }

    #[test]
    fn overlapping_and_close_rects_merge_within_the_slack() {
        let r = |x, y, width, height| PixelRect {
            x,
            y,
            width,
            height,
        };
        let mut rects = vec![
            r(150, 50, 4, 4),
            r(10, 10, 8, 8),
            r(11, 10, 8, 8),
            r(80, 30, 5, 5),
            r(80, 30, 5, 5),
            r(151, 51, 2, 2),
        ];
        merge_overlapping_rects(&mut rects);
        assert_eq!(
            rects,
            vec![r(10, 10, 9, 8), r(80, 30, 5, 5), r(150, 50, 4, 4)]
        );
        let mut rects = vec![r(12, 0, 8, 8), r(0, 0, 8, 8), r(52, 0, 8, 8), r(3, 3, 0, 5)];
        merge_overlapping_rects(&mut rects);
        assert_eq!(rects, vec![r(0, 0, 20, 8), r(52, 0, 8, 8)]);
        let mut rects = vec![r(0, 0, 8, 8), r(7, 7, 8, 8)];
        merge_overlapping_rects(&mut rects);
        assert_eq!(rects.len(), 2);
        let mut rects = vec![
            r(0, 0, 8, 2),
            r(0, 5, 8, 2),
            r(100, 0, 8, 2),
            r(100, 6, 8, 2),
        ];
        merge_overlapping_rects(&mut rects);
        assert_eq!(rects, vec![r(0, 0, 8, 7), r(100, 0, 8, 2), r(100, 6, 8, 2)]);
    }

    #[test]
    fn unchanged_overlays_rebuild_nothing() {
        let bitmap = [200u8; 16];
        let list = moving_list(&bitmap, &[(64, 64, 0), (640, 640, 1)]);
        assert!(changed_overlay_rects(&list, &list, 64, 64).is_empty());
        let copy = bitmap;
        let same = moving_list(&copy, &[(64, 64, 0), (640, 640, 1)]);
        assert!(changed_overlay_rects(&list, &same, 64, 64).is_empty());
        let moved = moving_list(&bitmap, &[(64, 64, 0), (704, 640, 1)]);
        assert_eq!(
            changed_overlay_rects(&list, &moved, 64, 64),
            vec![
                PixelRect {
                    x: 10,
                    y: 10,
                    width: 3,
                    height: 3
                },
                PixelRect {
                    x: 11,
                    y: 10,
                    width: 3,
                    height: 3
                },
            ]
        );
    }
}
