#[cfg(test)]
use crate::sprite_interp::CollectionCache;
use crate::sprite_interp::{
    Inbetween, MAGIC, STEPS, VERSION, cache_base_dir, cache_file_name, content_hash, next_forward,
    portable_exe_dir, read_header_hash,
};
use mandrake_common::FourCC;
use mandrake_render::rife46::{
    DEFAULT_WEIGHTS_URL, RIFE46_M2RW_FILE, RIFE46_WEIGHTS_URL, Rife46Gpu, Rife46Output,
    Rife46Weights,
};
use mandrake_tag::TagArchive;
use mandrake_tag::collection::{
    CollectionSequence, CollectionTag, CorePermutations, displayed_views,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

const UPSCALE: usize = 4;
const PAD: usize = 8;
const GROUND: [u8; 3] = [106, 104, 96];
const ALGO_BASE: &str = "rife4.6/u4/v1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PolicyMode {
    G,
    LegsHeld { leg_line: f64 },
}

impl PolicyMode {
    pub(crate) fn algo_name(self) -> String {
        match self {
            PolicyMode::G => format!("g/{ALGO_BASE}"),
            PolicyMode::LegsHeld { leg_line } => format!("legs_held:{leg_line:.3}/{ALGO_BASE}"),
        }
    }
}

pub(crate) fn policy_for_tfl(coll: FourCC) -> PolicyMode {
    if coll == FourCC(*b"bers") {
        PolicyMode::LegsHeld { leg_line: 0.62 }
    } else {
        PolicyMode::G
    }
}

fn is_locomotion_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "walk" | "walking" | "run" | "running" | "move" | "slither" | "shuffle"
    )
}

fn i16_be_at(d: &[u8], o: usize) -> i16 {
    d.get(o..o + 2)
        .map_or(-1, |b| i16::from_be_bytes([b[0], b[1]]))
}

fn tag_data(archive: &TagArchive, group: &[u8; 4], id: FourCC) -> Option<Vec<u8>> {
    let e = archive.find_tag_by_id(FourCC(*group), id)?;
    archive.get_tag_data(e).ok()
}

fn core_of(archive: &TagArchive, coll_id: FourCC) -> Option<CorePermutations> {
    let core = archive
        .find_tags_by_type(FourCC(*b"unit"))
        .into_iter()
        .find_map(|e| {
            let u = archive.get_tag_data(e).ok()?;
            let mid = FourCC(u.get(0..4)?.try_into().ok()?);
            let m = tag_data(archive, b"mons", mid)?;
            (m.get(4..8)? == coll_id.0).then(|| FourCC(u[4..8].try_into().unwrap()))
        })?;
    let data = tag_data(archive, b"core", core)?;
    CorePermutations::from_bytes(&data)
        .ok()
        .filter(|p| !p.is_empty())
}

pub(crate) struct CollectionPlan {
    pub(crate) id: FourCC,
    pub(crate) hash: u64,
    pub(crate) sequences: Vec<(usize, usize)>,
}

/// Discovers every TFL unit collection and its locomotion sequences (`mons` slot 1 when distinct
/// from slot 0, plus sequences named walk/walking/run/running/move/slither/shuffle).
pub(crate) fn discover_tfl_locomotion(archive: &TagArchive) -> Vec<CollectionPlan> {
    let mut slot1: HashMap<FourCC, BTreeSet<usize>> = HashMap::new();
    let mut unit_colls: BTreeSet<FourCC> = BTreeSet::new();

    for e in archive.find_tags_by_type(FourCC(*b"unit")) {
        let Ok(u) = archive.get_tag_data(e) else {
            continue;
        };
        let Some(mid) = u.get(0..4).and_then(|b| b.try_into().ok()).map(FourCC) else {
            continue;
        };
        let Some(m) = tag_data(archive, b"mons", mid) else {
            continue;
        };
        let Some(cid) = m.get(4..8).and_then(|b| b.try_into().ok()).map(FourCC) else {
            continue;
        };
        unit_colls.insert(cid);
        let s0 = i16_be_at(&m, 0x08);
        let s1 = i16_be_at(&m, 0x0a);
        if s1 >= 0 && s1 != s0 {
            slot1.entry(cid).or_default().insert(s1 as usize);
        }
    }

    let mut plans = Vec::new();
    for cid in unit_colls {
        let Some(raw) = tag_data(archive, b".256", cid) else {
            continue;
        };
        let hash = content_hash(&raw);
        let Ok(coll) = CollectionTag::from_bytes(&raw) else {
            continue;
        };
        let mut seqs = Vec::new();
        for s in &coll.sequences {
            if s.frames_per_view < 2 {
                continue;
            }
            let in_slot1 = slot1.get(&cid).is_some_and(|set| set.contains(&s.index));
            if !in_slot1 && !is_locomotion_name(&s.name) {
                continue;
            }
            let n = s.num_views as usize;
            let stored = if s.mirrors_views() {
                displayed_views(n)
            } else {
                n
            };
            seqs.push((s.index, stored));
        }
        if !seqs.is_empty() {
            plans.push(CollectionPlan {
                id: cid,
                hash,
                sequences: seqs,
            });
        }
    }
    plans
}

struct CanvasFrames {
    width: usize,
    height: usize,
    rx: i32,
    ry: i32,
    loop_frame: i16,
    frames: Vec<Vec<u8>>,
    rife_inputs: Vec<Vec<f32>>,
}

fn prepare_view_canvas(
    coll: &CollectionTag,
    seq: &CollectionSequence,
    natural_palette: &[[u8; 4]],
    view: usize,
) -> Option<CanvasFrames> {
    let (sv, flip) = seq.resolve_view(view);
    let n = seq.frames_per_view as usize;
    let mut raw_frames: Vec<(usize, usize, i32, i32, Vec<u8>)> = Vec::with_capacity(n);

    for f in 0..n {
        let fr = seq.bitmap_index(sv, f).and_then(|i| coll.frame(i))?;
        if !fr.wide_pixels.is_empty() {
            return None;
        }
        let w = fr.width as usize;
        let h = fr.height as usize;
        let mut px = fr.pixels[..w * h].to_vec();
        let mut ox = i32::from(fr.offset_x);
        let oy = i32::from(fr.offset_y);
        if flip {
            for row in 0..h {
                px[row * w..(row + 1) * w].reverse();
            }
            ox = w as i32 - ox;
        }
        raw_frames.push((w, h, ox, oy, px));
    }

    let l = raw_frames.iter().map(|f| f.2).max()?;
    let r = raw_frames.iter().map(|f| f.0 as i32 - f.2).max()?;
    let u = raw_frames.iter().map(|f| f.3).max()?;
    let dn = raw_frames.iter().map(|f| f.1 as i32 - f.3).max()?;
    let cw = (l + r + 2 * PAD as i32) as usize;
    let ch = (u + dn + 2 * PAD as i32) as usize;
    let rx = PAD as i32 + l;
    let ry = PAD as i32 + u;

    let mut frames = Vec::with_capacity(n);
    let mut rife_inputs = Vec::with_capacity(n);
    let w4 = cw * UPSCALE;
    let h4 = ch * UPSCALE;

    for (w, h, ox, oy, px) in raw_frames {
        let mut canvas = vec![0u8; cw * ch];
        let x0 = (rx - ox) as usize;
        let y0 = (ry - oy) as usize;
        for y in 0..h {
            let dst_off = (y0 + y) * cw + x0;
            let src_off = y * w;
            canvas[dst_off..dst_off + w].copy_from_slice(&px[src_off..src_off + w]);
        }

        let mut rgb4 = vec![0.0f32; w4 * h4 * 3];
        for y in 0..ch {
            for x in 0..cw {
                let idx = canvas[y * cw + x];
                let c = if idx == 0 {
                    GROUND
                } else {
                    let p = natural_palette
                        .get(usize::from(idx))
                        .copied()
                        .unwrap_or([0, 0, 0, 255]);
                    [p[0], p[1], p[2]]
                };
                let rf = f32::from(c[0]) / 255.0;
                let gf = f32::from(c[1]) / 255.0;
                let bf = f32::from(c[2]) / 255.0;
                for dy in 0..UPSCALE {
                    let row = (y * UPSCALE + dy) * w4;
                    for dx in 0..UPSCALE {
                        let dst = (row + x * UPSCALE + dx) * 3;
                        rgb4[dst] = rf;
                        rgb4[dst + 1] = gf;
                        rgb4[dst + 2] = bf;
                    }
                }
            }
        }
        frames.push(canvas);
        rife_inputs.push(rgb4);
    }

    Some(CanvasFrames {
        width: cw,
        height: ch,
        rx,
        ry,
        loop_frame: seq.loop_frame,
        frames,
        rife_inputs,
    })
}

pub(crate) fn below_legs_cut_dilated(
    i0: &[u8],
    i1: &[u8],
    cw: usize,
    ch: usize,
    leg_line: f64,
) -> Option<usize> {
    let mut y_min = None;
    let mut y_max = 0usize;
    for y in 0..ch {
        let row0 = &i0[y * cw..(y + 1) * cw];
        let row1 = &i1[y * cw..(y + 1) * cw];
        if row0.iter().any(|&p| p > 0) || row1.iter().any(|&p| p > 0) {
            if y_min.is_none() {
                y_min = Some(y);
            }
            y_max = y;
        }
    }
    let y0 = y_min?;
    let cut = ((y0 as f64 + leg_line * ((y_max + 1 - y0) as f64)) * UPSCALE as f64) as usize;
    let h4 = ch * UPSCALE;
    if cut >= h4 {
        return None;
    }
    // 5x5 dilation of the half-plane y >= cut expands it upward by 2 magnified rows.
    Some(cut.saturating_sub(UPSCALE / 2))
}

pub(crate) fn warp_and_pool(
    i0: &[u8],
    i1: &[u8],
    cw: usize,
    ch: usize,
    rife: &Rife46Output,
    mode: PolicyMode,
) -> Vec<u8> {
    let w4 = cw * UPSCALE;
    let h4 = ch * UPSCALE;
    let hold_from_y = match mode {
        PolicyMode::G => None,
        PolicyMode::LegsHeld { leg_line } => below_legs_cut_dilated(i0, i1, cw, ch, leg_line),
    };

    let mut mid = vec![0u8; w4 * h4];
    let max_x = (w4 - 1) as f32;
    let max_y = (h4 - 1) as f32;

    for y in 0..h4 {
        let hold_row = hold_from_y.is_some_and(|cut| y >= cut);
        let row_off = y * w4;
        if hold_row {
            let sy = y / UPSCALE;
            for x in 0..w4 {
                mid[row_off + x] = i0[sy * cw + x / UPSCALE];
            }
            continue;
        }
        for x in 0..w4 {
            let p = row_off + x;
            let fl = rife.flow[p];
            let pick0 = rife.mask[p] >= 0.5;
            let (fx, fy, src) = if pick0 {
                (fl[0], fl[1], i0)
            } else {
                (fl[2], fl[3], i1)
            };
            let sx = (x as f32 + fx).round_ties_even().clamp(0.0, max_x) as usize;
            let sy = (y as f32 + fy).round_ties_even().clamp(0.0, max_y) as usize;
            mid[p] = src[(sy / UPSCALE) * cw + (sx / UPSCALE)];
        }
    }

    mode_pool(&mid, cw, ch)
}

pub(crate) fn mode_pool(mid: &[u8], cw: usize, ch: usize) -> Vec<u8> {
    let w4 = cw * UPSCALE;
    let mut out = vec![0u8; cw * ch];
    for y in 0..ch {
        for x in 0..cw {
            let mut block = [0u8; UPSCALE * UPSCALE];
            let mut opaque = 0usize;
            let mut k = 0usize;
            for dy in 0..UPSCALE {
                let row = (y * UPSCALE + dy) * w4 + x * UPSCALE;
                for dx in 0..UPSCALE {
                    let v = mid[row + dx];
                    block[k] = v;
                    if v > 0 {
                        opaque += 1;
                    }
                    k += 1;
                }
            }
            if opaque * 2 < UPSCALE * UPSCALE {
                out[y * cw + x] = 0;
                continue;
            }
            let mut best_idx = 0u8;
            let mut best_count = 0usize;
            for &v in &block {
                if v == 0 {
                    continue;
                }
                let count = block.iter().filter(|&&b| b == v).count();
                if count > best_count {
                    best_count = count;
                    best_idx = v;
                }
            }
            out[y * cw + x] = best_idx;
        }
    }
    out
}

pub(crate) fn crop_inbetween(
    pixels: &[u8],
    cw: usize,
    ch: usize,
    rx: i32,
    ry: i32,
) -> Option<Inbetween> {
    let mut x0 = cw;
    let mut x1 = 0usize;
    let mut y0 = ch;
    let mut y1 = 0usize;
    for y in 0..ch {
        for x in 0..cw {
            if pixels[y * cw + x] > 0 {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let w = x1 - x0;
    let h = y1 - y0;
    let mut cropped = Vec::with_capacity(w * h);
    for y in y0..y1 {
        cropped.extend_from_slice(&pixels[y * cw + x0..y * cw + x1]);
    }
    Some(Inbetween {
        width: w as u16,
        height: h as u16,
        offset_x: (rx - x0 as i32) as i16,
        offset_y: (ry - y0 as i32) as i16,
        pixels: cropped,
    })
}

pub(crate) type M2siKey = (u16, u16, u16, u16, u8, String);

pub(crate) fn serialize_m2si(hash: u64, entries: &BTreeMap<M2siKey, Inbetween>) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&VERSION.to_le_bytes());
    b.extend_from_slice(&STEPS.to_le_bytes());
    b.extend_from_slice(&hash.to_le_bytes());
    b.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for ((seq, view, i, j, step, algo), e) in entries {
        for v in [seq, view, i, j] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.push(*step);
        b.push(algo.len() as u8);
        b.extend_from_slice(algo.as_bytes());
        b.extend_from_slice(&e.width.to_le_bytes());
        b.extend_from_slice(&e.height.to_le_bytes());
        b.extend_from_slice(&e.offset_x.to_le_bytes());
        b.extend_from_slice(&e.offset_y.to_le_bytes());
        b.extend_from_slice(&e.pixels);
    }
    b
}

pub(crate) fn write_m2si_atomic(
    path: &Path,
    hash: u64,
    entries: &BTreeMap<M2siKey, Inbetween>,
) -> std::io::Result<usize> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serialize_m2si(hash, entries);
    let tmp = path.with_extension("m2si.part");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(bytes.len())
}

fn find_local_weights_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for var in ["SPRITE_INTERP_WEIGHTS", "RIFE46_WEIGHTS"] {
        if let Some(env_p) = std::env::var_os(var).filter(|v| !v.is_empty()) {
            paths.push(PathBuf::from(env_p));
        }
    }
    if let Some(exe_dir) = portable_exe_dir() {
        paths.push(exe_dir.join(RIFE46_M2RW_FILE));
        paths.push(
            exe_dir
                .join("cache")
                .join("sprite-interp")
                .join(RIFE46_M2RW_FILE),
        );
    }
    if let Some(base) = cache_base_dir() {
        paths.push(base.join(RIFE46_M2RW_FILE));
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        let rife_dir = PathBuf::from(home).join(".cache").join("myth2-rife");
        paths.push(rife_dir.join(RIFE46_M2RW_FILE));
        paths.push(rife_dir.join("flownet_v4.6.pkl"));
    }
    paths
}

fn save_m2rw_cache(weights: &Rife46Weights, cache_dir: &Path) {
    let base = cache_dir
        .parent()
        .map(Path::to_path_buf)
        .or_else(cache_base_dir);
    let Some(base) = base else {
        return;
    };
    let dst = base.join(RIFE46_M2RW_FILE);
    if dst.is_file() {
        return;
    }
    let _ = std::fs::create_dir_all(&base);
    let tmp = dst.with_extension("m2rw.part");
    let bytes = weights.to_m2rw();
    if std::fs::write(&tmp, &bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &dst);
    }
}

pub(crate) fn try_load_local_weights(
    cache_dir: Option<&Path>,
) -> Option<Result<Rife46Weights, String>> {
    for path in find_local_weights_paths() {
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let is_pkl = !bytes.starts_with(b"M2RW");
        match Rife46Weights::from_bytes(&bytes) {
            Ok(w) => {
                tracing::info!(
                    "sprite interpolation: loaded RIFE v4.6 weights from {}",
                    path.display()
                );
                if is_pkl && let Some(dir) = cache_dir {
                    save_m2rw_cache(&w, dir);
                }
                return Some(Ok(w));
            }
            Err(e) => {
                tracing::warn!(
                    "sprite interpolation: ignoring invalid weights at {}: {e}",
                    path.display()
                );
            }
        }
    }
    None
}

fn download_url() -> String {
    if let Ok(url) = std::env::var("SPRITE_INTERP_WEIGHTS_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    DEFAULT_WEIGHTS_URL
        .unwrap_or(RIFE46_WEIGHTS_URL)
        .to_string()
}

struct DownloadProgress {
    downloaded: AtomicU64,
    total: AtomicU64,
}

fn download_weights_from(
    url: &str,
    cache_dir: &Path,
    progress: &DownloadProgress,
) -> Result<Rife46Weights, String> {
    tracing::info!("sprite interpolation: downloading RIFE v4.6 weights from {url}");
    let bytes = if url.starts_with("https://") {
        download_https_via_curl(url, cache_dir, progress)?
    } else {
        let mut resp = ureq::get(url)
            .call()
            .map_err(|e| format!("failed to download RIFE v4.6 weights: {e}"))?;
        if let Some(len) = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
        {
            progress.total.store(len, Ordering::Relaxed);
        }
        let mut reader = resp.body_mut().as_reader().take(32 * 1024 * 1024);
        let mut bytes = Vec::with_capacity(11 * 1024 * 1024);
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("failed to read RIFE v4.6 weights response: {e}"))?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            progress
                .downloaded
                .store(bytes.len() as u64, Ordering::Relaxed);
        }
        bytes
    };
    let weights = Rife46Weights::from_bytes(&bytes)?;
    save_m2rw_cache(&weights, cache_dir);
    Ok(weights)
}

/// Fetches an HTTPS URL using the system `curl` binary (preinstalled on Windows 10/11, macOS, and
/// Linux) so TLS works on all targets without extra TLS crate dependencies; SHA-256 is verified
/// by [`Rife46Weights::from_bytes`].
fn download_https_via_curl(
    url: &str,
    cache_dir: &Path,
    progress: &DownloadProgress,
) -> Result<Vec<u8>, String> {
    let _ = std::fs::create_dir_all(cache_dir);
    let tmp = cache_dir.join(format!(".rife46-download-{}.tmp", std::process::id()));
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    let mut child = std::process::Command::new(curl)
        .args([
            "-fsSL",
            "--max-redirs",
            "5",
            "--connect-timeout",
            "15",
            "--max-time",
            "300",
            "-o",
        ])
        .arg(&tmp)
        .arg(url)
        .spawn()
        .map_err(|e| format!("failed to launch {curl} for RIFE v4.6 weights: {e}"))?;

    // Approximate expected size (~10.08 MiB for rife46.m2rw) so the progress bar moves smoothly.
    progress.total.store(10_569_768, Ordering::Relaxed);
    loop {
        if let Ok(meta) = std::fs::metadata(&tmp) {
            progress.downloaded.store(meta.len(), Ordering::Relaxed);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("{curl} failed downloading {url} ({status})"));
                }
                break;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("failed waiting on {curl}: {e}"));
            }
        }
    }
    let bytes = std::fs::read(&tmp)
        .map_err(|e| format!("failed reading downloaded weights {}: {e}", tmp.display()))?;
    let _ = std::fs::remove_file(&tmp);
    progress
        .downloaded
        .store(bytes.len() as u64, Ordering::Relaxed);
    Ok(bytes)
}

enum WeightsPhase {
    Downloading(
        Receiver<Result<Rife46Weights, String>>,
        Arc<DownloadProgress>,
    ),
    Ready(Rife46Weights),
}

struct ActiveCollection {
    plan_idx: usize,
    coll: CollectionTag,
    nat: Vec<[u8; 4]>,
    mode: PolicyMode,
    algo: String,
    seq_pos: usize,
    view_pos: usize,
    active_view: Option<CanvasFrames>,
    frame_pos: usize,
    entries: BTreeMap<M2siKey, Inbetween>,
}

pub(crate) enum StepResult {
    Working,
    Done { generated: usize, elapsed: Duration },
    Failed(String),
}

/// Incremental in-client TFL sprite interpolation cache generator, stepped a small budget per
/// frame so the TFL loading/progress screen stays responsive.
pub(crate) struct TflSpriteCacheGen {
    archive: Arc<TagArchive>,
    cache_dir: PathBuf,
    missing: Vec<CollectionPlan>,
    weights_phase: Option<WeightsPhase>,
    gpu: Option<Rife46Gpu>,
    active: Option<ActiveCollection>,
    next_plan: usize,
    total_pairs: usize,
    done_pairs: usize,
    started: Instant,
    status_text: String,
}

impl TflSpriteCacheGen {
    /// Checks all TFL unit locomotion collections against `cache_dir`. Returns `None` immediately
    /// when every collection already has a valid `.m2si` cache file.
    pub(crate) fn start(archive: Arc<TagArchive>, cache_dir: PathBuf) -> Option<Self> {
        let plans = discover_tfl_locomotion(&archive);
        let mut missing = Vec::new();
        let mut total_pairs = 0usize;

        for plan in plans {
            let path = cache_dir.join(cache_file_name(plan.id));
            if read_header_hash(&path) == Some(plan.hash) {
                continue;
            }
            if let Some(raw) = tag_data(&archive, b".256", plan.id)
                && let Ok(coll) = CollectionTag::from_bytes(&raw)
            {
                for &(seq_idx, stored_views) in &plan.sequences {
                    if let Some(seq) = coll.sequences.get(seq_idx) {
                        total_pairs += stored_views * seq.frames_per_view as usize;
                    }
                }
            }
            missing.push(plan);
        }

        if missing.is_empty() {
            return None;
        }

        let (weights_phase, status_text) = match try_load_local_weights(Some(&cache_dir)) {
            Some(Ok(w)) => (
                Some(WeightsPhase::Ready(w)),
                format!("Building sprite cache (0/{})...", missing.len()),
            ),
            _ => {
                let (tx, rx) = std::sync::mpsc::channel();
                let progress = Arc::new(DownloadProgress {
                    downloaded: AtomicU64::new(0),
                    total: AtomicU64::new(0),
                });
                let prog_bg = Arc::clone(&progress);
                let url = download_url();
                let dir_bg = cache_dir.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(download_weights_from(&url, &dir_bg, &prog_bg));
                });
                (
                    Some(WeightsPhase::Downloading(rx, progress)),
                    "Downloading RIFE v4.6 weights...".to_string(),
                )
            }
        };

        Some(Self {
            archive,
            cache_dir,
            missing,
            weights_phase,
            gpu: None,
            active: None,
            next_plan: 0,
            total_pairs: total_pairs.max(1),
            done_pairs: 0,
            started: Instant::now(),
            status_text,
        })
    }

    pub(crate) fn status(&self) -> (String, i16) {
        if let Some(WeightsPhase::Downloading(_, prog)) = &self.weights_phase {
            let dl = prog.downloaded.load(Ordering::Relaxed);
            let tot = prog.total.load(Ordering::Relaxed);
            let frac = (dl * 256).checked_div(tot).map_or(0, |v| v.min(256) as i16);
            let text = if tot > 0 {
                format!(
                    "Downloading RIFE v4.6 weights ({:.1} / {:.1} MB)...",
                    dl as f64 / (1024.0 * 1024.0),
                    tot as f64 / (1024.0 * 1024.0),
                )
            } else {
                self.status_text.clone()
            };
            return (text, frac);
        }
        let frac =
            ((self.done_pairs as u64 * 256) / self.total_pairs.max(1) as u64).min(256) as i16;
        (self.status_text.clone(), frac)
    }

    pub(crate) fn step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: Duration,
    ) -> StepResult {
        if self.gpu.is_none() {
            match self.weights_phase.take() {
                Some(WeightsPhase::Downloading(rx, prog)) => match rx.try_recv() {
                    Ok(Ok(w)) => {
                        self.gpu = Some(Rife46Gpu::new(device, &w));
                    }
                    Ok(Err(e)) => return StepResult::Failed(e),
                    Err(TryRecvError::Empty) => {
                        self.weights_phase = Some(WeightsPhase::Downloading(rx, prog));
                        return StepResult::Working;
                    }
                    Err(TryRecvError::Disconnected) => {
                        return StepResult::Failed("weights download thread aborted".into());
                    }
                },
                Some(WeightsPhase::Ready(w)) => {
                    self.gpu = Some(Rife46Gpu::new(device, &w));
                }
                None => return StepResult::Failed("missing RIFE weights".into()),
            }
        }

        let slice_start = Instant::now();
        let gpu = self.gpu.as_mut().unwrap();

        loop {
            if self.active.is_none() {
                if self.next_plan >= self.missing.len() {
                    let elapsed = self.started.elapsed();
                    tracing::info!(
                        "sprite interpolation: generated {} TFL collections in {:.2}s -> {}",
                        self.missing.len(),
                        elapsed.as_secs_f64(),
                        self.cache_dir.display()
                    );
                    return StepResult::Done {
                        generated: self.missing.len(),
                        elapsed,
                    };
                }
                let idx = self.next_plan;
                self.next_plan += 1;
                let plan = &self.missing[idx];
                self.status_text = format!(
                    "Building sprite cache ({}/{}: {})...",
                    idx + 1,
                    self.missing.len(),
                    plan.id.to_string().trim()
                );
                let Some(raw) = tag_data(&self.archive, b".256", plan.id) else {
                    continue;
                };
                let Ok(coll) = CollectionTag::from_bytes(&raw) else {
                    continue;
                };
                let core = core_of(&self.archive, plan.id);
                let nat = match &core {
                    Some(p) => coll.permutation_palette(p, 0),
                    None => coll.palette.clone(),
                };
                let mode = policy_for_tfl(plan.id);
                let algo = mode.algo_name();
                self.active = Some(ActiveCollection {
                    plan_idx: idx,
                    coll,
                    nat,
                    mode,
                    algo,
                    seq_pos: 0,
                    view_pos: 0,
                    active_view: None,
                    frame_pos: 0,
                    entries: BTreeMap::new(),
                });
            }

            let act = self.active.as_mut().unwrap();
            let plan = &self.missing[act.plan_idx];

            if act.seq_pos >= plan.sequences.len() {
                if !act.entries.is_empty() {
                    let path = self.cache_dir.join(cache_file_name(plan.id));
                    if let Err(e) = write_m2si_atomic(&path, plan.hash, &act.entries) {
                        tracing::warn!(
                            "sprite interpolation: failed writing {}: {e}",
                            path.display()
                        );
                    }
                }
                self.active = None;
                if slice_start.elapsed() >= budget {
                    return StepResult::Working;
                }
                continue;
            }

            let (seq_idx, stored_views) = plan.sequences[act.seq_pos];
            if act.view_pos >= stored_views {
                act.seq_pos += 1;
                act.view_pos = 0;
                act.active_view = None;
                act.frame_pos = 0;
                continue;
            }

            if act.active_view.is_none() {
                let seq = &act.coll.sequences[seq_idx];
                match prepare_view_canvas(&act.coll, seq, &act.nat, act.view_pos) {
                    Some(cv) => {
                        act.active_view = Some(cv);
                        act.frame_pos = 0;
                    }
                    None => {
                        act.view_pos += 1;
                        continue;
                    }
                }
            }

            let cv = act.active_view.as_ref().unwrap();
            let n = cv.frames.len();
            if act.frame_pos >= n {
                act.active_view = None;
                act.view_pos += 1;
                continue;
            }

            let i = act.frame_pos;
            act.frame_pos += 1;
            self.done_pairs += 1;
            let j = next_forward(i, n, cv.loop_frame);
            if j != i {
                let w4 = (cv.width * UPSCALE) as u32;
                let h4 = (cv.height * UPSCALE) as u32;
                match gpu.infer(
                    device,
                    queue,
                    &cv.rife_inputs[i],
                    &cv.rife_inputs[j],
                    w4,
                    h4,
                ) {
                    Ok(out) => {
                        let pooled = warp_and_pool(
                            &cv.frames[i],
                            &cv.frames[j],
                            cv.width,
                            cv.height,
                            &out,
                            act.mode,
                        );
                        if let Some(ib) = crop_inbetween(&pooled, cv.width, cv.height, cv.rx, cv.ry)
                        {
                            act.entries.insert(
                                (
                                    seq_idx as u16,
                                    act.view_pos as u16,
                                    i as u16,
                                    j as u16,
                                    1,
                                    act.algo.clone(),
                                ),
                                ib,
                            );
                        }
                    }
                    Err(e) => return StepResult::Failed(e),
                }
            }

            if slice_start.elapsed() >= budget {
                return StepResult::Working;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_uses_legs_held_for_berserk_and_g_otherwise() {
        assert_eq!(
            policy_for_tfl(FourCC(*b"bers")),
            PolicyMode::LegsHeld { leg_line: 0.62 }
        );
        assert_eq!(
            policy_for_tfl(FourCC(*b"bers")).algo_name(),
            "legs_held:0.620/rife4.6/u4/v1"
        );
        assert_eq!(policy_for_tfl(FourCC(*b"warr")), PolicyMode::G);
        assert_eq!(
            policy_for_tfl(FourCC(*b"warr")).algo_name(),
            "g/rife4.6/u4/v1"
        );
    }

    #[test]
    fn mode_pool_and_crop_and_m2si_roundtrip() {
        // 2x1 sprite -> 8x4 magnified. Left pixel has 10 opaque (7x idx 5, 3x idx 9) -> 5.
        // Right pixel has 7 opaque (< 8) -> 0.
        let mut mid = vec![0u8; 8 * 4];
        for dy in 0..4 {
            for dx in 0..4 {
                let k = dy * 4 + dx;
                mid[dy * 8 + dx] = if k < 7 {
                    5
                } else if k < 10 {
                    9
                } else {
                    0
                };
                mid[dy * 8 + 4 + dx] = if k < 7 { 3 } else { 0 };
            }
        }
        let pooled = mode_pool(&mid, 2, 1);
        assert_eq!(pooled, vec![5, 0]);
        let ib = crop_inbetween(&pooled, 2, 1, 1, 0).expect("non-empty");
        assert_eq!(ib.width, 1);
        assert_eq!(ib.height, 1);
        assert_eq!(ib.offset_x, 1);
        assert_eq!(ib.offset_y, 0);
        assert_eq!(ib.pixels, vec![5]);

        let mut entries = BTreeMap::new();
        entries.insert((1, 0, 0, 1, 1, "g/rife4.6/u4/v1".into()), ib.clone());
        let bytes = serialize_m2si(0x1234_5678, &entries);
        let parsed = CollectionCache::parse(&bytes, 0x1234_5678).expect("valid m2si");
        assert_eq!(parsed.get(1, 0, 0, 1), Some(&ib));
    }

    #[test]
    fn weights_download_from_local_http_verifies_sha256_and_caches_m2rw() {
        let Some(Ok(weights)) = try_load_local_weights(None) else {
            return;
        };
        let m2rw = weights.to_m2rw();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind local http");
        let addr = listener.local_addr().unwrap();
        let body = m2rw.clone();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req = [0u8; 1024];
                let _ = stream.read(&mut req);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
        });

        let tmp_base = std::env::temp_dir().join(format!("m2-rife-dl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_base);
        let cache_dir = tmp_base.join("tfl");
        let progress = DownloadProgress {
            downloaded: AtomicU64::new(0),
            total: AtomicU64::new(0),
        };
        let url = format!("http://{addr}/rife46.m2rw");
        let loaded = download_weights_from(&url, &cache_dir, &progress).expect("download m2rw");
        let _ = server.join();
        assert_eq!(
            progress.downloaded.load(Ordering::Relaxed),
            m2rw.len() as u64
        );
        assert_eq!(progress.total.load(Ordering::Relaxed), m2rw.len() as u64);
        assert_eq!(loaded.to_m2rw().len(), m2rw.len());
        let cached_path = tmp_base.join(RIFE46_M2RW_FILE);
        assert!(
            cached_path.is_file(),
            "should cache rife46.m2rw beside game cache"
        );
        let _ = std::fs::remove_dir_all(&tmp_base);
    }
}
