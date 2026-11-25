use crate::*;

impl Simulation {
    pub fn local_movement_header(
        &self,
        unit_ids: &[u32],
        facing: Option<u16>,
    ) -> Result<engine_rng::MovementHeader, engine_rng::Unmodelled> {
        let w = &self.engine;
        let selection: Vec<usize> = unit_ids
            .iter()
            .filter_map(|&id| w.monster_index_by_identifier(id as u16))
            .collect();
        w.local_movement_header(&selection, facing)
    }

    pub fn local_move_click(
        &mut self,
        unit_ids: &[u32],
        click: engine_rng::MoveClick,
        facing: Option<u16>,
        add_waypoint: bool,
    ) -> Result<Option<LocalMovement>, engine_rng::Unmodelled> {
        let w = &self.engine;
        let selection: Vec<usize> = unit_ids
            .iter()
            .filter_map(|&id| w.monster_index_by_identifier(id as u16))
            .collect();
        let sent = w.local_move_click(
            &mut self.local_waypoints,
            &selection,
            click,
            facing,
            add_waypoint,
        )?;
        Ok(sent.map(|(header, points)| {
            let c = LocalMovement {
                unit_ids: unit_ids.to_vec(),
                points,
                header,
            };
            self.queue_local_movement(c.clone());
            c
        }))
    }

    pub fn queue_local_movement(&mut self, mut command: LocalMovement) {
        let w = &self.engine;
        let paired = if w.formation_globals.gameplay_version <= game_options::GAMEPLAY_1_8_3
            || command.header.flags & 8 == 0
        {
            None
        } else {
            let selection: Vec<usize> = command
                .unit_ids
                .iter()
                .filter_map(|&id| w.monster_index_by_identifier(id as u16))
                .collect();
            match w.pair_local_movement(&selection, &command.points, command.header) {
                Ok(pair) => pair.map(|(order, centre)| {
                    let ids = order
                        .iter()
                        .map(|&m| u32::from(w.monsters[m].identifier as u16))
                        .collect();
                    (ids, centre)
                }),
                Err(e) => {
                    tracing::debug!("pairing a local movement: {e}");
                    None
                }
            }
        };
        if let Some((ids, centre)) = paired {
            command.unit_ids = ids;
            command.header.flags |= mandrake_rec::MOVEMENT_FLAG_MATCHED;
            self.local_commands
                .push(QueuedCommand::MatchedMovement(command, centre));
            return;
        }
        self.local_commands.push(QueuedCommand::Movement(command));
    }

    pub fn queue_local_general(&mut self, command: LocalGeneral) {
        self.local_commands.push(QueuedCommand::General(command));
    }

    pub fn queue_local_ally(&mut self, team: i16, other: i16) {
        let data = ally::ally_command_data(team, other);
        self.local_commands
            .push(QueuedCommand::Packet(CommandPacket::parse(
                0,
                0,
                mandrake_rec::CommandType::Ally.to_u8(),
                &data,
            )));
    }

    pub fn queue_local_target(&mut self, mut command: LocalTarget) {
        if self.engine.formation_globals.gameplay_version > game_options::GAMEPLAY_1_8_3
            && command.target_ids.len() > 1
        {
            let index = |ids: &[u32]| -> Vec<usize> {
                ids.iter()
                    .filter_map(|&id| self.engine.monster_index_by_identifier(id as u16))
                    .collect()
            };
            let (attackers, targets) = (index(&command.unit_ids), index(&command.target_ids));
            if let Some(pairs) = self.engine.pair_local_targets(&attackers, &targets) {
                let id = |&m: &usize| u32::from(self.engine.monsters[m].identifier as u16);
                command.unit_ids = attackers.iter().take(0x7e).map(id).collect();
                command.target_ids = pairs.iter().map(id).collect();
                command.flags |= mandrake_rec::TARGET_FLAG_MATCHED;
            }
        }
        self.local_commands.push(QueuedCommand::Target(command));
    }

    pub fn queue_local_attack_location(&mut self, command: LocalAttackLocation) {
        self.local_commands
            .push(QueuedCommand::AttackLocation(command));
    }

    pub fn queue_local_pick_up(&mut self, mut command: LocalPickUp) {
        let max = if self.engine.formation_globals.gameplay_version <= game_options::GAMEPLAY_1_3 {
            0x1e
        } else {
            0x7e
        };
        command.unit_ids.truncate(max);
        self.local_commands.push(QueuedCommand::PickUp(command));
    }

    pub fn could_pick_up(&self, unit_ids: &[u32], index: i32) -> Option<i16> {
        let w = &self.engine;
        if unit_ids.is_empty() {
            return None;
        }
        let (records, dormant) = (&self.combat.flight.flights, &self.combat.flight.dormant);
        let is = |o: &engine_rng::EngineObject| o.index == index;
        let identifier = w
            .projectiles
            .iter()
            .map(|p| &p.object)
            .chain(dormant.iter().map(|d| &d.object))
            .chain(records.iter().filter_map(|p| p.shape.as_ref()))
            .chain(w.scenery.iter().filter(|s| !s.deleted).map(|s| &s.object))
            .find(|o| is(o))?
            .identifier;
        let object = (i16::try_from(index).ok()?, identifier);
        for &id in unit_ids {
            let Some(m) = w.monster_index_by_identifier(id as u16) else {
                continue;
            };
            match w.monster_could_pick_up_object_in(m, object, records, dormant) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("monster_could_pick_up_object not modelled ({e})");
                    return None;
                }
            }
        }
        Some(identifier)
    }

    pub fn is_replay(&self) -> bool {
        self.engine.mode == 0
    }

    pub fn local_player(&self) -> i16 {
        self.engine.local_player
    }

    pub(crate) fn stamp_local_commands(&mut self) {
        if self.is_replay() {
            self.local_commands.clear();
            return;
        }
        if self.is_network_game() {
            return;
        }
        let tick = self.current_tick + self.command_lead;
        let player = self.local_player() as u8;
        for c in std::mem::take(&mut self.local_commands) {
            let (sniffed, mut packet) = outgoing_packet(c);
            if !self.sniff_local_command(&sniffed) {
                continue;
            }
            packet.tick = tick;
            packet.player_index = player;
            if let Some(recorded) = self.recorded_commands.as_mut() {
                recorded.push(packet.clone());
            }
            self.commands_by_tick.entry(tick).or_default().push(packet);
        }
    }

    pub(crate) fn queue_sync_command(&mut self) {
        let (seed, tick) = self.sync_snapshot;
        let payload = mandrake_rec::writer::sync_command(seed, tick);
        self.local_commands
            .push(QueuedCommand::Packet(CommandPacket {
                tick: 0,
                player_index: 0,
                command_type: mandrake_rec::CommandType::Sync,
                raw_payload: payload.encode().unwrap_or_default(),
                payload,
            }));
    }

    pub(crate) fn count_down_sync(&mut self) {
        if !(self.engine.mode == 2) {
            return;
        }
        self.sync_timer -= 1;
        if self.sync_timer < 0 {
            self.queue_sync_command();
            self.sync_timer = SYNC_TIMER_RESET;
        }
    }
}
