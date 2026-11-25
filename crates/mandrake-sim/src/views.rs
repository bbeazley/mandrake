use mandrake_common::FourCC;

use crate::Simulation;
use crate::engine_rng::{MovementHeader, Unmodelled};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectShape {
    pub collection: FourCC,
    pub sequence: u8,
    pub frame: i8,
    pub view: u8,
    pub facing: u16,
    pub permutation: i16,
    pub mirrored: bool,
    pub z: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ObjectKind {
    Projectile,
    Dormant,
    Copy,
}

impl Simulation {
    pub fn unit_shape(&self, id: u32) -> Option<ObjectShape> {
        let unit = self.units.get(&id)?;
        let facing = unit.engine.as_ref().map_or(0, |m| m.facing);
        let engine_monster = self
            .engine
            .monsters
            .iter()
            .find(|m| m.unit_key() == Some(id));
        let mirrored = engine_monster.is_some_and(|m| m.mirrored);
        let z = engine_monster.map_or(0, |m| m.object.z);
        if self.engine_stop.is_some() {
            return None;
        }
        let w = &self.engine;
        let m = w
            .monsters
            .iter()
            .find(|m| m.unit_key() == Some(id) && m.unmodelled.is_none())?;
        let o = &m.object;
        (o.sequence != 0xff).then_some(ObjectShape {
            collection: o.collection,
            sequence: o.sequence,
            frame: o.frame,
            view: o.view,
            facing,
            permutation: o.permutation,
            mirrored,
            z,
        })
    }

    pub fn projectile_objects(&self) -> Vec<crate::ProjectileView> {
        self.projectile_views()
            .into_iter()
            .filter(|v| v.object >= 0)
            .collect()
    }

    #[allow(clippy::type_complexity)]
    pub fn movement_command_destinations(
        &self,
        unit_ids: &[u32],
        points: &[(i32, i32)],
        header: MovementHeader,
    ) -> Result<(Vec<Vec<(i32, i32)>>, Vec<u16>), Unmodelled> {
        let w = &self.engine;
        let monsters: Vec<usize> = unit_ids
            .iter()
            .filter_map(|&id| w.monster_index_by_identifier(id as u16))
            .collect();
        w.movement_command_destinations(&monsters, points, header)
    }

    pub fn unit_is_moving(&self, id: u32) -> bool {
        self.units
            .get(&id)
            .and_then(|u| u.engine.as_ref())
            .is_some_and(|m| m.is_moving(self.engine.monster_flags_by_identifier(id as u16)))
    }

    pub fn terrain_type_at(&self, x: i32, y: i32) -> Option<usize> {
        self.engine_terrain
            .as_ref()
            .or(Some(&self.engine.cells))
            .filter(|c| c.width > 0)
            .map(|c| c.terrain_type(x, y))
    }

    pub fn monster_impassable_mask(&self, id: u32) -> Option<u16> {
        let w = &self.engine;
        w.monsters
            .iter()
            .find(|m| m.unit_key() == Some(id))
            .map(|m| m.impassable)
    }
}
