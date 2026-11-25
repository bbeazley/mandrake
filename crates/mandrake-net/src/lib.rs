//! TCP/IP multiplayer protocol for Myth II and Myth: The Fallen Lords.
//!
//! Contains the message wire format ([`wire`]), player records and join requests
//! ([`player_data`]), game descriptions and new-game data ([`game_description`]), joiner and host
//! protocol state machines ([`topology`], [`lobby`], [`client`], [`server`], [`midgame`],
//! [`netsave`], [`ours`], composed in [`endpoint`]), socket transport ([`transport`]),
//! game-variant differences ([`game_kind`], [`tfl`]), and LAN discovery ([`lan`]).

pub mod client;
pub mod endpoint;
pub mod game_description;
pub mod game_kind;
pub mod gameplay;
pub mod lan;
pub mod lobby;
pub mod midgame;
pub mod netsave;
pub mod ours;
pub mod player_data;
pub mod server;
pub mod tfl;
pub mod topology;
pub mod transport;
pub mod wire;
