use mandrake_common::game_options;
use mandrake_rec::writer::{NEW_GAME_DATA_SIZE, NewGameData};

use crate::gameplay::gameplay_version_from_version_id;
use crate::wire::{Body, Message, TO_ALL};

pub const HEADER_SIZE: usize = NEW_GAME_DATA_SIZE;
pub const FILM_HEAD_SIZE: usize = mandrake_rec::writer::NEW_GAME_DATA_OFFSET;
pub const BODY_SIZE: usize = HEADER_SIZE + 4;

use crate::game_description::{PLAYER_COUNT, PLAYERS};
use crate::player_data::PLAYER_DATA_SIZE as PLAYER_SIZE;

const FLAGS: usize = 0x20;
const OPTIONS: usize = 0x04;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedGame {
    pub header: Vec<u8>,
    pub ticks: u32,
}

impl SavedGame {
    pub fn from_film(film: &[u8]) -> Option<SavedGame> {
        let ticks = u32::from_be_bytes(film.get(8..12)?.try_into().ok()?);
        let header = film.get(FILM_HEAD_SIZE..FILM_HEAD_SIZE + HEADER_SIZE)?;
        Some(SavedGame {
            header: host_header(header),
            ticks,
        })
    }

    pub fn from_body(raw: &[u8]) -> Option<SavedGame> {
        let header = raw.get(..HEADER_SIZE)?.to_vec();
        let ticks = u32::from_be_bytes(raw.get(HEADER_SIZE..BODY_SIZE)?.try_into().ok()?);
        Some(SavedGame { header, ticks })
    }

    pub fn message(&self, from: u8) -> Message {
        let mut raw = self.header.clone();
        raw.resize(HEADER_SIZE, 0);
        raw.extend_from_slice(&self.ticks.to_be_bytes());
        Message::new(TO_ALL, from, Body::SavedGameHeader(raw))
    }

    pub fn new_game_data(&self) -> NewGameData {
        let mut d = self.header.clone();
        d.resize(NEW_GAME_DATA_SIZE, 0);
        NewGameData(d)
    }
}

pub fn host_header(header: &[u8]) -> Vec<u8> {
    let mut d = header.to_vec();
    d.resize(HEADER_SIZE, 0);
    d[0..2].copy_from_slice(&1i16.to_be_bytes());
    let flags = u16::from_be_bytes([d[FLAGS], d[FLAGS + 1]]) | 1;
    d[FLAGS..FLAGS + 2].copy_from_slice(&flags.to_be_bytes());
    let options = u32::from_be_bytes(d[OPTIONS..OPTIONS + 4].try_into().unwrap());
    let options = (options & !game_options::NO_AUTOMATIC_TABLE) | game_options::COOPERATIVE;
    d[OPTIONS..OPTIONS + 4].copy_from_slice(&options.to_be_bytes());
    let count = i16::from_be_bytes([d[PLAYER_COUNT], d[PLAYER_COUNT + 1]]).clamp(0, 16) as usize;
    for i in 0..count {
        let at = PLAYERS + i * PLAYER_SIZE;
        if d[at + 2..at + 4] != [0, 0] {
            d[at + 0x0c..at + 0x10].copy_from_slice(&(-1i32).to_be_bytes());
        }
    }
    d
}

pub fn loaded_from_network_save(parameters: &[u8]) -> bool {
    let Some(p) = parameters.get(..0x28) else {
        return false;
    };
    let build = i16::from_be_bytes([p[0x22], p[0x23]]);
    let version = i32::from_be_bytes([p[0x24], p[0x25], p[0x26], p[0x27]]);
    i32::from(gameplay_version_from_version_id(version, build)) > game_options::GAMEPLAY_1_8_4
        && p[FLAGS + 1] & 1 != 0
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    ticks: std::collections::BTreeMap<u8, u32>,
}

impl Progress {
    pub fn set(&mut self, from: u8, tick: u32) {
        self.ticks.insert(from, tick);
    }

    pub fn of(&self, id: u8) -> u32 {
        self.ticks.get(&id).copied().unwrap_or(0)
    }

    pub fn all_reached(&self, players: &[u8], local: u8, tick: i64) -> bool {
        players
            .iter()
            .all(|&id| id == local || i64::from(self.of(id)) >= tick)
    }

    pub fn clear(&mut self) {
        self.ticks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Vec<u8> {
        let mut d = vec![0u8; HEADER_SIZE];
        d[0..2].copy_from_slice(&2i16.to_be_bytes());
        d[4..8].copy_from_slice(&0x0000_0109u32.to_be_bytes());
        d[0x22..0x24].copy_from_slice(&471u16.to_be_bytes());
        d[0x24..0x28].copy_from_slice(&u32::from(game_options::VERSION_ID_1_8_5).to_be_bytes());
        d[PLAYER_COUNT..PLAYER_COUNT + 2].copy_from_slice(&2i16.to_be_bytes());
        d[PLAYERS + 0x0c..PLAYERS + 0x10].copy_from_slice(&3i32.to_be_bytes());
        let dark = PLAYERS + PLAYER_SIZE;
        d[dark + 2..dark + 4].copy_from_slice(&1i16.to_be_bytes());
        d[dark + 0x0c..dark + 0x10].copy_from_slice(&5i32.to_be_bytes());
        d
    }

    #[test]
    fn the_hosts_header_is_marked_and_its_computer_players_lose_their_ids() {
        let h = host_header(&header());
        assert_eq!(&h[0..2], &1i16.to_be_bytes());
        assert_eq!(u32::from_be_bytes(h[4..8].try_into().unwrap()), 0x181);
        assert!(loaded_from_network_save(&h));
        assert!(!loaded_from_network_save(&header()));
        assert_eq!(&h[PLAYERS + 0x0c..PLAYERS + 0x10], &3i32.to_be_bytes());
        let dark = PLAYERS + PLAYER_SIZE;
        assert_eq!(&h[dark + 0x0c..dark + 0x10], &(-1i32).to_be_bytes());
    }

    #[test]
    fn the_header_message_is_0xa08_bytes_and_reads_back() {
        let save = SavedGame {
            header: host_header(&header()),
            ticks: 1234,
        };
        let bytes = save.message(0).encode().unwrap();
        assert_eq!(bytes.len(), 0xa08);
        assert_eq!(&bytes[0..2], &[0, 0xc]);
        let m = Message::decode(&bytes).unwrap();
        let Body::SavedGameHeader(raw) = m.body else {
            panic!("{m:?}")
        };
        assert_eq!(SavedGame::from_body(&raw), Some(save));
    }

    #[test]
    fn everyone_but_the_host_must_reach_the_time() {
        let mut p = Progress::default();
        assert!(!p.all_reached(&[0, 1, 2], 0, 10));
        p.set(1, 10);
        assert!(!p.all_reached(&[0, 1, 2], 0, 10));
        p.set(2, 12);
        assert!(p.all_reached(&[0, 1, 2], 0, 10));
        assert!(!p.all_reached(&[0, 1, 2], 0, 11));
    }
}
