use crate::unit_info::string_list;
use mandrake_common::FourCC;
use mandrake_session::{
    EpiloguePage, FilmPages, Graph, GraphLine, PostgameReport, RankRow, ReportSection, SectionKind,
    Text,
};
use mandrake_sim::Simulation;
use mandrake_sim::engine_rng::TagSource;
use mandrake_sim::game_state::{GameEnd, GameState};
use mandrake_sim::statistics::{
    EpilogueReport, RankingLine, StatisticsView, UnitStatistics, format_damage, monster_name,
    monster_spelling, ranking_columns,
};
use mandrake_tag::TagLookup;
use std::io::{self, Write};
use std::path::PathBuf;

pub(crate) const LIVE_FADE_TICKS: i32 = 60;
pub(crate) const FILM_FADE_SECONDS: f64 = 2.0;
const BLACK_WAIT_SECONDS: f64 = 1.0;
const BLACK_HOLD_SECONDS: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndStage {
    Playing,
    Live { ticks_left: i32 },
    FilmFrozen,
    Over,
}

impl EndStage {
    pub(crate) fn of(sim: &Simulation) -> Self {
        match &sim.engine_stop {
            None => Self::of_game(&sim.engine.game),
            Some(_) => Self::Playing,
        }
    }

    fn of_game(game: &GameState) -> Self {
        if game.end().is_none() {
            Self::Playing
        } else if game.game_over {
            Self::Over
        } else if game.carnage_report_open {
            Self::FilmFrozen
        } else {
            Self::Live {
                ticks_left: game.timer,
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct MapEnd {
    alternative_next_level: bool,
    alpha: f32,
    film_frozen_at: Option<f64>,
    black_at: Option<f64>,
    updated_at: f64,
    hold_from: Option<f64>,
    tag_files: Option<(PathBuf, Vec<String>)>,
}

impl MapEnd {
    pub(crate) fn new(alternative_next_level: bool) -> Self {
        Self {
            alternative_next_level,
            tag_files: None,
            alpha: 0.0,
            film_frozen_at: None,
            black_at: None,
            updated_at: 0.0,
            hold_from: None,
        }
    }

    pub(crate) fn with_tag_files(mut self, game_dir: PathBuf, plugins: Vec<String>) -> Self {
        self.tag_files = Some((game_dir, plugins));
        self
    }

    pub(crate) fn open_tags(&self) -> Option<TagLookup> {
        let (game_dir, plugins) = self.tag_files.as_ref()?;
        let plugins: Vec<&str> = plugins.iter().map(String::as_str).collect();
        Some(TagLookup::open(&mandrake_tag::engine_interface_tag_files(
            game_dir, &plugins,
        )))
    }

    pub(crate) fn alternative(&self) -> bool {
        self.alternative_next_level
    }

    pub(crate) fn alpha(&self) -> f32 {
        self.alpha
    }

    pub(crate) fn update(&mut self, stage: EndStage, now: f64) {
        self.alpha = match stage {
            EndStage::Playing => 0.0,
            EndStage::Live { ticks_left } => {
                (LIVE_FADE_TICKS - ticks_left) as f32 / LIVE_FADE_TICKS as f32
            }
            EndStage::FilmFrozen => {
                let start = *self.film_frozen_at.get_or_insert_with(|| {
                    tracing::info!("The film is frozen at its end: fading");
                    now
                });
                ((now - start) / FILM_FADE_SECONDS) as f32
            }
            EndStage::Over => 1.0,
        }
        .clamp(0.0, 1.0);
        self.black_at = (self.alpha >= 1.0).then(|| self.black_at.unwrap_or(now));
        self.hold_from = self.hold_from.filter(|_| self.alpha >= 1.0);
        self.updated_at = now;
    }

    pub(crate) fn frame_presented(&mut self, alpha: f32) {
        if alpha >= 1.0 && self.hold_from.is_none() {
            tracing::info!("Black is on the screen: holding");
            self.hold_from = Some(self.updated_at);
        }
    }

    pub(crate) fn finished(&self, now: f64) -> bool {
        match (self.hold_from, self.black_at) {
            (Some(from), _) => now - from >= BLACK_HOLD_SECONDS,
            (None, Some(at)) => now - at >= BLACK_WAIT_SECONDS,
            (None, None) => false,
        }
    }
}

const MESH_ALTERNATIVE_NEXT_LEVEL_OFFSET: usize = 0xa0;

pub(crate) fn has_alternative_next_level(mesh_data: &[u8]) -> bool {
    mesh_data
        .get(MESH_ALTERNATIVE_NEXT_LEVEL_OFFSET..MESH_ALTERNATIVE_NEXT_LEVEL_OFFSET + 4)
        .is_some_and(|b| b != [0xff; 4])
}

pub(crate) fn result_word(code: i16, alternative_next_level: bool) -> &'static str {
    match code {
        1 => "VICTORY",
        2 if alternative_next_level => "VICTORY",
        _ => "DEFEAT",
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ReportHeader<'a> {
    pub(crate) map: &'a str,
    pub(crate) end: GameEnd,
    pub(crate) alternative_next_level: bool,
    pub(crate) film: bool,
    pub(crate) names: &'a [(Text, Text)],
}

fn write_header(out: &mut impl Write, report: &PostgameReport) -> io::Result<()> {
    writeln!(out, "CARNAGE REPORT")?;
    writeln!(out, "Map: {}", report.map.to_plain_utf8())?;
    writeln!(out, "Result: {}", report.result_word.to_plain_utf8())?;
    writeln!(out, "End tick: {}", report.end_tick)?;
    writeln!(out, "End code: {}", report.end_code)?;
    match report.winner {
        -1 => writeln!(out, "Winner: none"),
        team => writeln!(out, "Winner: team {team}"),
    }
}

#[derive(Clone, Copy)]
enum Arg<'a> {
    Int(i64),
    Float(f64),
    Text(&'a [u8]),
}

fn format_tag_string(format: &[u8], args: &[Arg]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut args = args.iter();
    let mut bytes = format.iter().copied().peekable();
    while let Some(c) = bytes.next() {
        if c != b'%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        while let Some(&d) = bytes.peek() {
            if !(d.is_ascii_digit() || d == b'.') {
                break;
            }
            spec.push(char::from(d));
            bytes.next();
        }
        let conversion = bytes.next();
        if conversion == Some(b'%') {
            out.push(b'%');
            continue;
        }
        let (width, precision) = spec.split_once('.').unwrap_or((&spec, ""));
        let zero = width.starts_with('0');
        let width: usize = width.parse().unwrap_or(0).min(255);
        let text = match (conversion, args.next()) {
            (Some(b'd'), Some(Arg::Int(v))) => v.to_string().into_bytes(),
            (Some(b'f'), Some(Arg::Float(v))) => {
                let prec = precision.parse::<usize>().unwrap_or(6).min(32);
                format!("{v:.prec$}").into_bytes()
            }
            (Some(b's'), Some(Arg::Text(v))) => v.to_vec(),
            _ => Vec::new(),
        };
        let fill = if zero { b'0' } else { b' ' };
        for _ in text.len()..width {
            out.push(fill);
        }
        out.extend_from_slice(&text);
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EpilogueLayout {
    pub(crate) survivor: Option<usize>,
    pub(crate) survivors: usize,
    pub(crate) unit_headers: [usize; 4],
    pub(crate) kills: usize,
    pub(crate) enemy_headers: [usize; 2],
    pub(crate) heroes: usize,
    pub(crate) hero_headers: [usize; 3],
    pub(crate) damage_ratio: usize,
    pub(crate) kill_ratio: usize,
    pub(crate) hero_name: usize,
    pub(crate) damage_share: usize,
    pub(crate) unit_name: usize,
    pub(crate) count: usize,
    pub(crate) casualties: usize,
    pub(crate) no_damage: usize,
    pub(crate) no_casualties: usize,
    pub(crate) title_difficulty: bool,
}

pub(crate) const MYTH2_EPILOGUE: EpilogueLayout = EpilogueLayout {
    survivor: Some(0),
    survivors: 1,
    unit_headers: [2, 3, 4, 5],
    kills: 6,
    enemy_headers: [7, 8],
    heroes: 9,
    hero_headers: [10, 11, 12],
    damage_ratio: 0xd,
    kill_ratio: 0xe,
    hero_name: 0x10,
    damage_share: 0x11,
    unit_name: 0x12,
    count: 0x13,
    casualties: 0x14,
    no_damage: 0x15,
    no_casualties: 0x16,
    title_difficulty: true,
};

pub(crate) const TFL_EPILOGUE: EpilogueLayout = EpilogueLayout {
    survivor: None,
    survivors: 0,
    unit_headers: [6, 2, 3, 4],
    kills: 5,
    enemy_headers: [6, 7],
    heroes: 8,
    hero_headers: [9, 10, 11],
    damage_ratio: 0xc,
    kill_ratio: 0xd,
    hero_name: 0xf,
    damage_share: 0x10,
    unit_name: 0x11,
    count: 0x12,
    casualties: 0x13,
    no_damage: 0x14,
    no_casualties: 0x15,
    title_difficulty: false,
};

const IN02_TIME: usize = 7;

const IN01_NAME: usize = 0;
const IN01_SCORE: usize = 1;
const IN01_KILLS: usize = 2;
const IN01_DAMAGE: usize = 3;
const IN01_LOSSES: usize = 4;
const IN01_PLACES: [usize; 3] = [5, 6, 7];

const POCR_RANK: usize = 4;
const POCR_PERM: usize = 5;
const POCR_FIRST_GRAPH: usize = 6;
const GRAPHS: std::ops::Range<i16> = 0..5;

#[derive(Debug, Default, Clone)]
pub(crate) struct ReportStrings {
    epilogue: Vec<Vec<u8>>,
    rank: Vec<Vec<u8>>,
    perm: Vec<Vec<u8>>,
    difficulty: Vec<Vec<u8>>,
    pages: Vec<Vec<u8>>,
}

fn tag_bytes(list: &[Vec<u8>], index: usize) -> &[u8] {
    list.get(index).map_or(&[], Vec::as_slice)
}

fn tag_string(list: &[Vec<u8>], index: usize) -> String {
    Text(tag_bytes(list, index).to_vec()).to_plain_utf8()
}

impl ReportStrings {
    pub(crate) fn epilogue_only(epilogue: Vec<Vec<u8>>, difficulty: Vec<Vec<u8>>) -> Self {
        Self {
            epilogue,
            difficulty,
            ..Self::default()
        }
    }

    pub(crate) fn load(tags: &dyn TagSource) -> Self {
        let list = |id: &[u8; 4]| {
            tags.tag(*b"stli", FourCC(*id))
                .map_or_else(Vec::new, |d| string_list(&d))
        };
        Self {
            epilogue: list(b"in03"),
            rank: list(b"in02"),
            perm: list(b"in01"),
            difficulty: list(b"indl"),
            pages: tags
                .tag(*b"ditl", FourCC(*b"pocr"))
                .map_or_else(Vec::new, |d| mandrake_tag::parse_ditl(&d)),
        }
    }

    fn report(&self, index: usize) -> Text {
        Text(tag_bytes(&self.epilogue, index).to_vec())
    }

    fn report_format(&self, index: usize, args: &[Arg]) -> Text {
        Text(format_tag_string(tag_bytes(&self.epilogue, index), args))
    }

    fn headers(&self, indices: &[usize]) -> Vec<Text> {
        indices.iter().map(|&i| self.report(i)).collect()
    }

    fn page(&self, index: usize) -> String {
        tag_string(&self.pages, index)
    }
}

fn write_table(out: &mut impl Write, header: &[String], rows: &[Vec<String>]) -> io::Result<()> {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    for row in std::iter::once(header).chain(rows.iter().map(Vec::as_slice)) {
        let mut line = String::new();
        for (i, (cell, width)) in row.iter().zip(&widths).enumerate() {
            let pad = width.saturating_sub(cell.chars().count());
            if i == 0 {
                line.push_str(cell);
                line.extend(std::iter::repeat_n(' ', pad));
            } else {
                line.extend(std::iter::repeat_n(' ', pad + 2));
                line.push_str(cell);
            }
        }
        writeln!(out, "{}", line.trim_end())?;
    }
    Ok(())
}

fn section(out: &mut impl Write, title: &str) -> io::Result<()> {
    writeln!(out, "\n== {title} ==")
}

fn write_disabled_page(out: &mut impl Write, title: &str, because: &str) -> io::Result<()> {
    section(out, title)?;
    writeln!(
        out,
        "The engine disables this page for this game ({because})."
    )
}

fn build_epilogue(
    map: &Text,
    report: &EpilogueReport,
    strings: &ReportStrings,
    tags: &dyn TagSource,
) -> EpiloguePage {
    build_epilogue_with(
        &MYTH2_EPILOGUE,
        map,
        report,
        strings,
        &|mons, count| monster_spelling(tags, mons, count),
        &|mons, index| monster_name(tags, mons, index),
    )
}

pub(crate) fn build_epilogue_with(
    layout: &EpilogueLayout,
    map: &Text,
    report: &EpilogueReport,
    strings: &ReportStrings,
    spelling: &dyn Fn(u32, i16) -> Option<Vec<u8>>,
    name: &dyn Fn(u32, i8) -> Option<Vec<u8>>,
) -> EpiloguePage {
    let l = layout;
    let count = |v: i64| strings.report_format(l.count, &[Arg::Int(v)]);
    let unit_name = |mons, fielded| {
        let name = spelling(mons, fielded).unwrap_or_default();
        strings.report_format(l.unit_name, &[Arg::Text(&name)])
    };
    let difficulty =
        usize::try_from(report.difficulty).map_or(&[][..], |d| tag_bytes(&strings.difficulty, d));
    let title = if l.title_difficulty {
        Text(format_tag_string(
            b"%s (%s)",
            &[Arg::Text(&map.0), Arg::Text(difficulty)],
        ))
    } else {
        map.clone()
    };
    let mut sections = Vec::new();
    if !report.units.is_empty() {
        let headline = match l.survivor {
            Some(one) if report.survivors == 1 => one,
            _ => l.survivors,
        };
        sections.push(ReportSection {
            kind: SectionKind::Survivors,
            heading: strings.report_format(headline, &[Arg::Int(i64::from(report.survivors))]),
            columns: strings.headers(&l.unit_headers),
            rows: report
                .units
                .iter()
                .map(|u| {
                    vec![
                        unit_name(u.mons, u.fielded),
                        strings.report_format(l.damage_share, &[Arg::Float(u.damage_share)]),
                        count(i64::from(u.kills)),
                        strings.report_format(
                            l.casualties,
                            &[Arg::Int(i64::from(u.dead)), Arg::Int(i64::from(u.fielded))],
                        ),
                    ]
                })
                .collect(),
        });
    }
    if !report.heroes.is_empty() {
        sections.push(ReportSection {
            kind: SectionKind::Heroes,
            heading: strings.report(l.heroes),
            columns: strings.headers(&l.hero_headers),
            rows: report
                .heroes
                .iter()
                .map(|h| {
                    let name = if h.custom_name.is_empty() {
                        name(h.mons, h.name_index).unwrap_or_default()
                    } else {
                        Text::from_plain_utf8(&h.custom_name).0
                    };
                    let spelling = spelling(h.mons, 1).unwrap_or_default();
                    vec![
                        strings
                            .report_format(l.hero_name, &[Arg::Text(&name), Arg::Text(&spelling)]),
                        count(i64::from(h.battles)),
                        count(i64::from(h.experience)),
                    ]
                })
                .collect(),
        });
    }
    if !report.enemies.is_empty() {
        sections.push(ReportSection {
            kind: SectionKind::Kills,
            heading: strings.report_format(l.kills, &[Arg::Int(i64::from(report.kills))]),
            columns: strings.headers(&l.enemy_headers),
            rows: report
                .enemies
                .iter()
                .map(|e| vec![unit_name(e.mons, e.fielded), count(i64::from(e.dead))])
                .collect(),
        });
    }
    let line = |kind, heading| ReportSection {
        kind,
        heading,
        columns: Vec::new(),
        rows: Vec::new(),
    };
    sections.push(line(
        SectionKind::DamageRatio,
        report.damage_ratio.map_or_else(
            || strings.report(l.no_damage),
            |x| strings.report_format(l.damage_ratio, &[Arg::Float(x)]),
        ),
    ));
    sections.push(line(
        SectionKind::KillRatio {
            no_casualties: report.kill_ratio.is_none(),
        },
        report.kill_ratio.map_or_else(
            || strings.report(l.no_casualties),
            |x| strings.report_format(l.kill_ratio, &[Arg::Float(x)]),
        ),
    ));
    EpiloguePage { title, sections }
}

pub(crate) fn build_report(
    view: Option<&StatisticsView>,
    header: &ReportHeader,
    strings: &ReportStrings,
    tags: &dyn TagSource,
) -> PostgameReport {
    let map = Text::from_plain_utf8(header.map);
    let end = &header.end;
    let epilogue = view
        .and_then(StatisticsView::epilogue)
        .map(|report| build_epilogue(&map, &report, strings, tags));
    let pages = view.map(|view| film_pages(view, strings, header.names));
    PostgameReport {
        map,
        result_word: Text::from_plain_utf8(result_word(end.code, header.alternative_next_level)),
        end_tick: end.tick,
        end_code: end.code,
        winner: end.winner,
        film: header.film,
        epilogue,
        pages,
    }
}

fn player_name(names: &[(Text, Text)], player: i16, team: bool) -> Text {
    usize::try_from(player)
        .ok()
        .and_then(|p| names.get(p))
        .map(|(n, t)| if team { t.clone() } else { n.clone() })
        .unwrap_or_default()
}

fn team_player(view: &StatisticsView, team: i16) -> i16 {
    usize::try_from(team)
        .ok()
        .and_then(|t| view.players.teams.get(t))
        .map_or(-1, |t| t.player)
}

fn printed_name(name: Text, fallback: String) -> String {
    if name.0.is_empty() {
        fallback
    } else {
        name.to_plain_utf8()
    }
}

fn film_pages(view: &StatisticsView, strings: &ReportStrings, names: &[(Text, Text)]) -> FilmPages {
    let players = view.players;
    let color = |player: i16, which: usize| -> [u16; 3] {
        usize::try_from(player)
            .ok()
            .and_then(|p| players.colors.get(p))
            .map_or([0; 3], |c| [c[which][0], c[which][1], c[which][2]])
    };
    let name = |player: i16, team: bool| player_name(names, player, team);
    let team_player = |team: i16| team_player(view, team);
    let columns = ranking_columns(view.game_type);
    let first = ranking_first_column(strings, view.game_type);
    let rank = view
        .ranking_lines()
        .iter()
        .map(|line| {
            let (row_name, team, numbers, fill, is_team, in_team) = match *line {
                RankingLine::Team(t) => {
                    let p = team_player(t.team);
                    (
                        name(p, true),
                        t,
                        (t.survivors, t.kills, t.damage_dealt, t.losses),
                        color(p, 0),
                        true,
                        false,
                    )
                }
                RankingLine::Player {
                    ranking: r,
                    team,
                    in_team,
                } => (
                    name(r.player, false),
                    team,
                    (r.survivors, r.kills, r.damage_dealt, r.losses),
                    color(r.player, usize::from(in_team)),
                    false,
                    in_team,
                ),
            };
            let score = usize::try_from(team.team)
                .ok()
                .and_then(|t| view.statistics.team_scores.get(t).copied())
                .unwrap_or_default();
            let cells = (0u8..8)
                .map(|bit| {
                    if columns & (1 << bit) == 0 {
                        return Text::default();
                    }
                    let s = match bit {
                        0 if first.is_some() => team.score.to_string(),
                        0 => String::new(),
                        1 => return row_name.clone(),
                        2 => ranking_time(strings, score.field_0c),
                        3 => ranking_time(strings, score.flag_time),
                        4 => numbers.0.to_string(),
                        5 => numbers.1.to_string(),
                        6 => format_damage(numbers.2).to_string(),
                        _ => numbers.3.to_string(),
                    };
                    Text::from_plain_utf8(&s)
                })
                .collect();
            RankRow {
                color: fill,
                team: is_team,
                in_team,
                cells,
            }
        })
        .collect();
    let perm = view.stats_page_valid(1).then(|| {
        view.permanent_lines()
            .iter()
            .map(|r| {
                let score = r.points as f32 * 1.525_878_9e-5;
                let mut cells = vec![
                    Text::from_plain_utf8(&format!("{score:2.2}")),
                    name(r.player, false),
                ];
                cells.extend(
                    r.places
                        .iter()
                        .map(|p| Text::from_plain_utf8(&p.to_string())),
                );
                cells.push(Text::from_plain_utf8(&r.kills.to_string()));
                cells.push(Text::from_plain_utf8(
                    &format_damage(r.damage_dealt).to_string(),
                ));
                cells.push(Text::from_plain_utf8(&r.losses.to_string()));
                RankRow {
                    color: color(r.player, 0),
                    team: false,
                    in_team: false,
                    cells,
                }
            })
            .collect()
    });
    let range = view.statistics.sample_range();
    let graphs = GRAPHS
        .map(|graph| {
            view.graph_valid(graph).then(|| Graph {
                count: range.count,
                period: range.period,
                tick: range.tick,
                lines: (0..players.teams.len() as i16)
                    .map(|team| GraphLine {
                        color: color(team_player(team), 0),
                        values: (0..range.count.max(0) as usize)
                            .map(|sample| view.graph_value(graph, sample, team))
                            .collect(),
                    })
                    .collect(),
            })
        })
        .collect();
    FilmPages {
        game_type: view.game_type,
        rank_columns: columns,
        rank,
        perm,
        graphs,
    }
}

fn plain(cells: &[Text]) -> Vec<String> {
    cells.iter().map(Text::to_plain_utf8).collect()
}

pub(crate) fn write_epilogue(out: &mut impl Write, page: &EpiloguePage) -> io::Result<()> {
    writeln!(out, "{}", page.title.to_plain_utf8())?;
    for section in &page.sections {
        if section.kind
            != (SectionKind::KillRatio {
                no_casualties: false,
            })
        {
            writeln!(out)?;
        }
        writeln!(out, "{}", section.heading.to_plain_utf8())?;
        if !section.columns.is_empty() {
            let rows: Vec<Vec<String>> = section.rows.iter().map(|row| plain(row)).collect();
            write_table(out, &plain(&section.columns), &rows)?;
        }
    }
    Ok(())
}

fn ranking_time(strings: &ReportStrings, ticks: i32) -> String {
    Text(format_tag_string(
        tag_bytes(&strings.rank, IN02_TIME),
        &[
            Arg::Int(i64::from(ticks / 0x708)),
            Arg::Int(i64::from(ticks % 0x708 / 0x1e)),
        ],
    ))
    .to_plain_utf8()
}

fn ranking_first_column(strings: &ReportStrings, game_type: i16) -> Option<String> {
    let index = match game_type {
        0 => 5,
        3 => 10,
        4 => 9,
        9 | 14 | 15 => 8,
        _ => return None,
    };
    Some(tag_string(&strings.rank, index))
}

fn write_rank_page(
    out: &mut impl Write,
    view: &StatisticsView,
    strings: &ReportStrings,
    names: &[(Text, Text)],
) -> io::Result<()> {
    let columns = ranking_columns(view.game_type);
    let first = ranking_first_column(strings, view.game_type);
    let has = |bit: u8| columns & (1 << bit) != 0;
    let mut header = Vec::new();
    if has(0)
        && let Some(title) = &first
    {
        header.push(title.clone());
    }
    for bit in 1u8..8 {
        if has(bit) {
            header.push(tag_string(&strings.rank, usize::from(bit) - 1));
        }
    }
    let rows: Vec<Vec<String>> = view
        .ranking_lines()
        .iter()
        .map(|line| {
            let (name, team, numbers) = match *line {
                RankingLine::Team(t) => (
                    printed_name(
                        player_name(names, team_player(view, t.team), true),
                        format!("Team {}", t.team),
                    ),
                    t,
                    (t.survivors, t.kills, t.damage_dealt, t.losses),
                ),
                RankingLine::Player {
                    ranking: r, team, ..
                } => (
                    printed_name(
                        player_name(names, r.player, false),
                        format!("Player {}", r.player),
                    ),
                    team,
                    (r.survivors, r.kills, r.damage_dealt, r.losses),
                ),
            };
            let score = usize::try_from(team.team)
                .ok()
                .and_then(|t| view.statistics.team_scores.get(t).copied())
                .unwrap_or_default();
            let mut row = Vec::new();
            if has(0) && first.is_some() {
                row.push(team.score.to_string());
            }
            for bit in 1u8..8 {
                if !has(bit) {
                    continue;
                }
                row.push(match bit {
                    1 => name.clone(),
                    2 => ranking_time(strings, score.field_0c),
                    3 => ranking_time(strings, score.flag_time),
                    4 => numbers.0.to_string(),
                    5 => numbers.1.to_string(),
                    6 => format_damage(numbers.2).to_string(),
                    _ => numbers.3.to_string(),
                });
            }
            row
        })
        .collect();
    section(out, &strings.page(POCR_RANK))?;
    write_table(out, &header, &rows)
}

fn write_perm_page(
    out: &mut impl Write,
    view: &StatisticsView,
    strings: &ReportStrings,
    names: &[(Text, Text)],
) -> io::Result<()> {
    if !view.stats_page_valid(1) {
        let because = format!("game mode {}", view.game_mode);
        return write_disabled_page(out, &strings.page(POCR_PERM), &because);
    }
    section(out, &strings.page(POCR_PERM))?;
    let header: Vec<String> = [IN01_NAME, IN01_SCORE]
        .iter()
        .chain(&IN01_PLACES)
        .chain(&[IN01_KILLS, IN01_DAMAGE, IN01_LOSSES])
        .map(|&i| tag_string(&strings.perm, i))
        .collect();
    let rows: Vec<Vec<String>> = view
        .permanent_lines()
        .iter()
        .map(|r| {
            let score = r.points as f32 * 1.525_878_9e-5;
            let name = printed_name(
                player_name(names, r.player, false),
                format!("Player {}", r.player),
            );
            let mut row = vec![name, format!("{score:.2}")];
            row.extend(r.places.iter().map(i16::to_string));
            row.push(r.kills.to_string());
            row.push(format_damage(r.damage_dealt).to_string());
            row.push(r.losses.to_string());
            row
        })
        .collect();
    write_table(out, &header, &rows)
}

fn labels(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

fn cells<T: ToString, const N: usize>(values: [T; N]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn tag_text(tag: u32) -> String {
    let mut bytes = Vec::with_capacity(4);
    for b in tag.to_be_bytes() {
        if b == b'|' {
            bytes.push(b'|');
        }
        bytes.push(b);
    }
    Text(bytes).to_plain_utf8()
}

fn write_totals(
    out: &mut impl Write,
    view: &StatisticsView,
    tags: &dyn TagSource,
) -> io::Result<()> {
    const RANKING: [&str; 8] = [
        "Losses",
        "Losses cost",
        "Surv.",
        "Surv. cost",
        "Uncontrollable",
        "Kills",
        "Damage dealt",
        "Damage taken",
    ];
    let s = view.statistics;
    section(out, "Block")?;
    write_table(
        out,
        &labels(&[
            "Sample period",
            "Sample tick",
            "Sample",
            "Countdown",
            "Rebuilds",
            "Finished",
            "Flagged monster",
            "Custom scoring",
        ]),
        &[cells([
            i32::from(s.sample_period),
            i32::from(s.sample_tick),
            i32::from(s.sample),
            i32::from(s.countdown),
            i32::from(s.rebuilds),
            i32::from(s.finished),
            i32::from(s.seen_flagged_monster),
            i32::from(s.custom_scoring),
        ])],
    )?;

    section(out, "Team rankings")?;
    let mut header = labels(&["Team", "Rank"]);
    header.extend(labels(&RANKING));
    header.push("Score".to_string());
    let rows: Vec<Vec<String>> = s
        .team_rankings
        .iter()
        .map(|t| {
            cells([
                i32::from(t.team),
                i32::from(t.rank),
                t.losses,
                t.losses_cost,
                t.survivors,
                t.survivors_cost,
                t.uncontrollable,
                t.kills,
                t.damage_dealt,
                t.damage_taken,
                t.score,
            ])
        })
        .collect();
    write_table(out, &header, &rows)?;

    section(out, "Player rankings")?;
    let mut header = labels(&["Player", "Parent"]);
    header.extend(labels(&RANKING));
    let rows: Vec<Vec<String>> = s
        .player_rankings
        .iter()
        .map(|r| {
            cells([
                i32::from(r.player),
                i32::from(r.parent),
                r.losses,
                r.losses_cost,
                r.survivors,
                r.survivors_cost,
                r.uncontrollable,
                r.kills,
                r.damage_dealt,
                r.damage_taken,
            ])
        })
        .collect();
    write_table(out, &header, &rows)?;

    section(out, "Permanent rankings")?;
    let rows: Vec<Vec<String>> = s
        .permanent_rankings
        .iter()
        .map(|r| {
            cells([
                i32::from(r.player),
                i32::from(r.id),
                r.points,
                r.losses,
                r.kills,
                r.damage_dealt,
                i32::from(r.places[0]),
                i32::from(r.places[1]),
                i32::from(r.places[2]),
                i32::from(r.parent),
            ])
        })
        .collect();
    write_table(
        out,
        &labels(&[
            "Player",
            "Id",
            "Points",
            "Losses",
            "Kills",
            "Damage dealt",
            "First",
            "Second",
            "Third",
            "Parent",
        ]),
        &rows,
    )?;

    section(out, "Player totals")?;
    let rows: Vec<Vec<String>> = s
        .totals
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| cells([i as i32, p.own_damage, p.score]))
        .collect();
    write_table(out, &labels(&["Player", "Own damage", "Score"]), &rows)?;
    writeln!(out, "No player's score: {}", s.totals.no_player_score)?;

    section(out, "Team totals")?;
    let rows: Vec<Vec<String>> = s
        .totals
        .teams
        .iter()
        .enumerate()
        .map(|(i, t)| cells([i as i32, t.kills, t.damage, t.killed_by_local_team]))
        .collect();
    write_table(
        out,
        &labels(&["Team", "Kills", "Damage", "Killed by local team"]),
        &rows,
    )?;

    section(out, "Team data")?;
    let rows: Vec<Vec<String>> = s
        .team_scores
        .iter()
        .enumerate()
        .map(|(i, t)| {
            cells([
                i as i32,
                t.field_04,
                t.flag_time,
                t.field_0c,
                t.score_tick,
                t.best_tick,
                t.level_tick,
                i32::from(t.taken_count),
                i32::from(t.flags_held),
                i32::from(t.best),
                i32::from(t.score),
            ])
        })
        .collect();
    write_table(
        out,
        &labels(&[
            "Team",
            "+0x04",
            "Flag time",
            "+0x0c",
            "Score tick",
            "Best tick",
            "Level tick",
            "+0x20",
            "Flags held",
            "Best",
            "Score",
        ]),
        &rows,
    )?;

    section(out, "Unit totals")?;
    let mut header = labels(&["Name", "Row", "Tag", "Owner", "Total", "Hit points"]);
    header.extend(labels(&[
        "Alive",
        "Hit points and damage",
        "Uncontrollable",
        "Kills",
        "Damage dealt",
        "Damage taken",
    ]));
    let rows: Vec<Vec<String>> = s
        .units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.totals != UnitStatistics::default())
        .map(|(i, u)| {
            let t = u.totals;
            let count = t.total.wrapping_add(t.uncontrollable);
            let name = Text(monster_spelling(tags, u.mons_tag, count as i16).unwrap_or_default())
                .to_plain_utf8();
            let mut row = vec![name, i.to_string(), tag_text(u.tag), u.owner.to_string()];
            row.extend(cells([
                t.total,
                t.total_hit_points,
                t.alive,
                t.hit_points_and_damage,
                t.uncontrollable,
                t.kills,
                t.damage_dealt,
                t.damage_taken,
            ]));
            row
        })
        .collect();
    write_table(out, &header, &rows)
}

fn graph_note(graph: i16) -> &'static str {
    match graph {
        0 => "The score of the team's players.",
        1 => "The team's kills of other teams' units.",
        2 => "The team's damage to other teams' units, scaled, 256 to one Dmg.",
        _ => "The team's units killed by the local team.",
    }
}

fn write_graph_table(out: &mut impl Write, view: &StatisticsView, graph: i16) -> io::Result<()> {
    let range = view.statistics.sample_range();
    writeln!(out, "{}", graph_note(graph))?;
    writeln!(
        out,
        "{} ticks per sample, {} ticks into the last.",
        range.period, range.tick
    )?;
    let teams: Vec<_> = (0..view.players.teams.len() as i16)
        .map(|t| view.graph_points(graph, t))
        .collect();
    let mut header = vec!["Sample".to_string(), "Tick".to_string()];
    for t in 0..teams.len() {
        header.push(format!("Team {t}"));
        header.push(format!("Team {t} sum"));
    }
    let rows: Vec<Vec<String>> = (0..range.count.max(0) as usize)
        .map(|i| {
            let mut row = vec![(i + 1).to_string(), (range.period * i as i32).to_string()];
            for points in &teams {
                row.push(points[i].value.to_string());
                row.push(points[i].running.to_string());
            }
            row
        })
        .collect();
    write_table(out, &header, &rows)
}

fn write_graph_pages(
    out: &mut impl Write,
    view: &StatisticsView,
    strings: &ReportStrings,
) -> io::Result<()> {
    for graph in GRAPHS {
        let title = strings.page(POCR_FIRST_GRAPH + graph as usize);
        if view.graph_valid(graph) {
            section(out, &title)?;
            write_graph_table(out, view, graph)?;
        } else {
            write_disabled_page(out, &title, &format!("game type {}", view.game_type))?;
        }
    }
    Ok(())
}

pub(crate) fn print_report(
    out: &mut impl Write,
    report: &PostgameReport,
    view: Option<&StatisticsView>,
    strings: &ReportStrings,
    tags: &dyn TagSource,
    names: &[(Text, Text)],
) -> io::Result<()> {
    write_header(out, report)?;
    let Some(view) = view else {
        return Ok(());
    };
    let write_epilogue_page = |out: &mut _| -> io::Result<()> {
        let Some(page) = &report.epilogue else {
            return Ok(());
        };
        section(out, "Epilogue")?;
        write_epilogue(out, page)
    };
    if !report.film {
        write_epilogue_page(out)?;
    }
    write_rank_page(out, view, strings, names)?;
    write_perm_page(out, view, strings, names)?;
    if report.film {
        write_epilogue_page(out)?;
    }
    write_totals(out, view, tags)?;
    write_graph_pages(out, view, strings)
}

pub(crate) fn print(
    sim: &Simulation,
    map: &str,
    end: GameEnd,
    alternative_next_level: bool,
    lookup: Option<&TagLookup>,
    names: &[(Text, Text)],
) -> (PostgameReport, io::Result<()>) {
    let tags = |group: [u8; 4], id: FourCC| lookup.and_then(|l| l.get(FourCC(group), id));
    let header = ReportHeader {
        map,
        end,
        alternative_next_level,
        film: (sim.engine.game.carnage_report_open),
        names,
    };
    let strings = ReportStrings::load(&tags);
    let view = sim.statistics();
    let report = build_report(view.as_ref(), &header, &strings, &tags);
    #[cfg(not(target_arch = "wasm32"))]
    let mut out = io::stdout().lock();
    #[cfg(target_arch = "wasm32")]
    let mut out = crate::web::ConsoleLine(Vec::new());
    let printed = print_report(&mut out, &report, view.as_ref(), &strings, &tags, names)
        .and_then(|()| out.flush());
    (report, printed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_sim::game_state::GameState;
    use mandrake_sim::statistics::{EpilogueEnemy, EpilogueHero, EpilogueUnit};

    fn ended(edi: i32) -> GameState {
        let mut game = GameState::new(0, 0);
        game.winner = 0;
        game.end_game(edi, 100, &mut 0).unwrap();
        game
    }

    #[test]
    fn a_game_not_ended_has_no_fade() {
        let mut end = MapEnd::new(false);
        end.update(EndStage::of_game(&GameState::new(0, 0)), 5.0);
        assert_eq!(end.alpha(), 0.0);
        assert!(!end.finished(1000.0));
    }

    #[test]
    fn a_live_game_fades_over_the_last_60_of_its_300_ticks_to_black_at_game_over() {
        let mut game = ended(0);
        let mut end = MapEnd::new(false);
        let mut tick = 100;
        let mut ticks_to_over = 0;
        let mut first_dark = None;
        while !game.game_over {
            end.update(EndStage::of_game(&game), 0.0);
            if end.alpha() > 0.0 && first_dark.is_none() {
                first_dark = Some(ticks_to_over);
                assert!((end.alpha() - 1.0 / 60.0).abs() < 1e-6, "{}", end.alpha());
            }
            assert!(end.alpha() < 1.0, "black before the game is over");
            assert!(!end.finished(100.0));
            tick += 1;
            game.end_of_tick_body(tick, &mut 0).1.unwrap();
            ticks_to_over += 1;
        }
        assert_eq!(ticks_to_over, 300);
        assert_eq!(first_dark, Some(241), "59 ticks left, a 60th of the way");
        end.update(EndStage::of_game(&game), 0.0);
        assert_eq!(end.alpha(), 1.0);
    }

    #[test]
    fn a_live_fade_is_linear_and_halfway_at_30_ticks_left() {
        let mut end = MapEnd::new(false);
        for (ticks_left, alpha) in [(300, 0.0), (61, 0.0), (60, 0.0), (30, 0.5), (15, 0.75)] {
            end.update(EndStage::Live { ticks_left }, 0.0);
            assert_eq!(end.alpha(), alpha, "{ticks_left}");
        }
    }

    #[test]
    fn a_short_timer_still_ends_on_black() {
        let mut game = ended(1);
        assert_eq!(EndStage::of_game(&game), EndStage::Live { ticks_left: 1 });
        game.end_of_tick_body(101, &mut 0).1.unwrap();
        assert_eq!(EndStage::of_game(&game), EndStage::Over);
        let mut end = MapEnd::new(false);
        end.update(EndStage::of_game(&game), 0.0);
        end.frame_presented(end.alpha());
        assert!(end.finished(BLACK_HOLD_SECONDS));
    }

    #[test]
    fn it_prints_when_black_has_been_held_for_2_seconds_and_not_before() {
        let mut end = MapEnd::new(false);
        end.update(EndStage::FilmFrozen, 0.0);
        end.frame_presented(end.alpha());
        end.update(EndStage::FilmFrozen, 1.9);
        end.frame_presented(end.alpha());
        assert!(!end.finished(1.9));
        end.update(EndStage::FilmFrozen, 2.0);
        assert!(!end.finished(2.0), "black is drawn but not on screen");
        end.update(EndStage::FilmFrozen, 2.5);
        end.frame_presented(end.alpha());
        assert!(!end.finished(4.5 - 0.01), "the hold runs from the frame");
        assert!(end.finished(4.5));
    }

    #[test]
    fn the_hold_is_2_seconds_of_wall_clock_with_black_frames_all_the_way() {
        for frame in [1.0 / 240.0, 1.0 / 30.0, 0.5] {
            let mut end = MapEnd::new(false);
            let mut now = 7.0;
            while now < 9.0 - 1e-9 {
                end.update(EndStage::Over, now);
                assert_eq!(end.alpha(), 1.0);
                end.frame_presented(end.alpha());
                now += frame;
                assert!(!end.finished(now.min(9.0 - 1e-6)), "{frame} at {now}");
            }
            assert!(end.finished(9.0), "{frame}");
        }
    }

    #[test]
    fn a_window_hidden_during_the_hold_still_ends() {
        let mut end = MapEnd::new(false);
        end.update(EndStage::Over, 7.0);
        end.frame_presented(end.alpha());
        assert!(
            !end.finished(8.5),
            "the wait for a frame is over: the hold isn't"
        );
        assert!(end.finished(9.0));
    }

    #[test]
    fn a_window_that_presents_nothing_ends_a_moment_after_black() {
        let mut end = MapEnd::new(false);
        end.update(EndStage::Over, 7.0);
        assert!(!end.finished(7.5));
        end.update(EndStage::Over, 8.0);
        assert!(end.finished(8.0));
    }

    #[test]
    fn a_stop_after_the_end_prints_nothing() {
        let mut end = MapEnd::new(false);
        end.update(EndStage::Live { ticks_left: 30 }, 0.0);
        end.update(EndStage::Playing, 1.0);
        assert_eq!(end.alpha(), 0.0);
        assert!(!end.finished(100.0));
        let mut end = MapEnd::new(false);
        end.update(EndStage::Over, 0.0);
        end.frame_presented(end.alpha());
        end.update(EndStage::Playing, 1.0);
        assert_eq!(end.alpha(), 0.0);
        assert!(!end.finished(100.0));
    }

    #[test]
    fn the_result_is_the_engines_rule() {
        assert_eq!(result_word(1, false), "VICTORY");
        assert_eq!(result_word(1, true), "VICTORY");
        assert_eq!(result_word(2, true), "VICTORY");
        assert_eq!(result_word(2, false), "DEFEAT");
        assert_eq!(result_word(3, false), "DEFEAT");
        assert_eq!(result_word(3, true), "DEFEAT");
    }

    #[test]
    fn the_alternative_next_level_is_header_a0() {
        let mut mesh = vec![0xff; 0x100];
        assert!(!has_alternative_next_level(&mesh));
        mesh[0xa0..0xa4].copy_from_slice(b"l02s");
        assert!(has_alternative_next_level(&mesh));
        assert!(!has_alternative_next_level(&mesh[..0xa2]));
    }

    fn report(end: GameEnd, alternative: bool) -> String {
        let mut out = Vec::new();
        let header = ReportHeader {
            map: "Salvation",
            end,
            alternative_next_level: alternative,
            film: false,
            names: &[],
        };
        let model = build_report(None, &header, &ReportStrings::default(), &fake_tags);
        write_header(&mut out, &model).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_header_has_the_map_the_result_and_the_end_tick() {
        let end = GameEnd {
            tick: 25440,
            winner: 0,
            code: 1,
        };
        assert_eq!(
            report(end, false),
            "CARNAGE REPORT\nMap: Salvation\nResult: VICTORY\nEnd tick: 25440\nEnd code: 1\n\
             Winner: team 0\n"
        );
    }

    #[test]
    fn code_2_follows_the_mesh() {
        let end = GameEnd {
            tick: 5,
            winner: 0,
            code: 2,
        };
        assert!(report(end, true).contains("Result: VICTORY\n"));
        assert!(report(end, false).contains("Result: DEFEAT\n"));
    }

    #[test]
    fn an_end_game_command_prints_the_engines_data() {
        let mut game = GameState::new(0, 0);
        game.command_reason = 2;
        game.end_game(1, 40, &mut 0).unwrap();
        let text = report(game.end().unwrap(), false);
        assert_eq!(
            text,
            "CARNAGE REPORT\nMap: Salvation\nResult: DEFEAT\nEnd tick: 40\nEnd code: 3\n\
             Winner: none\n"
        );
    }

    #[test]
    fn the_screen_fade_darkens_the_whole_target() {
        let Some((device, queue)) = crate::tests::test_gpu() else {
            eprintln!("Notice: no GPU adapter, skipping");
            return;
        };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let fade = mandrake_render::ScreenFade::new(&device, format);
        let pixels_after = |alpha: f32| -> Vec<u8> {
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Fade Test Target"),
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Fade Test Readback"),
                size: 256 * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 0.6,
                            b: 0.2,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            fade.encode(&mut encoder, &view, alpha);
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(4),
                    },
                },
                wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(std::iter::once(encoder.finish()));
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = read.slice(..).get_mapped_range().unwrap().to_vec();
            bytes
                .chunks(256)
                .flat_map(|row| row[..16].to_vec())
                .collect()
        };
        for (alpha, rgb) in [
            (0.0f32, [255, 153, 51]),
            (0.5, [128, 77, 26]),
            (1.0, [0, 0, 0]),
        ] {
            let pixels = pixels_after(alpha);
            assert_eq!(pixels.len(), 64);
            for px in pixels.chunks(4) {
                for (got, want) in px[..3].iter().zip(rgb) {
                    assert!((i32::from(*got) - want).abs() <= 1, "{alpha}: {px:?}");
                }
                assert_eq!(px[3], 255, "the frame stays opaque");
            }
        }
    }

    const IN03: &[u8] = b"%d Survivor\r%d Survivors\r|bUnit|p\r|bDamage Inflicted|p\r|bKills|p\r\
        |bCasualties|p\r%d Kills\r|bUnit|p\r|bDead|p\rHeroes\r|bName|p\r|bBattles Survived|p\r\
        |bKills|p\rDamage Ratio %.2f:1\rKill Ratio %.2f:1\rTime Elapsed %02d:%02d\r%s |i(%s)|p\r\
        %.1f%%\r|i%s|p\r%d\r%d/%d\r|iNo Damage Taken|p\r|iNo Casualties!|p";
    const IN02: &[u8] =
        b"Name\rCaptured\rHeld\rSurv.\rKills\rDmg\rLosses\r%d:%02d\rTime\rFlags\rBalls";
    const IN01: &[u8] = b"Name\rScore\rKills\rDmg\rLosses\r1st\r2nd\r3rd\r4th\r5th";
    const INDL: &[u8] = b"Timid\rSimple\rNormal\rHeroic\rLegendary";
    const POCR: &[u8] = b"OK\r\r\r\rRank\rPerm\rScore\rKills\rDmg\rUnits\rKills\r";

    fn fake_tags(group: [u8; 4], id: FourCC) -> Option<Vec<u8>> {
        let key = id.0;
        match &group {
            b"mons" => {
                let mut tag = vec![0; 0x1b0];
                tag[0x1a8..0x1ac].copy_from_slice(&[b's', key[0], key[1], key[2]]);
                tag[0x1ac..0x1b0].copy_from_slice(&[b'n', key[0], key[1], key[2]]);
                Some(tag)
            }
            b"stli" => {
                let list: &[u8] = match &key {
                    b"in03" => IN03,
                    b"in02" => IN02,
                    b"in01" => IN01,
                    b"indl" => INDL,
                    b"sbow" => b"Bowman\rBowmen",
                    b"nbow" => b"Simone\rPiers\rPetrarch",
                    b"swar" => b"Warrior\rWarriors",
                    b"sdwa" => b"Dwarf\rDwarves",
                    b"ndwa" => b"Nori",
                    b"sbri" => b"Brigand\rBrigands",
                    b"sgha" => b"Ghast\rGhasts",
                    _ => return None,
                };
                Some(list.to_vec())
            }
            b"ditl" => (key == *b"pocr").then(|| POCR.to_vec()),
            _ => None,
        }
    }

    fn mons(id: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*id)
    }

    fn epilogue_text(report: &EpilogueReport) -> String {
        let mut out = Vec::new();
        let strings = ReportStrings::load(&fake_tags);
        let map = Text::from_plain_utf8("Salvation");
        let page = build_epilogue(&map, report, &strings, &fake_tags);
        write_epilogue(&mut out, &page).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn unit(id: &[u8; 4], damage_share: f64, kills: i16, dead: i16, fielded: i16) -> EpilogueUnit {
        EpilogueUnit {
            mons: mons(id),
            damage_share,
            kills,
            dead,
            fielded,
        }
    }

    fn enemy(id: &[u8; 4], dead: i16) -> EpilogueEnemy {
        EpilogueEnemy {
            mons: mons(id),
            dead,
            fielded: dead,
        }
    }

    fn screenshot_page(kill_ratio: Option<f64>) -> EpiloguePage {
        let strings = ReportStrings::load(&fake_tags);
        let report = EpilogueReport {
            difficulty: 3,
            survivors: 1,
            units: vec![unit(b"bowm", 34.3, 14, 6, 6)],
            heroes: vec![EpilogueHero {
                mons: mons(b"dwar"),
                name_index: 0,
                custom_name: String::new(),
                battles: 1,
                experience: 20,
            }],
            kills: 36,
            enemies: vec![enemy(b"brig", 26)],
            damage_ratio: Some(1.78),
            kill_ratio,
        };
        let map = Text::from_plain_utf8("Salvation");
        build_epilogue(&map, &report, &strings, &fake_tags)
    }

    #[test]
    fn a_heros_own_name_replaces_the_name_list() {
        let strings = ReportStrings::load(&fake_tags);
        let map = Text::from_plain_utf8("Salvation");
        let hero = |custom_name: &str| EpilogueHero {
            mons: mons(b"dwar"),
            name_index: 0,
            custom_name: custom_name.to_owned(),
            battles: 2,
            experience: 20,
        };
        let page = build_epilogue(
            &map,
            &EpilogueReport {
                difficulty: 3,
                survivors: 1,
                units: Vec::new(),
                heroes: vec![hero(""), hero("Vet")],
                kills: 0,
                enemies: Vec::new(),
                damage_ratio: None,
                kill_ratio: None,
            },
            &strings,
            &fake_tags,
        );
        let names: Vec<String> = page.sections[0]
            .rows
            .iter()
            .map(|row| row[0].to_plain_utf8())
            .collect();
        assert_eq!(names, ["Nori (Dwarf)", "Vet (Dwarf)"]);
    }

    #[test]
    fn the_model_holds_the_engines_strings_in_the_engines_order() {
        let page = screenshot_page(Some(2.77));
        assert_eq!(page.title, Text(b"Salvation (Heroic)".to_vec()));
        let kinds: Vec<SectionKind> = page.sections.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            [
                SectionKind::Survivors,
                SectionKind::Heroes,
                SectionKind::Kills,
                SectionKind::DamageRatio,
                SectionKind::KillRatio {
                    no_casualties: false
                },
            ]
        );
        let text = |t: &Text| String::from_utf8(t.0.clone()).unwrap();
        let line = |row: &[Text]| row.iter().map(text).collect::<Vec<_>>();
        let survivors = &page.sections[0];
        assert_eq!(text(&survivors.heading), "1 Survivor");
        assert_eq!(
            line(&survivors.columns),
            [
                "|bUnit|p",
                "|bDamage Inflicted|p",
                "|bKills|p",
                "|bCasualties|p"
            ]
        );
        assert_eq!(
            line(&survivors.rows[0]),
            ["|iBowmen|p", "34.3%", "14", "6/6"]
        );
        assert_eq!(
            line(&page.sections[1].rows[0]),
            ["Nori |i(Dwarf)|p", "1", "20"]
        );
        assert_eq!(text(&page.sections[2].heading), "36 Kills");
        assert_eq!(text(&page.sections[3].heading), "Damage Ratio 1.78:1");
        assert_eq!(text(&page.sections[4].heading), "Kill Ratio 2.77:1");
        for section in &page.sections {
            assert!(
                section
                    .rows
                    .iter()
                    .all(|r| r.len() == section.columns.len()),
                "{:?}",
                section.kind
            );
        }
    }

    #[test]
    fn nothing_lost_is_no_casualties_in_the_kill_ratios_place() {
        let page = screenshot_page(None);
        let last = page.sections.last().unwrap();
        assert_eq!(
            last.kind,
            SectionKind::KillRatio {
                no_casualties: true
            }
        );
        assert_eq!(last.heading, Text(b"|iNo Casualties!|p".to_vec()));
    }

    #[test]
    fn the_printer_leaves_out_hidden_text() {
        let mut page = screenshot_page(Some(2.77));
        page.sections[0].rows[0][0] = Text(b"Bowmen|xsecret|p".to_vec());
        let mut out = Vec::new();
        write_epilogue(&mut out, &page).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\nBowmen "), "{text}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[test]
    fn the_heroic_defeat_screenshot() {
        let text = epilogue_text(&EpilogueReport {
            difficulty: 3,
            survivors: 1,
            units: vec![
                unit(b"bowm", 34.3, 14, 6, 6),
                unit(b"warr", 33.1, 13, 6, 6),
                unit(b"dwar", 32.6, 9, 1, 2),
            ],
            heroes: Vec::new(),
            kills: 36,
            enemies: vec![enemy(b"brig", 26), enemy(b"ghas", 10)],
            damage_ratio: Some(1.78),
            kill_ratio: Some(2.77),
        });
        assert_eq!(
            text,
            "\
Salvation (Heroic)

1 Survivor
Unit      Damage Inflicted  Kills  Casualties
Bowmen               34.3%     14         6/6
Warriors             33.1%     13         6/6
Dwarves              32.6%      9         1/2

36 Kills
Unit      Dead
Brigands    26
Ghasts      10

Damage Ratio 1.78:1
Kill Ratio 2.77:1
"
        );
    }

    #[test]
    fn the_legendary_victory_screenshot() {
        let hero = |id: &[u8; 4], name_index, experience| EpilogueHero {
            mons: mons(id),
            name_index,
            custom_name: String::new(),
            battles: 2,
            experience,
        };
        let text = epilogue_text(&EpilogueReport {
            difficulty: 4,
            survivors: 14,
            units: vec![
                unit(b"bowm", 41.2, 19, 0, 6),
                unit(b"warr", 31.2, 19, 0, 6),
                unit(b"dwar", 27.6, 8, 0, 2),
            ],
            heroes: vec![
                hero(b"dwar", 0, 136),
                hero(b"bowm", 0, 25),
                hero(b"bowm", 1, 23),
                hero(b"bowm", 2, 15),
            ],
            kills: 46,
            enemies: vec![enemy(b"ghas", 30), enemy(b"brig", 16)],
            damage_ratio: Some(14.63),
            kill_ratio: None,
        });
        assert_eq!(
            text,
            "\
Salvation (Legendary)

14 Survivors
Unit      Damage Inflicted  Kills  Casualties
Bowmen               41.2%     19         0/6
Warriors             31.2%     19         0/6
Dwarves              27.6%      8         0/2

Heroes
Name               Battles Survived  Kills
Nori (Dwarf)                      2    136
Simone (Bowman)                   2     25
Piers (Bowman)                    2     23
Petrarch (Bowman)                 2     15

46 Kills
Unit      Dead
Ghasts      30
Brigands    16

Damage Ratio 14.63:1

No Casualties!
"
        );
    }

    #[test]
    fn nothing_taken_says_no_damage_and_no_units_have_no_table() {
        let text = epilogue_text(&EpilogueReport {
            difficulty: 0,
            survivors: 0,
            units: Vec::new(),
            heroes: Vec::new(),
            kills: 0,
            enemies: Vec::new(),
            damage_ratio: None,
            kill_ratio: None,
        });
        assert_eq!(
            text,
            "Salvation (Timid)\n\nNo Damage Taken\n\nNo Casualties!\n"
        );
    }

    #[test]
    fn the_engine_formats_and_styles() {
        assert_eq!(Text(vec![b'a', 0x8e, 0xd2, 0xd3]).to_plain_utf8(), "aé“”");
        assert_eq!(
            Text(b"Brokk |i(Dwarf)|p".to_vec()).to_plain_utf8(),
            "Brokk (Dwarf)"
        );
        let f = format_tag_string;
        assert_eq!(f(b"%d:%02d", &[Arg::Int(3), Arg::Int(7)]), b"3:07");
        assert_eq!(f(b"%.1f%%", &[Arg::Float(34.25)]), b"34.2%");
        assert_eq!(
            f(b"%s |i(%s)|p", &[Arg::Text(b"a"), Arg::Text(b"b")]),
            b"a |i(b)|p"
        );
    }

    #[test]
    fn format_tag_string_clamps_width_and_precision() {
        let padded = format_tag_string(b"%999999999d", &[Arg::Int(7)]);
        assert_eq!(padded.len(), 255);
        assert!(padded.ends_with(b"7"));
        let prec = format_tag_string(b"%.999999f", &[Arg::Float(1.5)]);
        assert!(prec.len() <= 40, "got len {}", prec.len());
        assert!(prec.starts_with(b"1.5"));
    }
}
