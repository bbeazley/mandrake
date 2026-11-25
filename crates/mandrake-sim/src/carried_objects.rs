use mandrake_common::FourCC;
use mandrake_common::game_options;

use crate::draw_site;
use crate::engine_rng::{
    EngineObject, EngineRng, EngineWorld, Unmodelled, be_i16, be_i32, tag_ref,
};

const MONS_HAS_INVENTORY: u32 = 0x20_0000;
const OBJECT_NO_SHAPE: u32 = 0x200;
const OBJECT_CLEARED_WHEN_CARRIED: u32 = 0x1_0000;

struct PickUp {
    projectile: Option<FourCC>,
    slot: u8,
    forced: bool,
}

struct Artifact<'a>(&'a [u8]);

impl Artifact<'_> {
    fn flags(&self) -> u32 {
        be_i32(self.0, 0) as u32
    }

    fn monster(&self) -> Option<FourCC> {
        tag_ref(self.0, 0x04)
    }

    fn max_charges(&self) -> i16 {
        match be_i16(self.0, 0x08) {
            -1 => 0x7fff,
            v => v,
        }
    }

    fn charges(&self, rng: &mut EngineRng, site: u32) -> i16 {
        let delta = be_i16(self.0, 0x0c);
        let extra = if delta > 0 {
            (i32::from(delta) * i32::from(rng.draw(site))) >> 16
        } else {
            0
        };
        be_i16(self.0, 0x0a).wrapping_add(extra as i16)
    }

    fn becomes(&self) -> Option<FourCC> {
        tag_ref(self.0, 0x98)
    }

    fn recharge(&self) -> Option<FourCC> {
        tag_ref(self.0, 0x1c)
    }
}

impl EngineWorld {
    pub fn carried_projectile(&self, m: usize) -> Option<FourCC> {
        let carried = i32::from(self.combat(m).carried);
        self.projectiles
            .iter()
            .find(|p| p.object.index == carried && p.object.kind == 4)
            .map(|p| p.definition)
    }

    pub fn monster_artifact_charges(&self, m: usize) -> Result<i16, Unmodelled> {
        let c = self.combat(m);
        if c.carried == -1 || c.special == -1 {
            return Ok(0);
        }
        let carried = i32::from(c.carried);
        self.projectiles
            .iter()
            .find(|p| p.object.index == carried && p.object.kind == 4)
            .map(|p| p.object.charges)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::ARTIFACT_CHARGES_NOT_PROJECTILE,
                    format!(
                        "monster {}: the carried object {carried} isn't a projectile's",
                        self.monsters[m].identifier
                    ),
                )
            })
    }

    pub(crate) fn artifact_flags(&self, projectile: FourCC) -> Result<Option<u32>, Unmodelled> {
        Ok(self.artifact_of(projectile)?.map(|(_, a)| a.flags()))
    }

    pub(crate) fn artifact_recharge(
        &self,
        projectile: FourCC,
    ) -> Result<Option<FourCC>, Unmodelled> {
        Ok(self
            .artifact_of(projectile)?
            .and_then(|(_, a)| a.recharge()))
    }

    fn artifact_of(&self, projectile: FourCC) -> Result<Option<(i16, Artifact<'_>)>, Unmodelled> {
        let Some(tag) = self
            .projectile_raw(projectile)?
            .and_then(|d| tag_ref(d, 0x78))
        else {
            return Ok(None);
        };
        let Some(k) = self.catalog.artifacts.iter().position(|(t, _)| *t == tag) else {
            return Err(Unmodelled::new(
                draw_site::ARTIFACT_NOT_IN_DATA,
                format!("projectile {projectile}: artifact {tag} isn't in the data"),
            ));
        };
        Ok(Some((k as i16, Artifact(&self.catalog.artifacts[k].1))))
    }

    fn projectile_raw(&self, projectile: FourCC) -> Result<Option<&[u8]>, Unmodelled> {
        match self.catalog.projectiles.get(&projectile) {
            Some(p) => Ok(Some(&p.raw)),
            None => Err(Unmodelled::new(
                draw_site::PROJECTILE_RAW_MISSING,
                format!("projectile {projectile} isn't in the catalog"),
            )),
        }
    }

    fn flag_4(&self, a: &Artifact<'_>) -> bool {
        a.flags() & 4 != 0
            && a.becomes()
                .is_none_or(|t| self.monster_definitions.iter().any(|d| d.tag == t))
    }

    fn held_definition(&self, index: i16) -> Option<FourCC> {
        self.projectiles
            .iter()
            .find(|p| p.object.index == i32::from(index))
            .map(|p| p.definition)
    }

    fn held_charges(&self, index: i16) -> i16 {
        self.projectiles
            .iter()
            .find(|p| p.object.index == i32::from(index))
            .map_or(-1, |p| p.object.charges)
    }

    fn recharge_target(&self, m: usize, projectile: FourCC) -> Result<Option<i16>, Unmodelled> {
        let c = self.combat(m);
        if c.special != -1 {
            let a = Artifact(&self.catalog.artifacts[c.special as usize].1);
            if c.carried != -1 && a.flags() & 0x2_0000 != 0 && a.recharge() == Some(projectile) {
                let charges = self.held_charges(c.carried);
                if a.max_charges() > charges && charges != -1 {
                    return Ok(Some(c.carried));
                }
            }
            if a.flags() & 0x20 != 0 {
                return Ok(None);
            }
        }
        for &object in c.inventory.objects() {
            if object == -1 {
                continue;
            }
            let Some(definition) = self.held_definition(object) else {
                return Err(Unmodelled::new(
                    draw_site::RECHARGE_NOT_AT_REST,
                    format!(
                        "monster {}: inventory object {object} isn't a projectile's at rest",
                        self.monsters[m].identifier
                    ),
                ));
            };
            let Some((_, a)) = self.artifact_of(definition)? else {
                continue;
            };
            let charges = self.held_charges(object);
            if a.flags() & 0x2_0000 != 0
                && a.recharge() == Some(projectile)
                && charges != -1
                && a.max_charges() > charges
            {
                return Ok(Some(object));
            }
        }
        Ok(None)
    }

    fn could_pick_up(
        &self,
        m: usize,
        projectile: FourCC,
        shading_slot: u8,
    ) -> Result<u32, Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let o = &self.monsters[m];
        let c = self.combat(m);
        let raw = &self.monster_definitions[o.definition].ai.raw;
        let inventory = be_i32(raw, 0) as u32 & MONS_HAS_INVENTORY != 0;
        let room = c.inventory.len < c.inventory_size;
        let carried_keeps = if inventory && room && c.carried != -1 {
            let Some(d) = self.held_definition(c.carried) else {
                return Err(Unmodelled::new(
                    draw_site::PICK_UP_CARRIED_NOT_AT_REST,
                    format!(
                        "monster {}: the carried object {} isn't a projectile's at rest",
                        o.identifier, c.carried
                    ),
                ));
            };
            self.artifact_of(d)?
                .is_some_and(|(_, a)| a.flags() & 0x20 != 0)
        } else {
            false
        };
        let take = if !inventory {
            (c.carried == -1).then_some(0)
        } else if !room {
            (c.carried == -1 && gameplay <= game_options::GAMEPLAY_1_5_1).then_some(0)
        } else if c.carried == -1 || !carried_keeps {
            Some(0x10)
        } else {
            None
        };
        let mut first = 0;
        let mut proj = projectile;
        for replacement in [false, true] {
            let bits = if replacement { first | 8 } else { 0 };
            if tag_ref(raw, 0x1ec) == Some(proj)
                && shading_slot != 1
                && i16::from(c.ammunition as i8) < be_i16(raw, 0x19a)
            {
                return Ok(bits | 5);
            }
            if gameplay > game_options::GAMEPLAY_1_7_2 && self.recharge_target(m, proj)?.is_some() {
                return Ok(bits | 0x21);
            }
            if let Some(extra) = take {
                if !replacement {
                    first = extra;
                }
                if self.carrier_takes(m, proj)? {
                    return Ok(bits | extra | 1);
                }
            }
            if replacement {
                return Ok(0);
            }
            let d = self.projectile_raw(proj)?.unwrap_or_default();
            let Some(next) = tag_ref(d, 0x5c) else {
                return Ok(0);
            };
            proj = next;
        }
        unreachable!("the replacement pass returns")
    }

    fn calculate_monster_speed(&mut self, m: usize) -> Result<(), Unmodelled> {
        let c = self.combat(m);
        if c.carried != -1
            && c.special != -1
            && Artifact(&self.catalog.artifacts[c.special as usize].1).flags() & 0x20 != 0
        {
            return Ok(());
        }
        if c.inventory.is_empty() {
            return Ok(());
        }
        let mut speed = self.monsters[m].speed;
        for &object in c.inventory.objects() {
            let Some(definition) = self.held_definition(object) else {
                return Err(Unmodelled::new(
                    draw_site::SPEED_INVENTORY_NOT_AT_REST,
                    format!(
                        "monster {}: inventory object {object} isn't a projectile's at rest",
                        self.monsters[m].identifier
                    ),
                ));
            };
            let Some((_, a)) = self.artifact_of(definition)? else {
                continue;
            };
            if a.flags() & 0x1004 != 0x1004 {
                continue;
            }
            let Some(t) = a.becomes() else {
                return Err(Unmodelled::new(
                    draw_site::SPEED_ARTIFACT_WITHOUT_UNIT,
                    format!(
                        "monster {}: an inventory artifact of flags 0x1004 without +0x98",
                        self.monsters[m].identifier
                    ),
                ));
            };
            if let Some(d) = self.monster_definitions.iter().find(|d| d.tag == t) {
                speed = speed.min(d.speed);
            }
        }
        self.monsters[m].speed = speed;
        if let Some(Some(mv)) = self.movers.get_mut(m) {
            mv.speed = speed;
        }
        Ok(())
    }

    fn carrier_takes(&self, m: usize, proj: FourCC) -> Result<bool, Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let current = &self.monster_definitions[self.monsters[m].definition];
        let own = &self.monster_definitions[self.own_definition(m)];
        let raw = &own.ai.raw;
        let d = self.projectile_raw(proj)?.unwrap_or_default();
        let class = be_i16(&current.ai.raw, 0x8a);
        if class != -1 && class == be_i16(d, 0x26) {
            return Ok(true);
        }
        let Some((_, a)) = self.artifact_of(proj)? else {
            return Ok(false);
        };
        let Some(t) = a.monster() else {
            return Ok(true);
        };
        if t == own.tag {
            return Ok(true);
        }
        if gameplay <= game_options::GAMEPLAY_1_5_1 || a.flags() & 0x100 == 0 {
            return Ok(false);
        }
        let Some(theirs) = self.monster_definitions.iter().find(|d| d.tag == t) else {
            return Err(Unmodelled::new(
                draw_site::CARRIER_MONSTER_NOT_LOADED,
                format!(
                    "monster {}: the artifact's monster {t} isn't loaded",
                    self.monsters[m].identifier
                ),
            ));
        };
        Ok(theirs.ai.raw.get(4..8) == raw.get(4..8))
    }

    fn switch_monster_artifact(
        &mut self,
        m: usize,
        p: Option<usize>,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let id = self.monsters[m].identifier;
        let mut special = if gameplay > game_options::GAMEPLAY_1_7_2 {
            -1
        } else {
            self.combat(m).special
        };
        self.combat_mut(m).byte_8d |= 8;
        let mut target = self.own_definition(m);
        if let Some(p) = p
            && self.projectiles[p].object.kind == 4
            && let Some((k, a)) = self.artifact_of(self.projectiles[p].definition)?
        {
            special = k;
            let charges = self.projectiles[p].object.charges;
            let flags = a.flags();
            let time = be_i16(a.0, 0x32);
            let new_charges =
                (charges == -1).then(|| a.charges(rng, draw_site::SWITCH_ARTIFACT_CHARGES));
            let becomes = (self.flag_4(&a) && self.carrier_becomes(m, &a)?).then(|| a.becomes());
            let held = &mut self.projectiles[p];
            if let Some(v) = new_charges {
                held.object.charges = v;
            }
            if held.charge_time < 1 {
                held.charge_time = time;
            }
            if let Some(becomes) = becomes {
                let Some(d) =
                    becomes.and_then(|t| self.monster_definitions.iter().position(|d| d.tag == t))
                else {
                    return Err(Unmodelled::new(
                        draw_site::SWITCH_ARTIFACT_WITHOUT_UNIT,
                        format!("monster {id}: an artifact of flag 4 without +0x98's mons"),
                    ));
                };
                target = d;
                if flags & 0x80 != 0 {
                    return Err(Unmodelled::new(
                        draw_site::SWITCH_ARTIFACT_SETS_AC,
                        format!("monster {id}: an artifact of flags 0x84 sets +0xac"),
                    ));
                }
            }
        }
        self.combat_mut(m).special = special;
        if self.monsters[m].definition != target {
            self.change_monster_definition(m, target, rng)?;
        } else {
            self.calculate_monster_speed(m)?;
            if gameplay <= 0x6ad
                && special != -1
                && self.flag_4(&Artifact(&self.catalog.artifacts[special as usize].1))
            {
                return Err(Unmodelled::new(
                    draw_site::SWITCH_ARTIFACT_OLD_GAMEPLAY,
                    format!(
                        "monster {id}: switch_monster_artifact's artifact branch at gameplay <= 0x6ad"
                    ),
                ));
            }
        }
        if self.combat(m).carried == -1 {
            self.combat_mut(m).special = -1;
        }
        Ok(())
    }

    fn carrier_becomes(&self, m: usize, a: &Artifact<'_>) -> Result<bool, Unmodelled> {
        let flags = a.flags();
        let Some(t) = a.monster().filter(|_| flags & 0x10 != 0) else {
            return Ok(true);
        };
        let own = &self.monster_definitions[self.own_definition(m)];
        if t == own.tag {
            return Ok(true);
        }
        if flags & 0x100 == 0 {
            return Ok(false);
        }
        let Some(theirs) = self.monster_definitions.iter().find(|d| d.tag == t) else {
            return Err(Unmodelled::new(
                draw_site::CARRIER_BECOMES_NOT_LOADED,
                format!(
                    "monster {}: the artifact's monster {t} isn't loaded",
                    self.monsters[m].identifier
                ),
            ));
        };
        Ok(theirs.ai.raw.get(4..8) == own.ai.raw.get(4..8))
    }

    fn change_monster_definition(
        &mut self,
        m: usize,
        d: usize,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let id = self.monsters[m].identifier;
        let stop = |site: u32, why: &str| {
            Err(Unmodelled::new(
                site,
                format!("monster {id}: a definition change {why}"),
            ))
        };
        if gameplay <= 0x6ad {
            return stop(
                draw_site::CHANGE_DEFINITION_OLD_GAMEPLAY,
                "at gameplay <= 0x6ad",
            );
        }
        let old = self.monsters[m].definition;
        let new = &self.monster_definitions[d];
        if new.ai.raw.get(4..8) != self.monster_definitions[old].ai.raw.get(4..8) {
            return stop(
                draw_site::CHANGE_DEFINITION_COLLECTION,
                &format!("to {}, of another collection", new.tag),
            );
        }
        if gameplay > game_options::GAMEPLAY_1_7_2 && self.monsters[m].flags_8 & 0x1000_0000 != 0 {
            return stop(
                draw_site::CHANGE_DEFINITION_OTHER_UNIT,
                "of a monster with another unit (+0x08 & 0x10000000)",
            );
        }
        let special = self.combat(m).special;
        if gameplay > game_options::GAMEPLAY_1_7_2
            && special != -1
            && be_i32(&self.catalog.artifacts[special as usize].1, 0) as u32 & 0x8_0000 != 0
        {
            return stop(
                draw_site::CHANGE_DEFINITION_ARTIFACT_UNIT,
                "with an artifact of flag 0x80000 (its unit)",
            );
        }
        let lent = m < self.movers.len();
        if !lent && self.monsters[m].mode != 1 {
            return stop(
                draw_site::CHANGE_DEFINITION_NO_MOVER,
                &format!(
                    "of a monster without its movement model in mode {}",
                    self.monsters[m].mode
                ),
            );
        }
        let movement = match self.movement_of(m, d) {
            Ok(v) => v,
            Err(e) => {
                return stop(
                    draw_site::CHANGE_DEFINITION_COLLECTION,
                    &format!("to {}: {e}", new.tag),
                );
            }
        };
        let mut mv = if lent {
            let Some(mv) = self.movers[m].take() else {
                return stop(
                    draw_site::CHANGE_DEFINITION_NO_MOVER,
                    "of a monster without its movement model",
                );
            };
            mv
        } else {
            let (x, y) = self.monsters[m].position;
            crate::engine_move::EngineMover::new(movement.clone(), x, y, self.monsters[m].facing)
        };
        let new = &self.monster_definitions[d];
        let (speed, mana_max, mons_flags, field_199) =
            (new.speed, new.ai.word(0x214), new.flags, new.field_199);
        let own = self.own_definition(m);
        self.monsters[m].definition = d;
        self.refresh_def_impassable();
        self.combat_mut(m).own_definition = (own != d).then_some(own);
        self.monsters[m].speed = speed;
        mv.speed = speed;
        let c = self.combat_mut(m);
        c.mana = c.mana.min(mana_max);
        let o = &mut self.monsters[m].object;
        o.flags &= !0x1800;
        if mons_flags & 4 != 0 {
            o.flags |= 0x1800;
        }
        if gameplay > game_options::GAMEPLAY_1_7_1 && field_199 & 0x10 != 0 {
            o.flags |= 0x1000;
        }
        self.turn_off_monster_local_group(m);
        crate::engine_move::set_movement(&mut mv, movement);
        let own_mover = if lent {
            self.movers[m] = Some(mv);
            None
        } else {
            Some(mv)
        };
        self.calculate_monster_speed(m)?;
        let mode = self.monsters[m].mode;
        if (11..=14).contains(&mode) && gameplay > game_options::GAMEPLAY_1_5_1 {
            return Ok(());
        }
        let Some(move_mode) = crate::engine_move::MoveMode::from_byte(mode) else {
            return stop(
                draw_site::CHANGE_DEFINITION_MODE,
                &format!("in mode {mode}"),
            );
        };
        let mut mv = match own_mover {
            Some(mv) => mv,
            None => {
                let Some(mv) = self.movers[m].take() else {
                    return stop(
                        draw_site::CHANGE_DEFINITION_NO_MOVER,
                        "of a monster without its movement model",
                    );
                };
                mv
            }
        };
        let mut actions = self.map_actions.take();
        let mut host = crate::engine_attack::NoHost;
        let r = {
            let mut hooks = crate::engine_rng::WorldHooks {
                world: self,
                actions: &mut actions,
                rng,
                monster: m,
                time: 0,
                host: &mut host,
            };
            mv.set_mode(move_mode, crate::engine_move::ModeState::Zero, &mut hooks)
        };
        self.map_actions = actions;
        if lent {
            self.movers[m] = Some(mv);
        }
        r
    }

    fn movement_of(
        &self,
        m: usize,
        d: usize,
    ) -> Result<crate::engine_move::MovementDefinition, String> {
        let new = &self.monster_definitions[d];
        let headers = self
            .collections
            .get(&self.monsters[m].object.collection)
            .map_or(&[][..], |v| v);
        crate::engine_move::MovementDefinition::from_tags(
            new.tag,
            &new.ai.raw,
            headers,
            self.formation_globals.gameplay_version,
        )
    }

    pub fn changed_movement(
        &self,
        m: usize,
        mover: &mut crate::engine_move::EngineMover,
    ) -> Result<(), Unmodelled> {
        let d = self.monsters[m].definition;
        if self.monster_definitions[d].tag == mover.definition.tag {
            return Ok(());
        }
        let movement = self.movement_of(m, d).map_err(|e| {
            Unmodelled::new(
                draw_site::CHANGE_DEFINITION_COLLECTION,
                format!(
                    "monster {}: a definition change to {}: {e}",
                    self.monsters[m].identifier, self.monster_definitions[d].tag
                ),
            )
        })?;
        crate::engine_move::set_movement(mover, movement);
        Ok(())
    }

    pub fn give_object_to_monster(
        &mut self,
        m: usize,
        identifier: i16,
        rng: &mut EngineRng,
    ) -> Result<u32, Unmodelled> {
        let Some(object) = self
            .filter_objects()
            .into_iter()
            .find(|o| o.identifier != 0 && o.identifier == identifier)
        else {
            return Ok(0);
        };
        self.give_object_at(m, object.index as i16, identifier, rng)
    }

    fn object_to_pick_up(
        &self,
        m: usize,
        index: i16,
        identifier: i16,
    ) -> Result<Option<PickUp>, Unmodelled> {
        self.object_to_pick_up_in(
            m,
            (index, identifier),
            &self.projectile_records,
            &self.dormant_objects,
        )
    }

    fn object_to_pick_up_in(
        &self,
        m: usize,
        (index, identifier): (i16, i16),
        records: &[Box<crate::projectiles::FlightProjectile>],
        dormant: &[crate::flight::DormantObject],
    ) -> Result<Option<PickUp>, Unmodelled> {
        let id = self.monsters[m].identifier;
        let index = i32::from(index);
        let is =
            |o: &crate::engine_rng::EngineObject| o.index == index && o.identifier == identifier;
        let found = self
            .projectiles
            .iter()
            .find(|p| is(&p.object))
            .map(|p| (Some(p.definition), &p.object))
            .or_else(|| {
                dormant
                    .iter()
                    .find(|d| is(&d.object))
                    .map(|d| (Some(d.definition), &d.object))
            })
            .or_else(|| {
                records.iter().find_map(|p| {
                    p.shape
                        .as_ref()
                        .filter(|o| is(o))
                        .map(|o| (Some(p.definition), o))
                })
            })
            .or_else(|| {
                self.scenery
                    .iter()
                    .find(|s| !s.deleted && is(&s.object))
                    .map(|s| (s.projectile, &s.object))
            });
        let Some((projectile, o)) = found else {
            return Ok(None);
        };
        if o.no_shape {
            return Ok(None);
        }
        let slot = o.shading_slot;
        if o.flags & 0x10_0000 != 0 {
            return Ok(Some(PickUp {
                projectile,
                slot,
                forced: true,
            }));
        }
        if o.kind != 3 && o.kind != 4 {
            return Ok(None);
        }
        if projectile.is_none() {
            return Err(Unmodelled::new(
                draw_site::PICK_UP_NO_LIST,
                format!("monster {id}: picking up object {index} of no list"),
            ));
        }
        Ok(Some(PickUp {
            projectile,
            slot,
            forced: false,
        }))
    }

    fn could_pick_up_found(&self, m: usize, found: &PickUp) -> Result<u32, Unmodelled> {
        match (found.forced, found.projectile) {
            (true, _) => Ok(3),
            (false, Some(projectile)) => self.could_pick_up(m, projectile, found.slot),
            (false, None) => unreachable!("object_to_pick_up stops first"),
        }
    }

    pub fn monster_could_pick_up_object(
        &self,
        m: usize,
        index: i16,
        identifier: i16,
    ) -> Result<u32, Unmodelled> {
        match self.object_to_pick_up(m, index, identifier)? {
            Some(found) => self.could_pick_up_found(m, &found),
            None => Ok(0),
        }
    }

    pub fn monster_could_pick_up_object_in(
        &self,
        m: usize,
        (index, identifier): (i16, i16),
        records: &[Box<crate::projectiles::FlightProjectile>],
        dormant: &[crate::flight::DormantObject],
    ) -> Result<u32, Unmodelled> {
        match self.object_to_pick_up_in(m, (index, identifier), records, dormant)? {
            Some(found) => self.could_pick_up_found(m, &found),
            None => Ok(0),
        }
    }

    pub(crate) fn pick_up_point(&self, index: i16, identifier: i16) -> Option<(i32, i32)> {
        self.filter_objects()
            .into_iter()
            .find(|o| o.index == i32::from(index) && o.identifier == identifier)
            .map(|o| o.point)
    }

    pub fn execute_pick_up_command(
        &mut self,
        monsters: &[usize],
        index: i16,
        identifier: i16,
        player: i16,
    ) -> Result<(), Unmodelled> {
        self.execute_pick_up(monsters, index, identifier, -1, player)
    }

    pub fn handle_pick_up_command(
        &mut self,
        source: i32,
        monsters: &[usize],
        index: i16,
        identifier: i16,
    ) -> Result<(), Unmodelled> {
        if source == -1 {
            return Err(Unmodelled::new(
                draw_site::PICK_UP_COMMAND_QUEUE,
                "handle_pick_up_command: queue_outgoing_command",
            ));
        }
        let max = if self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3 {
            0x7e
        } else {
            0x1e
        };
        let monsters: Vec<usize> = monsters.iter().take(max).copied().collect();
        let player = self.local_player;
        self.execute_pick_up(&monsters, index, identifier, source, player)
    }

    fn execute_pick_up(
        &mut self,
        monsters: &[usize],
        index: i16,
        identifier: i16,
        source: i32,
        player: i16,
    ) -> Result<(), Unmodelled> {
        for m in self.filter_command_monsters(monsters, source, player) {
            if self.monster_could_pick_up_object(m, index, identifier)? == 0 {
                continue;
            }
            let c = self.combat_mut(m);
            c.pending = -1;
            c.pick_up = (index, identifier);
            let o = &mut self.monsters[m];
            o.object.flags &= !crate::collide::OBJECT_POSED;
            o.flags_8 &= 0xf7db_f9ff;
            o.flags = (o.flags & 0x7f07_ff62) | 0x4022;
            if let Some(Some(mv)) = self.movers.get_mut(m) {
                mv.waypoints.clear();
            }
        }
        Ok(())
    }

    fn delete_object_at_rest(&mut self, index: i32) {
        self.projectiles.retain(|p| p.object.index != index);
        self.dormant_objects.retain(|d| d.object.index != index);
        for s in self.scenery.iter_mut() {
            if !s.deleted && s.object.index == index {
                s.deleted = true;
                self.scenery_taken.push(index);
            }
        }
        self.free_objects.insert(index);
    }

    fn object_at_rest_mut(&mut self, index: i32) -> Option<(&mut EngineObject, &mut FourCC)> {
        if let Some(p) = self
            .projectiles
            .iter_mut()
            .find(|p| p.object.index == index)
        {
            return Some((&mut p.object, &mut p.definition));
        }
        if let Some(d) = self
            .dormant_objects
            .iter_mut()
            .find(|d| d.object.index == index)
        {
            return Some((&mut d.object, &mut d.definition));
        }
        self.scenery
            .iter_mut()
            .find(|s| !s.deleted && s.object.index == index)
            .and_then(|s| Some((&mut s.object, s.projectile.as_mut()?)))
    }

    pub fn give_object_at(
        &mut self,
        m: usize,
        index: i16,
        identifier: i16,
        rng: &mut EngineRng,
    ) -> Result<u32, Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let id = self.monsters[m].identifier;
        let Some(found) = self.object_to_pick_up(m, index, identifier)? else {
            return Ok(0);
        };
        let result = self.could_pick_up_found(m, &found)?;
        if result == 0 {
            return Ok(0);
        }
        let mut projectile = found.projectile;
        let object_index = i32::from(index);
        if let Some(at) = self
            .projectile_records
            .iter()
            .position(|p| p.shape.as_ref().is_some_and(|o| o.index == object_index))
        {
            let mut record = self.projectile_records.remove(at);
            if let Some(d) = crate::flight::sleep_projectile(&mut record, Some(&self.flag_data)) {
                self.dormant_objects.push(d);
            }
        }
        if result & 8 != 0 {
            let from = projectile.expect("could_pick_up's replacement has a definition");
            let next = self
                .projectile_raw(from)?
                .and_then(|d| tag_ref(d, 0x5c))
                .expect("could_pick_up took the replacement");
            if let Some((_, definition)) = self.object_at_rest_mut(object_index) {
                *definition = next;
            }
            projectile = Some(next);
        }
        if result & 4 != 0 {
            let c = self.combat_mut(m);
            c.ammunition = c.ammunition.wrapping_add(1);
            c.byte_8d |= 0x80;
            self.delete_object_at_rest(object_index);
            return Ok(result);
        }
        if result & 0x20 != 0 {
            let projectile = projectile.expect("could_pick_up's charges have a definition");
            return self.recharge_from(m, object_index, projectile, result, rng);
        }
        self.hold_object(object_index);
        let Some(p) = self
            .projectiles
            .iter()
            .position(|p| p.object.index == object_index)
        else {
            return Err(Unmodelled::new(
                draw_site::GIVE_OBJECT_NO_LIST,
                format!("monster {id}: carrying object {object_index} of no list"),
            ));
        };
        let c = self.combat_mut(m);
        if result & 0x10 != 0 {
            c.inventory.push(index);
        }
        c.byte_8d |= 8;
        c.carried = index;
        let artifact = match projectile {
            Some(projectile) => self.artifact_of(projectile)?,
            None => None,
        };
        let new = match &artifact {
            Some((k, a)) => {
                if self.projectiles[p].object.charges == -1 {
                    let flags = a.flags();
                    let v = a.charges(rng, draw_site::GIVE_OBJECT_CHARGES);
                    self.projectiles[p].object.charges = v;
                    if flags & 0x2000 != 0 {
                        let facing = self.projectiles[p].facing;
                        self.monsters[m].facing = facing;
                        if let Some(Some(mv)) = self.movers.get_mut(m) {
                            mv.facing = facing;
                        }
                    }
                }
                Some(*k)
            }
            None => None,
        };
        self.combat_mut(m).special = new.unwrap_or(-1);
        if gameplay >= 0x515 {
            self.switch_monster_artifact(m, Some(p), rng)?;
            if gameplay > game_options::GAMEPLAY_1_7 && self.monsters[m].anim_state == 0xbb {
                self.monsters[m].anim_state = 0xff;
            }
        }
        let held = &mut self.projectiles[p].object;
        held.flags = held.flags & !OBJECT_CLEARED_WHEN_CARRIED | OBJECT_NO_SHAPE;
        held.no_shape = true;
        Ok(result)
    }

    pub(crate) fn put_away_carried_artifact(
        &mut self,
        m: usize,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let raw = &self.monster_definitions[self.own_definition(m)].ai.raw;
        let c = self.combat(m);
        if be_i32(raw, 0) as u32 & MONS_HAS_INVENTORY == 0
            || c.inventory.len <= 0
            || c.carried == -1
        {
            return Ok(());
        }
        let keeps = raw.get(0x224).is_some_and(|b| b & 2 != 0);
        let carried = c.carried;
        let Some(definition) = self.held_definition(carried) else {
            return Err(Unmodelled::new(
                draw_site::PUT_AWAY_NOT_AT_REST,
                format!(
                    "monster {}: the carried object {carried} isn't a projectile's at rest",
                    self.monsters[m].identifier
                ),
            ));
        };
        let Some((_, a)) = self.artifact_of(definition)? else {
            return Ok(());
        };
        if a.flags() & 0x20 != 0 && (gameplay <= game_options::GAMEPLAY_1_7_2 || !keeps) {
            return Ok(());
        }
        self.combat_mut(m).carried = -1;
        self.switch_monster_artifact(m, None, rng)
    }

    pub fn initial_inventory(
        &mut self,
        m: usize,
        group: FourCC,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let Some(g) = self.catalog.groups.get(&group).cloned() else {
            return Err(Unmodelled::new(
                draw_site::INITIAL_INVENTORY_GROUP_MISSING,
                format!("prgr {group} (an initial inventory) not loaded"),
            ));
        };
        for part in &g.parts {
            let (base, delta) = part.count();
            let mut n = i32::from(base);
            if delta > 0 {
                n += (i32::from(delta) * i32::from(rng.draw(draw_site::INITIAL_INVENTORY_COUNT)))
                    >> 16;
            }
            if part.projectile.is_some() {
                for _ in 0..n.max(0) {
                    let r = rng.draw(draw_site::INITIAL_INVENTORY_PART);
                    let definition = if r <= part.threshold() {
                        part.projectile
                    } else {
                        part.projectile_18
                    };
                    let Some(definition) = definition else {
                        continue;
                    };
                    if self.inventory_object(m, definition, rng)?
                        && gameplay > game_options::GAMEPLAY_1_7_2
                    {
                        self.put_away_carried_artifact(m, rng)?;
                    }
                }
            }
            if gameplay <= game_options::GAMEPLAY_1_7_2 {
                self.put_away_carried_artifact(m, rng)?;
            }
        }
        Ok(())
    }

    fn inventory_object(
        &mut self,
        m: usize,
        definition: FourCC,
        rng: &mut EngineRng,
    ) -> Result<bool, Unmodelled> {
        let o = &self.monsters[m];
        let desc = crate::projectile_groups::ProjectileDescriptor {
            definition,
            flags: 0,
            position: [o.position.0, o.position.1, o.object.z],
            velocity: o.object.velocity,
            facing: o.facing,
            elevation: 0,
            word_1c: 0,
            owner_monster: -1,
            parent: -1,
            owner_object: -1,
            target: -1,
            target_point: [0; 3],
        };
        let made = crate::projectile_groups::new_projectile(
            &mut crate::projectile_groups::EngineSpawn {
                world: self,
                rng,
                source: None,
                host: None,
            },
            &desc,
        )?;
        let Some(record) = made else {
            return Ok(false);
        };
        let Some((index, identifier)) = self
            .projectile_records
            .iter()
            .find(|p| p.index == record)
            .and_then(|p| p.shape.as_ref())
            .map(|o| (o.index, o.identifier))
        else {
            return Err(Unmodelled::new(
                draw_site::INITIAL_INVENTORY_NO_OBJECT,
                format!("initial inventory: record {record} of {definition} has no object"),
            ));
        };
        if self.give_object_at(m, index as i16, identifier, rng)? != 0 {
            return Ok(true);
        }
        if let Some(at) = self
            .projectile_records
            .iter()
            .position(|p| p.index == record)
        {
            self.projectile_records.remove(at);
        }
        self.delete_object_at_rest(index);
        Ok(false)
    }

    fn hold_object(&mut self, object_index: i32) {
        if let Some(pos) = self
            .dormant_objects
            .iter()
            .position(|d| d.object.index == object_index)
        {
            let d = self.dormant_objects.remove(pos);
            self.projectiles.push(crate::engine_rng::EngineProjectile {
                marker: usize::MAX,
                identifier: d.object.identifier,
                definition: d.definition,
                first_draw: 0,
                object: d.object,
                position: (d.position[0], d.position[1]),
                charge_time: 0,
                facing: d.facing,
                elevation: d.elevation,
                cell_link: d.cell_link,
            });
        }
        if let Some(s) = self
            .scenery
            .iter_mut()
            .find(|s| !s.deleted && s.object.index == object_index)
            && let Some(definition) = s.projectile
        {
            s.deleted = true;
            self.scenery_taken.push(object_index);
            let p = crate::engine_rng::EngineProjectile {
                marker: usize::MAX,
                identifier: s.object.identifier,
                definition,
                first_draw: 0,
                object: s.object.clone(),
                position: s.position,
                charge_time: 0,
                facing: s.facing,
                elevation: 0,
                cell_link: s.cell_link,
            };
            self.projectiles.push(p);
        }
    }

    fn recharge_from(
        &mut self,
        m: usize,
        object: i32,
        projectile: FourCC,
        result: u32,
        rng: &mut EngineRng,
    ) -> Result<u32, Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let held = self
            .recharge_target(m, projectile)?
            .expect("could_pick_up's sub_50f2f0 found it");
        let target = self
            .projectiles
            .iter()
            .position(|p| p.object.index == i32::from(held))
            .expect("recharge_target holds it");
        let target_definition = self.projectiles[target].definition;
        let (max, base, delta) = {
            let (_, a) = self
                .artifact_of(target_definition)?
                .expect("sub_50f2f0's artifact");
            (
                i32::from(a.max_charges()),
                i32::from(be_i16(a.0, 0x0a)),
                be_i16(a.0, 0x0c),
            )
        };
        let own = self
            .object_at_rest_mut(object)
            .map_or(-1, |(o, _)| i32::from(o.charges));
        let add = if projectile != target_definition {
            1
        } else if own != -1 {
            own
        } else if delta > 0 {
            base + ((i32::from(delta) * i32::from(rng.draw(draw_site::RECHARGE_AMOUNT))) >> 16)
        } else {
            base
        };
        let total = i32::from(self.projectiles[target].object.charges) + add;
        let charges = total.min(max);
        self.projectiles[target].object.charges = charges as i16;
        let left = total - charges;
        if self.combat(m).carried != held {
            let c = self.combat_mut(m);
            c.carried = held;
            c.special = -1;
            self.switch_monster_artifact(m, Some(target), rng)?;
        }
        self.combat_mut(m).byte_8d |= 0x80;
        if left == 0 || gameplay <= 0x70d {
            self.delete_object_at_rest(object);
            return Ok(result);
        }
        if let Some((o, _)) = self.object_at_rest_mut(object) {
            o.charges = left as i16;
        }
        let c = self.combat(m);
        let raw = &self.monster_definitions[self.monsters[m].definition].ai.raw;
        if be_i32(raw, 0) as u32 & MONS_HAS_INVENTORY != 0 && c.inventory.len < c.inventory_size {
            self.hold_object(object);
            self.combat_mut(m).inventory.push(object as i16);
            if let Some(p) = self
                .projectiles
                .iter_mut()
                .find(|p| p.object.index == object)
            {
                p.object.flags = p.object.flags & !OBJECT_CLEARED_WHEN_CARRIED | OBJECT_NO_SHAPE;
                p.object.no_shape = true;
            }
        }
        Ok(result)
    }

    pub(crate) fn attack_takes_carried(&mut self, m: usize, a: i16) -> Result<FourCC, Unmodelled> {
        let id = self.monsters[m].identifier;
        let Some(projectile) = self.carried_projectile(m) else {
            return Err(Unmodelled::new(
                draw_site::ATTACK_CARRIED_NOT_AT_REST,
                format!("monster {id}: the carried object isn't a projectile's at rest"),
            ));
        };
        let definition = self.monsters[m].definition;
        if let Some(attack) = self.monster_definitions[definition].ai.attack_mut(a as i8) {
            attack.projectile = Some(projectile);
        }
        Ok(projectile)
    }

    pub(crate) fn special_launched(&mut self, m: usize) -> Result<bool, Unmodelled> {
        let c = self.combat(m);
        let special_attack = self.monsters[m].anim_state == 0xbb;
        let changed = self.own_definition(m) != self.monsters[m].definition;
        if c.special == -1 || c.carried == -1 || !(special_attack || changed) {
            return Ok(false);
        }
        let carried = i32::from(c.carried);
        let art = Artifact(&self.catalog.artifacts[c.special as usize].1).flags();
        let Some(at) = self
            .projectiles
            .iter()
            .position(|p| p.object.index == carried)
        else {
            return Err(Unmodelled::new(
                draw_site::SPECIAL_LAUNCHED_NOT_PROJECTILE,
                format!(
                    "monster {}: the special ability's object isn't a projectile's",
                    self.monsters[m].identifier
                ),
            ));
        };
        let mut charges = self.projectiles[at].object.charges;
        if charges > 0 {
            if !special_attack && art & 0x4000 != 0 {
                return Ok(false);
            }
            charges -= 1;
            self.projectiles[at].object.charges = charges;
            self.combat_mut(m).byte_8d |= 8;
        }
        Ok(charges == 0 && art & 2 != 0)
    }

    pub fn carried_launched(
        &mut self,
        m: usize,
        record: Option<i32>,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let id = self.monsters[m].identifier;
        let carried = i32::from(self.combat(m).carried);
        let at = if carried != -1 {
            let Some(at) = self
                .projectiles
                .iter()
                .position(|p| p.object.index == carried)
            else {
                return Err(Unmodelled::new(
                    draw_site::CARRIED_LAUNCHED_NOT_AT_REST,
                    format!("monster {id}: the carried object isn't a projectile's at rest"),
                ));
            };
            if let Some(index) = record {
                let identifier = self.projectiles[at].object.identifier;
                let Some(definition) = self
                    .projectile_records
                    .iter()
                    .find(|p| p.index == index)
                    .map(|p| p.definition)
                else {
                    return Err(Unmodelled::new(
                        draw_site::CARRIED_LAUNCHED_NO_RECORD,
                        format!("monster {id}: new_projectile's record {index} isn't there"),
                    ));
                };
                let charges = self
                    .artifact_of(definition)?
                    .is_some_and(|(_, a)| a.flags() & 8 != 0)
                    .then_some(self.projectiles[at].object.charges);
                let new = self
                    .projectile_records
                    .iter_mut()
                    .find(|p| p.index == index)
                    .expect("found above");
                if let Some(shape) = new.shape.as_mut() {
                    shape.identifier = identifier;
                    if let Some(charges) = charges {
                        shape.charges = charges;
                    }
                }
            }
            Some(at)
        } else {
            None
        };
        if self.combat(m).inventory.len != 0 {
            return self.inventory_launched(m, carried, rng);
        }
        if let Some(at) = at {
            let deleted = self.projectiles.remove(at);
            self.free_objects.insert(deleted.object.index);
            self.combat_mut(m).carried = -1;
        }
        let gameplay = self.formation_globals.gameplay_version;
        let c = self.combat_mut(m);
        if gameplay > game_options::GAMEPLAY_1_7_2 {
            c.special = -1;
        }
        c.byte_8d |= 8;
        if self.monsters[m].anim_state == 0xbb {
            self.monsters[m].anim_state = 0xff;
        }
        Ok(())
    }

    fn inventory_launched(
        &mut self,
        m: usize,
        old: i32,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let id = self.monsters[m].identifier;
        let gameplay = self.formation_globals.gameplay_version;
        let n = self.combat(m).inventory.len;
        if gameplay > game_options::GAMEPLAY_1_3 && n > 0 {
            let c = self.combat(m);
            let carried = c.carried;
            let slots = c.inventory.slots;
            let len = (n as usize).min(9);
            if let Some(k) = slots[..len].iter().position(|&o| o == carried) {
                if carried == -1 {
                    return Err(Unmodelled::new(
                        draw_site::INVENTORY_LAUNCHED_EMPTY_SLOT,
                        format!(
                            "monster {id}: an empty inventory slot taken as the carried object"
                        ),
                    ));
                }
                let mut next = k;
                if gameplay > game_options::GAMEPLAY_1_5 {
                    let definition = |o: i16| {
                        self.projectiles
                            .iter()
                            .find(|p| p.object.index == i32::from(o))
                            .map(|p| p.definition)
                    };
                    let Some(own) = definition(carried) else {
                        return Err(Unmodelled::new(
                            draw_site::INVENTORY_LAUNCHED_CARRIED_NOT_AT_REST,
                            format!(
                                "monster {id}: the carried object isn't a projectile's at rest"
                            ),
                        ));
                    };
                    for (j, &o) in slots[..len].iter().enumerate() {
                        if j == k {
                            continue;
                        }
                        let Some(d) = definition(o) else {
                            return Err(Unmodelled::new(
                                draw_site::INVENTORY_LAUNCHED_OBJECT_NOT_AT_REST,
                                format!(
                                    "monster {id}: inventory object {o} isn't a projectile's at rest"
                                ),
                            ));
                        };
                        if d == own {
                            next = if j > k { j - 1 } else { j };
                            break;
                        }
                    }
                }
                let c = self.combat_mut(m);
                let inv = &mut c.inventory;
                if len - 1 > k {
                    inv.slots.copy_within(k + 1..len, k);
                } else {
                    inv.slots[k] = -1;
                }
                inv.len -= 1;
                let left = i32::from(inv.len);
                if left <= next as i32 {
                    next = (left - 1).max(0) as usize;
                }
                if left == 0 {
                    self.delete_launched(m, old)?;
                    let c = self.combat_mut(m);
                    if i32::from(c.carried) == old {
                        c.carried = -1;
                        c.special = -1;
                    }
                } else {
                    if gameplay > game_options::GAMEPLAY_1_8_3 {
                        self.delete_launched(m, old)?;
                    }
                    let c = self.combat_mut(m);
                    c.carried = c.inventory.slots[next];
                }
            }
        }
        let c = self.combat(m);
        if self.monsters[m].mode == 8 && c.inventory.len != 0 && c.carried != -1 {
            return Err(Unmodelled::new(
                draw_site::INVENTORY_LAUNCHED_MODE_8,
                format!("monster {id}: sub_510e20(6) after a launch in mode 8 with an inventory"),
            ));
        }
        if gameplay > game_options::GAMEPLAY_1_3 {
            let carried = i32::from(self.combat(m).carried);
            let p = if carried == -1 {
                None
            } else {
                let Some(p) = self
                    .projectiles
                    .iter()
                    .position(|p| p.object.index == carried)
                else {
                    return Err(Unmodelled::new(
                        draw_site::INVENTORY_LAUNCHED_NOT_AT_REST,
                        format!("monster {id}: the carried object isn't a projectile's at rest"),
                    ));
                };
                Some(p)
            };
            self.switch_monster_artifact(m, p, rng)?;
        }
        self.combat_mut(m).byte_8d |= 8;
        if self.monsters[m].anim_state == 0xbb {
            self.monsters[m].anim_state = 0xff;
        }
        Ok(())
    }

    fn delete_launched(&mut self, m: usize, old: i32) -> Result<(), Unmodelled> {
        let Some(at) = self.projectiles.iter().position(|p| p.object.index == old) else {
            return Err(Unmodelled::new(
                draw_site::DELETE_LAUNCHED_NOT_AT_REST,
                format!(
                    "monster {}: the launched object {old} isn't a projectile's at rest",
                    self.monsters[m].identifier
                ),
            ));
        };
        let deleted = self.projectiles.remove(at);
        self.free_objects.insert(deleted.object.index);
        Ok(())
    }

    pub(crate) fn unit_control_switch(
        &mut self,
        m: usize,
        tag: i32,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let c = self.combat(m);
        if c.inventory.len == 0 {
            return Ok(());
        }
        if tag == 0x6e75_6c6c {
            self.combat_mut(m).carried = -1;
            return self.switch_monster_artifact(m, None, rng);
        }
        let tag = FourCC::new(tag.to_be_bytes());
        let objects = c.inventory.objects().to_vec();
        for object in objects {
            let Some(p) = self
                .projectiles
                .iter()
                .position(|p| p.object.index == i32::from(object))
            else {
                return Err(Unmodelled::new(
                    draw_site::SWITCH_CARRIED_OBJECT_NOT_PROJECTILE,
                    format!(
                        "unit_control 'swit': monster {}: inventory object {object} isn't a \
                         projectile's at rest",
                        self.monsters[m].identifier
                    ),
                ));
            };
            if self.projectiles[p].object.kind == 4 && self.projectiles[p].definition == tag {
                self.combat_mut(m).carried = object;
                return self.switch_monster_artifact(m, Some(p), rng);
            }
        }
        Ok(())
    }

    pub fn delete_monster_carried_object_unit_control(
        &mut self,
        m: usize,
        k: usize,
    ) -> Result<(), Unmodelled> {
        let id = self.monsters[m].identifier;
        let c = self.combat(m);
        if c.carried != -1 && c.inventory.len > 0 {
            let len = (c.inventory.len as usize).min(9);
            let carried = c.carried;
            if let Some(k) = c.inventory.slots[..len].iter().position(|&o| o == carried) {
                let inv = &mut self.combat_mut(m).inventory;
                if len - 1 > k {
                    inv.slots.copy_within(k + 1..len, k);
                } else {
                    inv.slots[k] = -1;
                }
                inv.len -= 1;
            }
        }
        let c = self.combat(m);
        if c.carried != -1 {
            if c.special != -1 && self.formation_globals.gameplay_version <= 0x6ad {
                return Err(Unmodelled::new(
                    draw_site::REMOVE_CARRIED_OLD_GAMEPLAY,
                    format!(
                        "monster {id}: remo's switch_monster_artifact({k}) at gameplay <= 0x6ad"
                    ),
                ));
            }
            let carried = i32::from(c.carried);
            let Some(at) = self
                .projectiles
                .iter()
                .position(|p| p.object.index == carried)
            else {
                return Err(Unmodelled::new(
                    draw_site::REMOVE_CARRIED_NOT_AT_REST,
                    format!("monster {id}: the carried object isn't a projectile's at rest"),
                ));
            };
            let deleted = self.projectiles.remove(at);
            self.free_objects.insert(deleted.object.index);
            self.combat_mut(m).carried = -1;
            if self.combat(m).special != -1 {
                self.calculate_monster_speed(m)?;
            }
        }
        let c = self.combat_mut(m);
        c.byte_8d |= 8;
        c.special = -1;
        if self.monsters[m].anim_state == 0xbb {
            self.monsters[m].anim_state = 0xff;
        }
        Ok(())
    }
}
