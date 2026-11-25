use crate::draw_site;
use crate::engine_move::distance2d;
use crate::engine_rng::{EngineWorld, MovementHeader, Unmodelled};
use crate::map_actions::{
    MapActionContext, MapActions, TYPE_DISTANCE, TYPE_INTEGER, TYPE_MONSTERS,
};
use crate::spatial_groups::{SpatialSupergroup, supergroups};
use mandrake_common::game_options;

pub const ATTACK_BUFFER: i64 = 0x4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackConfig {
    pub flags: u32,
    pub point: (i32, i32),
    pub radius: i32,
}

impl AttackConfig {
    pub fn new() -> Self {
        AttackConfig {
            flags: 1,
            point: (0, 0),
            radius: 0,
        }
    }
}

impl Default for AttackConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackRequest {
    pub config: AttackConfig,
    pub monsters: Vec<(usize, i16)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttackQueue {
    pub requests: Vec<AttackRequest>,
    pub used: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackOrder {
    pub source: i32,
    pub flags: u16,
    pub monsters: Vec<usize>,
    pub targets: Vec<usize>,
}

pub fn reset_attack_ai(w: &mut EngineWorld) {
    w.attack_queue = AttackQueue::default();
}

pub fn attack_with_monster_indexes(w: &mut EngineWorld, monsters: &[usize], config: AttackConfig) {
    if monsters.is_empty() {
        return;
    }
    let size = i64::from((monsters.len() as i32 * 4 + 0x20) as i16);
    if w.attack_queue.used + size > ATTACK_BUFFER {
        return;
    }
    let monsters = monsters
        .iter()
        .map(|&m| (m, w.monsters[m].identifier))
        .collect();
    w.attack_queue
        .requests
        .push(AttackRequest { config, monsters });
    w.attack_queue.used += size;
}

pub fn handle_target_command(
    w: &mut EngineWorld,
    source: i32,
    flags: u16,
    monsters: &[usize],
    targets: &[usize],
) -> Result<(), Unmodelled> {
    let max = if w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3 {
        0x7e
    } else {
        0x1e
    };
    let monsters = &monsters[..monsters.len().min(max)];
    let targets = &targets[..targets.len().min(max)];
    w.pending_attacks.push(AttackOrder {
        source,
        flags,
        monsters: monsters.to_vec(),
        targets: targets.to_vec(),
    });
    if source == -1 {
        return Err(Unmodelled::new(
            draw_site::LOCAL_TARGET_COMMAND,
            "a local player's TARGET command from the map actions",
        ));
    }
    let player = w.local_player;
    w.execute_target_command(source, player, flags, monsters, targets)
}

#[derive(Debug, Clone, Default)]
struct GroupLoad {
    strength: u16,
    load: i32,
    attackers: Vec<usize>,
}

pub fn update_attack_ai(w: &mut EngineWorld) -> Result<(), Unmodelled> {
    w.apply_pending_orders()?;
    if w.attack_queue.requests.is_empty() {
        return Ok(());
    }
    let ngroups = w.spatial_groups.len();
    let mut loads = vec![GroupLoad::default(); ngroups];
    let requests = std::mem::take(&mut w.attack_queue.requests);
    let gameplay = w.formation_globals.gameplay_version;
    for req in requests.iter().rev() {
        for &(m, identifier) in &req.monsters {
            let mon = &w.monsters[m];
            if gameplay > game_options::GAMEPLAY_1_7 && mon.identifier != identifier {
                continue;
            }
            let def = &w.monster_definitions[mon.definition];
            if def.field_1b6 == 0 && mon.flags & 0x101 == 0x101 {
                continue;
            }
            let (team, position, add) = (mon.team, mon.position, def.spatial_group_strength as u16);
            let (mut best, mut best_score) = (None, -1i32);
            for (g, group) in w.spatial_groups.iter().enumerate() {
                if group.team == team || w.teams_are_allied(team, group.team) {
                    continue;
                }
                let d = if req.config.flags & 1 == 0 {
                    let p = req.config.point;
                    distance2d(p.0 - group.center.0, p.1 - group.center.1) - i32::from(group.radius)
                } else {
                    0
                };
                if req.config.radius < d {
                    continue;
                }
                let d = distance2d(position.0 - group.center.0, position.1 - group.center.1)
                    - i32::from(group.radius);
                let near = (0x2800 - d).max(0);
                let score = near.wrapping_add(0x18000i32.wrapping_sub(loads[g].load) >> 4);
                if best_score == -1 || score > best_score {
                    best = Some(g);
                    best_score = score;
                }
            }
            let Some(g) = best else {
                continue;
            };
            let l = &mut loads[g];
            l.strength = l.strength.wrapping_add(add);
            let strength = i32::from(w.spatial_groups[g].strength);
            l.load = if strength == 0 {
                0x7fff_ffff
            } else {
                ((u32::from(l.strength) << 16) as i32).wrapping_div(strength)
            };
            if w.attack_queue.used + 0x10 > ATTACK_BUFFER {
                continue;
            }
            l.attackers.push(m);
            w.attack_queue.used += 0x10;
        }
    }
    w.attack_queue.requests = requests;
    let per_command = if gameplay > game_options::GAMEPLAY_1_3 {
        0x7e
    } else {
        0x1e
    };
    for (g, load) in loads.iter_mut().enumerate() {
        let mut attackers = std::mem::take(&mut load.attackers);
        if attackers.is_empty() {
            continue;
        }
        attackers.reverse();
        if gameplay > game_options::GAMEPLAY_1_6 {
            attackers.truncate(0x190);
        }
        let members = w.spatial_groups[g].members.clone();
        let mut chosen: Vec<Option<usize>> = vec![None; attackers.len()];
        for (a, &m) in attackers.iter().enumerate() {
            let mut best = -1i32;
            for &t in &members {
                let (dm, dt) = (
                    &w.monster_definitions[w.monsters[m].definition],
                    &w.monster_definitions[w.monsters[t].definition],
                );
                let bonus = if dm.flags & 0x40 != 0 {
                    (dt.flags.wrapping_mul(8) & 0x400) as i32
                } else {
                    0
                };
                let mut score = ((i32::from(dm.speed) - i32::from(dt.speed)) << 7) + bonus;
                let (p, q) = (w.monsters[m].position, w.monsters[t].position);
                score += (0x2800 - distance2d(p.0 - q.0, p.1 - q.1)).max(0);
                if best == -1 || score > best {
                    best = score;
                    chosen[a] = Some(t);
                }
            }
        }
        for &t in &members {
            let group: Vec<usize> = attackers
                .iter()
                .zip(&chosen)
                .filter(|(_, c)| **c == Some(t))
                .map(|(&m, _)| m)
                .collect();
            for chunk in group.chunks(per_command) {
                handle_target_command(w, -2, 4, chunk, &[t])?;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeleeState {
    pub last: i16,
    pub formation: i32,
    pub dither: i32,
}

impl MapActions {
    pub(crate) fn melee(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let mut state = self.melee_glo.get(&i).copied().unwrap_or(MeleeState {
            last: 0,
            formation: self.long(i, b"form", TYPE_INTEGER).unwrap_or(0),
            dither: self.long(i, b"dith", TYPE_DISTANCE).unwrap_or(0x600),
        });
        let subjects = self.subjects(i, ctx, 0x190);
        if subjects.is_empty() {
            return Ok(4);
        }
        let w = &mut *ctx.world;
        let objects: Vec<usize> = self
            .combined(i, b"obje", TYPE_MONSTERS, 0x190)
            .into_iter()
            .filter_map(|id| w.monster_index_by_identifier(id as u16))
            .collect();
        state.last = match melee_decision(w, &subjects, &objects, &state) {
            MeleeDecision::None => -1,
            MeleeDecision::Attack => {
                attack_with_monster_indexes(w, &subjects, AttackConfig::new());
                1
            }
            MeleeDecision::Move { point, count } => {
                let header = MovementHeader {
                    flags: 8,
                    formation: state.formation as i16,
                    facing: 0,
                };
                w.movement_command(&subjects[..count], &[point], header, -2)?;
                0
            }
        };
        self.melee_glo.insert(i, state);
        Ok(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeleeDecision {
    None,
    Attack,
    Move { point: (i32, i32), count: usize },
}

pub fn melee_decision(
    w: &EngineWorld,
    subjects: &[usize],
    objects: &[usize],
    state: &MeleeState,
) -> MeleeDecision {
    let team = w.monsters[subjects[0]].team;
    let center = w.centerpoint(subjects);
    let (mut min, mut max) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
    for &m in subjects {
        let p = w.monsters[m].position;
        min = (min.0.min(p.0), min.1.min(p.1));
        max = (max.0.max(p.0), max.1.max(p.1));
    }
    let radius = (max.0 - center.0)
        .max(center.0 - min.0)
        .max(center.1 - min.1)
        .max(max.1 - center.1) as i16;
    let radius = i32::from(radius.max(0x200));
    let gameplay = w.formation_globals.gameplay_version;
    let groups: Vec<&SpatialSupergroup> = supergroups(&w.spatial_groups).collect();
    let mut best: Option<(usize, i32)> = None;
    for (g, group) in groups.iter().enumerate() {
        if group.team == team || w.teams_are_allied(team, group.team) {
            continue;
        }
        let (dx, dy) = (
            center.0.wrapping_sub(group.center.0).wrapping_abs(),
            center.1.wrapping_sub(group.center.1).wrapping_abs(),
        );
        let long = if dx > dy {
            dx + (dy >> 1)
        } else {
            dy + (dx >> 1)
        };
        let d = long - i32::from(group.radius);
        if let Some((b, best_d)) = best
            && d >= best_d
            && (state.last == 1
                || group.strength <= groups[b].strength
                || d - state.dither >= best_d)
        {
            continue;
        }
        if !objects.is_empty() {
            let listed = w
                .spatial_groups
                .iter()
                .filter(|f| f.supergroup == Some(g) && !f.members.is_empty())
                .any(|f| {
                    gameplay <= game_options::GAMEPLAY_1_3
                        || f.members.iter().any(|m| objects.contains(m))
                });
            if !listed {
                continue;
            }
        }
        best = Some((g, d));
    }
    match best {
        None => MeleeDecision::None,
        Some((_, d)) if d.wrapping_sub(radius) < 0 => MeleeDecision::Attack,
        Some((g, _)) => MeleeDecision::Move {
            point: groups[g].center,
            count: subjects
                .len()
                .min(if gameplay > game_options::GAMEPLAY_1_3 {
                    0x7e
                } else {
                    0x1e
                }),
        },
    }
}

#[cfg(test)]
mod tests {
    use mandrake_common::FourCC;
    use mandrake_math::BungieRng;
    use mandrake_tag::MapActionParameter;

    use super::*;
    use crate::engine_rng::{
        EngineMonster, EngineObject, EngineRng, MonsterDefinition, MonsterFormation, SiteCounts,
    };
    use crate::map_actions::MapAction;
    use crate::spatial_groups::{SpatialGroup, rebuild_second_level};

    fn monster(index: usize, identifier: i16, team: i16, position: (i32, i32)) -> EngineMonster {
        EngineMonster {
            index,
            identifier,
            first_draw: 0,
            definition: 0,
            object: EngineObject::new(index as i32, FourCC::new(*b"test")),
            mode: 1,
            anim_state: 0x1a,
            speed: 0,
            position,
            flags: 0,
            flags_8: 0,
            neighbours: 0,
            impassable: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team,
            owner: 0,
            facing: 0,
            formation: MonsterFormation::default(),
            cell_link: 0,
            flying_height: 0,
        }
    }

    fn world(monsters: Vec<EngineMonster>) -> EngineWorld {
        let mut w = EngineWorld::default();
        w.mode = 2;
        w.mesh_cells = (256, 256);
        w.monster_definitions =
            vec![MonsterDefinition::from_bytes(FourCC::new(*b"wam1"), &vec![0u8; 0x224]).unwrap()];
        w.monsters = monsters;
        w
    }

    fn group(
        team: i16,
        definition: usize,
        center: (i32, i32),
        radius: i16,
        strength: i16,
        members: &[usize],
    ) -> SpatialGroup {
        SpatialGroup {
            team,
            definition,
            center,
            radius,
            facing: 0,
            strength,
            range: 0,
            velocity: (0, 0),
            supergroup: None,
            seeded: None,
            members: members.to_vec(),
        }
    }

    fn parameter(name: &[u8; 4], kind: i16, values: &[i32]) -> MapActionParameter {
        MapActionParameter {
            kind,
            count: values.len() as i16,
            name: FourCC::new(*name),
            values: values.to_vec(),
        }
    }

    fn melee_action(parameters: Vec<MapActionParameter>) -> MapActions {
        let mut actions = MapActions::default();
        actions.actions.push(MapAction {
            identifier: 100,
            kind: 3,
            tag: u32::from_be_bytes(*b"mele"),
            flags: 4,
            delay: 0,
            delay_range: 0,
            time: 0,
            parameters,
        });
        actions
    }

    fn run(actions: &mut MapActions, w: &mut EngineWorld) -> Result<u32, Unmodelled> {
        let mut rng = BungieRng::new(1);
        let mut sites = SiteCounts::default();
        let mut ctx = MapActionContext {
            world: w,
            rng: &mut EngineRng {
                rng: &mut rng,
                sites: &mut sites,
            },
            time: 64,
        };
        let r = actions.melee(0, &mut ctx);
        assert!(sites.counts().is_empty(), "no draws");
        r
    }

    #[test]
    fn mele_without_subjects() {
        let mut actions = melee_action(vec![parameter(b"subj", 2, &[7])]);
        let mut w = world(vec![monster(0, 8, 0, (0, 0))]);
        assert_eq!(run(&mut actions, &mut w), Ok(4));
        assert!(actions.melee_glo.is_empty());
    }

    #[test]
    fn mele_attacks_a_near_group_and_stores_what_it_did() {
        let mut actions = melee_action(vec![parameter(b"subj", 2, &[7])]);
        let mut w = world(vec![
            monster(0, 7, 0, (10000, 10000)),
            monster(1, 8, 1, (10300, 10000)),
        ]);
        w.spatial_groups = vec![group(1, 0, (10300, 10000), 0, 64, &[1])];
        rebuild_second_level(&mut w);
        assert_eq!(run(&mut actions, &mut w), Ok(0));
        assert_eq!(actions.melee_glo[&0].last, 1);
        assert_eq!(actions.melee_glo[&0].dither, 0x600);
        assert_eq!(w.attack_queue.requests.len(), 1);
        assert_eq!(w.attack_queue.requests[0].config, AttackConfig::new());
        w.spatial_groups.clear();
        assert_eq!(run(&mut actions, &mut w), Ok(0));
        assert_eq!(actions.melee_glo[&0].last, -1);
        assert_eq!(w.attack_queue.requests.len(), 1);
    }

    fn state(last: i16) -> MeleeState {
        MeleeState {
            last,
            formation: 3,
            dither: 0x600,
        }
    }

    #[test]
    fn mele_chooses_the_group() {
        let mut w = world(vec![
            monster(0, 7, 0, (0, 0)),
            monster(1, 8, 1, (0x2000, 0)),
            monster(2, 9, 1, (0x2400, 0)),
            monster(3, 10, 1, (0x4000, 0)),
        ]);
        w.spatial_groups = vec![
            group(0, 1, (100, 0), 0, 500, &[0]),
            group(1, 1, (0x2000, 0), 0, 64, &[1]),
            group(1, 2, (0x2400, 0), 0, 128, &[2]),
            group(1, 3, (0x4000, 0), 0, 500, &[3]),
        ];
        rebuild_second_level(&mut w);
        let go = |point| MeleeDecision::Move { point, count: 1 };
        assert_eq!(melee_decision(&w, &[0], &[], &state(0)), go((0x2400, 0)));
        assert_eq!(melee_decision(&w, &[0], &[], &state(1)), go((0x2000, 0)));
        assert_eq!(melee_decision(&w, &[0], &[3], &state(0)), go((0x4000, 0)));
        w.formation_globals.gameplay_version = 0x514;
        assert_eq!(melee_decision(&w, &[0], &[3], &state(1)), go((0x2000, 0)));
    }

    #[test]
    fn team_groups_rules() {
        let at = |i, x| monster(i, i as i16 + 1, 0, (x, 0));
        let mut w = world((0..6).map(|i| at(i, i as i32 * 0x200)).collect());
        w.monsters[5].position = (0x10000, 0);
        w.spatial_groups = vec![
            group(0, 1, (0, 0), 0, 64, &[0]),
            group(0, 1, (0x200, 0), 0, 64, &[1]),
            group(0, 2, (0x400, 0), 0, 192, &[2]),
            group(1, 2, (0x600, 0), 0, 64, &[3]),
            group(0, 3, (0x600, 0), 0, 64, &[4]),
            group(0, 2, (0x10000, 0), 0, 64, &[5]),
        ];
        w.monsters[2].position = (0x280, 0);
        w.monsters[4].position = (0x500, 0);
        rebuild_second_level(&mut w);
        let parents: Vec<_> = w.spatial_groups.iter().map(|g| g.supergroup).collect();
        let groups: Vec<_> = supergroups(&w.spatial_groups).collect();
        assert_eq!(
            parents,
            [Some(0), Some(1), Some(0), Some(2), Some(0), Some(3)]
        );
        assert_eq!(groups.len(), 4);
        let g = groups[0];
        assert_eq!(g.strength, 64 + 192 + 64);
        let c = (0x400 * 192 + 0x600 * 64) / (64 + 192 + 64);
        assert_eq!(g.center, (c, 0));
        assert_eq!(i32::from(g.radius), c);
    }

    #[test]
    fn team_groups_strength_zero_and_the_full_queue() {
        let mut w = world(
            (0..18)
                .map(|i| monster(i, i as i16 + 1, 0, (5000, 5000)))
                .collect(),
        );
        w.spatial_groups = (0..18)
            .map(|i| group(0, i, (5000 + i as i32, 5000), 0, 10, &[i]))
            .collect();
        rebuild_second_level(&mut w);
        assert_eq!(supergroups(&w.spatial_groups).count(), 1);
        assert!(w.spatial_groups.iter().all(|g| g.supergroup == Some(0)));
        assert_eq!(supergroups(&w.spatial_groups).next().unwrap().strength, 160);
        for g in &mut w.spatial_groups {
            g.strength = 0;
        }
        rebuild_second_level(&mut w);
        let g = supergroups(&w.spatial_groups).next().unwrap();
        assert_eq!((g.center, g.radius), ((0, 0), 0));
    }
}
