use mandrake_common::FourCC;
use mandrake_render::billboard::TextureAtlas;
use mandrake_sim::particles::ParticleSystem;
use mandrake_tag::collection::INSTANCE_FLAG_VELOCITY_ALIGNED;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_view::billboard::{
    BILLBOARD_FLAG_MIRROR_Y, BILLBOARD_FLAG_PRERENDER_LAYER, BillboardInstance,
};
use mandrake_view::media::{ShadingSlot, build_tinting_shader};
use mandrake_view::model_surfaces::WorldFrame;
use mandrake_view::projectile::{EngineViewRows, InstanceRect, VelocityQuad};
use mandrake_view::view_region::{RegionView, ViewRegion};

use crate::media;
use crate::unit_anim::SequenceVisual;

const MAX_PARTICLES: usize = 0x100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleDefinition {
    pub flags: u32,
    pub collection_reference: FourCC,
    pub min_depth: i32,
    pub fade_start: i32,
    pub fade_end: i32,
    pub sequence: i16,
    pub count: i16,
    pub scale: i16,
    pub ambient_sound: Option<FourCC>,
    pub splash_group: Option<FourCC>,
    pub splash_tries: i16,
    pub splash_period: i16,
    pub velocity: [i32; 3],
    pub velocity_range: [i32; 3],
    pub grid: i16,
    pub top: i16,
    pub floor: i16,
    pub cell_shift: u32,
    pub first_bitmap: bool,
}

const FLAG_FADES: u32 = 4;

const TFL_GRID: i16 = 4;
const TFL_TOP: i16 = 0xc00;
const TFL_FLOOR: i16 = 0;
const TFL_CELL_SHIFT: u32 = 10;

impl ParticleDefinition {
    pub fn parse(d: &[u8]) -> Option<Self> {
        if d.len() < 0x66 {
            return None;
        }
        let i16_at = |o: usize| i16::from_be_bytes([d[o], d[o + 1]]);
        let i32_at = |o: usize| i32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        let grid = i16_at(0x5c);
        if grid <= 0 || grid & (grid - 1) != 0 {
            return None;
        }
        Some(Self {
            flags: i32_at(0) as u32,
            collection_reference: FourCC::new([d[4], d[5], d[6], d[7]]),
            min_depth: i32_at(0x08),
            fade_start: i32_at(0x0c),
            fade_end: i32_at(0x10),
            sequence: i16_at(0x14),
            count: i16_at(0x16).clamp(0, 0x100),
            scale: i16_at(0x1a),
            ambient_sound: Some(FourCC::new([d[0x54], d[0x55], d[0x56], d[0x57]]))
                .filter(|a| !matches!(u32::from_be_bytes(a.0), 0 | 0xffff_ffff)),
            splash_group: Some(FourCC::new([d[0x58], d[0x59], d[0x5a], d[0x5b]]))
                .filter(|g| !matches!(u32::from_be_bytes(g.0), 0 | 0xffff_ffff)),
            splash_tries: i16_at(0x62),
            splash_period: i16_at(0x64),
            velocity: [i32_at(0x1c), i32_at(0x20), i32_at(0x24)],
            velocity_range: [i32_at(0x28), i32_at(0x2c), i32_at(0x30)],
            grid,
            top: i16_at(0x5e),
            floor: i16_at(0x60),
            cell_shift: 9,
            first_bitmap: false,
        })
    }

    pub fn parse_tfl(d: &[u8]) -> Option<Self> {
        if d.len() < 0x5c {
            return None;
        }
        let mut padded = d.to_vec();
        padded.resize(padded.len().max(0x66), 0);
        padded[0x5c..0x5e].copy_from_slice(&TFL_GRID.to_be_bytes());
        let mut def = Self::parse(&padded)?;
        def.count = i16::from_be_bytes([d[0x16], d[0x17]]);
        def.top = TFL_TOP;
        def.floor = TFL_FLOOR;
        def.splash_tries = 0;
        def.splash_period = 0;
        def.cell_shift = TFL_CELL_SHIFT;
        def.first_bitmap = true;
        Some(def)
    }
}

pub fn observer_intensity(
    system: &ParticleSystem,
    point: (i32, i32),
    mesh_cells: (i32, i32),
) -> u16 {
    let mut system = system.clone();
    system.definition.flags &= !2;
    system
        .intensity([point.0, point.1, 0], mesh_cells)
        .unwrap_or(0)
}

pub fn splash_intensity(system: &ParticleSystem, point: (i32, i32), mesh_cells: (i32, i32)) -> u16 {
    if system.definition.flags & 2 != 0 {
        0
    } else {
        observer_intensity(system, point, mesh_cells)
    }
}

fn target_count(count: i16, intensity: u16) -> i16 {
    ((i32::from(count) * i32::from(intensity)) >> 16) as i16
}

pub(crate) fn ambient_volume(active: i16, count: i16) -> i16 {
    if count == 0 {
        return 0;
    }
    ((i32::from(active) << 8) / i32::from(count)) as i16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceShape {
    pub views: i16,
    pub frames: i16,
    pub ticks_per_frame: i16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Particle {
    used: bool,
    view: i16,
    frame: i16,
    phase: u16,
    position: [i32; 3],
    next: i16,
    velocity: [i32; 3],
}

#[derive(Clone, Debug)]
pub struct ParticleField {
    definition: ParticleDefinition,
    shape: SequenceShape,
    particles: Vec<Particle>,
    first_free: usize,
    end: usize,
    active: i16,
    side: i32,
    cells: Vec<i16>,
    random: LocalRandom,
}

fn drawn(random: &mut LocalRandom, range: i32) -> i32 {
    if range > 0 {
        (i32::from(random.next_u16()).wrapping_mul(range)) >> 16
    } else {
        0
    }
}

impl ParticleField {
    pub fn for_map(
        definition: ParticleDefinition,
        shape: SequenceShape,
        intensity: u16,
        seed: u32,
    ) -> Self {
        let mut random = LocalRandom::new(seed);
        let mut field = Self::for_map_on(definition, shape, intensity, &mut random);
        field.random = random;
        field
    }

    pub fn for_map_on(
        definition: ParticleDefinition,
        shape: SequenceShape,
        intensity: u16,
        random: &mut LocalRandom,
    ) -> Self {
        let n = definition.grid as usize;
        let mut field = Self {
            side: i32::from(definition.grid) << definition.cell_shift,
            definition,
            shape,
            particles: vec![Particle::default(); MAX_PARTICLES],
            first_free: 0,
            end: 0,
            active: 0,
            cells: vec![-1; n * n],
            random: LocalRandom::new(0),
        };
        std::mem::swap(&mut field.random, random);
        for _ in 0..target_count(field.definition.count, intensity) {
            field.spawn();
        }
        std::mem::swap(&mut field.random, random);
        field
    }

    pub fn random_mut(&mut self) -> &mut LocalRandom {
        &mut self.random
    }

    fn cell_of(&self, x: i32, y: i32) -> usize {
        let shift = self.definition.cell_shift;
        ((y >> shift) * i32::from(self.definition.grid) + (x >> shift)) as usize
    }

    fn new_datum(&mut self) -> Option<usize> {
        let i = (self.first_free..MAX_PARTICLES).find(|&i| !self.particles[i].used)?;
        self.first_free = i + 1;
        self.end = self.end.max(i + 1);
        self.active += 1;
        self.particles[i] = Particle {
            used: true,
            ..Particle::default()
        };
        Some(i)
    }

    fn delete_datum(&mut self, i: usize) {
        self.particles[i].used = false;
        self.first_free = self.first_free.min(i);
        if i + 1 == self.end {
            while self.end > 0 {
                self.end -= 1;
                if self.end == 0 || self.particles[self.end - 1].used {
                    break;
                }
            }
        }
        self.active -= 1;
    }

    fn spawn(&mut self) {
        let Some(i) = self.new_datum() else {
            return;
        };
        let def = &self.definition;
        let r = &mut self.random;
        let x = drawn(r, self.side);
        let y = drawn(r, self.side);
        let velocity = [0, 1, 2].map(|k| {
            ((def.velocity[k] >> 8) + drawn(r, def.velocity_range[k] >> 8)).wrapping_shl(8)
        });
        let view = drawn(r, i32::from(self.shape.views)) as i16;
        let frame = drawn(r, i32::from(self.shape.frames)) as i16;
        let p = &mut self.particles[i];
        p.position = [x, y, i32::from(def.top)];
        p.velocity = velocity;
        p.view = view;
        p.frame = frame;
    }

    pub fn ambient_sound(&self) -> Option<FourCC> {
        self.definition.ambient_sound
    }

    pub fn ambient_volume(&self) -> i16 {
        ambient_volume(self.active, self.definition.count)
    }

    pub fn update(&mut self, ticks: i32, intensity: u16, wind: Option<[i32; 3]>) {
        let target = target_count(self.definition.count, intensity);
        if self.active < target {
            for _ in 0..target - self.active {
                self.spawn();
            }
        }
        let ticks_per_frame = i32::from(self.shape.ticks_per_frame).max(1);
        let frames = i32::from(self.shape.frames);
        let wind = wind.unwrap_or([0; 3]);
        self.cells.fill(-1);
        let mut i = 0;
        while i < self.end {
            if !self.particles[i].used {
                i += 1;
                continue;
            }
            let side = self.side;
            let (floor, top) = (self.definition.floor, self.definition.top);
            let p = &mut self.particles[i];
            if frames > 1 {
                let phase = (i32::from(p.phase) + ticks) & 0xffff;
                p.phase = (phase % ticks_per_frame) as u16;
                p.frame = ((phase / ticks_per_frame + i32::from(p.frame)) % frames) as i16;
            }
            let step = |k: usize, p: &Particle| {
                (p.velocity[k].wrapping_add(wind[k]).wrapping_mul(ticks) >> 16) + p.position[k]
            };
            let x = step(0, p).wrapping_add(side) & (side - 1);
            let y = step(1, p).wrapping_add(side) & (side - 1);
            let mut z = step(2, p);
            let mut delete = false;
            if z < i32::from(floor) {
                z = i32::from(top);
                delete = target < self.active;
            }
            p.position = [x, y, z];
            if delete {
                self.delete_datum(i);
            } else {
                let cell = self.cell_of(x, y);
                let p = &mut self.particles[i];
                p.next = self.cells[cell];
                self.cells[cell] = i as i16;
            }
            i += 1;
        }
    }

    pub fn surfaces(
        &self,
        view: &RegionView,
        region: &ViewRegion,
        sprites: &ParticleSprites,
        fraction: i32,
        wind: Option<[i32; 3]>,
    ) -> Vec<ParticleSurface> {
        let def = &self.definition;
        let n = i32::from(def.grid);
        let mut out = Vec::new();
        for row in region.first_row..region.last_row {
            let Some((first, last)) = region.span(row) else {
                continue;
            };
            for col in i32::from(first)..i32::from(last) {
                let mut next = self.cells[((row % n) * n + col % n) as usize];
                while next >= 0 {
                    let p = &self.particles[next as usize];
                    next = p.next;
                    let moved = [0, 1, 2].map(|k| {
                        let v = p.velocity[k].wrapping_add(wind.map_or(0, |w| w[k]));
                        p.position[k] + ((i64::from(v) * i64::from(fraction)) >> 32) as i32
                    });
                    let world = [
                        (((col / n) * n) << def.cell_shift) + moved[0],
                        (((row / n) * n) << def.cell_shift) + moved[1],
                        moved[2],
                    ];
                    if let Some(s) = particle_surface(def, p, world, view, sprites, wind) {
                        out.push(s);
                    }
                }
            }
        }
        out
    }

    pub fn load_state(&mut self, text: &str) -> Result<usize, String> {
        self.particles.fill(Particle::default());
        self.cells.fill(-1);
        let (mut count, mut end) = (0, 0);
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let v: Vec<i32> = line
                .split_whitespace()
                .map(str::parse)
                .collect::<Result<_, _>>()
                .map_err(|e| format!("{line:?}: {e}"))?;
            let [index, view, frame, phase, x, y, z, vx, vy, vz] = v[..] else {
                return Err(format!("{line:?}: not 10 fields"));
            };
            let i = usize::try_from(index)
                .ok()
                .filter(|&i| i < MAX_PARTICLES)
                .ok_or_else(|| format!("{line:?}: index"))?;
            if !(0..self.side).contains(&x) || !(0..self.side).contains(&y) {
                return Err(format!("{line:?}: position out of bounds"));
            }
            let cell = self.cell_of(x, y);
            let Some(&head) = self.cells.get(cell) else {
                return Err(format!("{line:?}: position out of bounds"));
            };
            self.particles[i] = Particle {
                used: true,
                view: view as i16,
                frame: frame as i16,
                phase: phase as u16,
                position: [x, y, z],
                next: head,
                velocity: [vx, vy, vz],
            };
            self.cells[cell] = i as i16;
            count += 1;
            end = end.max(i + 1);
        }
        self.active = count as i16;
        self.end = end;
        self.first_free = 0;
        Ok(count)
    }

    #[cfg(test)]
    pub(crate) fn active(&self) -> i16 {
        self.active
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleShape {
    Streak([i16; 3]),
    Rectangle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleSurface {
    pub world: [i32; 3],
    pub view: i16,
    pub frame: i16,
    pub transparency: u16,
    pub shape: ParticleShape,
}

fn particle_surface(
    def: &ParticleDefinition,
    p: &Particle,
    world: [i32; 3],
    view: &RegionView,
    sprites: &ParticleSprites,
    wind: Option<[i32; 3]>,
) -> Option<ParticleSurface> {
    let [right, up, depth] = view.view_coords(world.map(|c| c as f32));
    let fades = def.flags & FLAG_FADES != 0;
    if depth >= -def.min_depth || (fades && depth <= -def.fade_end) {
        return None;
    }
    let (view_index, frame) = if def.first_bitmap {
        (0, 0)
    } else {
        (p.view, p.frame)
    };
    let bitmap = sprites.bitmap(view_index, frame)?;
    let scale = i32::from(def.scale);
    let scaled = |word: i16| (i32::from(word) * scale) >> 8;
    let k = view.focal / depth as f32;
    let [cx, cy] = view.center.map(|c| c as f32);
    let x0 = (cx - (right + scaled(bitmap.rect.left)) as f32 * k) as i32;
    let x1 = (cx - (right + scaled(bitmap.rect.right)) as f32 * k) as i32;
    let y0 = ((up + scaled(bitmap.rect.top)) as f32 * k + cy) as i32;
    let y1 = ((up + scaled(bitmap.rect.bottom)) as f32 * k + cy) as i32;
    if x0 <= 0 || x1 >= view.size[0] || y0 <= 0 || y1 >= view.size[1] || x0 >= x1 || y0 >= y1 {
        return None;
    }
    let mut transparency = 0;
    if fades {
        let d = i32::from(depth.wrapping_neg() as i16);
        if d > def.fade_start {
            let f = (d - def.fade_start) as f32 / (def.fade_end - def.fade_start) as f32;
            transparency = (f * 65536.0) as i64 as u16;
        }
    }
    let shape = if bitmap.flags & INSTANCE_FLAG_VELOCITY_ALIGNED == 0 {
        ParticleShape::Rectangle
    } else {
        let word = |v: i32| (v >> 16) as i16;
        ParticleShape::Streak(match wind {
            None => p.velocity.map(word),
            Some(w) => [
                word(w[0]),
                word(w[1]),
                word(p.velocity[2].wrapping_add(w[2])),
            ],
        })
    };
    Some(ParticleSurface {
        world,
        view: view_index,
        frame,
        transparency,
        shape,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleBitmap {
    atlas_index: usize,
    world_size: [f32; 2],
    rect: InstanceRect,
    flags: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ParticleSprites {
    bitmaps: Vec<Option<ParticleBitmap>>,
    frames: usize,
    scale: i32,
    core: media::CoreRef,
}

impl ParticleSprites {
    pub fn new(
        visual: &SequenceVisual,
        flags: impl Fn(usize, usize) -> u32,
        scale: i16,
        core: media::CoreRef,
        anchor: impl Fn(usize) -> [f32; 2],
    ) -> Self {
        Self::with_pixel_scale(visual, flags, scale, core, anchor, 1.0)
    }

    pub fn with_pixel_scale(
        visual: &SequenceVisual,
        flags: impl Fn(usize, usize) -> u32,
        scale: i16,
        core: media::CoreRef,
        anchor: impl Fn(usize) -> [f32; 2],
        pixel_scale: f32,
    ) -> Self {
        let frames = visual.frames.len();
        let views = usize::from(visual.sequence.num_views).max(1);
        let mut bitmaps = Vec::with_capacity(views * frames);
        for view in 0..views {
            #[allow(clippy::needless_range_loop)]
            for frame in 0..frames {
                let sprite = visual.frames[frame]
                    .views
                    .get(view)
                    .and_then(Option::as_ref);
                bitmaps.push(sprite.map(|s| {
                    let world_size = s.world_size.map(|v| v * pixel_scale);
                    ParticleBitmap {
                        atlas_index: s.atlas_index,
                        world_size,
                        rect: InstanceRect::from_sprite(world_size, anchor(s.atlas_index)),
                        flags: flags(view, frame),
                    }
                }));
            }
        }
        Self {
            bitmaps,
            frames,
            scale: i32::from(scale),
            core,
        }
    }

    pub fn shade(&self, global_tint: media::CoreTint) -> [u8; 8] {
        let tint = self.core.resolve(global_tint);
        let slot = ShadingSlot {
            kind: -1,
            ..ShadingSlot::default()
        };
        build_tinting_shader(slot, tint.fraction, tint.color, slot)
    }

    fn bitmap(&self, view: i16, frame: i16) -> Option<&ParticleBitmap> {
        let (v, f) = (usize::try_from(view).ok()?, usize::try_from(frame).ok()?);
        self.bitmaps.get(v * self.frames + f)?.as_ref()
    }

    pub fn instances(
        &self,
        surfaces: &[ParticleSurface],
        atlas: &TextureAtlas,
        frame: WorldFrame,
        axes: ([f32; 3], [f32; 3], [f32; 3]),
        shade: [u8; 8],
    ) -> Vec<BillboardInstance> {
        let rows = EngineViewRows::from_camera_axes(axes.0, axes.1, axes.2);
        let k = self.scale as f32 / 256.0;
        let mut out = Vec::with_capacity(surfaces.len());
        for s in surfaces {
            let Some(bitmap) = self.bitmap(s.view, s.frame) else {
                continue;
            };
            let position = frame.from_engine(s.world.map(|c| c as f32));
            let (size, flip) = match s.shape {
                ParticleShape::Streak(_) => (bitmap.world_size, false),
                ParticleShape::Rectangle => (
                    [bitmap.world_size[0] * k, bitmap.world_size[1] * k],
                    bitmap.flags & 1 != 0,
                ),
            };
            let Some(base) =
                atlas.create_instance_with_world_size(bitmap.atlas_index, position, size, flip)
            else {
                continue;
            };
            let mut instance = match s.shape {
                ParticleShape::Streak(velocity) => {
                    VelocityQuad::new(&rows, velocity, bitmap.rect, self.scale, false)
                        .instance(base, false)
                }
                ParticleShape::Rectangle => {
                    let mut i = base;
                    if bitmap.flags & 2 != 0 {
                        i.flags |= BILLBOARD_FLAG_MIRROR_Y;
                    }
                    i
                }
            };
            instance = instance.with_shade(shade);
            instance.flags |= BILLBOARD_FLAG_PRERENDER_LAYER;
            if s.transparency != 0 {
                instance.tint[3] *= mandrake_view::media::surface_opacity(s.transparency);
            }
            out.push(instance);
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct LocalWind {
    period: i32,
    period_range: i32,
    rate: i32,
    base: [i32; 3],
    range: [i32; 3],
    countdown: i32,
    velocity: [i32; 3],
    target: [i32; 3],
    random: LocalRandom,
}

impl LocalWind {
    pub fn new_on(d: &[u8], random: &mut LocalRandom) -> Option<Self> {
        let i32_at = |o: usize| Some(i32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?));
        let mut w = Self {
            period: i32_at(0x04)?,
            period_range: i32_at(0x08)?,
            rate: i32_at(0x0c)?,
            base: [i32_at(0x10)?, i32_at(0x14)?, i32_at(0x18)?],
            range: [i32_at(0x1c)?, i32_at(0x20)?, i32_at(0x24)?],
            countdown: 0,
            velocity: [0; 3],
            target: [0; 3],
            random: LocalRandom::new(0),
        };
        std::mem::swap(&mut w.random, random);
        w.change();
        std::mem::swap(&mut w.random, random);
        w.velocity = w.target;
        Some(w)
    }

    fn change(&mut self) {
        self.countdown = if self.period_range > 0 {
            self.period + (i32::from(self.random.next_u16()).wrapping_mul(self.period_range) >> 16)
        } else {
            self.period
        };
        for k in 0..3 {
            let d = if (self.range[k] & !0xff) > 0 {
                i32::from(self.random.next_u16()).wrapping_mul(self.range[k] >> 8) >> 16
            } else {
                0
            };
            self.target[k] = (self.base[k] & !0xff).wrapping_add(d << 8);
        }
    }

    pub fn update(&mut self, ticks: i32) {
        self.countdown -= ticks;
        let step = self.rate.wrapping_mul(ticks);
        if self.countdown < 1 {
            self.change();
        }
        for k in 0..3 {
            let (v, t) = (self.velocity[k], self.target[k]);
            self.velocity[k] = if t < v {
                (v - step).max(t)
            } else if v < t {
                (v + step).min(t)
            } else {
                v
            };
        }
    }

    pub fn velocity(&self) -> [i32; 3] {
        self.velocity
    }

    pub fn random_mut(&mut self) -> &mut LocalRandom {
        &mut self.random
    }
}

pub fn definition_of_mesh(
    mesh_data: &[u8],
    tag: impl Fn(&[u8; 4], FourCC) -> Option<Vec<u8>>,
) -> Option<ParticleDefinition> {
    let id: [u8; 4] = mesh_data.get(0x50..0x54)?.try_into().ok()?;
    if matches!(u32::from_be_bytes(id), 0 | 0xffff_ffff) {
        return None;
    }
    ParticleDefinition::parse(&tag(b"part", FourCC::new(id))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rain() -> ParticleDefinition {
        ParticleDefinition {
            flags: 5,
            collection_reference: FourCC::new(*b"rain"),
            min_depth: 2048,
            fade_start: 14336,
            fade_end: 18432,
            sequence: 0,
            count: 80,
            scale: 192,
            ambient_sound: Some(FourCC::new(*b"amra")),
            splash_group: Some(FourCC::new(*b"fosp")),
            splash_tries: 1,
            splash_period: 8,
            velocity: [-7 << 16, -7 << 16, -65 << 16],
            velocity_range: [0xe0001, 0xe0001, 0x140001],
            grid: 8,
            top: 2048,
            floor: -512,
            cell_shift: 9,
            first_bitmap: false,
        }
    }

    const TFL_RAIN: &str = "00000004 6d617261 00000400 00001c00 00002400 000000c8 000300c0 fff90000 \
         fff90000 ffbf0000 000e0001 000e0001 00140001 01000100 01000100 0a8c0a8d 00000000 01c2012d \
         0a8c0a8d 99996667 03840000 616d7261 6d617772";

    fn tfl_rain_bytes() -> Vec<u8> {
        let hex: String = TFL_RAIN.chars().filter(|c| !c.is_whitespace()).collect();
        let mut b: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        b.resize(0x80, 0);
        b
    }

    #[test]
    fn a_tfl_part_tag_keeps_myth_ii_s_fields_and_tfl_s_square() {
        let d = ParticleDefinition::parse_tfl(&tfl_rain_bytes()).unwrap();
        assert_eq!(d.collection_reference, FourCC::new(*b"mara"));
        assert_eq!(
            (d.flags, d.min_depth, d.fade_start, d.fade_end),
            (4, 0x400, 0x1c00, 0x2400)
        );
        assert_eq!((d.sequence, d.count, d.scale), (0, 200, 0xc0));
        assert_eq!(d.velocity, [-7 << 16, -7 << 16, -65 << 16]);
        assert_eq!(d.velocity_range, [0xe0001, 0xe0001, 0x140001]);
        assert_eq!(d.ambient_sound, Some(FourCC::new(*b"amra")));
        assert_eq!(d.splash_group, Some(FourCC::new(*b"mawr")));
        assert_eq!((d.grid, d.top, d.floor, d.cell_shift), (4, 0xc00, 0, 10));
        assert!(d.first_bitmap);
    }

    #[test]
    fn a_local_wind_moves_toward_its_targets() {
        let tag: Vec<u8> = [
            0,
            0x1e,
            0x5b,
            0x2666,
            -40 << 16,
            -40 << 16,
            0,
            0x50_0001,
            0x50_0001,
            0,
        ]
        .iter()
        .flat_map(|v: &i32| v.to_be_bytes())
        .collect();
        let mut w = LocalWind::new_on(&tag, &mut LocalRandom::new(0)).unwrap();
        assert_eq!(w.velocity(), w.target);
        assert!((30..=30 + 0x5b).contains(&w.countdown));
        for _ in 0..2000 {
            let before = w.velocity();
            w.update(1);
            for (now, before) in w.velocity().iter().zip(before).take(2) {
                assert!((-40 << 16..=40 << 16).contains(now));
                assert!((now - before).abs() <= 0x2666);
            }
            assert_eq!(w.velocity()[2], 0);
        }
    }

    #[test]
    fn tfl_particles_and_wind_draw_from_a_shared_generator() {
        let d = ParticleDefinition::parse_tfl(&tfl_rain_bytes()).unwrap();
        let shape = SequenceShape {
            views: 1,
            frames: 0,
            ticks_per_frame: 1,
        };
        let own = ParticleField::for_map(d.clone(), shape, 0xffff, 9);
        let mut shared = LocalRandom::new(9);
        let f = ParticleField::for_map_on(d.clone(), shape, 0xffff, &mut shared);
        assert_eq!((shared, f.active()), (own.random, own.active()));
        assert_eq!(f.random, LocalRandom::new(0));
        let mut f = ParticleField::for_map_on(d, shape, 0, &mut shared);
        let before = shared;
        std::mem::swap(f.random_mut(), &mut shared);
        for _ in 0..30 {
            f.update(1, 0xffff, None);
        }
        std::mem::swap(f.random_mut(), &mut shared);
        assert_ne!(shared, before);
        assert_eq!(f.random, LocalRandom::new(0));
        let tag: Vec<u8> = [
            0,
            30,
            0x5b,
            0x2666,
            -40 << 16,
            -40 << 16,
            0,
            0x50_0001,
            0x50_0001,
            0,
        ]
        .iter()
        .flat_map(|v: &i32| v.to_be_bytes())
        .collect();
        let mut r = LocalRandom::new(4);
        let w = LocalWind::new_on(&tag, &mut r).unwrap();
        let mut expected = LocalRandom::new(4);
        for _ in 0..3 {
            expected.next_u16();
        }
        assert_eq!((r, w.random), (expected, LocalRandom::new(0)));
    }

    #[test]
    fn tfl_particles_wrap_in_4096_and_fall_from_0xc00() {
        let d = ParticleDefinition::parse_tfl(&tfl_rain_bytes()).unwrap();
        let shape = SequenceShape {
            views: 1,
            frames: 0,
            ticks_per_frame: 1,
        };
        let mut f = ParticleField::for_map(d, shape, 0xffff, 0);
        assert_eq!(f.active(), 199);
        assert!(f.particles.iter().filter(|p| p.used).all(|p| {
            p.position[2] == 0xc00 && (0..0x1000).contains(&p.position[0]) && p.frame == 0
        }));
        for _ in 0..100 {
            f.update(1, 0xffff, None);
            assert!(f.particles.iter().filter(|p| p.used).all(|p| {
                (0..0x1000).contains(&p.position[0])
                    && (0..0x1000).contains(&p.position[1])
                    && (0..=0xc00).contains(&p.position[2])
            }));
        }
        let listed: usize = f
            .cells
            .iter()
            .map(|&c| {
                let (mut n, mut i) = (0, c);
                while i >= 0 {
                    n += 1;
                    i = f.particles[i as usize].next;
                }
                n
            })
            .sum();
        assert_eq!((f.cells.len(), listed), (16, f.active() as usize));
    }

    const SHAPE: SequenceShape = SequenceShape {
        views: 1,
        frames: 2,
        ticks_per_frame: 4,
    };

    #[test]
    fn update_moves_the_engines_particles_as_the_engine_does() {
        let text = include_str!("testdata/particles_cr01_t300.txt");
        let mut ticks: Vec<(i32, String)> = Vec::new();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            if let Some(t) = line.strip_prefix("T ") {
                ticks.push((t.parse().unwrap(), String::new()));
            } else {
                let last = ticks.last_mut().unwrap();
                last.1.push_str(line);
                last.1.push('\n');
            }
        }
        assert_eq!(ticks.len(), 7);
        let dump = |f: &ParticleField| -> String {
            let used = f.particles.iter().enumerate().filter(|(_, p)| p.used);
            used.map(|(i, p)| {
                let [x, y, z] = p.position;
                let [vx, vy, vz] = p.velocity;
                let (v, fr, ph) = (p.view, p.frame, p.phase);
                format!("{i} {v} {fr} {ph} {x} {y} {z} {vx} {vy} {vz}\n")
            })
            .collect()
        };
        let mut f = ParticleField::for_map(rain(), SHAPE, 0, 0);
        f.load_state(&ticks[0].1).unwrap();
        assert_eq!(f.active(), 39);
        assert_eq!(target_count(80, ((0xffff * 128) >> 8) as u16), 39);
        for (tick, want) in &ticks[1..] {
            f.update(1, ((0xffff * 128) >> 8) as u16, Some([-3840, -899840, 0]));
            assert_eq!(&dump(&f), want, "tick {tick}");
        }
    }

    #[test]
    fn the_rain_tag_parses() {
        let mut d = vec![0u8; 0x84];
        d[0..4].copy_from_slice(&5u32.to_be_bytes());
        d[4..8].copy_from_slice(b"rain");
        d[8..12].copy_from_slice(&2048i32.to_be_bytes());
        d[0x0c..0x10].copy_from_slice(&14336i32.to_be_bytes());
        d[0x10..0x14].copy_from_slice(&18432i32.to_be_bytes());
        d[0x16..0x18].copy_from_slice(&80i16.to_be_bytes());
        d[0x1a..0x1c].copy_from_slice(&192i16.to_be_bytes());
        d[0x1c..0x20].copy_from_slice(&(-7i32 << 16).to_be_bytes());
        d[0x20..0x24].copy_from_slice(&(-7i32 << 16).to_be_bytes());
        d[0x24..0x28].copy_from_slice(&(-65i32 << 16).to_be_bytes());
        d[0x28..0x2c].copy_from_slice(&0xe0001i32.to_be_bytes());
        d[0x2c..0x30].copy_from_slice(&0xe0001i32.to_be_bytes());
        d[0x30..0x34].copy_from_slice(&0x140001i32.to_be_bytes());
        d[0x54..0x58].copy_from_slice(b"amra");
        d[0x58..0x5c].copy_from_slice(b"fosp");
        d[0x5c..0x5e].copy_from_slice(&8i16.to_be_bytes());
        d[0x5e..0x60].copy_from_slice(&2048i16.to_be_bytes());
        d[0x60..0x62].copy_from_slice(&(-512i16).to_be_bytes());
        d[0x62..0x64].copy_from_slice(&1i16.to_be_bytes());
        d[0x64..0x66].copy_from_slice(&8i16.to_be_bytes());
        assert_eq!(ParticleDefinition::parse(&d), Some(rain()));
        assert_eq!(ParticleDefinition::parse(&d[..0x60]), None);
        d[0x5c..0x5e].copy_from_slice(&6i16.to_be_bytes());
        assert_eq!(ParticleDefinition::parse(&d), None);
    }

    #[test]
    fn a_new_particle_takes_its_draws_in_the_engines_order() {
        let f = ParticleField::for_map(rain(), SHAPE, 0xffff, 7);
        let mut r = LocalRandom::new(7);
        let d = |r: &mut LocalRandom, range: i32| (i32::from(r.next_u16()) * range) >> 16;
        let x = d(&mut r, 8 << 9);
        let y = d(&mut r, 8 << 9);
        let vx = ((-7 << 8) + d(&mut r, 0xe00)) << 8;
        let vy = ((-7 << 8) + d(&mut r, 0xe00)) << 8;
        let vz = ((-65 << 8) + d(&mut r, 0x1400)) << 8;
        let view = d(&mut r, 1) as i16;
        let frame = d(&mut r, 2) as i16;
        let p = f.particles[0];
        assert_eq!(p.position, [x, y, 2048]);
        assert_eq!(p.velocity, [vx, vy, vz]);
        assert_eq!((p.view, p.frame), (view, frame));
    }

    #[test]
    fn the_count_follows_the_intensity_up_at_once_and_down_at_the_floor() {
        let mut f = ParticleField::for_map(rain(), SHAPE, 0xffff, 7);
        assert_eq!(f.active(), 79);
        let lower = ((0xffff * 230) >> 8) as u16;
        f.update(1, lower, None);
        assert_eq!(f.active(), 79);
        for _ in 0..200 {
            f.update(1, lower, None);
        }
        assert_eq!(f.active(), 71);
        f.update(1, 0xffff, None);
        assert_eq!(f.active(), 79);
    }

    #[test]
    fn a_particle_moves_and_wraps_in_the_square() {
        let mut f = ParticleField::for_map(rain(), SHAPE, 0, 7);
        assert_eq!(f.active(), 0);
        f.spawn();
        let side = 8 << 9;
        f.particles[0].position = [1, side - 1, 100];
        f.particles[0].velocity = [-2 << 16, 3 << 16, -5 << 16];
        f.particles[0].frame = 0;
        f.update(1, 0, None);
        assert_eq!(f.particles[0].position, [side - 1, 2, 95]);
        f.update(2, 0, Some([1 << 16, 0, -(5 << 16)]));
        assert_eq!(f.particles[0].position, [side - 3, 8, 75]);
        assert_eq!(f.particles[0].velocity, [-2 << 16, 3 << 16, -5 << 16]);
        f.particles[0].position[2] = -510;
        f.update(1, 0, None);
        assert_eq!(f.active(), 0);
        assert_eq!(f.cells.iter().filter(|&&c| c >= 0).count(), 0);
    }

    #[test]
    fn the_frame_advances_every_ticks_per_frame() {
        let mut f = ParticleField::for_map(rain(), SHAPE, 0, 7);
        f.spawn();
        f.particles[0].frame = 0;
        let frames: Vec<i16> = (0..9)
            .map(|_| {
                f.update(1, 0x400, None);
                f.particles[0].frame
            })
            .collect();
        assert_eq!(frames, [0, 0, 0, 1, 1, 1, 1, 0, 0]);
    }

    #[test]
    fn the_datum_slots_are_reused_lowest_first() {
        let mut f = ParticleField::for_map(rain(), SHAPE, 0, 7);
        for _ in 0..3 {
            f.spawn();
        }
        assert_eq!(f.end, 3);
        f.delete_datum(2);
        assert_eq!(f.end, 2);
        f.delete_datum(0);
        assert_eq!((f.first_free, f.end, f.active()), (0, 2, 1));
        f.spawn();
        assert!(f.particles[0].used);
        assert_eq!((f.first_free, f.end, f.active()), (1, 2, 2));
        f.delete_datum(1);
        f.delete_datum(0);
        assert_eq!((f.end, f.active()), (0, 0));
    }

    fn view() -> RegionView {
        RegionView {
            eye: [0.0, 0.0, 0.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
            extents: [0.5, -1.0, -0.5, 1.0],
            focal: 640.0,
            center: [640, 320],
            size: [1280, 640],
        }
    }

    fn sprites(flags: u32) -> ParticleSprites {
        ParticleSprites {
            bitmaps: vec![Some(ParticleBitmap {
                atlas_index: 0,
                world_size: [1.0, 0.25],
                rect: InstanceRect {
                    top: 64,
                    left: -256,
                    right: 256,
                    bottom: -64,
                },
                flags,
            })],
            frames: 1,
            scale: 256,
            core: media::CoreRef::default(),
        }
    }

    #[test]
    fn the_rain_core_tints_slot_0() {
        let mut s = sprites(INSTANCE_FLAG_VELOCITY_ALIGNED);
        s.core = media::CoreRef {
            tint: media::CoreTint {
                fraction: 0xc0,
                color: [10086, 16934, 17694],
            },
            global: false,
        };
        assert_eq!(
            s.shade(media::CoreTint::default()),
            [63, 0, 0, 0, 29, 49, 51, 0]
        );
    }

    fn at(position: [i32; 3], velocity: [i32; 3]) -> Particle {
        Particle {
            used: true,
            position,
            velocity,
            next: -1,
            ..Particle::default()
        }
    }

    #[test]
    fn nearer_than_the_minimum_or_past_the_fade_is_not_drawn() {
        let s = sprites(INSTANCE_FLAG_VELOCITY_ALIGNED);
        let draw = |y: i32| {
            particle_surface(
                &rain(),
                &at([0, y, 0], [0; 3]),
                [0, y, 0],
                &view(),
                &s,
                None,
            )
        };
        assert!(draw(2048).is_none(), "the depth must be under -2048");
        assert!(draw(2049).is_some());
        assert!(draw(18431).is_some());
        assert!(draw(18432).is_none());
    }

    #[test]
    fn the_fade_runs_from_its_start_to_its_end() {
        let s = sprites(INSTANCE_FLAG_VELOCITY_ALIGNED);
        let t = |y: i32| {
            particle_surface(
                &rain(),
                &at([0, y, 0], [0; 3]),
                [0, y, 0],
                &view(),
                &s,
                None,
            )
            .map(|s| s.transparency)
        };
        assert_eq!(t(14336), Some(0));
        assert_eq!(t(14336 + 2048), Some(0x8000));
        assert_eq!(t(14336 + 1024), Some(0x4000));
    }

    #[test]
    fn a_rectangle_not_wholly_on_screen_is_not_drawn() {
        let s = sprites(INSTANCE_FLAG_VELOCITY_ALIGNED);
        let draw = |x: i32, z: i32| {
            particle_surface(
                &rain(),
                &at([0; 3], [0; 3]),
                [x, 6400, z],
                &view(),
                &s,
                None,
            )
        };
        assert!(draw(0, 0).is_some());
        assert!(draw(-6190, 0).is_some());
        assert!(draw(-6220, 0).is_none());
        assert!(draw(6190, 0).is_some());
        assert!(draw(6220, 0).is_none());
        assert!(draw(0, 3100).is_some());
        assert!(draw(0, 3160).is_none());
        assert!(draw(0, -3100).is_some());
        assert!(draw(0, -3160).is_none());
    }

    #[test]
    fn the_streak_takes_the_winds_direction_in_x_and_y() {
        let s = sprites(INSTANCE_FLAG_VELOCITY_ALIGNED);
        let p = at([0; 3], [3 << 16, 4 << 16, -50 << 16]);
        let shape = |wind| {
            particle_surface(&rain(), &p, [0, 6400, 0], &view(), &s, wind)
                .unwrap()
                .shape
        };
        assert_eq!(shape(None), ParticleShape::Streak([3, 4, -50]));
        assert_eq!(
            shape(Some([1 << 16, -(2 << 16), -(5 << 16)])),
            ParticleShape::Streak([1, -2, -55])
        );
    }

    #[test]
    fn a_plain_bitmap_is_a_rectangle() {
        let p = at([0; 3], [0; 3]);
        let s = particle_surface(&rain(), &p, [0, 6400, 0], &view(), &sprites(0), None).unwrap();
        assert_eq!(s.shape, ParticleShape::Rectangle);
    }

    #[test]
    fn the_square_repeats_over_the_view_region() {
        let mut f = ParticleField::for_map(rain(), SHAPE, 0, 7);
        f.spawn();
        f.particles[0].position = [100, 200, 100];
        f.particles[0].velocity = [2 << 16, 0, 0];
        f.update(1, 0x400, None);
        assert_eq!(f.particles[0].position, [102, 200, 100]);
        let region = ViewRegion {
            first_row: 8,
            last_row: 9,
            spans: vec![(0, 9)],
            built: vec![(0, 9)],
            rect_last_row: 9,
        };
        let s = f.surfaces(
            &view_far(),
            &region,
            &sprites(INSTANCE_FLAG_VELOCITY_ALIGNED),
            0,
            None,
        );
        let worlds: Vec<_> = s.iter().map(|s| s.world).collect();
        assert_eq!(worlds, [[102, 4296, 100], [4198, 4296, 100]]);
        let s = f.surfaces(
            &view_far(),
            &region,
            &sprites(INSTANCE_FLAG_VELOCITY_ALIGNED),
            0x8000,
            None,
        );
        assert_eq!(s[0].world, [103, 4296, 100]);
    }

    fn view_far() -> RegionView {
        RegionView {
            eye: [0.0, -2048.0, 0.0],
            ..view()
        }
    }

    #[test]
    fn the_observer_intensity_ignores_flag_2() {
        let def = mandrake_sim::particles::ParticleSystemDefinition {
            flags: 2,
            scale_x: [0x100, 0x100],
            scale_y: [0x100, 0x100],
            ..Default::default()
        };
        let system = ParticleSystem {
            definition: def,
            state: 0,
            timer: 0,
            transitioning: false,
            transition_duration: 0,
            previous_intensity: 0,
            intensity: 0xffff,
            multiplier: 0x100,
        };
        assert_eq!(system.intensity([0; 3], (64, 64)), Ok(0));
        assert_eq!(observer_intensity(&system, (0, 0), (64, 64)), 0xffff);
    }

    #[test]
    fn load_state_rejects_out_of_bounds_coordinates() {
        let mut field = ParticleField::for_map(rain(), SHAPE, 0, 1);
        assert!(field.load_state("0 0 0 0 -1 0 0 0 0 0").is_err());
        assert!(field.load_state("0 0 0 0 0 -10 0 0 0 0").is_err());
        assert!(field.load_state("0 0 0 0 999999 0 0 0 0 0").is_err());
        assert_eq!(field.load_state("0 0 0 0 10 20 30 0 0 0").unwrap(), 1);
    }
}
