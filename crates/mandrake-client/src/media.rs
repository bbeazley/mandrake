use mandrake_common::FourCC;
use mandrake_tag::mesh::{MediaCoverage, MeshLod};
use mandrake_tag::{MeshTag, Myth2Cell, TagArchive};
use mandrake_view::billboard::{
    BILLBOARD_FLAG_MEDIA_CLIP, BILLBOARD_FLAG_MEDIA_EFFECT, BILLBOARD_FLAG_REFLECTION,
    BILLBOARD_FLAG_SUBMERGED, BillboardInstance,
};
use mandrake_view::media::{self, MediaSurface, MediaWobble, ShadingSlot};
use tracing::{info, warn};

pub const CELL_FLAG_MEDIA: u16 = 0x1800;
pub const CELL_FLAG_REFLECTIVE: u16 = 0x2000;
pub const MESH_GLOBAL_TINT_COLOR: usize = 0xd8;
pub const MESH_GLOBAL_TINT_FRACTION: usize = 0xe0;

pub const OBJECT_FLAG_MEDIA_SURFACE: u32 = 0x4000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreTint {
    pub fraction: i16,
    pub color: [u16; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreRef {
    pub tint: CoreTint,
    pub global: bool,
}

impl CoreRef {
    pub fn from_core_bytes(core: &[u8]) -> Self {
        let be16 = |o: usize| {
            core.get(o..o + 2)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
        };
        Self {
            tint: CoreTint {
                fraction: be16(0x06) as i16,
                color: [be16(0x148), be16(0x14a), be16(0x14c)],
            },
            global: be16(0x14e) & 2 != 0,
        }
    }

    pub fn of_core(archives: &[&TagArchive], id: FourCC) -> Option<Self> {
        archives.iter().rev().find_map(|a| {
            let e = a.find_tag_by_id(FourCC::new(*b"core"), id)?;
            let d = a.get_tag_data(e).ok()?;
            Some(Self::from_core_bytes(&d))
        })
    }

    pub fn resolve(self, global: CoreTint) -> CoreTint {
        if self.global { global } else { self.tint }
    }
}

pub const LOCAL_TIME_LEAD: i32 = 1;

pub fn local_time(tick: u32) -> i32 {
    tick as i32 + LOCAL_TIME_LEAD
}

pub const OBJECT_FLAG_IN_MEDIA: u32 = 0x40;
pub const OBJECT_FLAG_BELOW_MEDIA: u32 = 0x80;

pub const SUBMERGED_WHOLE_TRANSPARENCY: u16 = 0x8000;

pub const PROJECTILE_UNDERWATER_TRANSPARENCY: u16 = 0xd556;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonsterMediaFlags {
    pub position: (i32, i32, i32),
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnderwaterVisibility {
    Visible,
    Hidden,
    Clipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnderwaterGame {
    pub local_team: i16,
    pub mode: i16,
    pub game_options_high: u16,
}

impl UnderwaterGame {
    pub fn monster(&self, team: i16, flags_8: u32) -> UnderwaterVisibility {
        let visible = i32::from(team) == i32::from(self.local_team)
            || flags_8 & 0x1000 != 0
            || self.mode == 0
            || (self.mode == 1 && self.local_team == -1);
        if visible {
            UnderwaterVisibility::Visible
        } else if self.game_options_high
            & mandrake_common::game_options::high(mandrake_common::game_options::TFL)
            != 0
        {
            UnderwaterVisibility::Clipped
        } else {
            UnderwaterVisibility::Hidden
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaDraw {
    Dry,
    Clipped { surface: i16 },
    ClippedShaded { surface: i16 },
    ClippedNoCopy { surface: i16 },
    Translucent { transparency: u16 },
    Whole,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Remains,
    Scenery,
    Monster {
        below: bool,
        visibility: UnderwaterVisibility,
    },
    Projectile {
        top: i32,
        flag_400000: bool,
    },
    Copy,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaObject {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub height: i32,
    pub kind: ObjectKind,
    pub still: bool,
    pub scale: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectMediaDraws {
    pub draw: MediaDraw,
    pub reflection: Option<(i16, bool, i32)>,
}

#[derive(Clone, Debug)]
pub struct MapMedia {
    pub coverage: MediaCoverage,
    pub cell_width: u16,
    pub cell_height: u16,
    pub cells: Vec<Myth2Cell>,
    pub min_cell_height: i32,
    pub reflection_slot: ShadingSlot,
    pub ripple_amplitude: i16,
    pub ripple_frequency: i16,
    pub global_tint: CoreTint,
    pub surface: MediaSurface,
    pub no_reflections: bool,
    pub lod: Option<MeshLod>,
    pub reflection_transparency: u16,
    pub reflective_media_distance: Vec<u32>,
    pub cell_models: Vec<i16>,
    pub placed_models: Vec<PlacedModel>,
    pub effect_sprites: Vec<(usize, FourCC, i16)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaLocalEffect {
    pub group: FourCC,
    pub chance: u16,
    pub tries: i16,
    pub period: i16,
}

impl MediaLocalEffect {
    fn of_definition(d: &mandrake_tag::media::MediaDefinition) -> Option<Self> {
        Some(Self {
            group: d.local_projectile_group?,
            chance: d.local_effect_chance,
            tries: d.local_effect_count,
            period: d.local_effect_period,
        })
    }

    pub fn of_mesh(mesh: &MeshTag, archives: &[&TagArchive]) -> Option<Self> {
        let d = archives.iter().rev().find_map(|a| {
            let e = a.find_tag_by_id(FourCC::new(*b"medi"), mesh.media_tag)?;
            a.get_tag_data(e).ok()
        })?;
        Self::of_definition(
            &mandrake_tag::media::MediaDefinition::from_bytes(mesh.media_tag, &d).ok()?,
        )
    }
}

#[derive(Clone, Debug)]
pub struct PlacedModel {
    pub geometry: std::sync::Arc<mandrake_view::model_surfaces::EngineGeometry>,
    hit_geometry: mandrake_tag::model_hit::HitGeometry,
    placement: mandrake_tag::model_hit::ModelPlacement,
    animation: Option<(
        usize,
        Vec<Option<std::sync::Arc<mandrake_view::model_surfaces::EngineGeometry>>>,
    )>,
}

impl PlacedModel {
    pub fn new(
        geometry: mandrake_view::model_surfaces::EngineGeometry,
        position: [i32; 3],
        yaw: u16,
        pitch: u16,
    ) -> Self {
        let hit_geometry = geometry.hit_geometry();
        let placement =
            mandrake_tag::model_hit::ModelPlacement::new(position, yaw, pitch, &hit_geometry);
        Self {
            geometry: std::sync::Arc::new(geometry),
            hit_geometry,
            placement,
            animation: None,
        }
    }

    pub fn with_animation(
        mut self,
        marker: usize,
        frames: Vec<Option<std::sync::Arc<mandrake_view::model_surfaces::EngineGeometry>>>,
    ) -> Self {
        self.animation = Some((marker, frames));
        self
    }

    pub fn bounds(&self) -> [[i32; 3]; 2] {
        self.placement.bounds
    }

    pub fn height_at(&self, x: i32, y: i32) -> i32 {
        i32::from(mandrake_tag::model_hit::get_model_height_at_position(
            &self.placement,
            &self.hit_geometry,
            x,
            y,
            mandrake_tag::model_hit::SurfaceTest::Gameplay,
        ))
    }
}

fn min_cell_height(cells: &[Myth2Cell]) -> i32 {
    cells
        .iter()
        .map(|c| {
            if c.flags & 0x1800 != 0 {
                i32::from(c.media_height)
            } else {
                i32::from(c.physical_height)
            }
        })
        .min()
        .unwrap_or(0)
}

impl MapMedia {
    pub fn load(mesh_data: &[u8], mesh: &MeshTag, archives: &[&TagArchive]) -> Self {
        let coverage = MediaCoverage::from_mesh_bytes(mesh_data, mesh.cell_width, mesh.cell_height);
        let lod = MeshLod::from_mesh_bytes(mesh_data, mesh.cell_width, mesh.cell_height);
        if lod.is_none() {
            warn!("Mesh has no LOD section; the landscape is drawn at cell resolution");
        }
        let be16 = |d: &[u8], o: usize| {
            d.get(o..o + 2)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
        };
        let global_tint = CoreTint {
            fraction: be16(mesh_data, MESH_GLOBAL_TINT_FRACTION) as i16,
            color: [
                be16(mesh_data, MESH_GLOBAL_TINT_COLOR),
                be16(mesh_data, MESH_GLOBAL_TINT_COLOR + 2),
                be16(mesh_data, MESH_GLOBAL_TINT_COLOR + 4),
            ],
        };
        let medi = archives.iter().rev().find_map(|a| {
            let e = a.find_tag_by_id(FourCC::new(*b"medi"), mesh.media_tag)?;
            a.get_tag_data(e).ok()
        });
        let mesh_flags = mesh_data
            .get(media::MESH_HEADER_FLAGS..media::MESH_HEADER_FLAGS + 4)
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        let wobble = medi
            .as_deref()
            .map(MediaWobble::from_media_bytes)
            .unwrap_or_default();
        let definition = medi
            .as_deref()
            .and_then(|d| mandrake_tag::media::MediaDefinition::from_bytes(mesh.media_tag, d).ok());
        let effects = definition
            .as_ref()
            .map(|def| {
                [
                    mandrake_tag::media::MEDIA_GROUP_PROJECTILE,
                    mandrake_tag::media::MEDIA_GROUP_PROJECTILE_FLAG_10000,
                    mandrake_tag::media::MEDIA_GROUP_MONSTER,
                ]
                .into_iter()
                .map(|t| {
                    (
                        t,
                        mandrake_tag::media::media_effect_sprites(archives, def, t),
                    )
                })
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (t, sprites) in &effects {
            for sp in sprites {
                info!(
                    "Media effect type {}: prgr '{}' proj '{}' -> '{}' sequence {} '{}' ({} frames x {} ticks)",
                    t,
                    sp.group,
                    sp.projectile,
                    sp.collection,
                    sp.sequence,
                    sp.sequence_name.trim(),
                    sp.frames,
                    sp.ticks_per_frame
                );
            }
        }
        let (reflection_slot, ripple_amplitude, ripple_frequency) = match &medi {
            Some(d) => (
                ShadingSlot::media_reflection(d).unwrap_or_default(),
                be16(d, media::MEDIA_RIPPLE_AMPLITUDE) as i16,
                be16(d, media::MEDIA_RIPPLE_FREQUENCY) as i16,
            ),
            None => {
                if mesh.media_tag != FourCC::new([0; 4]) && mesh.media_tag != FourCC::new(*b"none")
                {
                    warn!("Media tag '{}' not found", mesh.media_tag);
                }
                (ShadingSlot::default(), 0, 0)
            }
        };
        let media_pixels: usize = coverage
            .rows
            .iter()
            .enumerate()
            .map(|(y, _)| {
                (0..coverage.width)
                    .filter(|&x| coverage.pixel_is_media(x, y as i32))
                    .count()
            })
            .sum();
        info!(
            "Media '{}': {} coverage pixels, reflection slot {:?}, ripple {} x {}, surface wobble {:?}, mesh flags {:#x}",
            mesh.media_tag,
            media_pixels,
            reflection_slot,
            ripple_amplitude,
            ripple_frequency,
            wobble,
            mesh_flags
        );
        Self {
            coverage,
            cell_width: mesh.cell_width,
            cell_height: mesh.cell_height,
            cells: mesh.cells.clone(),
            min_cell_height: min_cell_height(&mesh.cells),
            reflection_slot,
            ripple_amplitude,
            ripple_frequency,
            global_tint,
            surface: MediaSurface { mesh_flags, wobble },
            no_reflections: definition.as_ref().is_some_and(|d| d.no_reflections()),
            lod,
            reflection_transparency: definition.as_ref().map_or(0, |d| d.reflection_transparency),
            reflective_media_distance: match &definition {
                Some(d) if !d.no_reflections() => {
                    let (w, h) = (usize::from(mesh.cell_width), usize::from(mesh.cell_height));
                    mandrake_view::view_region::reflective_media_distances(w, h, |c, r| {
                        let cell = &mesh.cells[r * w + c];
                        cell.flags & 0x2100 != 0 && cell.media_height > cell.physical_height
                    })
                }
                _ => Vec::new(),
            },
            cell_models: Vec::new(),
            placed_models: Vec::new(),
            effect_sprites: effects
                .iter()
                .flat_map(|(t, sprites)| {
                    sprites
                        .iter()
                        .map(move |sp| (*t, sp.collection, sp.sequence))
                })
                .collect(),
        }
    }

    pub fn claim_model_cells(&mut self, placed_models: Vec<PlacedModel>) -> (Vec<bool>, usize) {
        let (w, h) = (i32::from(self.cell_width), i32::from(self.cell_height));
        self.cell_models = vec![-1; self.cells.len()];
        let mut reflects = Vec::with_capacity(placed_models.len());
        let mut overlapping = 0;
        for (index, placed) in placed_models.iter().enumerate() {
            let rect = mandrake_view::model_surfaces::bounds_cells(
                placed.bounds().map(|c| c.map(|v| v as f32)),
            );
            let mut overlap = false;
            for row in rect[0][1]..=rect[1][1] {
                for col in rect[0][0]..=rect[1][0] {
                    let i = (row.clamp(0, h - 1) * w + col.clamp(0, w - 1)) as usize;
                    if self.cell_models[i] == -1 {
                        self.cell_models[i] = index as i16;
                    } else {
                        overlap = true;
                    }
                }
            }
            overlapping += usize::from(overlap);
            let flags = |col: i32, row: i32| {
                let i = (row.clamp(0, h - 1) * w + col.clamp(0, w - 1)) as usize;
                self.cells.get(i).map_or(0, |c| c.flags)
            };
            reflects.push(mandrake_view::model_surfaces::model_reflects(
                rect,
                &flags,
                &placed.geometry,
            ));
        }
        self.placed_models = placed_models;
        (reflects, overlapping)
    }

    pub fn animation_frame_geometry(
        &self,
        marker: usize,
        frame: usize,
    ) -> Option<&mandrake_view::model_surfaces::EngineGeometry> {
        self.placed_models.iter().find_map(|p| {
            let (m, frames) = p.animation.as_ref().filter(|(m, _)| *m == marker)?;
            debug_assert_eq!(*m, marker);
            frames.get(frame)?.as_deref()
        })
    }

    pub fn placed_model_geometry(
        &self,
        placement: &mandrake_tag::model_hit::ModelPlacement,
    ) -> Option<&mandrake_view::model_surfaces::EngineGeometry> {
        self.placed_models
            .iter()
            .find(|p| {
                p.animation.is_none()
                    && p.placement.position == placement.position
                    && p.placement.matrix == placement.matrix
            })
            .map(|p| &*p.geometry)
    }

    pub fn cell_media_height(&self, x: i32, y: i32) -> i32 {
        let (w, h) = (i32::from(self.cell_width), i32::from(self.cell_height));
        let (cx, cy) = ((x >> 9).clamp(0, w - 1), (y >> 9).clamp(0, h - 1));
        self.cells
            .get((cy * w + cx) as usize)
            .map_or(0, |c| i32::from(c.media_height))
    }

    fn under_cell_model(&self, object: &MediaObject) -> Option<bool> {
        let (cx, cy) = (object.x >> 9, object.y >> 9);
        let i = cy * i32::from(self.cell_width) + cx;
        let model = *self.cell_models.get(usize::try_from(i).ok()?)?;
        if model < 0 {
            return None;
        }
        let placed = self.placed_models.get(model as usize)?;
        Some(object.z < placed.height_at(object.x, object.y))
    }

    fn cell(&self, x: i32, y: i32) -> Option<&Myth2Cell> {
        let (cx, cy) = (x >> 9, y >> 9);
        if cx < 0 || cy < 0 || cx >= i32::from(self.cell_width) || cy >= i32::from(self.cell_height)
        {
            return None;
        }
        self.cells
            .get(cy as usize * usize::from(self.cell_width) + cx as usize)
    }

    pub fn reflection_tint(&self, core: CoreRef) -> [u8; 8] {
        let core = core.resolve(self.global_tint);
        media::build_tinting_shader(
            self.reflection_slot,
            core.fraction,
            core.color,
            self.reflection_slot,
        )
    }

    pub fn ripple(&self, x: i32, y: i32, local_time: i32) -> i32 {
        media::reflection_ripple(
            x,
            y,
            local_time,
            self.ripple_amplitude,
            self.ripple_frequency,
            &mandrake_math::engine::trig_tables().sine,
        )
    }

    pub fn ripple_render(ripple: i32, scale: u16) -> f32 {
        ripple as f32 * f32::from(scale) / 256.0 / 512.0
    }

    pub fn submerged_surface(&self, x: i32, y: i32) -> Option<i16> {
        let cell = self.cell(x, y)?;
        (cell.flags & CELL_FLAG_MEDIA != 0 && self.coverage.point_is_media(x, y))
            .then_some(cell.media_height)
    }

    pub fn object_draws(&self, object: &MediaObject, local_time: i32) -> ObjectMediaDraws {
        let Some(cell) = self.cell(object.x, object.y) else {
            return ObjectMediaDraws {
                draw: MediaDraw::Dry,
                reflection: None,
            };
        };
        let surface = cell.media_height;
        let submerged =
            cell.flags & CELL_FLAG_MEDIA != 0 && self.coverage.point_is_media(object.x, object.y);
        let deep = object.z + object.height < i32::from(surface) - i32::from(self.ripple_amplitude);
        let draw = if !submerged {
            MediaDraw::Dry
        } else {
            match object.kind {
                ObjectKind::Scenery if deep => MediaDraw::Whole,
                ObjectKind::Monster { below, visibility } if deep && below => match visibility {
                    UnderwaterVisibility::Visible => MediaDraw::Whole,
                    UnderwaterVisibility::Hidden => MediaDraw::Hidden,
                    UnderwaterVisibility::Clipped => MediaDraw::ClippedShaded { surface },
                },
                ObjectKind::Projectile { top, flag_400000 } => {
                    self.projectile_draw(object.z, top, flag_400000, surface)
                }
                _ => MediaDraw::Clipped { surface },
            }
        };
        let reflective = !self.no_reflections
            && cell.flags & CELL_FLAG_REFLECTIVE != 0
            && self.under_cell_model(object).unwrap_or(true);
        let reflection = (reflective
            && matches!(
                draw,
                MediaDraw::Dry
                    | MediaDraw::Clipped { .. }
                    | MediaDraw::ClippedShaded { .. }
                    | MediaDraw::ClippedNoCopy { .. }
            ))
        .then(|| {
            let ripple = if object.still {
                self.ripple(object.x, object.y, local_time)
            } else {
                0
            };
            (surface, submerged, ripple)
        });
        ObjectMediaDraws { draw, reflection }
    }

    pub fn instances(
        &self,
        sprite: BillboardInstance,
        draws: &ObjectMediaDraws,
        scale: u16,
        tint: [u8; 8],
    ) -> MediaInstances {
        let (own, copy) = match draws.draw {
            MediaDraw::Dry => (Some(sprite), None),
            MediaDraw::Clipped { surface } => {
                let own = sprite.with_media_clip(world_to_render(surface));
                (
                    Some(own),
                    Some(own.submerged_copy(media::SUBMERGED_COPY_TRANSPARENCY)),
                )
            }
            MediaDraw::ClippedShaded { surface } => (
                Some(
                    sprite
                        .with_media_clip(world_to_render(surface))
                        .with_shade(tint),
                ),
                None,
            ),
            MediaDraw::ClippedNoCopy { surface } => {
                (Some(sprite.with_media_clip(world_to_render(surface))), None)
            }
            MediaDraw::Translucent { transparency } => {
                let mut own = sprite;
                own.tint[3] *= media::surface_opacity(transparency);
                (Some(own), None)
            }
            MediaDraw::Whole => {
                let mut own = sprite.with_shade(tint);
                own.tint[3] *= media::surface_opacity(SUBMERGED_WHOLE_TRANSPARENCY);
                (Some(own), None)
            }
            MediaDraw::Hidden => (None, None),
        };
        let reflection = draws.reflection.map(|(surface, clip, ripple)| {
            let ripple = Self::ripple_render(ripple, scale);
            let mut reflection = sprite.reflection(world_to_render(surface), ripple, clip, tint);
            reflection.tint[3] = self.reflection_opacity(sprite.tint[3], draws.draw);
            reflection
        });
        MediaInstances {
            copy,
            own,
            reflection,
        }
    }

    pub fn surface_effect_instance(
        drawn: &[BillboardInstance],
        ripple: f32,
    ) -> Option<BillboardInstance> {
        const DERIVED: u32 = BILLBOARD_FLAG_REFLECTION | BILLBOARD_FLAG_SUBMERGED;
        drawn
            .iter()
            .any(|i| i.flags & BILLBOARD_FLAG_REFLECTION != 0)
            .then(|| drawn.iter().find(|i| i.flags & DERIVED == 0))
            .flatten()
            .map(|own| {
                let mut surface = own.with_top_extended(ripple);
                surface.flags &= !BILLBOARD_FLAG_MEDIA_CLIP;
                surface.flags |= BILLBOARD_FLAG_MEDIA_EFFECT;
                surface
            })
    }

    fn reflection_opacity(&self, own: f32, draw: MediaDraw) -> f32 {
        if own != 1.0 {
            own
        } else if matches!(draw, MediaDraw::ClippedShaded { .. }) {
            media::surface_opacity(0x8000)
        } else {
            media::surface_opacity(self.reflection_transparency)
        }
    }

    pub fn projectile_draw(&self, z: i32, top: i32, flag_400000: bool, surface: i16) -> MediaDraw {
        let m = i32::from(surface);
        if z > m {
            return MediaDraw::Clipped { surface };
        }
        if flag_400000 {
            return MediaDraw::ClippedNoCopy { surface };
        }
        let half = |v: i32| i32::from((v as i16) / 2);
        let h = m - half(i32::from(self.ripple_amplitude));
        if z + half(top) >= h || h - 0xff > z + top {
            if h - 0x100 > z + top {
                MediaDraw::Hidden
            } else {
                MediaDraw::Clipped { surface }
            }
        } else {
            MediaDraw::Translucent {
                transparency: PROJECTILE_UNDERWATER_TRANSPARENCY,
            }
        }
    }

    pub fn new_object_media_flags(&self, x: i32, y: i32, z: i32, top: i32) -> u32 {
        let Some(cell) = self.cell(x, y) else {
            return 0;
        };
        let m = i32::from(cell.media_height);
        if m <= z || cell.flags & CELL_FLAG_MEDIA == 0 || !self.coverage.point_is_media(x, y) {
            return 0;
        }
        if z + top >= m {
            OBJECT_FLAG_IN_MEDIA
        } else {
            OBJECT_FLAG_IN_MEDIA | OBJECT_FLAG_BELOW_MEDIA
        }
    }

    pub fn moved_monster_media_flags(
        &self,
        x: i32,
        y: i32,
        z: i32,
        top: i32,
        definition_199: u8,
    ) -> u32 {
        let Some(cell) = self.cell(x, y) else {
            return 0;
        };
        let m = i32::from(cell.media_height);
        let in_media = cell.flags & CELL_FLAG_MEDIA != 0 && self.coverage.point_is_media(x, y);
        if !in_media || (z >= m && definition_199 & 0x10 == 0) {
            return 0;
        }
        if m > z + top {
            OBJECT_FLAG_IN_MEDIA | OBJECT_FLAG_BELOW_MEDIA
        } else {
            OBJECT_FLAG_IN_MEDIA
        }
    }

    pub fn update_monster_media(
        &self,
        previous: Option<MonsterMediaFlags>,
        position: (i32, i32, i32),
        top: i32,
        definition_199: u8,
    ) -> MonsterMediaFlags {
        let (x, y, z) = position;
        let flags = match previous {
            None => self.new_object_media_flags(x, y, z, top),
            Some(p) if p.position == position => return p,
            Some(_) => self.moved_monster_media_flags(x, y, z, top, definition_199),
        };
        MonsterMediaFlags { position, flags }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaInstances {
    pub copy: Option<BillboardInstance>,
    pub own: Option<BillboardInstance>,
    pub reflection: Option<BillboardInstance>,
}

fn world_to_render(h: i16) -> f32 {
    f32::from(h) / 512.0
}

pub fn object_top(bounds: mandrake_tag::SequenceWorldBounds, scale: u16) -> i32 {
    i32::from(((i32::from(bounds.top) * i32::from(scale)) >> 8) as i16)
}

pub fn object_height(bounds: mandrake_tag::SequenceWorldBounds, scale: u16) -> i32 {
    let s = |v: i16| ((i32::from(v) * i32::from(scale)) >> 8) as i16;
    i32::from(s(bounds.bottom).max(s(bounds.top)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_tint_uses_the_global_tint_for_flag_2() {
        let mut core = vec![0u8; 0x180];
        core[6..8].copy_from_slice(&0x40i16.to_be_bytes());
        core[0x148..0x14a].copy_from_slice(&0xffffu16.to_be_bytes());
        let global = CoreTint {
            fraction: 7,
            color: [1, 2, 3],
        };
        let own = CoreTint {
            fraction: 0x40,
            color: [0xffff, 0, 0],
        };
        assert_eq!(CoreRef::from_core_bytes(&core).resolve(global), own);
        core[0x14e..0x150].copy_from_slice(&2u16.to_be_bytes());
        assert_eq!(CoreRef::from_core_bytes(&core).resolve(global), global);
    }

    #[test]
    fn local_time_leads_the_tick() {
        assert_eq!(local_time(0), 1);
        assert_eq!(local_time(30), 31);
    }
}
