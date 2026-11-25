use mandrake_common::game_options;
use mandrake_math::BungieRng;
use mandrake_tag::map_action::MeshMapActions;

use crate::draw_site;
use crate::engine_rng::{EngineRng, Unmodelled};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerSetup {
    pub team: i16,
    pub field_02: i16,
    pub flags: u16,
    pub follow: i32,
    pub id: i32,
    pub colors: [PlayerColor; 2],
    pub name: PlayerName,
}

pub type PlayerName = [u8; 32];

pub fn player_name(name: &[u8]) -> PlayerName {
    let mut out = [0u8; 32];
    let n = name.len().min(31);
    out[..n].copy_from_slice(&name[..n]);
    out
}

pub type PlayerColor = [u16; 4];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameSetup {
    pub options: u32,
    pub team_seed: u16,
    pub maximum_teams: i16,
    pub local_player: i16,
    pub players: Vec<PlayerSetup>,
    pub complete: bool,
}

pub const COMMAND_LINE_GAME_OPTIONS: u32 = 0x200c_40fc;

pub const REFERENCE_COMMAND_LINE_SEED: u32 = 0;

pub fn set_random_seed(seed: u32) -> BungieRng {
    BungieRng::new(if seed == 0 { 0xfded } else { seed })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoloGame {
    pub seed: u32,
    pub difficulty: i16,
    pub options: u32,
}

pub const SOLO_GAME_OPTIONS: u32 = COMMAND_LINE_GAME_OPTIONS;

pub const NEW_GAME_OPTIONS: u32 = 0x200c_40bc;

pub use mandrake_common::game_options::CARRY_VETERANS as VETERANS_ALLOWED;

pub use mandrake_common::game_options::SOLO_RESTARTED as SOLO_GAME_RESTARTED;

pub const TRAINING_MESH: [u8; 4] = *b"00tm";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoloStart {
    NewGame,
    Map { restart: bool },
}

pub fn solo_game_options(start: SoloStart, mesh: [u8; 4]) -> u32 {
    match start {
        SoloStart::NewGame => NEW_GAME_OPTIONS,
        SoloStart::Map { restart } => {
            let mut options = NEW_GAME_OPTIONS;
            if mesh != TRAINING_MESH {
                options |= VETERANS_ALLOWED;
            }
            if restart {
                options |= SOLO_GAME_RESTARTED;
            }
            options
        }
    }
}

impl GameSetup {
    pub fn solo(game: SoloGame) -> Self {
        let p = |team: i16, field_02: i16, id: i32, name: &[u8]| PlayerSetup {
            team,
            field_02,
            flags: 0,
            follow: -1,
            id,
            colors: [[0; 4]; 2],
            name: player_name(name),
        };
        Self {
            options: game.options,
            team_seed: game.seed as u16,
            maximum_teams: 0,
            local_player: 0,
            players: vec![
                p(0, 0, 0, b"Light"),
                p(1, 1, -1, b"Dark"),
                p(2, 1, -1, b"Neutral"),
            ],
            complete: true,
        }
    }

    pub fn command_line(options: u32, seed: u32) -> Self {
        let p = |team: i16, field_02: i16, name: &[u8]| PlayerSetup {
            team,
            field_02,
            flags: 0,
            follow: -1,
            id: 0,
            colors: [[0; 4]; 2],
            name: player_name(name),
        };
        Self {
            options,
            team_seed: seed as u16,
            maximum_teams: 0,
            local_player: 0,
            players: vec![p(0, 0, b"Light"), p(1, 1, b"Dark"), p(2, 1, b"Neutral")],
            complete: true,
        }
    }

    pub fn from_block(block: &[u8]) -> Option<Self> {
        let i16_at = |o: usize| {
            block
                .get(o..o + 2)
                .map(|b| i16::from_be_bytes([b[0], b[1]]))
        };
        let i32_at = |o: usize| {
            block
                .get(o..o + 4)
                .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        let count = i16_at(0x22c)?.clamp(0, 16) as usize;
        let mut players = Vec::with_capacity(count);
        for i in 0..count {
            let e = 0x230 + i * 0x7c;
            let color = |o: usize| -> Option<PlayerColor> {
                Some([
                    i16_at(o)? as u16,
                    i16_at(o + 2)? as u16,
                    i16_at(o + 4)? as u16,
                    i16_at(o + 6)? as u16,
                ])
            };
            players.push(PlayerSetup {
                team: i16_at(e)?,
                field_02: i16_at(e + 2)?,
                flags: i16_at(e + 4)? as u16,
                follow: i32_at(e + 8)?,
                id: i32_at(e + 0x0c)?,
                colors: [color(e + 0x6c)?, color(e + 0x74)?],
                name: block.get(e + 0x2c..e + 0x4c)?.try_into().ok()?,
            });
        }
        Some(Self {
            options: i32_at(0x04)? as u32,
            team_seed: i16_at(0x14)? as u16,
            maximum_teams: i16_at(0x16)?,
            local_player: i16_at(0x22e)?,
            players,
            complete: true,
        })
    }

    pub fn from_film(film: &[u8]) -> Option<Self> {
        Self::from_block(mandrake_rec::header::game_parameters(film)?)
    }

    pub fn from_header(header: &mandrake_rec::RecordingHeader) -> Self {
        Self {
            options: header.game_options_flags,
            team_seed: 0,
            maximum_teams: 0,
            local_player: 0,
            players: header
                .players
                .iter()
                .take(16)
                .map(|p| PlayerSetup {
                    team: p.team,
                    field_02: p.field_02,
                    flags: 0,
                    follow: -1,
                    id: 0,
                    colors: [[0; 4]; 2],
                    name: player_name(&mandrake_rec::mac_roman::encode(&p.name)),
                })
                .collect(),
            complete: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Player {
    pub flags: u8,
    pub field_09: u8,
    pub team: i8,
    pub field_0b: i8,
    pub id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Team {
    pub flags: u16,
    pub players: u8,
    pub player: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Players {
    pub players: Vec<Player>,
    pub colors: Vec<[PlayerColor; 2]>,
    pub names: Vec<PlayerName>,
    pub teams: Vec<Team>,
    pub occupied_teams: i32,
    pub without_team: Vec<usize>,
}

impl Players {
    pub fn local_without_team(&self, local: i16) -> bool {
        usize::try_from(local).is_ok_and(|l| self.without_team.contains(&l))
    }
}

pub fn reserved_teams(actions: Option<&MeshMapActions>, game_type: i16) -> (u32, i32) {
    let (mut mask, mut count) = (0u32, 0);
    let Some(actions) = actions else {
        return (0, 0);
    };
    for a in actions
        .actions
        .iter()
        .filter(|a| a.identifier != 0 && a.tag == u32::from_be_bytes(*b"prta"))
    {
        for p in &a.parameters {
            if p.kind != 5
                || p.count <= 1
                || p.values.first().copied() != Some(i32::from(game_type))
            {
                continue;
            }
            for &t in p.values.iter().take(p.count as usize).skip(1) {
                if (t as u32) <= 0xf && mask & (1 << t) == 0 {
                    mask |= 1 << t;
                    count += 1;
                }
            }
        }
    }
    (mask, count)
}

fn preprocess(
    setup: &mut GameSetup,
    gameplay: i32,
    rng: &mut EngineRng,
) -> Result<usize, Unmodelled> {
    let n = setup.players.len();
    if n == 0 {
        return Ok(0);
    }

    let first_observer_id = setup
        .players
        .iter()
        .find(|p| p.flags & 0x40 != 0)
        .map(|p| p.id)
        .unwrap_or(0x7fff_ffff);

    if first_observer_id != 0x7fff_ffff
        && n > 3
        && gameplay > game_options::GAMEPLAY_1_5
        && setup.options & game_options::COOPERATIVE != 0
    {
        let count = setup
            .players
            .iter()
            .filter(|p| p.flags & 0x40 != 0 || p.follow == first_observer_id)
            .count();
        if count == n - 2 {
            for p in &mut setup.players {
                if p.follow == first_observer_id && p.flags & 0x40 == 0 {
                    p.follow = -1;
                }
            }
            setup.options = (setup.options & !game_options::COOPERATIVE_TEAM) | game_options::TEAMS;
        }
    }

    let mut active_count = n;
    for i in (0..n).rev() {
        if setup.players[i].flags & 0x40 != 0 || setup.players[i].follow == first_observer_id {
            let saved = setup.players[i];
            if setup.local_player == i as i16 {
                setup.local_player = -1;
            } else if setup.local_player > i as i16 {
                setup.local_player -= 1;
            }
            for k in i..active_count - 1 {
                setup.players[k] = setup.players[k + 1];
            }
            active_count -= 1;
            setup.players[active_count] = saved;
            setup.players[active_count].follow = -1;
        }
    }

    let co_op_teams = game_options::COOPERATIVE | game_options::TEAMS;
    if gameplay > game_options::GAMEPLAY_1_3
        && setup.options & (game_options::COOPERATIVE_TEAM | co_op_teams) == co_op_teams
    {
        let min = 2 + i32::from(gameplay < 0x714);
        if (active_count as i32) > min {
            let r = i32::from(rng.draw(draw_site::PLAYERS_PREPROCESS));
            let m = active_count as i32 - 2;
            let id = setup.players[(r % m) as usize].id;
            for p in setup.players.iter_mut().take(m as usize) {
                p.follow = id;
            }
        }
    }
    Ok(active_count)
}

const SUBSTITUTE_COLORS: [PlayerColor; 20] = [
    [0xe146, 0, 0, 0],
    [0, 0xbd6f, 0, 0],
    [0x7fff, 0x2b84, 0, 0],
    [0xf0a2, 0xf850, 0, 0],
    [0xbae0, 0xbae0, 0xffff, 0],
    [0x547a, 0x51eb, 0x6666, 0],
    [0, 0xd47a, 0xd47a, 0],
    [0, 0, 0xee13, 0],
    [0xae13, 0xffff, 0xffff, 0],
    [0xd47a, 0x5709, 0xffff, 0],
    [0xdeb7, 0xb851, 0, 0],
    [0x6666, 0x6666, 0, 0],
    [0, 0xa665, 0xca3c, 0],
    [0xffff, 0xa3d6, 0xa3d6, 0],
    [0xca3c, 0x9709, 0xffff, 0],
    [0, 0x3851, 0x8a3c, 0],
    [0x6b84, 0, 0x6147, 0],
    [0xffff, 0xffff, 0xcf5b, 0],
    [0x35c2, 0x63d6, 0, 0],
    [0xbfff, 0xc28e, 0xa665, 0],
];

pub(crate) fn substitute_color(at: i32) -> PlayerColor {
    SUBSTITUTE_COLORS[(i64::from(at) as u64 % 20) as usize]
}

pub(crate) fn arbitrate_color(own: &mut PlayerColor, id: i32, others: &[PlayerColor]) {
    let mut at = id.wrapping_mul(0x2e);
    let end = at.wrapping_add(0x14);
    loop {
        let clash = others
            .iter()
            .rev()
            .any(|o| (0..3).all(|c| own[c].abs_diff(o[c]) < 0x3f00));
        if !clash || at == end {
            return;
        }
        *own = substitute_color(at);
        at = at.wrapping_add(1);
    }
}

fn arbitrate_single_appearance(players: &mut [PlayerSetup], who: usize, among: &[usize]) {
    let id = players[who].id;
    let others: Vec<PlayerColor> = among
        .iter()
        .copied()
        .filter(|&o| o != who)
        .map(|o| players[o].colors[0])
        .collect();
    arbitrate_color(&mut players[who].colors[0], id, &others);
}

fn arbitrate_player_appearances(players: &mut [PlayerSetup]) {
    let leaders = leaders(players);
    for &i in leaders.iter().rev() {
        arbitrate_single_appearance(players, i, &leaders);
    }
    for i in 0..players.len() {
        let id = players[i].id;
        let list: Vec<usize> = if id == -1 {
            leaders.clone()
        } else {
            let same = (0..players.len()).filter(|&j| players[j].id == id);
            let joined =
                (0..players.len()).filter(|&j| players[j].follow == id && players[j].id != id);
            same.chain(joined).collect()
        };
        for &j in list.iter().rev() {
            arbitrate_single_appearance(players, j, &list);
        }
    }
    for i in 0..players.len() {
        if players[i].follow == -1 {
            continue;
        }
        if let Some(j) = (0..players.len()).find(|&j| players[j].id == players[i].follow) {
            players[i].colors[1] = players[i].colors[0];
            players[i].colors[0] = players[j].colors[0];
        }
    }
}

fn leaders(players: &[PlayerSetup]) -> Vec<usize> {
    (0..players.len())
        .filter(|&i| players[i].id == players[i].follow || players[i].follow == -1)
        .collect()
}

pub fn colors_replace_core(core: &[u8], hue_flags: &[u16], colors: &[PlayerColor; 2]) -> bool {
    let Ok(permutations) = mandrake_tag::collection::CorePermutations::from_bytes(core) else {
        return false;
    };
    let off = |c: &PlayerColor| c[3] & 4 != 0;
    let replaceable = (0..permutations.len()).any(|p| {
        permutations
            .colors(p, hue_flags.len())
            .iter()
            .zip(hue_flags)
            .any(|(permuted, &flags)| {
                let used = if flags & 1 != 0 && !off(&colors[0]) {
                    true
                } else {
                    flags & 2 != 0 && !off(&colors[1])
                };
                used && !off(permuted)
            })
    });
    replaceable && core.get(0x153).is_some_and(|f| f & 1 == 0)
}

pub fn collection_hue_flags(collection: &[u8]) -> Vec<u16> {
    let word = |o: usize| {
        collection
            .get(o..o + 4)
            .map_or(0, |b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let at = 0x140 + word(0x54).max(0) as usize;
    (0..word(0x50).max(0) as usize)
        .map_while(|j| {
            let f = collection.get(at + 0x80 * j + 0x68..at + 0x80 * j + 0x6a)?;
            Some(u16::from_be_bytes([f[0], f[1]]))
        })
        .collect()
}

fn shuffle_teams(
    follow: &mut [i32],
    slots: i32,
    maximum_teams: i16,
    rng: &mut EngineRng,
) -> Result<(), Unmodelled> {
    let n = follow.len();
    if n == 0 {
        return Ok(());
    }
    let limit = if maximum_teams > 0 {
        slots.min(i32::from(maximum_teams))
    } else {
        slots
    };
    let followers = loop {
        let mut followers = [0i32; 16];
        let mut alone = Vec::with_capacity(n);
        let (mut captains, mut teams) = (0, 0);
        for (i, &f) in follow.iter().enumerate() {
            if f == i as i32 {
                captains += 1;
                teams += 1;
            } else if f == -1 {
                teams += 1;
                alone.push(i);
                continue;
            }
            followers[f as usize] += 1;
        }
        while limit < captains {
            let mut smallest = (0x10, None);
            for (k, &c) in followers.iter().enumerate().take(n) {
                if c != 0 && c < smallest.0 {
                    smallest = (c, Some(k));
                }
            }
            let Some(k) = smallest.1 else {
                captains -= 1;
                continue;
            };
            for (p, f) in follow.iter_mut().enumerate() {
                if *f == k as i32 {
                    *f = -1;
                    alone.push(p);
                }
            }
            captains -= 1;
            followers[k] = 0;
        }
        if limit >= teams {
            break followers;
        }
        if captains < limit {
            if alone.is_empty() {
                break followers;
            }
            let r = usize::from(rng.draw(draw_site::SHUFFLE_TEAMS));
            let p = alone[r % alone.len()];
            follow[p] = p as i32;
        } else {
            let mut fewest = i32::from(i16::MAX);
            let mut smallest = Vec::with_capacity(n);
            for (i, &f) in follow.iter().enumerate() {
                if f == i as i32 && followers[i] <= fewest {
                    if followers[i] < fewest {
                        fewest = followers[i];
                        smallest.clear();
                    }
                    smallest.push(i);
                }
            }
            if alone.is_empty() || smallest.is_empty() {
                break followers;
            }
            let r = usize::from(rng.draw(draw_site::SHUFFLE_TEAMS_SECOND));
            let p = alone[r % alone.len()];
            let r = usize::from(rng.draw(draw_site::SHUFFLE_TEAMS_THIRD));
            follow[p] = smallest[r % smallest.len()] as i32;
        }
    };
    for (i, f) in follow.iter_mut().enumerate() {
        if *f == i as i32 && followers[i] < 2 {
            *f = -1;
        }
    }
    Ok(())
}

pub fn initialize_players(
    setup: &mut GameSetup,
    team_count: usize,
    reserved: (u32, i32),
    gameplay: i32,
    rng: &mut EngineRng,
) -> Result<Players, Unmodelled> {
    if gameplay <= 0x713 {
        return Err(Unmodelled::new(
            draw_site::INITIALIZE_PLAYERS_OLD_GAMEPLAY,
            format!("initialize_players_for_new_map for gameplay 0x{gameplay:x}"),
        ));
    }
    let active_count = preprocess(setup, gameplay, rng)?;
    let (mask, reserved_count) = reserved;
    let slots = (team_count as i32 - reserved_count).max(0);
    let mut out = Players {
        players: Vec::new(),
        colors: Vec::new(),
        names: Vec::new(),
        teams: vec![Team::default(); team_count],
        occupied_teams: 0,
        without_team: Vec::new(),
    };
    let mut follow = vec![-1i32; setup.players.len()];
    for (i, e) in setup.players.iter().enumerate() {
        let mut team = e.team as u8;
        if team != 0xff && (1u32 << (team & 31)) & mask != 0 {
            team = 0xff;
        }
        out.players.push(Player {
            flags: e.flags as u8,
            field_09: e.field_02 as u8,
            team: team as i8,
            field_0b: 0,
            id: e.id,
        });
        out.names.push(e.name);
        if e.follow != -1
            && let Some(j) = setup.players[..active_count]
                .iter()
                .position(|o| o.id == e.follow)
        {
            follow[i] = j as i32;
        }
    }
    if slots != 0 && setup.options & game_options::COOPERATIVE == 0 {
        let before = follow[..active_count].to_vec();
        shuffle_teams(&mut follow[..active_count], slots, setup.maximum_teams, rng)?;
        for i in 0..active_count {
            if follow[i] != before[i] {
                setup.players[i].follow = match follow[i] {
                    -1 => -1,
                    f => setup.players[f as usize].id,
                };
            }
        }
    }
    arbitrate_player_appearances(&mut setup.players[..active_count]);
    out.colors = out
        .players
        .iter()
        .map(|pl| {
            let by_team = pl.field_09 == 1 && gameplay >= 0x732;
            setup.players[..active_count]
                .iter()
                .find(|e| {
                    if by_team {
                        i16::from(pl.team) == e.team
                    } else {
                        pl.id == e.id
                    }
                })
                .map_or([[0; 4]; 2], |e| e.colors)
        })
        .collect();

    let mut team_rng = set_random_seed(u32::from(setup.team_seed));
    let mut team_sites = crate::engine_rng::SiteCounts::default();
    let mut tr = EngineRng {
        rng: &mut team_rng,
        sites: &mut team_sites,
    };
    let tc = team_count as i32;
    for (i, &f) in follow[..active_count].iter().enumerate() {
        if slots == 0 || (f != -1 && f != i as i32) {
            out.players[i].team = -1;
            continue;
        }
        let mut t = i32::from(out.players[i].team);
        if t == -1 {
            t = i32::from(tr.draw(draw_site::INITIALIZE_PLAYERS_TEAM)) % tc;
        }
        if t < 0 || t >= tc {
            t = 0;
        }
        out.players[i].team = -1;
        let mut d = t;
        loop {
            let team = &mut out.teams[d as usize];
            if team.players == 0 && (1u32 << (d & 31)) & mask == 0 {
                team.players = 1;
                team.player = i as i16;
                out.players[i].team = d as i8;
                break;
            }
            d = (d + 1) % tc;
            if d == t {
                out.without_team.push(i);
                break;
            }
        }
    }
    for (i, &f) in follow.iter().enumerate() {
        if out.players[i].team == -1 && f != -1 {
            let t = out.players[f as usize].team;
            out.players[i].team = t;
            if t != -1 {
                out.teams[t as u8 as usize].players += 1;
            }
        }
        out.players[i].field_0b = out.players[i].team;
    }
    for pl in &mut out.players[active_count..] {
        if gameplay <= game_options::GAMEPLAY_1_8_4 {
            pl.flags |= 0x44;
        } else {
            pl.flags |= 0x40;
        }
        pl.team = -1;
    }
    out.occupied_teams = out.teams.iter().filter(|t| t.players != 0).count() as i32;
    Ok(out)
}

#[cfg(test)]
mod solo_options_tests {
    use super::*;

    #[test]
    fn solo_game_options_are_the_engines() {
        assert_eq!(solo_game_options(SoloStart::NewGame, *b"le3e"), 0x200c_40bc);
        let next = SoloStart::Map { restart: false };
        let restart = SoloStart::Map { restart: true };
        assert_eq!(solo_game_options(next, *b"cr01"), 0x200c_40fc);
        assert_eq!(solo_game_options(next, *b"cr01"), SOLO_GAME_OPTIONS);
        assert_eq!(solo_game_options(restart, *b"cr01"), 0x200e_40fc);
        assert_eq!(solo_game_options(next, TRAINING_MESH), 0x200c_40bc);
        assert_eq!(solo_game_options(restart, TRAINING_MESH), 0x200e_40bc);
    }
}

#[cfg(test)]
mod shuffle_tests {
    use super::*;
    use crate::engine_rng::SiteCounts;

    #[test]
    fn one_captain_of_sixteen_over_no_slots() {
        let mut follow = [0i32; 16];
        let (mut r, mut sites) = (BungieRng::new(1), SiteCounts::default());
        let mut rng = EngineRng {
            rng: &mut r,
            sites: &mut sites,
        };
        shuffle_teams(&mut follow, 0, 4, &mut rng).unwrap();
        assert_eq!(follow, [0; 16]);
        assert_eq!(sites.total(), 0);
    }

    #[test]
    fn the_captain_with_fewest_followers_drops() {
        let mut follow = [0, 0, 0, 3, 3];
        let (mut r, mut sites) = (BungieRng::new(1), SiteCounts::default());
        let mut rng = EngineRng {
            rng: &mut r,
            sites: &mut sites,
        };
        shuffle_teams(&mut follow, 1, 0, &mut rng).unwrap();
        assert_eq!(follow[..3], [0, 0, 0]);
        assert!(follow[3..].iter().all(|&f| f == 0));
    }

    #[test]
    fn from_header_truncates_to_sixteen_players() {
        let header = mandrake_rec::RecordingHeader {
            name: String::new(),
            tag_type: mandrake_rec::RecordingHeader::TAG_TYPE,
            version: 0,
            sub_version: 0,
            mode: 0,
            game_type: 0,
            game_options_flags: 0,
            time_limit: 0,
            end_tick: 0,
            planning_time: 0,
            mesh_tag_id: mandrake_common::FourCC(*b"test"),
            map_name: String::new(),
            version_id: 0,
            initial_seed: 0,
            seed: 0,
            active_plugins: Vec::new(),
            players: (0..20)
                .map(|i| mandrake_rec::PlayerInfo {
                    team: 0,
                    field_02: 0,
                    name: format!("p{i}"),
                })
                .collect(),
            teams: Vec::new(),
            monster_initializers: Vec::new(),
        };
        let setup = GameSetup::from_header(&header);
        assert_eq!(setup.players.len(), 16);
    }
}
