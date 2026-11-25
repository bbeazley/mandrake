use std::collections::BTreeMap;

use mandrake_common::FourCC;
use mandrake_math::engine::{angle_index, arctangent, trig_tables};
use mandrake_render::billboard::TextureAtlas;
use mandrake_sim::explosions::distance3d;
use mandrake_sim::lightning::{LightningRecord, bezier};
use mandrake_sim::projectile_groups::ProjectileCatalog;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_view::billboard::BillboardInstance;
use mandrake_view::media::{ShadingSlot, build_tinting_shader};
use mandrake_view::model_surfaces::WorldFrame;
use mandrake_view::projectile::{EngineViewRows, VelocityQuad};
use mandrake_view::view_region::ViewRegion;

use crate::corpse;
use crate::media::{self, CoreRef};
use crate::projectile_local::{LocalSpriteSource, LocalSprites, TagPayload, in_cell_region};
use crate::scenery_billboards::{ObjectShadow, local_projectile_overlay};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightningLook {
    pub flags: u32,
    pub collection_reference: FourCC,
    pub overrun: i32,
    pub sequences: [i16; 2],
    pub speed: i16,
    pub branch_steps: i16,
    pub branch_steps_random: i16,
    pub extra_length: [i16; 2],
    pub branch_spread: u16,
    pub bolts: i16,
}

impl LightningLook {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 0x3a {
            return None;
        }
        let w = |o: usize| i16::from_be_bytes([data[o], data[o + 1]]);
        let d = |o: usize| i32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
        Some(Self {
            flags: d(0) as u32,
            collection_reference: FourCC::new(data[4..8].try_into().ok()?),
            overrun: d(0x10),
            sequences: [w(0x14), w(0x16)],
            speed: w(0x1a),
            branch_steps: w(0x1e),
            branch_steps_random: w(0x20),
            extra_length: [w(0x22), w(0x24)],
            branch_spread: w(0x26) as u16,
            bolts: w(0x38),
        })
    }

    pub fn sequence(&self, branch: bool) -> Option<u8> {
        u8::try_from(self.sequences[usize::from(!branch)]).ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightningSegment {
    pub definition: FourCC,
    pub start: [i32; 3],
    pub end: [i32; 3],
    pub branch: bool,
    pub transparency: u16,
    pub reflection: Option<i16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightningOverlay {
    pub definition: FourCC,
    pub point: [i32; 3],
    pub branch: bool,
}

pub struct LightningView<'a> {
    pub region: Option<&'a ViewRegion>,
    pub reflective: &'a dyn Fn(i32, i32) -> bool,
    pub media_height: &'a dyn Fn(i32, i32) -> i16,
    pub media_transparency: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bolt {
    pub system: FourCC,
    pub origin: [i32; 3],
    pub end: [i32; 3],
    pub mid1: [i32; 3],
    pub mid2: [i32; 3],
    pub ticks: i32,
    pub segments: i16,
}

impl From<&LightningRecord> for Bolt {
    fn from(r: &LightningRecord) -> Self {
        Self {
            system: r.system,
            origin: r.origin,
            end: r.end,
            mid1: r.mid1,
            mid2: r.mid2,
            ticks: r.ticks,
            segments: r.segments,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightningRules {
    pub cell_shift: u32,
    pub ground_overlays: bool,
    pub reflection_over_media: bool,
}

impl LightningRules {
    pub const MYTH2: Self = Self {
        cell_shift: 9,
        ground_overlays: true,
        reflection_over_media: true,
    };
    pub const TFL: Self = Self {
        cell_shift: 10,
        ground_overlays: true,
        reflection_over_media: false,
    };
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LightningSurfaces {
    pub segments: Vec<LightningSegment>,
    pub overlays: Vec<LightningOverlay>,
}

#[derive(Clone, Debug, Default)]
pub struct LightningDraw {
    pub looks: BTreeMap<FourCC, LightningLook>,
}

impl LightningDraw {
    #[cfg(test)]
    pub fn new(looks: BTreeMap<FourCC, LightningLook>) -> Self {
        Self { looks }
    }

    pub fn for_catalog(
        catalog: &ProjectileCatalog,
        tag: &TagPayload<'_>,
    ) -> (Self, Vec<LocalSpriteSource>) {
        let mut looks = BTreeMap::new();
        let mut sources = Vec::new();
        let mut ids: Vec<FourCC> = catalog.lightnings.keys().copied().collect();
        ids.sort_unstable_by_key(|i| i.as_u32());
        for id in ids {
            let Some(look) = tag(b"ligh", id).and_then(|d| LightningLook::from_bytes(&d)) else {
                continue;
            };
            looks.insert(id, look);
            let reference = look.collection_reference;
            let core = tag(b"core", reference);
            let collection = core
                .as_ref()
                .and_then(|c| Some(FourCC::new(c.get(0..4)?.try_into().ok()?)))
                .filter(|c| !matches!(u32::from_be_bytes(c.0), 0 | 0xffff_ffff))
                .unwrap_or(reference);
            let core = core
                .as_deref()
                .map_or_else(Default::default, CoreRef::from_core_bytes);
            for branch in [false, true] {
                let Some(sequence) = look.sequence(branch) else {
                    continue;
                };
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
        }
        (Self { looks }, sources)
    }

    pub fn surfaces(
        &self,
        records: &[LightningRecord],
        view: &LightningView<'_>,
        random: &mut LocalRandom,
    ) -> LightningSurfaces {
        self.bolt_surfaces(
            records.iter().map(Bolt::from),
            view,
            LightningRules::MYTH2,
            random,
        )
    }

    pub fn bolt_surfaces(
        &self,
        bolts: impl IntoIterator<Item = Bolt>,
        view: &LightningView<'_>,
        rules: LightningRules,
        random: &mut LocalRandom,
    ) -> LightningSurfaces {
        let mut out = LightningSurfaces::default();
        for r in bolts.into_iter().filter(|r| r.segments != 0) {
            let Some(look) = self.looks.get(&r.system) else {
                continue;
            };
            record_surfaces(&r, look, view, rules, random, &mut out);
        }
        out
    }
}

fn drawn(random: &mut LocalRandom, range: i32) -> i32 {
    (i32::from(random.next_u16()).wrapping_mul(range)) >> 16
}

type SegmentCall<'a> = dyn FnMut(&mut LightningSurfaces, [i32; 3], [i32; 3], bool) + 'a;

fn record_surfaces(
    r: &Bolt,
    look: &LightningLook,
    view: &LightningView<'_>,
    rules: LightningRules,
    random: &mut LocalRandom,
    out: &mut LightningSurfaces,
) {
    let head = i32::from(look.speed).wrapping_mul(r.ticks);
    let length = distance3d(r.origin, r.end);
    let tail = if look.flags & 1 != 0 {
        head - look.overrun
    } else {
        0
    };
    let shown = |d: i32| tail < d && d < head;
    let shift = rules.cell_shift;
    let visible = |p: [i32; 3]| {
        view.region
            .is_none_or(|g| in_cell_region(g, p[0] >> shift, p[1] >> shift))
    };
    let overlay = |out: &mut LightningSurfaces, point: [i32; 3], branch: bool| {
        if rules.ground_overlays {
            out.overlays.push(LightningOverlay {
                definition: r.system,
                point,
                branch,
            });
        }
    };
    let axis = (0..3)
        .map(|i| (r.end[i] - r.origin[i]).abs())
        .max()
        .unwrap_or(0);
    let segments = i32::from(r.segments);
    let jitter = ((axis << 8) / segments).wrapping_mul(2);
    let range = jitter / 0x300;
    let offset = -range / 2;
    for _ in 0..look.bolts.max(0) {
        let mut previous = r.origin;
        let transparency = random.next_u16();
        let mut segment =
            |out: &mut LightningSurfaces, start: [i32; 3], end: [i32; 3], branch: bool| {
                push_segment(
                    out,
                    view,
                    rules,
                    r.system,
                    (start, end),
                    branch,
                    transparency,
                )
            };
        for i in 1..segments {
            let mut p = bezier(
                ((i << 8) / segments) as i16,
                r.origin,
                r.mid1,
                r.mid2,
                r.end,
            );
            if jitter < 0x300 {
                p = p.map(|c| c + offset);
            } else {
                for c in &mut p {
                    *c += drawn(random, range) + offset;
                }
            }
            if shown(distance3d(r.origin, p)) && visible(p) {
                segment(out, previous, p, false);
                overlay(out, previous, false);
                if random.next_u16() >> 14 == 0 {
                    branch_steps(out, look, (previous, p), random, &mut segment, &overlay);
                }
            }
            previous = p;
        }
        if shown(length) && visible(r.end) {
            segment(out, previous, r.end, false);
            overlay(out, previous, false);
        }
    }
}

fn branch_steps(
    out: &mut LightningSurfaces,
    look: &LightningLook,
    (from, to): ([i32; 3], [i32; 3]),
    random: &mut LocalRandom,
    segment: &mut SegmentCall<'_>,
    overlay: &dyn Fn(&mut LightningSurfaces, [i32; 3], bool),
) {
    let trig = trig_tables();
    let half = distance3d(to, from) / 2;
    let mut facing = arctangent(to[0] - from[0], to[1] - from[1]);
    let mut at = to;
    let mut k = 0;
    loop {
        let mut steps = i32::from(look.branch_steps);
        if look.branch_steps_random > 0 {
            steps += drawn(random, i32::from(look.branch_steps_random));
        }
        if k >= steps {
            break;
        }
        let spread = u32::from(look.branch_spread);
        let turn = if spread * 2 != 0 {
            (u32::from(random.next_u16()) * (spread * 2)) >> 16
        } else {
            0
        };
        let angle = (turn + u32::from(facing)).wrapping_sub(spread) as u16;
        let i = angle_index(angle) & 0x3ff;
        let next = [
            ((i32::from(trig.cosine[i]) * half) >> 14) + at[0],
            ((i32::from(trig.sine[i]) * half) >> 14) + at[1],
            at[2],
        ];
        segment(out, at, next, true);
        k += 1;
        overlay(out, at, true);
        facing = arctangent(next[0] - at[0], next[1] - at[1]);
        at = next;
    }
}

fn push_segment(
    out: &mut LightningSurfaces,
    view: &LightningView<'_>,
    rules: LightningRules,
    definition: FourCC,
    (start, end): ([i32; 3], [i32; 3]),
    branch: bool,
    transparency: u16,
) {
    let base = LightningSegment {
        definition,
        start,
        end,
        branch,
        transparency,
        reflection: None,
    };
    out.segments.push(base);
    if (view.reflective)(start[0], start[1]) || (view.reflective)(end[0], end[1]) {
        let transparency = if rules.reflection_over_media {
            let t = (u32::from(!view.media_transparency) * u32::from(!transparency)) >> 16;
            !(t as u16)
        } else {
            transparency
        };
        out.segments.push(LightningSegment {
            transparency,
            reflection: Some((view.media_height)(start[0], start[1])),
            ..base
        });
    }
}

fn quad_mode(look: &LightningLook, branch: bool) -> (i32, i32) {
    if branch {
        (i32::from(look.extra_length[0]), 0x100)
    } else {
        (i32::from(look.extra_length[1]), 0x80)
    }
}

pub struct SegmentScreen<'a> {
    pub depth: &'a dyn Fn([i32; 3]) -> i32,
    pub on_screen: &'a dyn Fn([i32; 3]) -> bool,
}

impl SegmentScreen<'_> {
    pub(crate) fn with_camera<R>(
        frame: mandrake_view::model_surfaces::WorldFrame,
        camera: &mandrake_view::camera::Camera,
        aspect: f32,
        f: impl FnOnce(&SegmentScreen<'_>, ([f32; 3], [f32; 3], [f32; 3])) -> R,
    ) -> R {
        let axes @ (right, up, forward) = camera.view_axes();
        let rows = mandrake_view::projectile::EngineViewRows::from_camera_axes(right, up, forward);
        let eye = frame.to_engine(camera.eye_position()).map(|c| c as i32);
        let depth = |p: [i32; 3]| {
            (0..3).fold(0i32, |sum, i| {
                sum.wrapping_add(
                    p[i].wrapping_sub(eye[i])
                        .wrapping_mul(i32::from(rows.depth[i])),
                )
            }) >> 14
        };
        let view_proj = camera.view_proj_matrix(aspect);
        let on_screen = |p: [i32; 3]| {
            depth(p) < 0 && {
                let ndc = view_proj.transform_point3(frame.from_engine(p.map(|c| c as f32)));
                (-1.0..=1.0).contains(&ndc[0]) && (-1.0..=1.0).contains(&ndc[1])
            }
        };
        let screen = SegmentScreen {
            depth: &depth,
            on_screen: &on_screen,
        };
        f(&screen, axes)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn instances(
    draw: &LightningDraw,
    segments: &[LightningSegment],
    sprites: &LocalSprites,
    atlas: &TextureAtlas,
    frame: WorldFrame,
    axes: ([f32; 3], [f32; 3], [f32; 3]),
    screen: &SegmentScreen<'_>,
    permuted: &dyn Fn(FourCC, usize) -> usize,
    (global_tint, reflection_tint): (media::CoreTint, &dyn Fn(CoreRef) -> [u8; 8]),
) -> Vec<BillboardInstance> {
    let rows = EngineViewRows::from_camera_axes(axes.0, axes.1, axes.2);
    let mut out = Vec::with_capacity(segments.len());
    for s in segments {
        if (screen.depth)(s.start) >= -0x22
            || !((screen.on_screen)(s.start) || (screen.on_screen)(s.end))
        {
            continue;
        }
        let Some(look) = draw.looks.get(&s.definition) else {
            continue;
        };
        let Some(sequence) = look.sequence(s.branch) else {
            continue;
        };
        let Some((bitmap, core)) = sprites.bitmap(&(look.collection_reference, sequence), 0) else {
            continue;
        };
        let Some(mut instance) = atlas.create_instance_with_world_size(
            permuted(look.collection_reference, bitmap.atlas_index),
            frame.from_engine(s.start.map(|c| c as f32)),
            bitmap.world_size,
            bitmap.flags & 1 != 0,
        ) else {
            continue;
        };
        let velocity = [0, 1, 2].map(|i| s.start[i].wrapping_sub(s.end[i]) as i16);
        let (extra, height) = quad_mode(look, s.branch);
        let reflection = s.reflection.is_some();
        let base = match s.reflection {
            Some(surface) => corpse::reflected_origin(instance, surface, axes.1),
            None => instance,
        };
        instance = VelocityQuad::segment(&rows, velocity, bitmap.rect, height, extra, reflection)
            .instance(base, reflection);
        instance.tint[3] *= mandrake_view::media::surface_opacity(s.transparency);
        let tint = core.resolve(global_tint);
        if tint.fraction != 0 {
            let slot = ShadingSlot {
                kind: -1,
                ..ShadingSlot::default()
            };
            instance =
                instance.with_shade(build_tinting_shader(slot, tint.fraction, tint.color, slot));
        }
        if reflection {
            instance = instance.with_shade(reflection_tint(core));
        }
        out.push(instance);
    }
    out
}

pub fn overlays(
    draw: &LightningDraw,
    overlays: &[LightningOverlay],
    sprites: &LocalSprites,
    ground: &dyn Fn(i32, i32) -> i32,
) -> Vec<ObjectShadow> {
    overlays
        .iter()
        .filter_map(|o| {
            let look = draw.looks.get(&o.definition)?;
            let key = (look.collection_reference, look.sequence(o.branch)?);
            let shadow = sprites.overlay(&key, 0)?;
            let height = (o.point[2] - ground(o.point[0], o.point[1])) / 4;
            local_projectile_overlay(shadow, (o.point[0], o.point[1]), height, 0xffff)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FELI: [u8; 64] = [
        0x00, 0x00, 0x00, 0x05, 0x6c, 0x69, 0x67, 0x68, 0x00, 0x00, 0x00, 0x5a, 0x00, 0x00, 0x00,
        0x11, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x02, 0x66, 0x00, 0xfa,
        0x00, 0x02, 0x00, 0x04, 0x00, 0x0a, 0x00, 0x14, 0x31, 0xc7, 0x00, 0x04, 0x01, 0x02, 0x01,
        0x00, 0x01, 0x01, 0x08, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x57, 0x00, 0x02, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ];

    fn feli() -> LightningLook {
        LightningLook::from_bytes(&FELI).unwrap()
    }

    fn record_7729() -> LightningRecord {
        LightningRecord {
            index: 0,
            identifier: -32768,
            system: FourCC::new(*b"feli"),
            source: (751, 31770),
            target: (-1, -1),
            origin: [62585, 39920, -2228],
            end: [65379, 43276, -2675],
            mid1: [62837, 41459, -2288],
            mid2: [65050, 43512, -2475],
            last: [63164, 41259, -2306],
            ticks: 3,
            state_ticks: 3,
            travel_ticks: 7,
            segments: 27,
            state: 0,
            head: [63164, 41259, -2306],
        }
    }

    fn draw() -> (LightningDraw, LocalRandom) {
        (
            LightningDraw::new(BTreeMap::from([(FourCC::new(*b"feli"), feli())])),
            LocalRandom::new(1),
        )
    }

    const NOWHERE: LightningView<'static> = LightningView {
        region: None,
        reflective: &|_, _| false,
        media_height: &|_, _| 0,
        media_transparency: 0,
    };

    #[test]
    fn the_fetchs_lightning_definition() {
        let l = feli();
        assert_eq!(l.flags, 5);
        assert_eq!(l.collection_reference, FourCC::new(*b"ligh"));
        assert_eq!(l.overrun, 0x800);
        assert_eq!(l.sequence(false), Some(1), "body");
        assert_eq!(l.sequence(true), Some(0), "branch");
        assert_eq!(l.speed, 614);
        assert_eq!((l.branch_steps, l.branch_steps_random), (2, 4));
        assert_eq!(l.extra_length, [10, 20]);
        assert_eq!(l.branch_spread, 0x31c7);
        assert_eq!(l.bolts, 2);
        assert_eq!(quad_mode(&l, false), (20, 0x80));
        assert_eq!(quad_mode(&l, true), (10, 0x100));
    }

    #[test]
    fn the_bolts_grow_with_the_head() {
        let r = record_7729();
        assert_eq!(distance3d(r.origin, r.end), 4390);
        let (d, mut random) = draw();
        let s = d.surfaces(&[r], &NOWHERE, &mut random);
        let body: Vec<_> = s.segments.iter().filter(|s| !s.branch).collect();
        let starts: Vec<_> = body.iter().filter(|s| s.start == r.origin).collect();
        assert_eq!(starts.len(), 2);
        assert!(body.iter().all(|s| distance3d(r.origin, s.end) < 1842));
        assert!(body.iter().all(|s| s.end != r.end));
        let per_bolt = body.len() / 2;
        let within = |limit: i32| {
            (1..27)
                .map(|i| bezier(((i << 8) / 27) as i16, r.origin, r.mid1, r.mid2, r.end))
                .take_while(|p| distance3d(r.origin, *p) < limit)
                .count()
        };
        assert!(
            (within(1842 - 72)..=within(1842 + 72)).contains(&per_bolt),
            "{per_bolt}"
        );
        assert_eq!(per_bolt, 9);
        for w in body.windows(2) {
            if w[1].start != r.origin {
                assert_eq!(w[0].end, w[1].start);
                assert_eq!(w[0].transparency, w[1].transparency, "one draw a bolt");
            }
        }
        for (i, s) in body.iter().take(per_bolt).enumerate() {
            let on = bezier(
                (((i as i32 + 1) << 8) / 27) as i16,
                r.origin,
                r.mid1,
                r.mid2,
                r.end,
            );
            for (c, (got, curve)) in s.end.iter().zip(on).enumerate() {
                assert!((-41..41).contains(&(got - curve)), "{i} {c}");
            }
        }
        assert_eq!(s.overlays.len(), s.segments.len());
        for (seg, o) in s.segments.iter().zip(&s.overlays) {
            assert_eq!((seg.start, seg.branch), (o.point, o.branch));
        }
    }

    #[test]
    fn a_branch_steps_half_a_segment_at_its_height() {
        let r = record_7729();
        let (d, mut random) = draw();
        let mut branched = 0;
        for _ in 0..50 {
            let s = d.surfaces(&[r], &NOWHERE, &mut random);
            let mut i = 0;
            while i < s.segments.len() {
                let body = s.segments[i];
                let steps = s.segments[i + 1..].iter().take_while(|s| s.branch).count();
                if steps > 0 {
                    branched += 1;
                    assert!((2..=5).contains(&steps), "{steps}");
                    let half = distance3d(body.end, body.start) / 2;
                    let first = s.segments[i + 1];
                    assert_eq!(first.start, body.end);
                    assert_eq!(first.end[2], body.end[2]);
                    let len = distance3d(first.end, first.start);
                    assert!((half - 1..=half + 1).contains(&len), "{len} {half}");
                }
                i += 1 + steps;
            }
        }
        assert!(branched > 0);
    }

    #[test]
    fn the_tail_follows_the_head_and_the_end_is_drawn_last() {
        let mut r = record_7729();
        r.ticks = 9;
        let (d, mut random) = draw();
        let s = d.surfaces(&[r], &NOWHERE, &mut random);
        let body: Vec<_> = s.segments.iter().filter(|s| !s.branch).collect();
        assert!(body.iter().all(|s| distance3d(r.origin, s.end) > 3478));
        assert_eq!(body.iter().filter(|s| s.end == r.end).count(), 2);
        r.ticks = 11;
        assert!(d.surfaces(&[r], &NOWHERE, &mut random).segments.is_empty());
        r.ticks = 3;
        r.segments = 0;
        assert!(d.surfaces(&[r], &NOWHERE, &mut random).segments.is_empty());
    }

    #[test]
    fn a_reflecting_cell_adds_the_reflected_pass() {
        let r = record_7729();
        let view = LightningView {
            region: None,
            reflective: &|x, _| x < 62700,
            media_height: &|_, _| -2400,
            media_transparency: 0x8000,
        };
        let (d, mut random) = draw();
        let s = d.surfaces(&[r], &view, &mut random);
        let first = s.segments[0];
        assert_eq!(first.reflection, None);
        let mirrored = s.segments[1];
        assert_eq!((mirrored.start, mirrored.end), (first.start, first.end));
        assert_eq!(mirrored.reflection, Some(-2400));
        let t = (0x7fffu32 * u32::from(!first.transparency)) >> 16;
        assert_eq!(mirrored.transparency, !(t as u16));
        assert!(
            s.segments
                .iter()
                .filter(|s| s.reflection.is_some())
                .all(|s| s.start[0] < 62700 || s.end[0] < 62700)
        );
    }

    #[test]
    #[ignore]
    fn the_cost_of_a_frames_surfaces() {
        let mut r = record_7729();
        r.ticks = 5;
        let (d, mut random) = draw();
        let n = 20_000;
        let started = crate::clock::Instant::now();
        let mut segments = 0;
        for _ in 0..n {
            segments += d.surfaces(&[r], &NOWHERE, &mut random).segments.len();
        }
        let each = started.elapsed().as_secs_f64() / f64::from(n);
        println!(
            "{:.2} us a frame, {} segments a frame",
            each * 1e6,
            segments / n as usize
        );
    }

    #[test]
    fn tfl_draws_the_same_bolts_and_overlays() {
        let r = record_7729();
        let (d, _) = draw();
        let view = LightningView {
            region: None,
            reflective: &|x, _| x < 62700,
            media_height: &|_, _| 0,
            media_transparency: 0x8000,
        };
        let (mut a, mut b) = (LocalRandom::new(7), LocalRandom::new(7));
        let m2 = d.bolt_surfaces([Bolt::from(&r)], &view, LightningRules::MYTH2, &mut a);
        let tfl = d.bolt_surfaces([Bolt::from(&r)], &view, LightningRules::TFL, &mut b);
        assert_eq!(a.next_u16(), b.next_u16(), "the same draws");
        assert!(!m2.overlays.is_empty());
        assert_eq!(tfl.overlays, m2.overlays);
        let none = LightningRules {
            ground_overlays: false,
            ..LightningRules::TFL
        };
        let (mut c, mut e) = (LocalRandom::new(7), LocalRandom::new(7));
        let bare = d.bolt_surfaces([Bolt::from(&r)], &view, none, &mut c);
        d.bolt_surfaces([Bolt::from(&r)], &view, LightningRules::TFL, &mut e);
        assert!(bare.overlays.is_empty());
        assert_eq!(c.next_u16(), e.next_u16(), "the overlays draw nothing");
        assert_eq!(m2.segments.len(), tfl.segments.len());
        let mut reflected = 0;
        for (m, t) in m2.segments.iter().zip(&tfl.segments) {
            assert_eq!((m.start, m.end, m.branch), (t.start, t.end, t.branch));
            if t.reflection.is_some() {
                reflected += 1;
                assert_eq!(t.reflection, Some(0));
                assert_ne!(t.transparency, m.transparency);
            }
        }
        assert!(reflected > 0);
        for w in tfl.segments.windows(2) {
            if w[1].reflection.is_some() {
                assert_eq!(w[1].transparency, w[0].transparency);
            }
        }
        let region = ViewRegion {
            first_row: 38,
            last_row: 43,
            spans: vec![(61, 64); 6],
            built: Vec::new(),
            rect_last_row: 43,
        };
        let view = LightningView {
            region: Some(&region),
            ..NOWHERE
        };
        let (mut a, mut b) = (LocalRandom::new(7), LocalRandom::new(7));
        assert!(
            !d.bolt_surfaces([Bolt::from(&r)], &view, LightningRules::TFL, &mut a)
                .segments
                .is_empty()
        );
        assert!(
            d.bolt_surfaces([Bolt::from(&r)], &view, LightningRules::MYTH2, &mut b)
                .segments
                .is_empty()
        );
    }

    #[test]
    fn outside_the_view_region_nothing_is_drawn() {
        let r = record_7729();
        let region = ViewRegion {
            first_row: 0,
            last_row: 10,
            spans: vec![(0, 10); 11],
            built: Vec::new(),
            rect_last_row: 10,
        };
        let view = LightningView {
            region: Some(&region),
            ..NOWHERE
        };
        let (d, mut random) = draw();
        assert!(d.surfaces(&[r], &view, &mut random).segments.is_empty());
    }
}
