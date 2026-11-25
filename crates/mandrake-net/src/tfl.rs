use mandrake_rec::tfl::{TflGameParameters, TflNetPlayer, TflNewGame};

use crate::game_description::{
    GameDescription, GameParameters, MAX_PLAYERS, NewGameError, PluginList,
};
use crate::game_kind::TFL_GAMEPLAY_VERSION;
use crate::player_data::PlayerData;

pub const NO_PLUGIN: u32 = u32::MAX;

pub type DataChecksums = [u32; 2];
const NOBODY: u8 = 0xff;

pub fn description(
    parameters: &GameParameters,
    checksums: DataChecksums,
    game_name: &[u8],
    map_name: &[u8],
    password: bool,
) -> GameDescription {
    let mut parameters = parameters.clone();
    parameters.flags = 0;
    parameters.build = 0;
    parameters.version = TFL_GAMEPLAY_VERSION;
    parameters.field_28 = 0;
    parameters.plugin_count = 0;
    GameDescription {
        parameters,
        plugins: PluginList::default(),
        foundation_checksum: !checksums[0],
        field_230: checksums[1],
        flags: if password {
            crate::game_description::FLAG_PASSWORD
        } else {
            0
        },
        player_count: 1,
        game_name: game_name.to_vec(),
        map_name: map_name.to_vec(),
    }
}

pub fn archive_checksum(description: &GameDescription) -> u32 {
    !description.foundation_checksum
}

pub fn plugin_checksum(description: &GameDescription) -> Option<u32> {
    (description.field_230 != NO_PLUGIN).then_some(description.field_230)
}

pub fn film_checksums(description: &GameDescription) -> DataChecksums {
    [archive_checksum(description), description.field_230]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataMismatch {
    Plugin { host: u32, ours: u32 },
    Archive { host: u32, ours: u32 },
}

pub fn check_data(description: &GameDescription, ours: DataChecksums) -> Result<(), DataMismatch> {
    let host = description.field_230;
    if host != ours[1] {
        return Err(DataMismatch::Plugin {
            host,
            ours: ours[1],
        });
    }
    let host = archive_checksum(description);
    if host != ours[0] {
        return Err(DataMismatch::Archive {
            host,
            ours: ours[0],
        });
    }
    Ok(())
}

pub fn parameters(description: &GameDescription) -> TflGameParameters {
    let bytes = description.parameters.to_bytes();
    let mut p = TflGameParameters::from_bytes(
        bytes[..TflGameParameters::SIZE]
            .try_into()
            .expect("0x28 of 0x2c"),
    );
    p.field_20 = [0; 8];
    p
}

pub fn net_player(player: &PlayerData) -> TflNetPlayer {
    TflNetPlayer::from_bytes(&player.to_bytes())
}

pub fn new_game(
    description: &GameDescription,
    players: &[PlayerData],
    local_net_id: i32,
) -> Result<TflNewGame, NewGameError> {
    let players: Vec<TflNetPlayer> = players
        .iter()
        .filter(|p| !is_our_server(p))
        .map(net_player)
        .collect();
    if players.is_empty() || players.len() > MAX_PLAYERS {
        return Err(NewGameError::PlayerCount(players.len()));
    }
    Ok(TflNewGame::network(
        &parameters(description),
        &players,
        local_net_id,
    ))
}

pub fn is_our_server(player: &PlayerData) -> bool {
    player.field_22 == crate::ours::SERVER_MARK
}

pub fn player_order(description: &GameDescription, players: &[PlayerData]) -> Vec<i32> {
    new_game(description, players, -1)
        .map(|g| g.players.iter().map(|p| i32::from(p.net_id)).collect())
        .unwrap_or_default()
}

pub fn player_index(order: &[i32], byte: u8) -> u8 {
    if byte == NOBODY {
        return byte;
    }
    let id = i32::from(byte as i8);
    order
        .iter()
        .position(|&n| n == id)
        .map_or(byte, |i| i as u8)
}
