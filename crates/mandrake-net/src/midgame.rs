use std::collections::BTreeMap;

use mandrake_common::game_options;
use mandrake_rec::writer::NewGameData;

use crate::game_description::{GameParameters, new_game_players};
use crate::gameplay::{
    compute_gameplay_version, gameplay_version_from_version_id, gameplay_version_to_string,
};
use crate::player_data::{PLAYER_DATA_SIZE, PlayerData};
use crate::wire::{Body, Command, Message};

pub const BATCH_BYTES: usize = 0x1000;
pub const LAST_COMMAND_TICKS: i32 = 900;
pub const COMMAND_ADD_PLAYER: u8 = 0x16;
pub const ADD_PLAYER_SIZE: usize = 0x60;
pub const MAX_SIM_PLAYERS: usize = 0x7f;

const CHECKED_BELOW: u16 = 0x88a;

pub const TOO_OLD: &[u8] = b"Mid-game joining is only supported in Myth II 1.8.2 or later.";
pub const NO_MORE_PLAYERS: &[u8] = b"Mid-game join failed as no more players are able to join.";

pub fn minimum_build(records: &[PlayerData]) -> i16 {
    if records.is_empty() {
        return 0x7fff;
    }
    let low = records
        .iter()
        .filter(|r| r.field_02 == 0)
        .map(|r| r.build as i16)
        .fold(0x7fff, i16::min);
    if low == 0 { -1 } else { low }
}

pub fn game_gameplay(params: &GameParameters, start: &[PlayerData]) -> i16 {
    compute_gameplay_version(params.options, params.version as i32, minimum_build(start))
}

pub fn reopens(params: &GameParameters, gameplay: i16) -> bool {
    let gameplay = i32::from(gameplay);
    gameplay > game_options::GAMEPLAY_1_8_4
        || (params.options & game_options::COOPERATIVE != 0
            && gameplay >= game_options::GAMEPLAY_1_8_2)
}

pub fn check_join(
    record: &PlayerData,
    gameplay: i16,
    not_arrived: usize,
    sim_players: usize,
) -> Result<(), Vec<u8>> {
    if record.version < game_options::VERSION_ID_1_8_2 {
        return Err(TOO_OLD.to_vec());
    }
    if record.version < CHECKED_BELOW
        && gameplay_version_from_version_id(i32::from(record.version), record.build as i16)
            < gameplay
    {
        return Err(format!(
            "Mid-game join failed as the game is using a newer gameplay version ({}) than your \
             client.",
            gameplay_version_to_string(gameplay)
        )
        .into_bytes());
    }
    if not_arrived + sim_players == MAX_SIM_PLAYERS {
        return Err(NO_MORE_PLAYERS.to_vec());
    }
    Ok(())
}

pub fn begin_midgame(to: u8, from: u8, start: &NewGameData) -> Message {
    let (count, records) = new_game_players(start);
    let mut raw = count.to_be_bytes().to_vec();
    raw.extend_from_slice(records);
    Message::new(to, from, Body::BeginGameMidgame(raw))
}

pub fn start_records(start: &NewGameData) -> Vec<PlayerData> {
    let (_, records) = new_game_players(start);
    records
        .as_chunks::<PLAYER_DATA_SIZE>()
        .0
        .iter()
        .filter_map(|r| PlayerData::read(r).ok())
        .collect()
}

pub fn add_player_command(net_id: u8, record: &PlayerData, options: u32) -> Command {
    let team: i16 = if options & game_options::COOPERATIVE != 0 {
        0
    } else {
        -1
    };
    let mut d = Vec::with_capacity(ADD_PLAYER_SIZE);
    d.extend_from_slice(&i32::from(net_id).to_be_bytes());
    d.extend_from_slice(&team.to_be_bytes());
    d.extend_from_slice(&record.version.to_be_bytes());
    d.extend_from_slice(&record.metaserver_id.to_be_bytes());
    d.extend_from_slice(&record.to_bytes()[0x28..PLAYER_DATA_SIZE]);
    debug_assert_eq!(d.len(), ADD_PLAYER_SIZE);
    Command {
        kind: COMMAND_ADD_PLAYER,
        player: crate::client::UNSTAMPED_PLAYER,
        tick: 0,
        payload: d,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Catchup {
    pub cursor: Option<usize>,
    pub wants_more: bool,
}

pub fn batch(
    history: &[Command],
    cursor: usize,
    to: u8,
    from: u8,
) -> (Vec<Message>, Option<usize>) {
    let mut budget = BATCH_BYTES;
    let mut out = Vec::new();
    let Some(first) = history.get(cursor) else {
        return (out, None);
    };
    let first_tick = first.tick;
    let mut at = cursor;
    while let Some(c) = history.get(at) {
        if at + 1 == history.len() && c.tick.wrapping_sub(first_tick) > LAST_COMMAND_TICKS {
            break;
        }
        let m = Message::new(to, from, Body::Command(c.clone()));
        let size = m.encode().map_or(usize::MAX, |b| b.len());
        if size > budget {
            break;
        }
        budget -= size;
        out.push(m);
        at += 1;
    }
    (out, (at < history.len()).then_some(at))
}

#[derive(Debug, Clone, Default)]
pub struct Midgame {
    pub start: Option<NewGameData>,
    pub gameplay: i16,
    pub joining: BTreeMap<u8, Option<Catchup>>,
    pub added: usize,
}

impl Midgame {
    pub fn sim_players(&self) -> usize {
        self.start
            .as_ref()
            .map_or(0, |s| new_game_players(s).0 as usize)
            + self.added
    }
}
