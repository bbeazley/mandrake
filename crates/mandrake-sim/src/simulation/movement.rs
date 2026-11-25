use crate::*;

impl Simulation {
    pub(crate) fn client_frame(&self) -> (Fixed16, Fixed16, Fixed16) {
        match self.mesh_grid {
            Some(ref g) => (
                Fixed16::from_i32(g.width as i32) / Fixed16::from_i32(2),
                Fixed16::from_i32(g.height as i32) / Fixed16::from_i32(2),
                g.cell_size,
            ),
            None => (Fixed16::ZERO, Fixed16::ZERO, Fixed16::ONE),
        }
    }

    pub fn order_move(&mut self, unit_ids: &[u32], target_world: Vec3Fixed) {
        if self.is_replay() {
            return;
        }
        let point = self.world_to_command_point(target_world);
        for &id in unit_ids {
            let Some(u) = self.units.get_mut(&id).filter(|u| u.is_alive()) else {
                continue;
            };
            let Some(m) = u.engine.as_mut() else {
                self.engine_stop.get_or_insert(engine_rng::Unmodelled::new(
                    draw_site::ORDER_WITHOUT_MOVER,
                    format!("a move order for unit {id}, which has no movement model"),
                ));
                continue;
            };
            let Some(record) = self
                .engine
                .monsters
                .iter_mut()
                .find(|e| u32::from(e.identifier as u16) == id)
            else {
                self.engine_stop.get_or_insert(engine_rng::Unmodelled::new(
                    draw_site::ORDER_WITHOUT_MOVER,
                    format!("a move order for unit {id}, which has no monster"),
                ));
                continue;
            };
            m.order(&[point], 0, record.record_flags());
            if m.is_moving(record.flags) {
                u.state = UnitState::Moving;
            }
        }
    }

    pub(crate) fn publish_movement(&mut self, ids: &[u32]) {
        if ids.is_empty() {
            return;
        }
        let running = self.engine_stop.is_none();
        for &id in ids {
            let Some((x, y, facing, moving, monster_idx)) =
                self.units
                    .get(&id)
                    .and_then(|u| u.engine.as_ref())
                    .map(|m| {
                        let idx = m.monster_index.filter(|&i| {
                            self.engine.monsters.get(i).is_some_and(|mon| {
                                mon.identifier != 0 && mon.unit_key() == Some(id)
                            })
                        });
                        let flags = idx.map_or(0, |i| self.engine.monsters[i].flags);
                        (m.x, m.y, m.facing, m.is_moving(flags), idx)
                    })
            else {
                continue;
            };
            let p = self.command_point_to_world(x, y);
            let w = &self.engine;
            let object_z = monster_idx.map(|i| {
                if running {
                    w.monsters[i].object.z
                } else {
                    w.monster_location_height(i, (x, y))
                }
            });
            let grid = self.mesh_grid.as_ref();
            if let Some(u) = self.units.get_mut(&id) {
                u.position.x = p.x;
                u.position.z = p.z;
                if let Some(z) = object_z {
                    u.position.y = Fixed16::from_raw(
                        z.wrapping_mul(65536 / mandrake_rec::WORLD_UNITS_PER_CELL),
                    );
                } else if let Some(g) = grid {
                    u.position.y = u.ground_height(g);
                }
                let myth = i32::from(facing >> 7);
                u.facing = Angle::from_raw((256 - myth).rem_euclid(512) as u16);
                u.state = match (moving, u.state) {
                    (true, UnitState::Idle) => UnitState::Moving,
                    (false, UnitState::Moving) => UnitState::Idle,
                    (_, s) => s,
                };
            }
        }
    }
}
