use mandrake_common::FourCC;
use mandrake_math::Fixed16;
use mandrake_tag::MeshHeightMode;

use crate::engine_rng::EngineWorld;
use crate::flight::DormantObject;
use crate::mesh_grid::MeshGrid;
use crate::passability::MeshCells;
use crate::projectiles::{FlightTerrain, isqrt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshEffectDefinition {
    pub tag: FourCC,
    pub flags: u32,
    pub amplitude: i16,
    pub radius: i16,
    pub wavelength: i16,
    pub speed: i16,
    pub initial_distance: i16,
    pub shaking: i16,
    pub collection: Option<FourCC>,
    pub sequence: i16,
}

impl MeshEffectDefinition {
    pub fn from_bytes(tag: FourCC, data: &[u8]) -> Option<Self> {
        if data.len() < 0x24 {
            return None;
        }
        let flags = u32::from_be_bytes(data[0..4].try_into().ok()?);
        let amplitude = i16::from_be_bytes(data[4..6].try_into().ok()?);
        let radius = i16::from_be_bytes(data[6..8].try_into().ok()?);
        let wavelength = i16::from_be_bytes(data[8..10].try_into().ok()?);
        let speed = i16::from_be_bytes(data[10..12].try_into().ok()?);
        let initial_distance = i16::from_be_bytes(data[12..14].try_into().ok()?);
        let shaking = i16::from_be_bytes(data[14..16].try_into().ok()?);
        let col_bytes: [u8; 4] = data[16..20].try_into().ok()?;
        let collection = if col_bytes == [0xff; 4] || col_bytes == [0; 4] {
            None
        } else {
            Some(FourCC::new(col_bytes))
        };
        let sequence = i16::from_be_bytes(data[20..22].try_into().ok()?);
        Some(Self {
            tag,
            flags,
            amplitude,
            radius,
            wavelength,
            speed,
            initial_distance,
            shaking,
            collection,
            sequence,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshEffectDatum {
    pub definition: FourCC,
    pub center_cell_x: i16,
    pub center_cell_y: i16,
    pub center_height: i32,
    pub time: i32,
    pub sequence_frame: u16,
    pub sequence_tick: u16,
}

pub const MAX_MESH_EFFECTS: usize = 16;

pub fn new_mesh_effect(
    catalog: &crate::projectile_groups::ProjectileCatalog,
    effects: &mut Vec<Option<MeshEffectDatum>>,
    effect_tag: FourCC,
    x: i32,
    y: i32,
    center_height: i32,
) {
    if !catalog.mesh_effects.contains_key(&effect_tag) {
        return;
    }
    let datum = MeshEffectDatum {
        definition: effect_tag,
        center_cell_x: ((x + 0x100) >> 9) as i16,
        center_cell_y: ((y + 0x100) >> 9) as i16,
        center_height,
        time: 0,
        sequence_frame: 0,
        sequence_tick: 0,
    };
    if let Some(slot) = effects.iter_mut().find(|e| e.is_none()) {
        *slot = Some(datum);
    } else if effects.len() < MAX_MESH_EFFECTS {
        effects.push(Some(datum));
    }
}

fn mesh_height_from_cells(
    cells: &MeshCells,
    heights: &[(i16, i16)],
    x: i32,
    y: i32,
    mode: MeshHeightMode,
) -> i32 {
    let (w, h) = (cells.width.max(0) as usize, cells.height.max(0) as usize);
    mandrake_tag::get_mesh_height(w, h, x, y, mode, |cx, cy| {
        let i = cy * w + cx;
        let (physical, media) = heights.get(i).copied().unwrap_or((0, 0));
        let flags = cells.flags.get(i).copied().unwrap_or(0);
        (physical, media, flags)
    })
}

struct CellRect {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

impl CellRect {
    fn offset(&self, x: i32, y: i32) -> u32 {
        ((y - self.min_y) * (self.max_x - self.min_x + 1) + (x - self.min_x)) as u32
    }

    fn bucket(&self, positions: impl Iterator<Item = Option<(i32, i32)>>) -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = positions
            .enumerate()
            .filter_map(|(i, p)| {
                let (cx, cy) = p.map(|p| (p.0 >> 9, p.1 >> 9))?;
                ((self.min_x..=self.max_x).contains(&cx) && (self.min_y..=self.max_y).contains(&cy))
                    .then(|| (self.offset(cx, cy), i as u32))
            })
            .collect();
        v.sort_unstable();
        v
    }
}

fn in_cell<'a>(
    list: &'a [(u32, u32)],
    c: &mut usize,
    cur: u32,
) -> impl Iterator<Item = usize> + 'a {
    while list.get(*c).is_some_and(|e| e.0 < cur) {
        *c += 1;
    }
    let from = *c;
    while list.get(*c).is_some_and(|e| e.0 == cur) {
        *c += 1;
    }
    list[from..*c].iter().map(|e| e.1 as usize)
}

#[allow(clippy::too_many_arguments)]
pub fn update_mesh_effects(
    world: &mut EngineWorld,
    mut flight_terrain: Option<&mut FlightTerrain>,
    mut flight_ground: Option<&mut FlightTerrain>,
    mut mesh_grid: Option<&mut MeshGrid>,
    dormant_objects: &mut [DormantObject],
    flights: &mut [Box<crate::projectiles::FlightProjectile>],
    scenery_cylinders: &mut [crate::projectiles::CollisionObject],
    mut units: Option<&mut crate::entities::UnitMap>,
) {
    if world.mesh_effects.is_empty() {
        return;
    }
    let (width, height) = world.mesh_cells;
    if width <= 0 || height <= 0 {
        return;
    }

    let mut effects = std::mem::take(&mut world.mesh_effects);

    for slot in &mut effects {
        let Some(effect) = slot.as_mut() else {
            continue;
        };
        if !update_mesh_effect(
            world,
            effect,
            flight_terrain.as_deref_mut(),
            flight_ground.as_deref_mut(),
            mesh_grid.as_deref_mut(),
            dormant_objects,
            flights,
            scenery_cylinders,
            units.as_deref_mut(),
        ) {
            *slot = None;
        }
    }
    while effects.last().is_some_and(Option::is_none) {
        effects.pop();
    }
    world.mesh_effects = effects;
}

#[allow(clippy::too_many_arguments)]
fn update_mesh_effect(
    world: &mut EngineWorld,
    effect: &mut MeshEffectDatum,
    mut flight_terrain: Option<&mut FlightTerrain>,
    mut flight_ground: Option<&mut FlightTerrain>,
    mut mesh_grid: Option<&mut MeshGrid>,
    dormant_objects: &mut [DormantObject],
    flights: &mut [Box<crate::projectiles::FlightProjectile>],
    scenery_cylinders: &mut [crate::projectiles::CollisionObject],
    mut units: Option<&mut crate::entities::UnitMap>,
) -> bool {
    let (width, height) = world.mesh_cells;
    let Some(def) = world.catalog.mesh_effects.get(&effect.definition).cloned() else {
        return false;
    };
    let radius = i32::from(def.radius);
    if radius <= 0 {
        return false;
    }

    let speed = i32::from(def.speed);
    let current_dist = i32::from(def.initial_distance) + speed * effect.time;
    if current_dist > radius {
        return false;
    }

    let (prev_dist, prev_amp) = if effect.time == 0 {
        (0, 0)
    } else {
        let prev_d = current_dist - speed;
        let prev_ratio = (prev_d << 16) / radius;
        let prev_factor = (0x10000 - prev_ratio).max(0);
        let prev_a = (prev_factor * i32::from(def.amplitude)) >> 16;
        (prev_d, prev_a)
    };

    let curr_ratio = (current_dist << 16) / radius;
    let curr_factor = (0x10000 - curr_ratio).max(0);
    let curr_amp = (curr_factor * i32::from(def.amplitude)) >> 16;

    let cell_radius = (radius + 0x1ff) >> 9;
    let min_x = (i32::from(effect.center_cell_x) - cell_radius).clamp(0, width - 1);
    let max_x = (i32::from(effect.center_cell_x) + cell_radius).clamp(0, width - 1);
    let min_y = (i32::from(effect.center_cell_y) - cell_radius).clamp(0, height - 1);
    let max_y = (i32::from(effect.center_cell_y) + cell_radius).clamp(0, height - 1);

    let rect = CellRect {
        min_x,
        max_x,
        min_y,
        max_y,
    };
    let mons = rect.bucket(world.monsters.iter().map(|m| Some(m.position)));
    let dormant = rect.bucket(
        dormant_objects
            .iter()
            .map(|d| Some((d.position[0], d.position[1]))),
    );
    let marker = rect.bucket(
        world
            .projectiles
            .iter()
            .map(|p| (p.object.kind == 4).then_some(p.position)),
    );
    let scenery = rect.bucket(world.scenery.iter().map(|sc| Some(sc.position)));
    let records = rect.bucket(
        flights
            .iter()
            .map(|p| p.shape.is_some().then_some((p.object.x, p.object.y))),
    );
    let fades = rect.bucket(world.fade_records.iter().map(|r| Some(r.position)));
    let mut cursors = [0usize; 6];

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let cur = rect.offset(x, y);
            let [c_mons, c_dormant, c_marker, c_scenery, c_records, c_fades] = &mut cursors;
            let mons_here = in_cell(&mons, c_mons, cur);
            let dormant_here = in_cell(&dormant, c_dormant, cur);
            let marker_here = in_cell(&marker, c_marker, cur);
            let scenery_here = in_cell(&scenery, c_scenery, cur);
            let records_here = in_cell(&records, c_records, cur);
            let fades_here = in_cell(&fades, c_fades, cur);
            let prev_x = (x - 1).clamp(0, width - 1);
            let prev_y = (y - 1).clamp(0, height - 1);
            let model_at = |cx: i32, cy: i32| -> i16 {
                let idx = world.cells.cell_index(cx, cy);
                world.cells.models.get(idx).copied().unwrap_or(-1)
            };
            if model_at(x, y) != -1
                || model_at(prev_x, y) != -1
                || model_at(x, prev_y) != -1
                || model_at(prev_x, prev_y) != -1
            {
                continue;
            }

            let dx = (x - i32::from(effect.center_cell_x)) << 9;
            let dy = (y - i32::from(effect.center_cell_y)) << 9;
            let dist = isqrt((dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))) as u32);

            let def_08 = i32::from(def.wavelength);

            let d_prev = prev_dist - dist;
            let falloff_prev = (((d_prev.wrapping_mul(d_prev)) >> 9).wrapping_mul(def_08)) >> 8;
            let h_prev = (prev_amp - falloff_prev).max(0);

            let d_curr = current_dist - dist;
            let falloff_curr = (((d_curr.wrapping_mul(d_curr)) >> 9).wrapping_mul(def_08)) >> 8;
            let h_curr = (curr_amp - falloff_curr).max(0);

            let delta = if (def.flags & 1) == 0 {
                h_curr - h_prev
            } else {
                h_prev - h_curr
            };

            let delta_i16 = delta as i16;

            let cell_idx = (y * width + x) as usize;
            let cell_flags = world.cells.flags.get(cell_idx).copied().unwrap_or(0);
            let is_media = (cell_flags & 0x1800) != 0;

            if is_media {
                if let Some(h) = world.heights.get_mut(cell_idx) {
                    h.1 = h.1.wrapping_add(delta_i16);
                }
            } else if let Some(h) = world.heights.get_mut(cell_idx) {
                h.0 = h.0.wrapping_add(delta_i16);
            }

            if let Some(t) = flight_terrain.as_deref_mut() {
                t.update_cell_height(x, y, delta_i16);
            }
            if let Some(g) = flight_ground.as_deref_mut() {
                g.update_cell_height(x, y, delta_i16);
            }
            if let Some(g) = mesh_grid.as_deref_mut() {
                g.update_cell_height(x as usize, y as usize, delta_i16, is_media);
            }

            for i in mons_here {
                let m = &mut world.monsters[i];
                let flags = m.object.flags | if m.object.no_shape { 0x200 } else { 0 };
                if (flags & 0x210) == 0x10 {
                    if world
                        .monster_definitions
                        .get(m.definition)
                        .is_some_and(|d| d.flags & 8 != 0)
                    {
                        continue;
                    }
                    let mode = if is_media && (m.object.flags & 0x1000) != 0 {
                        MeshHeightMode::Media
                    } else {
                        MeshHeightMode::Physical
                    };
                    m.object.z = mesh_height_from_cells(
                        &world.cells,
                        &world.heights,
                        m.position.0,
                        m.position.1,
                        mode,
                    );
                    if let Some(u_map) = units.as_mut()
                        && let Some(id) = m.unit_key()
                        && let Some(u) = u_map.get_mut(&id)
                    {
                        u.position.y = Fixed16::from_raw(m.object.z.wrapping_mul(128));
                    }
                }
            }

            for i in dormant_here {
                let d = &mut dormant_objects[i];
                let flags = d.object.flags | if d.object.no_shape { 0x200 } else { 0 };
                if (flags & 0x210) == 0x10 {
                    let mode = if is_media && (d.object.flags & 0x1000) != 0 {
                        MeshHeightMode::Media
                    } else {
                        MeshHeightMode::Physical
                    };
                    d.position[2] = mesh_height_from_cells(
                        &world.cells,
                        &world.heights,
                        d.position[0],
                        d.position[1],
                        mode,
                    );
                    d.object.z = d.position[2];
                }
            }

            for i in marker_here {
                let p = &mut world.projectiles[i];
                let flags = p.object.flags | if p.object.no_shape { 0x200 } else { 0 };
                if (flags & 0x210) == 0x10 {
                    let mode = if is_media && (p.object.flags & 0x1000) != 0 {
                        MeshHeightMode::Media
                    } else {
                        MeshHeightMode::Physical
                    };
                    p.object.z = mesh_height_from_cells(
                        &world.cells,
                        &world.heights,
                        p.position.0,
                        p.position.1,
                        mode,
                    );
                }
            }

            for i in scenery_here {
                let sc = &mut world.scenery[i];
                let flags = sc.object.flags | if sc.object.no_shape { 0x200 } else { 0 };
                if (flags & 0x210) == 0x10 {
                    let mode = if is_media && (sc.object.flags & 0x1000) != 0 {
                        MeshHeightMode::Media
                    } else {
                        MeshHeightMode::Physical
                    };
                    sc.object.z = mesh_height_from_cells(
                        &world.cells,
                        &world.heights,
                        sc.position.0,
                        sc.position.1,
                        mode,
                    );
                    let index = sc.object.index;
                    for c in scenery_cylinders.iter_mut().filter(|c| c.object == index) {
                        c.z = sc.object.z;
                    }
                }
            }

            for i in records_here {
                let p = &mut flights[i];
                let Some(shape) = p.shape.as_mut() else {
                    continue;
                };
                let flags = (shape.flags & !0x1f0)
                    | u32::from(p.object.shape_flags & 0x1f0)
                    | if shape.no_shape { 0x200 } else { 0 };
                if (flags & 0x210) == 0x10 {
                    let mode = if is_media && p.object.flag_1000 {
                        MeshHeightMode::Media
                    } else {
                        MeshHeightMode::Physical
                    };
                    p.object.z = mesh_height_from_cells(
                        &world.cells,
                        &world.heights,
                        p.object.x,
                        p.object.y,
                        mode,
                    );
                    shape.z = p.object.z;
                }
            }

            for i in fades_here {
                let r = &mut world.fade_records[i];
                let flags = r.object.flags | if r.object.no_shape { 0x200 } else { 0 };
                if (flags & 0x210) == 0x10 {
                    let mode = if is_media && (r.object.flags & 0x1000) != 0 {
                        MeshHeightMode::Media
                    } else {
                        MeshHeightMode::Physical
                    };
                    r.object.z = mesh_height_from_cells(
                        &world.cells,
                        &world.heights,
                        r.position.0,
                        r.position.1,
                        mode,
                    );
                }
            }
        }
    }

    effect.time += 1;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_mesh_effect_takes_the_lowest_free_datum() {
        let tag = FourCC::new(*b"wiex");
        let mut catalog = crate::projectile_groups::ProjectileCatalog::default();
        catalog.mesh_effects.insert(
            tag,
            MeshEffectDefinition {
                tag,
                flags: 0,
                amplitude: 0,
                radius: 0,
                wavelength: 0,
                speed: 0,
                initial_distance: 0,
                shaking: -1,
                collection: None,
                sequence: -1,
            },
        );
        let mut effects = Vec::new();
        for i in 0..3 {
            new_mesh_effect(&catalog, &mut effects, tag, i << 9, 0, 0);
        }
        effects[0] = None;
        new_mesh_effect(&catalog, &mut effects, tag, 7 << 9, 0, 0);
        let cells: Vec<_> = effects
            .iter()
            .map(|e| e.as_ref().map(|e| e.center_cell_x))
            .collect();
        assert_eq!(cells, [Some(7), Some(1), Some(2)]);
        for _ in 0..20 {
            new_mesh_effect(&catalog, &mut effects, tag, 0, 0, 0);
        }
        assert_eq!(effects.len(), MAX_MESH_EFFECTS);
        effects.clear();
        new_mesh_effect(&catalog, &mut effects, FourCC::new(*b"none"), 0, 0, 0);
        assert!(effects.is_empty());
    }
}
