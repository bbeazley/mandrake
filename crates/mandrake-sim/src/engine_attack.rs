use crate::draw_site;
use crate::engine_move::{EngineMover, ModeHooks, ModeState, MoveMode, arctangent};
use crate::engine_rng::{EngineRng, EngineWorld, Unmodelled, WorldHooks};
use crate::projectile_groups::ProjectileDescriptor;
use mandrake_common::game_options;

pub trait CombatHost {
    fn new_projectile(
        &mut self,
        world: &mut EngineWorld,
        rng: &mut EngineRng,
        actions: &mut Option<crate::map_actions::MapActions>,
        time: i32,
        launcher: (usize, &mut EngineMover),
        desc: &ProjectileDescriptor,
    ) -> Result<Option<i32>, Unmodelled>;

    fn detonate_at_creation(
        &mut self,
        _world: &mut EngineWorld,
        _rng: &mut EngineRng,
        _actions: &mut Option<crate::map_actions::MapActions>,
        _time: i32,
        _launcher: (usize, &mut EngineMover),
        _index: i32,
    ) -> Result<(), Unmodelled> {
        Err(Unmodelled::new(
            draw_site::DETONATE_AT_CREATION,
            "a projectile of the engine model detonates in new_projectile",
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn monster_bumped_flag(
        &mut self,
        _world: &mut EngineWorld,
        _rng: &mut EngineRng,
        _actions: &mut Option<crate::map_actions::MapActions>,
        _time: i32,
        launcher: (usize, &mut EngineMover),
        _object: i16,
        _magnitude: i16,
    ) -> Result<(), Unmodelled> {
        Err(Unmodelled::new(
            draw_site::COLLIDE_FLYING_FLAG,
            format!(
                "monster {}: bumps a flag that flies, with no simulation",
                launcher.0
            ),
        ))
    }
}

pub struct NoHost;

impl CombatHost for NoHost {
    fn new_projectile(
        &mut self,
        world: &mut EngineWorld,
        rng: &mut EngineRng,
        _actions: &mut Option<crate::map_actions::MapActions>,
        _time: i32,
        launcher: (usize, &mut EngineMover),
        desc: &ProjectileDescriptor,
    ) -> Result<Option<i32>, Unmodelled> {
        let (i, mv) = launcher;
        let source = Some(crate::projectile_groups::EngineSpawn::monster_object(
            world,
            i,
            (mv.x, mv.y),
            mv.facing,
        ));
        let mut w = crate::projectile_groups::EngineSpawn {
            world,
            rng,
            source,
            host: None,
        };
        crate::projectile_groups::new_projectile(&mut w, desc)
    }

    fn detonate_at_creation(
        &mut self,
        world: &mut EngineWorld,
        rng: &mut EngineRng,
        _actions: &mut Option<crate::map_actions::MapActions>,
        _time: i32,
        _launcher: (usize, &mut EngineMover),
        index: i32,
    ) -> Result<(), Unmodelled> {
        crate::projectile_groups::detonate_engine_record(world, rng, index)
    }
}

enum Next {
    Chosen,
    ReleaseStored,
    Release,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelObstructionRun {
    pub time: i32,
    pub from: [i32; 3],
    pub to: [i32; 3],
    pub elevation: u16,
    pub flag: bool,
    pub a: bool,
    pub b: bool,
    pub to_after: [i32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrowdCheckRun {
    pub time: i32,
    pub identifier: i16,
    pub flags: u32,
    pub aim: (i32, i32),
    pub allied: i32,
    pub enemy: i32,
}

pub fn crowd_check_fires(allied: i32, enemy: i32, ordered: bool) -> bool {
    allied.wrapping_mul(3) <= enemy || ordered
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThrownSolve {
    Ballistic,
    Reach {
        elevation: u16,
        time: i16,
        velocity: i16,
    },
    OutOfReach {
        elevation: u16,
    },
    NoFlightTime,
}

pub fn thrown_solve(
    own: [i32; 3],
    kp: [i32; 3],
    q: [i32; 3],
    attack_flags: u16,
    mean: i16,
    speed: i16,
    gravity: i16,
) -> ThrownSolve {
    use crate::engine_move::{angle_index, trig_tables};
    use crate::projectiles::isqrt;
    let (dx, dy) = (q[0].wrapping_sub(own[0]), q[1].wrapping_sub(own[1]));
    let dz = q[2].wrapping_sub(own[2]);
    let d = isqrt(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32);
    if attack_flags & 0x8000 != 0 && dz < -0x300 && d <= 0x7ff {
        return ThrownSolve::Ballistic;
    }
    let elevation = arctangent(dz.wrapping_neg(), d) >> 1;
    let (ex, ey) = (q[0].wrapping_sub(kp[0]), q[1].wrapping_sub(kp[1]));
    let d2 = isqrt(ex.wrapping_mul(ex).wrapping_add(ey.wrapping_mul(ey)) as u32);
    let t = trig_tables();
    let k = angle_index(elevation);
    let c = i32::from(t.cosine[k]);
    if c == 0 {
        return ThrownSolve::OutOfReach { elevation };
    }
    let r = q[2]
        .wrapping_sub(kp[2])
        .wrapping_sub(i32::from(t.sine[k]).wrapping_mul(d2) / c);
    if r >= 0 || gravity == 0 {
        return ThrownSolve::OutOfReach { elevation };
    }
    let vmax = i32::from(((i32::from(mean) * i32::from(speed)) >> 8) as i16);
    let ft = isqrt((r.wrapping_mul(2) / i32::from(gravity)) as u32);
    let divisor = c.wrapping_mul(ft);
    if divisor == 0 {
        return ThrownSolve::NoFlightTime;
    }
    let v = d2.wrapping_shl(14) / divisor;
    if (vmax * 0x19) >> 8 > v || vmax < v {
        return ThrownSolve::OutOfReach { elevation };
    }
    ThrownSolve::Reach {
        elevation,
        time: ft as i16,
        velocity: v as i16,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrownAimRun {
    pub time: i32,
    pub identifier: i16,
    pub index: usize,
    pub flags: u32,
    pub mode: u8,
    pub attack_flags: u16,
    pub mean: i16,
    pub speed: i16,
    pub gravity: i16,
    pub own: [i32; 3],
    pub aim: [i32; 3],
    pub key_point: [i32; 3],
    pub solve: ThrownSolve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRangeRun {
    pub time: i32,
    pub index: usize,
    pub flags: u32,
    pub mode: u8,
    pub attack_flags: u16,
    pub elevation: u16,
    pub flight_time: i16,
}

thread_local! {
    pub static THROWN_AIM_LOG: std::cell::RefCell<Option<Vec<ThrownAimRun>>> =
        const { std::cell::RefCell::new(None) };
    pub static OUT_OF_RANGE_LOG: std::cell::RefCell<Option<Vec<OutOfRangeRun>>> =
        const { std::cell::RefCell::new(None) };
    pub static CROWD_CHECK_LOG: std::cell::RefCell<Option<Vec<CrowdCheckRun>>> =
        const { std::cell::RefCell::new(None) };
    pub static MODEL_OBSTRUCTION_LOG: std::cell::RefCell<Option<Vec<ModelObstructionRun>>> =
        const { std::cell::RefCell::new(None) };
}

impl EngineWorld {
    pub fn experience(&self, i: usize) -> Result<i32, Unmodelled> {
        let m = &self.monsters[i];
        let def = &self.monster_definitions[m.definition];
        let mut xp = i32::from(self.combat(i).experience);
        let f = def.flags;
        if f & 0x20 != 0 {
            let c = (i32::from(m.object.max_vitality as i16) - i32::from(m.object.vitality as i16))
                >> 8;
            xp += if self.game_options & mandrake_common::game_options::TFL != 0 {
                c / 2
            } else {
                c
            };
        }
        if self.game_options & 0x4000_0000 != 0 && f & 0x400_0000 != 0 {
            xp += self.leaders_experience(i);
        }
        if f & 0x10_0000 != 0 {
            let b = i32::from(def.ai.raw.get(0x51).copied().unwrap_or(0));
            if b != 0 {
                return Ok(b.min(xp));
            }
        }
        Ok(xp.min(5))
    }

    fn leaders_experience(&self, i: usize) -> i32 {
        let me = &self.monsters[i];
        let (x, y) = me.position;
        let (w, h) = self.mesh_cells;
        let x0 = ((x - 0x1000) >> 9).max(0);
        let x1 = ((x + 0x11ff) >> 9).min(w);
        let y0 = ((y - 0x1000) >> 9).max(0);
        let y1 = ((y + 0x11ff) >> 9).min(h);
        if x1 <= x0 || y1 <= y0 {
            return 0;
        }
        let (mut ally, mut enemy) = (0i32, 0i32);
        for j in self.monsters_in_cell_box(x0, y0, x1 - x0, y1 - y0) {
            if j == i {
                continue;
            }
            let other = &self.monsters[j];
            let def = &self.monster_definitions[other.definition];
            if def.flags & 0x800_0000 == 0 {
                continue;
            }
            let raw = &def.ai.raw;
            let b42 = i32::from(raw.get(0x42).copied().unwrap_or(0));
            let b43 = i32::from(raw.get(0x43).copied().unwrap_or(0));
            let w202 = i32::from(def.ai.word(0x202));
            if b42 <= ally && w202 >= enemy {
                continue;
            }
            let (dx, dy) = (other.position.0 - x, other.position.1 - y);
            let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
            if d2 >= b43.wrapping_mul(b43) << 18 {
                continue;
            }
            let allied = self.teams_are_allied(me.team, other.team);
            if b42 != 0 && allied {
                ally = b42;
                continue;
            }
            if w202 != 0 && !allied {
                enemy = w202;
            }
        }
        enemy + ally
    }

    pub fn key_point(&self, i: usize, (x, y): (i32, i32), sequence: i32, facing: u16) -> [i32; 3] {
        let o = &self.monsters[i].object;
        let headers = self.collections.get(&o.collection).map_or(&[][..], |v| v);
        let points = self.key_points.get(&o.collection);
        let s = usize::try_from(sequence)
            .ok()
            .filter(|&s| s < headers.len())
            .unwrap_or(0);
        let key = headers.get(s).map_or(0, |h| h.key_frame);
        let kp = points
            .and_then(|p| p.get(s))
            .and_then(|f| f.get(usize::try_from(key).ok()?))
            .copied()
            .unwrap_or([0; 3]);
        crate::projectiles::object_key_point(
            [x, y, o.z],
            kp,
            o.scale,
            self.monsters[i].mirrored,
            facing,
        )
    }
}

impl EngineWorld {
    pub fn flight_ceiling(&self) -> Option<i16> {
        (self.mesh_flags & 0x0200_0000 != 0).then_some(self.mesh_ceiling)
    }

    pub(crate) fn line_of_fire_blocked(
        &self,
        from: [i32; 3],
        to: [i32; 3],
        elevation: u16,
        gravity: Option<i16>,
    ) -> Result<bool, Unmodelled> {
        use crate::engine_move::{angle_index, trig_tables};
        if self.mesh_flags & 0x0200_0000 != 0 {
            let cells = &self.cells;
            return self.walk_segment_cells(from, to, |x, y| {
                let w = cells.cell_flags(x, y);
                Ok((w >> 4) & 0xf == 0xf || w & 0xf == 0xf)
            });
        }
        let dist = crate::explosions::distance3d(from, to);
        let half = if elevation > 0x8000 {
            -(i32::from(elevation.wrapping_neg()) >> 1)
        } else {
            i32::from(elevation >> 1)
        };
        let (slope, limit) = match gravity {
            None => (half, 0xa00),
            Some(g) if g < 0 => (half, 0x1000 - 0x600),
            Some(_) => (i32::from(elevation), 0x1000),
        };
        let length = if dist <= limit { dist } else { limit };
        let t = trig_tables();
        let k = angle_index(slope as u16);
        let (dx, dy) = (to[0].wrapping_sub(from[0]), to[1].wrapping_sub(from[1]));
        let sin = i32::from(t.sine[k]);
        let mut gain = 0;
        if sin != 0 {
            let cos = i32::from(t.cosine[k]);
            if cos != 0 {
                let h = crate::projectiles::isqrt(
                    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32,
                );
                gain = h.wrapping_mul(sin) / cos;
            }
        }
        let below = |x: i32, y: i32, z: i32| {
            z < i32::from(self.mesh_height(x, y, mandrake_tag::MeshHeightMode::Physical) as i16)
        };
        if dist == 0 {
            return Ok(below(from[0], from[1], from[2]));
        }
        for k in 0..15 {
            let f = ((k << 16) / 15) * length / dist;
            let x = from[0] + (dx.wrapping_mul(f) >> 16);
            let y = from[1] + (dy.wrapping_mul(f) >> 16);
            let z = (f.wrapping_mul(gain) >> 16) + from[2];
            if below(x, y, z) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl EngineWorld {
    fn segment_crosses_model(
        &self,
        a: [i32; 3],
        b: &mut [i32; 3],
        test: mandrake_tag::model_hit::SurfaceTest,
    ) -> Result<bool, Unmodelled> {
        let cells = &self.cells;
        let end = *b;
        let hit_b = std::cell::Cell::new(end);
        let cb = |x: i32, y: i32| -> Result<bool, Unmodelled> {
            let (x, y) = (x as i16, y as i16);
            if x < 0 || i32::from(x) >= cells.width || y < 0 || i32::from(y) >= cells.height {
                return Ok(false);
            }
            let k = (i32::from(y) * cells.width + i32::from(x)) as usize;
            let model = cells.models.get(k).copied().unwrap_or(-1);
            if model == -1 {
                return Ok(false);
            }
            let unmodelled = |what: &str| {
                Unmodelled::new(
                    draw_site::SEGMENT_CROSSES_MODEL,
                    format!("object_hit_model_index: model {model} in cell ({x}, {y}): {what}"),
                )
            };
            let m = self
                .models
                .get(model as usize)
                .ok_or_else(|| unmodelled("no model record"))?;
            match mandrake_tag::model_hit::object_hit_model_index(
                &m.placement,
                &m.geometry,
                a,
                hit_b.get(),
                test,
                0,
            ) {
                Some(h) => {
                    hit_b.set(h.point);
                    Ok(true)
                }
                None => Ok(false),
            }
        };
        let r = self.walk_segment_cells(a, end, cb);
        *b = hit_b.get();
        r
    }

    fn walk_segment_cells(
        &self,
        a: [i32; 3],
        b: [i32; 3],
        cb: impl Fn(i32, i32) -> Result<bool, Unmodelled>,
    ) -> Result<bool, Unmodelled> {
        let (cx0, cy0) = (a[0] >> 9, a[1] >> 9);
        if cb(cx0, cy0)? {
            return Ok(true);
        }
        let (cx1, cy1) = ((b[0] >> 9) as i16, (b[1] >> 9) as i16);
        let w16 = |v: i32| v as i16;
        if w16(cy0) == cy1 {
            if w16(cx0) == cx1 {
                return Ok(false);
            }
            let step = if w16(cx0) < cx1 { 1 } else { -1 };
            let mut x = cx0 + step;
            loop {
                if cb(i32::from(w16(x)), i32::from(w16(cy0)))? {
                    return Ok(true);
                }
                if w16(x) == cx1 {
                    return Ok(false);
                }
                x += step;
            }
        }
        if w16(cx0) == cx1 {
            let step = if w16(cy0) < cy1 { 1 } else { -1 };
            let mut y = cy0 + step;
            loop {
                if cb(i32::from(w16(cx0)), i32::from(w16(y)))? {
                    return Ok(true);
                }
                if w16(y) == cy1 {
                    return Ok(false);
                }
                y += step;
            }
        }
        let (dx, dy) = (b[0].wrapping_sub(a[0]), b[1].wrapping_sub(a[1]));
        if dx.abs() > dy.abs() {
            let slope = i32::from(((i32::from(dy as i16) << 8) / dx) as i16);
            let c = a[1].wrapping_sub(a[0].wrapping_mul(slope) >> 8);
            let row =
                |x: i32| ((i32::from(x as i16) << 9).wrapping_mul(slope) >> 8).wrapping_add(c) >> 9;
            let (mut x, mut y) = (cx0, cy0);
            if w16(cx0) < cx1 {
                loop {
                    let nx = x + 1;
                    let ny = row(nx);
                    let mut yy = i32::from(w16(y));
                    if w16(ny) != w16(y) {
                        yy = i32::from(w16(ny));
                        if cb(i32::from(w16(x)), yy)? {
                            return Ok(true);
                        }
                    }
                    if cb(i32::from(w16(nx)), yy)? {
                        return Ok(true);
                    }
                    if cx1 == w16(nx) {
                        return Ok(false);
                    }
                    x = nx;
                    y = ny;
                }
            }
            loop {
                let nx = x - 1;
                let ny = row(nx);
                let mut yy = i32::from(w16(y));
                if w16(ny) != w16(y) {
                    if cb(i32::from(w16(nx)), yy)? {
                        return Ok(true);
                    }
                    yy = i32::from(w16(ny));
                }
                if cb(i32::from(w16(nx)), yy)? {
                    return Ok(true);
                }
                if cx1 == w16(nx) {
                    return Ok(false);
                }
                x = nx;
                y = ny;
            }
        }
        let slope = i32::from(((i32::from(dx as i16) << 8) / dy) as i16);
        let c = a[0].wrapping_sub(a[1].wrapping_mul(slope) >> 8);
        let col =
            |y: i32| ((i32::from(y as i16) << 9).wrapping_mul(slope) >> 8).wrapping_add(c) >> 9;
        let (mut x, mut y) = (cx0, cy0);
        if w16(cy0) < cy1 {
            loop {
                let ny = y + 1;
                let nx = col(ny);
                let mut xx = i32::from(w16(x));
                if w16(nx) != w16(x) {
                    xx = i32::from(w16(nx));
                    if cb(xx, i32::from(w16(y)))? {
                        return Ok(true);
                    }
                }
                if cb(xx, i32::from(w16(ny)))? {
                    return Ok(true);
                }
                if cy1 == w16(ny) {
                    return Ok(false);
                }
                x = nx;
                y = ny;
            }
        }
        loop {
            let ny = y - 1;
            let nx = col(ny);
            if w16(x) != w16(nx) && cb(i32::from(w16(x)), i32::from(w16(ny)))? {
                return Ok(true);
            }
            if cb(i32::from(w16(nx)), i32::from(w16(ny)))? {
                return Ok(true);
            }
            if cy1 == w16(ny) {
                return Ok(false);
            }
            x = nx;
            y = ny;
        }
    }

    pub fn model_obstruction(
        &self,
        from: [i32; 3],
        to: [i32; 3],
        elevation: u16,
        flag: bool,
    ) -> Result<(bool, bool, [i32; 3]), Unmodelled> {
        use crate::engine_move::{angle_index, trig_tables};
        use mandrake_tag::model_hit::SurfaceTest;
        let test = if flag {
            SurfaceTest::Legacy
        } else {
            SurfaceTest::Gameplay
        };
        let mut to = to;
        let d = crate::explosions::distance3d(from, to);
        if d <= 0x7ff {
            let r = self.segment_crosses_model(from, &mut to, test)?;
            return Ok((r, r, to));
        }
        let f = 0x800_0000 / d;
        let half = if elevation > 0x8000 {
            -(i32::from(elevation.wrapping_neg()) >> 1)
        } else {
            i32::from(elevation >> 1)
        };
        let ex = (to[0].wrapping_sub(from[0])).wrapping_mul(f) >> 16;
        let ey = (to[1].wrapping_sub(from[1])).wrapping_mul(f) >> 16;
        let t = trig_tables();
        let k = angle_index(half as u16);
        let (sin, cos) = (i32::from(t.sine[k]), i32::from(t.cosine[k]));
        let mut h = 0;
        if sin != 0 && cos != 0 {
            let r = crate::projectiles::isqrt(
                ex.wrapping_mul(ex).wrapping_add(ey.wrapping_mul(ey)) as u32,
            );
            h = r.wrapping_mul(sin) / cos;
        }
        let mut first = [from[0] + ex, from[1] + ey, from[2].wrapping_add(h)];
        let a = self.segment_crosses_model(from, &mut first, test)?;
        let b = self.segment_crosses_model(
            [to[0] - ex, to[1] - ey, to[2].wrapping_add(h)],
            &mut to,
            test,
        )?;
        Ok((a, b, to))
    }

    #[allow(clippy::too_many_arguments)]
    fn object_in_line(
        &self,
        object: Option<usize>,
        from: [i32; 3],
        to: [i32; 3],
        length: i32,
        elevation: u16,
        height_test: bool,
        any_monster: bool,
        wide: bool,
    ) -> Result<Option<crate::pathfinding::ObjectView>, Unmodelled> {
        let dist = crate::explosions::distance3d(from, to);
        let mut search = if dist <= length { dist } else { length };
        let f = if dist != 0 { (search << 16) / dist } else { 0 };
        let me_index = object.map(|m| self.monsters[m].object.index as i16);
        if let Some(m) = object {
            let def = &self.monster_definitions[self.monsters[m].definition];
            if def.field_199 & 2 != 0 {
                let d = crate::engine_move::distance2d(to[0] - from[0], to[1] - from[1]);
                search = if length <= d { length } else { d };
            }
        }
        let end = (
            from[0] + ((to[0] - from[0]).wrapping_mul(f) >> 16),
            from[1] + ((to[1] - from[1]).wrapping_mul(f) >> 16),
        );
        let quarter = if elevation > 0x8000 {
            (-(i32::from(elevation.wrapping_neg()) >> 2)) as u16
        } else {
            elevation >> 2
        };
        let (w, h) = (self.cells.width, self.cells.height);
        let lo = |a: i32, b: i32, size: i32| {
            let v = (a as i16).min(b as i16) as i32;
            if v <= 0 {
                0
            } else if v <= size {
                v - 1
            } else {
                size - 1
            }
        };
        let hi = |a: i32, b: i32, size: i32| {
            let v = (a as i16).max(b as i16) as i32;
            if v < -1 {
                0
            } else if v < size - 1 {
                v + 1
            } else {
                size - 1
            }
        };
        let (x0, x1) = (
            lo(from[0] >> 9, end.0 >> 9, w),
            hi(from[0] >> 9, end.0 >> 9, w),
        );
        let (y0, y1) = (
            lo(from[1] >> 9, end.1 >> 9, h),
            hi(from[1] >> 9, end.1 >> 9, h),
        );
        let pad = if wide { 0x200 } else { 0x100 };
        let sine = i32::from(
            crate::engine_move::trig_tables().sine[crate::engine_move::angle_index(quarter)],
        );
        let mut best: Option<(crate::pathfinding::ObjectView, i32)> = None;
        let cols = x1 - x0 + 1;
        let mut lists = self.objects_in_box(x0, y0, cols, y1 - y0 + 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let list = std::mem::take(&mut lists[((y - y0) * cols + (x - x0)) as usize]);
                for o in self.with_resting_projectiles(list, (x, y)) {
                    if o.flags & 0x600 != 0x400 || Some(o.index) == me_index {
                        continue;
                    }
                    let d = crate::engine_move::distance2d(
                        o.position.0 - from[0],
                        o.position.1 - from[1],
                    );
                    if search <= d || best.as_ref().is_some_and(|(_, bd)| d >= *bd) {
                        continue;
                    }
                    let (on, pt) = crate::pathfinding::closest_point_on_segment(
                        o.position,
                        (from[0], from[1]),
                        end,
                    );
                    if !on {
                        continue;
                    }
                    let dd =
                        crate::engine_move::distance2d(pt.0 - o.position.0, pt.1 - o.position.1);
                    if i32::from(o.radius) + pad <= dd {
                        continue;
                    }
                    if height_test && !(any_monster && o.kind == 2) {
                        let z = ((sine * d) >> 14) + from[2];
                        if z < o.z + i32::from(o.bounds_low) || z > o.z + i32::from(o.bounds_high) {
                            continue;
                        }
                    }
                    best = Some((o, d));
                }
            }
        }
        Ok(best.map(|(o, _)| o))
    }
}

impl EngineWorld {
    pub fn crowd_at_aim(
        &self,
        i: usize,
        p: &mandrake_tag::ProjectileDefinition,
        q: [i32; 3],
    ) -> (i32, i32) {
        self.crowd_at_aim_with_words(i, p.uword(0x70), p.uword(0x72), q)
    }

    fn crowd_at_aim_with_words(&self, i: usize, w70: u16, w72: u16, q: [i32; 3]) -> (i32, i32) {
        let radius = (i32::from(w70.wrapping_add(w72) as i16) + 0x200) >> 9;
        let bound = |c: i32, n: i32| if c < 0 { 0 } else { c.min(n - 1) };
        let (width, height) = self.mesh_cells;
        let (cx, cy) = (q[0] >> 9, q[1] >> 9);
        let xs = bound(cx - radius, width)..=bound(cx + radius, width);
        let ys = bound(cy - radius, height)..=bound(cy + radius, height);
        let team = self.monsters[i].team;
        let (mut allied, mut enemy) = (0i32, 0i32);
        for (j, m) in self.monsters.iter().enumerate() {
            if m.identifier == 0
                || !xs.contains(&(m.position.0 >> 9))
                || !ys.contains(&(m.position.1 >> 9))
                || self.monster_object_view(j).flags & 0x200 != 0
            {
                continue;
            }
            let vitality = i32::from(m.object.vitality as i16);
            if self.teams_are_allied(team, m.team) {
                allied = allied.wrapping_add(vitality);
            } else {
                enemy = enemy.wrapping_add(vitality);
            }
        }
        (allied, enemy)
    }
}

#[derive(Clone, Copy)]
struct AttackMonsterDef {
    byte_1b4: u8,
    recovery_bounds: (i16, i16),
    max_mana: i16,
}

impl AttackMonsterDef {
    fn of(def: &crate::engine_rng::MonsterDefinition) -> Self {
        Self {
            byte_1b4: def.ai.byte_1b4,
            recovery_bounds: def.ai.recovery_bounds,
            max_mana: def.ai.word(0x214),
        }
    }
}

#[derive(Clone, Copy)]
struct AttackProjectile {
    tag_id: mandrake_common::FourCC,
    flags: u32,
    speed: i16,
    crowd_w70: u16,
    crowd_w72: u16,
}

impl AttackProjectile {
    fn of(p: &mandrake_tag::ProjectileDefinition) -> Self {
        Self {
            tag_id: p.tag_id,
            flags: p.flags,
            speed: p.speed,
            crowd_w70: p.uword(0x70),
            crowd_w72: p.uword(0x72),
        }
    }
}

impl<'r> WorldHooks<'_, 'r> {
    pub(crate) fn for_monster<'s>(&'s mut self, j: usize) -> WorldHooks<'s, 'r> {
        WorldHooks {
            world: &mut *self.world,
            actions: &mut *self.actions,
            rng: &mut *self.rng,
            monster: j,
            time: self.time,
            host: &mut *self.host,
        }
    }

    pub(crate) fn recheck_hold(&mut self, mv: &mut EngineMover) -> Result<bool, Unmodelled> {
        let i = self.monster;
        let old = self.world.monsters[i].flags;
        self.world.monsters[i].flags = old & !0x1_0000;
        let fail8 = |w: &mut EngineWorld| {
            let m = &mut w.monsters[i];
            m.flags_8 = (m.flags_8 & !0x8000) | 0x20;
        };
        if old & 0x2_0000 != 0 {
            fail8(self.world);
            return Ok(false);
        }
        let own = crate::attack_ai::Own::of(mv);
        let a = self.world.choose_attack(i, 1, &own)?;
        let r = a != -1 && self.attack(mv, a, 0)?;
        mv.pose_timer = self.world.hold_timer(i);
        if r {
            let m = &mut self.world.monsters[i];
            m.flags &= !0x10_0000;
            m.flags_8 = (m.flags_8 & !0x20) | 0x8000;
            return Ok(true);
        }
        let w = &mut *self.world;
        let before = w.monsters[i].flags;
        w.monsters[i].flags = before & !0x2000_0000;
        if w.monsters[i].mode == 6 {
            let mut leg = true;
            if before & 0x8_0009 == 9 {
                let own = crate::attack_ai::Own::of(mv);
                let a = w.choose_attack(i, 0, &own)?;
                if a != -1
                    && let Some(atk) = w.attack_definition(i, a)?
                    && let Some((p, _)) = w.attack_projectile(&atk)?
                    && p.flags & 0x2000 != 0
                    && w.walk_for_attack(i, mv)? == 0
                {
                    leg = false;
                }
            }
            let m = &mut w.monsters[i];
            if leg {
                m.flags |= 0x20;
            }
            m.flags &= !0x80;
        }
        fail8(self.world);
        Ok(false)
    }

    pub(crate) fn attack(
        &mut self,
        mv: &mut EngineMover,
        a: i16,
        mode: u8,
    ) -> Result<bool, Unmodelled> {
        use crate::attack_ai::{LocationAttackRun, log_location_attack};
        let i = self.monster;
        let location = self.world.monsters[i].flags & 0x10 != 0;
        if location {
            let f4 = self.world.monsters[i].flags;
            log_location_attack(|| LocationAttackRun::Attack {
                monster: i,
                attack: a,
                mode,
                f4,
                position: (mv.x, mv.y),
            });
        }
        let r = self.attack_inner(mv, a, mode)?;
        if location {
            let f4 = self.world.monsters[i].flags;
            log_location_attack(|| LocationAttackRun::Attacked {
                monster: i,
                result: r,
                f4,
                velocity: self.world.combat(i).velocity,
            });
        }
        Ok(r)
    }

    fn attack_inner(&mut self, mv: &mut EngineMover, a: i16, mode: u8) -> Result<bool, Unmodelled> {
        let i = self.monster;
        if a == -1 {
            return Ok(false);
        }
        let id = self.world.monsters[i].identifier;
        let Some(mut atk) = self.world.attack_definition(i, a)? else {
            return Err(Unmodelled::new(
                draw_site::ATTACK_WITHOUT_ATTACKS,
                format!("monster {id}: sub_51c420 without attacks"),
            ));
        };
        let def = AttackMonsterDef::of(
            &self.world.monster_definitions[self.world.monsters[i].definition],
        );
        let c = self.world.combat(i);
        let m = &self.world.monsters[i];
        let mut ebp = (m.flags & 8 != 0 && c.target == i as i16) || atk.flags & 0x40 != 0;
        if m.flags_8 & 4 != 0 {
            let m = &mut self.world.monsters[i];
            m.object.flags &= 0xcfdf_ffff;
            m.flags_8 &= 0xffff_ffe3;
        }
        let m = &self.world.monsters[i];
        if mode == 2 && m.flags & 0x2000 != 0 {
            return Ok(false);
        }
        if atk.flags & 0x20 != 0 && self.world.combat(i).carried != -1 {
            atk.projectile = Some(self.world.attack_takes_carried(i, a)?);
        }
        let m = &self.world.monsters[i];
        let proj = self
            .world
            .attack_projectile(&atk)?
            .map(|(p, _)| AttackProjectile::of(p));
        let mut result;
        if proj.is_none() || ebp {
            self.world.monsters[i].flags &= !0x8000;
            ebp = true;
            result = true;
        } else if mode != 0 || m.flags & 0x18 != 0 {
            result = false;
        } else {
            return Ok(false);
        }
        let (list, len) = atk.key_point_sequences_array();
        let len = len.max(1);
        let r = i32::from(self.rng.draw(draw_site::ATTACK_CHOICE));
        let key_sequence = i32::from(list[(r % len as i32) as usize]);
        if mode == 0 {
            let extra = if atk.velocity_delta > 0 {
                (u32::from(self.rng.draw(draw_site::ATTACK_VELOCITY)) * atk.velocity_delta as u32)
                    >> 16
            } else {
                0
            };
            self.world.combat_mut(i).velocity =
                (atk.velocity as u16).wrapping_add(extra as u16) as i16;
        }
        let own = (mv.x, mv.y);
        let obj_sequence = i32::from(self.world.monsters[i].object.sequence);
        let mut facing;
        let mut elevation: u16 = 0;
        let mut flight_time: i16 = 0;
        let mut q: [i32; 3];
        let kp: [i32; 3];
        let flags = self.world.monsters[i].flags;
        let next;
        if mode == 2 || ebp || flags & 0x18 == 0 {
            facing = mv.desired_facing;
            self.world.monsters[i].flags |= 0x1000;
            let s = if mode == 0 {
                key_sequence
            } else {
                obj_sequence
            };
            kp = self.world.key_point(i, own, s, facing);
            let t = crate::engine_move::trig_tables();
            let k = crate::engine_move::angle_index(facing);
            let max = i32::from(atk.maximum_range);
            let x = kp[0] + ((i32::from(t.cosine[k]) * max) >> 14);
            let y = kp[1] + ((i32::from(t.sine[k]) * max) >> 14);
            let (w, h) = self.world.mesh_cells;
            let z = if x >= 0 && x < w << 9 && y >= 0 && y < h << 9 {
                i32::from(
                    self.world
                        .mesh_height(x, y, mandrake_tag::MeshHeightMode::Surface)
                        as i16,
                )
            } else {
                kp[2]
            };
            q = [x, y, z];
            next = if result {
                Next::Chosen
            } else if mode != 0 {
                Next::ReleaseStored
            } else {
                Next::Fail
            };
        } else {
            let c = self.world.combat(i);
            let lead = if atk.flags & 8 != 0
                && (flags & 8 == 0
                    || usize::try_from(c.target)
                        .ok()
                        .is_some_and(|t| self.world.monsters[t].flags_8 & 1 != 0))
            {
                i32::from(c.flight_time)
            } else {
                0
            };
            let (mut x, mut y, z);
            if flags & 0x10 != 0 {
                (x, y) = c.location;
                z = i32::from(c.location_z);
            } else {
                let ti = usize::try_from(c.target).map_err(|_| {
                    Unmodelled::new(
                        draw_site::ATTACK_NO_TARGET,
                        format!("monster {id}: no target +0x62"),
                    )
                })?;
                let tm = &self.world.monsters[ti];
                (x, y) = if ti == i { own } else { tm.position };
                let mut tz = tm.object.z;
                let theaders = self
                    .world
                    .collections
                    .get(&tm.object.collection)
                    .map_or(&[][..], |v| v);
                if atk.flags & 4 != 0 {
                    tz -= tm.object.bounds(theaders).map_or(0, |b| i32::from(b.1));
                } else {
                    let s = mandrake_tag::get_sequence(theaders, i32::from(tm.object.sequence))
                        .map_or(Default::default(), |h| h.world_bounds);
                    let hgt = (i32::from(tm.object.scale as i16)
                        * (i32::from(s.top) - i32::from(s.bottom)))
                        >> 8;
                    tz += 3 * (hgt / 4);
                }
                z = tz;
                if lead > 0 {
                    let v = tm.object.velocity;
                    x += i32::from(v[0]) * lead;
                    y += i32::from(v[1]) * lead;
                }
            }
            let p = proj.expect("a projectile without ebp");
            if p.flags & 0x100_0000 != 0 && flags & 8 != 0 {
                if mode == 1 {
                    if flags & 0x8000 != 0 && c.aim.0 != 0 && c.aim.1 != 0 {
                        (x, y) = c.aim;
                    }
                } else if mode == 0 {
                    self.world.combat_mut(i).aim = (x, y);
                }
            }
            q = [x, y, z];
            let (dx, dy) = (x - own.0, y - own.1);
            facing = if dx | dy != 0 {
                arctangent(dx, dy)
            } else {
                mv.desired_facing
            };
            if mode == 0 {
                kp = self.world.key_point(i, own, key_sequence, facing);
            } else {
                kp = self.world.key_point(i, own, obj_sequence, facing);
                if mode == 1 {
                    let (dx, dy) = (x - kp[0], y - kp[1]);
                    if dx | dy != 0 {
                        facing = arctangent(dx, dy);
                    }
                }
            }
            let (reachable, aim_elevation, aim_time) = if p.flags & 0x2000 == 0 {
                let own_z = self.world.monsters[i].object.z;
                self.ranged_aim(i, mode, &atk, p, [own.0, own.1, own_z], kp, &mut q)?
            } else if atk.flags & 0x40 != 0 {
                (true, 0, 0)
            } else {
                let (dx, dy) = (x - own.0, y - own.1);
                let d = crate::projectiles::isqrt(
                    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32,
                );
                if d > i32::from(atk.maximum_range) {
                    (false, 0, 0)
                } else {
                    let target = self.world.combat(i).target;
                    let r = !(self.world.monsters[i].object.flags & 0x80 != 0
                        && flags & 8 != 0
                        && usize::try_from(target)
                            .is_ok_and(|t| self.world.monsters[t].object.flags & 0x1000 != 0));
                    (r, 0, 0)
                }
            };
            if reachable {
                elevation = aim_elevation;
                flight_time = aim_time;
                result = true;
                next = if mode != 1 {
                    Next::Chosen
                } else if self.world.monsters[i].flags & 0x8000 != 0 {
                    Next::ReleaseStored
                } else {
                    Next::Release
                };
            } else {
                let old = self.world.monsters[i].flags;
                self.world.monsters[i].flags = old | 0x1_0000;
                OUT_OF_RANGE_LOG.with(|l| {
                    if let Some(l) = l.borrow_mut().as_mut() {
                        l.push(OutOfRangeRun {
                            time: self.time,
                            index: i,
                            flags: old,
                            mode,
                            attack_flags: atk.flags,
                            elevation: aim_elevation,
                            flight_time: aim_time,
                        });
                    }
                });
                elevation = aim_elevation;
                flight_time = aim_time;
                if atk.flags & 2 != 0 {
                    result = old & 0x20_0000 == 0;
                    next = if mode == 1 {
                        if old & 0x8000 != 0 || !result {
                            Next::ReleaseStored
                        } else {
                            Next::Release
                        }
                    } else if result {
                        Next::Chosen
                    } else if mode != 0 {
                        Next::ReleaseStored
                    } else {
                        Next::Fail
                    };
                } else {
                    next = if mode == 0 {
                        Next::Fail
                    } else {
                        Next::ReleaseStored
                    };
                }
            }
        }
        let next = match next {
            Next::Chosen if mode != 0 => Next::Release,
            n => n,
        };
        match next {
            Next::Fail => {
                self.take_pending();
                Ok(false)
            }
            Next::Chosen => {
                if ebp {
                    facing = mv.facing;
                } else if let Some(p) = proj.filter(|p| p.flags & 0x2000 == 0)
                    && !self.ranged_line_of_fire(mv, &atk, p, kp, q, elevation)?
                {
                    self.take_pending();
                    return Ok(false);
                }
                mv.desired_facing = facing;
                self.world.monsters[i].flags &= 0xffff_5fff;
                let c = self.world.combat_mut(i);
                c.flight_time = flight_time;
                c.elevation = elevation;
                mv.set_mode(MoveMode::Aiming, ModeState::Value(a as i8), self)?;
                let r12 = self.recover_unless_deleted(mv, &atk, def)?;
                self.attack_tail(mv, &atk, result, r12, false)
            }
            Next::ReleaseStored | Next::Release => {
                if matches!(next, Next::ReleaseStored) {
                    elevation = self.world.combat(i).elevation;
                    facing = mv.desired_facing;
                    result = true;
                }
                let launched =
                    self.release(mv, mode, &atk, def, proj, ebp, (q, kp, facing, elevation))?;
                let m = &mut self.world.monsters[i];
                if m.flags_8 & 0x4_0000 != 0 && launched {
                    m.flags &= !0x19;
                    m.flags_8 &= !0x4_0000;
                    m.flags |= 0x1022;
                }
                let r12 = self.recover_unless_deleted(mv, &atk, def)?;
                self.attack_tail(mv, &atk, result, r12, launched)
            }
        }
    }

    fn ranged_line_of_fire(
        &mut self,
        mv: &mut EngineMover,
        atk: &crate::attack_ai::AttackDefinition,
        p: AttackProjectile,
        kp: [i32; 3],
        q: [i32; 3],
        elevation: u16,
    ) -> Result<bool, Unmodelled> {
        use crate::engine_move::{angle_index, trig_tables};
        let i = self.monster;
        let w = &*self.world;
        let id = w.monsters[i].identifier;
        let physics = w
            .catalog
            .flights
            .get(&p.tag_id)
            .map(|f| f.physics)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::LINE_OF_FIRE_PROJECTILE_MISSING,
                    format!("monster {id}: projectile {} not loaded", p.tag_id),
                )
            })?;
        let r10 = p.flags & 0x100_0000 != 0;
        let r15 = physics.flags & 4 != 0;
        let wide = p.flags & 2 != 0;
        let m = &w.monsters[i];
        if m.flags_8 & 0x200 == 0 {
            let flag = m.flags_8 & 0x800_0000 != 0;
            let (a, b, to) = w.model_obstruction(kp, q, elevation, flag)?;
            MODEL_OBSTRUCTION_LOG.with(|l| {
                if let Some(l) = l.borrow_mut().as_mut() {
                    l.push(ModelObstructionRun {
                        time: self.time,
                        from: kp,
                        to: q,
                        elevation,
                        flag,
                        a,
                        b,
                        to_after: to,
                    });
                }
            });
            if a || b {
                return Ok(false);
            }
        }
        if !r10 && !r15 {
            let def = &w.monster_definitions[m.definition];
            let g = (def.field_1b5 & 1 != 0).then_some(physics.gravity);
            if w.line_of_fire_blocked(kp, q, elevation, g)? {
                return Ok(false);
            }
        }
        let t = trig_tables();
        let k = angle_index(arctangent(q[0] - kp[0], q[1] - kp[1]));
        let headers = w
            .collections
            .get(&m.object.collection)
            .map_or(&[][..], |v| v);
        let radius = i32::from(m.object.bounds(headers).map_or(0, |b| b.0));
        let start = [
            kp[0] - ((i32::from(t.cosine[k]) * radius) >> 14),
            kp[1] - ((i32::from(t.sine[k]) * radius) >> 14),
            kp[2],
        ];
        let length = if m.flags & 0x10 == 0 { 0x800 } else { 0x300 };
        let hit = w.object_in_line(
            Some(i),
            start,
            q,
            length,
            elevation,
            !r15,
            atk.flags & 0x4000 != 0,
            wide,
        )?;
        let enemy_or_none = |w: &EngineWorld, o: &Option<crate::pathfinding::ObjectView>| match o {
            None => true,
            Some(o) if o.kind == 2 => !w.monsters_are_allied(i, o.owner as usize),
            Some(_) => false,
        };
        if !enemy_or_none(w, &hit) {
            let o = hit.expect("an object");
            if o.kind == 2 && w.monsters[o.owner as usize].mode == 4 {
                self.world.monster_voice(i, 9, (mv.x, mv.y));
                return Ok(false);
            }
            let a = arctangent(o.position.0 - mv.x, o.position.1 - mv.y);
            let d = crate::engine_move::signed_angular_difference(mv.desired_facing, a);
            if (d + 0x4000) as u32 > 0x8000 {
                return Ok(false);
            }
            let budget = i32::from(o.radius) + radius + 0x55;
            let w = &mut *self.world;
            let m = &mut w.monsters[i];
            let facing = if m.flags & 0x6_0000 != 0 {
                let side = if m.flags_8 & 0x40 == 0 { 0x8000 } else { 0 };
                a.wrapping_sub(0x4000).wrapping_add(side)
            } else if d > 0 {
                m.flags_8 |= 0x40;
                a.wrapping_sub(0x4000)
            } else {
                m.flags_8 &= !0x40;
                a.wrapping_add(0x4000)
            };
            mv.desired_facing = facing;
            mv.leg_budget = budget as i16;
            m.flags = (m.flags & !0x80) | 0x6_0000;
            let kf = angle_index(facing);
            mv.target = (
                mv.x + ((i32::from(t.cosine[kf]) * budget) >> 14),
                mv.y + ((i32::from(t.sine[kf]) * budget) >> 14),
            );
            let state = if mv.mode == MoveMode::Walking {
                ModeState::Keep
            } else {
                ModeState::Zero
            };
            mv.set_mode(MoveMode::Walking, state, self)?;
            return Ok(false);
        }
        let m = &self.world.monsters[i];
        let f = m.flags;
        if f & 0x18 != 8 {
            return Ok(true);
        }
        if f & 4 == 0 {
            let w = &*self.world;
            let target = usize::try_from(w.combat(i).target).ok();
            let back = w.object_in_line(target, q, kp, 0x800, elevation, false, false, wide)?;
            if !enemy_or_none(w, &back) {
                self.world.monsters[i].flags &= !4;
                return Ok(false);
            }
        }
        if atk.flags & 0x400 == 0 {
            return Ok(true);
        }
        let (allied, enemy) = self
            .world
            .crowd_at_aim_with_words(i, p.crowd_w70, p.crowd_w72, q);
        let ordered = self.world.monsters[i].flags & 4 != 0;
        CROWD_CHECK_LOG.with(|l| {
            if let Some(l) = l.borrow_mut().as_mut() {
                l.push(CrowdCheckRun {
                    time: self.time,
                    identifier: id,
                    flags: self.world.monsters[i].flags,
                    aim: (q[0], q[1]),
                    allied,
                    enemy,
                });
            }
        });
        let fires = crowd_check_fires(allied, enemy, ordered);
        if fires && allied.wrapping_mul(3) > enemy {
            self.world.monster_voice(i, 0xb, (mv.x, mv.y));
        }
        Ok(fires)
    }

    #[allow(clippy::too_many_arguments)]
    fn ranged_aim(
        &mut self,
        i: usize,
        mode: u8,
        atk: &crate::attack_ai::AttackDefinition,
        p: AttackProjectile,
        own: [i32; 3],
        kp: [i32; 3],
        q: &mut [i32; 3],
    ) -> Result<(bool, u16, i16), Unmodelled> {
        use crate::engine_move::{angle_index, trig_tables};
        use crate::projectiles::isqrt;
        let id = self.world.monsters[i].identifier;
        let physics = self
            .world
            .catalog
            .flights
            .get(&p.tag_id)
            .map(|f| f.physics)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::AIM_PROJECTILE_MISSING,
                    format!("monster {id}: projectile {} not loaded", p.tag_id),
                )
            })?;
        let g = physics.gravity;
        let speed = p.speed;
        if physics.flags & 4 != 0 {
            let d = crate::engine_move::distance2d(q[0] - own[0], q[1] - own[1]);
            if atk.velocity == 0 {
                return Ok((true, 0, 0));
            }
            return Ok((true, 0, (d / i32::from(atk.velocity)) as i16));
        }
        let half = atk.velocity_delta / 2;
        let mean = atk.velocity.wrapping_add(half);
        let def = &self.world.monster_definitions[self.world.monsters[i].definition];
        let check_terrain = def.field_1b5 & 1 != 0;
        let high = atk.flags & 1 != 0;
        let t = trig_tables();
        let mut ballistic = atk.flags & 0x2000 == 0;
        if !ballistic {
            let solve = thrown_solve(own, kp, *q, atk.flags, mean, speed, g);
            THROWN_AIM_LOG.with(|l| {
                if let Some(l) = l.borrow_mut().as_mut() {
                    l.push(ThrownAimRun {
                        time: self.time,
                        identifier: id,
                        index: i,
                        flags: self.world.monsters[i].flags,
                        mode,
                        attack_flags: atk.flags,
                        mean,
                        speed,
                        gravity: g,
                        own,
                        aim: *q,
                        key_point: kp,
                        solve,
                    });
                }
            });
            match solve {
                ThrownSolve::Ballistic => ballistic = true,
                ThrownSolve::OutOfReach { elevation } => return Ok((false, elevation, 0)),
                ThrownSolve::NoFlightTime => {
                    return Err(Unmodelled::new(
                        draw_site::AIM_NO_FLIGHT_TIME,
                        format!("monster {id}: a thrown aim with no flight time (idiv by 0)"),
                    ));
                }
                ThrownSolve::Reach {
                    elevation,
                    time,
                    velocity,
                } => {
                    self.world.combat_mut(i).velocity = velocity;
                    return Ok((true, elevation, time));
                }
            }
        }
        debug_assert!(ballistic);
        let (dx, dy, dz) = (q[0] - kp[0], q[1] - kp[1], q[2] - kp[2]);
        let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        let d = isqrt(d2 as u32);
        let v = (i32::from(speed) * i32::from(mean)) >> 8;
        let v16 = v as i16;
        if g >= 0 {
            let e = arctangent(d, dz);
            let time = if v16 != 0 {
                isqrt(dz.wrapping_mul(dz).wrapping_add(d2) as u32) / i32::from(v16)
            } else {
                0
            };
            if check_terrain {
                if self.world.line_of_fire_blocked(kp, *q, e, Some(g))? {
                    return Ok((false, e, time as i16));
                }
                let (a, b, to) = self.world.model_obstruction(kp, *q, e, false)?;
                MODEL_OBSTRUCTION_LOG.with(|l| {
                    if let Some(l) = l.borrow_mut().as_mut() {
                        l.push(ModelObstructionRun {
                            time: self.time,
                            from: kp,
                            to: *q,
                            elevation: e,
                            flag: false,
                            a,
                            b,
                            to_after: to,
                        });
                    }
                });
                return Ok((!(a || b), e, time as i16));
            }
            return Ok((true, e, time as i16));
        }
        let r10 = i32::from(v16);
        let gi = i32::from(g);
        let g2 = gi.wrapping_mul(gi);
        let a = (-r10).wrapping_mul(r10).wrapping_sub(gi.wrapping_mul(dz));
        let disc = a.wrapping_mul(a).wrapping_sub(
            d.wrapping_mul(d)
                .wrapping_add(dz.wrapping_mul(dz))
                .wrapping_mul(g2),
        );
        let mut time = None;
        if disc >= 0 {
            let s = isqrt(disc as u32);
            let mut t2 = (-a).wrapping_sub(s).wrapping_mul(2) / g2;
            if t2 <= 0 || high {
                t2 = s.wrapping_sub(a).wrapping_mul(2) / g2;
            }
            if t2 >= 0 {
                time = Some(isqrt(t2 as u32));
            }
        }
        let Some(ft) = time else {
            let h = arctangent(dx, dy);
            let e = arctangent(-dz, d) >> 1;
            let k = angle_index(e);
            let range = {
                let c = (i32::from(t.cosine[k]) * r10) >> 14;
                let s = (i32::from(t.sine[k]) * r10) >> 14;
                (-(2 * c).wrapping_mul(s)) / gi
            };
            let kh = angle_index(h);
            q[0] = kp[0] + ((i32::from(t.cosine[kh]) * range) >> 14);
            q[1] = kp[1] + ((i32::from(t.sine[kh]) * range) >> 14);
            return Ok((false, e, 0));
        };
        if v16 == 0 || ft == 0 {
            return Ok((true, 0, ft as i16));
        }
        let num = dz.wrapping_sub(gi.wrapping_mul(ft).wrapping_mul(ft) >> 1) << 14;
        let sin = (num / r10.wrapping_mul(ft)).clamp(-0x4000, 0x4000);
        Ok((true, crate::pathfinding::arcsine(sin), ft as i16))
    }

    fn recover_unless_deleted(
        &mut self,
        mv: &mut EngineMover,
        atk: &crate::attack_ai::AttackDefinition,
        def: AttackMonsterDef,
    ) -> Result<bool, Unmodelled> {
        let i = self.monster;
        if self.world.monsters[i].identifier == 0 {
            return Ok(false);
        }
        if atk.flags_3c & 0x20 != 0 {
            return Ok(true);
        }
        let jitter = if def.byte_1b4 == 0 {
            let xp = self.world.experience(i)?;
            let base = i32::from(atk.recovery) - i32::from(atk.recovery_experience) * xp;
            let r = i32::from(self.rng.draw(draw_site::RECOVERY_CHANCE));
            base + ((r * 3) >> 15)
        } else {
            let j = i32::from(def.recovery_bounds.1) - i32::from(def.recovery_bounds.0);
            let r15 = if j > 0 {
                (j * i32::from(self.rng.draw(draw_site::RECOVERY_TIME))) >> 16
            } else {
                0
            };
            let xp = self.world.experience(i)?;
            i32::from(atk.recovery) - i32::from(atk.recovery_experience) * xp + r15
        };
        mv.pose_timer = mv.pose_timer.wrapping_add(jitter as i16);
        Ok(true)
    }

    fn take_pending(&mut self) {
        let i = self.monster;
        let p = self.world.combat(i).pending;
        if p == -1 {
            return;
        }
        let c = self.world.combat_mut(i);
        c.target = p;
        c.pending = -1;
        let m = &mut self.world.monsters[i];
        if m.flags_8 & 0x400_0000 != 0 {
            m.flags |= 0x80_0000;
        } else {
            m.flags &= !0x80_0000;
        }
        m.flags_8 &= !0x400_0000;
    }

    fn attack_tail(
        &mut self,
        mv: &EngineMover,
        atk: &crate::attack_ai::AttackDefinition,
        result: bool,
        r12: bool,
        r13: bool,
    ) -> Result<bool, Unmodelled> {
        let i = self.monster;
        let w = &*self.world;
        let m = &w.monsters[i];
        if result
            && m.flags & 8 != 0
            && let Ok(t) = usize::try_from(w.combat(i).target)
            && w.monsters[t].team != m.team
            && w.monsters_are_allied(t, i)
            && !(w.formation_globals.gameplay_version > 0x513
                && atk.flags & 0x80 != 0
                && atk.flags_3c & 1 != 0)
        {
            let (a, b) = (w.monsters[t].team, m.team);
            self.world.ally_with_team(a, b, false);
        }
        let w = &*self.world;
        let m = &w.monsters[i];
        if !r12 || !r13 {
            return Ok(result);
        }
        let st = m.anim_state as i8;
        if st != -1 {
            let check = m.flags & 1 != 0
                || self
                    .world
                    .attack_definition(i, i16::from(st))?
                    .is_some_and(|a2| a2.flags & 0x40 != 0);
            if check && m.flags & 0x8000 == 0 && mv.bump_count > 1 {
                return Ok(result);
            }
        }
        self.take_pending();
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn release(
        &mut self,
        mv: &mut EngineMover,
        mode: u8,
        atk: &crate::attack_ai::AttackDefinition,
        def: AttackMonsterDef,
        proj: Option<AttackProjectile>,
        ebp: bool,
        (q, kp, facing, elevation): ([i32; 3], [i32; 3], u16, u16),
    ) -> Result<bool, Unmodelled> {
        let not_ebp = !ebp;
        let i = self.monster;
        let id = self.world.monsters[i].identifier;
        self.world.combat_mut(i).aim = (0, 0);
        if self.world.monsters[i].flags & 0x2000 != 0 {
            return Ok(false);
        }
        let r = self.rng.draw(draw_site::RELEASE_CHANCE);
        if atk.miss_fraction > r {
            return Ok(false);
        }
        let c = self.world.combat(i);
        let mut velocity = i32::from(c.velocity);
        let m = &mut self.world.monsters[i];
        m.flags |= 0x2000;
        m.flags_8 &= !0x8000;
        let flags = m.flags;
        let owner_object = if atk.flags & 0x20 != 0 {
            i32::from(c.carried)
        } else {
            m.object.index
        };
        let mut target: i16 = -1;
        if mode != 2 && flags & 0x8000 == 0 {
            let t = if flags & 0x18 == 8 { c.target } else { -1 };
            target = if ebp { i as i16 } else { t };
            if let Some(p) = proj
                && p.flags & 0x2000 != 0
                && not_ebp
                && flags & 0x10 == 0
                && c.target != -1
                && self.world.monsters[c.target as usize].identifier != 0
                && self.world.monsters[i].flags_8 & 0x400 != 0
            {
                target = c.target;
            }
        }
        let mut definition = atk.projectile;
        let (mut facing, mut elevation) = (facing, elevation);
        if mode == 2 {
            let half = velocity >> 1;
            if half != 0 {
                let r = i32::from(self.rng.draw(draw_site::RELEASE_SPREAD));
                velocity = r % half + half;
            }
            let r = i32::from(self.rng.draw(draw_site::RELEASE_YAW_ERROR)) & 0x1fff;
            elevation = elevation.wrapping_add((0x1000 - r) as u16);
            let r = i32::from(self.rng.draw(draw_site::RELEASE_PITCH_ERROR)) & 0x1fff;
            facing = facing.wrapping_add((0x1000 - r) as u16);
        } else if flags & 0x8008 == 0x8008 && proj.is_some_and(|p| p.flags & 0x100_0000 != 0) {
            target = -1;
        }
        if proj.is_none() {
            definition = None;
        }
        if mode == 2 && atk.flags & 0x10 != 0 {
            let raw = &self.world.monster_definitions[self.world.monsters[i].definition]
                .ai
                .raw;
            if let Some(ammo) = crate::engine_rng::tag_ref(raw, 0x1ec) {
                if !self.world.catalog.projectiles.contains_key(&ammo) {
                    return Err(Unmodelled::new(
                        draw_site::RELEASE_AMMUNITION_MISSING,
                        format!("monster {id}: ammunition projectile {ammo} is not in the catalog"),
                    ));
                }
                definition = Some(ammo);
            }
        }
        let mut v = crate::projectiles::build_world_vector3d(velocity, facing, elevation);
        if atk.error != 0 {
            let xp = self.world.experience(i)?;
            let red = (atk.error_experience as u16 as i32 * xp) as i16;
            let e = (i32::from(atk.error) - i32::from(red)).max(0);
            if e != 0 {
                for (k, site) in [
                    draw_site::RELEASE_ERROR_X,
                    draw_site::RELEASE_ERROR_Y,
                    draw_site::RELEASE_ERROR_Z,
                ]
                .into_iter()
                .enumerate()
                {
                    let d = crate::projectiles::launch_error(self.rng.draw(site), e);
                    v[k] = v[k].wrapping_add(d as i16);
                }
            }
        }
        let mut record = None;
        if let Some(definition) = definition {
            let desc = ProjectileDescriptor {
                definition,
                flags: 0x10,
                position: kp,
                velocity: v,
                facing,
                elevation,
                word_1c: 0x100,
                owner_monster: i as i16,
                parent: -1,
                owner_object,
                target,
                target_point: q,
            };
            record = self.host.new_projectile(
                &mut *self.world,
                &mut *self.rng,
                &mut *self.actions,
                self.time,
                (i, mv),
                &desc,
            )?;
        }
        let ebp2 = mode == 2
            && atk.flags & 0x10 != 0
            && self.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3;
        let spent = self.world.special_launched(i)?;
        if spent || atk.flags & 0x20 != 0 {
            self.world.carried_launched(i, record, self.rng)?;
        }
        let c = self.world.combat_mut(i);
        if atk.flags & 0x10 != 0 {
            if atk.repetitions < 0 {
                if !ebp2 && c.ammunition == 0 {
                    c.ammunition = (-atk.repetitions) as u8;
                }
            } else {
                c.ammunition = c.ammunition.wrapping_sub(1);
            }
            c.byte_8d |= 0x80;
        }
        let cost = atk.mana;
        if cost > 0 {
            c.mana = (i32::from(c.mana) - i32::from(cost)).max(0) as i16;
        } else if cost < 0 {
            c.mana = i32::from(def.max_mana).min(i32::from(c.mana) - i32::from(cost)) as i16;
        }
        let m = &mut self.world.monsters[i];
        m.flags_8 &= 0xf7ff_fdff;
        let old = m.flags;
        m.flags = old & !0x10;
        if old & 8 == 0 {
            m.flags = old & !0x11;
        }
        Ok(true)
    }

    pub(crate) fn aim_done(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let st = self.world.monsters[i].anim_state as i8;
        let id = self.world.monsters[i].identifier;
        let atk = self.world.attack_definition(i, i16::from(st))?;
        mv.set_mode(MoveMode::Attacking, ModeState::Value(st), self)?;
        let w = &mut *self.world;
        let m = &mut w.monsters[i];
        if m.flags & 0x80_0000 != 0 {
            m.flags = (m.flags & !0x80_0000) | 0x100_0000;
        }
        let Some(atk) = atk else {
            return Ok(());
        };
        let Some((p, _)) = w.attack_projectile(&atk)? else {
            return Ok(());
        };
        let m = &w.monsters[i];
        if p.flags & 0x2000 == 0 || m.flags & 0x18 != 8 || atk.flags & 0x40 != 0 {
            return Ok(());
        }
        let Ok(t) = usize::try_from(w.combat(i).target) else {
            return Ok(());
        };
        let o = &m.object;
        let ticks = mandrake_tag::get_sequence(w.headers(o.collection), i32::from(o.sequence))
            .map_or(0, |h| i32::from(h.ticks_per_frame) * i32::from(h.key_frame));
        let tdef = &w.monster_definitions[w.monsters[t].definition];
        if tdef.ai.block_sequence == -1
            || w.combat(t).target != i as i16
            || w.monsters[t].mode != 6
            || !(1..=25).contains(&ticks)
        {
            return Ok(());
        }
        let tid = w.monsters[t].identifier;
        let Some(mut tmv) = w.movers.get_mut(t).and_then(Option::take) else {
            return Err(Unmodelled::new(
                draw_site::BLOCK_WARNING_NO_MOVER,
                format!("monster {id}: the block warning to {tid} without its mover"),
            ));
        };
        let block = i16::from_le_bytes([tdef.ai.byte_198, tdef.ai.byte_199]);
        let extra = block_warning_bonus(block, tdef.ai.raw.get(0x4d).copied().unwrap_or(0));
        let r = if tmv.pose_timer > 9 {
            Ok(())
        } else {
            tmv.pose_timer = tmv.pose_timer.wrapping_add(0x1e).wrapping_add(extra);
            let mut h = self.for_monster(t);
            tmv.set_mode(
                MoveMode::Blocking,
                ModeState::Value((ticks + 4) as i8),
                &mut h,
            )
        };
        self.world.movers[t] = Some(tmv);
        r
    }

    pub(crate) fn attack_handler(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let anim = self.world.monsters[i].object.anim_flags;
        if anim & 2 != 0 {
            let st = self.world.monsters[i].anim_state as i8;
            self.attack(mv, i16::from(st), 1)?;
        }
        let m = &self.world.monsters[i];
        if m.identifier == 0 || m.object.anim_flags & 4 == 0 {
            return Ok(());
        }
        let st = m.anim_state as i8;
        let mut finish = st == -1;
        if !finish {
            let f = m.flags;
            if f & 1 == 0 {
                finish = !self
                    .world
                    .attack_definition(i, i16::from(st))?
                    .is_some_and(|a| a.flags & 0x40 != 0);
            }
            if !finish {
                if f & 0x8000 != 0 {
                    finish = true;
                } else {
                    mv.bump_count = mv.bump_count.wrapping_sub(1);
                    if mv.bump_count > 0 {
                        self.world.monsters[i].flags &= !0x2000;
                        return Ok(());
                    }
                    finish = true;
                }
            }
        }
        debug_assert!(finish);
        self.finish_action(mv)?;
        let m = &mut self.world.monsters[i];
        if m.flags_8 & 0x4_0000 != 0 {
            m.flags &= !0x19;
        }
        m.flags_8 &= !0x4_0000;
        Ok(())
    }

    pub(crate) fn finish_action(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        if !self.finish(mv)? {
            return Err(Unmodelled::new(
                draw_site::FINISH_ACTION_NO_MODEL,
                "sub_5178b0 without the engine model",
            ));
        }
        Ok(())
    }

    pub(crate) fn flinch_handler(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        if self.world.monsters[i].object.anim_flags & 4 == 0 {
            return Ok(());
        }
        self.finish_action(mv)
    }

    pub(crate) fn block_handler(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let m = &mut self.world.monsters[i];
        m.anim_state = m.anim_state.wrapping_sub(1);
        if (m.anim_state as i8) >= 0 {
            return Ok(());
        }
        self.finish_action(mv)
    }
}

fn block_warning_bonus(block: i16, byte_4d: u8) -> i16 {
    if block < 0 {
        i16::from((byte_4d as i8).max(-0x1d))
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CombatHost, EngineMover, EngineRng, EngineWorld, ProjectileDescriptor, Unmodelled,
        WorldHooks,
    };
    use mandrake_common::FourCC;

    #[derive(Default)]
    struct Launches(Vec<ProjectileDescriptor>);

    impl CombatHost for Launches {
        fn new_projectile(
            &mut self,
            _world: &mut EngineWorld,
            _rng: &mut EngineRng,
            _actions: &mut Option<crate::map_actions::MapActions>,
            _time: i32,
            _launcher: (usize, &mut EngineMover),
            desc: &ProjectileDescriptor,
        ) -> Result<Option<i32>, Unmodelled> {
            self.0.push(*desc);
            Ok(None)
        }
    }

    const BOLT: FourCC = FourCC::new(*b"bolt");
    const AMMO: FourCC = FourCC::new(*b"ammo");

    fn release(mode: u8, flags: u16, ammo: Option<FourCC>) -> (Vec<FourCC>, u8) {
        let mut w = crate::collide::tests::world((10_000, 10_000), (20_000, 20_000));
        w.formation_globals.gameplay_version = 0x73c;
        let raw = &mut w.monster_definitions[0].ai.raw;
        let bytes = ammo.map_or([0xff; 4], |a| a.0);
        raw[0x1ec..0x1f0].copy_from_slice(&bytes);
        for tag in [BOLT, AMMO] {
            let p =
                mandrake_tag::projectile::ProjectileDefinition::from_bytes(tag, &[0; 256]).unwrap();
            w.catalog.projectiles.insert(tag, p);
        }
        w.combat_mut(0).velocity = 40;
        w.combat_mut(0).ammunition = 3;
        let mut atk = crate::attack_ai::AttackDefinition::at(&[0; 0x40], 0);
        atk.flags = flags;
        atk.projectile = Some(BOLT);
        atk.repetitions = 1;
        let proj = w.catalog.projectiles.get(&BOLT).cloned();
        let def = w.monster_definitions[0].clone();
        let mut mv = crate::collide::tests::mover((10_000, 10_000), (0, 0));
        let (mut rng, mut sites) = (mandrake_math::BungieRng::new(5), Default::default());
        let mut r = EngineRng {
            rng: &mut rng,
            sites: &mut sites,
        };
        let mut actions = None;
        let mut host = Launches::default();
        let mut h = WorldHooks {
            world: &mut w,
            actions: &mut actions,
            rng: &mut r,
            monster: 0,
            time: 0,
            host: &mut host,
        };
        let aim = ([20_000, 20_000, 0], [10_000, 10_000, 0], 0, 0);
        let launched = h
            .release(
                &mut mv,
                mode,
                &atk,
                super::AttackMonsterDef::of(&def),
                proj.as_ref().map(super::AttackProjectile::of),
                false,
                aim,
            )
            .unwrap();
        assert!(launched);
        let count = w.combat(0).ammunition;
        (host.0.iter().map(|d| d.definition).collect(), count)
    }

    #[test]
    fn a_hit_release_of_an_ammunition_attack_launches_the_ammunition() {
        assert_eq!(release(2, 0x10, Some(AMMO)), (vec![AMMO], 2));
        assert_eq!(release(2, 0x10, None), (vec![BOLT], 2));
        assert_eq!(release(2, 0, Some(AMMO)), (vec![BOLT], 3));
        assert_eq!(release(1, 0x10, Some(AMMO)), (vec![BOLT], 2));
    }

    fn with_hooks<R>(w: &mut EngineWorld, f: impl FnOnce(&mut WorldHooks<'_, '_>) -> R) -> R {
        let (mut rng, mut sites) = (mandrake_math::BungieRng::new(5), Default::default());
        let mut r = EngineRng {
            rng: &mut rng,
            sites: &mut sites,
        };
        let mut actions = None;
        let mut host = Launches::default();
        let mut h = WorldHooks {
            world: w,
            actions: &mut actions,
            rng: &mut r,
            monster: 0,
            time: 0,
            host: &mut host,
        };
        f(&mut h)
    }

    #[test]
    fn the_hit_reaction_and_the_block_end_in_sub_511b30() {
        use crate::engine_move::MoveMode;
        let start = |mode: MoveMode, anim_flags: u8, anim_state: u8| {
            let mut w = crate::collide::tests::world((10_000, 10_000), (20_000, 20_000));
            w.formation_globals.gameplay_version = 0x73c;
            w.game.winner = -1;
            let m = &mut w.monsters[0];
            m.flags = 0x100_0000 | 0x9002;
            m.flags_8 = 0x4_0000;
            m.object.anim_flags = anim_flags;
            m.anim_state = anim_state;
            let mut mv = crate::collide::tests::mover((10_000, 10_000), (0, 0));
            mv.mode = mode;
            (w, mv)
        };
        let (mut w, mut mv) = start(MoveMode::Flinching, 0, 0);
        with_hooks(&mut w, |h| h.flinch_handler(&mut mv)).unwrap();
        assert_eq!(mv.mode, MoveMode::Flinching);
        let (mut w, mut mv) = start(MoveMode::Flinching, 4, 0);
        with_hooks(&mut w, |h| h.flinch_handler(&mut mv)).unwrap();
        assert_eq!(mv.mode, MoveMode::Idle);
        assert_eq!(w.monsters[0].flags & 0x100_0000, 0);
        assert_eq!(w.monsters[0].flags_8 & 0x4_0000, 0);
        let (mut w, mut mv) = start(MoveMode::Blocking, 0, 1);
        with_hooks(&mut w, |h| h.block_handler(&mut mv)).unwrap();
        assert_eq!((mv.mode, w.monsters[0].anim_state), (MoveMode::Blocking, 0));
        with_hooks(&mut w, |h| h.block_handler(&mut mv)).unwrap();
        assert_eq!(mv.mode, MoveMode::Idle);
        assert_eq!(w.monsters[0].flags & 0x100_0000, 0);
    }

    #[test]
    fn leaders_around_change_the_experience() {
        let run = |other: (i32, i32), team: i16, b42: u8, w202: i16, options: u32| {
            let mut w = crate::collide::tests::world((10_000, 10_000), other);
            w.mesh_cells = (160, 160);
            w.game_options = options;
            let mut leader = w.monster_definitions[0].clone();
            leader.flags = 0x800_0000;
            leader.ai.raw[0x42] = b42;
            leader.ai.raw[0x43] = 2;
            leader.ai.raw[0x202..0x204].copy_from_slice(&w202.to_be_bytes());
            w.monster_definitions.push(leader);
            let own = &mut w.monster_definitions[0];
            own.flags = 0x400_0000 | 0x10_0000;
            own.ai.raw[0x51] = 100;
            w.monsters[1].definition = 1;
            w.monsters[1].team = team;
            w.combat_mut(0).experience = 3;
            w.experience(0).unwrap()
        };
        assert_eq!(run((10_300, 10_000), 1, 2, -3, 0x4000_0000), 5);
        assert_eq!(run((10_300, 10_000), 2, 2, -3, 0x4000_0000), 0);
        assert_eq!(run((10_300, 10_000), 2, 0, 4, 0x4000_0000), 3);
        assert_eq!(run((10_300, 10_000), 2, 1, 4, 0x4000_0000), 7);
        assert_eq!(run((10_300, 10_000), 1, 0, 4, 0x4000_0000), 3);
        assert_eq!(run((10_300, 10_000), 2, 0, 0, 0x4000_0000), 3);
        assert_eq!(run((11_024, 10_000), 1, 2, -3, 0x4000_0000), 3);
        assert_eq!(run((11_023, 10_000), 1, 2, -3, 0x4000_0000), 5);
        assert_eq!(run((10_300, 10_000), 1, 2, -3, 0), 3);
    }

    #[test]
    fn the_line_meets_a_projectile_at_rest() {
        let run = |flags: u32| {
            let mut w = crate::collide::tests::world((10_000, 10_000), (20_000, 20_000));
            let mut object = crate::engine_rng::EngineObject::new(2000, FourCC::new(*b"thmy"));
            object.sequence = 0;
            object.scale = 0x100;
            object.flags = flags;
            object.kind = 4;
            w.projectiles.push(crate::engine_rng::EngineProjectile {
                marker: 0,
                identifier: 7,
                definition: FourCC::new(*b"thmy"),
                first_draw: 0,
                object,
                position: (31_000, 30_000),
                charge_time: 0,
                facing: 0,
                elevation: 0,
                cell_link: 5,
            });
            w.object_in_line(
                None,
                [30_000, 30_000, 0],
                [32_000, 30_000, 0],
                0x800,
                0,
                false,
                false,
                false,
            )
            .unwrap()
            .map(|o| (o.index, o.kind))
        };
        assert_eq!(run(0x400), Some((2000, 4)));
        assert_eq!(run(0), None);
        assert_eq!(run(0x600), None);
    }

    #[test]
    fn a_block_warning_adds_byte_4d_for_a_negative_198() {
        assert_eq!(super::block_warning_bonus(0x40, 0x10), 0);
        assert_eq!(super::block_warning_bonus(-0x8000, 0x10), 0x10);
        assert_eq!(super::block_warning_bonus(-1, 0xf0), -0x10);
        assert_eq!(super::block_warning_bonus(-1, 0x80), -0x1d);
        assert_eq!(super::block_warning_bonus(-1, 0xe3), -0x1d);
    }

    #[test]
    fn a_straight_shot_checks_the_models() {
        let mut w = crate::collide::tests::world((10_000, 10_000), (20_000, 20_000));
        w.formation_globals.gameplay_version = 0x73c;
        w.monster_definitions[0].field_1b5 = 1;
        let p =
            mandrake_tag::projectile::ProjectileDefinition::from_bytes(BOLT, &[0; 256]).unwrap();
        let flight = crate::projectiles::FlightDefinition::new(
            &p,
            mandrake_tag::object::ObjectPhysics::default(),
            Default::default(),
        );
        w.catalog.flights.insert(BOLT, std::sync::Arc::new(flight));
        let atk = crate::attack_ai::AttackDefinition::at(&[0; 0x40], 0);
        super::MODEL_OBSTRUCTION_LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
        let (kp, mut q) = ([10_000, 10_000, 500], [12_000, 10_000, 500]);
        let r = with_hooks(&mut w, |h| {
            h.ranged_aim(
                0,
                1,
                &atk,
                super::AttackProjectile::of(&p),
                [10_000, 10_000, 0],
                kp,
                &mut q,
            )
        });
        let log = super::MODEL_OBSTRUCTION_LOG.with(|l| l.borrow_mut().take().unwrap());
        assert_eq!(r.unwrap(), (true, 0, 0));
        assert_eq!(log.len(), 1);
        assert_eq!((log[0].from, log[0].to, log[0].flag), (kp, q, false));
    }
}
