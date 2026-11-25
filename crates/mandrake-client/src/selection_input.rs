use mandrake_view::selection::PixelRect;

pub const LEFT_HELD: u32 = 0x22;
pub const LEFT_PRESSED: u32 = 0x23;
pub const LEFT_DOUBLE: u32 = 0x24;
pub const RIGHT_HELD: u32 = 0x25;
pub const RIGHT_PRESSED: u32 = 0x26;
pub const RIGHT_DOUBLE: u32 = 0x27;
pub const ALT: u32 = 0x28;
pub const OPTION: u32 = 0x29;
pub const CONTROL: u32 = 0x2a;
pub const SHIFT: u32 = 0x2b;
pub const CLEAR_SELECTION: u32 = 0x0a;

pub const ATTACK_GROUND_HELD: u32 = 0x13;
pub const ATTACK_GROUND_PRESSED: u32 = 0x2e;

pub const GESTURE_DRAG_TARGET: i32 = 4;
pub const DRAG_GROUND: i32 = 8;
pub const SLOW_PRESS_TICKS: i32 = 15;
pub const FAR_DRAG: i32 = 450;
pub const MOVING_SPEED: i32 = 4;
pub const REST_TICKS: i16 = 1;
pub const SELECTION_LIMIT: usize = 0x7e;
const DOUBLE_CLICK_COUNTER: core::ops::RangeInclusive<i16> = 6..=14;
pub const DOUBLE_CLICK_DISTANCE: i32 = 7;
pub const DOUBLE_CLICK_TICKS: i32 = 9;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionFlags(u64);

impl ActionFlags {
    pub fn bits(self) -> u64 {
        self.0
    }

    pub fn test(self, flag: u32) -> bool {
        self.0 & (1 << flag) != 0
    }

    pub fn set(&mut self, flag: u32) {
        self.0 |= 1 << flag;
    }

    pub fn clear(&mut self, flag: u32) {
        self.0 &= !(1 << flag);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputAction {
    pub time: i32,
    pub elapsed: i16,
    pub cursor: (i16, i16),
    pub flags: ActionFlags,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollState {
    pub cursor: (i16, i16),
    pub left: bool,
    pub right: bool,
    pub alt: bool,
    pub option: bool,
    pub control: bool,
    pub shift: bool,
    pub clear_selection: bool,
    pub attack_ground: bool,
}

#[derive(Clone, Copy, Debug)]
struct EdgeRule {
    double_click: bool,
    source: u32,
    target: u32,
    counter: i16,
}

#[derive(Clone, Debug)]
pub struct ActionBuilder {
    time: i32,
    previous_time: Option<i32>,
    rules: [EdgeRule; 6],
}

impl Default for ActionBuilder {
    fn default() -> Self {
        let rule = |double_click, source, target| EdgeRule {
            double_click,
            source,
            target,
            counter: 0,
        };
        Self {
            time: 0,
            previous_time: None,
            rules: [
                rule(true, LEFT_HELD, LEFT_DOUBLE),
                rule(true, RIGHT_HELD, RIGHT_DOUBLE),
                rule(false, LEFT_HELD, LEFT_PRESSED),
                rule(false, RIGHT_HELD, RIGHT_PRESSED),
                rule(false, CLEAR_SELECTION, CLEAR_SELECTION),
                rule(false, ATTACK_GROUND_HELD, ATTACK_GROUND_PRESSED),
            ],
        }
    }
}

impl ActionBuilder {
    pub fn poll(&mut self, state: PollState) -> InputAction {
        let mut flags = ActionFlags::default();
        for (held, flag) in [
            (state.left, LEFT_HELD),
            (state.right, RIGHT_HELD),
            (state.alt, ALT),
            (state.option, OPTION),
            (state.control, CONTROL),
            (state.shift, SHIFT),
            (state.clear_selection, CLEAR_SELECTION),
            (state.attack_ground, ATTACK_GROUND_HELD),
        ] {
            if held {
                flags.set(flag);
            }
        }
        let elapsed = self
            .previous_time
            .map_or(0, |p| (self.time - p).clamp(0, 1)) as i16;
        self.previous_time = Some(self.time);
        let mut action = InputAction {
            time: self.time,
            elapsed,
            cursor: state.cursor,
            flags,
        };
        self.preprocess(&mut action);
        self.time += 1;
        action
    }

    fn preprocess(&mut self, action: &mut InputAction) {
        for rule in &mut self.rules {
            if action.flags.test(rule.source) {
                if rule.double_click {
                    if DOUBLE_CLICK_COUNTER.contains(&rule.counter) {
                        action.flags.set(rule.target);
                    }
                } else if rule.counter == 15 {
                    action.flags.clear(rule.target);
                } else {
                    action.flags.set(rule.target);
                }
                rule.counter = 15;
            } else if rule.counter > 0 {
                rule.counter -= action.elapsed;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum State {
    Idle = 1,
    ClickPending = 2,
    Gesture = 3,
    BandArmed = 4,
    Band = 5,
    Consumed = 6,
}

pub const EVAL_CONTROLLED: u8 = 1;
pub const EVAL_ALLIED: u8 = 2;
pub const EVAL_OWNED: u8 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonsterPick {
    pub id: u32,
    pub evaluation: u8,
    pub forced: bool,
    pub ambient: bool,
    pub flag_80000: bool,
    pub flags_8: u32,
    pub same_team: bool,
    pub owned: bool,
    pub selected: bool,
    pub near_miss: bool,
}

impl MonsterPick {
    pub fn own(id: u32, selected: bool) -> Self {
        Self {
            id,
            evaluation: EVAL_CONTROLLED | EVAL_ALLIED | EVAL_OWNED,
            same_team: true,
            owned: true,
            selected,
            ..Default::default()
        }
    }

    pub fn enemy(id: u32) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressTarget {
    Unit(MonsterPick),
    Model { point: (i32, i32, i32) },
    Ground { point: (i32, i32) },
    Nothing,
}

pub trait SelectionWorld {
    fn press_target(&mut self, cursor: (i16, i16)) -> PressTarget;
    fn band_units(&mut self, rect: PixelRect) -> Vec<u32>;
    fn polygon_target(&mut self, cursor: (i16, i16)) -> PressTarget {
        self.gesture_point(cursor)
            .map_or(PressTarget::Nothing, |point| PressTarget::Ground { point })
    }
    fn pick_up_target(&mut self, _cursor: (i16, i16), _selection: &[u32]) -> Option<(i16, i16)> {
        None
    }
    fn gesture_point(&mut self, _cursor: (i16, i16)) -> Option<(i32, i32)> {
        None
    }
    fn gesture_facing(&mut self, _from: (i16, i16), _to: (i16, i16)) -> u16 {
        0
    }
    fn unit_location(&mut self, _id: u32) -> Option<(i32, i32)> {
        None
    }
    fn same_type_flood(&mut self, base: u32) -> Vec<u32> {
        vec![base]
    }
    fn monster_visible(&mut self, _id: u32) -> bool {
        true
    }
    fn special_against_friendlies(&mut self, _selection: &[u32]) -> Vec<u32> {
        Vec::new()
    }
    fn all_controlled(&mut self, _selection: &[u32]) -> bool {
        true
    }
}

fn band_rect(a: (i16, i16), c: (i16, i16)) -> PixelRect {
    PixelRect::new(
        i32::from(a.0.min(c.0)),
        i32::from(a.1.min(c.1)),
        i32::from(a.0.max(c.0)),
        i32::from(a.1.max(c.1)),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PressContext {
    pub game_type: i16,
    pub game_options_high: u16,
    pub selection_mode: i16,
}

impl Default for PressContext {
    fn default() -> Self {
        Self {
            game_type: 0,
            game_options_high: 0,
            selection_mode: -1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonsterPress {
    Attack,
    Look,
    Own,
    Nothing,
    SpecialAttack,
    SpecialEmptied { own: bool },
    Through,
}

pub fn monster_press(
    pick: &MonsterPick,
    option: bool,
    shift: bool,
    selection_len: usize,
    ctx: &PressContext,
    any_against_friendlies: bool,
) -> MonsterPress {
    let mut e = pick.evaluation;
    if pick.forced {
        e &= if pick.ambient { !3 } else { !1 };
    }
    let option_shift = option && shift;
    let mode_3 = ctx.selection_mode == 3;
    let mut ecx = if pick.forced || !pick.same_team {
        option_shift && (pick.forced || !pick.owned)
    } else {
        option_shift
            && !pick.owned
            && ctx.game_options_high
                & mandrake_common::game_options::high(mandrake_common::game_options::TFL)
                != 0
            && !mode_3
    };
    if pick.flags_8 & 0x4000 != 0 {
        ecx = true;
    }
    let game_type_flag = match ctx.game_type {
        0xb => pick.flags_8 & 0x800 != 0,
        0xc => pick.flags_8 & 0x2000 != 0,
        _ => false,
    };
    let r8 = (pick.forced && (game_type_flag || pick.ambient)) || pick.flag_80000 || ecx;
    if mode_3 && !r8 && (pick.evaluation & EVAL_ALLIED != 0 || pick.forced || pick.same_team) {
        if any_against_friendlies {
            return MonsterPress::SpecialAttack;
        }
        if pick.forced {
            return MonsterPress::Through;
        }
        return MonsterPress::SpecialEmptied {
            own: e & EVAL_CONTROLLED != 0,
        };
    }
    if !r8 && pick.forced && !mode_3 {
        return MonsterPress::Nothing;
    }
    if selection_len > 0 && (e & EVAL_ALLIED == 0 || r8) {
        return MonsterPress::Attack;
    }
    if e & EVAL_CONTROLLED != 0 {
        MonsterPress::Own
    } else {
        MonsterPress::Look
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RightClickMode {
    Ignored,
    #[default]
    GestureMove,
    AttackGround,
    DeselectUnits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Game {
    #[default]
    Myth2,
    Tfl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionRules {
    pub game: Game,
    pub tfl_press: bool,
    pub press_drops_look: bool,
    pub far_drag: i32,
    pub tfl_drag: bool,
    pub screen_gesture: bool,
    pub waypoint_shift_at_release: bool,
    pub unit_order: bool,
    pub tfl_flood: bool,
    pub tfl_countdown: bool,
    pub attack_key_refuses_shift: bool,
    pub selection_cap: usize,
    pub selection_clears_orders: bool,
    pub modifier_flags: bool,
    pub select_all_drawn: bool,
    pub model_attack_keeps_mode: bool,
}

impl SelectionRules {
    pub const MYTH2: Self = Self {
        game: Game::Myth2,
        tfl_press: false,
        press_drops_look: true,
        far_drag: FAR_DRAG,
        tfl_drag: false,
        screen_gesture: false,
        waypoint_shift_at_release: false,
        unit_order: true,
        tfl_flood: false,
        tfl_countdown: false,
        attack_key_refuses_shift: true,
        selection_cap: SELECTION_LIMIT,
        selection_clears_orders: false,
        modifier_flags: true,
        select_all_drawn: true,
        model_attack_keeps_mode: true,
    };
    pub const TFL: Self = Self {
        game: Game::Tfl,
        tfl_press: true,
        press_drops_look: false,
        far_drag: TFL_FAR_DRAG,
        tfl_drag: true,
        screen_gesture: true,
        waypoint_shift_at_release: true,
        unit_order: false,
        tfl_flood: true,
        tfl_countdown: true,
        attack_key_refuses_shift: false,
        selection_cap: TFL_SELECTION_CAP,
        selection_clears_orders: true,
        modifier_flags: false,
        select_all_drawn: false,
        model_attack_keeps_mode: false,
    };
    pub const TFL_MYTH2: Self = Self {
        game: Game::Tfl,
        selection_cap: TFL_SELECTION_CAP,
        ..Self::MYTH2
    };
}

impl Default for SelectionRules {
    fn default() -> Self {
        Self::MYTH2
    }
}

pub const TFL_FAR_DRAG: i32 = 250;
pub const TFL_SELECTION_CAP: usize = 0x1e;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adjust {
    Add,
    Remove,
    Toggle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionChanges {
    pub nothing: bool,
    pub adjusted: bool,
}

pub fn tfl_monster_press(
    pick: &MonsterPick,
    commandable: bool,
    special_mode: bool,
) -> MonsterPress {
    if commandable && (pick.evaluation & EVAL_ALLIED == 0 || special_mode) {
        MonsterPress::Attack
    } else if pick.evaluation & EVAL_CONTROLLED == 0 {
        MonsterPress::Look
    } else {
        MonsterPress::Own
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackGroundStep {
    None,
    Refused,
    Enter,
    Exit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    None,
    AttackGround {
        point: (i32, i32),
        special: bool,
    },
    AttackModel {
        point: (i32, i32, i32),
        special: bool,
    },
    SelectOnly(u32),
    Added(u32),
    Attack {
        target: u32,
    },
    AttackAgain {
        attackers: Vec<u32>,
        target: u32,
        shift: bool,
    },
    Move {
        point: (i32, i32),
        facing: Option<u16>,
        add_waypoint: bool,
    },
    UnitOrder {
        facing: Option<u16>,
    },
    PickUp {
        index: i16,
        identifier: i16,
        units: Vec<u32>,
    },
    Released(State),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PressKind {
    #[default]
    Other,
    AttackTarget,
    OwnUnit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PressPick {
    Ground((i32, i32)),
    Unit(u32),
    #[default]
    Other,
}

#[derive(Clone, Debug)]
pub struct SelectionMachine {
    state: State,
    gesture_button: bool,
    anchor: InputAction,
    previous: InputAction,
    rest_ticks: i16,
    pick: PressPick,
    pressed_unit: Option<u32>,
    kind: PressKind,
    base_unit: Option<u32>,
    look: bool,
    attack: Option<(Vec<u32>, u32)>,
    saved_selection: Vec<u32>,
    bounds: PixelRect,
    pub left_drag_bands: bool,
    pub right_click: RightClickMode,
    pub context: PressContext,
    pub presets: crate::local_input::PresetKeys,
    pub rules: SelectionRules,
    changes: SelectionChanges,
}

impl Default for SelectionMachine {
    fn default() -> Self {
        Self {
            state: State::Idle,
            gesture_button: false,
            anchor: InputAction::default(),
            previous: InputAction::default(),
            rest_ticks: -1,
            pick: PressPick::Other,
            pressed_unit: None,
            kind: PressKind::Other,
            base_unit: None,
            look: false,
            attack: None,
            saved_selection: Vec::new(),
            bounds: PixelRect::new(0, 0, 0, 0),
            left_drag_bands: false,
            right_click: RightClickMode::default(),
            context: PressContext::default(),
            presets: crate::local_input::PresetKeys::default(),
            rules: SelectionRules::MYTH2,
            changes: SelectionChanges::default(),
        }
    }
}

pub(crate) fn chebyshev(a: (i16, i16), b: (i16, i16)) -> i32 {
    let dx = (i32::from(a.0) - i32::from(b.0)).abs();
    let dy = (i32::from(a.1) - i32::from(b.1)).abs();
    dx.max(dy)
}

fn euclid(a: (i16, i16), b: (i16, i16)) -> i32 {
    let dx = i32::from(a.0) - i32::from(b.0);
    let dy = i32::from(a.1) - i32::from(b.1);
    mandrake_math::engine::isqrt((dx * dx + dy * dy) as u32) as i32
}

impl SelectionMachine {
    pub fn new(right_click: RightClickMode) -> Self {
        Self {
            right_click,
            ..Default::default()
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn looking(&self) -> bool {
        self.look
    }

    pub fn stop_looking(&mut self) {
        self.look = false;
    }

    pub fn press_pick(&self) -> PressPick {
        self.pick
    }

    pub fn with_rules(rules: SelectionRules) -> Self {
        let mut presets = crate::local_input::PresetKeys::default();
        presets.tfl_countdown = rules.tfl_countdown;
        Self {
            rules,
            presets,
            right_click: if rules.game == Game::Tfl {
                RightClickMode::AttackGround
            } else {
                RightClickMode::default()
            },
            ..Default::default()
        }
    }

    pub fn consume(&mut self) {
        self.state = State::Consumed;
    }

    pub fn double_click(&self, action: &InputAction) -> bool {
        action.flags.test(LEFT_DOUBLE)
            && chebyshev(action.cursor, self.anchor.cursor) <= DOUBLE_CLICK_DISTANCE
            && self.anchor.time + DOUBLE_CLICK_TICKS >= action.time
    }

    pub fn held_units(&self) -> impl Iterator<Item = u32> + '_ {
        self.saved_selection
            .iter()
            .copied()
            .chain(self.pressed_unit)
    }

    pub fn forget_units(&mut self, keep: impl Fn(u32) -> bool) {
        self.saved_selection.retain(|&id| keep(id));
        if self.pressed_unit.is_some_and(|id| !keep(id)) {
            self.pressed_unit = None;
        }
    }

    pub fn held_gesture(&self) -> Option<((i16, i16), (i16, i16))> {
        (self.state == State::Gesture).then_some((self.anchor.cursor, self.previous.cursor))
    }

    pub fn gesture_facing(&self, world: &mut dyn SelectionWorld, end: (i16, i16)) -> u16 {
        if self.rules.screen_gesture {
            return world.gesture_facing(self.anchor.cursor, end);
        }
        let origin = match self.pick {
            PressPick::Ground(p) => Some(p),
            PressPick::Unit(id) => world.unit_location(id),
            PressPick::Other => None,
        };
        match (origin, world.gesture_point(end)) {
            (Some(o), Some(e)) => {
                mandrake_math::engine::arctangent(e.0.wrapping_sub(o.0), e.1.wrapping_sub(o.1))
            }
            _ => 0,
        }
    }

    pub fn band_bounds(&self) -> Option<PixelRect> {
        (self.state == State::Band).then_some(self.bounds)
    }

    pub fn update(
        &mut self,
        action: &InputAction,
        world: &mut dyn SelectionWorld,
        selection: &mut Vec<u32>,
    ) -> Outcome {
        self.changes = SelectionChanges::default();
        if self.look {
            if world.all_controlled(selection) {
                self.look = false;
            } else {
                selection.retain(|&id| world.monster_visible(id));
            }
        }
        let f = action.flags;
        if self.right_click == RightClickMode::Ignored
            && (f.test(RIGHT_HELD) || f.test(RIGHT_PRESSED) || f.test(RIGHT_DOUBLE))
        {
            return Outcome::Ignored;
        }
        let outcome = self.dispatch(action, world, selection);
        self.previous = *action;
        outcome
    }

    fn select_nothing(&mut self, selection: &mut Vec<u32>) {
        selection.clear();
        self.look = false;
        self.changes.nothing = true;
    }

    pub fn adjust(&mut self, selection: &mut Vec<u32>, id: u32, mode: Adjust) {
        let at = selection.iter().position(|&x| x == id);
        match (mode, at) {
            (Adjust::Add, Some(_)) | (Adjust::Remove, None) => return,
            (_, Some(i)) => {
                selection.remove(i);
            }
            (_, None) => {
                if selection.len() < self.rules.selection_cap {
                    selection.push(id);
                }
            }
        }
        self.changes.adjusted = true;
    }

    pub fn take_changes(&mut self) -> SelectionChanges {
        std::mem::take(&mut self.changes)
    }

    pub fn clear_selection(&mut self, selection: &mut Vec<u32>) {
        self.select_nothing(selection);
        if self.state == State::Band {
            for id in self.saved_selection.clone() {
                if !selection.contains(&id) {
                    self.adjust(selection, id, Adjust::Toggle);
                }
            }
        }
        self.state = State::Consumed;
    }

    fn dispatch(
        &mut self,
        action: &InputAction,
        world: &mut dyn SelectionWorld,
        selection: &mut Vec<u32>,
    ) -> Outcome {
        let f = action.flags;
        if f.test(CLEAR_SELECTION) {
            self.clear_selection(selection);
            return Outcome::None;
        }
        if self.double_click(action) {
            let outcome = match self.kind {
                PressKind::OwnUnit if self.rules.tfl_flood => {
                    if let Some(&base) = selection.last() {
                        let keep = f.test(ALT) || f.test(SHIFT);
                        let list = world.same_type_flood(base);
                        let all = list.iter().all(|id| selection.contains(id));
                        if !keep {
                            self.select_nothing(selection);
                        }
                        if all {
                            self.adjust(selection, base, Adjust::Toggle);
                        } else {
                            for id in list {
                                if !selection.contains(&id) {
                                    self.adjust(selection, id, Adjust::Toggle);
                                }
                            }
                        }
                    }
                    Outcome::None
                }
                PressKind::OwnUnit => {
                    if let Some(base) = self.base_unit {
                        let keep = f.test(ALT) || f.test(SHIFT);
                        let list = world.same_type_flood(base);
                        flood_selection_capped(
                            selection,
                            &list,
                            base,
                            keep,
                            self.rules.selection_cap,
                        );
                        if !keep {
                            self.look = false;
                        }
                    }
                    Outcome::None
                }
                PressKind::AttackTarget => match &self.attack {
                    Some((attackers, target)) if !attackers.is_empty() => Outcome::AttackAgain {
                        attackers: attackers.clone(),
                        target: *target,
                        shift: f.test(SHIFT),
                    },
                    _ => Outcome::None,
                },
                PressKind::Other => Outcome::None,
            };
            self.state = State::Consumed;
            self.kind = PressKind::Other;
            return outcome;
        }
        let idle = self.state == State::Idle;
        let left = if idle { LEFT_PRESSED } else { LEFT_HELD };
        let right = if idle { RIGHT_PRESSED } else { RIGHT_HELD };
        if f.test(RIGHT_PRESSED) && self.right_click == RightClickMode::DeselectUnits {
            self.select_nothing(selection);
        }
        if !(f.test(left) || f.test(right)) {
            return self.release(action, world, selection);
        }
        self.gesture_button = f.test(OPTION) || f.test(RIGHT_HELD);
        match self.state {
            State::Idle if self.rules.tfl_press => return self.press_tfl(action, world, selection),
            State::Idle => return self.press(action, world, selection),
            State::ClickPending => {
                if f.test(LEFT_HELD) && self.left_drag_bands {
                    self.state = State::BandArmed;
                }
                if self.rules.tfl_drag {
                    self.drag_tfl(action, world, selection);
                } else {
                    self.drag(action, world, selection);
                }
            }
            State::Gesture | State::BandArmed | State::Band if self.rules.tfl_drag => {
                self.drag_tfl(action, world, selection)
            }
            State::Gesture | State::BandArmed | State::Band => self.drag(action, world, selection),
            State::Consumed => {}
        }
        Outcome::None
    }

    fn release(
        &mut self,
        action: &InputAction,
        world: &mut dyn SelectionWorld,
        selection: &mut Vec<u32>,
    ) -> Outcome {
        let was = self.state;
        self.state = State::Idle;
        let mut facing = None;
        match was {
            State::Gesture => {
                let end = if self.rules.screen_gesture {
                    action.cursor
                } else {
                    self.previous.cursor
                };
                facing = Some(self.gesture_facing(world, end));
            }
            State::ClickPending | State::BandArmed => {
                if let Some(m) = self.pressed_unit
                    && !self.gesture_button
                {
                    self.select_only(selection, m);
                    self.kind = PressKind::OwnUnit;
                    self.base_unit = Some(m);
                    return Outcome::SelectOnly(m);
                }
            }
            State::Band => return Outcome::Released(was),
            State::Idle | State::Consumed => return Outcome::None,
        }
        if selection.is_empty() || self.look {
            return Outcome::Released(was);
        }
        let shift_at = if self.rules.waypoint_shift_at_release {
            action
        } else {
            &self.anchor
        };
        match self.pick {
            PressPick::Ground(point) => Outcome::Move {
                point,
                facing,
                add_waypoint: shift_at.flags.test(SHIFT),
            },
            PressPick::Unit(_) if self.rules.unit_order => Outcome::UnitOrder { facing },
            PressPick::Unit(_) => Outcome::Released(was),
            PressPick::Other => Outcome::Released(was),
        }
    }

    fn attack_ground_entry(&self, selection: &[u32], replay: bool) -> AttackGroundStep {
        if selection.is_empty() || self.look || replay {
            AttackGroundStep::Refused
        } else {
            AttackGroundStep::Enter
        }
    }

    pub fn attack_ground_right_press(
        &self,
        action: &InputAction,
        selection: &[u32],
        replay: bool,
    ) -> AttackGroundStep {
        let f = action.flags;
        if self.right_click == RightClickMode::AttackGround
            && f.test(RIGHT_PRESSED)
            && !f.test(CLEAR_SELECTION)
        {
            self.attack_ground_entry(selection, replay)
        } else {
            AttackGroundStep::None
        }
    }

    pub fn attack_ground_countdown(&mut self, action: &InputAction) -> AttackGroundStep {
        if self
            .presets
            .attack_ground_over(action.flags.test(ATTACK_GROUND_HELD), action.elapsed)
        {
            AttackGroundStep::Exit
        } else {
            AttackGroundStep::None
        }
    }

    pub fn attack_ground_key(
        &self,
        action: &InputAction,
        selection: &[u32],
        replay: bool,
    ) -> AttackGroundStep {
        if action.flags.test(ATTACK_GROUND_PRESSED)
            && !(self.rules.attack_key_refuses_shift && action.flags.test(SHIFT))
        {
            self.attack_ground_entry(selection, replay)
        } else {
            AttackGroundStep::None
        }
    }

    fn select_only(&mut self, selection: &mut Vec<u32>, m: u32) {
        self.select_nothing(selection);
        self.adjust(selection, m, Adjust::Toggle);
    }

    fn press(
        &mut self,
        action: &InputAction,
        world: &mut dyn SelectionWorld,
        selection: &mut Vec<u32>,
    ) -> Outcome {
        let f = action.flags;
        let (shift, alt, control) = (f.test(SHIFT), f.test(ALT), f.test(CONTROL));
        if self.look {
            self.select_nothing(selection);
        }
        self.pressed_unit = None;
        let mode = self.context.selection_mode;
        let target = if mode == 2 {
            world.polygon_target(action.cursor)
        } else {
            world.press_target(action.cursor)
        };
        let target = if let PressTarget::Unit(p) = target {
            let can = if mode == 3 {
                world.special_against_friendlies(selection)
            } else {
                Vec::new()
            };
            let press = monster_press(
                &p,
                f.test(OPTION),
                shift,
                selection.len(),
                &self.context,
                !can.is_empty(),
            );
            match press {
                MonsterPress::SpecialAttack => selection.retain(|id| can.contains(id)),
                MonsterPress::SpecialEmptied { .. } => selection.clear(),
                _ => {}
            }
            let near_own = p.near_miss
                && !shift
                && !alt
                && matches!(
                    press,
                    MonsterPress::Own | MonsterPress::SpecialEmptied { own: true }
                );
            if near_own {
                self.pressed_unit = Some(p.id);
            }
            if press == MonsterPress::Through || near_own {
                (world.polygon_target(action.cursor), press)
            } else {
                (PressTarget::Unit(p), press)
            }
        } else {
            (target, MonsterPress::Nothing)
        };
        let (target, press) = target;
        let second = !matches!(target, PressTarget::Unit(_));
        if second
            && matches!(mode, -1 | 9)
            && !selection.is_empty()
            && !self.look
            && let Some((index, identifier)) = world.pick_up_target(action.cursor, selection)
        {
            self.pick = PressPick::Other;
            self.state = State::Consumed;
            self.anchor = *action;
            self.kind = PressKind::Other;
            return Outcome::PickUp {
                index,
                identifier,
                units: selection.clone(),
            };
        }
        self.pick = match target {
            PressTarget::Unit(p) => PressPick::Unit(p.id),
            PressTarget::Ground { point } => PressPick::Ground(point),
            PressTarget::Model { .. } | PressTarget::Nothing => PressPick::Other,
        };
        let mut outcome = Outcome::None;
        let mut kind = PressKind::Other;
        let next = match target {
            PressTarget::Unit(p) => match press {
                MonsterPress::Attack | MonsterPress::SpecialAttack => {
                    if self.context.selection_mode != 3 {
                        self.attack = Some((selection.clone(), p.id));
                        kind = PressKind::AttackTarget;
                    }
                    outcome = Outcome::Attack { target: p.id };
                    Some(State::Consumed)
                }
                MonsterPress::Look | MonsterPress::SpecialEmptied { own: false } => {
                    self.select_nothing(selection);
                    self.adjust(selection, p.id, Adjust::Toggle);
                    self.look = true;
                    Some(State::Consumed)
                }
                MonsterPress::Nothing | MonsterPress::Through => None,
                MonsterPress::Own | MonsterPress::SpecialEmptied { own: true } => {
                    let (next, own) = self.own_unit_press(&p, shift, alt, selection);
                    if next.is_some() {
                        kind = PressKind::OwnUnit;
                    }
                    outcome = own;
                    next
                }
            },
            PressTarget::Ground { .. } | PressTarget::Model { .. } => {
                let model = matches!(target, PressTarget::Model { .. });
                if mode != 2 && (shift || alt) {
                    Some(if control && !model {
                        State::Consumed
                    } else {
                        State::BandArmed
                    })
                } else if selection.is_empty() {
                    Some(State::BandArmed)
                } else if mode == 2 || mode == 3 {
                    let special = mode == 3;
                    outcome = match target {
                        PressTarget::Model { point } => Outcome::AttackModel { point, special },
                        PressTarget::Ground { point } => Outcome::AttackGround { point, special },
                        _ => unreachable!(),
                    };
                    Some(State::Consumed)
                } else {
                    self.rest_ticks = -1;
                    Some(State::ClickPending)
                }
            }
            PressTarget::Nothing => Some(State::Consumed),
        };
        if let Some(next) = next {
            self.state = next;
            self.anchor = *action;
            self.kind = kind;
        }
        outcome
    }

    fn press_tfl(
        &mut self,
        action: &InputAction,
        world: &mut dyn SelectionWorld,
        selection: &mut Vec<u32>,
    ) -> Outcome {
        let f = action.flags;
        let (shift, alt) = (f.test(SHIFT), f.test(ALT));
        self.pressed_unit = None;
        self.anchor = *action;
        self.kind = PressKind::Other;
        let mode = self.context.selection_mode;
        let target = if mode == 2 {
            world.polygon_target(action.cursor)
        } else {
            world.press_target(action.cursor)
        };
        let commandable = !selection.is_empty() && !self.look;
        if !matches!(target, PressTarget::Unit(_))
            && mode == -1
            && commandable
            && let Some((index, identifier)) = world.pick_up_target(action.cursor, selection)
        {
            self.pick = PressPick::Other;
            self.state = State::Consumed;
            return Outcome::PickUp {
                index,
                identifier,
                units: selection.clone(),
            };
        }
        self.pick = match target {
            PressTarget::Unit(p) => PressPick::Unit(p.id),
            PressTarget::Ground { point } => PressPick::Ground(point),
            PressTarget::Model { .. } | PressTarget::Nothing => PressPick::Other,
        };
        let attack_mode = mode == 2 || mode == 3;
        let special = mode == 3;
        let (next, outcome) = match target {
            PressTarget::Unit(p) => match tfl_monster_press(&p, commandable, special) {
                MonsterPress::Attack => {
                    if !special {
                        self.attack = Some((selection.clone(), p.id));
                        self.kind = PressKind::AttackTarget;
                    }
                    (State::Consumed, Outcome::Attack { target: p.id })
                }
                MonsterPress::Look => {
                    self.select_nothing(selection);
                    self.adjust(selection, p.id, Adjust::Toggle);
                    self.look = true;
                    (State::Consumed, Outcome::None)
                }
                _ => {
                    let (next, own) = self.own_unit_press(&p, shift, false, selection);
                    self.kind = PressKind::OwnUnit;
                    (next.unwrap_or(State::Consumed), own)
                }
            },
            PressTarget::Ground { point } if !shift && commandable && attack_mode => {
                (State::Consumed, Outcome::AttackGround { point, special })
            }
            PressTarget::Model { point } if !shift && !alt && commandable && attack_mode => {
                (State::Consumed, Outcome::AttackModel { point, special })
            }
            PressTarget::Ground { .. } if !shift && commandable => {
                (State::ClickPending, Outcome::None)
            }
            PressTarget::Nothing if mode == 2 => {
                self.select_nothing(selection);
                (State::Consumed, Outcome::None)
            }
            _ => (State::BandArmed, Outcome::None),
        };
        self.state = next;
        outcome
    }

    fn drag_tfl(
        &mut self,
        action: &InputAction,
        world: &mut dyn SelectionWorld,
        selection: &mut Vec<u32>,
    ) {
        let pending = self.state == State::ClickPending;
        if (pending || self.state == State::Gesture) && !self.gesture_button {
            let dx = f64::from(i32::from(action.cursor.0) - i32::from(self.anchor.cursor.0));
            let dy = f64::from(i32::from(action.cursor.1) - i32::from(self.anchor.cursor.1));
            if action.time - self.anchor.time > SLOW_PRESS_TICKS
                || (dx * dx + dy * dy).sqrt() as i32 > self.rules.far_drag
            {
                self.state = if matches!(self.pick, PressPick::Unit(_)) {
                    State::Consumed
                } else {
                    State::BandArmed
                };
            }
        }
        if matches!(self.state, State::ClickPending | State::BandArmed)
            && chebyshev(action.cursor, self.anchor.cursor) >= DRAG_GROUND
        {
            if self.state == State::ClickPending {
                self.state = State::Gesture;
            } else {
                if !action.flags.test(SHIFT) {
                    self.select_nothing(selection);
                }
                self.saved_selection = selection.clone();
                self.state = State::Band;
            }
        }
        if self.state == State::Band {
            self.bounds = band_rect(self.anchor.cursor, action.cursor);
            self.select_nothing(selection);
            for id in self.saved_selection.clone() {
                self.adjust(selection, id, Adjust::Toggle);
            }
            for id in world.band_units(self.bounds) {
                self.adjust(selection, id, Adjust::Add);
            }
        }
    }

    fn own_unit_press(
        &mut self,
        p: &MonsterPick,
        shift: bool,
        alt: bool,
        selection: &mut Vec<u32>,
    ) -> (Option<State>, Outcome) {
        let m = p.id;
        let selected = selection.contains(&m);
        if shift {
            self.adjust(selection, m, Adjust::Toggle);
            self.base_unit = Some(m);
            let outcome = if selected {
                Outcome::None
            } else {
                Outcome::Added(m)
            };
            return (Some(State::Consumed), outcome);
        }
        if alt {
            if selected {
                self.adjust(selection, m, Adjust::Remove);
                self.base_unit = Some(m);
                return (Some(State::Consumed), Outcome::None);
            }
            return (None, Outcome::None);
        }
        self.rest_ticks = -1;
        self.base_unit = Some(m);
        if selected {
            self.pressed_unit = Some(m);
            (Some(State::ClickPending), Outcome::None)
        } else {
            self.select_only(selection, m);
            (Some(State::ClickPending), Outcome::SelectOnly(m))
        }
    }

    fn drag(
        &mut self,
        action: &InputAction,
        world: &mut dyn SelectionWorld,
        selection: &mut Vec<u32>,
    ) {
        let pending = self.state == State::ClickPending;
        let threshold = match self.state {
            State::ClickPending | State::Gesture
                if self.gesture_button || matches!(self.pick, PressPick::Unit(_)) =>
            {
                pending.then_some(GESTURE_DRAG_TARGET)
            }
            State::ClickPending | State::Gesture => {
                let from_press = euclid(action.cursor, self.anchor.cursor);
                let speed = euclid(action.cursor, self.previous.cursor);
                if speed > MOVING_SPEED {
                    self.rest_ticks = 0;
                } else if self.rest_ticks != -1 {
                    self.rest_ticks += action.elapsed;
                }
                if action.time - self.anchor.time > SLOW_PRESS_TICKS
                    || from_press > FAR_DRAG
                    || self.rest_ticks > REST_TICKS
                {
                    self.state = State::BandArmed;
                    Some(DRAG_GROUND)
                } else {
                    pending.then_some(DRAG_GROUND)
                }
            }
            State::BandArmed => Some(DRAG_GROUND),
            State::Band => None,
            State::Idle | State::Consumed => return,
        };
        if let Some(t) = threshold
            && chebyshev(action.cursor, self.anchor.cursor) >= t
        {
            match self.state {
                State::ClickPending => self.state = State::Gesture,
                State::BandArmed => {
                    let f = action.flags;
                    if !f.test(SHIFT) && !f.test(ALT) {
                        selection.clear();
                    }
                    self.saved_selection = selection.clone();
                    self.state = State::Band;
                }
                _ => {}
            }
        }
        if self.state == State::Band {
            self.band_step(action, world, selection);
        }
    }

    fn band_step(
        &mut self,
        action: &InputAction,
        world: &mut dyn SelectionWorld,
        selection: &mut Vec<u32>,
    ) {
        let subtract = self.anchor.flags.test(ALT);
        self.bounds = band_rect(self.anchor.cursor, action.cursor);
        let picked = world.band_units(self.bounds);
        let mut wanted = self.saved_selection.clone();
        let cap = self.rules.selection_cap;
        for id in picked {
            if wanted.len() >= cap {
                break;
            }
            band_list_add(&mut wanted, id, subtract, cap);
        }
        let removed: Vec<u32> = selection
            .iter()
            .copied()
            .filter(|id| !wanted.contains(id))
            .collect();
        let added: Vec<u32> = wanted
            .iter()
            .copied()
            .filter(|id| !selection.contains(id))
            .collect();
        for id in added {
            if selection.len() >= cap {
                break;
            }
            selection.push(id);
        }
        selection.retain(|id| !removed.contains(id));
    }
}

pub fn flood_selection_capped(
    selection: &mut Vec<u32>,
    list: &[u32],
    base: u32,
    keep: bool,
    cap: usize,
) {
    let all_selected = list.iter().all(|id| selection.contains(id));
    if !keep {
        selection.clear();
    }
    let toggle = |selection: &mut Vec<u32>, id: u32| {
        if let Some(i) = selection.iter().position(|&x| x == id) {
            selection.remove(i);
        } else if selection.len() < cap {
            selection.push(id);
        }
    };
    if all_selected {
        toggle(selection, base);
        return;
    }
    for &id in list {
        if !selection.contains(&id) {
            toggle(selection, id);
        }
    }
}

fn band_list_add(list: &mut Vec<u32>, id: u32, subtract: bool, cap: usize) {
    match (list.iter().position(|&x| x == id), subtract) {
        (Some(i), true) => {
            list.remove(i);
        }
        (None, false) if list.len() < cap => list.push(id),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct World {
        polygon: Option<PressTarget>,
        target: Option<PressTarget>,
        units: Vec<(u32, i32, i32)>,
        band_queries: usize,
        gesture: Option<(i32, i32)>,
        gesture_cursor: Option<(i16, i16)>,
        location: Option<(i32, i32)>,
        flood: Vec<u32>,
        controlled: bool,
        hidden: Vec<u32>,
        objects: Vec<(i16, i16, Vec<u32>)>,
    }

    impl SelectionWorld for World {
        fn pick_up_target(&mut self, _: (i16, i16), selection: &[u32]) -> Option<(i16, i16)> {
            self.objects
                .iter()
                .find(|(_, _, can)| selection.iter().all(|u| can.contains(u)))
                .map(|&(index, identifier, _)| (index, identifier))
        }

        fn press_target(&mut self, _: (i16, i16)) -> PressTarget {
            self.target.unwrap_or(PressTarget::Ground { point: (0, 0) })
        }

        fn polygon_target(&mut self, cursor: (i16, i16)) -> PressTarget {
            self.polygon.unwrap_or_else(|| {
                self.gesture_point(cursor)
                    .map_or(PressTarget::Nothing, |point| PressTarget::Ground { point })
            })
        }

        fn gesture_point(&mut self, cursor: (i16, i16)) -> Option<(i32, i32)> {
            self.gesture_cursor = Some(cursor);
            self.gesture
        }

        fn unit_location(&mut self, _: u32) -> Option<(i32, i32)> {
            self.location
        }

        fn same_type_flood(&mut self, base: u32) -> Vec<u32> {
            if self.flood.is_empty() {
                vec![base]
            } else {
                self.flood.clone()
            }
        }

        fn all_controlled(&mut self, _: &[u32]) -> bool {
            self.controlled
        }

        fn monster_visible(&mut self, id: u32) -> bool {
            !self.hidden.contains(&id)
        }

        fn band_units(&mut self, r: PixelRect) -> Vec<u32> {
            self.band_queries += 1;
            self.units
                .iter()
                .filter(|&&(_, x, y)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                .map(|&(id, _, _)| id)
                .collect()
        }
    }

    fn poll(b: &mut ActionBuilder, cursor: (i16, i16), left: bool) -> InputAction {
        b.poll(PollState {
            cursor,
            left,
            ..Default::default()
        })
    }

    #[test]
    fn tfl_adjust_modes_and_the_30_cap() {
        let mut m = SelectionMachine::with_rules(SelectionRules::TFL);
        let mut sel = Vec::new();
        m.adjust(&mut sel, 1, Adjust::Remove);
        assert_eq!(
            m.take_changes(),
            SelectionChanges::default(),
            "mode 1, not in"
        );
        m.adjust(&mut sel, 1, Adjust::Add);
        assert_eq!(sel, [1]);
        assert!(m.take_changes().adjusted);
        m.adjust(&mut sel, 1, Adjust::Add);
        assert!(!m.take_changes().adjusted, "mode 0, in already");
        m.adjust(&mut sel, 1, Adjust::Toggle);
        assert!(sel.is_empty() && m.take_changes().adjusted);
        for id in 0..40 {
            m.adjust(&mut sel, id, Adjust::Add);
        }
        assert_eq!(sel, (0..30).collect::<Vec<_>>());
        let _ = m.take_changes();
        m.adjust(&mut sel, 99, Adjust::Toggle);
        assert_eq!(sel.len(), 30);
        assert!(
            m.take_changes().adjusted,
            "the refused add clears the markers too"
        );
        let mut m2 = SelectionMachine::default();
        let mut sel = Vec::new();
        for id in 0..200 {
            m2.adjust(&mut sel, id, Adjust::Add);
        }
        assert_eq!(sel.len(), SELECTION_LIMIT);
    }

    #[test]
    fn tfl_band_is_select_nothing_each_poll_and_holds_30() {
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::with_rules(SelectionRules::TFL);
        let mut w = World {
            units: (0..40).map(|i| (100 + i, 10 + i as i32, 10)).collect(),
            ..World::default()
        };
        let mut sel = vec![7];
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        let mut shift = PollState {
            cursor: (0, 0),
            left: true,
            shift: true,
            ..Default::default()
        };
        m.update(&b.poll(shift), &mut w, &mut sel);
        assert_eq!(m.state(), State::BandArmed);
        shift.cursor = (5, 5);
        m.update(&b.poll(shift), &mut w, &mut sel);
        assert!(!m.take_changes().nothing);
        shift.cursor = (100, 20);
        m.update(&b.poll(shift), &mut w, &mut sel);
        assert_eq!(m.state(), State::Band);
        assert!(
            m.take_changes().nothing,
            "the band's poll is select_nothing"
        );
        assert_eq!(sel.len(), 30);
        assert_eq!(sel[0], 7);
        assert_eq!(&sel[1..], (100..129).collect::<Vec<_>>());
        let mut clear = sel.clone();
        m.clear_selection(&mut clear);
        assert!(m.take_changes().nothing);
        assert_eq!(clear, [7], "a band is undone to the selection before it");
    }

    #[test]
    fn tfl_flood_of_a_type_all_selected_keeps_the_one() {
        let mut m = SelectionMachine::with_rules(SelectionRules::TFL);
        let mut w = World {
            target: Some(PressTarget::Unit(MonsterPick::own(3, false))),
            flood: vec![2, 3],
            ..World::default()
        };
        let mut b = ActionBuilder::default();
        let mut sel = vec![2];
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        let added = m.update(
            &b.poll(PollState {
                left: true,
                shift: true,
                ..Default::default()
            }),
            &mut w,
            &mut sel,
        );
        assert_eq!(added, Outcome::Added(3));
        assert_eq!(sel, [2, 3]);
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        let double = poll(&mut b, (0, 0), true);
        assert!(double.flags.test(LEFT_DOUBLE));
        let _ = m.take_changes();
        let out = m.update(&double, &mut w, &mut sel);
        assert_eq!(out, Outcome::None);
        assert_eq!(sel, [3], "the last selected, alone");
        assert!(m.take_changes().nothing);
    }

    use mandrake_math::engine::isqrt;

    fn pre_myth_951_isqrt(n: u32) -> i32 {
        let (mut rem, mut root) = (n, 0u32);
        let mut bit = 1u32 << 30;
        while bit > rem {
            bit >>= 2;
        }
        while bit != 0 {
            if rem >= root + bit {
                rem -= root + bit;
                root = (root >> 1) + bit;
            } else {
                root >>= 1;
            }
            bit >>= 2;
        }
        if root < rem {
            root += 1;
        }
        root as i32
    }

    #[test]
    fn isqrt_matches_the_old_copy() {
        for k in 0u64..=0x1_0000 {
            for v in [k * k, k * k + k, k * k + k + 1]
                .into_iter()
                .flat_map(|v| [v.wrapping_sub(1), v, v + 1])
            {
                if let Ok(v) = u32::try_from(v) {
                    assert_eq!(isqrt(v) as i32, pre_myth_951_isqrt(v), "isqrt({v})");
                }
            }
        }
    }

    #[test]
    #[ignore = "exhaustive, 2^32 inputs"]
    fn isqrt_matches_the_old_copy_everywhere() {
        let threads = std::thread::available_parallelism().map_or(8, |n| n.get()) as u64;
        std::thread::scope(|s| {
            for t in 0..threads {
                s.spawn(move || {
                    for v in (t..=u64::from(u32::MAX)).step_by(threads as usize) {
                        let v = v as u32;
                        assert_eq!(isqrt(v) as i32, pre_myth_951_isqrt(v), "isqrt({v})");
                    }
                });
            }
        });
    }

    #[test]
    fn test_isqrt_rounds_to_nearest() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(2), 1);
        assert_eq!(isqrt(3), 2);
        assert_eq!(isqrt(20), 4);
        assert_eq!(isqrt(21), 5);
        assert_eq!(isqrt(202_500), 450);
    }

    #[test]
    fn test_action_builder_press_and_double_click() {
        let mut b = ActionBuilder::default();
        let a0 = poll(&mut b, (0, 0), false);
        assert_eq!(a0.elapsed, 0);
        let a1 = poll(&mut b, (0, 0), true);
        assert_eq!(a1.elapsed, 1);
        assert!(a1.flags.test(LEFT_HELD) && a1.flags.test(LEFT_PRESSED));
        assert!(!a1.flags.test(LEFT_DOUBLE));
        let a2 = poll(&mut b, (0, 0), true);
        assert!(a2.flags.test(LEFT_HELD) && !a2.flags.test(LEFT_PRESSED));
        poll(&mut b, (0, 0), false);
        let a4 = poll(&mut b, (0, 0), true);
        assert!(a4.flags.test(LEFT_PRESSED) && a4.flags.test(LEFT_DOUBLE));
        for _ in 0..10 {
            poll(&mut b, (0, 0), false);
        }
        let a = poll(&mut b, (0, 0), true);
        assert!(a.flags.test(LEFT_PRESSED) && !a.flags.test(LEFT_DOUBLE));
    }

    #[test]
    fn test_ground_press_without_selection_bands_at_8_pixels() {
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut w = World::default();
        let mut sel = Vec::new();
        m.update(&poll(&mut b, (100, 100), false), &mut w, &mut sel);
        assert_eq!(m.state(), State::Idle);
        m.update(&poll(&mut b, (100, 100), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::BandArmed);
        for x in 101..=107 {
            m.update(&poll(&mut b, (x, 103), true), &mut w, &mut sel);
            assert_eq!(m.state(), State::BandArmed, "x {x}");
            assert_eq!(m.band_bounds(), None);
        }
        m.update(&poll(&mut b, (108, 103), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Band);
        assert_eq!(m.band_bounds(), Some(PixelRect::new(100, 100, 108, 103)));
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        m.update(&poll(&mut b, (50, 50), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (50, 50), true), &mut w, &mut sel);
        m.update(&poll(&mut b, (45, 43), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::BandArmed);
        m.update(&poll(&mut b, (45, 42), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Band);
        assert_eq!(m.band_bounds(), Some(PixelRect::new(45, 42, 50, 50)));
        let out = m.update(&poll(&mut b, (45, 43), false), &mut w, &mut sel);
        assert_eq!(out, Outcome::Released(State::Band));
        assert_eq!(m.state(), State::Idle);
        assert_eq!(m.band_bounds(), None);
    }

    #[test]
    fn test_ground_press_with_selection_gesture_or_band() {
        let mut w = World::default();
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut sel = vec![7];
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::ClickPending);
        for x in [3, 6] {
            m.update(&poll(&mut b, (x, 0), true), &mut w, &mut sel);
            assert_eq!(m.state(), State::ClickPending);
        }
        m.update(&poll(&mut b, (9, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Gesture);
        assert_eq!(sel, [7]);
        let out = m.update(&poll(&mut b, (9, 0), false), &mut w, &mut sel);
        assert_eq!(
            out,
            Outcome::Move {
                point: (0, 0),
                facing: Some(0),
                add_waypoint: false
            }
        );

        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        for _ in 0..15 {
            m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
            assert_eq!(m.state(), State::ClickPending);
        }
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::BandArmed);
        m.update(&poll(&mut b, (0, 8), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Band);
        assert!(sel.is_empty());

        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut sel = vec![7];
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        m.update(&poll(&mut b, (5, 0), true), &mut w, &mut sel);
        m.update(&poll(&mut b, (5, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::ClickPending);
        m.update(&poll(&mut b, (5, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::BandArmed);
    }

    #[test]
    fn test_ground_gesture_becomes_band_after_15_ticks() {
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut w = World {
            units: vec![(1, 20, 5)],
            ..Default::default()
        };
        let mut sel = vec![7];
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        let mut x = 0;
        for _ in 0..15 {
            x += 6;
            m.update(&poll(&mut b, (x, 10), true), &mut w, &mut sel);
        }
        assert_eq!(m.state(), State::Gesture);
        x += 6;
        m.update(&poll(&mut b, (x, 10), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Band);
        assert_eq!(sel, [1]);
    }

    #[test]
    fn a_press_on_an_object_picks_it_up() {
        let press = |w: &mut World, sel: &mut Vec<u32>, mode: i16| {
            let mut b = ActionBuilder::default();
            let mut m = SelectionMachine::default();
            m.context.selection_mode = mode;
            m.update(&poll(&mut b, (0, 0), false), w, sel);
            let out = m.update(&poll(&mut b, (0, 0), true), w, sel);
            (out, m.state())
        };
        let mut w = World {
            target: Some(PressTarget::Ground { point: (5, 6) }),
            objects: vec![(963, 10095, vec![1, 2]), (960, 10042, vec![1])],
            ..Default::default()
        };
        let (out, state) = press(&mut w, &mut vec![1, 2], -1);
        assert_eq!(
            out,
            Outcome::PickUp {
                index: 963,
                identifier: 10095,
                units: vec![1, 2],
            }
        );
        assert_eq!(state, State::Consumed);
        w.objects[0].2 = vec![2];
        let (out, _) = press(&mut w, &mut vec![1], 9);
        assert!(matches!(out, Outcome::PickUp { index: 960, .. }), "{out:?}");
        let (out, state) = press(&mut w, &mut vec![3], -1);
        assert_eq!((out, state), (Outcome::None, State::ClickPending));
        let (out, state) = press(&mut w, &mut vec![], -1);
        assert_eq!((out, state), (Outcome::None, State::BandArmed));
        w.objects[0].2 = vec![1];
        w.polygon = w.target;
        let (out, _) = press(&mut w, &mut vec![1], 2);
        assert!(matches!(out, Outcome::AttackGround { .. }), "{out:?}");
    }

    #[test]
    fn test_unit_press_gestures_at_4_pixels() {
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut w = World {
            target: Some(PressTarget::Unit(MonsterPick::own(3, false))),
            ..Default::default()
        };
        let mut sel = Vec::new();
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::ClickPending);
        m.update(&poll(&mut b, (3, 3), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::ClickPending);
        m.update(&poll(&mut b, (4, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Gesture);
        for _ in 0..30 {
            m.update(&poll(&mut b, (40, 40), true), &mut w, &mut sel);
        }
        assert_eq!(m.state(), State::Gesture);
        assert_eq!(m.band_bounds(), None);
        let mut b = ActionBuilder::default();
        w.target = Some(PressTarget::Model { point: (5, 6, 7) });
        let mut m = SelectionMachine::default();
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut vec![1]);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut vec![1]);
        assert_eq!(m.state(), State::ClickPending);
        let out = m.update(&poll(&mut b, (0, 0), false), &mut w, &mut vec![1]);
        assert_eq!(out, Outcome::Released(State::ClickPending));
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut vec![]);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut vec![]);
        assert_eq!(m.state(), State::BandArmed);
        let mut b = ActionBuilder::default();
        w.target = Some(PressTarget::Unit(MonsterPick::enemy(9)));
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Consumed);
        m.update(&poll(&mut b, (50, 50), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Consumed);
    }

    #[test]
    fn test_modifiers_on_ground_press() {
        let mut b = ActionBuilder::default();
        let mut w = World {
            units: vec![(1, 10, 10), (2, 20, 10)],
            ..Default::default()
        };
        let press = |b: &mut ActionBuilder, shift, alt, control, cursor, left| {
            b.poll(PollState {
                cursor,
                left,
                shift,
                alt,
                control,
                ..Default::default()
            })
        };
        let mut m = SelectionMachine::default();
        let mut sel = vec![5];
        m.update(
            &press(&mut b, true, false, false, (0, 0), false),
            &mut w,
            &mut sel,
        );
        m.update(
            &press(&mut b, true, false, false, (0, 0), true),
            &mut w,
            &mut sel,
        );
        assert_eq!(m.state(), State::BandArmed);
        m.update(
            &press(&mut b, true, false, false, (15, 15), true),
            &mut w,
            &mut sel,
        );
        assert_eq!(m.state(), State::Band);
        assert_eq!(sel, [5, 1]);
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        m.update(
            &press(&mut b, true, false, true, (0, 0), false),
            &mut w,
            &mut sel,
        );
        m.update(
            &press(&mut b, true, false, true, (0, 0), true),
            &mut w,
            &mut sel,
        );
        assert_eq!(m.state(), State::Consumed);
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut sel = vec![1, 2, 5];
        m.update(
            &press(&mut b, false, true, false, (0, 0), false),
            &mut w,
            &mut sel,
        );
        m.update(
            &press(&mut b, false, true, false, (0, 0), true),
            &mut w,
            &mut sel,
        );
        m.update(
            &press(&mut b, false, true, false, (15, 15), true),
            &mut w,
            &mut sel,
        );
        assert_eq!(m.state(), State::Band);
        assert_eq!(sel, [2, 5]);
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut sel = vec![1, 2, 5];
        m.update(
            &press(&mut b, false, true, false, (0, 0), false),
            &mut w,
            &mut sel,
        );
        m.update(
            &press(&mut b, false, true, false, (0, 0), true),
            &mut w,
            &mut sel,
        );
        m.update(
            &press(&mut b, false, false, false, (15, 15), true),
            &mut w,
            &mut sel,
        );
        assert!(sel.is_empty());
    }

    #[test]
    fn test_band_repicks_every_action() {
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut w = World {
            units: vec![(1, 10, 10), (2, 30, 10), (3, 50, 10)],
            ..Default::default()
        };
        let mut sel = Vec::new();
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        m.update(&poll(&mut b, (20, 20), true), &mut w, &mut sel);
        assert_eq!(sel, [1]);
        m.update(&poll(&mut b, (60, 20), true), &mut w, &mut sel);
        assert_eq!(sel, [1, 2, 3]);
        m.update(&poll(&mut b, (40, 20), true), &mut w, &mut sel);
        assert_eq!(sel, [1, 2]);
        m.update(&poll(&mut b, (30, 20), true), &mut w, &mut sel);
        assert_eq!(sel, [1]);
        let queries = w.band_queries;
        assert_eq!(queries, 4);
        m.update(&poll(&mut b, (30, 20), false), &mut w, &mut sel);
        assert_eq!(sel, [1]);
        assert_eq!(w.band_queries, queries);
    }

    #[test]
    fn test_clear_selection_undoes_band() {
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut w = World {
            units: vec![(1, 10, 10)],
            ..Default::default()
        };
        let mut sel = vec![9];
        let p = |b: &mut ActionBuilder, cursor, left, shift, clear| {
            b.poll(PollState {
                cursor,
                left,
                shift,
                clear_selection: clear,
                ..Default::default()
            })
        };
        m.update(&p(&mut b, (0, 0), false, true, false), &mut w, &mut sel);
        m.update(&p(&mut b, (0, 0), true, true, false), &mut w, &mut sel);
        m.update(&p(&mut b, (20, 20), true, true, false), &mut w, &mut sel);
        assert_eq!(sel, [9, 1]);
        m.update(&p(&mut b, (20, 20), true, false, true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Consumed);
        assert_eq!(sel, [9]);
    }

    #[test]
    fn test_right_button_and_ignored_mode() {
        let mut w = World::default();
        let r = |b: &mut ActionBuilder, cursor, right| {
            b.poll(PollState {
                cursor,
                right,
                ..Default::default()
            })
        };
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut sel = Vec::new();
        m.update(&r(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&r(&mut b, (0, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::BandArmed);
        m.update(&r(&mut b, (8, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Band);
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut sel = vec![1];
        m.update(&r(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&r(&mut b, (0, 0), true), &mut w, &mut sel);
        m.update(&r(&mut b, (4, 0), true), &mut w, &mut sel);
        assert_eq!(m.state(), State::Gesture);
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine {
            right_click: RightClickMode::Ignored,
            ..Default::default()
        };
        m.update(&r(&mut b, (0, 0), false), &mut w, &mut sel);
        let out = m.update(&r(&mut b, (0, 0), true), &mut w, &mut sel);
        assert_eq!(out, Outcome::Ignored);
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn test_double_click_is_consumed() {
        let mut b = ActionBuilder::default();
        let mut m = SelectionMachine::default();
        let mut w = World::default();
        let mut sel = Vec::new();
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), true), &mut w, &mut sel);
        m.update(&poll(&mut b, (0, 0), false), &mut w, &mut sel);
        let a = poll(&mut b, (2, 2), true);
        assert!(a.flags.test(LEFT_DOUBLE));
        m.update(&a, &mut w, &mut sel);
        assert_eq!(m.state(), State::Consumed);
        m.update(&poll(&mut b, (40, 40), true), &mut w, &mut sel);
        assert_eq!(m.band_bounds(), None);
    }

    #[test]
    fn test_monster_press_rules() {
        let ctx = PressContext::default();
        let ally = MonsterPick {
            evaluation: EVAL_ALLIED,
            same_team: true,
            ..MonsterPick::enemy(4)
        };
        assert_eq!(
            monster_press(&ally, false, false, 3, &ctx, false),
            MonsterPress::Look
        );
        assert_eq!(
            monster_press(&ally, false, false, 0, &ctx, false),
            MonsterPress::Look
        );
        let hurt = MonsterPick {
            flags_8: 0x4000,
            ..ally
        };
        assert_eq!(
            monster_press(&hurt, false, false, 3, &ctx, false),
            MonsterPress::Attack
        );
        let flagged = MonsterPick {
            flag_80000: true,
            ..ally
        };
        assert_eq!(
            monster_press(&flagged, false, false, 3, &ctx, false),
            MonsterPress::Attack
        );
        assert_eq!(
            monster_press(&ally, true, true, 3, &ctx, false),
            MonsterPress::Look
        );
        let opt = PressContext {
            game_options_high: 0x20,
            ..ctx
        };
        assert_eq!(
            monster_press(&ally, true, true, 3, &opt, false),
            MonsterPress::Attack
        );
        let own = MonsterPick::own(5, false);
        assert_eq!(
            monster_press(&own, true, true, 3, &opt, false),
            MonsterPress::Own
        );
        let forced = MonsterPick {
            forced: true,
            ..MonsterPick::enemy(6)
        };
        assert_eq!(
            monster_press(&forced, false, false, 3, &ctx, false),
            MonsterPress::Nothing
        );
        let ambient = MonsterPick {
            ambient: true,
            ..forced
        };
        assert_eq!(
            monster_press(&ambient, false, false, 3, &ctx, false),
            MonsterPress::Attack
        );
        assert_eq!(
            monster_press(&ambient, false, false, 0, &ctx, false),
            MonsterPress::Look
        );
        let flag_ball = MonsterPick {
            flags_8: 0x800,
            ..forced
        };
        let ball_game = PressContext {
            game_type: 0xb,
            ..ctx
        };
        assert_eq!(
            monster_press(&flag_ball, false, false, 3, &ball_game, false),
            MonsterPress::Attack
        );
        let special = PressContext {
            selection_mode: 3,
            ..PressContext::default()
        };
        assert_eq!(
            monster_press(&MonsterPick::enemy(7), false, false, 3, &special, false),
            MonsterPress::Attack
        );
        assert_eq!(
            monster_press(&ally, false, false, 3, &special, true),
            MonsterPress::SpecialAttack
        );
        assert_eq!(
            monster_press(&ally, false, false, 3, &special, false),
            MonsterPress::SpecialEmptied { own: false }
        );
        let own = MonsterPick {
            evaluation: EVAL_ALLIED | EVAL_CONTROLLED,
            same_team: true,
            owned: true,
            ..MonsterPick::enemy(4)
        };
        assert_eq!(
            monster_press(&own, false, false, 3, &special, false),
            MonsterPress::SpecialEmptied { own: true }
        );
        let animal = MonsterPick {
            forced: true,
            ..MonsterPick::enemy(5)
        };
        assert_eq!(
            monster_press(&animal, false, false, 3, &special, false),
            MonsterPress::Through
        );
    }
}
