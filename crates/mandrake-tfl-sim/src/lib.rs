mod actions;
pub mod attack;
mod attacks;
mod carried_objects;
pub mod combat;
pub mod commands;
pub mod damage;
pub mod data;
pub mod draw_site;
pub mod flags;
#[cfg(test)]
mod flags_tests;
mod geometry_filter;
pub mod group_actions;
pub mod groups;
pub mod invisible;
mod knock_down;
pub mod knot;
pub mod legion;
pub mod lightning;
mod lightning_action;
pub mod local_groups;
pub mod map_actions;
pub mod mesh_effects;
pub mod model_effects;
mod models;
pub mod monsters;
mod move_marker_action;
mod movement;
pub mod net_commands;
pub mod net_game;
#[cfg(test)]
mod net_game_tests;
pub mod objects;
pub mod obstacles;
pub mod particles;
pub mod paths;
pub mod player_colours;
pub mod procs;
pub mod projectiles;
pub mod rng;
mod setup;
mod shaman;
pub mod sight;
pub mod sniffing;
pub mod sounds;
pub mod stains;
pub mod statistics;
pub mod suicide;
mod targets;
mod tc2_trace;
pub mod terrain;
#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;
pub mod tracking;
pub mod trig;

use std::collections::HashMap;
use std::path::Path;

use mandrake_common::FourCC;

pub use actions::{Meander, Order};
pub use data::TflData;
pub use local_groups::{LocalGroupObject, LocalGroupSource, LocalGroupStart, LocalGroupTarget};
pub use models::ModelInstance;
pub use monsters::TflMonster;
pub use objects::TflObject;
pub use player_colours::{GameColours, game_colours, set_game_colours};
pub use projectiles::{ColourSource, TflProjectile};
pub use rng::{Draw, TflRng};
pub use setup::{MonsterInitializer, UnitCopy, film_monster_initializers};
pub use sounds::{SoundEvent, SoundKind, SoundSource};
pub use stains::{Blood, Footprint, Mark, Stain};

pub use mandrake_common::Stop;

pub(crate) mod be {
    #[inline]
    pub fn u16_at(b: &[u8], o: usize) -> Option<u16> {
        b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
    }

    #[inline]
    pub fn i16_at(b: &[u8], o: usize) -> Option<i16> {
        u16_at(b, o).map(|v| v as i16)
    }

    #[inline]
    pub fn u32_at(b: &[u8], o: usize) -> Option<u32> {
        b.get(o..o + 4)
            .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    #[inline]
    pub fn i32_at(b: &[u8], o: usize) -> Option<i32> {
        u32_at(b, o).map(|v| v as i32)
    }
}

pub fn film_difficulty(film: &[u8]) -> Result<i16, Stop> {
    be::u32_at(film, 0x30)
        .and_then(|d| be::i16_at(film, d as usize + 0x34))
        .ok_or_else(|| Stop::new(draw_site::FILM_HEADER_SHORT, "film header too short"))
}

pub fn film_game_options(film: &[u8]) -> Result<u32, Stop> {
    be::u32_at(film, 0x30)
        .and_then(|d| be::u32_at(film, d as usize + 0x28))
        .ok_or_else(|| Stop::new(draw_site::FILM_HEADER_SHORT, "film header too short"))
}

pub struct TflSim {
    pub data: TflData,
    pub rng: TflRng,
    pub difficulty: i16,
    pub tick: u32,
    pub objects: Vec<TflObject>,
    pub monsters: Vec<TflMonster>,
    pub(crate) mons_counters: HashMap<FourCC, i16>,
    pub(crate) unit_copies: Vec<setup::UnitCopy>,
    pub map_actions: map_actions::MapActions,
    pub orders: Vec<Order>,
    pub(crate) record_orders: bool,
    pub(crate) command_cursor: usize,
    pub(crate) cached_local_team: Option<(i16, Option<i16>, bool)>,
    pub meanders: HashMap<usize, Meander>,
    pub(crate) spatial_groups: Option<Vec<groups::Group>>,
    pub mesh_effects: mesh_effects::MeshEffects,
    pub(crate) spatial_supergroups: Option<Vec<groups::Supergroup>>,
    pub legions: HashMap<usize, legion::LegionState>,
    pub suicides: HashMap<usize, suicide::SuicideState>,
    pub game_mode: i16,
    pub cells: Vec<mandrake_tag::tfl::TflCell>,
    pub cell_heads: Vec<i16>,
    pub game_options: u32,
    pub obstacles: Vec<obstacles::Obstacle>,
    pub paths: paths::PathPool,
    pub winning_team: i16,
    pub media: terrain::MediaCoverage,
    pub object_pool: objects::DatumPool,
    pub monster_pool: objects::DatumPool,
    pub projectiles: projectiles::Projectiles,
    pub(crate) monster_initializers: Vec<setup::MonsterInitializer>,
    pub attack_ai: attack::AttackAi,
    pub sight: sight::SightMaps,
    pub explosions: damage::Explosions,
    pub lightning: lightning::Lightnings,
    pub(crate) model_instances: Vec<models::ModelInstance>,
    pub(crate) model_pool: objects::DatumPool,
    pub model_effects: model_effects::ModelEffects,
    pub(crate) marker_motion: Vec<[i16; 4]>,
    pub particles: Option<particles::ParticleSystem>,
    pub(crate) sound_events: Vec<sounds::SoundEvent>,
    pub(crate) sound_sources: Vec<sounds::SoundSource>,
    pub(crate) stains: Vec<stains::Stain>,
    pub(crate) marks: Vec<stains::Mark>,
    pub(crate) colour_sources: Vec<projectiles::ColourSource>,
    pub(crate) mark_count: u32,
    pub(crate) local_group_starts: Vec<local_groups::LocalGroupStart>,
    pub(crate) interest_events: Vec<tracking::InterestEvent>,
    pub(crate) local_projectile_group_orders: Vec<procs::LocalProjectileGroupOrder>,
    pub sniffer: Option<procs::Sniffer>,
    pub overhead_map_state: Option<i16>,
    pub camera_control_flags: u16,
    pub(crate) camera_movements: Vec<procs::CameraMoveEvent>,
    pub statistics: statistics::Statistics,
    pub flags: flags::FlagData,
    pub(crate) player_table: Option<net_commands::PlayerTable>,
    pub(crate) interface_commands: Vec<net_commands::InterfaceCommand>,
}

impl TflSim {
    pub fn new(game_dir: &Path, film: &[u8]) -> Result<Self, Stop> {
        Self::set_up(game_dir, film).map_err(|b| b.1)
    }

    pub fn new_recording(game_dir: &Path, film: &[u8]) -> Result<Self, Stop> {
        Self::set_up_recording(game_dir, film).map_err(|b| b.1)
    }

    pub fn set_record_orders(&mut self, record: bool) {
        self.record_orders = record;
    }

    #[inline]
    pub(crate) fn record_order(&mut self, make: impl FnOnce() -> Order) {
        if self.record_orders {
            self.orders.push(make());
        }
    }

    #[inline]
    pub(crate) fn set_shape(
        &mut self,
        object: usize,
        seq: i16,
        speed: i32,
        flags: u8,
    ) -> Result<(), Stop> {
        let collection = self.objects[object].collection;
        let headers = self.data.sequences(collection)?;
        crate::objects::set_object_shape(
            &mut self.objects[object],
            headers,
            seq,
            speed,
            flags,
            &mut self.rng,
        )
    }

    pub fn marker_motion(&self) -> &[[i16; 4]] {
        &self.marker_motion
    }

    pub fn sight(&self) -> &sight::SightMaps {
        &self.sight
    }

    pub fn step(&mut self) -> Result<(), Stop> {
        self.execute_monster_commands()?;
        self.update_lightning()?;
        self.update_projectiles()?;
        self.update_monsters()?;
        self.update_mesh_effects()?;
        self.update_map_actions()?;
        self.update_explosions()?;
        self.update_particle_clock();
        self.update_player_statistics()?;
        self.update_model_effects()?;
        self.tick += 1;
        self.update_game_time();
        Ok(())
    }

    fn place_marker_objects(&mut self) -> Result<(), Stop> {
        let mut monster_of = vec![None; self.objects.len()];
        for (k, m) in self.monsters.iter().enumerate() {
            if m.identifier != 0 {
                monster_of[m.object] = Some(k);
            }
        }
        for (o, owner) in monster_of.into_iter().enumerate() {
            if owner.is_none()
                && (self.objects[o].scenery.is_none() || self.objects[o].identifier == 0)
            {
                continue;
            }
            let (x, y) = (self.objects[o].x, self.objects[o].y);
            let ground = self.mesh_height(x, y)? as i16;
            let (physics, mask) = match owner {
                Some(k) => {
                    let mons = self.data.def(b"mons", self.monsters[k].definition)?;
                    (mons.tag(0x90), mons.closed_terrain())
                }
                None => (self.objects[o].physics, 0),
            };
            let obj = &mut self.objects[o];
            let z = i32::from(ground) + i32::from(obj.above_ground);
            obj.z = z;
            obj.above_ground = (z as i16).wrapping_sub(ground);
            obj.physics = physics;
            if obj.kind != crate::flags::OBJECT_KIND_FLAG {
                obj.terrain_mask = mask;
            }
            let cell = self.cell_at(x >> 10, y >> 10);
            let mut bits = 0;
            if z < 0 && self.cells[cell].flags & 0x18 != 0 && self.media_at(x, y)? {
                bits |= 0xc0;
            }
            self.objects[o].flags |= bits;
            self.objects[o].cell_flags = self.closed_sides(x, y, mask)?;
        }
        Ok(())
    }

    fn update_monsters(&mut self) -> Result<(), Stop> {
        self.update_monsters_first_loop()?;
        for i in 0..self.monsters.len() {
            let m = &self.monsters[i];
            if m.identifier == 0 || m.flags & 0x4000_0000 != 0 {
                continue;
            }
            let object = m.object;
            self.animate_object(object)?;
            let action = self.monsters[i].action;
            if !(8..=15).contains(&action) {
                if self.monsters[i].flags & 0x20 != 0 {
                    self.plan_movement(i)?;
                }
                let f = self.monsters[i].flags;
                if f & 0x40 != 0 {
                    self.set_monster_action(i, 1, 0)?;
                    if f & 0x80_0000 != 0 {
                        self.monsters[i].flags |= 0x80_0000;
                        let k = self.choose_attack(i, 3)?;
                        if k != -1 {
                            self.handle_monster_attack(i, k, 0)?;
                            self.monsters[i].flags |= 0x100_0000;
                        }
                        self.monsters[i].flags &= 0xff7f_ffff;
                    }
                    let m = &mut self.monsters[i];
                    m.flags &= 0xffff_efbf;
                    m.waypoint_count = 0;
                }
            }
            let m = &mut self.monsters[i];
            if matches!(m.action, 1..=4 | 6 | 9 | 11..=15) && m.field_4a >= 0 {
                m.field_4a -= 1;
            }
            match self.monsters[i].action {
                1 => self.monster_idle(i)?,
                2 => self.monster_celebrating(i)?,
                11 => self.monster_healing(i)?,
                12 => self.monster_paralyzed(i)?,
                3 | 4 | 6 | 7 => self.monster_moving(i)?,
                8 => self.monster_attacking(i)?,
                10 => self.monster_picking_up(i)?,
                13 => self.monster_stunned(i)?,
                14 => self.monster_knocked_down(i)?,
                15 => {
                    let o = self.monsters[i].object;
                    if self.objects[o].flags & 0x101 != 0 {
                        self.stop_moving(i)?;
                    }
                }
                9 => {
                    let m = &mut self.monsters[i];
                    m.substate = m.substate.wrapping_sub(1);
                    if (m.substate as i8) < 0 {
                        self.stop_moving(i)?;
                    }
                }
                a => {
                    return Err(Stop::new(
                        if matches!(a, 3 | 4 | 6 | 7) {
                            draw_site::UPDATE_MONSTER_ACTION
                        } else {
                            draw_site::UPDATE_MONSTER
                        },
                        format!(
                            "tick {}: monster {i}: action {a} in update_monsters",
                            self.tick
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}
