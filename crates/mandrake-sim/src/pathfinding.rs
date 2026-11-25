use mandrake_common::game_options;

use crate::draw_site;
use crate::engine_move::{arctangent, distance2d, trig_tables};
use crate::engine_rng::{Unmodelled, angular_difference};
use crate::passability::{MeshCells, clip_line_to_box, world_distance2d_less_than};
use crate::projectiles::isqrt;

pub const DIRECTIONS: [(i32, i32); 8] = [
    (1, 1),
    (1, -1),
    (-1, -1),
    (-1, 1),
    (0, 1),
    (1, 0),
    (0, -1),
    (-1, 0),
];

pub const PATH_CAPACITY: usize = 400;

const NODE_LIMIT: u32 = 0x3ff;

const CORNER_LIMIT: i16 = 0x16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathRequest {
    pub flags: u16,
    pub impassable: u16,
    pub object: i32,
    pub start: (i32, i32),
    pub goal: (i32, i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Path {
    pub flags: u16,
    pub point: (i32, i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewPath {
    Path(Path),
    Retry,
}

pub trait FireCells {
    fn any(&self) -> bool;
    fn on_fire(&self, x: i32, y: i32) -> bool;
    fn fire_modifier(&self) -> Result<u16, Unmodelled>;
}

pub struct NoFire;

impl FireCells for NoFire {
    fn any(&self) -> bool {
        false
    }
    fn on_fire(&self, _x: i32, _y: i32) -> bool {
        false
    }
    fn fire_modifier(&self) -> Result<u16, Unmodelled> {
        Ok(0)
    }
}

fn clamp_world(v: i32, size: i32) -> i32 {
    if v < 0 {
        0
    } else if v >= size << 9 {
        (size << 9) - 1
    } else {
        v
    }
}

pub fn new_path(
    cells: &MeshCells,
    fire: &dyn FireCells,
    request: &mut PathRequest,
    radius: Option<i16>,
) -> Result<NewPath, Unmodelled> {
    request.start = (
        clamp_world(request.start.0, cells.width),
        clamp_world(request.start.1, cells.height),
    );
    request.goal = (
        clamp_world(request.goal.0, cells.width),
        clamp_world(request.goal.1, cells.height),
    );
    let start_cell = (request.start.0 >> 9, request.start.1 >> 9);
    let goal_cell = (request.goal.0 >> 9, request.goal.1 >> 9);
    if start_cell == goal_cell {
        return Ok(NewPath::Path(Path {
            flags: 0,
            point: request.goal,
        }));
    }
    PATH_SCRATCH.with(|cell| {
        let mut scratch = cell.borrow_mut();
        let found = search_scratch(cells, fire, request, start_cell, goal_cell, &mut scratch)?;
        let Some(end) = found else {
            request.goal = request.start;
            return Ok(NewPath::Retry);
        };
        let PathScratch {
            nodes,
            dirs,
            corners,
            ..
        } = &mut *scratch;
        first_point_scratch(
            cells, fire, request, start_cell, nodes, end, radius, dirs, corners,
        )
        .map(NewPath::Path)
    })
}

const SEARCH_NONE: u16 = u16::MAX;

#[derive(Debug, Clone, Copy)]
struct SearchNode {
    next: u16,
    parent: u16,
    cost: i16,
    total: i16,
    x: u8,
    y: u8,
    direction: i8,
}

struct PathScratch {
    visited: [u32; 128 * 4],
    nodes: Vec<SearchNode>,
    heap: std::collections::BinaryHeap<(std::cmp::Reverse<i16>, u16)>,
    dirs: Vec<i8>,
    corners: Vec<(i32, i32)>,
}

impl PathScratch {
    fn new() -> Self {
        Self {
            visited: [0; 128 * 4],
            nodes: Vec::with_capacity(NODE_LIMIT as usize + 1),
            heap: std::collections::BinaryHeap::with_capacity(NODE_LIMIT as usize + 1),
            dirs: Vec::with_capacity(64),
            corners: Vec::with_capacity(CORNER_LIMIT as usize + 1),
        }
    }
}

thread_local! {
    static PATH_SCRATCH: std::cell::RefCell<PathScratch> =
        std::cell::RefCell::new(PathScratch::new());
}

fn search_scratch(
    cells: &MeshCells,
    fire: &dyn FireCells,
    request: &mut PathRequest,
    start: (i32, i32),
    goal: (i32, i32),
    scratch: &mut PathScratch,
) -> Result<Option<usize>, Unmodelled> {
    let (w, h) = (cells.width, cells.height);
    let mask = request.impassable;
    let origin = |s: i32, size: i32| -> i32 {
        if s > 0x3f {
            let a = (s - 0x40) as u32;
            let b = size.wrapping_sub(0x80) as u32;
            (if a <= b { a } else { b }) as i32
        } else {
            0
        }
    };
    let (ox, oy) = (origin(start.0, w), origin(start.1, h));
    let mut fire_cost: Option<i32> = None;
    scratch.visited.fill(0);
    scratch.nodes.clear();
    scratch.heap.clear();
    let visited = &mut scratch.visited;
    let nodes = &mut scratch.nodes;
    let heap = &mut scratch.heap;
    let h0 = estimate(start.0 - goal.0, start.1 - goal.1);
    nodes.push(SearchNode {
        next: SEARCH_NONE,
        parent: SEARCH_NONE,
        x: (start.0 - ox) as u8,
        y: (start.1 - oy) as u8,
        direction: -1,
        cost: 0,
        total: h0 as i16,
    });
    let mut best = 0usize;
    let mut best_estimate = i32::from(h0 as i16);
    let mut count: u32 = 1;
    let mut walk = false;
    let mut head: u16 = SEARCH_NONE;
    let mut cur = 0usize;
    let flags = request.flags;
    loop {
        let n = nodes[cur];
        let (lx, ly) = (i32::from(n.x), i32::from(n.y));
        let (cx, cy) = (ox + lx, oy + ly);
        let f = flags_at(cells, cx, cy);
        let hi_open = u32::from(mask) & (1 << (f >> 4 & 0xf)) == 0;
        let lo_open = u32::from(mask) & (1 << (f & 0xf)) == 0;
        let dirs: u32 = if (cy ^ cx) & 1 != 0 {
            match (lo_open, hi_open) {
                (true, true) => 0xff,
                (true, false) => 0x3b,
                (false, true) => 0xce,
                (false, false) => 0x0a,
            }
        } else {
            match (lo_open, hi_open) {
                (true, true) => 0xff,
                (true, false) => 0x67,
                (false, true) => 0x9d,
                (false, false) => 0x05,
            }
        };
        for (dir, &(dx, dy)) in DIRECTIONS.iter().enumerate() {
            if dirs >> dir & 1 == 0 {
                continue;
            }
            let nlx = lx + dx;
            let nx = nlx + ox;
            if (w as u32) <= nx as u32 {
                continue;
            }
            let nly = ly + dy;
            let ny = nly + oy;
            if (h as u32) <= ny as u32 {
                continue;
            }
            if (nlx | nly) as u32 > 0x7f {
                if closed(mask, flags_at(cells, nx, ny)) {
                    continue;
                }
                request.flags = flags | 1;
                return Ok(Some(cur));
            }
            let bit = (nly * 128 + nlx) as usize;
            if visited[bit >> 5] >> (bit & 31) & 1 != 0 {
                continue;
            }
            let nf = flags_at(cells, nx, ny);
            if closed(mask, nf) {
                continue;
            }
            visited[bit >> 5] |= 1 << (bit & 31);
            let mut step: i32 = if n.direction <= 3 { 0x16 } else { 0x10 };
            if nf & 0x200 != 0 && fire.on_fire(nx, ny) {
                let k = match fire_cost {
                    Some(k) => k,
                    None => {
                        let k = i32::from((i32::from(fire.fire_modifier()?) * 5) as i16);
                        fire_cost = Some(k);
                        k
                    }
                };
                step += (k * step) >> 8;
            }
            let est = estimate(nx - goal.0, ny - goal.1) as i32;
            if count > NODE_LIMIT {
                continue;
            }
            count += 1;
            let cost = step + i32::from(n.cost);
            let total = est.wrapping_add(cost);
            let k = nodes.len();
            nodes.push(SearchNode {
                next: SEARCH_NONE,
                parent: cur as u16,
                x: nlx as u8,
                y: nly as u8,
                direction: dir as i8,
                cost: cost as i16,
                total: total as i16,
            });
            let new_best = if est < best_estimate {
                best = k;
                est
            } else {
                best_estimate
            };
            if (nx, ny) == goal {
                request.flags = flags;
                return Ok(Some(k));
            }
            match i16::try_from(total) {
                Ok(t) if !walk => heap.push((std::cmp::Reverse(t), k as u16)),
                _ => {
                    if !walk {
                        walk = true;
                        let drained = std::mem::take(heap).into_sorted_vec();
                        for (_, a) in drained {
                            nodes[a as usize].next = head;
                            head = a;
                        }
                    }
                    let mut prev = SEARCH_NONE;
                    let mut at = head;
                    while at != SEARCH_NONE {
                        if total <= i32::from(nodes[at as usize].total) {
                            break;
                        }
                        prev = at;
                        at = nodes[at as usize].next;
                    }
                    nodes[k].next = at;
                    if prev == SEARCH_NONE {
                        head = k as u16;
                    } else {
                        nodes[prev as usize].next = k as u16;
                    }
                }
            }
            best_estimate = new_best;
        }
        let next = if walk {
            let n = head;
            if n != SEARCH_NONE {
                head = nodes[n as usize].next;
                Some(n as usize)
            } else {
                None
            }
        } else {
            heap.pop().map(|(_, n)| n as usize)
        };
        match next {
            Some(next) => {
                cur = next;
            }
            None => {
                if best == 0 {
                    request.flags = flags;
                    return Ok(None);
                }
                request.flags = flags | 1;
                return Ok(Some(best));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn first_point_scratch(
    cells: &MeshCells,
    fire: &dyn FireCells,
    request: &PathRequest,
    start_cell: (i32, i32),
    nodes: &[SearchNode],
    end: usize,
    radius: Option<i16>,
    dirs: &mut Vec<i8>,
    corners: &mut Vec<(i32, i32)>,
) -> Result<Path, Unmodelled> {
    let flags = request.flags;
    dirs.clear();
    let mut n = end;
    while nodes[n].parent != SEARCH_NONE {
        dirs.push(nodes[n].direction);
        n = nodes[n].parent as usize;
    }
    corners.clear();
    let mut count: i16 = 0;
    let (mut x, mut y) = (start_cell.0 as i16, start_cell.1 as i16);
    let mut prev: i8 = -1;
    for &d in dirs.iter().rev() {
        let (dx, dy) = DIRECTIONS[d as usize];
        x = x.wrapping_add(dx as i16);
        y = y.wrapping_add(dy as i16);
        if prev != -1 && d == prev {
            count -= 1;
        } else {
            prev = d;
        }
        if count <= CORNER_LIMIT {
            let p = ((i32::from(x) << 9) + 0x100, (i32::from(y) << 9) + 0x100);
            let i = count as usize;
            if i < corners.len() {
                corners[i] = p;
            } else {
                corners.push(p);
            }
            count += 1;
        }
    }
    first_point_from_corners(cells, fire, request, corners, count, flags, radius)
}

#[derive(Debug, Clone, Copy)]
pub struct Node {
    next: Option<usize>,
    pub parent: Option<usize>,
    pub x: u8,
    pub y: u8,
    pub direction: i8,
    pub cost: i16,
    pub total: i16,
}

fn estimate(dx: i32, dy: i32) -> u32 {
    let a = (dx.unsigned_abs()) << 4;
    let b = (dy.unsigned_abs()) << 4;
    if a > b { a + (b >> 1) } else { b + (a >> 1) }
}

fn closed(mask: u16, flags: u16) -> bool {
    let m = u32::from(mask);
    m & ((1 << (flags >> 4 & 0xf)) | (1 << (flags & 0xf))) != 0
}

fn flags_at(cells: &MeshCells, x: i32, y: i32) -> u16 {
    cells.flags[(y * cells.width + x) as usize]
}

pub fn search(
    cells: &MeshCells,
    fire: &dyn FireCells,
    request: &mut PathRequest,
    start: (i32, i32),
    goal: (i32, i32),
) -> Result<Option<(Vec<Node>, usize)>, Unmodelled> {
    let (w, h) = (cells.width, cells.height);
    let mask = request.impassable;
    let origin = |s: i32, size: i32| -> i32 {
        if s > 0x3f {
            let a = (s - 0x40) as u32;
            let b = size.wrapping_sub(0x80) as u32;
            (if a <= b { a } else { b }) as i32
        } else {
            0
        }
    };
    let (ox, oy) = (origin(start.0, w), origin(start.1, h));
    let mut fire_cost: Option<i32> = None;
    let mut visited = vec![0u32; 128 * 4];
    let mut nodes = Vec::with_capacity(NODE_LIMIT as usize + 1);
    let h0 = estimate(start.0 - goal.0, start.1 - goal.1);
    nodes.push(Node {
        next: None,
        parent: None,
        x: (start.0 - ox) as u8,
        y: (start.1 - oy) as u8,
        direction: -1,
        cost: 0,
        total: h0 as i16,
    });
    let mut best = 0usize;
    let mut best_estimate = i32::from(h0 as i16);
    let mut count: u32 = 1;
    let mut heap: std::collections::BinaryHeap<(std::cmp::Reverse<i16>, usize)> =
        std::collections::BinaryHeap::with_capacity(NODE_LIMIT as usize + 1);
    let mut walk = false;
    let mut head: Option<usize> = None;
    let mut cur = 0usize;
    let flags = request.flags;
    loop {
        let n = nodes[cur];
        let (lx, ly) = (i32::from(n.x), i32::from(n.y));
        let (cx, cy) = (ox + lx, oy + ly);
        let f = flags_at(cells, cx, cy);
        let hi_open = u32::from(mask) & (1 << (f >> 4 & 0xf)) == 0;
        let lo_open = u32::from(mask) & (1 << (f & 0xf)) == 0;
        let dirs: u32 = if (cy ^ cx) & 1 != 0 {
            match (lo_open, hi_open) {
                (true, true) => 0xff,
                (true, false) => 0x3b,
                (false, true) => 0xce,
                (false, false) => 0x0a,
            }
        } else {
            match (lo_open, hi_open) {
                (true, true) => 0xff,
                (true, false) => 0x67,
                (false, true) => 0x9d,
                (false, false) => 0x05,
            }
        };
        for (dir, &(dx, dy)) in DIRECTIONS.iter().enumerate() {
            if dirs >> dir & 1 == 0 {
                continue;
            }
            let nlx = lx + dx;
            let nx = nlx + ox;
            if (w as u32) <= nx as u32 {
                continue;
            }
            let nly = ly + dy;
            let ny = nly + oy;
            if (h as u32) <= ny as u32 {
                continue;
            }
            if (nlx | nly) as u32 > 0x7f {
                if closed(mask, flags_at(cells, nx, ny)) {
                    continue;
                }
                request.flags = flags | 1;
                return Ok(Some((nodes, cur)));
            }
            let bit = (nly * 128 + nlx) as usize;
            if visited[bit >> 5] >> (bit & 31) & 1 != 0 {
                continue;
            }
            let nf = flags_at(cells, nx, ny);
            if closed(mask, nf) {
                continue;
            }
            visited[bit >> 5] |= 1 << (bit & 31);
            let mut step: i32 = if n.direction <= 3 { 0x16 } else { 0x10 };
            if nf & 0x200 != 0 && fire.on_fire(nx, ny) {
                let k = match fire_cost {
                    Some(k) => k,
                    None => {
                        let k = i32::from((i32::from(fire.fire_modifier()?) * 5) as i16);
                        fire_cost = Some(k);
                        k
                    }
                };
                step += (k * step) >> 8;
            }
            let est = estimate(nx - goal.0, ny - goal.1) as i32;
            if count > NODE_LIMIT {
                continue;
            }
            count += 1;
            let cost = step + i32::from(n.cost);
            let total = est.wrapping_add(cost);
            let k = nodes.len();
            nodes.push(Node {
                next: None,
                parent: Some(cur),
                x: nlx as u8,
                y: nly as u8,
                direction: dir as i8,
                cost: cost as i16,
                total: total as i16,
            });
            let new_best = if est < best_estimate {
                best = k;
                est
            } else {
                best_estimate
            };
            if (nx, ny) == goal {
                request.flags = flags;
                return Ok(Some((nodes, k)));
            }
            match i16::try_from(total) {
                Ok(t) if !walk => heap.push((std::cmp::Reverse(t), k)),
                _ => {
                    if !walk {
                        walk = true;
                        for (_, a) in std::mem::take(&mut heap).into_sorted_vec() {
                            nodes[a].next = head;
                            head = Some(a);
                        }
                    }
                    let mut prev: Option<usize> = None;
                    let mut at = head;
                    while let Some(a) = at {
                        if total <= i32::from(nodes[a].total) {
                            break;
                        }
                        prev = Some(a);
                        at = nodes[a].next;
                    }
                    nodes[k].next = at;
                    match prev {
                        None => head = Some(k),
                        Some(p) => nodes[p].next = Some(k),
                    }
                }
            }
            best_estimate = new_best;
        }
        let next = if walk {
            let n = head;
            if let Some(n) = n {
                head = nodes[n].next;
            }
            n
        } else {
            heap.pop().map(|(_, n)| n)
        };
        match next {
            Some(next) => {
                cur = next;
            }
            None => {
                if best == 0 {
                    request.flags = flags;
                    return Ok(None);
                }
                request.flags = flags | 1;
                return Ok(Some((nodes, best)));
            }
        }
    }
}

fn blocked(cells: &MeshCells, mask: u16, x: i32, y: i32) -> bool {
    let f = cells.cell_flags(x, y);
    let m = u32::from(mask);
    ((m >> (f >> 4 & 0xf)) | (m >> (f & 0xf))) & 1 != 0
}

fn cell_fails(
    cells: &MeshCells,
    fire: &dyn FireCells,
    mask: u16,
    fire_mod: u16,
    x: i32,
    y: i32,
) -> bool {
    if blocked(cells, mask, x, y) {
        return true;
    }
    fire_mod != 0 && cells.cell_flags(x, y) & 0x200 != 0 && fire.on_fire(x, y)
}

fn line_is_open(
    cells: &MeshCells,
    fire: &dyn FireCells,
    mask: u16,
    fire_mod: u16,
    from: (i32, i32),
    to: (i32, i32),
) -> bool {
    let dx = to.0.wrapping_sub(from.0);
    let dy = to.1.wrapping_sub(from.1);
    let longest = dx.abs().max(dy.abs());
    let pieces: i16 = if longest <= 0x7800 {
        1
    } else {
        (longest / 0x7800 + 1) as i16
    };
    if pieces <= 0 {
        return true;
    }
    let n = i32::from(pieces);
    let fails = |x: i32, y: i32| cell_fails(cells, fire, mask, fire_mod, x, y);
    let mut k: i32 = 0;
    loop {
        let t0 = i32::from(((k << 8) / n) as i16);
        let p0 = (
            (dx.wrapping_mul(t0) >> 8) + from.0,
            (dy.wrapping_mul(t0) >> 8) + from.1,
        );
        let t1 = i32::from(((i32::from((k + 1) as u16) << 8) / n) as i16);
        let p1 = (
            (dx.wrapping_mul(t1) >> 8) + from.0,
            (dy.wrapping_mul(t1) >> 8) + from.1,
        );
        let (c0x, c0y) = (p0.0 >> 9, p0.1 >> 9);
        let (c1x, c1y) = (p1.0 >> 9, p1.1 >> 9);
        let (s0x, s0y) = (i32::from(c0x as i16), i32::from(c0y as i16));
        if fails(s0x, s0y) {
            return false;
        }
        if c0y as i16 == c1y as i16 {
            if c0x as i16 != c1x as i16 {
                let step = if (c0x as i16) < (c1x as i16) { 1 } else { -1 };
                let mut x = step + c0x;
                loop {
                    if fails(i32::from(x as i16), s0y) {
                        return false;
                    }
                    if c1x as i16 == x as i16 {
                        break;
                    }
                    x += step;
                }
            }
        } else if c0x as i16 == c1x as i16 {
            let step = if (c0y as i16) < (c1y as i16) { 1 } else { -1 };
            let mut y = step + c0y;
            loop {
                if fails(s0x, i32::from(y as i16)) {
                    return false;
                }
                if c1y as i16 == y as i16 {
                    break;
                }
                y += step;
            }
        } else {
            let ddx = p1.0.wrapping_sub(p0.0);
            let ddy = p1.1.wrapping_sub(p0.1);
            if ddx.abs() > ddy.abs() {
                let step: i32 = if (c0x as i16) >= (c1x as i16) { -1 } else { 1 };
                let slope = i32::from(((i32::from(ddy as i16) << 8) / ddx) as i16);
                let intercept = p0.1.wrapping_sub(p0.0.wrapping_mul(slope) >> 8);
                let (mut cx, mut cy) = (s0x, s0y);
                loop {
                    let nx = i32::from((cx + step) as i16);
                    let ny = ((((nx << 9).wrapping_mul(slope)) >> 8).wrapping_add(intercept)) >> 9;
                    if ny as i16 != cy as i16 {
                        if step == 1 {
                            cy = i32::from(ny as i16);
                            if fails(cx, cy) {
                                return false;
                            }
                        } else {
                            if fails(nx, cy) {
                                return false;
                            }
                            cy = i32::from(ny as i16);
                        }
                    }
                    if fails(nx, cy) {
                        return false;
                    }
                    cx = nx;
                    if c1x as i16 == cx as i16 {
                        break;
                    }
                    cy = i32::from(ny as i16);
                }
            } else {
                let step: i32 = if (c0y as i16) >= (c1y as i16) { -1 } else { 1 };
                let slope = i32::from(((i32::from(ddx as i16) << 8) / ddy) as i16);
                let intercept = p0.0.wrapping_sub(p0.1.wrapping_mul(slope) >> 8);
                let (mut cx, mut cy) = (s0x, s0y);
                loop {
                    let ny = i32::from((cy + step) as i16);
                    let nx = ((((ny << 9).wrapping_mul(slope)) >> 8).wrapping_add(intercept)) >> 9;
                    if nx as i16 != cx as i16 {
                        if step == 1 {
                            cx = i32::from(nx as i16);
                            if fails(cx, cy) {
                                return false;
                            }
                        } else {
                            if fails(cx, ny) {
                                return false;
                            }
                            cx = i32::from(nx as i16);
                        }
                    }
                    if fails(cx, ny) {
                        return false;
                    }
                    cy = ny;
                    if c1y as i16 == cy as i16 {
                        break;
                    }
                    cx = i32::from(nx as i16);
                }
            }
        }
        k += 1;
        if pieces <= k as i16 {
            return true;
        }
    }
}

pub fn first_point(
    cells: &MeshCells,
    fire: &dyn FireCells,
    request: &PathRequest,
    start_cell: (i32, i32),
    nodes: &[Node],
    end: usize,
    radius: Option<i16>,
) -> Result<Path, Unmodelled> {
    let flags = request.flags;
    let mut dirs: Vec<i8> = Vec::new();
    let mut n = end;
    while let Some(p) = nodes[n].parent {
        dirs.push(nodes[n].direction);
        n = p;
    }
    let mut corners: Vec<(i32, i32)> = Vec::new();
    let mut count: i16 = 0;
    let (mut x, mut y) = (start_cell.0 as i16, start_cell.1 as i16);
    let mut prev: i8 = -1;
    for &d in dirs.iter().rev() {
        let (dx, dy) = DIRECTIONS[d as usize];
        x = x.wrapping_add(dx as i16);
        y = y.wrapping_add(dy as i16);
        if prev != -1 && d == prev {
            count -= 1;
        } else {
            prev = d;
        }
        if count <= CORNER_LIMIT {
            let p = ((i32::from(x) << 9) + 0x100, (i32::from(y) << 9) + 0x100);
            let i = count as usize;
            if i < corners.len() {
                corners[i] = p;
            } else {
                corners.push(p);
            }
            count += 1;
        }
    }
    first_point_from_corners(cells, fire, request, &corners, count, flags, radius)
}

#[allow(clippy::too_many_arguments)]
fn first_point_from_corners(
    cells: &MeshCells,
    fire: &dyn FireCells,
    request: &PathRequest,
    corners: &[(i32, i32)],
    total: i16,
    flags: u16,
    radius: Option<i16>,
) -> Result<Path, Unmodelled> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    if total <= 0 {
        if flags & 1 != 0 {
            return Err(Unmodelled::new(
                draw_site::PARTIAL_PATH_NO_CORNERS,
                "a partial path without corners (the record point is not set)",
            ));
        }
        out.push(request.goal);
    } else {
        let fire_mod = if fire.any() { fire.fire_modifier()? } else { 0 };
        let mut i: i16 = 1;
        loop {
            if total == i && flags & 1 != 0 {
                out.push(corners[(total - 1) as usize]);
            } else {
                let origin = out.last().copied().unwrap_or(request.start);
                let target = if total == i {
                    request.goal
                } else {
                    corners[i as usize]
                };
                if !line_is_open(cells, fire, request.impassable, fire_mod, origin, target) {
                    out.push(corners[(i - 1) as usize]);
                }
            }
            i += 1;
            if total < i {
                break;
            }
        }
        if flags & 1 != 0 {
            return Ok(Path {
                flags,
                point: out[0],
            });
        }
        if !out.is_empty() {
            return Ok(Path {
                flags,
                point: out[0],
            });
        }
        out.push(request.goal);
    }
    let (gx, gy) = (out[0].0 >> 9, out[0].1 >> 9);
    let around = [(gx, gy - 1), (gx, gy + 1), (gx - 1, gy), (gx + 1, gy)];
    if around
        .iter()
        .any(|&(x, y)| cells.obstruction_box(x, y, 0).is_some())
    {
        let r = radius.ok_or_else(|| {
            Unmodelled::new(
                draw_site::PATH_RADIUS_UNKNOWN,
                "object radius (+0x20) unknown next to an obstructing model cell",
            )
        })?;
        let boxes: Vec<[i32; 4]> = around
            .iter()
            .filter_map(|&(x, y)| cells.obstruction_box(x, y, i32::from(r)))
            .collect();
        for [x0, y0, x1, y1] in boxes {
            if let Some((p, _)) = clip_line_to_box(x0, x1, y0, y1, request.start, out[0]) {
                out[0] = p;
            }
        }
    }
    Ok(Path {
        flags,
        point: out[0],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterView {
    pub index: i16,
    pub definition: i16,
    pub flags: u32,
    pub flags_8: u32,
    pub object: i16,
    pub team: i8,
    pub target: i16,
    pub pick_up: i16,
    pub pick_up_identifier: u16,
    pub flying_height: i16,
    pub speed: i16,
    pub mode: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectView {
    pub index: i16,
    pub identifier: u16,
    pub flags: u32,
    pub position: (i32, i32),
    pub facing: u16,
    pub radius: i16,
    pub bounds_low: i16,
    pub bounds_high: i16,
    pub owner: i16,
    pub kind: u8,
    pub z: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionView {
    pub flags: u32,
    pub impassable: u16,
    pub radius: i16,
    pub field_82: i16,
    pub field_1b5: u8,
    pub class: i16,
}

pub enum WorldFireCells<'a> {
    None,
    World(crate::fire::WorldFire<'a>),
}

impl FireCells for WorldFireCells<'_> {
    fn any(&self) -> bool {
        match self {
            Self::None => false,
            Self::World(w) => w.any(),
        }
    }

    fn on_fire(&self, x: i32, y: i32) -> bool {
        match self {
            Self::None => false,
            Self::World(w) => w.on_fire(x, y),
        }
    }

    fn fire_modifier(&self) -> Result<u16, Unmodelled> {
        match self {
            Self::None => Ok(0),
            Self::World(w) => w.fire_modifier(),
        }
    }
}

pub trait ObstacleWorld {
    fn cells(&self) -> &MeshCells;
    fn fire_of(&self, _monster: i16) -> WorldFireCells<'_> {
        WorldFireCells::None
    }
    fn cell_objects(&self, x: i32, y: i32) -> Result<Vec<ObjectView>, Unmodelled>;
    fn window_cells(&self, _x: i32, _y: i32) -> [u16; 16] {
        [0xffff; 16]
    }
    fn window_objects(&self, _x: i32, _y: i32) -> Option<Vec<Vec<ObjectView>>> {
        None
    }
    fn object(&self, index: i16) -> Result<ObjectView, Unmodelled>;
    fn monster(&self, index: i16) -> Result<MonsterView, Unmodelled>;
    fn definition(&self, definition: i16) -> Result<DefinitionView, Unmodelled>;
    fn scenery_flags(&self, definition: i16) -> Result<u32, Unmodelled>;
    fn scenery_flags_of_object(&self, index: i16) -> Result<u32, Unmodelled> {
        let o = self.object(index)?;
        self.scenery_flags(o.owner)
    }
    fn game_options(&self) -> u32;
    fn alliances(&self) -> crate::ally::AllianceView {
        crate::ally::AllianceView::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Obstacle {
    pub kind: i16,
    pub position: (i32, i32),
    pub radius: i16,
    pub object: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdjustState {
    pub target: (i32, i32),
    pub detour: bool,
    pub detour_side: bool,
    pub avoid: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjustTrace {
    pub obstacles: Vec<Obstacle>,
    pub radius: i16,
    pub monster_radius: i16,
    pub pulled: Option<(i32, i32)>,
    pub ends: [(i32, i32); 2],
    pub around: [i16; 2],
    pub keep_side: bool,
}

fn dist(a: (i32, i32), b: (i32, i32)) -> i32 {
    distance2d(a.0.wrapping_sub(b.0), a.1.wrapping_sub(b.1))
}

pub fn monsters_are_allied(
    alliances: &crate::ally::AllianceView,
    a: &MonsterView,
    ad: &DefinitionView,
    b: &MonsterView,
    bd: &DefinitionView,
) -> bool {
    if ad.class == 5 || bd.class == 5 {
        return false;
    }
    if a.flags_8 & 0x4000 != 0 || b.flags_8 & 0x4000 != 0 {
        return false;
    }
    if a.team == b.team {
        return true;
    }
    if a.team < 0 || b.team < 0 {
        return false;
    }
    alliances.teams_are_allied(i16::from(a.team), i16::from(b.team))
}

pub(crate) fn use_monster_pathfinding_radius(
    options: u32,
    alliances: &crate::ally::AllianceView,
    a: &MonsterView,
    ad: &DefinitionView,
    b: &MonsterView,
    bd: &DefinitionView,
) -> bool {
    if options & game_options::TFL != 0 {
        return false;
    }
    if !monsters_are_allied(alliances, a, ad, b, bd) {
        return false;
    }
    if a.flags & 0x11 == 0x11 || b.flags & 0x11 == 0x11 {
        return false;
    }
    let holding = |m: u8| m.wrapping_sub(6) <= 2;
    if !holding(a.mode) && !holding(b.mode) {
        return true;
    }
    a.flags & 9 == 9 && b.index != a.target
}

pub(crate) fn closest_point_on_segment(
    p: (i32, i32),
    a: (i32, i32),
    b: (i32, i32),
) -> (bool, (i32, i32)) {
    let dx = b.0.wrapping_sub(a.0);
    let dy = b.1.wrapping_sub(a.1);
    let len = (dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))) >> 9;
    if len == 0 {
        return (false, b);
    }
    let t = (p.0.wrapping_sub(a.0).wrapping_mul(dx))
        .wrapping_add(p.1.wrapping_sub(a.1).wrapping_mul(dy))
        .wrapping_div(len);
    if t < 0 {
        return (false, a);
    }
    if t > 0x200 {
        return (false, b);
    }
    (
        true,
        (
            (dx.wrapping_mul(t) >> 9).wrapping_add(a.0),
            (dy.wrapping_mul(t) >> 9).wrapping_add(a.1),
        ),
    )
}

pub fn arcsine(s: i32) -> u16 {
    if (s.wrapping_add(0x4000) as u32) > 0x8000 {
        return 0;
    }
    let t = trig_tables();
    let sin = |i: i32| i32::from(t.sine[i.rem_euclid(1024) as usize]);
    let mut i = s >> 6;
    let d0 = sin(i) - s;
    if d0 == 0 {
        return ((i << 6) & 0xffff) as u16;
    }
    if d0 > 0 {
        let mut d = d0;
        loop {
            let j = i - 1;
            let dj = sin(j) - s;
            if dj < 0 {
                let r = if -dj > d { i } else { j };
                return ((r << 6) & 0xffff) as u16;
            }
            if dj == 0 {
                return ((j << 6) & 0xffff) as u16;
            }
            i = j;
            d = dj;
        }
    }
    let mut prev = d0;
    loop {
        i += 1;
        let d = sin(i) - s;
        if d < 0 {
            prev = d;
            continue;
        }
        if d == 0 {
            return ((i << 6) & 0xffff) as u16;
        }
        let r = if -prev < d { i - 1 } else { i };
        return ((r << 6) & 0xffff) as u16;
    }
}

fn scan_window(cells: &MeshCells, position: (i32, i32), facing: u16) -> (i32, i32) {
    let a = u32::from(facing) + 0x2000;
    let c = 2 * (a & 0x3fff) + 0x4000;
    let (bx, by) = match (a & 0xffff) >> 14 {
        2 => (0xc, (c >> 12) as i32),
        3 => (((0x10000 - c) as i32) >> 12, 0xc),
        1 => ((c >> 12) as i32, 4),
        _ => (4, ((0x10000 - c) as i32) >> 12),
    };
    let x0 = (position.0 >> 9) - bx;
    let y0 = (position.1 >> 9) - by;
    let x = if x0 < 0 {
        0
    } else {
        (cells.width - 0x10).min(x0)
    };
    let y = if y0 < 0 {
        0
    } else {
        (cells.height - 0x10).min(y0)
    };
    (x, y)
}

pub fn scan_obstacles(
    world: &dyn ObstacleWorld,
    m: &MonsterView,
    me: &ObjectView,
    def: &DefinitionView,
    facing: u16,
    target_definition: i16,
) -> Result<Vec<Obstacle>, Unmodelled> {
    let cells = world.cells();
    let options = world.game_options();
    let alliances = world.alliances();
    let (wx, wy) = scan_window(cells, me.position, facing);
    let occupied = world.window_cells(wx, wy);
    let mut window = world.window_objects(wx, wy);
    let mut list: Vec<Obstacle> = Vec::new();
    for j in 0..16 {
        'cell: for i in 0..16 {
            if occupied[j as usize] >> i & 1 == 0 {
                #[cfg(feature = "identity-checks")]
                assert!(
                    world
                        .cell_objects(wx + i, wy + j)
                        .is_ok_and(|o| o.is_empty()),
                    "window_cells left out cell ({}, {})",
                    wx + i,
                    wy + j
                );
                continue;
            }
            let objects = match window.as_mut() {
                Some(w) => std::mem::take(&mut w[(16 * j + i) as usize]),
                None => world.cell_objects(wx + i, wy + j)?,
            };
            for o in objects {
                if o.flags & 0x600 != 0x400 || o.index == m.object {
                    continue;
                }
                let mut radius = o.radius;
                let mut kind: i16 = 0;
                if o.kind == 2 {
                    let other = world.monster(o.owner)?;
                    let odef = world.definition(other.definition)?;
                    let (mut r9, r10) = if (m.team != 0 && options & game_options::COOPERATIVE != 0)
                        || m.flags & 0x1000_0000 != 0
                    {
                        (true, false)
                    } else {
                        let d = dist(me.position, o.position);
                        if d - (i32::from(odef.radius) + i32::from(def.radius)) <= 0x600 {
                            (true, true)
                        } else {
                            (def.field_1b5 >> 3 & 1 != 0, true)
                        }
                    };
                    let (k8, k11);
                    let neither = (def.field_82 > 0 && odef.field_82 == 0)
                        || (def.field_82 == 0 && odef.field_82 > 0);
                    if neither {
                        k8 = 2;
                        k11 = 0;
                        r9 = false;
                    } else if use_monster_pathfinding_radius(
                        options, &alliances, m, def, &other, &odef,
                    ) {
                        radius = odef.radius;
                        k8 = 3;
                        k11 = 1;
                    } else {
                        k8 = 2;
                        k11 = 0;
                    }
                    if (def.flags ^ odef.flags) & 8 != 0 {
                        r9 = false;
                    }
                    let solid_pose = o.flags & 0x80_0000 != 0;
                    let wide = |a: u16, b: u16| angular_difference(a, b) > 0x3fff;
                    if other.mode == 4
                        && monsters_are_allied(&alliances, m, def, &other, &odef)
                        && !solid_pose
                        && !wide(me.facing, o.facing)
                        && (m.speed <= other.speed || dist(me.position, o.position) > 0x400)
                    {
                        continue;
                    }
                    let check_facing = if r10 {
                        other.mode == 4
                    } else {
                        def.field_1b5 & 8 != 0
                            && other.mode == 4
                            && monsters_are_allied(&alliances, m, def, &other, &odef)
                    };
                    if check_facing && (solid_pose || !wide(me.facing, o.facing)) {
                        continue;
                    }
                    if solid_pose || other.flags_8 & 4 != 0 {
                        continue;
                    }
                    if other.mode == 1 && other.flags & 0x20 != 0 {
                        continue;
                    }
                    if other.definition == target_definition
                        && !monsters_are_allied(&alliances, m, def, &other, &odef)
                    {
                        continue;
                    }
                    if !r9 {
                        continue;
                    }
                    kind = if matches!(other.mode, 4 | 5) || other.flags & 0x20 != 0 {
                        k8
                    } else {
                        k11
                    };
                } else {
                    if m.flags & 0x4000 != 0
                        && m.pick_up == o.index
                        && o.identifier == m.pick_up_identifier
                    {
                        continue;
                    }
                    if options & game_options::TFL != 0
                        && o.flags & 0x10_0000 != 0
                        && me.flags & 0x1000 != 0
                    {
                        continue;
                    }
                    if def.flags & 8 != 0
                        && matches!(o.kind, 1 | 3 | 4)
                        && i32::from(m.flying_height)
                            > i32::from(o.bounds_high) - i32::from(o.bounds_low)
                    {
                        continue;
                    }
                }
                if list.len() > 0x3f {
                    continue 'cell;
                }
                list.push(Obstacle {
                    kind,
                    position: o.position,
                    radius,
                    object: o.index,
                });
            }
        }
    }
    Ok(list)
}

pub fn adjust_radius(options: u32, def: &DefinitionView, me: &ObjectView) -> i16 {
    if options & (game_options::TFL | game_options::ANTI_CLUMP) != 0 || def.field_1b5 & 8 != 0 {
        def.radius.max(me.radius)
    } else {
        me.radius
    }
}

pub fn adjust_path_destination_for_obstacles(
    world: &dyn ObstacleWorld,
    m: &MonsterView,
    facing: u16,
    path: Option<&Path>,
    state: &mut AdjustState,
) -> Result<AdjustTrace, Unmodelled> {
    let me = world.object(m.object)?;
    let trace = AdjustTrace {
        obstacles: Vec::new(),
        radius: 0,
        monster_radius: 0,
        pulled: None,
        ends: [(-1, -1); 2],
        around: [-1; 2],
        keep_side: false,
    };
    let Some(path) = path else {
        state.target = me.position;
        return Ok(trace);
    };
    state.target = path.point;
    let def = world.definition(m.definition)?;
    let target_definition = if m.flags & 8 != 0 {
        world.monster(m.target)?.definition
    } else {
        -1
    };
    let mut trace = trace;
    trace.radius = adjust_radius(world.game_options(), &def, &me);
    trace.monster_radius = def.radius;
    let obstacles = scan_obstacles(world, m, &me, &def, facing, target_definition)?;
    adjust_with_obstacles(world, m, &me, &def, path, state, obstacles, trace)
}

#[allow(clippy::too_many_arguments)]
pub fn adjust_with_obstacles(
    world: &dyn ObstacleWorld,
    m: &MonsterView,
    me: &ObjectView,
    def: &DefinitionView,
    path: &Path,
    state: &mut AdjustState,
    mut obstacles: Vec<Obstacle>,
    mut trace: AdjustTrace,
) -> Result<AdjustTrace, Unmodelled> {
    let options = world.game_options();
    state.target = path.point;
    let origin = me.position;
    let (tx, ty) = path.point;
    let mut pull: i32 = 9;
    if m.flags & 8 == 0 && !obstacles.is_empty() {
        let mut keep_cache: [Option<bool>; 64] = [None; 64];
        'pull: loop {
            for (idx, e) in obstacles.iter().enumerate() {
                if e.object == -1 || e.kind & 2 != 0 {
                    continue;
                }
                let d = dist(e.position, state.target);
                let mine = if e.kind & 1 != 0 {
                    trace.monster_radius
                } else {
                    trace.radius
                };
                let mut reach = i32::from(e.radius) + i32::from(mine) + 0x5f;
                let keep = if let Some(Some(k)) = keep_cache.get(idx).copied() {
                    k
                } else {
                    let o = world.object(e.object)?;
                    let k = match o.kind {
                        1 => world.scenery_flags_of_object(o.index)? & 8 != 0,
                        3 => o.flags & 0x10_0000 != 0,
                        4 => false,
                        _ => true,
                    };
                    if let Some(slot) = keep_cache.get_mut(idx) {
                        *slot = Some(k);
                    }
                    k
                };
                if !keep {
                    reach /= 2;
                }
                if d < reach {
                    state.target = (
                        origin.0 + (tx - origin.0).wrapping_mul(pull) / 10,
                        origin.1 + (ty - origin.1).wrapping_mul(pull) / 10,
                    );
                    pull -= 1;
                    if pull == -1 {
                        break 'pull;
                    }
                    continue 'pull;
                }
            }
            break;
        }
    }
    let dx = origin.0.wrapping_sub(state.target.0);
    let dy = origin.1.wrapping_sub(state.target.1);
    let near = |v: i32, r: i32| (v.wrapping_add(r) as u32) <= 2 * r as u32;
    if near(dx, 0x7f) && near(dy, 0x7f) {
        state.detour = false;
        trace.obstacles = obstacles;
        return Ok(trace);
    }
    if m.flags & 8 != 0 && near(dx, 0x1ff) && near(dy, 0x1ff) {
        state.detour = false;
        trace.obstacles = obstacles;
        return Ok(trace);
    }
    let vx = state.target.0.wrapping_sub(origin.0);
    let vy = state.target.1.wrapping_sub(origin.1);
    let len = isqrt(vx.wrapping_mul(vx).wrapping_add(vy.wrapping_mul(vy)) as u32);
    if len > 0x2000 {
        state.target = ((vx << 13) / len + origin.0, (vy << 13) / len + origin.1);
    }
    trace.pulled = Some(state.target);
    if obstacles.is_empty() {
        trace.obstacles = obstacles;
        return Ok(trace);
    }
    let mut g = Tangents {
        world,
        m,
        def,
        origin,
        radius: trace.radius,
        monster_radius: trace.monster_radius,
        obstacles: &mut obstacles,
        zeroed: Vec::new(),
        ends: [(-1, -1); 2],
        around: [-1; 2],
        keep_side: false,
        options,
    };
    let target = state.target;
    let mut went = g.walk(target, 0, state.avoid, 0)?;
    if went {
        went = g.walk(target, 1, state.avoid, 0)?;
    }
    trace.ends = g.ends;
    trace.around = g.around;
    trace.keep_side = g.keep_side;
    let ends = g.ends;
    let around = g.around;
    let keep_side = g.keep_side;
    for &(k, r) in g.zeroed.iter().rev() {
        g.obstacles[k].radius = r;
    }
    drop(g);
    trace.obstacles = obstacles;
    if ends[0].0 == -1 || ends[0].1 == -1 || ends[1].0 == -1 || ends[1].1 == -1 {
        return Ok(trace);
    }
    let cells = world.cells();
    let clamp = |p: (i32, i32)| {
        (
            clamp_world(p.0, cells.width),
            clamp_world(p.1, cells.height),
        )
    };
    let a = clamp(ends[0]);
    let b = clamp(ends[1]);
    trace.ends = [a, b];
    let open = |p: (i32, i32)| !blocked(cells, def.impassable, p.0 >> 9, p.1 >> 9);
    let (a_open, b_open) = (open(a), open(b));
    let take_a = if !b_open {
        if !a_open {
            state.detour = true;
            return Ok(trace);
        }
        true
    } else if !a_open {
        false
    } else {
        let t = arctangent(state.target.0 - origin.0, state.target.1 - origin.1);
        let ta = arctangent(a.0 - origin.0, a.1 - origin.1);
        let tb = arctangent(b.0 - origin.0, b.1 - origin.1);
        let mut side = angular_difference(ta, t) < angular_difference(tb, t);
        let skip =
            (m.team != 0 && options & game_options::COOPERATIVE != 0) || m.flags & 0x1000_0000 != 0;
        if went && keep_side && (!skip || (def.radius > 0x63 && def.field_1b5 & 8 != 0)) {
            side = state.detour_side;
        }
        side
    };
    if take_a {
        state.target = a;
        state.detour_side = true;
        state.avoid = around[0];
    } else {
        state.target = b;
        state.detour_side = false;
        state.avoid = around[1];
    }
    if went {
        state.detour = true;
    } else {
        state.detour = false;
        state.avoid = -1;
    }
    Ok(trace)
}

struct Tangents<'a> {
    world: &'a dyn ObstacleWorld,
    m: &'a MonsterView,
    def: &'a DefinitionView,
    origin: (i32, i32),
    radius: i16,
    monster_radius: i16,
    obstacles: &'a mut Vec<Obstacle>,
    zeroed: Vec<(usize, i16)>,
    ends: [(i32, i32); 2],
    around: [i16; 2],
    keep_side: bool,
    options: u32,
}

impl Tangents<'_> {
    fn walk(
        &mut self,
        target: (i32, i32),
        side: usize,
        avoid: i16,
        depth: i32,
    ) -> Result<bool, Unmodelled> {
        if depth > 0x14 {
            return Ok(false);
        }
        let o = self.origin;
        let mut best: Option<usize> = None;
        let mut best_angle: u16 = 0;
        let mut best_half: u16 = 0;
        let mut inside = false;
        let d_target = dist(target, o);
        let td = (target.0 - o.0, target.1 - o.1);
        for k in 0..self.obstacles.len() {
            let e = self.obstacles[k];
            let mine = if e.kind & 1 != 0 {
                self.monster_radius
            } else {
                self.radius
            };
            let reach = i32::from(e.radius) + i32::from(mine);
            let r = reach + 0x55;
            let t = if d_target != 0 {
                ((r + d_target) << 16) / d_target
            } else {
                0
            };
            let far = (
                (td.0.wrapping_mul(t) >> 16) + o.0,
                (td.1.wrapping_mul(t) >> 16) + o.1,
            );
            let (on, near) = closest_point_on_segment(e.position, o, far);
            if !on || !world_distance2d_less_than(e.position, near, r) {
                continue;
            }
            let angle = arctangent(e.position.0 - o.0, e.position.1 - o.1);
            let de = dist(e.position, o);
            let r2 = reach + 0xff;
            let half = if r2 <= de {
                arcsine((r2 << 14) / de)
            } else if de > reach + 0x5e {
                arcsine(((reach + 0x5f) << 14) / de)
            } else {
                best = Some(k);
                best_angle = angle;
                best_half = 0x4000;
                inside = true;
                break;
            };
            if half > best_half {
                best_half = half;
                best = Some(k);
                best_angle = angle;
            }
        }
        let Some(k) = best else {
            self.ends[side] = target;
            return Ok(false);
        };
        let e = self.obstacles[k];
        let mine = if e.kind & 1 != 0 {
            self.monster_radius
        } else {
            self.radius
        };
        let d = dist(e.position, o);
        let reach = i32::from(e.radius) + i32::from(mine);
        let near_r = reach + 0x55;
        let len = if near_r < d {
            let far_r = reach + 0xff;
            let r = if far_r <= d { far_r } else { near_r };
            isqrt(d.wrapping_mul(d).wrapping_sub(r.wrapping_mul(r)) as u32) + 0x100
        } else {
            i32::from(mine) + 0x100
        };
        let t = trig_tables();
        let at = |a: u16| -> (i32, i32) {
            let i = (a.wrapping_add(0x20) >> 6) as usize;
            (
                ((i32::from(t.cosine[i]) * len) >> 14) + o.0,
                ((i32::from(t.sine[i]) * len) >> 14) + o.1,
            )
        };
        let p1 = at(best_angle.wrapping_sub(best_half));
        let p2 = at(best_angle.wrapping_add(best_half));
        if inside {
            self.zeroed.push((k, self.obstacles[k].radius));
            self.obstacles[k].radius = 0;
        }
        let me_flag = if self.m.team != 0 && self.options & game_options::COOPERATIVE != 0 {
            false
        } else {
            self.m.flags & 0x1000_0000 == 0
        };
        let cross = (p2.0 - o.0)
            .wrapping_mul(target.1 - o.1)
            .wrapping_sub((p2.1 - o.1).wrapping_mul(target.0 - o.0));
        let mut check = self.def.field_1b5 & 8 != 0;
        if !check && me_flag {
            let me = self.world.object(self.m.object)?;
            let other = self.world.object(e.object)?;
            check = world_distance2d_less_than(
                me.position,
                other.position,
                i32::from(self.def.radius) + i32::from(self.obstacles[k].radius) + 0x200,
            );
        }
        if check
            && avoid != -1
            && i32::from(avoid) == i32::from(e.object)
            && (me_flag || self.def.radius > 0x63)
        {
            self.keep_side = true;
        }
        self.around[side] = e.object;
        let next = if side == 0 {
            if cross <= 0 { p2 } else { p1 }
        } else if cross > 0 {
            p2
        } else {
            p1
        };
        self.walk(next, side, avoid, depth + 1)?;
        Ok(true)
    }
}
