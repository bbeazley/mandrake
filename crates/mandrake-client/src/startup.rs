use crate::args::Args;
use crate::interface_config::{self, InterfaceConfig};
use crate::interface_setup::load_interface_colors;
use crate::map_load::{
    DEFAULT_MESH, LoadedMap, MODEL_ARCHIVES, MeshQuery, load_mesh_and_texture, open_model_archives,
    resolve_film_path,
};
use crate::region_view::{VIEW_BARS, open_view_bars};
use crate::start_camera::{StartTeams, initial_camera_for, start_observer_state};
use crate::{camera_control, media, move_marker, scenery, unit_anim};
use mandrake_common::FourCC;
use mandrake_rec::Recording;
use mandrake_rec::header::MonsterInitializer;
use mandrake_sim::Simulation;
use mandrake_sim::engine_rng::GameParameters;
use mandrake_sim::players::{SoloGame, SoloStart, solo_game_options};
use mandrake_tag::collection::{CollectionTag, TerrainTexture};
use mandrake_tag::{InterfaceColorTable, MeshConnectorList, MeshTag, TagArchive};
use mandrake_view::camera::Camera;
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedGameStart {
    pub(crate) end_tick: u32,
    pub(crate) header: Option<mandrake_rec::saved_game::SavedGameHeader>,
}

pub(crate) struct Startup {
    pub(crate) interface: InterfaceConfig,
    pub(crate) mesh_tag: MeshTag,
    pub(crate) mesh_name: String,
    pub(crate) mesh_data: Vec<u8>,
    pub(crate) terrain_texture: Option<TerrainTexture>,
    pub(crate) landscape_detail: Option<std::sync::Arc<mandrake_render::detail::LandscapeDetail>>,
    pub(crate) archive: TagArchive,
    pub(crate) large_archive: Option<TagArchive>,
    pub(crate) connectors: MeshConnectorList,
    pub(crate) ambient_tag: Option<FourCC>,
    pub(crate) replay_recording: Option<Recording>,
    pub(crate) restore: Option<SavedGameStart>,
    pub(crate) film_plugin_names: Vec<String>,
    pub(crate) film_names: Vec<(mandrake_session::Text, mandrake_session::Text)>,
    pub(crate) initial_camera: Camera,
    pub(crate) start_state: Option<camera_control::ObserverState>,
    pub(crate) interface_colors: Option<InterfaceColorTable>,
    pub(crate) interface_collection: Option<CollectionTag>,
    pub(crate) scenery_objects: scenery::SceneryObjects,
    pub(crate) engine_idle: Option<unit_anim::EngineIdleModel>,
    pub(crate) map_load_seed: Option<mandrake_math::BungieRng>,
    pub(crate) sim: Simulation,
}

fn solo_simulation(
    args: &Args,
    mesh_id: [u8; 4],
    veterans: &[MonsterInitializer],
) -> anyhow::Result<(Simulation, GameParameters)> {
    let game = SoloGame {
        seed: args.seed.unwrap_or_else(|| {
            crate::clock::SystemTime::now()
                .duration_since(crate::clock::UNIX_EPOCH)
                .map_or(1, |d| d.as_millis() as u32)
        }),
        difficulty: args.difficulty,
        options: args
            .game_options
            .unwrap_or_else(|| solo_game_options(SoloStart::Map { restart: false }, mesh_id)),
    };
    info!(
        "Solo mission: difficulty {}, seed {}, options {:#x}",
        game.difficulty, game.seed, game.options
    );
    let mut sim = Simulation::solo_game(game);
    if let Some(path) = &args.input_film {
        let film = crate::game_files::read(path)?;
        let (_, input) = Simulation::live_game_of_film(&film)
            .map_err(|e| anyhow::anyhow!("--input-film {path:?}: {e}"))?;
        sim.scripted_input = Some(input);
    }
    sim.carried_initializers = if !veterans.is_empty() {
        veterans.to_vec()
    } else {
        args.carried_initializers.clone()
    };
    sim.carried_rankings.clone_from(&args.carried_rankings);
    Ok((sim, GameParameters::solo(game.difficulty)))
}

fn flag_or_saved<F: Copy, S>(
    args: &Args,
    id: &str,
    flag: Option<F>,
    saved: impl Fn(F) -> S,
) -> (Option<F>, Option<S>) {
    match flag {
        Some(f) if args.from_conf.contains(&id) => (None, Some(saved(f))),
        _ => (flag, None),
    }
}

pub(crate) fn detail_textures_wanted(args: &Args) -> bool {
    crate::args::detail_textures_enabled(args.detail_textures, None)
}

pub(crate) fn vsync_wanted(args: &Args) -> (bool, &'static str) {
    let (flag, saved) = flag_or_saved(args, "vsync", args.vsync, |v| v == crate::args::Vsync::On);
    crate::args::vsync_wanted(flag, saved)
}

pub(crate) fn ten_bit_wanted(args: &Args) -> (bool, &'static str) {
    let (flag, saved) = flag_or_saved(args, "ten_bit", args.ten_bit, |v| {
        v == crate::args::TenBit::On
    });
    crate::args::ten_bit_wanted(flag, saved)
}

pub(crate) fn fps_cap_wanted(args: &Args) -> (crate::app::fps_cap::FpsCap, &'static str) {
    let (flag, saved) = flag_or_saved(args, "fps_cap", args.fps_cap, |c| c);
    crate::app::fps_cap::cap_wanted(flag, saved)
}

pub(crate) fn start_game(args: &Args) -> anyhow::Result<Startup> {
    let interface = interface_config::load(
        args.conf.as_deref(),
        args.headless || args.screenshot.is_some(),
    );
    start_level(args, interface, &[])
}

pub(crate) fn start_level(
    args: &Args,
    interface: InterfaceConfig,
    veterans: &[MonsterInitializer],
) -> anyhow::Result<Startup> {
    let film_bytes = match &args.replay {
        Some(replay_path) => {
            let replay_path = resolve_film_path(replay_path, &args.archive);
            Some(crate::game_files::read(&replay_path).map_err(|e| {
                anyhow::anyhow!("Failed to read replay file {:?}: {}", replay_path, e)
            })?)
        }
        None => None,
    };
    start_level_of(args, interface, veterans, film_bytes, None)
}

pub(crate) fn start_network_level(
    args: &Args,
    interface: InterfaceConfig,
    film: Vec<u8>,
    role: mandrake_sim::net_game::NetRole,
) -> anyhow::Result<Startup> {
    start_level_of(args, interface, &[], Some(film), Some(role))
}

fn game_plugin_names(
    film: Option<Vec<String>>,
    plugin_args: &[String],
    enhancement_packs: &[(String, std::path::PathBuf)],
) -> Vec<String> {
    let named = plugin_args
        .iter()
        .filter(|n| enhancement_packs.iter().all(|(e, _)| e != *n));
    match film {
        Some(film) => {
            for name in named.filter(|n| !film.contains(n)) {
                warn!("--plugin {name:?} is ignored: the film names its plugins {film:?}");
            }
            film
        }
        None => named.cloned().collect(),
    }
}

fn start_level_of(
    args: &Args,
    interface: InterfaceConfig,
    veterans: &[MonsterInitializer],
    film_bytes: Option<Vec<u8>>,
    network: Option<mandrake_sim::net_game::NetRole>,
) -> anyhow::Result<Startup> {
    let replay_recording = if let Some(data) = &film_bytes {
        let rec = Recording::from_bytes(data)
            .map_err(|e| anyhow::anyhow!("Failed to parse replay file {:?}: {}", args.replay, e))?;
        info!(
            "Loaded replay recording: version {}, ticks: {}, initial seed: {}, mesh {}, plugins {:?}",
            rec.header.version,
            rec.tick_commands.len(),
            rec.header.initial_seed,
            rec.header.mesh_tag_id,
            rec.header
                .active_plugins
                .iter()
                .map(|p| &p.name)
                .collect::<Vec<_>>()
        );
        Some(rec)
    } else {
        None
    };
    let restore = replay_recording
        .as_ref()
        .filter(|r| r.header.is_saved_game())
        .map(|r| SavedGameStart {
            end_tick: r.header.end_tick,
            header: film_bytes
                .as_deref()
                .and_then(|b| mandrake_rec::saved_game::SavedGame::from_bytes(b).ok())
                .map(|s| s.header),
        });
    let interface =
        interface.for_game(replay_recording.is_some() && restore.is_none() && network.is_none());
    let film_names = film_bytes
        .as_deref()
        .and_then(mandrake_session::films::read_header)
        .map(mandrake_session::films::player_names)
        .unwrap_or_default();
    let enhancement_packs = crate::map_load::enhancement_packs(&args.archive, &args.plugin);
    let film_plugin_names = game_plugin_names(
        replay_recording.as_ref().map(|r| {
            r.header
                .active_plugins
                .iter()
                .map(|p| p.name.clone())
                .collect()
        }),
        &args.plugin,
        &enhancement_packs,
    );
    let film_plugins: Vec<&str> = film_plugin_names.iter().map(String::as_str).collect();
    let mesh_query = match &replay_recording {
        Some(r) => {
            let id = r.header.mesh_tag_id;
            if let Some(mesh) = &args.mesh
                && MeshQuery::from_arg(mesh).id != Some(id)
                && mesh != DEFAULT_MESH
            {
                warn!("--mesh {mesh} is ignored: the film plays mesh {id}");
            }
            MeshQuery::film(id)
        }
        None => MeshQuery::from_arg(
            args.play
                .as_deref()
                .or(args.mesh.as_deref())
                .unwrap_or(DEFAULT_MESH),
        ),
    };
    info!("Target mesh: {}", mesh_query);

    MODEL_ARCHIVES.get_or_init(|| open_model_archives(&args.archive));
    let LoadedMap {
        mesh_tag,
        mesh_name,
        mesh_data,
        terrain_texture,
        archive,
        large_archive,
        connectors,
        ambient_tag,
    } = load_mesh_and_texture(
        &args.archive,
        args.textures_archive.as_deref(),
        &mesh_query,
        &film_plugins,
    )?;
    VIEW_BARS.init(|| open_view_bars(&args.archive), interface.control_bar);
    let landscape_detail = detail_textures_wanted(args)
        .then(|| {
            let vfs = mandrake_common::game_vfs();
            let packs: Vec<TagArchive> = enhancement_packs
                .iter()
                .filter_map(|(_, p)| match TagArchive::open(vfs, &p.to_string_lossy()) {
                    Ok(a) => Some(a),
                    Err(e) => {
                        warn!("Detail textures: {}: {e}", p.display());
                        None
                    }
                })
                .collect();
            let search: Vec<&TagArchive> = packs
                .iter()
                .chain(std::iter::once(&archive))
                .chain(large_archive.as_ref())
                .collect();
            crate::map_load::load_landscape_detail(&search, &mesh_tag)
        })
        .flatten();

    info!(
        "Loaded mesh '{}' (id: {}): dimensions {}x{} (submeshes: {}x{}), total cells: {}, markers: {}, palettes: {}",
        mesh_name,
        mesh_tag.mesh_id,
        mesh_tag.cell_width,
        mesh_tag.cell_height,
        mesh_tag.submesh_w,
        mesh_tag.submesh_h,
        mesh_tag.cells.len(),
        mesh_tag.markers.len(),
        mesh_tag.palettes.len()
    );

    if let Some(tex) = &terrain_texture {
        info!(
            "Associated landscape texture ready: {}x{} RGBA",
            tex.width, tex.height
        );
    } else {
        warn!("No landscape texture found; rendering with untextured fallback pipeline");
    }

    let mut game = {
        let film_params = film_bytes.as_deref().and_then(GameParameters::from_film);
        if film_bytes.is_some() && args.seed.is_some() {
            warn!("--seed is ignored with a film: the film's seed starts its game");
        }
        let fallback: Vec<&TagArchive> = large_archive
            .iter()
            .chain(std::iter::once(&archive))
            .collect();
        if let (Some(role), Some(film)) = (network, film_bytes.as_deref()) {
            let mut sim = Simulation::network_game(film, role)
                .map_err(|e| anyhow::anyhow!("the network game's start: {e}"))?;
            sim.carried_initializers
                .clone_from(&args.carried_initializers);
            scenery::client_game_on(
                sim,
                &args.archive,
                &film_plugins,
                &fallback,
                mesh_tag.mesh_id,
                &mesh_tag,
                film_params.unwrap_or(GameParameters::COMMAND_LINE),
                !args.no_units,
            )
        } else if args.play.is_some() {
            let (sim, params) = solo_simulation(args, mesh_tag.mesh_id.0, veterans)?;
            scenery::client_game_on(
                sim,
                &args.archive,
                &film_plugins,
                &fallback,
                mesh_tag.mesh_id,
                &mesh_tag,
                params,
                !args.no_units,
            )
        } else {
            scenery::client_game(
                &args.archive,
                &film_plugins,
                &fallback,
                mesh_tag.mesh_id,
                &mesh_tag,
                scenery::GameStart {
                    film: film_bytes.as_deref(),
                    seed: args.seed.unwrap_or(scenery::DEFAULT_GAME_SEED),
                    params: film_params.unwrap_or(GameParameters::COMMAND_LINE),
                },
                !args.no_units,
            )
        }
    };
    let start_teams = StartTeams::of(&game.sim);
    let initial_camera = initial_camera_for(&mesh_tag, args, &start_teams);
    let start_state = start_observer_state(&mesh_tag, args, &start_teams);
    if args.play.is_some() || network.is_some() {
        game.sim.halt_on_stop = true;
    }
    info!(
        "Initialized simulation: {} unit entities ({} map-load monsters, {} on the placeholder \
         locomotion), engine model {}",
        game.sim.units.len(),
        game.monsters.as_ref().map_or(0, Vec::len),
        game.movement.as_ref().map_or(0, |m| m.missing.len()),
        match &game.sim.engine_stop {
            Some(e) if e.site == mandrake_sim::draw_site::NO_ENGINE_MAP => {
                format!("off ({})", e.reason)
            }
            None => "running".to_string(),
            Some(e) => format!("stopped at load ({e})"),
        }
    );
    {
        let archives: Vec<&TagArchive> = large_archive
            .iter()
            .chain(std::iter::once(&archive))
            .collect();
        let archetypes: Vec<FourCC> = game.sim.units.values().map(|u| u.archetype).collect();
        let defs = mandrake_sim::CombatDefinitions::load(&archives, archetypes);
        info!(
            "Combat definitions: {} unit archetype(s), {} projectile(s), {} projectile group(s)",
            defs.monsters.len(),
            defs.projectiles.len(),
            defs.groups.len()
        );
        game.sim.set_combat_definitions(defs);
    }

    let interface_colors =
        load_interface_colors(&args.archive, &[Some(&archive), large_archive.as_ref()]);
    let interface_collection = args
        .archive
        .parent()
        .and_then(Path::parent)
        .and_then(move_marker::load_interface_collection);

    let mut scenery_objects = std::mem::take(&mut game.scenery);
    let engine_idle = unit_anim::EngineIdleModel::new(&game.sim);
    let map_load_seed = game.rng_at_start;
    let sim = game.sim;
    {
        let archives: Vec<&TagArchive> = large_archive
            .iter()
            .chain(std::iter::once(&archive))
            .collect();
        scenery_objects.media = Some(std::sync::Arc::new(media::MapMedia::load(
            &mesh_data, &mesh_tag, &archives,
        )));
    }

    let replay_recording = replay_recording.filter(|_| restore.is_none() && network.is_none());
    Ok(Startup {
        interface,
        mesh_tag,
        mesh_name,
        mesh_data,
        terrain_texture,
        landscape_detail,
        archive,
        large_archive,
        connectors,
        ambient_tag,
        replay_recording,
        restore,
        film_plugin_names,
        film_names,
        initial_camera,
        start_state,
        interface_colors,
        interface_collection,
        scenery_objects,
        engine_idle,
        map_load_seed,
        sim,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use mandrake_sim::statistics::PermanentRanking;

    #[test]
    fn a_films_plugins_are_its_own() {
        let flags = [
            "Patch 1.8.5 Maps".to_owned(),
            "Patch 1.8.5 Unity".to_owned(),
        ];
        assert!(game_plugin_names(Some(vec![]), &flags, &[]).is_empty());
        let mdl = vec!["Morte Della Luna 1.0".to_owned()];
        assert_eq!(
            game_plugin_names(Some(mdl.clone()), &mdl, &[]),
            mdl,
            "no double"
        );
        assert_eq!(game_plugin_names(None, &flags, &[]), flags);
        let pack = (
            "Magma - HD Megapack 1of2".to_owned(),
            std::path::PathBuf::new(),
        );
        let with_pack = [pack.0.clone(), flags[0].clone()];
        assert_eq!(
            game_plugin_names(None, &with_pack, std::slice::from_ref(&pack)),
            flags[..1]
        );
    }

    #[test]
    fn solo_simulation_sets_carried_initializers_and_rankings() {
        let mut args = Args::try_parse_from(["mandrake", "--play", "l1"]).unwrap();
        let ranking = PermanentRanking {
            player: 0,
            id: 1,
            points: 12345,
            ..Default::default()
        };
        args.carried_rankings = vec![ranking];

        let veteran = MonsterInitializer {
            player_id: 0,
            unit_tag: FourCC(*b"mysm"),
            monster_tag: FourCC(*b"mysn"),
            experience: 5,
            field_8f_plus_one: 2,
            permutation: 1,
            name_index: 0,
            name: "Veteran".to_string(),
        };

        let (sim, _) = solo_simulation(&args, *b"le3e", std::slice::from_ref(&veteran)).unwrap();
        assert_eq!(sim.carried_initializers, vec![veteran.clone()]);
        assert_eq!(sim.carried_rankings, vec![ranking]);

        args.carried_initializers = vec![veteran.clone()];
        let (sim2, _) = solo_simulation(&args, *b"le3e", &[]).unwrap();
        assert_eq!(sim2.carried_initializers, vec![veteran]);
        assert_eq!(sim2.carried_rankings, vec![ranking]);
    }
}
