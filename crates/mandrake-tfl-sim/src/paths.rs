use mandrake_common::game_options::tfl;

use crate::{Stop, TflSim};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub salt: i16,
    pub flags: u16,
    pub index: i16,
    pub goal_cell: (i16, i16),
    pub points: Vec<(i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct PathPool {
    pub paths: Vec<Option<Path>>,
    pub first_free: usize,
    pub salt: u16,
}

pub const PATH_CAPACITY: usize = 0xc8;

impl Default for PathPool {
    fn default() -> Self {
        Self {
            paths: vec![None; PATH_CAPACITY],
            first_free: 0,
            salt: 0x8000,
        }
    }
}

impl PathPool {
    pub fn delete(&mut self, index: u8) {
        let i = usize::from(index);
        if let Some(p) = self.paths.get_mut(i) {
            *p = None;
        }
        self.first_free = self.first_free.min(i);
    }

    fn allocate(&mut self) -> u8 {
        for i in self.first_free..self.paths.len() {
            if self.paths[i].is_none() {
                self.paths[i] = Some(Path {
                    salt: self.salt as i16,
                    flags: 0,
                    index: 0,
                    goal_cell: (0, 0),
                    points: Vec::new(),
                });
                self.salt = self.salt.wrapping_add(1);
                if self.salt == 0 {
                    self.salt = 0x8000;
                }
                self.first_free = i + 1;
                return i as u8;
            }
        }
        0xff
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PathRequest {
    pub flags: u16,
    pub mask: u16,
    pub start: (i32, i32),
    pub goal: (i32, i32),
    pub start_cell: (i32, i32),
    pub goal_cell: (i32, i32),
}

const DX: [i32; 8] = [1, 1, -1, -1, 0, 1, 0, -1];
const DY: [i32; 8] = [1, -1, -1, 1, 1, 0, -1, 0];
const ENTRY: [u32; 8] = [2, 3, 0, 1, 6, 7, 4, 5];
const OPEN_DIRECTIONS: [u16; 8] = [0xf0, 0xc0, 0x30, 0, 0xf0, 0x90, 0x60, 0];

const NONE: u16 = u16::MAX;

#[derive(Debug, Clone, Copy)]
struct Node {
    next: u16,
    parent: u16,
    direction: i16,
    open: u16,
    x: i16,
    y: i16,
    cost: i16,
    total: i16,
}

impl Node {
    const EMPTY: Self = Self {
        next: NONE,
        parent: NONE,
        direction: -1,
        open: 0,
        x: 0,
        y: 0,
        cost: 0,
        total: 0,
    };
}

fn estimate(dx: i32, dy: i32) -> i32 {
    let a = (dx << 4).abs();
    let b = (dy << 4).abs();
    if a > b { (b >> 1) + a } else { (a >> 1) + b }
}

impl TflSim {
    fn cell_terrain(&self, cx: i32, cy: i32) -> u8 {
        let (w, h) = (
            self.data.mesh.cell_width as i32,
            self.data.mesh.cell_height as i32,
        );
        let k = if cx >= 0 && cx < w && cy >= 0 && cy < h {
            (w * cy + cx) as usize
        } else {
            0
        };
        self.cells[k].byte_5
    }

    pub fn terrain_at(&self, p: (i32, i32)) -> u8 {
        let (x, y) = p;
        let low = if ((y ^ x) >> 10) & 1 == 0 {
            0x400 - (x & 0x3ff) <= (y & 0x3ff)
        } else {
            (y & 0x3ff) <= (x & 0x3ff)
        };
        let b = self.cell_terrain(x >> 10, y >> 10);
        (if low { b } else { b >> 4 }) & 0xf
    }

    fn open_directions(&self, x: i32, y: i32, mask: u16) -> u16 {
        let f = u32::from(self.cell_terrain(x, y));
        let m = u32::from(mask);
        let hi = if m & (1 << ((f >> 4) & 0xf)) != 0 {
            2
        } else {
            0
        };
        let lo = if m & (1 << (f & 0xf)) != 0 { 1 } else { 0 };
        OPEN_DIRECTIONS[(hi + lo + ((x ^ y) & 1) as u32 * 4) as usize]
    }

    pub fn new_path(&mut self, _monster: usize, req: &mut PathRequest) -> Result<u8, Stop> {
        let closed = req.mask;
        let goal = req.goal;
        for k in (0..=9).rev() {
            if u32::from(closed) & (1 << self.terrain_at(req.goal)) == 0 {
                break;
            }
            req.goal = (
                (goal.0 - req.start.0) * k / 10 + req.start.0,
                (goal.1 - req.start.1) * k / 10 + req.start.1,
            );
        }
        req.start_cell = (req.start.0 >> 10, req.start.1 >> 10);
        req.goal_cell = (req.goal.0 >> 10, req.goal.1 >> 10);
        let index = self.paths.allocate();
        if index == 0xff {
            return Ok(index);
        }
        let (flags, points) = match self.search(req) {
            None => (0, vec![req.goal]),
            Some(mut directions) => {
                if directions.len() > 0x2c {
                    directions.drain(..directions.len() - 0x2c);
                    req.flags |= 4;
                }
                let mut steps = Steps {
                    flags: req.flags,
                    count: directions.len() as i16,
                    cell: req.start_cell,
                    goal: req.goal,
                    directions,
                };
                let mut points = Vec::with_capacity(0xe);
                let mut current = req.start;
                loop {
                    let (r, out) = self.next_point_in_path(req, &mut steps, current);
                    if points.len() >= 0xe {
                        break;
                    }
                    points.push(out);
                    current = out;
                    if r == -1 {
                        break;
                    }
                }
                if steps.flags & 4 != 0
                    && self.game_options & tfl::FIXES_2000 != 0
                    && (crate::groups::distance(req.start, req.goal) as i32) < 0x1000
                {
                    (0, vec![req.start])
                } else {
                    (steps.flags, points)
                }
            }
        };
        let p = self.paths.paths[index as usize]
            .as_mut()
            .expect("allocated");
        p.flags = flags;
        p.index = 0;
        p.points = points;
        p.goal_cell = (req.goal_cell.0 as i16, req.goal_cell.1 as i16);
        Ok(index)
    }

    fn search(&self, req: &mut PathRequest) -> Option<Vec<u8>> {
        let (w, h) = (
            self.data.mesh.cell_width as i32,
            self.data.mesh.cell_height as i32,
        );
        let mut flags = req.flags;
        let window = |c: i32, size: i32| {
            let v = c - 0x20;
            if v < 0 {
                0
            } else if v > size - 0x40 {
                size - 0x40
            } else {
                v
            }
        };
        let wx = window(req.start_cell.0, w);
        let wy = window(req.start_cell.1, h);
        let mut visited = [0u64; 64];
        let mut nodes = [Node::EMPTY; 256];
        nodes[0] = Node {
            next: NONE,
            parent: NONE,
            direction: -1,
            open: self.open_directions(req.start_cell.0, req.start_cell.1, req.mask),
            x: (req.start_cell.0 - wx) as i16,
            y: (req.start_cell.1 - wy) as i16,
            cost: 0,
            total: estimate(
                req.start_cell.0 - req.goal_cell.0,
                req.start_cell.1 - req.goal_cell.1,
            ) as i16,
        };
        let mut node_len = 1usize;
        let mut result: u16 = NONE;
        if req.start_cell == req.goal_cell {
            result = 0;
        } else {
            let mut current = 0usize;
            let mut head: u16;
            'search: loop {
                head = nodes[current].next;
                let mut limit = i32::MAX;
                for dir in 4..8usize {
                    let node = nodes[current];
                    if u32::from(node.open) & (1 << dir) == 0 {
                        continue;
                    }
                    let nx = DX[dir] + i32::from(node.x);
                    let ny = DY[dir] + i32::from(node.y);
                    if !(0..0x40).contains(&nx) || !(0..0x40).contains(&ny) {
                        result = current as u16;
                        flags |= 4;
                        break 'search;
                    }
                    if visited[ny as usize] & (1 << nx) != 0 {
                        continue;
                    }
                    let (ax, ay) = (wx + nx, wy + ny);
                    let open = self.open_directions(ax, ay, req.mask);
                    flags |= 8;
                    visited[ny as usize] |= 1 << nx;
                    let entry = ENTRY[dir];
                    if u32::from(open) & (1 << entry) == 0 {
                        continue;
                    }
                    let cost = i32::from(node.cost)
                        + if i32::from(node.direction) != dir as i32 {
                            6
                        } else {
                            0x10
                        };
                    let total = estimate(ax - req.goal_cell.0, ay - req.goal_cell.1) + cost;
                    if total >= limit || node_len >= 0x100 {
                        continue;
                    }
                    let k = node_len;
                    node_len += 1;
                    nodes[k] = Node {
                        next: NONE,
                        parent: current as u16,
                        direction: dir as i16,
                        open: !(1u16 << entry) & open,
                        x: nx as i16,
                        y: ny as i16,
                        cost: cost as i16,
                        total: total as i16,
                    };
                    if (ax, ay) == req.goal_cell {
                        result = k as u16;
                        break 'search;
                    }
                    let mut link = NONE;
                    let mut ahead = 0;
                    let mut dropped = false;
                    loop {
                        let at = if link == NONE {
                            head
                        } else {
                            nodes[link as usize].next
                        };
                        if at == NONE {
                            break;
                        }
                        if i32::from(nodes[at as usize].total) >= total {
                            break;
                        }
                        ahead += 1;
                        if ahead > 9 {
                            limit = i32::from(nodes[at as usize].total);
                            dropped = true;
                            break;
                        }
                        link = at;
                    }
                    if !dropped {
                        if link == NONE {
                            nodes[k].next = head;
                            head = k as u16;
                        } else {
                            nodes[k].next = nodes[link as usize].next;
                            nodes[link as usize].next = k as u16;
                        }
                    }
                }
                if head == NONE {
                    break;
                }
                current = head as usize;
            }
            if result == NONE {
                let mut best: Option<(usize, i32)> = None;
                for (k, n) in nodes[..node_len].iter().enumerate() {
                    let e = i32::from(n.total) - i32::from(n.cost);
                    if best.is_none_or(|(_, b)| e < b) {
                        best = Some((k, e));
                    }
                }
                if let Some((k, _)) = best
                    && k != 0
                {
                    flags |= 4;
                    result = k as u16;
                }
            }
        }
        req.flags = flags;
        if result == NONE {
            return None;
        }
        let mut k = result as usize;
        let mut dirs = Vec::new();
        while nodes[k].parent != NONE {
            dirs.push(nodes[k].direction as u8);
            k = nodes[k].parent as usize;
        }
        Some(dirs)
    }

    fn next_point_in_path(
        &self,
        req: &PathRequest,
        s: &mut Steps,
        current: (i32, i32),
    ) -> (i16, (i32, i32)) {
        if s.flags & 0xb == 0 {
            s.count = -1;
            return (-1, s.goal);
        }
        let (mut cx, mut cy) = s.cell;
        let mut pts = [(0i32, 0i32); 10];
        let mut n = 0usize;
        let mut i = i32::from(s.count) - 1;
        loop {
            if i > 0 || (i == 0 && s.flags & 4 != 0) {
                let d = s.directions[i as usize] as usize;
                cx += DX[d];
                cy += DY[d];
                pts[n] = (cx * 0x400 + 0x200, cy * 0x400 + 0x200);
            } else {
                pts[n] = s.goal;
            }
            if i <= 0 {
                n += 1;
                break;
            }
            if n > 5 && i < 10 {
                break;
            }
            n += 1;
            i -= 1;
            if n >= 10 {
                break;
            }
        }
        let mut k = n as i32 - 1;
        while k > 0 {
            if self.valid_straight_line_path(req, current, pts[k as usize]) {
                break;
            }
            if k < 2 {
                k = 0;
                break;
            }
            k -= 2;
        }
        let out = pts[k.max(0) as usize];
        s.count += -1 - k as i16;
        if s.count < 0 {
            s.count = -1;
        }
        s.cell = (out.0 >> 10, out.1 >> 10);
        (s.count, out)
    }

    fn valid_straight_line_path(
        &self,
        req: &PathRequest,
        from: (i32, i32),
        to: (i32, i32),
    ) -> bool {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let len =
            crate::groups::isqrt((dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx))) as u32)
                as i32;
        if len == 0 {
            return false;
        }
        let px = (dy * -0x100) / len;
        let py = (dx * 0x100) / len;
        let mut c = [
            (from.0 + px, from.1 + py),
            (to.0 + px, to.1 + py),
            (from.0 - px, from.1 - py),
            (to.0 - px, to.1 - py),
        ];
        let x0 = c.iter().map(|p| p.0).min().unwrap() >> 10;
        let y0 = c.iter().map(|p| p.1).min().unwrap() >> 10;
        let w = (c.iter().map(|p| p.0).max().unwrap() >> 10) - x0 + 1;
        let h = (c.iter().map(|p| p.1).max().unwrap() >> 10) - y0 + 1;
        for p in &mut c {
            p.0 -= x0 * 0x400;
            p.1 -= y0 * 0x400;
        }
        let (mw, mh) = (
            self.data.mesh.cell_width as i32,
            self.data.mesh.cell_height as i32,
        );
        if !(w < 9 && h < 9 && x0 >= 0 && y0 >= 0 && w + x0 <= mw && h + y0 <= mh) {
            return false;
        }
        let mut rows = [0u8; 8];
        mark_edge(&mut rows, c[0], c[1], w, h);
        mark_edge(&mut rows, c[2], c[3], w, h);
        let m = u32::from(req.mask);
        for (r, row) in rows.iter().enumerate().take(h as usize) {
            for col in 0..w {
                if row & (1 << (col & 7)) != 0 {
                    let f = u32::from(self.cell_terrain(x0 + col, y0 + r as i32));
                    if m & (1 << ((f >> 4) & 0xf)) != 0 || m & (1 << (f & 0xf)) != 0 {
                        return false;
                    }
                }
            }
        }
        true
    }
}

fn mark_edge(rows: &mut [u8; 8], p0: (i32, i32), p1: (i32, i32), w: i32, h: i32) {
    let dx = p1.0 - p0.0;
    let dy = p1.1 - p0.1;
    if dy != 0 {
        for j in 1..h {
            let x = (((j * 0x400 - p0.1) * dx) / dy + p0.0) >> 10;
            let x = x.max(0).min(w - 1);
            let bit = 1u8 << (x & 7);
            rows[j as usize] |= bit;
            rows[j as usize - 1] |= bit;
        }
    }
    if dx != 0 {
        for j in 1..w {
            let y = (((j * 0x400 - p0.0) * dy) / dx + p0.1) >> 10;
            let y = y.max(0).min(h - 1);
            rows[y as usize] |= (1u8 << ((j - 1) & 7)) | (1u8 << (j & 7));
        }
    }
}

struct Steps {
    flags: u16,
    count: i16,
    cell: (i32, i32),
    goal: (i32, i32),
    directions: Vec<u8>,
}
