use std::path::Path;

use crate::header::SBMonoHeader;
use crate::tfl::gor::read_gor_directory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Game {
    Myth2,
    Tfl,
}

impl Game {
    pub fn detect(game_dir: &Path) -> Option<Self> {
        let vfs = mandrake_common::game_vfs();
        let tags = game_dir.join("tags");
        let tags = tags.to_string_lossy();
        if !vfs.exists(&tags) || vfs.is_file(&tags) {
            return None;
        }
        let files: Vec<_> = vfs
            .list_dir(&tags)
            .into_iter()
            .filter(|p| vfs.is_file(p))
            .collect();
        let is_myth2 = |p: &str| {
            vfs.open_reader(p)
                .ok()
                .and_then(|mut f| SBMonoHeader::read(&mut f).ok())
                .is_some_and(|h| {
                    h.signature == SBMonoHeader::SIG_MTH2 || h.signature == SBMonoHeader::SIG_DNG2
                })
        };
        let is_gor = |p: &str| {
            Path::new(p)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("gor"))
                && vfs
                    .open_reader(p)
                    .ok()
                    .and_then(|mut f| read_gor_directory(&mut f).ok().flatten())
                    .is_some()
        };
        if files.iter().any(|p| is_myth2(p)) {
            Some(Self::Myth2)
        } else if files.iter().any(|p| is_gor(p)) {
            Some(Self::Tfl)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Myth2 => "myth2",
            Self::Tfl => "tfl",
        }
    }
}

impl std::str::FromStr for Game {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "myth2" | "myth ii" | "soulblighter" => Ok(Self::Myth2),
            "tfl" | "fallen lords" => Ok(Self::Tfl),
            _ => Err(format!("unknown game '{s}' (expected myth2 or tfl)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for g in [Game::Myth2, Game::Tfl] {
            assert_eq!(g.as_str().parse::<Game>(), Ok(g));
        }
        assert!("myth3".parse::<Game>().is_err());
    }

    #[test]
    fn a_folder_with_neither_is_none() {
        let dir = std::env::temp_dir().join(format!("mandrake-tag-game-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("tags")).unwrap();
        std::fs::write(dir.join("tags").join("readme.txt"), b"hello").unwrap();
        assert_eq!(Game::detect(&dir), None);
        std::fs::write(
            dir.join("tags").join("TAGS.GOR"),
            crate::tfl::gor::tests::gor(&[(b"mesh", b"00tm", b"x")]),
        )
        .unwrap();
        assert_eq!(Game::detect(&dir), Some(Game::Tfl));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(Game::detect(&dir), None);
    }
}
