use mandrake_common::FourCC;

use crate::{Stop, TflSim, draw_site};

pub const MAX_FLAGS: usize = 0x20;

pub const OBJECT_KIND_FLAG: u8 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflFlag {
    pub kind: i16,
    pub flags: u32,
    pub object: usize,
    pub projectile: Option<FourCC>,
    pub team: i16,
    pub previous_team: i16,
    pub original_team: i16,
    pub number: i16,
    pub marker: usize,
    pub time: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagData {
    pub records: Vec<Option<TflFlag>>,
    pub pool: crate::objects::DatumPool,
    pub unnumbered: i16,
    pub numbered: i16,
    pub numbers: u32,
    pub goals: [Option<[i32; 4]>; 16],
}

impl Default for FlagData {
    fn default() -> Self {
        Self {
            records: vec![None; MAX_FLAGS],
            pool: crate::objects::DatumPool::new(MAX_FLAGS),
            unnumbered: 0,
            numbered: 0,
            numbers: 0,
            goals: [None; 16],
        }
    }
}

impl FlagData {
    pub fn flag(&self, k: usize) -> Option<&TflFlag> {
        self.records.get(k).and_then(Option::as_ref)
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &TflFlag)> {
        self.records
            .iter()
            .take(self.pool.high)
            .enumerate()
            .filter_map(|(k, f)| f.as_ref().map(|f| (k, f)))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagCount {
    pub held: Vec<i16>,
    pub settled: bool,
    pub unnumbered: i16,
    pub numbered: i16,
}

pub fn flag_count_mode(game_type: i16) -> u32 {
    match game_type {
        1 | 2 => 0x1a,
        5 | 6 => 0xe,
        10 => 9,
        _ => 8,
    }
}

use crate::statistics::TeamData;

impl TflSim {
    pub(crate) fn new_flag(
        &mut self,
        marker: usize,
        object: usize,
        team: i16,
        number: i16,
        marker_flags: u32,
    ) -> Result<bool, Stop> {
        if team != -1 {
            let claims = self.map_team_claims()?;
            let players = claims.player_teams.iter().filter(|&&t| t == team).count();
            if players == 0 {
                return Ok(false);
            }
        }
        let records = &self.flags.records;
        let Some((k, _)) = self.flags.pool.allocate(|k| records[k].is_some()) else {
            return Ok(true);
        };
        let ob = &mut self.objects[object];
        let (kind, projectile) = if ob.kind == 4 {
            ob.flags |= 0x50_0000;
            (1, ob.sleeping)
        } else {
            (0, None)
        };
        if number == -1 {
            self.flags.unnumbered = self.flags.unnumbered.wrapping_add(1);
        } else {
            let bit = 1u32.wrapping_shl(number as u32);
            if self.flags.numbers & bit == 0 {
                self.flags.numbers |= bit;
                self.flags.numbered = self.flags.numbered.wrapping_add(1);
            }
        }
        let mut flags = 0;
        if marker_flags & 2 != 0 {
            flags |= 1;
        }
        if marker_flags & 1 != 0 {
            flags |= 2;
        }
        self.flags.records[k] = Some(TflFlag {
            kind,
            flags,
            object,
            projectile,
            team: -1,
            previous_team: -1,
            original_team: team,
            number,
            marker,
            time: 0,
        });
        self.change_flag_owner(k, team);
        let ob = &mut self.objects[object];
        ob.kind = OBJECT_KIND_FLAG;
        ob.owner = k as i16;
        ob.sleeping = None;
        ob.terrain_mask = 0x8000;
        Ok(true)
    }

    fn change_flag_owner(&mut self, k: usize, team: i16) {
        let Some(f) = self.flags.records[k].as_mut() else {
            return;
        };
        f.team = team;
        f.time = 0;
        if team != -1 {
            f.previous_team = team;
        }
        let o = f.object;
        self.objects[o].render_flags &= !7;
    }

    fn new_flag_owner(&self, k: usize) -> Result<(i16, u32), Stop> {
        let f = self.flags.records[k]
            .as_ref()
            .ok_or_else(|| Stop::new(draw_site::FLAG_MISSING, format!("flag {k} not in use")))?;
        let (fx, fy) = (self.objects[f.object].x, self.objects[f.object].y);
        let (w, h) = (self.width(), self.height_cells());
        let x0 = ((fx - 0xa00) >> 10).max(0);
        let x1 = ((fx + 0xdff) >> 10).min(w - 1);
        let y0 = ((fy - 0xa00) >> 10).max(0);
        let y1 = ((fy + 0xdff) >> 10).min(h - 1);
        let (mut present, mut count, mut last) = (0u32, 0, -1i16);
        for y in y0..=y1 {
            let columns = x1 - x0;
            if columns == 0 {
                continue;
            }
            let base = self.cell_at(x0, y);
            for c in 0..columns {
                let cell = self.cell_offset(base, c, 0)?;
                let mut o = self.cell_heads[cell];
                while o >= 0 {
                    let ob = &self.objects[o as usize];
                    if ob.flags & 0x280 == 0 && ob.kind == 2 {
                        let team = self.monsters[ob.owner as usize].team as i8 as i16;
                        let bit = 1u32.wrapping_shl(team as u32);
                        if present & bit == 0
                            && crate::groups::distance((ob.x, ob.y), (fx, fy)) < 0xa00
                        {
                            count += 1;
                            last = team;
                            present |= bit;
                        }
                    }
                    o = ob.next_in_cell;
                }
            }
        }
        let holder = match count {
            0 => f.team,
            1 => last,
            _ => -1,
        };
        Ok((holder, present))
    }

    pub(crate) fn count_team_flags(&mut self, mode: u32, ticks: i32) -> Result<FlagCount, Stop> {
        let teams = self.statistics_teams()?.teams;
        if self.statistics.team_data.len() < teams {
            self.statistics.team_data.resize(teams, TeamData::default());
        }
        let local = self.local_team()?.unwrap_or(-1);
        let tick = self.tick;
        let mut out = FlagCount {
            held: vec![0; teams],
            settled: true,
            unnumbered: self.flags.unnumbered,
            numbered: self.flags.numbered,
        };
        let bump = |held: &mut [i16], t: i16| {
            if let Some(h) = usize::try_from(t).ok().and_then(|t| held.get_mut(t)) {
                *h = h.wrapping_add(1);
            }
        };
        for k in 0..self.flags.pool.high {
            if self.flags.records[k].is_none() {
                continue;
            }
            let (owner, present) = self.new_flag_owner(k)?;
            let mut f = self.flags.records[k].clone().expect("in use");
            f.time = f.time.wrapping_add(ticks);
            self.flags.records[k].as_mut().expect("in use").time = f.time;
            let ball = if f.kind != 0 { 3 } else { 0 };
            let mut sound = None;
            let mut owner = owner;
            if f.number == -1 {
                if owner == -1 {
                    if present == 0 && mode & 4 != 0 && f.previous_team != -1 {
                        owner = f.previous_team;
                    } else if mode & 0x10 != 0 || present != 0 {
                        out.settled = false;
                    }
                }
                if mode & 2 != 0 && present != 0 {
                    #[allow(clippy::needless_range_loop)]
                    for t in 0..teams {
                        if present & 1u32.wrapping_shl(t as u32) != 0 {
                            out.held[t] = out.held[t].wrapping_add(1);
                        }
                    }
                    if owner == -1 {
                        out.settled = false;
                    }
                } else if owner != -1 {
                    bump(&mut out.held, owner);
                }
                if owner != f.team {
                    if f.flags & 1 != 0 {
                        if f.kind == 1 && owner != -1 {
                            return Err(self.deflate_ball(k));
                        }
                    } else {
                        if owner == -1 {
                            sound = Some(0xc + ball);
                        } else if owner != f.previous_team {
                            sound = Some(if f.previous_team != -1 && f.previous_team == local {
                                0xd + ball
                            } else {
                                0xb + ball
                            });
                        }
                        self.change_flag_owner(k, owner);
                        if owner != -1 {
                            self.team_data_mut(owner)?.last_taken = tick as i32;
                        }
                    }
                }
                let time = self.flags.records[k].as_ref().map_or(0, |f| f.time);
                if mode & 8 != 0 && time < 0xb4 {
                    out.settled = false;
                }
            } else {
                let bit = 1u32.wrapping_shl(f.number as u32);
                if owner != -1 {
                    let d = self.team_data_mut(owner)?;
                    if d.numbers_taken & bit != 0 {
                        owner = -1;
                    } else {
                        d.numbers_taken |= bit;
                        d.field_18 = d.field_18.wrapping_add(1);
                        sound = Some(0xb + ball);
                    }
                }
                if owner != f.team {
                    if owner != -1 {
                        self.team_data_mut(owner)?.last_taken = tick as i32;
                    }
                    self.change_flag_owner(k, owner);
                }
                #[allow(clippy::needless_range_loop)]
                for t in 0..teams {
                    if present & 1u32.wrapping_shl(t as u32) != 0
                        && self.statistics.team_data[t].numbers_taken & bit == 0
                    {
                        out.settled = false;
                    }
                }
            }
            if let Some(f) = self.flags.records[k].clone()
                && f.kind == 1
                && f.team != -1
                && mode & 1 != 0
            {
                let (px, py) = (self.objects[f.object].x, self.objects[f.object].y);
                for t in 0..teams.min(16) {
                    let Some([x0, y0, x1, y1]) = self.flags.goals[t] else {
                        continue;
                    };
                    if t as i16 == f.team {
                        continue;
                    }
                    let a = ((x1 - x0) >> 10).wrapping_mul((py - y0) >> 10);
                    let b = ((px - x0) >> 10).wrapping_mul((y1 - y0) >> 10);
                    if a.wrapping_sub(b) < 0 {
                        let d = self.team_data_mut(f.team)?;
                        d.score = d.score.wrapping_add(1);
                        d.last_taken = tick as i32;
                        return Err(self.deflate_ball(k));
                    }
                }
            }
            if let Some(s) = sound {
                self.game_sound(draw_site::FLAG_SOUND, s);
            }
        }
        Ok(out)
    }

    fn team_data_mut(&mut self, team: i16) -> Result<&mut TeamData, Stop> {
        usize::try_from(team)
            .ok()
            .and_then(|t| self.statistics.team_data.get_mut(t))
            .ok_or_else(|| {
                Stop::new(
                    draw_site::FLAG_TEAM_OUT_OF_RANGE,
                    format!("a flag's team {team} past the team table"),
                )
            })
    }

    fn deflate_ball(&self, k: usize) -> Stop {
        Stop::new(
            draw_site::FLAG_DEFLATE_BALL,
            format!("tick {}: flag {k}: deflate_ball", self.tick),
        )
    }

    pub(crate) fn accelerate_flag(
        &mut self,
        def: &crate::damage::DamageDef,
        ctx: &crate::damage::DamageContext,
        o: usize,
    ) -> Result<(), Stop> {
        let Some(f) = usize::try_from(self.objects[o].owner)
            .ok()
            .and_then(|k| self.flags.flag(k))
            .cloned()
        else {
            return Err(Stop::new(
                draw_site::FLAG_MISSING,
                format!("object {o} of kind 6 is no flag's"),
            ));
        };
        if f.kind != 1 {
            return Ok(());
        }
        let ob = &mut self.objects[f.object];
        ob.kind = 4;
        ob.owner = -1;
        ob.sleeping = f.projectile;
        self.damage_projectile(def, ctx, f.object)
    }

    pub(crate) fn monster_hits_object(
        &mut self,
        i: usize,
        hit: usize,
        mons: &crate::data::Def,
    ) -> Result<bool, Stop> {
        let Some(f) = usize::try_from(self.objects[hit].owner)
            .ok()
            .and_then(|k| self.flags.flag(k))
            .cloned()
        else {
            return Err(Stop::new(
                draw_site::FLAG_MISSING,
                format!("object {hit} of kind 6 is no flag's"),
            ));
        };
        if f.kind != 1 {
            return Ok(false);
        }
        let mo = &self.objects[self.monsters[i].object];
        let ctx = crate::damage::DamageContext {
            attacker_palette: -1,
            attacker: -1,
            attacker_id: -1,
            group: None,
            point: [mo.x, mo.y, mo.z],
            velocity: mo.velocity,
            scale: 0,
            force: mons.i16(0x8a).wrapping_shl(1),
            amount: 0,
            flags: 0,
            record: None,
        };
        let def = crate::damage::DamageDef {
            kind: 4,
            ..crate::damage::DamageDef::default()
        };
        let ob = &mut self.objects[f.object];
        ob.kind = 4;
        ob.owner = -1;
        ob.sleeping = f.projectile;
        self.damage_projectile(&def, &ctx, f.object)?;
        Ok(true)
    }

    pub(crate) fn reclaim_dormant_flag(&mut self, o: usize) {
        if let Some((k, _)) = self.flags.iter().find(|(_, f)| f.object == o) {
            let ob = &mut self.objects[o];
            ob.owner = k as i16;
            ob.flags |= 0x400;
            ob.kind = OBJECT_KIND_FLAG;
            ob.sleeping = None;
        }
    }
}
