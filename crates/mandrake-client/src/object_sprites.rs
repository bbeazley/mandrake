use mandrake_common::FourCC;
use mandrake_render::TextureAtlas;
use mandrake_sim::ProjectileView;
pub use mandrake_sim::views::ObjectKind;
use mandrake_tag::MarkerEntry;
use mandrake_tag::collection::INSTANCE_FLAG_VELOCITY_ALIGNED;
use mandrake_view::billboard::{BILLBOARD_FLAG_MIRROR_Y, BillboardInstance};
use mandrake_view::projectile::{EngineViewRows, InstanceRect, VelocityQuad, bob_offset};

use crate::corpse::{object_transparency, object_view, reflected_origin};
use crate::media;
use crate::sprite_manager::{ClientSpriteManager, TEAM_CACHE_FLAG, TeamSprite};
use crate::unit_anim::SequenceVisual;
use crate::world_view::ClientRules;

#[derive(Clone, Debug, PartialEq)]
pub struct DrawnObject {
    pub kind: ObjectKind,
    pub object: i32,
    pub world: [i32; 3],
    pub position: [f32; 3],
    pub velocity: [i16; 3],
    pub facing: u16,
    pub collection: FourCC,
    pub sequence: u8,
    pub frame: i32,
    pub view: u8,
    pub scale: i32,
    pub flags: u32,
    pub hidden: bool,
    pub mirror_x: bool,
    pub mirror_y: bool,
    pub permutation: i16,
    pub player: i16,
    pub shading_slot: u8,
    pub core: Option<FourCC>,
    pub transfer: (u8, u8, u8),
    pub monster: Option<MonsterDraw>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonsterDraw {
    pub id: u32,
    pub bob_height: i32,
    pub colour_row: Option<(FourCC, i16)>,
    pub bounds: mandrake_view::selection::ObjectBounds,
    pub still: bool,
    pub definition_199: u8,
    pub underwater: media::UnderwaterVisibility,
    pub outer_ring: bool,
}

pub fn drawn_point(o: &DrawnObject, rules: &ClientRules, tick: i32) -> [f32; 3] {
    let bob_height = match &o.monster {
        Some(m) => m.bob_height,
        None if rules.sprites_bob => bob_offset(
            o.flags,
            o.object,
            tick,
            &mandrake_math::engine::trig_tables().cosine,
        ),
        None => 0,
    };
    [
        o.position[0],
        o.position[1] + bob_height as f32 / MarkerEntry::WORLD_ONE,
        o.position[2],
    ]
}

pub const OUTER_RING_OPACITY: f32 = (0x7f_u32 as f32) / 255.0;

impl From<&ProjectileView> for DrawnObject {
    fn from(v: &ProjectileView) -> Self {
        Self {
            kind: v.kind,
            object: v.object,
            world: v.world,
            position: [
                v.position.x.to_f32(),
                v.position.y.to_f32(),
                v.position.z.to_f32(),
            ],
            velocity: v.velocity,
            facing: v.facing,
            collection: v.collection,
            sequence: v.sequence,
            frame: i32::from(v.frame),
            view: v.view,
            scale: v.scale,
            flags: v.flags,
            hidden: v.hidden,
            mirror_x: v.mirror_x,
            mirror_y: v.mirror_y,
            permutation: v.permutation,
            player: -1,
            shading_slot: v.shading_slot,
            core: v.core,
            transfer: v.transfer,
            monster: None,
        }
    }
}

pub trait ObjectSprites {
    fn sequence(&self, o: &DrawnObject) -> Option<&SequenceVisual>;
    fn instance_flags(&self, o: &DrawnObject, index: usize) -> u32;
    fn atlas(&self) -> &TextureAtlas;
    fn atlas_index(&self, o: &DrawnObject, base: usize) -> usize;
    fn placed(&self, o: &DrawnObject, base: usize) -> Option<(&TextureAtlas, usize, u32)> {
        Some((self.atlas(), self.atlas_index(o, base), 0))
    }
    fn size_factor(&self, o: &DrawnObject) -> f32 {
        o.scale as f32 / mandrake_view::selection::OBJECT_SCALE_ONE as f32
            * self.rules().sprite_scale
    }
    fn media_object(&self, _o: &DrawnObject, object: media::MediaObject) -> media::MediaObject {
        object
    }
    fn table_core(&self, o: &DrawnObject) -> media::CoreRef;
    fn media(&self) -> Option<&media::MapMedia>;
    fn surface_reflection(&self, _o: &DrawnObject) -> Option<SurfaceReflection> {
        None
    }
    fn rules(&self) -> &'static ClientRules {
        &ClientRules::MYTH2
    }
}

impl ObjectSprites for ClientSpriteManager {
    fn sequence(&self, o: &DrawnObject) -> Option<&SequenceVisual> {
        if o.monster.is_some() {
            return self
                .unit_visuals
                .get(&o.collection)?
                .sequences
                .get(&usize::from(o.sequence));
        }
        self.corpse_sprites
            .sequences
            .get(&(o.collection, o.sequence))
    }

    fn instance_flags(&self, o: &DrawnObject, index: usize) -> u32 {
        if o.monster.is_some() {
            return 0;
        }
        self.corpse_sprites
            .instance_flags
            .get(&(o.collection, o.sequence))
            .and_then(|f| f.get(index))
            .copied()
            .unwrap_or(0)
    }

    fn atlas(&self) -> &TextureAtlas {
        &self.atlas
    }

    fn atlas_index(&self, o: &DrawnObject, base: usize) -> usize {
        if o.monster.is_some() {
            return self.permuted(o.core.as_ref(), base, o.permutation);
        }
        self.permuted(
            o.core
                .as_ref()
                .or_else(|| self.collection_cores.get(&o.collection)),
            base,
            o.permutation,
        )
    }

    fn table_core(&self, o: &DrawnObject) -> media::CoreRef {
        if o.monster.is_some() {
            return self
                .unit_cores
                .get(&o.collection)
                .copied()
                .unwrap_or_default();
        }
        if let Some(core_id) = o.core
            && let Some(&core) = self.core_tints.get(&core_id)
        {
            return core;
        }
        self.collection_core_tints
            .get(&o.collection)
            .copied()
            .unwrap_or_default()
    }

    fn media(&self) -> Option<&media::MapMedia> {
        self.scenery_objects.media.as_deref()
    }

    fn placed(&self, o: &DrawnObject, base: usize) -> Option<(&TextureAtlas, usize, u32)> {
        let Some(m) = &o.monster else {
            return Some((self.atlas(), self.atlas_index(o, base), 0));
        };
        Some(match self.team_sprite(m.colour_row, base) {
            Some(TeamSprite::Cache(index)) => {
                (self.team_cache.atlas.as_ref()?, index, TEAM_CACHE_FLAG)
            }
            Some(TeamSprite::Atlas(index)) => (&self.atlas, index, 0),
            None => (&self.atlas, self.atlas_index(o, base), 0),
        })
    }

    fn size_factor(&self, o: &DrawnObject) -> f32 {
        if o.monster.is_none() {
            return o.scale as f32 / mandrake_view::selection::OBJECT_SCALE_ONE as f32
                * self.rules().sprite_scale;
        }
        crate::scenery::unit_scale_factor(
            u16::try_from(o.scale).unwrap_or(mandrake_view::selection::OBJECT_SCALE_ONE as u16),
            self.unit_visuals
                .get(&o.collection)
                .and_then(|v| v.object)
                .map_or(mandrake_view::selection::OBJECT_SCALE_ONE as u16, |d| {
                    d.scale
                }),
        )
    }

    fn media_object(&self, o: &DrawnObject, object: media::MediaObject) -> media::MediaObject {
        let (Some(m), Some(map)) = (&o.monster, self.media()) else {
            return object;
        };
        let [x, y, z] = o.world;
        let top = i32::from(m.bounds.top);
        let flags = {
            let mut tracked = self.unit_media.borrow_mut();
            let now = map.update_monster_media(
                tracked.get(&m.id).copied(),
                (x, y, z),
                top,
                m.definition_199,
            );
            tracked.insert(m.id, now);
            now.flags
        };
        tracing::debug!(
            "Unit {} media: world ({x}, {y}, {z}) top {top} flags {flags:#x}",
            m.id
        );
        media::MediaObject {
            height: i32::from(m.bounds.bottom.max(m.bounds.top)),
            kind: media::ObjectKind::Monster {
                below: flags & media::OBJECT_FLAG_BELOW_MEDIA != 0,
                visibility: m.underwater,
            },
            still: m.still,
            ..object
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceReflection {
    pub surface: i16,
    pub clip: bool,
    pub tint: [u8; 8],
    pub ripple: i32,
}

pub struct ObjectFrame<'a, S: ?Sized> {
    pub sprites: &'a S,
    pub camera_ground: (i32, i32),
    pub axes: ([f32; 3], [f32; 3], [f32; 3]),
    pub tick: i32,
    pub local_time: i32,
    pub stopped: bool,
}

fn object_core<S: ObjectSprites + ?Sized>(sprites: &S, o: &DrawnObject) -> media::CoreRef {
    if o.shading_slot != 0 && o.monster.is_none() {
        return media::CoreRef::default();
    }
    sprites.table_core(o)
}

#[derive(Clone, Debug, Default)]
pub struct ObjectDraw {
    pub copy: Option<BillboardInstance>,
    pub own: Option<BillboardInstance>,
    pub reflection: Option<BillboardInstance>,
    pub whole: bool,
}

pub fn push_object_instances_with_rows<S: ObjectSprites + ?Sized>(
    v: &DrawnObject,
    frame: &ObjectFrame<'_, S>,
    rows: &EngineViewRows,
    out: &mut Vec<BillboardInstance>,
) {
    let Some(mut draw) = object_draw(v, frame, rows) else {
        return;
    };
    if let Some(m) = &v.monster {
        let Some(own) = draw.own.as_mut() else {
            return;
        };
        if m.outer_ring {
            if !draw.whole {
                own.tint[3] *= OUTER_RING_OPACITY;
            }
            if let Some(r) = draw.reflection.as_mut() {
                r.tint[3] *= OUTER_RING_OPACITY;
            }
        }
    }
    out.extend(draw.copy);
    out.extend(draw.own);
    out.extend(draw.reflection);
}

pub fn object_instances_with<S: ObjectSprites + ?Sized>(
    objects: &[DrawnObject],
    frame: &ObjectFrame<'_, S>,
    mut between: impl FnMut(usize, &DrawnObject, bool, &mut Vec<BillboardInstance>),
    out: &mut Vec<BillboardInstance>,
) {
    let rows = EngineViewRows::from_camera_axes(frame.axes.0, frame.axes.1, frame.axes.2);
    for (index, v) in objects.iter().enumerate() {
        let Some(mut draw) = object_draw(v, frame, &rows) else {
            continue;
        };
        if let Some(m) = &v.monster {
            let Some(own) = draw.own.as_mut() else {
                continue;
            };
            if m.outer_ring {
                if !draw.whole {
                    own.tint[3] *= OUTER_RING_OPACITY;
                }
                if let Some(r) = draw.reflection.as_mut() {
                    r.tint[3] *= OUTER_RING_OPACITY;
                }
            }
        }
        out.extend(draw.copy);
        out.extend(draw.own);
        between(index, v, draw.whole, out);
        out.extend(draw.reflection);
    }
}

struct SpriteChoice<'s> {
    sequence: &'s SequenceVisual,
    sprite: &'s crate::unit_anim::ViewSprite,
    flip: bool,
    instance_flags: u32,
}

fn sprite_choice<'s, S: ObjectSprites + ?Sized>(
    v: &DrawnObject,
    frame: &ObjectFrame<'s, S>,
) -> Option<SpriteChoice<'s>> {
    let sprites = frame.sprites;
    let sequence = sprites.sequence(v)?;
    let (stored, flip) = object_view(
        sequence,
        v.view,
        crate::scenery::camera_relative_angle(
            (v.world[0], v.world[1]),
            v.facing,
            frame.camera_ground,
        ),
        v.mirror_x,
    );
    let frames = sequence.frames.len();
    let frame_index = usize::try_from(v.frame).unwrap_or(0) % frames.max(1);
    let sprite = sequence
        .frames
        .get(frame_index)
        .and_then(|f| f.views.get(stored))
        .and_then(Option::as_ref)?;
    let instance_flags = sprites.instance_flags(v, stored * frames + frame_index);
    Some(SpriteChoice {
        sequence,
        sprite,
        flip,
        instance_flags,
    })
}

pub fn drawn_at_point<S: ObjectSprites + ?Sized>(
    v: &DrawnObject,
    frame: &ObjectFrame<'_, S>,
) -> bool {
    sprite_choice(v, frame).is_none_or(|c| c.instance_flags & INSTANCE_FLAG_VELOCITY_ALIGNED == 0)
}

pub fn object_draw<S: ObjectSprites + ?Sized>(
    v: &DrawnObject,
    frame: &ObjectFrame<'_, S>,
    rows: &EngineViewRows,
) -> Option<ObjectDraw> {
    let sprites = frame.sprites;
    if v.hidden || v.sequence == 0xff || (frame.stopped && v.kind == ObjectKind::Projectile) {
        return None;
    }
    let SpriteChoice {
        sequence,
        sprite,
        flip,
        instance_flags,
    } = sprite_choice(v, frame)?;
    let position = drawn_point(v, sprites.rules(), frame.tick);
    let flip_x = flip ^ v.mirror_x ^ (instance_flags & 1 != 0);
    let mirror_y = v.mirror_y ^ (instance_flags & 2 != 0);
    let scale = u16::try_from(v.scale).unwrap_or(mandrake_view::selection::OBJECT_SCALE_ONE as u16);
    let media = sprites.media();
    let draws = || {
        media.map(|m| {
            let bounds = sequence.object_bounds;
            let object = media::MediaObject {
                x: v.world[0],
                y: v.world[1],
                z: v.world[2],
                height: media::object_height(bounds, scale),
                kind: if v.kind == ObjectKind::Copy {
                    media::ObjectKind::Copy
                } else {
                    media::ObjectKind::Projectile {
                        top: media::object_top(bounds, scale),
                        flag_400000: v.flags & 0x40_0000 != 0,
                    }
                },
                still: v.velocity == [0; 3],
                scale,
            };
            (
                m,
                m.object_draws(&sprites.media_object(v, object), frame.local_time),
            )
        })
    };
    let core = object_core(sprites, v);
    let tint = |m: &media::MapMedia| m.reflection_tint(core);
    let own_tint = |sprite: BillboardInstance| match media {
        _ if v.shading_slot != 0 => crate::scenery::with_slot_tint(
            sprite,
            v.shading_slot,
            sprites.table_core(v),
            media.map_or_else(Default::default, |m| m.global_tint),
            media.map_or_else(Default::default, |m| m.reflection_slot),
        ),
        Some(m) => crate::scenery::with_slot0_tint(sprite, core, m.global_tint),
        None => sprite,
    };
    let (atlas, atlas_index, atlas_flags) = sprites.placed(v, sprite.atlas_index)?;

    if instance_flags & INSTANCE_FLAG_VELOCITY_ALIGNED != 0 {
        let draws = draws();
        let world_size = sprite.world_size.map(|w| w * sprites.rules().sprite_scale);
        let base =
            atlas.create_instance_with_world_size(atlas_index, position, world_size, false)?;
        let base = own_tint(base);
        let rect = InstanceRect::from_sprite(world_size, base.anchor);
        let quad = |reflection| VelocityQuad::new(rows, v.velocity, rect, v.scale, reflection);
        let mut draw = ObjectDraw::default();
        if draws
            .as_ref()
            .is_none_or(|(_, d)| d.draw != media::MediaDraw::Hidden)
        {
            draw.own = Some(quad(false).instance(base, false));
        }
        let surface = match &draws {
            Some((m, d)) => d.reflection.map(|(surface, _, _)| (surface, tint(m))),
            None => sprites.surface_reflection(v).map(|r| (r.surface, r.tint)),
        };
        if let Some((surface, tint)) = surface {
            draw.reflection = Some(
                quad(true)
                    .instance(reflected_origin(base, surface, frame.axes.1), true)
                    .with_shade(tint),
            );
        }
        return Some(draw);
    }

    let k = sprites.size_factor(v);
    let mut inst = atlas.create_instance_with_world_size(
        atlas_index,
        position,
        [sprite.world_size[0] * k, sprite.world_size[1] * k],
        flip_x,
    )?;
    inst.flags |= atlas_flags;
    if mirror_y {
        inst.flags |= BILLBOARD_FLAG_MIRROR_Y;
    }
    let transparency = object_transparency(v.transfer);
    if transparency != 0 {
        inst.tint[3] *= mandrake_view::media::surface_opacity(transparency);
    }
    let inst = own_tint(inst);
    Some(match draws() {
        Some((m, d)) => {
            let i = m.instances(inst, &d, scale, tint(m));
            ObjectDraw {
                copy: i.copy,
                own: i.own,
                reflection: i.reflection,
                whole: d.draw == media::MediaDraw::Whole,
            }
        }
        None => ObjectDraw {
            reflection: sprites.surface_reflection(v).map(|r| {
                inst.reflection(
                    f32::from(r.surface) / MarkerEntry::WORLD_ONE,
                    media::MapMedia::ripple_render(r.ripple, scale),
                    r.clip,
                    r.tint,
                )
            }),
            own: Some(inst),
            ..ObjectDraw::default()
        },
    })
}
