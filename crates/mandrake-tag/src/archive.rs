//! Myth II monolithic archive loader and parser; it also opens TFL's `.gor` archives and loose
//! tags ([`crate::tfl`]), told apart by the data's format ([`ArchiveFormat`]).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{Cursor, Seek, SeekFrom};
use std::sync::Mutex;

use mandrake_common::{FourCC, M2Error, SeekableRead, VirtualFileSystem};

use crate::header::{SBEntryTag, SBMonoHeader, SBTagHeader, encode_c_string};
use crate::tfl::gor::{GorHeader, GorTagHeader, read_gor_directory};

/// Which format an archive's file is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArchiveFormat {
    /// A Myth II monolith (`mth2`/`dng2`, [`SBMonoHeader`]).
    Myth2Monolith,
    /// A TFL `.gor` archive ([`crate::tfl::gor`]).
    TflGor,
    /// TFL loose tags (`tags/Local`, [`crate::tfl::loose`]), held in memory.
    TflLoose,
}

/// Represents the storage backing a `TagArchive`.
pub enum ArchiveSource {
    /// In-memory buffer of the entire archive file.
    Memory(Vec<u8>),
    /// Dynamic seekable reader (e.g. file on disk).
    Reader(Mutex<Box<dyn SeekableRead + Send>>),
}

impl fmt::Debug for ArchiveSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(buf) => write!(f, "ArchiveSource::Memory({} bytes)", buf.len()),
            Self::Reader(_) => write!(f, "ArchiveSource::Reader(..)"),
        }
    }
}

/// Metadata describing a single tag entry within an archive.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TagEntry {
    pub index: usize,
    pub name: String,
    pub tag_type: FourCC,
    pub tag_id: FourCC,
    pub offset: u32,
    pub size: u32,
    pub header: SBTagHeader,
    /// Which file of a [`TagArchive::with_overrides`] archive holds the tag: 0 the archive's
    /// own file, `n` its `n`-th override file.
    #[serde(default)]
    pub layer: u32,
}

impl TagEntry {
    pub fn tag_type_str(&self) -> &str {
        self.tag_type.as_str()
    }

    pub fn tag_id_str(&self) -> &str {
        self.tag_id.as_str()
    }
}

#[derive(Debug)]
pub struct TagArchive {
    header: SBMonoHeader,
    entry_tags: Vec<SBEntryTag>,
    tags: Vec<TagEntry>,
    index_by_id: HashMap<(FourCC, FourCC), usize>,
    source: ArchiveSource,
    source_len: u64,
    overrides: Vec<TagArchive>,
    format: ArchiveFormat,
}

impl TagArchive {
    fn build_index(tags: &[TagEntry]) -> HashMap<(FourCC, FourCC), usize> {
        let mut index = HashMap::with_capacity(tags.len());
        for (i, t) in tags.iter().enumerate() {
            index.entry((t.tag_type, t.tag_id)).or_insert(i);
        }
        index
    }

    pub fn open(vfs: &dyn VirtualFileSystem, path: &str) -> Result<Self, M2Error> {
        let reader = vfs.open_reader(path)?;
        Self::from_reader(reader)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self, M2Error> {
        let source_len = data.len() as u64;
        let mut cursor = Cursor::new(&data);
        let (header, entry_tags, tags, format) = Self::parse_directory(&mut cursor)?;
        let index_by_id = Self::build_index(&tags);
        Ok(Self {
            header,
            entry_tags,
            tags,
            index_by_id,
            source: ArchiveSource::Memory(data),
            source_len,
            overrides: Vec::new(),
            format,
        })
    }

    pub fn from_reader(mut reader: Box<dyn SeekableRead + Send>) -> Result<Self, M2Error> {
        let source_len = reader.seek(SeekFrom::End(0))?;
        let (header, entry_tags, tags, format) = Self::parse_directory(&mut reader)?;
        let index_by_id = Self::build_index(&tags);
        Ok(Self {
            header,
            entry_tags,
            tags,
            index_by_id,
            source: ArchiveSource::Reader(Mutex::new(reader)),
            source_len,
            overrides: Vec::new(),
            format,
        })
    }

    /// An in-memory archive of TFL tags, each a `myth` header and its payload: the loose tags of
    /// `tags/Local` ([`crate::tfl::loose`]). `name` becomes the header's name.
    pub fn from_tfl_tags(name: &str, tags: Vec<(GorTagHeader, Vec<u8>)>) -> Self {
        let mut data = Vec::new();
        let mut entries = Vec::with_capacity(tags.len());
        for (i, (mut h, payload)) in tags.into_iter().enumerate() {
            h.offset = data.len() as u32;
            h.size = payload.len() as u32;
            data.extend_from_slice(&payload);
            entries.push(Self::tfl_entry(i, &h));
        }
        let source_len = data.len() as u64;
        let index_by_id = Self::build_index(&entries);
        Self {
            header: Self::tfl_header(name, 0, 0, 0, entries.len(), data.len() as u32),
            entry_tags: Vec::new(),
            tags: entries,
            index_by_id,
            source: ArchiveSource::Memory(data),
            source_len,
            overrides: Vec::new(),
            format: ArchiveFormat::TflLoose,
        }
    }

    /// The format of this archive's own file.
    pub fn format(&self) -> ArchiveFormat {
        self.format
    }

    /// A [`TagEntry`] for a TFL tag header. The Myth II header it carries has TFL's name, group,
    /// id, offset, size, user data and version, the signature `myth`, and zero elsewhere.
    fn tfl_entry(index: usize, h: &GorTagHeader) -> TagEntry {
        TagEntry {
            index,
            name: h.name.clone(),
            tag_type: h.group,
            tag_id: h.id,
            offset: h.offset,
            size: h.size,
            header: SBTagHeader {
                identifier: 0,
                flags: 0,
                tag_type_idx: 0,
                name: h.name_bytes,
                tag_type: h.group,
                tag_id: h.id,
                tag_data_offset: h.offset as i32,
                tag_data_size: h.size as i32,
                user_data: h.user_data,
                version: h.version as i16,
                destination: 0,
                owner_index: 0,
                signature: GorTagHeader::SIGNATURE,
            },
            layer: 0,
        }
    }

    /// The Myth II header standing for a TFL archive's: its type, version, name, checksum,
    /// flags and tag count, with the signature `myth`.
    fn tfl_header(
        name: &str,
        ty: i16,
        version: u16,
        checksum: u32,
        count: usize,
        size: u32,
    ) -> SBMonoHeader {
        SBMonoHeader {
            r#type: ty,
            version,
            name: encode_c_string(name),
            description: [0; 64],
            entry_tag_count: 0,
            tag_list_count: count as u16,
            checksum: checksum.to_be_bytes(),
            flags: 0,
            size,
            header_checksum: [0; 4],
            padding: [0; 4],
            signature: GorTagHeader::SIGNATURE,
        }
    }

    /// A `.gor` file's directory as Myth II entries; an error for `LOCAL.GOR`, whose tags are
    /// loose files ([`GorHeader::holds_its_data`]).
    fn gor_parts(
        header: GorHeader,
        tags: Vec<GorTagHeader>,
    ) -> Result<(SBMonoHeader, Vec<SBEntryTag>, Vec<TagEntry>, ArchiveFormat), M2Error> {
        if !header.holds_its_data(&tags) {
            return Err(M2Error::TagParse(format!(
                "'{}' indexes loose tags (tags/Local) and holds no tag data",
                header.name
            )));
        }
        let mut mono = Self::tfl_header(
            &header.name,
            header.r#type,
            header.version,
            header.checksum,
            tags.len(),
            header.directory_offset,
        );
        mono.flags = header.flags as u32;
        let entries = tags
            .iter()
            .enumerate()
            .map(|(i, h)| Self::tfl_entry(i, h))
            .collect();
        Ok((mono, Vec::new(), entries, ArchiveFormat::TflGor))
    }

    /// Internal parser for headers and directory table.
    fn parse_directory<R: SeekableRead>(
        reader: &mut R,
    ) -> Result<(SBMonoHeader, Vec<SBEntryTag>, Vec<TagEntry>, ArchiveFormat), M2Error> {
        reader.seek(SeekFrom::Start(0))?;
        let header = SBMonoHeader::read(reader);
        let myth2 = matches!(&header, Ok(h)
            if h.signature == SBMonoHeader::SIG_DNG2 || h.signature == SBMonoHeader::SIG_MTH2);
        if !myth2 && let Some((gor, tags)) = read_gor_directory(reader)? {
            return Self::gor_parts(gor, tags);
        }
        let header = header?;
        reader.seek(SeekFrom::Start(SBMonoHeader::SIZE as u64))?;

        // Validate archive signature ('dng2' or 'mth2')
        if !myth2 {
            return Err(M2Error::InvalidMagic {
                expected: "dng2 or mth2".to_string(),
                found: header.signature.to_string(),
            });
        }

        // Read entry tags if present
        let mut entry_tags = Vec::with_capacity(header.entry_tag_count as usize);
        for _ in 0..header.entry_tag_count {
            let entry = SBEntryTag::read(reader)?;
            entry_tags.push(entry);
        }

        // Read tag headers list
        let mut tags = Vec::with_capacity(header.tag_list_count as usize);
        for i in 0..header.tag_list_count {
            let tag_header = SBTagHeader::read(reader)?;
            if tag_header.signature != SBTagHeader::SIGNATURE {
                return Err(M2Error::TagParse(format!(
                    "Tag header {} has invalid signature: expected 'mth2', got '{}'",
                    i, tag_header.signature
                )));
            }

            let offset = u32::try_from(tag_header.tag_data_offset).map_err(|_| {
                M2Error::TagParse(format!(
                    "Tag header {i} has negative offset {}",
                    tag_header.tag_data_offset
                ))
            })?;
            let size = u32::try_from(tag_header.tag_data_size).map_err(|_| {
                M2Error::TagParse(format!(
                    "Tag header {i} has negative size {}",
                    tag_header.tag_data_size
                ))
            })?;

            let entry = TagEntry {
                index: i as usize,
                name: tag_header.name_as_str().to_string(),
                tag_type: tag_header.tag_type,
                tag_id: tag_header.tag_id,
                offset,
                size,
                header: tag_header,
                layer: 0,
            };
            tags.push(entry);
        }

        Ok((header, entry_tags, tags, ArchiveFormat::Myth2Monolith))
    }

    /// This archive with the tags of `files` over its own, in the engine's search order: later
    /// files override earlier ones, and all of them override this archive
    /// ([`crate::tag_files`]). Every lookup and [`Self::list_tags`] then sees the merged set,
    /// with an overridden tag gone; the header and entry tags stay this archive's.
    pub fn with_overrides(mut self, files: Vec<TagArchive>) -> Self {
        let mut file_base_layers = Vec::with_capacity(files.len());
        let mut next_layer = self.overrides.len() as u32 + 1;
        for file in &files {
            file_base_layers.push(next_layer);
            next_layer += 1 + file.overrides.len() as u32;
        }
        let mut seen = HashSet::new();
        let mut merged = Vec::new();
        for (file, base_layer) in files.iter().zip(file_base_layers.iter().copied()).rev() {
            let new: Vec<TagEntry> = file
                .tags
                .iter()
                .filter(|t| !seen.contains(&(t.tag_type, t.tag_id)))
                .map(|t| TagEntry {
                    layer: base_layer + t.layer,
                    ..t.clone()
                })
                .collect();
            seen.extend(new.iter().map(|t| (t.tag_type, t.tag_id)));
            merged.extend(new);
        }
        merged.extend(
            std::mem::take(&mut self.tags)
                .into_iter()
                .filter(|t| !seen.contains(&(t.tag_type, t.tag_id))),
        );
        for mut file in files {
            let nested = std::mem::take(&mut file.overrides);
            self.overrides.push(file);
            self.overrides.extend(nested);
        }
        self.tags = merged;
        self.index_by_id = Self::build_index(&self.tags);
        self
    }

    /// Returns a reference to the archive header.
    pub fn header(&self) -> &SBMonoHeader {
        &self.header
    }

    /// Returns the list of entry tags (mission/level mappings).
    pub fn entry_tags(&self) -> &[SBEntryTag] {
        &self.entry_tags
    }

    /// Returns the slice of all tag entries in the archive.
    pub fn list_tags(&self) -> &[TagEntry] {
        &self.tags
    }

    /// Number of tags in the archive.
    pub fn tag_count(&self) -> usize {
        self.tags.len()
    }

    /// Find the first tag matching `tag_type` and `name`.
    /// Checks exact name first, falling back to case-insensitive match.
    pub fn find_tag(&self, tag_type: FourCC, name: &str) -> Option<&TagEntry> {
        self.tags
            .iter()
            .find(|t| t.tag_type == tag_type && t.name == name)
            .or_else(|| {
                self.tags
                    .iter()
                    .find(|t| t.tag_type == tag_type && t.name.eq_ignore_ascii_case(name))
            })
    }

    pub fn find_tag_by_id(&self, tag_type: FourCC, tag_id: FourCC) -> Option<&TagEntry> {
        let &index = self.index_by_id.get(&(tag_type, tag_id))?;
        self.tags.get(index)
    }

    /// Return all tags matching a given `tag_type`.
    pub fn find_tags_by_type(&self, tag_type: FourCC) -> Vec<&TagEntry> {
        self.tags
            .iter()
            .filter(|t| t.tag_type == tag_type)
            .collect()
    }

    /// Read the binary data payload for a tag entry.
    pub fn get_tag_data(&self, entry: &TagEntry) -> Result<Vec<u8>, M2Error> {
        match entry.layer {
            0 => self.read_tag_data(entry),
            n => self
                .overrides
                .get(n as usize - 1)
                .ok_or_else(|| M2Error::TagParse(format!("tag '{}': no file {n}", entry.name)))?
                .read_tag_data(entry),
        }
    }

    pub fn read_tag_prefix(&self, entry: &TagEntry, len: usize) -> Result<Vec<u8>, M2Error> {
        let capped = u32::try_from(len).unwrap_or(u32::MAX);
        let short = TagEntry {
            size: entry.size.min(capped),
            ..entry.clone()
        };
        match entry.layer {
            0 => self.read_tag_data(&short),
            n => self
                .overrides
                .get(n as usize - 1)
                .ok_or_else(|| M2Error::TagParse(format!("tag '{}': no file {n}", entry.name)))?
                .read_tag_data(&short),
        }
    }

    fn read_tag_data(&self, entry: &TagEntry) -> Result<Vec<u8>, M2Error> {
        let offset = entry.offset as u64;
        let size = entry.size as usize;
        if size == 0 {
            return Ok(Vec::new());
        }

        match &self.source {
            ArchiveSource::Memory(bytes) => {
                let start = offset as usize;
                let end = start.checked_add(size).ok_or_else(|| {
                    M2Error::TagParse(format!("tag '{}' offset + size overflowed", entry.name))
                })?;
                if end > bytes.len() {
                    return Err(M2Error::TagParse(format!(
                        "tag '{}' range [{}..{}] exceeds archive size {}",
                        entry.name,
                        start,
                        end,
                        bytes.len()
                    )));
                }
                Ok(bytes[start..end].to_vec())
            }
            ArchiveSource::Reader(reader_mutex) => {
                let len = self.source_len;
                if offset.checked_add(size as u64).is_none_or(|end| end > len) {
                    return Err(M2Error::TagParse(format!(
                        "tag '{}' range [{offset}..+{size}] exceeds archive size {len}",
                        entry.name
                    )));
                }
                let mut reader = reader_mutex.lock().map_err(|e| {
                    M2Error::TagParse(format!("failed to lock archive reader: {e}"))
                })?;
                reader.seek(SeekFrom::Start(offset))?;
                let mut buffer = vec![0u8; size];
                reader.read_exact(&mut buffer)?;
                Ok(buffer)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A monolith with the given `(group, id, payload)` tags.
    fn monolith(tags: &[(&[u8; 4], &[u8; 4], &[u8])]) -> TagArchive {
        let mut out = Vec::new();
        SBMonoHeader {
            r#type: 1,
            version: 1,
            name: [0; 32],
            description: [0; 64],
            entry_tag_count: 0,
            tag_list_count: tags.len() as u16,
            checksum: [0; 4],
            flags: 0,
            size: 0,
            header_checksum: [0; 4],
            padding: [0; 4],
            signature: SBMonoHeader::SIG_MTH2,
        }
        .write(&mut out)
        .unwrap();
        let mut offset = SBMonoHeader::SIZE + tags.len() * SBTagHeader::SIZE;
        for (group, id, data) in tags {
            SBTagHeader {
                identifier: 0,
                flags: 0,
                tag_type_idx: 0,
                name: [0; 32],
                tag_type: FourCC::new(**group),
                tag_id: FourCC::new(**id),
                tag_data_offset: offset as i32,
                tag_data_size: data.len() as i32,
                user_data: 0,
                version: 0,
                destination: 0,
                owner_index: 0,
                signature: SBTagHeader::SIGNATURE,
            }
            .write(&mut out)
            .unwrap();
            offset += data.len();
        }
        for (_, _, data) in tags {
            out.extend_from_slice(data);
        }
        TagArchive::from_bytes(out).unwrap()
    }

    #[test]
    fn overrides_win_in_file_order_and_keep_the_rest() {
        let base = monolith(&[(b"mesh", b"aaaa", b"base a"), (b"mesh", b"bbbb", b"base b")]);
        let patch = monolith(&[
            (b"mesh", b"aaaa", b"patch a"),
            (b".256", b"cccc", b"patch c"),
        ]);
        let plugin = monolith(&[(b"mesh", b"aaaa", b"plugin a")]);
        let merged = base.with_overrides(vec![patch, plugin]);
        let data = |group: &[u8; 4], id: &[u8; 4]| {
            let e = merged
                .find_tag_by_id(FourCC::new(*group), FourCC::new(*id))
                .unwrap();
            merged.get_tag_data(e).unwrap()
        };
        assert_eq!(data(b"mesh", b"aaaa"), b"plugin a");
        assert_eq!(data(b"mesh", b"bbbb"), b"base b");
        assert_eq!(data(b".256", b"cccc"), b"patch c");
        assert_eq!(merged.tag_count(), 3, "the overridden tags are gone");
        assert_eq!(merged.find_tags_by_type(FourCC::new(*b"mesh")).len(), 2);
    }
}
