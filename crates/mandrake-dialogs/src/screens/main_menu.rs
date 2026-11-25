use mandrake_common::FourCC;
use mandrake_session::{Action, MainMenu, Screen, SoundCue};
use mandrake_tag::Rect;

use super::{OpenScreen, ScreenFrame};
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const IMM6: FourCC = FourCC(*b"imm6");
const FLAGS: u16 = dialog_flags::CENTERED | dialog_flags::NO_BACKGROUND | 0x01;
const BACKGROUND_SEQUENCE: i32 = 9;
const BITMAP_ITEM: i16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Dialog(i32),
    BitmapItem(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub dialog: FourCC,
    pub art: FourCC,
    pub flags: u16,
    pub background: Background,
    pub version: &'static [u8],
}

pub const MYTH2: Layout = Layout {
    dialog: IMM6,
    art: IMM6,
    flags: FLAGS,
    background: Background::Dialog(BACKGROUND_SEQUENCE),
    version: VERSION_TEXT,
};
pub const VERSION_ITEM: i16 = 1000;
pub const REFERENCE_VERSION_RECT: Rect = Rect::new(1, 1, 18, 480);
pub const VERSION_RECT: Rect = REFERENCE_VERSION_RECT;
const VERSION_FONT: usize = 2;
pub const VERSION_TEXT: &[u8] = b"|i1.8.5-mandrake|p";

pub const fn version_rect(width: i32, height: i32) -> Rect {
    if height >= 600 {
        Rect::new(height - 18, width - 300, height - 2, width - 24)
    } else {
        Rect::new(height - 15, width - 250, height - 2, width - 20)
    }
}

pub const NEW_GAME: i16 = 2;
pub const MULTIPLAYER: i16 = 3;
pub const SAVED_GAMES: i16 = 4;
pub const FILMS: i16 = 5;
pub const TUTORIAL: i16 = 6;
pub const INTRO: i16 = 7;
pub const PREFERENCES: i16 = 8;
pub const QUIT: i16 = 9;

pub const BUTTONS: [(i16, i32, char); 8] = [
    (SAVED_GAMES, 1, 'C'),
    (MULTIPLAYER, 2, 'G'),
    (NEW_GAME, 0, 'N'),
    (FILMS, 3, 'F'),
    (TUTORIAL, 4, 'T'),
    (INTRO, 5, 'I'),
    (PREFERENCES, 6, 'P'),
    (QUIT, 7, 'Q'),
];

const fn enabled(item: i16, data: &MainMenu) -> bool {
    match item {
        NEW_GAME => data.new_game,
        MULTIPLAYER => data.multiplayer,
        SAVED_GAMES => data.saved_games,
        FILMS => data.films,
        TUTORIAL => data.tutorial,
        INTRO => data.intro,
        PREFERENCES => data.preferences,
        _ => true,
    }
}

fn set_enabled(dialog: &mut Dialog, data: &MainMenu) {
    for (item, _, _) in BUTTONS {
        dialog.set_enabled(item, enabled(item, data));
    }
}

const fn action(item: i16, shift_held: bool) -> Option<Action> {
    match item {
        NEW_GAME => Some(Action::NewGame {
            all_levels: shift_held,
        }),
        TUTORIAL => Some(Action::Tutorial),
        FILMS => Some(Action::OpenFilms),
        SAVED_GAMES => Some(Action::OpenSavedGames),
        MULTIPLAYER => Some(Action::Multiplayer),
        INTRO => Some(Action::ReplayIntro),
        QUIT => Some(Action::Quit),
        _ => None,
    }
}

pub fn open(
    data: &MainMenu,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    open_with(&MYTH2, data, res, stack)
}

pub fn open_with(
    layout: &Layout,
    data: &MainMenu,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    stack.push(dialog(layout, data, res)?);
    Some(Box::new(Menu { id: layout.dialog }))
}

pub fn push_behind(res: &mut Resources, stack: &mut DialogStack) -> Option<()> {
    let data = MainMenu {
        new_game: true,
        tutorial: true,
        ..MainMenu::default()
    };
    stack.push(dialog(&MYTH2, &data, res)?);
    Some(())
}

pub fn dialog(layout: &Layout, data: &MainMenu, res: &mut Resources) -> Option<Dialog> {
    let def = res.dialog(layout.dialog)?;
    let art = res.art(layout.art);
    let mut dialog = Dialog::new(&def, layout.dialog, layout.flags, res);
    dialog.set_cancel_item(QUIT);
    dialog.add_control(
        FourCC(*b"stat"),
        VERSION_ITEM,
        version_rect(def.width(), def.height()),
        layout.version,
        res,
    );
    if let Some((item, text)) = dialog
        .control_mut(VERSION_ITEM)
        .and_then(|c| c.static_text_mut())
    {
        text.set_appearance(
            item,
            res.font(VERSION_FONT).cloned(),
            mandrake_tag::font::STYLE_PLAIN,
            None,
            Some(mandrake_tag::font::text_flags::RIGHT),
        );
    }
    match layout.background {
        Background::Dialog(sequence) => {
            dialog.set_background(art.clone(), sequence);
            dialog.set_visible(BITMAP_ITEM, false);
        }
        Background::BitmapItem(sequence) => {
            if let Some((item, bitmap)) =
                dialog.control_mut(BITMAP_ITEM).and_then(|c| c.bitmap_mut())
            {
                bitmap.set_art(item, art.clone(), sequence, 0);
            }
        }
    }
    for (id, sequence, key) in BUTTONS {
        if let Some((item, button)) = dialog.control_mut(id).and_then(|c| c.button_mut()) {
            button.set_shape(item, art.clone(), sequence);
        }
        dialog.set_keyboard_equivalent(id, key, true);
    }
    set_enabled(&mut dialog, data);
    Some(dialog)
}

struct Menu {
    id: FourCC,
}

impl OpenScreen for Menu {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::MainMenu(data) = screen else {
            return;
        };
        if let Some(dialog) = frame.stack.find_mut(self.id) {
            set_enabled(dialog, data);
        }
        if !frame.input.sound_busy
            && let Some(music) = frame.res.next_menu_track(&data.music)
        {
            frame.out.sounds.push(SoundCue::Music(music));
        }
        for hit in frame.hits {
            if !BUTTONS.iter().any(|(item, _, _)| *item == hit.item) {
                continue;
            }
            match action(hit.item, frame.input.shift_held) {
                Some(action) => frame.out.actions.push(action),
                None => tracing::info!(item = hit.item, "main menu: not built yet"),
            }
        }
    }
}
