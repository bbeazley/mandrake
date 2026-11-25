use crate::engine_rng::{EngineWorld, Unmodelled};
use crate::map_actions::{MapActionContext, MapActions, TYPE_INTEGER, TYPE_MONSTERS};
use crate::spatial_groups::{SpatialGroup, supergroups};
use mandrake_common::game_options;

const MAX_SUBJECTS: usize = 400;

const RINGS: i32 = 20;

impl MapActions {
    pub fn move_marker_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let subjects = self.combined(i, b"subj", TYPE_MONSTERS, MAX_SUBJECTS);
        if subjects.is_empty() {
            return Ok(2);
        }
        if let Some(team) = self.indexed(i, b"team", TYPE_INTEGER, 0) {
            let Some(center) = team_center(&ctx.world.spatial_groups, team) else {
                return Ok(2);
            };
            let point = free_cell_near(ctx.world, center)?.unwrap_or(center);
            let z = i32::from(ctx.world.mesh_height(
                center.0,
                center.1,
                mandrake_tag::MeshHeightMode::Physical,
            ) as i16);
            let yaw = crate::engine_move::arctangent(
                center.0.wrapping_sub(point.0),
                center.1.wrapping_sub(point.1),
            );
            for &id in &subjects {
                ctx.world.move_unit_marker(id as i16, point, z, yaw);
            }
            return Ok(2);
        }
        if let Some(dest) = self.indexed_point(i, b"dest", 0) {
            let monsters = identifiers_to_indexes(ctx.world, &subjects);
            if monsters.is_empty() {
                return Ok(2);
            }
            let center = ctx.world.centerpoint(&monsters);
            let d = (dest.0.wrapping_sub(center.0), dest.1.wrapping_sub(center.1));
            for m in monsters {
                ctx.world.offset_monster_location(m, d);
            }
            return Ok(2);
        }
        let targets = self.combined(i, b"targ", TYPE_MONSTERS, MAX_SUBJECTS);
        let subjects = identifiers_to_indexes(ctx.world, &subjects);
        let targets = identifiers_to_indexes(ctx.world, &targets);
        for (&s, &t) in subjects.iter().zip(&targets) {
            ctx.world.take_monster_place(s, t);
        }
        Ok(2)
    }
}

fn identifiers_to_indexes(w: &EngineWorld, ids: &[i32]) -> Vec<usize> {
    ids.iter()
        .filter_map(|&id| w.monster_index_by_identifier(id as u16))
        .collect()
}

pub fn team_center(groups: &[SpatialGroup], team: i32) -> Option<(i32, i32)> {
    let mut most = 0i16;
    let mut center = None;
    for g in supergroups(groups) {
        if i32::from(g.team) == team && g.members > most {
            most = g.members;
            center = Some(g.center);
        }
    }
    center
}

fn cell_taken(w: &EngineWorld, x: i32, y: i32) -> Result<bool, Unmodelled> {
    use crate::pathfinding::ObstacleWorld;
    let cells = &w.cells;
    if cells
        .models
        .get(cells.cell_index(x, y))
        .is_some_and(|&m| m != -1)
    {
        return Ok(true);
    }
    Ok(w.cell_objects(x, y)?
        .iter()
        .any(|o| o.flags & 0x600 == 0x400))
}

pub fn free_cell_near(
    w: &EngineWorld,
    center: (i32, i32),
) -> Result<Option<(i32, i32)>, Unmodelled> {
    let (cx, cy) = (
        i32::from((center.0 >> 9) as i16),
        i32::from((center.1 >> 9) as i16),
    );
    let (wmax, hmax) = (w.cells.width - 1, w.cells.height - 1);
    let low = |c: i32, r: i32, max: i32| if c - r < 0 { 0 } else { (c - r).min(max) };
    let high = |c: i32, r: i32, max: i32| if c + r < 0 { 0 } else { (c + r).min(max) };
    let middle = |x: i32, y: i32| ((x << 9) + 0x100, (y << 9) + 0x100);
    for r in 0..RINGS {
        let (x0, x1) = (low(cx, r, wmax), high(cx, r, wmax));
        let (y0, y1) = (low(cy, r, hmax), high(cy, r, hmax));
        let mut found = None;
        'ring: {
            for y in y0..=y1 {
                for x in [x0, x1] {
                    if !cell_taken(w, x, y)? {
                        found = Some(middle(x, y));
                        break 'ring;
                    }
                }
            }
            for x in x0..=x1 {
                for y in [y0, y1] {
                    if !cell_taken(w, x, y)? {
                        found = Some(middle(x, y));
                        break 'ring;
                    }
                }
            }
        }
        if let Some(p) = found {
            return Ok((r + 1 != RINGS).then_some(p));
        }
    }
    Ok(None)
}

impl EngineWorld {
    pub fn move_unit_marker(&mut self, id: i16, point: (i32, i32), z: i32, yaw: u16) {
        if let Some(creation) = self.marker_creation.as_mut() {
            let creation = std::sync::Arc::make_mut(creation);
            if let Some(m) = creation
                .mesh
                .markers
                .iter_mut()
                .find(|m| m.marker_type == 3 && m.marker_id == id)
            {
                m.x = point.0;
                m.y = point.1;
                m.z = z;
                m.yaw = yaw as i16;
            }
        }
        if let Some(u) = self.unit_markers.iter_mut().find(|u| u.identifier == id) {
            u.point = point;
        }
    }

    pub fn offset_monster_location(&mut self, m: usize, d: (i32, i32)) {
        let link = self.next_cell_link();
        let (x, y) = self.monsters[m].position;
        let p = (x.wrapping_add(d.0), y.wrapping_add(d.1));
        let z =
            i32::from(self.mesh_height(p.0, p.1, mandrake_tag::MeshHeightMode::Physical) as i16);
        let mon = &mut self.monsters[m];
        mon.cell_link = link;
        mon.position = p;
        mon.object.z = z;
        self.invalidate_monster_index();
        let neighbours = (self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_8_3)
            .then(|| {
                self.cells
                    .neighbour_mask(self.monsters[m].impassable, p.0, p.1)
            });
        if let Some(Some(mv)) = self.movers.get_mut(m) {
            mv.x = p.0;
            mv.y = p.1;
            if let Some(n) = neighbours {
                mv.passability.neighbours = n;
            }
        }
        if let Some(n) = neighbours {
            self.monsters[m].neighbours = n;
            self.copy_to_parasite(m);
        }
    }

    fn copy_to_parasite(&mut self, m: usize) {
        let (z, velocity, facing) = {
            let mon = &self.monsters[m];
            (mon.object.z, mon.object.velocity, mon.facing)
        };
        if let Some(r) = self.fade_records.iter_mut().find(|r| r.host == m) {
            r.object.z = z;
            r.object.velocity = velocity;
            r.facing = facing;
        }
    }

    pub fn take_monster_place(&mut self, s: usize, t: usize) {
        let link = self.next_cell_link();
        let gameplay = self.formation_globals.gameplay_version;
        let src = self.monsters[t].clone();
        let src_mover = self
            .movers
            .get(t)
            .and_then(Option::as_ref)
            .map(|mv| (mv.frame_tick, mv.ticks_per_frame));
        let mon = &mut self.monsters[s];
        mon.cell_link = link;
        mon.position = src.position;
        mon.facing = src.facing;
        mon.neighbours = src.neighbours;
        let (o, so) = (&mut mon.object, &src.object);
        o.z = so.z;
        o.velocity = so.velocity;
        o.vitality = so.vitality;
        o.max_vitality = so.max_vitality;
        o.scale = so.scale;
        o.collection = so.collection;
        o.sequence = so.sequence;
        o.core = so.core;
        o.permutation = so.permutation;
        if gameplay <= game_options::GAMEPLAY_1_7_2 {
            o.shading_slot = so.shading_slot;
        }
        o.frame = so.frame;
        o.tick = so.tick;
        o.ticks_per_frame = so.ticks_per_frame;
        o.view = so.view;
        if let Some(Some(mv)) = self.movers.get_mut(s) {
            mv.x = src.position.0;
            mv.y = src.position.1;
            mv.facing = src.facing;
            mv.velocity = src.object.velocity;
            mv.passability.neighbours = src.neighbours;
            if let Some((tick, per)) = src_mover {
                mv.frame_tick = tick;
                mv.ticks_per_frame = per;
            } else {
                mv.frame_tick = src.object.tick;
                mv.ticks_per_frame = src.object.ticks_per_frame;
            }
        }
        let tc = self.combat(t);
        let sc = self.combat_mut(s);
        sc.battles = tc.battles;
        sc.name_index = tc.name_index;
        sc.ammunition = tc.ammunition;
        sc.experience = tc.experience;
        let tcm = self.combat_mut(t);
        tcm.experience = 0;
        tcm.battles = 0;
        let name = self.combat(t).custom_name;
        self.combat_mut(s).custom_name = name;
        self.invalidate_monster_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_rng::{EngineMonster, EngineObject, MonsterFormation};
    use crate::passability::MeshCells;
    use mandrake_common::FourCC;

    fn monster(index: usize, identifier: i16, position: (i32, i32)) -> EngineMonster {
        EngineMonster {
            index,
            identifier,
            first_draw: 0,
            definition: 0,
            object: EngineObject::new(index as i32, FourCC::new(*b"test")),
            mode: 1,
            anim_state: 0x1a,
            speed: 0,
            position,
            flags: 0,
            flags_8: 0,
            neighbours: 0,
            impassable: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team: 0,
            owner: 0,
            facing: 0,
            formation: MonsterFormation::default(),
            cell_link: 0,
            flying_height: 0,
        }
    }

    fn world(taken: i32, also: &[(i32, i32)], free: &[(i32, i32)]) -> EngineWorld {
        let mut w = EngineWorld::default();
        w.cells = MeshCells {
            width: 64,
            height: 64,
            flags: vec![0; 64 * 64],
            models: vec![-1; 64 * 64],
        };
        for y in 0..64 {
            for x in 0..64 {
                let near = (x - 32i32).abs().max((y - 32i32).abs()) <= taken;
                if (near || also.contains(&(x, y))) && !free.contains(&(x, y)) {
                    w.cells.models[(y * 64 + x) as usize] = 0;
                }
            }
        }
        w
    }

    const CENTER: (i32, i32) = (32 * 512 + 100, 32 * 512 + 400);
    fn middle(x: i32, y: i32) -> Option<(i32, i32)> {
        Some((x * 512 + 0x100, y * 512 + 0x100))
    }

    #[test]
    fn the_centres_cell_when_free() {
        assert_eq!(
            free_cell_near(&world(-1, &[], &[]), CENTER).unwrap(),
            middle(32, 32)
        );
    }

    #[test]
    fn the_columns_row_by_row_then_the_rows() {
        let w = world(1, &[], &[(33, 31), (31, 32)]);
        assert_eq!(free_cell_near(&w, CENTER).unwrap(), middle(33, 31));
        let w = world(1, &[], &[(31, 32), (32, 31)]);
        assert_eq!(free_cell_near(&w, CENTER).unwrap(), middle(31, 32));
        let w = world(1, &[], &[(32, 33), (32, 31)]);
        assert_eq!(free_cell_near(&w, CENTER).unwrap(), middle(32, 31));
        let w = world(1, &[], &[(32, 33)]);
        assert_eq!(free_cell_near(&w, CENTER).unwrap(), middle(32, 33));
    }

    #[test]
    fn ring_19_counts_as_none() {
        let w = world(17, &[], &[]);
        assert_eq!(free_cell_near(&w, CENTER).unwrap(), middle(14, 14));
        assert_eq!(free_cell_near(&world(18, &[], &[]), CENTER).unwrap(), None);
        assert_eq!(free_cell_near(&world(25, &[], &[]), CENTER).unwrap(), None);
    }

    #[test]
    fn rings_clamped_to_the_mesh() {
        let w = world(-1, &[(0, 0), (1, 0), (0, 1)], &[]);
        assert_eq!(free_cell_near(&w, (100, 100)).unwrap(), middle(1, 1));
    }

    #[test]
    fn solid_monsters_take_their_cell() {
        let mut w = world(-1, &[], &[]);
        let mut m = monster(0, 1, CENTER);
        m.object.flags = 0x400;
        w.monsters = vec![m];
        w.monster_definitions = vec![
            crate::engine_rng::MonsterDefinition::from_bytes(
                FourCC::new(*b"wam1"),
                &vec![0u8; 0x224],
            )
            .unwrap(),
        ];
        assert_eq!(free_cell_near(&w, CENTER).unwrap(), middle(31, 31));
        w.monsters[0].object.no_shape = true;
        assert_eq!(free_cell_near(&w, CENTER).unwrap(), middle(32, 32));
    }

    #[test]
    fn the_subject_takes_the_targets_place_and_veteran_data() {
        let mut w = EngineWorld::default();
        w.formation_globals.gameplay_version = crate::gameplay::GAMEPLAY_1_8_5;
        let s = monster(0, 1, (1000, 2000));
        let mut t = monster(1, 2, (5000, 6000));
        t.facing = 0x4000;
        t.neighbours = 0x21;
        t.object.z = 77;
        t.object.vitality = 300;
        t.object.max_vitality = 400;
        t.object.scale = 0x180;
        t.object.collection = FourCC::new(*b"targ");
        t.object.sequence = 5;
        t.object.frame = 3;
        t.object.shading_slot = 4;
        w.monsters = vec![s, t];
        let tc = w.combat_mut(1);
        tc.experience = 9;
        tc.battles = 2;
        tc.ammunition = 6;
        tc.name_index = 7;
        tc.custom_name[0] = b'Q';
        w.take_monster_place(0, 1);
        let (s, t) = (&w.monsters[0], &w.monsters[1]);
        assert_eq!(s.position, (5000, 6000));
        assert_eq!((s.facing, s.neighbours, s.object.z), (0x4000, 0x21, 77));
        assert_eq!(
            (s.object.vitality, s.object.max_vitality, s.object.scale),
            (300, 400, 0x180)
        );
        assert_eq!(s.object.collection, t.object.collection);
        assert_eq!((s.object.sequence, s.object.frame), (5, 3));
        assert_eq!(s.object.shading_slot, 0);
        assert!(s.cell_link > t.cell_link);
        let (sc, tc) = (w.combat(0), w.combat(1));
        assert_eq!(
            (sc.experience, sc.battles, sc.ammunition, sc.name_index),
            (9, 2, 6, 7)
        );
        assert_eq!(sc.custom_name[0], b'Q');
        assert_eq!((tc.experience, tc.battles), (0, 0));
    }

    #[test]
    fn the_parasite_follows() {
        let mut w = world(-1, &[], &[]);
        w.heights = vec![(0, 0); 64 * 64];
        w.formation_globals.gameplay_version = crate::gameplay::GAMEPLAY_1_8_5;
        let mut m = monster(0, 1, (1000, 2000));
        m.facing = 0x1234;
        m.object.z = 50;
        w.monsters = vec![m];
        w.fade_records.push(crate::engine_rng::FadeRecord {
            index: 0,
            host: 0,
            object: EngineObject::new(1, FourCC::new(*b"test")),
            position: (0, 0),
            facing: 0,
            cell_link: 0,
            countdown: 0,
            group: None,
            monster: None,
        });
        w.offset_monster_location(0, (512, -512));
        assert_eq!(w.monsters[0].position, (1512, 1488));
        assert_eq!(w.monsters[0].object.z, 0);
        let r = &w.fade_records[0];
        assert_eq!((r.facing, r.object.z), (0x1234, 0));
    }
}
