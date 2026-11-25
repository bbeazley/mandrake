use crate::*;

impl Simulation {
    pub fn step_with_commands(&mut self) -> TickStateHash {
        if self.ticks_stopped() || !self.net_body_may_run() {
            return TickStateHash {
                tick: self.current_tick,
                rng_state: self.rng.state,
                active_commands_count: 0,
                state_checksum: self.state_checksum,
            };
        }
        let active_commands_count = self.run_command_stage();
        self.finish_tick_body(active_commands_count)
    }

    pub fn run_command_stage(&mut self) -> usize {
        let body_tick = self.current_tick;
        self.net_before_body(body_tick);
        self.record_object_history();

        if self.replay.is_some() {
            self.engine.formation_globals.gameplay_version =
                gameplay::replay_clamp(self.engine.formation_globals.gameplay_version);
        }
        if let Some(mut input) = self.scripted_input.take() {
            self.give_film_commands(&mut input);
            self.scripted_input = Some(input);
        }
        self.stamp_local_commands();
        if body_tick != 0 && body_tick.is_multiple_of(SYNC_SNAPSHOT_TICKS) {
            self.sync_snapshot = (self.rng.state, body_tick);
        }
        self.sync_solid_objects();
        let n = self.execute_monster_commands(body_tick);
        self.total_dispatched_commands += n;
        let live = self.engine.mode != 0;
        if std::mem::take(&mut self.first_body) && live {
            self.broadcast_formation_definition();
        }
        n
    }

    fn broadcast_formation_definition(&mut self) {
        let parameters = formations::FormationDefinition::STATIC_DEFAULT
            .fields
            .to_vec();
        let payload =
            CommandPayload::SyncFormation(mandrake_rec::SyncFormationCommand { parameters });
        self.local_commands
            .push(QueuedCommand::Packet(CommandPacket {
                tick: 0,
                player_index: 0,
                command_type: mandrake_rec::CommandType::SyncFormation,
                raw_payload: payload.encode().unwrap_or_default(),
                payload,
            }));
    }

    pub fn finish_tick_body(&mut self, active_commands_count: usize) -> TickStateHash {
        let body_tick = self.current_tick;
        let had_lightning = !self.engine.lightning.records.is_empty();
        self.update_lightning(body_tick);
        if had_lightning {
            self.sync_solid_objects();
        }
        self.update_projectile_flight(body_tick);
        self.sync_solid_objects();

        self.update_engine_monsters(body_tick);
        if !self.engine.mesh_effects.is_empty() {
            self.update_engine_mesh_effects();
            self.sync_solid_objects();
        }
        self.update_engine_map_actions(body_tick);
        let had_explosions = !self.combat.explosions.is_empty();
        self.update_explosions(body_tick);
        if had_explosions {
            self.sync_solid_objects();
        }
        self.update_engine_fire_and_wind(body_tick);
        self.sync_solid_objects();
        self.update_engine_model_animations();

        self.refresh_flag_positions();
        self.update_engine_player_statistics(body_tick as i32);
        self.sync_unit_vitality();

        self.current_tick = body_tick + 1;
        self.end_of_tick_body();
        self.count_down_sync();

        let mut entity_hash = 0u64;
        for unit in self.units.values() {
            entity_hash = entity_hash.wrapping_add(
                (unit.id as u64).wrapping_mul(0x94d049bb133111eb)
                    ^ ((unit.position.x.0 as u64) << 32 | (unit.position.z.0 as u32 as u64)),
            );
        }
        for proj in &self.combat.flight.flights {
            entity_hash = entity_hash.wrapping_add(
                (proj.id as u64).wrapping_mul(0xbf58476d1ce4e5b9)
                    ^ ((proj.object.x as u64) << 32 | (proj.object.y as u32 as u64)),
            );
        }

        let tick_hash = (self.current_tick as u64).wrapping_mul(0x517cc1b727220a95)
            ^ ((self.rng.state as u64).wrapping_mul(0x9e3779b97f4a7c15))
            ^ ((active_commands_count as u64).wrapping_mul(0xbf58476d1ce4e5b9))
            ^ entity_hash;

        self.state_checksum = self.state_checksum.rotate_left(5) ^ tick_hash;

        TickStateHash {
            tick: self.current_tick,
            rng_state: self.rng.state,
            active_commands_count,
            state_checksum: self.state_checksum,
        }
    }

    fn sync_unit_vitality(&mut self) {
        let world = &self.engine;
        for m in &world.monsters {
            let vitality = m.object.vitality;
            if vitality <= 0 {
                continue;
            }
            let Some(unit) = m.unit_key().and_then(|id| self.units.get_mut(&id)) else {
                continue;
            };
            if unit.health > Fixed16::ZERO && unit.state != UnitState::Dead {
                unit.health = Fixed16::from_i32(vitality.min(i32::from(i16::MAX)));
                unit.max_health = engine_max_health(&m.object);
            }
        }
    }

    fn update_engine_monsters(&mut self, time: u32) {
        if self.engine_stop.is_some() {
            return;
        }
        self.lend_movers();
        let mut world = std::mem::take(&mut self.engine);
        let terrain = self.engine_terrain.take();
        let mut rng_state = std::mem::take(&mut self.rng);
        let mut sites = std::mem::take(&mut self.engine_sites);
        world.projectile_records = std::mem::take(&mut self.combat.flight.flights);
        world.dormant_objects = std::mem::take(&mut self.combat.flight.dormant);
        world.solid_elsewhere.clear();
        let result = {
            let mut rng = engine_rng::EngineRng {
                rng: &mut rng_state,
                sites: &mut sites,
            };
            world.update_monsters(&mut rng, terrain.as_ref(), time as i32, self)
        };
        self.rng = rng_state;
        self.engine_sites = sites;
        if let Err((_, e)) = result {
            self.engine_stop = Some(e);
        }
        self.combat.flight.flights = std::mem::take(&mut world.projectile_records);
        self.combat.flight.dormant = std::mem::take(&mut world.dormant_objects);
        for i in std::mem::take(&mut world.scenery_taken) {
            self.combat.flight.scenery.retain(|c| c.object != i);
        }
        for (d, media_height) in std::mem::take(&mut world.launched_groups) {
            self.combat
                .events
                .push(combat::CombatEvent::ProjectileGroupLaunched {
                    tick: time,
                    group: d.group,
                    flags: d.flags,
                    object: d.object,
                    point: d.point,
                    media_height,
                });
        }
        for p in self.combat.flight.flights.iter_mut().filter(|p| p.id == 0) {
            self.combat.next_projectile_id += 1;
            p.id = self.combat.next_projectile_id;
            p.launched_tick = time;
        }
        self.engine_terrain = terrain;
        self.engine = world;
        self.sync_solid_objects();
        let mut moved = Vec::new();
        self.return_movers_to(Some(&mut moved));
        self.publish_movement(&moved);
    }

    fn update_engine_mesh_effects(&mut self) {
        if self.engine_stop.is_some() {
            return;
        }
        let mut world = std::mem::take(&mut self.engine);
        if world.mesh_effects.is_empty() {
            self.engine = world;
            return;
        }
        let mut terrain = self.combat.flight.terrain.take();
        let mut ground = self.combat.flight.ground.take();
        let mut grid = self.mesh_grid.take();
        let mut dormant = std::mem::take(&mut self.combat.flight.dormant);

        crate::mesh_effects::update_mesh_effects(
            &mut world,
            terrain.as_mut(),
            ground.as_mut(),
            grid.as_mut(),
            &mut dormant,
            &mut self.combat.flight.flights,
            &mut self.combat.flight.scenery,
            Some(&mut self.units),
        );

        self.combat.flight.dormant = dormant;
        self.mesh_grid = grid;
        self.combat.flight.ground = ground;
        self.combat.flight.terrain = terrain;
        self.engine = world;
    }

    fn update_engine_fire_and_wind(&mut self, body_tick: u32) {
        if self.engine_stop.is_some() {
            return;
        }
        let world = &mut self.engine;
        let mut rng = engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        world.update_particle_clock(&mut rng);
        self.update_engine_model_effects(body_tick);
        if self.engine_stop.is_some() {
            return;
        }
        self.update_engine_fire(body_tick);
        if self.engine_stop.is_some() {
            return;
        }
        if self.engine.connectors_changing() {
            self.sync_engine_positions();
            self.engine.update_connectors();
            self.apply_engine_orders();
        }
        let world = &mut self.engine;
        let mut rng = engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        let r = world.update_wind(&mut rng);
        if let Err(e) = r {
            self.engine_stop = Some(e);
        }
    }

    fn update_engine_model_effects(&mut self, body_tick: u32) {
        if self.engine_stop.is_some() || !(self.engine.model_effects.any()) {
            return;
        }
        self.sync_engine_positions();
        let world = &mut self.engine;
        let mut rng = engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        if let Err(e) = world.update_model_effects(body_tick as i32, &mut rng) {
            self.engine_stop = Some(e);
        }
        self.apply_engine_orders();
    }

    fn update_engine_model_animations(&mut self) {
        if self.engine_stop.is_some() {
            return;
        }
        let running = {
            self.engine
                .model_animations
                .animations
                .iter()
                .any(|a| a.playback.active)
        };
        if !running {
            return;
        }
        self.sync_engine_positions();
        let world = &mut self.engine;
        let mut animations = std::mem::take(&mut world.model_animations);
        let r = animations.update(world);
        world.model_animations = animations;
        if let Err(e) = r {
            self.engine_stop = Some(e);
        }
        self.apply_engine_orders();
    }

    pub fn model_animation_frames(&self) -> Vec<model_animations::ModelAnimationFrameState> {
        self.engine.model_animations.frames()
    }

    pub fn take_model_animation_sounds(&mut self) -> Vec<model_animations::ModelAnimationSound> {
        std::mem::take(&mut self.engine.model_animations.sounds)
    }

    pub(crate) fn lend_objects(&mut self) {
        {
            let world = &mut self.engine;
            world.projectile_records = std::mem::take(&mut self.combat.flight.flights);
            world.dormant_objects = std::mem::take(&mut self.combat.flight.dormant);
            world.solid_elsewhere.clear();
        }
    }

    pub(crate) fn sync_solid_objects(&mut self) {
        let mut v = std::mem::take(&mut self.engine.solid_elsewhere);
        v.clear();
        let f = &self.combat.flight;
        if !(f.flights.is_empty() && f.dormant.is_empty()) {
            v.extend(
                self.engine
                    .solid_views(f.flights.iter().map(|p| &**p), &f.dormant),
            );
        }
        self.engine.solid_elsewhere = v;
    }

    pub(crate) fn return_objects(&mut self) {
        {
            let world = &mut self.engine;
            self.combat.flight.flights = std::mem::take(&mut world.projectile_records);
            self.combat.flight.dormant = std::mem::take(&mut world.dormant_objects);
            for i in std::mem::take(&mut world.scenery_taken) {
                self.combat.flight.scenery.retain(|c| c.object != i);
            }
        }
        self.sync_solid_objects();
    }

    fn update_engine_player_statistics(&mut self, tick: i32) {
        let w = &mut self.engine;
        if self.engine_stop.is_some() || !w.statistics_rebuild_due() {
            let mode = w.mode;
            w.update_statistics(tick, tick + 1, None);
            w.game.update_player_statistics(mode);
            if let Some(e) = w.statistics_stop.take() {
                self.engine_stop.get_or_insert(e);
            }
            return;
        }
        self.sync_engine_positions();
        self.lend_movers();
        self.lend_objects();
        let mut rng = engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        let w = &mut self.engine;
        let mode = w.mode;
        w.update_statistics(tick, tick + 1, Some(&mut rng));
        w.game.update_player_statistics(mode);
        let stop = w.statistics_stop.take();

        self.return_objects();
        self.return_movers();
        if let Some(e) = stop {
            self.engine_stop = Some(e);
        }
        self.apply_engine_orders();
    }

    pub fn lend_for_map_actions(&mut self) {
        self.sync_engine_positions();
        self.lend_movers();
        self.lend_objects();
    }

    fn update_engine_map_actions(&mut self, time: u32) {
        if self.engine_stop.is_some() {
            return;
        }
        if time & 0x1f != 0 {
            self.apply_engine_orders();
            return;
        }
        self.sync_engine_positions();
        self.lend_movers();
        self.lend_objects();
        let mut selection: Vec<i16> = self
            .units
            .values()
            .filter(|u| u.selected)
            .map(|u| u.id as u16 as i16)
            .collect();
        selection.sort_unstable();
        {
            let w = &mut self.engine;
            w.local_selection = selection;
        }
        let mut rng = engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        let r = Some(self.engine.update_map_actions(time as i32, &mut rng));
        self.return_objects();
        self.return_movers();
        if let Some(Err(e)) = r {
            self.engine_stop = Some(e);
        }
        self.apply_engine_orders();
    }

    fn refresh_flag_positions(&mut self) {
        let w = &mut self.engine;
        for flag in w.flag_data.records.iter_mut().flatten() {
            let index = flag.object;
            if let Some(p) = self
                .combat
                .flight
                .flights
                .iter()
                .find(|p| p.shape.as_ref().is_some_and(|o| o.index == index))
            {
                flag.position = (p.object.x, p.object.y);
            } else if let Some(d) = self
                .combat
                .flight
                .dormant
                .iter()
                .find(|d| d.object.index == index)
            {
                flag.position = (d.position[0], d.position[1]);
            }
        }
    }

    fn end_of_tick_body(&mut self) {
        let tick = self.current_tick as i32;
        let w = &mut self.engine;
        let game_was_over = w.game.game_over;
        let (event, r) = w.game.end_of_tick_body(tick, &mut w.game_options);
        let just_over = w.game.game_over && !game_was_over;
        if just_over {
            w.build_monster_initializers_at_game_over();
        }
        let version = w.formation_globals.gameplay_version;
        let team = w.local_team();
        if let Err(e) = r {
            self.engine_stop.get_or_insert(e);
        }
        if just_over {
            self.myth_game_is_over_pause(version, team);
        }
        if event == game_state::TimerEvent::PreGameEnded {
            self.pre_game = false;
            if version > 0x6ad {
                for m in &mut self.engine.monsters {
                    m.flags_8 &= !0x80_0000;
                }
            }
        }
    }

    pub(crate) fn myth_game_is_over_pause(&mut self, gameplay_version: i32, team: i16) {
        let host = self
            .net
            .as_ref()
            .is_some_and(|n| n.role == net_game::NetRole::Host);
        if self.is_network_game()
            && game_speed::coop_host_may_send(
                host,
                self.game_option_flags(),
                gameplay_version,
                team,
            )
        {
            let packet = CommandPacket::parse(0, 0xff, net_game::PAUSE, &[0, 0, 0, 0]);
            self.local_commands.push(QueuedCommand::Packet(packet));
        }
    }

    pub fn ticks_stopped(&self) -> bool {
        match &self.engine_stop {
            Some(s) if self.halt_on_stop || s.site == draw_site::NO_ENGINE_MAP => true,
            _ => self.engine.game.ticks_stopped(),
        }
    }

    pub fn game_end(&self) -> Option<game_state::GameEnd> {
        self.engine.game.end()
    }

    pub fn game_option_flags(&self) -> u32 {
        self.engine.game_options
    }

    pub fn eliminate_team(&mut self, team: i16) -> Result<(), engine_rng::Unmodelled> {
        let w = &mut self.engine;
        let mut rng = engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        let r = w.eliminate_team(team, Some(&mut rng));
        self.units.retain(|_, u| u.team != team as u8);
        r
    }

    pub fn statistics(&self) -> Option<statistics::StatisticsView<'_>> {
        Some(statistics::StatisticsView {
            statistics: &self.engine.statistics,
            players: &self.engine.players,
            game_type: self.engine.game_type,
            game_mode: self.engine.mode,
            frozen: self.engine.game.flags & game_state::FLAG_GAME_OVER != 0,
            local_team: self.engine.local_team(),
            difficulty: self.engine.difficulty,
            end_code: self.engine.game.end_code,
            world: &self.engine,
        })
    }

    pub fn statistics_samples(&self) -> Option<statistics::SampleRange<'_>> {
        Some(self.engine.statistics.sample_range())
    }

    pub fn close_carnage_report(&mut self) {
        {
            let w = &mut self.engine;
            w.game.close_carnage_report();
        }
    }

    pub fn game_over(&self) -> bool {
        self.engine.game.game_over
    }

    pub fn monster_initializers(&self) -> Vec<mandrake_rec::header::MonsterInitializer> {
        self.engine.monster_initializers.records()
    }

    pub fn permanent_rankings(&self) -> Vec<statistics::PermanentRanking> {
        self.engine.statistics.permanent_rankings.clone()
    }

    pub fn step(&mut self) -> TickStateHash {
        self.step_with_commands()
    }
}
