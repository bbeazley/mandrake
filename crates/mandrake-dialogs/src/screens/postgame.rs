use mandrake_common::FourCC;
use mandrake_session::{Action, Postgame, Screen, SoundCue, rules};
use mandrake_tag::font::RgbColor;

use super::save_film::{self, Prompt};
use super::{OpenScreen, ScreenFrame};
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::Resources;

pub const EPDI: FourCC = FourCC(*b"epdi");
const BLACK_ITEM: i16 = 1;
pub const NEXT_LEVEL: i16 = 2;
pub const LAST_SAVED: i16 = 3;
pub const RESTART: i16 = 4;
pub const QUIT: i16 = 5;
pub const SKIP: i16 = 6;
const REPORT_ITEM: i16 = 7;
pub const SAVE_FILM: i16 = 9;
const CLOSES: [i16; 5] = [NEXT_LEVEL, LAST_SAVED, RESTART, QUIT, SKIP];

const BLACK: RgbColor = [0; 3];
const FADE_TICKS: i32 = 8;
const REPORT_COLORS_SEQUENCE: i32 = 2;

const fn fade_strength(step: i32) -> u32 {
    let strength = (step << 8) / FADE_TICKS;
    (if strength > 0xff { 0xff } else { strength }) as u32
}

const fn fade_in_transparency(step: i32) -> u32 {
    fade_strength(step) << 8
}

const fn fade_out_transparency(step: i32) -> u32 {
    (!fade_strength(step) & 0xff) << 8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    FadingIn(i32),
    Waiting,
    FadingOut(i32),
    Left,
}

pub fn open(
    data: &Postgame,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let Some(mut dialog) = make_dialog(data, res) else {
        tracing::warn!(art = ?data.art, "dialogs: the post-game page has nothing to show");
        return Some(Box::new(NoDialog { answered: false }));
    };
    set_black(&mut dialog, Some(BLACK), fade_in_transparency(0));
    tracing::info!(art = ?data.art, "postgame: the page opens");
    stack.push(dialog);
    Some(Box::new(Page {
        stage: Stage::FadingIn(0),
        pressed: None,
        ambient_started: false,
        prompt: None,
    }))
}

fn make_dialog(data: &Postgame, res: &mut Resources) -> Option<Dialog> {
    let (art_id, art, sequence) = match data.art.and_then(|id| Some((id, res.art(id)?))) {
        Some((id, art)) => (id, art, i32::from(data.art_sequence)),
        None => (
            rules::FALLBACK_POST_ART,
            res.art(rules::FALLBACK_POST_ART)?,
            0,
        ),
    };
    art.bitmap(sequence, 0, 0)?;
    let def = res.dialog(EPDI)?;
    let mut dialog = Dialog::new(&def, EPDI, dialog_flags::CENTERED, res);
    let exits = &data.exits;
    dialog.set_visible(NEXT_LEVEL, exits.next);
    dialog.set_visible(LAST_SAVED, exits.last_saved);
    dialog.set_visible(RESTART, exits.restart);
    dialog.set_visible(SKIP, exits.skip);
    dialog.set_enabled(SAVE_FILM, exits.save_film);
    dialog.set_visible(QUIT, exits.quit);
    dialog.set_cancel_item(if exits.quit { QUIT } else { NEXT_LEVEL });
    let default = if exits.next {
        Some(NEXT_LEVEL)
    } else if exits.last_saved {
        Some(LAST_SAVED)
    } else {
        exits.restart.then_some(RESTART)
    };
    if let Some(default) = default {
        dialog.set_default_item(default);
    }
    dialog.set_background(Some(art.clone()), sequence);
    if let Some((item, bitmap)) = dialog.control_mut(BLACK_ITEM).and_then(|c| c.bitmap_mut()) {
        bitmap.set_drawing_flags(item, 0, 0);
    }
    dialog.set_enabled(BLACK_ITEM, false);
    if let Some((item, report)) = dialog.control_mut(REPORT_ITEM).and_then(|c| c.report_mut()) {
        report.set_page(item, data.report.epilogue.as_ref());
        if art_id != rules::FALLBACK_POST_ART {
            let color = |entry| {
                mandrake_tag::hud::rgb_color_from_sequence(
                    &art.raw,
                    &art.coll,
                    REPORT_COLORS_SEQUENCE,
                    entry,
                )
            };
            report.set_colors(item, [color(0), color(1), color(2)]);
        }
    }
    Some(dialog)
}

fn set_black(dialog: &mut Dialog, color: Option<RgbColor>, transparency: u32) {
    if let Some((item, bitmap)) = dialog.control_mut(BLACK_ITEM).and_then(|c| c.bitmap_mut()) {
        bitmap.set_color(item, color);
        bitmap.set_transparency(item, transparency);
    }
}

struct Page {
    stage: Stage,
    pressed: Option<i16>,
    ambient_started: bool,
    prompt: Option<Prompt>,
}

impl Page {
    fn press(&mut self, item: i16, dialog: &mut Dialog) {
        if !CLOSES.contains(&item) {
            return;
        }
        self.pressed = Some(item);
        self.stage = match self.stage {
            Stage::FadingIn(_) | Stage::Waiting => {
                set_black(dialog, Some(BLACK), fade_out_transparency(0));
                Stage::FadingOut(0)
            }
            Stage::FadingOut(_) | Stage::Left => Stage::Left,
        };
    }

    fn tick(&mut self, dialog: &mut Dialog) {
        self.stage = match self.stage {
            Stage::FadingIn(step) if step < FADE_TICKS => {
                set_black(dialog, Some(BLACK), fade_in_transparency(step + 1));
                Stage::FadingIn(step + 1)
            }
            Stage::FadingIn(_) => {
                set_black(dialog, None, 0);
                Stage::Waiting
            }
            Stage::FadingOut(step) if step < FADE_TICKS => {
                set_black(dialog, Some(BLACK), fade_out_transparency(step + 1));
                Stage::FadingOut(step + 1)
            }
            Stage::FadingOut(_) => Stage::Left,
            stage => stage,
        };
    }
}

fn action(item: i16) -> Option<Action> {
    match item {
        NEXT_LEVEL => Some(Action::NextLevel),
        LAST_SAVED => Some(Action::LastSavedGame),
        RESTART => Some(Action::RestartLevel),
        QUIT => Some(Action::QuitToMenu),
        SKIP => Some(Action::SkipLevel),
        _ => None,
    }
}

impl OpenScreen for Page {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Postgame(data) = screen else {
            return;
        };
        if !self.ambient_started {
            self.ambient_started = true;
            if let Some(ambient) = data.ambient {
                frame.out.sounds.push(SoundCue::Ambient(ambient));
            }
        }
        if save_film::follow(&mut self.prompt, data.save_film.as_ref(), frame) {
            return;
        }
        let Some(dialog) = frame.stack.find_mut(EPDI) else {
            return;
        };
        if self.stage == Stage::Left {
            return;
        }
        dialog.set_enabled(SAVE_FILM, data.exits.save_film);
        for _ in 0..frame.ticks {
            self.tick(dialog);
        }
        for hit in frame.hits.iter().filter(|h| h.dialog == EPDI) {
            if hit.item == SAVE_FILM && self.stage == Stage::Waiting {
                frame.out.actions.push(Action::SaveFilm);
                continue;
            }
            self.press(hit.item, dialog);
        }
        if self.stage == Stage::Left {
            match self.pressed.and_then(action) {
                Some(action) => frame.out.actions.push(action),
                None => tracing::warn!(pressed = ?self.pressed, "postgame: no action"),
            }
        }
    }
}

struct NoDialog {
    answered: bool,
}

impl OpenScreen for NoDialog {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Postgame(data) = screen else {
            return;
        };
        if !std::mem::replace(&mut self.answered, true) {
            frame.out.actions.push(if data.exits.next {
                Action::NextLevel
            } else {
                Action::RestartLevel
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineDialogs;
    use mandrake_session::report::{EpiloguePage, PostgameReport, ReportSection, SectionKind};
    use mandrake_session::screen::PostgameExits;
    use mandrake_session::{FrameInput, FrontEnd, Text, UiEvent};
    use mandrake_tag::TagLookup;

    fn text(s: &str) -> Text {
        Text(s.as_bytes().to_vec())
    }

    fn willow_page() -> EpiloguePage {
        let unit = |name: &str, casualties: &str| {
            vec![
                text(&format!("|i{name}|p")),
                text("0.0%"),
                text("0"),
                text(casualties),
            ]
        };
        let line = |kind, heading: &str| ReportSection {
            kind,
            heading: text(heading),
            columns: Vec::new(),
            rows: Vec::new(),
        };
        EpiloguePage {
            title: text("Willow Creek (Normal)"),
            sections: vec![
                ReportSection {
                    kind: SectionKind::Survivors,
                    heading: text("12 Survivors"),
                    columns: vec![
                        text("|bUnit|p"),
                        text("|bDamage Inflicted|p"),
                        text("|bKills|p"),
                        text("|bCasualties|p"),
                    ],
                    rows: vec![
                        unit("Warriors", "0/8"),
                        unit("Bowmen", "0/3"),
                        unit("Dwarf", "0/1"),
                    ],
                },
                line(SectionKind::DamageRatio, "|iNo Damage Taken|p"),
                line(
                    SectionKind::KillRatio {
                        no_casualties: true,
                    },
                    "|iNo Casualties!|p",
                ),
            ],
        }
    }

    fn postgame(won: bool, save_film: bool) -> Postgame {
        Postgame {
            report: PostgameReport {
                end_code: if won { 1 } else { 3 },
                epilogue: Some(willow_page()),
                ..PostgameReport::default()
            },
            art: Some(FourCC(*b"01po")),
            art_sequence: i16::from(!won),
            ambient: None,
            exits: PostgameExits {
                next: won,
                restart: !won,
                last_saved: false,
                skip: false,
                save_film,
                quit: true,
            },
            save_film: None,
        }
    }

    const SIZE: (u32, u32) = (800, 600);

    fn input(now_ms: u64, events: &[UiEvent]) -> FrameInput<'_> {
        FrameInput {
            now_ms,
            size: SIZE,
            events,
            sound_busy: false,
            shift_held: false,
        }
    }

    const fn ms(tick: u64) -> u64 {
        (tick * 1000).div_ceil(30)
    }

    #[test]
    fn the_fades_transparency_is_the_engines() {
        let steps = |f: fn(i32) -> u32| (0..=FADE_TICKS).map(f).collect::<Vec<_>>();
        assert_eq!(
            steps(fade_in_transparency),
            [
                0, 0x2000, 0x4000, 0x6000, 0x8000, 0xa000, 0xc000, 0xe000, 0xff00
            ]
        );
        assert_eq!(
            steps(fade_out_transparency),
            [
                0xff00, 0xdf00, 0xbf00, 0x9f00, 0x7f00, 0x5f00, 0x3f00, 0x1f00, 0
            ]
        );
    }

    #[test]
    fn with_nothing_to_show_the_game_goes_on() {
        let mut front_end = EngineDialogs::new(TagLookup::empty());
        for (won, wanted) in [(true, Action::NextLevel), (false, Action::RestartLevel)] {
            let data = postgame(won, false);
            let screen = Screen::Postgame(&data);
            front_end.frame(&Screen::Loading, &input(0, &[]));
            assert_eq!(front_end.frame(&screen, &input(0, &[])).actions, [wanted]);
            assert_eq!(front_end.frame(&screen, &input(ms(1), &[])).actions, []);
        }
    }
}
