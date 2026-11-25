use crate::tfl_game::FrameTime;
use crate::tfl_map::TflMap;
use crate::tfl_objects::{TFL_CELL_SIZE, WORLD_PER_RENDER_UNIT};
use mandrake_render::mesh::{grid_vertices, landscape_triangles};
use mandrake_render::{OccluderRule, TerrainMesh};
use mandrake_tag::tfl::TflCell;

pub(crate) const CELL_WATER: u8 = 0x01;
pub(crate) const CELL_WAVES: u8 = 0x04;
pub(crate) const CELL_FIRST_PASS: u8 = 0x10;
const CELL_REFLECTS: u8 = 0x38;
pub(crate) const CELL_NOT_DRAWN: u8 = 0x40;

pub(crate) fn reflects(cell: &TflCell) -> bool {
    cell.flags & CELL_REFLECTS != 0
}

pub(crate) fn drawn_height(cell: &TflCell) -> i16 {
    if cell.flags & CELL_WATER != 0 && cell.physical_height < 1 {
        0
    } else {
        cell.physical_height
    }
}

pub(crate) fn tfl_terrain_mesh(
    device: &wgpu::Device,
    map: &TflMap,
    cells: &[TflCell],
) -> TerrainMesh {
    let mut drawn = map.mesh.clone();
    for (c, t) in drawn.cells.iter_mut().zip(cells) {
        c.physical_height = drawn_height(t);
    }
    let vertices = grid_vertices(&drawn, TFL_CELL_SIZE, 0);
    let (w, h) = (u32::from(drawn.cell_width), u32::from(drawn.cell_height));
    let vw = w + 1;
    let cell_of = |tri: &[u32; 3]| {
        let col = tri.iter().map(|i| i % vw).min().unwrap_or(0).min(w - 1);
        let row = tri.iter().map(|i| i / vw).min().unwrap_or(0).min(h - 1);
        cells.get((row * w + col) as usize)
    };
    let (mut first, mut occluders) = (Vec::new(), Vec::new());
    for (tri, _) in landscape_triangles(&drawn, None) {
        let cell = cell_of(&tri);
        if cell.is_some_and(|c| c.flags & CELL_NOT_DRAWN != 0) {
            continue;
        }
        if cell.is_none_or(|c| c.flags & CELL_FIRST_PASS == 0) {
            occluders.extend(tri);
        } else {
            first.extend(tri);
        }
    }
    let num_first = first.len() as u32;
    first.extend_from_slice(&occluders);
    let mut mesh = TerrainMesh::from_raw_with_occluders(
        device,
        "TFL Landscape",
        &vertices,
        &first,
        &occluders,
    );
    mesh.occluder_rule = OccluderRule::TflBuckets;
    mesh.num_media_indices = num_first;
    mesh
}

pub(crate) fn changed_ground(
    old: &[TflCell],
    new: &[TflCell],
    (w, h): (u32, u32),
) -> Vec<(u32, i32)> {
    let mut out = Vec::new();
    for (k, (a, b)) in old.iter().zip(new).enumerate() {
        let z = drawn_height(b);
        if drawn_height(a) == z {
            continue;
        }
        let (c, r) = (k as u32 % w, k as u32 / w);
        let (cs, rs) = (
            if c + 1 == w { &[c, w][..] } else { &[c][..] },
            if r + 1 == h { &[r, h][..] } else { &[r][..] },
        );
        for &r in rs {
            for &c in cs {
                out.push((r * (w + 1) + c, i32::from(z)));
            }
        }
    }
    out.sort_unstable();
    out
}

pub(crate) fn ground_position(i: u32, z: i32, (w, h): (u32, u32)) -> [f32; 3] {
    let (c, r) = ((i % (w + 1)) as i32, (i / (w + 1)) as i32);
    let half_w = w as f32 * TFL_CELL_SIZE * 0.5;
    let half_d = h as f32 * TFL_CELL_SIZE * 0.5;
    [
        half_w - (c * 0x400) as f32 / WORLD_PER_RENDER_UNIT,
        z as f32 / WORLD_PER_RENDER_UNIT,
        (r * 0x400) as f32 / WORLD_PER_RENDER_UNIT - half_d,
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Waves {
    pub(crate) amplitude: [i16; 3],
    pub(crate) rate: [i16; 3],
}

impl Waves {
    pub(crate) fn of(map: &TflMap) -> Option<Self> {
        let medi = map.tag(b"medi", map.tfl_mesh.media_tag?)?;
        let at = |o: usize| Some(i16::from_be_bytes(medi.get(o..o + 2)?.try_into().ok()?));
        let w = Self {
            amplitude: [at(0x68)?, at(0x6a)?, at(0x6c)?],
            rate: [at(0x6e)?, at(0x70)?, at(0x72)?],
        };
        (w.amplitude != [0; 3]).then_some(w)
    }

    pub(crate) fn offset(&self, t: i32) -> [i32; 3] {
        let sine = &mandrake_tfl_sim::trig::trig().sine;
        std::array::from_fn(|k| {
            let a = i32::from(self.amplitude[k]);
            if a == 0 {
                return 0;
            }
            let i = (i32::from(self.rate[k]).wrapping_mul(t).wrapping_add(0x20) >> 6) & 0x3ff;
            (i32::from(sine[i as usize]) * a) >> 14
        })
    }

    pub(crate) fn offset_at(&self, time: FrameTime, x: i32, y: i32) -> [i32; 3] {
        time.interpolate(|t| self.offset(Self::phase(t, x, y)))
    }

    pub(crate) fn phase(time: u32, x: i32, y: i32) -> i32 {
        (time as i32)
            .wrapping_add((x >> 10).wrapping_mul(0x179))
            .wrapping_add((y >> 10).wrapping_mul(0xd3))
    }
}

pub(crate) struct TflWaves {
    waves: Waves,
    vertices: Vec<(u32, i32, i32, i32)>,
    cells: (u32, u32),
    time: Option<FrameTime>,
}

impl TflWaves {
    pub(crate) fn new(map: &TflMap, cells: &[TflCell]) -> Option<Self> {
        let waves = Waves::of(map)?;
        let (w, h) = (map.tfl_mesh.cell_width, map.tfl_mesh.cell_height);
        let mut vertices = Vec::new();
        for r in 0..=h {
            for c in 0..=w {
                let cell = cells.get((r.min(h - 1) * w + c.min(w - 1)) as usize);
                if let Some(cell) = cell.filter(|cell| cell.flags & CELL_WAVES != 0) {
                    vertices.push((
                        r * (w + 1) + c,
                        c as i32,
                        r as i32,
                        i32::from(drawn_height(cell)),
                    ));
                }
            }
        }
        (!vertices.is_empty()).then_some(Self {
            waves,
            vertices,
            cells: (w, h),
            time: None,
        })
    }

    pub(crate) fn set_ground(&mut self, changed: &[(u32, i32)]) {
        for v in &mut self.vertices {
            if let Ok(k) = changed.binary_search_by_key(&v.0, |&(i, _)| i) {
                v.3 = changed[k].1;
                self.time = None;
            }
        }
    }

    pub(crate) fn update(&mut self, queue: &wgpu::Queue, mesh: &mut TerrainMesh, time: FrameTime) {
        if self.time == Some(time) {
            return;
        }
        self.time = Some(time);
        let half_w = self.cells.0 as f32 * TFL_CELL_SIZE * 0.5;
        let half_d = self.cells.1 as f32 * TFL_CELL_SIZE * 0.5;
        let moved: Vec<(u32, [f32; 3])> = self
            .vertices
            .iter()
            .map(|&(i, c, r, z)| {
                let [dx, dy, dz] = self.waves.offset_at(time, c << 10, r << 10);
                let (x, y) = (c * 0x400 + dx, r * 0x400 + dy);
                (
                    i,
                    [
                        half_w - x as f32 / WORLD_PER_RENDER_UNIT,
                        (z + dz) as f32 / WORLD_PER_RENDER_UNIT,
                        y as f32 / WORLD_PER_RENDER_UNIT - half_d,
                    ],
                )
            })
            .collect();
        mesh.move_vertices(queue, &moved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(h: i16, flags: u8) -> TflCell {
        TflCell {
            physical_height: h,
            flags,
            ..TflCell::default()
        }
    }

    #[test]
    fn a_water_cell_is_drawn_at_zero_where_it_is_lower() {
        assert_eq!(drawn_height(&cell(-840, 0x15)), 0);
        assert_eq!(drawn_height(&cell(0, 0x11)), 0);
        assert_eq!(drawn_height(&cell(260, 0x11)), 260);
        assert_eq!(drawn_height(&cell(-840, 0x20)), -840);
    }

    #[test]
    fn the_waves_are_tfls() {
        let w = Waves {
            amplitude: [0x100, 0, -0x80],
            rate: [0x40, 0x40, 0x100],
        };
        assert_eq!(
            Waves::phase(10, 2 << 10, 3 << 10),
            10 + 2 * 0x179 + 3 * 0xd3
        );
        assert_eq!(w.offset(0x100), [0x100, 0, 0]);
        assert_eq!(w.offset(0x40)[2], -0x80);
    }

    #[test]
    fn the_waves_between_two_ticks_are_between_their_offsets() {
        let w = Waves {
            amplitude: [0x100, 0x40, -0x80],
            rate: [0x40, 0x100, 0x100],
        };
        let (x, y) = (5 << 10, 9 << 10);
        let (a, b) = (
            w.offset(Waves::phase(40, x, y)),
            w.offset(Waves::phase(41, x, y)),
        );
        assert_ne!(a, b, "the point moves in the tick");
        let time = |fraction| FrameTime {
            before: 40,
            live: 41,
            fraction,
        };
        assert_eq!(w.offset_at(time(0), x, y), a);
        let half = w.offset_at(time(0x8000), x, y);
        for k in 0..3 {
            assert_eq!(half[k], a[k] + ((b[k] - a[k]) >> 1));
        }
        assert_eq!(
            w.offset_at(time(crate::object_history::FRACTION_ONE), x, y),
            b
        );
        assert_eq!(w.offset_at(FrameTime::whole(41), x, y), b);
    }

    #[test]
    fn a_changed_cell_moves_its_vertices() {
        let old = vec![cell(0, 0); 6];
        let mut new = old.clone();
        new[0].physical_height = 100;
        new[5].physical_height = -40;
        assert_eq!(
            changed_ground(&old, &new, (3, 2)),
            vec![(0, 100), (6, -40), (7, -40), (10, -40), (11, -40)]
        );
        let mut water = old.clone();
        water[1] = cell(-300, CELL_WATER);
        assert!(changed_ground(&old, &water, (3, 2)).is_empty());
        assert_eq!(
            ground_position(5, 1024, (3, 2)),
            [3.0 - 2.0, 2.0, 2.0 - 2.0]
        );
    }
}
