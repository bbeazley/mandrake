//! Myth II string list tag (`stli` / `text`) deserializer.

use crate::header::decode_c_string;
use mandrake_common::M2Error;

/// Deserialized Myth II string list tag (`stli` / `text`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct StringListTag {
    pub strings: Vec<String>,
}

impl StringListTag {
    /// Deserialize from raw tag payload.
    ///
    /// Supports both:
    /// 1. Myth II offset-table format: string count (`u16` or `u32`), offset table, and strings.
    /// 2. Delimited text format: strings separated by carriage return (`\r`), newline (`\n`), or null (`\0`).
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.is_empty() {
            return Ok(Self {
                strings: Vec::new(),
            });
        }

        // Try parsing structured offset table first:
        if let Ok(tag) = Self::try_parse_offset_table(data) {
            return Ok(tag);
        }

        // Fall back to delimited string parsing:
        Ok(Self::parse_delimited(data))
    }

    fn try_parse_offset_table(data: &[u8]) -> Result<Self, M2Error> {
        if let Some(parsed) = Self::try_parse_offset_table_width(data, 2)
            .or_else(|| Self::try_parse_offset_table_width(data, 4))
        {
            return Ok(parsed);
        }
        Err(M2Error::TagParse("not a valid offset table".into()))
    }

    fn try_parse_offset_table_width(data: &[u8], width: usize) -> Option<Self> {
        let read = |pos: usize| -> Option<usize> {
            match width {
                2 => crate::parse::be_u16(data, pos).map(|v| v as usize),
                4 => crate::parse::be_u32(data, pos).map(|v| v as usize),
                _ => None,
            }
        };
        let count = read(0)?;
        if count == 0 || count >= 4096 {
            return None;
        }
        let table_bytes = width.checked_add(count.checked_mul(width)?)?;
        if data.len() < table_bytes {
            return None;
        }
        let mut offsets = Vec::with_capacity(count);
        let mut last = 0usize;
        for i in 0..count {
            let off = read(width + i * width)?;
            if off < table_bytes || off > data.len() || off < last {
                return None;
            }
            last = off;
            offsets.push(off);
        }
        if offsets.first().copied() != Some(table_bytes) {
            return None;
        }
        let mut strings = Vec::with_capacity(count);
        for i in 0..count {
            let start = offsets[i];
            let end = if i + 1 < count {
                offsets[i + 1]
            } else {
                data.len()
            };
            strings.push(decode_c_string(&data[start..end]));
        }
        Some(Self { strings })
    }

    fn parse_delimited(data: &[u8]) -> Self {
        let mut strings = Vec::new();
        let mut current = Vec::new();

        for &b in data {
            if b == b'\r' || b == b'\n' || b == 0 {
                if !current.is_empty() {
                    strings.push(decode_c_string(&current));
                    current.clear();
                }
            } else {
                current.push(b);
            }
        }
        if !current.is_empty() {
            strings.push(decode_c_string(&current));
        }

        Self { strings }
    }

    /// Number of strings in the list.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Whether the string list is empty.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Retrieve string by index.
    pub fn get(&self, index: usize) -> Option<&str> {
        self.strings.get(index).map(|s| s.as_str())
    }

    /// Iterate over string slices.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.strings.iter().map(|s| s.as_str())
    }
}
