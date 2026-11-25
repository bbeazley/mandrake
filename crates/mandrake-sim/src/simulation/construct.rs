use crate::*;

pub(crate) fn no_engine_map() -> engine_rng::Unmodelled {
    engine_rng::Unmodelled::new(draw_site::NO_ENGINE_MAP, "no engine map load")
}

impl Simulation {
    pub fn new(seed: u32) -> Self {
        Self {
            current_tick: 0,
            rng: BungieRng::new(seed),
            commands_by_tick: std::collections::BTreeMap::new(),
            total_dispatched_commands: 0,
            state_checksum: 0,
            mesh_grid: None,
            units: Default::default(),
            next_unit_id: 1,
            next_projectile_id: 1,
            combat: CombatState::default(),
            engine: engine_rng::EngineWorld::default(),
            engine_stop: Some(no_engine_map()),
            engine_sites: engine_rng::SiteCounts::default(),
            engine_terrain: None,
            engine_attack_launch: false,
            engine_hit: None,
            game_options_high: 0,
            carried_initializers: Vec::new(),
            carried_rankings: Vec::new(),
            game_type: None,
            game_setup: None,
            command_log: None,
            logging_command: false,
            collision_cache: Default::default(),
            sequence_headers_cache: Default::default(),
            replay: None,
            local_commands: Vec::new(),
            local_waypoints: Vec::new(),
            attack_click: attack_ai::AttackClick::default(),
            scripted_input: None,
            halt_on_stop: false,
            command_lead: 0,
            pre_game: false,
            time_limit: 0,
            planning_time: 0,
            late_commands: Vec::new(),
            recorded_commands: None,
            recording_ended: false,
            save_loaded_observer: None,
            sync_snapshot: (seed, 0),
            sync_timer: INITIAL_SYNC_TIMER,
            net: None,
            out_of_sync: Vec::new(),

            first_body: true,
        }
    }

    pub fn command_line_game(seed: u32) -> Self {
        let mut sim = Self::new(seed);
        sim.rng = players::set_random_seed(seed);
        sim.engine.game_options = players::COMMAND_LINE_GAME_OPTIONS;
        sim.game_setup = Some(players::GameSetup::command_line(
            players::COMMAND_LINE_GAME_OPTIONS,
            seed,
        ));
        sim
    }

    pub fn solo_game(game: players::SoloGame) -> Self {
        let mut sim = Self::new(game.seed);
        sim.rng = players::set_random_seed(game.seed);
        sim.engine.game_options = game.options;
        sim.game_options_high = (game.options >> 16) as u16;
        sim.game_type = Some(engine_rng::SOLO_GAME_TYPE);
        sim.game_setup = Some(players::GameSetup::solo(game));
        sim
    }

    pub fn live_game_of_film(
        film: &[u8],
    ) -> Result<(Self, LiveFilmInput), mandrake_common::M2Error> {
        let recording = Recording::from_bytes(film)?;
        let mut sim = Self::from_recording(&recording);
        let by_tick: Vec<(u32, Vec<CommandPacket>)> = std::mem::take(&mut sim.commands_by_tick)
            .into_iter()
            .collect();
        let options = recording.header.game_options_flags;
        sim.replay = None;
        sim.engine.game_options = options;
        sim.game_options_high = (options >> 16) as u16;
        if let Some(setup) = players::GameSetup::from_film(film) {
            sim.game_setup = Some(setup);
        }
        Ok((
            sim,
            LiveFilmInput {
                by_tick,
                next: 0,
                too_early: 0,
            },
        ))
    }

    pub fn give_film_commands(&mut self, input: &mut LiveFilmInput) -> usize {
        let tick = self.current_tick + self.command_lead;
        let mut n = 0;
        while let Some((t, commands)) = input.by_tick.get(input.next) {
            if *t > tick {
                break;
            }
            let own = |c: &&CommandPacket| {
                !matches!(
                    c.command_type,
                    mandrake_rec::CommandType::Sync | mandrake_rec::CommandType::SyncFormation
                )
            };
            for c in commands.iter().filter(own) {
                if *t < tick {
                    input.too_early += 1;
                }
                self.local_commands.push(QueuedCommand::Packet(c.clone()));
                n += 1;
            }
            input.next += 1;
        }
        n
    }

    pub fn from_film(film: &[u8]) -> Result<Self, mandrake_common::M2Error> {
        let recording = Recording::from_bytes(film)?;
        let mut sim = Self::from_recording(&recording);
        if let Some(mut setup) = players::GameSetup::from_film(film) {
            if let Some(r) = sim.replay {
                r.apply_to_setup(&mut setup);
            }
            sim.game_setup = Some(setup);
        }
        Ok(sim)
    }

    pub fn from_recording(recording: &Recording) -> Self {
        let mut commands_by_tick = std::collections::BTreeMap::new();
        for tc in &recording.tick_commands {
            commands_by_tick.insert(tc.tick, tc.commands.clone());
        }
        let replay =
            engine_rng::ReplayGame::new(recording.header.mode, recording.header.game_options_flags);
        let mut game_setup = players::GameSetup::from_header(&recording.header);
        replay.apply_to_setup(&mut game_setup);

        let mut engine = engine_rng::EngineWorld::default();
        engine.game_options = replay.apply_to_options(recording.header.game_options_flags);

        Self {
            current_tick: 0,
            rng: players::set_random_seed(recording.header.seed),
            commands_by_tick,
            total_dispatched_commands: 0,
            state_checksum: 0,
            mesh_grid: None,
            units: Default::default(),
            next_unit_id: 1,
            next_projectile_id: 1,
            combat: CombatState::default(),
            engine,
            engine_stop: Some(no_engine_map()),
            engine_sites: engine_rng::SiteCounts::default(),
            engine_terrain: None,
            engine_attack_launch: false,
            engine_hit: None,
            game_options_high: (recording.header.game_options_flags >> 16) as u16,
            carried_initializers: recording.header.monster_initializers.clone(),
            carried_rankings: Vec::new(),
            game_type: Some(recording.header.game_type),
            game_setup: Some(game_setup),
            command_log: None,
            logging_command: false,
            collision_cache: Default::default(),
            sequence_headers_cache: Default::default(),
            replay: Some(replay),
            local_commands: Vec::new(),
            local_waypoints: Vec::new(),
            attack_click: attack_ai::AttackClick::default(),
            scripted_input: None,
            halt_on_stop: false,
            command_lead: 0,
            pre_game: recording.header.time_limit as i32 > 0
                && recording.header.planning_time as i32 > 0,
            time_limit: recording.header.time_limit as i32,
            planning_time: recording.header.planning_time as i32,
            late_commands: Vec::new(),
            recorded_commands: None,
            recording_ended: false,
            save_loaded_observer: None,
            sync_snapshot: (recording.header.initial_seed, 0),
            sync_timer: INITIAL_SYNC_TIMER,
            net: None,
            out_of_sync: Vec::new(),

            first_body: true,
        }
    }

    pub fn load_engine_map(
        &mut self,
        mesh_data: &[u8],
        tags: &dyn engine_rng::TagSource,
        params: engine_rng::GameParameters,
    ) -> Result<engine_rng::MapLoadDraws, engine_rng::Unmodelled> {
        self.load_engine_map_with_options(
            mesh_data,
            tags,
            params,
            &engine_rng::MapActionOptions::default(),
        )
    }

    pub fn load_engine_map_with_options(
        &mut self,
        mesh_data: &[u8],
        tags: &dyn engine_rng::TagSource,
        params: engine_rng::GameParameters,
        options: &engine_rng::MapActionOptions,
    ) -> Result<engine_rng::MapLoadDraws, engine_rng::Unmodelled> {
        let mut options = options.clone();
        if options.game_options_high == 0 {
            options.game_options_high = self.game_options_high;
        }
        if options.game_type.is_none() {
            options.game_type = self.game_type;
        }
        if let (Some(r), Some(setup)) = (self.replay, self.game_setup.as_mut()) {
            r.apply_to_setup(setup);
        }
        if options.setup.is_none() {
            options.setup = self.game_setup.clone();
        }
        if options.replay.is_none() {
            options.replay = self.replay;
        }
        options.time_limit = self.time_limit;
        options.planning_time = self.planning_time;
        options
            .monster_initializers
            .clone_from(&self.carried_initializers);
        options
            .permanent_rankings
            .clone_from(&self.carried_rankings);
        options.option_flags = self.engine.game_options;
        let load =
            engine_rng::map_load_with_options(&mut self.rng, mesh_data, tags, params, &options)
                .inspect_err(|e| {
                    self.engine_stop = Some(engine_rng::Unmodelled::new(
                        draw_site::NO_ENGINE_MAP,
                        format!("the map load stopped: {e}"),
                    ));
                })?;

        for (site, n) in load.sites.counts() {
            self.engine_sites.add(*site, *n);
        }
        let mut world = load.world;
        world.game_options = options.option_flags;
        world.initialize_monster_placements();
        for mut p in std::mem::take(&mut world.projectile_records) {
            self.combat.next_projectile_id += 1;
            p.id = self.combat.next_projectile_id;
            self.combat.flight.flights.push(p);
        }
        for e in &world.group_events {
            if let combat::CombatEvent::StainMesh { stain, .. } = e {
                self.combat.flight.stains.push(*stain);
            }
        }
        self.engine = world;
        self.sequence_headers_cache.clear();
        self.engine_stop = load.unmodelled.first().cloned();
        Ok(load.draws)
    }

    pub fn set_engine_world(&mut self, world: engine_rng::EngineWorld) {
        self.engine = world;
        self.sequence_headers_cache.clear();
        if self
            .engine_stop
            .as_ref()
            .is_some_and(|s| s.site == draw_site::NO_ENGINE_MAP)
        {
            self.engine_stop = None;
        }
    }

    pub fn bridge_unmodelled_draws(&mut self, n: u64) {
        for _ in 0..n {
            self.rng.step();
        }
        self.engine_stop = None;
    }

    pub fn attach_engine_movement(
        &mut self,
        mesh: &MeshTag,
        tags: &dyn engine_rng::TagSource,
    ) -> MovementLoad {
        {
            let w = &mut self.engine;
            w.catalog.load_media_groups(tags, mesh.media_tag);
        }
        let postprocess = self.engine.mons_postprocess;
        let postprocessed = mons_postprocess::PostprocessedTags { tags, postprocess };
        let tags: &dyn engine_rng::TagSource = &postprocessed;
        let mut out = MovementLoad::default();
        {
            let w = &self.engine;
            self.combat.flight.ground = Some(
                projectiles::FlightTerrain::from_mesh(mesh)
                    .with_media(w.media.clone())
                    .with_models(models::ModelGrid::new(&w.cells, &w.models))
                    .with_cell_flags(&w.cells.flags)
                    .with_heights(&w.heights)
                    .with_ceiling(w.flight_ceiling()),
            );
            self.combat.flight.scenery = self.scenery_cylinders(mesh);
            self.combat.flight.scenery_damage = self.scenery_damage_table(mesh, tags);
        }
        let cells = if self.engine.cells.width > 0 {
            out.cells_from_map_load = true;
            self.engine.cells.clone()
        } else {
            passability::MeshCells::from_mesh(mesh)
        };
        for m in mesh.unit_markers() {
            let Ok(id) = u32::try_from(m.marker_id) else {
                continue;
            };
            if !self.units.contains_key(&id) {
                continue;
            }
            match Self::movement_definition(mesh, m, tags, postprocess.gameplay_version) {
                Ok(def) => {
                    let mut mover = engine_move::EngineMover::new(def, m.x, m.y, m.yaw as u16);
                    mover.place(&cells);
                    if let Some(em) = self
                        .engine
                        .monsters
                        .iter()
                        .find(|em| em.identifier == m.marker_id)
                    {
                        mover.speed = em.speed;
                        mover.monster_index = Some(em.index);
                        mover.set_object_scale(em.object.scale);
                        mover.passability.neighbours = em.neighbours;
                        if mover.block.as_ref().is_some_and(|b| {
                            b.site == draw_site::MOVE_CREATION_SPEED
                                || b.site == draw_site::MOVE_DEFINITION_198
                        }) {
                            mover.block = None;
                        }
                        if let Err(e) = self.engine.changed_movement(em.index, &mut mover) {
                            self.engine_stop.get_or_insert(e);
                        }
                    } else if let Some(scale) = Self::fixed_object_scale(mesh, m, tags) {
                        mover.set_object_scale(scale);
                    }
                    if let Some(u) = self.units.get_mut(&id) {
                        u.engine = Some(mover);
                        out.attached += 1;
                    }
                }
                Err(e) => out.missing.push((id, e)),
            }
        }
        self.engine_terrain = Some(cells);
        {
            let w = &self.engine;
            for m in w.monsters.iter().filter(|m| m.identifier != 0) {
                if let Some(u) = m
                    .unit_key()
                    .and_then(|id| self.units.get_mut(&id))
                    .filter(|u| u.engine.is_some())
                {
                    u.position.y = Fixed16::from_raw(
                        m.object
                            .z
                            .wrapping_mul(65536 / mandrake_rec::WORLD_UNITS_PER_CELL),
                    );
                }
            }
        }
        self.apply_engine_orders();
        out
    }

    pub(crate) fn attach_shown_monsters(&mut self) {
        let world = &mut self.engine;
        let Some(creation) = world.marker_creation.clone() else {
            return;
        };
        for i in std::mem::take(&mut world.shown_monsters) {
            let m = &world.monsters[i];
            let Some(id) = m.unit_key() else {
                continue;
            };
            let Some(marker) = creation
                .mesh
                .markers
                .iter()
                .find(|k| k.marker_type == 3 && k.marker_id == m.identifier)
            else {
                if let Err(e) = Self::attach_located_monster(
                    &mut self.units,
                    self.mesh_grid.as_ref(),
                    world,
                    &creation,
                    i,
                ) {
                    self.engine_stop.get_or_insert(e);
                }
                continue;
            };
            if !self.units.contains_key(&id) {
                let Some(placement) = creation
                    .mesh
                    .unit_placements(1.0)
                    .into_iter()
                    .find(|p| p.marker_id == m.identifier)
                else {
                    continue;
                };
                let position = Vec3Fixed::new(
                    Fixed16::from_f32(placement.position.0),
                    Fixed16::from_f32(placement.position.1),
                    Fixed16::from_f32(placement.position.2),
                );
                let unit_tag = {
                    let u = world.combat(i).unit.0;
                    if u != 0 {
                        FourCC(u.to_be_bytes())
                    } else {
                        placement.unit_tag
                    }
                };
                let mut unit =
                    UnitEntity::spawn_archetype(id, unit_tag, m.team.max(0) as u8, position);
                if let Some(grid) = self.mesh_grid.as_ref() {
                    unit.position.y = unit.ground_height(grid);
                }
                self.units.insert(id, unit);
            }
            let Some(unit) = self.units.get_mut(&id) else {
                continue;
            };
            let object = EngineUnitObject {
                scale: m.object.scale.clamp(0, i32::from(u16::MAX)) as u16,
                vitality: m.object.vitality.clamp(0, i32::from(i16::MAX)) as i16,
                permutation: m.object.permutation,
            };
            unit.engine_object = Some(object);
            unit.health = Fixed16::from_i32(i32::from(object.vitality));
            unit.max_health = engine_max_health(&m.object);
            if unit
                .engine
                .as_ref()
                .is_some_and(|e| e.monster_index == Some(i))
            {
                continue;
            }
            match Self::movement_definition(
                &creation.mesh,
                marker,
                creation.as_ref(),
                world.formation_globals.gameplay_version,
            ) {
                Ok(def) => {
                    let mut mover =
                        engine_move::EngineMover::new(def, marker.x, marker.y, marker.yaw as u16);
                    mover.place(&world.cells);
                    mover.speed = m.speed;
                    mover.monster_index = Some(i);
                    mover.set_object_scale(m.object.scale);
                    mover.passability.neighbours = m.neighbours;
                    if mover.block.as_ref().is_some_and(|b| {
                        b.site == draw_site::MOVE_CREATION_SPEED
                            || b.site == draw_site::MOVE_DEFINITION_198
                    }) {
                        mover.block = None;
                    }
                    if let Err(e) = world.changed_movement(i, &mut mover) {
                        self.engine_stop.get_or_insert(e);
                    }
                    unit.engine = Some(mover);
                }
                Err(e) => {
                    self.engine_stop.get_or_insert(engine_rng::Unmodelled::new(
                        draw_site::SHOWN_MONSTER_NO_MOVER,
                        format!("make_monster_visible: monster {id} has no movement model: {e}"),
                    ));
                }
            }
        }
    }

    fn attach_located_monster(
        units: &mut entities::UnitMap,
        grid: Option<&MeshGrid>,
        world: &engine_rng::EngineWorld,
        creation: &engine_rng::MarkerCreation,
        i: usize,
    ) -> Result<(), engine_rng::Unmodelled> {
        let m = &world.monsters[i];
        let Some(id) = m.unit_key() else {
            return Ok(());
        };
        let unit_tag = FourCC(world.combat(i).unit.0.to_be_bytes());
        if units.contains_key(&id) {
            return Err(engine_rng::Unmodelled::new(
                draw_site::NEW_MONSTER_AT_LOCATION_NO_TAGS,
                format!("new_monster_at_location: monster {id} has the key of another unit"),
            ));
        }
        let Some(mut place) = creation.mesh.markers.first().cloned() else {
            return Ok(());
        };
        (place.x, place.y, place.z) = (m.position.0, m.position.1, m.object.z);
        let (px, py, pz) = creation.mesh.marker_world_position(&place, 1.0);
        let position = Vec3Fixed::new(
            Fixed16::from_f32(px),
            Fixed16::from_f32(py),
            Fixed16::from_f32(pz),
        );
        let mut unit = UnitEntity::spawn_archetype(id, unit_tag, m.team.max(0) as u8, position);
        if let Some(grid) = grid {
            unit.position.y = unit.ground_height(grid);
        }
        let myth = (m.facing >> 7) as i32;
        unit.facing = Angle::from_raw((256 - myth).rem_euclid(512) as u16);
        let object = EngineUnitObject {
            scale: m.object.scale.clamp(0, i32::from(u16::MAX)) as u16,
            vitality: m.object.vitality.clamp(0, i32::from(i16::MAX)) as i16,
            permutation: m.object.permutation,
        };
        unit.engine_object = Some(object);
        unit.health = Fixed16::from_i32(i32::from(object.vitality));
        unit.max_health = engine_max_health(&m.object);
        let def = Self::unit_movement_definition(
            &creation.mesh,
            unit_tag,
            creation,
            world.formation_globals.gameplay_version,
        )
        .map_err(|e| {
            engine_rng::Unmodelled::new(
                draw_site::NEW_MONSTER_AT_LOCATION_NO_TAGS,
                format!("new_monster_at_location: monster {id} has no movement model: {e}"),
            )
        })?;
        let mut mover = engine_move::EngineMover::new(def, m.position.0, m.position.1, m.facing);
        mover.place(&world.cells);
        mover.speed = m.speed;
        mover.monster_index = Some(i);
        mover.set_object_scale(m.object.scale);
        mover.passability.neighbours = m.neighbours;
        if mover.block.as_ref().is_some_and(|b| {
            b.site == draw_site::MOVE_CREATION_SPEED || b.site == draw_site::MOVE_DEFINITION_198
        }) {
            mover.block = None;
        }
        world.changed_movement(i, &mut mover)?;
        unit.engine = Some(mover);
        units.insert(id, unit);
        Ok(())
    }

    pub(crate) fn take_cells_change(&mut self, taken: Option<&mut projectiles::FlightTerrain>) {
        let world = &mut self.engine;
        if !std::mem::take(&mut world.cells_changed) {
            return;
        }
        self.engine_terrain = Some(world.cells.clone());
        let grid = |t: projectiles::FlightTerrain| {
            t.with_models(models::ModelGrid::new(&world.cells, &world.models))
                .with_cell_flags(&world.cells.flags)
                .with_heights(&world.heights)
        };
        self.combat.flight.ground = self.combat.flight.ground.take().map(grid);
        self.combat.flight.terrain = self.combat.flight.terrain.take().map(grid);
        if let Some(t) = taken {
            *t = grid(std::mem::take(t));
        }
    }

    pub(crate) fn apply_engine_orders(&mut self) {
        self.attach_shown_monsters();
        let world = &mut self.engine;
        for i in std::mem::take(&mut world.impassability_updates) {
            let (id, nb) = (world.monsters[i].identifier, world.monsters[i].neighbours);
            if let Some(id) = engine_rng::unit_key(id)
                && let Some(m) = self.units.get_mut(&id).and_then(|u| u.engine.as_mut())
            {
                m.passability.neighbours = nb;
            }
        }
        self.take_cells_change(None);
        let world = &mut self.engine;
        world.pending_attacks.clear();
        let orders = std::mem::take(&mut world.pending_orders);
        for o in orders {
            let id = world.monsters[o.monster].identifier;
            if self.logging_command
                && let Some(trace) = self.command_log.as_mut()
            {
                let points: Vec<String> = o
                    .waypoints
                    .iter()
                    .map(|p| format!("{},{}", p.0, p.1))
                    .collect();
                trace.push(format!(
                    "W t={} m={id} n={} pts={} flags={:#x} facing={:#x}",
                    self.current_tick,
                    o.waypoints.len(),
                    points.join(";"),
                    o.flags,
                    o.facing,
                ));
            }
            if let Some(id) = engine_rng::unit_key(id)
                && let Some(m) = self.units.get_mut(&id).and_then(|u| u.engine.as_mut())
                && let Err(e) = world.order_monster(&o, m)
            {
                self.engine_stop.get_or_insert(e);
            }
        }
    }

    pub(crate) fn lend_movers(&mut self) {
        let world = &mut self.engine;
        world.movers.clear();
        world.movers.extend(world.monsters.iter().map(|m| {
            m.unit_key()
                .and_then(|id| self.units.get_mut(&id))
                .filter(|u| u.health > Fixed16::ZERO && u.state != UnitState::Dead)
                .and_then(|u| u.engine.take())
        }));
    }

    pub(crate) fn return_movers(&mut self) {
        self.return_movers_to(None);
    }

    pub(crate) fn return_movers_to(&mut self, mut ids: Option<&mut Vec<u32>>) {
        let world = &mut self.engine;
        for &(old, new) in &world.renamed_monsters {
            if let (Some(old), Some(new)) = (engine_rng::unit_key(old), engine_rng::unit_key(new))
                && !self.units.contains_key(&new)
                && let Some(mut u) = self.units.remove(&old)
            {
                u.id = new;
                self.units.insert(new, u);
            }
        }
        let mut movers = std::mem::take(&mut world.movers);
        for (m, mover) in world.monsters.iter().zip(movers.drain(..)) {
            if let (Some(id), Some(mover)) = (m.unit_key(), mover)
                && let Some(u) = self.units.get_mut(&id)
            {
                u.engine = Some(mover);
                if let Some(ids) = ids.as_deref_mut() {
                    ids.push(id);
                }
            }
        }
        world.movers = movers;
        self.attach_shown_monsters();
    }

    pub(crate) fn sync_engine_positions(&mut self) {
        let world = &mut self.engine;
        for i in 0..world.monsters.len() {
            if let Some(id) = world.monsters[i].unit_key()
                && let Some(mv) = self.units.get(&id).and_then(|u| u.engine.as_ref())
            {
                world.move_monster(i, (mv.x, mv.y));
                let m = &mut world.monsters[i];
                m.facing = mv.facing;
                m.object.velocity = mv.velocity;
            }
        }
    }

    fn fixed_object_scale(
        mesh: &MeshTag,
        marker: &mandrake_tag::MarkerEntry,
        tags: &dyn engine_rng::TagSource,
    ) -> Option<i32> {
        let unit = tags.tag(*b"unit", mesh.palette_tag_for_marker(marker)?)?;
        let mons = tags.tag(*b"mons", FourCC::new(unit.get(0..4)?.try_into().ok()?))?;
        let obje = tags.tag(
            *b"obje",
            FourCC::new(mons.get(0x84..0x88)?.try_into().ok()?),
        )?;
        let be16 = |o: usize| obje.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
        let base = i32::from(be16(0x0c)? as u16);
        (be16(0x0e)? <= 0).then_some(base)
    }

    fn movement_definition(
        mesh: &MeshTag,
        marker: &mandrake_tag::MarkerEntry,
        tags: &dyn engine_rng::TagSource,
        gameplay_version: i32,
    ) -> Result<engine_move::MovementDefinition, String> {
        let unit_tag = mesh
            .palette_tag_for_marker(marker)
            .ok_or_else(|| "no palette entry".to_string())?;
        Self::unit_movement_definition(mesh, unit_tag, tags, gameplay_version)
    }

    fn unit_movement_definition(
        mesh: &MeshTag,
        unit_tag: FourCC,
        tags: &dyn engine_rng::TagSource,
        gameplay_version: i32,
    ) -> Result<engine_move::MovementDefinition, String> {
        let unit = tags
            .tag(*b"unit", unit_tag)
            .filter(|u| u.len() >= 8)
            .ok_or_else(|| format!("unit {unit_tag} missing"))?;
        let mons_tag = FourCC::new([unit[0], unit[1], unit[2], unit[3]]);
        let core_tag = FourCC::new([unit[4], unit[5], unit[6], unit[7]]);
        let mons = tags
            .tag(*b"mons", mons_tag)
            .ok_or_else(|| format!("mons {mons_tag} missing"))?;
        let collection = tags
            .tag(*b"core", core_tag)
            .and_then(|c| mandrake_tag::CollectionReference::from_bytes(&c).ok())
            .ok_or_else(|| format!("core {core_tag} missing"))?
            .collection_tag;
        let mut headers = tags
            .tag(*b".256", collection)
            .and_then(|d| mandrake_tag::sequence_headers(&d).ok())
            .ok_or_else(|| format!("collection {collection} missing"))?;
        engine_rng::equalize_collection_for_palette(mesh, tags, collection, &mut headers)
            .map_err(|e| format!("{e:?}"))?;
        engine_move::MovementDefinition::from_tags(mons_tag, &mons, &headers, gameplay_version)
    }

    pub fn resolve_command_units(&self, ids: &[u16]) -> Vec<u32> {
        ids.iter()
            .take(0x7e)
            .filter(|&&id| id != 0)
            .filter(|&&id| {
                self.engine
                    .monsters
                    .iter()
                    .any(|m| m.identifier as u16 == id)
            })
            .map(|&id| u32::from(id))
            .collect()
    }

    pub fn unit_world_position(&self, id: u32) -> Option<(i32, i32)> {
        self.units
            .get(&id)
            .and_then(|u| u.engine.as_ref())
            .map(|m| (m.x, m.y))
    }

    pub fn set_mesh_grid(&mut self, grid: MeshGrid) {
        self.mesh_grid = Some(grid);
    }

    pub fn load_mesh(&mut self, mesh: &MeshTag) {
        self.mesh_grid = Some(MeshGrid::from_mesh(mesh));
    }

    pub fn spawn_unit(&mut self, archetype: FourCC, team: u8, position: Vec3Fixed) -> u32 {
        while self.units.contains_key(&self.next_unit_id) {
            self.next_unit_id += 1;
        }
        let id = self.next_unit_id;
        self.next_unit_id += 1;
        self.spawn_unit_with_id(id, archetype, team, position)
    }

    pub fn spawn_unit_with_id(
        &mut self,
        id: u32,
        archetype: FourCC,
        team: u8,
        position: Vec3Fixed,
    ) -> u32 {
        let mut unit = UnitEntity::spawn_archetype(id, archetype, team, position);
        if let Some(ref grid) = self.mesh_grid {
            unit.position.y = unit.ground_height(grid);
        }

        self.units.insert(id, unit);
        id
    }

    pub fn spawn_initial_units_from_mesh(&mut self, mesh: &MeshTag) -> Vec<u32> {
        let mut spawned = Vec::new();
        for placement in mesh.unit_placements(1.0) {
            let pos = Vec3Fixed::new(
                Fixed16::from_f32(placement.position.0),
                Fixed16::from_f32(placement.position.1),
                Fixed16::from_f32(placement.position.2),
            );
            let team = placement.team.max(0) as u8;
            let uid = match u32::try_from(placement.marker_id) {
                Ok(id) if id > 0 && !self.units.contains_key(&id) => {
                    self.spawn_unit_with_id(id, placement.unit_tag, team, pos)
                }
                _ => self.spawn_unit(placement.unit_tag, team, pos),
            };
            if let Some(unit) = self.units.get_mut(&uid) {
                let myth = ((placement.yaw as u16) >> 7) as i32;
                unit.facing = Angle::from_raw((256 - myth).rem_euclid(512) as u16);
            }
            spawned.push(uid);
        }
        spawned
    }

    pub fn spawn_units_for_monsters(
        &mut self,
        mesh: &MeshTag,
        monsters: &[engine_rng::EngineMonster],
    ) -> Vec<u32> {
        let placements = mesh.unit_placements(1.0);
        let mut spawned = Vec::new();
        for m in monsters {
            if m.flags & 0x4000_0000 != 0 || m.object.no_shape {
                continue;
            }
            let (Some(uid), Some(placement)) = (
                m.unit_key(),
                placements.iter().find(|p| p.marker_id == m.identifier),
            ) else {
                continue;
            };
            if uid == 0 || self.units.contains_key(&uid) {
                continue;
            }
            let pos = Vec3Fixed::new(
                Fixed16::from_f32(placement.position.0),
                Fixed16::from_f32(placement.position.1),
                Fixed16::from_f32(placement.position.2),
            );
            let unit_tag = ({
                let u = self.engine.combat(m.index).unit.0;
                (u != 0).then_some(FourCC(u.to_be_bytes()))
            })
            .unwrap_or(placement.unit_tag);
            self.spawn_unit_with_id(uid, unit_tag, m.team.max(0) as u8, pos);
            if let Some(unit) = self.units.get_mut(&uid) {
                let myth = ((placement.yaw as u16) >> 7) as i32;
                unit.facing = Angle::from_raw((256 - myth).rem_euclid(512) as u16);
                let object = EngineUnitObject {
                    scale: m.object.scale.clamp(0, i32::from(u16::MAX)) as u16,
                    vitality: m.object.vitality.clamp(0, i32::from(i16::MAX)) as i16,
                    permutation: m.object.permutation,
                };
                unit.engine_object = Some(object);
                unit.health = Fixed16::from_i32(i32::from(object.vitality));
                unit.max_health = engine_max_health(&m.object);
            }
            spawned.push(uid);
        }
        spawned
    }
}
