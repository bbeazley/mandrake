use super::*;

pub(crate) struct WorldHooks<'a, 'r> {
    pub(crate) world: &'a mut EngineWorld,
    pub(crate) actions: &'a mut Option<MapActions>,
    pub(crate) rng: &'a mut EngineRng<'r>,
    pub(crate) monster: usize,
    pub(crate) time: i32,
    pub(crate) host: &'a mut dyn crate::engine_attack::CombatHost,
}

impl WorldHooks<'_, '_> {
    pub(super) fn general_attack(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        self.world.monsters[i].flags |= 0x80_0000;
        let own = crate::attack_ai::Own::of(mv);
        let a = self.world.choose_attack(i, 3, &own)?;
        if a != -1 {
            self.attack(mv, a, 0)?;
            self.world.monsters[i].flags |= 0x100_0000;
        }
        self.world.monsters[i].flags &= !0x80_0000;
        Ok(())
    }
}

impl ModeHooks for WorldHooks<'_, '_> {
    fn flags(&mut self) -> crate::engine_move::RecordFlags<'_> {
        let m = &mut self.world.monsters[self.monster];
        crate::engine_move::RecordFlags {
            flags: &mut m.flags,
            flags_8: &mut m.flags_8,
        }
    }

    fn change_mode(
        &mut self,
        mover: &mut EngineMover,
        mode: MoveMode,
        state: ModeState,
    ) -> Result<Option<(i8, i8)>, Unmodelled> {
        let i = self.monster;
        if mode != MoveMode::Walking {
            self.world.monsters[i].object.flags &= !crate::collide::OBJECT_POSED;
        }
        if mode != MoveMode::Held {
            self.world.monsters[i].flags_8 &= !0x0100_0000;
        }
        if mode == MoveMode::Held {
            self.world.monsters[i].flags &= !0x0400_0000;
            let def_idx = self.world.monsters[i].definition;
            let held_vitality_multiplier =
                self.world.monster_definitions[def_idx].held_vitality_multiplier;
            let field_199 = self.world.monster_definitions[def_idx].field_199;
            if held_vitality_multiplier == 0 {
                self.world.monsters[i].flags |= 0x0400_0000;
            }
            self.internal_group(mover, crate::engine_damage::GROUP_HELD_START)?;
            if field_199 & 4 == 0 {
                mover.pose_timer = 75;
            }
        }
        if mode == MoveMode::Paralyzed {
            self.internal_group(mover, crate::engine_damage::GROUP_PARALYSIS_EDGE)?;
            mover.pose_timer = 0x1e;
        }
        if mode == MoveMode::Stunned {
            mover.pose_timer = 0x636;
        }
        if mode == MoveMode::LowVitality {
            if self.world.monsters[i].flags_8 & 0x200_0000 == 0 {
                self.internal_group(mover, crate::engine_damage::GROUP_LOW_VITALITY_START)?;
            }
            mover.pose_timer = 0x2d;
        }
        let w = &mut *self.world;
        let m = &w.monsters[i];
        let def = &w.monster_definitions[m.definition];
        let solid_outside_walking = def.flags & 0x4000 != 0;
        let value = |st: ModeState, m: &EngineMonster| match st {
            ModeState::Keep => m.anim_state,
            ModeState::Zero => 0,
            ModeState::Value(v) => v as u8,
        };
        let walking = |st: u8| match st {
            0x13 if def.sequence_22 != -1 => def.sequence_22,
            0x14 if def.sequence_2a != -1 => def.sequence_2a,
            _ => def.walk_sequence,
        };
        let mut repetitions = None;
        let mut flags_8_or = 0u32;
        let (sequence, speed, new_state, mode_byte, shape_flags) = match mode {
            MoveMode::Idle => {
                let r = self.rng.draw(draw_site::CHANGE_MODE_ROLL);
                (def.idle_sequence, 0x300 + (i32::from(r) >> 9), 0x1a, 1, 3)
            }
            MoveMode::Walking => {
                let st = value(state, m);
                (walking(st), 0x100, st, 4, 3)
            }
            MoveMode::Turning => (def.turn_sequence, 0x100, 0, 5, 3),
            MoveMode::Celebrating => {
                let r = self.rng.draw(draw_site::CHANGE_MODE_SECOND_ROLL);
                let st = value(state, m);
                (def.ai.word(0x1a), 0x100 + (i32::from(r) >> 8), st, 2, 0)
            }
            MoveMode::Taunting => {
                let r = self.rng.draw(draw_site::CHANGE_MODE_THIRD_ROLL);
                let st = value(state, m);
                (def.ai.word(0x28), 0x100 + (i32::from(r) >> 8), st, 3, 0)
            }
            MoveMode::Holding => (def.ai.engage_sequence, 0x100, 0, 6, 3),
            MoveMode::Aiming => {
                let st = value(state, m);
                (walking(st), 0x100, st, 7, 3)
            }
            MoveMode::Attacking => {
                let st = value(state, m);
                let a = st as i8;
                let Some(atk) = w.attack_definition(i, i16::from(a))? else {
                    return Err(Unmodelled::new(
                        draw_site::MODE_8_WITHOUT_ATTACKS,
                        format!("mons {}: mode 8 without attacks", def.tag),
                    ));
                };
                let special = w.combat(i).special;
                let any_monster = a == -0x45
                    && special != -1
                    && w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5
                    && be_i32(&w.catalog.artifacts[special as usize].1, 0x04) == -1;
                let rep = atk.repetitions;
                let atk = if any_monster {
                    def.ai.attack(0).cloned().unwrap_or(atk)
                } else {
                    atk
                };
                let e = u32::from(w.combat(i).elevation);
                let pick = |mask: u16| -> Vec<i16> {
                    atk.sequences
                        .iter()
                        .filter(|(f, s)| *s != -1 && f & mask != 0)
                        .map(|(_, s)| *s)
                        .collect()
                };
                let mut list = if e.wrapping_sub(0x1000) > 0xebff {
                    Vec::new()
                } else {
                    pick(if e <= 0x7fff { 4 } else { 2 })
                };
                if list.is_empty() {
                    list = pick(1);
                }
                if list.is_empty() {
                    list.push(0);
                }
                let r = i32::from(self.rng.draw(draw_site::CHANGE_MODE_ATTACK));
                repetitions = Some(rep);
                (list[(r % list.len() as i32) as usize], 0x100, st, 8, 0)
            }
            MoveMode::Blocking => (def.ai.block_sequence, 0x100, value(state, m), 9, 0),
            MoveMode::PickingUp => (def.ai.word(0x16), 0x100, value(state, m), 10, 0),
            MoveMode::LowVitality => (i16::from(m.object.sequence), 0x100, value(state, m), 13, 0),
            MoveMode::Paralyzed => (def.ai.hit_sequence, 0x100, value(state, m), 14, 0),
            MoveMode::Held => {
                let seq = if m.mode == 13 {
                    flags_8_or = 0x0100_0000;
                    m.object.sequence as i16
                } else {
                    def.ai.hit_sequence
                };
                let st = value(state, m);
                (seq, 0x100, st, 11, 0)
            }
            MoveMode::Stunned => {
                let seq = if m.mode == 13
                    && w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1
                {
                    i16::from(m.object.sequence)
                } else {
                    def.ai.hit_sequence
                };
                (seq, 0x100, value(state, m), 12, 0)
            }
            MoveMode::Flinching => (def.ai.hit_sequence, 0x100, value(state, m), 15, 0),
        };
        let collection = m.object.collection;
        let idle_sequence = def.idle_sequence;
        let one_shot = m.flags & 0x0100_0000 != 0;
        let m = &mut w.monsters[i];
        m.flags_8 |= flags_8_or;
        m.flags &= !0x20000;
        if mode == MoveMode::Attacking {
            m.flags &= 0xfffd_ff5f;
            m.flags_8 &= !0x10_0000;
        }
        if mode == MoveMode::Celebrating {
            m.flags |= 0x200_0000;
            m.flags_8 &= !0x10_0000;
        }
        if mode == MoveMode::Taunting {
            m.flags_8 &= !0x10_0000;
        }
        if mode == MoveMode::Idle {
            m.flags_8 &= !0x10_0000;
        }
        let drops_order = matches!(
            mode,
            MoveMode::Idle | MoveMode::Celebrating | MoveMode::Taunting
        );
        if drops_order || one_shot {
            if drops_order {
                m.flags &= !0x200a0;
            }
            m.flags = (m.flags & 0xde7e_ffe0) | 0x8000;
            m.flags_8 &= 0xf7ff_7ddf;
            let c = w.combat_mut(i);
            c.target = -1;
            c.aim = (0, 0);
        }
        let EngineWorld {
            monsters,
            collections,
            ..
        } = &mut *w;
        let m = &mut monsters[i];
        if let Some(r) = repetitions {
            mover.bump_count = r;
        }
        m.mode = mode_byte;
        m.anim_state = new_state;
        m.object.velocity = [0; 3];
        let sequence = if sequence == -1 {
            idle_sequence
        } else {
            sequence
        };
        let mut clock = None;
        if sequence != -1 {
            let headers = collections
                .get(&collection)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let changed = i32::from(sequence) != i32::from(m.object.sequence);
            set_object_shape(
                &mut m.object,
                headers,
                i32::from(sequence),
                speed,
                shape_flags,
                self.rng,
            )?;
            clock = changed.then_some((m.object.tick, m.object.ticks_per_frame));
        }
        if solid_outside_walking {
            if matches!(mode, MoveMode::Walking | MoveMode::Turning) {
                m.object.flags &= !0x400;
            } else {
                m.object.flags |= 0x400;
            }
        }
        if mode == MoveMode::Paralyzed {
            self.fade_parasite(2, 0x1e)?;
        }
        if mode == MoveMode::Held && self.world.monsters[i].object.shading_slot != 0 {
            self.world.monsters[i].flags |= 0x400_0000;
            self.fade_parasite(0, 0x4b)?;
        }
        if mode == MoveMode::LowVitality {
            self.world.monsters[i].object.ticks_per_frame = 0;
            self.fade_parasite(1, 0x2d)?;
        }
        Ok(clock)
    }

    fn engaged(&mut self, mover: &mut EngineMover) -> Result<bool, Unmodelled> {
        let i = self.monster;
        let gone = {
            let w = &*self.world;
            let m = &w.monsters[i];
            m.flags & 9 == 9 && {
                let def = &w.monster_definitions[m.definition].ai;
                let t = usize::try_from(w.combat(i).target).unwrap_or(i);
                let tm = &w.monsters[t];
                tm.flags & 0x4000_0000 != 0
                    || (def.byte_1b5 & 4 == 0
                        && def.class == 0
                        && w.monster_definitions[tm.definition].flags & 8 != 0)
            }
        };
        if gone {
            self.world.monsters[i].flags &= 0xffff_ffe6;
            mover.set_mode(MoveMode::Idle, ModeState::Zero, self)?;
        }
        let w = &mut *self.world;
        let m = &w.monsters[i];
        Ok(matches!(m.mode, 4 | 6)
            && m.flags & 9 != 0
            && mover.pose_timer <= 0
            && m.flags & 0x8_0000 == 0
            && self.recheck_hold(mover)?)
    }

    fn finish(&mut self, mover: &mut EngineMover) -> Result<bool, Unmodelled> {
        let i = self.monster;
        let w = &mut *self.world;
        let m = &w.monsters[i];
        if m.flags & 0x100_0000 != 0 {
            let version = w.formation_globals.gameplay_version;
            let has_waypoints = !mover.waypoints.is_empty();
            let f8 = m.flags_8;
            if f8 & 0x4_0000 != 0 && has_waypoints && version > game_options::GAMEPLAY_1_3 {
                let m = &mut w.monsters[i];
                m.flags = (m.flags & 0x7e5f_ffe2) | 0x1022;
                m.flags_8 &= 0xffdb_ffff;
            } else if f8 & 0x4_0000 != 0 || version < 0x641 || !has_waypoints {
                if f8 & 0x400_0000 != 0 {
                    let mode = m.mode;
                    let state = walking_state(mode, f8);
                    let m = &mut w.monsters[i];
                    m.flags_8 &= 0xfbfb_ffff;
                    m.flags = (m.flags & !0x100_0000) | 0x80_0020;
                    mover.set_mode(MoveMode::Walking, state, self)?;
                    return Ok(true);
                }
                w.drop_waypoint_count(i)?;
                mover.waypoints.clear();
                let m = &mut w.monsters[i];
                m.flags &= 0xfe7f_fff6;
                m.flags_8 &= 0xfbfb_ffff;
                mover.set_mode(MoveMode::Idle, ModeState::Zero, self)?;
                return Ok(true);
            }
        }
        let m = &w.monsters[i];
        if m.team == w.game.winner
            && w.monster_definitions[m.definition].ai.class != 5
            && m.flags & 0x200_0000 == 0
        {
            mover.set_mode(MoveMode::Celebrating, ModeState::Value(0), self)?;
            return Ok(true);
        }
        if m.flags & 0x4000 != 0 {
            let state = walking_state(m.mode, m.flags_8);
            mover.set_mode(MoveMode::Walking, state, self)?;
            return Ok(true);
        }
        let (byte_198, byte_199) = {
            let d = &w.monster_definitions[m.definition].ai;
            (d.byte_198, d.byte_199)
        };
        let mode = m.mode;
        let new_leg = |w: &mut EngineWorld| {
            let m = &mut w.monsters[i];
            m.flags = (m.flags & !0x80) | 0x20;
        };
        enum Then {
            Walk,
            Hold,
            Idle,
        }
        let then = if m.flags & 1 != 0 {
            let check = if m.flags & 0x10_0000 == 0 {
                if byte_199 & 8 == 0 {
                    Some(Then::Hold)
                } else {
                    None
                }
            } else {
                if !matches!(mode, 4 | 5) {
                    new_leg(w);
                }
                if byte_199 & 8 == 0 {
                    Some(Then::Walk)
                } else {
                    None
                }
            };
            match check {
                Some(t) => t,
                None => {
                    let t = w.combat(i).target;
                    let hold = t != -1
                        && (w.monsters[i].flags & 0x2000_0000 != 0
                            || w.monsters_are_allied(i, t as usize));
                    if hold {
                        Then::Hold
                    } else {
                        new_leg(w);
                        Then::Walk
                    }
                }
            }
        } else if mover.waypoint < mover.waypoints.len() {
            if !matches!(mode, 4 | 5) {
                new_leg(w);
            }
            Then::Walk
        } else {
            let own = crate::attack_ai::Own::of(mover);
            let t = w.find_target(i, 3, &own)?;
            if t != -1 && w.engage_monster(i, t as usize, 2)? {
                if byte_198 & 1 != 0 {
                    w.monsters[i].flags &= !0x80;
                    Then::Idle
                } else {
                    new_leg(w);
                    Then::Walk
                }
            } else {
                Then::Idle
            }
        };
        match then {
            Then::Walk => {
                let state = walking_state(mode, self.world.monsters[i].flags_8);
                mover.set_mode(MoveMode::Walking, state, self)?;
            }
            Then::Hold => mover.set_mode(MoveMode::Holding, ModeState::Zero, self)?,
            Then::Idle => mover.set_mode(MoveMode::Idle, ModeState::Zero, self)?,
        }
        Ok(true)
    }

    fn aimed(&mut self, mover: &mut EngineMover) -> Result<(), Unmodelled> {
        self.aim_done(mover)
    }

    fn walking_becomes(&self) -> Option<MoveMode> {
        let flags = self.world.monsters[self.monster].flags;
        ((flags as i32) < 0).then_some(if flags & 8 == 0 {
            MoveMode::Idle
        } else {
            MoveMode::Holding
        })
    }

    fn posed(&self) -> bool {
        self.world.monsters[self.monster].object.flags & crate::collide::OBJECT_POSED != 0
    }

    fn running(&self) -> bool {
        self.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3
            && self.world.monsters[self.monster].flags_8 & 0x400 != 0
    }

    fn set_state(&mut self, state: u8) {
        self.world.monsters[self.monster].anim_state = state;
    }

    fn leg_gait(&mut self, mover: &mut EngineMover) -> Result<(), Unmodelled> {
        self.common_gait(mover)
    }

    fn step(
        &mut self,
        mover: &mut EngineMover,
        terrain: Option<&MeshCells>,
    ) -> Result<u32, Unmodelled> {
        self.world.step_monster(self.monster, mover, terrain)
    }

    fn react(&mut self, mover: &mut EngineMover, r: u32) -> Result<(), Unmodelled> {
        let i = self.monster;
        let (host, actions, time) = (&mut *self.host, &mut *self.actions, self.time);
        let mut bump_flag = |w: &mut EngineWorld,
                             mv: &mut EngineMover,
                             rng: &mut EngineRng<'_>,
                             object: i16,
                             magnitude: i16| {
            host.monster_bumped_flag(w, rng, actions, time, (i, mv), object, magnitude)
        };
        if self
            .world
            .react_monster_with(i, mover, r, self.rng, &mut bump_flag)?
        {
            self.world.monsters[i].anim_state = 0;
            let def = &self.world.monster_definitions[self.world.monsters[i].definition];
            if def.ai.class != 1 {
                mover.set_mode(MoveMode::Holding, ModeState::Value(0), self)?;
            }
            self.world.react_after_engage(i, mover, self.rng)?;
        }
        if r & 0x20 != 0 {
            self.off_map_edge()?;
        }
        Ok(())
    }

    fn plan_leg(&mut self, mover: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        self.world.monsters[i].flags &= 0xfffb_ffff;
        let w = &*self.world;
        let (m, me) = w.path_views(i, mover)?;
        let (trace, detour) = mover.plan_with(&w.cells, w, m, me)?;
        detour.write(self.flags());
        let time = self.time;
        if let Some(log) = self.world.path_log.as_mut() {
            log.push(PathCall {
                time,
                monster: i,
                site: draw_site::PLAN_LEG,
                position: (mover.x, mover.y),
                path: mover.path,
                target: mover.target,
                detour: detour.detour,
                detour_side: detour.side,
                avoid: mover.avoid,
                obstacles: trace.obstacles,
            });
        }
        Ok(())
    }

    fn under_order(&self) -> bool {
        self.world.monsters[self.monster].flags & 2 != 0
    }

    fn end_order(&mut self) {
        self.world.monsters[self.monster].flags &= 0xffff_effd;
    }

    fn order_done(&mut self, action: i16) -> Result<(), Unmodelled> {
        let Some(actions) = self.actions.as_mut() else {
            return Ok(());
        };
        let Some(j) = actions.index_of(i32::from(action)) else {
            return Ok(());
        };
        actions.succeed(
            j,
            &mut MapActionContext {
                world: self.world,
                rng: self.rng,
                time: self.time,
            },
        )
    }
}

impl EngineWorld {
    pub(crate) fn runs_at_distance(&self, i: usize, d: i32) -> bool {
        let m = &self.monsters[i];
        let def = &self.monster_definitions[m.definition].ai;
        let k = i32::from(def.raw.get(0x52).copied().unwrap_or(0) as i8);
        if k << 9 <= d {
            return false;
        }
        let version = self.formation_globals.gameplay_version;
        let always = if version > 0x6ad {
            def.flags & 0x8000_0000 == 0
        } else {
            version <= game_options::GAMEPLAY_1_6 || def.flags & 0x2000_0000 != 0
        };
        always || m.flags & 8 != 0 || k << 7 > d
    }

    pub(crate) fn distance_gait(&mut self, i: usize, d: i32) -> bool {
        let def = &self.monster_definitions[self.monsters[i].definition].ai;
        if def.byte_198 & 4 == 0 {
            return false;
        }
        if self.runs_at_distance(i, d) {
            self.monsters[i].flags_8 |= 0x400;
        } else if !matches!(self.monsters[i].mode, 15 | 16) {
            self.monsters[i].flags_8 &= !0x400;
        }
        true
    }
}

impl WorldHooks<'_, '_> {
    pub(crate) fn common_gait(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let bit4 = self.world.monster_definitions[self.world.monsters[i].definition]
            .ai
            .byte_198
            & 4
            != 0;
        if bit4 {
            let own = crate::attack_ai::Own::of(mv);
            let d = self.world.target_distance(i, &own)?;
            self.world.distance_gait(i, d);
        }
        let w = &mut *self.world;
        let version = w.formation_globals.gameplay_version;
        let m = &w.monsters[i];
        if version > game_options::GAMEPLAY_1_3 && m.flags_8 & 0x400 != 0 && m.mode == 4 {
            w.monsters[i].anim_state = 0x13;
            mv.animation_state = 0x13;
            return Ok(());
        }
        if bit4 && m.mode == 4 && m.anim_state == 0x13 {
            let flags = w.monster_definitions[m.definition].ai.flags;
            let reset = if version > 0x6ad {
                flags & 0x8000_0000 != 0
            } else {
                version > game_options::GAMEPLAY_1_6 && flags & 0x2000_0000 == 0
            };
            if reset {
                w.monsters[i].anim_state = 0;
                mv.animation_state = 0;
            }
        }
        Ok(())
    }

    pub(super) fn taunt_handler(&mut self, mover: &mut EngineMover) -> Result<(), Unmodelled> {
        let m = &mut self.world.monsters[self.monster];
        if m.object.anim_flags & 4 == 0 {
            return Ok(());
        }
        m.anim_state = m.anim_state.wrapping_sub(1);
        if m.anim_state as i8 <= 0 {
            self.finish(mover)?;
        }
        Ok(())
    }

    pub(super) fn celebrate_handler(&mut self, mover: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let m = &mut self.world.monsters[i];
        if m.object.anim_flags & 4 == 0 {
            return Ok(());
        }
        m.anim_state = m.anim_state.wrapping_sub(1);
        if m.anim_state as i8 <= 0 {
            self.finish(mover)?;
            return Ok(());
        }
        let r = self.rng.draw(draw_site::CELEBRATE_HANDLER);
        let collection = m.object.collection;
        let sequence = i32::from(m.object.sequence);
        let EngineWorld {
            monsters,
            collections,
            ..
        } = &mut *self.world;
        let headers = collections
            .get(&collection)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        set_object_shape(
            &mut monsters[i].object,
            headers,
            sequence,
            0x100 + (i32::from(r) >> 8),
            0x30,
            self.rng,
        )
    }

    pub(super) fn update_confusion(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let mode = self.world.monsters[i].mode;
        let gameplay = self.world.formation_globals.gameplay_version;
        let c = self.world.combat_mut(i);
        c.confusion = c.confusion.wrapping_sub(1);
        if c.confusion < 1 {
            let m = &mut self.world.monsters[i];
            if gameplay <= game_options::GAMEPLAY_1_5_1 {
                m.flags &= !0x1000_0000;
            }
            m.flags_8 &= !0x108;
            if !(11..=14).contains(&mode) || gameplay <= game_options::GAMEPLAY_1_5 {
                self.finish_action(mv)?;
            }
            self.world.turn_off_monster_local_group(i);
            return Ok(());
        }
        if mode != 1 {
            return Ok(());
        }
        let m = &mut self.world.monsters[i];
        if m.flags & 8 != 0 {
            m.flags = (m.flags & 0xde7e_ffe0) | 0x8000;
            m.flags_8 &= 0xf7ff_7ddf;
            self.world.combat_mut(i).target = -1;
        }
        let mut walk = true;
        if self.rng.draw(draw_site::CONFUSION_CHANCE) & 1 != 0 {
            let def = &self.world.monster_definitions[self.world.monsters[i].definition].ai;
            let n = def.word(0x88);
            if n > 0 {
                for k in 0..4usize {
                    let ground = def
                        .attacks
                        .get(k)
                        .and_then(|a| a.projectile)
                        .and_then(|p| self.world.catalog.projectiles.get(&p))
                        .is_some_and(|p| p.flags & 0x2000 != 0);
                    if ground {
                        walk = false;
                        break;
                    }
                    if k < 3 && i32::from(n) == k as i32 + 1 {
                        break;
                    }
                }
            }
        }
        let r = self.rng.draw(draw_site::CONFUSION_TIMER);
        let t = crate::engine_move::trig_tables();
        let a = ((i32::from(r) + 0x20) >> 6) as usize & 0x3ff;
        let x = ((i32::from(t.cosine[a]) << 9) >> 14).wrapping_add(mv.x);
        let y = ((i32::from(t.sine[a]) << 9) >> 14).wrapping_add(mv.y);
        let z = self
            .world
            .mesh_height(x, y, mandrake_tag::MeshHeightMode::Physical) as i16;
        if walk {
            let facing = self.rng.draw(draw_site::CONFUSION_FACING);
            let action = mv.action;
            mv.order_with(
                &[(x, y)],
                0,
                facing,
                action,
                self.world.monsters[i].record_flags(),
            );
            self.world.set_monster_waypoints_flags(i, &[(x, y)], 0)?;
        } else {
            self.world.monsters[i].flags_8 &= !0x4_0000;
            mv.waypoints.clear();
            mv.waypoint = 0;
            self.world.combat_mut(i).pending = -1;
            self.world.set_target_location(i, 0, (x, y, i32::from(z)));
        }
        Ok(())
    }

    pub(crate) fn off_map_edge(&mut self) -> Result<(), Unmodelled> {
        let i = self.monster;
        let gameplay = self.world.formation_globals.gameplay_version;
        let m = &mut self.world.monsters[i];
        m.object.no_shape = true;
        m.flags |= 0x4000_0000;
        if gameplay > game_options::GAMEPLAY_1_7 {
            m.flags &= !0x19;
            let c = self.world.combat_mut(i);
            c.target = -1;
            c.pending = -1;
            c.attacker = -1;
            c.held_unit = -1;
        }
        self.call_monster_died_procs(2)
    }

    pub(super) fn pick_up_leg_start(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let id = self.world.monsters[i].identifier;
        self.world.monsters[i].flags_8 &= !0x4_0000;
        let object = self.world.combat(i).pick_up;
        if self
            .world
            .monster_could_pick_up_object(i, object.0, object.1)?
            == 0
        {
            self.world.monsters[i].flags &= !0x4000;
            self.finish_action(mv)?;
            let flags = self.world.monsters[i].flags;
            self.world.monsters[i].flags |= crate::engine_move::NEW_LEG;
            return if flags & 1 != 0 {
                self.engaged_leg_start(mv)
            } else {
                mv.start_leg_if_needed(self)
            };
        }
        let Some((x, y)) = self.world.pick_up_point(object.0, object.1) else {
            return Err(Unmodelled::new(
                draw_site::PICK_UP_LEG_NO_POINT,
                format!("monster {id}: a pick-up's object without a point"),
            ));
        };
        self.common_gait(mv)?;
        self.world.monsters[i].flags &= !0x20;
        if mv.leg_to((x, y), self)? {
            return Ok(());
        }
        self.pick_up_arrived(mv, (x, y))
    }

    fn pick_up_arrived(
        &mut self,
        mv: &mut EngineMover,
        point: (i32, i32),
    ) -> Result<(), Unmodelled> {
        let i = self.monster;
        self.world.monsters[i].flags &= 0xfffd_bfff;
        let object = self.world.combat(i).pick_up;
        let could = self
            .world
            .monster_could_pick_up_object(i, object.0, object.1)?;
        let near = crate::engine_move::distance2d(point.0 - mv.x, point.1 - mv.y) < 0x101;
        if could != 0 && near {
            return mv.set_mode(MoveMode::PickingUp, ModeState::Zero, self);
        }
        self.finish_action(mv)
    }

    pub(crate) fn pick_up_handler(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        if self.world.monsters[i].object.anim_flags & 2 != 0 {
            let object = self.world.combat(i).pick_up;
            if let Some(point) = self.world.pick_up_point(object.0, object.1)
                && crate::engine_move::distance2d(point.0 - mv.x, point.1 - mv.y) < 0x101
                && self.world.give_object_at(i, object.0, object.1, self.rng)? != 0
                && self.world.monsters[i].flags & 2 != 0
                && mv.action != -1
            {
                self.order_done(mv.action)?;
            }
        }
        if self.world.monsters[i].object.anim_flags & 4 != 0 {
            self.finish_action(mv)?;
        }
        Ok(())
    }

    pub(super) fn engaged_leg_start(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let own = crate::attack_ai::Own::of(mv);
        let w = &mut *self.world;
        let r8 = w.target_distance(i, &own)?;
        w.monsters[i].flags_8 &= !0x4_0000;
        let r14 = w.walk_for_attack(i, mv)?;
        let m = &w.monsters[i];
        let id = m.identifier;
        let own_target = w.combat(i).target == i as i16;
        let mut point;
        let mut edx = false;
        if m.flags & 0x10 != 0 {
            point = w.combat(i).location;
            crate::attack_ai::log_location_attack(|| crate::attack_ai::LocationAttackRun::Leg {
                monster: i,
                point,
                r14,
            });
        } else {
            let t = usize::try_from(w.combat(i).target).map_err(|_| {
                Unmodelled::new(
                    draw_site::ENGAGED_LEG_NO_TARGET,
                    format!("monster {id}: no target"),
                )
            })?;
            let tobj = &w.monsters[t];
            point = if t == i { own.position } else { tobj.position };
            if w.game_options & mandrake_common::game_options::TFL == 0
                && t != i
                && let Some(p) = w.lead_point(t, &own)
            {
                point = p;
            }
            let tobj = &w.monsters[t];
            if tobj.mode == 4 && t != i {
                let tf = w
                    .movers
                    .get(t)
                    .and_then(Option::as_ref)
                    .map(|tm| tm.desired_facing)
                    .ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::ENGAGED_LEG_TARGET_NO_MOVER,
                            format!("monster {id}: target without a mover"),
                        )
                    })?;
                edx = angular_difference(mv.desired_facing, tf) > 0x3fff;
            }
        }
        if matches!(w.monsters[i].anim_state, 0x12 | 0x13) {
            w.distance_gait(i, r8);
        }
        let m = &w.monsters[i];
        let def = &w.monster_definitions[m.definition];
        let turn_sequence = def.turn_sequence;
        let def = &def.ai;
        let (class, b198, b199) = (def.class, def.byte_198, def.byte_199);
        let (w70, w78) = (i32::from(def.engage_distance), i32::from(def.vision));
        let mut flags = m.flags;
        let mut mode = 4u8;
        if r14 < 0 && flags & 0x8000_0000 == 0 {
            point = self.world.generate_point_of_retreat(i, Some(point))?.1;
            flags |= 0x10_0000;
        } else {
            if !own_target {
                let (dx, dy) = (point.0 - mv.x, point.1 - mv.y);
                if dx | dy != 0 {
                    mv.desired_facing = crate::engine_move::arctangent(dx, dy);
                }
            }
            let mut melee_test = true;
            if r14 >= 0 {
                if r14 == 0 && class == 1 && !(flags & 0x1_0000 != 0 && !edx) {
                    mode = 6;
                }
                if flags & 4 == 0 && class == 1 {
                    if r8 > w70 || b199 & 0x40 == 0 {
                        mode = 6;
                    }
                    melee_test = false;
                }
            }
            if melee_test && class == 0 && b198 & 0x40 != 0 && r8 >= w78 && flags & 4 == 0 {
                mode = 6;
            }
            if flags & 0x8_0000 != 0 {
                mode = 4 + u8::from(turn_sequence != -1);
                flags &= 0xffff_ffe6;
                w.monsters[i].flags_8 &= 0xf7ff_fdff;
            }
            flags &= 0xffe7_ffff;
        }
        self.world.monsters[i].flags = flags;
        self.common_gait(mv)?;
        let w = &mut *self.world;
        w.monsters[i].flags &= !0x20;
        match mode {
            6 => return mv.set_mode(MoveMode::Holding, ModeState::Zero, self),
            5 => return mv.set_mode(MoveMode::Turning, ModeState::Zero, self),
            _ => {}
        }
        if mv.leg_to(point, self)? {
            return Ok(());
        }
        let m = &mut self.world.monsters[i];
        m.flags_8 &= 0xf7ff_fdff;
        let f = m.flags;
        m.flags = if f & 8 != 0 { f & !0x10 } else { f & !0x11 };
        if m.flags & 8 == 0 {
            mv.waypoint_reached(self)?;
            if self.world.monsters[i].flags & crate::engine_move::NEW_LEG != 0 {
                self.world.monsters[i].flags = (self.world.monsters[i].flags & !0x80) | 0x20;
            }
            return Ok(());
        }
        mv.set_mode(MoveMode::Holding, ModeState::Zero, self)?;
        let m = &mut self.world.monsters[i];
        m.flags = (m.flags & !0x80) | 1;
        Ok(())
    }
}
