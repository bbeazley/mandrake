//! The way from a briefing or a menu to a level (ours: design, section 4a). There's no dialog: the
//! last picture stays on the canvas ([`crate::EngineDialogs`]) with the shell's fade over it, then black
//! while the level loads, then the level. All this has is the sound: the music, or anything else left,
//! goes with the picture.

use mandrake_session::{Screen, SoundCue};

use super::{OpenScreen, ScreenFrame};
use crate::core::DialogStack;
use crate::resources::Resources;

/// `FADE_OUT_SECONDS` in the client, in ticks.
pub const SOUND_FADE_TICKS: u16 = 60;

pub fn open(_res: &mut Resources, _stack: &mut DialogStack) -> Option<Box<dyn OpenScreen>> {
    Some(Box::new(Loading { faded: false }))
}

struct Loading {
    faded: bool,
}

impl OpenScreen for Loading {
    fn frame(&mut self, _screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        if !self.faded {
            self.faded = true;
            frame.out.sounds.push(SoundCue::FadeOut {
                ticks: SOUND_FADE_TICKS,
            });
        }
    }
}
