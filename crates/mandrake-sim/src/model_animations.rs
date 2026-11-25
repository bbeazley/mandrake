use std::sync::Arc;

use mandrake_common::FourCC;
use mandrake_tag::model_anim::{ModelAnimationPlayback, ModelAnimationTag};
use mandrake_tag::model_hit::HitGeometry;

use crate::draw_site;
use crate::engine_rng::{
    EngineWorld, TagSource, Unmodelled, update_monster_impassability_for_cell,
};
use crate::models::{
    ModelShape, WorldModel, clear_model_bounds_between, delete_model_cell_with_matrix,
    new_model_cells_for_version,
};

const DELETED_MODEL_UPDATE_ABOVE: i32 = 0x6b3;

#[derive(Clone, Debug)]
pub struct FrameModel {
    pub shape: Arc<ModelShape>,
    pub geometry: Arc<HitGeometry>,
    pub geometry_tag: Option<FourCC>,
    pub surfaces: Arc<Vec<crate::models::SurfaceMaterial>>,
}

#[derive(Clone, Debug)]
pub struct AnimationFrame {
    pub mode_tag: FourCC,
    pub permutation: i16,
    pub model: Option<FrameModel>,
}

#[derive(Clone, Debug)]
pub struct AnimationDefinition {
    pub tag: ModelAnimationTag,
    pub frames: Vec<AnimationFrame>,
}

impl AnimationDefinition {
    pub fn from_tags(anim: &[u8], tags: &dyn TagSource) -> Result<Self, Unmodelled> {
        let tag = ModelAnimationTag::from_bytes(anim)
            .map_err(|e| Unmodelled::new(draw_site::MODEL_ANIMATIONS, format!("anim: {e}")))?;
        let mut frames = Vec::with_capacity(tag.frames.len());
        for (i, f) in tag.frames.iter().enumerate() {
            let o = 0x12 + ModelAnimationTag::FRAME_SIZE * i;
            let permutation = anim
                .get(o..o + 2)
                .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]));
            let model = match tags.tag(*b"mode", f.model_tag) {
                None => None,
                Some(mode) => {
                    let geom_tag = FourCC::new([mode[4], mode[5], mode[6], mode[7]]);
                    let geom = tags.tag(*b"geom", geom_tag).ok_or_else(|| {
                        Unmodelled::new(
                            draw_site::MODEL_GEOMETRY_MISSING,
                            format!("geom {geom_tag} missing"),
                        )
                    })?;
                    let shape = ModelShape::from_tags(&mode, &geom).map_err(|e| {
                        Unmodelled::new(
                            draw_site::MODEL_GEOMETRY_MISSING,
                            format!("mode {}: {e}", f.model_tag),
                        )
                    })?;
                    let parsed =
                        mandrake_tag::model::GeometryTag::from_bytes(&geom).map_err(|e| {
                            Unmodelled::new(
                                draw_site::MODEL_GEOMETRY_MISSING,
                                format!("geom {geom_tag}: {e}"),
                            )
                        })?;
                    Some(FrameModel {
                        shape: Arc::new(shape),
                        geometry: Arc::new(HitGeometry::new(&parsed)),
                        geometry_tag: Some(geom_tag),
                        surfaces: Arc::new(crate::models::surface_materials(&parsed)),
                    })
                }
            };
            frames.push(AnimationFrame {
                mode_tag: f.model_tag,
                permutation,
                model,
            });
        }
        Ok(Self { tag, frames })
    }
}

#[derive(Clone, Debug)]
pub struct ModelAnimation {
    pub marker: usize,
    pub identifier: i16,
    pub definition: Arc<AnimationDefinition>,
    pub playback: ModelAnimationPlayback,
    pub position: (i32, i32, i32),
    pub pitch: u16,
    pub model: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelAnimationFrameState {
    pub marker: usize,
    pub identifier: i16,
    pub frame: usize,
    pub running: bool,
    pub model: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelAnimationSound {
    pub animation: usize,
    pub sound: FourCC,
    pub position: (i32, i32, i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelAnimationEvent {
    Advance(usize),
    Jump {
        animation: usize,
        frame: usize,
        old: usize,
        flags: u16,
        running: bool,
        model: i16,
    },
    Delete(i16),
    New(i16),
}

thread_local! {
    pub static MODEL_ANIMATION_LOG: std::cell::RefCell<Option<Vec<ModelAnimationEvent>>> =
        const { std::cell::RefCell::new(None) };
}

fn log_event(e: ModelAnimationEvent) {
    MODEL_ANIMATION_LOG.with(|l| {
        if let Some(l) = l.borrow_mut().as_mut() {
            l.push(e);
        }
    });
}

fn datum_flags(p: &ModelAnimationPlayback) -> u16 {
    u16::from(p.forward) | u16::from(p.ping_pong) << 1 | u16::from(p.looping) << 2
}

const MAX_SOUNDS: usize = 256;

#[derive(Clone, Debug, Default)]
pub struct ModelAnimations {
    pub animations: Vec<ModelAnimation>,
    pub sounds: Vec<ModelAnimationSound>,
    pub(crate) free_models: Vec<i16>,
}

impl ModelAnimations {
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        marker: usize,
        identifier: i16,
        definition: Arc<AnimationDefinition>,
        frame: usize,
        position: (i32, i32, i32),
        pitch: u16,
        model: i16,
    ) {
        let mut playback = ModelAnimationPlayback::new(&definition.tag, frame);
        playback.frame = frame;
        self.animations.push(ModelAnimation {
            marker,
            identifier,
            definition,
            playback,
            position,
            pitch,
            model,
        });
    }

    pub fn indexes(&self, identifiers: &[i32]) -> Vec<usize> {
        identifiers
            .iter()
            .filter_map(|&id| {
                self.animations
                    .iter()
                    .position(|a| a.identifier != 0 && a.identifier == id as i16)
            })
            .collect()
    }

    pub fn frames(&self) -> Vec<ModelAnimationFrameState> {
        self.animations
            .iter()
            .map(|a| ModelAnimationFrameState {
                marker: a.marker,
                identifier: a.identifier,
                frame: a.playback.frame,
                running: a.playback.active,
                model: a.model,
            })
            .collect()
    }

    pub fn play_sound(&mut self, i: usize, slot: usize) {
        let a = &self.animations[i];
        let Some(sound) = a.definition.tag.sounds[slot] else {
            return;
        };
        let position = a.position;
        if self.sounds.len() >= MAX_SOUNDS {
            self.sounds.remove(0);
        }
        self.sounds.push(ModelAnimationSound {
            animation: i,
            sound,
            position,
        });
    }

    pub fn update(&mut self, world: &mut EngineWorld) -> Result<(), Unmodelled> {
        for i in 0..self.animations.len() {
            let a = &mut self.animations[i];
            if !a.playback.active {
                continue;
            }
            a.playback.tick_counter = a.playback.tick_counter.wrapping_add(1);
            if a.playback.tick_counter <= a.definition.tag.ticks_per_frame {
                continue;
            }
            a.playback.tick_counter = 0;
            if a.definition.tag.frame_count() > 1 {
                self.advance(i, world)?;
            }
        }
        Ok(())
    }

    pub fn advance(&mut self, i: usize, world: &mut EngineWorld) -> Result<(), Unmodelled> {
        log_event(ModelAnimationEvent::Advance(i));
        let a = &mut self.animations[i];
        let old = a.playback.frame;
        let definition = a.definition.clone();
        let slot = a.playback.advance(&definition.tag);
        self.swap_frames(i, old, world)?;
        if let Some(slot) = slot {
            self.play_sound(i, slot);
        }
        Ok(())
    }

    pub fn jump(
        &mut self,
        i: usize,
        frame: usize,
        world: &mut EngineWorld,
    ) -> Result<(), Unmodelled> {
        let a = &mut self.animations[i];
        let old = a.playback.frame;
        a.playback.tick_counter = 0;
        a.playback.frame = frame;
        self.swap_frames(i, old, world)
    }

    fn swap_frames(
        &mut self,
        i: usize,
        old: usize,
        world: &mut EngineWorld,
    ) -> Result<(), Unmodelled> {
        let a = &self.animations[i];
        let new = a.playback.frame;
        log_event(ModelAnimationEvent::Jump {
            animation: i,
            frame: new,
            old,
            flags: datum_flags(&a.playback),
            running: a.playback.active,
            model: a.model,
        });
        if new == old {
            return Ok(());
        }
        let definition = a.definition.clone();
        let (position, pitch, model) = (a.position, a.pitch, a.model);
        let (from, to) = (&definition.frames[old], &definition.frames[new]);
        if from.mode_tag == to.mode_tag && from.permutation != to.permutation {
            return Ok(());
        }
        if model != -1 {
            log_event(ModelAnimationEvent::Delete(model));
            self.delete_model(model, world);
        }
        let model = match to.model.as_ref() {
            Some(frame) => self.new_model(i, frame, to.permutation, position, pitch, world)?,
            None => -1,
        };
        log_event(ModelAnimationEvent::New(model));
        self.animations[i].model = model;
        Ok(())
    }

    pub(crate) fn delete_model(&mut self, model: i16, world: &mut EngineWorld) {
        if let Some(record) = world.models.get(model as usize) {
            let p = record.placement.position;
            let (position, matrix, bounds) = (
                (p[0], p[1], p[2]),
                record.placement.matrix,
                record.placement.bounds,
            );
            let shape = record.shape.clone();
            for &entry in &shape.cells {
                if let Some((x, y)) = delete_model_cell_with_matrix(
                    &mut world.cells,
                    position,
                    &matrix,
                    &shape,
                    entry,
                ) {
                    update_monster_impassability_for_cell(world, x, y);
                }
            }
            clear_model_bounds_between(&mut world.cells, bounds[0], bounds[1]);
            world.cells_changed = true;
            world.models[model as usize].identifier = 0;
        }
        if world.formation_globals.gameplay_version > DELETED_MODEL_UPDATE_ABOVE {
            for a in self.animations.iter_mut().filter(|a| a.model == model) {
                a.model = -1;
            }
        }
        if !self.free_models.contains(&model) {
            self.free_models.push(model);
        }
    }

    fn new_model(
        &mut self,
        animation: usize,
        frame: &FrameModel,
        permutation: i16,
        position: (i32, i32, i32),
        pitch: u16,
        world: &mut EngineWorld,
    ) -> Result<i16, Unmodelled> {
        let index = match self.free_models.iter().enumerate().min_by_key(|&(_, &m)| m) {
            Some((k, _)) => self.free_models.swap_remove(k),
            None => world.models.len() as i16,
        };
        let mut record = WorldModel::new(
            world.new_model_identifier(),
            [position.0, position.1, position.2],
            0,
            pitch,
            permutation,
            frame.geometry.clone(),
            frame.shape.clone(),
            true,
            animation as i16,
        );
        record.geometry_tag = frame.geometry_tag;
        if let Some(tag) = frame.geometry_tag {
            world
                .geometry_surfaces
                .entry(tag)
                .or_insert_with(|| frame.surfaces.to_vec());
        }
        match world.models.get_mut(index as usize) {
            Some(slot) => *slot = record,
            None => world.models.push(record),
        }
        let cells = new_model_cells_for_version(
            &mut world.cells,
            index,
            position,
            0,
            pitch,
            &frame.shape,
            world.formation_globals.gameplay_version,
        );
        world.cells_changed = true;
        for (x, y) in cells.changed {
            update_monster_impassability_for_cell(world, x, y);
        }
        if cells.overlapped {
            self.delete_model(index, world);
            return Ok(-1);
        }
        crate::models::new_model_heights(
            &world.cells,
            &mut world.heights,
            position,
            0,
            pitch,
            &frame.shape,
        );
        Ok(index)
    }
}
