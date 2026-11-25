use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mandrake_view::camera::myth_observer::{EDGE_SCROLL_ZONE, ObserverAxes, WHEEL_NOTCH_TICKS};

pub static DEFAULT_CONTROLS_TOML: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| crate::conf::shipped_table(&["controls"]));
pub const LEFT_HANDED_TOML: &str = include_str!("../controls.left_handed.toml");

pub const PRESETS: [&str; 2] = ["right_handed", "left_handed"];
pub const TFL_PRESETS: [&str; 2] = ["tfl_right_handed", "tfl_left_handed"];

#[path = "controls_tfl.rs"]
pub(crate) mod tfl;

const KEY_NAMES: [&str; 100] = [
    "Escape",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "Backquote",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "0",
    "Minus",
    "Plus",
    "Delete",
    "Tab",
    "Q",
    "W",
    "E",
    "R",
    "T",
    "Y",
    "U",
    "I",
    "O",
    "P",
    "Left Brace",
    "Right Brace",
    "Backslash",
    "Caps Lock",
    "A",
    "S",
    "D",
    "F",
    "G",
    "H",
    "J",
    "K",
    "L",
    "Semicolon",
    "Quote",
    "Return",
    "Shift",
    "Z",
    "X",
    "C",
    "V",
    "B",
    "N",
    "M",
    "Less Than",
    "Greater Than",
    "Question Mark",
    "Control",
    "Option",
    "Alt/Command",
    "Space",
    "Up Arrow",
    "Down Arrow",
    "Left Arrow",
    "Right Arrow",
    "Help",
    "Home",
    "Page Up",
    "Forward Delete",
    "End",
    "Page Down",
    "Keypad Clear",
    "Keypad Equal",
    "Keypad Slash",
    "Keypad Asterisk",
    "Keypad 0",
    "Keypad 1",
    "Keypad 2",
    "Keypad 3",
    "Keypad 4",
    "Keypad 5",
    "Keypad 6",
    "Keypad 7",
    "Keypad 8",
    "Keypad 9",
    "Keypad Minus",
    "Keypad Plus",
    "Keypad Enter",
    "Keypad Period",
];

const KEY_ALIASES: [(&str, u8); 22] = [
    ("Esc", 0x00),
    ("Grave", 0x10),
    ("Tilde", 0x10),
    ("Equal", 0x1c),
    ("Equals", 0x1c),
    ("Backspace", 0x1d),
    ("Enter", 0x38),
    ("Ctrl", 0x44),
    ("Alt", 0x46),
    ("Command", 0x46),
    ("Cmd", 0x46),
    ("Up", 0x48),
    ("Down", 0x49),
    ("Left", 0x4a),
    ("Right", 0x4b),
    ("Insert", 0x4c),
    ("PageUp", 0x4e),
    ("PageDown", 0x51),
    ("Comma", 0x41),
    ("Period", 0x42),
    ("Slash", 0x43),
    ("Apostrophe", 0x37),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MythKey(pub u8);

impl MythKey {
    pub const SHIFT: Self = Self(0x39);
    pub const CONTROL: Self = Self(0x44);
    pub const OPTION: Self = Self(0x45);
    pub const ALT_COMMAND: Self = Self(0x46);

    pub fn name(self) -> &'static str {
        KEY_NAMES.get(self.0 as usize).copied().unwrap_or("?")
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let norm = |s: &str| {
            s.chars()
                .filter(|c| !c.is_whitespace() && *c != '_')
                .flat_map(char::to_lowercase)
                .collect::<String>()
        };
        let wanted = norm(name);
        if wanted.is_empty() {
            return None;
        }
        KEY_NAMES
            .iter()
            .position(|n| norm(n) == wanted)
            .map(|i| Self(i as u8))
            .or_else(|| {
                KEY_ALIASES
                    .iter()
                    .find(|(n, _)| norm(n) == wanted)
                    .map(|&(_, code)| Self(code))
            })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Modifiers {
    fn count(self) -> usize {
        self.ctrl as usize + self.shift as usize + self.alt as usize
    }

    fn satisfied_by(self, held: Modifiers) -> bool {
        (!self.ctrl || held.ctrl) && (!self.shift || held.shift) && (!self.alt || held.alt)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Binding {
    pub key: MythKey,
    pub modifiers: Modifiers,
}

impl Binding {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (mods, key) = if text == "+" {
            ("", "Plus")
        } else if let Some(prefix) = text.strip_suffix("++") {
            (prefix, "Plus")
        } else {
            match text.rfind('+') {
                Some(i) if i > 0 => (&text[..i], &text[i + 1..]),
                _ => ("", text),
            }
        };
        let key = MythKey::from_name(key).ok_or_else(|| format!("unknown key name {key:?}"))?;
        let mut modifiers = Modifiers::default();
        for m in mods.split('+').map(str::trim).filter(|m| !m.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.ctrl = true,
                "shift" => modifiers.shift = true,
                "alt" | "option" | "cmd" | "command" | "alt/command" => modifiers.alt = true,
                _ => return Err(format!("unknown modifier {m:?} in {text:?}")),
            }
        }
        Ok(Self { key, modifiers })
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.modifiers.shift {
            f.write_str("Shift+")?;
        }
        if self.modifiers.alt {
            f.write_str("Alt+")?;
        }
        f.write_str(self.key.name())
    }
}

macro_rules! actions {
    ($($variant:ident = $name:literal, $engine:expr;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum ControlAction {
            $($variant,)*
        }

        impl ControlAction {
            pub const ALL: &'static [ControlAction] = &[$(ControlAction::$variant,)*];

            pub fn config_name(self) -> &'static str {
                match self {
                    $(ControlAction::$variant => $name,)*
                }
            }

            pub fn engine_id(self) -> Option<u16> {
                match self {
                    $(ControlAction::$variant => $engine,)*
                }
            }

            pub fn from_config_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(ControlAction::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

actions! {
    MoveForward = "move_forward", Some(2);
    MoveBackward = "move_backward", Some(3);
    MoveLeft = "move_left", Some(6);
    MoveRight = "move_right", Some(7);
    TurnLeft = "turn_left", Some(4);
    TurnRight = "turn_right", Some(5);
    OrbitLeft = "orbit_left", Some(0);
    OrbitRight = "orbit_right", Some(1);
    ZoomIn = "zoom_in", Some(8);
    ZoomOut = "zoom_out", Some(9);
    ClearSelection = "clear_selection", Some(10);
    RotatePreset = "rotate_preset", Some(11);
    StopUnits = "stop_units", Some(12);
    SelectAll = "select_all", Some(13);
    ClearPreset = "clear_preset", Some(14);
    RecallSetPreset = "recall_set_preset", Some(15);
    CenterSelection = "center_selection", Some(16);
    ScatterUnits = "scatter_units", Some(17);
    RetreatUnits = "retreat_units", Some(18);
    AttackGround = "attack_ground", Some(19);
    SpecialAbility = "special_ability", Some(20);
    RenameUnits = "rename_units", Some(21);
    ChatMessage = "chat_message", Some(22);
    DetachUnits = "detach_units", Some(23);
    GuardUnits = "guard_units", Some(24);
    OverheadMap = "overhead_map", Some(30);
    PauseGame = "pause_game", Some(31);
    DisplayChat = "display_chat", Some(33);
    Taunt = "taunt", Some(50);
    RotateUnitsRight = "rotate_units_right", Some(51);
    RotateUnitsLeft = "rotate_units_left", Some(52);
    Inventory = "inventory", Some(53);
    PresetSelection1 = "preset_selection_1", Some(25);
    PresetSelection2 = "preset_selection_2", Some(25);
    PresetSelection3 = "preset_selection_3", Some(25);
    PresetSelection4 = "preset_selection_4", Some(25);
    PresetSelection5 = "preset_selection_5", Some(25);
    PresetSelection6 = "preset_selection_6", Some(25);
    PresetSelection7 = "preset_selection_7", Some(25);
    PresetSelection8 = "preset_selection_8", Some(25);
    PresetSelection9 = "preset_selection_9", Some(25);
    PresetSelection0 = "preset_selection_0", Some(25);
    FormBreakAlliance = "form_break_alliance", Some(32);
    OrbitZoomModifier = "orbit_zoom_modifier", Some(48);
    Quit = "quit", None;
    GameSpeedDown = "game_speed_down", None;
    GameSpeedUp = "game_speed_up", None;
    Screenshot = "screenshot", None;
    ToggleFpsDisplay = "toggle_fps_display", None;
    ToggleFullscreen = "toggle_fullscreen", None;
    Quicksave = "quicksave", None;
    ToggleControlBar = "toggle_control_bar", None;
    ShowScores = "show_scores", None;
    ToggleScores = "toggle_scores", None;
    ShowKeyBindings = "show_key_bindings", None;
}

impl ControlAction {
    pub const PRESET_SELECTIONS: [ControlAction; 10] = [
        ControlAction::PresetSelection1,
        ControlAction::PresetSelection2,
        ControlAction::PresetSelection3,
        ControlAction::PresetSelection4,
        ControlAction::PresetSelection5,
        ControlAction::PresetSelection6,
        ControlAction::PresetSelection7,
        ControlAction::PresetSelection8,
        ControlAction::PresetSelection9,
        ControlAction::PresetSelection0,
    ];

    pub fn preset_index(self) -> Option<usize> {
        Self::PRESET_SELECTIONS.iter().position(|&a| a == self)
    }

    pub fn is_held_only(self) -> bool {
        matches!(
            self,
            ControlAction::MoveForward
                | ControlAction::MoveBackward
                | ControlAction::MoveLeft
                | ControlAction::MoveRight
                | ControlAction::TurnLeft
                | ControlAction::TurnRight
                | ControlAction::OrbitLeft
                | ControlAction::OrbitRight
                | ControlAction::AttackGround
                | ControlAction::RecallSetPreset
                | ControlAction::OrbitZoomModifier
                | ControlAction::ShowScores
                | ControlAction::ShowKeyBindings
        )
    }
}

macro_rules! option_enum {
    ($(#[$doc:meta])* $name:ident { $($variant:ident = $text:literal,)* }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            pub const VALUES: &'static [&'static str] = &[$($text,)*];

            fn parse(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Self::$variant),)*
                    _ => None,
                }
            }

            #[allow(dead_code)]
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)*
                }
            }
        }
    };
}

option_enum! {
    RightClick {
        Ignored = "ignored",
        GestureMove = "gesture_move",
        AttackGround = "attack_ground",
        DeselectUnits = "deselect_units",
    }
}

option_enum! {
    MouseWheel {
        Ignored = "ignored",
        Move = "move",
        Zoom = "zoom",
        CyclePresets = "cycle_presets",
        CycleInventory = "cycle_inventory",
    }
}

option_enum! {
    CapsLock {
        Ignored = "ignored",
        SelectionBoxes = "selection_boxes",
        HealthBars = "health_bars",
        PlayerStats = "player_stats",
        ChatBacklog = "chat_backlog",
    }
}

option_enum! {
    HorizMouseMovement {
        Ignored = "ignored",
        Move = "move",
        Rotate = "rotate",
        Orbit = "orbit",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlOptions {
    pub right_click: RightClick,
    pub mouse_wheel: MouseWheel,
    pub caps_lock: CapsLock,
    pub horiz_mouse_movement: HorizMouseMovement,
    pub enable_mouse_movement: bool,
    pub top_corners_rotate_camera: bool,
    pub bottom_corners_orbit_camera: bool,
    pub reverse_axis: bool,
    pub left_click_doesnt_gesture: bool,
    pub tfl_mouse_actions: bool,
    pub use_control_bar_inventory: bool,
    pub alternate_preset_recall: bool,
    pub right_click_through_ui: bool,
    pub tilde_ignores_inventory: bool,
    pub system_keyboard_layout: bool,
    pub alternate_unit_selection: bool,
}

impl Default for ControlOptions {
    fn default() -> Self {
        Self {
            right_click: RightClick::GestureMove,
            mouse_wheel: MouseWheel::Zoom,
            caps_lock: CapsLock::SelectionBoxes,
            horiz_mouse_movement: HorizMouseMovement::Move,
            enable_mouse_movement: true,
            top_corners_rotate_camera: false,
            bottom_corners_orbit_camera: false,
            reverse_axis: false,
            left_click_doesnt_gesture: false,
            tfl_mouse_actions: false,
            use_control_bar_inventory: false,
            alternate_preset_recall: false,
            right_click_through_ui: false,
            tilde_ignores_inventory: false,
            system_keyboard_layout: true,
            alternate_unit_selection: false,
        }
    }
}

impl ControlOptions {
    fn bool_mut(&mut self, name: &str) -> Option<&mut bool> {
        Some(match name {
            "enable_mouse_movement" => &mut self.enable_mouse_movement,
            "top_corners_rotate_camera" => &mut self.top_corners_rotate_camera,
            "bottom_corners_orbit_camera" => &mut self.bottom_corners_orbit_camera,
            "reverse_axis" => &mut self.reverse_axis,
            "left_click_doesnt_gesture" => &mut self.left_click_doesnt_gesture,
            "tfl_mouse_actions" => &mut self.tfl_mouse_actions,
            "use_control_bar_inventory" => &mut self.use_control_bar_inventory,
            "alternate_preset_recall" => &mut self.alternate_preset_recall,
            "right_click_through_ui" => &mut self.right_click_through_ui,
            "tilde_ignores_inventory" => &mut self.tilde_ignores_inventory,
            "system_keyboard_layout" => &mut self.system_keyboard_layout,
            "alternate_unit_selection" => &mut self.alternate_unit_selection,
            _ => return None,
        })
    }

    fn set_enum(&mut self, name: &str, value: &str) -> Option<Result<(), &'static [&'static str]>> {
        fn set<T: Copy>(
            slot: &mut T,
            v: Option<T>,
            values: &'static [&'static str],
        ) -> Result<(), &'static [&'static str]> {
            v.map(|v| *slot = v).ok_or(values)
        }
        Some(match name {
            "right_click" => set(
                &mut self.right_click,
                RightClick::parse(value),
                RightClick::VALUES,
            ),
            "mouse_wheel" => set(
                &mut self.mouse_wheel,
                MouseWheel::parse(value),
                MouseWheel::VALUES,
            ),
            "caps_lock" => set(
                &mut self.caps_lock,
                CapsLock::parse(value),
                CapsLock::VALUES,
            ),
            "horiz_mouse_movement" => set(
                &mut self.horiz_mouse_movement,
                HorizMouseMovement::parse(value),
                HorizMouseMovement::VALUES,
            ),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: String,
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{}: {}", self.source, line, self.message),
            None => write!(f, "{}: {}", self.source, self.message),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controls {
    bindings: BTreeMap<ControlAction, Vec<Binding>>,
    pub options: ControlOptions,
}

impl Default for Controls {
    fn default() -> Self {
        Self::defaults()
    }
}

pub(crate) fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

impl Controls {
    pub fn defaults() -> Self {
        let mut controls = Self {
            bindings: BTreeMap::new(),
            options: ControlOptions::default(),
        };
        let diags = controls.apply_toml(&DEFAULT_CONTROLS_TOML, "<defaults>", false);
        debug_assert!(diags.is_empty(), "shipped defaults are invalid: {diags:?}");
        controls
    }

    pub fn preset(name: &str) -> Option<Self> {
        let mut controls = Self::defaults();
        match name {
            "right_handed" => {}
            "left_handed" => {
                controls.apply_toml(LEFT_HANDED_TOML, "<left_handed>", false);
            }
            "tfl_right_handed" => {
                let set = tfl::bindings_toml(&tfl::RIGHT_HANDED);
                controls.apply_toml(&set, "<TFL Right Handed>", false);
                controls.apply_toml(tfl::PRESET_OPTIONS, "<TFL options>", false);
            }
            "tfl_left_handed" => {
                let set = tfl::bindings_toml(&tfl::left_handed());
                controls.apply_toml(&set, "<TFL Left Handed>", false);
                controls.apply_toml(tfl::PRESET_OPTIONS, "<TFL options>", false);
            }
            _ => return None,
        }
        Some(controls)
    }

    pub fn with_override(text: &str, source: &str) -> (Self, Vec<Diagnostic>) {
        let mut controls = Self::defaults();
        let diags = controls.apply_toml(text, source, true);
        let mut all = diags;
        all.extend(controls.duplicate_warnings(source));
        (controls, all)
    }

    pub fn tfl_with_override(text: &str, source: &str) -> (Self, Vec<Diagnostic>) {
        let mut controls = Self::preset(TFL_PRESETS[0]).unwrap_or_else(Self::defaults);
        let diags = controls.apply_toml_with(text, source, Some("tfl_"));
        let mut all = diags;
        all.extend(controls.duplicate_warnings(source));
        (controls, all)
    }

    fn apply_toml(&mut self, text: &str, source: &str, allow_preset: bool) -> Vec<Diagnostic> {
        self.apply_toml_with(text, source, allow_preset.then_some(""))
    }

    fn apply_toml_with(
        &mut self,
        text: &str,
        source: &str,
        presets: Option<&str>,
    ) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        let at = |offset: Option<std::ops::Range<usize>>| offset.map(|r| line_of(text, r.start));
        let mut report = |line: Option<usize>, message: String| {
            diags.push(Diagnostic {
                source: source.to_string(),
                line,
                message,
            })
        };
        let doc = match toml_edit::Document::parse(text.to_string()) {
            Ok(doc) => doc,
            Err(e) => {
                report(
                    at(e.span()),
                    format!("TOML syntax error: {}", e.message().trim()),
                );
                return diags;
            }
        };
        let root = doc.as_table();

        if let Some((key, item)) = root.get_key_value("preset") {
            let line = at(key.span());
            match (item.as_str(), presets) {
                (Some(name), Some(prefix)) => {
                    let full = if name.starts_with(prefix) {
                        name.to_string()
                    } else {
                        format!("{prefix}{name}")
                    };
                    let known = prefix.is_empty() != full.starts_with("tfl_");
                    match Self::preset(&full).filter(|_| known) {
                        Some(base) => *self = base,
                        None if prefix.is_empty() => report(
                            line,
                            format!("unknown preset {name:?} (expected one of {PRESETS:?})"),
                        ),
                        None => report(
                            line,
                            format!("unknown preset {name:?} (expected one of {TFL_PRESETS:?})"),
                        ),
                    }
                }
                (Some(_), None) => report(line, "`preset` is only allowed in a user file".into()),
                (None, _) => report(line, "`preset` must be a string".into()),
            }
        }

        for (name, item) in root.iter() {
            if matches!(name, "preset" | "bindings" | "options") {
                continue;
            }
            let line = root.get_key_value(name).and_then(|(k, _)| at(k.span()));
            report(
                line,
                format!("unknown top-level entry `{name}` (expected [bindings] or [options])"),
            );
            let _ = item;
        }

        if let Some(item) = root.get("bindings") {
            match item.as_table() {
                Some(table) => {
                    for (name, item) in table.iter() {
                        let line = table.get_key_value(name).and_then(|(k, _)| at(k.span()));
                        let Some(action) = ControlAction::from_config_name(name) else {
                            report(line, format!("unknown action `{name}`"));
                            continue;
                        };
                        let values: Vec<(Option<usize>, Option<&str>)> = match item.as_array() {
                            Some(array) => array
                                .iter()
                                .map(|v| (at(v.span()).or(line), v.as_str()))
                                .collect(),
                            None => vec![(at(item.span()).or(line), item.as_str())],
                        };
                        let mut bindings = Vec::new();
                        let mut ok = true;
                        for (vline, value) in values {
                            match value.map(Binding::parse) {
                                Some(Ok(b)) => {
                                    if !bindings.contains(&b) {
                                        bindings.push(b);
                                    }
                                }
                                Some(Err(e)) => {
                                    report(vline, format!("`{name}`: {e}; keeping the default"));
                                    ok = false;
                                }
                                None => {
                                    report(
                                        vline,
                                        format!(
                                            "`{name}`: bindings must be strings; keeping the default"
                                        ),
                                    );
                                    ok = false;
                                }
                            }
                        }
                        if ok {
                            self.bindings.insert(action, bindings);
                        }
                    }
                }
                None => report(at(item.span()), "`bindings` must be a table".into()),
            }
        }

        if let Some(item) = root.get("options") {
            match item.as_table() {
                Some(table) => {
                    for (name, item) in table.iter() {
                        let line = table.get_key_value(name).and_then(|(k, _)| at(k.span()));
                        if self.options.bool_mut(name).is_some() {
                            match item.as_bool() {
                                Some(v) => *self.options.bool_mut(name).expect("checked") = v,
                                None => report(
                                    line,
                                    format!(
                                        "option `{name}` must be true or false; keeping the default"
                                    ),
                                ),
                            }
                            continue;
                        }
                        let value = item.as_str().unwrap_or("");
                        match self.options.set_enum(name, value) {
                            Some(Ok(())) => {}
                            Some(Err(values)) => report(
                                line,
                                format!(
                                    "option `{name}`: bad value {:?} (expected one of {values:?}); keeping the default",
                                    item.as_str().map_or_else(
                                        || item.type_name().to_string(),
                                        str::to_string
                                    )
                                ),
                            ),
                            None => report(line, format!("unknown option `{name}`")),
                        }
                    }
                }
                None => report(at(item.span()), "`options` must be a table".into()),
            }
        }
        diags
    }

    pub fn duplicate_warnings(&self, source: &str) -> Vec<Diagnostic> {
        let mut by_binding: BTreeMap<Binding, Vec<ControlAction>> = BTreeMap::new();
        for (&action, bindings) in &self.bindings {
            for &b in bindings {
                by_binding.entry(b).or_default().push(action);
            }
        }
        by_binding
            .into_iter()
            .filter(|(_, actions)| actions.len() > 1)
            .map(|(b, actions)| Diagnostic {
                source: source.to_string(),
                line: None,
                message: format!(
                    "`{b}` is bound to several actions: {}",
                    actions
                        .iter()
                        .map(|a| a.config_name())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
            .collect()
    }

    pub fn bindings(&self, action: ControlAction) -> &[Binding] {
        self.bindings.get(&action).map_or(&[], Vec::as_slice)
    }

    pub fn is_held(&self, action: ControlAction, keys: &HeldKeys) -> bool {
        let mods = keys.modifiers();
        self.bindings(action)
            .iter()
            .any(|b| keys.is_down(b.key) && b.modifiers.satisfied_by(mods))
    }

    pub fn actions_for_press(&self, key: MythKey, keys: &HeldKeys) -> Vec<ControlAction> {
        self.actions_for_press_with(key, keys.modifiers())
    }

    pub fn actions_for_press_with(&self, key: MythKey, mods: Modifiers) -> Vec<ControlAction> {
        let mut best = 0;
        let mut found: Vec<(usize, ControlAction)> = Vec::new();
        for (&action, bindings) in &self.bindings {
            for b in bindings {
                if b.key == key && b.modifiers.satisfied_by(mods) {
                    let n = b.modifiers.count();
                    best = best.max(n);
                    found.push((n, action));
                }
            }
        }
        let mut out: Vec<ControlAction> = found
            .into_iter()
            .filter(|&(n, _)| n == best)
            .map(|(_, a)| a)
            .collect();
        out.dedup();
        out
    }

    pub fn toggles_fullscreen(&self, key: MythKey, mods: Modifiers) -> bool {
        self.actions_for_press_with(key, mods)
            .contains(&ControlAction::ToggleFullscreen)
    }

    pub fn keyboard_axes(&self, keys: &HeldKeys) -> ObserverAxes {
        let held = |a| if self.is_held(a, keys) { 1.0 } else { 0.0 };
        ObserverAxes {
            forward: held(ControlAction::MoveForward) - held(ControlAction::MoveBackward),
            left: held(ControlAction::MoveLeft) - held(ControlAction::MoveRight),
            turn_left: held(ControlAction::TurnLeft) - held(ControlAction::TurnRight),
            orbit_left: held(ControlAction::OrbitLeft) - held(ControlAction::OrbitRight),
            zoom_in: held(ControlAction::ZoomIn) - held(ControlAction::ZoomOut),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeldKeys {
    down: BTreeSet<MythKey>,
}

impl HeldKeys {
    pub fn press(&mut self, key: MythKey) {
        self.down.insert(key);
    }

    pub fn release(&mut self, key: MythKey) {
        self.down.remove(&key);
    }

    pub fn clear(&mut self) {
        self.down.clear();
    }

    pub fn is_down(&self, key: MythKey) -> bool {
        self.down.contains(&key)
    }

    pub fn modifiers(&self) -> Modifiers {
        Modifiers {
            ctrl: self.is_down(MythKey::CONTROL),
            shift: self.is_down(MythKey::SHIFT),
            alt: self.is_down(MythKey::ALT_COMMAND) || self.is_down(MythKey::OPTION),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeScroll {
    pub zone: f32,
    pub ramp: bool,
    pub reverse_axis: bool,
}

impl EdgeScroll {
    pub const MYTH2: Self = Self {
        zone: EDGE_SCROLL_ZONE,
        ramp: true,
        reverse_axis: true,
    };
}

pub fn edge_scroll_axes(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    options: &ControlOptions,
) -> ObserverAxes {
    edge_scroll_axes_as(x, y, width, height, options, &EdgeScroll::MYTH2)
}

pub fn edge_scroll_axes_as(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    options: &ControlOptions,
    edge: &EdgeScroll,
) -> ObserverAxes {
    let zone = edge.zone;
    let mut axes = ObserverAxes::default();
    if !options.enable_mouse_movement || width <= 2.0 * zone || height <= 2.0 * zone {
        return axes;
    }
    let ramp = |d: f32| {
        if edge.ramp {
            (1.0 - d / zone).clamp(0.0, 1.0)
        } else {
            1.0
        }
    };
    let (mut strength, mut forward) = (0.0f32, 0.0f32);
    if y <= zone {
        strength = ramp(y);
        forward = 1.0;
    } else if y >= height - zone {
        strength = ramp(height - y);
        forward = -1.0;
    }
    let mut side = 0.0f32;
    if x <= zone {
        strength = strength.max(ramp(x));
        side = 1.0;
    } else if x >= width - zone {
        strength = strength.max(ramp(width - x));
        side = -1.0;
    }
    if options.horiz_mouse_movement == HorizMouseMovement::Ignored {
        side = 0.0;
    }
    if options.reverse_axis && edge.reverse_axis {
        side = -side;
    }
    axes.forward = forward * strength;
    axes.left = side * strength;
    axes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelEffect {
    None,
    Hold,
    ZoomPreset(bool),
    CyclePresets { up: bool },
    NotImplemented(MouseWheel),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelState {
    remaining: i32,
}

impl WheelState {
    pub fn notch(&mut self, towards_view: bool, shift: bool, mode: MouseWheel) -> WheelEffect {
        match mode {
            MouseWheel::Ignored => WheelEffect::None,
            MouseWheel::Zoom if shift => {
                self.remaining = 0;
                WheelEffect::ZoomPreset(towards_view)
            }
            MouseWheel::Zoom | MouseWheel::Move => {
                let n = WHEEL_NOTCH_TICKS as i32;
                self.remaining = if towards_view { n } else { -n };
                WheelEffect::Hold
            }
            MouseWheel::CyclePresets => WheelEffect::CyclePresets { up: towards_view },
            other => WheelEffect::NotImplemented(other),
        }
    }

    pub fn tick(&mut self, mode: MouseWheel) -> ObserverAxes {
        let mut axes = ObserverAxes::default();
        let dir = self.remaining.signum() as f32;
        match mode {
            MouseWheel::Zoom => axes.zoom_in = dir,
            MouseWheel::Move => axes.forward = dir,
            _ => {}
        }
        self.remaining -= self.remaining.signum();
        axes
    }

    pub fn clear(&mut self) {
        self.remaining = 0;
    }
}

pub fn sum_axes(parts: &[ObserverAxes]) -> ObserverAxes {
    parts
        .iter()
        .fold(ObserverAxes::default(), |a, b| ObserverAxes {
            forward: a.forward + b.forward,
            left: a.left + b.left,
            turn_left: a.turn_left + b.turn_left,
            orbit_left: a.orbit_left + b.orbit_left,
            zoom_in: a.zoom_in + b.zoom_in,
        })
}

#[derive(Clone, Debug, Default)]
pub struct NotImplementedLog {
    seen: BTreeSet<String>,
}

impl NotImplementedLog {
    pub fn report(&mut self, what: &str) -> bool {
        if self.seen.insert(what.to_string()) {
            tracing::info!("{what} not implemented yet");
            true
        } else {
            false
        }
    }
}

#[path = "user_dirs.rs"]
pub mod user_dirs;

mod native {
    use super::{Controls, Diagnostic};
    use std::path::{Path, PathBuf};

    pub fn load_native(
        conf: Option<&Path>,
        headless: bool,
    ) -> (Controls, Option<PathBuf>, Vec<Diagnostic>) {
        load_file(crate::conf::to_read(conf, headless))
    }

    pub fn load(conf: Option<&Path>, headless: bool) -> (Controls, Option<PathBuf>) {
        let (controls, path, diagnostics) = load_native(conf, headless);
        for d in &diagnostics {
            eprintln!("controls: {d}");
        }
        (controls, path)
    }

    pub(super) fn load_file(path: Option<PathBuf>) -> (Controls, Option<PathBuf>, Vec<Diagnostic>) {
        crate::conf::load_table(
            path,
            &["controls"],
            Controls::defaults,
            Controls::with_override,
        )
    }
}

pub use native::load;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_files_controls_table_is_read() {
        let dir = std::env::temp_dir().join(format!("m2-controls-conf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mandrake.toml");
        std::fs::write(
            &path,
            "[display]\nui_zoom = 150\n\n[controls.bindings]\nmove_forward = \"Up Arrow\"\nnot_an_action = \"W\"\n",
        )
        .unwrap();
        let (controls, used, diags) = native::load_file(Some(path.clone()));
        assert_eq!(used.as_deref(), Some(path.as_path()));
        assert_eq!(
            controls.bindings(ControlAction::MoveForward),
            &[Binding::parse("Up Arrow").unwrap()]
        );
        assert!(diags.iter().any(|d| d.line == Some(6)), "{diags:?}");
        std::fs::remove_dir_all(&dir).unwrap();
        let (controls, used, diags) = native::load_file(Some(path));
        assert_eq!((controls, used), (Controls::defaults(), None));
        assert!(diags[0].message.contains("cannot read"), "{diags:?}");
    }

    fn held(keys: &[&str]) -> HeldKeys {
        let mut h = HeldKeys::default();
        for k in keys {
            h.press(MythKey::from_name(k).unwrap());
        }
        h
    }

    #[test]
    fn test_defaults_match_configure_controls_dialog() {
        let c = Controls::defaults();
        let table: &[(ControlAction, &str)] = &[
            (ControlAction::MoveForward, "W"),
            (ControlAction::MoveBackward, "S"),
            (ControlAction::MoveLeft, "Z"),
            (ControlAction::MoveRight, "X"),
            (ControlAction::TurnLeft, "A"),
            (ControlAction::TurnRight, "D"),
            (ControlAction::OrbitLeft, "Q"),
            (ControlAction::OrbitRight, "E"),
            (ControlAction::ZoomIn, "C"),
            (ControlAction::ZoomOut, "V"),
            (ControlAction::ClearSelection, "Backquote"),
            (ControlAction::RotatePreset, "F"),
            (ControlAction::StopUnits, "Space"),
            (ControlAction::SelectAll, "Return"),
            (ControlAction::ClearPreset, "Delete"),
            (ControlAction::RecallSetPreset, "Alt/Command"),
            (ControlAction::CenterSelection, "H"),
            (ControlAction::ScatterUnits, "B"),
            (ControlAction::RetreatUnits, "R"),
            (ControlAction::AttackGround, "Control"),
            (ControlAction::SpecialAbility, "T"),
            (ControlAction::RenameUnits, "N"),
            (ControlAction::ChatMessage, "Y"),
            (ControlAction::DetachUnits, "Backslash"),
            (ControlAction::GuardUnits, "G"),
            (ControlAction::OverheadMap, "Tab"),
            (ControlAction::PauseGame, "Escape"),
            (ControlAction::DisplayChat, "Up Arrow"),
            (ControlAction::Taunt, "U"),
            (ControlAction::RotateUnitsRight, "Right Arrow"),
            (ControlAction::RotateUnitsLeft, "Left Arrow"),
            (ControlAction::Inventory, "I"),
            (ControlAction::PresetSelection1, "1"),
            (ControlAction::PresetSelection0, "0"),
            (ControlAction::Quit, "Ctrl+Q"),
        ];
        for &(action, key) in table {
            let names: Vec<String> = c.bindings(action).iter().map(|b| b.to_string()).collect();
            assert_eq!(names, vec![key.to_string()], "{}", action.config_name());
        }
        for &a in ControlAction::ALL {
            assert!(!c.bindings(a).is_empty(), "{} unbound", a.config_name());
        }
        let o = c.options;
        assert_eq!(o.right_click, RightClick::GestureMove);
        assert_eq!(o.mouse_wheel, MouseWheel::Zoom);
        assert_eq!(o.caps_lock, CapsLock::SelectionBoxes);
        assert_eq!(o.horiz_mouse_movement, HorizMouseMovement::Move);
        assert!(o.enable_mouse_movement && o.system_keyboard_layout);
        assert!(!o.top_corners_rotate_camera && !o.bottom_corners_orbit_camera && !o.reverse_axis);
        assert!(c.duplicate_warnings("x").is_empty());
    }

    #[test]
    fn test_key_codes_match_engine_table() {
        assert_eq!(MythKey::from_name("W"), Some(MythKey(0x20)));
        assert_eq!(MythKey::from_name("A"), Some(MythKey(0x2d)));
        assert_eq!(MythKey::from_name("Q"), Some(MythKey(0x1f)));
        assert_eq!(MythKey::from_name("Space"), Some(MythKey(0x47)));
        assert_eq!(MythKey::from_name("Escape"), Some(MythKey(0x00)));
        assert_eq!(MythKey::from_name("up arrow"), Some(MythKey(0x48)));
        assert_eq!(MythKey::from_name("Keypad 8"), Some(MythKey(0x5e)));
        assert_eq!(MythKey::from_name("Ctrl"), Some(MythKey::CONTROL));
        assert_eq!(MythKey::from_name("Nope"), None);
        assert_eq!(MythKey(0x63).name(), "Keypad Period");
    }

    #[test]
    fn test_left_handed_preset_and_partial_override() {
        let lh = Controls::preset("left_handed").unwrap();
        assert_eq!(
            lh.bindings(ControlAction::MoveForward)[0].to_string(),
            "Keypad 8"
        );
        assert_eq!(
            lh.bindings(ControlAction::ZoomOut)[0].to_string(),
            "Keypad Minus"
        );
        assert_eq!(
            lh.bindings(ControlAction::SelectAll)[0].to_string(),
            "Return"
        );

        let (c, diags) = Controls::with_override(
            "preset = \"left_handed\"\n[bindings]\nturn_left = \"J\"\n[options]\nreverse_axis = true\n",
            "user.toml",
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(c.bindings(ControlAction::TurnLeft)[0].to_string(), "J");
        assert_eq!(
            c.bindings(ControlAction::MoveForward)[0].to_string(),
            "Keypad 8"
        );
        assert!(c.options.reverse_axis);
        assert_eq!(c.options.mouse_wheel, MouseWheel::Zoom);
    }

    #[test]
    fn test_multiple_bindings_and_unbinding() {
        let (c, diags) = Controls::with_override(
            "[bindings]\nmove_forward = [\"W\", \"Up Arrow\"]\ndisplay_chat = []\nzoom_out = []\n",
            "user.toml",
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(c.bindings(ControlAction::MoveForward).len(), 2);
        assert!(c.bindings(ControlAction::DisplayChat).is_empty());
        assert!(c.is_held(ControlAction::MoveForward, &held(&["Up Arrow"])));
        assert!(!c.is_held(ControlAction::ZoomOut, &held(&["V"])));
    }

    #[test]
    fn test_screenshot_is_bound_and_rebinds() {
        let shots = |c: &Controls, keys: &[&str], key: &str| {
            c.actions_for_press(MythKey::from_name(key).unwrap(), &held(keys))
                .contains(&ControlAction::Screenshot)
        };
        let c = Controls::defaults();
        assert!(shots(&c, &["F13"], "F13"));
        assert!(shots(&c, &["Option", "F12"], "F12"));
        assert!(shots(&c, &["Alt/Command", "F12"], "F12"));
        assert!(!shots(&c, &["F12"], "F12"));

        let (c, diags) =
            Controls::with_override("[bindings]\nscreenshot = \"Ctrl+P\"\n", "user.toml");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(shots(&c, &["Control", "P"], "P"));
        assert!(!shots(&c, &["P"], "P"));
        assert!(!shots(&c, &["F13"], "F13"));
        assert!(!shots(&c, &["Option", "F12"], "F12"));

        let (c, diags) = Controls::with_override("[bindings]\nscreenshot = []\n", "user.toml");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(c.bindings(ControlAction::Screenshot).is_empty());
    }

    #[test]
    fn test_fps_display_is_bound_and_rebinds() {
        let toggles = |c: &Controls, keys: &[&str], key: &str| {
            c.actions_for_press(MythKey::from_name(key).unwrap(), &held(keys))
                .contains(&ControlAction::ToggleFpsDisplay)
        };
        let c = Controls::defaults();
        assert!(toggles(&c, &["Forward Delete"], "Forward Delete"));
        assert!(toggles(&c, &["Shift", "Forward Delete"], "Forward Delete"));
        assert!(toggles(&c, &["Shift", "Delete"], "Delete"));
        assert!(toggles(&c, &["Shift", "Backspace"], "Backspace"));
        assert!(!toggles(&c, &["Delete"], "Delete"));
        assert_eq!(
            c.actions_for_press(MythKey::from_name("Delete").unwrap(), &held(&["Delete"])),
            vec![ControlAction::ClearPreset]
        );
        assert_eq!(ControlAction::ToggleFpsDisplay.engine_id(), None);

        let (c, diags) =
            Controls::with_override("[bindings]\ntoggle_fps_display = \"P\"\n", "user.toml");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(toggles(&c, &["P"], "P"));
        assert!(!toggles(&c, &["Forward Delete"], "Forward Delete"));
        assert!(!toggles(&c, &["Shift", "Delete"], "Delete"));

        let (c, diags) =
            Controls::with_override("[bindings]\ntoggle_fps_display = []\n", "user.toml");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(c.bindings(ControlAction::ToggleFpsDisplay).is_empty());
    }

    #[test]
    fn test_quicksave_is_bound_and_rebinds() {
        let key = |name| MythKey::from_name(name).unwrap();
        for c in [
            Controls::defaults(),
            Controls::preset("left_handed").unwrap(),
        ] {
            assert_eq!(
                c.actions_for_press(key("F5"), &held(&["F5"])),
                vec![ControlAction::Quicksave]
            );
            assert_eq!(
                c.actions_for_press(key("F12"), &held(&["F12"])),
                vec![ControlAction::Quicksave]
            );
            assert_eq!(
                c.actions_for_press(key("F12"), &held(&["Alt/Command", "F12"])),
                vec![ControlAction::Screenshot]
            );
        }
        assert_eq!(ControlAction::Quicksave.engine_id(), None);
        let (c, diags) = Controls::with_override("[bindings]\nquicksave = \"F9\"\n", "user.toml");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(
            c.actions_for_press(key("F9"), &held(&["F9"])),
            vec![ControlAction::Quicksave]
        );
        assert!(c.actions_for_press(key("F5"), &held(&["F5"])).is_empty());
    }

    #[test]
    fn test_control_bar_toggle_is_bound_and_rebinds() {
        let key = |name| MythKey::from_name(name).unwrap();
        for c in [
            Controls::defaults(),
            Controls::preset("left_handed").unwrap(),
        ] {
            assert_eq!(
                c.bindings(ControlAction::ToggleControlBar)[0].to_string(),
                "Shift+F6"
            );
            assert_eq!(
                c.actions_for_press(key("F6"), &held(&["Shift", "F6"])),
                vec![ControlAction::ToggleControlBar]
            );
            assert!(c.actions_for_press(key("F6"), &held(&["F6"])).is_empty());
        }
        assert_eq!(ControlAction::ToggleControlBar.engine_id(), None);
        let (c, diags) =
            Controls::with_override("[bindings]\ntoggle_control_bar = \"F9\"\n", "user.toml");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(
            c.actions_for_press(key("F9"), &held(&["F9"])),
            vec![ControlAction::ToggleControlBar]
        );
        assert!(
            c.actions_for_press(key("F6"), &held(&["Shift", "F6"]))
                .is_empty()
        );
    }

    #[test]
    fn test_score_keys_are_bound_and_rebind() {
        let key = |name| MythKey::from_name(name).unwrap();
        for c in [
            Controls::defaults(),
            Controls::preset("left_handed").unwrap(),
        ] {
            assert!(c.is_held(ControlAction::ShowScores, &held(&["F7"])));
            assert!(c.is_held(ControlAction::ShowScores, &held(&["Shift", "F7"])));
            assert!(!c.is_held(ControlAction::ShowScores, &held(&["F6"])));
            assert_eq!(
                c.actions_for_press(key("F7"), &held(&["Shift", "F7"])),
                vec![ControlAction::ToggleScores]
            );
            assert!(
                !c.actions_for_press(key("F7"), &held(&["F7"]))
                    .contains(&ControlAction::ToggleScores)
            );
        }
        assert!(ControlAction::ShowScores.is_held_only());
        assert_eq!(ControlAction::ShowScores.engine_id(), None);
        assert_eq!(ControlAction::ToggleScores.engine_id(), None);
        let (c, diags) = Controls::with_override(
            "[bindings]\nshow_scores = \"F9\"\ntoggle_scores = \"Ctrl+F9\"\n",
            "user.toml",
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert!(c.is_held(ControlAction::ShowScores, &held(&["F9"])));
        assert!(!c.is_held(ControlAction::ShowScores, &held(&["F7"])));
        assert_eq!(
            c.actions_for_press(key("F9"), &held(&["Control", "F9"])),
            vec![ControlAction::ToggleScores]
        );
    }

    #[test]
    fn test_key_bindings_key_is_held_and_rebinds() {
        for c in [
            Controls::defaults(),
            Controls::preset("left_handed").unwrap(),
            Controls::preset("tfl_right_handed").unwrap(),
        ] {
            assert!(c.is_held(ControlAction::ShowKeyBindings, &held(&["F8"])));
            assert!(!c.is_held(ControlAction::ShowKeyBindings, &held(&["F7"])));
        }
        assert!(ControlAction::ShowKeyBindings.is_held_only());
        let (c, diags) =
            Controls::with_override("[bindings]\nshow_key_bindings = \"F9\"\n", "user.toml");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(c.is_held(ControlAction::ShowKeyBindings, &held(&["F9"])));
        assert!(!c.is_held(ControlAction::ShowKeyBindings, &held(&["F8"])));
    }

    #[test]
    fn test_fullscreen_toggle_is_bound_and_rebinds() {
        let key = |name| MythKey::from_name(name).unwrap();
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        for c in [
            Controls::defaults(),
            Controls::preset("left_handed").unwrap(),
        ] {
            assert_eq!(
                c.bindings(ControlAction::ToggleFullscreen)[0].to_string(),
                "Alt+F11"
            );
            assert!(c.toggles_fullscreen(key("F11"), alt));
            assert!(!c.toggles_fullscreen(key("F11"), Modifiers::default()));
            assert!(!c.toggles_fullscreen(key("Return"), alt));
            assert_eq!(
                c.actions_for_press(key("Return"), &held(&["Alt/Command", "Return"])),
                vec![ControlAction::SelectAll]
            );
        }
        assert_eq!(ControlAction::ToggleFullscreen.engine_id(), None);

        let (c, diags) = Controls::with_override(
            "[bindings]\ntoggle_fullscreen = \"Alt+Return\"\n",
            "user.toml",
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert!(c.toggles_fullscreen(key("Return"), alt));
        assert!(!c.toggles_fullscreen(key("Return"), Modifiers::default()));
        assert!(!c.toggles_fullscreen(key("F11"), alt));
        assert_eq!(
            c.actions_for_press(key("Return"), &held(&["Option", "Return"])),
            vec![ControlAction::ToggleFullscreen]
        );
        assert_eq!(
            c.actions_for_press(key("Return"), &held(&["Return"])),
            vec![ControlAction::SelectAll]
        );
    }

    #[test]
    fn test_errors_are_located_and_fall_back() {
        let text = "[bindings]\nmove_forward = \"Wx\"\nfly = \"F\"\nturn_left = \"J\"\n\n[options]\nmouse_wheel = \"spin\"\nreverse_axis = 3\nbogus = true\n";
        let (c, diags) = Controls::with_override(text, "user.toml");
        let lines: Vec<(Option<usize>, &str)> =
            diags.iter().map(|d| (d.line, d.message.as_str())).collect();
        assert_eq!(diags.len(), 5, "{lines:?}");
        assert!(
            diags[0].to_string().starts_with("user.toml:2: "),
            "{}",
            diags[0]
        );
        assert!(diags[0].message.contains("unknown key name"));
        assert_eq!(diags[1].line, Some(3));
        assert!(diags[1].message.contains("unknown action `fly`"));
        assert_eq!(diags[2].line, Some(7));
        assert!(diags[2].message.contains("spin"));
        assert_eq!(diags[3].line, Some(8));
        assert_eq!(diags[4].line, Some(9));
        assert_eq!(c.bindings(ControlAction::MoveForward)[0].to_string(), "W");
        assert_eq!(c.bindings(ControlAction::TurnLeft)[0].to_string(), "J");
        assert_eq!(c.options.mouse_wheel, MouseWheel::Zoom);
        assert!(!c.options.reverse_axis);

        let (c, diags) = Controls::with_override("[bindings\nx=", "broken.toml");
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .to_string()
                .starts_with("broken.toml:1: TOML syntax error")
        );
        assert_eq!(c, Controls::defaults());

        let (_, diags) = Controls::with_override("preset = \"ambidextrous\"\n", "p.toml");
        assert_eq!(diags[0].line, Some(1));
    }

    #[test]
    fn test_duplicate_bindings_are_warned() {
        let (_, diags) = Controls::with_override("[bindings]\nturn_left = \"Q\"\n", "user.toml");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("orbit_left") && diags[0].message.contains("turn_left"));
    }

    #[test]
    fn test_binding_parse_and_modifiers() {
        assert_eq!(Binding::parse("Ctrl+Q").unwrap().to_string(), "Ctrl+Q");
        assert_eq!(Binding::parse("shift + c").unwrap().to_string(), "Shift+C");
        assert_eq!(Binding::parse("Ctrl++").unwrap().to_string(), "Ctrl+Plus");
        assert!(Binding::parse("Hyper+Q").is_err());
        assert!(Binding::parse("").is_err());

        let c = Controls::defaults();
        let q = MythKey::from_name("Q").unwrap();
        assert_eq!(
            c.actions_for_press(q, &held(&["Control", "Q"])),
            vec![ControlAction::Quit]
        );
        assert!(
            c.actions_for_press(q, &held(&["Q"]))
                .contains(&ControlAction::OrbitLeft)
        );
        assert!(!c.is_held(ControlAction::Quit, &held(&["Q"])));
        assert!(c.is_held(ControlAction::ZoomIn, &held(&["Shift", "C"])));
        assert!(c.is_held(ControlAction::AttackGround, &held(&["Control"])));
    }

    #[test]
    fn test_held_keys_to_camera_axes() {
        let c = Controls::defaults();
        let axes = |keys: &[&str]| c.keyboard_axes(&held(keys));
        assert_eq!(axes(&["A"]).turn_left, 1.0);
        assert_eq!(axes(&["D"]).turn_left, -1.0);
        assert_eq!(axes(&["Q"]).orbit_left, 1.0);
        assert_eq!(axes(&["E"]).orbit_left, -1.0);
        assert_eq!(axes(&["Z"]).left, 1.0);
        assert_eq!(axes(&["X"]).left, -1.0);
        assert_eq!(axes(&["W"]).forward, 1.0);
        assert_eq!(axes(&["S"]).forward, -1.0);
        assert_eq!(axes(&["C"]).zoom_in, 1.0);
        assert_eq!(axes(&["V"]).zoom_in, -1.0);
        assert_eq!(axes(&["W", "S"]).forward, 0.0);
        let a = axes(&["A"]);
        assert_eq!(
            (a.forward, a.left, a.orbit_left, a.zoom_in),
            (0.0, 0.0, 0.0, 0.0)
        );
        assert_eq!(axes(&["Up Arrow", "Left Arrow"]), ObserverAxes::default());
    }

    #[test]
    fn test_wheel_modes() {
        let mut w = WheelState::default();
        assert_eq!(w.notch(true, false, MouseWheel::Zoom), WheelEffect::Hold);
        for _ in 0..WHEEL_NOTCH_TICKS {
            assert_eq!(w.tick(MouseWheel::Zoom).zoom_in, 1.0);
        }
        assert_eq!(w.tick(MouseWheel::Zoom).zoom_in, 0.0);
        w.notch(false, false, MouseWheel::Zoom);
        assert_eq!(w.tick(MouseWheel::Zoom).zoom_in, -1.0);
        assert_eq!(
            w.notch(true, true, MouseWheel::Zoom),
            WheelEffect::ZoomPreset(true)
        );
        assert_eq!(w.tick(MouseWheel::Zoom), ObserverAxes::default());
        w.notch(true, false, MouseWheel::Move);
        let a = w.tick(MouseWheel::Move);
        assert_eq!((a.forward, a.zoom_in), (1.0, 0.0));
        assert_eq!(w.notch(true, false, MouseWheel::Ignored), WheelEffect::None);
        assert_eq!(
            w.notch(true, false, MouseWheel::CyclePresets),
            WheelEffect::CyclePresets { up: true }
        );
        assert_eq!(
            w.notch(false, true, MouseWheel::CyclePresets),
            WheelEffect::CyclePresets { up: false }
        );
        assert_eq!(
            w.notch(true, false, MouseWheel::CycleInventory),
            WheelEffect::NotImplemented(MouseWheel::CycleInventory)
        );
    }

    #[test]
    fn test_edge_scroll_zones() {
        let o = ControlOptions::default();
        let (w, h) = (800.0, 600.0);
        assert_eq!(
            edge_scroll_axes(400.0, 300.0, w, h, &o),
            ObserverAxes::default()
        );
        assert_eq!(
            edge_scroll_axes(6.0, 300.0, w, h, &o),
            ObserverAxes::default()
        );
        let top = edge_scroll_axes(400.0, 0.0, w, h, &o);
        assert_eq!((top.forward, top.left), (1.0, 0.0));
        let bottom = edge_scroll_axes(400.0, 597.5, w, h, &o);
        assert_eq!(bottom.forward, -0.5);
        let left = edge_scroll_axes(1.0, 300.0, w, h, &o);
        assert!((left.left - 0.8).abs() < 1e-6 && left.forward == 0.0);
        let right = edge_scroll_axes(799.0, 300.0, w, h, &o);
        assert!((right.left + 0.8).abs() < 1e-6);
        let corner = edge_scroll_axes(0.0, 4.0, w, h, &o);
        assert_eq!((corner.forward, corner.left), (1.0, 1.0));
        let rev = ControlOptions {
            reverse_axis: true,
            ..o
        };
        let r = edge_scroll_axes(0.0, 0.0, w, h, &rev);
        assert_eq!((r.forward, r.left), (1.0, -1.0));
        let ign = ControlOptions {
            horiz_mouse_movement: HorizMouseMovement::Ignored,
            ..o
        };
        assert_eq!(edge_scroll_axes(0.0, 300.0, w, h, &ign).left, 0.0);
        let off = ControlOptions {
            enable_mouse_movement: false,
            ..o
        };
        assert_eq!(
            edge_scroll_axes(0.0, 0.0, w, h, &off),
            ObserverAxes::default()
        );
    }

    #[test]
    fn test_sum_axes() {
        let s = sum_axes(&[
            ObserverAxes {
                forward: 1.0,
                ..Default::default()
            },
            ObserverAxes {
                forward: 0.5,
                zoom_in: -1.0,
                ..Default::default()
            },
        ]);
        assert_eq!((s.forward, s.zoom_in), (1.5, -1.0));
        assert_eq!(s.clamped().forward, 1.0);
    }

    #[test]
    fn test_not_implemented_logs_once() {
        let mut log = NotImplementedLog::default();
        assert!(log.report("action scatter_units"));
        assert!(!log.report("action scatter_units"));
        assert!(log.report("action guard_units"));
    }

    #[test]
    fn binding_parse_handles_utf8_and_trailing_plus() {
        let plus = MythKey::from_name("Plus").unwrap();
        assert_eq!(
            Binding::parse("+").unwrap(),
            Binding {
                key: plus,
                modifiers: Modifiers::default(),
            }
        );
        assert_eq!(
            Binding::parse("Ctrl++").unwrap(),
            Binding {
                key: plus,
                modifiers: Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                },
            }
        );
        assert!(Binding::parse("é+").is_err());
        assert!(Binding::parse("⌘+").is_err());
        assert!(Binding::parse("a+").is_err());
        assert!(Binding::parse("Ctrl+").is_err());
    }
}
