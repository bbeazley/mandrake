use std::cell::OnceCell;

use crate::controls::list::list_flags;
use crate::core::{Dialog, DialogStack, Hit};
use crate::resources::Resources;
use crate::screens::briefing::{self, BriefingStyle};
use crate::screens::main_menu::{self, Background, Layout};
use crate::screens::postgame;
use crate::screens::{OpenScreen, ScreenFrame};
use crate::{MenuCanvas, controls::prol::PrologueGpuLayer};
use mandrake_common::FourCC;
use mandrake_session::net::NetScreen;
use mandrake_session::tfl::TflGame;
use mandrake_session::{
    Action, Briefing, FrameInput, FrameOutput, MainMenu, NewGame, Postgame, Screen, Session,
    SoundCue, UiEvent,
};
use mandrake_tag::font::Bitmap32;
use mandrake_tag::{Rect, TagArchive};

pub mod endi;
pub mod lobby;
pub mod net;
pub mod notice;
pub mod options;

pub const CANVAS: (u32, u32) = (640, 480);
const TICKS_PER_SECOND: u64 = 30;
const BLACK: u32 = 0xff00_0000;

pub(crate) fn tag_data(archive: &TagArchive, group: &[u8; 4], id: FourCC) -> Option<Vec<u8>> {
    if group == b"stli"
        && id == DELETE_QUESTIONS
        && archive.find_tag_by_id(FourCC::new(*group), id).is_none()
    {
        return delete_questions(archive);
    }
    let e = archive.find_tag_by_id(FourCC::new(*group), id)?;
    let data = archive.get_tag_data(e).ok()?;
    if group == b"stli" && (id == RESTORING_STRINGS || id == TRADE_STRINGS) {
        return Some(mandrake_session::tfl::style_codes(&data));
    }
    if group == b"stli" && id == GAME_OPTION_STRINGS {
        return Some(game_option_strings(&data));
    }
    if group == b"bina" && id == SERD {
        return Some(with_rename_film(&data));
    }
    Some(data)
}

fn with_rename_film(data: &[u8]) -> Vec<u8> {
    use crate::screens::films::{RENAME, REPLAY_LAST, VIEW};
    use mandrake_tag::dialog::{DIALOG_HEADER_SIZE as HEADER, DIALOG_ITEM_SIZE as ITEM};
    let be16 = |o: usize| data.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
    let Some(count) = be16(2).and_then(|c| usize::try_from(c).ok()) else {
        return data.to_vec();
    };
    let tail = HEADER + ITEM * count;
    if data.len() < tail {
        return data.to_vec();
    }
    let at = |id: i16| {
        (0..count)
            .map(|i| HEADER + ITEM * i)
            .find(|&o| be16(o + 6) == Some(id))
    };
    let (Some(view), Some(replay), None) = (at(VIEW), at(REPLAY_LAST), at(RENAME)) else {
        return data.to_vec();
    };
    let step = be16(replay + 0x12).unwrap_or(0) - be16(view + 0x12).unwrap_or(0);
    let mut item = data[replay..replay + ITEM].to_vec();
    item[6..8].copy_from_slice(&RENAME.to_be_bytes());
    for o in [0x12, 0x16] {
        let x = i16::from_be_bytes([item[o], item[o + 1]]) + step;
        item[o..o + 2].copy_from_slice(&x.to_be_bytes());
    }
    let mut names_end = tail;
    for _ in 0..count {
        match data[names_end..].iter().position(|&b| b == 0) {
            Some(n) => names_end += n + 1,
            None => return data.to_vec(),
        }
    }
    let mut out = Vec::with_capacity(data.len() + ITEM + RENAME_FILM.len() + 1);
    out.extend_from_slice(&data[..2]);
    out.extend_from_slice(&((count + 1) as i16).to_be_bytes());
    out.extend_from_slice(&data[4..tail]);
    out.extend_from_slice(&item);
    out.extend_from_slice(&data[tail..names_end]);
    out.extend_from_slice(RENAME_FILM);
    out.push(0);
    out.extend_from_slice(&data[names_end..]);
    out
}

const RENAME_FILM: &[u8] = b"Rename Film";

const DELETE_QUESTIONS: FourCC = FourCC(*b"irp!");

fn delete_questions(archive: &TagArchive) -> Option<Vec<u8>> {
    let e = archive.find_tag_by_id(FourCC(*b"bina"), crate::screens::film_manage::SU01)?;
    let def =
        mandrake_tag::dialog::DialogDefinition::from_bytes(&archive.get_tag_data(e).ok()?).ok()?;
    let text = &def.item(crate::screens::film_manage::QUESTION as u32)?.name;
    let at = text.windows(2).position(|w| w == b"^0")?;
    let line = [&text[..at], b"%s", &text[at + 2..]].concat();
    let mut out = [line.as_slice(); 4].join(&b'\r');
    out.push(0);
    Some(out)
}

const GAME_OPTION_STRINGS: FourCC = FourCC(*b"ingo");

fn game_option_strings(data: &[u8]) -> Vec<u8> {
    const TFL_LINES: usize = 29;
    let lines = mandrake_tag::mesh_links::string_list_lines(data);
    if lines.len() != TFL_LINES {
        return data.to_vec();
    }
    let mut out: Vec<&[u8]> = lines.iter().map(Vec::as_slice).collect();
    out.insert(11, b"Deathmatch");
    for (at, line) in [
        (23, &b"No Alliances"[..]),
        (24, b"Server is Neutral Observer"),
        (25, b"No Overhead Map"),
        (26, b"Deathmatch"),
    ] {
        out.insert(at, line);
    }
    let mut joined = out.join(&b'\r');
    joined.push(0);
    mandrake_session::tfl::style_codes(&joined)
}

const TRADE_STRINGS: FourCC = FourCC(*b"inut");

const RESTORING_STRINGS: FourCC = FourCC(*b"in05");

pub const MAIN: FourCC = FourCC(*b"main");
pub const NEGA: FourCC = FourCC(*b"nega");
pub const SERD: FourCC = FourCC(*b"serd");
pub const PRDI: FourCC = FourCC(*b"prdi");

const fn main_layout(background2: bool) -> Layout {
    Layout {
        dialog: MAIN,
        art: MAIN,
        flags: if background2 { 0x90 } else { 0x15 },
        background: if background2 {
            Background::Dialog(9)
        } else {
            Background::BitmapItem(8)
        },
        version: VERSION_TEXT,
    }
}

pub const VERSION_TEXT: &[u8] = b"|i1.5-mandrake|p";

const NEGA_OK: i16 = 1;
const NEGA_CANCEL: i16 = 2;
const NEGA_LEVELS: i16 = 3;
const NEGA_DIFFICULTIES: i16 = 4;

const EDIT_RING: mandrake_tag::font::RgbColor = [0; 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    Main,
    NewGame,
    Films,
    Prologue,
    Epilogue,
    SavedGames,
    FilmReport,
    Restoring,
    Net,
    NetLobby,
    NetBarrier,
    NetEnd,
    NetNotice(notice::Kind),
}

const NARRATION_STYLE: BriefingStyle = BriefingStyle {
    caption_below_art: true,
};

pub struct MenuInput<'a> {
    pub events: &'a [UiEvent],
    pub now_ms: u64,
    pub shift: bool,
    pub g_held: bool,
    pub music_busy: bool,
}

#[derive(Debug, Default)]
pub struct MenuOutput {
    pub actions: Vec<Action>,
    pub sounds: Vec<SoundCue>,
}

pub struct TflDialogs {
    res: Resources,
    stack: DialogStack,
    shown: Option<Shown>,
    canvas: OnceCell<Bitmap32>,
    static_canvas: Bitmap32,
    revision: u64,
    static_revision: u64,
    static_composed: Option<u64>,
    prologue_scroll: Option<(Rect, f32, i32, bool)>,
    tick: Option<u64>,
    main: Option<Box<dyn OpenScreen>>,
    levels: Vec<FourCC>,
    narration: Option<(Box<dyn OpenScreen>, Briefing)>,
    epilogue: Option<Box<dyn OpenScreen>>,
    shared: Option<Box<dyn OpenScreen>>,
}

impl TflDialogs {
    pub fn new(archive: TagArchive) -> Self {
        let mut res = Resources::from_source(move |group, id| tag_data(&archive, &group.0, id));
        res.set_edit_ring(EDIT_RING);
        Self {
            res,
            stack: DialogStack::tfl(),
            shown: None,
            canvas: OnceCell::new(),
            static_canvas: Bitmap32::new(CANVAS.0 as i32, CANVAS.1 as i32, BLACK),
            revision: 0,
            static_revision: 0,
            static_composed: None,
            prologue_scroll: None,
            tick: None,
            main: None,
            levels: Vec::new(),
            narration: None,
            epilogue: None,
            shared: None,
        }
    }

    pub fn top(&self) -> Option<&Dialog> {
        self.stack.top()
    }

    pub fn canvas(&self) -> &Bitmap32 {
        self.canvas.get_or_init(|| self.render_canvas())
    }

    pub fn menu_canvas(&mut self) -> MenuCanvas<'_> {
        self.compose_static();
        let prologue: Option<PrologueGpuLayer<'_>> = self
            .stack
            .iter()
            .find_map(|d| d.gpu_prologue())
            .map(|(_, p)| p);
        MenuCanvas {
            canvas: &self.static_canvas,
            revision: self.static_revision,
            size: CANVAS,
            prologue,
            scroll: self.prologue_scroll,
        }
    }

    pub fn stack(&self) -> &DialogStack {
        &self.stack
    }

    pub fn reset(&mut self) {
        self.stack.clear();
        self.shown = None;
        self.tick = None;
        self.main = None;
        self.narration = None;
        self.epilogue = None;
        self.shared = None;
        self.canvas.take();
        self.static_composed = None;
    }

    pub fn frame(&mut self, session: &Session, input: &MenuInput<'_>) -> MenuOutput {
        let mut out = MenuOutput::default();
        let Some(tfl) = session.tfl() else {
            return out;
        };
        let screen = session.screen();
        let want = match screen {
            Screen::MainMenu(_) => Shown::Main,
            Screen::NewGame(_) => Shown::NewGame,
            Screen::Films(_) => Shown::Films,
            Screen::Briefing(_) => Shown::Prologue,
            Screen::Postgame(_) => Shown::Epilogue,
            Screen::SavedGames(_) => Shown::SavedGames,
            Screen::Restoring(_) => Shown::Restoring,
            Screen::FilmReport(_) => Shown::FilmReport,
            Screen::Net(NetScreen::Lobby(_) | NetScreen::GameOptions(_)) => Shown::NetLobby,
            Screen::Net(NetScreen::Postgame(..)) => Shown::NetEnd,
            Screen::Net(NetScreen::EndBarrier { .. }) => Shown::NetBarrier,
            Screen::Net(n) if let Some(kind) = notice::kind(&n) => Shown::NetNotice(kind),
            Screen::Net(n) if net::is_tfl_screen(&n) => Shown::Net,
            _ => return out,
        };
        self.stack.place((CANVAS.0 as i32, CANVAS.1 as i32));
        if self.shown != Some(want) {
            self.open(tfl, &screen, &mut out);
            self.shown = Some(want);
            self.revision += 1;
            self.static_revision = self.revision;
            self.canvas.take();
            self.stack.place((CANVAS.0 as i32, CANVAS.1 as i32));
        }
        let mut hits = Vec::new();
        self.stack.set_shift_held(input.shift);
        for event in input.events {
            self.stack.event(event, &mut hits);
        }
        let now_tick = input.now_ms * TICKS_PER_SECOND / 1000;
        let ticks = now_tick.saturating_sub(self.tick.unwrap_or(now_tick)) as u32;
        self.tick = Some(now_tick);
        self.stack.tick(ticks as i32, now_tick, &mut hits);
        match want {
            Shown::Main => {
                self.main_frame(&screen, input, now_tick, ticks as i32, &hits, &mut out);
            }
            Shown::NewGame => self.new_game_frame(&hits, &mut out),
            Shown::Prologue => {
                self.narration_frame(input, now_tick, ticks as i32, &hits, &mut out);
            }
            Shown::Epilogue => {
                if let Screen::Postgame(page) = screen {
                    self.epilogue_frame(page, input, now_tick, ticks as i32, &hits, &mut out);
                }
            }
            Shown::Films
            | Shown::SavedGames
            | Shown::Restoring
            | Shown::FilmReport
            | Shown::Net
            | Shown::NetLobby
            | Shown::NetBarrier
            | Shown::NetEnd
            | Shown::NetNotice(_) => {
                self.shared_frame(&screen, input, now_tick, ticks as i32, &hits, &mut out);
            }
        }
        let (any_dirty, static_dirty) = self.stack.redraw_with_static(&self.res, now_tick);
        if any_dirty {
            self.revision += 1;
            self.canvas.take();
        }
        if static_dirty {
            self.static_revision = self.revision;
        }
        self.prologue_scroll = self.stack.iter().find_map(|d| {
            d.gpu_prologue()
                .map(|(rect, p)| (rect, p.scroll, p.strength, p.is_in_view(rect.height())))
        });
        out
    }

    fn open(&mut self, tfl: &TflGame, screen: &Screen<'_>, out: &mut MenuOutput) {
        self.stack.clear();
        self.main = None;
        self.epilogue = None;
        self.shared = None;
        match *screen {
            Screen::SavedGames(saves) => {
                let menu = MainMenu {
                    saved_games: true,
                    ..behind(tfl)
                };
                if let Some(d) = main_menu::dialog(&layout(tfl), &menu, &mut self.res) {
                    self.stack.push(d);
                }
                self.shared = crate::screens::saved_games::open_over_game(
                    saves,
                    &mut self.res,
                    &mut self.stack,
                );
            }
            Screen::Net(net @ (NetScreen::Lobby(_) | NetScreen::GameOptions(_))) => {
                self.shared = lobby::open(&net, &mut self.res, &mut self.stack);
            }
            Screen::Net(NetScreen::Postgame(..)) => {
                self.shared = endi::open(screen, &mut self.res, &mut self.stack);
            }
            Screen::Net(NetScreen::EndBarrier { .. }) => {
                self.shared = crate::screens::open(screen, &mut self.res, &mut self.stack);
            }
            Screen::Net(n) if notice::is_notice(&n) => {
                if !matches!(n, NetScreen::Notice(_)) {
                    self.shared = notice::open(&n, &mut self.res, &mut self.stack);
                } else {
                    let menu = MainMenu {
                        multiplayer: true,
                        ..behind(tfl)
                    };
                    if let Some(d) = main_menu::dialog(&layout(tfl), &menu, &mut self.res) {
                        self.stack.push(d);
                    }
                    self.shared = notice::open(&n, &mut self.res, &mut self.stack);
                }
            }
            Screen::Net(net) => {
                let menu = MainMenu {
                    multiplayer: true,
                    ..behind(tfl)
                };
                if let Some(d) = main_menu::dialog(&layout(tfl), &menu, &mut self.res) {
                    self.stack.push(d);
                }
                self.shared = net::open(&net, &mut self.res, &mut self.stack);
            }
            Screen::Restoring(restoring) => {
                out.sounds.push(SoundCue::Stop);
                self.shared =
                    crate::screens::restoring::open(restoring, &mut self.res, &mut self.stack);
            }
            Screen::Postgame(page) => {
                out.sounds.push(SoundCue::Stop);
                self.epilogue = postgame::open(page, &mut self.res, &mut self.stack);
            }
            Screen::MainMenu(menu) => {
                self.main =
                    main_menu::open_with(&layout(tfl), menu, &mut self.res, &mut self.stack);
            }
            Screen::NewGame(new_game) => {
                if let Some(d) = main_menu::dialog(&layout(tfl), &behind(tfl), &mut self.res) {
                    self.stack.push(d);
                }
                if let Some(d) = self.new_game(new_game) {
                    self.stack.push(d);
                }
            }
            Screen::Films(films) => {
                out.sounds.push(SoundCue::Stop);
                self.shared = crate::screens::films::open(films, &mut self.res, &mut self.stack);
            }
            Screen::FilmReport(report) => {
                out.sounds.push(SoundCue::Stop);
                self.shared =
                    crate::screens::film_report::open(report, &mut self.res, &mut self.stack);
            }
            Screen::Briefing(narration) => {
                out.sounds.push(SoundCue::Stop);
                let data = Briefing {
                    narration: None,
                    ..narration.clone()
                };
                self.narration =
                    briefing::open_styled(&data, &mut self.res, &mut self.stack, NARRATION_STYLE)
                        .map(|open| (open, data));
                if self.stack.find_mut(PRDI).is_some()
                    && let Some(music) = narration.narration
                {
                    out.sounds.push(SoundCue::Narration(music));
                }
            }
            _ => {}
        }
    }

    fn main_frame(
        &mut self,
        screen: &Screen<'_>,
        input: &MenuInput<'_>,
        now_tick: u64,
        ticks: i32,
        hits: &[Hit],
        out: &mut MenuOutput,
    ) {
        let Some(open) = self.main.as_mut() else {
            return;
        };
        let frame_input = FrameInput {
            now_ms: input.now_ms,
            size: CANVAS,
            events: input.events,
            sound_busy: input.music_busy,
            shift_held: input.shift || input.g_held,
        };
        let mut done = FrameOutput::default();
        open.frame(
            screen,
            &mut ScreenFrame {
                stack: &mut self.stack,
                res: &mut self.res,
                input: &frame_input,
                now_tick,
                ticks,
                hits,
                out: &mut done,
            },
        );
        out.actions.extend(done.actions);
        out.sounds.extend(done.sounds);
    }

    fn shared_frame(
        &mut self,
        screen: &Screen<'_>,
        input: &MenuInput<'_>,
        now_tick: u64,
        ticks: i32,
        hits: &[Hit],
        out: &mut MenuOutput,
    ) {
        let Some(open) = self.shared.as_mut() else {
            return;
        };
        let frame_input = FrameInput {
            now_ms: input.now_ms,
            size: CANVAS,
            events: input.events,
            sound_busy: input.music_busy,
            shift_held: input.shift,
        };
        let mut done = FrameOutput::default();
        open.frame(
            screen,
            &mut ScreenFrame {
                stack: &mut self.stack,
                res: &mut self.res,
                input: &frame_input,
                now_tick,
                ticks,
                hits,
                out: &mut done,
            },
        );
        out.actions.extend(done.actions);
        out.sounds.extend(done.sounds);
    }

    fn new_game(&mut self, new_game: &NewGame) -> Option<Dialog> {
        let def = self.res.dialog(NEGA)?;
        let res = &mut self.res;
        let mut d = Dialog::new(&def, NEGA, 0x18, res);
        d.set_background(res.art(FourCC(*b"inng")), 0);
        d.set_list_unfocusable(NEGA_DIFFICULTIES);
        if let Some(list) = d.control_mut(NEGA_DIFFICULTIES).and_then(|c| c.list_mut()) {
            list.set_flag(list_flags::FOCUSABLE, false);
            list.set_flag(list_flags::FRAME, false);
            for (i, d) in new_game.difficulties.iter().enumerate() {
                list.add_row(&d.name.0, i as i32, true);
            }
            list.select_value(i32::from(new_game.difficulty));
        }
        if let Some(list) = d.control_mut(NEGA_LEVELS).and_then(|c| c.list_mut()) {
            list.set_double_click_item(NEGA_OK);
            list.set_flag(list_flags::FOCUS_RING, false);
            list.set_flag(list_flags::FRAME, false);
            let look = crate::controls::list::EntryLook {
                listed: true,
                order: 0,
            };
            for (i, level) in new_game.levels.iter().enumerate() {
                list.add_unmarked_entry(&level.name.0, i as i32, look);
            }
            if let Some(first) = new_game
                .levels
                .iter()
                .position(|l| Some(l.mesh) == new_game.level)
            {
                list.select(first);
            }
        }
        d.set_focus(NEGA_LEVELS);
        d.set_default_item(NEGA_OK);
        d.set_cancel_item(NEGA_CANCEL);
        self.levels = new_game.levels.iter().map(|l| l.mesh).collect();
        Some(d)
    }

    fn new_game_frame(&mut self, hits: &[Hit], out: &mut MenuOutput) {
        for hit in hits.iter().filter(|h| h.dialog == NEGA) {
            match hit.item {
                NEGA_OK => {
                    let Some(d) = self.stack.find_mut(NEGA) else {
                        continue;
                    };
                    let level = d
                        .control(NEGA_LEVELS)
                        .and_then(|c| c.list())
                        .and_then(|l| l.selected_value())
                        .and_then(|v| self.levels.get(v as usize).copied());
                    let difficulty = d
                        .control(NEGA_DIFFICULTIES)
                        .and_then(|c| c.list())
                        .and_then(|l| l.selected_value())
                        .unwrap_or(2) as i16;
                    match level {
                        Some(level) => out.actions.extend([
                            Action::ChooseLevel(level),
                            Action::ChooseDifficulty(difficulty),
                            Action::StartLevel,
                        ]),
                        None => out.actions.push(Action::Cancel),
                    }
                }
                NEGA_CANCEL => out.actions.push(Action::Cancel),
                _ => {}
            }
        }
    }

    fn narration_frame(
        &mut self,
        input: &MenuInput<'_>,
        now_tick: u64,
        ticks: i32,
        hits: &[Hit],
        out: &mut MenuOutput,
    ) {
        let Some((open, data)) = self.narration.as_mut() else {
            out.actions.push(Action::BriefingDone);
            return;
        };
        let frame_input = FrameInput {
            now_ms: input.now_ms,
            size: CANVAS,
            events: input.events,
            sound_busy: input.music_busy,
            shift_held: input.shift,
        };
        let mut done = FrameOutput::default();
        open.frame(
            &Screen::Briefing(data),
            &mut ScreenFrame {
                stack: &mut self.stack,
                res: &mut self.res,
                input: &frame_input,
                now_tick,
                ticks,
                hits,
                out: &mut done,
            },
        );
        out.sounds.extend(done.sounds);
        if done
            .actions
            .iter()
            .any(|a| matches!(a, mandrake_session::Action::BriefingDone))
        {
            self.narration = None;
            out.actions.push(Action::BriefingDone);
        }
    }

    fn epilogue_frame(
        &mut self,
        page: &Postgame,
        input: &MenuInput<'_>,
        now_tick: u64,
        ticks: i32,
        hits: &[Hit],
        out: &mut MenuOutput,
    ) {
        let Some(open) = self.epilogue.as_mut() else {
            return;
        };
        let frame_input = FrameInput {
            now_ms: input.now_ms,
            size: CANVAS,
            events: input.events,
            sound_busy: input.music_busy,
            shift_held: input.shift,
        };
        let mut done = FrameOutput::default();
        open.frame(
            &Screen::Postgame(page),
            &mut ScreenFrame {
                stack: &mut self.stack,
                res: &mut self.res,
                input: &frame_input,
                now_tick,
                ticks,
                hits,
                out: &mut done,
            },
        );
        for sound in done.sounds {
            if let SoundCue::Ambient(id) | SoundCue::Music(id) = sound {
                out.sounds.push(SoundCue::Music(id));
            }
        }
        for action in done.actions {
            match action {
                Action::NextLevel
                | Action::SkipLevel
                | Action::RestartLevel
                | Action::LastSavedGame
                | Action::QuitToMenu => {
                    self.epilogue = None;
                    out.actions.push(action);
                    break;
                }
                Action::SaveFilm | Action::SaveFilmAs(_) | Action::Cancel => {
                    out.actions.push(action);
                }
                _ => {}
            }
        }
    }

    fn compose_static(&mut self) {
        if self.static_composed == Some(self.static_revision) {
            return;
        }
        self.static_canvas.pixels.fill(BLACK);
        for d in self.stack.iter() {
            crate::core::blit(&mut self.static_canvas, d.bounds(), d.bitmap());
        }
        self.static_composed = Some(self.static_revision);
    }

    fn render_canvas(&self) -> Bitmap32 {
        let mut bitmap = if self.static_composed == Some(self.static_revision) {
            self.static_canvas.clone()
        } else {
            let mut b = Bitmap32::new(CANVAS.0 as i32, CANVAS.1 as i32, BLACK);
            for d in self.stack.iter() {
                crate::core::blit(&mut b, d.bounds(), d.bitmap());
            }
            b
        };
        let now_tick = self.tick.unwrap_or(0);
        for d in self.stack.iter() {
            d.draw_prologues(&self.res, now_tick, &mut bitmap);
        }
        bitmap
    }
}

pub struct TflProgressScreen {
    res: Resources,
    stack: DialogStack,
    canvas: Bitmap32,
    revision: u64,
    composed: Option<u64>,
    last_text: String,
    last_fraction: i16,
}

impl TflProgressScreen {
    pub fn new(archive: std::sync::Arc<TagArchive>) -> Option<Self> {
        let mut res = Resources::from_source(move |group, id| tag_data(&archive, &group.0, id));
        res.set_edit_ring(EDIT_RING);
        let def = res.dialog(crate::screens::restoring::PRDB)?;
        let mut dialog = Dialog::new(&def, crate::screens::restoring::PRDB, 0x58, &mut res);
        dialog.set_background(res.art(FourCC(*b"inbp")), 0);
        dialog.set_visible(crate::screens::restoring::CANCEL, false);
        if let Some((item, bar)) = dialog
            .control_mut(crate::screens::restoring::BAR)
            .and_then(|c| c.progress_mut())
        {
            bar.set_color(item, crate::controls::progress::TFL_COLOR);
            bar.set_fraction(item, 0);
        }
        let mut stack = DialogStack::tfl();
        stack.push(dialog);
        Some(Self {
            res,
            stack,
            canvas: Bitmap32::new(CANVAS.0 as i32, CANVAS.1 as i32, BLACK),
            revision: 1,
            composed: None,
            last_text: String::new(),
            last_fraction: -1,
        })
    }

    pub fn update(&mut self, text: &str, fraction: i16) {
        let frac = fraction.clamp(0, 256);
        if self.last_text == text && self.last_fraction == frac {
            return;
        }
        if let Some(dialog) = self.stack.find_mut(crate::screens::restoring::PRDB) {
            if self.last_text != text {
                dialog.set_static_text(crate::screens::restoring::TEXT, text.as_bytes());
                self.last_text = text.to_string();
            }
            if self.last_fraction != frac {
                if let Some((item, bar)) = dialog
                    .control_mut(crate::screens::restoring::BAR)
                    .and_then(|c| c.progress_mut())
                {
                    bar.set_fraction(item, frac);
                }
                self.last_fraction = frac;
            }
        }
        self.stack.redraw(&self.res, 0);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn menu_canvas(&mut self) -> MenuCanvas<'_> {
        if self.composed != Some(self.revision) {
            self.stack.redraw(&self.res, 0);
            self.canvas.pixels.fill(BLACK);
            for d in self.stack.iter() {
                crate::core::blit(&mut self.canvas, d.bounds(), d.bitmap());
            }
            self.composed = Some(self.revision);
        }
        MenuCanvas {
            canvas: &self.canvas,
            revision: self.revision,
            size: CANVAS,
            prologue: None,
            scroll: None,
        }
    }
}

const fn layout(tfl: &TflGame) -> Layout {
    main_layout(tfl.prefs.menu_background2)
}

fn behind(tfl: &TflGame) -> MainMenu {
    MainMenu {
        new_game: true,
        tutorial: true,
        films: tfl.has_films(),
        intro: tfl.has_intro(),
        ..MainMenu::default()
    }
}

pub const QUDI: FourCC = FourCC(*b"qudi");

pub struct TflQuitDialog {
    res: Resources,
    stack: DialogStack,
    canvas: Bitmap32,
    revision: u64,
    tick: Option<u64>,
    shift_held: bool,
    open: Option<(crate::screens::ScreenKind, Box<dyn OpenScreen>)>,
}

impl TflQuitDialog {
    pub fn open(archive: TagArchive) -> Option<Self> {
        let mut res = Resources::from_source(move |group, id| tag_data(&archive, &group.0, id));
        res.set_edit_ring(EDIT_RING);
        res.dialog(QUDI)?;
        Some(Self {
            res,
            stack: DialogStack::tfl(),
            canvas: Bitmap32::new(CANVAS.0 as i32, CANVAS.1 as i32, 0),
            revision: 1,
            tick: None,
            shift_held: false,
            open: None,
        })
    }

    pub fn set_shift_held(&mut self, shift: bool) {
        self.shift_held = shift;
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn modeless_rect(&self) -> Option<[i32; 4]> {
        use crate::screens::ScreenKind;
        let top = self.stack.top()?;
        (self.open.as_ref().map(|(k, _)| *k) == Some(ScreenKind::UnitTrading)
            && top.id == crate::screens::unit_trading::UNTR)
            .then(|| {
                let b = top.bounds();
                [b.left, b.top, b.width(), b.height()]
            })
    }

    pub fn stack(&self) -> &DialogStack {
        &self.stack
    }

    pub fn frame(&mut self, session: &Session, events: &[UiEvent], now_ms: u64) -> Vec<Action> {
        self.frame_screen(&session.screen(), events, now_ms)
    }

    pub fn frame_screen(
        &mut self,
        screen: &Screen<'_>,
        events: &[UiEvent],
        now_ms: u64,
    ) -> Vec<Action> {
        self.frame_screen_with_shift(screen, events, now_ms, self.shift_held)
    }

    pub fn frame_screen_with_shift(
        &mut self,
        screen: &Screen<'_>,
        events: &[UiEvent],
        now_ms: u64,
        shift_held: bool,
    ) -> Vec<Action> {
        use crate::screens::ScreenKind;
        self.shift_held = shift_held;
        let kind = ScreenKind::of(screen);
        let wanted = kind.over_the_game() && kind != ScreenKind::Playing;
        let mut opened = false;
        if self.open.as_ref().map(|(k, _)| *k) != Some(kind) {
            self.stack.clear();
            self.open = None;
            if wanted {
                self.open = crate::screens::open(screen, &mut self.res, &mut self.stack)
                    .map(|open| (kind, open));
                self.stack.place((CANVAS.0 as i32, CANVAS.1 as i32));
                opened = self.open.is_some();
            }
            if !opened {
                self.compose();
                self.revision += 1;
            }
        }
        let Some((_, open)) = self.open.as_mut() else {
            return Vec::new();
        };
        let mut hits = Vec::new();
        self.stack.set_shift_held(shift_held);
        for event in events {
            self.stack.event(event, &mut hits);
        }
        let now_tick = now_ms * TICKS_PER_SECOND / 1000;
        let ticks = now_tick.saturating_sub(self.tick.unwrap_or(now_tick)) as i32;
        self.tick = Some(now_tick);
        self.stack.tick(ticks, now_tick, &mut hits);
        let input = FrameInput {
            now_ms,
            size: CANVAS,
            events,
            sound_busy: false,
            shift_held,
        };
        let mut out = FrameOutput::default();
        open.frame(
            screen,
            &mut ScreenFrame {
                stack: &mut self.stack,
                res: &mut self.res,
                input: &input,
                now_tick,
                ticks,
                hits: &hits,
                out: &mut out,
            },
        );
        self.stack.place((CANVAS.0 as i32, CANVAS.1 as i32));
        if self.stack.redraw(&self.res, now_tick) || opened || self.revision == 1 {
            self.compose();
            self.revision += 1;
        }
        out.actions
    }

    pub fn canvas(&self) -> &Bitmap32 {
        &self.canvas
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn bounds(&self) -> Rect {
        self.stack
            .iter()
            .map(|d| d.bounds())
            .reduce(|a, b| {
                Rect::new(
                    a.top.min(b.top),
                    a.left.min(b.left),
                    a.bottom.max(b.bottom),
                    a.right.max(b.right),
                )
            })
            .unwrap_or(Rect::new(0, 0, 0, 0))
    }

    fn compose(&mut self) {
        self.canvas.pixels.fill(0);
        for d in self.stack.iter() {
            crate::core::blit(&mut self.canvas, d.bounds(), d.bitmap());
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::screens::main_menu::BUTTONS;

    use mandrake_tag::tfl::menus::TFL_MAIN_MENU;

    #[test]
    fn tfls_buttons_are_myth_iis() {
        let tfl: Vec<(i16, i32, char)> = TFL_MAIN_MENU
            .iter()
            .map(|b| (b.item as i16, i32::from(b.art), b.key as char))
            .collect();
        assert_eq!(tfl, BUTTONS);
    }
}
