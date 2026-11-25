use crate::catalog::Level;
use crate::rules::{self, PostgameChoice};
use crate::{Action, Progress};
use mandrake_common::FourCC;
use mandrake_tag::TagArchive;
use mandrake_tag::tfl::menus::{TFL_FIRST_LEVEL, TflStringList};
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflPrefs {
    pub menu_background2: bool,
    pub difficulty: i16,
    pub visited: Vec<FourCC>,
}

impl Default for TflPrefs {
    fn default() -> Self {
        Self {
            menu_background2: false,
            difficulty: 2,
            visited: Vec::new(),
        }
    }
}

const PREFS_HEADER: usize = 0x40;

impl TflPrefs {
    pub fn read(game_dir: &Path) -> Self {
        let path = game_dir
            .join("tags")
            .join("Local")
            .join("PREF")
            .join("GAME");
        match mandrake_common::game_vfs().read_file(&path.to_string_lossy()) {
            Ok(d) => Self::from_file(&d),
            Err(e) => {
                info!(
                    "TFL menus: no preferences ({}: {e}); a fresh install's",
                    path.display()
                );
                Self::default()
            }
        }
    }

    pub fn from_file(file: &[u8]) -> Self {
        let d = file.get(PREFS_HEADER..).unwrap_or_default();
        let i16_at = |o: usize| d.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
        let i32_at = |o: usize| {
            d.get(o..o + 4)
                .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        let count = i16_at(0x16).unwrap_or(0).max(0) as usize;
        let visited = (0..count)
            .filter_map(|i| d.get(0x18 + 4 * i..0x1c + 4 * i))
            .map(|b| FourCC([b[0], b[1], b[2], b[3]]))
            .collect();
        Self {
            menu_background2: i32_at(0x04).unwrap_or(0) != 0,
            difficulty: i16_at(0x14).unwrap_or(2),
            visited,
        }
    }

    pub fn visited(&self, level: FourCC) -> bool {
        self.visited.contains(&level)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflLevelHeader {
    pub flags: u32,
    pub names: Option<FourCC>,
    pub pictures: Option<FourCC>,
    pub narration: Option<FourCC>,
    pub caption: Option<FourCC>,
    pub music: Option<FourCC>,
    pub post_art: Option<FourCC>,
    pub next: Option<FourCC>,
    pub alternative: Option<FourCC>,
    pub epilogue_sound: Option<FourCC>,
    pub movies: [Option<FourCC>; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TflMovie {
    pub id: FourCC,
    pub skippable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AfterMovie {
    MainMenu,
    Level {
        level: FourCC,
        difficulty: i16,
    },
    Epilogue {
        level: FourCC,
        difficulty: i16,
        choice: PostgameChoice,
        action: Action,
    },
}

pub const LEVEL_SOLO: u32 = 0x1_0000;
pub const LEVEL_HIDDEN: u32 = 0x80_0000;
const UNLISTED_MESHES: [FourCC; 3] = [FourCC(*b"20m2"), FourCC(*b"62ri"), FourCC(*b"2 ml")];

impl TflLevelHeader {
    pub fn from_mesh(d: &[u8]) -> Option<Self> {
        let tag = |o: usize| {
            let b: [u8; 4] = d.get(o..o + 4)?.try_into().ok()?;
            (b != [0xff; 4] && b != [0; 4]).then_some(FourCC(b))
        };
        let flags = u32::from_be_bytes(d.get(0x4c..0x50)?.try_into().ok()?);
        Some(Self {
            flags,
            names: tag(0x8c),
            pictures: tag(0x94),
            narration: tag(0xb0),
            caption: tag(0xfc),
            music: tag(0x100),
            post_art: tag(0x90),
            next: tag(0x9c),
            alternative: tag(0xa0),
            epilogue_sound: tag(0x108),
            movies: [tag(0xa4), tag(0xa8), tag(0xac)],
        })
    }

    pub fn movie(&self, index: usize) -> Option<TflMovie> {
        let id = (*self.movies.get(index)?)?;
        let skippable = match index {
            rules::WIN_FILM => self.next.is_some(),
            rules::LOSS_FILM => self.alternative.is_some(),
            _ => true,
        };
        Some(TflMovie { id, skippable })
    }
}

pub fn tfl_tag(archive: &TagArchive, group: &[u8; 4], id: FourCC) -> Option<(u16, Vec<u8>)> {
    let e = archive.find_tag_by_id(FourCC(*group), id)?;
    Some((e.header.version as u16, archive.get_tag_data(e).ok()?))
}

pub fn tfl_string(archive: &TagArchive, id: FourCC, index: usize) -> Option<Vec<u8>> {
    let (version, d) = tfl_tag(archive, b"stli", id)?;
    let list = TflStringList::from_tag(id, version, &d).ok()?;
    (index < list.len()).then(|| list.get(index).to_vec())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflLevel {
    pub id: FourCC,
    pub name: Vec<u8>,
}

pub fn level_name(archive: &TagArchive, id: FourCC, header: &TflLevelHeader) -> Vec<u8> {
    header
        .names
        .and_then(|n| tfl_string(archive, n, 0))
        .unwrap_or_else(|| {
            archive
                .find_tag_by_id(FourCC(*b"mesh"), id)
                .map_or_else(|| id.to_string(), |e| e.name.clone())
                .into_bytes()
        })
}

pub fn solo_levels(archive: &TagArchive, prefs: &TflPrefs, all_levels: bool) -> Vec<TflLevel> {
    let mesh = FourCC(*b"mesh");
    let mut seen = Vec::new();
    let mut out = Vec::new();
    let mut meshes: Vec<_> = archive
        .list_tags()
        .iter()
        .filter(|e| e.tag_type == mesh)
        .collect();
    meshes.sort_by_key(|e| e.name.to_lowercase());
    for e in meshes {
        if seen.contains(&e.tag_id) || UNLISTED_MESHES.contains(&e.tag_id) {
            continue;
        }
        seen.push(e.tag_id);
        let Some(header) = archive
            .get_tag_data(e)
            .ok()
            .and_then(|d| TflLevelHeader::from_mesh(&d))
        else {
            continue;
        };
        let open = all_levels || e.tag_id == TFL_FIRST_LEVEL || prefs.visited(e.tag_id);
        if header.flags & LEVEL_SOLO != 0 && header.flags & LEVEL_HIDDEN == 0 && open {
            out.push(TflLevel {
                id: e.tag_id,
                name: level_name(archive, e.tag_id, &header),
            });
        }
    }
    out
}

pub const LEVEL_NET_TYPES: u32 = 0xffff;

pub fn net_mapset_with_teams(
    archive: &TagArchive,
    name: &[u8],
) -> (crate::catalog::Campaign, Vec<crate::catalog::MaxTeams>) {
    let mesh = FourCC(*b"mesh");
    let mut tags: Vec<_> = archive
        .list_tags()
        .iter()
        .filter(|e| e.tag_type == mesh)
        .collect();
    tags.sort_by_key(|e| e.name.to_lowercase());
    let mut entries: Vec<crate::catalog::ListEntry> = Vec::new();
    let mut max_teams: Vec<crate::catalog::MaxTeams> = Vec::new();
    for e in tags {
        if entries.iter().any(|l| l.mesh == e.tag_id) || UNLISTED_MESHES.contains(&e.tag_id) {
            continue;
        }
        let Ok(data) = archive.get_tag_data(e) else {
            continue;
        };
        let Some(header) = TflLevelHeader::from_mesh(&data) else {
            continue;
        };
        if header.flags & LEVEL_NET_TYPES == 0 || header.flags & LEVEL_HIDDEN != 0 {
            continue;
        }
        entries.push(crate::catalog::ListEntry {
            mesh: e.tag_id,
            name: crate::Text(level_name(archive, e.tag_id, &header)),
            flags: (header.flags & (LEVEL_NET_TYPES | LEVEL_TRADING | LEVEL_VETERANS))
                | rules::ENTRY_LISTED,
            order: entries.len() as i16,
            extra_flags: 0,
            plugin_checksum: NO_PLUGIN,
        });
        if let Some(b) = data
            .get(0x54..0x58)
            .and_then(|s| <[u8; 4]>::try_from(s).ok())
            && let Ok(most) = i16::try_from(i32::from_be_bytes(b))
        {
            max_teams.push(((NO_PLUGIN, e.tag_id), most));
        }
    }
    (
        crate::catalog::Campaign {
            name: crate::Text(name.to_vec()),
            checksum: NO_PLUGIN,
            plugins: Vec::new(),
            entries,
        },
        max_teams,
    )
}

pub fn net_mapset(archive: &TagArchive, name: &[u8]) -> crate::catalog::Campaign {
    net_mapset_with_teams(archive, name).0
}

pub const LEVEL_TRADING: u32 = 0x2_0000;
pub const LEVEL_VETERANS: u32 = 0x4_0000;

pub fn net_max_teams(archive: &TagArchive) -> Vec<crate::catalog::MaxTeams> {
    net_mapset_with_teams(archive, b"").1
}

pub fn coop_mapset(archive: &TagArchive, prefs: &TflPrefs) -> crate::net::NetMapset {
    crate::net::NetMapset {
        name: crate::Text(b"Myth: The Fallen Lords".to_vec()),
        plugin_names: Vec::new(),
        plugins: Vec::new(),
        maps: solo_levels(archive, prefs, false)
            .into_iter()
            .enumerate()
            .map(|(i, level)| crate::net::NetMap {
                mesh: level.id,
                name: crate::Text(level.name),
                flags: LEVEL_SOLO | rules::ENTRY_LISTED,
                order: i as i16,
                max_teams: 0,
            })
            .collect(),
    }
}

pub fn describe_net_game(archive: &TagArchive, game: &mut crate::films::FilmGame) {
    game.plugins_missing = false;
    game.plugins_present = true;
    let Some((_, mesh)) = tfl_tag(archive, b"mesh", game.mesh) else {
        game.map_name = None;
        return;
    };
    if let Some(header) = TflLevelHeader::from_mesh(&mesh) {
        game.map_name = Some(crate::Text(level_name(archive, game.mesh, &header)));
        game.art = header.pictures;
        game.art_plugins = Vec::new();
    }
    if let Some(b) = mesh.get(0x54..0x58) {
        game.starting_locations = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    }
}

pub fn films_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("tags").join("Local").join("RECO")
}

pub fn look_up_film(archive: &TagArchive, game: &mut crate::films::FilmGame) {
    game.plugins_missing = false;
    game.plugins_present = true;
    game.map_name = None;
    game.version_of_id = i16::MAX;
    let Some((_, mesh)) = tfl_tag(archive, b"mesh", game.mesh) else {
        return;
    };
    let Some(header) = TflLevelHeader::from_mesh(&mesh) else {
        return;
    };
    game.map_name = Some(crate::Text(level_name(archive, game.mesh, &header)));
    game.art = header.pictures;
    if let Some(b) = mesh.get(0x54..0x58) {
        game.starting_locations = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflPrologue {
    pub title: Vec<u8>,
    pub pictures: FourCC,
    pub text: Vec<u8>,
    pub caption: Option<Vec<u8>>,
    pub music: Option<FourCC>,
}

impl TflPrologue {
    pub fn of_level(archive: &TagArchive, level: FourCC) -> Option<Self> {
        let (_, mesh) = tfl_tag(archive, b"mesh", level)?;
        let h = TflLevelHeader::from_mesh(&mesh)?;
        let pictures = h.pictures?;
        let narration = h.narration?;
        archive.find_tag_by_id(FourCC(*b".256"), pictures)?;
        let (version, text) = tfl_tag(archive, b"text", narration)?;
        let text =
            style_codes(&mandrake_tag::tfl::menus::tfl_text(narration, version, &text).ok()?);
        Some(Self {
            title: h
                .names
                .and_then(|n| tfl_string(archive, n, 0))
                .unwrap_or_default(),
            pictures,
            text,
            caption: h.caption.and_then(|c| tfl_string(archive, c, 0)),
            music: h.music,
        })
    }
}

pub fn style_codes(text: &[u8]) -> Vec<u8> {
    let mut out = text.to_vec();
    for i in 0..out.len().saturating_sub(1) {
        if out[i] == b'\\' && out[i + 1].is_ascii_alphabetic() {
            out[i] = b'|';
        }
    }
    out
}

pub const TFL_MAX_VISITED: usize = 300;

pub(crate) const NO_PLUGIN: i32 = -1;

pub trait LevelCarry: std::any::Any + std::fmt::Debug + Send {
    fn counts(&self) -> (usize, usize);
    fn as_any(&self) -> &dyn std::any::Any;
}

pub const TFL_PROGRESS_FILE: &str = "tfl-progress.txt";

pub fn epilogue_level(archive: &TagArchive, level: FourCC) -> Option<Level> {
    let (_, mesh) = tfl_tag(archive, b"mesh", level)?;
    let h = TflLevelHeader::from_mesh(&mesh)?;
    Some(Level {
        post_art: h.post_art,
        next: h.next,
        alternate: h.alternative,
        ambient_win: h.epilogue_sound,
        ambient_loss: h.epilogue_sound,
        ..Level::new(level)
    })
}

pub struct TflGame {
    pub game_dir: PathBuf,
    pub archive: TagArchive,
    pub prefs: TflPrefs,
    pub our_films: Option<PathBuf>,
    pub(crate) carry: Option<Box<dyn LevelCarry>>,
    pub(crate) carry_serial: u32,
    pub(crate) level: Option<(FourCC, i16)>,
    pub(crate) epilogue: Option<PostgameChoice>,
}

impl std::fmt::Debug for TflGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TflGame")
            .field("game_dir", &self.game_dir)
            .field("prefs", &self.prefs)
            .field("our_films", &self.our_films)
            .field("carry", &self.carry)
            .field("level", &self.level)
            .finish_non_exhaustive()
    }
}

impl TflGame {
    pub fn new(game_dir: &Path, our_films: Option<PathBuf>) -> Result<Self, String> {
        let archive = mandrake_tag::tag_files::open_tfl_tags(game_dir)
            .map_err(|e| format!("TFL tags in {}: {e}", game_dir.display()))?;
        Ok(Self::with(
            game_dir,
            archive,
            TflPrefs::read(game_dir),
            our_films,
        ))
    }

    pub fn with(
        game_dir: &Path,
        archive: TagArchive,
        prefs: TflPrefs,
        our_films: Option<PathBuf>,
    ) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
            archive,
            prefs,
            our_films,
            carry: None,
            carry_serial: 0,
            level: None,
            epilogue: None,
        }
    }

    pub(crate) fn set_carry(&mut self, carry: Option<Box<dyn LevelCarry>>) {
        self.carry = carry;
        self.carry_serial = self.carry_serial.wrapping_add(1);
    }

    pub fn imported_progress(&self) -> Progress {
        let mut progress = Progress {
            visited: Vec::new(),
            difficulty: self.prefs.difficulty,
            ..Progress::default()
        };
        for &level in &self.prefs.visited {
            progress.mark_visited_up_to(level, NO_PLUGIN, TFL_MAX_VISITED);
        }
        progress
    }

    pub fn level_movie(&self, level: FourCC, index: usize) -> Option<TflMovie> {
        let (_, mesh) = tfl_tag(&self.archive, b"mesh", level)?;
        let movie = TflLevelHeader::from_mesh(&mesh)?.movie(index)?;
        self.has_movie(movie.id).then_some(movie)
    }

    fn has_movie(&self, id: FourCC) -> bool {
        self.archive.find_tag_by_id(FourCC(*b"moov"), id).is_some()
    }

    pub fn movie_data(&self, id: FourCC) -> Option<Vec<u8>> {
        tfl_tag(&self.archive, b"moov", id).map(|(_, d)| d)
    }

    pub fn has_films(&self) -> bool {
        !crate::films::tfl::films_of(self.our_films.as_deref(), &films_dir(&self.game_dir))
            .is_empty()
    }

    pub fn has_intro(&self) -> bool {
        self.has_movie(mandrake_tag::tfl::menus::TFL_INTRO_MOVIE)
    }

    pub fn difficulties(&self) -> Vec<Vec<u8>> {
        tfl_tag(&self.archive, b"stli", FourCC(*b"indl"))
            .and_then(|(v, d)| TflStringList::from_tag(FourCC(*b"indl"), v, &d).ok())
            .map(|l| l.strings)
            .unwrap_or_default()
    }

    pub fn film_folder(&self) -> crate::films::FilmFolder {
        let untitled = tfl_string(&self.archive, FourCC(*b"inrf"), 2)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| b"Untitled Replay".to_vec());
        crate::films::tfl::list_folders(
            self.our_films.as_deref(),
            &films_dir(&self.game_dir),
            untitled,
            |game| look_up_film(&self.archive, game),
        )
    }

    pub fn is_our_film(&self, path: &Path) -> bool {
        self.our_films
            .as_deref()
            .is_some_and(|ours| path.parent() == Some(ours))
    }

    fn ours(&self) -> std::io::Result<&Path> {
        self.our_films.as_deref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no folder for our TFL films")
        })
    }

    pub fn save_last_film(&self, name: &[u8]) -> std::io::Result<Vec<u8>> {
        let ours = self.ours()?;
        let films = crate::films::tfl::films_of(Some(ours), &films_dir(&self.game_dir));
        let last = films
            .iter()
            .find(|f| {
                f.file_name
                    .eq_ignore_ascii_case(crate::films::tfl::LAST_RECORDING_FILE)
            })
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no last recording")
            })?;
        let taken: Vec<FourCC> = films.iter().map(|f| f.header.tag_id).collect();
        crate::films::tfl::save_last_film(ours, &last.path, name, &taken)
    }

    pub fn delete_film(&self, file_name: &[u8]) -> std::io::Result<()> {
        crate::films::tfl::delete_film(self.ours()?, file_name)
    }

    pub fn rename_film(&self, file_name: &[u8], name: &[u8]) -> std::io::Result<Vec<u8>> {
        let ours = self.ours()?;
        let names: Vec<Vec<u8>> =
            crate::films::tfl::films_of(Some(ours), &films_dir(&self.game_dir))
                .into_iter()
                .map(|f| f.file_name)
                .collect();
        crate::films::tfl::rename_film(ours, file_name, name, &names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_read_as_the_exe_keeps_them() {
        let mut f = vec![0u8; PREFS_HEADER + 0x4cc];
        let d = &mut f[PREFS_HEADER..];
        d[0x04..0x08].copy_from_slice(&1i32.to_be_bytes());
        d[0x14..0x16].copy_from_slice(&3i16.to_be_bytes());
        d[0x16..0x18].copy_from_slice(&2i16.to_be_bytes());
        d[0x18..0x20].copy_from_slice(b"ambu00tm");
        let p = TflPrefs::from_file(&f);
        assert!(p.menu_background2);
        assert_eq!(p.difficulty, 3);
        assert_eq!(p.visited, [FourCC(*b"ambu"), FourCC(*b"00tm")]);
        assert!(p.visited(FourCC(*b"00tm")) && !p.visited(FourCC(*b"rend")));
        assert_eq!(TflPrefs::from_file(&[]), TflPrefs::default());
    }

    #[test]
    fn narration_styles_become_the_fonts_codes() {
        assert_eq!(
            style_codes(b"\\iWednesday\\p a\\ b\\"),
            b"|iWednesday|p a\\ b\\"
        );
    }

    #[test]
    fn a_levels_movies_and_what_skips_them() {
        let mut d = vec![0xffu8; 0x110];
        d[0x4c..0x50].copy_from_slice(&0u32.to_be_bytes());
        d[0xa4..0xa8].copy_from_slice(b"01cr");
        d[0xa8..0xac].copy_from_slice(b"99ep");
        d[0xac..0xb0].copy_from_slice(b"lose");
        let h = TflLevelHeader::from_mesh(&d).unwrap();
        let movie = |h: &TflLevelHeader, i| h.movie(i).map(|m| (m.id, m.skippable));
        assert_eq!(
            movie(&h, rules::PREGAME_FILM),
            Some((FourCC(*b"01cr"), true))
        );
        assert_eq!(movie(&h, rules::WIN_FILM), Some((FourCC(*b"99ep"), false)));
        assert_eq!(movie(&h, rules::LOSS_FILM), Some((FourCC(*b"lose"), false)));
        assert_eq!(h.movie(3), None);
        d[0x9c..0xa0].copy_from_slice(b"next");
        d[0xa0..0xa4].copy_from_slice(b"altn");
        let h = TflLevelHeader::from_mesh(&d).unwrap();
        assert_eq!(movie(&h, rules::WIN_FILM), Some((FourCC(*b"99ep"), true)));
        assert_eq!(movie(&h, rules::LOSS_FILM), Some((FourCC(*b"lose"), true)));
        d[0xa4..0xb0].fill(0xff);
        let h = TflLevelHeader::from_mesh(&d).unwrap();
        assert!((0..3).all(|i| h.movie(i).is_none()));
    }

    #[test]
    fn a_level_header_reads_its_tags() {
        let mut d = vec![0u8; 0x104];
        d[0x4c..0x50].copy_from_slice(&LEVEL_SOLO.to_be_bytes());
        d[0x8c..0x90].copy_from_slice(b"01lv");
        d[0x94..0x98].copy_from_slice(b"01pr");
        d[0xb0..0xb4].copy_from_slice(b"01tx");
        d[0xfc..0x100].copy_from_slice(&[0xff; 4]);
        d[0x100..0x104].copy_from_slice(b"01mu");
        let h = TflLevelHeader::from_mesh(&d).unwrap();
        assert_eq!(h.flags, LEVEL_SOLO);
        assert_eq!(h.names, Some(FourCC(*b"01lv")));
        assert_eq!(h.caption, None);
        assert_eq!(h.music, Some(FourCC(*b"01mu")));
        assert!(TflLevelHeader::from_mesh(&d[..0x40]).is_none());
    }
}
