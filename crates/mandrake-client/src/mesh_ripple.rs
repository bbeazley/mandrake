use mandrake_sim::engine_rng::EngineWorld;
use mandrake_sim::mesh_effects::MeshEffectDatum;
use mandrake_tag::MeshTag;
use mandrake_tag::mesh::MeshLod;

#[derive(Debug, Default)]
pub(crate) struct RippleStep {
    pub cells: Vec<(usize, i16, i16)>,
    pub lod_changed: bool,
}

#[derive(Debug)]
pub(crate) struct MeshRipple {
    mesh: Option<MeshTag>,
    lod: Option<MeshLod>,
    lod_changed: bool,
    seen: Vec<Option<MeshEffectDatum>>,
}

fn lod_region(e: &MeshEffectDatum, radius: i32, (w, h): (i32, i32)) -> [i32; 4] {
    let r = (radius + 0x1ff) >> 9;
    let (cx, cy) = (i32::from(e.center_cell_x), i32::from(e.center_cell_y));
    [
        (cx - r - 1).clamp(0, w - 1),
        (cy - r - 1).clamp(0, h - 1),
        (cx + r + 1).clamp(0, w - 1),
        (cy + r + 1).clamp(0, h - 1),
    ]
}

fn walk_region(e: &MeshEffectDatum, radius: i32, (w, h): (i32, i32)) -> [i32; 4] {
    let r = (radius + 0x1ff) >> 9;
    let (cx, cy) = (i32::from(e.center_cell_x), i32::from(e.center_cell_y));
    [
        (cx - r).clamp(0, w - 1),
        (cy - r).clamp(0, h - 1),
        (cx + r).clamp(0, w - 1),
        (cy + r).clamp(0, h - 1),
    ]
}

fn continues(prev: &MeshEffectDatum, cur: &MeshEffectDatum) -> bool {
    prev.definition == cur.definition
        && (prev.center_cell_x, prev.center_cell_y) == (cur.center_cell_x, cur.center_cell_y)
        && prev.center_height == cur.center_height
        && cur.time == prev.time + 1
}

impl MeshRipple {
    pub fn new(lod: Option<&MeshLod>) -> Self {
        Self {
            mesh: None,
            lod: lod.cloned(),
            lod_changed: false,
            seen: Vec::new(),
        }
    }

    pub fn mesh<'a>(&'a self, mesh_tag: &'a MeshTag) -> &'a MeshTag {
        self.mesh.as_ref().unwrap_or(mesh_tag)
    }

    pub fn lod(&self) -> Option<&MeshLod> {
        self.lod_changed.then_some(self.lod.as_ref()).flatten()
    }

    pub fn follow(&mut self, mesh_tag: &MeshTag, world: &EngineWorld) -> RippleStep {
        let mut step = RippleStep::default();
        if self.seen.is_empty() && world.mesh_effects.is_empty() {
            return step;
        }
        let size = world.mesh_cells;
        let (w, h) = size;
        if w <= 0 || h <= 0 || mesh_tag.cells.len() < (w * h) as usize {
            return step;
        }
        let radius = |e: &MeshEffectDatum| {
            world
                .catalog
                .mesh_effects
                .get(&e.definition)
                .map_or(0, |d| i32::from(d.radius))
        };
        let mut ended = Vec::new();
        let mut started_before = Vec::new();
        let mut started_after = Vec::new();
        let n = self.seen.len().max(world.mesh_effects.len());
        for i in 0..n {
            let prev = self.seen.get(i).and_then(Option::as_ref);
            let cur = world.mesh_effects.get(i).and_then(Option::as_ref);
            match (prev, cur) {
                (Some(p), Some(c)) if continues(p, c) => {}
                _ => {
                    if let Some(p) = prev {
                        ended.push(p.clone());
                    }
                    if let Some(c) = cur {
                        if c.time > 0 {
                            started_before.push(c.clone());
                        } else {
                            started_after.push(c.clone());
                        }
                    }
                }
            }
        }
        self.seen = world.mesh_effects.clone();
        let cell = |x: i32, y: i32| {
            let i = (y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize;
            mandrake_render::mesh::LodCell {
                height: world.heights.get(i).map_or(0, |h| h.0),
                flags: world.cells.flags.get(i).copied().unwrap_or(0),
                model: world.cells.models.get(i).copied().unwrap_or(-1),
            }
        };
        if let Some(lod) = self.lod.as_mut() {
            let before = lod.words.clone();
            let high = |lod: &mut MeshLod, e: &MeshEffectDatum| {
                if radius(e) >= 1 {
                    let [x0, y0, x1, y1] = lod_region(e, radius(e), size);
                    mandrake_render::mesh::make_cell_region_high_lod(lod, x0, y0, x1, y1);
                }
            };
            for e in &started_before {
                high(lod, e);
            }
            for e in &ended {
                if radius(e) >= 1 {
                    let [x0, y0, x1, y1] = lod_region(e, radius(e), size);
                    mandrake_render::mesh::recalculate_lod_for_cell_region(
                        lod, w, h, cell, x0, y0, x1, y1,
                    );
                }
            }
            for e in &started_after {
                high(lod, e);
            }
            if lod.words != before {
                self.lod_changed = true;
                step.lod_changed = true;
            }
        }
        let mesh = self.mesh.get_or_insert_with(|| mesh_tag.clone());
        for e in world.mesh_effects.iter().flatten() {
            if e.time == 0 || radius(e) < 1 {
                continue;
            }
            let [x0, y0, x1, y1] = walk_region(e, radius(e), size);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let i = (y * w + x) as usize;
                    let Some(&(physical, media)) = world.heights.get(i) else {
                        continue;
                    };
                    let c = &mut mesh.cells[i];
                    if (c.physical_height, c.media_height) != (physical, media) {
                        c.physical_height = physical;
                        c.media_height = media;
                        step.cells.push((i, physical, media));
                    }
                }
            }
        }
        step.cells.sort_unstable_by_key(|c| c.0);
        step.cells.dedup_by_key(|c| c.0);
        step
    }
}
