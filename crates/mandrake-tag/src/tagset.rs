use std::fs::File;
use std::io;
use std::path::Path;

use crate::header::SBMonoHeader;
use crate::tag_files::{TYPE_PATCH, TYPE_PLUGIN};

pub const FOUNDATION_NAMES: [&str; 5] = [
    "small install",
    "medium install",
    "large install",
    "international small install",
    "international large install",
];
pub const FOUNDATION_SALT: u32 = 0x101;
pub const TYPE_FOUNDATION: i16 = 3;
pub const TYPE_INTERFACE: i16 = 5;
pub const TYPE_6: i16 = 6;
pub const TYPE_COMPONENT: i16 = 7;
pub const DEFAULT_METASERVER: &str = "bagrada metaserver";
pub const REQUIRED_PATCHES: [(u32, &str); 5] = [
    (0x35bb_8084, "Patch 1.2"),
    (0x7d32_5546, "Patch 1.3"),
    (0xb1ff_61d0, "Patch 1.4"),
    (0xa45c_5dba, "Patch 1.5"),
    (0xc0df_cc8d, PATCH_18_INTERFACE),
];
const PATCH_18_INTERFACE: &str = "Patch 1.8 Interface";
pub const TFL_PLUGINS: [&str; 3] = ["Patch 1.4 Unity", "Patch 1.5 Unity", "Patch 1.8.5 Unity"];
const NAME_MAX: usize = 0x1f;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEntry {
    pub name: Vec<u8>,
    pub url: Vec<u8>,
    pub checksum: u32,
    pub kind: i16,
    pub version: u16,
}

fn read_header(path: &Path) -> Option<SBMonoHeader> {
    let mut f = File::open(path).ok()?;
    let h = SBMonoHeader::read(&mut f).ok()?;
    (h.signature == SBMonoHeader::SIG_DNG2 || h.signature == SBMonoHeader::SIG_MTH2).then_some(h)
}

fn headers(dir: &Path) -> io::Result<Vec<(Vec<u8>, SBMonoHeader)>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        if !e.file_type()?.is_file() && !e.path().is_file() {
            continue;
        }
        let name = e.file_name().into_encoded_bytes();
        if let Some(h) = read_header(&e.path()) {
            out.push((name, h));
        }
    }
    out.sort_by_key(|(name, _)| name.to_ascii_lowercase());
    Ok(out)
}

pub fn plugin_entries(plugins_dir: &Path) -> io::Result<Vec<PluginEntry>> {
    let mut out = Vec::new();
    for (file_name, h) in headers(plugins_dir)? {
        if ![
            TYPE_PLUGIN,
            TYPE_PATCH,
            TYPE_INTERFACE,
            TYPE_6,
            TYPE_COMPONENT,
        ]
        .contains(&h.r#type)
        {
            continue;
        }
        let checksum = u32::from_be_bytes(h.checksum);
        let name = file_name[..file_name.len().min(NAME_MAX)].to_vec();
        if h.r#type == TYPE_PATCH
            && !REQUIRED_PATCHES
                .iter()
                .any(|&(c, n)| c == checksum && n.as_bytes() == name.as_slice())
        {
            continue;
        }
        if checksum == u32::MAX {
            continue;
        }
        let url = crate::parse::until_nul(&h.description).to_vec();
        let mut version = h.version;
        if h.r#type == TYPE_PLUGIN {
            if version != 0xffff
                && version & 4 != 0
                && crate::parse::is_plugin_url_description(&url)
            {
                version &= !4;
            }
            if TFL_PLUGINS.iter().any(|t| t.as_bytes() == name.as_slice()) {
                version = 0;
            }
        }
        out.push(PluginEntry {
            name,
            url,
            checksum,
            kind: h.r#type,
            version,
        });
    }
    Ok(out)
}

pub fn foundation_checksums(tags_dir: &Path) -> io::Result<[Option<u32>; 5]> {
    let files = headers(tags_dir)?;
    Ok(FOUNDATION_NAMES.map(|want| {
        files
            .iter()
            .find(|(name, h)| {
                h.r#type == TYPE_FOUNDATION && name.eq_ignore_ascii_case(want.as_bytes())
            })
            .map(|(_, h)| u32::from_be_bytes(h.checksum))
    }))
}

pub fn patch_limit(plugins: &[PluginEntry]) -> u16 {
    let mut limit = 1u16;
    while plugins
        .iter()
        .any(|p| p.kind == TYPE_PATCH && p.version == limit + 1)
    {
        limit += 1;
    }
    limit
}

pub fn foundation_checksum(game_dir: &Path, metaserver: &str) -> io::Result<Option<u32>> {
    let plugins = plugin_entries(&game_dir.join("plugins"))?;
    let foundations = foundation_checksums(&game_dir.join("tags"))?;
    Ok(foundation_checksum_of(&foundations, &plugins, metaserver))
}

pub fn foundation_checksum_of(
    foundations: &[Option<u32>; 5],
    plugins: &[PluginEntry],
    metaserver: &str,
) -> Option<u32> {
    let mut sum = FOUNDATION_SALT;
    for f in foundations {
        sum ^= (*f)?;
    }
    let limit = patch_limit(plugins);
    for v in 2..=limit {
        for p in plugins {
            if p.kind == TYPE_PATCH && p.version == v {
                sum ^= p.checksum;
            }
        }
    }
    if let Some(p) = plugins
        .iter()
        .find(|p| p.name == PATCH_18_INTERFACE.as_bytes())
    {
        sum ^= p.checksum;
    }
    if let Some(p) = plugins
        .iter()
        .find(|p| p.kind == TYPE_COMPONENT && p.name == metaserver.as_bytes())
    {
        sum ^= p.checksum;
    }
    Some(sum)
}

pub fn chosen_metaserver(game_dir: &Path) -> String {
    let path = game_dir.join("preferences/metaserver plugin");
    let bytes = mandrake_common::game_vfs()
        .open_reader(&path.to_string_lossy())
        .ok()
        .and_then(|mut r| {
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut r, &mut b).ok()?;
            Some(b)
        });
    metaserver_named(bytes.as_deref())
}

pub fn metaserver_named(preference: Option<&[u8]>) -> String {
    preference
        .map(|b| String::from_utf8_lossy(crate::parse::until_nul(b)).into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_METASERVER.to_string())
}
