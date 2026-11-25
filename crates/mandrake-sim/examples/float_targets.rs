use std::fmt::Write as _;
use std::path::Path;

use mandrake_common::{FourCC, NativeVfs};
use mandrake_rec::Recording;
use mandrake_sim::Simulation;
use mandrake_sim::engine_rng::{DRAW_LOG, GameParameters};
use mandrake_sim::float_targets::{Fnv, hashes};
use mandrake_sim::players::GameSetup;
use mandrake_tag::{MeshTag, TagArchive, engine_tag_files};

fn debug_hash(v: &dyn std::fmt::Debug) -> u64 {
    let mut h = Fnv::default();
    h.bytes(format!("{v:?}").as_bytes());
    h.0
}

fn state_line(text: &mut String, t: u32, sim: &Simulation) {
    let draws = DRAW_LOG
        .with(|l| l.borrow_mut().replace(Vec::new()))
        .unwrap();
    let mut units: Vec<_> = sim.units.iter().collect();
    units.sort_by_key(|(id, _)| **id);
    let w = &sim.engine;
    writeln!(
        text,
        "{t} rng {:#x} sum {:#x} draws {} {:x} monsters {:x} scenery {:x} history {:x} \
         flights {} {:x} dormant {:x} units {:x}",
        sim.rng.state,
        sim.state_checksum,
        draws.len(),
        debug_hash(&draws),
        debug_hash(&w.monsters),
        debug_hash(&w.scenery),
        debug_hash(&w.object_history),
        sim.combat.flight.flights.len(),
        debug_hash(&sim.combat.flight.flights),
        debug_hash(&sim.combat.flight.dormant),
        debug_hash(&units),
    )
    .unwrap();
}

fn film(path: &Path, game: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let rec = Recording::open(&mut std::io::Cursor::new(&bytes)).unwrap();
    let plugins: Vec<&str> = rec
        .header
        .active_plugins
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    let files = engine_tag_files(game, &plugins);
    let vfs = NativeVfs::default_cwd();
    let archives: Vec<TagArchive> = files
        .paths
        .iter()
        .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
        .collect();
    let tags = |g: [u8; 4], id: FourCC| {
        archives.iter().rev().find_map(|a| {
            a.find_tag_by_id(FourCC::new(g), id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    };
    let data = tags(*b"mesh", rec.header.mesh_tag_id).unwrap();
    let mesh = MeshTag::from_bytes(&data).unwrap();
    let params = GameParameters::from_film(&bytes).unwrap();
    DRAW_LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
    let mut sim = Simulation::from_recording(&rec);
    sim.game_setup = Some(GameSetup::from_film(&bytes).unwrap());
    sim.load_engine_map(&data, &tags, params).unwrap();
    sim.load_mesh(&mesh);
    sim.spawn_initial_units_from_mesh(&mesh);
    sim.attach_engine_movement(&mesh, &tags);
    let mut text = String::new();
    while !sim.ticks_stopped() && sim.engine_stop.is_none() {
        let t = sim.current_tick;
        sim.step_with_commands();
        state_line(&mut text, t, &sim);
    }
    writeln!(text, "end {} stop {:?}", sim.current_tick, sim.engine_stop).unwrap();
    print!("{text}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("hashes") => {
            let stride = args.get(2).map_or(1, |s| s.parse().unwrap());
            for (name, h) in hashes(stride) {
                println!("{name}\t{h:016x}");
            }
        }
        Some("film") => film(Path::new(&args[2]), Path::new(&args[3])),
        _ if cfg!(all(target_arch = "wasm32", target_os = "unknown")) => {}
        _ => {
            eprintln!("usage: float_targets hashes [STRIDE] | film FILM DATA_DIR");
            std::process::exit(2);
        }
    }
}

thread_local! {
    static HASHES: std::cell::RefCell<Vec<(&'static str, u64)>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[unsafe(no_mangle)]
pub extern "C" fn hash_stride(stride: u32) -> u32 {
    HASHES.with(|h| {
        *h.borrow_mut() = hashes(stride);
        h.borrow().len() as u32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn hash(i: u32) -> u64 {
    HASHES.with(|h| h.borrow()[i as usize].1)
}

#[unsafe(no_mangle)]
pub extern "C" fn hash_name(i: u32) -> u64 {
    HASHES.with(|h| {
        let n = h.borrow()[i as usize].0;
        (n.as_ptr() as u64) << 32 | n.len() as u64
    })
}
