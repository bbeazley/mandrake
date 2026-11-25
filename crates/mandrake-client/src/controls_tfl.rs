use super::{ControlAction, ControlOptions, Controls, MythKey};
use mandrake_view::camera::myth_observer::ObserverAxes;
use std::path::{Path, PathBuf};

pub(crate) fn edge_scroll(
    cursor: Option<(f32, f32)>,
    size: (u32, u32),
    options: &ControlOptions,
    left_down: bool,
) -> ObserverAxes {
    match cursor {
        Some((x, y)) if !left_down => {
            super::edge_scroll_axes(x, y, size.0 as f32, size.1 as f32, options)
        }
        _ => ObserverAxes::default(),
    }
}

pub(crate) fn edge_actions(axes: &ObserverAxes) -> Vec<ControlAction> {
    let mut out = Vec::new();
    if axes.forward > 0.0 {
        out.push(ControlAction::MoveForward);
    } else if axes.forward < 0.0 {
        out.push(ControlAction::MoveBackward);
    }
    if axes.left > 0.0 {
        out.push(ControlAction::MoveLeft);
    } else if axes.left < 0.0 {
        out.push(ControlAction::MoveRight);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TflKey {
    pub(crate) device: u8,
    pub(crate) code: u8,
    pub(crate) action: u8,
    pub(crate) flags: u16,
}

const fn k(code: u8, action: u8, flags: u16) -> TflKey {
    TflKey {
        device: 1,
        code,
        action,
        flags,
    }
}

const fn mouse(code: u8, action: u8) -> TflKey {
    TflKey {
        device: 2,
        code,
        action,
        flags: 0,
    }
}

pub(crate) const SHOWN: u16 = 0x80;
pub(crate) const WITH_VALUE: u16 = 0x20;

pub(crate) const RIGHT_HANDED: [TflKey; 46] = [
    mouse(3, 34),
    mouse(4, 37),
    k(70, 40, 0),
    k(69, 41, 0),
    k(68, 42, 0),
    k(57, 43, 0),
    k(32, 2, SHOWN),
    k(46, 3, SHOWN),
    k(58, 6, SHOWN),
    k(59, 7, SHOWN),
    k(45, 4, SHOWN),
    k(47, 5, SHOWN),
    k(31, 0, SHOWN),
    k(33, 1, SHOWN),
    k(60, 8, SHOWN),
    k(61, 9, SHOWN),
    k(16, 10, SHOWN),
    k(48, 11, SHOWN),
    k(71, 12, SHOWN),
    k(56, 13, SHOWN),
    k(29, 14, SHOWN),
    k(70, 15, SHOWN),
    k(50, 16, SHOWN),
    k(62, 17, SHOWN),
    k(34, 18, SHOWN),
    k(68, 19, SHOWN),
    k(35, 20, SHOWN),
    k(63, 21, SHOWN),
    k(36, 22, SHOWN),
    k(43, 23, SHOWN),
    k(49, 24, SHOWN),
    k(30, 30, SHOWN),
    k(0, 31, SHOWN),
    k(28, 32, 0),
    k(27, 33, 0),
    k(17, 25, WITH_VALUE),
    k(18, 25, WITH_VALUE | 1),
    k(19, 25, WITH_VALUE | 2),
    k(20, 25, WITH_VALUE | 3),
    k(21, 25, WITH_VALUE | 4),
    k(22, 25, WITH_VALUE | 5),
    k(23, 25, WITH_VALUE | 6),
    k(24, 25, WITH_VALUE | 7),
    k(25, 25, WITH_VALUE | 8),
    k(26, 25, WITH_VALUE | 9),
    k(86, 48, SHOWN),
];

pub(crate) fn left_handed() -> [TflKey; 46] {
    let mut set = RIGHT_HANDED;
    for (n, code) in [94, 91, 87, 89, 90, 92, 93, 95, 97, 96]
        .into_iter()
        .enumerate()
    {
        set[6 + n].code = code;
    }
    set[33].flags = SHOWN;
    set[34].flags = SHOWN;
    set
}

fn action(id: u8) -> Option<ControlAction> {
    use ControlAction as A;
    Some(match id {
        0 => A::OrbitLeft,
        1 => A::OrbitRight,
        2 => A::MoveForward,
        3 => A::MoveBackward,
        4 => A::TurnLeft,
        5 => A::TurnRight,
        6 => A::MoveLeft,
        7 => A::MoveRight,
        8 => A::ZoomIn,
        9 => A::ZoomOut,
        10 => A::ClearSelection,
        11 => A::RotatePreset,
        12 => A::StopUnits,
        13 => A::SelectAll,
        14 => A::ClearPreset,
        15 => A::RecallSetPreset,
        16 => A::CenterSelection,
        17 => A::ScatterUnits,
        18 => A::RetreatUnits,
        19 => A::AttackGround,
        20 => A::SpecialAbility,
        21 => A::RenameUnits,
        22 => A::ChatMessage,
        23 => A::DetachUnits,
        24 => A::GuardUnits,
        30 => A::OverheadMap,
        31 => A::PauseGame,
        48 => A::OrbitZoomModifier,
        _ => return None,
    })
}

pub(crate) fn shown_actions() -> Vec<ControlAction> {
    let mut out: Vec<ControlAction> = Vec::new();
    for key in RIGHT_HANDED
        .iter()
        .filter(|k| k.device == 1 && k.flags & SHOWN != 0)
    {
        if let Some(a) = action(key.action).filter(|a| !out.contains(a)) {
            out.push(a);
        }
    }
    out
}

const NOT_IN_TFL: [ControlAction; 5] = [
    ControlAction::Taunt,
    ControlAction::RotateUnitsRight,
    ControlAction::RotateUnitsLeft,
    ControlAction::Inventory,
    ControlAction::FormBreakAlliance,
];

pub(crate) fn bindings_toml(set: &[TflKey]) -> String {
    let mut out = String::from("[bindings]\n");
    let mut seen: Vec<ControlAction> = Vec::new();
    for key in set.iter().filter(|k| k.device == 1) {
        let actions: Vec<ControlAction> = if key.action == 25 {
            vec![ControlAction::PRESET_SELECTIONS[(key.flags & 0xf) as usize % 10]]
        } else {
            action(key.action).into_iter().collect()
        };
        for a in actions {
            if seen.contains(&a) {
                continue;
            }
            seen.push(a);
            let keys: Vec<String> = set
                .iter()
                .filter(|o| {
                    o.device == 1
                        && if key.action == 25 {
                            o.action == 25 && o.flags & 0xf == key.flags & 0xf
                        } else {
                            o.action == key.action
                        }
                })
                .map(|o| format!("{:?}", MythKey(o.code).name()))
                .collect();
            out += &format!("{} = [{}]\n", a.config_name(), keys.join(", "));
        }
    }
    for a in NOT_IN_TFL {
        out += &format!("{} = []\n", a.config_name());
    }
    out += FIXED_KEYS;
    out
}

pub(crate) const PRESET_OPTIONS: &str = "[options]\nright_click = \"attack_ground\"\n";

const FIXED_KEYS: &str = "toggle_fps_display = [\"Forward Delete\"]\n";

pub(crate) fn tfl_defaults() -> Controls {
    Controls::preset(super::TFL_PRESETS[0]).unwrap_or_else(Controls::defaults)
}

pub(crate) fn load(conf: Option<&Path>) -> (Controls, Option<PathBuf>) {
    let (controls, path, diagnostics) = crate::conf::load_table(
        crate::conf::to_read(conf, false),
        &["tfl", "controls"],
        tfl_defaults,
        Controls::tfl_with_override,
    );
    for d in &diagnostics {
        eprintln!("controls: {d}");
    }
    (controls, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::Controls;
    use crate::controls::{HeldKeys, Modifiers};

    #[test]
    fn the_right_handed_set() {
        let c = tfl_defaults();
        let press = |name: &str| {
            c.actions_for_press_with(MythKey::from_name(name).unwrap(), Modifiers::default())
        };
        assert!(press("Space").contains(&ControlAction::StopUnits));
        assert!(press("Return").contains(&ControlAction::SelectAll));
        assert!(press("Backquote").contains(&ControlAction::ClearSelection));
        assert!(press("G").contains(&ControlAction::GuardUnits));
        assert!(press("3").contains(&ControlAction::PresetSelection3));
        assert!(press("U").is_empty(), "no taunt");
        assert!(press("I").is_empty(), "no inventory");
        assert!(
            press("F1").contains(&ControlAction::GameSpeedDown),
            "the client's own"
        );
        let mut keys = HeldKeys::default();
        keys.press(MythKey::from_name("W").unwrap());
        assert!(c.is_held(ControlAction::MoveForward, &keys));
        assert_eq!(c.bindings(ControlAction::ZoomIn)[0].key.name(), "C");
        assert_eq!(
            c.bindings(ControlAction::OrbitZoomModifier)[0].key.name(),
            "Keypad 0"
        );
        assert!(c.bindings(ControlAction::RotateUnitsLeft).is_empty());
    }

    #[test]
    fn the_fps_display_is_forward_delete() {
        for c in [
            tfl_defaults(),
            Controls::preset(super::super::TFL_PRESETS[1]).unwrap(),
        ] {
            let toggles = |keys: &[&str], key: &str| {
                let mut held = HeldKeys::default();
                for k in keys {
                    held.press(MythKey::from_name(k).unwrap());
                }
                c.actions_for_press(MythKey::from_name(key).unwrap(), &held)
                    .contains(&ControlAction::ToggleFpsDisplay)
            };
            assert!(toggles(&["Forward Delete"], "Forward Delete"));
            assert!(toggles(&["Shift", "Forward Delete"], "Forward Delete"));
            assert!(toggles(&["Control", "Forward Delete"], "Forward Delete"));
            assert!(!toggles(&["Shift", "Delete"], "Delete"));
        }
    }

    #[test]
    fn a_controls_file_overrides() {
        let (c, d) = Controls::tfl_with_override("[bindings]\nstop_units = \"K\"\n", "test");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(c.bindings(ControlAction::StopUnits)[0].key.name(), "K");
        assert_eq!(c.bindings(ControlAction::GuardUnits)[0].key.name(), "G");
        assert!(c.bindings(ControlAction::Taunt).is_empty());
        let (c, d) = Controls::tfl_with_override("preset = \"left_handed\"\n", "test");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            c.bindings(ControlAction::MoveForward)[0].key.name(),
            "Keypad 8"
        );
        assert_eq!(c.bindings(ControlAction::StopUnits)[0].key.name(), "Space");
    }

    #[test]
    fn tfl_edge_scroll_is_myth2s() {
        let o = ControlOptions::default();
        let size = (640, 480);
        let at = |o: &ControlOptions, x: f32, y: f32| {
            let a = edge_scroll(Some((x, y)), size, o, false);
            (a.forward, a.left)
        };
        assert_eq!(at(&o, 320.0, 0.0), (1.0, 0.0));
        assert_eq!(at(&o, 320.0, 2.5), (0.5, 0.0));
        assert_eq!(at(&o, 320.0, 5.0), (0.0, 0.0));
        assert_eq!(at(&o, 320.0, 480.0), (-1.0, 0.0));
        assert_eq!(at(&o, 0.0, 240.0), (0.0, 1.0));
        assert_eq!(at(&o, 640.0, 240.0), (0.0, -1.0));
        assert_eq!(
            edge_actions(&edge_scroll(Some((640.0, 480.0)), size, &o, false)),
            [ControlAction::MoveBackward, ControlAction::MoveRight]
        );
        let rev = ControlOptions {
            reverse_axis: true,
            ..o
        };
        assert_eq!(at(&rev, 0.0, 240.0), (0.0, -1.0), "Reverse Axis");
        assert_eq!(
            edge_scroll(Some((0.0, 0.0)), size, &o, true),
            ObserverAxes::default()
        );
        assert_eq!(edge_scroll(None, size, &o, false), ObserverAxes::default());
        let off = ControlOptions {
            enable_mouse_movement: false,
            ..o
        };
        assert_eq!(at(&off, 0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn tfl_key_sets_right_click_attack_ground() {
        use crate::controls::RightClick;
        for name in crate::controls::TFL_PRESETS {
            let c = Controls::preset(name).unwrap();
            assert_eq!(c.options.right_click, RightClick::AttackGround, "{name}");
        }
        assert_eq!(
            Controls::defaults().options.right_click,
            RightClick::GestureMove,
            "Myth II's default"
        );
        let (c, d) =
            Controls::tfl_with_override("[options]\nright_click = \"gesture_move\"\n", "test");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(c.options.right_click, RightClick::GestureMove);
    }
}
