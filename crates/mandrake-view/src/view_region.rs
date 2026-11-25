#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionView {
    pub eye: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub extents: [f32; 4],
    pub focal: f32,
    pub center: [i32; 2],
    pub size: [i32; 2],
}

#[derive(Clone, Copy)]
pub(crate) struct ViewIntegers<'a> {
    view: &'a RegionView,
    eye: [i32; 3],
    rotation: [[i32; 3]; 3],
}

impl ViewIntegers<'_> {
    pub(crate) fn view_coords(&self, p: [f32; 3]) -> [i32; 3] {
        let d = [0, 1, 2].map(|k| p[k] as i32 - self.eye[k]);
        self.rotation.map(|a| {
            a[0].wrapping_mul(d[0])
                .wrapping_add(a[1].wrapping_mul(d[1]))
                .wrapping_add(a[2].wrapping_mul(d[2]))
                >> 14
        })
    }

    pub(crate) fn screen_point_and_depth(&self, p: [f32; 3]) -> Option<([i32; 2], i32)> {
        let [right, up, depth] = self.view_coords(p);
        if depth >= 0 {
            return None;
        }
        let k = self.view.focal / depth as f32;
        let sx = (self.view.center[0] as f32 - right as f32 * k) as i32;
        let sy = (k * up as f32 + self.view.center[1] as f32) as i32;
        if sx != i32::from(sx as i16) || sy != i32::from(sy as i16) {
            return None;
        }
        Some(([sx, sy], depth))
    }

    fn screen_point(&self, p: [f32; 3]) -> Option<[i32; 2]> {
        self.screen_point_and_depth(p).map(|(s, _)| s)
    }

    fn clip_word(&self, p: [f32; 3], top: i32, near_media: bool) -> u16 {
        let Some([sx, sy]) = self.screen_point(p) else {
            return 0x700;
        };
        if near_media {
            return 0x600;
        }
        let mut clip: u16 = if sy < self.view.size[1] {
            if sy <= top { 0x610 } else { 0x600 }
        } else if sy > top {
            0x640
        } else {
            0x650
        };
        if sx <= 0 {
            clip |= 1;
        }
        if sx >= self.view.size[0] {
            clip |= 4;
        }
        clip
    }
}

impl RegionView {
    fn rotation_shorts(&self) -> [[i32; 3]; 3] {
        self.axes.map(|a| a.map(|v| (v * 16384.0).round() as i32))
    }

    pub fn view_coords(&self, p: [f32; 3]) -> [i32; 3] {
        self.integers().view_coords(p)
    }

    pub(crate) fn integers(&self) -> ViewIntegers<'_> {
        ViewIntegers {
            view: self,
            eye: self.eye.map(|v| v as i32),
            rotation: self.rotation_shorts(),
        }
    }

    pub fn screen_point(&self, p: [f32; 3]) -> Option<[i32; 2]> {
        self.integers().screen_point(p)
    }

    pub fn clip_word(&self, p: [f32; 3], top: i32, near_media: bool) -> u16 {
        self.integers().clip_word(p, top, near_media)
    }

    pub fn dynamic_top(&self) -> i32 {
        -self.size[1] / 10
    }

    pub fn ground_point(&self, u: f32, v: f32, height: i32) -> [i32; 2] {
        self.ground_point_scaled(u, v, height, 1.0)
    }

    pub fn ground_point_scaled(&self, u: f32, v: f32, height: i32, scale: f32) -> [i32; 2] {
        let [top, left, bottom, right] = self.extents;
        let a = (right - left) * u + left;
        let b = (bottom - top) * v + top;
        let s = 11.25 * 16384.0;
        let dir =
            [0, 1, 2].map(|k| (self.axes[0][k] * a + self.axes[1][k] * b - self.axes[2][k]) * s);
        let (mut dx, mut dy) = (dir[0], dir[1]);
        if dir[2] != 0.0 {
            let t = (height as f32 - self.eye[2].trunc()) / dir[2];
            if t > 0.0 && t < 1.0 {
                dx = dx * t * scale;
                dy = dy * t * scale;
            }
        }
        [
            (dx + self.eye[0].trunc()) as i32,
            (dy + self.eye[1].trunc()) as i32,
        ]
    }
}

pub struct RegionMap<'a> {
    pub width: i32,
    pub height: i32,
    pub min_cell_height: i32,
    pub small_region: bool,
    pub vertex: &'a dyn Fn(i32, i32) -> [f32; 3],
    pub model_vertex: &'a dyn Fn(i32, i32) -> bool,
    pub dynamic_features: bool,
    pub near_reflective_media: &'a dyn Fn(i32, i32) -> bool,
}

pub const DYNAMIC_TOP_CORNER_SCALE: f32 = 1.75;

pub const REFLECTIVE_MEDIA_REACH: u32 = 1000;

pub fn reflective_media_distances(
    width: usize,
    height: usize,
    seed: impl Fn(usize, usize) -> bool,
) -> Vec<u32> {
    let (w, h) = (width as u32, height as u32);
    let far = (!(h.wrapping_mul(h))).wrapping_sub(w.wrapping_mul(w));
    let mut d = vec![far; width * height];
    for row in 0..height {
        for col in 0..width {
            if seed(col, row) {
                d[row * width + col] = 0;
            }
        }
    }
    for col in 0..width {
        let mut step = 1u32;
        for row in 1..height {
            let new = d[(row - 1) * width + col].wrapping_add(step);
            if new < d[row * width + col] {
                d[row * width + col] = new;
                step += 2;
            } else {
                step = 1;
            }
        }
        step = 1;
        for row in (0..height.saturating_sub(1)).rev() {
            let new = d[(row + 1) * width + col].wrapping_add(step);
            if new < d[row * width + col] {
                d[row * width + col] = new;
                step += 2;
            } else {
                step = 1;
            }
        }
    }
    let (mut v, mut z, mut f) = (vec![0u32; width], vec![0u32; width], vec![0u32; width]);
    let sq = |a: u32| a.wrapping_mul(a);
    for row in d.chunks_exact_mut(width.max(1)) {
        f.copy_from_slice(row);
        v[0] = 0;
        z[0] = 0;
        let mut k = 0usize;
        for q in 1..w {
            let fq = f[q as usize];
            loop {
                let (r, s) = (v[k], z[k]);
                let fr = f[r as usize];
                if sq(s.wrapping_sub(r)).wrapping_add(fr) <= sq(s.wrapping_sub(q)).wrapping_add(fq)
                {
                    let sep = sq(q).wrapping_add(fq).wrapping_sub(sq(r)).wrapping_sub(fr)
                        / ((q - r) * 2)
                        + 1;
                    if sep < w {
                        k += 1;
                        v[k] = q;
                        z[k] = sep;
                    }
                    break;
                }
                if k == 0 {
                    v[0] = q;
                    break;
                }
                k -= 1;
            }
        }
        for x in (0..w).rev() {
            row[x as usize] = sq(x.wrapping_sub(v[k])).wrapping_add(f[v[k] as usize]);
            if z[k] == x && k > 0 {
                k -= 1;
            }
        }
    }
    d
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewRegion {
    pub first_row: i32,
    pub last_row: i32,
    pub spans: Vec<(i16, i16)>,
    pub built: Vec<(i16, i16)>,
    pub rect_last_row: i32,
}

impl ViewRegion {
    pub fn contains(&self, col: i32, row: i32) -> bool {
        if row < self.first_row || row >= self.last_row {
            return false;
        }
        let Some(&(first, last)) = self.spans.get((row - self.first_row) as usize) else {
            return false;
        };
        col >= i32::from(first) && col < i32::from(last)
    }

    pub fn vertex_inside(&self, col: i32, row: i32) -> bool {
        if row < self.first_row || row >= self.last_row {
            return false;
        }
        let n = (self.last_row - self.first_row) as usize;
        self.spans
            .iter()
            .take(n)
            .any(|&(first, last)| col >= i32::from(first) && col < i32::from(last))
    }

    pub fn vertex_built(&self, col: i32, row: i32) -> bool {
        let Ok(i) = usize::try_from(row - self.first_row) else {
            return false;
        };
        self.built
            .get(i)
            .is_some_and(|&(first, last)| col >= i32::from(first) && col <= i32::from(last))
    }

    pub fn span(&self, row: i32) -> Option<(i16, i16)> {
        if row < self.first_row || row >= self.last_row {
            return None;
        }
        self.spans.get((row - self.first_row) as usize).copied()
    }
}

pub fn convex_hull(points: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let n = points.len();
    if n <= 1 {
        return Vec::new();
    }
    let mut p: Vec<[i64; 2]> = points.iter().map(|q| [q[0] as i64, q[1] as i64]).collect();
    let mut best = 0usize;
    for i in 1..n {
        if p[i][1] < p[best][1] || (p[i][1] == p[best][1] && p[i][0] < p[best][0]) {
            best = i;
        }
    }
    p.swap(0, best);
    let pivot = p[0];
    for q in &mut p {
        q[0] -= pivot[0];
        q[1] -= pivot[1];
    }
    if n > 2 {
        for k in 1..n - 1 {
            let mut b = k;
            for j in k + 1..n {
                let (pj, pb) = (p[j], p[b]);
                let l = pj[0] * pb[1];
                let r = pb[0] * pj[1];
                if l < r
                    || (l == r && pj[0] * pj[0] + pj[1] * pj[1] > pb[0] * pb[0] + pb[1] * pb[1])
                {
                    b = j;
                }
            }
            p.swap(k, b);
        }
    }
    let mut kept = vec![p[0], p[1]];
    let mut last = p[1];
    for q in p.iter().take(n).skip(2) {
        if last[0] * q[1] != last[1] * q[0] {
            kept.push(*q);
            last = *q;
        }
    }
    if kept.len() == 2 {
        return Vec::new();
    }
    let mut stack = vec![kept[0], kept[1]];
    for &q in &kept[2..] {
        while stack.len() >= 2 {
            let top = stack[stack.len() - 1];
            let prev = stack[stack.len() - 2];
            let a = (q[0] - top[0]) * (prev[1] - top[1]);
            let b = (prev[0] - top[0]) * (q[1] - top[1]);
            if a - b >= 0 {
                stack.pop();
            } else {
                break;
            }
        }
        stack.push(q);
    }
    stack
        .into_iter()
        .map(|q| [(q[0] + pivot[0]) as i32, (q[1] + pivot[1]) as i32])
        .collect()
}

pub fn dda(x0: i32, y0: i32, x1: i32, y1: i32, mut visit: impl FnMut(i32, i32)) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (sx, sy) = (dx.signum(), dy.signum());
    let (ax, ay) = (dx.abs(), dy.abs());
    let (mut x, mut y, mut err) = (x0, y0, 0);
    if ax > ay {
        for _ in 0..=ax {
            visit(x, y);
            x += sx;
            err += ay;
            if err >= ax {
                y += sy;
                err -= ax;
            }
        }
    } else {
        for _ in 0..=ay {
            visit(x, y);
            y += sy;
            err += ax;
            if err >= ay {
                x += sx;
                err -= ay;
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalkedRegion {
    pub hull: Vec<[i32; 2]>,
    pub rect: [i32; 6],
    pub rows: (i32, i32),
    pub cols: (i32, i32),
    pub spans: Vec<(i16, i16)>,
}

pub fn walk_region(view: &RegionView, map: &RegionMap<'_>) -> WalkedRegion {
    let (lim, half, max) = if map.small_region {
        (0x1e, 0x20, 0x40)
    } else {
        (0x7e, 0x80, 0x100)
    };
    let eye = [view.eye[0] as i32, view.eye[1] as i32];
    let square = |c: [i32; 2]| {
        [
            [c[0] - 0x600, c[1] - 0x600],
            [c[0] + 0x600, c[1] - 0x600],
            [c[0] + 0x600, c[1] + 0x600],
            [c[0] - 0x600, c[1] + 0x600],
        ]
    };
    let mut points: Vec<[i32; 2]> = square(eye).to_vec();
    for i in 0..4 {
        let scale = if map.dynamic_features && i >> 1 == 0 {
            DYNAMIC_TOP_CORNER_SCALE
        } else {
            1.0
        };
        let g =
            view.ground_point_scaled((i & 1) as f32, (i >> 1) as f32, map.min_cell_height, scale);
        points.extend(square(g));
    }
    let hull: Vec<[i32; 2]> = convex_hull(&points)
        .into_iter()
        .map(|q| [q[0] >> 9, q[1] >> 9])
        .collect();
    let mut walked = WalkedRegion {
        hull: hull.clone(),
        rows: (0x7fff, -0x8000),
        cols: (0x7fff, -0x8000),
        ..Default::default()
    };
    if hull.is_empty() {
        return walked;
    }
    let (mut a2, mut cx, mut cy) = (0i64, 0i64, 0i64);
    for i in 0..hull.len() {
        let (p, q) = (hull[i], hull[(i + 1) % hull.len()]);
        let cross = p[0] as i64 * q[1] as i64 - p[1] as i64 * q[0] as i64;
        a2 += cross;
        cx += (p[0] + q[0]) as i64 * cross;
        cy += (p[1] + q[1]) as i64 * cross;
    }
    if a2 == 0 {
        return walked;
    }
    let (mut col, mut row) = ((cx / (3 * a2)) as i32, (cy / (3 * a2)) as i32);
    let (ec, er) = (eye[0] >> 9, eye[1] >> 9);
    let (dc, dr) = (col - ec, row - er);
    let dist = ((dc as i64 * dc as i64 + dr as i64 * dr as i64) as f64).sqrt() as f32;
    if dist > lim as f32 {
        let s = lim as f32 / dist;
        col = (dc as f32 * s + ec as f32) as i32;
        row = (dr as f32 * s + er as f32) as i32;
    }
    let around = [row - half, col - half, row + half, col + half];
    let mut bounds = [0x7fff, 0x7fff, -0x8000, -0x8000];
    for q in &hull {
        bounds[0] = bounds[0].min(q[1]);
        bounds[1] = bounds[1].min(q[0]);
        bounds[2] = bounds[2].max(q[1]);
        bounds[3] = bounds[3].max(q[0]);
    }
    let intersect = |a: [i32; 4], b: [i32; 4]| {
        let r = [
            a[0].max(b[0]),
            a[1].max(b[1]),
            a[2].min(b[2]),
            a[3].min(b[3]),
        ];
        if r[0] < r[2] && r[1] < r[3] {
            r
        } else {
            [0; 4]
        }
    };
    let r = intersect(intersect(around, bounds), [0, 0, map.height, map.width]);
    if r[0] >= r[2] || r[1] >= r[3] {
        return walked;
    }
    let (col0, row0) = (r[1] & !1, r[0] & !1);
    let width = ((r[3] + 1 - col0) as u32).min(max as u32) as i32;
    let height = ((r[2] + 1 - row0) as u32).min(max as u32) as i32;
    let (last_col, last_row) = (col0 + width - 2, row0 + height - 2);
    walked.rect = [col0, row0, width, height, last_col, last_row];
    let mut spans = vec![(0x7fffi16, -0x8000i16); height.max(0) as usize];
    let (mut rows, mut cols) = ((0x7fff, -0x8000), (0x7fff, -0x8000));
    for i in 0..hull.len() {
        let (p, q) = (hull[i], hull[(i + 1) % hull.len()]);
        dda(p[0], p[1], q[0], q[1], |c, w| {
            if w >= map.height - 1 {
                return;
            }
            let k = w - row0;
            if k < 0 || k >= height {
                return;
            }
            let c = c.max(col0).min(last_col);
            let s = &mut spans[k as usize];
            s.0 = s.0.min(c as i16);
            s.1 = s.1.max(c as i16);
            cols = (cols.0.min(c), cols.1.max(c));
            rows = (rows.0.min(w), rows.1.max(w));
        });
    }
    walked.rows = rows;
    walked.cols = cols;
    walked.spans = spans;
    walked
}

pub fn build_view_region(view: &RegionView, map: &RegionMap<'_>) -> ViewRegion {
    let walked = walk_region(view, map);
    let top = if map.dynamic_features {
        view.dynamic_top()
    } else {
        0
    };
    let integers = view.integers();
    final_spans(&walked, &|col, row| {
        let near_media = map.dynamic_features && (map.near_reflective_media)(col, row);
        (map.model_vertex)(col, row)
            || integers.clip_word((map.vertex)(col, row), top, near_media) & 0x115 == 0
    })
}

pub fn final_spans(walked: &WalkedRegion, counts: &dyn Fn(i32, i32) -> bool) -> ViewRegion {
    let (first_row, last_row) = walked.rows;
    if walked.hull.is_empty() || last_row < first_row {
        return ViewRegion::default();
    }
    let row0 = walked.rect[1];
    let mut spans = Vec::with_capacity((last_row - first_row + 1) as usize);
    let mut built = Vec::with_capacity((last_row - first_row + 1) as usize);
    for row in first_row..=last_row {
        let (first, last) = walked.spans[(row - row0) as usize];
        built.push((first, last));
        let mut columns = i32::from(first)..=i32::from(last);
        let span = match columns.find(|&col| counts(col, row)) {
            Some(left) => (
                left as i16,
                columns.rfind(|&col| counts(col, row)).unwrap_or(left) as i16,
            ),
            None => (0x7fff, -0x8000),
        };
        spans.push(span);
    }
    let n = spans.len();
    let mut widened = vec![(0x7fffi16, -0x8000i16); n];
    for (i, &(first, last)) in spans.iter().enumerate() {
        if first > last {
            continue;
        }
        let c0 = (i32::from(first) - 2).max(walked.cols.0) as i16;
        let c1 = (i32::from(last) + 2).min(walked.cols.1) as i16;
        for w in &mut widened[i.saturating_sub(2)..=(i + 2).min(n - 1)] {
            w.0 = w.0.min(c0);
            w.1 = w.1.max(c1);
        }
    }
    ViewRegion {
        first_row,
        last_row,
        spans: widened,
        built,
        rect_last_row: walked.rect[5],
    }
}

pub const TFL_REGION_SIDE: i32 = 0x20;

const TFL_NEIGHBOURS: [(i32, i32); 8] = [
    (1, 1),
    (1, -1),
    (-1, -1),
    (-1, 1),
    (0, 1),
    (1, 0),
    (0, -1),
    (-1, 0),
];

pub fn build_tfl_view_region(
    view: &RegionView,
    facing: u16,
    (width, height): (i32, i32),
    (cosine, sine): (&[i16; 1024], &[i16; 1024]),
    vertex: &dyn Fn(i32, i32) -> [f32; 3],
) -> ViewRegion {
    const SIDE: i32 = TFL_REGION_SIDE;
    let eye = [view.eye[0] as i32, view.eye[1] as i32];
    let back = ((i32::from(facing) + 0x8020) >> 6 & 0x3ff) as usize;
    let mut col0 = (eye[0] >> 10) - ((i32::from(cosine[back]) * 15) >> 14) - 0x10;
    let mut row0 = (eye[1] >> 10) - ((i32::from(sine[back]) * 15) >> 14) - 0x10;
    col0 = col0.max(0);
    if width < col0 + SIDE {
        col0 = width - SIDE;
    }
    row0 = row0.max(0);
    if height < row0 + SIDE {
        row0 = height - SIDE;
    }
    let ahead = ((i32::from(facing) + 0x20) >> 6 & 0x3ff) as usize;
    let start = [
        ((((i32::from(cosine[ahead]) * 0x1800) >> 14) + eye[0]) >> 10) - col0,
        ((((i32::from(sine[ahead]) * 0x1800) >> 14) + eye[1]) >> 10) - row0,
    ]
    .map(|c| c.clamp(0, SIDE - 1));
    let integers = view.integers();
    let mut spans = [(0x7fff_i32, -0x8000_i32); SIDE as usize];
    let mut visited = [0u32; SIDE as usize];
    visited[start[1] as usize] |= 1 << start[0];
    let mut queue = [(0i32, 0i32, false); 0x80];
    queue[0] = (start[0], start[1], false);
    let (mut head, mut tail) = (0usize, 1usize);
    let mut found = false;
    loop {
        let (c, r, late) = queue[head];
        head += 1;
        if head == 0x80 {
            if !found {
                break;
            }
            head = 0;
        }
        let (col, row) = (c + col0, r + row0);
        let span = &mut spans[r as usize];
        span.0 = span.0.min(col);
        span.1 = span.1.max(col);
        let counts = integers.clip_word(vertex(col, row), 0, false) & 0x155 == 0;
        let outside_after = if counts {
            found = true;
            false
        } else {
            found
        };
        if !outside_after || !late {
            for (dc, dr) in TFL_NEIGHBOURS {
                let (nc, nr) = (c + dc, r + dr);
                if !(0..SIDE).contains(&nc) || !(0..SIDE).contains(&nr) {
                    continue;
                }
                if visited[nr as usize] & 1 << nc != 0 {
                    continue;
                }
                queue[tail] = (nc, nr, outside_after);
                tail += 1;
                if tail == 0x80 {
                    tail = 0;
                }
                if tail == head {
                    break;
                }
                if !outside_after {
                    visited[nr as usize] |= 1 << nc;
                }
            }
        }
        if head == tail {
            break;
        }
    }
    let Some(first) = spans.iter().position(|s| s.1 >= 0) else {
        return ViewRegion::default();
    };
    let last = spans.iter().rposition(|s| s.1 >= 0).unwrap();
    let to_span = |(a, b): (i32, i32)| {
        if b < 0 { (0, 0) } else { (a as i16, b as i16) }
    };
    ViewRegion {
        first_row: row0 + first as i32,
        last_row: row0 + last as i32,
        spans: spans[first..=last].iter().copied().map(to_span).collect(),
        built: Vec::new(),
        rect_last_row: row0 + last as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_view() -> RegionView {
        let m = [
            -12207.0, -4962.0, 9736.0, 10928.0, -5543.0, 10875.0, 0.0, 14597.0, 7439.0,
        ]
        .map(|v: f32| v / 16384.0);
        RegionView {
            eye: [40692.0, 23138.0, 1771.0],
            axes: [[m[0], m[3], m[6]], [m[1], m[4], m[7]], [m[2], m[5], m[8]]],
            extents: [0.191519, -0.466308, -0.420510, 0.466308],
            focal: 1097.9875,
            center: [512, 211],
            size: [1024, 672],
        }
    }

    #[test]
    fn final_spans_equal_asking_about_every_column() {
        let walked = WalkedRegion {
            hull: vec![[0, 0]],
            rect: [0, 0, 41, 8, 39, 7],
            rows: (0, 7),
            cols: (0, 39),
            spans: vec![
                (0, 39),
                (3, 3),
                (5, 30),
                (0, 39),
                (10, 20),
                (0, 39),
                (0, 39),
                (2, 9),
            ],
        };
        let mut seed = 0x2545_f491u32;
        for _ in 0..200 {
            let mut hit = [[false; 40]; 8];
            for row in &mut hit {
                seed = seed.wrapping_mul(0x9e37_79b1).wrapping_add(11);
                for _ in 0..(seed >> 29) {
                    seed = seed.wrapping_mul(0x9e37_79b1).wrapping_add(11);
                    row[(seed >> 16) as usize % 40] = true;
                }
            }
            let counts = |c: i32, r: i32| hit[r as usize][c as usize];
            let every = walked.spans.iter().enumerate().map(|(r, &(a, b))| {
                let columns = i32::from(a)..=i32::from(b);
                let mut span = (0x7fffi16, -0x8000i16);
                for c in columns.filter(|&c| counts(c, r as i32)) {
                    span = (span.0.min(c as i16), span.1.max(c as i16));
                }
                span
            });
            let n = walked.spans.len();
            let mut widened = vec![(0x7fffi16, -0x8000i16); n];
            for (i, (first, last)) in every.enumerate() {
                if first <= last {
                    let (c0, c1) = ((first - 2).max(0), (last + 2).min(39));
                    for w in &mut widened[i.saturating_sub(2)..=(i + 2).min(n - 1)] {
                        *w = (w.0.min(c0), w.1.max(c1));
                    }
                }
            }
            assert_eq!(final_spans(&walked, &counts).spans, widened);
        }
    }

    #[test]
    fn engine_sine_matches_the_old_build() {
        let mut sine = [0i16; 1024];
        for (i, v) in sine.iter_mut().enumerate() {
            let x = i as f64 * core::f64::consts::TAU * 0.0009765625;
            *v = match i {
                0 | 512 => 0,
                256 => 16384,
                768 => -16384,
                _ => (x.sin() * 16384.0 + 0.5) as i16,
            };
        }
        assert_eq!(sine, mandrake_math::engine::SINE);
    }

    #[test]
    fn region_contains_rule() {
        let r = ViewRegion {
            first_row: 10,
            last_row: 12,
            spans: vec![(5, 8), (0x7fff, -0x8000), (1, 2)],
            built: Vec::new(),
            rect_last_row: 12,
        };
        assert!(r.contains(5, 10) && r.contains(7, 10));
        assert!(!r.contains(8, 10) && !r.contains(4, 10));
        assert!(!r.contains(5, 11));
        assert!(!r.contains(1, 12));
        assert!(r.vertex_inside(5, 11) && !r.vertex_inside(1, 10) && !r.vertex_inside(5, 12));
    }

    #[test]
    fn dda_walks_the_engine_cells() {
        let mut cells = Vec::new();
        dda(0, 0, 4, 2, |x, y| cells.push((x, y)));
        assert_eq!(cells, vec![(0, 0), (1, 0), (2, 1), (3, 1), (4, 2)]);
        let mut cells = Vec::new();
        dda(2, 3, 2, 3, |x, y| cells.push((x, y)));
        assert_eq!(cells, vec![(2, 3)]);
    }

    #[test]
    fn clip_word_rules() {
        let v = engine_view();
        let ahead = [0, 1, 2].map(|k| v.eye[k] - v.axes[2][k] * 4000.0);
        assert_eq!(v.clip_word(ahead, 0, false), 0x600);
        let behind = [0, 1, 2].map(|k| v.eye[k] + v.axes[2][k] * 4000.0);
        assert_eq!(v.clip_word(behind, 0, false), 0x700);
        assert_eq!(v.clip_word(behind, 0, true), 0x700);
    }

    #[test]
    fn dynamic_top_moves_only_the_top_edge() {
        let v = engine_view();
        assert_eq!(v.dynamic_top(), -v.size[1] / 10);
        let mut p = [0, 1, 2].map(|k| v.eye[k] - v.axes[2][k] * 4000.0);
        loop {
            let [_, sy] = v.screen_point(p).unwrap();
            if sy < 0 && sy > v.dynamic_top() {
                break;
            }
            p = [0, 1, 2].map(|k| p[k] + v.axes[1][k] * if sy >= 0 { 8.0 } else { -2.0 });
        }
        assert_eq!(v.clip_word(p, 0, false) & 0x115, 0x10);
        assert_eq!(v.clip_word(p, v.dynamic_top(), false), 0x600);
        let far_up = [0, 1, 2].map(|k| p[k] + v.axes[1][k] * 2000.0);
        assert_ne!(v.clip_word(far_up, v.dynamic_top(), false) & 0x115, 0);
        assert_eq!(v.clip_word(far_up, v.dynamic_top(), true), 0x600);
    }

    #[test]
    fn reflective_media_distances_are_squared_cell_distances() {
        let (w, h) = (23usize, 17usize);
        let seeds = [(3usize, 2usize), (19, 15), (12, 8)];
        let d = reflective_media_distances(w, h, |c, r| seeds.contains(&(c, r)));
        for r in 0..h {
            for c in 0..w {
                let want = seeds
                    .iter()
                    .map(|&(x, y)| (x.abs_diff(c).pow(2) + y.abs_diff(r).pow(2)) as u32)
                    .min()
                    .unwrap();
                assert_eq!(d[r * w + c], want, "({c}, {r})");
            }
        }
        let none = reflective_media_distances(w, h, |_, _| false);
        assert!(none.iter().all(|&v| v > REFLECTIVE_MEDIA_REACH));
    }

    #[test]
    fn walk_region_off_map_empty_intersection_has_no_negative_column() {
        let mut view = engine_view();
        view.eye = [-200_000.0, 0.0, 1771.0];
        let vertex = |c: i32, r: i32| [(c << 9) as f32, (r << 9) as f32, 0.0];
        let none = |_: i32, _: i32| false;
        let map = RegionMap {
            width: 32,
            height: 32,
            min_cell_height: 0,
            small_region: false,
            dynamic_features: false,
            near_reflective_media: &none,
            vertex: &vertex,
            model_vertex: &none,
        };
        let walked = walk_region(&view, &map);
        assert!(walked.rows.1 < walked.rows.0);
        assert!(walked.spans.is_empty());
        assert_eq!(build_view_region(&view, &map), ViewRegion::default());
    }
}
