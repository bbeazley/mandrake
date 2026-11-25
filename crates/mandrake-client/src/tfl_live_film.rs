use mandrake_common::FourCC;
use mandrake_rec::tfl::{TflFilmPlayer, TflGameParameters, TflNewGame};
use mandrake_rec::{CommandPacket, TickCommands};

const SOLO_WORD_2: u16 = 11;
const DEFAULT_OPTIONS: u32 = 0xe034;
const SOLO_OPTIONS: u32 = 0xe08c;
const DEFAULT_WORD_12: u16 = 0x10;
const DEFAULT_WORD_16: u16 = 0xffff;
pub(crate) const SOLO_GAME: u16 = 2;
pub(crate) const CHECKSUMS: [u32; 2] = [0x1841_8fcf, 0x22cb_50e1];

struct Team {
    name: &'static [u8],
    colour: [u16; 3],
    word_2: u16,
}

const TEAMS: [Team; 3] = [
    Team {
        name: b"Light",
        colour: [0xffff, 0xffff, 0xffff],
        word_2: 0,
    },
    Team {
        name: b"Dark",
        colour: [0x4000, 0x4000, 0x4000],
        word_2: 1,
    },
    Team {
        name: b"Neutral",
        colour: [0x4000, 0, 0],
        word_2: 1,
    },
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SoloGame {
    pub(crate) mesh: FourCC,
    pub(crate) difficulty: i16,
    pub(crate) seed: u32,
}

impl SoloGame {
    pub(crate) fn now(mesh: FourCC, difficulty: i16) -> Self {
        let ms = crate::clock::SystemTime::now()
            .duration_since(crate::clock::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u32);
        Self {
            mesh,
            difficulty,
            seed: ms,
        }
    }
}

pub(crate) const VETERANS_OPTION: u32 = 0x40;

pub(crate) fn solo_new_game(game: &SoloGame, veterans: bool) -> TflNewGame {
    let veterans = if veterans { VETERANS_OPTION } else { 0 };
    TflNewGame {
        parameters: TflGameParameters {
            mode: SOLO_GAME as i16,
            game_type: SOLO_WORD_2 as i16,
            options: DEFAULT_OPTIONS | SOLO_OPTIONS | veterans,
            time_limit: 0,
            mesh: game.mesh,
            difficulty: game.difficulty,
            max_players: DEFAULT_WORD_12 as i16,
            field_14: game.seed as u16 as i16,
            team_size: DEFAULT_WORD_16 as i16,
            seed: game.seed,
            planning_time: 0,
            field_20: [0; 8],
        },
        local_player: i16::from(crate::tfl_live::LOCAL_PLAYER),
        players: TEAMS
            .iter()
            .enumerate()
            .map(|(k, team)| {
                let colour = [team.colour[0], team.colour[1], team.colour[2], 0];
                TflFilmPlayer {
                    slot: k,
                    name: String::from_utf8_lossy(team.name).into_owned(),
                    team_name: String::new(),
                    primary: colour,
                    secondary: colour,
                    team: k as i16,
                    field_02: team.word_2 as i16,
                    flags: 0,
                    captain: -1,
                    net_id: 0,
                    field_0a: 0,
                    metaserver_id: 0,
                    field_10: [0; 8],
                    appearance_00: 0,
                    appearance_02: 0,
                }
            })
            .collect(),
    }
}

pub(crate) fn solo_film(
    game: &SoloGame,
    name: &[u8],
    id: FourCC,
    end_tick: u32,
    veterans: &[u8],
    commands: &[TickCommands],
) -> Vec<u8> {
    mandrake_rec::tfl::tfl_film(
        &solo_new_game(game, !veterans.is_empty()),
        CHECKSUMS,
        name,
        id,
        end_tick,
        veterans,
        commands,
    )
}

pub(crate) fn movement_payload(
    flags: u16,
    formation: u16,
    facing: u16,
    monsters: &[i16],
    waypoints: &[(i32, i32)],
) -> Vec<u8> {
    let mut p = Vec::with_capacity(12 + 2 * monsters.len() + 8 * waypoints.len());
    for w in [flags, formation, facing, 0] {
        p.extend_from_slice(&w.to_be_bytes());
    }
    p.extend_from_slice(&(monsters.len() as i16).to_be_bytes());
    for m in monsters {
        p.extend_from_slice(&m.to_be_bytes());
    }
    p.extend_from_slice(&(waypoints.len() as i16).to_be_bytes());
    for (x, y) in waypoints {
        p.extend_from_slice(&x.to_be_bytes());
        p.extend_from_slice(&y.to_be_bytes());
    }
    p
}

pub(crate) fn movement_command(tick: u32, player: u8, payload: &[u8]) -> CommandPacket {
    CommandPacket::parse(
        tick,
        player,
        mandrake_rec::CommandType::Movement.to_u8(),
        payload,
    )
}

fn put_list(p: &mut Vec<u8>, ids: &[i16]) {
    p.extend_from_slice(&(ids.len() as i16).to_be_bytes());
    for id in ids {
        p.extend_from_slice(&id.to_be_bytes());
    }
}

pub(crate) const GENERAL_COMMAND: u8 = 2;
pub(crate) const TARGET_COMMAND: u8 = 4;
pub(crate) const ATTACK_LOCATION_COMMAND: u8 = 5;
pub(crate) const PICK_UP_COMMAND: u8 = 6;
pub(crate) const RENAME_COMMAND: u8 = 7;
pub(crate) const DETACH_COMMAND: u8 = 10;

pub(crate) fn general_payload(code: u16, monsters: &[i16]) -> Vec<u8> {
    let mut p = Vec::with_capacity(6 + 2 * monsters.len());
    p.extend_from_slice(&0u16.to_be_bytes());
    p.extend_from_slice(&code.to_be_bytes());
    put_list(&mut p, monsters);
    p
}

pub(crate) fn target_payload(flags: u16, monsters: &[i16], targets: &[i16]) -> Vec<u8> {
    let mut p = Vec::with_capacity(6 + 2 * (monsters.len() + targets.len()));
    p.extend_from_slice(&flags.to_be_bytes());
    put_list(&mut p, monsters);
    put_list(&mut p, targets);
    p
}

pub(crate) fn attack_location_payload(
    flags: u16,
    point: (i32, i32, i32),
    monsters: &[i16],
) -> Vec<u8> {
    let mut p = Vec::with_capacity(0x12 + 2 * monsters.len());
    p.extend_from_slice(&flags.to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    for v in [point.0, point.1, point.2] {
        p.extend_from_slice(&v.to_be_bytes());
    }
    put_list(&mut p, monsters);
    p
}

pub(crate) fn pick_up_payload(object: i16, object_word: i16, monsters: &[i16]) -> Vec<u8> {
    let mut p = Vec::with_capacity(10 + 2 * monsters.len());
    p.extend_from_slice(&[0, 0, 0, 0]);
    p.extend_from_slice(&object.to_be_bytes());
    p.extend_from_slice(&object_word.to_be_bytes());
    put_list(&mut p, monsters);
    p
}

pub(crate) fn detach_payload(player: i16, monsters: &[i16]) -> Vec<u8> {
    let mut p = Vec::with_capacity(6 + 2 * monsters.len());
    p.extend_from_slice(&0u16.to_be_bytes());
    p.extend_from_slice(&player.to_be_bytes());
    put_list(&mut p, monsters);
    p
}

pub(crate) fn rename_payload(monsters: &[i16], name: &[u8]) -> Vec<u8> {
    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
    let name = &name[..name.len().min(0x11)];
    let mut p = Vec::with_capacity(5 + 2 * monsters.len() + name.len());
    p.extend_from_slice(&0u16.to_be_bytes());
    put_list(&mut p, monsters);
    p.extend_from_slice(name);
    p.push(0);
    p
}

pub(crate) fn command(command_type: u8, payload: &[u8]) -> CommandPacket {
    CommandPacket::parse(0, 0, command_type, payload)
}
