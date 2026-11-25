use crate::*;

impl Simulation {
    pub(crate) fn execute_monster_commands(&mut self, t: u32) -> usize {
        let mut n = 0;
        while let Some(entry) = self.commands_by_tick.first_entry() {
            let k = *entry.key();
            if k > t {
                break;
            }
            for c in entry.remove() {
                if matches!(
                    c.command_type.to_u8(),
                    net_game::PAUSE | net_game::GAME_SPEED
                ) {
                    continue;
                }
                n += 1;
                if k != t {
                    self.late_commands.push((k, t));
                    self.game_is_out_of_sync("Myth2Code/monster_commands.c", 0xd2);
                }
                let mut c = c;
                c.player_index = self.map_command_player_index(c.player_index);
                let players = Some(self.engine.players.players.len());
                let p = c.player_index;
                if p != 0xff && (p >= 0x80 || players.is_some_and(|n| usize::from(p) >= n)) {
                    continue;
                }
                let order = matches!(
                    c.command_type,
                    mandrake_rec::CommandType::General
                        | mandrake_rec::CommandType::Movement
                        | mandrake_rec::CommandType::Target
                        | mandrake_rec::CommandType::AttackLocation
                        | mandrake_rec::CommandType::PickUp
                        | mandrake_rec::CommandType::Rotation
                );
                if order && (self.engine.game.state == 0) {
                    continue;
                }
                if self.command_log.is_some() {
                    self.engine.target_log = Some(Vec::new());
                }
                self.logging_command = self.command_log.is_some();
                self.execute_monster_command(&c);
                self.logging_command = false;
                self.flush_target_log();
            }
        }
        n
    }

    fn flush_target_log(&mut self) {
        let t = self.current_tick;
        let w = &mut self.engine;
        let Some(calls) = w.target_log.take() else {
            return;
        };
        let id = |m: i32| -> i32 {
            usize::try_from(m).map_or(-1, |m| i32::from(w.monsters[m].identifier))
        };
        if let Some(trace) = self.command_log.as_mut() {
            for (a, flags, target) in calls {
                trace.push(format!(
                    "T t={t} a={} flags={flags:#x} target={}",
                    id(a as i32),
                    id(i32::from(target))
                ));
            }
        }
    }

    pub(crate) fn map_command_player_index(&self, player: u8) -> u8 {
        let w = &self.engine;
        if w.formation_globals.gameplay_version <= game_options::GAMEPLAY_1_8_4 || player == 0xff {
            return player;
        }
        let id = i32::from(player as i8);
        w.players
            .players
            .iter()
            .position(|p| p.id == id)
            .map_or(player, |i| i as u8)
    }

    pub(crate) fn execute_monster_command(&mut self, c: &CommandPacket) {
        let player = i16::from(c.player_index as i8);
        match &c.payload {
            CommandPayload::Null => {}
            CommandPayload::Sync(sync) => {
                if net_game::sync_differs(sync, self.sync_snapshot) && !self.is_out_of_sync() {
                    self.game_is_out_of_sync("Myth2Code/monster_commands.c", 0x679);
                }
            }
            CommandPayload::General(g) => {
                let ids: Vec<u32> = g.unit_ids.iter().map(|&u| u32::from(u)).collect();
                self.execute_general(&ids, g.action, player);
            }
            CommandPayload::Movement(m) => {
                let ids: Vec<u32> = m.unit_ids.iter().map(|&u| u32::from(u)).collect();
                let points: Vec<(i32, i32)> = m.waypoints.iter().map(|w| (w.x, w.y)).collect();
                let header = engine_rng::MovementHeader {
                    flags: m.flags,
                    formation: m.formation,
                    facing: m.facing as u16,
                };
                let matched = {
                    self.engine.formation_globals.gameplay_version > game_options::GAMEPLAY_1_8_3
                        && m.flags & mandrake_rec::MOVEMENT_FLAG_MATCHED != 0
                };
                let centre = if matched {
                    mandrake_rec::movement_formation_centre(&c.raw_payload)
                        .map(Ok)
                        .unwrap_or(Err(engine_rng::Unmodelled::new(
                            draw_site::MOVEMENT_NO_FORMATION_CENTRE,
                            "MOVEMENT with flag 0x80 and no formation centre",
                        )))
                } else {
                    Ok((0, 0))
                };
                match centre {
                    Ok(centre) => self.execute_movement(
                        &ids,
                        &points,
                        header,
                        player,
                        matched.then_some(centre),
                    ),
                    Err(e) => {
                        self.engine_stop.get_or_insert(e);
                        self.execute_movement(&ids, &points, header, player, None);
                    }
                }
            }
            CommandPayload::Target(t) => {
                let units = self.resolve_command_units(&t.unit_ids);
                let r = self.with_lent_movers(|w| {
                    let ids = |v: &[u16]| -> Vec<usize> {
                        v.iter()
                            .take(0x7e)
                            .filter_map(|&id| w.monster_index_by_identifier(id))
                            .collect()
                    };
                    if w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_8_3
                        && t.flags & mandrake_rec::TARGET_FLAG_MATCHED != 0
                    {
                        let keep = |v: &[u16]| -> Vec<Option<usize>> {
                            v.iter()
                                .take(0x7e)
                                .map(|&id| w.monster_index_by_identifier(id))
                                .collect()
                        };
                        let (a, b) = (keep(&t.unit_ids), keep(&t.target_ids));
                        return w.execute_matched_target_command(-1, player, t.flags, &a, &b);
                    }
                    let (a, b) = (ids(&t.unit_ids), ids(&t.target_ids));
                    w.execute_target_command(-1, player, t.flags, &a, &b)
                });
                self.after_engine_command(r);
                self.record_attack_order(
                    &units,
                    AttackTarget::Units(t.target_ids.iter().map(|&u| u32::from(u)).collect()),
                );
            }
            CommandPayload::AttackLocation(a) => {
                let units = self.resolve_command_units(&a.unit_ids);
                let r = self.with_lent_movers(|w| {
                    let monsters: Vec<usize> = a
                        .unit_ids
                        .iter()
                        .filter_map(|&id| w.monster_index_by_identifier(id))
                        .collect();
                    w.attack_location_command(
                        &monsters,
                        a.flags,
                        (a.target_x, a.target_y, a.target_z),
                        player,
                    )
                });
                self.after_engine_command(r);
                self.record_attack_order(
                    &units,
                    AttackTarget::Location(self.command_point_to_world(a.target_x, a.target_y)),
                );
            }
            CommandPayload::SyncFormation(f) => {
                let w = &mut self.engine;
                let mut fields = [0i16; 20];
                for (d, v) in fields.iter_mut().zip(&f.parameters) {
                    *d = *v;
                }
                w.set_formation_definition_for_player(
                    player,
                    formations::FormationDefinition { fields },
                );
            }
            _ if c.command_type == mandrake_rec::CommandType::Chat => {}
            _ if c.command_type == mandrake_rec::CommandType::UnitAdjustment => {
                self.execute_unit_adjustment(c);
            }
            _ if c.command_type == mandrake_rec::CommandType::Sketch => {}
            _ if c.command_type == mandrake_rec::CommandType::PresetSelection => {}
            _ if c.command_type == mandrake_rec::CommandType::Rename => {
                self.execute_rename(c, player)
            }
            _ if c.command_type == mandrake_rec::CommandType::Detach => {
                self.execute_detach(c, player)
            }
            _ if c.command_type == mandrake_rec::CommandType::Rotation => {
                self.execute_rotation(c, player);
            }
            _ if c.command_type.to_u8() == save_restore::ADD_PLAYER => self.execute_add_player(c),
            _ if c.command_type.to_u8() == save_restore::REPLACE_PLAYER => {
                self.execute_replace_player(c);
            }
            _ if c.command_type.to_u8() == save_restore::SET_TEAM_CAPTAIN => {
                let w = &mut self.engine;
                w.set_team_captain(&c.raw_payload);
            }
            _ if c.command_type.to_u8() == save_restore::SAVE_LOADED => {
                let w = &mut self.engine;
                let observer =
                    save_restore::SaveLoaded::from_bytes(&c.raw_payload).unwrap_or_default();
                self.save_loaded_observer = Some(w.save_loaded(observer));
            }
            _ if c.command_type == mandrake_rec::CommandType::DropPlayer
                || c.command_type.to_u8() == 9 =>
            {
                self.execute_drop_player(i16::from(c.player_index as i8));
            }
            CommandPayload::PickUp(p) => {
                self.lend_objects();
                let r = self.with_lent_movers(|w| {
                    let ids: Vec<usize> = p
                        .unit_ids
                        .iter()
                        .take(0x7e)
                        .filter_map(|&id| w.monster_index_by_identifier(id))
                        .collect();
                    w.execute_pick_up_command(&ids, p.object_index, p.object_identifier, player)
                });
                self.return_objects();
                self.after_engine_command(r);
            }
            _ if c.command_type == mandrake_rec::CommandType::Ally => {
                if let Some((a, b)) = ally::ally_command_teams(&c.raw_payload) {
                    let w = &mut self.engine;
                    let allied = w.teams_are_allied(a, b);
                    w.ally_with_team(a, b, !allied);
                }
            }
            _ => {
                let (site, name) = match c.command_type.to_u8() {
                    17 => (draw_site::COMMAND_INVENTORY, "INVENTORY (sub_502750)"),
                    18 => (
                        draw_site::COMMAND_INVENTORY_DROP,
                        "INVENTORY_DROP (sub_5061e0)",
                    ),
                    19 => (draw_site::COMMAND_TYPE_19, "type 19 (sub_503490)"),
                    20 => (draw_site::COMMAND_TYPE_20, "type 20 (sub_502ed0)"),
                    21 | 23 | 24 | 0x1c.. => return,
                    _ => (draw_site::COMMAND_UNDECODED, "an undecoded"),
                };
                self.engine_stop.get_or_insert(engine_rng::Unmodelled::new(
                    site,
                    format!(
                        "execute_monster_commands: {name} command {}",
                        c.command_type.to_u8()
                    ),
                ));
            }
        }
    }

    fn execute_rename(&mut self, c: &CommandPacket, player: i16) {
        let word = |at: usize| {
            c.raw_payload
                .get(at..at + 2)
                .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
        };
        let count = word(2).max(0) as usize;
        let ids: Vec<u16> = (0..count).map(|i| word(4 + 2 * i) as u16).collect();
        {
            let w = &mut self.engine;
            let max = if w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3 {
                0x7e
            } else {
                0x1e
            };
            let monsters: Vec<usize> = ids
                .iter()
                .take(max)
                .filter_map(|&id| w.monster_index_by_identifier(id))
                .collect();
            w.filter_command_monsters(&monsters, -1, player);
        }
    }

    fn execute_detach(&mut self, c: &CommandPacket, player: i16) {
        let word = |at: usize| {
            c.raw_payload
                .get(at..at + 2)
                .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
        };
        let target = word(2);
        let count = word(4).max(0) as usize;
        let ids: Vec<u16> = (0..count).map(|i| word(6 + 2 * i) as u16).collect();
        let w = &mut self.engine;
        match w.detach_command(&ids, target, player) {
            Ok(monsters) => {
                for m in monsters {
                    let (id, team) = (w.monsters[m].identifier, w.monsters[m].team);
                    if let Some(u) = engine_rng::unit_key(id).and_then(|id| self.units.get_mut(&id))
                    {
                        u.team = team.max(0) as u8;
                    }
                }
            }
            Err(e) => {
                self.engine_stop.get_or_insert(e);
            }
        }
    }

    fn execute_drop_player(&mut self, player: i16) {
        let mut rng_state = std::mem::take(&mut self.rng);
        let mut sites = std::mem::take(&mut self.engine_sites);
        self.lend_objects();
        let r = self.with_lent_movers(|w| {
            let mut rng = engine_rng::EngineRng {
                rng: &mut rng_state,
                sites: &mut sites,
            };
            let r = w.drop_player(player, Some(&mut rng));
            let teams: Vec<(u32, i16)> = r.as_ref().map_or_else(
                |_| Vec::new(),
                |d| {
                    d.detached
                        .iter()
                        .filter_map(|&m| {
                            let m = &w.monsters[m];
                            engine_rng::unit_key(m.identifier).map(|id| (id, m.team))
                        })
                        .collect()
                },
            );
            (r, teams)
        });
        self.return_objects();
        self.rng = rng_state;
        self.engine_sites = sites;
        let (r, teams) = r;
        for (id, team) in teams {
            if let Some(u) = self.units.get_mut(&id) {
                u.team = team.max(0) as u8;
            }
        }
        if let Err(e) = r {
            self.engine_stop.get_or_insert(e);
        }
        self.apply_engine_orders();
    }

    fn execute_replace_player(&mut self, c: &CommandPacket) {
        let Some(command) = save_restore::ReplacePlayer::from_bytes(&c.raw_payload) else {
            return;
        };
        let mut rng_state = std::mem::take(&mut self.rng);
        let mut sites = std::mem::take(&mut self.engine_sites);
        let r = Some({
            let mut rng = engine_rng::EngineRng {
                rng: &mut rng_state,
                sites: &mut sites,
            };
            self.engine.replace_player(&command, &mut rng)
        });
        self.rng = rng_state;
        self.engine_sites = sites;
        if let Some(Err(e)) = r {
            self.engine_stop.get_or_insert(e);
        }
    }

    pub fn end_saved_game_replay(&mut self, observer: save_restore::SaveLoaded) {
        let tick = self.current_tick;
        self.replay = None;
        let w = &mut self.engine;
        w.mode = 2;
        w.replay = None;
        let mut commands = Vec::new();
        if w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_8_4 {
            let mut entries = save_restore::solo_restore_entries(w.players.players.len() as i16);
            commands = w.restore_player_commands(&mut entries, tick);
        }
        commands.push(save_restore::save_loaded_command(tick, observer));
        self.commands_by_tick
            .entry(tick)
            .or_default()
            .extend(commands);
    }

    pub fn save_loaded(&self) -> bool {
        self.engine.game.flags & game_state::FLAG_SAVE_LOADED != 0
    }

    fn execute_rotation(&mut self, c: &CommandPacket, player: i16) {
        let raw = &c.raw_payload;
        let word = |at: usize| {
            raw.get(at..at + 2)
                .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
        };
        let long = |at: usize| {
            raw.get(at..at + 4)
                .map_or(0, |b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        let (flags, angle) = (word(0) as u16, word(2) as u16);
        let centre = (long(4), long(8));
        let count = word(12).max(0) as usize;
        let ids: Vec<i16> = (0..count).map(|i| word(14 + 2 * i)).collect();
        let w = &mut self.engine;
        let max = if w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3 {
            0x7e
        } else {
            0x1e
        };
        let monsters: Vec<usize> = ids
            .iter()
            .take(max)
            .filter_map(|&id| w.monster_index_by_identifier(id as u16))
            .collect();
        let monsters = w.filter_command_monsters(&monsters, -1, player);
        let t = engine_move::trig_tables();
        for m in monsters {
            let mon = &w.monsters[m];
            let mover = mon
                .unit_key()
                .and_then(|id| self.units.get(&id))
                .and_then(|u| u.engine.as_ref());
            let (point, base) = match mover {
                Some(mv) if !mv.waypoints.is_empty() => (
                    mv.waypoints
                        .get(mv.waypoint)
                        .copied()
                        .unwrap_or(mon.position),
                    mv.word_26,
                ),
                _ => (mon.position, mon.facing),
            };
            let facing = angle.wrapping_add(base);
            w.monsters[m].formation.facing = facing;
            let reach = engine_move::distance2d(
                point.0.wrapping_sub(centre.0),
                point.1.wrapping_sub(centre.1),
            );
            let turned = engine_move::arctangent(
                point.0.wrapping_sub(centre.0),
                point.1.wrapping_sub(centre.1),
            )
            .wrapping_add(angle);
            let k = ((i32::from(turned) + 0x20) >> 6 & 0x3ff) as usize;
            let along = |v: i16| (i32::from(v) * reach) / 0x4000;
            w.pending_orders.push(map_actions::MovementOrder {
                monster: m,
                waypoints: vec![(
                    centre.0.wrapping_add(along(t.cosine[k])),
                    centre.1.wrapping_add(along(t.sine[k])),
                )],
                flags,
                facing,
                action: -1,
            });
        }
        self.apply_engine_orders();
    }

    fn execute_unit_adjustment(&mut self, c: &CommandPacket) {
        let time = self.current_tick as i32;
        let w = &mut self.engine;
        let player = c.player_index as i8;
        let version = w.formation_globals.gameplay_version;
        let team = usize::try_from(player)
            .ok()
            .and_then(|p| w.players.players.get(p))
            .map(|p| i16::from(p.team));
        let Some(team) = team.filter(|&t| t >= 0) else {
            self.engine_stop.get_or_insert(engine_rng::Unmodelled::new(
                draw_site::UNIT_ADJUSTMENT_NO_TEAM,
                format!("UNIT_ADJUSTMENT from player {player}, who has no team"),
            ));
            return;
        };
        let Some(record) = w.players.teams.get(team as usize).copied() else {
            return;
        };
        if (version >= 0x641
            && (record.flags & 4 != 0 || w.game_options & game_options::UNIT_TRADING == 0))
            || record.player != i16::from(player)
        {
            return;
        }
        let raw = &c.raw_payload;
        let word = |at: usize| {
            raw.get(at..at + 2)
                .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
        };
        let count = word(2).max(0) as usize;
        let counts: Vec<i16> = (0..count).map(|i| word(4 + 2 * i)).collect();
        let mut rng_state = std::mem::take(&mut self.rng);
        let mut sites = std::mem::take(&mut self.engine_sites);
        self.lend_movers();
        let r = Some({
            let mut rng = engine_rng::EngineRng {
                rng: &mut rng_state,
                sites: &mut sites,
            };
            let r = self
                .engine
                .adjust_unit_placement(team, &counts, &mut rng, time);
            self.engine.players.teams[team as usize].flags |= 4;
            r
        });
        self.rng = rng_state;
        self.engine_sites = sites;
        let deleted = match r {
            Some(Ok(deleted)) => deleted,
            Some(Err(e)) => {
                self.engine_stop.get_or_insert(e);
                Vec::new()
            }
            None => Vec::new(),
        };
        for id in deleted {
            if let Some(id) = engine_rng::unit_key(id) {
                self.units.remove(&id);
            }
        }
        self.return_movers();
    }

    fn execute_add_player(&mut self, c: &CommandPacket) {
        let raw = &c.raw_payload;
        if raw.len() < 0x60 {
            return;
        }
        let w = &mut self.engine;
        let gameplay = w.formation_globals.gameplay_version;
        let id = i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
        let team = i16::from_be_bytes([raw[4], raw[5]]);
        let _build_id = u16::from_be_bytes([raw[6], raw[7]]);
        let _follow = i32::from_be_bytes([raw[8], raw[9], raw[10], raw[11]]);

        let (team_is_none, captain_index) = if team < 0 {
            if team != -1 || gameplay <= game_options::GAMEPLAY_1_8_4 {
                return;
            }
            (true, None)
        } else {
            let t = team as usize;
            if t >= w.players.teams.len() {
                return;
            }
            w.players.teams[t].players = w.players.teams[t].players.saturating_add(1);
            let captain = w.players.teams[t].player;
            let captain_idx = if captain >= 0 {
                Some(captain as usize)
            } else {
                None
            };
            (false, captain_idx)
        };

        let mut match_index = None;
        if gameplay > game_options::GAMEPLAY_1_8_4 {
            for (i, p) in w.players.players.iter().enumerate() {
                if p.field_09 == 0 && (p.flags & 0x04) != 0 {
                    let is_obs_team = team == -1;
                    let is_obs_player = (p.flags & 0x40) != 0;
                    if is_obs_team == is_obs_player && p.id == id {
                        match_index = Some(i);
                        break;
                    }
                }
            }
        }

        let color = |o: usize| -> players::PlayerColor {
            let s = &raw[80 + o..80 + o + 8];
            [
                u16::from_be_bytes([s[0], s[1]]),
                u16::from_be_bytes([s[2], s[3]]),
                u16::from_be_bytes([s[4], s[5]]),
                u16::from_be_bytes([s[6], s[7]]),
            ]
        };
        let mut player_colors: [players::PlayerColor; 2] = [color(0), color(8)];

        if let Some(c_idx) = captain_index
            && let Some(&capt_colors) = w.players.colors.get(c_idx)
        {
            player_colors[1] = player_colors[0];
            player_colors[0] = capt_colors[0];
        }

        let flags = if team_is_none { 0x41 } else { 0x01 };

        let (player_index, is_new) = if let Some(i) = match_index {
            let p = &mut w.players.players[i];
            p.flags = flags;
            p.team = team as i8;
            p.field_0b = team as i8;
            p.id = id;
            if i < w.players.colors.len() {
                w.players.colors[i] = player_colors;
            }
            (i, false)
        } else {
            let i = w.players.players.len();
            w.players.players.push(players::Player {
                flags,
                field_09: 0,
                team: team as i8,
                field_0b: team as i8,
                id,
            });
            w.players.colors.push(player_colors);
            (i, true)
        };

        if team >= 0 {
            w.players.occupied_teams =
                w.players.teams.iter().filter(|t| t.players != 0).count() as i32;
        }

        if is_new {
            if w.statistics.player_rankings.len() <= player_index {
                w.statistics
                    .player_rankings
                    .push(statistics::PlayerRanking {
                        player: player_index as i16,
                        parent: -1,
                        ..Default::default()
                    });
            }
            if w.statistics.permanent_rankings.len() <= player_index {
                w.statistics
                    .permanent_rankings
                    .push(statistics::PermanentRanking {
                        player: player_index as i16,
                        parent: -1,
                        ..Default::default()
                    });
            }
        }
    }

    fn with_lent_movers<T>(&mut self, f: impl FnOnce(&mut engine_rng::EngineWorld) -> T) -> T {
        if self.engine_stop.is_some() {
            self.sync_engine_positions();
        }
        self.lend_movers();
        let r = f(&mut self.engine);
        self.return_movers();
        r
    }

    fn after_engine_command(&mut self, r: Result<(), engine_rng::Unmodelled>) {
        if let Err(e) = r {
            self.engine_stop.get_or_insert(e);
        }
        self.apply_engine_orders();
    }

    fn execute_general(&mut self, unit_ids: &[u32], kind: u16, player: i16) {
        let orders = self.engine.pending_orders.len();

        let time = self.current_tick as i32;
        let mut rng_state = std::mem::take(&mut self.rng);
        let mut sites = std::mem::take(&mut self.engine_sites);
        let r = self.with_lent_movers(|w| {
            let monsters: Vec<usize> = unit_ids
                .iter()
                .take(0x7e)
                .filter_map(|&id| w.monster_index_by_identifier(id as u16))
                .collect();
            let mut rng = engine_rng::EngineRng {
                rng: &mut rng_state,
                sites: &mut sites,
            };
            w.execute_general_command(&monsters, kind, -1, player, &mut rng, time)
                .map(|()| {
                    w.pending_orders[orders..]
                        .iter()
                        .filter_map(|o| w.monsters[o.monster].unit_key())
                        .collect::<Vec<u32>>()
                })
        });
        self.rng = rng_state;
        self.engine_sites = sites;
        match r {
            Ok(ordered) => {
                self.apply_engine_orders();
                self.mark_moving(&ordered);
            }
            Err(e) => {
                self.engine_stop.get_or_insert(e);
                self.apply_engine_orders();
            }
        }
    }

    fn mark_moving(&mut self, ids: &[u32]) {
        for id in ids {
            if let Some(u) = self.units.get_mut(id)
                && u.engine.as_ref().is_some_and(|m| {
                    m.is_moving(self.engine.monster_flags_by_identifier(*id as u16))
                })
            {
                u.state = UnitState::Moving;
            }
        }
    }

    fn execute_movement(
        &mut self,
        unit_ids: &[u32],
        points: &[(i32, i32)],
        header: engine_rng::MovementHeader,
        player: i16,
        centre: Option<(i32, i32)>,
    ) {
        let r = self.with_lent_movers(|w| {
            if let Some(centre) = centre {
                let list: Vec<Option<usize>> = unit_ids
                    .iter()
                    .take(0x7e)
                    .map(|&id| w.monster_index_by_identifier(id as u16))
                    .collect();
                return w
                    .execute_matched_movement_command(&list, points, header, centre, -1, player);
            }
            let monsters: Vec<usize> = unit_ids
                .iter()
                .take(0x7e)
                .filter_map(|&id| w.monster_index_by_identifier(id as u16))
                .collect();
            w.execute_movement_command(&monsters, points, header, -1, player)
        });
        match r {
            Ok(()) => {
                self.apply_engine_orders();
                self.mark_moving(unit_ids);
            }
            Err(e) => {
                self.engine_stop.get_or_insert(e);
                self.apply_engine_orders();
            }
        }
    }

    pub(crate) fn sniff_local_command(&mut self, command: &map_actions::SniffedCommand) -> bool {
        if self.engine_stop.is_some() {
            return true;
        }
        let time = self.current_tick as i32;
        let mut rng = engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        let w = &mut self.engine;
        let Some(mut actions) = w.map_actions.take() else {
            return true;
        };
        let r = actions.handle_command_sniffing(
            command,
            &mut map_actions::MapActionContext {
                world: w,
                rng: &mut rng,
                time,
            },
        );
        w.map_actions = Some(actions);
        match r {
            Ok(go) => go,
            Err(e) => {
                self.engine_stop = Some(e);
                true
            }
        }
    }

    pub fn local_input_action(&mut self, action: &map_actions::InputAction) -> bool {
        if self.engine_stop.is_some() {
            return true;
        }
        let time = self.current_tick as i32;
        let mut rng = engine_rng::EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        let w = &mut self.engine;
        let Some(mut actions) = w.map_actions.take() else {
            return true;
        };
        let r = actions.handle_action_sniffing(
            action,
            &mut map_actions::MapActionContext {
                world: w,
                rng: &mut rng,
                time,
            },
        );
        w.map_actions = Some(actions);
        match r {
            Ok(go) => go,
            Err(e) => {
                self.engine_stop = Some(e);
                true
            }
        }
    }

    pub fn apply_engine_unit_objects(&mut self, monsters: &[engine_rng::EngineMonster]) -> usize {
        let mut matched = 0;
        for m in monsters {
            let Some(unit) = m.unit_key().and_then(|id| self.units.get_mut(&id)) else {
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
            matched += 1;
        }
        matched
    }

    pub fn command_point_to_world(&self, x: i32, y: i32) -> Vec3Fixed {
        let (half_w, half_d, cell_size) = self.client_frame();
        let col = Fixed16::from_raw(x.wrapping_mul(65536 / mandrake_rec::WORLD_UNITS_PER_CELL));
        let row = Fixed16::from_raw(y.wrapping_mul(65536 / mandrake_rec::WORLD_UNITS_PER_CELL));
        Vec3Fixed::new(
            half_w - col * cell_size,
            Fixed16::ZERO,
            row * cell_size - half_d,
        )
    }

    pub fn world_to_command_point(&self, pos: Vec3Fixed) -> (i32, i32) {
        let (half_w, half_d, cell_size) = self.client_frame();
        let col = (half_w - pos.x) / cell_size;
        let row = (pos.z + half_d) / cell_size;
        let shift = (65536 / mandrake_rec::WORLD_UNITS_PER_CELL).trailing_zeros();
        (col.to_raw() >> shift, row.to_raw() >> shift)
    }
}
