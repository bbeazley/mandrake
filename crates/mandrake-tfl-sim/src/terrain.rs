use mandrake_tag::mesh::MeshTag;
use mandrake_tag::tfl::TflCell;

use crate::draw_site;
use crate::{Stop, TflSim};

const CORNERS: [(i32, i32); 9] = [
    (1, 1),
    (1, -1),
    (-1, -1),
    (-1, 1),
    (0, 1),
    (1, 0),
    (0, -1),
    (-1, 0),
    (0, 0),
];

const TRIANGLES: [[usize; 3]; 4] = [[4, 0, 8], [5, 8, 0], [8, 5, 4], [0, 4, 5]];

const SIDES: [[(i32, i32, bool); 8]; 2] = [
    [
        (0, 0, true),
        (0, 0, false),
        (0, 0, false),
        (0, 0, false),
        (0, 1, true),
        (1, 0, false),
        (0, -1, false),
        (-1, 0, true),
    ],
    [
        (0, 0, false),
        (0, 0, true),
        (0, 0, false),
        (0, 0, false),
        (0, 1, false),
        (1, 0, false),
        (0, -1, true),
        (-1, 0, true),
    ],
];

impl TflSim {
    pub(crate) fn width(&self) -> i32 {
        self.data.mesh.cell_width as i32
    }

    pub(crate) fn height_cells(&self) -> i32 {
        self.data.mesh.cell_height as i32
    }

    pub(crate) fn cell_at(&self, cx: i32, cy: i32) -> usize {
        if cx >= 0 && cx < self.width() && cy >= 0 && cy < self.height_cells() {
            (self.width() * cy + cx) as usize
        } else {
            0
        }
    }

    pub(crate) fn cell_offset(&self, cell: usize, dx: i32, dy: i32) -> Result<usize, Stop> {
        let k = cell as i64 + i64::from(dx) + i64::from(dy) * i64::from(self.width());
        if k < 0 || k as usize >= self.cells.len() {
            return Err(Stop::new(
                draw_site::CELL_OFF_MESH,
                "a cell off the mesh's array",
            ));
        }
        Ok(k as usize)
    }

    pub(crate) fn cell_read(&self, cell: usize, dx: i32, dy: i32) -> Result<TflCell, Stop> {
        let k = cell as i64 + i64::from(dx) + i64::from(dy) * i64::from(self.width());
        if k >= 0 && (k as usize) < self.cells.len() {
            return Ok(self.cells[k as usize]);
        }
        let past = k - self.cells.len() as i64;
        let c = self.memory_after_cells(past).ok_or_else(|| {
            Stop::new(
                draw_site::CELL_OFF_MESH,
                format!("a cell off the mesh's array ({k})"),
            )
        })?;
        crate::tc2_trace::log(|| format!("CR {} {k} {}", self.tick, c.physical_height));
        Ok(c)
    }

    fn memory_after_cells(&self, past: i64) -> Option<TflCell> {
        let b = &self.data.mesh_bytes;
        let u32_at = |o: usize| -> Option<usize> {
            Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize)
        };
        let end = u32_at(0x0c)? + u32_at(0x10)?;
        let (palette, size) = (u32_at(0x28)?, u32_at(0x2c)?);
        let at = end as i64 + past * 8;
        if past < 0 || palette != end || at + 8 > (palette + size) as i64 {
            return None;
        }
        let at = at as usize;
        let mut m = [0u8; 8];
        for (j, v) in m.iter_mut().enumerate() {
            let o = at + j - palette;
            let (e, f) = (o / 0x20 * 0x20, o % 0x20);
            let src = match f {
                0..2 | 2..4 | 8..10 => {
                    let w = f & !1;
                    w + 1 - (f - w)
                }
                4..8 => 4 + 3 - (f - 4),
                _ => f,
            };
            *v = *b.get(MeshTag::HEADER_SIZE + palette + e + src)?;
        }
        Some(TflCell {
            physical_height: i16::from_le_bytes([m[0], m[1]]),
            word_2: u16::from_le_bytes([m[2], m[3]]),
            flags: m[5],
            byte_5: m[4],
            first_object_index: i16::from_le_bytes([m[6], m[7]]),
        })
    }

    pub fn mesh_height(&self, x: i32, y: i32) -> Result<i32, Stop> {
        let cell = self.cell_at(x >> 10, y >> 10);
        if self.cells[cell].flags & 0x40 != 0 {
            return Ok(-0x1000);
        }
        let (mut fx, mut fy) = (x & 0x3ff, y & 0x3ff);
        let t = if (x ^ y) & 0x400 != 0 {
            usize::from(fy <= fx)
        } else {
            usize::from(fy >= 0x400 - fx) + 2
        };
        let h = |k: usize| -> Result<i32, Stop> {
            let (dx, dy) = CORNERS[TRIANGLES[t][k]];
            Ok(i32::from(self.cell_read(cell, dx, dy)?.physical_height))
        };
        let (h0, h1, h2) = (h(0)?, h(1)?, h(2)?);
        match t {
            0 => fy = 0x400 - fy,
            1 => fx = 0x400 - fx,
            3 => {
                fx = 0x400 - fx;
                fy = 0x400 - fy;
            }
            _ => {}
        }
        Ok((((h2 - h0) * fy + (h1 - h0) * fx) >> 10) + h0)
    }

    pub fn mesh_normal(&self, x: i32, y: i32) -> [i16; 3] {
        let cell = self.cell_at(x >> 10, y >> 10);
        let (fx, fy) = ((x & 0x3ff) as i16, (y & 0x3ff) as i16);
        let low = if ((y ^ x) >> 10) & 1 == 0 {
            0x400 - i32::from(fx) <= i32::from(fy)
        } else {
            fy <= fx
        };
        let word = self.cells[cell].word_2;
        let k = if low { word & 0xff } else { word >> 8 & 0xff };
        mesh_normal_table()[usize::from(k)]
    }

    pub fn closed_sides(&self, x: i32, y: i32, mask: u16) -> Result<u8, Stop> {
        let cx = (x >> 10).clamp(1, (self.width() - 2).max(1));
        let cy = (y >> 10).clamp(1, (self.height_cells() - 2).max(1));
        let cell = self.cell_at(cx, cy);
        let sides = &SIDES[(((x ^ y) >> 10) & 1) as usize];
        let mut out = 0u8;
        for (bit, &(dx, dy, low)) in sides.iter().enumerate() {
            let b = self.cell_read(cell, dx, dy)?.byte_5;
            let t = if low { b } else { b >> 4 } & 0xf;
            if u32::from(mask) & (1 << t) != 0 {
                out |= 1 << bit;
            }
        }
        Ok(out)
    }

    pub fn media_pixel(&self, px: i32, py: i32) -> Result<bool, Stop> {
        let row = usize::try_from(py)
            .ok()
            .and_then(|r| self.media.rows.get(r))
            .ok_or_else(|| {
                Stop::new(
                    draw_site::MEDIA_ROW_OFF_MAP,
                    format!("media row {py} off the map"),
                )
            })?;
        let mut inside = false;
        for &b in &self.media.data[*row..] {
            if i32::from(b) >= px {
                break;
            }
            inside = !inside;
        }
        Ok(inside)
    }

    pub fn media_at(&self, x: i32, y: i32) -> Result<bool, Stop> {
        self.media_pixel(x >> 6, y >> 6)
    }
}

pub fn mesh_normal_table() -> &'static [[i16; 3]; 256] {
    static TABLE: std::sync::OnceLock<[[i16; 3]; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let t = crate::trig::trig();
        let mut out = [[0i16; 3]; 256];
        let mut k = 0usize;
        let mut e: i32 = 0x4020;
        loop {
            let ei = (e >> 6 & 0x3ff) as usize;
            let c = i32::from(t.cosine[ei]);
            let mut n = (c << 16 >> 14) / 0x444 + 1;
            let step = 0x10000 / i64::from(n.max(1));
            let mut a: i32 = 0x20;
            while n > 0 && k < 256 {
                let ai = (a >> 6 & 0x3ff) as usize;
                out[k] = [
                    ((i32::from(t.cosine[ai]) * c) >> 14) as i16,
                    ((i32::from(t.sine[ai]) * c) >> 14) as i16,
                    t.sine[ei],
                ];
                k += 1;
                a += step as i32;
                n -= 1;
            }
            e -= 0x444;
            if k > 0xff {
                break;
            }
        }
        out
    })
}

#[derive(Debug, Clone, Default)]
pub struct MediaCoverage {
    pub data: Vec<i16>,
    pub rows: Vec<usize>,
}

impl MediaCoverage {
    pub fn new(mesh_bytes: &[u8], width: u32, height: u32) -> Result<Self, Stop> {
        let be32 = |o: usize| {
            mesh_bytes
                .get(o..o + 4)
                .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
        };
        let (offset, size) = (be32(0xc0), be32(0xc4));
        let end = (width * 16) as i16;
        let n = (height * 16) as usize;
        let data: Vec<i16> = if size == 0 {
            vec![end; n]
        } else {
            let at = 1024 + offset;
            mesh_bytes
                .get(at..at + size)
                .ok_or_else(|| {
                    Stop::new(
                        draw_site::MEDIA_COVERAGE,
                        "the media coverage is off the mesh",
                    )
                })?
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_be_bytes(*b))
                .collect()
        };
        let mut rows = Vec::with_capacity(n);
        let mut k = 0usize;
        for _ in 0..n {
            rows.push(k);
            loop {
                let v = *data.get(k).ok_or_else(|| {
                    Stop::new(
                        draw_site::MEDIA_COVERAGE,
                        "a media coverage row without its end",
                    )
                })?;
                k += 1;
                if v >= end {
                    break;
                }
            }
        }
        Ok(Self { data, rows })
    }
}
