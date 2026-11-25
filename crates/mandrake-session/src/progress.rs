use mandrake_common::FourCC;

pub const MAX_VISITED: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisitedLevel {
    pub mesh: FourCC,
    pub plugin_checksum: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub visited: Vec<VisitedLevel>,
    pub last_level: Option<FourCC>,
    pub last_campaign: Option<i32>,
    pub difficulty: i16,
    pub show_hints: bool,
    pub hint_index: i16,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            visited: Vec::new(),
            last_level: None,
            last_campaign: None,
            difficulty: 2,
            show_hints: true,
            hint_index: 0,
        }
    }
}

impl Progress {
    pub fn is_visited(&self, mesh: FourCC, plugin_checksum: i32) -> bool {
        self.visited.contains(&VisitedLevel {
            mesh,
            plugin_checksum,
        })
    }

    pub fn mark_visited(&mut self, mesh: FourCC, plugin_checksum: i32) -> bool {
        self.mark_visited_up_to(mesh, plugin_checksum, MAX_VISITED)
    }

    pub fn mark_visited_up_to(&mut self, mesh: FourCC, plugin_checksum: i32, max: usize) -> bool {
        if self.is_visited(mesh, plugin_checksum) || self.visited.len() >= max {
            return false;
        }
        self.visited.push(VisitedLevel {
            mesh,
            plugin_checksum,
        });
        true
    }

    pub fn first_hint(&self, count: i32) -> i32 {
        let saved = i32::from(self.hint_index);
        if saved < 0 { 0 } else { saved.min(count) }
    }

    pub fn to_text(&self) -> String {
        let hex = |tag: FourCC| format!("{:08x}", u32::from_be_bytes(tag.0));
        let mut text = format!(
            "# Myth II client progress, written by the client.\nversion {PROGRESS_VERSION}\n\
             difficulty {}\nshow_hints {}\nhint_index {}\n",
            self.difficulty, self.show_hints, self.hint_index
        );
        if let Some(mesh) = self.last_level {
            text += &format!("last_level {}\n", hex(mesh));
        }
        if let Some(checksum) = self.last_campaign {
            text += &format!("last_campaign {checksum}\n");
        }
        for level in &self.visited {
            text += &format!("visited {} {}\n", hex(level.mesh), level.plugin_checksum);
        }
        text
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut progress = Self {
            visited: Vec::new(),
            ..Self::default()
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || format!("progress: bad line: {line}");
            let mut words = line.split_ascii_whitespace();
            let key = words.next().ok_or_else(bad)?;
            let mut value = || words.next().ok_or_else(bad);
            match key {
                "version" => {
                    let version: u32 = value()?.parse().map_err(|_| bad())?;
                    if version > PROGRESS_VERSION {
                        return Err(format!("progress: version {version} is too new"));
                    }
                }
                "difficulty" => progress.difficulty = value()?.parse().map_err(|_| bad())?,
                "show_hints" => progress.show_hints = value()?.parse().map_err(|_| bad())?,
                "hint_index" => progress.hint_index = value()?.parse().map_err(|_| bad())?,
                "last_level" => progress.last_level = Some(parse_tag(value()?).ok_or_else(bad)?),
                "last_campaign" => {
                    progress.last_campaign = Some(value()?.parse().map_err(|_| bad())?);
                }
                "visited" => {
                    let mesh = parse_tag(value()?).ok_or_else(bad)?;
                    let plugin_checksum = value()?.parse().map_err(|_| bad())?;
                    progress.visited.push(VisitedLevel {
                        mesh,
                        plugin_checksum,
                    });
                }
                _ => {}
            }
        }
        Ok(progress)
    }

    pub fn import_game2(file: &[u8]) -> Result<Self, String> {
        let group = file.get(GAME2_GROUP_AT..GAME2_GROUP_AT + 8);
        if group != Some(b"prefga2e".as_slice()) {
            return Err("game 2: not the game preferences".to_owned());
        }
        let body = file.get(GAME2_HEADER_SIZE..).unwrap_or_default();
        if body.len() < GAME2_MIN_BODY {
            return Err(format!("game 2: {} bytes is too short", body.len()));
        }
        let word = |o: usize| [body[o], body[o + 1], body[o + 2], body[o + 3]];
        let short = |o: usize| i16::from_be_bytes([body[o], body[o + 1]]);

        let count = usize::try_from(short(GAME2_VISITED_COUNT))
            .unwrap_or(0)
            .min(MAX_VISITED);
        let visited = (0..count)
            .map(|i| VisitedLevel {
                mesh: FourCC(word(GAME2_VISITED_MESHES + i * 4)),
                plugin_checksum: i32::from_be_bytes(word(GAME2_VISITED_PLUGINS + i * 4)),
            })
            .collect();

        let defaults = Self::default();
        let difficulty = short(GAME2_NEW_GAME + NEW_GAME_DIFFICULTY);
        let last_level = u32::from_be_bytes(word(GAME2_NEW_GAME + NEW_GAME_MESH));
        Ok(Self {
            visited,
            last_level: (!matches!(last_level, 0 | u32::MAX))
                .then(|| FourCC(last_level.to_be_bytes())),
            last_campaign: Some(i32::from_be_bytes(word(GAME2_LAST_CAMPAIGN))),
            difficulty: if (0..5).contains(&difficulty) {
                difficulty
            } else {
                defaults.difficulty
            },
            show_hints: u32::from_be_bytes(word(GAME2_FLAGS)) & GAME2_FLAG_SHOW_HINTS != 0,
            hint_index: short(GAME2_HINT_INDEX),
        })
    }
}

const PROGRESS_VERSION: u32 = 1;

fn parse_tag(hex: &str) -> Option<FourCC> {
    let value = u32::from_str_radix(hex, 16).ok()?;
    (hex.len() == 8).then(|| FourCC(value.to_be_bytes()))
}

const GAME2_GROUP_AT: usize = 0x24;
const GAME2_HEADER_SIZE: usize = 64;
const GAME2_FLAGS: usize = 0;
const GAME2_FLAG_SHOW_HINTS: u32 = 0x100;
const GAME2_HINT_INDEX: usize = 6;
const GAME2_LAST_CAMPAIGN: usize = 0x4c;
const GAME2_NEW_GAME: usize = 0x58;
const NEW_GAME_MESH: usize = 0xc;
const NEW_GAME_DIFFICULTY: usize = 0x10;
const GAME2_VISITED_COUNT: usize = 0x4b0;
const GAME2_VISITED_MESHES: usize = 0x4b4;
const GAME2_VISITED_PLUGINS: usize = 0xcb4;
const GAME2_MIN_BODY: usize = GAME2_VISITED_PLUGINS + MAX_VISITED * 4;

#[cfg(test)]
mod tests {
    use super::*;

    const WILLOW: FourCC = FourCC(*b"l1  ");
    const SALVATION: FourCC = FourCC(*b"cr01");
    const QUOTED: FourCC = FourCC(*b"i'do");

    fn game2_fixture(flags: u32, visited: &[(FourCC, i32)]) -> Vec<u8> {
        let mut file = vec![0u8; GAME2_HEADER_SIZE + GAME2_MIN_BODY];
        file[4..10].copy_from_slice(b"game 2");
        file[GAME2_GROUP_AT..GAME2_GROUP_AT + 8].copy_from_slice(b"prefga2e");
        file[0x3c..0x40].copy_from_slice(b"mth2");
        let body = &mut file[GAME2_HEADER_SIZE..];
        body[..4].copy_from_slice(&flags.to_be_bytes());
        body[4..6].copy_from_slice(&1i16.to_be_bytes());
        body[GAME2_HINT_INDEX..GAME2_HINT_INDEX + 2].copy_from_slice(&7i16.to_be_bytes());
        body[GAME2_LAST_CAMPAIGN..GAME2_LAST_CAMPAIGN + 4].copy_from_slice(&(-1i32).to_be_bytes());
        let new_game = GAME2_NEW_GAME;
        body[new_game + NEW_GAME_MESH..new_game + NEW_GAME_MESH + 4].copy_from_slice(&SALVATION.0);
        body[new_game + NEW_GAME_DIFFICULTY..new_game + NEW_GAME_DIFFICULTY + 2]
            .copy_from_slice(&3i16.to_be_bytes());
        body[GAME2_VISITED_COUNT..GAME2_VISITED_COUNT + 2]
            .copy_from_slice(&(visited.len() as i16).to_be_bytes());
        for (i, (mesh, plugin)) in visited.iter().enumerate() {
            let at = GAME2_VISITED_MESHES + i * 4;
            body[at..at + 4].copy_from_slice(&mesh.0);
            let at = GAME2_VISITED_PLUGINS + i * 4;
            body[at..at + 4].copy_from_slice(&plugin.to_be_bytes());
        }
        file
    }

    #[test]
    fn progress_round_trips_through_text() {
        let progress = Progress {
            visited: vec![
                VisitedLevel {
                    mesh: WILLOW,
                    plugin_checksum: -1,
                },
                VisitedLevel {
                    mesh: QUOTED,
                    plugin_checksum: 0x1234_5678,
                },
                VisitedLevel {
                    mesh: FourCC([0xd5, 0, 0x20, 0xff]),
                    plugin_checksum: i32::MIN,
                },
            ],
            last_level: Some(WILLOW),
            last_campaign: Some(-1),
            difficulty: 4,
            show_hints: false,
            hint_index: 12,
        };
        assert_eq!(Progress::from_text(&progress.to_text()), Ok(progress));
        let fresh = Progress::default();
        assert_eq!(Progress::from_text(&fresh.to_text()), Ok(fresh));
    }

    #[test]
    fn a_file_it_cannot_read_is_an_error_and_a_new_key_is_not() {
        assert!(Progress::from_text("visited 6c312020").is_err());
        assert!(Progress::from_text("visited l1 -1").is_err());
        assert!(Progress::from_text("difficulty hard").is_err());
        assert!(Progress::from_text("show_hints 1").is_err());
        assert!(Progress::from_text("version 2").is_err());
        let later = Progress::from_text("version 1\nnew_thing 1 2 3\n").unwrap();
        assert_eq!(later, Progress::default());
    }

    #[test]
    fn imports_a_game2_fixture() {
        let file = game2_fixture(
            0x0058_011e,
            &[(WILLOW, -1), (SALVATION, -1), (QUOTED, 0x0102_0304)],
        );
        let progress = Progress::import_game2(&file).unwrap();
        assert!(progress.show_hints);
        assert_eq!(progress.hint_index, 7);
        assert_eq!(progress.last_level, Some(SALVATION));
        assert_eq!(progress.last_campaign, Some(-1));
        assert_eq!(progress.difficulty, 3);
        assert_eq!(progress.visited.len(), 3);
        assert!(progress.is_visited(WILLOW, -1));
        assert!(progress.is_visited(QUOTED, 0x0102_0304));
        assert!(!progress.is_visited(QUOTED, -1));
        assert_eq!(Progress::from_text(&progress.to_text()), Ok(progress));
    }

    #[test]
    fn the_show_hints_flag_is_the_only_bit_read() {
        let off = game2_fixture(0xffff_feff, &[]);
        assert!(!Progress::import_game2(&off).unwrap().show_hints);
        let on = game2_fixture(0x100, &[]);
        assert!(Progress::import_game2(&on).unwrap().show_hints);
    }

    #[test]
    fn import_refuses_what_is_not_game2_and_limits_the_count() {
        let file = game2_fixture(0, &[]);
        assert!(Progress::import_game2(&file[..file.len() - 1]).is_err());
        let mut other = file.clone();
        other[GAME2_GROUP_AT] = b'x';
        assert!(Progress::import_game2(&other).is_err());
        assert!(Progress::import_game2(&[]).is_err());

        let mut many = file;
        let at = GAME2_HEADER_SIZE + GAME2_VISITED_COUNT;
        many[at..at + 2].copy_from_slice(&1000i16.to_be_bytes());
        let progress = Progress::import_game2(&many).unwrap();
        assert_eq!(progress.visited.len(), MAX_VISITED);
        let last_ok = {
            many[at..at + 2].copy_from_slice(&(-1i16).to_be_bytes());
            Progress::import_game2(&many).unwrap()
        };
        assert!(last_ok.visited.is_empty());
    }

    #[test]
    fn no_new_game_yet_leaves_the_choices_alone() {
        let mut file = game2_fixture(0, &[]);
        let at = GAME2_HEADER_SIZE + GAME2_NEW_GAME + NEW_GAME_MESH;
        file[at..at + 4].copy_from_slice(&[0xff; 4]);
        let at = GAME2_HEADER_SIZE + GAME2_NEW_GAME + NEW_GAME_DIFFICULTY;
        file[at..at + 2].copy_from_slice(&99i16.to_be_bytes());
        let progress = Progress::import_game2(&file).unwrap();
        assert_eq!(progress.last_level, None);
        assert_eq!(progress.difficulty, Progress::default().difficulty);
    }

    #[test]
    fn the_first_hint_is_kept_as_the_engine_keeps_it() {
        let at = |hint_index| {
            Progress {
                hint_index,
                ..Progress::default()
            }
            .first_hint(30)
        };
        assert_eq!(at(-1), 0);
        assert_eq!(at(7), 7);
        assert_eq!(at(30), 30);
        assert_eq!(at(31), 30);
    }
}
