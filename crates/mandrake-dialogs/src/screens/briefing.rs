use std::sync::Arc;

use mandrake_common::FourCC;
use mandrake_session::{Action, Briefing, Screen, SoundCue, UiEvent, UiKey};
use mandrake_tag::font::{self, RgbColor};
use mandrake_view::interface_bitmap::InterfaceArt;

use super::{OpenScreen, ScreenFrame};
use crate::controls::bitmap::CROSSFADE;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::fade::{BRIEFING_CROSSFADE, BRIEFING_FADE_IN};
use crate::resources::Resources;

pub const PRDI: FourCC = FourCC(*b"prdi");

pub const BLACK: i16 = 1;
const HIDDEN_BITMAP: i16 = 2;
pub const TEXT: i16 = 3;
pub const TITLE: i16 = 4;
pub const IMAGE: i16 = 5;
pub const MAP_OVER: i16 = 6;

const DIMMED_MAP_SEQUENCE: i32 = 0;
const IMAGES_SEQUENCE: i32 = 2;
const COLORS_SEQUENCE: i32 = 3;
const MAP_SEQUENCE: i32 = 4;
const TITLE_COLOR: i32 = 0;
const TEXT_COLOR: i32 = 1;

const TITLE_FONT: usize = 3;
const TEXT_FONT: usize = 4;
const CAPTION_FONT: usize = 5;
const TITLE_FLAGS: u32 = 0x20;
const CAPTION_FLAGS: u32 = 0x10;
const IMAGE_FLAGS: u32 = CROSSFADE | 0x14;
const MAX_IMAGES: usize = 19;

pub const FADE_IN_TICKS: i32 = BRIEFING_FADE_IN.steps();
pub const HOLD_TICKS: i32 = 0x97;
pub const CROSSFADE_TICKS: i32 = BRIEFING_CROSSFADE.steps();
pub const SCROLL_STEP_TICKS: i32 = 3;
pub const SOUND_FADE_TICKS: u16 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    FadeIn(i32),
    Hold(i32),
    Crossfade(i32),
    Scroll,
    Over,
}

impl Phase {
    pub const fn can_skip(self) -> bool {
        matches!(self, Self::Hold(_) | Self::Scroll)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    FadeIn(u32),
    FadeInOver,
    CrossfadeBegins,
    Crossfade(u32),
    CrossfadeOver,
    Scroll,
}

const fn fade_value(k: i32) -> u32 {
    BRIEFING_FADE_IN.value(k)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeline {
    pub phase: Phase,
    credit: i32,
    wait: i32,
}

impl Timeline {
    pub const fn new() -> Self {
        Self {
            phase: Phase::FadeIn(0),
            credit: 0,
            wait: 0,
        }
    }

    pub fn advance(&mut self, ticks: i32, mut on_step: impl FnMut(Step) -> bool) {
        self.credit += ticks;
        while self.phase != Phase::Over && self.credit >= self.wait {
            self.credit -= self.wait;
            self.wait = 0;
            match self.phase {
                Phase::FadeIn(k) if k < FADE_IN_TICKS => {
                    on_step(Step::FadeIn(fade_value(k)));
                    self.phase = Phase::FadeIn(k + 1);
                    self.wait = 1;
                }
                Phase::FadeIn(_) => {
                    on_step(Step::FadeInOver);
                    self.begin_loop(Phase::Hold(0));
                }
                Phase::Hold(k) if k < HOLD_TICKS => {
                    self.phase = Phase::Hold(k + 1);
                    self.wait = 1;
                }
                Phase::Hold(_) => {
                    on_step(Step::CrossfadeBegins);
                    self.begin_loop(Phase::Crossfade(0));
                }
                Phase::Crossfade(k) if k < CROSSFADE_TICKS => {
                    on_step(Step::Crossfade(fade_value(k)));
                    self.phase = Phase::Crossfade(k + 1);
                    self.wait = 1;
                }
                Phase::Crossfade(_) => {
                    on_step(Step::CrossfadeOver);
                    self.begin_loop(Phase::Scroll);
                }
                Phase::Scroll => {
                    if on_step(Step::Scroll) {
                        self.phase = Phase::Over;
                    } else {
                        self.wait = SCROLL_STEP_TICKS;
                    }
                }
                Phase::Over => {}
            }
        }
    }

    const fn begin_loop(&mut self, phase: Phase) {
        self.phase = phase;
        self.credit = 0;
    }

    pub const fn skip(&mut self) -> bool {
        if self.phase.can_skip() {
            self.phase = Phase::Over;
            true
        } else {
            false
        }
    }
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new()
    }
}

pub fn image_times(name: &str) -> Vec<i32> {
    let mut total = 0;
    name.split(',')
        .filter(|part| !part.is_empty())
        .take(MAX_IMAGES)
        .map(|part| {
            total += strtol(part) * 1000;
            total
        })
        .collect()
}

fn strtol(text: &str) -> i32 {
    let text = text.trim_start();
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    sign * digits[..end].parse::<i32>().unwrap_or(0)
}

pub fn image_at(times: &[i32], elapsed_ms: i32) -> usize {
    let mut index = 0;
    while index + 1 < times.len() && elapsed_ms > times[index] {
        index += 1;
    }
    index
}

fn is_skip(event: &UiEvent) -> bool {
    matches!(
        event,
        UiEvent::Key {
            key: UiKey::Space | UiKey::Return | UiKey::Escape,
            pressed: true,
            ..
        }
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BriefingStyle {
    pub caption_below_art: bool,
}

pub fn open(
    data: &Briefing,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    open_styled(data, res, stack, BriefingStyle::default())
}

pub fn open_styled(
    data: &Briefing,
    res: &mut Resources,
    stack: &mut DialogStack,
    style: BriefingStyle,
) -> Option<Box<dyn OpenScreen>> {
    match Running::open(data, res, stack, style) {
        Some(running) => Some(Box::new(running)),
        None => {
            tracing::warn!("briefing: its dialog or art isn't there");
            Some(Box::new(Nothing))
        }
    }
}

struct Nothing;

impl OpenScreen for Nothing {
    fn frame(&mut self, _screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        frame.out.actions.push(Action::BriefingDone);
    }
}

struct Running {
    timeline: Timeline,
    art: Arc<InterfaceArt>,
    narration: Option<FourCC>,
    started: bool,
    times: Vec<i32>,
    clock_ms: Option<u64>,
    image: Option<usize>,
    captions: Vec<Vec<u8>>,
    style: BriefingStyle,
    done_sent: bool,
}

fn color_of(art: &InterfaceArt, entry: i32) -> RgbColor {
    mandrake_tag::hud::rgb_color_from_sequence(&art.raw, &art.coll, COLORS_SEQUENCE, entry)
}

impl Running {
    fn open(
        data: &Briefing,
        res: &mut Resources,
        stack: &mut DialogStack,
        style: BriefingStyle,
    ) -> Option<Self> {
        let art = res.art_of_plugins(&data.art_plugins, data.art?)?;
        art.coll.sequence(COLORS_SEQUENCE as usize)?;
        let def = res.dialog(PRDI)?;
        let mut dialog = Dialog::new(&def, PRDI, dialog_flags::CENTERED, res);
        dialog.set_visible(HIDDEN_BITMAP, false);
        if let Some((item, map)) = dialog.control_mut(MAP_OVER).and_then(|c| c.bitmap_mut()) {
            map.set_drawing_flags(item, 0, 0);
        }
        dialog.set_enabled(MAP_OVER, false);
        if let Some((item, black)) = dialog.control_mut(BLACK).and_then(|c| c.bitmap_mut()) {
            black.set_drawing_flags(item, 0, 0);
            black.set_color(item, Some([0; 3]));
        }
        let title_color = color_of(&art, TITLE_COLOR);
        if let Some((item, title)) = dialog.control_mut(TITLE).and_then(|c| c.static_text_mut()) {
            title.set_text(item, &data.title.0);
            title.set_appearance(
                item,
                res.font(TITLE_FONT).cloned(),
                font::STYLE_PLAIN,
                Some(title_color),
                Some(TITLE_FLAGS),
            );
        }
        if let (Some(family), Some((item, text))) = (
            res.font(TEXT_FONT).cloned(),
            dialog.control_mut(TEXT).and_then(|c| c.prologue_mut()),
        ) {
            text.set_data(item, color_of(&art, TEXT_COLOR), &family, &data.text.0, res);
        }
        dialog.set_background(Some(art.clone()), MAP_SEQUENCE);
        for id in [MAP_OVER, TEXT, TITLE, IMAGE] {
            dialog.set_visible(id, false);
        }
        stack.push(dialog);
        let times = art
            .coll
            .sequence(IMAGES_SEQUENCE as usize)
            .map(|s| image_times(&s.name))
            .unwrap_or_default();
        Some(Self {
            timeline: Timeline::new(),
            art,
            narration: data.narration,
            started: false,
            times,
            clock_ms: None,
            image: None,
            captions: data.captions.iter().map(|c| c.0.clone()).collect(),
            style,
            done_sent: false,
        })
    }

    fn show_image(&mut self, dialog: &mut Dialog, res: &Resources, now_ms: u64, now_tick: u64) {
        if self.times.is_empty() {
            return;
        }
        let elapsed = now_ms.saturating_sub(self.clock_ms.unwrap_or(now_ms)) as i32;
        let index = image_at(&self.times, elapsed);
        if self.image == Some(index) {
            return;
        }
        self.image = Some(index);
        let Some((item, image)) = dialog.control_mut(IMAGE).and_then(|c| c.bitmap_mut()) else {
            return;
        };
        image.set_art(item, Some(self.art.clone()), IMAGES_SEQUENCE, index as i32);
        image.set_drawing_flags(item, IMAGE_FLAGS, now_tick);
        if self.captions.is_empty() {
            return;
        }
        let row = if self.captions.len() == 1 { 0 } else { index };
        if let (Some(text), Some(family)) = (self.captions.get(row), res.font(CAPTION_FONT)) {
            image.set_caption(
                family.clone(),
                color_of(&self.art, TITLE_COLOR),
                text,
                CAPTION_FLAGS,
            );
            image.set_caption_below_art(self.style.caption_below_art);
        }
    }

    fn step(
        &mut self,
        step: Step,
        dialog: &mut Dialog,
        res: &Resources,
        sound_busy: bool,
        now_ms: u64,
        now_tick: u64,
    ) -> bool {
        match step {
            Step::FadeIn(u) => set_bitmap_transparency(dialog, BLACK, u << 8),
            Step::FadeInOver => {
                if let Some((item, black)) = dialog.control_mut(BLACK).and_then(|c| c.bitmap_mut())
                {
                    black.set_color(item, None);
                }
                dialog.set_visible(BLACK, false);
            }
            Step::CrossfadeBegins => {
                dialog.set_background(Some(self.art.clone()), DIMMED_MAP_SEQUENCE);
                if let Some((item, map)) = dialog.control_mut(MAP_OVER).and_then(|c| c.bitmap_mut())
                {
                    map.set_art(item, Some(self.art.clone()), MAP_SEQUENCE, 0);
                }
                for id in [MAP_OVER, TEXT, TITLE, IMAGE] {
                    dialog.set_visible(id, true);
                }
            }
            Step::Crossfade(u) => {
                let inverse = (!u & 0xff) << 8;
                set_text_transparency(dialog, inverse);
                set_bitmap_transparency(dialog, IMAGE, inverse);
                set_bitmap_transparency(dialog, MAP_OVER, u << 8);
            }
            Step::CrossfadeOver => {
                if let Some((item, map)) = dialog.control_mut(MAP_OVER).and_then(|c| c.bitmap_mut())
                {
                    map.set_art(item, None, -1, 0);
                }
                set_text_transparency(dialog, 0);
                dialog.set_visible(MAP_OVER, false);
                set_bitmap_transparency(dialog, IMAGE, 0);
                self.clock_ms = Some(now_ms);
            }
            Step::Scroll => {
                self.show_image(dialog, res, now_ms, now_tick);
                if let Some((item, text)) = dialog.control_mut(TEXT).and_then(|c| c.prologue_mut())
                {
                    text.update(item);
                    return text.done() && !sound_busy;
                }
                return !sound_busy;
            }
        }
        false
    }
}

fn set_bitmap_transparency(dialog: &mut Dialog, id: i16, transparency: u32) {
    if let Some((item, bitmap)) = dialog.control_mut(id).and_then(|c| c.bitmap_mut()) {
        bitmap.set_transparency(item, transparency);
    }
}

fn set_text_transparency(dialog: &mut Dialog, transparency: u32) {
    if let Some((item, text)) = dialog.control_mut(TEXT).and_then(|c| c.prologue_mut()) {
        text.set_transparency(item, transparency);
    }
}

impl OpenScreen for Running {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        if !matches!(screen, Screen::Briefing(_)) {
            return;
        }
        let now_ms = frame.input.now_ms;
        let now_tick = frame.now_tick;
        let sound_busy = frame.input.sound_busy;
        let double_click = frame.stack.take_double_click();
        let Some(dialog) = frame.stack.find_mut(PRDI) else {
            return;
        };
        if !self.started {
            self.started = true;
            self.clock_ms = Some(now_ms);
            self.show_image(dialog, frame.res, now_ms, now_tick);
            if let Some(narration) = self.narration {
                frame.out.sounds.push(SoundCue::Narration(narration));
            }
        }
        let skip = frame.input.events.iter().any(is_skip) || double_click;
        let mut timeline = self.timeline;
        if !(skip && timeline.skip()) {
            let res = &*frame.res;
            timeline.advance(frame.ticks, |step| {
                self.step(step, dialog, res, sound_busy, now_ms, now_tick)
            });
        }
        self.timeline = timeline;
        if self.timeline.phase == Phase::Scroll
            && let Some((item, text)) = dialog.control_mut(TEXT).and_then(|c| c.prologue_mut())
        {
            let elapsed_ms = now_ms.saturating_sub(self.clock_ms.unwrap_or(now_ms));
            let initial = (item.rect.top - item.rect.bottom) as f32;
            text.set_scroll_f32(initial + 1.0 + (elapsed_ms as f32) * (10.0 / 1000.0));
        }
        if self.timeline.phase == Phase::Over && !self.done_sent {
            self.done_sent = true;
            frame.out.sounds.push(SoundCue::FadeOut {
                ticks: SOUND_FADE_TICKS,
            });
            frame.out.actions.push(Action::BriefingDone);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ticks: i32, text_steps: i32, sound_until: i32) -> (Vec<(i32, Step)>, Option<i32>) {
        let mut timeline = Timeline::new();
        let mut steps = Vec::new();
        let mut scrolled = 0;
        let mut ended = None;
        for tick in 0..=ticks {
            let advance = i32::from(tick != 0);
            timeline.advance(advance, |step| {
                steps.push((tick, step));
                if step == Step::Scroll {
                    scrolled += 1;
                    return scrolled >= text_steps && tick >= sound_until;
                }
                false
            });
            if timeline.phase == Phase::Over {
                ended = Some(tick);
                break;
            }
        }
        (steps, ended)
    }

    #[test]
    fn the_ticks_are_the_engines() {
        let (steps, ended) = run(400, 50, 0);
        let at = |wanted: Step| steps.iter().find(|(_, s)| *s == wanted).map(|(t, _)| *t);
        assert_eq!(at(Step::FadeIn(0)), Some(0));
        assert_eq!(at(Step::FadeIn(42)), Some(1));
        assert_eq!(at(Step::FadeIn(170)), Some(4));
        assert_eq!(at(Step::FadeIn(213)), Some(5));
        assert_eq!(at(Step::FadeIn(255)), Some(6));
        assert_eq!(at(Step::FadeInOver), Some(7));
        assert_eq!(at(Step::CrossfadeBegins), Some(158));
        assert_eq!(at(Step::Crossfade(0)), Some(158));
        assert_eq!(at(Step::Crossfade(255)), Some(164));
        assert_eq!(at(Step::CrossfadeOver), Some(165));
        let scrolls: Vec<i32> = steps
            .iter()
            .filter(|(_, s)| *s == Step::Scroll)
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(scrolls[..3], [165, 168, 171]);
        assert_eq!(scrolls.len(), 50);
        assert_eq!(ended, Some(165 + 49 * 3));
    }

    #[test]
    fn it_waits_for_the_narration() {
        let (_, ended) = run(1000, 10, 500);
        assert_eq!(ended, Some(501));
    }

    #[test]
    fn a_skip_is_taken_only_at_the_map_and_the_text() {
        let walk = [
            (0, false),
            (6, false),
            (7, true),
            (157, true),
            (158, false),
            (164, false),
            (165, true),
            (300, true),
        ];
        for (ticks, allowed) in walk {
            let mut timeline = Timeline::new();
            timeline.advance(0, |_| false);
            for _ in 0..ticks {
                timeline.advance(1, |_| false);
            }
            assert_eq!(timeline.phase.can_skip(), allowed, "at tick {ticks}");
            assert_eq!(timeline.skip(), allowed, "at tick {ticks}");
        }
    }

    #[test]
    fn a_long_frame_runs_every_step() {
        let mut timeline = Timeline::new();
        let mut steps = Vec::new();
        timeline.advance(10, |step| {
            steps.push(step);
            false
        });
        assert_eq!(steps.len(), 8);
        assert_eq!(steps[6], Step::FadeIn(255));
        assert_eq!(steps[7], Step::FadeInOver);
        assert_eq!(timeline.phase, Phase::Hold(1));
    }

    #[test]
    fn the_images_change_on_their_seconds() {
        let times = image_times("40,60,x,20");
        assert_eq!(times, [40_000, 100_000, 100_000, 120_000]);
        assert_eq!(image_at(&times, 0), 0);
        assert_eq!(image_at(&times, 40_000), 0);
        assert_eq!(image_at(&times, 40_001), 1);
        assert_eq!(image_at(&times, 100_001), 3);
        assert_eq!(image_at(&times, 1_000_000), 3);
        assert_eq!(image_at(&[], 5), 0);
        assert_eq!(image_times("").len(), 0);
    }
}
