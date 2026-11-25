use crate::players::Players;
use crate::statistics::qsort_private;
use mandrake_common::FourCC;
use mandrake_rec::header::MonsterInitializer;

pub const MAX_BUILT: usize = 400;

pub const NAME_LENGTH: usize = 17;

pub type CustomName = [u8; NAME_LENGTH + 1];

pub fn custom_name(name: &str) -> CustomName {
    let mut out = [0; NAME_LENGTH + 1];
    let bytes = mandrake_rec::mac_roman::encode(name);
    let n = bytes.len().min(NAME_LENGTH);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

pub fn custom_name_text(name: &CustomName) -> String {
    let n = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    mandrake_rec::mac_roman::decode(&name[..n])
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    used: bool,
    initializer: MonsterInitializer,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MonsterInitializers {
    records: Vec<Record>,
    enabled: bool,
}

impl MonsterInitializers {
    pub fn for_new_map(records: &[MonsterInitializer], options: u32) -> Self {
        let keep: &[MonsterInitializer] =
            if options & mandrake_common::game_options::CARRY_VETERANS != 0 {
                records
            } else {
                &[]
            };
        Self {
            records: keep
                .iter()
                .map(|initializer| Record {
                    used: false,
                    initializer: initializer.clone(),
                })
                .collect(),
            enabled: options & mandrake_common::game_options::VETERANS != 0,
        }
    }

    pub fn records(&self) -> Vec<MonsterInitializer> {
        self.records.iter().map(|r| r.initializer.clone()).collect()
    }

    pub fn can_give(&self) -> bool {
        self.enabled && !self.records.is_empty()
    }

    pub fn mark_unused(&mut self) {
        for r in &mut self.records {
            r.used = false;
        }
    }

    pub fn take(
        &mut self,
        unit: FourCC,
        monster: FourCC,
        owner: i16,
        players: &Players,
        tags_exist: impl Fn(&MonsterInitializer) -> bool,
    ) -> Option<(usize, MonsterInitializer)> {
        if !self.enabled || owner == -1 {
            return None;
        }
        let player = players.players.get(owner as usize)?;
        let mut best: Option<usize> = None;
        for (i, r) in self.records.iter().enumerate() {
            let v = &r.initializer;
            if r.used || (v.unit_tag != unit && v.monster_tag != monster) || !tags_exist(v) {
                continue;
            }
            let on_side = v.player_id == player.id as u32
                || players
                    .players
                    .iter()
                    .any(|p| p.id as u32 == v.player_id && p.team == player.team);
            if !on_side {
                continue;
            }
            if best.is_none_or(|b| self.records[b].initializer.experience < v.experience) {
                best = Some(i);
            }
        }
        let i = best?;
        self.records[i].used = true;
        Some((i, self.records[i].initializer.clone()))
    }

    pub fn release(&mut self, index: usize) {
        if let Some(r) = self.records.get_mut(index) {
            r.used = false;
        }
    }

    pub fn build(&mut self, survivors: impl IntoIterator<Item = MonsterInitializer>) {
        let mut built: Vec<MonsterInitializer> = self
            .records
            .iter()
            .filter(|r| !r.used && r.initializer.experience != 0)
            .map(|r| r.initializer.clone())
            .collect();
        for s in survivors {
            built.push(s);
            if built.len() >= MAX_BUILT {
                break;
            }
        }
        let mut order: Vec<(u8, usize)> = built
            .iter()
            .enumerate()
            .map(|(i, v)| (v.experience, i))
            .collect();
        qsort_private(&mut order, |a, b| i32::from(b.0) - i32::from(a.0));
        self.records = order
            .into_iter()
            .map(|(_, i)| Record {
                used: false,
                initializer: built[i].clone(),
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::players::Player;

    const OPTIONS: u32 = 0x60;

    fn record(unit: &[u8; 4], monster: &[u8; 4], player: u32, xp: u8) -> MonsterInitializer {
        MonsterInitializer {
            player_id: player,
            unit_tag: FourCC(*unit),
            monster_tag: FourCC(*monster),
            experience: xp,
            field_8f_plus_one: 1,
            permutation: 0,
            name_index: xp,
            name: String::new(),
        }
    }

    fn players() -> Players {
        let player = |id, team| Player {
            flags: 0,
            field_09: 0,
            team,
            field_0b: team,
            id,
        };
        Players {
            players: vec![player(7, 0), player(8, 0), player(9, 1)],
            ..Default::default()
        }
    }

    fn take(
        t: &mut MonsterInitializers,
        unit: &[u8; 4],
        monster: &[u8; 4],
        owner: i16,
    ) -> Option<u8> {
        t.take(FourCC(*unit), FourCC(*monster), owner, &players(), |_| true)
            .map(|(_, v)| v.experience)
    }

    #[test]
    fn a_new_map_keeps_the_table_only_under_option_0x40_and_gives_only_under_0x20() {
        let r = [record(b"wam1", b"wam1", 7, 3)];
        assert!(
            MonsterInitializers::for_new_map(&r, 0x20)
                .records()
                .is_empty()
        );
        let mut kept = MonsterInitializers::for_new_map(&r, 0x40);
        assert_eq!(kept.records(), r);
        assert_eq!(take(&mut kept, b"wam1", b"wam1", 0), None);
        let mut both = MonsterInitializers::for_new_map(&r, OPTIONS);
        assert_eq!(take(&mut both, b"wam1", b"wam1", 0), Some(3));
    }

    #[test]
    fn the_lookup_takes_the_first_of_the_highest_once() {
        let r = [
            record(b"wam1", b"wam1", 7, 2),
            record(b"wam1", b"wam1", 7, 9),
            record(b"wam1", b"wam1", 7, 9),
            record(b"arch", b"arch", 7, 50),
        ];
        let mut t = MonsterInitializers::for_new_map(&r, OPTIONS);
        let got = t
            .take(FourCC(*b"wam1"), FourCC(*b"wam1"), 0, &players(), |_| true)
            .unwrap();
        assert_eq!(got.0, 1);
        assert_eq!(take(&mut t, b"wam1", b"wam1", 0), Some(9));
        assert_eq!(take(&mut t, b"wam1", b"wam1", 0), Some(2));
        assert_eq!(take(&mut t, b"wam1", b"wam1", 0), None);
        t.release(1);
        assert_eq!(take(&mut t, b"wam1", b"wam1", 0), Some(9));
        t.mark_unused();
        assert_eq!(take(&mut t, b"wam1", b"wam1", 0), Some(9));
    }

    #[test]
    fn the_lookup_goes_by_unit_or_monster_side_and_existing_tags() {
        let r = [record(b"unit", b"mons", 7, 4)];
        let fresh = || MonsterInitializers::for_new_map(&r, OPTIONS);
        assert_eq!(take(&mut fresh(), b"unit", b"xxxx", 0), Some(4));
        assert_eq!(take(&mut fresh(), b"xxxx", b"mons", 0), Some(4));
        assert_eq!(take(&mut fresh(), b"xxxx", b"yyyy", 0), None);
        assert_eq!(take(&mut fresh(), b"unit", b"mons", 1), Some(4));
        assert_eq!(take(&mut fresh(), b"unit", b"mons", 2), None);
        assert_eq!(take(&mut fresh(), b"unit", b"mons", -1), None);
        let got = fresh().take(FourCC(*b"unit"), FourCC(*b"mons"), 0, &players(), |_| false);
        assert_eq!(got, None);
    }

    #[test]
    fn the_build_keeps_unused_records_with_experience_sorts_and_stops_at_400() {
        let r = [
            record(b"used", b"used", 7, 9),
            record(b"keep", b"keep", 7, 5),
            record(b"none", b"none", 7, 0),
        ];
        let mut t = MonsterInitializers::for_new_map(&r, OPTIONS);
        assert_eq!(take(&mut t, b"used", b"used", 0), Some(9));
        t.build([
            record(b"newa", b"newa", 7, 1),
            record(b"newb", b"newb", 7, 8),
        ]);
        let tags: Vec<_> = t
            .records()
            .iter()
            .map(|v| (v.unit_tag.0, v.experience))
            .collect();
        assert_eq!(tags, [(*b"newb", 8), (*b"keep", 5), (*b"newa", 1)]);

        let mut t = MonsterInitializers::for_new_map(&[], OPTIONS);
        t.build((0..500).map(|i| record(b"wam1", b"wam1", 7, (i % 200) as u8)));
        let built = t.records();
        assert_eq!(built.len(), MAX_BUILT);
        assert!(built.windows(2).all(|w| w[0].experience >= w[1].experience));
    }

    #[test]
    fn a_name_is_cut_as_strlcpy_cuts_it() {
        assert_eq!(custom_name_text(&custom_name("Vet")), "Vet");
        assert_eq!(custom_name_text(&custom_name("")), "");
        assert_eq!(
            custom_name_text(&custom_name("abcdefghijklmnopqrstuvwxyz")).len(),
            17
        );
    }
}
