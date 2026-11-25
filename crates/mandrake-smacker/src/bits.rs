//! Little-endian bit reader: bits are taken from the least significant end of each byte
//! first, and multi-bit fields are assembled least significant bit first.

/// Reads bits from a byte slice. Reading past the end yields zero bits and sets
/// [`BitReader::overrun`], so a truncated stream decodes to something instead of panicking.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Bit position from the start of `data`.
    pos: usize,
    /// Up to 64 bits starting at bit `pos`, low bit first.
    cache: u64,
    /// Number of valid bits in `cache`.
    avail: u32,
    overrun: bool,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut r = BitReader {
            data,
            pos: 0,
            cache: 0,
            avail: 0,
            overrun: false,
        };
        r.refill();
        r
    }

    fn refill(&mut self) {
        while self.avail <= 56 {
            let byte_index = (self.pos + self.avail as usize) >> 3;
            match self.data.get(byte_index) {
                Some(&b) => {
                    self.cache |= (b as u64) << self.avail;
                    self.avail += 8;
                }
                None => break,
            }
        }
    }

    #[inline]
    pub(crate) fn peek(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if self.avail < n {
            self.refill();
        }
        (self.cache & ((1u64 << n) - 1)) as u32
    }

    #[inline]
    pub(crate) fn consume(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        if self.avail >= n {
            self.cache >>= n;
            self.avail -= n;
            self.pos += n as usize;
        } else {
            self.cache = 0;
            self.avail = 0;
            self.pos += n as usize;
            self.overrun = true;
        }
    }

    /// Reads one bit.
    #[inline]
    pub(crate) fn bit(&mut self) -> bool {
        if self.avail == 0 {
            self.refill();
            if self.avail == 0 {
                self.overrun = true;
                self.pos += 1;
                return false;
            }
        }
        let b = self.cache & 1 != 0;
        self.cache >>= 1;
        self.avail -= 1;
        self.pos += 1;
        b
    }

    /// Reads `n` (at most 32) bits, the first bit read being the least significant.
    pub(crate) fn bits(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if self.avail < n {
            self.refill();
        }
        if self.avail >= n {
            let v = (self.cache & ((1u64 << n) - 1)) as u32;
            self.cache >>= n;
            self.avail -= n;
            self.pos += n as usize;
            v
        } else {
            let mut v = 0;
            for i in 0..n {
                v |= (self.bit() as u32) << i;
            }
            v
        }
    }

    /// True once a read went past the end of the data.
    pub(crate) fn overrun(&self) -> bool {
        self.overrun
    }

    /// Number of bits consumed so far.
    #[cfg(test)]
    pub(crate) fn position(&self) -> usize {
        self.pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_are_lsb_first() {
        let mut r = BitReader::new(&[0b1010_0110, 0xff, 0x01]);
        assert!(!r.bit());
        assert!(r.bit());
        assert!(r.bit());
        assert_eq!(r.bits(5), 0b10100);
        assert_eq!(r.bits(9), 0x1ff);
        assert_eq!(r.position(), 17);
        assert_eq!(r.bits(7), 0);
        assert!(!r.overrun());
        assert!(!r.bit());
        assert!(r.overrun());
    }

    #[test]
    fn multi_byte_fields_cross_bytes() {
        let mut r = BitReader::new(&[0x34, 0x12, 0x78, 0x56]);
        assert_eq!(r.bits(4), 0x4);
        assert_eq!(r.bits(16), 0x8123);
        assert_eq!(r.bits(12), 0x567);
    }
}
