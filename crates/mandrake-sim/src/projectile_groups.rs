use mandrake_common::FourCC;
use mandrake_tag::object::ObjectPhysics;
use mandrake_tag::{
    MeshHeightMode, ObjectDefinition, ProjectileDefinition, ProjectileGroupDefinition,
    SequenceHeader,
};

use crate::combat::MINIMUM_GAMEPLAY_VERSION;
use crate::draw_site;
use crate::engine_rng::{EngineObject, EngineRng, TagSource, Unmodelled, set_object_shape_header};
use crate::projectiles::{
    CollisionObject, FlightDefinition, FlightObject, FlightProjectile, isqrt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectSnapshot {
    pub index: i32,
    pub position: [i32; 3],
    pub velocity: [i16; 3],
    pub facing: u16,
    pub elevation: u16,
    pub radius: i16,
    pub bottom: i16,
    pub top: i16,
    pub kind: u8,
    pub owner: i16,
    pub collection: FourCC,
    pub sequence: u8,
    pub frame: i8,
    pub view: u8,
    pub shading: i16,
    pub shading_slot: u8,
    pub core: Option<FourCC>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterSnapshot {
    pub identifier: i32,
    pub object: i32,
    pub word_14: crate::statistics::UnitRef,
    pub byte_17: i8,
    pub flags: u32,
    pub flags_8: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSnapshot {
    pub word_0e: i16,
    pub word_0a: crate::statistics::UnitRef,
    pub owner_monster: i16,
    pub word_36: i16,
    pub flags: u16,
    pub target: Option<u32>,
}

impl RecordSnapshot {
    pub fn of(p: &FlightProjectile) -> Self {
        Self {
            word_0e: p.record_0e,
            word_0a: p.record_0a,
            owner_monster: p.owner_monster,
            word_36: p.record_36,
            flags: p.record_flags,
            target: p.target,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatedProjectile {
    pub definition: FourCC,
    pub object: i32,
    pub identifier: i16,
    pub point: [i32; 3],
    pub facing: u16,
    pub velocity: [i16; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupDescriptor {
    pub flags: u32,
    pub group: FourCC,
    pub shading_object: i32,
    pub object: i32,
    pub owner: i16,
    pub point: [i32; 3],
    pub magnitude: i32,
    pub velocity: [i16; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectileDescriptor {
    pub definition: FourCC,
    pub flags: u16,
    pub position: [i32; 3],
    pub velocity: [i16; 3],
    pub facing: u16,
    pub elevation: u16,
    pub word_1c: i16,
    pub owner_monster: i16,
    pub parent: i16,
    pub owner_object: i32,
    pub target: i16,
    pub target_point: [i32; 3],
}

#[derive(Debug, Clone, Default)]
pub struct ProjectileCatalog {
    pub projectiles: crate::entities::TagIdMap<ProjectileDefinition>,
    pub flights: crate::entities::TagIdMap<std::sync::Arc<FlightDefinition>>,
    pub groups: crate::entities::TagIdMap<ProjectileGroupDefinition>,
    pub objects: crate::entities::TagIdMap<ObjectDefinition>,
    pub mesh_effects: crate::entities::TagIdMap<crate::mesh_effects::MeshEffectDefinition>,
    pub lightnings: crate::entities::TagIdMap<crate::lightning::LightningDefinition>,
    pub artifacts: Vec<(FourCC, Vec<u8>)>,
    pub media_groups: Option<[Option<FourCC>; mandrake_tag::media::MEDIA_PROJECTILE_GROUP_COUNT]>,
}

impl ProjectileCatalog {
    pub fn load_media_groups(&mut self, tags: &dyn TagSource, media_tag: FourCC) {
        self.media_groups = tags
            .tag(*b"medi", media_tag)
            .and_then(|d| mandrake_tag::media::MediaDefinition::from_bytes(media_tag, &d).ok())
            .map(|m| m.projectile_groups);
    }

    pub fn add_group(
        &mut self,
        tags: &dyn TagSource,
        headers: &mut dyn FnMut(FourCC) -> Option<Vec<SequenceHeader>>,
        id: FourCC,
    ) {
        if self.groups.contains_key(&id) {
            return;
        }
        let Some(g) = tags
            .tag(*b"prgr", id)
            .and_then(|d| ProjectileGroupDefinition::from_bytes(id, &d).ok())
        else {
            return;
        };
        let parts: Vec<FourCC> = g
            .parts
            .iter()
            .flat_map(|p| [p.projectile, p.projectile_18])
            .flatten()
            .collect();
        self.groups.insert(id, g.clone());
        if let Some(effect) = g.mesh_effect {
            self.add_mesh_effect(tags, effect);
        }
        for p in parts {
            self.add_projectile(tags, headers, p);
        }
    }

    pub fn add_mesh_effect(&mut self, tags: &dyn TagSource, id: FourCC) {
        if self.mesh_effects.contains_key(&id) {
            return;
        }
        let Some(data) = tags.tag(*b"meef", id) else {
            return;
        };
        let Some(def) = crate::mesh_effects::MeshEffectDefinition::from_bytes(id, &data) else {
            return;
        };
        self.mesh_effects.insert(id, def);
    }

    pub fn add_projectile(
        &mut self,
        tags: &dyn TagSource,
        headers: &mut dyn FnMut(FourCC) -> Option<Vec<SequenceHeader>>,
        id: FourCC,
    ) {
        if self.projectiles.contains_key(&id) {
            return;
        }
        let Some(p) = tags
            .tag(*b"proj", id)
            .and_then(|d| ProjectileDefinition::from_bytes(id, &d).ok())
        else {
            return;
        };
        self.projectiles.insert(id, p.clone());
        if let Some(l) = lightning_tag(&p)
            && !self.lightnings.contains_key(&l)
            && let Some(def) = tags
                .tag(*b"ligh", l)
                .and_then(|d| crate::lightning::LightningDefinition::from_bytes(l, &d))
        {
            self.lightnings.insert(l, def);
        }
        if let Some(o) = p.object
            && let Some(d) = tags.tag(*b"obje", o)
            && let (Ok(def), Ok(physics)) = (
                ObjectDefinition::from_bytes(o, &d),
                ObjectPhysics::from_bytes(&d),
            )
        {
            self.objects.insert(o, def);
            let bounds = p
                .collection
                .and_then(&mut *headers)
                .and_then(|h| mandrake_tag::get_sequence(&h, i32::from(p.sequences[0])).copied())
                .map(|h| h.world_bounds)
                .unwrap_or_default();
            self.flights.insert(
                id,
                std::sync::Arc::new(FlightDefinition::new(&p, physics, bounds)),
            );
        }
        for g in [p.detonation_group].into_iter().chain(p.groups).flatten() {
            self.add_group(tags, headers, g);
        }
        for q in [p.projectile_14, p.projectile_5c].into_iter().flatten() {
            self.add_projectile(tags, headers, q);
        }
    }
}

pub fn lightning_tag(p: &ProjectileDefinition) -> Option<FourCC> {
    if p.flags & 0x0100_0000 == 0 {
        return None;
    }
    let b: [u8; 4] = p.raw.get(0x34..0x38)?.try_into().ok()?;
    match u32::from_be_bytes(b) {
        0 | u32::MAX => None,
        _ => Some(FourCC::new(b)),
    }
}

pub const MAXIMUM_PROJECTILES: usize = 0x400;

fn lowest_free_record(used: impl Iterator<Item = i32> + Clone) -> i32 {
    const MARKS: usize = 4 * MAXIMUM_PROJECTILES;
    let mut marks = [0u64; MARKS / 64];
    let mut n = 0;
    for i in used.clone() {
        n += 1;
        if let Some(i) = usize::try_from(i).ok().filter(|&i| i < MARKS) {
            marks[i / 64] |= 1 << (i % 64);
        }
    }
    if n < MARKS {
        let w = marks
            .iter()
            .position(|&w| w != u64::MAX)
            .unwrap_or(MARKS / 64);
        return (w * 64 + marks.get(w).map_or(0, |w| w.trailing_ones() as usize)) as i32;
    }
    let used: Vec<i32> = used.collect();
    let mut marks = vec![false; used.len() + 1];
    for i in used {
        if let Some(m) = usize::try_from(i).ok().and_then(|i| marks.get_mut(i)) {
            *m = true;
        }
    }
    marks.iter().position(|&m| !m).unwrap_or(marks.len()) as i32
}

fn used_records<'a, I: Iterator<Item = i32> + Clone>(
    lists: impl IntoIterator<Item = &'a crate::record_list::UsedIndexes>,
    extra: Option<i32>,
    walk: impl Fn() -> I,
) -> (usize, i32) {
    let kept = crate::record_list::UsedSet::of(lists, extra).map(|s| (s.count(), s.lowest_free()));
    let walked = || (distinct_records(walk()), lowest_free_record(walk()));
    match kept {
        Some(k) => {
            #[cfg(feature = "identity-checks")]
            assert_eq!(k, walked(), "kept record indexes differ from the lists");
            k
        }
        None => walked(),
    }
}

fn distinct_records(used: impl Iterator<Item = i32>) -> usize {
    let mut marks = [0u64; MAXIMUM_PROJECTILES / 64];
    let mut others: Vec<i32> = Vec::new();
    let mut n = 0;
    for i in used {
        match usize::try_from(i).ok().filter(|&i| i < MAXIMUM_PROJECTILES) {
            Some(i) => {
                let (w, b) = (i / 64, 1u64 << (i % 64));
                if marks[w] & b == 0 {
                    marks[w] |= b;
                    n += 1;
                }
            }
            None => others.push(i),
        }
    }
    others.sort_unstable();
    others.dedup();
    n + others.len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellMonster {
    pub monster: i16,
    pub flags: u32,
    pub position: [i32; 2],
}

pub fn search_target(w: &dyn ProjectileWorld, def: &ProjectileDefinition, point: [i32; 3]) -> i16 {
    search_target_cells(
        w.mesh_size(),
        def.word(0x8c),
        def.word(0x8e),
        point,
        |x, y| w.cell_monsters(x, y),
        |_, _| {},
    )
}

pub fn search_target_cells(
    (width, height): (i32, i32),
    radius: i16,
    hole_raw: i16,
    point: [i32; 3],
    cell: impl Fn(i32, i32) -> Vec<CellMonster>,
    mut visit: impl FnMut(CellMonster, i32),
) -> i16 {
    let hole = if hole_raw < 0 { 0 } else { hole_raw };
    let mut half = radius;
    let side = if half <= 0 {
        half = 4;
        9i16
    } else {
        half.wrapping_mul(2).wrapping_add(1)
    };
    let rows = side.min(height as i16);
    let cols = side.min(width as i16);
    let dx = ((point[0] >> 9) as i16).wrapping_sub(half);
    let x0 = if dx < 0 {
        0
    } else {
        (width - i32::from(cols)).min(i32::from(dx)) as i16
    };
    let dy = ((point[1] >> 9) - i32::from(half)) as i16;
    let y0 = if dy < 0 {
        0
    } else {
        (height - i32::from(rows)).min(i32::from(dy)) as i16
    };
    let lo = half.wrapping_sub(hole);
    let hi = half.wrapping_add(hole);
    let mut best: i16 = -1;
    let mut best_distance = 0;
    for j in 0..rows {
        for i in 0..cols {
            if hole != 0 && lo <= i && i < hi && lo <= j && j < hi {
                continue;
            }
            let x = i32::from(x0) + i32::from(i);
            let y = i32::from(y0) + i32::from(j);
            for o in cell(x, y) {
                if o.flags & 0x200_0200 != 0 {
                    continue;
                }
                let d = crate::engine_move::distance2d(
                    point[0] - o.position[0],
                    point[1] - o.position[1],
                );
                visit(o, d);
                if best == -1 || d < best_distance {
                    best_distance = d;
                    best = o.monster;
                }
            }
        }
    }
    best
}

pub trait ProjectileWorld {
    fn catalog(&self) -> &ProjectileCatalog;
    fn sequence_header(&self, collection: FourCC, sequence: i32) -> Option<SequenceHeader>;
    fn rng(&mut self) -> EngineRng<'_>;
    fn object(&self, index: i32) -> Option<ObjectSnapshot>;
    fn monster(&self, index: i16) -> Option<MonsterSnapshot>;
    fn monster_middle(&self, index: i16) -> Option<[i32; 3]>;
    fn mark_target_object(&mut self, identifier: u32);
    fn cell_monsters(&self, x: i32, y: i32) -> Vec<CellMonster>;
    fn mesh_size(&self) -> (i32, i32);
    fn record(&self, index: i16) -> Option<RecordSnapshot>;
    fn mesh_cell(&self, x: i32, y: i32) -> (u16, i16);
    fn mesh_height(&self, x: i32, y: i32, mode: MeshHeightMode) -> i32;
    fn model_height(&self, x: i32, y: i32) -> Result<Option<i16>, Unmodelled>;
    fn collection_table0_core(&self, collection: FourCC) -> Option<FourCC>;
    fn game_options_high(&self) -> u16;
    fn new_object(
        &mut self,
        obje: &ObjectDefinition,
        collection: FourCC,
        scale: i32,
        position: [i32; 3],
    ) -> EngineObject;
    fn projectile_count(&self) -> usize;
    fn add_projectile(&mut self, p: FlightProjectile) -> i32;
    fn projectile_mut(&mut self, index: i32) -> Option<&mut FlightProjectile>;
    fn engine_model(&self) -> bool;
    fn group_launched(&mut self, desc: &GroupDescriptor, media_height: i16);
    fn object_request_flags(&self, index: i32) -> u16;
    fn object_identifier(&self, index: i32) -> i16;
    fn group_event(&mut self, event: crate::CombatEvent);
    fn projectile_created(&mut self, index: i32, created: CreatedProjectile);
    fn wind_velocity(&self) -> Option<[i16; 3]>;
    fn particle_system_intensity(&self, point: [i32; 3]) -> Result<u16, Unmodelled>;
    fn detonate_at_creation(&mut self, index: i32) -> Result<(), Unmodelled>;
    fn delete_record(&mut self, index: i32);
    fn new_mesh_effect(&mut self, effect: FourCC, x: i32, y: i32);
    fn monster_index(&self, unit: u32) -> Option<i16>;
    fn new_lightning(
        &mut self,
        system: Option<FourCC>,
        source: i16,
        target: i16,
        origin: [i32; 3],
        end: [i32; 3],
    ) -> Option<(i16, i16)>;
}

pub fn line_segment_to_vector(from: [i32; 3], to: [i32; 3], magnitude: i32) -> [i16; 3] {
    let d = [from[0] - to[0], from[1] - to[1], from[2] - to[2]];
    let sum = d[0]
        .wrapping_mul(d[0])
        .wrapping_add(d[1].wrapping_mul(d[1]))
        .wrapping_add(d[2].wrapping_mul(d[2])) as u32;
    let len = isqrt(sum).max(1);
    d.map(|c| (c.wrapping_mul(magnitude) / len) as i16)
}

struct PartSites {
    height: u32,
    y: u32,
    x: u32,
    countdown: u32,
}

const PART_SITES: PartSites = PartSites {
    height: draw_site::AMMUNITION_PART_HEIGHT,
    y: draw_site::AMMUNITION_PART_Y,
    x: draw_site::AMMUNITION_PART_X,
    countdown: draw_site::AMMUNITION_PART_COUNTDOWN,
};
const SECOND_SITES: PartSites = PartSites {
    height: draw_site::LINE_GROUP_HEIGHT,
    y: draw_site::LINE_GROUP_Y,
    x: draw_site::LINE_GROUP_X,
    countdown: draw_site::LINE_GROUP_COUNTDOWN,
};

#[derive(Clone, Copy)]
struct GroupPartFields {
    projectile: Option<FourCC>,
    projectile_18: Option<FourCC>,
    count: (i16, i16),
    threshold: u16,
    flags: u32,
    height: (i16, i16),
}

impl GroupPartFields {
    fn of(part: &mandrake_tag::ProjectileGroupPart) -> Self {
        Self {
            projectile: part.projectile,
            projectile_18: part.projectile_18,
            count: part.count(),
            threshold: part.threshold(),
            flags: part.flags(),
            height: part.height(),
        }
    }
}

pub fn new_projectile_group(
    w: &mut dyn ProjectileWorld,
    desc: &GroupDescriptor,
) -> Result<(), Unmodelled> {
    let Some((mesh_effect, sound, local_group, part_count)) = w
        .catalog()
        .groups
        .get(&desc.group)
        .map(|g| (g.mesh_effect, g.sound, g.local_group, g.parts.len()))
    else {
        return Err(Unmodelled::new(
            draw_site::GROUP_NOT_LOADED,
            format!("prgr {} not loaded", desc.group),
        ));
    };
    let source_obj = w.object(desc.object);
    if desc.flags & 0x10 == 0
        && let Some(effect) = mesh_effect
    {
        let (x, y) = if let Some(o) = source_obj {
            (o.position[0], o.position[1])
        } else {
            (desc.point[0], desc.point[1])
        };
        w.new_mesh_effect(effect, x, y);
    }
    let media_height = w.mesh_cell(desc.point[0], desc.point[1]).1;
    w.group_launched(desc, media_height);
    if desc.flags & 0x10 == 0
        && let Some(sound) = sound
        && let Some(o) = source_obj
    {
        let request_flags = w.object_request_flags(desc.object);
        w.group_event(crate::CombatEvent::ProjectileGroupSound {
            tick: 0,
            sound,
            world: o.position,
            request_flags,
            object: desc.object,
            identifier: w.object_identifier(desc.object),
            velocity: o.velocity,
        });
    }
    if let Some(local_group) = local_group
        && let Some(o) = source_obj
    {
        let mut point = o.position;
        point[2] += (i32::from(o.bottom) + i32::from(o.top)) / 2;
        w.group_event(crate::CombatEvent::ProjectileGroupLocal {
            tick: 0,
            local_group,
            point,
            facing: o.facing,
            object_point: o.position,
            cylinder: [o.radius, o.bottom, o.top],
        });
    }
    for part_idx in 0..part_count {
        let part = GroupPartFields::of(&w.catalog().groups[&desc.group].parts[part_idx]);
        let (base, delta) = part.count;
        let mut n = i32::from(base);
        if delta > 0 {
            n += (i32::from(w.rng().draw(draw_site::GROUP_COUNT)) * i32::from(delta)) >> 16;
        }
        if n < 0 {
            return Err(Unmodelled::new(
                draw_site::GROUP_NEGATIVE_COUNT,
                "negative part count",
            ));
        }
        for _ in 0..n {
            let r = w.rng().draw(draw_site::GROUP_PART);
            if r <= part.threshold {
                if let Some(p) = part.projectile {
                    group_part(w, desc, source_obj, part, p, &PART_SITES)?;
                }
            } else if let Some(p) = part.projectile_18 {
                group_part(w, desc, source_obj, part, p, &SECOND_SITES)?;
            }
        }
    }
    Ok(())
}

pub fn media_group_descriptor(
    catalog: &ProjectileCatalog,
    p: &crate::projectiles::FlightProjectile,
    t: usize,
    media_height: i16,
) -> Result<GroupDescriptor, Unmodelled> {
    let Some(groups) = catalog.media_groups.as_ref() else {
        return Err(Unmodelled::new(
            draw_site::MEDIA_WITHOUT_DEFINITION,
            format!("projectile entered media, media group type {t}, without a media definition"),
        ));
    };
    let Some(group) = groups.get(t).copied().flatten() else {
        return Err(Unmodelled::new(
            draw_site::MEDIA_GROUP_NONE,
            format!("projectile entered media: media group type {t} has no group (-1)"),
        ));
    };
    let object = p.shape.as_ref().map_or(-1, |s| s.index);
    Ok(GroupDescriptor {
        flags: 3,
        group,
        shading_object: object,
        object,
        owner: p.owner_monster,
        point: [p.object.x, p.object.y, i32::from(media_height)],
        magnitude: 0,
        velocity: [
            p.object.vx,
            p.object.vy,
            (i32::from(p.object.vz).abs() * 2) as i16,
        ],
    })
}

pub fn new_media_group(
    sim: &mut crate::Simulation,
    tick: u32,
    p: &crate::projectiles::FlightProjectile,
    t: usize,
) -> Result<(), Unmodelled> {
    let mut w = SimSpawn::of_record(sim, p, tick);
    let media_height = w.mesh_cell(p.object.x, p.object.y).1;
    let desc = media_group_descriptor(w.catalog(), p, t, media_height)?;
    new_projectile_group(&mut w, &desc)
}

pub(crate) const INTERNAL_PROJECTILE_FLAG_4: [u8; 4] = *b"indc";
pub(crate) const INTERNAL_PROJECTILE_FLAG_8: [u8; 4] = *b"inst";

fn group_part(
    w: &mut dyn ProjectileWorld,
    desc: &GroupDescriptor,
    source_obj: Option<ObjectSnapshot>,
    part: GroupPartFields,
    projectile: FourCC,
    sites: &PartSites,
) -> Result<(), Unmodelled> {
    let Some((def_flags, def_class)) = w
        .catalog()
        .projectiles
        .get(&projectile)
        .map(|d| (d.flags, d.class()))
    else {
        return Err(Unmodelled::new(
            draw_site::AMMUNITION_PART_NOT_LOADED,
            format!("proj {projectile} not loaded"),
        ));
    };
    if desc.flags & 0x100 != 0 && def_flags & 0x8000 != 0 {
        return Ok(());
    }
    let src = source_obj.ok_or_else(|| {
        Unmodelled::new(
            draw_site::AMMUNITION_PART_SOURCE,
            format!("group source object {} not modelled", desc.object),
        )
    })?;
    let pflags = part.flags;
    let (hbase, hdelta) = part.height;
    let mut hfrac = 0i32;
    if hdelta > 0 {
        hfrac = (i32::from(w.rng().draw(sites.height)) * i32::from(hdelta)) >> 16;
    }
    let mut n = ProjectileDescriptor {
        definition: projectile,
        flags: 0,
        position: src.position,
        velocity: src.velocity,
        facing: src.facing,
        elevation: src.elevation,
        word_1c: 0x100,
        owner_monster: -1,
        parent: -1,
        owner_object: -1,
        target: -1,
        target_point: [0; 3],
    };
    if desc.flags & 2 != 0 {
        n.position = desc.point;
    } else if pflags & 1 != 0 {
        n.position = src.position;
        if pflags & 2 != 0 {
            n.position[2] = w.mesh_height(n.position[0], n.position[1], MeshHeightMode::Physical);
        }
    } else {
        let r = i32::from(src.radius);
        let (x, y) = if r > 0 {
            let y = src.position[1] + ((i32::from(w.rng().draw(sites.y)) * 2 * r) >> 16) - r;
            let x = src.position[0] + ((i32::from(w.rng().draw(sites.x)) * 2 * r) >> 16) - r;
            (x, y)
        } else {
            (src.position[0] - r, src.position[1] - r)
        };
        let f = i32::from((i32::from(hbase) + hfrac) as i16);
        let h = i32::from((i32::from(src.top) - i32::from(src.bottom)) as i16);
        let z = ((f * h) >> 8) + src.position[2] + 2 * i32::from(src.bottom);
        n.position = [x, y, z];
        if pflags & 2 != 0 {
            n.position[2] = w.mesh_height(x, y, MeshHeightMode::Physical);
        }
    }
    n.velocity = if desc.flags & 1 != 0 {
        desc.velocity
    } else {
        line_segment_to_vector(n.position, desc.point, desc.magnitude)
    };
    if desc.flags & 4 != 0 {
        if def_flags & 0x2100 == 0 {
            let global = FourCC::new(INTERNAL_PROJECTILE_FLAG_4);
            if !w.catalog().projectiles.contains_key(&global) {
                return Err(Unmodelled::new(
                    draw_site::AMMUNITION_GLOBAL_NOT_LOADED,
                    format!("group flag 4: the global projectile {global} not loaded"),
                ));
            }
            n.definition = global;
        }
        n.flags |= 6;
    }
    if desc.flags & 8 != 0 {
        n.flags |= 8;
    }
    if desc.flags & 0x10 != 0 {
        n.flags |= 2;
    }
    if desc.flags & 0x20 != 0 && matches!(def_class, 8 | 9) {
        n.flags |= 1;
    }
    match src.kind {
        3 => {
            n.parent = src.owner;
            n.owner_object = desc.shading_object;
        }
        k => {
            n.owner_object = desc.shading_object;
            if k == 2 {
                n.owner_monster = src.owner;
            }
        }
    }
    if w.game_options_high()
        & mandrake_common::game_options::high(mandrake_common::game_options::TFL)
        == 0
        && desc.owner != -1
    {
        n.owner_monster = desc.owner;
    }
    let index = new_projectile(w, &n)?;
    if let Some(i) = index
        && desc.flags & 0x80 != 0
    {
        let c = ((i32::from(w.rng().draw(sites.countdown)) * 0x1e) >> 16) as i16;
        if let Some(p) = w.projectile_mut(i) {
            p.countdown = c;
            if let Some(s) = p.shape.as_mut() {
                s.no_shape = true;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct NewProjectileDef {
    flags: u32,
    search_radius: i16,
    search_hole: i16,
    does_something: bool,
    object: Option<FourCC>,
    collection: Option<FourCC>,
    speed: i16,
    speed_delta: i16,
    velocity_error: i16,
    lifetime: i16,
    lifetime_delta: i16,
    countdown: i16,
    countdown_delta: i16,
    shape_speed: (u16, i16),
    sequence_0: i16,
    class: i16,
    lightning: Option<FourCC>,
    word_54: i16,
    bounds: mandrake_tag::SequenceWorldBounds,
}

impl NewProjectileDef {
    fn of(p: &ProjectileDefinition, f: &FlightDefinition) -> Self {
        let raw_90 = p
            .raw
            .get(0x90..0x94)
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        let does_something = p.damage.damage_type != -1
            || p.flags & 4 != 0
            || f.physics.gravity != 0
            || p.groups[1].is_some()
            || p.detonation_group.is_some()
            || p.groups[0].is_some()
            || p.projectile_5c.is_some()
            || !matches!(raw_90, 0 | u32::MAX);
        Self {
            flags: p.flags,
            search_radius: p.word(0x8c),
            search_hole: p.word(0x8e),
            does_something,
            object: p.object,
            collection: p.collection,
            speed: p.speed,
            speed_delta: p.speed_delta,
            velocity_error: p.velocity_error,
            lifetime: p.lifetime,
            lifetime_delta: p.lifetime_delta,
            countdown: p.countdown,
            countdown_delta: p.countdown_delta,
            shape_speed: p.shape_speed(),
            sequence_0: p.sequences[0],
            class: p.class(),
            lightning: lightning_tag(p),
            word_54: p.word_54(),
            bounds: f.bounds,
        }
    }
}

pub fn new_projectile(
    w: &mut dyn ProjectileWorld,
    desc: &ProjectileDescriptor,
) -> Result<Option<i32>, Unmodelled> {
    let tag = desc.definition;
    let proj = {
        let c = w.catalog();
        match (c.projectiles.get(&tag), c.flights.get(&tag)) {
            (Some(p), Some(f)) => NewProjectileDef::of(p, f),
            _ => {
                return Err(Unmodelled::new(
                    draw_site::NEW_PROJECTILE_NOT_LOADED,
                    format!("proj {tag} (or its obje) not loaded"),
                ));
            }
        }
    };
    let flags = proj.flags;
    let mut searched = *desc;
    if flags & 0x0200_0000 != 0 {
        let t = search_target_cells(
            w.mesh_size(),
            proj.search_radius,
            proj.search_hole,
            desc.position,
            |x, y| w.cell_monsters(x, y),
            |_, _| {},
        );
        if t == -1 {
            return Ok(None);
        }
        let m = w.monster(t).ok_or_else(|| {
            Unmodelled::new(
                draw_site::NEW_PROJECTILE_TARGET_MISSING,
                format!("target monster {t} not modelled"),
            )
        })?;
        if let Some(id) = crate::engine_rng::unit_key(m.identifier as i16) {
            w.mark_target_object(id);
        }
        searched.target = t;
    }
    let desc = &searched;
    let count = w.projectile_count();
    if count > 0x39c
        && (flags & 0x20_8000 != 0 || (count > 0x3d8 && flags & 0x18 != 0 && !proj.does_something))
    {
        return Ok(None);
    }
    if count >= MAXIMUM_PROJECTILES {
        return Ok(None);
    }
    let obje_tag = proj.object.ok_or_else(|| {
        Unmodelled::new(
            draw_site::NEW_PROJECTILE_NO_OBJECT,
            format!("proj {tag} has no obje"),
        )
    })?;
    let obje = w.catalog().objects.get(&obje_tag).copied().ok_or_else(|| {
        Unmodelled::new(
            draw_site::NEW_PROJECTILE_NO_OBJECT,
            format!("obje {obje_tag} not loaded"),
        )
    })?;
    let collection = proj.collection.ok_or_else(|| {
        Unmodelled::new(
            draw_site::NEW_PROJECTILE_NO_COLLECTION,
            format!("proj {tag} has no collection"),
        )
    })?;
    let mut scale = i32::from(obje.scale);
    if obje.scale_delta > 0 {
        scale += (i32::from(w.rng().draw(draw_site::NEW_PROJECTILE_SCALE))
            * i32::from(obje.scale_delta)) as u32 as i32
            >> 16;
    }
    let scale = scale & 0xffff;
    let mut speed = i32::from(proj.speed as u16);
    if proj.speed_delta > 0 {
        speed += ((u32::from(w.rng().draw(draw_site::NEW_PROJECTILE_SPEED))
            * proj.speed_delta as u32)
            >> 16) as i32;
    }
    let speed = i32::from(speed as i16);
    let mut v = desc.velocity.map(|c| ((i32::from(c) * speed) >> 8) as i16);
    let e = proj.velocity_error;
    if e > 0 {
        let e = i32::from(e);
        for (c, site) in v.iter_mut().zip([
            draw_site::NEW_PROJECTILE_ERROR_X,
            draw_site::NEW_PROJECTILE_ERROR_Y,
            draw_site::NEW_PROJECTILE_ERROR_Z,
        ]) {
            *c = c.wrapping_add((((i32::from(w.rng().draw(site)) * (2 * e + 1)) >> 16) - e) as i16);
        }
    }
    if flags & 0x40 != 0
        && let Some(wv) = w.wind_velocity()
    {
        for (c, d) in v.iter_mut().zip(wv) {
            *c = c.wrapping_add(d);
        }
    }
    let mut position = desc.position;
    if flags & 0x2000_0000 != 0
        && desc.target != -1
        && let Some(point) = w.monster_middle(desc.target)
    {
        position = point;
    }
    let desc_point = position;
    if flags & 0x20_0000 != 0 {
        position[2] = i32::from(w.mesh_cell(position[0], position[1]).1);
    }
    let hover = if obje.flags & 4 != 0 {
        let surface = w.mesh_height(position[0], position[1], MeshHeightMode::Surface);
        let below = if obje.flags & 0x20 != 0 && surface as i16 == -0x1000 {
            w.model_height(position[0], position[1])?.map(i32::from)
        } else {
            Some(surface)
        };
        below.map_or(0x17c, |h| (position[2] - h) as i16)
    } else {
        0
    };
    let mut obj = w.new_object(&obje, collection, scale, position);
    obj.kind = 3;
    obj.core = w.collection_table0_core(collection);
    if obj.flags & 0x400 != 0 {
        return Err(Unmodelled::new(
            draw_site::NEW_PROJECTILE_SOLID,
            format!("proj {tag}: solid kind 3 object, obje flag 0x10"),
        ));
    }
    let mut lifetime = i32::from(proj.lifetime as u16);
    if proj.lifetime_delta > 0 {
        lifetime += ((u32::from(w.rng().draw(draw_site::NEW_PROJECTILE_LIFETIME))
            * proj.lifetime_delta as u32)
            >> 16) as i32;
    }
    let mut record_flags = desc.flags;
    if MINIMUM_GAMEPLAY_VERSION > 0x6a4 && proj.lifetime == 0 && proj.lifetime_delta == 0 {
        record_flags |= 0x100;
    } else {
        record_flags &= !0x100;
    }
    let mut countdown = i32::from(proj.countdown as u16);
    if proj.countdown_delta > 0 {
        countdown += ((u32::from(w.rng().draw(draw_site::NEW_PROJECTILE_COUNTDOWN))
            * proj.countdown_delta as u32)
            >> 16) as i32;
    }
    let countdown = countdown as i16;
    let homing_speed = if flags & 2 != 0 {
        let (vx, vy) = (i32::from(v[0]), i32::from(v[1]));
        isqrt(vx.wrapping_mul(vx).wrapping_add(vy.wrapping_mul(vy)) as u32)
    } else {
        0
    };
    let target_point = (desc.flags & 0x10 != 0).then_some(desc.target_point);
    if flags & 0x4_0000 != 0 {
        obj.flags |= 0x2000;
    }
    if flags & 0x10_0000 != 0 {
        obj.flags |= 0x1000;
    }
    if flags & 0x20_0000 != 0 {
        obj.flags |= 0x400_4000;
    }
    if flags & 0x8000 != 0 {
        obj.flags &= !0xc0;
    }
    obj.flags &= !0x110;
    let (sbase, sdelta) = proj.shape_speed;
    let mut sspeed = i32::from(sbase);
    if sdelta > 0 {
        sspeed += ((u32::from(w.rng().draw(draw_site::NEW_PROJECTILE_SECOND_SPEED))
            * sdelta as u32)
            >> 16) as i32;
    }
    let sequence = i32::from(proj.sequence_0);
    let header = w.sequence_header(collection, sequence);
    let shape_flags = if flags & 8 != 0 { 8 } else { 0xb };
    set_object_shape_header(
        &mut obj,
        header,
        sequence,
        i32::from(sspeed as i16),
        shape_flags,
        &mut w.rng(),
    )?;
    let shaped = !obj.no_shape;
    if countdown != 0 {
        obj.no_shape = true;
    }
    if flags & 0x200 != 0 && w.rng().draw(draw_site::NEW_PROJECTILE_FLIP_200) & 1 != 0 {
        obj.flags |= 0x8_0000;
    }
    if flags & 0x800 != 0 {
        if w.rng().draw(draw_site::NEW_PROJECTILE_FLIP) & 1 != 0 {
            obj.flags |= 0x2_0000;
        }
        if flags & 0x8_0000 == 0 && w.rng().draw(draw_site::NEW_PROJECTILE_FLIP_80000) & 1 != 0 {
            obj.flags |= 0x4_0000;
        }
    }
    if flags & 0x4000 != 0 {
        obj.flags |= 0x8000;
    }
    if matches!(proj.class, 8 | 9) {
        obj.flags |= 0x40_0000;
    }
    let mut record_0a = crate::statistics::UnitRef::NONE;
    let mut record_36 = -1;
    let mut ignore = -1;
    if desc.owner_monster != -1 {
        let m = w.monster(desc.owner_monster).ok_or_else(|| {
            Unmodelled::new(
                draw_site::NEW_PROJECTILE_OWNER_MISSING,
                format!("owner monster {} not modelled", desc.owner_monster),
            )
        })?;
        record_0a = m.word_14;
        record_36 = i16::from(m.byte_17);
        ignore = m.object;
        if m.flags & 0x1000_0000 != 0 {
            record_flags |= 0x20;
        }
        if m.flags_8 & 0x1000 != 0 {
            record_flags |= 0x40;
        }
        if m.flags_8 & 2 != 0 {
            record_flags |= 0x80;
        }
    }
    if desc.owner_object != -1
        && let Some(o) = w.object(desc.owner_object)
    {
        if flags & 1 != 0 && proj.collection == Some(o.collection) {
            obj.permutation = o.shading;
            obj.shading_slot = o.shading_slot;
            obj.core = o.core;
        }
        if o.collection == obj.collection && o.sequence == obj.sequence {
            obj.frame = o.frame;
            obj.view = o.view;
        }
    }
    let mut target = if desc.target == -1 {
        None
    } else {
        let m = w.monster(desc.target).ok_or_else(|| {
            Unmodelled::new(
                draw_site::NEW_PROJECTILE_TARGET_MONSTER_MISSING,
                format!("target monster {} not modelled", desc.target),
            )
        })?;
        crate::engine_rng::unit_key(m.identifier as i16)
    };
    let mut owner_monster = desc.owner_monster;
    let mut record_0e = desc.word_1c;
    if desc.parent != -1 {
        let r = w.record(desc.parent).ok_or_else(|| {
            Unmodelled::new(
                draw_site::NEW_PROJECTILE_PARENT_MISSING,
                format!("parent record {} not modelled", desc.parent),
            )
        })?;
        record_0e = r.word_0e;
        record_0a = r.word_0a;
        owner_monster = r.owner_monster;
        record_36 = r.word_36;
        record_flags |= r.flags & 0xe0;
        if desc.target == -1 {
            target = r.target;
        }
    }
    if flags & 0x4000_0000 != 0
        && let Some(id) = target
    {
        w.mark_target_object(id);
    }
    if desc.flags & 4 != 0 {
        obj.shading_slot = 1;
    }
    if desc.flags & 8 != 0 {
        obj.shading_slot = 4;
    }
    if flags & 0x2000_0000 != 0 {
        record_flags |= 1;
    }
    let mut lightning = None;
    let mut lightning_failed = false;
    if flags & 0x0100_0000 != 0 {
        let source = if owner_monster == -1 {
            -1
        } else {
            w.monster(owner_monster).map_or(-1, |m| m.object as i16)
        };
        let target_object = match target {
            Some(u) => {
                let m = w
                    .monster_index(u)
                    .and_then(|i| w.monster(i))
                    .ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::LIGHTNING_TARGET_MISSING,
                            format!("lightning target {u} not modelled"),
                        )
                    })?;
                Some(m.object as i16)
            }
            None => None,
        };
        let end = match target_object {
            Some(_) => Some(desc_point),
            None if desc.flags & 0x10 != 0 => Some(desc.target_point),
            None => None,
        };
        lightning = end.and_then(|end| {
            w.new_lightning(
                proj.lightning,
                source,
                target_object.unwrap_or(-1),
                desc_point,
                end,
            )
        });
        lightning_failed = lightning.is_none();
    }
    let fuse_threshold = w.particle_system_intensity(position)?;
    let (radius, bottom, top) = if !shaped {
        (0, 0, 0)
    } else {
        obj.bounds_of_header(w.sequence_header(collection, i32::from(obj.sequence)))
            .unwrap_or_else(|| CollisionObject::scaled(proj.bounds, scale))
    };
    let shape_flags = (obj.flags & 0x1f0) as u16;
    let object = FlightObject {
        x: position[0],
        y: position[1],
        z: position[2],
        vx: v[0],
        vy: v[1],
        vz: v[2],
        above_ground: 0,
        hover,
        shape_flags,
        flag_8000: obj.flags & 0x8000 != 0,
        flag_1000: obj.flags & 0x1000 != 0,
        ignore_object: ignore,
        radius,
        bottom,
        top,
    };
    let created = CreatedProjectile {
        definition: tag,
        object: obj.index,
        identifier: obj.identifier,
        point: position,
        facing: desc.facing,
        velocity: v,
    };
    let p = FlightProjectile {
        id: 0,
        definition: tag,
        owner: 0,
        target,
        object,
        record_flags,
        lifetime: lifetime as u16,
        facing: desc.facing,
        elevation: desc.elevation,
        launched_tick: 0,
        created_at: None,
        moves: 0,
        index: 0,
        fuse_threshold,
        shape: Some(obj),
        countdown,
        owner_monster,
        record_0a,
        record_36,
        record_0e,
        engine: w.engine_model(),
        stain_point: [0; 2],
        trail_counter: 0,
        trail_count: 0,
        trail_threshold: proj.word_54 as u16,
        target_point,
        cell_link: 0,
        woken: false,
        homing_speed,
        homing_nearest: None,
        lightning,
        relinked: false,
    };
    let index = w.add_projectile(p);
    w.projectile_created(index, created);
    if lightning_failed && flags & 0x8000_2000 == 0 {
        w.delete_record(index);
        return Ok(None);
    }
    if flags & 0x2000 != 0 || flags & 0x8000_0000 != 0 {
        if flags & 0x8000_0000 != 0 || target.is_some() {
            w.detonate_at_creation(index)?;
        }
        w.delete_record(index);
        return Ok(None);
    }
    Ok(Some(index))
}

pub struct EngineSpawn<'a, 'r> {
    pub world: &'a mut crate::engine_rng::EngineWorld,
    pub rng: &'a mut EngineRng<'r>,
    pub source: Option<ObjectSnapshot>,
    pub host: Option<StageHost<'a>>,
}

pub(crate) fn detonate_engine_record(
    world: &mut crate::engine_rng::EngineWorld,
    rng: &mut EngineRng,
    index: i32,
) -> Result<(), Unmodelled> {
    let Some(p) = world.projectile_records.iter().find(|p| p.index == index) else {
        return Ok(());
    };
    let Some((
        proj_flags,
        impact_frequency,
        has_group_0,
        has_proj_5c,
        raw_90,
        stain_sequence,
        collection,
        deals_damage,
        detonation_group,
        group_scale,
    )) = world.catalog.projectiles.get(&p.definition).map(|proj| {
        let raw_at = |at: usize| {
            proj.raw
                .get(at..at + 4)
                .map_or([0; 4], |b| [b[0], b[1], b[2], b[3]])
        };
        (
            proj.flags,
            proj.impact_frequency,
            proj.groups[0].is_some(),
            raw_at(0x5c) != [0; 4] && proj.projectile_5c.is_some(),
            raw_at(0x90),
            proj.sequences[1],
            proj.collection,
            proj.damage.deals_damage(),
            proj.detonation_group,
            proj.group_scale(),
        )
    })
    else {
        return Err(Unmodelled::new(
            draw_site::DETONATE_PROJECTILE_MISSING,
            format!("proj {} not loaded", p.definition),
        ));
    };
    if p.record_flags & 2 != 0 && proj_flags & 0xa000 == 0 {
        return Ok(());
    }
    let r = rng.draw(crate::projectiles::FREQUENCY_DRAW_SITE);
    let threshold = impact_frequency;
    let replaced = if crate::combat::MINIMUM_GAMEPLAY_VERSION <= 0x6b8 {
        r <= threshold
    } else {
        r < threshold || (crate::combat::MINIMUM_GAMEPLAY_VERSION > 0x70d && threshold > 0xfffe)
    };
    if replaced {
        if has_group_0 || has_proj_5c {
            return Err(Unmodelled::new(
                draw_site::ENGINE_RECORD_REPLACEMENT,
                "the replacement of an engine-model record without a simulation",
            ));
        }
        let p = p.clone();
        world.replacement_monster(raw_90, &p, rng)?;
        return Ok(());
    }
    let record_flags = p.record_flags;
    let point = [p.object.x, p.object.y, p.object.z];
    let above_ground = p.object.above_ground;
    let owner = p.owner_monster;
    if let Some(p) = world
        .projectile_records
        .iter_mut()
        .find(|p| p.index == index)
    {
        p.record_flags |= 1;
    }
    if record_flags & 2 == 0 && above_ground <= 0x5ff && stain_sequence != -1 {
        let random = rng.draw(draw_site::MESH_STAIN);
        let stain = crate::flight::Stain {
            collection: collection.unwrap_or(FourCC::new([0xff; 4])),
            sequence: stain_sequence,
            point,
            random,
        };
        world
            .group_events
            .push(crate::CombatEvent::StainMesh { tick: 0, stain });
    }
    if record_flags & 2 == 0 && deals_damage {
        return Err(Unmodelled::new(
            draw_site::ENGINE_RECORD_DAMAGE,
            "cause_damage of an engine-model record without a simulation",
        ));
    }
    if let Some(group) = detonation_group {
        let Some(p) = world.projectile_records.iter().find(|p| p.index == index) else {
            return Ok(());
        };
        let src = record_object(p).ok_or_else(|| {
            Unmodelled::new(
                draw_site::ENGINE_RECORD_GROUP_NO_OBJECT,
                "detonation group without an object",
            )
        })?;
        let (flags, magnitude) = if record_flags & 2 != 0 {
            (if record_flags & 4 != 0 { 0x14 } else { 0x10 }, 0)
        } else {
            (
                if record_flags & 4 != 0 { 4 } else { 0 },
                i32::from(group_scale.max(0)),
            )
        };
        let desc = GroupDescriptor {
            flags,
            group,
            shading_object: src.index,
            object: src.index,
            owner,
            point: [
                src.position[0],
                src.position[1],
                src.position[2] + ((i32::from(src.bottom) + i32::from(src.top)) >> 1),
            ],
            magnitude,
            velocity: [0; 3],
        };
        let mut w = EngineSpawn {
            world: &mut *world,
            rng: &mut *rng,
            source: Some(src),
            host: None,
        };
        new_projectile_group(&mut w, &desc)?;
    }
    if proj_flags & 0x1000_0000 != 0 && world.fire_timer.is_some() {
        return Err(Unmodelled::new(
            draw_site::ENGINE_RECORD_FIRE,
            "try_to_start_fire of an engine-model record without a simulation",
        ));
    }
    if proj_flags & 0x800_0000 != 0 && owner != -1 {
        return Err(Unmodelled::new(
            draw_site::ENGINE_RECORD_SHOWS_OWNER,
            "an engine-model record without a simulation shows its owner again",
        ));
    }
    Ok(())
}

pub struct StageHost<'a> {
    pub host: &'a mut dyn crate::engine_attack::CombatHost,
    pub actions: &'a mut Option<crate::map_actions::MapActions>,
    pub time: i32,
    pub launcher: (usize, &'a mut crate::engine_move::EngineMover),
}

impl EngineSpawn<'_, '_> {
    pub fn monster_object(
        world: &crate::engine_rng::EngineWorld,
        i: usize,
        (x, y): (i32, i32),
        facing: u16,
    ) -> ObjectSnapshot {
        let o = &world.monsters[i].object;
        let (radius, bottom, top) = o
            .bounds(world.collections.get(&o.collection).map_or(&[], |v| v))
            .unwrap_or((0, 0, 0));
        ObjectSnapshot {
            index: o.index,
            position: [x, y, o.z],
            velocity: o.velocity,
            facing,
            elevation: 0,
            radius,
            bottom,
            top,
            kind: 2,
            owner: i as i16,
            collection: o.collection,
            sequence: o.sequence,
            frame: o.frame,
            view: o.view,
            shading: o.permutation,
            shading_slot: o.shading_slot,
            core: o.core,
        }
    }
}

impl ProjectileWorld for EngineSpawn<'_, '_> {
    fn collection_table0_core(&self, collection: FourCC) -> Option<FourCC> {
        self.world.collection_core(collection)
    }
    fn monster_middle(&self, index: i16) -> Option<[i32; 3]> {
        self.world.monster_middle(index)
    }
    fn mark_target_object(&mut self, identifier: u32) {
        self.world.mark_target_object(identifier);
    }
    fn cell_monsters(&self, x: i32, y: i32) -> Vec<CellMonster> {
        self.world.cell_monster_objects(x, y)
    }
    fn mesh_size(&self) -> (i32, i32) {
        (self.world.cells.width, self.world.cells.height)
    }
    fn catalog(&self) -> &ProjectileCatalog {
        &self.world.catalog
    }
    fn sequence_header(&self, collection: FourCC, sequence: i32) -> Option<SequenceHeader> {
        mandrake_tag::get_sequence(self.world.headers(collection), sequence).copied()
    }
    fn rng(&mut self) -> EngineRng<'_> {
        EngineRng {
            rng: &mut *self.rng.rng,
            sites: &mut *self.rng.sites,
        }
    }
    fn object(&self, index: i32) -> Option<ObjectSnapshot> {
        if let Some(s) = self.source.filter(|s| s.index == index) {
            return Some(s);
        }
        let Some((i, m)) = self
            .world
            .monsters
            .iter()
            .enumerate()
            .find(|(_, m)| m.object.index == index)
        else {
            return resting_object(self.world, index);
        };
        Some(Self::monster_object(self.world, i, m.position, m.facing))
    }
    fn monster(&self, index: i16) -> Option<MonsterSnapshot> {
        let m = self.world.monsters.get(usize::try_from(index).ok()?)?;
        Some(MonsterSnapshot {
            identifier: i32::from(m.identifier),
            object: m.object.index,
            word_14: crate::statistics::UnitRef(self.world.statistics.monster_unit(index as usize)),
            byte_17: m.owner as i8,
            flags: m.flags,
            flags_8: m.flags_8,
        })
    }
    fn record(&self, index: i16) -> Option<RecordSnapshot> {
        self.world
            .projectile_records
            .iter()
            .find(|p| p.index == i32::from(index))
            .map(|p| RecordSnapshot::of(p))
    }
    fn mesh_cell(&self, x: i32, y: i32) -> (u16, i16) {
        let c = self.world.cells.cell_index(x >> 9, y >> 9);
        (
            self.world.cells.flags.get(c).copied().unwrap_or(0),
            self.world.heights.get(c).map_or(0, |h| h.1),
        )
    }
    fn mesh_height(&self, x: i32, y: i32, mode: MeshHeightMode) -> i32 {
        self.world.mesh_height(x, y, mode)
    }
    fn model_height(&self, x: i32, y: i32) -> Result<Option<i16>, Unmodelled> {
        crate::models::model_height_at(
            &self.world.cells,
            &self.world.models,
            x,
            y,
            draw_site::GROUP_MODEL_HEIGHT,
        )
    }
    fn game_options_high(&self) -> u16 {
        (self.world.game_options >> 16) as u16
    }
    fn new_object(
        &mut self,
        obje: &ObjectDefinition,
        collection: FourCC,
        scale: i32,
        position: [i32; 3],
    ) -> EngineObject {
        let mut r = EngineRng {
            rng: &mut *self.rng.rng,
            sites: &mut *self.rng.sites,
        };
        self.world.new_object_with(
            (obje.flags, obje.vitality, obje.vitality_delta),
            collection,
            scale,
            (position[0], position[1], position[2]),
            &mut r,
        )
    }
    fn projectile_count(&self) -> usize {
        let w = &*self.world;
        used_records(
            [
                w.projectile_records.used(),
                w.fade_records.used(),
                w.reserved_records.used(),
            ],
            None,
            || {
                (w.projectile_records.iter().map(|q| q.index))
                    .chain(w.fade_records.iter().map(|r| r.index))
                    .chain(w.reserved_records.iter().copied())
            },
        )
        .0
    }
    fn add_projectile(&mut self, mut p: FlightProjectile) -> i32 {
        let w = &*self.world;
        let index = used_records(
            [
                w.projectile_records.used(),
                w.fade_records.used(),
                w.reserved_records.used(),
            ],
            None,
            || {
                (w.projectile_records.iter().map(|q| q.index))
                    .chain(w.fade_records.iter().map(|r| r.index))
                    .chain(w.reserved_records.iter().copied())
            },
        )
        .1;
        p.index = index;
        p.cell_link = self.world.next_cell_link();
        if let Some(m) = self
            .world
            .monsters
            .get(usize::try_from(p.owner_monster).unwrap_or(usize::MAX))
        {
            p.owner = u32::from(m.identifier as u16);
        }
        self.world.projectile_records.push(Box::new(p));
        index
    }
    fn projectile_mut(&mut self, index: i32) -> Option<&mut FlightProjectile> {
        self.world
            .projectile_records
            .iter_mut()
            .find(|p| p.index == index)
            .map(|p| &mut **p)
    }
    fn engine_model(&self) -> bool {
        true
    }
    fn group_launched(&mut self, desc: &GroupDescriptor, media_height: i16) {
        self.world.launched_groups.push((*desc, media_height));
    }
    fn object_request_flags(&self, index: i32) -> u16 {
        let w = &*self.world;
        let flags = w
            .monsters
            .iter()
            .find(|m| m.object.index == index)
            .map(|m| m.object.flags)
            .or_else(|| {
                w.projectile_records
                    .iter()
                    .find(|p| p.shape.as_ref().is_some_and(|o| o.index == index))
                    .map(|p| u32::from(p.object.shape_flags))
            })
            .unwrap_or(0);
        u16::from(flags & 0x80 == 0)
    }
    fn object_identifier(&self, index: i32) -> i16 {
        let w = &*self.world;
        w.monsters
            .iter()
            .find(|m| m.object.index == index)
            .map(|m| m.object.identifier)
            .or_else(|| {
                w.projectile_records
                    .iter()
                    .filter_map(|p| p.shape.as_ref())
                    .find(|o| o.index == index)
                    .map(|o| o.identifier)
            })
            .or_else(|| resting_identifier(w, index))
            .unwrap_or(0)
    }
    fn group_event(&mut self, event: crate::CombatEvent) {
        self.world.group_events.push(event);
    }
    fn projectile_created(&mut self, index: i32, created: CreatedProjectile) {
        if let Some(p) = self.projectile_mut(index) {
            p.created_at = Some((created.point, created.velocity));
        }
    }
    fn wind_velocity(&self) -> Option<[i16; 3]> {
        self.world.wind.as_ref().map(|w| w.velocity_words())
    }
    fn particle_system_intensity(&self, point: [i32; 3]) -> Result<u16, Unmodelled> {
        self.world.particle_system_intensity(point)
    }
    fn detonate_at_creation(&mut self, index: i32) -> Result<(), Unmodelled> {
        let Some(h) = self.host.as_mut() else {
            return detonate_engine_record(self.world, self.rng, index);
        };
        h.host.detonate_at_creation(
            self.world,
            self.rng,
            h.actions,
            h.time,
            (h.launcher.0, &mut *h.launcher.1),
            index,
        )
    }
    fn delete_record(&mut self, index: i32) {
        let records = &mut self.world.projectile_records;
        if let Some(at) = records.iter().position(|p| p.index == index) {
            let p = records.remove(at);
            if let Some(s) = p.shape {
                self.world.free_objects.insert(s.index);
            }
        }
    }
    fn monster_index(&self, unit: u32) -> Option<i16> {
        self.world
            .monster_index_by_identifier(unit as u16)
            .map(|i| i as i16)
    }
    fn new_lightning(
        &mut self,
        system: Option<FourCC>,
        source: i16,
        target: i16,
        origin: [i32; 3],
        end: [i32; 3],
    ) -> Option<(i16, i16)> {
        self.world
            .new_lightning(self.rng, system, source, target, origin, end)
    }
    fn new_mesh_effect(&mut self, effect: FourCC, x: i32, y: i32) {
        let center_height = self
            .world
            .mesh_height(x, y, mandrake_tag::MeshHeightMode::Surface);
        crate::mesh_effects::new_mesh_effect(
            &self.world.catalog,
            &mut self.world.mesh_effects,
            effect,
            x,
            y,
            center_height,
        );
    }
}

pub struct SimSpawn<'a> {
    pub sim: &'a mut crate::Simulation,
    pub tick: u32,
    pub source: Option<ObjectSnapshot>,
    pub reserved: Option<i32>,
    pub source_record: Option<RecordSnapshot>,
    pub engine: bool,
    pub updating_object: Option<(i32, u32, i16)>,
}

impl<'a> SimSpawn<'a> {
    pub fn of_record(sim: &'a mut crate::Simulation, p: &FlightProjectile, tick: u32) -> Self {
        Self {
            sim,
            tick,
            source: record_object(p),
            reserved: Some(p.index),
            source_record: Some(RecordSnapshot::of(p)),
            engine: p.engine,
            updating_object: p
                .shape
                .as_ref()
                .map(|o| (o.index, u32::from(p.object.shape_flags), o.identifier)),
        }
    }
}

impl SimSpawn<'_> {
    pub fn unit_object(sim: &crate::Simulation, id: u32) -> Option<ObjectSnapshot> {
        let w = &sim.engine;
        let i = w.monsters.iter().position(|m| m.unit_key() == Some(id))?;
        Self::unit_object_at(sim, id, i)
    }

    pub(crate) fn unit_object_at(
        sim: &crate::Simulation,
        id: u32,
        i: usize,
    ) -> Option<ObjectSnapshot> {
        let w = &sim.engine;
        let (x, y) = sim.unit_world_xy(id)?;
        let o = w.monsters[i].object.clone();
        let mv = sim.units.get(&id)?.engine.as_ref();
        let headers = w.collections.get(&o.collection).map_or(&[][..], |v| v);
        let (radius, bottom, top) = o
            .bounds(headers)
            .or_else(|| sim.unit_cylinder(id))
            .unwrap_or((0, 0, 0));
        Some(ObjectSnapshot {
            index: o.index,
            position: [x, y, o.z],
            velocity: mv.map_or([0; 3], |m| m.velocity),
            facing: mv.map_or(0, |m| m.facing),
            elevation: 0,
            radius,
            bottom,
            top,
            kind: 2,
            owner: i as i16,
            collection: o.collection,
            sequence: o.sequence,
            frame: o.frame,
            view: o.view,
            shading: o.permutation,
            shading_slot: o.shading_slot,
            core: o.core,
        })
    }
}

pub fn record_object(p: &FlightProjectile) -> Option<ObjectSnapshot> {
    let o = p.shape.as_ref()?;
    Some(ObjectSnapshot {
        index: o.index,
        position: [p.object.x, p.object.y, p.object.z],
        velocity: [p.object.vx, p.object.vy, p.object.vz],
        facing: p.facing,
        elevation: p.elevation,
        radius: p.object.radius,
        bottom: p.object.bottom,
        top: p.object.top,
        kind: 3,
        owner: p.index as i16,
        collection: o.collection,
        sequence: o.sequence,
        frame: o.frame,
        view: o.view,
        shading: o.permutation,
        shading_slot: o.shading_slot,
        core: o.core,
    })
}

fn resting_object(w: &crate::engine_rng::EngineWorld, index: i32) -> Option<ObjectSnapshot> {
    let p = w
        .projectiles
        .iter()
        .find(|p| p.object.index == index && p.object.kind == 4)?;
    let o = &p.object;
    let headers = w.collections.get(&o.collection).map_or(&[][..], |v| v);
    let (radius, bottom, top) = o.bounds(headers).unwrap_or((0, 0, 0));
    Some(ObjectSnapshot {
        index,
        position: [p.position.0, p.position.1, o.z],
        velocity: [0; 3],
        facing: 0,
        elevation: 0,
        radius,
        bottom,
        top,
        kind: 4,
        owner: -1,
        collection: o.collection,
        sequence: o.sequence,
        frame: o.frame,
        view: o.view,
        shading: o.permutation,
        shading_slot: o.shading_slot,
        core: o.core,
    })
}

fn resting_identifier(w: &crate::engine_rng::EngineWorld, index: i32) -> Option<i16> {
    w.projectiles
        .iter()
        .find(|p| p.object.index == index && p.object.kind == 4)
        .map(|p| p.object.identifier)
}

impl SimSpawn<'_> {
    fn fade_records(&self) -> &[crate::engine_rng::FadeRecord] {
        self.sim.engine.fade_records.as_slice()
    }

    fn used_records(&self) -> (usize, i32) {
        let flights = &self.sim.combat.flight.flights;
        let w = &self.sim.engine;
        used_records(
            [
                flights.used(),
                w.fade_records.used(),
                w.reserved_records.used(),
            ],
            self.reserved,
            || {
                (flights.iter().map(|q| q.index))
                    .chain(self.fade_records().iter().map(|r| r.index))
                    .chain(self.reserved)
                    .chain(w.reserved_records.iter().copied())
            },
        )
    }
}

impl ProjectileWorld for SimSpawn<'_> {
    fn collection_table0_core(&self, collection: FourCC) -> Option<FourCC> {
        self.sim.engine.collection_core(collection)
    }
    fn monster_middle(&self, index: i16) -> Option<[i32; 3]> {
        self.sim.engine.monster_middle(index)
    }
    fn mark_target_object(&mut self, identifier: u32) {
        self.sim.engine.mark_target_object(identifier);
    }
    fn cell_monsters(&self, x: i32, y: i32) -> Vec<CellMonster> {
        self.sim.engine.cell_monster_objects(x, y)
    }
    fn mesh_size(&self) -> (i32, i32) {
        (self.sim.engine.cells.width, self.sim.engine.cells.height)
    }
    fn catalog(&self) -> &ProjectileCatalog {
        &self.sim.engine.catalog
    }

    fn sequence_header(&self, collection: FourCC, sequence: i32) -> Option<SequenceHeader> {
        mandrake_tag::get_sequence(self.sim.engine.headers(collection), sequence).copied()
    }
    fn rng(&mut self) -> EngineRng<'_> {
        EngineRng {
            rng: &mut self.sim.rng,
            sites: &mut self.sim.engine_sites,
        }
    }
    fn object(&self, index: i32) -> Option<ObjectSnapshot> {
        if let Some(s) = self.source.filter(|s| s.index == index) {
            return Some(s);
        }
        let w = &self.sim.engine;
        if let Some(m) = w.monsters.iter().find(|m| m.object.index == index) {
            return Self::unit_object(self.sim, m.unit_key()?);
        }
        self.sim
            .combat
            .flight
            .flights
            .iter()
            .find(|p| p.shape.as_ref().is_some_and(|s| s.index == index))
            .and_then(|p| record_object(p))
            .or_else(|| resting_object(w, index))
    }
    fn monster(&self, index: i16) -> Option<MonsterSnapshot> {
        let w = &self.sim.engine;
        let m = w.monsters.get(usize::try_from(index).ok()?)?;
        Some(MonsterSnapshot {
            identifier: i32::from(m.identifier),
            object: m.object.index,
            word_14: crate::statistics::UnitRef(w.statistics.monster_unit(index as usize)),
            byte_17: m.owner as i8,
            flags: m.flags,
            flags_8: m.flags_8,
        })
    }
    fn record(&self, index: i16) -> Option<RecordSnapshot> {
        if self.reserved == Some(i32::from(index)) {
            return self.source_record;
        }
        self.sim
            .combat
            .flight
            .flights
            .iter()
            .find(|p| p.index == i32::from(index))
            .map(|p| RecordSnapshot::of(p))
    }
    fn mesh_cell(&self, x: i32, y: i32) -> (u16, i16) {
        let w = &self.sim.engine;
        let c = w.cells.cell_index(x >> 9, y >> 9);
        (
            w.cells.flags.get(c).copied().unwrap_or(0),
            w.heights.get(c).map_or(0, |h| h.1),
        )
    }
    fn mesh_height(&self, x: i32, y: i32, mode: MeshHeightMode) -> i32 {
        self.sim.engine.mesh_height(x, y, mode)
    }
    fn model_height(&self, x: i32, y: i32) -> Result<Option<i16>, Unmodelled> {
        let w = &self.sim.engine;
        crate::models::model_height_at(&w.cells, &w.models, x, y, draw_site::GROUP_MODEL_HEIGHT)
    }
    fn game_options_high(&self) -> u16 {
        self.sim.game_options_high
    }
    fn new_object(
        &mut self,
        obje: &ObjectDefinition,
        collection: FourCC,
        scale: i32,
        position: [i32; 3],
    ) -> EngineObject {
        let crate::Simulation {
            engine,
            rng,
            engine_sites,
            ..
        } = &mut *self.sim;
        let mut r = EngineRng {
            rng,
            sites: engine_sites,
        };
        engine.new_object_with(
            (obje.flags, obje.vitality, obje.vitality_delta),
            collection,
            scale,
            (position[0], position[1], position[2]),
            &mut r,
        )
    }
    fn projectile_count(&self) -> usize {
        self.used_records().0
    }
    fn add_projectile(&mut self, mut p: FlightProjectile) -> i32 {
        let index = self.used_records().1;
        p.index = index;
        self.sim.combat.next_projectile_id += 1;
        p.id = self.sim.combat.next_projectile_id;
        p.launched_tick = self.tick;
        {
            let w = &mut self.sim.engine;
            p.cell_link = w.next_cell_link();
        }
        if let Some(m) = {
            self.sim
                .engine
                .monsters
                .get(usize::try_from(p.owner_monster).unwrap_or(usize::MAX))
        } {
            p.owner = u32::from(m.identifier as u16);
        }
        self.sim.combat.flight.flights.push(Box::new(p));
        index
    }
    fn projectile_mut(&mut self, index: i32) -> Option<&mut FlightProjectile> {
        self.sim
            .combat
            .flight
            .flights
            .iter_mut()
            .find(|p| p.index == index)
            .map(|p| &mut **p)
    }
    fn engine_model(&self) -> bool {
        self.engine
    }
    fn detonate_at_creation(&mut self, index: i32) -> Result<(), Unmodelled> {
        let flights = &mut self.sim.combat.flight.flights;
        let Some(at) = flights.iter().position(|p| p.index == index) else {
            return Ok(());
        };
        let mut p = flights.remove(at);
        let target = p.target;
        if self
            .sim
            .detonate_record(&mut p, false, target.is_some(), target, self.tick)?
        {
            self.sim.combat.flight.flights.push(p);
        }
        Ok(())
    }
    fn delete_record(&mut self, index: i32) {
        let flights = &mut self.sim.combat.flight.flights;
        if let Some(at) = flights.iter().position(|p| p.index == index) {
            let p = flights.remove(at);
            if let Some(s) = p.shape {
                self.sim.engine.free_objects.insert(s.index);
            }
        }
    }
    fn wind_velocity(&self) -> Option<[i16; 3]> {
        (self.sim.engine.wind.as_ref()).map(|w| w.velocity_words())
    }
    fn particle_system_intensity(&self, point: [i32; 3]) -> Result<u16, Unmodelled> {
        self.sim.engine.particle_system_intensity(point)
    }
    fn group_launched(&mut self, desc: &GroupDescriptor, media_height: i16) {
        self.sim
            .combat
            .events
            .push(crate::CombatEvent::ProjectileGroupLaunched {
                tick: self.tick,
                group: desc.group,
                flags: desc.flags,
                object: desc.object,
                point: desc.point,
                media_height,
            });
    }
    fn object_request_flags(&self, index: i32) -> u16 {
        let flight = &self.sim.combat.flight;
        let flags = self
            .updating_object
            .filter(|(i, _, _)| *i == index)
            .map(|(_, f, _)| f)
            .or_else(|| {
                flight
                    .flights
                    .iter()
                    .find(|p| p.shape.as_ref().is_some_and(|o| o.index == index))
                    .map(|p| u32::from(p.object.shape_flags))
            })
            .or_else(|| {
                let w = &self.sim.engine;
                let m = w.monsters.iter().find(|m| m.object.index == index)?;
                Some(m.object.flags)
            })
            .unwrap_or(0);
        u16::from(flags & 0x80 == 0)
    }
    fn object_identifier(&self, index: i32) -> i16 {
        let flight = &self.sim.combat.flight;
        self.updating_object
            .filter(|(i, _, _)| *i == index)
            .map(|(_, _, identifier)| identifier)
            .or_else(|| {
                flight
                    .flights
                    .iter()
                    .filter_map(|p| p.shape.as_ref())
                    .find(|o| o.index == index)
                    .map(|o| o.identifier)
            })
            .or_else(|| {
                let w = &self.sim.engine;
                w.monsters
                    .iter()
                    .find(|m| m.object.index == index)
                    .map(|m| m.object.identifier)
                    .or_else(|| resting_identifier(w, index))
            })
            .unwrap_or(0)
    }
    fn group_event(&mut self, mut event: crate::CombatEvent) {
        if let crate::CombatEvent::ProjectileGroupSound { tick, .. }
        | crate::CombatEvent::ProjectileGroupLocal { tick, .. } = &mut event
        {
            *tick = self.tick;
        }
        self.sim.combat.events.push(event);
    }
    fn projectile_created(&mut self, _: i32, c: CreatedProjectile) {
        self.sim
            .combat
            .events
            .push(crate::CombatEvent::ProjectileCreated {
                tick: self.tick,
                definition: c.definition,
                object: c.object,
                identifier: c.identifier,
                point: c.point,
                facing: c.facing,
                velocity: c.velocity,
            });
    }
    fn monster_index(&self, unit: u32) -> Option<i16> {
        self.sim
            .engine
            .monster_index_by_identifier(unit as u16)
            .map(|i| i as i16)
    }
    fn new_lightning(
        &mut self,
        system: Option<FourCC>,
        source: i16,
        target: i16,
        origin: [i32; 3],
        end: [i32; 3],
    ) -> Option<(i16, i16)> {
        let crate::Simulation {
            engine,
            rng,
            engine_sites,
            ..
        } = &mut *self.sim;
        let mut r = EngineRng {
            rng,
            sites: engine_sites,
        };
        engine.new_lightning(&mut r, system, source, target, origin, end)
    }
    fn new_mesh_effect(&mut self, effect: FourCC, x: i32, y: i32) {
        {
            let w = &mut self.sim.engine;
            let center_height = w.mesh_height(x, y, mandrake_tag::MeshHeightMode::Surface);
            crate::mesh_effects::new_mesh_effect(
                &w.catalog,
                &mut w.mesh_effects,
                effect,
                x,
                y,
                center_height,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowest_free_record_is_the_first_unused_index() {
        let free = |used: &[i32]| lowest_free_record(used.iter().copied());
        assert_eq!(free(&[]), 0);
        assert_eq!(free(&[0, 1, 2]), 3);
        assert_eq!(free(&[2, 0, 1, 4]), 3);
        assert_eq!(free(&[1, 1, 1]), 0);
        assert_eq!(free(&[0, 0, 1]), 2);
        assert_eq!(free(&[-1, 0, 900]), 1);
        assert_eq!(free(&[0, 2, 3, 1, 5, 4]), 6);
    }

    #[test]
    fn line_segment_to_vector_points_away_and_never_divides_by_zero() {
        assert_eq!(line_segment_to_vector([10, 0, 0], [0, 0, 0], 4), [4, 0, 0]);
        assert_eq!(
            line_segment_to_vector([0, 0, 0], [0, 30, 40], 10),
            [0, -6, -8]
        );
        assert_eq!(line_segment_to_vector([5, 5, 5], [5, 5, 5], 7), [0, 0, 0]);
    }
}
