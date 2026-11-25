pub mod briefing;
pub mod campaign_end;
pub mod end_barrier;
pub mod film_manage;
pub mod film_report;
pub mod films;
pub mod game_options;
pub mod hints;
pub mod join_failed;
pub mod loading;
pub mod lobby;
pub mod lobby_failed;
pub mod main_menu;
pub mod net_join;
pub mod net_player;
pub mod net_postgame;
pub mod new_game;
pub mod paused;
pub mod plugins_download;
pub mod plugins_missing;
pub mod postgame;
pub mod restoring;
pub mod save_film;
pub mod save_game;
pub mod saved_games;
pub mod unit_trading;

use mandrake_session::{FrameInput, FrameOutput, NetScreen, Overlay, Screen};

use crate::core::{DialogStack, Hit};
use crate::resources::Resources;

pub struct ScreenFrame<'a> {
    pub stack: &'a mut DialogStack,
    pub res: &'a mut Resources,
    pub input: &'a FrameInput<'a>,
    pub now_tick: u64,
    pub ticks: i32,
    pub hits: &'a [Hit],
    pub out: &'a mut FrameOutput,
}

pub trait OpenScreen {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    MainMenu,
    NewGame,
    Loading,
    Briefing,
    Playing,
    Hints,
    Paused,
    SaveGame,
    SavedGamesOverGame,
    NewGameOverGame,
    UnitTrading,
    CampaignEnd,
    Postgame,
    Films,
    FilmReport,
    SavedGames,
    Restoring,
    Cutscene,
    NetPlayer,
    NetJoin,
    Lobby,
    MissingPlugins,
    DownloadingPlugins,
    LobbyFailed,
    GameOptions,
    NetPostgame,
    EndBarrier,
    Net,
    Quit,
}

impl ScreenKind {
    pub const fn of(screen: &Screen<'_>) -> Self {
        match screen {
            Screen::MainMenu(_) => Self::MainMenu,
            Screen::NewGame(_) => Self::NewGame,
            Screen::Loading => Self::Loading,
            Screen::Briefing(_) => Self::Briefing,
            Screen::Playing { overlay: None } => Self::Playing,
            Screen::Playing {
                overlay: Some(Overlay::Hints(_)),
            } => Self::Hints,
            Screen::Playing {
                overlay: Some(Overlay::Paused(_)),
            } => Self::Paused,
            Screen::Playing {
                overlay: Some(Overlay::SaveGame(_)),
            } => Self::SaveGame,
            Screen::Playing {
                overlay: Some(Overlay::SavedGames(_)),
            } => Self::SavedGamesOverGame,
            Screen::Playing {
                overlay: Some(Overlay::NewGame(_)),
            } => Self::NewGameOverGame,
            Screen::Playing {
                overlay: Some(Overlay::UnitTrading(_)),
            } => Self::UnitTrading,
            Screen::CampaignEnd(_) => Self::CampaignEnd,
            Screen::Postgame(_) => Self::Postgame,
            Screen::Films(_) => Self::Films,
            Screen::FilmReport(_) => Self::FilmReport,
            Screen::SavedGames(_) => Self::SavedGames,
            Screen::Restoring(_) => Self::Restoring,
            Screen::Cutscene(_) => Self::Cutscene,
            Screen::Net(NetScreen::Player(_) | NetScreen::Protocol(_)) => Self::NetPlayer,
            Screen::Net(
                NetScreen::HostOrJoin(_) | NetScreen::Joining(_) | NetScreen::JoinFailed(_),
            ) => Self::NetJoin,
            Screen::Net(NetScreen::Lobby(_)) => Self::Lobby,
            Screen::Net(NetScreen::MissingPlugins(..)) => Self::MissingPlugins,
            Screen::Net(NetScreen::Downloading(..)) => Self::DownloadingPlugins,
            Screen::Net(NetScreen::LobbyFailed(..)) => Self::LobbyFailed,
            Screen::Net(NetScreen::GameOptions(_)) => Self::GameOptions,
            Screen::Net(NetScreen::Postgame(..)) => Self::NetPostgame,
            Screen::Net(NetScreen::EndBarrier { .. }) => Self::EndBarrier,
            Screen::Net(_) => Self::Net,
            Screen::Quit => Self::Quit,
        }
    }

    pub const fn over_the_game(self) -> bool {
        matches!(
            self,
            Self::Playing
                | Self::Hints
                | Self::Paused
                | Self::SaveGame
                | Self::SavedGamesOverGame
                | Self::NewGameOverGame
                | Self::UnitTrading
        )
    }
}

pub fn open(
    screen: &Screen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    match screen {
        Screen::MainMenu(data) => main_menu::open(data, res, stack),
        Screen::NewGame(data) => new_game::open(data, res, stack),
        Screen::Loading => loading::open(res, stack),
        Screen::Briefing(data) => briefing::open(data, res, stack),
        Screen::Playing {
            overlay: Some(Overlay::Hints(data)),
        } => hints::open(data, res, stack),
        Screen::Playing {
            overlay: Some(Overlay::Paused(data)),
        } => paused::open(data, res, stack),
        Screen::Playing {
            overlay: Some(Overlay::SaveGame(data)),
        } => save_game::open(data, res, stack),
        Screen::Playing {
            overlay: Some(Overlay::SavedGames(data)),
        } => saved_games::open_over_game(data, res, stack),
        Screen::Playing {
            overlay: Some(Overlay::NewGame(data)),
        } => new_game::open_over_game(data, res, stack),
        Screen::Playing {
            overlay: Some(Overlay::UnitTrading(data)),
        } => unit_trading::open(data, res, stack),
        Screen::CampaignEnd(data) => campaign_end::open(data, res, stack),
        Screen::Postgame(data) => postgame::open(data, res, stack),
        Screen::Films(data) => films::open(data, res, stack),
        Screen::FilmReport(data) => film_report::open(data, res, stack),
        Screen::SavedGames(data) => saved_games::open(data, res, stack),
        Screen::Restoring(data) => restoring::open(data, res, stack),
        Screen::Net(net @ (NetScreen::Player(_) | NetScreen::Protocol(_))) => {
            net_player::open(net, res, stack)
        }
        Screen::Net(
            net @ (NetScreen::HostOrJoin(_) | NetScreen::Joining(_) | NetScreen::JoinFailed(_)),
        ) => net_join::open(net, res, stack),
        Screen::Net(net @ NetScreen::Lobby(_)) => lobby::open(net, res, stack),
        Screen::Net(net @ NetScreen::MissingPlugins(..)) => plugins_missing::open(net, res, stack),
        Screen::Net(net @ NetScreen::Downloading(..)) => plugins_download::open(net, res, stack),
        Screen::Net(net @ NetScreen::LobbyFailed(..)) => lobby_failed::open(net, res, stack),
        Screen::Net(net @ NetScreen::GameOptions(_)) => game_options::open(net, res, stack),
        Screen::Net(net @ NetScreen::Postgame(..)) => net_postgame::open(net, res, stack),
        Screen::Net(NetScreen::EndBarrier { .. }) => end_barrier::open(res, stack),
        Screen::Playing { overlay: None } | Screen::Cutscene(_) | Screen::Net(_) | Screen::Quit => {
            None
        }
    }
}
