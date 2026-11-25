use crate::projectile_local::{
    AmbientSource, LocalDefinition, LocalProjectiles, LocalSequence, LocalSprites, LocalSurface,
    LocalWorld, NewGroup, ObjectRef, Rules, SurfaceView, TargetCylinder,
};
use crate::tfl_game::TflReplay;
use crate::tfl_map::TflMap;
use crate::tfl_objects::TflSprites;
use crate::unit_anim::SequenceVisual;
use mandrake_common::FourCC;
use mandrake_tfl_sim::local_groups::LocalGroupStart;
use mandrake_view::billboard::BillboardInstance;
use mandrake_view::camera::Camera;
use mandrake_view::view_region::ViewRegion;
use std::collections::{BTreeMap, BTreeSet};
use tracing::info;

const AMBIENT_TRIES: i16 = 4;
const AMBIENT_PERIOD: i16 = 3;
const DRAW_REACH: i32 = 0x7800;
const MOST_TICKS: u32 = 30;
const MOST_STARTED_TICKS: u32 = 900;
const MOST_FRAME_TICKS: u32 = 6;

pub(super) const SOUNDS_KEPT: usize = 64;

pub(super) struct TflLocal {
    pub(super) local: LocalProjectiles,
    sources: Vec<crate::tfl_objects::LocalSpriteSource>,
    pub(super) sprites: Option<LocalSprites>,
    tick: u32,
    orders_run: usize,
    starts_run: usize,
    tried: BTreeSet<FourCC>,
    collections: BTreeMap<FourCC, FourCC>,
    size: [i32; 2],
    no_splashes: bool,
    pub(super) surfaces: Vec<LocalSurface>,
    waiting_sounds: Vec<mandrake_sim::combat::SequenceSound>,
    pub(super) fixed_frames: bool,
}

fn four(d: &[u8], at: usize) -> Option<FourCC> {
    let b: [u8; 4] = d.get(at..at + 4)?.try_into().ok()?;
    (!matches!(u32::from_be_bytes(b), 0 | 0xffff_ffff)).then_some(FourCC::new(b))
}

fn be_u16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn sequence_sounds(map: &TflMap, collection: FourCC, sequence: usize) -> [bool; 3] {
    map.tag(b".256", collection)
        .as_deref()
        .and_then(crate::map_sound::sequence_sound_tags)
        .and_then(|q| q.get(sequence).copied())
        .map_or([false; 3], |t| t.map(|s| s.is_some_and(|s| s.0 != [0; 4])))
}

fn local_sequence(visual: &SequenceVisual, sounds: [bool; 3]) -> LocalSequence {
    let q = &visual.sequence;
    let frames = q.frames_per_view as i16;
    LocalSequence {
        frames,
        ticks_per_frame: q.ticks_per_frame,
        key_frame: q.key_frame,
        loop_frame: if q.loop_frame < 0 || q.loop_frame >= frames {
            0
        } else {
            q.loop_frame
        },
        stored_view: q.flags & 4 != 0,
        views: q.num_views as i16,
        permutations: 1,
        sounds,
    }
}

fn load_definition(
    local: &mut LocalProjectiles,
    sources: &mut Vec<crate::tfl_objects::LocalSpriteSource>,
    collections: &mut BTreeMap<FourCC, FourCC>,
    map: &TflMap,
    sprites: &mut TflSprites,
    id: FourCC,
) -> bool {
    let Some(definition) = map.tag(b"lpgr", id).and_then(|d| {
        LocalDefinition::parse_with_rules(id, &d, |ph| map.tag(b"phys", ph), Rules::Tfl)
    }) else {
        info!("TFL local projectiles: '{id}' not loaded (its physics type is not ported)");
        return false;
    };
    let Some((visual, flags)) = sprites.particle_sequence(
        map,
        definition.collection_reference,
        usize::from(definition.sequence),
    ) else {
        info!("TFL local projectiles: '{id}' has no sequence");
        return false;
    };
    let sounds = match map.tag(b"core", definition.collection_reference) {
        Some(core) => match four(&core, 0) {
            Some(collection) => {
                collections.insert(definition.collection_reference, collection);
                sequence_sounds(map, collection, usize::from(definition.sequence))
            }
            None => [false; 3],
        },
        None => [false; 3],
    };
    local.add_definition(definition, local_sequence(&visual, sounds));
    let key = (definition.collection_reference, definition.sequence);
    if !sources.iter().any(|s| (s.0, s.1) == key) {
        sources.push((key.0, key.1, visual, flags));
    }
    true
}

fn new_group(start: &LocalGroupStart) -> NewGroup {
    NewGroup {
        definition: start.definition,
        point: start.point,
        central: start.central.map(|o| ObjectRef {
            index: o.slot as i32,
            identifier: o.identifier,
        }),
        target: start.target.map(|t| TargetCylinder {
            point: t.point,
            radius: t.radius,
            bottom: t.bottom,
            top: t.top,
        }),
        opacity: start.opacity,
        facing: start.facing,
    }
}

impl TflLocal {
    pub(super) fn new(
        map: &TflMap,
        replay: &TflReplay,
        sprites: &mut TflSprites,
        always: bool,
    ) -> Option<Self> {
        let sim = replay.sim()?;
        let size = [
            map.tfl_mesh.cell_width as i32 * 1024,
            map.tfl_mesh.cell_height as i32 * 1024,
        ];
        let media = sim
            .data
            .mesh
            .media_tag
            .and_then(|m| map.tag(b"medi", m))
            .and_then(|d| Some((four(&d, 0x30)?, be_u16(&d, 0x62)?)));
        let part = sim.particle_system_tag().and_then(|t| map.tag(b"part", t));
        let splash = part.as_deref().and_then(|d| four(d, 0x58));
        let no_splashes = part
            .as_deref()
            .and_then(|d| d.get(3))
            .is_some_and(|f| f & 2 != 0);
        let palettes = &map.tfl_mesh.palettes;
        let mut ids: Vec<FourCC> = media
            .map(|m| m.0)
            .into_iter()
            .chain(splash)
            .chain(
                palettes
                    .iter()
                    .filter(|p| p.marker_type == 10)
                    .map(|p| p.tag_id),
            )
            .collect();
        ids.sort_unstable_by_key(|i| i.as_u32());
        ids.dedup();
        if ids.is_empty() && !always {
            return None;
        }
        let mut local = LocalProjectiles::with_rules(0, Rules::Tfl);
        let mut sources = Vec::new();
        let mut collections = BTreeMap::new();
        for id in ids {
            load_definition(&mut local, &mut sources, &mut collections, map, sprites, id);
        }
        let ambient = |group| AmbientSource {
            group,
            tries: AMBIENT_TRIES,
            period: AMBIENT_PERIOD,
        };
        local.start_ambient(
            media.map(|(g, chance)| (ambient(g), chance)),
            false,
            splash.map(ambient),
            size,
        );
        local.start_markers(palettes, &map.tfl_mesh.markers);
        let (projectiles, groups) = local.counts();
        info!(
            "TFL local projectiles: {} definitions, {groups} groups, {projectiles} projectiles at set-up",
            sources.len()
        );
        Some(Self {
            local,
            sources,
            sprites: None,
            tick: replay.tick().saturating_sub(MOST_STARTED_TICKS),
            orders_run: 0,
            starts_run: 0,
            tried: BTreeSet::new(),
            collections,
            size,
            no_splashes,
            surfaces: Vec::new(),
            waiting_sounds: Vec::new(),
            fixed_frames: true,
        })
    }

    pub(super) fn load_started(
        &mut self,
        map: &TflMap,
        replay: &TflReplay,
        sprites: &mut TflSprites,
    ) {
        let starts = replay.local_group_starts_from(self.starts_run);
        for s in starts {
            if !self.tried.insert(s.definition) || self.local.has_definition(s.definition) {
                continue;
            }
            if load_definition(
                &mut self.local,
                &mut self.sources,
                &mut self.collections,
                map,
                sprites,
                s.definition,
            ) {
                info!(
                    "TFL local projectiles: '{}' loaded for the sim's starts",
                    s.definition
                );
                self.sprites = None;
            }
        }
    }

    pub(super) fn consumed(&self) -> (usize, usize) {
        (self.orders_run, self.starts_run)
    }

    pub(super) fn start_groups(
        &mut self,
        replay: &TflReplay,
        before: Option<u32>,
        random: &mut mandrake_tag::sound_manager::LocalRandom,
    ) {
        if replay.tick() > self.tick + 1 {
            return;
        }
        std::mem::swap(self.local.random_mut(), random);
        self.start_due(replay, |s| before.is_none_or(|b| s.marks_before <= b));
        std::mem::swap(self.local.random_mut(), random);
    }

    fn start_due(&mut self, replay: &TflReplay, due: impl Fn(&LocalGroupStart) -> bool) {
        let starts = replay.local_group_starts_from(self.starts_run);
        for s in starts.iter().take_while(|s| due(s)) {
            let made = self.local.new_group(&new_group(s));
            self.starts_run += 1;
            tracing::debug!(
                "TFL local projectiles: '{}' started at tick {} at {:?}: group {made:?}",
                s.definition,
                s.tick,
                s.point
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn follow(
        &mut self,
        replay: &TflReplay,
        region: &ViewRegion,
        cells: &[mandrake_tag::tfl::TflCell],
        cols: i32,
        eye: (i32, i32),
        in_front: &dyn Fn([i32; 3]) -> bool,
        wind: Option<[i32; 3]>,
        between: &mut dyn FnMut(&mut mandrake_tag::sound_manager::LocalRandom),
    ) {
        self.follow_ticks(replay, region, cells, cols, eye, in_front, wind, between);
        let sounds = self.take_sounds();
        let room = SOUNDS_KEPT.saturating_sub(self.waiting_sounds.len());
        self.waiting_sounds.extend(sounds.into_iter().take(room));
    }

    pub(super) fn take_waiting_sounds(&mut self) -> Vec<mandrake_sim::combat::SequenceSound> {
        std::mem::take(&mut self.waiting_sounds)
    }

    pub(super) fn take_sounds(&mut self) -> Vec<mandrake_sim::combat::SequenceSound> {
        let mut sounds = self.local.take_sounds();
        for s in &mut sounds {
            if let Some(&collection) = self.collections.get(&s.collection) {
                s.collection = collection;
            }
        }
        sounds
    }

    #[allow(clippy::too_many_arguments)]
    fn follow_ticks(
        &mut self,
        replay: &TflReplay,
        region: &ViewRegion,
        cells: &[mandrake_tag::tfl::TflCell],
        cols: i32,
        eye: (i32, i32),
        in_front: &dyn Fn([i32; 3]) -> bool,
        wind: Option<[i32; 3]>,
        between: &mut dyn FnMut(&mut mandrake_tag::sound_manager::LocalRandom),
    ) {
        let now = replay.tick();
        let Some(sim) = replay.sim() else {
            return;
        };
        let orders = replay.local_orders_from(self.orders_run);
        for o in orders {
            info!(
                "TFL local projectiles: lpgr at tick {}: groups {:?}, on {}, opacity {:?}, off {}",
                o.tick, o.groups, o.on, o.opacity, o.off
            );
            self.local.run_order(&o.groups, o.on, o.opacity, o.off);
        }
        self.orders_run += orders.len();
        let behind = now.saturating_sub(self.tick);
        if !self.fixed_frames && behind <= MOST_FRAME_TICKS {
            if let Some(last) = now.checked_sub(1).filter(|_| behind > 0) {
                self.start_due(replay, |s| s.tick <= last);
            }
            self.step(
                sim,
                (region, cells, cols),
                eye,
                in_front,
                wind,
                (now, behind as i32),
                between,
            );
            self.tick = now;
            return;
        }
        if behind == 0 {
            return;
        }
        let starts = replay.local_group_starts_from(self.starts_run);
        let started = starts.first().map_or(u32::MAX, |s| {
            s.tick.max(now.saturating_sub(MOST_STARTED_TICKS))
        });
        let first = self.tick.max(now.saturating_sub(MOST_TICKS).min(started));
        self.starts_run += starts.iter().take_while(|s| s.tick < first).count();
        for t in first..now {
            let last = t + 1 == now;
            self.start_due(replay, |s| s.tick <= t);
            self.step(
                sim,
                (region, cells, cols),
                eye,
                in_front,
                wind,
                (t + 1, 1),
                &mut |random| {
                    if last {
                        between(random);
                    }
                },
            );
        }
        self.tick = now;
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        sim: &mandrake_tfl_sim::TflSim,
        (region, cells, cols): (&ViewRegion, &[mandrake_tag::tfl::TflCell], i32),
        eye: (i32, i32),
        in_front: &dyn Fn([i32; 3]) -> bool,
        wind: Option<[i32; 3]>,
        (now, ticks): (u32, i32),
        between: &mut dyn FnMut(&mut mandrake_tag::sound_manager::LocalRandom),
    ) {
        let cell = |col: i32, row: i32| {
            (col >= 0 && row >= 0 && col < cols)
                .then(|| cells.get(row as usize * cols as usize + col as usize))
                .flatten()
                .map_or(0, |c| c.flags)
        };
        let mesh = &sim.data.mesh;
        let density = |x: i32, y: i32| {
            if self.no_splashes {
                return 0;
            }
            sim.particles
                .as_ref()
                .and_then(|p| p.density(x, y, mesh.cell_width as i16, mesh.cell_height as i16))
                .unwrap_or(0)
        };
        let made = self.local.build_tfl_ambient(ticks, region, &cell, &density);
        between(self.local.random_mut());
        let (c0, c1) = (region.first_row..region.last_row)
            .filter_map(|r| region.span(r))
            .fold((i32::MAX, i32::MIN), |(lo, hi), (a, b)| {
                (lo.min(i32::from(a)), hi.max(i32::from(b)))
            });
        self.local.set_view_cells((c0 <= c1).then_some([
            c0,
            c1,
            region.first_row,
            region.last_row,
        ]));
        if ticks > 0 {
            self.local.record_history();
        }
        let height = |x: i32, y: i32| sim.mesh_height(x, y).map_or(0, |h| i32::from(h as i16));
        let (w, h) = (mesh.cell_width as i32, mesh.cell_height as i32);
        let tfl_cell = |x: i32, y: i32| {
            let (c, r) = (x >> 10, y >> 10);
            let bits = if c >= 0 && r >= 0 && c < w && r < h {
                sim.cells.get((r * w + c) as usize).map_or(0, |c| c.flags)
            } else {
                0
            };
            (bits, sim.media_pixel(x >> 6, y >> 6).unwrap_or(false))
        };
        let object = |o: ObjectRef| {
            let ob = sim.objects.get(usize::try_from(o.index).ok()?)?;
            (o.identifier != 0 && ob.identifier == o.identifier).then_some([ob.x, ob.y, ob.z])
        };
        let cell_bits = |_: ObjectRef, _: [i32; 3]| None;
        let drawn_count = std::cell::Cell::new(0usize);
        let drawn = |w: [i32; 3]| {
            let (dx, dy) = (w[0] - eye.0, w[1] - eye.1);
            let distance = if dx.abs() < 0x7fff && dy.abs() < 0x7fff {
                ((dx * dx + dy * dy) as f64).sqrt() as i32
            } else {
                0x7fff
            };
            let drawn = distance < DRAW_REACH && in_front(w);
            drawn_count.set(drawn_count.get() + usize::from(drawn));
            drawn
        };
        self.local.update(
            ticks,
            &LocalWorld {
                size: self.size,
                height: &height,
                wind,
                object: &object,
                cell_bits: &cell_bits,
                tfl_cell: Some(&tfl_cell),
                tfl_drawn: Some(&drawn),
            },
        );
        let drawn = drawn_count.get();
        let (projectiles, groups) = self.local.counts();
        tracing::debug!(
            "TFL local projectiles at tick {now}: {made} made, {projectiles} live in {groups} groups, {drawn} drawn"
        );
    }

    pub(super) fn surfaces(
        &self,
        replay: &TflReplay,
        cells: &[mandrake_tag::tfl::TflCell],
        cols: i32,
        fraction: i32,
    ) -> Vec<LocalSurface> {
        let Some(sim) = replay.sim() else {
            return Vec::new();
        };
        let height = |x: i32, y: i32| sim.mesh_height(x, y).unwrap_or(0);
        let zero = |_: i32, _: i32| 0;
        let reflective = |x: i32, y: i32| {
            let (c, r) = (x >> 10, y >> 10);
            (c >= 0 && r >= 0 && c < cols)
                && cells
                    .get(r as usize * cols as usize + c as usize)
                    .is_some_and(|c| c.flags & 0x38 != 0)
        };
        self.local.surfaces(
            fraction,
            &SurfaceView {
                eye: [0; 3],
                height: &height,
                media_height: &zero,
                region: None,
                media_transparency: 0,
                reflective: &reflective,
                mirrors: &[],
            },
        )
    }

    pub(super) fn make_sprites(&mut self, atlas: &mandrake_render::TextureAtlas) {
        self.sprites = Some(crate::tfl_objects::local_sprites(&self.sources, atlas));
    }

    pub(super) fn billboards(
        &self,
        camera: &Camera,
        atlas: &mandrake_render::TextureAtlas,
        frame: mandrake_view::model_surfaces::WorldFrame,
        reflection_tint: [u8; 8],
    ) -> Vec<BillboardInstance> {
        let Some(sprites) = self.sprites.as_ref() else {
            return Vec::new();
        };
        let mut out = sprites.instances(
            &self.surfaces,
            atlas,
            frame,
            camera.view_axes(),
            &|_, index, _| index,
            (Default::default(), &|_| reflection_tint),
        );
        crate::tfl_objects::at_bucket_depth(&mut out);
        out
    }
}
