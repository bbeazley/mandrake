pub mod action;
pub mod catalog;
pub mod chat_commands;
pub mod films;
pub mod front_end;
pub mod net;
pub mod net_game;
pub mod progress;
pub mod report;
pub mod rules;
pub mod saves;
pub mod screen;
pub mod session;
#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;
pub mod text;
pub mod tfl;

pub use action::{Action, GameEvent, GameStart, LevelStart, Request};
pub use catalog::Catalog;
pub use films::{FilmFile, FilmFolder, FilmGame};
pub use front_end::{FrameInput, FrameOutput, FrontEnd, SoundCue, UiEvent, UiKey};
pub use mandrake_rec::saved_game::Snapshot;
pub use net::{NetEvent, NetRequest, NetScreen};
pub use progress::Progress;
pub use report::{
    EpiloguePage, FilmPages, Graph, GraphLine, PostgameReport, RankRow, ReportSection, SectionKind,
};
pub use saves::{SaveFile, SaveFolder};
pub use screen::{
    Briefing, Cutscene, DeleteFilm, DeleteSave, Difficulty, FilmDetails, Films, Hints,
    INTRO_CUTSCENE, LevelEntry, MainMenu, MenuTrack, Narration, NewGame, Overlay, Paused, Postgame,
    PostgameExits, RenameFilm, Restoring, SaveFilm, SaveGame, SavedGames, Screen,
};
pub use session::Session;
pub use text::Text;
