use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Default download URL for the compact f16 `rife46.m2rw` weights archive.
/// Overridable via `SPRITE_INTERP_WEIGHTS_URL`.
pub const DEFAULT_WEIGHTS_URL: Option<&str> =
    Some("https://github.com/bbeazley/mandrake/releases/download/weights-v1/rife46.m2rw");
/// Upstream Practical-RIFE / vs-rife MIT weights fallback when `DEFAULT_WEIGHTS_URL` is `None`.
pub const RIFE46_WEIGHTS_URL: &str =
    "https://github.com/HolyWu/vs-rife/releases/download/model/flownet_v4.6.pkl";
pub const RIFE46_PKL_SHA256: &str =
    "008646e761f0e67cb77f0c6c44cfe3c3e5a05d9d9465311b9681ca650ce030db";
pub const RIFE46_M2RW_SHA256: &str =
    "cf7f2a5f98737c20c79134e4a0ee8ece6b84de303226c605ff9a3cd311a73a9b";
pub const RIFE46_M2RW_FILE: &str = "rife46.m2rw";

const M2RW_MAGIC: &[u8; 4] = b"M2RW";
const M2RW_VERSION: u16 = 1;
const M2RW_FLAGS: u16 = 0;

pub const RIFE46_SHADER_SRC: &str = include_str!("../shaders/rife46.wgsl");

const BLOCK_SPECS: [(usize, usize, f32); 4] =
    [(7, 192, 8.0), (12, 128, 4.0), (12, 96, 2.0), (12, 64, 1.0)];

const RES_BLOCKS: usize = 8;
const LAST_CONV_OUT: usize = 24;
const LAST_CONV_USED_OUT: usize = 20;

pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((data.len() + 9 + 63) & !63);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, &v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256(data);
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

pub fn f32_to_f16(val: f32) -> u16 {
    let bits = val.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let frac = bits & 0x007f_ffff;

    if exp == 255 {
        return sign | 0x7c00 | if frac != 0 { 0x0200 } else { 0 };
    }
    let new_exp = exp - 127 + 15;
    if new_exp >= 31 {
        return sign | 0x7c00;
    }
    if new_exp <= 0 {
        if new_exp < -10 {
            return sign;
        }
        let mant = frac | 0x0080_0000;
        let shift = (14 - new_exp) as u32;
        let half = 1u32 << (shift - 1);
        let rem_mask = (1u32 << shift) - 1;
        let mut rounded = mant >> shift;
        let rem = mant & rem_mask;
        if rem > half || (rem == half && (rounded & 1) != 0) {
            rounded += 1;
        }
        return sign | (rounded as u16);
    }
    let mut h_exp = (new_exp as u32) << 10;
    let mut h_frac = frac >> 13;
    let rem = frac & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && (h_frac & 1) != 0) {
        h_frac += 1;
        if h_frac == 0x400 {
            h_frac = 0;
            h_exp += 0x400;
        }
    }
    sign | ((h_exp | h_frac) as u16)
}

#[cfg(test)]
fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exp = (h >> 10) & 0x1f;
    let frac = u32::from(h & 0x03ff);
    let bits = if exp == 0 {
        if frac == 0 {
            sign
        } else {
            let mut m = frac;
            let mut e = 127 - 14;
            while (m & 0x0400) == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x03ff) << 13)
        }
    } else if exp == 31 {
        sign | 0x7f80_0000 | (frac << 13)
    } else {
        sign | ((u32::from(exp) + (127 - 15)) << 23) | (frac << 13)
    };
    f32::from_bits(bits)
}

#[derive(Clone)]
struct ConvLayerWeights {
    in_c: usize,
    out_c: usize,
    /// Packed f16 pairs in layout `[oc, ky, kx, ic_pair]`.
    weights_f16_packed: Vec<u32>,
    /// `[bias[0..out_c]]` or `[bias[0..out_c], beta[0..out_c]]` when `has_beta`.
    bias_and_beta: Vec<f32>,
}

#[derive(Clone)]
struct LastConvWeights {
    in_c: usize,
    /// Packed f16 pairs in layout `[ct_oc (0..20), ky (0..4), kx (0..4), ic_pair]`.
    weights_f16_packed: Vec<u32>,
    bias: Vec<f32>,
}

#[derive(Clone)]
struct BlockWeights {
    conv0_0: ConvLayerWeights,
    conv0_1: ConvLayerWeights,
    res: Vec<ConvLayerWeights>,
    lastconv: LastConvWeights,
}

#[derive(Clone)]
pub struct Rife46Weights {
    blocks: Vec<BlockWeights>,
}

fn pack_conv3x3_f32(w: &[f32], out_c: usize, in_c: usize) -> Vec<u32> {
    let pairs = in_c.div_ceil(2);
    let mut packed = Vec::with_capacity(out_c * 9 * pairs);
    for oc in 0..out_c {
        for ky in 0..3 {
            for kx in 0..3 {
                for p in 0..pairs {
                    let ic0 = p * 2;
                    let v0 = w[((oc * in_c + ic0) * 3 + ky) * 3 + kx];
                    let v1 = if ic0 + 1 < in_c {
                        w[((oc * in_c + ic0 + 1) * 3 + ky) * 3 + kx]
                    } else {
                        0.0
                    };
                    let h0 = u32::from(f32_to_f16(v0));
                    let h1 = u32::from(f32_to_f16(v1));
                    packed.push(h0 | (h1 << 16));
                }
            }
        }
    }
    packed
}

fn pack_conv_transpose_f32(w: &[f32], in_c: usize) -> Vec<u32> {
    let pairs = in_c / 2;
    let mut packed = Vec::with_capacity(LAST_CONV_USED_OUT * 16 * pairs);
    for ct_oc in 0..LAST_CONV_USED_OUT {
        for ky in 0..4 {
            for kx in 0..4 {
                for p in 0..pairs {
                    let ic0 = p * 2;
                    let v0 = w[((ic0 * LAST_CONV_OUT + ct_oc) * 4 + ky) * 4 + kx];
                    let v1 = w[(((ic0 + 1) * LAST_CONV_OUT + ct_oc) * 4 + ky) * 4 + kx];
                    let h0 = u32::from(f32_to_f16(v0));
                    let h1 = u32::from(f32_to_f16(v1));
                    packed.push(h0 | (h1 << 16));
                }
            }
        }
    }
    packed
}

fn parse_stored_zip(data: &[u8]) -> Result<HashMap<String, &[u8]>, String> {
    if data.len() < 22 {
        return Err("zip file too small".into());
    }
    // Locate End of Central Directory (EOCD) signature 0x06054b50.
    let search_start = data.len().saturating_sub(65557);
    let eocd_pos = (search_start..=data.len() - 22)
        .rev()
        .find(|&p| {
            u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) == 0x0605_4b50
        })
        .ok_or("missing zip EOCD")?;

    let entry_count = u16::from_le_bytes([data[eocd_pos + 10], data[eocd_pos + 11]]) as usize;
    let cd_size = u32::from_le_bytes([
        data[eocd_pos + 12],
        data[eocd_pos + 13],
        data[eocd_pos + 14],
        data[eocd_pos + 15],
    ]) as usize;
    let cd_offset = u32::from_le_bytes([
        data[eocd_pos + 16],
        data[eocd_pos + 17],
        data[eocd_pos + 18],
        data[eocd_pos + 19],
    ]) as usize;

    let cd_end = cd_offset
        .checked_add(cd_size)
        .ok_or("zip central directory overflow")?;
    if cd_end > data.len() {
        return Err("truncated zip central directory".into());
    }

    let mut entries = HashMap::with_capacity(entry_count);
    let mut o = cd_offset;
    for _ in 0..entry_count {
        if o + 46 > cd_end {
            return Err("truncated zip central directory entry".into());
        }
        let sig = u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
        if sig != 0x0201_4b50 {
            return Err(format!("unexpected zip CD signature {sig:#x}"));
        }
        let method = u16::from_le_bytes([data[o + 10], data[o + 11]]);
        if method != 0 {
            return Err(format!("unsupported zip compression method {method}"));
        }
        let comp_size =
            u32::from_le_bytes([data[o + 20], data[o + 21], data[o + 22], data[o + 23]]) as usize;
        let name_len = u16::from_le_bytes([data[o + 28], data[o + 29]]) as usize;
        let extra_len = u16::from_le_bytes([data[o + 30], data[o + 31]]) as usize;
        let comment_len = u16::from_le_bytes([data[o + 32], data[o + 33]]) as usize;
        let local_off =
            u32::from_le_bytes([data[o + 42], data[o + 43], data[o + 44], data[o + 45]]) as usize;

        let name_start = o + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or("zip name overflow")?;
        if name_end > cd_end {
            return Err("truncated zip entry name".into());
        }
        let name = std::str::from_utf8(&data[name_start..name_end])
            .map_err(|_| "non-utf8 zip entry name")?;

        if local_off + 30 > data.len() {
            return Err("truncated zip local header".into());
        }
        let lsig = u32::from_le_bytes([
            data[local_off],
            data[local_off + 1],
            data[local_off + 2],
            data[local_off + 3],
        ]);
        if lsig != 0x0403_4b50 {
            return Err(format!("unexpected zip local signature {lsig:#x}"));
        }
        let l_name_len = u16::from_le_bytes([data[local_off + 26], data[local_off + 27]]) as usize;
        let l_extra_len = u16::from_le_bytes([data[local_off + 28], data[local_off + 29]]) as usize;
        let data_start = local_off
            .checked_add(30)
            .and_then(|v| v.checked_add(l_name_len))
            .and_then(|v| v.checked_add(l_extra_len))
            .ok_or("zip local header overflow")?;
        let data_end = data_start
            .checked_add(comp_size)
            .ok_or("zip data overflow")?;
        if data_end > data.len() {
            return Err("truncated zip entry data".into());
        }
        entries.insert(name.to_string(), &data[data_start..data_end]);
        o = name_end
            .checked_add(extra_len)
            .and_then(|v| v.checked_add(comment_len))
            .ok_or("zip CD advance overflow")?;
    }
    Ok(entries)
}

/// Extracts `(tensor_name -> storage_id)` from PyTorch's `archive/data.pkl` without executing any
/// pickle opcodes.
fn parse_pytorch_pickle_index(pkl: &[u8]) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    let mut cur_tensor: Option<String> = None;
    let mut i = 0usize;
    while i < pkl.len() {
        let op = pkl[i];
        i += 1;
        match op {
            // PROTO / BININT1 / BINGET / BINPUT
            0x80 | 0x4b | 0x68 | 0x71 => {
                i = i.checked_add(1).ok_or("pickle overflow")?;
            }
            // BININT2
            0x4d => {
                i = i.checked_add(2).ok_or("pickle overflow")?;
            }
            // BININT / LONG_BINGET / LONG_BINPUT
            0x4a | 0x6a | 0x72 => {
                i = i.checked_add(4).ok_or("pickle overflow")?;
            }
            // BINUNICODE (0x58): u32 len + utf8 bytes
            0x58 => {
                if i + 4 > pkl.len() {
                    return Err("truncated BINUNICODE".into());
                }
                let len = u32::from_le_bytes([pkl[i], pkl[i + 1], pkl[i + 2], pkl[i + 3]]) as usize;
                i += 4;
                let end = i.checked_add(len).ok_or("BINUNICODE overflow")?;
                if end > pkl.len() {
                    return Err("truncated BINUNICODE payload".into());
                }
                let s = std::str::from_utf8(&pkl[i..end]).map_err(|_| "invalid utf8 in pickle")?;
                i = end;
                if s.starts_with("module.") {
                    cur_tensor = Some(s.to_string());
                } else if !s.is_empty()
                    && s.bytes().all(|b| b.is_ascii_digit())
                    && let Some(name) = cur_tensor.take()
                {
                    map.insert(name, s.to_string());
                }
            }
            // SHORT_BINUNICODE (0x8c): u8 len + utf8 bytes
            0x8c => {
                if i >= pkl.len() {
                    return Err("truncated SHORT_BINUNICODE".into());
                }
                let len = pkl[i] as usize;
                i += 1;
                let end = i.checked_add(len).ok_or("SHORT_BINUNICODE overflow")?;
                if end > pkl.len() {
                    return Err("truncated SHORT_BINUNICODE payload".into());
                }
                let s = std::str::from_utf8(&pkl[i..end]).map_err(|_| "invalid utf8 in pickle")?;
                i = end;
                if s.starts_with("module.") {
                    cur_tensor = Some(s.to_string());
                } else if !s.is_empty()
                    && s.bytes().all(|b| b.is_ascii_digit())
                    && let Some(name) = cur_tensor.take()
                {
                    map.insert(name, s.to_string());
                }
            }
            // GLOBAL (0x63): two newline-terminated ASCII lines
            0x63 => {
                for _ in 0..2 {
                    let nl = pkl[i..]
                        .iter()
                        .position(|&b| b == b'\n')
                        .ok_or("unterminated GLOBAL")?;
                    i += nl + 1;
                }
            }
            // STOP
            0x2e => break,
            // Single-byte opcodes (MARK, EMPTY_TUPLE, TUPLE, TUPLE1/2/3, REDUCE, BINPERSID,
            // NEWTRUE, NEWFALSE, NONE, SETITEMS, BUILD, etc.)
            0x28 | 0x29 | 0x74 | 0x85 | 0x86 | 0x87 | 0x52 | 0x51 | 0x88 | 0x89 | 0x4e | 0x75
            | 0x62 | 0x7d | 0x5d | 0x73 | 0x61 | 0x65 => {}
            other => {
                return Err(format!("unsupported pickle opcode {other:#x} at {}", i - 1));
            }
        }
    }
    Ok(map)
}

fn read_f32_slice(bytes: &[u8], expected: usize, name: &str) -> Result<Vec<f32>, String> {
    if bytes.len() != expected * 4 {
        return Err(format!(
            "tensor {name}: expected {} bytes, got {}",
            expected * 4,
            bytes.len()
        ));
    }
    let mut out = Vec::with_capacity(expected);
    for chunk in bytes.as_chunks::<4>().0 {
        out.push(f32::from_le_bytes(*chunk));
    }
    Ok(out)
}

impl Rife46Weights {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.starts_with(M2RW_MAGIC) {
            Self::from_m2rw(bytes)
        } else {
            Self::from_pytorch_pkl(bytes)
        }
    }

    pub fn from_pytorch_pkl(bytes: &[u8]) -> Result<Self, String> {
        let digest = sha256_hex(bytes);
        if digest != RIFE46_PKL_SHA256 {
            return Err(format!(
                "RIFE v4.6 weights SHA-256 mismatch: expected {RIFE46_PKL_SHA256}, got {digest}"
            ));
        }
        let zip = parse_stored_zip(bytes)?;
        let pkl = zip
            .get("archive/data.pkl")
            .ok_or("missing archive/data.pkl in weights")?;
        let index = parse_pytorch_pickle_index(pkl)?;

        let get_tensor = |name: &str, expected: usize| -> Result<Vec<f32>, String> {
            let sid = index
                .get(name)
                .ok_or_else(|| format!("missing tensor {name} in weights"))?;
            let path = format!("archive/data/{sid}");
            let raw = zip
                .get(&path)
                .ok_or_else(|| format!("missing storage {path} for {name}"))?;
            read_f32_slice(raw, expected, name)
        };

        let mut blocks = Vec::with_capacity(4);
        for (bi, &(in_planes, c, _)) in BLOCK_SPECS.iter().enumerate() {
            let w0 = get_tensor(
                &format!("module.block{bi}.conv0.0.0.weight"),
                (c / 2) * in_planes * 9,
            )?;
            let b0 = get_tensor(&format!("module.block{bi}.conv0.0.0.bias"), c / 2)?;
            let conv0_0 = ConvLayerWeights {
                in_c: in_planes,
                out_c: c / 2,
                weights_f16_packed: pack_conv3x3_f32(&w0, c / 2, in_planes),
                bias_and_beta: b0,
            };

            let w1 = get_tensor(
                &format!("module.block{bi}.conv0.1.0.weight"),
                c * (c / 2) * 9,
            )?;
            let b1 = get_tensor(&format!("module.block{bi}.conv0.1.0.bias"), c)?;
            let conv0_1 = ConvLayerWeights {
                in_c: c / 2,
                out_c: c,
                weights_f16_packed: pack_conv3x3_f32(&w1, c, c / 2),
                bias_and_beta: b1,
            };

            let mut res = Vec::with_capacity(RES_BLOCKS);
            for ri in 0..RES_BLOCKS {
                let wr = get_tensor(
                    &format!("module.block{bi}.convblock.{ri}.conv.weight"),
                    c * c * 9,
                )?;
                let mut br = get_tensor(&format!("module.block{bi}.convblock.{ri}.conv.bias"), c)?;
                let beta = get_tensor(&format!("module.block{bi}.convblock.{ri}.beta"), c)?;
                br.extend_from_slice(&beta);
                res.push(ConvLayerWeights {
                    in_c: c,
                    out_c: c,
                    weights_f16_packed: pack_conv3x3_f32(&wr, c, c),
                    bias_and_beta: br,
                });
            }

            let wl = get_tensor(
                &format!("module.block{bi}.lastconv.0.weight"),
                c * LAST_CONV_OUT * 16,
            )?;
            let bl = get_tensor(&format!("module.block{bi}.lastconv.0.bias"), LAST_CONV_OUT)?;
            let lastconv = LastConvWeights {
                in_c: c,
                weights_f16_packed: pack_conv_transpose_f32(&wl, c),
                bias: bl[..LAST_CONV_USED_OUT].to_vec(),
            };

            blocks.push(BlockWeights {
                conv0_0,
                conv0_1,
                res,
                lastconv,
            });
        }

        Ok(Self { blocks })
    }

    pub fn to_m2rw(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(11 * 1024 * 1024);
        let write_conv = |buf: &mut Vec<u8>, l: &ConvLayerWeights| {
            for &u in &l.weights_f16_packed {
                buf.extend_from_slice(&u.to_le_bytes());
            }
            for &f in &l.bias_and_beta {
                buf.extend_from_slice(&f.to_le_bytes());
            }
        };
        for b in &self.blocks {
            write_conv(&mut payload, &b.conv0_0);
            write_conv(&mut payload, &b.conv0_1);
            for r in &b.res {
                write_conv(&mut payload, r);
            }
            for &u in &b.lastconv.weights_f16_packed {
                payload.extend_from_slice(&u.to_le_bytes());
            }
            for &f in &b.lastconv.bias {
                payload.extend_from_slice(&f.to_le_bytes());
            }
        }
        let payload_hash = sha256(&payload);
        let mut out = Vec::with_capacity(40 + payload.len());
        out.extend_from_slice(M2RW_MAGIC);
        out.extend_from_slice(&M2RW_VERSION.to_le_bytes());
        out.extend_from_slice(&M2RW_FLAGS.to_le_bytes());
        out.extend_from_slice(&payload_hash);
        out.extend_from_slice(&payload);
        out
    }

    pub fn from_m2rw(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 40 || &bytes[0..4] != M2RW_MAGIC {
            return Err("not an M2RW weights file".into());
        }
        let file_digest = sha256_hex(bytes);
        if file_digest != RIFE46_M2RW_SHA256 {
            return Err(format!(
                "M2RW weights SHA-256 mismatch: expected {RIFE46_M2RW_SHA256}, got {file_digest}"
            ));
        }
        let ver = u16::from_le_bytes([bytes[4], bytes[5]]);
        let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        if ver != M2RW_VERSION || flags != M2RW_FLAGS {
            return Err(format!("unsupported M2RW version {ver} flags {flags}"));
        }
        let expected_hash = &bytes[8..40];
        let payload = &bytes[40..];
        let actual_hash = sha256(payload);
        if actual_hash != expected_hash {
            return Err("M2RW weights SHA-256 checksum mismatch".into());
        }

        let mut o = 0usize;
        let read_u32s = |o: &mut usize, n: usize| -> Result<Vec<u32>, String> {
            let end = o.checked_add(n * 4).ok_or("M2RW overflow")?;
            let slice = payload.get(*o..end).ok_or("truncated M2RW weights")?;
            *o = end;
            Ok(slice
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect())
        };
        let read_f32s = |o: &mut usize, n: usize| -> Result<Vec<f32>, String> {
            let end = o.checked_add(n * 4).ok_or("M2RW overflow")?;
            let slice = payload.get(*o..end).ok_or("truncated M2RW biases")?;
            *o = end;
            Ok(slice
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect())
        };

        let mut blocks = Vec::with_capacity(4);
        for &(in_planes, c, _) in &BLOCK_SPECS {
            let conv0_0 = ConvLayerWeights {
                in_c: in_planes,
                out_c: c / 2,
                weights_f16_packed: read_u32s(&mut o, (c / 2) * 9 * in_planes.div_ceil(2))?,
                bias_and_beta: read_f32s(&mut o, c / 2)?,
            };
            let conv0_1 = ConvLayerWeights {
                in_c: c / 2,
                out_c: c,
                weights_f16_packed: read_u32s(&mut o, c * 9 * (c / 2).div_ceil(2))?,
                bias_and_beta: read_f32s(&mut o, c)?,
            };
            let mut res = Vec::with_capacity(RES_BLOCKS);
            for _ in 0..RES_BLOCKS {
                res.push(ConvLayerWeights {
                    in_c: c,
                    out_c: c,
                    weights_f16_packed: read_u32s(&mut o, c * 9 * c.div_ceil(2))?,
                    bias_and_beta: read_f32s(&mut o, c * 2)?,
                });
            }
            let lastconv = LastConvWeights {
                in_c: c,
                weights_f16_packed: read_u32s(&mut o, LAST_CONV_USED_OUT * 16 * (c / 2))?,
                bias: read_f32s(&mut o, LAST_CONV_USED_OUT)?,
            };
            blocks.push(BlockWeights {
                conv0_0,
                conv0_1,
                res,
                lastconv,
            });
        }
        if o != payload.len() {
            return Err(format!(
                "M2RW trailing bytes: consumed {o}, payload {}",
                payload.len()
            ));
        }
        Ok(Self { blocks })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DownsampleParams {
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    scale: f32,
    timestep: f32,
    has_flow: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ConvParams {
    in_c: u32,
    in_c_pairs: u32,
    out_c: u32,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    stride: u32,
    has_res: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LastConvParams {
    in_c: u32,
    in_c_pairs: u32,
    fw: u32,
    fh: u32,
    out_w: u32,
    out_h: u32,
    _p0: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UpsampleParams {
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    scale: f32,
    accum: u32,
    _p0: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WarpParams {
    w: u32,
    h: u32,
    _p0: u32,
    _p1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ExtractParams {
    pw: u32,
    ph: u32,
    w: u32,
    h: u32,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct ConvGpu {
    in_c: u32,
    in_c_pairs: u32,
    out_c: u32,
    weights: wgpu::Buffer,
    bias: wgpu::Buffer,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct LastConvGpu {
    in_c: u32,
    in_c_pairs: u32,
    weights: wgpu::Buffer,
    bias: wgpu::Buffer,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct BlockGpu {
    scale: f32,
    conv0_0: ConvGpu,
    conv0_1: ConvGpu,
    res: Vec<ConvGpu>,
    lastconv: LastConvGpu,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct ScratchBuffers {
    pw: u32,
    ph: u32,
    img0: wgpu::Buffer,
    img1: wgpu::Buffer,
    w0: wgpu::Buffer,
    w1: wgpu::Buffer,
    flow: wgpu::Buffer,
    mask: wgpu::Buffer,
    x_in: wgpu::Buffer,
    conv0_0_out: wgpu::Buffer,
    feat_a: wgpu::Buffer,
    feat_b: wgpu::Buffer,
    ps_out: wgpu::Buffer,
    extract_gpu: wgpu::Buffer,
    extract_readback: wgpu::Buffer,
}

pub struct Rife46Output {
    pub width: u32,
    pub height: u32,
    pub flow: Vec<[f32; 4]>,
    pub mask: Vec<f32>,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct Rife46Gpu {
    ds_pipeline: wgpu::ComputePipeline,
    ds_bgl: wgpu::BindGroupLayout,
    conv_pipeline: wgpu::ComputePipeline,
    conv_bgl: wgpu::BindGroupLayout,
    lc_pipeline: wgpu::ComputePipeline,
    lc_bgl: wgpu::BindGroupLayout,
    up_pipeline: wgpu::ComputePipeline,
    up_bgl: wgpu::BindGroupLayout,
    warp_pipeline: wgpu::ComputePipeline,
    warp_bgl: wgpu::BindGroupLayout,
    ex_pipeline: wgpu::ComputePipeline,
    ex_bgl: wgpu::BindGroupLayout,
    uniform_buf: wgpu::Buffer,
    uniform_stride: u32,
    blocks: Vec<BlockGpu>,
    scratch: Option<ScratchBuffers>,
}

const MAX_UNIFORM_SLOTS: usize = 64;

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, min_size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(min_size),
        },
        count: None,
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry_point: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(entry_point),
        entries,
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry_point),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&pl),
        module,
        entry_point: Some(entry_point),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    (pipeline, bgl)
}

fn upload_conv(device: &wgpu::Device, l: &ConvLayerWeights, label: &str) -> ConvGpu {
    let weights = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(&l.weights_f16_packed),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bias = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(&l.bias_and_beta),
        usage: wgpu::BufferUsages::STORAGE,
    });
    ConvGpu {
        in_c: l.in_c as u32,
        in_c_pairs: l.in_c.div_ceil(2) as u32,
        out_c: l.out_c as u32,
        weights,
        bias,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn storage_buf(device: &wgpu::Device, floats: usize, extra: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rife46_scratch"),
        size: (floats.max(1) * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | extra,
        mapped_at_creation: false,
    })
}

impl Rife46Gpu {
    pub fn new(device: &wgpu::Device, weights: &Rife46Weights) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rife46"),
            source: wgpu::ShaderSource::Wgsl(RIFE46_SHADER_SRC.into()),
        });

        let (ds_pipeline, ds_bgl) = make_pipeline(
            device,
            &module,
            "downsample_concat",
            &[
                uniform_entry(0, std::mem::size_of::<DownsampleParams>() as u64),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, true),
                storage_entry(4, true),
                storage_entry(5, false),
            ],
        );
        let (conv_pipeline, conv_bgl) = make_pipeline(
            device,
            &module,
            "conv2d_3x3",
            &[
                uniform_entry(0, std::mem::size_of::<ConvParams>() as u64),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, true),
                storage_entry(4, false),
            ],
        );
        let (lc_pipeline, lc_bgl) = make_pipeline(
            device,
            &module,
            "conv_transpose_ps",
            &[
                uniform_entry(0, std::mem::size_of::<LastConvParams>() as u64),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, true),
                storage_entry(4, false),
            ],
        );
        let (up_pipeline, up_bgl) = make_pipeline(
            device,
            &module,
            "upsample_accum",
            &[
                uniform_entry(0, std::mem::size_of::<UpsampleParams>() as u64),
                storage_entry(1, true),
                storage_entry(2, false),
                storage_entry(3, false),
            ],
        );
        let (warp_pipeline, warp_bgl) = make_pipeline(
            device,
            &module,
            "warp_bilinear",
            &[
                uniform_entry(0, std::mem::size_of::<WarpParams>() as u64),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, true),
                storage_entry(4, false),
                storage_entry(5, false),
            ],
        );
        let (ex_pipeline, ex_bgl) = make_pipeline(
            device,
            &module,
            "extract_output",
            &[
                uniform_entry(0, std::mem::size_of::<ExtractParams>() as u64),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
            ],
        );

        let uniform_stride = device.limits().min_uniform_buffer_offset_alignment.max(256);
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rife46_uniforms"),
            size: u64::from(uniform_stride) * MAX_UNIFORM_SLOTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let blocks = weights
            .blocks
            .iter()
            .enumerate()
            .map(|(bi, b)| {
                let (_, _, scale) = BLOCK_SPECS[bi];
                let conv0_0 = upload_conv(device, &b.conv0_0, "rife_conv0_0");
                let conv0_1 = upload_conv(device, &b.conv0_1, "rife_conv0_1");
                let res = b
                    .res
                    .iter()
                    .map(|r| upload_conv(device, r, "rife_res"))
                    .collect();
                let lastconv = LastConvGpu {
                    in_c: b.lastconv.in_c as u32,
                    in_c_pairs: (b.lastconv.in_c / 2) as u32,
                    weights: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("rife_lastconv_w"),
                        contents: bytemuck::cast_slice(&b.lastconv.weights_f16_packed),
                        usage: wgpu::BufferUsages::STORAGE,
                    }),
                    bias: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("rife_lastconv_b"),
                        contents: bytemuck::cast_slice(&b.lastconv.bias),
                        usage: wgpu::BufferUsages::STORAGE,
                    }),
                };
                BlockGpu {
                    scale,
                    conv0_0,
                    conv0_1,
                    res,
                    lastconv,
                }
            })
            .collect();

        Self {
            ds_pipeline,
            ds_bgl,
            conv_pipeline,
            conv_bgl,
            lc_pipeline,
            lc_bgl,
            up_pipeline,
            up_bgl,
            warp_pipeline,
            warp_bgl,
            ex_pipeline,
            ex_bgl,
            uniform_buf,
            uniform_stride,
            blocks,
            scratch: None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ensure_scratch(&mut self, device: &wgpu::Device, pw: u32, ph: u32) {
        if let Some(s) = &self.scratch
            && s.pw >= pw
            && s.ph >= ph
        {
            return;
        }
        let phw = (pw as usize) * (ph as usize);
        // Worst-case intermediate sizes across blocks 0..3:
        // x_in: block3 has 12 * ph * pw
        // conv0_0_out: block3 has 32 * (ph/2) * (pw/2) = 8 * phw
        // feat_a/b: block3 has 64 * (ph/4) * (pw/4) = 4 * phw
        // ps_out: block3 has 5 * ph * pw
        let extract_floats = 5 * phw;
        self.scratch = Some(ScratchBuffers {
            pw,
            ph,
            img0: storage_buf(device, 3 * phw, wgpu::BufferUsages::COPY_DST),
            img1: storage_buf(device, 3 * phw, wgpu::BufferUsages::COPY_DST),
            w0: storage_buf(device, 3 * phw, wgpu::BufferUsages::empty()),
            w1: storage_buf(device, 3 * phw, wgpu::BufferUsages::empty()),
            flow: storage_buf(device, 4 * phw, wgpu::BufferUsages::empty()),
            mask: storage_buf(device, phw, wgpu::BufferUsages::empty()),
            x_in: storage_buf(device, 12 * phw, wgpu::BufferUsages::empty()),
            conv0_0_out: storage_buf(device, 8 * phw, wgpu::BufferUsages::empty()),
            feat_a: storage_buf(device, 4 * phw, wgpu::BufferUsages::empty()),
            feat_b: storage_buf(device, 4 * phw, wgpu::BufferUsages::empty()),
            ps_out: storage_buf(device, 5 * phw, wgpu::BufferUsages::empty()),
            extract_gpu: storage_buf(device, extract_floats, wgpu::BufferUsages::COPY_SRC),
            extract_readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rife46_readback"),
                size: (extract_floats * 4) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        });
    }

    /// Runs RIFE v4.6 inference at `t = 0.5` on two RGB images (`height * width * 3` f32 in
    /// `0..=1`, row-major interleaved RGB).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn infer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img0_rgb: &[f32],
        img1_rgb: &[f32],
        width: u32,
        height: u32,
    ) -> Result<Rife46Output, String> {
        let w = width as usize;
        let h = height as usize;
        if img0_rgb.len() != w * h * 3 || img1_rgb.len() != w * h * 3 {
            return Err("RIFE input image size mismatch".into());
        }
        let pw = width.div_ceil(32) * 32;
        let ph = height.div_ceil(32) * 32;
        self.ensure_scratch(device, pw, ph);

        let pad_chw = |rgb: &[f32]| -> Vec<f32> {
            let pwu = pw as usize;
            let phu = ph as usize;
            let phw = pwu * phu;
            let mut out = vec![0.0f32; 3 * phw];
            for y in 0..phu {
                let sy = y.min(h - 1);
                for x in 0..pwu {
                    let sx = x.min(w - 1);
                    let src = (sy * w + sx) * 3;
                    let dst = y * pwu + x;
                    out[dst] = rgb[src].clamp(0.0, 1.0);
                    out[phw + dst] = rgb[src + 1].clamp(0.0, 1.0);
                    out[2 * phw + dst] = rgb[src + 2].clamp(0.0, 1.0);
                }
            }
            out
        };

        let s = self.scratch.as_ref().unwrap();
        queue.write_buffer(&s.img0, 0, bytemuck::cast_slice(&pad_chw(img0_rgb)));
        queue.write_buffer(&s.img1, 0, bytemuck::cast_slice(&pad_chw(img1_rgb)));

        let stride = self.uniform_stride as usize;
        let mut uniform_stage = vec![0u8; MAX_UNIFORM_SLOTS * stride];
        let mut next_slot = 0u32;
        let mut push_uniform = |bytes: &[u8]| -> u32 {
            let off = (next_slot as usize) * stride;
            uniform_stage[off..off + bytes.len()].copy_from_slice(bytes);
            let slot_off = next_slot * self.uniform_stride;
            next_slot += 1;
            slot_off
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rife46_infer"),
        });

        let mut pass_ops: Vec<PassOp> = Vec::with_capacity(60);

        for (bi, block) in self.blocks.iter().enumerate() {
            let sc = block.scale as u32;
            let ih = ph / sc;
            let iw = pw / sc;
            let h2 = ih / 2;
            let w2 = iw / 2;
            let fh = ih / 4;
            let fw = iw / 4;

            let ds_off = push_uniform(bytemuck::bytes_of(&DownsampleParams {
                in_w: pw,
                in_h: ph,
                out_w: iw,
                out_h: ih,
                scale: block.scale,
                timestep: 0.5,
                has_flow: u32::from(bi > 0),
                _pad: 0,
            }));
            pass_ops.push(PassOp::Downsample {
                bi,
                off: ds_off,
                w: iw,
                h: ih,
            });

            let c0_off = push_uniform(bytemuck::bytes_of(&ConvParams {
                in_c: block.conv0_0.in_c,
                in_c_pairs: block.conv0_0.in_c_pairs,
                out_c: block.conv0_0.out_c,
                in_w: iw,
                in_h: ih,
                out_w: w2,
                out_h: h2,
                stride: 2,
                has_res: 0,
                _p0: 0,
                _p1: 0,
                _p2: 0,
            }));
            pass_ops.push(PassOp::Conv {
                bi,
                kind: ConvKind::Conv0_0,
                off: c0_off,
                w: w2,
                h: h2,
                oc: block.conv0_0.out_c,
            });

            let c1_off = push_uniform(bytemuck::bytes_of(&ConvParams {
                in_c: block.conv0_1.in_c,
                in_c_pairs: block.conv0_1.in_c_pairs,
                out_c: block.conv0_1.out_c,
                in_w: w2,
                in_h: h2,
                out_w: fw,
                out_h: fh,
                stride: 2,
                has_res: 0,
                _p0: 0,
                _p1: 0,
                _p2: 0,
            }));
            pass_ops.push(PassOp::Conv {
                bi,
                kind: ConvKind::Conv0_1,
                off: c1_off,
                w: fw,
                h: fh,
                oc: block.conv0_1.out_c,
            });

            for (ri, r) in block.res.iter().enumerate() {
                let r_off = push_uniform(bytemuck::bytes_of(&ConvParams {
                    in_c: r.in_c,
                    in_c_pairs: r.in_c_pairs,
                    out_c: r.out_c,
                    in_w: fw,
                    in_h: fh,
                    out_w: fw,
                    out_h: fh,
                    stride: 1,
                    has_res: 1,
                    _p0: 0,
                    _p1: 0,
                    _p2: 0,
                }));
                pass_ops.push(PassOp::Conv {
                    bi,
                    kind: ConvKind::Res(ri),
                    off: r_off,
                    w: fw,
                    h: fh,
                    oc: r.out_c,
                });
            }

            let lc_off = push_uniform(bytemuck::bytes_of(&LastConvParams {
                in_c: block.lastconv.in_c,
                in_c_pairs: block.lastconv.in_c_pairs,
                fw,
                fh,
                out_w: iw,
                out_h: ih,
                _p0: 0,
                _p1: 0,
            }));
            pass_ops.push(PassOp::LastConv {
                bi,
                off: lc_off,
                w: iw,
                h: ih,
            });

            let up_off = push_uniform(bytemuck::bytes_of(&UpsampleParams {
                in_w: iw,
                in_h: ih,
                out_w: pw,
                out_h: ph,
                scale: block.scale,
                accum: u32::from(bi > 0),
                _p0: 0,
                _p1: 0,
            }));
            pass_ops.push(PassOp::Upsample {
                off: up_off,
                w: pw,
                h: ph,
            });

            if bi < 3 {
                let wp_off = push_uniform(bytemuck::bytes_of(&WarpParams {
                    w: pw,
                    h: ph,
                    _p0: 0,
                    _p1: 0,
                }));
                pass_ops.push(PassOp::Warp {
                    off: wp_off,
                    w: pw,
                    h: ph,
                });
            }
        }

        let ex_off = push_uniform(bytemuck::bytes_of(&ExtractParams {
            pw,
            ph,
            w: width,
            h: height,
        }));
        pass_ops.push(PassOp::Extract {
            off: ex_off,
            w: width,
            h: height,
        });

        queue.write_buffer(
            &self.uniform_buf,
            0,
            &uniform_stage[..(next_slot as usize) * stride],
        );

        let ub_binding = |size: usize| {
            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.uniform_buf,
                offset: 0,
                size: wgpu::BufferSize::new(size as u64),
            })
        };

        let ds_bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rife_ds0"),
            layout: &self.ds_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub_binding(std::mem::size_of::<DownsampleParams>()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: s.img0.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: s.img1.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: s.flow.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: s.mask.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: s.x_in.as_entire_binding(),
                },
            ],
        });
        let ds_bgw = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rife_dsw"),
            layout: &self.ds_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub_binding(std::mem::size_of::<DownsampleParams>()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: s.w0.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: s.w1.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: s.flow.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: s.mask.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: s.x_in.as_entire_binding(),
                },
            ],
        });
        let up_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rife_up"),
            layout: &self.up_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub_binding(std::mem::size_of::<UpsampleParams>()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: s.ps_out.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: s.flow.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: s.mask.as_entire_binding(),
                },
            ],
        });
        let warp_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rife_warp"),
            layout: &self.warp_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub_binding(std::mem::size_of::<WarpParams>()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: s.img0.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: s.img1.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: s.flow.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: s.w0.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: s.w1.as_entire_binding(),
                },
            ],
        });
        let ex_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rife_ex"),
            layout: &self.ex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub_binding(std::mem::size_of::<ExtractParams>()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: s.flow.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: s.mask.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: s.extract_gpu.as_entire_binding(),
                },
            ],
        });

        let make_conv_bg = |src: &wgpu::Buffer, layer: &ConvGpu, dst: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rife_conv"),
                layout: &self.conv_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ub_binding(std::mem::size_of::<ConvParams>()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: src.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: layer.weights.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: layer.bias.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: dst.as_entire_binding(),
                    },
                ],
            })
        };

        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("rife46_pass"),
            timestamp_writes: None,
        });
        for op in pass_ops {
            match op {
                PassOp::Downsample { bi, off, w, h } => {
                    cpass.set_pipeline(&self.ds_pipeline);
                    cpass.set_bind_group(0, if bi == 0 { &ds_bg0 } else { &ds_bgw }, &[off]);
                    cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                }
                PassOp::Conv {
                    bi,
                    kind,
                    off,
                    w,
                    h,
                    oc,
                } => {
                    let b = &self.blocks[bi];
                    let bg = match kind {
                        ConvKind::Conv0_0 => make_conv_bg(&s.x_in, &b.conv0_0, &s.conv0_0_out),
                        ConvKind::Conv0_1 => make_conv_bg(&s.conv0_0_out, &b.conv0_1, &s.feat_a),
                        ConvKind::Res(ri) => {
                            if ri % 2 == 0 {
                                make_conv_bg(&s.feat_a, &b.res[ri], &s.feat_b)
                            } else {
                                make_conv_bg(&s.feat_b, &b.res[ri], &s.feat_a)
                            }
                        }
                    };
                    cpass.set_pipeline(&self.conv_pipeline);
                    cpass.set_bind_group(0, &bg, &[off]);
                    cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), oc);
                }
                PassOp::LastConv { bi, off, w, h } => {
                    let lc = &self.blocks[bi].lastconv;
                    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("rife_lc"),
                        layout: &self.lc_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: ub_binding(std::mem::size_of::<LastConvParams>()),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: s.feat_a.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: lc.weights.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: lc.bias.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: s.ps_out.as_entire_binding(),
                            },
                        ],
                    });
                    cpass.set_pipeline(&self.lc_pipeline);
                    cpass.set_bind_group(0, &bg, &[off]);
                    cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 5);
                }
                PassOp::Upsample { off, w, h } => {
                    cpass.set_pipeline(&self.up_pipeline);
                    cpass.set_bind_group(0, &up_bg, &[off]);
                    cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                }
                PassOp::Warp { off, w, h } => {
                    cpass.set_pipeline(&self.warp_pipeline);
                    cpass.set_bind_group(0, &warp_bg, &[off]);
                    cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                }
                PassOp::Extract { off, w, h } => {
                    cpass.set_pipeline(&self.ex_pipeline);
                    cpass.set_bind_group(0, &ex_bg, &[off]);
                    cpass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                }
            }
        }
        drop(cpass);

        let out_bytes = (w * h * 5 * 4) as u64;
        encoder.copy_buffer_to_buffer(&s.extract_gpu, 0, &s.extract_readback, 0, out_bytes);
        queue.submit(std::iter::once(encoder.finish()));

        let slice = s.extract_readback.slice(0..out_bytes);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|_| "RIFE readback channel closed")?
            .map_err(|e| format!("RIFE readback map failed: {e:?}"))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|e| format!("RIFE get_mapped_range failed: {e:?}"))?;
        let floats: &[f32] = bytemuck::cast_slice(&mapped);
        let mut flow = Vec::with_capacity(w * h);
        let mut mask = Vec::with_capacity(w * h);
        for pix in floats.as_chunks::<5>().0 {
            flow.push([pix[0], pix[1], pix[2], pix[3]]);
            mask.push(pix[4]);
        }
        drop(mapped);
        s.extract_readback.unmap();

        Ok(Rife46Output {
            width,
            height,
            flow,
            mask,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum ConvKind {
    Conv0_0,
    Conv0_1,
    Res(usize),
}

#[cfg(not(target_arch = "wasm32"))]
enum PassOp {
    Downsample {
        bi: usize,
        off: u32,
        w: u32,
        h: u32,
    },
    Conv {
        bi: usize,
        kind: ConvKind,
        off: u32,
        w: u32,
        h: u32,
        oc: u32,
    },
    LastConv {
        bi: usize,
        off: u32,
        w: u32,
        h: u32,
    },
    Upsample {
        off: u32,
        w: u32,
        h: u32,
    },
    Warp {
        off: u32,
        w: u32,
        h: u32,
    },
    Extract {
        off: u32,
        w: u32,
        h: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rife46_wgsl_parses_cleanly() {
        let parsed = naga::front::wgsl::parse_str(RIFE46_SHADER_SRC);
        assert!(
            parsed.is_ok(),
            "rife46.wgsl parse failed: {:?}",
            parsed.err()
        );
    }

    #[test]
    fn sha256_matches_standard_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn f16_roundtrip_normal_subnormal_and_zero() {
        for &v in &[
            0.0f32, -0.0, 1.0, -1.0, 0.5, 0.25, -3.5, 65504.0, 1e-4, -1e-4,
        ] {
            let h = f32_to_f16(v);
            let r = f16_to_f32(h);
            assert!(
                (r - v).abs() <= v.abs() * 1e-3 + 1e-5,
                "v={v} h={h:#x} r={r}"
            );
        }
    }

    #[test]
    fn pkl_to_m2rw_roundtrip_when_weights_present() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let pkl_path = std::path::PathBuf::from(home).join(".cache/myth2-rife/flownet_v4.6.pkl");
        let Ok(pkl_bytes) = std::fs::read(&pkl_path) else {
            return;
        };
        let w = Rife46Weights::from_pytorch_pkl(&pkl_bytes).expect("parse pkl");
        let m2rw = w.to_m2rw();
        assert_eq!(sha256_hex(&m2rw), RIFE46_M2RW_SHA256);
        let w2 = Rife46Weights::from_m2rw(&m2rw).expect("parse m2rw");
        assert_eq!(w.blocks.len(), w2.blocks.len());
        assert_eq!(
            w.blocks[0].conv0_0.weights_f16_packed,
            w2.blocks[0].conv0_0.weights_f16_packed
        );
    }
}
