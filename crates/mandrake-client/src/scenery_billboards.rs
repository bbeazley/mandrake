use crate::app::GpuWindowState;
use crate::clock::Instant;
use crate::media;
use crate::object_history;
use crate::scenery;
use crate::sprite_manager::ClientSpriteManager;
use crate::unit_anim::ShadowSprite;
use mandrake_tag::{ConnectorTag, MarkerEntry, MeshConnectorList, MeshTag};
use mandrake_view::landscape::{
    OverlayMode, ShadeOverlay, map_overlay_intensity, map_overlay_intensity_of,
};
use mandrake_view::{
    billboard::{BILLBOARD_FLAG_SCREEN_ALIGNED, BillboardInstance},
    connector::{ConnectorStrip, connector_chain_spans, connector_span_instances},
};
use tracing::{debug, info, trace};

#[derive(Clone, Copy)]
pub(crate) struct SceneryView<'a> {
    pub(crate) camera: (i32, i32),
    pub(crate) animator: &'a scenery::SceneryAnimator,
    pub(crate) history_shape: bool,
}

impl<'a> SceneryView<'a> {
    pub(crate) fn new(
        mesh_tag: &MeshTag,
        camera_eye: [f32; 3],
        animator: &'a scenery::SceneryAnimator,
        fraction: i32,
    ) -> Self {
        Self {
            camera: scenery::engine_ground_point(mesh_tag, [camera_eye[0], camera_eye[2]]),
            animator,
            history_shape: object_history::shows_history_shape(fraction),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneryPart {
    All,
    Static,
    Dynamic,
}

impl SceneryPart {
    fn includes(self, dynamic: bool) -> bool {
        match self {
            Self::All => true,
            Self::Static => !dynamic,
            Self::Dynamic => dynamic,
        }
    }
}

fn scenery_object_shape<'a>(
    mesh_tag: &'a MeshTag,
    sprite_mgr: &'a ClientSpriteManager,
    index: usize,
    history_shape: bool,
) -> Option<(
    &'a MarkerEntry,
    &'a scenery::SceneryVisual,
    scenery::SceneryObject,
)> {
    let marker = mesh_tag.markers.get(index)?;
    let visual = sprite_mgr
        .scenery_visuals
        .get(&mesh_tag.palette_tag_for_marker(marker)?)?;
    let object = sprite_mgr
        .scenery_objects
        .drawn(index, visual.base_scale, history_shape)?;
    Some((marker, visual.shape(&object)?, object))
}

fn scenery_instance_from(
    mesh_tag: &MeshTag,
    sprite_mgr: &ClientSpriteManager,
    index: usize,
    marker: &MarkerEntry,
    visual: &scenery::SceneryVisual,
    object: scenery::SceneryObject,
    view: SceneryView<'_>,
) -> Option<(
    Option<scenery::ScenerySprite>,
    Option<ShadowSprite>,
    scenery::SceneryObject,
)> {
    let frame = view.animator.frame(index, &object);
    let relative =
        scenery::camera_relative_angle((marker.x, marker.y), marker.yaw as u16, view.camera);
    let drawn_view = visual.view(&object, relative);
    let shadow = visual.frame(frame).and_then(|f| {
        crate::shadowgen::drawn_shadow(
            &visual.sequence,
            f,
            &crate::shadowgen::ShadowObject {
                facing: marker.yaw as u16,
                stored_view: object.view,
                scale: object.scale,
                flags: 0,
            },
        )
    });
    let scen = mesh_tag.palette_tag_for_marker(marker)?;
    let core = sprite_mgr.scenery_core_ids.get(&scen);
    let sprite = visual.sprite(&object, frame, drawn_view).map(|mut s| {
        s.atlas_index = object
            .flag_team
            .and_then(|team| sprite_mgr.flag_sprite(scen, s.atlas_index, team))
            .unwrap_or_else(|| sprite_mgr.permuted(core, s.atlas_index, object.permutation));
        s
    });
    Some((sprite, shadow, object))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ObjectShadow {
    x: i32,
    y: i32,
    intensity: u16,
    shadow: ShadowSprite,
}

impl ObjectShadow {
    pub(crate) fn overlay(&self) -> ShadeOverlay<'_> {
        ShadeOverlay {
            x: self.x,
            y: self.y,
            pixels: &self.shadow.pixels,
            width: self.shadow.width as i16,
            height: self.shadow.height as i16,
            row_bytes: self.shadow.width as i16,
            intensity: self.intensity,
            mode: OverlayMode::from_descriptor_flags(self.shadow.flags),
        }
    }
}

pub(crate) fn object_shadow(
    mesh_tag: &MeshTag,
    map_media: Option<&media::MapMedia>,
    shadow: &ShadowSprite,
    (x, y, z): (i32, i32, i32),
    top: i32,
) -> Option<ObjectShadow> {
    if let Some(surface) = map_media.and_then(|m| m.submerged_surface(x, y))
        && z + top <= i32::from(surface)
    {
        return None;
    }
    placed_shadow(mesh_tag, shadow, (x, y, z), (0, 0))
}

pub(crate) fn placed_shadow(
    mesh_tag: &MeshTag,
    shadow: &ShadowSprite,
    (x, y, z): (i32, i32, i32),
    (mode, phase): (i8, i16),
) -> Option<ObjectShadow> {
    let height = z - mesh_tag.mesh_height(x, y, mandrake_tag::MeshHeightMode::Surface);
    ObjectShadow::above_ground(shadow, (x, y), height, (mode, phase))
}

impl ObjectShadow {
    pub(crate) fn above_ground(
        shadow: &ShadowSprite,
        (x, y): (i32, i32),
        height: i32,
        (mode, phase): (i8, i16),
    ) -> Option<Self> {
        let (intensity, shift) = map_overlay_intensity(height, shadow.flags & 1 != 0, mode, phase)?;
        Some(Self::at((x + shift, y + shift), intensity, shadow))
    }

    pub(crate) fn at((x, y): (i32, i32), intensity: u16, shadow: &ShadowSprite) -> Self {
        Self {
            x: x - (shadow.offset_x >> 10),
            y: y - (shadow.offset_y >> 10),
            intensity,
            shadow: shadow.clone(),
        }
    }
}

pub(crate) fn local_projectile_overlay(
    shadow: &ShadowSprite,
    (x, y): (i32, i32),
    height: i32,
    opacity: u16,
) -> Option<ObjectShadow> {
    let (intensity, shift) =
        map_overlay_intensity_of(opacity, height, shadow.flags & 1 != 0, 0, 0)?;
    Some(ObjectShadow {
        x: x + shift - (shadow.offset_x >> 10),
        y: y + shift - (shadow.offset_y >> 10),
        intensity,
        shadow: shadow.clone(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReflectionRipple {
    index: usize,
    x: i32,
    y: i32,
    scale: u16,
}

pub(crate) fn apply_reflection_ripples(
    instances: &mut [BillboardInstance],
    ripples: &[ReflectionRipple],
    media: &media::MapMedia,
    local_time: i32,
) {
    for r in ripples {
        if let Some(inst) = instances.get_mut(r.index) {
            inst.media[2] =
                media::MapMedia::ripple_render(media.ripple(r.x, r.y, local_time), r.scale);
        }
    }
}

pub(crate) fn generate_scenery_billboards(
    mesh_tag: &MeshTag,
    sprite_mgr: &ClientSpriteManager,
    local_time: i32,
    view: SceneryView<'_>,
    part: SceneryPart,
) -> (
    Vec<BillboardInstance>,
    Vec<ReflectionRipple>,
    Vec<ObjectShadow>,
) {
    let mut instances = Vec::new();
    let mut ripples = Vec::new();
    let mut shadows = Vec::new();
    let map_media = sprite_mgr.scenery_objects.media.as_deref();

    let mut emit = |index: usize, filter_part: bool| {
        let in_history = sprite_mgr.scenery_objects.history.contains_key(&index);
        let Some((marker, visual, object)) =
            scenery_object_shape(mesh_tag, sprite_mgr, index, view.history_shape)
                .filter(|(_, v, o)| !filter_part || part.includes(in_history || v.is_dynamic(o)))
        else {
            return;
        };
        let Some((sprite, shadow, object)) =
            scenery_instance_from(mesh_tag, sprite_mgr, index, marker, visual, object, view)
        else {
            return;
        };
        let (wx, wy, wz) = mesh_tag.marker_world_position(marker, 1.0);
        let top = media::object_top(visual.object_bounds, object.scale);
        if let Some(s) = shadow.and_then(|s| {
            object_shadow(mesh_tag, map_media, &s, (marker.x, marker.y, marker.z), top)
        }) {
            shadows.push(s);
        }

        if let Some(sprite) = sprite
            && let Some(inst) = sprite_mgr.atlas.create_instance_with_world_size(
                sprite.atlas_index,
                [wx, wy, wz],
                sprite.world_size,
                sprite.flip_x,
            )
        {
            debug_assert!(inst.flags & BILLBOARD_FLAG_SCREEN_ALIGNED != 0);
            trace!(
                "Scenery marker {} ({:?}): scale {:#x} frame {} view {} at ({wx:.2}, {wy:.2}, {wz:.2})",
                marker.marker_id,
                mesh_tag.palette_tag_for_marker(marker),
                object.scale,
                object.frame,
                object.view
            );
            let Some(map_media) = map_media else {
                instances.push(inst);
                return;
            };
            let tag = mesh_tag.palette_tag_for_marker(marker);
            let (x, y, z) = (
                marker.x,
                marker.y,
                (wy * MarkerEntry::WORLD_ONE).round() as i32,
            );
            let obj = media::MediaObject {
                x,
                y,
                z,
                height: media::object_height(visual.object_bounds, object.scale),
                kind: if object.kind == 0 {
                    media::ObjectKind::Remains
                } else {
                    media::ObjectKind::Scenery
                },
                still: true,
                scale: object.scale,
            };
            let draws = map_media.object_draws(&obj, local_time);
            let core = tag
                .and_then(|t| sprite_mgr.scenery_cores.get(&t))
                .copied()
                .unwrap_or_default();
            let media::MediaInstances {
                copy,
                own,
                reflection,
            } = map_media.instances(
                scenery::with_slot0_tint(inst, core, map_media.global_tint),
                &draws,
                object.scale,
                map_media.reflection_tint(core),
            );
            instances.extend(copy);
            instances.extend(own);
            if let Some(r) = reflection {
                ripples.push(ReflectionRipple {
                    index: instances.len(),
                    x,
                    y,
                    scale: object.scale,
                });
                instances.push(r);
            }
        }
    };

    if part == SceneryPart::Dynamic {
        let generation = sprite_mgr.scenery_objects.generation;
        let rebuild = sprite_mgr
            .dynamic_scenery
            .borrow()
            .as_ref()
            .is_none_or(|(g, _)| *g != generation);
        if rebuild {
            let mut indices = Vec::new();
            for index in scenery::scenery_marker_indices(mesh_tag) {
                let in_history = sprite_mgr.scenery_objects.history.contains_key(&index);
                if in_history
                    || scenery_object_shape(mesh_tag, sprite_mgr, index, false)
                        .is_some_and(|(_, v, o)| v.is_dynamic(&o))
                {
                    indices.push(index);
                }
            }
            *sprite_mgr.dynamic_scenery.borrow_mut() = Some((generation, indices));
        }
        let cache = sprite_mgr.dynamic_scenery.borrow();
        if let Some((_, indices)) = cache.as_ref() {
            for &index in indices {
                emit(index, false);
            }
        }
    } else {
        for index in scenery::scenery_marker_indices(mesh_tag) {
            emit(index, true);
        }
    }

    if part != SceneryPart::Dynamic {
        debug!(
            "Constructed {} {part:?} scenery billboard instances from original tags ({} reflections)",
            instances.len(),
            ripples.len()
        );
    }
    (instances, ripples, shadows)
}

pub(crate) fn refresh_static_scenery(
    gpu: &mut GpuWindowState,
    mesh_tag: &MeshTag,
    connectors: &MeshConnectorList,
    sim: &mandrake_sim::Simulation,
    local_time: i32,
    camera_eye: [f32; 3],
) {
    let start = Instant::now();
    let (mut billboards, ripples, shadows) = generate_scenery_billboards(
        mesh_tag,
        &gpu.sprite_mgr,
        local_time,
        SceneryView::new(mesh_tag, camera_eye, &gpu.scenery_animator, 0),
        SceneryPart::Static,
    );
    billboards.extend(generate_connector_billboards(
        mesh_tag,
        connectors,
        &gpu.sprite_mgr,
    ));
    if let Some((texels, _)) = &gpu.landscape {
        gpu.scenery_shadow_rects.extend(changed_shadow_rects(
            &gpu.static_shadows,
            &shadows,
            (texels.width, texels.height),
        ));
    }
    gpu.scenery_billboards = billboards;
    gpu.scenery_ripples = ripples;
    gpu.static_shadows = shadows;
    gpu.static_lights =
        crate::lighting::extract_static_torch_lights(&gpu.static_shadows, sim, mesh_tag);
    info!(
        "Scenery changed: {} static billboards rebuilt in {:?}",
        gpu.scenery_billboards.len(),
        start.elapsed()
    );
}

fn changed_shadow_rects(
    old: &[ObjectShadow],
    new: &[ObjectShadow],
    (map_w, map_h): (u32, u32),
) -> Vec<mandrake_view::landscape::PixelRect> {
    let old_set: std::collections::HashSet<&ObjectShadow> = old.iter().collect();
    let new_set: std::collections::HashSet<&ObjectShadow> = new.iter().collect();
    let gone = old.iter().filter(|s| !new_set.contains(s));
    let came = new.iter().filter(|s| !old_set.contains(s));
    gone.chain(came)
        .filter_map(|s| s.overlay().pixel_rect(map_w, map_h))
        .collect()
}

fn connector_attach_point(
    mesh_tag: &MeshTag,
    sprite_mgr: &ClientSpriteManager,
    definition: &ConnectorTag,
    marker_identifier: i16,
) -> Option<[f32; 3]> {
    if marker_identifier == -1 {
        return None;
    }
    let index = scenery::scenery_marker_indices(mesh_tag)
        .find(|&i| mesh_tag.markers[i].marker_id == marker_identifier)?;
    let (marker, visual, object) = scenery_object_shape(mesh_tag, sprite_mgr, index, false)?;
    let scaled = |bound: i16| ((i32::from(bound) * i32::from(object.scale)) >> 8) as i16;
    let bounds = visual.object_bounds;
    let sum = i32::from(scaled(bounds.bottom)) + i32::from(scaled(bounds.top));
    let height = (sum * i32::from(definition.height_fraction)) >> 8;

    let (wx, _my, wz) = mesh_tag.marker_world_position(marker, 1.0);
    let wy = (marker.z + height) as f32 / MarkerEntry::WORLD_ONE;
    Some([wx, wy, wz])
}

pub(crate) fn generate_connector_billboards(
    mesh_tag: &MeshTag,
    connectors: &MeshConnectorList,
    sprite_mgr: &ClientSpriteManager,
) -> Vec<BillboardInstance> {
    let mut instances = Vec::new();
    let Some(visual) = &sprite_mgr.connector_visual else {
        return instances;
    };
    let Some(frame) = sprite_mgr.atlas.frame(visual.frame_index) else {
        return instances;
    };

    let mut span_count = 0usize;
    for chain in &connectors.connectors {
        let points: Vec<Option<[f32; 3]>> = chain
            .marker_identifiers
            .iter()
            .map(|&id| connector_attach_point(mesh_tag, sprite_mgr, &visual.definition, id))
            .collect();

        for (i, j) in connector_chain_spans(&points) {
            let (Some(p0), Some(p1)) = (points[i], points[j]) else {
                continue;
            };
            let strip =
                ConnectorStrip::from_atlas_frame(frame, visual.rect, MarkerEntry::WORLD_ONE);
            let d = ((p1[0] - p0[0]).powi(2) + (p1[1] - p0[1]).powi(2) + (p1[2] - p0[2]).powi(2))
                .sqrt();
            let segments = visual.definition.segment_count(d);
            instances.extend(connector_span_instances(p0, p1, segments, &strip));
            span_count += 1;
        }
    }

    info!(
        "Constructed {} connector wire spans ({} strip quads) from {} mesh connector chains",
        span_count,
        instances.len(),
        connectors.connectors.len()
    );
    instances
}
