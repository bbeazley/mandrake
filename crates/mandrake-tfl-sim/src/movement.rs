use crate::draw_site;
use crate::{Stop, TflSim};

impl TflSim {
    pub(crate) fn plan_movement(&mut self, i: usize) -> Result<(), Stop> {
        let mut action = -1i32;
        let mut goal = (0, 0);
        let mut substate = 0u8;
        let mut picking = false;
        if self.monsters[i].flags & 0x4000 != 0 {
            let (o, id) = self.monsters[i].pick_up;
            if self.monster_could_take_object_id(i, o, id)? != 0 {
                let ob = &self.objects[o as usize];
                goal = (ob.x, ob.y);
                action = 3;
                picking = true;
            } else {
                self.monsters[i].flags &= 0xffff_bfff;
                self.stop_moving(i)?;
            }
        }
        let m = &self.monsters[i];
        if !picking && m.flags & 1 != 0 {
            let range = self.target_range(i)?;
            let m = &self.monsters[i];
            let (seq_22, ranged) = {
                let mons = self.data.def(b"mons", m.definition)?;
                (mons.i16(0x22), mons.i16(0x146))
            };
            let m = &self.monsters[i];
            let mut fleeing_target = false;
            if m.flags & 0x10 == 0 {
                let t = &self.monsters[m.target as usize];
                let to = &self.objects[t.object];
                goal = (to.x, to.y);
                if t.action == 3
                    && crate::trig::angle_difference(i32::from(t.facing), i32::from(m.facing))
                        > 0x3fff
                {
                    fleeing_target = true;
                }
                if seq_22 != -1 && range < 0x1000 {
                    substate = 0x13;
                }
            } else {
                goal = (m.attack_point.0, m.attack_point.1);
            }
            if range < 0 || m.flags & 0x10_0000 != 0 {
                goal = self.point_of_retreat(i, goal);
                action = 3;
                self.monsters[i].flags |= 0x10_0000;
            } else {
                action = 3;
                let o = &self.objects[m.object];
                let facing = crate::trig::arctangent(goal.0 - o.x, goal.1 - o.y);
                let m = &mut self.monsters[i];
                m.facing = facing;
                if range == 0 && ranged == 1 && (m.flags & 0x1_0000 == 0 || fleeing_target) {
                    action = 6;
                }
                if m.flags & 4 == 0 && ranged == 1 {
                    action = 6;
                }
                if m.flags & 0x8_0000 != 0 {
                    action = 4;
                    m.flags &= 0xffff_ffe6;
                }
                let m = &mut self.monsters[i];
                m.flags &= 0xffe7_ffff;
            }
        } else if !picking && m.waypoint_count > 0 {
            goal = m.waypoints[m.waypoint_index as usize];
            action = 3;
        } else if !picking && m.flags & 0x1000 != 0 {
            let m = &mut self.monsters[i];
            m.flags &= !0x1000;
            m.facing = m.final_facing;
            action = 4;
        }
        let m = &mut self.monsters[i];
        m.flags &= !0x20;
        if action == 3 {
            m.flags &= 0xfffb_ffdf;
            m.leg = goal;
            if m.path != 0xff {
                let p = m.path;
                self.paths.delete(p);
            }
            let object = self.monsters[i].object;
            let mask = self.data.mons_closed_terrain(self.monsters[i].definition)?;
            let o = &self.objects[object];
            let start = (o.x, o.y);
            let mut req = crate::paths::PathRequest {
                flags: 0,
                mask,
                start,
                goal,
                start_cell: (0, 0),
                goal_cell: (0, 0),
            };
            let path = self.new_path(i, &mut req)?;
            self.monsters[i].path = path;
            loop {
                self.adjust_path_destination(i)?;
                let o = &self.objects[object];
                let pos = (o.x, o.y);
                let m = &mut self.monsters[i];
                let d = crate::groups::distance(pos, m.leg);
                m.leg_distance = d as i16;
                if m.flags & 8 == 0
                    || m.flags & 0x100 == 0
                    || m.flags_44 & 2 == 0
                    || m.flags & 0x80 != 0
                    || m.flags & 0x14_0000 != 0
                {
                    m.facing = crate::trig::arctangent(m.leg.0 - pos.0, m.leg.1 - pos.1);
                } else {
                    let t = m.target as usize;
                    let f = self.face_toward_monster(i, t)?;
                    self.monsters[i].facing = f;
                }
                let m = &mut self.monsters[i];
                if m.leg_distance > 0x54 || m.flags & 0x80 != 0 {
                    break;
                }
                m.flags &= 0xfffd_ffff;
                action = -1;
                if self.next_path_step(i) {
                    if self.monsters[i].flags & 0x4000 != 0 {
                        return self.reached_pick_up(i);
                    }
                    return self.next_waypoint(i);
                }
            }
        }
        if action != -1 {
            self.set_monster_action(i, action as u8, substate)?;
        }
        Ok(())
    }
}

impl TflSim {
    fn reached_pick_up(&mut self, i: usize) -> Result<(), Stop> {
        self.monsters[i].flags &= 0xffff_bfff;
        let (o, id) = self.monsters[i].pick_up;
        if self.monster_could_take_object_id(i, o, id)? == 0
            || !self.within_pick_up_reach(i, o as usize)
        {
            return self.stop_moving(i);
        }
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let object = self.monsters[i].object;
        self.objects[object].flags &= 0xff7f_ffff;
        let seq = mons.i16(0x16);
        self.begin_action(
            i,
            &mons,
            crate::monsters::ActionTransition {
                action: 10,
                substate: 0,
                seq,
                speed: 0x100,
                shape_flags: 0,
                interest_site: crate::tracking::site::NEW_PATH,
                interest: 0x19,
                reset_order: false,
                reset_target: false,
            },
        )
    }

    pub(crate) fn monster_picking_up(&mut self, i: usize) -> Result<(), Stop> {
        let object = self.monsters[i].object;
        if self.objects[object].flags & 2 != 0 {
            let (o, id) = self.monsters[i].pick_up;
            if self.object_with_identifier(o, id)
                && self.within_pick_up_reach(i, o as usize)
                && self.give_object_to_monster(i, o as usize)? != 0
            {
                let m = &self.monsters[i];
                let commander = m.commander;
                if m.flags & 2 != 0
                    && commander != -1
                    && let Some(a) =
                        crate::map_actions::index_of(&self.map_actions.actions, commander)
                {
                    self.succeed(a)?;
                }
            }
        }
        if self.objects[object].flags & 4 != 0 {
            self.stop_moving(i)?;
        }
        Ok(())
    }

    fn next_path_step(&mut self, i: usize) -> bool {
        let p = self.monsters[i].path;
        if p == 0xff {
            return true;
        }
        match self
            .paths
            .paths
            .get_mut(usize::from(p))
            .and_then(|p| p.as_mut())
        {
            Some(path) => {
                path.index = path.index.wrapping_add(1);
                path.points.len() as i16 <= path.index
            }
            None => true,
        }
    }

    pub(crate) fn monster_moving(&mut self, i: usize) -> Result<(), Stop> {
        let (def, action, mflags, field_4a) = {
            let m = &self.monsters[i];
            (m.definition, m.action, m.flags, m.field_4a)
        };
        let mons = self.data.def_arc(b"mons", def)?;
        if matches!(action, 3 | 6)
            && mflags & 9 != 0
            && field_4a < 1
            && mflags & 0x8_0000 == 0
            && self.try_monster_attack(i)?
        {
            return Ok(());
        }
        let (action, leg_distance, mflags, facing, object) = {
            let m = &self.monsters[i];
            (m.action, m.leg_distance, m.flags, m.facing, m.object)
        };
        if action == 3 && leg_distance < 1 {
            self.monsters[i].flags |= 0x20;
        }
        let mflags_def = mons.u32(0);
        if self.objects[object].flags & 1 == 0 && mflags_def & 1 == 0 {
            return Ok(());
        }
        let turn = i32::from(facing) - i32::from(self.objects[object].facing);
        if action == 3 {
            if mflags & 0x80 == 0 && leg_distance <= 0x200 && turn != 0 {
                self.objects[object].velocity = [0; 3];
            } else {
                self.monster_step(i, &mons)?;
            }
            if (mflags_def >> 8) as u8 & 0x80 != 0 && self.objects[object].flags & 1 != 0 {
                return Err(Stop::new(
                    draw_site::MONSTER_TRAIL,
                    "a monster leaving a trail (duplicate_object)",
                ));
            }
        }
        let action = self.monsters[i].action;
        let mut abs = turn.abs();
        if turn == 0 {
            if action == 3 {
                return Ok(());
            }
            abs = 0;
        }
        let mut rate = i32::from(mons.u16(0x88));
        if mflags_def & 1 == 0 {
            rate *= i32::from(self.objects[object].tick);
        }
        let facing = if abs <= rate {
            match action {
                4 => {
                    self.stop_moving(i)?;
                    facing
                }
                7 => {
                    self.begin_attacking(i)?;
                    self.objects[object].facing = facing;
                    return Ok(());
                }
                _ => facing,
            }
        } else {
            if (abs < 0x8001 && turn < 0) || (abs >= 0x8001 && turn > 0) {
                rate = -rate;
            }
            let f = (rate as u16).wrapping_add(self.objects[object].facing);
            if action == 6 {
                self.set_monster_action(i, 4, 0)?;
            }
            f
        };
        self.objects[object].facing = facing;
        Ok(())
    }
}

impl TflSim {
    pub(crate) fn model_at(&self, cx: i32, cy: i32) -> Option<i16> {
        let c = self.cell_at(cx, cy);
        if self.cells[c].flags & 0x80 == 0 {
            return None;
        }
        let mut k = self.cell_heads[c];
        while k >= 0 {
            k = self.objects[k as usize].next_in_cell;
        }
        (k != -1).then_some(-2 - k)
    }
}

pub mod moved {
    pub const GROUND: u32 = 1;
    pub const OBJECT: u32 = 2;
    pub const SECOND_OBJECT: u32 = 4;
    pub const BLOCKED: u32 = 8;
    pub const EDGE: u32 = 0x10;
    pub const PASS_THROUGH: u32 = 0x20;
    pub const CELL_CHANGED: u32 = 0x40;
    pub const ENTERED_MEDIA: u32 = 0x80;
    pub const LEFT_MEDIA: u32 = 0x100;
}

const EDGE: i32 = 0x7c0;

impl TflSim {
    pub(crate) fn move_object(&mut self, o: usize) -> Result<u32, Stop> {
        let (w, h) = (self.width(), self.height_cells());
        let obj = self.objects[o].clone();
        let old_cell = self.cell_at(obj.x >> 10, obj.y >> 10);
        let mut flags = 0u32;
        let mut stop = false;
        let mut collided = false;
        let mut nx = obj.x + i32::from(obj.velocity[0]);
        let mut ny = obj.y + i32::from(obj.velocity[1]);
        let mut v = obj.velocity;
        if nx < EDGE {
            nx = EDGE;
            v[0] = v[0].wrapping_neg();
            flags = moved::EDGE;
        }
        if w * 0x400 - EDGE <= nx {
            nx = w * 0x400 - EDGE - 1;
            v[0] = v[0].wrapping_neg();
            flags = moved::EDGE;
        }
        if ny < EDGE {
            ny = EDGE;
            v[1] = v[1].wrapping_neg();
            flags = moved::EDGE;
        }
        if h * 0x400 - EDGE <= ny {
            ny = h * 0x400 - EDGE - 1;
            v[1] = v[1].wrapping_neg();
            flags = moved::EDGE;
        }
        self.objects[o].velocity = v;
        let mut cell = self.cell_at(nx >> 10, ny >> 10);
        let mut ground = self.mesh_height(nx, ny)? as i16;
        let media = if self.cells[cell].flags & 0x18 == 0 {
            false
        } else {
            let m = self.media_at(nx, ny)?;
            if obj.flags & 0x1000 != 0 && ground < 0 {
                ground = 0;
            }
            m
        };
        let mut normal = [0i16; 3];
        let nz: i32;
        let above_ground: i16;
        let mut bits: u32;
        if obj.flags & 0x10 == 0 {
            let def = obj.physics.ok_or_else(|| {
                Stop::new(
                    draw_site::MOVE_OBJECT_NO_PHYSICS,
                    format!("object {o}: no physics"),
                )
            })?;
            let phys = self.data.def(b"obje", def)?;
            let (gravity, terminal) = (phys.i16(0), phys.i16(4));
            let ob = &mut self.objects[o];
            if terminal == 0 || terminal < ob.velocity[2] {
                ob.velocity[2] = ob.velocity[2].wrapping_add(gravity);
            }
            let mut z = i32::from(ob.velocity[2]) + obj.z;
            bits = 0;
            if z < i32::from(ground) {
                normal = if !media || ground != 0 {
                    self.mesh_normal(nx, ny)
                } else {
                    [0, 0, -0x4000]
                };
                flags |= moved::GROUND;
                if media {
                    bits = 0x40;
                }
                collided = true;
                z = i32::from(ground);
            }
            nz = z;
            above_ground = (z as i16).wrapping_sub(ground);
        } else {
            nz = i32::from(ground);
            above_ground = 0;
            bits = if v == [0, 0, 0] { 0x110 } else { 0x10 };
        }
        if media && nz < 0 {
            if obj.flags & 0x40 == 0 {
                flags |= moved::ENTERED_MEDIA;
            }
            bits |= if i32::from(obj.top) + nz < 0 {
                0xc0
            } else {
                0x40
            };
        } else if obj.flags & 0x40 != 0 {
            flags |= moved::LEFT_MEDIA;
        }
        let mut nz = nz;
        if obj.flags & 0x8000 == 0 {
            if obj.flags & 0x10 == 0 {
                let mut to = [nx, ny, nz];
                if let Some(n) = self.model_collision([obj.x, obj.y, obj.z], &mut to, obj.radius)? {
                    normal = n;
                    nx = to[0];
                    ny = to[1];
                    nz = to[2];
                    cell = self.cell_at(nx >> 10, ny >> 10);
                    flags |= moved::GROUND;
                    collided = true;
                }
            }
            let col = if nx & 0x3ff >= 0x200 { 1 } else { -1 };
            let row = if ny & 0x3ff >= 0x200 { 1 } else { -1 };
            'cells: for k in 0..4 {
                if stop {
                    break;
                }
                let dy = if k & 2 != 0 { row } else { 0 };
                let dx = if k & 1 != 0 { col } else { 0 };
                let c = self.cell_offset(cell, dx, dy)?;
                let mut next = self.cell_heads[c];
                while next >= 0 {
                    let i = next as usize;
                    let other = &self.objects[i];
                    next = other.next_in_cell;
                    if other.flags & 0x400 == 0
                        || other.flags & 0x200 != 0
                        || i == o
                        || i as i16 == obj.ignore
                    {
                        continue;
                    }
                    let dx = nx - other.x;
                    let dy = ny - other.y;
                    let dz = other.z - nz;
                    let dz_old = other.z - obj.z;
                    let r = i32::from(other.radius) + i32::from(obj.radius);
                    let below = i32::from(obj.bottom) - i32::from(other.top);
                    let above = i32::from(obj.top) - i32::from(other.bottom);
                    if dx.abs() >= r
                        || dy.abs() >= r
                        || dz <= below
                        || dz >= above
                        || dy * dy + dx * dx >= r * r
                    {
                        continue;
                    }
                    if flags & moved::OBJECT != 0 {
                        flags |= moved::SECOND_OBJECT;
                        stop = true;
                        break 'cells;
                    }
                    if dz_old < below && bits & 0x10 == 0 {
                        normal = [0, 0, 0x4000];
                        nz = i32::from(other.top) + other.z;
                    } else if above < dz_old && bits & 0x10 == 0 {
                        normal = [0, 0, -0x4000];
                        nz = i32::from(other.bottom) + other.z;
                    } else {
                        let a = crate::trig::index(i32::from(crate::trig::arctangent(dx, dy)));
                        let t = crate::trig::trig();
                        let (c, s) = (t.cosine[a], t.sine[a]);
                        normal = [c, s, 0];
                        nx = ((i32::from(c) * r) >> 14) + other.x;
                        ny = ((i32::from(s) * r) >> 14) + other.y;
                    }
                    flags |= ((i as u32) << 16) | moved::OBJECT;
                    if other.flags & 0x2000 != 0 || obj.flags & 0x2000 != 0 {
                        flags |= moved::PASS_THROUGH;
                    }
                    collided = true;
                    if flags & moved::BLOCKED != 0 || other.kind == 3 {
                        stop = true;
                        break 'cells;
                    }
                }
            }
        }
        if !stop && obj.terrain_mask != 0 {
            let sides = obj.cell_flags;
            let (ocx, ocy) = (obj.x >> 10, obj.y >> 10);
            let (mut cx, mut cy) = (nx >> 10, ny >> 10);
            if sides != 0 {
                if (cx < ocx && sides & 0x80 != 0) || (ocx < cx && sides & 0x20 != 0) {
                    nx = obj.x;
                    cx = ocx;
                    flags |= moved::BLOCKED;
                }
                if (cy < ocy && sides & 0x40 != 0) || (ocy < cy && sides & 0x10 != 0) {
                    ny = obj.y;
                    cy = ocy;
                    flags |= moved::BLOCKED;
                }
            }
            if cx == ocx && cy == ocy {
                if sides != 0 {
                    let mut fx = nx & 0x3ff;
                    if ((obj.y ^ obj.x) >> 10) & 1 == 0 {
                        fx = 0x3ff - fx;
                    }
                    let side = if fx < ny & 0x3ff {
                        sides & 1
                    } else {
                        sides & 2
                    };
                    if side != 0 {
                        nx = obj.x;
                        ny = obj.y;
                        flags |= moved::BLOCKED;
                    }
                }
            } else {
                let r = self.closed_sides(nx, ny, obj.terrain_mask)?;
                let open = if r & 3 == 3 || r & 0xf0 == 0xf0 {
                    false
                } else if (cx ^ cy) & 1 == 0 {
                    r & 0x62 != 0x62 && r & 0x91 != 0x91
                } else {
                    r & 0xc2 != 0xc2 && r & 0x31 != 0x31
                };
                if !open {
                    flags |= moved::BLOCKED;
                    stop = true;
                }
            }
        }
        if flags & moved::BLOCKED != 0 {
            if flags & moved::OBJECT != 0 {
                stop = true;
            }
            ground = self.mesh_height(nx, ny)? as i16;
            nz = i32::from(ground);
            cell = self.cell_at(nx >> 10, ny >> 10);
        }
        if stop {
            return Ok(flags);
        }
        if collided {
            if flags & moved::PASS_THROUGH == 0 {
                let def = obj.physics.ok_or_else(|| {
                    Stop::new(
                        draw_site::MOVE_OBJECT_NO_PHYSICS_2,
                        format!("object {o}: no physics"),
                    )
                })?;
                let mut e = i32::from(self.data.def(b"obje", def)?.i16(2));
                if flags & moved::OBJECT == 0 {
                    let n = i32::from(normal[2]).abs();
                    if n < 0x800 && e < 0xc0 {
                        e += ((0xc0 - e) * (0x800 - n)) >> 11;
                    }
                }
                let v = &mut self.objects[o].velocity;
                let p = (i32::from(v[1]) * i32::from(normal[1])
                    + i32::from(v[2]) * i32::from(normal[2])
                    + i32::from(v[0]) * i32::from(normal[0]))
                    >> 14;
                for k in 0..3 {
                    let r = v[k].wrapping_sub(((i32::from(normal[k]) * p) >> 13) as i16);
                    v[k] = ((i32::from(r) * e) as u32 >> 8) as i16;
                }
            }
            if flags & moved::GROUND != 0 {
                let v = &mut self.objects[o].velocity;
                if v.iter().map(|&c| i32::from(c).abs()).sum::<i32>() < 10 {
                    *v = [0; 3];
                    bits |= if media && ground == 0 { 0x130 } else { 0x110 };
                }
            }
            cell = self.cell_at(nx >> 10, ny >> 10);
        }
        if self.objects[o].flags & 0x80_0000 == 0 {
            let ob = &mut self.objects[o];
            ob.x = nx;
            ob.y = ny;
            ob.z = nz;
            ob.above_ground = above_ground;
            if old_cell != cell {
                self.unlink_object(o, old_cell);
                self.objects[o].next_in_cell = self.cell_heads[cell];
                self.cell_heads[cell] = o as i16;
                flags |= moved::CELL_CHANGED;
                let mask = self.objects[o].terrain_mask;
                self.objects[o].cell_flags = self.closed_sides(nx, ny, mask)?;
            }
        }
        let ob = &mut self.objects[o];
        ob.flags = (ob.flags & 0xffff_fe0f) | (bits & 0xffff);
        Ok(flags)
    }

    pub(crate) fn unlink_object(&mut self, o: usize, cell: usize) {
        let next = self.objects[o].next_in_cell;
        if self.cell_heads[cell] == o as i16 {
            self.cell_heads[cell] = next;
            return;
        }
        let mut k = self.cell_heads[cell];
        while k >= 0 {
            let n = self.objects[k as usize].next_in_cell;
            if n == o as i16 {
                self.objects[k as usize].next_in_cell = next;
                return;
            }
            k = n;
        }
    }
}

impl TflSim {
    fn monster_step(&mut self, i: usize, mons: &crate::data::Def) -> Result<(), Stop> {
        let object = self.monsters[i].object;
        let o = &self.objects[object];
        let terrain = self.terrain_at((o.x, o.y));
        let mflags = mons.u32(0);
        let mut base = (i32::from(
            mons.0
                .get(0x68 + usize::from(terrain))
                .copied()
                .unwrap_or(0),
        ) + 1)
            * i32::from(mons.i16(0x8a));
        if self.monsters[i].substate == 0x13 {
            base <<= 1;
        }
        let step = if mflags & 1 != 0 {
            base
        } else {
            i32::from(o.tick) * base
        } >> 8;
        let a = crate::trig::index(i32::from(o.facing));
        let t = crate::trig::trig();
        let (c, s) = (i32::from(t.cosine[a]), i32::from(t.sine[a]));
        let ob = &mut self.objects[object];
        ob.velocity = [((c * step) >> 14) as i16, ((s * step) >> 14) as i16, 0];
        ob.radius = ob.radius.wrapping_add(0xaa);
        let r = self.move_object(object);
        let ob = &mut self.objects[object];
        ob.radius = ob.radius.wrapping_sub(0xaa);
        let r = r?;
        let m = &mut self.monsters[i];
        m.leg_distance = m.leg_distance.wrapping_sub(step as i16);
        if mflags & 1 == 0 {
            let b = base >> 8;
            let ob = &mut self.objects[object];
            ob.velocity[0] = ((c * b) >> 14) as i16;
            ob.velocity[1] = ((s * b) >> 14) as i16;
            if r & 0xa == 0 {
                self.footprint_mesh(object, terrain)?;
            }
        }
        let mut waits = false;
        if r & moved::OBJECT == 0 {
            self.monsters[i].field_16 = 0;
        } else {
            let hit = (r >> 16) as usize;
            match self.objects[hit].kind {
                2 => waits = self.bump_into_monster(i, hit, r, mons)?,
                3 => {}
                6 => {
                    if !self.monster_hits_object(i, hit, mons)? {
                        waits = self.blocked_step(i, mons)?;
                    }
                }
                _ => waits = self.blocked_step(i, mons)?,
            }
        }
        if !waits && self.objects[object].flags & 0x80_0000 != 0 && self.monsters[i].field_4a < 0 {
            let seq = mons.i16(0x0a);
            self.set_shape(object, seq, 0x100, 0)?;
            self.objects[object].flags &= !0x80_0000;
        }
        if r & moved::EDGE != 0 {
            self.objects[object].flags |= 0x200;
            self.monsters[i].flags |= 0x4000_0000;
            self.monster_removed(i)?;
        }
        Ok(())
    }

    fn bump_into_monster(
        &mut self,
        i: usize,
        hit: usize,
        r: u32,
        mons: &crate::data::Def,
    ) -> Result<bool, Stop> {
        let other = self.objects[hit].owner as usize;
        let m = &self.monsters[i];
        if m.team != self.monsters[other].team || m.flags & 8 != 0 {
            let object = m.object;
            let mut mode = 4;
            if r & moved::SECOND_OBJECT != 0
                && crate::trig::angle_difference(
                    i32::from(self.objects[object].facing),
                    i32::from(m.facing),
                ) < 0x2000
            {
                mode = 5;
            }
            if m.team as i8 != self.monsters[other].team as i8 {
                for k in [i, other] {
                    if self.monsters[k].flags_44 & 8 != 0 {
                        let o = self.monsters[k].object;
                        self.objects[o].flags &= 0xcfff_ffff;
                        self.monsters[k].flags_44 &= 0xc7;
                    }
                }
            }
            if self.try_and_switch_target(i, other, mode)? {
                self.monsters[i].substate = 0;
                if mons.i16(0x146) != 1 {
                    self.set_monster_action(i, 6, 0)?;
                }
                return Ok(true);
            }
            return self.blocked_step(i, mons);
        }
        if self.monsters[other].action != 3 || self.objects[hit].flags & 0x80_0000 != 0 {
            return self.blocked_step(i, mons);
        }
        let object = self.monsters[i].object;
        if self.objects[object].flags & 0x80_0000 == 0 {
            self.wait(i, mons, 10)?;
        }
        Ok(true)
    }

    fn blocked_step(&mut self, i: usize, mons: &crate::data::Def) -> Result<bool, Stop> {
        let m = &mut self.monsters[i];
        m.field_16 = m.field_16.wrapping_add(1);
        if m.field_16 == 0x1e && m.flags & 8 == 0 {
            self.wait(i, mons, 0x1e)?;
            self.monsters[i].field_16 = 0;
            return Ok(true);
        }
        Ok(false)
    }

    fn wait(&mut self, i: usize, mons: &crate::data::Def, ticks: i16) -> Result<(), Stop> {
        let object = self.monsters[i].object;
        let seq = mons.i16(0x26);
        self.set_shape(object, seq, 0x100, 0)?;
        self.objects[object].flags |= 0x80_0000;
        self.monsters[i].field_4a = ticks;
        Ok(())
    }

    fn footprint_mesh(&mut self, object: usize, terrain: u8) -> Result<(), Stop> {
        let o = &self.objects[object];
        let side: i32 = if o.frame & 1 != 0 { 0x8000 } else { 0 };
        let a = crate::trig::index(side - 0x4000 + i32::from(o.facing));
        let t = crate::trig::trig();
        let r = i32::from(o.radius);
        let x = ((i32::from(t.cosine[a]) * r) >> 15) + o.x;
        let y = ((i32::from(t.sine[a]) * r) >> 15) + o.y;
        let flags = o.flags;
        if self.media_at(x, y)? {
            if flags & 0x40 != 0 && flags & 0x80 == 0 {
                self.footprint_splash(object, x, y)?;
            }
        } else if (1u32 << terrain) & 0x680 != 0 && flags & 0x800 == 0 {
            self.footprint(object, terrain, (x, y));
        }
        Ok(())
    }
}
