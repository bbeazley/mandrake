use mandrake_common::FourCC;

use crate::films::FilmGame;
use crate::report::PostgameReport;
use crate::text::Text;
use mandrake_rec::saved_game::Snapshot;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Screen<'a> {
    MainMenu(&'a MainMenu),
    NewGame(&'a NewGame),
    Loading,
    Briefing(&'a Briefing),
    Playing { overlay: Option<Overlay<'a>> },
    CampaignEnd(&'a Narration),
    Postgame(&'a Postgame),
    Films(&'a Films),
    FilmReport(&'a PostgameReport),
    SavedGames(&'a SavedGames),
    Restoring(&'a Restoring),
    Cutscene(&'a Cutscene),
    Net(crate::net::NetScreen<'a>),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cutscene {
    pub name: String,
    pub movie: Option<crate::tfl::TflMovie>,
}

pub const INTRO_CUTSCENE: &str = "prologue.smk";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Overlay<'a> {
    Hints(&'a Hints),
    Paused(&'a Paused),
    SaveGame(&'a SaveGame),
    SavedGames(&'a SavedGames),
    NewGame(&'a NewGame),
    UnitTrading(&'a UnitTrading),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MainMenu {
    pub new_game: bool,
    pub multiplayer: bool,
    pub saved_games: bool,
    pub films: bool,
    pub tutorial: bool,
    pub intro: bool,
    pub preferences: bool,
    pub music: Vec<MenuTrack>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuTrack {
    pub tag: FourCC,
    pub weight: i16,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewGame {
    pub campaigns: Vec<Text>,
    pub campaign: usize,
    pub levels: Vec<LevelEntry>,
    pub level: Option<FourCC>,
    pub difficulties: Vec<Difficulty>,
    pub difficulty: i16,
    pub description: Text,
    pub preview: Option<FourCC>,
    pub splash: Option<FourCC>,
    pub art_plugins: Vec<String>,
    pub can_start: bool,
    pub tagsets: bool,
    pub gameplay_options: bool,
    pub forced_gameplay: Option<ForcedGameplay>,
    pub tfl_conflict: Option<Text>,
    pub no_menu: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForcedGameplay {
    AntiClump,
    Tfl,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelEntry {
    pub mesh: FourCC,
    pub name: Text,
    pub selectable: bool,
    pub listed: bool,
    pub order: i16,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Difficulty {
    pub name: Text,
    pub description: Text,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Briefing {
    pub art: Option<FourCC>,
    pub art_plugins: Vec<String>,
    pub title: Text,
    pub text: Text,
    pub captions: Vec<Text>,
    pub narration: Option<FourCC>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hints {
    pub objectives: Text,
    pub first_open_objective: i32,
    pub tab_names: Vec<Text>,
    pub general: Vec<Text>,
    pub first_general: i32,
    pub level: Vec<Text>,
    pub show_at_start: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paused {
    pub can_save: bool,
    pub can_open: bool,
    pub can_revert: bool,
    pub can_restart: bool,
    pub replay: bool,
    pub network: Option<NetPause>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitTrading {
    pub rows: Vec<TradeOffer>,
    pub counts: Vec<i16>,
    pub top: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TradeOffer {
    pub name: Text,
    pub adjustable: bool,
    pub minimum: i16,
    pub maximum: i16,
    pub placed: i16,
    pub cost: i16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetPause {
    pub originator: bool,
    pub rehost_items: bool,
    pub can_rehost: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaveGame {
    pub names: Vec<Text>,
    pub name: Text,
    pub snapshot: Option<Snapshot>,
    pub replace: Option<Text>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Narration {
    pub art: Option<FourCC>,
    pub text: Text,
    pub narration: Option<FourCC>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Postgame {
    pub report: PostgameReport,
    pub art: Option<FourCC>,
    pub art_sequence: i16,
    pub ambient: Option<FourCC>,
    pub exits: PostgameExits,
    pub save_film: Option<SaveFilm>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaveFilm {
    pub names: Vec<Text>,
    pub name: Text,
    pub last_name: Text,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PostgameExits {
    pub next: bool,
    pub restart: bool,
    pub last_saved: bool,
    pub skip: bool,
    pub save_film: bool,
    pub quit: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Films {
    pub names: Vec<Text>,
    pub selected: Option<usize>,
    pub details: Option<FilmDetails>,
    pub can_view: bool,
    pub find_plugins: bool,
    pub last_film: bool,
    pub save_last: bool,
    pub delete: bool,
    pub rename: bool,
    pub save_film: Option<SaveFilm>,
    pub delete_film: Option<DeleteFilm>,
    pub rename_film: Option<RenameFilm>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeleteFilm {
    pub name: Text,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenameFilm {
    pub names: Vec<Text>,
    pub name: Text,
    pub exists: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilmDetails {
    pub name: Text,
    pub game: FilmGame,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavedGames {
    pub names: Vec<Text>,
    pub selected: Option<usize>,
    pub snapshot: Option<Snapshot>,
    pub date: Text,
    pub can_open: bool,
    pub delete: bool,
    pub delete_save: Option<DeleteSave>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeleteSave {
    pub name: Text,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Restoring {
    pub name: Text,
    pub fraction: i16,
    pub network: bool,
}
