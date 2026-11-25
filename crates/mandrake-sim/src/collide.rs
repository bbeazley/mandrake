use mandrake_tag::MeshHeightMode;

use crate::draw_site;
use crate::engine_move::{EngineMover, angle_index, arctangent, distance2d, trig_tables};
use crate::engine_rng::{EngineRng, EngineWorld, Unmodelled, set_object_shape};
use crate::passability::{MeshCells, ObjectHits, move_monster_with};
use crate::pathfinding::{
    MonsterView, ObjectView, ObstacleWorld, closest_point_on_segment,
    use_monster_pathfinding_radius,
};

pub const OBJECT_POSED: u32 = 0x80_0000;

pub(crate) type BumpFlag<'b> = dyn FnMut(
        &mut EngineWorld,
        &mut EngineMover,
        &mut EngineRng<'_>,
        i16,
        i16,
    ) -> Result<(), Unmodelled>
    + 'b;

pub const MONSTER_OVERLAP: u32 = 0x2_0000;

const POSE_WALKING_ALLY: i16 = 10;
const POSE_BUMPS: i16 = 30;

fn push_goes_back(
    cells: &MeshCells,
    impassable: u16,
    p: (i32, i32),
    c: (i32, i32),
    push: (i32, i32),
    o: (i32, i32),
    r: i32,
) -> bool {
    let to = |a: (i32, i32)| distance2d(a.0 - o.0, a.1 - o.1);
    let (d_push, d_start, d_candidate) = (to(push), to(p), to(c));
    let reach = if r > 0xaa { r - r / 40 } else { r - r / 20 };
    if reach > d_push && d_push < d_start && d_start >= d_candidate {
        return true;
    }
    let terrain = (cells.cell_flags(push.0 >> 9, push.1 >> 9) & 0xff) >> 4;
    impassable & (1 << terrain) != 0
}

impl EngineWorld {
    pub fn mesh_height(&self, x: i32, y: i32, mode: MeshHeightMode) -> i32 {
        let (w, h) = (
            self.cells.width.max(0) as usize,
            self.cells.height.max(0) as usize,
        );
        mandrake_tag::get_mesh_height(w, h, x, y, mode, |cx, cy| {
            let i = cy * w + cx;
            let (physical, media) = self.heights.get(i).copied().unwrap_or((0, 0));
            (
                physical,
                media,
                self.cells.flags.get(i).copied().unwrap_or(0),
            )
        })
    }

    fn step_height(&self, i: usize, point: (i32, i32), fallback: i32) -> (i32, bool) {
        let m = &self.monsters[i];
        let def = &self.monster_definitions[m.definition];
        let mut z = i32::from(self.mesh_height(point.0, point.1, MeshHeightMode::Physical) as i16);
        let mut media = false;
        if self.cells.cell_flags(point.0 >> 9, point.1 >> 9) & 0x1800 != 0 {
            media = self.media.pixel_is_media(point.0 >> 6, point.1 >> 6);
            if m.object.flags & 0x1000 != 0 {
                z = i32::from(self.mesh_height(point.0, point.1, MeshHeightMode::Media) as i16);
            }
        }
        if def.object_flags & 4 != 0 {
            let h0 = self.mesh_height(point.0, point.1, MeshHeightMode::Surface) as i16;
            z = if i32::from(h0) == mandrake_tag::MESH_HEIGHT_VOID {
                fallback
            } else {
                i32::from(h0) + i32::from(m.flying_height)
            };
        }
        (z, media)
    }

    pub fn monster_location_height(&self, i: usize, point: (i32, i32)) -> i32 {
        self.step_height(i, point, self.monsters[i].object.z).0
    }

    pub(crate) fn with_resting_projectiles(
        &self,
        list: Vec<ObjectView>,
        cell: (i32, i32),
    ) -> Vec<ObjectView> {
        let mut resting: Vec<(u64, ObjectView)> = self
            .projectiles
            .iter()
            .filter(|pr| {
                (pr.position.0 >> 9, pr.position.1 >> 9) == cell
                    && !pr.object.no_shape
                    && pr.object.flags & 0x400 != 0
            })
            .map(|pr| (pr.cell_link, self.resting_projectile_view(pr)))
            .collect();
        if resting.is_empty() {
            return list;
        }
        let mut keyed: Vec<(u64, ObjectView)> = list
            .into_iter()
            .map(|o| (self.cell_link_of(&o), o))
            .collect();
        keyed.append(&mut resting);
        keyed.sort_by_key(|&(link, _)| std::cmp::Reverse(link));
        keyed.into_iter().map(|(_, o)| o).collect()
    }

    pub(crate) fn resting_projectile_view(
        &self,
        pr: &crate::engine_rng::EngineProjectile,
    ) -> ObjectView {
        let o = &pr.object;
        let (radius, low, high) = o.bounds(self.headers(o.collection)).unwrap_or((0, 0, 0));
        ObjectView {
            index: o.index as i16,
            identifier: o.identifier as u16,
            flags: o.flags,
            position: pr.position,
            facing: pr.facing,
            radius,
            bounds_low: low,
            bounds_high: high,
            owner: o.owner,
            kind: o.kind,
            z: o.z,
        }
    }

    fn cell_link_of(&self, o: &ObjectView) -> u64 {
        let index = i32::from(o.index);
        if o.kind == 2
            && let Some(m) = usize::try_from(o.owner)
                .ok()
                .and_then(|m| self.monsters.get(m))
        {
            return m.cell_link;
        }
        let by_index = self.scenery_index.of(&self.scenery);
        if let Some(sc) = by_index
            .of_object(index)
            .map(|i| &self.scenery[i])
            .find(|s| !s.deleted)
        {
            return sc.cell_link;
        }
        if let Some(r) = self.solid_copies().find(|r| r.object.index == index) {
            return r.cell_link;
        }
        self.solid_projectile_objects()
            .find(|(_, o)| o.index == index as i16)
            .map_or(0, |(link, _)| link)
    }

    #[allow(clippy::too_many_arguments)]
    fn object_hits(
        &self,
        i: usize,
        m: &MonsterView,
        me: &ObjectView,
        p: (i32, i32),
        c: (i32, i32),
        cz: i32,
    ) -> Result<(ObjectHits, Option<(i32, bool)>), Unmodelled> {
        let em = &self.monsters[i];
        let def = &self.monster_definitions[em.definition];
        let sd = self.definition(em.definition as i16)?;
        let options = self.game_options;
        let alliances = self.alliance_view();
        let tab = trig_tables();
        let (cx, cy) = (c.0 >> 9, c.1 >> 9);
        let dx = if c.0 & 0x1ff > 0xff { 1 } else { -1 };
        let dy = if c.1 & 0x1ff > 0xff { 1 } else { -1 };
        let mut hits = ObjectHits::default();
        let mut push_height = None;
        let near = |o: (i32, i32)| distance2d(p.0 - o.0, p.1 - o.1);
        let has_resting = self
            .projectiles
            .iter()
            .any(|pr| !pr.object.no_shape && pr.object.flags & 0x400 != 0);
        let (bx, by) = (cx.min(cx + dx), cy.min(cy + dy));
        self.with_objects_in_2x2(bx, by, |lists| {
            for k in 0..4 {
                let x = cx + if k & 1 != 0 { dx } else { 0 };
                let y = cy + if k & 2 != 0 { dy } else { 0 };
                let in_box = ((y - by) * 2 + (x - bx)) as usize;
                let resting_list;
                let list: &[ObjectView] = if has_resting {
                    resting_list = self.with_resting_projectiles(lists[in_box].clone(), (x, y));
                    &resting_list
                } else {
                    &lists[in_box]
                };
                for &o in list {
                    if o.flags & 0x600 != 0x400 || o.index == me.index {
                        continue;
                    }
                    let mut r = i32::from(me.radius) + i32::from(o.radius) + 0x55;
                    match o.kind {
                        2 => {
                            let om = self.monster(o.owner)?;
                            let od = self.definition(om.definition)?;
                            let (a, b) = (sd.field_82, od.field_82);
                            let sum = i32::from(sd.radius) + i32::from(od.radius);
                            let overlap_rule =
                                em.flags_8 & MONSTER_OVERLAP != 0 && m.flags & 9 == 0;
                            if (a > 0 && b == 0) || (a == 0 && b > 0) {
                                if overlap_rule && near(o.position) <= sum {
                                    hits.overlap = true;
                                }
                                continue;
                            }
                            if use_monster_pathfinding_radius(options, &alliances, m, &sd, &om, &od)
                            {
                                r = sum;
                            }
                            if overlap_rule && near(o.position) <= sum {
                                hits.overlap = true;
                                continue;
                            }
                        }
                        3 | 4 => {
                            if o.index == m.pick_up && m.flags & 0x4000 != 0 {
                                continue;
                            }
                            if i32::from(me.radius) + i32::from(o.radius) + 0xa9 >= near(o.position)
                            {
                                continue;
                            }
                        }
                        1 => {
                            let f = self.scenery_flags_of_object(o.index)?;
                            if f & 8 != 0 || f & 0x80 != 0 {
                                continue;
                            }
                        }
                        _ => {}
                    }
                    if r <= 0 {
                        continue;
                    }
                    if hits.flags & 4 != 0 {
                        let (_, q) = closest_point_on_segment(o.position, p, c);
                        let sq = |a: (i32, i32)| {
                            let (ex, ey) = (
                                a.0.wrapping_sub(o.position.0),
                                a.1.wrapping_sub(o.position.1),
                            );
                            ex.wrapping_mul(ex).wrapping_add(ey.wrapping_mul(ey))
                        };
                        if sq(p) <= sq(q) {
                            r = 0;
                        }
                    }
                    let (ex, ey) = (c.0 - o.position.0, c.1 - o.position.1);
                    if r <= ex.abs() || r <= ey.abs() {
                        continue;
                    }
                    let dz = o.z - cz;
                    if dz <= i32::from(me.bounds_low) - i32::from(o.bounds_high)
                        || dz >= i32::from(me.bounds_high) - i32::from(o.bounds_low)
                    {
                        continue;
                    }
                    if ex.wrapping_mul(ex).wrapping_add(ey.wrapping_mul(ey)) >= r.wrapping_mul(r) {
                        continue;
                    }
                    if hits.flags & 4 != 0 {
                        hits.flags |= 8;
                        hits.stop = true;
                        return Ok((hits, None));
                    }
                    let t = angle_index(arctangent(ex, ey));
                    let mut push = (
                        ((i32::from(tab.cosine[t]) * r) >> 14) + o.position.0,
                        ((i32::from(tab.sine[t]) * r) >> 14) + o.position.1,
                    );
                    if def.field_1b5 & 8 != 0
                        && !hits.overlap
                        && push_goes_back(&self.cells, sd.impassable, p, c, push, o.position, r)
                    {
                        push = p;
                    }
                    push_height = Some(self.step_height(i, push, cz));
                    hits.flags = (hits.flags | 4) | ((o.index as u16 as u32) << 16);
                    if o.flags & 0x2000 != 0 || me.flags & 0x2000 != 0 {
                        hits.flags |= 0x40;
                    }
                    if o.kind == 3 {
                        hits.stop = true;
                        return Ok((hits, None));
                    }
                    hits.push = Some(push);
                }
            }
            Ok((hits, push_height))
        })
    }

    pub(crate) fn step_monster(
        &mut self,
        i: usize,
        mover: &mut EngineMover,
        terrain: Option<&MeshCells>,
    ) -> Result<u32, Unmodelled> {
        self.monsters[i].flags_8 &= !1;
        let [vx, vy, _] = mover.velocity;
        if self.monsters[i].object.flags & OBJECT_POSED != 0 || (vx == 0 && vy == 0) {
            self.monsters[i].flags_8 &= !MONSTER_OVERLAP;
            return Ok(0);
        }
        let Some(cells) = terrain else {
            return Ok(mover.translate(vx, vy, None).0);
        };
        let (m, mut me) = self.path_views(i, mover)?;
        let z0 = self.monsters[i].object.z;
        me.z = z0;
        let p = (mover.x, mover.y);
        let mut candidate = None;
        let mut pushed = None;
        let out = {
            let world = &*self;
            let mut objects = |c: (i32, i32)| -> Result<ObjectHits, Unmodelled> {
                let (cz, media) = world.step_height(i, c, z0);
                candidate = Some((c, cz, media));
                let (hits, push) = world.object_hits(i, &m, &me, p, c, cz)?;
                pushed = push;
                Ok(hits)
            };
            move_monster_with(
                cells,
                &mut mover.passability,
                p,
                (vx, vy),
                Some(&mut objects),
            )?
        };
        mover.x = out.x;
        mover.y = out.y;
        if out.new_leg {
            self.monsters[i].flags |= crate::engine_move::NEW_LEG;
        }
        let mut r = out.flags;
        if out.moved
            && let Some((c, cz, cmedia)) = candidate
        {
            let def = &self.monster_definitions[self.monsters[i].definition];
            let (mut z, mut media) = match (out.pushed, pushed) {
                (Some(_), Some(h)) => h,
                _ => (cz, cmedia),
            };
            if r & 0x10 != 0 {
                z = z0;
                if !(def.flags & 8 != 0 && def.field_199 & 0x10 != 0) {
                    (z, media) = self.step_height(i, (out.x, out.y), z0);
                }
            }
            let cell = self.cells.cell_index(c.0 >> 9, c.1 >> 9);
            let media_height = self.heights.get(cell).map_or(0, |h| i32::from(h.1));
            let floats = def.field_199 & 0x10 != 0;
            let o = &self.monsters[i].object;
            let high = o
                .bounds(self.collections.get(&o.collection).map_or(&[], |v| v))
                .map_or(0, |b| i32::from(b.2));
            let bits = if media && (z < media_height || floats) {
                if o.flags & 0x40 == 0 {
                    r |= 0x100;
                }
                if media_height <= z + high { 0x50 } else { 0xd0 }
            } else {
                if o.flags & 0x40 != 0 {
                    r |= 0x200;
                }
                0x10
            };
            let changed = (out.x, out.y, z) != (p.0, p.1, z0);
            let em = &mut self.monsters[i];
            if changed {
                em.flags_8 |= 1;
            }
            em.object.z = z;
            em.object.flags = (em.object.flags & !0x1f0) | bits;
        }
        if !out.overlap {
            self.monsters[i].flags_8 &= !MONSTER_OVERLAP;
        }
        Ok(r)
    }

    #[cfg(test)]
    pub(crate) fn react_monster(
        &mut self,
        i: usize,
        mover: &mut EngineMover,
        r: u32,
        rng: &mut EngineRng,
    ) -> Result<bool, Unmodelled> {
        self.react_monster_with(i, mover, r, rng, &mut |_, _, _, _, _| {
            Err(Unmodelled::new(
                draw_site::COLLIDE_FLYING_FLAG,
                "a bumped ball with no simulation",
            ))
        })
    }

    pub(crate) fn react_monster_with(
        &mut self,
        i: usize,
        mover: &mut EngineMover,
        r: u32,
        rng: &mut EngineRng,
        bump_flag: &mut BumpFlag<'_>,
    ) -> Result<bool, Unmodelled> {
        let (def_flags, blocked_sequence, walk_sequence) = {
            let def = &self.monster_definitions[self.monsters[i].definition];
            (def.flags, def.blocked_sequence, def.walk_sequence)
        };
        if def_flags & 0xc == 0 && r & 0x14 == 0 {
            self.media_check(i, mover, rng)?;
        }
        enum Next {
            Count,
            Pose10,
            Unpose,
        }
        let next = if r & 4 == 0 {
            mover.bump_count = 0;
            Next::Unpose
        } else {
            let o = self.object((r >> 16) as u16 as i16)?;
            match o.kind {
                3 => Next::Unpose,
                6 => match self.flag_data.flag(o.owner) {
                    Some(f) if f.kind == 1 => {
                        let (object, magnitude) =
                            (f.object as i16, self.monsters[i].speed.wrapping_mul(2));
                        bump_flag(self, mover, rng, object, magnitude)?;
                        Next::Unpose
                    }
                    _ => Next::Count,
                },
                2 => {
                    let j = o.owner as usize;
                    if !self.monsters_are_allied(i, j) || self.monsters[i].flags & 8 != 0 {
                        if self.engage(i, j, mover, r)? {
                            return Ok(true);
                        }
                        Next::Count
                    } else if self.monsters[j].mode != 4 || o.flags & OBJECT_POSED != 0 {
                        Next::Count
                    } else {
                        Next::Pose10
                    }
                }
                _ => Next::Count,
            }
        };
        let Self {
            monsters,
            collections,
            ..
        } = self;
        let em = &mut monsters[i];
        let headers = collections
            .get(&em.object.collection)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        match next {
            Next::Count => {
                mover.bump_count = mover.bump_count.wrapping_add(1);
                if mover.bump_count == 30 && em.flags & 8 == 0 {
                    set_object_shape(
                        &mut em.object,
                        headers,
                        i32::from(blocked_sequence),
                        0x100,
                        0,
                        rng,
                    )?;
                    em.object.flags |= OBJECT_POSED;
                    mover.pose_timer = POSE_BUMPS;
                    mover.bump_count = 0;
                } else {
                    unpose(em, mover, walk_sequence, headers, rng)?;
                }
            }
            Next::Pose10 => {
                if em.object.flags & OBJECT_POSED == 0 {
                    set_object_shape(
                        &mut em.object,
                        headers,
                        i32::from(blocked_sequence),
                        0x100,
                        0,
                        rng,
                    )?;
                    em.object.flags |= OBJECT_POSED;
                    mover.pose_timer = POSE_WALKING_ALLY;
                }
            }
            Next::Unpose => unpose(em, mover, walk_sequence, headers, rng)?,
        }
        Ok(false)
    }

    pub(crate) fn react_after_engage(
        &mut self,
        i: usize,
        mover: &mut EngineMover,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let walk_sequence = self.monster_definitions[self.monsters[i].definition].walk_sequence;
        let Self {
            monsters,
            collections,
            ..
        } = self;
        let em = &mut monsters[i];
        let headers = collections
            .get(&em.object.collection)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        unpose(em, mover, walk_sequence, headers, rng)
    }

    fn engage(
        &mut self,
        i: usize,
        j: usize,
        mover: &mut EngineMover,
        r: u32,
    ) -> Result<bool, Unmodelled> {
        let code = if r & 8 != 0
            && crate::engine_rng::angular_difference(mover.facing, mover.desired_facing) <= 0x1fff
        {
            5
        } else {
            4
        };
        if !self.monsters_are_allied(i, j) {
            for k in [i, j] {
                let m = &mut self.monsters[k];
                if m.flags_8 & 4 != 0 {
                    m.object.flags &= 0xcfdf_ffff;
                    m.flags_8 &= !0x1c;
                }
            }
        }
        self.engage_monster(i, j, code)
    }

    pub(crate) fn order_monster(
        &mut self,
        o: &crate::map_actions::MovementOrder,
        mv: &mut EngineMover,
    ) -> Result<(), Unmodelled> {
        mv.order_with(
            &o.waypoints,
            o.flags,
            o.facing,
            o.action,
            self.monsters[o.monster].record_flags(),
        );
        self.set_monster_waypoints_flags(o.monster, &o.waypoints, o.flags)
    }

    pub fn set_monster_waypoints_flags(
        &mut self,
        i: usize,
        waypoints: &[(i32, i32)],
        flags: u16,
    ) -> Result<(), Unmodelled> {
        self.combat_mut(i).pending = -1;
        let m = &mut self.monsters[i];
        m.flags_8 &= !0x4_0000;
        let attacking = m.flags & 0x18;
        let mut ending = false;
        if m.mode == 8 {
            ending = attacking != 0;
            if m.anim_state != 0xff {
                let def = &self.monster_definitions[m.definition].ai;
                if def
                    .attack(m.anim_state as i8)
                    .is_some_and(|a| a.flags_3c & 4 != 0)
                {
                    self.monsters[i].mode = 6;
                    ending = false;
                }
            }
        }
        let m = &mut self.monsters[i];
        if waypoints.len() == 1 && waypoints[0] == m.position {
            return Ok(());
        }
        m.object.flags &= !OBJECT_POSED;
        m.flags &= 0x7f27_b17f;
        if flags & 4 == 0 {
            m.flags &= 0x7f07_b162;
            m.flags_8 &= 0xf7df_fdff;
            if ending {
                m.flags_8 |= 0x4_0000;
                m.flags |= attacking;
            }
        }
        m.flags |= if waypoints.len() > 1 && flags & 1 != 0 {
            if flags & 2 != 0 { 0x1622 } else { 0x1222 }
        } else {
            0x1022
        };
        if flags & 0x20 != 0 {
            m.flags_8 |= 0x400;
        } else {
            m.flags_8 &= !0x400;
        }
        Ok(())
    }

    fn media_check(
        &mut self,
        i: usize,
        mover: &EngineMover,
        rng: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let o = &self.monsters[i].object;
        let radius = o
            .bounds(self.collections.get(&o.collection).map_or(&[], |v| v))
            .map_or(0, |b| i32::from(b.0));
        let side = if o.frame & 1 != 0 { 0x4000u16 } else { 0xc000 };
        let t = angle_index(mover.facing.wrapping_add(side));
        let tab = trig_tables();
        let f = (
            mover.x + ((i32::from(tab.cosine[t]) * radius) >> 15),
            mover.y + ((i32::from(tab.sine[t]) * radius) >> 15),
        );
        if !self.media.pixel_is_media(f.0 >> 6, f.1 >> 6) {
            if FOOTSTEP_TERRAIN >> mover.step_terrain & 1 != 0 && o.flags & 0x800 == 0 {
                self.group_events
                    .push(crate::CombatEvent::Footstep { tick: 0, point: f });
            }
            return Ok(());
        }
        if o.flags & 0xc0 != 0x40 {
            return Ok(());
        }
        let identifier = self.monsters[i].identifier;
        let group = match self.media_splash {
            None => {
                return Err(Unmodelled::new(
                    draw_site::MEDIA_WITHOUT_DEFINITION,
                    format!("monster {identifier} in media without a media definition"),
                ));
            }
            Some(None) => return Ok(()),
            Some(Some(group)) => group,
        };
        let c = self.cells.cell_index(mover.x >> 9, mover.y >> 9);
        let media = self.heights.get(c).map_or(0, |h| i32::from(h.1));
        let mut source = crate::projectile_groups::EngineSpawn::monster_object(
            self,
            i,
            (mover.x, mover.y),
            mover.facing,
        );
        source.velocity = mover.velocity;
        let desc = crate::projectile_groups::GroupDescriptor {
            flags: 2,
            group,
            shading_object: source.index,
            object: source.index,
            owner: -1,
            point: [f.0, f.1, media],
            magnitude: 0,
            velocity: [0; 3],
        };
        let mut w = crate::projectile_groups::EngineSpawn {
            world: self,
            rng,
            source: Some(source),
            host: None,
        };
        crate::projectile_groups::new_projectile_group(&mut w, &desc)
    }
}

const FOOTSTEP_TERRAIN: u32 = 0x680;

fn unpose(
    em: &mut crate::engine_rng::EngineMonster,
    mover: &EngineMover,
    walk_sequence: i16,
    headers: &[mandrake_tag::SequenceHeader],
    rng: &mut EngineRng,
) -> Result<(), Unmodelled> {
    if em.object.flags & OBJECT_POSED != 0 && mover.pose_timer < 0 {
        set_object_shape(
            &mut em.object,
            headers,
            i32::from(walk_sequence),
            0x100,
            0,
            rng,
        )?;
        em.object.flags &= !OBJECT_POSED;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine_move::{MoveMode, MovementDefinition};
    use crate::engine_rng::{
        EngineMonster, EngineObject, MonsterDefinition, MonsterFormation, SiteCounts,
    };
    use mandrake_common::FourCC;
    use mandrake_math::BungieRng;
    use mandrake_tag::{SequenceHeader, SequenceWorldBounds};

    const C: FourCC = FourCC::new(*b"thmy");

    fn seq(ticks: i16) -> SequenceHeader {
        SequenceHeader {
            number_of_views: 1,
            frames_per_view: 8,
            ticks_per_frame: ticks,
            key_frame: -1,
            world_bounds: SequenceWorldBounds {
                half_width: 120,
                bottom: -40,
                top: 580,
            },
            ..SequenceHeader::default()
        }
    }

    pub(crate) fn world(a: (i32, i32), b: (i32, i32)) -> EngineWorld {
        let mut w = EngineWorld::default();
        w.cells = MeshCells {
            width: 160,
            height: 160,
            flags: vec![0; 160 * 160],
            models: vec![-1; 160 * 160],
        };
        w.heights = vec![(0, 0); 160 * 160];
        w.collections.insert(C, vec![seq(3), seq(3), seq(30)]);
        let mut mons = vec![0u8; 0x224];
        for (o, v) in [(0x0a, 0i16), (0x0e, 2), (0x10, -1), (0x5a, 77), (0x82, 1)] {
            mons[o..o + 2].copy_from_slice(&v.to_be_bytes());
        }
        w.monster_definitions
            .push(MonsterDefinition::from_bytes(C, &mons).unwrap());
        for (i, p) in [a, b].into_iter().enumerate() {
            let mut object = EngineObject::new(1000 + i as i32, C);
            object.sequence = 0;
            object.ticks_per_frame = 3;
            object.scale = 0x100;
            object.flags = 0x0440_0410;
            object.kind = 2;
            object.owner = i as i16;
            w.monsters.push(EngineMonster {
                index: i,
                identifier: 10_000 + i as i16,
                first_draw: 0,
                definition: 0,
                object,
                mode: 4,
                anim_state: 0,
                speed: 10,
                position: p,
                flags: 0x9002,
                flags_8: 0,
                neighbours: 0,
                impassable: 0,
                ambient: None,
                unmodelled: None,
                mirrored: false,
                team: 1,
                owner: 0,
                facing: 0,
                formation: MonsterFormation::default(),
                cell_link: i as u64,
                flying_height: 0,
            });
        }
        w
    }

    pub(crate) fn mover(p: (i32, i32), v: (i16, i16)) -> EngineMover {
        let def = MovementDefinition {
            tag: C,
            per_tick: false,
            speed: 10,
            turn_rate: 0x400,
            terrain_speed: [0xff; 16],
            impassable: 0,
            flags: 0,
            slide_without_nudge: false,
            walk_half_width: 120,
            walking_sequence: 0,
            turning_sequence: -1,
            walk_ticks_per_frame: 3,
            turn_ticks_per_frame: 1,
            field_198_clear: true,
            run_sequence: -1,
            run_bonus: 0,
            field_198_bit4: false,
            field_198_bit8: false,
            pathfinding_radius: 77,
            field_82: 1,
            field_1b5: 0,
            class: 0,
        };
        let mut m = EngineMover::new(def, p.0, p.1, 0);
        m.set_object_scale(0x100);
        m.mode = MoveMode::Walking;
        m.velocity = [v.0, v.1, 0];
        m
    }

    #[test]
    fn the_push_goes_back_to_the_start() {
        let mut cells = MeshCells {
            width: 4,
            height: 4,
            flags: vec![0; 16],
            models: vec![-1; 16],
        };
        let o = (1000, 1000);
        let (p, c) = ((1300, 1000), (1250, 1000));
        assert!(push_goes_back(&cells, 0, p, c, (1190, 1000), o, 200));
        assert!(!push_goes_back(&cells, 0, p, c, (1195, 1000), o, 200));
        assert!(!push_goes_back(
            &cells,
            0,
            (1240, 1000),
            c,
            (1190, 1000),
            o,
            200
        ));
        assert!(!push_goes_back(
            &cells,
            0,
            (1190, 1000),
            (1100, 1000),
            (1190, 1000),
            o,
            200
        ));
        assert!(push_goes_back(&cells, 0, p, c, (1161, 1000), o, 0xaa));
        assert!(!push_goes_back(&cells, 0, p, c, (1162, 1000), o, 0xaa));
        cells.flags[4 + 2] = 0x35;
        assert!(push_goes_back(&cells, 1 << 3, p, c, (1195, 1000), o, 200));
        assert!(!push_goes_back(&cells, 1 << 5, p, c, (1195, 1000), o, 200));
    }

    #[test]
    fn a_step_into_a_resting_projectile_is_pushed_out() {
        let mut w = world((6679, 65886), (20_000, 20_000));
        let mut object = EngineObject::new(2000, C);
        object.sequence = 0;
        object.scale = 0x100;
        object.flags = 0x400;
        object.kind = 4;
        w.projectiles.push(crate::engine_rng::EngineProjectile {
            marker: 0,
            identifier: 7,
            definition: C,
            first_draw: 0,
            object,
            position: (6979, 65586),
            charge_time: 0,
            facing: 0,
            elevation: 0,
            cell_link: 5,
        });
        let mut m = mover((6679, 65886), (100, -100));
        let r = w.step_monster(0, &mut m, Some(&w.cells.clone())).unwrap();
        assert_eq!(r & 0xffff_0004, 4 | (2000 << 16));
        let d = distance2d(m.x - 6979, m.y - 65586);
        assert!((323..=326).contains(&d), "{d}");
        let (mut rng, mut sites) = (BungieRng::new(1), SiteCounts::default());
        let mut er = EngineRng {
            rng: &mut rng,
            sites: &mut sites,
        };
        w.react_monster(0, &mut m, r, &mut er).unwrap();
        assert_eq!(m.bump_count, 1);
        let mut w = world((6679, 65886), (20_000, 20_000));
        let mut object = EngineObject::new(2000, C);
        object.kind = 4;
        w.projectiles.push(crate::engine_rng::EngineProjectile {
            marker: 0,
            identifier: 7,
            definition: C,
            first_draw: 0,
            object,
            position: (6979, 65586),
            charge_time: 0,
            facing: 0,
            elevation: 0,
            cell_link: 5,
        });
        let mut m = mover((6679, 65886), (100, -100));
        let r = w.step_monster(0, &mut m, Some(&w.cells.clone())).unwrap();
        assert_eq!(r & 4, 0);
    }

    #[test]
    fn a_step_into_a_walking_ally_is_pushed_out_and_poses_the_walker() {
        let mut w = world((6679, 65886), (6831, 65787));
        let mut m = mover((6679, 65886), (21, -21));
        let r = w.step_monster(0, &mut m, Some(&w.cells.clone())).unwrap();
        assert_eq!(r, 4 | (1001 << 16));
        assert_eq!((m.x, m.y), (6698, 65866));
        let (mut rng, mut sites) = (BungieRng::new(1), SiteCounts::default());
        let mut er = EngineRng {
            rng: &mut rng,
            sites: &mut sites,
        };
        w.react_monster(0, &mut m, r, &mut er).unwrap();
        let o = &w.monsters[0].object;
        assert_eq!(
            (o.sequence, o.ticks_per_frame, o.frame, o.tick),
            (2, 30, 0, 0)
        );
        assert!(o.flags & OBJECT_POSED != 0);
        assert_eq!((m.pose_timer, m.bump_count), (10, 0));
        assert_eq!(er.sites.total(), 0, "the reactions draw nothing");
        m.velocity = [221, -203, 0];
        assert_eq!(
            w.step_monster(0, &mut m, Some(&w.cells.clone())).unwrap(),
            0
        );
        assert_eq!((m.x, m.y), (6698, 65866));
        m.pose_timer = -1;
        w.react_monster(0, &mut m, 0, &mut er).unwrap();
        let o = &w.monsters[0].object;
        assert_eq!((o.sequence, o.ticks_per_frame), (0, 3));
        assert!(o.flags & OBJECT_POSED == 0);
    }

    #[test]
    fn a_step_leaves_a_footstep_only_on_its_ground() {
        let mut w = world((10_000, 10_000), (20_000, 20_000));
        let mut m = mover((10_000, 10_000), (0, 0));
        let (mut rng, mut sites) = (BungieRng::new(1), SiteCounts::default());
        let start = rng.state;
        let mut fired = |w: &mut EngineWorld, m: &mut EngineMover, terrain, r| {
            let mut er = EngineRng {
                rng: &mut rng,
                sites: &mut sites,
            };
            m.step_terrain = terrain;
            w.react_monster(0, m, r, &mut er).unwrap();
            std::mem::take(&mut w.group_events)
        };
        let points = |e: Vec<crate::CombatEvent>| {
            e.iter()
                .map(|e| match e {
                    crate::CombatEvent::Footstep { point, .. } => *point,
                    e => panic!("{e:?}"),
                })
                .collect::<Vec<_>>()
        };
        for t in 0..16 {
            let want = if matches!(t, 7 | 9 | 10) {
                vec![(10_000, 9_940)]
            } else {
                vec![]
            };
            assert_eq!(points(fired(&mut w, &mut m, t, 0)), want, "terrain {t}");
        }
        assert!(fired(&mut w, &mut m, 7, 0x10).is_empty());
        w.monsters[0].object.frame = 1;
        assert_eq!(points(fired(&mut w, &mut m, 7, 0)), [(10_000, 10_060)]);
        w.monsters[0].object.flags |= 0x800;
        assert!(fired(&mut w, &mut m, 7, 0).is_empty());
        assert_eq!(rng.state, start);
    }

    #[test]
    fn the_creation_overlap_bit_lets_allies_separate() {
        let mut w = world((10_000, 10_000), (10_100, 10_000));
        w.monsters[0].flags_8 = MONSTER_OVERLAP;
        let mut m = mover((10_000, 10_000), (10, 0));
        let r = w.step_monster(0, &mut m, Some(&w.cells.clone())).unwrap();
        assert_eq!(r, 0);
        assert_eq!((m.x, m.y), (10_010, 10_000));
        assert!(w.monsters[0].flags_8 & MONSTER_OVERLAP != 0);
        w.monsters[1].position = (12_000, 10_000);
        w.step_monster(0, &mut m, Some(&w.cells.clone())).unwrap();
        assert!(w.monsters[0].flags_8 & MONSTER_OVERLAP == 0);
    }

    #[test]
    fn set_monster_waypoints_sets_the_order_bits_and_ends_the_pose() {
        let mut w = world((10_000, 10_000), (20_000, 20_000));
        w.monsters[0].flags = 0x8000;
        w.monsters[0].flags_8 = 0x4_0000;
        w.monsters[0].object.flags |= OBJECT_POSED;
        w.set_monster_waypoints_flags(0, &[(10_000, 10_000)], 0)
            .unwrap();
        assert_eq!((w.monsters[0].flags, w.monsters[0].flags_8), (0x8000, 0));
        w.set_monster_waypoints_flags(0, &[(11_000, 10_000)], 0)
            .unwrap();
        assert_eq!(w.monsters[0].flags, 0x9022);
        assert!(w.monsters[0].object.flags & OBJECT_POSED == 0);
    }

    #[test]
    fn an_order_keeps_what_the_record_got_since_the_movers_update() {
        let order = |p: (i32, i32)| crate::map_actions::MovementOrder {
            monster: 0,
            waypoints: vec![p],
            flags: 0,
            facing: 0,
            action: -1,
        };
        let mut w = world((10_000, 10_000), (20_000, 20_000));
        w.monsters[0].flags = 0x80a2;
        w.monsters[0].flags_8 = 0x10_0000;
        let mut m = mover((10_000, 10_000), (0, 0));
        w.order_monster(&order((10_000, 10_000)), &mut m).unwrap();
        assert_eq!(w.monsters[0].flags, 0x80a2);
        assert_eq!(w.monsters[0].flags_8, 0x10_0000);
        w.order_monster(&order((11_000, 10_000)), &mut m).unwrap();
        assert_eq!(w.monsters[0].flags, 0x9022);
        assert_eq!(w.monsters[0].flags_8, 0x10_0000);
    }

    #[test]
    fn a_pending_order_keeps_what_the_record_got_since_the_movers_update() {
        let mut w = world((10_000, 10_000), (20_000, 20_000));
        w.monsters[0].flags = 0x80a2;
        w.monsters[0].flags_8 = 0x10_0000;
        w.movers = vec![Some(mover((10_000, 10_000), (0, 0))), None];
        w.pending_orders.push(crate::map_actions::MovementOrder {
            monster: 0,
            waypoints: vec![(10_000, 10_000)],
            flags: 0,
            facing: 0,
            action: -1,
        });
        w.drop_waypoint_count(0).unwrap();
        assert_eq!(w.monsters[0].flags, 0x80a2);
        assert_eq!(w.monsters[0].flags_8, 0x10_0000);
    }
}
