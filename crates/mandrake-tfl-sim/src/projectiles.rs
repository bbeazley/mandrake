use mandrake_common::FourCC;

use crate::data::Def;
use crate::draw_site;
use crate::objects::{DatumPool, TflObject};
use crate::{Stop, TflSim};

pub const MAX_PROJECTILES: usize = 0x400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColourSource {
    pub object: usize,
    pub identifier: i16,
    pub source: usize,
    pub source_identifier: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflProjectile {
    pub identifier: i16,
    pub flags: u16,
    pub definition: Option<FourCC>,
    pub object: usize,
    pub target: i16,
    pub owner_unit: Option<FourCC>,
    pub owner_palette: i16,
    pub owner: i16,
    pub field_0e: i16,
    pub trail_ticks: i16,
    pub trails: i16,
    pub lifetime: i16,
    pub chance: u16,
    pub intensity: Option<i16>,
    pub delay: i16,
    pub stain: (i32, i32),
    pub lightning: i16,
    pub lightning_id: i16,
    pub delay_group: Option<FourCC>,
}

#[derive(Debug, Clone)]
pub struct Projectiles {
    pub list: Vec<TflProjectile>,
    pub pool: DatumPool,
}

impl Default for Projectiles {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            pool: DatumPool::new(MAX_PROJECTILES),
        }
    }
}

impl Projectiles {
    fn used(list: &[TflProjectile]) -> impl Fn(usize) -> bool + '_ {
        |k| list.get(k).is_some_and(|p| p.identifier != 0)
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &TflProjectile)> {
        self.list
            .iter()
            .enumerate()
            .filter(|(_, p)| p.identifier != 0)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObjectRequest {
    pub flags: u16,
    pub physics: Option<FourCC>,
    pub kind: u8,
    pub owner: i16,
    pub interest: u8,
    pub scale: i16,
    pub position: [i32; 3],
    pub velocity: [i16; 3],
    pub height_offset: i16,
    pub facing: u16,
    pub field_1e: u16,
    pub collection: Option<FourCC>,
    pub permutation: i16,
    pub terrain_mask: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectileRequest {
    pub definition: Option<FourCC>,
    pub flags: u16,
    pub position: [i32; 3],
    pub velocity: [i16; 3],
    pub height_offset: i16,
    pub facing: u16,
    pub field_1a: u16,
    pub field_1c: i16,
    pub owner_monster: i16,
    pub owner_projectile: i16,
    pub source_object: i16,
    pub target: i16,
    pub end: [i32; 3],
}

impl ProjectileRequest {
    pub(crate) fn new(definition: Option<FourCC>) -> Self {
        Self {
            definition,
            flags: 0,
            position: [0; 3],
            velocity: [0; 3],
            height_offset: 0,
            facing: 0,
            field_1a: 0,
            field_1c: 0,
            owner_monster: -1,
            owner_projectile: -1,
            source_object: -1,
            target: -1,
            end: [0; 3],
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GroupRequest {
    pub flags: u32,
    pub group: Option<FourCC>,
    pub source_object: i16,
    pub object: usize,
    pub position: [i32; 3],
    pub speed: i32,
    pub velocity: [i16; 3],
}

pub(crate) fn line_segment_to_vector(a: [i32; 3], b: [i32; 3], scale: i32) -> [i16; 3] {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let n = (d[2].wrapping_mul(d[2]))
        .wrapping_add(d[1].wrapping_mul(d[1]))
        .wrapping_add(d[0].wrapping_mul(d[0])) as u32;
    let len = crate::groups::isqrt(n).max(1) as i32;
    [
        (d[0].wrapping_mul(scale) / len) as i16,
        (d[1].wrapping_mul(scale) / len) as i16,
        (d[2].wrapping_mul(scale) / len) as i16,
    ]
}

fn magnitude(v: [i16; 3]) -> i32 {
    let n = v.iter().map(|&c| i32::from(c) * i32::from(c)).sum::<i32>() as u32;
    crate::groups::isqrt(n) as i32
}

pub(crate) type RecordPoint = ([i32; 3], Option<(usize, i16)>);

const TRAIL_4: FourCC = FourCC(*b"indc");
const TRAIL_8: FourCC = FourCC(*b"inst");

const DRIFT: [i16; 3] = [4, 4, 3];
const DRIFT_RANGE: [i16; 3] = [4, 4, 3];

impl TflSim {
    pub fn take_colour_sources(&mut self) -> Vec<ColourSource> {
        std::mem::take(&mut self.colour_sources)
    }

    fn draw_upto(&mut self, site: u32, n: i16) -> i16 {
        if n >= 1 {
            self.rng.draw_scaled(site, i32::from(n)) as i16
        } else {
            0
        }
    }

    fn proj(&mut self, id: FourCC) -> Result<std::sync::Arc<Def>, Stop> {
        self.data.def_arc(b"proj", id)
    }

    fn proj_of(&mut self, id: Option<FourCC>) -> Result<std::sync::Arc<Def>, Stop> {
        let id = id.ok_or_else(|| {
            Stop::new(
                draw_site::PROJECTILE_NO_DEFINITION,
                "a projectile without a definition",
            )
        })?;
        self.proj(id)
    }

    pub(crate) fn new_object(&mut self, r: &ObjectRequest) -> Result<Option<usize>, Stop> {
        let (w, h) = (self.width(), self.height_cells());
        let mut p = r.position;
        if p[0] < 0 || w * 0x400 <= p[0] || p[1] < 0 || h * 0x400 <= p[1] {
            tracing::debug!("### object created off map");
            p[0] = p[0].clamp(0, w * 0x400 - 1);
            p[1] = p[1].clamp(0, h * 0x400 - 1);
        }
        let cell = self.cell_at(p[0] >> 10, p[1] >> 10);
        let ground = self.mesh_height(p[0], p[1])? as i16;
        let mut obj = TflObject::new(
            p[0],
            p[1],
            r.collection.unwrap_or(FourCC([0xff; 4])),
            r.permutation,
        );
        obj.flags = 0;
        obj.physics = r.physics;
        obj.kind = r.kind;
        obj.owner = r.owner;
        obj.scale = r.scale;
        obj.interest = r.interest;
        obj.velocity = r.velocity;
        obj.facing = r.facing;
        obj.field_1e = r.field_1e;
        obj.terrain_mask = r.terrain_mask;
        obj.cell_flags = self.closed_sides(p[0], p[1], r.terrain_mask)?;
        let mut z = p[2];
        if r.flags & 2 != 0 {
            z += i32::from(ground);
        }
        let mut flags = 0u32;
        if r.flags & 1 != 0 {
            flags |= 0x10;
            z = i32::from(ground);
        } else if z < i32::from(ground) {
            z = i32::from(ground);
        }
        z += i32::from(r.height_offset);
        obj.z = z;
        obj.above_ground = (z as i16).wrapping_sub(ground);
        if z < 0 && self.cells[cell].flags & 0x18 != 0 && self.media_at(p[0], p[1])? {
            flags |= if i32::from(obj.top) + z < 0 {
                0xc0
            } else {
                0x40
            };
        }
        if r.collection.is_none() {
            flags |= 0x200;
        }
        obj.flags = flags;
        Ok(self.try_add_object(obj))
    }

    pub(crate) fn delete_object(&mut self, o: usize) -> Result<(), Stop> {
        let (x, y) = (self.objects[o].x, self.objects[o].y);
        let cell = self.cell_at(x >> 10, y >> 10);
        let mut k = self.cell_heads[cell];
        let mut carried_by = None;
        while k >= 0 && k as usize != o {
            if self.objects[k as usize].carried == o as i16 {
                carried_by = Some(k as usize);
                break;
            }
            k = self.objects[k as usize].next_in_cell;
        }
        match carried_by {
            Some(c) => self.objects[c].carried = -1,
            None => self.unlink_object(o, cell),
        }
        let carried = self.objects[o].carried;
        if carried != -1 {
            self.free_object(carried as usize);
        }
        self.free_object(o);
        Ok(())
    }

    fn free_object(&mut self, o: usize) {
        self.objects[o].identifier = 0;
        let objects = &self.objects;
        self.object_pool
            .free(o, |k| objects.get(k).is_some_and(|ob| ob.identifier != 0));
    }

    pub(crate) fn free_projectile(&mut self, k: usize) {
        self.projectiles.list[k].identifier = 0;
        let list = &self.projectiles.list;
        self.projectiles.pool.free(k, Projectiles::used(list));
    }

    pub(crate) fn new_projectile(&mut self, q: &ProjectileRequest) -> Result<Option<usize>, Stop> {
        let Some(def_id) = q.definition else {
            return Ok(None);
        };
        let def = self.proj(def_id)?;
        let f = def.u32(0);
        let mut q = q.clone();
        if f & 0x0200_0000 != 0 {
            let center = q.position;
            match self.choose_nearby_target(center, def.i16(0x90))? {
                Some(t) => q.target = t,
                None => return Ok(None),
            }
        }
        if f & 0x2000_0000 != 0 && q.target != -1 {
            let o = self.monsters[q.target as usize].object;
            q.position = self.object_centre(o);
        }
        let list = &self.projectiles.list;
        let Some((slot, salt)) = self.projectiles.pool.allocate(Projectiles::used(list)) else {
            return Ok(None);
        };
        let scale = def
            .i16(0x4c)
            .wrapping_add(self.draw_upto(draw_site::PROJECTILE_SCALE, def.i16(0x4e)));
        let extra = if def.i16(0x22) > 0 {
            self.rng
                .draw_scaled(draw_site::PROJECTILE_EXTRA, i32::from(def.i16(0x22)))
                as i16
        } else {
            0
        };
        let speed = i32::from(def.i16(0x20).wrapping_add(extra));
        let mut v = q
            .velocity
            .map(|c| ((i32::from(c) * speed) as u32 >> 8) as i16);
        let error = def.i16(0x24);
        if error != 0 {
            for (k, site) in [
                draw_site::PROJECTILE_ERROR_X,
                draw_site::PROJECTILE_ERROR_Y,
                draw_site::PROJECTILE_ERROR_Z,
            ]
            .into_iter()
            .enumerate()
            {
                let e = if error >= 1 {
                    (self.rng.draw_scaled(site, i32::from(error) * 2 + 1) as i16)
                        .wrapping_sub(error)
                } else {
                    0
                };
                v[k] = v[k].wrapping_add(e);
            }
        }
        if f & 0x40 != 0 {
            for (k, site) in [
                draw_site::PROJECTILE_DRIFT_X,
                draw_site::PROJECTILE_DRIFT_Y,
                draw_site::PROJECTILE_DRIFT_Z,
            ]
            .into_iter()
            .enumerate()
            {
                let d = self.draw_upto(site, DRIFT_RANGE[k]);
                v[k] = v[k].wrapping_add(d).wrapping_add(DRIFT[k]);
            }
        }
        let mut position = q.position;
        if f & 0x20_0000 != 0 {
            position[2] = 0;
        }
        let record = TflProjectile {
            identifier: salt,
            flags: q.flags,
            definition: Some(def_id),
            object: 0,
            target: q.target,
            owner_unit: None,
            owner_palette: -1,
            owner: -1,
            field_0e: q.field_1c,
            trail_ticks: 0,
            trails: 0,
            lifetime: 0,
            chance: 0,
            intensity: None,
            delay: 0,
            stain: (0, 0),
            lightning: -1,
            lightning_id: -1,
            delay_group: None,
        };
        if slot == self.projectiles.list.len() {
            self.projectiles.list.push(record);
        } else {
            self.projectiles.list[slot] = record;
        }
        let Some(object) = self.new_object(&ObjectRequest {
            flags: 0,
            physics: def.tag(0x1c),
            kind: 3,
            owner: slot as i16,
            interest: def.i16(0x28) as u8,
            scale,
            position,
            velocity: v,
            height_offset: q.height_offset,
            facing: q.facing,
            field_1e: q.field_1a,
            collection: def.tag(0x04),
            permutation: 0,
            terrain_mask: 0,
        })?
        else {
            self.free_projectile(slot);
            return Ok(None);
        };
        let lifetime = def
            .i16(0x54)
            .wrapping_add(self.draw_upto(draw_site::PROJECTILE_LIFETIME, def.i16(0x56)));
        let delay = def
            .i16(0x88)
            .wrapping_add(self.draw_upto(draw_site::PROJECTILE_DELAY, def.i16(0x8a)));
        let intensity = self
            .particle_intensity_at(self.objects[object].x, self.objects[object].y)
            .map(|v| v as i16);
        {
            let p = &mut self.projectiles.list[slot];
            p.object = object;
            p.lifetime = lifetime;
            p.delay = delay;
            p.chance = def.u16(0x58);
            p.intensity = intensity;
        }
        let ob = &mut self.objects[object];
        if f & 0x4_0000 != 0 {
            ob.flags |= 0x2000;
        }
        if f & 0x10_0000 != 0 {
            ob.flags |= 0x1000;
        }
        if f & 0x20_0000 != 0 {
            ob.flags |= 0x4000;
        }
        if f & 4 != 0 {
            ob.flags |= 0x8000_0000;
        }
        if f & 0x8000 != 0 {
            ob.flags = (ob.flags & 0xffff_ff3f) | 0x4000_0000;
        }
        ob.flags &= 0xffff_feef;
        let speed = def
            .i16(0x50)
            .wrapping_add(self.draw_upto(draw_site::PROJECTILE_SPEED, def.i16(0x52)));
        let shape = if f & 8 != 0 { 8 } else { 0xb };
        self.set_shape(object, def.i16(0x08), i32::from(speed), shape)?;
        if delay != 0 {
            self.objects[object].flags |= 0x200;
        }
        if f & 0x200 != 0 && self.rng.draw(draw_site::PROJECTILE_FLIP_200) & 1 != 0 {
            self.objects[object].flags |= 0x8_0000;
        }
        if f & 0x800 != 0 {
            if self.rng.draw(draw_site::PROJECTILE_FLIP_800) & 1 != 0 {
                self.objects[object].flags |= 0x2_0000;
            }
            if f & 0x8_0000 == 0 && self.rng.draw(draw_site::PROJECTILE_FLIP_80000) & 1 != 0 {
                self.objects[object].flags |= 0x4_0000;
            }
        }
        if f & 0x4000 != 0 {
            self.objects[object].flags |= 0x8000;
        }
        if matches!(def.i16(0x32), 8 | 9) {
            self.objects[object].flags |= 0x40_0000;
        }
        if def.tag(0x84).is_some() {
            return Err(Stop::new(
                draw_site::PROJECTILE_GROUP_AT_84,
                format!("proj {def_id}: a projectile group at +0x84 (object +0x3a)"),
            ));
        }
        if q.owner_monster != -1 {
            let m = &self.monsters[q.owner_monster as usize];
            let (unit, mo, palette) = (m.unit, m.object, m.palette);
            let p = &mut self.projectiles.list[slot];
            p.owner_unit = Some(unit);
            p.owner_palette = palette;
            p.owner = q.owner_monster;
            self.objects[object].ignore = mo as i16;
        }
        if q.source_object != -1 {
            let src = self.objects[q.source_object as usize].clone();
            let ob = &mut self.objects[object];
            if f & 1 != 0 && def.tag(0x04) == Some(src.collection) {
                ob.permutation = src.permutation;
                ob.permutation_index = src.permutation_index;
                ob.render_flags = src.render_flags;
                let made = ob.identifier;
                self.colour_sources.push(ColourSource {
                    object,
                    identifier: made,
                    source: q.source_object as usize,
                    source_identifier: src.identifier,
                });
            }
            if ob.collection == src.collection && ob.sequence as i8 == src.sequence as i8 {
                ob.frame = src.frame;
                ob.view = src.view;
            }
        }
        if q.owner_projectile != -1 {
            let o = self.projectiles.list[q.owner_projectile as usize].clone();
            let p = &mut self.projectiles.list[slot];
            p.field_0e = o.field_0e;
            p.owner_unit = o.owner_unit;
            p.owner_palette = o.owner_palette;
            p.owner = o.owner;
            if q.target == -1 {
                p.target = o.target;
            }
        }
        let target = self.projectiles.list[slot].target;
        if target != -1 && f & 0x4000_0000 != 0 {
            let o = self.monsters[target as usize].object;
            self.objects[o].flags |= 0x200_0000;
        }
        if q.flags & 4 != 0 {
            let ob = &mut self.objects[object];
            ob.render_flags = (ob.render_flags & 0xf9) | 1;
        }
        if q.flags & 8 != 0 {
            let ob = &mut self.objects[object];
            ob.render_flags = (ob.render_flags & 0xfc) | 4;
        }
        if f & 0x2000_0000 != 0 {
            self.projectiles.list[slot].flags |= 1;
        }
        let mut failed = false;
        if f & 0x100_0000 != 0 {
            let p = &self.projectiles.list[slot];
            let source = if p.owner == -1 {
                -1
            } else {
                self.monsters[p.owner as usize].object as i16
            };
            let bolt = if p.target != -1 {
                Some((self.monsters[p.target as usize].object as i16, q.position))
            } else if q.flags & 0x10 != 0 {
                Some((-1, q.end))
            } else {
                None
            };
            if let Some((target, end)) = bolt {
                let r = crate::lightning::LightningRequest {
                    definition: def.tag(0x34),
                    source,
                    target,
                    start: q.position,
                    end,
                };
                match self.new_lightning(&r)? {
                    Some(l) => {
                        let id = self.lightning.list[l].identifier;
                        let p = &mut self.projectiles.list[slot];
                        p.lightning = l as i16;
                        p.lightning_id = id;
                    }
                    None => {
                        self.projectiles.list[slot].lightning = -1;
                        failed = true;
                    }
                }
            } else {
                failed = true;
            }
        }
        self.start_projectile_local_group(def_id, &def, slot, object);
        if f & 0x8000_2000 != 0 {
            if f & 0x8000_0000 != 0 || target != -1 {
                let hit = if target == -1 {
                    None
                } else {
                    Some(self.monsters[target as usize].object)
                };
                self.detonate_projectile(None, 0, slot, hit, false)?;
            }
            if self.projectiles.list[slot].identifier != 0 {
                let o = self.projectiles.list[slot].object;
                self.delete_object(o)?;
                self.free_projectile(slot);
            }
            return Ok(None);
        }
        if failed {
            let tick = self.tick;
            crate::tc2_trace::log(|| format!("LB {tick} {slot}"));
            self.free_projectile(slot);
            return Ok(None);
        }
        Ok(Some(slot))
    }

    pub(crate) fn choose_nearby_target(
        &mut self,
        point: [i32; 3],
        radius: i16,
    ) -> Result<Option<i16>, Stop> {
        let r = if radius > 0 { radius } else { 4 };
        let size = r.wrapping_mul(2).wrapping_add(1);
        let corner = |c: i32, cells: i32| -> i16 {
            let v = (c >> 10).wrapping_sub(i32::from(r)) as i16;
            if v < 0 {
                0
            } else {
                let limit = (cells - i32::from(size)) as i16;
                if v > limit { limit } else { v }
            }
        };
        let x0 = corner(point[0], self.width());
        let y0 = corner(point[1], self.height_cells());
        let base = self.cell_at(i32::from(x0), i32::from(y0));
        let mut best: Option<(i16, i32)> = None;
        if size > 0 {
            for row in 0..i32::from(size) {
                for col in 0..i32::from(size) {
                    let cell = self.cell_offset(base, col, row)?;
                    let mut k = self.cell_heads[cell];
                    while k >= 0 {
                        let ob = &self.objects[k as usize];
                        if ob.flags & 0x200_0200 == 0 && ob.kind == 2 {
                            let d =
                                crate::groups::distance((point[0], point[1]), (ob.x, ob.y)) as i32;
                            if best.is_none_or(|(_, b)| d < b) {
                                best = Some((ob.owner, d));
                            }
                        }
                        k = ob.next_in_cell;
                    }
                }
            }
        }
        let Some((m, _)) = best else {
            return Ok(None);
        };
        let o = self.monsters[m as usize].object;
        self.objects[o].flags |= 0x200_0000;
        Ok(Some(m))
    }

    pub(crate) fn object_centre(&self, o: usize) -> [i32; 3] {
        let ob = &self.objects[o];
        [
            ob.x,
            ob.y,
            ((i32::from(ob.top) + i32::from(ob.bottom)) >> 1) + ob.z,
        ]
    }

    pub(crate) fn new_projectile_group(&mut self, r: &GroupRequest) -> Result<(), Stop> {
        let Some(group) = r.group else {
            return Err(Stop::new(
                draw_site::PROJECTILE_GROUP_NONE,
                "a projectile group -1",
            ));
        };
        let prgr = self.data.def_arc(b"prgr", group)?;
        if r.flags & 0x10 == 0
            && let Some(meef) = prgr.tag(0x08)
        {
            self.new_mesh_effect(meef, r.object)?;
        }
        if r.flags & 0x10 == 0 {
            self.object_sound(
                draw_site::PROJECTILE_GROUP_SOUND,
                crate::sounds::SoundKind::ProjectileGroup,
                r.object,
                prgr.tag(0x0c),
            );
        }
        self.start_group_local_group(group, &prgr, r.object);
        for k in 0..prgr.i16(4).max(0) as usize {
            let a = 0x20 + 0x20 * k;
            let extra =
                i32::from(self.draw_upto(draw_site::PROJECTILE_GROUP_EXTRA, prgr.i16(a + 0x0a)));
            let n = i32::from(prgr.i16(a + 0x08)) + extra;
            if n < 0 {
                return Err(Stop::new(
                    draw_site::PROJECTILE_GROUP_COUNT,
                    format!("prgr {group}: a count of {n}"),
                ));
            }
            for _ in 0..n {
                if self.rng.draw(draw_site::PROJECTILE_GROUP_CHANCE) <= prgr.u16(a + 0x14) {
                    self.new_projectile_group_part(
                        r,
                        prgr.i16(a + 0x0c),
                        prgr.i16(a + 0x0e),
                        prgr.tag(a),
                        prgr.u32(a + 4) as u8,
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn new_projectile_group_part(
        &mut self,
        r: &GroupRequest,
        height: i16,
        range: i16,
        def: Option<FourCC>,
        part_flags: u8,
    ) -> Result<Option<usize>, Stop> {
        let src = self.objects[r.object].clone();
        let height = self
            .draw_upto(draw_site::PROJECTILE_GROUP_HEIGHT, range)
            .wrapping_add(height);
        let mut pos = [src.x, src.y, src.z];
        if r.flags & 2 != 0 {
            pos = r.position;
        } else if part_flags & 1 == 0 {
            let rad = i32::from(src.radius);
            let dy = if rad * 2 >= 1 {
                ((self.rng.draw(draw_site::PROJECTILE_GROUP_OFFSET_X) as i32 * rad) << 1) >> 16
            } else {
                0
            };
            let dx = if rad * 2 >= 1 {
                ((self.rng.draw(draw_site::PROJECTILE_GROUP_OFFSET_Y) as i32 * rad) << 1) >> 16
            } else {
                0
            };
            let span = i32::from(src.top.wrapping_sub(src.bottom));
            pos = [
                dx - rad + src.x,
                dy - rad + src.y,
                i32::from(src.bottom)
                    + ((span * i32::from(height)) >> 8)
                    + src.z
                    + i32::from(src.bottom),
            ];
        }
        if r.flags & 2 == 0 && part_flags & 2 != 0 {
            pos[2] = i32::from(self.mesh_height(pos[0], pos[1])? as i16);
        }
        let velocity = if r.flags & 1 == 0 {
            line_segment_to_vector(pos, r.position, r.speed)
        } else {
            r.velocity
        };
        let mut q = ProjectileRequest::new(def);
        let f = match def {
            Some(d) => self.proj(d)?.u32(0),
            None => 0,
        };
        if r.flags & 4 != 0 {
            if f & 0x2100 == 0 {
                q.definition = Some(TRAIL_4);
            }
            q.flags |= 6;
        }
        if r.flags & 8 != 0 {
            q.flags |= 8;
        }
        if r.flags & 0x10 != 0 {
            q.flags |= 2;
        }
        if r.flags & 0x20 != 0
            && let Some(d) = def
            && matches!(self.proj(d)?.i16(0x32), 8 | 9)
        {
            q.flags |= 1;
        }
        q.position = pos;
        q.velocity = velocity;
        q.facing = src.facing;
        q.field_1a = src.field_1e;
        q.field_1c = 0x100;
        q.owner_monster = if src.kind == 2 { src.owner } else { -1 };
        q.owner_projectile = if src.kind == 3 { src.owner } else { -1 };
        q.source_object = r.source_object;
        q.target = -1;
        let made = self.new_projectile(&q)?;
        if let Some(k) = made
            && r.flags & 0x80 != 0
        {
            let d = self
                .rng
                .draw_scaled(draw_site::PROJECTILE_GROUP_DELAY, 0x1e) as i16;
            self.projectiles.list[k].delay = d;
            let o = self.projectiles.list[k].object;
            self.objects[o].flags |= 0x200;
        }
        Ok(made)
    }

    pub(crate) fn detonate_projectile(
        &mut self,
        point: Option<RecordPoint>,
        speed: i16,
        k: usize,
        hit: Option<usize>,
        keep: bool,
    ) -> Result<(), Stop> {
        let p = self.projectiles.list[k].clone();
        let def = self.proj_of(p.definition)?;
        let f = def.u32(0);
        if p.flags & 2 != 0 && f & 0xa000 == 0 {
            return Ok(());
        }
        if self.rng.draw(draw_site::DETONATE_CHANCE) <= def.u16(0x2a) {
            return self.promote_projectile(k);
        }
        let mut group = def.tag(0x10);
        let mut given = false;
        self.projectiles.list[k].flags |= 1;
        let o = p.object;
        if f & 0x8000 != 0 {
            let ob = &self.objects[o];
            self.bloody_mesh([ob.x, ob.y, ob.z]);
        }
        if self.projectiles.list[k].flags & 2 == 0 {
            if self.objects[o].above_ground < 0x800 && def.i16(0x0a) != -1 {
                let ob = &self.objects[o];
                let at = [ob.x, ob.y, ob.z];
                self.stain_mesh(
                    draw_site::DETONATION_STAIN,
                    def.tag(0x04),
                    def.i16(0x0a),
                    at,
                );
            }
            if def.i16(0x6c) != -1 {
                let ob = &self.objects[o];
                let attacker_id = if p.owner != -1 {
                    self.monsters[p.owner as usize].identifier
                } else {
                    0
                };
                let mut ctx = crate::damage::DamageContext {
                    attacker_palette: p.owner_palette,
                    attacker: p.owner,
                    attacker_id,
                    group: None,
                    point: [ob.x, ob.y, ob.z],
                    velocity: ob.velocity,
                    scale: p.field_0e,
                    force: 0,
                    amount: 0,
                    flags: 0,
                    record: None,
                };
                let mut d = crate::damage::DamageDef::read(&def, 0x6c);
                if f & 0x2000 == 0 {
                    d.flags |= 8;
                }
                self.damage_object(hit, &d, &mut ctx)?;
                if ctx.flags & 1 != 0 {
                    group = ctx.group;
                    given = true;
                }
            }
        }
        if let Some(group) = group {
            let flags = self.projectiles.list[k].flags;
            let mut r = GroupRequest {
                group: Some(group),
                ..GroupRequest::default()
            };
            if flags & 2 != 0 {
                r.flags = 0x10;
            }
            if flags & 4 != 0 {
                r.flags |= 4;
            }
            r.speed = i32::from(if flags & 2 == 0 {
                def.i16(0x30).max(speed)
            } else {
                speed
            });
            r.object = o;
            r.source_object = o as i16;
            match (given, hit) {
                (true, Some(h)) => {
                    r.source_object = h as i16;
                    if def.u16(0x6e) & 0x400 != 0 {
                        r.object = h;
                    }
                    r.position = self.object_centre(h);
                }
                _ => match point {
                    Some((at, record)) => {
                        r.position = match record
                            .and_then(|(e, id)| self.explosions.list.get(e).map(|x| (x, id)))
                        {
                            Some((Some(x), id)) if x.identifier != id => x.ctx.point,
                            _ => at,
                        }
                    }
                    None => r.position = self.object_centre(o),
                },
            }
            self.new_projectile_group(&r)?;
        }
        if f & 0x800_0000 != 0 {
            self.release_owner(k);
        }
        if keep {
            self.projectiles.list[k].flags &= !1;
            return Ok(());
        }
        self.delete_object(o)?;
        self.free_projectile(k);
        Ok(())
    }

    fn release_owner(&mut self, k: usize) {
        let owner = self.projectiles.list[k].owner;
        if owner != -1 {
            let m = &mut self.monsters[owner as usize];
            m.flags &= 0xbfff_ffff;
            let o = m.object;
            self.objects[o].flags &= 0xffff_fdff;
        }
    }

    pub(crate) fn promote_projectile(&mut self, k: usize) -> Result<(), Stop> {
        let p = self.projectiles.list[k].clone();
        let def = self.proj_of(p.definition)?;
        let ob = self.objects[p.object].clone();
        if def.u32(0x64) != 0xffff_ffff {
            let r = GroupRequest {
                flags: 1,
                group: def.tag(0x64),
                source_object: p.object as i16,
                object: p.object,
                position: [ob.x, ob.y, ob.z],
                speed: magnitude(ob.velocity) as i16 as i32,
                velocity: ob.velocity,
            };
            self.new_projectile_group(&r)?;
        }
        if def.u32(0x60) != 0 {
            let mut q = ProjectileRequest::new(def.tag(0x60));
            q.position = [ob.x, ob.y, ob.z];
            q.velocity = ob.velocity;
            q.facing = ob.facing;
            q.field_1a = ob.field_1e;
            q.field_1c = 0x100;
            q.owner_monster = -1;
            q.owner_projectile = k as i16;
            q.source_object = p.object as i16;
            q.target = p.target;
            self.new_projectile(&q)?;
        }
        self.delete_object(p.object)?;
        self.free_projectile(k);
        Ok(())
    }

    pub(crate) fn sleep_projectile(&mut self, k: usize) -> Result<(), Stop> {
        let p = self.projectiles.list[k].clone();
        let ob = &mut self.objects[p.object];
        ob.kind = 4;
        ob.owner = -1;
        ob.sleeping = p.definition;
        if ob.flags & 0x10_0000 != 0 {
            self.reclaim_dormant_flag(p.object);
        }
        let ob = &mut self.objects[p.object];
        ob.flags |= 0x110;
        self.free_projectile(k);
        Ok(())
    }

    pub(crate) fn wake_projectile(&mut self, o: usize) -> Result<Option<usize>, Stop> {
        let ob = self.objects[o].clone();
        let mut q = ProjectileRequest::new(ob.sleeping);
        if ob.render_flags & 7 == 1 {
            q.flags |= 6;
        }
        q.position = [ob.x, ob.y, ob.z];
        q.velocity = ob.velocity;
        q.facing = ob.facing;
        q.field_1a = ob.field_1e;
        q.field_1c = 0x100;
        q.source_object = o as i16;
        let Some(k) = self.new_projectile(&q)? else {
            return Ok(None);
        };
        let made = self.projectiles.list[k].object;
        if ob.flags & 0x10_0000 != 0 {
            self.objects[made].flags |= 0x10_0000;
        }
        if ob.flags & 0x400 != 0 {
            self.objects[made].flags |= 0x400;
        }
        self.projectile_takes_object(k, o)?;
        Ok(Some(k))
    }

    pub(crate) fn projectile_takes_object(&mut self, k: usize, o: usize) -> Result<(), Stop> {
        let made = self.projectiles.list[k].object;
        self.replace_object(o, made)?;
        self.projectiles.list[k].object = o;
        Ok(())
    }

    fn replace_object(&mut self, o: usize, other: usize) -> Result<(), Stop> {
        let mut copy = self.objects[other].clone();
        self.delete_object(other)?;
        copy.identifier = self.objects[o].identifier;
        let (x, y) = (self.objects[o].x, self.objects[o].y);
        let cell = self.cell_at(x >> 10, y >> 10);
        self.unlink_object(o, cell);
        self.objects[o] = copy;
        let cell = self.cell_at(self.objects[o].x >> 10, self.objects[o].y >> 10);
        self.objects[o].next_in_cell = self.cell_heads[cell];
        self.cell_heads[cell] = o as i16;
        Ok(())
    }

    pub(crate) fn new_projectile_marker(
        &mut self,
        m: &mandrake_tag::mesh::MarkerEntry,
        definition: Option<FourCC>,
        raw: &[u8],
    ) -> Result<Option<usize>, Stop> {
        let Some(def) = definition else {
            return Ok(None);
        };
        let be16 = |o: usize| crate::be::i16_at(raw, o).unwrap_or(0);
        let mut q = ProjectileRequest::new(Some(def));
        q.position = [m.x, m.y, m.z];
        q.velocity = [be16(0x18), be16(0x1a), be16(0x1c)];
        q.height_offset = be16(0x1e);
        q.facing = be16(0x20) as u16;
        q.field_1a = be16(0x22) as u16;
        q.field_1c = 0;
        let Some(k) = self.new_projectile(&q)? else {
            return Ok(None);
        };
        let o = self.projectiles.list[k].object;
        let objects = &self.objects;
        if let Some(id) = self.object_pool.set_identifier_with(o, m.marker_id, |j| {
            objects.get(j).map_or(0, |ob| ob.identifier)
        }) {
            self.objects[o].identifier = id;
        }
        if m.flags & 4 != 0 {
            self.projectiles.list[k].flags |= 1;
        } else if be16(0x1c) == 0 {
            self.sleep_projectile(k)?;
        }
        Ok(Some(k))
    }

    pub(crate) fn update_projectiles(&mut self) -> Result<(), Stop> {
        let tick = self.tick;
        let mut k = 0usize;
        while k < self.projectiles.pool.high {
            if self.projectiles.list[k].identifier != 0 {
                self.update_projectile(k, tick)?;
            }
            k += 1;
        }
        Ok(())
    }

    fn update_projectile(&mut self, k: usize, tick: u32) -> Result<(), Stop> {
        let o = self.projectiles.list[k].object;
        if self.projectiles.list[k].delay != 0 {
            let p = &mut self.projectiles.list[k];
            p.delay -= 1;
            if p.delay != 0 {
                return Ok(());
            }
            if let Some(group) = self.projectiles.list[k].delay_group {
                let (x, y) = (self.objects[o].x, self.objects[o].y);
                self.new_projectile_group(&GroupRequest {
                    flags: 0,
                    group: Some(group),
                    source_object: o as i16,
                    object: o,
                    position: [x, y, 0],
                    speed: 0,
                    velocity: [0; 3],
                })?;
            }
            self.objects[o].flags &= 0xffff_fdff;
        }
        let Some(def_id) = self.projectiles.list[k].definition else {
            self.animate_object(o)?;
            if self.objects[o].flags & 8 != 0 {
                self.release_owner(k);
                self.delete_object(o)?;
                self.free_projectile(k);
            }
            return Ok(());
        };
        let def = self.proj(def_id)?;
        let f = def.u32(0);
        let mut hit: Option<usize> = None;
        let mut outcome = 0u8;
        if self.objects[o].flags & 0x100 == 0 || f & 0x98 != 0 {
            self.animate_object(o)?;
            let of = self.objects[o].flags;
            if (of & 4 != 0 && f & 8 != 0) || (of & 8 != 0 && f & 0x10 != 0) {
                self.projectiles.list[k].flags &= 0xfffd;
                outcome = 1;
            }
        }
        if def.u32(0x14) != 0xffff_ffff
            && self.objects[o].flags & 0x80 == 0
            && self.projectiles.list[k].trails < def.i16(0x1a)
        {
            let p = &mut self.projectiles.list[k];
            let old = p.trail_ticks;
            p.trail_ticks = old.wrapping_add(1);
            if def.i16(0x18) < old {
                let chance = self.projectiles.list[k].chance;
                if self.rng.draw(draw_site::PROJECTILE_HEIGHT_CHANCE) <= chance {
                    self.trail(k, &def)?;
                }
            }
        }
        let mut moved = 0u32;
        if f & 0x100_0000 == 0 || self.projectiles.list[k].lightning == -1 {
            let of = self.objects[o].flags;
            if of & 0x100 == 0 {
                moved = self.move_object(o)?;
                if moved & 0x180 != 0 {
                    let t = def.i16(0x0e);
                    let mut kind = if (0..3).contains(&t) {
                        i32::from(t)
                    } else {
                        -1
                    };
                    if f & 0x8000 != 0 {
                        kind = 3;
                    }
                    if f & 0x10_0000 != 0 {
                        kind = 4;
                    }
                    if f & 0x1_0000 != 0 {
                        kind = 5;
                    }
                    if kind != -1 {
                        let group = self.media_projectile_group(kind)?;
                        let ob = &self.objects[o];
                        let r = GroupRequest {
                            flags: 3,
                            group,
                            source_object: o as i16,
                            object: o,
                            position: [ob.x, ob.y, 0],
                            speed: 0,
                            velocity: [
                                ob.velocity[0],
                                ob.velocity[1],
                                ob.velocity[2].wrapping_abs().wrapping_mul(2),
                            ],
                        };
                        self.new_projectile_group(&r)?;
                    }
                }
                if moved & 0xb != 0 {
                    self.object_sound(
                        draw_site::PROJECTILE_REBOUND_SOUND,
                        crate::sounds::SoundKind::ProjectileRebound,
                        o,
                        def.tag(0x40),
                    );
                    if moved & 2 != 0 {
                        let h = (moved >> 16) as usize;
                        hit = Some(h);
                        self.objects[o].ignore = h as i16;
                        self.projectiles.list[k].target = self.objects[h].owner;
                    }
                    if def.u16(0x2c) < self.rng.draw(draw_site::PROJECTILE_REBOUND_CHANCE) {
                        if moved & 1 != 0 && def.i16(0x0c) != -1 {
                            let ob = &self.objects[o];
                            let s = self.projectiles.list[k].stain;
                            let d = crate::groups::distance((ob.x, ob.y), s);
                            if d < 0x80 {
                                let at = [ob.x, ob.y, ob.z];
                                self.stain_mesh(
                                    draw_site::PROJECTILE_REBOUND_STAIN,
                                    def.tag(0x04),
                                    def.i16(0x0c),
                                    at,
                                );
                                let ob = &self.objects[o];
                                self.projectiles.list[k].stain = (ob.x, ob.y);
                            }
                        }
                    } else {
                        outcome = 1;
                    }
                    if f & 0x400 != 0 {
                        self.projectiles.list[k].chance = def.u16(0x58);
                    }
                    let v = magnitude(self.objects[o].velocity) as i16;
                    if i32::from(def.i16(0x68)) * 3 < i32::from(v)
                        && self.rng.draw(draw_site::PROJECTILE_MEDIA_CHANCE) <= def.u16(0x6a)
                    {
                        outcome = 1;
                    }
                }
                if moved & 0x80 != 0
                    && self.rng.draw(draw_site::PROJECTILE_MOVED_CHANCE) <= def.u16(0x2e)
                {
                    outcome = 1;
                }
                if self.objects[o].flags & 0x100 != 0 {
                    if f & 0x20 != 0 {
                        outcome = 1;
                    }
                    if f & 0x100 != 0 {
                        outcome = 3;
                    }
                }
            }
            let of = self.objects[o].flags;
            if f & 0x1_0000 != 0
                && (of & 0x40 != 0
                    || ((tick as u8).wrapping_add(k as u8) & 0x1f) == 0
                        && {
                            let Some(i) = self.projectiles.list[k].intensity else {
                                return Err(Stop::new(
                                    draw_site::PROJECTILE_INTENSITY,
                                    "the particle system's intensity on a mesh of size 0 (a division by zero)",
                                ));
                            };
                            self.rng.draw(draw_site::PROJECTILE_INTENSITY) <= i as u16
                        })
            {
                outcome = 2;
            }
            let p = &mut self.projectiles.list[k];
            p.lifetime = p.lifetime.wrapping_sub(1);
            if p.lifetime == 0 {
                outcome = 1 + u8::from(f & 0x1000 != 0);
            }
            if p.flags & 1 != 0 {
                outcome = 1;
            }
        } else if let Some(l) = {
            let p = &self.projectiles.list[k];
            self.lightning.datum(p.lightning, p.lightning_id)
        } {
            let tip = self.lightning.list[l].tip;
            let ob = &mut self.objects[o];
            ob.velocity = [
                (tip[0] as i16).wrapping_sub(ob.x as i16),
                (tip[1] as i16).wrapping_sub(ob.y as i16),
                (tip[2] as i16).wrapping_sub(ob.z as i16),
            ];
            ob.radius = 0x100;
            ob.top = 0x80;
            ob.bottom = -0x80;
            moved = self.move_object(o)?;
            if self.lightning.list[l].state == 1 {
                self.let_go_object(o);
                let end = self.lightning.list[l].end;
                let ob = &mut self.objects[o];
                (ob.x, ob.y, ob.z) = (end[0], end[1], end[2]);
                let cell = self.cell_at(end[0] >> 10, end[1] >> 10);
                self.objects[o].next_in_cell = self.cell_heads[cell];
                self.cell_heads[cell] = o as i16;
                let (t, tid) = (
                    self.lightning.list[l].target,
                    self.lightning.list[l].target_id,
                );
                if t != -1 && self.object_datum(t, tid).is_some() {
                    hit = Some(t as usize);
                }
                outcome = 1;
            } else if moved & 2 != 0 {
                let h = (moved as i32 >> 16) as usize;
                if self.objects[h].kind == 2 {
                    self.projectiles.list[k].target = self.objects[h].owner;
                    hit = Some(h);
                    let id = self.objects[h].identifier;
                    let lt = &mut self.lightning.list[l];
                    lt.target = h as i16;
                    lt.target_id = id;
                    outcome = 1;
                }
            } else if moved & 1 != 0 && {
                let definition = self.lightning.list[l].definition;
                self.data.def(b"ligh", definition)?.u32(0) & 8 == 0
            } {
                let lt = &mut self.lightning.list[l];
                lt.target = -1;
                lt.target_id = -1;
                let tip = lt.tip;
                self.set_lightning_ends(l, None, Some(tip))?;
                outcome = 1;
            }
        }
        match outcome {
            1 => {
                let keep = f & 0x2_0000 != 0 || (moved & 0x20 != 0 && f & 0x40_0000 != 0);
                self.detonate_projectile(None, 0, k, hit, keep)?;
            }
            2 => self.promote_projectile(k)?,
            3 => self.sleep_projectile(k)?,
            _ => {}
        }
        Ok(())
    }

    fn trail(&mut self, k: usize, def: &Def) -> Result<(), Stop> {
        let p = &mut self.projectiles.list[k];
        p.trails = p.trails.wrapping_add(1);
        p.trail_ticks = 0;
        let mut q = ProjectileRequest::new(def.tag(0x14));
        if p.flags & 4 != 0 {
            q.definition = Some(TRAIL_4);
        }
        if p.flags & 8 != 0 {
            q.definition = Some(TRAIL_8);
        }
        let ob = self.objects[p.object].clone();
        q.position = [ob.x, ob.y, ob.z];
        q.velocity = ob.velocity;
        q.facing = ob.facing;
        q.field_1a = ob.field_1e;
        q.field_1c = 0;
        q.owner_projectile = k as i16;
        q.source_object = p.object as i16;
        self.new_projectile(&q)?;
        let delta = def.u16(0x5c);
        if delta != 0 {
            let (chance, limit) = (
                u32::from(self.projectiles.list[k].chance),
                u32::from(def.u16(0x5a)),
            );
            let v = if limit < chance {
                let v = chance as i32 - i32::from(delta);
                if v < limit as i32 { limit } else { v as u32 }
            } else {
                let v = u32::from(delta) + chance;
                if limit < v { limit } else { v }
            };
            self.projectiles.list[k].chance = v as u16;
        }
        Ok(())
    }

    fn media_projectile_group(&mut self, kind: i32) -> Result<Option<FourCC>, Stop> {
        let Some(medi) = self.data.mesh.media_tag else {
            return Err(Stop::new(
                draw_site::MEDIA_PROJECTILE_GROUP_NO_MEDIA,
                "a media projectile group without a media",
            ));
        };
        let d = self.data.def(b"medi", medi)?;
        Ok(d.tag(0x18 + 4 * kind as usize))
    }

    pub(crate) fn footprint_splash(&mut self, object: usize, x: i32, y: i32) -> Result<(), Stop> {
        if let Some(group) = self.media_projectile_group(6)? {
            let r = GroupRequest {
                flags: 2,
                group: Some(group),
                source_object: object as i16,
                object,
                position: [x, y, 0],
                speed: 0,
                velocity: [0; 3],
            };
            self.new_projectile_group(&r)?;
        }
        Ok(())
    }
}
