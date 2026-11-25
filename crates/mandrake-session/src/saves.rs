use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use mandrake_common::FourCC;
use mandrake_rec::header::{MonsterInitializer, RecordingHeader};
use mandrake_rec::saved_game::{SAVED_GAME_HEADER_SIZE, SavedGame, SavedGameHeader, Snapshot};
use mandrake_tag::mesh_links::CampaignTags;

use crate::films::{self, FilmGame, FilmTags};
use crate::text::Text;

pub const SAVE: FourCC = FourCC(*b"save");
pub const FOLDER: &str = "saved games";
const NEWEST_TAG_VERSION: i16 = 3;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

#[derive(Debug, Clone, PartialEq)]
pub struct SaveFile {
    pub path: PathBuf,
    pub name: Text,
    pub modified: i64,
    pub date: Text,
    pub game: FilmGame,
    pub end_tick: u32,
    pub veterans: Vec<MonsterInitializer>,
    pub snapshot: Option<Snapshot>,
    pub subgroup: FourCC,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaveFolder {
    pub saves: Vec<SaveFile>,
    pub any: bool,
    pub new_name: Text,
}

pub const AUTOSAVE_PREFIX: &[u8] = b"Autosave- ";

pub fn autosave_name(time: i64, zone: &LocalZone) -> Vec<u8> {
    let local = time + i64::from(zone.offset(time));
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    let mut name = AUTOSAVE_PREFIX.to_vec();
    name.extend_from_slice(
        format!(
            "{year:04}-{month:02}-{day:02} {:02}.{:02}.{:02}",
            secs / 3600,
            secs / 60 % 60,
            secs % 60
        )
        .as_bytes(),
    );
    name
}

pub const QUICKSAVES: u32 = 5;

pub fn quicksave_name(folder: &SaveFolder) -> Vec<u8> {
    let name = |n: u32| format!("Quicksave {n}").into_bytes();
    let mut oldest: Option<(i64, u32)> = None;
    for n in 1..=QUICKSAVES {
        let wanted = name(n);
        let Some(save) = folder
            .saves
            .iter()
            .find(|s| s.name.0.eq_ignore_ascii_case(&wanted))
        else {
            return wanted;
        };
        match oldest {
            Some((time, _)) if time != 0 && save.modified >= time => {}
            _ => oldest = Some((save.modified, n)),
        }
    }
    name(oldest.map_or(1, |(_, n)| n))
}

pub fn save_name_ok(name: &[u8]) -> bool {
    mandrake_rec::writer::save_names(name).is_some_and(|(_, _, changed)| !changed)
}

pub fn subgroup_for_save(game_dir: &Path, tag_name: &[u8], file_name: &[u8]) -> FourCC {
    let mut taken = Vec::new();
    if let Ok(dir) = std::fs::read_dir(game_dir.join(FOLDER)) {
        for path in dir.filter_map(|e| e.ok().map(|e| e.path())) {
            let mut head = [0u8; 64];
            let read = std::fs::File::open(&path)
                .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
                .is_ok();
            if !read || !is_save(&head) {
                continue;
            }
            let subgroup = FourCC([head[40], head[41], head[42], head[43]]);
            if path
                .file_name()
                .map(|n| n.to_string_lossy().as_bytes().to_vec())
                == Some(file_name.to_vec())
            {
                return subgroup;
            }
            taken.push(subgroup);
        }
    }
    mandrake_rec::writer::subgroup_for_name(tag_name, &taken)
}

pub fn write_save(game_dir: &Path, name: &[u8], data: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    let (tag_name, file_name, _) = mandrake_rec::writer::save_names(name)
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?;
    let path = films::file_in(game_dir, FOLDER, &file_name)?;
    let subgroup = subgroup_for_save(game_dir, &tag_name, &file_name);
    let folder = game_dir.join(FOLDER);
    std::fs::create_dir_all(&folder)?;
    std::fs::write(
        &path,
        mandrake_rec::writer::save_file(&tag_name, subgroup, data),
    )?;
    Ok(path)
}

fn be16(d: &[u8], o: usize) -> i16 {
    i16::from_be_bytes([d[o], d[o + 1]])
}

pub fn is_save(file: &[u8]) -> bool {
    file.get(36..40) == Some(&SAVE.0[..])
}

pub fn read_header(file: &[u8]) -> Option<&[u8]> {
    if !is_save(file) || file.len() < 64 || be16(file, 56) > NEWEST_TAG_VERSION {
        return None;
    }
    let offset = usize::try_from(i32::from_be_bytes(file[44..48].try_into().ok()?))
        .ok()?
        .max(64);
    let header = file.get(offset..offset + films::HEADER_SIZE)?;
    (be16(header, 0) <= 3).then_some(header)
}

pub fn snapshot_of(file: &[u8]) -> Option<Snapshot> {
    SavedGame::from_bytes(file).ok()?.snapshot
}

fn read_prefix(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match file.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

fn read_snapshot_from_file(
    file: &mut std::fs::File,
    header: &SavedGameHeader,
    file_len: u64,
    pixels_at: u64,
) -> Option<Snapshot> {
    let (w, h) = (header.snapshot_width, header.snapshot_height);
    if w <= 0 || h <= 0 {
        return None;
    }
    let (w, h) = (w as usize, h as usize);
    let bpp = header.bytes_per_pixel();
    let pixel_bytes = w.checked_mul(h)?.checked_mul(bpp)?;
    let remaining = file_len.checked_sub(pixels_at)?;
    if (pixel_bytes as u64) > remaining {
        return None;
    }
    let mut pixels = vec![0u8; pixel_bytes];
    file.read_exact(&mut pixels).ok()?;
    let mut rgba = Vec::with_capacity(w.checked_mul(h)?.checked_mul(4)?);
    for p in pixels.chunks_exact(bpp) {
        if bpp == 4 {
            rgba.extend_from_slice(&[p[1], p[2], p[3], 0xff]);
        } else {
            let v = u16::from_be_bytes([p[0], p[1]]);
            let c = |s: u16| {
                let x = (v >> s & 0x1f) as u8;
                x << 3 | x >> 2
            };
            rgba.extend_from_slice(&[c(10), c(5), c(0), 0xff]);
        }
    }
    Some(Snapshot {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

fn read_saved_details(
    file: &mut std::fs::File,
    data_offset: u64,
    header: &[u8; films::HEADER_SIZE],
    file_len: u64,
) -> Option<(u32, Vec<MonsterInitializer>, Option<Snapshot>)> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let (recording, _) = RecordingHeader::read(file).ok()?;
    if !recording.is_saved_game() {
        return None;
    }
    let rel = u32::from_be_bytes([header[0x0c], header[0x0d], header[0x0e], header[0x0f]]);
    let block_at = data_offset.checked_add(u64::from(rel))?;
    file.seek(SeekFrom::Start(block_at)).ok()?;
    let mut block = [0u8; SAVED_GAME_HEADER_SIZE];
    file.read_exact(&mut block).ok()?;
    let saved_header = SavedGameHeader::parse(&block);
    let pixels_at = block_at.checked_add(SAVED_GAME_HEADER_SIZE as u64)?;
    let snapshot = read_snapshot_from_file(file, &saved_header, file_len, pixels_at);
    Some((recording.end_tick, recording.monster_initializers, snapshot))
}

fn read_save_entry(path: &Path, tags: &FilmTags<'_>, zone: &LocalZone) -> (bool, Option<SaveFile>) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return (false, None);
    };
    let mut head = [0u8; 64];
    let Ok(read) = read_prefix(&mut file, &mut head) else {
        return (false, None);
    };
    if !is_save(&head[..read]) {
        return (false, None);
    }
    if read < 64 || be16(&head, 56) > NEWEST_TAG_VERSION {
        return (true, None);
    }
    let Ok(raw_offset) = head[44..48].try_into() else {
        return (true, None);
    };
    let Ok(offset) = usize::try_from(i32::from_be_bytes(raw_offset)) else {
        return (true, None);
    };
    let offset = offset.max(64) as u64;
    if offset != 64 && file.seek(SeekFrom::Start(offset)).is_err() {
        return (true, None);
    }
    let mut header = [0u8; films::HEADER_SIZE];
    if file.read_exact(&mut header).is_err() || be16(&header, 0) > 3 {
        return (true, None);
    }
    let mut game = films::parse_game(&header);
    tags.look_up(&mut game);
    let meta = file.metadata().ok();
    let file_len = meta.as_ref().map_or(0, |m| m.len());
    let modified = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    let (end_tick, veterans, snapshot) =
        read_saved_details(&mut file, offset, &header, file_len).unwrap_or_default();
    let Some(file_name) = path
        .file_name()
        .map(|n| n.to_string_lossy().as_bytes().to_vec())
    else {
        return (true, None);
    };
    let save = SaveFile {
        path: path.to_path_buf(),
        name: Text(file_name),
        modified,
        date: Text(date_string(modified, zone).into_bytes()),
        game,
        end_tick,
        veterans,
        snapshot,
        subgroup: FourCC([head[40], head[41], head[42], head[43]]),
    };
    (true, Some(save))
}

pub fn read_save(path: &Path, tags: &FilmTags<'_>, zone: &LocalZone) -> Option<SaveFile> {
    read_save_entry(path, tags, zone).1
}

pub fn delete_save(game_dir: &Path, file_name: &[u8]) -> std::io::Result<()> {
    std::fs::remove_file(films::file_in(game_dir, FOLDER, file_name)?)
}

fn newest_first(a: &SaveFile, b: &SaveFile) -> std::cmp::Ordering {
    b.modified.cmp(&a.modified)
}

pub fn list_folder(game_dir: &Path, tags: &CampaignTags) -> SaveFolder {
    let tags = FilmTags::new(tags);
    let zone = LocalZone::system();
    let folder = game_dir.join(FOLDER);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map(|dir| {
            dir.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    let mut any = false;
    let mut saves = Vec::new();
    for path in &paths {
        let (is_tag, save) = read_save_entry(path, &tags, &zone);
        any |= is_tag;
        if let Some(save) = save {
            saves.push(save);
        }
    }
    saves.sort_by(newest_first);
    SaveFolder {
        saves,
        any,
        new_name: Text(tags.new_saved_game_name()),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalZone {
    transitions: Vec<(i64, i32)>,
    initial: i32,
}

impl LocalZone {
    pub fn system() -> Self {
        let named = std::env::var("TZ").ok().and_then(|tz| {
            let tz = tz.strip_prefix(':').unwrap_or(&tz).to_owned();
            if tz.is_empty() {
                return None;
            }
            let path = if tz.starts_with('/') {
                PathBuf::from(tz)
            } else {
                Path::new("/usr/share/zoneinfo").join(tz)
            };
            std::fs::read(path).ok()
        });
        named
            .or_else(|| std::fs::read("/etc/localtime").ok())
            .and_then(|data| Self::from_tzif(&data))
            .unwrap_or_default()
    }

    pub fn from_tzif(data: &[u8]) -> Option<Self> {
        let counts = |at: usize| -> Option<[usize; 6]> {
            let magic_end = at.checked_add(4)?;
            if data.get(at..magic_end)? != b"TZif" {
                return None;
            }
            let mut c = [0usize; 6];
            for (i, c) in c.iter_mut().enumerate() {
                let start = at.checked_add(20)?.checked_add(i.checked_mul(4)?)?;
                let end = start.checked_add(4)?;
                let w = data.get(start..end)?;
                *c = u32::from_be_bytes(w.try_into().ok()?) as usize;
            }
            Some(c)
        };
        let v1 = counts(0)?;
        let block_len = |c: [usize; 6], time: usize| -> Option<usize> {
            let [isut, isstd, leap, time_count, types, chars] = c;
            time_count
                .checked_mul(time.checked_add(1)?)?
                .checked_add(types.checked_mul(6)?)?
                .checked_add(chars)?
                .checked_add(leap.checked_mul(time.checked_add(4)?)?)?
                .checked_add(isstd)?
                .checked_add(isut)
        };
        let (at, time) = if *data.get(4)? >= b'2' {
            (44usize.checked_add(block_len(v1, 4)?)?, 8)
        } else {
            (0, 4)
        };
        let [_, _, _, time_count, types, _] = counts(at)?;
        let base = at.checked_add(44)?;
        let times_end = base.checked_add(time_count.checked_mul(time)?)?;
        let indices_end = times_end.checked_add(time_count)?;
        let infos_end = indices_end.checked_add(types.checked_mul(6)?)?;
        let times = data.get(base..times_end)?;
        let indices = data.get(times_end..indices_end)?;
        let infos = data.get(indices_end..infos_end)?;
        let offset_of = |i: usize| -> Option<i32> {
            let start = i.checked_mul(6)?;
            let end = start.checked_add(4)?;
            Some(i32::from_be_bytes(infos.get(start..end)?.try_into().ok()?))
        };
        let transitions = (0..time_count)
            .map(|i| {
                let t = &times[i * time..(i + 1) * time];
                let t = if time == 8 {
                    i64::from_be_bytes(t.try_into().ok()?)
                } else {
                    i64::from(i32::from_be_bytes(t.try_into().ok()?))
                };
                Some((t, offset_of(usize::from(indices[i]))?))
            })
            .collect::<Option<Vec<_>>>()?;
        let initial = (0..types)
            .find(|&i| infos[i * 6 + 4] == 0)
            .and_then(offset_of)
            .or_else(|| offset_of(0))
            .unwrap_or(0);
        Some(Self {
            transitions,
            initial,
        })
    }

    pub fn offset(&self, time: i64) -> i32 {
        match self.transitions.partition_point(|&(t, _)| t <= time) {
            0 => self.initial,
            n => self.transitions[n - 1].1,
        }
    }
}

pub fn date_string(time: i64, zone: &LocalZone) -> String {
    let local = time + i64::from(zone.offset(time));
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (secs / 3600, secs / 60 % 60);
    let hour12 = match hour % 12 {
        0 => 12,
        h => h,
    };
    let ampm = if hour < 12 { "AM" } else { "PM" };
    format!(
        "{} {day:02}, {year} {hour12:02}:{minute:02} {ampm}",
        MONTHS[month as usize - 1]
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sydney() -> LocalZone {
        LocalZone {
            transitions: vec![(1_775_318_400, 36_000), (1_791_043_200, 39_600)],
            initial: 39_600,
        }
    }

    #[test]
    fn the_date_reads_as_the_engines() {
        let zone = sydney();
        let time = 1_793_649_051;
        assert_eq!(date_string(time, &zone), "Nov 03, 2026 06:50 AM");
        assert_eq!(date_string(time + 7 * 3600, &zone), "Nov 03, 2026 01:50 PM");
        assert_eq!(date_string(time - 7 * 3600, &zone), "Nov 02, 2026 11:50 PM");
        assert_eq!(
            date_string(0, &LocalZone::default()),
            "Jan 01, 1970 12:00 AM"
        );
    }

    #[test]
    fn newer_saves_come_first() {
        let save = |modified| SaveFile {
            path: PathBuf::new(),
            name: Text::default(),
            modified,
            date: Text::default(),
            game: FilmGame::default(),
            end_tick: 0,
            veterans: Vec::new(),
            snapshot: None,
            subgroup: FourCC(*b"nesg"),
        };
        let mut saves = [save(5), save(9), save(7)];
        saves.sort_by(newest_first);
        let order: Vec<i64> = saves.iter().map(|s| s.modified).collect();
        assert_eq!(order, [9, 7, 5]);
    }

    #[test]
    fn an_autosave_is_named_by_the_local_time() {
        assert_eq!(
            autosave_name(1_793_649_051, &sydney()),
            b"Autosave- 2026-11-03 06.50.51"
        );
        assert_eq!(
            autosave_name(0, &LocalZone::default()),
            b"Autosave- 1970-01-01 00.00.00"
        );
    }

    fn folder_of(saves: &[(&str, i64)]) -> SaveFolder {
        SaveFolder {
            saves: saves
                .iter()
                .map(|&(name, modified)| SaveFile {
                    path: PathBuf::new(),
                    name: Text(name.as_bytes().to_vec()),
                    modified,
                    date: Text::default(),
                    game: FilmGame::default(),
                    end_tick: 0,
                    veterans: Vec::new(),
                    snapshot: None,
                    subgroup: FourCC(*b"quic"),
                })
                .collect(),
            any: !saves.is_empty(),
            new_name: Text::default(),
        }
    }

    #[test]
    fn quicksaves_take_the_first_free_number_then_the_oldest() {
        assert_eq!(quicksave_name(&folder_of(&[])), b"Quicksave 1");
        assert_eq!(
            quicksave_name(&folder_of(&[("Quicksave 1", 5), ("Other", 1)])),
            b"Quicksave 2"
        );
        assert_eq!(
            quicksave_name(&folder_of(&[("Quicksave 1", 5), ("Quicksave 3", 1)])),
            b"Quicksave 2"
        );
        let full = [
            ("Quicksave 1", 1_793_309_460),
            ("Quicksave 2", 1_793_354_903),
            ("Quicksave 3", 1_793_354_904),
            ("Quicksave 4", 1_793_424_660),
            ("Quicksave 5", 1_793_528_220),
        ];
        assert_eq!(quicksave_name(&folder_of(&full)), b"Quicksave 1");
        let mut next = full;
        next[0].1 = 1_800_000_000;
        assert_eq!(quicksave_name(&folder_of(&next)), b"Quicksave 2");
        let same = [
            ("quicksave 1", 7),
            ("Quicksave 2", 7),
            ("Quicksave 3", 7),
            ("Quicksave 4", 7),
            ("QUICKSAVE 5", 7),
        ];
        assert_eq!(quicksave_name(&folder_of(&same)), b"Quicksave 1");
        let mut zero = same;
        zero[0].1 = 0;
        zero[2].1 = 3;
        assert_eq!(quicksave_name(&folder_of(&zero)), b"Quicksave 3");
    }

    #[test]
    fn a_save_name_is_ok_as_typed() {
        assert!(save_name_ok(b"New Saved Game"));
        assert!(save_name_ok(b"Autosave- 2026-11-03 06.50.51"));
        assert!(
            !save_name_ok(b"a/b"),
            "a name fit for a file only once changed"
        );
        assert!(!save_name_ok(b""));
        assert!(!save_name_ok(b"name "), "a space at its end goes");
    }

    #[test]
    fn the_system_zone_reads() {
        let Ok(data) = std::fs::read("/etc/localtime") else {
            return;
        };
        let zone = LocalZone::from_tzif(&data).expect("a TZif file");
        assert_eq!(zone.offset(1_800_000_000) % 60, 0);
    }

    #[test]
    fn newest_first_is_a_total_order_across_32_bit_wrap() {
        let save = |modified| SaveFile {
            path: PathBuf::new(),
            name: Text::default(),
            modified,
            date: Text::default(),
            game: FilmGame::default(),
            end_tick: 0,
            veterans: Vec::new(),
            snapshot: None,
            subgroup: FourCC(*b"nesg"),
        };
        let a = save(0);
        let b = save(0x8000_0000);
        assert_eq!(newest_first(&a, &b), std::cmp::Ordering::Greater);
        assert_eq!(newest_first(&b, &a), std::cmp::Ordering::Less);

        let mut saves = [
            save(0),
            save(0x8000_0000),
            save(1_700_000_000),
            save(i64::from(i32::MAX) + 1000),
            save(i64::MIN),
            save(i64::MAX),
            save(-1),
        ];
        saves.sort_by(newest_first);
        let order: Vec<i64> = saves.iter().map(|s| s.modified).collect();
        assert_eq!(
            order,
            [
                i64::MAX,
                i64::from(i32::MAX) + 1000,
                0x8000_0000,
                1_700_000_000,
                0,
                -1,
                i64::MIN,
            ]
        );
    }

    #[test]
    fn from_tzif_rejects_overflowing_counts_without_panicking() {
        let mut v1 = [0u8; 44];
        v1[..4].copy_from_slice(b"TZif");
        for chunk in v1[20..44].as_chunks_mut::<4>().0 {
            *chunk = u32::MAX.to_be_bytes();
        }
        assert_eq!(LocalZone::from_tzif(&v1), None);

        let mut v2 = v1;
        v2[4] = b'2';
        assert_eq!(LocalZone::from_tzif(&v2), None);
    }
}
