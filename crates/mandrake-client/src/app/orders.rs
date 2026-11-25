use crate::app::ClientApp;
use crate::{attack_click, command_bar, media, move_marker, selection_input};
use tracing::{info, warn};

impl ClientApp {
    pub(crate) fn order_attack_on(&mut self, target_id: u32, flags: selection_input::ActionFlags) {
        let attackers: Vec<u32> = self
            .selected_unit_ids
            .iter()
            .copied()
            .filter(|id| self.sim.units.get(id).is_some_and(|u| u.is_alive()))
            .collect();
        if attackers.is_empty() {
            return;
        }
        info!(
            "Dispatched attack order for {} unit(s) on unit #{}",
            attackers.len(),
            target_id
        );
        let input = attack_click::Input {
            modifiers: attack_click::Modifiers {
                option: flags.test(selection_input::OPTION),
                control: flags.test(selection_input::CONTROL),
                shift: flags.test(selection_input::SHIFT),
            },
            selection_mode: self.selection_mode(),
            tfl_mouse_actions: self.controls.options.tfl_mouse_actions,
            game_options_high: self.sim.game_options_high,
        };
        let (targets, voiced) =
            attack_click::order_attack(&mut self.sim, &attackers, target_id, input);
        info!(
            "Attack click {:?}: {} target(s) {targets:?}",
            self.sim.attack_click,
            targets.len()
        );
        self.interface
            .targets
            .command(&targets, &self.selected_unit_ids);
        if voiced {
            self.play_unit_voices(&attackers, mandrake_tag::MonsterSoundType::ATTACK_ORDER);
        }
        self.clear_local_order_marks(true);
        self.interface.order_hide.order(&self.selected_unit_ids);
    }

    pub(crate) fn order_attack_again(&mut self, attackers: &[u32], target: u32, shift: bool) {
        let flags = if shift { 0x81 } else { 1 };
        let w = &self.sim.engine;
        let selection: Vec<usize> = attackers
            .iter()
            .filter_map(|&id| w.monster_index_by_identifier(id as u16))
            .collect();
        let Some(t) = w.monster_index_by_identifier(target as u16) else {
            return;
        };
        if selection.is_empty() {
            return;
        }
        let (flags, targets) = w.local_target_command(&selection, t, flags, false, false);
        let target_ids: Vec<u32> = targets
            .iter()
            .map(|&j| u32::from(w.monsters[j].identifier as u16))
            .collect();
        info!(
            "Attack double click: TARGET flags {flags:#x} for {} unit(s), {} target(s) \
             {target_ids:?}",
            attackers.len(),
            target_ids.len()
        );
        self.sim.queue_local_target(mandrake_sim::LocalTarget {
            flags,
            unit_ids: attackers.to_vec(),
            target_ids: target_ids.clone(),
        });
        self.interface
            .targets
            .command(&target_ids, &self.selected_unit_ids);
    }

    pub(crate) fn order_ground_move(
        &mut self,
        point: (i32, i32),
        facing: Option<u16>,
        add_waypoint: bool,
    ) {
        let target = self.sim.command_point_to_world(point.0, point.1);
        let (x, z) = (target.x.to_f32(), target.z.to_f32());
        let kind = self
            .sim
            .engine_terrain
            .as_ref()
            .map(|c| c.terrain_type(point.0, point.1));
        let selection = self.selected_unit_ids.clone();
        let now = media::local_time(self.sim.current_tick);
        match move_marker::ground_click(
            &mut self.local_sprites,
            &mut self.sim,
            &selection,
            point,
            facing,
            add_waypoint,
            now,
        ) {
            Ok(Some(command)) => {
                info!(
                    "Movement command for {} unit(s) to world {point:?} (render ({x:.2}, \
                     {z:.2}), terrain type {kind:?}), header {:?}, waypoints {:?}",
                    command.unit_ids.len(),
                    command.header,
                    command.points,
                );
                self.log_local_sprites();
                self.interface.order_hide.order(&command.unit_ids);
                if let Some(h) = self.hud.as_mut() {
                    h.console.clear();
                }
                self.walking_orders.extend(selection.iter().copied());
                self.play_unit_voices(&selection, mandrake_tag::MonsterSoundType::MOVE_ORDER);
            }
            Ok(None) => {
                info!(
                    "Waypoint {} at world {point:?} (render ({x:.2}, {z:.2}), terrain \
                     type {kind:?}) for {} unit(s)",
                    self.sim.local_waypoints.len(),
                    selection.len(),
                );
                self.log_local_sprites();
            }
            Err(e) => {
                warn!("Movement command not modelled ({e}); ordering the movers");
                self.sim.local_waypoints.clear();
                self.sim.order_move(&selection, target);
                self.walking_orders.extend(selection.iter().copied());
                self.play_unit_voices(&selection, mandrake_tag::MonsterSoundType::MOVE_ORDER);
            }
        }
    }

    pub(crate) fn order_unit_click(&mut self, facing: Option<u16>) {
        let selection = self.selected_unit_ids.clone();
        match self.sim.local_move_click(
            &selection,
            mandrake_sim::engine_rng::MoveClick::LocalSprite,
            facing,
            false,
        ) {
            Ok(Some(command)) => {
                info!(
                    "Movement command for {} unit(s) from a unit click, header {:?}, \
                     waypoints {:?}",
                    command.unit_ids.len(),
                    command.header,
                    command.points,
                );
                self.clear_local_order_marks(true);
                self.interface.order_hide.order(&command.unit_ids);
                self.walking_orders.extend(selection.iter().copied());
            }
            Ok(None) => {}
            Err(e) => warn!("Unit click not modelled ({e})"),
        }
    }

    pub(crate) fn order_attack_point(&mut self, point: (i32, i32), special: bool) {
        let (flags, z) = self
            .sim
            .engine
            .local_attack_location(point.0, point.1, special);
        if self.sim.is_replay() {
            return;
        }
        let selection = self.selected_unit_ids.clone();
        info!(
            "Attack location for {} unit(s) at {point:?}, flags {flags}, height {z}",
            selection.len()
        );
        self.sim
            .queue_local_attack_location(mandrake_sim::LocalAttackLocation {
                flags,
                point: (point.0, point.1, z),
                unit_ids: selection.clone(),
            });
        let now = media::local_time(self.sim.current_tick);
        move_marker::attack_ground_marker(&mut self.local_sprites, [point.0, point.1, z], now);
        command_bar::bar_tail(
            &mut self.sim,
            self.hud.as_mut(),
            &mut self.local_sprites,
            &mut self.interface.order_hide,
            &selection,
            now,
        );
        if let Some(h) = self.hud.as_mut() {
            h.attack_ground_mode();
        }
    }

    pub(crate) fn order_attack_model(&mut self, point: (i32, i32, i32), special: bool) {
        if self.sim.is_replay() {
            return;
        }
        let selection = self.selected_unit_ids.clone();
        let flags = 2 | u16::from(special);
        info!(
            "Attack location on a model for {} unit(s) at {point:?}, flags {flags}",
            selection.len()
        );
        self.sim
            .queue_local_attack_location(mandrake_sim::LocalAttackLocation {
                flags,
                point,
                unit_ids: selection.clone(),
            });
        let now = media::local_time(self.sim.current_tick);
        command_bar::bar_tail(
            &mut self.sim,
            self.hud.as_mut(),
            &mut self.local_sprites,
            &mut self.interface.order_hide,
            &selection,
            now,
        );
        if let Some(h) = self.hud.as_mut() {
            h.attack_ground_mode();
        }
    }

    pub(crate) fn order_pick_up(&mut self, index: i16, identifier: i16, units: &[u32]) {
        if self.sim.is_replay() {
            return;
        }
        let alive: Vec<u32> = units
            .iter()
            .copied()
            .filter(|id| self.sim.units.get(id).is_some_and(|u| u.is_alive()))
            .collect();
        info!(
            "Pick-up of object {index} (identifier {identifier}) for {} unit(s)",
            alive.len()
        );
        self.sim.queue_local_pick_up(mandrake_sim::LocalPickUp {
            object_index: index,
            object_identifier: identifier,
            unit_ids: alive,
        });
        self.interface.targets.pick_up(i32::from(index));
        let now = media::local_time(self.sim.current_tick);
        command_bar::bar_tail(
            &mut self.sim,
            self.hud.as_mut(),
            &mut self.local_sprites,
            &mut self.interface.order_hide,
            units,
            now,
        );
    }
}
