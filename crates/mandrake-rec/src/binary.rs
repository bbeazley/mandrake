//! Shared big-endian binary helpers, C-string decoders, and command-block packing.

use byteorder::{BigEndian, ByteOrder};

use crate::writer::MAX_BLOCK_BYTES;

#[inline]
pub(crate) fn be_u16(d: &[u8], o: usize) -> u16 {
    BigEndian::read_u16(&d[o..o + 2])
}

#[inline]
pub(crate) fn be_i16(d: &[u8], o: usize) -> i16 {
    BigEndian::read_i16(&d[o..o + 2])
}

#[inline]
pub(crate) fn be_u32(d: &[u8], o: usize) -> u32 {
    BigEndian::read_u32(&d[o..o + 4])
}

#[inline]
pub(crate) fn be_i32(d: &[u8], o: usize) -> i32 {
    BigEndian::read_i32(&d[o..o + 4])
}

#[inline]
pub(crate) fn put_u16(d: &mut [u8], o: usize, v: u16) {
    BigEndian::write_u16(&mut d[o..o + 2], v);
}

#[inline]
pub(crate) fn put_u32(d: &mut [u8], o: usize, v: u32) {
    BigEndian::write_u32(&mut d[o..o + 4], v);
}

pub(crate) fn c_bytes(b: &[u8]) -> &[u8] {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    &b[..end]
}

pub(crate) fn c_string(b: &[u8]) -> String {
    crate::mac_roman::decode(c_bytes(b))
}

pub(crate) fn c_string_trimmed(b: &[u8]) -> String {
    crate::mac_roman::decode(crate::mac_roman::trim(c_bytes(b)))
}

pub(crate) struct SliceReader<'a> {
    bytes: &'a [u8],
    off: usize,
}

impl<'a> SliceReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, off: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.off
    }

    pub(crate) fn skip(&mut self, n: usize) -> Option<()> {
        if self.remaining() < n {
            return None;
        }
        self.off += n;
        Some(())
    }

    pub(crate) fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.off.checked_add(n)?;
        let slice = self.bytes.get(self.off..end)?;
        self.off = end;
        Some(slice)
    }

    pub(crate) fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Option<u16> {
        let b = self.take(2)?;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }

    pub(crate) fn i16(&mut self) -> Option<i16> {
        let b = self.take(2)?;
        Some(i16::from_be_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn i32(&mut self) -> Option<i32> {
        let b = self.take(4)?;
        Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Read a signed 16-bit array count clamped to `[0, remaining / elem_size]`.
    pub(crate) fn bounded_count(&mut self, elem_size: usize) -> Option<usize> {
        let raw = self.i16()?;
        Some((raw.max(0) as usize).min(self.remaining() / elem_size))
    }

    /// Read `count` big-endian `u16`s, bounding `count` to what remains in the slice.
    pub(crate) fn u16_vec(&mut self, raw_count: i16) -> Vec<u16> {
        let count = (raw_count.max(0) as usize).min(self.remaining() / 2);
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(v) = self.u16() {
                out.push(v);
            }
        }
        out
    }

    /// Read an optional signed 16-bit count followed by that many `u16`s.
    pub(crate) fn u16_vec_opt(&mut self) -> Vec<u16> {
        match self.i16() {
            Some(raw) => self.u16_vec(raw),
            None => Vec::new(),
        }
    }
}

pub(crate) fn write_command_block(out: &mut Vec<u8>, payload: &[u8], count: u16, last: bool) {
    out.extend_from_slice(&u32::from(last).to_be_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(payload);
}

pub(crate) fn flush_command_block(
    out: &mut Vec<u8>,
    buffer: &mut Vec<u8>,
    count: &mut u16,
    last: bool,
) -> bool {
    if buffer.is_empty() && !last {
        return false;
    }
    write_command_block(out, buffer, *count, last);
    buffer.clear();
    *count = 0;
    true
}

pub(crate) fn push_command_block(
    out: &mut Vec<u8>,
    buffer: &mut Vec<u8>,
    count: &mut u16,
    command: &[u8],
) -> bool {
    let Some(&kind) = command.get(2) else {
        return false;
    };
    if command.len() > MAX_BLOCK_BYTES {
        return false;
    }
    let flushed = if buffer.len() + command.len() > MAX_BLOCK_BYTES {
        flush_command_block(out, buffer, count, false)
    } else {
        false
    };
    if kind != 0 {
        buffer.extend_from_slice(command);
        *count = count.wrapping_add(1);
    }
    flushed
}

/// Buffers encoded commands into <= 0x800-byte film blocks with 8-byte headers.
#[derive(Debug, Clone)]
pub(crate) struct CommandBlockPacker {
    buffer: Vec<u8>,
    count: u16,
    blocks_flushed: usize,
}

impl CommandBlockPacker {
    pub(crate) fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(MAX_BLOCK_BYTES),
            count: 0,
            blocks_flushed: 0,
        }
    }

    pub(crate) fn push(&mut self, out: &mut Vec<u8>, command: &[u8]) {
        if push_command_block(out, &mut self.buffer, &mut self.count, command) {
            self.blocks_flushed += 1;
        }
    }

    pub(crate) fn flush(&mut self, out: &mut Vec<u8>, last: bool) {
        if flush_command_block(out, &mut self.buffer, &mut self.count, last) {
            self.blocks_flushed += 1;
        }
    }

    pub(crate) fn blocks_flushed(&self) -> usize {
        self.blocks_flushed
    }
}
