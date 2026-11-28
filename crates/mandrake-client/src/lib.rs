#[cfg(target_arch = "wasm32")]
macro_rules! eprintln {
    ($($t:tt)*) => { $crate::web::console(&format!($($t)*)) };
}
#[cfg(target_arch = "wasm32")]
macro_rules! println {
    ($($t:tt)*) => { $crate::web::console(&format!($($t)*)) };
}

mod app;
pub mod args;
mod attack_click;
mod audio_out;
mod camera_control;
mod carnage_report;
mod clock;
mod command_bar;
#[allow(dead_code, unused_imports)]
mod conf;
mod config_store;
mod config_watch;
mod console;
mod controls;
mod effects;
mod fog;
mod fog_config;
#[cfg(not(target_arch = "wasm32"))]
mod folder_picker;
mod game_files;
mod headless;
mod headless_render;
mod headless_run;
mod interface_config;
mod interface_setup;
mod landscape_edit;
mod lighting;
mod lightning_draw;
mod map_load;
mod model_effects;
mod model_instances;
mod net_game;
mod overlays;
mod picking;
mod region_view;
mod render_passes;
mod scenery_billboards;
mod sprite_interp;
#[cfg(not(target_arch = "wasm32"))]
mod sprite_interp_gen;
mod sprite_manager;
mod start_camera;
mod startup;
mod tfl_frame_parts;
mod tfl_game;
#[allow(dead_code)]
mod tfl_hud;
mod tfl_hud_scores;
mod tfl_hud_selection;
mod tfl_input;
mod tfl_landscape;
mod tfl_lightning;
mod tfl_live;
mod tfl_local_projectiles;
mod tfl_map;
mod tfl_menu;
mod tfl_minimap;
mod tfl_models;
mod tfl_net;
mod tfl_objects;
mod tfl_sound;
mod tfl_view;
mod tfl_water;
mod tfl_window;
mod unit_billboards;
mod water_bed;
mod water_config;
mod water_ripples_input;
mod world_assembly;
mod world_view;
use controls::user_dirs;
mod corpse;
mod film_report;
mod game_speed;
mod hud;
mod interface_state;
mod key_bindings;
mod local_input;
mod map_sound;
mod media;
mod mesh_effect_sprites;
mod mesh_ripple;
mod minimap;
mod move_marker;
mod object_bounds;
mod object_history;
mod object_sprites;
mod particles;
mod projectile_local;
mod projfx_log;
mod scenery;
mod scores;
mod screenshot;
mod selection_input;
mod selection_rules;
mod settings;
mod shadowgen;
mod team_colours;
mod ui_scale;
mod unit_anim;
mod unit_info;
mod win_hdr;
mod window_surface;

#[cfg(target_arch = "wasm32")]
pub mod web;

use crate::app::shell_game::run_rebuilds;
use crate::app::shell_menu::run_menu;
use crate::app::window::run_window;
use crate::args::Args;
use crate::headless_run::run_headless;
use crate::startup::start_game;
use tracing::info;

pub fn log_filter() -> tracing_subscriber::EnvFilter {
    log_filter_from("RUST_LOG")
}

pub(crate) fn log_filter_from(var: &str) -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::ERROR.into())
        .with_env_var(var)
        .from_env_lossy()
}

pub fn parse_command_line(words: Vec<std::ffi::OsString>) -> Result<Args, clap::Error> {
    conf::parse_args(words)
}

pub fn init_audio_env() {
    audio_out::point_alsa_at_host_plugins();
}

pub fn run(#[allow(unused_mut)] mut args: Args) -> anyhow::Result<()> {
    if let Some(path) = &args.write_default_conf {
        conf::write_defaults(path)
            .map_err(|e| anyhow::anyhow!("Failed to write settings file {:?}: {}", path, e))?;
        println!("Wrote the shipped settings to {}", path.display());
        return Ok(());
    }

    conf::migrate(args.conf.as_deref());
    conf::set_in_use(args.conf.clone());
    #[cfg(not(target_arch = "wasm32"))]
    if !folder_picker::ensure_game_data(&mut args)? {
        return Ok(());
    }
    if let (mandrake_tag::tfl::Game::Tfl, Some(dir)) = tfl_game::chosen_game(&args) {
        if args.net_host.is_some() || args.net_join.is_some() {
            return tfl_net::run(&args, &dir);
        }
        return tfl_view::run_tfl(&args, &dir);
    }
    if let Some(path) = args
        .film_report
        .as_ref()
        .filter(|_| args.rebuild_first.is_empty())
    {
        film_report::start(path)?;
    }

    if let Some(values) = &args.view_camera {
        screenshot::set_view_camera(values)?;
    }

    info!("Initializing Myth II 3D Client...");
    info!("Tag archive path: {:?}", args.archive);

    if !args.rebuild_first.is_empty() {
        return run_rebuilds(args);
    }
    if args::wants_menu(&args) {
        let result = run_menu(args);
        interface_config::save_at_exit(false);
        return result;
    }

    if args.net_host.is_some() || args.net_join.is_some() {
        let host = args.net_host.is_some();
        if args.headless {
            return if host {
                net_game::run_headless_host(args)
            } else {
                net_game::run_headless_join(args)
            };
        }
        let net = if host {
            net_game::host_and_wait(&args)?
        } else {
            net_game::join_and_wait(&args)?
        };
        let interface = interface_config::load(args.conf.as_deref(), false);
        let film = net.film_header();
        let start = startup::start_network_level(&args, interface, film, net.role)?;
        let result = app::window::run_network_window(args, start, net);
        interface_config::save_at_exit(false);
        return result;
    }

    let start = start_game(&args)?;
    if args.headless || args.screenshot.is_some() {
        return run_headless(args, start);
    }
    let result = run_window(args, start);
    interface_config::save_at_exit(false);
    result
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;

#[cfg(test)]
mod sprite_texel_tests;

#[cfg(test)]
mod interface_blend_tests;

#[cfg(test)]
mod sprite_filter_tests;
