use std::sync::Arc;

use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_session::FilmDetails;
use mandrake_session::films::gameplay_version_to_string;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, TextLocale, text_flags};
use mandrake_view::interface_bitmap::{InterfaceArt, copy_bitmap};

use super::list::{fill_rect, inset};
use super::{ControlProc, DrawContext, ItemState};
use crate::core::format_one;
use crate::resources::Resources;

pub const NO_ART: FourCC = FourCC(*b"insr");
pub const ART_SEQUENCE: i32 = 1;
const LINES: FourCC = FourCC(*b"ingo");
const GAME_TYPES: FourCC = FourCC(*b"innn");
const DIFFICULTIES: FourCC = FourCC(*b"indl");
const SOLO_GAME_TYPE: i16 = 13;
const NORMAL_DIFFICULTY: i16 = 2;
const INSET: i32 = 15;
const MINUTE: i32 = 0x708;
const SECOND: i32 = 0x1e;
const FIRST_OPTION: usize = 0x14;
const LAST_OPTION: usize = 0x21;
const WHITE: RgbColor = [0xffff; 3];
const RED: RgbColor = [0xffff, 0, 0];

pub struct GameGraphic {
    is_replay: bool,
    details: Option<FilmDetails>,
    art: Option<Arc<InterfaceArt>>,
    lines: Vec<Vec<u8>>,
    game_types: Vec<Vec<u8>>,
    difficulties: Vec<Vec<u8>>,
    family: Option<Arc<FontFamily>>,
    locale: Arc<TextLocale>,
}

impl GameGraphic {
    pub fn new(res: &mut Resources) -> Self {
        Self {
            is_replay: false,
            details: None,
            art: None,
            lines: res.string_list(LINES).to_vec(),
            game_types: res.string_list(GAME_TYPES).to_vec(),
            difficulties: res.string_list(DIFFICULTIES).to_vec(),
            family: res.font(0).cloned(),
            locale: Arc::new(res.locale().clone()),
        }
    }

    pub fn set_is_replay(&mut self, item: &mut ItemState, replay: bool) {
        self.is_replay = replay;
        item.dirty = true;
    }

    pub fn set_description(
        &mut self,
        item: &mut ItemState,
        details: Option<FilmDetails>,
        art: Option<Arc<InterfaceArt>>,
    ) {
        self.details = details;
        self.art = art;
        item.dirty = true;
    }

    fn line(&self, index: usize) -> &[u8] {
        self.lines.get(index).map_or(&[], Vec::as_slice)
    }

    fn difficulty(&self, d: i16) -> &[u8] {
        usize::try_from(d)
            .ok()
            .and_then(|d| self.difficulties.get(d))
            .map_or(&[], Vec::as_slice)
    }

    fn solo_options(&self, d: &FilmDetails) -> Vec<u8> {
        let g = &d.game;
        let on = |bit: u32| g.options & bit != 0;
        let mut s = Vec::new();
        for index in FIRST_OPTION..=LAST_OPTION {
            let line = self.line(index);
            let append = match index.wrapping_sub(FIRST_OPTION + 1) {
                0 => (on(game_options::VETERANS)).then(|| line.to_vec()),
                1 if on(game_options::TEAMS) && g.captain == -1 => {
                    Some([line, b" |i(", b"random captain if none selected", b")|p"].concat())
                }
                3 => (on(game_options::HOST_OBSERVER)).then(|| line.to_vec()),
                4 => (!on(game_options::SHOW_ENEMY)).then(|| line.to_vec()),
                10 => {
                    let ironman = self.is_replay
                        && (g.flags & 2 != 0
                            || i32::from(g.version_of_id) <= game_options::GAMEPLAY_1_8_4);
                    if ironman {
                        let mut l = format_one(line, self.difficulty(g.difficulty));
                        l.extend_from_slice(b" |i(Ironman)|p");
                        Some(l)
                    } else if g.difficulty == NORMAL_DIFFICULTY {
                        None
                    } else {
                        Some(format_one(line, self.difficulty(g.difficulty)))
                    }
                }
                _ => None,
            };
            if let Some(line) = append {
                s.extend_from_slice(&line);
                s.push(b'\r');
            }
        }
        s
    }

    fn game_options(&self, d: &FilmDetails) -> Vec<u8> {
        let g = &d.game;
        let on = |bit: u32| g.options & bit != 0;
        let number = |n: i32| n.to_string().into_bytes();
        let mut s = Vec::new();
        for index in FIRST_OPTION..=LAST_OPTION {
            let line = self.line(index);
            let append = match index.wrapping_sub(FIRST_OPTION + 1) {
                0 => (on(game_options::VETERANS)).then(|| line.to_vec()),
                1 => (on(game_options::TEAMS) && g.captain == -1).then(|| line.to_vec()),
                2 => (!on(game_options::ALLIANCES)).then(|| line.to_vec()),
                3 => (on(game_options::HOST_OBSERVER)).then(|| line.to_vec()),
                4 => (!on(game_options::SHOW_ENEMY)).then(|| line.to_vec()),
                5 => (on(game_options::DEATHMATCH)).then(|| line.to_vec()),
                6 => (on(game_options::TEAMS) && g.captain != -1)
                    .then(|| format_one(line, &number(i32::from(g.captain)))),
                7 => Some(format_one(line, &number(g.time_limit / MINUTE))),
                8 if g.planning_time >= MINUTE => {
                    let (minutes, seconds) =
                        (g.planning_time / MINUTE, g.planning_time % MINUTE / SECOND);
                    if seconds == 0 || !line.starts_with(b"%d ") {
                        Some(format_one(line, &number(minutes)))
                    } else {
                        Some([format!("{minutes}:{seconds:02}").as_bytes(), &line[2..]].concat())
                    }
                }
                9 if (1..MINUTE).contains(&g.planning_time) => {
                    Some(format_one(line, &number(g.planning_time / SECOND)))
                }
                10 if g.difficulty != NORMAL_DIFFICULTY => {
                    Some(format_one(line, self.difficulty(g.difficulty)))
                }
                11 => {
                    s.push(b'\r');
                    s.extend_from_slice(&format_one(line, &number(g.starting_locations)));
                    None
                }
                12 => break,
                _ if index == FIRST_OPTION => {
                    (g.planning_time > 0 && on(game_options::UNIT_TRADING)).then(|| line.to_vec())
                }
                _ => None,
            };
            if let Some(line) = append {
                s.extend_from_slice(&line);
                s.push(b'\r');
            }
        }
        s
    }
}

impl ControlProc for GameGraphic {
    fn draw(&self, item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let b = item.rect;
        match &self.art {
            Some(art) if let Some((src, _)) = art.bitmap(ART_SEQUENCE, -1, 0) => {
                let x = b.left + (b.right - b.left) / 2 - src.width / 2;
                let y = b.top + (b.bottom - b.top) / 2 - src.height / 2;
                copy_bitmap(bitmap, src, art.palette(), x, y, b, 0);
            }
            _ => fill_rect(bitmap, [0; 3], b, 0x8000),
        }
        let (Some(family), Some(d)) = (&self.family, &self.details) else {
            return;
        };
        let locale = &self.locale;
        let draw =
            |bitmap: &mut Bitmap32, pen: (i32, i32), rect, style, color, text: &[u8], flags| {
                font::draw_string_at(
                    bitmap,
                    pen,
                    rect,
                    TextClip::None,
                    family,
                    style,
                    text,
                    color,
                    flags,
                    0,
                    locale,
                )
            };
        let _ = ctx;
        if self.is_replay {
            let version = gameplay_version_to_string(d.game.gameplay_version);
            let version = match version.as_str() {
                "1.3" => "1.3.0".to_owned(),
                "1.2" => "1.2.0".to_owned(),
                _ => version,
            };
            let text = format!("v{}", &version[..version.len().min(5)]);
            draw(
                bitmap,
                (0, 0),
                b,
                font::STYLE_BOLD,
                RED,
                text.as_bytes(),
                text_flags::SHADOW | text_flags::RIGHT,
            );
        }
        let rect = inset(b, INSET, INSET);
        let mut text = format_one(self.line(2), &d.name.0);
        text.push(b'\r');
        let mut pen = draw(
            bitmap,
            (0, 0),
            rect,
            font::STYLE_BOLD,
            WHITE,
            &text,
            text_flags::SHADOW,
        );
        if d.game.game_type != SOLO_GAME_TYPE {
            let kind = usize::try_from(d.game.game_type)
                .ok()
                .and_then(|t| self.game_types.get(t))
                .map_or(&[][..], Vec::as_slice);
            let text = format_one(self.line(3), kind);
            pen = draw(
                bitmap,
                pen,
                rect,
                font::STYLE_PLAIN,
                WHITE,
                &text,
                text_flags::SHADOW,
            );
            pen = draw(
                bitmap,
                pen,
                rect,
                font::STYLE_PLAIN,
                WHITE,
                b"\r",
                text_flags::SHADOW,
            );
        }
        let (name, warning) = match &d.game.map_name {
            Some(name) => (name.0.clone(), false),
            None if d.game.plugins_missing => {
                (format_one(b"|b%s", b"Warning: Plugin(s) Not Found"), true)
            }
            None => (b"ERROR".to_vec(), true),
        };
        let mut name = format_one(self.line(2), &name);
        if !self.is_replay
            && i32::from(d.game.version_of_id) > game_options::GAMEPLAY_1_8_4
            && d.game.flags & 1 != 0
        {
            name.extend_from_slice(FROM_SAVE);
        }
        let text = if d.game.options & game_options::TFL != 0 {
            format_one(b"|bTFL|i %s\r\r", &name)
        } else if d.game.options & game_options::ANTI_CLUMP != 0 {
            [&name[..], b" (anti-clump)\r\r"].concat()
        } else {
            [&name[..], b"\r\r"].concat()
        };
        pen = draw(
            bitmap,
            pen,
            rect,
            font::STYLE_ITALIC,
            if warning { RED } else { WHITE },
            &text,
            text_flags::SHADOW,
        );
        let options = if d.game.options & game_options::COOPERATIVE != 0 {
            self.solo_options(d)
        } else {
            self.game_options(d)
        };
        draw(
            bitmap,
            pen,
            rect,
            font::STYLE_PLAIN,
            WHITE,
            &options,
            text_flags::SHADOW | text_flags::WRAP,
        );
    }
}

const FROM_SAVE: &[u8] = b" (From Save)";
