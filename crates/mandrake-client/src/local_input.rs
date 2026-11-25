use mandrake_sim::engine_rng::{EngineWorld, MoveClick, Unmodelled};
use mandrake_sim::map_actions::InputAction as EngineAction;
use mandrake_sim::{LocalMovement, Simulation};

use crate::controls::MythKey;
use crate::selection_input::{ActionFlags, InputAction};
use crate::selection_rules;

pub fn engine_action(flags: ActionFlags, actions: &[u16]) -> EngineAction {
    let mut words = [0u32; 4];
    let bits = flags.bits();
    words[0] = bits as u32;
    words[1] = (bits >> 32) as u32;
    for &a in actions {
        if let Some(w) = words.get_mut(usize::from(a >> 5)) {
            *w |= 1 << (a & 31);
        }
    }
    EngineAction {
        flags: words,
        key: -1,
    }
}

pub fn sniff_polled_action(
    sim: &mut Simulation,
    action: InputAction,
    actions: &[u16],
) -> Option<InputAction> {
    sim.local_input_action(&engine_action(action.flags, actions))
        .then_some(action)
}

pub fn ground_move_click(
    sim: &mut Simulation,
    selection: &[u32],
    point: (i32, i32),
    facing: Option<u16>,
    add_waypoint: bool,
) -> Result<Option<LocalMovement>, Unmodelled> {
    sim.local_move_click(
        selection,
        MoveClick::Ground(point.0, point.1),
        facing,
        add_waypoint,
    )
}

pub const PRESET_COUNT: usize = 16;
pub use crate::selection_input::SELECTION_LIMIT;
pub const DOUBLE_TAP_TICKS: i32 = 0x14;
pub const PRESET_HOLD_TICKS: i32 = 0xf;
pub const PRESET_MODE_GRACE: i32 = 7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DoubleTap {
    key: Option<MythKey>,
    words: [u32; 2],
    timer: i32,
}

impl DoubleTap {
    pub fn key_event(&mut self, key: MythKey, words: [u32; 2]) -> bool {
        if self.key == Some(key) && self.timer > 0 {
            let double = self.words == words;
            self.key = None;
            self.timer = 0;
            return double;
        }
        self.key = Some(key);
        self.words = words;
        self.timer = -1;
        false
    }

    pub fn poll(&mut self, elapsed: i32) {
        if self.timer == -1 {
            self.timer = (DOUBLE_TAP_TICKS - elapsed).max(0);
        } else if self.timer >= 1 {
            self.timer = (self.timer - elapsed).max(0);
        }
        if self.timer == 0 {
            self.key = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetEffect {
    Formation { formation: i16, in_place: bool },
    Recall { preset: usize, toggle: bool },
    Create { preset: usize },
    SelectNothing,
    Center { in_view_test: bool },
    ClearSprites,
    EnterMode(i16),
    ExitMode,
    AllyPick { number: usize, shift: bool },
}

#[derive(Clone, Copy, Debug)]
pub struct PresetPoll {
    pub elapsed: i32,
    pub recall_held: bool,
    pub number: Option<usize>,
    pub mode: i16,
    pub alternate: bool,
    pub mouse_busy: bool,
    pub selection_empty: bool,
    pub look: bool,
    pub replay: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetCommand {
    pub index: usize,
    pub identifiers: Vec<u32>,
}

#[derive(Clone, Copy)]
pub enum PresetWalk<'a> {
    Myth2(&'a EngineWorld),
    Members(&'a dyn Fn(u32) -> bool),
}

impl PresetWalk<'_> {
    fn count(self, p: &[u32]) -> usize {
        match self {
            Self::Myth2(w) => p
                .iter()
                .filter(|&&id| selection_rules::preset_member_commanded(w, id))
                .count(),
            Self::Members(member) => p.iter().filter(|&&id| member(id)).count(),
        }
    }
}

#[derive(Clone, Copy)]
enum PresetFit {
    Exact,
    AtLeast,
}

const TFL_ATTACK_LATCH: i32 = 7;

#[derive(Clone, Debug, Default)]
pub struct PresetKeys {
    presets: [Vec<u32>; PRESET_COUNT],
    current: usize,
    pending: Option<usize>,
    held: i32,
    countdown: i32,
    recall_was_held: bool,
    pub double_tap: DoubleTap,
    doubled: [bool; 10],
    pub tfl_countdown: bool,
    took_poll: bool,
    wheel: i32,
}

impl PresetKeys {
    pub fn key_event(
        &mut self,
        key: MythKey,
        words: [u32; 2],
        numbers: impl IntoIterator<Item = usize>,
    ) {
        let double = self.double_tap.key_event(key, words);
        for n in numbers {
            self.doubled[n] = double;
        }
    }

    pub fn poll(&mut self, p: PresetPoll) -> Vec<PresetEffect> {
        use crate::command_bar::{SELECTION_MODE_NORMAL, SELECTION_MODE_PRESET};
        let mut out = Vec::new();
        self.took_poll = false;
        self.double_tap.poll(p.elapsed);
        let pressed = p.recall_held && !self.recall_was_held;
        self.recall_was_held = p.recall_held;
        if p.mouse_busy {
            if p.mode == SELECTION_MODE_PRESET {
                out.push(PresetEffect::ClearSprites);
                out.push(PresetEffect::ExitMode);
            }
            return out;
        }
        let mut mode = p.mode;
        if mode == SELECTION_MODE_PRESET {
            if p.recall_held {
                self.countdown = if p.alternate { 0 } else { PRESET_MODE_GRACE };
            } else {
                let was = self.countdown;
                self.countdown -= p.elapsed;
                let over = if self.tfl_countdown {
                    was < 0
                } else {
                    self.countdown < 0
                };
                if over {
                    out.push(PresetEffect::ClearSprites);
                    out.push(PresetEffect::ExitMode);
                    mode = SELECTION_MODE_NORMAL;
                }
            }
        }
        if mode == SELECTION_MODE_PRESET && !p.alternate {
            self.took_poll = true;
            if let Some(n) = p.number {
                self.pending = Some(n);
            }
            match p.number.filter(|_| !p.selection_empty) {
                None => {
                    if let Some(preset) = self.pending {
                        out.push(PresetEffect::Recall {
                            preset,
                            toggle: false,
                        });
                    }
                }
                Some(preset) => {
                    self.held += p.elapsed;
                    if self.held > PRESET_HOLD_TICKS {
                        if !p.look {
                            out.push(PresetEffect::Create { preset });
                        }
                        out.push(PresetEffect::ClearSprites);
                        out.push(PresetEffect::ExitMode);
                    }
                }
            }
            return out;
        }
        if pressed {
            out.push(PresetEffect::ClearSprites);
            if !p.replay {
                self.pending = None;
                self.held = 0;
                out.push(PresetEffect::EnterMode(SELECTION_MODE_PRESET));
            }
        }
        out
    }

    pub fn attack_ground_over(&mut self, held: bool, elapsed: i16) -> bool {
        if self.tfl_countdown {
            if held {
                self.countdown = TFL_ATTACK_LATCH;
                return false;
            }
            let was = self.countdown;
            self.countdown -= i32::from(elapsed);
            return was < 0;
        }
        if held {
            self.countdown = 0;
        } else {
            self.countdown -= i32::from(elapsed);
        }
        self.countdown < 0
    }

    pub fn took_poll(&self) -> bool {
        self.took_poll
    }

    fn holding(
        &mut self,
        walk: PresetWalk<'_>,
        fit: PresetFit,
        selection: &[u32],
        look: bool,
    ) -> Option<usize> {
        if selection.is_empty() || look {
            return None;
        }
        let found = (1..=PRESET_COUNT)
            .map(|k| (self.current + k) % PRESET_COUNT)
            .find(|&k| {
                let p = &self.presets[k];
                let fits = match fit {
                    PresetFit::Exact => walk.count(p) == selection.len(),
                    PresetFit::AtLeast => walk.count(p) >= selection.len(),
                };
                fits && selection.iter().all(|id| p.contains(id))
            })?;
        self.current = found;
        Some(found)
    }

    pub fn next_preset(
        &mut self,
        walk: PresetWalk<'_>,
        backward: bool,
        selection: &[u32],
        look: bool,
    ) -> Option<usize> {
        let found = self
            .holding(walk, PresetFit::Exact, selection, look)
            .or_else(|| self.holding(walk, PresetFit::AtLeast, selection, look));
        let n = PRESET_COUNT;
        let has_monsters = |i: usize| walk.count(&self.presets[i]) != 0;
        match found {
            None if backward => (0..n).rev().find(|&i| has_monsters(i)),
            None => (0..n).find(|&i| has_monsters(i)),
            Some(c) => (1..=n)
                .map(|k| {
                    if backward {
                        (c + n - k % n) % n
                    } else {
                        (c + k) % n
                    }
                })
                .find(|&i| i == c || has_monsters(i)),
        }
    }

    pub fn clear_exact(
        &mut self,
        walk: PresetWalk<'_>,
        selection: &[u32],
        look: bool,
    ) -> Option<usize> {
        let k = self.holding(walk, PresetFit::Exact, selection, look)?;
        self.presets[k].clear();
        Some(k)
    }

    pub fn recall_with<T: PartialEq>(
        &self,
        preset: usize,
        toggle: bool,
        cap: usize,
        selection: &mut Vec<T>,
        member: impl Fn(u32) -> Option<(T, bool)>,
    ) {
        if !toggle {
            selection.clear();
        }
        for &id in &self.presets[preset] {
            let Some((m, allowed)) = member(id) else {
                continue;
            };
            if let Some(at) = selection.iter().position(|s| *s == m) {
                if toggle {
                    selection.remove(at);
                }
            } else if allowed && selection.len() < cap {
                selection.push(m);
            }
        }
    }

    pub fn number_press(&mut self, number: usize, shift: bool, p: PresetPoll) -> Vec<PresetEffect> {
        use crate::command_bar::{
            SELECTION_MODE_FORMATION_KEY, SELECTION_MODE_FORMATION_KEY_IN_PLACE,
            SELECTION_MODE_PRESET,
        };
        let double = std::mem::take(&mut self.doubled[number]);
        let formation = number as i16;
        match p.mode {
            SELECTION_MODE_PRESET if p.alternate => {
                let mut out = Vec::new();
                if !p.look {
                    out.push(PresetEffect::Create { preset: number });
                }
                out.push(PresetEffect::ClearSprites);
                out.push(PresetEffect::ExitMode);
                out
            }
            SELECTION_MODE_PRESET => {
                self.pending = Some(number);
                if p.selection_empty {
                    vec![PresetEffect::Recall {
                        preset: number,
                        toggle: false,
                    }]
                } else {
                    Vec::new()
                }
            }
            crate::command_bar::SELECTION_MODE_ALLY => {
                vec![PresetEffect::AllyPick { number, shift }]
            }
            7 | 9..=11 => Vec::new(),
            SELECTION_MODE_FORMATION_KEY | SELECTION_MODE_FORMATION_KEY_IN_PLACE => {
                vec![PresetEffect::Formation {
                    formation,
                    in_place: p.mode == SELECTION_MODE_FORMATION_KEY_IN_PLACE,
                }]
            }
            _ => {
                let double = double && !p.recall_held;
                if p.alternate {
                    let mut out = vec![PresetEffect::Recall {
                        preset: number,
                        toggle: shift,
                    }];
                    if double {
                        out.push(PresetEffect::Center {
                            in_view_test: false,
                        });
                    }
                    out
                } else {
                    vec![PresetEffect::Formation {
                        formation,
                        in_place: double,
                    }]
                }
            }
        }
    }

    pub fn rotate_press(
        &mut self,
        w: &EngineWorld,
        shift: bool,
        selection: &[u32],
        p: PresetPoll,
    ) -> Vec<PresetEffect> {
        self.rotate_press_by(PresetWalk::Myth2(w), shift, selection, p)
    }

    pub fn rotate_press_by(
        &mut self,
        walk: PresetWalk<'_>,
        shift: bool,
        selection: &[u32],
        p: PresetPoll,
    ) -> Vec<PresetEffect> {
        use crate::command_bar::{
            SELECTION_MODE_FORMATION_KEY, SELECTION_MODE_FORMATION_KEY_IN_PLACE,
        };
        if p.alternate {
            let mut out = vec![PresetEffect::ClearSprites];
            if !selection.is_empty() && !p.look && !p.replay {
                out.push(PresetEffect::EnterMode(if shift {
                    SELECTION_MODE_FORMATION_KEY_IN_PLACE
                } else {
                    SELECTION_MODE_FORMATION_KEY
                }));
            }
            return out;
        }
        let Some(preset) = self.next_preset(walk, shift, selection, p.look) else {
            return Vec::new();
        };
        let mut out = vec![PresetEffect::Recall {
            preset,
            toggle: false,
        }];
        if !shift {
            out.push(PresetEffect::Center { in_view_test: true });
        }
        out
    }

    pub fn wheel_notch(&mut self, up: bool) {
        self.wheel += if up { -1 } else { 1 };
    }

    pub fn wheel_poll(
        &mut self,
        w: &EngineWorld,
        selection: &[u32],
        look: bool,
    ) -> Vec<PresetEffect> {
        self.wheel_poll_by(PresetWalk::Myth2(w), selection, look)
    }

    pub fn wheel_poll_by(
        &mut self,
        walk: PresetWalk<'_>,
        selection: &[u32],
        look: bool,
    ) -> Vec<PresetEffect> {
        let wheel = std::mem::take(&mut self.wheel);
        if wheel == 0 {
            return Vec::new();
        }
        let Some(preset) = self.next_preset(walk, wheel > 0, selection, look) else {
            return Vec::new();
        };
        vec![PresetEffect::Recall {
            preset,
            toggle: false,
        }]
    }

    pub fn drop_wheel(&mut self) {
        self.wheel = 0;
    }

    pub fn clear_press(
        &mut self,
        w: &EngineWorld,
        selection: &[u32],
        look: bool,
    ) -> Vec<PresetEffect> {
        self.clear_press_by(PresetWalk::Myth2(w), selection, look)
    }

    pub fn clear_press_by(
        &mut self,
        walk: PresetWalk<'_>,
        selection: &[u32],
        look: bool,
    ) -> Vec<PresetEffect> {
        match self.clear_exact(walk, selection, look) {
            Some(_) => vec![PresetEffect::SelectNothing],
            None => Vec::new(),
        }
    }

    pub fn recall(&self, w: &EngineWorld, preset: usize, toggle: bool, selection: &mut Vec<u32>) {
        self.recall_with(preset, toggle, SELECTION_LIMIT, selection, |id| {
            if !selection_rules::preset_member_commanded(w, id) {
                return None;
            }
            let i = w.monster_index_by_identifier(id as u16)?;
            Some((id, selection_rules::selection_may_take(w, i)))
        });
    }

    pub fn create(&mut self, preset: usize, selection: &[u32]) -> PresetCommand {
        self.presets[preset] = selection[..selection.len().min(SELECTION_LIMIT)].to_vec();
        PresetCommand {
            index: preset,
            identifiers: self.presets[preset].clone(),
        }
    }
}

pub fn selection_focus(
    w: &EngineWorld,
    selection: &[u32],
    zoom_factor: i32,
    in_view: Option<&dyn Fn((i32, i32, i32)) -> bool>,
) -> Option<(i32, i32, i32)> {
    let units = selection.iter().filter_map(|&id| {
        let i = w.monster_index_by_identifier(id as u16)?;
        let m = &w.monsters[i];
        let classless =
            (6..=9).contains(&m.mode) && w.monster_definitions[m.definition].field_1b6 == 0;
        Some(FocusUnit {
            position: (m.position.0, m.position.1, m.object.z),
            facing: m.facing,
            ahead: m.owner == w.local_player && m.mode != 5 && !classless,
        })
    });
    focus_of(units, zoom_factor, in_view)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusUnit {
    pub position: (i32, i32, i32),
    pub facing: u16,
    pub ahead: bool,
}

pub fn focus_of(
    units: impl IntoIterator<Item = FocusUnit>,
    zoom_factor: i32,
    in_view: Option<&dyn Fn((i32, i32, i32)) -> bool>,
) -> Option<(i32, i32, i32)> {
    let zoom = zoom_factor as f32 * 1.525_878_9e-5;
    let lead = if -1.0 <= zoom {
        if zoom <= 1.0 {
            (1.0 - (f64::from(zoom) + 1.0) as f32 * 0.5) * 5.5
        } else {
            0.0
        }
    } else {
        5.5
    };
    let (mut sum, mut seen, mut n) = ((0i64, 0i64, 0i64), 0u32, 0i64);
    for u in units {
        let (x, y, z) = u.position;
        if in_view.is_some_and(|f| f((x, y, z))) {
            seen += 1;
        }
        let (mut ox, mut oy) = (0, 0);
        if u.ahead {
            let a = f64::from(f32::from(u.facing) * std::f32::consts::TAU * 1.525_878_9e-5);
            ox = i32::from((a.cos() * f64::from(lead) * 512.0) as i32 as i16);
            oy = i32::from((f64::from(lead) * a.sin() * 512.0) as i32 as i16);
        }
        sum.0 += i64::from(x + ox);
        sum.1 += i64::from(y + oy);
        sum.2 += i64::from(z);
        n += 1;
    }
    if n == 0 || (in_view.is_some() && seen as f32 / n as f32 >= 0.75) {
        return None;
    }
    Some(((sum.0 / n) as i32, (sum.1 / n) as i32, (sum.2 / n) as i32))
}

pub fn double_tap_words(
    (left, right): (bool, bool),
    (alt, ctrl, shift): (bool, bool, bool),
    repeat: bool,
    held: &[u16],
) -> [u32; 2] {
    use crate::selection_input as si;
    let mut flags = ActionFlags::default();
    for (on, flag) in [
        (left, si::LEFT_HELD),
        (right, si::RIGHT_HELD),
        (alt, si::ALT),
        (ctrl, si::CONTROL),
        (shift, si::SHIFT),
        (repeat, 0x2f),
    ] {
        if on {
            flags.set(flag);
        }
    }
    let record = engine_action(flags, held);
    [record.flags[0], record.flags[1]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection_input::{LEFT_HELD, SHIFT};

    #[test]
    fn rebinding_moves_the_special_ability() {
        use crate::controls::{ControlAction, Controls, HeldKeys, MythKey};
        let t = MythKey::from_name("T").unwrap();
        let free = MythKey::from_name("K").unwrap();
        let held = |k: MythKey| {
            let mut h = HeldKeys::default();
            h.press(k);
            h
        };
        let default = Controls::defaults();
        assert!(default.is_held(ControlAction::SpecialAbility, &held(t)));
        assert!(!default.is_held(ControlAction::SpecialAbility, &held(free)));
        let (moved, diags) =
            Controls::with_override("[bindings]\nspecial_ability = \"K\"\n", "user.toml");
        assert!(diags.is_empty(), "{diags:#?}");
        assert!(moved.is_held(ControlAction::SpecialAbility, &held(free)));
        assert!(!moved.is_held(ControlAction::SpecialAbility, &held(t)));
    }

    #[test]
    fn engine_action_sets_the_flag_words_and_no_key() {
        let mut flags = ActionFlags::default();
        flags.set(LEFT_HELD);
        flags.set(SHIFT);
        let a = engine_action(flags, &[2, 31, 51]);
        assert_eq!(a.key, -1);
        for f in [LEFT_HELD, SHIFT, 2, 31, 51] {
            assert!(a.flag(f), "flag {f}");
        }
        assert!(!a.flag(0) && !a.flag(0x1e));
        assert_eq!(a.flags[2..], [0, 0]);
    }

    #[test]
    fn without_the_engine_model_every_action_passes() {
        let mut sim = Simulation::new(1);
        let action = InputAction {
            time: 7,
            ..Default::default()
        };
        assert_eq!(
            sniff_polled_action(&mut sim, action, &[0, 31]),
            Some(action)
        );
    }

    #[test]
    fn double_tap_countdown_never_steps_onto_sentinel() {
        let mut dt = DoubleTap::default();
        assert!(!dt.key_event(MythKey(1), [1, 0]));
        dt.poll(DOUBLE_TAP_TICKS - 1);
        dt.poll(2);
        dt.poll(1);
        assert!(!dt.key_event(MythKey(1), [1, 0]));
    }
}
