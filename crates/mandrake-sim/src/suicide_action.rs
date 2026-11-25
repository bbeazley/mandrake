use crate::draw_site;
use crate::engine_move::{arctangent, distance2d, trig_tables};
use crate::engine_rng::{EngineWorld, MovementHeader, Unmodelled};
use crate::map_actions::{
    FLAG_SUCCEEDED, MapActionContext, MapActions, TYPE_ACTIONS, TYPE_DISTANCE, TYPE_FIXED,
    TYPE_INTEGER, TYPE_MONSTERS,
};
use crate::spatial_groups::{SpatialGroup, SpatialSupergroup, supergroups};

thread_local! {
    pub static SUICIDE_LOG: std::cell::RefCell<Option<Vec<(i32, String)>>> =
        const { std::cell::RefCell::new(None) };
}

fn log(time: i32, f: impl FnOnce() -> String) {
    SUICIDE_LOG.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            v.push((time, f()));
        }
    });
}

pub const SUICIDE_INITIALISED: u32 = 1;
pub const SUICIDE_DUMB: u32 = 2;
const MAX_SUBJECTS: usize = 0x40;
const SOURCE: i32 = -2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuicideEntry {
    pub monster: i16,
    pub identifier: i16,
    pub flank: i16,
    pub timer: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuicideState {
    pub flags: u32,
    pub action: i16,
    pub suin: i16,
    pub radius: i32,
    pub fedu: i32,
    pub deth: i32,
    pub mipt: i32,
    pub dith: i32,
    pub dera: i32,
    pub entries: Vec<SuicideEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    Flank,
    Ally,
    Charge,
    Burst,
}

impl Mode {
    fn code(self) -> i16 {
        match self {
            Mode::Idle => -1,
            Mode::Flank => 0,
            Mode::Ally => 1,
            Mode::Charge => 2,
            Mode::Burst => 3,
        }
    }
}

fn trig(a: i32) -> (i32, i32) {
    let t = trig_tables();
    let k = (a.wrapping_add(0x20) >> 6) as usize & 0x3ff;
    (i32::from(t.cosine[k]), i32::from(t.sine[k]))
}

type Nearest<'a> = Option<(&'a SpatialSupergroup, i32)>;

fn second_level<'a>(
    groups: &'a [SpatialGroup],
    alliances: &crate::ally::AllianceView,
    team: i16,
    p: (i32, i32),
    s: &SuicideState,
) -> (Nearest<'a>, Nearest<'a>) {
    let mut enemy: Nearest = None;
    let mut ally: Nearest = None;
    for g in supergroups(groups) {
        let dx = p.0.wrapping_sub(g.center.0).wrapping_abs();
        let dy = p.1.wrapping_sub(g.center.1).wrapping_abs();
        let d = if dx > dy {
            (dy >> 1).wrapping_add(dx)
        } else {
            (dx >> 1).wrapping_add(dy)
        }
        .wrapping_sub(i32::from(g.radius));
        if !alliances.teams_are_allied(team, g.team) {
            let take = match enemy {
                None => true,
                Some((b, bd)) => {
                    d < bd
                        || (i32::from(g.strength) > s.mipt >> 8 && s.mipt > i32::from(b.strength))
                        || (g.strength > b.strength && d.wrapping_sub(s.dith) < bd)
                }
            };
            if take {
                enemy = Some((g, d));
            }
        } else if (dx | dy) != 0 && ally.is_none_or(|(_, ad)| d < ad) {
            ally = Some((g, d));
        }
    }
    (enemy, ally)
}

fn class_1_group(w: &EngineWorld, team: i16, p: (i32, i32)) -> Option<(&SpatialGroup, i32)> {
    let mut best: Option<(&SpatialGroup, i32)> = None;
    for g in &w.spatial_groups {
        if w.teams_are_allied(team, g.team) || w.monster_definitions[g.definition].field_1b6 != 1 {
            continue;
        }
        let d = distance2d(p.0.wrapping_sub(g.center.0), p.1.wrapping_sub(g.center.1))
            .wrapping_sub(i32::from(g.radius) + i32::from(g.range));
        if d < 0x4000 && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((g, d));
        }
    }
    best
}

fn enemies_near(w: &EngineWorld, team: i16, p: (i32, i32)) -> i16 {
    let clamp = |v: i32, size: i32| if v < 0 { 0 } else { v.min(size - 5) };
    let x0 = clamp((p.0 >> 9) - 2, w.cells.width);
    let y0 = clamp((p.1 >> 9) - 2, w.cells.height);
    let mut n: i16 = 0;
    for m in w.monsters_in_cell_box(x0, y0, 5, 5) {
        let mon = &w.monsters[m];
        if !mon.object.no_shape && !w.teams_are_allied(team, mon.team) {
            n = n.wrapping_add(1);
        }
    }
    n
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuicideOrder {
    Move { point: (i32, i32) },
    Burst,
}

fn decide(
    w: &EngineWorld,
    s: &SuicideState,
    entry: &SuicideEntry,
    team: i16,
    target: &mut Option<(i32, i32)>,
    reach: &mut Option<i32>,
) -> Result<(Mode, Option<SuicideOrder>, i16, i16), Unmodelled> {
    let m = entry.monster as usize;
    let mon = &w.monsters[m];
    let p = mon.position;
    let (enemy, ally) = second_level(&w.spatial_groups, &w.alliance_view(), team, p, s);
    let class_1 = class_1_group(w, team, p);
    let mut flank = entry.flank;
    let ally_near = ally.filter(|&(_, ad)| s.dera > ad);
    let mut mode = Mode::Idle;
    let mut coming: Nearest = None;
    if s.flags & SUICIDE_DUMB != 0 {
        if let Some((e, ed)) = enemy {
            *target = Some(e.center);
            mode = Mode::Charge;
            coming = Some((e, ed));
        } else if let Some((a, _)) = ally_near {
            *target = Some(a.center);
            *reach = Some(i32::from(a.radius).wrapping_add(s.dera));
            mode = Mode::Ally;
        }
    } else {
        let at_enemy = match (ally_near, enemy) {
            (Some((_, ad)), Some((_, ed))) if ad >= ed => true,
            (Some((a, _)), e) => {
                *target = Some(a.center);
                *reach = Some(i32::from(a.radius).wrapping_add(s.dera));
                mode = Mode::Ally;
                coming = e;
                false
            }
            (None, Some(_)) => true,
            (None, None) => false,
        };
        if at_enemy {
            let (e, ed) = enemy.expect("an enemy");
            mode = Mode::Charge;
            *target = Some(e.center);
            if let Some((g, gd)) = class_1
                && gd < 0
                && i32::from(e.radius).wrapping_add(ed) > s.dera
            {
                let (cos, sin) = trig(i32::from(g.facing));
                let dot =
                    p.0.wrapping_sub(g.center.0)
                        .wrapping_mul(cos)
                        .wrapping_add(sin.wrapping_mul(p.1.wrapping_sub(g.center.1)));
                if dot > 0 {
                    flank = s.fedu as i16;
                }
                if flank != 0 {
                    *target = Some(g.center);
                    *reach = Some(i32::from(g.radius) + i32::from(g.range));
                    mode = Mode::Flank;
                }
            }
            coming = Some((e, ed));
        }
    }
    if let Some((e, ed)) = coming {
        let dot = e
            .center
            .0
            .wrapping_sub(p.0)
            .wrapping_mul(i32::from(e.velocity.0))
            .wrapping_add(
                e.center
                    .1
                    .wrapping_sub(p.1)
                    .wrapping_mul(i32::from(e.velocity.1)),
            );
        if dot > 0 && i32::from(e.radius).wrapping_add(ed) < s.dera.wrapping_sub(0x200) {
            mode = Mode::Burst;
        }
    }
    let n = enemies_near(w, team, p);
    let walk = |target: &Option<(i32, i32)>, at: u32| -> Result<SuicideOrder, Unmodelled> {
        target
            .map(|point| SuicideOrder::Move { point })
            .ok_or_else(|| {
                Unmodelled::new(
                    at,
                    format!("suic: monster {m} walks to a target no subject has set"),
                )
            })
    };
    if i32::from(n) < s.deth && enemy.is_none_or(|(e, _)| e.members > n) && mode != Mode::Burst {
        let order = match mode {
            Mode::Charge => Some(walk(target, draw_site::SUICIDE_CHARGE_NO_TARGET)?),
            Mode::Idle => None,
            _ => {
                let r = reach.ok_or_else(|| {
                    Unmodelled::new(
                        draw_site::SUICIDE_NO_REACH,
                        format!("suic: monster {m} without a reach"),
                    )
                })?;
                if r != 0 {
                    let div = r.wrapping_mul(0x648) >> 8;
                    if div == 0 {
                        return Err(Unmodelled::new(
                            draw_site::SUICIDE_DIVISION_BY_ZERO,
                            "suic: a division by zero",
                        ));
                    }
                    let mut off = (0x400_0000i32 / div).min(0x1000);
                    if entry.monster & 1 != 0 {
                        off = off.wrapping_neg();
                    }
                    let t = target.ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::SUICIDE_NO_TARGET,
                            format!("suic: monster {m} without a target"),
                        )
                    })?;
                    if distance2d(p.0.wrapping_sub(t.0), p.1.wrapping_sub(t.1)) < s.dera {
                        off = 0;
                    }
                    let a = i32::from(arctangent(p.0.wrapping_sub(t.0), p.1.wrapping_sub(t.1)));
                    let (cos, sin) = trig(off.wrapping_add(a));
                    *target = Some((
                        t.0.wrapping_add(cos.wrapping_mul(r) >> 14),
                        t.1.wrapping_add(sin.wrapping_mul(r) >> 14),
                    ));
                }
                Some(walk(target, draw_site::SUICIDE_CHARGE_NO_TARGET)?)
            }
        };
        return Ok((mode, order, flank, n));
    }
    let o = &mon.object;
    let c = w.cells.cell_index(p.0 >> 9, p.1 >> 9);
    let media = w.heights.get(c).map_or(0, |h| i32::from(h.1));
    let top = || {
        o.bounds(w.collections.get(&o.collection).map_or(&[], |v| v))
            .map_or(0, |b| b.2)
    };
    let order = if o.flags & 0x80 != 0
        || (o.flags & 0x40 != 0 && i32::from(top() >> 1).wrapping_add(o.z) < media)
    {
        walk(target, draw_site::SUICIDE_MEDIA_NO_TARGET)?
    } else {
        SuicideOrder::Burst
    };
    Ok((mode, Some(order), flank, n))
}

impl MapActions {
    pub fn suicide_state(&self, i: usize) -> Option<&SuicideState> {
        self.suicide_glo.get(&i)
    }

    fn suicide_initial(&self, i: usize, ctx: &mut MapActionContext) -> SuicideState {
        let mut s = SuicideState {
            action: i as i16,
            radius: self.long(i, b"radi", TYPE_DISTANCE).unwrap_or(0x6400),
            fedu: self.long(i, b"fedu", TYPE_INTEGER).unwrap_or(0x14),
            deth: self.long(i, b"deth", TYPE_INTEGER).unwrap_or(3),
            dith: self.long(i, b"dith", TYPE_DISTANCE).unwrap_or(0xc00),
            dera: self.long(i, b"dera", TYPE_DISTANCE).unwrap_or(0x1000),
            mipt: self.long(i, b"mipt", TYPE_FIXED).unwrap_or(0x10000),
            ..SuicideState::default()
        };
        let suin = self.indexed(i, b"suin", TYPE_ACTIONS, 0).unwrap_or(-1);
        s.suin = self
            .index_of(i32::from(suin as i16))
            .map_or(-1, |j| j as i16);
        let dela = self.long(i, b"dela", TYPE_INTEGER).unwrap_or(0);
        if self.flag(i, b"dumb") {
            s.flags |= SUICIDE_DUMB;
        }
        for id in self.combined(i, b"subj", TYPE_MONSTERS, MAX_SUBJECTS) {
            let timer = if dela > 0 {
                ((u32::from(ctx.rng.draw(draw_site::SUICIDE_DELAY)) as i32).wrapping_mul(dela)
                    as u32
                    >> 16) as i16
            } else {
                0
            };
            s.entries.push(SuicideEntry {
                monster: -1,
                identifier: id as i16,
                flank: 0,
                timer,
            });
        }
        s
    }

    pub(crate) fn suicide_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        log(ctx.time, || {
            format!("E {i} id={}", self.actions[i].identifier)
        });
        let r = self.suicide_inner(i, ctx)?;
        log(ctx.time, || format!("R {i} result={r}"));
        Ok(r)
    }

    fn suicide_inner(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let mut s = match self.suicide_glo.get(&i) {
            Some(s) => s.clone(),
            None => self.suicide_initial(i, ctx),
        };
        let mut team: i16 = -1;
        let mut k = 0;
        while k < s.entries.len() {
            let e = s.entries[k];
            if e.timer != 0 {
                k += 1;
                continue;
            }
            let mut monster = e.monster;
            if monster == -1 {
                monster = ctx
                    .world
                    .make_monster_visible(e.identifier, -1, ctx.rng, ctx.time)?
                    .map_or(-1, |m| m as i16);
                s.entries[k].monster = monster;
            }
            if monster != -1 {
                let mon = &ctx.world.monsters[monster as usize];
                if mon.identifier != 0 && mon.identifier == e.identifier {
                    team = i16::from(mon.team as i8);
                    k += 1;
                    continue;
                }
                log(ctx.time, || format!("K {i} {k}"));
                self.harass_casualty(i, ctx)?;
            }
            s.entries.remove(k);
        }
        if s.entries.is_empty() {
            return Ok(4);
        }
        let running = s.suin == -1 || self.actions[s.suin as usize].flags & FLAG_SUCCEEDED != 0;
        let mut target: Option<(i32, i32)> = None;
        let mut reach: Option<i32> = None;
        for k in 0..s.entries.len() {
            let e = s.entries[k];
            if e.timer != 0 {
                if running {
                    s.entries[k].timer = s.entries[k].timer.wrapping_sub(1);
                }
                continue;
            }
            let (mode, order, flank, n) = decide(ctx.world, &s, &e, team, &mut target, &mut reach)?;
            let m = e.monster as usize;
            log(ctx.time, || {
                format!("C {i} {k} {m} mode={} n={n}", mode.code())
            });
            match order {
                Some(SuicideOrder::Move { point }) => {
                    log(ctx.time, || {
                        format!("M {m} x={} y={} source={SOURCE}", point.0, point.1)
                    });
                    ctx.world.movement_command(
                        &[m],
                        &[point],
                        MovementHeader {
                            flags: 0,
                            formation: 0,
                            facing: 0,
                        },
                        SOURCE,
                    )?;
                }
                Some(SuicideOrder::Burst) => {
                    log(ctx.time, || format!("B {m} kind=3 source={SOURCE}"));
                    ctx.world
                        .general_command(&[m], 3, SOURCE as i16, ctx.rng, ctx.time)?;
                }
                None => {}
            }
            s.entries[k].flank = if flank != 0 { flank.wrapping_sub(1) } else { 0 };
        }
        s.flags |= SUICIDE_INITIALISED;
        log(ctx.time, || {
            let es: Vec<String> = s
                .entries
                .iter()
                .enumerate()
                .map(|(k, e)| format!("{k}:{}:{}:{}:{}", e.monster, e.identifier, e.flank, e.timer))
                .collect();
            format!(
                "G {} count={} flags={:#x} suin={} | {}",
                s.action,
                s.entries.len(),
                s.flags,
                s.suin,
                es.join(" ")
            )
        });
        self.suicide_glo.insert(i, s);
        Ok(0)
    }
}
