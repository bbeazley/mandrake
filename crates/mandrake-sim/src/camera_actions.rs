use crate::engine_rng::Unmodelled;
use crate::map_actions::{MapActionContext, MapActions, TYPE_DISTANCE, TYPE_INTEGER};

pub const CAMERA_ACTIONS_KEPT: usize = 64;

const TYPE_FLAG: i16 = 0;
const TYPE_SHORT: i16 = 4;
const TYPE_TIME: i16 = 8;

const MAX_SUBJECTS: usize = 0x190;

pub const SUBJECT_CAMERA_RADIUS: i32 = 0x1400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraAction {
    pub time: i32,
    pub action: usize,
    pub identifier: u16,
    pub control_flags: Option<i16>,
    pub keep_control: Option<bool>,
    pub movement: Option<CameraMove>,
    pub shake: Option<CameraShake>,
    pub result: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraTarget {
    Point {
        point: (i32, i32),
        facing: u16,
    },
    Monster {
        monster: usize,
        position: (i32, i32),
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraMove {
    pub target: CameraTarget,
    pub line: Option<i32>,
    pub angu: Option<i32>,
    pub time: Option<i32>,
    pub maxz: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraShake {
    pub intensity: i32,
    pub duration: i32,
}

impl MapActions {
    pub(crate) fn observer_movement(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        if !self.local_proc_runs(i, ctx) {
            return Ok(0);
        }
        let mut run = CameraAction {
            time: ctx.time,
            action: i,
            identifier: self.actions[i].identifier,
            control_flags: None,
            keep_control: None,
            movement: None,
            shake: None,
            result: 2,
        };
        if ctx.world.mode != 0 {
            match self.observer_mode(i, ctx, &mut run) {
                Some(()) => {}
                None => {
                    run.control_flags = Some(0);
                    run.result = 0;
                    self.record_camera_action(run);
                    return Ok(0);
                }
            }
        }
        if self.find(i, 2, b"shak", Some(TYPE_FLAG)).is_some() {
            let duration = self
                .indexed(i, b"dura", TYPE_TIME, 0)
                .map_or(-1, |d| d.wrapping_mul(30) >> 16);
            let intensity = self.indexed(i, b"inte", TYPE_DISTANCE, 0).unwrap_or(-1);
            run.shake = Some(CameraShake {
                intensity,
                duration,
            });
        }
        self.record_camera_action(run);
        Ok(2)
    }

    fn observer_mode(
        &self,
        i: usize,
        ctx: &MapActionContext,
        run: &mut CameraAction,
    ) -> Option<()> {
        let mode = self.indexed(i, b"mode", TYPE_INTEGER, 0)?;
        if mode == -1 {
            run.control_flags = Some(0);
            return Some(());
        }
        let bits = 1u32 << (mode as u32 & 31);
        run.control_flags = Some(bits as u16 as i16);
        if bits & 2 == 0 {
            return Some(());
        }
        run.keep_control = Some(self.find(i, 2, b"drco", Some(TYPE_FLAG)).is_some());
        let target = if self
            .combined(i, b"subj", crate::map_actions::TYPE_MONSTERS, MAX_SUBJECTS)
            .is_empty()
        {
            let point = self.indexed_point(i, b"dest", 0)?;
            let facing = self.indexed(i, b"faci", TYPE_SHORT, 0)? as u16;
            CameraTarget::Point { point, facing }
        } else {
            match self.subjects(i, ctx, MAX_SUBJECTS)[..] {
                [m] => CameraTarget::Monster {
                    monster: m,
                    position: ctx.world.monsters[m].position,
                },
                _ => return None,
            }
        };
        run.movement = Some(CameraMove {
            target,
            line: self.indexed(i, b"line", TYPE_DISTANCE, 0),
            angu: self.indexed(i, b"angu", TYPE_INTEGER, 0),
            time: self.indexed(i, b"time", TYPE_TIME, 0),
            maxz: self.find(i, 2, b"maxz", Some(TYPE_FLAG)).is_some(),
        });
        Some(())
    }

    fn record_camera_action(&mut self, run: CameraAction) {
        if self.camera_actions.len() >= CAMERA_ACTIONS_KEPT {
            self.camera_actions.remove(0);
        }
        self.camera_actions.push(run);
    }
}
