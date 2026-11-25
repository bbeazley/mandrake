use mandrake_tag::collection::TerrainTexture;
use mandrake_tag::mesh::MediaCoverage;
use mandrake_view::water::bed::{BED_SEED, BED_VERSION, repaint_water_paint};
use tracing::info;

pub fn repainted_texture(
    texture: &TerrainTexture,
    mesh: &mandrake_tag::MeshTag,
    coverage: &MediaCoverage,
) -> Option<TerrainTexture> {
    let start = crate::clock::Instant::now();
    let key = input_key(texture, mesh, coverage);
    if let Some(changes) = read_cache(key, texture.color_index.len()) {
        if changes.is_empty() {
            info!(
                "--water enhanced: no river bed (cached) in {:.1} ms",
                start.elapsed().as_secs_f64() * 1e3
            );
            return None;
        }
        let mut out = texture.clone();
        for &(o, v) in &changes {
            out.color_index[o as usize] = v;
        }
        info!(
            "--water enhanced: river bed from the cache ({} px changed) in {:.1} ms",
            changes.len(),
            start.elapsed().as_secs_f64() * 1e3
        );
        return Some(out);
    }
    match repaint_water_paint(texture, mesh, coverage, BED_SEED) {
        Ok((index, report)) => {
            let changes: Vec<(u32, u8)> = index
                .iter()
                .zip(&texture.color_index)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(o, (&a, _))| (o as u32, a))
                .collect();
            write_cache(key, &changes);
            info!(
                "--water enhanced: river bed repainted in {:.0} ms: {report:?}",
                start.elapsed().as_secs_f64() * 1e3
            );
            let mut out = texture.clone();
            out.color_index = index;
            Some(out)
        }
        Err(why) => {
            write_cache(key, &[]);
            info!(
                "--water enhanced: no river bed ({why:?}) in {:.1} ms",
                start.elapsed().as_secs_f64() * 1e3
            );
            None
        }
    }
}

pub fn painted_rgba(
    rgba: &[u8],
    texels: &mandrake_view::landscape::LandscapeTexels,
    original: &[u8],
) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let (index, shade, table) = (texels.color_index(), texels.shade(), texels.table());
    for (o, (&now, &was)) in index.iter().zip(original).enumerate() {
        if now != was && o * 4 + 3 < out.len() {
            let c = table
                .get(usize::from(shade[o]) << 8 | usize::from(was))
                .copied()
                .unwrap_or_default();
            out[o * 4..o * 4 + 3].copy_from_slice(&c);
        }
    }
    out
}

fn input_key(
    texture: &TerrainTexture,
    mesh: &mandrake_tag::MeshTag,
    coverage: &MediaCoverage,
) -> u64 {
    let mut h = Hash64::default();
    h.u64(u64::from(BED_VERSION));
    h.u64(BED_SEED);
    h.u64(u64::from(texture.width) << 32 | u64::from(texture.height));
    h.bytes(&texture.color_index);
    for c in &texture.palette16 {
        h.u64(u64::from(c[0]) << 32 | u64::from(c[1]) << 16 | u64::from(c[2]));
    }
    h.u64(texture.color_count as u64);
    h.u64(u64::from(mesh.cell_width) << 16 | u64::from(mesh.cell_height));
    for c in &mesh.cells {
        h.u64(
            u64::from(c.physical_height as u16) << 32
                | u64::from(c.flags) << 16
                | u64::from(c.media_height as u16),
        );
    }
    for row in &coverage.rows {
        h.u64(row.len() as u64);
        for &v in row {
            h.u64(v as u16 as u64);
        }
    }
    h.0
}

struct Hash64(u64);

impl Default for Hash64 {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hash64 {
    fn u64(&mut self, v: u64) {
        self.0 = (self.0 ^ v).wrapping_mul(0x0000_0100_0000_01b3);
        self.0 ^= self.0 >> 29;
    }

    fn bytes(&mut self, b: &[u8]) {
        let (words, rest) = b.as_chunks::<8>();
        for w in words {
            self.u64(u64::from_le_bytes(*w));
        }
        for &v in rest {
            self.u64(u64::from(v));
        }
        self.u64(b.len() as u64);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn cache_dir() -> Option<std::path::PathBuf> {
    Some(
        crate::user_dirs::cache_base()?
            .join("myth2")
            .join("water-bed"),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn read_cache(key: u64, len: usize) -> Option<Vec<(u32, u8)>> {
    read_cache_in(&cache_dir()?, key, len)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_cache(key: u64, changes: &[(u32, u8)]) {
    if let Some(dir) = cache_dir() {
        write_cache_in(&dir, key, changes);
    }
}

#[cfg(not(target_arch = "wasm32"))]
const MAGIC: &[u8; 4] = b"M2WB";

#[cfg(not(target_arch = "wasm32"))]
fn read_cache_in(dir: &std::path::Path, key: u64, len: usize) -> Option<Vec<(u32, u8)>> {
    let data = std::fs::read(dir.join(format!("{key:016x}.bin"))).ok()?;
    let body = data.strip_prefix(MAGIC.as_slice())?;
    let (count, body) = body.split_first_chunk::<8>()?;
    let count = u64::from_le_bytes(*count) as usize;
    let (head, check) = body.split_at_checked(count.checked_mul(5)?)?;
    if check.len() != 8 || u64::from_le_bytes(check.try_into().ok()?) != key {
        return None;
    }
    let out: Vec<(u32, u8)> = head
        .as_chunks::<5>()
        .0
        .iter()
        .map(|c| (u32::from_le_bytes([c[0], c[1], c[2], c[3]]), c[4]))
        .collect();
    out.iter().all(|&(o, _)| (o as usize) < len).then_some(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_cache_in(dir: &std::path::Path, key: u64, changes: &[(u32, u8)]) {
    let path = dir.join(format!("{key:016x}.bin"));
    let mut data = Vec::with_capacity(20 + changes.len() * 5);
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&(changes.len() as u64).to_le_bytes());
    for &(o, v) in changes {
        data.extend_from_slice(&o.to_le_bytes());
        data.push(v);
    }
    data.extend_from_slice(&key.to_le_bytes());
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let result = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&tmp, &data))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        tracing::warn!(
            "--water enhanced: river bed not cached at {}: {e}",
            path.display()
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn read_cache(_key: u64, _len: usize) -> Option<Vec<(u32, u8)>> {
    None
}

#[cfg(target_arch = "wasm32")]
fn write_cache(_key: u64, _changes: &[(u32, u8)]) {}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn cache_round_trips_and_rejects_another_key() {
        let dir = std::env::temp_dir().join(format!("m2-water-bed-{}", std::process::id()));
        let changes = vec![(3u32, 7u8), (10, 200)];
        write_cache_in(&dir, 0xabcd, &changes);
        assert_eq!(read_cache_in(&dir, 0xabcd, 11), Some(changes));
        assert_eq!(read_cache_in(&dir, 0xabce, 11), None);
        assert_eq!(
            read_cache_in(&dir, 0xabcd, 10),
            None,
            "an offset past the map"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_changes_with_any_byte() {
        let mut b = vec![1u8; 77];
        let key = |b: &[u8]| {
            let mut h = Hash64::default();
            h.bytes(b);
            h.0
        };
        let a = key(&b);
        for i in [0, 8, 70, 76] {
            b[i] = 2;
            assert_ne!(a, key(&b), "{i}");
            b[i] = 1;
        }
    }
}
