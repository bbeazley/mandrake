//! The game options word and the gameplay versions the rules test.
//!
//! The options are one `u32`: the game parameters' options word and the film header's
//! `game_options_flags`. Code that holds the whole word tests the `u32` bits below; code that
//! holds the high half alone (`Simulation::game_options_high`) tests that `u16` with [`high`] of
//! the bit, e.g. `game_options_high & high(TFL)`.

/// Teams are allowed (gates the teams items and team requests).
pub const TEAMS: u32 = 0x2;
/// Limited visibility; cleared at the end of a non-cooperative game and in non-solo replays.
pub const LIMITED_VISIBILITY: u32 = 0x4;
/// No automatic score table; a network save's host header clears it.
pub const NO_AUTOMATIC_TABLE: u32 = 0x8;
/// Unit trading is allowed.
pub const UNIT_TRADING: u32 = 0x10;
/// Veteran units are allowed.
pub const VETERANS: u32 = 0x20;
/// The last game's veterans are carried ("Veterans Allowed" in a film's details).
pub const CARRY_VETERANS: u32 = 0x40;
/// A co-operative game, which every solo game is: no Ally button or alliances, no rules line,
/// VICTORY or DEFEAT in the score header.
pub const COOPERATIVE: u32 = 0x80;
/// The camera starts tracking.
pub const CAMERA_TRACKING: u32 = 0x400;
/// Show Narration; with [`COOPERATIVE`] a network game shows its prologue and epilogue.
pub const NARRATION: u32 = 0x800;
/// Alliances are allowed.
pub const ALLIANCES: u32 = 0x2000;
/// Show Enemy on the overhead map; its button is opaque.
pub const SHOW_ENEMY: u32 = 0x4000;

/// "Server is Observer": the host's own record gets no units.
pub const HOST_OBSERVER: u32 = 0x1_0000;
/// A solo mission restarted or carried on, with no prologue.
pub const SOLO_RESTARTED: u32 = 0x2_0000;
/// Deathmatch; every team's score line is drawn as having survivors.
pub const DEATHMATCH: u32 = 0x10_0000;
/// TFL gameplay; body count scores as a share of all the scores.
pub const TFL: u32 = 0x20_0000;
/// Anti-clump; below gameplay [`GAMEPLAY_1_7_1`] the body count share behaves like [`TFL`].
pub const ANTI_CLUMP: u32 = 0x40_0000;
/// Co-operative game whose players keep the team they took in the lobby; without it, with
/// [`TEAMS`], they join a random team.
pub const COOPERATIVE_TEAM: u32 = 0x80_0000;
/// A 1.5.1 game (gameplay [`GAMEPLAY_1_5_1`], version id [`VERSION_ID_1_5_1`]).
pub const GAMEPLAY_1_5_1_GAME: u32 = 0x100_0000;
/// Build numbers decide the gameplay version within a release.
pub const BUILD_VERSIONS: u32 = 0x2000_0000;

/// The high half's bit of `bit`, for a `u16` holding the high half of the options word.
pub const fn high(bit: u32) -> u16 {
    (bit >> 16) as u16
}

// Gameplay versions: the release a game's rules follow, 1000 times the release number plus a step.
// A rule tests the version it changed in; `<= v` corresponds to `< v + 1`.

pub const GAMEPLAY_1_3: i32 = 0x514;
pub const GAMEPLAY_1_5: i32 = 0x5dc;
pub const GAMEPLAY_1_5_1: i32 = 0x5e6;
pub const GAMEPLAY_1_6: i32 = 0x640;
pub const GAMEPLAY_1_7: i32 = 0x6a4;
pub const GAMEPLAY_1_7_1: i32 = 0x6ae;
pub const GAMEPLAY_1_7_2: i32 = 0x6b8;
pub const GAMEPLAY_1_8_2: i32 = 0x71c;
pub const GAMEPLAY_1_8_3: i32 = 0x726;
pub const GAMEPLAY_1_8_4: i32 = 0x731;
pub const GAMEPLAY_1_8_5: i32 = 0x73c;

// Version ids: 2000 plus 10 times the release.

/// 1.5.1, which forces [`GAMEPLAY_1_5_1_GAME`].
pub const VERSION_ID_1_5_1: u16 = 0x867;
/// 1.7, the first that carries veterans to the next game.
pub const VERSION_ID_1_7: u16 = 0x87a;
/// Last version id that gets no pings; pings go to records above it.
pub const VERSION_ID_NO_PINGS: u16 = 0x883;
pub const VERSION_ID_1_8_1: u16 = 0x885;
/// 1.8.2, the first that joins a game in progress.
pub const VERSION_ID_1_8_2: u16 = 0x886;
/// 1.8.5.
pub const VERSION_ID_1_8_5: u16 = 0x889;

/// The bits of TFL 1.5's options word that mean something other than Myth II's.
///
/// TFL shares the low bits [`TEAMS`](super::TEAMS), [`LIMITED_VISIBILITY`](super::LIMITED_VISIBILITY),
/// [`VETERANS`](super::VETERANS), [`CARRY_VETERANS`](super::CARRY_VETERANS),
/// [`COOPERATIVE`](super::COOPERATIVE) ("Use Single Player Levels") and
/// [`CAMERA_TRACKING`](super::CAMERA_TRACKING). It has no alliances: its `0x2000` is not
/// [`ALLIANCES`](super::ALLIANCES). Every TFL 1.5 game sets `0x2000`, `0x4000` and `0x8000`
/// to enable rules that older releases' films do not have.
pub mod tfl {
    /// In a co-op game a computer's monster waits 60 more ticks before an attack at difficulty 0
    /// and 30 at 1, and a person's monster takes a quarter of the damage at 0 and a half at 1
    /// (instead of a half and three quarters); a partial path to a goal nearer than `0x1000` is
    /// dropped; a monster with a target keeps a leg within `0x200` of it; a team's best flags are
    /// the fewest held, places start from 0 and equals share one, game type 0 has no winner by
    /// elimination, and teams tied on score are not ranked by damage.
    pub const FIXES_2000: u32 = 0x2000;
    /// An attack aimed past its range plus `0x400` is aimed at its range along the facing;
    /// a player's damage score excludes damage taken from nobody and done to itself; teams tied
    /// on score are not ranked by losses' cost or (with [`FIXES_2000`] clear) by damage.
    pub const FIXES_8000: u32 = 0x8000;
    /// Set in every TFL 1.5 game alongside [`FIXES_2000`] and [`FIXES_8000`].
    pub const BIT_4000: u32 = 0x4000;
    /// The bits every TFL 1.5 game start sets.
    pub const EVERY_GAME: u32 = FIXES_2000 | BIT_4000 | FIXES_8000;
}
