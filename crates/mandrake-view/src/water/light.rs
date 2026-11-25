pub fn normalized(d: [f32; 3]) -> [f32; 3] {
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    [d[0] / len, d[1] / len, d[2] / len]
}

/// Toward the light in render space from its elevation and map azimuth (degrees; azimuth 0 along
/// +column, render -x, and 90 along +row, render +z), as [`fit_light`]'s log gives them.
pub fn light_from_angles(elevation: f32, azimuth: f32) -> [f32; 3] {
    let (e, a) = (elevation.to_radians(), azimuth.to_radians());
    [-e.cos() * a.cos(), e.sin(), e.cos() * a.sin()]
}

/// The light direction recovered from a map's baked landscape shading ([`fit_light`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightFit {
    /// Toward the light, render space (y up), normalized.
    pub direction: [f32; 3],
    /// Share of the per-cell shading variance the fit explains.
    pub r2: f32,
    /// Standard deviation of the per-cell shading level (0..255 levels).
    pub spread: f32,
    /// Dry cells fitted.
    pub cells: usize,
}

/// Shading spread (levels of 256) below which a map counts as flat-shaded.
pub const FLAT_SHADING_SPREAD: f32 = 2.0;

/// Myth II has no light direction (`docs/research/LIGHTING.md`): the landscape's shading is baked
/// into the `.256` (one shading level per texel, [`crate::landscape::LandscapeTexels::shade`]).
/// Where it was baked from a light, Lambert's law holds: `shade ~ a + k N.L`. This fits
/// `shade = c0 + c . N` by least squares over the dry cells (no media flags `0x1800`, not on the
/// map's edge), each cell's mean shading level against its normal from the four corner heights
/// (`cell +0x00`), and returns `L = c / |c|` in render space with the fit's R². `None` when
/// there are too few cells or `shade` does not cover the map.
pub fn fit_light(
    mesh: &mandrake_tag::MeshTag,
    shade: &[u8],
    (width, height): (u32, u32),
) -> Option<LightFit> {
    let (cw, ch) = (usize::from(mesh.cell_width), usize::from(mesh.cell_height));
    let (w, h) = (width as usize, height as usize);
    if cw < 3 || ch < 3 || shade.len() < w * h || w < cw || h < ch {
        return None;
    }
    let height_at = |c: usize, r: usize| {
        f64::from(
            mesh.cell_at(c.min(cw - 1), r.min(ch - 1))
                .map_or(0, |v| v.physical_height),
        ) / f64::from(crate::WORLD_UNITS_PER_CELL)
    };
    let mut samples: Vec<([f64; 4], f64)> = Vec::new();
    for r in 1..ch - 1 {
        for c in 1..cw - 1 {
            let wet = [(c, r), (c + 1, r), (c, r + 1), (c + 1, r + 1)]
                .iter()
                .any(|&(x, y)| {
                    mesh.cell_at(x.min(cw - 1), y.min(ch - 1))
                        .is_none_or(|v| v.flags & 0x1800 != 0)
                });
            if wet {
                continue;
            }
            let (x0, x1) = (c * w / cw, ((c + 1) * w / cw).max(c * w / cw + 1));
            let (y0, y1) = (r * h / ch, ((r + 1) * h / ch).max(r * h / ch + 1));
            let step = ((x1 - x0) / 4).max(1);
            let (mut sum, mut n) = (0.0f64, 0.0f64);
            for y in (y0..y1).step_by(step) {
                for x in (x0..x1).step_by(step) {
                    sum += f64::from(shade[y * w + x]);
                    n += 1.0;
                }
            }
            if n == 0.0 {
                continue;
            }
            // Height slopes in cells per cell along the column and the row.
            let dcol = (height_at(c + 1, r) + height_at(c + 1, r + 1)
                - height_at(c, r)
                - height_at(c, r + 1))
                * 0.5;
            let drow = (height_at(c, r + 1) + height_at(c + 1, r + 1)
                - height_at(c, r)
                - height_at(c + 1, r))
                * 0.5;
            // Map space (column, row, up) normal (-dcol, -drow, 1); render x falls with the
            // column, z grows with the row.
            let len = (dcol * dcol + drow * drow + 1.0).sqrt();
            let n_render = [dcol / len, 1.0 / len, -drow / len];
            samples.push(([1.0, n_render[0], n_render[1], n_render[2]], sum / n));
        }
    }
    if samples.len() < 16 {
        return None;
    }
    let mut ata = [[0.0f64; 4]; 4];
    let mut atb = [0.0f64; 4];
    for (x, y) in &samples {
        for i in 0..4 {
            for j in 0..4 {
                ata[i][j] += x[i] * x[j];
            }
            atb[i] += x[i] * y;
        }
    }
    // A touch of ridge on the normal's terms: on a flat map they are nearly collinear with the
    // constant.
    for (i, row) in ata.iter_mut().enumerate().skip(1) {
        row[i] += 1e-6 * samples.len() as f64;
    }
    let c = solve4(ata, atb)?;
    let mean = samples.iter().map(|(_, y)| y).sum::<f64>() / samples.len() as f64;
    let (mut ss_tot, mut ss_res) = (0.0f64, 0.0f64);
    for (x, y) in &samples {
        let fit: f64 = (0..4).map(|i| x[i] * c[i]).sum();
        ss_tot += (y - mean).powi(2);
        ss_res += (y - fit).powi(2);
    }
    let spread = (ss_tot / samples.len() as f64).sqrt() as f32;
    let r2 = if ss_tot > 0.0 {
        (1.0 - ss_res / ss_tot) as f32
    } else {
        0.0
    };
    let len = (c[1] * c[1] + c[2] * c[2] + c[3] * c[3]).sqrt().max(1e-12);
    Some(LightFit {
        direction: [
            (c[1] / len) as f32,
            (c[2] / len) as f32,
            (c[3] / len) as f32,
        ],
        r2,
        spread,
        cells: samples.len(),
    })
}

/// `a x = b` for a 4 x 4 system, Gaussian elimination with partial pivoting.
pub fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> Option<[f64; 4]> {
    for col in 0..4 {
        let pivot = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..4 {
            let f = a[row][col] / a[col][col];
            let pivot_row = a[col];
            for (k, v) in a[row].iter_mut().enumerate().skip(col) {
                *v -= f * pivot_row[k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0f64; 4];
    for row in (0..4).rev() {
        let s: f64 = (row + 1..4).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    Some(x)
}
