use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::header::SBMonoHeader;

static DOWNLOADS_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_downloads_dir(dir: Option<PathBuf>) {
    *DOWNLOADS_DIR.write().unwrap_or_else(|e| e.into_inner()) = dir;
}

pub fn downloads_dir() -> Option<PathBuf> {
    DOWNLOADS_DIR
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn downloaded_plugin_heads() -> Vec<TagFileHead> {
    let Some(dir) = downloads_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<TagFileHead> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_none_or(|e| e != "part"))
        .filter_map(|path| {
            let name = plugin_name(&path)?;
            let mut f = std::fs::File::open(&path).ok()?;
            let header = SBMonoHeader::read(&mut f).ok()?;
            (header.r#type == TYPE_PLUGIN
                && (header.signature == SBMonoHeader::SIG_DNG2
                    || header.signature == SBMonoHeader::SIG_MTH2))
                .then_some(TagFileHead { path, name, header })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn downloaded_plugins() -> Vec<PathBuf> {
    downloaded_plugin_heads()
        .into_iter()
        .map(|h| h.path)
        .collect()
}

pub const FOUNDATION_FILES: [&str; 3] = ["large install", "medium install", "small install"];

pub const REQUIRED_PATCHES: [&str; 5] = [
    "Patch 1.2",
    "Patch 1.3",
    "Patch 1.4",
    "Patch 1.5",
    "Patch 1.8 Interface",
];

pub const TFL_ONLY_PATCH: &str = "Patch 1.5 Unity";

pub const INTERNATIONAL_FILES: [&str; 2] =
    ["international small install", "international large install"];

pub const TYPE_PATCH: i16 = 2;
pub const TYPE_PLUGIN: i16 = 1;

pub const TYPE_INTERFACE: i16 = 5;
pub const INTERFACE_800X600: &str = "Myth II Interface (800x600)";

#[derive(Debug, Clone, Default)]
pub struct EngineTagFiles {
    pub paths: Vec<PathBuf>,
    pub missing_patches: Vec<String>,
    pub missing_plugins: Vec<String>,
}

impl EngineTagFiles {
    pub fn with_international(mut self, game_dir: &Path) -> Self {
        let intl: Vec<PathBuf> = international_files(game_dir)
            .into_iter()
            .filter(|p| !self.paths.contains(p))
            .collect();
        self.paths.splice(0..0, intl);
        self
    }

    pub fn complete(&self) -> bool {
        self.missing_patches.is_empty() && self.missing_plugins.is_empty()
    }
}

fn international_files(game_dir: &Path) -> Vec<PathBuf> {
    INTERNATIONAL_FILES
        .iter()
        .map(|n| game_dir.join("tags").join(n))
        .filter(|p| is_file(p))
        .collect()
}

fn is_file(path: &Path) -> bool {
    mandrake_common::game_vfs().is_file(&path.to_string_lossy())
}

fn list_dir(dir: &Path) -> Vec<PathBuf> {
    mandrake_common::game_vfs()
        .list_dir(&dir.to_string_lossy())
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

const PLUGIN_NAME_MAX: usize = 0x1f;

pub fn plugin_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.as_encoded_bytes();
    let name = &name[..name.len().min(PLUGIN_NAME_MAX)];
    Some(match std::str::from_utf8(name) {
        Ok(ascii) if ascii.is_ascii() => ascii.to_owned(),
        _ => mandrake_common::mac_roman::decode(name),
    })
}

fn header(path: &Path) -> Option<SBMonoHeader> {
    let mut file = mandrake_common::game_vfs()
        .open_reader(&path.to_string_lossy())
        .ok()?;
    let h = SBMonoHeader::read(&mut file).ok()?;
    (h.signature == SBMonoHeader::SIG_DNG2 || h.signature == SBMonoHeader::SIG_MTH2).then_some(h)
}

pub fn engine_tag_files(game_dir: &Path, plugins: &[&str]) -> EngineTagFiles {
    let tags = game_dir.join("tags");
    order_engine_tag_files(
        &|name| {
            let p = tags.join(name);
            is_file(&p).then_some(p)
        },
        &installed_files(game_dir),
        plugins,
        &crate::tagset::chosen_metaserver(game_dir),
    )
}

pub fn order_engine_tag_files(
    tag_file: &dyn Fn(&str) -> Option<PathBuf>,
    installed: &[TagFileHead],
    plugins: &[&str],
    metaserver: &str,
) -> EngineTagFiles {
    let mut out = EngineTagFiles::default();
    out.paths.extend(
        INTERNATIONAL_FILES
            .iter()
            .chain(FOUNDATION_FILES.iter())
            .filter_map(|n| tag_file(n)),
    );
    let find = |name: &str, ty: i16| {
        installed
            .iter()
            .find(|f| f.header.r#type == ty && f.name == name)
    };
    for version in 2..=5 {
        out.paths.extend(
            installed
                .iter()
                .filter(|f| f.header.r#type == TYPE_PATCH && f.header.version == version)
                .map(|f| f.path.clone()),
        );
    }
    if let Some(f) = find(metaserver, crate::tagset::TYPE_COMPONENT) {
        out.paths.push(f.path.clone());
    }
    if let Some(f) = find("Patch 1.8 Interface", TYPE_PATCH) {
        out.paths.push(f.path.clone());
    }
    for name in plugins {
        match installed
            .iter()
            .find(|f| f.header.r#type == TYPE_PLUGIN && f.name.eq_ignore_ascii_case(name))
        {
            Some(f) => out.paths.push(f.path.clone()),
            None => out.missing_plugins.push(name.to_string()),
        }
    }
    out.missing_patches = REQUIRED_PATCHES
        .iter()
        .filter(|n| {
            installed
                .iter()
                .all(|f| f.header.r#type != TYPE_PATCH || f.name != **n)
        })
        .map(|n| n.to_string())
        .collect();
    out
}

#[derive(Debug, Clone)]
pub struct TagFileHead {
    pub path: PathBuf,
    pub name: String,
    pub header: SBMonoHeader,
}

impl TagFileHead {
    pub fn read(path: PathBuf, name: String, head: &[u8]) -> Option<Self> {
        let header = SBMonoHeader::read(&mut std::io::Cursor::new(head)).ok()?;
        (header.signature == SBMonoHeader::SIG_DNG2 || header.signature == SBMonoHeader::SIG_MTH2)
            .then_some(Self { path, name, header })
    }
}

fn installed_files(game_dir: &Path) -> Vec<TagFileHead> {
    let mut installed: Vec<TagFileHead> = list_dir(&game_dir.join("plugins"))
        .into_iter()
        .filter(|p| is_file(p))
        .filter_map(|path| {
            Some(TagFileHead {
                name: plugin_name(&path)?,
                header: header(&path)?,
                path,
            })
        })
        .collect();
    installed.sort_by(|a, b| a.path.cmp(&b.path));
    installed.extend(downloaded_plugin_heads());
    installed
}

pub fn engine_interface_tag_files(game_dir: &Path, plugins: &[&str]) -> EngineTagFiles {
    engine_tag_files(game_dir, plugins).with_international(game_dir)
}

pub fn dialog_tag_files(game_dir: &Path, interface: Option<&str>) -> EngineTagFiles {
    let mut out = engine_tag_files(game_dir, &[]);
    let Some(name) = interface else {
        return out;
    };
    let found = installed_files(game_dir)
        .into_iter()
        .find(|f| f.header.r#type == TYPE_INTERFACE && f.name.eq_ignore_ascii_case(name));
    match found {
        Some(f) => out.paths.push(f.path),
        None => out.missing_plugins.push(name.to_string()),
    }
    out
}

pub struct TagLookup {
    archives: Vec<crate::archive::TagArchive>,
}

impl TagLookup {
    pub fn empty() -> Self {
        Self {
            archives: Vec::new(),
        }
    }

    pub fn open(files: &EngineTagFiles) -> Self {
        let vfs = mandrake_common::game_vfs();
        Self {
            archives: files
                .paths
                .iter()
                .filter_map(|p| crate::archive::TagArchive::open(vfs, &p.to_string_lossy()).ok())
                .collect(),
        }
    }

    pub fn get(
        &self,
        group: mandrake_common::FourCC,
        id: mandrake_common::FourCC,
    ) -> Option<Vec<u8>> {
        self.archives.iter().rev().find_map(|a| {
            a.find_tag_by_id(group, id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    }

    pub fn get_beside(
        &self,
        group: mandrake_common::FourCC,
        id: mandrake_common::FourCC,
        beside: (mandrake_common::FourCC, mandrake_common::FourCC),
    ) -> Option<Vec<u8>> {
        self.archives
            .iter()
            .rev()
            .find(|a| a.find_tag_by_id(beside.0, beside.1).is_some())
            .and_then(|a| {
                a.find_tag_by_id(group, id)
                    .and_then(|e| a.get_tag_data(e).ok())
            })
            .or_else(|| self.get(group, id))
    }

    pub fn len(&self) -> usize {
        self.archives.len()
    }

    pub fn is_empty(&self) -> bool {
        self.archives.is_empty()
    }
}

pub fn tfl_tag_files(game_dir: &Path) -> Vec<PathBuf> {
    let vfs = mandrake_common::game_vfs();
    let dir = game_dir.join("tags");
    let mut gors: Vec<PathBuf> = vfs
        .list_dir(&dir.to_string_lossy())
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| {
            let s = p.to_string_lossy();
            vfs.is_file(&s)
                && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gor"))
                && vfs
                    .open_reader(&s)
                    .ok()
                    .and_then(|mut f| crate::tfl::gor::read_gor_directory(&mut f).ok().flatten())
                    .is_some_and(|(h, tags)| h.holds_its_data(&tags))
        })
        .collect();
    gors.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
    gors
}

pub fn open_tfl_tags(game_dir: &Path) -> Result<crate::TagArchive, mandrake_common::M2Error> {
    let mut files = tfl_tag_files(game_dir).into_iter();
    let first = files.next().ok_or_else(|| {
        mandrake_common::M2Error::TagParse(format!("no .gor in {}", game_dir.display()))
    })?;
    let open =
        |p: &Path| crate::TagArchive::open(mandrake_common::game_vfs(), &p.to_string_lossy());
    let mut over = files.map(|p| open(&p)).collect::<Result<Vec<_>, _>>()?;
    over.push(crate::tfl::loose::loose_archive(
        &game_dir.join("tags").join("Local"),
    )?);
    Ok(open(&first)?.with_overrides(over))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_header(path: &Path, ty: i16, version: u16) {
        let mut b = vec![0u8; SBMonoHeader::SIZE];
        b[0..2].copy_from_slice(&ty.to_be_bytes());
        b[2..4].copy_from_slice(&version.to_be_bytes());
        b[124..128].copy_from_slice(b"mth2");
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn a_data_dir_without_the_tfl_only_patch_is_complete() {
        let dir = std::env::temp_dir().join(format!("mandrake-tag-demo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        std::fs::create_dir_all(dir.join("tags")).unwrap();
        for (name, version) in [
            ("Patch 1.2", 2),
            ("Patch 1.3", 3),
            ("Patch 1.4", 4),
            ("Patch 1.5", 5),
            ("Patch 1.8 Interface", 8),
        ] {
            write_header(&dir.join("plugins").join(name), TYPE_PATCH, version);
        }
        let f = engine_tag_files(&dir, &[]);
        assert!(f.complete(), "{f:?}");
        std::fs::remove_file(dir.join("plugins/Patch 1.3")).unwrap();
        let f = engine_tag_files(&dir, &[]);
        assert_eq!(f.missing_patches, ["Patch 1.3"]);
        write_header(&dir.join("plugins").join(TFL_ONLY_PATCH), TYPE_PLUGIN, 5);
        let f = engine_tag_files(&dir, &[]);
        assert!(f.paths.iter().all(|p| !p.ends_with(TFL_ONLY_PATCH)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    fn head(name: &str, ty: i16, version: u16) -> TagFileHead {
        let header = SBMonoHeader {
            r#type: ty,
            version,
            name: [0; 32],
            description: [0; 64],
            entry_tag_count: 0,
            tag_list_count: 0,
            checksum: [0; 4],
            flags: 0,
            size: 0,
            header_checksum: [0; 4],
            padding: [0; 4],
            signature: SBMonoHeader::SIG_MTH2,
        };
        let mut bytes = Vec::new();
        header.write(&mut bytes).unwrap();
        TagFileHead::read(format!("plugins/{name}").into(), name.into(), &bytes).unwrap()
    }

    #[test]
    fn order_over_names_and_headers() {
        let installed = [
            head("Patch 1.2", TYPE_PATCH, 2),
            head("Patch 1.3", TYPE_PATCH, 3),
            head("Patch 1.4", TYPE_PATCH, 4),
            head("Patch 1.5", TYPE_PATCH, 5),
            head("Patch 1.8 Interface", TYPE_PATCH, 8),
            head("bagrada metaserver", crate::tagset::TYPE_COMPONENT, 1),
            head("Chimera", TYPE_PLUGIN, 1),
            head("Unnamed", TYPE_PLUGIN, 1),
        ];
        let mut installed = installed.to_vec();
        installed.reverse();
        installed.sort_by(|a, b| a.path.cmp(&b.path));
        let tags = ["small install", "international small install"];
        let tag_file = |n: &str| {
            tags.contains(&n)
                .then(|| PathBuf::from(format!("tags/{n}")))
        };
        let files =
            order_engine_tag_files(&tag_file, &installed, &["chimera"], "bagrada metaserver");
        let got: Vec<String> = files
            .paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            got,
            [
                "tags/international small install",
                "tags/small install",
                "plugins/Patch 1.2",
                "plugins/Patch 1.3",
                "plugins/Patch 1.4",
                "plugins/Patch 1.5",
                "plugins/bagrada metaserver",
                "plugins/Patch 1.8 Interface",
                "plugins/Chimera",
            ]
        );
        assert!(files.complete());

        installed.retain(|f| f.name != "Patch 1.2");
        let files = order_engine_tag_files(&tag_file, &installed, &["Gone"], "x");
        assert_eq!(files.missing_plugins, ["Gone"]);
        assert_eq!(files.missing_patches, ["Patch 1.2"]);
    }
}
