use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_math::Fixed16;
use mandrake_tag::DamageDefinition;

use crate::Simulation;
use crate::combat::{CombatEvent, damage_object, damage_object_draws, kill_code};
use crate::draw_site;
use crate::engine_rng::Unmodelled;

pub fn distance3d(a: [i32; 3], b: [i32; 3]) -> i32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    if d.iter().any(|&c| !(-0x7fff..=0x7ffe).contains(&c)) {
        return 0x7fff;
    }
    let s = d
        .iter()
        .fold(0u32, |s, &c| s.wrapping_add(c.wrapping_mul(c) as u32));
    crate::projectiles::isqrt(s)
}

pub fn explosion_scale(d: i16, radius: i16) -> i16 {
    if radius == 0 {
        return 0x100;
    }
    let three_quarters = i32::from(d.wrapping_sub(d >> 2));
    let v = 0x100 - ((three_quarters << 8) / i32::from(radius));
    (v as i16).max(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VictimObject {
    Unit(u32),
    Scenery(usize),
    Flight(u32),
    Dormant(i32),
    Marker(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Victim {
    pub object: VictimObject,
    pub index: i32,
    pub identifier: i16,
    pub kind: u8,
    pub distance: i16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ScanObject {
    object: VictimObject,
    index: i32,
    identifier: i16,
    kind: u8,
    position: [i32; 3],
    flags: u32,
    undamaged: bool,
    link: u64,
}

#[derive(Debug, Clone, Copy)]
enum ScanScope {
    #[cfg_attr(not(feature = "identity-checks"), expect(dead_code))]
    All,
    Cells {
        x: (i32, i32),
        y: (i32, i32),
    },
    #[cfg_attr(not(feature = "identity-checks"), expect(dead_code))]
    Cell(i32, i32),
    One(VictimObject),
}

impl ScanScope {
    fn kind(self, object: VictimObject) -> bool {
        match self {
            Self::One(o) => o == object,
            _ => true,
        }
    }

    fn cell(self, x: i32, y: i32) -> bool {
        let (x, y) = (x >> 9, y >> 9);
        match self {
            Self::Cells {
                x: (x0, x1),
                y: (y0, y1),
            } => (x0..x1).contains(&x) && (y0..=y1).contains(&y),
            Self::Cell(cx, cy) => x == cx && y == cy,
            Self::One(_) | Self::All => true,
        }
    }

    fn keeps(self, object: VictimObject, position: [i32; 3]) -> bool {
        self.kind(object) && self.cell(position[0], position[1])
    }
}

struct AreaObjects {
    x: (i32, i32),
    y: (i32, i32),
    cells: Option<Vec<Vec<ScanObject>>>,
}

impl AreaObjects {
    fn cell(
        &mut self,
        sim: &Simulation,
        source: Option<ExplosionSource>,
        x: i32,
        y: i32,
    ) -> &[ScanObject] {
        let ((x0, x1), (y0, y1)) = (self.x, self.y);
        let width = (x1 - x0) as usize;
        let at = |cx: i32, cy: i32| (cy - y0) as usize * width + (cx - x0) as usize;
        let cells = self.cells.get_or_insert_with(|| {
            let mut cells = vec![Vec::new(); width * (y1 - y0 + 1) as usize];
            let scope = ScanScope::Cells {
                x: (x0, x1),
                y: (y0, y1),
            };
            for o in sim.scan_objects(source, scope) {
                cells[at(o.position[0] >> 9, o.position[1] >> 9)].push(o);
            }
            cells
        });
        &cells[at(x, y)]
    }
}

pub fn scan_cells(point: [i32; 3], r: i32, mesh: (i32, i32)) -> ((i32, i32), (i32, i32)) {
    let x0 = (i32::from(((point[0] - r) >> 9) as i16)).max(0);
    let y0 = (i32::from(((point[1] - r) >> 9) as i16)).max(0);
    let x1 = i32::from(((point[0] + r + 0x1ff) >> 9) as i16);
    let y1 = i32::from(((point[1] + r + 0x1ff) >> 9) as i16);
    let x1 = if mesh.0 > x1 { x1 } else { mesh.0 - 1 };
    let y1 = if mesh.1 > y1 { y1 } else { mesh.1 - 1 };
    ((x0, x1), (y0, y1))
}

pub fn area_scale(d: i32, radius: i32) -> i16 {
    let three_quarters = i32::from((d - (d >> 2)) as i16);
    (0x100 - (three_quarters << 8) / radius) as i16
}

pub fn velocity_scale(velocity: [i16; 3], scale: i16) -> i16 {
    let sum = velocity.iter().fold(0u32, |s, &c| {
        s.wrapping_add((i32::from(c) * i32::from(c)) as u32)
    });
    let magnitude = i32::from(crate::projectiles::isqrt(sum) as i16);
    ((magnitude * i32::from(scale)) >> 8) as i16
}

pub fn eviction_choice(records: impl Iterator<Item = (i16, i16)>) -> Option<usize> {
    let mut best: Option<(usize, i32)> = None;
    for (k, (reach, radius)) in records.enumerate() {
        let v = if radius == 0 {
            0
        } else {
            (i32::from(reach) << 16) / i32::from(radius)
        };
        if best.is_none_or(|(_, b)| v > b) {
            best = Some((k, v));
        }
    }
    best.map(|(k, _)| k)
}

pub(crate) fn push_victim(
    victims: &mut Vec<Victim>,
    next: &mut usize,
    victim: Victim,
    flags: u32,
    flags_of: impl Fn(&Victim) -> u32,
) {
    if victims.len() < 0x58 {
        victims.push(victim);
        return;
    }
    if *next > 0x57 || flags & 0x40_0000 == 0 {
        return;
    }
    while *next < 0x58 {
        if flags_of(&victims[*next]) & 0x40_0000 == 0 {
            victims[*next] = victim;
            return;
        }
        *next += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explosion {
    pub definition: FourCC,
    pub damage: DamageDefinition,
    pub reach: i16,
    pub radius: i16,
    pub victims: Vec<Victim>,
    pub owner: u32,
    pub point: [i32; 3],
    pub created: u32,
    pub info: Option<ExplosionInfo>,
    pub slot: i16,
    pub updating: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplosionInfo {
    pub unit: crate::statistics::UnitRef,
    pub attacker: i16,
    pub attacker_id: i16,
    pub velocity: [i16; 3],
    pub flags: u16,
    pub mana: i16,
    pub factor_bits: u32,
    pub source: Option<ExplosionSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplosionSource {
    pub id: u32,
    pub index: i32,
    pub identifier: i16,
    pub definition: FourCC,
    pub position: [i32; 3],
    pub flags: u32,
    pub vitality: i32,
    pub max_vitality: i32,
    pub link: u64,
}

impl ExplosionSource {
    pub fn of(p: &crate::projectiles::FlightProjectile) -> Option<Self> {
        let o = p.shape.as_ref()?;
        Some(Self {
            id: p.id,
            index: o.index,
            identifier: o.identifier,
            definition: p.definition,
            position: [p.object.x, p.object.y, p.object.z],
            flags: o.flags | if o.no_shape { 0x200 } else { 0 },
            vitality: o.vitality,
            max_vitality: o.max_vitality,
            link: p.cell_link,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneryDamage {
    pub flags: u32,
    pub destroyed_sequence: i16,
    pub damaged_group: Option<FourCC>,
    pub destroyed_group: Option<FourCC>,
    pub object_flags: u16,
    pub modifiers: [i16; 9],
    pub threshold: i16,
    pub largest_scale: i32,
    pub model_permutation: i16,
}

impl SceneryDamage {
    pub fn from_tags(scen: &[u8], obje: &[u8]) -> Option<Self> {
        let be16 = |b: &[u8], o: usize| Some(i16::from_be_bytes([*b.get(o)?, *b.get(o + 1)?]));
        let group = |o: usize| {
            let w: [u8; 4] = scen.get(o..o + 4)?.try_into().ok()?;
            (w != [0xff; 4] && w != [0; 4]).then(|| FourCC::new(w))
        };
        let mut modifiers = [0i16; 9];
        for (k, m) in modifiers.iter_mut().enumerate() {
            *m = be16(obje, 0x10 + 2 * k)?;
        }
        Some(Self {
            flags: u32::from_be_bytes(scen.get(0..4)?.try_into().ok()?),
            destroyed_sequence: be16(scen, 0x2a)?,
            damaged_group: group(0x18),
            destroyed_group: group(0x1c),
            object_flags: be16(obje, 0)? as u16,
            modifiers,
            threshold: be16(obje, 0x30)?,
            largest_scale: i32::from(be16(obje, 0x0c)?) + i32::from(be16(obje, 0x0e)?),
            model_permutation: be16(scen, 0x34)?,
        })
    }
}

pub fn scenery_effect_permutation(offset: i16, permutation: i16, count: i16) -> i16 {
    let sum = offset.wrapping_add(permutation);
    if sum < 0 {
        0
    } else {
        (i32::from(count) - 1).min(i32::from(sum)) as i16
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneryHit {
    pub destroyed: bool,
    pub resisted: bool,
    pub vitality: i32,
    pub max_vitality: i32,
}

pub fn damage_scenery_object(
    def: &SceneryDamage,
    damage: &DamageDefinition,
    amount: i32,
    vitality: i32,
    max_vitality: i32,
    draw: &mut dyn FnMut() -> u16,
) -> SceneryHit {
    damage_plain_object(
        def.modifiers,
        def.threshold,
        damage,
        amount,
        vitality,
        max_vitality,
        draw,
    )
}

pub fn damage_plain_object(
    modifiers: [i16; 9],
    threshold: i16,
    damage: &DamageDefinition,
    amount: i32,
    vitality: i32,
    max_vitality: i32,
    draw: &mut dyn FnMut() -> u16,
) -> SceneryHit {
    let m = modifiers;
    let threshold = i32::from(threshold);
    let f = damage.flags;
    let t = damage.damage_type as u16;
    let scaled = |w: i16| (i32::from(w) * amount) >> 8;
    let resisted = SceneryHit {
        destroyed: false,
        resisted: true,
        vitality,
        max_vitality,
    };
    let mut no_harm = false;
    let d = match t {
        0 => Some(scaled(m[2])),
        1 | 9 => Some(scaled(m[4])),
        8 => Some(scaled(m[3])),
        10 => Some(scaled(m[7])),
        11 => None,
        _ => {
            draw();
            if amount == 0 {
                None
            } else {
                let d = scaled(m[usize::from((f >> 3) & 1)]);
                if d == 0 {
                    no_harm = true;
                    None
                } else {
                    Some(d)
                }
            }
        }
    };
    let d = match d {
        None if threshold >= 0 => return resisted,
        None => 0,
        Some(d) if d > threshold || d < 0 => d,
        Some(_) => return resisted,
    };
    if no_harm {
        return resisted;
    }
    let d = d.min(vitality);
    let mut max_vitality = max_vitality;
    if f & 0x80 != 0 {
        let max = max_vitality - d;
        max_vitality = if 0xffff - d <= 0x1fffe {
            max.clamp(0, 0x7fff)
        } else {
            0x7fff
        };
    }
    let new = vitality - d;
    let (destroyed, mut v) = if 0xffff - d <= 0x1fffe {
        if new < 0 {
            (true, 0)
        } else {
            let v = new.min(0x7fff);
            (v as i16 <= 0, v)
        }
    } else {
        (false, 0x7fff)
    };
    let max = max_vitality as i16 as i32;
    if max < v {
        v = max;
    }
    SceneryHit {
        destroyed,
        resisted: false,
        vitality: v,
        max_vitality,
    }
}

fn scenery_mark_radius(w: &crate::engine_rng::EngineWorld, i: usize, largest_scale: i32) -> i16 {
    let o = &w.scenery[i].object;
    let half_width = w
        .collections
        .get(&o.collection)
        .and_then(|h| mandrake_tag::get_sequence(h, i32::from(o.sequence)))
        .map_or(0, |h| i32::from(h.world_bounds.half_width));
    ((largest_scale * half_width) >> 8) as i16
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ObjectHit {
    pub(crate) definition: FourCC,
    pub(crate) damage: DamageDefinition,
    pub(crate) owner: u32,
    pub(crate) point: [i32; 3],
    pub(crate) info: Option<ExplosionInfo>,
    pub(crate) scale: i16,
    pub(crate) site: u32,
}

impl Simulation {
    pub(crate) fn accelerate_scenery(
        &mut self,
        tick: u32,
        i: usize,
        damage: &DamageDefinition,
        amount: i32,
        magnitude: i32,
        point: [i32; 3],
    ) -> Result<Option<FourCC>, Unmodelled> {
        if matches!(damage.damage_type, 2 | 3 | 11) {
            return Ok(None);
        }
        let w = &self.engine;
        let sc = &w.scenery[i];
        let index = sc.object.index;
        let Some(def) = self.combat.flight.scenery_damage.get(&index).cloned() else {
            return Err(Unmodelled::new(
                draw_site::ACCELERATE_SCENERY_NO_DEFINITION,
                format!("scenery object {index} without its definition"),
            ));
        };
        let (vitality, max_vitality) = (sc.object.vitality, sc.object.max_vitality);
        let hit = {
            let rng = &mut self.rng;
            let sites = &mut self.engine_sites;
            damage_scenery_object(&def, damage, amount, vitality, max_vitality, &mut || {
                sites.add(draw_site::DAMAGE_MONSTER_OBJECT_CHANCE, 1);
                crate::engine_rng::log_draw(draw_site::DAMAGE_MONSTER_OBJECT_CHANCE, rng.state);
                rng.next_u16()
            })
        };
        {
            let w = &mut self.engine;
            let o = &mut w.scenery[i].object;
            o.vitality = hit.vitality;
            o.max_vitality = hit.max_vitality;
        }
        if !hit.destroyed {
            if hit.resisted {
                return Ok(None);
            }
            if def.flags & 0x100 != 0 {
                self.scenery_model_frame(i, false)?;
            }
            return self.scenery_group(tick, i, def.damaged_group, magnitude, point);
        }
        if def.flags & 0x100 != 0 {
            self.scenery_model_frame(i, true)?;
        }
        if def.flags & 0x200 != 0 {
            self.start_cell_model_animation(i)?;
        }
        if let Some(c) = self.scenery_snapshot(i) {
            let to = [
                c.position[0],
                c.position[1],
                c.position[2] + ((i32::from(c.bottom) + i32::from(c.top)) >> 1),
            ];
            let v = crate::projectile_groups::line_segment_to_vector(point, to, magnitude);
            {
                let w = &mut self.engine;
                let o = &mut w.scenery[i].object;
                for (a, b) in o.velocity.iter_mut().zip(v) {
                    *a = a.wrapping_add(b);
                }
            }
        }
        if def.flags & 0x20 != 0 {
            self.scenery_model_effect(i, &def)?;
        }
        if def.destroyed_sequence != -1 {
            let crate::engine_rng::EngineWorld {
                scenery,
                collections,
                ..
            } = &mut self.engine;
            let o = &mut scenery[i].object;
            let headers = collections
                .get(&o.collection)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut rng = crate::engine_rng::EngineRng {
                rng: &mut self.rng,
                sites: &mut self.engine_sites,
            };
            crate::engine_rng::set_object_shape(
                o,
                headers,
                i32::from(def.destroyed_sequence),
                0x100,
                0xf,
                &mut rng,
            )?;
            o.flags &= !0x400;
            o.kind = 0;
            self.combat.flight.scenery.retain(|c| c.object != index);
            return self.scenery_group(tick, i, def.destroyed_group, magnitude, point);
        }
        let group = self.scenery_group(tick, i, def.destroyed_group, magnitude, point)?;
        {
            let w = &mut self.engine;
            let radius = scenery_mark_radius(w, i, def.largest_scale);
            let (x, y) = w.scenery[i].position;
            w.scenery[i].deleted = true;
            w.free_objects.insert(index);
            if def.flags & 0x80 != 0 {
                let ((x0, x1), (y0, y1)) = w.cells.unmark_scenery(x, y, radius);
                let marks: Vec<(i32, i32, i16)> = (0..w.scenery.len())
                    .filter(|&j| {
                        let sc = &w.scenery[j];
                        let (cx, cy) = (sc.position.0 >> 9, sc.position.1 >> 9);
                        !sc.deleted
                            && sc.object.kind == 1
                            && sc.scenery_flags & 0x80 != 0
                            && (x0..=x1).contains(&cx)
                            && (y0..=y1).contains(&cy)
                    })
                    .filter_map(|j| {
                        let d = self
                            .combat
                            .flight
                            .scenery_damage
                            .get(&w.scenery[j].object.index)?;
                        let r = scenery_mark_radius(w, j, d.largest_scale);
                        Some((w.scenery[j].position.0, w.scenery[j].position.1, r))
                    })
                    .collect();
                for (x, y, r) in marks {
                    w.cells.mark_scenery(x, y, r);
                }
                self.engine_terrain = Some(w.cells.clone());
            }
        }
        self.combat.flight.scenery.retain(|c| c.object != index);
        Ok(group)
    }

    fn scenery_group(
        &mut self,
        tick: u32,
        i: usize,
        group: Option<FourCC>,
        magnitude: i32,
        point: [i32; 3],
    ) -> Result<Option<FourCC>, Unmodelled> {
        let Some(group) = group else {
            return Ok(None);
        };
        let src = self.scenery_snapshot(i).ok_or_else(|| {
            Unmodelled::new(
                draw_site::SCENERY_GROUP_NO_SHAPE,
                "a scenery's group without its shape",
            )
        })?;
        let desc = crate::projectile_groups::GroupDescriptor {
            flags: 0,
            group,
            shading_object: src.index,
            object: src.index,
            owner: -1,
            point,
            magnitude,
            velocity: [0; 3],
        };
        let mut w = crate::projectile_groups::SimSpawn {
            sim: self,
            tick,
            source: Some(src),
            reserved: None,
            source_record: None,
            engine: true,
            updating_object: None,
        };
        crate::projectile_groups::new_projectile_group(&mut w, &desc)?;
        Ok(Some(group))
    }

    fn scenery_model_frame(&mut self, i: usize, destroyed: bool) -> Result<(), Unmodelled> {
        let w = &mut self.engine;
        let sc = &w.scenery[i];
        let (px, py) = sc.position;
        let Some(model) = w.model_index_of_cell(px, py) else {
            return Ok(());
        };
        let animation = w
            .models
            .get(model as usize)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::SCENERY_MODEL_NO_RECORD,
                    format!("model {model} without its record"),
                )
            })?
            .animation;
        if animation == -1 {
            return Ok(());
        }
        let k = animation as usize;
        let a = w.model_animations.animations.get(k).ok_or_else(|| {
            Unmodelled::new(
                draw_site::SCENERY_MODEL_ANIMATION_MISSING,
                format!("model animation {animation} not loaded"),
            )
        })?;
        let last = a.definition.tag.frame_count() as i32 - 1;
        let frame = if destroyed {
            last
        } else {
            let (vitality, max) = (sc.object.vitality as i16, sc.object.max_vitality as i16);
            let f = if max == 0 {
                0xff
            } else {
                i32::from((0xff - (i32::from(vitality) << 8) / i32::from(max)) as i16)
            };
            last.wrapping_mul(f) >> 8
        };
        if a.playback.frame as i32 == i32::from(frame as i16) {
            return Ok(());
        }
        if !(0..=last).contains(&frame) {
            return Err(Unmodelled::new(
                draw_site::SCENERY_MODEL_FRAME_OUTSIDE,
                format!("model animation {animation}: frame {frame} of {}", last + 1),
            ));
        }
        let mut animations = std::mem::take(&mut w.model_animations);
        let r = animations.jump(k, frame as usize, w);
        w.model_animations = animations;
        r?;
        let ground = i32::from(w.mesh_height(px, py, mandrake_tag::MeshHeightMode::Surface) as i16);
        let (a, b) = ([px, py, 0x1000], [px, py, ground]);
        let m = w.models.get(model as usize).ok_or_else(|| {
            Unmodelled::new(
                draw_site::SCENERY_MODEL_NO_RECORD_2,
                format!("model {model} without its record"),
            )
        })?;
        let point = mandrake_tag::model_hit::object_hit_model_index(
            &m.placement,
            &m.geometry,
            a,
            b,
            mandrake_tag::model_hit::SurfaceTest::Gameplay,
            0,
        )
        .map_or(b, |h| h.point);
        if (point[0] >> 9, point[1] >> 9) != (px >> 9, py >> 9) {
            return Err(Unmodelled::new(
                draw_site::SCENERY_MODEL_OTHER_CELL,
                format!("scenery {i} put on model {model} in another cell: {point:?}"),
            ));
        }
        let sc = &mut w.scenery[i];
        sc.position = (point[0], point[1]);
        sc.object.z = point[2];
        let index = sc.object.index;
        for c in self
            .combat
            .flight
            .scenery
            .iter_mut()
            .filter(|c| c.object == index)
        {
            (c.x, c.y, c.z) = (point[0], point[1], point[2]);
        }
        Ok(())
    }

    fn scenery_model_effect(&mut self, i: usize, def: &SceneryDamage) -> Result<(), Unmodelled> {
        let w = &mut self.engine;
        let (px, py) = w.scenery[i].position;
        let Some(model) = w.model_index_of_cell(px, py) else {
            return Ok(());
        };
        let m = w.models.get(model as usize).ok_or_else(|| {
            Unmodelled::new(
                draw_site::SCENERY_MODEL_EFFECT_NO_RECORD,
                format!("model {model} without its record"),
            )
        })?;
        let permutation = scenery_effect_permutation(
            def.model_permutation,
            m.permutation,
            m.shape.permutation_count,
        );
        let d = crate::model_effects::ModelEffectDescriptor {
            flags: crate::model_effects::PERMUTATION_ONLY,
            duration: 0,
            new_permutation: -1,
            permutation: i32::from(permutation),
            material: 0,
            ..Default::default()
        };
        let mut rng = crate::engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        crate::model_effects::new_model_effect(w, model, -1, &d, &mut rng)?;
        Ok(())
    }

    fn start_cell_model_animation(&mut self, i: usize) -> Result<(), Unmodelled> {
        let w = &mut self.engine;
        let (px, py) = w.scenery[i].position;
        let Some(model) = w.model_index_of_cell(px, py) else {
            return Ok(());
        };
        let animation = w
            .models
            .get(model as usize)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::CELL_MODEL_ANIMATION_NO_RECORD,
                    format!("model {model} without its record"),
                )
            })?
            .animation;
        if animation == -1 {
            return Ok(());
        }
        let a = w
            .model_animations
            .animations
            .get_mut(animation as usize)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::MODEL_ANIMATION_NOT_LOADED,
                    format!("model animation {animation} not loaded"),
                )
            })?;
        a.playback.active = true;
        Ok(())
    }

    fn scenery_snapshot(&self, i: usize) -> Option<crate::projectile_groups::ObjectSnapshot> {
        let w = &self.engine;
        let sc = &w.scenery[i];
        let o = &sc.object;
        let (radius, bottom, top) = o.bounds(w.collections.get(&o.collection)?)?;
        Some(crate::projectile_groups::ObjectSnapshot {
            index: o.index,
            position: [sc.position.0, sc.position.1, o.z],
            velocity: o.velocity,
            facing: sc.facing,
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
        })
    }

    fn scan_objects(&self, source: Option<ExplosionSource>, scope: ScanScope) -> Vec<ScanObject> {
        let out = self.scan_objects_in(source, scope);
        #[cfg(feature = "identity-checks")]
        assert_eq!(
            out,
            self.scan_objects_in(source, ScanScope::All)
                .into_iter()
                .filter(|o| scope.keeps(o.object, o.position))
                .collect::<Vec<_>>()
        );
        out
    }

    fn scan_objects_in(
        &self,
        source: Option<ExplosionSource>,
        scope: ScanScope,
    ) -> Vec<ScanObject> {
        let mut out = Vec::new();
        let held = &self.combat.flight.detonating_records;
        let mut seen: Vec<u32> = Vec::new();
        let flights = &self.combat.flight.flights;
        let held = held
            .iter()
            .filter(|s| flights.iter().all(|p| p.id != s.id))
            .copied();
        for s in source.into_iter().chain(held) {
            if seen.contains(&s.id) || !scope.keeps(VictimObject::Flight(s.id), s.position) {
                continue;
            }
            seen.push(s.id);
            let catalog = &self.engine.catalog;
            let undamaged = catalog
                .projectiles
                .get(&s.definition)
                .and_then(|p| catalog.objects.get(&p.object?))
                .is_some_and(|o| o.flags & 0x40 != 0);
            out.push(ScanObject {
                object: VictimObject::Flight(s.id),
                index: s.index,
                identifier: s.identifier,
                kind: 3,
                position: s.position,
                flags: s.flags,
                undamaged,
                link: s.link,
            });
        }
        let one = match scope {
            ScanScope::One(o) => Some(o),
            _ => None,
        };
        let first_map: Option<crate::entities::UnitIdMap<usize>> = match scope {
            ScanScope::All | ScanScope::Cells { .. } => {
                let mut first: crate::entities::UnitIdMap<usize> = Default::default();
                for (i, m) in self.engine.monsters.iter().enumerate() {
                    if let Some(key) = m.unit_key() {
                        first.entry(key).or_insert(i);
                    }
                }
                Some(first)
            }
            _ => None,
        };
        let first = |id: u32| -> Option<usize> {
            match &first_map {
                Some(map) => map.get(&id).copied(),
                None => self
                    .engine
                    .monsters
                    .iter()
                    .position(|m| m.unit_key() == Some(id)),
            }
        };
        let units: Box<dyn Iterator<Item = (&u32, &crate::entities::UnitEntity)>> = match one {
            Some(VictimObject::Unit(id)) => Box::new(self.units.get_key_value(&id).into_iter()),
            Some(_) => Box::new(std::iter::empty()),
            None => Box::new(self.units.iter()),
        };
        for (id, u) in units {
            if !scope.kind(VictimObject::Unit(*id)) {
                continue;
            }
            let (x, y) = match u.engine.as_ref() {
                Some(m) => (m.x, m.y),
                None => self.world_to_command_point(u.position),
            };
            if !scope.cell(x, y) {
                continue;
            }
            let Some(m) = first(*id).map(|i| &self.engine.monsters[i]) else {
                continue;
            };
            out.push(ScanObject {
                object: VictimObject::Unit(*id),
                index: m.object.index,
                identifier: m.object.identifier,
                kind: 2,
                position: [x, y, m.object.z],
                flags: m.object.flags | if m.object.no_shape { 0x200 } else { 0 },
                undamaged: false,
                link: m.cell_link,
            });
        }
        let w = &self.engine;
        let scenery: Box<dyn Iterator<Item = (usize, &crate::engine_rng::EngineScenery)>> =
            match one {
                Some(VictimObject::Scenery(i)) => {
                    Box::new(w.scenery.get(i).map(|sc| (i, sc)).into_iter())
                }
                Some(_) => Box::new(std::iter::empty()),
                None => Box::new(w.scenery.iter().enumerate()),
            };
        for (i, sc) in scenery {
            let o = &sc.object;
            if sc.deleted
                || !matches!(o.kind, 1 | 4 | crate::flags::OBJECT_KIND_FLAG)
                || !scope.keeps(VictimObject::Scenery(i), [sc.position.0, sc.position.1, 0])
            {
                continue;
            }
            let Some(d) = self.combat.flight.scenery_damage.get(&o.index) else {
                continue;
            };
            out.push(ScanObject {
                object: VictimObject::Scenery(i),
                index: o.index,
                identifier: o.identifier,
                kind: o.kind,
                position: [sc.position.0, sc.position.1, o.z],
                flags: o.flags | if o.no_shape { 0x200 } else { 0 },
                undamaged: d.object_flags & 0x40 != 0,
                link: sc.cell_link,
            });
        }
        let undamaged = |definition: FourCC| {
            w.catalog
                .projectiles
                .get(&definition)
                .and_then(|p| p.object)
                .and_then(|o| w.catalog.objects.get(&o))
                .is_some_and(|o| o.flags & 0x40 != 0)
        };
        let of_kind = |k: fn(VictimObject) -> bool| one.is_none_or(k);
        let flights: &[Box<crate::projectiles::FlightProjectile>] =
            if of_kind(|o| matches!(o, VictimObject::Flight(_))) {
                &self.combat.flight.flights
            } else {
                &[]
            };
        for p in flights {
            let Some(o) = p.shape.as_ref() else {
                continue;
            };
            if !scope.keeps(VictimObject::Flight(p.id), [p.object.x, p.object.y, 0]) {
                continue;
            }
            out.push(ScanObject {
                object: VictimObject::Flight(p.id),
                index: o.index,
                identifier: o.identifier,
                kind: 3,
                position: [p.object.x, p.object.y, p.object.z],
                flags: o.flags | if o.no_shape { 0x200 } else { 0 },
                undamaged: undamaged(p.definition),
                link: p.cell_link,
            });
        }
        let dormant = if of_kind(|o| matches!(o, VictimObject::Dormant(_))) {
            &self.combat.flight.dormant[..]
        } else {
            &[]
        };
        for d in dormant {
            let o = &d.object;
            if !scope.keeps(VictimObject::Dormant(o.index), d.position) {
                continue;
            }
            out.push(ScanObject {
                object: VictimObject::Dormant(o.index),
                index: o.index,
                identifier: o.identifier,
                kind: o.kind,
                position: d.position,
                flags: o.flags | if o.no_shape { 0x200 } else { 0 },
                undamaged: undamaged(d.definition),
                link: d.cell_link,
            });
        }
        let markers = if of_kind(|o| matches!(o, VictimObject::Marker(_))) {
            &w.projectiles[..]
        } else {
            &[]
        };
        for p in markers.iter().filter(|p| p.object.kind == 4) {
            let o = &p.object;
            if !scope.keeps(
                VictimObject::Marker(o.index),
                [p.position.0, p.position.1, 0],
            ) {
                continue;
            }
            out.push(ScanObject {
                object: VictimObject::Marker(o.index),
                index: o.index,
                identifier: o.identifier,
                kind: 4,
                position: [p.position.0, p.position.1, o.z],
                flags: o.flags | if o.no_shape { 0x200 } else { 0 },
                undamaged: undamaged(p.definition),
                link: p.cell_link,
            });
        }
        out
    }

    fn in_scan(o: &ScanObject, damage: &DamageDefinition, mask: u32, owner: u32) -> bool {
        if o.flags & 0x10_0000 != 0 && damage.damage_type == 9 {
            return false;
        }
        if o.flags & 0x8200 != 0 || mask & (1 << o.kind) == 0 {
            return false;
        }
        !(damage.flags & 0x2000 != 0 && o.object == VictimObject::Unit(owner))
    }

    fn mesh_cells(&self) -> (i32, i32) {
        self.engine.mesh_cells
    }

    pub(crate) fn cause_area_damage(
        &mut self,
        tick: u32,
        definition: FourCC,
        damage: DamageDefinition,
        owner: u32,
        point: [i32; 3],
    ) -> Result<Option<FourCC>, Unmodelled> {
        let info = self.combat.flight.detonating.take();
        let source = info.and_then(|n| n.source);
        let mut r = i32::from(damage.radius_lower_bound as u16);
        if damage.radius_delta > 0 {
            self.engine_sites.add(draw_site::AREA_DAMAGE_RADIUS, 1);
            crate::engine_rng::log_draw(draw_site::AREA_DAMAGE_RADIUS, self.rng.state);
            r += ((u32::from(self.rng.next_u16()) * damage.radius_delta as u32) >> 16) as i32;
        }
        let r = i32::from(r as i16);
        let radius = r.clamp(0, 0x1000);
        let mask = if damage.damage_type == 10 { 4 } else { 0x5e };
        let ((x0, x1), (y0, y1)) = scan_cells(point, r, self.mesh_cells());
        let mut started = None;
        if x1 <= x0 {
            return Ok(None);
        }
        self.combat.flight.area_scans += 1;
        let mut objects = AreaObjects {
            x: (x0, x1),
            y: (y0, y1),
            cells: None,
        };
        let r = (|| {
            for y in y0..=y1 {
                for x in x0..x1 {
                    self.area_scan_cell(
                        tick,
                        (x, y),
                        source,
                        (definition, &damage, owner, point, info),
                        (radius, mask),
                        &mut started,
                        &mut objects,
                    )?;
                }
            }
            Ok(started)
        })();
        self.combat.flight.area_scans -= 1;
        if self.combat.flight.area_scans == 0 {
            self.combat.flight.relinked.clear();
        }
        r
    }

    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    fn area_scan_cell(
        &mut self,
        tick: u32,
        (x, y): (i32, i32),
        source: Option<crate::explosions::ExplosionSource>,
        (definition, damage, owner, point, info): (
            FourCC,
            &DamageDefinition,
            u32,
            [i32; 3],
            Option<ExplosionInfo>,
        ),
        (radius, mask): (i32, u32),
        started: &mut Option<FourCC>,
        objects: &mut AreaObjects,
    ) -> Result<(), Unmodelled> {
        let damage = *damage;
        let read = |sim: &Self, objects: &mut AreaObjects, below: u64| {
            let mut cell: Vec<ScanObject> = objects
                .cell(sim, source, x, y)
                .iter()
                .filter(|o| o.link < below)
                .copied()
                .collect();
            #[cfg(feature = "identity-checks")]
            assert_eq!(
                cell,
                sim.scan_objects(source, ScanScope::Cell(x, y))
                    .into_iter()
                    .filter(|o| o.position[0] >> 9 == x
                        && o.position[1] >> 9 == y
                        && o.link < below)
                    .collect::<Vec<_>>()
            );
            cell.sort_by_key(|o| std::cmp::Reverse(o.link));
            cell
        };
        let mut cell = read(self, objects, u64::MAX);
        let mut at = 0;
        while at < cell.len() {
            let o = cell[at];
            at += 1;
            let now = self
                .scan_objects(source, ScanScope::One(o.object))
                .into_iter()
                .find(|n| n.object == o.object);
            let Some(o) = now else {
                continue;
            };
            if !Self::in_scan(&o, &damage, mask, owner) {
                continue;
            }
            let d = distance3d(point, o.position);
            if d >= radius {
                continue;
            }
            if let VictimObject::Unit(id) = o.object
                && self.units.get(&id).is_some_and(|u| !u.is_alive())
            {
                return Err(Unmodelled::new(
                    draw_site::DAMAGE_AMOUNT,
                    format!("area damage on monster {id} while it is being killed"),
                ));
            }
            let hit = ObjectHit {
                definition,
                damage,
                owner,
                point,
                info,
                scale: area_scale(d, radius),
                site: draw_site::DAMAGE_AMOUNT,
            };
            objects.cells = None;
            if let Some(g) = self.object_hit(tick, o.object, &hit)? {
                *started = Some(g);
            }
            let relinked = &mut self.combat.flight.relinked;
            if let Some(k) = relinked.iter().rposition(|(i, _)| *i == o.index) {
                let (_, link) = relinked.remove(k);
                cell = read(self, objects, link);
                at = 0;
            }
        }
        Ok(())
    }

    pub(crate) fn cause_delayed_explosion(
        &mut self,
        tick: u32,
        definition: FourCC,
        damage: DamageDefinition,
        owner: u32,
        point: [i32; 3],
    ) -> Result<usize, Unmodelled> {
        if self.combat.explosions.len() >= 0x40
            && let Some(k) =
                eviction_choice(self.combat.explosions.iter().map(|e| (e.reach, e.radius)))
        {
            self.combat.explosions.remove(k);
        }
        let slot = (0..0x40i16)
            .find(|s| self.combat.explosions.iter().all(|e| e.slot != *s))
            .unwrap_or(0);
        let mut r = i32::from(damage.radius_lower_bound as u16);
        if damage.radius_delta > 0 {
            self.engine_sites
                .add(draw_site::DELAYED_EXPLOSION_RADIUS, 1);
            crate::engine_rng::log_draw(draw_site::DELAYED_EXPLOSION_RADIUS, self.rng.state);
            r += ((u32::from(self.rng.next_u16()) * damage.radius_delta as u32) >> 16) as i32;
        }
        let r = i32::from(r as i16);
        let radius = r.clamp(0, 0x1000) as i16;
        let info = self.combat.flight.detonating.take();
        let source = info.and_then(|n| n.source);
        let ((x0, x1), (y0, y1)) = scan_cells(point, r, self.mesh_cells());
        let at = self
            .combat
            .explosions
            .iter()
            .position(|e| e.slot > slot)
            .unwrap_or(self.combat.explosions.len());
        let mut record = Explosion {
            definition,
            damage,
            reach: 0,
            radius,
            victims: Vec::new(),
            owner,
            point,
            created: tick,
            info,
            slot,
            updating: false,
        };
        if x1 <= x0 {
            record.radius = 0;
            record.damage.word_0c = 0;
            self.combat.explosions.insert(at, record);
            return Ok(0);
        }
        let mut objects: Vec<ScanObject> = self.scan_objects(
            source,
            ScanScope::Cells {
                x: (x0, x1),
                y: (y0, y1),
            },
        );
        objects.sort_by_key(|o| {
            (
                o.position[1] >> 9,
                o.position[0] >> 9,
                std::cmp::Reverse(o.link),
            )
        });
        let mut next = 0;
        for o in &objects {
            if o.undamaged || !Self::in_scan(o, &damage, 0x5e, owner) {
                continue;
            }
            let d = distance3d(point, o.position);
            if d < i32::from(radius) {
                push_victim(
                    &mut record.victims,
                    &mut next,
                    Victim {
                        object: o.object,
                        index: o.index,
                        identifier: o.identifier,
                        kind: o.kind,
                        distance: d as i16,
                    },
                    o.flags,
                    |v| {
                        objects
                            .iter()
                            .rfind(|o| o.index == v.index)
                            .map_or(0, |o| o.flags)
                    },
                );
            }
        }
        let n = record.victims.len();
        self.combat.explosions.insert(at, record);
        Ok(n)
    }

    fn victim_now(&self, v: &Victim) -> Option<VictimObject> {
        let w = &self.engine;
        let same = |o: &crate::engine_rng::EngineObject| {
            o.index == v.index && o.identifier == v.identifier
        };
        let in_use = |o: &crate::engine_rng::EngineObject, object| {
            (same(o) && !o.no_shape).then_some(object)
        };
        if let Some(m) = w
            .monsters
            .iter()
            .find(|m| m.identifier != 0 && same(&m.object))
        {
            let unit = m.unit_key()?;
            return if self.units.contains_key(&unit) {
                in_use(&m.object, VictimObject::Unit(unit))
            } else {
                None
            };
        }
        let flight = &self.combat.flight;
        w.scenery
            .iter()
            .enumerate()
            .filter(|(_, sc)| !sc.deleted)
            .find_map(|(i, sc)| in_use(&sc.object, VictimObject::Scenery(i)))
            .or_else(|| {
                flight
                    .flights
                    .iter()
                    .find_map(|p| in_use(p.shape.as_ref()?, VictimObject::Flight(p.id)))
            })
            .or_else(|| {
                flight
                    .dormant
                    .iter()
                    .find_map(|d| in_use(&d.object, VictimObject::Dormant(d.object.index)))
            })
            .or_else(|| {
                w.projectiles
                    .iter()
                    .find_map(|p| in_use(&p.object, VictimObject::Marker(p.object.index)))
            })
    }

    pub(crate) fn update_explosions(&mut self, tick: u32) {
        let mut last = -1i16;
        while let Some(k) = self.combat.explosions.iter().position(|e| e.slot > last) {
            last = self.combat.explosions[k].slot;
            let speed = self.combat.explosions[k].damage.word_0c;
            let e = &mut self.combat.explosions[k];
            e.reach = e.reach.wrapping_add(speed);
            e.updating = true;
            let mut v = 0;
            let mut evicted = false;
            loop {
                let Some(e) = self
                    .combat
                    .explosions
                    .iter()
                    .find(|e| e.slot == last && e.updating)
                else {
                    evicted = true;
                    break;
                };
                let Some(&victim) = e.victims.get(v) else {
                    break;
                };
                if victim.distance >= e.reach {
                    v += 1;
                    continue;
                }
                let hit = ObjectHit {
                    definition: e.definition,
                    damage: e.damage,
                    owner: e.owner,
                    point: e.point,
                    info: e.info,
                    scale: explosion_scale(victim.distance, e.radius),
                    site: draw_site::EXPLOSION_DAMAGE,
                };
                if let Some(object) = self.victim_now(&victim)
                    && let Err(e) = self.object_hit(tick, object, &hit)
                {
                    self.engine_stop.get_or_insert(e);
                }
                if let Some(e) = self
                    .combat
                    .explosions
                    .iter_mut()
                    .find(|e| e.slot == last && e.updating)
                {
                    e.victims[v].distance = 0x7fff;
                }
                v += 1;
            }
            if evicted {
                continue;
            }
            if let Some(k) = self
                .combat
                .explosions
                .iter()
                .position(|e| e.slot == last && e.updating)
            {
                let e = &mut self.combat.explosions[k];
                e.updating = false;
                if e.radius <= e.reach {
                    self.combat.explosions.remove(k);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scenery_object_hit(
        &mut self,
        tick: u32,
        i: usize,
        damage: &DamageDefinition,
        info: Option<ExplosionInfo>,
        amount: i32,
        magnitude: i32,
        point: [i32; 3],
    ) -> Result<Option<FourCC>, Unmodelled> {
        let object = VictimObject::Scenery(i);
        let kind = self.engine.scenery[i].object.kind;
        match kind {
            1 => self.accelerate_scenery(tick, i, damage, amount, magnitude, point),
            4 => {
                self.accelerate_projectile(tick, object, damage, info, amount, magnitude, point)?;
                Ok(None)
            }
            crate::flags::OBJECT_KIND_FLAG => {
                let flying = {
                    let flag_kind = self
                        .engine
                        .flag_data
                        .flag(self.engine.scenery[i].object.owner)
                        .map_or(0, |f| f.kind);
                    if flag_kind == 1 {
                        self.engine.scenery[i].object.kind = 4;
                    }
                    flag_kind == 1
                };
                if flying {
                    self.accelerate_projectile(
                        tick, object, damage, info, amount, magnitude, point,
                    )?;
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn accelerate_projectile_object(
        &mut self,
        tick: u32,
        object: VictimObject,
        damage: &DamageDefinition,
        info: Option<ExplosionInfo>,
        amount: i32,
        magnitude: i32,
        point: [i32; 3],
    ) -> Result<(), Unmodelled> {
        if let VictimObject::Dormant(index) = object {
            let flight = &mut self.combat.flight;
            if let Some(d) = (flight.dormant.iter_mut()).find(|d| {
                d.object.index == index && d.object.kind == crate::flags::OBJECT_KIND_FLAG
            }) {
                let flag_kind = self
                    .engine
                    .flag_data
                    .flag(d.object.owner)
                    .map_or(0, |f| f.kind);
                if flag_kind != 1 {
                    return Ok(());
                }
                d.object.kind = 4;
            }
        }
        self.accelerate_projectile(tick, object, damage, info, amount, magnitude, point)
    }

    pub(crate) fn monster_bumped_ball(
        &mut self,
        monster: usize,
        object: i16,
        magnitude: i16,
    ) -> Result<(), Unmodelled> {
        let tick = self.current_tick;
        let index = i32::from(object);
        let victim = if let Some(i) =
            (self.engine.scenery.iter()).position(|s| s.object.index == index && !s.deleted)
        {
            self.engine.scenery[i].object.kind = 4;
            VictimObject::Scenery(i)
        } else if let Some(d) =
            (self.combat.flight.dormant.iter_mut()).find(|d| d.object.index == index)
        {
            d.object.kind = 4;
            VictimObject::Dormant(index)
        } else {
            return Err(Unmodelled::new(
                draw_site::COLLIDE_FLYING_FLAG,
                format!("a bumped ball's object {object} is neither scenery nor at rest"),
            ));
        };
        let m = &self.engine.monsters[monster];
        let point = [m.position.0, m.position.1, m.object.z];
        let damage = DamageDefinition {
            damage_type: 4,
            flags: 0,
            damage_lower_bound: 0,
            damage_delta: 0,
            radius_lower_bound: 0,
            radius_delta: 0,
            word_0c: 0,
            word_0e: 0,
        };
        self.accelerate_projectile(tick, victim, &damage, None, 0, i32::from(magnitude), point)
    }

    pub(crate) fn object_hit(
        &mut self,
        tick: u32,
        object: VictimObject,
        hit: &ObjectHit,
    ) -> Result<Option<FourCC>, Unmodelled> {
        let damage = hit.damage;
        let mut amount = i32::from(damage.damage_lower_bound as u16);
        if damage.damage_delta > 0 {
            self.engine_sites.add(hit.site, 1);
            crate::engine_rng::log_draw(hit.site, self.rng.state);
            amount += ((u32::from(self.rng.next_u16()) * damage.damage_delta as u32) >> 16) as i32;
        }
        let mut amount = i32::from(amount as i16);
        if let Some(f) = hit.info.map(|i| f32::from_bits(i.factor_bits))
            && amount != 0
            && f != 0.0
            && (self.engine.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3)
        {
            let v = amount as f32 * f;
            amount = if v.abs() < 2_147_483_648.0 {
                i32::from(v as i32 as i16)
            } else {
                0
            };
        }
        let mut scale = hit.scale;
        if damage.flags & 1 != 0 {
            let Some(info) = hit.info else {
                self.combat.flight.reports.push((
                    tick,
                    Unmodelled::new(
                        draw_site::EXPLOSION_VELOCITY_SCALE,
                        "explosion scaled by the velocity, no info",
                    ),
                ));
                return Ok(None);
            };
            scale = velocity_scale(info.velocity, scale);
        }
        let amount = i32::from(((amount * i32::from(scale)) >> 8) as i16);
        let magnitude = if damage.flags & 0x1000 != 0 {
            0
        } else {
            i32::from(((i32::from(damage.word_0e) * amount) >> 12) as i16).min(0x100)
        };
        let (definition, owner, point, info) = (hit.definition, hit.owner, hit.point, hit.info);
        match object {
            VictimObject::Scenery(i) => {
                self.scenery_object_hit(tick, i, &damage, info, amount, magnitude, point)
            }
            VictimObject::Flight(_) | VictimObject::Dormant(_) | VictimObject::Marker(_) => {
                self.accelerate_projectile_object(
                    tick, object, &damage, info, amount, magnitude, point,
                )?;
                Ok(None)
            }
            VictimObject::Unit(unit) => {
                if damage.flags & 0x100 != 0 {
                    return Ok(None);
                }
                if let Some(info) = info
                    && (self
                        .engine
                        .monster_index_by_identifier(unit as u16)
                        .is_some())
                {
                    let mut h = crate::engine_damage::DamageInfo::new(
                        info.unit,
                        info.attacker,
                        info.attacker_id,
                    );
                    h.point = point;
                    h.velocity = info.velocity;
                    h.flags = info.flags;
                    h.mana = info.mana;
                    h.factor = f32::from_bits(info.factor_bits);
                    h.scale = scale;
                    h.amount = amount as i16;
                    h.magnitude = magnitude as i16;
                    return self.engine_explosion_hit(tick, unit, owner, definition, &damage, h);
                }
                self.accelerate_monster(tick, unit, owner, definition, &damage, amount, point);
                Ok(None)
            }
        }
    }

    fn projectile_obje(
        &self,
        definition: FourCC,
    ) -> Result<mandrake_tag::object::ObjectPhysics, Unmodelled> {
        (self.engine.catalog.flights.get(&definition))
            .map(|f| f.physics)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::PROJECTILE_OBJECT_MISSING,
                    format!("proj {definition} (or its obje) not loaded"),
                )
            })
    }

    fn damage_projectile_object(
        &mut self,
        object: VictimObject,
        damage: &DamageDefinition,
        amount: i32,
        source: Option<ExplosionSource>,
    ) -> Result<bool, Unmodelled> {
        let (modifiers, threshold, vitality, max_vitality) = match object {
            VictimObject::Flight(id) if source.is_some_and(|s| s.id == id) => {
                let s = source.expect("tested");
                let d = self.projectile_obje(s.definition)?;
                (
                    d.damage_modifiers,
                    d.damage_threshold,
                    s.vitality,
                    s.max_vitality,
                )
            }
            VictimObject::Flight(id) => {
                let p = self.combat.flight.flights.iter().find(|p| p.id == id);
                let Some(p) = p.filter(|p| p.shape.is_some()) else {
                    return Ok(false);
                };
                let s = p
                    .shape
                    .as_ref()
                    .map_or((0, 0), |s| (s.vitality, s.max_vitality));
                let d = self.projectile_obje(p.definition)?;
                (d.damage_modifiers, d.damage_threshold, s.0, s.1)
            }
            VictimObject::Dormant(i) => {
                let Some(p) = self
                    .combat
                    .flight
                    .dormant
                    .iter()
                    .find(|d| d.object.index == i)
                else {
                    return Ok(false);
                };
                let d = self.projectile_obje(p.definition)?;
                (
                    d.damage_modifiers,
                    d.damage_threshold,
                    p.object.vitality,
                    p.object.max_vitality,
                )
            }
            VictimObject::Marker(i) => {
                let Some(p) = self.engine.projectiles.iter().find(|p| p.object.index == i) else {
                    return Ok(false);
                };
                let (v, m) = (p.object.vitality, p.object.max_vitality);
                let d = self.projectile_obje(p.definition)?;
                (d.damage_modifiers, d.damage_threshold, v, m)
            }
            VictimObject::Scenery(i) => {
                let o = &self.engine.scenery[i].object;
                let (index, vitality, max_vitality) = (o.index, o.vitality, o.max_vitality);
                let Some(d) = self.combat.flight.scenery_damage.get(&index) else {
                    return Err(Unmodelled::new(
                        draw_site::PROJECTILE_OBJECT_MISSING,
                        "scenery without its definition",
                    ));
                };
                (d.modifiers, d.threshold, vitality, max_vitality)
            }
            VictimObject::Unit(_) => return Ok(false),
        };
        let r = {
            let rng = &mut self.rng;
            let sites = &mut self.engine_sites;
            damage_plain_object(
                modifiers,
                threshold,
                damage,
                amount,
                vitality,
                max_vitality,
                &mut || {
                    sites.add(draw_site::DAMAGE_MONSTER_OBJECT_CHANCE, 1);
                    crate::engine_rng::log_draw(draw_site::DAMAGE_MONSTER_OBJECT_CHANCE, rng.state);
                    rng.next_u16()
                },
            )
        };
        match object {
            VictimObject::Flight(id) => {
                if let Some(s) = self
                    .combat
                    .flight
                    .flights
                    .iter_mut()
                    .find(|p| p.id == id)
                    .and_then(|p| p.shape.as_mut())
                {
                    s.vitality = r.vitality;
                    s.max_vitality = r.max_vitality;
                }
                let definition = match source {
                    Some(s) if s.id == id => Some(s.definition),
                    _ => self
                        .combat
                        .flight
                        .flights
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.definition),
                };
                let indestructible = definition
                    .and_then(|d| self.engine.catalog.flights.get(&d))
                    .is_some_and(|f| f.flags & 0x2_0000 != 0);
                if indestructible {
                    if let Some(p) = self.combat.flight.flights.iter_mut().find(|p| p.id == id) {
                        p.target = None;
                    }
                    return Ok(false);
                }
            }
            VictimObject::Dormant(i) => {
                if let Some(d) = self
                    .combat
                    .flight
                    .dormant
                    .iter_mut()
                    .find(|d| d.object.index == i)
                {
                    d.object.vitality = r.vitality;
                    d.object.max_vitality = r.max_vitality;
                }
            }
            VictimObject::Marker(i) => {
                if let Some(p) = self
                    .engine
                    .projectiles
                    .iter_mut()
                    .find(|p| p.object.index == i)
                {
                    p.object.vitality = r.vitality;
                    p.object.max_vitality = r.max_vitality;
                }
            }
            VictimObject::Scenery(i) => {
                let w = &mut self.engine;
                w.scenery[i].object.vitality = r.vitality;
                w.scenery[i].object.max_vitality = r.max_vitality;
            }
            VictimObject::Unit(_) => {}
        }
        Ok(r.destroyed)
    }

    pub fn wake_object(
        &mut self,
        tick: u32,
        object: VictimObject,
        unit: crate::statistics::UnitRef,
    ) -> Result<Option<i32>, Unmodelled> {
        let w = &self.engine;
        let (definition, old, position, facing, elevation, record_36, flags_3d) = match object {
            VictimObject::Dormant(i) => {
                let Some(d) = self
                    .combat
                    .flight
                    .dormant
                    .iter()
                    .find(|d| d.object.index == i)
                else {
                    return Ok(None);
                };
                (
                    d.definition,
                    d.object.clone(),
                    d.position,
                    d.facing,
                    d.elevation,
                    d.record_36,
                    d.flags_3d,
                )
            }
            VictimObject::Scenery(i) => {
                let sc = &w.scenery[i];
                let Some(definition) = sc.projectile else {
                    return Err(Unmodelled::new(
                        draw_site::WAKE_SCENERY_NO_PROJECTILE,
                        "scenery of kind 4 without its proj",
                    ));
                };
                (
                    definition,
                    sc.object.clone(),
                    [sc.position.0, sc.position.1, sc.object.z],
                    sc.facing,
                    0,
                    0,
                    0,
                )
            }
            VictimObject::Marker(i) => {
                let Some(p) = w.projectiles.iter().find(|p| p.object.index == i) else {
                    return Ok(None);
                };
                (
                    p.definition,
                    p.object.clone(),
                    [p.position.0, p.position.1, p.object.z],
                    p.facing,
                    p.elevation,
                    -1,
                    0,
                )
            }
            _ => return Ok(None),
        };
        let desc = crate::projectile_groups::ProjectileDescriptor {
            definition,
            flags: if old.shading_slot == 1 { 6 } else { 0 },
            position,
            velocity: old.velocity,
            facing,
            elevation,
            word_1c: 0x100,
            owner_monster: -1,
            parent: -1,
            owner_object: old.index,
            target: -1,
            target_point: [0; 3],
        };
        let source = crate::projectile_groups::ObjectSnapshot {
            index: old.index,
            position,
            velocity: old.velocity,
            facing,
            elevation,
            radius: 0,
            bottom: 0,
            top: 0,
            kind: 4,
            owner: old.owner,
            collection: old.collection,
            sequence: old.sequence,
            frame: old.frame,
            view: old.view,
            shading: old.permutation,
            shading_slot: old.shading_slot,
            core: old.core,
        };
        let made = crate::projectile_groups::new_projectile(
            &mut crate::projectile_groups::SimSpawn {
                sim: self,
                tick,
                source: Some(source),
                reserved: None,
                source_record: None,
                engine: true,
                updating_object: None,
            },
            &desc,
        )?;
        let Some(record) = made else {
            return Ok(None);
        };
        match object {
            VictimObject::Dormant(i) => self.combat.flight.dormant.retain(|d| d.object.index != i),
            VictimObject::Marker(i) => {
                let w = &mut self.engine;
                w.projectiles.retain(|p| p.object.index != i);
            }
            VictimObject::Scenery(i) => {
                {
                    let w = &mut self.engine;
                    w.scenery[i].deleted = true;
                }
                self.combat.flight.scenery.retain(|c| c.object != old.index);
            }
            _ => {}
        }
        let link = self.engine.next_cell_link();
        if self.combat.flight.area_scans > 0 {
            self.combat.flight.relinked.push((old.index, link));
        }
        let mut freed = None;
        if let Some(p) = self
            .combat
            .flight
            .flights
            .iter_mut()
            .find(|p| p.index == record)
        {
            p.record_0a = unit;
            p.record_36 = record_36;
            p.record_flags |= u16::from(flags_3d & 0xe0);
            p.woken = true;
            p.cell_link = link;
            if let Some(s) = p.shape.as_mut() {
                s.flags |= old.flags & 0x410_0400;
                freed = Some(s.index);
                s.index = old.index;
                s.identifier = old.identifier;
            }
        }
        if let Some(f) = freed {
            self.engine.free_objects.insert(f);
        }
        Ok(Some(record))
    }

    fn detonate_by_damage(
        &mut self,
        tick: u32,
        record: i32,
        point: [i32; 3],
        scale: i32,
    ) -> Result<(), Unmodelled> {
        let flights = &mut self.combat.flight.flights;
        let Some(at) = flights.iter().position(|p| p.index == record) else {
            return Ok(());
        };
        let mut q = flights.remove(at);
        match self.detonate_record_at(&mut q, point, scale, tick) {
            r @ (Ok(true) | Err(_)) => {
                let at = at.min(self.combat.flight.flights.len());
                self.combat.flight.flights.insert(at, q);
                r.map(|_| ())
            }
            Ok(false) => Ok(()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn accelerate_projectile(
        &mut self,
        tick: u32,
        object: VictimObject,
        damage: &DamageDefinition,
        info: Option<ExplosionInfo>,
        amount: i32,
        magnitude: i32,
        point: [i32; 3],
    ) -> Result<(), Unmodelled> {
        if damage.damage_type == 3 {
            return Ok(());
        }
        let unit = info.map_or(crate::statistics::UnitRef::NONE, |n| n.unit);
        let source = info.and_then(|n| n.source);
        let record_of = |sim: &mut Self| -> Result<Option<i32>, Unmodelled> {
            match object {
                VictimObject::Flight(id) => Ok(sim
                    .combat
                    .flight
                    .flights
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.index)),
                _ => sim.wake_object(tick, object, unit),
            }
        };
        let flagged = |sim: &Self, r: i32| {
            sim.combat
                .flight
                .flights
                .iter()
                .find(|p| p.index == r)
                .is_none_or(|p| p.record_flags & 1 != 0)
        };
        if damage.flags & 0x1000 != 0 {
            if !self.damage_projectile_object(object, damage, amount, source)? {
                return Ok(());
            }
            let Some(r) = record_of(self)? else {
                return Ok(());
            };
            if flagged(self, r) {
                return Ok(());
            }
            return self.detonate_by_damage(tick, r, point, 0);
        }
        if magnitude == 0 {
            return Ok(());
        }
        let Some(r) = record_of(self)? else {
            return Ok(());
        };
        if flagged(self, r) {
            return Ok(());
        }
        let Some(p) = self.combat.flight.flights.iter().find(|p| p.index == r) else {
            return Ok(());
        };
        let (id, definition) = (p.id, p.definition);
        let obje = self.projectile_obje(definition)?;
        if obje.flags & 0x80 == 0
            && let Some(p) = self.combat.flight.flights.iter_mut().find(|p| p.index == r)
        {
            let from = [p.object.x, p.object.y, p.object.z];
            let v = crate::projectile_groups::line_segment_to_vector(
                from,
                [point[0], point[1], point[2] - 0x80],
                magnitude,
            );
            p.object.vx = p.object.vx.wrapping_add(v[0]);
            p.object.vy = p.object.vy.wrapping_add(v[1]);
            p.object.vz = p.object.vz.wrapping_add(v[2]);
            if p.object.shape_flags & crate::projectiles::OBJECT_AT_REST != 0 && p.object.vz < 0 {
                p.object.vz = p.object.vz.wrapping_neg();
            }
            p.object.shape_flags &= !0x110;
            if let Some(s) = p.shape.as_mut() {
                s.flags &= !0x110;
            }
        }
        let destroyed =
            self.damage_projectile_object(VictimObject::Flight(id), damage, amount, None)?;
        let shatter = self.engine.catalog.projectiles.get(&definition).map(|d| {
            (
                d.raw.get(0xdf) == Some(&1),
                d.word(0x64),
                d.word(0x66) as u16,
            )
        });
        let shatters = !destroyed
            && shatter.is_some_and(|(flag, limit, _)| flag && magnitude > i32::from(limit))
            && {
                let threshold = shatter.map_or(0, |(_, _, t)| t);
                self.engine_sites
                    .add(crate::projectiles::FREQUENCY_DRAW_SITE, 1);
                crate::engine_rng::log_draw(
                    crate::projectiles::FREQUENCY_DRAW_SITE,
                    self.rng.state,
                );
                crate::combat::frequency_corrected(&mut self.rng, threshold, 0x6b8)
            };
        if destroyed || shatters {
            return self.detonate_by_damage(tick, r, point, magnitude);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn accelerate_monster(
        &mut self,
        tick: u32,
        victim: u32,
        attacker: u32,
        definition: FourCC,
        damage: &DamageDefinition,
        amount: i32,
        point: [i32; 3],
    ) {
        if damage_object_draws(damage.damage_type) {
            self.engine_sites
                .add(draw_site::DAMAGE_MONSTER_OBJECT_CHANCE, 1);
            crate::engine_rng::log_draw(draw_site::DAMAGE_MONSTER_OBJECT_CHANCE, self.rng.state);
            self.rng.next_u16();
        }
        let (threshold, has_29) = self
            .units
            .get(&victim)
            .and_then(|u| self.combat.definitions.monsters.get(&u.archetype))
            .map_or((0xffff, false), |m| {
                (
                    m.unit.kill_ratio_threshold,
                    m.unit.death_groups[2].is_some(),
                )
            });
        let Some(unit) = self.units.get_mut(&victim) else {
            return;
        };
        let max_hp = unit.max_health.to_i32();
        let result = damage_object(unit.health.to_i32(), max_hp, amount);
        unit.health = Fixed16::from_i32(result.hp_after);
        self.combat.events.push(CombatEvent::UnitDamaged {
            tick,
            unit_id: victim,
            attacker,
            definition,
            damage_type: damage.damage_type,
            amount,
            hp_before: result.hp_before,
            hp_after: result.hp_after,
            lethal: result.lethal,
        });
        if result.lethal {
            let code = kill_code(damage.damage_type, amount, max_hp, threshold, false, has_29);
            self.combat.events.push(CombatEvent::UnitKilled {
                tick,
                unit_id: victim,
                killer: attacker,
                animation_code: code,
            });
            self.kill_monster_group(tick, victim, code, damage, amount, Some(point));
            self.units.remove(&victim);
        } else {
            self.combat.events.push(CombatEvent::HitReaction {
                tick,
                unit_id: victim,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_65th_explosion_evicts_the_record_furthest_through_its_radius() {
        let r = [(10, 20), (30, 40), (15, 20), (5, 0)];
        assert_eq!(eviction_choice(r.into_iter()), Some(1));
        assert_eq!(eviction_choice([(0, 0)].into_iter()), Some(0));
        assert_eq!(eviction_choice(std::iter::empty()), None);
    }

    #[test]
    fn a_velocity_scaled_hit_multiplies_the_scale_by_the_speed() {
        assert_eq!(velocity_scale([3, 4, 0], 0x100), 5);
        assert_eq!(velocity_scale([0, 0, 0], 0x100), 0);
        assert_eq!(velocity_scale([300, 400, 0], 0x80), 250);
    }

    #[test]
    fn area_scale_uses_three_quarters_of_the_distance() {
        assert_eq!(area_scale(0, 512), 0x100);
        assert_eq!(area_scale(400, 512), 0x100 - 150);
    }

    #[test]
    fn the_scan_ends_its_columns_before_the_end_cell_and_includes_the_end_row() {
        let (x, y) = scan_cells([1000, 1000, 0], 100, (64, 64));
        assert_eq!((x, y), ((1, 3), (1, 3)));
        let (x, y) = scan_cells([1000, 1000, 0], 100, (2, 2));
        assert_eq!((x, y), ((1, 1), (1, 1)));
        let (x, _) = scan_cells([100, 1000, 0], 200, (64, 64));
        assert_eq!(x.0, 0);
    }

    #[test]
    fn full_victim_slots_only_give_way_to_flag_0x400000() {
        let v = |index| Victim {
            object: VictimObject::Dormant(index),
            index,
            identifier: 0,
            kind: 4,
            distance: 1,
        };
        let mut slots: Vec<Victim> = (0..0x58).map(v).collect();
        let flags = |w: &Victim| if w.index < 2 { 0x40_0000 } else { 0 };
        let mut next = 0;
        push_victim(&mut slots, &mut next, v(100), 0, flags);
        assert!(slots.iter().all(|s| s.index != 100), "no flag: left out");
        push_victim(&mut slots, &mut next, v(101), 0x40_0000, flags);
        assert_eq!(slots[2].index, 101);
        assert_eq!(next, 2);
        assert_eq!(slots.len(), 0x58);
    }

    #[test]
    fn scenery_damage_is_resisted_up_to_the_threshold() {
        let mut def = SceneryDamage {
            flags: 0,
            destroyed_sequence: -1,
            damaged_group: None,
            destroyed_group: None,
            object_flags: 0,
            modifiers: [0x100; 9],
            threshold: 0x7fff,
            largest_scale: 0x100,
            model_permutation: 0,
        };
        let damage = DamageDefinition {
            damage_type: 0,
            flags: 0,
            damage_lower_bound: 0,
            damage_delta: 0,
            radius_lower_bound: 0,
            radius_delta: 0,
            word_0c: 0,
            word_0e: 0,
        };
        let mut no_draw = || -> u16 { panic!("type 0 does not draw") };
        let r = damage_scenery_object(&def, &damage, 801, 256, 256, &mut no_draw);
        assert!(r.resisted && !r.destroyed);
        assert_eq!(r.vitality, 256);
        def.threshold = 0;
        let r = damage_scenery_object(&def, &damage, 100, 256, 256, &mut no_draw);
        assert!(!r.resisted && !r.destroyed);
        assert_eq!(r.vitality, 156);
        let r = damage_scenery_object(&def, &damage, 459, 256, 256, &mut no_draw);
        assert!(r.destroyed);
        assert_eq!(r.vitality, 0);
    }

    #[test]
    fn scale_and_distance() {
        assert_eq!(explosion_scale(0, 1152), 0x100);
        assert_eq!(explosion_scale(100, 0), 0x100);
        assert_eq!(explosion_scale(1152, 1152), 0x100 - 192);
        assert_eq!(explosion_scale(0x7fff, 1152), 0);
        assert_eq!(distance3d([0, 0, 0], [3, 4, 0]), 5);
        assert_eq!(distance3d([0, 0, 0], [0x8000, 0, 0]), 0x7fff);
    }
}
