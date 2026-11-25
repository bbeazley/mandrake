use crate::draw_site;
use crate::engine_rng::Unmodelled;

pub const TERRAIN_TYPES: usize = 16;
pub const TERRAIN_WALKING_IMPASSABLE: usize = 14;
pub const TERRAIN_FLYING_IMPASSABLE: usize = 15;
pub const CELL_MEDIA: u16 = 0x1800;
pub const CELL_KILLED: u16 = 0x4000;

pub fn cell_terrain_types(flags: u16) -> (usize, usize) {
    (usize::from((flags >> 4) & 0xf), usize::from(flags & 0xf))
}

const NEIGHBOURS_ODD: [(i32, i32, u8); 8] = [
    (0, 0, 0),
    (0, 0, 1),
    (0, 0, 0),
    (0, 0, 0),
    (0, -1, 3),
    (-1, 0, 3),
    (0, 1, 2),
    (1, 0, 2),
];
const NEIGHBOURS_EVEN: [(i32, i32, u8); 8] = [
    (0, 0, 2),
    (0, 0, 3),
    (0, 0, 0),
    (0, 0, 0),
    (0, -1, 0),
    (-1, 0, 1),
    (0, 1, 1),
    (1, 0, 0),
];
const NEIGHBOUR_BITS: [(usize, u8); 6] = [
    (0, 0x01),
    (1, 0x02),
    (4, 0x10),
    (5, 0x20),
    (6, 0x40),
    (7, 0x80),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeshCells {
    pub width: i32,
    pub height: i32,
    pub flags: Vec<u16>,
    pub models: Vec<i16>,
}

impl MeshCells {
    pub fn from_mesh(mesh: &mandrake_tag::MeshTag) -> Self {
        let n = mesh.cells.len();
        Self {
            width: i32::from(mesh.cell_width),
            height: i32::from(mesh.cell_height),
            flags: mesh.cells.iter().map(|c| c.flags).collect(),
            models: vec![-1; n],
        }
    }

    pub fn cell_index(&self, x: i32, y: i32) -> usize {
        let cx = if (x as u32) < self.width as u32 {
            x
        } else if x >= self.width {
            self.width - 1
        } else {
            0
        };
        let cy = if (y as u32) < self.height as u32 {
            y
        } else if y >= self.height {
            self.height - 1
        } else {
            0
        };
        (cy * self.width + cx) as usize
    }

    pub fn cell_flags(&self, x: i32, y: i32) -> u16 {
        self.flags[self.cell_index(x, y)]
    }

    pub fn terrain_type(&self, x: i32, y: i32) -> usize {
        let (fx, fy) = (x & 0x1ff, y & 0x1ff);
        let (cx, cy) = (x >> 9, y >> 9);
        let low = if (cx ^ cy) & 1 != 0 {
            fy >= 0x200 - fx
        } else {
            fx >= fy
        };
        let w = self.cell_flags(cx, cy);
        usize::from(if low { w & 0xf } else { (w >> 4) & 0xf })
    }

    pub fn neighbour_mask(&self, mask: u16, x: i32, y: i32) -> u8 {
        let (cx, cy) = (x >> 9, y >> 9);
        let clamp = |c: i32, n: i32| {
            if c <= 0 {
                1
            } else if c < n - 1 {
                c
            } else {
                n - 2
            }
        };
        let (bx, by) = (clamp(cx, self.width), clamp(cy, self.height));
        let table = if (cx ^ cy) & 1 == 0 {
            &NEIGHBOURS_EVEN
        } else {
            &NEIGHBOURS_ODD
        };
        let mut out = 0u8;
        for &(entry, bit) in &NEIGHBOUR_BITS {
            let (dx, dy, nibble) = table[entry];
            let w = self.flags[((by + dy) * self.width + bx + dx) as usize];
            let t = if nibble & 1 != 0 {
                w & 0xf
            } else {
                (w >> 4) & 0xf
            };
            if mask >> t & 1 != 0 {
                out |= bit;
            }
        }
        out
    }

    pub fn obstruction_box(&self, x: i32, y: i32, radius: i32) -> Option<[i32; 4]> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        let i = (y * self.width + x) as usize;
        if self.models[i] == -1 {
            return None;
        }
        let (hi, lo) = cell_terrain_types(self.flags[i]);
        if !(14..=15).contains(&hi) && !(14..=15).contains(&lo) {
            return None;
        }
        let far = |c: i32, n: i32| if c < n - 1 { c + 1 } else { n - 1 };
        Some([
            (x << 9) - radius,
            (y << 9) - radius,
            (far(x, self.width) << 9) + radius,
            (far(y, self.height) << 9) + radius,
        ])
    }

    fn scenery_box(&self, x: i32, y: i32, radius: i16) -> (i16, i16, i16, i16) {
        let r = i32::from(radius);
        let x0 = ((x - r) >> 9) as i16;
        let x1 = ((x + r) >> 9) as i16;
        let y0 = ((y - r) >> 9) as i16;
        let y1 = ((y + r) >> 9) as i16;
        let (w, h) = (self.width as i16, self.height as i16);
        let lo = |c: i16, n: i16| {
            if c < 0 {
                0
            } else if n > c {
                c
            } else {
                n - 1
            }
        };
        let hi = |c: i16, n: i16| {
            if c < 0 {
                0
            } else if n <= c {
                n - 1
            } else {
                c
            }
        };
        (lo(x0, w), hi(x1, w), lo(y0, h), hi(y1, h))
    }

    pub fn unmark_scenery(&mut self, x: i32, y: i32, radius: i16) -> ((i32, i32), (i32, i32)) {
        let (x0, x1, y0, y1) = self.scenery_box(x, y, radius);
        let (width, height) = (self.width, self.height);
        let before = |c: i16, n: i32| {
            let c = i32::from(c);
            if c <= 2 {
                0
            } else if c - 2 <= n {
                c - 3
            } else {
                n - 1
            }
        };
        let after = |c: i16, n: i32| {
            let c = i32::from(c);
            if c + 2 < n - 1 { c + 3 } else { n - 1 }
        };
        let scan = (
            (before(x0, width), after(x1, width)),
            (before(y0, height), after(y0, height)),
        );
        for cy in i32::from(y0)..=i32::from(y1) {
            for cx in i32::from(x0)..=i32::from(x1) {
                let i = self.cell_index(cx, cy);
                if self.flags[i] & CELL_KILLED == 0 {
                    self.flags[i] = (self.flags[i] & 0xff00) | 0x66;
                }
            }
        }
        scan
    }

    pub fn mark_scenery(&mut self, x: i32, y: i32, radius: i16) {
        let r = i32::from(radius);
        let (x0, x1, y0, y1) = self.scenery_box(x, y, radius);
        let mark = |cells: &mut Self, cx: i32, cy: i32| {
            let i = cells.cell_index(cx, cy);
            if cells.flags[i] & CELL_KILLED == 0 {
                cells.flags[i] = (cells.flags[i] & 0xff00) | 0xee;
            }
        };
        for cy in i32::from(y0)..=i32::from(y1) {
            for cx in i32::from(x0)..=i32::from(x1) {
                let inside = radius <= 0xff || {
                    let corners = [
                        (cx << 9, cy << 9),
                        ((cx + 1) << 9, cy << 9),
                        ((cx + 1) << 9, (cy + 1) << 9),
                        (cx << 9, (cy + 1) << 9),
                    ];
                    corners
                        .iter()
                        .any(|&(px, py)| world_distance2d_less_than((x, y), (px, py), r))
                };
                if inside {
                    mark(self, cx, cy);
                }
            }
        }
        if radius <= 0xff {
            mark(self, x >> 9, y >> 9);
        }
    }

    pub fn modify_cells(
        &mut self,
        points: &[(i32, i32)],
        kill: Option<bool>,
        terrain: Option<i32>,
    ) -> Vec<(i32, i32)> {
        let (mut and, low_mask, mut or): (u32, u32, u32) = match kill {
            None => (0xffff_ffff, 0xffff_ff00, 0),
            Some(true) => (0xffff_bfff, 0xffff_bf00, 0x4000),
            Some(false) => (0xffff_bfff, 0xffff_bf00, 0),
        };
        if let Some(t) = terrain {
            and = low_mask;
            or |= ((t as u32) << 4) | t as u32;
        }
        let mut out = Vec::new();
        for &(px, py) in points {
            let (cx, cy) = (px >> 9, py >> 9);
            if cx < 0 || cx >= self.width || cy < 0 || cy >= self.height {
                continue;
            }
            let i = self.cell_index(cx, cy);
            self.flags[i] = ((u32::from(self.flags[i]) & and) | or) as u16;
            out.push((cx, cy));
        }
        out
    }
}

impl MeshCells {
    pub fn mark_connector(&mut self, a: (i32, i32), b: (i32, i32), kill: bool) -> Vec<(i32, i32)> {
        let cells = connector_cells(a, b);
        for &(x, y) in &cells {
            self.set_connector_cell(x, y, kill);
        }
        if kill { Vec::new() } else { cells }
    }

    pub fn set_connector_cell(&mut self, x: i32, y: i32, kill: bool) {
        let i = self.cell_index(x, y);
        let v = if kill { 0xee } else { 0x66 };
        self.flags[i] = (self.flags[i] & 0xff00) | v;
    }
}

pub fn connector_cells(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    {
        let cells = &mut out;
        let set = |cells: &mut Vec<(i32, i32)>, x: i32, y: i32| cells.push((x, y));
        let w = |v: i32| v as i16 as i32;
        let (ax, ay) = (w(a.0 >> 9), w(a.1 >> 9));
        let (bx, by) = (w(b.0 >> 9), w(b.1 >> 9));
        set(cells, ax, ay);
        if ay == by {
            if ax != bx {
                let step = if ax < bx { 1 } else { -1 };
                let mut x = ax;
                while x != bx {
                    x += step;
                    set(cells, x, ay);
                }
            }
            return out;
        }
        if ax == bx {
            let step = if ay < by { 1 } else { -1 };
            let mut y = ay;
            while y != by {
                y += step;
                set(cells, ax, y);
            }
            return out;
        }
        let dx = b.0.wrapping_sub(a.0);
        let dy = b.1.wrapping_sub(a.1);
        if dx.abs() > dy.abs() {
            let slope = (((dy as i16 as i32) << 8).wrapping_div(dx)) as i16 as i32;
            let c = a.1.wrapping_sub(a.0.wrapping_mul(slope) >> 8);
            let dir = if ax < bx { 1 } else { -1 };
            let (mut x, mut y) = (ax, ay);
            loop {
                let nx = w(x + dir);
                let ny = w((((nx << 9).wrapping_mul(slope)) >> 8).wrapping_add(c) >> 9);
                if ny == y {
                    set(cells, nx, y);
                } else if dir == 1 {
                    set(cells, x, ny);
                    set(cells, nx, ny);
                } else {
                    set(cells, nx, y);
                    set(cells, nx, ny);
                }
                if nx == bx {
                    break;
                }
                x = nx;
                y = ny;
            }
        } else {
            let slope = (((dx as i16 as i32) << 8).wrapping_div(dy)) as i16 as i32;
            let c = a.0.wrapping_sub(a.1.wrapping_mul(slope) >> 8);
            let dir = if ay < by { 1 } else { -1 };
            let (mut x, mut y) = (ax, ay);
            loop {
                let ny = w(y + dir);
                let nx = w((((ny << 9).wrapping_mul(slope)) >> 8).wrapping_add(c) >> 9);
                if nx == x {
                    set(cells, x, ny);
                } else if dir == 1 {
                    set(cells, nx, y);
                    set(cells, nx, ny);
                } else {
                    set(cells, x, ny);
                    set(cells, nx, ny);
                }
                if ny == by {
                    break;
                }
                x = nx;
                y = ny;
            }
        }
    }
    out
}

pub fn impassability_update_range(cells: &MeshCells, x: i32, y: i32) -> (i32, i32, i32, i32) {
    let lo = |c: i32, n: i32| {
        if c <= 0 {
            0
        } else if c > n {
            n - 1
        } else {
            c - 1
        }
    };
    let hi = |c: i32, n: i32| {
        if c < -1 {
            0
        } else if c >= n - 1 {
            n - 1
        } else {
            c + 1
        }
    };
    let (x0, x1) = (lo(x, cells.width), hi(x, cells.width));
    let (y0, y1) = (lo(y, cells.height), hi(y, cells.height));
    (x0, y0, x1, y1)
}

pub fn world_distance2d_less_than(a: (i32, i32), b: (i32, i32), d: i32) -> bool {
    if (d.wrapping_sub(1) as u32) > 0x7ffe {
        return d > 0x7fff;
    }
    let dx = a.0.wrapping_sub(b.0);
    let dy = a.1.wrapping_sub(b.1);
    if (dx.wrapping_add(0x7fff) as u32) > 0xfffd || (dy.wrapping_add(0x7fff) as u32) > 0xfffd {
        return false;
    }
    let sq = (dx.wrapping_mul(dx) as u32).wrapping_add(dy.wrapping_mul(dy) as u32);
    sq <= (d.wrapping_mul(d - 1)) as u32
}

pub fn clip_line_to_box(
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    from: (i32, i32),
    to: (i32, i32),
) -> Option<((i32, i32), i32)> {
    let (ox, oy) = from;
    let dy_lo = (to.1 as u16 as i32).wrapping_sub(oy);
    let dx_full = to.0.wrapping_sub(ox);
    if (dy_lo as u16) | (dx_full as u16) == 0 {
        return None;
    }
    let dx = dx_full as i16 as i32;
    let dy = dy_lo as i16 as i32;
    let scale = |v: i32| (if v < 0 { v + 0xff } else { v }) >> 8;
    let div = |n: i32, d: i32| (n.wrapping_shl(16)).wrapping_div(d);
    let mut tmin = 0i32;
    let mut tmax = 0x10000i32;

    let num = x0.wrapping_sub(ox);
    if dx == 0 {
        if num > 0 {
            return None;
        }
        let q = ox.wrapping_sub(x1);
        if q > 0 {
            return None;
        }
    } else {
        let (n, d) = if num.abs() > 0x8000 {
            (scale(num), scale(dx))
        } else {
            (num, dx)
        };
        if d == 0 {
            if dx > 0 {
                return None;
            }
            tmax = 0x10000;
            tmin = 0;
        } else {
            let t = div(n, d);
            if dx > 0 {
                if t > 0x10000 {
                    return None;
                }
                tmin = t.max(0);
                tmax = 0x10000;
            } else {
                if t < 0 {
                    return None;
                }
                tmax = t.min(0x10000);
                tmin = 0;
            }
        }
        let q = ox.wrapping_sub(x1);
        let p = -dx;
        let (n, d) = if q.abs() > 0x8000 {
            (scale(q), scale(p))
        } else {
            (q, p)
        };
        if d == 0 {
            if p > 0 {
                return None;
            }
        } else {
            let t = div(n, d);
            if p > 0 {
                if tmax < t {
                    return None;
                }
                tmin = tmin.max(t);
            } else {
                if t < tmin {
                    return None;
                }
                tmax = tmax.min(t);
            }
        }
    }

    let num = y0.wrapping_sub(oy);
    if dy == 0 {
        if num > 0 {
            return None;
        }
        let q = oy.wrapping_sub(y1);
        if q > 0 {
            return None;
        }
    } else {
        let d = if num.abs() > 0x8000 { scale(dy) } else { dy };
        let n = if num.abs() > 0x8000 { scale(num) } else { num };
        if d == 0 {
            if dy > 0 {
                return None;
            }
        } else {
            let t = div(n, d);
            if dy > 0 {
                if tmax < t {
                    return None;
                }
                tmin = tmin.max(t);
            } else {
                if tmin > t {
                    return None;
                }
                tmax = tmax.min(t);
            }
        }
        let q = oy.wrapping_sub(y1);
        let p = -dy;
        let (n, d) = if q.abs() > 0x8000 {
            (scale(q), scale(p))
        } else {
            (q, p)
        };
        if d == 0 {
            if p > 0 {
                return None;
            }
        } else {
            let t = div(n, d);
            if p > 0 {
                if tmax < t {
                    return None;
                }
                tmin = tmin.max(t);
            } else if t < tmin {
                return None;
            }
        }
    }

    let ex = ox.wrapping_add(to.0.wrapping_sub(ox).wrapping_mul(tmin) >> 16);
    let ey = oy.wrapping_add(to.1.wrapping_sub(oy).wrapping_mul(tmin) >> 16);
    let near = |a: i32, b: i32| (a.wrapping_sub(b).wrapping_add(1) as u32) <= 2;
    let mut side = if near(ex, x0) || near(ex, x1) { 0 } else { -1 };
    if near(ey, y0) || near(ey, y1) {
        side = 1;
    }
    Some(((ex, ey), side))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MonsterPassability {
    pub impassable: u16,
    pub neighbours: u8,
    pub flies: bool,
    pub slide_without_nudge: bool,
    pub radius: Option<i16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    pub x: i32,
    pub y: i32,
    pub flags: u32,
    pub new_leg: bool,
    pub moved: bool,
    pub overlap: bool,
    pub pushed: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ObjectHits {
    pub flags: u32,
    pub stop: bool,
    pub push: Option<(i32, i32)>,
    pub overlap: bool,
}

pub type ObjectLoop<'a> = &'a mut dyn FnMut((i32, i32)) -> Result<ObjectHits, Unmodelled>;

pub fn move_monster(
    cells: &MeshCells,
    m: &mut MonsterPassability,
    from: (i32, i32),
    v: (i16, i16),
) -> Result<StepOutcome, Unmodelled> {
    move_monster_with(cells, m, from, v, None)
}

pub fn move_monster_with(
    cells: &MeshCells,
    m: &mut MonsterPassability,
    from: (i32, i32),
    v: (i16, i16),
    objects: Option<ObjectLoop<'_>>,
) -> Result<StepOutcome, Unmodelled> {
    let (ox, oy) = from;
    let mut res = 0u32;
    if v == (0, 0) {
        return Ok(StepOutcome {
            x: ox,
            y: oy,
            flags: 0,
            new_leg: false,
            moved: false,
            overlap: false,
            pushed: None,
        });
    }
    let mut px = ox + i32::from(v.0);
    let mut py = oy + i32::from(v.1);
    let margin = crate::engine_move::EDGE_MARGIN;
    if px < margin {
        px = margin;
        res = 0x20;
    }
    if (cells.width << 9) - margin <= px {
        px = (cells.width << 9) - margin - 1;
        res = 0x20;
    }
    if py < margin {
        py = margin;
        res = 0x20;
    }
    if (cells.height << 9) - margin <= py {
        py = (cells.height << 9) - margin - 1;
        res = 0x20;
    }

    let mut overlap = false;
    let mut pushed = None;
    if let Some(objects) = objects {
        let hits = objects((px, py))?;
        res |= hits.flags;
        overlap = hits.overlap;
        if hits.stop {
            return Ok(StepOutcome {
                x: ox,
                y: oy,
                flags: res,
                new_leg: false,
                moved: false,
                overlap,
                pushed: None,
            });
        }
        if let Some(p) = hits.push {
            (px, py) = p;
            pushed = Some(p);
        }
    }
    let refused = |flags: u32, new_leg: bool| StepOutcome {
        x: ox,
        y: oy,
        flags,
        new_leg,
        moved: false,
        overlap,
        pushed,
    };

    if !m.flies {
        let (bx, by) = (px >> 9, py >> 9);
        let neighbours = [(bx, by - 1), (bx, by + 1), (bx - 1, by), (bx + 1, by)];
        let obstructing = neighbours
            .iter()
            .any(|&(x, y)| cells.obstruction_box(x, y, 0).is_some());
        let boxes: Vec<[i32; 4]> = if obstructing {
            let r = m.radius.ok_or_else(|| {
                Unmodelled::new(
                    draw_site::MOVE_RADIUS_UNKNOWN,
                    "object radius (+0x20) unknown next to an obstructing model cell".to_string(),
                )
            })?;
            neighbours
                .iter()
                .filter_map(|&(x, y)| cells.obstruction_box(x, y, i32::from(r)))
                .collect()
        } else {
            Vec::new()
        };
        for b in boxes {
            let [x0, y0, x1, y1] = b;
            if px > x1 || px < x0 || py < y0 || py > y1 {
                continue;
            }
            let d = crate::engine_move::distance2d(px - ox, py - oy);
            match clip_line_to_box(x0, x1, y0, y1, (ox, oy), (px, py)) {
                Some((_, 0)) => {
                    px = ox;
                    py = if oy < py { oy + d } else { oy - d };
                }
                Some((_, 1)) => {
                    py = oy;
                    px = if ox < px { ox + d } else { ox - d };
                }
                _ => {}
            }
            res |= 0x10;
        }
    }

    let mask = m.impassable;
    if mask != 0 {
        let (bx, by) = (px >> 9, py >> 9);
        let (ocx, ocy) = (ox >> 9, oy >> 9);
        let odd = (bx ^ by) & 1 != 0;
        let nb = m.neighbours;
        let blocked = |t: usize| mask >> t & 1 != 0;
        enum Next {
            SameCell,
            Neighbours,
        }
        let next = if nb == 0 {
            if ocx != bx || ocy != by {
                Next::Neighbours
            } else {
                Next::SameCell
            }
        } else {
            let xblock = (ocx > bx && nb & 0x20 != 0) || (ocx < bx && nb & 0x80 != 0);
            let yblock = (ocy > by && nb & 0x10 != 0) || (ocy < by && nb & 0x40 != 0);
            match (xblock, yblock) {
                (true, true) => {
                    if blocked(cells.terrain_type(px, py)) {
                        px = ox;
                        py = oy;
                        res |= 0x10;
                        Next::SameCell
                    } else if ocx != bx || ocy != by {
                        Next::Neighbours
                    } else {
                        Next::SameCell
                    }
                }
                (false, true) => {
                    py = oy;
                    res |= 0x10;
                    if ocx == bx {
                        Next::SameCell
                    } else {
                        Next::Neighbours
                    }
                }
                (true, false) => {
                    px = ox;
                    res |= 0x10;
                    if ocy != by {
                        Next::Neighbours
                    } else {
                        Next::SameCell
                    }
                }
                (false, false) => {
                    if ocx == bx && ocy == by {
                        Next::SameCell
                    } else {
                        Next::Neighbours
                    }
                }
            }
        };
        match next {
            Next::SameCell => {
                let fx = px & 0x1ff;
                let fy0 = py & 0x1ff;
                let fy = if odd { 0x1ff - fy0 } else { fy0 };
                let own_blocked = if fy > fx { nb & 1 != 0 } else { nb & 2 != 0 };
                if own_blocked {
                    let (cellx, celly) = (px & !0x1ff, py & !0x1ff);
                    if m.slide_without_nudge {
                        let e = if odd { 0x1ff - fy } else { fy };
                        px = cellx + e;
                        py = celly + e;
                    } else {
                        px = cellx + fy;
                        py = if odd { celly + 0x1ff - fy } else { celly + fy };
                        let fx2 = px & 0x1ff;
                        let high = nb & 1 != 0;
                        match (odd, high) {
                            (false, false) => {
                                if fx2 > 0x100 {
                                    px -= 1;
                                } else {
                                    py += 1;
                                }
                            }
                            (false, true) => {
                                if fx2 > 0xff {
                                    py -= 1;
                                } else {
                                    px += 1;
                                }
                            }
                            (true, false) => {
                                if fx2 > 0x100 {
                                    px -= 1;
                                } else {
                                    py -= 1;
                                }
                            }
                            (true, true) => {
                                if fx2 <= 0xff {
                                    px += 1;
                                } else {
                                    py += 1;
                                }
                            }
                        }
                    }
                    res |= 0x10;
                }
            }
            Next::Neighbours => {
                let nb2 = cells.neighbour_mask(mask, px, py);
                let refused_here = nb2 & 3 == 3
                    || nb2 & 0xf0 == 0xf0
                    || (!odd && (nb2 & 0x91 == 0x91 || nb2 & 0x62 == 0x62));
                if refused_here {
                    return Ok(refused(res | 0x10, false));
                }
            }
        }
        {
            let t1 = cells.terrain_type(px, py);
            let t0 = cells.terrain_type(ox, oy);
            if blocked(t1) && !blocked(t0) {
                res |= 0x10;
                let (cx, cy) = (px, py);
                if !blocked(cells.terrain_type(ox, cy)) {
                    px = ox;
                    py = cy;
                } else if !blocked(cells.terrain_type(cx, oy)) {
                    px = cx;
                    py = oy;
                } else {
                    return Ok(refused(res, true));
                }
            }
        }
    }

    if res & 0x10 != 0 && res & 4 != 0 {
        return Ok(refused(res, false));
    }

    if (px >> 9, py >> 9) != (ox >> 9, oy >> 9) {
        res |= 0x80;
        m.neighbours = cells.neighbour_mask(mask, px, py);
    }
    Ok(StepOutcome {
        x: px,
        y: py,
        flags: res,
        new_leg: false,
        moved: true,
        overlap,
        pushed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: i32, h: i32, f: u16) -> MeshCells {
        MeshCells {
            width: w,
            height: h,
            flags: vec![f; (w * h) as usize],
            models: vec![-1; (w * h) as usize],
        }
    }

    #[test]
    fn terrain_type_follows_the_cell_parity() {
        let mut c = grid(4, 4, 0x66);
        c.flags[0] = 0x21;
        assert_eq!(c.terrain_type(300, 100), 1);
        assert_eq!(c.terrain_type(100, 300), 2);
        c.flags[1] = 0x43;
        assert_eq!(c.terrain_type(512 + 400, 400), 3);
        assert_eq!(c.terrain_type(512 + 10, 10), 4);
        assert_eq!(c.terrain_type(-5, -5), c.terrain_type(0, 0));
    }

    #[test]
    fn neighbour_mask_reads_the_edge_triangles() {
        let mut c = grid(5, 5, 0x66);
        let at = |x: i32, y: i32| (y * 5 + x) as usize;
        c.flags[at(2, 1)] = 0xe6;
        c.flags[at(3, 2)] = 0x6e;
        let mask = 1 << 14;
        assert_eq!(c.neighbour_mask(mask, 2 * 512 + 10, 2 * 512 + 10), 0x10);
        c.flags[at(3, 2)] = 0xe6;
        assert_eq!(c.neighbour_mask(mask, 2 * 512 + 10, 2 * 512 + 10), 0x90);
    }

    #[test]
    fn mesh_cell_modifier_kills_and_sets_terrain() {
        let mut c = grid(4, 4, 0x2066);
        let changed = c.modify_cells(&[(600, 600), (-10, 0)], Some(true), Some(3));
        assert_eq!(changed, vec![(1, 1)]);
        assert_eq!(c.cell_flags(1, 1), 0x6033);
        c.modify_cells(&[(600, 600)], Some(false), None);
        assert_eq!(c.cell_flags(1, 1), 0x2033);
    }

    #[test]
    fn scenery_marks_spare_killed_cells() {
        let mut c = grid(8, 8, 0x66);
        c.flags[(3 * 8 + 3) as usize] = 0x4066;
        c.mark_scenery(3 * 512 + 256, 3 * 512 + 256, 0x1ff);
        assert_eq!(c.cell_flags(3, 3), 0x4066);
        assert_eq!(c.cell_flags(2, 3) & 0xff, 0xee);
        assert_eq!(c.cell_flags(4, 4) & 0xff, 0xee);
        assert_eq!(c.cell_flags(5, 3) & 0xff, 0x66);
    }

    #[test]
    fn connector_lines_walk_the_long_axis() {
        let mut c = grid(16, 16, 0x66);
        c.mark_connector((512 + 256, 512 + 256), (6 * 512 + 256, 3 * 512 + 256), true);
        let marked: Vec<(i32, i32)> = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .filter(|&(x, y)| c.cell_flags(x, y) & 0xff == 0xee)
            .collect();
        assert!(marked.contains(&(1, 1)) && marked.contains(&(6, 3)));
        assert!(
            marked
                .iter()
                .all(|&(x, y)| (1..=6).contains(&x) && (1..=3).contains(&y))
        );
    }

    #[test]
    fn clip_reports_the_edge_entered() {
        let ((x, _), side) = clip_line_to_box(100, 200, 0, 100, (50, 50), (150, 50)).unwrap();
        assert_eq!((x, side), (100, 0));
        let ((_, y), side) = clip_line_to_box(100, 200, 0, 100, (150, -50), (150, 50)).unwrap();
        assert_eq!((y, side), (0, 1));
        assert!(clip_line_to_box(100, 200, 0, 100, (0, 300), (50, 300)).is_none());
    }

    #[test]
    fn a_step_into_closed_terrain_slides_or_stops() {
        let mut c = grid(8, 8, 0x66);
        for y in 0..8 {
            c.flags[(y * 8 + 2) as usize] = 0xee;
        }
        let mut m = MonsterPassability {
            impassable: 1 << 14,
            neighbours: 0,
            flies: false,
            slide_without_nudge: false,
            radius: Some(0),
        };
        let from = (2 * 512 - 10, 4 * 512 + 100);
        m.neighbours = c.neighbour_mask(m.impassable, from.0, from.1);
        let out = move_monster(&c, &mut m, from, (20, 30)).unwrap();
        assert_eq!((out.x, out.y), (from.0, from.1 + 30));
        assert_eq!(out.flags & 0x10, 0x10);
    }
}
