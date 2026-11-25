use std::path::PathBuf;
use std::process::ExitCode;

use mandrake_tfl_sim::{Stop, TflSim};

fn tfl_dir() -> Option<PathBuf> {
    let d = mandrake_common::data_dirs::tfl_dir();
    d.join("Myth_TFL.exe").is_file().then_some(d)
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    fn i16(&mut self, v: i16) {
        self.bytes(&v.to_le_bytes());
    }
}

fn draws_hash(sim: &TflSim) -> u64 {
    let mut h = Fnv::new();
    for d in &sim.rng.log {
        h.u32(d.site);
        h.u32(d.state);
    }
    h.0
}

fn state_hash(sim: &TflSim) -> u64 {
    let mut h = Fnv::new();
    h.u32(sim.tick);
    h.u32(sim.rng.state());
    h.i16(sim.winning_team);
    h.u32(sim.monsters.len() as u32);
    for m in &sim.monsters {
        h.i16(m.identifier);
        h.bytes(&m.definition.0);
        h.u32(m.object as u32);
        h.i16(m.team);
        h.u32(m.flags);
        h.bytes(&[m.action, m.substate]);
        h.bytes(&m.facing.to_le_bytes());
    }
    h.u32(sim.objects.len() as u32);
    for o in &sim.objects {
        h.i32(o.x);
        h.i32(o.y);
        h.i32(o.z);
        h.u32(o.flags);
        h.bytes(&o.facing.to_le_bytes());
        h.i16(o.sequence);
    }
    h.0
}

fn set_up(
    dir: &std::path::Path,
    film: &[u8],
) -> Result<(TflSim, Option<Stop>, &'static str), Stop> {
    match TflSim::new_recording(dir, film) {
        Ok(sim) => Ok((sim, None, "new")),
        Err(new_stop) => match TflSim::set_up_recording(dir, film) {
            Ok(sim) => Ok((sim, None, "set_up")),
            Err(b) => match *b {
                (Some(sim), s) => Ok((sim, Some(s), "set_up")),
                (None, s) => Err(if s.site == 0 { s } else { new_stop }),
            },
        },
    }
}

fn one_line(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn main() -> ExitCode {
    let films: Vec<String> = std::env::args().skip(1).collect();
    if films.is_empty() {
        eprintln!("usage: tfl_film_bench <film> [<film>...]");
        return ExitCode::from(2);
    }
    let Some(dir) = tfl_dir() else {
        println!(
            "skip: no TFL data (TFL_DATA_DIR, or {} without Myth_TFL.exe)",
            mandrake_common::data_dirs::tfl_dir().display()
        );
        return ExitCode::SUCCESS;
    };
    let mut failed = false;
    for path in &films {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{path}: {e}");
                failed = true;
                continue;
            }
        };
        let (mut sim, mut stop, how) = match set_up(&dir, &bytes) {
            Ok(r) => r,
            Err(s) => {
                println!(
                    "row\t{path}\t-\t0\t-\t0\t-\t-\t{:#x}\t{}",
                    s.site,
                    one_line(&s.reason)
                );
                continue;
            }
        };
        let end = sim.data.film.header.end_tick;
        while stop.is_none() && sim.tick < end {
            stop = sim.step().err();
        }
        let (site, reason) = match &stop {
            Some(s) => (format!("{:#x}", s.site), one_line(&s.reason)),
            None => ("-".into(), format!("end at tick {}", sim.tick)),
        };
        println!(
            "row\t{path}\t{how}\t{}\t{end}\t{}\t{:016x}\t{:016x}\t{site}\t{reason}",
            sim.tick,
            sim.rng.log.len(),
            draws_hash(&sim),
            state_hash(&sim),
        );
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
