use std::cell::RefCell;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use mandrake_sim::Simulation;
use mandrake_sim::engine_rng::DRAW_LOG;

const HELD_POLLS: u32 = 300;

struct Report {
    draws: BufWriter<File>,
    end_path: PathBuf,
    count: u64,
    stopped_polls: Option<u32>,
    stop_reported: bool,
    at_stop: (u32, u32, u64),
}

thread_local! {
    static REPORT: RefCell<Option<Report>> = const { RefCell::new(None) };
}

pub fn start(path: &Path) -> std::io::Result<()> {
    let report = Report {
        draws: BufWriter::new(File::create(path)?),
        end_path: path.with_extension("end"),
        count: 0,
        stopped_polls: None,
        stop_reported: false,
        at_stop: (0, 0, 0),
    };
    DRAW_LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
    REPORT.with(|r| *r.borrow_mut() = Some(report));
    Ok(())
}

fn flush(report: &mut Report, tick: i64) {
    let lines = DRAW_LOG.with(|l| l.borrow_mut().as_mut().map(std::mem::take));
    for (site, seed) in lines.unwrap_or_default() {
        report.count += 1;
        if let Err(e) = writeln!(report.draws, "{tick} {site:#x} {seed:#x}") {
            tracing::error!("--film-report: {e}");
        }
    }
}

fn state_line(sim: &Simulation, draws: u64) -> String {
    let end = sim.game_end();
    let game = Some(&sim.engine.game);
    format!(
        "end_tick={:?} winner={:?} code={:?} state={:?} timer={:?} flags={:#x} report_open={:?} \
         next_tick={} rng={} checksum={} draws={draws} stop={:?}\n",
        end.map(|e| e.tick),
        end.map(|e| e.winner),
        end.map(|e| e.code),
        game.map(|g| g.state),
        game.map(|g| g.timer),
        game.map_or(0, |g| g.flags),
        game.map(|g| g.carnage_report_open),
        sim.current_tick,
        sim.rng.state,
        sim.state_checksum,
        sim.engine_stop.as_ref().map(ToString::to_string),
    )
}

pub fn step(sim: &mut Simulation) {
    let on = REPORT.with(|r| r.borrow().is_some());
    if !on {
        sim.step();
        return;
    }
    let tick = i64::from(sim.current_tick);
    REPORT.with(|r| {
        if let Some(report) = r.borrow_mut().as_mut() {
            flush(report, -1);
        }
    });
    sim.step();
    REPORT.with(|r| {
        let mut r = r.borrow_mut();
        let Some(report) = r.as_mut() else {
            return;
        };
        flush(report, tick);
        observe_stop(report, sim);
    });
}

pub fn idle_poll(sim: &Simulation) {
    REPORT.with(|r| {
        if let Some(report) = r.borrow_mut().as_mut() {
            flush(report, i64::from(sim.current_tick));
            observe_stop(report, sim);
        }
    });
}

fn observe_stop(report: &mut Report, sim: &Simulation) {
    if !sim.ticks_stopped() && sim.engine_stop.is_none() {
        return;
    }
    let line = state_line(sim, report.count);
    let polls = report.stopped_polls.get_or_insert(0);
    *polls += 1;
    let polls = *polls;
    let now = (sim.current_tick, sim.rng.state, report.count);
    if !report.stop_reported {
        report.stop_reported = true;
        report.at_stop = now;
        tracing::info!("film report: the sim stopped running ticks: {line}");
        let _ = report.draws.flush();
        let _ = std::fs::write(&report.end_path, format!("stopped {line}"));
    } else if polls == HELD_POLLS {
        let verdict = if now == report.at_stop {
            "held"
        } else {
            "MOVED"
        };
        tracing::info!("film report: {verdict} after {polls} polls: {line}");
        let _ = report.draws.flush();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .open(&report.end_path)
        {
            let _ = write!(f, "{verdict} after {polls} polls {line}");
        }
    }
}

pub fn finish(sim: &Simulation) {
    REPORT.with(|r| {
        if let Some(report) = r.borrow_mut().as_mut() {
            let _ = report.draws.flush();
            if !report.stop_reported {
                let line = state_line(sim, report.count);
                let _ = std::fs::write(&report.end_path, format!("running {line}"));
            }
        }
    });
}
