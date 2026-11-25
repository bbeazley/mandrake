use mandrake_common::FourCC;
use mandrake_tag::object::{
    OBJECT_FLAG_FOLLOWS_GROUND, OBJECT_FLAG_NO_OBJECT_COLLISIONS, ObjectPhysics,
};
use mandrake_tag::projectile::ProjectileDefinition;
use mandrake_tag::{
    MeshHeightCorner, MeshHeightMode, MeshTag, SequenceWorldBounds, get_mesh_height,
};

use crate::combat::MINIMUM_GAMEPLAY_VERSION;
use crate::draw_site;
use crate::engine_move::{angle_index, arctangent, signed_angular_difference, trig_tables};
use crate::engine_rng::Unmodelled;

pub const MAP_EDGE: i32 = 0x3e0;
pub const COLLISION_CHANCE_VERSION_CUT: u32 = mandrake_common::game_options::GAMEPLAY_1_7_2 as u32;
pub const FREQUENCY_DRAW_SITE: u32 = draw_site::PROJECTILE_FREQUENCY;

pub mod moved {
    pub const GROUND: u32 = 0x1;
    pub const CEILING: u32 = 0x2;
    pub const OBJECT: u32 = 0x4;
    pub const SECOND_OBJECT: u32 = 0x8;
    pub const EDGE: u32 = 0x20;
    pub const PASS_THROUGH: u32 = 0x40;
    pub const CELL_CHANGED: u32 = 0x80;
    pub const MEDIA: u32 = 0x300;
}

pub const OBJECT_AT_REST: u16 = 0x100;

#[derive(Debug, Clone, Default)]
pub struct FlightTerrain {
    pub width: i32,
    pub height: i32,
    corners: Vec<MeshHeightCorner>,
    normals: Vec<u16>,
    media: Option<mandrake_tag::mesh::MediaCoverage>,
    models: Option<std::sync::Arc<crate::models::ModelGrid>>,
    ceiling: Option<i16>,
}

impl FlightTerrain {
    pub fn from_mesh(mesh: &MeshTag) -> Self {
        Self::new(
            i32::from(mesh.cell_width),
            i32::from(mesh.cell_height),
            mesh.cells
                .iter()
                .map(|c| (c.physical_height, c.media_height, c.flags))
                .collect(),
        )
        .with_normals(mesh.cells.iter().map(|c| c.normal).collect())
    }

    pub fn with_normals(mut self, normals: Vec<u16>) -> Self {
        self.normals = normals;
        self
    }

    pub fn new(width: i32, height: i32, corners: Vec<MeshHeightCorner>) -> Self {
        Self {
            width,
            height,
            corners,
            normals: Vec::new(),
            media: None,
            models: None,
            ceiling: None,
        }
    }

    pub fn with_media(mut self, media: mandrake_tag::mesh::MediaCoverage) -> Self {
        self.media = Some(media);
        self
    }

    pub fn with_ceiling(mut self, ceiling: Option<i16>) -> Self {
        self.ceiling = ceiling;
        self
    }

    pub fn with_models(mut self, models: crate::models::ModelGrid) -> Self {
        self.models = Some(std::sync::Arc::new(models));
        self
    }

    pub fn with_cell_flags(mut self, flags: &[u16]) -> Self {
        for (c, &f) in self.corners.iter_mut().zip(flags) {
            c.2 = f;
        }
        self
    }

    pub fn with_heights(mut self, heights: &[(i16, i16)]) -> Self {
        for (c, &(physical, _)) in self.corners.iter_mut().zip(heights) {
            c.0 = physical;
        }
        self
    }

    pub fn update_cell_height(&mut self, x: i32, y: i32, delta: i16) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let idx = (y * self.width + x) as usize;
        if let Some(c) = self.corners.get_mut(idx) {
            if c.2 & 0x1800 != 0 {
                c.1 = c.1.wrapping_add(delta);
            } else {
                c.0 = c.0.wrapping_add(delta);
            }
        }
    }

    fn hit_model(
        &self,
        a: [i32; 3],
        b: [i32; 3],
        offset: i32,
    ) -> Result<Option<mandrake_tag::model_hit::ModelHit>, Unmodelled> {
        let Some(models) = self.models.as_ref() else {
            return Ok(None);
        };
        if MINIMUM_GAMEPLAY_VERSION > 0x6b8 {
            models.object_hit_model(a, b, offset)
        } else {
            models.object_hit_model_old(a, b, offset)
        }
    }

    fn clamped_cell(&self, x: i32, y: i32) -> (usize, usize) {
        let cx = (x >> 9).clamp(0, (self.width - 1).max(0)) as usize;
        let cy = (y >> 9).clamp(0, (self.height - 1).max(0)) as usize;
        (cx, cy)
    }

    pub fn cell_media_height(&self, x: i32, y: i32) -> i32 {
        let (cx, cy) = self.clamped_cell(x, y);
        i32::from(self.corner(cx, cy).1)
    }

    pub fn flat(width: i32, height: i32, h: i16) -> Self {
        Self::new(width, height, vec![(h, h, 0); (width * height) as usize])
    }

    fn corner(&self, cx: usize, cy: usize) -> MeshHeightCorner {
        self.corners
            .get(cy * self.width as usize + cx)
            .copied()
            .unwrap_or_default()
    }

    pub fn height_with(&self, x: i32, y: i32, mode: MeshHeightMode) -> i32 {
        get_mesh_height(
            self.width.max(0) as usize,
            self.height.max(0) as usize,
            x,
            y,
            mode,
            |cx, cy| self.corner(cx, cy),
        )
    }

    pub fn height(&self, x: i32, y: i32) -> i32 {
        self.height_with(x, y, MeshHeightMode::Physical)
    }

    pub fn cell_flags(&self, x: i32, y: i32) -> u16 {
        let (cx, cy) = self.clamped_cell(x, y);
        self.corner(cx, cy).2
    }

    pub fn normal(&self, x: i32, y: i32) -> [i16; 3] {
        let (cx, cy) = self.clamped_cell(x, y);
        let (fx, fy) = ((x & 0x1ff) as i16, (y & 0x1ff) as i16);
        let word = self
            .normals
            .get(cy * self.width as usize + cx)
            .copied()
            .unwrap_or(0);
        let high = if ((x >> 9) ^ (y >> 9)) & 1 != 0 {
            i32::from(fy) < 0x200 - i32::from(fx)
        } else {
            fx < fy
        };
        let index = if high { word >> 8 } else { word & 0xff };
        mesh_normal_table()[index as usize]
    }
}

pub fn mesh_normal_table() -> &'static [[i16; 3]; 256] {
    static TABLE: std::sync::OnceLock<[[i16; 3]; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let t = trig_tables();
        let mut out = [[0i16; 3]; 256];
        let mut k = 0usize;
        let mut e: i32 = 0x4020;
        while k <= 0xff {
            let ei = ((e as u32 & 0xffff) >> 6) as usize;
            let c = i32::from(t.cosine[ei]);
            let n = (c << 16 >> 14) / 0x444 + 1;
            let step = if n != 0 { 0x10000 / n } else { 0 };
            let mut a: i32 = 0x20;
            let mut left = n;
            while k <= 0xff && left > 0 {
                let ai = ((a as u32 & 0xffff) >> 6) as usize;
                let x = (c * i32::from(t.cosine[ai])) >> 14;
                let y = (c * i32::from(t.sine[ai])) >> 14;
                out[k] = [x as i16, y as i16, t.sine[ei]];
                k += 1;
                left -= 1;
                a += step;
            }
            e -= 0x444;
        }
        out
    })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlightDefinition {
    pub tag: FourCC,
    pub flags: u32,
    pub physics: ObjectPhysics,
    pub bounds: SequenceWorldBounds,
    pub collision_chance: u16,
    pub rebound_sound: Option<FourCC>,
    pub speed: i16,
    pub speed_delta: i16,
    pub velocity_error: i16,
    pub lifetime: i16,
    pub lifetime_delta: i16,
    pub countdown: i16,
    pub countdown_delta: i16,
    pub trail: bool,
    pub stain_sequence: i16,
    pub damage_type: i16,
    pub byte_df: u8,
    #[serde(default, with = "arc_opt_serde")]
    pub proj: Option<std::sync::Arc<ProjectileDefinition>>,
}

mod arc_opt_serde {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, T>(v: &Option<Arc<T>>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        v.as_deref().serialize(s)
    }

    pub fn deserialize<'de, D, T>(d: D) -> Result<Option<Arc<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Ok(Option::<T>::deserialize(d)?.map(Arc::new))
    }
}

impl FlightDefinition {
    pub fn new(
        proj: &ProjectileDefinition,
        physics: ObjectPhysics,
        bounds: SequenceWorldBounds,
    ) -> Self {
        Self {
            tag: proj.tag_id,
            flags: proj.flags,
            physics,
            bounds,
            collision_chance: proj.collision_chance,
            rebound_sound: proj.rebound_sound,
            speed: proj.speed,
            speed_delta: proj.speed_delta,
            velocity_error: proj.velocity_error,
            lifetime: proj.lifetime,
            lifetime_delta: proj.lifetime_delta,
            countdown: proj.countdown,
            countdown_delta: proj.countdown_delta,
            trail: proj.projectile_14.is_some(),
            stain_sequence: proj.sequences[2],
            damage_type: proj.damage.damage_type,
            byte_df: proj.byte_df,
            proj: Some(std::sync::Arc::new(proj.clone())),
        }
    }
}

pub mod kind {
    pub const SCENERY: u8 = 1;
    pub const MONSTER: u8 = 2;
    pub const PROJECTILE: u8 = 3;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CollisionObject {
    pub object: i32,
    pub unit: Option<u32>,
    pub kind: u8,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: i16,
    pub bottom: i16,
    pub top: i16,
    pub flag_2000: bool,
}

impl CollisionObject {
    pub fn scaled(bounds: SequenceWorldBounds, scale: i32) -> (i16, i16, i16) {
        let s = |v: i16| ((i32::from(v) * scale) >> 8) as i16;
        (s(bounds.half_width), s(bounds.bottom), s(bounds.top))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlightObject {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub vx: i16,
    pub vy: i16,
    pub vz: i16,
    pub above_ground: i16,
    #[serde(default)]
    pub hover: i16,
    pub shape_flags: u16,
    pub flag_8000: bool,
    #[serde(default)]
    pub flag_1000: bool,
    pub ignore_object: i32,
    pub radius: i16,
    pub bottom: i16,
    pub top: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveResult {
    pub flags: u32,
    pub hit: Option<usize>,
    pub model_hit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelHitMove {
    pub flags: u32,
    pub position: [i32; 3],
    pub velocity: [i16; 3],
}

thread_local! {
    pub static MODEL_HIT_MOVE_LOG: std::cell::RefCell<Option<Vec<ModelHitMove>>> =
        const { std::cell::RefCell::new(None) };
}

pub fn reflect_vector(v: [i16; 3], n: [i16; 3]) -> [i16; 3] {
    let d = (i32::from(v[1]) * i32::from(n[1])
        + i32::from(v[0]) * i32::from(n[0])
        + i32::from(v[2]) * i32::from(n[2]))
        >> 14;
    let r = |a: i16, b: i16| (i32::from(a) - ((i32::from(b) * d) >> 13)) as i16;
    [r(v[0], n[0]), r(v[1], n[1]), r(v[2], n[2])]
}

#[derive(Debug, Default)]
pub struct ObjectCells {
    entries: std::cell::OnceCell<Vec<((i32, i32), u32)>>,
    walks: std::cell::Cell<u32>,
}

impl ObjectCells {
    const WALKS: u32 = 8;

    pub fn reset(&mut self) {
        self.entries.take();
        self.walks.set(0);
    }

    fn of(&self, objects: &[CollisionObject]) -> Option<&[((i32, i32), u32)]> {
        if self.entries.get().is_none() {
            let w = self.walks.get();
            if w < Self::WALKS {
                self.walks.set(w + 1);
                return None;
            }
        }
        let e = self.entries.get_or_init(|| {
            let mut e: Vec<((i32, i32), u32)> = objects
                .iter()
                .enumerate()
                .map(|(i, o)| ((o.x >> 9, o.y >> 9), i as u32))
                .collect();
            e.sort_unstable();
            e
        });
        debug_assert_eq!(e.len(), objects.len());
        Some(e)
    }
}

fn cell_positions(
    entries: &[((i32, i32), u32)],
    c: (i32, i32),
) -> impl Iterator<Item = usize> + '_ {
    let from = entries.partition_point(|e| e.0 < c);
    entries[from..]
        .iter()
        .take_while(move |e| e.0 == c)
        .map(|e| e.1 as usize)
}

#[derive(Debug, Clone, Copy)]
pub struct CollisionObjects<'a> {
    pub list: &'a [CollisionObject],
    cells: Option<&'a ObjectCells>,
}

impl<'a> CollisionObjects<'a> {
    pub const NONE: CollisionObjects<'static> = CollisionObjects {
        list: &[],
        cells: None,
    };

    pub fn with_cells(list: &'a [CollisionObject], cells: &'a ObjectCells) -> Self {
        Self {
            list,
            cells: Some(cells),
        }
    }
}

impl<'a> From<&'a [CollisionObject]> for CollisionObjects<'a> {
    fn from(list: &'a [CollisionObject]) -> Self {
        Self { list, cells: None }
    }
}

impl std::ops::Deref for CollisionObjects<'_> {
    type Target = [CollisionObject];
    fn deref(&self) -> &[CollisionObject] {
        self.list
    }
}

#[allow(clippy::too_many_arguments)]
fn collide_objects(
    obj: &FlightObject,
    pos: &mut [i32; 3],
    copy: [i32; 3],
    normal: &mut [i16; 3],
    flags: &mut u32,
    objects: CollisionObjects<'_>,
    own_flag_2000: bool,
) -> (bool, bool, Option<usize>) {
    let (mut collided, mut stop, mut hit) = (false, false, None);
    let cell = (pos[0] >> 9, pos[1] >> 9);
    let sx = if pos[0] & 0x1ff > 0xff { 1 } else { -1 };
    let sy = if pos[1] & 0x1ff > 0xff { 1 } else { -1 };
    let walk = || {
        let mut in_cell: [Vec<usize>; 4] = Default::default();
        for (i, o) in objects.iter().enumerate() {
            let (dx, dy) = ((o.x >> 9) - cell.0, (o.y >> 9) - cell.1);
            let k = match (dx, dy) {
                (0, 0) => 0,
                (x, 0) if x == sx => 1,
                (0, y) if y == sy => 2,
                (x, y) if x == sx && y == sy => 3,
                _ => continue,
            };
            if o.object != obj.ignore_object {
                in_cell[k].push(i);
            }
        }
        in_cell
    };
    let mut test_object = |i: usize, stop: &mut bool| -> bool {
        let o = &objects[i];
        if o.object == obj.ignore_object {
            return false;
        }
        let dx = pos[0] - o.x;
        let dy = pos[1] - o.y;
        let r = i32::from(o.radius) + i32::from(obj.radius);
        if r <= dx.abs() || r <= dy.abs() {
            return false;
        }
        let below = i32::from(obj.bottom) - i32::from(o.top);
        let above = i32::from(obj.top) - i32::from(o.bottom);
        let dz = o.z - pos[2];
        if dz <= below || dz >= above {
            return false;
        }
        if dx * dx + dy * dy >= r * r {
            return false;
        }
        if *flags & moved::OBJECT != 0 {
            *flags |= moved::SECOND_OBJECT;
            *stop = true;
            return true;
        }
        let old_dz = o.z - obj.z;
        if old_dz < below {
            *normal = [0, 0, 0x4000];
            pos[2] = o.z + i32::from(o.top);
        } else if old_dz > above {
            *normal = [0, 0, -0x4000];
            pos[2] = o.z + i32::from(o.bottom);
        } else {
            let a = angle_index(arctangent(dx, dy));
            let t = trig_tables();
            let (c, s) = (t.cosine[a], t.sine[a]);
            *normal = [c, s, 0];
            let r = r + 0x20;
            pos[0] = ((i32::from(c) * r) >> 14) + o.x;
            pos[1] = ((i32::from(s) * r) >> 14) + o.y;
        }
        *flags |= moved::OBJECT;
        hit = Some(i);
        if o.flag_2000 || own_flag_2000 {
            *flags |= moved::PASS_THROUGH;
            if MINIMUM_GAMEPLAY_VERSION > 0x514 && own_flag_2000 {
                *pos = copy;
            }
        }
        collided = true;
        if o.kind == kind::PROJECTILE {
            *stop = true;
        }
        false
    };
    match objects.cells.and_then(|c| c.of(objects.list)) {
        Some(cells) => {
            let at = [
                cell,
                (cell.0 + sx, cell.1),
                (cell.0, cell.1 + sy),
                (cell.0 + sx, cell.1 + sy),
            ];
            #[cfg(feature = "identity-checks")]
            {
                let in_cell = at.map(|c| {
                    cell_positions(cells, c)
                        .filter(|&i| objects[i].object != obj.ignore_object)
                        .collect::<Vec<usize>>()
                });
                assert_eq!(in_cell, walk());
            }
            for c in at {
                if stop {
                    break;
                }
                for i in cell_positions(cells, c) {
                    if test_object(i, &mut stop) {
                        break;
                    }
                }
            }
        }
        None => {
            let in_cell = walk();
            for cell_objects in &in_cell {
                if stop {
                    break;
                }
                for &i in cell_objects {
                    if test_object(i, &mut stop) {
                        break;
                    }
                }
            }
        }
    }
    (collided, stop, hit)
}

fn model_hit_response(
    hit: mandrake_tag::model_hit::ModelHit,
    pos: &mut [i32; 3],
    normal: &mut [i16; 3],
    flags: &mut u32,
    collided: &mut bool,
    terrain: &FlightTerrain,
) -> bool {
    *normal = hit.normal;
    *pos = hit.point;
    *flags |= moved::GROUND;
    *collided = true;
    pos[2] < i32::from(terrain.height(pos[0], pos[1]) as i16)
}

fn tests_objects(obj: &FlightObject, physics: &ObjectPhysics, proj_flags: u32) -> bool {
    physics.flags & OBJECT_FLAG_NO_OBJECT_COLLISIONS == 0
        && (proj_flags & 0x4_0000 != 0 || !obj.flag_8000)
}

pub fn move_object(
    obj: &mut FlightObject,
    physics: &ObjectPhysics,
    proj_flags: u32,
    terrain: &FlightTerrain,
    objects: CollisionObjects<'_>,
) -> Result<MoveResult, Unmodelled> {
    let mut flags = 0u32;
    let mut nx = obj.x + i32::from(obj.vx);
    let mut ny = obj.y + i32::from(obj.vy);
    let (vx0, vy0) = (obj.vx, obj.vy);
    if nx < MAP_EDGE {
        nx = MAP_EDGE;
        flags |= moved::EDGE;
        obj.vx = vx0.wrapping_neg();
    }
    let max_x = terrain.width * 512 - MAP_EDGE;
    if max_x <= nx {
        nx = terrain.width * 512 - MAP_EDGE - 1;
        flags |= moved::EDGE;
        obj.vx = vx0.wrapping_neg();
    }
    if ny < MAP_EDGE {
        ny = MAP_EDGE;
        flags |= moved::EDGE;
        obj.vy = vy0.wrapping_neg();
    }
    let max_y = terrain.height * 512 - MAP_EDGE;
    if max_y <= ny {
        ny = terrain.height * 512 - MAP_EDGE - 1;
        flags |= moved::EDGE;
        obj.vy = vy0.wrapping_neg();
    }
    let mut h = terrain.height(nx, ny);
    let mut media_pixel = false;
    if terrain.cell_flags(nx, ny) & 0x1800 != 0 {
        let Some(m) = terrain.media.as_ref() else {
            return Err(Unmodelled::new(
                draw_site::PROJECTILE_MEDIA_COVERAGE,
                "projectile over a media cell without the media coverage",
            ));
        };
        media_pixel = m.pixel_is_media(nx >> 6, ny >> 6);
        if obj.flag_1000 {
            h = terrain.height_with(nx, ny, MeshHeightMode::Media);
        }
    }
    if h as i16 == -0x1000 && MINIMUM_GAMEPLAY_VERSION > 0x6b8 {
        let kept = (obj.z as i16).wrapping_sub(obj.above_ground);
        let model = match terrain.models.as_ref() {
            Some(m) => m.height_at(nx, ny)?,
            None => None,
        };
        h = i32::from(model.map_or(kept, |m| kept.min(m)));
    }
    let mut nz = if physics.flags & OBJECT_FLAG_FOLLOWS_GROUND != 0 {
        let surface = terrain.height_with(nx, ny, MeshHeightMode::Surface) as i16;
        let surface = if surface == -0x1000 {
            match terrain.models.as_ref() {
                Some(m) => m.height_at(nx, ny)?,
                None => None,
            }
        } else {
            Some(surface)
        };
        match surface {
            None => i32::from(obj.z as i16),
            Some(surface) => {
                let wanted = surface.wrapping_add(obj.hover);
                let above = (obj.z as i16).wrapping_sub(wanted);
                let wanted = i32::from(wanted);
                if above > 0 {
                    wanted.max(obj.z - 0x20)
                } else if above < 0 {
                    wanted.min(obj.z + 0x20)
                } else {
                    wanted
                }
            }
        }
    } else {
        if physics.terminal_velocity == 0 || physics.terminal_velocity < obj.vz {
            obj.vz = obj.vz.wrapping_add(physics.gravity);
        }
        i32::from(obj.vz) + obj.z
    };
    let mut normal = [0i16; 3];
    let mut collided = false;
    let mut rest: u16 = 0;
    let mut media_bits: u16 = 0;
    let media_h = terrain.cell_media_height(nx, ny);
    if h > nz {
        if media_pixel {
            normal = if h as i16 == media_h as i16 {
                [0, 0, -0x4000]
            } else {
                terrain.normal(nx, ny)
            };
            media_bits = 0x40;
        } else {
            normal = terrain.normal(nx, ny);
        }
        flags |= moved::GROUND;
        nz = h;
        collided = true;
    } else if let Some(c) = terrain.ceiling
        && i32::from(c) < nz
    {
        normal = [0, 0, 0x4000];
        flags |= moved::CEILING;
        nz = i32::from(c);
        collided = true;
    }
    if media_pixel && media_h > nz {
        if obj.shape_flags & 0x40 == 0 {
            flags |= 0x100;
        }
        media_bits = if media_h > nz + i32::from(obj.top) {
            0xc0
        } else {
            0x40
        };
    } else if obj.shape_flags & 0x40 != 0 {
        flags |= 0x200;
    }
    let z_before_objects = nz;
    let mut pos = [nx, ny, nz];
    let copy = pos;
    let test_objects = tests_objects(obj, physics, proj_flags);
    let mut hit = None;
    let mut model_hit = false;
    if test_objects && (pos[0] != obj.x || pos[1] != obj.y || pos[2] != obj.z) {
        let offset = i32::from(obj.radius);
        let from = [obj.x, obj.y, obj.z];
        let mut stop = false;
        if MINIMUM_GAMEPLAY_VERSION <= 0x6b8
            && let Some(m) = terrain.hit_model(from, pos, offset)?
        {
            model_hit = true;
            stop = model_hit_response(m, &mut pos, &mut normal, &mut flags, &mut collided, terrain);
        }
        let (c, s, h_obj) =
            collide_objects(obj, &mut pos, copy, &mut normal, &mut flags, objects, false);
        hit = h_obj;
        collided |= c;
        stop |= s;
        if MINIMUM_GAMEPLAY_VERSION > 0x6b8
            && let Some(m) = terrain.hit_model(from, pos, offset)?
        {
            model_hit = true;
            stop |=
                model_hit_response(m, &mut pos, &mut normal, &mut flags, &mut collided, terrain);
        }
        if stop {
            obj.z = terrain.height_with(obj.x, obj.y, MeshHeightMode::Surface);
            obj.vx = 0;
            obj.vy = 0;
            obj.vz = 0;
            return Ok(MoveResult {
                flags,
                hit,
                model_hit,
            });
        }
    }
    if collided {
        if flags & moved::PASS_THROUGH == 0 {
            let mut e = i32::from(physics.elasticity);
            let nzw = normal[2];
            if flags & moved::OBJECT == 0 && (nzw.wrapping_add(0x7ff) as u16) <= 0xffe && e <= 0xbf
            {
                e += ((0xc0 - e) * (0x800 - i32::from(nzw).abs())) >> 11;
            }
            let v = reflect_vector([obj.vx, obj.vy, obj.vz], normal);
            obj.vx = ((i32::from(v[0]) * e) >> 8) as i16;
            obj.vy = ((i32::from(v[1]) * e) >> 8) as i16;
            obj.vz = ((i32::from(v[2]) * e) >> 8) as i16;
        }
        if flags & moved::GROUND != 0 {
            let s = i32::from(obj.vx).abs() + i32::from(obj.vy).abs() + i32::from(obj.vz).abs();
            if s <= 4 {
                if normal[2] > 0x3000 || (obj.vy <= 4 && obj.vx <= 4) {
                    obj.vx = 0;
                    obj.vy = 0;
                    obj.vz = 0;
                    rest |= 0x110;
                } else {
                    obj.vz = 5;
                }
            }
        }
    }
    if (pos[0] >> 9, pos[1] >> 9) != (obj.x >> 9, obj.y >> 9) {
        flags |= moved::CELL_CHANGED;
    }
    obj.x = pos[0];
    obj.y = pos[1];
    obj.z = pos[2];
    obj.above_ground = (z_before_objects - h) as i16;
    obj.shape_flags = rest | media_bits;
    Ok(MoveResult {
        flags,
        hit,
        model_hit,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlightProjectile {
    pub id: u32,
    pub definition: FourCC,
    pub owner: u32,
    pub target: Option<u32>,
    pub object: FlightObject,
    pub record_flags: u16,
    pub lifetime: u16,
    pub facing: u16,
    pub elevation: u16,
    pub launched_tick: u32,
    pub created_at: Option<([i32; 3], [i16; 3])>,
    pub moves: u32,
    pub index: i32,
    pub fuse_threshold: u16,
    #[serde(skip)]
    pub shape: Option<crate::engine_rng::EngineObject>,
    #[serde(default)]
    pub countdown: i16,
    #[serde(default)]
    pub owner_monster: i16,
    #[serde(default)]
    pub record_0a: crate::statistics::UnitRef,
    #[serde(default)]
    pub record_36: i16,
    #[serde(default)]
    pub record_0e: i16,
    #[serde(default)]
    pub engine: bool,
    #[serde(default)]
    pub stain_point: [i32; 2],
    #[serde(default)]
    pub trail_counter: i16,
    #[serde(default)]
    pub trail_count: i16,
    #[serde(default)]
    pub trail_threshold: u16,
    #[serde(default)]
    pub target_point: Option<[i32; 3]>,
    #[serde(default)]
    pub cell_link: u64,
    #[serde(default)]
    pub woken: bool,
    #[serde(default)]
    pub homing_speed: i32,
    #[serde(default)]
    pub homing_nearest: Option<i32>,
    #[serde(default)]
    pub lightning: Option<(i16, i16)>,
    #[serde(default)]
    pub relinked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightningView {
    pub head: [i32; 3],
    pub end: [i32; 3],
    pub target: i16,
    pub target_alive: bool,
    pub state: i16,
    pub flags: u32,
}

pub fn media_group_type(word_0e: i16, flags: u32) -> Option<usize> {
    if flags & 0x1_0000 != 0 {
        return Some(5);
    }
    if flags & 0x10_0000 != 0 {
        return Some(4);
    }
    if flags & 0x8000 != 0 {
        return Some(3);
    }
    (word_0e as u16 <= 2).then_some(word_0e as usize)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlightStep {
    Flying,
    Rebounded { sound: Option<FourCC> },
    Detonated { hit: Option<usize>, keep: bool },
    Replaced { sound: Option<FourCC> },
    Dormant { sound: Option<FourCC> },
}

pub trait UpdateWorld {
    fn chance(&mut self, site: u32, threshold: u16) -> bool;
    fn draw(&mut self, site: u32) -> u16;
    fn stain(&mut self, collection: FourCC, sequence: i16, point: [i32; 3]);
    fn trail(
        &mut self,
        p: &FlightProjectile,
        desc: &crate::projectile_groups::ProjectileDescriptor,
    ) -> Result<(), Unmodelled>;
    fn game_options_high(&self) -> u16 {
        0
    }
    fn media_group(&mut self, p: &FlightProjectile, t: usize) -> Result<(), Unmodelled> {
        let _ = p;
        Err(Unmodelled::new(
            draw_site::MEDIA_GROUP_NONE,
            format!("projectile entered media: media group type {t}"),
        ))
    }
    fn target_monster_position(&self, target: u32) -> Option<(i32, i32)> {
        let _ = target;
        None
    }
    fn lightning(&self, link: (i16, i16)) -> Option<LightningView> {
        let _ = link;
        None
    }
    fn lightning_ground_hit(&mut self, index: i16) {
        let _ = index;
    }
    fn lightning_object_hit(&mut self, index: i16, object: i32) {
        let _ = (index, object);
    }
}

struct ChanceOnly<'a>(&'a mut dyn FnMut(u32, u16) -> bool);

impl UpdateWorld for ChanceOnly<'_> {
    fn chance(&mut self, site: u32, threshold: u16) -> bool {
        (self.0)(site, threshold)
    }
    fn stain(&mut self, _: FourCC, _: i16, _: [i32; 3]) {}
    fn draw(&mut self, site: u32) -> u16 {
        let _ = site;
        0
    }
    fn trail(
        &mut self,
        _: &FlightProjectile,
        _: &crate::projectile_groups::ProjectileDescriptor,
    ) -> Result<(), Unmodelled> {
        Err(Unmodelled::new(
            draw_site::TRAIL_WITHOUT_WORLD,
            "trail projectile without a world",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchDescriptor {
    pub position: [i32; 3],
    pub velocity: [i16; 3],
    pub facing: u16,
    pub elevation: u16,
    pub flags: u16,
    pub owner_object: i32,
}

impl FlightProjectile {
    pub fn launch(
        id: u32,
        def: &FlightDefinition,
        owner: u32,
        target: Option<u32>,
        desc: &LaunchDescriptor,
        tick: u32,
        draw: &mut dyn FnMut(u32) -> u16,
    ) -> Result<Self, Unmodelled> {
        if def.physics.flags & OBJECT_FLAG_FOLLOWS_GROUND != 0 {
            return Err(Unmodelled::new(
                draw_site::LAUNCH_FOLLOWS_GROUND,
                "obje flag 4 without the mesh for its height",
            ));
        }
        let mut speed = i32::from(def.speed);
        if def.speed_delta > 0 {
            speed += (i32::from(draw(draw_site::NEW_PROJECTILE_SPEED))
                * i32::from(def.speed_delta))
                >> 16;
        }
        let speed = i32::from(speed as i16);
        let mut v = desc.velocity.map(|c| ((i32::from(c) * speed) >> 8) as i16);
        if def.velocity_error > 0 {
            let e = i32::from(def.velocity_error);
            for (c, site) in v.iter_mut().zip([
                draw_site::NEW_PROJECTILE_ERROR_X,
                draw_site::NEW_PROJECTILE_ERROR_Y,
                draw_site::NEW_PROJECTILE_ERROR_Z,
            ]) {
                *c = c.wrapping_add((((i32::from(draw(site)) * (2 * e + 1)) >> 16) - e) as i16);
            }
        }
        let mut lifetime = i32::from(def.lifetime);
        if def.lifetime_delta > 0 {
            lifetime += (i32::from(draw(draw_site::NEW_PROJECTILE_LIFETIME))
                * i32::from(def.lifetime_delta))
                >> 16;
        }
        let mut record_flags = desc.flags & !0x100;
        if def.lifetime == 0 && def.lifetime_delta == 0 {
            record_flags |= 0x100;
        }
        let mut countdown = i32::from(def.countdown as u16);
        if def.countdown_delta > 0 {
            countdown += (i32::from(draw(draw_site::NEW_PROJECTILE_COUNTDOWN))
                * i32::from(def.countdown_delta))
                >> 16;
        }
        if def.flags & 0x2000_0000 != 0 {
            record_flags |= 1;
        }
        let (radius, bottom, top) = CollisionObject::scaled(def.bounds, 0x100);
        Ok(Self {
            id,
            definition: def.tag,
            owner,
            target,
            object: FlightObject {
                x: desc.position[0],
                y: desc.position[1],
                z: desc.position[2],
                vx: v[0],
                vy: v[1],
                vz: v[2],
                above_ground: 0,
                hover: 0,
                shape_flags: 0,
                flag_8000: def.flags & 0x4000 != 0,
                flag_1000: def.flags & 0x10_0000 != 0,
                ignore_object: desc.owner_object,
                radius,
                bottom,
                top,
            },
            record_flags,
            lifetime: lifetime as u16,
            facing: desc.facing,
            elevation: desc.elevation,
            launched_tick: tick,
            created_at: None,
            moves: 0,
            index: 0,
            fuse_threshold: 0,
            shape: None,
            countdown: countdown as i16,
            owner_monster: -1,
            record_0a: crate::statistics::UnitRef::NONE,
            record_36: -1,
            record_0e: 0,
            engine: false,
            stain_point: [0; 2],
            trail_counter: 0,
            trail_count: 0,
            trail_threshold: 0,
            target_point: None,
            cell_link: 0,
            woken: false,
            homing_speed: if def.flags & 2 != 0 {
                let (vx, vy) = (i32::from(v[0]), i32::from(v[1]));
                isqrt(vx.wrapping_mul(vx).wrapping_add(vy.wrapping_mul(vy)) as u32)
            } else {
                0
            },
            homing_nearest: None,
            lightning: None,
            relinked: false,
        })
    }

    pub fn step(
        &mut self,
        def: &FlightDefinition,
        terrain: &FlightTerrain,
        objects: &[CollisionObject],
        tick: u32,
        chance: &mut dyn FnMut(u32, u16) -> bool,
    ) -> Result<FlightStep, Unmodelled> {
        self.update(
            def,
            &[],
            terrain,
            objects.into(),
            tick,
            &mut ChanceOnly(chance),
        )
    }

    pub(crate) fn update_reads_objects(&self, def: &FlightDefinition) -> bool {
        if self.countdown != 0 && self.countdown != 1 {
            return false;
        }
        if def.flags & 0x0100_0000 != 0 && self.lightning.is_some() {
            return true;
        }
        let at_rest = self.object.shape_flags & OBJECT_AT_REST != 0;
        !at_rest && tests_objects(&self.object, &def.physics, def.flags)
    }

    pub fn update(
        &mut self,
        def: &FlightDefinition,
        headers: &[mandrake_tag::SequenceHeader],
        terrain: &FlightTerrain,
        objects: CollisionObjects<'_>,
        tick: u32,
        world: &mut dyn UpdateWorld,
    ) -> Result<FlightStep, Unmodelled> {
        let reads_objects = self.update_reads_objects(def);
        let f = def.flags;
        if self.countdown != 0 {
            self.countdown = self.countdown.wrapping_sub(1);
            if self.countdown != 0 {
                return Ok(FlightStep::Flying);
            }
            if let Some(s) = self.shape.as_mut() {
                s.no_shape = false;
            }
        }
        let at_rest = self.object.shape_flags & OBJECT_AT_REST != 0;
        if tick & 7 == 7
            && !at_rest
            && f & 2 != 0
            && let Some(target) = self.target
        {
            if let Some((target_x, target_y)) = world.target_monster_position(target) {
                let current_angle =
                    arctangent(i32::from(self.object.vx), i32::from(self.object.vy));
                let target_angle = arctangent(target_x - self.object.x, target_y - self.object.y);
                let diff = signed_angular_difference(current_angle, target_angle);
                if diff != 0 {
                    let turn_rate = def.proj.as_ref().map_or(0, |p| p.uword(0x5a));
                    let max_turn = turn_rate.wrapping_mul(8);
                    let new_angle = if diff.abs() <= i32::from(max_turn) {
                        target_angle
                    } else if diff > 0 {
                        current_angle.wrapping_add(max_turn)
                    } else {
                        current_angle.wrapping_sub(max_turn)
                    };
                    let idx = angle_index(new_angle);
                    let t = trig_tables();
                    self.object.vx = ((i32::from(t.cosine[idx]) * self.homing_speed) >> 14) as i16;
                    self.object.vy = ((i32::from(t.sine[idx]) * self.homing_speed) >> 14) as i16;
                }
            } else {
                return Err(Unmodelled::new(
                    draw_site::HOMING_TARGET_MISSING,
                    format!(
                        "proj {}: homing target {} position not found",
                        def.tag, target
                    ),
                ));
            }
        }
        let mut action = 0u8;
        if !at_rest || f & 0x98 != 0 {
            match self.shape.as_mut() {
                Some(s) => {
                    crate::engine_rng::animate_object(s, headers);
                    if (s.anim_flags & 4 != 0 && f & 8 != 0)
                        || (s.anim_flags & 8 != 0 && f & 0x10 != 0)
                    {
                        self.record_flags &= !2;
                        action = 1;
                    }
                }
                None if f & 0x18 != 0 => {
                    return Err(Unmodelled::new(
                        draw_site::PROJECTILE_EVENTS_NO_SHAPE,
                        "proj flags 8 / 0x10 (animation events) without a shape",
                    ));
                }
                None => {}
            }
        }
        if def.trail {
            let proj = def.proj.as_ref();
            let word = |o: usize| proj.map_or(0, |p| p.word(o));
            if self.object.shape_flags & 0x80 == 0 && self.trail_count < word(0x1a) {
                let old = self.trail_counter;
                self.trail_counter = old.wrapping_add(1);
                if old > word(0x18) {
                    let r = world.draw(draw_site::PROJECTILE_TRAIL);
                    if r <= self.trail_threshold {
                        self.trail_count += 1;
                        self.trail_counter = 0;
                        let global = if self.record_flags & 8 != 0 {
                            Some(FourCC::new(
                                crate::projectile_groups::INTERNAL_PROJECTILE_FLAG_8,
                            ))
                        } else if self.record_flags & 4 != 0 {
                            Some(FourCC::new(
                                crate::projectile_groups::INTERNAL_PROJECTILE_FLAG_4,
                            ))
                        } else {
                            None
                        };
                        let trail = global
                            .or_else(|| proj.and_then(|p| p.projectile_14))
                            .ok_or_else(|| {
                                Unmodelled::new(
                                    draw_site::TRAIL_UNRESOLVED,
                                    "trail projectile not resolved",
                                )
                            })?;
                        let desc = crate::projectile_groups::ProjectileDescriptor {
                            definition: trail,
                            flags: 0,
                            position: [self.object.x, self.object.y, self.object.z],
                            velocity: [self.object.vx, self.object.vy, self.object.vz],
                            facing: self.facing,
                            elevation: self.elevation,
                            word_1c: 0,
                            owner_monster: -1,
                            parent: self.index as i16,
                            owner_object: self.shape.as_ref().map_or(-1, |s| s.index),
                            target: -1,
                            target_point: [0; 3],
                        };
                        world.trail(self, &desc)?;
                        let step = word(0x58) as u16;
                        if step != 0 {
                            let lim = word(0x56) as u16;
                            let t = self.trail_threshold;
                            self.trail_threshold = if lim >= t {
                                (i32::from(t) + i32::from(step)).min(i32::from(lim)) as u16
                            } else {
                                (i32::from(t) - i32::from(step)).max(i32::from(lim)) as u16
                            };
                        }
                    }
                }
            }
        }
        if f & 0x0100_0000 != 0
            && let Some(link) = self.lightning
        {
            debug_assert!(reads_objects, "update_reads_objects: a lightning record");
            return self.follow_lightning(def, terrain, objects, link, action, world);
        }
        if at_rest {
            return self.finish_update(def, action, None, 0, None, tick, world);
        }
        let mut rebound: Option<Option<FourCC>> = None;
        let mut hit = None;
        debug_assert!(
            reads_objects || !tests_objects(&self.object, &def.physics, f),
            "update_reads_objects: a move"
        );
        let r = move_object(&mut self.object, &def.physics, f, terrain, objects)?;
        if let Some(s) = self.shape.as_mut() {
            s.flags = (s.flags & !0x1f0) | u32::from(self.object.shape_flags & 0x1f0);
        }
        self.moves += 1;
        if f & 2 != 0
            && self.record_flags & 0x10 != 0
            && let Some(point) = self.target_point
        {
            let d =
                crate::engine_move::distance2d(self.object.x - point[0], self.object.y - point[1]);
            match self.homing_nearest {
                Some(nearest) if d >= nearest => {
                    if d > nearest + 0x40 {
                        self.homing_speed = 0;
                        self.object.vx = 0;
                        self.object.vy = 0;
                        action = 1;
                    }
                }
                _ => self.homing_nearest = Some(d),
            }
        }
        if r.model_hit {
            MODEL_HIT_MOVE_LOG.with(|l| {
                if let Some(l) = l.borrow_mut().as_mut() {
                    let o = &self.object;
                    l.push(ModelHitMove {
                        flags: r.flags,
                        position: [o.x, o.y, o.z],
                        velocity: [o.vx, o.vy, o.vz],
                    });
                }
            });
        }
        let chance_after_reaction = world.game_options_high()
            & mandrake_common::game_options::high(mandrake_common::game_options::TFL)
            != 0;
        if r.flags & moved::MEDIA != 0 {
            let t = media_group_type(def.proj.as_ref().map_or(0, |p| p.media_group_type()), f);
            if let Some(t) = t {
                world.media_group(self, t)?;
            }
            if r.flags & 0x100 != 0 && !chance_after_reaction {
                let c = def.proj.as_ref().map_or(0, |p| p.media_chance());
                if world.chance(draw_site::PROJECTILE_BOUNCE_CHANCE, c) {
                    action = 1;
                }
            }
        }
        if r.flags & 0x17 != 0 {
            rebound = Some(def.rebound_sound);
            hit = r.hit;
            if r.flags & moved::OBJECT != 0
                && let Some(i) = r.hit
            {
                self.object.ignore_object = objects[i].object;
            }
            if world.chance(draw_site::PROJECTILE_COLLISION_CHANCE, def.collision_chance) {
                let o = &self.object;
                let slow_type_4 = def.damage_type == 4
                    && o.vx <= 0xf
                    && o.vy <= 0xf
                    && o.vz <= 0xf
                    && world.game_options_high()
                        & mandrake_common::game_options::high(mandrake_common::game_options::TFL)
                        == 0
                    && def.byte_df != 1;
                if (!slow_type_4 || action == 1)
                    && let Some(u) = r.hit.and_then(|i| objects[i].unit)
                {
                    self.target = Some(u);
                }
                if !slow_type_4 {
                    action = 1;
                }
            } else if r.flags & moved::GROUND != 0
                && def.stain_sequence != -1
                && crate::engine_move::distance2d(
                    self.object.x - self.stain_point[0],
                    self.object.y - self.stain_point[1],
                ) <= 0x3f
            {
                let collection = def
                    .proj
                    .as_ref()
                    .and_then(|p| p.collection)
                    .unwrap_or(FourCC::new([0xff; 4]));
                world.stain(
                    collection,
                    def.stain_sequence,
                    [self.object.x, self.object.y, self.object.z],
                );
                self.stain_point = [self.object.x, self.object.y];
            }
            if f & 0x400 != 0
                && let Some(p) = def.proj.as_ref()
            {
                self.trail_threshold = p.word_54() as u16;
            }
            if def.byte_df == 1 {
                let o = &self.object;
                let sum = [o.vx, o.vy, o.vz].iter().fold(0u32, |s, &c| {
                    s.wrapping_add((i32::from(c) * i32::from(c)) as u32)
                });
                let speed = i32::from(isqrt(sum) as i16);
                let proj = def.proj.as_ref();
                let limit = 3 * i32::from(proj.map_or(0, |p| p.word(0x64)));
                if speed > limit
                    && world.chance(
                        draw_site::PROJECTILE_SPEED_CHANCE,
                        proj.map_or(0, |p| p.uword(0x66)),
                    )
                {
                    action = 1;
                }
            }
        }
        if chance_after_reaction && r.flags & 0x100 != 0 {
            let c = def.proj.as_ref().map_or(0, |p| p.media_chance());
            if world.chance(draw_site::PROJECTILE_MEDIA_CHANCE, c) {
                action = 1;
            }
        }
        if self.object.shape_flags & OBJECT_AT_REST != 0 {
            if f & 0x20 != 0 {
                action = 1;
            }
            if f & 0x100 != 0 {
                action = 3;
            }
        }
        self.finish_update(def, action, hit, r.flags, rebound, tick, world)
    }

    fn follow_lightning(
        &mut self,
        def: &FlightDefinition,
        terrain: &FlightTerrain,
        objects: CollisionObjects<'_>,
        link: (i16, i16),
        action: u8,
        world: &mut dyn UpdateWorld,
    ) -> Result<FlightStep, Unmodelled> {
        let Some(l) = world.lightning(link) else {
            return if action == 1 {
                self.detonated(def, None, 0)
            } else {
                Ok(FlightStep::Flying)
            };
        };
        let (bottom, top, radius) = if world.game_options_high()
            & mandrake_common::game_options::high(
                mandrake_common::game_options::TFL | mandrake_common::game_options::ANTI_CLUMP,
            )
            != 0
            || l.flags & 0x10 != 0
        {
            (-0x60, 0x60, 0xc0)
        } else {
            (-0x40, 0x40, 0x80)
        };
        let o = &mut self.object;
        o.vx = (l.head[0] as i16).wrapping_sub(o.x as i16);
        o.vy = (l.head[1] as i16).wrapping_sub(o.y as i16);
        o.vz = (l.head[2] as i16).wrapping_sub(o.z as i16);
        o.shape_flags &= !0x10;
        o.radius = radius;
        o.bottom = bottom;
        o.top = top;
        let r = move_object(&mut self.object, &def.physics, def.flags, terrain, objects)?;
        if let Some(s) = self.shape.as_mut() {
            s.flags = (s.flags & !0x1f0) | u32::from(self.object.shape_flags & 0x1f0);
        }
        self.moves += 1;
        if l.state == 1 {
            self.object.x = l.end[0];
            self.object.y = l.end[1];
            self.object.z = l.end[2];
            self.relinked = true;
            let hit = if l.target != -1 && l.target_alive {
                let i = objects
                    .iter()
                    .position(|o| o.object == i32::from(l.target) && o.kind == 2)
                    .ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::LIGHTNING_TARGET_NOT_COLLIDABLE,
                            format!("lightning target object {} not collidable", l.target),
                        )
                    })?;
                Some(i)
            } else {
                None
            };
            return self.detonated(def, hit, r.flags);
        }
        if r.flags & moved::OBJECT != 0 {
            if let Some(i) = r.hit
                && objects[i].kind == 2
            {
                self.target = objects[i].unit;
                world.lightning_object_hit(link.0, objects[i].object);
                return self.detonated(def, Some(i), r.flags);
            }
        } else if r.flags & moved::GROUND != 0 && l.flags & 8 == 0 {
            world.lightning_ground_hit(link.0);
            return self.detonated(def, None, r.flags);
        }
        if action == 1 {
            self.detonated(def, None, r.flags)
        } else {
            Ok(FlightStep::Flying)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_update(
        &mut self,
        def: &FlightDefinition,
        mut action: u8,
        hit: Option<usize>,
        moved: u32,
        rebound: Option<Option<FourCC>>,
        tick: u32,
        world: &mut dyn UpdateWorld,
    ) -> Result<FlightStep, Unmodelled> {
        let f = def.flags;
        if f & 0x1_0000 != 0
            && (self.object.shape_flags & 0x40 != 0
                || ((tick as i32).wrapping_add(self.index) & 0x1f == 0
                    && world.chance(draw_site::PROJECTILE_FUSE, self.fuse_threshold)))
        {
            action = 2;
        }
        if self.record_flags & 0x100 == 0 {
            self.lifetime = self.lifetime.wrapping_sub(1);
            if self.lifetime == 0 {
                action = if f & 0x1000 != 0 { 2 } else { 1 };
            }
        }
        if self.record_flags & 1 != 0 || f & 0x400_0000 != 0 {
            return self.detonated(def, hit, moved);
        }
        match action {
            1 => self.detonated(def, hit, moved),
            2 => Ok(FlightStep::Replaced {
                sound: rebound.flatten(),
            }),
            3 => Ok(FlightStep::Dormant {
                sound: rebound.flatten(),
            }),
            _ => Ok(match rebound {
                Some(sound) => FlightStep::Rebounded { sound },
                None => FlightStep::Flying,
            }),
        }
    }

    fn detonated(
        &self,
        def: &FlightDefinition,
        hit: Option<usize>,
        moved: u32,
    ) -> Result<FlightStep, Unmodelled> {
        let f = def.flags;
        if f & 0x80_0000 != 0 {
            return Ok(FlightStep::Detonated {
                hit: None,
                keep: self.lifetime as i16 > 0,
            });
        }
        let keep = if f & 0x2_0000 != 0 {
            true
        } else if moved & moved::PASS_THROUGH == 0 || def.byte_df != 0 {
            false
        } else {
            self.shape.as_ref().is_some_and(|s| s.flags & 0x2000 != 0) || f & 0x2000_0002 == 0
        };
        Ok(FlightStep::Detonated { hit, keep })
    }

    pub fn heading(&self) -> u16 {
        if self.object.vx == 0 && self.object.vy == 0 {
            self.facing
        } else {
            arctangent(i32::from(self.object.vx), i32::from(self.object.vy))
        }
    }
}

pub fn build_world_vector3d(magnitude: i32, facing: u16, elevation: u16) -> [i16; 3] {
    let t = trig_tables();
    let (e, f) = (angle_index(elevation), angle_index(facing));
    let h = (i32::from(t.cosine[e]) * magnitude) >> 14;
    [
        ((i32::from(t.cosine[f]) * h) >> 14) as i16,
        ((i32::from(t.sine[f]) * h) >> 14) as i16,
        ((magnitude * i32::from(t.sine[e])) >> 14) as i16,
    ]
}

pub fn isqrt(v: u32) -> i32 {
    let v = u64::from(v);
    let mut r = (v as f64).sqrt() as u64;
    while r * r > v {
        r -= 1;
    }
    while (r + 1) * (r + 1) <= v {
        r += 1;
    }
    if v - r * r > r {
        r += 1;
    }
    r as i32
}

pub fn arcsine(s: i32) -> u16 {
    if (s + 0x4000) as u32 > 0x8000 {
        return 0;
    }
    let t = trig_tables();
    let sin = |i: i32| i32::from(t.sine[i.rem_euclid(1024) as usize]);
    let start = s >> 6;
    let mut best = start;
    for i in -256..=256 {
        let (d, db) = ((sin(i) - s).abs(), (sin(best) - s).abs());
        if d < db || (d == db && (i - start).abs() < (best - start).abs()) {
            best = i;
        }
    }
    ((best << 6) & 0xffff) as u16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BallisticAim {
    pub elevation: u16,
    pub flight_ticks: i32,
    pub reachable: bool,
}

pub fn ballistic_aim(
    from: [i32; 3],
    to: [i32; 3],
    speed: i32,
    proj_speed: i16,
    gravity: i16,
    high: bool,
) -> BallisticAim {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    let d = isqrt(d2 as u32);
    let v = (i32::from(proj_speed) * i32::from(speed as i16)) >> 8;
    let v16 = i32::from(v as i16);
    if gravity >= 0 {
        let e = arctangent(d, dz);
        let t = if v16 != 0 {
            isqrt(dz.wrapping_mul(dz).wrapping_add(d2) as u32) / v16
        } else {
            0
        };
        return BallisticAim {
            elevation: e,
            flight_ticks: t,
            reachable: true,
        };
    }
    let g = i32::from(gravity);
    let v = v16;
    let g2 = g.wrapping_mul(g);
    let a = (-v).wrapping_mul(v).wrapping_sub(g.wrapping_mul(dz));
    let disc = a.wrapping_mul(a).wrapping_sub(
        d.wrapping_mul(d)
            .wrapping_add(dz.wrapping_mul(dz))
            .wrapping_mul(g2),
    );
    if disc < 0 {
        return BallisticAim {
            elevation: (arctangent(-dz, d) as i32 >> 1) as u16,
            flight_ticks: 0,
            reachable: false,
        };
    }
    let s = isqrt(disc as u32);
    let mut t2 = (-a).wrapping_sub(s).wrapping_mul(2) / g2;
    if t2 <= 0 || high {
        t2 = s.wrapping_sub(a).wrapping_mul(2) / g2;
    }
    if t2 < 0 {
        return BallisticAim {
            elevation: (arctangent(-dz, d) as i32 >> 1) as u16,
            flight_ticks: 0,
            reachable: false,
        };
    }
    let t = isqrt(t2 as u32);
    if v == 0 || t == 0 {
        return BallisticAim {
            elevation: 0,
            flight_ticks: t,
            reachable: true,
        };
    }
    let num = dz.wrapping_sub(g.wrapping_mul(t).wrapping_mul(t) >> 1) << 14;
    let sin = (num / v.wrapping_mul(t)).clamp(-0x4000, 0x4000);
    BallisticAim {
        elevation: arcsine(sin),
        flight_ticks: t,
        reachable: true,
    }
}

pub fn thrown_aim(
    owner: [i32; 3],
    from: [i32; 3],
    to: [i32; 3],
    attack_flags: u16,
    mean: i32,
    proj_speed: i16,
    gravity: i16,
) -> Option<ThrownAim> {
    let (dx, dy, dz) = (to[0] - owner[0], to[1] - owner[1], to[2] - owner[2]);
    let d = isqrt(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32);
    if attack_flags & 0x8000 != 0 && dz < -0x300 && d <= 0x7ff {
        let aim = ballistic_aim(from, to, mean, proj_speed, gravity, false);
        return Some(ThrownAim {
            elevation: aim.elevation,
            magnitude: None,
            flight_ticks: aim.flight_ticks,
        });
    }
    let e = (i32::from(arctangent(-dz, d)) >> 1) as u16;
    let (ex, ey) = (to[0] - from[0], to[1] - from[1]);
    let d2 = isqrt(ex.wrapping_mul(ex).wrapping_add(ey.wrapping_mul(ey)) as u32);
    let t = trig_tables();
    let c = i32::from(t.cosine[angle_index(e)]);
    if c == 0 {
        return None;
    }
    let s = i32::from(t.sine[angle_index(e)]);
    let r = to[2] - from[2] - (s * d2) / c;
    if r >= 0 || gravity == 0 {
        return None;
    }
    let vmax = i32::from(((i32::from(proj_speed) * i32::from(mean as i16)) >> 8) as i16);
    let ft = isqrt(((2 * r) / i32::from(gravity)) as u32);
    if ft == 0 {
        return None;
    }
    let v = (d2 << 14) / (c * ft);
    if (vmax * 0x19) >> 8 > v || vmax < v {
        return None;
    }
    Some(ThrownAim {
        elevation: e,
        magnitude: Some(v),
        flight_ticks: ft,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThrownAim {
    pub elevation: u16,
    pub magnitude: Option<i32>,
    pub flight_ticks: i32,
}

pub fn object_key_point(
    position: [i32; 3],
    key_point: [i16; 3],
    scale: i32,
    mirrored: bool,
    facing: u16,
) -> [i32; 3] {
    let a = (i32::from(key_point[0]) * scale) >> 8;
    let mut b = (i32::from(key_point[1]) * scale) >> 8;
    let c = (scale * i32::from(key_point[2])) >> 8;
    if mirrored {
        b = -b;
    }
    let t = trig_tables();
    let i = angle_index(0u16.wrapping_sub(facing));
    let (sn, cs) = (i32::from(t.sine[i]), i32::from(t.cosine[i]));
    [
        position[0] + ((b * sn + a * cs) >> 14),
        position[1] + ((cs * b - a * sn) >> 14),
        position[2] + c,
    ]
}

pub fn attack_velocity(magnitude: i32, facing: u16, elevation: u16, error: [i32; 3]) -> [i16; 3] {
    let v = build_world_vector3d(magnitude, facing, elevation);
    [
        v[0].wrapping_add(error[0] as i16),
        v[1].wrapping_add(error[1] as i16),
        v[2].wrapping_add(error[2] as i16),
    ]
}

pub fn launch_error(r: u16, e: i32) -> i32 {
    ((i32::from(r) * (2 * e + 1)) >> 16) - e
}

impl crate::record_list::RecordIndex for FlightProjectile {
    fn record_index(&self) -> i32 {
        self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrow_def() -> FlightDefinition {
        FlightDefinition {
            tag: FourCC::new(*b"arar"),
            flags: 0x21,
            physics: ObjectPhysics {
                flags: 0,
                gravity: -2,
                elasticity: 0x80,
                terminal_velocity: 0,
                ..Default::default()
            },
            bounds: SequenceWorldBounds {
                half_width: 10,
                bottom: -10,
                top: 10,
            },
            collision_chance: 0xffff,
            rebound_sound: Some(FourCC::new(*b"brar")),
            speed: 0x100,
            speed_delta: 0,
            velocity_error: 0,
            lifetime: 0,
            lifetime_delta: 0,
            countdown: 0,
            countdown_delta: 0,
            trail: false,
            stain_sequence: -1,
            damage_type: 5,
            byte_df: 0,
            proj: None,
        }
    }

    fn launch(def: &FlightDefinition, pos: [i32; 3], v: [i16; 3]) -> FlightProjectile {
        FlightProjectile::launch(
            1,
            def,
            7,
            None,
            &LaunchDescriptor {
                position: pos,
                velocity: v,
                facing: 0,
                elevation: 0,
                flags: 0x10,
                owner_object: -1,
            },
            0,
            &mut |_| panic!("no draws"),
        )
        .unwrap()
    }

    #[test]
    fn launch_velocity_error_and_countdown_as_new_projectile() {
        let mut def = arrow_def();
        def.velocity_error = -5;
        let p = launch(&def, [10_000, 10_000, 0], [30, -7, 4]);
        assert_eq!((p.object.vx, p.object.vy, p.object.vz), (30, -7, 4));
        assert_eq!(p.countdown, 0);
        def.velocity_error = 0;
        def.countdown = 3;
        def.countdown_delta = 8;
        def.lifetime = 10;
        def.lifetime_delta = 4;
        let mut sites = Vec::new();
        let p = FlightProjectile::launch(
            1,
            &def,
            7,
            None,
            &LaunchDescriptor {
                position: [10_000, 10_000, 0],
                velocity: [30, 0, 0],
                facing: 0,
                elevation: 0,
                flags: 0x10,
                owner_object: -1,
            },
            0,
            &mut |site| {
                sites.push(site);
                0x8000
            },
        )
        .unwrap();
        assert_eq!(
            sites,
            [
                crate::draw_site::NEW_PROJECTILE_LIFETIME,
                crate::draw_site::NEW_PROJECTILE_COUNTDOWN
            ]
        );
        assert_eq!(p.countdown, 3 + 4);
    }

    #[test]
    fn gravity_is_added_before_the_move_and_the_ground_stops_it() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let def = arrow_def();
        let mut p = launch(&def, [10_000, 10_000, 100], [10, -3, 20]);
        let mut ticks = 0;
        loop {
            ticks += 1;
            match p
                .step(&def, &terrain, &[], 0, &mut |_, t| t == 0xffff)
                .unwrap()
            {
                FlightStep::Flying => {}
                FlightStep::Detonated { hit, .. } => {
                    assert_eq!(hit, None);
                    break;
                }
                s => panic!("{s:?}"),
            }
            let k = ticks;
            assert_eq!(p.object.z, 100 + 20 * k - k * (k + 1));
        }
        assert_eq!(ticks, 24);
        assert_eq!(p.object.x, 10_000 + 240);
        assert_eq!(p.object.y, 10_000 - 72);
        assert_eq!(p.object.z, 0);
    }

    #[test]
    fn a_failed_chance_rebounds_with_the_sound() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let def = arrow_def();
        let mut p = launch(&def, [10_000, 10_000, 10], [10, 0, -20]);
        let s = p.step(&def, &terrain, &[], 0, &mut |_, _| false).unwrap();
        assert_eq!(
            s,
            FlightStep::Rebounded {
                sound: Some(FourCC::new(*b"brar"))
            }
        );
        assert!(p.object.vz > 0, "reflected upwards: {:?}", p.object);
    }

    #[test]
    fn a_monster_in_the_way_is_hit_from_the_side() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let def = arrow_def();
        let target = CollisionObject {
            object: 5,
            unit: Some(42),
            kind: kind::MONSTER,
            x: 10_300,
            y: 10_000,
            z: 0,
            radius: 40,
            bottom: 0,
            top: 300,
            flag_2000: false,
        };
        let mut p = launch(&def, [10_000, 10_000, 100], [30, 0, 4]);
        let mut n = 0;
        let hit = loop {
            n += 1;
            if let FlightStep::Detonated { hit, .. } = p
                .step(&def, &terrain, &[target], 0, &mut |_, t| t == 0xffff)
                .unwrap()
            {
                break hit;
            }
        };
        assert_eq!(hit, Some(0));
        assert_eq!(p.target, Some(42));
        assert_eq!(n, 9);
        assert_eq!(p.object.x, 10_300 - (40 + 10 + 0x20));
    }

    #[test]
    fn a_slow_damage_type_4_projectile_does_not_detonate_on_the_collision() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let mut def = arrow_def();
        def.damage_type = 4;
        let mut p = launch(&def, [10_000, 10_000, 10], [3, -40, -20]);
        let s = p.step(&def, &terrain, &[], 0, &mut |_, _| true).unwrap();
        assert!(p.object.vx <= 15 && p.object.vy <= 15 && p.object.vz <= 15);
        assert_eq!(
            s,
            FlightStep::Rebounded {
                sound: Some(FourCC::new(*b"brar"))
            }
        );
        let mut p = launch(&def, [10_000, 10_000, 10], [80, 0, -20]);
        let s = p.step(&def, &terrain, &[], 0, &mut |_, _| true).unwrap();
        assert!(p.object.vx > 15, "{:?}", p.object);
        assert!(matches!(s, FlightStep::Detonated { .. }), "{s:?}");
        def.byte_df = 1;
        def.proj = Some(std::sync::Arc::new(df_proj(1000, 0)));
        let mut p = launch(&def, [10_000, 10_000, 10], [3, -40, -20]);
        let s = p.step(&def, &terrain, &[], 0, &mut |_, _| true).unwrap();
        assert!(matches!(s, FlightStep::Detonated { .. }), "{s:?}");
    }

    fn df_proj(limit: i16, chance: u16) -> mandrake_tag::ProjectileDefinition {
        let mut raw = [0u8; 256];
        raw[0x64..0x66].copy_from_slice(&limit.to_be_bytes());
        raw[0x66..0x68].copy_from_slice(&chance.to_be_bytes());
        mandrake_tag::ProjectileDefinition::from_bytes(FourCC::new(*b"dfdf"), &raw).unwrap()
    }

    #[test]
    fn proj_df_1_detonates_by_its_speed_chance() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let mut def = arrow_def();
        def.byte_df = 1;
        for (limit, wins, detonates, drawn) in [
            (1, true, true, true),
            (1, false, false, true),
            (1000, true, false, false),
        ] {
            def.proj = Some(std::sync::Arc::new(df_proj(limit, 0x1234)));
            let mut p = launch(&def, [10_000, 10_000, 10], [80, 0, -20]);
            let mut sites = Vec::new();
            let s = p
                .step(&def, &terrain, &[], 0, &mut |site, t| {
                    sites.push((site, t));
                    site == crate::draw_site::PROJECTILE_SPEED_CHANCE && wins
                })
                .unwrap();
            assert_eq!(
                sites.contains(&(crate::draw_site::PROJECTILE_SPEED_CHANCE, 0x1234)),
                drawn,
                "{limit}: {sites:x?}"
            );
            assert_eq!(
                matches!(s, FlightStep::Detonated { .. }),
                detonates,
                "{limit} {wins}: {s:?}"
            );
        }
    }

    fn fuse(flags: u32, rest: bool) -> (FlightDefinition, FlightProjectile) {
        let mut def = arrow_def();
        def.flags = 0x1_0000 | flags;
        let mut p = launch(&def, [10_000, 10_000, 0], [0, 0, 0]);
        p.fuse_threshold = 0x7fff;
        if rest {
            p.object.shape_flags = OBJECT_AT_REST;
        }
        (def, p)
    }

    #[test]
    fn a_fuse_at_rest_burns_only_on_its_phase_with_the_chance() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let (def, mut p) = fuse(0, true);
        let s = p.step(&def, &terrain, &[], 1, &mut |_, _| panic!("no draw"));
        assert_eq!(s.unwrap(), FlightStep::Flying);
        let s = p.step(&def, &terrain, &[], 32, &mut |site, t| {
            assert_eq!((site, t), (crate::draw_site::PROJECTILE_FUSE, 0x7fff));
            false
        });
        assert_eq!(s.unwrap(), FlightStep::Flying);
        let s = p.step(&def, &terrain, &[], 32, &mut |_, _| true);
        assert_eq!(s.unwrap(), FlightStep::Replaced { sound: None });
    }

    #[test]
    fn a_fuse_in_media_burns_without_a_draw() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let (def, mut p) = fuse(0, true);
        p.object.shape_flags |= 0x40;
        let s = p.step(&def, &terrain, &[], 1, &mut |_, _| panic!("no draw"));
        assert_eq!(s.unwrap(), FlightStep::Replaced { sound: None });
    }

    #[test]
    fn a_fuse_that_lands_with_flag_0x20_detonates_unless_it_burns() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let (def, mut p) = fuse(0x20, false);
        let s = p.step(&def, &terrain, &[], 1, &mut |_, _| false).unwrap();
        assert!(matches!(s, FlightStep::Detonated { .. }), "{s:?}");
        let (def, mut p) = fuse(0x20, false);
        let s = p.step(&def, &terrain, &[], 32, &mut |_, _| false).unwrap();
        assert!(matches!(s, FlightStep::Detonated { .. }), "{s:?}");
        let (def, mut p) = fuse(0x20, false);
        let s = p
            .step(&def, &terrain, &[], 32, &mut |site, _| {
                site == crate::draw_site::PROJECTILE_FUSE
            })
            .unwrap();
        assert!(matches!(s, FlightStep::Replaced { .. }), "{s:?}");
    }

    #[test]
    fn a_fuse_that_lands_with_flag_0x100_goes_dormant_unless_it_burns() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let (def, mut p) = fuse(0x120, false);
        let s = p.step(&def, &terrain, &[], 1, &mut |_, _| false).unwrap();
        assert_eq!(
            s,
            FlightStep::Dormant {
                sound: Some(FourCC::new(*b"brar"))
            }
        );
        let (def, mut p) = fuse(0x100, false);
        let s = p
            .step(&def, &terrain, &[], 32, &mut |site, _| {
                site == crate::draw_site::PROJECTILE_FUSE
            })
            .unwrap();
        assert_eq!(
            s,
            FlightStep::Replaced {
                sound: Some(FourCC::new(*b"brar"))
            }
        );
    }

    #[test]
    fn the_lifetime_of_a_fuse_at_rest_overrides_the_burn() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let (mut def, mut p) = fuse(0, true);
        p.record_flags &= !0x100;
        p.lifetime = 1;
        let s = p.step(&def, &terrain, &[], 32, &mut |_, _| true).unwrap();
        assert!(matches!(s, FlightStep::Detonated { .. }), "{s:?}");
        def.flags |= 0x1000;
        p.lifetime = 1;
        let s = p.step(&def, &terrain, &[], 1, &mut |_, _| false).unwrap();
        assert_eq!(s, FlightStep::Replaced { sound: None });
    }

    #[test]
    fn a_mesh_ceiling_stops_the_move_there() {
        let def = arrow_def();
        let mut p = launch(&def, [10_000, 10_000, 500], [4, 0, 40]);
        let terrain = FlightTerrain::flat(64, 64, 100).with_ceiling(Some(520));
        let r = move_object(
            &mut p.object,
            &def.physics,
            def.flags,
            &terrain,
            CollisionObjects::NONE,
        )
        .unwrap();
        assert_ne!(r.flags & moved::CEILING, 0);
        assert_eq!(r.flags & moved::GROUND, 0);
        assert_eq!(p.object.z, 520);
        assert_eq!(p.object.above_ground, 420);
        let mut q = launch(&def, [10_000, 10_000, 500], [4, 0, 40]);
        let open = FlightTerrain::flat(64, 64, 100);
        let r = move_object(
            &mut q.object,
            &def.physics,
            def.flags,
            &open,
            CollisionObjects::NONE,
        )
        .unwrap();
        assert_eq!(r.flags & moved::CEILING, 0);
        assert!(q.object.z > 520);
    }

    #[test]
    fn obje_flag_4_keeps_its_height_above_the_surface_by_at_most_0x20_a_move() {
        let mut def = arrow_def();
        let mut p = launch(&def, [10_000, 10_000, 500], [4, 0, 0]);
        def.physics.flags = OBJECT_FLAG_FOLLOWS_GROUND;
        def.physics.gravity = -7;
        let terrain = FlightTerrain::flat(64, 64, 100);
        p.object.hover = 50;
        let mut z = vec![];
        for _ in 0..3 {
            move_object(
                &mut p.object,
                &def.physics,
                def.flags,
                &terrain,
                CollisionObjects::NONE,
            )
            .unwrap();
            z.push(p.object.z);
        }
        assert_eq!(z, [468, 436, 404]);
        assert_eq!(p.object.vz, 0);
        p.object.z = 149;
        move_object(
            &mut p.object,
            &def.physics,
            def.flags,
            &terrain,
            CollisionObjects::NONE,
        )
        .unwrap();
        assert_eq!(p.object.z, 150);
        p.object.z = 100;
        move_object(
            &mut p.object,
            &def.physics,
            def.flags,
            &terrain,
            CollisionObjects::NONE,
        )
        .unwrap();
        assert_eq!(p.object.z, 132);
    }

    struct TargetWorld(Option<(i32, i32)>);
    impl UpdateWorld for TargetWorld {
        fn chance(&mut self, _: u32, _: u16) -> bool {
            false
        }
        fn draw(&mut self, _: u32) -> u16 {
            0
        }
        fn stain(&mut self, _: FourCC, _: i16, _: [i32; 3]) {}
        fn trail(
            &mut self,
            _: &FlightProjectile,
            _: &crate::projectile_groups::ProjectileDescriptor,
        ) -> Result<(), Unmodelled> {
            Ok(())
        }
        fn target_monster_position(&self, _: u32) -> Option<(i32, i32)> {
            self.0
        }
    }

    #[test]
    fn homing_at_a_target_turns_towards_target_every_8th_tick() {
        let mut def = arrow_def();
        def.flags |= 2;
        let terrain = FlightTerrain::flat(64, 64, 0);
        let mut p = launch(&def, [10_000, 10_000, 500], [100, 0, 0]);
        p.target = Some(1);
        p.homing_speed = 100;
        let mut no_target_world = TargetWorld(None);
        assert!(
            p.update(
                &def,
                &[],
                &terrain,
                CollisionObjects::NONE,
                6,
                &mut no_target_world
            )
            .is_ok()
        );
        let e = p
            .update(
                &def,
                &[],
                &terrain,
                CollisionObjects::NONE,
                7,
                &mut no_target_world,
            )
            .expect_err("no target pos");
        assert_eq!(e.site, crate::draw_site::HOMING_TARGET_MISSING);

        let mut raw = vec![0u8; 256];
        raw[0x5a..0x5c].copy_from_slice(&16i16.to_be_bytes());
        let proj =
            mandrake_tag::ProjectileDefinition::from_bytes(FourCC::new(*b"maeb"), &raw).unwrap();
        def.proj = Some(std::sync::Arc::new(proj));
        let mut target_world = TargetWorld(Some((10_000, 20_000)));
        p.update(
            &def,
            &[],
            &terrain,
            CollisionObjects::NONE,
            7,
            &mut target_world,
        )
        .unwrap();
        let t = trig_tables();
        let idx = angle_index(128);
        let want_vx = ((i32::from(t.cosine[idx]) * 100) >> 14) as i16;
        let want_vy = ((i32::from(t.sine[idx]) * 100) >> 14) as i16;
        assert_eq!(p.object.vx, want_vx);
        assert_eq!(p.object.vy, want_vy);
    }

    #[test]
    fn homing_past_target_point_detonates() {
        let mut def = arrow_def();
        def.flags |= 2;
        let terrain = FlightTerrain::flat(64, 64, 0);
        let mut p = launch(&def, [10_000, 10_000, 500], [100, 0, 0]);
        p.target = None;
        p.record_flags |= 0x10;
        p.target_point = Some([10_000, 10_000, 0]);
        p.homing_nearest = None;
        p.homing_speed = 100;
        p.object.x = 10_000;
        p.object.vx = 40;
        let mut no_chance = |_: u32, _: u16| false;
        p.step(&def, &terrain, &[], 8, &mut no_chance).unwrap();
        assert_eq!(p.homing_nearest, Some(40));
        p.step(&def, &terrain, &[], 9, &mut no_chance).unwrap();
        assert_eq!(p.homing_nearest, Some(40));
        p.object.x = 10_000 + 40 + 0x41;
        let s = p.step(&def, &terrain, &[], 10, &mut no_chance).unwrap();
        assert_eq!(p.homing_speed, 0);
        assert_eq!(p.object.vx, 0);
        assert_eq!(p.object.vy, 0);
        assert!(matches!(s, FlightStep::Detonated { .. }));
    }

    #[test]
    fn arcsine_and_isqrt() {
        assert_eq!(arcsine(0), 0);
        assert_eq!(arcsine(0x4000), 0x4000);
        assert_eq!(arcsine(-0x4000), 0xc000);
        assert_eq!(isqrt(15), 4);
        assert_eq!(isqrt(12), 3);
        assert_eq!(isqrt(16), 4);
    }

    #[test]
    fn ballistic_aim_reaches_the_point() {
        let terrain = FlightTerrain::flat(64, 64, 0);
        let def = arrow_def();
        let from = [5_000, 5_000, 600];
        let to = [5_000 + 6_000, 5_000 + 1_000, 0];
        let aim = ballistic_aim(from, to, 128, 0x100, -2, false);
        assert!(aim.reachable);
        let facing = arctangent(to[0] - from[0], to[1] - from[1]);
        let v = attack_velocity(128, facing, aim.elevation, [0; 3]);
        let mut p = launch(&def, from, v);
        let mut n = 0;
        while !matches!(
            p.step(&def, &terrain, &[], 0, &mut |_, t| t == 0xffff)
                .unwrap(),
            FlightStep::Detonated { .. }
        ) {
            n += 1;
        }
        let miss = (((p.object.x - to[0]).pow(2) + (p.object.y - to[1]).pow(2)) as f64).sqrt();
        assert!(miss < 200.0, "{miss} after {n}");
        assert!(
            (n - aim.flight_ticks).abs() <= 2,
            "{n} vs {}",
            aim.flight_ticks
        );
    }

    #[test]
    fn ballistic_aim_handles_truncated_zero_speed_and_large_distance() {
        let aim = ballistic_aim([0, 0, 0], [50_000, 0, 10_000], 0x400, 0x4000, 0, false);
        assert!(aim.reachable);
        assert_eq!(aim.flight_ticks, 0);
        let aim2 = ballistic_aim([0, 0, 0], [46_340, 0, 10_000], 0x100, 0x100, 0, false);
        assert!(aim2.reachable);
    }

    #[test]
    fn countdown_and_homing_turn_rate_wrap_without_panic() {
        let mut def = arrow_def();
        def.flags |= 2;
        let mut raw = vec![0u8; 256];
        raw[0x5a..0x5c].copy_from_slice(&0x4000u16.to_be_bytes());
        let proj =
            mandrake_tag::ProjectileDefinition::from_bytes(FourCC::new(*b"maeb"), &raw).unwrap();
        def.proj = Some(std::sync::Arc::new(proj));
        let terrain = FlightTerrain::flat(64, 64, 0);
        let mut p = launch(&def, [4_000, 4_000, 500], [50, 0, 0]);
        p.countdown = i16::MIN;
        let _ = p.step(&def, &terrain, &[], 0, &mut |_, _| false).unwrap();
        assert_eq!(p.countdown, i16::MAX);

        p.countdown = 0;
        p.target = Some(1);
        p.homing_speed = 100;
        let mut world = TargetWorld(Some((4_000, 8_000)));
        let _ = p
            .update(&def, &[], &terrain, CollisionObjects::NONE, 7, &mut world)
            .unwrap();
    }
}
