use std::collections::HashMap;

use mandrake_common::FourCC;
use mandrake_sim::map_actions::{SoundActionPlay, SoundPlace};
use mandrake_tag::font::{self, TextClip, text_flags};
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_tag::{PermutationHistory, Rect, SoundTag, TagLookup};

use crate::command_bar::{self, PromptTags};
use crate::unit_info::{string_from_list, string_list};

pub const CONSOLE_TEXT_LIMIT: usize = 0x1000;
pub const LINE_FADE: i32 = 15;
pub const LINE_BAND_TRANSPARENCY: u16 = 0xd000;
pub const LINE_HEIGHT: i32 = 0x16;
pub const LINE_BOTTOM_MARGIN: i32 = 0x14;
pub const PROMPTS_RAISE: i32 = 0x1e;
pub const LINE_TEXT_FLAGS: u32 = text_flags::MIDDLE | text_flags::SHADOW;
pub const SUBTITLES_PREFERENCE: bool = true;
pub const SOUND_FLAG_FIRST_SUBTITLE: u32 = 0x40;

pub const ALLIANCE_STRINGS_ID: FourCC = FourCC(*b"inal");

pub fn alliance_line(
    strings: &[Vec<u8>],
    change: &mandrake_sim::ally::AllianceChange,
    local_team: i16,
    game_mode: i16,
    team_name: impl Fn(i16) -> Vec<u8>,
) -> Option<Vec<u8>> {
    if change.team == local_team && game_mode != 0 {
        return None;
    }
    let format = string_from_list(Some(strings), if change.allied { 0 } else { 1 });
    let a = team_name(change.team);
    let b = if change.other == local_team && game_mode != 0 {
        string_from_list(Some(strings), 2).to_vec()
    } else {
        team_name(change.other)
    };
    Some(printf_strings(format, &[&a, &b]))
}

pub fn team_display_name(
    teams: &[mandrake_sim::players::Team],
    names: &[(mandrake_session::Text, mandrake_session::Text)],
    team: i16,
) -> Vec<u8> {
    let Some(t) = usize::try_from(team).ok().and_then(|i| teams.get(i)) else {
        return Vec::new();
    };
    usize::try_from(t.player)
        .ok()
        .and_then(|p| names.get(p))
        .map(|(n, tn)| {
            if t.players <= 1 {
                n.0.clone()
            } else {
                tn.0.clone()
            }
        })
        .unwrap_or_default()
}

fn printf_strings(format: &[u8], args: &[&[u8]]) -> Vec<u8> {
    let (mut out, mut rest, mut args) = (Vec::new(), format, args.iter());
    while let Some(at) = rest.windows(2).position(|w| w == b"%s") {
        out.extend_from_slice(&rest[..at]);
        out.extend_from_slice(args.next().copied().unwrap_or_default());
        rest = &rest[at + 2..];
    }
    out.extend_from_slice(rest);
    out
}

pub fn stop_text(sim: &mandrake_sim::Simulation) -> Option<Vec<u8>> {
    if let Some(e) = &sim.engine_stop {
        let tick = sim.current_tick;
        let text = if e.site == mandrake_sim::draw_site::NO_ENGINE_MAP {
            format!("Stopped: the map load is not ported: {}", e.reason)
        } else if sim.halt_on_stop {
            format!("Stopped at tick {tick}: not ported: {e}")
        } else {
            format!("The simulation left the engine's model at tick {tick}: {e}")
        };
        return Some(text.into_bytes());
    }
    if !sim.game_over() {
        return None;
    }
    let end = sim.game_end()?;
    let who = match end.winner {
        0 => "Victory",
        1 => "Defeat",
        _ => "Game over",
    };
    Some(format!("{who}: the game ended at tick {}", end.tick).into_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleLine {
    pub player: i16,
    pub stamp: i32,
    pub text: Vec<u8>,
}

pub const CACHED_LINES: usize = 10;
const CACHED_LINE_BYTES: usize = 0x100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Console {
    lines: Vec<ConsoleLine>,
    cached: Vec<ConsoleLine>,
    pub life: i32,
    pub cap: usize,
    teams: Vec<bool>,
    cached_teams: Vec<bool>,
    revision: u64,
}

impl Console {
    pub fn for_mode(mode: i16) -> Self {
        let (life, cap) = if mode == 0 { (180, 50) } else { (90, 6) };
        Self {
            lines: Vec::new(),
            cached: Vec::new(),
            life,
            cap,
            teams: Vec::new(),
            cached_teams: Vec::new(),
            revision: 0,
        }
    }

    pub fn cached(&self) -> &[ConsoleLine] {
        &self.cached
    }

    pub fn set_cached(&mut self, lines: Vec<ConsoleLine>) {
        self.cached = lines;
        self.cached.truncate(CACHED_LINES);
        self.cached_teams = vec![false; self.cached.len()];
        self.revision += 1;
    }

    pub fn lines(&self) -> &[ConsoleLine] {
        &self.lines
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn text_bytes(&self) -> usize {
        self.lines.iter().map(|l| l.text.len() + 1).sum()
    }

    pub fn cputs(&mut self, player: i16, text: &[u8], now: i32) {
        self.cputs_line(player, text, false, now);
    }

    pub fn is_team_line(&self, k: usize) -> bool {
        self.teams.get(k).copied().unwrap_or(false)
    }

    pub fn cputs_line(&mut self, player: i16, text: &[u8], team: bool, now: i32) {
        let text = &text[..text.iter().position(|&b| b == 0).unwrap_or(text.len())];
        while !self.lines.is_empty()
            && (text.len() + self.text_bytes() > CONSOLE_TEXT_LIMIT || self.lines.len() >= self.cap)
        {
            self.lines.pop();
            self.teams.truncate(self.lines.len());
        }
        if text.len() + 1 > CONSOLE_TEXT_LIMIT {
            return;
        }
        if self.cached.len() == CACHED_LINES {
            self.cached.remove(0);
            self.cached_teams.remove(0);
        }
        self.cached_teams.push(team);
        self.cached.push(ConsoleLine {
            player,
            stamp: 0,
            text: text[..text.len().min(CACHED_LINE_BYTES - 1)].to_vec(),
        });
        self.lines.insert(
            0,
            ConsoleLine {
                player,
                stamp: now,
                text: text.to_vec(),
            },
        );
        self.teams.insert(0, team);
        self.revision += 1;
    }

    pub fn expire(&mut self, now: i32) {
        if let Some(k) = self
            .lines
            .iter()
            .position(|l| now > l.stamp + self.life + LINE_FADE - 1)
        {
            self.lines.truncate(k);
            self.teams.truncate(k);
            self.revision += 1;
        }
    }
}

pub fn line_band(frame: Rect, k: usize, prompts_shown: bool) -> Rect {
    let bottom = frame.bottom - if prompts_shown { PROMPTS_RAISE } else { 0 };
    let height = if k == 0 { LINE_HEIGHT - 2 } else { LINE_HEIGHT };
    Rect::new(0, 0, height, frame.right - frame.left).offset(
        frame.left,
        bottom - LINE_BOTTOM_MARGIN - LINE_HEIGHT * k as i32,
    )
}

fn ends_sentence(text: &[u8]) -> bool {
    text.iter()
        .rev()
        .find(|b| !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\'' | b')' | 0xd3 | 0xd5))
        .is_some_and(|b| matches!(b, b'.' | b'!' | b'?' | 0xc9))
}

fn wrap_to_band(tags: &PromptTags, text: &[u8], band_width: i32) -> Vec<Vec<u8>> {
    let limit = band_width - 2 * tags.family.base.margin();
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let line = font::wrap_string(
            &tags.family,
            font::STYLE_PLAIN,
            rest,
            limit,
            font::TEXT_WRAP,
            &tags.locale,
        );
        let count = line.count.clamp(1, rest.len());
        let printed = &rest[..count];
        let end = printed
            .iter()
            .rposition(|b| !b.is_ascii_whitespace())
            .map_or(0, |i| i + 1);
        if end > 0 {
            lines.push(printed[..end].to_vec());
        }
        rest = &rest[count..];
    }
    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownLine {
    pub player: i16,
    pub team: bool,
    pub stamp: i32,
    pub text: Vec<u8>,
}

pub fn displayed_rows(console: &Console, tags: &PromptTags, band_width: i32) -> Vec<ShownLine> {
    rows(
        console.lines(),
        |k| console.is_team_line(k),
        tags,
        band_width,
    )
}

fn rows(
    lines: &[ConsoleLine],
    team: impl Fn(usize) -> bool,
    tags: &PromptTags,
    band_width: i32,
) -> Vec<ShownLine> {
    let mut texts: Vec<ShownLine> = Vec::new();
    for (k, line) in lines.iter().enumerate().rev() {
        if line.player != -1 {
            texts.push(ShownLine {
                player: line.player,
                team: team(k),
                stamp: line.stamp,
                text: line.text.clone(),
            });
            continue;
        }
        match texts.iter_mut().rev().find(|t| t.player == -1) {
            Some(t) if t.stamp == line.stamp && !ends_sentence(&t.text) => {
                let end = t
                    .text
                    .iter()
                    .rposition(|b| !b.is_ascii_whitespace())
                    .map_or(0, |i| i + 1);
                t.text.truncate(end);
                t.text.push(b' ');
                t.text.extend_from_slice(&line.text);
            }
            _ => texts.push(ShownLine {
                player: -1,
                team: false,
                stamp: line.stamp,
                text: line.text.clone(),
            }),
        }
    }
    let mut shown: Vec<ShownLine> = texts
        .into_iter()
        .flat_map(|row| {
            if row.player != -1 {
                return vec![row];
            }
            wrap_to_band(tags, &row.text, band_width)
                .into_iter()
                .map(|text| ShownLine {
                    text,
                    ..row.clone()
                })
                .collect()
        })
        .collect();
    shown.reverse();
    shown
}

pub fn back_log_rows(console: &Console, tags: &PromptTags, band_width: i32) -> Vec<ShownLine> {
    let mut shown = Vec::new();
    for (k, line) in console.cached.iter().enumerate().rev() {
        let team = console.cached_teams.get(k).copied().unwrap_or(false);
        if line.player != -1 {
            shown.push(ShownLine {
                player: line.player,
                team,
                stamp: line.stamp,
                text: line.text.clone(),
            });
            continue;
        }
        let parts = wrap_to_band(tags, &line.text, band_width);
        shown.extend(parts.into_iter().rev().map(|text| ShownLine {
            player: -1,
            team,
            stamp: line.stamp,
            text,
        }));
    }
    shown
}

pub const CHAT_TAG_WIDTH: i32 = 0x8c;
pub const TEAM_LINE_BAND_TRANSPARENCY: u16 = 0xb000;
pub const CHAT_NAME_FLAGS: u32 = text_flags::MIDDLE | text_flags::RIGHT | text_flags::SHADOW;
const CHAT_TEXT_LIMIT: usize = 0x3ff;

pub fn append_chat(
    console: &mut Console,
    tags: &PromptTags,
    player: i16,
    text: &[u8],
    team: bool,
    screen_width: i32,
    now: i32,
) -> usize {
    let text = &text[..text.iter().position(|&b| b == 0).unwrap_or(text.len())];
    let text = &text[..text.len().min(CHAT_TEXT_LIMIT)];
    let width = screen_width - if player != -1 { CHAT_TAG_WIDTH } else { 0 };
    let mut rest = text;
    let mut printed = 0;
    loop {
        let line = font::wrap_string(
            &tags.family,
            font::STYLE_PLAIN,
            rest,
            width,
            font::TEXT_WRAP,
            &tags.locale,
        );
        let count = line.count.clamp(usize::from(!rest.is_empty()), rest.len());
        let part = &rest[..count];
        let end = part.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
        console.cputs_line(player, &part[..end], team, now);
        printed += 1;
        rest = &rest[count..];
        while rest.first() == Some(&b' ') {
            rest = &rest[1..];
        }
        if rest.is_empty() {
            return printed;
        }
    }
}

pub fn chat_fade_stamp(stamp: i32, life: i32, now: i32) -> u32 {
    let past = now - stamp - life;
    if past <= 0 {
        return 0;
    }
    ((past.min(LINE_FADE - 1) as u32) << 16) / LINE_FADE as u32
}

pub fn chat_fade(line: &ShownLine, life: i32, now: i32) -> u32 {
    chat_fade_stamp(line.stamp, life, now)
}

pub fn chat_tag_transparency(fade: u32) -> u16 {
    ((fade >> 2) + 0x4000 + (fade >> 1)).min(0xffff) as u16
}

pub fn chat_rects(band: Rect) -> (Rect, Rect) {
    let split = (band.left + CHAT_TAG_WIDTH).min(band.right);
    (
        Rect::new(band.top, band.left, band.bottom, split),
        Rect::new(band.top, split, band.bottom, band.right),
    )
}

pub fn text_rgba(tags: &PromptTags, text: &[u8], rect: Rect, flags: u32) -> (u32, u32, Vec<u8>) {
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    command_bar::coverage_image(w, h, |bmp| {
        font::draw_string(
            bmp,
            Rect::new(0, 0, h, w),
            TextClip::None,
            &tags.family,
            font::STYLE_PLAIN,
            text,
            tags.color,
            flags,
            0,
            &tags.locale,
        );
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsoleInput {
    pub state: InputState,
    pub text: Vec<u8>,
    pub max: usize,
    pub since: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InputState {
    #[default]
    Off,
    Typing,
    Entered,
    Cancelled,
}

pub const INPUT_ENTER_KEYS: [u8; 2] = [0x38, 0x62];
pub const INPUT_DELETE_KEY: u8 = 0x1d;
pub const INPUT_EDIT_FLAGS: u16 = 3;
const _: () = assert!(INPUT_EDIT_FLAGS & 1 != 0);
#[cfg(test)]
pub const INPUT_CARET_KEYS: [u8; 6] = [0x48, 0x49, 0x4a, 0x4b, 0x4d, 0x50];
#[cfg(test)]
pub const INPUT_FORWARD_DELETE_KEY: u8 = 0x4f;

impl ConsoleInput {
    pub fn start(&mut self, max: usize, now: i32) {
        *self = Self {
            state: InputState::Typing,
            text: Vec::new(),
            max,
            since: now,
        };
    }

    pub fn stop(&mut self) {
        self.state = InputState::Off;
    }

    pub fn typing(&self) -> bool {
        self.state == InputState::Typing
    }

    pub fn key(&mut self, key: u8, ch: Option<u8>, fits: impl Fn(&[u8]) -> bool, now: i32) -> bool {
        if !self.typing() {
            return false;
        }
        if INPUT_ENTER_KEYS.contains(&key) {
            self.state = InputState::Entered;
            return true;
        }
        let changed = if key == INPUT_DELETE_KEY {
            self.text.pop().is_some()
        } else if let Some(c) = ch.filter(|_| self.text.len() < self.max) {
            let mut next = self.text.clone();
            next.push(c);
            let ok = fits(&next);
            if ok {
                self.text = next;
            }
            ok
        } else {
            false
        };
        if changed {
            self.since = now;
        }
        true
    }

    pub fn click(&mut self) {
        if self.typing() {
            self.state = InputState::Cancelled;
        }
    }

    pub fn cursor(&self, now: i32) -> Option<u16> {
        if !self.typing() {
            return None;
        }
        let phase = (now - self.since).rem_euclid(30);
        (phase < 0x16).then(|| {
            if phase > 0x12 {
                ((phase - 0x13) as u32 * 0x10000 / 3) as u16
            } else {
                0
            }
        })
    }
}

pub fn line_rgba(tags: &PromptTags, text: &[u8], band: Rect) -> (u32, u32, Vec<u8>) {
    text_rgba(tags, text, band, LINE_TEXT_FLAGS)
}

struct Subtitled {
    tag: SoundTag,
    history: PermutationHistory,
    strings: Option<Vec<Vec<u8>>>,
}

pub struct SoundSubtitles {
    sounds: HashMap<FourCC, Option<Subtitled>>,
    random: LocalRandom,
}

impl Default for SoundSubtitles {
    fn default() -> Self {
        Self {
            sounds: HashMap::new(),
            random: LocalRandom::new(0),
        }
    }
}

impl SoundSubtitles {
    pub fn play(&mut self, tags: &TagLookup, sound: FourCC) -> Option<Vec<u8>> {
        self.play_from(|group, id| tags.get(group, id), sound)
    }

    pub fn play_from(
        &mut self,
        get: impl Fn(FourCC, FourCC) -> Option<Vec<u8>>,
        sound: FourCC,
    ) -> Option<Vec<u8>> {
        let entry = self
            .sounds
            .entry(sound)
            .or_insert_with(|| {
                let tag = SoundTag::from_bytes(&get(FourCC(*b"soun"), sound)?).ok()?;
                let strings = tag
                    .subtitle_string_list
                    .and_then(|id| get(FourCC(*b"stli"), id))
                    .map(|d| string_list(&d));
                Some(Subtitled {
                    tag,
                    history: PermutationHistory::default(),
                    strings,
                })
            })
            .as_mut()?;
        let rng = &mut self.random;
        let picked = entry
            .tag
            .choose_permutation(&mut entry.history, || rng.next_u16());
        if !SUBTITLES_PREFERENCE {
            return None;
        }
        let strings = entry.strings.as_deref()?;
        let i = match picked {
            Some(p) if entry.tag.flags & SOUND_FLAG_FIRST_SUBTITLE == 0 => p as i32,
            _ => 0,
        };
        let first = i32::from(entry.tag.first_subtitle_index);
        if first < 0 {
            return None;
        }
        Some(string_from_list(Some(strings), first + i).to_vec())
    }

    pub fn print(
        &mut self,
        tags: &TagLookup,
        plays: &[SoundActionPlay],
        console: &mut Console,
    ) -> Vec<Vec<u8>> {
        let mut printed = Vec::new();
        for p in plays {
            if matches!(&p.place, SoundPlace::NearestVisible(c) if c.is_empty()) {
                continue;
            }
            let Some(text) = self.play(tags, FourCC(p.sound.to_be_bytes())) else {
                continue;
            };
            let now = crate::media::local_time(p.time.max(0) as u32 + 1);
            console.cputs(-1, &text, now);
            printed.push(text);
        }
        printed
    }
}

pub fn take_sound_plays(sim: &mut mandrake_sim::Simulation) -> Vec<SoundActionPlay> {
    (sim.engine.map_actions.as_mut())
        .map(|a| std::mem::take(&mut a.sound_plays))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alliance_lines_as_ally_with_team() {
        use mandrake_sim::ally::AllianceChange;
        let strings: Vec<Vec<u8>> = vec![
            b"%s allied with %s.".to_vec(),
            b"%s broke the alliance with %s.".to_vec(),
            b"you".to_vec(),
        ];
        let name = |t: i16| [&b"Second"[..], b"Joiner", b"Ours"][t as usize].to_vec();
        let ours = AllianceChange {
            team: 2,
            other: 1,
            allied: true,
        };
        let line = |c: &AllianceChange, local, mode| alliance_line(&strings, c, local, mode, name);
        assert_eq!(
            line(&ours, 2, 1),
            None,
            "the local team's own change: a sound only"
        );
        assert_eq!(line(&ours, 1, 1).unwrap(), b"Ours allied with you.");
        assert_eq!(line(&ours, 0, 1).unwrap(), b"Ours allied with Joiner.");
        assert_eq!(line(&ours, 2, 0).unwrap(), b"Ours allied with Joiner.");
        assert_eq!(line(&ours, 1, 0).unwrap(), b"Ours allied with Joiner.");
        let broken = AllianceChange {
            allied: false,
            ..ours
        };
        assert_eq!(
            line(&broken, 0, 1).unwrap(),
            b"Ours broke the alliance with Joiner."
        );
    }

    #[test]
    fn a_team_is_named_by_its_captain() {
        use mandrake_session::Text;
        use mandrake_sim::players::Team;
        let teams = [
            Team {
                players: 1,
                player: 1,
                ..Default::default()
            },
            Team {
                players: 2,
                player: 0,
                ..Default::default()
            },
        ];
        let names = [
            (Text(b"Cap".to_vec()), Text(b"Wolves".to_vec())),
            (Text(b"Solo".to_vec()), Text(b"Lone".to_vec())),
        ];
        assert_eq!(team_display_name(&teams, &names, 0), b"Solo");
        assert_eq!(team_display_name(&teams, &names, 1), b"Wolves");
        assert!(team_display_name(&teams, &names, 2).is_empty());
        assert!(team_display_name(&teams, &names, -1).is_empty());
    }

    fn line(console: &Console) -> Vec<&[u8]> {
        console.lines().iter().map(|l| l.text.as_slice()).collect()
    }

    #[test]
    fn cputs_inserts_at_the_top_and_keeps_the_cap() {
        let mut c = Console::for_mode(1);
        assert_eq!((c.life, c.cap), (90, 6));
        assert_eq!(Console::for_mode(0).life, 180);
        assert_eq!(Console::for_mode(0).cap, 50);
        for k in 0..8u8 {
            c.cputs(-1, &[b'a' + k], i32::from(k));
        }
        assert_eq!(line(&c), [b"h", b"g", b"f", b"e", b"d", b"c"]);
        assert_eq!(c.text_bytes(), 12);
        for k in 8..12u8 {
            c.cputs(-1, &[b'a' + k], i32::from(k));
        }
        let cached: Vec<&[u8]> = c.cached().iter().map(|l| l.text.as_slice()).collect();
        assert_eq!(
            cached,
            [b"c", b"d", b"e", b"f", b"g", b"h", b"i", b"j", b"k", b"l"]
        );
        let mut c = Console::for_mode(0);
        c.cputs(-1, &[b'x'; 3000], 0);
        c.cputs(-1, &[b'y'; 1000], 1);
        assert_eq!(c.lines().len(), 2);
        c.cputs(-1, &[b'z'; 200], 2);
        assert_eq!(c.lines().len(), 2);
        assert_eq!(c.lines()[1].text.len(), 1000);
    }

    #[test]
    fn the_console_input_takes_keys_as_the_edit_text() {
        let mut i = ConsoleInput::default();
        assert!(!i.key(0x20, Some(b'a'), |_| true, 0), "not typing");
        i.start(3, 100);
        for c in b"abcd" {
            assert!(i.key(0x20, Some(*c), |_| true, 101));
        }
        assert_eq!(i.text, b"abc");
        assert!(i.key(INPUT_DELETE_KEY, None, |_| true, 102));
        assert!(
            i.key(0x20, Some(b'x'), |t| t.len() < 2, 103),
            "taken, not typed"
        );
        assert_eq!((i.text.as_slice(), i.since), (&b"ab"[..], 102));
        assert_eq!(i.cursor(102), Some(0));
        assert_eq!(i.cursor(102 + 18), Some(0));
        assert_eq!(i.cursor(102 + 20), Some(0x5555));
        assert_eq!(i.cursor(102 + 22), None);
        assert_eq!(i.cursor(102 + 30), Some(0));
        assert!(i.key(INPUT_ENTER_KEYS[1], None, |_| true, 104));
        assert_eq!(i.state, InputState::Entered);
        assert_eq!(i.cursor(104), None);
        i.start(3, 0);
        i.click();
        assert_eq!(i.state, InputState::Cancelled);
    }

    #[test]
    fn the_console_input_has_no_caret_keys() {
        let mut i = ConsoleInput::default();
        i.start(10, 0);
        for c in b"ab" {
            i.key(0x20, Some(*c), |_| true, 1);
        }
        for key in INPUT_CARET_KEYS
            .into_iter()
            .chain([INPUT_FORWARD_DELETE_KEY])
        {
            assert!(i.key(key, None, |_| true, 5), "key {key:#x} taken");
            assert_eq!((i.text.as_slice(), i.since), (&b"ab"[..], 1), "{key:#x}");
        }
        i.key(0x20, Some(b'c'), |_| true, 6);
        assert_eq!(i.text, b"abc", "typed at the end");
    }

    #[test]
    fn lines_live_life_plus_14_and_take_the_older_ones_with_them() {
        let mut c = Console::for_mode(1);
        c.cputs(-1, b"old", 10);
        assert_eq!(c.cached().len(), 1);
        c.cputs(-1, b"new", 50);
        c.expire(10 + 90 + 14);
        assert_eq!(c.lines().len(), 2);
        c.expire(10 + 90 + 15);
        assert_eq!(line(&c), [b"new"]);
        let mut c = Console::for_mode(1);
        c.cputs(-1, b"a", 10);
        c.cputs(-1, b"b", 10);
        c.expire(200);
        assert!(c.lines().is_empty());
    }

    #[test]
    fn bands_stack_above_the_control_bar() {
        let frame = Rect::new(64, 0, 736, 1024);
        assert_eq!(line_band(frame, 0, false), Rect::new(716, 0, 736, 1024));
        assert_eq!(line_band(frame, 1, false), Rect::new(694, 0, 716, 1024));
        assert_eq!(line_band(frame, 2, false), Rect::new(672, 0, 694, 1024));
        assert_eq!(line_band(frame, 0, true), Rect::new(686, 0, 706, 1024));
        let frame = Rect::new(64, 0, 2128, 3840);
        assert_eq!(line_band(frame, 0, false), Rect::new(2108, 0, 2128, 3840));
    }
}
