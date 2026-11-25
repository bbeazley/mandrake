pub(crate) struct Sha256 {
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
    pub(crate) fn new() -> Self {
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

    pub(crate) fn update(&mut self, mut data: &[u8]) {
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

    pub(crate) fn short_hex(mut self) -> String {
        let bits = self.len * 8;
        self.update(&[0x80]);
        while self.block.len() != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        format!("{:08x}{:08x}", self.h[0], self.h[1])
    }
}

pub(crate) fn sha256_short(data: &[u8]) -> String {
    let mut s = Sha256::new();
    s.update(data);
    s.short_hex()
}

pub(crate) const GAME_OVER_TICKS: i64 = 303;

pub(crate) fn early_end(last: u32, end: i64) -> String {
    if i64::from(last) > end + GAME_OVER_TICKS {
        format!("early end: the film goes to {last}")
    } else {
        "none".into()
    }
}

#[test]
fn sha256_is_sha256() {
    assert_eq!(sha256_short(b"abc"), "ba7816bf8f01cfea");
    assert_eq!(sha256_short(&[b'a'; 1000]), "41edece42d63e8d9");
}

#[test]
fn an_early_end_is_a_miss() {
    assert_eq!(early_end(2736, 2560), "none");
    assert_eq!(early_end(2736, 2433), "none");
    assert_eq!(early_end(2736, 2432), "early end: the film goes to 2736");
}
