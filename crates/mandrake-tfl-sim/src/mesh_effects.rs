use mandrake_common::FourCC;

use crate::draw_site;
use crate::objects::DatumPool;
use crate::{Stop, TflSim};

pub const MAX_MESH_EFFECTS: usize = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshEffect {
    pub identifier: i16,
    pub definition: FourCC,
    pub cell: (i16, i16),
    pub ticks: i32,
}

#[derive(Debug, Clone)]
pub struct MeshEffects {
    pub slots: Vec<Option<MeshEffect>>,
    pub pool: DatumPool,
}

impl Default for MeshEffects {
    fn default() -> Self {
        Self {
            slots: vec![None; MAX_MESH_EFFECTS],
            pool: DatumPool::new(MAX_MESH_EFFECTS),
        }
    }
}

struct Meef {
    flags: u32,
    amplitude: i32,
    speed: i32,
    coefficient: f32,
    radius: i32,
    start: i32,
}

fn clamp(v: i32, n: i32) -> i32 {
    if v < 0 { 0 } else { v.min(n - 1) }
}

impl TflSim {
    fn meef(&mut self, id: FourCC) -> Result<Meef, Stop> {
        let d = self.data.def(b"meef", id)?;
        Ok(Meef {
            flags: d.u32(0),
            amplitude: i32::from(d.i16(4)),
            speed: d.u32(8) as i32,
            coefficient: f32::from_bits(d.u32(0xc)),
            radius: i32::from(d.i16(0x10)),
            start: d.u32(0x14) as i32,
        })
    }

    pub(crate) fn new_mesh_effect(&mut self, id: FourCC, object: usize) -> Result<(), Stop> {
        let m = self.meef(id)?;
        if m.flags & 2 != 0 {
            return Err(Stop::new(
                draw_site::MESH_EFFECT_HEIGHT_ARRAY,
                format!("meef {id}: a mesh effect with a height array (flag 2)"),
            ));
        }
        let fx = &mut self.mesh_effects;
        let slots = &fx.slots;
        let Some((k, identifier)) = fx.pool.allocate(|j| slots[j].is_some()) else {
            return Ok(());
        };
        let o = &self.objects[object];
        self.mesh_effects.slots[k] = Some(MeshEffect {
            identifier,
            definition: id,
            cell: (((o.x + 0x200) >> 10) as i16, ((o.y + 0x200) >> 10) as i16),
            ticks: 0,
        });
        Ok(())
    }

    pub(crate) fn update_mesh_effects(&mut self) -> Result<(), Stop> {
        for k in 0..self.mesh_effects.pool.high {
            if self.mesh_effects.slots[k].is_some() {
                self.update_mesh_effect(k)?;
            }
        }
        Ok(())
    }

    fn update_mesh_effect(&mut self, k: usize) -> Result<(), Stop> {
        let e = self.mesh_effects.slots[k].expect("in use");
        let m = self.meef(e.definition)?;
        let (w, h) = (self.width(), self.height_cells());
        let (cx, cy) = (i32::from(e.cell.0), i32::from(e.cell.1));
        let r = m.radius;
        let (x0, x1) = (clamp(cx - r, w), clamp(cx + r, w));
        let (y0, y1) = (clamp(cy - r, h), clamp(cy + r, h));
        if m.speed == 0 {
            return Err(Stop::new(
                draw_site::MESH_EFFECT_SPEED_ZERO,
                "a mesh effect of speed 0",
            ));
        }
        if (r << 16) / m.speed < e.ticks {
            self.mesh_effects.slots[k] = None;
            let slots = &self.mesh_effects.slots;
            self.mesh_effects.pool.free(k, |j| slots[j].is_some());
            return Ok(());
        }
        if r == 0 {
            return Err(Stop::new(
                draw_site::MESH_EFFECT_RADIUS_ZERO,
                "a mesh effect of radius 0",
            ));
        }
        let t = e.ticks;
        let amplitude = |p: i32| {
            let a = 0x10000 - p / r;
            (if a < 0 { 0 } else { a }).wrapping_mul(m.amplitude)
        };
        let (p0, a0) = if t == 0 {
            (m.start, 0)
        } else {
            let p = (t - 1).wrapping_mul(m.speed).wrapping_add(m.start);
            (p, amplitude(p))
        };
        let p1 = t.wrapping_mul(m.speed).wrapping_add(m.start);
        let a1 = amplitude(p1);
        let k32 = f64::from(m.coefficient);
        let profile = |a: i32, u: i32| {
            let u = f64::from(u);
            let v = (f64::from(a) - u * u * k32) as i64 as i32;
            v.max(0)
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                if self.beside_a_model(x, y) {
                    continue;
                }
                let dx = i32::from((x as i16).wrapping_sub(e.cell.0)) << 8;
                let dy = i32::from((y as i16).wrapping_sub(e.cell.1)) << 8;
                let d = crate::groups::isqrt(
                    dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32
                ) as i32;
                let before = profile(a0, (p0 >> 8).wrapping_sub(d));
                let after = profile(a1, (p1 >> 8).wrapping_sub(d));
                let delta = (after.wrapping_sub(before) >> 16) as i16;
                let c = self.cell_at(x, y);
                let ht = &mut self.cells[c].physical_height;
                *ht = if m.flags & 1 != 0 {
                    ht.wrapping_sub(delta)
                } else {
                    ht.wrapping_add(delta)
                };
                self.move_objects_in_cell(x, y)?;
            }
        }
        if let Some(e) = &mut self.mesh_effects.slots[k] {
            e.ticks += 1;
        }
        Ok(())
    }

    fn beside_a_model(&self, x: i32, y: i32) -> bool {
        let (xm, ym) = (
            clamp(x - 1, self.width()),
            clamp(y - 1, self.height_cells()),
        );
        [(x, y), (xm, y), (x, ym), (xm, ym)]
            .iter()
            .any(|&(a, b)| self.cells[self.cell_at(a, b)].flags & 0x80 != 0)
    }

    fn move_objects_in_cell(&mut self, x: i32, y: i32) -> Result<(), Stop> {
        let mut o = self.cell_heads[self.cell_at(x, y)];
        while o >= 0 {
            let ob = &self.objects[o as usize];
            let next = ob.next_in_cell;
            if ob.flags & 0x200 == 0 && ob.flags & 0x10 != 0 {
                let mut z = i32::from(self.mesh_height(ob.x, ob.y)? as i16);
                if self.objects[o as usize].flags & 0x1800 != 0 && z < 0 {
                    z = 0;
                }
                self.objects[o as usize].z = z;
            }
            o = next;
        }
        Ok(())
    }
}
