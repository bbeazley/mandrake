use std::path::Path;

use mandrake_common::game_options;
use mandrake_rec::header::MonsterInitializer;

use crate::net::{GameDescription, Lobby, PlayerData};
use crate::text::Text;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DotCommand {
    Version,
    Count,
    Host,
    Plugins,
    Info,
    UserId,
    Vets,
    LoadVets(Text),
    SaveVets(Text),
    Ping,
    Seeds,
    Help,
}

pub const DEFAULT_VETERANS_FILE: &[u8] = b"vets";
pub const VETERANS_FOLDER: &str = "veterans";
pub const PREFERENCE_SHOW_PING: u32 = 0x40_0000;

const VERSION_LINE: &[u8] = b"You are using Myth II |iv1.8.5 - |pBuild 471b|i (x86_64)|p.";
const HELP_LINE: &[u8] = b"Available commands: .count, .help, .host, .info, .ping, .plugins, \
                           .userid, .version, .vets, .loadvets, .savevets";
const USERID_LINE: &[u8] = b"The .userid command can only be used in metaserver hosted games.";
const VETS_NOT_HOST: &[u8] = b"You must be the game host to use the .vets command.";
const VETS_DISABLED: &[u8] = b"Veterans disabled.";
const VETS_NONE: &[u8] = b"Veterans enabled; no veterans present.";
const VETS_PRESENT: &[u8] = b"Veterans enabled and present.";
const PASSWORD_LINE: &[u8] = b"Game password has been set.";
const ACTIVE_PLUGINS: &[u8] = b"Active Plugins:";
const NO_PLUGINS: &[u8] = b"No plugins are currently active.";
const NO_LEVELS_MARK: &[u8] = b"\xc6 ";
const SAVE_VETERANS: &[u8] = b"Save Veterans";
const LOAD_VETERANS: &[u8] = b"Load Veterans";
const NO_VETERANS_TO_SAVE: &[u8] = b"No veterans to save";
const ERROR_WRITING: &[u8] = b"Error writing file";
const ERROR_READING: &[u8] = b"Error reading file";
const NOT_COOP_HOST: &[u8] = b"You must be the game host of a co-op game to use this command";
const OK: &[u8] = b"OK";

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

pub fn parse(text: &[u8]) -> Option<DotCommand> {
    Some(match text {
        b".version" => DotCommand::Version,
        b".count" => DotCommand::Count,
        b".host" => DotCommand::Host,
        b".plugins" => DotCommand::Plugins,
        b".info" => DotCommand::Info,
        b".userid" => DotCommand::UserId,
        b".vets" => DotCommand::Vets,
        b".ping" => DotCommand::Ping,
        b".seeds" => DotCommand::Seeds,
        b".help" => DotCommand::Help,
        _ if text.starts_with(b".loadvets") || text.starts_with(b".savevets") => {
            let mut name = &text[9..];
            while name.first().is_some_and(|&b| is_space(b)) {
                name = &name[1..];
            }
            let name = Text(if name.is_empty() {
                DEFAULT_VETERANS_FILE.to_vec()
            } else {
                name.to_vec()
            });
            if text[1] == b's' {
                DotCommand::SaveVets(name)
            } else {
                DotCommand::LoadVets(name)
            }
        }
        _ => return None,
    })
}

pub fn goes_nowhere(text: &[u8]) -> bool {
    text.first() == Some(&b'.') && text.get(1).is_some_and(u8::is_ascii_alphabetic)
}

pub fn count_line(lobby: &Lobby) -> Vec<u8> {
    format!(
        "Players in game: {} / {}",
        lobby.players.len(),
        lobby.description.parameters.max_players
    )
    .into_bytes()
}

pub fn host_line(lobby: &Lobby) -> Option<Vec<u8>> {
    let host = lobby.players.iter().find(|p| p.net_id == 0)?;
    let mut line = b"Game host: ".to_vec();
    line.extend_from_slice(player_name(host));
    Some(line)
}

fn player_name(p: &PlayerData) -> &[u8] {
    let name = &p.appearance.name;
    &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())]
}

pub fn seeds_line(description: &GameDescription) -> Vec<u8> {
    let p = &description.parameters;
    format!("Random: {}, Team: {}", p.seed as i32, p.seed_low).into_bytes()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPlugin {
    pub name: Vec<u8>,
    pub checksum: u32,
    pub has_levels: bool,
}

pub fn plugin_lines(description: &GameDescription, local: &[LocalPlugin]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    if description.parameters.plugin_count != 0 {
        for plugin in &description.plugins.0 {
            let Some(found) = local.iter().find(|l| {
                l.checksum == plugin.checksum && l.name.eq_ignore_ascii_case(&plugin.name)
            }) else {
                continue;
            };
            if lines.is_empty() {
                lines.push(ACTIVE_PLUGINS.to_vec());
            }
            let mut line = b"  ".to_vec();
            if !found.has_levels {
                line.extend_from_slice(NO_LEVELS_MARK);
            }
            line.extend_from_slice(&found.name);
            line.push(b'\r');
            lines.push(line);
        }
    }
    if lines.is_empty() {
        lines.push(NO_PLUGINS.to_vec());
    }
    lines
}

pub fn vets_line(host: bool, options: u32, veterans: usize) -> &'static [u8] {
    if !host {
        VETS_NOT_HOST
    } else if options & game_options::VETERANS == 0 {
        VETS_DISABLED
    } else if veterans == 0 || options & game_options::CARRY_VETERANS == 0 {
        VETS_NONE
    } else {
        VETS_PRESENT
    }
}

fn file_line(title: &[u8], name: &[u8], result: &[u8]) -> Vec<u8> {
    let mut line = title.to_vec();
    line.extend_from_slice(b" |K");
    line.extend_from_slice(name);
    line.extend_from_slice(b"|0: ");
    line.extend_from_slice(result);
    line.push(b'.');
    line
}

pub fn save_line(name: &[u8], saved: Option<bool>) -> Vec<u8> {
    let result = match saved {
        None => NO_VETERANS_TO_SAVE,
        Some(false) => ERROR_WRITING,
        Some(true) => OK,
    };
    file_line(SAVE_VETERANS, name, result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadRefused {
    NotCoopHost,
    Unreadable,
}

pub fn load_line(name: &[u8], result: Result<(), LoadRefused>) -> Vec<u8> {
    let result = match result {
        Ok(()) => OK,
        Err(LoadRefused::NotCoopHost) => NOT_COOP_HOST,
        Err(LoadRefused::Unreadable) => ERROR_READING,
    };
    file_line(LOAD_VETERANS, name, result)
}

pub fn lines(command: &DotCommand, lobby: &Lobby, local: &[LocalPlugin]) -> Vec<Vec<u8>> {
    match command {
        DotCommand::Version => vec![VERSION_LINE.to_vec()],
        DotCommand::Count => vec![count_line(lobby)],
        DotCommand::Host => host_line(lobby).into_iter().collect(),
        DotCommand::Plugins => plugin_lines(&lobby.description, local),
        DotCommand::Info => {
            let mut lines =
                vec![[b"Game name: ".as_slice(), &lobby.description.game_name].concat()];
            lines.extend(host_line(lobby));
            if lobby.description.flags & mandrake_net::game_description::FLAG_PASSWORD != 0 {
                lines.push(PASSWORD_LINE.to_vec());
            }
            lines.push(count_line(lobby));
            lines.extend(plugin_lines(&lobby.description, local));
            lines.push(VERSION_LINE.to_vec());
            lines
        }
        DotCommand::UserId => vec![USERID_LINE.to_vec()],
        DotCommand::Seeds => vec![seeds_line(&lobby.description)],
        DotCommand::Help => vec![HELP_LINE.to_vec()],
        DotCommand::Vets | DotCommand::LoadVets(_) | DotCommand::SaveVets(_) | DotCommand::Ping => {
            Vec::new()
        }
    }
}

pub fn save_veterans(game_dir: &Path, name: &[u8], block: &[u8]) -> Option<bool> {
    let _ = std::fs::create_dir_all(game_dir.join(VETERANS_FOLDER));
    if block.is_empty() {
        return None;
    }
    let Ok(path) = crate::films::file_in(game_dir, VETERANS_FOLDER, name) else {
        return Some(false);
    };
    Some(std::fs::write(&path, block).is_ok())
}

pub fn load_veterans(game_dir: &Path, name: &[u8]) -> Option<Vec<MonsterInitializer>> {
    let path = crate::films::file_in(game_dir, VETERANS_FOLDER, name).ok()?;
    let bytes = std::fs::read(path).ok()?;
    parse_veterans(&bytes)
}

pub fn parse_veterans(bytes: &[u8]) -> Option<Vec<MonsterInitializer>> {
    const SIZE: usize = MonsterInitializer::SIZE;
    if bytes.len() < SIZE
        || !bytes.len().is_multiple_of(SIZE)
        || bytes.len() > MonsterInitializer::MAX_BLOCK_BYTES as usize
    {
        return None;
    }
    let (records, _) = bytes.as_chunks::<SIZE>();
    Some(
        records
            .iter()
            .map(|r| {
                let word = |at: usize| [r[at], r[at + 1], r[at + 2], r[at + 3]];
                let raw_name = &r[0x16..SIZE - 1];
                let len = raw_name
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(raw_name.len());
                MonsterInitializer {
                    player_id: u32::from_be_bytes(word(0x04)),
                    unit_tag: mandrake_common::FourCC(word(0x08)),
                    monster_tag: mandrake_common::FourCC(word(0x0c)),
                    experience: r[0x10],
                    field_8f_plus_one: r[0x11],
                    permutation: r[0x12],
                    name_index: r[0x13],
                    name: mandrake_common::mac_roman::decode(&raw_name[..len]),
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dot_and_a_letter_goes_nowhere() {
        assert!(goes_nowhere(b".hosts"));
        assert!(goes_nowhere(b".x"));
        assert!(!goes_nowhere(b"."));
        assert!(!goes_nowhere(b".5 apples"));
        assert!(!goes_nowhere(b". hi"));
        assert!(!goes_nowhere(b"hi."));
    }

    #[test]
    fn plugins_are_the_descriptions_found_here() {
        use mandrake_net::game_description::Plugin;
        let mut d = crate::net::host_description(&Text::default(), &Text::default(), 0);
        let plugin = |name: &[u8], checksum| Plugin {
            checksum,
            name: name.to_vec(),
            url: Vec::new(),
        };
        let local = [
            LocalPlugin {
                name: b"Chimera Multiplayer".to_vec(),
                checksum: 7,
                has_levels: true,
            },
            LocalPlugin {
                name: b"Unit Pack".to_vec(),
                checksum: 9,
                has_levels: false,
            },
        ];
        assert_eq!(
            plugin_lines(&d, &local),
            [b"No plugins are currently active.".to_vec()]
        );
        d.plugins.0 = vec![
            plugin(b"Myth II 1.8.5", 0xddc1_f2d4),
            plugin(b"unit pack", 9),
            plugin(b"Chimera Multiplayer", 8),
            plugin(b"chimera multiplayer", 7),
        ];
        d.parameters.plugin_count = 4;
        assert_eq!(
            plugin_lines(&d, &local),
            [
                b"Active Plugins:".to_vec(),
                b"  \xc6 Unit Pack\r".to_vec(),
                b"  Chimera Multiplayer\r".to_vec(),
            ]
        );
        d.parameters.plugin_count = 0;
        assert_eq!(plugin_lines(&d, &local).len(), 1);
    }

    #[test]
    fn a_veterans_file_round_trips() {
        let table = vec![MonsterInitializer {
            player_id: 3,
            unit_tag: mandrake_common::FourCC(*b"mysm"),
            monster_tag: mandrake_common::FourCC(*b"mysn"),
            experience: 5,
            field_8f_plus_one: 1,
            permutation: 2,
            name_index: 4,
            name: "Xeno".to_string(),
        }];
        let mut block = mandrake_rec::writer::initializer_block(&table);
        assert_eq!(parse_veterans(&block), Some(table.clone()));
        block[0x27] = b'z';
        block[0x16..0x27].fill(b'a');
        assert_eq!(parse_veterans(&block).unwrap()[0].name, "a".repeat(0x11));
        assert_eq!(parse_veterans(&block[..39]), None);
        assert_eq!(parse_veterans(&[]), None);
        assert_eq!(parse_veterans(&[0; 41]), None);
        let dir = std::env::temp_dir().join(format!("m2-vets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bytes = mandrake_rec::writer::initializer_block(&table);
        assert_eq!(save_veterans(&dir, b"none", &[]), None);
        assert!(dir.join("veterans").is_dir() && !dir.join("veterans/none").exists());
        assert_eq!(save_veterans(&dir, b"vets", &bytes), Some(true));
        assert_eq!(std::fs::read(dir.join("veterans/vets")).unwrap(), bytes);
        assert_eq!(load_veterans(&dir, b"vets"), Some(table));
        assert_eq!(load_veterans(&dir, b"other"), None);
        assert_eq!(save_veterans(&dir, b"../x", &bytes), Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
