use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_tag::SequenceHeader;

use crate::draw_site;
use crate::engine_move::EngineMover;
use crate::engine_rng::Unmodelled;

fn be16(d: &[u8], o: usize) -> i16 {
    i16::from_be_bytes([d[o], d[o + 1]])
}

fn tag_ref(d: &[u8], o: usize) -> Option<FourCC> {
    let v = i32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    (v != 0 && v != -1).then(|| FourCC::new([d[o], d[o + 1], d[o + 2], d[o + 3]]))
}

pub mod flag {
    pub const ENGAGED: u32 = 0x1;
    pub const TARGET: u32 = 0x8;
    pub const LOCATION: u32 = 0x10;
    pub const NEW_LEG: u32 = 0x20;
    pub const FIRED: u32 = 0x2000;
    pub const OUT_OF_RANGE: u32 = 0x10000;
    pub const FLEEING: u32 = 0x80000;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackDefinition {
    pub flags: u16,
    pub miss_fraction: u16,
    pub projectile: Option<FourCC>,
    pub minimum_range: i16,
    pub maximum_range: i16,
    pub sequences: [(u16, i16); 4],
    pub repetitions: i16,
    pub velocity: i16,
    pub velocity_delta: i16,
    pub error: i16,
    pub error_experience: i16,
    pub recovery: i16,
    pub recovery_experience: i16,
    pub flags_3c: u16,
    pub mana: i16,
}

impl AttackDefinition {
    pub fn from_mons(mons: &[u8], i: usize) -> Self {
        Self::at(mons, 0x8c + 0x40 * i)
    }

    pub fn at(mons: &[u8], o: usize) -> Self {
        let w = |k: usize| be16(mons, o + k);
        let mut sequences = [(0u16, -1i16); 4];
        for (j, s) in sequences.iter_mut().enumerate() {
            *s = (w(0x0c + 8 * j) as u16, w(0x0e + 8 * j));
        }
        Self {
            flags: w(0) as u16,
            miss_fraction: w(2) as u16,
            projectile: tag_ref(mons, o + 4),
            minimum_range: w(0x08),
            maximum_range: w(0x0a),
            sequences,
            repetitions: w(0x2c),
            velocity: w(0x2e),
            velocity_delta: w(0x30),
            error: w(0x32),
            recovery: w(0x34),
            recovery_experience: w(0x36),
            error_experience: w(0x38),
            flags_3c: w(0x3c) as u16,
            mana: w(0x3a),
        }
    }

    pub fn key_point_sequences(&self) -> Vec<i16> {
        let (list, len) = self.key_point_sequences_array();
        list[..len].to_vec()
    }

    pub(crate) fn key_point_sequences_array(&self) -> ([i16; 4], usize) {
        let mut out = [0i16; 4];
        let mut len = 0;
        for &(f, s) in &self.sequences {
            if s != -1 && f & 1 != 0 {
                out[len] = s;
                len += 1;
            }
        }
        (out, len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiDefinition {
    pub tag: FourCC,
    pub flags: u32,
    pub idle_sequence: i16,
    pub walk_sequence: i16,
    pub turn_sequence: i16,
    pub block_sequence: i16,
    pub hit_sequence: i16,
    pub engage_sequence: i16,
    pub hit_sequence_2c: i16,
    pub alert_radius: i16,
    pub engage_distance: i16,
    pub vision: i16,
    pub recovery_bounds: (i16, i16),
    pub byte_198: u8,
    pub byte_199: u8,
    pub byte_1b4: u8,
    pub byte_1b5: u8,
    pub class: i16,
    pub attacks: Vec<AttackDefinition>,
    pub headers: Vec<SequenceHeader>,
    pub raw: Vec<u8>,
}

impl Default for AiDefinition {
    fn default() -> Self {
        Self::from_tags(FourCC::new(*b"\0\0\0\0"), &[0; 0x1b8], Vec::new()).expect("long enough")
    }
}

impl AiDefinition {
    pub fn from_tags(
        tag: FourCC,
        mons: &[u8],
        headers: Vec<SequenceHeader>,
    ) -> Result<Self, String> {
        if mons.len() < 0x1b8 {
            return Err(format!("mons {tag} too short ({} bytes)", mons.len()));
        }
        let count = usize::try_from(be16(mons, 0x88)).unwrap_or(0).min(4);
        Ok(Self {
            tag,
            flags: u32::from_be_bytes([mons[0], mons[1], mons[2], mons[3]]),
            idle_sequence: be16(mons, 0x08),
            walk_sequence: be16(mons, 0x0a),
            turn_sequence: be16(mons, 0x10),
            block_sequence: be16(mons, 0x12),
            hit_sequence: be16(mons, 0x14),
            engage_sequence: be16(mons, 0x26),
            hit_sequence_2c: be16(mons, 0x2c),
            alert_radius: be16(mons, 0x6e),
            engage_distance: be16(mons, 0x70),
            vision: be16(mons, 0x78),
            recovery_bounds: (be16(mons, 0x190), be16(mons, 0x192)),
            byte_198: mons[0x198],
            byte_199: mons[0x199],
            byte_1b4: mons[0x1b4],
            byte_1b5: mons[0x1b5],
            class: be16(mons, 0x1b6),
            attacks: (0..count)
                .map(|i| AttackDefinition::from_mons(mons, i))
                .collect(),
            headers,
            raw: mons.to_vec(),
        })
    }

    pub fn word(&self, o: usize) -> i16 {
        if self.raw.len() < o + 2 {
            return 0;
        }
        be16(&self.raw, o)
    }

    pub fn attack(&self, a: i8) -> Option<&AttackDefinition> {
        let n = self.attacks.len();
        if n == 0 {
            return None;
        }
        Some(&self.attacks[(a.max(0) as usize).min(n - 1)])
    }

    pub(crate) fn attack_mut(&mut self, a: i8) -> Option<&mut AttackDefinition> {
        let n = self.attacks.len();
        if n == 0 {
            return None;
        }
        Some(&mut self.attacks[(a.max(0) as usize).min(n - 1)])
    }

    pub fn untargetable(&self) -> bool {
        self.flags & 0x800 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterCombat {
    pub target: i16,
    pub pending: i16,
    pub location: (i32, i32),
    pub velocity: i16,
    pub elevation: u16,
    pub flight_time: i16,
    pub aim: (i32, i32),
    pub location_z: i16,
    pub byte_8d: u8,
    pub experience: u8,
    pub battles: i8,
    pub custom_name: crate::veterans::CustomName,
    pub name_index: i8,
    pub ammunition: u8,
    pub mana: i16,
    pub carried: i16,
    pub pick_up: (i16, i16),
    pub special: i16,
    pub inventory_size: i8,
    pub inventory: Inventory,
    pub unit: (u32, i16),
    pub attacker: i16,
    pub held_unit: i16,
    pub confusion: i16,
    pub local_group: Option<FourCC>,
    pub marker_flags: u32,
    pub own_definition: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inventory {
    pub len: i8,
    pub slots: [i16; 9],
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            len: 0,
            slots: [-1; 9],
        }
    }
}

impl Inventory {
    pub fn objects(&self) -> &[i16] {
        &self.slots[..self.len.clamp(0, 9) as usize]
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, object: i16) {
        let at = self.len.clamp(0, 8) as usize;
        self.slots[at] = object;
        self.len = self.len.wrapping_add(1);
    }
}

impl Default for MonsterCombat {
    fn default() -> Self {
        Self {
            target: -1,
            pending: -1,
            location: (0, 0),
            velocity: 0,
            elevation: 0,
            flight_time: 0,
            aim: (0, 0),
            location_z: 0,
            byte_8d: 0,
            experience: 0,
            battles: 0,
            custom_name: [0; 18],
            name_index: 0,
            ammunition: 0,
            mana: 0,
            carried: -1,
            pick_up: (0, 0),
            special: -1,
            inventory_size: 0,
            inventory: Inventory::default(),
            unit: (0, -1),
            attacker: -1,
            held_unit: -1,
            confusion: 0,
            local_group: None,
            marker_flags: 0,
            own_definition: None,
        }
    }
}

impl crate::engine_rng::EngineWorld {
    pub fn combat(&self, i: usize) -> MonsterCombat {
        self.combat.get(i).copied().unwrap_or_default()
    }

    pub fn own_definition(&self, i: usize) -> usize {
        self.combat(i)
            .own_definition
            .unwrap_or(self.monsters[i].definition)
    }

    pub fn name_index(&self, i: usize) -> i8 {
        self.combat(i).name_index
    }

    pub fn name_counter(&self, d: usize) -> i16 {
        self.monster_definitions
            .get(d)
            .map_or(-1, |d| d.name_counter)
    }

    pub fn take_selection_flags(&mut self, selected: &[usize]) -> bool {
        let mut changed = false;
        for &i in selected {
            if let Some(c) = self.combat.get_mut(i)
                && c.byte_8d != 0
            {
                c.byte_8d = 0;
                changed = true;
            }
        }
        changed
    }

    pub fn combat_mut(&mut self, i: usize) -> &mut MonsterCombat {
        if self.combat.len() <= i {
            self.combat
                .resize(self.monsters.len().max(i + 1), MonsterCombat::default());
        }
        &mut self.combat[i]
    }

    pub fn execute_target_command(
        &mut self,
        source: i32,
        player: i16,
        flags: u16,
        attackers: &[usize],
        targets: &[usize],
    ) -> Result<(), Unmodelled> {
        let attackers = self.filter_command_monsters(attackers, source, player);
        self.target_the_rest(flags, &attackers, targets)
    }

    pub fn execute_matched_target_command(
        &mut self,
        source: i32,
        player: i16,
        flags: u16,
        attackers: &[Option<usize>],
        targets: &[Option<usize>],
    ) -> Result<(), Unmodelled> {
        let attackers = self.filter_command_list(attackers, source, player);
        let mut rest_attackers = Vec::new();
        let mut rest_targets = Vec::new();
        for (i, &a) in attackers.iter().enumerate() {
            let Some(&t) = targets.get(i) else {
                return Err(Unmodelled::new(
                    draw_site::MATCHED_TARGETS_SHORT,
                    "execute_target_command: a matched target list shorter than the attackers",
                ));
            };
            if let Some(t) = t {
                rest_targets.push(t);
            }
            match (a, t) {
                (Some(a), Some(t)) => self.target_monster(a, u32::from(flags), t as i16)?,
                (Some(a), None) => rest_attackers.push(a),
                (None, _) => {}
            }
        }
        self.target_the_rest(flags, &rest_attackers, &rest_targets)
    }

    fn target_the_rest(
        &mut self,
        flags: u16,
        attackers: &[usize],
        targets: &[usize],
    ) -> Result<(), Unmodelled> {
        if attackers.is_empty() || targets.is_empty() {
            return Ok(());
        }
        if flags & 8 == 0 {
            return self.share_targets(u32::from(flags), attackers, targets);
        }
        let c = self.centerpoint(attackers);
        let mut t: i16 = -1;
        let mut best = 0i32;
        for &j in targets {
            let p = self.monsters[j].position;
            let (dx, dy) = (p.0.wrapping_sub(c.0), p.1.wrapping_sub(c.1));
            if dx > 0x77ff || dy > 0x77ff {
                continue;
            }
            let d = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
            if t == -1 || d < best {
                t = j as i16;
                best = d;
            }
        }
        for &a in attackers {
            if flags & 0x10 != 0 && self.monsters[a].flags & 0x18 != 0 {
                continue;
            }
            self.target_monster(a, u32::from(flags), t)?;
        }
        Ok(())
    }

    pub fn filter_command_list(
        &mut self,
        list: &[Option<usize>],
        source: i32,
        player: i16,
    ) -> Vec<Option<usize>> {
        list.iter()
            .map(|&m| {
                let m = m?;
                self.filter_command_monsters(&[m], source, player)
                    .first()
                    .copied()
            })
            .collect()
    }

    pub fn filter_command_monsters(
        &mut self,
        monsters: &[usize],
        source: i32,
        player: i16,
    ) -> Vec<usize> {
        if source == -1 {
            return self.filter_player_command(monsters, player);
        }
        let mut kept = Vec::new();
        for &a in monsters {
            self.set_order_source(a, source as i16);
            if self.monsters[a].flags_8 & 0x4100 == 0 {
                kept.push(a);
            }
        }
        kept
    }

    pub(crate) fn filter_player_command(&mut self, attackers: &[usize], player: i16) -> Vec<usize> {
        let team = self
            .players
            .players
            .get(usize::try_from(player).unwrap_or(usize::MAX))
            .map(|p| i32::from(p.team))
            .filter(|&t| {
                t != -1
                    && self
                        .players
                        .teams
                        .get(t as usize)
                        .is_some_and(|r| r.player == player)
            })
            .unwrap_or(-1);
        let mut kept = Vec::new();
        for &a in attackers {
            self.set_order_source(a, -1);
            let m = &self.monsters[a];
            let mine = m.owner == player || i32::from(m.team) == team;
            if m.flags & 0x1000_0000 != 0 || !mine || m.mode == 14 || m.flags & 0x4000_0000 != 0 {
                continue;
            }
            let m = &mut self.monsters[a];
            m.flags |= 0x800_0000;
            if m.flags_8 & 0x4100 == 0 {
                kept.push(a);
            }
        }
        kept
    }

    fn set_order_source(&mut self, i: usize, source: i16) {
        if let Some(Some(mv)) = self.movers.get_mut(i) {
            mv.action = source;
        }
    }

    pub(crate) fn share_targets(
        &mut self,
        flags: u32,
        attackers: &[usize],
        targets: &[usize],
    ) -> Result<(), Unmodelled> {
        let n = attackers.len();
        let m = targets.len() as i32;
        let c = self.centerpoint(attackers);
        let mut nearest = 0x7d000i32;
        let d: Vec<i32> = targets
            .iter()
            .map(|&j| {
                let p = self.monsters[j].position;
                let (dx, dy) = (p.0.wrapping_sub(c.0), p.1.wrapping_sub(c.1));
                let d = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
                nearest = nearest.min(d.wrapping_neg());
                d
            })
            .collect();
        let mut order: Vec<usize> = (0..targets.len()).collect();
        crate::formations::heap_sort(&mut order, |&a, &b| d[b].wrapping_sub(d[a]));
        let mut done = vec![false; n];
        let mut free = n as i32;
        if flags & 0x10 != 0 {
            for (k, &a) in attackers.iter().enumerate() {
                if self.monsters[a].flags & 0x18 != 0 {
                    done[k] = true;
                    free -= 1;
                }
            }
        }
        let version = self.formation_globals.gameplay_version;
        if version > game_options::GAMEPLAY_1_3
            && version <= game_options::GAMEPLAY_1_8_3
            && flags & 1 == 0
        {
            let load = attackers.iter().any(|&a| {
                let def = &self.monster_definitions[self.monsters[a].definition].ai;
                def.class == 1 && def.byte_198 & 0x40 != 0
            }) && nearest > 0x1400;
            if load {
                return Err(Unmodelled::new(
                    draw_site::SHARE_TARGETS_LOAD_PATH,
                    "sub_5087d0: the load path (every target distance wrapped)",
                ));
            }
        }
        while free > 0 {
            let start = (m - free).max(0);
            for &j in &order[start as usize..] {
                let p = self.monsters[targets[j]].position;
                let mut best: Option<(usize, i32)> = None;
                for (k, &a) in attackers.iter().enumerate() {
                    if done[k] {
                        continue;
                    }
                    let q = self.monsters[a].position;
                    let (dx, dy) = (p.0.wrapping_sub(q.0), p.1.wrapping_sub(q.1));
                    let e = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx));
                    if best.is_none_or(|(_, b)| e < b) {
                        best = Some((k, e));
                    }
                }
                let Some((k, _)) = best else {
                    return Err(Unmodelled::new(
                        draw_site::SHARE_TARGETS_NO_FREE_ATTACKER,
                        "sub_5087d0: a pass without a free attacker",
                    ));
                };
                self.target_monster(attackers[k], flags, targets[j] as i16)?;
                done[k] = true;
            }
            free = free - m + start;
        }
        Ok(())
    }

    pub fn target_monster(&mut self, i: usize, flags: u32, t: i16) -> Result<(), Unmodelled> {
        if let Some(trace) = self.target_log.as_mut() {
            trace.push((i, flags, t));
        }
        self.drop_waypoints(i)?;
        let old8 = self.monsters[i].flags_8;
        self.monsters[i].flags_8 = old8 & !0x4_0000;
        self.combat_mut(i).pending = -1;
        let mode = self.monsters[i].mode;
        let mut skip_clear = false;
        if matches!(mode, 7 | 8) {
            let def = &self.monster_definitions[self.monsters[i].definition].ai;
            let a = def.attack(self.monsters[i].anim_state as i8);
            if a.is_some_and(|a| a.flags_3c & 4 != 0) {
                self.monsters[i].mode = 6;
                skip_clear = true;
            } else {
                self.combat_mut(i).pending = t;
                let m = &mut self.monsters[i];
                if flags & 2 != 0 {
                    m.flags_8 |= 0x400_0000;
                } else {
                    m.flags_8 &= !0x400_0000;
                }
                if old8 & 0x4_0000 != 0 && m.flags & 0x18 != 0 {
                    m.flags |= 1;
                }
                return Ok(());
            }
        }
        let m = &mut self.monsters[i];
        if !skip_clear {
            m.flags_8 = old8 & 0xf7fb_fdff;
            let f = m.flags;
            m.flags = if f & 8 != 0 { f & !0x10 } else { f & !0x11 };
        }
        if flags & 4 != 0 && m.flags & 0x2000_0008 == 0x2000_0008 {
            return Ok(());
        }
        m.object.flags &= !crate::collide::OBJECT_POSED;
        let old = m.flags;
        m.flags_8 &= 0xf7df_f9ff;
        let mut f = old & 0x7f07_bf6f;
        if old & 8 == 0 {
            f = old & 0x7f07_bf6e;
        }
        m.flags = f;
        if t == -1 {
            m.flags = if f & 0x10 != 0 { f & !8 } else { f & !9 };
            self.combat_mut(i).target = -1;
            return Ok(());
        }
        let same = f & 8 != 0 && self.combat(i).target == t;
        let mut r9 = flags & 1 != 0;
        if same && !(flags & 1 != 0 && f & 0x20_0000 == 0) {
            r9 = flags & 1 != 0 && f & 0x20_0000 != 0;
        } else {
            f = (f & 0xdffe_feff) | 0x8020;
        }
        let byte_1b5 = self.monster_definitions[self.monsters[i].definition]
            .ai
            .byte_1b5;
        let mut e = f | 0xf;
        if byte_1b5 & 0x20 != 0 {
            if flags & 0x22 == 0 {
                e = (f & !2) | 0xd;
            }
            e |= 0x20;
        }
        if flags & 0x22 != 0 {
            e |= 2;
        }
        if r9 {
            e |= 0x20_0000;
        }
        let m = &mut self.monsters[i];
        if flags & 0x100 != 0 {
            m.flags_8 |= 0x20_0000;
        }
        if flags & 2 != 0 {
            e |= 0x80_0000;
        } else {
            e &= !0x80_0000;
        }
        m.flags = e;
        self.combat_mut(i).target = t;
        Ok(())
    }

    pub(crate) fn drop_waypoints(&mut self, i: usize) -> Result<(), Unmodelled> {
        self.drop_waypoint_count(i)?;
        if let Some(Some(mv)) = self.movers.get_mut(i) {
            mv.waypoint = 0;
        }
        Ok(())
    }

    pub(crate) fn apply_pending_orders(&mut self) -> Result<(), Unmodelled> {
        let orders = std::mem::take(&mut self.pending_orders);
        let mut r = Ok(());
        for o in orders {
            let lent = self.movers.get_mut(o.monster).and_then(Option::take);
            match lent {
                Some(mut mv) if r.is_ok() => {
                    r = self.order_monster(&o, &mut mv);
                    self.movers[o.monster] = Some(mv);
                }
                Some(mv) => {
                    self.movers[o.monster] = Some(mv);
                    self.pending_orders.push(o);
                }
                None => self.pending_orders.push(o),
            }
        }
        r
    }

    pub(crate) fn drop_waypoint_count(&mut self, i: usize) -> Result<(), Unmodelled> {
        while let Some(k) = self.pending_orders.iter().position(|o| o.monster == i) {
            let o = self.pending_orders.remove(k);
            if let Some(mut mv) = self.movers.get_mut(i).and_then(Option::take) {
                let r = self.order_monster(&o, &mut mv);
                self.movers[i] = Some(mv);
                r?;
            } else {
                self.set_monster_waypoints_flags(i, &o.waypoints, o.flags)?;
            }
        }
        if let Some(Some(mv)) = self.movers.get_mut(i) {
            mv.waypoints.clear();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationAttackRun {
    Walk {
        monster: usize,
        f4: u32,
        f8: u32,
        location: (i32, i32, i32),
        target: i16,
        position: (i32, i32),
    },
    Walked {
        monster: usize,
        result: i32,
        f4: u32,
        leg: (i32, i32),
    },
    Choose {
        monster: usize,
        mode: i16,
    },
    Chosen {
        monster: usize,
        attack: i16,
    },
    Leg {
        monster: usize,
        point: (i32, i32),
        r14: i32,
    },
    Attack {
        monster: usize,
        attack: i16,
        mode: u8,
        f4: u32,
        position: (i32, i32),
    },
    Attacked {
        monster: usize,
        result: bool,
        f4: u32,
        velocity: i16,
    },
}

thread_local! {
    pub static LOCATION_ATTACK_LOG: std::cell::RefCell<Option<Vec<LocationAttackRun>>> =
        const { std::cell::RefCell::new(None) };
}

pub(crate) fn log_location_attack(run: impl FnOnce() -> LocationAttackRun) {
    LOCATION_ATTACK_LOG.with(|l| {
        if let Some(l) = l.borrow_mut().as_mut() {
            l.push(run());
        }
    });
}

#[derive(Debug, Clone, Copy)]
pub struct Own {
    pub position: (i32, i32),
    pub timer: i16,
    pub speed: i16,
    pub facing: u16,
    pub desired_facing: u16,
}

impl Own {
    pub fn of(mv: &EngineMover) -> Self {
        Self {
            position: (mv.x, mv.y),
            timer: mv.pose_timer,
            speed: mv.speed,
            facing: mv.facing,
            desired_facing: mv.desired_facing,
        }
    }
}

impl crate::engine_rng::EngineWorld {
    fn aim_point(&self, j: usize) -> (i32, i32, i32) {
        let m = &self.monsters[j];
        let bottom = self
            .collections
            .get(&m.object.collection)
            .and_then(|h| m.object.bounds(h))
            .map_or(0, |b| i32::from(b.1));
        (m.position.0, m.position.1, m.object.z - bottom)
    }

    fn position_of(&self, i: usize, own: &Own, j: usize) -> (i32, i32) {
        if i == j {
            own.position
        } else {
            self.monsters[j].position
        }
    }

    fn attack_point(&self, i: usize, own: &Own) -> Result<(i32, i32, i32), Unmodelled> {
        let m = &self.monsters[i];
        if m.flags & 0x10 != 0 {
            let c = self.combat(i);
            return Ok((c.location.0, c.location.1, i32::from(c.location_z)));
        }
        let t = self.combat(i).target;
        let Ok(j) = usize::try_from(t) else {
            return Err(Unmodelled::new(
                draw_site::ATTACK_POINT_NO_TARGET,
                format!("monster {}: no target +0x62", m.identifier),
            ));
        };
        let mut p = self.aim_point(j);
        if j == i {
            p.0 = own.position.0;
            p.1 = own.position.1;
        }
        Ok(p)
    }

    pub(crate) fn attack_definition(
        &self,
        i: usize,
        a: i16,
    ) -> Result<Option<AttackDefinition>, Unmodelled> {
        let def = &self.monster_definitions[self.monsters[i].definition].ai;
        if a == -0x45 {
            let special = self.combat(i).special;
            if special != -1 {
                let raw = &self.catalog.artifacts[special as usize].1;
                return Ok(Some(AttackDefinition::at(raw, 0x54)));
            }
            if self.formation_globals.gameplay_version <= game_options::GAMEPLAY_1_5_1 {
                return Err(Unmodelled::new(
                    draw_site::SPECIAL_ATTACK_OLD_GAMEPLAY,
                    "the special ability's attack without an artifact (gameplay <= 1.5.1)",
                ));
            }
            return Ok(def.attack(0).copied());
        }
        let version = self.formation_globals.gameplay_version;
        let n = def.attacks.len() as i16;
        let s = attack_slot(a, n, version);
        match usize::try_from(s).ok().and_then(|s| def.attacks.get(s)) {
            Some(atk) => Ok(Some(*atk)),
            None => Err(Unmodelled::new(
                if version > game_options::GAMEPLAY_1_6 {
                    draw_site::ATTACK_DEFINITION_MISSING
                } else {
                    draw_site::ATTACK_DEFINITION_MISSING_OLD
                },
                format!(
                    "get_monster_attack_definition: attack {a} of {n} (gameplay {version:#x}) \
                     is outside the definition's attacks"
                ),
            )),
        }
    }

    fn preferred_attack(
        &self,
        i: usize,
        def: &AiDefinition,
        dist: i32,
        (t1000, t400, t8): (bool, bool, bool),
    ) -> Result<i16, Unmodelled> {
        let version = self.formation_globals.gameplay_version;
        let count = def.attacks.len();
        let slot = |k: usize| -> Result<&AttackDefinition, Unmodelled> {
            let at = if version > game_options::GAMEPLAY_1_6 {
                k.min(count.wrapping_sub(1))
            } else {
                k
            };
            def.attacks.get(at).ok_or_else(|| {
                Unmodelled::new(
                    draw_site::PREFERRED_ATTACK_SLOT,
                    format!(
                        "monster {}: the preferred attack's slot {k} of {count} attacks",
                        self.monsters[i].identifier
                    ),
                )
            })
        };
        let (mut found, mut best_max, mut pref) = (false, -1i32, -1i16);
        let (mut noted, mut noted_min) = (-1i16, 0x7d000i32);
        for k in 0..4usize {
            let a = slot(k)?;
            let max = i32::from(a.maximum_range);
            if best_max != -1 && max >= best_max && found {
                continue;
            }
            let flags = a.flags;
            if max == 0 || flags & 0xc0 != 0 {
                continue;
            }
            if flags & 0x800 != 0 {
                if !t1000 || flags & 0x1000 != 0 {
                    continue;
                }
            } else if flags & 0x1000 != 0 && t1000 {
                continue;
            }
            if version > game_options::GAMEPLAY_1_5_1 {
                let b = a.flags_3c as u8;
                if b & 8 != 0 {
                    if !t8 || b & 0x10 != 0 {
                        continue;
                    }
                } else if b & 0x10 != 0 && t8 {
                    continue;
                }
                if b & 0x40 != 0 {
                    if !t400 || b & 0x80 != 0 {
                        continue;
                    }
                } else if b & 0x80 != 0 && t400 {
                    continue;
                }
            }
            if !self.attack_usable(i, a) {
                continue;
            }
            let min = i32::from(a.minimum_range);
            if flags & 0x200 != 0 && min < noted_min {
                noted = k as i16;
                noted_min = min;
            }
            if best_max != -1 && max >= best_max {
                continue;
            }
            pref = k as i16;
            best_max = max;
            found = flags != 0;
        }
        if pref == -1 {
            return Ok(0);
        }
        if slot(pref as usize)?.flags & 0x200 != 0 || dist < noted_min {
            return Ok(pref);
        }
        Ok(if noted != -1 { noted } else { 0 })
    }

    fn special_attack(&self, i: usize) -> Result<Option<(AttackDefinition, bool)>, Unmodelled> {
        let c = self.combat(i);
        if c.special == -1 {
            return Ok(None);
        }
        let id = self.monsters[i].identifier;
        let raw = &self.catalog.artifacts[c.special as usize].1;
        let flags = crate::engine_rng::be_i32(raw, 0) as u32;
        if flags & 1 == 0 {
            return Ok(None);
        }
        if crate::engine_rng::be_i16(raw, 0x0a) != 0 {
            if c.carried == -1 {
                return Err(Unmodelled::new(
                    draw_site::SPECIAL_CHARGES_NOT_CARRIED,
                    format!("monster {id}: a special ability with charges and no carried object"),
                ));
            }
            let charges = self
                .projectiles
                .iter()
                .find(|p| p.object.index == i32::from(c.carried))
                .map(|p| p.object.charges);
            let Some(charges) = charges else {
                return Err(Unmodelled::new(
                    draw_site::SPECIAL_OBJECT_NOT_PROJECTILE,
                    format!("monster {id}: the special ability's object isn't a projectile's"),
                ));
            };
            if charges <= 0 {
                return Ok(None);
            }
        }
        let monster = crate::engine_rng::be_i32(raw, 0x04);
        let t = mandrake_common::FourCC::new([raw[4], raw[5], raw[6], raw[7]]);
        if flags & 0x10 != 0
            && monster != -1
            && t != self.monster_definitions[self.own_definition(i)].tag
        {
            if flags & 0x100 == 0 {
                return Ok(None);
            }
            let Some(theirs) = self.monster_definitions.iter().find(|d| d.tag == t) else {
                return Err(Unmodelled::new(
                    draw_site::SPECIAL_MONSTER_NOT_LOADED,
                    format!("monster {id}: the special ability's monster {t} isn't loaded"),
                ));
            };
            let ours = &self.monster_definitions[self.monsters[i].definition];
            if definition_collection(theirs) != definition_collection(ours) {
                return Ok(None);
            }
        }
        let only = flags & 0x8000 != 0
            && self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1;
        Ok(Some((AttackDefinition::at(raw, 0x54), only)))
    }

    pub(crate) fn attack_projectile(
        &self,
        a: &AttackDefinition,
    ) -> Result<Option<(&mandrake_tag::projectile::ProjectileDefinition, u16)>, Unmodelled> {
        let Some(tag) = a.projectile else {
            return Ok(None);
        };
        let Some(p) = self.catalog.projectiles.get(&tag) else {
            return Err(Unmodelled::new(
                draw_site::ATTACK_PROJECTILE_MISSING,
                format!("projectile {tag} is not in the catalog"),
            ));
        };
        let obje = p
            .object
            .and_then(|o| self.catalog.objects.get(&o))
            .map_or(0, |o| o.flags);
        Ok(Some((p, obje)))
    }

    fn ballistic_range(
        &self,
        def: &AiDefinition,
        a: &AttackDefinition,
        dz: i32,
    ) -> Result<(i32, bool), Unmodelled> {
        let (min, mut max) = (i32::from(a.minimum_range), i32::from(a.maximum_range));
        let Some((p, obje)) = self.attack_projectile(a)? else {
            return Ok((max, false));
        };
        let sum = i32::from(a.velocity) + i32::from(a.velocity_delta);
        if p.flags & 0x100_2000 != 0 || obje & 4 != 0 || sum == 0 {
            return Ok((max, false));
        }
        if def.byte_199 & 2 == 0 {
            let v = a.velocity.wrapping_add(a.velocity_delta);
            if v != 0 {
                max += (0xcc_0000 / i32::from(v)).wrapping_mul(dz) >> 16;
            }
            if min >= max {
                max = min;
            }
        }
        Ok((max, true))
    }

    fn attack_usable(&self, i: usize, a: &AttackDefinition) -> bool {
        let c = self.combat(i);
        if a.flags & 0x20 != 0 && c.carried == -1 {
            return false;
        }
        if a.flags & 0x10 != 0 {
            if a.repetitions >= 0 {
                if c.ammunition == 0 {
                    return false;
                }
            } else if c.ammunition > 0 && a.flags & 0x80 == 0 {
                return false;
            }
        }
        let cost = a.mana;
        if cost > 0 && c.mana < cost {
            return false;
        }
        if cost < 0 {
            let def = &self.monster_definitions[self.monsters[i].definition].ai;
            return i32::from(c.mana) <= i32::from(cost) + i32::from(def.word(0x214));
        }
        true
    }

    pub fn choose_attack(&self, i: usize, mode: i16, own: &Own) -> Result<i16, Unmodelled> {
        let location = self.monsters[i].flags & 0x10 != 0;
        if location {
            log_location_attack(|| LocationAttackRun::Choose { monster: i, mode });
        }
        let a = self.choose_attack_inner(i, mode, own)?;
        if location {
            log_location_attack(|| LocationAttackRun::Chosen {
                monster: i,
                attack: a,
            });
        }
        Ok(a)
    }

    fn choose_attack_inner(&self, i: usize, mode: i16, own: &Own) -> Result<i16, Unmodelled> {
        let m = &self.monsters[i];
        let def = &self.monster_definitions[m.definition].ai;
        let c = self.combat(i);
        let target = (m.flags & 8 != 0)
            .then(|| usize::try_from(c.target).ok())
            .flatten();
        let mut dist = 0;
        let mut dz_point = 0;
        let (mut t1000, mut t400, mut t8) = (false, false, false);
        let mut pref: i16 = -1;
        if !matches!(mode, 2 | 3) {
            let p = self.attack_point(i, own)?;
            dz_point = p.2;
            dist = crate::engine_move::distance2d(p.0 - own.position.0, p.1 - own.position.1);
            if let Some(t) = target {
                let tf = self.monster_definitions[self.monsters[t].definition].flags;
                t1000 = tf & 0x1000 != 0;
                t400 = tf & 0x400 != 0;
                t8 = tf & 8 != 0;
            }
            if m.flags_8 & 0x20_0000 != 0
                && self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3
            {
                pref = self.preferred_attack(i, def, dist, (t1000, t400, t8))?;
            }
        }
        let special = self.special_attack(i)?;
        let only_special = special.as_ref().is_some_and(|(_, only)| *only);
        let dz = m.object.z - dz_point;
        let mut result: i16 = -1;
        let mut best_r12 = 0;
        let attacks = special
            .iter()
            .map(|(a, _)| (-0x45, a))
            .chain(def.attacks.iter().enumerate().map(|(k, a)| (k as i16, a)));
        for (k, a) in attacks {
            let mut class_ok = true;
            'attack: {
                let flags = a.flags;
                let proj = self.attack_projectile(a)?;
                let (r12, in_range, mut usable);
                match mode {
                    2 => {
                        r12 = 0;
                        in_range = true;
                        usable = flags & 0x80 != 0;
                    }
                    3 => {
                        r12 = 0;
                        in_range = true;
                        usable = flags & 0xc0 == 0xc0;
                    }
                    _ => {
                        usable = !(pref != -1 && m.flags_8 & 0x20_0000 != 0 && pref != k);
                        let mut min = i32::from(a.minimum_range);
                        let (max, ballistic) = self.ballistic_range(def, a, dz)?;
                        if ballistic && (flags as i16) < 0 && dz < -0x300 && dist <= 0x7ff {
                            min = 0;
                        }
                        if mode == 1 && m.flags & 0x10_0000 != 0 {
                            let tspeed = match target {
                                Some(t) if t != i => self.monsters[t].speed,
                                _ => own.speed,
                            };
                            let ms = def.word(0x7c) as u16;
                            let f = if ms == 0 {
                                0x2d
                            } else {
                                (0x8000 / i32::from(ms)) + 0x2d
                            };
                            dist -= i32::from((f * i32::from(tspeed as u16)) as i16);
                        }
                        if dist < min {
                            r12 = 1;
                            in_range = false;
                        } else {
                            r12 = if dist > max { 2 } else { 0 };
                            in_range = dist <= max;
                        }
                        if let (Some(t), Some((p, _))) = (target, proj)
                            && self.monsters[t].flags_8 & 0x100 != 0
                            && p.word(0x6a) < 0
                        {
                            usable = false;
                        }
                    }
                }
                if matches!(mode, 2 | 3) {
                    class_ok = usable;
                } else {
                    if flags & 0x80 != 0 {
                        class_ok = m.flags & 0x80_0000 != 0;
                        usable &= class_ok;
                    } else if m.flags & 0x80_0000 != 0 {
                        usable = false;
                        class_ok = false;
                    } else if flags & 0x200 == 0 {
                        match def.class {
                            0 if mode == 0 => {
                                if m.flags & 0x20_0000 != 0 {
                                    usable = false;
                                    class_ok = false;
                                }
                            }
                            1 => {
                                class_ok = m.flags & 0x20_0000 != 0;
                                usable &= class_ok;
                            }
                            _ => {}
                        }
                    }
                    if mode == 1 {
                        usable &= r12 == 0;
                    } else if result != -1 && r12 == 1 {
                        usable &= best_r12 & 2 == 0;
                    }
                    if (flags & 0x800 != 0 && !t1000) || (flags & 0x1000 != 0 && t1000) {
                        usable = false;
                    } else {
                        let f = a.flags_3c;
                        if (f & 8 != 0 && !t8) || (f & 0x10 != 0 && t8) || (f & 0x40 != 0 && !t400)
                        {
                            usable = false;
                        }
                        if f & 0x80 != 0 && t400 {
                            usable = false;
                        }
                    }
                }
                let can = self.attack_usable(i, a);
                let location = m.flags & 0x11 == 0x11;
                if !can && !location {
                    break 'attack;
                }
                if !can {
                    usable = false;
                }
                if location {
                    let mode2 = mode == 2 && flags & 0x80 != 0 && m.mode != 8;
                    if flags & 0x40 != 0 {
                        if !mode2 {
                            break 'attack;
                        }
                    } else if !mode2
                        && proj.is_some_and(|(p, _)| p.flags & 0x2000_2000 == 0x2000_2000)
                    {
                        break 'attack;
                    }
                }
                if !usable {
                    break 'attack;
                }
                if matches!(mode, 2 | 3) || in_range {
                    return Ok(k);
                }
                best_r12 = r12;
                result = k;
            }
            if k == -0x45 && only_special && class_ok {
                return Ok(result);
            }
        }
        Ok(result)
    }

    pub fn target_distance(&self, i: usize, own: &Own) -> Result<i32, Unmodelled> {
        let m = &self.monsters[i];
        let t = if m.flags & 8 == 0 {
            let Ok(a) = usize::try_from(self.combat(i).attacker) else {
                return Ok(0x7d000);
            };
            if a >= self.monsters.len()
                || self.monsters_are_allied(i, a)
                || self.monsters[a].flags_8 & 0x200_0004 != 0
            {
                return Ok(0x7d000);
            }
            a
        } else {
            let Ok(t) = usize::try_from(self.combat(i).target) else {
                return Ok(0x7d000);
            };
            if self.monsters[t].flags_8 & 0x200_0000 != 0 {
                return Ok(0x7d000);
            }
            t
        };
        let p = self.position_of(i, own, t);
        Ok(crate::engine_move::distance2d(
            p.0 - own.position.0,
            p.1 - own.position.1,
        ))
    }

    pub fn walk_for_attack(&mut self, i: usize, mv: &mut EngineMover) -> Result<i32, Unmodelled> {
        let location = self.monsters[i].flags & 0x10 != 0;
        if location {
            let (m, c) = (&self.monsters[i], self.combat(i));
            log_location_attack(|| LocationAttackRun::Walk {
                monster: i,
                f4: m.flags,
                f8: m.flags_8,
                location: (c.location.0, c.location.1, i32::from(c.location_z)),
                target: c.target,
                position: (mv.x, mv.y),
            });
        }
        let r = self.walk_for_attack_inner(i, mv)?;
        if location {
            log_location_attack(|| LocationAttackRun::Walked {
                monster: i,
                result: r,
                f4: self.monsters[i].flags,
                leg: mv.target,
            });
        }
        Ok(r)
    }

    fn walk_for_attack_inner(&mut self, i: usize, mv: &mut EngineMover) -> Result<i32, Unmodelled> {
        let own = Own::of(mv);
        let m = &self.monsters[i];
        let c = self.combat(i);
        let target = if m.flags & 8 != 0 {
            usize::try_from(c.target).ok()
        } else {
            None
        };
        let p = if m.flags & 0x10 != 0 {
            (c.location.0, c.location.1, i32::from(c.location_z))
        } else {
            let Ok(t) = usize::try_from(c.target) else {
                return Err(Unmodelled::new(
                    draw_site::WALK_FOR_ATTACK_NO_TARGET,
                    format!("monster {}: no target +0x62", m.identifier),
                ));
            };
            let mut p = self.aim_point(t);
            if t == i {
                p.0 = own.position.0;
                p.1 = own.position.1;
            }
            if own.timer > 0 {
                let v = self.monsters[t].object.velocity;
                p.0 += i32::from(v[0]) * i32::from(own.timer);
                p.1 += i32::from(v[1]) * i32::from(own.timer);
            }
            p
        };
        let d = crate::engine_move::distance2d(p.0 - own.position.0, p.1 - own.position.1);
        let m = &mut self.monsters[i];
        if d <= 0x7ff {
            m.flags |= 0x100;
        } else {
            m.flags &= !0x100;
        }
        if m.flags & 0x8_0000 != 0 {
            let near = target.is_none_or(|t| {
                d < i32::from(
                    self.monster_definitions[self.monsters[t].definition]
                        .ai
                        .word(0x1f4),
                )
            });
            if near {
                return Ok(i32::MIN);
            }
            self.monsters[i].flags &= !0x10_0000;
            return Ok(0);
        }
        let a = self.choose_attack(i, 0, &own)?;
        if a == -1 {
            return Ok(0);
        }
        let Some(atk) = self.attack_definition(i, a)? else {
            return Ok(0);
        };
        let def = &self.monster_definitions[self.monsters[i].definition].ai;
        let min = i32::from(atk.minimum_range);
        let dz = self.monsters[i].object.z - p.2;
        let (mut max, ballistic) = self.ballistic_range(def, &atk, dz)?;
        let mut min = min;
        if ballistic && (atk.flags as i16) < 0 && dz < -0x300 && d <= 0x7ff {
            min = 0;
        }
        if !ballistic {
            max = i32::from(atk.maximum_range);
        }
        if d < min {
            return Ok(d - min);
        }
        if d >= max && d - i32::from(own.speed) * i32::from(own.timer) >= min {
            return Ok(d - max);
        }
        mv.target = own.position;
        Ok(0)
    }

    pub(crate) fn hold_timer(&self, i: usize) -> i16 {
        let m = &self.monsters[i];
        if self.game_options & mandrake_common::game_options::COOPERATIVE == 0 || m.owner == -1 {
            return 15;
        }
        let human = self
            .players
            .players
            .get(m.owner as usize)
            .is_some_and(|p| p.field_09 == 1);
        match (human, self.difficulty) {
            (true, 0) => 45,
            (true, 1) => 30,
            _ => 15,
        }
    }
}

impl crate::engine_rng::EngineWorld {
    fn monsters_in_box(&self, x0: i32, y0: i32, n: i32) -> Vec<usize> {
        let mut out = self.monsters_in_cell_box(x0, y0, n.max(0), n.max(0));
        out.retain(|&j| !self.monsters[j].object.no_shape);
        out
    }

    fn clamp_box(&self, cx: i32, cy: i32, n: i32) -> (i32, i32) {
        let (w, h) = self.mesh_cells;
        let x = if cx < 0 { 0 } else { (w - n).min(cx) };
        let y = if cy < 0 { 0 } else { (h - n).min(cy) };
        (x, y)
    }

    pub fn find_target(&self, i: usize, mode: i32, own: &Own) -> Result<i16, Unmodelled> {
        let m = &self.monsters[i];
        if m.flags_8 & 0x100 != 0 {
            return Ok(-1);
        }
        let def = &self.monster_definitions[m.definition].ai;
        let c = self.combat(i);
        let mut tgt = (m.flags & 8 != 0)
            .then(|| usize::try_from(c.target).ok())
            .flatten();
        let (mut n, exclude) = match mode {
            1 if tgt.is_some() => (0x15, c.target),
            1 => ((i32::from(def.vision) >> 9) + 1, c.target),
            2 => (12, c.target),
            3 => (4, -1),
            _ if tgt.is_some() => (0x15, -1),
            _ => ((i32::from(def.vision) >> 9) + 1, -1),
        };
        let mut directional = mode != 2;
        let centre = if mode == 2 {
            let t = tgt.ok_or_else(|| {
                Unmodelled::new(
                    draw_site::FIND_TARGET_MODE_2_NONE,
                    format!("sub_515180 mode 2 of {} without a target", m.identifier),
                )
            })?;
            self.position_of(i, own, t)
        } else {
            own.position
        };
        if m.flags_8 & 0x4000 != 0 {
            n = 0x14;
            directional = false;
        }
        if (def.class == 1 || (def.class == 0 && def.byte_198 & 0x40 != 0))
            && m.flags & 4 == 0
            && m.flags_8 & 0x20 != 0
        {
            tgt = None;
        }
        if n <= 0 {
            return Ok(-1);
        }
        let (ex, ey) = if directional {
            let mut f = own.facing;
            if let Some(t) = tgt {
                let p = self.position_of(i, own, t);
                let (dx, dy) = (p.0 - own.position.0, p.1 - own.position.1);
                if dx | dy != 0 {
                    f = crate::engine_move::arctangent(dx, dy);
                }
            }
            let a = f.wrapping_add(0x2000);
            let t = (u32::from(a & 0x3fff) << 1) + 0x4000;
            match a >> 14 {
                0 => (0x10000 - t, 0x4000),
                1 => (0x4000, t),
                2 => (t, 0xc000),
                _ => (0xc000, 0x10000 - t),
            }
        } else {
            (0x8000, 0x8000)
        };
        let cx = (centre.0 >> 9) - ((ey as i32 * n) >> 16);
        let cy = (centre.1 >> 9) - ((ex as i32 * n) >> 16);
        let (x0, y0) = self.clamp_box(cx, cy, n);
        let me_media = m.object.flags & 0x80;
        let mut best: i16 = -1;
        let (mut best_priority, mut best_distance) = (0i32, 0x27_1000i32);
        for o in self.monsters_in_box(x0, y0, n) {
            let om = &self.monsters[o];
            if (om.object.flags & 0x80) != me_media {
                continue;
            }
            let cand = if self.monsters_are_allied(i, o) {
                if om.flags & 0x8_000c != 8 {
                    continue;
                }
                if self.combat(o).unit != c.unit {
                    continue;
                }
                self.combat(o).target
            } else if mode == 3 {
                continue;
            } else {
                o as i16
            };
            if cand == -1 || cand == exclude || cand as usize == i {
                continue;
            }
            let cu = cand as usize;
            let cp = self.position_of(i, own, cu);
            let mut d = crate::engine_move::distance2d(cp.0 - centre.0, cp.1 - centre.1);
            if mode == 1 {
                let op = self.position_of(i, own, o);
                let a =
                    crate::engine_move::arctangent(op.0 - own.position.0, op.1 - own.position.1);
                let diff = (i32::from(own.desired_facing) - i32::from(a)).abs();
                if diff > 0x1000 {
                    d += d / 8;
                    if diff > 0x2000 {
                        d += d / 6;
                        if diff > 0x4000 {
                            d += d / 4;
                        }
                    }
                }
            }
            let to_me = i32::from(self.combat(cu).target as isize == i as isize);
            let near = d < i32::from(def.engage_distance);
            let mut priority = match tgt {
                Some(t) => {
                    let same =
                        i32::from(self.monsters[t].definition == self.monsters[cu].definition);
                    if near {
                        same + 1 + to_me
                    } else if same > 0 {
                        same + to_me
                    } else {
                        0
                    }
                }
                None => i32::from(near) + to_me,
            };
            let mut take = best == -1
                || priority > best_priority
                || (priority == best_priority && best_distance > d);
            if mode == 1
                && best_distance > d
                && tgt.is_none_or(|t| self.monsters[t].definition == self.monsters[cu].definition)
                && m.flags & 0x20_0000 != 0
            {
                priority = best_priority;
                take = true;
            }
            if take && self.monsters[cu].flags_8 & 0x10 == 0 {
                best = cand;
                best_priority = priority;
                best_distance = d;
            }
        }
        Ok(best)
    }

    pub fn engage_monster(&mut self, i: usize, t: usize, flags: u32) -> Result<bool, Unmodelled> {
        let m = &self.monsters[i];
        if (m.flags_8 & 8 != 0 && m.flags & 4 == 0) || self.combat(i).pending != -1 {
            return Ok(false);
        }
        if m.flags_8 & 0x100 != 0
            || self.monsters_are_allied(i, t)
            || (m.flags & 2 != 0 && flags & 1 == 0)
        {
            return Ok(self.engaged_with(i, t));
        }
        let (def_class, def_byte_198, def_alert_radius) = {
            let d = &self.monster_definitions[self.monsters[i].definition].ai;
            (d.class, d.byte_198, d.alert_radius)
        };
        let m = &self.monsters[i];
        let old = m.flags;
        let mut table_fresh = true;
        let mut alert_skip = false;
        let mut had_target = false;
        if old & 8 != 0 {
            had_target = true;
            let c = usize::try_from(self.combat(i).target).map_err(|_| {
                Unmodelled::new(
                    draw_site::ENGAGE_NO_TARGET,
                    format!("monster {}: target bit without +0x62", m.identifier),
                )
            })?;
            let same_def = self.monsters[t].definition == self.monsters[c].definition;
            enum Next {
                Keep,
                Rules,
                Ordered,
                Switch,
            }
            let f8_20 = m.flags_8 & 0x20 != 0;
            let mut next = if def_class == 1 {
                if !f8_20 {
                    Next::Rules
                } else if old & 4 == 0 {
                    Next::Switch
                } else if old & 0x2000_0000 != 0 {
                    Next::Keep
                } else {
                    Next::Ordered
                }
            } else if f8_20 && flags & 4 != 0 && same_def {
                Next::Switch
            } else {
                Next::Rules
            };
            if matches!(next, Next::Rules) {
                next = if old & 0x2000_0000 != 0 {
                    Next::Keep
                } else if old & 4 != 0 {
                    Next::Ordered
                } else {
                    Next::Rules
                };
            }
            if matches!(next, Next::Ordered) {
                next = if flags & 1 == 0 && !same_def {
                    Next::Keep
                } else {
                    Next::Rules
                };
            }
            if matches!(next, Next::Rules) {
                let me = self.monsters[i].position;
                let tp = self.monsters[t].position;
                let cp = self.monsters[c].position;
                let to_me = |j: usize| usize::try_from(self.combat(j).target) == Ok(i);
                next = if !to_me(t) || to_me(c) {
                    Next::Keep
                } else if flags & 4 == 0 {
                    let dc = crate::engine_move::distance2d(cp.0 - me.0, cp.1 - me.1);
                    let dt = crate::engine_move::distance2d(tp.0 - me.0, tp.1 - me.1);
                    if dt < dc - 0x800 {
                        Next::Switch
                    } else {
                        Next::Keep
                    }
                } else if self.monster_definitions[self.monsters[i].definition].flags & 0x200 != 0 {
                    let f = self.monsters[i].facing;
                    let ac = crate::engine_rng::angular_difference(
                        f,
                        crate::engine_move::arctangent(cp.0 - me.0, cp.1 - me.1),
                    );
                    let at = crate::engine_rng::angular_difference(
                        f,
                        crate::engine_move::arctangent(tp.0 - me.0, tp.1 - me.1),
                    );
                    if at < ac - 0x1000 {
                        Next::Switch
                    } else {
                        Next::Keep
                    }
                } else {
                    Next::Switch
                };
            }
            if let Next::Keep = next {
                return Ok(self.engaged_with(i, t));
            }
            if t == c {
                return Ok(self.result_one(i));
            }
            if !f8_20 {
                table_fresh = false;
                alert_skip = true;
            }
        }
        let tclass = self.monster_definitions[self.monsters[t].definition]
            .ai
            .class;
        let fleeing = old & 0x8_0000 != 0;
        let mut flee = match def_class {
            0 | 3 => tclass == 2 || (def_byte_198 & 0x40 != 0 && fleeing),
            2 => {
                if !table_fresh || had_target {
                    fleeing
                } else if (old as i32) >= 0
                    || self.game_options
                        & (mandrake_common::game_options::TFL
                            | mandrake_common::game_options::ANTI_CLUMP)
                        == 0
                {
                    true
                } else {
                    fleeing
                }
            }
            4..=6 => true,
            _ => false,
        };
        let class1_retarget = def_class == 1 || def_class > 6;
        if !class1_retarget && self.monsters[i].flags_8 & 0x4100 != 0 {
            flee = false;
        }
        let m = &mut self.monsters[i];
        m.object.flags &= !crate::collide::OBJECT_POSED;
        m.flags_8 &= 0xffff_7fdf;
        let mut f = (old & 0xde2e_fe7f) | 0x8029;
        if def_class == 0 && def_byte_198 & 1 != 0 {
            f = (old & 0xde2e_fe7f) | 0x802b;
        }
        self.combat_mut(i).target = t as i16;
        if flee {
            self.monsters[i].flags = f | 0x8_0000;
            return Ok(self.result_one(i));
        }
        if old & 4 != 0 && def_class == 0 {
            f |= 4;
        }
        f &= !0x8_0000;
        self.monsters[i].flags = f;
        if alert_skip || flags & 2 == 0 {
            return Ok(self.result_one(i));
        }
        let r = i32::from(def_alert_radius);
        let n = 2 * i32::from(def_alert_radius >> 9) + 1;
        let me = self.monsters[i].position;
        let (x0, y0) = self.clamp_box((me.0 - r) >> 9, (me.1 - r) >> 9, n);
        let team = self.monsters[i].team;
        let tp = self.monsters[t].position;
        for a in self.monsters_in_box(x0, y0, n) {
            let am = &self.monsters[a];
            if am.flags & 0x40_0000 != 0 || am.team != team || a == i {
                continue;
            }
            let ad = &self.monster_definitions[am.definition].ai;
            if ad.class == 5 || ad.class != def_class {
                continue;
            }
            if def_class != 0
                && crate::engine_move::distance2d(am.position.0 - tp.0, am.position.1 - tp.1)
                    >= i32::from(ad.engage_distance) + i32::from(ad.vision)
            {
                continue;
            }
            self.engage_monster(a, t, 0)?;
        }
        Ok(self.result_one(i))
    }

    fn engaged_with(&mut self, i: usize, t: usize) -> bool {
        if self.monsters[i].flags & 8 == 0 || usize::try_from(self.combat(i).target) != Ok(t) {
            return false;
        }
        self.result_one(i)
    }

    fn result_one(&mut self, i: usize) -> bool {
        let def = &self.monster_definitions[self.monsters[i].definition].ai;
        if def.class == 0 && def.byte_198 & 1 != 0 {
            self.monsters[i].flags |= 2;
        }
        true
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttackClick {
    pub flag_2b: bool,
    pub flag_2a: bool,
    pub state_3: bool,
    pub tfl_mouse_actions: bool,
    pub r8: bool,
}

#[derive(Debug, Clone, Copy)]
struct TargetSpread {
    player: i16,
    owner_mode: i16,
    team: i16,
    team_mode: i16,
    collection: Option<[u8; 4]>,
    collection_mode: i16,
    bit19: u32,
}

impl crate::engine_rng::EngineWorld {
    pub fn local_attack_click(
        &self,
        selection: &[usize],
        target: usize,
        click: AttackClick,
    ) -> Result<(u16, Vec<usize>), Unmodelled> {
        let defs: Vec<&crate::engine_rng::MonsterDefinition> = selection
            .iter()
            .map(|&i| &self.monster_definitions[self.monsters[i].definition])
            .collect();
        let mut counts = [0i16; 7];
        for d in &defs {
            let c = usize::try_from(d.ai.class)
                .ok()
                .filter(|&c| c < 7)
                .ok_or_else(|| {
                    Unmodelled::new(
                        draw_site::LOCAL_ATTACK_SELECTION_CLASS,
                        format!("selection class {}", d.ai.class),
                    )
                })?;
            counts[c] += 1;
        }
        let mut class = 0usize;
        for (k, &c) in counts.iter().enumerate().skip(1) {
            if c > counts[class] {
                class = k;
            }
        }
        let word_198 = defs.iter().fold(0u16, |a, d| {
            a | u16::from(d.ai.byte_198) | u16::from(d.ai.byte_199) << 8
        });
        let byte_1b5 = defs.iter().fold(0u8, |a, d| a | d.ai.byte_1b5);
        let mut r12 = click.flag_2b;
        let mut flags: u16 = if click.state_3 { 2 } else { 0 };
        if word_198 & 4 != 0 && r12 && click.flag_2a {
            r12 = true;
            flags = 0x100;
        } else {
            match class {
                0 => {
                    let r13 = self.monsters[target].flags_8 & 0x4000 != 0;
                    r12 = if click.tfl_mouse_actions {
                        r12 || r13
                    } else {
                        !r12 || r13
                    };
                }
                1 => {
                    if word_198 & 0x40 != 0 {
                        r12 = !r12;
                    } else if byte_1b5 & 2 == 0 {
                        r12 = false;
                    }
                }
                _ => r12 = false,
            }
        }
        Ok(self.local_target_command(selection, target, flags, r12, click.r8))
    }

    pub fn local_target_command(
        &self,
        selection: &[usize],
        target: usize,
        flags: u16,
        ecx: bool,
        r8: bool,
    ) -> (u16, Vec<usize>) {
        let mut f = flags;
        let mut r9 = f & 0x80 == 0 && selection.len() > 2;
        if f & 1 != 0 {
            f |= 0x20;
        }
        if !ecx {
            for &a in selection {
                let d = &self.monster_definitions[self.monsters[a].definition];
                if d.ai.class == 1 {
                    r9 = false;
                    if f & 0x80 != 0 {
                        f |= 1;
                    }
                }
            }
        }
        if !r9 {
            return (f, vec![target]);
        }
        let tdef = &self.monster_definitions[self.monsters[target].definition];
        let bit19 = (tdef.flags >> 19) & 1;
        let (player, owner_mode) = if r8 { (self.local_player, 1) } else { (-1, 3) };
        let spread = if ecx {
            TargetSpread {
                player,
                owner_mode,
                team: self.monsters[selection[0]].team,
                team_mode: if r8 { 3 } else { 2 },
                collection: None,
                collection_mode: 3,
                bit19,
            }
        } else {
            TargetSpread {
                player,
                owner_mode,
                team: self.monsters[target].team,
                team_mode: if r8 { 3 } else { 0 },
                collection: Some(definition_collection(tdef)),
                collection_mode: 0,
                bit19,
            }
        };
        (f, self.spread_target(target, spread))
    }

    fn spread_target(&self, target: usize, s: TargetSpread) -> Vec<usize> {
        let (w, h) = self.mesh_cells;
        let orig = &self.monsters[target];
        let odef = &self.monster_definitions[orig.definition];
        let local_team = self.local_team();
        let accept = |c: usize| -> bool {
            let m = &self.monsters[c];
            let d = &self.monster_definitions[m.definition];
            let owner_ok = match s.owner_mode {
                0 => m.owner == s.player,
                1 => m.owner != s.player,
                _ => true,
            };
            let team_ok = match s.team_mode {
                0 => m.team == s.team,
                1 => m.team != s.team,
                2 => !self.teams_are_allied(s.team, m.team),
                _ => true,
            };
            if !owner_ok || !team_ok {
                return false;
            }
            let same = Some(definition_collection(d)) == s.collection;
            match s.collection_mode {
                0 => {
                    let same = if same && (odef.ai.byte_199 & 1 != 0 || d.ai.byte_199 & 1 != 0) {
                        m.definition == orig.definition
                    } else {
                        same
                    };
                    if !same {
                        return false;
                    }
                }
                1 if same => return false,
                _ => {}
            }
            let ebx = (d.flags >> 19) & 1 == s.bit19;
            if m.flags_8 & 0x1000 != 0 && orig.flags_8 & 0x1000 == 0 {
                return false;
            }
            if self.teams_are_allied(orig.team, local_team)
                && orig.flags_8 & 0x4000 != 0
                && m.flags_8 & 0x4000 == 0
            {
                return false;
            }
            ebx
        };
        let mut list = vec![target];
        let mut i = 0;
        while i < list.len() {
            let p = self.monsters[list[i]].position;
            i += 1;
            let x = (p.0 >> 9) as i16 - 2;
            let x = if x < 0 { 0 } else { i32::from(x).min(w - 5) };
            let y = (p.1 >> 9) as i16 - 2;
            let y = if y < 0 { 0 } else { i32::from(y).min(h - 5) };
            for c in self.monsters_in_cell_box(x, y, 5, 5) {
                if self.monsters[c].object.no_shape || !accept(c) {
                    continue;
                }
                if list.contains(&c) || list.len() > 0x1d {
                    continue;
                }
                list.push(c);
            }
        }
        list
    }

    pub fn local_attack_location(&self, x: i32, y: i32, state_3: bool) -> (u16, i32) {
        let mut z = i32::from(self.mesh_height(x, y, mandrake_tag::MeshHeightMode::Surface) as i16);
        if self.cells.cell_flags(x >> 9, y >> 9) & 0x1800 != 0 {
            let (w, _) = self.mesh_cells;
            let i = usize::try_from((y >> 9) * w + (x >> 9)).unwrap_or(usize::MAX);
            if let Some(&(_, media)) = self.heights.get(i) {
                z = z.max(i32::from(media));
            }
        }
        (u16::from(state_3), z)
    }
}

fn definition_collection(d: &crate::engine_rng::MonsterDefinition) -> [u8; 4] {
    let r = &d.ai.raw;
    if r.len() < 8 {
        return [0; 4];
    }
    [r[4], r[5], r[6], r[7]]
}

fn attack_slot(a: i16, n: i16, version: i32) -> i16 {
    if version <= game_options::GAMEPLAY_1_6 {
        a
    } else if a < 0 {
        0
    } else {
        a.min(n - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attack_slot_clamps_above_gameplay_0x640() {
        assert_eq!(attack_slot(5, 2, 0x726), 1);
        assert_eq!(attack_slot(1, 2, 0x726), 1);
        assert_eq!(attack_slot(-3, 2, 0x726), 0);
        assert_eq!(attack_slot(0, 0, 0x726), -1);
        assert_eq!(attack_slot(5, 2, 0x640), 5);
        assert_eq!(attack_slot(-3, 2, 0x640), -3);
    }

    fn attack(flags: u16, min: i16, max: i16, seq: i16) -> AttackDefinition {
        AttackDefinition {
            flags,
            miss_fraction: 0,
            projectile: Some(FourCC::new(*b"test")),
            minimum_range: min,
            maximum_range: max,
            sequences: [(1, seq), (0, -1), (0, -1), (0, -1)],
            repetitions: 1,
            velocity: 0,
            velocity_delta: 0,
            error: 0,
            error_experience: 0,
            recovery: 75,
            recovery_experience: 2,
            flags_3c: 0,
            mana: 0,
        }
    }

    #[test]
    fn attack_layout_from_the_archer_tag() {
        let mut mons = vec![0u8; 0x224];
        mons[0x88..0x8a].copy_from_slice(&2i16.to_be_bytes());
        let words: [i16; 30] = [
            0x208, 0, 0x6172, 0x6172, 1536, 10240, 1, 8, 0, 0, 4, 6, 0, 0, 2, 7, 0, 0, 1, -1, 0, 0,
            1, 123, 11, 4, 75, 2, 1, 0,
        ];
        for (i, w) in words.iter().enumerate() {
            let o = 0x8c + 0x40 + 2 * i;
            mons[o..o + 2].copy_from_slice(&w.to_be_bytes());
        }
        let a = AttackDefinition::from_mons(&mons, 1);
        assert_eq!(a.flags, 0x208);
        assert_eq!(a.projectile, Some(FourCC::new(*b"arar")));
        assert_eq!((a.minimum_range, a.maximum_range), (1536, 10240));
        assert_eq!(a.sequences, [(1, 8), (4, 6), (2, 7), (1, -1)]);
        assert_eq!((a.repetitions, a.velocity, a.velocity_delta), (1, 123, 11));
        assert_eq!(
            (
                a.error,
                a.recovery,
                a.recovery_experience,
                a.error_experience
            ),
            (4, 75, 2, 1)
        );
        assert_eq!(a.key_point_sequences(), vec![8]);
    }

    #[test]
    fn untargetable_and_attack_clamp() {
        let def = AiDefinition {
            tag: FourCC::new(*b"test"),
            flags: 0x10800,
            idle_sequence: 0,
            walk_sequence: 1,
            turn_sequence: -1,
            block_sequence: -1,
            hit_sequence: 2,
            engage_sequence: 0,
            hit_sequence_2c: -1,
            alert_radius: 2048,
            engage_distance: 4096,
            vision: 2048,
            recovery_bounds: (30, 16),
            byte_198: 0,
            byte_199: 0,
            byte_1b4: 0,
            byte_1b5: 0,
            class: 5,
            attacks: vec![attack(0x200, 0, 614, 1), attack(0x208, 1536, 10240, 8)],
            headers: Vec::new(),
            raw: Vec::new(),
        };
        assert!(def.untargetable());
        assert_eq!(
            def.attack(7).unwrap().maximum_range,
            10240,
            "clamped to the last attack"
        );
        assert_eq!(def.attack(-1).unwrap().maximum_range, 614);
    }

    #[test]
    fn the_preferred_attack_is_the_shortest_usable_one() {
        let mut w = crate::collide::tests::world((10_000, 10_000), (20_000, 20_000));
        w.formation_globals.gameplay_version = 0x73c;
        let none = (false, false, false);
        let def = |attacks: Vec<AttackDefinition>| AiDefinition {
            attacks,
            ..AiDefinition::default()
        };
        let d = def(vec![
            attack(0, 0, 0x300, 0),
            attack(0x200, 0x200, 0x1000, 0),
            attack(0x200, 0x100, 0x2000, 0),
        ]);
        assert_eq!(w.preferred_attack(0, &d, 0x80, none).unwrap(), 0);
        assert_eq!(w.preferred_attack(0, &d, 0x100, none).unwrap(), 2);
        let d = def(vec![
            attack(1, 0, 0x300, 0),
            attack(0x200, 0x200, 0x1000, 0),
            attack(0x200, 0x100, 0x2000, 0),
        ]);
        assert_eq!(w.preferred_attack(0, &d, 0x100, none).unwrap(), 0);
        let d = def(vec![
            attack(0x200, 0x400, 0x300, 0),
            attack(0, 0, 0x1000, 0),
        ]);
        assert_eq!(w.preferred_attack(0, &d, 0x7d000, none).unwrap(), 0);
        let d = def(vec![
            attack(0x40, 0, 0x100, 0),
            attack(0, 0, 0, 0),
            attack(0, 0, 0x500, 0),
        ]);
        assert_eq!(w.preferred_attack(0, &d, 0, none).unwrap(), 2);
        let d = def(vec![attack(0x80, 0, 0x100, 0), attack(0x40, 0, 0x200, 0)]);
        assert_eq!(w.preferred_attack(0, &d, 0, none).unwrap(), 0);
        let d = def(vec![attack(0, 0, 0x500, 0), attack(0x800, 0, 0x100, 0)]);
        assert_eq!(w.preferred_attack(0, &d, 0, none).unwrap(), 0);
        assert_eq!(
            w.preferred_attack(0, &d, 0, (true, false, false)).unwrap(),
            1
        );
        let d = def(vec![attack(0, 0, 0x500, 0), attack(0x1000, 0, 0x100, 0)]);
        assert_eq!(
            w.preferred_attack(0, &d, 0, (true, false, false)).unwrap(),
            0
        );
        let mut a = attack(0, 0, 0x100, 0);
        a.flags_3c = 0x48;
        let d = def(vec![attack(0, 0, 0x500, 0), a]);
        assert_eq!(
            w.preferred_attack(0, &d, 0, (false, false, true)).unwrap(),
            0
        );
        assert_eq!(
            w.preferred_attack(0, &d, 0, (false, true, true)).unwrap(),
            1
        );
        a.flags_3c = 0x80;
        let d = def(vec![attack(0, 0, 0x500, 0), a]);
        assert_eq!(
            w.preferred_attack(0, &d, 0, (false, true, false)).unwrap(),
            0
        );
        assert_eq!(w.preferred_attack(0, &d, 0, none).unwrap(), 1);
        let d = def(vec![attack(0, 0, 0x500, 0), attack(0x20, 0, 0x100, 0)]);
        assert_eq!(w.preferred_attack(0, &d, 0, none).unwrap(), 0);
        assert!(w.preferred_attack(0, &def(vec![]), 0, none).is_err());
    }

    #[test]
    fn find_target_takes_other_modes_as_mode_0() {
        let mut w = crate::collide::tests::world((10_000, 10_000), (10_200, 10_000));
        w.monsters[1].team = 2;
        w.monster_definitions[0].ai.vision = 0x1000;
        w.mesh_cells = (160, 160);
        let mv = crate::collide::tests::mover((10_000, 10_000), (0, 0));
        let own = Own::of(&mv);
        let t0 = w.find_target(0, 0, &own).unwrap();
        assert_eq!(t0, 1, "the enemy in the next cell");
        for mode in [4, 7, -1] {
            assert_eq!(w.find_target(0, mode, &own).unwrap(), t0, "mode {mode}");
        }
    }

    #[test]
    fn a_special_ability_is_for_the_monsters_family() {
        let other = FourCC::new(*b"othr");
        let run = |flags: u32, collection: Option<[u8; 4]>| {
            let mut w = crate::collide::tests::world((10_000, 10_000), (20_000, 20_000));
            w.formation_globals.gameplay_version = 0x73c;
            let mut arti = vec![0u8; 0x94];
            arti[0..4].copy_from_slice(&flags.to_be_bytes());
            arti[4..8].copy_from_slice(&other.0);
            arti[0x54..0x56].copy_from_slice(&0x1234u16.to_be_bytes());
            w.catalog.artifacts.push((FourCC::new(*b"arti"), arti));
            w.combat_mut(0).special = 0;
            if let Some(c) = collection {
                let mut mons = vec![0u8; 0x224];
                mons[4..8].copy_from_slice(&c);
                w.monster_definitions
                    .push(crate::engine_rng::MonsterDefinition::from_bytes(other, &mons).unwrap());
            }
            w.special_attack(0)
                .map(|s| s.map(|(a, only)| (a.flags, only)))
        };
        assert_eq!(run(0x111, Some([0; 4])).unwrap(), Some((0x1234, false)));
        assert_eq!(run(0x111, Some(*b"coll")).unwrap(), None);
        assert_eq!(run(0x11, Some([0; 4])).unwrap(), None);
        assert_eq!(
            run(0x111, None).unwrap_err().site,
            crate::draw_site::SPECIAL_MONSTER_NOT_LOADED
        );
    }
}
