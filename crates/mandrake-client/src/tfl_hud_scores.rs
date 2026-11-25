use crate::scores::{self, ScoreLine};
use mandrake_tag::font::RgbColor;

pub(crate) const SCORE_COLUMN_WIDTHS: [i32; 11] = [50, 20, 20, 70, 70, 20, 20, 50, 50, 60, 20];
pub(crate) const TEXT_COLOR: usize = 0x38;
pub(crate) const FONT: usize = 1;
const TABLE_TICKS: i32 = 0x96;
const MINUTE: i32 = 0x708;
const LAST_SECONDS: i32 = 0x384;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TflClock {
    pub(crate) elapsed: i32,
    pub(crate) state: i16,
    pub(crate) limit: i32,
    pub(crate) remaining: i32,
    pub(crate) flags: u32,
    pub(crate) end_reason: i16,
}

impl TflClock {
    pub(crate) fn of_sim(sim: &mandrake_tfl_sim::TflSim) -> Self {
        let h = &sim.data.film.header;
        let mut c = Self::of_game(
            h.time_limit as i32,
            h.planning_time as i32,
            sim.tick as i32,
            sim.map_actions.game.ended_at.map(|e| e as i32),
        );
        if c.flags & 2 != 0 {
            c.end_reason = sim.map_actions.game.end_reason;
        }
        c
    }

    fn of_game(limit: i32, planning: i32, tick: i32, ended: Option<i32>) -> Self {
        let mut c = if limit < 1 {
            Self::of_film(0, tick)
        } else if ended.is_none() && planning > 0 && tick < planning {
            Self {
                elapsed: tick,
                state: 0,
                limit,
                remaining: planning - tick,
                ..Self::default()
            }
        } else {
            let t = ended.map_or(tick, |e| tick.min(e));
            let played = (t - planning.max(0)).max(0);
            Self {
                elapsed: tick,
                state: 2,
                limit,
                remaining: (limit - played).clamp(0, limit),
                ..Self::default()
            }
        };
        if ended.is_some() {
            c.flags |= 2;
        }
        c
    }

    pub(crate) fn of_film(time_limit: i32, tick: i32) -> Self {
        if time_limit < 1 {
            Self {
                elapsed: tick,
                state: 1,
                ..Self::default()
            }
        } else {
            Self {
                elapsed: tick,
                state: 2,
                limit: time_limit,
                remaining: (time_limit - tick).max(0),
                ..Self::default()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    Nothing,
    Header,
    Table,
}

pub(crate) fn shown(c: &TflClock, f7: bool) -> Shown {
    let mut header = match c.state {
        1 => false,
        2 => {
            c.remaining < LAST_SECONDS
                || c.remaining % MINUTE < TABLE_TICKS
                || c.remaining > c.limit - TABLE_TICKS
        }
        _ => true,
    };
    if c.flags & 1 != 0 {
        header = true;
    }
    if c.elapsed < TABLE_TICKS || f7 || c.flags & 2 != 0 {
        Shown::Table
    } else if header {
        Shown::Header
    } else {
        Shown::Nothing
    }
}

pub(crate) fn header_name(c: &TflClock, victory: bool, title: &[u8]) -> Vec<u8> {
    if victory && c.flags & 2 != 0 {
        if c.end_reason == 1 {
            b"VICTORY".to_vec()
        } else {
            b"DEFEAT".to_vec()
        }
    } else {
        title.to_vec()
    }
}

pub(crate) fn time_text(c: &TflClock) -> Vec<u8> {
    let t = if c.state == 1 { c.elapsed } else { c.remaining };
    scores::time_to_string(t)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TflTeamScore {
    pub(crate) captain: Vec<u8>,
    pub(crate) team_name: Vec<u8>,
    pub(crate) primary: RgbColor,
    pub(crate) alive: bool,
    pub(crate) lost: i32,
    pub(crate) surviving: i32,
    pub(crate) score: i32,
    pub(crate) numbers: u32,
    pub(crate) members: i16,
    pub(crate) players: Vec<(Vec<u8>, RgbColor, i32)>,
}

pub(crate) fn score_text(game_type: i16, t: &TflTeamScore) -> Option<Vec<u8>> {
    let s = t.score;
    Some(match game_type {
        0 => format!("{s}%").into_bytes(),
        1 | 2 | 5 | 6 if s != 0 => scores::CHECK_MARK.to_vec(),
        3 | 4 => (0..5)
            .flat_map(|i| {
                if t.numbers & (1 << i) != 0 {
                    format!("{}", i + 1).into_bytes()
                } else {
                    b" ".to_vec()
                }
            })
            .collect(),
        7 | 8 | 10 if s != 0 => format!("{s}").into_bytes(),
        9 if s != 0 => {
            let secs = s / 30;
            format!("{:02}:{:02}", secs / 60, secs % 60).into_bytes()
        }
        _ => return None,
    })
}

fn percent(part: i32, total: i32) -> Vec<u8> {
    let v = if total != 0 { part * 100 / total } else { 0 };
    format!("{v}%").into_bytes()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TflScorePlayer {
    pub(crate) name: Vec<u8>,
    pub(crate) team_name: Vec<u8>,
    pub(crate) primary: RgbColor,
    pub(crate) secondary: RgbColor,
    pub(crate) gone: bool,
}

pub(crate) fn live_teams(
    sim: &mandrake_tfl_sim::TflSim,
    players: &[TflScorePlayer],
) -> Option<Vec<TflTeamScore>> {
    let s = &sim.statistics;
    if s.countdown < 0 {
        return None;
    }
    let teams = sim.statistics_teams().ok()?;
    let dropped = sim.dropped_players();
    let gone = |p: usize| players.get(p).is_none_or(|x| x.gone) || dropped.contains(&p);
    let mut out = Vec::new();
    for r in &s.teams {
        let Some(t) = usize::try_from(r.team).ok() else {
            continue;
        };
        let Some(d) = s.team_data.get(t).filter(|d| d.players > 0) else {
            continue;
        };
        let Some(c) = teams.claims.get(t).copied().flatten().filter(|&c| !gone(c)) else {
            continue;
        };
        let captain = &players[c];
        let members = if d.players > 1 {
            s.players
                .iter()
                .filter_map(|pr| {
                    let p = usize::try_from(pr.player).ok()?;
                    (teams.player_teams.get(p) == Some(&r.team) && !gone(p)).then(|| {
                        let x = &players[p];
                        (x.name.clone(), x.secondary, pr.survivors_cost)
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        out.push(TflTeamScore {
            captain: captain.name.clone(),
            team_name: captain.team_name.clone(),
            primary: captain.primary,
            alive: r.survivors != 0,
            lost: r.losses_cost,
            surviving: r.survivors_cost,
            score: r.score,
            numbers: d.numbers_taken,
            members: d.players,
            players: members,
        });
    }
    Some(out)
}

pub(crate) fn lines(
    game_type: i16,
    title: &[u8],
    clock: &TflClock,
    shown: Shown,
    teams: &[TflTeamScore],
) -> Vec<ScoreLine> {
    if shown == Shown::Nothing {
        return Vec::new();
    }
    let mut out = vec![ScoreLine {
        row: 0,
        player: false,
        active: true,
        fill: None,
        score: None,
        name: title.to_vec(),
        right: time_text(clock),
        stars: Vec::new(),
    }];
    if shown != Shown::Table {
        return out;
    }
    let mut row = 1;
    for t in teams {
        let total = t.lost + t.surviving;
        out.push(ScoreLine {
            row,
            player: false,
            active: t.alive,
            fill: Some(t.primary),
            score: score_text(game_type, t),
            name: if t.members > 1 {
                t.team_name.clone()
            } else {
                t.captain.clone()
            },
            right: percent(t.surviving, total),
            stars: Vec::new(),
        });
        row += 1;
        if t.members > 1 {
            for (name, secondary, part) in &t.players {
                out.push(ScoreLine {
                    row,
                    player: true,
                    active: t.alive,
                    fill: Some(*secondary),
                    score: None,
                    name: name.clone(),
                    right: percent(*part, total),
                    stars: Vec::new(),
                });
                row += 1;
            }
        }
    }
    out
}

pub(crate) fn score_width(game_type: i16) -> i32 {
    usize::try_from(game_type)
        .ok()
        .and_then(|t| SCORE_COLUMN_WIDTHS.get(t).copied())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_planning_time_counts_down_then_the_time_limit() {
        let at = |t| TflClock::of_game(1800, 1800, t, None);
        assert_eq!(time_text(&at(270)), scores::time_to_string(1530));
        assert_eq!(at(270).state, 0);
        assert_eq!(shown(&at(600), false), Shown::Header);
        assert_eq!((at(1800).state, at(1800).remaining), (2, 1800));
        assert_eq!(at(1800 + 270).remaining, 1530);
        let c = TflClock::of_game(1800, 1800, 900, Some(900));
        assert_eq!((c.state, c.remaining, c.flags & 2), (2, 1800, 2));
    }

    #[test]
    fn the_whole_table_shows_for_150_ticks_then_the_header_at_each_minute() {
        let at = |t| TflClock::of_film(18000, t);
        assert_eq!(shown(&at(110), false), Shown::Table);
        assert_eq!(shown(&at(150), false), Shown::Nothing);
        assert_eq!(shown(&at(150), true), Shown::Table);
        assert_eq!(shown(&at(18000 - 9 * 1800), false), Shown::Header);
        assert_eq!(shown(&at(18000 - 9 * 1800 - 150), false), Shown::Nothing);
        assert_eq!(shown(&at(18000 - 899), false), Shown::Header);
        assert_eq!(time_text(&at(110)), b"9:56\r".to_vec());
        assert_eq!(shown(&TflClock::of_film(0, 10), false), Shown::Table);
        assert_eq!(shown(&TflClock::of_film(0, 1000), false), Shown::Nothing);
    }

    #[test]
    fn a_cooperative_game_ends_with_myth2s_victory_header() {
        let mut c = TflClock::of_film(18000, 500);
        assert_eq!(header_name(&c, true, b"Body Count"), b"Body Count".to_vec());
        c.flags |= 2;
        c.end_reason = 1;
        assert_eq!(header_name(&c, true, b"Body Count"), b"VICTORY".to_vec());
        assert_eq!(
            header_name(&c, false, b"Body Count"),
            b"Body Count".to_vec()
        );
        c.end_reason = 2;
        assert_eq!(header_name(&c, true, b"Body Count"), b"DEFEAT".to_vec());
    }

    #[test]
    fn a_team_of_one_shows_its_player() {
        let t = TflTeamScore {
            captain: b"steamuser".to_vec(),
            team_name: b"steamuser's Team".to_vec(),
            primary: [0x4ccc, 0x4d7f, 0xffff],
            alive: true,
            lost: 0,
            surviving: 1000,
            score: 0,
            numbers: 0,
            members: 1,
            players: Vec::new(),
        };
        let c = TflClock::of_film(18000, 110);
        let l = lines(0, b"Body Count", &c, Shown::Table, &[t]);
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].name, b"steamuser".to_vec());
        assert_eq!(l[1].score, Some(b"0%".to_vec()));
        assert_eq!(l[1].right, b"100%".to_vec());
        assert_eq!(score_width(0), 50);
    }

    #[test]
    fn the_flag_games_show_a_check_mark() {
        let mut t = TflTeamScore {
            captain: b"a".to_vec(),
            team_name: Vec::new(),
            primary: [0; 3],
            alive: true,
            lost: 0,
            surviving: 1,
            score: 0,
            numbers: 0,
            members: 1,
            players: Vec::new(),
        };
        for ty in [1, 2, 5, 6, 10] {
            assert_eq!(score_text(ty, &t), None, "type {ty}");
        }
        t.score = 1;
        for ty in [1, 2, 5, 6] {
            assert_eq!(score_text(ty, &t), Some(b"\xc3".to_vec()), "type {ty}");
        }
        t.score = 3;
        assert_eq!(score_text(10, &t), Some(b"3".to_vec()));
    }
}
