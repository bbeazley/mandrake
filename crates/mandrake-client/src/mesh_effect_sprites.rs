use std::collections::BTreeMap;

use mandrake_common::FourCC;
use mandrake_render::billboard::TextureAtlas;
use mandrake_sim::Simulation;
use mandrake_sim::mesh_effects::MeshEffectDatum;
use mandrake_sim::projectile_groups::ProjectileCatalog;
use mandrake_tag::MeshTag;
use mandrake_view::billboard::BillboardInstance;
use mandrake_view::camera::Camera;
use mandrake_view::media::{ShadingSlot, build_tinting_shader};
use mandrake_view::model_surfaces::WorldFrame;
use mandrake_view::view_region::ViewRegion;

use crate::lightning_draw::SegmentScreen;
use crate::media::{self, CoreRef};
use crate::projectile_local::{LocalSpriteSource, LocalSprites, TagPayload, in_cell_region};
use crate::sprite_manager::ClientSpriteManager;

pub const MESH_EFFECT_FLAG_SPRITES: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshEffectLook {
    pub flags: u32,
    pub radius: i16,
    pub speed: i16,
    pub initial_distance: i16,
    pub collection_reference: FourCC,
    pub sequence: i16,
    pub segments: i16,
    pub width: i16,
    pub height: i16,
}

impl MeshEffectLook {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 0x1c {
            return None;
        }
        let w = |o: usize| i16::from_be_bytes([data[o], data[o + 1]]);
        Some(Self {
            flags: u32::from_be_bytes(data[0..4].try_into().ok()?),
            radius: w(0x06),
            speed: w(0x0a),
            initial_distance: w(0x0c),
            collection_reference: FourCC::new(data[0x10..0x14].try_into().ok()?),
            sequence: w(0x14),
            segments: w(0x16),
            width: w(0x18),
            height: w(0x1a),
        })
    }

    fn sequence_key(&self) -> Option<(FourCC, u8)> {
        Some((self.collection_reference, u8::try_from(self.sequence).ok()?))
    }

    pub fn inner_radius(&self, time: i32, fraction: i32) -> i32 {
        let t = time.wrapping_mul(0x10000).wrapping_add(fraction);
        (t.wrapping_mul(i32::from(self.speed)) >> 16) + i32::from(self.initial_distance)
    }

    pub fn transparency(&self, r: i32) -> u16 {
        if self.radius == 0 {
            return 0x8000;
        }
        let step = (i32::from(r as i16) << 8) / i32::from(self.radius);
        ((i32::from(step as i16) * 0x80 + 0x8000).clamp(0, 0xffff)) as u16
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingQuad {
    pub definition: FourCC,
    pub frame: usize,
    pub corners: [[i32; 3]; 4],
    pub transparency: u16,
}

#[derive(Clone, Debug, Default)]
pub struct MeshEffectSprites {
    pub looks: BTreeMap<FourCC, MeshEffectLook>,
    timing: BTreeMap<(FourCC, u8), (u16, i16)>,
}

impl MeshEffectSprites {
    pub fn for_catalog(
        catalog: &ProjectileCatalog,
        tag: &TagPayload<'_>,
    ) -> (Self, Vec<LocalSpriteSource>) {
        let mut looks = BTreeMap::new();
        let mut sources = Vec::new();
        let mut ids: Vec<FourCC> = catalog.mesh_effects.keys().copied().collect();
        ids.sort_unstable_by_key(|i| i.as_u32());
        for id in ids {
            let Some(look) = tag(b"meef", id).and_then(|d| MeshEffectLook::from_bytes(&d)) else {
                continue;
            };
            if look.flags & MESH_EFFECT_FLAG_SPRITES == 0 {
                continue;
            }
            let Some((reference, sequence)) = look.sequence_key() else {
                continue;
            };
            looks.insert(id, look);
            let core = tag(b"core", reference);
            let collection = core
                .as_ref()
                .and_then(|c| Some(FourCC::new(c.get(0..4)?.try_into().ok()?)))
                .filter(|c| !matches!(u32::from_be_bytes(c.0), 0 | 0xffff_ffff))
                .unwrap_or(reference);
            let core = core
                .as_deref()
                .map_or_else(Default::default, CoreRef::from_core_bytes);
            let source = LocalSpriteSource {
                reference,
                collection,
                sequence,
                stain_sequence: -1,
                core,
            };
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        (
            Self {
                looks,
                timing: BTreeMap::new(),
            },
            sources,
        )
    }

    pub fn set_timing(&mut self, key: (FourCC, u8), frames: u16, ticks: i16) {
        if self.looks.values().any(|l| l.sequence_key() == Some(key)) {
            self.timing.insert(key, (frames, ticks));
        }
    }

    fn frame(&self, key: (FourCC, u8), time: i32) -> usize {
        let Some(&(frames, ticks)) = self.timing.get(&key) else {
            return 0;
        };
        if frames == 0 || time <= 0 {
            return 0;
        }
        let period = i64::from(ticks.max(0)) + 1;
        ((i64::from(time) / period) % i64::from(frames)) as usize
    }

    pub fn surfaces(
        &self,
        effects: &[Option<MeshEffectDatum>],
        region: Option<&ViewRegion>,
        fraction: i32,
        mesh: (i32, i32),
    ) -> Vec<RingQuad> {
        let trig = mandrake_math::engine::trig_tables();
        let (max_x, max_y) = (mesh.0 * 0x200 - 1, mesh.1 * 0x200 - 1);
        let mut out = Vec::new();
        for e in effects.iter().flatten() {
            let Some(look) = self.looks.get(&e.definition) else {
                continue;
            };
            let (cx, cy) = (i32::from(e.center_cell_x), i32::from(e.center_cell_y));
            if region.is_some_and(|g| !in_cell_region(g, cx, cy)) {
                continue;
            }
            let center = [cx << 9, cy << 9];
            let r = look.inner_radius(e.time, fraction);
            let transparency = look.transparency(r);
            let outer = i32::from(look.width) + r;
            let z = i32::from(look.height) + e.center_height;
            let frame = look.sequence_key().map_or(0, |k| self.frame(k, e.time));
            let n = i32::from(look.segments);
            if n <= 0 {
                continue;
            }
            let point = |i: i32| {
                let a = (((((i << 16) / n) & 0xffff) + 0x20) >> 6 & 0x3ff) as usize;
                let (c, s) = (i32::from(trig.cosine[a]), i32::from(trig.sine[a]));
                let at = |radius: i32| {
                    [
                        ((radius.wrapping_mul(c) >> 14) + center[0]).clamp(0, max_x),
                        ((radius.wrapping_mul(s) >> 14) + center[1]).clamp(0, max_y),
                        z,
                    ]
                };
                (at(r), at(outer))
            };
            let points: Vec<_> = (0..n).map(point).collect();
            for i in 0..n as usize {
                let (inner0, outer0) = points[i];
                let (inner1, outer1) = points[(i + 1) % n as usize];
                out.push(RingQuad {
                    definition: e.definition,
                    frame,
                    corners: [inner0, outer0, outer1, inner1],
                    transparency,
                });
            }
        }
        out
    }
}

#[allow(clippy::too_many_arguments)]
pub fn instances(
    draw: &MeshEffectSprites,
    quads: &[RingQuad],
    sprites: &LocalSprites,
    atlas: &TextureAtlas,
    frame: WorldFrame,
    screen: &SegmentScreen<'_>,
    permuted: &dyn Fn(FourCC, usize) -> usize,
    global_tint: media::CoreTint,
) -> Vec<BillboardInstance> {
    let mut out = Vec::with_capacity(quads.len());
    for q in quads {
        if q.corners.iter().any(|&c| (screen.depth)(c) >= -0x22)
            || !q.corners.iter().any(|&c| (screen.on_screen)(c))
        {
            continue;
        }
        let Some(key) = draw.looks.get(&q.definition).and_then(|l| l.sequence_key()) else {
            continue;
        };
        let Some((bitmap, core)) = sprites.bitmap(&key, q.frame) else {
            continue;
        };
        let Some(atlas_frame) = atlas.frame(permuted(key.0, bitmap.atlas_index)) else {
            continue;
        };
        let corners = q.corners.map(|c| frame.from_engine(c.map(|v| v as f32)));
        let mut instance =
            BillboardInstance::world_quad(corners, atlas_frame.uv_min, atlas_frame.uv_max);
        instance.tint[3] *= mandrake_view::media::surface_opacity(q.transparency);
        let tint = core.resolve(global_tint);
        if tint.fraction != 0 {
            let slot = ShadingSlot {
                kind: -1,
                ..ShadingSlot::default()
            };
            instance =
                instance.with_shade(build_tinting_shader(slot, tint.fraction, tint.color, slot));
        }
        out.push(instance);
    }
    out
}

pub(crate) fn mesh_effect_billboards(
    sprite_mgr: &ClientSpriteManager,
    sim: &Simulation,
    camera: &Camera,
    mesh_tag: &MeshTag,
    region: Option<&ViewRegion>,
    fraction: i32,
    aspect: f32,
) -> Vec<BillboardInstance> {
    let world = &sim.engine;
    let draw = &sprite_mgr.mesh_effect_sprites;
    if world.mesh_effects.is_empty() || draw.looks.is_empty() {
        return Vec::new();
    }
    let quads = draw.surfaces(
        &world.mesh_effects,
        region,
        fraction,
        (
            i32::from(mesh_tag.cell_width),
            i32::from(mesh_tag.cell_height),
        ),
    );
    if quads.is_empty() {
        return Vec::new();
    }
    let frame = WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    SegmentScreen::with_camera(frame, camera, aspect, |screen, _| {
        instances(
            draw,
            &quads,
            &sprite_mgr.local_projectile_sprites,
            &sprite_mgr.atlas,
            frame,
            screen,
            &|reference, base| sprite_mgr.permuted(Some(&reference), base, 0),
            sprite_mgr
                .scenery_objects
                .media
                .as_deref()
                .map_or_else(Default::default, |m| m.global_tint),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAFD: [u8; 0x24] = [
        0, 0, 0, 2, 0x00, 0xcd, 0x04, 0x00, 0x01, 0x00, 0x00, 0x33, 0, 0, 0xff, 0xff, b'i', b'n',
        b'm', b'e', 0x00, 0x10, 0x00, 0x20, 0x02, 0x00, 0x02, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    fn wafd() -> (FourCC, MeshEffectSprites) {
        let id = FourCC::new(*b"wafd");
        let look = MeshEffectLook::from_bytes(&WAFD).unwrap();
        let mut draw = MeshEffectSprites::default();
        draw.looks.insert(id, look);
        (id, draw)
    }

    fn datum(id: FourCC, cell: (i16, i16), time: i32) -> Option<MeshEffectDatum> {
        Some(MeshEffectDatum {
            definition: id,
            center_cell_x: cell.0,
            center_cell_y: cell.1,
            center_height: 1000,
            time,
            sequence_frame: 0,
            sequence_tick: 0,
        })
    }

    #[test]
    fn the_wafd_look_is_read_from_its_tag() {
        let look = MeshEffectLook::from_bytes(&WAFD).unwrap();
        assert_eq!(look.flags & MESH_EFFECT_FLAG_SPRITES, 2);
        assert_eq!(
            (look.radius, look.speed, look.initial_distance),
            (0x400, 0x33, 0)
        );
        assert_eq!(look.collection_reference, FourCC::new(*b"inme"));
        assert_eq!(
            (look.sequence, look.segments, look.width, look.height),
            (16, 32, 0x200, 0x200)
        );
        assert_eq!(look.transparency(0), 0x8000);
        assert_eq!(look.transparency(0x200), 0xc000);
        assert_eq!(look.transparency(0x400), 0xffff);
        assert_eq!(look.inner_radius(4, 0), 4 * 0x33);
        assert_eq!(look.inner_radius(4, 0x8000), (4 * 0x33) + 0x33 / 2);
    }

    #[test]
    fn a_ring_is_its_quads_around_the_centre() {
        let (id, draw) = wafd();
        let quads = draw.surfaces(&[None, datum(id, (157, 25), 10)], None, 0, (200, 200));
        assert_eq!(quads.len(), 32);
        let (cx, cy) = (157 << 9, 25 << 9);
        let r = 10 * 0x33;
        assert_eq!(quads[0].corners[0], [cx + r, cy, 1000 + 0x200]);
        assert_eq!(quads[0].corners[1], [cx + r + 0x200, cy, 1000 + 0x200]);
        assert_eq!(quads[8].corners[0], [cx, cy + r, 1000 + 0x200]);
        for i in 0..32 {
            let next = &quads[(i + 1) % 32];
            assert_eq!(quads[i].corners[3], next.corners[0]);
            assert_eq!(quads[i].corners[2], next.corners[1]);
        }
        assert!(
            quads
                .iter()
                .all(|q| q.transparency == draw.looks[&id].transparency(r))
        );
    }

    #[test]
    fn a_ring_is_clamped_to_the_mesh_and_skipped_out_of_view() {
        let (id, draw) = wafd();
        let quads = draw.surfaces(&[datum(id, (1, 1), 10)], None, 0, (64, 64));
        assert!(
            quads
                .iter()
                .flat_map(|q| q.corners)
                .all(|c| c[0] >= 0 && c[1] >= 0)
        );
        assert!(quads.iter().any(|q| q.corners[1][0] == 0));
        let quads = draw.surfaces(&[datum(id, (63, 63), 10)], None, 0, (64, 64));
        assert!(
            quads
                .iter()
                .flat_map(|q| q.corners)
                .all(|c| c[0] < 64 * 512 && c[1] < 64 * 512)
        );
        let region = ViewRegion {
            first_row: 0,
            last_row: 0,
            spans: vec![(0, 0)],
            ..Default::default()
        };
        assert!(
            draw.surfaces(&[datum(id, (5, 5), 10)], Some(&region), 0, (64, 64))
                .is_empty()
        );
    }

    #[test]
    fn the_frame_steps_after_each_ticks_per_frame_plus_one_updates() {
        let (id, mut draw) = wafd();
        let key = (FourCC::new(*b"inme"), 16);
        draw.set_timing((FourCC::new(*b"inme"), 3), 4, 1);
        assert!(draw.timing.is_empty());
        draw.set_timing(key, 4, 1);
        let frames: Vec<usize> = (0..10).map(|t| draw.frame(key, t)).collect();
        assert_eq!(frames, [0, 0, 1, 1, 2, 2, 3, 3, 0, 0]);
        let quads = draw.surfaces(&[datum(id, (20, 20), 5)], None, 0, (64, 64));
        assert!(quads.iter().all(|q| q.frame == 2));
    }
}
