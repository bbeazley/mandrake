use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::ffi::c_long;
use std::fmt::Write as _;
use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::os::fd::FromRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use mandrake_common::{FourCC, NativeVfs};
use mandrake_rec::Recording;
use mandrake_sim::Simulation;
use mandrake_sim::engine_rng::{DRAW_LOG, GameParameters};
use mandrake_sim::players::GameSetup;
use mandrake_tag::{MeshTag, TagArchive, engine_tag_files};

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe extern "C" {
    fn syscall(n: c_long, ...) -> c_long;
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[repr(C)]
struct PerfAttr {
    kind: u32,
    size: u32,
    config: u64,
    sample: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    rest: [u64; 10],
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn instruction_counter() -> Option<File> {
    const SYS_PERF_EVENT_OPEN: c_long = 298;
    let attr = PerfAttr {
        kind: 0,
        size: size_of::<PerfAttr>() as u32,
        config: 1,
        sample: 0,
        sample_type: 0,
        read_format: 0,
        flags: (1 << 5) | (1 << 6),
        rest: [0; 10],
    };
    let fd = unsafe {
        syscall(
            SYS_PERF_EVENT_OPEN,
            &raw const attr,
            0 as c_long,
            -1 as c_long,
            -1 as c_long,
            0 as c_long,
        )
    };
    (fd >= 0).then(|| unsafe { File::from_raw_fd(fd as i32) })
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn instruction_counter() -> Option<File> {
    None
}

fn read_counter(f: &mut Option<File>) -> u64 {
    let mut b = [0u8; 8];
    if let Some(f) = f {
        f.read_exact(&mut b).unwrap();
    }
    u64::from_ne_bytes(b)
}

fn debug_hash(v: &dyn std::fmt::Debug) -> u64 {
    let mut h = DefaultHasher::new();
    format!("{v:?}").hash(&mut h);
    h.finish()
}

fn strip_mover_fields(mut s: String) -> String {
    for field in [
        "needs_leg: ",
        "backwards: ",
        "final_facing: ",
        "detour: ",
        "detour_side: ",
    ] {
        let pat = format!(", {field}");
        while let Some(at) = s.find(&pat) {
            let start = at + pat.len();
            let mut depth = 0i32;
            let mut end = s.len();
            for (k, c) in s[start..].char_indices() {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' if depth > 0 => depth -= 1,
                    ',' | ' ' | '}' if depth == 0 => {
                        end = start + k;
                        break;
                    }
                    _ => {}
                }
            }
            s.replace_range(at..end, "");
        }
    }
    s
}

fn state_line(text: &mut String, t: u32, sim: &Simulation, mask: bool) {
    let draws = DRAW_LOG
        .with(|l| l.borrow_mut().replace(Vec::new()))
        .unwrap();
    let mut units: Vec<_> = sim.units.iter().collect();
    units.sort_by_key(|(id, _)| **id);
    let w = &sim.engine;
    let (monsters, units) = if mask {
        let mut monsters = w.monsters.clone();
        for m in &mut monsters {
            m.flags &= !(0x20 | 0x80 | 0x800 | 0x1000);
            m.flags_8 &= !0x10_0000;
        }
        let mut h = DefaultHasher::new();
        strip_mover_fields(format!("{units:?}")).hash(&mut h);
        (debug_hash(&monsters), h.finish())
    } else {
        (debug_hash(&w.monsters), debug_hash(&units))
    };
    writeln!(
        text,
        "{t} rng {:#x} sum {:#x} draws {} {:x} monsters {:x} scenery {:x} history {:x} \
         flights {} {:x} dormant {:x} units {:x}",
        sim.rng.state,
        sim.state_checksum,
        draws.len(),
        debug_hash(&draws),
        monsters,
        debug_hash(&w.scenery),
        debug_hash(&w.object_history),
        sim.combat.flight.flights.len(),
        debug_hash(&sim.combat.flight.flights),
        debug_hash(&sim.combat.flight.dormant),
        units,
    )
    .unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let film = &args[1];
    let out = &args[2];
    let ctl = (args.get(3).map(String::as_str) == Some("--ctl")).then(|| {
        let from: u32 = args[5].parse().unwrap();
        let to: u32 = args[6].parse().unwrap();
        (
            File::options().write(true).open(&args[4]).unwrap(),
            from,
            to,
        )
    });
    let game = std::env::var("MYTH2_DATA_DIR").unwrap();
    let game = Path::new(&game);
    let path = game.join("recordings").join(format!("{film}.m2rec"));
    let bytes = std::fs::read(&path).unwrap();
    let rec = Recording::open(&mut File::open(&path).unwrap()).unwrap();
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
    let state = args.get(3).map(String::as_str) == Some("--state");
    let mask = args.iter().any(|a| a == "--no-mover-flags");
    if state {
        DRAW_LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
    }
    let mut sim = Simulation::from_recording(&rec);
    sim.game_setup = Some(GameSetup::from_film(&bytes).unwrap());
    sim.load_engine_map(&data, &tags, params).unwrap();
    sim.load_mesh(&mesh);
    sim.spawn_initial_units_from_mesh(&mesh);
    sim.attach_engine_movement(&mesh, &tags);

    let mut counter = instruction_counter();
    let mut text = String::from("tick\tns\tinstructions\tallocs\n");
    let mut ctl = ctl;
    while !sim.ticks_stopped() && sim.engine_stop.is_none() {
        let t = sim.current_tick;
        if let Some((f, from, to)) = &mut ctl {
            if t == *from {
                f.write_all(b"enable\n").unwrap();
            } else if t == *to {
                f.write_all(b"disable\n").unwrap();
            }
        }
        if state {
            sim.step_with_commands();
            state_line(&mut text, t, &sim, mask);
            continue;
        }
        let a0 = ALLOCS.load(Ordering::Relaxed);
        let i0 = read_counter(&mut counter);
        let t0 = Instant::now();
        sim.step_with_commands();
        let ns = t0.elapsed().as_nanos();
        let i1 = read_counter(&mut counter);
        let a1 = ALLOCS.load(Ordering::Relaxed);
        writeln!(text, "{t}\t{ns}\t{}\t{}", i1 - i0, a1 - a0).unwrap();
    }
    std::fs::write(out, text).unwrap();
}
