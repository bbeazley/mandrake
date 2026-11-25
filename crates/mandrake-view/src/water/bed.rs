#![allow(clippy::needless_range_loop)]

use mandrake_tag::collection::TerrainTexture;
use mandrake_tag::mesh::MediaCoverage;

pub const BED_VERSION: u32 = 1;
pub const BED_SEED: u64 = 854;

const P: usize = 32;
const O: usize = 8;
const K: usize = 300;
const TOL: f32 = 0.15;
const ALPHA: f32 = 0.25;
const BLOCK: usize = 16;
const GUIDE_SIGMA: f32 = 12.0;
const TONE_W: f32 = 0.5;
const TONE_SIGMA: f32 = 24.0;
const GRAVEL_SHARE: f32 = 0.80;
const RING_PX: usize = 24;
const MOOR_SHARE: f32 = 0.30;
const SHALLOW_GAIN: f32 = 1.04;
const DEEP_GAIN: f32 = 0.80;
const MIN_GB: i32 = 15;
const DETAIL: f32 = 0.75;
const WATER_SHARE: f32 = 0.95;
const NEAR_PX: u32 = 16;
const FRINGE_PX: u32 = 6;
const FEATHER_PX: f32 = 4.0;
const WAVE_PX: f32 = 2.0;
const BANK_PX: f32 = 3.0;
const EDGE_CELLS: usize = 5;
const CLEAR_PX: u32 = 8;
const DEPTH_DARK_M: f32 = 2.0;
const UNITS_PER_M: f32 = 322.2;
const MIN_WINDOWS: usize = 64;
const MAX_PAINT_CONTRAST: f32 = 0.9;
const MAX_WINDOWS: usize = 3000;
const MIN_WATER_PIXELS: u64 = 64;
const FAMILY_SHARE: f32 = 0.75;
const FAMILY_DIST: f32 = 16.0;
const MARGIN: usize = 48;
const LUMA: [f32; 3] = [0.3, 0.59, 0.11];
const QUANT_W: [f32; 3] = [0.9, 1.2, 0.9];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BedReport {
    pub water_indices: usize,
    pub paint_px: usize,
    pub region_px: usize,
    pub coverage_px: usize,
    pub changed_px: usize,
    pub windows: usize,
    pub tier: u8,
    pub allowed: usize,
    pub bed_indices: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BedSkip {
    Size,
    NoPaint,
    NoLibrary,
    RealBed(f32),
}

pub fn repaint_water_paint(
    texture: &TerrainTexture,
    mesh: &mandrake_tag::MeshTag,
    coverage: &MediaCoverage,
    seed: u64,
) -> Result<(Vec<u8>, BedReport), BedSkip> {
    let (w, h) = (texture.width as usize, texture.height as usize);
    if w == 0
        || h == 0
        || texture.color_index.len() != w * h
        || w != usize::from(mesh.cell_width) * 8
        || h != usize::from(mesh.cell_height) * 8
    {
        return Err(BedSkip::Size);
    }
    let idx = &texture.color_index;
    let pal: Vec<[u8; 3]> = (0..256)
        .map(|i| {
            if i < texture.color_count {
                texture.palette16[i].map(|v| (v >> 8) as u8)
            } else {
                [0; 3]
            }
        })
        .collect();
    let mut rng = Rng::new(seed);
    let cov: Vec<bool> = (0..w * h)
        .map(|i| coverage.pixel_is_media((i % w) as i32, (i / w) as i32))
        .collect();
    let mut report = BedReport {
        coverage_px: count(&cov),
        ..Default::default()
    };
    let m = masks(idx, &pal, &cov, (w, h));
    report.water_indices = m.water_idx.len();
    report.paint_px = count(&m.paint);
    report.region_px = count(&m.region);
    if report.paint_px == 0 || report.region_px == 0 {
        return Err(BedSkip::NoPaint);
    }
    let contrast = smoothness(idx, &pal, &m, (w, h));
    if contrast >= MAX_PAINT_CONTRAST {
        return Err(BedSkip::RealBed(contrast));
    }

    let rgb: Vec<[f32; 3]> = idx
        .iter()
        .map(|&i| pal[i as usize].map(f32::from))
        .collect();
    let avoid = dilate(&m.region, (w, h), CLEAR_PX);
    let (patches, org, tier) = bed_windows(&rgb, (w, h), &avoid, &mut rng);
    if patches.is_empty() {
        return Err(BedSkip::NoLibrary);
    }
    report.windows = org.len();
    report.tier = tier;

    let (x0, y0, x1, y1) = bbox(&m.region, (w, h), MARGIN);
    let (bw, bh) = (x1 - x0, y1 - y0);
    let sub = |v: &[bool]| -> Vec<bool> {
        (0..bw * bh)
            .map(|i| v[(y0 + i / bw) * w + x0 + i % bw])
            .collect()
    };
    let region = sub(&m.region);
    let protect = sub(&m.protect);
    let paint = sub(&m.paint);
    let inside = edt_to(&invert(&region), (bw, bh));
    let outside = edt_to(&region, (bw, bh));
    let noise = smooth_noise((bw, bh), &mut rng);
    let wav: Vec<f32> = (0..bw * bh)
        .map(|i| inside[i] - outside[i] + BANK_PX + WAVE_PX * noise[i])
        .collect();
    let bed_mask: Vec<bool> = (0..bw * bh).map(|i| wav[i] > 0.0 && !protect[i]).collect();
    let wgt: Vec<f32> = (0..bw * bh)
        .map(|i| {
            if protect[i] {
                return 0.0;
            }
            let v = ((wav[i] + FEATHER_PX / 2.0) / FEATHER_PX).clamp(0.0, 1.0);
            if paint[i] { v.max(0.75) } else { v }
        })
        .collect();

    let n = patches.len() / (P * P);
    let means: Vec<[f32; 3]> = (0..n)
        .map(|k| {
            let mut s = [0.0f32; 3];
            for p in &patches[k * P * P..(k + 1) * P * P] {
                for c in 0..3 {
                    s[c] += f32::from(p[c]);
                }
            }
            s.map(|v| v / (P * P) as f32)
        })
        .collect();
    let lumas: Vec<f32> = means.iter().map(|&c| luma(c)).collect();
    let median = percentile(&lumas, 50.0);
    let light = mean_of(
        means
            .iter()
            .zip(&lumas)
            .filter(|(_, l)| **l >= median)
            .map(|(c, _)| *c),
    )
    .map(|v| v * SHALLOW_GAIN);
    let dark = mean_of(means.iter().copied()).map(|v| v * DEEP_GAIN);
    let depth = pixel_depth(mesh, (x0, y0), (bw, bh));
    let mut guide: Vec<[f32; 3]> = depth
        .iter()
        .map(|&d| {
            let t = (d / DEPTH_DARK_M).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            [0, 1, 2].map(|c| light[c] * (1.0 - t) + dark[c] * t)
        })
        .collect();
    blur3(&mut guide, (bw, bh), GUIDE_SIGMA);
    let fill = dilate(&bed_mask, (bw, bh), 4);
    let mut qrng = Rng::new(seed ^ 0x9e37_79b9_7f4a_7c15);
    let mut bed = quilt(&patches, &fill, (bw, bh), &mut qrng, &guide);
    let mut g24 = guide.clone();
    blur3(&mut g24, (bw, bh), TONE_SIGMA);
    let mut b24 = bed.clone();
    blur3(&mut b24, (bw, bh), TONE_SIGMA);
    for i in 0..bw * bh {
        if fill[i] {
            for c in 0..3 {
                bed[i][c] += TONE_W * (g24[i][c] - b24[i][c]);
            }
        }
    }
    let mut lo = bed.clone();
    blur3(&mut lo, (bw, bh), 2.0);
    for i in 0..bw * bh {
        for c in 0..3 {
            bed[i][c] = (lo[i][c] + DETAIL * (bed[i][c] - lo[i][c])).clamp(0.0, 255.0);
        }
    }

    let far = invert(&dilate(&m.region, (w, h), 8));
    let mut land = [0u64; 256];
    for (i, &f) in far.iter().enumerate() {
        if f {
            land[idx[i] as usize] += 1;
        }
    }
    let allowed: Vec<u8> = (0..256)
        .filter(|&i| land[i] >= 50 && !m.water_idx.contains(&(i as u8)))
        .map(|i| i as u8)
        .collect();
    let mut win = [0u64; 256];
    for &(y, x) in &org {
        for r in 0..P {
            for c in 0..P {
                win[idx[(y + r) * w + x + c] as usize] += 1;
            }
        }
    }
    let in_windows: Vec<u8> = allowed
        .iter()
        .copied()
        .filter(|&i| win[i as usize] >= 20)
        .collect();
    let warm: Vec<u8> = in_windows
        .iter()
        .copied()
        .filter(|&i| {
            let p = pal[i as usize];
            i32::from(p[1]) - i32::from(p[2]) >= MIN_GB
        })
        .collect();
    let bed_idx = if warm.len() >= 8 { warm } else { in_windows };
    let bed_idx = if bed_idx.is_empty() {
        allowed.clone()
    } else {
        bed_idx
    };
    report.allowed = allowed.len();
    report.bed_indices = bed_idx.len();
    if allowed.is_empty() {
        return Err(BedSkip::NoLibrary);
    }
    let mut bed_q = Quantiser::new(&pal, &bed_idx);
    let mut all_q = Quantiser::new(&pal, &allowed);
    let mut new = idx.clone();
    for r in 0..bh {
        for c in 0..bw {
            let i = r * bw + c;
            let wi = wgt[i];
            if wi <= 0.0 {
                continue;
            }
            let o = (y0 + r) * w + x0 + c;
            let src = pal[idx[o] as usize];
            let col = [0, 1, 2].map(|k| f32::from(src[k]) * (1.0 - wi) + bed[i][k] * wi);
            new[o] = if wi >= 1.0 {
                bed_q.nearest(col)
            } else {
                all_q.nearest(col)
            };
        }
    }
    report.changed_px = new.iter().zip(idx).filter(|(a, b)| a != b).count();
    Ok((new, report))
}

struct Masks {
    paint: Vec<bool>,
    region: Vec<bool>,
    protect: Vec<bool>,
    water_idx: Vec<u8>,
}

fn masks(idx: &[u8], pal: &[[u8; 3]], cov: &[bool], (w, h): (usize, usize)) -> Masks {
    let near = dilate(cov, (w, h), NEAR_PX);
    let (mut tot, mut inn) = ([0u64; 256], [0u64; 256]);
    for (i, &v) in idx.iter().enumerate() {
        tot[v as usize] += 1;
        if near[i] {
            inn[v as usize] += 1;
        }
    }
    let share: Vec<f32> = (0..256)
        .map(|i| inn[i] as f32 / tot[i].max(1) as f32)
        .collect();
    let water_idx: Vec<u8> = (0..256)
        .filter(|&i| share[i] >= WATER_SHARE && tot[i] >= MIN_WATER_PIXELS)
        .map(|i| i as u8)
        .collect();
    let mut is_water = [false; 256];
    for &i in &water_idx {
        is_water[i as usize] = true;
    }
    let weighted = |a: [u8; 3], b: [u8; 3]| -> f32 {
        (0..3)
            .map(|k| ((f32::from(a[k]) - f32::from(b[k])) * QUANT_W[k]).powi(2))
            .sum::<f32>()
            .sqrt()
    };
    let mut is_family = [false; 256];
    for i in 0..256 {
        is_family[i] = !is_water[i]
            && tot[i] >= MIN_WATER_PIXELS
            && share[i] >= FAMILY_SHARE
            && water_idx
                .iter()
                .any(|&j| weighted(pal[i], pal[j as usize]) <= FAMILY_DIST);
    }
    let close = dilate(cov, (w, h), FRINGE_PX);
    let core: Vec<bool> = (0..w * h)
        .map(|i| {
            let v = idx[i] as usize;
            (is_water[v] && near[i]) || (is_family[v] && close[i])
        })
        .collect();
    let water_idx: Vec<u8> = (0..256)
        .filter(|&i| is_water[i] || is_family[i])
        .map(|i| i as u8)
        .collect();
    for &i in &water_idx {
        is_water[i as usize] = true;
    }
    let mut sum = [0.0f64; 3];
    let mut n = 0.0f64;
    for &i in &water_idx {
        for c in 0..3 {
            sum[c] += f64::from(pal[i as usize][c]) * tot[i as usize] as f64;
        }
        n += tot[i as usize] as f64;
    }
    let mean = sum.map(|v| (v / n.max(1.0)) as f32);
    let tint = |c: [f32; 3]| {
        let g = (c[0] + c[1] + c[2]) / 3.0;
        c.map(|v| v - g)
    };
    let wt = tint(mean);
    let mut is_fringe = [false; 256];
    for i in 0..256 {
        let c = pal[i].map(f32::from);
        let t = tint(c);
        let sat = (c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]))
            / c[0].max(c[1]).max(c[2]).max(1.0);
        let dot = t[0] * wt[0] + t[1] * wt[1] + t[2] * wt[2];
        is_fringe[i] = !is_water[i]
            && tot[i] > 0
            && share[i] >= 0.5
            && luma(c) >= 110.0
            && (dot > 0.0 || sat < 0.12);
    }
    let core_near = dilate(&core, (w, h), FRINGE_PX);
    let paint: Vec<bool> = (0..w * h)
        .map(|i| core[i] || (is_fringe[idx[i] as usize] && core_near[i]))
        .collect();
    let other: Vec<bool> = (0..w * h).map(|i| cov[i] && !paint[i]).collect();
    let big = big_blobs(&other, (w, h), 150);
    let big = dilate(&big, (w, h), 2);
    let protect: Vec<bool> = (0..w * h).map(|i| big[i] && !core[i]).collect();
    let closed = erode(&dilate(&paint, (w, h), 2), (w, h), 2);
    let grown = dilate(&paint, (w, h), 4);
    let region: Vec<bool> = (0..w * h)
        .map(|i| (closed[i] || (cov[i] && grown[i])) && !protect[i])
        .collect();
    Masks {
        paint,
        region,
        protect,
        water_idx,
    }
}

fn smoothness(idx: &[u8], pal: &[[u8; 3]], m: &Masks, (w, h): (usize, usize)) -> f32 {
    let far = invert(&dilate(&m.region, (w, h), 8));
    let lum: Vec<f32> = pal.iter().map(|p| luma(p.map(f32::from))).collect();
    let (mut paint, mut land) = ((0.0f64, 0u64), (0.0f64, 0u64));
    for y in 0..h {
        for x in 0..w - 1 {
            let i = y * w + x;
            let d = f64::from((lum[idx[i] as usize] - lum[idx[i + 1] as usize]).abs());
            if m.paint[i] && m.paint[i + 1] {
                paint = (paint.0 + d, paint.1 + 1);
            } else if far[i] && far[i + 1] {
                land = (land.0 + d, land.1 + 1);
            }
        }
    }
    let mean = |(s, n): (f64, u64)| s / n.max(1) as f64;
    (mean(paint) / mean(land).max(1e-6)) as f32
}

fn big_blobs(mask: &[bool], (w, h): (usize, usize), min: usize) -> Vec<bool> {
    let mut label = vec![u32::MAX; w * h];
    let mut out = vec![false; w * h];
    let mut stack = Vec::new();
    let mut members = Vec::new();
    for s in 0..w * h {
        if !mask[s] || label[s] != u32::MAX {
            continue;
        }
        label[s] = s as u32;
        stack.push(s);
        members.clear();
        while let Some(p) = stack.pop() {
            members.push(p);
            let (x, y) = (p % w, p / w);
            let mut push = |q: usize| {
                if mask[q] && label[q] == u32::MAX {
                    label[q] = s as u32;
                    stack.push(q);
                }
            };
            if x > 0 {
                push(p - 1);
            }
            if x + 1 < w {
                push(p + 1);
            }
            if y > 0 {
                push(p - w);
            }
            if y + 1 < h {
                push(p + w);
            }
        }
        if members.len() >= min {
            for &p in &members {
                out[p] = true;
            }
        }
    }
    out
}

#[allow(clippy::type_complexity)]
fn bed_windows(
    rgb: &[[f32; 3]],
    (w, h): (usize, usize),
    avoid: &[bool],
    rng: &mut Rng,
) -> (Vec<[u8; 3]>, Vec<(usize, usize)>, u8) {
    if w < P + 2 * EDGE_CELLS * 8 || h < P + 2 * EDGE_CELLS * 8 {
        return (Vec::new(), Vec::new(), 0);
    }
    let mut b = rgb.to_vec();
    blur3(&mut b, (w, h), 2.0);
    let e = EDGE_CELLS * 8;
    let ok = |i: usize| {
        let (x, y) = (i % w, i / w);
        !avoid[i] && x >= e && y >= e && x < w - e && y < h - e
    };
    let sat = |c: [f32; 3]| {
        let mx = c[0].max(c[1]).max(c[2]);
        (mx - c[0].min(c[1]).min(c[2])) / mx.max(1.0)
    };
    let gravel: Vec<bool> = (0..w * h)
        .map(|i| {
            let c = b[i];
            let l = luma(c);
            ok(i) && sat(c) < 0.33 && (90.0..=170.0).contains(&l) && c[0] <= c[1] + 4.0
        })
        .collect();
    let grass: Vec<bool> = b.iter().map(|&c| sat(c) > 0.42).collect();
    let bedlike: Vec<bool> = (0..w * h)
        .map(|i| {
            let c = b[i];
            let l = luma(c);
            ok(i) && sat(c) <= 0.42 && (70.0..=190.0).contains(&l)
        })
        .collect();
    let gi = Integral::new(&gravel, (w, h));
    let si = Integral::new(&grass, (w, h));
    let bi = Integral::new(&bedlike, (w, h));
    let need = (GRAVEL_SHARE * (P * P) as f32).ceil() as u32;
    let ring = ((P + 2 * RING_PX).pow(2) - P * P) as f32;
    let tiers: [(u8, &Integral, bool); 3] = [(1, &gi, true), (2, &gi, false), (3, &bi, false)];
    for (tier, cnt, moor) in tiers {
        let mut org = Vec::new();
        for y in (0..=h - P).step_by(4) {
            for x in (0..=w - P).step_by(4) {
                if cnt.sum(x as i64, y as i64, P as i64) < need {
                    continue;
                }
                if moor {
                    let g = si.sum(
                        x as i64 - RING_PX as i64,
                        y as i64 - RING_PX as i64,
                        (P + 2 * RING_PX) as i64,
                    ) - si.sum(x as i64, y as i64, P as i64);
                    if (g as f32) < MOOR_SHARE * ring {
                        continue;
                    }
                }
                org.push((y, x));
            }
        }
        if org.len() < MIN_WINDOWS {
            continue;
        }
        let even: Vec<f32> = org
            .iter()
            .map(|&(y, x)| {
                let mut blocks = [0.0f32; (P / 8) * (P / 8)];
                for r in 0..P {
                    for c in 0..P {
                        blocks[(r / 8) * (P / 8) + c / 8] += luma(rgb[(y + r) * w + x + c]);
                    }
                }
                let m = blocks.iter().sum::<f32>() / blocks.len() as f32;
                (blocks.iter().map(|v| (v - m).powi(2)).sum::<f32>() / blocks.len() as f32).sqrt()
                    / 64.0
            })
            .collect();
        let cut = percentile(&even, 50.0);
        let mut calm: Vec<usize> = (0..org.len()).filter(|&k| even[k] <= cut).collect();
        if calm.len() > MAX_WINDOWS {
            let mut pick = rng.choose(calm.len(), MAX_WINDOWS);
            pick.sort_unstable();
            calm = pick.into_iter().map(|k| calm[k]).collect();
        }
        let org: Vec<(usize, usize)> = calm.iter().map(|&k| org[k]).collect();
        let mut px = Vec::with_capacity(org.len() * P * P);
        for &(y, x) in &org {
            for r in 0..P {
                for c in 0..P {
                    px.push(rgb[(y + r) * w + x + c].map(|v| v as u8));
                }
            }
        }
        return (px, org, tier);
    }
    (Vec::new(), Vec::new(), 0)
}

struct Integral {
    w: usize,
    h: usize,
    s: Vec<u32>,
}

impl Integral {
    fn new(mask: &[bool], (w, h): (usize, usize)) -> Self {
        let mut s = vec![0u32; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0u32;
            for x in 0..w {
                row += u32::from(mask[y * w + x]);
                s[(y + 1) * (w + 1) + x + 1] = s[y * (w + 1) + x + 1] + row;
            }
        }
        Self { w, h, s }
    }

    fn sum(&self, x: i64, y: i64, n: i64) -> u32 {
        let cx = |v: i64| v.clamp(0, self.w as i64) as usize;
        let cy = |v: i64| v.clamp(0, self.h as i64) as usize;
        let (a, b, c, d) = (cx(x), cx(x + n), cy(y), cy(y + n));
        let at = |x: usize, y: usize| self.s[y * (self.w + 1) + x];
        at(b, d) + at(a, c) - at(a, d) - at(b, c)
    }
}

#[cfg(test)]
fn min_cut_vertical(e: &[f32], size: (usize, usize)) -> Vec<bool> {
    let mut c = Vec::new();
    let mut back = Vec::new();
    let mut mask = Vec::new();
    min_cut_vertical_into(e, size, &mut c, &mut back, &mut mask);
    mask
}

fn min_cut_vertical_into(
    e: &[f32],
    (ww, hh): (usize, usize),
    c: &mut Vec<f32>,
    back: &mut Vec<i8>,
    mask: &mut Vec<bool>,
) {
    c.clear();
    c.extend_from_slice(e);
    back.clear();
    back.resize(ww * hh, 0);
    for y in 1..hh {
        for x in 0..ww {
            let p = |dx: i64| {
                let xx = x as i64 + dx;
                if xx < 0 || xx >= ww as i64 {
                    f32::INFINITY
                } else {
                    c[(y - 1) * ww + xx as usize]
                }
            };
            let (l, m, r) = (p(-1), p(0), p(1));
            let (k, v) = if l <= m && l <= r {
                (-1, l)
            } else if m <= r {
                (0, m)
            } else {
                (1, r)
            };
            c[y * ww + x] += v;
            back[y * ww + x] = k;
        }
    }
    let last = &c[(hh - 1) * ww..];
    let mut x = (0..ww).fold(0, |b, i| if last[i] < last[b] { i } else { b });
    mask.clear();
    mask.resize(ww * hh, false);
    for y in (0..hh).rev() {
        for v in &mut mask[y * ww + x..(y + 1) * ww] {
            *v = true;
        }
        x = (x as i64 + i64::from(back[y * ww + x])).clamp(0, ww as i64 - 1) as usize;
    }
}

fn positions(n: usize, p: usize, s: usize) -> Vec<usize> {
    let mut pos: Vec<usize> = (0..=n.saturating_sub(p)).step_by(s).collect();
    if *pos.last().unwrap() != n.saturating_sub(p) {
        pos.push(n - p);
    }
    pos
}

fn quilt(
    patches: &[[u8; 3]],
    region: &[bool],
    (w, h): (usize, usize),
    rng: &mut Rng,
    guide: &[[f32; 3]],
) -> Vec<[f32; 3]> {
    let s = P - O;
    let n = patches.len() / (P * P);
    let mut img = vec![[0.0f32; 3]; w * h];
    let mut filled = vec![false; w * h];
    if w < P || h < P || n == 0 {
        return img;
    }
    const B: usize = P / BLOCK;
    let pblocks: Vec<[f32; 3]> = (0..n * B * B)
        .map(|q| {
            let (k, by, bx) = (q / (B * B), q / B % B, q % B);
            let mut sum = [0.0f32; 3];
            for r in 0..BLOCK {
                for c in 0..BLOCK {
                    let p = patches[k * P * P + (by * BLOCK + r) * P + bx * BLOCK + c];
                    for ch in 0..3 {
                        sum[ch] += f32::from(p[ch]);
                    }
                }
            }
            sum.map(|v| v / (BLOCK * BLOCK) as f32)
        })
        .collect();
    let mut last = std::collections::HashMap::new();
    let mut fm = [false; P * P];
    let mut ex = [[0.0f32; 3]; P * P];
    let mut ov_px: Vec<usize> = Vec::with_capacity(P * P);
    let mut choose_scratch = Vec::with_capacity(n);
    let mut cand = Vec::with_capacity(K.min(n));
    let mut err = Vec::with_capacity(K.min(n));
    let mut ok = Vec::with_capacity(K.min(n));
    let mut cut_c = Vec::with_capacity(P * O);
    let mut cut_back = Vec::with_capacity(P * O);
    let mut cut_mask = Vec::with_capacity(P * O);
    let ys = positions(h, P, s);
    let xs = positions(w, P, s);
    for &y in &ys {
        for &x in &xs {
            if !(0..P).any(|r| {
                region[(y + r) * w + x..(y + r) * w + x + P]
                    .iter()
                    .any(|&v| v)
            }) {
                continue;
            }
            rng.choose_into(n, K.min(n), &mut choose_scratch, &mut cand);
            for nb in [
                (y.wrapping_sub(s), x),
                (y, x.wrapping_sub(s)),
                (y.wrapping_sub(s), x.wrapping_sub(s)),
                (y.wrapping_sub(s), x + s),
            ] {
                if let Some(&k) = last.get(&nb) {
                    cand.retain(|&c| c != k);
                }
            }
            if cand.is_empty() {
                cand.push(0);
            }
            ov_px.clear();
            for r in 0..P {
                for c in 0..P {
                    let o = (y + r) * w + x + c;
                    fm[r * P + c] = filled[o];
                    ex[r * P + c] = img[o];
                    if filled[o] {
                        ov_px.push(r * P + c);
                    }
                }
            }
            let any = !ov_px.is_empty();
            let g: [[f32; 3]; B * B] = std::array::from_fn(|q| {
                let (by, bx) = (q / B, q % B);
                let mut sum = [0.0f32; 3];
                for r in 0..BLOCK {
                    for c in 0..BLOCK {
                        let v = guide[(y + by * BLOCK + r) * w + x + bx * BLOCK + c];
                        for ch in 0..3 {
                            sum[ch] += v[ch];
                        }
                    }
                }
                sum.map(|v| v / (BLOCK * BLOCK) as f32)
            });
            err.clear();
            err.extend(cand.iter().map(|&k| {
                let pk = &patches[k * P * P..(k + 1) * P * P];
                let mut ov = 0.0f32;
                for &q in &ov_px {
                    let (a, e) = (pk[q], ex[q]);
                    let d = [0, 1, 2].map(|c| f32::from(a[c]) - e[c]);
                    ov += d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                }
                let ov = if any { ov / ov_px.len() as f32 } else { 0.0 };
                let mut tr = 0.0f32;
                for q in 0..B * B {
                    let (pb, gb) = (pblocks[k * B * B + q], g[q]);
                    tr += (0..3).map(|c| (pb[c] - gb[c]).powi(2)).sum::<f32>();
                }
                let tr = tr / (B * B) as f32;
                if any {
                    (1.0 - ALPHA) * ov + ALPHA * tr
                } else {
                    tr
                }
            }));
            let best = err.iter().copied().fold(f32::INFINITY, f32::min);
            ok.clear();
            ok.extend((0..cand.len()).filter(|&i| err[i] <= best * (1.0 + TOL) + 1e-3));
            let k = cand[ok[rng.below(ok.len())]];
            let pk = &patches[k * P * P..(k + 1) * P * P];
            let mut keep = [true; P * P];
            if any {
                let mut e = [0.0f32; P * P];
                for q in 0..P * P {
                    let d = [0, 1, 2].map(|c| f32::from(pk[q][c]) - ex[q][c]);
                    e[q] = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                }
                let mut strip = [0.0f32; P * O];
                if x > 0 && (0..P).any(|r| (0..O).any(|c| fm[r * P + c])) {
                    for q in 0..P * O {
                        let (r, c) = (q / O, q % O);
                        strip[q] = if fm[r * P + c] { e[r * P + c] } else { 0.0 };
                    }
                    min_cut_vertical_into(&strip, (O, P), &mut cut_c, &mut cut_back, &mut cut_mask);
                    for r in 0..P {
                        for c in 0..O {
                            keep[r * P + c] &= cut_mask[r * O + c];
                        }
                    }
                }
                if y > 0 && (0..O).any(|r| (0..P).any(|c| fm[r * P + c])) {
                    for q in 0..P * O {
                        let (c, r) = (q / O, q % O);
                        strip[q] = if fm[r * P + c] { e[r * P + c] } else { 0.0 };
                    }
                    min_cut_vertical_into(&strip, (O, P), &mut cut_c, &mut cut_back, &mut cut_mask);
                    for c in 0..P {
                        for r in 0..O {
                            keep[r * P + c] &= cut_mask[c * O + r];
                        }
                    }
                }
            }
            for r in 0..P {
                for c in 0..P {
                    let q = r * P + c;
                    if !fm[q] || keep[q] {
                        let o = (y + r) * w + x + c;
                        img[o] = pk[q].map(f32::from);
                        filled[o] = true;
                    }
                }
            }
            last.insert((y, x), k);
        }
    }
    img
}

fn count(v: &[bool]) -> usize {
    v.iter().filter(|&&b| b).count()
}

fn invert(v: &[bool]) -> Vec<bool> {
    v.iter().map(|&b| !b).collect()
}

fn luma(c: [f32; 3]) -> f32 {
    c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2]
}

fn mean_of(it: impl Iterator<Item = [f32; 3]>) -> [f32; 3] {
    let (mut s, mut n) = ([0.0f32; 3], 0.0f32);
    for c in it {
        for k in 0..3 {
            s[k] += c[k];
        }
        n += 1.0;
    }
    s.map(|v| v / n.max(1.0))
}

fn percentile(v: &[f32], q: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    let pos = q / 100.0 * (s.len() - 1) as f32;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    s[lo] + (s[hi] - s[lo]) * (pos - lo as f32)
}

fn bbox(mask: &[bool], (w, h): (usize, usize), margin: usize) -> (usize, usize, usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (i, &m) in mask.iter().enumerate() {
        if m {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    let x0 = x0.saturating_sub(margin);
    let y0 = y0.saturating_sub(margin);
    let mut x1 = (x1 + margin).min(w);
    let mut y1 = (y1 + margin).min(h);
    if x1 - x0 < P {
        x1 = (x0 + P).min(w);
    }
    if y1 - y0 < P {
        y1 = (y0 + P).min(h);
    }
    (x0, y0, x1, y1)
}

fn l1_distance(seed: &[bool], (w, h): (usize, usize)) -> Vec<u32> {
    let big = u32::MAX / 2;
    let mut d: Vec<u32> = seed.iter().map(|&s| if s { 0 } else { big }).collect();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x > 0 {
                d[i] = d[i].min(d[i - 1] + 1);
            }
            if y > 0 {
                d[i] = d[i].min(d[i - w] + 1);
            }
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if x + 1 < w {
                d[i] = d[i].min(d[i + 1] + 1);
            }
            if y + 1 < h {
                d[i] = d[i].min(d[i + w] + 1);
            }
        }
    }
    d
}

fn dilate(mask: &[bool], size: (usize, usize), n: u32) -> Vec<bool> {
    l1_distance(mask, size)
        .into_iter()
        .map(|d| d <= n)
        .collect()
}

fn erode(mask: &[bool], (w, h): (usize, usize), n: u32) -> Vec<bool> {
    let d = l1_distance(&invert(mask), (w, h));
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let edge = (x + 1).min(y + 1).min(w - x).min(h - y) as u32;
            d[i] > n && edge > n
        })
        .collect()
}

fn edt_to(seed: &[bool], size: (usize, usize)) -> Vec<f32> {
    super::grid::squared_distance_f64(seed, size)
        .into_iter()
        .map(|d| d.sqrt().min(1e4) as f32)
        .collect()
}

fn box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let wlf = wl as f32;
    let m = ((12.0 * sigma * sigma - n * wlf * wlf - 4.0 * n * wlf - 3.0 * n) / (-4.0 * wlf - 4.0))
        .round() as i64;
    [0, 1, 2].map(|i| (((if i < m { wl } else { wu }) - 1) / 2).max(0) as usize)
}

fn blur3(img: &mut [[f32; 3]], (w, h): (usize, usize), sigma: f32) {
    let mut tmp = vec![[0.0f32; 3]; w.max(h)];
    for r in box_radii(sigma) {
        if r == 0 {
            continue;
        }
        for y in 0..h {
            box_line(&mut img[y * w..(y + 1) * w], 1, w, r, &mut tmp);
        }
        for x in 0..w {
            box_line(&mut img[x..], w, h, r, &mut tmp);
        }
    }
}

fn box_line(v: &mut [[f32; 3]], stride: usize, n: usize, r: usize, tmp: &mut [[f32; 3]]) {
    for i in 0..n {
        tmp[i] = v[i * stride];
    }
    let at = |i: i64| tmp[i.clamp(0, n as i64 - 1) as usize];
    let mut s = [0.0f32; 3];
    for i in -(r as i64)..=r as i64 {
        let p = at(i);
        for c in 0..3 {
            s[c] += p[c];
        }
    }
    let inv = 1.0 / (2 * r + 1) as f32;
    for i in 0..n {
        v[i * stride] = s.map(|x| x * inv);
        let (add, sub) = (at(i as i64 + r as i64 + 1), at(i as i64 - r as i64));
        for c in 0..3 {
            s[c] += add[c] - sub[c];
        }
    }
}

fn blur1(img: &mut [f32], (w, h): (usize, usize), sigma: f32) {
    let mut tmp = vec![0.0f32; w.max(h)];
    for r in box_radii(sigma) {
        if r == 0 {
            continue;
        }
        for y in 0..h {
            box_line1(&mut img[y * w..(y + 1) * w], 1, w, r, &mut tmp);
        }
        for x in 0..w {
            box_line1(&mut img[x..], w, h, r, &mut tmp);
        }
    }
}

fn box_line1(v: &mut [f32], stride: usize, n: usize, r: usize, tmp: &mut [f32]) {
    for i in 0..n {
        tmp[i] = v[i * stride];
    }
    let at = |i: i64| tmp[i.clamp(0, n as i64 - 1) as usize];
    let mut s = 0.0f32;
    for i in -(r as i64)..=r as i64 {
        s += at(i);
    }
    let inv = 1.0 / (2 * r + 1) as f32;
    for i in 0..n {
        v[i * stride] = s * inv;
        s += at(i as i64 + r as i64 + 1) - at(i as i64 - r as i64);
    }
}

fn smooth_noise((w, h): (usize, usize), rng: &mut Rng) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    let mut n = vec![0.0f32; w * h];
    for (sigma, weight) in [(3.0f32, 0.6f32), (8.0, 0.4)] {
        for v in &mut n {
            *v = rng.normal();
        }
        blur1(&mut n, (w, h), sigma);
        let m = n.iter().copied().sum::<f32>() / (w * h) as f32;
        let sd = (n.iter().map(|&v| (v - m).powi(2)).sum::<f32>() / (w * h) as f32)
            .sqrt()
            .max(1e-6);
        for (o, &v) in out.iter_mut().zip(&n) {
            *o += weight * v / sd;
        }
    }
    out
}

fn pixel_depth(
    mesh: &mandrake_tag::MeshTag,
    (x0, y0): (usize, usize),
    (w, h): (usize, usize),
) -> Vec<f32> {
    let (cw, ch) = (usize::from(mesh.cell_width), usize::from(mesh.cell_height));
    let dv = |c: i64, r: i64| {
        let c = c.clamp(0, cw as i64 - 1) as usize;
        let r = r.clamp(0, ch as i64 - 1) as usize;
        mesh.cell_at(c, r).map_or(0.0, |v| {
            if v.flags & 0x1800 != 0 {
                (f32::from(v.media_height) - f32::from(v.physical_height)).max(0.0) / UNITS_PER_M
            } else {
                0.0
            }
        })
    };
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let fy = (y0 + y) as f32 / 8.0;
        let (r, ty) = (fy.floor() as i64, fy.fract());
        for x in 0..w {
            let fx = (x0 + x) as f32 / 8.0;
            let (c, tx) = (fx.floor() as i64, fx.fract());
            let top = dv(c, r) * (1.0 - tx) + dv(c + 1, r) * tx;
            let bot = dv(c, r + 1) * (1.0 - tx) + dv(c + 1, r + 1) * tx;
            out.push(top * (1.0 - ty) + bot * ty);
        }
    }
    out
}

struct Quantiser {
    colors: Vec<([f32; 3], u8)>,
    memo: std::collections::HashMap<[u8; 3], u8>,
}

impl Quantiser {
    fn new(pal: &[[u8; 3]], allowed: &[u8]) -> Self {
        Self {
            colors: allowed
                .iter()
                .map(|&i| (pal[i as usize].map(f32::from), i))
                .collect(),
            memo: Default::default(),
        }
    }

    fn nearest(&mut self, c: [f32; 3]) -> u8 {
        let key = c.map(|v| v.round().clamp(0.0, 255.0) as u8);
        if let Some(&i) = self.memo.get(&key) {
            return i;
        }
        let k = key.map(f32::from);
        let mut best = (f32::INFINITY, 0u8);
        for &(p, i) in &self.colors {
            let d: f32 = (0..3).map(|j| ((k[j] - p[j]) * QUANT_W[j]).powi(2)).sum();
            if d < best.0 {
                best = (d, i);
            }
        }
        self.memo.insert(key, best.1);
        best.1
    }
}

struct Rng([u64; 4]);

impl Rng {
    fn new(seed: u64) -> Self {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            x ^ (x >> 31)
        };
        Self([next(), next(), next(), next()])
    }

    fn next(&mut self) -> u64 {
        let s = &mut self.0;
        let out = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }

    fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next()) * n as u128) >> 64) as usize
    }

    fn unit(&mut self) -> f32 {
        ((self.next() >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }

    fn normal(&mut self) -> f32 {
        let (a, b) = (self.unit(), self.unit());
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
    }

    fn choose(&mut self, n: usize, k: usize) -> Vec<usize> {
        let mut scratch = Vec::with_capacity(n);
        let mut out = Vec::with_capacity(k.min(n));
        self.choose_into(n, k, &mut scratch, &mut out);
        out
    }

    fn choose_into(&mut self, n: usize, k: usize, scratch: &mut Vec<usize>, out: &mut Vec<usize>) {
        scratch.clear();
        scratch.extend(0..n);
        let count = k.min(n);
        for i in 0..count {
            let j = i + self.below(n - i);
            scratch.swap(i, j);
        }
        out.clear();
        out.extend_from_slice(&scratch[..count]);
    }
}

pub fn stock_map(id: &[u8; 4]) -> Option<(mandrake_tag::MeshTag, TerrainTexture, MediaCoverage)> {
    use mandrake_common::FourCC;
    let game = mandrake_common::data_dirs::myth2_dir();
    let files = mandrake_tag::engine_tag_files(&game, &[]);
    let vfs = mandrake_common::NativeVfs::default_cwd();
    let extra: Vec<std::path::PathBuf> = std::env::var("M2_WATER_BED_PLUGINS")
        .map(|v| v.split(':').map(std::path::PathBuf::from).collect())
        .unwrap_or_default();
    let archives: Vec<mandrake_tag::archive::TagArchive> = files
        .paths
        .iter()
        .chain(&extra)
        .filter_map(|p| mandrake_tag::archive::TagArchive::open(&vfs, &p.to_string_lossy()).ok())
        .collect();
    let tag = |g: [u8; 4], id: FourCC| {
        archives.iter().rev().find_map(|a| {
            a.find_tag_by_id(FourCC::new(g), id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    };
    let data = tag(*b"mesh", FourCC::new(*id))?;
    let mesh = mandrake_tag::MeshTag::from_bytes(&data).ok()?;
    let texture = TerrainTexture::from_dot256(
        &tag(*b".256", mesh.landscape_tag)?,
        u32::from(mesh.submesh_w),
        u32::from(mesh.submesh_h),
    )
    .ok()?;
    let cov = MediaCoverage::from_mesh_bytes(&data, mesh.cell_width, mesh.cell_height);
    Some((mesh, texture, cov))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_match_brute_force() {
        let (w, h) = (23, 17);
        let seed: Vec<bool> = (0..w * h).map(|i| (i * 7919) % 53 == 0).collect();
        let e = edt_to(&seed, (w, h));
        let l = l1_distance(&seed, (w, h));
        for i in 0..w * h {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            let (mut be, mut bl) = (f32::INFINITY, u32::MAX);
            for j in 0..w * h {
                if seed[j] {
                    let (dx, dy) = ((j % w) as i64 - x, (j / w) as i64 - y);
                    be = be.min(((dx * dx + dy * dy) as f32).sqrt());
                    bl = bl.min((dx.abs() + dy.abs()) as u32);
                }
            }
            assert!((e[i] - be).abs() < 1e-3, "{i}: {} vs {be}", e[i]);
            assert_eq!(l[i], bl);
        }
    }

    #[test]
    fn min_cut_follows_the_cheap_column() {
        let e: Vec<f32> = (0..20)
            .map(|q| if q % 4 == 2 { 0.0 } else { 9.0 })
            .collect();
        let m = min_cut_vertical(&e, (4, 5));
        for y in 0..5 {
            assert_eq!(&m[y * 4..y * 4 + 4], &[false, false, true, true]);
        }
    }

    #[test]
    fn blur_keeps_the_mean_and_spreads() {
        let (w, h) = (64, 64);
        let mut img = vec![[0.0f32; 3]; w * h];
        img[32 * w + 32] = [1000.0; 3];
        blur3(&mut img, (w, h), 4.0);
        let s: f32 = img.iter().map(|v| v[0]).sum();
        assert!((s - 1000.0).abs() < 1.0);
        let row: Vec<f32> = (0..w).map(|x| img[32 * w + x][0]).collect();
        let t: f32 = row.iter().sum();
        let var: f32 = row
            .iter()
            .enumerate()
            .map(|(x, v)| v * (x as f32 - 32.0).powi(2))
            .sum::<f32>()
            / t;
        assert!((var.sqrt() - 4.0).abs() < 0.3, "{}", var.sqrt());
    }

    #[test]
    fn rng_is_fixed() {
        let mut a = Rng::new(BED_SEED);
        let mut b = Rng::new(BED_SEED);
        for _ in 0..100 {
            assert_eq!(a.next(), b.next());
        }
        let c = a.choose(10, 10);
        let mut s = c.clone();
        s.sort_unstable();
        assert_eq!(s, (0..10).collect::<Vec<_>>());
    }

    fn palette(tex: &TerrainTexture) -> Vec<[u8; 3]> {
        (0..256)
            .map(|i| {
                if i < tex.color_count {
                    tex.palette16[i].map(|v| (v >> 8) as u8)
                } else {
                    [0; 3]
                }
            })
            .collect()
    }

    fn coverage_mask(tex: &TerrainTexture, cov: &MediaCoverage) -> Vec<bool> {
        let w = tex.width as usize;
        (0..tex.color_index.len())
            .map(|i| cov.pixel_is_media((i % w) as i32, (i / w) as i32))
            .collect()
    }

    fn check_output(tex: &TerrainTexture, cov: &MediaCoverage, new: &[u8], water: &[u8]) {
        let (w, h) = (tex.width as usize, tex.height as usize);
        let c = coverage_mask(tex, cov);
        let near = dilate(&c, (w, h), 40);
        let mut land = [0u64; 256];
        for (i, &v) in tex.color_index.iter().enumerate() {
            if !near[i] {
                land[v as usize] += 1;
            }
        }
        let mut left = 0;
        for i in 0..w * h {
            if new[i] != tex.color_index[i] {
                assert!(near[i], "pixel {i} changed far from the water");
                assert!(
                    land[new[i] as usize] > 0,
                    "pixel {i}: index {} is no land index",
                    new[i]
                );
                assert!(
                    !water.contains(&new[i]),
                    "pixel {i}: water index {}",
                    new[i]
                );
            }
            if c[i] && water.contains(&new[i]) {
                left += 1;
            }
        }
        assert!(left * 200 < count(&c), "{left} water-paint pixels left");
    }

    #[test]
    fn synthetic_river_is_repainted_with_land_indices() {
        let (cw, ch) = (48u16, 40u16);
        let (w, h) = (usize::from(cw) * 8, usize::from(ch) * 8);
        let mut palette16 = [[0u16; 3]; 256];
        for k in 0..16u16 {
            palette16[k as usize] = [100 + 3 * k, 104 + 3 * k, 96 + 3 * k].map(|v| v << 8);
            palette16[16 + k as usize] = [70 + k, 120 + 2 * k, 30].map(|v| v << 8);
            palette16[32 + k as usize] = [30 + k, 50 + k, 140 + 2 * k].map(|v| v << 8);
        }
        let mut rng = Rng::new(1);
        let river = |x: usize, y: usize| {
            let cx = w as f32 / 2.0 + 30.0 * (y as f32 / 40.0).sin();
            (x as f32 - cx).abs() < 24.0
        };
        let color_index: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let n = rng.below(16) as u8;
                if river(x, y) {
                    32 + n
                } else if (x / 48 + y / 48) % 3 == 0 {
                    16 + n
                } else {
                    n
                }
            })
            .collect();
        let tex = TerrainTexture {
            width: w as u32,
            height: h as u32,
            color_index,
            shadow_map: vec![0; w * h],
            palette16,
            color_count: 48,
            blood_remap: None,
        };
        let rows = (0..h)
            .map(|y| {
                let xs: Vec<usize> = (0..w).filter(|&x| river(x, y)).collect();
                vec![xs[0] as i16 - 1, *xs.last().unwrap() as i16, w as i16]
            })
            .collect();
        let cov = MediaCoverage {
            width: w as i32,
            height: h as i32,
            rows,
        };
        let mut mesh = crate::water::synthetic_mesh(cw, ch, |_, _| 0);
        for r in 0..usize::from(ch) {
            for c in 0..usize::from(cw) {
                if river(c * 8, r * 8) {
                    let cell = &mut mesh.cells[r * usize::from(cw) + c];
                    cell.flags |= 0x0800;
                    cell.media_height = 600;
                }
            }
        }
        let (new, report) = repaint_water_paint(&tex, &mesh, &cov, BED_SEED).expect("a repaint");
        let water: Vec<u8> = (32..48).collect();
        let m = masks(
            &tex.color_index,
            &palette(&tex),
            &coverage_mask(&tex, &cov),
            (w, h),
        );
        assert_eq!(m.water_idx, water, "the blue paint, and only it");
        assert!(report.windows >= MIN_WINDOWS, "{report:?}");
        check_output(&tex, &cov, &new, &water);
        let (again, _) = repaint_water_paint(&tex, &mesh, &cov, BED_SEED).unwrap();
        assert!(again == new, "deterministic");
        for i in 0..w * h {
            if river(i % w, i / w) {
                assert!(new[i] < 16, "pixel {i}: {}", new[i]);
            }
        }
    }

    #[test]
    #[ignore = "needs the game data; cargo test --release -p mandrake-render --lib water::bed -- --ignored"]
    fn stock_maps_classify_and_repaint() {
        for (id, green) in [(b"neme", true), (b"85gi", false)] {
            let Some((mesh, tex, cov)) = stock_map(id) else {
                eprintln!(
                    "Notice: {} not found, skipping",
                    String::from_utf8_lossy(id)
                );
                continue;
            };
            let pal = palette(&tex);
            let (w, h) = (tex.width as usize, tex.height as usize);
            let m = masks(&tex.color_index, &pal, &coverage_mask(&tex, &cov), (w, h));
            assert!(m.water_idx.len() >= 20, "{:?}", m.water_idx);
            let off: Vec<(u8, [u8; 3])> = m
                .water_idx
                .iter()
                .map(|&i| (i, pal[i as usize]))
                .filter(|&(_, p)| {
                    let [r, g, b] = p.map(i32::from);
                    let hue = if green {
                        g > r && g > b
                    } else {
                        b > r && b >= g
                    };
                    let sat =
                        (r.max(g).max(b) - r.min(g).min(b)) as f32 / r.max(g).max(b).max(1) as f32;
                    !hue && sat >= 0.2
                })
                .collect();
            assert!(
                off.is_empty(),
                "{}: not water paint: {off:?}",
                String::from_utf8_lossy(id)
            );
            let (new, _) = repaint_water_paint(&tex, &mesh, &cov, BED_SEED).unwrap();
            check_output(&tex, &cov, &new, &m.water_idx);
            let (again, _) = repaint_water_paint(&tex, &mesh, &cov, BED_SEED).unwrap();
            assert!(again == new, "deterministic");
        }
    }
}
