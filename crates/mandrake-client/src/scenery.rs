use std::collections::BTreeMap;
use std::path::Path;

use mandrake_common::FourCC;
use mandrake_sim::engine_rng::{self, GameParameters};
use mandrake_tag::{CollectionSequence, MeshTag, TagArchive, view_for_relative_angle};
use mandrake_view::billboard::BillboardInstance;
use tracing::{info, warn};

use mandrake_sim::{MapSource, SetupRole, Simulation};

use crate::world_view::ViewScenery;

use crate::media::{CoreRef, CoreTint};
use crate::unit_anim::FrameVisual;

pub const DEFAULT_GAME_SEED: u32 = mandrake_sim::players::REFERENCE_COMMAND_LINE_SEED;

pub fn with_slot0_tint(
    sprite: BillboardInstance,
    core: CoreRef,
    global: CoreTint,
) -> BillboardInstance {
    let tint = core.resolve(global);
    let shade = mandrake_view::media::slot0_tint(tint.fraction, tint.color);
    if shade == mandrake_view::media::SLOT0_IDENTITY {
        sprite
    } else {
        sprite.with_shade(shade)
    }
}

pub fn with_slot_tint(
    sprite: BillboardInstance,
    slot: u8,
    core: CoreRef,
    global: CoreTint,
    slot3: mandrake_view::media::ShadingSlot,
) -> BillboardInstance {
    if slot == 0 {
        return with_slot0_tint(sprite, core, global);
    }
    let tint = core.resolve(global);
    sprite.with_shade(mandrake_view::media::slot_tint(
        slot,
        tint.fraction,
        tint.color,
        slot3,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneryObject {
    pub scale: u16,
    pub frame: usize,
    pub view: usize,
    pub tick: i8,
    pub permutation: i16,
    pub sequence: u8,
    pub kind: u8,
    pub flag_team: Option<i16>,
}

#[derive(Clone, Debug, Default)]
pub struct SceneryObjects {
    pub objects: Option<BTreeMap<usize, SceneryObject>>,
    pub media: Option<std::sync::Arc<crate::media::MapMedia>>,
    pub history: BTreeMap<usize, SceneryObject>,
    pub team_colours: std::sync::Arc<crate::team_colours::TeamColours>,
    pub generation: u64,
}

impl SceneryObjects {
    pub fn object(&self, marker: usize, base_scale: u16) -> Option<SceneryObject> {
        match &self.objects {
            Some(objects) => objects.get(&marker).copied(),
            None => Some(SceneryObject {
                permutation: 0,
                flag_team: None,
                scale: base_scale,
                frame: 0,
                view: 0,
                tick: 0,
                sequence: 0xff,
                kind: 1,
            }),
        }
    }

    pub fn drawn(
        &self,
        marker: usize,
        base_scale: u16,
        history_shape: bool,
    ) -> Option<SceneryObject> {
        let live = self.object(marker, base_scale)?;
        Some(match self.history.get(&marker) {
            Some(h) if history_shape => SceneryObject {
                sequence: h.sequence,
                frame: h.frame,
                tick: h.tick,
                view: h.view,
                ..live
            },
            _ => live,
        })
    }

    pub fn follow(&mut self, scenery: impl IntoIterator<Item = ViewScenery>) -> bool {
        let Some(objects) = self.objects.as_mut() else {
            return false;
        };
        let mut changed = !self.history.is_empty();
        self.history.clear();
        for s in scenery {
            if s.deleted {
                changed |= objects.remove(&s.marker).is_some();
            } else if let Some(object) = objects.get_mut(&s.marker) {
                let now = s.object;
                if now.flag_team != object.flag_team {
                    object.flag_team = now.flag_team;
                    changed = true;
                }
                if *object != now {
                    self.history.insert(s.marker, *object);
                    *object = now;
                    changed = true;
                }
            }
        }
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
        changed
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GameStart<'a> {
    pub film: Option<&'a [u8]>,
    pub seed: u32,
    pub params: GameParameters,
}

pub struct ClientGame {
    pub sim: Simulation,
    pub scenery: SceneryObjects,
    pub monsters: Option<Vec<engine_rng::EngineMonster>>,
    pub movement: Option<mandrake_sim::MovementLoad>,
    pub rng_at_start: Option<mandrake_math::BungieRng>,
}

impl std::fmt::Debug for ClientGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientGame")
            .field("units", &self.sim.units.len())
            .field("engine_stop", &self.sim.engine_stop)
            .finish()
    }
}

pub fn tag_lookup<'a>(
    archives: &'a [&'a TagArchive],
) -> impl Fn([u8; 4], FourCC) -> Option<Vec<u8>> + 'a {
    move |group, id| {
        archives.iter().rev().find_map(|a| {
            a.find_tag_by_id(FourCC::new(group), id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    }
}

fn log_movement(sim: &Simulation, load: &mandrake_sim::MovementLoad) {
    info!(
        "Engine movement model: {} of {} units attached, {} on the placeholder locomotion \
         (cells from the map load: {})",
        load.attached,
        sim.units.len(),
        load.missing.len(),
        load.cells_from_map_load
    );
    for (id, reason) in &load.missing {
        warn!("Unit {id} keeps the placeholder locomotion: {reason}");
    }
}

fn open_engine_tag_files(archive_path: &Path, plugins: &[&str]) -> Vec<TagArchive> {
    let vfs = mandrake_common::game_vfs();
    let Some(game_dir) = archive_path.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let files = mandrake_tag::engine_tag_files(game_dir, plugins);
    if !files.complete() && !files.paths.is_empty() {
        warn!(
            "Map load: not the engine's tag set (missing patches {:?}, plugins {:?})",
            files.missing_patches, files.missing_plugins
        );
    }
    files
        .paths
        .iter()
        .filter_map(|p| TagArchive::open(vfs, &p.to_string_lossy()).ok())
        .collect()
}

pub fn open_override_tag_files(
    archive_path: &Path,
    plugins: &[&str],
) -> (Vec<TagArchive>, Vec<String>) {
    let vfs = mandrake_common::game_vfs();
    let Some(game_dir) = archive_path.parent().and_then(Path::parent) else {
        return (Vec::new(), plugins.iter().map(|p| p.to_string()).collect());
    };
    let files = mandrake_tag::engine_tag_files(game_dir, plugins);
    let foundation = game_dir.join("tags");
    let opened = files
        .paths
        .iter()
        .filter(|p| !p.starts_with(&foundation))
        .filter_map(|p| TagArchive::open(vfs, &p.to_string_lossy()).ok())
        .collect();
    (opened, files.missing_plugins)
}

pub fn client_game(
    archive_path: &Path,
    plugins: &[&str],
    fallback: &[&TagArchive],
    mesh_id: FourCC,
    client_mesh: &MeshTag,
    start: GameStart<'_>,
    spawn_units: bool,
) -> ClientGame {
    client_game_on(
        start_simulation(&start),
        archive_path,
        plugins,
        fallback,
        mesh_id,
        client_mesh,
        start.params,
        spawn_units,
    )
}

fn start_simulation(start: &GameStart<'_>) -> Simulation {
    match start.film.map(Simulation::from_film) {
        Some(Ok(sim)) => sim,
        Some(Err(e)) => {
            warn!("Film not readable for the simulation ({e}); starting the -mesh game");
            Simulation::command_line_game(start.seed)
        }
        None => Simulation::command_line_game(start.seed),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn client_game_on(
    mut sim: Simulation,
    archive_path: &Path,
    plugins: &[&str],
    fallback: &[&TagArchive],
    mesh_id: FourCC,
    client_mesh: &MeshTag,
    params: GameParameters,
    spawn_units: bool,
) -> ClientGame {
    let opened = open_engine_tag_files(archive_path, plugins);
    let archives: Vec<&TagArchive> = if opened.is_empty() {
        fallback.to_vec()
    } else {
        opened.iter().collect()
    };
    let tags = tag_lookup(&archives);
    let mesh_data = tags(*b"mesh", mesh_id);
    let mesh = mesh_data
        .as_deref()
        .and_then(|d| MeshTag::from_bytes(d).ok())
        .unwrap_or_else(|| client_mesh.clone());
    let role = SetupRole::Game { live: false };
    let map = MapSource {
        mesh_data: mesh_data.as_deref().unwrap_or_default(),
        mesh: &mesh,
        tags: &tags,
        params,
    };
    let rng_at_start = sim.rng;
    let mut engine_load_seed = None;
    let mut scenery = SceneryObjects::default();
    let mut monsters = None;
    match mesh_data.as_deref() {
        None => {
            sim.command_lead = mandrake_sim::LOCAL_COMMAND_LEAD;
            sim.load_mesh(&mesh);
            warn!(
                "Map load: mesh '{mesh_id}' not found; scenery uses frame 0 and the base scale, \
                 units the obje lower bounds"
            )
        }
        Some(data) => {
            match crate::team_colours::with_custom_core_draws(|| sim.load_map(&map, role)) {
                (Ok(draws), custom_core_draws) => {
                    let world = &sim.engine;
                    info!(
                        "Engine map load: seed state {}, {} scenery objects, {} monsters, {} draws \
                     (scenery {}, monsters {}, map actions {})",
                        rng_at_start.state,
                        world.scenery.len(),
                        world.monsters.len(),
                        draws.total(),
                        draws.scenery,
                        draws.monsters,
                        draws.map_actions
                    );
                    if let Some(e) = &sim.engine_stop {
                        warn!("Engine model stopped in the map load's update_map_actions: {e}");
                    }
                    scenery = scenery_objects(&world.scenery, &world.flag_data);
                    scenery.team_colours =
                        std::sync::Arc::new(crate::team_colours::TeamColours::of_load(
                            world,
                            &mesh,
                            &custom_core_draws,
                            &|unit| crate::team_colours::core_permutations(&tags, unit),
                        ));
                    monsters = Some(world.monsters.clone());
                    engine_load_seed = Some(rng_at_start);
                }
                (Err(e), _) => {
                    warn!("Engine map load stopped ({e}): the game does not run");
                    sim.rng = rng_at_start;

                    match engine_rng::monster_load(&mut { rng_at_start }, data, &tags, params) {
                        Ok(load) => {
                            scenery = scenery_objects(&load.world.scenery, &load.world.flag_data);
                            monsters = Some(load.world.monsters);
                        }
                        Err(e) => {
                            warn!(
                                "Map load up to the monsters stopped ({e}); units use the obje lower bounds"
                            );
                            match engine_rng::scenery_load(
                                &mut { rng_at_start },
                                data,
                                &tags,
                                params,
                            ) {
                                Ok(load) => {
                                    scenery =
                                        scenery_objects(&load.world.scenery, &load.world.flag_data)
                                }
                                Err(e) => warn!(
                                    "Scenery load stopped ({e}); scenery uses frame 0 and the base scale"
                                ),
                            }
                        }
                    }
                }
            }
        }
    }
    let mut movement = None;
    if spawn_units {
        let population = sim.populate_map(&map, role, monsters.as_deref());
        info!(
            "Units: {} spawned ({} unit markers on the mesh)",
            population.spawned.len(),
            mesh.unit_markers().count()
        );
        log_movement(&sim, &population.movement);
        movement = Some(population.movement);
    }
    ClientGame {
        sim,
        scenery,
        monsters,
        movement,
        rng_at_start: engine_load_seed,
    }
}

fn scenery_objects(
    scenery: &[engine_rng::EngineScenery],
    flags: &mandrake_sim::flags::FlagData,
) -> SceneryObjects {
    SceneryObjects {
        media: None,
        history: BTreeMap::new(),
        team_colours: Default::default(),
        generation: 0,
        objects: Some(
            scenery
                .iter()
                .filter(|s| !s.deleted)
                .map(|s| (s.marker, crate::world_view::myth2::scenery_object(s, flags)))
                .collect(),
        ),
    }
}

pub fn unit_scale_factor(scale: u16, base_scale: u16) -> f32 {
    if base_scale == 0 {
        1.0
    } else {
        f32::from(scale) / f32::from(base_scale)
    }
}

pub const SCEN_FLAG_ANIMATING: u32 = 0x40;

#[derive(Clone, Debug)]
pub struct SceneryVisual {
    pub sequence: CollectionSequence,
    pub frames: Vec<FrameVisual>,
    pub object_bounds: mandrake_tag::SequenceWorldBounds,
    pub base_scale: u16,
    pub scen_flags: u32,
    pub sequence_index: i16,
    pub remains: Option<Box<SceneryVisual>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScenerySprite {
    pub atlas_index: usize,
    pub world_size: [f32; 2],
    pub flip_x: bool,
}

pub fn camera_relative_angle(object: (i32, i32), facing: u16, camera: (i32, i32)) -> u16 {
    mandrake_math::engine::arctangent(
        camera.0.wrapping_sub(object.0),
        camera.1.wrapping_sub(object.1),
    )
    .wrapping_sub(facing)
}

pub fn engine_ground_point(mesh: &MeshTag, render_xz: [f32; 2]) -> (i32, i32) {
    engine_ground_point_in(mesh.cell_width, mesh.cell_height, render_xz)
}

fn engine_ground_point_in(cell_width: u16, cell_height: u16, render_xz: [f32; 2]) -> (i32, i32) {
    let half_w = f32::from(cell_width) * 0.5;
    let half_d = f32::from(cell_height) * 0.5;
    (
        ((half_w - render_xz[0]) * mandrake_tag::MarkerEntry::WORLD_ONE).round() as i32,
        ((render_xz[1] + half_d) * mandrake_tag::MarkerEntry::WORLD_ONE).round() as i32,
    )
}

impl SceneryVisual {
    pub fn shape(&self, object: &SceneryObject) -> Option<&SceneryVisual> {
        let sequence = i16::from(object.sequence);
        if object.sequence == 0xff || sequence == self.sequence_index {
            return Some(self);
        }
        self.remains
            .as_deref()
            .filter(|r| r.sequence_index == sequence)
    }

    pub fn is_dynamic(&self, object: &SceneryObject) -> bool {
        (object.kind == 1 && self.animates()) || self.view_follows_camera()
    }

    pub fn animates(&self) -> bool {
        self.scen_flags & SCEN_FLAG_ANIMATING != 0
    }

    pub fn view_follows_camera(&self) -> bool {
        self.sequence.flags & 4 == 0 && self.sequence.num_views > 1
    }

    pub fn frame(&self, frame: usize) -> Option<&FrameVisual> {
        self.frames.get(frame % self.frames.len().max(1))
    }

    pub fn view(&self, object: &SceneryObject, relative: u16) -> usize {
        if self.sequence.flags & 4 != 0 {
            object.view
        } else {
            view_for_relative_angle(relative, usize::from(self.sequence.num_views), false)
        }
    }

    pub fn sprite(
        &self,
        object: &SceneryObject,
        frame: usize,
        view: usize,
    ) -> Option<ScenerySprite> {
        let (stored, flip_x) = self.sequence.resolve_view(view);
        let sprite = self.frame(frame)?.views.get(stored)?.as_ref()?;
        let k = f32::from(object.scale) / 256.0;
        Some(ScenerySprite {
            atlas_index: sprite.atlas_index,
            world_size: [sprite.world_size[0] * k, sprite.world_size[1] * k],
            flip_x,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimatingScenery {
    pub frame: i8,
    pub tick: i8,
    pub ticks_per_frame: i8,
    pub reverse: bool,
}

impl AnimatingScenery {
    pub fn new(object: &SceneryObject, sequence: &CollectionSequence) -> Self {
        let tpf = ((i32::from(sequence.ticks_per_frame) * 0x100) >> 8) as i8;
        Self {
            frame: object.frame as i8,
            tick: object.tick,
            ticks_per_frame: if tpf <= 0 { 1 } else { tpf },
            reverse: false,
        }
    }

    pub fn tick(&mut self, sequence: &CollectionSequence) {
        let tpf = i32::from(self.ticks_per_frame);
        if tpf <= 0 {
            return;
        }
        let tick = i32::from(self.tick);
        if tpf > tick {
            self.tick = (tick + 1) as i8;
            return;
        }
        let frames = i32::from(sequence.frames_per_view as i16);
        let frame = i32::from(self.frame);
        self.frame = if self.reverse {
            if frame - 1 < 0 {
                (frames - 1) as i8
            } else {
                (frame - 1) as i8
            }
        } else if frame + 1 < frames {
            (frame + 1) as i8
        } else {
            effective_loop_frame(sequence) as i8
        };
        self.tick = 0;
    }
}

fn effective_loop_frame(sequence: &CollectionSequence) -> i16 {
    let frames = sequence.frames_per_view as i16;
    if sequence.loop_frame < 0 || sequence.loop_frame >= frames {
        0
    } else {
        sequence.loop_frame
    }
}

#[derive(Clone, Debug, Default)]
pub struct SceneryAnimator {
    records: BTreeMap<usize, (FourCC, AnimatingScenery)>,
}

impl SceneryAnimator {
    pub fn new(
        mesh: &MeshTag,
        objects: &SceneryObjects,
        visuals: &BTreeMap<FourCC, SceneryVisual>,
    ) -> Self {
        let mut records = BTreeMap::new();
        for index in scenery_marker_indices(mesh) {
            let Some(tag) = mesh.palette_tag_for_marker(&mesh.markers[index]) else {
                continue;
            };
            let Some(visual) = visuals.get(&tag).filter(|v| v.animates()) else {
                continue;
            };
            if let Some(object) = objects.object(index, visual.base_scale) {
                records.insert(
                    index,
                    (tag, AnimatingScenery::new(&object, &visual.sequence)),
                );
            }
        }
        Self { records }
    }

    pub fn tick(&mut self, visuals: &BTreeMap<FourCC, SceneryVisual>) {
        for (tag, record) in self.records.values_mut() {
            if let Some(visual) = visuals.get(tag) {
                record.tick(&visual.sequence);
            }
        }
    }

    pub fn record(&self, index: usize) -> Option<&AnimatingScenery> {
        self.records.get(&index).map(|(_, r)| r)
    }

    pub fn frame(&self, index: usize, object: &SceneryObject) -> usize {
        match self.record(index) {
            Some(r) if object.kind == 1 => r.frame.max(0) as usize,
            _ => object.frame,
        }
    }

    pub fn count(&self) -> usize {
        self.records.len()
    }
}

pub fn scenery_marker_indices(mesh: &MeshTag) -> impl Iterator<Item = usize> + '_ {
    mesh.markers
        .iter()
        .enumerate()
        .filter(|(_, m)| m.marker_type == mandrake_tag::MarkerPaletteEntry::TYPE_SCENERY)
        .map(|(i, _)| i)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::unit_anim::{ShadowSprite, ViewSprite};
    use mandrake_tag::SequenceBounds;

    #[test]
    fn slot0_tint_of_a_sprite() {
        use mandrake_view::billboard::BILLBOARD_FLAG_SHADE;
        let sprite = BillboardInstance::default();
        let fesp = CoreRef {
            tint: CoreTint {
                fraction: 0xd9,
                color: [10093, 21654, 22937],
            },
            global: false,
        };
        let global = CoreTint {
            fraction: 0x65,
            color: [0x322c, 0x3d70, 0x3126],
        };
        let own = with_slot0_tint(sprite, fesp, global);
        assert_ne!(own.flags & BILLBOARD_FLAG_SHADE, 0);
        assert_eq!(own.shade_kpqr[0], f32::from(0x26u8) / 255.0);
        assert_eq!(own.shade_xyz[2], f32::from(0x4bu8) / 255.0);

        let taken = CoreRef {
            global: true,
            ..fesp
        };
        assert_eq!(
            with_slot0_tint(sprite, taken, global).shade_kpqr[0],
            f32::from(0x9au8) / 255.0
        );
        assert_eq!(with_slot0_tint(sprite, taken, CoreTint::default()), sprite);
        assert_eq!(with_slot0_tint(sprite, CoreRef::default(), global), sprite);
    }

    fn visual(flags: u32, views: u16) -> SceneryVisual {
        let frames = (0..3)
            .map(|f| FrameVisual {
                views: (0..views as usize)
                    .map(|v| {
                        Some(ViewSprite {
                            atlas_index: f * 10 + v,
                            world_size: [1.0, 2.0],
                        })
                    })
                    .collect(),
                shadow: Some(ShadowSprite {
                    flags: 0,
                    offset_x: 0,
                    offset_y: 0,
                    width: 1,
                    height: 1,
                    pixels: vec![f as u8].into(),
                }),
                shadow_sources: Vec::new(),
            })
            .collect();
        SceneryVisual {
            sequence: CollectionSequence {
                index: 0,
                name: "s".into(),
                flags,
                pixels_to_world: 1,
                num_views: views,
                frames_per_view: 3,
                ticks_per_frame: 4,
                key_frame: 0,
                loop_frame: 0,
                bitmap_indices: vec![0; 3 * views as usize],
                shadow_indices: vec![0; 3],
                bounds: SequenceBounds::default(),
            },
            frames,
            object_bounds: Default::default(),
            base_scale: 0xe5,
            scen_flags: 0,
            sequence_index: 0,
            remains: None,
        }
    }

    #[test]
    fn sprite_takes_the_engine_frame_and_scale() {
        let v = visual(2, 1);
        let object = SceneryObject {
            permutation: 0,
            flag_team: None,
            sequence: 0xff,
            kind: 1,
            scale: 0x180,
            frame: 2,
            view: 0,
            tick: 0,
        };
        let s = v.sprite(&object, object.frame, 0).expect("sprite");
        assert_eq!(s.atlas_index, 20);
        assert_eq!(s.world_size, [1.5, 3.0]);
        assert!(!s.flip_x);
        assert_eq!(
            v.frame(object.frame)
                .and_then(|f| f.shadow.clone())
                .map(|s| s.pixels[0]),
            Some(2)
        );
    }

    #[test]
    fn stored_view_only_for_sequence_flag_4() {
        let object = SceneryObject {
            permutation: 0,
            flag_team: None,
            sequence: 0xff,
            kind: 1,
            scale: 0x100,
            frame: 1,
            view: 3,
            tick: 0,
        };
        let v = visual(6, 8);
        assert_eq!(v.view(&object, 0x4000), 3);
        assert_eq!(v.sprite(&object, 1, 3).map(|s| s.atlas_index), Some(13));
        let v = visual(2, 8);
        assert!(v.view_follows_camera());
        assert_eq!(v.view(&object, 0), 0);
        assert_eq!(v.view(&object, 0x4000), 2);
        assert!(!visual(2, 1).view_follows_camera());
        assert!(!visual(6, 8).view_follows_camera());
    }

    #[test]
    fn camera_angle_view_matches_the_engine_rule() {
        let v = visual(2, 8);
        let object = SceneryObject {
            permutation: 0,
            flag_team: None,
            sequence: 0xff,
            kind: 1,
            scale: 0x100,
            frame: 0,
            view: 5,
            tick: 0,
        };
        assert_eq!(camera_relative_angle((1000, 1000), 0, (5000, 1000)), 0);
        assert_eq!(camera_relative_angle((1000, 1000), 0, (1000, 5000)), 0x4000);
        assert_eq!(
            camera_relative_angle((1000, 1000), 0, (-3000, 1000)),
            0x8000
        );
        assert_eq!(
            camera_relative_angle((1000, 1000), 0, (1000, -3000)),
            0xc000
        );
        assert_eq!(camera_relative_angle((1000, 1000), 0x4000, (1000, 5000)), 0);
        let views: Vec<usize> = (0..16u32)
            .map(|k| v.view(&object, (k * 0x1000) as u16))
            .collect();
        assert_eq!(views, [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 0]);
        assert_eq!(v.view(&object, 0xefff), 7);
        assert_eq!(v.view(&object, 0xf000), 0);
    }

    #[test]
    fn engine_ground_point_inverts_the_marker_placement() {
        let (x, z) = (48.0 - 24512.0 / 512.0, 18140.0 / 512.0 - 48.0);
        assert_eq!(engine_ground_point_in(96, 96, [x, z]), (24512, 18140));
    }

    #[test]
    fn animating_scenery_steps_every_ticks_per_frame_plus_one() {
        let mut v = visual(2, 1);
        v.sequence.frames_per_view = 5;
        v.sequence.ticks_per_frame = 3;
        v.sequence.loop_frame = 2;
        v.frames = (0..5).map(|_| v.frames[0].clone()).collect();
        let object = SceneryObject {
            permutation: 0,
            flag_team: None,
            sequence: 0xff,
            kind: 1,
            scale: 0x100,
            frame: 3,
            view: 0,
            tick: 1,
        };
        let mut r = AnimatingScenery::new(&object, &v.sequence);
        assert_eq!(
            (r.frame, r.tick, r.ticks_per_frame, r.reverse),
            (3, 1, 3, false)
        );
        let mut frames = Vec::new();
        for _ in 0..20 {
            r.tick(&v.sequence);
            frames.push(r.frame);
        }
        let expect = |n: usize| -> i8 { [3i8, 4, 2][((1 + n) / 4) % 3] };
        let closed: Vec<i8> = (1..=20).map(expect).collect();
        assert_eq!(frames, closed);
        assert_eq!(&frames[..8], &[3, 3, 4, 4, 4, 4, 2, 2]);
    }

    #[test]
    fn animating_scenery_backwards_wraps_to_the_last_frame() {
        let mut v = visual(2, 1);
        v.sequence.frames_per_view = 3;
        v.sequence.ticks_per_frame = 0;
        let mut r = AnimatingScenery::new(
            &SceneryObject {
                permutation: 0,
                flag_team: None,
                sequence: 0xff,
                kind: 1,
                scale: 0x100,
                frame: 0,
                view: 0,
                tick: 0,
            },
            &v.sequence,
        );
        assert_eq!(r.ticks_per_frame, 1);
        r.reverse = true;
        let frames: Vec<i8> = (0..6)
            .map(|_| {
                r.tick(&v.sequence);
                r.frame
            })
            .collect();
        assert_eq!(frames, [0, 2, 2, 1, 1, 0]);
    }

    #[test]
    fn unit_scale_factor_is_relative_to_the_decoded_scale() {
        assert_eq!(unit_scale_factor(0x100, 0x100), 1.0);
        assert_eq!(unit_scale_factor(0x180, 0x100), 1.5);
        assert_eq!(unit_scale_factor(0x180, 0), 1.0);
    }

    #[test]
    fn without_a_model_every_marker_gets_the_base_values() {
        let none = SceneryObjects::default();
        assert_eq!(
            none.object(7, 0xe5),
            Some(SceneryObject {
                permutation: 0,
                flag_team: None,
                sequence: 0xff,
                kind: 1,
                scale: 0xe5,
                frame: 0,
                view: 0,
                tick: 0,
            })
        );
        let some = SceneryObjects {
            media: None,
            history: BTreeMap::new(),
            team_colours: Default::default(),
            generation: 0,
            objects: Some(BTreeMap::from([(
                3,
                SceneryObject {
                    permutation: 0,
                    flag_team: None,
                    sequence: 0xff,
                    kind: 1,
                    scale: 0x111,
                    frame: 4,
                    view: 0,
                    tick: 0,
                },
            )])),
        };
        assert_eq!(some.object(7, 0xe5), None);
        assert_eq!(some.object(3, 0xe5).map(|o| o.frame), Some(4));
    }

    #[test]
    fn remains_are_drawn_from_their_own_sequence_after_half_a_tick() {
        let intact = SceneryObject {
            scale: 0x100,
            frame: 1,
            view: 0,
            tick: 0,
            permutation: 0,
            flag_team: None,
            sequence: 0,
            kind: 1,
        };
        let remains = SceneryObject {
            frame: 2,
            sequence: 22,
            kind: 0,
            ..intact
        };
        let mut v = visual(2, 1);
        v.scen_flags = SCEN_FLAG_ANIMATING;
        assert!(v.shape(&remains).is_none(), "no remains decoded");
        let mut r = visual(6, 1);
        r.sequence_index = 22;
        v.remains = Some(Box::new(r));
        assert_eq!(v.shape(&intact).unwrap().sequence_index, 0);
        assert_eq!(v.shape(&remains).unwrap().sequence_index, 22);
        assert_eq!(
            v.shape(&SceneryObject {
                sequence: 0xff,
                ..intact
            })
            .unwrap()
            .sequence_index,
            0
        );
        assert!(
            v.shape(&SceneryObject {
                sequence: 3,
                ..intact
            })
            .is_none()
        );
        assert!(v.is_dynamic(&intact));
        assert!(!v.is_dynamic(&remains));
        let mut animator = SceneryAnimator::default();
        animator.records.insert(
            7,
            (
                FourCC::new(*b"test"),
                AnimatingScenery {
                    frame: 0,
                    tick: 0,
                    ticks_per_frame: 1,
                    reverse: false,
                },
            ),
        );
        assert_eq!(animator.frame(7, &intact), 0);
        assert_eq!(animator.frame(7, &remains), 2);

        let objects = SceneryObjects {
            media: None,
            objects: Some(BTreeMap::from([(7, remains)])),
            history: BTreeMap::from([(7, intact)]),
            team_colours: Default::default(),
            generation: 0,
        };
        let drawn = objects.drawn(7, 0x100, true).unwrap();
        assert_eq!((drawn.sequence, drawn.frame, drawn.kind), (0, 1, 0));
        assert_eq!(objects.drawn(7, 0x100, false), Some(remains));
        assert_eq!(objects.drawn(8, 0x100, true), None);
    }
}
