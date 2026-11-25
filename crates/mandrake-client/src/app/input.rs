use crate::app::ClientApp;
use crate::app::shell::{WindowFacts, binding_modifiers};
use crate::clock::Instant;
use crate::controls::{ControlAction, MythKey, WheelEffect};
use crate::interface_setup::world_frame;
use crate::overlays::MoveGesture;
use crate::picking::{
    PickWorld, drawn_units_in_rect, hover_pick, ray_terrain_intersection, unproject_cursor,
};
use crate::region_view::{VIEW_BARS, ViewBars, view_bars};
use crate::{
    command_bar, controls, hud, media, move_marker, selection_input, selection_rules, ui_scale,
};
use mandrake_view::selection;
use tracing::{debug, info};
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

const PIXELS_PER_WHEEL_NOTCH: f64 = 40.0;

#[derive(Default)]
pub(crate) struct WheelNotches {
    pixels: f64,
}

impl WheelNotches {
    pub(crate) fn clear(&mut self) {
        self.pixels = 0.0;
    }

    pub(crate) fn push(&mut self, delta: MouseScrollDelta) -> i32 {
        match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                self.pixels = 0.0;
                if y > 0.0 {
                    1
                } else if y < 0.0 {
                    -1
                } else {
                    0
                }
            }
            MouseScrollDelta::PixelDelta(pos) => {
                if pos.y == 0.0 {
                    return 0;
                }
                if self.pixels != 0.0 && pos.y.signum() != self.pixels.signum() {
                    self.pixels = 0.0;
                }
                self.pixels += pos.y;
                let notches = (self.pixels / PIXELS_PER_WHEEL_NOTCH).trunc() as i32;
                self.pixels -= f64::from(notches) * PIXELS_PER_WHEEL_NOTCH;
                notches
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogAsk {
    Pause,
    Hints,
    Quicksave,
}

pub(crate) fn right_click_mode(option: controls::RightClick) -> selection_input::RightClickMode {
    use selection_input::RightClickMode;
    match option {
        controls::RightClick::Ignored => RightClickMode::Ignored,
        controls::RightClick::GestureMove => RightClickMode::GestureMove,
        controls::RightClick::AttackGround => RightClickMode::AttackGround,
        controls::RightClick::DeselectUnits => RightClickMode::DeselectUnits,
    }
}

const UP_ARROW: MythKey = MythKey(0x48);

pub(crate) fn myth_key(code: KeyCode) -> Option<MythKey> {
    use KeyCode::*;
    let name = match code {
        Escape => "Escape",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        F13 => "F13",
        F14 => "F14",
        F15 => "F15",
        Backquote => "Backquote",
        Digit1 => "1",
        Digit2 => "2",
        Digit3 => "3",
        Digit4 => "4",
        Digit5 => "5",
        Digit6 => "6",
        Digit7 => "7",
        Digit8 => "8",
        Digit9 => "9",
        Digit0 => "0",
        Minus => "Minus",
        Equal => "Plus",
        Backspace => "Delete",
        Tab => "Tab",
        KeyQ => "Q",
        KeyW => "W",
        KeyE => "E",
        KeyR => "R",
        KeyT => "T",
        KeyY => "Y",
        KeyU => "U",
        KeyI => "I",
        KeyO => "O",
        KeyP => "P",
        BracketLeft => "Left Brace",
        BracketRight => "Right Brace",
        Backslash => "Backslash",
        CapsLock => "Caps Lock",
        KeyA => "A",
        KeyS => "S",
        KeyD => "D",
        KeyF => "F",
        KeyG => "G",
        KeyH => "H",
        KeyJ => "J",
        KeyK => "K",
        KeyL => "L",
        Semicolon => "Semicolon",
        Quote => "Quote",
        Enter => "Return",
        ShiftLeft | ShiftRight => "Shift",
        KeyZ => "Z",
        KeyX => "X",
        KeyC => "C",
        KeyV => "V",
        KeyB => "B",
        KeyN => "N",
        KeyM => "M",
        Comma => "Less Than",
        Period => "Greater Than",
        Slash => "Question Mark",
        ControlLeft | ControlRight => "Control",
        AltLeft | AltRight | SuperLeft | SuperRight => "Alt/Command",
        Space => "Space",
        ArrowUp => "Up Arrow",
        ArrowDown => "Down Arrow",
        ArrowLeft => "Left Arrow",
        ArrowRight => "Right Arrow",
        Insert | Help => "Help",
        Home => "Home",
        PageUp => "Page Up",
        Delete => "Forward Delete",
        End => "End",
        PageDown => "Page Down",
        NumLock => "Keypad Clear",
        NumpadEqual => "Keypad Equal",
        NumpadDivide => "Keypad Slash",
        NumpadMultiply => "Keypad Asterisk",
        Numpad0 => "Keypad 0",
        Numpad1 => "Keypad 1",
        Numpad2 => "Keypad 2",
        Numpad3 => "Keypad 3",
        Numpad4 => "Keypad 4",
        Numpad5 => "Keypad 5",
        Numpad6 => "Keypad 6",
        Numpad7 => "Keypad 7",
        Numpad8 => "Keypad 8",
        Numpad9 => "Keypad 9",
        NumpadSubtract => "Keypad Minus",
        NumpadAdd => "Keypad Plus",
        NumpadEnter => "Keypad Enter",
        NumpadDecimal => "Keypad Period",
        _ => return None,
    };
    MythKey::from_name(name)
}

impl ClientApp {
    pub(crate) fn cursor_ground_point(&self) -> Option<[f32; 3]> {
        let (cx, cy) = self.current_cursor_pos;
        let media = self
            .gpu_state
            .as_ref()
            .and_then(|gpu| gpu.sprite_mgr.scenery_objects.media.as_deref());
        if media.is_some() {
            use selection_input::SelectionWorld;
            let (x, y) = PickWorld::myth2(
                &self.sim,
                &self.mesh_tag,
                &self.render_camera,
                (self.width, self.height),
                None,
                Vec::new(),
                media,
                (
                    view_bars(&self.ui),
                    media::local_time(self.sim.current_tick),
                ),
            )
            .gesture_point((cx as i16, cy as i16))?;
            let p = self.sim.command_point_to_world(x, y);
            let z = self
                .mesh_tag
                .elevation_at((x >> 9).max(0) as usize, (y >> 9).max(0) as usize)
                .unwrap_or(0);
            return Some([p.x.to_f32(), f32::from(z) / 512.0, p.z.to_f32()]);
        }
        let (ray_orig, ray_dir) =
            unproject_cursor(cx, cy, self.width, self.height, &self.render_camera);
        ray_terrain_intersection(ray_orig, ray_dir, &self.mesh_tag)
    }

    pub(crate) fn poll_action(&mut self) -> selection_input::InputAction {
        let (left, right) = std::mem::take(&mut self.pressed_since_poll);
        let clamp = |v: f32, size: u32| v.clamp(0.0, size.saturating_sub(1) as f32) as i16;
        let keys = &self.held_keys;
        let poll = selection_input::PollState {
            cursor: (
                clamp(self.current_cursor_pos.0, self.width),
                clamp(self.current_cursor_pos.1, self.height),
            ),
            left: self.mouse_left_down || left,
            right: self.mouse_right_down || right,
            alt: keys.is_down(MythKey::ALT_COMMAND),
            option: keys.is_down(MythKey::OPTION),
            control: keys.is_down(MythKey::CONTROL),
            shift: keys.is_down(MythKey::SHIFT),
            clear_selection: self.controls.is_held(ControlAction::ClearSelection, keys),
            attack_ground: self.controls.is_held(ControlAction::AttackGround, keys),
        };
        self.selection_actions.poll(poll)
    }

    pub(crate) fn step_selection(&mut self, action: selection_input::InputAction) {
        let now_ms = self.machine_clock.elapsed().as_millis() as u32;
        let pointer_active = self.cursor_in_window && self.focused;
        let sprites = self
            .gpu_state
            .as_ref()
            .map(|gpu| (&gpu.sprite_mgr, &self.unit_animator));
        let (sim, camera, viewport) = (&self.sim, &self.render_camera, (self.width, self.height));
        let cursor = self.current_cursor_pos;
        self.hover.update(now_ms, pointer_active, || {
            hover_pick(sim, camera, viewport, cursor, sprites)
        });
        self.interface.targets.update(action.elapsed);
        self.interface.order_hide.update(action.elapsed);
        let replay = self.sim.engine.mode == 0;
        let step = self.selection_machine.attack_ground_right_press(
            &action,
            &self.selected_unit_ids,
            replay,
        );
        self.attack_ground_step(step);
        let mut selection = self.selected_unit_ids.clone();
        let mut world = PickWorld::myth2(
            &self.sim,
            &self.mesh_tag,
            &self.render_camera,
            (self.width, self.height),
            self.gpu_state
                .as_ref()
                .map(|gpu| (&gpu.sprite_mgr, &self.unit_animator)),
            selection.clone(),
            self.gpu_state
                .as_ref()
                .and_then(|gpu| gpu.sprite_mgr.scenery_objects.media.as_deref()),
            (
                view_bars(&self.ui),
                media::local_time(self.sim.current_tick),
            ),
        );
        let selection_mode = self.selection_mode();
        let machine = &mut self.selection_machine;
        machine.left_drag_bands = self.controls.options.left_click_doesnt_gesture;
        machine.context = selection_input::PressContext {
            game_type: (self.sim.engine.game_type),
            game_options_high: self.sim.game_options_high,
            selection_mode,
        };
        let before = machine.state();
        let (left, right) = if before == selection_input::State::Idle {
            (
                selection_input::LEFT_PRESSED,
                selection_input::RIGHT_PRESSED,
            )
        } else {
            (selection_input::LEFT_HELD, selection_input::RIGHT_HELD)
        };
        let detach_exit = machine.context.selection_mode == super::detach::SELECTION_MODE_DETACH
            && (action.flags.test(left) || action.flags.test(right));
        let outcome = if detach_exit {
            selection_input::Outcome::None
        } else {
            machine.update(&action, &mut world, &mut selection)
        };
        let after = machine.state();
        let f = action.flags;
        let mouse_busy = if before == selection_input::State::Idle {
            f.test(selection_input::LEFT_PRESSED) || f.test(selection_input::RIGHT_PRESSED)
        } else {
            f.test(selection_input::LEFT_HELD) || f.test(selection_input::RIGHT_HELD)
        };
        let replaced =
            machine.looking() || matches!(outcome, selection_input::Outcome::SelectOnly(_));
        if replaced && let Some(h) = self.hud.as_mut() {
            h.console.clear();
        }
        if detach_exit {
            self.detach_mouse_exit();
        }
        if after != before {
            debug!(
                "selection state {before:?} -> {after:?} at t {} cursor {:?} band {:?}",
                action.time,
                action.cursor,
                self.selection_machine.band_bounds()
            );
        }
        if selection != self.selected_unit_ids {
            self.set_selection(selection);
        }
        self.selection_outcome(outcome, &action);
        let keys_run = !mouse_busy && !f.test(selection_input::CLEAR_SELECTION);
        if keys_run && self.selection_mode() == command_bar::SELECTION_MODE_ATTACK_GROUND {
            let step = self.selection_machine.attack_ground_countdown(&action);
            self.attack_ground_step(step);
        }
        let poll = crate::local_input::PresetPoll {
            mouse_busy,
            ..self.preset_poll(i32::from(action.elapsed))
        };
        let effects = self.selection_machine.presets.poll(poll);
        self.preset_effects(effects);
        let polled = keys_run && action != selection_input::InputAction::default();
        self.wheel_poll(polled);
        if polled
            && !self.preset_mode_skips_keys()
            && self
                .controls
                .is_held(ControlAction::CenterSelection, &self.held_keys)
        {
            self.center_selection(false);
        }
        if keys_run {
            let step =
                self.selection_machine
                    .attack_ground_key(&action, &self.selected_unit_ids, replay);
            self.attack_ground_step(step);
        }
        if after != selection_input::State::Gesture {
            self.move_gesture = None;
        } else {
            if self.move_gesture.is_none() {
                self.move_gesture = self.gesture_visual();
            }
            let ground = self.cursor_ground_point();
            if let (Some(g), Some(ground)) = (self.move_gesture.as_mut(), ground) {
                g.drag_to(ground);
            }
        }
    }

    pub(super) fn clear_local_order_marks(&mut self, clear_prompts: bool) {
        let now = media::local_time(self.sim.current_tick);
        self.local_sprites
            .remove_local_sprites(move_marker::RemoveMode::ClearUnlessTimed, now);
        self.sim.local_waypoints.clear();
        if clear_prompts && let Some(h) = self.hud.as_mut() {
            h.console.clear();
        }
    }

    fn attack_ground_step(&mut self, step: selection_input::AttackGroundStep) {
        use selection_input::AttackGroundStep as S;
        if step == S::None {
            return;
        }
        self.clear_local_order_marks(false);
        if let Some(h) = self.hud.as_mut() {
            match step {
                S::Enter => h.attack_ground_mode(),
                S::Exit => h.console.clear(),
                S::None | S::Refused => {}
            }
        }
    }

    fn selection_outcome(
        &mut self,
        outcome: selection_input::Outcome,
        action: &selection_input::InputAction,
    ) {
        use selection_input::Outcome;
        match outcome {
            Outcome::SelectOnly(id) | Outcome::Added(id) => {
                if let Some(u) = self.sim.units.get(&id) {
                    info!(
                        "Selected unit #{} ({}, team {}) at ({:.1}, {:.1})",
                        id,
                        u.archetype,
                        u.team,
                        u.position.x.to_f32(),
                        u.position.z.to_f32(),
                    );
                }
                self.play_unit_voices(&[id], mandrake_tag::MonsterSoundType::SELECTION);
            }
            Outcome::Attack { target } => self.order_attack_on(target, action.flags),
            Outcome::AttackGround { point, special } => self.order_attack_point(point, special),
            Outcome::AttackModel { point, special } => self.order_attack_model(point, special),
            Outcome::AttackAgain {
                attackers,
                target,
                shift,
            } => self.order_attack_again(&attackers, target, shift),
            Outcome::Move {
                point,
                facing,
                add_waypoint,
            } => self.order_ground_move(point, facing, add_waypoint),
            Outcome::UnitOrder { facing } => self.order_unit_click(facing),
            Outcome::PickUp {
                index,
                identifier,
                units,
            } => self.order_pick_up(index, identifier, &units),
            Outcome::Ignored | Outcome::None | Outcome::Released(_) => {}
        }
    }

    fn gesture_visual(&self) -> Option<MoveGesture> {
        let origin = match self.selection_machine.press_pick() {
            selection_input::PressPick::Ground((x, y)) => {
                let p = self.sim.command_point_to_world(x, y);
                let h = self
                    .mesh_tag
                    .terrain_height_at(x as f32 / 512.0, y as f32 / 512.0);
                return Some(MoveGesture {
                    origin: [p.x.to_f32(), h, p.z.to_f32()],
                    on_selection: false,
                    facing: None,
                });
            }
            selection_input::PressPick::Unit(id) => self.sim.units.get(&id)?.position,
            selection_input::PressPick::Other => return None,
        };
        Some(MoveGesture {
            origin: [origin.x.to_f32(), origin.y.to_f32(), origin.z.to_f32()],
            on_selection: true,
            facing: None,
        })
    }

    fn press_key(&mut self, key: MythKey) {
        let shift = self.held_keys.modifiers().shift;
        let actions = self.controls.actions_for_press(key, &self.held_keys);
        self.number_key_event(key, &actions, false);
        for action in actions {
            if action.engine_id().is_some() {
                self.pending_presses.push((action, shift));
            } else if action == ControlAction::Quit {
                self.quit();
            } else {
                self.dispatch_press(action, shift);
            }
        }
    }

    fn repeat_key(&mut self, key: MythKey) {
        let shift = self.held_keys.modifiers().shift;
        let actions = self.controls.actions_for_press(key, &self.held_keys);
        self.number_key_event(key, &actions, true);
        for action in actions {
            if matches!(
                action,
                ControlAction::GameSpeedDown | ControlAction::GameSpeedUp
            ) {
                self.dispatch_press(action, shift);
            }
        }
    }

    fn number_key_event(&mut self, key: MythKey, actions: &[ControlAction], repeat: bool) {
        use selection_input as si;
        let mut flags = si::ActionFlags::default();
        let mods = self.held_keys.modifiers();
        for (held, flag) in [
            (self.mouse_left_down, si::LEFT_HELD),
            (self.mouse_right_down, si::RIGHT_HELD),
            (mods.alt, si::ALT),
            (mods.ctrl, si::CONTROL),
            (mods.shift, si::SHIFT),
            (repeat, 0x2f),
        ] {
            if held {
                flags.set(flag);
            }
        }
        let mut held = [0u16; 64];
        let mut held_len = 0;
        for id in ControlAction::ALL
            .iter()
            .filter(|&&a| self.controls.is_held(a, &self.held_keys))
            .filter_map(|a| a.engine_id())
        {
            if held_len < held.len() {
                held[held_len] = id;
                held_len += 1;
            }
        }
        let record = crate::local_input::engine_action(flags, &held[..held_len]);
        self.selection_machine.presets.key_event(
            key,
            [record.flags[0], record.flags[1]],
            actions.iter().filter_map(|a| a.preset_index()),
        );
    }

    fn toggle_control_bar(&mut self) {
        let Some(shown) = self
            .hud
            .as_mut()
            .and_then(|hud| toggle_control_bar(hud, &VIEW_BARS))
        else {
            return;
        };
        crate::interface_config::remember_control_bar(shown);
        info!("Control bar {}", if shown { "shown" } else { "hidden" });
    }

    pub(super) fn net_coop_host(&self) -> bool {
        let (Some(net), w) = (self.net.as_ref(), &self.sim.engine) else {
            return false;
        };
        crate::game_speed::coop_host_may_send(
            net.is_host(),
            self.sim.game_option_flags(),
            w.formation_globals.gameplay_version,
            w.local_team(),
        )
    }

    pub(super) fn queue_system_command(&mut self, kind: u8, word: i16) {
        let [hi, lo] = word.to_be_bytes();
        let packet = mandrake_rec::CommandPacket::parse(0, 0xff, kind, &[hi, lo, 0, 0]);
        info!(
            "net: the coop host's {} {word} at tick {}",
            if kind == mandrake_sim::net_game::PAUSE {
                "pause"
            } else {
                "game speed"
            },
            self.sim.current_tick
        );
        self.sim
            .local_commands
            .push(mandrake_sim::QueuedCommand::Packet(packet));
    }

    fn adjust_game_speed(&mut self, faster: bool, shift: bool) {
        if self.net.is_some() {
            if !self.net_coop_host() {
                return;
            }
            let Some(index) = self.game_speed.adjust_as_coop_host(faster, shift) else {
                return;
            };
            info!("Game speed: index {index} requested");
            let polls = self.game_speed.polls();
            if let Some(hud) = self.hud.as_mut() {
                hud.show_speed_popup(index, polls);
            }
            self.queue_system_command(mandrake_sim::net_game::GAME_SPEED, index);
            return;
        }
        let speed = &mut self.game_speed;
        let Some(index) = speed.adjust(faster, shift) else {
            return;
        };
        let (value, polls) = (speed.speed(), speed.polls());
        info!("Game speed: {value}/256 (index {index})");
        if let Some(hud) = self.hud.as_mut() {
            hud.show_speed_popup(index, polls);
        }
    }

    pub(crate) fn dispatch_press(&mut self, action: ControlAction, shift: bool) {
        if self.preset_mode_skips_keys()
            && matches!(
                action,
                ControlAction::RotatePreset
                    | ControlAction::ClearPreset
                    | ControlAction::SelectAll
                    | ControlAction::CenterSelection
                    | ControlAction::StopUnits
                    | ControlAction::ScatterUnits
                    | ControlAction::RetreatUnits
                    | ControlAction::SpecialAbility
                    | ControlAction::ChatMessage
                    | ControlAction::DetachUnits
            )
        {
            return;
        }
        match action {
            ControlAction::ZoomIn | ControlAction::ZoomOut
                if shift && !self.camera_control.user_camera_locked() =>
            {
                self.observer
                    .cycle_zoom_preset(action == ControlAction::ZoomIn);
            }
            ControlAction::ZoomIn | ControlAction::ZoomOut => {}
            _ if action.is_held_only() => {}
            ControlAction::Quit | ControlAction::ToggleFullscreen => {}
            ControlAction::Screenshot => self.screenshot_requested = true,
            ControlAction::ToggleFpsDisplay => {
                if let Some(hud) = self.hud.as_mut() {
                    hud.fps.toggle();
                }
            }
            ControlAction::Quicksave if self.session_map => {
                let held = self.held_keys.modifiers();
                if !shift && !held.shift && !held.ctrl {
                    self.dialog_ask = Some(DialogAsk::Quicksave);
                }
            }
            ControlAction::Quicksave => {}
            ControlAction::ToggleControlBar => self.toggle_control_bar(),
            ControlAction::ToggleScores => self.scores.toggle(),
            ControlAction::GameSpeedDown => self.adjust_game_speed(false, shift),
            ControlAction::GameSpeedUp => self.adjust_game_speed(true, shift),
            ControlAction::PauseGame if self.session_map => {
                self.dialog_ask = Some(DialogAsk::Pause);
            }
            ControlAction::PauseGame if self.own_pause.is_some() => self.open_own_pause(),
            ControlAction::PauseGame => {
                self.sim_paused = !self.sim_paused;
                info!(
                    "Game {}",
                    if self.sim_paused { "PAUSED" } else { "RESUMED" }
                );
            }
            ControlAction::OverheadMap => {
                match self.hud.as_mut().and_then(|h| {
                    h.push(
                        hud::BarId::Status,
                        mandrake_tag::hud::status_item::OVERHEAD_MAP,
                    )
                }) {
                    Some(event) => self.hud_event(event),
                    None => {
                        self.not_implemented.report("action overhead_map");
                    }
                }
            }
            ControlAction::ClearSelection => self.set_selection(Vec::new()),
            ControlAction::SelectAll => self.select_all(),
            ControlAction::CenterSelection => {
                if !self
                    .controls
                    .is_held(ControlAction::CenterSelection, &self.held_keys)
                {
                    self.center_selection(false);
                }
            }
            ControlAction::RotatePreset => {
                let selection = self.selected_unit_ids.clone();
                let p = self.preset_poll(0);
                let effects = self.selection_machine.presets.rotate_press(
                    &self.sim.engine,
                    shift,
                    &selection,
                    p,
                );
                self.preset_effects(effects);
            }
            ControlAction::ClearPreset => {
                let look = self.selection_machine.looking();
                let selection = self.selected_unit_ids.clone();
                let effects =
                    self.selection_machine
                        .presets
                        .clear_press(&self.sim.engine, &selection, look);
                self.preset_effects(effects);
            }
            ControlAction::StopUnits => self.bar_action(
                command_bar::BarAction::General(command_bar::GENERAL_STOP),
                shift,
            ),
            ControlAction::ScatterUnits | ControlAction::RetreatUnits => self.bar_action(
                command_bar::BarAction::General(command_bar::GENERAL_SCATTER),
                shift,
            ),
            ControlAction::SpecialAbility => {
                self.bar_action(command_bar::BarAction::Special, shift)
            }
            ControlAction::ChatMessage => {
                let shift = shift || self.held_keys.modifiers().shift;
                self.bar_action(command_bar::BarAction::Whisper, shift);
            }
            ControlAction::DisplayChat => {}
            ControlAction::DetachUnits => self.detach_press(),
            other => {
                if let Some(number) = other.preset_index() {
                    self.number_press(number, shift);
                } else {
                    self.not_implemented
                        .report(&format!("action {}", other.config_name()));
                }
            }
        }
    }

    fn preset_mode_skips_keys(&self) -> bool {
        self.selection_mode() == command_bar::SELECTION_MODE_PRESET
            && !self.controls.options.alternate_preset_recall
    }

    pub(super) fn selection_mode(&self) -> i16 {
        self.hud
            .as_ref()
            .map_or(command_bar::SELECTION_MODE_NORMAL, |h| {
                h.console.selection_mode
            })
    }

    fn preset_poll(&self, elapsed: i32) -> crate::local_input::PresetPoll {
        crate::local_input::PresetPoll {
            elapsed,
            recall_held: self
                .controls
                .is_held(ControlAction::RecallSetPreset, &self.held_keys),
            number: ControlAction::PRESET_SELECTIONS
                .iter()
                .position(|&a| self.controls.is_held(a, &self.held_keys)),
            mode: self.selection_mode(),
            alternate: self.controls.options.alternate_preset_recall,
            mouse_busy: false,
            selection_empty: self.selected_unit_ids.is_empty(),
            look: self.selection_machine.looking(),
            replay: (self.sim.engine.mode == 0),
        }
    }

    fn wheel_poll(&mut self, reached: bool) {
        let skipped = self.preset_mode_skips_keys();
        let look = self.selection_machine.looking();
        let effects = if reached && !skipped {
            self.selection_machine.presets.wheel_poll(
                &self.sim.engine,
                &self.selected_unit_ids,
                look,
            )
        } else {
            self.selection_machine.presets.drop_wheel();
            Vec::new()
        };
        if !effects.is_empty() {
            self.preset_effects(effects);
        }
    }

    fn number_press(&mut self, number: usize, shift: bool) {
        if self.selection_machine.state() != selection_input::State::Idle
            && (self.mouse_left_down || self.mouse_right_down)
        {
            return;
        }
        if self.detach_number(number, shift) {
            return;
        }
        let p = self.preset_poll(0);
        let effects = self
            .selection_machine
            .presets
            .number_press(number, shift, p);
        self.preset_effects(effects);
    }

    fn preset_effects(&mut self, effects: Vec<crate::local_input::PresetEffect>) {
        use crate::local_input::PresetEffect as E;
        for effect in effects {
            info!("Number keys: {effect:?}");
            match effect {
                E::Formation {
                    formation,
                    in_place,
                } => self.bar_action(command_bar::BarAction::Formation(formation), in_place),
                E::Recall { preset, toggle } => {
                    let mut selection = self.selected_unit_ids.clone();
                    {
                        let w = &self.sim.engine;
                        self.selection_machine
                            .presets
                            .recall(w, preset, toggle, &mut selection);
                    }
                    if !toggle {
                        self.selection_machine.stop_looking();
                        if let Some(h) = self.hud.as_mut() {
                            h.console.clear();
                        }
                    }
                    if !toggle || selection != self.selected_unit_ids {
                        self.clear_local_order_marks(false);
                    }
                    self.set_selection(selection);
                }
                E::Create { preset } => {
                    let polls = self.game_speed.polls();
                    if let (Some(h), w) = (self.hud.as_mut(), &self.sim.engine) {
                        h.show_preset_popup(w, &self.selected_unit_ids, preset, polls);
                    }
                    let c = self
                        .selection_machine
                        .presets
                        .create(preset, &self.selected_unit_ids);
                    info!("Preset selection command: {c:?}");
                }
                E::SelectNothing => {
                    self.selection_machine.stop_looking();
                    self.clear_local_order_marks(true);
                    self.set_selection(Vec::new());
                }
                E::Center { in_view_test } => self.center_selection(in_view_test),
                E::ClearSprites => self.clear_local_order_marks(false),
                E::EnterMode(mode) => {
                    let alternate = self.controls.options.alternate_preset_recall;
                    if let Some(h) = self.hud.as_mut() {
                        let (left, right) = h
                            .prompt_tags()
                            .map(|t| t.preset_mode_prompts(mode, alternate))
                            .unwrap_or_default();
                        h.console.set(&left, &right, mode);
                    }
                }
                E::ExitMode => {
                    if let Some(h) = self.hud.as_mut() {
                        h.console.clear();
                    }
                }
                E::AllyPick { number, shift } => {
                    let team =
                        command_bar::ally_pick(&mut self.sim, self.hud.as_mut(), number, shift);
                    info!("Ally: number {number} picks team {team:?}");
                }
            }
        }
    }

    fn minimap_press(&mut self, x: i32, y: i32, pressed: bool) -> bool {
        let origin = world_frame(self.ui.logical.0, self.ui.logical.1).1;
        let (fx, fy) = (x - origin.0, y - origin.1);
        let Some(mm) = self.minimap.as_mut() else {
            return false;
        };
        if !pressed {
            let was = mm.dragging;
            mm.dragging = false;
            mm.map.release();
            return was;
        }
        if self.mouse_left_down || !mm.map.takes(fx, fy) {
            return false;
        }
        let target = mm.map.click(fx, fy, true);
        mm.dragging = true;
        if let Some(p) = target {
            self.set_observer_location(p);
        }
        true
    }

    fn minimap_drag(&mut self, x: i32, y: i32) {
        let origin = world_frame(self.ui.logical.0, self.ui.logical.1).1;
        let target = self
            .minimap
            .as_mut()
            .and_then(|mm| mm.map.click(x - origin.0, y - origin.1, false));
        if let Some(p) = target {
            self.set_observer_location(p);
        }
    }

    fn hud_event(&mut self, event: hud::HudEvent) {
        match event {
            hud::HudEvent::Pause => self.dispatch_press(ControlAction::PauseGame, false),
            hud::HudEvent::Help { open } => {
                if open && self.session_map {
                    self.dialog_ask = Some(DialogAsk::Hints);
                } else {
                    info!(
                        "Help button: hints dialog {}",
                        if open { "open (no session)" } else { "closed" }
                    );
                }
            }
            hud::HudEvent::OverheadMap { shown } => {
                info!("Overhead map {}", if shown { "shown" } else { "hidden" });
                let local = media::local_time(self.sim.current_tick);
                if let Some(mm) = self.minimap.as_mut() {
                    mm.set_visible(shown, local);
                }
            }
            hud::HudEvent::Command { kind } => {
                if let Some(action) = command_bar::bar_action(kind) {
                    let shift = self.held_keys.modifiers().shift;
                    self.bar_action(action, shift);
                }
            }
        }
    }

    pub(crate) fn chat_typing(&self) -> bool {
        self.hud.as_ref().is_some_and(|h| h.console.input.typing())
    }

    fn typing_key(&mut self, code: KeyCode, text: Option<&str>, repeat: bool) {
        if code == KeyCode::Escape && !repeat {
            self.dispatch_press(ControlAction::PauseGame, false);
            return;
        }
        if let Some(key) = myth_key(code) {
            if !repeat && key.0 == selection::state::CAPS_LOCK_KEY as u8 {
                self.caps_lock_key.press();
            }
            let backlog = self
                .controls
                .bindings(ControlAction::DisplayChat)
                .iter()
                .any(|b| b.key == key);
            let camera = ControlAction::ALL
                .iter()
                .filter(|a| a.is_held_only())
                .any(|&a| self.controls.bindings(a).iter().any(|b| b.key == key));
            if key == MythKey::SHIFT || (backlog && !camera) {
                self.held_keys.press(key);
            }
            self.update_display_chat((!repeat).then_some(key));
        }
        self.chat_key(code, text);
    }

    fn update_display_chat(&mut self, pressed: Option<MythKey>) {
        let held = self
            .controls
            .is_held(ControlAction::DisplayChat, &self.held_keys);
        let shift_up = pressed.is_some_and(|k| k == UP_ARROW) && self.held_keys.modifiers().shift;
        let caps = self.caps_lock_key.is_down()
            && self.controls.options.caps_lock == crate::controls::CapsLock::ChatBacklog;
        if let Some(hud) = self.hud.as_mut() {
            hud.display_chat_action(held, shift_up, caps);
        }
    }

    fn chat_key(&mut self, code: KeyCode, text: Option<&str>) {
        let Some(key) = myth_key(code) else {
            return;
        };
        let ch = text
            .map(str::as_bytes)
            .filter(|b| b.len() == 1 && (0x20..0x7f).contains(&b[0]))
            .map(|b| b[0]);
        let now = media::local_time(self.sim.current_tick);
        let Some(hud) = self.hud.as_mut() else {
            return;
        };
        let mut input = hud.console.input.clone();
        input.key(key.0, ch, |t| hud.chat_input_fits(t), now);
        if input != hud.console.input {
            hud.console.input = input;
            hud.console.input_changed();
        }
        if !hud.console.input.typing() {
            self.chat_end();
        }
    }

    fn chat_click(&mut self) {
        if !self.chat_typing() {
            return;
        }
        if let Some(hud) = self.hud.as_mut() {
            hud.console.input.click();
        }
        self.chat_end();
    }

    fn chat_end(&mut self) {
        let now = media::local_time(self.sim.current_tick);
        let Some(hud) = self.hud.as_mut() else {
            return;
        };
        let mode = hud.console.selection_mode;
        let entered = hud.console.input.state == crate::console::InputState::Entered;
        let text = std::mem::take(&mut hud.console.input.text);
        hud.console.clear();
        if entered
            && matches!(
                mode,
                command_bar::CHAT_MODE_TEAM | command_bar::CHAT_MODE_ALL
            )
            && !text.is_empty()
        {
            let team = mode == command_bar::CHAT_MODE_TEAM;
            let data = command_bar::chat_command_data(team, &text);
            info!(
                "net: chat {} \"{}\" at tick {}",
                if team { "to the team" } else { "to everyone" },
                String::from_utf8_lossy(&text),
                self.sim.current_tick
            );
            self.sim
                .local_commands
                .push(mandrake_sim::QueuedCommand::Packet(
                    mandrake_rec::CommandPacket::parse(0, 0xff, command_bar::CHAT_COMMAND, &data),
                ));
        }
        self.local_sprites
            .remove_local_sprites(move_marker::RemoveMode::ClearUnlessTimed, now);
    }

    fn bar_action(&mut self, action: command_bar::BarAction, shift: bool) {
        let selection = self.selected_unit_ids.clone();
        let now = media::local_time(self.sim.current_tick);
        let outcome = command_bar::run_bar_action(
            action,
            shift,
            &selection,
            self.selection_machine.looking(),
            &mut self.sim,
            self.hud.as_mut(),
            &mut self.local_sprites,
            &mut self.interface.order_hide,
            now,
        );
        match outcome {
            command_bar::BarOutcome::NotPorted(what) => {
                self.not_implemented.report(what);
            }
            command_bar::BarOutcome::FormationMove(c) => {
                if let Err(e) = move_marker::make_markers(
                    &mut self.local_sprites,
                    &self.sim,
                    &c.unit_ids,
                    c.header,
                    &c.points,
                    move_marker::MarkerKind::Order,
                ) {
                    tracing::warn!("Move markers not made ({e})");
                }
                self.local_sprites
                    .remove_local_sprites(move_marker::RemoveMode::StartTimer, now);
                self.play_unit_voices(&selection, mandrake_tag::MonsterSoundType::MOVE_ORDER);
                command_bar::bar_tail(
                    &mut self.sim,
                    self.hud.as_mut(),
                    &mut self.local_sprites,
                    &mut self.interface.order_hide,
                    &selection,
                    now,
                );
                self.walking_orders.extend(selection.iter().copied());
                info!("Formation movement for {} unit(s): {c:?}", selection.len());
            }
            outcome => info!("{action:?} for {} unit(s): {outcome:?}", selection.len()),
        }
    }

    pub(super) fn set_selection(&mut self, mut ids: Vec<u32>) {
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(*id));
        for u in self.sim.units.values_mut() {
            u.selected = seen.contains(&u.id);
        }
        info!("Selected {} unit(s)", ids.len());
        if ids.is_empty()
            && let Some(h) = self.hud.as_mut()
        {
            h.console.clear();
        }
        self.interface.order_hide.units.clear();
        self.selected_unit_ids = ids;
    }

    fn select_all(&mut self) {
        let drawn = match self.gpu_state.as_ref() {
            Some(gpu) => drawn_units_in_rect(
                &crate::world_view::myth2::drawn_units(
                    &self.sim,
                    &self.unit_animator,
                    &gpu.sprite_mgr,
                ),
                &self.render_camera,
                &gpu.sprite_mgr,
                (self.width, self.height),
                selection::PixelRect::new(-0x8000, -0x8000, 0x7fff, 0x7fff),
                |_| true,
            ),
            None => Vec::new(),
        };
        let replay = self.sim.engine.mode == 0;
        if let Some(ids) = selection_rules::select_all(
            drawn,
            |id| selection_rules::select_all_eligible(&self.sim.engine, id),
            &self.selected_unit_ids,
            replay,
        ) {
            self.set_selection(ids);
        }
    }

    fn center_selection(&mut self, in_view_test: bool) {
        if self.camera_control.user_camera_locked() {
            return;
        }
        let frame = self.camera_frame();
        let observer = self.camera_control.observer(&self.observer, frame);
        let point = {
            let aspect = self.width as f32 / self.height.max(1) as f32;
            let view = self.render_camera.view_proj_matrix(aspect);
            let in_view = |p: (i32, i32, i32)| {
                let c = frame.from_engine([p.0 as f32, p.1 as f32, p.2 as f32]);
                let row = |r: usize| {
                    (0..3).map(|k| view.cols[k][r] * c[k]).sum::<f32>() + view.cols[3][r]
                };
                let (x, y, w) = (row(0), row(1), row(3));
                w > 0.0 && x.abs() <= w && y.abs() <= w
            };
            let test: Option<&dyn Fn((i32, i32, i32)) -> bool> = in_view_test.then_some(&in_view);
            let Some(p) = crate::local_input::selection_focus(
                &self.sim.engine,
                &self.selected_unit_ids,
                observer.zoom_factor(),
                test,
            ) else {
                return;
            };
            p
        };
        let angle = observer.location().facing.wrapping_sub(0x8000);
        self.camera_control.set_observer_focus(
            point,
            angle,
            crate::camera_control::view_slope(self.width, self.height.max(1)),
            self.sim.current_tick as i32,
            &mut self.observer,
            frame,
        );
        let moved = self.camera_control.observer(&self.observer, frame);
        debug!("Center selection on {point:?}: {moved:?}");
        self.place_camera_at_observer();
        self.prev_camera_state = self.camera.current_state();
        self.render_camera = self.camera.clone();
    }
}

impl ClientApp {
    pub(crate) fn redraw(
        &mut self,
        surface: Option<crate::window_surface::FrameSurface<'_>>,
        front_end: Option<&mut dyn mandrake_render::DrawInterface>,
    ) {
        match self.own_pause.take() {
            Some(mut own) if front_end.is_none() && own.is_open() => {
                let actions = own.step(self.dialog_ui().logical);
                if self.own_pause_actions(&mut own, actions) {
                    self.render_frame(surface, Some(own.front_end()));
                } else {
                    self.render_frame(surface, None);
                }
                self.own_pause = Some(own);
            }
            own => {
                self.own_pause = own;
                self.render_frame(surface, front_end);
            }
        }
        let frame = self.camera_frame();
        self.camera_control.view_built(&self.observer, frame);
    }

    fn open_own_pause(&mut self) {
        let originator = self.net.as_ref().is_some_and(|net| net.is_host());
        let gameplay = self.sim.engine.formation_globals.gameplay_version;
        let data = mandrake_session::Paused {
            network: Some(mandrake_session::screen::NetPause {
                originator,
                rehost_items: originator
                    && !self.sim.game_over()
                    && gameplay > mandrake_common::game_options::GAMEPLAY_1_7_2,
                can_rehost: true,
            }),
            ..mandrake_session::Paused::default()
        };
        let Some(own) = self.own_pause.as_mut() else {
            return;
        };
        info!("Pause dialog open at tick {}", self.sim.current_tick);
        own.open(data);
        self.input_taken();
        self.set_paused(true);
    }

    fn own_pause_actions(
        &mut self,
        own: &mut crate::app::shell_menu::OwnPauseDialog,
        actions: Vec<mandrake_session::Action>,
    ) -> bool {
        for action in actions {
            match action {
                mandrake_session::Action::Resume => {
                    info!("Pause dialog closed at tick {}", self.sim.current_tick);
                    own.close(self.dialog_ui().logical);
                    self.set_paused(false);
                    self.cursor_in_window = own.cursor_in_window;
                    return false;
                }
                mandrake_session::Action::QuitToMenu => {
                    info!("Pause dialog: Quit at tick {}", self.sim.current_tick);
                    own.close(self.dialog_ui().logical);
                    self.set_paused(false);
                    if let Some(net) = self.net.take() {
                        info!("net: leaving the game");
                        net.leave();
                    }
                    self.quit();
                    return false;
                }
                mandrake_session::Action::LastSavedGame
                | mandrake_session::Action::RestartLevel => {
                    let kind = if action == mandrake_session::Action::LastSavedGame {
                        2
                    } else {
                        3
                    };
                    info!("Pause dialog closed at tick {}", self.sim.current_tick);
                    own.close(self.dialog_ui().logical);
                    self.set_paused(false);
                    self.cursor_in_window = own.cursor_in_window;
                    self.force_network_endgame(kind);
                    return false;
                }
                action => info!(?action, "pause dialog: not in a direct window"),
            }
        }
        true
    }

    fn own_pause_event(&mut self, event: &WindowEvent) {
        let mapped = self.event_for_dialog(event);
        let Some(own) = self.own_pause.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.current_cursor_pos = (position.x as f32, position.y as f32);
                own.cursor_in_window = true;
            }
            WindowEvent::CursorLeft { .. } => own.cursor_in_window = false,
            _ => {}
        }
        own.window_event(mapped.as_ref().unwrap_or(event));
        let held = own.held;
        self.key_under_dialog(event, held);
        if let WindowEvent::KeyboardInput {
            event:
                KeyEvent {
                    physical_key: PhysicalKey::Code(code),
                    state: ElementState::Pressed,
                    repeat: false,
                    ..
                },
            ..
        } = event
        {
            let mods = binding_modifiers(held);
            let quits = myth_key(*code).is_some_and(|key| {
                self.controls
                    .actions_for_press_with(key, mods)
                    .contains(&ControlAction::Quit)
            });
            if quits {
                self.quit();
            }
        }
    }

    fn key_event(&mut self, code: KeyCode, state: ElementState, repeat: bool) {
        let Some(key) = myth_key(code) else {
            return;
        };
        match state {
            ElementState::Pressed => {
                let held = self.held_keys.modifiers();
                if let Some(delta) = ui_scale::zoom_key_delta(key, held) {
                    if !repeat {
                        self.adjust_ui_zoom(delta);
                    }
                    return;
                }
                self.held_keys.press(key);
                if !repeat {
                    if key.0 == selection::state::CAPS_LOCK_KEY as u8 {
                        self.caps_lock_key.press();
                    }
                    self.press_key(key);
                } else {
                    self.repeat_key(key);
                }
            }
            ElementState::Released => self.held_keys.release(key),
        }
    }

    pub(crate) fn window_event(&mut self, event: WindowEvent) {
        if self
            .own_pause
            .as_ref()
            .is_some_and(crate::app::shell_menu::OwnPauseDialog::is_open)
            && !matches!(event, WindowEvent::Occluded(_) | WindowEvent::Focused(_))
        {
            self.own_pause_event(&event);
            return;
        }
        match event {
            WindowEvent::Occluded(occluded) => {
                if occluded != self.occluded {
                    debug!(
                        "Window {}; rendering {}",
                        if occluded { "occluded" } else { "visible" },
                        if occluded { "paused" } else { "resumed" }
                    );
                }
                self.occluded = occluded;
            }
            WindowEvent::Focused(true) => {
                debug!("window focused at tick {}", self.sim.current_tick);
                self.focused = true;
            }
            WindowEvent::Focused(false) => {
                debug!("window unfocused at tick {}", self.sim.current_tick);
                self.focused = false;
                self.clear_held_input();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        repeat,
                        text,
                        ..
                    },
                ..
            } => {
                if state == ElementState::Pressed && self.chat_typing() {
                    self.typing_key(code, text.as_deref(), repeat);
                    return;
                }
                self.key_event(code, state, repeat);
                let pressed = state == ElementState::Pressed && !repeat;
                self.update_display_chat(myth_key(code).filter(|_| pressed));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        let (cx, cy) = self
                            .ui
                            .to_logical(self.current_cursor_pos.0, self.current_cursor_pos.1);
                        if let Some(hud) = self.hud.as_mut() {
                            if pressed && !self.mouse_left_down && hud.press(cx, cy) {
                                return;
                            }
                            if !pressed && hud.tracking() {
                                if let Some(event) = hud.release(cx, cy) {
                                    self.hud_event(event);
                                }
                                return;
                            }
                        }
                        if self.minimap_press(cx, cy, pressed) {
                            return;
                        }
                        if pressed {
                            self.chat_click();
                        }
                        self.mouse_left_down = pressed;
                        if pressed {
                            self.pressed_since_poll.0 = true;
                        }
                    }
                    MouseButton::Right => {
                        self.mouse_right_down = pressed;
                        if pressed {
                            self.chat_click();
                            self.pressed_since_poll.1 = true;
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::CursorEntered { .. } => self.cursor_in_window = true,
            WindowEvent::CursorLeft { .. } => {
                self.cursor_in_window = false;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_in_window = true;
                self.current_cursor_pos = (position.x as f32, position.y as f32);
                let (ix, iy) = self.ui.to_logical(position.x as f32, position.y as f32);
                if let Some(hud) = self.hud.as_mut() {
                    hud.moved(ix, iy);
                }
                if self.minimap.as_ref().is_some_and(|m| m.dragging) {
                    self.minimap_drag(ix, iy);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = self.wheel_notches.push(delta);
                if notches != 0 {
                    let up = notches > 0;
                    let shift = self.held_keys.modifiers().shift;
                    for _ in 0..notches.unsigned_abs() {
                        match self
                            .wheel
                            .notch(up, shift, self.controls.options.mouse_wheel)
                        {
                            WheelEffect::ZoomPreset(zoom_in) => {
                                self.observer.cycle_zoom_preset(zoom_in)
                            }
                            WheelEffect::CyclePresets { up } => {
                                self.selection_machine.presets.wheel_notch(up);
                            }
                            WheelEffect::NotImplemented(mode) => {
                                let label = match mode {
                                    controls::MouseWheel::Ignored => {
                                        "option mouse_wheel = \"ignored\""
                                    }
                                    controls::MouseWheel::Move => "option mouse_wheel = \"move\"",
                                    controls::MouseWheel::Zoom => "option mouse_wheel = \"zoom\"",
                                    controls::MouseWheel::CyclePresets => {
                                        "option mouse_wheel = \"cycle_presets\""
                                    }
                                    controls::MouseWheel::CycleInventory => {
                                        "option mouse_wheel = \"cycle_inventory\""
                                    }
                                };
                                self.not_implemented.report(label);
                            }
                            WheelEffect::None | WheelEffect::Hold => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn clear_held_input(&mut self) {
        self.held_keys.clear();
        self.wheel.clear();
        self.wheel_notches.clear();
        self.observer.stop();
        self.mouse_left_down = false;
        self.mouse_right_down = false;
        self.pressed_since_poll = (false, false);
        if let Some(hud) = self.hud.as_mut()
            && hud.tracking()
        {
            let _ = hud.release(i32::MIN, i32::MIN);
        }
        if let Some(minimap) = self.minimap.as_mut() {
            minimap.dragging = false;
        }
    }

    pub(crate) fn input_taken(&mut self) {
        self.clear_held_input();
        self.cursor_in_window = false;
        self.pending_presses.clear();
    }

    pub(crate) fn take_dialog_ask(&mut self) -> Option<DialogAsk> {
        self.dialog_ask.take()
    }

    pub(crate) fn set_hints_open(&mut self, open: bool) {
        if self.hints_open == Some(open) {
            return;
        }
        let Some(hud) = self.hud.as_mut() else {
            return;
        };
        self.hints_open = Some(open);
        let item = mandrake_tag::hud::status_item::HELP;
        if hud
            .button(hud::BarId::Status, item)
            .is_some_and(|b| (b.state == 1) != open)
        {
            let _ = hud.push(hud::BarId::Status, item);
        }
    }

    pub(crate) fn dialog_ui(&self) -> ui_scale::UiScale {
        self.ui.dialogs(self.dialog_scale)
    }

    pub(crate) fn event_for_dialog(&self, event: &WindowEvent) -> Option<WindowEvent> {
        let WindowEvent::CursorMoved {
            device_id,
            position,
        } = event
        else {
            return None;
        };
        let (x, y) = self
            .dialog_ui()
            .to_logical(position.x as f32, position.y as f32);
        Some(WindowEvent::CursorMoved {
            device_id: *device_id,
            position: PhysicalPosition::new(f64::from(x), f64::from(y)),
        })
    }

    pub(crate) fn key_under_dialog(&mut self, event: &WindowEvent, held: ModifiersState) {
        let WindowEvent::KeyboardInput {
            event:
                KeyEvent {
                    physical_key: PhysicalKey::Code(code),
                    state: ElementState::Pressed,
                    repeat,
                    ..
                },
            ..
        } = event
        else {
            return;
        };
        self.speed_key_under_dialog(*code, held);
        if *repeat {
            return;
        }
        self.zoom_key_under_dialog(*code, held);
        self.quicksave_key_under_dialog(*code, held);
    }

    fn speed_key_under_dialog(&mut self, code: KeyCode, held: ModifiersState) {
        if self.net.is_none() {
            return;
        }
        let mods = binding_modifiers(held);
        let Some(key) = myth_key(code) else {
            return;
        };
        for action in self.controls.actions_for_press_with(key, mods) {
            match action {
                ControlAction::GameSpeedDown => self.adjust_game_speed(false, mods.shift),
                ControlAction::GameSpeedUp => self.adjust_game_speed(true, mods.shift),
                _ => {}
            }
        }
    }

    fn quicksave_key_under_dialog(&mut self, code: KeyCode, held: ModifiersState) {
        if !self.session_map || held.shift_key() || held.control_key() {
            return;
        }
        let mods = binding_modifiers(held);
        let quicksave = myth_key(code).is_some_and(|key| {
            self.controls
                .actions_for_press_with(key, mods)
                .contains(&ControlAction::Quicksave)
        });
        if quicksave {
            self.dialog_ask = Some(DialogAsk::Quicksave);
        }
    }

    fn zoom_key_under_dialog(&mut self, code: KeyCode, held: ModifiersState) {
        let held = binding_modifiers(held);
        if let Some(delta) = myth_key(code).and_then(|key| ui_scale::zoom_key_delta(key, held)) {
            self.adjust_ui_zoom(delta);
        }
    }

    pub(crate) fn set_view_drop(&mut self, drop: i32) {
        if let Some(hud) = self.hud.as_mut() {
            hud.set_view_drop(drop);
        }
        if let Some(minimap) = self.minimap.as_mut() {
            minimap.map.view_drop = drop;
        }
    }

    pub(crate) fn input_given(&mut self, facts: WindowFacts) {
        self.cursor_in_window = facts.cursor_in_window;
        self.current_cursor_pos = facts.cursor;
    }

    pub(crate) fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) -> bool {
        self.update_title_tick();
        let local = self.net.is_some().then(|| self.game_speed.polls());
        if let Some(hud) = self.hud.as_mut() {
            hud.set_local_time(local);
        }
        if self.frame_budget.as_ref().is_some_and(|b| b.finished()) {
            self.quit();
            return false;
        }
        let now = self.machine_clock.elapsed().as_secs_f64();
        if self.map_end.as_ref().is_some_and(|end| end.finished(now)) {
            self.finish_map();
            return false;
        }
        if self.window.is_none() {
            return false;
        }
        if !self.occluded {
            return true;
        }
        self.update_physics_and_simulation();
        let to_next_tick = (1.0 / 30.0 - self.physics_accumulator).max(0.001);
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + std::time::Duration::from_secs_f32(to_next_tick),
        ));
        false
    }
}

pub(crate) fn toggle_control_bar(hud: &mut hud::Hud, bars: &ViewBars) -> Option<bool> {
    let shown = hud.toggle_control_bar()?;
    bars.set_control_bar(shown);
    Some(shown)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelessRoute {
    Dialog,
    Game,
    Both,
}

pub(crate) fn modeless_route(
    event: &WindowEvent,
    pointer: (i32, i32),
    rect: [i32; 4],
    pressed_in_dialog: &mut bool,
) -> ModelessRoute {
    let inside = pointer.0 >= rect[0]
        && pointer.0 < rect[0] + rect[2]
        && pointer.1 >= rect[1]
        && pointer.1 < rect[1] + rect[3];
    match event {
        WindowEvent::CursorMoved { .. }
        | WindowEvent::CursorEntered { .. }
        | WindowEvent::CursorLeft { .. } => ModelessRoute::Both,
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } => {
            if inside || *pressed_in_dialog {
                *pressed_in_dialog = true;
                ModelessRoute::Dialog
            } else {
                ModelessRoute::Game
            }
        }
        WindowEvent::MouseInput {
            state: ElementState::Released,
            ..
        } => {
            if std::mem::take(pressed_in_dialog) {
                ModelessRoute::Dialog
            } else {
                ModelessRoute::Game
            }
        }
        WindowEvent::MouseWheel { .. } if inside => ModelessRoute::Dialog,
        _ => ModelessRoute::Game,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_wheel_accumulates_before_emitting_a_notch() {
        let mut wheel = WheelNotches::default();
        let px = |y| MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y));
        assert_eq!(wheel.push(px(15.0)), 0);
        assert_eq!(wheel.push(px(15.0)), 0);
        assert_eq!(wheel.push(px(15.0)), 1);
        assert_eq!(wheel.push(px(-10.0)), 0);
        assert_eq!(wheel.push(px(-30.0)), -1);
        assert_eq!(wheel.push(MouseScrollDelta::LineDelta(0.0, 2.0)), 1);
        assert_eq!(wheel.push(MouseScrollDelta::LineDelta(0.0, -2.0)), -1);
    }

    #[test]
    fn binding_modifiers_maps_super_to_alt() {
        let mods = binding_modifiers(ModifiersState::SUPER);
        assert!(mods.alt && !mods.ctrl && !mods.shift);
    }
}
