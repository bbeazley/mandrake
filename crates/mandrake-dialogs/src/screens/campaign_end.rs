use mandrake_common::FourCC;
use mandrake_session::{Action, Narration, Screen, SoundCue, UiEvent, UiKey};

use super::{OpenScreen, ScreenFrame};
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const INGE: FourCC = FourCC(*b"inge");
pub const PROLOGUE_ITEM: i16 = 1;
pub const TEXT_FONT: usize = 4;
pub const TEXT_COLOR: i32 = 1;
pub const SOUND_FADE_TICKS: u16 = 15;
pub const SCROLL_STEP_TICKS: i32 = 4;

pub fn open(
    data: &Narration,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    match Running::open(data, res, stack) {
        Some(running) => Some(Box::new(running)),
        None => {
            tracing::warn!("campaign_end: its dialog or art isn't there");
            Some(Box::new(Nothing))
        }
    }
}

struct Nothing;

impl OpenScreen for Nothing {
    fn frame(&mut self, _screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        frame.out.actions.push(Action::NarrationDone);
    }
}

struct Running {
    narration: Option<FourCC>,
    started: bool,
    clock_ms: Option<u64>,
    credit: i32,
    over: bool,
    done_sent: bool,
}

impl Running {
    fn open(data: &Narration, res: &mut Resources, stack: &mut DialogStack) -> Option<Self> {
        let art_id = data.art.unwrap_or(INGE);
        let art = res.art(art_id)?;
        let def = res.dialog(INGE)?;
        let mut dialog = Dialog::new(&def, INGE, dialog_flags::CENTERED, res);
        dialog.set_background(Some(art.clone()), 0);

        let color = if art.coll.sequence(1).is_some() {
            mandrake_tag::hud::rgb_color_from_sequence(&art.raw, &art.coll, 1, TEXT_COLOR)
        } else {
            [0xffff; 3]
        };
        let family = res.font(TEXT_FONT).cloned();
        if let (Some(family), Some((item, text))) = (
            family,
            dialog
                .control_mut(PROLOGUE_ITEM)
                .and_then(|c| c.prologue_mut()),
        ) {
            text.set_data(item, color, &family, &data.text.0, res);
        }
        stack.push(dialog);
        Some(Self {
            narration: data.narration,
            started: false,
            clock_ms: None,
            credit: 0,
            over: false,
            done_sent: false,
        })
    }
}

fn is_skip(event: &UiEvent) -> bool {
    matches!(
        event,
        UiEvent::Key {
            key: UiKey::Space | UiKey::Return | UiKey::Escape,
            pressed: true,
            ..
        } | UiEvent::Button {
            left: true,
            pressed: true,
        }
    )
}

impl OpenScreen for Running {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        if !matches!(screen, Screen::CampaignEnd(_)) {
            return;
        }
        let now_ms = frame.input.now_ms;
        let sound_busy = frame.input.sound_busy;
        let double_click = frame.stack.take_double_click();
        let Some(dialog) = frame.stack.find_mut(INGE) else {
            return;
        };

        if !self.started {
            self.started = true;
            self.clock_ms = Some(now_ms);
            if let Some(narration) = self.narration {
                frame.out.sounds.push(SoundCue::Narration(narration));
            }
        }

        let skip = frame.input.events.iter().any(is_skip) || double_click;
        if skip {
            self.over = true;
        } else {
            self.credit += frame.ticks;
            while self.credit >= SCROLL_STEP_TICKS {
                self.credit -= SCROLL_STEP_TICKS;
                if let Some((item, text)) = dialog
                    .control_mut(PROLOGUE_ITEM)
                    .and_then(|c| c.prologue_mut())
                {
                    text.update(item);
                }
            }
            if let Some((_item, text)) = dialog
                .control_mut(PROLOGUE_ITEM)
                .and_then(|c| c.prologue_mut())
                && text.done()
                && !sound_busy
            {
                self.over = true;
            }
        }

        if !self.over
            && let Some((item, text)) = dialog
                .control_mut(PROLOGUE_ITEM)
                .and_then(|c| c.prologue_mut())
        {
            let elapsed_ms = now_ms.saturating_sub(self.clock_ms.unwrap_or(now_ms));
            let initial = (item.rect.top - item.rect.bottom) as f32;
            text.set_scroll_f32(initial + 1.0 + (elapsed_ms as f32) * (7.5 / 1000.0));
        }

        if self.over && !self.done_sent {
            self.done_sent = true;
            frame.out.sounds.push(SoundCue::FadeOut {
                ticks: SOUND_FADE_TICKS,
            });
            frame.out.actions.push(Action::NarrationDone);
        }
    }
}
