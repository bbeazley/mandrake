use mandrake_common::FourCC;
use mandrake_rec::Recording;
use mandrake_rec::header::MonsterInitializer;
use mandrake_tag::{MeshTag, TagArchive};

use crate::engine_rng::{EngineMonster, GameParameters, MapLoadDraws, TagSource, Unmodelled};
use crate::net_game::NetRole;
use crate::players::GameSetup;
use crate::{CombatDefinitions, MovementLoad, Simulation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupRole {
    Headless,
    Game { live: bool },
}

#[derive(Clone, Copy)]
pub struct MapSource<'a> {
    pub mesh_data: &'a [u8],
    pub mesh: &'a MeshTag,
    pub tags: &'a dyn TagSource,
    pub params: GameParameters,
}

#[derive(Debug, Default)]
pub struct MapPopulation {
    pub spawned: Vec<u32>,
    pub movement: MovementLoad,
}

#[derive(Debug)]
pub struct MapSetup {
    pub load: Result<MapLoadDraws, Unmodelled>,
    pub monsters: Option<Vec<EngineMonster>>,
    pub population: Option<MapPopulation>,
}

impl Simulation {
    pub fn load_map(
        &mut self,
        map: &MapSource<'_>,
        role: SetupRole,
    ) -> Result<MapLoadDraws, Unmodelled> {
        if let SetupRole::Game { .. } = role {
            self.command_lead = crate::LOCAL_COMMAND_LEAD;
            self.load_mesh(map.mesh);
        }
        self.load_engine_map(map.mesh_data, map.tags, map.params)
    }

    pub fn populate_map(
        &mut self,
        map: &MapSource<'_>,
        role: SetupRole,
        monsters: Option<&[EngineMonster]>,
    ) -> MapPopulation {
        let spawned = match role {
            SetupRole::Headless => {
                self.load_mesh(map.mesh);
                self.spawn_initial_units_from_mesh(map.mesh)
            }
            SetupRole::Game { .. } => match monsters {
                Some(m) => {
                    let spawned = self.spawn_units_for_monsters(map.mesh, m);
                    self.apply_engine_unit_objects(m);
                    spawned
                }
                None => self.spawn_initial_units_from_mesh(map.mesh),
            },
        };
        let movement = self.attach_engine_movement(map.mesh, map.tags);
        if let SetupRole::Game { live: true } = role {
            self.halt_on_stop = true;
        }
        MapPopulation { spawned, movement }
    }

    pub fn set_up_map(&mut self, map: &MapSource<'_>, role: SetupRole) -> MapSetup {
        let load = self.load_map(map, role);
        if load.is_err() {
            return MapSetup {
                load,
                monsters: None,
                population: None,
            };
        }
        let monsters = Some(self.engine.monsters.clone());
        let population = Some(self.populate_map(map, role, monsters.as_deref()));
        MapSetup {
            load,
            monsters,
            population,
        }
    }

    pub fn load_combat_definitions(&mut self, archives: &[&TagArchive]) {
        let archetypes: Vec<FourCC> = self.units.values().map(|u| u.archetype).collect();
        let defs = CombatDefinitions::load(archives, archetypes);
        self.set_combat_definitions(defs);
    }

    pub fn replay_film(
        recording: &Recording,
        setup: GameSetup,
        map: &MapSource<'_>,
    ) -> (Self, MapSetup) {
        let mut sim = Simulation::from_recording(recording);
        sim.game_setup = Some(setup);
        let out = sim.set_up_map(map, SetupRole::Headless);
        (sim, out)
    }

    pub fn start_network_game(
        film: &[u8],
        role: NetRole,
        carried: &[MonsterInitializer],
        map: &MapSource<'_>,
        archives: &[&TagArchive],
    ) -> Result<(Self, MapSetup), String> {
        let mut sim = Simulation::network_game(film, role)
            .map_err(|e| format!("the network game's start: {e}"))?;
        sim.carried_initializers = carried.to_vec();
        let out = sim.set_up_map(map, SetupRole::Game { live: false });
        if let Err(e) = &out.load {
            return Err(format!("the engine map load stopped: {e}"));
        }
        sim.load_combat_definitions(archives);
        sim.halt_on_stop = true;
        Ok((sim, out))
    }
}
