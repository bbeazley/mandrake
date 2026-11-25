use mandrake_common::M2Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pix0r1Image {
    pub width: u32,
    pub height: u32,
    pub primary: u8,
    pub rgb: Vec<u8>,
    pub bits_read: u64,
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: u32,
    cur: u32,
}

impl BitReader<'_> {
    fn word(&self, index: u32) -> u32 {
        let start = index as usize * 4;
        let mut bytes = [0u8; 4];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = self.data.get(start + i).copied().unwrap_or(0);
        }
        u32::from_le_bytes(bytes)
    }

    fn bits(&mut self, n: u32) -> u8 {
        let p = self.pos & 31;
        let r = 32 - p;
        let v = if n < r {
            let v = self.cur >> (32 - n);
            self.cur <<= n;
            v
        } else {
            let k = n - r;
            let mut v = self.cur >> p;
            let next = self.word((self.pos + n) >> 5);
            if k != 0 {
                v |= (next >> (32 - k)) << r;
                self.cur = next << k;
            } else {
                self.cur = next;
            }
            v
        };
        self.pos += n;
        v as u8
    }

    fn bit(&mut self) -> bool {
        let b = self.cur >> 31 != 0;
        self.cur <<= 1;
        if self.pos & 31 == 31 {
            self.cur = self.word((self.pos + 1) >> 5);
        }
        self.pos += 1;
        b
    }

    fn residual(&mut self, n: u8) -> u8 {
        if n == 7 {
            return 0;
        }
        let mut q: u32 = 0;
        while self.bit() {
            q = q.wrapping_add(1);
        }
        let mut v = q.wrapping_shl(n as u32) as u8;
        if n != 0 {
            v |= self.bits(n as u32);
        }
        v
    }
}

fn unzig(b: u8) -> u8 {
    let b = b as i32;
    ((-(b & 1) ^ b) >> 1) as u8
}

pub fn predict(a: i32, b: i32, c: i32) -> i32 {
    if a < b {
        if c < b {
            if a < c { a + b - c } else { (a + 7 * b) >> 3 }
        } else {
            (b + 7 * a) >> 3
        }
    } else if c < a {
        if c <= b { (b + 7 * a) >> 3 } else { a + b - c }
    } else {
        (a + 7 * b) >> 3
    }
}

pub fn decode(data: &[u8]) -> Result<Pix0r1Image, M2Error> {
    if data.len() < 4 {
        return Err(M2Error::TagParse(
            "pix0r1: stream shorter than its header".into(),
        ));
    }
    let w0 = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let width = w0 >> 20;
    let height = (w0 & 0xfffff) >> 8;
    let primary = ((w0 & 0xff) >> 6) as u8;
    if width == 0 || height == 0 {
        return Err(M2Error::TagParse(format!(
            "pix0r1: empty image {width} x {height}"
        )));
    }
    if primary == 3 {
        return Err(M2Error::TagParse(
            "pix0r1: primary channel 3 (undefined in the engine)".into(),
        ));
    }
    let (w, h) = (width as usize, height as usize);
    let bw = w.div_ceil(8);
    let bh = h.div_ceil(8);

    let mut rd = BitReader {
        data,
        pos: 26,
        cur: w0 << 26,
    };
    let mut widths = vec![0u8; bw * bh * 3];
    for v in widths.iter_mut() {
        *v = rd.bits(3);
    }

    let mut out = vec![0u8; w * h * 3];
    let read_res = |rd: &mut BitReader<'_>, x: usize, y: usize| -> [u8; 3] {
        let blk = ((x >> 3) + (y >> 3) * bw) * 3;
        [
            rd.residual(widths[blk]),
            rd.residual(widths[blk + 1]),
            rd.residual(widths[blk + 2]),
        ]
    };
    let r0 = read_res(&mut rd, 0, 0);
    out[..3].copy_from_slice(&r0);
    let from_one = |out: &mut [u8], px: usize, n: usize, r: [u8; 3], p: usize| {
        let (s1, s2) = ((p + 2) % 3, (p + 1) % 3);
        let np = out[n + p];
        let new = np.wrapping_sub(unzig(r[0]));
        let d1 = np.wrapping_sub(out[n + s1]);
        let d2 = np.wrapping_sub(out[n + s2]);
        out[px + p] = new;
        out[px + s1] = unzig(r[1]).wrapping_add(new).wrapping_sub(d1);
        out[px + s2] = new.wrapping_add(unzig(r[2])).wrapping_sub(d2);
    };
    for x in 1..w {
        let r = read_res(&mut rd, x, 0);
        from_one(&mut out, x * 3, (x - 1) * 3, r, 0);
    }
    let p = primary as usize;
    let (s1, s2) = ((p + 2) % 3, (p + 1) % 3);
    for y in 1..h {
        let row = y * w * 3;
        let r = read_res(&mut rd, 0, y);
        from_one(&mut out, row, row - w * 3, r, p);
        for x in 1..w {
            let r = read_res(&mut rd, x, y);
            let px = row + x * 3;
            let a = px - 3;
            let b = px - w * 3;
            let c = b - 3;
            let pv = |i: usize| out[i + p] as i32;
            let diff = |i: usize, s: usize| out[i + p] as i32 - out[i + s] as i32;
            let pred = predict(pv(a), pv(b), pv(c)) as u8;
            let pd1 = predict(diff(a, s1), diff(b, s1), diff(c, s1)) as u8;
            let pd2 = predict(diff(a, s2), diff(b, s2), diff(c, s2)) as u8;
            let new = pred.wrapping_sub(unzig(r[0]));
            out[px + p] = new;
            out[px + s1] = unzig(r[1]).wrapping_add(new).wrapping_sub(pd1);
            out[px + s2] = new.wrapping_add(unzig(r[2])).wrapping_sub(pd2);
        }
    }

    Ok(Pix0r1Image {
        width,
        height,
        primary,
        rgb: out,
        bits_read: rd.pos as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Writer {
        words: Vec<u32>,
        pos: u32,
    }

    impl Writer {
        fn put(&mut self, v: u32, n: u32) {
            for i in 0..n {
                let p = self.pos & 31;
                let r = 32 - p;
                let bit = if n > r && i < r {
                    (v >> (r - 1 - i)) & 1
                } else if n > r {
                    (v >> (n - 1 - (i - r))) & 1
                } else {
                    (v >> (n - 1 - i)) & 1
                };
                let at = self.pos + i;
                while self.words.len() <= (at >> 5) as usize {
                    self.words.push(0);
                }
                self.words[(at >> 5) as usize] |= bit << (31 - (at & 31));
            }
            self.pos += n;
        }
        fn bytes(&self) -> Vec<u8> {
            self.words.iter().flat_map(|w| w.to_le_bytes()).collect()
        }
    }

    #[test]
    fn unzig_and_predict() {
        assert_eq!(
            [0, 1, 2, 3, 4].map(unzig),
            [0, 0xff, 1, 0xfe, 2],
            "0, -1, 1, -2, 2"
        );
        assert_eq!(predict(10, 20, 15), 15);
        assert_eq!(predict(10, 20, 25), (20 + 70) >> 3);
        assert_eq!(predict(10, 20, 5), (10 + 140) >> 3);
        assert_eq!(predict(-16, -8, -100), (-16 - 56) >> 3);
    }

    #[test]
    fn split_field_takes_high_bits_from_the_next_word() {
        let mut wr = Writer {
            words: vec![],
            pos: 0,
        };
        wr.put(0, 29);
        wr.put(0b101101, 6);
        let bytes = wr.bytes();
        let mut rd = BitReader {
            data: &bytes,
            pos: 0,
            cur: wr.words[0],
        };
        rd.bits(7);
        rd.bits(7);
        rd.bits(7);
        rd.bits(7);
        assert_eq!(rd.bits(1), 0);
        assert_eq!(rd.bits(6), 0b101101);
    }

    #[test]
    fn flat_image_from_width_7_blocks() {
        let mut wr = Writer {
            words: vec![],
            pos: 0,
        };
        wr.put(9, 12);
        wr.put(2, 12);
        wr.put(1, 2);
        for _ in 0..2 * 3 {
            wr.put(7, 3);
        }
        let img = decode(&wr.bytes()).unwrap();
        assert_eq!((img.width, img.height, img.primary), (9, 2, 1));
        assert!(img.rgb.iter().all(|&b| b == 0));
    }
}
