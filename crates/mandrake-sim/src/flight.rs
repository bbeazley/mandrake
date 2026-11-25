use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_math::{Fixed16, Vec3Fixed};
use mandrake_tag::{MeshTag, SequenceHeader, SequenceWorldBounds};

use crate::Simulation;
use crate::combat::{CombatEvent, frequency_corrected};
use crate::draw_site;
use crate::engine_move::arctangent;
use crate::engine_rng::Unmodelled;
use crate::projectiles::{
    COLLISION_CHANCE_VERSION_CUT, CollisionObject, FREQUENCY_DRAW_SITE, FlightProjectile,
    FlightStep, FlightTerrain, kind,
};

#[derive(Clone, Copy)]
struct DetonateProjectile {
    flags: u32,
    impact_frequency: u16,
    stain_sequence: i16,
    collection: Option<FourCC>,
    damage: mandrake_tag::projectile::DamageDefinition,
    raw_damage_flags: u16,
    detonation_group: Option<FourCC>,
    group_scale: i16,
}

impl DetonateProjectile {
    fn of(p: &mandrake_tag::projectile::ProjectileDefinition) -> Self {
        Self {
            flags: p.flags,
            impact_frequency: p.impact_frequency,
            stain_sequence: p.sequences[1],
            collection: p.collection,
            damage: p.effective_damage(),
            raw_damage_flags: p.damage.flags,
            detonation_group: p.detonation_group,
            group_scale: p.group_scale(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FlightState {
    pub replace_depth: i32,
    pub replace_overflow: bool,
    pub terrain: Option<FlightTerrain>,
    pub flights: crate::record_list::RecordList<Box<FlightProjectile>>,
    pub reports: Vec<(u32, Unmodelled)>,
    pub scenery: Vec<CollisionObject>,
    pub(crate) detonating: Option<crate::explosions::ExplosionInfo>,
    pub(crate) detonating_records: Vec<crate::explosions::ExplosionSource>,
    pub(crate) detonate_at: Option<([i32; 3], i32)>,
    pub(crate) area_scans: u32,
    pub(crate) relinked: Vec<(i32, u64)>,
    pub scenery_damage: std::collections::BTreeMap<i32, crate::explosions::SceneryDamage>,
    pub ground: Option<FlightTerrain>,
    pub dormant: Vec<DormantObject>,
    pub stains: Vec<Stain>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stain {
    pub collection: FourCC,
    pub sequence: i16,
    pub point: [i32; 3],
    pub random: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DormantObject {
    pub definition: FourCC,
    pub object: crate::engine_rng::EngineObject,
    pub position: [i32; 3],
    pub facing: u16,
    pub record_36: i16,
    pub flags_3d: u8,
    pub elevation: u16,
    pub cell_link: u64,
}

pub(crate) fn sleep_projectile(
    p: &mut FlightProjectile,
    flags: Option<&crate::flags::FlagData>,
) -> Option<DormantObject> {
    let mut o = p.shape.take()?;
    o.kind = 4;
    if o.flags & 0x10_0000 != 0
        && let Some(flag) = flags.and_then(|f| f.flag_of_object(o.index))
    {
        o.flags |= 0x400;
        o.kind = crate::flags::OBJECT_KIND_FLAG;
        o.owner = flag;
    }
    o.flags |= 0x110;
    o.velocity = [p.object.vx, p.object.vy, p.object.vz];
    Some(DormantObject {
        definition: p.definition,
        object: o,
        position: [p.object.x, p.object.y, p.object.z],
        facing: p.facing,
        record_36: p.record_36,
        flags_3d: (p.record_flags & 0xe0) as u8,
        elevation: p.elevation,
        cell_link: p.cell_link,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectileView {
    pub kind: crate::views::ObjectKind,
    pub id: u32,
    pub record: i32,
    pub object: i32,
    pub definition: FourCC,
    pub owner: u32,
    pub target: Option<u32>,
    pub world: [i32; 3],
    pub position: Vec3Fixed,
    pub velocity: [i16; 3],
    pub facing: u16,
    pub elevation: u16,
    pub heading: u16,
    pub pitch: u16,
    pub launched_tick: u32,
    pub collection: FourCC,
    pub sequence: u8,
    pub frame: i8,
    pub view: u8,
    pub scale: i32,
    pub flags: u32,
    pub hidden: bool,
    pub mirror_x: bool,
    pub mirror_y: bool,
    pub permutation: i16,
    pub shading_slot: u8,
    pub core: Option<FourCC>,
    pub transfer: (u8, u8, u8),
}

#[derive(Clone, Default)]
pub struct ObjectHistoryMap {
    at: Vec<u32>,
    entries: Vec<(i32, ObjectHistory)>,
    order: Vec<(i32, u32)>,
    first_monster: crate::entities::UnitIdMap<usize>,
}

impl ObjectHistoryMap {
    const DENSE: usize = 1 << 16;

    fn dense(index: i32) -> Option<usize> {
        usize::try_from(index).ok().filter(|&i| i < Self::DENSE)
    }

    fn position(&self, index: i32) -> Option<usize> {
        match Self::dense(index) {
            Some(i) => match self.at.get(i) {
                None | Some(0) => None,
                Some(&k) => Some(k as usize - 1),
            },
            None => self.entries.iter().position(|e| e.0 == index),
        }
    }

    pub fn get(&self, index: &i32) -> Option<&ObjectHistory> {
        self.position(*index).map(|k| &self.entries[k].1)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn remove(&mut self, index: &i32) -> Option<ObjectHistory> {
        let k = self.position(*index)?;
        if let Some(i) = Self::dense(*index) {
            self.at[i] = 0;
        }
        let gone = self.entries.swap_remove(k);
        if let Some(moved) = self.entries.get(k)
            && let Some(i) = Self::dense(moved.0)
        {
            self.at[i] = k as u32 + 1;
        }
        Some(gone.1)
    }

    pub fn clear(&mut self) {
        for e in &self.entries {
            if let Some(i) = Self::dense(e.0) {
                self.at[i] = 0;
            }
        }
        self.entries.clear();
    }

    fn put(&mut self, index: i32, replace: bool, h: ObjectHistory) {
        let found = match Self::dense(index) {
            Some(i) => {
                if i >= self.at.len() {
                    self.at.resize((i + 1).next_power_of_two(), 0);
                }
                match self.at[i] {
                    0 => {
                        self.at[i] = self.entries.len() as u32 + 1;
                        None
                    }
                    k => Some(k as usize - 1),
                }
            }
            None => self.entries.iter().position(|e| e.0 == index),
        };
        match found {
            Some(k) if replace => self.entries[k].1 = h,
            Some(_) => {}
            None => self.entries.push((index, h)),
        }
    }

    fn sorted(&self) -> Vec<(i32, &ObjectHistory)> {
        let mut v: Vec<(i32, &ObjectHistory)> = self.entries.iter().map(|e| (e.0, &e.1)).collect();
        v.sort_unstable_by_key(|e| e.0);
        v
    }
}

impl std::fmt::Debug for ObjectHistoryMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.sorted()).finish()
    }
}

impl PartialEq for ObjectHistoryMap {
    fn eq(&self, other: &Self) -> bool {
        self.sorted() == other.sorted()
    }
}

impl ObjectHistory {
    fn of_view(v: &ProjectileView) -> Self {
        Self {
            position: v.world,
            velocity: v.velocity,
            facing: v.facing,
            elevation: v.elevation,
            scale: v.scale,
            collection: v.collection,
            sequence: v.sequence,
            permutation: v.permutation,
            shading_slot: v.shading_slot,
            core: v.core,
            frame: v.frame,
            view: v.view,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectHistory {
    pub position: [i32; 3],
    pub velocity: [i16; 3],
    pub facing: u16,
    pub elevation: u16,
    pub scale: i32,
    pub collection: FourCC,
    pub sequence: u8,
    pub permutation: i16,
    pub shading_slot: u8,
    pub core: Option<FourCC>,
    pub frame: i8,
    pub view: u8,
}

impl Simulation {
    pub fn attach_projectile_flight(&mut self, mesh: &MeshTag) {
        let mut terrain = FlightTerrain::from_mesh(mesh);
        {
            let w = &self.engine;
            terrain = terrain
                .with_media(w.media.clone())
                .with_models(crate::models::ModelGrid::new(&w.cells, &w.models))
                .with_cell_flags(&w.cells.flags)
                .with_heights(&w.heights)
                .with_ceiling(w.flight_ceiling());
        }
        self.combat.flight.scenery = self.scenery_cylinders(mesh);
        self.combat.flight.terrain = Some(terrain);
    }

    pub(crate) fn scenery_cylinders(&self, mesh: &MeshTag) -> Vec<CollisionObject> {
        let world = &self.engine;
        let mut scenery: Vec<(u64, CollisionObject)> = Vec::new();
        for sc in &world.scenery {
            let o = &sc.object;
            if sc.deleted || o.no_shape || o.sequence == 0xff || o.flags & 0x400 == 0 {
                continue;
            }
            let Some(h) = world
                .collections
                .get(&o.collection)
                .and_then(|h| mandrake_tag::get_sequence(h, i32::from(o.sequence)))
            else {
                continue;
            };
            let Some(m) = mesh.markers.get(sc.marker) else {
                continue;
            };
            let (radius, bottom, top) = CollisionObject::scaled(h.world_bounds, o.scale);
            scenery.push((
                sc.cell_link,
                CollisionObject {
                    object: o.index,
                    unit: None,
                    kind: kind::SCENERY,
                    x: m.x,
                    y: m.y,
                    z: m.z,
                    radius,
                    bottom,
                    top,
                    flag_2000: o.flags & 0x2000 != 0,
                },
            ));
        }
        scenery.sort_by_key(|&(link, _)| std::cmp::Reverse(link));
        scenery.into_iter().map(|(_, c)| c).collect()
    }

    pub(crate) fn scenery_damage_table(
        &self,
        mesh: &MeshTag,
        tags: &dyn crate::engine_rng::TagSource,
    ) -> std::collections::BTreeMap<i32, crate::explosions::SceneryDamage> {
        let mut out = std::collections::BTreeMap::new();
        let world = &self.engine;
        for sc in &world.scenery {
            let palette = mesh.markers.get(sc.marker).and_then(|m| {
                let n = usize::try_from(m.palette_index).ok()?;
                mesh.palettes
                    .iter()
                    .filter(|p| p.marker_type == m.marker_type)
                    .nth(n)
            });
            let Some(scen) = palette.and_then(|p| tags.tag(*b"scen", p.tag_id)) else {
                continue;
            };
            let Some(tag) = scen.get(8..12).and_then(|w| <[u8; 4]>::try_from(w).ok()) else {
                continue;
            };
            let Some(obje) = tags.tag(*b"obje", FourCC::new(tag)) else {
                continue;
            };
            if let Some(d) = crate::explosions::SceneryDamage::from_tags(&scen, &obje) {
                out.insert(sc.object.index, d);
            }
        }
        out
    }

    pub fn attach_projectile_terrain(&mut self, terrain: FlightTerrain) {
        self.combat.flight.terrain = Some(terrain);
    }

    pub fn scenery_collision_objects(&self) -> Vec<CollisionObject> {
        self.combat.flight.scenery.clone()
    }

    pub fn unit_collision_object(
        &self,
        id: u32,
        object: i32,
        pos: [i32; 3],
    ) -> Option<CollisionObject> {
        let (radius, bottom, top) = self.unit_cylinder(id)?;
        Some(CollisionObject {
            object,
            unit: Some(id),
            kind: kind::MONSTER,
            x: pos[0],
            y: pos[1],
            z: pos[2],
            radius,
            bottom,
            top,
            flag_2000: false,
        })
    }

    pub fn projectiles_in_flight(&self) -> Vec<ProjectileView> {
        let mut v: Vec<ProjectileView> = self
            .combat
            .flight
            .flights
            .iter()
            .map(|p| self.record_view(p))
            .collect();
        v.sort_by_key(|p| p.record);
        v
    }

    pub fn projectile_views(&self) -> Vec<ProjectileView> {
        let mut out = self.projectiles_in_flight();
        out.extend(
            self.combat
                .flight
                .dormant
                .iter()
                .map(|d| self.dormant_view(d)),
        );
        out
    }

    fn dormant_view(&self, d: &DormantObject) -> ProjectileView {
        let mut v = self.object_view(
            crate::views::ObjectKind::Dormant,
            &d.object,
            d.position,
            [0; 3],
            (d.facing, 0),
        );
        v.definition = d.definition;
        v
    }

    fn fade_copy_view(&self, r: &crate::engine_rng::FadeRecord) -> ProjectileView {
        let o = &r.object;
        let mut v = self.object_view(
            crate::views::ObjectKind::Copy,
            o,
            [r.position.0, r.position.1, o.z],
            [0; 3],
            (r.facing, 0),
        );
        v.record = r.index;
        v
    }

    pub fn drawn_projectile_views(&self) -> Vec<ProjectileView> {
        self.projectile_views()
            .into_iter()
            .map(|mut v| {
                if let Some(h) = self.engine.object_history.get(&v.object) {
                    v.world = h.position;
                    v.position = self.world_units_to_sim(h.position);
                    v.velocity = h.velocity;
                    v.facing = h.facing;
                    v.elevation = h.elevation;
                    v.scale = h.scale;
                    v.collection = h.collection;
                    v.sequence = h.sequence;
                    v.permutation = h.permutation;
                    v.shading_slot = h.shading_slot;
                    v.core = h.core;
                    v.frame = h.frame;
                    v.view = h.view;
                    let (heading, pitch) = velocity_angles(h.velocity, v.facing);
                    v.heading = heading;
                    v.pitch = pitch;
                }
                v
            })
            .collect()
    }

    pub fn fade_copy_views(&self) -> Vec<ProjectileView> {
        let w = &self.engine;
        let mut v: Vec<ProjectileView> = w
            .fade_records
            .iter()
            .filter(|r| r.host == crate::engine_rng::FadeRecord::NO_HOST)
            .map(|r| self.fade_copy_view(r))
            .collect();
        v.sort_by_key(|p| p.record);
        v
    }

    pub fn drawn_fade_copy_views(&self) -> Vec<ProjectileView> {
        let history = &self.engine.object_history;
        self.fade_copy_views()
            .into_iter()
            .map(|mut v| {
                if let Some(h) = history.get(&v.object) {
                    v.collection = h.collection;
                    v.sequence = h.sequence;
                    v.permutation = h.permutation;
                    v.shading_slot = h.shading_slot;
                    v.core = h.core;
                    v.frame = h.frame;
                    v.view = h.view;
                }
                v
            })
            .collect()
    }

    pub(crate) fn record_object_history(&mut self) {
        let mut h = std::mem::take(&mut self.engine.object_history);
        h.clear();
        let mut order = std::mem::take(&mut h.order);
        let mut put = |v: ProjectileView| {
            if !v.hidden && v.object >= 0 {
                h.put(v.object, true, ObjectHistory::of_view(&v));
            }
        };
        let flight = &self.combat.flight;
        order.clear();
        order.extend(
            flight
                .flights
                .iter()
                .enumerate()
                .map(|(k, p)| (p.index, k as u32)),
        );
        order.sort_unstable();
        for &(_, k) in &order {
            put(self.record_view(&flight.flights[k as usize]));
        }
        for d in &flight.dormant {
            put(self.dormant_view(d));
        }
        {
            let w = &self.engine;
            order.clear();
            order.extend(
                w.fade_records
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.host == crate::engine_rng::FadeRecord::NO_HOST)
                    .map(|(k, r)| (r.index, k as u32)),
            );
            order.sort_unstable();
            for &(_, k) in &order {
                put(self.fade_copy_view(&w.fade_records[k as usize]));
            }
        }
        h.order = order;
        let mut first = std::mem::take(&mut h.first_monster);
        {
            let w = &self.engine;
            first.clear();
            for (i, m) in w.monsters.iter().enumerate() {
                if let Some(key) = m.unit_key() {
                    first.entry(key).or_insert(i);
                }
            }
            for m in &w.monsters {
                let Some(id) = m.unit_key() else {
                    continue;
                };
                if m.object.no_shape {
                    continue;
                }
                let Some(o) =
                    crate::projectile_groups::SimSpawn::unit_object_at(self, id, first[&id])
                else {
                    continue;
                };
                h.put(
                    o.index,
                    false,
                    ObjectHistory {
                        position: o.position,
                        velocity: o.velocity,
                        facing: o.facing,
                        elevation: o.elevation,
                        scale: m.object.scale,
                        collection: o.collection,
                        sequence: o.sequence,
                        permutation: o.shading,
                        shading_slot: o.shading_slot,
                        core: o.core,
                        frame: o.frame,
                        view: o.view,
                    },
                );
            }
        }
        h.first_monster = first;
        {
            let w = &mut self.engine;
            w.object_history = h;
        }
    }

    fn object_view(
        &self,
        kind: crate::views::ObjectKind,
        o: &crate::engine_rng::EngineObject,
        world: [i32; 3],
        velocity: [i16; 3],
        (facing, elevation): (u16, u16),
    ) -> ProjectileView {
        let (heading, pitch) = velocity_angles(velocity, facing);
        ProjectileView {
            kind,
            id: 0,
            record: -1,
            object: o.index,
            definition: FourCC::new([0; 4]),
            owner: 0,
            target: None,
            world,
            position: self.world_units_to_sim(world),
            velocity,
            facing,
            elevation,
            heading,
            pitch,
            launched_tick: 0,
            collection: o.collection,
            sequence: o.sequence,
            frame: o.frame,
            view: o.view,
            scale: o.scale,
            flags: o.flags,
            hidden: o.no_shape,
            mirror_x: o.flags & 0x2_0000 != 0,
            mirror_y: o.flags & 0x4_0000 != 0,
            permutation: o.permutation,
            shading_slot: o.shading_slot,
            core: o.core,
            transfer: (o.sub_kind, o.sub_count, o.sub_limit),
        }
    }

    fn record_view(&self, p: &FlightProjectile) -> ProjectileView {
        let o = &p.object;
        let empty;
        let shape = match p.shape.as_ref() {
            Some(s) => s,
            None => {
                empty = crate::engine_rng::EngineObject::new(-1, FourCC::new([0xff; 4]));
                &empty
            }
        };
        let mut v = self.object_view(
            crate::views::ObjectKind::Projectile,
            shape,
            [o.x, o.y, o.z],
            [o.vx, o.vy, o.vz],
            (p.facing, p.elevation),
        );
        v.id = p.id;
        v.record = p.index;
        v.definition = p.definition;
        v.owner = p.owner;
        v.target = p.target;
        v.launched_tick = p.launched_tick;
        v
    }

    pub fn world_units_to_sim(&self, p: [i32; 3]) -> Vec3Fixed {
        let mut v = self.command_point_to_world(p[0], p[1]);
        v.y = Fixed16::from_raw(p[2].wrapping_mul(65536 / mandrake_rec::WORLD_UNITS_PER_CELL));
        v
    }

    pub fn object_location(&self, index: i32, identifier: i16) -> Option<[i32; 3]> {
        let same =
            |o: &crate::engine_rng::EngineObject| o.index == index && o.identifier == identifier;
        let flight = &self.combat.flight;
        if let Some(p) = flight
            .flights
            .iter()
            .find(|p| p.shape.as_ref().is_some_and(same))
        {
            return Some([p.object.x, p.object.y, p.object.z]);
        }
        if let Some(d) = flight.dormant.iter().find(|d| same(&d.object)) {
            return Some(d.position);
        }
        let w = &self.engine;
        let at = w.monsters.iter().position(|m| same(&m.object))?;
        let m = &w.monsters[at];
        let (x, y) = match w.movers.get(at).and_then(Option::as_ref) {
            Some(v) => (v.x, v.y),
            None => self.unit_world_xy(m.unit_key()?)?,
        };
        Some([x, y, m.object.z])
    }

    pub fn object_flags(&self, index: i32, identifier: i16) -> Option<u32> {
        let same =
            |o: &crate::engine_rng::EngineObject| o.index == index && o.identifier == identifier;
        let with_shape =
            |o: &crate::engine_rng::EngineObject| o.flags | if o.no_shape { 0x200 } else { 0 };
        let flight = &self.combat.flight;
        if let Some(o) = flight
            .flights
            .iter()
            .filter_map(|p| p.shape.as_ref())
            .find(|o| same(o))
        {
            return Some(with_shape(o));
        }
        if let Some(d) = flight.dormant.iter().find(|d| same(&d.object)) {
            return Some(with_shape(&d.object));
        }
        let w = &self.engine;
        w.monsters
            .iter()
            .find(|m| same(&m.object))
            .map(|m| with_shape(&m.object))
    }

    pub fn mesh_height_at(&self, x: i32, y: i32) -> i32 {
        self.ground(x, y)
    }

    pub(crate) fn unit_world_xy(&self, id: u32) -> Option<(i32, i32)> {
        let u = self.units.get(&id)?;
        Some(match u.engine.as_ref() {
            Some(m) => (m.x, m.y),
            None => self.world_to_command_point(u.position),
        })
    }

    pub(crate) fn ground(&self, x: i32, y: i32) -> i32 {
        self.combat
            .flight
            .terrain
            .as_ref()
            .or(self.combat.flight.ground.as_ref())
            .map_or(0, |t| t.height(x, y))
    }

    pub(crate) fn unit_cylinder(&self, id: u32) -> Option<(i16, i16, i16)> {
        let u = self.units.get(&id)?;
        let bounds: SequenceWorldBounds =
            *self.combat.definitions.monster_bounds.get(&u.archetype)?;
        let scale = u.engine_object.map_or(0x100, |o| i32::from(o.scale));
        Some(CollisionObject::scaled(bounds, scale))
    }

    pub fn collision_objects(&self) -> Vec<CollisionObject> {
        let mut list = CollisionList::default();
        self.refresh_collision_objects(&mut list);
        list.sorted
    }

    pub(crate) fn refresh_collision_objects(&self, list: &mut CollisionList) {
        let mut ids = std::mem::take(&mut list.ids);
        list.tables.living_units(
            self.units
                .iter()
                .filter(|(_, u)| u.is_alive())
                .map(|(id, u)| {
                    let xy = match u.engine.as_ref() {
                        Some(m) => (m.x, m.y),
                        None => self.world_to_command_point(u.position),
                    };
                    (*id, xy)
                }),
            &mut ids,
        );
        {
            let w = &self.engine;
            list.tables.begin();
            list.tables
                .scenery_links(w.scenery_index.arc(&w.scenery), &w.scenery);
            list.tables.index_monsters(&w.monsters);
            for r in w.solid_copies() {
                list.tables.set_link(r.object.index, r.cell_link);
            }
        }
        let tables = &list.tables;
        let mut out = std::mem::take(&mut list.spare);
        out.clear();
        out.extend_from_slice(&self.combat.flight.scenery);
        let mut last_collection: Option<(FourCC, Option<&Vec<SequenceHeader>>)> = None;
        out.extend(ids.iter().filter_map(|&(id, (x, y))| {
            let slot = tables.monster_slot(id);
            let engine = slot.map(|i| (&self.engine, &self.engine.monsters[i]));
            if engine.is_some_and(|(_, m)| m.object.no_shape || m.object.flags & 0x400 == 0) {
                return None;
            }
            let engine_cylinder = engine.and_then(|(w, m)| {
                let c = m.object.collection;
                let headers = match last_collection {
                    Some((k, h)) if k == c => h,
                    _ => {
                        let h = w.collections.get(&c);
                        last_collection = Some((c, h));
                        h
                    }
                }?;
                m.object.bounds(headers).map(|c| (c, m.object.z))
            });
            let ((radius, bottom, top), z) = match engine_cylinder {
                Some(c) => c,
                None => (self.unit_cylinder(id)?, self.ground(x, y)),
            };
            let object = engine.map_or(id as i32, |(_, m)| m.object.index);
            let flag_2000 = engine.is_some_and(|(_, m)| m.object.flags & 0x2000 != 0);
            Some(CollisionObject {
                object,
                unit: Some(id),
                kind: kind::MONSTER,
                x,
                y,
                z,
                radius,
                bottom,
                top,
                flag_2000,
            })
        }));
        list.ids = ids;
        let mut keys = std::mem::take(&mut list.spare_keys);
        keys.clear();
        {
            let w = &self.engine;
            out.extend(w.solid_copies().filter_map(|r| {
                let o = &r.object;
                let (radius, bottom, top) = o.bounds(w.collections.get(&o.collection)?)?;
                Some(CollisionObject {
                    object: o.index,
                    unit: None,
                    kind: o.kind,
                    x: r.position.0,
                    y: r.position.1,
                    z: o.z,
                    radius,
                    bottom,
                    top,
                    flag_2000: o.flags & 0x2000 != 0,
                })
            }));
            let solid =
                |o: &crate::engine_rng::EngineObject| !o.no_shape && o.flags & 0x600 == 0x400;
            for p in self.combat.flight.flights.iter() {
                let Some(o) = p.shape.as_ref().filter(|o| solid(o)) else {
                    continue;
                };
                let Some((radius, bottom, top)) =
                    w.collections.get(&o.collection).and_then(|h| o.bounds(h))
                else {
                    continue;
                };
                list.tables.set_link(o.index, p.cell_link);
                out.push(CollisionObject {
                    object: o.index,
                    unit: None,
                    kind: kind::PROJECTILE,
                    x: p.object.x,
                    y: p.object.y,
                    z: p.object.z,
                    radius,
                    bottom,
                    top,
                    flag_2000: o.flags & 0x2000 != 0,
                });
            }
            for d in self
                .combat
                .flight
                .dormant
                .iter()
                .filter(|d| solid(&d.object))
            {
                let o = &d.object;
                let Some((radius, bottom, top)) =
                    w.collections.get(&o.collection).and_then(|h| o.bounds(h))
                else {
                    continue;
                };
                list.tables.set_link(o.index, d.cell_link);
                out.push(CollisionObject {
                    object: o.index,
                    unit: None,
                    kind: o.kind,
                    x: d.position[0],
                    y: d.position[1],
                    z: d.position[2],
                    radius,
                    bottom,
                    top,
                    flag_2000: o.flags & 0x2000 != 0,
                });
            }
            let tables = &list.tables;
            keys.extend(out.iter().map(|o| tables.link(o.object)));
        }
        if list.valid && out == list.unsorted && keys == list.keys {
            list.spare = out;
            list.spare_keys = keys;
            return;
        }
        list.sorted.clear();
        let mut keyed: Vec<(std::cmp::Reverse<u64>, u32)> = keys
            .iter()
            .enumerate()
            .map(|(i, &k)| (std::cmp::Reverse(k), i as u32))
            .collect();
        keyed.sort_unstable();
        list.sorted
            .extend(keyed.into_iter().map(|(_, i)| out[i as usize]));

        list.cells.reset();
        list.spare = std::mem::replace(&mut list.unsorted, out);
        list.spare_keys = std::mem::replace(&mut list.keys, keys);
        list.valid = true;
    }

    fn rebound_event(&self, p: &FlightProjectile, sound: Option<FourCC>, tick: u32) -> CombatEvent {
        CombatEvent::ProjectileRebounded {
            tick,
            projectile_id: p.id,
            definition: p.definition,
            sound,
            position: self.world_units_to_sim([p.object.x, p.object.y, p.object.z]),
            world: [p.object.x, p.object.y, p.object.z],
            request_flags: u16::from(p.shape.as_ref().is_none_or(|o| o.flags & 0x80 == 0)),
            object: p.shape.as_ref().map_or(-1, |o| o.index),
            identifier: p.shape.as_ref().map_or(0, |o| o.identifier),
            velocity: [p.object.vx, p.object.vy, p.object.vz],
        }
    }

    pub fn update_projectile_flight(&mut self, tick: u32) {
        let mut terrain = match self.combat.flight.terrain.take() {
            Some(t) => (t, true),
            None => match self.combat.flight.ground.take() {
                Some(g) => (g, false),
                None => return,
            },
        };
        self.take_cells_change(Some(&mut terrain.0));
        let idle = self.combat.flight.flights.is_empty() && (self.engine.fade_records.is_empty());
        let mut list = std::mem::take(&mut self.collision_cache);
        list.valid = false;
        list.pending = !idle;
        let no_headers: std::sync::Arc<[SequenceHeader]> = std::sync::Arc::from([]);
        let mut last = -1;
        let mut at_of: Vec<u32> = Vec::new();
        let mut shift = 0usize;
        let solid = |o: &crate::engine_rng::EngineObject| !o.no_shape && o.flags & 0x600 == 0x400;
        let mut any_solid = {
            let f = &self.combat.flight;
            (f.flights.iter()).any(|p| p.shape.as_ref().is_some_and(solid))
                || f.dormant.iter().any(|d| solid(&d.object))
        };
        loop {
            let flights = &self.combat.flight.flights;
            let adjacent = usize::try_from(last + 1)
                .ok()
                .and_then(|i| at_of.get(i))
                .and_then(|&k| {
                    let k = k as usize;
                    (k.saturating_sub(shift)..=k)
                        .rev()
                        .find(|&k| flights.get(k).is_some_and(|p| p.index == last + 1))
                })
                .map(|k| (k, last + 1));
            let next = adjacent.or_else(|| {
                at_of.clear();
                shift = 0;
                let mut next: Option<(usize, i32)> = None;
                for (k, p) in flights.iter().enumerate() {
                    if let Ok(i) = usize::try_from(p.index) {
                        if i >= at_of.len() {
                            at_of.resize(i + 1, u32::MAX);
                        }
                        debug_assert_eq!(at_of[i], u32::MAX, "two records with index {i}");
                        at_of[i] = k as u32;
                    }
                    if p.index > last && next.is_none_or(|(_, n)| p.index < n) {
                        next = Some((k, p.index));
                    }
                }
                next
            });
            let fade = adjacent
                .is_none()
                .then(|| {
                    self.engine
                        .fade_records
                        .iter()
                        .map(|r| r.index)
                        .filter(|&r| r > last)
                        .min()
                })
                .flatten()
                .filter(|&r| next.is_none_or(|(_, n)| r < n));
            if let Some(r) = fade {
                last = r;
                if self.engine_stop.is_none() {
                    let solid = self.engine.solid_copies().count();
                    if std::mem::take(&mut list.pending) {
                        self.refresh_collision_objects(&mut list);
                    }
                    match self.update_fade_turn(r, tick) {
                        Ok(changed) => {
                            let solid_now = Some(self.engine.solid_copies().count());
                            if changed || solid_now != Some(solid) {
                                list.pending = true;
                            }
                        }
                        Err(e) => {
                            self.engine_stop.get_or_insert(e);
                        }
                    }
                }
                continue;
            }
            let Some((at, _)) = next else {
                break;
            };
            let records_before = self.combat.flight.flights.len();
            let mut p = self.combat.flight.flights.remove(at);
            last = p.index;
            if any_solid {
                list.pending = true;
            }
            if p.engine && self.engine_stop.is_some() {
                self.combat.flight.flights.insert(at, p);
                continue;
            }
            let def = self
                .combat
                .definitions
                .flights
                .get(&p.definition)
                .or_else(|| self.engine.catalog.flights.get(&p.definition))
                .cloned();
            let Some(def) = def else {
                if let Some(s) = p.shape.as_ref() {
                    self.engine.free_objects.insert(s.index);
                }
                shift += 1;
                continue;
            };
            let headers = p
                .shape
                .as_ref()
                .and_then(|s| {
                    let c = s.collection;
                    if let Some(h) = self.sequence_headers_cache.get(&c) {
                        return Some(std::sync::Arc::clone(h));
                    }
                    let h: std::sync::Arc<[SequenceHeader]> =
                        std::sync::Arc::from(self.engine.collections.get(&c)?.as_slice());
                    self.sequence_headers_cache
                        .insert(c, std::sync::Arc::clone(&h));
                    Some(h)
                })
                .unwrap_or_else(|| std::sync::Arc::clone(&no_headers));
            let cell = (p.object.x >> 9, p.object.y >> 9);
            let before = [p.object.x, p.object.y, p.object.z];
            let velocity_before = [p.object.vx, p.object.vy, p.object.vz];
            let reads = p.update_reads_objects(&def);
            if reads && std::mem::take(&mut list.pending) {
                self.refresh_collision_objects(&mut list);
            }
            let (objects, pending) = if reads || !list.pending {
                (
                    crate::projectiles::CollisionObjects::with_cells(&list.sorted, &list.cells),
                    None,
                )
            } else {
                (crate::projectiles::CollisionObjects::NONE, Some(&mut list))
            };
            let step = p.update(
                &def,
                &headers,
                &terrain.0,
                objects,
                tick,
                &mut SimUpdate {
                    sim: self,
                    tick,
                    pending,
                },
            );
            let refresh = matches!(
                step,
                Ok(FlightStep::Detonated { .. } | FlightStep::Replaced { .. })
            );
            any_solid |= p.shape.as_ref().is_some_and(solid);
            if let Some(o) = p.shape.as_mut()
                && let Some(sounds) = o.anim_sounds.take()
            {
                self.combat
                    .sequence_sounds
                    .push(crate::combat::SequenceSound {
                        tick,
                        object: o.index,
                        identifier: o.identifier,
                        collection: o.collection,
                        sounds,
                        world: before,
                        request_flags: u16::from(o.flags & 0x80 == 0),
                        velocity: velocity_before,
                    });
            }
            if cell != (p.object.x >> 9, p.object.y >> 9) || std::mem::take(&mut p.relinked) {
                p.cell_link = self.engine.next_cell_link();
            }
            match step {
                Ok(FlightStep::Flying) => self.combat.flight.flights.insert(at, p),
                Ok(FlightStep::Rebounded { sound }) => {
                    self.combat.events.push(self.rebound_event(&p, sound, tick));
                    self.combat.flight.flights.insert(at, p);
                }
                Ok(FlightStep::Dormant { sound }) => {
                    if sound.is_some() {
                        self.combat.events.push(self.rebound_event(&p, sound, tick));
                    }
                    let flags = Some(&self.engine.flag_data);
                    if let Some(d) = sleep_projectile(&mut p, flags) {
                        self.combat.flight.dormant.push(d);
                    }
                }
                Ok(FlightStep::Replaced { sound }) => {
                    if sound.is_some() {
                        self.combat.events.push(self.rebound_event(&p, sound, tick));
                    }
                    if let Err(e) = self.replace_record(&p, tick) {
                        if p.engine {
                            self.engine_stop.get_or_insert(e);
                        } else {
                            self.combat.flight.reports.push((tick, e));
                        }
                    }
                }
                Ok(FlightStep::Detonated { keep, hit }) => {
                    let hit_other = hit
                        .map(|i| &list.sorted[i])
                        .filter(|o| o.unit.is_none())
                        .map(|o| (o.kind, o.object));
                    match self.detonate_record_hit(
                        &mut p,
                        keep,
                        hit.is_some(),
                        hit.and_then(|i| list.sorted[i].unit),
                        hit_other,
                        tick,
                    ) {
                        Ok(true) => self.combat.flight.flights.insert(at, p),
                        Ok(false) => {}
                        Err(e) => {
                            if p.engine {
                                self.engine_stop.get_or_insert(e);
                            } else {
                                self.combat.flight.reports.push((tick, e));
                            }
                            self.combat.flight.flights.insert(at, p);
                        }
                    }
                }
                Err(e) if p.engine => {
                    self.engine_stop.get_or_insert(e);
                    self.combat.flight.flights.insert(at, p);
                }
                Err(e) => {
                    if let Some(s) = p.shape.as_ref() {
                        self.engine.free_objects.insert(s.index);
                    }
                    self.combat.flight.reports.push((tick, e));
                }
            }
            if refresh {
                list.pending = true;
            }
            if any_solid {
                self.sync_solid_objects();
            }
            self.take_cells_change(Some(&mut terrain.0));
            shift += records_before.saturating_sub(self.combat.flight.flights.len());
        }
        self.collision_cache = list;
        if terrain.1 {
            self.combat.flight.terrain = Some(terrain.0);
        } else {
            self.combat.flight.ground = Some(terrain.0);
        }
    }

    pub(crate) fn detonate_record(
        &mut self,
        p: &mut crate::projectiles::FlightProjectile,
        keep: bool,
        hit: bool,
        hit_unit: Option<u32>,
        tick: u32,
    ) -> Result<bool, Unmodelled> {
        self.detonate_record_hit(p, keep, hit, hit_unit, None, tick)
    }

    pub(crate) fn detonate_record_at(
        &mut self,
        p: &mut crate::projectiles::FlightProjectile,
        point: [i32; 3],
        scale: i32,
        tick: u32,
    ) -> Result<bool, Unmodelled> {
        self.combat.flight.detonate_at = Some((point, scale));
        let r = self.detonate_record_hit(p, false, false, None, None, tick);
        self.combat.flight.detonate_at = None;
        r
    }

    fn release_owner_monster(&mut self, p: &crate::projectiles::FlightProjectile) {
        let w = &mut self.engine;
        let Ok(i) = usize::try_from(p.owner_monster) else {
            return;
        };
        let gameplay = w.formation_globals.gameplay_version;
        let m = &mut w.monsters[i];
        m.flags &= 0xbdff_ffff;
        m.object.no_shape = false;
        if gameplay > 0x6ad {
            let z = w.monster_location_height(i, w.monsters[i].position);
            w.monsters[i].object.z = z;
        }
    }

    pub(crate) fn update_fade_turn(&mut self, r: i32, tick: u32) -> Result<bool, Unmodelled> {
        let w = &mut self.engine;
        let Some(at) = w.fade_records.iter().position(|f| f.index == r) else {
            return Ok(false);
        };
        let gameplay = w.formation_globals.gameplay_version;
        let mut changed = false;
        if w.fade_records[at].countdown != 0 {
            let f = &mut w.fade_records[at];
            f.countdown -= 1;
            if f.countdown != 0 {
                return Ok(false);
            }
            if gameplay <= game_options::GAMEPLAY_1_7_1 {
                return Err(Unmodelled::new(
                    draw_site::FADE_COUNTDOWN_OLD_GAMEPLAY,
                    format!("the end of a fade record's countdown at gameplay {gameplay:#x}"),
                ));
            }
            if let Some(group) = f.group {
                let o = &f.object;
                let headers = w.collections.get(&o.collection).map_or(&[][..], |v| v);
                let (radius, bottom, top) = o.bounds(headers).unwrap_or((0, 0, 0));
                let source = crate::projectile_groups::ObjectSnapshot {
                    index: o.index,
                    position: [f.position.0, f.position.1, o.z],
                    velocity: o.velocity,
                    facing: f.facing,
                    elevation: 0,
                    radius,
                    bottom,
                    top,
                    kind: o.kind,
                    owner: o.owner,
                    collection: o.collection,
                    sequence: o.sequence,
                    frame: o.frame,
                    view: o.view,
                    shading: o.permutation,
                    shading_slot: o.shading_slot,
                    core: o.core,
                };
                let desc = crate::projectile_groups::GroupDescriptor {
                    flags: 0,
                    group,
                    shading_object: o.index,
                    object: o.index,
                    owner: f.monster.map_or(-1, |m| m as i16),
                    point: [f.position.0, f.position.1, 0],
                    magnitude: 0,
                    velocity: [0; 3],
                };
                let mut spawn = crate::projectile_groups::SimSpawn {
                    sim: self,
                    tick,
                    source: Some(source),
                    reserved: None,
                    source_record: None,
                    engine: true,
                    updating_object: None,
                };
                crate::projectile_groups::new_projectile_group(&mut spawn, &desc)?;
                changed = true;
            }
            let w = &mut self.engine;
            if let Some(f) = w.fade_records.iter_mut().find(|f| f.index == r) {
                f.object.no_shape = false;
            }
        }
        let place = self.fade_object_place(r);
        let w = &mut self.engine;
        if let Some(f) = w.fade_records.iter_mut().find(|f| f.index == r) {
            f.object.anim_sounds = None;
        }
        let ended = w.update_fade_record(r);
        let object = match &ended {
            Some(f) => Some(&f.object),
            None => w
                .fade_records
                .iter()
                .find(|f| f.index == r)
                .map(|f| &f.object),
        };
        if let (Some(o), Some(world)) = (object, place)
            && let Some(sounds) = o.anim_sounds
        {
            self.combat
                .sequence_sounds
                .push(crate::combat::SequenceSound {
                    tick,
                    object: o.index,
                    identifier: o.identifier,
                    collection: o.collection,
                    sounds,
                    world,
                    request_flags: u16::from(o.flags & 0x80 == 0),
                    velocity: o.velocity,
                });
        }
        let w = &mut self.engine;
        if let Some(ended) = ended
            && let Some(i) = ended.monster
        {
            let m = &mut w.monsters[i];
            m.flags &= 0xbdff_ffff;
            m.object.no_shape = false;
            if gameplay > 0x6ad {
                let z = w.monster_location_height(i, w.monsters[i].position);
                w.monsters[i].object.z = z;
            }
            changed = true;
        }
        Ok(changed)
    }

    pub(crate) fn fade_object_place(&self, r: i32) -> Option<[i32; 3]> {
        let w = &self.engine;
        let f = w.fade_records.iter().find(|f| f.index == r)?;
        let (x, y) = if f.host == crate::engine_rng::FadeRecord::NO_HOST {
            f.position
        } else {
            self.monster_place(f.host)?
        };
        Some([x, y, f.object.z])
    }

    pub(crate) fn monster_place(&self, i: usize) -> Option<(i32, i32)> {
        let w = &self.engine;
        let m = w.monsters.get(i)?;
        Some(match w.movers.get(i).and_then(Option::as_ref) {
            Some(v) => (v.x, v.y),
            None => m
                .unit_key()
                .and_then(|id| self.unit_world_xy(id))
                .unwrap_or(m.position),
        })
    }

    pub fn detonate_record_hit(
        &mut self,
        p: &mut crate::projectiles::FlightProjectile,
        keep: bool,
        hit: bool,
        hit_unit: Option<u32>,
        hit_other: Option<(u8, i32)>,
        tick: u32,
    ) -> Result<bool, Unmodelled> {
        {
            let w = &mut self.engine;
            w.reserved_records.push(p.index);
        }
        let held = crate::explosions::ExplosionSource::of(p);
        if let Some(s) = held {
            self.combat.flight.detonating_records.push(s);
        }
        let r = self.detonate_record_reserved(p, keep, hit, hit_unit, hit_other, tick);
        if held.is_some() {
            self.combat.flight.detonating_records.pop();
        }
        {
            let w = &mut self.engine;
            w.reserved_records.pop();
        }
        r
    }

    fn detonate_record_reserved(
        &mut self,
        p: &mut crate::projectiles::FlightProjectile,
        keep: bool,
        hit: bool,
        mut hit_unit: Option<u32>,
        hit_other: Option<(u8, i32)>,
        tick: u32,
    ) -> Result<bool, Unmodelled> {
        let at = self.combat.flight.detonate_at.take();
        let Some(proj) = self
            .engine
            .catalog
            .projectiles
            .get(&p.definition)
            .map(DetonateProjectile::of)
        else {
            return Err(Unmodelled::new(
                draw_site::DETONATE_PROJECTILE_MISSING,
                format!("proj {} not loaded", p.definition),
            ));
        };
        if p.record_flags & 2 != 0 && proj.flags & 0xa000 == 0 {
            return Ok(true);
        }
        self.engine_sites.add(FREQUENCY_DRAW_SITE, 1);
        crate::engine_rng::log_draw(FREQUENCY_DRAW_SITE, self.rng.state);
        if frequency_corrected(&mut self.rng, proj.impact_frequency, 0x6b8) {
            self.replace_record(p, tick)?;
            return Ok(false);
        }
        p.record_flags |= 1;
        if p.record_flags & 2 == 0 && p.object.above_ground <= 0x5ff && proj.stain_sequence != -1 {
            let collection = proj.collection.unwrap_or(FourCC::new([0xff; 4]));
            self.stain_mesh(
                tick,
                collection,
                proj.stain_sequence,
                [p.object.x, p.object.y, p.object.z],
            );
        }
        let point = [p.object.x, p.object.y, p.object.z];
        let damage = proj.damage;
        let impact = if p.record_flags & 2 == 0 && damage.deals_damage() {
            let mana = self.take_detonation_mana(p, proj);
            let single_target = damage.flags & 0x1002 == 0;
            let other = hit_other.filter(|_| single_target && hit_unit.is_none());
            if let Some((kind::SCENERY, index)) = other {
                return self.detonate_on_scenery(p, proj, &damage, index, keep, tick, mana);
            }
            if let Some((k @ (kind::PROJECTILE | 4 | crate::flags::OBJECT_KIND_FLAG), index)) =
                other
            {
                let flight = &self.combat.flight;
                let victim = if k == kind::PROJECTILE {
                    (flight.flights.iter())
                        .find(|q| q.shape.as_ref().is_some_and(|o| o.index == index))
                        .map(|q| crate::explosions::VictimObject::Flight(q.id))
                } else {
                    (flight.dormant.iter())
                        .any(|d| d.object.index == index)
                        .then_some(crate::explosions::VictimObject::Dormant(index))
                };
                let Some(victim) = victim else {
                    return Err(Unmodelled::new(
                        draw_site::DETONATION_ON_PROJECTILE_OBJECT,
                        format!("object {index} of kind {k} is gone"),
                    ));
                };
                return self.detonate_on_object(p, proj, &damage, victim, keep, tick, mana);
            }
            if hit && hit_unit.is_none() && single_target && !matches!(other, Some((5, _))) {
                return Err(Unmodelled::new(
                    draw_site::DETONATION_ON_PROJECTILE_OBJECT,
                    "cause_damage on a projectile object (accelerate_projectile)",
                ));
            }
            if let Some((5, _)) = other {
                if damage.damage_delta > 0 {
                    self.engine_sites.add(draw_site::DAMAGE_AMOUNT, 1);
                    crate::engine_rng::log_draw(draw_site::DAMAGE_AMOUNT, self.rng.state);
                    self.rng.next_u16();
                }
                None
            } else if single_target
                && (self.engine_attack_launch || p.engine)
                && let Some(u) = hit_unit
                && (self.engine.monster_index_by_identifier(u as u16).is_some())
            {
                let r = self.engine_cause_damage(tick, p, proj, &damage, u, mana)?;
                {
                    let w = &self.engine;
                    hit_unit = Some(w.renamed_unit(u));
                }
                r
            } else {
                self.combat.flight.detonating = Some(crate::explosions::ExplosionInfo {
                    unit: p.record_0a,
                    attacker: p.owner_monster,
                    attacker_id: usize::try_from(p.owner_monster)
                        .ok()
                        .and_then(|a| self.engine.monsters.get(a))
                        .map_or(0, |m| m.identifier),
                    velocity: [p.object.vx, p.object.vy, p.object.vz],
                    flags: if proj.flags & 0x400_0000 != 0 { 2 } else { 0 },
                    mana: mana.0,
                    factor_bits: mana.1.to_bits(),
                    source: crate::explosions::ExplosionSource::of(p),
                });
                let r =
                    self.cause_damage(tick, p.definition, &damage, p.owner, hit_unit, Some(point));
                self.combat.flight.detonating = None;
                r
            }
        } else {
            None
        };
        let victim = impact.and(hit_unit);
        let group = match impact {
            Some(g) => g,
            None => proj.detonation_group,
        };
        let detonated = CombatEvent::ProjectileDetonated {
            tick,
            projectile_id: p.id,
            definition: p.definition,
            owner: p.owner,
            hit_unit,
            position: self.world_units_to_sim(point),
            group,
        };
        if let Some(group) = group {
            let record_src = crate::projectile_groups::record_object(p).ok_or_else(|| {
                Unmodelled::new(
                    draw_site::DETONATION_GROUP_NO_OBJECT,
                    "detonation group without an object",
                )
            })?;
            let src = match victim {
                Some(u) => {
                    crate::projectile_groups::SimSpawn::unit_object(self, u).ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::IMPACT_GROUP_VICTIM,
                            "impact group victim object",
                        )
                    })?
                }
                None => record_src,
            };
            let scale = at.map_or(0, |a| a.1);
            let (flags, magnitude) = if p.record_flags & 2 != 0 {
                (if p.record_flags & 4 != 0 { 0x14 } else { 0x10 }, scale)
            } else {
                (
                    if p.record_flags & 4 != 0 { 4 } else { 0 },
                    i32::from(proj.group_scale.max(scale as i16)),
                )
            };
            let point = at.map_or(
                [
                    src.position[0],
                    src.position[1],
                    src.position[2] + ((i32::from(src.bottom) + i32::from(src.top)) >> 1),
                ],
                |a| a.0,
            );
            let desc = crate::projectile_groups::GroupDescriptor {
                flags,
                group,
                shading_object: src.index,
                object: record_src.index,
                owner: p.owner_monster,
                point,
                magnitude,
                velocity: [0; 3],
            };
            let mut w = crate::projectile_groups::SimSpawn::of_record(self, p, tick);
            crate::projectile_groups::new_projectile_group(&mut w, &desc)?;
        }
        self.combat.events.push(detonated);
        if proj.flags & 0x1000_0000 != 0 {
            self.projectile_starts_fire(p)?;
        }
        if proj.flags & 0x800_0000 != 0 {
            self.release_owner_monster(p);
        }
        if keep {
            p.record_flags &= !1;
        } else if let Some(s) = p.shape.as_ref() {
            self.engine.free_objects.insert(s.index);
        }
        Ok(keep)
    }

    fn take_detonation_mana(
        &mut self,
        p: &crate::projectiles::FlightProjectile,
        proj: DetonateProjectile,
    ) -> (i16, f32) {
        if proj.raw_damage_flags & 0x400 == 0 {
            return (0, 0.0);
        }
        let w = &mut self.engine;
        let Some(a) = usize::try_from(p.owner_monster)
            .ok()
            .filter(|&a| a < w.monsters.len())
        else {
            return (0, 0.0);
        };
        let mana = w.combat(a).mana;
        let max = w.monster_definitions[w.monsters[a].definition]
            .ai
            .word(0x214);
        let factor = if mana != 0 && proj.flags & 0x40_0000 != 0 {
            f32::from(mana) / f32::from(max)
        } else {
            0.0
        };
        w.combat_mut(a).mana = 0;
        (mana, factor)
    }

    #[allow(clippy::too_many_arguments)]
    fn detonate_on_scenery(
        &mut self,
        p: &mut crate::projectiles::FlightProjectile,
        proj: DetonateProjectile,
        damage: &mandrake_tag::projectile::DamageDefinition,
        index: i32,
        keep: bool,
        tick: u32,
        mana: (i16, f32),
    ) -> Result<bool, Unmodelled> {
        let point = [p.object.x, p.object.y, p.object.z];
        let Some(i) = self
            .engine
            .scenery
            .iter()
            .position(|s| !s.deleted && s.object.index == index)
        else {
            return Err(Unmodelled::new(
                draw_site::DETONATION_ON_PROJECTILE_OBJECT,
                format!("object {index}"),
            ));
        };
        let mut amount = i32::from(damage.damage_lower_bound as u16);
        if damage.damage_delta > 0 {
            self.engine_sites.add(draw_site::DAMAGE_AMOUNT, 1);
            crate::engine_rng::log_draw(draw_site::DAMAGE_AMOUNT, self.rng.state);
            amount += ((u32::from(self.rng.next_u16()) * damage.damage_delta as u32) >> 16) as i32;
        }
        let v = [p.object.vx, p.object.vy, p.object.vz];
        let scale = if damage.flags & 1 != 0 {
            crate::explosions::velocity_scale(v, 0x100)
        } else {
            0x100
        };
        let amount = i32::from(((i32::from(amount as i16) * i32::from(scale)) >> 8) as i16);
        let magnitude = if damage.flags & 0x1000 != 0 {
            0
        } else {
            i32::from(((i32::from(damage.word_0e) * amount) >> 12) as i16).min(0x100)
        };
        let info = Some(crate::explosions::ExplosionInfo {
            unit: p.record_0a,
            attacker: p.owner_monster,
            attacker_id: usize::try_from(p.owner_monster)
                .ok()
                .and_then(|a| self.engine.monsters.get(a))
                .map_or(0, |m| m.identifier),
            velocity: [p.object.vx, p.object.vy, p.object.vz],
            flags: if proj.flags & 0x400_0000 != 0 { 2 } else { 0 },
            mana: mana.0,
            factor_bits: mana.1.to_bits(),
            source: crate::explosions::ExplosionSource::of(p),
        });
        let started = self.scenery_object_hit(tick, i, damage, info, amount, magnitude, point)?;
        self.detonate_on_object_group(p, proj, started, keep, tick)
    }

    #[allow(clippy::too_many_arguments)]
    fn detonate_on_object(
        &mut self,
        p: &mut crate::projectiles::FlightProjectile,
        proj: DetonateProjectile,
        damage: &mandrake_tag::projectile::DamageDefinition,
        victim: crate::explosions::VictimObject,
        keep: bool,
        tick: u32,
        mana: (i16, f32),
    ) -> Result<bool, Unmodelled> {
        let point = [p.object.x, p.object.y, p.object.z];
        let info = Some(crate::explosions::ExplosionInfo {
            unit: p.record_0a,
            attacker: p.owner_monster,
            attacker_id: usize::try_from(p.owner_monster)
                .ok()
                .and_then(|a| self.engine.monsters.get(a))
                .map_or(0, |m| m.identifier),
            velocity: [p.object.vx, p.object.vy, p.object.vz],
            flags: if proj.flags & 0x400_0000 != 0 { 2 } else { 0 },
            mana: mana.0,
            factor_bits: mana.1.to_bits(),
            source: crate::explosions::ExplosionSource::of(p),
        });
        let hit = crate::explosions::ObjectHit {
            definition: p.definition,
            damage: *damage,
            owner: p.owner,
            point,
            info,
            scale: 0x100,
            site: draw_site::DAMAGE_AMOUNT,
        };
        let started = self.object_hit(tick, victim, &hit)?;
        self.detonate_on_object_group(p, proj, started, keep, tick)
    }

    fn detonate_on_object_group(
        &mut self,
        p: &mut crate::projectiles::FlightProjectile,
        proj: DetonateProjectile,
        started: Option<FourCC>,
        keep: bool,
        tick: u32,
    ) -> Result<bool, Unmodelled> {
        let point = [p.object.x, p.object.y, p.object.z];
        let group = started.or(proj.detonation_group);
        if let Some(group) = group {
            let record_src = crate::projectile_groups::record_object(p).ok_or_else(|| {
                Unmodelled::new(
                    draw_site::DETONATION_GROUP_NO_OBJECT,
                    "detonation group without an object",
                )
            })?;
            let (flags, magnitude) = (
                if p.record_flags & 4 != 0 { 4 } else { 0 },
                i32::from(proj.group_scale.max(0)),
            );
            let desc = crate::projectile_groups::GroupDescriptor {
                flags,
                group,
                shading_object: record_src.index,
                object: record_src.index,
                owner: p.owner_monster,
                point: [
                    record_src.position[0],
                    record_src.position[1],
                    record_src.position[2]
                        + ((i32::from(record_src.bottom) + i32::from(record_src.top)) >> 1),
                ],
                magnitude,
                velocity: [0; 3],
            };
            let mut w = crate::projectile_groups::SimSpawn::of_record(self, p, tick);
            crate::projectile_groups::new_projectile_group(&mut w, &desc)?;
        }
        self.combat.events.push(CombatEvent::ProjectileDetonated {
            tick,
            projectile_id: p.id,
            definition: p.definition,
            owner: p.owner,
            hit_unit: None,
            position: self.world_units_to_sim(point),
            group,
        });
        if proj.flags & 0x1000_0000 != 0 {
            self.projectile_starts_fire(p)?;
        }
        if proj.flags & 0x800_0000 != 0 {
            self.release_owner_monster(p);
        }
        if keep {
            p.record_flags &= !1;
        } else if let Some(s) = p.shape.as_ref() {
            self.engine.free_objects.insert(s.index);
        }
        Ok(keep)
    }

    fn engine_cause_damage(
        &mut self,
        tick: u32,
        p: &crate::projectiles::FlightProjectile,
        proj: DetonateProjectile,
        damage: &mandrake_tag::projectile::DamageDefinition,
        unit: u32,
        mana: (i16, f32),
    ) -> Result<Option<Option<FourCC>>, Unmodelled> {
        let Some((time, mut actions)) = self.engine_hit.take().or_else(|| {
            let w = &mut self.engine;
            Some((tick as i32, w.map_actions.take()))
        }) else {
            return Err(Unmodelled::new(
                draw_site::CAUSE_DAMAGE_NO_ENGINE_MODEL,
                "no engine model",
            ));
        };
        let in_stage = self.engine_attack_launch;
        let r = self.engine_cause_damage_in_stage(
            tick,
            p,
            proj,
            damage,
            (unit, mana),
            time,
            &mut actions,
        );
        if in_stage {
            self.engine_hit = Some((time, actions));
        } else {
            self.engine.map_actions = actions;
        }
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn engine_cause_damage_in_stage(
        &mut self,
        tick: u32,
        p: &crate::projectiles::FlightProjectile,
        proj: DetonateProjectile,
        damage: &mandrake_tag::projectile::DamageDefinition,
        (unit, mana): (u32, (i16, f32)),
        time: i32,
        actions: &mut Option<crate::map_actions::MapActions>,
    ) -> Result<Option<Option<FourCC>>, Unmodelled> {
        let world = &self.engine;
        let Some(v) = world.monster_index_by_identifier(unit as u16) else {
            return Err(Unmodelled::new(
                draw_site::CAUSE_DAMAGE_NO_ENGINE_MODEL,
                format!("unit {unit} is no engine monster"),
            ));
        };
        let attacker_id = usize::try_from(p.owner_monster)
            .ok()
            .and_then(|a| world.monsters.get(a))
            .map_or(0, |m| m.identifier);
        let mut info =
            crate::engine_damage::DamageInfo::new(p.record_0a, p.owner_monster, attacker_id);
        info.point = [p.object.x, p.object.y, p.object.z];
        info.velocity = [p.object.vx, p.object.vy, p.object.vz];
        info.scale = p.record_0e;
        (info.mana, info.factor) = mana;
        if proj.flags & 0x400_0000 != 0 {
            info.flags = 2;
        }
        let hp_before = world.monsters[v].object.vitality;
        self.in_engine_stage(time, actions, v, |h, mv| {
            h.cause_damage(mv, damage, &mut info)
        })?;
        self.record_engine_hit_result(
            (tick, time),
            actions,
            (v, unit, hp_before),
            (p.owner, p.definition),
            damage,
            &info,
            Some(p.index),
        )?;
        Ok((info.flags & 1 != 0).then_some(info.group))
    }

    pub(crate) fn engine_explosion_hit(
        &mut self,
        tick: u32,
        unit: u32,
        owner: u32,
        definition: FourCC,
        damage: &mandrake_tag::projectile::DamageDefinition,
        mut hit: crate::engine_damage::DamageInfo,
    ) -> Result<Option<FourCC>, Unmodelled> {
        let Some(v) = self.engine.monster_index_by_identifier(unit as u16) else {
            return Ok(None);
        };
        let time = tick as i32;
        let mut actions = self.engine.map_actions.take();
        let r = (|| {
            let hp_before = self.engine.monsters[v].object.vitality;
            self.in_engine_stage(time, &mut actions, v, |h, mv| {
                h.accelerate_monster(mv, damage, &mut hit)
            })?;
            self.record_engine_hit_result(
                (tick, time),
                &mut actions,
                (v, unit, hp_before),
                (owner, definition),
                damage,
                &hit,
                None,
            )?;
            Ok((hit.flags & 1 != 0).then_some(hit.group).flatten())
        })();
        self.engine.map_actions = actions;
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn record_engine_hit_result(
        &mut self,
        (tick, time): (u32, i32),
        actions: &mut Option<crate::map_actions::MapActions>,
        (v, unit, hp_before): (usize, u32, i32),
        (owner, definition): (u32, FourCC),
        damage: &mandrake_tag::projectile::DamageDefinition,
        info: &crate::engine_damage::DamageInfo,
        record: Option<i32>,
    ) -> Result<(), Unmodelled> {
        let unit = self.engine.monsters[v].unit_key().unwrap_or(unit);
        let hp_after = self.engine.monsters[v].object.vitality;
        let flinch = self.engine.monsters[v].mode == 15;
        if let Some(u) = self.units.get_mut(&unit) {
            u.health = Fixed16::from_i32(hp_after);
        }
        self.combat.events.push(CombatEvent::UnitDamaged {
            tick,
            unit_id: unit,
            attacker: owner,
            definition,
            damage_type: damage.damage_type,
            amount: hp_before - hp_after,
            hp_before,
            hp_after,
            lethal: info.killed.is_some(),
        });
        if let Some(code) = info.killed {
            self.kill_engine_monster(
                (tick, time),
                actions,
                (v, unit),
                code,
                damage,
                (i32::from(info.amount), info.point),
                record,
            )?;
            self.combat.events.push(CombatEvent::UnitKilled {
                tick,
                unit_id: unit,
                killer: owner,
                animation_code: code,
            });
            self.units.remove(&unit);
        } else if flinch {
            self.combat.events.push(CombatEvent::HitReaction {
                tick,
                unit_id: unit,
            });
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn kill_engine_monster(
        &mut self,
        (tick, time): (u32, i32),
        actions: &mut Option<crate::map_actions::MapActions>,
        (v, unit): (usize, u32),
        code: u8,
        damage: &mandrake_tag::projectile::DamageDefinition,
        (amount, point): (i32, [i32; 3]),
        record: Option<i32>,
    ) -> Result<(), Unmodelled> {
        if self.engine.monsters[v].mode == 14 {
            self.engine.monsters[v].object.shading_slot = 0;
        }
        self.launch_death_group(tick, unit, code, damage, amount, Some(point), true, record)?;
        self.kill_monster_body_13(tick, unit, code, damage, amount, point, record)?;
        self.kill_monster_ammunition(tick, unit, code, damage, amount, point, record)?;
        self.kill_monster_carried(tick, unit, code, damage, amount, point, record)?;
        self.in_engine_stage(time, actions, v, |h, _| h.kill_monster())
    }

    fn in_engine_stage<R>(
        &mut self,
        time: i32,
        actions: &mut Option<crate::map_actions::MapActions>,
        v: usize,
        f: impl FnOnce(
            &mut crate::engine_rng::WorldHooks<'_, '_>,
            Option<&mut crate::engine_move::EngineMover>,
        ) -> Result<R, Unmodelled>,
    ) -> Result<R, Unmodelled> {
        self.lend_movers();
        let mut world = std::mem::take(&mut self.engine);
        let mut rng_state = std::mem::take(&mut self.rng);
        let mut sites = std::mem::take(&mut self.engine_sites);
        world.projectile_records = std::mem::take(&mut self.combat.flight.flights);
        let mut solid = std::mem::take(&mut world.solid_elsewhere);
        solid.clear();
        solid.extend(world.solid_views(std::iter::empty(), &self.combat.flight.dormant));
        world.solid_elsewhere = solid;
        let mut mv = world.movers.get_mut(v).and_then(Option::take);
        let r = {
            let mut rng = crate::engine_rng::EngineRng {
                rng: &mut rng_state,
                sites: &mut sites,
            };
            let mut hooks = crate::engine_rng::WorldHooks {
                world: &mut world,
                actions,
                rng: &mut rng,
                monster: v,
                time,
                host: self,
            };
            f(&mut hooks, mv.as_mut())
        };
        if let Some(slot) = world.movers.get_mut(v)
            && world.monsters[v].identifier != 0
        {
            *slot = mv;
        }
        self.combat.flight.flights = std::mem::take(&mut world.projectile_records);
        self.rng = rng_state;
        self.engine_sites = sites;
        self.engine = world;
        self.sync_solid_objects();
        self.return_movers();
        r
    }

    pub(crate) fn stain_mesh(
        &mut self,
        tick: u32,
        collection: FourCC,
        sequence: i16,
        point: [i32; 3],
    ) {
        self.engine_sites.add(draw_site::MESH_STAIN, 1);
        crate::engine_rng::log_draw(draw_site::MESH_STAIN, self.rng.state);
        let random = self.rng.next_u16();
        let stain = Stain {
            collection,
            sequence,
            point,
            random,
        };
        self.combat.flight.stains.push(stain);
        self.combat
            .events
            .push(CombatEvent::StainMesh { tick, stain });
    }

    pub fn replace_record(
        &mut self,
        p: &crate::projectiles::FlightProjectile,
        tick: u32,
    ) -> Result<(), Unmodelled> {
        let depth = self.combat.flight.replace_depth;
        let run = depth < 0x100 && !self.combat.flight.replace_overflow;
        if depth >= 0x100 {
            self.combat.flight.replace_overflow = true;
        }
        if run {
            self.combat.flight.replace_depth = depth + 1;
            let r = self.replace_with(p, tick);
            self.combat.flight.replace_depth = depth;
            r?;
        }
        if depth == 0 {
            self.combat.flight.replace_overflow = false;
        }
        if let Some(s) = p.shape.as_ref() {
            self.engine.free_objects.insert(s.index);
        }
        Ok(())
    }

    fn replace_with(
        &mut self,
        p: &crate::projectiles::FlightProjectile,
        tick: u32,
    ) -> Result<(), Unmodelled> {
        let Some((group_0, projectile_5c, unit_90)) = self
            .engine
            .catalog
            .projectiles
            .get(&p.definition)
            .map(|proj| {
                let next = proj
                    .raw
                    .get(0x5c..0x60)
                    .is_some_and(|b| b != [0, 0, 0, 0])
                    .then_some(proj.projectile_5c)
                    .flatten();
                let unit = proj
                    .raw
                    .get(0x90..0x94)
                    .and_then(|b| <[u8; 4]>::try_from(b).ok())
                    .filter(|b| *b != [0, 0, 0, 0] && *b != [0xff; 4]);
                (proj.groups[0], next, unit)
            })
        else {
            return Err(Unmodelled::new(
                draw_site::REPLACE_PROJECTILE_MISSING,
                format!("proj {} not loaded", p.definition),
            ));
        };
        let src = crate::projectile_groups::record_object(p);
        let pos = [p.object.x, p.object.y, p.object.z];
        let v = [p.object.vx, p.object.vy, p.object.vz];
        let object = src.map_or(-1, |s| s.index);
        if let Some(group) = group_0 {
            let speed = crate::projectiles::isqrt(v.iter().fold(0u32, |s, &c| {
                s.wrapping_add((i32::from(c) * i32::from(c)) as u32)
            }));
            let desc = crate::projectile_groups::GroupDescriptor {
                flags: 1,
                group,
                shading_object: object,
                object,
                owner: p.owner_monster,
                point: pos,
                magnitude: i32::from(speed as i16),
                velocity: v,
            };
            crate::projectile_groups::new_projectile_group(
                &mut crate::projectile_groups::SimSpawn::of_record(self, p, tick),
                &desc,
            )?;
        }
        if let Some(next) = projectile_5c {
            let target = p
                .target
                .and_then(|t| self.engine.monster_index_by_identifier(t as u16))
                .map_or(-1, |i| i as i16);
            let desc = crate::projectile_groups::ProjectileDescriptor {
                definition: next,
                flags: 0,
                position: pos,
                velocity: v,
                facing: p.facing,
                elevation: p.elevation,
                word_1c: 0x100,
                owner_monster: -1,
                parent: p.index as i16,
                owner_object: object,
                target,
                target_point: [0; 3],
            };
            crate::projectile_groups::new_projectile(
                &mut crate::projectile_groups::SimSpawn::of_record(self, p, tick),
                &desc,
            )?;
        }
        let gameplay = self.engine.formation_globals.gameplay_version;
        if gameplay > game_options::GAMEPLAY_1_6
            && p.record_36 != -1
            && let Some(unit) = unit_90
        {
            self.replacement_monster(unit, p)?;
        }
        Ok(())
    }

    fn replacement_monster(
        &mut self,
        unit: [u8; 4],
        p: &crate::projectiles::FlightProjectile,
    ) -> Result<(), Unmodelled> {
        self.sync_engine_positions();
        self.lend_objects();
        let w = &mut self.engine;
        w.reserved_records.push(p.index);
        let r = w.replacement_monster(
            unit,
            p,
            &mut crate::engine_rng::EngineRng {
                rng: &mut self.rng,
                sites: &mut self.engine_sites,
            },
        );
        w.reserved_records.pop();

        self.return_objects();
        r?;
        self.attach_shown_monsters();
        Ok(())
    }
}

fn velocity_angles(v: [i16; 3], facing: u16) -> (u16, u16) {
    let heading = if v[0] == 0 && v[1] == 0 {
        facing
    } else {
        arctangent(i32::from(v[0]), i32::from(v[1]))
    };
    let h = (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2)).sqrt() as i32;
    (heading, arctangent(h, i32::from(v[2])))
}

struct SimUpdate<'a> {
    sim: &'a mut Simulation,
    tick: u32,
    pending: Option<&'a mut CollisionList>,
}

impl SimUpdate<'_> {
    fn force(&mut self) {
        if let Some(list) = self.pending.take() {
            list.pending = false;
            self.sim.refresh_collision_objects(list);
        }
    }
}

impl crate::projectiles::UpdateWorld for SimUpdate<'_> {
    fn chance(&mut self, _site: u32, threshold: u16) -> bool {
        crate::engine_rng::log_draw(FREQUENCY_DRAW_SITE, self.sim.rng.state);
        self.sim.engine_sites.add(FREQUENCY_DRAW_SITE, 1);
        frequency_corrected(&mut self.sim.rng, threshold, COLLISION_CHANCE_VERSION_CUT)
    }
    fn draw(&mut self, site: u32) -> u16 {
        self.sim.engine_sites.add(site, 1);
        crate::engine_rng::log_draw(site, self.sim.rng.state);
        self.sim.rng.next_u16()
    }
    fn stain(&mut self, collection: FourCC, sequence: i16, point: [i32; 3]) {
        self.force();
        self.sim.stain_mesh(self.tick, collection, sequence, point);
    }
    fn trail(
        &mut self,
        p: &crate::projectiles::FlightProjectile,
        desc: &crate::projectile_groups::ProjectileDescriptor,
    ) -> Result<(), Unmodelled> {
        self.force();
        let mut w = crate::projectile_groups::SimSpawn::of_record(self.sim, p, self.tick);
        crate::projectile_groups::new_projectile(&mut w, desc).map(|_| ())
    }
    fn game_options_high(&self) -> u16 {
        self.sim.game_options_high
    }
    fn media_group(
        &mut self,
        p: &crate::projectiles::FlightProjectile,
        t: usize,
    ) -> Result<(), Unmodelled> {
        self.force();
        crate::projectile_groups::new_media_group(self.sim, self.tick, p, t)
    }
    fn target_monster_position(&self, target: u32) -> Option<(i32, i32)> {
        let engine = &self.sim.engine;
        if let Some(idx) = engine.monster_index_by_identifier(target as u16) {
            return Some(engine.monsters[idx].position);
        }
        self.sim
            .units
            .get(&target)
            .map(|u| self.sim.world_to_command_point(u.position))
    }
    fn lightning(&self, link: (i16, i16)) -> Option<crate::projectiles::LightningView> {
        let w = &self.sim.engine;
        let r = w.lightning.get(link.0, link.1)?;
        let flags = w.catalog.lightnings.get(&r.system).map_or(0, |d| d.flags);
        Some(crate::projectiles::LightningView {
            head: r.head,
            end: r.end,
            target: r.target.0,
            target_alive: r.target.0 != -1 && w.lightning_object_alive(r.target.0, r.target.1),
            state: r.state,
            flags,
        })
    }
    fn lightning_ground_hit(&mut self, index: i16) {
        self.force();
        {
            let w = &mut self.sim.engine;
            w.lightning_ground_hit(index);
        }
    }
    fn lightning_object_hit(&mut self, index: i16, object: i32) {
        self.force();
        {
            let w = &mut self.sim.engine;
            w.lightning_object_hit(index, object as i16);
        }
    }
}

#[derive(Default)]
pub(crate) struct CollisionList {
    pub(crate) sorted: Vec<CollisionObject>,
    cells: crate::projectiles::ObjectCells,
    unsorted: Vec<CollisionObject>,
    keys: Vec<u64>,
    spare: Vec<CollisionObject>,
    spare_keys: Vec<u64>,
    ids: Vec<(u32, (i32, i32))>,
    tables: CollisionTables,
    valid: bool,
    pending: bool,
}

#[derive(Default)]
struct CollisionTables {
    stamp: u32,
    slots: Vec<(u32, u32)>,
    links: Vec<(u32, u64)>,
    other_links: std::collections::HashMap<i32, u64>,
    scenery: SceneryLinks,
    id_bits: Vec<u64>,
    unit_xy: Vec<(i32, i32)>,
}

impl CollisionTables {
    const IDS: usize = 1 << 16;

    fn living_units(
        &mut self,
        living: impl Iterator<Item = (u32, (i32, i32))>,
        ids: &mut Vec<(u32, (i32, i32))>,
    ) {
        ids.clear();
        self.id_bits.resize(Self::IDS / 64, 0);
        self.unit_xy.resize(Self::IDS, (0, 0));
        let mut large = false;
        let mut words = 0;
        for (id, xy) in living {
            match self.id_bits.get_mut(id as usize / 64) {
                Some(w) => {
                    *w |= 1 << (id % 64);
                    self.unit_xy[id as usize] = xy;
                    words = words.max(id as usize / 64 + 1);
                }
                None => {
                    large = true;
                    ids.push((id, xy));
                }
            }
        }
        let first_large = ids.len();
        for (k, w) in self.id_bits[..words].iter_mut().enumerate() {
            let mut bits = std::mem::take(w);
            while bits != 0 {
                let id = k as u32 * 64 + bits.trailing_zeros();
                ids.push((id, self.unit_xy[id as usize]));
                bits &= bits - 1;
            }
        }
        if large {
            ids.rotate_left(first_large);
            ids.sort_unstable_by_key(|e| e.0);
        }
    }

    fn begin(&mut self) {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.slots.clear();
            self.links.clear();
            self.stamp = 1;
        }
        self.other_links.clear();
    }

    fn index_monsters(&mut self, monsters: &[crate::engine_rng::EngineMonster]) {
        self.slots.resize(Self::IDS, (0, 0));
        for (i, m) in monsters.iter().enumerate() {
            if let Some(key) = m.unit_key() {
                let e = &mut self.slots[key as usize];
                if e.0 != self.stamp {
                    *e = (self.stamp, i as u32);
                }
                self.set_link(m.object.index, m.cell_link);
            }
        }
    }

    fn monster_slot(&self, id: u32) -> Option<usize> {
        self.slots
            .get(id as usize)
            .filter(|e| e.0 == self.stamp)
            .map(|e| e.1 as usize)
    }

    fn set_link(&mut self, index: i32, link: u64) {
        match usize::try_from(index).ok().filter(|&i| i < Self::IDS) {
            Some(i) => {
                if i >= self.links.len() {
                    self.links.resize((i + 1).next_power_of_two(), (0, 0));
                }
                self.links[i] = (self.stamp, link);
            }
            None => {
                self.other_links.insert(index, link);
            }
        }
    }

    fn scenery_links(
        &mut self,
        cells: std::sync::Arc<crate::engine_rng::SceneryCells>,
        scenery: &[crate::engine_rng::EngineScenery],
    ) {
        if !self
            .scenery
            .made_for
            .as_ref()
            .is_some_and(|c| std::sync::Arc::ptr_eq(c, &cells))
        {
            self.scenery = SceneryLinks::new(scenery);
            self.scenery.made_for = Some(cells);
        }
        #[cfg(feature = "identity-checks")]
        {
            let fresh = SceneryLinks::new(scenery);
            assert!(fresh.links == self.scenery.links && fresh.others == self.scenery.others);
        }
    }

    fn link(&self, index: i32) -> u64 {
        match usize::try_from(index).ok().filter(|&i| i < Self::IDS) {
            Some(i) => match self.links.get(i).filter(|e| e.0 == self.stamp) {
                Some(e) => e.1,
                None => self.scenery.links.get(i).copied().unwrap_or(0),
            },
            None => self
                .other_links
                .get(&index)
                .or_else(|| self.scenery.others.get(&index))
                .copied()
                .unwrap_or(0),
        }
    }
}

#[derive(Default)]
struct SceneryLinks {
    made_for: Option<std::sync::Arc<crate::engine_rng::SceneryCells>>,
    links: Vec<u64>,
    others: std::collections::HashMap<i32, u64>,
}

impl SceneryLinks {
    fn new(scenery: &[crate::engine_rng::EngineScenery]) -> Self {
        let mut out = Self::default();
        for s in scenery {
            match usize::try_from(s.object.index)
                .ok()
                .filter(|&i| i < CollisionTables::IDS)
            {
                Some(i) => {
                    if i >= out.links.len() {
                        out.links.resize(i + 1, 0);
                    }
                    out.links[i] = s.cell_link;
                }
                None => {
                    out.others.insert(s.object.index, s.cell_link);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod history_map_tests {
    use super::{ObjectHistory, ObjectHistoryMap};
    use mandrake_common::FourCC;
    use std::collections::BTreeMap;

    fn copy(n: i32) -> ObjectHistory {
        ObjectHistory {
            position: [n, 0, 0],
            velocity: [0; 3],
            facing: 0,
            elevation: 0,
            scale: 0,
            collection: FourCC::new(*b"test"),
            sequence: 0,
            permutation: 0,
            shading_slot: 0,
            core: None,
            frame: 0,
            view: 0,
        }
    }

    #[test]
    fn the_kept_table_is_the_map_it_replaced() {
        let mut ours = ObjectHistoryMap::default();
        let mut map: BTreeMap<i32, ObjectHistory> = BTreeMap::new();
        let mut seed = 1u32;
        let mut next = |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 8) % n
        };
        for tick in 0..50 {
            ours.clear();
            map.clear();
            for step in 0..200 {
                let index = match next(10) {
                    0 => 70_000 + next(4) as i32,
                    1 => -1 - next(3) as i32,
                    _ => next(120) as i32,
                };
                let h = copy(tick * 1000 + step);
                match next(3) {
                    0 => {
                        ours.put(index, true, h);
                        map.insert(index, h);
                    }
                    1 => {
                        ours.put(index, false, h);
                        map.entry(index).or_insert(h);
                    }
                    _ => assert_eq!(ours.remove(&index), map.remove(&index)),
                }
                assert_eq!(ours.len(), map.len());
            }
            for index in -5..130 {
                assert_eq!(ours.get(&index), map.get(&index), "{index}");
            }
            assert_eq!(format!("{ours:?}"), format!("{map:?}"));
            assert_eq!(format!("{ours:#?}"), format!("{map:#?}"));
        }
    }

    #[test]
    fn homing_target_position_does_not_fall_back_to_monster_slot() {
        use super::SimUpdate;
        use crate::Simulation;
        use crate::projectiles::UpdateWorld;
        let mut sim = Simulation::new(1);
        let mut w = crate::collide::tests::world((5_000, 6_000), (9_000, 9_000));
        w.monsters[0].identifier = 42;
        sim.set_engine_world(w);
        let u = SimUpdate {
            sim: &mut sim,
            tick: 0,
            pending: None,
        };
        assert_eq!(u.target_monster_position(42), Some((5_000, 6_000)));
        assert_eq!(u.target_monster_position(0), None);
    }

    #[test]
    fn dropped_flight_record_frees_its_object_index() {
        use crate::Simulation;
        use crate::projectiles::{
            FlightDefinition, FlightProjectile, FlightTerrain, LaunchDescriptor,
        };
        let mut sim = Simulation::new(1);
        let w = crate::collide::tests::world((5_000, 6_000), (9_000, 9_000));
        let mut shape = w.monsters[0].object.clone();
        shape.index = 77;
        sim.set_engine_world(w);
        sim.combat.flight.terrain = Some(FlightTerrain::flat(16, 16, 0));
        let proj =
            mandrake_tag::ProjectileDefinition::from_bytes(FourCC::new(*b"miss"), &vec![0u8; 256])
                .unwrap();
        let def = FlightDefinition::new(
            &proj,
            mandrake_tag::object::ObjectPhysics::default(),
            mandrake_tag::SequenceWorldBounds::default(),
        );
        let desc = LaunchDescriptor {
            flags: 0,
            position: [0, 0, 0],
            velocity: [0, 0, 0],
            facing: 0,
            elevation: 0,
            owner_object: -1,
        };
        let mut p = FlightProjectile::launch(1, &def, 0, None, &desc, 0, &mut |_| 0).unwrap();
        p.shape = Some(shape);
        sim.combat.flight.flights.push(Box::new(p));
        sim.update_projectile_flight(0);
        assert!(sim.engine.free_objects.contains(&77));
    }
}
