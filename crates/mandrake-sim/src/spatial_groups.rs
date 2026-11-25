use crate::engine_move::{arctangent, trig_tables};
use crate::engine_rng::EngineWorld;
use crate::projectiles::isqrt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialGroup {
    pub team: i16,
    pub definition: usize,
    pub center: (i32, i32),
    pub radius: i16,
    pub facing: u16,
    pub strength: i16,
    pub range: i16,
    pub velocity: (i16, i16),
    pub supergroup: Option<usize>,
    pub seeded: Option<SpatialSupergroup>,
    pub members: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpatialSupergroup {
    pub team: i16,
    pub first_member: usize,
    pub center: (i32, i32),
    pub radius: i16,
    pub facing: u16,
    pub strength: i16,
    pub members: i16,
    pub range: i16,
    pub velocity: (i16, i16),
}

pub fn supergroups(groups: &[SpatialGroup]) -> impl Iterator<Item = &SpatialSupergroup> {
    groups.iter().filter_map(|g| g.seeded.as_ref())
}

const MAX_SEARCHED: usize = 0x7e;
const POOL_FIRST_LEVEL_MAX: i32 = 0x5ca0;
const POOL_SIZE: i32 = 0x6000;
const SUPERGROUP_RECORD: i32 = 0x28;
const MAX_QUEUED: usize = 16;
const SUPERGROUP_MARGIN: i32 = 0x800;
const MEMBERS_TOUCH: i32 = 0x2ff;

fn window(cell: i32, size: i32) -> i32 {
    let c = cell - 1;
    if c < 0 {
        0
    } else if c >= size - 2 {
        size - 3
    } else {
        c
    }
}

pub fn build_spatial_groups(w: &EngineWorld) -> Vec<SpatialGroup> {
    let n = w.monsters.len();
    let mut grouped = vec![false; n];
    let mut groups = Vec::new();
    let mut pool = 0i32;
    let (cw, ch) = w.mesh_cells;
    let eligible = |i: usize| {
        let m = &w.monsters[i];
        m.flags_8 & 0x10 == 0 && !m.object.no_shape
    };
    for i in 0..n {
        let m = &w.monsters[i];
        if grouped[i] || !eligible(i) || w.monster_definitions[m.definition].field_1b6 == 5 {
            continue;
        }
        if pool > POOL_FIRST_LEVEL_MAX {
            return groups;
        }
        grouped[i] = true;
        let mut members = vec![i];
        let mut k = 0;
        while k < members.len() && members.len() < MAX_SEARCHED {
            let (x, y) = w.monsters[members[k]].position;
            let x0 = window(x >> 9, cw);
            let y0 = window(y >> 9, ch);
            for j in w.monsters_in_cell_box(x0, y0, 3, 3) {
                let o = &w.monsters[j];
                if grouped[j] || !eligible(j) || o.team != m.team || o.definition != m.definition {
                    continue;
                }
                grouped[j] = true;
                members.push(j);
            }
            k += 1;
        }
        pool += 0x40 + 2 * members.len() as i32;
        groups.push(group(w, members));
    }
    build_second_level(w, &mut groups, pool);
    groups
}

pub fn rebuild_second_level(w: &mut EngineWorld) {
    let mut groups = std::mem::take(&mut w.spatial_groups);
    let mut pool = 0;
    for g in &mut groups {
        g.supergroup = None;
        g.seeded = None;
        pool += 0x40 + 2 * g.members.len() as i32;
    }
    build_second_level(w, &mut groups, pool);
    w.spatial_groups = groups;
}

pub fn isqrt_distance2d_less_than(dx: i32, dy: i32, r: i32) -> bool {
    let less = r.wrapping_sub(1);
    if less as u32 > 0xffff {
        return r > 0x10000;
    }
    let d = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32;
    d <= r.wrapping_mul(less) as u32
}

fn members_touch(w: &EngineWorld, a: &SpatialGroup, b: &SpatialGroup) -> bool {
    a.members.iter().any(|&i| {
        let (x, y) = w.monsters[i].position;
        b.members.iter().any(|&j| {
            let (ox, oy) = w.monsters[j].position;
            let dx = x.wrapping_sub(ox).wrapping_abs();
            let dy = y.wrapping_sub(oy).wrapping_abs();
            let d = if dx > dy {
                dx.wrapping_add(dy >> 1)
            } else {
                dy.wrapping_add(dx >> 1)
            };
            d <= MEMBERS_TOUCH
        })
    })
}

pub fn build_second_level(w: &EngineWorld, groups: &mut [SpatialGroup], mut pool: i32) {
    let t = trig_tables();
    let mut taken = vec![false; groups.len()];
    let mut records = 0;
    for seed in 0..groups.len() {
        if taken[seed] {
            continue;
        }
        if pool + SUPERGROUP_RECORD > POOL_SIZE {
            return;
        }
        let (team, definition) = (groups[seed].team, groups[seed].definition);
        let mut rec = SpatialSupergroup {
            team,
            first_member: groups[seed].members[0],
            center: (0, 0),
            radius: 0,
            facing: 0,
            strength: 0,
            members: 0,
            range: 0,
            velocity: (0, 0),
        };
        taken[seed] = true;
        groups[seed].supergroup = Some(records);
        let mut queue = vec![seed];
        let (mut sx, mut sy, mut svx, mut svy, mut fc, mut fs) =
            (0i32, 0i32, 0i32, 0i32, 0i32, 0i32);
        let mut k = 0;
        while k < queue.len() {
            let g = queue[k];
            k += 1;
            let h = &groups[g];
            let (center, radius) = (h.center, i32::from(h.radius));
            rec.members = rec.members.wrapping_add(h.members.len() as i16);
            rec.strength = rec.strength.wrapping_add(h.strength);
            let s = i32::from(h.strength);
            sx = sx.wrapping_add(center.0.wrapping_mul(s));
            sy = sy.wrapping_add(center.1.wrapping_mul(s));
            svx = svx.wrapping_add(i32::from(h.velocity.0).wrapping_mul(s));
            svy = svy.wrapping_add(i32::from(h.velocity.1).wrapping_mul(s));
            let a = (h.facing.wrapping_add(0x20) >> 6) as usize;
            fc = fc.wrapping_add(i32::from(t.cosine[a]).wrapping_mul(s));
            fs = fs.wrapping_add(i32::from(t.sine[a]).wrapping_mul(s));
            for j in g + 1..groups.len() {
                let o = &groups[j];
                if taken[j] || o.team != team || o.definition == definition {
                    continue;
                }
                let near = isqrt_distance2d_less_than(
                    o.center.0.wrapping_sub(center.0),
                    o.center.1.wrapping_sub(center.1),
                    i32::from(o.radius) + radius + SUPERGROUP_MARGIN,
                );
                if !near || !members_touch(w, &groups[g], o) {
                    continue;
                }
                taken[j] = true;
                groups[j].supergroup = Some(records);
                if queue.len() < MAX_QUEUED {
                    queue.push(j);
                }
            }
        }
        if rec.strength != 0 {
            let n = i32::from(rec.strength);
            rec.center = (sx.wrapping_div(n), sy.wrapping_div(n));
            rec.velocity = (svx.wrapping_div(n) as i16, svy.wrapping_div(n) as i16);
            rec.facing = arctangent(fc, fs);
            for &g in &queue {
                let h = &groups[g];
                let ex = h.center.0.wrapping_sub(rec.center.0);
                let ey = h.center.1.wrapping_sub(rec.center.1);
                let d = isqrt(ex.wrapping_mul(ex).wrapping_add(ey.wrapping_mul(ey)) as u32);
                let range = d.wrapping_add(i32::from(h.range));
                rec.range = range.max(i32::from(rec.range)) as i16;
                let radius = d.wrapping_add(i32::from(h.radius));
                rec.radius = radius.max(i32::from(rec.radius)) as i16;
            }
        }
        groups[seed].seeded = Some(rec);
        records += 1;
        pool += SUPERGROUP_RECORD;
    }
}

fn group(w: &EngineWorld, members: Vec<usize>) -> SpatialGroup {
    let t = trig_tables();
    let first = &w.monsters[members[0]];
    let def = &w.monster_definitions[first.definition];
    let n = members.len() as i32;
    let (mut sx, mut sy) = (0i32, 0i32);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let (mut fc, mut fs) = (0i32, 0i32);
    let (mut vx, mut vy) = (0i32, 0i32);
    for &i in &members {
        let m = &w.monsters[i];
        let (x, y) = m.position;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
        sx = sx.wrapping_add(x);
        sy = sy.wrapping_add(y);
        vx = vx.wrapping_add(i32::from(m.object.velocity[0]));
        vy = vy.wrapping_add(i32::from(m.object.velocity[1]));
        let k = (m.facing.wrapping_add(0x20) >> 6) as usize;
        fc = fc.wrapping_add(i32::from(t.cosine[k]));
        fs = fs.wrapping_add(i32::from(t.sine[k]));
    }
    let (cx, cy) = (sx / n, sy / n);
    let radius = (max_x - cx).max(cx - min_x).max(cy - min_y).max(max_y - cy);
    SpatialGroup {
        team: first.team,
        definition: first.definition,
        center: (cx, cy),
        radius: radius as i16,
        facing: arctangent(fc, fs),
        strength: (members.len() as i16).wrapping_mul(def.spatial_group_strength),
        range: def.spatial_group_range,
        velocity: ((vx / n) as i16, (vy / n) as i16),
        supergroup: None,
        seeded: None,
        members,
    }
}
