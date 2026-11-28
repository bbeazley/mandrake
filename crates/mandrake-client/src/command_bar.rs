use mandrake_common::FourCC;
use mandrake_sim::attack_ai::{AttackDefinition, Own};
use mandrake_sim::engine_rng::{EngineWorld, Unmodelled};
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, TextLocale, text_flags};
use mandrake_tag::interface::{
    COLOR_CONSOLE_TEXT, FONT_SCREEN_HIGH_RES, FONT_SCREEN_LOW_RES, InterfaceDefinition,
};
use mandrake_tag::{CollectionTag, InterfaceColorTable, Rect, TagLookup};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarAction {
    Whisper,
    Ally,
    General(u16),
    Special,
    Formation(i16),
}

pub const GENERAL_STOP: u16 = 0;
pub const GENERAL_SCATTER: u16 = 1;
pub const GENERAL_HOLD: u16 = 4;

pub fn bar_action(kind: u8) -> Option<BarAction> {
    Some(match kind {
        1 => BarAction::Whisper,
        2 => BarAction::Ally,
        3 => BarAction::General(GENERAL_STOP),
        4 => BarAction::General(GENERAL_HOLD),
        5 | 6 => BarAction::General(GENERAL_SCATTER),
        7 => BarAction::Special,
        8..=17 => BarAction::Formation(i16::from(kind) - 8),
        _ => return None,
    })
}

pub const GENERAL_SPECIAL: u16 = 3;

pub const SELECTION_MODE_NORMAL: i16 = -1;
pub const SELECTION_MODE_FORMATION: i16 = 1;
pub const SELECTION_MODE_ATTACK_GROUND: i16 = 2;
pub const SELECTION_MODE_SPECIAL: i16 = 3;

pub const SELECTION_MODE_PRESET: i16 = 0;
pub const SELECTION_MODE_ALLY: i16 = 8;
pub const SELECTION_MODE_FORMATION_KEY: i16 = 12;
pub const SELECTION_MODE_FORMATION_KEY_IN_PLACE: i16 = 13;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolePrompts {
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    pub selection_mode: i16,
    pub input: crate::console::ConsoleInput,
    pub cursor: Option<u16>,
    revision: u64,
}

impl Default for ConsolePrompts {
    fn default() -> Self {
        Self {
            left: Vec::new(),
            right: Vec::new(),
            selection_mode: SELECTION_MODE_NORMAL,
            input: crate::console::ConsoleInput::default(),
            cursor: None,
            revision: 0,
        }
    }
}

impl ConsolePrompts {
    pub fn set(&mut self, left: &[u8], right: &[u8], mode: i16) {
        if self.input.state != crate::console::InputState::Off {
            self.input.stop();
            self.cursor = None;
            self.revision += 1;
        }
        self.set_prompts(left, right, mode);
    }

    pub fn start_chat(&mut self, left: &[u8], right: &[u8], mode: i16, now: i32) {
        if !matches!(self.selection_mode, CHAT_MODE_TEAM | CHAT_MODE_ALL) {
            self.input.start(CHAT_INPUT_LENGTH, now);
            self.revision += 1;
        }
        self.set_prompts(left, right, mode);
    }

    pub fn input_changed(&mut self) {
        self.revision += 1;
    }

    pub fn update_cursor(&mut self, now: i32) {
        let cursor = self.input.cursor(now);
        if cursor != self.cursor {
            self.cursor = cursor;
            self.revision += 1;
        }
    }

    fn set_prompts(&mut self, left: &[u8], right: &[u8], mode: i16) {
        let cut = |s: &[u8]| s[..s.len().min(127)].to_vec();
        let (left, right) = (cut(left), cut(right));
        if (&left, &right, mode) != (&self.left, &self.right, self.selection_mode) {
            self.left = left;
            self.right = right;
            self.selection_mode = mode;
            self.revision += 1;
        }
    }

    pub fn clear(&mut self) {
        self.set(b"", b"", SELECTION_MODE_NORMAL);
    }

    pub fn shown(&self) -> bool {
        !self.left.is_empty() || !self.right.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}

pub const SELECTION_STRINGS_ID: FourCC = FourCC(*b"insm");
pub const FORMATION_NAMES_ID: FourCC = FourCC(*b"infn");
pub const FORMATION_MODE_STRING: usize = 2;
pub const ATTACK_GROUND_STRINGS: (usize, usize) = (4, 5);
pub const SPECIAL_ABILITY_STRINGS: (usize, usize) = (6, 7);

pub const ALLY_STRINGS: (usize, usize) = (16, 17);

pub const CHAT_MODE_TEAM: i16 = 5;
pub const CHAT_MODE_ALL: i16 = 6;
pub const CHAT_INPUT_LENGTH: usize = 0xc0;
pub const CHAT_COMMAND: u8 = 10;

pub fn chat_command_data(team: bool, text: &[u8]) -> Vec<u8> {
    let text = &text[..text.iter().position(|&b| b == 0).unwrap_or(text.len())];
    let text = &text[..text.len().min(CHAT_INPUT_LENGTH)];
    let mut d = u16::from(team).to_be_bytes().to_vec();
    d.extend_from_slice(text);
    d.push(0);
    d
}

pub fn chat_command_parts(data: &[u8]) -> Option<(u16, &[u8])> {
    let flags = u16::from_be_bytes([*data.first()?, *data.get(1)?]);
    let text = &data[2..];
    let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    Some((flags, &text[..end.min(CHAT_INPUT_LENGTH)]))
}

pub const CONSOLE_MODE: i16 = 1;

pub const PROMPT_BAND_TRANSPARENCY: u16 = 0x8000;
pub const PROMPT_BAND_HEIGHT: i32 = 0x16;

pub struct PromptTags {
    pub family: FontFamily,
    pub locale: TextLocale,
    pub color: RgbColor,
    pub selection_strings: Option<Vec<Vec<u8>>>,
    pub formation_names: Option<Vec<Vec<u8>>>,
}

impl PromptTags {
    pub fn load(tags: &TagLookup, inte: &InterfaceDefinition, locale: TextLocale) -> Option<Self> {
        Self::load_with_console_mode(tags, inte, locale, CONSOLE_MODE)
    }

    pub fn load_with_console_mode(
        tags: &TagLookup,
        inte: &InterfaceDefinition,
        locale: TextLocale,
        mode: i16,
    ) -> Option<Self> {
        let fc = |s: &[u8; 4]| FourCC(*s);
        let font = if mode != 0 {
            FONT_SCREEN_HIGH_RES
        } else {
            FONT_SCREEN_LOW_RES
        };
        let family = FontFamily::load(inte.font(font)?, |id| tags.get(fc(b"font"), id)).ok()?;
        let colors = inte
            .collection
            .and_then(|id| tags.get(fc(b".256"), id))
            .and_then(|d| CollectionTag::from_bytes(&d).ok())
            .and_then(|c| InterfaceColorTable::from_collection(&c))?;
        let c = colors.color(COLOR_CONSOLE_TEXT);
        let list = |id| {
            tags.get(fc(b"stli"), id)
                .map(|d| crate::unit_info::string_list(&d))
        };
        Some(Self {
            family,
            locale,
            color: [
                u16::from(c[0]) << 8,
                u16::from(c[1]) << 8,
                u16::from(c[2]) << 8,
            ],
            selection_strings: list(SELECTION_STRINGS_ID),
            formation_names: list(FORMATION_NAMES_ID),
        })
    }

    pub fn formation_prompts(&self, formation: i16) -> (Vec<u8>, Vec<u8>) {
        let get = |l: &Option<Vec<Vec<u8>>>, i: usize| {
            l.as_ref()
                .and_then(|l| l.get(i))
                .cloned()
                .unwrap_or_default()
        };
        (
            get(&self.selection_strings, FORMATION_MODE_STRING),
            get(&self.formation_names, formation.max(0) as usize),
        )
    }

    pub fn preset_mode_prompts(&self, mode: i16, alternate: bool) -> (Vec<u8>, Vec<u8>) {
        match mode {
            SELECTION_MODE_PRESET if alternate => (self.selection_prompts((1, 1)).0, Vec::new()),
            SELECTION_MODE_PRESET => self.selection_prompts((0, 1)),
            SELECTION_MODE_FORMATION_KEY_IN_PLACE => (
                self.selection_prompts((2, 2)).0,
                b"(change in place)".to_vec(),
            ),
            _ => self.selection_prompts((2, 25)),
        }
    }

    pub fn selection_prompts(&self, pair: (usize, usize)) -> (Vec<u8>, Vec<u8>) {
        let get = |i: usize| {
            self.selection_strings
                .as_ref()
                .and_then(|l| l.get(i))
                .cloned()
                .unwrap_or_default()
        };
        (get(pair.0), get(pair.1))
    }
}

pub fn prompt_band(frame: Rect) -> Rect {
    Rect::new(0, 0, PROMPT_BAND_HEIGHT, frame.right - frame.left - 8)
        .offset(frame.left + 4, frame.bottom - 0x1a)
}

pub fn draw_prompts(
    tags: &PromptTags,
    prompts: &ConsolePrompts,
    band: Rect,
    background: u32,
) -> Bitmap32 {
    let (w, h) = (band.right - band.left, band.bottom - band.top);
    let mut bmp = Bitmap32::new(w, h, background);
    let rect = Rect::new(0, 0, h, w);
    let pen = font::draw_string_at(
        &mut bmp,
        (0, 0),
        rect,
        TextClip::None,
        &tags.family,
        font::STYLE_PLAIN,
        &prompts.left,
        tags.color,
        text_flags::MIDDLE,
        0,
        &tags.locale,
    );
    let mut pen = (pen.0 + 3, pen.1);
    if prompts.input.typing() {
        pen = font::draw_string_at(
            &mut bmp,
            pen,
            rect,
            TextClip::None,
            &tags.family,
            font::STYLE_PLAIN,
            &prompts.input.text,
            tags.color,
            text_flags::MIDDLE,
            0,
            &tags.locale,
        );
        if let Some(t) = prompts.cursor {
            pen = font::draw_string_at(
                &mut bmp,
                pen,
                rect,
                TextClip::None,
                &tags.family,
                font::STYLE_PLAIN,
                &[INPUT_CURSOR],
                tags.color,
                text_flags::MIDDLE,
                u32::from(t),
                &tags.locale,
            );
        }
    }
    font::draw_string_at(
        &mut bmp,
        pen,
        rect,
        TextClip::None,
        &tags.family,
        font::STYLE_PLAIN,
        &prompts.right,
        tags.color,
        text_flags::MIDDLE | text_flags::RIGHT,
        0,
        &tags.locale,
    );
    bmp
}

pub const INPUT_CURSOR: u8 = 0xff;

pub fn input_fits(tags: &PromptTags, prompts: &ConsolePrompts, band: Rect, text: &[u8]) -> bool {
    let (w, h) = (band.right - band.left, band.bottom - band.top);
    let mut bmp = Bitmap32::new(w.max(1), h.max(1), 0);
    let pen = font::draw_string_at(
        &mut bmp,
        (0, 0),
        Rect::new(0, 0, h, w),
        TextClip::None,
        &tags.family,
        font::STYLE_PLAIN,
        &prompts.left,
        tags.color,
        text_flags::MIDDLE,
        0,
        &tags.locale,
    );
    let (left, right) = (pen.0 + 3, w - 0xe);
    font::string_width(&tags.family, font::STYLE_PLAIN, text, &tags.locale) < right - 2 - left
}

pub fn coverage_image(w: i32, h: i32, mut draw: impl FnMut(&mut Bitmap32)) -> (u32, u32, Vec<u8>) {
    let (w, h) = (w.max(1), h.max(1));
    let mut black = Bitmap32::new(w, h, 0xff00_0000);
    let mut white = Bitmap32::new(w, h, 0xffff_ffff);
    draw(&mut black);
    draw(&mut white);
    (w as u32, h as u32, coverage_rgba(&black, &white))
}

pub fn prompt_rgba(tags: &PromptTags, prompts: &ConsolePrompts, band: Rect) -> (u32, u32, Vec<u8>) {
    let black = draw_prompts(tags, prompts, band, 0xff00_0000);
    let white = draw_prompts(tags, prompts, band, 0xffff_ffff);
    (
        black.width.max(0) as u32,
        black.height.max(0) as u32,
        coverage_rgba(&black, &white),
    )
}

pub fn coverage_rgba(black: &Bitmap32, white: &Bitmap32) -> Vec<u8> {
    let mut out = Vec::with_capacity(black.pixels.len() * 4);
    for (&b, &wh) in black.pixels.iter().zip(&white.pixels) {
        let ch = |p: u32, s: u32| ((p >> s) & 0xff) as i32;
        let diff = [16, 8, 0].map(|s| ch(wh, s) - ch(b, s));
        let a = (255 - diff.iter().copied().max().unwrap_or(255)).clamp(0, 255);
        let c = [16, 8, 0].map(|s| {
            if a == 0 {
                0
            } else {
                ((ch(b, s) * 255 + a / 2) / a).min(255) as u8
            }
        });
        out.extend_from_slice(&[c[0], c[1], c[2], a as u8]);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarOutcome {
    NoSelection,
    LookedAt,
    General(u16),
    Formation { formation: i16, prompts: bool },
    SpecialTarget { list: Option<FourCC> },
    NoSpecialAbility,
    FormationMove(mandrake_sim::LocalMovement),
    Chat { mode: i16 },
    Ignored,
    NotPorted(&'static str),
    AllyMode,
}

pub const SPECIAL_NONE: i16 = -1;

pub fn special_ability_of_selection(
    w: &EngineWorld,
    selection: &[usize],
) -> Result<(i16, Option<FourCC>), Unmodelled> {
    let (mut kind, mut uniform, mut list) = (SPECIAL_NONE, true, None);
    for &i in selection {
        let Some(a) = special_attack(w, i)? else {
            continue;
        };
        let k = i16::from(a.flags & 0x40 == 0);
        if uniform && kind == SPECIAL_NONE {
            kind = k;
        } else if k != kind {
            uniform = false;
            kind = SPECIAL_NONE;
        }
        let d = &w.monster_definitions[w.monsters[i].definition].ai;
        list = definition_special_strings(&d.raw);
    }
    Ok((kind, list))
}

fn special_attack(w: &EngineWorld, i: usize) -> Result<Option<AttackDefinition>, Unmodelled> {
    let m = &w.monsters[i];
    let own = Own {
        position: m.position,
        timer: 0,
        speed: m.speed,
        facing: 0,
        desired_facing: 0,
    };
    let a = w.choose_attack(i, 2, &own)?;
    if a == -1 {
        return Ok(None);
    }
    let d = &w.monster_definitions[m.definition].ai;
    Ok(d.attack(a as i8).cloned())
}

fn definition_special_strings(mons: &[u8]) -> Option<FourCC> {
    let b: [u8; 4] = mons.get(0x20c..0x210)?.try_into().ok()?;
    let v = u32::from_be_bytes(b);
    (v != 0 && v != u32::MAX).then_some(FourCC(b))
}

pub fn special_against_friendlies(w: &EngineWorld, i: usize) -> Result<bool, Unmodelled> {
    if w.formation_globals.gameplay_version <= mandrake_common::game_options::GAMEPLAY_1_5 {
        let d = &w.monster_definitions[w.monsters[i].definition].ai;
        return Ok(d
            .attacks
            .iter()
            .any(|a| a.flags & 0x80 != 0 && a.flags_3c & 1 != 0));
    }
    Ok(special_attack(w, i)?.is_some_and(|a| a.flags & 0x80 != 0 && a.flags_3c & 1 != 0))
}

#[allow(clippy::too_many_arguments)]
pub fn run_bar_action(
    action: BarAction,
    shift: bool,
    selection: &[u32],
    look: bool,
    sim: &mut mandrake_sim::Simulation,
    hud: Option<&mut crate::hud::Hud>,
    sprites: &mut crate::move_marker::LocalSprites,
    order_hide: &mut crate::interface_state::OrderFlagHide,
    now: i32,
) -> BarOutcome {
    use crate::move_marker::RemoveMode;
    match action {
        BarAction::General(kind) => {
            if selection.is_empty() {
                return BarOutcome::NoSelection;
            }
            if look {
                return BarOutcome::LookedAt;
            }
            sim.queue_local_general(mandrake_sim::LocalGeneral {
                unit_ids: selection.to_vec(),
                kind,
            });
            bar_tail(sim, hud, sprites, order_hide, selection, now);
            BarOutcome::General(kind)
        }
        BarAction::Formation(formation) => {
            let mut most_used = formation;
            {
                let w = &mut sim.engine;
                let monsters: Vec<usize> = selection
                    .iter()
                    .filter_map(|&id| w.monster_index_by_identifier(id as u16))
                    .collect();
                match w.assign_formation(&monsters, formation) {
                    Ok(best) => most_used = best,
                    Err(e) => tracing::warn!("assign_formation_to_selection: {e}"),
                }
            }
            if shift && !selection.is_empty() {
                return match formation_movement(sim, selection, formation, most_used) {
                    Some(c) => {
                        sim.queue_local_movement(c.clone());
                        BarOutcome::FormationMove(c)
                    }
                    None => BarOutcome::NoSelection,
                };
            }
            sprites.remove_local_sprites(RemoveMode::ClearUnlessTimed, now);
            sim.local_waypoints.clear();
            if selection.is_empty() {
                return BarOutcome::NoSelection;
            }
            let quiet = look || (sim.engine.mode == 0);
            let prompts = !quiet && hud.is_some();
            if let (false, Some(h)) = (quiet, hud) {
                h.formation_mode(formation);
            }
            BarOutcome::Formation { formation, prompts }
        }
        BarAction::Special => {
            sprites.remove_local_sprites(RemoveMode::ClearUnlessTimed, now);
            sim.local_waypoints.clear();
            if selection.is_empty() {
                return BarOutcome::NoSelection;
            }
            if look {
                return BarOutcome::LookedAt;
            }
            let w = &sim.engine;
            if w.mode == 0 {
                return BarOutcome::NoSpecialAbility;
            }
            let monsters: Vec<usize> = selection
                .iter()
                .filter_map(|&id| w.monster_index_by_identifier(id as u16))
                .collect();
            let (kind, list) = match special_ability_of_selection(w, &monsters) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("make_selected_units_use_special_ability: {e}");
                    return BarOutcome::NotPorted(
                        "the special ability of an artifact (attack -0x45, [0x85f270])",
                    );
                }
            };
            match kind {
                0 => {
                    sim.queue_local_general(mandrake_sim::LocalGeneral {
                        unit_ids: selection.to_vec(),
                        kind: GENERAL_SPECIAL,
                    });
                    bar_tail(sim, hud, sprites, order_hide, selection, now);
                    BarOutcome::General(GENERAL_SPECIAL)
                }
                1 => {
                    if let Some(h) = hud {
                        h.special_ability_mode(list);
                    }
                    BarOutcome::SpecialTarget { list }
                }
                _ => BarOutcome::NoSpecialAbility,
            }
        }
        BarAction::Whisper => {
            let teams = crate::hud::BarTeams::of(&sim.engine);
            let Some(mode) = teams.chat_mode(shift) else {
                return BarOutcome::Ignored;
            };
            sprites.remove_local_sprites(RemoveMode::ClearUnlessTimed, now);
            if !sim.is_network_game() {
                return BarOutcome::Ignored;
            }
            if let Some(h) = hud {
                h.start_chat(mode, now);
            }
            BarOutcome::Chat { mode }
        }
        BarAction::Ally => {
            sprites.remove_local_sprites(RemoveMode::ClearUnlessTimed, now);
            let options = sim.game_setup.as_ref().map_or(0, |g| g.options);
            let selects = crate::hud::BarTeams::of(&sim.engine).ally_selects(options);
            if !selects {
                return BarOutcome::Ignored;
            }
            if let Some(h) = hud {
                let (left, right) = h
                    .prompt_tags()
                    .map(|t| t.selection_prompts(ALLY_STRINGS))
                    .unwrap_or_default();
                h.console.set(&left, &right, SELECTION_MODE_ALLY);
            }
            BarOutcome::AllyMode
        }
    }
}

pub fn ally_pick_team(
    w: &mandrake_sim::engine_rng::EngineWorld,
    number: usize,
    shift: bool,
) -> Option<i16> {
    let local = w.local_team();
    let teams = &w.players.teams;
    let others =
        |t: usize| t as i32 != i32::from(local) && teams[t].players != 0 && teams[t].flags & 1 == 0;
    let mut n = number;
    if shift {
        let count = (0..teams.len()).filter(|&t| others(t)).count();
        if n + 10 < count {
            n += 10;
        }
    }
    let own = usize::try_from(local).ok().and_then(|l| teams.get(l))?;
    if own.flags & 1 != 0 {
        return None;
    }
    (0..teams.len())
        .filter(|&t| others(t))
        .nth(n)
        .map(|t| t as i16)
}

pub fn ally_pick(
    sim: &mut mandrake_sim::Simulation,
    hud: Option<&mut crate::hud::Hud>,
    number: usize,
    shift: bool,
) -> Option<i16> {
    let w = &sim.engine;
    let team = ally_pick_team(w, number, shift)?;
    let local = w.local_team();
    sim.queue_local_ally(local, team);
    if let Some(h) = hud {
        h.console.clear();
    }
    Some(team)
}

pub fn bar_tail(
    sim: &mut mandrake_sim::Simulation,
    hud: Option<&mut crate::hud::Hud>,
    sprites: &mut crate::move_marker::LocalSprites,
    order_hide: &mut crate::interface_state::OrderFlagHide,
    selection: &[u32],
    now: i32,
) {
    sprites.remove_local_sprites(crate::move_marker::RemoveMode::ClearUnlessTimed, now);
    sim.local_waypoints.clear();
    if let Some(h) = hud {
        h.console.clear();
    }
    order_hide.order(selection);
}

fn change_basis2d(v: (f32, f32), basis: (f32, f32), forward: bool) -> (f32, f32) {
    let by = if forward { basis.1 } else { -basis.1 };
    (basis.0 * v.0 + by * v.1, basis.0 * v.1 - by * v.0)
}

fn formation_movement(
    sim: &mut mandrake_sim::Simulation,
    selection: &[u32],
    formation: i16,
    most_used: i16,
) -> Option<mandrake_sim::LocalMovement> {
    use mandrake_math::engine::{arctangent, distance2d, trig_tables};
    let w = &sim.engine;
    let t = trig_tables();
    let table = |facing: u16| ((usize::from(facing) + 0x20) >> 6) & 0x3ff;
    let ends: Vec<((i32, i32), u16)> = selection
        .iter()
        .filter_map(|&id| {
            let m = w.monster_index_by_identifier(id as u16)?;
            let mover = sim.units.get(&id)?.engine.as_ref();
            Some(match mover {
                Some(mv) if !mv.waypoints.is_empty() => (*mv.waypoints.last()?, mv.word_26),
                Some(mv) => ((mv.x, mv.y), mv.facing),
                None => (w.monsters[m].position, 0),
            })
        })
        .collect();
    let n = ends.len() as i64;
    if n == 0 {
        return None;
    }
    let (mut sx, mut sy, mut sc, mut ss) = (0i64, 0i64, 0i64, 0i64);
    for &((x, y), f) in &ends {
        sx += i64::from(x);
        sy += i64::from(y);
        sc += i64::from(t.cosine[table(f)]);
        ss += i64::from(t.sine[table(f)]);
    }
    let centre = ((sx / n) as i32, (sy / n) as i32);
    let facing = arctangent(i32::from((sc / n) as i16), i32::from((ss / n) as i16));
    let k = table(facing);
    let basis = (f32::from(t.cosine[k]), f32::from(t.sine[k]));
    let length = (basis.0 * basis.0 + basis.1 * basis.1).sqrt();
    let basis = (basis.0 * (1.0 / length), basis.1 * (1.0 / length));
    let (mut least, mut most) = (f32::MAX, f32::MIN_POSITIVE);
    for &((x, y), _) in &ends {
        let d = (
            x.wrapping_sub(centre.0) as f32,
            y.wrapping_sub(centre.1) as f32,
        );
        let along = change_basis2d(d, basis, true).0;
        least = along.min(least);
        most = along.max(most);
    }
    let ahead = match most_used as u16 {
        0..=4 | 8 => most,
        6 | 7 => least,
        _ => 0.0,
    };
    let offset = change_basis2d((ahead, 0.0), basis, false);
    let point = (
        (centre.0 as f32 + offset.0) as i32,
        (centre.1 as f32 + offset.1) as i32,
    );
    if !sim
        .local_waypoints
        .iter()
        .any(|w| distance2d(point.0.wrapping_sub(w.0), point.1.wrapping_sub(w.1)) < 0x200)
        && sim.local_waypoints.len() < 4
    {
        sim.local_waypoints.push(point);
    }
    Some(mandrake_sim::LocalMovement {
        unit_ids: selection.to_vec(),
        points: sim.local_waypoints.clone(),
        header: mandrake_sim::engine_rng::MovementHeader {
            flags: 0x18,
            formation,
            facing,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ally_world(teams: &[(u16, u8)], local: i8) -> mandrake_sim::engine_rng::EngineWorld {
        let mut w = mandrake_sim::engine_rng::EngineWorld::default();
        w.mode = 1;
        w.players.teams = teams
            .iter()
            .map(|&(flags, players)| mandrake_sim::players::Team {
                flags,
                players,
                player: 0,
            })
            .collect();
        w.players.players = vec![mandrake_sim::players::Player {
            flags: 0,
            field_09: 0,
            team: local,
            field_0b: local,
            id: 0,
        }];
        w.local_player = 0;
        w
    }

    #[test]
    fn the_ally_pick_counts_the_other_teams_with_players() {
        let w = ally_world(&[(0, 1), (0, 0), (0, 1), (1, 1), (0, 1)], 0);
        assert_eq!(ally_pick_team(&w, 0, false), Some(2));
        assert_eq!(ally_pick_team(&w, 1, false), Some(4));
        assert_eq!(ally_pick_team(&w, 2, false), None);
        assert_eq!(ally_pick_team(&w, 0, true), Some(2));
        let many: Vec<(u16, u8)> = (0..14).map(|_| (0, 1)).collect();
        let w = ally_world(&many, 0);
        assert_eq!(ally_pick_team(&w, 2, true), Some(13));
        assert_eq!(
            ally_pick_team(&w, 3, true),
            Some(4),
            "3 + 10 is past the 13 others"
        );
        assert_eq!(
            ally_pick_team(&ally_world(&[(0, 1), (0, 1)], -1), 0, false),
            None
        );
        assert_eq!(
            ally_pick_team(&ally_world(&[(1, 1), (0, 1)], 0), 0, false),
            None
        );
    }

    #[test]
    fn the_ally_pick_queues_the_ally_command() {
        let mut sim = mandrake_sim::Simulation::new(1);
        sim.set_engine_world(ally_world(&[(0, 1), (0, 1), (0, 1)], 1));
        assert_eq!(ally_pick(&mut sim, None, 1, false), Some(2));
        let out = sim.take_outgoing_commands();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].command_type, mandrake_rec::CommandType::Ally);
        assert_eq!(out[0].raw_payload, [0, 0, 0, 0, 0, 1, 0, 2]);
    }

    #[test]
    fn kinds_follow_the_sub_4a4560_table() {
        assert_eq!(bar_action(3), Some(BarAction::General(0)));
        assert_eq!(bar_action(4), Some(BarAction::General(4)));
        assert_eq!(bar_action(5), bar_action(6));
        assert_eq!(bar_action(5), Some(BarAction::General(1)));
        assert_eq!(bar_action(7), Some(BarAction::Special));
        assert_eq!(bar_action(8), Some(BarAction::Formation(0)));
        assert_eq!(bar_action(17), Some(BarAction::Formation(9)));
        assert_eq!(bar_action(0), None);
        assert_eq!(bar_action(18), None);
        assert_eq!(bar_action(1), Some(BarAction::Whisper));
    }

    #[test]
    fn the_band_sits_above_the_control_bar() {
        let band = prompt_band(Rect::new(64, 0, 736, 1024));
        assert_eq!(band, Rect::new(710, 4, 732, 1020));
        let band = prompt_band(Rect::new(64, 0, 2128, 3840));
        assert_eq!(band, Rect::new(2102, 4, 2124, 3836));
    }

    #[test]
    fn the_chat_command_data() {
        assert_eq!(chat_command_data(true, b"hi"), b"\0\x01hi\0");
        let long = chat_command_data(false, &[b'a'; 300]);
        assert_eq!(long.len(), 2 + CHAT_INPUT_LENGTH + 1);
        assert_eq!(chat_command_parts(&long), Some((0, &[b'a'; 0xc0][..])));
        assert_eq!(chat_command_parts(b"\0\x01hi\0"), Some((1, &b"hi"[..])));
        assert_eq!(chat_command_parts(b"\0"), None);
    }

    #[test]
    fn prompts_change_only_when_they_differ() {
        let mut p = ConsolePrompts::default();
        assert!(!p.shown());
        p.set(b"Formation Mode", b"Short Line", SELECTION_MODE_FORMATION);
        let r = p.revision();
        assert!(p.shown());
        p.set(b"Formation Mode", b"Short Line", SELECTION_MODE_FORMATION);
        assert_eq!(p.revision(), r);
        p.clear();
        assert!(!p.shown());
        assert_eq!(p.selection_mode, SELECTION_MODE_NORMAL);
        p.set(&[b'x'; 200], b"", 1);
        assert_eq!(p.left.len(), 127);
    }

    #[test]
    fn coverage_image_clamps_empty_and_negative_dimensions() {
        let (w, h, rgba) = coverage_image(-10, 0, |_| {});
        assert_eq!((w, h, rgba.len()), (1, 1, 4));
    }
}
