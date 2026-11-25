use mandrake_common::FourCC;
use mandrake_common::game_options::UNIT_TRADING;

use crate::commands::heapsort;
use crate::draw_site;
use crate::net_game::Chat;
use crate::setup::{TeamClaims, UNIT, marker_monster, set_marker_monster};
use crate::{Stop, TflSim};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlayerTable {
    pub claims: TeamClaims,
    pub dropped: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceCommand {
    Chat(Chat),
    Sketch {
        player: i8,
        flags: u16,
        points: Vec<i16>,
    },
    SaveLoaded {
        player: i8,
        header: [u16; 2],
        words: Vec<i16>,
    },
    OwnerChanged {
        monster: usize,
        from: i8,
        to: i8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementRow {
    pub team: i16,
    pub entry: (FourCC, bool),
    pub placed: i16,
    pub minimum: i16,
    pub maximum: i16,
    pub cost: i16,
}

fn header_and_words(p: &[u8], header: usize) -> Option<(&[u8], Vec<i16>)> {
    let head = p.get(..header)?;
    let n = i16::from_be_bytes(p.get(header..header + 2)?.try_into().ok()?);
    let words = (0..n.max(0) as usize)
        .map(|k| {
            let o = header + 2 + 2 * k;
            p.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((head, words))
}

impl TflSim {
    pub fn take_interface_commands(&mut self) -> Vec<InterfaceCommand> {
        std::mem::take(&mut self.interface_commands)
    }

    pub fn game_state(&self) -> (i16, i32) {
        let h = &self.data.film.header;
        let (limit, planning, t) = (
            h.time_limit as i32,
            h.planning_time as i32,
            self.tick as i32,
        );
        if self.map_actions.game.ended_at.is_some() {
            return (5, 0);
        }
        if self.map_actions.game.overtime {
            return (4, limit - 1);
        }
        if limit <= 0 {
            return (1, 0);
        }
        if planning > 0 && t < planning {
            return (0, limit - (planning - t));
        }
        let since = t - planning.max(0);
        if since >= limit {
            return (5, 0);
        }
        (2, since)
    }

    pub(crate) fn update_game_time(&mut self) {
        let h = &self.data.film.header;
        let (limit, planning, t) = (
            h.time_limit as i32,
            h.planning_time as i32,
            self.tick as i32,
        );
        if self.map_actions.game.ended_at.is_some() {
            return;
        }
        if self.map_actions.game.overtime {
            self.end_game();
            return;
        }
        if limit <= 0 {
            return;
        }
        if planning > 0 && t <= planning {
            if t == planning {
                self.game_sound(crate::draw_site::TIME_LEFT_SOUND, 0x19);
            }
            return;
        }
        match limit - (t - planning.max(0)) {
            0 => self.end_game(),
            0x2328 => self.game_sound(crate::draw_site::TIME_LEFT_SOUND, 0),
            0x708 => self.game_sound(crate::draw_site::TIME_LEFT_SOUND, 1),
            0x384 => self.game_sound(crate::draw_site::TIME_LEFT_SOUND, 2),
            _ => {}
        }
    }

    fn player_flags(&self, p: usize) -> u16 {
        let record = self.data.film.header.players.get(p).map_or(0, |r| r.flags);
        let dropped = self
            .player_table
            .as_ref()
            .and_then(|t| t.dropped.get(p).copied())
            .unwrap_or(false);
        record | if dropped { 4 } else { 0 }
    }

    pub fn dropped_players(&self) -> Vec<usize> {
        self.player_table.as_ref().map_or_else(Vec::new, |t| {
            t.dropped
                .iter()
                .enumerate()
                .filter(|(_, d)| **d)
                .map(|(i, _)| i)
                .collect()
        })
    }

    fn player_table_mut(&mut self) -> Result<&mut PlayerTable, Stop> {
        if self.player_table.is_none() {
            let claims = self.map_team_claims()?;
            let n = self.data.film.header.players.len();
            self.player_table = Some(PlayerTable {
                claims,
                dropped: vec![false; n],
            });
        }
        Ok(self.player_table.as_mut().expect("just set"))
    }

    pub(crate) fn drop_player(&mut self, p: i8) -> Result<(), Stop> {
        let Ok(pi) = usize::try_from(p) else {
            return Ok(());
        };
        let claims = self.team_claims()?;
        let Some(&team) = claims.player_teams.get(pi).filter(|&&t| t != -1) else {
            return Ok(());
        };
        let t = team as usize;
        let captain = claims.claims.get(t).copied().flatten();
        let players = self.data.film.header.players.len();
        let next = if captain == Some(pi) {
            (0..players).find(|&c| {
                c != pi
                    && self.player_flags(c) & 4 == 0
                    && claims.player_teams.get(c) == Some(&team)
            })
        } else {
            captain
        };
        match next {
            None => self.eliminate_team(team)?,
            Some(next) => {
                for i in 0..self.monsters.len() {
                    if self.monsters[i].identifier != 0 && self.monsters[i].player == p {
                        self.detach_monster(i, next as i16)?;
                    }
                }
                let table = self.player_table_mut()?;
                if table.claims.claims.get(t).copied().flatten() == Some(pi) {
                    table.claims.claims[t] = Some(next);
                }
                if let Some(d) = self.statistics.team_data.get_mut(t) {
                    d.players = d.players.wrapping_sub(1);
                }
            }
        }
        let table = self.player_table_mut()?;
        table.dropped[pi] = true;
        table.claims.player_teams[pi] = -1;
        Ok(())
    }

    fn eliminate_team(&mut self, team: i16) -> Result<(), Stop> {
        let Some(d) = usize::try_from(team)
            .ok()
            .and_then(|t| self.statistics.team_data.get_mut(t))
        else {
            return Ok(());
        };
        if d.flags & 1 != 0 || d.players <= 0 {
            return Ok(());
        }
        d.flags |= 1;
        let sound = if d.players != 1 { 10 } else { 9 };
        self.team_eliminated(draw_site::ELIMINATE_TEAM_SOUND, team, sound)
    }

    pub(crate) fn detach_monster(&mut self, s: usize, to: i16) -> Result<(), Stop> {
        let from = self.monsters[s].player;
        if i16::from(from) == to || to == -1 {
            return Ok(());
        }
        let Some(ti) = usize::try_from(to)
            .ok()
            .filter(|&i| i < self.data.film.header.players.len())
        else {
            return Err(Stop::new(
                draw_site::DETACH_PAST_PLAYERS,
                format!(
                    "tick {}: detach to player {to}, past the film's players",
                    self.tick
                ),
            ));
        };
        let team = self.team_claims()?.player_teams[ti];
        if self.player_flags(ti) & 4 != 0 || team != self.monsters[s].team as i8 as i16 {
            return Ok(());
        }
        let mons = self.monsters[s].definition;
        let mut entry = None;
        for k in 0..self.unit_copies.len() {
            let c = &self.unit_copies[k];
            if c.player != ti {
                continue;
            }
            let unit = c.unit;
            if self.data.def(b"unit", unit)?.tag(0) == Some(mons) {
                entry = Some(k);
                break;
            }
        }
        let Some(entry) = entry else {
            return Ok(());
        };
        let (unit, coloured) = (
            self.unit_copies[entry].unit,
            self.unit_copies[entry].permutation,
        );
        let old = self.monsters[s].palette;
        let old_entry = usize::try_from(old)
            .ok()
            .and_then(|e| self.statistics.palette.get(e))
            .copied();
        if old_entry.is_some_and(|e| e.unit == unit && e.player == to && e.mons == mons) {
            return Ok(());
        }
        let vitality = i32::from(self.monsters[s].vitality);
        self.statistics.add_monsters(old, -1, -vitality);
        let new = self.statistics.entry(unit, to, mons);
        self.statistics.add_monsters(new, 1, vitality);
        let old_coloured = old_entry.is_some_and(|e| {
            e.player >= 0
                && self.unit_copies.iter().any(|c| {
                    c.unit == e.unit && c.player as i16 == e.player && c.permutation.is_some()
                })
        });
        let o = self.monsters[s].object;
        let k = if old_coloured {
            0
        } else {
            self.objects[o].permutation_index
        };
        let index = match coloured {
            Some(q) if k == 0 => q,
            _ => k,
        };
        let core_id =
            self.data.def(b"unit", unit)?.tag(4).ok_or_else(|| {
                Stop::new(draw_site::DETACH_COLOURS, format!("unit {unit}: no core"))
            })?;
        let core = self.data.core(core_id)?;
        let value = *core
            .permutations
            .get(usize::try_from(index).unwrap_or(usize::MAX))
            .ok_or_else(|| {
                Stop::new(
                    draw_site::DETACH_COLOURS,
                    format!(
                        "tick {}: core {core_id} has no permutation {index}",
                        self.tick
                    ),
                )
            })?;
        let ob = &mut self.objects[o];
        ob.permutation = value;
        ob.permutation_index = index;
        let m = &mut self.monsters[s];
        m.palette = new;
        m.unit = unit;
        m.player = to as i8;
        m.team = team;
        self.interface_commands
            .push(InterfaceCommand::OwnerChanged {
                monster: s,
                from,
                to: to as i8,
            });
        Ok(())
    }

    fn unit_palettes(&self) -> Vec<(usize, mandrake_tag::mesh::MarkerPaletteEntry)> {
        self.data
            .mesh
            .palettes
            .iter()
            .filter(|p| p.marker_type == UNIT)
            .cloned()
            .enumerate()
            .collect()
    }

    fn palette_markers(&self, index: usize) -> Vec<usize> {
        (0..self.data.mesh.markers.len())
            .filter(|&k| {
                let m = &self.data.mesh.markers[k];
                m.marker_type == UNIT && m.palette_index as i32 == index as i32
            })
            .collect()
    }

    pub fn monster_placement_data(&mut self, team: i16) -> Result<Option<Vec<PlacementRow>>, Stop> {
        if team == -1 {
            return Ok(None);
        }
        let mut rows: Vec<PlacementRow> = Vec::new();
        for (index, p) in self.unit_palettes() {
            if p.team_index() != team || p.flags & 8 != 0 {
                continue;
            }
            let key = (p.tag_id, p.flags & 1 != 0);
            let at = match rows.iter().position(|r| r.entry == key) {
                Some(at) => at,
                None if rows.len() < 16 => {
                    let mons = self.data.def(b"unit", p.tag_id)?.tag(0).ok_or_else(|| {
                        Stop::new(
                            draw_site::PLACEMENT_DATA,
                            format!("unit {}: no mons", p.tag_id),
                        )
                    })?;
                    let cost = self.data.def(b"mons", mons)?.i16(0x19e).max(0);
                    rows.push(PlacementRow {
                        team,
                        entry: key,
                        placed: 0,
                        minimum: 0,
                        maximum: 0,
                        cost,
                    });
                    rows.len() - 1
                }
                None => continue,
            };
            for k in self.palette_markers(index) {
                let r = &mut rows[at];
                r.maximum = r.maximum.wrapping_add(1);
                if self.data.mesh.markers[k].flags & 1 == 0 {
                    r.placed = r.placed.wrapping_add(1);
                    if p.flags & 2 == 0 {
                        r.minimum = r.minimum.wrapping_add(1);
                    }
                }
            }
        }
        heapsort(&mut rows, |a, b| {
            let d = i32::from(b.cost) - i32::from(a.cost);
            if d != 0 {
                d
            } else {
                i32::from(b.maximum) - i32::from(a.maximum)
            }
        });
        Ok(Some(rows))
    }

    pub(crate) fn unit_adjustment_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let Some((_, counts)) = header_and_words(p, 2) else {
            return Err(Stop::new(
                draw_site::UNIT_ADJUSTMENT_SHORT,
                "a short unit adjustment command",
            ));
        };
        let claims = self.team_claims()?;
        let Ok(pi) = usize::try_from(player) else {
            return Err(Stop::new(
                draw_site::UNIT_ADJUSTMENT_SHORT,
                "a unit adjustment command from player -1",
            ));
        };
        let team = claims.player_teams.get(pi).copied().unwrap_or(-1);
        let captain = usize::try_from(team)
            .ok()
            .and_then(|t| claims.claims.get(t).copied().flatten());
        if team == -1 || captain != Some(pi) {
            return Ok(());
        }
        self.adjust_unit_placement(team, &counts)
    }

    pub(crate) fn adjust_unit_placement(&mut self, team: i16, counts: &[i16]) -> Result<(), Stop> {
        let Some(rows) = self.monster_placement_data(team)? else {
            return Ok(());
        };
        let n = counts.len();
        let (state, since) = self.game_state();
        if self.game_options & UNIT_TRADING == 0
            || n >= 0x10
            || !(state == 0 || (state == 2 && since < 0x1c2))
            || rows.len() != n
        {
            return Ok(());
        }
        let mut left = counts.to_vec();
        let (mut valid, mut spent, mut points) = (true, 0i32, 0i32);
        for (r, &c) in rows.iter().zip(counts) {
            valid &= r.minimum <= c && c <= r.maximum;
            points += i32::from(r.placed) * i32::from(r.cost);
            spent += i32::from(c) * i32::from(r.cost);
        }
        for (r, c) in rows.iter().zip(&mut left) {
            let cost = i32::from(r.cost);
            let mut rest = points - spent;
            if cost > rest {
                continue;
            }
            loop {
                if *c >= r.maximum {
                    break;
                }
                *c += 1;
                rest -= cost;
                spent += cost;
                if cost > rest {
                    break;
                }
            }
        }
        if spent > points || !valid {
            return Ok(());
        }
        for (index, p) in self.unit_palettes() {
            let key = (p.tag_id, p.flags & 1 != 0);
            let Some(at) = rows.iter().position(|r| r.entry == key) else {
                continue;
            };
            if p.team_index() != team || p.flags & 8 != 0 {
                continue;
            }
            let markers = self.palette_markers(index);
            for &k in &markers {
                let existing = marker_monster(&self.data.mesh.markers[k]);
                if existing != -1 {
                    let i = existing as usize;
                    let (entry, vitality) = self
                        .monsters
                        .get(i)
                        .map_or((-1, 0), |m| (m.palette, i32::from(m.vitality)));
                    self.statistics.add_monsters(entry, -1, -vitality);
                    self.remove_monster(i)?;
                    set_marker_monster(&mut self.data.mesh.markers[k], None);
                }
                let m = &mut self.data.mesh.markers[k];
                if m.flags & 1 == 0 {
                    if left[at] != 0 {
                        left[at] -= 1;
                    } else {
                        m.flags |= 1;
                    }
                }
            }
            for &k in &markers {
                let m = &mut self.data.mesh.markers[k];
                if m.flags & 1 != 0 && left[at] != 0 {
                    m.flags &= !1;
                    left[at] -= 1;
                }
            }
            for v in &mut self.monster_initializers {
                v.used = false;
            }
            for &k in &markers {
                let m = self.data.mesh.markers[k].clone();
                let made = self.new_monster(k, &m, &p, true)?;
                set_marker_monster(&mut self.data.mesh.markers[k], made);
            }
        }
        Ok(())
    }

    pub(crate) fn sketch_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let (head, points) = header_and_words(p, 2)
            .ok_or_else(|| Stop::new(draw_site::SKETCH_COMMAND_SHORT, "a short sketch command"))?;
        let flags = u16::from_be_bytes([head[0], head[1]]);
        self.interface_commands.push(InterfaceCommand::Sketch {
            player,
            flags,
            points,
        });
        Ok(())
    }

    pub(crate) fn save_loaded_command(&mut self, player: i8, p: &[u8]) -> Result<(), Stop> {
        let (head, words) = header_and_words(p, 4).ok_or_else(|| {
            Stop::new(
                draw_site::SAVE_LOADED_COMMAND_SHORT,
                "a short save loaded command",
            )
        })?;
        let header = [
            u16::from_be_bytes([head[0], head[1]]),
            u16::from_be_bytes([head[2], head[3]]),
        ];
        self.interface_commands.push(InterfaceCommand::SaveLoaded {
            player,
            header,
            words,
        });
        Ok(())
    }

    pub(crate) fn chat_command(&mut self, player: i8, p: &[u8]) {
        self.interface_commands
            .push(InterfaceCommand::Chat(Chat::of_payload(player, p)));
    }
}
