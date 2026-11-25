use mandrake_common::FourCC;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) const MAGIC: &[u8; 4] = b"M2SI";
pub(crate) const VERSION: u16 = 1;
pub(crate) const STEPS: u16 = 2;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn install(enabled: bool) {
    INSTALLED.store(true, Ordering::Relaxed);
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub(crate) fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

pub(crate) fn enabled() -> bool {
    INSTALLED.load(Ordering::Relaxed) && ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn toggle() -> bool {
    !ENABLED.fetch_xor(true, Ordering::Relaxed)
}

pub(crate) fn start_state(setting: Option<bool>, default: bool) -> bool {
    match std::env::var("M2_SPRITE_INTERP").ok().as_deref() {
        Some("0" | "off" | "false") => false,
        Some("1" | "on" | "true") => true,
        _ => setting.unwrap_or(default),
    }
}

pub(crate) fn content_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub(crate) fn portable_dir_from_exe(exe: &std::path::Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    if dir.file_name().and_then(|n| n.to_str()) == Some("MacOS")
        && let Some(contents) = dir.parent()
        && contents.file_name().and_then(|n| n.to_str()) == Some("Contents")
        && let Some(bundle) = contents.parent()
        && bundle
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        && let Some(beside_app) = bundle.parent()
    {
        return Some(beside_app.to_path_buf());
    }
    Some(dir.to_path_buf())
}

pub(crate) fn portable_exe_dir() -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    portable_dir_from_exe(&exe)
}

fn dir_is_writable(dir: &std::path::Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let check_file = dir.join(format!(".m2si-write-check-{}", std::process::id()));
    if std::fs::write(&check_file, b"").is_ok() {
        let _ = std::fs::remove_file(&check_file);
        true
    } else {
        false
    }
}

pub(crate) fn cache_base_from(
    env_override: Option<std::ffi::OsString>,
    portable_dir: Option<PathBuf>,
    os_cache_base: Option<PathBuf>,
    is_writable: impl Fn(&std::path::Path) -> bool,
) -> Option<PathBuf> {
    if let Some(base) = env_override.filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(base));
    }
    if let Some(dir) = portable_dir {
        let candidate = dir.join("cache").join("sprite-interp");
        if is_writable(&candidate) {
            return Some(candidate);
        }
    }
    Some(
        os_cache_base?
            .join("myth2-sourceport")
            .join("sprite-interp"),
    )
}

pub(crate) fn cache_base_dir() -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    cache_base_from(
        std::env::var_os("SPRITE_INTERP_CACHE"),
        portable_exe_dir(),
        crate::user_dirs::cache_base(),
        dir_is_writable,
    )
}

pub(crate) fn cache_dir(game: &str) -> Option<PathBuf> {
    Some(cache_base_dir()?.join(game))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_header_hash(path: &std::path::Path) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut hdr = [0u8; 20];
    std::io::Read::read_exact(&mut f, &mut hdr).ok()?;
    if &hdr[0..4] != MAGIC {
        return None;
    }
    let ver = u16::from_le_bytes([hdr[4], hdr[5]]);
    let steps = u16::from_le_bytes([hdr[6], hdr[7]]);
    if ver != VERSION || steps != STEPS {
        return None;
    }
    Some(u64::from_le_bytes(hdr[8..16].try_into().ok()?))
}

pub(crate) fn cache_file_name(id: FourCC) -> String {
    let b = id.0;
    format!("{:02x}{:02x}{:02x}{:02x}.m2si", b[0], b[1], b[2], b[3])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Inbetween {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) offset_x: i16,
    pub(crate) offset_y: i16,
    pub(crate) pixels: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CollectionCache {
    pub(crate) entries: HashMap<(u16, u16, u16, u16), Inbetween>,
}

impl CollectionCache {
    pub(crate) fn parse(d: &[u8], hash: u64) -> Option<Self> {
        let mut r = Reader { d, o: 0 };
        if r.take(4)? != MAGIC || r.u16()? != VERSION || r.u16()? != STEPS || r.u64()? != hash {
            return None;
        }
        let n = r.u32()?;
        let mut entries = HashMap::new();
        for _ in 0..n {
            let (seq, view, i, j) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
            let step = r.take(1)?[0];
            let alen = usize::from(r.take(1)?[0]);
            r.take(alen)?;
            let (width, height) = (r.u16()?, r.u16()?);
            let (offset_x, offset_y) = (r.u16()? as i16, r.u16()? as i16);
            let pixels = r.take(usize::from(width) * usize::from(height))?.to_vec();
            if step == 1 {
                entries.insert(
                    (seq, view, i, j),
                    Inbetween {
                        width,
                        height,
                        offset_x,
                        offset_y,
                        pixels,
                    },
                );
            }
        }
        Some(Self { entries })
    }

    pub(crate) fn load(game: &str, id: FourCC, data: &[u8]) -> Option<Self> {
        let path = cache_dir(game)?.join(cache_file_name(id));
        let bytes = std::fs::read(&path).ok()?;
        let c = Self::parse(&bytes, content_hash(data));
        match &c {
            Some(c) => tracing::info!(
                "sprite interpolation: {id}: {} in-betweens from {}",
                c.entries.len(),
                path.display()
            ),
            None => tracing::warn!(
                "sprite interpolation: {} is stale or not a cache; {id} keeps its drawn frames",
                path.display()
            ),
        }
        c
    }

    pub(crate) fn has_sequence(&self, seq: usize) -> bool {
        self.entries.keys().any(|k| usize::from(k.0) == seq)
    }

    pub(crate) fn get(&self, seq: usize, view: usize, i: usize, j: usize) -> Option<&Inbetween> {
        let k = |v: usize| u16::try_from(v).ok();
        self.entries.get(&(k(seq)?, k(view)?, k(i)?, k(j)?))
    }
}

struct Reader<'a> {
    d: &'a [u8],
    o: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.o..self.o.checked_add(n)?)?;
        self.o += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

pub(crate) fn next_forward(frame: usize, frames: usize, loop_frame: i16) -> usize {
    if frame + 1 < frames {
        frame + 1
    } else {
        usize::try_from(loop_frame)
            .ok()
            .filter(|&l| l < frames)
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AnimPhase {
    pub(crate) frame: usize,
    pub(crate) elapsed: i32,
    pub(crate) ticks_per_frame: i32,
    pub(crate) reverse: bool,
}

impl AnimPhase {
    pub(crate) fn of_sim(frame: usize, tick: i32, ticks_per_frame: i32, reverse: bool) -> Self {
        let tpf = ticks_per_frame.max(1);
        Self {
            frame,
            elapsed: tick.rem_euclid(tpf),
            ticks_per_frame: tpf,
            reverse,
        }
    }

    pub(crate) fn doubled_index(
        &self,
        frames: usize,
        loop_frame: i16,
        fraction: i32,
        one: i32,
        history: usize,
    ) -> usize {
        let n = frames.max(1);
        let f = self.frame % n;
        if n < 2 {
            return 2 * f;
        }
        let tpf = i64::from(self.ticks_per_frame.max(1));
        let one = i64::from(one.max(1));
        let at = i64::from(self.elapsed) * one + i64::from(fraction).clamp(0, one) - one / 2;
        let between = |a: usize, b: usize, fallback: usize| {
            let (i, j) = if self.reverse { (b, a) } else { (a, b) };
            if next_forward(i, n, loop_frame) == j {
                2 * i + 1
            } else {
                2 * fallback
            }
        };
        if at < 0 {
            let h = history % n;
            return if h == f { 2 * f } else { between(h, f, h) };
        }
        if 2 * at < tpf * one {
            return 2 * f;
        }
        let next = if self.reverse {
            if f == 0 { n - 1 } else { f - 1 }
        } else {
            next_forward(f, n, loop_frame)
        };
        between(f, next, f)
    }
}

pub(crate) fn doubled_frames<F, V>(
    drawn: &[F],
    loop_frame: i16,
    views: impl Fn(&F) -> &[Option<V>],
    with_views: impl Fn(&F, Vec<Option<V>>) -> F,
    mut mid: impl FnMut(usize, usize, usize) -> Option<V>,
) -> Vec<F>
where
    F: Clone,
    V: Clone,
{
    let n = drawn.len();
    let mut out = Vec::with_capacity(2 * n);
    for (f, frame) in drawn.iter().enumerate() {
        out.push(frame.clone());
        let j = next_forward(f, n, loop_frame);
        let vs = views(frame)
            .iter()
            .enumerate()
            .map(|(v, drawn_view)| match drawn_view {
                Some(d) if j != f => mid(v, f, j).or_else(|| Some(d.clone())),
                other => other.clone(),
            })
            .collect();
        out.push(with_views(frame, vs));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: i32 = 0x10000;

    fn phase(frame: usize, elapsed: i32, tpf: i32, reverse: bool) -> AnimPhase {
        AnimPhase {
            frame,
            elapsed,
            ticks_per_frame: tpf,
            reverse,
        }
    }

    #[test]
    fn the_sim_tick_counts_from_the_frame_start() {
        assert_eq!(AnimPhase::of_sim(2, 4, 4, false).elapsed, 0);
        assert_eq!(AnimPhase::of_sim(2, 1, 4, false).elapsed, 1);
        assert_eq!(AnimPhase::of_sim(2, 3, 4, false).elapsed, 3);
        assert_eq!(AnimPhase::of_sim(2, 0, 0, false).ticks_per_frame, 1);
    }

    fn played(
        n: usize,
        loop_frame: i16,
        tpf: i32,
        reverse: bool,
        q: i32,
        ticks: usize,
    ) -> Vec<usize> {
        let (mut frame, mut tick) = (if reverse { n - 1 } else { 0 }, 0i32);
        let mut out = Vec::new();
        for _ in 0..ticks {
            let history = frame;
            if tick >= tpf {
                tick = 0;
            }
            tick += 1;
            if tick >= tpf {
                frame = if reverse {
                    if frame == 0 { n - 1 } else { frame - 1 }
                } else {
                    next_forward(frame, n, loop_frame)
                };
            }
            let a = AnimPhase::of_sim(frame, tick, tpf, reverse);
            for k in 0..q {
                out.push(a.doubled_index(n, loop_frame, k * ONE / q, ONE, history));
            }
        }
        out
    }

    fn runs(v: &[usize]) -> Vec<(usize, usize)> {
        let mut r: Vec<(usize, usize)> = Vec::new();
        for &x in v {
            match r.last_mut() {
                Some((y, c)) if *y == x => *c += 1,
                _ => r.push((x, 1)),
            }
        }
        r
    }

    #[test]
    fn the_pictures_follow_the_frames_time_evenly() {
        for tpf in [1, 2, 3] {
            let shown = played(4, 0, tpf, false, 4, 40);
            let r = runs(&shown);
            for (k, w) in r[1..r.len() - 1].windows(2).enumerate() {
                assert_eq!(w[0].1, 2 * tpf as usize, "tpf {tpf} run {k}: {r:?}");
                assert_eq!(w[1].0, (w[0].0 + 1) % 8, "tpf {tpf} run {k}: {r:?}");
            }
            let off: Vec<usize> = (0..40)
                .flat_map(|t| {
                    let tick_frame = |t: i32| ((t / tpf) as usize) % 4;
                    (0..4).map(move |k| 2 * tick_frame(if k < 2 { t } else { t + 1 }))
                })
                .collect();
            for (i, (&on, &o)) in shown.iter().zip(&off).enumerate() {
                if on % 2 == 0 {
                    assert_eq!(on, o, "tpf {tpf} frame {i}");
                }
            }
        }
    }

    #[test]
    fn loops_and_reverse_play_take_their_pairs() {
        let shown = played(4, 1, 1, false, 2, 12);
        assert!(shown.contains(&7));
        assert!(!shown[2..].contains(&0) && !shown[2..].contains(&1));
        let back = played(4, 0, 1, true, 2, 12);
        let r: Vec<usize> = runs(&back).into_iter().map(|(x, _)| x).collect();
        assert_eq!(&r[..7], &[5, 4, 3, 2, 1, 0, 7]);
        let back = played(4, 1, 1, true, 2, 12);
        assert!(!back.contains(&7));
        let r: Vec<usize> = runs(&back).into_iter().map(|(x, _)| x).collect();
        assert_eq!(&r[..7], &[5, 4, 3, 2, 1, 0, 6]);
        let a = AnimPhase::of_sim(0, 1, 1, false);
        assert_eq!(a.doubled_index(1, 0, 0, ONE, 0), 0);
        assert_eq!(a.doubled_index(4, 0, 0, ONE, 2), 4);
        assert_eq!(a.doubled_index(4, 0, ONE / 2, ONE, 2), 0);
        assert_eq!(phase(1, 0, 2, false).doubled_index(4, 0, ONE, ONE, 1), 2);
        assert_eq!(phase(1, 1, 2, false).doubled_index(4, 0, ONE, ONE, 1), 3);
    }

    #[test]
    fn next_frame_follows_the_loop_frame() {
        assert_eq!(next_forward(0, 4, 0), 1);
        assert_eq!(next_forward(3, 4, 0), 0);
        assert_eq!(next_forward(3, 4, 2), 2);
        assert_eq!(next_forward(3, 4, -1), 0);
        assert_eq!(next_forward(3, 4, 9), 0);
    }

    type Entry<'a> = ((u16, u16, u16, u16), u8, &'a str, Inbetween);

    fn file(hash: u64, entries: &[Entry<'_>]) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&STEPS.to_le_bytes());
        b.extend_from_slice(&hash.to_le_bytes());
        b.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for ((s, v, i, j), step, algo, e) in entries {
            for x in [s, v, i, j] {
                b.extend_from_slice(&x.to_le_bytes());
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

    #[test]
    fn the_cache_is_keyed_by_content_hash_sequence_view_and_pair() {
        let tag = b"a collection's tag data";
        let h = content_hash(tag);
        let e = Inbetween {
            width: 2,
            height: 1,
            offset_x: 1,
            offset_y: -3,
            pixels: vec![0, 7],
        };
        let b = file(
            h,
            &[
                ((17, 3, 2, 3), 1, "legs_held:0.620/rife4.6/u4/v1", e.clone()),
                ((17, 3, 3, 0), 2, "g/rife4.6/u4/v1", e.clone()),
            ],
        );
        let c = CollectionCache::parse(&b, h).expect("parsed");
        assert_eq!(c.get(17, 3, 2, 3), Some(&e));
        assert_eq!(c.get(17, 3, 3, 2), None);
        assert_eq!(c.get(17, 2, 2, 3), None);
        assert_eq!(c.get(17, 3, 3, 0), None);
        assert!(c.has_sequence(17) && !c.has_sequence(1));
        assert!(CollectionCache::parse(&b, content_hash(b"other data")).is_none());
        assert!(CollectionCache::parse(&b[..b.len() - 1], h).is_none());
        let mut v2 = b.clone();
        v2[4] = 2;
        assert!(CollectionCache::parse(&v2, h).is_none());
        assert_eq!(content_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(cache_file_name(FourCC(*b"orc ")), "6f726320.m2si");
    }

    #[test]
    fn doubling_keeps_drawn_frames_and_falls_back_per_view() {
        let drawn: Vec<Vec<Option<(usize, char)>>> = (0..3)
            .map(|f| vec![Some((f, 'd')), Some((f, 'd')), None])
            .collect();
        let out = doubled_frames(
            &drawn,
            0,
            |f| f.as_slice(),
            |_, vs| vs,
            |v, i, j| (v == 0).then_some((i * 10 + j, 'm')),
        );
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], drawn[0]);
        assert_eq!(out[2], drawn[1]);
        assert_eq!(out[1], vec![Some((1, 'm')), Some((0, 'd')), None]);
        assert_eq!(out[5], vec![Some((20, 'm')), Some((2, 'd')), None]);
    }
}
