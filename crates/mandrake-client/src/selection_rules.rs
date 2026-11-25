use std::collections::HashMap;

use mandrake_sim::engine_rng::{EngineMonster, EngineWorld, MonsterDefinition};

use crate::selection_input::{EVAL_ALLIED, EVAL_CONTROLLED, MonsterPick, SELECTION_LIMIT};

const PALETTE_ALLY: u32 = 0x1000_0000;
const TARGET_HIGHLIGHT: u32 = 0x4000_0000;

fn definition_collection(d: &MonsterDefinition) -> [u8; 4] {
    let r = &d.ai.raw;
    if r.len() < 8 {
        return [0; 4];
    }
    [r[4], r[5], r[6], r[7]]
}

pub fn evaluate_monster(w: &EngineWorld, i: usize) -> u8 {
    let m = &w.monsters[i];
    if !w.teams_are_allied(w.local_team(), m.team) {
        return 0;
    }
    let o = u8::from(m.owner == w.local_player);
    let mut e = if w.players.local_without_team(w.local_player) {
        4 * o + 1
    } else {
        5 * o
    };
    if m.flags & PALETTE_ALLY != 0 || w.mode == 0 {
        e &= !1;
    }
    e | EVAL_ALLIED
}

pub fn under_media(w: &EngineWorld, i: usize, margin: i32) -> bool {
    let m = &w.monsters[i];
    if m.object.flags & 0x80 == 0 || w.mode == 0 {
        return false;
    }
    if w.mode == 1 && w.local_team() == -1 {
        return false;
    }
    let mut e = evaluate_monster(w, i) & EVAL_ALLIED != 0;
    let d = &w.monster_definitions[m.definition];
    if (m.flags_8 & 0x1000 != 0 || d.field_1b6 == 5) && d.flags & 0x0200_0000 != 0 {
        e = false;
    }
    if e {
        return false;
    }
    let (cols, rows) = w.mesh_cells;
    let (x, y) = (m.position.0 >> 9, m.position.1 >> 9);
    if x < 0 || y < 0 || x >= cols || y >= rows {
        return false;
    }
    let Some(&(_, media)) = w.heights.get((y * cols + x) as usize) else {
        return false;
    };
    let headers = w
        .collections
        .get(&m.object.collection)
        .map_or(&[][..], |v| v);
    let top = m
        .object
        .bounds(headers)
        .map_or(0, |b| i32::from(b.1.max(b.2)));
    top + m.object.z < i32::from(media) - margin
}

pub fn band_eligible(w: &EngineWorld, id: u32) -> bool {
    let Some(i) = w.monster_index_by_identifier(id as u16) else {
        return false;
    };
    let m = &w.monsters[i];
    let d = &w.monster_definitions[m.definition];
    m.flags_8 & 0x1000 == 0 && d.field_1b6 != 5 && evaluate_monster(w, i) & EVAL_CONTROLLED != 0
}

pub fn monster_pick(w: &EngineWorld, id: u32, selected: bool) -> Option<MonsterPick> {
    let i = w.monster_index_by_identifier(id as u16)?;
    let m = &w.monsters[i];
    let d = &w.monster_definitions[m.definition];
    Some(MonsterPick {
        id,
        evaluation: evaluate_monster(w, i),
        forced: m.flags_8 & 0x1000 != 0 || d.field_1b6 == 5,
        ambient: d.flags & 0x0200_0000 != 0,
        flag_80000: d.flags & 0x0008_0000 != 0,
        flags_8: m.flags_8,
        same_team: m.team == w.local_team(),
        owned: m.owner == w.local_player,
        selected,
        near_miss: false,
    })
}

pub fn preset_member_commanded(w: &EngineWorld, id: u32) -> bool {
    let Some(i) = w.monster_index_by_identifier(id as u16) else {
        return false;
    };
    let m = &w.monsters[i];
    let d = &w.monster_definitions[m.definition];
    let forced = m.flags_8 & 0x1000 != 0 || d.field_1b6 == 5;
    evaluate_monster(w, i) & EVAL_CONTROLLED != 0 && !forced && m.flags & TARGET_HIGHLIGHT == 0
}

pub fn selection_may_take(w: &EngineWorld, i: usize) -> bool {
    if w.mode == 0 {
        return true;
    }
    let m = &w.monsters[i];
    let d = &w.monster_definitions[m.definition];
    if m.flags_8 & 0x1000 == 0 && d.field_1b6 != 5 {
        return d.flags & 0x0008_0000 == 0;
    }
    d.flags & 0x0200_0000 != 0
        || (w.game_type == 11 && m.flags_8 & 0x800 != 0)
        || (w.game_type == 12 && m.flags_8 & 0x2000 != 0)
}

pub fn all_controlled(w: &EngineWorld, selection: &[u32]) -> bool {
    selection.iter().all(|&id| {
        let Some(i) = w.monster_index_by_identifier(id as u16) else {
            return true;
        };
        let m = &w.monsters[i];
        let d = &w.monster_definitions[m.definition];
        let forced = m.flags_8 & 0x1000 != 0 || d.field_1b6 == 5;
        evaluate_monster(w, i) & EVAL_CONTROLLED != 0 && !forced && m.flags & TARGET_HIGHLIGHT == 0
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloodMonster {
    pub id: u32,
    pub cell: (i32, i32),
    pub cell_link: u64,
    pub no_shape: bool,
    pub team: i16,
    pub owner: i16,
    pub definition: usize,
    pub collection: [u8; 4],
    pub definition_only: bool,
    pub flags_8: u32,
    pub flags_4: u32,
}

impl FloodMonster {
    fn of(w: &EngineWorld, m: &EngineMonster) -> Self {
        let d = &w.monster_definitions[m.definition];
        Self {
            id: u32::from(m.identifier as u16),
            cell: (m.position.0 >> 9, m.position.1 >> 9),
            cell_link: m.cell_link,
            no_shape: m.object.no_shape,
            team: m.team,
            owner: m.owner,
            definition: m.definition,
            collection: definition_collection(d),
            definition_only: d.field_199 & 1 != 0,
            flags_8: m.flags_8,
            flags_4: m.flags,
        }
    }
}

pub fn same_type_flood(
    monsters: &[FloodMonster],
    base: u32,
    mesh: (i32, i32),
    local_team: i16,
    alliances: &mandrake_sim::ally::AllianceView,
) -> Vec<u32> {
    let Some(b) = monsters.iter().find(|m| m.id == base).copied() else {
        return Vec::new();
    };
    let mut cells: HashMap<(i32, i32), Vec<&FloodMonster>> = HashMap::new();
    for m in monsters.iter().filter(|m| !m.no_shape) {
        cells.entry(m.cell).or_default().push(m);
    }
    for list in cells.values_mut() {
        list.sort_by_key(|m| std::cmp::Reverse(m.cell_link));
    }
    let by_id: HashMap<u32, &FloodMonster> = monsters.iter().map(|m| (m.id, m)).collect();
    let b_allied_4000 = alliances.teams_are_allied(b.team, local_team) && b.flags_8 & 0x4000 != 0;
    let same_type = |c: &FloodMonster| {
        let mut v = c.owner == b.owner && c.collection == b.collection;
        if v && (b.definition_only || c.definition_only) {
            v = c.definition == b.definition;
        }
        if c.flags_8 & 0x1000 != 0 && b.flags_8 & 0x1000 == 0 {
            return false;
        }
        if b_allied_4000 && c.flags_8 & 0x4000 == 0 {
            return false;
        }
        v
    };
    let corner = |c: i32, size: i32| if c - 4 < 0 { 0 } else { (c - 4).min(size - 9) };
    let mut list = vec![base];
    let mut i = 0;
    while i < list.len() {
        let Some(at) = by_id.get(&list[i]) else {
            i += 1;
            continue;
        };
        let (x0, y0) = (corner(at.cell.0, mesh.0), corner(at.cell.1, mesh.1));
        for y in y0..y0 + 9 {
            for x in x0..x0 + 9 {
                for c in cells.get(&(x, y)).into_iter().flatten() {
                    if same_type(c) && !list.contains(&c.id) && list.len() < SELECTION_LIMIT {
                        list.push(c.id);
                    }
                }
            }
        }
        i += 1;
    }
    list.retain(|id| by_id.get(id).is_some_and(|m| m.flags_4 & PALETTE_ALLY == 0));
    list
}

pub fn engine_same_type_flood(w: &EngineWorld, base: u32) -> Vec<u32> {
    let monsters: Vec<FloodMonster> = w
        .monsters
        .iter()
        .filter(|m| m.identifier != 0)
        .map(|m| FloodMonster::of(w, m))
        .collect();
    same_type_flood(
        &monsters,
        base,
        w.mesh_cells,
        w.local_team(),
        &w.alliance_view(),
    )
}

pub fn select_all(
    drawn: impl IntoIterator<Item = u32>,
    eligible: impl Fn(u32) -> bool,
    selection: &[u32],
    replay: bool,
) -> Option<Vec<u32>> {
    if replay {
        return None;
    }
    let mut list = Vec::new();
    let mut all_selected = true;
    for id in drawn {
        if !eligible(id) || list.len() >= SELECTION_LIMIT {
            continue;
        }
        if all_selected && !selection.contains(&id) {
            all_selected = false;
        }
        list.push(id);
    }
    Some(if all_selected { Vec::new() } else { list })
}

pub fn select_all_eligible(w: &EngineWorld, id: u32) -> bool {
    let Some(i) = w.monster_index_by_identifier(id as u16) else {
        return false;
    };
    let m = &w.monsters[i];
    let d = &w.monster_definitions[m.definition];
    m.owner == w.local_player
        && m.flags & PALETTE_ALLY == 0
        && m.flags_8 & 0x1000 == 0
        && d.field_1b6 != 5
        && d.flags & 0x0008_0000 == 0
}

pub fn fallback_pick(id: u32, own: bool, selected: bool) -> MonsterPick {
    if own {
        MonsterPick::own(id, selected)
    } else {
        MonsterPick {
            selected,
            ..MonsterPick::enemy(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warrior(id: u32, cell: (i32, i32)) -> FloodMonster {
        FloodMonster {
            id,
            cell,
            cell_link: u64::from(id),
            no_shape: false,
            team: 0,
            owner: 0,
            definition: 2,
            collection: *b"warr",
            definition_only: false,
            flags_8: 0x20000,
            flags_4: 0x8000,
        }
    }

    #[test]
    fn le3e_flood_takes_the_warriors_block_by_block() {
        let mut ms = vec![
            warrior(21, (30, 81)),
            warrior(14, (27, 77)),
            warrior(36, (32, 77)),
            warrior(41, (26, 78)),
            warrior(50, (31, 79)),
            warrior(76, (29, 75)),
            warrior(60, (27, 76)),
        ];
        ms.push(FloodMonster {
            collection: *b"avis",
            definition: 3,
            ..warrior(53, (30, 86))
        });
        ms.push(FloodMonster {
            collection: *b"arch",
            definition: 4,
            ..warrior(7, (30, 81))
        });
        ms.push(FloodMonster {
            flags_4: 0x1000_8000,
            ..warrior(8, (31, 81))
        });
        let list = same_type_flood(&ms, 21, (192, 192), 0, &Default::default());
        assert_eq!(list, [21, 14, 36, 41, 50, 76, 60]);
    }

    #[test]
    fn flood_block_is_clamped_to_the_mesh() {
        let ms = vec![
            warrior(1, (2, 190)),
            warrior(2, (0, 183)),
            warrior(3, (8, 191)),
            warrior(4, (9, 191)),
            warrior(5, (0, 182)),
        ];
        assert_eq!(
            same_type_flood(&ms, 1, (192, 192), 0, &Default::default()),
            [1, 2, 3, 5, 4]
        );
    }

    #[test]
    fn flood_type_rules() {
        let b = warrior(1, (10, 10));
        let ms = vec![
            b,
            FloodMonster {
                owner: 1,
                ..warrior(2, (10, 10))
            },
            FloodMonster {
                flags_8: 0x1000,
                ..warrior(3, (10, 11))
            },
            FloodMonster {
                definition: 9,
                ..warrior(4, (10, 12))
            },
            FloodMonster {
                no_shape: true,
                ..warrior(5, (10, 12))
            },
        ];
        assert_eq!(
            same_type_flood(&ms, 1, (64, 64), 0, &Default::default()),
            [1, 4]
        );
        let mut ms2 = ms.clone();
        ms2[0].definition_only = true;
        assert_eq!(
            same_type_flood(&ms2, 1, (64, 64), 0, &Default::default()),
            [1]
        );
        let mut ms3 = ms.clone();
        ms3[0].flags_8 |= 0x4000;
        assert_eq!(
            same_type_flood(&ms3, 1, (64, 64), 0, &Default::default()),
            [1]
        );
        ms3[3].flags_8 |= 0x4000;
        assert_eq!(
            same_type_flood(&ms3, 1, (64, 64), 0, &Default::default()),
            [1, 4]
        );
        let ms4 = vec![
            warrior(1, (10, 10)),
            FloodMonster {
                cell_link: 5,
                ..warrior(2, (11, 10))
            },
            FloodMonster {
                cell_link: 9,
                ..warrior(3, (11, 10))
            },
        ];
        assert_eq!(
            same_type_flood(&ms4, 1, (64, 64), 0, &Default::default()),
            [1, 3, 2]
        );
    }

    #[test]
    fn select_all_toggles_and_caps() {
        let drawn = [5, 3, 9, 4];
        let eligible = |id: u32| id != 9;
        assert_eq!(select_all(drawn, eligible, &[], false), Some(vec![5, 3, 4]));
        assert_eq!(
            select_all(drawn, eligible, &[4, 3, 5, 7], false),
            Some(vec![])
        );
        assert_eq!(
            select_all(drawn, eligible, &[4, 3], false),
            Some(vec![5, 3, 4])
        );
        assert_eq!(select_all([9], eligible, &[1], false), Some(vec![]));
        assert_eq!(select_all(drawn, eligible, &[], true), None);
        let many: Vec<u32> = (1..=200).collect();
        assert_eq!(
            select_all(many, |_| true, &[], false).unwrap().len(),
            SELECTION_LIMIT
        );
    }
}
