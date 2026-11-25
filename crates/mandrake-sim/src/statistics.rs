use mandrake_common::FourCC;
use mandrake_common::game_options;

use crate::engine_rng::{EngineWorld, TagSource};
use crate::game_state::{FLAG_GAME_OVER, GAME_SOUND_ABOUT_TO_WIN, GAME_SOUND_CASUALTY};
use crate::players::{Player, Players};

#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct UnitRef(pub i16);

impl UnitRef {
    pub const NONE: Self = Self(-1);

    pub fn is_none(self) -> bool {
        self.0 == -1
    }
}

impl std::fmt::Debug for UnitRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&if self.is_none() { -1i16 } else { 0 }, f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitStatistics {
    pub total: i32,
    pub total_hit_points: i32,
    pub alive: i32,
    pub hit_points_and_damage: i32,
    pub uncontrollable: i32,
    pub kills: i32,
    pub damage_dealt: i32,
    pub damage_taken: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitCosts {
    pub cost: i16,
    pub class: i16,
    pub max_hit_points: i16,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitRow {
    pub tag: u32,
    pub owner: i16,
    pub custom: bool,
    pub mons: i16,
    pub mons_tag: u32,
    pub costs: Option<UnitCosts>,
    pub totals: UnitStatistics,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerSample {
    pub own_damage: i32,
    pub score: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TeamSample {
    pub kills: i32,
    pub damage: i32,
    pub killed_by_local_team: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatisticsSample {
    pub players: Vec<PlayerSample>,
    pub teams: Vec<TeamSample>,
    pub no_player_score: i32,
}

impl StatisticsSample {
    fn new(players: usize, teams: usize) -> Self {
        Self {
            players: vec![PlayerSample::default(); players],
            teams: vec![TeamSample::default(); teams],
            no_player_score: 0,
        }
    }

    fn clear(&mut self) {
        self.players.fill(PlayerSample::default());
        self.teams.fill(TeamSample::default());
        self.no_player_score = 0;
    }

    fn plus(&self, other: &Self) -> Self {
        let mut sum = self.clone();
        for (o, b) in sum.players.iter_mut().zip(&other.players) {
            o.own_damage = o.own_damage.wrapping_add(b.own_damage);
            o.score = o.score.wrapping_add(b.score);
        }
        for (o, b) in sum.teams.iter_mut().zip(&other.teams) {
            o.kills = o.kills.wrapping_add(b.kills);
            o.damage = o.damage.wrapping_add(b.damage);
            o.killed_by_local_team = o.killed_by_local_team.wrapping_add(b.killed_by_local_team);
        }
        sum.no_player_score = sum.no_player_score.wrapping_add(other.no_player_score);
        sum
    }

    fn add_score(&mut self, player: i16, amount: i32) {
        if player == -1 {
            self.no_player_score = self.no_player_score.wrapping_add(amount);
        } else if let Some(p) = slot(&mut self.players, player) {
            p.score = p.score.wrapping_add(amount);
        }
    }

    fn add_damage(
        &mut self,
        victim: (i16, i16),
        attacker: (i16, i16),
        amount: i32,
        killed: bool,
        local_team: i16,
    ) {
        let (victim_player, victim_team) = victim;
        let (attacker_player, attacker_team) = attacker;
        if victim_player == attacker_player
            && let Some(p) = slot(&mut self.players, victim_player)
        {
            p.own_damage = p.own_damage.wrapping_add(amount);
        }
        if attacker_team == -1 {
            return;
        }
        if attacker_team != victim_team
            && let Some(t) = slot(&mut self.teams, attacker_team)
        {
            t.damage = t.damage.wrapping_add(amount);
            if killed {
                t.kills += 1;
            }
        }
        if killed
            && attacker_team == local_team
            && let Some(t) = slot(&mut self.teams, victim_team)
        {
            t.killed_by_local_team += 1;
        }
    }
}

fn slot<T>(v: &mut [T], i: i16) -> Option<&mut T> {
    v.get_mut(usize::try_from(i).ok()?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerRanking {
    pub player: i16,
    pub parent: i16,
    pub losses: i32,
    pub losses_cost: i32,
    pub survivors: i32,
    pub survivors_cost: i32,
    pub uncontrollable: i32,
    pub kills: i32,
    pub damage_dealt: i32,
    pub damage_taken: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TeamRanking {
    pub team: i16,
    pub rank: i16,
    pub losses: i32,
    pub losses_cost: i32,
    pub survivors: i32,
    pub survivors_cost: i32,
    pub uncontrollable: i32,
    pub kills: i32,
    pub damage_dealt: i32,
    pub damage_taken: i32,
    pub score: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PermanentRanking {
    pub player: i16,
    pub id: i16,
    pub points: i32,
    pub losses: i32,
    pub kills: i32,
    pub damage_dealt: i32,
    pub places: [i16; 3],
    pub parent: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamScore {
    pub field_04: i32,
    pub flag_time: i32,
    pub field_0c: i32,
    pub score_tick: i32,
    pub best_tick: i32,
    pub level_tick: i32,
    pub numbers_taken: u32,
    pub taken_count: i16,
    pub flags_held: i16,
    pub best: i16,
    pub score: i16,
}

impl Default for TeamScore {
    fn default() -> Self {
        Self {
            field_04: -1,
            flag_time: 0,
            field_0c: -1,
            score_tick: 0,
            best_tick: 0,
            level_tick: 0,
            numbers_taken: 0,
            taken_count: 0,
            flags_held: 0,
            best: 0,
            score: 0,
        }
    }
}

pub const SAMPLES: usize = 20;

#[derive(Debug, Clone, Copy)]
pub struct SampleRange<'a> {
    pub count: i32,
    pub period: i32,
    pub tick: i32,
    pub samples: &'a [StatisticsSample],
}

#[derive(Debug, Clone, Default)]
pub struct Statistics {
    pub sample_period: i16,
    pub sample_tick: i16,
    pub sample: i16,
    pub countdown: i16,
    pub rebuilds: i16,
    pub seen_flagged_monster: bool,
    pub custom_scoring: bool,
    pub finished: bool,
    pub samples: Vec<StatisticsSample>,
    pub totals: StatisticsSample,
    pub units: Vec<UnitRow>,
    free_units: i32,
    pub player_rankings: Vec<PlayerRanking>,
    pub team_rankings: Vec<TeamRanking>,
    pub permanent_rankings: Vec<PermanentRanking>,
    pub team_scores: Vec<TeamScore>,
    pub mons: Vec<u32>,
    monster_unit: Vec<i16>,
    palette_unit: Vec<i16>,
    clock: i32,
    pub trace: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy)]
pub struct PaletteUnit {
    pub tag: u32,
    pub mons: Option<u32>,
    pub flags: u16,
    pub team: i16,
}

impl Statistics {
    pub fn new_map(players: &Players, carried: &[PermanentRanking]) -> Self {
        let n = players.players.len();
        let t = players.teams.len();
        Self {
            sample_period: 300,
            sample_tick: 0,
            sample: 1,
            countdown: -1,
            samples: vec![StatisticsSample::new(n, t); SAMPLES],
            totals: StatisticsSample::new(n, t),
            player_rankings: (0..n)
                .map(|i| PlayerRanking {
                    player: i as i16,
                    parent: -1,
                    ..Default::default()
                })
                .collect(),
            team_rankings: vec![TeamRanking::default(); t],
            permanent_rankings: players
                .players
                .iter()
                .enumerate()
                .map(|(i, p)| PermanentRanking {
                    player: i as i16,
                    id: p.id as i16,
                    parent: -1,
                    ..carried
                        .iter()
                        .find(|c| c.id == p.id as i16)
                        .copied()
                        .unwrap_or_default()
                })
                .collect(),
            team_scores: vec![TeamScore::default(); t],
            ..Default::default()
        }
    }

    pub(crate) fn tag_row(&mut self, tag: u32, mons: Option<u32>) -> i16 {
        if let Some(i) = self.own_row(tag) {
            return i as i16;
        }
        let index = mons.map_or(-1, |m| {
            let at = self.mons.iter().position(|&x| x == m).unwrap_or_else(|| {
                self.mons.push(m);
                self.mons.len() - 1
            });
            at as i16
        });
        self.new_row(UnitRow {
            tag,
            owner: -1,
            custom: false,
            mons: index,
            mons_tag: mons.unwrap_or(0),
            costs: None,
            totals: UnitStatistics::default(),
            deleted: false,
        })
    }

    fn own_row(&self, tag: u32) -> Option<usize> {
        self.units
            .iter()
            .position(|u| u.tag == tag && !u.custom && !u.deleted)
    }

    fn new_row(&mut self, row: UnitRow) -> i16 {
        if self.free_units > 0
            && let Some(i) = self.units.iter().position(|u| u.deleted)
        {
            let totals = self.units[i].totals;
            self.units[i] = UnitRow { totals, ..row };
            self.free_units -= 1;
            return i as i16;
        }
        self.units.push(row);
        self.units.len() as i16 - 1
    }

    pub fn find_unit(&self, player: i16, tag: u32) -> Option<usize> {
        self.units
            .iter()
            .position(|u| u.owner == player && u.tag == tag && !u.deleted)
    }

    pub(crate) fn delete_player_rows(&mut self, player: i16, gameplay: i32) {
        for i in (0..self.units.len()).rev() {
            let u = self.units[i];
            if u.owner != player || !(u.custom || gameplay < 0x71e) || self.own_row(u.tag).is_none()
            {
                continue;
            }
            self.units[i].totals = UnitStatistics::default();
            if i == self.units.len() - 1 {
                self.units.pop();
            } else if !u.deleted {
                self.units[i].deleted = true;
                self.free_units += 1;
            }
        }
    }

    pub(crate) fn player_row(&mut self, tag: u32, player: i16) -> i16 {
        if let Some(i) = self.find_unit(player, tag) {
            return i as i16;
        }
        let own = self.tag_row(tag, None);
        let (mons, mons_tag) = (
            self.units[own as usize].mons,
            self.units[own as usize].mons_tag,
        );
        self.new_row(UnitRow {
            tag,
            owner: player,
            custom: true,
            mons,
            mons_tag,
            costs: None,
            totals: UnitStatistics::default(),
            deleted: false,
        })
    }

    pub(crate) fn new_palette_entries(
        &mut self,
        entries: impl Iterator<Item = PaletteUnit>,
        players: &Players,
        game_type: i16,
    ) {
        for p in entries {
            let mut unit = -1;
            if p.mons.is_none() {
            } else if p.flags & 0x10 != 0 {
                let i = self.tag_row(p.tag, p.mons);
                self.units[i as usize].owner = 0;
                unit = i;
            } else if p.team != -1 {
                let own = self.tag_row(p.tag, p.mons);
                for (j, pl) in players.players.iter().enumerate() {
                    if i16::from(pl.team) != p.team {
                        continue;
                    }
                    let row = self.player_row(p.tag, j as i16);
                    let claimed = players
                        .teams
                        .get(p.team as usize)
                        .is_some_and(|t| t.player == j as i16);
                    if claimed {
                        unit = if p.flags & 1 != 0 && game_type == 13 {
                            own
                        } else {
                            row
                        };
                    }
                }
            }
            self.palette_unit.push(unit);
        }
    }

    pub(crate) fn palette_row(
        &mut self,
        palette: usize,
        tag: u32,
        owner: i16,
        entry_owner: i16,
    ) -> i16 {
        if owner != entry_owner {
            return self.player_row(tag, owner);
        }
        self.palette_unit.get(palette).copied().unwrap_or(-1)
    }

    pub(crate) fn palette_copy_row(
        &mut self,
        copy: PaletteUnit,
        players: &Players,
        game_type: i16,
    ) -> i16 {
        self.new_palette_entries(std::iter::once(copy), players, game_type);
        self.palette_unit.pop().unwrap_or(-1)
    }

    pub(crate) fn unit_row_owner(&self, row: i16) -> i16 {
        usize::try_from(row)
            .ok()
            .and_then(|r| self.units.get(r))
            .map_or(-1, |u| u.owner)
    }

    pub(crate) fn monster_created(
        &mut self,
        monster: usize,
        unit: i16,
        costs: UnitCosts,
        hit_points: i16,
        object_flags: u32,
    ) {
        if self.monster_unit.len() <= monster {
            self.monster_unit.resize(monster + 1, -1);
        }
        self.monster_unit[monster] = unit;
        if let Some(row) = slot(&mut self.units, unit) {
            row.costs = Some(costs);
        }
        if object_flags & 0x200 == 0 {
            self.adjust_unit_statistics(unit, 1, i32::from(hit_points));
        }
    }

    pub(crate) fn monster_detached(&mut self, monster: usize, row: i16, hit_points: i32) {
        let old = self.monster_unit(monster);
        self.adjust_unit_statistics(old, -1, -hit_points);
        self.adjust_unit_statistics(row, 1, hit_points);
        let costs = slot(&mut self.units, old).and_then(|u| u.costs);
        if let Some(new) = slot(&mut self.units, row)
            && new.costs.is_none()
        {
            new.costs = costs;
        }
        self.monster_unit[monster] = row;
    }

    pub fn monster_unit(&self, monster: usize) -> i16 {
        self.monster_unit.get(monster).copied().unwrap_or(-1)
    }

    pub fn adjust_unit_statistics(&mut self, unit: i16, count: i32, hit_points: i32) {
        if let Some(t) = self.trace.as_mut() {
            t.push(format!("{} U {unit} {count} {hit_points}", self.clock));
        }
        if let Some(row) = slot(&mut self.units, unit) {
            let u = &mut row.totals;
            u.total = u.total.wrapping_add(count);
            u.total_hit_points = u.total_hit_points.wrapping_add(hit_points);
            u.alive = u.alive.wrapping_add(count);
            u.hit_points_and_damage = u.hit_points_and_damage.wrapping_add(hit_points);
        }
    }

    pub fn adjust_unit_statistics_for_uncontrollability(&mut self, unit: i16, count: i32) {
        if let Some(t) = self.trace.as_mut() {
            t.push(format!("{} C {unit} {count}", self.clock));
        }
        if let Some(row) = slot(&mut self.units, unit) {
            row.totals.uncontrollable = row.totals.uncontrollable.wrapping_add(count);
        }
    }

    pub fn adjust_player_score(&mut self, player: i16, amount: i32, game_flags: u32) {
        if game_flags & FLAG_GAME_OVER != 0 {
            return;
        }
        if let Some(s) = self.current_sample() {
            s.add_score(player, amount);
        }
        self.totals.add_score(player, amount);
    }

    fn current_sample(&mut self) -> Option<&mut StatisticsSample> {
        slot(&mut self.samples, self.sample - 1)
    }

    pub fn sample_range(&self) -> SampleRange<'_> {
        SampleRange {
            count: i32::from(self.sample),
            period: i32::from(self.sample_period),
            tick: i32::from(self.sample_tick),
            samples: &self.samples,
        }
    }

    fn sample_rollover(&mut self) {
        self.sample += 1;
        if self.sample > SAMPLES as i16 {
            for i in 0..self.samples.len() / 2 {
                self.samples[i] = self.samples[i * 2].plus(&self.samples[i * 2 + 1]);
            }
            self.sample_period <<= 1;
            self.sample = 0xb;
        }
        if let Some(s) = self.current_sample() {
            s.clear();
        }
        self.sample_tick = 0;
    }
}

#[derive(Clone)]
struct RebuildContext<'a> {
    players: &'a Players,
    game_type: i16,
    tick: i32,
    winner: i16,
    state: i16,
    timer: i32,
    options: u32,
    game_over: bool,
    flags: crate::flags::FlagCount,
    unnumbered: i32,
    numbered: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GameAction {
    Eliminate(i16),
    Suspend(bool),
    AboutToWin(i16),
    EndGame,
    BreakAlliances,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FlaggedMonsters {
    teams: Vec<i32>,
    flagged: i32,
    with_0x2000: i32,
}

impl RebuildContext<'_> {
    fn player(&self, player: i16) -> Option<&Player> {
        self.players.players.get(usize::try_from(player).ok()?)
    }

    fn team_of(&self, player: i16) -> i16 {
        self.player(player).map_or(-1, |p| i16::from(p.team))
    }
}

pub fn qsort_private<T: Copy>(v: &mut [T], mut compare: impl FnMut(&T, &T) -> i32) {
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

fn compare_players(ctx: &RebuildContext, a: &PlayerRanking, b: &PlayerRanking) -> i32 {
    let (ta, tb) = (ctx.team_of(a.player), ctx.team_of(b.player));
    if ta == -1 {
        return i32::from(tb != -1);
    }
    if tb == -1 {
        return -1;
    }
    let claimed = |team: i16| {
        ctx.players
            .teams
            .get(team as usize)
            .map_or(0, |t: &crate::players::Team| t.player)
    };
    match (a.player == claimed(ta), b.player == claimed(tb)) {
        (true, false) => return -1,
        (false, true) => return 1,
        _ => {}
    }
    match b.kills.wrapping_sub(a.kills) {
        0 => b.damage_dealt.wrapping_sub(a.damage_dealt),
        d => d,
    }
}

fn compare_teams(
    ctx: &RebuildContext,
    scores: &[TeamScore],
    a: &TeamRanking,
    b: &TeamRanking,
) -> i32 {
    let (sa, sb) = (
        scores.get(a.team as usize).copied().unwrap_or_default(),
        scores.get(b.team as usize).copied().unwrap_or_default(),
    );
    let players = |t: i16| ctx.players.teams.get(t as usize).map_or(0, |t| t.players);
    let result = if players(a.team) == 0 {
        i32::from(players(b.team) != 0)
    } else if players(b.team) == 0 {
        -1
    } else {
        match b.score.wrapping_sub(a.score) {
            0 => match ctx.game_type {
                0 | 12 => sa.best_tick.wrapping_sub(sb.best_tick),
                1 | 2 => sb.field_0c.wrapping_sub(sa.field_0c),
                3 | 4 => sa.field_0c.wrapping_sub(sb.field_0c),
                7 | 8 => {
                    let d = i32::from(sb.best) - i32::from(sa.best);
                    if d != 0 {
                        d
                    } else {
                        match sb.level_tick.wrapping_sub(sa.level_tick) {
                            0 => sa.best_tick.wrapping_sub(sb.best_tick),
                            d => d,
                        }
                    }
                }
                10 | 11 => sa.score_tick.wrapping_sub(sb.score_tick),
                _ => 0,
            },
            d => d,
        }
    };
    if ctx.winner != -1 {
        if a.team == ctx.winner {
            return if b.team != a.team { -1 } else { result };
        }
        if b.team == ctx.winner {
            return 1;
        }
    }
    result
}

impl Statistics {
    fn rebuild(
        &mut self,
        mut ctx: RebuildContext,
        flagged: &FlaggedMonsters,
    ) -> (Vec<GameAction>, i16) {
        let players = ctx.players;
        let n = players.players.len();
        let mut rows = vec![PlayerRanking::default(); n];
        for u in self.units.iter().filter(|u| !u.deleted) {
            let (Ok(owner), Some(costs)) = (usize::try_from(u.owner), u.costs) else {
                continue;
            };
            if (costs.class.wrapping_sub(5) as u16) < 2 || owner >= n {
                continue;
            }
            let cost = i32::from(costs.cost);
            let t = u.totals;
            let r = &mut rows[owner];
            let team = players.players[owner].team;
            let out = usize::try_from(team)
                .ok()
                .and_then(|i| players.teams.get(i))
                .is_none_or(|t| t.flags & 1 != 0);
            if !out {
                r.survivors = r.survivors.wrapping_add(t.alive);
                r.survivors_cost = r.survivors_cost.wrapping_add(cost.wrapping_mul(t.alive));
                r.uncontrollable = r.uncontrollable.wrapping_add(t.uncontrollable);
            }
            let lost = t.total.wrapping_sub(t.alive);
            r.kills = r.kills.wrapping_add(t.kills);
            r.losses = r.losses.wrapping_add(lost);
            r.losses_cost = r.losses_cost.wrapping_add(cost.wrapping_mul(lost));
            r.damage_dealt = r.damage_dealt.wrapping_add(t.damage_dealt);
            r.damage_taken = r.damage_taken.wrapping_add(t.damage_taken);
        }
        for (i, s) in self.team_scores.iter_mut().enumerate() {
            let held = ctx.flags.held.get(i).copied().unwrap_or(0);
            s.flags_held = held;
            if ctx.state != 0 && held != 0 {
                let time = if ctx.game_type == 15 {
                    i32::from(held) * 15
                } else {
                    15
                };
                s.flag_time = s.flag_time.wrapping_add(time);
            }
            if held > s.best {
                s.best = held;
                s.best_tick = ctx.tick;
            }
            if held >= s.best {
                s.level_tick = ctx.tick;
            }
            self.team_rankings[i] = TeamRanking {
                team: i as i16,
                score: i32::from(s.score),
                ..Default::default()
            };
        }
        let (mut alive, mut last_team) = (0u32, -1i16);
        let mut slot_index = 0;
        while slot_index < n {
            let p = self.player_rankings[slot_index].player;
            let flags = ctx.player(p).map_or(0, |pl| pl.flags);
            if flags & 0x44 == 0 || p as usize == slot_index {
                slot_index += 1;
                continue;
            }
            self.player_rankings.swap(slot_index, p as usize);
            let moved = &mut self.player_rankings[p as usize];
            moved.survivors = 0;
            moved.survivors_cost = 0;
            moved.uncontrollable = 0;
        }
        for (i, pl) in players.players.iter().enumerate() {
            if pl.flags & 0x44 == 0 {
                let own_damage = self.totals.players.get(i).map_or(0, |p| p.own_damage);
                let mut r = rows[i];
                r.player = i as i16;
                r.parent = -1;
                r.damage_dealt = r.damage_dealt.wrapping_sub(own_damage);
                self.player_rankings[i] = r;
            }
            if pl.team != -1 || pl.field_0b != -1 {
                let r = self.player_rankings[i];
                if let Some(t) = slot(&mut self.team_rankings, i16::from(pl.field_0b)) {
                    t.losses = t.losses.wrapping_add(r.losses);
                    t.losses_cost = t.losses_cost.wrapping_add(r.losses_cost);
                    t.survivors = t.survivors.wrapping_add(r.survivors);
                    t.survivors_cost = t.survivors_cost.wrapping_add(r.survivors_cost);
                    t.uncontrollable = t.uncontrollable.wrapping_add(r.uncontrollable);
                    t.kills = t.kills.wrapping_add(r.kills);
                    t.damage_dealt = t.damage_dealt.wrapping_add(r.damage_dealt);
                    t.damage_taken = t.damage_taken.wrapping_add(r.damage_taken);
                }
                if pl.team != -1
                    && (r.survivors.wrapping_add(r.uncontrollable) != 0
                        || ctx.options & game_options::DEATHMATCH != 0)
                    && players
                        .teams
                        .get(pl.field_0b as usize)
                        .is_some_and(|t| t.flags & 1 == 0)
                {
                    alive |= 1 << (pl.field_0b as u32 & 0x1f);
                    if last_team == -1 {
                        last_team = i16::from(pl.field_0b);
                    }
                }
            }
        }
        let mut actions = Vec::new();
        let mut alive_teams = 0;
        for (t, team) in players.teams.iter().enumerate() {
            if team.players == 0 {
                continue;
            }
            if alive & (1 << (t as u32 & 0x1f)) != 0 {
                alive_teams += 1;
            } else if ctx.options & game_options::COOPERATIVE == 0 {
                actions.push(GameAction::Eliminate(t as i16));
            }
        }
        for (i, s) in self.team_scores.iter_mut().enumerate() {
            let held = i32::from(s.flags_held);
            let r = &mut self.team_rankings[i];
            if matches!(ctx.game_type, 0 | 12) && (i32::from(s.best) as u32) < (r.score as u32) {
                s.best = r.score as i16;
                s.best_tick = ctx.tick;
            }
            match ctx.game_type {
                0 => r.score = r.damage_dealt,
                1 | 2 | 5..=8 => r.score = held,
                3 | 4 => r.score = i32::from(s.taken_count),
                9 | 14 | 15 => r.score = s.flag_time,
                13 if !self.custom_scoring => r.score = held,
                _ => {}
            }
        }
        if flagged.flagged != 0 {
            self.seen_flagged_monster = true;
        }
        if !ctx.game_over && ctx.options & game_options::COOPERATIVE == 0 && alive_teams == 2 {
            actions.push(GameAction::BreakAlliances);
        }
        let end = !ctx.game_over && self.game_ends(&mut ctx, flagged, alive_teams, last_team);
        let compare_scores = self.team_scores.clone();
        qsort_private(&mut self.player_rankings, |a, b| {
            compare_players(&ctx, a, b)
        });
        qsort_private(&mut self.team_rankings, |a, b| {
            compare_teams(&ctx, &compare_scores, a, b)
        });
        let mut rank = 0;
        if let Some(first) = self.team_rankings.first_mut() {
            first.rank = 0;
        }
        for i in 1..self.team_rankings.len() {
            let (a, b) = (self.team_rankings[i], self.team_rankings[i - 1]);
            if compare_teams(&ctx, &compare_scores, &a, &b) != 0 {
                rank += 1;
            }
            self.team_rankings[i].rank = rank;
        }
        if end {
            actions.push(GameAction::Suspend(false));
            actions.push(GameAction::EndGame);
        }
        let contested = !ctx.flags.settled;
        match ctx.game_type {
            1 | 2 => actions.push(GameAction::Suspend(contested)),
            3 | 4 => {
                actions.push(GameAction::Suspend(contested));
                for (t, s) in self.team_scores.iter().enumerate() {
                    let taken = i32::from(s.taken_count);
                    if taken - ctx.numbered == -1 {
                        actions.push(GameAction::AboutToWin(t as i16));
                    } else if taken == ctx.numbered {
                        actions.push(GameAction::EndGame);
                    }
                }
            }
            5 | 6 => {
                actions.push(GameAction::Suspend(contested));
                for (t, &held) in ctx.flags.held.iter().enumerate() {
                    if held == 0 {
                        actions.push(GameAction::Eliminate(t as i16));
                    }
                }
            }
            7 | 8 => {
                actions.push(GameAction::Suspend(contested));
                for &held in &ctx.flags.held {
                    if i32::from(held) == ctx.unnumbered {
                        if ctx.options & game_options::TFL == 0 {
                            actions.push(GameAction::Suspend(false));
                        }
                        actions.push(GameAction::EndGame);
                    }
                }
            }
            10 if flagged.flagged == 0 && self.seen_flagged_monster => {
                actions.push(GameAction::Suspend(false));
                actions.push(GameAction::EndGame);
            }
            11 => {
                for (t, &n) in flagged.teams.iter().enumerate() {
                    if n == 0 {
                        actions.push(GameAction::Eliminate(t as i16));
                    }
                }
            }
            12 if flagged.with_0x2000 == 0 => {
                actions.push(GameAction::Suspend(false));
                actions.push(GameAction::EndGame);
            }
            _ => {}
        }
        (actions, ctx.winner)
    }

    fn game_ends(
        &mut self,
        ctx: &mut RebuildContext,
        flagged: &FlaggedMonsters,
        alive_teams: i32,
        last_team: i16,
    ) -> bool {
        let teams = ctx.players.occupied_teams;
        let last_standing = ctx.options & game_options::COOPERATIVE == 0
            && alive_teams != 2
            && match teams {
                1 => alive_teams < 1,
                t if t >= 2 => alive_teams <= 1,
                _ => false,
            };
        if last_standing {
            self.last_team_standing(ctx, flagged, last_team);
            return true;
        }
        !ctx.players.players.is_empty()
            && !ctx
                .players
                .players
                .iter()
                .any(|p| p.field_09 == 0 && p.team != -1 && p.flags & 0x44 == 0)
    }

    fn last_team_standing(
        &mut self,
        ctx: &mut RebuildContext,
        flagged: &FlaggedMonsters,
        team: i16,
    ) {
        let slot = usize::try_from(team).ok();
        let tick = ctx.tick;
        let time = |this: &mut Self, amount: i32| {
            if let Some(t) = slot {
                this.team_scores[t].flag_time = this.team_scores[t].flag_time.wrapping_add(amount);
                this.team_rankings[t].score = this.team_rankings[t].score.wrapping_add(amount);
            }
        };
        match ctx.game_type {
            0 | 11 => {}
            9 => time(self, ctx.timer),
            14 => {
                if ctx.players.occupied_teams == 2 {
                    time(self, ctx.timer);
                }
            }
            15 => time(self, ctx.timer.wrapping_mul(ctx.unnumbered)),
            10 => {
                if let Some(t) = slot {
                    let n = flagged.teams.get(t).copied().unwrap_or(0);
                    let s = &mut self.team_scores[t];
                    s.score_tick = tick;
                    s.score = s.score.wrapping_add(n as i16);
                    self.team_rankings[t].score = self.team_rankings[t].score.wrapping_add(n);
                }
            }
            12 => {
                if ctx.options & game_options::TFL == 0
                    && let Some(t) = slot
                {
                    let n = flagged.with_0x2000;
                    let s = &mut self.team_scores[t];
                    s.score_tick = tick;
                    s.best_tick = tick;
                    s.score = s.score.wrapping_add(n as i16);
                    self.team_rankings[t].score = self.team_rankings[t].score.wrapping_add(n);
                }
            }
            _ => ctx.winner = team,
        }
    }

    fn finish(&mut self, players: &Players) {
        for t in self.team_rankings.clone() {
            for r in self.player_rankings.clone() {
                let Some(pl) = players.players.get(r.player as usize) else {
                    continue;
                };
                let member = i16::from(pl.team) == t.team;
                if !member && i16::from(pl.field_0b) != t.team {
                    continue;
                }
                let Some(p) = self.permanent_rankings.get_mut(r.player as usize) else {
                    continue;
                };
                if member {
                    p.points = p.points.wrapping_add(0x10000 / (i32::from(t.rank) + 1));
                }
                if let Some(place) = usize::try_from(t.rank)
                    .ok()
                    .and_then(|x| p.places.get_mut(x))
                {
                    *place += 1;
                }
                p.losses = p.losses.wrapping_add(r.losses);
                p.kills = p.kills.wrapping_add(r.kills);
                p.damage_dealt = p.damage_dealt.wrapping_add(r.damage_dealt);
                p.parent = r.parent;
            }
        }
        qsort_private(&mut self.permanent_rankings, |a, b| {
            b.points.wrapping_sub(a.points)
        });
        self.finished = true;
    }
}

impl EngineWorld {
    fn surviving_initializers(&self) -> Vec<mandrake_rec::header::MonsterInitializer> {
        let mut out = Vec::new();
        for (i, m) in self.monsters.iter().enumerate() {
            let player = usize::try_from(m.owner)
                .ok()
                .and_then(|o| self.players.players.get(o));
            let Some(player) = player.filter(|p| p.field_09 == 0) else {
                continue;
            };
            if m.identifier == 0 || m.mode == 13 {
                continue;
            }
            let c = self.combat.get(i).copied().unwrap_or_default();
            let row = usize::try_from(self.statistics.monster_unit(i))
                .ok()
                .and_then(|u| self.statistics.units.get(u));
            out.push(mandrake_rec::header::MonsterInitializer {
                player_id: player.id as u32,
                unit_tag: mandrake_common::FourCC(c.unit.0.to_be_bytes()),
                monster_tag: mandrake_common::FourCC(row.map_or(0, |r| r.mons_tag).to_be_bytes()),
                experience: c.experience,
                field_8f_plus_one: (c.battles as u8).wrapping_add(1),
                permutation: m.object.permutation as u8,
                name_index: c.name_index as u8,
                name: crate::veterans::custom_name_text(&c.custom_name),
            });
        }
        out
    }

    fn end_builds_initializers(&self) -> bool {
        self.game.command_reason == 0
            && (self.game_options & game_options::COOPERATIVE == 0
                || usize::try_from(self.game.winner)
                    .ok()
                    .and_then(|t| self.players.teams.get(t))
                    .and_then(|t| usize::try_from(t.player).ok())
                    .and_then(|p| self.players.players.get(p))
                    .is_some_and(|p| p.field_09 == 0))
    }

    pub(crate) fn build_monster_initializers_at_game_over(&mut self) {
        if self.end_builds_initializers() {
            let survivors = self.surviving_initializers();
            self.monster_initializers.build(survivors);
        }
    }

    pub(crate) fn initializers_at_game_over(&self) -> crate::veterans::MonsterInitializers {
        let mut table = self.monster_initializers.clone();
        if !self.game.game_over && self.end_builds_initializers() {
            table.build(self.surviving_initializers());
        }
        table
    }
}

impl EngineWorld {
    fn rebuild_context(&self, tick: i32, flags: crate::flags::FlagCount) -> RebuildContext<'_> {
        RebuildContext {
            players: &self.players,
            game_type: self.game_type,
            tick,
            winner: self.game.winner,
            state: self.game.state,
            timer: self.game.timer,
            options: self.setup_options,
            game_over: self.game.flags & FLAG_GAME_OVER != 0,
            flags,
            unnumbered: i32::from(self.flag_data.unnumbered),
            numbered: i32::from(self.flag_data.numbered),
        }
    }

    fn rebuild_statistics(
        &mut self,
        stats: &mut Statistics,
        tick: i32,
        mut rng: Option<&mut crate::engine_rng::EngineRng>,
    ) {
        let mode = crate::flags::flag_count_mode(self.game_type);
        let flags = match self.count_team_flags(
            &mut stats.team_scores,
            mode,
            0xf,
            tick,
            rng.as_deref_mut(),
        ) {
            Ok(flags) => flags,
            Err(e) => {
                self.statistics_stop.get_or_insert(e);
                crate::flags::FlagCount {
                    held: vec![0; self.players.teams.len()],
                    settled: true,
                    scored: Vec::new(),
                }
            }
        };
        for &m in &flags.scored {
            stats.adjust_unit_statistics(stats.monster_unit(m), -1, 0);
        }
        let flagged = self.flagged_monsters();
        let ctx = self.rebuild_context(tick, flags);
        let (actions, winner) = stats.rebuild(ctx, &flagged);
        self.game.winner = winner;
        let tied = stats.team_rankings.len() > 1
            && stats.team_rankings[0].rank == stats.team_rankings[1].rank;
        self.game.standings_leader = Some(match stats.team_rankings.first() {
            Some(top) if self.players.occupied_teams < 2 || !tied => top.team,
            _ => -1,
        });
        for action in actions {
            let r = match action {
                GameAction::Eliminate(team) => self.eliminate_team(team, rng.as_deref_mut()),
                GameAction::Suspend(s) => {
                    self.game.suspend_end_game(s);
                    Ok(())
                }
                GameAction::AboutToWin(team) => {
                    if let Some(t) = self.players.teams.get_mut(team as usize)
                        && t.flags & 2 == 0
                    {
                        t.flags |= 2;
                        self.game.game_sound_requests.push(GAME_SOUND_ABOUT_TO_WIN);
                    }
                    Ok(())
                }
                GameAction::EndGame => self.game.end_game(0, tick, &mut self.game_options),
                GameAction::BreakAlliances => {
                    self.break_all_alliances();
                    Ok(())
                }
            };
            if let Err(e) = r {
                self.statistics_stop.get_or_insert(e);
            }
        }
    }

    pub(crate) fn update_statistics(
        &mut self,
        tick: i32,
        next_tick: i32,
        mut rng: Option<&mut crate::engine_rng::EngineRng>,
    ) {
        let mut stats = std::mem::take(&mut self.statistics);
        if self.game.flags & FLAG_GAME_OVER == 0 {
            stats.countdown = stats.countdown.wrapping_sub(1);
            if stats.countdown < 1 {
                stats.rebuilds = stats.rebuilds.wrapping_add(1);
                stats.countdown = 0xf;
                self.rebuild_statistics(&mut stats, tick, rng.as_deref_mut());
            }
            stats.sample_tick = stats.sample_tick.wrapping_add(1);
            if stats.sample_period < stats.sample_tick {
                stats.sample_rollover();
            }
        } else if !stats.finished {
            self.rebuild_statistics(&mut stats, tick, rng);
            stats.finish(&self.players);
        }
        stats.clock = next_tick;
        self.statistics = stats;
    }

    pub(crate) fn statistics_rebuild_due(&self) -> bool {
        let stats = &self.statistics;
        if self.game.flags & FLAG_GAME_OVER == 0 {
            stats.countdown.wrapping_sub(1) < 1
        } else {
            !stats.finished
        }
    }

    pub(crate) fn force_statistics_update(
        &mut self,
        tick: i32,
        rng: Option<&mut crate::engine_rng::EngineRng>,
    ) {
        self.statistics.countdown = 0;
        self.update_statistics(tick, tick, rng);
    }

    fn flagged_monsters(&self) -> FlaggedMonsters {
        let mut out = FlaggedMonsters {
            teams: vec![0; self.players.teams.len()],
            ..Default::default()
        };
        if !(10..=12).contains(&self.game_type) {
            return out;
        }
        for m in &self.monsters {
            if m.identifier == 0 || m.flags & 0x4200_0000 == 0x4000_0000 {
                continue;
            }
            if m.flags_8 & 0x2000 != 0 {
                out.with_0x2000 += 1;
            }
            if m.flags_8 & 0x800 != 0 && m.team != -1 {
                out.flagged += 1;
                if let Some(t) = slot(&mut out.teams, m.team) {
                    *t += 1;
                }
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn adjust_damage_statistics(
        &mut self,
        attacker: UnitRef,
        victim: usize,
        amount: i16,
        killed: bool,
        victim_flags: u32,
        no_blame: u32,
        count: bool,
    ) {
        let attacker = attacker.0;
        if self.game.flags & FLAG_GAME_OVER != 0 {
            return;
        }
        let Some(m) = self.monsters.get(victim) else {
            return;
        };
        let (flags_4, flags_8, player) = (m.flags, m.flags_8, m.owner);
        let unit = self.statistics.monster_unit(victim);
        if let Some(t) = self.statistics.trace.as_mut() {
            t.push(format!(
                "{} A {attacker} {victim} {amount} {} {victim_flags} {no_blame} {} {unit} {player}",
                self.statistics.clock,
                i32::from(killed),
                i32::from(count),
            ));
        }
        let Some(row) = usize::try_from(unit)
            .ok()
            .and_then(|u| self.statistics.units.get(u))
            .copied()
        else {
            return;
        };
        let costs = row.costs.unwrap_or_default();
        let owner = row.owner;
        let scaled = if costs.max_hit_points == 0 {
            0
        } else {
            i32::from(costs.cost).wrapping_mul(i32::from(amount) << 8)
                / i32::from(costs.max_hit_points)
        };
        let team = |p: i16| {
            self.players
                .players
                .get(usize::try_from(p).unwrap_or(usize::MAX))
                .map_or(-1, |pl| i16::from(pl.team))
        };
        let attacker_owner = usize::try_from(attacker)
            .ok()
            .and_then(|a| self.statistics.units.get(a))
            .map_or(-1, |u| u.owner);
        let has_attacker = attacker != -1;
        let both_players = attacker_owner != -1 && owner != -1;
        let same_team = both_players && team(attacker_owner) == team(owner);
        let blamed = no_blame == 0 && owner != -1;
        let enemy_fire = has_attacker && !same_team;
        let local_team = self.local_team();
        if blamed && count {
            let victim_id = (owner, team(owner));
            let attacker_id = (
                if has_attacker { attacker_owner } else { -1 },
                if has_attacker && attacker_owner != -1 {
                    team(attacker_owner)
                } else {
                    -1
                },
            );
            if let Some(s) = self.statistics.current_sample() {
                s.add_damage(victim_id, attacker_id, scaled, killed, local_team);
            }
            self.statistics
                .totals
                .add_damage(victim_id, attacker_id, scaled, killed, local_team);
        }
        let stats = &mut self.statistics;
        if enemy_fire
            && count
            && costs.class != 5
            && let Some(a) = slot(&mut stats.units, attacker)
        {
            a.totals.damage_dealt = a.totals.damage_dealt.wrapping_add(scaled);
            if killed {
                a.totals.kills += 1;
            }
        }
        let u = &mut stats.units[unit as usize].totals;
        u.damage_taken = u.damage_taken.wrapping_add(scaled);
        u.hit_points_and_damage = u.hit_points_and_damage.wrapping_add(scaled);
        if !killed {
            return;
        }
        u.alive -= 1;
        if flags_4 & 0x1000_0000 != 0
            && flags_8 & 0x2000_0000 != 0
            && let Some(i) = stats.find_unit(player, row.tag)
        {
            let own = &mut stats.units[i].totals;
            own.uncontrollable -= 1;
            own.total += 1;
            stats.units[unit as usize].totals.total -= 1;
        }
        if casualty_sound(
            (self.mode, local_team, self.local_player),
            owner,
            no_blame,
            costs.class,
            same_team,
        ) {
            self.game.game_sound_requests.push(GAME_SOUND_CASUALTY);
        }
        if victim_flags != 0 && attacker_owner != -1 {
            let attacker_team = team(attacker_owner);
            let victim_team = if owner == -1 {
                team(player)
            } else {
                team(owner)
            };
            let tick = stats.clock;
            let Some(s) = slot(&mut stats.team_scores, attacker_team) else {
                return;
            };
            if self.game_type == 12 {
                s.score = s.score.wrapping_add(1);
                s.score_tick = tick;
            } else if self.game_type == 11 && no_blame == 0 {
                if victim_team == attacker_team {
                    s.score = s.score.wrapping_sub(1);
                } else {
                    s.score = s.score.wrapping_add(1);
                    s.score_tick = tick;
                }
            }
        }
    }

    pub(crate) fn hit_statistics(
        &mut self,
        unit: UnitRef,
        attacker: i16,
        victim: usize,
        dealt: i32,
        lethal: bool,
    ) {
        let Some(m) = self.monsters.get(victim) else {
            return;
        };
        let (f4, f8) = (m.flags, m.flags_8);
        let carried = f8 & 0x2800;
        let scoring_attacker = usize::try_from(attacker)
            .ok()
            .and_then(|a| self.monsters.get(a))
            .is_some_and(|a| a.flags_8 & 0x2000 != 0);
        let uncontrollable = f4 & 0x1000_0000 != 0
            && f8 & 2 != 0
            && !(matches!(self.game_type, 11 | 12) && carried != 0);
        let amount = dealt as i16;
        if self.game_type == 12 && scoring_attacker {
            self.adjust_damage_statistics(UnitRef::NONE, victim, amount, lethal, 0, 1, true);
        } else if uncontrollable {
            self.adjust_damage_statistics(UnitRef::NONE, victim, amount, lethal, carried, 1, true);
        } else {
            self.adjust_damage_statistics(unit, victim, amount, lethal, carried, f8 & 0x1000, true);
        }
    }

    pub(crate) fn deleted_by_action_statistics(&mut self, victim: usize) {
        let hit_points = self
            .monsters
            .get(victim)
            .map_or(0, |m| m.object.vitality as i16);
        self.adjust_damage_statistics(UnitRef::NONE, victim, hit_points, true, 0, 0, false);
    }

    pub(crate) fn control_lost_statistics(&mut self, monster: usize) {
        let unit = self.statistics.monster_unit(monster);
        let Some(row) = usize::try_from(unit)
            .ok()
            .and_then(|u| self.statistics.units.get(u))
            .copied()
        else {
            return;
        };
        let hit_points = self
            .monsters
            .get(monster)
            .map_or(0, |m| i32::from(m.object.vitality as i16));
        let stats = &mut self.statistics;
        let own = stats.tag_row(row.tag, None);
        stats.adjust_unit_statistics(unit, -1, -hit_points);
        stats.adjust_unit_statistics(own, 1, hit_points);
        stats.adjust_unit_statistics_for_uncontrollability(unit, 1);
        stats.units[own as usize].costs = row.costs;
        stats.monster_unit[monster] = own;
    }

    pub(crate) fn control_regained_statistics(&mut self, monster: usize) {
        let unit = self.statistics.monster_unit(monster);
        let Some(row) = usize::try_from(unit)
            .ok()
            .and_then(|u| self.statistics.units.get(u))
            .copied()
        else {
            return;
        };
        let Some(mon) = self.monsters.get(monster) else {
            return;
        };
        let (owner, hit_points, flags_8) = (
            mon.owner,
            i32::from(mon.object.vitality as i16),
            mon.flags_8,
        );
        let Some(new) = self.statistics.find_unit(owner, row.tag) else {
            return;
        };
        let new = new as i16;
        let gameplay = self.formation_globals.gameplay_version;
        let stats = &mut self.statistics;
        if gameplay > game_options::GAMEPLAY_1_7 {
            stats.adjust_unit_statistics(unit, -1, -hit_points);
            stats.adjust_unit_statistics(new, 1, hit_points);
            if gameplay > game_options::GAMEPLAY_1_7_2 && flags_8 & 0x2000_0000 != 0 {
                stats.adjust_unit_statistics_for_uncontrollability(new, -1);
            }
        }
        stats.monster_unit[monster] = new;
    }
}

pub fn format_damage(scaled: i32) -> i32 {
    scaled.wrapping_add(0x80) >> 8
}

pub fn stats_type_is_valid(page: i16, game_mode: i16) -> bool {
    match page {
        0 => true,
        1 => game_mode < 2,
        _ => false,
    }
}

pub fn graph_type_is_valid(graph: i16, game_type: i16) -> bool {
    match graph {
        0 => game_type == 10,
        1 | 2 | 4 => true,
        _ => false,
    }
}

pub fn ranking_columns(game_type: i16) -> u8 {
    if !(0..16).contains(&game_type) {
        return 0x73;
    }
    let bit = 1u32 << game_type;
    if bit & 0xfa61 != 0 {
        0xf3
    } else if bit & 0x198 != 0 {
        0xf7
    } else if bit & 6 != 0 {
        0x7f
    } else {
        0x73
    }
}

fn casualty_sound(
    local: (i16, i16, i16),
    owner: i16,
    no_blame: u32,
    class: i16,
    same_team: bool,
) -> bool {
    let (mode, local_team, local_player) = local;
    (mode != 1 || local_team != -1)
        && local_player == owner
        && no_blame == 0
        && class != 6
        && (!matches!(class, 2 | 5) || !same_team)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankingLine {
    Team(TeamRanking),
    Player {
        ranking: PlayerRanking,
        team: TeamRanking,
        in_team: bool,
    },
}

pub type PermanentLine = PermanentRanking;

#[derive(Debug, Clone, Copy)]
pub struct StatisticsView<'a> {
    pub statistics: &'a Statistics,
    pub players: &'a Players,
    pub game_type: i16,
    pub game_mode: i16,
    pub frozen: bool,
    pub local_team: i16,
    pub difficulty: i16,
    pub end_code: i16,
    pub world: &'a EngineWorld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphPoint {
    pub tick: i32,
    pub value: i16,
    pub running: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EpilogueUnit {
    pub mons: u32,
    pub damage_share: f64,
    pub kills: i16,
    pub dead: i16,
    pub fielded: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpilogueEnemy {
    pub mons: u32,
    pub dead: i16,
    pub fielded: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpilogueHero {
    pub mons: u32,
    pub name_index: i8,
    pub custom_name: String,
    pub battles: i32,
    pub experience: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpilogueReport {
    pub difficulty: i16,
    pub survivors: i32,
    pub units: Vec<EpilogueUnit>,
    pub heroes: Vec<EpilogueHero>,
    pub kills: i32,
    pub enemies: Vec<EpilogueEnemy>,
    pub damage_ratio: Option<f64>,
    pub kill_ratio: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ReportRow {
    own: bool,
    class_6: bool,
    mons: u32,
    mons_flags: u32,
    kills: i16,
    dead: i16,
    fielded: i16,
    damage: i32,
    total_hit_points: i32,
    hit_points_and_damage: i32,
}

impl Statistics {
    fn report_table(&self, players: &Players, local_team: i16, own: bool) -> Vec<ReportRow> {
        let mut table: Vec<ReportRow> = self
            .mons
            .iter()
            .map(|&mons| ReportRow {
                mons,
                ..Default::default()
            })
            .collect();
        for u in self.units.iter().filter(|u| !u.deleted) {
            let (Ok(owner), Ok(mons), Some(costs)) =
                (usize::try_from(u.owner), usize::try_from(u.mons), u.costs)
            else {
                continue;
            };
            let team = players.players.get(owner).map_or(-1, |p| i16::from(p.team));
            let t = u.totals;
            let fielded = t.uncontrollable.wrapping_add(t.total);
            if costs.flags & 0x800 != 0 || (team == local_team) != own || fielded == 0 {
                continue;
            }
            let Some(r) = table.get_mut(mons) else {
                continue;
            };
            r.fielded = r.fielded.wrapping_add(fielded as i16);
            r.kills = r.kills.wrapping_add(t.kills as i16);
            r.dead = r
                .dead
                .wrapping_add(t.total as i16)
                .wrapping_sub(t.alive as i16);
            r.damage = r.damage.wrapping_add(t.damage_dealt);
            r.total_hit_points = r.total_hit_points.wrapping_add(t.total_hit_points);
            r.hit_points_and_damage = r
                .hit_points_and_damage
                .wrapping_add(t.hit_points_and_damage);
            r.own = own;
            r.class_6 = costs.class == 6;
            r.mons_flags = costs.flags;
        }
        table
    }

    pub fn epilogue(
        &self,
        players: &Players,
        local_team: i16,
        difficulty: i16,
    ) -> Option<EpilogueReport> {
        let claimed = players.teams.get(usize::try_from(local_team).ok()?)?.player;
        if claimed == -1 {
            return None;
        }
        let team = *self.team_rankings.iter().find(|t| t.team == local_team)?;
        let mut own = self.report_table(players, local_team, true);
        qsort_private(&mut own, |a, b| b.damage.wrapping_sub(a.damage));
        let total: i32 = own.iter().fold(0, |n, r| n.wrapping_add(r.damage));
        let taken: i32 = own.iter().fold(0, |n, r| {
            n.wrapping_add(r.total_hit_points)
                .wrapping_sub(r.hit_points_and_damage)
        });
        let units = own
            .iter()
            .filter(|r| r.own && !r.class_6)
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
        let mut others = self.report_table(players, local_team, false);
        qsort_private(&mut others, |a, b| i32::from(b.dead) - i32::from(a.dead));
        let listed = |r: &&ReportRow| r.fielded != 0 && r.dead > 0 && r.mons_flags & 0x800 == 0;
        let kills = others
            .iter()
            .filter(listed)
            .map(|r| i32::from(r.dead))
            .sum();
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
        let damage_ratio = (taken != 0).then(|| {
            if team.damage_dealt < 0 {
                0.0
            } else {
                f64::from(-(team.damage_dealt as f32) / taken as f32)
            }
        });
        let kill_ratio =
            (team.losses != 0).then(|| f64::from(team.kills as f32 / team.losses as f32));
        Some(EpilogueReport {
            difficulty,
            survivors: team.survivors.wrapping_add(team.uncontrollable),
            units,
            heroes: Vec::new(),
            kills,
            enemies,
            damage_ratio,
            kill_ratio,
        })
    }
}

fn string_of_list(list: &[u8], index: usize) -> Option<&[u8]> {
    list.split(|&b| b == b'\r' || b == 0).nth(index)
}

fn mons_list(tags: &dyn TagSource, mons: u32, offset: usize) -> Option<Vec<u8>> {
    let data = tags.tag(*b"mons", FourCC(mons.to_be_bytes()))?;
    let list = FourCC(data.get(offset..offset + 4)?.try_into().ok()?);
    tags.tag(*b"stli", list)
}

pub fn monster_spelling(tags: &dyn TagSource, mons: u32, count: i16) -> Option<Vec<u8>> {
    let list = mons_list(tags, mons, 0x1a8)?;
    let strings = list
        .split(|&b| b == b'\r' || b == 0)
        .filter(|s| !s.is_empty())
        .count();
    string_of_list(&list, usize::from(count != 1 && strings != 1)).map(<[u8]>::to_vec)
}

pub fn monster_name(tags: &dyn TagSource, mons: u32, index: i8) -> Option<Vec<u8>> {
    let list = mons_list(tags, mons, 0x1ac)?;
    string_of_list(&list, usize::try_from(index).ok()?).map(<[u8]>::to_vec)
}

impl StatisticsView<'_> {
    pub fn epilogue(&self) -> Option<EpilogueReport> {
        let mut report =
            self.statistics
                .epilogue(self.players, self.local_team, self.difficulty)?;
        if !matches!(self.end_code, 1 | 2) {
            return Some(report);
        }
        let table = self.world.initializers_at_game_over().records();
        if table.len() > 1 {
            report.heroes = table
                .iter()
                .filter(|v| self.world.monster_def_index.contains_key(&v.monster_tag))
                .map(|v| EpilogueHero {
                    mons: u32::from_be_bytes(v.monster_tag.0),
                    name_index: v.name_index as i8,
                    custom_name: v.name.clone(),
                    battles: i32::from(v.field_8f_plus_one as i8),
                    experience: v.experience,
                })
                .filter(|h| i32::from(h.experience) + h.battles > 5)
                .take(4)
                .collect();
        }
        Some(report)
    }

    pub fn stats_page_valid(&self, page: i16) -> bool {
        stats_type_is_valid(page, self.game_mode)
    }

    pub fn graph_valid(&self, graph: i16) -> bool {
        graph_type_is_valid(graph, self.game_type)
    }

    pub fn graph_value(&self, graph: i16, sample: usize, team: i16) -> i16 {
        let Some(s) = self.statistics.samples.get(sample) else {
            return 0;
        };
        let t = usize::try_from(team).ok().and_then(|t| s.teams.get(t));
        match graph {
            0 => {
                let mut sum = 0i16;
                for (p, v) in self.players.players.iter().zip(&s.players) {
                    let counted =
                        i16::from(p.team) == team && (self.game_type == 13 || p.field_09 == 0);
                    if counted {
                        sum = sum.wrapping_add(v.score as i16);
                    }
                }
                sum
            }
            1 => t.map_or(0, |t| t.kills as i16),
            2 => t.map_or(0, |t| t.damage as i16),
            4 => t.map_or(0, |t| t.killed_by_local_team as i16),
            _ => 0,
        }
    }

    pub fn graph_points(&self, graph: i16, team: i16) -> Vec<GraphPoint> {
        let range = self.statistics.sample_range();
        let mut running = 0i32;
        (0..range.count.max(0))
            .map(|i| {
                let value = self.graph_value(graph, i as usize, team);
                running = running.wrapping_add(i32::from(value));
                GraphPoint {
                    tick: range.period.wrapping_mul(i),
                    value,
                    running,
                }
            })
            .collect()
    }

    pub fn ranking_lines(&self) -> Vec<RankingLine> {
        let s = self.statistics;
        let mut lines = Vec::new();
        let player = |r: &PlayerRanking| self.players.players.get(r.player as usize).copied();
        for t in &s.team_rankings {
            let members = s
                .player_rankings
                .iter()
                .filter(|r| player(r).is_some_and(|p| i16::from(p.field_0b) == t.team))
                .count();
            if members == 0 {
                continue;
            }
            let in_team = members > 1;
            if in_team {
                lines.push(RankingLine::Team(*t));
            }
            for r in &s.player_rankings {
                let Some(p) = player(r) else { continue };
                if i16::from(p.field_0b) != t.team
                    || p.flags & 0x40 != 0
                    || r.parent != -1
                    || p.field_09 != 0
                {
                    continue;
                }
                let mut merged = *r;
                for c in s.player_rankings.iter().filter(|c| c.parent == r.player) {
                    merged.losses = merged.losses.wrapping_add(c.losses);
                    merged.losses_cost = merged.losses_cost.wrapping_add(c.losses_cost);
                    merged.kills = merged.kills.wrapping_add(c.kills);
                    merged.damage_dealt = merged.damage_dealt.wrapping_add(c.damage_dealt);
                    merged.damage_taken = merged.damage_taken.wrapping_add(c.damage_taken);
                }
                lines.push(RankingLine::Player {
                    ranking: merged,
                    team: *t,
                    in_team,
                });
            }
        }
        lines
    }

    pub fn permanent_lines(&self) -> Vec<PermanentLine> {
        let s = self.statistics;
        s.permanent_rankings
            .iter()
            .filter(|r| {
                self.players
                    .players
                    .get(r.player as usize)
                    .is_some_and(|p| p.field_09 == 0 && p.flags & 0x40 == 0)
                    && r.parent == -1
            })
            .map(|r| {
                let mut m = *r;
                for c in s.permanent_rankings.iter().filter(|c| c.parent == r.player) {
                    m.points = m.points.wrapping_add(c.points);
                    m.losses = m.losses.wrapping_add(c.losses);
                    m.kills = m.kills.wrapping_add(c.kills);
                    m.damage_dealt = m.damage_dealt.wrapping_add(c.damage_dealt);
                }
                m
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_players_rows_leave_holes_the_next_rows_fill() {
        let mut s = Statistics::default();
        let (a, b) = (0x6161_6161, 0x6262_6262);
        s.tag_row(a, Some(1));
        s.tag_row(b, Some(2));
        assert_eq!((s.player_row(a, 1), s.player_row(a, 2)), (2, 3));
        assert_eq!((s.player_row(b, 2), s.player_row(b, 1)), (4, 5));
        s.adjust_unit_statistics(2, 1, 10);
        s.delete_player_rows(1, 0x73c);
        assert_eq!(s.units.len(), 5);
        assert!(s.units[2].deleted && !s.units[3].deleted && !s.units[4].deleted);
        assert_eq!(s.units[2].totals, UnitStatistics::default());
        assert_eq!((s.find_unit(1, a), s.free_units), (None, 1));
        s.delete_player_rows(2, 0x73c);
        assert_eq!((s.units.len(), s.free_units), (3, 1));
        assert_eq!(s.player_row(b, 3), 2);
        assert_eq!((s.free_units, s.units[2].deleted), (0, false));
        assert_eq!(s.player_row(a, 3), 3);
    }

    #[test]
    fn a_tie_of_two_swaps_them_as_the_heap_sort_does() {
        let mut v = [(0, 5), (1, 5)];
        qsort_private(&mut v, |a, b| b.1 - a.1);
        assert_eq!(v, [(1, 5), (0, 5)]);
    }

    #[test]
    fn sort_orders_by_the_comparison() {
        let mut v = [3, 1, 4, 1, 5, 9, 2, 6];
        qsort_private(&mut v, |a, b| b - a);
        assert_eq!(v, [9, 6, 5, 4, 3, 2, 1, 1]);
    }

    #[test]
    fn damage_rounds_as_the_dialog() {
        assert_eq!(format_damage(18276), 71);
        assert_eq!(format_damage(36562), 143);
        assert_eq!(format_damage(28859), 113);
    }

    #[test]
    fn pages_as_the_engine_allows_them() {
        assert!(stats_type_is_valid(1, 0));
        assert!(!stats_type_is_valid(1, 2));
        assert!(!graph_type_is_valid(0, 13));
        assert!(graph_type_is_valid(0, 10));
        assert!(!graph_type_is_valid(3, 10));
        assert!(graph_type_is_valid(4, 13));
        assert_eq!(ranking_columns(13), 0xf3);
        assert_eq!(ranking_columns(1), 0x7f);
        assert_eq!(ranking_columns(10), 0x73);
    }

    #[test]
    fn a_score_of_nobody_goes_to_its_own_slot_and_stops_at_game_over() {
        let mut s = Statistics {
            totals: StatisticsSample::new(1, 1),
            ..Default::default()
        };
        s.adjust_player_score(-1, 3, 0);
        s.adjust_player_score(0, 2, 0);
        s.adjust_player_score(0, 2, FLAG_GAME_OVER);
        assert_eq!(s.totals.no_player_score, 3);
        assert_eq!(s.totals.players[0].score, 2);
    }

    #[test]
    fn the_twenty_first_sample_merges_pairs_and_keeps_the_stale_ones() {
        let mut s = Statistics {
            sample_period: 300,
            sample: 20,
            sample_tick: 7,
            samples: vec![StatisticsSample::new(1, 1); SAMPLES],
            ..Default::default()
        };
        for (i, r) in s.samples.iter_mut().enumerate() {
            r.players[0].own_damage = 1 << i;
            r.teams[0].kills = 1 << i;
            r.no_player_score = 1 << i;
        }
        s.sample_rollover();
        assert_eq!((s.sample, s.sample_period, s.sample_tick), (11, 600, 0));
        for (i, sample) in s.samples[..10].iter().enumerate() {
            let want = (1 << (i * 2)) | (1 << (i * 2 + 1));
            assert_eq!(sample.players[0].own_damage, want);
            assert_eq!(sample.teams[0].kills, want);
            assert_eq!(sample.no_player_score, want);
        }
        assert_eq!(s.samples[10], StatisticsSample::new(1, 1));
        assert_eq!(s.samples[11].teams[0].kills, 1 << 11);
        s.adjust_player_score(0, 5, 0);
        assert_eq!(s.samples[10].players[0].score, 5);
        s.sample_rollover();
        assert_eq!((s.sample, s.sample_period), (12, 600));
        assert_eq!(s.samples[11], StatisticsSample::new(1, 1));
    }
}
