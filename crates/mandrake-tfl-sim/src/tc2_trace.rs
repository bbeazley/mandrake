#[cfg(test)]
thread_local! {
    static LOG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[inline]
pub(crate) fn log(line: impl FnOnce() -> String) {
    #[cfg(test)]
    LOG.with(|l| l.borrow_mut().push(line()));
    #[cfg(not(test))]
    let _ = line;
}

#[cfg(test)]
mod tests {
    use crate::TflSim;

    #[test]
    fn tc2_trace_film() {
        let Some(path) = std::env::var_os("TC2_FILM") else {
            return;
        };
        let dir = mandrake_common::data_dirs::tfl_dir();
        let film = std::fs::read(path).unwrap();
        let mut sim = TflSim::new(&dir, &film).unwrap();
        let end = sim.data.film.tick_commands.last().map_or(0, |g| g.tick);
        let ticks: u32 = std::env::var("TC2_TICKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(end);
        let draws: Option<(u32, u32)> = std::env::var("TC2_DRAWS").ok().and_then(|v| {
            let (a, b) = v.split_once("..")?;
            Some((a.parse().ok()?, b.parse().ok()?))
        });
        while sim.tick < ticks {
            let t = sim.tick;
            let n = sim.rng.log.len();
            let r = sim.step();
            if let Some((a, b)) = draws
                && (a..=b).contains(&t)
            {
                for (k, d) in sim.rng.log[n..].iter().enumerate() {
                    println!("DR {t} {} {d:x?}", n + k);
                }
            }
            super::LOG.with(|l| {
                for line in l.borrow_mut().drain(..) {
                    println!("{line}");
                }
            });
            if let Err(s) = r {
                println!("STOP {t} {s:?}");
                break;
            }
        }
        println!("END {}", sim.tick);
    }
}
