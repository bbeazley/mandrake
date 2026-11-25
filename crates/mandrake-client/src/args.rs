use crate::game_speed;
use crate::ui_scale;
use clap::Parser;
use mandrake_rec::header::MonsterInitializer;
use mandrake_sim::statistics::PermanentRanking;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Water {
    Enhanced,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Fog {
    On,
    Off,
    Engine,
}

impl Fog {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Fog::On => "on",
            Fog::Off => "off",
            Fog::Engine => "engine",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum ReflectionSoftEdge {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CutsceneHd {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum DetailTextures {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum DetailShadows {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Vsync {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum TenBit {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Lighting {
    On,
    #[default]
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ForceEnd {
    pub(crate) win: bool,
    pub(crate) tick: u32,
}

fn parse_force_end(text: &str) -> Result<ForceEnd, String> {
    let (kind, tick) = text
        .split_once('@')
        .ok_or_else(|| "win@TICK or loss@TICK".to_owned())?;
    let win = match kind {
        "win" => true,
        "loss" => false,
        _ => return Err(format!("{kind}: win or loss")),
    };
    let tick = tick.parse().map_err(|e| format!("{tick}: {e}"))?;
    Ok(ForceEnd { win, tick })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum BudgetOverlay {
    Paused,
    Hints,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScriptStep {
    Tutorial,
    Quit,
    Intro,
    Restart,
    Next,
    Skip,
    Menu,
    Pause(u32),
    PauseRestart(u32),
    PauseQuit(u32),
    PauseResume(u32),
    FailNextLoad,
}

fn parse_script_step(text: &str) -> Result<ScriptStep, String> {
    let (word, tick) = match text.split_once('@') {
        Some((word, tick)) => (
            word,
            Some(tick.parse().map_err(|e| format!("{tick}: {e}"))?),
        ),
        None => (text, None),
    };
    Ok(match (word, tick) {
        ("tutorial", None) => ScriptStep::Tutorial,
        ("quit", None) => ScriptStep::Quit,
        ("intro", None) => ScriptStep::Intro,
        ("restart", None) => ScriptStep::Restart,
        ("next", None) => ScriptStep::Next,
        ("skip", None) => ScriptStep::Skip,
        ("menu", None) => ScriptStep::Menu,
        ("fail-next-load", None) => ScriptStep::FailNextLoad,
        ("pause", Some(tick)) => ScriptStep::Pause(tick),
        ("pause-restart", Some(tick)) => ScriptStep::PauseRestart(tick),
        ("pause-quit", Some(tick)) => ScriptStep::PauseQuit(tick),
        ("pause-resume", Some(tick)) => ScriptStep::PauseResume(tick),
        _ => return Err(format!("{text}: no such step")),
    })
}

#[derive(Parser, Debug, Clone)]
#[command(group(clap::ArgGroup::new("session_game").args(["play", "replay"])))]
#[command(
    name = "mandrake",
    author,
    version,
    about = "Myth II: Soulblighter 3D Client"
)]
pub struct Args {
    #[arg(
        short,
        long,
        default_value_os_t = default_archive(),
        help = "Path to tag archive (e.g. 'small install'; default: tags/small install of \
                MYTH2_DATA_DIR, else of the settings file's [game] myth2_data, else of the \
                current folder, else of data/ beside the executable)"
    )]
    pub(crate) archive: PathBuf,

    #[arg(
        long,
        value_name = "GAME",
        value_parser = parse_game,
        help = "The game: soulblighter (Myth II: Soulblighter, the default; myth2 also works) or \
                tfl (Myth: The Fallen Lords). Without it, the settings file's [game] game; \
                without that, a --replay that is a TFL film, or a --tfl-data folder that holds \
                TFL's data, means tfl, and anything else soulblighter"
    )]
    pub(crate) game: Option<mandrake_tag::tfl::Game>,

    #[arg(
        long,
        value_name = "DIR",
        help = "TFL's game folder, the one holding tags/*.gor (default: TFL_DATA_DIR, else the \
                settings file's [game] tfl_data, else TFL in a Steam Proton prefix). A --replay name is looked up in its \
                tags/Local/RECO"
    )]
    pub(crate) tfl_data: Option<PathBuf>,

    #[arg(
        long,
        help = "Path to landscape textures archive (defaults to 'large install' in tags dir)"
    )]
    pub(crate) textures_archive: Option<PathBuf>,

    #[arg(
        short,
        long,
        help = "Mesh ID or substring (a film's own mesh takes its place, see --replay). In a window, \
                with no map named, the run opens on the main menu; --mesh 00tm is the old bare run"
    )]
    pub(crate) mesh: Option<String>,

    #[arg(
        long,
        value_name = "NAME",
        help = "Load this installed plugin over the tags, as a film's plugin list does, to reach \
                a map of its with --mesh (e.g. --plugin 'Patch 1.8.5 Maps' --mesh 84fd). The name \
                is the plugin's file name, as the engine's. May be given more than once. A film names its \
                own plugins, so with --replay only an enhancement pack is taken"
    )]
    pub(crate) plugin: Vec<String>,

    #[arg(
        long,
        value_name = "FILE",
        help = "Test hook (headless): write every unit each frame draws, per render tick, one \
                line per unit"
    )]
    pub(crate) unit_draw_log: Option<PathBuf>,

    #[arg(
        long,
        value_name = "RATIO",
        default_value_t = 0.0,
        help = "Test hook (headless): draw the frame this part of a tick (0..=1) after the last \
                tick. 0 is the first frame after a tick, the original engine's one frame per tick"
    )]
    pub(crate) frame_fraction: f32,

    #[arg(long, help = "Render offscreen and save frame to PNG image")]
    pub(crate) screenshot: Option<PathBuf>,

    #[arg(
        long,
        value_name = "FILE",
        help = "Write the sim's draws (`tick site seed`) to FILE and the end of the game to \
                FILE with the extension .end, to compare two runs. A window \
                then holds at the map's end, with no fade, report or exit"
    )]
    pub(crate) film_report: Option<PathBuf>,

    #[arg(
        long,
        value_name = "SPEED",
        default_value = "1",
        value_parser = game_speed::parse_speed,
        help = "Starting game speed as a multiple of normal: 0 (stopped), 1/16, 1/8, 1/4, 1/2, 1, \
                2, 4, 8, 16, 32 or 64, as F1 and F2 reach in a film. It isn't limited \
                as the keys are. A window runs that many ticks per 30 Hz poll; a headless run \
                still draws one frame a tick, and only its camera interpolation's clock follows"
    )]
    pub(crate) game_speed: i16,

    #[arg(long, help = "Run in headless mode without opening a desktop window")]
    pub(crate) headless: bool,

    #[arg(
        long,
        value_name = "MESH",
        conflicts_with = "replay",
        help = "Play a solo mission on this mesh, as the shell's New Game starts it, e.g. cr01 \
                for Salvation. --seed is the game seed, by default the clock's. With --game \
                tfl always as --session does: New Game's route to the level (its mesh, \
                e.g. 03di, or a piece of its name), then Briefing, Loading, Playing, Postgame"
    )]
    pub(crate) play: Option<String>,

    #[arg(
        long,
        value_name = "HOST:PORT",
        conflicts_with_all = ["replay", "play"],
        help = "Join the TCP/IP game hosted at this address, ready, and play it when the host \
                starts. The player's name, team and colours are the settings file's \
                [net] table, or --net-name. With --headless it plays with no window"
    )]
    pub(crate) net_join: Option<String>,

    #[arg(
        long,
        value_name = "PORT",
        conflicts_with_all = ["replay", "play", "net_join"],
        help = "Host a TCP/IP game on this port (3453 is the engine's): the game of \
                --net-description and --net-map, --net-game-type, --net-time-limit, --net-coop; \
                start once --net-players are in and every joiner is ready, then play it in a \
                window, or with --headless with none"
    )]
    pub(crate) net_host: Option<u16>,

    #[arg(
        long,
        requires = "net_host",
        help = "--net-host's game is co-operative: game option 0x80 over --net-description's. \
                Only a coop host's pause and speed keys pause and change the game's speed"
    )]
    pub(crate) net_coop: bool,

    #[arg(
        long,
        value_name = "FILE",
        requires = "net_host",
        help = "--net-host's game: a type-6 game description message as it travels, \
                by default a new host's"
    )]
    pub(crate) net_description: Option<PathBuf>,

    #[arg(
        long,
        value_name = "MESH",
        requires = "net_host",
        help = "--net-host's map, the mesh tag's four characters, over --net-description's"
    )]
    pub(crate) net_map: Option<String>,

    #[arg(
        long,
        value_name = "N",
        requires = "net_host",
        help = "--net-host's game type (the description's), over --net-description's"
    )]
    pub(crate) net_game_type: Option<i16>,

    #[arg(
        long,
        value_name = "TICKS",
        requires = "net_host",
        help = "--net-host's time limit in ticks (30 a second), over --net-description's"
    )]
    pub(crate) net_time_limit: Option<i32>,

    #[arg(
        long,
        value_name = "N",
        default_value_t = 2,
        help = "--net-host starts once this many players (itself included) have joined and \
                every joiner is ready"
    )]
    pub(crate) net_players: usize,

    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 3.0,
        help = "--net-host starts this long after every player is in and ready"
    )]
    pub(crate) net_start_after: f64,

    #[arg(
        long,
        value_name = "TEXT",
        help = "The game's password for --net-host, the one sent for --net-join"
    )]
    pub(crate) net_password: Option<String>,

    #[arg(
        long,
        value_name = "NAME",
        help = "The player's name in a network game, over the settings file's"
    )]
    pub(crate) net_name: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Where a network game's film goes as well as the last recording (headless runs)"
    )]
    pub(crate) net_film: Option<PathBuf>,

    #[arg(
        long,
        value_name = "FILM",
        help = "A test aid, not a game feature: play FILM's orders in the --net-join \
                or --net-host game, re-timed and moved onto this game's units; its busiest \
                player's are the joiner's, the next one's the host's, and the other side's go \
                to --net-orders-host for a peer running the original engine"
    )]
    pub(crate) net_orders: Option<PathBuf>,

    #[arg(
        long,
        value_name = "PATH",
        requires = "net_orders",
        help = "A test aid: where --net-orders writes the other side's orders as they fall due \
                (`tick kind payload-hex` lines)"
    )]
    pub(crate) net_orders_host: Option<PathBuf>,

    #[arg(
        long,
        value_name = "FILE",
        requires = "net_host",
        help = "--net-host's game is this saved game, restored for everyone as the engine's \
                co-operative host restores a network save (`!ngp`'s saved game)"
    )]
    pub(crate) net_load_save: Option<PathBuf>,

    #[arg(
        long,
        value_name = "TICK",
        requires = "net_save",
        help = "A test aid: a headless co-operative host saves the game at TICK into --net-save"
    )]
    pub(crate) net_save_at: Option<u32>,

    #[arg(
        long,
        value_name = "FILE",
        help = "Where --net-save-at's saved game goes"
    )]
    pub(crate) net_save: Option<PathBuf>,

    #[arg(
        long,
        value_name = "TICK",
        help = "A test aid: a headless network game's player gives every unit it owns \
                to its teammate at TICK, as the Detach Units key does (DETACH)"
    )]
    pub(crate) net_detach_at: Option<u32>,

    #[arg(
        long,
        value_name = "TICKS",
        help = "A test aid: a headless TFL network game's player (--game tfl) orders its own \
                units every TICKS ticks: at the nearest monsters of the other teams (the target \
                command), or, with none left, a move to the middle of the map"
    )]
    pub(crate) net_script_every: Option<u32>,

    #[arg(
        long,
        value_name = "TICK",
        help = "A test aid: a headless TFL network game's player (--game tfl) leaves the game \
                at TICK, as quitting it does: the host drops it (DROP_PLAYER) for the others"
    )]
    pub(crate) net_leave_at: Option<u32>,

    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 600.0,
        help = "A --net-join or --net-host run gives up after this long in the pregame, and a \
                headless one after this long in all"
    )]
    pub(crate) net_timeout: f64,

    #[arg(
        long,
        requires = "session_game",
        conflicts_with_all = ["headless", "screenshot"],
        help = "With --play: run that level inside a session, as New Game with all levels open \
                reaches it: Briefing, Loading, Playing, Postgame, then the menu. With --replay: play \
                that film of the recordings folder from Saved Films, then its report and the menu \
                (--film-report then holds at its end, as with --replay alone). The short way to \
                reach a screen"
    )]
    pub(crate) session: bool,

    #[arg(
        long,
        value_name = "win|loss@TICK",
        value_parser = parse_force_end,
        conflicts_with_all = ["headless", "screenshot", "film_report", "frame_budget", "replay"],
        help = "Test hook: call the sim's end_game at this tick of each game, as a scripted \
                ending does: win@401 or loss@401"
    )]
    pub(crate) force_end: Option<ForceEnd>,

    #[arg(
        long,
        value_delimiter = ',',
        value_name = "STEP,...",
        value_parser = parse_script_step,
        conflicts_with_all = ["headless", "screenshot", "film_report", "frame_budget"],
        help = "Test hook, in a session: answers given in order on the first screen that takes them. \
                The menu: tutorial, quit, intro (Replay Intro). The post-game screen and a film's report: menu; the \
                post-game screen also restart, next, skip. In play \
                from a tick: pause@T, pause-restart@T, pause-quit@T, pause-resume@T. Anywhere: \
                fail-next-load"
    )]
    pub(crate) session_script: Vec<ScriptStep>,

    #[arg(
        long,
        value_delimiter = ',',
        value_name = "MESH,...",
        conflicts_with_all = ["headless", "screenshot", "session"],
        help = "Test hook: before the run's own start, play these meshes in turn for 300 \
                ticks each and drop them, --rebuild-count times in all, logging each cycle's \
                memory, GPU and sound counts to stderr"
    )]
    pub(crate) rebuild_first: Vec<String>,

    #[arg(
        long,
        default_value_t = 1,
        value_name = "N",
        help = "With --rebuild-first: how many maps to play and drop"
    )]
    pub(crate) rebuild_count: u32,

    #[arg(
        long,
        default_value_t = 0,
        value_parser = clap::value_parser!(i16).range(0..=4),
        help = "With --play: 0 Timid, 1 Simple, 2 Normal, 3 Heroic, 4 Legendary (TFL's New Game \
                lists the same five)"
    )]
    pub(crate) difficulty: i16,

    #[arg(
        long,
        value_name = "FILM",
        requires = "play",
        help = "Test hook: with --play, give this film's commands as the player's \
                input, at their ticks"
    )]
    pub(crate) input_film: Option<PathBuf>,

    #[arg(skip)]
    pub(crate) carried_initializers: Vec<MonsterInitializer>,

    #[arg(skip)]
    pub(crate) carried_rankings: Vec<PermanentRanking>,

    #[arg(skip)]
    pub(crate) game_options: Option<u32>,

    #[arg(skip)]
    pub(crate) from_conf: Vec<&'static str>,

    #[arg(
        long,
        help = "Game seed (game parameters +0x18) of the -mesh game: the RNG starts at \
                set_random_seed(seed), 0 -> 0xfded. Default 0, the original engine's -mesh game; the 00tm \
                visual-gate reference is seed 183968. Ignored with --replay (the film's seed)"
    )]
    pub(crate) seed: Option<u32>,

    #[arg(
        long,
        default_value_t = 0,
        help = "Number of 30 Hz simulation ticks to run before capturing headless screenshot"
    )]
    pub(crate) ticks: u32,

    #[arg(
        long,
        value_name = "SCALE",
        help = "Interface scale. `native` (default, parity): the engine's interface at the \
                window's resolution. A zoom level (developer option, not parity, as \
                1.8.5's Pixel Zoom): 1, 1.25, 1.5, 1.75, 2, 2.5, 3 or 4, or in percent (150), \
                lowered while the window can't hold a 640x480 interface. `768p` (developer \
                option, not parity): the interface is laid out on a round(w/s)x768 \
                screen, s = height/768. Scaled, the world view keeps the full resolution. \
                Ctrl + and Ctrl - change the level in a window and save it to \
                the settings file's [display] ui_zoom, used when this flag is absent; headless \
                runs ignore the saved level"
    )]
    pub(crate) ui_scale: Option<ui_scale::UiScaleMode>,

    #[arg(
        long,
        value_enum,
        help = "Upscale filter of a scaled interface: sharp (the default of 768p; sharp bilinear: \
                even texel blocks, antialiased seams), nearest, linear or lanczos2 (the default \
                of a zoom level; separable Lanczos-2, 4x4 texels: smooth edges with a slight \
                sharpening ring). At the menus it wins over menu_filter for this run"
    )]
    pub(crate) ui_filter: Option<ui_scale::UiFilter>,

    #[arg(
        long,
        value_enum,
        help = "Upscale filter of the cutscenes, apart from the menus': fsr1 (AMD FidelityFX \
                Super Resolution 1, EASU then RCAS), sharp, lanczos2, linear or nearest. Wins \
                over the interface file's cutscene_filter (default fsr1) for this run"
    )]
    pub(crate) cutscene_filter: Option<crate::interface_config::CutsceneFilter>,

    #[arg(
        long,
        value_enum,
        help = "Whether a cutscene plays the HD film scripts/upscale-cutscenes.sh made of it \
                ($XDG_DATA_HOME/myth2-sourceport/cutscenes-hd/<name>.ivf, with the .smk's sound) \
                when there is one with the .smk's frame count: on or off. Wins over the \
                interface file's cutscene_hd (default true) for this run"
    )]
    pub(crate) cutscene_hd: Option<CutsceneHd>,

    #[arg(
        long,
        value_enum,
        help = "How The Fallen Lords selects units and orders them: myth2 (Myth II 1.8.5's \
                rules) or tfl (TFL 1.5's). Wins over the interface file's selection (default \
                myth2) for this run"
    )]
    pub(crate) selection: Option<crate::interface_config::SelectionChoice>,

    #[arg(
        long,
        value_enum,
        default_value = "software",
        help = "How the world's textures are sampled: software (the default, the engine's \
                software renderer: no mip levels, unfiltered sprites and models, the landscape's \
                two-pixel filtered spans), linear (the engine's OpenGL renderer: GL_LINEAR and \
                mip chains, the original engine's OpenGL look), bicubic \
                (not parity: Catmull-Rom on landscape and models, Lanczos-2 on sprites) or sharp \
                (not parity: even texel blocks, antialiased seams). bicubic and sharp act while \
                a texture is magnified; minified they are linear"
    )]
    pub(crate) sampling: ui_scale::Sampling,

    #[arg(
        long,
        value_enum,
        help = "Draw the 3D fog of a map with a fog density, in either game: on (height fog with \
                drifting patches), off, or engine (the engine's fog: Myth II's veil by depth, as \
                the original engine draws it; none in The Fallen Lords, as TFL 1.5). Without \
                this option it follows --sampling: engine with linear, which stays the engine's \
                picture, on otherwise"
    )]
    pub(crate) fog: Option<Fog>,

    #[arg(
        long,
        help = "With --fog on, in place of the [fog] table: the fog's opacity looking straight down \
                at its base height, 0 to below 1"
    )]
    pub(crate) fog_opacity: Option<f32>,

    #[arg(
        long,
        help = "With --fog on, in place of the [fog] table: the height over which the fog thins to \
                about a third, as a share of the map's lowest to high ground"
    )]
    pub(crate) fog_falloff: Option<f32>,

    #[arg(
        long,
        allow_hyphen_values = true,
        help = "With --fog on, in place of the [fog] table: the fog's base, in cells above the \
                map's lowest ground"
    )]
    pub(crate) fog_base: Option<f32>,

    #[arg(
        long,
        value_name = "R,G,B",
        value_parser = parse_fog_color,
        help = "With --fog on, in place of the [fog] table: fog colour, each 0 to 1"
    )]
    pub(crate) fog_color: Option<[f32; 3]>,

    #[arg(
        long,
        help = "With --fog on, in place of the [fog] table: how patchy the fog is, 0 for even, up \
                to 1 for clear gaps"
    )]
    pub(crate) fog_patchiness: Option<f32>,

    #[arg(
        long,
        help = "With --fog on, in place of the [fog] table: patch size, in cells"
    )]
    pub(crate) fog_patch_size: Option<f32>,

    #[arg(
        long,
        allow_hyphen_values = true,
        help = "With --fog on, in place of the [fog] table: how fast the patches drift, in cells \
                per second"
    )]
    pub(crate) fog_speed: Option<f32>,

    #[arg(
        long,
        value_parser = parse_fog_share,
        help = "With --fog on, in place of the [fog] table: the share of the ground under the \
                patches, 0 to 1; lower for separate patches with clear ground between"
    )]
    pub(crate) fog_coverage: Option<f32>,

    #[arg(
        long,
        value_parser = parse_fog_softness,
        help = "With --fog on, in place of the [fog] table: how soft a patch's edge is, above 0 up \
                to 1: small for defined banks, 0.5 for wisps"
    )]
    pub(crate) fog_softness: Option<f32>,

    #[arg(
        long,
        value_name = "SECONDS",
        help = "With --fog on: where the patches' clock stands. A window starts here and runs \
                on with the wall clock; a headless frame stands here, by default the tick's time, \
                so the same frame gives the same picture. A screenshot's reproducing line sets it"
    )]
    pub(crate) fog_time: Option<f32>,

    #[arg(
        long,
        value_enum,
        help = "Real-time lighting, in either game: on or off. Without this option: off in Myth \
                II, on in The Fallen Lords"
    )]
    pub(crate) lighting: Option<Lighting>,

    #[arg(
        long,
        value_name = "FACTOR",
        help = "With --lighting on: darkness factor for ambient light (lower means darker ambient)"
    )]
    pub(crate) lighting_darkness: Option<f32>,

    #[arg(
        long,
        value_name = "R,G,B",
        value_parser = parse_ambient_color,
        help = "With --lighting on: ambient light tint, each 0 to 1 as R,G,B (e.g. 0.25,0.32,0.45)"
    )]
    pub(crate) lighting_ambient_color: Option<[f32; 3]>,

    #[arg(
        long,
        value_name = "INTENSITY",
        help = "With --lighting on: point light intensity for projectiles and fire effects"
    )]
    pub(crate) lighting_light_intensity: Option<f32>,

    #[arg(
        long,
        value_enum,
        help = "Fade a reflection over a few pixels where the dry shore is drawn over it (not \
                parity): on or off. Without this option it follows --sampling: off with linear, \
                as the original engine draws it, on otherwise"
    )]
    pub(crate) reflection_soft_edge: Option<ReflectionSoftEdge>,

    #[arg(
        long,
        value_enum,
        help = "Not parity: how the water is drawn. enhanced: the terrain under the water seen \
                through it, a surface with wind waves, reflections through the waves and the \
                sun's glint; off (the default): the engine's look"
    )]
    pub(crate) water: Option<Water>,

    #[arg(
        long,
        value_delimiter = ',',
        value_name = "MESH,...",
        help = "With --water enhanced: only the maps of these mesh ids (e.g. 85tl) draw it, the \
                others the engine's water. Without it every map with water does. The web page's \
                presets use it for the Tarn water demo"
    )]
    pub(crate) water_maps: Vec<String>,

    #[arg(
        long,
        value_enum,
        help = "Draw a plugin's detail textures on the landscape (dmap and dtex tags, such as \
                Magma's Detail Texture Megapack; stock maps have none): on or off. Independent \
                of --sampling: detail draws with every sampling, the software look too (the \
                engine's software renderer has none). Without this option: detail_textures in \
                the settings file, else off"
    )]
    pub(crate) detail_textures: Option<DetailTextures>,

    #[arg(
        long,
        value_enum,
        help = "The Fallen Lords: draw Myth II 1.8.5's detail shadows under its objects (on, the \
                default) or TFL 1.5's shadow bitmaps (off). Myth II always draws its detail \
                shadows, as 1.8.5 does"
    )]
    pub(crate) detail_shadows: Option<DetailShadows>,

    #[arg(
        long,
        value_enum,
        help = "How in-game sprites are sampled while magnified, instead of what --sampling says \
                (developer option, not parity): linear (the engine's GL_LINEAR), sharp \
                (even texel blocks, antialiased seams), nearest or lanczos2 (4x4 texels, smooth \
                edges with a slight ring). Minified sprites keep the engine's mip blend in every \
                mode"
    )]
    pub(crate) sprite_filter: Option<ui_scale::UiFilter>,

    #[arg(
        long,
        help = "Path to .m2rec recording file to play back; a relative path that does not exist \
                is looked up in the game's recordings folder (next to the archive's tags folder)"
    )]
    pub(crate) replay: Option<PathBuf>,

    #[arg(
        long,
        value_name = "SECONDS",
        help = "Developer: turn vsync off, measure a whole frame by stage (sim, CPU \
                build, GPU by pass from timestamp queries, present) for this long after a 2 s \
                warm-up, print average, 99th percentile and maximum against the 4 ms budget, and \
                quit. Windowed only. With --session the level or film is measured inside the \
                session, from BeginPlay"
    )]
    pub(crate) frame_budget: Option<f32>,

    #[arg(
        long,
        value_enum,
        help = "Wait for the display's refresh: on (AutoVsync) or off (Immediate, \
                else Mailbox, else Fifo when the surface has neither). Without this option: vsync \
                in the client settings file, else off, which suits a variable refresh rate \
                display. --frame-budget is always off"
    )]
    pub(crate) vsync: Option<Vsync>,

    #[arg(
        long,
        value_enum,
        help = "Draw the window on a 10-bit surface (Rgb10a2Unorm) where the GPU and the \
                display server offer one: on, or off for the 8-bit sRGB surface with the water's \
                dither. Without this option: ten_bit in the client settings file, else on. The \
                browser is always 8-bit"
    )]
    pub(crate) ten_bit: Option<TenBit>,

    #[arg(
        long,
        value_name = "auto|N|off",
        help = "Cap the window's frame rate: auto (the monitor's refresh rate less 1 \
                (239 at 240 Hz), which keeps a variable refresh rate display in its range; 240 when the rate is \
                unknown), N frames a second, or off (or 0). With vsync on, only a cap below the \
                refresh rate counts. Without this option: fps_cap in the client settings file, \
                else auto. --frame-budget is uncapped"
    )]
    pub(crate) fps_cap: Option<crate::app::fps_cap::FpsCap>,

    #[arg(long, hide = true, value_name = "SECONDS")]
    pub(crate) fps_report: Option<f32>,

    #[arg(long, hide = true, value_name = "TICK")]
    pub(crate) hold_at_tick: Option<u32>,

    #[arg(long, hide = true, value_name = "PASSES")]
    pub(crate) skip_passes: Option<String>,

    #[arg(
        long,
        value_name = "TICK",
        requires = "frame_budget",
        help = "With --frame-budget: play the film at --game-speed to this tick, then measure at \
                normal speed"
    )]
    pub(crate) frame_budget_tick: Option<u32>,

    #[arg(
        long,
        value_name = "TICKS",
        requires = "frame_budget",
        help = "With --frame-budget: instead of one table, print a line per this many game ticks \
                (what is drawn, where the living units are, GPU, build and sim times), to find \
                the heavy part of a film. Runs until stopped"
    )]
    pub(crate) frame_budget_scan: Option<u32>,

    #[arg(
        long,
        requires = "frame_budget",
        help = "With --frame-budget: wait for the GPU after each submit and report the wait, a \
                wall clock check of the timestamps. It serializes the frame, so its whole-frame \
                figure isn't the real one"
    )]
    pub(crate) frame_budget_wait: bool,

    #[arg(
        long,
        value_enum,
        value_name = "DIALOG",
        requires = "frame_budget",
        help = "With --frame-budget: open this dialog over the game where the measuring starts, \
                pressing a button in it every 15 frames, and report its still and redrawn frames. \
                Set the tick with --frame-budget-tick, and compare a run without this flag"
    )]
    pub(crate) frame_budget_overlay: Option<BudgetOverlay>,

    #[arg(
        long,
        value_name = "UNIT",
        requires = "frame_budget",
        num_args = 0..=1,
        default_missing_value = "10084",
        help = "With --frame-budget (a test hook): hold the view centre on this unit \
                each tick, and where it fell after it dies, with the mouse and keys ignored, so \
                that two runs compare. Without a value the dwarf 10084 that salvation_vic_new's \
                benchmark follows. RUST_LOG=frame_budget_follow=info logs the camera each tick"
    )]
    pub(crate) frame_budget_follow: Option<u32>,

    #[arg(long, default_value_t = 1280, help = "Window/frame width")]
    pub(crate) width: u32,

    #[arg(long, default_value_t = 720, help = "Window/frame height")]
    pub(crate) height: u32,

    #[arg(
        long,
        help = "Camera distance (zoom) override; any camera override switches from the \
                authentic map-start observer camera to the legacy orbit camera"
    )]
    pub(crate) zoom: Option<f32>,

    #[arg(
        long,
        allow_hyphen_values = true,
        help = "Camera yaw override in radians"
    )]
    pub(crate) yaw: Option<f32>,

    #[arg(
        long,
        allow_hyphen_values = true,
        help = "Camera pitch override in radians"
    )]
    pub(crate) pitch: Option<f32>,

    #[arg(long, allow_hyphen_values = true, help = "Camera target X override")]
    pub(crate) target_x: Option<f32>,

    #[arg(long, allow_hyphen_values = true, help = "Camera target Z override")]
    pub(crate) target_z: Option<f32>,

    #[arg(
        long,
        help = "Pre-select every unit of this team (e.g. 0) before headless rendering"
    )]
    pub(crate) select_team: Option<u8>,

    #[arg(
        long,
        help = "Pre-select every unit with this unit tag ID (e.g. wari) before headless rendering"
    )]
    pub(crate) select_unit: Option<String>,

    #[arg(
        long,
        value_delimiter = ',',
        allow_hyphen_values = true,
        value_name = "X0,Y0,X1,Y1",
        help = "Headless: drive a left-button drag from (X0,Y0) to (X1,Y1) pixels through the \
                selection state machine (one 30 Hz poll per step, see --band-drag-ticks) and \
                render the last poll: the band rectangle if the drag reached the band state, and \
                the selection it made"
    )]
    pub(crate) band_drag: Option<Vec<f32>>,

    #[arg(
        long,
        default_value_t = 10,
        value_name = "N",
        help = "Headless: polls the --band-drag cursor takes from the press to the end point"
    )]
    pub(crate) band_drag_ticks: u32,

    #[arg(
        long,
        value_delimiter = ',',
        allow_hyphen_values = true,
        value_name = "X,Y",
        help = "Headless: the pointer rests at pixel (X,Y); the monster under it is hovered \
                (update_selection's 250 ms hover refresh) and draws its frame at 0xaaaa"
    )]
    pub(crate) hover: Option<Vec<f32>>,

    #[arg(
        long,
        help = "Headless: the engine's Caps Lock key is down (SDL 1.2 lock state), showing the \
                Caps Lock option's overlays (Selection Boxes: every monster's frame and bar)"
    )]
    pub(crate) caps_lock: bool,

    #[arg(
        long,
        help = "Headless: the score table is toggled on (Shift+F7), \
                drawn with --hud"
    )]
    pub(crate) scores: bool,

    #[arg(
        long,
        help = "Headless: the Show Key Bindings key (F8) is held, so every key binding of the \
                controls (--conf's) shows over the frame, TFL 1.5's F8 list; drawn with --hud \
                in Myth II"
    )]
    pub(crate) key_bindings: bool,

    #[arg(
        long,
        value_delimiter = ',',
        value_name = "ID,...",
        help = "Headless: these units are the targets of a command just given (object flag \
                0x40000000, highlighted for 7 ticks)"
    )]
    pub(crate) command_targets: Option<Vec<u32>>,

    #[arg(
        long,
        help = "Preference 'TFL Style Selection Boxes' (bit 0x10000): 2 px selection frames"
    )]
    pub(crate) tfl_selection_boxes: bool,

    #[arg(
        long,
        value_delimiter = ',',
        allow_hyphen_values = true,
        value_name = "X,Y,FACING,ZOOM",
        help = "Engine observer: location X,Y in cells, facing in radians and zoom, at the \
                map-start height and pitch"
    )]
    pub(crate) observer: Option<Vec<f32>>,

    #[arg(
        long,
        value_delimiter = ',',
        allow_hyphen_values = true,
        value_name = "TX,TY,TZ,DISTANCE,YAW,PITCH",
        help = "Headless: draw the frame from this camera, shake included, as a screenshot's text \
                file records it"
    )]
    pub(crate) view_camera: Option<Vec<f32>>,

    #[arg(
        long,
        value_name = "INDEX",
        help = "Test hook (headless): trigger the mesh's obmo map action INDEX at the \
                load through mandrake-sim's own proc in a mesh-start world (mode 2), for maps whose \
                load stops before its update_map_actions (10la and 11an), to check the \
                camera move"
    )]
    pub(crate) trigger_obmo: Option<usize>,

    #[arg(
        long,
        value_name = "FILE",
        help = "Test hook (headless): draw the objects and landscape stains of this \
                trace of the original engine instead of what the simulation reports, to check \
                the renderer against states the simulation does not reach"
    )]
    pub(crate) projfx_log: Option<PathBuf>,

    #[arg(
        long,
        help = "Headless: the map's particles as the reference engine held them in the captured \
                frame, instead of the client's own, whose positions \
                come from a local random state of their own. A check of the drawing."
    )]
    pub(crate) particle_state: Option<PathBuf>,

    #[arg(
        long,
        value_delimiter = ',',
        value_name = "UNIT,X,Y",
        help = "Headless: move the first unit with this unit tag (e.g. wam1) to world X,Y \
                (512 units per cell) on the ground before rendering"
    )]
    pub(crate) place_unit: Option<Vec<String>>,

    #[arg(
        long,
        help = "Do not spawn the map's unit markers (as the unit-free reference screenshots)"
    )]
    pub(crate) no_units: bool,

    #[arg(long, help = "Disable all audio output (no audio device is opened)")]
    pub(crate) mute: bool,

    #[arg(
        long,
        help = "Master volume multiplier (0.0 = silent, 1.0 = the games' own level). Without it, \
                the settings file's [sound] volume, else 1.0"
    )]
    pub(crate) master_volume: Option<f32>,

    #[arg(
        long,
        help = "Debug: play this decoded `soun` tag id (e.g. 03vd) once at startup"
    )]
    pub(crate) debug_sound: Option<String>,

    #[arg(
        long,
        value_name = "FILE",
        help = "The settings file: every table of mandrake.default.toml, merged over the \
                shipped values, and the [display] table's keys for the flags not given here. A \
                bare name (mandrake.modernretro.toml) is looked up in the config folder \
                ($XDG_CONFIG_HOME/myth2-sourceport, else ~/.config/myth2-sourceport), a path with \
                a folder used as it is; it must exist. Default: mandrake.toml in the config \
                folder, read in a window only (a headless run reads no file unless given this). \
                What the client saves (control bar, Ctrl +/- zoom, multiplayer name) goes to it"
    )]
    pub(crate) conf: Option<PathBuf>,

    #[arg(
        long,
        value_name = "FILE",
        help = "Write the shipped settings, every table with its comments, to this path and exit"
    )]
    pub(crate) write_default_conf: Option<PathBuf>,

    #[arg(
        long,
        help = "Headless: draw the HUD bars (status and control bar). The window always draws \
                them; headless renders leave them out unless asked, like the bar-free visual-gate \
                reference"
    )]
    pub(crate) hud: bool,

    #[arg(
        long,
        value_name = "X,Y",
        allow_hyphen_values = true,
        help = "Headless with --hud: a left click (press and release) at pixel X,Y on the bars \
                before rendering, or with TICK:X,Y in that tick's input poll (a control bar \
                command runs in tick body T + 5, the engine's lead; on the overhead map the \
                camera jumps, set_observer_location); repeatable, in order"
    )]
    pub(crate) hud_click: Vec<String>,

    #[arg(
        long,
        value_name = "X,Y",
        allow_hyphen_values = true,
        help = "Headless with --hud: the left button is held down at pixel X,Y (a held bar \
                button draws pressed)"
    )]
    pub(crate) hud_press: Option<String>,

    #[arg(
        long = "minimap-toggle",
        value_delimiter = ',',
        help = "Headless with --hud: show or hide the overhead map (Tab) in the input poll of \
                these ticks"
    )]
    pub(crate) minimap_toggle: Vec<u32>,

    #[arg(
        long,
        help = "Preference 'translucent bars' (bit 9): the bars blend at transparency 0xb333"
    )]
    pub(crate) translucent_bars: bool,

    #[arg(
        long,
        value_delimiter = ',',
        value_name = "ID,...",
        help = "Headless, or a window's --frame-budget: select these units, in this selection \
                order (the order the move markers read)"
    )]
    pub(crate) select_ids: Option<Vec<u32>>,

    #[arg(
        long,
        value_name = "TICK:X,Y[:shift]",
        help = "Headless: a ground click of the selection after TICK ticks (the engine's order \
                tick), at pixel X,Y (picked with the headless camera) or at `world=X,Y` (engine \
                world units, the engine's picked point); `:shift` holds the waypoint modifier. \
                Sends the move command and makes the move markers; repeatable"
    )]
    pub(crate) move_click: Vec<String>,

    #[arg(
        long,
        value_name = "SCHEDULE",
        help = "Headless: mouse input through the selection state machine, as a scripted \
                input: `TICK:click=BUTTON@X/Y` or `TICK:drag=BUTTON@X0/Y0-X1/Y1/N`, BUTTON \
                `left` or `right`, `;`-separated, pixels in the window. As the original \
                engine polls them, the press is in the poll of tick \
                TICK + 2, the N steps follow one per poll, and the release comes one poll \
                later; the release's orders are carried out"
    )]
    pub(crate) mouse: Option<String>,
}

pub(crate) fn default_archive() -> PathBuf {
    mandrake_common::data_dirs::myth2_dir().join("tags/small install")
}

pub(crate) fn fog_mode(fog: Option<Fog>, sampling: ui_scale::Sampling) -> Fog {
    fog.unwrap_or(if sampling == ui_scale::Sampling::Linear {
        Fog::Engine
    } else {
        Fog::On
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TflExtras {
    pub(crate) detail_shadows: bool,
    pub(crate) fog: bool,
    pub(crate) lighting: bool,
}

impl TflExtras {
    pub(crate) fn of(args: &Args) -> Self {
        Self {
            detail_shadows: args.detail_shadows != Some(DetailShadows::Off),
            fog: !matches!(args.fog, Some(Fog::Off | Fog::Engine)),
            lighting: args.lighting != Some(Lighting::Off),
        }
    }
}

pub(crate) fn parse_game(text: &str) -> Result<mandrake_tag::tfl::Game, String> {
    text.parse()
        .map_err(|_| format!("unknown game '{text}' (expected soulblighter or tfl)"))
}

fn parse_fog_color(text: &str) -> Result<[f32; 3], String> {
    let parts = text
        .split(',')
        .map(|t| t.trim().parse::<f32>().map_err(|e| format!("{t}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    match parts[..] {
        [r, g, b] if parts.iter().all(|v| (0.0..=1.0).contains(v)) => Ok([r, g, b]),
        _ => Err("three numbers from 0 to 1, as R,G,B".to_owned()),
    }
}

fn parse_fog_share(text: &str) -> Result<f32, String> {
    match text.trim().parse::<f32>() {
        Ok(v) if (0.0..=1.0).contains(&v) => Ok(v),
        _ => Err("a number from 0 to 1".to_owned()),
    }
}

fn parse_fog_softness(text: &str) -> Result<f32, String> {
    match text.trim().parse::<f32>() {
        Ok(v) if v > 0.0 && v <= 1.0 => Ok(v),
        _ => Err("a number above 0, up to 1".to_owned()),
    }
}

pub(crate) fn parse_ambient_color(text: &str) -> Result<[f32; 3], String> {
    let parts = text
        .split(',')
        .map(|t| t.trim().parse::<f32>().map_err(|e| format!("{t}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    match parts[..] {
        [r, g, b] if parts.iter().all(|v| (0.0..=1.0).contains(v)) => Ok([r, g, b]),
        _ => Err("three numbers from 0 to 1, as R,G,B".to_owned()),
    }
}

#[cfg(test)]
pub(crate) fn water_enhanced_on(a: &Args, mesh: &str) -> bool {
    water_on(a.water, &a.water_maps, mesh)
}

pub(crate) fn water_on(water: Option<Water>, maps: &[String], mesh: &str) -> bool {
    water == Some(Water::Enhanced) && (maps.is_empty() || maps.iter().any(|m| m == mesh))
}

pub(crate) fn reflection_soft_edge_enabled(
    edge: Option<ReflectionSoftEdge>,
    sampling: ui_scale::Sampling,
) -> bool {
    match edge {
        Some(edge) => edge == ReflectionSoftEdge::On,
        None => sampling != ui_scale::Sampling::Linear,
    }
}

pub(crate) fn detail_textures_enabled(flag: Option<DetailTextures>, saved: Option<bool>) -> bool {
    match flag {
        Some(flag) => flag == DetailTextures::On,
        None => saved.unwrap_or(false),
    }
}

pub(crate) fn vsync_wanted(flag: Option<Vsync>, saved: Option<bool>) -> (bool, &'static str) {
    match (flag, saved) {
        (Some(flag), _) => (flag == Vsync::On, "--vsync"),
        (None, Some(on)) => (on, "the settings file"),
        (None, None) => (false, "the default"),
    }
}

pub(crate) fn ten_bit_wanted(flag: Option<TenBit>, saved: Option<bool>) -> (bool, &'static str) {
    match (flag, saved) {
        (Some(flag), _) => (flag == TenBit::On, "--ten-bit"),
        (None, Some(on)) => (on, "the settings file"),
        (None, None) => (true, "the default"),
    }
}

pub(crate) fn session_error(a: &Args) -> Option<&'static str> {
    (a.session && a.film_report.is_some() && a.replay.is_none())
        .then_some("--session: --film-report needs --replay, a film")
}

pub(crate) fn records_direct_game(a: &Args) -> bool {
    (a.mesh.is_some() || a.play.is_some() || a.net_join.is_some() || a.net_host.is_some())
        && a.replay.is_none()
        && !a.headless
        && a.screenshot.is_none()
        && a.film_report.is_none()
        && a.frame_budget.is_none()
        && a.fps_report.is_none()
}

pub(crate) fn wants_menu(a: &Args) -> bool {
    a.session
        || (a.mesh.is_none()
            && a.play.is_none()
            && a.net_join.is_none()
            && a.net_host.is_none()
            && a.replay.is_none()
            && !a.headless
            && a.screenshot.is_none()
            && a.film_report.is_none()
            && a.frame_budget.is_none())
}

#[cfg(test)]
mod fog_tests {
    use super::*;

    fn mode(extra: &[&str]) -> Fog {
        let mut argv = vec!["mandrake"];
        argv.extend_from_slice(extra);
        let a = Args::try_parse_from(argv).unwrap();
        fog_mode(a.fog, a.sampling)
    }

    #[test]
    fn fog_follows_sampling_and_the_flag_beats_it() {
        assert_eq!(mode(&[]), Fog::On, "software is the default sampling");
        assert_eq!(mode(&["--sampling", "software"]), Fog::On);
        assert_eq!(mode(&["--sampling", "bicubic"]), Fog::On);
        assert_eq!(mode(&["--sampling", "sharp"]), Fog::On);
        assert_eq!(mode(&["--sampling", "linear"]), Fog::Engine);
        assert_eq!(mode(&["--sampling", "linear", "--fog", "on"]), Fog::On);
        assert_eq!(mode(&["--sampling", "linear", "--fog", "off"]), Fog::Off);
        assert_eq!(mode(&["--fog", "engine"]), Fog::Engine);
        assert_eq!(mode(&["--fog", "off"]), Fog::Off);
        assert!(Args::try_parse_from(["mandrake", "--fog", "maybe"]).is_err());
    }

    #[test]
    fn fog_flags_have_no_values_of_their_own() {
        let a = Args::try_parse_from(["mandrake"]).unwrap();
        assert_eq!(a.fog_opacity, None);
        assert_eq!(a.fog_falloff, None);
        assert_eq!(a.fog_base, None);
        assert_eq!(a.fog_color, None);
        assert_eq!(a.fog_patchiness, None);
        assert_eq!(a.fog_patch_size, None);
        assert_eq!(a.fog_speed, None);
        assert_eq!(a.fog_coverage, None);
        assert_eq!(a.fog_softness, None);
    }

    #[test]
    fn cutscene_filter_flag_parses() {
        use crate::interface_config::CutsceneFilter;
        let parse = |extra: &[&str]| {
            Args::try_parse_from(std::iter::once("mandrake").chain(extra.iter().copied()))
                .map(|a| a.cutscene_filter)
        };
        assert_eq!(parse(&[]).unwrap(), None);
        for (value, filter) in [
            ("fsr1", CutsceneFilter::Fsr1),
            ("sharp", CutsceneFilter::Sharp),
            ("lanczos2", CutsceneFilter::Lanczos2),
            ("linear", CutsceneFilter::Linear),
            ("nearest", CutsceneFilter::Nearest),
        ] {
            assert_eq!(parse(&["--cutscene-filter", value]).unwrap(), Some(filter));
        }
        assert!(parse(&["--cutscene-filter", "fsr2"]).is_err());
        let hd = |args: &[&str]| {
            Args::try_parse_from(std::iter::once("mandrake").chain(args.iter().copied()))
                .map(|a| a.cutscene_hd)
        };
        assert_eq!(hd(&[]).unwrap(), None);
        assert_eq!(hd(&["--cutscene-hd", "on"]).unwrap(), Some(CutsceneHd::On));
        assert_eq!(
            hd(&["--cutscene-hd", "off"]).unwrap(),
            Some(CutsceneHd::Off)
        );
        assert!(hd(&["--cutscene-hd", "yes"]).is_err());
        use crate::interface_config::SelectionChoice;
        let selection = |args: &[&str]| {
            Args::try_parse_from(std::iter::once("mandrake").chain(args.iter().copied()))
                .map(|a| a.selection)
        };
        assert_eq!(selection(&[]).unwrap(), None);
        assert_eq!(
            selection(&["--selection", "myth2"]).unwrap(),
            Some(SelectionChoice::Myth2)
        );
        assert_eq!(
            selection(&["--selection", "tfl"]).unwrap(),
            Some(SelectionChoice::Tfl)
        );
        assert!(selection(&["--selection", "tfl15"]).is_err());
    }

    #[test]
    fn a_bare_run_is_the_menu_and_a_named_map_or_tool_is_not() {
        let menu = |extra: &[&str]| {
            let argv = std::iter::once("mandrake").chain(extra.iter().copied());
            wants_menu(&Args::try_parse_from(argv).unwrap())
        };
        assert!(menu(&[]));
        for extra in [
            &["--width", "800", "--height", "600"][..],
            &["--mute"],
            &["--archive", "elsewhere/tags/small install"],
            &["--ui-scale", "768p"],
            &["--ui-filter", "lanczos2"],
            &["--cutscene-filter", "sharp"],
            &["--conf", "a.toml"],
        ] {
            assert!(menu(extra), "{extra:?}");
        }
        for extra in [
            &["--mesh", "00tm"][..],
            &["--play", "le3e"],
            &["--replay", "a.m2rec"],
            &["--headless"],
            &["--screenshot", "a.png"],
            &["--replay", "a.m2rec", "--film-report", "a"],
            &["--film-report", "a"],
            &["--frame-budget", "5"],
        ] {
            assert!(!menu(extra), "{extra:?}");
        }
    }

    #[test]
    fn session_goes_to_the_menus_route_and_the_tools_refuse_the_test_hooks() {
        let parse = |extra: &[&str]| {
            Args::try_parse_from(std::iter::once("mandrake").chain(extra.iter().copied()))
        };
        assert!(wants_menu(
            &parse(&["--session", "--play", "le3e"]).unwrap()
        ));
        assert!(parse(&["--session"]).is_err(), "needs --play or --replay");
        for tool in [
            &["--headless"][..],
            &["--screenshot", "a.png"],
            &["--film-report", "a"],
        ] {
            let with = |flags: &[&str]| parse(&[flags, &["--play", "le3e"], tool].concat());
            let refused = with(&["--session"]).map_or(true, |a| session_error(&a).is_some());
            assert!(refused, "--session {tool:?}");
            assert!(with(&["--force-end", "win@401"]).is_err(), "{tool:?}");
            assert!(with(&["--session-script", "menu"]).is_err(), "{tool:?}");
        }
        let budget = parse(&["--session", "--play", "le3e", "--frame-budget", "5"]).unwrap();
        assert!(wants_menu(&budget) && session_error(&budget).is_none());
        let budget_film =
            parse(&["--session", "--replay", "a.m2rec", "--frame-budget", "5"]).unwrap();
        assert!(wants_menu(&budget_film) && session_error(&budget_film).is_none());
        assert!(
            parse(&[
                "--force-end",
                "win@401",
                "--play",
                "le3e",
                "--frame-budget",
                "5"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--session-script",
                "menu",
                "--play",
                "le3e",
                "--frame-budget",
                "5"
            ])
            .is_err()
        );
        let film = parse(&["--session", "--replay", "a.m2rec", "--film-report", "r"]).unwrap();
        assert!(wants_menu(&film) && session_error(&film).is_none());
        assert!(parse(&["--session", "--replay", "a", "--play", "le3e"]).is_err());
        assert!(parse(&["--rebuild-first", "le3e", "--headless"]).is_err());
        assert!(
            parse(&[
                "--rebuild-first",
                "le3e",
                "--replay",
                "a",
                "--film-report",
                "b"
            ])
            .is_ok()
        );
    }

    #[test]
    fn force_end_and_the_script_read_their_words() {
        let end = |text| parse_force_end(text);
        assert_eq!(
            end("win@401"),
            Ok(ForceEnd {
                win: true,
                tick: 401
            })
        );
        assert_eq!(
            end("loss@7"),
            Ok(ForceEnd {
                win: false,
                tick: 7
            })
        );
        assert!(end("win").is_err() && end("draw@1").is_err() && end("win@x").is_err());
        assert_eq!(parse_script_step("intro"), Ok(ScriptStep::Intro));
        assert_eq!(parse_script_step("next"), Ok(ScriptStep::Next));
        assert_eq!(parse_script_step("pause@40"), Ok(ScriptStep::Pause(40)));
        assert_eq!(
            parse_script_step("pause-quit@9"),
            Ok(ScriptStep::PauseQuit(9))
        );
        assert!(parse_script_step("next@9").is_err());
        assert!(parse_script_step("pause-quit").is_err());
    }

    #[test]
    fn conf_takes_a_path() {
        let a = Args::try_parse_from(["mandrake"]).unwrap();
        assert_eq!(a.conf, None);
        let a = Args::try_parse_from(["mandrake", "--conf", "a.toml"]).unwrap();
        assert_eq!(a.conf, Some(PathBuf::from("a.toml")));
    }

    #[test]
    fn plugin_may_be_given_more_than_once() {
        let a = Args::try_parse_from(["mandrake", "--plugin", "A", "--plugin", "B c"]).unwrap();
        assert_eq!(a.plugin, ["A", "B c"]);
        assert!(
            Args::try_parse_from(["mandrake"])
                .unwrap()
                .plugin
                .is_empty()
        );
    }

    #[test]
    fn fog_color_takes_three_numbers_from_0_to_1() {
        let color =
            |text| Args::try_parse_from(["mandrake", "--fog-color", text]).map(|a| a.fog_color);
        assert_eq!(color("0.28,0.38,0.32").unwrap(), Some([0.28, 0.38, 0.32]));
        assert!(color("0.28,0.38").is_err());
        assert!(color("0.28,0.38,1.5").is_err());
        assert!(color("grey").is_err());
    }

    #[test]
    fn reflection_soft_edge_follows_sampling_and_the_flag_beats_it() {
        let enabled = |extra: &[&str]| {
            let mut argv = vec!["mandrake"];
            argv.extend_from_slice(extra);
            let a = Args::try_parse_from(argv).unwrap();
            reflection_soft_edge_enabled(a.reflection_soft_edge, a.sampling)
        };
        assert!(enabled(&[]));
        assert!(enabled(&["--sampling", "software"]));
        assert!(enabled(&["--sampling", "bicubic"]));
        assert!(enabled(&["--sampling", "sharp"]));
        assert!(!enabled(&["--sampling", "linear"]));
        assert!(enabled(&[
            "--sampling",
            "linear",
            "--reflection-soft-edge",
            "on"
        ]));
        assert!(!enabled(&["--reflection-soft-edge", "off"]));
        assert!(Args::try_parse_from(["mandrake", "--reflection-soft-edge", "maybe"]).is_err());
    }

    #[test]
    fn detail_textures_is_off_by_default_and_ignores_sampling() {
        let enabled = |extra: &[&str], saved: Option<bool>| {
            let mut argv = vec!["mandrake"];
            argv.extend_from_slice(extra);
            let a = Args::try_parse_from(argv).unwrap();
            detail_textures_enabled(a.detail_textures, saved)
        };
        assert!(!enabled(&[], None));
        for sampling in ["software", "linear", "bicubic", "sharp"] {
            assert!(!enabled(&["--sampling", sampling], None));
            assert!(enabled(
                &["--sampling", sampling, "--detail-textures", "on"],
                None
            ));
            assert!(enabled(&["--sampling", sampling], Some(true)));
        }
        assert!(enabled(&["--detail-textures", "on"], Some(false)));
        assert!(!enabled(&["--detail-textures", "off"], Some(true)));
        assert!(!enabled(&[], Some(false)));
        assert!(Args::try_parse_from(["mandrake", "--detail-textures", "maybe"]).is_err());
    }

    #[test]
    fn vsync_is_off_by_default_and_the_flag_wins() {
        let wanted = |extra: &[&str], saved: Option<bool>| {
            let mut argv = vec!["mandrake"];
            argv.extend_from_slice(extra);
            let a = Args::try_parse_from(argv).unwrap();
            vsync_wanted(a.vsync, saved)
        };
        assert_eq!(wanted(&[], None), (false, "the default"));
        assert_eq!(wanted(&[], Some(true)), (true, "the settings file"));
        assert_eq!(wanted(&[], Some(false)), (false, "the settings file"));
        assert_eq!(wanted(&["--vsync", "on"], None), (true, "--vsync"));
        assert_eq!(wanted(&["--vsync", "on"], Some(false)), (true, "--vsync"));
        assert_eq!(wanted(&["--vsync", "off"], Some(true)), (false, "--vsync"));
        assert!(Args::try_parse_from(["mandrake", "--vsync", "maybe"]).is_err());
        assert!(Args::try_parse_from(["mandrake", "--vsync"]).is_err());
    }

    #[test]
    fn ten_bit_is_on_by_default_and_the_flag_wins() {
        let wanted = |extra: &[&str], saved: Option<bool>| {
            let mut argv = vec!["mandrake"];
            argv.extend_from_slice(extra);
            let a = Args::try_parse_from(argv).unwrap();
            ten_bit_wanted(a.ten_bit, saved)
        };
        assert_eq!(wanted(&[], None), (true, "the default"));
        assert_eq!(wanted(&[], Some(false)), (false, "the settings file"));
        assert_eq!(
            wanted(&["--ten-bit", "off"], Some(true)),
            (false, "--ten-bit")
        );
        assert_eq!(
            wanted(&["--ten-bit", "on"], Some(false)),
            (true, "--ten-bit")
        );
        assert!(Args::try_parse_from(["mandrake", "--ten-bit", "maybe"]).is_err());
    }

    #[test]
    fn fps_cap_parses() {
        use crate::app::fps_cap::FpsCap;
        let cap = |argv: &[&str]| {
            let argv = [&["mandrake"], argv].concat();
            Args::try_parse_from(argv).map(|a| a.fps_cap)
        };
        assert_eq!(cap(&[]).unwrap(), None);
        assert_eq!(cap(&["--fps-cap", "auto"]).unwrap(), Some(FpsCap::Auto));
        assert_eq!(
            cap(&["--fps-cap", "120"]).unwrap(),
            Some(FpsCap::Fixed(120))
        );
        assert_eq!(cap(&["--fps-cap", "off"]).unwrap(), Some(FpsCap::Off));
        assert_eq!(cap(&["--fps-cap", "0"]).unwrap(), Some(FpsCap::Off));
        assert!(cap(&["--fps-cap", "fast"]).is_err());
        assert!(cap(&["--fps-cap"]).is_err());
        let a = Args::try_parse_from(["mandrake", "--fps-report", "10"]).unwrap();
        assert_eq!(a.fps_report, Some(10.0));
    }

    #[test]
    fn lighting_args_default_and_flags() {
        let a = Args::try_parse_from(["mandrake"]).unwrap();
        assert_eq!(a.lighting, None);
        assert_eq!(a.lighting_darkness, None);
        assert_eq!(a.lighting_ambient_color, None);
        assert_eq!(a.lighting_light_intensity, None);

        let a = Args::try_parse_from([
            "mandrake",
            "--lighting",
            "on",
            "--lighting-darkness",
            "0.2",
            "--lighting-ambient-color",
            "0.25,0.32,0.45",
            "--lighting-light-intensity",
            "0.85",
        ])
        .unwrap();
        assert_eq!(a.lighting, Some(Lighting::On));
        assert_eq!(a.lighting_darkness, Some(0.2));
        assert_eq!(a.lighting_ambient_color, Some([0.25, 0.32, 0.45]));
        assert_eq!(a.lighting_light_intensity, Some(0.85));
    }
}

#[cfg(test)]
mod water_maps_tests {
    use super::*;

    fn on(extra: &[&str], mesh: &str) -> bool {
        let mut argv = vec!["mandrake"];
        argv.extend_from_slice(extra);
        water_enhanced_on(&Args::try_parse_from(argv).unwrap(), mesh)
    }

    #[test]
    fn water_maps_narrow_the_enhanced_water() {
        assert!(!on(&[], "85tl"), "off by default");
        assert!(on(&["--water", "enhanced"], "85tl"));
        assert!(
            on(&["--water", "enhanced"], "01bt"),
            "every map without a list"
        );
        let listed = ["--water", "enhanced", "--water-maps", "85tl,85gs"];
        assert!(on(&listed, "85tl") && on(&listed, "85gs"));
        assert!(!on(&listed, "01bt"));
        assert!(
            !on(&["--water-maps", "85tl"], "85tl"),
            "the list alone draws nothing"
        );
        assert!(!on(&["--water", "off", "--water-maps", "85tl"], "85tl"));
    }
}
