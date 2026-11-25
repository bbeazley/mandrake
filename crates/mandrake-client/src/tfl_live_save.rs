use mandrake_common::FourCC;
use mandrake_rec::saved_game::Snapshot;
use mandrake_session::films::FilmGame;
use mandrake_session::saves::{LocalZone, SaveFile, SaveFolder, date_string};
use mandrake_session::text::Text;
use mandrake_tfl_sim::MonsterInitializer;
use mandrake_tfl_sim::statistics::PermanentRanking;
use std::path::{Path, PathBuf};

pub(crate) const MAGIC: &[u8; 8] = b"M2TFLSV2";
pub(crate) const MAGIC_V1: &[u8; 8] = b"M2TFLSV1";
const RANKING: usize = 0x1c;
pub(crate) const SAVE_ID: FourCC = FourCC(*b"save");

pub(crate) fn saves_dir() -> Option<PathBuf> {
    #[cfg(test)]
    return TEST_SAVES.with(|d| d.borrow().clone());
    #[cfg(not(test))]
    crate::user_dirs::config_file("tfl").map(|d| d.join("SAVE"))
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_SAVES: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TflSave {
    pub(crate) film: Vec<u8>,
    pub(crate) permanent: Vec<PermanentRanking>,
    pub(crate) veterans: Vec<MonsterInitializer>,
    pub(crate) snapshot: Option<Snapshot>,
}

fn take_u32(b: &mut &[u8]) -> Option<u32> {
    let (head, rest) = b.split_first_chunk::<4>()?;
    *b = rest;
    Some(u32::from_be_bytes(*head))
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (head, rest) = b.split_at_checked(n)?;
    *b = rest;
    Some(head)
}

fn veterans_of(block: &[u8]) -> Vec<MonsterInitializer> {
    block
        .as_chunks::<{ MonsterInitializer::SIZE }>()
        .0
        .iter()
        .map(|r| {
            let tag = |o: usize| FourCC(r[o..o + 4].try_into().unwrap());
            let name = &r[0x16..];
            MonsterInitializer {
                used: false,
                player: u32::from_be_bytes(r[4..8].try_into().unwrap()),
                unit: tag(8),
                mons: tag(0xc),
                experience: r[0x10] as i8,
                field_11: r[0x11],
                permutation: r[0x12] as i8,
                field_13: r[0x13],
                name: name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())].to_vec(),
            }
        })
        .collect()
}

fn myth2_veteran(v: &MonsterInitializer) -> mandrake_rec::header::MonsterInitializer {
    mandrake_rec::header::MonsterInitializer {
        player_id: v.player,
        unit_tag: v.unit,
        monster_tag: v.mons,
        experience: v.experience as u8,
        field_8f_plus_one: v.field_11,
        permutation: v.permutation as u8,
        name_index: v.field_13,
        name: mandrake_rec::mac_roman::decode(&v.name),
    }
}

impl TflSave {
    pub(crate) fn of_film(
        film: Vec<u8>,
        permanent: Vec<PermanentRanking>,
        snapshot: Option<Snapshot>,
    ) -> Self {
        let veterans = mandrake_tfl_sim::film_monster_initializers(&film).unwrap_or_default();
        Self {
            film,
            permanent,
            veterans,
            snapshot,
        }
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        self.rankings_to(&mut out);
        out.extend_from_slice(&(self.veterans.len() as u32).to_be_bytes());
        out.extend(mandrake_tfl_sim::statistics::encode_monster_initializers(
            &self.veterans,
        ));
        match &self.snapshot {
            Some(s) => {
                out.extend_from_slice(&s.width.to_be_bytes());
                out.extend_from_slice(&s.height.to_be_bytes());
                out.extend(s.to_bytes());
            }
            None => out.extend_from_slice(&[0; 8]),
        }
        out.extend_from_slice(&self.film);
        out
    }

    fn rankings_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&(self.permanent.len() as u32).to_be_bytes());
        for r in &self.permanent {
            out.extend_from_slice(&r.player.to_be_bytes());
            out.extend_from_slice(&r.id.to_be_bytes());
            for v in [r.points, r.losses, r.kills, r.damage] {
                out.extend_from_slice(&v.to_be_bytes());
            }
            for p in r.places {
                out.extend_from_slice(&p.to_be_bytes());
            }
            out.extend_from_slice(&[0, 0]);
        }
    }

    pub(crate) fn from_bytes(b: &[u8]) -> Option<Self> {
        let (v2, mut rest) = match (b.strip_prefix(MAGIC), b.strip_prefix(MAGIC_V1)) {
            (Some(rest), _) => (true, rest),
            (None, Some(rest)) => (false, rest),
            (None, None) => return None,
        };
        let count = take_u32(&mut rest)? as usize;
        let rankings = take(&mut rest, count.checked_mul(RANKING)?)?;
        let i16_at = |r: &[u8], o: usize| i16::from_be_bytes([r[o], r[o + 1]]);
        let i32_at = |r: &[u8], o: usize| i32::from_be_bytes(r[o..o + 4].try_into().unwrap());
        let permanent = rankings
            .as_chunks::<RANKING>()
            .0
            .iter()
            .map(|r| PermanentRanking {
                player: i16_at(r, 0),
                id: i16_at(r, 2),
                points: i32_at(r, 4),
                losses: i32_at(r, 8),
                kills: i32_at(r, 0xc),
                damage: i32_at(r, 0x10),
                places: [i16_at(r, 0x14), i16_at(r, 0x16), i16_at(r, 0x18)],
            })
            .collect();
        let (veterans, snapshot) = if v2 {
            let count = take_u32(&mut rest)? as usize;
            let veterans = veterans_of(take(
                &mut rest,
                count.checked_mul(MonsterInitializer::SIZE)?,
            )?);
            let (width, height) = (take_u32(&mut rest)?, take_u32(&mut rest)?);
            let size = (width as usize)
                .checked_mul(height as usize)?
                .checked_mul(4)?;
            let pixels = take(&mut rest, size)?;
            let snapshot = (size > 0).then(|| Snapshot {
                width,
                height,
                rgba: pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|p| [p[1], p[2], p[3], 0xff])
                    .collect(),
            });
            (Some(veterans), snapshot)
        } else {
            (None, None)
        };
        let film = rest.to_vec();
        mandrake_rec::tfl::TflRecording::from_bytes(&film).ok()?;
        let veterans = veterans.unwrap_or_else(|| {
            mandrake_tfl_sim::film_monster_initializers(&film).unwrap_or_default()
        });
        Some(Self {
            film,
            permanent,
            veterans,
            snapshot,
        })
    }

    pub(crate) fn carry(&self) -> super::TflCarry {
        super::TflCarry {
            veterans: self.veterans.clone(),
            permanent: self.permanent.clone(),
        }
    }

    pub(crate) fn game(&self) -> Option<super::film::SoloGame> {
        let s = super::LiveScript::of_film(&self.film)?;
        Some(super::film::SoloGame {
            mesh: s.mesh,
            difficulty: s.difficulty,
            seed: s.seed,
        })
    }

    pub(crate) fn end_tick(&self) -> u32 {
        mandrake_rec::tfl::TflRecording::from_bytes(&self.film).map_or(0, |r| r.header.end_tick)
    }
}

pub(crate) fn read_save(path: &Path, zone: &LocalZone) -> Option<SaveFile> {
    let bytes = std::fs::read(path).ok()?;
    let save = TflSave::from_bytes(&bytes)?;
    let game = save.game()?;
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    let name = path.file_name()?.to_string_lossy().as_bytes().to_vec();
    let mut subgroup = [b' '; 4];
    for (s, c) in subgroup.iter_mut().zip(&name) {
        *s = c.to_ascii_lowercase();
    }
    Some(SaveFile {
        path: path.to_path_buf(),
        name: Text(name),
        modified,
        date: Text(if modified == 0 {
            Vec::new()
        } else {
            date_string(modified, zone).into_bytes()
        }),
        game: FilmGame {
            mode: super::film::SOLO_GAME as i16,
            mesh: game.mesh,
            difficulty: game.difficulty,
            ..FilmGame::default()
        },
        end_tick: save.end_tick(),
        veterans: save.veterans.iter().map(myth2_veteran).collect(),
        snapshot: save.snapshot,
        subgroup: FourCC(subgroup),
    })
}

pub(crate) fn list_saves(dir: Option<&Path>, new_name: Text) -> SaveFolder {
    let zone = LocalZone::system();
    let mut saves: Vec<SaveFile> = dir
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_none_or(|e| e != "tmp"))
        .filter_map(|p| read_save(&p, &zone))
        .collect();
    saves.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.0.cmp(&b.name.0)));
    SaveFolder {
        any: !saves.is_empty(),
        saves,
        new_name,
    }
}

pub(crate) fn write_save(dir: &Path, name: &[u8], save: &TflSave) -> std::io::Result<SaveFile> {
    let file = mandrake_rec::writer::save_names(name)
        .map(|(_, file, _)| file)
        .ok_or_else(|| std::io::Error::other("not a save's name"))?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(String::from_utf8_lossy(&file).as_ref());
    let tmp = dir.join(format!("{}.tmp", String::from_utf8_lossy(&file)));
    std::fs::write(&tmp, save.to_bytes())?;
    std::fs::rename(&tmp, &path)?;
    read_save(&path, &LocalZone::system())
        .ok_or_else(|| std::io::Error::other("the save doesn't read back"))
}
