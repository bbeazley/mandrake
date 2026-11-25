use mandrake_common::FourCC;

use crate::draw_site;
use crate::engine_move::distance2d;
use crate::engine_rng::{EngineRng, EngineWorld, Unmodelled};
use crate::statistics::TeamScore;

pub const OBJECT_KIND_FLAG: u8 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag {
    pub kind: i16,
    pub flags: u32,
    pub object: i32,
    pub projectile: Option<FourCC>,
    pub team: i16,
    pub previous_team: i16,
    pub original_team: i16,
    pub number: i16,
    pub time: i32,
    pub marker: usize,
    pub position: (i32, i32),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagData {
    pub records: Vec<Option<Flag>>,
    pub unnumbered: i16,
    pub numbered: i16,
    pub numbers: u32,
}

impl FlagData {
    #[allow(clippy::too_many_arguments)]
    pub fn new_flag(
        &mut self,
        object: i32,
        projectile: Option<FourCC>,
        team: i16,
        number: i16,
        marker: usize,
        marker_flags: u32,
        position: (i32, i32),
    ) -> i16 {
        if number == -1 {
            self.unnumbered += 1;
        } else {
            let bit = 1u32 << (number as u32 & 0x1f);
            if self.numbers & bit == 0 {
                self.numbered += 1;
                self.numbers |= bit;
            }
        }
        let mut flags = 0;
        if marker_flags & 2 != 0 {
            flags |= 1;
        }
        if marker_flags & 1 != 0 {
            flags |= 2;
        }
        let record = Flag {
            kind: i16::from(projectile.is_some()),
            flags,
            object,
            projectile,
            team,
            previous_team: team,
            original_team: team,
            number,
            time: 0,
            marker,
            position,
        };
        match self.records.iter().position(Option::is_none) {
            Some(i) => {
                self.records[i] = Some(record);
                i as i16
            }
            None => {
                self.records.push(Some(record));
                (self.records.len() - 1) as i16
            }
        }
    }

    pub fn flag_of_object(&self, object: i32) -> Option<i16> {
        self.records
            .iter()
            .position(|f| f.as_ref().is_some_and(|f| f.object == object))
            .map(|i| i as i16)
    }

    pub fn flag(&self, index: i16) -> Option<&Flag> {
        self.records
            .get(usize::try_from(index).ok()?)
            .and_then(Option::as_ref)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagCount {
    pub held: Vec<i16>,
    pub settled: bool,
    pub scored: Vec<usize>,
}

pub fn flag_scan_window(
    p: (i32, i32),
    radius: i32,
    (width, height): (i32, i32),
) -> (i32, i32, i32, i32) {
    let x0 = match (p.0 - radius) >> 9 {
        x if x < 0 => 0,
        x => x.min(width - 1),
    };
    let x1 = match (p.0 + radius + 0x1ff) >> 9 {
        x if x < 0 => 0,
        x => x.min(width - 1),
    };
    let y1 = (p.1 + radius + 0x1ff) >> 9;
    let y0 = match (p.1 - radius) >> 9 {
        _ if y1 < 0 => return (x0, x1 - x0, 0, 0),
        y if y < 0 => 0,
        y => y.min(height - 1),
    };
    let y1 = y1.min(height - 1);
    (x0, x1 - x0, y0, (y1 - y0 + 1).max(0))
}

pub fn flag_count_mode(game_type: i16) -> u32 {
    const MODES: [u32; 15] = [0x1a, 0x1a, 8, 8, 0xe, 0xe, 8, 8, 0xa, 9, 8, 8, 8, 8, 0xa];
    usize::try_from(game_type - 1)
        .ok()
        .and_then(|i| MODES.get(i))
        .copied()
        .unwrap_or(8)
}

impl EngineWorld {
    pub(crate) fn cell_monster_objects(
        &self,
        x: i32,
        y: i32,
    ) -> Vec<crate::projectile_groups::CellMonster> {
        self.monsters_in_cell(x, y)
            .into_iter()
            .map(|i| {
                let m = &self.monsters[i];
                crate::projectile_groups::CellMonster {
                    monster: i as i16,
                    flags: m.object.flags | if m.object.no_shape { 0x200 } else { 0 },
                    position: [m.position.0, m.position.1],
                }
            })
            .collect()
    }

    pub(crate) fn monster_middle(&self, index: i16) -> Option<[i32; 3]> {
        let m = self.monsters.get(usize::try_from(index).ok()?)?;
        let o = &m.object;
        let headers = self.collections.get(&o.collection).map_or(&[][..], |v| v);
        let (_, bottom, top) = o.bounds(headers).unwrap_or((0, 0, 0));
        Some([
            m.position.0,
            m.position.1,
            ((i32::from(bottom) + i32::from(top)) >> 1) + o.z,
        ])
    }

    pub(crate) fn mark_target_object(&mut self, identifier: u32) {
        if let Some(m) = self
            .monsters
            .iter_mut()
            .find(|m| m.unit_key() == Some(identifier))
        {
            m.object.flags |= 0x200_0000;
        }
    }

    pub(crate) fn count_team_flags(
        &mut self,
        scores: &mut [TeamScore],
        mode: u32,
        ticks: i32,
        tick: i32,
        mut rng: Option<&mut EngineRng>,
    ) -> Result<FlagCount, Unmodelled> {
        let teams = self.players.teams.len();
        let mut out = FlagCount {
            held: vec![0; teams],
            settled: true,
            scored: Vec::new(),
        };
        let radius = if self.game_type == 10 { 0x800 } else { 0xa00 };
        for slot in 0..self.flag_data.records.len() {
            let Some(mut flag) = self.flag_data.records[slot].clone() else {
                continue;
            };
            let base = if flag.kind == 0 { 0xb_u16 } else { 0xe };
            let mut sound = None;
            let (x0, cols, y0, rows) =
                flag_scan_window(flag.position, radius, (self.cells.width, self.cells.height));
            let (mut present, mut count, mut last) = (0u32, 0, -1i16);
            for i in self.monsters_in_cell_box(x0, y0, cols, rows) {
                let m = &self.monsters[i];
                let o = &m.object;
                if o.no_shape
                    || o.flags & 0x280 != 0
                    || o.kind != 2
                    || m.flags_8 & 0x8_0000 != 0
                    || matches!(self.monster_definitions[m.definition].field_1b6, 5 | 6)
                {
                    continue;
                }
                let dx = m.position.0 - flag.position.0;
                let dy = m.position.1 - flag.position.1;
                let bit = 1u32 << (m.team as u32 & 0x1f);
                if present & bit == 0 && distance2d(dx, dy) < radius {
                    count += 1;
                    present |= bit;
                    last = m.team;
                }
                if mode & 1 != 0
                    && (flag.team == -1 || flag.team == m.team)
                    && m.flags_8 & 0x800 != 0
                    && m.flags & 0x4000_0000 == 0
                    && m.team != -1
                {
                    let (identifier, team) = (m.identifier, m.team);
                    let Some(rng) = rng.as_deref_mut() else {
                        return Err(Unmodelled::new(
                            draw_site::FLAG_SCORE_INVISIBLE,
                            format!(
                                "monster {identifier} scores at a flag: make_monster_invisible \
                                 without RNG"
                            ),
                        ));
                    };
                    self.make_monster_invisible_with_mode(identifier, 1, rng, tick)?;
                    out.scored.push(i);
                    if let Some(s) = scores.get_mut(team as usize) {
                        s.score = s.score.wrapping_add(1);
                        s.score_tick = tick;
                    }
                }
            }
            flag.time = flag.time.wrapping_add(ticks);
            let candidate = match count {
                0 => flag.team,
                1 => last,
                _ => -1,
            };
            let lost = |flag: &mut Flag| {
                if flag.team != -1 {
                    flag.team = -1;
                    flag.time = 0;
                }
            };
            if flag.number == -1 {
                let holder =
                    self.unnumbered_holder(&mut out, &flag, candidate, count, present, mode);
                if i32::from(flag.team) != i32::from(holder) {
                    if flag.flags & 1 == 0 {
                        if holder == -1 {
                            sound = Some(base + 1);
                            flag.team = -1;
                            flag.time = 0;
                        } else {
                            if flag.previous_team != holder {
                                let was_local = flag.previous_team != -1
                                    && flag.previous_team == self.local_team();
                                sound = Some(base + if was_local { 2 } else { 0 });
                            }
                            flag.team = holder;
                            flag.previous_team = holder;
                            flag.time = 0;
                            if let Some(s) = scores.get_mut(holder as usize) {
                                s.field_0c = tick;
                            }
                        }
                    } else if flag.kind == 1 && holder != -1 {
                        return Err(Unmodelled::new(
                            draw_site::FLAG_REMADE,
                            format!("flag {slot}: taken, deleted and made again (marker flag 2)"),
                        ));
                    }
                }
                if mode & 8 != 0 && flag.time <= 0xb3 {
                    out.settled = false;
                }
            } else {
                let bit = 1u32 << (flag.number as u32 & 0x1f);
                let takes = count < 2
                    && candidate != -1
                    && scores
                        .get(candidate as usize)
                        .is_some_and(|s| s.numbers_taken & bit == 0);
                if takes {
                    sound = Some(base);
                    let s = &mut scores[candidate as usize];
                    s.taken_count = s.taken_count.wrapping_add(1);
                    s.numbers_taken |= bit;
                    if candidate != flag.team {
                        s.field_0c = tick;
                        flag.team = candidate;
                        flag.previous_team = candidate;
                        flag.time = 0;
                    }
                } else {
                    lost(&mut flag);
                }
                for (t, s) in scores.iter().enumerate().take(teams) {
                    if present & (1 << (t as u32 & 0x1f)) != 0 && s.numbers_taken & bit == 0 {
                        out.settled = false;
                    }
                }
            }
            self.flag_data.records[slot] = Some(flag);
            self.game.game_sound_requests.extend(sound);
        }
        Ok(out)
    }

    fn unnumbered_holder(
        &self,
        out: &mut FlagCount,
        flag: &Flag,
        candidate: i16,
        count: i32,
        present: u32,
        mode: u32,
    ) -> i16 {
        let add = |out: &mut FlagCount, team: i16| {
            if let Some(h) = usize::try_from(team).ok().and_then(|t| out.held.get_mut(t)) {
                *h += 1;
            }
        };
        let add_present = |out: &mut FlagCount| {
            for t in 0..out.held.len() {
                if present & (1 << (t as u32 & 0x1f)) != 0 {
                    out.held[t] += 1;
                }
            }
        };
        if candidate == -1 || count >= 2 {
            if present == 0 {
                if mode & 4 != 0 && flag.previous_team != -1 {
                    add(out, flag.previous_team);
                    return flag.previous_team;
                }
                if mode & 0x10 != 0 {
                    out.settled = false;
                }
            } else {
                if mode & 2 != 0 {
                    add_present(out);
                }
                out.settled = false;
            }
            return -1;
        }
        if mode & 2 == 0 || present == 0 {
            add(out, candidate);
        } else {
            add_present(out);
        }
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_rng::EngineMonster;

    #[test]
    fn the_table_of_modes() {
        assert_eq!(flag_count_mode(0), 8);
        assert_eq!(flag_count_mode(1), 0x1a);
        assert_eq!(flag_count_mode(4), 8);
        assert_eq!(flag_count_mode(5), 0xe);
        assert_eq!(flag_count_mode(7), 8);
        assert_eq!(flag_count_mode(10), 9);
        assert_eq!(flag_count_mode(15), 0xa);
        assert_eq!(flag_count_mode(16), 8);
    }

    #[test]
    fn the_scan_window_is_the_engines() {
        assert_eq!(
            flag_scan_window((5000, 5000), 0x800, (64, 64)),
            (5, 9, 5, 10)
        );
        assert_eq!(flag_scan_window((100, 100), 0xa00, (64, 64)), (0, 6, 0, 7));
        assert_eq!(
            flag_scan_window((63 * 512, 63 * 512), 0x800, (64, 64)),
            (59, 4, 59, 5)
        );
        assert_eq!(flag_scan_window((5000, -0x2000), 0x800, (64, 64)).3, 0);
    }

    fn scorer(index: usize, identifier: i16, position: (i32, i32), team: i16) -> EngineMonster {
        let mut object =
            crate::engine_rng::EngineObject::new(100 + index as i32, FourCC::new(*b"test"));
        object.kind = 2;
        EngineMonster {
            index,
            identifier,
            first_draw: 0,
            definition: 0,
            object,
            mode: 1,
            anim_state: 0x1a,
            speed: 0,
            position,
            flags: 0,
            flags_8: 0x800,
            neighbours: 0,
            impassable: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team,
            owner: 0,
            facing: 0,
            formation: Default::default(),
            cell_link: index as u64,
            flying_height: 0,
        }
    }

    #[test]
    fn monsters_score_at_a_flag_in_game_type_10() {
        use crate::engine_rng::SiteCounts;
        use mandrake_math::BungieRng;
        let mut w = EngineWorld::default();
        w.cells.width = 64;
        w.cells.height = 64;
        w.game_type = 10;
        w.formation_globals.gameplay_version = 0x73c;
        w.monster_definitions = vec![
            crate::engine_rng::MonsterDefinition::from_bytes(
                FourCC::new(*b"wam1"),
                &vec![0u8; 0x224],
            )
            .unwrap(),
        ];
        w.players.teams = vec![Default::default(); 3];
        let flag = (10 * 512 + 256, 10 * 512 + 256);
        w.monsters = vec![
            scorer(0, 1, (12 * 512 + 10, 15 * 512 + 10), 2),
            scorer(1, 2, flag, 1),
            scorer(2, 3, (15 * 512 + 10, 10 * 512), 1),
        ];
        w.flag_data.new_flag(50, None, -1, -1, 0, 0, flag);
        let mut scores = vec![TeamScore::default(); 3];
        let (mut r, mut sites) = (BungieRng::new(7), SiteCounts::default());
        let mut rng = EngineRng {
            rng: &mut r,
            sites: &mut sites,
        };
        let out = w
            .count_team_flags(&mut scores, flag_count_mode(10), 0xf, 99, Some(&mut rng))
            .unwrap();
        assert_eq!(out.scored, vec![1, 0]);
        assert_eq!(sites.get(crate::draw_site::FADE_COPY_PAUSE), 2);
        assert_eq!((scores[1].score, scores[1].score_tick), (1, 99));
        assert_eq!((scores[2].score, scores[2].score_tick), (1, 99));
        assert!(w.monsters[0].object.no_shape && w.monsters[1].object.no_shape);
        assert!(!w.monsters[2].object.no_shape);
        assert_eq!(w.fade_records.len(), 2);
        w.monsters[2].position = flag;
        let err = w
            .count_team_flags(&mut scores, flag_count_mode(10), 0xf, 100, None)
            .unwrap_err();
        assert_eq!(err.site, crate::draw_site::FLAG_SCORE_INVISIBLE);
    }

    #[test]
    fn new_flag_counts_numbers_once_and_takes_the_first_free_slot() {
        let mut d = FlagData::default();
        assert_eq!(d.new_flag(5, None, -1, -1, 0, 0, (0, 0)), 0);
        assert_eq!(d.new_flag(6, None, 2, 3, 1, 3, (0, 0)), 1);
        assert_eq!(d.new_flag(7, None, -1, 3, 2, 0, (0, 0)), 2);
        assert_eq!((d.unnumbered, d.numbered, d.numbers), (1, 1, 8));
        let f = d.flag(1).unwrap();
        assert_eq!((f.team, f.previous_team, f.flags), (2, 2, 3));
        assert_eq!(d.flag_of_object(7), Some(2));
        d.records[0] = None;
        assert_eq!(d.new_flag(8, None, -1, -1, 3, 0, (0, 0)), 0);
    }
}
