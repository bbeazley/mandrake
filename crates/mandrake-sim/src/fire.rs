use mandrake_common::FourCC;

use crate::draw_site;
use crate::engine_rng::{EngineRng, EngineWorld, Unmodelled};
use crate::map_actions::MovementOrder;
use crate::pathfinding::FireCells;

const FIRE_CELLS: usize = 0x80;
const MIRROR_GROUPS: usize = 0x100;
const SPREAD_LIST: usize = 0x80;
const STARTING_CHANCE: [u16; 16] = [
    0, 0, 0, 0, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0, 0xffff, 0xffff, 0xffff, 0xffff,
    0xffff,
];
const SPREADING_CHANCE: [u16; 16] = [
    0, 0, 0, 0, 0x8000, 0, 0x8000, 0, 0, 0x8000, 0, 0x8000, 0, 0, 0x8000, 0x8000,
];
const FIRE_DAMAGE: mandrake_tag::projectile::DamageDefinition =
    mandrake_tag::projectile::DamageDefinition {
        damage_type: 9,
        flags: 0x5000,
        damage_lower_bound: 0x40,
        damage_delta: 0x20,
        radius_lower_bound: 0x100,
        radius_delta: 0x100,
        word_0c: 0,
        word_0e: 0,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FireCell {
    pub x: i16,
    pub y: i16,
    pub chance: u16,
    pub player: crate::statistics::UnitRef,
    pub owner: i16,
    pub owner_identifier: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirrorGroup {
    pub flame: bool,
    pub slot: u8,
    pub point: [i32; 3],
    pub lifetime: i32,
    pub fade_in: i16,
    pub fade_out: i16,
    pub strength: i16,
    pub state: i16,
    pub ticks: i16,
}

#[derive(Debug, Clone, Default)]
pub struct FireState {
    pub cells: Vec<Option<FireCell>>,
    pub groups: Vec<Option<MirrorGroup>>,
}

fn new_slot<T>(slots: &mut Vec<Option<T>>, capacity: usize, value: T) -> Option<usize> {
    if let Some(i) = slots.iter().position(Option::is_none) {
        slots[i] = Some(value);
        return Some(i);
    }
    if slots.len() >= capacity {
        return None;
    }
    slots.push(Some(value));
    Some(slots.len() - 1)
}

fn delete_slot<T>(slots: &mut Vec<Option<T>>, i: usize) {
    slots[i] = None;
    while matches!(slots.last(), Some(None)) {
        slots.pop();
    }
}

impl FireState {
    pub fn cell_is_on_fire(&self, x: i32, y: i32) -> bool {
        self.cells
            .iter()
            .flatten()
            .any(|c| i32::from(c.x) == x && i32::from(c.y) == y)
    }

    pub fn monster_died(&mut self, m: i16) {
        for c in self.cells.iter_mut().flatten() {
            if c.owner == m {
                c.owner = -1;
                c.owner_identifier = -1;
            }
        }
    }
}

pub struct WorldFire<'a> {
    pub world: &'a EngineWorld,
    pub monster: usize,
}

impl FireCells for WorldFire<'_> {
    fn any(&self) -> bool {
        !self.world.fire.cells.is_empty()
    }
    fn on_fire(&self, x: i32, y: i32) -> bool {
        self.world.fire.cell_is_on_fire(x, y)
    }
    fn fire_modifier(&self) -> Result<u16, Unmodelled> {
        Ok(self.world.effect_modifiers(self.monster)?[4] as u16)
    }
}

impl EngineWorld {
    pub fn update_fire(&mut self, rng: &mut EngineRng) -> Result<bool, Unmodelled> {
        let Some(t) = self.fire_timer.as_mut() else {
            return Ok(false);
        };
        *t -= 1;
        if *t > 0 {
            return Ok(false);
        }
        *t = 0x78 + ((i32::from(rng.draw(draw_site::FIRE_TIMER)) * 0x96) >> 16);
        Ok(true)
    }

    fn clamp_cell(v: i32, size: i32) -> i32 {
        v.clamp(0, size - 1)
    }

    pub(crate) fn try_to_start_fire(
        &mut self,
        rng: &mut EngineRng,
        (player, owner, owner_identifier): (crate::statistics::UnitRef, i16, i16),
        x: i32,
        y: i32,
    ) -> Result<(), Unmodelled> {
        let flags = self.cells.cell_flags(x, y);
        if flags & 0x200 != 0 {
            return Ok(());
        }
        let (w, h) = (self.cells.width, self.cells.height);
        for cy in Self::clamp_cell(y - 1, h)..=Self::clamp_cell(y + 1, h) {
            for cx in Self::clamp_cell(x - 1, w)..=Self::clamp_cell(x + 1, w) {
                if self.cells.models[self.cells.cell_index(cx, cy)] != -1 {
                    return Ok(());
                }
            }
        }
        let a = STARTING_CHANCE[usize::from((flags >> 4) & 0xf)];
        let b = STARTING_CHANCE[usize::from(flags & 0xf)];
        let r = rng.draw(draw_site::FIRE_START_CHANCE);
        if r <= ((u32::from(a) + u32::from(b)) >> 1) as u16 {
            self.start_fire(
                rng,
                FireCell {
                    x: x as i16,
                    y: y as i16,
                    chance: 0xffff,
                    player,
                    owner,
                    owner_identifier,
                },
            )?;
        }
        Ok(())
    }

    fn fire_neighbours(&self, x: i32, y: i32) -> Vec<(usize, i32, i32)> {
        let (w, h) = (self.cells.width, self.cells.height);
        let x0 = Self::clamp_cell(x - 2, w);
        let x1 = Self::clamp_cell(x + 2, w);
        let y0 = Self::clamp_cell(y - 2, h);
        let y1 = Self::clamp_cell(y + 2, h);
        self.monsters_in_cell_box(x0, y0, x1 - x0 + 1, y1 - y0 + 1)
            .into_iter()
            .map(|m| {
                let p = self.monsters[m].position;
                (m, (p.0 >> 9) - x, (p.1 >> 9) - y)
            })
            .collect()
    }

    fn replan(&mut self, m: usize) {
        self.monsters[m].flags |= 0x20;
    }

    fn start_fire(&mut self, rng: &mut EngineRng, entry: FireCell) -> Result<(), Unmodelled> {
        let (x, y) = (i32::from(entry.x), i32::from(entry.y));
        let i = self.cells.cell_index(x, y);
        self.cells.flags[i] |= 0x200;
        let Some(_) = new_slot(&mut self.fire.cells, FIRE_CELLS, entry) else {
            return Ok(());
        };
        let (w, h) = (self.cells.width, self.cells.height);
        for (m, dx, dy) in self.fire_neighbours(x, y) {
            if self.effect_modifiers(m)?[4] <= 0 {
                continue;
            }
            self.replan(m);
            let flags = self.monsters[m].flags;
            let (no_path, facing) = self
                .movers
                .get(m)
                .and_then(|mv| mv.as_ref())
                .map_or((false, 0), |mv| (mv.path.is_none(), mv.word_26));
            if no_path && (flags as i32) >= 0 && dx.abs() <= 1 && dy.abs() <= 1 {
                let p = self.monsters[m].position;
                let clamp = |v: i32, cells: i32| {
                    if v < 0 { 0 } else { v.min(cells * 0x200 - 1) }
                };
                self.pending_orders.push(MovementOrder {
                    monster: m,
                    waypoints: vec![(clamp(p.0 + dx * 0x200, w), clamp(p.1 + dy * 0x200, h))],
                    flags: 0,
                    facing,
                    action: -1,
                });
            }
        }
        let lifetime = ((u32::from(rng.draw(draw_site::FIRE_LIFETIME)) * 0x78) >> 16) as i32 + 0x5a;
        let point = (x * 0x200 + 0x100, y * 0x200 + 0x100);
        let z = i32::from(
            self.mesh_height(point.0, point.1, mandrake_tag::MeshHeightMode::Physical) as i16,
        );
        let flame = ((u32::from(rng.draw(draw_site::FIRE_FLAME)) * 5) >> 16) as u8;
        let group = |flame_group: bool, slot: u8, z: i32| MirrorGroup {
            flame: flame_group,
            slot,
            point: [point.0, point.1, z],
            lifetime,
            fade_in: 0x96,
            fade_out: 0x96,
            strength: 0,
            state: 0,
            ticks: 0,
        };
        new_slot(&mut self.fire.groups, MIRROR_GROUPS, group(true, flame, z));
        let smoke = ((u32::from(rng.draw(draw_site::FIRE_SMOKE)) * 3) >> 16) as u8;
        new_slot(
            &mut self.fire.groups,
            MIRROR_GROUPS,
            group(false, smoke, z + 0x280),
        );
        Ok(())
    }

    fn extinguish_fire_in_cell(&mut self, x: i32, y: i32) -> Result<(), Unmodelled> {
        let Some(slot) = self
            .fire
            .cells
            .iter()
            .position(|c| c.is_some_and(|c| i32::from(c.x) == x && i32::from(c.y) == y))
        else {
            return Ok(());
        };
        for (m, _, _) in self.fire_neighbours(x, y) {
            if self.effect_modifiers(m)?[4] > 0 {
                self.replan(m);
            }
        }
        delete_slot(&mut self.fire.cells, slot);
        Ok(())
    }

    pub fn update_mirror_groups(&mut self) -> Result<(), Unmodelled> {
        let fraction = |t: i32, of: i16| ((t * 0x1_0000) / i32::from(of)) as u32;
        let squash = |v: u32| (v.wrapping_mul(0x1_0000).wrapping_sub(v & 0xffff) >> 16) as i16;
        for i in 0..self.fire.groups.len() {
            let Some(mut g) = self.fire.groups.get(i).copied().flatten() else {
                continue;
            };
            if g.state == 1 {
                if g.lifetime != -1 {
                    g.lifetime -= 1;
                    if g.lifetime < 0 {
                        g.state = 2;
                        g.ticks = 0;
                    }
                }
                g.strength = -2;
            } else {
                g.ticks = g.ticks.wrapping_add(1);
                let t = i32::from(g.ticks as u16);
                if g.state == 0 {
                    if g.ticks < g.fade_in {
                        g.strength = if g.fade_in != 0 {
                            squash(fraction(t, g.fade_in))
                        } else {
                            0
                        };
                    } else {
                        g.state = 1;
                        g.ticks = 0;
                        g.strength = -2;
                    }
                } else {
                    if g.fade_out <= g.ticks {
                        if g.flame {
                            self.extinguish_fire_in_cell(g.point[0] >> 9, g.point[1] >> 9)?;
                        }
                        self.fire.groups[i] = None;
                        continue;
                    }
                    g.strength = if g.fade_out != 0 {
                        squash(!fraction(t, g.fade_out))
                    } else {
                        -2
                    };
                }
            }
            self.fire.groups[i] = Some(g);
        }
        while matches!(self.fire.groups.last(), Some(None)) {
            self.fire.groups.pop();
        }
        Ok(())
    }

    fn spread_to(
        &self,
        rng: &mut EngineRng,
        list: &mut Vec<FireCell>,
        from: &FireCell,
        x: i32,
        y: i32,
    ) {
        let (w, h) = (self.cells.width, self.cells.height);
        if list.len() >= SPREAD_LIST || x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        if self.cells.cell_flags(x, y) & 0x200 != 0 {
            return;
        }
        for cy in (y - 1).max(0)..=(y + 1).min(h - 1) {
            for cx in (x - 1).max(0)..=(x + 1).min(w - 1) {
                if self.cells.models[self.cells.cell_index(cx, cy)] != -1 {
                    return;
                }
            }
        }
        if rng.draw(draw_site::FIRE_SPREAD_CHANCE) >= from.chance {
            return;
        }
        let chance = i32::from(from.chance).wrapping_sub(0x5555).max(0) as u16;
        if let Some(c) = list
            .iter_mut()
            .find(|c| i32::from(c.x) == x && i32::from(c.y) == y)
        {
            c.chance = c.chance.max(chance);
            return;
        }
        list.push(FireCell {
            x: x as i16,
            y: y as i16,
            chance,
            ..*from
        });
    }
}

impl crate::Simulation {
    pub(crate) fn update_engine_fire(&mut self, tick: u32) {
        if self.engine_stop.is_some() {
            return;
        }
        let world = &mut self.engine;
        if world.fire_timer.is_none() {
            return;
        }
        let mut rng = EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        let spread = match world.update_fire(&mut rng) {
            Ok(spread) => spread,
            Err(e) => {
                self.engine_stop = Some(e);
                return;
            }
        };
        let mut list = Vec::new();
        let mut slot = 0;
        loop {
            let world = &self.engine;
            if slot >= world.fire.cells.len() {
                break;
            }
            let Some(cell) = world.fire.cells[slot] else {
                slot += 1;
                continue;
            };
            if slot as u32 & 7 == tick & 7 {
                let point = (
                    i32::from(cell.x) * 0x200 + 0x100,
                    i32::from(cell.y) * 0x200 + 0x100,
                );
                let z = world.mesh_height(point.0, point.1, mandrake_tag::MeshHeightMode::Physical);
                let owner = u32::from(cell.owner_identifier as u16);
                self.combat.flight.detonating = Some(crate::explosions::ExplosionInfo {
                    unit: cell.player,
                    attacker: cell.owner,
                    attacker_id: cell.owner_identifier,
                    velocity: [0; 3],
                    flags: 0,
                    mana: 0,
                    factor_bits: 0,
                    source: None,
                });
                let r = self.cause_area_damage(
                    tick,
                    FourCC(*b"fire"),
                    FIRE_DAMAGE,
                    owner,
                    [point.0, point.1, i32::from(z as i16)],
                );
                self.combat.flight.detonating = None;
                if let Err(e) = r {
                    self.engine_stop.get_or_insert(e);
                    return;
                }
            }
            if spread {
                let world = &self.engine;
                let mut rng = EngineRng {
                    rng: &mut self.rng,
                    sites: &mut self.engine_sites,
                };
                let (x, y) = (i32::from(cell.x), i32::from(cell.y));
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    world.spread_to(&mut rng, &mut list, &cell, nx, ny);
                }
            }
            slot += 1;
        }
        let idle = list.is_empty() && (self.engine.fire.groups.is_empty());
        if idle {
            return;
        }
        self.lend_movers();
        let r = (|| -> Result<(), Unmodelled> {
            let world = &mut self.engine;
            let mut rng = EngineRng {
                rng: &mut self.rng,
                sites: &mut self.engine_sites,
            };
            for entry in list {
                let flags = world
                    .cells
                    .cell_flags(i32::from(entry.x), i32::from(entry.y));
                let a = SPREADING_CHANCE[usize::from(flags & 0xf)];
                let b = SPREADING_CHANCE[usize::from((flags >> 4) & 0xf)];
                if rng.draw(draw_site::FIRE_BURN_CHANCE)
                    <= ((u32::from(a) + u32::from(b)) >> 1) as u16
                {
                    world.start_fire(&mut rng, entry)?;
                }
            }
            world.update_mirror_groups()
        })();
        self.return_movers();
        if let Err(e) = r {
            self.engine_stop.get_or_insert(e);
        }
        self.apply_engine_orders();
    }

    pub(crate) fn projectile_starts_fire(
        &mut self,
        p: &crate::projectiles::FlightProjectile,
    ) -> Result<(), Unmodelled> {
        let world = &self.engine;
        if world.fire_timer.is_none() {
            return Ok(());
        }
        let owner = p.owner_monster;
        let id = usize::try_from(owner)
            .ok()
            .and_then(|m| world.monsters.get(m))
            .map_or(-1, |m| m.identifier);
        self.lend_movers();
        let mut rng = EngineRng {
            rng: &mut self.rng,
            sites: &mut self.engine_sites,
        };
        let r = self.engine.try_to_start_fire(
            &mut rng,
            (p.record_0a, owner, id),
            p.object.x >> 9,
            p.object.y >> 9,
        );

        self.return_movers();
        self.apply_engine_orders();
        r
    }
}
