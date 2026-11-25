use mandrake_common::game_options::LIMITED_VISIBILITY;

pub const SIGHT_RADII: [(i32, i32); 3] = [(0x0a, 0x0b), (0x0d, 0x0e), (5, 6)];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SightMask {
    pub radius: i32,
    pub rows: Vec<(i32, i32, Vec<u32>)>,
}

impl SightMask {
    pub fn new(inner: i32, outer: i32) -> Self {
        let isqrt = |v: i32| crate::groups::isqrt(v as u32) as i32;
        let mut rows = Vec::new();
        for dy in -outer..=outer {
            let half = isqrt((outer + 1) * (outer + 1) - dy * dy).min(outer);
            let inner_half = if dy < inner && -inner < dy {
                isqrt((inner + 1) * (inner + 1) - dy * dy).min(inner)
            } else {
                0
            };
            let mut words = Vec::new();
            let mut bit = 0;
            for dx in -half..=half {
                let v = if inner_half != 0 && -inner_half <= dx && dx <= inner_half {
                    3
                } else {
                    1
                };
                if bit == 0 {
                    words.push(0);
                }
                *words.last_mut().expect("a word") |= v << bit;
                bit += 2;
                if bit > 0x1f {
                    bit = 0;
                }
            }
            rows.push((outer - half, 2 * half + 1, words));
        }
        Self {
            radius: outer,
            rows,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SightMaps {
    pub width: i32,
    pub height: i32,
    pub centre: (i16, i16),
    pub sums: (i32, i32, i16),
    pub building: Vec<u32>,
    pub visible: Vec<u32>,
    pub explored: Vec<u32>,
    masks: Vec<SightMask>,
}

impl SightMaps {
    pub fn new(width: i32, height: i32) -> Self {
        let words = (width >> 4).max(0) as usize * height.max(0) as usize;
        Self {
            width,
            height,
            centre: (0, 0),
            sums: (0, 0, 0),
            building: vec![0; words],
            visible: vec![0; words],
            explored: vec![0; words],
            masks: SIGHT_RADII
                .iter()
                .map(|&(i, o)| SightMask::new(i, o))
                .collect(),
        }
    }

    fn bits(map: &[u32], width: i32, height: i32, cx: i32, cy: i32) -> u8 {
        if cx < 0 || cy < 0 || cx >= width || cy >= height {
            return 0;
        }
        let k = ((width >> 4) * cy + (cx >> 4)) as usize;
        map.get(k).map_or(0, |w| (w >> ((cx & 0xf) * 2) & 3) as u8)
    }

    pub fn visible_bits(&self, cx: i32, cy: i32) -> u8 {
        Self::bits(&self.visible, self.width, self.height, cx, cy)
    }

    pub fn explored(&self, cx: i32, cy: i32) -> bool {
        Self::bits(&self.explored, self.width, self.height, cx, cy) != 0
    }

    pub fn visible_cells(&self) -> Vec<u8> {
        (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .map(|(x, y)| self.visible_bits(x, y))
            .collect()
    }

    pub fn explored_cells(&self) -> Vec<bool> {
        (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .map(|(x, y)| self.explored(x, y))
            .collect()
    }

    pub fn swap(&mut self, game_options: u32) {
        let (sx, sy, n) = self.sums;
        let hidden = game_options & LIMITED_VISIBILITY != 0;
        if n != 0 || !hidden {
            if n != 0 {
                self.centre = ((sx / i32::from(n)) as i16, (sy / i32::from(n)) as i16);
            }
            for (e, b) in self.explored.iter_mut().zip(&self.building) {
                *e |= *b;
            }
            std::mem::swap(&mut self.visible, &mut self.building);
            let fill = if hidden { 0 } else { u32::MAX };
            self.building.iter_mut().for_each(|w| *w = fill);
        }
        self.sums = (0, 0, 0);
    }

    pub fn mark(&mut self, k: usize, cx: i32, cy: i32) {
        self.sums.0 += cx;
        self.sums.1 += cy;
        self.sums.2 = self.sums.2.wrapping_add(1);
        let Some(mask) = self.masks.get(k) else {
            return;
        };
        let r = mask.radius;
        let row_words = self.width >> 4;
        for (j, (start, width, words)) in mask.rows.iter().enumerate() {
            let y = cy - r + j as i32;
            if y < 0 || y >= self.height {
                continue;
            }
            let mut x = start + (cx - r);
            let mut left = *width;
            let mut done = 0i32;
            let mut wi = 0usize;
            let mut at = row_words as isize * y as isize + (x >> 4) as isize;
            while left > 0 {
                let shift = ((x & 0xf) * 2) as u32;
                let n = left.min(16).min(16 - (x & 0xf));
                let c = (done & 0xf) as u32;
                let mut v = words
                    .get(wi)
                    .copied()
                    .unwrap_or(0)
                    .wrapping_shr(c * 2)
                    .wrapping_shl(shift);
                if 16 - (done & 0xf) <= n {
                    wi += 1;
                    let next = words.get(wi).copied().unwrap_or(0);
                    v |= next.wrapping_shl(shift + (16 - c) * 2);
                }
                if x >= 0 && x < self.width {
                    let m = u32::MAX
                        .wrapping_shr((0x20 - 2 * n as u32) & 0x1f)
                        .wrapping_shl(shift);
                    if let Some(w) = usize::try_from(at)
                        .ok()
                        .and_then(|a| self.building.get_mut(a))
                    {
                        *w |= m & v;
                    }
                }
                done += n;
                at += 1;
                x += n;
                left -= n;
            }
        }
    }
}
