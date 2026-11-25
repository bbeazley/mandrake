use std::sync::OnceLock;

use mandrake_tag::MonsterSequenceSlot;
use mandrake_tfl_sim::TflSim;

use super::{UnitObject, UnitPick, ViewMonster, ViewSight, ViewUnit, WorldView};
use crate::minimap::BlipMonster;
use crate::object_history::{self, FRACTION_ONE};
use crate::object_sprites::DrawnObject;
use crate::tfl_game::TflReplay;
use crate::tfl_objects::{drawn_object, transfer, world_render_position};

const OBJECT_MIRRORED: u32 = 0x20000;
const OBJECT_FLOATS: u32 = 0x800;
const OBJECT_FLOATS_IF_20: u32 = 0x1000;

pub(crate) fn drawn_height(z: i32, flags: u32, index: usize, time: u32) -> i32 {
    let mut h = z as i16;
    if flags & OBJECT_FLOATS != 0 || (flags & OBJECT_FLOATS_IF_20 != 0 && flags & 0x20 != 0) {
        let a = (time as i32).wrapping_sub((index as i32).wrapping_mul(7));
        let i = (a.wrapping_mul(0x400) >> 6) & 0x3f0;
        let c = mandrake_tfl_sim::trig::trig().cosine[i as usize];
        h = h.wrapping_add(c >> 9);
    }
    i32::from(h)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ObjectHistory {
    pub(crate) tick: u32,
    objects: Vec<(i16, DrawnObject)>,
}

impl ObjectHistory {
    pub(crate) fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    fn copy(&self, index: i32, identifier: i16) -> Option<&DrawnObject> {
        let i = self
            .objects
            .binary_search_by_key(&index, |(_, o)| o.object)
            .ok()?;
        let (id, o) = &self.objects[i];
        (*id == identifier).then_some(o)
    }
}

pub(crate) fn history(replay: &TflReplay) -> ObjectHistory {
    ObjectHistory {
        tick: replay.tick(),
        objects: live_objects(replay),
    }
}

pub(crate) fn live_objects(replay: &TflReplay) -> Vec<(i16, DrawnObject)> {
    let cells = replay.cells_across();
    let tick = replay.tick();
    replay
        .drawable()
        .filter_map(|(index, o, look)| {
            let world = [o.x, o.y, drawn_height(o.z, o.flags, index, tick)];
            let mut drawn = drawn_object(
                index,
                world,
                cells,
                (o.facing, o.view, i32::from(o.frame.max(0))),
                (o.collection, u8::try_from(o.sequence).ok()?),
                (o.flags, o.flags & OBJECT_MIRRORED != 0, o.scale as u16),
                look,
            );
            drawn.transfer = transfer(o.sub_kind, o.sub_count, o.sub_limit);
            drawn.velocity = o.velocity;
            Some((o.identifier, drawn))
        })
        .collect()
}

pub(crate) fn objects(replay: &TflReplay, fraction: i32) -> Vec<DrawnObject> {
    let live = live_objects(replay);
    interpolate_live(replay, live, fraction)
}

fn interpolate_live(
    replay: &TflReplay,
    live: Vec<(i16, DrawnObject)>,
    fraction: i32,
) -> Vec<DrawnObject> {
    let history = replay.history().filter(|_| fraction < FRACTION_ONE);
    let cells = replay.cells_across();
    live.into_iter()
        .map(
            |(identifier, o)| match history.and_then(|h| h.copy(o.object, identifier)) {
                Some(h) => {
                    let mut d = object_history::interpolated_drawn(h, &o, fraction);
                    d.position = world_render_position(d.world, cells);
                    d
                }
                None => o,
            },
        )
        .collect()
}

fn interpolate_live_slice(
    replay: &TflReplay,
    live: &[(i16, DrawnObject)],
    fraction: i32,
) -> Vec<DrawnObject> {
    let history = replay.history().filter(|_| fraction < FRACTION_ONE);
    let cells = replay.cells_across();
    live.iter()
        .map(
            |(identifier, o)| match history.and_then(|h| h.copy(o.object, *identifier)) {
                Some(h) => {
                    let mut d = object_history::interpolated_drawn(h, o, fraction);
                    d.position = world_render_position(d.world, cells);
                    d
                }
                None => o.clone(),
            },
        )
        .collect()
}

pub(crate) fn pick_view(
    replay: &TflReplay,
    fraction: i32,
    live: Option<&ObjectHistory>,
) -> WorldView {
    let objects = match live.filter(|h| h.tick == replay.tick()) {
        Some(h) => interpolate_live_slice(replay, &h.objects, fraction),
        None => objects(replay, fraction),
    };
    let sim = replay.sim();
    let units = sim.map_or_else(Vec::new, |sim| units(sim, &objects));
    WorldView {
        tick: replay.tick(),
        engine_stopped: replay.stop().is_some(),
        units,
        objects,
        ..WorldView::default()
    }
}

pub(crate) fn units_for_water(sim: &TflSim, objects: &[DrawnObject]) -> Vec<ViewUnit> {
    let mut units: Vec<ViewUnit> = sim
        .monsters
        .iter()
        .filter(|m| m.identifier != 0)
        .filter_map(|m| {
            let index = i32::try_from(m.object).ok()?;
            let i = objects.binary_search_by_key(&index, |o| o.object).ok()?;
            Some(unit_view(m, &objects[i]))
        })
        .collect();
    units.sort_by_key(|u| u.id);
    units
}

pub(crate) fn framed_units(
    sim: &TflSim,
    objects: &[DrawnObject],
    framed_objects: &[usize],
) -> Vec<ViewUnit> {
    if framed_objects.is_empty() {
        return Vec::new();
    }
    let mut units: Vec<ViewUnit> = sim
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, m)| m.identifier != 0 && framed_objects.contains(&m.object))
        .filter_map(|(slot, m)| {
            let index = i32::try_from(m.object).ok()?;
            let i = objects.binary_search_by_key(&index, |o| o.object).ok()?;
            let mut unit = unit_view(m, &objects[i]);
            unit.pick = Some(unit_pick(sim, slot, &objects[i]));
            Some(unit)
        })
        .collect();
    units.sort_by_key(|u| u.id);
    units
}

pub(crate) fn film_sight(sim: &TflSim, observer: bool) -> Option<ViewSight> {
    let s = sight(sim)?;
    Some(if observer {
        ViewSight {
            centre: s.centre,
            ..ViewSight::all_seen(s.cells, &[])
        }
    } else {
        s
    })
}

pub(crate) fn interface_view(sim: &TflSim, tick: u32) -> WorldView {
    WorldView {
        tick,
        blips: blips(sim),
        sight: sight(sim),
        ..WorldView::default()
    }
}

pub(crate) fn blips(sim: &TflSim) -> Vec<BlipMonster> {
    sim.monsters
        .iter()
        .enumerate()
        .filter(|(_, m)| m.identifier != 0 && m.flags & MONSTER_NO_BLIP == 0)
        .filter_map(|(i, m)| {
            let o = sim.objects.get(m.object)?;
            Some(crate::tfl_minimap::tfl_blip(
                i,
                (o.x, o.y),
                m.team,
                m.player,
                m.flags,
                o.flags,
            ))
        })
        .collect()
}

const MONSTER_NO_BLIP: u32 = 0x4000_0000;

#[derive(Clone, Copy)]
enum SightOverride {
    Sim,
    All,
    Off,
}

fn sight_override() -> SightOverride {
    static OVERRIDE: OnceLock<SightOverride> = OnceLock::new();
    *OVERRIDE.get_or_init(|| match std::env::var("M2_TFL_SIGHT").as_deref() {
        Ok("off") => SightOverride::Off,
        Ok("all") => SightOverride::All,
        _ => SightOverride::Sim,
    })
}

pub(crate) fn sight(sim: &TflSim) -> Option<ViewSight> {
    let s = sim.sight();
    let cells = (s.width, s.height);
    match sight_override() {
        SightOverride::Off => None,
        SightOverride::All => {
            let mut all = ViewSight::all_seen(cells, &[]);
            all.centre = s.centre;
            Some(all)
        }
        SightOverride::Sim => Some(ViewSight {
            cells,
            visible: s.visible_cells(),
            explored: s.explored_cells(),
            centre: s.centre,
        }),
    }
}

fn units(sim: &TflSim, objects: &[DrawnObject]) -> Vec<ViewUnit> {
    let mut units: Vec<ViewUnit> = sim
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, m)| m.identifier != 0)
        .filter_map(|(slot, m)| {
            let index = i32::try_from(m.object).ok()?;
            let i = objects.binary_search_by_key(&index, |o| o.object).ok()?;
            let mut unit = unit_view(m, &objects[i]);
            unit.pick = Some(unit_pick(sim, slot, &objects[i]));
            Some(unit)
        })
        .collect();
    units.sort_by_key(|u| u.id);
    units
}

fn unit_pick(sim: &TflSim, slot: usize, o: &DrawnObject) -> UnitPick {
    let m = &sim.monsters[slot];
    let (radius, bottom, top) = sim
        .objects
        .get(m.object)
        .map_or((0, 0, 0), |ob| (ob.radius, ob.bottom, ob.top));
    UnitPick {
        monster: slot as u32,
        world: o.world,
        bounds: mandrake_view::selection::ObjectBounds {
            half_width: radius,
            bottom,
            top,
        },
        vitality: sim.data.peek_def(b"mons", m.definition).map(|d| {
            mandrake_view::selection::tfl::MonsterVitality {
                current: m.vitality,
                maximum: m.max_vitality,
                definition: d.i16(0x128),
            }
        }),
        evaluation: crate::tfl_input::monster_flags(sim, slot) as u8,
        definition: m.definition,
        dying: m.flags & 0x4000_0000 != 0,
    }
}

fn unit_view(m: &mandrake_tfl_sim::monsters::TflMonster, o: &DrawnObject) -> ViewUnit {
    ViewUnit {
        id: o.object as u32,
        archetype: m.unit,
        team: m.team as u8,
        selected: false,
        position: o.position,
        engine_facing: o.facing,
        state_slot: MonsterSequenceSlot(0),
        still: false,
        health: f32::from(m.vitality),
        max_health: f32::from(m.max_vitality),
        object: Some(UnitObject {
            scale: o.scale as u16,
            vitality: m.max_vitality,
            permutation: o.permutation,
        }),
        monster: Some(ViewMonster {
            team: m.team,
            owner: i16::from(m.player),
            flags: 0,
            flags_8: m.flags,
            definition_199: 0,
            mana: None,
        }),
        colour_row: Some((m.unit, i16::from(m.player))),
        pick: None,
    }
}

impl super::pick::PickSim for TflSim {
    fn could_pick_up(&self, selection: &[u32], object: i32) -> Option<i16> {
        let o = usize::try_from(object).ok()?;
        selection
            .iter()
            .all(|&m| self.could_take_object(m as usize, o))
            .then(|| self.objects[o].identifier)
    }
}
