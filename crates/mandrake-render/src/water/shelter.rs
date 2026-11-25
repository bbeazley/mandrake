use super::*;

/// The fetch's rays, degrees around the wind.
pub(super) const FETCH_RAYS: [f64; 5] = [-20.0, -10.0, 0.0, 10.0, 20.0];

/// [`ShelterMap::sweep`]'s mark for past the map's edge.
pub(super) const OPEN: u16 = u16::MAX;

/// [`ShelterMap::sweep`]'s threads at most.
#[cfg(not(target_arch = "wasm32"))]
pub(super) const SHELTER_THREADS: usize = 4;

/// [`ShelterMap::sweep`]'s water vertices a thread at least (fewer cost less than a thread).
#[cfg(not(target_arch = "wasm32"))]
pub(super) const SHELTER_BAND_MIN: usize = 1500;

/// Fetch (metres) the shelter texture holds at most: `r = log2(1 + F) / log2(1 + FETCH_MAX)`.
pub const FETCH_MAX: f32 = 256.0;

/// The map's water for the fetch ([`ShelterMap::fetch`]), on the grid's vertices: where the
/// surface is drawn (a water level above the floor, `grid_info`), and each such vertex's
/// distance to the nearest dry one (a 3-4 chamfer distance transform, metres).
#[derive(Clone, Debug)]
pub struct ShelterMap {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) wet: Vec<bool>,
    pub(super) shore: Vec<f32>,
    pub(super) meters_per_cell: f32,
    /// The march's samples a wet vertex shows are wet ([`Self::sweep`]): its distance to the
    /// nearest dry vertex, less the samples' rounding; 0 where dry. On a grid with a margin as
    /// wide as the march goes, [`OPEN`] past the map.
    pub(super) padded: Vec<u16>,
    /// The march's reach (grid cells, [`FETCH_MAX`]) and its samples (the reach rounded up).
    pub(super) reach: f32,
    pub(super) samples: usize,
}

impl ShelterMap {
    /// From `grid_info`'s `w x h` vertices.
    pub fn new(info: &[[f32; 4]], (w, h): (u32, u32), meters_per_cell: f32) -> Self {
        let meters_per_cell = if meters_per_cell.is_finite() {
            meters_per_cell.max(0.1)
        } else {
            1.589
        };
        let (w, h) = (w as usize, h as usize);
        let wet: Vec<bool> = (0..w * h)
            .map(|i| info.get(i).is_some_and(|v| v[2] > 0.5 && v[1] > v[0]))
            .collect();
        // Chamfer 3-4: dry vertices 0, water vertices the distance in thirds of a cell.
        let dry: Vec<bool> = wet.iter().map(|&x| !x).collect();
        let d = chamfer(&dry, (w, h));
        let shore = d
            .iter()
            .map(|&v| (v as f32 / 3.0 * meters_per_cell).min(FETCH_MAX))
            .collect();
        // A sample is half a cell a side off its line at most, so two are within their steps
        // plus sqrt 2 of each other: from a wet vertex E from the nearest dry one, the samples
        // before the `skip`th, s - 1 + sqrt 2 < E, are wet.
        let skip = squared_distance(&dry, (w, h))
            .iter()
            .zip(&wet)
            .map(|(&d2, &wet)| {
                if wet {
                    let x = d2.sqrt() + 1.0 - std::f32::consts::SQRT_2;
                    (x.ceil() - 1.0).clamp(1.0, 4096.0) as u16
                } else {
                    0
                }
            })
            .collect::<Vec<u16>>();
        let reach = FETCH_MAX / meters_per_cell;
        // Any ray longer than the map diagonal + 1 has already exited into `OPEN`, so capping
        // `samples` bounds memory while preserving exact fetch values.
        let max_steps = ((w as f64).hypot(h as f64).ceil() as usize + 1).max(1);
        let samples = (reach.ceil().max(1.0) as usize).min(max_steps);
        let pad = samples + 1;
        let pw = w + 2 * pad;
        let mut padded = vec![OPEN; pw * (h + 2 * pad)];
        for r in 0..h {
            let at = (r + pad) * pw + pad;
            padded[at..at + w].copy_from_slice(&skip[r * w..(r + 1) * w]);
        }
        Self {
            width: w,
            height: h,
            wet,
            shore,
            meters_per_cell,
            padded,
            reach,
            samples,
        }
    }

    /// The fetch per vertex (RGBA8, `r` log-encoded, [`FETCH_MAX`]): the open water upwind
    /// (against the wind blowing toward `direction`, radians in render x/z, to the first dry
    /// vertex; the map's edge counts as open water), blended by `windiness` with the distance to
    /// the nearest shore. Dry vertices 0. Pure map data.
    ///
    /// Upwind over [`FETCH_RAYS`] directions spread around the wind, weighted like the waves'
    /// spread (`water::fft::spread`, cos^8 of half the angle): a lee's edge is diffuse, not one
    /// ray's straight line. Each ray is one directional sweep ([`Self::sweep`]), O(vertices).
    pub fn fetch(&self, direction: f64, windiness: f64) -> Vec<u8> {
        let cells = self.sweep(&Self::rays(direction));
        self.encode(&cells, windiness)
    }

    /// [`Self::fetch`] by marching each vertex's rays a cell a step (the prototype's, about
    /// 80 times slower on a big lake): the reference [`Self::fetch`]'s sweep is tested against.
    #[doc(hidden)]
    pub fn fetch_march(&self, direction: f64, windiness: f64) -> Vec<u8> {
        let (w, h) = (self.width, self.height);
        let reach = FETCH_MAX / self.meters_per_cell.max(1e-3);
        let rays = Self::rays(direction);
        let total: f32 = rays.iter().map(|r| r.2).sum();
        let mut cells = vec![0.0f32; w * h];
        for r in 0..h {
            for c in 0..w {
                if !self.wet[r * w + c] {
                    continue;
                }
                for &(dc, dr, weight) in &rays {
                    let (mut x, mut y, mut n) = (c as f32, r as f32, 0.0f32);
                    while n < reach {
                        x += dc;
                        y += dr;
                        let (xi, yi) = (x.round(), y.round());
                        if xi < 0.0 || yi < 0.0 || xi >= w as f32 || yi >= h as f32 {
                            n = reach;
                            break;
                        }
                        if !self.wet[yi as usize * w + xi as usize] {
                            break;
                        }
                        n += 1.0;
                    }
                    cells[r * w + c] += weight * n / total;
                }
            }
        }
        self.encode(&cells, windiness)
    }

    /// The upwind steps (render x falls with the column, z grows with the row) and weights of
    /// [`FETCH_RAYS`] around `direction`.
    pub(super) fn rays(direction: f64) -> [(f32, f32, f32); FETCH_RAYS.len()] {
        FETCH_RAYS.map(|deg| {
            let a = direction + deg.to_radians();
            let weight = (deg.to_radians() * 0.5).cos().powi(8) as f32;
            (a.cos() as f32, -(a.sin() as f32), weight)
        })
    }

    /// The upwind open water per vertex (grid cells, wet vertices only): over `rays` (unit
    /// steps and weights), the march's count of the wet samples (a step apart, each the vertex
    /// nearest) before the first dry one, all `reach` past the map's edge.
    ///
    /// The samples' offsets are the same from every vertex, so they are made once; and from a
    /// wet sample the march skips the samples its distance to the nearest dry vertex shows are
    /// wet (`skip`), so open water costs a few steps a ray rather than one a cell. A vertex's
    /// rays step together (independent loads, not one chain). Exactly the march, but for a
    /// sample a float's last bit puts on the other side of a half cell (the march sums its
    /// steps, here they are multiplied).
    pub(super) fn sweep(&self, rays: &[(f32, f32, f32)]) -> Vec<f32> {
        let (w, h) = (self.width, self.height);
        let (samples, pad) = (self.samples, self.samples + 1);
        let pw = (w + 2 * pad) as isize;
        let reach = self.reach;
        let total: f32 = rays.iter().map(|r| r.2).sum();
        // Per ray and sample its offset on the padded grid (rounded half up, as the march's
        // positions, never below zero on the map, are), and its share of the fetch.
        let rays: Vec<(Vec<isize>, f32)> = rays
            .iter()
            .map(|&(dc, dr, weight)| {
                let offsets = (0..=samples)
                    .map(|k| {
                        let k = k as f32;
                        let x = (k * dc + 0.5).floor() as isize;
                        let y = (k * dr + 0.5).floor() as isize;
                        y * pw + x
                    })
                    .collect();
                (offsets, weight / total)
            })
            .collect();
        let rows = |first: usize, cells: &mut [f32]| {
            for (n, out) in cells.chunks_exact_mut(w).enumerate() {
                let r = first + n;
                let row = ((r + pad) as isize) * pw + pad as isize;
                for (c, out) in out.iter_mut().enumerate() {
                    if !self.wet[r * w + c] {
                        continue;
                    }
                    let at = row + c as isize;
                    for (offsets, share) in &rays {
                        let mut k = 1;
                        let run = loop {
                            let skip = self.padded[(at + offsets[k]) as usize];
                            if skip == 0 {
                                break (k - 1) as f32;
                            }
                            if skip == OPEN {
                                break reach;
                            }
                            if k == samples {
                                break samples as f32;
                            }
                            k = (k + usize::from(skip)).min(samples);
                        };
                        *out += share * run;
                    }
                }
            }
        };
        let mut cells = vec![0.0f32; w * h];
        // Natively a few threads take a band of rows each (Tarn's lake is ~1.5 ms on one).
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Bands of about as many water vertices each; one for a small water.
            let wet_rows: Vec<usize> = self
                .wet
                .chunks_exact(w)
                .map(|row| row.iter().filter(|&&x| x).count())
                .collect();
            let all: usize = wet_rows.iter().sum();
            let threads = std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .clamp(1, SHELTER_THREADS)
                .min(all.div_ceil(SHELTER_BAND_MIN).max(1));
            let mut cuts = vec![0];
            let mut sum = 0;
            for (r, &n) in wet_rows.iter().enumerate() {
                sum += n;
                if cuts.len() < threads && sum * threads >= all * cuts.len() {
                    cuts.push(r + 1);
                }
            }
            std::thread::scope(|scope| {
                let mut rest = cells.as_mut_slice();
                for (b, &first) in cuts.iter().enumerate() {
                    let end = cuts.get(b + 1).copied().unwrap_or(h);
                    let (band, tail) = rest.split_at_mut((end - first) * w);
                    rest = tail;
                    let rows = &rows;
                    if b + 1 == cuts.len() {
                        rows(first, band);
                    } else {
                        scope.spawn(move || rows(first, band));
                    }
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        rows(0, &mut cells);
        cells
    }

    /// RGBA8 of the upwind open water (`cells` per vertex, grid cells) blended with the shore.
    pub(super) fn encode(&self, cells: &[f32], windiness: f64) -> Vec<u8> {
        let scale = 1.0 / (1.0 + FETCH_MAX).log2();
        let wind = windiness as f32;
        let mut out = vec![0u8; self.width * self.height * 4];
        for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            px[3] = 255;
            if !self.wet[i] {
                continue;
            }
            let upwind = (cells[i] + 0.5) * self.meters_per_cell;
            let fetch = (wind * upwind + (1.0 - wind) * self.shore[i]).min(FETCH_MAX);
            px[0] = ((1.0 + fetch).log2() * scale * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
        out
    }

    /// Decode [`Self::fetch`]'s `r` (as the shader does).
    pub fn decode(r: u8) -> f32 {
        (f32::from(r) / 255.0 * (1.0 + FETCH_MAX).log2()).exp2() - 1.0
    }
}

/// The first fetch's histogram (water vertices by fetch, metres), in the log.
pub(super) fn log_fetch(fetch: &[u8], map: &ShelterMap) {
    let edges = [1.0f32, 3.0, 10.0, 30.0, 100.0, FETCH_MAX + 1.0];
    let mut counts = [0usize; 6];
    for (i, px) in fetch.as_chunks::<4>().0.iter().enumerate() {
        if !map.wet[i] {
            continue;
        }
        let f = ShelterMap::decode(px[0]);
        counts[edges.iter().position(|&e| f < e).unwrap_or(5)] += 1;
    }
    tracing::info!(
        "--water enhanced shelter: water vertices by fetch <1 m {}, 1-3 m {}, 3-10 m {}, 10-30 m {}, 30-100 m {}, 100-256 m {}",
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4],
        counts[5]
    );
}

/// A fetch job: its number, the wind's direction (radians) and windiness.
pub(super) type ShelterJob = (u64, f64, f64);

/// The shelter worker's job queue and its results (job number, texels).
#[cfg(not(target_arch = "wasm32"))]
pub(super) type ShelterWorker = (
    std::sync::mpsc::Sender<ShelterJob>,
    std::sync::mpsc::Receiver<(u64, Vec<u8>)>,
);

/// `shelter`'s fetch texture and who computes it: a worker thread natively (latest wins: a job
/// the wind has overtaken is dropped unseen, and only the newest result is uploaded), inline on
/// the web. The wind turns fast in game (a 330 degree swing in 6 s), so this runs several times
/// a second.
pub(super) struct Shelter {
    pub(super) texture: wgpu::Texture,
    pub(super) size: (u32, u32),
    /// The wind key the last job was asked for (degrees in 10s, windiness in 1/20).
    pub(super) made: Option<(i64, i64)>,
    /// The last job asked for and the last one uploaded.
    pub(super) sent: u64,
    pub(super) shown: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) worker: Option<ShelterWorker>,
    /// The map and coverage, when computed on this thread.
    pub(super) inline: Option<(ShelterMap, Vec<u8>)>,
}

/// [`ShelterMap::fetch`] with the coverage in `g`; the first one's histogram in the log.
pub(super) fn shelter_texels(map: &ShelterMap, coverage: &[u8], job: ShelterJob) -> Vec<u8> {
    let (n, direction, windiness) = job;
    #[cfg(not(target_arch = "wasm32"))]
    let started = std::time::Instant::now();
    let mut texels = map.fetch(direction, windiness);
    for (px, &g) in texels.chunks_mut(4).zip(coverage) {
        px[1] = g;
    }
    #[cfg(not(target_arch = "wasm32"))]
    tracing::debug!(
        "water shelter: fetch recomputed in {:.3} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    if n == 1 {
        log_fetch(&texels, map);
    }
    texels
}

impl Shelter {
    pub(super) fn new(texture: wgpu::Texture, map: ShelterMap, coverage: Vec<u8>) -> Self {
        let size = (map.width as u32, map.height as u32);
        let mut shelter = Self {
            texture,
            size,
            made: None,
            sent: 0,
            shown: 0,
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
            inline: None,
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (job_tx, job_rx) = std::sync::mpsc::channel::<ShelterJob>();
            let (out_tx, out_rx) = std::sync::mpsc::channel();
            let (worker_map, worker_coverage) = (map.clone(), coverage.clone());
            let spawned = std::thread::Builder::new()
                .name("water-shelter".into())
                .spawn(move || {
                    while let Ok(mut job) = job_rx.recv() {
                        // Only the newest of the jobs waiting.
                        while let Ok(newer) = job_rx.try_recv() {
                            job = newer;
                        }
                        let texels = shelter_texels(&worker_map, &worker_coverage, job);
                        if out_tx.send((job.0, texels)).is_err() {
                            break;
                        }
                    }
                });
            if spawned.is_ok() {
                shelter.worker = Some((job_tx, out_rx));
                return shelter;
            }
        }
        shelter.inline = Some((map, coverage));
        shelter
    }

    /// Upload the newest fetch the worker finished, and ask for the fetch again when the wind
    /// turned by 10 degrees or its share changed by 1/20. The worker's result shows a frame or so
    /// later.
    pub(super) fn update(&mut self, queue: &wgpu::Queue, direction: f64, windiness: f64) {
        self.receive(queue, false);
        let key = (
            (direction.to_degrees() / 10.0).round() as i64,
            (windiness * 20.0).round() as i64,
        );
        if self.made == Some(key) {
            return;
        }
        self.made = Some(key);
        self.sent += 1;
        let job = (self.sent, direction, windiness);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((tx, _)) = &self.worker {
            let _ = tx.send(job);
            return;
        }
        if let Some((map, coverage)) = &self.inline {
            let texels = shelter_texels(map, coverage, job);
            self.upload(queue, &texels);
            self.shown = job.0;
        }
    }

    /// Upload the newest finished fetch; with `wait`, until the last one asked for is in.
    pub(super) fn receive(&mut self, queue: &wgpu::Queue, wait: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((_, rx)) = &self.worker {
            let mut newest = None;
            while let Ok(done) = rx.try_recv() {
                newest = Some(done);
            }
            while wait && newest.as_ref().map_or(self.shown, |d| d.0) < self.sent {
                match rx.recv() {
                    Ok(done) => newest = Some(done),
                    Err(_) => break,
                }
            }
            if let Some((n, texels)) = newest {
                self.upload(queue, &texels);
                self.shown = n;
            }
        }
        let _ = (queue, wait);
    }

    pub(super) fn upload(&self, queue: &wgpu::Queue, texels: &[u8]) {
        let (w, h) = self.size;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
}
