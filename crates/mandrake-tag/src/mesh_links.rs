use std::path::Path;

use mandrake_common::FourCC;

use crate::archive::{TagArchive, TagEntry};
use crate::header::SBEntryTag;
use crate::tag_files::{TYPE_PLUGIN, engine_tag_files, plugin_name};

pub const MESH_HEADER_SIZE: usize = 0x400;

const TYPE_FOUNDATION: i16 = 3;
const PLUGIN_FLAG_NAMES_REQUIRED: u16 = 4;
const REQUIRED_PLUGINS: [(&str, &str); 5] = [
    ("TSG Level-Pack v1.4", "TSG Tagset v1.4"),
    ("JINN Mappack v.1", "JINN Tagset v.1"),
    ("Blue & Grey Levelpack", "Blue & Grey Tagset"),
    ("SF_CarnageIslands(v1.0)", "SF_SpecialForces(v1.0)"),
    ("Green Berets Mappack v1.2", "Green Berets Tagset v1.2"),
];

const MESH: FourCC = FourCC(*b"mesh");
const COLLECTION: FourCC = FourCC(*b".256");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshLinks {
    pub flags: u32,
    pub strings: Option<FourCC>,
    pub post_art: Option<FourCC>,
    pub pre_art: Option<FourCC>,
    pub next: Option<FourCC>,
    pub alternate: Option<FourCC>,
    pub briefing_text: Option<FourCC>,
    pub captions: Option<FourCC>,
    pub narration: Option<FourCC>,
    pub ambient_win: Option<FourCC>,
    pub ambient_loss: Option<FourCC>,
    pub hints: Option<FourCC>,
    pub campaign_flags: u32,
}

impl MeshLinks {
    pub fn from_header(header: &[u8]) -> Option<Self> {
        if header.len() < MESH_HEADER_SIZE {
            return None;
        }
        let word = |o: usize| [header[o], header[o + 1], header[o + 2], header[o + 3]];
        let tag = |o: usize| match u32::from_be_bytes(word(o)) {
            0 | u32::MAX => None,
            _ => Some(FourCC(word(o))),
        };
        Some(Self {
            flags: u32::from_be_bytes(word(0x4c)),
            strings: tag(0x8c),
            post_art: tag(0x90),
            pre_art: tag(0x94),
            next: tag(0x9c),
            alternate: tag(0xa0),
            briefing_text: tag(0xb0),
            captions: tag(0xfc),
            narration: tag(0x100),
            ambient_win: tag(0x104),
            ambient_loss: tag(0x108),
            hints: tag(0x1ec),
            campaign_flags: u32::from_be_bytes(word(0x224)),
        })
    }
}

pub fn string_list_lines(data: &[u8]) -> Vec<Vec<u8>> {
    data.split(|&b| matches!(b, 0 | b'\n' | b'\r'))
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

pub fn text_bytes(data: &[u8]) -> Vec<u8> {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    data[..end].to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedEntry {
    pub flags: u32,
    pub mesh: FourCC,
    pub file_name: Vec<u8>,
    pub name: Vec<u8>,
    pub order: i16,
    pub extra_flags: u32,
}

fn strtol_i16(text: &[u8]) -> i16 {
    let text = text.trim_ascii_start();
    let (negative, digits) = match text.first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let value = digits
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .fold(0i64, |value, b| {
            value.saturating_mul(10).saturating_add(i64::from(b - b'0'))
        });
    (if negative { -value } else { value }) as i16
}

fn cached_entries(file: &TagArchive) -> Option<Vec<CachedEntry>> {
    file.entry_tags()
        .iter()
        .map(|entry| {
            let mesh = file.find_tag_by_id(MESH, entry.tag_id)?;
            let header = file.read_tag_prefix(mesh, MESH_HEADER_SIZE).ok()?;
            let links = MeshLinks::from_header(&header)?;
            let file_name = text_bytes(&entry.internal_name);
            Some(CachedEntry {
                flags: links.flags,
                mesh: entry.tag_id,
                order: strtol_i16(&file_name),
                file_name,
                name: entry_display_name(entry),
                extra_flags: links.campaign_flags,
            })
        })
        .collect()
}

fn entry_display_name(entry: &SBEntryTag) -> Vec<u8> {
    let mut name = entry.display_name.to_vec();
    name.extend_from_slice(&entry.description);
    name.truncate(63);
    text_bytes(&name)
}

fn fix_foundation_entries(entries: &mut [CachedEntry]) {
    for entry in entries {
        if entry.order == 0 && entry.mesh == FourCC(*b"00tm") {
            entry.flags |= 0x90_0000;
        }
        if entry.mesh == FourCC(*b"84tf") && entry.name == b"Zatopione Kr\xf3lestwo" {
            entry.name = b"Zatopione Cesarstwo".to_vec();
        }
    }
}

fn fix_plugin_entries(entries: &mut [CachedEntry], plugin: &str) {
    for entry in entries {
        if entry.order == 301
            && entry.mesh == FourCC(*b"30tm")
            && plugin == "Magma - The Fallen Levels v2"
        {
            entry.flags |= 0x80_0000;
            entry.order = 300;
        }
        if plugin == "Magma - Wars of Reckoning (f)" {
            entry.flags |= 0x4000_0000;
        }
    }
}

fn drop_duplicates(entries: Vec<CachedEntry>) -> Vec<CachedEntry> {
    let is_duplicate = |i: usize, e: &CachedEntry| {
        entries
            .iter()
            .enumerate()
            .any(|(j, other)| i != j && (other.file_name == e.file_name || other.name == e.name))
    };
    entries
        .iter()
        .enumerate()
        .filter(|&(i, e)| !is_duplicate(i, e))
        .map(|(_, e)| e.clone())
        .collect()
}

pub struct PluginTags {
    file: TagArchive,
    pub name: String,
    pub checksum: i32,
    required: Vec<usize>,
    pub entries: Vec<CachedEntry>,
}

pub struct CampaignTags {
    base: Vec<TagArchive>,
    pub base_entries: Vec<CachedEntry>,
    pub plugins: Vec<PluginTags>,
}

impl CampaignTags {
    pub fn open(game_dir: &Path) -> Self {
        let vfs = mandrake_common::game_vfs();
        let open = |path: &Path| TagArchive::open(vfs, &path.to_string_lossy()).ok();
        let base = engine_tag_files(game_dir, &[])
            .paths
            .iter()
            .filter_map(|path| open(path))
            .collect();
        let plugins_dir = game_dir.join("plugins");
        let mut plugin_paths: Vec<_> = vfs
            .list_dir(&plugins_dir.to_string_lossy())
            .into_iter()
            .filter(|p| vfs.is_file(p))
            .map(std::path::PathBuf::from)
            .collect();
        plugin_paths.sort();
        let plugins = plugin_paths
            .iter()
            .filter_map(|path| Some((plugin_name(path)?, open(path)?)))
            .collect();
        Self::from_files(base, plugins)
    }

    pub fn from_files(base: Vec<TagArchive>, others: Vec<(String, TagArchive)>) -> Self {
        let mut base_entries: Vec<CachedEntry> = base
            .iter()
            .rev()
            .filter(|file| file.header().r#type == TYPE_FOUNDATION)
            .filter_map(cached_entries)
            .flatten()
            .collect();
        fix_foundation_entries(&mut base_entries);

        let mut plugins: Vec<PluginTags> = others
            .into_iter()
            .filter(|(_, file)| file.header().r#type == TYPE_PLUGIN)
            .filter_map(|(name, file)| PluginTags::new(name, file))
            .collect();
        let required: Vec<Vec<usize>> = plugins
            .iter()
            .map(|plugin| plugin.find_required(&plugins))
            .collect();
        for (plugin, required) in plugins.iter_mut().zip(required) {
            plugin.required = required;
        }
        Self {
            base,
            base_entries: drop_duplicates(base_entries),
            plugins,
        }
    }

    pub fn base_search(&self) -> TagSearch<'_> {
        TagSearch {
            files: self.base.iter().collect(),
        }
    }

    pub fn plugin_search(&self, index: usize) -> TagSearch<'_> {
        let mut search = self.base_search();
        if let Some(plugin) = self.plugins.get(index) {
            search
                .files
                .extend(plugin.required.iter().map(|&i| &self.plugins[i].file));
            search.files.push(&plugin.file);
        }
        search
    }

    pub fn plugins_search(&self, indices: &[usize]) -> TagSearch<'_> {
        let mut search = self.base_search();
        for &index in indices {
            let Some(plugin) = self.plugins.get(index) else {
                continue;
            };
            for &required in &plugin.required {
                let file = &self.plugins[required].file;
                if !search.files.iter().any(|f| std::ptr::eq(*f, file)) {
                    search.files.push(file);
                }
            }
            search.files.push(&plugin.file);
        }
        search
    }

    pub fn plugin_names(&self, index: usize) -> Vec<String> {
        let Some(plugin) = self.plugins.get(index) else {
            return Vec::new();
        };
        plugin
            .required
            .iter()
            .map(|&i| self.plugins[i].name.clone())
            .chain(std::iter::once(plugin.name.clone()))
            .collect()
    }
}

impl PluginTags {
    fn new(name: String, file: TagArchive) -> Option<Self> {
        let checksum = i32::from_be_bytes(file.header().checksum);
        if checksum == -1 {
            return None;
        }
        let mut entries = cached_entries(&file)?;
        fix_plugin_entries(&mut entries, &name);
        let entries = drop_duplicates(entries);
        Some(Self {
            file,
            name,
            checksum,
            required: Vec::new(),
            entries,
        })
    }

    fn required_name(&self) -> Option<String> {
        let header = self.file.header();
        if let Some((_, required)) = REQUIRED_PLUGINS.iter().find(|(p, _)| *p == self.name) {
            return Some((*required).to_owned());
        }
        if header.version == u16::MAX || header.version & PLUGIN_FLAG_NAMES_REQUIRED == 0 {
            return None;
        }
        let raw_description = crate::parse::until_nul(&header.description);
        (!crate::parse::is_plugin_url_description(raw_description))
            .then(|| header.description_as_str().into_owned())
    }

    fn find_required(&self, plugins: &[PluginTags]) -> Vec<usize> {
        self.required_name()
            .and_then(|required| plugins.iter().position(|p| p.name == required))
            .into_iter()
            .collect()
    }
}

pub struct TagSearch<'a> {
    files: Vec<&'a TagArchive>,
}

impl<'a> TagSearch<'a> {
    fn find(&self, group: FourCC, id: FourCC) -> Option<(&'a TagArchive, &'a TagEntry)> {
        self.files
            .iter()
            .rev()
            .find_map(|file| Some((*file, file.find_tag_by_id(group, id)?)))
    }

    pub fn has(&self, group: FourCC, id: FourCC) -> bool {
        self.find(group, id).is_some()
    }

    pub fn get(&self, group: FourCC, id: FourCC) -> Option<Vec<u8>> {
        let (file, entry) = self.find(group, id)?;
        file.get_tag_data(entry).ok()
    }

    pub fn mesh_header(&self, mesh: FourCC) -> Option<(MeshLinks, i16)> {
        let (file, entry) = self.find(MESH, mesh)?;
        let header = file.read_tag_prefix(entry, MESH_HEADER_SIZE).ok()?;
        Some((
            MeshLinks::from_header(&header)?,
            i16::from(entry.header.tag_type_idx),
        ))
    }

    pub fn mesh_header_bytes(&self, mesh: FourCC) -> Option<Vec<u8>> {
        let (file, entry) = self.find(MESH, mesh)?;
        file.read_tag_prefix(entry, MESH_HEADER_SIZE).ok()
    }

    pub fn sequence_count(&self, collection: FourCC) -> Option<i32> {
        let (file, entry) = self.find(COLLECTION, collection)?;
        let header = file.read_tag_prefix(entry, 0x84).ok()?;
        let count = header.get(0x80..0x84)?;
        Some(i32::from_be_bytes([count[0], count[1], count[2], count[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{SBMonoHeader, SBTagHeader, encode_c_string};

    fn tag(s: &[u8; 4]) -> FourCC {
        FourCC(*s)
    }

    fn mesh_header(flags: u32, campaign_flags: u32) -> Vec<u8> {
        let mut h = vec![0xff; MESH_HEADER_SIZE];
        h[0x4c..0x50].copy_from_slice(&flags.to_be_bytes());
        h[0x224..0x228].copy_from_slice(&campaign_flags.to_be_bytes());
        h
    }

    fn monolith(
        file_type: i16,
        version: u16,
        name: &str,
        description: &str,
        checksum: i32,
        entries: &[(&[u8; 4], &str, &str)],
        tags: &[(&[u8; 4], &[u8; 4], Vec<u8>)],
    ) -> Vec<u8> {
        let header = SBMonoHeader {
            r#type: file_type,
            version,
            name: encode_c_string(name),
            description: encode_c_string(description),
            entry_tag_count: entries.len() as u16,
            tag_list_count: tags.len() as u16,
            checksum: checksum.to_be_bytes(),
            flags: 1,
            size: 0,
            header_checksum: [0; 4],
            padding: [0; 4],
            signature: SBMonoHeader::SIG_DNG2,
        };
        let mut out = Vec::new();
        header.write(&mut out).unwrap();
        for (mesh, file_name, shown) in entries {
            let shown = shown.as_bytes();
            let mut display_name = [0u8; 32];
            let mut description = [0u8; 32];
            for (i, &b) in shown.iter().enumerate() {
                if i < 32 {
                    display_name[i] = b;
                } else {
                    description[i - 32] = b;
                }
            }
            SBEntryTag {
                tag_id: tag(mesh),
                reserved1: [0; 12],
                internal_name: encode_c_string(file_name),
                display_name,
                description,
            }
            .write(&mut out)
            .unwrap();
        }
        let mut offset =
            SBMonoHeader::SIZE + entries.len() * SBEntryTag::SIZE + tags.len() * SBTagHeader::SIZE;
        for (group, id, data) in tags {
            SBTagHeader {
                identifier: 0,
                flags: 0,
                tag_type_idx: file_type as i8,
                name: encode_c_string("a tag"),
                tag_type: tag(group),
                tag_id: tag(id),
                tag_data_offset: offset as i32,
                tag_data_size: data.len() as i32,
                user_data: 0,
                version: 1,
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
        out
    }

    fn archive(bytes: Vec<u8>) -> TagArchive {
        TagArchive::from_bytes(bytes).unwrap()
    }

    #[test]
    fn mesh_links_read_their_offsets_and_empty_tags_are_none() {
        let mut h = mesh_header(0x0111_0000, 3);
        for (offset, id) in [
            (0x8c, b"name"),
            (0x90, b"post"),
            (0x94, b"pre "),
            (0x9c, b"next"),
            (0xa0, b"altn"),
            (0xb0, b"text"),
            (0xfc, b"caps"),
            (0x100, b"narr"),
            (0x104, b"wins"),
            (0x108, b"loss"),
            (0x1ec, b"hint"),
        ] {
            h[offset..offset + 4].copy_from_slice(id);
        }
        let links = MeshLinks::from_header(&h).unwrap();
        assert_eq!(links.flags, 0x0111_0000);
        assert_eq!(links.campaign_flags, 3);
        assert_eq!(links.strings, Some(tag(b"name")));
        assert_eq!(links.post_art, Some(tag(b"post")));
        assert_eq!(links.pre_art, Some(tag(b"pre ")));
        assert_eq!(links.next, Some(tag(b"next")));
        assert_eq!(links.alternate, Some(tag(b"altn")));
        assert_eq!(links.briefing_text, Some(tag(b"text")));
        assert_eq!(links.captions, Some(tag(b"caps")));
        assert_eq!(links.narration, Some(tag(b"narr")));
        assert_eq!(links.ambient_win, Some(tag(b"wins")));
        assert_eq!(links.ambient_loss, Some(tag(b"loss")));
        assert_eq!(links.hints, Some(tag(b"hint")));

        h[0x90..0x94].copy_from_slice(&[0; 4]);
        h[0x94..0x98].copy_from_slice(&[0xff; 4]);
        let links = MeshLinks::from_header(&h).unwrap();
        assert_eq!((links.post_art, links.pre_art), (None, None));
        assert_eq!(MeshLinks::from_header(&h[..0x3ff]), None);
    }

    #[test]
    fn string_lists_split_as_the_engine_does() {
        let lines = string_list_lines(b"one\r\ntwo\r\r\n\xd5three\0four\r");
        assert_eq!(lines, [&b"one"[..], b"two", b"\xd5three", b"four"]);
        assert!(string_list_lines(b"").is_empty());
        assert_eq!(text_bytes(b"line\rline\0rest"), b"line\rline");
    }

    #[test]
    fn order_is_strtol_of_the_file_name() {
        assert_eq!(strtol_i16(b"01 village"), 1);
        assert_eq!(strtol_i16(b"04a hunting party"), 4);
        assert_eq!(strtol_i16(b"  -7 x"), -7);
        assert_eq!(strtol_i16(b"Ebb Tide"), 0);
        assert_eq!(strtol_i16(b"101 ch01"), 101);
    }

    #[test]
    fn entries_take_their_fields_from_the_mesh_in_their_own_file() {
        let foundation = archive(monolith(
            3,
            0,
            "small install",
            "",
            5,
            &[
                (b"00tm", "00 Training Map", "Tutorial"),
                (b"l1  ", "01 village", "Willow Creek"),
                (
                    b"long",
                    "02 long",
                    "A name of more than thirty-two bytes in all",
                ),
            ],
            &[
                (b"mesh", b"00tm", mesh_header(0x0121_0000, 0)),
                (b"mesh", b"l1  ", mesh_header(0x0111_0000, 8)),
                (b"mesh", b"long", mesh_header(0, 0)),
            ],
        ));
        let patch = archive(monolith(
            2,
            2,
            "Patch 1.2",
            "",
            6,
            &[(b"nope", "03 patch", "Not listed")],
            &[
                (b"mesh", b"l1  ", mesh_header(0, 0)),
                (b"mesh", b"nope", mesh_header(0, 0)),
            ],
        ));
        let tags = CampaignTags::from_files(vec![foundation, patch], Vec::new());
        let entries = &tags.base_entries;
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].flags, 0x01b1_0000);
        assert_eq!(entries[0].order, 0);
        assert_eq!(entries[1].mesh, tag(b"l1  "));
        assert_eq!(entries[1].flags, 0x0111_0000);
        assert_eq!(entries[1].order, 1);
        assert_eq!(entries[1].extra_flags, 8);
        assert_eq!(entries[1].name, b"Willow Creek");
        assert_eq!(entries[1].file_name, b"01 village");
        assert_eq!(
            entries[2].name,
            b"A name of more than thirty-two bytes in all"
        );
        let (links, file_type) = tags.base_search().mesh_header(tag(b"l1  ")).unwrap();
        assert_eq!(links.flags, 0);
        assert_eq!(file_type, 2);
    }

    #[test]
    fn duplicates_in_a_set_are_all_dropped() {
        let foundation = archive(monolith(
            3,
            0,
            "small install",
            "",
            5,
            &[
                (b"aaaa", "01 same", "First"),
                (b"bbbb", "01 same", "Second"),
                (b"cccc", "02 c", "Shown twice"),
                (b"dddd", "03 d", "Shown twice"),
                (b"eeee", "04 e", "Alone"),
            ],
            &[
                (b"mesh", b"aaaa", mesh_header(0, 0)),
                (b"mesh", b"bbbb", mesh_header(0, 0)),
                (b"mesh", b"cccc", mesh_header(0, 0)),
                (b"mesh", b"dddd", mesh_header(0, 0)),
                (b"mesh", b"eeee", mesh_header(0, 0)),
            ],
        ));
        let tags = CampaignTags::from_files(vec![foundation], Vec::new());
        let meshes: Vec<FourCC> = tags.base_entries.iter().map(|e| e.mesh).collect();
        assert_eq!(meshes, [tag(b"eeee")]);
    }

    #[test]
    fn plugins_need_their_meshes_and_a_checksum_and_can_name_what_they_need() {
        let level = |id: &'static [u8; 4]| (b"mesh", id, mesh_header(0x11_0000, 0));
        let needs = archive(monolith(
            1,
            4,
            "Levels",
            "Tagset",
            7,
            &[(b"lvl1", "01 one", "One")],
            &[level(b"lvl1")],
        ));
        let tagset = archive(monolith(1, u16::MAX, "Tagset v1", "Something", 8, &[], &[]));
        let web = archive(monolith(
            1,
            4,
            "Web",
            "www.example.com",
            9,
            &[(b"web1", "01 web", "Web")],
            &[level(b"web1")],
        ));
        let no_mesh = archive(monolith(
            1,
            0,
            "No mesh",
            "",
            10,
            &[(b"gone", "01 gone", "Gone")],
            &[],
        ));
        let no_checksum = archive(monolith(1, 0, "No checksum", "", -1, &[], &[]));
        let not_a_plugin = archive(monolith(5, 0, "Interface", "", 11, &[], &[]));
        let named = |name: &str, file| (name.to_owned(), file);
        let tags = CampaignTags::from_files(
            Vec::new(),
            vec![
                named("Levels", needs),
                named("Tagset", tagset),
                named("Web", web),
                named("No mesh", no_mesh),
                named("No checksum", no_checksum),
                named("Interface", not_a_plugin),
            ],
        );
        let names: Vec<&str> = tags.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Levels", "Tagset", "Web"]);
        assert_eq!(tags.plugin_names(0), ["Tagset", "Levels"]);
        assert_eq!(tags.plugin_names(2), ["Web"]);
        assert_eq!(tags.plugins[0].checksum, 7);
    }

    #[test]
    fn a_plugin_is_named_by_its_file_not_its_header() {
        let level = |id: &'static [u8; 4]| (b"mesh", id, mesh_header(0x11_0000, 0));
        let pack = archive(monolith(
            1,
            0xff05,
            "TSG Level-Pack v1",
            "http://www.flyingflip.com",
            7,
            &[(b"sg00", "00 sg", "Sg")],
            &[level(b"sg00")],
        ));
        let tagset = archive(monolith(1, u16::MAX, "TSG Tagset v1", "", 8, &[], &[]));
        let tags = CampaignTags::from_files(
            Vec::new(),
            vec![
                ("TSG Level-Pack v1.4".to_owned(), pack),
                ("TSG Tagset v1.4".to_owned(), tagset),
            ],
        );
        assert_eq!(tags.plugins[0].name, "TSG Level-Pack v1.4");
        assert_eq!(
            tags.plugin_names(0),
            ["TSG Tagset v1.4", "TSG Level-Pack v1.4"]
        );
    }

    #[test]
    fn a_plugins_tags_go_over_the_installs_only_in_its_own_search() {
        let foundation = archive(monolith(
            3,
            0,
            "small install",
            "",
            5,
            &[],
            &[(b"stli", b"both", b"base".to_vec())],
        ));
        let plugin = archive(monolith(
            1,
            0,
            "Plugin",
            "",
            7,
            &[],
            &[
                (b"stli", b"both", b"plugin".to_vec()),
                (b".256", b"##s#", {
                    let mut c = vec![0; 0x84];
                    c[0x83] = 4;
                    c
                }),
            ],
        ));
        let tags = CampaignTags::from_files(vec![foundation], vec![("Plugin".to_owned(), plugin)]);
        let stli = tag(b"stli");
        assert_eq!(tags.base_search().get(stli, tag(b"both")).unwrap(), b"base");
        assert_eq!(
            tags.plugin_search(0).get(stli, tag(b"both")).unwrap(),
            b"plugin"
        );
        assert!(!tags.base_search().has(COLLECTION, tag(b"##s#")));
        assert_eq!(tags.plugin_search(0).sequence_count(tag(b"##s#")), Some(4));
    }

    fn snapshot(dir: &Path) -> Vec<(std::path::PathBuf, Vec<u8>, std::time::SystemTime)> {
        let mut seen = Vec::new();
        let mut walk = vec![dir.to_path_buf()];
        while let Some(dir) = walk.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk.push(path);
                } else {
                    let modified = path.metadata().unwrap().modified().unwrap();
                    seen.push((path.clone(), std::fs::read(&path).unwrap(), modified));
                }
            }
        }
        seen.sort();
        seen
    }

    #[cfg(target_os = "linux")]
    fn open_modes_under(dir: &Path) -> Vec<u32> {
        let mut modes = Vec::new();
        for fd in std::fs::read_dir("/proc/self/fd").unwrap().flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            if !target.starts_with(dir) {
                continue;
            }
            let info = format!("/proc/self/fdinfo/{}", fd.file_name().to_string_lossy());
            let info = std::fs::read_to_string(info).unwrap();
            let flags = info
                .lines()
                .find_map(|line| line.strip_prefix("flags:"))
                .unwrap()
                .trim();
            modes.push(u32::from_str_radix(flags, 8).unwrap() & 0o3);
        }
        modes
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_game_directory_is_opened_read_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("m2-mesh-links-test-{}", std::process::id()));
        let tags_dir = dir.join("tags");
        let plugins_dir = dir.join("plugins");
        std::fs::create_dir_all(&tags_dir).unwrap();
        std::fs::create_dir_all(&plugins_dir).unwrap();
        std::fs::write(
            tags_dir.join("small install"),
            monolith(
                3,
                0,
                "small install",
                "",
                5,
                &[(b"l1  ", "01 village", "Willow Creek")],
                &[(b"mesh", b"l1  ", mesh_header(0x11_0000, 0))],
            ),
        )
        .unwrap();
        std::fs::write(
            plugins_dir.join("A plugin"),
            monolith(
                1,
                0,
                "A plugin",
                "",
                7,
                &[(b"lvl1", "01 one", "One")],
                &[(b"mesh", b"lvl1", mesh_header(0x11_0000, 0))],
            ),
        )
        .unwrap();
        let set_mode = |path: &Path, mode| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        for path in [&tags_dir, &plugins_dir, &dir] {
            set_mode(path, 0o555);
        }
        let before = snapshot(&dir);

        let tags = CampaignTags::open(&dir);
        assert_eq!(tags.base_entries.len(), 1);
        assert_eq!(tags.plugins.len(), 1);
        let modes = open_modes_under(&std::fs::canonicalize(&dir).unwrap());
        assert_eq!(modes, [0, 0], "the two files, both O_RDONLY");
        drop(tags);
        assert_eq!(snapshot(&dir), before);

        for path in [&dir, &tags_dir, &plugins_dir] {
            set_mode(path, 0o755);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
