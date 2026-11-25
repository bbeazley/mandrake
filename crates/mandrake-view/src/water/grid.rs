pub fn coverage_for(
    info: &[[f32; 4]],
    size: (usize, usize),
    mesh: &mandrake_tag::MeshTag,
) -> Vec<f32> {
    coverage(info, size, mesh)
        .iter()
        .map(|&g| f32::from(g) / 255.0 * 16.0 - 8.0)
        .collect()
}

pub fn grid_info_for(mesh: &mandrake_tag::MeshTag, mesh_flags: u32) -> Vec<[f32; 4]> {
    grid_info(mesh, mesh_flags).2
}

pub fn grid_info(mesh: &mandrake_tag::MeshTag, mesh_flags: u32) -> (u32, u32, Vec<[f32; 4]>) {
    let (cw, ch) = (i64::from(mesh.cell_width), i64::from(mesh.cell_height));
    if cw == 0 || ch == 0 {
        return (0, 0, Vec::new());
    }
    let (w, h) = (cw as u32 + 1, ch as u32 + 1);
    let cell = |c: i64, r: i64| {
        mesh.cell_at(c.clamp(0, cw - 1) as usize, r.clamp(0, ch - 1) as usize)
            .copied()
            .unwrap_or_default()
    };
    let media = |c: i64, r: i64| {
        let v = cell(c, r);
        crate::media::is_media_height_vertex(v.flags, v.model_index, mesh_flags)
            .then_some(v.media_height)
    };
    let mut info = Vec::with_capacity((w * h) as usize);
    for r in 0..i64::from(h) {
        for c in 0..i64::from(w) {
            let physical = f32::from(cell(c, r).physical_height) / crate::WORLD_UNITS_PER_CELL;
            let (level, flag) = if let Some(h) = media(c, r) {
                (Some(h), 1.0)
            } else {
                let mut min_dist_sq = i64::MAX;
                let mut max_level: Option<i16> = None;
                for y in (r - 2).max(0)..=(r + 2).min(ch) {
                    let dy = y - r;
                    for x in (c - 2).max(0)..=(c + 2).min(cw) {
                        if let Some(m) = media(x, y) {
                            let dx = x - c;
                            min_dist_sq = min_dist_sq.min(dx * dx + dy * dy);
                            max_level = Some(max_level.map_or(m, |cur| cur.max(m)));
                        }
                    }
                }
                let nearest = if min_dist_sq == i64::MAX {
                    f32::INFINITY
                } else {
                    (min_dist_sq as f32).sqrt()
                };
                (max_level, 0.9 * (1.0 - nearest / 2.0).clamp(0.0, 1.0))
            };
            info.push([
                physical,
                level.map_or(physical, |l| f32::from(l) / crate::WORLD_UNITS_PER_CELL),
                if level.is_some() { 1.0 } else { 0.0 },
                flag,
            ]);
        }
    }
    (w, h, info)
}

pub fn chamfer(seed: &[bool], (w, h): (usize, usize)) -> Vec<u32> {
    let big = u32::MAX / 4;
    let mut d: Vec<u32> = seed.iter().map(|&x| if x { 0 } else { big }).collect();
    let at = |c: i64, r: i64| {
        (c >= 0 && r >= 0 && (c as usize) < w && (r as usize) < h)
            .then(|| r as usize * w + c as usize)
    };
    let mut relax = |i: usize, sign: i64| {
        let (c, r) = ((i % w) as i64, (i / w) as i64);
        let mut best = d[i];
        for (dc, dr, cost) in [
            (sign, 0, 3),
            (0, sign, 3),
            (sign, sign, 4),
            (-sign, sign, 4),
        ] {
            if let Some(j) = at(c + dc, r + dr) {
                best = best.min(d[j].saturating_add(cost));
            }
        }
        d[i] = best;
    };
    for i in 0..w * h {
        relax(i, -1);
    }
    for i in (0..w * h).rev() {
        relax(i, 1);
    }
    d
}

pub(crate) fn squared_distance_f64(seed: &[bool], (w, h): (usize, usize)) -> Vec<f64> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    const FAR: f64 = 1.0e20;
    fn line(f: &[f64], out: &mut [f64], v: &mut [usize], z: &mut [f64]) {
        let n = f.len();
        if n == 0 {
            return;
        }
        let inf = f64::INFINITY;
        let mut k = 0usize;
        v[0] = 0;
        z[0] = -inf;
        z[1] = inf;
        for q in 1..n {
            let fq = f[q] + (q * q) as f64;
            loop {
                let p = v[k];
                let s = (fq - (f[p] + (p * p) as f64)) / (2.0 * q as f64 - 2.0 * p as f64);
                if s <= z[k] && k > 0 {
                    k -= 1;
                    continue;
                }
                if s <= z[k] {
                    v[0] = q;
                    z[0] = -inf;
                    z[1] = inf;
                    break;
                }
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = inf;
                break;
            }
        }
        let mut k = 0usize;
        for (q, dq) in out.iter_mut().enumerate() {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            let dx = q as f64 - p as f64;
            *dq = dx * dx + f[p];
        }
    }
    let mut d: Vec<f64> = seed.iter().map(|&x| if x { 0.0 } else { FAR }).collect();
    let n = w.max(h);
    let (mut v, mut z) = (vec![0usize; n], vec![0.0f64; n + 1]);
    let (mut f, mut out) = (vec![0.0f64; n], vec![0.0f64; n]);
    for c in 0..w {
        for r in 0..h {
            f[r] = d[r * w + c];
        }
        line(&f[..h], &mut out[..h], &mut v, &mut z);
        for r in 0..h {
            d[r * w + c] = out[r];
        }
    }
    for r in 0..h {
        f[..w].copy_from_slice(&d[r * w..(r + 1) * w]);
        line(&f[..w], &mut out[..w], &mut v, &mut z);
        d[r * w..(r + 1) * w].copy_from_slice(&out[..w]);
    }
    d
}

pub fn squared_distance(seed: &[bool], size: (usize, usize)) -> Vec<f32> {
    squared_distance_f64(seed, size)
        .into_iter()
        .map(|x| x.min(1.0e12) as f32)
        .collect()
}

pub fn coverage(
    info: &[[f32; 4]],
    (w, h): (usize, usize),
    mesh: &mandrake_tag::MeshTag,
) -> Vec<u8> {
    let model = |c: usize, r: usize| {
        [(0usize, 0usize), (1, 0), (0, 1), (1, 1)]
            .iter()
            .any(|&(dc, dr)| {
                c >= dc
                    && r >= dr
                    && mesh
                        .cell_at(c - dc, r - dr)
                        .is_some_and(|v| v.model_index >= 0)
            })
    };
    let has: Vec<bool> = (0..w * h)
        .map(|i| info.get(i).is_some_and(|v| v[2] > 0.5) || model(i % w, i / w))
        .collect();
    let none: Vec<bool> = has.iter().map(|&x| !x).collect();
    let inside = chamfer(&none, (w, h));
    let outside = chamfer(&has, (w, h));
    (0..w * h)
        .map(|i| {
            let sd = if has[i] {
                inside[i].min(48) as f32 / 3.0
            } else {
                -(outside[i].min(48) as f32 / 3.0)
            };
            ((sd + 8.0) / 16.0 * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_info_handles_empty_mesh_without_panicking() {
        let empty = super::super::synthetic_mesh(0, 0, |_, _| 0);
        assert_eq!(grid_info(&empty, 0), (0, 0, Vec::new()));
        let empty_w = super::super::synthetic_mesh(0, 4, |_, _| 0);
        assert_eq!(grid_info(&empty_w, 0), (0, 0, Vec::new()));
        let empty_h = super::super::synthetic_mesh(4, 0, |_, _| 0);
        assert_eq!(grid_info(&empty_h, 0), (0, 0, Vec::new()));
        assert!(squared_distance(&[], (0, 0)).is_empty());
    }

    #[test]
    fn grid_info_single_pass_sets_water_and_shore_flags() {
        let mut mesh = super::super::synthetic_mesh(5, 5, |_, _| 0);
        mesh.cells[2 * 5 + 2].flags = crate::media::CELL_FLAG_MEDIA_VERTEX;
        mesh.cells[2 * 5 + 2].media_height = 512;
        let (w, h, info) = grid_info(&mesh, 0);
        assert_eq!((w, h), (6, 6));
        let at = |c: usize, r: usize| info[r * w as usize + c];
        assert_eq!(at(2, 2), [0.0, 1.0, 1.0, 1.0]);
        assert!((at(3, 2)[3] - 0.45).abs() < 1e-6);
        assert_eq!(at(3, 2)[1], 1.0);
        assert_eq!(at(4, 2)[1], 1.0);
        assert_eq!(at(4, 2)[3], 0.0);
        assert_eq!(at(5, 5), [0.0, 0.0, 0.0, 0.0]);
    }
}
