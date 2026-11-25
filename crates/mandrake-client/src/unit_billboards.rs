use crate::corpse;
use crate::interface_state;
use crate::media;
use crate::object_bounds;
use crate::object_sprites::{
    DrawnObject, OUTER_RING_OPACITY, ObjectKind, ObjectSprites, drawn_at_point, drawn_point,
    object_draw, object_instances_with,
};
use crate::overlays::{
    object_overlay, object_selection_overlays, unit_overlay, unit_selection_overlays,
};
use crate::region_view::point_in_region;
use crate::scenery_billboards::{ObjectShadow, placed_shadow};
use crate::shadowgen;
use crate::sprite_manager::ClientSpriteManager;
use crate::unit_anim::{SequenceVisual, UnitAnimator, UnitSpriteVisual};
use crate::world_view::myth2::unit_object;
use crate::world_view::{ClientRules, UnitPose, ViewUnit, WorldView};
use mandrake_tag::{MarkerEntry, MeshTag};
use mandrake_view::projectile::{EngineViewRows, bob_offset, fade_ratio};
use mandrake_view::view_region::ViewRegion;
use mandrake_view::{billboard::BillboardInstance, camera::Camera, selection};

pub(crate) fn drawn_surface_position(
    unit: &(impl UnitPose + ?Sized),
    animator: &UnitAnimator,
) -> [f32; 3] {
    let mut position = animator.drawn_position(unit);
    position[1] += animator.bob_height(unit.id()) as f32 / MarkerEntry::WORLD_ONE;
    position
}

pub(crate) fn drawn_unit_sequence_index(
    unit: &(impl UnitPose + ?Sized),
    visual: &UnitSpriteVisual,
    animator: &UnitAnimator,
) -> Option<usize> {
    if animator.hidden(unit.id()) {
        return None;
    }
    animator
        .drawn_sequence_of(unit.id())
        .filter(|i| visual.sequences.contains_key(i))
        .or_else(|| {
            visual
                .slot_index(unit.state_slot())
                .filter(|i| visual.sequences.contains_key(i))
        })
}

pub(crate) fn drawn_unit_bounds(
    unit: &(impl UnitPose + ?Sized),
    visual: &UnitSpriteVisual,
    sequence: &SequenceVisual,
    animator: &UnitAnimator,
) -> selection::ObjectBounds {
    let scale = unit_object_scale(unit, visual) as i16;
    let of = |index: Option<usize>| {
        let s = index
            .and_then(|i| visual.sequences.get(&i))
            .unwrap_or(sequence);
        selection::ObjectBounds::from_sequence(s.object_bounds, scale)
    };
    let (history, live, fraction) = animator.bounds_sequences(unit.id());
    object_bounds::interpolated_bounds(of(history), of(live), fraction)
}

pub(crate) fn unit_object_scale(unit: &(impl UnitPose + ?Sized), visual: &UnitSpriteVisual) -> u16 {
    unit.object().map_or_else(
        || {
            visual
                .object
                .map_or(selection::OBJECT_SCALE_ONE as u16, |d| d.scale)
        },
        |o| o.scale,
    )
}

pub(crate) fn unit_original_vitality(
    unit: &(impl UnitPose + ?Sized),
    def: &mandrake_tag::ObjectDefinition,
) -> i16 {
    unit.object().map_or(def.vitality as i16, |o| o.vitality)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_unit_billboards(
    mesh_tag: &MeshTag,
    view: &WorldView,
    camera: &Camera,
    animator: &UnitAnimator,
    sprite_mgr: &ClientSpriteManager,
    viewport: (u32, u32),
    iface: &interface_state::InterfaceView,
    region: Option<&ViewRegion>,
) -> (Vec<BillboardInstance>, Vec<ObjectShadow>) {
    let mut instances = Vec::new();
    let mut shadows = Vec::new();
    let projectile_frame = projectile_frame(view, camera, sprite_mgr);
    let parasites = &view.parasites;
    let object_frame = projectile_frame.objects();
    let view_rows = mandrake_view::projectile::EngineViewRows::from_camera_axes(
        object_frame.axes.0,
        object_frame.axes.1,
        object_frame.axes.2,
    );
    shadows.extend(
        view.monsters
            .iter()
            .filter_map(|o| object_shadow(mesh_tag, o, &projectile_frame)),
    );
    object_instances_with(
        &view.monsters,
        &object_frame,
        |_, o, whole, out| {
            let Some(m) = &o.monster else {
                return;
            };
            let Some(unit) = view.unit(m.id) else {
                return;
            };
            let outer_ring = m.outer_ring;
            for (_, p) in parasites.iter().filter(|(host, _)| *host == unit.id) {
                let start = out.len();
                crate::object_sprites::push_object_instances_with_rows(
                    p,
                    &object_frame,
                    &view_rows,
                    out,
                );
                let mut i = start;
                while i < out.len() {
                    if out[i].flags & mandrake_view::billboard::BILLBOARD_FLAG_REFLECTION != 0 {
                        out.remove(i);
                    } else {
                        if outer_ring {
                            out[i].tint[3] *= OUTER_RING_OPACITY;
                        }
                        i += 1;
                    }
                }
            }
            if let Some(colors) = &sprite_mgr.interface_colors
                && let Some(visual) = sprite_mgr.unit_visuals.get(&unit.archetype)
                && let Some(overlay) =
                    unit_overlay(&view.game, unit, visual, sprite_mgr, iface, whole)
            {
                out.extend(unit_selection_overlays(
                    unit,
                    drawn_point(o, &ClientRules::MYTH2, view.tick as i32),
                    visual,
                    m.bounds,
                    colors,
                    camera,
                    viewport,
                    overlay,
                ));
            }
        },
        &mut instances,
    );

    let world_frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    for object in &view.objects {
        let seen = animator.other_object_visibility(
            object.flags,
            object_kind(object),
            (object.world[0], object.world[1]),
        );
        if seen.is_some_and(|v| !v.drawn) {
            continue;
        }
        shadows.extend(object_shadow(mesh_tag, object, &projectile_frame));
        if let Some(region) = region
            && !point_in_region(
                region,
                &world_frame,
                drawn_point(object, &ClientRules::MYTH2, projectile_frame.tick),
            )
            && drawn_at_point(object, &object_frame)
        {
            continue;
        }
        let start = instances.len();
        crate::object_sprites::push_object_instances_with_rows(
            object,
            &object_frame,
            &view_rows,
            &mut instances,
        );
        if object.flags & media::OBJECT_FLAG_MEDIA_SURFACE != 0
            && let Some(map) = sprite_mgr.scenery_objects.media.as_deref()
        {
            let surface = media::MapMedia::surface_effect_instance(
                &instances[start..],
                media_surface_ripple(map, object, projectile_frame.local_time),
            );
            instances.truncate(start);
            instances.extend(surface);
        }
        if seen.is_some_and(|v| !v.full) {
            for instance in &mut instances[start..] {
                instance.tint[3] *= OUTER_RING_OPACITY;
            }
        }
        let framed = instances.len() > start;
        if framed {
            instances.extend(object_frame_overlays(
                object,
                &projectile_frame,
                &view.game,
                iface,
                camera,
                viewport,
            ));
        }
    }

    (instances, shadows)
}

fn object_frame_overlays(
    object: &DrawnObject,
    frame: &corpse::ProjectileFrame<'_>,
    game: &crate::world_view::ViewGame,
    iface: &interface_state::InterfaceView,
    camera: &Camera,
    viewport: (u32, u32),
) -> Vec<BillboardInstance> {
    let sprite_mgr = frame.sprite_mgr;
    let (Some(overlay), Some(colors)) = (
        object_overlay(game, object.object, object.flags, iface),
        sprite_mgr.interface_colors.as_ref(),
    ) else {
        return Vec::new();
    };
    let Some(sequence) = sprite_mgr.sequence(object) else {
        return Vec::new();
    };
    let scale = i16::try_from(object.scale).unwrap_or(selection::OBJECT_SCALE_ONE);
    let bounds = selection::ObjectBounds::from_sequence(sequence.object_bounds, scale);
    object_selection_overlays(
        drawn_point(object, &ClientRules::MYTH2, frame.tick),
        bounds,
        colors,
        camera,
        viewport,
        overlay,
    )
}

fn projectile_frame<'a>(
    view: &WorldView,
    camera: &Camera,
    sprite_mgr: &'a ClientSpriteManager,
) -> corpse::ProjectileFrame<'a> {
    let eye = camera.eye_position();
    corpse::ProjectileFrame {
        sprite_mgr,
        camera_ground: view.ground_point([eye[0], eye[2]]),
        axes: camera.view_axes(),
        tick: view.tick as i32,
        local_time: media::local_time(view.tick),
        stopped: sprite_mgr.projfx_log.is_none() && view.engine_stopped,
    }
}

fn object_kind(object: &DrawnObject) -> u8 {
    match object.kind {
        ObjectKind::Projectile => 3,
        ObjectKind::Dormant => 4,
        ObjectKind::Copy => 5,
    }
}

pub(crate) fn drawn_object_sprites(
    view: &WorldView,
    camera: &Camera,
    animator: &UnitAnimator,
    sprite_mgr: &ClientSpriteManager,
) -> Vec<(i32, BillboardInstance)> {
    let frame = projectile_frame(view, camera, sprite_mgr);
    let object_frame = frame.objects();
    let view_rows = mandrake_view::projectile::EngineViewRows::from_camera_axes(
        object_frame.axes.0,
        object_frame.axes.1,
        object_frame.axes.2,
    );
    let mut out = Vec::new();
    let mut scratch = Vec::new();
    for object in &view.objects {
        let seen = animator.other_object_visibility(
            object.flags,
            object_kind(object),
            (object.world[0], object.world[1]),
        );
        if seen.is_some_and(|v| !v.drawn) {
            continue;
        }
        scratch.clear();
        crate::object_sprites::push_object_instances_with_rows(
            object,
            &object_frame,
            &view_rows,
            &mut scratch,
        );
        out.extend(scratch.drain(..).map(|i| (object.object, i)));
    }
    out
}

fn media_surface_ripple(map: &media::MapMedia, view: &DrawnObject, local_time: i32) -> f32 {
    if view.velocity != [0; 3] {
        return 0.0;
    }
    media::MapMedia::ripple_render(
        map.ripple(view.world[0], view.world[1], local_time),
        u16::try_from(view.scale).unwrap_or(selection::OBJECT_SCALE_ONE as u16),
    )
}

pub(crate) fn object_shadow(
    mesh_tag: &MeshTag,
    view: &DrawnObject,
    frame: &corpse::ProjectileFrame<'_>,
) -> Option<ObjectShadow> {
    if view.monster.is_some() {
        return unit_contact_shadow(mesh_tag, frame.sprite_mgr, view);
    }
    const OBJECT_FLAG_NO_OVERLAY: u32 = 0x20_0000;
    if view.flags & (OBJECT_FLAG_NO_OVERLAY | media::OBJECT_FLAG_MEDIA_SURFACE) != 0
        || view.hidden
        || view.sequence == 0xff
        || (frame.stopped && view.kind == ObjectKind::Projectile)
    {
        return None;
    }
    let sprite_mgr = frame.sprite_mgr;
    let sequence = sprite_mgr
        .corpse_sprites
        .sequences
        .get(&(view.collection, view.sequence))?;
    let frames = sequence.frames.len();
    let visual = sequence
        .frames
        .get(usize::try_from(view.frame).unwrap_or(0) % frames.max(1))?;
    let [x, y, z] = view.world;
    if let Some(surface) = sprite_mgr
        .scenery_objects
        .media
        .as_deref()
        .and_then(|m| m.submerged_surface(x, y))
        && z <= i32::from(surface)
    {
        return None;
    }
    let scale = u16::try_from(view.scale).unwrap_or(selection::OBJECT_SCALE_ONE as u16);
    let shadow = shadowgen::drawn_shadow(
        &sequence.sequence,
        visual,
        &shadowgen::ShadowObject {
            facing: view.facing,
            stored_view: view.view.into(),
            scale,
            flags: view.flags,
        },
    )?;
    let (mode, count, period) = view.transfer;
    let phase = fade_ratio(count, period).unwrap_or(0) as i16;
    let bob_height = bob_offset(
        view.flags,
        view.object,
        frame.tick,
        &mandrake_math::engine::trig_tables().cosine,
    );
    placed_shadow(
        mesh_tag,
        &shadow,
        (x, y, z + bob_height),
        (mode as i8, phase),
    )
}

fn unit_contact_shadow(
    mesh_tag: &MeshTag,
    sprite_mgr: &ClientSpriteManager,
    o: &DrawnObject,
) -> Option<ObjectShadow> {
    let m = o.monster.as_ref()?;
    let sequence = ObjectSprites::sequence(sprite_mgr, o)?;
    let frame = sequence
        .frames
        .get(usize::try_from(o.frame).unwrap_or(0) % sequence.frames.len().max(1))?;
    let shadow = shadowgen::drawn_shadow(
        &sequence.sequence,
        frame,
        &shadowgen::ShadowObject {
            facing: o.facing,
            stored_view: o.view.into(),
            scale: u16::try_from(o.scale).unwrap_or(selection::OBJECT_SCALE_ONE as u16),
            flags: o.flags,
        },
    )?;
    let [x, y, z] = o.world;
    crate::scenery_billboards::object_shadow(
        mesh_tag,
        sprite_mgr.scenery_objects.media.as_deref(),
        &shadow,
        (x, y, z),
        i32::from(m.bounds.top),
    )
}

pub(crate) fn drawn_unit_sprite(
    unit: &ViewUnit,
    view: &WorldView,
    camera: &Camera,
    animator: &UnitAnimator,
    sprite_mgr: &ClientSpriteManager,
) -> Option<BillboardInstance> {
    let o = unit_object(unit, &view.game, animator, sprite_mgr)?;
    drawn_monster_sprite(&o, view, camera, sprite_mgr)
}

pub(crate) fn drawn_monster_sprite(
    o: &DrawnObject,
    view: &WorldView,
    camera: &Camera,
    sprite_mgr: &ClientSpriteManager,
) -> Option<BillboardInstance> {
    let frame = projectile_frame(view, camera, sprite_mgr);
    let frame = frame.objects();
    let rows = EngineViewRows::from_camera_axes(frame.axes.0, frame.axes.1, frame.axes.2);
    object_draw(o, &frame, &rows)?.own
}
