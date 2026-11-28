use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_session::Text;
use mandrake_sim::statistics::StatisticsView;
use mandrake_tag::font::{self, Bitmap32, TextClip, text_flags};
use mandrake_tag::{Rect, TagLookup};

use crate::command_bar::{self, PromptTags};

pub const SCORE_COLUMN_WIDTHS: [i32; 16] = [
    50, 20, 20, 130, 130, 20, 20, 50, 50, 60, 40, 40, 35, 40, 60, 130,
];
pub const LINE_HEIGHT: i32 = 0x16;
pub const LINE_PITCH: i32 = 0x1b;
pub const MARGIN: i32 = 5;
pub const TEAM_LINE_WIDTH: i32 = 0x122;
pub const PLAYER_LINE_INDENT: i32 = 0x19;
pub const PLAYER_LINE_WIDTH: i32 = 0x10e;
pub const FILL_TRANSPARENCY: u16 = 0x8000;
pub const FADED_FILL_TRANSPARENCY: u16 = 0xe000;
pub const FADED_TEXT_TRANSPARENCY: u16 = 0x8000;
pub const RIGHT_COLUMN_GAP: i32 = 10;
pub const END_STRINGS_ID: FourCC = FourCC(*b"inne");
pub const CHECK_MARK: &[u8] = b"\xc3";
pub const OPTION_NO_AUTOMATIC_TABLE: u32 = game_options::NO_AUTOMATIC_TABLE;
pub const OPTION_VICTORY_HEADER: u32 = game_options::COOPERATIVE;
pub const OPTION_ALL_LINES_ACTIVE: u32 = game_options::DEATHMATCH;
pub const OPTION_BODY_COUNT_SHARE: u32 = game_options::TFL;
pub const OPTION_OLD_BODY_COUNT_SHARE: u32 = game_options::ANTI_CLUMP;

pub const STAR_POINTS: [(i32, i32); 10] = [
    (50, 10),
    (59, 39),
    (90, 39),
    (66, 58),
    (75, 90),
    (50, 70),
    (23, 90),
    (34, 58),
    (10, 39),
    (41, 39),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScoreClock {
    pub tick: i32,
    pub blink: i32,
    pub state: i16,
    pub timer: i32,
    pub game_time: i32,
    pub flags: u32,
    pub options: u32,
    pub game_type: i16,
    pub countdown: i16,
    pub end_code: i16,
    pub end_tick: i32,
    pub version: i32,
}

impl ScoreClock {
    pub fn of(sim: &mandrake_sim::Simulation) -> Option<Self> {
        let w = &sim.engine;
        let tick = sim.current_tick as i32;
        Some(Self {
            tick,
            blink: tick.wrapping_add(1),
            state: w.game.state,
            timer: w.game.timer,
            game_time: w.game.game_time,
            flags: w.game.flags,
            options: w.game_options,
            game_type: w.game_type,
            countdown: w.statistics.countdown,
            end_code: w.game.end_code,
            end_tick: w.game.end_tick,
            version: w.formation_globals.gameplay_version,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Nothing,
    Header,
    Table,
}

pub fn shown(c: &ScoreClock, keyed: bool) -> Shown {
    if keyed {
        return Shown::Table;
    }
    let (mut draw, mut level) = (false, 0u8);
    if c.countdown >= 0 && c.options & OPTION_NO_AUTOMATIC_TABLE == 0 {
        let mut header = false;
        if c.state != 1 {
            header = if c.state == 2 && c.timer > 899 && c.timer % 1800 > 0x95 {
                c.game_time - 150 < c.timer
            } else {
                true
            };
            draw = header;
        }
        if c.flags & 1 != 0 {
            header = true;
            draw = true;
        }
        if c.flags & 2 == 0 {
            if c.tick < 150 {
                draw = true;
                level = 2;
            } else {
                level = u8::from(header);
            }
        } else {
            draw = true;
            level = 2;
        }
    }
    match (draw, level) {
        (false, _) | (_, 0) => Shown::Nothing,
        (true, 1) => Shown::Header,
        _ => Shown::Table,
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScoreStrings {
    pub game_types: Option<Vec<Vec<u8>>>,
    pub end: Option<Vec<Vec<u8>>>,
    pub map_name: Option<Vec<u8>>,
    pub alternate_level: bool,
}

impl ScoreStrings {
    pub fn load(tags: &TagLookup, mesh_strings: Option<FourCC>, alternate_level: bool) -> Self {
        let list = |id| {
            tags.get(FourCC(*b"stli"), id)
                .map(|d| crate::unit_info::string_list(&d))
        };
        Self {
            game_types: list(mandrake_tag::hud::GAME_SCORING_NAMES_ID),
            end: list(END_STRINGS_ID),
            map_name: mesh_strings
                .and_then(list)
                .and_then(|l| l.into_iter().next()),
            alternate_level,
        }
    }

    fn end(&self, i: usize) -> Vec<u8> {
        string(&self.end, i)
    }
}

fn string(list: &Option<Vec<Vec<u8>>>, i: usize) -> Vec<u8> {
    list.as_ref()
        .and_then(|l| l.get(i))
        .cloned()
        .unwrap_or_default()
}

pub fn time_to_string(ticks: i32) -> Vec<u8> {
    let days = ticks / 0x278d00;
    let rest = ticks % 0x278d00;
    let hours = rest / 0x1a5e0;
    let rest = rest % 0x1a5e0;
    let (minutes, seconds) = (rest / 0x708, (rest % 0x708) / 0x1e);
    let mut s = String::new();
    if days != 0 {
        s += &format!("{days} day{} ", if days == 1 { "" } else { "s" });
    }
    if hours == 0 {
        s += &format!("{minutes}:{seconds:02}\r");
    } else {
        s += &format!("{hours}:{minutes:02}:{seconds:02}\r");
    }
    s.into_bytes()
}

pub fn header(c: &ScoreClock, strings: &ScoreStrings) -> (Vec<u8>, Vec<u8>) {
    let name = if c.state == 5 && c.options & OPTION_VICTORY_HEADER != 0 {
        if c.end_code == 1 || (c.end_code == 2 && strings.alternate_level) {
            b"VICTORY".to_vec()
        } else {
            b"DEFEAT".to_vec()
        }
    } else if c.state == 0 && c.blink & 0x40 != 0 {
        strings.end(4)
    } else if c.game_type == mandrake_sim::engine_rng::SOLO_GAME_TYPE {
        strings.map_name.clone().unwrap_or_default()
    } else {
        string(&strings.game_types, c.game_type.max(0) as usize)
    };
    let time = match c.state {
        4 if c.blink & 8 != 0 => Vec::new(),
        4 => strings.end(2),
        3 if c.blink & 0x10 != 0 => Vec::new(),
        3 => strings.end(1),
        1 => time_to_string(c.tick),
        0 | 2 => time_to_string(c.timer),
        5 if c.flags & 0x20 != 0 && c.options & OPTION_VICTORY_HEADER != 0 => {
            time_to_string(c.end_tick)
        }
        5 => strings.end(3),
        _ => strings.end(0),
    };
    (name, time)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreLine {
    pub row: i32,
    pub player: bool,
    pub active: bool,
    pub fill: Option<[u16; 3]>,
    pub score: Option<Vec<u8>>,
    pub name: Vec<u8>,
    pub right: Vec<u8>,
    pub stars: Vec<[u16; 3]>,
}

fn score_text(c: &ScoreClock, score: i32, total: i32, numbers: u32, flags_held: i16) -> Vec<u8> {
    match c.game_type {
        0 => {
            let share = c.options & OPTION_BODY_COUNT_SHARE != 0
                || (c.version <= game_options::GAMEPLAY_1_7_1
                    && c.options & OPTION_OLD_BODY_COUNT_SHARE != 0);
            if share {
                let mut v = if total != 0 {
                    ((i64::from(score) * 100) / i64::from(total)) as i32
                } else {
                    0
                };
                if score < 0 {
                    v = v.wrapping_neg();
                }
                format!("{v}%").into_bytes()
            } else if c.version > game_options::GAMEPLAY_1_7_2 {
                format!("{}", (score + 0x80) >> 8).into_bytes()
            } else {
                format!("{score}").into_bytes()
            }
        }
        1 | 2 | 5 | 6 if score != 0 => CHECK_MARK.to_vec(),
        3 | 4 => (0..9)
            .map(|i| {
                if numbers & (1 << i) != 0 {
                    b'1' + i as u8
                } else {
                    b' '
                }
            })
            .collect(),
        7 | 8 | 10..=13 if score != 0 => format!("{score}").into_bytes(),
        9 | 14 if score != 0 => {
            format!("{:02}:{:02}", score / 0x708, (score / 0x1e) % 0x3c).into_bytes()
        }
        15 if score != 0 => format!(
            "{:03}:{:02} (x{flags_held})",
            score / 0x708,
            (score / 0x1e) % 0x3c
        )
        .into_bytes(),
        _ => Vec::new(),
    }
}

fn percent(part: i32, total: i32) -> Vec<u8> {
    let v = if total != 0 {
        ((i64::from(part) * 100) / i64::from(total)) as i32
    } else {
        0
    };
    format!("{v}%").into_bytes()
}

pub fn score_lines(
    c: &ScoreClock,
    shown: Shown,
    view: &StatisticsView,
    strings: &ScoreStrings,
    names: &[(Text, Text)],
    alliances: &mandrake_sim::ally::AllianceView,
) -> Vec<ScoreLine> {
    if shown == Shown::Nothing {
        return Vec::new();
    }
    let (name, time) = header(c, strings);
    let mut lines = vec![ScoreLine {
        row: 0,
        player: false,
        active: true,
        fill: None,
        score: None,
        name,
        right: time,
        stars: Vec::new(),
    }];
    if shown != Shown::Table {
        return lines;
    }
    let s = view.statistics;
    let players = view.players;
    let name_of = |p: usize, team: bool| -> Vec<u8> {
        names
            .get(p)
            .map(|(n, t)| if team { t.0.clone() } else { n.0.clone() })
            .unwrap_or_default()
    };
    let colour = |p: usize, which: usize| -> [u16; 3] {
        players
            .colors
            .get(p)
            .map_or([0; 3], |c| [c[which][0], c[which][1], c[which][2]])
    };
    let total_score = s
        .team_rankings
        .iter()
        .fold(0i32, |a, t| a.wrapping_add(t.score));
    let mut row = 1;
    for t in &s.team_rankings {
        let Some(team) = usize::try_from(t.team)
            .ok()
            .and_then(|i| players.teams.get(i))
        else {
            continue;
        };
        let Some(captain) = usize::try_from(team.player).ok() else {
            continue;
        };
        let Some(cap) = players.players.get(captain) else {
            continue;
        };
        if team.players == 0 || cap.flags & 4 != 0 || cap.field_09 != 0 {
            continue;
        }
        let total = t.losses_cost.wrapping_add(t.survivors_cost);
        let score = s
            .team_scores
            .get(t.team as usize)
            .copied()
            .unwrap_or_default();
        let active = c.options & OPTION_ALL_LINES_ACTIVE != 0 || t.survivors != 0;
        let stars = (0..players.teams.len())
            .filter(|&j| j as i16 != t.team && alliances.teams_are_allied(t.team, j as i16))
            .map(|j| usize::try_from(players.teams[j].player).map_or([0; 3], |p| colour(p, 0)))
            .collect();
        lines.push(ScoreLine {
            row,
            player: false,
            active,
            fill: Some(colour(captain, 0)),
            score: Some(score_text(
                c,
                t.score,
                total_score,
                score.numbers_taken,
                score.flags_held,
            )),
            name: name_of(captain, team.players != 1),
            right: percent(t.survivors_cost, total),
            stars,
        });
        row += 1;
        if team.players <= 1 {
            continue;
        }
        for r in &s.player_rankings {
            let Some(p) = usize::try_from(r.player)
                .ok()
                .filter(|&p| p < players.players.len())
            else {
                continue;
            };
            let pl = players.players[p];
            if i16::from(pl.team) != t.team
                || pl.flags & 4 != 0
                || (c.version > game_options::GAMEPLAY_1_8_4 && pl.flags & 0x40 != 0)
            {
                continue;
            }
            lines.push(ScoreLine {
                row,
                player: true,
                active,
                fill: Some(colour(p, 1)),
                score: None,
                name: name_of(p, false),
                right: percent(r.survivors_cost, total),
                stars: Vec::new(),
            });
            row += 1;
        }
    }
    lines
}

pub fn line_rect(frame: Rect, line: &ScoreLine, score_width: i32) -> Rect {
    let (left, width) = if line.player {
        (score_width + PLAYER_LINE_INDENT, PLAYER_LINE_WIDTH)
    } else {
        (MARGIN, score_width + TEAM_LINE_WIDTH)
    };
    let per_column = ((frame.bottom - frame.top - MARGIN) / LINE_PITCH).max(1);
    let x = left + (width + left) * (line.row / per_column) + frame.left;
    let y = (line.row % per_column) * LINE_PITCH + MARGIN + frame.top;
    Rect::new(y, x, y + LINE_HEIGHT, x + width)
}

pub fn score_width(game_type: i16) -> i32 {
    usize::try_from(game_type)
        .ok()
        .and_then(|t| SCORE_COLUMN_WIDTHS.get(t).copied())
        .unwrap_or(0)
}

fn draw_text(bmp: &mut Bitmap32, tags: &PromptTags, rect: Rect, s: &[u8], flags: u32) {
    font::draw_string(
        bmp,
        rect,
        TextClip::None,
        &tags.family,
        font::STYLE_PLAIN,
        s,
        tags.color,
        flags,
        0,
        &tags.locale,
    );
}

fn draw_line_into(bmp: &mut Bitmap32, tags: &PromptTags, line: &ScoreLine, score_width: i32) {
    let mut rect = Rect::new(0, 0, LINE_HEIGHT, bmp.width);
    if let Some(score) = &line.score {
        draw_text(bmp, tags, rect, score, text_flags::MIDDLE);
        rect.left += score_width;
    }
    let outline = pixel(tags.color);
    for &colour in line.stars.iter().rev() {
        draw_star(
            bmp,
            rect.left,
            rect.top,
            rect.bottom - rect.top,
            pixel(colour),
            outline,
        );
        rect.left += rect.bottom - rect.top;
    }
    draw_text(
        bmp,
        tags,
        rect,
        &line.right,
        text_flags::MIDDLE | text_flags::RIGHT,
    );
    rect.right -= font::string_width(&tags.family, font::STYLE_PLAIN, &line.right, &tags.locale)
        + RIGHT_COLUMN_GAP;
    draw_text(bmp, tags, rect, &line.name, text_flags::MIDDLE);
}

fn pixel(c: [u16; 3]) -> u32 {
    0xff00_0000 | u32::from(c[0] >> 8) << 16 | u32::from(c[1] >> 8) << 8 | u32::from(c[2] >> 8)
}

pub fn draw_star(bmp: &mut Bitmap32, left: i32, top: i32, size: i32, fill: u32, outline: u32) {
    let points = STAR_POINTS.map(|(x, y)| (left + size * x / 100, top + size * y / 100));
    fill_polygon(bmp, &points, fill);
    for i in 0..points.len() {
        bresenham(bmp, points[i], points[(i + 1) % points.len()], outline);
    }
}

#[derive(Clone, Copy)]
struct Edge {
    top: i32,
    bottom: i32,
    x: i32,
    step: i32,
}

pub fn fill_polygon(bmp: &mut Bitmap32, points: &[(i32, i32)], color: u32) {
    let n = points.len();
    let mut edges: Vec<Edge> = Vec::new();
    for i in 0..n {
        let (prev, cur) = (points[(i + n - 1) % n], points[i]);
        if prev.1 == cur.1 {
            continue;
        }
        let (top, bottom) = if prev.1 > cur.1 {
            (cur, prev)
        } else {
            (prev, cur)
        };
        let e = Edge {
            top: top.1,
            bottom: bottom.1,
            x: top.0 << 16,
            step: ((bottom.0 - top.0) << 16) / (bottom.1 - top.1),
        };
        let at = edges
            .iter()
            .position(|o| o.top >= e.top)
            .unwrap_or(edges.len());
        edges.insert(at, e);
    }
    let Some(first) = edges.first().map(|e| e.top) else {
        return;
    };
    for e in edges.iter_mut().filter(|e| e.top < 0) {
        e.x += e.step * -e.top;
    }
    let (width, height) = (bmp.width, bmp.height);
    for y in first.max(0)..height {
        let mut active: Vec<usize> = Vec::new();
        for (k, e) in edges.iter().enumerate() {
            if e.top > y {
                break;
            }
            if e.bottom <= y {
                continue;
            }
            let at = active
                .iter()
                .position(|&o| edges[o].x >= e.x)
                .unwrap_or(active.len());
            active.insert(at, k);
        }
        if active.is_empty() {
            return;
        }
        for &[l, r] in active.as_chunks::<2>().0 {
            let from = (edges[l].x >> 16).max(0);
            let to = ((edges[r].x + 0xffff) >> 16).min(width);
            for x in from..to {
                bmp.set(x, y, color);
            }
        }
        for &k in &active {
            edges[k].x += edges[k].step;
        }
    }
}

fn bresenham(bmp: &mut Bitmap32, a: (i32, i32), b: (i32, i32), color: u32) {
    let (top, bottom) = if b.1 < a.1 { (b, a) } else { (a, b) };
    let ((mut x, mut y), (x1, y1)) = (top, bottom);
    let (dx, dy) = (x1 - x, y1 - y);
    let (ax2, ay2) = (dx.abs() * 2, dy.abs() * 2);
    let (sx, sy) = (dx.signum(), dy.signum());
    let mut d = if ay2 < ax2 {
        ay2 - (ax2 >> 1)
    } else {
        ax2 - (ay2 >> 1)
    };
    loop {
        if (0..bmp.width).contains(&x) && (0..bmp.height).contains(&y) {
            bmp.set(x, y, color);
        }
        if ay2 < ax2 {
            if x == x1 {
                return;
            }
            if d >= 0 {
                y += sy;
                d -= ax2;
            }
            x += sx;
            d += ay2;
        } else {
            if y == y1 {
                return;
            }
            if d >= 0 {
                x += sx;
                d -= ay2;
            }
            y += sy;
            d += ax2;
        }
    }
}

pub fn line_rgba(tags: &PromptTags, line: &ScoreLine, score_width: i32) -> (u32, u32, Vec<u8>) {
    let width = if line.player {
        PLAYER_LINE_WIDTH
    } else {
        score_width + TEAM_LINE_WIDTH
    };
    command_bar::coverage_image(width, LINE_HEIGHT, |bmp| {
        draw_line_into(bmp, tags, line, score_width);
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScoreDisplay {
    pub toggled: bool,
}

impl ScoreDisplay {
    pub fn toggle(&mut self) {
        self.toggled = !self.toggled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> ScoreClock {
        ScoreClock {
            tick: 4000,
            blink: 4001,
            state: 0,
            timer: 2300,
            game_time: 21600,
            flags: 0,
            options: 0x200c4012,
            game_type: 4,
            countdown: 5,
            version: 0x73c,
            ..Default::default()
        }
    }

    #[test]
    fn time_strings_match_the_engine() {
        assert_eq!(time_to_string(2300), b"1:16\r");
        assert_eq!(time_to_string(6200), b"3:26\r");
        assert_eq!(time_to_string(0x1a5e0 + 0x708 * 2 + 30 * 5), b"1:02:05\r");
        assert_eq!(time_to_string(0x278d00 + 30), b"1 day 0:01\r");
        assert_eq!(time_to_string(0x278d00 * 2), b"2 days 0:00\r");
    }

    #[test]
    fn when_it_shows() {
        let c = clock();
        assert_eq!(shown(&c, false), Shown::Header);
        assert_eq!(shown(&c, true), Shown::Table);
        assert_eq!(shown(&ScoreClock { tick: 100, ..c }, false), Shown::Table);
        assert_eq!(
            shown(&ScoreClock { countdown: -1, ..c }, false),
            Shown::Nothing
        );
        assert_eq!(
            shown(&ScoreClock { options: 8, ..c }, false),
            Shown::Nothing
        );
        let timed = ScoreClock { state: 2, ..c };
        let at = |timer| shown(&ScoreClock { timer, ..timed }, false);
        assert_eq!(at(21600 - 149), Shown::Header);
        assert_eq!(at(21600 - 150), Shown::Nothing);
        assert_eq!(at(1800 * 5 + 0x95), Shown::Header);
        assert_eq!(at(1800 * 5 + 0x96), Shown::Nothing);
        assert_eq!(at(899), Shown::Header);
        assert_eq!(shown(&ScoreClock { state: 1, ..c }, false), Shown::Nothing);
        assert_eq!(shown(&ScoreClock { flags: 2, ..c }, false), Shown::Table);
    }

    #[test]
    fn score_columns_by_game_type() {
        let c = clock();
        assert_eq!(score_text(&c, 1, 2, 0x100, 0), b"        9");
        assert_eq!(score_text(&c, 1, 2, 0x1, 0), b"1        ");
        let t = |game_type, score| score_text(&ScoreClock { game_type, ..c }, score, 0, 0, 3);
        assert_eq!(t(1, 1), CHECK_MARK);
        assert_eq!(t(1, 0), b"");
        assert_eq!(t(7, 12), b"12");
        assert_eq!(t(9, 1800 * 3 + 30 * 7), b"03:07");
        assert_eq!(t(15, 1800 * 3 + 30 * 7), b"003:07 (x3)");
        assert_eq!(t(0, 0x380), b"4");
        let share = ScoreClock {
            game_type: 0,
            options: OPTION_BODY_COUNT_SHARE,
            ..c
        };
        assert_eq!(score_text(&share, 25, 100, 0, 0), b"25%");
    }

    #[test]
    fn stars_are_drawn_after_the_score() {
        let (bg, fill, edge) = (0xff00_0000, 0xff12_3456, 0xffff_ffff);
        let mut bmp = Bitmap32::new(44, 22, bg);
        draw_star(&mut bmp, 0, 0, 22, fill, edge);
        let at = |b: &Bitmap32, x: i32, y: i32| b.pixels[(y * b.width + x) as usize];
        for (x, y) in STAR_POINTS.map(|(x, y)| (22 * x / 100, 22 * y / 100)) {
            assert_eq!(at(&bmp, x, y), edge, "({x}, {y})");
        }
        assert_eq!(at(&bmp, 11, 11), fill);
        for (x, y) in [(0, 0), (21, 0), (0, 21), (21, 21), (11, 18)] {
            assert_eq!(at(&bmp, x, y), bg, "({x}, {y})");
        }
        assert!((0..22).all(|y| (22..44).all(|x| at(&bmp, x, y) == bg)));
        assert!((0..44).all(|x| at(&bmp, x, 0) == bg && at(&bmp, x, 21) == bg));
    }

    #[test]
    fn team_lines_star_their_allies() {
        use mandrake_sim::players::{Player, Players, Team};
        use mandrake_sim::statistics::{Statistics, TeamRanking};
        let player = |team| Player {
            flags: 0,
            field_09: 0,
            team,
            field_0b: team,
            id: 0,
        };
        let players = Players {
            players: vec![player(0), player(1), player(2)],
            colors: (0..3u16)
                .map(|p| [[p + 1, p + 2, p + 3, 0], [0; 4]])
                .collect(),
            teams: (0..3)
                .map(|t| Team {
                    players: 1,
                    player: t,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut statistics = Statistics::default();
        statistics.team_rankings = (0..3)
            .map(|team| TeamRanking {
                team,
                ..Default::default()
            })
            .collect();
        let mut world = mandrake_sim::engine_rng::EngineWorld::default();
        world.players = players.clone();
        world.game_options = game_options::ALLIANCES;
        world.ally_with_team(2, 1, true);
        world.ally_with_team(2, 0, true);
        world.ally_with_team(0, 2, true);
        let view = StatisticsView {
            statistics: &statistics,
            players: &players,
            game_type: 0,
            game_mode: 1,
            frozen: false,
            local_team: 2,
            difficulty: 0,
            end_code: 0,
            world: &world,
        };
        let lines = score_lines(
            &clock(),
            Shown::Table,
            &view,
            &ScoreStrings::default(),
            &[],
            &world.alliance_view(),
        );
        let stars: Vec<_> = lines.iter().map(|l| l.stars.clone()).collect();
        assert_eq!(
            stars,
            [vec![], vec![[3, 4, 5]], vec![], vec![[1, 2, 3], [2, 3, 4]]]
        );
    }

    #[test]
    fn line_bands() {
        let frame = Rect::new(64, 0, 480, 640);
        let line = |row, player| ScoreLine {
            row,
            player,
            active: true,
            fill: None,
            score: None,
            name: Vec::new(),
            right: Vec::new(),
            stars: Vec::new(),
        };
        assert_eq!(
            line_rect(frame, &line(0, false), 130),
            Rect::new(69, 5, 91, 425)
        );
        assert_eq!(
            line_rect(frame, &line(2, true), 130),
            Rect::new(64 + 5 + 54, 155, 64 + 5 + 54 + 22, 425)
        );
        assert_eq!(
            line_rect(frame, &line(15, false), 130),
            Rect::new(69, 5 + 425, 91, 425 + 425)
        );
    }

    #[test]
    fn score_strings_preserve_mac_roman_bytes() {
        let mut bytes = Vec::new();
        let entries: &[(&[u8; 4], &[u8; 4], &[u8])] = &[
            (b"stli", b"gnen", b"Gonen\xd5s Bridge\r"),
            (b"stli", b"innn", b"King\xd5s Hill\r"),
            (
                b"stli",
                b"inne",
                b"End\xd5s 0\rEnd\xd5s 1\rEnd\xd5s 2\rTime\xd5s Up\rPlan\xd5s\r",
            ),
        ];
        mandrake_tag::SBMonoHeader {
            r#type: 1,
            version: 1,
            name: [0; 32],
            description: [0; 64],
            entry_tag_count: 0,
            tag_list_count: entries.len() as u16,
            checksum: [0; 4],
            flags: 0,
            size: 0,
            header_checksum: [0; 4],
            padding: [0; 4],
            signature: mandrake_tag::SBMonoHeader::SIG_MTH2,
        }
        .write(&mut bytes)
        .unwrap();
        let mut offset =
            mandrake_tag::SBMonoHeader::SIZE + entries.len() * mandrake_tag::SBTagHeader::SIZE;
        for (group, id, data) in entries {
            mandrake_tag::SBTagHeader {
                identifier: 0,
                flags: 0,
                tag_type_idx: 0,
                name: [0; 32],
                tag_type: FourCC::new(**group),
                tag_id: FourCC::new(**id),
                tag_data_offset: offset as i32,
                tag_data_size: data.len() as i32,
                user_data: 0,
                version: 0,
                destination: 0,
                owner_index: 0,
                signature: mandrake_tag::SBTagHeader::SIGNATURE,
            }
            .write(&mut bytes)
            .unwrap();
            offset += data.len();
        }
        for (_, _, data) in entries {
            bytes.extend_from_slice(data);
        }
        let path = std::env::temp_dir().join(format!(
            "mandrake-scores-macroman-{}.gor",
            std::process::id()
        ));
        std::fs::write(&path, &bytes).unwrap();
        let lookup = TagLookup::open(&mandrake_tag::EngineTagFiles {
            paths: vec![path.clone()],
            missing_patches: Vec::new(),
            missing_plugins: Vec::new(),
        });
        let _ = std::fs::remove_file(&path);

        let strings = ScoreStrings::load(&lookup, Some(FourCC(*b"gnen")), false);
        assert_eq!(strings.map_name.as_deref(), Some(&b"Gonen\xd5s Bridge"[..]));

        let solo = ScoreClock {
            state: 1,
            game_type: mandrake_sim::engine_rng::SOLO_GAME_TYPE,
            ..clock()
        };
        assert_eq!(header(&solo, &strings).0, b"Gonen\xd5s Bridge");

        let mp = ScoreClock {
            state: 5,
            options: 0,
            game_type: 0,
            ..clock()
        };
        assert_eq!(
            header(&mp, &strings),
            (b"King\xd5s Hill".to_vec(), b"Time\xd5s Up".to_vec())
        );
    }
}
