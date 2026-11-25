//! What a front end implements. No wgpu here: a test implements only this. Drawing is a second
//! trait, `mandrake_render::DrawInterface`, because it names wgpu.

use mandrake_common::FourCC;

use crate::action::Action;
use crate::screen::Screen;

/// A front end: shown the screen each frame, it answers with what the player did. The session never
/// waits for one, so a slow or absent front end changes nothing but what's shown.
pub trait FrontEnd {
    fn frame(&mut self, screen: &Screen<'_>, input: &FrameInput<'_>) -> FrameOutput;

    /// Where an open dialog that isn't modal shows over the game, `[left, top, width, height]` in
    /// interface pixels: the game goes on around it and takes the input outside it (untr,
    /// `new_dialog(.., 8)`). `None` when there's none, or the front end draws none.
    fn modeless_rect(&self) -> Option<[i32; 4]> {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrameInput<'a> {
    /// The session has no clock: a front end counts its own 30 Hz ticks from this.
    pub now_ms: u64,
    /// The interface screen, in the same pixels as the events.
    pub size: (u32, u32),
    pub events: &'a [UiEvent],
    /// Narration still playing.
    pub sound_busy: bool,
    /// The live state, as `key_down` reads it (section 4). The session never reads it.
    pub shift_held: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameOutput {
    pub actions: Vec<Action>,
    pub sounds: Vec<SoundCue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEvent {
    PointerMoved {
        x: i32,
        y: i32,
    },
    Button {
        left: bool,
        pressed: bool,
    },
    Key {
        key: UiKey,
        pressed: bool,
        repeat: bool,
        shift: bool,
        alt: bool,
    },
    /// Typed text, for an edit box.
    Text(char),
    Wheel(i32),
}

/// The keys a dialog can use, by what's on the key and not by what it types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiKey {
    Return,
    Escape,
    Space,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// A letter or digit key, lower case, for Alt+letter.
    Char(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCue {
    Music(FourCC),
    Narration(FourCC),
    Ambient(FourCC),
    FadeOut { ticks: u16 },
    Stop,
}
