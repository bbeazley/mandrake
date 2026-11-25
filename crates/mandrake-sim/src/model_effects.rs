use crate::draw_site;
use crate::engine_rng::{EngineRng, EngineWorld, Unmodelled};
use crate::map_actions::{MapActionContext, MapActions};

thread_local! {
    pub static MODEL_EFFECT_LOG: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

fn log(line: impl FnOnce() -> String) {
    MODEL_EFFECT_LOG.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            v.push(line());
        }
    });
}

pub fn effect_lines(world: &EngineWorld) -> Vec<String> {
    let mut out = Vec::new();
    for (k, e) in world.model_effects.slots.iter().enumerate() {
        let Some(e) = e else { continue };
        let m = world.models.get(e.model as usize);
        let words = m.map_or_else(String::new, |m| {
            format!(
                "{} {} {} {} {} {} {} {}",
                m.identifier,
                m.permutation,
                m.next_permutation,
                m.blend,
                m.transparency,
                m.yaw,
                m.pitch,
                m.scale
            )
        });
        out.push(format!(
            "E {k} {} {} {} {:#x} {} {} | {words}",
            e.model, e.action, e.elapsed, e.flags, e.duration, e.permutation
        ));
    }
    out
}

pub const MAX_MODEL_EFFECTS: usize = 16;

pub const FADE_IN: u16 = 1;
pub const FADE_OUT: u16 = 2;
pub const DELETE: u16 = 4;
pub const SCALE: u16 = 8;
pub const PROJECTILE: u16 = 0x10;
pub const PERMUTATION_ONLY: u16 = 0x20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelEffectDescriptor {
    pub flags: u16,
    pub duration: i16,
    pub yaw: i16,
    pub pitch: i16,
    pub new_permutation: i32,
    pub permutation: i32,
    pub material: i32,
    pub material_final: u16,
    pub scale: [i16; 2],
    pub origin: [i32; 3],
    pub projectile: u32,
}

impl Default for ModelEffectDescriptor {
    fn default() -> Self {
        Self {
            flags: 0,
            duration: -1,
            yaw: 0,
            pitch: 0,
            new_permutation: -1,
            permutation: -1,
            material: -1,
            material_final: 0,
            scale: [0; 2],
            origin: [0; 3],
            projectile: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelEffect {
    pub model: i16,
    pub action: i32,
    pub elapsed: i16,
    pub flags: u16,
    pub duration: i16,
    pub yaw: i16,
    pub pitch: i16,
    pub permutation: i16,
    pub material: i16,
    pub material_from: u16,
    pub material_to: u16,
    pub scale: [i16; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelEffects {
    pub slots: Vec<Option<ModelEffect>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewModelEffect {
    pub created: bool,
    pub error: bool,
}

impl ModelEffects {
    fn new_datum(&mut self) -> Option<usize> {
        if let Some(k) = self.slots.iter().position(Option::is_none) {
            return Some(k);
        }
        (self.slots.len() < MAX_MODEL_EFFECTS).then(|| {
            self.slots.push(None);
            self.slots.len() - 1
        })
    }

    fn delete_datum(&mut self, k: usize) {
        self.slots[k] = None;
        while self.slots.last().is_some_and(Option::is_none) {
            self.slots.pop();
        }
    }

    pub fn any(&self) -> bool {
        self.slots.iter().any(Option::is_some)
    }
}

fn model_mut(
    world: &mut EngineWorld,
    model: i16,
    site: u32,
) -> Result<&mut crate::models::WorldModel, Unmodelled> {
    world
        .models
        .get_mut(model as usize)
        .ok_or_else(|| Unmodelled::new(site, format!("no record of model {model}")))
}

pub fn new_model_effect(
    world: &mut EngineWorld,
    model: i16,
    action: i32,
    d: &ModelEffectDescriptor,
    rng: &mut EngineRng,
) -> Result<NewModelEffect, Unmodelled> {
    log(|| {
        format!(
            "N {model} {action} {:#x} {} {} {} {} {} {} {} {} {}",
            d.flags,
            d.duration,
            d.yaw,
            d.pitch,
            d.new_permutation,
            d.permutation,
            d.material,
            d.material_final,
            d.scale[0],
            d.scale[1]
        )
    });
    let has_action = action != -1;
    let none = NewModelEffect {
        created: false,
        error: false,
    };
    if d.flags & PERMUTATION_ONLY != 0 {
        let found = world
            .model_effects
            .slots
            .iter()
            .rposition(|e| e.is_some_and(|e| e.model == model));
        if let Some(k) = found {
            if d.new_permutation != -1 {
                let mut e = world.model_effects.slots[k].expect("found");
                let m = model_mut(world, model, draw_site::MODEL_EFFECT_NEW_PERMUTATION)?;
                m.permutation = e.permutation;
                e.permutation = d.new_permutation as i16;
                m.next_permutation = e.permutation;
                e.elapsed = e.duration.wrapping_sub(e.elapsed);
                world.model_effects.slots[k] = Some(e);
            }
            return Ok(none);
        }
    }
    let Some(k) = world.model_effects.new_datum() else {
        return Ok(NewModelEffect {
            created: false,
            error: has_action,
        });
    };
    if model == -1 || d.duration < 0 {
        world.model_effects.delete_datum(k);
        return Ok(NewModelEffect {
            created: false,
            error: has_action,
        });
    }
    let m = model_mut(world, model, draw_site::MODEL_EFFECT_NEW)?;
    let start = if d.permutation == -1 {
        m.permutation
    } else {
        d.permutation as i16
    };
    m.next_permutation = start;
    m.permutation = start;
    let end = if d.new_permutation == -1 {
        start
    } else {
        d.new_permutation as i16
    };
    m.next_permutation = end;
    let material = d.material as i16;
    let material_from = surfaces(world, model)
        .and_then(|s| s.iter().find(|s| s.material == material))
        .map_or(0, |s| s.transparency);
    let scale = if d.flags & SCALE != 0 {
        d.scale
    } else {
        [0x200, 0x200]
    };
    world.model_effects.slots[k] = Some(ModelEffect {
        model,
        action,
        elapsed: 0,
        flags: d.flags,
        duration: d.duration,
        yaw: d.yaw,
        pitch: d.pitch,
        permutation: end,
        material,
        material_from,
        material_to: d.material_final,
        scale,
    });
    let mut error = false;
    if d.flags & PROJECTILE != 0 {
        let definition = mandrake_common::FourCC::new(d.projectile.to_be_bytes());
        if world.catalog.projectiles.contains_key(&definition) {
            let desc = crate::projectile_groups::ProjectileDescriptor {
                definition,
                flags: 1,
                position: d.origin,
                velocity: [0; 3],
                facing: 0,
                elevation: 0,
                word_1c: 0x100,
                owner_monster: -1,
                parent: -1,
                owner_object: -1,
                target: -1,
                target_point: [0; 3],
            };
            crate::projectile_groups::new_projectile(
                &mut crate::projectile_groups::EngineSpawn {
                    world: &mut *world,
                    rng: &mut *rng,
                    source: None,
                    host: None,
                },
                &desc,
            )?;
        } else {
            error = has_action;
        }
    }
    Ok(NewModelEffect {
        created: true,
        error,
    })
}

fn surfaces(world: &EngineWorld, model: i16) -> Option<&Vec<crate::models::SurfaceMaterial>> {
    let tag = world.models.get(model as usize)?.geometry_tag?;
    world.geometry_surfaces.get(&tag)
}

fn set_material_transparency(world: &mut EngineWorld, model: i16, material: i16, value: u16) {
    let Some(tag) = world
        .models
        .get(model as usize)
        .and_then(|m| m.geometry_tag)
    else {
        return;
    };
    if let Some(list) = world.geometry_surfaces.get_mut(&tag) {
        for s in list.iter_mut().filter(|s| s.material == material) {
            s.transparency = value;
        }
    }
}

pub fn scale_matrix(m: &mut [i16; 9], scale: [i16; 3]) {
    for (i, v) in m.iter_mut().enumerate() {
        *v = ((i32::from(*v) * i32::from(scale[i % 3])) >> 9) as i16;
    }
}

pub fn world_matrix_inverse(m: &[i16; 9]) -> [i16; 9] {
    let e = |i: usize| i32::from(m[i]);
    let p = |a: usize, b: usize| (e(a) * e(b)) >> 9;
    let c0 = (p(4, 8) - p(5, 7)) as i16;
    let c1 = (p(5, 6) - p(3, 8)) as i16;
    let c2 = (p(3, 7) - p(4, 6)) as i16;
    let det = ((i32::from(c1) * e(1)) >> 9)
        + ((e(0) * i32::from(c0)) >> 9)
        + ((i32::from(c2) * e(2)) >> 9);
    let det = i32::from(det as i16);
    if det == 0 {
        return [0; 9];
    }
    let q = |cofactor: i32| ((i32::from(cofactor as i16) << 9) / det) as i16;
    let mut out = [0i16; 9];
    out[0] = q(i32::from(c0));
    out[3] = q(p(5, 6) - p(3, 8));
    out[6] = q(p(3, 7) - p(6, 4));
    out[1] = q(p(7, 2) - p(1, 8));
    out[4] = q(p(8, 0) - p(2, 6));
    out[7] = q(p(6, 1) - p(0, 7));
    out[2] = q(p(1, 5) - p(4, 2));
    out[5] = q(p(2, 3) - p(0, 5));
    out[8] = q(p(0, 4) - p(1, 3));
    out
}

pub fn geometry_matrices(world: &mut EngineWorld, model: i16) -> Result<(), Unmodelled> {
    let m = model_mut(world, model, draw_site::MODEL_EFFECT_MATRICES)?;
    let mut matrix = mandrake_tag::model_hit::model_world_matrix(m.yaw, m.pitch);
    if m.scale == 0x200 {
        m.placement.inverse = mandrake_tag::model_hit::model_inverse_matrix(&matrix);
    } else {
        scale_matrix(&mut matrix, [m.scale; 3]);
        m.placement.inverse = world_matrix_inverse(&matrix);
    }
    m.placement.matrix = matrix;
    world.cells_changed = true;
    Ok(())
}

impl ModelEffects {
    fn update(
        &mut self,
        world: &mut EngineWorld,
        mut actions: Option<&mut MapActions>,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<(), Unmodelled> {
        let mut k = 0;
        while k < self.slots.len() {
            let Some(mut e) = self.slots[k] else {
                k += 1;
                continue;
            };
            if e.model == -1 {
                k += 1;
                continue;
            }
            e.elapsed = e.elapsed.wrapping_add(1);
            self.slots[k] = Some(e);
            if e.elapsed < e.duration || e.duration == -1 {
                self.run(world, e)?;
            } else {
                self.end(k, world, e, actions.as_deref_mut(), rng, time)?;
            }
            k += 1;
        }
        Ok(())
    }

    fn run(&mut self, world: &mut EngineWorld, e: ModelEffect) -> Result<(), Unmodelled> {
        let mut matrices = false;
        if e.yaw != 0 || e.pitch != 0 {
            let m = model_mut(world, e.model, draw_site::MODEL_EFFECT_SPIN)?;
            m.yaw = m.yaw.wrapping_add(e.yaw as u16);
            m.pitch = m.pitch.wrapping_add(e.pitch as u16);
            if e.duration == -1 {
                return geometry_matrices(world, e.model);
            }
            matrices = true;
        } else if e.duration == -1 {
            return Ok(());
        }
        let elapsed = i32::from(e.elapsed);
        let duration = i32::from(e.duration);
        let blend = ((elapsed << 16).wrapping_sub(elapsed) / duration) as u16;
        let m = model_mut(world, e.model, draw_site::MODEL_EFFECT_BLEND)?;
        m.blend = blend;
        if e.flags & FADE_IN != 0 {
            m.transparency = !blend;
        }
        if e.flags & FADE_OUT != 0 {
            m.transparency = blend;
        }
        if e.flags & SCALE != 0 {
            let (s0, s1) = (i32::from(e.scale[0]), i32::from(e.scale[1]));
            m.scale = ((s1 - s0) * elapsed / duration + s0) as i16;
            matrices = true;
        }
        if e.material != -1 {
            let (from, to) = (i32::from(e.material_from), i32::from(e.material_to));
            let value = ((to - from) * elapsed / duration + from) as u16;
            set_material_transparency(world, e.model, e.material, value);
        }
        if matrices {
            geometry_matrices(world, e.model)?;
        }
        Ok(())
    }

    fn end(
        &mut self,
        k: usize,
        world: &mut EngineWorld,
        e: ModelEffect,
        actions: Option<&mut MapActions>,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<(), Unmodelled> {
        if e.action != -1
            && let Some(actions) = actions
            && let Some(j) = actions.index_of(e.action)
        {
            log(|| format!("S {j}"));
            actions.succeed(
                j,
                &mut MapActionContext {
                    world: &mut *world,
                    rng: &mut *rng,
                    time,
                },
            )?;
        }
        let m = model_mut(world, e.model, draw_site::MODEL_EFFECT_END)?;
        if e.flags & FADE_IN != 0 {
            m.transparency = 0;
        }
        if e.flags & FADE_OUT != 0 {
            m.transparency = 0xffff;
        }
        m.permutation = e.permutation;
        m.next_permutation = e.permutation;
        m.blend = 0;
        let mut matrices = false;
        if e.flags & SCALE != 0 {
            m.scale = e.scale[1];
            matrices = true;
        }
        if e.material != -1 {
            set_material_transparency(world, e.model, e.material, e.material_to);
        }
        if e.flags & DELETE != 0 {
            let mut animations = std::mem::take(&mut world.model_animations);
            animations.delete_model(e.model, world);
            world.model_animations = animations;
            matrices = false;
        }
        self.delete_datum(k);
        if matrices {
            geometry_matrices(world, e.model)?;
        }
        Ok(())
    }
}

impl EngineWorld {
    pub fn update_model_effects(
        &mut self,
        time: i32,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        if !self.model_effects.any() {
            return Ok(());
        }
        let mut effects = std::mem::take(&mut self.model_effects);
        let mut actions = self.map_actions.take();
        let r = effects.update(self, actions.as_mut(), rng, time);
        self.map_actions = actions;
        self.model_effects = effects;
        r
    }
}
