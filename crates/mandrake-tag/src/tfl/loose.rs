use std::path::Path;

use mandrake_common::M2Error;

use crate::archive::TagArchive;
use crate::tfl::gor::GorTagHeader;

pub fn read_loose_tag(bytes: &[u8]) -> Result<(GorTagHeader, Vec<u8>), M2Error> {
    let header = GorTagHeader::parse(bytes)?;
    let start = header.offset as usize;
    let end = start.checked_add(header.size as usize).ok_or_else(|| {
        M2Error::TagParse(format!(
            "loose tag '{}': offset {} + size {} overflow",
            header.name, header.offset, header.size
        ))
    })?;
    let payload = bytes.get(start..end).ok_or_else(|| {
        M2Error::TagParse(format!(
            "loose tag '{}': data {start}..{end} outside a {}-byte file",
            header.name,
            bytes.len()
        ))
    })?;
    Ok((header, payload.to_vec()))
}

pub fn loose_archive(dir: &Path) -> Result<TagArchive, M2Error> {
    let vfs = mandrake_common::game_vfs();
    let is_dir = |p: &str| vfs.exists(p) && !vfs.is_file(p);
    let mut files = Vec::new();
    let dir = dir.to_string_lossy();
    if is_dir(&dir) {
        for group in vfs.list_dir(&dir) {
            if is_dir(&group) {
                files.extend(vfs.list_dir(&group).into_iter().filter(|f| vfs.is_file(f)));
            }
        }
    }
    files.sort();
    let tags = files
        .iter()
        .map(|f| {
            read_loose_tag(&vfs.read_file(f)?).map_err(|e| M2Error::TagParse(format!("{f}: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TagArchive::from_tfl_tags("tags/Local", tags))
}
