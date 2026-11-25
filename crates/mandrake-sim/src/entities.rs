use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};

use mandrake_common::FourCC;
use mandrake_math::Fixed16;
use mandrake_math::Vec3Fixed;
use mandrake_math::trig::Angle512 as Angle;

use mandrake_tag::MeshHeightMode;

use crate::mesh_grid::MeshGrid;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum UnitState {
    #[default]
    Idle,
    Moving,
    Attacking,
    Dead,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnitIdHasher(u64);

impl Hasher for UnitIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u32(u32::from(b));
        }
    }
    fn write_u32(&mut self, id: u32) {
        self.0 = (self.0 ^ u64::from(id)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TagIdHasher(u64);

impl TagIdHasher {
    fn word(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

impl Hasher for TagIdHasher {
    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 32)
    }
    fn write(&mut self, bytes: &[u8]) {
        for c in bytes.chunks(8) {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            self.word(u64::from_le_bytes(w));
        }
    }
    fn write_u8(&mut self, b: u8) {
        self.word(u64::from(b));
    }
    fn write_u32(&mut self, n: u32) {
        self.word(u64::from(n));
    }
    fn write_usize(&mut self, n: usize) {
        self.word(n as u64);
    }
}

pub type TagIdMap<V> = HashMap<mandrake_common::FourCC, V, BuildHasherDefault<TagIdHasher>>;

pub type UnitIdMap<V> = HashMap<u32, V, BuildHasherDefault<UnitIdHasher>>;

pub type UnitIdSet = HashSet<u32, BuildHasherDefault<UnitIdHasher>>;

pub type UnitMap = UnitIdMap<UnitEntity>;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnitEntity {
    pub id: u32,
    pub archetype: FourCC,
    pub team: u8,
    pub position: Vec3Fixed,
    pub velocity: Vec3Fixed,
    pub facing: Angle,
    pub health: Fixed16,
    pub max_health: Fixed16,
    pub state: UnitState,
    pub waypoints: VecDeque<Vec3Fixed>,
    pub selected: bool,
    pub movement_speed: Fixed16,
    pub target_unit_id: Option<u32>,
    #[serde(default)]
    pub engine: Option<crate::engine_move::EngineMover>,
    #[serde(default)]
    pub engine_object: Option<EngineUnitObject>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EngineUnitObject {
    pub scale: u16,
    pub vitality: i16,
    #[serde(default)]
    pub permutation: i16,
}

impl UnitEntity {
    pub const DEFAULT_SPEED: Fixed16 = Fixed16::from_raw(5243);

    pub fn new(
        id: u32,
        archetype: FourCC,
        team: u8,
        position: Vec3Fixed,
        health: Fixed16,
        movement_speed: Fixed16,
    ) -> Self {
        Self {
            id,
            archetype,
            team,
            position,
            velocity: Vec3Fixed::ZERO,
            facing: Angle::ZERO,
            health,
            max_health: health,
            state: UnitState::Idle,
            waypoints: VecDeque::new(),
            selected: false,
            movement_speed,
            target_unit_id: None,
            engine: None,
            engine_object: None,
        }
    }

    pub fn spawn_archetype(id: u32, archetype: FourCC, team: u8, position: Vec3Fixed) -> Self {
        let code = archetype.as_bytes();
        let (health, speed) = match code {
            b"dwar" => (Fixed16::from_i32(90), Fixed16::from_raw(4587)),
            b"soul" | b"hawk" => (Fixed16::from_i32(80), Fixed16::from_raw(6554)),
            b"wam1" | b"warr" | b"bers" => (Fixed16::from_i32(120), Fixed16::from_raw(5898)),
            b"jom1" | b"jour" => (Fixed16::from_i32(110), Fixed16::from_raw(5243)),
            _ => (Fixed16::from_i32(100), Self::DEFAULT_SPEED),
        };

        Self::new(id, archetype, team, position, health, speed)
    }

    #[inline]
    pub fn is_alive(&self) -> bool {
        self.health > Fixed16::ZERO && self.state != UnitState::Dead
    }

    pub fn take_damage(&mut self, amount: Fixed16) {
        if !self.is_alive() {
            return;
        }

        self.health -= amount;
        if self.health <= Fixed16::ZERO {
            self.health = Fixed16::ZERO;
            self.state = UnitState::Dead;
            self.velocity = Vec3Fixed::ZERO;
            self.waypoints.clear();
        }
    }

    pub fn ground_height(&self, grid: &MeshGrid) -> Fixed16 {
        let flag4 = self
            .engine
            .as_ref()
            .is_some_and(|m| m.definition.flags & 4 != 0);
        let mode = if flag4 {
            MeshHeightMode::Surface
        } else {
            MeshHeightMode::Physical
        };
        grid.mesh_height_at_world(self.position.x, self.position.z, mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_archetype_health() {
        let dwar = UnitEntity::spawn_archetype(1, FourCC::new(*b"dwar"), 0, Vec3Fixed::ZERO);
        assert_eq!(dwar.health, Fixed16::from_i32(90));
        let arch = UnitEntity::spawn_archetype(2, FourCC::new(*b"arch"), 0, Vec3Fixed::ZERO);
        assert_eq!(arch.health, Fixed16::from_i32(100));
    }

    #[test]
    fn test_unit_death() {
        let mut unit = UnitEntity::spawn_archetype(1, FourCC::new(*b"arch"), 0, Vec3Fixed::ZERO);
        assert!(unit.is_alive());

        unit.take_damage(Fixed16::from_i32(50));
        assert!(unit.is_alive());
        assert_eq!(unit.health, Fixed16::from_i32(50));

        unit.take_damage(Fixed16::from_i32(60));
        assert!(!unit.is_alive());
        assert_eq!(unit.state, UnitState::Dead);
    }
}
