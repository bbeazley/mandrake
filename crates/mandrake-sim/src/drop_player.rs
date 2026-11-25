use crate::engine_rng::{EngineRng, EngineWorld, Unmodelled};
use mandrake_common::game_options;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DroppedPlayer {
    pub detached: Vec<usize>,
    pub eliminated: Option<i16>,
}

impl EngineWorld {
    pub fn drop_player(
        &mut self,
        p: i16,
        rng: Option<&mut EngineRng>,
    ) -> Result<DroppedPlayer, Unmodelled> {
        let mut out = DroppedPlayer::default();
        let Some(index) = usize::try_from(p)
            .ok()
            .filter(|&i| i < self.players.players.len())
        else {
            return Ok(out);
        };
        let gameplay = self.formation_globals.gameplay_version;
        let team = self.players.players[index].team;
        if team != -1 {
            let t = i16::from(team);
            let captain = self.players.teams.get(t as usize).map_or(0, |r| r.player);
            let next = if captain == p {
                (0..self.players.players.len())
                    .find(|&j| {
                        let pl = &self.players.players[j];
                        j != index && pl.flags & 4 == 0 && pl.team == team
                    })
                    .map_or(-1, |j| j as i16)
            } else {
                captain
            };
            if next != -1 {
                for i in 0..self.monsters.len() {
                    if self.monsters[i].identifier == 0 {
                        continue;
                    }
                    if self.monsters[i].owner == p {
                        let owner = self.monsters[i].owner;
                        self.detach_monster(i, next)?;
                        if self.monsters[i].owner != owner {
                            out.detached.push(i);
                        }
                    }
                    if let Some(tag) = self.held_row_of(i, p) {
                        let row = self
                            .statistics
                            .find_unit(next, tag)
                            .map_or(-1, |r| r as i16);
                        self.combat_mut(i).held_unit = row;
                    }
                }
                if let Some(record) = self.players.teams.get_mut(t as usize) {
                    if record.player == p {
                        record.player = next;
                    }
                    record.players = record.players.wrapping_sub(1);
                }
            } else {
                for i in 0..self.monsters.len() {
                    if self.monsters[i].identifier != 0 && self.held_row_of(i, p).is_some() {
                        self.combat_mut(i).held_unit = -1;
                    }
                }
                out.eliminated = Some(t);
                self.eliminate_team(t, rng)?;
            }
            if gameplay > 0x71b {
                self.statistics.delete_player_rows(p, gameplay);
                self.unit_rows.retain(|_, r| r.owner != p);
            }
            if gameplay > 0x713 {
                self.player_formations.retain(|(q, _)| *q != p);
            }
            let pl = &mut self.players.players[index];
            pl.flags |= 4;
            pl.team = -1;
        }
        if gameplay > game_options::GAMEPLAY_1_8_4 {
            let pl = &mut self.players.players[index];
            pl.flags |= 4;
            pl.id = -1;
        }
        Ok(out)
    }

    fn held_row_of(&self, i: usize, p: i16) -> Option<u32> {
        let held = self.combat(i).held_unit;
        if held == -1 {
            return None;
        }
        let row = self.statistics.units.get(usize::try_from(held).ok()?)?;
        (row.owner == p).then_some(row.tag)
    }
}
