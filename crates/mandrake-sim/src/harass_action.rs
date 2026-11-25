use crate::draw_site;
use crate::engine_move::{arctangent, distance2d, trig_tables};
use crate::engine_rng::{EngineWorld, MovementHeader, Unmodelled, be_i16};
use crate::map_actions::{MapActionContext, MapActions, TYPE_ACTIONS, TYPE_DISTANCE, TYPE_INTEGER};
use crate::map_attacks::{AttackConfig, attack_with_monster_indexes};
use crate::spatial_groups::SpatialGroup;
use mandrake_common::game_options;

thread_local! {
    pub static HARASS_LOG: std::cell::RefCell<Option<Vec<(i32, String)>>> =
        const { std::cell::RefCell::new(None) };
}

fn log(time: i32, f: impl FnOnce() -> String) {
    HARASS_LOG.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            v.push((time, f()));
        }
    });
}

pub const HARASS_INITIALISED: u32 = 1;
pub const HARASS_WAYPOINT: u32 = 2;
const MAX_SUBJECTS: usize = 0x40;
const SOURCE: i32 = -2;

pub const STATE_IDLE: i16 = 0;
pub const STATE_FETCHING: i16 = 1;
pub const STATE_RETURNING: i16 = 2;
pub const STATE_ENGAGED: i16 = 3;
pub const STATE_ATTACKING: i16 = 4;
pub const STATE_FINISHING: i16 = 5;
pub const STATE_DODGING: i16 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarassEntry {
    pub monster: usize,
    pub identifier: i16,
    pub vitality: i16,
    pub state: i16,
    pub object: i16,
    pub flags: u16,
    pub timer: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HarassState {
    pub action: i16,
    pub radius: i32,
    pub pick_radius: i32,
    pub attack_ratio: i16,
    pub dodge_ratio: i16,
    pub reev: i32,
    pub team: i16,
    pub entries: Vec<HarassEntry>,
    pub waypoint: (i32, i32),
    pub flags: u32,
}

fn nearest_enemy(w: &EngineWorld, team: i16, p: (i32, i32), radius: i32) -> Option<&SpatialGroup> {
    let mut best: Option<&SpatialGroup> = None;
    let mut best_d = i32::MAX;
    for g in &w.spatial_groups {
        if g.team == team || w.teams_are_allied(team, g.team) {
            continue;
        }
        let d = distance(p, g.center);
        if d < radius.min(best_d) {
            best = Some(g);
            best_d = d;
        }
    }
    best
}

fn distance(a: (i32, i32), b: (i32, i32)) -> i32 {
    distance2d(a.0.wrapping_sub(b.0), a.1.wrapping_sub(b.1))
}

fn trig(a: u16) -> (i32, i32) {
    let t = trig_tables();
    let k = ((i32::from(a) + 0x20) >> 6) as usize & 0x3ff;
    (i32::from(t.cosine[k]), i32::from(t.sine[k]))
}

fn polar(c: (i32, i32), a: u16, r: i32) -> (i32, i32) {
    let (cos, sin) = trig(a);
    (
        (cos.wrapping_mul(r) >> 14).wrapping_add(c.0),
        (sin.wrapping_mul(r) >> 14).wrapping_add(c.1),
    )
}

#[derive(Debug, Clone, Copy)]
struct Enemy {
    center: (i32, i32),
    radius: i16,
    range: i16,
    facing: u16,
    class: i16,
}

impl Enemy {
    fn of(w: &EngineWorld, g: &SpatialGroup) -> Self {
        Enemy {
            center: g.center,
            radius: g.radius,
            range: g.range,
            facing: g.facing,
            class: w.monster_definitions[g.definition].field_1b6,
        }
    }

    fn reach(&self) -> i32 {
        i32::from(self.radius) + i32::from(self.range)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarassOrder {
    Attack { point: (i32, i32), radius: i32 },
    Move { point: (i32, i32), facing: u16 },
    PickUp { index: i16, identifier: i16 },
    Walk { point: (i32, i32) },
}

fn attack(e: &Enemy) -> HarassOrder {
    HarassOrder::Attack {
        point: e.center,
        radius: i32::from(e.radius),
    }
}

fn side_step(m: usize, p: (i32, i32), e: &Enemy) -> HarassOrder {
    let a = arctangent(p.0.wrapping_sub(e.center.0), p.1.wrapping_sub(e.center.1));
    let a = if m & 1 != 0 {
        a.wrapping_add(0x800)
    } else {
        a.wrapping_sub(0x800)
    };
    HarassOrder::Move {
        point: polar(e.center, a, i32::from(e.radius)),
        facing: a.wrapping_add(0x8000),
    }
}

fn flank(p: (i32, i32), e: &Enemy) -> HarassOrder {
    let r = (i32::from(e.radius) + 0x400 + i32::from(e.range)).max(0xc00);
    let b = arctangent(p.0.wrapping_sub(e.center.0), p.1.wrapping_sub(e.center.1));
    let (angle, facing) = if b.wrapping_sub(e.facing) < 0x4000 {
        (
            e.facing.wrapping_add(0x4000),
            e.facing.wrapping_add(0xc000).wrapping_add(0x8000),
        )
    } else {
        (
            e.facing.wrapping_add(0x3000),
            e.facing.wrapping_add(0xb000).wrapping_add(0x8000),
        )
    };
    HarassOrder::Move {
        point: polar(e.center, angle, r),
        facing,
    }
}

fn in_front(p: (i32, i32), e: &Enemy) -> bool {
    let (cos, sin) = trig(e.facing);
    p.0.wrapping_sub(e.center.0)
        .wrapping_mul(cos)
        .wrapping_add(sin.wrapping_mul(p.1.wrapping_sub(e.center.1)))
        >= 0
}

#[derive(Debug, Clone, Copy)]
pub struct HarassView {
    pub monster: usize,
    pub position: (i32, i32),
    pub carrying: bool,
    pub vitality: i16,
    pub max_vitality: i16,
}

fn with_enemy(
    s: &HarassState,
    entry: &HarassEntry,
    v: &HarassView,
    e: &Enemy,
) -> Option<(i16, Option<HarassOrder>)> {
    let p = v.position;
    let m = v.monster;
    let d = distance(p, e.center);
    let near = d.wrapping_sub(e.reach()).wrapping_sub(0x400) < 0;
    let far = |stay: Option<i16>| -> (i16, Option<HarassOrder>) {
        if e.reach() <= d && d < s.radius {
            (stay.unwrap_or(STATE_ENGAGED), Some(attack(e)))
        } else {
            (stay.unwrap_or(STATE_DODGING), Some(side_step(m, p, e)))
        }
    };
    let close = |flank_state: i16| -> (i16, Option<HarassOrder>) {
        if !in_front(p, e) {
            if e.class == 1 {
                (STATE_ATTACKING, Some(attack(e)))
            } else {
                (STATE_DODGING, Some(side_step(m, p, e)))
            }
        } else {
            (flank_state, Some(flank(p, e)))
        }
    };
    Some(match entry.state {
        STATE_IDLE | STATE_FETCHING | STATE_RETURNING => {
            if near {
                close(STATE_ENGAGED)
            } else {
                far(None)
            }
        }
        STATE_ENGAGED => {
            if near {
                close(STATE_ENGAGED)
            } else {
                far(Some(STATE_ENGAGED))
            }
        }
        STATE_ATTACKING => {
            if !v.carrying && e.class != 1 {
                (STATE_DODGING, Some(side_step(m, p, e)))
            } else {
                (STATE_ATTACKING, Some(attack(e)))
            }
        }
        STATE_FINISHING => (STATE_FINISHING, Some(attack(e))),
        STATE_DODGING => {
            if near {
                if entry.vitality < v.vitality && d < 0x400 && v.vitality < v.max_vitality / 2 {
                    (STATE_FINISHING, Some(attack(e)))
                } else {
                    (STATE_DODGING, None)
                }
            } else if d < e.reach() || s.radius <= d {
                (STATE_DODGING, Some(side_step(m, p, e)))
            } else {
                (STATE_ENGAGED, Some(attack(e)))
            }
        }
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchCandidate {
    pub index: i16,
    pub distance: i32,
    pub class: i16,
}

pub fn best_fetch(candidates: &[FetchCandidate]) -> Option<FetchCandidate> {
    let mut best: Option<FetchCandidate> = None;
    for &c in candidates {
        let take = match best {
            None => true,
            Some(b) => match c.class {
                6 => match b.class {
                    6 => c.distance < b.distance,
                    8 => false,
                    _ => true,
                },
                8 => b.class != 8 || c.distance < b.distance,
                _ => b.class != 6 && b.class != 8 && c.distance < b.distance,
            },
        };
        if take {
            best = Some(c);
        }
    }
    best
}

impl MapActions {
    pub fn harass_state(&self, i: usize) -> Option<&HarassState> {
        self.harass_glo.get(&i)
    }

    fn harass_initial(
        &self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<HarassState, Unmodelled> {
        let dela = self.long(i, b"dela", TYPE_INTEGER).unwrap_or(0);
        let mut s = HarassState {
            action: i as i16,
            attack_ratio: 0x40,
            dodge_ratio: 0x100,
            reev: self.long(i, b"reev", TYPE_INTEGER).unwrap_or(1),
            radius: self.long(i, b"radi", TYPE_DISTANCE).unwrap_or(0x5000),
            pick_radius: self.long(i, b"prra", TYPE_DISTANCE).unwrap_or(0x5000),
            ..HarassState::default()
        };
        if let Some(p) = self.indexed_point(i, b"wayp", 0) {
            s.waypoint = p;
            s.flags |= HARASS_WAYPOINT;
        }
        for m in self.subjects(i, ctx, MAX_SUBJECTS) {
            let timer = if dela != 0 {
                ctx.rng.draw_mod(draw_site::HARASS_DELAY, dela)? as i16
            } else {
                0
            };
            let w = &ctx.world;
            let mon = &w.monsters[m];
            let class = be_i16(&w.monster_definitions[mon.definition].ai.raw, 0x8a);
            let flags = if class == -1 {
                2
            } else if w.combat(m).carried != -1 {
                1
            } else {
                0
            };
            s.entries.push(HarassEntry {
                monster: m,
                identifier: mon.identifier,
                vitality: 0,
                state: STATE_IDLE,
                object: 0,
                flags,
                timer,
            });
        }
        if let Some(e) = s.entries.first() {
            s.team = ctx.world.monsters[e.monster].team;
        }
        Ok(s)
    }

    pub(crate) fn harass_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        log(ctx.time, || {
            format!("E {i} id={}", self.actions[i].identifier)
        });
        let r = self.harass_inner(i, ctx)?;
        log(ctx.time, || format!("R {i} result={r}"));
        Ok(r)
    }

    fn harass_inner(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let mut s = match self.harass_glo.get(&i) {
            Some(s) => s.clone(),
            None => self.harass_initial(i, ctx)?,
        };
        if self.flag(i, b"timd") {
            return self.harass_timid(i, s, ctx);
        }
        let mut k = s.entries.len();
        while k > 0 {
            k -= 1;
            let e = s.entries[k];
            if e.identifier != 0 && ctx.world.monsters[e.monster].identifier == e.identifier {
                continue;
            }
            s.entries.remove(k);
            self.harass_casualty(i, ctx)?;
        }
        if s.entries.is_empty() {
            return Ok(4);
        }
        s.team = ctx.world.monsters[s.entries[0].monster].team;
        for k in 0..s.entries.len() {
            let Some(d) = harass_decision(ctx.world, &s, k)? else {
                continue;
            };
            if let Some(o) = d.object {
                s.entries[k].object = o;
            }
            if let Some(o) = d.order {
                issue(ctx.world, ctx.time, s.entries[k].monster, o)?;
            }
            s.entries[k].state = d.state;
        }
        for e in &mut s.entries {
            e.vitality = ctx.world.monsters[e.monster].object.vitality as i16;
        }
        s.flags |= HARASS_INITIALISED;
        log(ctx.time, || {
            let es: Vec<String> = s
                .entries
                .iter()
                .enumerate()
                .map(|(k, e)| {
                    format!(
                        "{k}:{}:{}:{}:{}:{}:{:#x}",
                        e.monster, e.identifier, e.vitality, e.state, e.object, e.flags
                    )
                })
                .collect();
            format!(
                "G {} count={} flags={:#x} wayp={},{} radi={} prra={} team={} | {}",
                s.action,
                s.entries.len(),
                s.flags,
                s.waypoint.0,
                s.waypoint.1,
                s.radius,
                s.pick_radius,
                s.team,
                es.join(" ")
            )
        });
        self.harass_glo.insert(i, s);
        Ok(0)
    }

    pub(crate) fn harass_casualty(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let ids: Vec<i32> = self
            .find(i, 0, b"casu", Some(TYPE_ACTIONS))
            .map(|p| p.values.iter().take(0x1e).copied().collect())
            .unwrap_or_default();
        for id in ids {
            let j = self.index_of(id);
            self.activate(j, ctx)?;
        }
        Ok(())
    }
}

pub const TIMID_FETCHING: i16 = 1;
pub const TIMID_ATTACKING: i16 = 4;
pub const TIMID_SEEKING: i16 = 8;
pub const TIMID_PRESSING: i16 = 9;
pub const TIMID_RETURNING: i16 = 10;

#[derive(Debug, Clone, Copy)]
struct TimidEnemy {
    center: (i32, i32),
    facing: u16,
    reach: i32,
    distance: i32,
}

fn timid_enemy(w: &EngineWorld, team: i16, p: (i32, i32), radius: i32) -> Option<TimidEnemy> {
    let mut best: Option<(TimidEnemy, i32)> = None;
    for g in &w.spatial_groups {
        if g.team == team {
            continue;
        }
        let d = distance(p, g.center);
        if radius <= d {
            continue;
        }
        let reach = i32::from(g.radius) + i32::from(g.range);
        let edge = d.wrapping_sub(reach);
        if best.as_ref().is_none_or(|&(_, b)| edge < b) {
            best = Some((
                TimidEnemy {
                    center: g.center,
                    facing: g.facing,
                    reach,
                    distance: d,
                },
                edge,
            ));
        }
    }
    best.map(|(e, _)| e)
}

fn timid_ratio(e: &TimidEnemy) -> Result<i32, Unmodelled> {
    e.distance
        .wrapping_shl(8)
        .checked_div(e.reach)
        .ok_or_else(|| {
            Unmodelled::new(
                draw_site::TIMID_GROUP_REACH_ZERO,
                "girl timd: an enemy group of reach 0",
            )
        })
}

pub fn best_timid_fetch(candidates: &[FetchCandidate]) -> Option<FetchCandidate> {
    let mut best: Option<FetchCandidate> = None;
    for &c in candidates {
        let take = match best {
            None => true,
            Some(b) => match c.class {
                6 | 8 => b.class != 8 || c.distance < b.distance,
                _ => c.distance < b.distance,
            },
        };
        if take {
            best = Some(c);
        }
    }
    best
}

impl MapActions {
    fn harass_timid(
        &mut self,
        i: usize,
        mut s: HarassState,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let mut k = 0;
        while k < s.entries.len() {
            let e = s.entries[k];
            let mon = &ctx.world.monsters[e.monster];
            if e.identifier == 0 || mon.identifier != e.identifier {
                s.entries.remove(k);
                self.harass_casualty(i, ctx)?;
                continue;
            }
            let e = &mut s.entries[k];
            if ctx.world.combat(e.monster).carried == -1 {
                e.flags &= !1;
            } else {
                if e.state == TIMID_FETCHING {
                    e.state = STATE_IDLE;
                    e.timer = 0;
                }
                e.flags |= 1;
            }
            k += 1;
        }
        if s.entries.is_empty() {
            return Ok(4);
        }
        let team = ctx.world.monsters[s.entries[0].monster].team;
        for k in 0..s.entries.len() {
            let e = &mut s.entries[k];
            e.timer = e.timer.wrapping_sub(1);
            if e.timer >= 0 {
                continue;
            }
            e.timer = s.reev as i16;
            self.timid_decide(&mut s, k, team, ctx)?;
        }
        s.flags |= HARASS_INITIALISED;
        log(ctx.time, || {
            let es: Vec<String> = s
                .entries
                .iter()
                .enumerate()
                .map(|(k, e)| {
                    format!(
                        "{k}:{}:{}:{}:{}:{:#x}:{}",
                        e.monster, e.identifier, e.state, e.object, e.flags, e.timer
                    )
                })
                .collect();
            format!(
                "T {} count={} team={team} | {}",
                s.action,
                s.entries.len(),
                es.join(" ")
            )
        });
        self.harass_glo.insert(i, s);
        Ok(0)
    }

    fn timid_decide(
        &mut self,
        s: &mut HarassState,
        k: usize,
        team: i16,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let state = s.entries[k].state;
        if !(0..=10).contains(&state) {
            return Ok(());
        }
        let m = s.entries[k].monster;
        let p = ctx.world.monsters[m].position;
        let behind_attack = match state {
            0 | 4 | 7 | 9 | 10 => true,
            1 => {
                s.entries[k].state = STATE_IDLE;
                true
            }
            TIMID_SEEKING => {
                s.entries[k].state = STATE_IDLE;
                if s.entries[k].flags & 3 != 0 {
                    return Ok(());
                }
                if let Some((index, identifier, d)) = timid_fetch(ctx.world, s, m)? {
                    issue(
                        ctx.world,
                        ctx.time,
                        m,
                        HarassOrder::PickUp { index, identifier },
                    )?;
                    let w = &ctx.world;
                    let speed =
                        if w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1 {
                            w.monsters[m].speed
                        } else {
                            be_i16(
                                &w.monster_definitions[w.monsters[m].definition].ai.raw,
                                0x7e,
                            )
                        };
                    let timer = if speed == 0 {
                        2
                    } else {
                        d.wrapping_div(i32::from(speed) << 5).wrapping_add(2)
                    };
                    s.entries[k].state = TIMID_FETCHING;
                    s.entries[k].timer = timer as i16;
                    return Ok(());
                }
                false
            }
            _ => return Ok(()),
        };
        let Some(e) = timid_enemy(ctx.world, team, p, s.radius) else {
            return self.timid_no_enemy(s, k, ctx);
        };
        if behind_attack {
            let (cos, sin) = trig(e.facing);
            let dot =
                p.0.wrapping_sub(e.center.0)
                    .wrapping_mul(cos)
                    .wrapping_add(p.1.wrapping_sub(e.center.1).wrapping_mul(sin));
            let def = ctx.world.monsters[m].definition;
            if dot <= 0
                && (s.entries[k].flags & 3 != 0
                    || ctx.world.monster_definitions[def].field_1b6 == 1)
            {
                let o = HarassOrder::Attack {
                    point: e.center,
                    radius: e.reach,
                };
                issue(ctx.world, ctx.time, m, o)?;
                s.entries[k].state = TIMID_ATTACKING;
                return Ok(());
            }
        }
        let ratio = timid_ratio(&e)?;
        self.timid_danger(s, k, ratio, p, &e, ctx)
    }

    fn timid_no_enemy(
        &mut self,
        s: &mut HarassState,
        k: usize,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let e = &mut s.entries[k];
        let flagged = e.flags & 3 != 0;
        if s.flags & HARASS_WAYPOINT != 0 {
            if e.state != TIMID_RETURNING || flagged {
                let o = HarassOrder::Walk { point: s.waypoint };
                return issue(ctx.world, ctx.time, e.monster, o);
            }
        } else if flagged {
            e.state = STATE_IDLE;
            return timid_stand(e.monster, ctx);
        }
        e.state = TIMID_SEEKING;
        e.timer = 0;
        Ok(())
    }

    fn timid_danger(
        &mut self,
        s: &mut HarassState,
        k: usize,
        ratio: i32,
        p: (i32, i32),
        e: &TimidEnemy,
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let entry = &mut s.entries[k];
        let m = entry.monster;
        let flagged = entry.flags & 3 != 0;
        let attack = HarassOrder::Attack {
            point: e.center,
            radius: e.reach,
        };
        if ratio < i32::from(s.attack_ratio) {
            issue(ctx.world, ctx.time, m, attack)?;
            entry.state = TIMID_ATTACKING;
        } else if ratio < i32::from(s.dodge_ratio) {
            let a = arctangent(p.0.wrapping_sub(e.center.0), p.1.wrapping_sub(e.center.1));
            let a = if m & 1 != 0 {
                a.wrapping_add(0x800)
            } else {
                a.wrapping_sub(0x800)
            };
            let o = HarassOrder::Move {
                point: polar(e.center, a, e.reach),
                facing: a.wrapping_add(0x8000),
            };
            issue(ctx.world, ctx.time, m, o)?;
        } else if ratio <= 0x1ff {
            if flagged {
                entry.state = STATE_IDLE;
                timid_stand(m, ctx)?;
            } else {
                entry.state = TIMID_SEEKING;
                entry.timer = 0;
            }
        } else if entry.state == TIMID_PRESSING && !flagged {
            entry.state = TIMID_SEEKING;
            entry.timer = 0;
        } else {
            issue(ctx.world, ctx.time, m, attack)?;
            entry.state = TIMID_PRESSING;
        }
        Ok(())
    }
}

fn timid_stand(m: usize, ctx: &mut MapActionContext) -> Result<(), Unmodelled> {
    log(ctx.time, || format!("S {m} type=4 source={SOURCE}"));
    ctx.world
        .general_command(&[m], 4, SOURCE as i16, ctx.rng, ctx.time)
}

fn timid_fetch(
    w: &EngineWorld,
    s: &HarassState,
    m: usize,
) -> Result<Option<(i16, i16, i32)>, Unmodelled> {
    let mon = &w.monsters[m];
    let class = be_i16(&w.monster_definitions[mon.definition].ai.raw, 0x8a);
    let p = mon.position;
    let r = s.pick_radius;
    let mut candidates = Vec::new();
    let mut identifiers = Vec::new();
    for o in w.iter_filter_objects() {
        if o.identifier == 0 || o.no_shape || o.kind != 4 {
            continue;
        }
        let (dx, dy) = (o.point.0.wrapping_sub(p.0), o.point.1.wrapping_sub(p.1));
        if dx.wrapping_abs().max(dy.wrapping_abs()) >= r
            || dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) >= r.wrapping_mul(r)
        {
            continue;
        }
        let Some(proj) = o.projectile else {
            continue;
        };
        let Some(def) = w.catalog.projectiles.get(&proj) else {
            return Err(Unmodelled::new(
                draw_site::TIMID_PROJECTILE_MISSING,
                format!("projectile {proj} isn't in the catalog"),
            ));
        };
        if be_i16(&def.raw, 0x26) != class {
            continue;
        }
        candidates.push(FetchCandidate {
            index: o.index as i16,
            distance: distance(p, o.point),
            class: be_i16(&def.raw, 0x32),
        });
        identifiers.push((o.index as i16, o.identifier));
    }
    candidates.sort_by_key(|c| c.index);
    Ok(best_timid_fetch(&candidates).and_then(|c| {
        identifiers
            .into_iter()
            .find(|x| x.0 == c.index)
            .map(|(i, id)| (i, id, c.distance))
    }))
}

fn view(w: &EngineWorld, m: usize) -> HarassView {
    let mon = &w.monsters[m];
    HarassView {
        monster: m,
        position: mon.position,
        carrying: w.combat(m).carried != -1,
        vitality: mon.object.vitality as i16,
        max_vitality: mon.object.max_vitality as i16,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub state: i16,
    pub order: Option<HarassOrder>,
    pub object: Option<i16>,
}

impl From<(i16, Option<HarassOrder>)> for Decision {
    fn from((state, order): (i16, Option<HarassOrder>)) -> Self {
        let object = match order {
            Some(HarassOrder::PickUp { index, .. }) => Some(index),
            _ => None,
        };
        Decision {
            state,
            order,
            object,
        }
    }
}

fn harass_decision(
    w: &EngineWorld,
    s: &HarassState,
    k: usize,
) -> Result<Option<Decision>, Unmodelled> {
    let entry = &s.entries[k];
    if !(0..=6).contains(&entry.state) {
        return Ok(None);
    }
    let v = view(w, entry.monster);
    let team = w.monsters[entry.monster].team;
    if let Some(g) = nearest_enemy(w, team, v.position, s.radius) {
        return Ok(with_enemy(s, entry, &v, &Enemy::of(w, g)).map(Decision::from));
    }
    let has_waypoint = s.flags & HARASS_WAYPOINT != 0;
    let back = HarassOrder::Move {
        point: s.waypoint,
        facing: 0x8000,
    };
    let d: Decision = match entry.state {
        STATE_IDLE => {
            if let Some((index, identifier)) = fetch(w, s, entry.monster)? {
                (
                    STATE_FETCHING,
                    Some(HarassOrder::PickUp { index, identifier }),
                )
                    .into()
            } else if has_waypoint && distance(v.position, s.waypoint) > 0xa00 {
                (STATE_RETURNING, Some(back)).into()
            } else {
                (STATE_IDLE, None).into()
            }
        }
        STATE_FETCHING => {
            if !v.carrying {
                let o = entry.object;
                match w
                    .iter_filter_objects()
                    .find(|f| f.index == i32::from(o))
                    .filter(|f| f.identifier != 0 && !f.no_shape && f.kind == 4)
                {
                    Some(f) => (
                        STATE_FETCHING,
                        Some(HarassOrder::PickUp {
                            index: o,
                            identifier: f.identifier,
                        }),
                    )
                        .into(),
                    None => (STATE_IDLE, None).into(),
                }
            } else if !has_waypoint {
                Decision {
                    state: STATE_IDLE,
                    order: None,
                    object: Some(-1),
                }
            } else if distance(v.position, s.waypoint) > 0x9ff {
                (STATE_RETURNING, Some(back)).into()
            } else {
                (STATE_IDLE, None).into()
            }
        }
        STATE_RETURNING => {
            if distance(v.position, s.waypoint) < 0xa00 {
                (STATE_IDLE, None).into()
            } else {
                (STATE_RETURNING, Some(back)).into()
            }
        }
        _ => (STATE_IDLE, None).into(),
    };
    Ok(Some(d))
}

fn fetch(w: &EngineWorld, s: &HarassState, m: usize) -> Result<Option<(i16, i16)>, Unmodelled> {
    let mon = &w.monsters[m];
    let class = be_i16(&w.monster_definitions[mon.definition].ai.raw, 0x8a);
    if class == -1 || w.combat(m).carried != -1 {
        return Ok(None);
    }
    let p = mon.position;
    let r = s.radius;
    let mut candidates = Vec::new();
    let mut identifiers = Vec::new();
    for o in w.iter_filter_objects() {
        if o.identifier == 0 || o.no_shape || o.kind != 4 {
            continue;
        }
        let (dx, dy) = (o.point.0.wrapping_sub(p.0), o.point.1.wrapping_sub(p.1));
        if dx.wrapping_abs().max(dy.wrapping_abs()) >= r
            || dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) >= r.wrapping_mul(r)
        {
            continue;
        }
        let Some(proj) = o.projectile else {
            continue;
        };
        let Some(def) = w.catalog.projectiles.get(&proj) else {
            return Err(Unmodelled::new(
                draw_site::FETCH_PROJECTILE_MISSING,
                format!("projectile {proj} isn't in the catalog"),
            ));
        };
        if be_i16(&def.raw, 0x26) != class {
            continue;
        }
        if s.entries.iter().any(|e| i32::from(e.object) == o.index) {
            continue;
        }
        let d = distance(p, o.point);
        if d >= s.pick_radius {
            continue;
        }
        candidates.push(FetchCandidate {
            index: o.index as i16,
            distance: d,
            class: be_i16(&def.raw, 0x32),
        });
        identifiers.push((o.index as i16, o.identifier));
    }
    candidates.sort_by_key(|c| c.index);
    Ok(best_fetch(&candidates).and_then(|c| identifiers.into_iter().find(|x| x.0 == c.index)))
}

fn issue(w: &mut EngineWorld, time: i32, m: usize, o: HarassOrder) -> Result<(), Unmodelled> {
    log(time, || match o {
        HarassOrder::Attack { point, radius } => format!(
            "A {m} flags=0x0 x={} y={} radius={radius}",
            point.0, point.1
        ),
        HarassOrder::Move { point, facing } => format!(
            "M {m} flags=0x10 form=0 facing={facing} x={} y={} source={SOURCE}",
            point.0, point.1
        ),
        HarassOrder::PickUp { index, identifier } => {
            format!("P {m} object={index} identifier={identifier} source={SOURCE}")
        }
        HarassOrder::Walk { point } => format!(
            "M {m} flags=0x0 form=0 facing=0 x={} y={} source={SOURCE}",
            point.0, point.1
        ),
    });
    match o {
        HarassOrder::Attack { point, radius } => {
            let mut config = AttackConfig::new();
            config.flags &= !1;
            config.point = point;
            config.radius = radius;
            attack_with_monster_indexes(w, &[m], config);
            Ok(())
        }
        HarassOrder::Move { point, facing } => w.movement_command(
            &[m],
            &[point],
            MovementHeader {
                flags: 0x10,
                formation: 0,
                facing,
            },
            SOURCE,
        ),
        HarassOrder::PickUp { index, identifier } => {
            w.handle_pick_up_command(SOURCE, &[m], index, identifier)
        }
        HarassOrder::Walk { point } => w.movement_command(
            &[m],
            &[point],
            MovementHeader {
                flags: 0,
                formation: 0,
                facing: 0,
            },
            SOURCE,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enemy(class: i16) -> Enemy {
        Enemy {
            center: (0, 0),
            radius: 0x200,
            range: 0x300,
            facing: 0,
            class,
        }
    }

    fn state() -> HarassState {
        HarassState {
            radius: 0x5000,
            pick_radius: 0x5000,
            ..HarassState::default()
        }
    }

    fn entry(state: i16) -> HarassEntry {
        HarassEntry {
            monster: 4,
            identifier: 1,
            vitality: 100,
            state,
            object: 0,
            flags: 0,
            timer: 0,
        }
    }

    fn view_at(p: (i32, i32)) -> HarassView {
        HarassView {
            monster: 4,
            position: p,
            carrying: false,
            vitality: 100,
            max_vitality: 256,
        }
    }

    fn decide(state: i16, p: (i32, i32), class: i16) -> (i16, Option<HarassOrder>) {
        with_enemy(&state_with(), &entry(state), &view_at(p), &enemy(class)).unwrap()
    }

    fn state_with() -> HarassState {
        state()
    }

    #[test]
    fn engage_rows() {
        let e = enemy(0);
        assert_eq!(decide(0, (0x600, 0), 0), (3, Some(flank((0x600, 0), &e))));
        assert_eq!(decide(1, (-0x600, 0), 1), (4, Some(attack(&e))));
        assert_eq!(
            decide(2, (-0x600, 0), 0),
            (6, Some(side_step(4, (-0x600, 0), &e)))
        );
        assert_eq!(decide(0, (0x1000, 0), 0), (3, Some(attack(&e))));
        assert_eq!(
            decide(0, (0x6000, 0), 0),
            (6, Some(side_step(4, (0x6000, 0), &e)))
        );
        assert_eq!(
            decide(3, (0x6000, 0), 0),
            (3, Some(side_step(4, (0x6000, 0), &e)))
        );
        assert_eq!(decide(3, (0x1000, 0), 0), (3, Some(attack(&e))));
        assert_eq!(decide(3, (-0x600, 0), 1), (4, Some(attack(&e))));
    }

    #[test]
    fn attack_rows() {
        let e = enemy(0);
        assert_eq!(
            decide(4, (0x600, 0), 0),
            (6, Some(side_step(4, (0x600, 0), &e)))
        );
        assert_eq!(decide(4, (0x600, 0), 1), (4, Some(attack(&e))));
        let mut v = view_at((0x600, 0));
        v.carrying = true;
        assert_eq!(
            with_enemy(&state(), &entry(4), &v, &e),
            Some((4, Some(attack(&e))))
        );
        assert_eq!(decide(5, (0x600, 0), 0), (5, Some(attack(&e))));
    }

    #[test]
    fn dodging_rows() {
        let e = enemy(0);
        assert_eq!(decide(6, (0x300, 0), 0), (6, None));
        let mut v = view_at((0x300, 0));
        v.vitality = 101;
        assert_eq!(
            with_enemy(&state(), &entry(6), &v, &e),
            Some((5, Some(attack(&e))))
        );
        v.vitality = 200;
        assert_eq!(with_enemy(&state(), &entry(6), &v, &e), Some((6, None)));
        assert_eq!(decide(6, (0x1000, 0), 0), (3, Some(attack(&e))));
        assert_eq!(
            decide(6, (0x6000, 0), 0),
            (6, Some(side_step(4, (0x6000, 0), &e)))
        );
        assert_eq!(with_enemy(&state(), &entry(7), &v, &e), None);
    }

    #[test]
    fn helpers() {
        let e = Enemy {
            center: (0x1000, 0x2000),
            radius: 0x100,
            range: 0x100,
            facing: 0x4000,
            class: 0,
        };
        let HarassOrder::Move { point, facing } = flank((0x2000, 0x2000), &e) else {
            panic!()
        };
        assert_eq!(facing, 0x7000);
        assert_eq!(point, polar(e.center, 0x7000, 0xc00));
        let HarassOrder::Move { facing, .. } = flank((0x1000, 0x3000), &e) else {
            panic!()
        };
        assert_eq!(facing, 0x8000);
        let HarassOrder::Move { point, facing } = side_step(3, (0x2000, 0x2000), &e) else {
            panic!()
        };
        assert_eq!(facing, 0x8800);
        assert_eq!(point, polar(e.center, 0x800, 0x100));
        let HarassOrder::Move { facing, .. } = side_step(2, (0x2000, 0x2000), &e) else {
            panic!()
        };
        assert_eq!(facing, 0x7800);
    }

    #[test]
    fn fetch_order() {
        let c = |index, distance, class| FetchCandidate {
            index,
            distance,
            class,
        };
        assert_eq!(best_fetch(&[]), None);
        assert_eq!(best_fetch(&[c(1, 5, 0), c(2, 9, 6)]), Some(c(2, 9, 6)));
        assert_eq!(best_fetch(&[c(1, 5, 8), c(2, 1, 6)]), Some(c(1, 5, 8)));
        assert_eq!(best_fetch(&[c(1, 5, 6), c(2, 1, 6)]), Some(c(2, 1, 6)));
        assert_eq!(best_fetch(&[c(1, 5, 8), c(2, 5, 8)]), Some(c(1, 5, 8)));
        assert_eq!(best_fetch(&[c(1, 5, 0), c(2, 4, 3)]), Some(c(2, 4, 3)));
        assert_eq!(best_fetch(&[c(1, 5, 6), c(2, 1, 3)]), Some(c(1, 5, 6)));
    }

    #[test]
    fn timid_fetch_order() {
        let c = |index, distance, class| FetchCandidate {
            index,
            distance,
            class,
        };
        assert_eq!(best_timid_fetch(&[]), None);
        assert_eq!(
            best_timid_fetch(&[c(1, 1, 6), c(2, 9, 6)]),
            Some(c(2, 9, 6))
        );
        assert_eq!(
            best_timid_fetch(&[c(1, 1, 0), c(2, 9, 8)]),
            Some(c(2, 9, 8))
        );
        assert_eq!(
            best_timid_fetch(&[c(1, 5, 8), c(2, 1, 6)]),
            Some(c(2, 1, 6))
        );
        assert_eq!(
            best_timid_fetch(&[c(1, 5, 8), c(2, 9, 6)]),
            Some(c(1, 5, 8))
        );
        assert_eq!(
            best_timid_fetch(&[c(1, 5, 8), c(2, 5, 8)]),
            Some(c(1, 5, 8))
        );
        assert_eq!(
            best_timid_fetch(&[c(1, 5, 6), c(2, 4, 3)]),
            Some(c(2, 4, 3))
        );
        assert_eq!(
            best_timid_fetch(&[c(1, 5, 6), c(2, 9, 3)]),
            Some(c(1, 5, 6))
        );
    }
}
