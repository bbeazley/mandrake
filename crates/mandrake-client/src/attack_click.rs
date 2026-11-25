use mandrake_sim::Simulation;
use mandrake_sim::attack_ai::AttackClick;
use mandrake_sim::engine_rng::EngineWorld;

use crate::command_bar::SELECTION_MODE_SPECIAL;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub option: bool,
    pub control: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub owned_by_local_player: bool,
    pub forced: bool,
    pub silent: bool,
}

impl Target {
    pub fn of(world: &EngineWorld, id: u32) -> Option<Self> {
        let i = world.monster_index_by_identifier(id as u16)?;
        let m = &world.monsters[i];
        let d = &world.monster_definitions[m.definition];
        Some(Self {
            owned_by_local_player: m.owner == world.local_player,
            forced: m.flags_8 & 0x1000 != 0 || d.field_1b6 == 5,
            silent: d.flags & 0x80000 != 0,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input {
    pub modifiers: Modifiers,
    pub selection_mode: i16,
    pub tfl_mouse_actions: bool,
    pub game_options_high: u16,
}

pub fn attack_click(input: Input, target: Target) -> AttackClick {
    let m = input.modifiers;
    let state_3 = input.selection_mode == SELECTION_MODE_SPECIAL;
    let r8 = m.option
        && m.shift
        && (target.forced || !target.owned_by_local_player)
        && input.game_options_high
            & mandrake_common::game_options::high(mandrake_common::game_options::TFL)
            != 0
        && !state_3;
    AttackClick {
        flag_2b: m.shift,
        flag_2a: m.control,
        state_3,
        tfl_mouse_actions: input.tfl_mouse_actions,
        r8,
    }
}

pub fn order_attack(
    sim: &mut Simulation,
    attackers: &[u32],
    target: u32,
    input: Input,
) -> (Vec<u32>, bool) {
    if sim.is_replay() {
        return (Vec::new(), false);
    }
    let facts = (Target::of(&sim.engine, target)).unwrap_or_default();
    sim.attack_click = attack_click(input, facts);
    let targets = sent_targets(sim, attackers, target).unwrap_or_else(|| vec![target]);
    sim.order_attack(attackers, mandrake_sim::AttackTarget::Units(vec![target]));
    (targets, !facts.silent)
}

fn sent_targets(sim: &Simulation, attackers: &[u32], target: u32) -> Option<Vec<u32>> {
    let w = &sim.engine;
    let selection: Vec<usize> = attackers
        .iter()
        .filter_map(|&id| w.monster_index_by_identifier(id as u16))
        .collect();
    if selection.is_empty() {
        return None;
    }
    let t = w.monster_index_by_identifier(target as u16)?;
    let (_, targets) = w.local_attack_click(&selection, t, sim.attack_click).ok()?;
    Some(
        targets
            .iter()
            .map(|&j| u32::from(w.monsters[j].identifier as u16))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_bar::{SELECTION_MODE_FORMATION, SELECTION_MODE_NORMAL};

    fn input(option: bool, control: bool, shift: bool) -> Input {
        Input {
            modifiers: Modifiers {
                option,
                control,
                shift,
            },
            selection_mode: SELECTION_MODE_NORMAL,
            tfl_mouse_actions: false,
            game_options_high: 0x20,
        }
    }

    const ENEMY: Target = Target {
        owned_by_local_player: false,
        forced: false,
        silent: false,
    };

    #[test]
    fn each_modifier_combination_gives_the_engine_flags() {
        for bits in 0..8u8 {
            let (option, control, shift) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
            let c = attack_click(input(option, control, shift), ENEMY);
            assert_eq!(
                c,
                AttackClick {
                    flag_2b: shift,
                    flag_2a: control,
                    state_3: false,
                    tfl_mouse_actions: false,
                    r8: option && shift,
                },
                "option {option} control {control} shift {shift}"
            );
        }
    }

    #[test]
    fn r8_needs_the_option_bit_the_mode_and_the_owner_test() {
        let os = input(true, false, true);
        assert!(attack_click(os, ENEMY).r8);
        assert!(
            !attack_click(
                Input {
                    game_options_high: 0,
                    ..os
                },
                ENEMY
            )
            .r8
        );
        assert!(
            !attack_click(
                Input {
                    game_options_high: !0x20,
                    ..os
                },
                ENEMY
            )
            .r8
        );
        let special = Input {
            selection_mode: SELECTION_MODE_SPECIAL,
            ..os
        };
        let c = attack_click(special, ENEMY);
        assert!(c.state_3 && !c.r8);
        let formation = Input {
            selection_mode: SELECTION_MODE_FORMATION,
            ..os
        };
        let c = attack_click(formation, ENEMY);
        assert!(!c.state_3 && c.r8);
        let own = Target {
            owned_by_local_player: true,
            ..ENEMY
        };
        assert!(!attack_click(os, own).r8);
        let forced = Target {
            forced: true,
            ..own
        };
        assert!(attack_click(os, forced).r8);
    }

    #[test]
    fn tfl_and_the_special_mode_pass_through() {
        let c = attack_click(
            Input {
                tfl_mouse_actions: true,
                selection_mode: SELECTION_MODE_SPECIAL,
                ..input(false, false, false)
            },
            ENEMY,
        );
        assert_eq!(
            c,
            AttackClick {
                flag_2b: false,
                flag_2a: false,
                state_3: true,
                tfl_mouse_actions: true,
                r8: false,
            }
        );
        assert_eq!(
            attack_click(input(false, false, false), ENEMY),
            AttackClick::default()
        );
    }
}
