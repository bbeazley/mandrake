use crate::commands::MovementCommand;
use crate::draw_site;
use crate::groups::{distance, isqrt};
use crate::map_actions::{Parameter, Walk, combined, combined_points, find, index_of};
use crate::trig::arctangent;
use crate::{Order, Stop, TflSim};

const SENDER: i16 = -2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMoveTarget {
    Point {
        point: (i32, i32),
        facing: u16,
    },
    Monster {
        monster: usize,
        position: (i32, i32),
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraMoveEvent {
    pub tick: u32,
    pub identifier: i16,
    pub target: CameraMoveTarget,
    pub line: Option<i32>,
    pub angu: Option<i32>,
    pub time: Option<i32>,
}

const CAMERA_MOVEMENTS_KEPT: usize = 64;

pub const FOLLOW_INITIALISED: u32 = 1;
pub const FOLLOW_FAR: u32 = 2;
pub const FOLLOW_LOST: u32 = 4;
pub const FOLLOW_OBJECTS: u32 = 8;
pub const FOLLOW_STOP: u32 = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowState {
    pub flags: u32,
    pub action: i16,
    pub radius: i32,
    pub forward: i32,
    pub centre: (i32, i32),
    pub destination: (i32, i32),
}

impl FollowState {
    pub fn to_bytes(self) -> Vec<u8> {
        let mut b = vec![0u8; 0x24];
        b[4..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8..10].copy_from_slice(&self.action.to_le_bytes());
        b[0xc..0x10].copy_from_slice(&self.radius.to_le_bytes());
        b[0x10..0x14].copy_from_slice(&self.forward.to_le_bytes());
        b[0x14..0x18].copy_from_slice(&self.centre.0.to_le_bytes());
        b[0x18..0x1c].copy_from_slice(&self.centre.1.to_le_bytes());
        b[0x1c..0x20].copy_from_slice(&self.destination.0.to_le_bytes());
        b[0x20..0x24].copy_from_slice(&self.destination.1.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Self {
            flags: i32_at(4) as u32,
            action: i16::from_le_bytes([b[8], b[9]]),
            radius: i32_at(0xc),
            forward: i32_at(0x10),
            centre: (i32_at(0x14), i32_at(0x18)),
            destination: (i32_at(0x1c), i32_at(0x20)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProjectileGroupOrder {
    pub tick: u32,
    pub action: usize,
    pub groups: Vec<i16>,
    pub on: bool,
    pub opacity: Option<u16>,
    pub off: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniffer {
    pub case: i16,
    pub flags: u32,
    pub action: i16,
    pub delay: i32,
    pub range: i32,
    pub deadline: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurroundState {
    pub state: i16,
    pub initial: i16,
    pub loss: i32,
    pub time: i32,
}

impl SurroundState {
    pub const SIZE: usize = 0xc;

    pub fn to_bytes(self) -> [u8; 0xc] {
        let mut b = [0u8; 0xc];
        b[0..2].copy_from_slice(&self.state.to_le_bytes());
        b[2..4].copy_from_slice(&self.initial.to_le_bytes());
        b[4..8].copy_from_slice(&self.loss.to_le_bytes());
        b[8..0xc].copy_from_slice(&self.time.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Self {
            state: i16::from_le_bytes([b[0], b[1]]),
            initial: i16::from_le_bytes([b[2], b[3]]),
            loss: i32_at(4),
            time: i32_at(8),
        }
    }
}

fn signed_angular_difference(a: i32, b: i32) -> i32 {
    let mut d = b.wrapping_sub(a);
    if d >= 0x8000 {
        d -= 0x10000;
    }
    if d <= -0x8000 {
        d = d.wrapping_mul(2).wrapping_add(0x10000);
    }
    d
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeleeState {
    pub flags: u32,
    pub action: i16,
    pub state: i16,
    pub formation: i32,
    pub dither: i32,
}

impl MeleeState {
    pub fn to_bytes(self) -> Vec<u8> {
        let mut b = vec![0u8; 0x10];
        b[0..4].copy_from_slice(&self.flags.to_le_bytes());
        b[4..6].copy_from_slice(&self.action.to_le_bytes());
        b[6..8].copy_from_slice(&self.state.to_le_bytes());
        b[8..0xc].copy_from_slice(&self.formation.to_le_bytes());
        b[0xc..0x10].copy_from_slice(&self.dither.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Self {
            flags: i32_at(0) as u32,
            action: i16::from_le_bytes([b[4], b[5]]),
            state: i16::from_le_bytes([b[6], b[7]]),
            formation: i32_at(8),
            dither: i32_at(0xc),
        }
    }
}

impl TflSim {
    fn long_param(&self, i: usize, name: &[u8; 4], kind: i16, k: usize) -> Option<i32> {
        find(&self.map_actions.actions, i, 0, name, kind)
            .filter(|p| (k as i32) < i32::from(p.count))
            .map(|p| p.i32_at(k))
    }

    fn flag_param(&self, i: usize, name: &[u8; 4]) -> bool {
        find(&self.map_actions.actions, i, 2, name, 0).is_some()
    }

    pub(crate) fn structure(&self, i: usize, name: &[u8; 4], size: usize) -> Option<Vec<u8>> {
        find(&self.map_actions.actions, i, 1, name, 1)
            .filter(|p| p.count as usize == size && p.data.len() == size)
            .map(|p| p.data.clone())
    }

    pub(crate) fn activate_listed(&mut self, i: usize, name: &[u8; 4]) -> Result<(), Stop> {
        let Some(ids) = find(&self.map_actions.actions, i, 0, name, 3).map(|p| p.shorts()) else {
            return Ok(());
        };
        for &id in ids.iter().take(30) {
            if let Some(j) = index_of(&self.map_actions.actions, id) {
                self.activate(j)?;
            }
        }
        Ok(())
    }

    fn team_of(&self, slot: usize) -> i16 {
        self.monsters[slot].team as i8 as i16
    }

    pub(crate) fn look_action(&mut self, i: usize) -> Result<u32, Stop> {
        let ids = combined(&self.map_actions.actions, i, b"subj", 2, 300);
        let slots = self.monster_slots(&ids);
        let (centre, spread) = self.monsters_centre_spread(&slots);
        if slots.is_empty() {
            return Ok(2);
        }
        let team = self.team_of(slots[0]);
        let seen = self.spatial_groups()?.iter().any(|g| {
            g.team != team
                && (distance(centre, g.centre) as i32)
                    .wrapping_sub(i32::from(g.spacing))
                    .wrapping_sub(i32::from(g.radius))
                    .wrapping_sub(spread)
                    < 0
        });
        Ok(if seen { 2 } else { 0 })
    }

    pub(crate) fn object_control_action(&mut self, i: usize) -> Result<u32, Stop> {
        let Some(kind) = self.long_param(i, b"type", 7, 0) else {
            return Ok(0);
        };
        let tag = mandrake_common::FourCC((kind as u32).to_be_bytes());
        let mut found = Vec::new();
        for (k, o) in self.objects.iter().enumerate() {
            if found.len() >= 0x10 {
                break;
            }
            if o.identifier != 0 && o.kind == 4 && o.sleeping == Some(tag) {
                found.push(k);
            }
        }
        if found.is_empty() {
            return Ok(0);
        }
        let mut result = 0;
        if self.flag_param(i, b"real") {
            for &k in &found {
                self.objects[k].flags &= !0x200;
            }
            result = 2;
        }
        if self.flag_param(i, b"rera") {
            let n = found.len() as i32;
            let pick = (i32::from(self.rng.draw(draw_site::OBJECT_CONTROL_PICK)) * n) >> 16;
            for (j, &k) in found.iter().enumerate() {
                if j as i32 == pick {
                    self.objects[k].flags &= !0x200;
                } else {
                    self.objects[k].flags |= 0x200;
                }
            }
            return Ok(result | 2);
        }
        Ok(result)
    }

    pub(crate) fn observer_movement_action(&mut self, i: usize) -> Result<u32, Stop> {
        if self.game_mode == 0 {
            return Ok(2);
        }
        let Some(mode) = self.long_param(i, b"mode", 5, 0) else {
            return Ok(0);
        };
        let bits = if mode == -1 {
            0
        } else {
            1u32.wrapping_shl(mode as u32)
        };
        self.camera_control_flags = bits as u16;
        if bits & 2 == 0 {
            return Ok(0);
        }
        let a = &self.map_actions.actions;
        let ids = combined(a, i, b"subj", 2, 300);
        let target = if ids.is_empty() {
            let dest = find(a, i, 0, b"dest", 12).filter(|p| p.count > 0);
            let faci = find(a, i, 0, b"faci", 4).filter(|p| p.count > 0);
            dest.zip(faci).map(|(d, f)| CameraMoveTarget::Point {
                point: (d.i32_at(0), d.i32_at(1)),
                facing: f.i16_at(0) as u16,
            })
        } else {
            let slots = self.monster_slots(&ids);
            if let [m] = slots[..] {
                self.objects
                    .get(self.monsters[m].object)
                    .map(|o| CameraMoveTarget::Monster {
                        monster: m,
                        position: (o.x, o.y),
                    })
            } else {
                None
            }
        };
        let Some(target) = target else {
            return Ok(0);
        };
        let event = CameraMoveEvent {
            tick: self.tick,
            identifier: self.map_actions.actions[i].identifier,
            target,
            line: self.long_param(i, b"line", 6, 0),
            angu: self.long_param(i, b"angu", 5, 0),
            time: self.long_param(i, b"time", 8, 0),
        };
        if self.camera_movements.len() >= CAMERA_MOVEMENTS_KEPT {
            self.camera_movements.remove(0);
        }
        self.camera_movements.push(event);
        Ok(2)
    }

    pub(crate) fn follow_action(&mut self, i: usize) -> Result<u32, Stop> {
        let mut s = match self.structure(i, b"!glb", 0x24) {
            Some(b) => FollowState::from_bytes(&b),
            None => FollowState {
                flags: if self.flag_param(i, b"stop") {
                    FOLLOW_STOP
                } else {
                    0
                },
                action: i as i16,
                radius: self.long_param(i, b"radi", 6, 0).unwrap_or(0x3c00),
                forward: self.long_param(i, b"fora", 6, 0).unwrap_or(0x200),
                centre: (0, 0),
                destination: (0, 0),
            },
        };
        let a = &self.map_actions.actions;
        let subj = combined(a, s.action as usize, b"subj", 2, 0x10);
        let obje = combined(a, s.action as usize, b"obje", 2, 0xc);
        let subjects = self.monster_slots(&subj);
        if obje.is_empty() {
            s.flags &= !FOLLOW_OBJECTS;
        } else {
            s.flags |= FOLLOW_OBJECTS;
        }
        let objects: Vec<usize> = self
            .monster_slots(&obje)
            .into_iter()
            .filter(|&m| self.monsters[m].action != 0xd)
            .collect();
        s.centre = self.monsters_centre_spread(&subjects).0;
        if subjects.is_empty() {
            self.activate_listed(s.action as usize, b"erel")?;
            return Ok(4);
        }
        if objects.is_empty() && s.flags & FOLLOW_OBJECTS != 0 {
            self.activate_listed(s.action as usize, b"edel")?;
            return Ok(4);
        }
        let team = self.team_of(subjects[0]);
        self.follow(&mut s, team, &subjects, &objects)?;
        s.flags |= FOLLOW_INITIALISED;
        self.set_param(i, b"!glb", 1, 0x24, s.to_bytes());
        Ok(0)
    }

    fn follow(
        &mut self,
        s: &mut FollowState,
        team: i16,
        subjects: &[usize],
        objects: &[usize],
    ) -> Result<(), Stop> {
        let groups = self.spatial_supergroups()?;
        let (mut sx, mut sy, mut strength) = (0i32, 0i32, 0i32);
        let mut best: Option<(usize, i32)> = None;
        for (k, g) in groups.iter().enumerate() {
            let dx = s.centre.0.wrapping_sub(g.centre.0);
            let dy = s.centre.1.wrapping_sub(g.centre.1);
            let d = isqrt(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32) as i32;
            let edge = d.wrapping_sub(i32::from(g.radius));
            if edge >= s.radius {
                continue;
            }
            if g.team == team {
                let take = match best {
                    None => true,
                    Some((b, bd)) => {
                        edge < bd
                            || (g.strength > groups[b].strength && edge.wrapping_sub(0x800) < bd)
                    }
                };
                if take && (dx != 0 || dy != 0) {
                    best = Some((k, edge));
                }
            } else if d != 0 {
                let w = i32::from(g.strength);
                sx = sx.wrapping_add(w.wrapping_mul(dx) / d);
                sy = sy.wrapping_add(w.wrapping_mul(dy) / d);
                strength = strength.wrapping_add(w);
            }
        }
        if strength != 0 {
            sx = (sx << 16) / strength;
            sy = (sy << 16) / strength;
        }
        let target: Option<((i32, i32), i32)> = if s.flags & FOLLOW_OBJECTS != 0 {
            let mut t = None;
            let mut near = i32::MAX;
            for (k, &m) in objects.iter().enumerate() {
                let o = &self.objects[self.monsters[m].object];
                let p = (o.x, o.y);
                let d = distance(s.centre, p) as i32;
                if k == 0 || d < near {
                    near = d;
                    t = Some(p);
                }
                let dot = s
                    .centre
                    .1
                    .wrapping_sub(p.1)
                    .wrapping_mul(sy)
                    .wrapping_add(s.centre.0.wrapping_sub(p.0).wrapping_mul(sx));
                if dot < 0 {
                    t = Some(p);
                    break;
                }
            }
            t.map(|p| (p, 0))
        } else {
            best.map(|(b, _)| (groups[b].centre, i32::from(groups[b].radius)))
        };
        let Some((t, radius)) = target else {
            for &m in subjects {
                self.monsters[m].commander = SENDER;
            }
            self.general_command(2, subjects)?;
            let flags = s.flags;
            if flags & FOLLOW_LOST == 0 {
                self.activate_listed(s.action as usize, b"acab")?;
            } else if flags & FOLLOW_FAR == 0 {
                self.activate_listed(s.action as usize, b"acfa")?;
            }
            s.flags |= FOLLOW_FAR | FOLLOW_LOST;
            return Ok(());
        };
        let reach = (radius as i16).wrapping_add(s.forward as i16);
        let angle = if strength != 0 {
            crate::trig::arctangent(sx, sy)
        } else {
            crate::trig::arctangent(s.centre.0.wrapping_sub(t.0), s.centre.1.wrapping_sub(t.1))
        };
        let tr = crate::trig::trig();
        let ix = crate::trig::index(i32::from(angle));
        let r = i32::from(reach);
        let point = (
            (i32::from(tr.cosine[ix]).wrapping_mul(r) >> 14).wrapping_add(t.0),
            (i32::from(tr.sine[ix]).wrapping_mul(r) >> 14).wrapping_add(t.1),
        );
        if s.flags & FOLLOW_INITIALISED == 0 || distance(point, s.destination) as i32 > 0x400 {
            let command = MovementCommand {
                flags: 0x10,
                formation: 0,
                facing: angle.wrapping_add(0x8000),
            };
            self.record_order(|| Order {
                site: draw_site::FOLLOW_ORDER,
                action: s.action as usize,
                monsters: subjects.to_vec(),
                target: Some(point),
                offset: None,
                done: true,
            });
            self.execute_movement_command(SENDER, &command, subjects, &[point])?;
            s.destination = point;
        } else if s.flags & FOLLOW_STOP != 0 {
            for &m in subjects {
                self.monsters[m].flags |= 0x40_0000;
            }
        }
        let flags = s.flags;
        let limit = if flags & FOLLOW_FAR != 0 {
            0xc00
        } else {
            0x1400
        };
        if distance(s.centre, s.destination) as i32 > limit {
            if flags & FOLLOW_FAR == 0 {
                self.activate_listed(s.action as usize, b"acfa")?;
            }
            s.flags |= FOLLOW_FAR;
        } else {
            s.flags &= !FOLLOW_FAR;
        }
        s.flags &= !FOLLOW_LOST;
        Ok(())
    }

    pub(crate) fn melee_action(&mut self, i: usize) -> Result<u32, Stop> {
        let mut g = match self.structure(i, b"#glo", 0x10) {
            Some(b) => MeleeState::from_bytes(&b),
            None => MeleeState {
                flags: 0,
                action: i as i16,
                state: 0,
                formation: self.long_param(i, b"form", 5, 0).unwrap_or(0),
                dither: self.long_param(i, b"dith", 6, 0).unwrap_or(0xc00),
            },
        };
        let ids = combined(
            &self.map_actions.actions,
            g.action as usize,
            b"subj",
            2,
            300,
        );
        let subjects = self.monster_slots(&ids);
        if subjects.is_empty() {
            return Ok(4);
        }
        let team = self.team_of(subjects[0]);
        let (centre, spread) = self.monsters_centre_spread(&subjects);
        let spread = (spread as i16).max(0x400);
        let groups = self.spatial_supergroups()?;
        let mut best: Option<(usize, i32)> = None;
        for (k, sg) in groups.iter().enumerate() {
            let dx = centre.0.wrapping_sub(sg.centre.0).wrapping_abs();
            let dy = centre.1.wrapping_sub(sg.centre.1).wrapping_abs();
            let d = if dx > dy {
                (dy >> 1).wrapping_add(dx)
            } else {
                (dx >> 1).wrapping_add(dy)
            }
            .wrapping_sub(i32::from(sg.radius));
            if sg.team == team {
                continue;
            }
            let take = match best {
                None => true,
                Some((b, bd)) => {
                    d < bd
                        || (g.state != 1
                            && sg.strength > groups[b].strength
                            && d.wrapping_sub(g.dither) < bd)
                }
            };
            if take {
                best = Some((k, d));
            }
        }
        g.state = match best {
            None => -1,
            Some((b, d)) => {
                let target = groups[b].centre;
                if d.wrapping_sub(i32::from(spread)) < 0 {
                    let config = self.attack_configuration(None);
                    self.attack_with_monsters(&subjects, config);
                    1
                } else {
                    let command = MovementCommand {
                        flags: 8,
                        formation: g.formation as u16,
                        facing: 0,
                    };
                    self.record_order(|| Order {
                        site: draw_site::MELEE_ORDER,
                        action: g.action as usize,
                        monsters: subjects.clone(),
                        target: Some(target),
                        offset: None,
                        done: true,
                    });
                    self.execute_movement_command(SENDER, &command, &subjects, &[target])?;
                    0
                }
            }
        };
        g.flags |= 1;
        self.set_param(i, b"#glo", 1, 0x10, g.to_bytes());
        Ok(0)
    }
}

pub const HARASS_INITIALISED: u32 = 1;
pub const HARASS_WAYPOINT: u32 = 2;
const HARASS_SUBJECTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarassEntry {
    pub monster: i16,
    pub identifier: i16,
    pub field_4: i16,
    pub flags: u8,
    pub state: i16,
    pub timer: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarassState {
    pub flags: u32,
    pub action: i16,
    pub radius: i32,
    pub pick_radius: i32,
    pub attack_ratio: i16,
    pub dodge_ratio: i16,
    pub reevaluate: i32,
    pub entries: Vec<HarassEntry>,
    pub waypoint: (i32, i32),
}

impl HarassState {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![0u8; 0xa0];
        b[0..4].copy_from_slice(&self.flags.to_le_bytes());
        b[4..6].copy_from_slice(&self.action.to_le_bytes());
        b[8..0xc].copy_from_slice(&self.radius.to_le_bytes());
        b[0x10..0x14].copy_from_slice(&self.pick_radius.to_le_bytes());
        b[0x14..0x16].copy_from_slice(&self.attack_ratio.to_le_bytes());
        b[0x16..0x18].copy_from_slice(&self.dodge_ratio.to_le_bytes());
        b[0x18..0x1c].copy_from_slice(&self.reevaluate.to_le_bytes());
        b[0x1c..0x1e].copy_from_slice(&(self.entries.len() as i16).to_le_bytes());
        for (k, e) in self.entries.iter().enumerate() {
            let o = 0x1e + 0xc * k;
            b[o..o + 2].copy_from_slice(&e.monster.to_le_bytes());
            b[o + 2..o + 4].copy_from_slice(&e.identifier.to_le_bytes());
            b[o + 4..o + 6].copy_from_slice(&e.field_4.to_le_bytes());
            b[o + 6] = e.flags;
            b[o + 8..o + 0xa].copy_from_slice(&e.state.to_le_bytes());
            b[o + 0xa..o + 0xc].copy_from_slice(&e.timer.to_le_bytes());
        }
        b[0x98..0x9c].copy_from_slice(&self.waypoint.0.to_le_bytes());
        b[0x9c..0xa0].copy_from_slice(&self.waypoint.1.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let i16_at = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
        let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let n = (i16_at(0x1c).max(0) as usize).min(HARASS_SUBJECTS);
        Self {
            flags: i32_at(0) as u32,
            action: i16_at(4),
            radius: i32_at(8),
            pick_radius: i32_at(0x10),
            attack_ratio: i16_at(0x14),
            dodge_ratio: i16_at(0x16),
            reevaluate: i32_at(0x18),
            entries: (0..n)
                .map(|k| {
                    let o = 0x1e + 0xc * k;
                    HarassEntry {
                        monster: i16_at(o),
                        identifier: i16_at(o + 2),
                        field_4: i16_at(o + 4),
                        flags: b[o + 6],
                        state: i16_at(o + 8),
                        timer: i16_at(o + 0xa),
                    }
                })
                .collect(),
            waypoint: (i32_at(0x98), i32_at(0x9c)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct HarassEnemy {
    centre: (i32, i32),
    facing: u16,
    reach: i32,
    distance: i32,
}

impl TflSim {
    pub(crate) fn harass_action(&mut self, i: usize) -> Result<u32, Stop> {
        let mut s = match self.structure(i, b"#glo", 0xa0) {
            Some(b) => HarassState::from_bytes(&b),
            None => self.harass_globals(i)?,
        };
        let mut k = 0;
        while k < s.entries.len() {
            let e = s.entries[k];
            let held = self
                .monsters
                .get(e.monster as usize)
                .filter(|m| m.identifier != 0 && m.identifier == e.identifier)
                .map(|m| m.carried);
            match held {
                None => {
                    s.entries.remove(k);
                    self.activate_listed(s.action as usize, b"casu")?;
                }
                Some(carried) => {
                    let e = &mut s.entries[k];
                    if carried == -1 {
                        e.flags &= !1;
                    } else {
                        if e.state == 3 {
                            e.state = 0;
                            e.timer = 0;
                        }
                        e.flags |= 1;
                    }
                    k += 1;
                }
            }
        }
        if s.entries.is_empty() {
            return Ok(4);
        }
        let team = self.team_of(s.entries[0].monster as usize);
        self.harass_states(&mut s, team)?;
        s.flags |= HARASS_INITIALISED;
        self.set_param(i, b"#glo", 1, 0xa0, s.to_bytes());
        Ok(0)
    }

    fn harass_globals(&mut self, i: usize) -> Result<HarassState, Stop> {
        let a = &self.map_actions.actions;
        let delay = self.long_param(i, b"dela", 5, 0).unwrap_or(0);
        let waypoint = find(a, i, 0, b"wayp", 12)
            .filter(|p| p.count > 0)
            .map(|p| (p.i32_at(0), p.i32_at(1)));
        let ids = combined(a, i, b"subj", 2, HARASS_SUBJECTS);
        let slots = self.monster_slots(&ids);
        let mut entries = Vec::with_capacity(slots.len());
        for &m in &slots {
            let mut e = HarassEntry {
                monster: m as i16,
                identifier: self.monsters[m].identifier,
                field_4: 0,
                flags: 0,
                state: 0,
                timer: 0,
            };
            if delay != 0 {
                let r = i32::from(self.rng.draw(draw_site::HARASS_GLOBALS));
                e.timer = (r % delay) as i16;
            }
            let class = self
                .data
                .def(b"mons", self.monsters[m].definition)?
                .i16(0x96);
            if class == -1 {
                e.flags |= 2;
            } else if self.monsters[m].carried != -1 {
                e.flags |= 1;
            }
            entries.push(e);
        }
        Ok(HarassState {
            flags: if waypoint.is_some() {
                HARASS_WAYPOINT
            } else {
                0
            },
            action: i as i16,
            radius: self.long_param(i, b"radi", 6, 0).unwrap_or(0x2800),
            pick_radius: self.long_param(i, b"prra", 6, 0).unwrap_or(0x2800),
            attack_ratio: 0x40,
            dodge_ratio: 0x100,
            reevaluate: self.long_param(i, b"reev", 5, 0).unwrap_or(1),
            entries,
            waypoint: waypoint.unwrap_or((0, 0)),
        })
    }

    fn harass_states(&mut self, s: &mut HarassState, team: i16) -> Result<(), Stop> {
        for k in 0..s.entries.len() {
            let e = &mut s.entries[k];
            e.timer = e.timer.wrapping_sub(1);
            if e.timer >= 0 {
                continue;
            }
            e.timer = s.reevaluate as i16;
            let m = e.monster as usize;
            let mons = self.data.def_arc(b"mons", self.monsters[m].definition)?;
            let o = self.monsters[m].object;
            let at = (self.objects[o].x, self.objects[o].y);
            let state = s.entries[k].state;
            match state {
                0..=3 | 5 | 6 => {
                    if state == 3 {
                        s.entries[k].state = 0;
                    }
                    match self.closest_timid_enemy(team, s.radius, at)? {
                        None => self.enemy_out_of_range(s, k)?,
                        Some(g) => {
                            let ratio = self.harass_ratio(&g)?;
                            let tr = crate::trig::trig();
                            let ix = crate::trig::index(i32::from(g.facing));
                            let dot = i32::from(tr.sine[ix])
                                .wrapping_mul(at.1.wrapping_sub(g.centre.1))
                                .wrapping_add(
                                    i32::from(tr.cosine[ix])
                                        .wrapping_mul(at.0.wrapping_sub(g.centre.0)),
                                );
                            if dot <= 0 && (s.entries[k].flags & 3 != 0 || mons.i16(0x146) == 1) {
                                self.harass_attack(s, k, g.centre, g.reach);
                                s.entries[k].state = 1;
                            } else {
                                self.unit_danger_fraction(s, k, ratio, at, g.centre, g.reach)?;
                            }
                        }
                    }
                }
                4 => {
                    s.entries[k].state = 0;
                    if s.entries[k].flags & 3 != 0 {
                        continue;
                    }
                    match self.nearby_projectile(at, mons.i16(0x96), s.pick_radius)? {
                        Some((object, distance)) => {
                            let identifier = self.objects[object].identifier;
                            self.map_pick_up_command(SENDER, &[m], object as i16, identifier)?;
                            let speed = i32::from(mons.i16(0x8a)) << 5;
                            let t = distance.checked_div(speed).ok_or_else(|| {
                                Stop::new(
                                    draw_site::HARASS_SPEED_ZERO,
                                    "a girl subject of speed 0 (mons +0x8a)",
                                )
                            })?;
                            s.entries[k].state = 3;
                            s.entries[k].timer = t.wrapping_add(2) as i16;
                        }
                        None => match self.closest_timid_enemy(team, s.radius, at)? {
                            None => self.enemy_out_of_range(s, k)?,
                            Some(g) => {
                                let ratio = self.harass_ratio(&g)?;
                                self.unit_danger_fraction(s, k, ratio, at, g.centre, g.reach)?;
                            }
                        },
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn harass_ratio(&self, g: &HarassEnemy) -> Result<i32, Stop> {
        g.distance
            .wrapping_shl(8)
            .checked_div(g.reach)
            .ok_or_else(|| {
                Stop::new(
                    draw_site::HARASS_REACH_ZERO,
                    format!("tick {}: girl: a group of reach 0", self.tick),
                )
            })
    }

    fn closest_timid_enemy(
        &mut self,
        team: i16,
        radius: i32,
        at: (i32, i32),
    ) -> Result<Option<HarassEnemy>, Stop> {
        let mut best: Option<(HarassEnemy, i32)> = None;
        for g in self.spatial_groups()?.iter() {
            if g.team == team {
                continue;
            }
            let d = distance(at, g.centre) as i32;
            let edge = d
                .wrapping_sub(i32::from(g.spacing))
                .wrapping_sub(i32::from(g.radius));
            if d < radius && best.as_ref().is_none_or(|&(_, b)| edge < b) {
                best = Some((
                    HarassEnemy {
                        centre: g.centre,
                        facing: g.facing,
                        reach: i32::from(g.spacing) + i32::from(g.radius),
                        distance: d,
                    },
                    edge,
                ));
            }
        }
        Ok(best.map(|(g, _)| g))
    }

    fn harass_attack(&mut self, s: &HarassState, k: usize, centre: (i32, i32), reach: i32) {
        let mut config = self.attack_configuration(None);
        config.flags &= !1;
        config.point = centre;
        config.radius = reach;
        self.attack_with_monsters(&[s.entries[k].monster as usize], config);
    }

    fn harass_stand(&mut self, s: &mut HarassState, k: usize) -> Result<(), Stop> {
        s.entries[k].state = 0;
        let m = s.entries[k].monster as usize;
        self.monsters[m].commander = SENDER;
        self.general_command(4, &[m])
    }

    fn unit_danger_fraction(
        &mut self,
        s: &mut HarassState,
        k: usize,
        ratio: i32,
        at: (i32, i32),
        centre: (i32, i32),
        reach: i32,
    ) -> Result<(), Stop> {
        let flagged = s.entries[k].flags & 3 != 0;
        if ratio < i32::from(s.attack_ratio) {
            self.harass_attack(s, k, centre, reach);
            s.entries[k].state = 1;
        } else if ratio < i32::from(s.dodge_ratio) {
            let m = s.entries[k].monster;
            let a =
                crate::trig::arctangent(at.0.wrapping_sub(centre.0), at.1.wrapping_sub(centre.1));
            let a = if m & 1 != 0 {
                a.wrapping_add(0x800)
            } else {
                a.wrapping_add(0xf800)
            };
            let tr = crate::trig::trig();
            let ix = crate::trig::index(i32::from(a));
            let point = (
                centre
                    .0
                    .wrapping_add(i32::from(tr.cosine[ix]).wrapping_mul(reach) >> 14),
                centre
                    .1
                    .wrapping_add(i32::from(tr.sine[ix]).wrapping_mul(reach) >> 14),
            );
            let command = MovementCommand {
                flags: 0x10,
                formation: 0,
                facing: a.wrapping_add(0x8000),
            };
            self.harass_move(s, k, draw_site::HARASS_DANGER_MOVE, &command, point)?;
        } else if ratio < 0x200 {
            if flagged {
                self.harass_stand(s, k)?;
            } else {
                s.entries[k].state = 4;
                s.entries[k].timer = 0;
            }
        } else if s.entries[k].state == 5 && !flagged {
            s.entries[k].state = 4;
            s.entries[k].timer = 0;
        } else {
            self.harass_attack(s, k, centre, reach);
            s.entries[k].state = 5;
        }
        Ok(())
    }

    fn harass_move(
        &mut self,
        s: &HarassState,
        k: usize,
        site: u32,
        command: &MovementCommand,
        point: (i32, i32),
    ) -> Result<(), Stop> {
        let m = s.entries[k].monster as usize;
        self.record_order(|| Order {
            site,
            action: s.action as usize,
            monsters: vec![m],
            target: Some(point),
            offset: None,
            done: true,
        });
        self.execute_movement_command(SENDER, command, &[m], &[point])
    }

    fn enemy_out_of_range(&mut self, s: &mut HarassState, k: usize) -> Result<(), Stop> {
        let flagged = s.entries[k].flags & 3 != 0;
        if s.flags & HARASS_WAYPOINT != 0 {
            if s.entries[k].state == 6 && !flagged {
                s.entries[k].state = 4;
                s.entries[k].timer = 0;
            } else {
                let command = MovementCommand {
                    flags: 0,
                    formation: 0,
                    facing: 0,
                };
                let point = s.waypoint;
                self.harass_move(s, k, draw_site::HARASS_OUT_OF_RANGE_MOVE, &command, point)?;
            }
        } else if flagged {
            self.harass_stand(s, k)?;
        } else {
            s.entries[k].state = 4;
            s.entries[k].timer = 0;
        }
        Ok(())
    }

    fn nearby_projectile(
        &mut self,
        at: (i32, i32),
        class: i16,
        radius: i32,
    ) -> Result<Option<(usize, i32)>, Stop> {
        let mut best: Option<(usize, i32, i16)> = None;
        for k in 0..self.objects.len() {
            let o = &self.objects[k];
            if o.identifier == 0 || o.flags & 0x200 != 0 || o.kind != 4 {
                continue;
            }
            let dx = o.x.wrapping_sub(at.0).wrapping_abs();
            let dy = o.y.wrapping_sub(at.1).wrapping_abs();
            if dx >= radius
                || dy >= radius
                || dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx))
                    >= radius.wrapping_mul(radius)
            {
                continue;
            }
            let p = (o.x, o.y);
            let Some(tag) = o.sleeping else {
                return Err(Stop::new(
                    draw_site::NEARBY_PROJECTILE_NO_DEFINITION,
                    format!("object {k} asleep with no definition"),
                ));
            };
            let def = self.data.def(b"proj", tag)?;
            if def.i16(0x26) != class {
                continue;
            }
            let kind = def.i16(0x32);
            let d = distance(at, p) as i32;
            let take = match best {
                None => true,
                Some((_, bd, bk)) => {
                    if (kind == 6 || kind == 8) && bk != 8 {
                        true
                    } else {
                        d < bd
                    }
                }
            };
            if take {
                best = Some((k, d, kind));
            }
        }
        Ok(best.map(|(k, d, _)| (k, d)))
    }
}

impl TflSim {
    pub(crate) fn ambient_sound_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let subjects = combined(a, i, b"subj", 0x10, 20);
        let long = |name: &[u8; 4]| {
            find(a, i, 0, name, 8)
                .filter(|p| p.count > 0)
                .map(|p| p.i32_at(0))
        };
        let (Some(volu), Some(dura)) = (long(b"volu"), long(b"dura")) else {
            return Ok(0);
        };
        if subjects.is_empty() {
            return Ok(0);
        }
        let volume = (volu >> 8) as i16;
        let ticks = (dura.wrapping_mul(0x1e) >> 16) as i16;
        for id in subjects {
            if self.sound_sources.iter().any(|s| s.identifier == id) {
                self.flat_sound(
                    draw_site::AMBIENT_SOUND,
                    crate::sounds::SoundKind::AmbientVolume {
                        identifier: id,
                        volume,
                        ticks,
                    },
                    None,
                );
            }
        }
        Ok(0)
    }

    pub(crate) fn pick_up_object_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let subjects = self.monster_slots(&combined(a, i, b"subj", 2, 300));
        if subjects.is_empty() {
            return Ok(0);
        }
        let objects: Vec<usize> = combined(a, i, b"obje", 0xe, 0x14)
            .iter()
            .filter_map(|&id| {
                self.objects
                    .iter()
                    .position(|o| o.identifier != 0 && o.flags & 0x200 == 0 && o.identifier == id)
            })
            .collect();
        if objects.is_empty() {
            return Ok(0);
        }
        let sender = self.map_actions.actions[i].identifier;
        for (k, &m) in subjects.iter().enumerate() {
            let o = objects[k % objects.len()];
            let identifier = self.objects[o].identifier;
            self.map_pick_up_command(sender, &[m], o as i16, identifier)?;
        }
        Ok(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockState(pub Vec<u8>);

impl LockState {
    pub const SIZE: usize = 0x4a;

    pub fn short(&self, k: usize) -> i16 {
        i16::from_le_bytes([self.0[2 * k], self.0[2 * k + 1]])
    }

    fn set_short(&mut self, k: usize, v: i16) {
        self.0[2 * k..2 * k + 2].copy_from_slice(&v.to_le_bytes());
    }

    pub fn keys(&self) -> usize {
        self.short(0).max(0) as usize
    }

    pub fn key_state(&self, k: usize) -> i16 {
        self.short(0x1b + k)
    }

    pub fn last_count(&self) -> i16 {
        self.short(0x23)
    }
}

impl TflSim {
    fn lock_initial(&self, i: usize) -> Option<LockState> {
        let a = &self.map_actions.actions;
        let mut s = LockState(vec![0; LockState::SIZE]);
        let list = |s: &mut LockState, at: usize, name: &[u8; 4]| {
            let ids = combined(a, i, name, 3, 8);
            s.set_short(at, ids.len() as i16);
            for (k, &id) in ids.iter().enumerate() {
                s.set_short(at + 1 + k, id);
            }
            ids.len() as i16
        };
        let keys = list(&mut s, 0, b"keys");
        if keys == 0 {
            return None;
        }
        let acti = list(&mut s, 9, b"acti");
        let deac = list(&mut s, 0x12, b"deac");
        if (acti != 0 || deac != 0) && (acti != keys || deac != keys) {
            return None;
        }
        s.set_short(0x23, -1);
        if self.flag_param(i, b"reqs") {
            s.0[0x48] |= 1;
        }
        Some(s)
    }

    pub(crate) fn lock_action(&mut self, i: usize) -> Result<u32, Stop> {
        let mut s = match self.structure(i, b"#sta", LockState::SIZE) {
            Some(b) => LockState(b),
            None => match self.lock_initial(i) {
                Some(s) => s,
                None => return Ok(0),
            },
        };
        let keys = s.keys();
        let before: Vec<i16> = (0..keys).map(|k| s.key_state(k)).collect();
        let mut count: i16 = 0;
        for k in 0..keys {
            s.set_short(0x1b + k, 0);
            if let Some(j) = index_of(&self.map_actions.actions, s.short(1 + k))
                && self.trigger(j)? & 2 != 0
            {
                s.set_short(0x1b + k, 1);
                count += 1;
            }
        }
        for (k, &was) in before.iter().enumerate() {
            let now = s.key_state(k);
            if now == was {
                continue;
            }
            let list = if now != 0 { 9 } else { 0x12 };
            if s.short(list) == 0 {
                continue;
            }
            let id = s.short(list + 1 + k);
            if id == -1 {
                continue;
            }
            if let Some(j) = index_of(&self.map_actions.actions, id) {
                self.trigger(j)?;
            }
        }
        if s.last_count() != count {
            let digits = count.to_string().into_bytes();
            let n = digits.len().min(4);
            let mut name = *b"____";
            name[4 - n..].copy_from_slice(&digits[..n]);
            let a = &self.map_actions.actions;
            let p = find(a, i, 0, &name, 3);
            let element = |k: usize| {
                p.filter(|p| k < p.count.max(0) as usize)
                    .map(|p| p.i16_at(k))
            };
            let (inhibition, transition) = (element(1), element(0));
            let mut go = true;
            if let Some(j) = inhibition.and_then(|id| index_of(a, id))
                && self.trigger(j)? & 2 != 0
            {
                go = false;
            }
            if go && let Some(j) = transition.and_then(|id| index_of(&self.map_actions.actions, id))
            {
                self.trigger(j)?;
                s.set_short(0x23, count);
            }
        }
        self.set_param(i, b"#sta", 1, LockState::SIZE as i16, s.0);
        Ok(0)
    }

    pub(crate) fn mesh_cell_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let points = combined_points(a, i, b"pnts", 0x10);
        let (mut clear, mut set) = (0u16, 0u16);
        if let Some(p) = find(a, i, 0, b"kill", 0).filter(|p| p.count > 0) {
            clear = 0x4000;
            if p.data.first().copied().unwrap_or(0) != 0 {
                set = 0x4000;
            }
        }
        if let Some(t) = self.long_param(i, b"teco", 5, 0) {
            clear |= 0xff;
            set |= ((t as u32) << 4 | t as u32) as u16;
        }
        for (x, y) in points {
            let (cx, cy) = (x >> 10, y >> 10);
            if cx < 0 || cx >= self.width() || cy < 0 || cy >= self.height_cells() {
                continue;
            }
            let k = self.cell_at(cx, cy);
            let c = &mut self.cells[k];
            let w = (u16::from(c.flags) << 8 | u16::from(c.byte_5)) & !clear | set;
            c.flags = (w >> 8) as u8;
            c.byte_5 = w as u8;
        }
        Ok(0)
    }

    pub(crate) fn local_projectile_group_action(&mut self, i: usize) -> Result<u32, Stop> {
        let groups = combined(&self.map_actions.actions, i, b"grou", 0x12, 64);
        let order = LocalProjectileGroupOrder {
            tick: self.tick,
            action: i,
            on: self.flag_param(i, b"__on"),
            opacity: self
                .long_param(i, b"opac", 8, 0)
                .map(|v| v.clamp(0, 0xffff) as u16),
            off: self.flag_param(i, b"_off"),
            groups,
        };
        self.local_projectile_group_orders.push(order);
        Ok(2)
    }

    pub(crate) fn delay_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        if find(a, i, 0, b"subj", 2).is_none_or(|p| p.count <= 0) {
            return Ok(0);
        }
        let slots = self.monster_slots(&combined(a, i, b"subj", 2, 300));
        let slots: Vec<usize> = slots
            .into_iter()
            .filter(|&s| self.monsters[s].flags & 0x4000_0000 == 0)
            .collect();
        let n = slots.len();
        if n == 0 {
            return Ok(0);
        }
        let Some(time) = self.long_param(i, b"time", 8, 0) else {
            return Ok(0);
        };
        let delay = time.wrapping_mul(30) / 0x10000;
        let now = self.tick as i32;
        let ids: Vec<i16> = slots.iter().map(|&s| self.monsters[s].identifier).collect();
        let found = Walk::new(i, 3, Some(b"#del"), 5).next(&self.map_actions.actions);
        let Some(f) = found else {
            let spread = self.flag_param(i, b"dist");
            let mut data = Vec::with_capacity(4 * n);
            let mut step = delay;
            for _ in 0..n {
                let t = if spread {
                    now.wrapping_add(step / n as i32)
                } else if delay > 0 {
                    now.wrapping_add(self.rng.draw_scaled(draw_site::DELAY_ACTION_DELAY, delay))
                } else {
                    now
                };
                data.extend_from_slice(&t.to_be_bytes());
                step = step.wrapping_add(delay);
            }
            self.map_actions.actions[i].parameters.push(Parameter {
                kind: 5,
                count: n as i16,
                name: mandrake_common::FourCC(*b"#del"),
                data,
            });
            return Ok(0);
        };
        let Some(result) = self.long_param_short(i, b"resu", 3) else {
            return Ok(0);
        };
        let Some(target) = index_of(&self.map_actions.actions, result) else {
            return Ok(0);
        };
        let p = &self.map_actions.actions[f.action].parameters[f.param];
        if (p.count as usize) < n {
            return Err(Stop::new(
                draw_site::DELAY_ACTION_SUBJECT_COUNT,
                format!(
                    "tick {}: map action {i}: dela's #del has {} entries for {n} subjects (read \
                     past its end)",
                    self.tick, p.count
                ),
            ));
        }
        let mut times: Vec<i32> = (0..n).map(|k| p.i32_at(k)).collect();
        let mut done = true;
        let mut listed = Vec::new();
        for (k, t) in times.iter_mut().enumerate() {
            if *t == -1 {
                continue;
            }
            done = false;
            if *t < now {
                listed.push(ids[k]);
                *t = -1;
            }
        }
        let p = &mut self.map_actions.actions[f.action].parameters[f.param];
        p.count = n as i16;
        p.data = times.iter().flat_map(|t| t.to_be_bytes()).collect();
        if !listed.is_empty() {
            let name = self
                .long_param(i, b"type", 7, 0)
                .map_or(*b"subj", |t| (t as u32).to_be_bytes());
            while let Some(g) = Walk::new(target, 3, Some(&name), 2).next(&self.map_actions.actions)
            {
                self.map_actions.actions[g.action]
                    .parameters
                    .remove(g.param);
            }
            self.map_actions.actions[target].parameters.push(Parameter {
                kind: 2,
                count: listed.len() as i16,
                name: mandrake_common::FourCC(name),
                data: listed.iter().flat_map(|v| v.to_be_bytes()).collect(),
            });
            self.activate(target)?;
        }
        Ok(if done { 2 } else { 0 })
    }

    fn long_param_short(&self, i: usize, name: &[u8; 4], kind: i16) -> Option<i16> {
        find(&self.map_actions.actions, i, 0, name, kind)
            .filter(|p| p.count > 0)
            .map(|p| p.i16_at(0))
    }

    pub(crate) fn sniffer_action(&mut self, i: usize) -> Result<u32, Stop> {
        let Some(case) = self.long_param(i, b"case", 5, 0) else {
            return Ok(0);
        };
        let mut flags = 0;
        if !self.flag_param(i, b"nclr") {
            flags |= 1;
        }
        if self.flag_param(i, b"keep") {
            flags |= 2;
        }
        let (mut delay, mut range) = (0x96, 0);
        if let Some(t) = find(&self.map_actions.actions, i, 0, b"time", 5)
            && t.count == 2
        {
            (delay, range) = (t.i32_at(0), t.i32_at(1));
        }
        let timeout = self.long_param(i, b"inde", 5, 0).unwrap_or(0x96);
        let case = case as i16;
        if case == 1 {
            if let Some(state) = self.overhead_map_state {
                return Ok(if state == 2 { 2 } else { 0 });
            }
            return Err(Stop::new(
                draw_site::SNIFFER_OVERHEAD_MAP,
                format!(
                    "tick {}: map action {i} ({}): 'snif' case 1 returns the overhead map state",
                    self.tick, self.map_actions.actions[i].identifier
                ),
            ));
        }
        self.sniffer = Some(Sniffer {
            case,
            flags: flags | 8,
            action: self.map_actions.actions[i].identifier,
            delay,
            range,
            deadline: (self.tick as i32).wrapping_add(timeout),
        });
        Ok(0)
    }

    pub(crate) fn surround_action(&mut self, i: usize) -> Result<u32, Stop> {
        let mut st = match self.structure(i, b"#cst", SurroundState::SIZE) {
            Some(b) => SurroundState::from_bytes(&b),
            None => SurroundState {
                state: 0,
                initial: -1,
                loss: self.long_param(i, b"loss", 5, 0).unwrap_or(-1),
                time: self.long_param(i, b"time", 5, 0).unwrap_or(-1),
            },
        };
        let ids = combined(&self.map_actions.actions, i, b"subj", 2, 0x1e);
        let slots = self.monster_slots(&ids);
        let count = slots.len() as i16;
        let centre = self.monsters_centre(&slots);
        if st.initial == -1 {
            st.initial = count;
        }
        let mut end = st.loss != -1 && (i32::from(st.initial) - i32::from(count)) >= st.loss;
        let old = st.time;
        st.time = old.wrapping_sub(1);
        if old == 0 {
            end = true;
        }
        if end && st.state != 1 {
            self.activate_listed(i, b"acat")?;
            st.state = 1;
        }
        let result = if count <= 0 {
            4
        } else {
            match st.state {
                0 => self.surround_walk(i, &slots)?,
                1 => {
                    let mut config = self.attack_configuration(Some(i));
                    config.point = centre;
                    self.attack_with_monsters(&slots, config);
                }
                _ => {}
            }
            0
        };
        self.set_param(
            i,
            b"#cst",
            1,
            SurroundState::SIZE as i16,
            st.to_bytes().to_vec(),
        );
        Ok(result)
    }

    fn surround_walk(&mut self, i: usize, slots: &[usize]) -> Result<(), Stop> {
        let team = self.team_of(slots[0]);
        let positions: Vec<(i32, i32)> = slots
            .iter()
            .map(|&s| {
                let o = &self.objects[self.monsters[s].object];
                (o.x, o.y)
            })
            .collect();
        let groups = self.spatial_groups()?;
        for (&s, &(x, y)) in slots.iter().zip(&positions) {
            let mut nearest: Option<(i32, (i32, i32), i32)> = None;
            for g in groups.iter().filter(|g| g.team != team) {
                let d = (distance((x, y), g.centre) as i32)
                    .wrapping_sub(i32::from(g.spacing))
                    .wrapping_sub(i32::from(g.radius));
                if nearest.is_none_or(|(best, _, _)| d < best) {
                    nearest = Some((d, g.centre, i32::from(g.spacing) + i32::from(g.radius)));
                }
            }
            let Some((_, (cx, cy), reach)) = nearest else {
                continue;
            };
            let (mut sum_x, mut sum_y, mut others) = (0i32, 0i32, 0);
            for &(ox, oy) in &positions {
                let (dx, dy) = (x.wrapping_sub(ox), y.wrapping_sub(oy));
                let len =
                    isqrt(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32) as i32;
                if len != 0 {
                    sum_x = sum_x.wrapping_add(dx.wrapping_shl(14) / len);
                    sum_y = sum_y.wrapping_add(dy.wrapping_shl(14) / len);
                    others += 1;
                }
            }
            if others == 0 {
                continue;
            }
            let away = i32::from(arctangent(sum_x, sum_y));
            let out = i32::from(arctangent(x.wrapping_sub(cx), y.wrapping_sub(cy)));
            let turn = signed_angular_difference(out, away).clamp(-0x2aaa, 0x2aaa);
            let k = crate::trig::index((turn + out) & 0xffff);
            let t = crate::trig::trig();
            let point = (
                (i32::from(t.cosine[k]).wrapping_mul(reach) >> 14).wrapping_add(cx),
                (i32::from(t.sine[k]).wrapping_mul(reach) >> 14).wrapping_add(cy),
            );
            let command = MovementCommand {
                flags: 0,
                formation: 0,
                facing: 0,
            };
            self.record_order(|| Order {
                site: draw_site::SURROUND_ORDER,
                action: i,
                monsters: vec![s],
                target: Some(point),
                offset: None,
                done: true,
            });
            self.execute_movement_command(i as i16, &command, &[s], &[point])?;
        }
        Ok(())
    }

    pub fn take_local_projectile_group_orders(&mut self) -> Vec<LocalProjectileGroupOrder> {
        std::mem::take(&mut self.local_projectile_group_orders)
    }

    pub fn camera_movements(&self) -> &[CameraMoveEvent] {
        &self.camera_movements
    }

    pub fn take_camera_movements(&mut self) -> Vec<CameraMoveEvent> {
        std::mem::take(&mut self.camera_movements)
    }
}
