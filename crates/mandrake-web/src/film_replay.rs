use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Cursor;

use mandrake_common::FourCC;
use mandrake_rec::Recording;
use mandrake_rec::command::CommandPayload;
use mandrake_sim::engine_rng::{DRAW_LOG, GameParameters};
use mandrake_sim::players::GameSetup;
use mandrake_sim::{MapSource, Simulation};
use mandrake_tag::tag_files::{TagFileHead, order_engine_tag_files};
use mandrake_tag::{MeshTag, TagArchive};

pub const DRAWS_TO: u32 = 60000;
const SNAPSHOT_EVERY: u32 = 512;

pub fn film_plugins(film: &[u8]) -> Result<Vec<String>, String> {
    let rec = Recording::open(&mut Cursor::new(film)).map_err(|e| e.to_string())?;
    Ok(rec
        .header
        .active_plugins
        .iter()
        .map(|p| p.name.clone())
        .collect())
}

pub fn split_plugin_heads(
    plugin_names: Vec<String>,
    plugin_heads: &[u8],
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let n = mandrake_tag::SBMonoHeader::SIZE;
    let expected = plugin_names
        .len()
        .checked_mul(n)
        .ok_or_else(|| "plugin_heads length overflow".to_string())?;
    if plugin_heads.len() != expected {
        return Err(format!(
            "plugin_heads length {} does not match {} plugins * {} bytes",
            plugin_heads.len(),
            plugin_names.len(),
            n
        ));
    }
    Ok(plugin_names
        .into_iter()
        .zip(plugin_heads.chunks_exact(n))
        .map(|(name, h)| (name, h.to_vec()))
        .collect())
}

pub fn order_tag_files(
    tags: &[String],
    plugins: &[(String, Vec<u8>)],
    film_plugins: &[String],
    metaserver: &str,
) -> Result<Vec<String>, String> {
    let mut installed: Vec<TagFileHead> = plugins
        .iter()
        .filter_map(|(name, head)| {
            TagFileHead::read(format!("plugins/{name}").into(), name.clone(), head)
        })
        .collect();
    installed.sort_by(|a, b| a.name.cmp(&b.name));
    let film_plugins: Vec<&str> = film_plugins.iter().map(String::as_str).collect();
    let files = order_engine_tag_files(
        &|n| {
            tags.iter()
                .any(|t| t == n)
                .then(|| format!("tags/{n}").into())
        },
        &installed,
        &film_plugins,
        metaserver,
    );
    let missing: Vec<String> = files
        .missing_plugins
        .iter()
        .map(|p| format!("plugin {p}"))
        .chain(files.missing_patches.iter().map(|p| format!("patch {p}")))
        .collect();
    if missing.is_empty() {
        Ok(files
            .paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect())
    } else {
        Err(format!(
            "not the engine's tag set: missing {}",
            missing.join(", ")
        ))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReplayReport {
    pub draws: u64,
    pub draws_sha256: String,
    pub first_sync_miss: String,
    pub end: String,
    pub ticks: u32,
    pub state_checksum: u64,
    pub rng_state: u32,
    pub units: usize,
    pub units_sha256: String,
    pub tags_used: usize,
    pub tag_bytes_used: usize,
    pub archives_ms: f64,
    pub map_load_ms: f64,
    pub tick_ms: Vec<f64>,
}

impl ReplayReport {
    pub fn tick_stats(&self) -> (f64, f64, f64, f64, f64, usize) {
        let mut v = self.tick_ms.clone();
        if v.is_empty() {
            return (0.0, 0.0, 0.0, 0.0, 0.0, 0);
        }
        let total: f64 = v.iter().sum();
        let slowest = self
            .tick_ms
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i);
        v.sort_by(f64::total_cmp);
        let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        (
            total / v.len() as f64,
            q(0.5),
            q(0.99),
            v[v.len() - 1],
            total,
            slowest,
        )
    }

    pub fn heaviest_window(&self, window: usize) -> (f64, usize) {
        if self.tick_ms.len() < window || window == 0 {
            return (0.0, 0);
        }
        let mut sum: f64 = self.tick_ms[..window].iter().sum();
        let (mut best, mut at) = (sum, 0);
        for i in window..self.tick_ms.len() {
            sum += self.tick_ms[i] - self.tick_ms[i - window];
            if sum > best {
                best = sum;
                at = i + 1 - window;
            }
        }
        (best / window as f64, at)
    }

    pub fn summary(&self) -> String {
        let (mean, p50, p99, max, total, slowest) = self.tick_stats();
        let (heavy, heavy_at) = self.heaviest_window(512);
        format!(
            "draws={} draws_sha256={} first_sync_miss={} ticks={} state_checksum={:#018x} \
             rng_state={:#010x} units={} units_sha256={} archives_ms={:.1} map_load_ms={:.1} \
             tick_mean_ms={mean:.4} tick_p50_ms={p50:.4} tick_p99_ms={p99:.4} tick_max_ms={max:.3} \
             slowest_tick={slowest} heaviest512_mean_ms={heavy:.4} heaviest512_from={heavy_at} \
             ticks_total_ms={total:.0} tags_used={} tag_bytes_used={} end=\"{}\"",
            self.draws,
            self.draws_sha256,
            self.first_sync_miss,
            self.ticks,
            self.state_checksum,
            self.rng_state,
            self.units,
            self.units_sha256,
            self.archives_ms,
            self.map_load_ms,
            self.tags_used,
            self.tag_bytes_used,
            self.end,
        )
    }
}

pub fn replay(
    film: &[u8],
    archives: Vec<Vec<u8>>,
    now: &dyn Fn() -> f64,
) -> Result<ReplayReport, String> {
    let mut report = ReplayReport::default();
    let t0 = now();
    let archives: Vec<TagArchive> = archives
        .into_iter()
        .enumerate()
        .map(|(i, b)| {
            TagArchive::from_bytes(b).map_err(|e| format!("failed to parse archive {i}: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    report.archives_ms = now() - t0;

    let rec = Recording::open(&mut Cursor::new(film)).map_err(|e| e.to_string())?;
    let used = std::cell::RefCell::new(BTreeMap::<([u8; 4], FourCC), usize>::new());
    let tags = |g: [u8; 4], id: FourCC| {
        let d = archives.iter().rev().find_map(|a| {
            a.find_tag_by_id(FourCC::new(g), id)
                .and_then(|e| a.get_tag_data(e).ok())
        });
        if let Some(d) = &d {
            used.borrow_mut().insert((g, id), d.len());
        }
        d
    };
    let data = tags(*b"mesh", rec.header.mesh_tag_id).ok_or("the film's mesh is not there")?;
    let mesh = MeshTag::from_bytes(&data).map_err(|e| e.to_string())?;
    let params = GameParameters::from_film(film).ok_or("no game parameters in the film")?;
    let setup = GameSetup::from_film(film).ok_or("no game setup in the film")?;

    let mut syncs = BTreeMap::<u32, Vec<u32>>::new();
    for t in &rec.tick_commands {
        for c in &t.commands {
            if let CommandPayload::Sync(s) = &c.payload
                && s.snapshot_tick == t.tick - t.tick % SNAPSHOT_EVERY
            {
                syncs
                    .entry(s.snapshot_tick)
                    .or_default()
                    .push(s.random_seed);
            }
        }
    }
    let mut first_sync_miss = None;

    let mut hash = Sha256::new();
    DRAW_LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
    let t0 = now();
    let map = MapSource {
        mesh_data: &data,
        mesh: &mesh,
        tags: &tags,
        params,
    };
    let (mut sim, set_up) = Simulation::replay_film(&rec, setup, &map);
    let load_failed = set_up.load.err().map(|u| u.to_string());
    report.map_load_ms = now() - t0;
    flush(-1, &mut hash, &mut report.draws);
    let load_stop = sim.engine_stop.as_ref().map(ToString::to_string);
    let mut tick_stop = None;
    report.tick_ms.reserve(30000);
    while sim.current_tick < DRAWS_TO && sim.engine_stop.is_none() && !sim.ticks_stopped() {
        let t = sim.current_tick;
        let seed = if t == 0 {
            rec.header.seed
        } else {
            sim.rng.state
        };
        if first_sync_miss.is_none() && syncs.get(&t).is_some_and(|s| s.iter().any(|&x| x != seed))
        {
            first_sync_miss = Some(t);
        }
        let a = now();
        sim.step_with_commands();
        report.tick_ms.push(now() - a);
        flush(i64::from(t), &mut hash, &mut report.draws);
        if let Some(e) = &sim.engine_stop {
            tick_stop = Some((t, e.to_string()));
        }
    }
    DRAW_LOG.with(|l| *l.borrow_mut() = None);

    report.draws_sha256 = hash.short_hex();
    report.first_sync_miss = first_sync_miss.map_or("none".into(), |t| t.to_string());
    report.end = match (load_stop, tick_stop, load_failed) {
        (Some(e), _, _) | (None, None, Some(e)) => format!("stop in load: {e}"),
        (None, Some((t, e)), _) => format!("stop in tick {t}: {e}"),
        _ => match sim.game_end() {
            Some(e) => format!(
                "end_game in tick {}: winner {}, code {}",
                e.tick, e.winner, e.code
            ),
            None => format!("no end_game by tick {}", sim.current_tick),
        },
    };
    report.ticks = sim.current_tick;
    report.state_checksum = sim.state_checksum;
    report.rng_state = sim.rng.state;
    report.units = sim.units.len();
    let mut units: Vec<_> = sim.units.values().collect();
    units.sort_by_key(|u| u.id);
    let mut uh = Sha256::new();
    let mut line = String::new();
    for u in units {
        line.clear();
        writeln!(line, "{u:?}").unwrap();
        uh.update(line.as_bytes());
    }
    report.units_sha256 = uh.short_hex();
    let used = used.borrow();
    report.tags_used = used.len();
    report.tag_bytes_used = used.values().sum();
    Ok(report)
}

fn flush(t: i64, hash: &mut Sha256, count: &mut u64) {
    let draws = DRAW_LOG
        .with(|l| l.borrow_mut().replace(Vec::new()))
        .unwrap_or_default();
    let mut line = String::new();
    for (site, seed) in draws {
        line.clear();
        writeln!(line, "{t} {site:#x} {seed:#x}").unwrap();
        hash.update(line.as_bytes());
        *count += 1;
    }
}

struct Sha256 {
    h: [u32; 8],
    block: Vec<u8>,
    len: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: Vec::with_capacity(64),
            len: 0,
        }
    }

    fn compress(h: &mut [u32; 8], block: &[u8]) {
        let mut w = [0u32; 64];
        for (w, b) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *w = u32::from_be_bytes(*b);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut s = *h;
        for i in 0..64 {
            let s1 = s[4].rotate_right(6) ^ s[4].rotate_right(11) ^ s[4].rotate_right(25);
            let ch = (s[4] & s[5]) ^ (!s[4] & s[6]);
            let t1 = s[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = s[0].rotate_right(2) ^ s[0].rotate_right(13) ^ s[0].rotate_right(22);
            let maj = (s[0] & s[1]) ^ (s[0] & s[2]) ^ (s[1] & s[2]);
            let t2 = s0.wrapping_add(maj);
            s = [
                t1.wrapping_add(t2),
                s[0],
                s[1],
                s[2],
                s[3].wrapping_add(t1),
                s[4],
                s[5],
                s[6],
            ];
        }
        for (h, s) in h.iter_mut().zip(s) {
            *h = h.wrapping_add(s);
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        while !data.is_empty() {
            let n = data.len().min(64 - self.block.len());
            self.block.extend_from_slice(&data[..n]);
            data = &data[n..];
            if self.block.len() == 64 {
                Self::compress(&mut self.h, &self.block);
                self.block.clear();
            }
        }
    }

    fn short_hex(mut self) -> String {
        let bits = self.len * 8;
        self.update(&[0x80]);
        while self.block.len() != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        format!("{:08x}{:08x}", self.h[0], self.h[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_sha256() {
        let mut s = Sha256::new();
        s.update(b"abc");
        assert_eq!(s.short_hex(), "ba7816bf8f01cfea");
    }

    #[test]
    fn heaviest_window_finds_the_slow_stretch() {
        let r = ReplayReport {
            tick_ms: vec![1.0, 1.0, 5.0, 6.0, 1.0],
            ..Default::default()
        };
        assert_eq!(r.heaviest_window(2), (5.5, 2));
        assert_eq!(r.tick_stats().5, 3);
    }

    #[test]
    fn split_plugin_heads_checks_exact_length() {
        let n = mandrake_tag::SBMonoHeader::SIZE;
        let names = vec!["a".to_string(), "b".to_string()];
        let heads = vec![0x5au8; n * 2];
        let split = split_plugin_heads(names.clone(), &heads).unwrap();
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].0, "a");
        assert_eq!(split[1].1.len(), n);

        assert!(split_plugin_heads(names.clone(), &heads[..n]).is_err());
        assert!(split_plugin_heads(names, &heads[..n * 2 - 1]).is_err());
    }

    #[test]
    fn replay_reports_unreadable_archive() {
        let err = replay(&[], vec![b"not a tag archive".to_vec()], &|| 0.0).unwrap_err();
        assert!(err.contains("failed to parse archive 0"), "got: {err}");
    }
}
