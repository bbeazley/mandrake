use crate::hud;
use crate::interface_config::InterfaceConfig;
use crate::minimap;
use crate::region_view::VIEW_BARS;
use crate::ui_scale;
use mandrake_common::FourCC;
use mandrake_sim::Simulation;
use mandrake_tag::collection::CollectionTag;
use mandrake_tag::{InterfaceColorTable, MeshTag, TagArchive};
use std::path::Path;
use tracing::{info, warn};

const SHOW_VERSION: bool = false;

pub(crate) fn load_hud(
    archive_path: &Path,
    plugins: &[&str],
    mesh_data: Option<&[u8]>,
    mesh_tag: &MeshTag,
    sim: &Simulation,
    map_load_seed: Option<mandrake_math::BungieRng>,
    ((width, height), interface): ((u32, u32), InterfaceConfig),
) -> Option<hud::Hud> {
    let game_dir = archive_path.parent().and_then(Path::parent)?;
    let tags = match hud::HudTags::load(game_dir, plugins) {
        Ok(t) => t,
        Err(e) => {
            warn!("HUD tags not loaded ({e}); no status or control bar");
            return None;
        }
    };
    let difficulty = sim.engine.difficulty;
    let game_type = sim.engine.game_type;
    let mesh_flags = mesh_data
        .and_then(|d| d.get(0x4c..0x50))
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let game_options = sim
        .game_setup
        .as_ref()
        .map_or(mandrake_sim::players::COMMAND_LINE_GAME_OPTIONS, |s| {
            s.options
        });
    let setup = hud::HudSetup {
        title: tags.title(mesh_tag.string_list_tag, game_type, difficulty),
        mesh_flags,
        network_game: sim.is_network_game(),
        game_options,
    };
    let title = String::from_utf8_lossy(&setup.title).into_owned();
    let rules = tags.rules(mesh_tag.string_list_tag, game_type);
    let mut hud = hud::Hud::new(tags, setup, width, height);
    hud.set_rules(rules);
    {
        let world = &sim.engine;
        hud.set_bar_teams(hud::BarTeams::of(world));
        hud.set_chat_players(&world.players);
        hud.set_network_replay(
            world.mode == 0 && world.replay.as_ref().is_some_and(|r| r.mode == 1),
        );
        if SHOW_VERSION {
            hud.set_version(hud::version_text(
                world.formation_globals.gameplay_version as i16,
                game_options,
                plugins,
            ));
        }
    }
    hud.set_score_strings(
        mesh_tag.string_list_tag,
        mesh_data
            .and_then(mandrake_tag::mesh_links::MeshLinks::from_header)
            .is_some_and(|l| l.alternate.is_some()),
    );
    hud.set_control_bar_shown(interface.control_bar);
    hud.set_control_bar_fixed(interface.film);
    if let (world, Some(seed)) = (&sim.engine, map_load_seed) {
        hud.set_map_load(world, seed);
    }
    info!(
        "HUD: title {title:?}, mesh flags {mesh_flags:#x}, overhead map {}",
        if hud.overhead_map_shown() {
            "shown"
        } else {
            "hidden"
        }
    );
    Some(hud)
}

pub(crate) fn load_minimap(
    archive_path: &Path,
    plugins: &[&str],
    mesh_data: Option<&[u8]>,
    mesh_tag: &MeshTag,
    sim: &Simulation,
    width: u32,
    height: u32,
) -> Option<minimap::Minimap> {
    let game_dir = archive_path.parent().and_then(Path::parent)?;
    let mesh_flags = mesh_data
        .and_then(|d| d.get(0x4c..0x50))
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let Some(mut mm) = crate::world_view::myth2::minimap_start(sim).and_then(|start| {
        minimap::Minimap::load(
            game_dir,
            plugins,
            mesh_tag,
            mesh_flags,
            &start,
            (width as i32, height as i32),
        )
    }) else {
        warn!("Overhead map not loaded (no map collection or engine world)");
        return None;
    };
    if mesh_flags & hud::MESH_FLAG_MAP_HIDDEN_AT_START == 0 {
        mm.set_visible(true, 1);
    }
    info!(
        "Overhead map: {}x{} bitmap, window {:?}, explored only {}, zoom {}",
        mm.map.map_w, mm.map.map_h, mm.map.window, mm.map.explored_only, mm.map.zoom
    );
    Some(mm)
}

pub(crate) fn layout_interface(
    hud: Option<&mut hud::Hud>,
    map: Option<&mut minimap::OverheadMap>,
    ui: &ui_scale::UiScale,
) {
    let (lw, lh) = ui.logical;
    if let Some(hud) = hud {
        hud.layout(lw, lh);
    }
    if let Some(map) = map {
        map.rezoom((lw as i32, lh as i32));
    }
}

pub(crate) fn world_frame(width: u32, height: u32) -> ((i32, i32), (i32, i32)) {
    let (top, bottom) = VIEW_BARS.get();
    ((width as i32, height as i32 - top - bottom), (0, top))
}

pub(crate) fn load_interface_colors(
    archive_path: &Path,
    archives: &[Option<&TagArchive>],
) -> Option<InterfaceColorTable> {
    let coll_type = FourCC::new(*b".256");
    let id = mandrake_tag::interface::STANDARD_INTERFACE_COLLECTION;
    let decode = |arc: &TagArchive| -> Option<InterfaceColorTable> {
        let entry = arc.find_tag_by_id(coll_type, id)?;
        let data = arc.get_tag_data(entry).ok()?;
        let coll = CollectionTag::from_bytes(&data).ok()?;
        InterfaceColorTable::from_collection(&coll)
    };

    let mut table = archives.iter().flatten().find_map(|arc| decode(arc));
    if table.is_none() {
        let vfs = mandrake_common::game_vfs();
        table = ["international small install", "international large install"]
            .iter()
            .filter_map(|name| archive_path.parent().map(|p| p.join(name)))
            .filter(|p| crate::game_files::exists(p))
            .filter_map(|p| TagArchive::open(vfs, &p.to_string_lossy()).ok())
            .find_map(|arc| decode(&arc));
    }

    match &table {
        Some(t) => info!("Loaded {} standard interface colors from 'stin'", t.len()),
        None => warn!("Standard interface collection 'stin' not found; selection frames disabled"),
    }
    table
}
