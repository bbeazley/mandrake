use mandrake_view::selection::state::{
    self, Alliances, CapsLock, GameContext, MonsterInterface, OBJECT_HOVERED, OBJECT_SELECTED,
    OBJECT_TARGETED, TargetHighlight,
};

use crate::controls;

pub const LOCAL_PLAYER: i8 = 0;

pub const GAMEPLAY_VERSION: i32 = mandrake_sim::gameplay::GAMEPLAY_1_8_5;

pub const PALETTE_FLAG_ALLY_COLORED: i16 = 0x1;
pub const PALETTE_FLAG_DEFINITION_COLORED: i16 = 0x10;
pub const PALETTE_FLAG_HUNTED: i16 = 0x20;
pub const MARKER_FLAG_OUTER_FRAME: u32 = 0x10;

pub const MONS_CLASS_OFFSET: usize = 0x1b6;

pub fn placed_monster_flags(palette_flags: i16, marker_flags: u32, game_type: u16) -> (u32, u32) {
    let (mut flags, mut flags8) = (0u32, 0u32);
    if palette_flags & PALETTE_FLAG_ALLY_COLORED != 0 {
        flags |= state::MONSTER_FLAG_ALLY_COLORED;
    }
    if palette_flags & PALETTE_FLAG_DEFINITION_COLORED != 0 {
        flags8 |= state::MONSTER_FLAGS8_DEFINITION_COLORED;
    }
    if palette_flags & PALETTE_FLAG_HUNTED != 0 {
        flags8 |= state::MONSTER_FLAGS8_HUNTED;
        if game_type == state::GAME_TYPE_HUNTED && GAMEPLAY_VERSION > 0x6b8 {
            flags |= state::MONSTER_FLAG_ALLY_COLORED;
        }
    }
    if marker_flags & MARKER_FLAG_OUTER_FRAME != 0 {
        flags8 |= state::MONSTER_FLAGS8_OUTER_FRAME;
    }
    (flags, flags8)
}

pub fn caps_lock_mode(option: controls::CapsLock) -> CapsLock {
    match option {
        controls::CapsLock::SelectionBoxes => CapsLock::SelectionBoxes,
        controls::CapsLock::Ignored => CapsLock::Ignored,
        controls::CapsLock::HealthBars => CapsLock::HealthBars,
        controls::CapsLock::PlayerStats => CapsLock::PlayerStats,
        controls::CapsLock::ChatBacklog => CapsLock::ChatBacklog,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapsLockKey {
    down: bool,
}

impl CapsLockKey {
    pub fn press(&mut self) {
        self.down = !self.down;
    }

    pub fn is_down(self) -> bool {
        self.down
    }
}

#[derive(Clone, Debug, Default)]
pub struct InterfaceView {
    pub caps_lock: Option<CapsLock>,
    pub hovered: Option<u32>,
    pub targets: TargetHighlight,
    pub tfl_selection_boxes: bool,
    pub order_hide: OrderFlagHide,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderFlagHide {
    pub units: Vec<u32>,
    pub countdown: i16,
}

pub const ORDER_FLAG_HIDE_TICKS: i16 = 7;

impl OrderFlagHide {
    pub fn order(&mut self, units: &[u32]) {
        self.units = units.to_vec();
        self.countdown = ORDER_FLAG_HIDE_TICKS;
    }

    pub fn update(&mut self, elapsed: i16) {
        if self.countdown == 0 {
            return;
        }
        self.countdown = self.countdown.wrapping_sub(elapsed);
        if self.countdown == 0 {
            self.units.clear();
        }
    }

    pub fn hides(&self, id: u32) -> bool {
        self.units.contains(&id)
    }
}

impl InterfaceView {
    pub fn object_flags(&self, id: u32, selected: bool) -> u32 {
        let mut flags = 0;
        if selected && !self.order_hide.hides(id) {
            flags |= OBJECT_SELECTED;
        }
        if self.hovered == Some(id) {
            flags |= OBJECT_HOVERED;
        }
        if self.targets.is_targeted(id) {
            flags |= OBJECT_TARGETED;
        }
        flags
    }
}

pub fn solo_game(local_team: i16) -> GameContext {
    GameContext::single_player(local_team, LOCAL_PLAYER)
}

pub fn game_alliances(w: &mandrake_sim::engine_rng::EngineWorld) -> Alliances {
    let view = w.alliance_view();

    let mut a = Alliances::new(view.teams.min(16) as i16);
    for (i, row) in a.table.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = view.teams_are_allied(i as i16, j as i16);
        }
    }
    a
}

pub fn monster_interface(
    team: u8,
    local_team: i16,
    placed: (u32, u32),
    definition: (u32, i16),
) -> MonsterInterface {
    let team = team as i8;
    MonsterInterface {
        team,
        player: if i16::from(team) == local_team {
            LOCAL_PLAYER
        } else {
            -1
        },
        flags: placed.0,
        flags8: placed.1,
        definition_flags: definition.0,
        definition_class: definition.1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placed_monster_flags() {
        assert_eq!(
            placed_monster_flags(0x4, 0, state::GAME_TYPE_SINGLE_PLAYER),
            (0, 0)
        );
        assert_eq!(
            placed_monster_flags(0x1, 0, state::GAME_TYPE_SINGLE_PLAYER),
            (state::MONSTER_FLAG_ALLY_COLORED, 0)
        );
        assert_eq!(
            placed_monster_flags(0x10, 0x10, state::GAME_TYPE_SINGLE_PLAYER),
            (
                0,
                state::MONSTER_FLAGS8_DEFINITION_COLORED | state::MONSTER_FLAGS8_OUTER_FRAME
            )
        );
        assert_eq!(
            placed_monster_flags(0x20, 0, state::GAME_TYPE_SINGLE_PLAYER),
            (0, state::MONSTER_FLAGS8_HUNTED)
        );
        assert_eq!(
            placed_monster_flags(0x20, 0, state::GAME_TYPE_HUNTED),
            (
                state::MONSTER_FLAG_ALLY_COLORED,
                state::MONSTER_FLAGS8_HUNTED
            )
        );
    }

    #[test]
    fn test_caps_lock_key_toggles() {
        let mut k = CapsLockKey::default();
        assert!(!k.is_down());
        k.press();
        assert!(k.is_down());
        k.press();
        assert!(!k.is_down());
        assert_eq!(caps_lock_mode(controls::CapsLock::Ignored) as u8, 1);
        assert_eq!(caps_lock_mode(controls::CapsLock::SelectionBoxes) as u8, 0);
        assert_eq!(caps_lock_mode(controls::CapsLock::HealthBars) as u8, 2);
    }
}

#[cfg(test)]
mod order_hide_tests {
    use super::*;

    #[test]
    fn a_move_order_hides_the_selected_flag_for_seven_polls() {
        let mut h = OrderFlagHide::default();
        h.order(&[10006, 10007]);
        assert!(h.hides(10006));
        for _ in 0..6 {
            h.update(1);
            assert!(h.hides(10007));
        }
        h.update(1);
        assert!(!h.hides(10006) && h.countdown == 0);
    }
}
