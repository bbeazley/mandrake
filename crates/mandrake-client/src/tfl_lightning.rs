use crate::lightning_draw::{
    Bolt, LightningDraw, LightningLook, LightningRules, LightningSurfaces, LightningView,
    SegmentScreen,
};
use crate::projectile_local::LocalSprites;
use crate::tfl_map::TflMap;
use crate::tfl_objects::TflSprites;
use mandrake_common::FourCC;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_view::billboard::BillboardInstance;
use mandrake_view::camera::Camera;
use mandrake_view::model_surfaces::WorldFrame;
use mandrake_view::projectile::EngineViewRows;
use mandrake_view::view_region::ViewRegion;
use std::collections::BTreeSet;
use tracing::info;

pub(super) struct TflBolts {
    draw: LightningDraw,
    tried: BTreeSet<FourCC>,
    sources: Vec<crate::tfl_objects::LocalSpriteSource>,
    pub(super) sprites: Option<LocalSprites>,
    tick: Option<u32>,
    pub(super) surfaces: LightningSurfaces,
}

pub(super) fn sim_bolts(sim: &mandrake_tfl_sim::TflSim) -> Vec<Bolt> {
    sim.lightning
        .iter()
        .map(|(_, l)| Bolt {
            system: l.definition,
            origin: l.start,
            end: l.end,
            mid1: l.control[0],
            mid2: l.control[1],
            ticks: l.age,
            segments: l.length,
        })
        .collect()
}

impl TflBolts {
    pub(super) fn new() -> Self {
        Self {
            draw: LightningDraw::default(),
            tried: BTreeSet::new(),
            sources: Vec::new(),
            sprites: None,
            tick: None,
            surfaces: LightningSurfaces::default(),
        }
    }

    fn load(&mut self, map: &TflMap, bolts: &[Bolt], sprites: &mut TflSprites) {
        for b in bolts {
            if !self.tried.insert(b.system) {
                continue;
            }
            let Some(look) = map
                .tag(b"ligh", b.system)
                .and_then(|d| LightningLook::from_bytes(&d))
            else {
                info!("TFL lightning: no 'ligh' {}", b.system);
                continue;
            };
            for branch in [false, true] {
                let Some(sequence) = look.sequence(branch) else {
                    continue;
                };
                let reference = look.collection_reference;
                if self
                    .sources
                    .iter()
                    .any(|s| (s.0, s.1) == (reference, sequence))
                {
                    continue;
                }
                if let Some((visual, flags)) =
                    sprites.particle_sequence(map, reference, usize::from(sequence))
                {
                    self.sources.push((reference, sequence, visual, flags));
                    self.sprites = None;
                }
            }
            info!(
                "TFL lightning: '{}' ({} bolts, core '{}')",
                b.system, look.bolts, look.collection_reference
            );
            self.draw.looks.insert(b.system, look);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn follow(
        &mut self,
        map: &TflMap,
        sim: &mandrake_tfl_sim::TflSim,
        tick: u32,
        region: &ViewRegion,
        cells: &[mandrake_tag::tfl::TflCell],
        cols: i32,
        sprites: &mut TflSprites,
        random: &mut LocalRandom,
    ) {
        if self.tick == Some(tick) {
            return;
        }
        self.tick = Some(tick);
        let bolts = sim_bolts(sim);
        if bolts.is_empty() {
            self.surfaces = LightningSurfaces::default();
            return;
        }
        self.load(map, &bolts, sprites);
        let reflective = |x: i32, y: i32| {
            let (c, r) = (x >> 10, y >> 10);
            (c >= 0 && r >= 0 && c < cols)
                && cells
                    .get(r as usize * cols as usize + c as usize)
                    .is_some_and(crate::tfl_landscape::reflects)
        };
        self.surfaces = self.draw.bolt_surfaces(
            bolts,
            &LightningView {
                region: Some(region),
                reflective: &reflective,
                media_height: &|_, _| 0,
                media_transparency: 0,
            },
            LightningRules::TFL,
            random,
        );
        tracing::debug!(
            "TFL lightning at tick {tick}: {} segments of {:?}",
            self.surfaces.segments.len(),
            sim_bolts(sim)
                .iter()
                .map(|b| (b.system, b.origin, b.end, b.ticks, b.segments))
                .collect::<Vec<_>>()
        );
    }

    pub(super) fn wants_sprites(&self) -> bool {
        self.sprites.is_none() && !self.sources.is_empty()
    }

    pub(super) fn make_sprites(&mut self, atlas: &mandrake_render::TextureAtlas) {
        self.sprites = Some(crate::tfl_objects::local_sprites(&self.sources, atlas));
    }

    pub(super) fn overlays(
        &self,
        ground: &dyn Fn(i32, i32) -> i32,
    ) -> Vec<crate::scenery_billboards::ObjectShadow> {
        match self.sprites.as_ref() {
            Some(sprites) if !self.surfaces.overlays.is_empty() => crate::lightning_draw::overlays(
                &self.draw,
                &self.surfaces.overlays,
                sprites,
                ground,
            ),
            _ => Vec::new(),
        }
    }

    pub(super) fn billboards(
        &self,
        camera: &Camera,
        size: (u32, u32),
        atlas: &mandrake_render::TextureAtlas,
        frame: WorldFrame,
        reflection_tint: [u8; 8],
    ) -> Vec<BillboardInstance> {
        let Some(sprites) = self.sprites.as_ref() else {
            return Vec::new();
        };
        if self.surfaces.segments.is_empty() {
            return Vec::new();
        }
        let (right, up, forward) = camera.view_axes();
        let rows = EngineViewRows::from_camera_axes(right, up, forward);
        let eye = frame.to_engine(camera.eye_position()).map(|c| c as i32);
        let depth = |p: [i32; 3]| {
            (0..3).fold(0i32, |sum, i| {
                sum.wrapping_add(
                    p[i].wrapping_sub(eye[i])
                        .wrapping_mul(i32::from(rows.depth[i])),
                )
            }) >> 14
        };
        let view_proj = camera.view_proj_matrix(size.0 as f32 / size.1.max(1) as f32);
        let on_screen = |p: [i32; 3]| {
            depth(p) < 0 && {
                let ndc = view_proj.transform_point3(frame.from_engine(p.map(|c| c as f32)));
                (-1.0..=1.0).contains(&ndc[0]) && (-1.0..=1.0).contains(&ndc[1])
            }
        };
        let mut out = crate::lightning_draw::instances(
            &self.draw,
            &self.surfaces.segments,
            sprites,
            atlas,
            frame,
            (right, up, forward),
            &SegmentScreen {
                depth: &depth,
                on_screen: &on_screen,
            },
            &|_, index| index,
            (Default::default(), &|_| reflection_tint),
        );
        crate::tfl_objects::at_bucket_depth(&mut out);
        tracing::debug!(
            "TFL lightning: {} of {} segments drawn",
            out.len(),
            self.surfaces.segments.len()
        );
        out
    }
}
