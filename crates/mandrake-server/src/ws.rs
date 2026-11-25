pub const MAX_PAYLOAD: usize = 65536;

const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub const OP_CONTINUATION: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xa;

pub const CLOSE_NORMAL: u16 = 1000;
pub const CLOSE_PROTOCOL: u16 = 1002;
pub const CLOSE_TOO_BIG: u16 = 1009;

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

pub fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn accept_key(key: &str) -> String {
    let mut s = key.trim().as_bytes().to_vec();
    s.extend_from_slice(GUID);
    base64(&sha1(&s))
}

pub fn encode(opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | opcode);
    let m = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(m | n as u8),
        n if n <= 0xffff => {
            out.push(m | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(m | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        None => out.extend_from_slice(payload),
        Some(k) => {
            out.extend_from_slice(&k);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ k[i & 3]));
        }
    }
    out
}

pub fn close_payload(code: u16) -> Vec<u8> {
    code.to_be_bytes().to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Data(Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    Close(Option<u16>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameError {
    pub code: u16,
    pub why: &'static str,
}

fn protocol(why: &'static str) -> FrameError {
    FrameError {
        code: CLOSE_PROTOCOL,
        why,
    }
}

#[derive(Debug)]
pub struct Decoder {
    buf: Vec<u8>,
    from_client: bool,
    fragmented: Option<u8>,
}

impl Decoder {
    pub fn new(from_client: bool) -> Decoder {
        Decoder {
            buf: Vec::new(),
            from_client,
            fragmented: None,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        loop {
            let Some((frame, used)) = self.parse()? else {
                return Ok(None);
            };
            self.buf.drain(..used);
            if let Some(f) = frame {
                return Ok(Some(f));
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn parse(&mut self) -> Result<Option<(Option<Frame>, usize)>, FrameError> {
        let b = &self.buf;
        if b.len() < 2 {
            return Ok(None);
        }
        let fin = b[0] & 0x80 != 0;
        if b[0] & 0x70 != 0 {
            return Err(protocol("a reserved bit"));
        }
        let opcode = b[0] & 0x0f;
        let masked = b[1] & 0x80 != 0;
        if masked != self.from_client {
            return Err(protocol(if masked {
                "a masked frame from the server"
            } else {
                "an unmasked frame from the client"
            }));
        }
        let (len, mut at) = match b[1] & 0x7f {
            126 => {
                if b.len() < 4 {
                    return Ok(None);
                }
                (u64::from(u16::from_be_bytes([b[2], b[3]])), 4)
            }
            127 => {
                if b.len() < 10 {
                    return Ok(None);
                }
                let mut n = [0u8; 8];
                n.copy_from_slice(&b[2..10]);
                (u64::from_be_bytes(n), 10)
            }
            n => (u64::from(n), 2),
        };
        let control = opcode & 0x8 != 0;
        if control && (len > 125 || !fin) {
            return Err(protocol("a control frame over 125 bytes or fragmented"));
        }
        if len > MAX_PAYLOAD as u64 {
            return Err(FrameError {
                code: CLOSE_TOO_BIG,
                why: "a frame over 64 KiB",
            });
        }
        let len = len as usize;
        let key = if masked {
            if b.len() < at + 4 {
                return Ok(None);
            }
            let k = [b[at], b[at + 1], b[at + 2], b[at + 3]];
            at += 4;
            Some(k)
        } else {
            None
        };
        if b.len() < at + len {
            return Ok(None);
        }
        let mut payload = b[at..at + len].to_vec();
        if let Some(k) = key {
            for (i, x) in payload.iter_mut().enumerate() {
                *x ^= k[i & 3];
            }
        }
        let used = at + len;
        let frame = match opcode {
            OP_PING => Some(Frame::Ping(payload)),
            OP_PONG => Some(Frame::Pong),
            OP_CLOSE => match payload.len() {
                0 => Some(Frame::Close(None)),
                1 => return Err(protocol("a 1-byte close")),
                _ => Some(Frame::Close(Some(u16::from_be_bytes([
                    payload[0], payload[1],
                ])))),
            },
            OP_TEXT | OP_BINARY => {
                if self.fragmented.is_some() {
                    return Err(protocol("a new message inside a fragmented one"));
                }
                if !fin {
                    self.fragmented = Some(opcode);
                }
                (opcode == OP_BINARY).then_some(Frame::Data(payload))
            }
            OP_CONTINUATION => {
                let Some(op) = self.fragmented else {
                    return Err(protocol("a continuation with no message"));
                };
                if fin {
                    self.fragmented = None;
                }
                (op == OP_BINARY).then_some(Frame::Data(payload))
            }
            _ => return Err(protocol("an unknown opcode")),
        };
        Ok(Some((frame, used)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha1_and_base64_match_the_standards_vectors() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    fn all(d: &mut Decoder) -> Result<Vec<Frame>, FrameError> {
        let mut v = Vec::new();
        while let Some(f) = d.next_frame()? {
            v.push(f);
        }
        Ok(v)
    }

    #[test]
    fn a_masked_frame_split_at_every_byte_comes_out_whole() {
        let payload: Vec<u8> = (0..300u32).map(|i| (i * 7) as u8).collect();
        let key = [0x37, 0xfa, 0x21, 0x3d];
        let bytes = encode(OP_BINARY, &payload, Some(key));
        assert_ne!(
            &bytes[8..],
            &payload[..],
            "the payload is masked on the wire"
        );
        for split in 0..bytes.len() {
            let mut d = Decoder::new(true);
            d.push(&bytes[..split]);
            assert_eq!(d.next_frame().unwrap(), None, "split {split}");
            d.push(&bytes[split..]);
            assert_eq!(all(&mut d).unwrap(), vec![Frame::Data(payload.clone())]);
            assert_eq!(d.pending(), 0);
        }
        for n in [0, 125, 126, 65535, 65536] {
            let p = vec![0xa5u8; n];
            let bytes = encode(OP_BINARY, &p, Some(key));
            let mut d = Decoder::new(true);
            let mut got = Vec::new();
            for b in &bytes {
                d.push(std::slice::from_ref(b));
                got.extend(all(&mut d).unwrap());
            }
            assert_eq!(got, vec![Frame::Data(p)], "length {n}");
        }
    }

    #[test]
    fn masking_is_required_one_way_and_refused_the_other() {
        let unmasked = encode(OP_BINARY, b"hi", None);
        let masked = encode(OP_BINARY, b"hi", Some([1, 2, 3, 4]));
        let mut server = Decoder::new(true);
        server.push(&unmasked);
        assert_eq!(server.next_frame().unwrap_err().code, CLOSE_PROTOCOL);
        let mut client = Decoder::new(false);
        client.push(&masked);
        assert_eq!(client.next_frame().unwrap_err().code, CLOSE_PROTOCOL);
        let mut client = Decoder::new(false);
        client.push(&unmasked);
        assert_eq!(
            client.next_frame().unwrap(),
            Some(Frame::Data(b"hi".to_vec()))
        );
    }

    fn frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut f = encode(opcode, payload, Some([9, 8, 7, 6]));
        if !fin {
            f[0] &= 0x7f;
        }
        f
    }

    #[test]
    fn a_fragmented_message_comes_out_in_order_with_controls_between() {
        let mut d = Decoder::new(true);
        d.push(&frame(false, OP_BINARY, b"ab"));
        d.push(&frame(true, OP_PING, b"p"));
        d.push(&frame(false, OP_CONTINUATION, b"cd"));
        d.push(&frame(true, OP_CONTINUATION, b"e"));
        d.push(&frame(true, OP_BINARY, b"f"));
        assert_eq!(
            all(&mut d).unwrap(),
            vec![
                Frame::Data(b"ab".to_vec()),
                Frame::Ping(b"p".to_vec()),
                Frame::Data(b"cd".to_vec()),
                Frame::Data(b"e".to_vec()),
                Frame::Data(b"f".to_vec()),
            ]
        );
        let mut d = Decoder::new(true);
        d.push(&frame(false, OP_TEXT, b"t1"));
        d.push(&frame(true, OP_CONTINUATION, b"t2"));
        d.push(&frame(true, OP_BINARY, b"b"));
        assert_eq!(all(&mut d).unwrap(), vec![Frame::Data(b"b".to_vec())]);
        let mut d = Decoder::new(true);
        d.push(&frame(true, OP_CONTINUATION, b"x"));
        assert_eq!(d.next_frame().unwrap_err().code, CLOSE_PROTOCOL);
        let mut d = Decoder::new(true);
        d.push(&frame(false, OP_BINARY, b"x"));
        d.push(&frame(true, OP_BINARY, b"y"));
        assert_eq!(d.next_frame().unwrap(), Some(Frame::Data(b"x".to_vec())));
        assert_eq!(d.next_frame().unwrap_err().code, CLOSE_PROTOCOL);
    }

    #[test]
    fn ping_pong_and_close() {
        let mut d = Decoder::new(true);
        d.push(&frame(true, OP_PING, b"are you there"));
        d.push(&frame(true, OP_PONG, b""));
        d.push(&frame(true, OP_CLOSE, &close_payload(CLOSE_NORMAL)));
        d.push(&frame(true, OP_CLOSE, b""));
        assert_eq!(
            all(&mut d).unwrap(),
            vec![
                Frame::Ping(b"are you there".to_vec()),
                Frame::Pong,
                Frame::Close(Some(CLOSE_NORMAL)),
                Frame::Close(None),
            ]
        );
        for bad in [
            frame(false, OP_PING, b"p"),
            frame(true, OP_PING, &[0; 126]),
            frame(true, OP_CLOSE, b"x"),
        ] {
            let mut d = Decoder::new(true);
            d.push(&bad);
            assert_eq!(d.next_frame().unwrap_err().code, CLOSE_PROTOCOL);
        }
    }

    #[test]
    fn an_oversize_frame_or_a_reserved_bit_closes() {
        let mut d = Decoder::new(true);
        let mut h = vec![0x80 | OP_BINARY, 0x80 | 127];
        h.extend_from_slice(&65537u64.to_be_bytes());
        d.push(&h);
        assert_eq!(d.next_frame().unwrap_err().code, CLOSE_TOO_BIG);
        let mut d = Decoder::new(true);
        let mut f = frame(true, OP_BINARY, b"x");
        f[0] |= 0x40;
        d.push(&f);
        assert_eq!(d.next_frame().unwrap_err().code, CLOSE_PROTOCOL);
        let mut d = Decoder::new(true);
        d.push(&frame(true, 0x3, b"x"));
        assert_eq!(d.next_frame().unwrap_err().code, CLOSE_PROTOCOL);
    }
}
