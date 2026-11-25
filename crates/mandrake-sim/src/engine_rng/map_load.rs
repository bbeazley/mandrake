use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameParameters {
    pub mode: i16,
    pub difficulty: i16,

    pub gameplay_version: i32,
}

impl GameParameters {
    pub const COMMAND_LINE: Self = Self {
        mode: 2,
        difficulty: 0,
        gameplay_version: crate::gameplay::GAMEPLAY_1_8_5,
    };

    pub const fn solo(difficulty: i16) -> Self {
        Self {
            mode: 2,
            difficulty,
            gameplay_version: crate::gameplay::GAMEPLAY_1_8_5,
        }
    }

    pub fn from_block(block: &[u8]) -> Option<Self> {
        (block.len() >= 0x12).then(|| Self {
            mode: be_i16(block, 0x00),
            difficulty: be_i16(block, 0x10),
            gameplay_version: crate::gameplay::gameplay_version_of_block(block)
                .unwrap_or(crate::gameplay::GAMEPLAY_1_8_5),
        })
    }

    pub fn from_film(film: &[u8]) -> Option<Self> {
        Self::from_block(mandrake_rec::header::game_parameters(film)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayGame {
    pub mode: i16,
    pub solo: bool,
}

impl ReplayGame {
    pub fn new(block_mode: i16, options: u32) -> Self {
        Self {
            mode: block_mode,
            solo: block_mode == 2 || options & mandrake_common::game_options::COOPERATIVE != 0,
        }
    }

    pub fn apply_to_options(&self, options: u32) -> u32 {
        if self.solo {
            options
        } else {
            options & !mandrake_common::game_options::LIMITED_VISIBILITY
        }
    }

    pub fn apply_to_setup(&self, setup: &mut crate::players::GameSetup) {
        setup.options = self.apply_to_options(setup.options);
        if !self.solo {
            setup.local_player = -1;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapLoadDraws {
    pub fire: u64,
    pub wind: u64,
    pub particles: u64,
    pub scenery: u64,
    pub scenery_shape: u64,
    pub monsters: u64,
    pub projectiles: u64,
    pub model_animations: u64,
    pub map_actions: u64,
}

impl MapLoadDraws {
    pub fn total(&self) -> u64 {
        self.fire
            + self.wind
            + self.scenery
            + self.monsters
            + self.projectiles
            + self.model_animations
            + self.map_actions
    }
}

#[derive(Debug, Clone)]
pub struct MapLoad {
    pub world: EngineWorld,
    pub draws: MapLoadDraws,
    pub sites: SiteCounts,
    pub unmodelled: Vec<Unmodelled>,
    pub assumptions: Vec<Unmodelled>,
}

pub fn map_load(
    rng: &mut BungieRng,
    mesh_data: &[u8],
    tags: &dyn TagSource,
    params: GameParameters,
) -> Result<MapLoad, Unmodelled> {
    map_load_with_progress(rng, mesh_data, tags, params, &mut MapLoadDraws::default())
}

#[derive(Debug, Clone, Default)]
pub struct MapActionOptions {
    pub skip_unimplemented: bool,
    pub bridged: HashMap<usize, Vec<u32>>,
    pub game_options_high: u16,
    pub game_type: Option<i16>,
    pub setup: Option<crate::players::GameSetup>,
    pub replay: Option<ReplayGame>,
    pub time_limit: i32,
    pub planning_time: i32,
    pub monster_initializers: Vec<mandrake_rec::header::MonsterInitializer>,
    pub permanent_rankings: Vec<crate::statistics::PermanentRanking>,
    pub option_flags: u32,
}

pub const COMMAND_LINE_GAME_TYPE: i16 = 13;
pub const SOLO_GAME_TYPE: i16 = 13;

pub fn map_load_with_options(
    rng: &mut BungieRng,
    mesh_data: &[u8],
    tags: &dyn TagSource,
    params: GameParameters,
    options: &MapActionOptions,
) -> Result<MapLoad, Unmodelled> {
    load_stages_with(
        rng,
        mesh_data,
        tags,
        params,
        &mut MapLoadDraws::default(),
        LoadStages::All,
        options,
    )
}

pub fn map_load_with_progress(
    rng: &mut BungieRng,
    mesh_data: &[u8],
    tags: &dyn TagSource,
    params: GameParameters,
    draws: &mut MapLoadDraws,
) -> Result<MapLoad, Unmodelled> {
    load_stages(rng, mesh_data, tags, params, draws, LoadStages::All)
}

pub fn scenery_load(
    rng: &mut BungieRng,
    mesh_data: &[u8],
    tags: &dyn TagSource,
    params: GameParameters,
) -> Result<MapLoad, Unmodelled> {
    load_stages(
        rng,
        mesh_data,
        tags,
        params,
        &mut MapLoadDraws::default(),
        LoadStages::Scenery,
    )
}

pub fn monster_load(
    rng: &mut BungieRng,
    mesh_data: &[u8],
    tags: &dyn TagSource,
    params: GameParameters,
) -> Result<MapLoad, Unmodelled> {
    load_stages(
        rng,
        mesh_data,
        tags,
        params,
        &mut MapLoadDraws::default(),
        LoadStages::Monsters,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadStages {
    Scenery,
    Monsters,
    All,
}

fn load_stages(
    rng: &mut BungieRng,
    mesh_data: &[u8],
    tags: &dyn TagSource,
    params: GameParameters,
    draws: &mut MapLoadDraws,
    stages: LoadStages,
) -> Result<MapLoad, Unmodelled> {
    load_stages_with(
        rng,
        mesh_data,
        tags,
        params,
        draws,
        stages,
        &MapActionOptions::default(),
    )
}

fn load_stages_with(
    rng: &mut BungieRng,
    mesh_data: &[u8],
    tags: &dyn TagSource,
    params: GameParameters,
    draws: &mut MapLoadDraws,
    stages: LoadStages,
    options: &MapActionOptions,
) -> Result<MapLoad, Unmodelled> {
    *draws = MapLoadDraws::default();
    if !matches!(
        params.gameplay_version,
        GAMEPLAY_VERSION_1_8_3 | crate::gameplay::GAMEPLAY_1_8_5
    ) {
        return Err(Unmodelled::new(
            draw_site::GAMEPLAY_VERSION,
            format!("gameplay version 0x{:x}", params.gameplay_version),
        ));
    }
    let mesh = MeshTag::from_bytes(mesh_data)
        .map_err(|e| Unmodelled::new(draw_site::MESH_HEADER, format!("mesh: {e}")))?;
    let game_mode = if options.replay.is_some() {
        0
    } else {
        params.mode
    };
    let mut sites = SiteCounts::default();
    let mut world = EngineWorld {
        mesh_cells: (i32::from(mesh.cell_width), i32::from(mesh.cell_height)),
        cells: MeshCells::from_mesh(&mesh),
        heights: mesh
            .cells
            .iter()
            .map(|c| (c.physical_height, c.media_height))
            .collect(),
        media: mandrake_tag::mesh::MediaCoverage::from_mesh_bytes(
            mesh_data,
            mesh.cell_width,
            mesh.cell_height,
        ),
        mesh_flags: be_i32(mesh_data, 0x4c) as u32,
        mesh_ceiling: be_i16(mesh_data, 0x70),
        media_splash: tags.tag(*b"medi", mesh.media_tag).and_then(|d| {
            mandrake_tag::media::MediaDefinition::from_bytes(mesh.media_tag, &d)
                .ok()
                .map(|m| m.projectile_group(mandrake_tag::media::MEDIA_GROUP_MONSTER))
        }),
        difficulty: params.difficulty,
        mode: game_mode,
        replay: options.replay,
        game_type: options.game_type.unwrap_or(COMMAND_LINE_GAME_TYPE),
        formation_globals: crate::formations::FormationGlobals {
            game_options_high: options.game_options_high,
            gameplay_version: params.gameplay_version,
        },
        ..EngineWorld::default()
    };
    let mesh_actions = MeshMapActions::from_mesh_bytes(mesh_data).ok();
    if stages == LoadStages::All {
        let actions = mesh_actions
            .as_ref()
            .ok_or_else(|| Unmodelled::new(draw_site::MESH_MAP_ACTIONS, "mesh map actions"))?;
        let mut actions = MapActions::from_mesh(actions);
        actions.skip_unimplemented = options.skip_unimplemented;
        actions.bridged = options.bridged.clone();
        world.map_actions = Some(actions);
    }
    let mut setup = options.setup.clone().unwrap_or_else(|| {
        crate::players::GameSetup::command_line(
            crate::players::COMMAND_LINE_GAME_OPTIONS,
            crate::players::REFERENCE_COMMAND_LINE_SEED,
        )
    });
    if let Some(r) = options.replay {
        r.apply_to_setup(&mut setup);
    }
    world.mons_postprocess = crate::mons_postprocess::MonsPostprocess {
        mode: game_mode,
        options: setup.options,
        gameplay_version: params.gameplay_version,
    };
    let postprocessed = crate::mons_postprocess::PostprocessedTags {
        tags,
        postprocess: world.mons_postprocess,
    };
    let tags: &dyn TagSource = &postprocessed;
    let mut assumptions = Vec::new();
    let marker_stops;
    if !setup.complete {
        assumptions.push(Unmodelled::new(
            draw_site::PLAYER_ENTRIES_ASSUMED,
            "player entries from RecordingHeader: +0x04, +0x08, +0x0c and the team seed (+0x14) taken as 0, -1, 0, 0 (use GameSetup::from_film)",
        ));
    }
    {
        let mut r = EngineRng {
            rng,
            sites: &mut sites,
        };

        let team_count =
            mesh.palettes
                .iter()
                .filter(|p| p.marker_type == 3)
                .fold(0i32, |n, p| n.max(i32::from(p.team_index()) + 1)) as usize;
        let reserved = crate::players::reserved_teams(mesh_actions.as_ref(), world.game_type);
        let players = crate::players::initialize_players(
            &mut setup,
            team_count,
            reserved,
            params.gameplay_version,
            &mut r,
        )?;

        let before = r.sites.total();
        let (fire, wind) = fire_and_wind_for_new_map(mesh_data, tags, &mut r)?;
        draws.fire = u64::from(fire.is_some());
        draws.wind = r.sites.total() - before - draws.fire;
        world.fire_timer = fire;
        world.wind = wind;

        let before = r.sites.total();
        world.particles = particle_system_for_new_map(mesh_data, tags, &mut r)?;
        draws.particles = r.sites.total() - before;

        let mut ctx = MarkerContext {
            mesh: &mesh,
            tags,
            params,
            monster_def_index: Default::default(),
            game_type: world.game_type,
            game_options: setup.options,
            mode: world.mode,
            replay: world.replay,
            players,
            unit_palettes: Vec::new(),
            row_entry_player: None,
            stops: Vec::new(),
            after_load: false,
        };
        world.players = ctx.players.clone();
        world.local_player = setup.local_player;
        world.setup_options = setup.options;
        assumptions.extend(ctx.equalize_palette_collections(&mut world)?);
        let before = r.sites.total();
        for (i, m) in mesh.markers.iter().enumerate() {
            if m.marker_type == 1 {
                ctx.new_scenery(&mut world, i, m, &mut r)?;
            }
        }
        draws.scenery = r.sites.total() - before;
        draws.scenery_shape = r.sites.at(&[
            draw_site::OBJECT_SHAPE_VIEW,
            draw_site::OBJECT_SHAPE_FRAME,
            draw_site::OBJECT_SHAPE_TICK,
            draw_site::OBJECT_SHAPE_LIMIT,
        ]);
        if stages == LoadStages::Scenery {
            return Ok(MapLoad {
                world,
                draws: draws.clone(),
                sites: r.sites.clone(),
                unmodelled: Vec::new(),
                assumptions: assumptions.clone(),
            });
        }
        ctx.resolve_unit_palettes(&mut world, &mut r)?;
        let keep = options.option_flags & mandrake_common::game_options::CARRY_VETERANS != 0;
        world.statistics = crate::statistics::Statistics::new_map(
            &ctx.players,
            if keep {
                &options.permanent_rankings
            } else {
                &[]
            },
        );
        world.monster_initializers = crate::veterans::MonsterInitializers::for_new_map(
            &options.monster_initializers,
            options.option_flags,
        );
        world.statistics.new_palette_entries(
            mesh.palettes
                .iter()
                .filter(|p| p.marker_type == 3)
                .map(|p| crate::statistics::PaletteUnit {
                    tag: u32::from_be_bytes(p.tag_id.0),
                    mons: ctx
                        .tags
                        .tag(*b"unit", p.tag_id)
                        .and_then(|u| u.get(..4).map(|m| [m[0], m[1], m[2], m[3]]))
                        .map(u32::from_be_bytes),
                    flags: p.flags as u16,
                    team: p.team_index(),
                }),
            &ctx.players,
            ctx.game_type,
        );
        world.game_options = options.option_flags;
        load_projectile_catalog(&mut world, &mesh, mesh_actions.as_ref(), tags);
        ctx.load_artifact_definitions(&mut world)?;
        let before = r.sites.total();
        world.unit_markers = crate::geometry_filter::unit_markers(&mesh);
        for m in mesh.markers.iter().filter(|m| m.marker_type == 3) {
            ctx.new_monster(&mut world, m, &mut r)?;
        }
        draws.monsters = r.sites.total() - before;
        if stages == LoadStages::Monsters {
            return Ok(MapLoad {
                world,
                draws: draws.clone(),
                sites: r.sites.clone(),
                unmodelled: Vec::new(),
                assumptions: assumptions.clone(),
            });
        }
        world.sound_sources =
            crate::lightning_action::sound_sources(&mesh.markers, &mesh.palettes, &|t| {
                tags.tag(*b"amso", t).is_some()
            });
        let mut models = 0i16;
        for m in mesh.markers.iter().filter(|m| m.marker_type == 6) {
            ctx.new_model_from_marker(&mut world, m, &mut models)?;
        }
        let before = r.sites.total();
        for (i, m) in mesh.markers.iter().enumerate() {
            if m.marker_type == 9 {
                let words = marker_words(mesh_data, &mesh, i);
                ctx.new_projectile_from_marker(&mut world, i, m, words, &mut r)?;
            }
        }
        draws.projectiles = r.sites.total() - before;
        let before = r.sites.total();
        for (i, m) in mesh.markers.iter().enumerate() {
            if m.marker_type == 11 {
                ctx.new_model_animation(&mut world, i, m, &mut models, &mut r)?;
            }
        }
        draws.model_animations = r.sites.total() - before;
        world.marker_creation = Some(std::sync::Arc::new(MarkerCreation::capture(&ctx)));
        world.monster_def_index = std::mem::take(&mut ctx.monster_def_index);
        marker_stops = ctx.stops;
    }
    create_connectors(&mut world, &mesh, mesh_data, tags)?;
    world.game = crate::game_state::GameState::new(options.time_limit, options.planning_time);
    world.update_statistics(
        0,
        0,
        Some(&mut EngineRng {
            rng,
            sites: &mut sites,
        }),
    );
    let mut unmodelled = marker_stops;
    unmodelled.extend(world.statistics_stop.take());
    {
        let mut r = EngineRng {
            rng,
            sites: &mut sites,
        };
        let before = r.sites.total();
        if let Err(e) = world.update_map_actions(0, &mut r) {
            unmodelled.push(e);
        }
        draws.map_actions = r.sites.total() - before;
    }
    if let Some(d) = tags
        .tag(*b"form", FourCC::new(*b"info"))
        .and_then(|d| crate::formations::FormationDefinition::from_tag(&d))
    {
        world.formation_definition = d;
    }
    Ok(MapLoad {
        world,
        draws: draws.clone(),
        sites,
        unmodelled,
        assumptions,
    })
}

pub fn update_monster_impassability_for_cell(world: &mut EngineWorld, x: i32, y: i32) {
    let (x0, y0, x1, y1) = crate::passability::impassability_update_range(&world.cells, x, y);
    for i in 0..world.monsters.len() {
        let (px, py) = world.monsters[i].position;
        let (cx, cy) = (px >> 9, py >> 9);
        if cx < x0 || cx > x1 || cy < y0 || cy > y1 {
            continue;
        }
        let mask = world.monsters[i].impassable;
        world.monsters[i].neighbours = world.cells.neighbour_mask(mask, px, py);
        world.monsters[i].flags |= 0x20;
        world.impassability_updates.push(i);
    }
}

pub fn new_monster_marker_height(
    z: i32,
    mesh_height: i32,
    gameplay: i32,
    r: &mut EngineRng,
) -> i32 {
    let h = i32::from(mesh_height as i16);
    if z > h + 0x200 {
        z
    } else if gameplay == game_options::GAMEPLAY_1_5 {
        h + 0x600 + (i32::from(r.draw(draw_site::MARKER_HEIGHT)) >> 5)
    } else {
        h + 0x800
    }
}

pub fn create_connectors(
    world: &mut EngineWorld,
    mesh: &MeshTag,
    mesh_data: &[u8],
    tags: &dyn TagSource,
) -> Result<(), Unmodelled> {
    if mesh_data.len() < 0x124 {
        return Ok(());
    }
    let Some(conn) = tag_ref(mesh_data, 0x48) else {
        return Ok(());
    };
    if tags.tag(*b"conn", conn).is_none() {
        return Ok(());
    }
    let count = be_i32(mesh_data, 0x11c);
    let base = 1024 + be_i32(mesh_data, 0x120) as usize;
    for c in 0..count.max(0) as usize {
        let rec = base + 64 * c;
        if rec + 64 > mesh_data.len() {
            return Err(Unmodelled::new(
                draw_site::CONNECTOR_OUTSIDE_MESH,
                format!("connector {c} outside the mesh"),
            ));
        }
        let n = be_i16(mesh_data, rec + 0x3e).clamp(0, 31) as usize;
        let mut posts: Vec<Option<(usize, (i32, i32))>> = Vec::with_capacity(n);
        for k in 0..n {
            let id = be_i16(mesh_data, rec + 2 * k);
            let marker = mesh
                .markers
                .iter()
                .position(|m| m.marker_type == 1 && m.marker_id == id && id != -1);
            let post = match marker {
                Some(mi) => match world.scenery.iter().position(|s| s.marker == mi) {
                    Some(si) => {
                        let s = &world.scenery[si];
                        let m = &mesh.markers[mi];
                        if !s.deleted && s.object.kind == crate::flags::OBJECT_KIND_FLAG {
                            return Err(Unmodelled::new(
                                draw_site::CONNECTOR_FLAG_SCENERY,
                                format!("connector post {id}: flag-8 scenery (game-type flag)"),
                            ));
                        }
                        Some((si, (m.x, m.y)))
                    }
                    None => None,
                },
                None => None,
            };
            posts.push(post);
        }
        if posts.iter().all(Option::is_none) {
            continue;
        }
        for k in 0..n.saturating_sub(1) {
            if let (Some((sa, a)), Some((sb, b))) = (posts[k], posts[k + 1])
                && !world.scenery[sa].deleted
                && !world.scenery[sb].deleted
            {
                world.cells.mark_connector(a, b, true);
            }
        }
        let posts: Vec<(Option<usize>, (i32, i32))> = posts
            .iter()
            .map(|p| p.map_or((None, (0, 0)), |(si, xy)| (Some(si), xy)))
            .collect();
        world
            .connectors
            .push(crate::connectors::Connector::new(&posts));
    }
    Ok(())
}

pub fn fire_and_wind_for_new_map(
    mesh_data: &[u8],
    tags: &dyn TagSource,
    r: &mut EngineRng,
) -> Result<(Option<i32>, Option<EngineWind>), Unmodelled> {
    if mesh_data.len() < 0xe8 {
        return Err(Unmodelled::new(
            draw_site::MESH_HEADER,
            "mesh header too short",
        ));
    }
    let fire = (be_i32(mesh_data, 0x4c) as u32 & 0x0100_0000 != 0)
        .then(|| 0x78 + ((i32::from(r.draw(draw_site::NEW_MAP_FIRE_TIMER)) * 0x96) >> 16));
    let mut wind = None;
    if let Some(id) = tag_ref(mesh_data, 0xe4) {
        let w = tags.tag(*b"wind", id).ok_or_else(|| {
            Unmodelled::new(draw_site::WIND_MISSING, format!("wind {id} missing"))
        })?;
        if w.len() < 0x28 {
            return Err(Unmodelled::new(
                draw_site::WIND_TOO_SHORT,
                format!("wind {id} too short"),
            ));
        }
        if w.len() < 0x44 {
            return Err(Unmodelled::new(
                draw_site::WIND_TOO_SHORT,
                format!("wind {id} too short"),
            ));
        }
        let mut state = EngineWind {
            countdown: 0,
            period: be_i32(&w, 0x04),
            period_range: be_i32(&w, 0x08),
            rate: be_i32(&w, 0x0c),
            base: [0x10, 0x14, 0x18].map(|o| be_i32(&w, o)),
            range: [0x1c, 0x20, 0x24].map(|o| be_i32(&w, o)),
            velocity: [0; 3],
            target: [0; 3],
        };
        state.change(r, draw_site::NEW_MAP_WIND_PERIOD);
        state.velocity = state.target;
        wind = Some(state);
    }
    Ok((fire, wind))
}

pub fn particle_system_for_new_map(
    mesh_data: &[u8],
    tags: &dyn TagSource,
    r: &mut EngineRng,
) -> Result<Option<crate::particles::ParticleSystem>, Unmodelled> {
    let Some(id) = tag_ref(mesh_data, 0x50) else {
        return Ok(None);
    };
    let d = tags.tag(*b"part", id).ok_or_else(|| {
        Unmodelled::new(
            draw_site::PARTICLE_SYSTEM_MISSING,
            format!("part {id} missing"),
        )
    })?;
    let def = crate::particles::ParticleSystemDefinition::parse(&d).ok_or_else(|| {
        Unmodelled::new(
            draw_site::PARTICLE_SYSTEM_SHORT,
            format!("part {id} too short"),
        )
    })?;
    Ok(Some(crate::particles::ParticleSystem::new_for_map(def, r)))
}

pub(super) fn engine_palette<'a>(
    mesh: &'a MeshTag,
    m: &MarkerEntry,
) -> Option<&'a mandrake_tag::MarkerPaletteEntry> {
    if m.palette_index < 0 {
        return None;
    }
    mesh.palettes
        .iter()
        .filter(|p| p.marker_type == m.marker_type)
        .nth(m.palette_index as usize)
}

pub(super) struct MarkerContext<'a> {
    pub(super) stops: Vec<Unmodelled>,
    pub(super) mesh: &'a MeshTag,
    pub(super) tags: &'a dyn TagSource,
    pub(super) params: GameParameters,
    pub(super) monster_def_index: crate::entities::TagIdMap<usize>,
    pub(super) game_type: i16,
    pub(super) game_options: u32,
    pub(super) mode: i16,
    pub(super) replay: Option<ReplayGame>,
    pub(super) players: crate::players::Players,
    pub(super) row_entry_player: Option<i16>,
    pub(super) unit_palettes: Vec<UnitPalette>,
    pub(super) after_load: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitRow {
    pub owner: i16,
    pub custom_core: bool,
}

impl UnitRow {
    const OWN: Self = Self {
        owner: -1,
        custom_core: false,
    };
}

struct NewMonster<'v> {
    index: usize,
    identifier: i16,
    first_draw: u64,
    x: i32,
    y: i32,
    z: i32,
    facing: u16,
    palette_flags: u16,
    marker_flags: u32,
    marker: bool,
    di: usize,
    mons: FourCC,
    core_tag: FourCC,
    unit_tag: FourCC,
    row: UnitRow,
    owner: i16,
    team: i16,
    veteran: Option<&'v mandrake_rec::header::MonsterInitializer>,
    statistics: StatisticsRow,
    located: Option<(u32, u32)>,
}

enum StatisticsRow {
    Fixed(i16),
    Palette {
        palette: usize,
        tag: u32,
        entry_player: i16,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MonsterPlacement {
    pub(crate) adjustable: bool,
    pub(super) palette: i16,
    pub(crate) row: (u32, i16),
    pub(crate) placed: i16,
    pub(crate) minimum: i16,
    pub(crate) maximum: i16,
    pub(crate) cost: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UnitPalette {
    pub(super) defined: bool,
    pub(super) row: UnitRow,
    pub(super) player: i16,
    pub(super) team: i16,
}

impl MarkerContext<'_> {
    pub(super) fn placement_row(
        &mut self,
        world: &mut EngineWorld,
        tag: FourCC,
        entry: UnitPalette,
    ) -> Result<((u32, i16), i16), Unmodelled> {
        let unit = self
            .tags
            .tag(*b"unit", tag)
            .filter(|u| entry.defined && u.len() >= 8)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::PLACEMENT_NO_ROW,
                    format!("unit placement: palette {tag} has no row"),
                )
            })?;
        let definition = self.monster_definition(world, fourcc(&unit, 0))?;
        let cost = world.monster_definitions[definition].ai.word(0x1f6);
        Ok(((u32::from_be_bytes(tag.0), entry.row.owner), cost))
    }

    fn object_definition(&self, tag: FourCC, site: u32) -> Result<Vec<u8>, Unmodelled> {
        let o = self
            .tags
            .tag(*b"obje", tag)
            .ok_or_else(|| Unmodelled::new(site, format!("obje {tag} missing")))?;
        if o.len() < 0x10 {
            return Err(Unmodelled::new(site, format!("obje {tag} too short")));
        }
        Ok(o)
    }

    fn new_object(
        &self,
        world: &mut EngineWorld,
        obje: &[u8],
        collection: FourCC,
        scale: i32,
        position: (i32, i32, i32),
        r: &mut EngineRng,
    ) -> EngineObject {
        world.new_object_with(
            (be_u16(obje, 0), be_u16(obje, 0x08), be_i16(obje, 0x0a)),
            collection,
            scale,
            position,
            r,
        )
    }

    pub(super) fn palette_compatible(
        &self,
        p: &mandrake_tag::MarkerPaletteEntry,
        site: u32,
    ) -> Result<bool, Unmodelled> {
        let mask_test = || {
            let mask = match self.game_type {
                14 => 0x200,
                15 => 0x80,
                t => 1u32.checked_shl(t as u32).unwrap_or(0),
            };
            let types = u32::from_be_bytes([
                p.user_data[4],
                p.user_data[5],
                p.user_data[6],
                p.user_data[7],
            ]);
            types & mask != 0
        };
        match self.mode {
            0 => match self.replay {
                Some(r) if r.solo => Ok(true),
                Some(_) => Ok(mask_test()),
                None => Err(Unmodelled::new(
                    site,
                    "palette_compatible_with_game_scoring in game mode 0 without a replay",
                )),
            },
            1 if self.game_options & mandrake_common::game_options::COOPERATIVE == 0 => {
                Ok(mask_test())
            }
            _ => Ok(true),
        }
    }

    fn resolve_unit_palettes(
        &mut self,
        world: &mut EngineWorld,
        r: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let palettes: Vec<_> = self
            .mesh
            .palettes
            .iter()
            .filter(|p| p.marker_type == 3)
            .collect();
        let mut out = Vec::with_capacity(palettes.len());
        for p in palettes {
            out.push(self.new_monster_palette_entry(
                world,
                p.tag_id,
                p.flags as u16,
                p.team_index(),
                r,
            )?);
        }
        self.unit_palettes = out;
        Ok(())
    }

    fn new_monster_palette_entry(
        &mut self,
        world: &mut EngineWorld,
        tag_id: FourCC,
        flags: u16,
        team: i16,
        r: &mut EngineRng,
    ) -> Result<UnitPalette, Unmodelled> {
        let unit_exists = self.tags.tag(*b"unit", tag_id).is_some();
        let tag = u32::from_be_bytes(tag_id.0);
        let mut e = UnitPalette {
            defined: false,
            row: UnitRow::OWN,
            player: -1,
            team,
        };
        if flags & 0x10 != 0 {
            e.defined = unit_exists;
            e.player = 0;
            e.team = self
                .players
                .players
                .first()
                .map_or(-1, |pl| i16::from(pl.team));
            if unit_exists {
                world.unit_rows.insert((tag, 0), UnitRow::OWN);
            }
        } else if team != -1 {
            let on_team: Vec<usize> = (0..self.players.players.len())
                .filter(|&j| i16::from(self.players.players[j].team) == team)
                .collect();
            for j in on_team {
                let row = self.customize_unit_definition(world, tag_id, j as i16, r)?;
                let claimed = self
                    .players
                    .teams
                    .get(team as usize)
                    .is_some_and(|t| t.player == j as i16);
                if claimed {
                    if flags & 1 != 0 && self.game_type == 13 {
                        e.defined = unit_exists;
                    } else {
                        e.defined = row.is_some();
                        e.row = row.unwrap_or(UnitRow::OWN);
                    }
                    e.player = j as i16;
                }
            }
        }
        Ok(e)
    }

    fn customize_unit_definition(
        &mut self,
        world: &mut EngineWorld,
        unit: FourCC,
        player: i16,
        r: &mut EngineRng,
    ) -> Result<Option<UnitRow>, Unmodelled> {
        let tag = u32::from_be_bytes(unit.0);
        if let Some(row) = world.unit_rows.get(&(tag, player)) {
            return Ok(Some(*row));
        }
        let Some(data) = self.tags.tag(*b"unit", unit) else {
            return Ok(None);
        };
        let colors = self
            .players
            .players
            .get(player as usize)
            .filter(|pl| pl.flags & 1 != 0)
            .and_then(|_| self.players.colors.get(player as usize));
        let core = colors
            .and(data.get(4..8))
            .and_then(|_| self.tags.tag(*b"core", fourcc(&data, 4)));
        let custom_core = match (core, colors) {
            (Some(core), Some(colors)) => {
                let collection = CollectionReference::from_bytes(&core)
                    .ok()
                    .and_then(|c| self.tags.tag(*b".256", c.collection_tag))
                    .ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::CUSTOM_CORE_NO_COLLECTION,
                            format!("core {unit}: no collection"),
                        )
                    })?;
                let hue_flags = crate::players::collection_hue_flags(&collection);
                crate::players::colors_replace_core(&core, &hue_flags, colors)
            }
            _ => false,
        };
        if custom_core {
            r.draw(draw_site::CUSTOM_CORE);
        }
        let row = UnitRow {
            owner: player,
            custom_core,
        };
        world.unit_rows.insert((tag, player), row);
        Ok(Some(row))
    }

    fn new_scenery(
        &mut self,
        world: &mut EngineWorld,
        marker: usize,
        m: &MarkerEntry,
        r: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let first_draw = r.sites.total();
        let Some(p) = engine_palette(self.mesh, m) else {
            return Ok(());
        };
        let Some(scen) = self.tags.tag(*b"scen", p.tag_id) else {
            return Ok(());
        };
        if !self.palette_compatible(p, draw_site::SCENERY_PALETTE)? {
            return Ok(());
        }
        if self.params.difficulty < m.difficulty_flags {
            return Ok(());
        }
        if scen.len() < 0x2a {
            return Err(Unmodelled::new(
                draw_site::SCENERY_SHORT,
                format!("scen {} too short", p.tag_id),
            ));
        }
        let flags = be_i32(&scen, 0) as u32;
        let core_tag = fourcc(&scen, 0x04);
        let core = self
            .tags
            .tag(*b"core", core_tag)
            .and_then(|c| CollectionReference::from_bytes(&c).ok())
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::SCENERY_CORE_MISSING,
                    format!("core {core_tag} missing"),
                )
            })?;
        let obje =
            self.object_definition(fourcc(&scen, 0x08), draw_site::SCENERY_OBJECT_DEFINITION)?;
        let perm = r.draw_mod(
            draw_site::SCENERY_PERMUTATION,
            i32::from(core.permutation_count),
        )?;
        let delta = i32::from(be_i16(&obje, 0x0e));
        let scale = i32::from(be_u16(&obje, 0x0c))
            + if delta > 0 {
                (i32::from(r.draw(draw_site::SCENERY_SCALE)) * delta) >> 16
            } else {
                0
            };
        if flags & 2 != 0 {
            r.draw(draw_site::SCENERY_FACING);
            r.draw(draw_site::SCENERY_PITCH);
        }
        world.load_collection(self.tags, core.collection_tag)?;
        world
            .collection_table0_cores
            .entry(core.collection_tag)
            .or_insert(core_tag);
        let mut obj = self.new_object(world, &obje, core.collection_tag, scale, (m.x, m.y, m.z), r);
        obj.flags |= 0x10;
        obj.permutation = perm as i16;
        obj.core = Some(core_tag);
        let headers = world.headers(core.collection_tag).to_vec();
        set_object_shape(
            &mut obj,
            &headers,
            i32::from(be_i16(&scen, 0x28)),
            0x100,
            0xf,
            r,
        )?;
        if flags & 0x80 != 0 {
            let seq = i32::from(be_i16(&scen, 0x28));
            let half_width = get_sequence(&headers, seq)
                .map(|h| i32::from(h.world_bounds.half_width))
                .ok_or_else(|| {
                    Unmodelled::new(
                        draw_site::SCENERY_SEQUENCE_MISSING,
                        format!("scen {}: sequence {seq} missing", p.tag_id),
                    )
                })?;
            let largest = i32::from(be_i16(&obje, 0x0c)) + i32::from(be_i16(&obje, 0x0e));
            let radius = ((largest * half_width) >> 8) as i16;
            world.cells.mark_scenery(m.x, m.y, radius);
        }
        if flags & 1 != 0 {
            obj.flags |= 0x400;
        }
        if flags & 0x10 != 0 {
            obj.flags |= 0x0400_0000;
        }
        obj.flags |= 0x100;
        if flags & 0x300 != 0 {
            obj.flags |= 0x0040_0100;
        }
        let projectile = tag_ref(&scen, 0x0c).filter(|t| self.tags.tag(*b"proj", *t).is_some());
        obj.kind = if projectile.is_some() { 4 } else { 1 };
        let cell_link = world.next_cell_link();
        let mut deleted = false;
        if flags & 8 != 0 {
            let flag_type = be_i16(&scen, 0x10);
            let g = self.game_type;
            let team = p.team_index();
            let team_ok = team == -1
                || usize::try_from(team)
                    .ok()
                    .and_then(|t| self.players.teams.get(t))
                    .is_some_and(|t| t.players != 0);
            if (g == flag_type || (g == 14 && flag_type == 9) || (g == 15 && flag_type == 7))
                && team_ok
            {
                let slot = world.flag_data.new_flag(
                    obj.index,
                    projectile.filter(|_| obj.kind == 4),
                    team,
                    be_i16(&scen, 0x12),
                    marker,
                    m.flags,
                    (m.x, m.y),
                );
                if obj.kind == 4 {
                    obj.flags |= 0x50_0000;
                }
                obj.kind = crate::flags::OBJECT_KIND_FLAG;
                obj.owner = slot;
            } else {
                deleted = true;
                world.free_objects.insert(obj.index);
            }
        }
        world.scenery.push(EngineScenery {
            marker,
            identifier: m.marker_id,
            first_draw,
            object: obj,
            position: (m.x, m.y),
            facing: m.yaw as u16,
            scenery_flags: flags,
            projectile,
            cell_link,
            deleted,
        });
        Ok(())
    }

    fn equalize_palette_collections(
        &self,
        world: &mut EngineWorld,
    ) -> Result<Vec<Unmodelled>, Unmodelled> {
        let mut assumptions = Vec::new();
        for p in self.mesh.palettes.iter().filter(|p| p.marker_type == 3) {
            let Some(unit) = self.tags.tag(*b"unit", p.tag_id).filter(|u| u.len() >= 8) else {
                continue;
            };
            let mons = fourcc(&unit, 0);
            let Some(data) = self.tags.tag(*b"mons", mons).filter(|d| d.len() >= 0x224) else {
                continue;
            };
            let collection = fourcc(&data, 4);
            if world.equalized.contains_key(&collection) {
                continue;
            }
            world.load_collection(self.tags, collection)?;
            let headers = world.collections.get_mut(&collection).expect("loaded");
            if let Some(a) =
                equalize_collection_for_palette(self.mesh, self.tags, collection, headers)?
            {
                assumptions.push(a);
            }
            world
                .equalized
                .insert(collection, (mons, be_i16(&data, 0x08)));
        }
        Ok(assumptions)
    }

    fn equalize_monster_collection(
        &self,
        world: &mut EngineWorld,
        mons: FourCC,
        data: &[u8],
    ) -> Result<(), Unmodelled> {
        let collection = fourcc(data, 4);
        let idle = be_i16(data, 0x08);
        let in_palette = self.mesh.palettes.iter().any(|p| {
            p.marker_type == 3
                && self
                    .tags
                    .tag(*b"unit", p.tag_id)
                    .is_some_and(|u| u.len() >= 8 && fourcc(&u, 0) == mons)
        });
        if in_palette {
            return Ok(());
        }
        if let Some(&(m, i)) = world.equalized.get(&collection)
            && i != idle
            && !self.equalized_in_any_order(collection, m, data)
        {
            return Err(Unmodelled::new(
                draw_site::EQUALIZE_COLLECTION_RECTANGLES,
                format!(
                    "collection {collection} equalized by mons {m} (idle {i}) and {mons} (idle {idle}): the order of the mons postprocess is not modelled"
                ),
            ));
        }
        world.load_collection(self.tags, collection)?;
        let headers = world.collections.get_mut(&collection).expect("loaded");
        equalize_monster_sequences(data, headers)
            .map_err(|e| Unmodelled::new(e.site, format!("mons {mons}: {}", e.reason)))?;
        world.equalized.entry(collection).or_insert((mons, idle));
        Ok(())
    }

    fn equalized_in_any_order(&self, collection: FourCC, first: FourCC, data: &[u8]) -> bool {
        let mut palette: Vec<Vec<u8>> = Vec::new();
        let mut seen = Vec::new();
        for p in self.mesh.palettes.iter().filter(|p| p.marker_type == 3) {
            let Some(unit) = self.tags.tag(*b"unit", p.tag_id).filter(|u| u.len() >= 8) else {
                continue;
            };
            let mons = fourcc(&unit, 0);
            if seen.contains(&mons) {
                continue;
            }
            seen.push(mons);
            if let Some(d) = self.tags.tag(*b"mons", mons).filter(|d| d.len() >= 0x224)
                && fourcc(&d, 4) == collection
            {
                palette.push(d);
            }
        }
        if !seen.contains(&first) {
            return false;
        }
        let Some(base) = self
            .tags
            .tag(*b".256", collection)
            .and_then(|d| mandrake_tag::sequence_headers(&d).ok())
        else {
            return false;
        };
        let run = |before: bool| {
            let mut h = base.clone();
            let mut order: Vec<&[u8]> = palette.iter().map(Vec::as_slice).collect();
            if before {
                order.insert(0, data);
            } else {
                order.push(data);
            }
            for m in order {
                equalize_monster_sequences(m, &mut h).ok()?;
            }
            Some(h.into_iter().map(|s| s.world_bounds).collect::<Vec<_>>())
        };
        matches!((run(true), run(false)), (Some(a), Some(b)) if a == b)
    }

    fn load_artifact_definitions(&mut self, world: &mut EngineWorld) -> Result<(), Unmodelled> {
        let names: Vec<FourCC> = world
            .catalog
            .artifacts
            .iter()
            .filter_map(|(_, a)| {
                let flags = be_i32(a, 0) as u32;
                let m = tag_ref(a, 0x98).filter(|_| flags & 4 != 0)?;
                Some(if flags & 0x8_0000 != 0 {
                    self.tags
                        .tag(*b"unit", m)
                        .filter(|u| u.len() >= 4)
                        .map_or(m, |u| fourcc(&u, 0))
                } else {
                    m
                })
            })
            .collect();
        for m in names {
            if self.tags.tag(*b"mons", m).is_some() {
                self.monster_definition(world, m)?;
            }
        }
        Ok(())
    }

    fn monster_definition(
        &mut self,
        world: &mut EngineWorld,
        mons: FourCC,
    ) -> Result<usize, Unmodelled> {
        if let Some(&i) = self.monster_def_index.get(&mons) {
            return Ok(i);
        }
        let data = self.tags.tag(*b"mons", mons).ok_or_else(|| {
            Unmodelled::new(
                draw_site::EQUALIZE_MONS_SHORT,
                format!("mons {mons} missing"),
            )
        })?;
        let mut def = MonsterDefinition::from_bytes(mons, &data)
            .map_err(|e| Unmodelled::new(draw_site::EQUALIZE_MONS_SHORT, e))?;
        if let Some(o) = self
            .tags
            .tag(*b"obje", def.object)
            .filter(|o| o.len() >= 0x0a)
        {
            def.object_vitality = be_i16(&o, 0x08);
            def.object_flags = u32::from(be_u16(&o, 0));
            if o.len() >= 0x32 {
                for (k, w) in def.object_modifiers.iter_mut().enumerate() {
                    *w = be_i16(&o, 0x10 + 2 * k);
                }
                def.object_threshold = be_i16(&o, 0x30);
            }
        }
        if let Some(names) = tag_ref(&data, 0x1ac).and_then(|n| self.tags.tag(*b"stli", n)) {
            let n = names
                .split(|&b| b == 0 || b == b'\n' || b == b'\r')
                .filter(|s| !s.is_empty())
                .count();
            def.name_count = n as i16;
        }
        self.equalize_monster_collection(world, mons, &data)?;
        world.monster_definitions.push(def);
        let i = world.monster_definitions.len() - 1;
        self.monster_def_index.insert(mons, i);
        Ok(i)
    }

    pub(super) fn new_monster(
        &mut self,
        world: &mut EngineWorld,
        m: &MarkerEntry,
        r: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let first_draw = r.sites.total();
        let Some(p) = engine_palette(self.mesh, m) else {
            return Ok(());
        };
        let palette_flags = p.flags as u16;
        let entry = *self
            .unit_palettes
            .get(m.palette_index as usize)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::MONSTER_PALETTE_UNRESOLVED,
                    "unit palette entry not resolved",
                )
            })?;
        let mut owner = world
            .marker_owners
            .get(&m.marker_id)
            .copied()
            .filter(|&o| o != -1)
            .unwrap_or(entry.player);
        let mut defined = entry.defined;
        let mut rejected = owner == -1;
        if !rejected {
            let pl = self
                .players
                .players
                .get(owner as usize)
                .copied()
                .ok_or_else(|| {
                    Unmodelled::new(
                        draw_site::PALETTE_OWNER_NOT_PLAYER,
                        format!("palette owner {owner} is not a player"),
                    )
                })?;
            if pl.flags & 4 != 0 && entry.team != -1 {
                let t = self
                    .players
                    .teams
                    .get(entry.team as usize)
                    .copied()
                    .unwrap_or_default();
                if t.flags & 1 != 0 || t.player == -1 {
                    rejected = true;
                } else {
                    owner = t.player;
                }
            }
        }
        let mut row = entry.row;
        if !rejected && owner != entry.player {
            let own = self.customize_unit_definition(world, p.tag_id, owner, r)?;
            defined = own.is_some();
            row = own.unwrap_or(UnitRow::OWN);
        }
        if rejected || (m.flags & 1 != 0 && palette_flags & 8 == 0) {
            return Ok(());
        }
        if !self.palette_compatible(p, draw_site::MONSTER_PALETTE)? {
            return Ok(());
        }
        if entry.team == -1 || !defined {
            return Ok(());
        }
        let Some(palette_unit) = self.tags.tag(*b"unit", p.tag_id) else {
            return Ok(());
        };
        let entry_player = self.row_entry_player.unwrap_or(entry.player);
        let palette_tag = u32::from_be_bytes(p.tag_id.0);
        let mut unit_tag = p.tag_id;
        let mut statistics_row = None;
        let mut veteran = None;
        if palette_flags & 4 != 0 || (palette_flags & 8 != 0 && m.flags & 1 != 0) {
            if world.monster_initializers.can_give() && palette_unit.len() >= 4 {
                let row_index = world.statistics.palette_row(
                    m.palette_index as usize,
                    palette_tag,
                    owner,
                    entry_player,
                );
                let row_owner = world.statistics.unit_row_owner(row_index);
                let tags = self.tags;
                veteran = world.monster_initializers.take(
                    p.tag_id,
                    fourcc(&palette_unit, 0),
                    row_owner,
                    &self.players,
                    |v| {
                        tags.tag(*b"unit", v.unit_tag).is_some()
                            && tags.tag(*b"mons", v.monster_tag).is_some()
                    },
                );
            }
            if veteran.is_none() && palette_flags & 8 != 0 {
                return Ok(());
            }
        }
        if let Some((record, v)) = &veteran
            && v.unit_tag != p.tag_id
        {
            let copy =
                self.new_monster_palette_entry(world, v.unit_tag, palette_flags, entry.team, r)?;
            unit_tag = v.unit_tag;
            if copy.player != -1 {
                row = copy.row;
                if !copy.defined {
                    world.monster_initializers.release(*record);
                    return Ok(());
                }
                statistics_row = Some(
                    world.statistics.palette_copy_row(
                        crate::statistics::PaletteUnit {
                            tag: u32::from_be_bytes(v.unit_tag.0),
                            mons: self
                                .tags
                                .tag(*b"unit", v.unit_tag)
                                .and_then(|u| u.get(..4).map(|m| [m[0], m[1], m[2], m[3]]))
                                .map(u32::from_be_bytes),
                            team: entry.team,
                            flags: palette_flags,
                        },
                        &self.players,
                        self.game_type,
                    ),
                );
            }
        }
        if self.params.difficulty < m.difficulty_flags {
            if let Some((record, _)) = &veteran {
                world.monster_initializers.release(*record);
            }
            return Ok(());
        }
        let Some(unit) = self.tags.tag(*b"unit", unit_tag) else {
            return Ok(());
        };
        if unit.len() < 8 {
            return Err(Unmodelled::new(
                draw_site::UNIT_SHORT,
                format!("unit {unit_tag} too short"),
            ));
        }
        let mons = fourcc(&unit, 0);
        let core_tag = fourcc(&unit, 4);
        let di = self.monster_definition(world, mons)?;
        let def = world.monster_definitions[di].clone();
        let z = if def.flags & 0x0100_0000 != 0 {
            new_monster_marker_height(
                m.z,
                world.mesh_height(m.x, m.y, mandrake_tag::MeshHeightMode::Surface),
                self.params.gameplay_version,
                r,
            )
        } else {
            m.z
        };
        let Some((index, _)) = world.alloc_monster_index() else {
            return Ok(());
        };
        self.create_monster(
            world,
            NewMonster {
                index,
                identifier: m.marker_id,
                first_draw,
                x: m.x,
                y: m.y,
                z,
                facing: m.yaw as u16,
                palette_flags,
                marker_flags: m.flags,
                marker: true,
                di,
                mons,
                core_tag,
                unit_tag,
                row,
                owner,
                team: entry.team,
                veteran: veteran.as_ref().map(|(_, v)| v),
                statistics: match statistics_row {
                    Some(row) => StatisticsRow::Fixed(row),
                    None => StatisticsRow::Palette {
                        palette: m.palette_index as usize,
                        tag: palette_tag,
                        entry_player,
                    },
                },
                located: None,
            },
            r,
        )?;
        if m.flags & 4 != 0 {
            if m.flags & 8 != 0 {
                self.stops.push(Unmodelled::new(
                    draw_site::MARKER_FLAGS_4_AND_8,
                    format!("new_monster: marker {} has flags 4 and 8", m.marker_id),
                ));
            }
            if let Some(u) = world
                .unit_markers
                .iter_mut()
                .find(|u| u.identifier == m.marker_id)
            {
                u.created = false;
            }
            let marker = (world.formation_globals.gameplay_version <= game_options::GAMEPLAY_1_8_4)
                .then(|| self.mesh.markers.first())
                .flatten();
            if let Err(e) = world.kill_monster_born_dead(index, marker, r) {
                self.stops.push(e);
            }
            return Ok(());
        }
        if m.flags & 8 != 0 {
            world.make_monster_invisible(index, r, 0)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_monster_at_location(
        &mut self,
        world: &mut EngineWorld,
        unit_tag: FourCC,
        row: UnitRow,
        player: i16,
        flags: (u32, u32),
        at: MonsterLocation,
        r: &mut EngineRng,
    ) -> Result<Option<usize>, Unmodelled> {
        let first_draw = r.sites.total();
        let unit = self
            .tags
            .tag(*b"unit", unit_tag)
            .filter(|u| u.len() >= 8)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::AT_LOCATION_UNIT_MISSING,
                    format!("unit {unit_tag} missing"),
                )
            })?;
        let mons = fourcc(&unit, 0);
        let core_tag = fourcc(&unit, 4);
        let di = self.monster_definition(world, mons)?;
        let Some((index, identifier)) = world.alloc_monster_index() else {
            return Ok(None);
        };
        let team = self
            .players
            .players
            .get(player as usize)
            .map(|p| i16::from(p.team))
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::AT_LOCATION_PLAYER_MISSING,
                    format!("player {player} missing"),
                )
            })?;
        let statistics = world
            .statistics
            .find_unit(player, u32::from_be_bytes(unit_tag.0))
            .map(|i| i as i16)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::AT_LOCATION_NO_STATISTICS,
                    format!("unit {unit_tag} of player {player}: no statistics row"),
                )
            })?;
        self.create_monster(
            world,
            NewMonster {
                index,
                identifier,
                first_draw,
                x: at.x,
                y: at.y,
                z: at.z,
                facing: at.facing,
                palette_flags: 0,
                marker_flags: 0,
                marker: false,
                di,
                mons,
                core_tag,
                unit_tag,
                row,
                owner: player,
                team,
                veteran: None,
                statistics: StatisticsRow::Fixed(statistics),
                located: Some(flags),
            },
            r,
        )?;
        let def_flags = world.monster_definitions[di].flags;
        let m = &mut world.monsters[index];
        let height = i32::from(m.flying_height);
        if def_flags & 8 == 0 && height > 0 {
            let headers = world.collections.get(&m.object.collection);
            let top = headers
                .and_then(|h| m.object.bounds(h))
                .map_or(0, |b| i32::from(b.2));
            if height > 2 * top {
                return Err(Unmodelled::new(
                    draw_site::AT_LOCATION_TOO_HIGH,
                    format!(
                        "new_monster_at_location: monster {identifier} made {height} above the ground is killed"
                    ),
                ));
            }
            m.object.z -= height;
            m.flying_height = 0;
        }
        let m = &world.monsters[index];
        let terrain = world.cells.terrain_type(at.x, at.y);
        if (u32::from(m.impassable) >> (terrain & 0x1f)) & 1 != 0 {
            return Err(Unmodelled::new(
                draw_site::AT_LOCATION_TOO_HIGH,
                format!(
                    "new_monster_at_location: monster {identifier} made on terrain {terrain} it can't take is killed"
                ),
            ));
        }
        Ok(Some(index))
    }

    fn create_monster(
        &mut self,
        world: &mut EngineWorld,
        c: NewMonster<'_>,
        r: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let NewMonster {
            index,
            identifier,
            first_draw,
            x,
            y,
            z,
            facing,
            palette_flags,
            marker_flags,
            marker,
            di,
            mons,
            core_tag,
            unit_tag,
            row,
            owner,
            team,
            veteran,
            statistics,
            located,
        } = c;
        let def = world.monster_definitions[di].clone();
        let core = self
            .tags
            .tag(*b"core", core_tag)
            .and_then(|c| CollectionReference::from_bytes(&c).ok())
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::MONSTER_CORE_MISSING,
                    format!("core {core_tag} missing"),
                )
            })?;
        let obje = self.object_definition(def.object, draw_site::MONSTER_OBJECT_DEFINITION)?;
        let permutations = if row.custom_core {
            1
        } else {
            i32::from(core.permutation_count)
        };
        let perm = match &veteran {
            Some(v) => i32::from(v.permutation as i8).clamp(0, (permutations - 1).max(0)),
            None => r.draw_mod(draw_site::MONSTER_PERMUTATION, permutations)?,
        };
        let delta = i32::from(be_i16(&obje, 0x0e));
        let scale = i32::from(be_u16(&obje, 0x0c))
            + if delta > 0 {
                (i32::from(r.draw(draw_site::MONSTER_SCALE)) * delta) >> 16
            } else {
                0
            };
        world.load_collection(self.tags, core.collection_tag)?;
        world
            .collection_table0_cores
            .entry(core.collection_tag)
            .or_insert(core_tag);
        let mut obj = self.new_object(world, &obje, core.collection_tag, scale, (x, y, z), r);
        obj.permutation = perm as i16;
        obj.core = Some(core_tag);

        let ammunition = i32::from(def.field_74)
            + if def.field_76 > 0 {
                (i32::from(r.draw(draw_site::MONSTER_FIELD_76)) * i32::from(def.field_76)) >> 16
            } else {
                0
            };
        let jitter = (i32::from(def.speed) * 7) >> 8;
        let mut speed_7c = i32::from(def.speed);
        if jitter > 0 {
            let r16 = u32::from(r.draw(draw_site::MOVE_CREATION_SPEED));
            speed_7c = speed_7c - jitter + ((r16 * (2 * jitter) as u32) >> 16) as i32;
        }
        let name_index = {
            let d = &mut world.monster_definitions[di];
            if d.name_counter < 0 {
                d.name_counter = (r.draw(draw_site::MONSTER_NAME_COUNTER) & 0x7fff) as i16;
            }
            let mut name = (d.name_counter & 0x7f) as i8;
            if d.name_count > 0 {
                name = (i32::from(d.name_counter) % i32::from(d.name_count)) as i8;
            }
            d.name_counter = d.name_counter.wrapping_add(1);
            name
        };
        let speed = (i32::from(r.draw(draw_site::CHANGE_MODE_ROLL)) >> 9) + 0x300;
        let headers = world.headers(core.collection_tag).to_vec();
        set_object_shape(
            &mut obj,
            &headers,
            i32::from(def.idle_sequence),
            speed,
            3,
            r,
        )?;
        let mirrored = r.draw(draw_site::MONSTER_MIRRORED) < def.creation_draw_threshold;

        let mut flags = 0x8000u32;
        let mut flags_8 = 0x20000u32;
        if palette_flags & 1 != 0 {
            flags |= 0x1000_0000;
            flags_8 |= 2;
        }
        if palette_flags & 0x10 != 0 {
            flags_8 |= 0x1000;
        }
        if palette_flags & 0x20 != 0 {
            flags_8 |= 0x2000;
            if self.game_type == 12 {
                flags |= 0x1000_0000;
            }
        }
        if def.flags & 0x800 != 0 {
            flags_8 |= 0x10;
        }
        if def.field_199 & 0x20 != 0 {
            flags_8 |= 8;
        }

        let inventory = def
            .inventory
            .filter(|_| self.params.gameplay_version > game_options::GAMEPLAY_1_5_1);
        if inventory.is_some() && def.map_action.is_some_and(|a| a.0 == *b"soar") {
            return Err(Unmodelled::new(
                draw_site::MONSTER_INVENTORY_AND_SOARING,
                format!("mons {mons}: an initial inventory and the soaring map action"),
            ));
        }
        let mut ambient = None;
        let mut flying_height = (obj.z as i16).wrapping_sub(world.mesh_height(
            x,
            y,
            mandrake_tag::MeshHeightMode::Surface,
        ) as i16);
        let position = (x, y);
        if let Some(action) = def.map_action {
            ambient = match &action.0 {
                b"soar" => {
                    let r16 = r.draw(draw_site::MONSTER_FLYING_HEIGHT);
                    flying_height = (0x500 + (i32::from(r16) >> 7)) as i16;
                    Some(AmbientData::Soaring {
                        timer: -1,
                        animation_timer: -1,
                    })
                }
                b"chic" => Some(AmbientData::Chicken {
                    timer: -1,
                    home: position,
                }),
                b"peas" => Some(AmbientData::Peasant {
                    timer: -1,
                    home: position,
                }),
                b"deer" => Some(AmbientData::Deer {
                    flags: 0,
                    timer: -1,
                    home: position,
                    vitality: obj.vitality as u16,
                }),
                b"bear" => {
                    return Err(Unmodelled::new(
                        draw_site::MONSTER_BEAR_ACTION,
                        format!("map action 'bear' for mons {mons}"),
                    ));
                }
                _ => None,
            };
        }
        obj.kind = 2;
        obj.owner = index as i16;
        obj.flags |= 0x0040_0400;
        if def.flags & 0x1_0000 != 0 {
            obj.flags |= 0x0800_0000;
        }
        if def.flags & 0x2000 == 0 {
            obj.flags |= 0x0400_0000;
        }
        if def.flags & 0x10 != 0 {
            obj.flags |= 0x2000;
        }
        if def.flags & 4 != 0 {
            obj.flags |= 0x1800;
        }
        if def.field_199 & 0x10 != 0 {
            obj.flags |= 0x1000;
            obj.z = world.mesh_height(x, y, mandrake_tag::MeshHeightMode::Surface);
            obj.flags &= !0xc0;
        }
        if matches!(ambient, Some(AmbientData::Soaring { .. })) {
            obj.z = world.mesh_height(x, y, mandrake_tag::MeshHeightMode::Surface)
                + i32::from(flying_height);
        }
        if let Some(action) = def.map_action {
            if self.after_load {
                world
                    .pending_map_actions
                    .push(crate::map_actions::PendingMonsterAction {
                        monster: index,
                        identifier,
                        action,
                    });
            } else if let Some(actions) = world.map_actions.as_mut() {
                let slot = actions.add_monster_action(action, identifier);
                world.monster_map_actions.insert(index, slot);
            }
        }
        let formation_identifier = world.next_formation_identifier();
        let cell_link = world.next_cell_link();
        let mons_data = self.tags.tag(*b"mons", mons).ok_or_else(|| {
            Unmodelled::new(
                draw_site::EQUALIZE_MONS_SHORT,
                format!("mons {mons} missing"),
            )
        })?;
        let impassable = mandrake_tag::unit::MonsterTerrain::from_mons_bytes(&mons_data)
            .map_err(|e| Unmodelled::new(draw_site::MONSTER_MOVEMENT, e.to_string()))?
            .impassable_mask(self.params.gameplay_version);
        let neighbours = world.cells.neighbour_mask(impassable, x, y);
        *world.combat_mut(index) = crate::attack_ai::MonsterCombat {
            ammunition: ammunition as u8,
            name_index: veteran.as_ref().map_or(name_index, |v| v.name_index as i8),
            experience: veteran.as_ref().map_or(0, |v| v.experience),
            battles: veteran.as_ref().map_or(0, |v| v.field_8f_plus_one as i8),
            custom_name: veteran
                .as_ref()
                .map_or([0; 18], |v| crate::veterans::custom_name(&v.name)),
            mana: def.ai.word(0x214),
            inventory_size: def.ai.word(0x21c) as i8,
            unit: (u32::from_be_bytes(unit_tag.0), row.owner),
            marker_flags,
            ..Default::default()
        };
        if marker
            && let Some(u) = world
                .unit_markers
                .iter_mut()
                .find(|u| u.identifier == identifier)
        {
            u.created = true;
        }
        let monster = EngineMonster {
            index,
            identifier,
            first_draw,
            definition: di,
            object: obj,
            mode: 1,
            anim_state: 0x1a,
            speed: speed_7c as i16,
            position,
            flags,
            flags_8,
            neighbours,
            impassable,
            ambient,
            unmodelled: None,
            mirrored,
            team: i16::from(team as i8),
            owner: i16::from(owner as i8),
            facing,
            formation: MonsterFormation {
                identifier: formation_identifier,
                ..MonsterFormation::default()
            },
            cell_link,
            flying_height,
        };
        match world.monsters.get_mut(index) {
            Some(slot) => *slot = monster,
            None => world.monsters.push(monster),
        }
        let unit = match statistics {
            StatisticsRow::Fixed(row) => row,
            StatisticsRow::Palette {
                palette,
                tag,
                entry_player,
            } => world
                .statistics
                .palette_row(palette, tag, owner, entry_player),
        };
        let object = &world.monsters[index].object;
        let (hit_points, object_flags) = (object.vitality as i16, object.flags);
        world.statistics.monster_created(
            index,
            unit,
            crate::statistics::UnitCosts {
                cost: def.ai.word(0x1f6),
                class: def.ai.word(0x1b6),
                max_hit_points: be_i16(&obje, 8),
                flags: def.flags,
            },
            hit_points,
            object_flags,
        );
        let own = crate::attack_ai::Own {
            position,
            timer: 0,
            speed: speed_7c as i16,
            facing,
            desired_facing: 0,
        };
        let t = world.find_target(index, 0, &own)?;
        if t != -1 {
            world.engage_monster(index, t as usize, 0xa)?;
        }
        let m = &mut world.monsters[index];
        let visible_group = if let Some((flags_4, flags_8)) = located {
            if world.formation_globals.gameplay_version > 0x69f {
                m.flags |= flags_4;
                m.flags_8 |= flags_8;
            }
            true
        } else {
            world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1
        };
        if visible_group
            && def.flags & 0x1000_0000 != 0
            && !m.object.no_shape
            && resolved_mons_group(&mons_data, &world.catalog).is_some()
        {
            m.flags_8 |= 0x40_0000;
        }
        if let Some(group) = inventory {
            world.initial_inventory(index, group, r).map_err(|e| {
                Unmodelled::new(
                    e.site,
                    format!(
                        "{} (initial inventory of mons {mons}, prgr {group})",
                        e.reason
                    ),
                )
            })?;
        }
        Ok(())
    }

    fn new_projectile_from_marker(
        &mut self,
        world: &mut EngineWorld,
        marker: usize,
        m: &MarkerEntry,
        words: [i16; 4],
        r: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let first_draw = r.sites.total();
        let Some(p) = engine_palette(self.mesh, m) else {
            return Ok(());
        };
        let Some(proj) = self.tags.tag(*b"proj", p.tag_id) else {
            return Ok(());
        };
        if self.params.difficulty < m.difficulty_flags {
            return Ok(());
        }
        let desc = crate::projectile_groups::ProjectileDescriptor {
            definition: p.tag_id,
            flags: 0,
            position: [m.x, m.y, m.z],
            velocity: [words[0], words[1], words[2]],
            facing: u16::from_be_bytes([m.type_data[0], m.type_data[1]]),
            elevation: u16::from_be_bytes([m.type_data[2], m.type_data[3]]),
            word_1c: 0,
            owner_monster: -1,
            parent: -1,
            owner_object: -1,
            target: -1,
            target_point: [0; 3],
        };
        let made = crate::projectile_groups::new_projectile(
            &mut crate::projectile_groups::EngineSpawn {
                world,
                rng: r,
                source: None,
                host: None,
            },
            &desc,
        )?;
        let Some(index) = made else {
            return Ok(());
        };
        let Some(at) = world
            .projectile_records
            .iter()
            .position(|q| q.index == index)
        else {
            return Ok(());
        };
        let record = &mut world.projectile_records[at];
        let object_index = record.shape.as_ref().map_or(-1, |o| o.index);
        if m.marker_id != 0 {
            let identifier = world.free_object_identifier(object_index, m.marker_id);
            if let Some(o) = world.projectile_records[at].shape.as_mut() {
                o.identifier = identifier;
            }
        }
        let record = &mut world.projectile_records[at];
        if m.flags & 4 != 0 {
            record.record_flags |= 1;
            return Ok(());
        }
        if words[2] != 0 || be_i32(&proj, 0) as u32 & 0x80 != 0 {
            return Ok(());
        }
        let q = world.projectile_records.remove(at);
        let Some(mut object) = q.shape else {
            return Ok(());
        };
        object.kind = 4;
        object.flags |= 0x110;
        object.velocity = [q.object.vx, q.object.vy, q.object.vz];
        world.projectiles.push(EngineProjectile {
            marker,
            identifier: object.identifier,
            definition: p.tag_id,
            first_draw,
            object,
            position: (q.object.x, q.object.y),
            charge_time: 0,
            facing: q.facing,
            elevation: q.elevation,
            cell_link: q.cell_link,
        });
        Ok(())
    }

    fn new_model_from_marker(
        &mut self,
        world: &mut EngineWorld,
        m: &MarkerEntry,
        models: &mut i16,
    ) -> Result<(), Unmodelled> {
        let Some(p) = engine_palette(self.mesh, m) else {
            return Ok(());
        };
        let Some(mode) = self.tags.tag(*b"mode", p.tag_id) else {
            return Ok(());
        };
        let geom_tag = fourcc(&mode, 0x04);
        let geom = self.tags.tag(*b"geom", geom_tag).ok_or_else(|| {
            Unmodelled::new(
                draw_site::MODEL_GEOMETRY_MISSING,
                format!("geom {geom_tag} missing"),
            )
        })?;
        let shape = crate::models::ModelShape::from_tags(&mode, &geom).map_err(|e| {
            Unmodelled::new(
                draw_site::MODEL_GEOMETRY_MISSING,
                format!("mode {}: {e}", p.tag_id),
            )
        })?;
        let index = *models;
        *models += 1;
        let yaw = u16::from_be_bytes([m.type_data[0], m.type_data[1]]);
        let pitch = u16::from_be_bytes([m.type_data[2], m.type_data[3]]);
        let permutation = i16::from_be_bytes([m.type_data[4], m.type_data[5]]);
        push_world_model(
            world,
            index,
            &geom,
            geom_tag,
            (&shape, permutation),
            [m.x, m.y, m.z],
            yaw,
            pitch,
            false,
            -1,
        )?;
        let cells = crate::models::new_model_cells_for_version(
            &mut world.cells,
            index,
            (m.x, m.y, m.z),
            yaw,
            pitch,
            &shape,
            self.params.gameplay_version,
        );
        for (cx, cy) in cells.changed {
            update_monster_impassability_for_cell(world, cx, cy);
        }
        if cells.overlapped {
            delete_load_model(world, index, models);
            return Ok(());
        }
        crate::models::new_model_heights(
            &world.cells,
            &mut world.heights,
            (m.x, m.y, m.z),
            yaw,
            pitch,
            &shape,
        );
        world.set_model_identifier(index as usize, m.marker_id);
        Ok(())
    }

    fn new_model_animation(
        &mut self,
        world: &mut EngineWorld,
        marker: usize,
        m: &MarkerEntry,
        models: &mut i16,
        r: &mut EngineRng,
    ) -> Result<(), Unmodelled> {
        let Some(p) = engine_palette(self.mesh, m) else {
            return Ok(());
        };
        let Some(anim) = self.tags.tag(*b"anim", p.tag_id) else {
            return Ok(());
        };
        let definition = std::sync::Arc::new(
            crate::model_animations::AnimationDefinition::from_tags(&anim, self.tags)?,
        );
        let frames = if anim.len() >= 6 { be_i16(&anim, 4) } else { 0 };
        let random_start = anim.len() >= 6 && be_i32(&anim, 0) & 1 != 0;
        let mut frame = 0i32;
        if random_start && frames > 0 {
            frame = (i32::from(frames) * i32::from(r.draw(draw_site::MODEL_ANIMATION_FRAME))) >> 16;
        }
        let o = 0x0c + 16 * frame as usize;
        if anim.len() < o + 4 {
            return Err(Unmodelled::new(
                draw_site::MODEL_ANIMATION_FRAME_OUTSIDE,
                format!("anim {}: frame {frame} outside the tag", p.tag_id),
            ));
        }
        let position = (m.x, m.y, m.z);
        let pitch = u16::from_be_bytes([m.type_data[2], m.type_data[3]]);
        let mut model = -1;
        if let Some(mode) = self.tags.tag(*b"mode", fourcc(&anim, o)) {
            let geom_tag = fourcc(&mode, 0x04);
            let geom = self.tags.tag(*b"geom", geom_tag).ok_or_else(|| {
                Unmodelled::new(
                    draw_site::MODEL_GEOMETRY_MISSING,
                    format!("geom {geom_tag} missing"),
                )
            })?;
            let shape = crate::models::ModelShape::from_tags(&mode, &geom).map_err(|e| {
                Unmodelled::new(
                    draw_site::MODEL_GEOMETRY_MISSING,
                    format!("anim {}: {e}", p.tag_id),
                )
            })?;
            let index = *models;
            *models += 1;
            let permutation = definition
                .frames
                .get(frame as usize)
                .map_or(0, |f| f.permutation);
            push_world_model(
                world,
                index,
                &geom,
                geom_tag,
                (&shape, permutation),
                [m.x, m.y, m.z],
                0,
                pitch,
                frames > 1,
                world.model_animations.animations.len() as i16,
            )?;
            let cells = crate::models::new_model_cells_for_version(
                &mut world.cells,
                index,
                position,
                0,
                pitch,
                &shape,
                self.params.gameplay_version,
            );
            for (cx, cy) in cells.changed {
                update_monster_impassability_for_cell(world, cx, cy);
            }
            if cells.overlapped {
                delete_load_model(world, index, models);
            } else {
                crate::models::new_model_heights(
                    &world.cells,
                    &mut world.heights,
                    position,
                    0,
                    pitch,
                    &shape,
                );
                model = index;
            }
        }
        let animations = &mut world.model_animations;
        animations.add(
            marker,
            m.marker_id,
            definition,
            frame as usize,
            position,
            pitch,
            model,
        );
        let i = animations.animations.len() - 1;
        if !random_start && animations.animations[i].playback.active {
            animations.play_sound(i, 0);
        }
        Ok(())
    }
}

fn delete_load_model(world: &mut EngineWorld, index: i16, models: &mut i16) {
    let mut animations = std::mem::take(&mut world.model_animations);
    animations.delete_model(index, world);
    animations.free_models.retain(|&k| k != index);
    world.model_animations = animations;
    debug_assert_eq!(world.models.len(), index as usize + 1);
    world.models.truncate(index as usize);
    *models = index;
}

#[allow(clippy::too_many_arguments)]
fn push_world_model(
    world: &mut EngineWorld,
    index: i16,
    geom: &[u8],
    geom_tag: FourCC,
    (shape, permutation): (&crate::models::ModelShape, i16),
    position: [i32; 3],
    yaw: u16,
    pitch: u16,
    animated: bool,
    animation: i16,
) -> Result<(), Unmodelled> {
    let tag = mandrake_tag::model::GeometryTag::from_bytes(geom).map_err(|e| {
        Unmodelled::new(
            draw_site::MODEL_GEOMETRY_MISSING,
            format!("geom {geom_tag}: {e}"),
        )
    })?;
    let geometry = mandrake_tag::model_hit::HitGeometry::new(&tag);
    debug_assert_eq!(world.models.len(), index as usize);
    let identifier = world.new_model_identifier();
    world
        .geometry_surfaces
        .entry(geom_tag)
        .or_insert_with(|| crate::models::surface_materials(&tag));
    let mut model = crate::models::WorldModel::new(
        identifier,
        position,
        yaw,
        pitch,
        permutation,
        std::sync::Arc::new(geometry),
        shape.clone(),
        animated,
        animation,
    );
    model.geometry_tag = Some(geom_tag);
    world.models.push(model);
    Ok(())
}
