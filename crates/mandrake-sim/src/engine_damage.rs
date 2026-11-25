use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_tag::projectile::DamageDefinition;

use crate::draw_site;
use crate::engine_move::{EngineMover, ModeState, MoveMode, distance2d};
use crate::engine_rng::{EngineRng, EngineWorld, Unmodelled, WorldHooks, be_i16, be_i32};
use crate::projectile_groups::{
    GroupDescriptor, ProjectileDescriptor, ProjectileWorld, SimSpawn, line_segment_to_vector,
    new_projectile,
};

enum Conversion {
    None,
    Change(i16),
    NoRow,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageInfo {
    pub unit: crate::statistics::UnitRef,
    pub attacker: i16,
    pub attacker_id: i16,
    pub group: Option<FourCC>,
    pub point: [i32; 3],
    pub velocity: [i16; 3],
    pub scale: i16,
    pub magnitude: i16,
    pub amount: i16,
    pub flags: u16,
    pub mana: i16,
    pub factor: f32,
    pub killed: Option<u8>,
}

impl DamageInfo {
    pub fn new(unit: crate::statistics::UnitRef, attacker: i16, attacker_id: i16) -> Self {
        Self {
            unit,
            attacker,
            attacker_id,
            group: None,
            point: [0; 3],
            velocity: [0; 3],
            scale: 0,
            magnitude: 0,
            amount: 0,
            flags: 0,
            mana: 0,
            factor: 0.0,
            killed: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageResult {
    pub lethal: bool,
    pub flags: u32,
    pub dealt: i32,
}

pub(crate) const GROUP_PARALYSIS_EDGE: [u8; 4] = *b"inps";
const GROUP_PARALYSIS_TICK: [u8; 4] = *b"inpe";
pub(crate) const GROUP_HELD_START: [u8; 4] = *b"inhs";
pub(crate) const GROUP_HELD_END: [u8; 4] = *b"inhe";
pub(crate) const GROUP_LOW_VITALITY_START: [u8; 4] = *b"inss";
const GROUP_LOW_VITALITY_TICK: [u8; 4] = *b"inse";
const GROUP_LOW_VITALITY_HIT: [u8; 4] = *b"insi";
const GROUP_LOW_VITALITY_BODY: [u8; 4] = *b"insd";
const GROUP_STUNNED: [u8; 4] = *b"in01";
const LOW_VITALITY_FALLING: u8 = 0x15;
const LOW_VITALITY_DOWN: u8 = 0x16;
const PARALYSIS_ENTERING: u8 = 0x17;
const PARALYSIS_HELD: u8 = 0x18;
const PARALYSIS_LEAVING: u8 = 0x19;

fn paralysis_ticks(r: u16, modifier: i16) -> i16 {
    let base = ((i32::from(r) * 0xd2) >> 16) as i16 + 0x96;
    ((i32::from(base) * i32::from(modifier)) >> 8) as i16
}

fn group_at(raw: &[u8], o: usize) -> Option<FourCC> {
    let b: [u8; 4] = raw.get(o..o + 4)?.try_into().ok()?;
    (b != [0xff; 4]).then_some(FourCC::new(b))
}

impl EngineWorld {
    pub(crate) fn rename_monster(&mut self, v: usize) {
        let mut fresh: i16 = i16::MIN;
        while self.monsters.iter().any(|m| m.identifier == fresh) {
            fresh = fresh.wrapping_add(1);
        }
        let id = self.monsters[v].identifier;
        self.monsters[v].identifier = fresh;
        self.renamed_monsters.push((id, fresh));
    }

    pub fn effect_modifiers(&self, i: usize) -> Result<[i16; 9], Unmodelled> {
        let special = self.combat(i).special;
        if special != -1 {
            let raw = &self.catalog.artifacts[special as usize].1;
            let flags = be_i32(raw, 0) as u32;
            if !(self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_6
                && flags & 0x1_0000 != 0)
            {
                return Ok(std::array::from_fn(|k| be_i16(raw, 0x34 + 2 * k)));
            }
        }
        Ok(self.monster_definitions[self.monsters[i].definition].object_modifiers)
    }

    pub fn kill_definition(&self, v: usize) -> usize {
        let c = self.combat(v);
        if c.carried != -1
            && c.special != -1
            && be_i32(&self.catalog.artifacts[c.special as usize].1, 0) & 0x200 != 0
        {
            return self.own_definition(v);
        }
        self.monsters[v].definition
    }

    pub fn hit_effect_modifiers(&self, i: usize) -> Result<[i16; 9], Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let special = self.combat(i).special;
        let own = &self.monster_definitions[self.monsters[i].definition];
        if special != -1 {
            let raw = &self.catalog.artifacts[special as usize].1;
            let flags = be_i32(raw, 0) as u32;
            let mut passed = false;
            if gameplay > 0x708 && flags & 4 != 0 {
                let monster = be_i32(raw, 0x04);
                let t = FourCC::new([raw[4], raw[5], raw[6], raw[7]]);
                passed = flags & 0x10 == 0
                    || monster == -1
                    || t == self.monster_definitions[self.own_definition(i)].tag;
                if !passed && flags & 0x100 != 0 {
                    let Some(theirs) = self.monster_definitions.iter().find(|d| d.tag == t) else {
                        return Err(Unmodelled::new(
                            draw_site::HIT_EFFECT_MONSTER_NOT_LOADED,
                            format!(
                                "monster {}: the artifact's monster {t} isn't loaded",
                                self.monsters[i].identifier
                            ),
                        ));
                    };
                    passed = theirs.ai.raw.get(4..8) == own.ai.raw.get(4..8);
                }
            }
            if !passed && !(gameplay > game_options::GAMEPLAY_1_6 && flags & 0x1_0000 != 0) {
                return Ok(std::array::from_fn(|k| be_i16(raw, 0x34 + 2 * k)));
            }
        }
        Ok(own.object_modifiers)
    }

    pub fn damage_monster_object(
        &mut self,
        i: usize,
        damage: &DamageDefinition,
        amount: i32,
        chance: u16,
        rng: &mut EngineRng,
    ) -> Result<DamageResult, Unmodelled> {
        let m = self.effect_modifiers(i)?;
        let def = &self.monster_definitions[self.monsters[i].definition];
        let threshold = i32::from(def.object_threshold);
        let resists_healing = def.flags & 0x2_0000 != 0;
        let f = damage.flags;
        let ecx = f & 0x10 != 0 && m[5] != 0;
        let esi = f & 0x8000 != 0 && m[8] != 0;
        let edi = f & 0x20 != 0;
        let out = |lethal: bool, dealt: i32, r10: bool, r11: bool| DamageResult {
            lethal,
            flags: u32::from(ecx)
                | (u32::from(r11) << 1)
                | (u32::from(esi) << 2)
                | (u32::from(edi) << 3)
                | (u32::from(r10) << 4),
            dealt,
        };
        let t = damage.damage_type as u16;
        let scaled = |w: i16| (i32::from(w) * amount) >> 8;
        let mut r10 = true;
        let mut r11 = false;
        let d = match t {
            0 => Some(scaled(m[2])),
            1 | 9 => Some(scaled(m[4])),
            8 => Some(scaled(m[3])),
            10 => Some(scaled(m[7])),
            11 => {
                r10 = m[8] != 0;
                None
            }
            _ => {
                r11 = rng.draw(draw_site::DAMAGE_MONSTER_OBJECT_CHANCE) < chance;
                if amount == 0 {
                    r10 = false;
                    None
                } else {
                    let d = scaled(m[usize::from((f >> 3) & 1)]);
                    if d == 0 {
                        r10 = false;
                        r11 = true;
                        None
                    } else {
                        Some(d)
                    }
                }
            }
        };
        let d = match d {
            None if threshold >= 0 => return Ok(out(false, 0, r10, true)),
            None => 0,
            Some(d) => {
                if d > threshold || d < 0 {
                    let heal = if d > threshold { r10 && d < 0 } else { r10 };
                    r10 = false;
                    if heal && resists_healing {
                        return Ok(out(false, 0, false, true));
                    }
                } else {
                    return Ok(out(false, 0, false, true));
                }
                d
            }
        };
        if r11 {
            return Ok(out(false, 0, r10, true));
        }
        let o = &mut self.monsters[i].object;
        let hp = i32::from(o.vitality as i16);
        let d = d.min(hp);
        if f & 0x80 != 0 {
            let max = i32::from(o.max_vitality as i16) - d;
            let max = if 0xffff - d <= 0x1fffe {
                let v = max.min(0x7fff);
                if v < 0 { 0 } else { v }
            } else {
                0x7fff
            };
            o.max_vitality = max;
        }
        let new = hp - d;
        let (lethal, mut ebp) = if 0xffff - d <= 0x1fffe {
            if new < 0 {
                (true, 0)
            } else {
                let v = new.min(0x7fff);
                (v as i16 <= 0, v)
            }
        } else {
            (false, 0x7fff)
        };
        o.vitality = ebp;
        let max = o.max_vitality as i16 as i32;
        if max < ebp {
            ebp = max;
            o.vitality = ebp;
        }
        Ok(out(lethal, d, r10, false))
    }
}

impl WorldHooks<'_, '_> {
    pub(crate) fn cause_damage(
        &mut self,
        mv: Option<&mut EngineMover>,
        damage: &DamageDefinition,
        info: &mut DamageInfo,
    ) -> Result<(), Unmodelled> {
        if damage.flags & 0x1002 != 0 {
            return Err(Unmodelled::new(
                draw_site::DAMAGE_AREA_POINT,
                "cause_damage on an area or with a delayed explosion",
            ));
        }
        if self.world.monsters[self.monster].object.no_shape {
            return Ok(());
        }
        info.scale = 0x100;
        self.damage_amount(damage, info);
        if damage.flags & 0x100 != 0 {
            return Ok(());
        }
        self.accelerate_monster(mv, damage, info)
    }

    fn damage_amount(&mut self, damage: &DamageDefinition, info: &mut DamageInfo) {
        let delta = i32::from(damage.damage_delta);
        let extra = if delta > 0 {
            (u32::from(self.rng.draw(draw_site::DAMAGE_AMOUNT)) * delta as u32) >> 16
        } else {
            0
        };
        let mut ax = (extra.wrapping_add(u32::from(damage.damage_lower_bound as u16))) as i16;
        info.amount = ax;
        if ax != 0 && info.factor != 0.0 {
            let v = f32::from(ax) * info.factor;
            ax = if v.abs() < 2_147_483_648.0 {
                v as i32 as i16
            } else {
                0
            };
            info.amount = ax;
        }
        if damage.flags & 1 != 0 {
            let sum = info.velocity.iter().fold(0u32, |s, &c| {
                s.wrapping_add((i32::from(c) * i32::from(c)) as u32)
            });
            let m = i32::from(crate::projectiles::isqrt(sum) as i16);
            info.scale = ((m * i32::from(info.scale)) >> 8) as i16;
        }
        ax = ((i32::from(info.scale) * i32::from(ax)) >> 8) as i16;
        info.amount = ax;
        if damage.flags & 0x1000 == 0 {
            let d = ((i32::from(damage.word_0e) * i32::from(ax)) >> 12) as i16;
            info.magnitude = d.min(0x100);
        }
    }

    pub(crate) fn panic_monster(&mut self, mv: Option<&mut EngineMover>) -> Result<(), Unmodelled> {
        let v = self.monster;
        let gameplay = self.world.formation_globals.gameplay_version;
        let id = self.world.monsters[v].identifier;
        self.world.rename_monster(v);
        let m = &mut self.world.monsters[v];
        m.flags_8 |= 0x4000;
        m.flags &= if gameplay > game_options::GAMEPLAY_1_7 {
            0xde3f_fff0
        } else {
            0xde7f_fff0
        };
        self.world.combat_mut(v).target = -1;
        let m = &self.world.monsters[v];
        if gameplay > game_options::GAMEPLAY_1_5
            && (m.identifier == 0 || m.flags & 0x4000_0000 != 0)
        {
            return Ok(());
        }
        self.world.combat_mut(v).pending = -1;
        if !(11..=14).contains(&self.world.monsters[v].mode) {
            let r = u32::from(self.rng.draw(draw_site::PANIC));
            let state = 3 + ((r * 3) >> 16) as i8;
            let Some(mv) = mv else {
                return Err(Unmodelled::new(
                    draw_site::PANIC_NO_MOVER,
                    format!("monster {id}: the panic's taunt without its mover"),
                ));
            };
            mv.set_mode(MoveMode::Taunting, ModeState::Value(state), self)?;
        }
        let winner = self.world.game.winner;
        let m = &mut self.world.monsters[v];
        if m.flags_8 & 0x4_0000 != 0 && winner == -1 {
            m.flags_8 &= !0x4_0000;
        }
        Ok(())
    }

    fn killer_taunt(&mut self, a: usize) -> Result<(), Unmodelled> {
        let gameplay = self.world.formation_globals.gameplay_version;
        let m = &self.world.monsters[a];
        if gameplay > game_options::GAMEPLAY_1_5
            && (m.identifier == 0 || m.flags & 0x4000_0000 != 0)
        {
            return Ok(());
        }
        let id = m.identifier;
        self.world.combat_mut(a).pending = -1;
        if !(11..=14).contains(&self.world.monsters[a].mode) {
            let r = u32::from(self.rng.draw(draw_site::PANIC));
            let state = 3 + ((r * 3) >> 16) as i8;
            let Some(mut mv) = self.world.movers.get_mut(a).and_then(Option::take) else {
                return Err(Unmodelled::new(
                    draw_site::PANIC_NO_MOVER,
                    format!("monster {id}: the killer's taunt without its mover"),
                ));
            };
            let done = {
                let mut h = self.for_monster(a);
                mv.set_mode(MoveMode::Taunting, ModeState::Value(state), &mut h)
            };
            self.world.movers[a] = Some(mv);
            done?;
        }
        let winner = self.world.game.winner;
        let m = &mut self.world.monsters[a];
        if m.flags_8 & 0x4_0000 != 0 && winner == -1 {
            m.flags_8 &= !0x4_0000;
        }
        Ok(())
    }

    fn release_on_hold(&mut self, mv: Option<&mut EngineMover>) -> Result<(), Unmodelled> {
        let v = self.monster;
        let st = self.world.monsters[v].anim_state as i8;
        if st != -1
            && let Some(mv) = mv
        {
            self.attack(mv, i16::from(st), 2)?;
        }
        let gameplay = self.world.formation_globals.gameplay_version;
        if gameplay <= game_options::GAMEPLAY_1_3 {
            return Ok(());
        }
        let m = &mut self.world.monsters[v];
        if gameplay > game_options::GAMEPLAY_1_6 && m.flags_8 & 0x4_0000 != 0 {
            m.flags &= if gameplay < 0x69b { !0x18 } else { !0x19 };
        }
        m.flags_8 &= !0x4_0000;
        Ok(())
    }

    fn conversion_player(&self, info: &DamageInfo) -> Result<Conversion, Unmodelled> {
        let w = &self.world;
        let v = self.monster;
        let gameplay = w.formation_globals.gameplay_version;
        let Some(row) = usize::try_from(info.unit.0)
            .ok()
            .and_then(|u| w.statistics.units.get(u))
        else {
            return Err(Unmodelled::new(
                draw_site::CONVERSION_UNIT_ROW,
                format!(
                    "monster {}: a conversion by unit row {}",
                    w.monsters[v].identifier, info.unit.0
                ),
            ));
        };
        let mut player = row.owner;
        if gameplay > game_options::GAMEPLAY_1_6 && player == -1 {
            let Ok(a) = usize::try_from(info.attacker) else {
                return Ok(Conversion::None);
            };
            player = w.monsters.get(a).map_or(-1, |m| m.owner);
        }
        if player == -1 || player == w.monsters[v].owner {
            return Ok(Conversion::None);
        }
        if gameplay <= game_options::GAMEPLAY_1_5 {
            return Ok(Conversion::Change(player));
        }
        if gameplay > game_options::GAMEPLAY_1_6 {
            let own = usize::try_from(w.statistics.monster_unit(v))
                .ok()
                .and_then(|u| w.statistics.units.get(u));
            let Some(own) = own else {
                return Err(Unmodelled::new(
                    draw_site::CONVERSION_NO_UNIT_ROW,
                    format!(
                        "monster {}: a conversion of a monster without a unit row",
                        w.monsters[v].identifier
                    ),
                ));
            };
            if w.statistics.find_unit(player, own.tag).is_some() {
                return Ok(Conversion::Change(player));
            }
        }
        let definition = w.own_definition(v) as i16;
        if w.statistics
            .units
            .iter()
            .any(|u| u.owner == player && u.mons == definition)
        {
            Ok(Conversion::Change(player))
        } else {
            Ok(Conversion::NoRow)
        }
    }

    pub(crate) fn change_monster_owner(
        &mut self,
        mv: Option<&mut EngineMover>,
        player: i16,
        rename: bool,
    ) -> Result<(), Unmodelled> {
        let v = self.monster;
        let id = self.world.monsters[v].identifier;
        if rename {
            self.world.rename_monster(v);
        }
        self.call_monster_died_procs(2)?;
        self.world.detach_monster(v, player)?;
        self.world.combat_mut(v).target = -1;
        let gameplay = self.world.formation_globals.gameplay_version;
        let m = &mut self.world.monsters[v];
        m.flags_8 &= 0xf7ff_7ddf;
        m.flags = (m.flags & 0xde7e_ffe0) | 0x8000;
        let mode = m.mode;
        if (11..=14).contains(&mode) && gameplay > game_options::GAMEPLAY_1_5 {
            return Ok(());
        }
        let Some(mv) = mv else {
            return Err(Unmodelled::new(
                draw_site::CHANGE_OWNER_NO_MOVER,
                format!("monster {id}: change_monster_owner without its mover"),
            ));
        };
        self.finish_action(mv)
    }

    fn confuse(&mut self, mv: Option<&mut EngineMover>, mana: i16, dealt: i32) {
        let v = self.monster;
        let gameplay = self.world.formation_globals.gameplay_version;
        let ticks = if gameplay <= game_options::GAMEPLAY_1_3 || mana != 0 {
            ((i32::from(mana) * 0x1e) >> 8) / 5
        } else {
            dealt.wrapping_mul(0x1e) >> 8
        };
        self.world.combat_mut(v).confusion = ticks as i16;
        self.world.combat_mut(v).target = -1;
        let m = &mut self.world.monsters[v];
        m.flags_8 = (m.flags_8 & 0xf7ff_7ddf) | 0x108;
        m.flags = (m.flags & 0xde7e_ff40) | 0x8000;
        let clear = |mv: &mut EngineMover| {
            mv.waypoints.clear();
            mv.path = None;
        };
        match mv {
            Some(mv) => clear(mv),
            None => {
                if let Some(Some(mv)) = self.world.movers.get_mut(v) {
                    clear(mv);
                }
            }
        }
        let local_group = crate::combat::CONFUSION_LOCAL_GROUP;
        self.world.combat_mut(v).local_group = Some(local_group);
        let object = &self.world.monsters[v].object;
        let event = crate::CombatEvent::MonsterLocalGroup {
            tick: 0,
            monster: v as u32,
            local_group,
            object: object.index,
            identifier: object.identifier,
        };
        self.world.group_events.push(event);
    }

    pub(crate) fn accelerate_monster(
        &mut self,
        mut mv: Option<&mut EngineMover>,
        damage: &DamageDefinition,
        info: &mut DamageInfo,
    ) -> Result<(), Unmodelled> {
        let v = self.monster;
        let id = self.world.monsters[v].identifier;
        let def =
            HitMonsterDef::of(&self.world.monster_definitions[self.world.monsters[v].definition]);
        self.world.combat_mut(v).attacker = info.attacker;
        let t = damage.damage_type as u16;
        let mut s14 = false;
        let mut s48 = None;
        match t {
            5 | 7 => {
                s14 = true;
                s48 = def.group_1a0;
            }
            2 => {
                let m = &self.world.monsters[v];
                if m.mode == 12 {
                    return Ok(());
                }
                if m.mode == 8 {
                    self.release_on_hold(mv.as_deref_mut())?;
                }
                let Some(mv) = mv.as_deref_mut() else {
                    return Err(Unmodelled::new(
                        draw_site::DAMAGE_TYPE_2_NO_MOVER,
                        format!("monster {id}: damage type 2 without its mover"),
                    ));
                };
                mv.set_mode(MoveMode::Stunned, ModeState::Value(-1), self)?;
                mv.pose_timer = ((i32::from(info.amount) * 0x1e) >> 8) as i16;
                info.flags |= 1;
                info.group = None;
                return Ok(());
            }
            3 => {
                if info.amount < 0 {
                    let att = info.attacker;
                    if att < 0
                        || self.world.monsters.get(att as usize).is_none_or(|a| {
                            a.identifier != info.attacker_id
                                || a.team != self.world.monsters[v].team
                        })
                    {
                        return Ok(());
                    }
                }
                let gameplay = self.world.formation_globals.gameplay_version;
                if gameplay == 0x69f && def.flags & 0x2_0000 != 0 {
                    return Ok(());
                }
                if self.world.monsters[v].mode == 8 {
                    self.release_on_hold(mv.as_deref_mut())?;
                }
                let Some(mv) = mv.as_deref_mut() else {
                    return Err(Unmodelled::new(
                        draw_site::DAMAGE_TYPE_3_NO_MOVER,
                        format!("monster {id}: damage type 3 without its mover"),
                    ));
                };
                mv.set_mode(MoveMode::Held, ModeState::Value(-1), self)?;
                let held_unit =
                    if let Some(att) = (info.attacker >= 0).then_some(info.attacker as usize) {
                        self.world.statistics.monster_unit(att)
                    } else {
                        -1
                    };
                self.world.combat_mut(v).held_unit = held_unit;
                info.flags |= 1;
                info.group = None;
                return Ok(());
            }
            6 => {
                if self.world.monsters[v].mode == 13 || self.world.hit_effect_modifiers(v)?[6] == 0
                {
                    return Ok(());
                }
                let Some(mv) = mv.as_deref_mut() else {
                    return Err(Unmodelled::new(
                        draw_site::DAMAGE_TYPE_6_NO_MOVER,
                        format!("monster {id}: damage type 6 without its mover"),
                    ));
                };
                mv.set_mode(
                    MoveMode::LowVitality,
                    ModeState::Value(LOW_VITALITY_FALLING as i8),
                    self,
                )?;
                info.flags |= 1;
                info.group = None;
                return Ok(());
            }
            _ => {}
        }
        let m = &self.world.monsters[v];
        let max = i32::from(m.object.max_vitality as i16);
        let mut amount = i32::from(info.amount);
        let ratio = crate::combat::hit_ratio(amount, max);
        let mut mode = m.mode;
        if mode == 9 && m.flags & 8 != 0 && self.world.combat(v).target == info.attacker {
            s48 = def.group_1e4;
            if damage.flags & 0x40 == 0 {
                info.flags |= 1;
                info.group = s48;
                return Ok(());
            }
            amount >>= 1;
            s14 = true;
        }
        let s28 = self.world.game_type == 12 && m.flags_8 & 0x2000 != 0;
        if m.flags_8 & 4 != 0 {
            let m = &mut self.world.monsters[v];
            m.object.flags &= 0xcfdf_ffff;
            m.flags_8 &= 0xffff_ffe3;
        }
        let m = &self.world.monsters[v];
        if self.world.formation_globals.gameplay_version <= game_options::GAMEPLAY_1_3
            || m.flags_8 & 0x200_0000 == 0
        {
            if mode == 13 {
                amount = amount.wrapping_mul(2);
                s14 = true;
                s48 = Some(FourCC::new(GROUP_LOW_VITALITY_HIT))
                    .filter(|g| self.world.catalog.groups.contains_key(g));
            } else if def.flags & 0x100 != 0 && (m.object.vitality as i16) < ((max as i16) >> 2) {
                let Some(mv) = mv.as_deref_mut() else {
                    return Err(Unmodelled::new(
                        draw_site::MODE_13_NO_MOVER,
                        format!("monster {id}: mode 13 without its mover"),
                    ));
                };
                mv.set_mode(
                    MoveMode::LowVitality,
                    ModeState::Value(LOW_VITALITY_FALLING as i8),
                    self,
                )?;
                mode = 13;
            }
        }
        let m = &self.world.monsters[v];
        if self.world.game_options & mandrake_common::game_options::COOPERATIVE != 0
            && m.owner != -1
            && m.flags & 0x1000_0000 == 0
        {
            let human = self
                .world
                .players
                .players
                .get(m.owner as usize)
                .is_none_or(|p| p.field_09 == 0);
            if human {
                match self.world.difficulty {
                    0 => amount >>= 1,
                    1 => amount = (amount * 3) / 4,
                    _ => {}
                }
            }
        }
        if amount > 0 && m.flags_8 & 0x80_0000 != 0 {
            amount = 0;
        }
        let chance = def.w6c as u16;
        let r = self
            .world
            .damage_monster_object(v, damage, amount, chance, self.rng)?;
        let lethal = r.lethal;
        let f58 = r.flags;
        let mut dealt = r.dealt;
        if !lethal {
            if f58 & 1 != 0 && !(11..=14).contains(&mode) {
                let Some(mv) = mv.as_deref_mut() else {
                    return Err(Unmodelled::new(
                        draw_site::MODE_14_NO_MOVER,
                        format!("monster {id}: mode 14 without its mover"),
                    ));
                };
                mv.set_mode(MoveMode::Paralyzed, ModeState::Value(0x17), self)?;
                mode = 14;
            }
            if f58 & 2 != 0 {
                s14 = true;
                s48 = def.group_1e8;
            } else if dealt == 0 {
                s48 = None;
            }
            if !info.unit.is_none() && f58 & 0x10 != 0 {
                match self.conversion_player(info)? {
                    Conversion::Change(player) => {
                        self.change_monster_owner(mv.as_deref_mut(), player, true)?;
                        s14 = false;
                    }
                    Conversion::NoRow => {
                        if self.world.formation_globals.gameplay_version
                            <= game_options::GAMEPLAY_1_6
                        {
                            s14 = false;
                        }
                    }
                    Conversion::None => {}
                }
            }
            if f58 & 4 != 0 && self.world.monsters[v].flags_8 & 0x100 == 0 {
                self.confuse(mv.as_deref_mut(), info.mana, dealt);
            }
        }
        let mut s18 = false;
        if (ratio >= def.w19e as u16 || f58 & 8 != 0)
            && !(12..=14).contains(&mode)
            && (dealt != 0 || f58 & 2 != 0)
        {
            if mode == 11 {
                s18 = (self.world.monsters[v].flags & 0x0400_0000 == 0)
                    && (damage.flags & 0x4000 == 0);
            } else {
                s18 = damage.flags & 0x4000 == 0;
            }
        }
        let knockback = def.flags & 0x80_0000 != 0
            && (def.w44 as u16 <= ratio || f58 & 8 != 0)
            && !(12..=14).contains(&mode)
            && (f58 & 2 != 0 || dealt != 0)
            && (t & 0xfffa == 0 || t == 8)
            && !(mode == 11 && self.world.monsters[v].flags & 0x400_0000 != 0)
            && damage.flags & 0x4000 == 0;
        if knockback {
            return Err(Unmodelled::new(
                draw_site::KNOCKBACK,
                format!("monster {id}: the knockback"),
            ));
        }
        let m = &self.world.monsters[v];
        if m.flags_8 & 0x4000 == 0 {
            let w218 = def.w218 as u16;
            let w21a = def.w21a as u16;
            let max = i32::from(m.object.max_vitality as i16);
            let low = w21a != 0
                && max != 0
                && (i32::from(m.object.vitality as i16) << 16) / max < i32::from(w21a);
            if (w218 != 0 && ratio > w218) || low {
                self.panic_monster(mv.as_deref_mut())?;
                mode = self.world.monsters[v].mode;
            }
        }
        if s18 && mode == 8 {
            let st = self.world.monsters[v].anim_state as i8;
            if st != -1 {
                let atk = self.world.attack_definition(v, i16::from(st))?;
                let flags = atk.map_or(0, |a| a.flags);
                if flags & 0x100 != 0 || damage.flags & 0x4 != 0 {
                    s18 = false;
                } else {
                    let Some(mv) = mv.as_deref_mut() else {
                        return Err(Unmodelled::new(
                            draw_site::HIT_RELEASE_NO_MOVER,
                            format!("monster {id}: the release on a hit without its mover"),
                        ));
                    };
                    self.attack(mv, i16::from(st), 2)?;
                    let m = &mut self.world.monsters[v];
                    if m.flags_8 & 0x4_0000 != 0 {
                        m.flags &= !0x19;
                    }
                    m.flags_8 &= !0x4_0000;
                }
            }
        }
        let m = &self.world.monsters[v];
        let a = usize::try_from(info.attacker).ok().filter(|&a| {
            (lethal || m.flags_8 & 0x4100 == 0)
                && self
                    .world
                    .monsters
                    .get(a)
                    .is_some_and(|x| x.identifier != 0 && x.identifier == info.attacker_id)
        });
        if let Some(a) = a {
            if !s28 && self.world.monsters[a].team == m.team {
                let attacker_speaks = self.rng.draw(draw_site::HIT_SPEAKER) & 1 != 0;
                let (speaker, kind) = if attacker_speaks {
                    (a, if lethal { 14 } else { 7 })
                } else {
                    (v, 8)
                };
                if dealt >= 0 && (attacker_speaks || self.world.monsters[v].mode != 13) {
                    let mover = mv.as_deref().map(|m| (m.x, m.y));
                    self.world.monster_voice_in_hit(speaker, v, mover, kind);
                }
            } else if lethal {
                if def.flags & 0x8_0000 == 0
                    && self.world.monsters[a].flags_8 & 0x4000 != 0
                    && self.rng.draw(draw_site::HIT_VOICE_CHANCE) & 3 != 0
                {
                    self.killer_taunt(a)?;
                }
                if def.flags & 0x8_0000 == 0 {
                    let kind = if def.flags & 0x400 != 0 { 15 } else { 13 };
                    let mover = mv.as_deref().map(|m| (m.x, m.y));
                    self.world.monster_voice_in_hit(a, v, mover, kind);
                }
                let xp = i32::from(def.w1ba).abs();
                let c = self.world.combat_mut(a);
                c.experience = (i32::from(c.experience) + xp).min(0xff) as u8;
                c.byte_8d |= 2;
            } else {
                let r8 = s18 || mode != 8;
                let gameplay = self.world.formation_globals.gameplay_version;
                if gameplay <= game_options::GAMEPLAY_1_6 {
                    return Err(Unmodelled::new(
                        draw_site::HIT_REACTION_OLD_GAMEPLAY,
                        format!("monster {id}: the hit reaction of gameplay {gameplay:#x}"),
                    ));
                }
                if damage.flags & 8 != 0 {
                    self.damage_flag_8_reaction(v, a, r8, damage, def.ai)?;
                } else {
                    self.hit_engagement(v, a, r8, damage, def.ai)?;
                }
            }
        }
        if dealt < 0 {
            dealt = 0;
        }
        self.world
            .hit_statistics(info.unit, info.attacker, v, dealt, lethal);
        if lethal {
            let mut code = if (def.w19c as u16) < ratio {
                0x27
            } else {
                0x26
            };
            let explosive = match t {
                9 => {
                    if def.group_3c.is_some() {
                        code = 0x29;
                    }
                    false
                }
                10 => {
                    code = 0x26;
                    false
                }
                0 | 1 | 8 => true,
                _ => false,
            };
            if (explosive || mode == 13) && self.world.monsters[v].flags_8 & 0x200_0000 == 0 {
                code = 0x27;
            }
            info.killed = Some(code);
            if s14 {
                info.flags |= 1;
                info.group = None;
            }
            return Ok(());
        }
        if s18 {
            let Some(mv) = mv else {
                return Err(Unmodelled::new(
                    draw_site::MODE_15_NO_MOVER,
                    format!("monster {id}: mode 15 without its mover"),
                ));
            };
            mv.set_mode(MoveMode::Flinching, ModeState::Value(0), self)?;
        }
        if s14 {
            info.flags |= 1;
            info.group = s48;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct HitAi {
    class: i16,
    engage_distance: i16,
    vision: i16,
    byte_198: u8,
}

#[derive(Clone, Copy)]
struct HitMonsterDef {
    flags: u32,
    group_1a0: Option<FourCC>,
    group_1e4: Option<FourCC>,
    group_1e8: Option<FourCC>,
    group_3c: Option<FourCC>,
    w6c: i16,
    w19e: i16,
    w44: i16,
    w218: i16,
    w21a: i16,
    w1ba: i16,
    w19c: i16,
    ai: HitAi,
}

impl HitMonsterDef {
    fn of(def: &crate::engine_rng::MonsterDefinition) -> Self {
        let raw = &def.ai.raw;
        Self {
            flags: def.flags,
            group_1a0: group_at(raw, 0x1a0),
            group_1e4: group_at(raw, 0x1e4),
            group_1e8: group_at(raw, 0x1e8),
            group_3c: group_at(raw, 0x3c),
            w6c: def.ai.word(0x6c),
            w19e: def.ai.word(0x19e),
            w44: def.ai.word(0x44),
            w218: def.ai.word(0x218),
            w21a: def.ai.word(0x21a),
            w1ba: def.ai.word(0x1ba),
            w19c: def.ai.word(0x19c),
            ai: HitAi {
                class: def.ai.class,
                engage_distance: def.ai.engage_distance,
                vision: def.ai.vision,
                byte_198: def.ai.byte_198,
            },
        }
    }
}

fn missile_drops_target(f4: u32, f8: u32) -> u32 {
    if f4 & 8 != 0 && f8 & 0x20 != 0 && f4 & 4 == 0 {
        if f4 & 0x10 != 0 { f4 & !8 } else { f4 & !9 }
    } else {
        f4
    }
}

fn class_1_flees_flag_8(dist: i32, vision: i32, options: u32, damage_type: u16) -> bool {
    dist > vision
        && (options
            & (mandrake_common::game_options::TFL | mandrake_common::game_options::ANTI_CLUMP)
            == 0
            || damage_type & 0xfff7 != 0)
}

impl EngineWorld {
    pub(crate) fn kill_monster_born_dead(
        &mut self,
        v: usize,
        marker: Option<&mandrake_tag::MarkerEntry>,
        r: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let m = &self.monsters[v];
        let def = &self.monster_definitions[self.kill_definition(v)];
        let at = |o: usize| -> Option<FourCC> {
            let b: [u8; 4] = def.ai.raw.get(o..o + 4)?.try_into().ok()?;
            (b != [0xff; 4]).then_some(FourCC::new(b))
        };
        let group = crate::combat::death_group([at(0x1a4), at(0x194), at(0x3c)], 0x27);
        let projectile = at(0x1ec);
        let mons_flags = def.flags;
        let object = m.object.index;
        let mut desc = GroupDescriptor {
            flags: kill_descriptor_mode_flags(m.flags_8, m.mode),
            group: group.unwrap_or(FourCC::new([0xff; 4])),
            shading_object: object,
            object,
            owner: -1,
            point: [m.position.0, m.position.1, m.object.z],
            magnitude: 0x28,
            velocity: [0; 3],
        };
        let body_13 = kill_body_in_mode_13(m.flags_8, m.mode);
        let count = i16::from(self.combat(v).ammunition as i8);
        let carried = self.combat(v).carried;
        let mut w = crate::projectile_groups::EngineSpawn {
            world: self,
            rng: r,
            source: None,
            host: None,
        };
        if group.is_some() {
            crate::projectile_groups::new_projectile_group(&mut w, &desc)?;
        }
        let insd = FourCC::new(GROUP_LOW_VITALITY_BODY);
        if body_13 && w.world.catalog.groups.contains_key(&insd) {
            crate::projectile_groups::new_projectile_group(
                &mut w,
                &GroupDescriptor {
                    group: insd,
                    ..desc
                },
            )?;
        }
        desc.magnitude = desc.magnitude.max(10);
        if drop_ammunition(&mut w, mons_flags, projectile, count, &desc, 0)? {
            self.combat_mut(v).ammunition = 0;
        }
        let mut hooks = WorldHooks {
            world: self,
            actions: &mut None,
            rng: r,
            monster: v,
            time: 0,
            host: &mut crate::engine_attack::NoHost,
        };
        hooks.kill_inventory(&desc)?;
        if carried != -1 {
            hooks.kill_carried_object(&desc)?;
        }
        hooks.delete_or_make_again(marker.cloned())
    }

    pub(crate) fn update_fade_record(
        &mut self,
        index: i32,
    ) -> Option<crate::engine_rng::FadeRecord> {
        let at = self.fade_records.iter().position(|r| r.index == index)?;
        let collection = self.fade_records[at].object.collection;
        let headers = self
            .collections
            .get(&collection)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let object = &mut self.fade_records[at].object;
        crate::engine_rng::animate_object(object, headers);
        if object.anim_flags & 8 != 0 {
            let r = self.fade_records.remove(at);
            self.free_objects.insert(r.object.index);
            return Some(r);
        }
        None
    }

    pub(crate) fn duplicate_flags(&self, i: usize) -> u32 {
        let m = &self.monsters[i];
        (m.object.flags | if m.mirrored { 0x2_0000 } else { 0 }) & 0x3ffe_ffff
    }

    pub(crate) fn monster_marker(&self, i: usize) -> Option<mandrake_tag::MarkerEntry> {
        let id = self.monsters[i].identifier;
        let mut m = self
            .marker_creation
            .as_ref()?
            .mesh
            .markers
            .iter()
            .find(|m| m.marker_type == 3 && m.marker_id == id)?
            .clone();
        if let Some(p) = self.marker_places.get(&id) {
            (m.x, m.y, m.z, m.yaw, m.pitch) = (p.x, p.y, p.z, p.yaw, p.pitch);
        }
        Some(m)
    }

    pub(crate) fn made_again(&self, i: usize, marker_flags: u32) -> bool {
        let own = self.monsters[i].flags_8 & 0x1000 != 0;
        if marker_flags & 0x20 != 0 {
            return !own || self.formation_globals.gameplay_version > 0x69f;
        }
        self.game_options & mandrake_common::game_options::DEATHMATCH != 0 && !own
    }

    pub(crate) fn delete_parasite(&mut self, i: usize) {
        if let Some(at) = self.fade_records.iter().position(|r| r.host == i) {
            let r = self.fade_records.remove(at);
            self.free_objects.insert(r.object.index);
        }
    }

    pub(crate) fn new_fade_record_index(&self) -> i32 {
        let mut index = 0;
        while self.projectile_records.iter().any(|p| p.index == index)
            || self.fade_records.iter().any(|r| r.index == index)
            || self.reserved_records.contains(&index)
        {
            index += 1;
        }
        index
    }
}

impl WorldHooks<'_, '_> {
    pub(crate) fn internal_group(
        &mut self,
        mv: &mut EngineMover,
        tag: [u8; 4],
    ) -> Result<(), Unmodelled> {
        let group = FourCC::new(tag);
        if !self.world.catalog.groups.contains_key(&group) {
            return Ok(());
        }
        let i = self.monster;
        let source = crate::projectile_groups::EngineSpawn::monster_object(
            self.world,
            i,
            (mv.x, mv.y),
            mv.facing,
        );
        let desc = GroupDescriptor {
            flags: 0,
            group,
            shading_object: source.index,
            object: source.index,
            owner: i as i16,
            point: source.position,
            magnitude: 0,
            velocity: [0; 3],
        };
        let mut w = crate::projectile_groups::EngineSpawn {
            world: &mut *self.world,
            rng: &mut *self.rng,
            source: Some(source),
            host: Some(crate::projectile_groups::StageHost {
                host: &mut *self.host,
                actions: &mut *self.actions,
                time: self.time,
                launcher: (i, mv),
            }),
        };
        crate::projectile_groups::new_projectile_group(&mut w, &desc)
    }

    pub(crate) fn fade_parasite(&mut self, shading: u8, ticks: u8) -> Result<(), Unmodelled> {
        let i = self.monster;
        let gameplay = self.world.formation_globals.gameplay_version;
        if gameplay <= 0x6b5 {
            return Err(Unmodelled::new(
                draw_site::FADE_PARASITE_OLD_GAMEPLAY,
                format!("fade_object of gameplay {gameplay:#x}"),
            ));
        }
        self.world.delete_parasite(i);
        let (index, identifier) = self.world.new_object_datum();
        if self.world.monsters[i].object.no_shape {
            self.world.free_objects.insert(index);
        } else {
            let record = self.world.new_fade_record_index();
            let mut object = self.world.monsters[i].object.clone();
            object.index = index;
            object.identifier = identifier;
            object.flags = self.world.duplicate_flags(i) | 0x20_0000;
            object.sub_kind = 1;
            object.sub_count = 0;
            object.sub_limit = ticks;
            object.kind = 5;
            object.owner = record as i16;
            self.world.fade_records.push(crate::engine_rng::FadeRecord {
                index: record,
                host: i,
                object,
                position: self.world.monsters[i].position,
                facing: self.world.monsters[i].facing,
                cell_link: 0,
                countdown: 0,
                group: None,
                monster: None,
            });
        }
        self.world.combat_mut(i).byte_8d |= 0x20;
        self.world.monsters[i].object.shading_slot = shading;
        Ok(())
    }

    pub(crate) fn low_vitality_handler(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        if self.world.monsters[i].anim_state != LOW_VITALITY_FALLING {
            return Ok(());
        }
        self.internal_group(mv, GROUP_LOW_VITALITY_TICK)?;
        if mv.pose_timer < 0 {
            self.world.monsters[i].anim_state = LOW_VITALITY_DOWN;
        }
        Ok(())
    }

    pub(crate) fn stunned_handler(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        self.internal_group(mv, GROUP_STUNNED)?;
        if mv.pose_timer >= 0 {
            return Ok(());
        }
        let slot = if self.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1 {
            self.world.monsters[i].object.shading_slot
        } else {
            0
        };
        match slot {
            1 => mv.set_mode(
                MoveMode::LowVitality,
                ModeState::Value(LOW_VITALITY_DOWN as i8),
                self,
            ),
            2 => mv.set_mode(
                MoveMode::Paralyzed,
                ModeState::Value(PARALYSIS_HELD as i8),
                self,
            ),
            _ => self.finish_action(mv),
        }
    }

    pub(crate) fn paralysis_handler(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let i = self.monster;
        let modifier = self.world.effect_modifiers(i)?[5];
        self.internal_group(mv, GROUP_PARALYSIS_TICK)?;
        if mv.pose_timer >= 0 {
            return Ok(());
        }
        match self.world.monsters[i].anim_state {
            PARALYSIS_ENTERING => {
                self.world.monsters[i].anim_state = PARALYSIS_HELD;
                let r = self.rng.draw(draw_site::PARALYSIS);
                mv.pose_timer = paralysis_ticks(r, modifier);
            }
            PARALYSIS_HELD => {
                self.fade_parasite(0, 0x1e)?;
                self.internal_group(mv, GROUP_PARALYSIS_EDGE)?;
                self.world.monsters[i].anim_state = PARALYSIS_LEAVING;
                mv.pose_timer = 0x1e;
            }
            PARALYSIS_LEAVING => self.finish_action(mv)?,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn held_handler(&mut self, mv: &mut EngineMover) -> Result<(), Unmodelled> {
        let v = self.monster;
        let (held_vitality_multiplier, xp_word, death_group) = {
            let def = &self.world.monster_definitions[self.world.monsters[v].definition];
            let raw = &def.ai.raw;
            (
                def.held_vitality_multiplier,
                def.ai.word(0x1ba),
                group_at(raw, 0x1a4).or_else(|| group_at(raw, 0x194)),
            )
        };
        let gameplay = self.world.formation_globals.gameplay_version;
        let held_unit = if gameplay > 0x71e {
            self.world.combat(v).held_unit
        } else {
            let att = self.world.combat(v).attacker;
            if att >= 0 {
                self.world.statistics.monster_unit(att as usize)
            } else {
                -1
            }
        };

        if held_vitality_multiplier != 0 {
            let max_v = self.world.monsters[v].object.max_vitality;
            let target_v = (i32::from(held_vitality_multiplier) * max_v) >> 8;
            let cur_v = self.world.monsters[v].object.vitality;
            if self.world.monsters[v].flags & 0x0400_0000 != 0 || cur_v >= target_v {
                self.internal_group(mv, GROUP_HELD_END)?;
                if mv.pose_timer < 0 {
                    self.finish_action(mv)?;
                    self.world.combat_mut(v).held_unit = -1;
                    if gameplay > game_options::GAMEPLAY_1_6
                        && self.world.monsters[v].object.ticks_per_frame == 0
                    {
                        let col = self.world.monsters[v].object.collection;
                        let seq = self.world.monsters[v].object.sequence as usize;
                        let tpf = self
                            .world
                            .headers(col)
                            .get(seq)
                            .map_or(1, |h| h.ticks_per_frame.max(1))
                            as i8;
                        self.world.monsters[v].object.ticks_per_frame = tpf;
                        mv.ticks_per_frame = tpf;
                    }
                }
            } else {
                let inc = target_v / 75;
                let new_v = (cur_v + inc).min(target_v);
                self.world.monsters[v].object.vitality = new_v;
                self.internal_group(mv, GROUP_HELD_END)?;
            }
        } else {
            let mut cur_v = self.world.monsters[v].object.vitality;
            if cur_v >= 0 {
                let max_v = self.world.monsters[v].object.max_vitality;
                let dec = max_v / 75;
                if self.world.game_options & mandrake_common::game_options::TFL == 0 {
                    let f8 = self.world.monsters[v].flags_8;
                    self.world.adjust_damage_statistics(
                        crate::statistics::UnitRef(held_unit),
                        v,
                        cur_v.min(dec) as i16,
                        false,
                        0,
                        f8 & 0x1000,
                        true,
                    );
                }
                cur_v -= dec;
                self.world.monsters[v].object.vitality = if cur_v < 0 { -1 } else { cur_v };
                self.internal_group(mv, GROUP_HELD_END)?;
            } else {
                self.internal_group(mv, GROUP_HELD_END)?;
                if mv.pose_timer < 0 {
                    let att = self.world.combat(v).attacker;
                    let att_alive = att >= 0
                        && usize::try_from(att).ok().is_some_and(|a| {
                            a < self.world.monsters.len() && self.world.monsters[a].identifier != 0
                        });
                    if let Some(a) = usize::try_from(att)
                        .ok()
                        .filter(|_| att_alive)
                        .filter(|&a| self.world.monsters[a].team != self.world.monsters[v].team)
                    {
                        let xp = i32::from(xp_word).abs();
                        let c = self.world.combat_mut(a);
                        c.experience = (i32::from(c.experience) + xp).min(0xff) as u8;
                        c.byte_8d |= 2;
                    }
                    if let Some(group) =
                        death_group.filter(|group| self.world.catalog.groups.contains_key(group))
                    {
                        let source = crate::projectile_groups::EngineSpawn::monster_object(
                            self.world,
                            v,
                            (mv.x, mv.y),
                            mv.facing,
                        );
                        let desc = GroupDescriptor {
                            flags: 0x10,
                            group,
                            shading_object: source.index,
                            object: source.index,
                            owner: -1,
                            point: source.position,
                            magnitude: 0,
                            velocity: [0; 3],
                        };
                        let mut w = crate::projectile_groups::EngineSpawn {
                            world: &mut *self.world,
                            rng: &mut *self.rng,
                            source: Some(source),
                            host: Some(crate::projectile_groups::StageHost {
                                host: &mut *self.host,
                                actions: &mut *self.actions,
                                time: self.time,
                                launcher: (v, &mut *mv),
                            }),
                        };
                        crate::projectile_groups::new_projectile_group(&mut w, &desc)?;
                    }
                    if !self.world.combat(v).inventory.is_empty() {
                        return Err(inventory_stop(self.world.monsters[v].identifier));
                    }
                    self.kill_monster()?;
                }
            }
        }
        Ok(())
    }

    fn hit_engagement(
        &mut self,
        v: usize,
        a: usize,
        mut r8: bool,
        damage: &DamageDefinition,
        ai: HitAi,
    ) -> Result<(), Unmodelled> {
        let (p, q) = (
            self.world.monsters[v].position,
            self.world.monsters[a].position,
        );
        let dist = distance2d(p.0 - q.0, p.1 - q.1);
        let r12 = match ai.class {
            0 => {
                if self.world.monsters[v].flags & 2 != 0
                    || (dist > i32::from(ai.engage_distance) && dist > i32::from(ai.vision))
                {
                    r8 = false;
                }
                false
            }
            1 => {
                let m = &mut self.world.monsters[v];
                m.flags = missile_drops_target(m.flags, m.flags_8);
                dist > i32::from(ai.vision)
            }
            _ => true,
        };
        self.hit_engage(v, a, r8, r12, 6, damage, ai)
    }

    fn damage_flag_8_reaction(
        &mut self,
        v: usize,
        a: usize,
        mut r8: bool,
        damage: &DamageDefinition,
        ai: HitAi,
    ) -> Result<(), Unmodelled> {
        if self.world.monsters[v].flags & 0x40_0000 != 0 {
            return Ok(());
        }
        let (p, q) = (
            self.world.monsters[v].position,
            self.world.monsters[a].position,
        );
        let dist = distance2d(p.0 - q.0, p.1 - q.1);
        let mut r12 = false;
        match ai.class {
            0 => {
                if dist > i32::from(ai.engage_distance) {
                    r8 = false;
                }
                if ai.byte_198 & 0x40 != 0 {
                    let f = self.world.monsters[v].flags;
                    let mut e = f;
                    if f & 8 != 0 && self.world.monsters[v].flags_8 & 0x20 != 0 && f & 2 == 0 {
                        e = if f & 0x10 != 0 { f & !8 } else { f & !9 };
                        self.world.monsters[v].flags = e;
                    }
                    if e & 4 == 0 {
                        let own = crate::attack_ai::Own {
                            position: p,
                            timer: 0,
                            speed: 0,
                            facing: 0,
                            desired_facing: 0,
                        };
                        let t = self.world.target_distance(v, &own)?;
                        if dist.min(t) > i32::from(ai.vision) {
                            self.world.monsters[v].flags |= 0x8_0000;
                            r12 = true;
                        }
                    }
                }
            }
            1 => {
                let m = &mut self.world.monsters[v];
                m.flags = missile_drops_target(m.flags, m.flags_8);
                r12 = class_1_flees_flag_8(
                    dist,
                    i32::from(ai.vision),
                    self.world.game_options,
                    damage.damage_type as u16,
                );
            }
            2..=6 => r12 = true,
            _ => {}
        }
        self.hit_engage(v, a, r8, r12, 2, damage, ai)
    }

    #[allow(clippy::too_many_arguments)]
    fn hit_engage(
        &mut self,
        v: usize,
        a: usize,
        r8: bool,
        mut r12: bool,
        flags: u32,
        damage: &DamageDefinition,
        ai: HitAi,
    ) -> Result<(), Unmodelled> {
        if !r8 {
            return Ok(());
        }
        if self.world.monsters[v].flags & 4 != 0 {
            r12 = false;
        }
        if self.world.engage_monster(v, a, flags)? {
            if ai.class == 0 && damage.flags & 8 == 0 {
                self.world.monsters[v].flags |= 0x2000_0000;
            }
            if r12 {
                self.world.monsters[v].flags |= 0x8_0000;
            }
        }
        Ok(())
    }
}

pub(crate) fn kill_body_in_mode_13(flags_8: u32, mode: u8) -> bool {
    flags_8 & 0x200_0000 == 0 && mode == 13
}

pub(crate) fn kill_descriptor_mode_flags(flags_8: u32, mode: u8) -> u32 {
    if kill_body_in_mode_13(flags_8, mode) {
        0x14
    } else if mode == 11 {
        0x10
    } else {
        0
    }
}

fn mons_flags(raw: &[u8]) -> u32 {
    raw.get(..4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

impl EngineWorld {
    fn carried_charges(&self, carried: i32) -> i16 {
        self.projectiles
            .iter()
            .find(|p| p.object.index == carried)
            .map_or(-1, |p| p.object.charges)
    }

    fn delete_held_object(&mut self, object: i32) -> Result<(), Unmodelled> {
        let Some(at) = self
            .projectiles
            .iter()
            .position(|p| p.object.index == object)
        else {
            return Err(Unmodelled::new(
                draw_site::DELETE_HELD_NOT_AT_REST,
                format!("delete_object: object {object} isn't at rest"),
            ));
        };
        let deleted = self.projectiles.remove(at);
        self.free_objects.insert(deleted.object.index);
        Ok(())
    }

    fn held_projectile(&self, object: i32) -> Option<FourCC> {
        self.projectiles
            .iter()
            .find(|p| p.object.index == object && p.object.kind == 4)
            .map(|p| p.definition)
    }
}

fn drop_ammunition(
    w: &mut dyn ProjectileWorld,
    mons_flags: u32,
    projectile: Option<FourCC>,
    count: i16,
    desc: &GroupDescriptor,
    flags: u32,
) -> Result<bool, Unmodelled> {
    let Some(projectile) = projectile else {
        return Ok(false);
    };
    if mons_flags & 0x4_0000 != 0 {
        let class = w.catalog().projectiles.get(&projectile).map(|p| p.class());
        if desc.flags & 0x20 == 0 || !matches!(class, Some(8 | 9)) {
            return Ok(false);
        }
    }
    for _ in 0..count {
        let record = ammunition_group_part(w, desc, projectile)?;
        if flags & 1 == 0 {
            continue;
        }
        let Some(record) = record else {
            continue;
        };
        let definition = w.projectile_mut(record).map(|p| p.definition);
        let artifact = definition
            .and_then(|d| w.catalog().projectiles.get(&d))
            .is_some_and(|d| crate::engine_rng::tag_ref(&d.raw, 0x78).is_some());
        if artifact {
            let Some(shape) = w.projectile_mut(record).and_then(|p| p.shape.as_mut()) else {
                return Err(Unmodelled::new(
                    draw_site::DROP_AMMUNITION_NO_OBJECT,
                    format!("sub_50e840: record {record} has no object"),
                ));
            };
            shape.charges = 1;
        }
    }
    Ok(true)
}

pub fn ammunition_group_part(
    w: &mut dyn ProjectileWorld,
    desc: &GroupDescriptor,
    projectile: FourCC,
) -> Result<Option<i32>, Unmodelled> {
    let Some((def_flags, def_class)) = w
        .catalog()
        .projectiles
        .get(&projectile)
        .map(|d| (d.flags, d.class()))
    else {
        return Err(Unmodelled::new(
            draw_site::AMMUNITION_PART_NOT_LOADED,
            format!("proj {projectile} not loaded"),
        ));
    };
    let src = w.object(desc.object).ok_or_else(|| {
        Unmodelled::new(
            draw_site::AMMUNITION_PART_SOURCE,
            format!("group source object {} not modelled", desc.object),
        )
    })?;
    let height = 0x80 + ((i32::from(w.rng().draw(draw_site::AMMUNITION_PART_HEIGHT)) * 0x80) >> 16);
    let mut n = ProjectileDescriptor {
        definition: projectile,
        flags: 0,
        position: src.position,
        velocity: src.velocity,
        facing: src.facing,
        elevation: src.elevation,
        word_1c: 0x100,
        owner_monster: -1,
        parent: -1,
        owner_object: -1,
        target: -1,
        target_point: [0; 3],
    };
    if desc.flags & 2 != 0 {
        n.position = desc.point;
    } else {
        let r = i32::from(src.radius);
        let (x, y) = if r > 0 {
            let y = src.position[1]
                + ((i32::from(w.rng().draw(draw_site::AMMUNITION_PART_Y)) * 2 * r) >> 16)
                - r;
            let x = src.position[0]
                + ((i32::from(w.rng().draw(draw_site::AMMUNITION_PART_X)) * 2 * r) >> 16)
                - r;
            (x, y)
        } else {
            (src.position[0] - r, src.position[1] - r)
        };
        let f = i32::from(height as i16);
        let h = i32::from((i32::from(src.top) - i32::from(src.bottom)) as i16);
        let z = ((f * h) >> 8) + src.position[2] + 2 * i32::from(src.bottom);
        n.position = [x, y, z];
    }
    n.velocity = if desc.flags & 1 != 0 {
        desc.velocity
    } else {
        line_segment_to_vector(n.position, desc.point, desc.magnitude)
    };
    if desc.flags & 4 != 0 {
        if def_flags & 0x2100 == 0 {
            let global = FourCC::new(crate::projectile_groups::INTERNAL_PROJECTILE_FLAG_4);
            if !w.catalog().projectiles.contains_key(&global) {
                return Err(Unmodelled::new(
                    draw_site::AMMUNITION_GLOBAL_NOT_LOADED,
                    format!("group flag 4: the global projectile {global} not loaded"),
                ));
            }
            n.definition = global;
        }
        n.flags |= 6;
    }
    if desc.flags & 8 != 0 {
        n.flags |= 8;
    }
    if desc.flags & 0x10 != 0 {
        n.flags |= 2;
    }
    if desc.flags & 0x20 != 0 && matches!(def_class, 8 | 9) {
        n.flags |= 1;
    }
    n.owner_object = desc.shading_object;
    match src.kind {
        3 => n.parent = src.owner,
        2 => n.owner_monster = src.owner,
        _ => {}
    }
    if w.game_options_high()
        & mandrake_common::game_options::high(mandrake_common::game_options::TFL)
        == 0
        && desc.owner != -1
    {
        n.owner_monster = desc.owner;
    }
    let record = new_projectile(w, &n)?;
    if let Some(i) = record
        && desc.flags & 0x80 != 0
    {
        let c =
            ((i32::from(w.rng().draw(draw_site::AMMUNITION_PART_COUNTDOWN)) * 0x1e) >> 16) as i16;
        if let Some(p) = w.projectile_mut(i) {
            p.countdown = c;
            if let Some(s) = p.shape.as_mut() {
                s.no_shape = true;
            }
        }
    }
    Ok(record)
}

impl crate::Simulation {
    #[allow(clippy::too_many_arguments)]
    pub fn kill_monster_ammunition(
        &mut self,
        tick: u32,
        victim: u32,
        code: u8,
        damage: &DamageDefinition,
        amount: i32,
        point: [i32; 3],
        record: Option<i32>,
    ) -> Result<(), Unmodelled> {
        let w = &self.engine;
        let Some(v) = w.monster_index_by_identifier(victim as u16) else {
            return Ok(());
        };
        let m = &w.monsters[v];
        let mode_flags = kill_descriptor_mode_flags(m.flags_8, m.mode);
        let count = i16::from(w.combat(v).ammunition as i8);
        let raw = &w.monster_definitions[w.kill_definition(v)].ai.raw;
        let mons_flags = mons_flags(raw);
        let projectile = raw
            .get(0x1ec..0x1f0)
            .filter(|b| *b != [0xff; 4])
            .map(|b| FourCC::new([b[0], b[1], b[2], b[3]]));
        if count <= 0 {
            return Ok(());
        }
        let Some(src) = SimSpawn::unit_object(self, victim) else {
            return Err(Unmodelled::new(
                draw_site::KILL_AMMUNITION_NO_OBJECT,
                format!("kill_monster: unit {victim} has no engine object"),
            ));
        };
        let mut desc = crate::combat::kill_descriptor(
            FourCC::new([0xff; 4]),
            src.index,
            code,
            damage,
            amount,
            point,
        );
        desc.flags |= mode_flags;
        if desc.magnitude <= 9 {
            desc.magnitude = 10;
        }
        let dropped = drop_ammunition(
            &mut SimSpawn {
                sim: self,
                tick,
                source: None,
                reserved: record,
                source_record: None,
                engine: true,
                updating_object: None,
            },
            mons_flags,
            projectile,
            count,
            &desc,
            0,
        )?;
        if dropped {
            self.engine.combat_mut(v).ammunition = 0;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn kill_monster_body_13(
        &mut self,
        tick: u32,
        victim: u32,
        code: u8,
        damage: &DamageDefinition,
        amount: i32,
        point: [i32; 3],
        record: Option<i32>,
    ) -> Result<(), Unmodelled> {
        let w = &self.engine;
        let Some(v) = w.monster_index_by_identifier(victim as u16) else {
            return Ok(());
        };
        let m = &w.monsters[v];
        let group = FourCC::new(GROUP_LOW_VITALITY_BODY);
        if !kill_body_in_mode_13(m.flags_8, m.mode) || !w.catalog.groups.contains_key(&group) {
            return Ok(());
        }
        let Some(src) = SimSpawn::unit_object(self, victim) else {
            return Err(Unmodelled::new(
                draw_site::KILL_BODY_NO_OBJECT,
                format!("kill_monster: unit {victim} has no engine object"),
            ));
        };
        let mut desc =
            crate::combat::kill_descriptor(group, src.index, code, damage, amount, point);
        desc.flags |= 0x14;
        let mut w = SimSpawn {
            sim: self,
            tick,
            source: None,
            reserved: record,
            source_record: None,
            engine: true,
            updating_object: None,
        };
        crate::projectile_groups::new_projectile_group(&mut w, &desc).map(|_| ())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn kill_monster_carried(
        &mut self,
        tick: u32,
        victim: u32,
        code: u8,
        damage: &DamageDefinition,
        amount: i32,
        point: [i32; 3],
        record: Option<i32>,
    ) -> Result<(), Unmodelled> {
        let w = &self.engine;
        let Some(v) = w.monster_index_by_identifier(victim as u16) else {
            return Ok(());
        };
        let m = &w.monsters[v];
        let c = w.combat(v);
        let mode_flags = kill_descriptor_mode_flags(m.flags_8, m.mode);
        if c.carried == -1 && c.inventory.len <= 0 {
            return Ok(());
        }
        let Some(src) = SimSpawn::unit_object(self, victim) else {
            return Err(Unmodelled::new(
                draw_site::KILL_CARRIED_NO_OBJECT,
                format!("kill_monster: unit {victim} has no engine object"),
            ));
        };
        let mut desc = crate::combat::kill_descriptor(
            FourCC::new([0xff; 4]),
            src.index,
            code,
            damage,
            amount,
            point,
        );
        desc.flags |= mode_flags;
        if desc.magnitude <= 9 {
            desc.magnitude = 10;
        }
        for k in 0..9 {
            let w = &self.engine;
            let c = w.combat(v);
            if i32::from(c.inventory.len) <= k as i32 {
                break;
            }
            let object = c.inventory.slots[k];
            if object == c.carried || object == -1 {
                continue;
            }
            if self.held_object_drops(tick, (v, victim), i32::from(object), &desc, record)? {
                self.engine.combat_mut(v).inventory.slots[k] = -1;
            }
        }
        let w = &self.engine;
        let carried = i32::from(w.combat(v).carried);
        if carried != -1 && self.held_object_drops(tick, (v, victim), carried, &desc, record)? {
            self.engine.combat_mut(v).carried = -1;
        }
        Ok(())
    }

    fn held_object_drops(
        &mut self,
        tick: u32,
        (v, victim): (usize, u32),
        carried: i32,
        desc: &GroupDescriptor,
        record: Option<i32>,
    ) -> Result<bool, Unmodelled> {
        let w = &self.engine;
        let m = &w.monsters[v];
        let Some(projectile) = w.held_projectile(carried) else {
            return Err(Unmodelled::new(
                draw_site::HELD_OBJECT_NOT_AT_REST,
                format!("sub_50e950: {victim} holds object {carried}, not a projectile at rest"),
            ));
        };
        let mons_flags = mons_flags(&w.monster_definitions[m.definition].ai.raw);
        let charges = w.carried_charges(carried);
        let artifact = w.artifact_flags(projectile)?;
        let recharge = w.artifact_recharge(projectile)?;
        let gameplay = w.formation_globals.gameplay_version;
        let mut desc = *desc;
        if let Some(flags) = artifact {
            let recharges = gameplay > game_options::GAMEPLAY_1_7_2 && flags & 0x6_0000 == 0x6_0000;
            if recharges {
                let mut w = SimSpawn {
                    sim: self,
                    tick,
                    source: None,
                    reserved: record,
                    source_record: None,
                    engine: true,
                    updating_object: None,
                };
                drop_ammunition(&mut w, mons_flags, recharge, charges, &desc, 1)?;
            }
            if recharges || flags & 0x400 != 0 {
                let w = &mut self.engine;
                w.delete_held_object(carried)?;
                return Ok(true);
            }
        }
        desc.shading_object = carried;
        let new = ammunition_group_part(
            &mut SimSpawn {
                sim: self,
                tick,
                source: None,
                reserved: record,
                source_record: None,
                engine: true,
                updating_object: None,
            },
            &desc,
            projectile,
        )?;
        let Some(new) = new else {
            return Ok(false);
        };
        self.replace_carried_object(carried, new)?;
        let definition = self
            .combat
            .flight
            .flights
            .iter()
            .find(|p| p.index == new)
            .map(|p| p.definition);
        let keeps = match definition {
            Some(d) => self.engine.artifact_flags(d)?.is_some_and(|f| f & 8 != 0),
            None => false,
        };

        if keeps
            && let Some(shape) = self
                .combat
                .flight
                .flights
                .iter_mut()
                .find(|p| p.index == new)
                .and_then(|p| p.shape.as_mut())
        {
            shape.charges = charges;
        }
        Ok(false)
    }

    fn replace_carried_object(&mut self, carried: i32, record: i32) -> Result<(), Unmodelled> {
        let w = &mut self.engine;
        let Some(at) = w.projectiles.iter().position(|p| p.object.index == carried) else {
            return Err(Unmodelled::new(
                draw_site::REPLACE_CARRIED_NOT_AT_REST,
                format!("replace_object: object {carried} isn't at rest"),
            ));
        };
        let new = self
            .combat
            .flight
            .flights
            .iter_mut()
            .find(|p| p.index == record);
        let Some(shape) = new.and_then(|p| p.shape.as_mut()) else {
            return Err(Unmodelled::new(
                draw_site::REPLACE_CARRIED_NOT_AT_REST,
                format!("replace_object: record {record} has no object"),
            ));
        };
        let old = w.projectiles.remove(at);
        w.free_objects.insert(shape.index);
        shape.index = carried;
        shape.identifier = old.object.identifier;
        Ok(())
    }
}

impl WorldHooks<'_, '_> {
    pub(crate) fn kill_monster_without_damage(
        &mut self,
        code: u8,
        point: [i32; 3],
        magnitude: i32,
    ) -> Result<(), Unmodelled> {
        let v = self.monster;
        let m = &self.world.monsters[v];
        let raw = &self.world.monster_definitions[self.world.kill_definition(v)]
            .ai
            .raw;
        let groups = [
            group_at(raw, 0x1a4),
            group_at(raw, 0x194),
            group_at(raw, 0x3c),
        ];
        let mode_flags = kill_descriptor_mode_flags(m.flags_8, m.mode);
        let body = kill_body_in_mode_13(m.flags_8, m.mode);
        let mons_flags = mons_flags(raw);
        let projectile = group_at(raw, 0x1ec);
        if m.mode == 14 {
            self.world.monsters[v].object.shading_slot = 0;
        }
        let m = &self.world.monsters[v];
        let source = crate::projectile_groups::EngineSpawn::monster_object(
            self.world, v, m.position, m.facing,
        );
        let mut desc = GroupDescriptor {
            flags: mode_flags,
            group: FourCC::new([0xff; 4]),
            shading_object: source.index,
            object: source.index,
            owner: -1,
            point,
            magnitude,
            velocity: [0; 3],
        };
        let mut spawn = crate::projectile_groups::EngineSpawn {
            world: &mut *self.world,
            rng: &mut *self.rng,
            source: Some(source),
            host: None,
        };
        if let Some(group) = crate::combat::death_group(groups, code) {
            desc.group = group;
            crate::projectile_groups::new_projectile_group(&mut spawn, &desc)?;
        }
        let insd = FourCC::new(GROUP_LOW_VITALITY_BODY);
        if body && spawn.world.catalog.groups.contains_key(&insd) {
            desc.group = insd;
            crate::projectile_groups::new_projectile_group(&mut spawn, &desc)?;
        }
        desc.group = FourCC::new([0xff; 4]);
        desc.magnitude = desc.magnitude.max(10);
        let count = i16::from(spawn.world.combat(v).ammunition as i8);
        if drop_ammunition(&mut spawn, mons_flags, projectile, count, &desc, 0)? {
            spawn.world.combat_mut(v).ammunition = 0;
        }
        self.kill_inventory(&desc)?;
        self.kill_carried_object(&desc)?;
        self.kill_monster()
    }

    fn kill_inventory(&mut self, desc: &GroupDescriptor) -> Result<(), Unmodelled> {
        let v = self.monster;
        for k in 0..9 {
            let c = self.world.combat(v);
            if i32::from(c.inventory.len) <= k as i32 {
                break;
            }
            let object = c.inventory.slots[k];
            if object == c.carried || object == -1 {
                continue;
            }
            if self.held_object_drops(desc, i32::from(object))? {
                self.world.combat_mut(v).inventory.slots[k] = -1;
            }
        }
        Ok(())
    }

    fn kill_carried_object(&mut self, desc: &GroupDescriptor) -> Result<(), Unmodelled> {
        let v = self.monster;
        let carried = i32::from(self.world.combat(v).carried);
        if carried != -1 && self.held_object_drops(desc, carried)? {
            self.world.combat_mut(v).carried = -1;
        }
        Ok(())
    }

    fn held_object_drops(
        &mut self,
        desc: &GroupDescriptor,
        object: i32,
    ) -> Result<bool, Unmodelled> {
        let v = self.monster;
        let w = &mut *self.world;
        let id = w.monsters[v].identifier;
        let carried = object;
        let Some(projectile) = w.held_projectile(carried) else {
            return Err(Unmodelled::new(
                draw_site::HELD_OBJECT_NOT_AT_REST,
                format!("sub_50e950: {id} holds object {carried}, not a projectile at rest"),
            ));
        };
        let mons_flags = mons_flags(&w.monster_definitions[w.monsters[v].definition].ai.raw);
        let charges = w.carried_charges(carried);
        if let Some(flags) = w.artifact_flags(projectile)? {
            let gameplay = w.formation_globals.gameplay_version;
            let recharges = gameplay > game_options::GAMEPLAY_1_7_2 && flags & 0x6_0000 == 0x6_0000;
            if recharges {
                let recharge = w.artifact_recharge(projectile)?;
                let m = &w.monsters[v];
                let source = crate::projectile_groups::EngineSpawn::monster_object(
                    w, v, m.position, m.facing,
                );
                let mut spawn = crate::projectile_groups::EngineSpawn {
                    world: &mut *w,
                    rng: &mut *self.rng,
                    source: Some(source),
                    host: None,
                };
                drop_ammunition(&mut spawn, mons_flags, recharge, charges, desc, 1)?;
            }
            if recharges || flags & 0x400 != 0 {
                w.delete_held_object(carried)?;
                return Ok(true);
            }
        }
        let mut desc = *desc;
        desc.shading_object = carried;
        let m = &w.monsters[v];
        let source =
            crate::projectile_groups::EngineSpawn::monster_object(w, v, m.position, m.facing);
        let new = ammunition_group_part(
            &mut crate::projectile_groups::EngineSpawn {
                world: &mut *w,
                rng: &mut *self.rng,
                source: Some(source),
                host: None,
            },
            &desc,
            projectile,
        )?;
        let Some(new) = new else {
            return Ok(false);
        };
        let definition = w
            .projectile_records
            .iter()
            .find(|p| p.index == new)
            .map(|p| p.definition);
        let keeps = match definition {
            Some(d) => w.artifact_flags(d)?.is_some_and(|f| f & 8 != 0),
            None => false,
        };
        let Some(at) = w.projectiles.iter().position(|p| p.object.index == carried) else {
            return Err(Unmodelled::new(
                draw_site::REPLACE_CARRIED_NOT_AT_REST,
                format!("replace_object: object {carried} isn't at rest"),
            ));
        };
        let Some(shape) = w
            .projectile_records
            .iter_mut()
            .find(|p| p.index == new)
            .and_then(|p| p.shape.as_mut())
        else {
            return Err(Unmodelled::new(
                draw_site::REPLACE_CARRIED_NOT_AT_REST,
                format!("replace_object: record {new} has no object"),
            ));
        };
        let old = w.projectiles.remove(at);
        w.free_objects.insert(shape.index);
        shape.index = carried;
        shape.identifier = old.object.identifier;
        if keeps {
            shape.charges = charges;
        }
        Ok(false)
    }

    pub(crate) fn kill_monster(&mut self) -> Result<(), Unmodelled> {
        let marker = self.world.monster_marker(self.monster);
        self.delete_or_make_again(marker)
    }

    pub(crate) fn delete_or_make_again(
        &mut self,
        marker: Option<mandrake_tag::MarkerEntry>,
    ) -> Result<(), Unmodelled> {
        let v = self.monster;
        let gameplay = self.world.formation_globals.gameplay_version;
        let m = &self.world.monsters[v];
        let (flags, flags_8, id, owner, team) = (m.flags, m.flags_8, m.identifier, m.owner, m.team);
        let (position, z, facing) = (m.position, m.object.z, m.facing);
        let option = self.world.game_options & mandrake_common::game_options::DEATHMATCH != 0;
        let marker_flags = marker.as_ref().map_or(0, |mk| mk.flags);
        if marker_flags & 0xa0 == 0xa0
            && let Some(mk) = &marker
        {
            self.world.marker_places.insert(
                mk.marker_id,
                crate::engine_rng::MarkerPlace {
                    x: position.0,
                    y: position.1,
                    z,
                    yaw: facing as i16,
                    pitch: mk.pitch,
                },
            );
        }
        let again = self.world.made_again(v, marker_flags);
        self.delete_monster()?;
        if !again || gameplay <= 0x513 {
            return Ok(());
        }
        let Some(t) = usize::try_from(team)
            .ok()
            .and_then(|t| self.world.players.teams.get(t))
            .copied()
        else {
            return Err(Unmodelled::new(
                draw_site::MADE_AGAIN_NO_TEAM,
                format!("kill_monster: {id} made again, of team {team}, which isn't a team"),
            ));
        };
        if t.flags & 1 != 0 || t.players == 0 {
            return Ok(());
        }
        let Some(mk) = marker else {
            return Ok(());
        };
        if !option && mk.flags & 0x40 != 0 {
            self.world.set_marker_created(&mk, false);
            if gameplay > game_options::GAMEPLAY_1_7_2 {
                self.world.marker_owners.insert(mk.marker_id, owner);
            }
            return Ok(());
        }
        let Some(creation) = self.world.marker_creation.clone() else {
            return Err(Unmodelled::new(
                draw_site::MADE_AGAIN_NO_MARKERS,
                format!("kill_monster: {id} made again without the load's markers"),
            ));
        };
        let mut entry = mk.clone();
        entry.flags = (entry.flags & !1) | u32::from(self.world.marker_is_unplaced(&mk));
        let Some(n) = self
            .world
            .new_monster_from_marker(&creation, &entry, self.rng)?
        else {
            return Ok(());
        };
        let nm = &mut self.world.monsters[n];
        let before = nm.flags;
        nm.flags = (nm.flags & !0x1000_0000) | (flags & 0x1000_0000) | 0x40;
        nm.flags_8 = (nm.flags_8 & !2) | (flags_8 & 2);
        if gameplay > 0x73b && (before ^ nm.flags) & 0x1000_0000 != 0 {
            self.world.monster_player_control_changed(n);
        }
        self.world.detach_monster_with(n, owner, true)?;
        let identifier = self.world.monsters[n].identifier;
        let mode = if gameplay > game_options::GAMEPLAY_1_3 && mk.flags & 0x100 != 0 {
            0
        } else {
            1
        };
        self.world
            .make_monster_visible(identifier, mode, self.rng, self.time)?;
        Ok(())
    }

    pub(crate) fn kill_monster_code_28(&mut self) -> Result<(), Unmodelled> {
        let v = self.monster;
        let m = &self.world.monsters[v];
        let id = m.identifier;
        if m.mode != 13 && m.flags & 0x4000_0000 == 0 {
            self.world.monsters[v].mode = 15;
            let (obj_idx, obj_identifier) = self.world.new_object_datum();
            let r = i32::from(self.rng.draw(draw_site::KILL_CODE_28));
            let ticks = (30 + ((r * 30) >> 16)) as u8;
            if self.world.monsters[v].object.no_shape {
                self.world.free_objects.insert(obj_idx);
            } else {
                let record = self.world.new_fade_record_index();
                let mut object = self.world.monsters[v].object.clone();
                object.index = obj_idx;
                object.identifier = obj_identifier;
                object.flags = self.world.duplicate_flags(v);
                object.sub_kind = 1;
                object.sub_count = 0;
                object.sub_limit = ticks;
                object.kind = 5;
                object.owner = record as i16;
                let cell_link = self.world.copy_cell_link(self.world.monsters[v].cell_link);
                self.world.fade_records.push(crate::engine_rng::FadeRecord {
                    index: record,
                    host: crate::engine_rng::FadeRecord::NO_HOST,
                    object,
                    position: self.world.monsters[v].position,
                    facing: self.world.monsters[v].facing,
                    cell_link,
                    countdown: 0,
                    group: None,
                    monster: None,
                });
            }
        }
        let marker = self.world.monster_marker(v);
        self.delete_or_make_again(marker)?;
        if let Some(u) = self
            .world
            .unit_markers
            .iter_mut()
            .find(|u| u.identifier == id)
        {
            u.created = false;
        }
        Ok(())
    }

    pub(crate) fn delete_monster(&mut self) -> Result<(), Unmodelled> {
        let v = self.monster;
        let id = self.world.monsters[v].identifier;
        self.call_monster_died_procs(3)?;
        self.world.delete_parasite(v);
        if self.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_7_2 {
            self.world.pending_map_actions.retain(|p| p.monster != v);
        }
        if self.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_7_2
            && let Some(action) = self.world.monster_map_actions.remove(&v)
        {
            let Some(actions) = self.actions.as_mut().or(self.world.map_actions.as_mut()) else {
                return Err(Unmodelled::new(
                    draw_site::DELETE_MONSTER_NO_ACTIONS,
                    format!("delete_monster: {id} has a map action, but the stage has none"),
                ));
            };
            actions.actions[action].identifier = 0;
        }
        self.world.turn_off_monster_local_group(v);
        self.world.unlink_monster_from_index(v);
        let m = &mut self.world.monsters[v];
        m.identifier = 0;
        m.object.no_shape = true;
        let object = m.object.index;
        self.world.free_objects.insert(object);
        if let Some(slot) = self.world.movers.get_mut(v) {
            *slot = None;
        }
        Ok(())
    }

    pub(crate) fn call_monster_died_procs(&mut self, flags: u32) -> Result<(), Unmodelled> {
        let v = self.monster;
        let id = self.world.monsters[v].identifier;
        for r in &mut self.world.projectile_records {
            if r.owner_monster == v as i16 {
                r.owner_monster = -1;
            }
            if r.target == crate::engine_rng::unit_key(id) {
                r.target = None;
            }
        }
        if self.world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_7 {
            self.world.fire.monster_died(v as i16);
        }
        self.retarget_after_death(flags)
    }

    fn retarget_after_death(&mut self, _flags: u32) -> Result<(), Unmodelled> {
        let d = self.monster as i16;
        for j in 0..self.world.monsters.len() {
            if self.world.monsters[j].identifier == 0 || j == self.monster {
                continue;
            }
            let f = self.world.monsters[j].flags;
            let c = self.world.combat_mut(j);
            if c.attacker == d {
                c.attacker = -1;
            }
            if c.pending == d {
                c.pending = -1;
            }
            let targets = f & 8 != 0 && c.target == d;
            if f & 0x4000_0000 != 0 {
                if targets {
                    c.target = -1;
                    self.world.monsters[j].flags &= if f & 0x10 != 0 { !8 } else { !9 };
                }
                continue;
            }
            if !targets {
                continue;
            }
            self.retarget(j)?;
        }
        Ok(())
    }

    fn retarget(&mut self, j: usize) -> Result<(), Unmodelled> {
        let jid = self.world.monsters[j].identifier;
        let Some(own) = self
            .world
            .movers
            .get(j)
            .and_then(Option::as_ref)
            .map(crate::attack_ai::Own::of)
        else {
            return Err(Unmodelled::new(
                draw_site::RETARGET_NO_MOVER,
                format!("sub_518b80: monster {jid} without its mover"),
            ));
        };
        let mut t = self.world.find_target(j, 1, &own)?;
        if t == -1 {
            t = self.world.find_target(j, 2, &own)?;
        }
        let def = &self.world.monster_definitions[self.world.monsters[j].definition];
        let m = &self.world.monsters[j];
        let ordered = def.ai.byte_199 & 0x40 != 0 && m.flags & 4 != 0;
        let f = m.flags;
        let f8 = m.flags_8;
        let m = &mut self.world.monsters[j];
        m.flags_8 = f8 & 0xffdf_7fdf;
        m.flags = (if f & 0x10 != 0 {
            f & 0xdf5e_fff1
        } else {
            f & 0xdf5e_fff0
        }) | 0x8000;
        let c = self.world.combat_mut(j);
        c.target = -1;
        let p = c.pending;
        if let Ok(pi) = usize::try_from(p)
            && (f8 & 0x400_0000 != 0 || !self.world.monsters_are_allied(j, pi))
            && self.world.monsters[pi].identifier != 0
        {
            let c = self.world.combat_mut(j);
            c.pending = -1;
            c.target = p;
            self.world.drop_waypoints(j)?;
            let m = &mut self.world.monsters[j];
            m.object.flags &= !0x80_0000;
            m.flags_8 &= 0xf3db_f9ff;
            let e = m.flags & 0x5f06_be6f;
            m.flags = e | 0x802f | if f8 & 0x400_0000 != 0 { 0x80_0000 } else { 0 };
            return Ok(());
        }
        if t != -1 && self.world.engage_monster(j, t as usize, 0)? {
            if ordered {
                self.world.monsters[j].flags |= 4;
            }
            return Ok(());
        }
        let mode = self.world.monsters[j].mode;
        if mode > 16 || (1u32 << mode) & 0x1_fd00 == 0 {
            let Some(mut mv) = self.world.movers.get_mut(j).and_then(Option::take) else {
                return Err(Unmodelled::new(
                    draw_site::RETARGET_MODE_1_NO_MOVER,
                    "sub_518b80: mode 1 without a mover",
                ));
            };
            let r = {
                let mut h = self.for_monster(j);
                mv.set_mode(MoveMode::Idle, ModeState::Zero, &mut h)
            };
            self.world.movers[j] = Some(mv);
            r?;
        }
        let m = &mut self.world.monsters[j];
        m.flags &= !0x1000;
        m.flags_8 &= !0x4000;
        self.world.drop_waypoint_count(j)
    }
}

fn inventory_stop(identifier: i16) -> Unmodelled {
    Unmodelled::new(
        draw_site::KILL_DROPS_INVENTORY,
        format!("kill_monster: monster {identifier} drops an inventory"),
    )
}

#[cfg(test)]
mod tests {
    use super::{EngineRng, WorldHooks, class_1_flees_flag_8};
    use crate::engine_damage::DamageInfo;
    use crate::engine_move::{EngineMover, MoveMode};
    use crate::engine_rng::EngineWorld;
    use mandrake_tag::projectile::DamageDefinition;

    fn hit_world() -> (EngineWorld, EngineMover) {
        let mut w = crate::collide::tests::world((10_000, 10_000), (20_000, 20_000));
        w.formation_globals.gameplay_version = 0x726;
        w.movers = vec![None, None];
        w.monsters[0].object.max_vitality = 0x100;
        w.monsters[0].object.vitality = 0x100;
        let mut mv = crate::collide::tests::mover((10_000, 10_000), (0, 0));
        mv.mode = MoveMode::Idle;
        w.monsters[0].mode = 1;
        (w, mv)
    }

    fn with_hooks<R>(
        w: &mut EngineWorld,
        f: impl FnOnce(&mut WorldHooks<'_, '_>) -> R,
    ) -> (R, mandrake_math::BungieRng) {
        let (mut rng, mut sites) = (mandrake_math::BungieRng::new(5), Default::default());
        let mut r = EngineRng {
            rng: &mut rng,
            sites: &mut sites,
        };
        let mut actions = None;
        let mut h = WorldHooks {
            world: w,
            actions: &mut actions,
            rng: &mut r,
            monster: 0,
            time: 0,
            host: &mut crate::engine_attack::NoHost,
        };
        let out = f(&mut h);
        (out, rng)
    }

    fn damage(damage_type: i16) -> DamageDefinition {
        DamageDefinition {
            damage_type,
            flags: 0,
            damage_lower_bound: 0x40,
            damage_delta: 0,
            radius_lower_bound: 0,
            radius_delta: 0,
            word_0c: 0,
            word_0e: 0,
        }
    }

    fn info(amount: i16) -> DamageInfo {
        let mut i = DamageInfo::new(crate::statistics::UnitRef(-1), 1, 10_001);
        i.amount = amount;
        i.group = Some(mandrake_common::FourCC::new(*b"grup"));
        i
    }

    #[test]
    fn damage_type_2_stuns_for_the_amount() {
        let (mut w, mut mv) = hit_world();
        let mut i = info(0x80);
        let (r, rng) = with_hooks(&mut w, |h| {
            h.accelerate_monster(Some(&mut mv), &damage(2), &mut i)
        });
        r.unwrap();
        assert_eq!(rng.state, mandrake_math::BungieRng::new(5).state);
        assert_eq!((w.monsters[0].mode, mv.mode), (12, MoveMode::Stunned));
        assert_eq!(w.monsters[0].anim_state, 0xff);
        assert_eq!(mv.pose_timer, (0x80 * 30) >> 8);
        assert_eq!((i.flags & 1, i.group), (1, None));
        assert_eq!(w.monsters[0].object.vitality, 0x100, "no damage_object");
        assert_eq!(w.combat(0).attacker, 1);

        let mut i = info(0x100);
        with_hooks(&mut w, |h| {
            h.accelerate_monster(Some(&mut mv), &damage(2), &mut i)
        })
        .0
        .unwrap();
        assert_eq!(mv.pose_timer, (0x80 * 30) >> 8);
        assert_eq!(i.flags, 0);

        let (mut w, mut mv) = hit_world();
        w.monsters[0].mode = 8;
        w.monsters[0].anim_state = 0xff;
        w.monsters[0].flags = 0x9002 | 0x19;
        w.monsters[0].flags_8 = 0x4_0000;
        let mut i = info(0x100);
        with_hooks(&mut w, |h| {
            h.accelerate_monster(Some(&mut mv), &damage(2), &mut i)
        })
        .0
        .unwrap();
        assert_eq!(w.monsters[0].flags_8 & 0x4_0000, 0);
        assert_eq!(w.monsters[0].mode, 12);
        assert_eq!(mv.pose_timer, 30);
    }

    #[test]
    fn the_stun_ends_by_the_shading_slot() {
        for (slot, mode, state) in [(1u8, 13u8, 0x16u8), (2, 14, 0x18), (0, 1, 0x1a)] {
            let (mut w, mut mv) = hit_world();
            let mut i = info(0x100);
            with_hooks(&mut w, |h| {
                h.accelerate_monster(Some(&mut mv), &damage(2), &mut i)
                    .unwrap();
                h.stunned_handler(&mut mv).unwrap();
            });
            assert_eq!(w.monsters[0].mode, 12);
            w.monsters[0].object.shading_slot = slot;
            mv.pose_timer = -1;
            with_hooks(&mut w, |h| h.stunned_handler(&mut mv).unwrap());
            assert_eq!(w.monsters[0].mode, mode, "slot {slot}");
            if slot != 0 {
                assert_eq!(w.monsters[0].anim_state, state, "slot {slot}");
            }
        }
    }

    #[test]
    fn damage_type_6_needs_the_modifier() {
        let (mut w, mut mv) = hit_world();
        let mut i = info(0x80);
        with_hooks(&mut w, |h| {
            h.accelerate_monster(Some(&mut mv), &damage(6), &mut i)
        })
        .0
        .unwrap();
        assert_eq!(w.monsters[0].mode, 1);
        assert_eq!(i.flags, 0);

        w.monster_definitions[0].object_modifiers[6] = 0x100;
        with_hooks(&mut w, |h| {
            h.accelerate_monster(Some(&mut mv), &damage(6), &mut i)
        })
        .0
        .unwrap();
        assert_eq!((w.monsters[0].mode, w.monsters[0].anim_state), (13, 0x15));
        assert_eq!((i.flags & 1, i.group), (1, None));
        assert_eq!(w.monsters[0].object.vitality, 0x100, "no damage_object");

        let mut i = info(0x80);
        with_hooks(&mut w, |h| {
            h.accelerate_monster(Some(&mut mv), &damage(6), &mut i)
        })
        .0
        .unwrap();
        assert_eq!(i.flags, 0);
    }

    #[test]
    fn the_hit_modifiers_pass_over_artifacts_of_flag_4() {
        let (mut w, _) = hit_world();
        w.monster_definitions[0].object_modifiers[6] = 7;
        let arti = |flags: u32, monster: [u8; 4]| {
            let mut raw = vec![0u8; 0x60];
            raw[0..4].copy_from_slice(&flags.to_be_bytes());
            raw[4..8].copy_from_slice(&monster);
            raw[0x34 + 12..0x34 + 14].copy_from_slice(&9i16.to_be_bytes());
            (mandrake_common::FourCC::new(*b"arti"), raw)
        };
        let own = w.monster_definitions[0].tag.0;
        w.combat_mut(0).special = 0;
        for (flags, monster, want) in [
            (0u32, [0xff; 4], 9),
            (4, *b"othr", 7),
            (0x14, [0xff; 4], 7),
            (0x14, own, 7),
            (0x14, *b"othr", 9),
            (0x1_0000, [0xff; 4], 7),
        ] {
            w.catalog.artifacts = vec![arti(flags, monster)];
            assert_eq!(
                w.hit_effect_modifiers(0).unwrap()[6],
                want,
                "flags {flags:#x}"
            );
        }
    }

    #[test]
    fn a_monster_is_made_again_by_its_marker_or_the_option() {
        let (mut w, _) = hit_world();
        assert!(!w.made_again(0, 0x80));
        assert!(w.made_again(0, 0x20));
        assert!(w.made_again(0, 0xa0));
        w.monsters[0].flags_8 = 0x1000;
        assert!(w.made_again(0, 0x20));
        w.formation_globals.gameplay_version = 0x69f;
        assert!(!w.made_again(0, 0x20));
        w.game_options = 0x10_0000;
        assert!(!w.made_again(0, 0));
        w.monsters[0].flags_8 = 0;
        assert!(w.made_again(0, 0));
    }

    #[test]
    fn kill_monster_has_no_inventory_to_drop() {
        let (mut w, _) = hit_world();
        w.monster_definitions[0].inventory = Some(mandrake_common::FourCC::new(*b"invt"));
        with_hooks(&mut w, |h| h.kill_monster()).0.unwrap();
        assert_eq!(w.monsters[0].identifier, 0);
    }

    #[test]
    fn the_mode_13_body_is_tested_on_the_top_byte_of_plus_8() {
        assert!(super::kill_body_in_mode_13(0, 13));
        assert!(super::kill_body_in_mode_13(0x200, 13));
        assert!(!super::kill_body_in_mode_13(0x200_0000, 13));
        assert!(!super::kill_body_in_mode_13(0, 4));
    }

    #[test]
    fn class_1_flees_by_the_game_options_and_the_damage_type() {
        assert!(!class_1_flees_flag_8(100, 100, 0, 5));
        assert!(class_1_flees_flag_8(101, 100, 0, 0));
        assert!(!class_1_flees_flag_8(101, 100, 0x20_0000, 0));
        assert!(!class_1_flees_flag_8(101, 100, 0x40_0000, 8));
        assert!(class_1_flees_flag_8(101, 100, 0x60_0000, 5));
    }

    #[test]
    fn losing_a_target_keeps_what_the_record_got_since_the_movers_update() {
        let mut w = crate::collide::tests::world((10_000, 10_000), (20_000, 20_000));
        w.monsters[1].mode = 8;
        w.monsters[1].flags = 0x90a8;
        w.monsters[1].flags_8 = 0x10_0100;
        w.combat_mut(1).target = 0;
        let stale = crate::collide::tests::mover((20_000, 20_000), (0, 0));
        w.movers = vec![None, Some(stale)];
        let (mut rng, mut sites) = (mandrake_math::BungieRng::new(5), Default::default());
        let mut r = EngineRng {
            rng: &mut rng,
            sites: &mut sites,
        };
        let mut actions = None;
        let mut h = WorldHooks {
            world: &mut w,
            actions: &mut actions,
            rng: &mut r,
            monster: 0,
            time: 0,
            host: &mut crate::engine_attack::NoHost,
        };
        h.retarget_after_death(0).unwrap();
        let m = &w.monsters[1];
        assert_eq!(
            m.flags & 0x20a8,
            0xa0,
            "the target bit goes, 0x20 and 0x80 stay"
        );
        assert_eq!(m.flags_8 & 0x10_0000, 0x10_0000);
        assert_eq!(w.combat(1).target, -1);
    }
}
