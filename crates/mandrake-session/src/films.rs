use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_tag::mesh_links::{CampaignTags, TagSearch, string_list_lines};

use crate::text::Text;

pub const RECORDING: FourCC = FourCC(*b"reco");
pub const FOLDER: &str = "recordings";
const LAST_RECORDING_DEFAULT: &[u8] = b"Last Recording";
const UNTITLED_DEFAULT: &[u8] = b"Untitled Replay";
const UNTITLED_STRING: usize = 2;
const NEW_SAVED_GAME_DEFAULT: &[u8] = b"New Saved Game";
const NEW_SAVED_GAME_STRING: usize = 1;
const LAST_RECORDING_NAME: FourCC = FourCC(*b"inrf");
const EXTENSION: &[u8] = b".m2rec";
const NEWEST_VERSION: i16 = 3;
pub const HEADER_SIZE: usize = 0xa28;
const NEW_GAME_DATA: usize = 0x24;
const TAG_HEADER_SIZE: usize = 64;
const NAME_BYTES: usize = 0x1f;
const MESH_STARTING_LOCATIONS: usize = 0x54;
const MESH_PRE_ART: usize = 0x94;
const PRE_ART_SEQUENCE: i32 = 1;
const COLLECTION: FourCC = FourCC(*b".256");
const STRINGS: FourCC = FourCC(*b"stli");

#[derive(Debug, Clone, PartialEq)]
pub struct FilmFile {
    pub path: PathBuf,
    pub file_name: Vec<u8>,
    pub name: Text,
    pub game: FilmGame,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilmGame {
    pub mode: i16,
    pub game_type: i16,
    pub options: u32,
    pub time_limit: i32,
    pub mesh: FourCC,
    pub difficulty: i16,
    pub captain: i16,
    pub planning_time: i32,
    pub flags: u16,
    pub build_number: i16,
    pub version_id: i32,
    pub plugins: Vec<FilmPlugin>,
    pub gameplay_version: i16,
    pub version_of_id: i16,
    pub map_name: Option<Text>,
    pub plugins_missing: bool,
    pub plugins_present: bool,
    pub art: Option<FourCC>,
    pub art_plugins: Vec<String>,
    pub starting_locations: i32,
}

impl Default for FilmGame {
    fn default() -> Self {
        Self {
            mode: 0,
            game_type: 0,
            options: 0,
            time_limit: 0,
            mesh: FourCC([0; 4]),
            difficulty: 0,
            captain: -1,
            planning_time: 0,
            flags: 0,
            build_number: 0,
            version_id: 0,
            plugins: Vec::new(),
            gameplay_version: 0,
            version_of_id: -1,
            map_name: None,
            plugins_missing: false,
            plugins_present: true,
            art: None,
            art_plugins: Vec::new(),
            starting_locations: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilmPlugin {
    pub name: String,
    pub checksum: i32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilmFolder {
    pub films: Vec<FilmFile>,
    pub last: Option<FilmFile>,
    pub any: bool,
    pub last_exists: bool,
    pub names: Vec<Text>,
    pub file_names: Vec<Vec<u8>>,
    pub last_name: Text,
    pub untitled: Text,
}

fn be16(d: &[u8], o: usize) -> i16 {
    i16::from_be_bytes([d[o], d[o + 1]])
}

fn be32(d: &[u8], o: usize) -> i32 {
    i32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

pub fn is_recording(file: &[u8]) -> bool {
    file.get(36..40) == Some(&RECORDING.0[..])
}

pub fn read_header(file: &[u8]) -> Option<&[u8]> {
    if !is_recording(file) || file.len() < TAG_HEADER_SIZE {
        return None;
    }
    let offset = mandrake_rec::header::data_offset(file);
    let header = file.get(offset..offset + HEADER_SIZE)?;
    (be16(header, 0) <= NEWEST_VERSION).then_some(header)
}

pub fn parse_game(header: &[u8]) -> FilmGame {
    let d = &header[NEW_GAME_DATA..];
    let mut game = FilmGame {
        mode: be16(d, 0x00),
        game_type: be16(d, 0x02),
        options: be32(d, 0x04) as u32,
        time_limit: be32(d, 0x08),
        mesh: FourCC([d[0x0c], d[0x0d], d[0x0e], d[0x0f]]),
        difficulty: be16(d, 0x10),
        captain: be16(d, 0x16),
        planning_time: be32(d, 0x1c),
        flags: be16(d, 0x20) as u16,
        build_number: be16(d, 0x22),
        version_id: be32(d, 0x24),
        plugins: decode_plugins(&d[0x2c..0x22c]),
        ..FilmGame::default()
    };
    game.gameplay_version = compute_gameplay_version(game.options, game.version_id, -1);
    game.version_of_id = gameplay_version_from_version_id(game.version_id, game.build_number);
    game
}

fn decode_plugins(block: &[u8]) -> Vec<FilmPlugin> {
    let count = be16(block, 0);
    if !(1..=16).contains(&count) {
        return Vec::new();
    }
    let mut at = 2;
    let string = |at: &mut usize, max: usize| -> Option<Vec<u8>> {
        let rest = block.get(*at..)?;
        let len = rest.iter().take(max + 1).position(|&b| b == 0)?;
        let s = rest[..len].to_vec();
        *at += len + 1;
        Some(s)
    };
    let mut plugins = Vec::new();
    for _ in 0..count {
        let Some(word) = block.get(at..at + 4) else {
            break;
        };
        let checksum = be32(word, 0);
        at += 4;
        let (Some(name), Some(_url)) = (string(&mut at, 0x1f), string(&mut at, 0x3f)) else {
            break;
        };
        plugins.push(FilmPlugin {
            name: mandrake_rec::mac_roman::decode(&name),
            checksum,
        });
    }
    plugins
}

pub use mandrake_net::gameplay::{
    compute_gameplay_version, gameplay_version_from_version_id, gameplay_version_to_string,
};

pub fn player_names(header: &[u8]) -> Vec<(Text, Text)> {
    let d = &header[NEW_GAME_DATA..];
    let field = |at: usize| {
        let raw = &d[at..at + 0x20];
        Text(raw[..raw.iter().position(|&b| b == 0).unwrap_or(raw.len())].to_vec())
    };
    game_player_order(header)
        .into_iter()
        .map(|i| {
            let entry = 0x230 + i * 0x7c;
            (field(entry + 0x2c), field(entry + 0x4c))
        })
        .collect()
}

pub fn game_player_order(header: &[u8]) -> Vec<usize> {
    let d = &header[NEW_GAME_DATA..];
    let count = usize::try_from(be16(d, 0x22c)).unwrap_or(0).min(16);
    let options = be32(d, 0x04) as u32;
    let gameplay = compute_gameplay_version(options, be32(d, 0x24), -1);
    let entry = |i: usize| 0x230 + i * 0x7c;
    let observer = |i: usize| be16(d, entry(i) + 0x04) as u16 & 0x40 != 0;
    let follow = |i: usize| be32(d, entry(i) + 0x08);
    let first = (0..count)
        .find(|&i| observer(i))
        .map(|i| be32(d, entry(i) + 0x0c));
    let mut followers_stay = false;
    if let Some(id) = first
        && count > 3
        && i32::from(gameplay) > game_options::GAMEPLAY_1_5
        && options & game_options::COOPERATIVE != 0
    {
        let moved = (0..count)
            .filter(|&i| observer(i) || follow(i) == id)
            .count();
        followers_stay = moved == count - 2;
    }
    let moves = |i: usize| observer(i) || (!followers_stay && first == Some(follow(i)));
    let (mut order, rest): (Vec<usize>, Vec<usize>) = (0..count).partition(|&i| !moves(i));
    order.extend(rest);
    order
}

pub fn csstrcmp(a: &[u8], b: &[u8]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    let number = |s: &[u8], at: &mut usize| -> i32 {
        let mut n: i32 = 0;
        while let Some(&c) = s.get(*at).filter(|c| c.is_ascii_digit()) {
            n = n.wrapping_mul(10).wrapping_add(i32::from(c - b'0'));
            *at += 1;
        }
        n.wrapping_add(0x100)
    };
    let fold = |c: u8| -> i32 {
        let c = c.to_ascii_lowercase();
        i32::from(c as i8)
    };
    loop {
        let (Some(&x), y) = (a.get(i), b.get(j)) else {
            return if b.get(j).is_some() {
                Ordering::Less
            } else {
                Ordering::Equal
            };
        };
        let Some(&y) = y else {
            return Ordering::Greater;
        };
        let left = if x.is_ascii_digit() {
            number(a, &mut i)
        } else {
            i += 1;
            fold(x)
        };
        let right = if y.is_ascii_digit() {
            number(b, &mut j)
        } else {
            j += 1;
            fold(y)
        };
        match left.cmp(&right) {
            Ordering::Equal => {}
            other => return other,
        }
    }
}

pub fn display_name(file_name: &[u8]) -> Vec<u8> {
    let n = file_name.len();
    if n > EXTENSION.len() && file_name[n - EXTENSION.len()..].eq_ignore_ascii_case(EXTENSION) {
        file_name[..n - EXTENSION.len()].to_vec()
    } else {
        file_name.to_vec()
    }
}

pub struct FilmTags<'a> {
    tags: &'a CampaignTags,
}

impl<'a> FilmTags<'a> {
    pub const fn new(tags: &'a CampaignTags) -> Self {
        Self { tags }
    }

    fn installed(&self, plugins: &[FilmPlugin]) -> Vec<Option<usize>> {
        plugins
            .iter()
            .map(|p| {
                self.tags.plugins.iter().position(|installed| {
                    installed.checksum == p.checksum && installed.name.eq_ignore_ascii_case(&p.name)
                })
            })
            .collect()
    }

    fn entry_name(&self, mesh: FourCC, plugins: &[usize]) -> Option<Text> {
        for &index in plugins {
            if let Some(e) = self.tags.plugins[index]
                .entries
                .iter()
                .find(|e| e.mesh == mesh)
            {
                return Some(Text(e.name.clone()));
            }
        }
        self.tags
            .base_entries
            .iter()
            .find(|e| e.mesh == mesh)
            .map(|e| Text(e.name.clone()))
    }

    pub fn look_up(&self, game: &mut FilmGame) {
        let installed = self.installed(&game.plugins);
        let found: Vec<usize> = installed.iter().flatten().copied().collect();
        game.plugins_missing = installed.iter().any(Option::is_none);
        game.plugins_present = !game.plugins_missing;
        game.map_name = self.entry_name(game.mesh, &found);
        let search = self.tags.plugins_search(&found);
        game.art_plugins = found
            .iter()
            .map(|&i| self.tags.plugins[i].name.clone())
            .collect();
        if let Some(header) = search.mesh_header_bytes(game.mesh) {
            let (starting_locations, art) = parse_mesh_film_info(&header);
            if let Some(locs) = starting_locations {
                game.starting_locations = locs;
            }
            game.art = art.filter(|&art| has_pre_art(&search, art));
        }
    }

    pub fn last_recording_name(&self) -> Vec<u8> {
        self.inrf_string(0)
            .unwrap_or_else(|| LAST_RECORDING_DEFAULT.to_vec())
    }

    pub fn untitled_name(&self) -> Vec<u8> {
        self.inrf_string(UNTITLED_STRING)
            .unwrap_or_else(|| UNTITLED_DEFAULT.to_vec())
    }

    pub fn new_saved_game_name(&self) -> Vec<u8> {
        self.inrf_string(NEW_SAVED_GAME_STRING)
            .unwrap_or_else(|| NEW_SAVED_GAME_DEFAULT.to_vec())
    }

    fn inrf_string(&self, index: usize) -> Option<Vec<u8>> {
        self.tags
            .base_search()
            .get(STRINGS, LAST_RECORDING_NAME)
            .and_then(|d| string_list_lines(&d).into_iter().nth(index))
    }
}

fn parse_mesh_film_info(header: &[u8]) -> (Option<i32>, Option<FourCC>) {
    let starting_locations = header
        .get(MESH_STARTING_LOCATIONS..MESH_STARTING_LOCATIONS + 4)
        .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let art = header
        .get(MESH_PRE_ART..MESH_PRE_ART + 4)
        .map(|b| FourCC([b[0], b[1], b[2], b[3]]));
    (starting_locations, art)
}

fn has_pre_art(search: &TagSearch<'_>, art: FourCC) -> bool {
    search.has(COLLECTION, art)
        && search
            .sequence_count(art)
            .is_some_and(|n| n > PRE_ART_SEQUENCE)
}

fn read_prefix(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match std::io::Read::read(file, &mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

fn read_header_from_open(
    file: &mut std::fs::File,
    head: &[u8; TAG_HEADER_SIZE],
) -> Option<[u8; HEADER_SIZE]> {
    if !is_recording(head) {
        return None;
    }
    let offset = mandrake_rec::header::data_offset(head);
    if offset != TAG_HEADER_SIZE {
        std::io::Seek::seek(file, std::io::SeekFrom::Start(offset as u64)).ok()?;
    }
    let mut header = [0u8; HEADER_SIZE];
    std::io::Read::read_exact(file, &mut header).ok()?;
    (be16(&header, 0) <= NEWEST_VERSION).then_some(header)
}

fn film_from_header(
    path: &Path,
    file_name: Vec<u8>,
    header: &[u8; HEADER_SIZE],
    tags: &FilmTags<'_>,
) -> FilmFile {
    let mut game = parse_game(header);
    tags.look_up(&mut game);
    let mut name = display_name(&file_name);
    name.truncate(0x400);
    FilmFile {
        path: path.to_path_buf(),
        name: Text(name),
        file_name,
        game,
    }
}

pub fn read_film(path: &Path, tags: &FilmTags<'_>) -> Option<FilmFile> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; TAG_HEADER_SIZE];
    std::io::Read::read_exact(&mut file, &mut head).ok()?;
    let header = read_header_from_open(&mut file, &head)?;
    let file_name = path.file_name()?.to_string_lossy().as_bytes().to_vec();
    Some(film_from_header(path, file_name, &header, tags))
}

pub fn short_name(name: &Text) -> Text {
    Text(name.0[..name.0.len().min(NAME_BYTES)].to_vec())
}

pub fn list_folder(game_dir: &Path, tags: &CampaignTags) -> FilmFolder {
    let tags = FilmTags::new(tags);
    let folder = game_dir.join(FOLDER);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map(|dir| {
            dir.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && !is_being_written(p))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    let mut any = false;
    let mut films = Vec::new();
    let mut names: Vec<(Vec<u8>, Text)> = Vec::new();
    for path in &paths {
        let Ok(mut file) = std::fs::File::open(path) else {
            continue;
        };
        let mut head = [0u8; TAG_HEADER_SIZE];
        let Ok(read) = read_prefix(&mut file, &mut head) else {
            continue;
        };
        if !is_recording(&head[..read]) {
            continue;
        }
        any = true;
        if read < TAG_HEADER_SIZE {
            continue;
        }
        let Some(file_name) = path
            .file_name()
            .map(|n| n.to_string_lossy().as_bytes().to_vec())
        else {
            continue;
        };
        names.push((file_name.clone(), Text(display_name(&file_name))));
        if let Some(header) = read_header_from_open(&mut file, &head) {
            films.push(film_from_header(path, file_name, &header, &tags));
        }
    }
    films.sort_by(|a, b| csstrcmp(&a.file_name, &b.file_name));
    names.sort_by(|a, b| csstrcmp(&a.0, &b.0));
    let last_name = tags.last_recording_name();
    let last = String::from_utf8(last_name.clone())
        .ok()
        .and_then(|_| {
            films
                .iter()
                .find(|f| f.file_name == last_name)
                .cloned()
                .or_else(|| {
                    read_film(
                        &folder.join(String::from_utf8_lossy(&last_name).as_ref()),
                        &tags,
                    )
                })
        })
        .filter(|f| f.game.map_name.is_some());
    let last_exists = folder
        .join(String::from_utf8_lossy(&last_name).as_ref())
        .is_file();
    FilmFolder {
        films,
        last,
        any,
        last_exists,
        file_names: names.iter().map(|(f, _)| f.clone()).collect(),
        names: names.into_iter().map(|(_, n)| n).collect(),
        last_name: Text(last_name),
        untitled: Text(tags.untitled_name()),
    }
}

struct RecordingTag {
    file_name: Vec<u8>,
    tag_name: Vec<u8>,
    subgroup: FourCC,
}

fn recordings(folder: &Path) -> Vec<RecordingTag> {
    let Ok(dir) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && !is_being_written(p))
        .filter_map(|p| {
            let mut head = [0u8; TAG_HEADER_SIZE];
            std::fs::File::open(&p)
                .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
                .ok()?;
            if !is_recording(&head) {
                return None;
            }
            let name = &head[4..36];
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            Some(RecordingTag {
                file_name: p.file_name()?.to_string_lossy().as_bytes().to_vec(),
                tag_name: name[..len].to_vec(),
                subgroup: FourCC([head[40], head[41], head[42], head[43]]),
            })
        })
        .collect()
}

pub fn film_name_ok(name: &[u8], last_name: &[u8]) -> bool {
    name != last_name
        && mandrake_rec::writer::film_names(name).is_some_and(|(_, _, changed)| !changed)
}

pub fn rename_name_ok(name: &[u8]) -> bool {
    mandrake_rec::writer::film_names(name).is_some_and(|(_, _, changed)| !changed)
}

pub fn found_tag_name(file_name: &[u8]) -> Text {
    let mut name = file_name[..file_name.len().min(NAME_BYTES)].to_vec();
    mandrake_rec::writer::sanitize_name(&mut name, NAME_BYTES);
    Text(name)
}

pub(crate) fn file_in_dir(dir: &Path, file_name: &[u8], what: &str) -> std::io::Result<PathBuf> {
    let name = String::from_utf8_lossy(file_name);
    let windows_drive =
        name.len() >= 2 && name.as_bytes()[0].is_ascii_alphabetic() && name.as_bytes()[1] == b':';
    if name.is_empty() || name.contains(['/', '\\', '\0']) || windows_drive {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a file of the {what} folder"),
        ));
    }
    let mut components = Path::new(name.as_ref()).components();
    let single_normal = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if !single_normal {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a file of the {what} folder"),
        ));
    }
    Ok(dir.join(name.as_ref()))
}

pub(crate) fn file_in(game_dir: &Path, folder: &str, file_name: &[u8]) -> std::io::Result<PathBuf> {
    file_in_dir(&game_dir.join(folder), file_name, folder)
}

fn in_folder(game_dir: &Path, file_name: &[u8]) -> std::io::Result<PathBuf> {
    file_in(game_dir, FOLDER, file_name)
}

pub fn delete_film(game_dir: &Path, file_name: &[u8]) -> std::io::Result<()> {
    std::fs::remove_file(in_folder(game_dir, file_name)?)
}

pub fn rename_film(game_dir: &Path, file_name: &[u8], name: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind};
    let old_path = in_folder(game_dir, file_name)?;
    let (tag_name, new_file, _) = mandrake_rec::writer::film_names(name)
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?;
    let same = new_file == file_name;
    if !same
        && recordings(&game_dir.join(FOLDER))
            .iter()
            .any(|t| t.file_name.eq_ignore_ascii_case(&new_file))
    {
        return Ok(file_name.to_vec());
    }
    let mut file = std::fs::read(&old_path)?;
    if file.len() < TAG_HEADER_SIZE {
        return Err(Error::new(ErrorKind::InvalidData, "no tag header"));
    }
    let path = if same {
        old_path
    } else {
        let new_path = in_folder(game_dir, &new_file)?;
        std::fs::rename(&old_path, &new_path)?;
        new_path
    };
    let lay_over = |header: &mut [u8], text: &[u8]| {
        let field = &mut header[4..4 + NAME_BYTES + 1];
        let len = text.len().min(NAME_BYTES);
        field[..len].copy_from_slice(&text[..len]);
        field[len] = 0;
    };
    let found = &file_name[..file_name.len().min(NAME_BYTES)];
    lay_over(&mut file, found);
    lay_over(&mut file, &found_tag_name(file_name).0);
    if !same {
        lay_over(&mut file, &tag_name);
    }
    file[2] = 0;
    let size = u32::try_from(file.len() - TAG_HEADER_SIZE).unwrap_or(u32::MAX);
    file[0x30..0x34].copy_from_slice(&size.to_be_bytes());
    let mut out = std::fs::OpenOptions::new().write(true).open(&path)?;
    std::io::Write::write_all(&mut out, &file[..TAG_HEADER_SIZE])?;
    Ok(if same { file_name.to_vec() } else { new_file })
}

pub fn save_last_film(game_dir: &Path, last_name: &[u8], name: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind};
    let folder = game_dir.join(FOLDER);
    let last_path = in_folder(game_dir, last_name)?;
    let last = std::fs::read(&last_path)?;
    let data_at = read_header(&last)
        .map(|_| mandrake_rec::header::data_offset(&last))
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "the last recording doesn't read"))?;
    let (tag_name, file_name, _) = mandrake_rec::writer::film_names(name)
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?;
    let out_path = in_folder(game_dir, &file_name)?;
    let tags = recordings(&folder);
    let subgroup = match tags.iter().find(|t| t.tag_name == tag_name) {
        Some(same) => same.subgroup,
        None => {
            let taken: Vec<FourCC> = tags.iter().map(|t| t.subgroup).collect();
            mandrake_rec::writer::subgroup_for_name(&tag_name, &taken)
        }
    };
    let file = mandrake_rec::writer::film_file(&tag_name, subgroup, &last[data_at..]);
    std::fs::write(out_path, file)?;
    Ok(file_name)
}

pub fn write_last_recording(
    game_dir: &Path,
    last_name: &[u8],
    data: &[u8],
    random: impl FnMut() -> u16,
) -> std::io::Result<PathBuf> {
    let subgroup = last_recording_subgroup(game_dir, last_name, random);
    write_recording(game_dir, last_name, subgroup, data)
}

pub fn last_recording_subgroup(
    game_dir: &Path,
    last_name: &[u8],
    random: impl FnMut() -> u16,
) -> FourCC {
    let taken: Vec<FourCC> = recordings(&game_dir.join(FOLDER))
        .into_iter()
        .filter(|t| t.tag_name != last_name)
        .map(|t| t.subgroup)
        .collect();
    mandrake_rec::writer::random_subgroup(random, &taken)
}

pub fn write_recording(
    game_dir: &Path,
    name: &[u8],
    subgroup: FourCC,
    data: &[u8],
) -> std::io::Result<PathBuf> {
    let path = in_folder(game_dir, name)?;
    let folder = game_dir.join(FOLDER);
    std::fs::create_dir_all(&folder)?;
    let file_name = String::from_utf8_lossy(name);
    let writing = file_in_dir(
        &folder,
        format!(".{file_name}{WRITING_SUFFIX}").as_bytes(),
        FOLDER,
    )?;
    std::fs::write(
        &writing,
        mandrake_rec::writer::film_file(name, subgroup, data),
    )?;
    std::fs::rename(&writing, &path)?;
    Ok(path)
}

pub const WRITING_SUFFIX: &str = ".writing";

fn is_being_written(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy())
        .is_some_and(|n| n.starts_with('.') && n.ends_with(WRITING_SUFFIX))
}

pub mod tfl {
    use std::path::{Path, PathBuf};

    use mandrake_common::FourCC;
    use mandrake_rec::tfl::TflFilmHeader;

    use super::{FilmFile, FilmFolder, FilmGame, csstrcmp};
    use crate::text::Text;

    pub const LAST_RECORDING_FILE: &[u8] = b"LAST RECORDING";
    const TFL_GAMEPLAY_VERSION: i16 = 1500;
    const SOLO_MODE: i16 = 2;
    const SOLO_GAME_TYPE: i16 = 13;
    const TFL_OPTIONS: u32 = 0xffff;
    const WRITING: [&str; 2] = [".tmp", super::WRITING_SUFFIX];
    const NAME_FIELD: usize = 0x20;
    const ID_FIELD: std::ops::Range<usize> = 0x24..0x28;

    #[derive(Debug, Clone)]
    pub struct TflFilmEntry {
        pub path: PathBuf,
        pub file_name: Vec<u8>,
        pub name: Vec<u8>,
        pub header: TflFilmHeader,
    }

    pub fn game_of(header: &TflFilmHeader) -> FilmGame {
        FilmGame {
            mode: header.mode,
            game_type: if header.mode == SOLO_MODE {
                SOLO_GAME_TYPE
            } else {
                header.game_type
            },
            options: header.game_options & TFL_OPTIONS,
            time_limit: header.time_limit as i32,
            mesh: header.mesh_tag_id,
            difficulty: header.difficulty,
            captain: -1,
            planning_time: header.planning_time as i32,
            gameplay_version: TFL_GAMEPLAY_VERSION,
            ..FilmGame::default()
        }
    }

    fn being_written(file_name: &str) -> bool {
        WRITING.iter().any(|s| file_name.ends_with(s))
    }

    pub fn films_in(dir: &Path) -> Vec<TflFilmEntry> {
        let vfs = mandrake_common::game_vfs();
        let mut paths: Vec<PathBuf> = vfs
            .list_dir(&dir.to_string_lossy())
            .into_iter()
            .filter(|p| vfs.is_file(p))
            .map(PathBuf::from)
            .collect();
        paths.sort();
        paths
            .into_iter()
            .filter_map(|path| {
                let file_name = path.file_name()?.to_string_lossy().into_owned();
                if being_written(&file_name) {
                    return None;
                }
                let bytes = vfs.read_file(&path.to_string_lossy()).ok()?;
                let (header, _) =
                    TflFilmHeader::read(&mut std::io::Cursor::new(&bytes[..])).ok()?;
                let end = bytes[..NAME_FIELD]
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(NAME_FIELD);
                Some(TflFilmEntry {
                    path,
                    file_name: file_name.into_bytes(),
                    name: bytes[..end].to_vec(),
                    header,
                })
            })
            .collect()
    }

    pub fn films_of(ours: Option<&Path>, install: &Path) -> Vec<TflFilmEntry> {
        let mut films = ours.map(films_in).unwrap_or_default();
        let own = films.len();
        for film in films_in(install) {
            if film.file_name.eq_ignore_ascii_case(LAST_RECORDING_FILE)
                || !films[..own]
                    .iter()
                    .any(|f| f.file_name.eq_ignore_ascii_case(&film.file_name))
            {
                films.push(film);
            }
        }
        films
    }

    pub fn list_folders(
        ours: Option<&Path>,
        install: &Path,
        untitled: Vec<u8>,
        look_up: impl Fn(&mut FilmGame),
    ) -> FilmFolder {
        let mut films: Vec<FilmFile> = films_of(ours, install)
            .into_iter()
            .map(|f| {
                let mut game = game_of(&f.header);
                look_up(&mut game);
                FilmFile {
                    path: f.path,
                    file_name: f.file_name,
                    name: Text(f.name),
                    game,
                }
            })
            .collect();
        films.sort_by(|a, b| csstrcmp(&a.file_name, &b.file_name));
        let last_file = films
            .iter()
            .find(|f| f.file_name.eq_ignore_ascii_case(LAST_RECORDING_FILE));
        let last_name =
            last_file.map_or_else(|| LAST_RECORDING_FILE.to_vec(), |f| f.file_name.clone());
        FilmFolder {
            last: last_file.filter(|f| f.game.map_name.is_some()).cloned(),
            last_exists: last_file.is_some(),
            any: !films.is_empty(),
            names: films.iter().map(|f| f.name.clone()).collect(),
            file_names: films.iter().map(|f| f.file_name.clone()).collect(),
            films,
            last_name: Text(last_name),
            untitled: Text(untitled),
        }
    }

    fn error(kind: std::io::ErrorKind, what: &str) -> std::io::Error {
        std::io::Error::new(kind, what.to_owned())
    }

    fn file_of(dir: &Path, file_name: &[u8]) -> std::io::Result<PathBuf> {
        super::file_in_dir(dir, file_name, "films")
    }

    fn retag(file: &mut [u8], name: &[u8], id: Option<FourCC>) {
        let field = &mut file[..NAME_FIELD];
        field.fill(0);
        let len = name.len().min(NAME_FIELD - 1);
        field[..len].copy_from_slice(&name[..len]);
        if let Some(id) = id {
            file[ID_FIELD].copy_from_slice(&id.0);
        }
    }

    pub fn save_last_film(
        ours: &Path,
        last: &Path,
        name: &[u8],
        taken: &[FourCC],
    ) -> std::io::Result<Vec<u8>> {
        use std::io::ErrorKind;
        let (tag_name, file_name, _) = mandrake_rec::writer::save_names(name)
            .ok_or_else(|| error(ErrorKind::InvalidInput, "no name"))?;
        if file_name.eq_ignore_ascii_case(LAST_RECORDING_FILE) {
            return Err(error(ErrorKind::InvalidInput, "the last recording's name"));
        }
        let mut file = std::fs::read(last)?;
        if !TflFilmHeader::is_tfl_film(&file) {
            return Err(error(ErrorKind::InvalidData, "not a TFL film"));
        }
        let same = films_in(ours)
            .into_iter()
            .find(|f| f.name == tag_name)
            .map(|f| f.header.tag_id);
        let id = same.unwrap_or_else(|| mandrake_rec::writer::subgroup_for_name(&tag_name, taken));
        retag(&mut file, &tag_name, Some(id));
        file[0x2c..0x30].copy_from_slice(&mandrake_rec::tfl::TFL_FILM_TAG_FLAGS);
        std::fs::create_dir_all(ours)?;
        let path = file_of(ours, &file_name)?;
        let writing = file_of(ours, &[&file_name[..], b".tmp"].concat())?;
        std::fs::write(&writing, &file)?;
        std::fs::rename(&writing, &path)?;
        Ok(file_name)
    }

    pub fn delete_film(ours: &Path, file_name: &[u8]) -> std::io::Result<()> {
        std::fs::remove_file(file_of(ours, file_name)?)
    }

    pub fn rename_film(
        ours: &Path,
        file_name: &[u8],
        name: &[u8],
        file_names: &[Vec<u8>],
    ) -> std::io::Result<Vec<u8>> {
        use std::io::ErrorKind;
        let old = file_of(ours, file_name)?;
        let (tag_name, new_file, _) = mandrake_rec::writer::save_names(name)
            .ok_or_else(|| error(ErrorKind::InvalidInput, "no name"))?;
        let same = new_file == file_name;
        if !same
            && (new_file.eq_ignore_ascii_case(LAST_RECORDING_FILE)
                || file_names.iter().any(|f| f.eq_ignore_ascii_case(&new_file)))
        {
            return Ok(file_name.to_vec());
        }
        let mut file = std::fs::read(&old)?;
        if !TflFilmHeader::is_tfl_film(&file) {
            return Err(error(ErrorKind::InvalidData, "not a TFL film"));
        }
        retag(&mut file, &tag_name, None);
        let path = file_of(ours, &new_file)?;
        if !same {
            std::fs::rename(&old, &path)?;
        }
        let mut out = std::fs::OpenOptions::new().write(true).open(&path)?;
        std::io::Write::write_all(&mut out, &file[..NAME_FIELD])?;
        Ok(new_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csstrcmp_folds_case_and_counts_numbers() {
        assert_eq!(csstrcmp(b"MDL.m2rec", b"level3.m2rec"), Ordering::Greater);
        assert_eq!(csstrcmp(b"Last Recording", b"level3"), Ordering::Less);
        assert_eq!(
            csstrcmp(b"a2", b"a10"),
            Ordering::Less,
            "numbers, not digits"
        );
        assert_eq!(
            csstrcmp(b"a1", b"az"),
            Ordering::Greater,
            "a number after any letter"
        );
        assert_eq!(csstrcmp(b"ABC", b"abc"), Ordering::Equal);
        assert_eq!(csstrcmp(b"ab", b"abc"), Ordering::Less);
        assert_eq!(
            csstrcmp(b"b_c", b"bagrada"),
            Ordering::Less,
            "'_' before 'a'"
        );
        assert_eq!(csstrcmp(b"broken_path", b"bagrada2025"), Ordering::Greater);
    }

    #[test]
    fn the_folder_sorts_as_the_engines_list() {
        let mut names: Vec<&[u8]> = vec![
            b"willow_vic_new.m2rec",
            b"MDL.m2rec",
            b"Last Recording",
            b"level3.m2rec",
            b"broken_path_victory.m2rec",
            b"salvation_def_new.m2rec",
            b"bagrada2025_04_27__18_45_17_24.m2rec",
        ];
        names.sort_by(|a, b| csstrcmp(a, b));
        let shown: Vec<Vec<u8>> = names.iter().map(|n| display_name(n)).collect();
        assert_eq!(
            shown,
            [
                &b"bagrada2025_04_27__18_45_17_24"[..],
                b"broken_path_victory",
                b"Last Recording",
                b"level3",
                b"MDL",
                b"salvation_def_new",
                b"willow_vic_new",
            ]
        );
    }

    #[test]
    fn versions_read_as_the_engine_names_them() {
        assert_eq!(compute_gameplay_version(0, 0x889, -1), 0x73c);
        assert_eq!(gameplay_version_to_string(0x73c), "1.8.5");
        assert_eq!(gameplay_version_to_string(0x726), "1.8.3");
        assert_eq!(gameplay_version_to_string(0x514), "1.3");
        assert_eq!(gameplay_version_from_version_id(0x889, 471), 1852);
        assert_eq!(gameplay_version_from_version_id(0x889, 466), 1851);
        assert_eq!(gameplay_version_from_version_id(1, 0), -1);
    }

    #[test]
    fn a_film_is_written_whole_or_not_at_all() {
        let game = std::env::temp_dir().join(format!("m2-films-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&game);
        let subgroup = FourCC(*b"abcd");
        write_recording(&game, b"Last Recording", subgroup, b"first").unwrap();
        let path = write_recording(&game, b"Last Recording", subgroup, b"second").unwrap();
        assert_eq!(&std::fs::read(&path).unwrap()[TAG_HEADER_SIZE..], b"second");
        let folder = game.join(FOLDER);
        let files = || std::fs::read_dir(&folder).unwrap().count();
        assert_eq!(files(), 1);
        let cut = folder.join(format!(".Last Recording{WRITING_SUFFIX}"));
        std::fs::write(
            &cut,
            mandrake_rec::writer::film_file(b"Last Recording", subgroup, b"cut"),
        )
        .unwrap();
        let listed: Vec<Vec<u8>> = recordings(&folder)
            .into_iter()
            .map(|t| t.file_name)
            .collect();
        assert_eq!(listed, [b"Last Recording".to_vec()]);
        let _ = std::fs::remove_dir_all(&game);
    }

    #[test]
    fn the_extension_comes_off_in_any_case() {
        assert_eq!(display_name(b"MDL.M2REC"), b"MDL");
        assert_eq!(display_name(b".m2rec"), b".m2rec");
        assert_eq!(display_name(b"Last Recording"), b"Last Recording");
    }

    #[test]
    fn parse_mesh_film_info_handles_truncated_headers() {
        assert_eq!(parse_mesh_film_info(&[]), (None, None));
        assert_eq!(
            parse_mesh_film_info(&[0u8; MESH_STARTING_LOCATIONS + 3]),
            (None, None)
        );
        let mut mid = vec![0u8; MESH_PRE_ART + 3];
        mid[MESH_STARTING_LOCATIONS..MESH_STARTING_LOCATIONS + 4]
            .copy_from_slice(&4i32.to_be_bytes());
        assert_eq!(parse_mesh_film_info(&mid), (Some(4), None));

        let mut full = vec![0u8; MESH_PRE_ART + 4];
        full[MESH_STARTING_LOCATIONS..MESH_STARTING_LOCATIONS + 4]
            .copy_from_slice(&6i32.to_be_bytes());
        full[MESH_PRE_ART..MESH_PRE_ART + 4].copy_from_slice(b"01pr");
        assert_eq!(
            parse_mesh_film_info(&full),
            (Some(6), Some(FourCC(*b"01pr")))
        );
    }

    #[test]
    fn list_folder_ignores_writing_temp_files() {
        let game =
            std::env::temp_dir().join(format!("m2-films-list-writing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&game);
        let folder = game.join(FOLDER);
        std::fs::create_dir_all(&folder).unwrap();
        let mut payload = vec![0u8; HEADER_SIZE];
        payload[0..2].copy_from_slice(&3i16.to_be_bytes());
        let cut = folder.join(format!(".Last Recording{WRITING_SUFFIX}"));
        std::fs::write(
            &cut,
            mandrake_rec::writer::film_file(b"Last Recording", FourCC(*b"abcd"), &payload),
        )
        .unwrap();
        let tags = CampaignTags::open(&game);
        let listed = list_folder(&game, &tags);
        assert!(!listed.any);
        assert!(listed.films.is_empty());
        assert!(listed.names.is_empty());
        assert!(listed.file_names.is_empty());

        write_recording(&game, b"good.m2rec", FourCC(*b"good"), &payload).unwrap();
        let listed = list_folder(&game, &tags);
        assert!(listed.any);
        assert_eq!(listed.films.len(), 1);
        assert_eq!(listed.file_names, [b"good.m2rec".to_vec()]);
        let _ = std::fs::remove_dir_all(&game);
    }

    #[test]
    fn file_in_rejects_traversal_backslashes_and_drive_prefixes() {
        let root = Path::new("/tmp/game");
        for bad in [
            b"".as_slice(),
            b".",
            b"..",
            b"a/b",
            b"../out",
            b"..\\out",
            b"a\\b",
            b"C:secret",
            b"c:foo",
            b"C:\\secret",
            b"a\0b",
        ] {
            assert!(
                file_in(root, FOLDER, bad).is_err(),
                "expected {:?} to be rejected",
                String::from_utf8_lossy(bad)
            );
        }
        assert_eq!(
            file_in(root, FOLDER, b"valid_film.m2rec").unwrap(),
            root.join(FOLDER).join("valid_film.m2rec")
        );
    }
}
