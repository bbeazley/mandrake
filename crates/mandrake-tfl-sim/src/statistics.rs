use mandrake_common::FourCC;
use mandrake_common::game_options::{COOPERATIVE, LIMITED_VISIBILITY, tfl};

use crate::draw_site;
use crate::flags::FlagCount;
use crate::setup::MonsterInitializer;

pub const ROW: usize = 33;

pub const SAMPLES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreMatrix {
    pub rows: Vec<[i32; ROW]>,
}

impl Default for ScoreMatrix {
    fn default() -> Self {
        Self {
            rows: vec![[0; ROW]; 17],
        }
    }
}

impl ScoreMatrix {
    fn add(&mut self, attacker: i16, victim: usize, amount: i32, killed: bool) {
        let row = if attacker == -1 {
            16
        } else {
            attacker as usize
        };
        let (Some(r), true) = (self.rows.get_mut(row), victim < 16) else {
            return;
        };
        r[victim] = r[victim].wrapping_add(amount);
        if killed {
            r[16 + victim] = r[16 + victim].wrapping_add(1);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub unit: FourCC,
    pub player: i16,
    pub mons: FourCC,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitRecord {
    pub fielded: i32,
    pub hit_points: i32,
    pub alive: i32,
    pub hit_points_and_damage: i32,
    pub kills: i32,
    pub damage: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerRanking {
    pub player: i16,
    pub losses: i32,
    pub losses_cost: i32,
    pub survivors: i32,
    pub survivors_cost: i32,
    pub kills: i32,
    pub damage: i32,
    pub score: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TeamRanking {
    pub team: i16,
    pub rank: i16,
    pub losses: i32,
    pub losses_cost: i32,
    pub survivors: i32,
    pub survivors_cost: i32,
    pub kills: i32,
    pub damage: i32,
    pub score: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statistics {
    pub period: i16,
    pub counter: i16,
    pub sample: i16,
    pub countdown: i16,
    pub updates: i16,
    pub samples: Vec<ScoreMatrix>,
    pub totals: ScoreMatrix,
    pub palette: Vec<PaletteEntry>,
    pub units: Vec<UnitRecord>,
    pub players: Vec<PlayerRanking>,
    pub teams: Vec<TeamRanking>,
    pub definitions: Vec<(FourCC, u32)>,
    pub team_data: Vec<TeamData>,
    pub teams_with_players: i16,
    pub permanent: Vec<PermanentRanking>,
    pub permanent_recorded: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TeamData {
    pub flags: u16,
    pub flag_time: i32,
    pub last_taken: i32,
    pub flag_tick: u32,
    pub numbers_taken: u32,
    pub field_18: i16,
    pub flags_held: i16,
    pub flags_best: i16,
    pub players: i16,
    pub players_at_start: i16,
    pub score: i16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PermanentRanking {
    pub player: i16,
    pub id: i16,
    pub points: i32,
    pub losses: i32,
    pub kills: i32,
    pub damage: i32,
    pub places: [i16; 3],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RankingOutcome {
    pub eliminated: Vec<(i16, i16)>,
    pub end: bool,
    pub late: Vec<LateOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LateOutcome {
    Suspend(bool),
    EndGame,
    AboutToWin(i16),
    Eliminated(i16, i16),
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            period: 300,
            counter: 0,
            sample: 1,
            countdown: -1,
            updates: 0,
            samples: vec![ScoreMatrix::default(); SAMPLES],
            totals: ScoreMatrix::default(),
            palette: Vec::new(),
            units: Vec::new(),
            players: Vec::new(),
            teams: Vec::new(),
            definitions: Vec::new(),
            team_data: Vec::new(),
            teams_with_players: 0,
            permanent: Vec::new(),
            permanent_recorded: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Teams {
    pub player_teams: Vec<i16>,
    pub teams: usize,
    pub game_type: i16,
    pub options: u32,
    pub claims: Vec<Option<usize>>,
    pub tick: u32,
    pub ended: bool,
}

impl Teams {
    fn no_elimination(&self) -> bool {
        self.options & COOPERATIVE != 0
    }
}

impl Statistics {
    pub fn entry(&mut self, unit: FourCC, player: i16, mons: FourCC) -> i16 {
        let e = PaletteEntry { unit, player, mons };
        if let Some(k) = self.palette.iter().position(|p| *p == e) {
            return k as i16;
        }
        self.palette.push(e);
        self.units.push(UnitRecord::default());
        (self.palette.len() - 1) as i16
    }

    pub fn add_monsters(&mut self, entry: i16, count: i32, vitality: i32) {
        if let Some(r) = usize::try_from(entry)
            .ok()
            .and_then(|e| self.units.get_mut(e))
        {
            r.fielded = r.fielded.wrapping_add(count);
            r.hit_points = r.hit_points.wrapping_add(vitality);
            r.alive = r.alive.wrapping_add(count);
            r.hit_points_and_damage = r.hit_points_and_damage.wrapping_add(vitality);
        }
    }

    pub fn score(
        &mut self,
        attacker: i16,
        victim: i16,
        scaled: i32,
        killed: bool,
        teams: &Teams,
    ) -> Option<(i16, bool)> {
        let v = usize::try_from(victim)
            .ok()
            .filter(|&v| v < self.palette.len())?;
        let player_of = |e: i16| {
            usize::try_from(e)
                .ok()
                .and_then(|e| self.palette.get(e))
                .map_or(-1, |p| p.player)
        };
        let victim_player = self.palette[v].player;
        let attacker_player = player_of(attacker);
        let team = |p: i16| {
            usize::try_from(p)
                .ok()
                .and_then(|p| teams.player_teams.get(p).copied())
                .unwrap_or(-1)
        };
        let same_team = attacker != -1
            && attacker_player != -1
            && victim_player != -1
            && team(attacker_player) == team(victim_player);
        if victim_player != -1 {
            let vp = victim_player as usize;
            let s = usize::try_from(self.sample - 1).unwrap_or(0);
            if let Some(m) = self.samples.get_mut(s) {
                m.add(attacker_player, vp, scaled, killed);
            }
            self.totals.add(attacker_player, vp, scaled, killed);
        }
        if attacker != -1
            && !same_team
            && let Some(r) = self.units.get_mut(attacker as usize)
        {
            r.damage = r.damage.wrapping_add(scaled);
            if killed {
                r.kills = r.kills.wrapping_add(1);
            }
        }
        let r = &mut self.units[v];
        r.hit_points_and_damage = r.hit_points_and_damage.wrapping_add(scaled);
        if killed {
            r.alive = r.alive.wrapping_sub(1);
        }
        Some((victim_player, same_team))
    }

    pub fn rankings_due(&self) -> bool {
        self.countdown.wrapping_sub(1) < 0
    }

    pub fn advance_sample_clock(&mut self) {
        self.counter = self.counter.wrapping_add(1);
        if self.period < self.counter {
            self.sample = self.sample.wrapping_add(1);
            if self.sample > SAMPLES as i16 {
                for k in 0..SAMPLES / 2 {
                    let mut merged = self.samples[2 * k].clone();
                    for (a, b) in merged.rows.iter_mut().zip(&self.samples[2 * k + 1].rows) {
                        for (x, y) in a.iter_mut().zip(b) {
                            *x = x.wrapping_add(*y);
                        }
                    }
                    self.samples[k] = merged;
                }
                self.sample = 11;
                self.period = self.period.wrapping_shl(1);
            }
            self.counter = 0;
            if let Some(m) = usize::try_from(self.sample - 1)
                .ok()
                .and_then(|s| self.samples.get_mut(s))
            {
                *m = ScoreMatrix::default();
            }
        }
    }

    pub fn update(
        &mut self,
        teams: &Teams,
        costs: &[i32],
        winner: &mut i16,
        flags: Option<&FlagCount>,
    ) -> Result<RankingOutcome, crate::Stop> {
        let mut out = RankingOutcome::default();
        self.countdown = self.countdown.wrapping_sub(1);
        if self.countdown < 0 {
            self.countdown = 0x3c;
            self.updates = self.updates.wrapping_add(1);
            let none = FlagCount {
                held: vec![0; teams.teams],
                settled: true,
                ..FlagCount::default()
            };
            out = self.update_rankings(teams, costs, winner, flags.unwrap_or(&none))?;
        }
        self.advance_sample_clock();
        Ok(out)
    }

    pub fn set_up_teams(&mut self, teams: &Teams, ids: &[i16], previous: &[PermanentRanking]) {
        self.team_data = vec![TeamData::default(); teams.teams];
        for &t in &teams.player_teams {
            if let Some(d) = usize::try_from(t)
                .ok()
                .and_then(|t| self.team_data.get_mut(t))
            {
                d.players = d.players.wrapping_add(1);
            }
        }
        self.teams_with_players = 0;
        for d in &mut self.team_data {
            d.players_at_start = d.players;
            if d.players > 0 {
                self.teams_with_players += 1;
            }
        }
        self.permanent = ids
            .iter()
            .enumerate()
            .map(|(k, &id)| PermanentRanking {
                player: k as i16,
                id,
                ..PermanentRanking::default()
            })
            .collect();
        self.carry_permanent(previous);
        self.permanent_recorded = false;
    }

    pub fn carry_permanent(&mut self, previous: &[PermanentRanking]) {
        for r in &mut self.permanent {
            if let Some(p) = previous.iter().find(|p| p.id == r.id) {
                *r = PermanentRanking {
                    player: r.player,
                    ..*p
                };
            }
        }
    }

    fn update_rankings(
        &mut self,
        teams: &Teams,
        costs: &[i32],
        winner: &mut i16,
        count: &FlagCount,
    ) -> Result<RankingOutcome, crate::Stop> {
        let options = teams.options;
        let mut flags = count.held.clone();
        flags.resize(teams.teams, 0);
        let mut out = RankingOutcome::default();
        if self.team_data.len() < teams.teams {
            self.team_data.resize(teams.teams, TeamData::default());
        }
        let old = std::mem::take(&mut self.teams);
        self.teams = (0..teams.teams)
            .map(|t| {
                let d = &mut self.team_data[t];
                let held = flags[t];
                if held != 0 {
                    d.flag_time = d.flag_time.wrapping_add(0x3c);
                }
                d.flags_held = held;
                let keep = if options & tfl::FIXES_2000 != 0 {
                    held >= d.flags_best
                } else {
                    held <= d.flags_best
                };
                if keep {
                    d.flags_best = held;
                    d.flag_tick = teams.tick;
                }
                TeamRanking {
                    team: t as i16,
                    rank: old.get(t).map_or(0, |o| o.rank),
                    score: i32::from(d.score),
                    ..TeamRanking::default()
                }
            })
            .collect();
        let players = teams.player_teams.len().min(16);
        let mut sums = vec![PlayerRanking::default(); players];
        for (k, (e, r)) in self.palette.iter().zip(&self.units).enumerate() {
            let Some(s) = usize::try_from(e.player).ok().and_then(|p| sums.get_mut(p)) else {
                continue;
            };
            let cost = costs.get(k).copied().unwrap_or(0);
            let lost = r.fielded.wrapping_sub(r.alive);
            s.survivors = s.survivors.wrapping_add(r.alive);
            s.survivors_cost = s.survivors_cost.wrapping_add(cost.wrapping_mul(r.alive));
            s.kills = s.kills.wrapping_add(r.kills);
            s.losses = s.losses.wrapping_add(lost);
            s.losses_cost = s.losses_cost.wrapping_add(lost.wrapping_mul(cost));
            s.damage = s.damage.wrapping_add(r.damage);
        }
        let mut total = 0i32;
        let (mut alive, mut first, mut several) = (0u32, -1i16, false);
        for (p, s) in sums.iter_mut().enumerate() {
            s.player = p as i16;
            s.score = self.totals.rows[p][32];
            if options & tfl::FIXES_8000 != 0 {
                s.damage = s
                    .damage
                    .wrapping_sub(self.totals.rows[16][p])
                    .wrapping_sub(self.totals.rows[p][p]);
            }
            total = total.wrapping_add(s.damage.wrapping_abs());
            let team = teams.player_teams[p];
            let Some(t) = usize::try_from(team).ok().filter(|&t| t < self.teams.len()) else {
                continue;
            };
            let r = &mut self.teams[t];
            r.survivors = r.survivors.wrapping_add(s.survivors);
            r.survivors_cost = r.survivors_cost.wrapping_add(s.survivors_cost);
            r.kills = r.kills.wrapping_add(s.kills);
            r.losses = r.losses.wrapping_add(s.losses);
            r.losses_cost = r.losses_cost.wrapping_add(s.losses_cost);
            r.damage = r.damage.wrapping_add(s.damage);
            r.score = r.score.wrapping_add(s.score);
            if s.survivors != 0 && self.team_data[t].flags & 1 == 0 {
                alive |= 1u32.wrapping_shl(t as u32);
                if first == -1 {
                    first = team;
                } else if team != first {
                    several = true;
                }
            }
        }
        for t in 0..self.teams.len() {
            let d = &mut self.team_data[t];
            if d.players > 0
                && !teams.no_elimination()
                && alive & 1u32.wrapping_shl(t as u32) == 0
                && d.flags & 1 == 0
            {
                d.flags |= 1;
                out.eliminated
                    .push((t as i16, if d.players == 1 { 9 } else { 10 }));
            }
            let d = self.team_data[t];
            let r = &mut self.teams[t];
            match teams.game_type {
                0 => {
                    r.score = if total == 0 {
                        0
                    } else {
                        r.damage.wrapping_mul(100).wrapping_div(total)
                    }
                }
                1 | 2 | 5..=8 => r.score = i32::from(flags.get(t).copied().unwrap_or(0)),
                3 | 4 => r.score = i32::from(d.field_18),
                9 => r.score = d.flag_time,
                _ => {}
            }
        }
        let all_dead_one_team = self.teams_with_players == 1 && first == -1;
        if !several
            && !teams.no_elimination()
            && (self.teams_with_players > 1 || all_dead_one_team)
            && !teams.ended
        {
            if !(teams.game_type == 9 || teams.game_type == 0 && options & tfl::FIXES_2000 != 0) {
                *winner = first;
            }
            out.end = true;
        }
        let claims = &teams.claims;
        let player_team = |p: i16| {
            usize::try_from(p)
                .ok()
                .and_then(|p| teams.player_teams.get(p).copied())
                .unwrap_or(-1)
        };
        qsort(&mut sums, |a, b| {
            let (ta, tb) = (player_team(a.player), player_team(b.player));
            if ta == -1 {
                return i32::from(tb != -1);
            }
            if tb == -1 {
                return -1;
            }
            let claimer = |t: i16, p: i16| {
                claims.get(t as usize).copied().flatten() == usize::try_from(p).ok()
            };
            let (ca, cb) = (claimer(ta, a.player), claimer(tb, b.player));
            if ca != cb {
                return if ca { -1 } else { 1 };
            }
            match b.kills.wrapping_sub(a.kills) {
                0 => b.damage.wrapping_sub(a.damage),
                r => r,
            }
        });
        self.players = sums;
        let data = &self.team_data;
        let game_type = teams.game_type;
        let w = *winner;
        let compare = |a: &TeamRanking, b: &TeamRanking| -> i32 {
            compare_team_scores(a, b, data, game_type, options, w)
        };
        let mut sorted = std::mem::take(&mut self.teams);
        qsort(&mut sorted, compare);
        let mut place = 0i16;
        for i in 0..sorted.len() {
            if options & tfl::FIXES_2000 == 0 || (i > 0 && compare(&sorted[i], &sorted[i - 1]) != 0)
            {
                place = place.wrapping_add(1);
            }
            sorted[i].rank = place;
        }
        self.teams = sorted;
        if (1..=8).contains(&teams.game_type) {
            out.late.push(LateOutcome::Suspend(!count.settled));
            #[allow(clippy::needless_range_loop)]
            for t in 0..teams.teams {
                match teams.game_type {
                    7 | 8 if flags.get(t) == Some(&count.unnumbered) => {
                        out.late.push(LateOutcome::EndGame)
                    }
                    3 | 4 => match self.team_data[t].field_18.wrapping_sub(count.numbered) {
                        -1 => {
                            let d = &mut self.team_data[t];
                            if d.flags & 2 == 0 {
                                d.flags |= 2;
                                out.late.push(LateOutcome::AboutToWin(t as i16));
                            }
                        }
                        0 => out.late.push(LateOutcome::EndGame),
                        _ => {}
                    },
                    5 | 6 => {
                        let d = &mut self.team_data[t];
                        if flags.get(t) == Some(&0) && d.flags & 1 == 0 && d.players > 0 {
                            d.flags |= 1;
                            let sound = if d.players == 1 { 9 } else { 10 };
                            out.late.push(LateOutcome::Eliminated(t as i16, sound));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(out)
    }

    pub fn record_permanent_rankings(&mut self, player_teams: &[i16]) {
        for t in &self.teams {
            for p in &self.players {
                let team = usize::try_from(p.player)
                    .ok()
                    .and_then(|k| player_teams.get(k).copied())
                    .unwrap_or(-1);
                if team != t.team {
                    continue;
                }
                let Some(r) = usize::try_from(p.player)
                    .ok()
                    .and_then(|k| self.permanent.get_mut(k))
                else {
                    continue;
                };
                let place = i64::from(t.rank);
                r.points = r.points.wrapping_add((0x10000 / (place + 1)) as i32);
                if (0..3).contains(&t.rank) {
                    r.places[t.rank as usize] = r.places[t.rank as usize].wrapping_add(1);
                }
                r.losses = r.losses.wrapping_add(p.losses);
                r.kills = r.kills.wrapping_add(p.kills);
                r.damage = r.damage.wrapping_add(p.damage);
            }
        }
        qsort(&mut self.permanent, |a, b| b.points.wrapping_sub(a.points));
        self.permanent_recorded = true;
    }

    pub fn gather(
        &self,
        order: &[FourCC],
        flags: &dyn Fn(FourCC) -> u32,
        teams: &Teams,
        local_team: i16,
        own: bool,
    ) -> Vec<MonsterRow> {
        order
            .iter()
            .map(|&mons| {
                let mut row = MonsterRow {
                    mons,
                    kills: 0,
                    dead: 0,
                    fielded: 0,
                    damage: 0,
                    hit_points: 0,
                    hit_points_and_damage: 0,
                };
                if flags(mons) & 0x800 != 0 {
                    return row;
                }
                for (e, r) in self.palette.iter().zip(&self.units) {
                    if e.player == -1 || e.mons != mons {
                        continue;
                    }
                    let team = teams
                        .player_teams
                        .get(e.player as usize)
                        .copied()
                        .unwrap_or(-1);
                    if (team == local_team) != own {
                        continue;
                    }
                    row.fielded = row.fielded.wrapping_add(r.fielded as i16);
                    row.kills = row.kills.wrapping_add(r.kills as i16);
                    row.dead = row
                        .dead
                        .wrapping_add(r.fielded.wrapping_sub(r.alive) as i16);
                    row.damage = row.damage.wrapping_add(r.damage);
                    row.hit_points = row.hit_points.wrapping_add(r.hit_points);
                    row.hit_points_and_damage = row
                        .hit_points_and_damage
                        .wrapping_add(r.hit_points_and_damage);
                }
                row
            })
            .collect()
    }

    pub fn epilogue(
        &self,
        order: &[FourCC],
        flags: &dyn Fn(FourCC) -> u32,
        teams: &Teams,
        local_team: Option<i16>,
    ) -> Option<Epilogue> {
        let local_team = local_team?;
        let team = *self.teams.iter().find(|t| t.team == local_team)?;
        let mut own = self.gather(order, flags, teams, local_team, true);
        qsort(&mut own, |a, b| b.damage.wrapping_sub(a.damage));
        let total = own.iter().fold(0i32, |n, r| n.wrapping_add(r.damage));
        let taken = own.iter().fold(0i32, |n, r| {
            n.wrapping_add(r.hit_points.wrapping_sub(r.hit_points_and_damage))
        });
        let units = own
            .iter()
            .filter(|r| r.damage != 0)
            .take(4)
            .map(|r| EpilogueUnit {
                mons: r.mons,
                damage_share: if total == 0 {
                    0.0
                } else {
                    f64::from(r.damage) * 100.0 / f64::from(total)
                },
                kills: r.kills,
                dead: r.dead,
                fielded: r.fielded,
            })
            .collect();
        let mut others = self.gather(order, flags, teams, local_team, false);
        qsort(&mut others, |a, b| i32::from(b.dead) - i32::from(a.dead));
        let listed = |r: &&MonsterRow| r.fielded != 0 && r.dead > 1 && flags(r.mons) & 0x800 == 0;
        let kills = others
            .iter()
            .filter(listed)
            .map(|r| i32::from(r.dead))
            .fold(0i32, i32::wrapping_add);
        let enemies = others
            .iter()
            .filter(listed)
            .take(4)
            .map(|r| EpilogueEnemy {
                mons: r.mons,
                dead: r.dead,
                fielded: r.fielded,
            })
            .collect();
        let damage_ratio = (taken != 0).then(|| -(f64::from(team.damage) / f64::from(taken)));
        let kill_ratio = (team.losses != 0).then(|| f64::from(team.kills) / f64::from(team.losses));
        Some(Epilogue {
            survivors: team.survivors,
            units,
            kills,
            enemies,
            damage_ratio,
            kill_ratio,
            heroes: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterRow {
    pub mons: FourCC,
    pub kills: i16,
    pub dead: i16,
    pub fielded: i16,
    pub damage: i32,
    pub hit_points: i32,
    pub hit_points_and_damage: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Epilogue {
    pub survivors: i32,
    pub units: Vec<EpilogueUnit>,
    pub kills: i32,
    pub enemies: Vec<EpilogueEnemy>,
    pub damage_ratio: Option<f64>,
    pub kill_ratio: Option<f64>,
    pub heroes: Vec<EpilogueHero>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpilogueHero {
    pub mons: FourCC,
    pub name: Vec<u8>,
    pub name_index: u8,
    pub battles: i8,
    pub kills: i8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EpilogueUnit {
    pub mons: FourCC,
    pub damage_share: f64,
    pub kills: i16,
    pub dead: i16,
    pub fielded: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpilogueEnemy {
    pub mons: FourCC,
    pub dead: i16,
    pub fielded: i16,
}

fn compare_team_scores(
    a: &TeamRanking,
    b: &TeamRanking,
    data: &[TeamData],
    game_type: i16,
    options: u32,
    winner: i16,
) -> i32 {
    let start = |t: i16| {
        usize::try_from(t)
            .ok()
            .and_then(|t| data.get(t))
            .map_or(0, |d| d.players_at_start)
    };
    let (sa, sb) = (start(a.team), start(b.team));
    let team = |t: i16| {
        usize::try_from(t)
            .ok()
            .and_then(|t| data.get(t))
            .copied()
            .unwrap_or_default()
    };
    let (da, db) = (team(a.team), team(b.team));
    let mut r = if sa == 0 {
        i32::from(sb != 0)
    } else if sb == 0 {
        -1
    } else {
        let mut r = b.score.wrapping_sub(a.score);
        if r == 0 && game_type == 0 && options & tfl::FIXES_8000 == 0 {
            r = a.losses_cost.wrapping_sub(b.losses_cost);
        }
        let mut decided = false;
        if r == 0 {
            match game_type {
                1 | 2 => r = db.last_taken.wrapping_sub(da.last_taken),
                3 | 4 => r = da.last_taken.wrapping_sub(db.last_taken),
                7 | 8 => {
                    r = i32::from(db.flags_best) - i32::from(da.flags_best);
                    decided = r != 0;
                    if !decided {
                        r = da.last_taken.wrapping_sub(db.last_taken);
                    }
                }
                _ => {}
            }
        }
        if r == 0 && !decided && options & (tfl::FIXES_2000 | tfl::FIXES_8000) == 0 {
            r = b.damage.wrapping_sub(a.damage);
            if r == 0 {
                r = a.losses_cost.wrapping_sub(b.losses_cost);
            }
        }
        r
    };
    if winner != -1 {
        if a.team == winner && b.team != winner {
            r = -1;
        } else if b.team == winner && a.team != winner {
            r = 1;
        }
    }
    r
}

pub fn qsort<T: Copy>(v: &mut [T], mut compare: impl FnMut(&T, &T) -> i32) {
    let n = v.len();
    if n < 2 {
        return;
    }
    let mut left = n / 2 + 1;
    let mut right = n;
    let mut last = n - 1;
    loop {
        if left > 1 {
            left -= 1;
        } else {
            v.swap(0, last);
            right = last;
            if last == 1 {
                return;
            }
            last -= 1;
        }
        let mut i = left - 1;
        let mut j = left * 2;
        while j <= right {
            let mut child = j - 1;
            if j < right && compare(&v[child], &v[child + 1]) < 0 {
                j += 1;
                child += 1;
            }
            if compare(&v[i], &v[child]) >= 0 {
                break;
            }
            v.swap(i, child);
            i = child;
            j *= 2;
        }
    }
}

impl crate::TflSim {
    pub fn statistics_teams(&self) -> Result<Teams, crate::Stop> {
        let claims = self.team_claims()?;
        Ok(Teams {
            player_teams: claims.player_teams,
            teams: claims.claims.len(),
            game_type: self.data.film.header.game_type,
            options: self.game_options,
            claims: claims.claims,
            tick: self.tick,
            ended: self.statistics_stopped(),
        })
    }

    pub fn local_player(&self) -> i16 {
        self.data.film.header.local_player
    }

    pub(crate) fn cache_local_team(&mut self) -> Result<(), crate::Stop> {
        let p = self.local_player();
        if p == -1 {
            self.cached_local_team = Some((p, None, false));
            return Ok(());
        }
        let claims = self.map_team_claims()?;
        let team = usize::try_from(p)
            .ok()
            .and_then(|i| claims.player_teams.get(i).copied())
            .ok_or_else(|| {
                crate::Stop::new(
                    crate::draw_site::LOCAL_PLAYER_PAST_FILM,
                    format!("the local player {p} past the film's players"),
                )
            })?;
        let leads = usize::try_from(p)
            .ok()
            .and_then(|i| claims.leaders.get(i).copied())
            .unwrap_or(false);
        self.cached_local_team = Some((p, (team >= 0).then_some(team), leads));
        Ok(())
    }

    pub fn local_team(&self) -> Result<Option<i16>, crate::Stop> {
        let p = self.local_player();
        if let Some((cached_p, team, _)) = self.cached_local_team
            && cached_p == p
        {
            return Ok(team);
        }
        if p == -1 {
            return Ok(None);
        }
        let claims = self.map_team_claims()?;
        let team = usize::try_from(p)
            .ok()
            .and_then(|i| claims.player_teams.get(i).copied())
            .ok_or_else(|| {
                crate::Stop::new(
                    crate::draw_site::LOCAL_PLAYER_PAST_FILM,
                    format!("the local player {p} past the film's players"),
                )
            })?;
        Ok((team >= 0).then_some(team))
    }

    pub fn local_player_leads(&self) -> Result<bool, crate::Stop> {
        let p = self.local_player();
        if let Some((cached_p, _, leads)) = self.cached_local_team
            && cached_p == p
        {
            return Ok(leads);
        }
        let claims = self.map_team_claims()?;
        Ok(usize::try_from(p)
            .ok()
            .and_then(|i| claims.leaders.get(i).copied())
            .unwrap_or(false))
    }

    pub fn game_over(&self) -> bool {
        self.map_actions
            .game
            .ended_at
            .is_some_and(|at| self.tick >= at.saturating_add(300))
    }

    fn statistics_stopped(&self) -> bool {
        self.map_actions.game.ended_at.is_some()
    }

    fn palette_costs(&mut self) -> Result<Vec<i32>, crate::Stop> {
        let mons: Vec<FourCC> = self.statistics.palette.iter().map(|p| p.mons).collect();
        mons.into_iter()
            .map(|m| Ok(i32::from(self.data.def(b"mons", m)?.i16(0x19e))))
            .collect()
    }

    pub(crate) fn set_up_statistics_teams(
        &mut self,
        previous: &[PermanentRanking],
    ) -> Result<(), crate::Stop> {
        let teams = self.statistics_teams()?;
        let ids: Vec<i16> = self
            .data
            .film
            .header
            .players
            .iter()
            .map(|p| p.net_id)
            .collect();
        self.statistics.set_up_teams(&teams, &ids, previous);
        Ok(())
    }

    pub fn set_previous_permanent_rankings(&mut self, previous: &[PermanentRanking]) {
        self.statistics.carry_permanent(previous);
    }

    pub(crate) fn update_player_statistics(&mut self) -> Result<(), crate::Stop> {
        if self.statistics_stopped() {
            if !self.statistics.permanent_recorded {
                let teams = self.statistics_teams()?;
                self.statistics
                    .record_permanent_rankings(&teams.player_teams);
            }
            return Ok(());
        }
        if !self.statistics.rankings_due() {
            self.statistics.countdown = self.statistics.countdown.wrapping_sub(1);
            self.statistics.advance_sample_clock();
            return Ok(());
        }
        let teams = self.statistics_teams()?;
        let costs = self.palette_costs()?;
        let mode = crate::flags::flag_count_mode(teams.game_type);
        let flags = self.count_team_flags(mode, 0x3c)?;
        let mut winner = self.winning_team;
        let out = self
            .statistics
            .update(&teams, &costs, &mut winner, Some(&flags))?;
        self.winning_team = winner;
        self.apply_rankings(out)
    }

    fn apply_rankings(&mut self, out: RankingOutcome) -> Result<(), crate::Stop> {
        for (team, sound) in out.eliminated {
            self.team_eliminated(draw_site::RANKINGS_ELIMINATED_SOUND, team, sound)?;
        }
        if out.end {
            self.map_actions.game.end_suspended = false;
            self.end_game();
        }
        for late in out.late {
            match late {
                LateOutcome::Suspend(s) => self.map_actions.game.end_suspended = s,
                LateOutcome::EndGame => self.end_game(),
                LateOutcome::AboutToWin(_) => self.game_sound(draw_site::ABOUT_TO_WIN_SOUND, 3),
                LateOutcome::Eliminated(team, sound) => {
                    self.team_eliminated(draw_site::RANKINGS_NO_FLAG_SOUND, team, sound)?
                }
            }
        }
        Ok(())
    }

    pub(crate) fn team_eliminated(
        &mut self,
        site: u32,
        team: i16,
        sound: i16,
    ) -> Result<(), crate::Stop> {
        let local = self.local_team()?;
        self.game_sound(site, sound);
        for m in 0..self.monsters.len() {
            if self.monsters[m].identifier != 0 && self.monsters[m].team as i8 as i16 == team {
                self.fade_monster(m)?;
            }
        }
        if local == Some(team) && self.game_options & COOPERATIVE == 0 {
            self.game_options &= !LIMITED_VISIBILITY;
        }
        Ok(())
    }

    pub(crate) fn end_game(&mut self) {
        let g = &mut self.map_actions.game;
        if g.end_suspended {
            if g.ended_at.is_none() && !g.overtime {
                g.overtime = true;
                self.game_sound(draw_site::END_GAME_SOUND, 5);
            }
            return;
        }
        self.finalize_winning_team();
        if self.game_options & COOPERATIVE == 0 {
            self.game_options &= !LIMITED_VISIBILITY;
        }
        if self.map_actions.game.ended_at.is_none() {
            self.game_sound(draw_site::END_GAME_SOUND, 6);
        }
        let g = &mut self.map_actions.game;
        g.ended_at.get_or_insert(self.tick);
        g.end_reason = 3;
    }

    fn finalize_winning_team(&mut self) {
        let s = &self.statistics;
        if self.winning_team != -1 || s.countdown < 0 {
            return;
        }
        let rank = |k: usize| s.teams.get(k).map_or(0, |t| t.rank);
        if s.teams_with_players < 2 || rank(0) != rank(1) {
            self.winning_team = s.teams.first().map_or(0, |t| t.team);
        }
    }

    pub(crate) fn force_player_statistics_update(&mut self) -> Result<(), crate::Stop> {
        self.statistics.countdown = 0;
        self.update_player_statistics()
    }

    pub(crate) fn adjust_monster_scoring(
        &mut self,
        attacker: i16,
        victim: i16,
        amount: i32,
        killed: bool,
    ) -> Result<(), crate::Stop> {
        if self.statistics_stopped() {
            return Ok(());
        }
        let Some(entry) = usize::try_from(victim)
            .ok()
            .and_then(|v| self.statistics.palette.get(v))
            .copied()
        else {
            return Ok(());
        };
        let mons = self.data.def(b"mons", entry.mons)?;
        let (vitality, cost) = (i32::from(mons.i16(0x128)), i32::from(mons.i16(0x19e)));
        let class = mons.i16(0x146);
        if vitality == 0 {
            return Err(crate::Stop::new(
                draw_site::SCORING_VITALITY_ZERO,
                format!("mons {}: scoring a definition of vitality 0", entry.mons),
            ));
        }
        let scaled = amount
            .wrapping_shl(8)
            .wrapping_div(vitality)
            .wrapping_mul(cost);
        let teams = self.statistics_teams()?;
        let scored = self
            .statistics
            .score(attacker, victim, scaled, killed, &teams);
        if let Some((victim_player, same_team)) = scored
            && casualty_sound(killed, victim_player, self.local_player(), class, same_team)
        {
            self.game_sound(draw_site::CASUALTY_SOUND, GAME_SOUND_CASUALTY);
        }
        Ok(())
    }

    pub fn mons_order(&mut self) -> Result<Vec<FourCC>, crate::Stop> {
        let units: Vec<FourCC> = self
            .data
            .mesh
            .palettes
            .iter()
            .filter(|p| p.marker_type == crate::setup::UNIT)
            .map(|p| p.tag_id)
            .collect();
        let mut order: Vec<FourCC> = Vec::new();
        for u in units {
            if let Some(m) = self.data.def(b"unit", u)?.tag(0)
                && !order.contains(&m)
            {
                order.push(m);
            }
        }
        for p in &self.statistics.palette {
            if !order.contains(&p.mons) {
                order.push(p.mons);
            }
        }
        Ok(order)
    }

    pub(crate) fn note_definitions(&mut self) -> Result<(), crate::Stop> {
        let order = self.mons_order()?;
        let mut defs = Vec::with_capacity(order.len());
        for m in order {
            defs.push((m, self.data.def(b"mons", m)?.u32(0)));
        }
        self.statistics.definitions = defs;
        Ok(())
    }

    pub fn epilogue(&self) -> Result<Option<Epilogue>, crate::Stop> {
        let defs = &self.statistics.definitions;
        let order: Vec<FourCC> = defs.iter().map(|d| d.0).collect();
        let f = |m: FourCC| defs.iter().find(|d| d.0 == m).map_or(0, |d| d.1);
        let teams = self.statistics_teams()?;
        let local = self.local_team()?;
        let Some(mut e) = self.statistics.epilogue(&order, &f, &teams, local) else {
            return Ok(None);
        };
        let reason = self.map_actions.game.end_reason;
        if reason == 1 || reason == 2 {
            let list = self.map_end_initializers()?;
            if list.len() > 1 {
                e.heroes = list
                    .iter()
                    .filter(|v| {
                        defs.iter().any(|d| d.0 == v.mons)
                            && i32::from(v.field_11 as i8) + i32::from(v.experience) > 5
                    })
                    .take(4)
                    .map(|v| EpilogueHero {
                        mons: v.mons,
                        name: v.name.clone(),
                        name_index: v.field_13,
                        battles: v.field_11 as i8,
                        kills: v.experience,
                    })
                    .collect();
            }
        }
        Ok(Some(e))
    }

    pub fn surviving_initializers(&self) -> Result<Vec<MonsterInitializer>, crate::Stop> {
        let players = &self.data.film.header.players;
        let mut list: Vec<MonsterInitializer> = Vec::new();
        for m in &self.monsters {
            if m.identifier == 0 || m.action == 13 || m.player == -1 {
                continue;
            }
            let Some(p) = players.get(m.player as u8 as usize) else {
                return Err(crate::Stop::new(
                    draw_site::SURVIVING_PLAYER_MISSING,
                    format!("a monster of player {}, who isn't there", m.player),
                ));
            };
            if p.field_02 != 0 {
                continue;
            }
            let coloured = self.unit_copies.iter().any(|c| {
                c.unit == m.unit && c.player == m.player as u8 as usize && c.permutation.is_some()
            });
            let permutation = if coloured {
                0
            } else {
                self.objects
                    .get(m.object)
                    .map_or(0, |o| o.permutation_index) as i8
            };
            let mut name = m.name.clone();
            name.truncate(0x11);
            list.push(MonsterInitializer {
                used: false,
                player: u32::from(p.net_id as u16),
                unit: m.unit,
                mons: m.definition,
                experience: m.experience,
                field_11: m.field_02.wrapping_add(1),
                permutation,
                field_13: m.name_index,
                name,
            });
            if list.len() >= 300 {
                break;
            }
        }
        let mut order: Vec<usize> = (0..list.len()).collect();
        qsort(&mut order, |&a, &b| {
            i32::from(list[b].experience) - i32::from(list[a].experience)
        });
        let mut slots: Vec<Option<MonsterInitializer>> = list.into_iter().map(Some).collect();
        Ok(order
            .into_iter()
            .map(|k| slots[k].take().expect("unique index"))
            .collect())
    }

    pub fn map_end_initializers(&self) -> Result<Vec<MonsterInitializer>, crate::Stop> {
        let claims = self.team_claims()?.claims;
        let person = usize::try_from(self.winning_team)
            .ok()
            .and_then(|t| claims.get(t).copied().flatten())
            .and_then(|p| self.data.film.header.players.get(p))
            .is_some_and(|p| p.field_02 == 0);
        if self.game_options & COOPERATIVE == 0 || person {
            self.surviving_initializers()
        } else {
            Ok(self.monster_initializers.clone())
        }
    }

    pub fn finish_map(&mut self) -> Result<(), crate::Stop> {
        self.monster_initializers = self.map_end_initializers()?;
        Ok(())
    }

    pub fn monster_initializers(&self) -> &[MonsterInitializer] {
        &self.monster_initializers
    }
}

impl crate::TflSim {
    pub(crate) fn healer_palette(&self, healer: i16) -> i16 {
        usize::try_from(healer)
            .ok()
            .and_then(|h| self.monsters.get(h))
            .map_or(-1, |m| m.palette)
    }
}

pub fn encode_monster_initializers(list: &[MonsterInitializer]) -> Vec<u8> {
    let mut out = Vec::with_capacity(list.len() * MonsterInitializer::SIZE);
    for v in list {
        let mut r = [0u8; MonsterInitializer::SIZE];
        r[0..4].copy_from_slice(&u32::from(v.used).to_be_bytes());
        r[4..8].copy_from_slice(&v.player.to_be_bytes());
        r[8..12].copy_from_slice(&v.unit.0);
        r[0xc..0x10].copy_from_slice(&v.mons.0);
        r[0x10] = v.experience as u8;
        r[0x11] = v.field_11;
        r[0x12] = v.permutation as u8;
        r[0x13] = v.field_13;
        let n = v.name.len().min(MonsterInitializer::SIZE - 0x16 - 1);
        r[0x16..0x16 + n].copy_from_slice(&v.name[..n]);
        out.extend_from_slice(&r);
    }
    out
}

pub const GAME_SOUND_CASUALTY: i16 = 7;

pub fn casualty_sound(
    killed: bool,
    victim_player: i16,
    local_player: i16,
    class: i16,
    same_team: bool,
) -> bool {
    killed && victim_player == local_player && (class != 2 || !same_team)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_teams(options: u32) -> (Statistics, Teams) {
        let mut s = Statistics::default();
        let teams = Teams {
            player_teams: vec![0, 1],
            teams: 2,
            game_type: 11,
            options,
            claims: vec![Some(0), Some(1)],
            tick: 61,
            ended: false,
        };
        s.set_up_teams(&teams, &[0, 0], &[]);
        let m = FourCC(*b"warr");
        let a = s.entry(m, 0, m);
        let b = s.entry(m, 1, m);
        s.add_monsters(a, 3, 300);
        s.add_monsters(b, 2, 200);
        s.score(a, b, 100, true, &teams);
        s.score(a, b, 100, true, &teams);
        (s, teams)
    }

    #[test]
    fn a_local_kill_is_a_casualty_unless_class_2_by_its_own_team() {
        assert!(casualty_sound(true, 0, 0, 0, false));
        assert!(casualty_sound(true, 0, 0, 0, true));
        assert!(casualty_sound(true, 0, 0, 1, true));
        assert!(casualty_sound(true, 0, 0, 2, false));
        assert!(!casualty_sound(true, 0, 0, 2, true));
        assert!(!casualty_sound(false, 0, 0, 0, false));
        assert!(!casualty_sound(true, 1, 0, 0, false));
        assert!(casualty_sound(true, -1, -1, 0, false));
        let (mut s, teams) = two_teams(0x20);
        let m = FourCC(*b"warr");
        let (a, b) = (s.entry(m, 0, m), s.entry(m, 1, m));
        assert_eq!(s.score(a, b, 1, true, &teams), Some((1, false)));
        assert_eq!(s.score(b, b, 1, true, &teams), Some((1, true)));
        assert_eq!(s.score(-1, b, 1, true, &teams), Some((1, false)));
        assert_eq!(s.score(a, 99, 1, true, &teams), None);
    }

    #[test]
    fn a_team_with_no_survivors_is_eliminated_and_the_other_wins() {
        let (mut s, teams) = two_teams(0x20);
        let mut winner = -1;
        let out = s.update(&teams, &[1, 1], &mut winner, None).unwrap();
        assert_eq!(
            out,
            RankingOutcome {
                eliminated: vec![(1, 9)],
                end: true,
                late: Vec::new(),
            }
        );
        assert_eq!(winner, 0);
        assert_eq!(s.team_data[1].flags, 1);
        let order: Vec<(i16, i16)> = s.teams.iter().map(|t| (t.team, t.rank)).collect();
        assert_eq!(order, [(0, 1), (1, 2)]);
        s.countdown = 0;
        let out = s.update(&teams, &[1, 1], &mut winner, None).unwrap();
        assert!(out.eliminated.is_empty());
    }

    #[test]
    fn one_team_eliminated_ends_the_game() {
        let mut s = Statistics::default();
        let mut teams = Teams {
            player_teams: vec![0],
            teams: 2,
            game_type: 0,
            options: 0xe024,
            claims: vec![Some(0), None],
            tick: 61,
            ended: false,
        };
        s.set_up_teams(&teams, &[0], &[]);
        assert_eq!(s.teams_with_players, 1);
        let m = FourCC(*b"warr");
        let a = s.entry(m, 0, m);
        s.add_monsters(a, 1, 100);
        let mut winner = -1;
        s.countdown = 0;
        let out = s.update(&teams, &[1], &mut winner, None).unwrap();
        assert_eq!(out, RankingOutcome::default());
        s.score(-1, a, 100, true, &teams);
        s.countdown = 0;
        let out = s.update(&teams, &[1], &mut winner, None).unwrap();
        assert_eq!(
            out,
            RankingOutcome {
                eliminated: vec![(0, 9)],
                end: true,
                late: Vec::new(),
            }
        );
        assert_eq!(winner, -1);
        assert_eq!(s.team_data[0].flags, 1);
        teams.ended = true;
        s.countdown = 0;
        let out = s.update(&teams, &[1], &mut winner, None).unwrap();
        assert_eq!(out, RankingOutcome::default());
    }

    #[test]
    fn two_teams_wiped_together_end_the_game_with_no_winner() {
        let (mut s, teams) = two_teams(0x20);
        let m = FourCC(*b"warr");
        let a = s.entry(m, 0, m);
        for _ in 0..3 {
            s.score(-1, a, 100, true, &teams);
        }
        let mut winner = -1;
        let out = s.update(&teams, &[1, 1], &mut winner, None).unwrap();
        assert_eq!(
            out,
            RankingOutcome {
                eliminated: vec![(0, 9), (1, 9)],
                end: true,
                late: Vec::new(),
            }
        );
        assert_eq!(winner, -1);
    }

    #[test]
    fn option_0x80_eliminates_nobody() {
        let (mut s, teams) = two_teams(0x80);
        let mut winner = -1;
        let out = s.update(&teams, &[1, 1], &mut winner, None).unwrap();
        assert_eq!(out, RankingOutcome::default());
        assert_eq!(winner, -1);
    }

    #[test]
    fn permanent_rankings_take_the_places() {
        let (mut s, teams) = two_teams(0x80);
        let mut winner = -1;
        s.update(&teams, &[1, 1], &mut winner, None).unwrap();
        s.record_permanent_rankings(&teams.player_teams);
        let p: Vec<(i16, i32, [i16; 3])> = s
            .permanent
            .iter()
            .map(|r| (r.player, r.points, r.places))
            .collect();
        assert_eq!(p, [(0, 0x8000, [0, 1, 0]), (1, 0x5555, [0, 0, 1])]);
    }

    #[test]
    fn permanent_rankings_carry_to_the_next_map() {
        let (mut s, teams) = two_teams(0x80);
        let mut winner = -1;
        s.update(&teams, &[1, 1], &mut winner, None).unwrap();
        s.record_permanent_rankings(&teams.player_teams);
        let previous = vec![PermanentRanking {
            id: 7,
            ..s.permanent[0]
        }];
        let mut next = Statistics::default();
        next.set_up_teams(&teams, &[3, 7], &[]);
        next.carry_permanent(&previous);
        assert_eq!(
            next.permanent[0],
            PermanentRanking {
                player: 0,
                id: 3,
                ..PermanentRanking::default()
            }
        );
        assert_eq!(
            next.permanent[1],
            PermanentRanking {
                player: 1,
                ..previous[0]
            }
        );
        let mut winner = -1;
        next.update(&teams, &[1, 1], &mut winner, None).unwrap();
        next.record_permanent_rankings(&teams.player_teams);
        let carried = next.permanent.iter().find(|r| r.id == 7).unwrap();
        let place = next.teams.iter().find(|t| t.team == 1).unwrap().rank;
        assert_eq!(
            carried.points,
            0x8000 + (0x10000 / (i64::from(place) + 1)) as i32
        );
        assert_eq!(carried.places.iter().sum::<i16>(), 2);
    }
}
