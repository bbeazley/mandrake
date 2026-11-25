use crate::draw_site;
use crate::map_actions::{find, index_of};
use crate::objects::DatumPool;
use crate::{Stop, TflSim};

pub const MAX_MODEL_EFFECTS: usize = 0x10;

pub const FADE_IN: u16 = 1;
pub const FADE_OUT: u16 = 2;
pub const DELETE: u16 = 4;
pub const SCALE: u16 = 8;
pub const PROJECTILE: u16 = 0x10;
pub const REVERSE: u16 = 0x20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelEffect {
    pub identifier: i16,
    pub model: i32,
    pub action: i32,
    pub ticks: i16,
    pub flags: u16,
    pub duration: i16,
    pub turn: (i16, i16),
    pub permutation: i16,
    pub material: i16,
    pub material_start: u16,
    pub material_end: u16,
    pub scale: (i16, i16),
}

#[derive(Debug, Clone)]
pub struct ModelEffects {
    pub slots: Vec<Option<ModelEffect>>,
    pub pool: DatumPool,
}

impl Default for ModelEffects {
    fn default() -> Self {
        Self {
            slots: vec![None; MAX_MODEL_EFFECTS],
            pool: DatumPool::new(MAX_MODEL_EFFECTS),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Request {
    pub(crate) flags: u16,
    pub(crate) duration: i16,
    pub(crate) turn: (i16, i16),
    pub(crate) new_permutation: i32,
    pub(crate) permutation: i32,
    pub(crate) material: i32,
    pub(crate) material_end: u16,
    pub(crate) scale: (i16, i16),
}

pub(crate) fn fixed(v: i32, k: i32) -> i16 {
    let x = v.wrapping_mul(k);
    (x.wrapping_add((x >> 31) & 0xffff) >> 16) as i16
}

impl TflSim {
    pub(crate) fn model_effect_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let get = |name: &[u8; 4], kind: i16| find(a, i, 0, name, kind).filter(|p| p.count > 0);
        let Some(model) = get(b"mode", 0xf).map(|p| p.i16_at(0)) else {
            return Ok(0);
        };
        let flag = |name: &[u8; 4]| find(a, i, 2, name, 0).is_some();
        let mut r = Request {
            flags: 0,
            duration: get(b"dura", 8).map_or(-1, |p| fixed(p.i32_at(0), 0x1e)),
            turn: (
                get(b"dyaw", 4).map_or(0, |p| p.i16_at(0)),
                get(b"dpit", 4).map_or(0, |p| p.i16_at(0)),
            ),
            new_permutation: get(b"newp", 5).map_or(-1, |p| p.i32_at(0)),
            permutation: get(b"perm", 5).map_or(-1, |p| p.i32_at(0)),
            material: -1,
            material_end: 0,
            scale: (0, 0),
        };
        for (name, bit) in [
            (b"fain", FADE_IN),
            (b"faou", FADE_OUT),
            (b"dele", DELETE),
            (b"peon", REVERSE),
        ] {
            if flag(name) {
                r.flags |= bit;
            }
        }
        if let (Some(m), Some(f)) = (get(b"mate", 5), get(b"mafi", 8)) {
            r.material = m.i32_at(0);
            r.material_end = !(f.i32_at(0).clamp(0, 0xffff) as u16);
        }
        if let Some(s) = get(b"insc", 8) {
            let v = fixed(s.i32_at(0), 0x400);
            r.flags |= SCALE;
            r.scale = (v, v);
        }
        if get(b"proj", 7).is_some() && find(a, i, 0, b"orig", 0xc).is_some() {
            r.flags |= PROJECTILE;
        }
        let action = i32::from(self.map_actions.actions[i].identifier);
        self.new_model_effect(i32::from(model), action, &r)?;
        Ok(0)
    }

    pub(crate) fn model_index(&self, id: i32) -> Option<usize> {
        self.model_instances
            .iter()
            .position(|m| m.identifier != 0 && i32::from(m.identifier) == id)
    }

    pub(crate) fn new_model_effect(
        &mut self,
        model: i32,
        action: i32,
        r: &Request,
    ) -> Result<(), Stop> {
        let t = self.tick;
        let index = if action == -1 {
            None
        } else {
            index_of(&self.map_actions.actions, action as i16)
        };
        let m = self.model_index(model);
        if r.flags & REVERSE != 0
            && let Some(e) = self
                .model_effects
                .slots
                .iter()
                .take(self.model_effects.pool.high)
                .rposition(|e| e.is_some_and(|e| e.model == model))
        {
            if r.new_permutation == -1 {
                return Ok(());
            }
            let Some(m) = m else {
                return Err(Stop::new(
                    draw_site::MODEL_EFFECT_NO_MODEL,
                    format!("tick {t}: moef: no model {model}"),
                ));
            };
            let e = self.model_effects.slots[e].as_mut().unwrap();
            let mi = &mut self.model_instances[m];
            mi.permutation = e.permutation;
            e.permutation = r.new_permutation as i16;
            mi.next_permutation = r.new_permutation as i16;
            e.ticks = e.duration.wrapping_sub(e.ticks);
            return Ok(());
        }
        let slots = &self.model_effects.slots;
        let Some((k, salt)) = self
            .model_effects
            .pool
            .allocate(|k| slots.get(k).is_some_and(|e| e.is_some()))
        else {
            if let Some(j) = index {
                self.set_error(j)?;
            }
            return Ok(());
        };
        let Some(m) = m.filter(|_| r.duration >= 0) else {
            if let Some(j) = index {
                self.set_error(j)?;
            }
            let slots = &self.model_effects.slots;
            self.model_effects
                .pool
                .free(k, |j| slots.get(j).is_some_and(|e| e.is_some()));
            return Ok(());
        };
        if r.flags & SCALE != 0 {
            return Err(Stop::new(
                draw_site::MODEL_EFFECT_SCALE,
                format!("tick {t}: moef on model {model}: a change of scale (insc)"),
            ));
        }
        if r.flags & PROJECTILE != 0 {
            return Err(Stop::new(
                draw_site::MODEL_EFFECT_PROJECTILE,
                format!("tick {t}: moef on model {model}: a projectile (new_projectile)"),
            ));
        }
        let mi = &mut self.model_instances[m];
        let permutation = if r.permutation == -1 {
            mi.permutation
        } else {
            r.permutation as i16
        };
        mi.permutation = permutation;
        let end = if r.new_permutation == -1 {
            permutation
        } else {
            r.new_permutation as i16
        };
        mi.next_permutation = end;
        self.model_effects.slots[k] = Some(ModelEffect {
            identifier: salt,
            model,
            action,
            ticks: 0,
            flags: r.flags,
            duration: r.duration,
            turn: r.turn,
            permutation: end,
            material: r.material as i16,
            material_start: 0,
            material_end: r.material_end,
            scale: if r.flags & SCALE != 0 {
                r.scale
            } else {
                (0x400, 0x400)
            },
        });
        Ok(())
    }

    pub(crate) fn update_model_effects(&mut self) -> Result<(), Stop> {
        let mut k = 0;
        while k < self.model_effects.pool.high {
            if let Some(e) = self.model_effects.slots[k]
                && let Some(m) = self.model_index(e.model)
            {
                let ticks = e.ticks.wrapping_add(1);
                self.model_effects.slots[k].as_mut().unwrap().ticks = ticks;
                if e.duration == -1 || ticks < e.duration {
                    let mi = &mut self.model_instances[m];
                    if e.turn != (0, 0) {
                        mi.yaw = mi.yaw.wrapping_add(e.turn.0 as u16);
                        mi.pitch = mi.pitch.wrapping_add(e.turn.1 as u16);
                        let (a, b) = crate::models::angle_vectors(mi.yaw, mi.pitch);
                        mi.a = a;
                        mi.b = b;
                    }
                    if e.duration != -1 {
                        if e.duration == 0 {
                            return Err(Stop::new(0, "model effect with duration 0"));
                        }
                        let f = ((i32::from(ticks) * 0xffff) / i32::from(e.duration)) as u16;
                        mi.transition = f;
                        if e.flags & FADE_IN != 0 {
                            mi.fade = 0xffff_u16.wrapping_sub(f);
                        }
                        if e.flags & FADE_OUT != 0 {
                            mi.fade = f;
                        }
                    }
                } else {
                    if e.action != -1
                        && let Some(j) = index_of(&self.map_actions.actions, e.action as i16)
                    {
                        self.succeed(j)?;
                    }
                    let mi = &mut self.model_instances[m];
                    if e.flags & FADE_IN != 0 {
                        mi.fade = 0;
                    }
                    if e.flags & FADE_OUT != 0 {
                        mi.fade = 0xffff;
                    }
                    mi.transition = 0;
                    mi.permutation = e.permutation;
                    mi.next_permutation = e.permutation;
                    if e.flags & DELETE != 0 {
                        return Err(Stop::new(
                            draw_site::MODEL_EFFECT_DELETE,
                            format!(
                                "tick {}: a model effect deletes model {m} (delete_model)",
                                self.tick
                            ),
                        ));
                    }
                    self.model_effects.slots[k] = None;
                    let slots = &self.model_effects.slots;
                    self.model_effects
                        .pool
                        .free(k, |j| slots.get(j).is_some_and(|e| e.is_some()));
                }
            }
            k += 1;
        }
        Ok(())
    }
}
