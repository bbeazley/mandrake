use mandrake_common::FourCC;
use mandrake_common::game_options;

use crate::draw_site;
use crate::engine_rng::{EngineObject, EngineWorld, Unmodelled};
use crate::map_actions::{MapActionContext, MapActions};

const TYPE_TAG_NAME: i16 = 7;
const MAX_OBJECTS: usize = 0x200;
pub const RANDOM_SITE: u32 = draw_site::OBJECT_CONTROL_RANDOM;
pub const OBJECT_CONTROL_DONE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holder {
    Scenery(usize),
    Projectile(usize),
    Dormant(usize),
}

impl EngineWorld {
    fn kind4_objects(&self) -> Result<Vec<(i32, Holder, FourCC)>, Unmodelled> {
        let mut v = Vec::new();
        for (k, s) in self.scenery.iter().enumerate() {
            if s.deleted || s.object.kind != 4 || s.object.identifier == 0 {
                continue;
            }
            let Some(p) = s.projectile else {
                return Err(Unmodelled::new(
                    draw_site::OBJECT_CONTROL_NO_PROJECTILE,
                    format!(
                        "obco: scenery object {} of kind 4 has no projectile",
                        s.object.index
                    ),
                ));
            };
            v.push((s.object.index, Holder::Scenery(k), p));
        }
        for (k, p) in self.projectiles.iter().enumerate() {
            if p.object.kind == 4 && p.object.identifier != 0 {
                v.push((p.object.index, Holder::Projectile(k), p.definition));
            }
        }
        for (k, d) in self.dormant_objects.iter().enumerate() {
            if d.object.kind == 4 && d.object.identifier != 0 {
                v.push((d.object.index, Holder::Dormant(k), d.definition));
            }
        }
        v.sort_by_key(|o| o.0);
        Ok(v)
    }

    fn holder_object(&self, h: Holder) -> &EngineObject {
        match h {
            Holder::Scenery(k) => &self.scenery[k].object,
            Holder::Projectile(k) => &self.projectiles[k].object,
            Holder::Dormant(k) => &self.dormant_objects[k].object,
        }
    }

    fn holder_object_mut(&mut self, h: Holder) -> &mut EngineObject {
        match h {
            Holder::Scenery(k) => &mut self.scenery[k].object,
            Holder::Projectile(k) => &mut self.projectiles[k].object,
            Holder::Dormant(k) => &mut self.dormant_objects[k].object,
        }
    }
}

impl MapActions {
    pub fn object_control_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let Some(kind) = self.long(i, b"type", TYPE_TAG_NAME) else {
            return Ok(0);
        };
        let kind = FourCC::new((kind as u32).to_be_bytes());
        let w = &mut *ctx.world;
        let found: Vec<(i32, Holder)> = w
            .kind4_objects()?
            .into_iter()
            .filter(|o| o.2 == kind)
            .map(|o| (o.0, o.1))
            .take(MAX_OBJECTS)
            .collect();
        let n = found.len();
        if n == 0 {
            return Ok(0);
        }
        let gameplay = w.formation_globals.gameplay_version;
        let mut hidden: Vec<bool> = found
            .iter()
            .map(|&(_, h)| w.holder_object(h).no_shape)
            .collect();
        let mut result = 0;
        if self.flag(i, b"real") {
            hidden.iter_mut().for_each(|h| *h = false);
            result = OBJECT_CONTROL_DONE;
        }
        if self.flag(i, b"rera") {
            let pick = ((i32::from(ctx.rng.draw(RANDOM_SITE)) * n as i32) >> 16) as usize;
            for (k, h) in hidden.iter_mut().enumerate() {
                if k != pick {
                    *h = true;
                    continue;
                }
                let keep = gameplay <= game_options::GAMEPLAY_1_7_1
                    && w.holder_object(found[k].1).anim_flags & 1 != 0;
                if !keep {
                    *h = false;
                }
            }
            result = OBJECT_CONTROL_DONE;
        }
        if gameplay > 0x69a && self.flag(i, b"hial") {
            hidden.iter_mut().for_each(|h| *h = true);
            result = OBJECT_CONTROL_DONE;
        }
        let carried: Vec<i16> = (0..w.monsters.len())
            .map(|m| w.combat(m).carried)
            .filter(|&c| c != -1)
            .collect();
        for (&(index, h), &hide) in found.iter().zip(&hidden) {
            let o = w.holder_object(h);
            if !o.no_shape || hide {
                continue;
            }
            if carried.contains(&(index as i16)) {
                return Err(Unmodelled::new(
                    draw_site::OBJECT_CONTROL_CARRIED_OBJECT,
                    format!("obco: shows object {index}, which a monster carries"),
                ));
            }
            if matches!(h, Holder::Scenery(_)) && o.flags & 0x400 != 0 && o.sequence != 0xff {
                return Err(Unmodelled::new(
                    draw_site::OBJECT_CONTROL_CARRIED_OBJECT,
                    format!("obco: shows solid scenery object {index} (collision cylinders)"),
                ));
            }
        }
        for (&(index, h), &hide) in found.iter().zip(&hidden) {
            let o = w.holder_object_mut(h);
            if o.no_shape == hide {
                continue;
            }
            o.no_shape = hide;
            if hide && matches!(h, Holder::Scenery(_)) {
                w.scenery_taken.push(index);
            }
        }
        Ok(result)
    }
}
