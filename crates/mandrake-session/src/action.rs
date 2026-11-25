use std::path::PathBuf;

use mandrake_common::FourCC;
use mandrake_rec::header::MonsterInitializer;

use crate::films::FilmFolder;
use crate::net::{GameDescription, NetEvent, NetGameStart, NetRequest, NetSettings};
use crate::saves::{SaveFile, SaveFolder};
use crate::text::Text;

use crate::progress::Progress;
use crate::report::PostgameReport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    NewGame {
        all_levels: bool,
    },
    Tutorial,
    ReplayIntro,
    Quit,
    ChooseCampaign {
        index: usize,
        all_levels: bool,
    },
    ChooseLevel(FourCC),
    ChooseDifficulty(i16),
    StartLevel,
    Cancel,
    BriefingDone,
    OpenPause,
    Resume,
    OpenHints,
    CloseHints,
    SetShowHints(bool),
    SetHintIndex(i16),
    RestartLevel,
    QuitToMenu,
    NextLevel,
    SkipLevel,
    NarrationDone,
    OpenFilms,
    ChooseFilm(usize),
    ViewFilm,
    ReplayLastFilm,
    CloseReport,
    SaveFilm,
    SaveFilmAs(Text),
    DeleteFilm,
    ConfirmDeleteFilm,
    RenameFilm,
    RenameFilmAs(Text),
    CloseNotice,
    OpenSavedGames,
    ChooseSave(usize),
    OpenSave,
    DeleteSave,
    ConfirmDeleteSave,
    SaveGame,
    LoadNetSave,
    ClearNetSave,
    SaveGameAs(Text),
    ReplaceSave,
    LastSavedGame,
    Quicksave,
    Ok,
    ReloadPlugins,
    Multiplayer,
    SetNetPlayer {
        name: Text,
        team: Text,
        icon: u16,
        colors: [u16; 8],
    },
    ChooseOther,
    HostGame {
        name: Text,
        password: Text,
        port: u16,
    },
    JoinByAddress {
        address: Text,
        password: Text,
    },
    JoinLanGame {
        address: Text,
    },
    SetReady(bool),
    OpenOptions,
    SetGameOptions(Box<GameDescription>),
    SetGamePassword(Text),
    StartNetGame,
    LobbyChat(Text),
    LobbyChatTo(crate::net::ChatTo, Text),
    ChatCommand(crate::chat_commands::DotCommand),
    TeamButton(Option<i32>),
    TeamKick(i32),
    TeamLock,
    Kick(i32),
    Lock(bool),
    Leave,
    RestartNetGame,
    SaveReplay {
        name: Text,
    },
    SetTradeCounts(Vec<i16>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    LoadGame(GameStart),
    BeginPlay,
    SetPaused(bool),
    DropGame,
    SaveProgress(Progress),
    ListFilms,
    ListSaves,
    SaveFilm { name: Text },
    DeleteFilm { file_name: Vec<u8> },
    DeleteSave { file_name: Vec<u8> },
    RenameFilm { file_name: Vec<u8>, name: Text },
    TakeSnapshot,
    SaveGame { name: Text },
    Autosave,
    Quicksave,
    PlayCutscene { name: String },
    Exit,
    Net(NetRequest),
    SaveNetPlayer(NetSettings),
    DescribeNetGame(Box<crate::films::FilmGame>),
    NetChatTo(crate::net::ChatTo, Text),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LevelStart {
    NewGame,
    #[default]
    ChangeMap,
    Restart,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GameStart {
    pub start: LevelStart,
    pub mesh: FourCC,
    pub difficulty: i16,
    pub plugins: Vec<String>,
    pub veterans: Vec<MonsterInitializer>,
    pub reset_rankings: bool,
    pub film: Option<PathBuf>,
    pub restore: bool,
    pub net: Option<NetGameStart>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GameEvent {
    Loaded,
    LoadFailed(String),
    MissionAdvanced(u32),
    MapOver {
        report: PostgameReport,
        veterans: Vec<MonsterInitializer>,
    },
    FilmsListed(FilmFolder),
    SavesListed(SaveFolder),
    Restoring {
        ticks: u32,
    },
    FilmSaved(Option<Vec<u8>>),
    FilmRenamed(Vec<u8>),
    Snapshot(mandrake_rec::saved_game::Snapshot),
    GameSaved(Option<SaveFile>),
    IntroFound(bool),
    CutsceneOver,
    Net(NetEvent),
}
