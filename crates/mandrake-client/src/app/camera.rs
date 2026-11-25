use crate::app::ClientApp;
use crate::{camera_control, controls};
use tracing::info;

impl ClientApp {
    pub(crate) fn step_observer(&mut self, accepted: bool) {
        if self.following() {
            let visibility = self.minimap.as_ref().map(|m| &m.vis);
            camera_control::step_user_observer(
                &self.camera_control,
                &mut self.observer,
                Default::default(),
                visibility,
            );
            self.place_camera_at_observer();
            return;
        }
        let keys = if accepted {
            self.controls.keyboard_axes(&self.held_keys)
        } else {
            Default::default()
        };
        let edge = if accepted && self.cursor_in_window && !self.mouse_left_down {
            let options = self.controls.options;
            if options.top_corners_rotate_camera {
                self.not_implemented
                    .report("option top_corners_rotate_camera");
            }
            if options.bottom_corners_orbit_camera {
                self.not_implemented
                    .report("option bottom_corners_orbit_camera");
            }
            match options.horiz_mouse_movement {
                controls::HorizMouseMovement::Rotate => {
                    self.not_implemented.report(
                        "option horiz_mouse_movement = \"rotate\" (the side edges move instead)",
                    );
                }
                controls::HorizMouseMovement::Orbit => {
                    self.not_implemented.report(
                        "option horiz_mouse_movement = \"orbit\" (the side edges move instead)",
                    );
                }
                controls::HorizMouseMovement::Ignored | controls::HorizMouseMovement::Move => {}
            }
            controls::edge_scroll_axes(
                self.current_cursor_pos.0,
                self.current_cursor_pos.1,
                self.width as f32,
                self.height as f32,
                &options,
            )
        } else {
            Default::default()
        };
        let wheel = self.wheel.tick(self.controls.options.mouse_wheel);
        self.observer.zoom_locked = self.held_keys.modifiers().shift;
        let visibility = self.minimap.as_ref().map(|m| &m.vis);
        if self.camera_control.explored_only && visibility.is_none() {
            self.not_implemented
                .report("explored-only camera limit without an overhead map");
        }
        camera_control::step_user_observer(
            &self.camera_control,
            &mut self.observer,
            controls::sum_axes(&[keys, edge, wheel]),
            visibility,
        );
        self.place_camera_at_observer();
    }

    pub(crate) fn camera_frame(&self) -> mandrake_view::model_surfaces::WorldFrame {
        mandrake_view::model_surfaces::WorldFrame::new(
            self.mesh_tag.cell_width,
            self.mesh_tag.cell_height,
        )
    }

    pub(crate) fn seed_observer(&mut self, state: camera_control::ObserverState) {
        let frame = self.camera_frame();
        self.camera_control.seed(state, &mut self.observer, frame);
    }

    pub(crate) fn camera_after_tick(&mut self, tick: i32) {
        let frame = self.camera_frame();
        let cells = (
            i32::from(self.mesh_tag.cell_width),
            i32::from(self.mesh_tag.cell_height),
        );
        let runs =
            self.camera_control
                .after_tick(&mut self.sim, tick, &mut self.observer, frame, cells);
        for run in &runs {
            info!("Camera action (obmo) at tick {}: {run:?}", run.time);
        }
        if self.camera_control.flags & camera_control::FLAG_TRACK != 0 {
            self.not_implemented
                .report("camera control flag 4 (update_object_tracking)");
        }
        if self.camera_control.user_camera_locked() {
            self.place_camera_at_observer();
        }
    }

    fn following(&self) -> bool {
        self.frame_budget
            .as_ref()
            .is_some_and(|b| b.follow.is_some())
    }

    pub(crate) fn follow_unit(&mut self) {
        let Some(follow) = self.frame_budget.as_mut().and_then(|b| b.follow.as_mut()) else {
            return;
        };
        let unit = follow.unit;
        let Some(point) = follow.step(&self.sim) else {
            return;
        };
        if !self.camera_control.user_camera_locked() {
            let frame = self.camera_frame();
            let mut loc = self
                .camera_control
                .observer(&self.observer, frame)
                .location();
            loc.point = point;
            self.camera_control
                .set_observer_location(loc, &mut self.observer, frame);
            self.place_camera_at_observer();
        }
        let eye = self.camera.eye_position();
        let observer = self
            .camera_control
            .observer(&self.observer, self.camera_frame());
        info!(
            target: "frame_budget_follow",
            "tick {} unit {unit} point {},{} eye {:.3},{:.3},{:.3} observer {:?},{:?},{:?},{:?} \
             view-camera {:?},{:?},{:?},{:?},{:?},{:?}",
            self.sim.current_tick,
            point.0,
            point.1,
            eye[0],
            eye[1],
            eye[2],
            observer.x,
            observer.y,
            observer.facing,
            observer.zoom,
            self.camera.target[0],
            self.camera.target[1],
            self.camera.target[2],
            self.camera.distance,
            self.camera.yaw,
            self.camera.pitch,
        );
    }

    pub(crate) fn place_camera_at_observer(&mut self) {
        let mesh = &self.mesh_tag;
        let half_w = mesh.cell_width as f32 * 0.5;
        let half_d = mesh.cell_height as f32 * 0.5;
        self.observer.apply_to(&mut self.camera, |x, z| {
            mesh.terrain_height_at(half_w - x, z + half_d)
        });
    }

    pub(crate) fn set_observer_location(&mut self, p: (i32, i32)) {
        if self.camera_control.user_camera_locked() {
            return;
        }
        let frame = self.camera_frame();
        let mut loc = self
            .camera_control
            .observer(&self.observer, frame)
            .location();
        loc.point = p;
        self.camera_control
            .set_observer_location(loc, &mut self.observer, frame);
        self.place_camera_at_observer();
        self.prev_camera_state = self.camera.current_state();
        self.render_camera = self.camera.clone();
        info!(
            "Overhead map click: view centre to world ({}, {})",
            p.0, p.1
        );
    }
}
