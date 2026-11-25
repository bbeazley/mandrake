use std::collections::BTreeMap;

use mandrake_common::FourCC;
use mandrake_math::engine::{arctangent, trig_tables};
use mandrake_render::billboard::TextureAtlas;
use mandrake_sim::combat::SequenceSound;
use mandrake_sim::engine_move::signed_angular_difference;
use mandrake_sim::engine_rng::AnimSounds;
use mandrake_sim::fire::MirrorGroup;
use mandrake_sim::flight::Stain;
use mandrake_sim::map_actions::LocalGroupAction;
use mandrake_sim::projectile_groups::ProjectileCatalog;
use mandrake_sim::projectiles::isqrt;
use mandrake_sim::{CombatEvent, Simulation};
use mandrake_tag::ProjectileDefinition;
use mandrake_tag::collection::INSTANCE_FLAG_VELOCITY_ALIGNED;
use mandrake_tag::mesh::{MarkerEntry, MarkerPaletteEntry};
use mandrake_view::billboard::{
    BILLBOARD_FLAG_MEDIA_EFFECT, BILLBOARD_FLAG_MIRROR_Y, BillboardInstance,
};
use mandrake_view::media::{ShadingSlot, build_tinting_shader};
use mandrake_view::model_surfaces::WorldFrame;
use mandrake_view::projectile::{EngineViewRows, InstanceRect, VelocityQuad};
use mandrake_view::view_region::ViewRegion;

use crate::scenery_billboards::{ObjectShadow, local_projectile_overlay};
use crate::unit_anim::ShadowSprite;
use mandrake_tag::sequence_header::sequence_headers;
use mandrake_tag::sound_manager::LocalRandom;

use crate::corpse;
use crate::media::{self, CoreRef};
use crate::unit_anim::SequenceVisual;

const MAX_GROUPS: usize = 0x180;
const MAX_PROJECTILES: usize = 0x800;

const FLAG_ONE_SHOT: u32 = 1;
const FLAG_WIND: u32 = 2;
const FLAG_GROUND: u32 = 4;
const FLAG_LIFETIME: u32 = 8;
const FLAG_DELETE_ON_WRAP: u32 = 0x10;
const FLAG_NO_ANIMATION: u32 = 0x20;
const FLAG_START_PERIOD: u32 = 0x40;
const FLAG_ON_DURATION: u32 = 0x80;
const FLAG_END_PERIOD: u32 = 0x100;
const FLAG_START_OPACITY: u32 = 0x200;
const FLAG_START_COUNT: u32 = 0x400;
const FLAG_UPDATE_UNSEEN: u32 = 0x800;
const FLAG_FOLLOW: u32 = 0x1000;
const FLAG_END_OPACITY: u32 = 0x2000;
const FLAG_END_COUNT: u32 = 0x4000;
const FLAG_END_NONE: u32 = 0x8000;
const FLAG_END_WHEN_EMPTY: u32 = 0x10000;
const FLAG_START_SCALE_X: u32 = 0x20000;
const FLAG_START_SCALE_Y: u32 = 0x40000;
const FLAG_END_SCALE_X: u32 = 0x80000;
const FLAG_END_SCALE_Y: u32 = 0x100000;
const FLAG_EVEN_ANGLES: u32 = 0x200000;
const FLAG_BELOW_GROUND: u32 = 0x400000;
const FLAG_FIRST_FRAME: u32 = 0x800000;
const FLAG_REFLECTED: u32 = 0x100_0000;
const FLAG_MEDIA_SURFACE: u32 = 0x200_0000;
const FLAG_TICK_BY_TICK: u32 = 0x400_0000;

pub const FIRE_GROUPS: [FourCC; 2] = [FourCC(*b"smff"), FourCC(*b"fire")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalPhysics {
    Drift(DriftPhysics),
    Orbit(OrbitPhysics),
    Wave(WavePhysics),
    Pull(PullPhysics),
    Converge(ConvergePhysics),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConvergePhysics {
    pub diverge: bool,
    pub spin: u16,
    pub spin_range: u16,
    pub elevation: u16,
    pub elevation_range: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriftPhysics {
    pub flags: u16,
    pub acceleration: [i32; 3],
    pub velocity: [i32; 3],
    pub velocity_range: [i32; 3],
    pub jitter: [i32; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrbitPhysics {
    pub spin_acceleration: u16,
    pub spin: u16,
    pub spin_range: u16,
    pub bob_height: i16,
    pub bob_rate: i32,
    pub radial_speed: i32,
    pub rise: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WavePhysics {
    pub random_phase: bool,
    pub spin: u16,
    pub spin_range: u16,
    pub rate: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PullPhysics {
    pub on_ground: bool,
    pub speed_limit: i32,
    pub pull: i32,
    pub height: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalDefinition {
    pub tag: FourCC,
    pub flags: u32,
    pub collection_reference: FourCC,
    pub sequence: u8,
    pub stain_sequence: i16,
    pub physics: Option<LocalPhysics>,
    pub scale: (i16, i16),
    pub speed: (i16, i16),
    pub count: (i16, i16),
    pub outward: (i32, i32),
    pub lifetime: (i32, i32),
    pub transfer: i16,
    pub height: i16,
    pub start_period: (i16, i16),
    pub end_period: (i16, i16),
    pub on_duration: (i32, i32),
    pub start_scale: [(i16, i16); 2],
    pub end_scale: [(i16, i16); 2],
    pub team_rule: u8,
}

impl LocalDefinition {
    pub fn parse(
        tag: FourCC,
        data: &[u8],
        phys: impl Fn(FourCC) -> Option<Vec<u8>>,
    ) -> Option<Self> {
        Self::parse_with_rules(tag, data, phys, Rules::Myth2)
    }

    pub fn parse_with_rules(
        tag: FourCC,
        data: &[u8],
        phys: impl Fn(FourCC) -> Option<Vec<u8>>,
        rules: Rules,
    ) -> Option<Self> {
        let w = |o: usize| Some(i16::from_be_bytes(data.get(o..o + 2)?.try_into().ok()?));
        let d = |o: usize| Some(i32::from_be_bytes(data.get(o..o + 4)?.try_into().ok()?));
        let id = |o: usize| Some(FourCC::new(data.get(o..o + 4)?.try_into().ok()?));
        let physics = if matches!(u32::from_be_bytes(id(8)?.0), 0 | 0xffff_ffff) {
            None
        } else {
            let p = phys(id(8)?)?;
            let pw = |o: usize| Some(i16::from_be_bytes(p.get(o..o + 2)?.try_into().ok()?));
            let pd = |o: usize| Some(i32::from_be_bytes(p.get(o..o + 4)?.try_into().ok()?));
            let triple = |o: usize| Some([pd(o)?, pd(o + 4)?, pd(o + 8)?]);
            let tfl = rules == Rules::Tfl;
            let flags = pd(4)? as u16;
            Some(match pw(0)? {
                0 => LocalPhysics::Drift(DriftPhysics {
                    flags,
                    acceleration: triple(8)?,
                    velocity: triple(0x14)?,
                    velocity_range: triple(0x20)?,
                    jitter: triple(0x2c)?,
                }),
                1 => LocalPhysics::Orbit(OrbitPhysics {
                    spin_acceleration: pw(8)? as u16,
                    spin: pw(0x0a)? as u16,
                    spin_range: pw(0x0c)? as u16,
                    bob_height: pw(0x0e)?,
                    bob_rate: pd(0x10)?,
                    radial_speed: pd(0x14)?,
                    rise: if tfl { 0 } else { pd(0x18)? },
                }),
                2 => LocalPhysics::Wave(WavePhysics {
                    random_phase: flags & 1 != 0,
                    spin: pw(8)? as u16,
                    spin_range: pw(0x0a)? as u16,
                    rate: pd(0x0c)?,
                }),
                3 => LocalPhysics::Pull(PullPhysics {
                    on_ground: flags & 1 != 0,
                    speed_limit: pd(8)?,
                    pull: pd(0x10)?,
                    height: pw(0x14)?,
                }),
                4 => LocalPhysics::Converge(ConvergePhysics {
                    diverge: flags & 1 != 0,
                    spin: pw(8)? as u16,
                    spin_range: pw(0x0a)? as u16,
                    elevation: pw(0x0c)? as u16,
                    elevation_range: pw(0x0e)? as u16,
                }),
                _ => return None,
            })
        };
        Some(Self {
            tag,
            flags: d(0)? as u32,
            collection_reference: id(4)?,
            sequence: w(0x0c)?.max(0) as u8,
            stain_sequence: w(0x0e)?,
            physics,
            scale: (w(0x10)?, w(0x12)?),
            speed: (w(0x14)?, w(0x16)?),
            count: (w(0x18)?, w(0x1a)?),
            outward: (d(0x1c)?, d(0x20)?),
            lifetime: (d(0x24)?, d(0x28)?),
            transfer: w(0x2c)?,
            height: w(0x2e)?,
            start_period: (w(0x30)?, w(0x32)?),
            end_period: (w(0x34)?, w(0x36)?),
            on_duration: (d(0x38)?, d(0x3c)?),
            start_scale: [(w(0x40)?, w(0x42)?), (w(0x44)?, w(0x46)?)],
            end_scale: [(w(0x48)?, w(0x4a)?), (w(0x4c)?, w(0x4e)?)],
            team_rule: *data.get(0x50)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalSequence {
    pub frames: i16,
    pub ticks_per_frame: i16,
    pub key_frame: i16,
    pub loop_frame: i16,
    pub stored_view: bool,
    pub views: i16,
    pub permutations: i16,
    pub sounds: [bool; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupState {
    Starting,
    On,
    Ending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    pub index: i32,
    pub identifier: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetCylinder {
    pub point: [i32; 3],
    pub radius: i16,
    pub bottom: i16,
    pub top: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewGroup {
    pub definition: FourCC,
    pub point: [i32; 3],
    pub central: Option<ObjectRef>,
    pub target: Option<TargetCylinder>,
    pub opacity: u16,
    pub facing: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Group {
    definition: LocalDefinition,
    sequence: LocalSequence,
    center: [i32; 3],
    central: Option<ObjectRef>,
    target: Option<TargetCylinder>,
    state: GroupState,
    timer: i16,
    wanted: i16,
    count: i16,
    on_duration: i32,
    start_period: i16,
    end_period: i16,
    opacity: u16,
    facing: u16,
    visible: bool,
    scale_x: i16,
    scale_y: i16,
    drawn_opacity: u16,
    mirrored: bool,
    stains: bool,
    identifier: Option<i16>,
    bounds: [i32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Projectile {
    group: usize,
    velocity: [i32; 3],
    period: i32,
    clock: i32,
    rising: i32,
    relative: [i32; 3],
    world: Option<[i32; 3]>,
    facing: u16,
    scale: i16,
    frame: i16,
    tick: i16,
    ticks_per_frame: i16,
    permutation: i16,
    age: i32,
    lifetime: i32,
    serial: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalSurface {
    pub collection_reference: FourCC,
    pub sequence: u8,
    pub frame: i16,
    pub view: i16,
    pub world: [i32; 3],
    pub above_ground: i32,
    pub facing: u16,
    pub transparency: u16,
    pub scale_x: i16,
    pub scale_y: i16,
    pub velocity: [i16; 3],
    pub permutation: i16,
    pub media_surface: bool,
    pub reflection: Option<i16>,
}

pub struct SurfaceView<'a> {
    pub eye: [i32; 3],
    pub height: &'a dyn Fn(i32, i32) -> i32,
    pub media_height: &'a dyn Fn(i32, i32) -> i32,
    pub region: Option<&'a ViewRegion>,
    pub media_transparency: u16,
    pub reflective: &'a dyn Fn(i32, i32) -> bool,
    pub mirrors: &'a [Option<MirrorGroup>],
}

pub(crate) fn in_cell_region(region: &ViewRegion, col: i32, row: i32) -> bool {
    if row < region.first_row || row > region.last_row {
        return false;
    }
    region
        .spans
        .get((row - region.first_row) as usize)
        .is_some_and(|&(first, last)| col >= i32::from(first) && col <= i32::from(last))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AmbientSource {
    pub group: FourCC,
    pub tries: i16,
    pub period: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ambient {
    group: usize,
    tries: i16,
    period: i16,
    timer: i32,
}

pub struct AmbientWorld<'a> {
    pub region: &'a ViewRegion,
    pub cell: &'a dyn Fn(i32, i32) -> (u16, i16),
    pub splash_intensity: &'a dyn Fn(i32, i32) -> u16,
}

const CELL_MEDIA: u16 = 0x1800;

fn point_in_cell(random: &mut LocalRandom, col: i32, row: i32, height: i16) -> [i32; 3] {
    let x = (i32::from(random.next_u16()) >> 7) + (col << 9);
    let y = (i32::from(random.next_u16()) >> 7) + (row << 9);
    [x, y, i32::from(height)]
}

pub struct LocalWorld<'a> {
    pub size: [i32; 2],
    pub height: &'a dyn Fn(i32, i32) -> i32,
    pub wind: Option<[i32; 3]>,
    pub object: &'a dyn Fn(ObjectRef) -> Option<[i32; 3]>,
    pub cell_bits: &'a dyn Fn(ObjectRef, [i32; 3]) -> Option<u32>,
    pub tfl_cell: Option<&'a dyn Fn(i32, i32) -> (u8, bool)>,
    pub tfl_drawn: Option<&'a dyn Fn([i32; 3]) -> bool>,
}

const TFL_CELL_WATER: u8 = 0x10;
const TFL_CELL_SHORE: u8 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LocalMarker {
    identifier: i16,
    definition: FourCC,
    point: [i32; 3],
    facing: u16,
    group: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rules {
    #[default]
    Myth2,
    Tfl,
}

fn short_speed(root: i32) -> i32 {
    i32::from(root as i16)
}

const SOUNDS_KEPT: usize = 64;

#[derive(Clone, Debug)]
pub struct LocalProjectiles {
    definitions: BTreeMap<FourCC, (LocalDefinition, LocalSequence)>,
    of_projectile: BTreeMap<FourCC, FourCC>,
    groups: Vec<Option<Group>>,
    projectiles: Vec<Option<Projectile>>,
    first_free_projectile: usize,
    history: Vec<Option<Projectile>>,
    random: LocalRandom,
    media_effect: Option<(Ambient, u16)>,
    splashes: Option<Ambient>,
    smoke_groups: [Option<usize>; 3],
    flame_groups: [Option<usize>; 5],
    mirror_stains: Vec<Option<(MirrorGroup, i16)>>,
    stains: Vec<Stain>,
    collections: BTreeMap<FourCC, FourCC>,
    markers: Vec<LocalMarker>,
    group_actions_run: u64,
    tick: u32,
    sounds: Vec<SequenceSound>,
    monster_groups: BTreeMap<u32, usize>,
    rules: Rules,
    serial: u32,
    view_cells: Option<[i32; 4]>,
}

fn range16(random: &mut LocalRandom, (base, delta): (i16, i16)) -> i16 {
    if delta > 0 {
        base.wrapping_add(((i32::from(delta) * i32::from(random.next_u16())) >> 16) as i16)
    } else {
        base
    }
}

fn range_u16(random: &mut LocalRandom, base: u16, delta: u16) -> u16 {
    if delta != 0 {
        base.wrapping_add(((u32::from(delta) * u32::from(random.next_u16())) >> 16) as u16)
    } else {
        base
    }
}

fn range32(random: &mut LocalRandom, (base, delta): (i32, i32)) -> i32 {
    if delta > 0 {
        base.wrapping_add((i32::from(random.next_u16()).wrapping_mul(delta)) >> 16)
    } else {
        base
    }
}

fn trig_index(angle: u16) -> usize {
    (((i32::from(angle) + 0x20) >> 6) & 0x3ff) as usize
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Motion {
    relative: [i32; 3],
    velocity: [i32; 3],
    period: i32,
    clock: i32,
    facing: u16,
}

impl Motion {
    fn at_rest(relative: [i32; 3], facing: u16) -> Self {
        Self {
            relative,
            facing,
            ..Self::default()
        }
    }
}

fn packed(angle: u16, spin: u16) -> i32 {
    ((u32::from(spin) << 16) | u32::from(angle)) as i32
}

fn unpacked(word: i32) -> (u16, u16) {
    (word as u16, (word as u32 >> 16) as u16)
}

fn period_and_phase(rate: i32, turn: u16) -> (i32, i32) {
    if rate == 0 {
        (0x10000, 0)
    } else {
        (0x10000 / rate, i32::from(turn) / rate)
    }
}

fn advance_clock(clock: i32, ticks: i32, period: i32) -> i32 {
    if period == 0 {
        clock + ticks
    } else {
        (clock + ticks) % period
    }
}

fn ring_point(angle: u16, radius: i32) -> [i32; 2] {
    let trig = trig_tables();
    let i = trig_index(angle);
    [
        i32::from(trig.cosine[i]).wrapping_mul(radius) >> 14,
        i32::from(trig.sine[i]).wrapping_mul(radius) >> 14,
    ]
}

fn start_orbit(
    r: &mut LocalRandom,
    outward: (i32, i32),
    p: &OrbitPhysics,
    even: Option<u16>,
) -> Motion {
    let trig = trig_tables();
    let radius = range32(r, outward);
    let phase = r.next_u16();
    let angle = even.unwrap_or_else(|| r.next_u16());
    let spin = range_u16(r, p.spin, p.spin_range);
    let (period, clock) = period_and_phase(p.bob_rate, phase);
    let bob_height = i32::from(p.bob_height);
    let twice = bob_height.wrapping_mul(2);
    let height = if twice > 0 {
        (twice.wrapping_mul(i32::from(r.next_u16())) >> 16).wrapping_sub(bob_height)
    } else {
        bob_height.wrapping_neg()
    };
    let [rx, ry] = ring_point(angle, radius);
    Motion {
        relative: [
            rx,
            ry,
            height.wrapping_mul(i32::from(trig.cosine[trig_index(phase)])) >> 14,
        ],
        velocity: [packed(angle, spin), radius, height],
        period,
        clock,
        facing: angle.wrapping_add(if spin != 0 { 0xc000 } else { 0x4000 }),
    }
}

fn start_drift(r: &mut LocalRandom, g: &Group, p: &DriftPhysics) -> Motion {
    let d = &g.definition;
    let trig = trig_tables();
    let speed = range32(r, d.outward);
    let angle = r.next_u16();
    let axis = |r: &mut LocalRandom, i: usize| {
        let (base, delta) = (p.velocity[i] >> 8, p.velocity_range[i] >> 8);
        range32(r, (base, delta)) << 8
    };
    let mut vx = axis(r, 0);
    let mut vy = axis(r, 1);
    let vz = axis(r, 2);
    if p.flags & 2 != 0 {
        let (x, y) = (i32::from((vx >> 8) as i16), i32::from((vy >> 8) as i16));
        let m = short_speed(isqrt((x * x + y * y) as u32));
        let i = trig_index(arctangent(x, y).wrapping_add(g.facing));
        vx = (i32::from(trig.cosine[i]).wrapping_mul(m) >> 14) << 8;
        vy = (i32::from(trig.sine[i]).wrapping_mul(m) >> 14) << 8;
    }
    let facing = arctangent(vx >> 16, vy >> 16);
    let (mut x, mut y, z) = match g.target.filter(|_| p.flags & 1 != 0) {
        None => {
            let [rx, ry] = ring_point(angle, speed);
            (rx, ry, 0)
        }
        Some(t) => {
            let bottom = i32::from(t.bottom) - g.center[2] + t.point[2];
            let [rx, ry] = ring_point(angle, i32::from(t.radius));
            (
                rx + (t.point[0] - g.center[0]),
                ry + (t.point[1] - g.center[1]),
                range32(r, (bottom, i32::from(t.top) - i32::from(t.bottom))),
            )
        }
    };
    if p.flags & 2 != 0 {
        let m = if (x.wrapping_add(0x7fff) as u32) < 0xfffe
            && (y.wrapping_add(0x7fff) as u32) < 0xfffe
        {
            isqrt((x * x + y * y) as u32)
        } else {
            0x7fff
        };
        let [rx, ry] = ring_point(arctangent(x, y).wrapping_add(g.facing), m);
        x = rx;
        y = ry;
    }
    Motion {
        relative: [x, y, z],
        velocity: [vx, vy, vz],
        facing,
        ..Motion::default()
    }
}

fn start_wave(r: &mut LocalRandom, outward: (i32, i32), p: &WavePhysics) -> Motion {
    let trig = trig_tables();
    let speed = range32(r, outward);
    let phase = if p.random_phase { r.next_u16() } else { 0 };
    let a = r.next_u16();
    let spin = range_u16(r, p.spin, p.spin_range);
    let b = r.next_u16();
    let (period, clock) = period_and_phase(p.rate, phase);
    let breath = i32::from(trig.cosine[if p.random_phase { trig_index(phase) } else { 0 }]);
    let (ia, ib) = (trig_index(a), trig_index(b));
    let flat = |t: i16| {
        let along = (i32::from(t) * speed) >> 14;
        (((along * i32::from(trig.cosine[ib])) >> 14) * breath) >> 14
    };
    let up = (((i32::from(trig.sine[ib]) * speed) >> 14) * breath) >> 14;
    Motion {
        relative: [flat(trig.cosine[ia]), flat(trig.sine[ia]), up],
        velocity: [packed(a, spin), packed(b, 0), speed],
        period,
        clock,
        facing: 0,
    }
}

fn start_converge(
    r: &mut LocalRandom,
    d: &LocalDefinition,
    p: &ConvergePhysics,
    tfl: bool,
) -> Motion {
    let trig = trig_tables();
    let radius = range32(r, d.outward);
    let angle = r.next_u16();
    let spin = range_u16(r, p.spin, p.spin_range);
    let elevation = range_u16(r, p.elevation, p.elevation_range);
    let time = if !tfl {
        range32(r, d.lifetime)
    } else if d.lifetime.1 == 0 {
        d.lifetime.0
    } else {
        d.lifetime
            .0
            .wrapping_add(((u32::from(r.next_u16()) * d.lifetime.1 as u32) >> 16) as i32)
    };
    let (ia, ie) = (trig_index(angle), trig_index(elevation));
    let flat = |t: i16| {
        (((i32::from(t).wrapping_mul(radius)) >> 14).wrapping_mul(i32::from(trig.cosine[ie]))) >> 14
    };
    Motion {
        relative: [
            flat(trig.cosine[ia]),
            flat(trig.sine[ia]),
            i32::from(trig.sine[ie]).wrapping_mul(radius) >> 14,
        ],
        velocity: [packed(angle, spin), packed(elevation, 0), radius],
        period: time,
        clock: 0,
        facing: 0,
    }
}

fn move_converge(p: &mut Projectile, ph: &ConvergePhysics, ticks: i32) {
    let trig = trig_tables();
    let total = p.period;
    p.clock = p.clock.wrapping_add(ticks).clamp(0, total.max(0));
    let radius = p.velocity[2];
    let r = match (total, ph.diverge) {
        (0, _) => 0,
        (t, false) => (t - p.clock).wrapping_mul(radius) / t,
        (t, true) => radius.wrapping_mul(p.clock) / t,
    };
    let (e, tilt) = unpacked(p.velocity[1]);
    let e = e.wrapping_add(tilt.wrapping_mul(ticks as u16));
    p.velocity[1] = packed(e, tilt);
    let (a, spin) = unpacked(p.velocity[0]);
    let a = a.wrapping_add(spin.wrapping_mul(ticks as u16));
    p.velocity[0] = packed(a, spin);
    let (ia, ie) = (trig_index(a), trig_index(e));
    let flat = i32::from(trig.cosine[ie]).wrapping_mul(r) >> 14;
    p.relative[0] = i32::from(trig.cosine[ia]).wrapping_mul(flat) >> 14;
    p.relative[1] = i32::from(trig.sine[ia]).wrapping_mul(flat) >> 14;
    p.relative[2] = i32::from(trig.sine[ie]).wrapping_mul(r) >> 14;
    p.facing = arctangent(p.relative[0], p.relative[1]);
}

fn move_orbit(p: &mut Projectile, ph: &OrbitPhysics, ticks: i32, tfl: bool) {
    let trig = trig_tables();
    p.clock = advance_clock(p.clock, ticks, p.period);
    let (angle, spin) = unpacked(p.velocity[0]);
    let spin = spin.wrapping_add(ph.spin_acceleration.wrapping_mul(ticks as u16));
    let angle = angle.wrapping_add(spin.wrapping_mul(ticks as u16));
    p.velocity[0] = packed(angle, spin);
    p.facing = if tfl {
        let quarter: u16 = if spin == 0 { 0xc000 } else { 0x4000 };
        angle
            .wrapping_add(arctangent(0, ph.radial_speed >> 16))
            .wrapping_add(quarter)
    } else {
        angle.wrapping_add(0x8000)
    };
    p.velocity[1] = (ph.radial_speed.wrapping_mul(ticks) >> 16).wrapping_add(p.velocity[1]);
    let i = trig_index(angle);
    p.relative[0] = i32::from(trig.cosine[i]).wrapping_mul(p.velocity[1]) >> 14;
    p.relative[1] = i32::from(trig.sine[i]).wrapping_mul(p.velocity[1]) >> 14;
    if ph.rise == 0 {
        let turn = ph.bob_rate.wrapping_mul(p.clock) as u16;
        p.relative[2] = i32::from(trig.cosine[trig_index(turn)]).wrapping_mul(p.velocity[2]) >> 14;
    } else {
        p.rising = p.rising.wrapping_add(ph.rise);
        p.relative[2] = (p.rising.wrapping_mul(ticks) >> 16).wrapping_add(p.relative[2]);
    }
}

fn move_wave(p: &mut Projectile, ph: &WavePhysics, ticks: i32) {
    let trig = trig_tables();
    p.clock = advance_clock(p.clock, ticks, p.period);
    let (a, spin) = unpacked(p.velocity[0]);
    let a = a.wrapping_add(spin.wrapping_mul(ticks as u16));
    p.velocity[0] = packed(a, spin);
    let turn = (p.clock as u16).wrapping_mul(ph.rate as u16);
    let radius = (i32::from(trig.cosine[trig_index(turn)]).wrapping_mul(p.velocity[2])) >> 14;
    let (b, tilt) = unpacked(p.velocity[1]);
    let b = b.wrapping_add(tilt.wrapping_mul(ticks as u16));
    p.velocity[1] = packed(b, tilt);
    let (ia, ib) = (trig_index(a), trig_index(b));
    let flat = i32::from(trig.cosine[ib]).wrapping_mul(radius) >> 14;
    p.relative[0] = i32::from(trig.cosine[ia]).wrapping_mul(flat) >> 14;
    p.relative[1] = i32::from(trig.sine[ia]).wrapping_mul(flat) >> 14;
    p.relative[2] = i32::from(trig.sine[ib]).wrapping_mul(radius) >> 14;
}

fn move_pull(
    p: &mut Projectile,
    ph: &PullPhysics,
    ticks: i32,
    center: [i32; 3],
    height: &dyn Fn(i32, i32) -> i32,
) {
    let (x, y) = (p.relative[0], p.relative[1]);
    let near = |v: i32| (v.wrapping_add(0x7fff) as u32) < 0xfffe;
    let distance = if near(x) && near(y) {
        (isqrt((x * x + y * y) as u32) as i32).max(1)
    } else {
        0x7fff
    };
    let acceleration = ph.pull.wrapping_mul(distance).wrapping_neg() / distance;
    let limited = |v: i32| {
        v.signum()
            .wrapping_mul(v.wrapping_abs().min(ph.speed_limit))
    };
    for (v, r) in p.velocity.iter_mut().zip([x, y]) {
        *v = limited(
            r.wrapping_mul(ticks)
                .wrapping_mul(acceleration)
                .wrapping_add(*v),
        );
    }
    p.facing = arctangent(p.velocity[0] >> 16, p.velocity[1] >> 16);
    p.relative[0] = x.wrapping_add(p.velocity[0].wrapping_mul(ticks) >> 16);
    p.relative[1] = y.wrapping_add(p.velocity[1].wrapping_mul(ticks) >> 16);
    if ph.on_ground {
        let ground = height(
            center[0].wrapping_add(p.relative[0]),
            center[1].wrapping_add(p.relative[1]),
        );
        p.relative[2] = i32::from(ph.height) - center[2] + ground;
    }
}

impl LocalProjectiles {
    pub fn new(seed: u32) -> Self {
        Self {
            definitions: BTreeMap::new(),
            of_projectile: BTreeMap::new(),
            groups: Vec::new(),
            projectiles: Vec::new(),
            first_free_projectile: 0,
            history: Vec::new(),
            random: LocalRandom::new(seed),
            media_effect: None,
            splashes: None,
            smoke_groups: [None; 3],
            flame_groups: [None; 5],
            mirror_stains: Vec::new(),
            stains: Vec::new(),
            collections: BTreeMap::new(),
            markers: Vec::new(),
            group_actions_run: 0,
            tick: 0,
            sounds: Vec::new(),
            monster_groups: BTreeMap::new(),
            rules: Rules::Myth2,
            serial: 0,
            view_cells: None,
        }
    }

    pub fn with_rules(seed: u32, rules: Rules) -> Self {
        Self {
            rules,
            ..Self::new(seed)
        }
    }

    pub fn random_mut(&mut self) -> &mut LocalRandom {
        &mut self.random
    }

    pub fn counts(&self) -> (usize, usize) {
        (
            self.projectiles.iter().flatten().count(),
            self.groups.iter().flatten().count(),
        )
    }

    pub fn add_definition(&mut self, definition: LocalDefinition, sequence: LocalSequence) {
        self.definitions
            .insert(definition.tag, (definition, sequence));
    }

    pub fn has_definition(&self, tag: FourCC) -> bool {
        self.definitions.contains_key(&tag)
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        self.projectiles.iter().flatten().count()
    }

    #[cfg(test)]
    pub fn group_states(&self) -> Vec<(i16, i16, u8, i16)> {
        self.groups
            .iter()
            .flatten()
            .map(|g| (g.wanted, g.count, g.state as u8, g.timer))
            .collect()
    }

    pub fn new_group(&mut self, new: &NewGroup) -> Option<usize> {
        let &(definition, sequence) = self.definitions.get(&new.definition)?;
        let index = match self.groups.iter().position(Option::is_none) {
            Some(i) => i,
            None if self.groups.len() < MAX_GROUPS => {
                self.groups.push(None);
                self.groups.len() - 1
            }
            None => return None,
        };
        let r = &mut self.random;
        let wanted = range16(r, definition.count);
        let mut start_period = 0;
        if definition.flags & FLAG_START_PERIOD != 0 {
            start_period = range16(r, definition.start_period);
        }
        let mut on_duration = 0;
        if definition.flags & FLAG_ON_DURATION != 0 {
            on_duration = range32(r, definition.on_duration);
        }
        let mut end_period = 0;
        if definition.flags & FLAG_END_PERIOD != 0 {
            end_period = range16(r, definition.end_period);
        }
        let mut center = new.point;
        center[2] = center[2].wrapping_add(i32::from(definition.height));
        self.groups[index] = Some(Group {
            definition,
            sequence,
            center,
            central: new.central,
            target: new.target,
            state: GroupState::Starting,
            timer: 0,
            wanted,
            count: 0,
            on_duration,
            start_period,
            end_period,
            opacity: new.opacity,
            facing: new.facing,
            visible: false,
            scale_x: -1,
            scale_y: -1,
            drawn_opacity: 0xffff,
            mirrored: false,
            stains: false,
            identifier: None,
            bounds: [center[0], center[1], center[0], center[1]],
        });
        if definition.flags & FLAG_START_PERIOD == 0 && self.rules == Rules::Myth2 {
            self.spawn(index, wanted);
        }
        Some(index)
    }

    fn spawn(&mut self, index: usize, target: i16) {
        self.spawn_into(index, target, None);
    }

    fn spawn_into(&mut self, index: usize, target: i16, mut bucket: Option<&mut Vec<usize>>) {
        let Some(g) = self.groups[index] else {
            return;
        };
        let missing = i32::from(target) - i32::from(g.count);
        if missing <= 0 {
            return;
        }
        let d = &g.definition;
        let tfl = self.rules == Rules::Tfl;
        for made in 0..missing {
            let r = &mut self.random;
            let motion = match d.physics {
                None => {
                    let speed = range32(r, d.outward);
                    let angle = if d.flags & FLAG_EVEN_ANGLES != 0 {
                        ((made << 16) / missing) as u16
                    } else {
                        r.next_u16()
                    };
                    let [x, y] = ring_point(angle, speed);
                    Motion::at_rest([x, y, 0], 0)
                }
                Some(p) => {
                    if tfl && d.outward.1 > 0 {
                        r.next_u16();
                    }
                    match p {
                        LocalPhysics::Drift(p) => start_drift(r, &g, &p),
                        LocalPhysics::Converge(p) => start_converge(r, d, &p, tfl),
                        LocalPhysics::Orbit(p) => {
                            let even = (d.flags & FLAG_EVEN_ANGLES != 0 && !tfl)
                                .then(|| ((made << 16) / missing) as u16);
                            start_orbit(r, d.outward, &p, even)
                        }
                        LocalPhysics::Wave(p) => start_wave(r, d.outward, &p),
                        LocalPhysics::Pull(_) => {
                            let speed = range32(r, d.outward);
                            let [x, y] = ring_point(r.next_u16(), speed);
                            Motion::at_rest([x, y, 0], arctangent(0, 0))
                        }
                    }
                }
            };
            if let Some(at) = self.new_projectile(index, motion)
                && let Some(b) = bucket.as_deref_mut()
            {
                b.push(at);
            }
        }
    }

    fn new_projectile(&mut self, group: usize, motion: Motion) -> Option<usize> {
        let start = self.first_free_projectile.min(self.projectiles.len());
        let at = match self.projectiles[start..].iter().position(Option::is_none) {
            Some(rel) => {
                let idx = start + rel;
                self.first_free_projectile = idx + 1;
                idx
            }
            None if self.projectiles.len() < MAX_PROJECTILES => {
                let idx = self.projectiles.len();
                self.projectiles.push(None);
                self.history.push(None);
                self.first_free_projectile = idx + 1;
                idx
            }
            None => {
                self.first_free_projectile = self.projectiles.len();
                return None;
            }
        };
        let g = self.groups[group].as_mut()?;
        let (d, s, stains) = (g.definition, g.sequence, g.stains);
        let r = &mut self.random;
        let speed = range16(r, d.speed);
        let ticks_per_frame = ((i32::from(s.ticks_per_frame) * i32::from(speed)) >> 8).max(1);
        let (mut frame, mut tick) = (0, 0);
        if d.flags & FLAG_FIRST_FRAME == 0 {
            if s.frames >= 1 {
                frame = ((i32::from(s.frames) * i32::from(r.next_u16())) >> 16) as i16;
            }
            if s.ticks_per_frame > 0 {
                tick = ((i32::from(s.ticks_per_frame) * i32::from(r.next_u16())) >> 16) as i16;
            }
        }
        let scale = range16(r, d.scale);
        let permutation = match self.rules {
            Rules::Myth2 => (i32::from(r.next_u16()) % i32::from(s.permutations.max(1))) as i16,
            Rules::Tfl => 0,
        };
        let (mut age, mut lifetime) = (0, 0);
        if d.flags & FLAG_LIFETIME != 0 {
            age = 0;
            lifetime = match self.rules {
                Rules::Tfl if d.lifetime.1 != 0 => d
                    .lifetime
                    .0
                    .wrapping_add(((u32::from(r.next_u16()) * d.lifetime.1 as u32) >> 16) as i32),
                _ => range32(r, d.lifetime),
            };
        }
        let serial = self.serial;
        self.serial = self.serial.wrapping_add(1);
        g.count += 1;
        self.history[at] = None;
        self.projectiles[at] = Some(Projectile {
            group,
            velocity: motion.velocity,
            period: motion.period,
            clock: motion.clock,
            rising: 0,
            relative: motion.relative,
            world: None,
            facing: motion.facing,
            scale,
            frame,
            tick,
            ticks_per_frame: ticks_per_frame as i16,
            permutation,
            age,
            lifetime,
            serial,
        });
        if stains {
            self.stain_at_mirrors(group, &d, motion.relative);
        }
        Some(at)
    }

    fn stain_at_mirrors(&mut self, group: usize, d: &LocalDefinition, relative: [i32; 3]) {
        if d.stain_sequence == -1 {
            return;
        }
        let Some(&collection) = self.collections.get(&d.collection_reference) else {
            return;
        };
        for at in 0..self.mirror_stains.len() {
            let Some((mirror, left)) = self.mirror_stains[at] else {
                continue;
            };
            if left <= 0 || self.mirror_group(&mirror) != Some(group) {
                continue;
            }
            let random = self.random.next_u16();
            self.stains.push(Stain {
                collection,
                sequence: d.stain_sequence,
                point: [
                    mirror.point[0].wrapping_add(relative[0]),
                    mirror.point[1].wrapping_add(relative[1]),
                    0,
                ],
                random,
            });
            self.mirror_stains[at] = Some((mirror, left - 1));
        }
    }

    pub fn take_stains(&mut self) -> Vec<Stain> {
        std::mem::take(&mut self.stains)
    }

    fn follow_mirrors(&mut self, mirrors: &[Option<MirrorGroup>]) {
        let same = |a: &MirrorGroup, b: &MirrorGroup| {
            (a.flame, a.slot, a.point) == (b.flame, b.slot, b.point)
        };
        self.mirror_stains.resize(mirrors.len(), None);
        for (kept, mirror) in self.mirror_stains.iter_mut().zip(mirrors) {
            *kept = mirror.map(|m| {
                let left = kept
                    .filter(|(k, _)| same(k, &m))
                    .map_or(7, |(_, left)| left);
                (m, left)
            });
        }
    }

    fn delete_projectile(&mut self, at: usize) {
        let Some(p) = self.projectiles[at].take() else {
            return;
        };
        self.history[at] = None;
        self.first_free_projectile = self.first_free_projectile.min(at);
        let Some(g) = self.groups[p.group].as_mut() else {
            return;
        };
        g.count -= 1;
        let flags = g.definition.flags;
        if flags & FLAG_ONE_SHOT != 0 {
            g.wanted -= 1;
        }
        if flags & 0x10081 == 0x10001
            && g.count == 0
            && g.wanted == 0
            && g.state != GroupState::Ending
        {
            g.state = GroupState::Ending;
            g.timer = 0;
        }
    }

    fn delete_group(&mut self, index: usize) {
        if let Some(id) = self.groups[index].and_then(|g| g.identifier)
            && let Some(m) = self.markers.iter_mut().find(|m| m.identifier == id)
        {
            m.group = None;
        }
        self.monster_groups.retain(|_, g| *g != index);
        if self.media_effect.is_some_and(|(a, _)| a.group == index) {
            self.media_effect = None;
        }
        if self.splashes.is_some_and(|a| a.group == index) {
            self.splashes = None;
        }
        for slot in &mut self.smoke_groups {
            if *slot == Some(index) {
                *slot = None;
            }
        }
        for slot in &mut self.flame_groups {
            if *slot == Some(index) {
                *slot = None;
            }
        }
        for at in 0..self.projectiles.len() {
            if self.projectiles[at].is_some_and(|p| p.group == index) {
                self.projectiles[at] = None;
                self.history[at] = None;
                self.first_free_projectile = self.first_free_projectile.min(at);
            }
        }
        self.groups[index] = None;
    }

    fn move_group_slots(&mut self, index: usize, point: [i32; 3], slots: &[usize]) {
        let Some(g) = self.groups[index].as_mut() else {
            return;
        };
        let mut to = point;
        to[2] = to[2].wrapping_add(i32::from(g.definition.height));
        let delta = [0, 1, 2].map(|i| g.center[i].wrapping_sub(to[i]));
        g.center = to;
        if g.definition.flags & FLAG_FOLLOW != 0 {
            return;
        }
        for &at in slots {
            if let Some(p) = self.projectiles[at].as_mut() {
                for (r, d) in p.relative.iter_mut().zip(delta) {
                    *r = r.wrapping_add(d);
                }
            }
        }
    }

    fn group_clock(&mut self, index: usize, ticks: i32) -> Option<(i16, bool)> {
        let g = self.groups[index].as_mut()?;
        let d = g.definition;
        let flags = d.flags;
        let mut count = g.wanted;
        let opacity_in = u32::from(g.opacity);
        g.scale_x = -1;
        g.scale_y = -1;
        let lerp = |a: i16, b: i16, t: i32| -> i16 {
            a.wrapping_add(((i32::from(b.wrapping_sub(a)) * t) >> 16) as i16)
        };
        let mut factor: u32 = 0xffff;
        match g.state {
            GroupState::On => {
                if flags & FLAG_ON_DURATION != 0 {
                    g.on_duration -= ticks;
                    if g.on_duration < 0 {
                        g.state = GroupState::Ending;
                        g.timer = 0;
                    }
                }
            }
            GroupState::Starting => {
                g.timer = g.timer.wrapping_add(ticks as i16);
                if flags & FLAG_START_PERIOD == 0 || g.start_period <= g.timer {
                    g.state = GroupState::On;
                    g.timer = 0;
                    g.drawn_opacity = ((opacity_in * 0xffff) >> 16) as u16;
                    return Some((count, false));
                }
                let t = (i32::from(g.timer as u16) << 16) / i32::from(g.start_period);
                if flags & FLAG_START_OPACITY != 0 {
                    factor = 0xffff - (!(t as u32) & 0xffff);
                }
                if flags & FLAG_START_COUNT != 0 {
                    count = ((i32::from(count) * t) >> 16) as i16;
                }
                if flags & FLAG_START_SCALE_X != 0 {
                    g.scale_x = lerp(d.start_scale[0].0, d.start_scale[0].1, t);
                }
                if flags & FLAG_START_SCALE_Y != 0 {
                    g.scale_y = lerp(d.start_scale[1].0, d.start_scale[1].1, t);
                }
            }
            GroupState::Ending => {
                g.timer = g.timer.wrapping_add(ticks as i16);
                let t = if flags & FLAG_END_PERIOD == 0 {
                    None
                } else if flags & FLAG_END_WHEN_EMPTY == 0 {
                    if g.end_period == 0 {
                        Some(0)
                    } else if g.end_period <= g.timer {
                        None
                    } else {
                        Some((i32::from(g.timer as u16) << 16) / i32::from(g.end_period))
                    }
                } else if g.end_period != 0 {
                    if g.count == 0 || g.end_period <= g.timer {
                        None
                    } else {
                        Some((i32::from(g.timer as u16) << 16) / i32::from(g.end_period))
                    }
                } else if g.count == 0 {
                    None
                } else {
                    Some(0)
                };
                let Some(t) = t else {
                    self.delete_group(index);
                    return Some((count, true));
                };
                if flags & FLAG_END_OPACITY != 0 {
                    factor = !(t as u32) & 0xffff;
                }
                if flags & FLAG_END_NONE != 0 {
                    count = 0;
                } else if flags & FLAG_END_COUNT != 0 {
                    count = ((i32::from(count) * t) >> 16) as i16;
                }
                if flags & FLAG_END_SCALE_X != 0 {
                    g.scale_x = lerp(d.end_scale[0].0, d.end_scale[0].1, t);
                }
                if flags & FLAG_END_SCALE_Y != 0 {
                    g.scale_y = lerp(d.end_scale[1].0, d.end_scale[1].1, t);
                }
            }
        }
        g.drawn_opacity = ((opacity_in * factor) >> 16) as u16;
        Some((count, false))
    }

    pub fn update(&mut self, ticks: i32, world: &LocalWorld<'_>) {
        let ticks = ticks.clamp(0, 6);
        let mut by_group: Vec<Vec<usize>> = vec![Vec::new(); self.groups.len()];
        for (at, p) in self.projectiles.iter().enumerate() {
            if let Some(p) = p
                && let Some(bucket) = by_group.get_mut(p.group)
            {
                bucket.push(at);
            }
        }
        for index in 0..self.groups.len() {
            let Some(g) = self.groups[index] else {
                continue;
            };
            let bucket = &mut by_group[index];
            let mut unseen = false;
            if let Some(central) = g.central {
                match (world.object)(central) {
                    Some(point) => {
                        self.move_group_slots(index, point, bucket);
                        if let Some(bits) = (world.cell_bits)(central, point) {
                            let g = self.groups[index].as_mut().expect("group");
                            g.opacity = match bits {
                                b if b & 1 == 0 => 0,
                                b if b & 2 == 0 => 0x8000,
                                _ => 0xffff,
                            };
                            unseen = bits & 1 == 0;
                            if unseen && g.definition.flags & FLAG_UPDATE_UNSEEN == 0 {
                                g.visible = false;
                                g.scale_x = -1;
                                g.scale_y = -1;
                                g.drawn_opacity = 0;
                                let center = g.center;
                                for &at in bucket.iter() {
                                    if let Some(p) = self.projectiles[at].as_mut() {
                                        p.world = Some(
                                            [0, 1, 2]
                                                .map(|i| center[i].wrapping_add(p.relative[i])),
                                        );
                                    }
                                }
                                continue;
                            }
                        }
                    }
                    None => {
                        let g = self.groups[index].as_mut().expect("group");
                        if g.state != GroupState::Ending {
                            g.state = GroupState::Ending;
                            g.timer = 0;
                        }
                    }
                }
            }
            let meets = self.meets_view(&g);
            if !meets && g.definition.flags & FLAG_UPDATE_UNSEEN == 0 {
                if let Some(g) = self.groups[index].as_mut() {
                    g.visible = false;
                }
                continue;
            }
            let Some((target, deleted)) = self.group_clock(index, ticks) else {
                continue;
            };
            if deleted {
                continue;
            }
            if self.rules == Rules::Tfl
                && let Some(g) = self.groups[index].as_mut()
            {
                g.bounds = [g.center[0], g.center[1], g.center[0], g.center[1]];
            }
            let (runs, step) = if g.definition.flags & FLAG_TICK_BY_TICK != 0 {
                (ticks, 1)
            } else {
                (1, ticks)
            };
            let mut ordered_for_tfl = false;
            for _ in 0..runs {
                let before = bucket.len();
                self.spawn_into(index, target, Some(bucket));
                let spawned = bucket.len() > before;
                if self.rules == Rules::Tfl {
                    if !ordered_for_tfl || spawned {
                        bucket.sort_unstable_by_key(|&at| {
                            std::cmp::Reverse(self.projectiles[at].map_or(0, |p| p.serial))
                        });
                        ordered_for_tfl = true;
                    }
                } else if spawned {
                    bucket.sort_unstable();
                }
                for &at in bucket.iter() {
                    self.update_projectile(at, step, world);
                }
                bucket.retain(|&at| self.projectiles[at].is_some());
            }
            let Some(g) = self.groups[index].as_mut() else {
                continue;
            };
            g.visible = !g.mirrored && !unseen && meets;
            let (center, visible) = (g.center, g.visible);
            for &at in bucket.iter() {
                if let Some(p) = self.projectiles[at].as_mut() {
                    p.world = Some([0, 1, 2].map(|i| center[i].wrapping_add(p.relative[i])));
                }
            }
            if visible {
                self.tfl_draws(index, bucket, world);
            }
        }
    }

    fn tfl_draws(&mut self, index: usize, slots: &[usize], world: &LocalWorld<'_>) {
        let (Some(drawn), Some(cell)) = (world.tfl_drawn, world.tfl_cell) else {
            return;
        };
        let Some(g) = self.groups[index].as_ref() else {
            return;
        };
        let flags = g.definition.flags;
        let media = flags & FLAG_MEDIA_SURFACE != 0;
        for &at in slots {
            let Some(w) = self.projectiles[at].and_then(|p| p.world) else {
                continue;
            };
            let z = if media { 0 } else { w[2] };
            let above = if media || flags & FLAG_BELOW_GROUND != 0 {
                0
            } else {
                z - i32::from((world.height)(w[0], w[1]) as i16)
            };
            if above < 0 {
                continue;
            }
            if !media && drawn([w[0], w[1], z]) {
                self.random.next_u16();
            }
            if flags & (FLAG_REFLECTED | FLAG_MEDIA_SURFACE) != 0
                && cell(w[0], w[1]).0 & 0x38 != 0
                && drawn([w[0], w[1], if z > 0 { -z } else { z }])
            {
                self.random.next_u16();
            }
        }
    }

    fn meets_view(&self, g: &Group) -> bool {
        let (Rules::Tfl, Some([c0, c1, r0, r1])) = (self.rules, self.view_cells) else {
            return true;
        };
        let b = g.bounds;
        (b[2] >> 10).min(c1) >= (b[0] >> 10).max(c0) && (b[3] >> 10).min(r1) >= (b[1] >> 10).max(r0)
    }

    pub fn set_view_cells(&mut self, cells: Option<[i32; 4]>) {
        self.view_cells = cells;
    }

    fn grow_bounds(&mut self, group: usize, [x, y]: [i32; 2]) {
        if self.rules != Rules::Tfl {
            return;
        }
        if let Some(g) = self.groups[group].as_mut() {
            let b = &mut g.bounds;
            *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        }
    }

    fn update_projectile(&mut self, at: usize, ticks: i32, world: &LocalWorld<'_>) {
        let Some(mut p) = self.projectiles[at] else {
            return;
        };
        let Some(g) = self.groups[p.group] else {
            return;
        };
        let d = g.definition;
        let (check_ground, on_map, point) = if let Some(ph) = d.physics {
            match ph {
                LocalPhysics::Orbit(ph) => move_orbit(&mut p, &ph, ticks, self.rules == Rules::Tfl),
                LocalPhysics::Wave(ph) => move_wave(&mut p, &ph, ticks),
                LocalPhysics::Converge(ph) => move_converge(&mut p, &ph, ticks),
                LocalPhysics::Pull(ph) => {
                    let height = |x: i32, y: i32| match world.tfl_cell {
                        Some(cell) => {
                            let (bits, pixel) = cell(x, y);
                            if bits & TFL_CELL_WATER != 0 || (bits & TFL_CELL_SHORE != 0 && pixel) {
                                0
                            } else {
                                (world.height)(x, y)
                            }
                        }
                        None => (world.height)(x, y),
                    };
                    move_pull(&mut p, &ph, ticks, g.center, &height)
                }
                LocalPhysics::Drift(ph) => {
                    let r = &mut self.random;
                    let jitter = |r: &mut LocalRandom, a: i32| {
                        range32(r, (a.wrapping_neg() >> 8, a.wrapping_mul(2) >> 8))
                            .wrapping_mul(0x100)
                    };
                    let (dx, dy, dz) = (
                        jitter(r, ph.jitter[0]),
                        jitter(r, ph.jitter[1]),
                        jitter(r, ph.jitter[2]),
                    );
                    let mut ax = ph.acceleration[0].wrapping_add(dx);
                    let mut ay = ph.acceleration[1].wrapping_add(dy);
                    let az = ph.acceleration[2].wrapping_add(dz);
                    if ph.flags & 2 != 0 {
                        let (x, y) = (i32::from((ax >> 8) as i16), i32::from((ay >> 8) as i16));
                        let m = short_speed(isqrt((x * x + y * y) as u32));
                        let i = trig_index(arctangent(x, y).wrapping_add(g.facing));
                        let trig = trig_tables();
                        ax = (i32::from(trig.cosine[i]).wrapping_mul(m) >> 14) << 8;
                        ay = (i32::from(trig.sine[i]).wrapping_mul(m) >> 14) << 8;
                    }
                    p.velocity[2] = p.velocity[2].wrapping_add(az.wrapping_mul(ticks));
                    p.velocity[0] = p.velocity[0].wrapping_add(ax.wrapping_mul(ticks));
                    p.velocity[1] = p.velocity[1].wrapping_add(ay.wrapping_mul(ticks));
                    p.facing = arctangent(p.velocity[0] >> 16, p.velocity[1] >> 16);
                    let wind = world.wind.filter(|_| d.flags & FLAG_WIND != 0);
                    for i in 0..3 {
                        let v = p.velocity[i].wrapping_add(wind.map_or(0, |w| w[i]));
                        p.relative[i] = (v.wrapping_mul(ticks) >> 16).wrapping_add(p.relative[i]);
                    }
                }
            }
            let point = [0, 1, 2].map(|i| p.relative[i].wrapping_add(g.center[i]));
            let on_map = point[0] >= 0
                && point[0] < world.size[0]
                && point[1] >= 0
                && point[1] < world.size[1];
            self.grow_bounds(
                p.group,
                [0, 1].map(|i| point[i].clamp(0, world.size[i] - 1)),
            );
            (true, on_map, point)
        } else {
            self.grow_bounds(
                p.group,
                [0, 1].map(|i| p.relative[i].wrapping_add(g.center[i])),
            );
            (false, true, [0; 3])
        };
        let mut delete = !on_map;
        if d.flags & (FLAG_LIFETIME | FLAG_DELETE_ON_WRAP) != 0 {
            p.age += ticks;
            if on_map && d.flags & FLAG_LIFETIME != 0 && p.lifetime < p.age {
                delete = true;
            }
        }
        if check_ground && on_map && !delete && d.flags & FLAG_GROUND != 0 {
            let ground = (world.height)(point[0], point[1]);
            let on_media = world.tfl_cell.is_some_and(|cell| {
                let (bits, pixel) = cell(point[0], point[1]);
                bits & (TFL_CELL_WATER | TFL_CELL_SHORE) != 0 && pixel
            });
            if point[2] < ground || (on_media && point[2] < 0) {
                delete = true;
            }
        }
        if !delete && d.flags & FLAG_NO_ANIMATION == 0 {
            delete =
                self.animate_and_sound(&mut p, &g, ticks) && d.flags & FLAG_DELETE_ON_WRAP != 0;
        }
        self.projectiles[at] = Some(p);
        if delete {
            self.delete_projectile(at);
        }
    }

    fn animate_and_sound(&mut self, p: &mut Projectile, g: &Group, ticks: i32) -> bool {
        let mut wrapped = false;
        for _ in 0..ticks {
            let tick = animate(p, &g.sequence);
            wrapped |= tick.wrapped;
            let sounds = AnimSounds {
                sequence: g.definition.sequence,
                first: tick.first && !tick.wrapped && !tick.key && g.sequence.sounds[0],
                key: tick.key && g.sequence.sounds[1],
                last: tick.wrapped && !tick.key && g.sequence.sounds[2],
            };
            if (sounds.first || sounds.key || sounds.last) && self.sounds.len() < SOUNDS_KEPT {
                let collection = g.definition.collection_reference;
                self.sounds.push(SequenceSound {
                    tick: self.tick,
                    object: -1,
                    identifier: -1,
                    collection: self
                        .collections
                        .get(&collection)
                        .copied()
                        .unwrap_or(collection),
                    sounds,
                    world: p.world.unwrap_or([0; 3]),
                    request_flags: 0,
                    velocity: [0; 3],
                });
            }
        }
        wrapped
    }

    pub fn take_sounds(&mut self) -> Vec<SequenceSound> {
        std::mem::take(&mut self.sounds)
    }

    pub fn record_history(&mut self) {
        for (h, p) in self.history.iter_mut().zip(&self.projectiles) {
            *h = p.filter(|p| p.world.is_some());
        }
    }

    pub fn surfaces(&self, fraction: i32, scene: &SurfaceView<'_>) -> Vec<LocalSurface> {
        let mut out = Vec::new();
        for (live, history) in self.projectiles.iter().zip(&self.history) {
            let Some(live) = live else { continue };
            let Some(live_world) = live.world else {
                continue;
            };
            let Some(g) = self.groups[live.group].as_ref() else {
                continue;
            };
            if !g.visible {
                continue;
            }
            if g.definition.flags & FLAG_MEDIA_SURFACE != 0
                && !match self.rules {
                    Rules::Myth2 => scene
                        .region
                        .is_some_and(|r| in_cell_region(r, live_world[0] >> 9, live_world[1] >> 9)),
                    Rules::Tfl => (scene.reflective)(live_world[0], live_world[1]),
                }
            {
                continue;
            }
            let (drawn, mut world) = interpolated(live, history.as_ref(), live_world, fraction);
            if g.definition.flags & FLAG_MEDIA_SURFACE != 0 {
                world[2] = (scene.media_height)(world[0], world[1]);
            }
            let own = surface(
                g,
                live,
                &drawn,
                world,
                g.drawn_opacity,
                (g.scale_x, g.scale_y),
                scene,
                false,
            );
            out.extend(own);
            if reflected_at(self.rules, g, live_world, scene) {
                out.extend(surface(
                    g,
                    live,
                    &drawn,
                    world,
                    g.drawn_opacity,
                    (g.scale_x, g.scale_y),
                    scene,
                    true,
                ));
            }
        }
        self.mirror_surfaces(fraction, scene, &mut out);
        out
    }

    fn mirror_surfaces(&self, fraction: i32, scene: &SurfaceView<'_>, out: &mut Vec<LocalSurface>) {
        if !scene.mirrors.iter().any(Option::is_some) {
            return;
        }
        let mut smoke_slots: [Vec<usize>; 3] = Default::default();
        let mut flame_slots: [Vec<usize>; 5] = Default::default();
        for (at, p) in self.projectiles.iter().enumerate() {
            let Some(p) = p else {
                continue;
            };
            if let Some(s) = self.smoke_groups.iter().position(|&g| g == Some(p.group)) {
                smoke_slots[s].push(at);
            } else if let Some(s) = self.flame_groups.iter().position(|&g| g == Some(p.group)) {
                flame_slots[s].push(at);
            }
        }
        for mirror in scene.mirrors.iter().flatten() {
            let Some(index) = self.mirror_group(mirror) else {
                continue;
            };
            let Some(g) = self.groups[index].as_ref() else {
                continue;
            };
            if g.definition.flags & FLAG_MEDIA_SURFACE != 0 {
                continue;
            }
            let slot = usize::from(mirror.slot);
            let slots = if mirror.flame {
                &flame_slots[slot]
            } else {
                &smoke_slots[slot]
            };
            for &at in slots {
                let Some(live) = self.projectiles[at].as_ref() else {
                    continue;
                };
                let Some(live_world) = live.world else {
                    continue;
                };
                let history = self.history[at].as_ref();
                let (drawn, _) = interpolated(live, history, live_world, fraction);
                let world = [0, 1, 2].map(|i| mirror.point[i].wrapping_add(live.relative[i]));
                for reflection in [false, true] {
                    if reflection && !reflected_at(self.rules, g, world, scene) {
                        continue;
                    }
                    out.extend(surface(
                        g,
                        live,
                        &drawn,
                        world,
                        mirror.strength as u16,
                        (-1, -1),
                        scene,
                        reflection,
                    ));
                }
            }
        }
    }

    fn mirror_group(&self, mirror: &MirrorGroup) -> Option<usize> {
        let slot = usize::from(mirror.slot);
        if mirror.flame {
            *self.flame_groups.get(slot)?
        } else {
            *self.smoke_groups.get(slot)?
        }
    }
}

fn reflected_at(rules: Rules, g: &Group, at: [i32; 3], scene: &SurfaceView<'_>) -> bool {
    let (col, row) = (at[0] >> 9, at[1] >> 9);
    g.definition.flags & FLAG_REFLECTED != 0
        && g.definition.flags & FLAG_MEDIA_SURFACE == 0
        && (rules == Rules::Tfl || scene.region.is_some_and(|r| in_cell_region(r, col, row)))
        && (scene.reflective)(at[0], at[1])
}

fn interpolated(
    live: &Projectile,
    history: Option<&Projectile>,
    live_world: [i32; 3],
    fraction: i32,
) -> (Projectile, [i32; 3]) {
    let mut drawn = *live;
    let mut world = live_world;
    if let Some(h) = history.filter(|_| fraction <= 0xffff)
        && let Some(from) = h.world
    {
        world = [0, 1, 2].map(|i| {
            from[i] + ((i64::from(live_world[i] - from[i]) * i64::from(fraction)) >> 16) as i32
        });
        let turn = signed_angular_difference(h.facing, live.facing);
        drawn.facing = h.facing.wrapping_add(((turn * fraction) >> 16) as u16);
        drawn.scale = h
            .scale
            .wrapping_add(((i32::from(live.scale - h.scale) * fraction) >> 16) as i16);
        if fraction <= 0x7fff {
            drawn.frame = h.frame;
            drawn.tick = h.tick;
            drawn.ticks_per_frame = h.ticks_per_frame;
            drawn.age = h.age;
            drawn.permutation = h.permutation;
        }
    }
    (drawn, world)
}

#[allow(clippy::too_many_arguments)]
fn surface(
    g: &Group,
    live: &Projectile,
    drawn: &Projectile,
    world: [i32; 3],
    opacity: u16,
    (scale_x, scale_y): (i16, i16),
    scene: &SurfaceView<'_>,
    reflection: bool,
) -> Option<LocalSurface> {
    let flags = g.definition.flags;
    let media_surface = flags & FLAG_MEDIA_SURFACE != 0;
    let above_ground = if media_surface || flags & FLAG_BELOW_GROUND != 0 {
        0
    } else {
        world[2] - (scene.height)(world[0], world[1])
    };
    if above_ground < 0 {
        return None;
    }
    let eye = scene.eye;
    let s = &g.sequence;
    let view = if s.stored_view {
        0
    } else {
        let a = arctangent(eye[0] - world[0], eye[1] - world[1]);
        let u = u32::from(a.wrapping_sub(drawn.facing));
        let v = ((u * s.views.max(1) as u32 + 0x8000) >> 16) as i16;
        if v == s.views { 0 } else { v }
    };
    let opacity = u32::from(opacity);
    let elapsed = i32::from(live.frame) * i32::from(live.ticks_per_frame) + i32::from(live.tick);
    let total = i32::from(s.frames) * i32::from(live.ticks_per_frame);
    let ratio = |n: i32, of: i32| ((n << 16) / of) as u32 & 0xffff;
    let full = (opacity * 0xffff) >> 16;
    let alpha = match g.definition.transfer {
        1 if live.age < live.lifetime => (ratio(live.age, live.lifetime) * opacity) >> 16,
        2 if elapsed < total => (ratio(elapsed, total) * opacity) >> 16,
        3 if live.age < live.lifetime => {
            ((!ratio(live.age, live.lifetime) & 0xffff) * opacity) >> 16
        }
        3 => 0,
        4 if elapsed < total => ((!ratio(elapsed, total) & 0xffff) * opacity) >> 16,
        4 => 0,
        _ => full,
    };
    let alpha = if media_surface || reflection {
        (alpha * u32::from(!scene.media_transparency)) >> 16
    } else {
        alpha
    };
    let scale = |v: i16| if v == -1 { drawn.scale } else { v };
    Some(LocalSurface {
        collection_reference: g.definition.collection_reference,
        sequence: g.definition.sequence,
        frame: drawn.frame,
        view,
        world,
        above_ground,
        facing: drawn.facing,
        transparency: !(alpha as u16),
        scale_x: scale(scale_x),
        scale_y: scale(scale_y),
        velocity: drawn.velocity.map(|v| (v >> 16) as i16),
        permutation: drawn.permutation,
        media_surface,
        reflection: reflection.then(|| (scene.media_height)(world[0], world[1]) as i16),
    })
}

pub type TagPayload<'a> = dyn Fn(&[u8; 4], FourCC) -> Option<Vec<u8>> + 'a;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalSpriteSource {
    pub reference: FourCC,
    pub collection: FourCC,
    pub sequence: u8,
    pub stain_sequence: i16,
    pub core: CoreRef,
}

impl LocalProjectiles {
    pub fn for_map(
        catalog: &ProjectileCatalog,
        ambient: &[FourCC],
        tag: &TagPayload<'_>,
        seed: u32,
    ) -> (Self, Vec<LocalSpriteSource>) {
        let mut local = Self::new(seed);
        let mut sources = Vec::new();
        let mut projs: Vec<_> = catalog.projectiles.values().collect();
        projs.sort_by_key(|p| p.tag_id.as_u32());
        for p in projs {
            if let Some(id) = projectile_local_group(p) {
                local.of_projectile.insert(p.tag_id, id);
            }
        }
        let mut ids: Vec<FourCC> = local
            .of_projectile
            .values()
            .copied()
            .chain(catalog.groups.values().filter_map(|g| g.local_group))
            .chain(std::iter::once(mandrake_sim::combat::CONFUSION_LOCAL_GROUP))
            .chain(ambient.iter().copied())
            .collect();
        ids.sort_unstable_by_key(|i| i.as_u32());
        ids.dedup();
        for id in ids {
            let Some(data) = tag(b"lpgr", id) else {
                tracing::debug!("[local projectiles] '{id}': not loaded");
                continue;
            };
            let Some(definition) = LocalDefinition::parse(id, &data, |ph| tag(b"phys", ph)) else {
                tracing::debug!(
                    "[local projectiles] '{id}': its phys is missing or of a type above 4"
                );
                continue;
            };
            let reference = definition.collection_reference;
            let core = tag(b"core", reference);
            let collection = core
                .as_ref()
                .and_then(|c| Some(FourCC::new(c.get(0..4)?.try_into().ok()?)))
                .filter(|c| !matches!(u32::from_be_bytes(c.0), 0 | 0xffff_ffff))
                .unwrap_or(reference);
            local.collections.insert(reference, collection);
            let core_ref = core
                .as_deref()
                .map_or_else(Default::default, CoreRef::from_core_bytes);
            let permutations = core
                .as_ref()
                .and_then(|c| Some(i16::from_be_bytes(c.get(4..6)?.try_into().ok()?)))
                .unwrap_or(1);
            let Some(data) = tag(b".256", collection) else {
                continue;
            };
            let Some(h) = sequence_headers(&data)
                .ok()
                .and_then(|q| q.get(usize::from(definition.sequence)).copied())
            else {
                continue;
            };
            let sounds = crate::map_sound::sequence_sound_tags(&data)
                .and_then(|q| q.get(usize::from(definition.sequence)).copied())
                .map_or([false; 3], |t| t.map(|s| s.is_some()));
            local.add_definition(
                definition,
                LocalSequence {
                    frames: h.frames_per_view,
                    ticks_per_frame: h.ticks_per_frame,
                    key_frame: h.key_frame,
                    loop_frame: h.effective_loop_frame(),
                    stored_view: h.flags & 4 != 0,
                    views: h.number_of_views,
                    permutations,
                    sounds,
                },
            );
            sources.push(LocalSpriteSource {
                reference,
                collection,
                sequence: definition.sequence,
                stain_sequence: definition.stain_sequence,
                core: core_ref,
            });
        }
        (local, sources)
    }

    pub fn start_for_event(&mut self, event: &CombatEvent) {
        match event {
            CombatEvent::ProjectileCreated {
                definition,
                object,
                identifier,
                point,
                facing,
                ..
            } => {
                if let Some(&id) = self.of_projectile.get(definition) {
                    self.new_group(&NewGroup {
                        definition: id,
                        point: *point,
                        central: Some(ObjectRef {
                            index: *object,
                            identifier: *identifier,
                        }),
                        target: None,
                        opacity: 0xffff,
                        facing: *facing,
                    });
                }
            }
            CombatEvent::MonsterLocalGroup {
                monster,
                local_group,
                object,
                identifier,
                ..
            } => {
                let group = self.new_group(&NewGroup {
                    definition: *local_group,
                    point: [0; 3],
                    central: Some(ObjectRef {
                        index: *object,
                        identifier: *identifier,
                    }),
                    target: None,
                    opacity: 0xffff,
                    facing: 0,
                });
                match group {
                    Some(g) => self.monster_groups.insert(*monster, g),
                    None => self.monster_groups.remove(monster),
                };
            }
            CombatEvent::MonsterLocalGroupOff { monster, .. } => {
                if let Some(index) = self.monster_groups.remove(monster) {
                    self.turn_off_group(index);
                }
            }
            CombatEvent::ProjectileGroupLocal {
                local_group,
                point,
                facing,
                object_point,
                cylinder,
                ..
            } => {
                self.new_group(&NewGroup {
                    definition: *local_group,
                    point: *point,
                    central: None,
                    target: Some(TargetCylinder {
                        point: *object_point,
                        radius: cylinder[0],
                        bottom: cylinder[1],
                        top: cylinder[2],
                    }),
                    opacity: 0xffff,
                    facing: *facing,
                });
            }
            _ => {}
        }
    }

    fn start_ambient_group(&mut self, source: AmbientSource, size: [i32; 2]) -> Option<Ambient> {
        let group = self.new_group(&NewGroup {
            definition: source.group,
            point: [size[0] >> 1, size[1] >> 1, 0],
            central: None,
            target: None,
            opacity: 0xffff,
            facing: 0,
        })?;
        if let Some(g) = self.groups[group].as_mut() {
            g.wanted = -1;
        }
        Some(Ambient {
            group,
            tries: source.tries,
            period: source.period,
            timer: -1,
        })
    }

    fn start_fire(&mut self, size: [i32; 2]) {
        let [smoke, flame] = FIRE_GROUPS;
        let mut start = |definition| {
            let group = self.new_group(&NewGroup {
                definition,
                point: [size[0] >> 1, size[1] >> 1, 0],
                central: None,
                target: None,
                opacity: 0xffff,
                facing: 0,
            })?;
            if let Some(g) = self.groups[group].as_mut() {
                g.mirrored = true;
                g.stains = definition == flame;
            }
            Some(group)
        };
        let smoke_groups = [(); 3].map(|()| start(smoke));
        let flame_groups = [(); 5].map(|()| start(flame));
        self.smoke_groups = smoke_groups;
        self.flame_groups = flame_groups;
    }

    pub fn start_ambient(
        &mut self,
        media_effect: Option<(AmbientSource, u16)>,
        fire: bool,
        splashes: Option<AmbientSource>,
        size: [i32; 2],
    ) {
        self.media_effect =
            media_effect.and_then(|(s, chance)| Some((self.start_ambient_group(s, size)?, chance)));
        if fire {
            self.start_fire(size);
        }
        self.splashes = splashes.and_then(|s| self.start_ambient_group(s, size));
    }

    pub fn start_markers(&mut self, palettes: &[MarkerPaletteEntry], markers: &[MarkerEntry]) {
        let palette: Vec<FourCC> = palettes
            .iter()
            .filter(|p| p.marker_type == MarkerPaletteEntry::TYPE_WAYPOINT)
            .map(|p| p.tag_id)
            .collect();
        for m in markers
            .iter()
            .filter(|m| m.marker_type == MarkerPaletteEntry::TYPE_WAYPOINT)
        {
            let Some(&definition) = usize::try_from(m.palette_index)
                .ok()
                .and_then(|i| palette.get(i))
            else {
                continue;
            };
            self.markers.push(LocalMarker {
                identifier: m.marker_id,
                definition,
                point: [m.x, m.y, m.z],
                facing: i16::from_be_bytes([m.type_data[0], m.type_data[1]]) as u16,
                group: None,
            });
            if m.flags & 1 == 0 {
                self.start_marker(self.markers.len() - 1);
            }
        }
    }

    fn start_marker(&mut self, at: usize) {
        let m = self.markers[at];
        let Some(group) = self.new_group(&NewGroup {
            definition: m.definition,
            point: m.point,
            central: None,
            target: None,
            opacity: 0xffff,
            facing: m.facing,
        }) else {
            return;
        };
        if let Some(g) = self.groups[group].as_mut() {
            g.identifier = Some(m.identifier);
        }
        self.markers[at].group = Some(group);
    }

    fn run_group_action(&mut self, action: &LocalGroupAction) {
        if action.on {
            for &id in &action.groups {
                if let Some(at) = self.markers.iter().position(|m| m.identifier == id)
                    && self.markers[at].group.is_none()
                {
                    self.start_marker(at);
                }
            }
        }
        for &id in &action.groups {
            let Some(g) = self
                .groups
                .iter_mut()
                .flatten()
                .find(|g| g.identifier == Some(id))
            else {
                continue;
            };
            if let Some(opacity) = action.opacity {
                g.opacity = opacity.clamp(0, 0xffff) as u16;
            }
            if action.off && g.state != GroupState::Ending {
                g.state = GroupState::Ending;
                g.timer = 0;
            }
        }
    }

    fn turn_off_group(&mut self, index: usize) {
        if let Some(g) = self.groups.get_mut(index).and_then(Option::as_mut)
            && g.state != GroupState::Ending
        {
            g.state = GroupState::Ending;
            g.timer = 0;
        }
    }

    pub fn ambient_due(&mut self, ticks: i32) -> bool {
        let clocks = [
            self.media_effect.as_mut().map(|m| &mut m.0),
            self.splashes.as_mut(),
        ];
        let mut due = false;
        for a in clocks.into_iter().flatten() {
            a.timer -= ticks;
            due |= a.timer < 0;
        }
        due
    }

    pub fn ambient_count(&self) -> i32 {
        let media = self.media_effect.map(|m| m.0.group);
        let splash = self.splashes.map(|a| a.group);
        media
            .into_iter()
            .chain(splash)
            .filter_map(|g| self.groups[g].as_ref())
            .map(|g| i32::from(g.count))
            .sum()
    }

    fn create_stationary(&mut self, group: usize, point: [i32; 3]) {
        let Some(g) = self.groups[group] else {
            return;
        };
        let relative = [0, 1, 2].map(|i| point[i].wrapping_sub(g.center[i]));
        let facing = match self.rules {
            Rules::Myth2 => self.random.next_u16(),
            Rules::Tfl => 0,
        };
        self.new_projectile(group, Motion::at_rest(relative, facing));
        if let Some(g) = self.groups[group].as_mut() {
            g.wanted = -1;
        }
    }

    pub fn build_ambient(&mut self, world: &AmbientWorld<'_>) {
        if let Some((mut a, chance)) = self.media_effect.filter(|m| m.0.timer < 0) {
            self.each_media_cell(world.region, world.cell, |local, col, row, height| {
                for _ in 0..a.tries {
                    if local.random.next_u16() < chance {
                        let point = point_in_cell(&mut local.random, col, row, height);
                        local.create_stationary(a.group, point);
                    }
                }
            });
            a.timer = i32::from(a.period);
            self.media_effect = Some((a, chance));
        }
        if let Some(mut a) = self.splashes.filter(|a| a.timer < 0) {
            self.each_media_cell(world.region, world.cell, |local, col, row, height| {
                let intensity = (world.splash_intensity)((col << 9) + 0x100, (row << 9) + 0x100);
                for _ in 0..a.tries {
                    if local.random.next_u16() < intensity {
                        let point = point_in_cell(&mut local.random, col, row, height);
                        local.create_stationary(a.group, point);
                    }
                }
            });
            a.timer = i32::from(a.period);
            self.splashes = Some(a);
        }
    }

    pub fn build_tfl_ambient(
        &mut self,
        ticks: i32,
        region: &ViewRegion,
        cell: &dyn Fn(i32, i32) -> u8,
        density: &dyn Fn(i32, i32) -> u16,
    ) -> usize {
        const TRIES: usize = 4;
        const PERIOD: i32 = 3;
        let mut made = 0;
        let mut each = |local: &mut Self, group: usize, chance: &dyn Fn(i32, i32) -> u16| {
            for row in region.first_row..region.last_row {
                let Some((first, last)) = region.span(row) else {
                    continue;
                };
                for col in i32::from(first)..i32::from(last) {
                    if cell(col, row) & 0x10 == 0 {
                        continue;
                    }
                    let chance = chance(col, row);
                    for _ in 0..TRIES {
                        if local.random.next_u16() < chance {
                            let x = (i32::from(local.random.next_u16()) >> 6) + (col << 10);
                            let y = (i32::from(local.random.next_u16()) >> 6) + (row << 10);
                            local.create_stationary(group, [x, y, 0]);
                            made += 1;
                        }
                    }
                }
            }
        };
        if let Some((mut a, chance)) = self.media_effect {
            a.timer -= ticks;
            if a.timer < 0 {
                each(self, a.group, &|_, _| chance);
                a.timer = PERIOD;
            }
            self.media_effect = Some((a, chance));
        }
        if let Some(mut a) = self.splashes {
            a.timer -= ticks;
            if a.timer < 0 {
                each(self, a.group, &|col, row| {
                    density((col << 10) + 0x200, (row << 10) + 0x200)
                });
                a.timer = PERIOD;
            }
            self.splashes = Some(a);
        }
        made
    }

    pub fn run_order(&mut self, groups: &[i16], on: bool, opacity: Option<u16>, off: bool) {
        self.run_group_action(&LocalGroupAction {
            groups: groups.to_vec(),
            on,
            off,
            opacity: opacity.map(i32::from),
        });
    }

    fn each_media_cell(
        &mut self,
        region: &ViewRegion,
        cell: &dyn Fn(i32, i32) -> (u16, i16),
        mut visit: impl FnMut(&mut Self, i32, i32, i16),
    ) {
        for row in region.first_row..region.last_row {
            let Some((first, last)) = region.span(row) else {
                continue;
            };
            for col in i32::from(first)..i32::from(last) {
                let (flags, height) = cell(col, row);
                if flags & CELL_MEDIA == CELL_MEDIA {
                    visit(self, col, row, height);
                }
            }
        }
    }

    pub fn update_from_sim(
        &mut self,
        ticks: i32,
        sim: &Simulation,
        cells: (i32, i32),
        visible: Option<&dyn Fn(i32, i32) -> u32>,
    ) {
        self.tick = sim.current_tick;
        if let Some(actions) = sim.engine.map_actions.as_ref() {
            let total = actions.local_group_actions_total;
            let kept = actions.local_group_actions.len() as u64;
            let new = total.saturating_sub(self.group_actions_run).min(kept) as usize;
            for action in &actions.local_group_actions[kept as usize - new..] {
                self.run_group_action(action);
            }
            self.group_actions_run = total;
        }
        self.follow_mirrors(&sim.engine.fire.groups);
        let height = |x: i32, y: i32| sim.mesh_height_at(x, y);
        let object = |o: ObjectRef| sim.object_location(o.index, o.identifier);
        let cell_bits = |o: ObjectRef, at: [i32; 3]| {
            let flags = sim.object_flags(o.index, o.identifier)?;
            (flags & 0x400_0000 != 0).then(|| visible.map(|v| v(at[0], at[1])))?
        };
        self.update(
            ticks,
            &LocalWorld {
                size: [cells.0 * 512, cells.1 * 512],
                height: &height,
                wind: (sim.engine.wind.as_ref()).map(|w| w.velocity),
                object: &object,
                cell_bits: &cell_bits,
                tfl_cell: None,
                tfl_drawn: None,
            },
        );
    }
}

fn projectile_local_group(p: &ProjectileDefinition) -> Option<FourCC> {
    let b: [u8; 4] = p.raw.get(0x88..0x8c)?.try_into().ok()?;
    (!matches!(u32::from_be_bytes(b), 0 | 0xffff_ffff)).then_some(FourCC::new(b))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FrameTick {
    first: bool,
    key: bool,
    wrapped: bool,
}

fn animate(p: &mut Projectile, s: &LocalSequence) -> FrameTick {
    let mut out = FrameTick::default();
    let period = p.ticks_per_frame;
    if period == 0 {
        return out;
    }
    let advance = if p.frame == 0 && p.tick == 0 {
        out.first = true;
        p.tick = 1;
        period == 1
    } else if p.tick < period {
        p.tick += 1;
        p.tick >= period
    } else {
        p.tick = 1;
        period == 1
    };
    if advance {
        p.frame += 1;
        if p.frame >= s.frames {
            p.frame = s.loop_frame;
            out.wrapped = true;
        }
        out.key = p.frame == s.key_frame;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LocalBitmap {
    pub(crate) atlas_index: usize,
    pub(crate) world_size: [f32; 2],
    pub(crate) rect: InstanceRect,
    pub(crate) flags: u32,
}

type LocalBitmaps = (Vec<Option<LocalBitmap>>, usize, CoreRef);

#[derive(Clone, Debug, Default)]
pub struct LocalSprites {
    sequences: BTreeMap<(FourCC, u8), LocalBitmaps>,
    overlays: BTreeMap<(FourCC, u8), Vec<Option<ShadowSprite>>>,
}

impl LocalSprites {
    pub fn add(
        &mut self,
        key: (FourCC, u8),
        visual: &SequenceVisual,
        core: CoreRef,
        flags: impl Fn(usize, usize) -> u32,
        anchor: impl Fn(usize) -> [f32; 2],
    ) {
        self.add_scaled(key, visual, core, flags, anchor, 1.0);
    }

    pub fn add_scaled(
        &mut self,
        key: (FourCC, u8),
        visual: &SequenceVisual,
        core: CoreRef,
        flags: impl Fn(usize, usize) -> u32,
        anchor: impl Fn(usize) -> [f32; 2],
        pixel_scale: f32,
    ) {
        let frames = visual.frames.len();
        let views = usize::from(visual.sequence.num_views).max(1);
        let mut bitmaps = Vec::with_capacity(views * frames);
        for view in 0..views {
            #[allow(clippy::needless_range_loop)]
            for frame in 0..frames {
                let sprite = visual.frames[frame]
                    .views
                    .get(view)
                    .and_then(Option::as_ref);
                bitmaps.push(sprite.map(|s| {
                    let world_size = s.world_size.map(|v| v * pixel_scale);
                    LocalBitmap {
                        atlas_index: s.atlas_index,
                        world_size,
                        rect: InstanceRect::from_sprite(world_size, anchor(s.atlas_index)),
                        flags: flags(view, frame),
                    }
                }));
            }
        }
        self.sequences.insert(key, (bitmaps, frames, core));
        if visual.frames.iter().any(|f| f.shadow.is_some()) {
            self.overlays.insert(
                key,
                visual.frames.iter().map(|f| f.shadow.clone()).collect(),
            );
        }
    }

    pub(crate) fn bitmap(&self, key: &(FourCC, u8), at: usize) -> Option<(LocalBitmap, CoreRef)> {
        let (bitmaps, _, core) = self.sequences.get(key)?;
        Some(((*bitmaps.get(at)?)?, *core))
    }

    pub(crate) fn overlay(&self, key: &(FourCC, u8), frame: usize) -> Option<&ShadowSprite> {
        self.overlays.get(key)?.get(frame)?.as_ref()
    }

    pub fn has_overlays(&self) -> bool {
        !self.overlays.is_empty()
    }

    pub fn is_lighten_overlay(&self, key: &(FourCC, u8), frame: usize) -> bool {
        self.overlays
            .get(key)
            .and_then(|frames| frames.get(frame))
            .and_then(|opt| opt.as_ref())
            .is_some_and(|shadow| shadow.flags & 1 != 0)
    }

    pub fn overlays(
        &self,
        surfaces: &[LocalSurface],
        eye: [i32; 3],
        depth_row: [i16; 3],
    ) -> Vec<ObjectShadow> {
        const IN_FRONT: i32 = -0x23;
        surfaces
            .iter()
            .filter(|s| s.reflection.is_none() && !s.media_surface)
            .filter(|s| {
                let depth = (0..3).fold(0i32, |sum, i| {
                    sum.wrapping_add(
                        s.world[i]
                            .wrapping_sub(eye[i])
                            .wrapping_mul(i32::from(depth_row[i])),
                    )
                }) >> 14;
                depth <= IN_FRONT
            })
            .filter_map(|s| {
                let frames = self.overlays.get(&(s.collection_reference, s.sequence))?;
                let shadow = frames.get(usize::try_from(s.frame).ok()?)?.as_ref()?;
                local_projectile_overlay(
                    shadow,
                    (s.world[0], s.world[1]),
                    s.above_ground,
                    !s.transparency,
                )
            })
            .collect()
    }

    pub fn instances(
        &self,
        surfaces: &[LocalSurface],
        atlas: &TextureAtlas,
        frame: WorldFrame,
        axes: ([f32; 3], [f32; 3], [f32; 3]),
        permuted: &dyn Fn(FourCC, usize, i16) -> usize,
        (global_tint, reflection_tint): (media::CoreTint, &dyn Fn(CoreRef) -> [u8; 8]),
    ) -> Vec<BillboardInstance> {
        let rows = EngineViewRows::from_camera_axes(axes.0, axes.1, axes.2);
        let mut out = Vec::with_capacity(surfaces.len());
        for s in surfaces {
            let Some((bitmaps, frames, core)) =
                self.sequences.get(&(s.collection_reference, s.sequence))
            else {
                continue;
            };
            let at = usize::try_from(s.view).unwrap_or(0) * frames
                + usize::try_from(s.frame).unwrap_or(0);
            let Some(Some(bitmap)) = bitmaps.get(at) else {
                continue;
            };
            let position = frame.from_engine(s.world.map(|c| c as f32));
            let k = [s.scale_x, s.scale_y].map(|v| f32::from(v) / 256.0);
            let velocity_aligned = bitmap.flags & INSTANCE_FLAG_VELOCITY_ALIGNED != 0;
            let Some(mut instance) = atlas.create_instance_with_world_size(
                permuted(s.collection_reference, bitmap.atlas_index, s.permutation),
                position,
                [bitmap.world_size[0] * k[0], bitmap.world_size[1] * k[1]],
                bitmap.flags & 1 != 0,
            ) else {
                continue;
            };
            if velocity_aligned {
                let reflection = s.reflection.is_some();
                let base = match s.reflection {
                    Some(surface) => corpse::reflected_origin(instance, surface, axes.1),
                    None => instance,
                };
                instance = VelocityQuad::new(
                    &rows,
                    s.velocity,
                    bitmap.rect,
                    i32::from(s.scale_x),
                    reflection,
                )
                .instance(base, reflection);
            } else if bitmap.flags & 2 != 0 {
                instance.flags |= BILLBOARD_FLAG_MIRROR_Y;
            }
            if s.transparency != 0 {
                instance.tint[3] *= mandrake_view::media::surface_opacity(s.transparency);
            }
            let tint = core.resolve(global_tint);
            if tint.fraction != 0 {
                let slot = ShadingSlot {
                    kind: -1,
                    ..ShadingSlot::default()
                };
                instance = instance.with_shade(build_tinting_shader(
                    slot,
                    tint.fraction,
                    tint.color,
                    slot,
                ));
            }
            if s.media_surface {
                instance.flags |= BILLBOARD_FLAG_MEDIA_EFFECT;
            }
            if let Some(surface) = s.reflection {
                let tint = reflection_tint(*core);
                instance = if velocity_aligned {
                    instance.with_shade(tint)
                } else {
                    instance.reflection(
                        f32::from(surface) / MarkerEntry::WORLD_ONE,
                        0.0,
                        false,
                        tint,
                    )
                };
            }
            out.push(instance);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_view::landscape::OverlayMode;

    #[test]
    fn a_frame_with_a_descriptor_adds_a_pool_under_the_surface() {
        let reference = FourCC::new(*b"kefc");
        let shadow = |flags| ShadowSprite {
            flags,
            offset_x: 16 << 10,
            offset_y: 8 << 10,
            width: 2,
            height: 2,
            pixels: vec![255u8; 4].into(),
        };
        let mut sprites = LocalSprites::default();
        sprites
            .overlays
            .insert((reference, 3), vec![Some(shadow(1)), None, Some(shadow(0))]);
        let pool = LocalSurface {
            collection_reference: reference,
            sequence: 3,
            frame: 0,
            view: 0,
            world: [1000, 2000, 0],
            above_ground: 0,
            facing: 0,
            transparency: 0x3fff,
            scale_x: 256,
            scale_y: 256,
            velocity: [0; 3],
            permutation: 0,
            media_surface: false,
            reflection: None,
        };
        let (eye, depth) = ([1000, 2000, 1000], [0, 0, 16384]);
        let overlay = |s: LocalSurface| {
            let mut o = sprites.overlays(&[s], eye, depth);
            let first = o.pop();
            assert!(o.is_empty());
            first.map(|o| {
                let o = o.overlay();
                (o.x, o.y, o.intensity, o.mode)
            })
        };
        assert_eq!(
            overlay(pool),
            Some((
                984,
                1992,
                ((0xc0 * 0xffff) >> 8) as u16,
                OverlayMode::Lighten
            ))
        );
        let above = |above_ground, frame| LocalSurface {
            above_ground,
            frame,
            ..pool
        };
        assert_eq!(overlay(above(1024, 0)), None);
        assert_eq!(
            overlay(above(512, 0)),
            Some((984, 1992, 0x6000, OverlayMode::Lighten))
        );
        assert_eq!(
            overlay(above(1024, 2)),
            Some((984 + 511, 1992 + 511, 0x8000, OverlayMode::Darken))
        );
        assert_eq!(overlay(above(0, 1)), None);
        assert_eq!(overlay(above(0, 3)), None);
        assert_eq!(
            overlay(LocalSurface {
                sequence: 4,
                ..pool
            }),
            None
        );
        let reflection = LocalSurface {
            reflection: Some(0),
            ..pool
        };
        assert_eq!(overlay(reflection), None);
        let media_surface = LocalSurface {
            media_surface: true,
            ..pool
        };
        assert_eq!(overlay(media_surface), None);
        let behind = LocalSurface {
            world: [1000, 2000, 2000],
            ..pool
        };
        assert_eq!(overlay(behind), None);
        let faint = LocalSurface {
            transparency: 0xff00,
            ..pool
        };
        assert_eq!(overlay(faint), None);
    }

    fn dwbs() -> LocalDefinition {
        let mut data = vec![0u8; 0x80];
        data[..0x20].copy_from_slice(&[
            0x01, 0x81, 0x8d, 0x52, b'd', b'w', b'n', b't', b'b', b'o', b's', b'r', 0x00, 0x1a,
            0xff, 0xff, 0x00, 0xd9, 0x00, 0x41, 0x00, 0xcc, 0x00, 0x81, 0x00, 0x0e, 0x00, 0x00, 0,
            0, 0, 0,
        ]);
        data[0x2c..0x32].copy_from_slice(&[0, 4, 0, 0, 0, 0x1e]);
        let mut phys = vec![0u8; 0x40];
        phys[0x1c..0x20].copy_from_slice(&[0, 0x0a, 0, 0]);
        phys[0x28..0x2c].copy_from_slice(&[0, 2, 0, 1]);
        LocalDefinition::parse(FourCC::new(*b"dwbs"), &data, |id| {
            (id == FourCC::new(*b"bosr")).then(|| phys.clone())
        })
        .unwrap()
    }

    fn sequence() -> LocalSequence {
        LocalSequence {
            frames: 6,
            ticks_per_frame: 4,
            key_frame: 0,
            loop_frame: 0,
            stored_view: false,
            views: 1,
            permutations: 1,
            sounds: [false; 3],
        }
    }

    #[test]
    fn a_mirror_group_draws_its_groups_projectiles_at_its_point() {
        let mut data = vec![0u8; 0x80];
        data[..0x38].copy_from_slice(&[
            0x04, 0x40, 0x0c, 0x50, b'f', b'i', b'r', b'e', 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 1, 1,
            0, 0, 0, 1, 0, 0, 0x31, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0x34, 0, 0, 0, 0x3c, 0, 0, 0,
            0x5b, 0, 0, 0, 0, 0, 0x3c, 0, 0x3d, 0, 0, 0, 0,
        ]);
        let flame = LocalDefinition::parse(FourCC::new(*b"fire"), &data, |_| None).unwrap();
        assert_eq!(flame.physics, None);
        assert_eq!(flame.flags & (FLAG_MEDIA_SURFACE | 0x100_0000), 0);
        let mut smoke = flame;
        smoke.tag = FourCC::new(*b"smff");
        let mut l = LocalProjectiles::new(1);
        l.add_definition(flame, sequence());
        l.add_definition(smoke, sequence());
        l.start_ambient(None, true, None, [0x8000, 0x8000]);
        let height = |_: i32, _: i32| 0;
        let none = |_: ObjectRef| None;
        for _ in 0..0x80 {
            l.update(1, &world(&none, &height));
        }
        assert!(l.active() > 0);
        assert_eq!(l.surfaces(0x10000, &view(&height)), []);

        let mirror = MirrorGroup {
            flame: true,
            slot: 2,
            point: [0x1100, 0x2300, 40],
            lifetime: 10,
            fade_in: 0x96,
            fade_out: 0x96,
            strength: 0x4000,
            state: 0,
            ticks: 0,
        };
        let mirrors = [None, Some(mirror)];
        let drawn = l.surfaces(
            0x10000,
            &SurfaceView {
                mirrors: &mirrors,
                ..view(&height)
            },
        );
        let group = l.flame_groups[2].unwrap();
        let of_group: Vec<_> = l
            .projectiles
            .iter()
            .flatten()
            .filter(|p| p.group == group)
            .collect();
        assert_eq!(drawn.len(), of_group.len());
        assert!(!drawn.is_empty());
        for (s, p) in drawn.iter().zip(of_group) {
            assert_eq!(s.world[0], 0x1100 + p.relative[0]);
            assert_eq!(s.world[1], 0x2300 + p.relative[1]);
            assert_eq!(s.world[2], 40 + p.relative[2]);
            assert_eq!(s.transparency, !0x3fffu16);
            assert_eq!(s.scale_x, p.scale);
        }

        l.collections
            .insert(FourCC::new(*b"fire"), FourCC::new(*b"fire"));
        l.follow_mirrors(&mirrors);
        for _ in 0..8 {
            l.new_projectile(group, Motion::at_rest([3, 4, 5], 0));
        }
        let stains = l.take_stains();
        assert_eq!(stains.len(), 7);
        assert_eq!(stains[0].point, [0x1103, 0x2304, 0]);
        assert_eq!(stains[0].sequence, 1);
        l.follow_mirrors(&mirrors);
        l.new_projectile(group, Motion::at_rest([3, 4, 5], 0));
        assert_eq!(l.take_stains(), []);
        let smoke_mirror = MirrorGroup {
            flame: false,
            slot: 0,
            ..mirror
        };
        l.follow_mirrors(&[Some(smoke_mirror)]);
        l.new_projectile(l.smoke_groups[0].unwrap(), Motion::at_rest([3, 4, 5], 0));
        assert_eq!(l.take_stains(), []);
    }

    #[test]
    fn the_tags_fields() {
        let d = dwbs();
        assert_eq!(d.flags, 0x0181_8d52);
        assert_eq!(d.collection_reference, FourCC::new(*b"dwnt"));
        assert_eq!(d.sequence, 26);
        assert_eq!(d.count, (14, 0));
        assert_eq!(d.scale, (0xd9, 0x41));
        assert_eq!(d.transfer, 4);
        assert_eq!(d.start_period, (30, 0));
        let Some(LocalPhysics::Drift(p)) = d.physics else {
            panic!("type 0");
        };
        assert_eq!(p.velocity[2], 0x000a_0000);
    }

    fn world<'a>(
        object: &'a dyn Fn(ObjectRef) -> Option<[i32; 3]>,
        height: &'a dyn Fn(i32, i32) -> i32,
    ) -> LocalWorld<'a> {
        LocalWorld {
            size: [1 << 20, 1 << 20],
            height,
            wind: None,
            object,
            cell_bits: &|_, _| None,
            tfl_cell: None,
            tfl_drawn: None,
        }
    }

    fn view<'a>(height: &'a dyn Fn(i32, i32) -> i32) -> SurfaceView<'a> {
        SurfaceView {
            eye: [0, 0, 5000],
            height,
            media_height: height,
            region: None,
            media_transparency: 0,
            reflective: &|_, _| false,
            mirrors: &[],
        }
    }

    fn marker(flags: u32, palette_index: i16, marker_id: i16, x: i32) -> MarkerEntry {
        let mut type_data = [0; 32];
        type_data[0] = 0x40;
        MarkerEntry {
            flags,
            marker_type: MarkerPaletteEntry::TYPE_WAYPOINT,
            palette_index,
            marker_id,
            difficulty_flags: 0,
            x,
            y: 0x2000,
            z: 0x100,
            yaw: 0x4000,
            pitch: 0,
            roll: 0,
            type_data,
        }
    }

    fn palette(tag_id: FourCC) -> MarkerPaletteEntry {
        MarkerPaletteEntry {
            marker_type: MarkerPaletteEntry::TYPE_WAYPOINT,
            flags: 0,
            tag_id,
            user_data: [0; 24],
        }
    }

    #[test]
    fn markers_start_at_load_or_by_their_action_and_start_again_after_their_group_is_deleted() {
        let d = dwbs();
        let mut l = LocalProjectiles::new(1);
        l.add_definition(d, sequence());
        l.start_markers(
            &[palette(d.tag)],
            &[marker(0, 0, 10000, 0x1000), marker(1, 0, 10001, 0x3000)],
        );
        let groups = |l: &LocalProjectiles| {
            let mut v: Vec<_> = l
                .groups
                .iter()
                .flatten()
                .map(|g| (g.identifier.unwrap(), g.center[0], g.facing))
                .collect();
            v.sort_unstable();
            v
        };
        assert_eq!(groups(&l), [(10000, 0x1000, 0x4000)]);
        let action = |groups: &[i16], on, off, opacity| LocalGroupAction {
            groups: groups.to_vec(),
            on,
            off,
            opacity,
        };
        l.run_group_action(&action(&[10000, 10001, 9], true, false, None));
        assert_eq!(
            groups(&l),
            [(10000, 0x1000, 0x4000), (10001, 0x3000, 0x4000)]
        );
        let of = |l: &LocalProjectiles, id| {
            *l.groups
                .iter()
                .flatten()
                .find(|g| g.identifier == Some(id))
                .unwrap()
        };
        l.run_group_action(&action(&[10001], false, false, Some(0x12345)));
        assert_eq!(of(&l, 10001).opacity, 0xffff);
        l.run_group_action(&action(&[10001], false, false, Some(0x1234)));
        assert_eq!(of(&l, 10001).opacity, 0x1234);
        assert_eq!(of(&l, 10000).opacity, 0xffff);

        l.run_group_action(&action(&[10001], false, true, None));
        assert_eq!(of(&l, 10001).state, GroupState::Ending);
        assert_eq!(of(&l, 10000).state, GroupState::Starting);
        let at = l
            .markers
            .iter()
            .position(|m| m.identifier == 10001)
            .unwrap();
        let group = l.markers[at].group.unwrap();
        l.delete_group(group);
        assert_eq!(l.markers[at].group, None);
        l.run_group_action(&action(&[10001], true, false, None));
        assert_eq!(of(&l, 10001).state, GroupState::Starting);
    }

    #[test]
    fn a_reflected_group_draws_again_only_on_a_reflective_cell_in_the_region() {
        let mut d = dwbs();
        d.physics = None;
        d.flags = 0x0100_0000 | FLAG_FIRST_FRAME;
        d.transfer = 0;
        d.outward = (0, 0);
        d.height = 0;
        d.count = (1, 0);
        let mut l = LocalProjectiles::new(1);
        l.add_definition(d, sequence());
        l.new_group(&NewGroup {
            definition: d.tag,
            point: [3 << 9 | 10, 1 << 9 | 20, 600],
            central: None,
            target: None,
            opacity: 0xffff,
            facing: 0,
        })
        .unwrap();
        let height = |_: i32, _: i32| 0;
        let none = |_: ObjectRef| None::<[i32; 3]>;
        l.update(1, &world(&none, &height));
        let region = water_region();
        let media = |_: i32, _: i32| 512;
        let reflective = |x: i32, y: i32| (x >> 9, y >> 9) == (3, 1);
        let mut v = SurfaceView {
            eye: [0, 0, 5000],
            height: &height,
            media_height: &media,
            region: Some(&region),
            media_transparency: 0x8000,
            reflective: &reflective,
            mirrors: &[],
        };
        let drawn = l.surfaces(0x10000, &v);
        assert_eq!(drawn.len(), 2);
        assert_eq!(drawn[0].reflection, None);
        assert_eq!(drawn[0].transparency, !0xfffdu16);
        assert_eq!(drawn[1].reflection, Some(512));
        assert_eq!(drawn[1].world, drawn[0].world);
        assert_eq!(!drawn[1].transparency, ((0xfffdu32 * 0x7fff) >> 16) as u16);

        let dry = |_: i32, _: i32| false;
        v.reflective = &dry;
        assert_eq!(l.surfaces(0x10000, &v).len(), 1);
        v.reflective = &reflective;
        v.region = None;
        assert_eq!(l.surfaces(0x10000, &v).len(), 1);
    }

    #[test]
    fn a_sequence_asks_for_its_sounds_at_the_first_frame_the_key_frame_and_the_wrap() {
        let mut d = dwbs();
        d.physics = None;
        d.flags = FLAG_FIRST_FRAME;
        d.outward = (0, 0);
        d.count = (1, 0);
        d.speed = (0x100, 0);
        let mut s = sequence();
        s.frames = 3;
        s.ticks_per_frame = 2;
        s.key_frame = 1;
        s.sounds = [true, true, true];
        let mut l = LocalProjectiles::new(1);
        l.add_definition(d, s);
        l.collections
            .insert(d.collection_reference, FourCC::new(*b"coll"));
        l.new_group(&NewGroup {
            definition: d.tag,
            point: [0x1000, 0x1000, 0],
            central: None,
            target: None,
            opacity: 0xffff,
            facing: 0,
        })
        .unwrap();
        let height = |_: i32, _: i32| -1000;
        let none = |_: ObjectRef| None::<[i32; 3]>;
        let mut heard = Vec::new();
        for tick in 0..8 {
            l.update(1, &world(&none, &height));
            for s in l.take_sounds() {
                assert_eq!(s.collection, FourCC::new(*b"coll"));
                assert_eq!(s.object, -1);
                heard.push((
                    tick,
                    s.sounds.first,
                    s.sounds.key,
                    s.sounds.last,
                    s.world[0],
                ));
            }
        }
        assert_eq!(
            heard,
            [
                (0, true, false, false, 0),
                (1, false, true, false, 0x1000),
                (5, false, false, true, 0x1000),
                (7, false, true, false, 0x1000),
            ]
        );
    }

    #[test]
    fn physics_type_3_is_drawn_to_the_center_within_its_speed_limit() {
        let ph = PullPhysics {
            on_ground: false,
            speed_limit: 0x14_0000,
            pull: 0x147,
            height: -3,
        };
        let mut p = Projectile {
            group: 0,
            velocity: [0; 3],
            period: 0,
            clock: 0,
            rising: 0,
            relative: [1000, 0, 7],
            world: None,
            facing: 0,
            scale: 0x100,
            frame: 0,
            tick: 0,
            ticks_per_frame: 1,
            permutation: 0,
            age: 0,
            lifetime: 0,
            serial: 0,
        };
        let ground = |_: i32, _: i32| 40;
        move_pull(&mut p, &ph, 1, [100, 100, 10], &ground);
        assert_eq!(p.velocity[0], -1000 * 0x147);
        assert_eq!(p.relative, [1000 + ((-1000 * 0x147) >> 16), 0, 7]);
        for _ in 0..200 {
            move_pull(&mut p, &ph, 1, [100, 100, 10], &ground);
            assert!(p.velocity[0].abs() <= ph.speed_limit);
        }
        let on_ground = PullPhysics {
            on_ground: true,
            ..ph
        };
        move_pull(&mut p, &on_ground, 1, [100, 100, 10], &ground);
        assert_eq!(p.relative[2], -3 - 10 + 40);
    }

    fn coef() -> LocalDefinition {
        let hex = |s: &str| -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        };
        let mut data = hex(concat!(
            "01203300696e6d65636f6566000effff004c000000cc00650005000000000100",
            "000000000000001e000000000000020000000000000f00000000000000000000",
        ));
        data.resize(0x80, 0);
        let mut phys = hex("000100000000000000000258000000660000051e");
        phys.resize(0x40, 0);
        LocalDefinition::parse(FourCC::new(*b"coef"), &data, |id| {
            (id == FourCC::new(*b"coef")).then(|| phys.clone())
        })
        .unwrap()
    }

    #[test]
    fn a_confused_monsters_group_follows_it_and_is_drawn_only_where_it_is_seen() {
        let mut l = LocalProjectiles::new(1);
        l.add_definition(coef(), sequence());
        let object = ObjectRef {
            index: 51,
            identifier: -77,
        };
        l.start_for_event(&CombatEvent::MonsterLocalGroup {
            tick: 1361,
            monster: 51,
            local_group: FourCC::new(*b"coef"),
            object: object.index,
            identifier: object.identifier,
        });
        let g = l.groups.iter().position(Option::is_some).unwrap();
        assert_eq!(l.monster_groups.get(&51), Some(&g));
        assert_eq!(l.group_states(), [(5, 5, 0, 0)]);
        assert_eq!(l.groups[g].unwrap().center, [0, 0, 0x200]);
        let height = |_: i32, _: i32| -1000;
        let at = [38755, 32635, -328];
        let here = move |o: ObjectRef| (o == object).then_some(at);
        let with_bits = |bits: Option<u32>| {
            move |o: ObjectRef, p: [i32; 3]| bits.filter(|_| o == object && p == at)
        };
        let unseen = with_bits(Some(0));
        for _ in 0..30 {
            let mut world = world(&here, &height);
            world.cell_bits = &unseen;
            l.update(1, &world);
        }
        let group = l.groups[g].unwrap();
        assert_eq!(l.group_states(), [(5, 5, 0, 0)]);
        assert_eq!(group.center, [at[0], at[1], at[2] + 0x200]);
        assert_eq!((group.visible, group.opacity), (false, 0));
        assert!(l.surfaces(0x10000, &view(&height)).is_empty());
        for p in l.projectiles.iter().flatten() {
            let w = p.world.unwrap();
            assert!(
                (w[0] - at[0]).abs() < 0x400 && (w[1] - at[1]).abs() < 0x400,
                "{w:?}"
            );
        }
        let half = with_bits(Some(1));
        let mut world = world(&here, &height);
        world.cell_bits = &half;
        l.update(1, &world);
        let group = l.groups[g].unwrap();
        assert_eq!(
            (group.state, group.visible, group.opacity),
            (GroupState::On, true, 0x8000)
        );
        let seen = with_bits(Some(3));
        world.cell_bits = &seen;
        l.update(1, &world);
        assert_eq!(l.groups[g].unwrap().opacity, 0xffff);
        assert_eq!(l.surfaces(0x10000, &view(&height)).len(), l.active());
        l.start_for_event(&CombatEvent::MonsterLocalGroupOff {
            tick: 1961,
            monster: 51,
        });
        assert!(l.monster_groups.is_empty());
        assert_eq!(l.groups[g].unwrap().state, GroupState::Ending);
    }

    #[test]
    fn a_bottle_grows_its_smoke_and_it_runs_out_when_the_bottle_is_gone() {
        let mut l = LocalProjectiles::new(1);
        l.add_definition(dwbs(), sequence());
        let central = ObjectRef {
            index: 7,
            identifier: -3,
        };
        l.new_group(&NewGroup {
            definition: FourCC::new(*b"dwbs"),
            point: [1000, 1000, 100],
            central: Some(central),
            target: None,
            opacity: 0xffff,
            facing: 0,
        })
        .unwrap();
        assert_eq!(l.active(), 0);
        let height = |_: i32, _: i32| -1000;
        let here = |_: ObjectRef| Some([1000, 1000, 100]);
        for _ in 0..15 {
            l.update(1, &world(&here, &height));
        }
        let halfway = l.active();
        assert!((6..=8).contains(&halfway), "{halfway}");
        for _ in 0..20 {
            l.update(1, &world(&here, &height));
        }
        assert!(l.active() >= 12, "{}", l.active());
        let drawn = l.surfaces(0x10000, &view(&height));
        assert_eq!(drawn.len(), l.active());
        assert!(drawn.iter().all(|s| s.world[2] > 100));
        let gone = |_: ObjectRef| None;
        for _ in 0..40 {
            l.update(1, &world(&gone, &height));
        }
        assert_eq!(l.active(), 0);
        assert!(l.groups.iter().all(Option::is_none));
    }

    #[test]
    fn the_transfer_of_the_sequence_fades_it_out() {
        let mut l = LocalProjectiles::new(1);
        l.add_definition(dwbs(), sequence());
        l.new_group(&NewGroup {
            definition: FourCC::new(*b"dwbs"),
            point: [1000, 1000, 100],
            central: None,
            target: None,
            opacity: 0xffff,
            facing: 0,
        })
        .unwrap();
        let height = |_: i32, _: i32| -1000;
        let none = |_: ObjectRef| None::<[i32; 3]>;
        for _ in 0..31 {
            l.update(1, &world(&none, &height));
        }
        for s in l.surfaces(0x10000, &view(&height)) {
            assert!(s.transparency < 0xffff || s.frame == 5);
        }
    }

    #[test]
    fn history_is_where_a_frame_right_after_a_tick_draws() {
        let mut l = LocalProjectiles::new(2);
        l.add_definition(dwbs(), sequence());
        let height = |_: i32, _: i32| -1000;
        let none = |_: ObjectRef| None::<[i32; 3]>;
        l.new_group(&NewGroup {
            definition: FourCC::new(*b"dwbs"),
            point: [1000, 1000, 100],
            central: None,
            target: None,
            opacity: 0xffff,
            facing: 0,
        });
        for _ in 0..31 {
            l.update(1, &world(&none, &height));
        }
        l.record_history();
        let before = l.surfaces(0, &view(&height));
        l.update(1, &world(&none, &height));
        let after = l.surfaces(0, &view(&height));
        assert_eq!(before[0].world, after[0].world);
    }

    fn fosp() -> LocalDefinition {
        let mut data = vec![0u8; 0x80];
        data[..0x20].copy_from_slice(&[
            0x02, 0x80, 0x94, 0x10, b'w', b'a', b't', b'e', 0xff, 0xff, 0xff, 0xff, 0x00, 0x03,
            0xff, 0xff, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x01, 0x00, 0x0a, 0x00, 0x06, 0,
            0, 0, 0x9a,
        ]);
        data[0x20..0x24].copy_from_slice(&[0, 0, 0x02, 0x67]);
        data[0x2c..0x32].copy_from_slice(&[0, 4, 0, 0, 0, 0x1e]);
        LocalDefinition::parse(FourCC::new(*b"fosp"), &data, |_| None).unwrap()
    }

    fn splashes(sequence_frames: i16) -> LocalProjectiles {
        let mut l = LocalProjectiles::new(3);
        l.add_definition(
            fosp(),
            LocalSequence {
                frames: sequence_frames,
                ..sequence()
            },
        );
        l.start_ambient(
            None,
            false,
            Some(AmbientSource {
                group: FourCC::new(*b"fosp"),
                tries: 1,
                period: 8,
            }),
            [1 << 16, 1 << 16],
        );
        l
    }

    fn water_region() -> ViewRegion {
        ViewRegion {
            first_row: 1,
            last_row: 3,
            spans: vec![(2, 5), (2, 5)],
            ..ViewRegion::default()
        }
    }

    fn water_cell(col: i32, _row: i32) -> (u16, i16) {
        (if col == 3 { 0x1800 } else { 0x1000 }, -64)
    }

    #[test]
    fn a_tag_without_physics_is_loaded() {
        let d = fosp();
        assert_eq!(d.physics, None);
        assert_eq!(d.flags & FLAG_MEDIA_SURFACE, FLAG_MEDIA_SURFACE);
        assert_eq!(d.sequence, 3);
        assert_eq!(d.count, (10, 6));
        assert_eq!(d.speed, (0x100, 0x101));
    }

    #[test]
    fn the_group_makes_its_first_ones_at_load_and_no_more() {
        let mut l = splashes(5);
        let first = l.active();
        assert!((10..16).contains(&first), "{first}");
        let height = |_: i32, _: i32| 0;
        let none = |_: ObjectRef| None::<[i32; 3]>;
        let w = world(&none, &height);
        for _ in 0..41 {
            l.update(1, &w);
        }
        assert_eq!(l.active(), 0);
        assert_eq!(l.group_states(), vec![(-1, 0, 1, 0)]);
    }

    #[test]
    fn splashes_fall_on_media_cells_of_the_region_every_period_plus_one_ticks() {
        let mut l = splashes(5);
        let at_load = l.active();
        let region = water_region();
        let intensity = |_: i32, _: i32| u16::MAX;
        let w = AmbientWorld {
            region: &region,
            cell: &water_cell,
            splash_intensity: &intensity,
        };
        assert!(l.ambient_due(0));
        l.build_ambient(&w);
        assert_eq!(l.active() - at_load, 2);
        for _ in 0..8 {
            assert!(!l.ambient_due(1));
        }
        assert!(l.ambient_due(1));
        l.build_ambient(&w);
        assert_eq!(l.active() - at_load, 4);
        for p in l.projectiles.iter().flatten().skip(at_load) {
            assert!(p.world.is_none(), "not drawn before the next update");
        }
        let g = l.groups[0].as_ref().unwrap();
        for p in l
            .projectiles
            .iter()
            .flatten()
            .filter(|p| p.velocity == [0; 3])
        {
            let [x, y, z] = [0, 1, 2].map(|i| p.relative[i] + g.center[i]);
            if z != -64 {
                continue;
            }
            assert_eq!(x >> 9, 3);
            assert!((1..3).contains(&(y >> 9)), "{y}");
        }
    }

    #[test]
    fn no_intensity_no_splash() {
        let mut l = splashes(5);
        let at_load = l.active();
        let region = water_region();
        let intensity = |_: i32, _: i32| 0u16;
        assert!(l.ambient_due(0));
        l.build_ambient(&AmbientWorld {
            region: &region,
            cell: &water_cell,
            splash_intensity: &intensity,
        });
        assert_eq!(l.active(), at_load);
    }

    #[test]
    fn a_media_surface_sprite_is_on_the_media_in_the_region_and_fades_with_the_media() {
        let mut l = splashes(5);
        l.projectiles.fill(None);
        let a = l.splashes.unwrap();
        l.groups[a.group].as_mut().unwrap().count = 0;
        l.create_stationary(a.group, [3 << 9 | 10, 1 << 9 | 20, -64]);
        let height = |_: i32, _: i32| 1000;
        let none = |_: ObjectRef| None::<[i32; 3]>;
        l.update(1, &world(&none, &height));
        let region = water_region();
        let media_height = |_: i32, _: i32| -64;
        let mut v = SurfaceView {
            eye: [0, 0, 5000],
            height: &height,
            media_height: &media_height,
            region: Some(&region),
            media_transparency: 0x8000,
            reflective: &|_, _| false,
            mirrors: &[],
        };
        let drawn = l.surfaces(0x10000, &v);
        assert_eq!(drawn.len(), 1);
        assert!(drawn[0].media_surface);
        assert_eq!(drawn[0].world, [3 << 9 | 10, 1 << 9 | 20, -64]);
        let alpha = !drawn[0].transparency;
        assert!((0x7000..=0x8000).contains(&alpha), "{alpha:#x}");
        let far = ViewRegion {
            first_row: 50,
            last_row: 51,
            spans: vec![(0, 1)],
            ..ViewRegion::default()
        };
        v.region = Some(&far);
        assert!(l.surfaces(0x10000, &v).is_empty());
    }

    #[test]
    fn tfl_splashes_draw_as_tfl_1_5_does() {
        let mut l = LocalProjectiles::with_rules(3, Rules::Tfl);
        let mut d = fosp();
        d.flags |= FLAG_UPDATE_UNSEEN;
        l.add_definition(
            d,
            LocalSequence {
                frames: 4,
                ..sequence()
            },
        );
        let source = AmbientSource {
            group: FourCC::new(*b"fosp"),
            tries: 4,
            period: 3,
        };
        l.start_ambient(None, false, Some(source), [1 << 16, 1 << 16]);
        assert_eq!(l.counts(), (0, 1));
        let mut expected = l.random;
        let cell = |col: i32, _: i32| if col == 3 { 0x10 } else { 0x08 };
        let all = |_: i32, _: i32| 0xffff;
        assert_eq!(l.build_tfl_ambient(1, &water_region(), &cell, &all), 8);
        for _ in 0..8 * 4 {
            expected.next_u16();
        }
        assert_eq!(
            {
                let mut r = l.random;
                r.next_u16()
            },
            expected.next_u16()
        );
        let points: Vec<[i32; 3]> = l
            .projectiles
            .iter()
            .flatten()
            .map(|p| [0, 1, 2].map(|i| p.relative[i] + l.groups[p.group].unwrap().center[i]))
            .collect();
        assert!(
            points
                .iter()
                .all(|p| p[0] >> 10 == 3 && (1..3).contains(&(p[1] >> 10)) && p[2] == 0),
            "{points:?}"
        );
        assert!(l.projectiles.iter().flatten().all(|p| p.facing == 0));
        assert_eq!(l.build_tfl_ambient(3, &water_region(), &cell, &all), 0);
        assert_eq!(l.build_tfl_ambient(1, &water_region(), &cell, &all), 8);
        let ground = |_: i32, _: i32| 0;
        let object = |_: ObjectRef| None;
        l.set_view_cells(Some([2, 5, 1, 3]));
        l.update(1, &world(&object, &ground));
        let v = view(&ground);
        assert!(l.surfaces(0x10000, &v).is_empty());
        l.update(1, &world(&object, &ground));
        let mut v = view(&ground);
        v.reflective = &|x, _| x >> 10 == 3;
        assert_eq!(l.surfaces(0x10000, &v).len(), 16);
        v.reflective = &|_, _| false;
        assert!(l.surfaces(0x10000, &v).is_empty());
        l.set_view_cells(Some([40, 50, 40, 50]));
        l.update(1, &world(&object, &ground));
        v.reflective = &|_, _| true;
        assert!(l.surfaces(0x10000, &v).is_empty());
        for g in l.groups.iter_mut().flatten() {
            g.definition.flags &= !FLAG_UPDATE_UNSEEN;
        }
        let before = l.projectiles.clone();
        l.update(1, &world(&object, &ground));
        assert_eq!(l.projectiles, before);
    }

    #[test]
    fn tfl_draws_follow_each_groups_update_once_a_frame() {
        let mut l = LocalProjectiles::with_rules(3, Rules::Tfl);
        let mut d = fosp();
        d.flags |= FLAG_UPDATE_UNSEEN;
        l.add_definition(
            d,
            LocalSequence {
                frames: 4,
                ..sequence()
            },
        );
        let source = AmbientSource {
            group: FourCC::new(*b"fosp"),
            tries: 4,
            period: 3,
        };
        l.start_ambient(None, false, Some(source), [1 << 16, 1 << 16]);
        let cell = |col: i32, _: i32| if col == 3 { 0x10 } else { 0x08 };
        let all = |_: i32, _: i32| 0xffff;
        assert_eq!(l.build_tfl_ambient(1, &water_region(), &cell, &all), 8);
        let ground = |_: i32, _: i32| 0;
        let object = |_: ObjectRef| None;
        l.set_view_cells(Some([2, 5, 1, 3]));
        l.update(1, &world(&object, &ground));
        let reflective = |x: i32, _: i32| (if x >> 10 == 3 { 0x38 } else { 0 }, false);
        let in_reach = |w: [i32; 3]| w[1] >> 10 == 1;
        let mut w = world(&object, &ground);
        w.tfl_cell = Some(&reflective);
        w.tfl_drawn = Some(&in_reach);
        let reached = l
            .projectiles
            .iter()
            .flatten()
            .filter(|p| p.world.is_some_and(|w| w[1] >> 10 == 1))
            .count();
        assert!(reached > 0 && reached < 8, "{reached}");
        for ticks in [1, 0, 0] {
            let mut expected = l.random;
            for _ in 0..reached {
                expected.next_u16();
            }
            let relatives = |l: &LocalProjectiles| -> Vec<Option<[i32; 3]>> {
                l.projectiles
                    .iter()
                    .map(|p| p.map(|p| p.relative))
                    .collect()
            };
            let before = relatives(&l);
            l.update(ticks, &w);
            let mut now = l.random;
            assert_eq!(now.next_u16(), expected.next_u16(), "{ticks} ticks");
            if ticks == 0 {
                assert_eq!(relatives(&l), before);
            }
        }
    }

    #[test]
    fn both_games_keep_type_0s_square_root_as_a_short() {
        assert_eq!(short_speed(0x7fff), 0x7fff);
        assert_eq!(short_speed(0x8000), -0x8000);
        assert_eq!(short_speed(46341), 46341 - 0x10000);
        let mut l = LocalProjectiles::new(5);
        let mut d = myth2_crco();
        d.physics = Some(LocalPhysics::Drift(DriftPhysics {
            flags: 2,
            acceleration: [0x60_0000, 0x60_0000, 0],
            velocity: [0; 3],
            velocity_range: [0; 3],
            jitter: [0; 3],
        }));
        d.count = (1, 0);
        l.add_definition(d, sequence());
        l.new_group(&NewGroup {
            definition: d.tag,
            point: [0x8000, 0x8000, 0x400],
            central: None,
            target: None,
            opacity: 0xffff,
            facing: 0,
        })
        .unwrap();
        let ground = |_: i32, _: i32| 0;
        let object = |_: ObjectRef| None;
        l.update(1, &world(&object, &ground));
        let p = l.projectiles.iter().flatten().next().unwrap();
        assert!(
            p.velocity[0] < -0x50_0000 && p.velocity[1] < -0x50_0000,
            "{:?}",
            p.velocity
        );
    }

    fn tfl_bytes(hex: &str, size: usize) -> Vec<u8> {
        let hex: String = hex.split_whitespace().collect();
        let mut out: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        out.resize(size, 0);
        out
    }

    fn tfl_phys(id: FourCC) -> Option<Vec<u8>> {
        let hex = match &id.0 {
            b"crco" => "0004 0000 00000000 0000 0000 0000 4000 00000000 ffe20000",
            b"tedi" => "0004 0000 00000001 0000 0000 2aaa 1555 00000000 ffe20000",
            b"crdi" => "0000 0000 00000003 00000000 00000000 00004ccc 003c0000 fff30000 fff70000",
            b"swar" => "0003 0000 00000001 003c0000 00000f5c 00000083 0200",
            b"teos" => "0001 0000 00000000 0000 01f4 00c9 00cd 00000000 00199999",
            _ => return None,
        };
        Some(tfl_bytes(hex, 0x40))
    }

    fn tfl_crco() -> LocalDefinition {
        let data = tfl_bytes(
            "00019ac9 72613031 6372636f 0000ffff 0065000b 01000000 00320000 00000c00 \
             00000801 0000005a 00000000 0000001e",
            0x80,
        );
        LocalDefinition::parse_with_rules(FourCC::new(*b"crco"), &data, tfl_phys, Rules::Tfl)
            .unwrap()
    }

    fn lpgr_with(phys: &[u8; 4]) -> Vec<u8> {
        let mut data = tfl_bytes("00000800 72613031", 0x80);
        data[8..12].copy_from_slice(phys);
        data[0x0e..0x10].copy_from_slice(&[0xff, 0xff]);
        data[0x18..0x1a].copy_from_slice(&[0, 1]);
        data
    }

    #[test]
    fn tfl_physics_flags_are_a_long_and_type_4_is_tfls() {
        let parse = |phys: &[u8; 4], rules| {
            LocalDefinition::parse_with_rules(FourCC::new(*phys), &lpgr_with(phys), tfl_phys, rules)
        };
        let drift = |d: Option<LocalDefinition>| match d.and_then(|d| d.physics) {
            Some(LocalPhysics::Drift(p)) => p.flags,
            other => panic!("{other:?}"),
        };
        assert_eq!(drift(parse(b"crdi", Rules::Tfl)), 3);
        assert_eq!(drift(parse(b"crdi", Rules::Myth2)), 3);
        assert!(matches!(
            parse(b"swar", Rules::Tfl).and_then(|d| d.physics),
            Some(LocalPhysics::Pull(PullPhysics {
                on_ground: true,
                speed_limit: 0x3c_0000,
                pull: 0x83,
                height: 0x200,
            }))
        ));
        assert_eq!(
            parse(b"crco", Rules::Myth2).and_then(|d| d.physics),
            tfl_crco().physics
        );
        assert_eq!(
            tfl_crco().physics,
            Some(LocalPhysics::Converge(ConvergePhysics {
                diverge: false,
                spin: 0,
                spin_range: 0,
                elevation: 0,
                elevation_range: 0x4000,
            }))
        );
        assert!(matches!(
            parse(b"tedi", Rules::Tfl).and_then(|d| d.physics),
            Some(LocalPhysics::Converge(ConvergePhysics {
                diverge: true,
                elevation: 0x2aaa,
                elevation_range: 0x1555,
                ..
            }))
        ));
        let Some(Some(LocalPhysics::Orbit(o))) = parse(b"teos", Rules::Tfl).map(|d| d.physics)
        else {
            panic!()
        };
        assert_eq!(
            (o.spin, o.spin_range, o.bob_height, o.bob_rate),
            (0x1f4, 0xc9, 0xcd, 0)
        );
        assert_eq!((o.radial_speed, o.rise), (0x19_9999, 0));
    }

    #[test]
    fn tfl_physics_type_4_converges_on_the_centre_over_its_time() {
        let mut l = LocalProjectiles::with_rules(7, Rules::Tfl);
        let d = tfl_crco();
        l.add_definition(d, sequence());
        let center = [0x8000, 0x8000, 0x400];
        l.new_group(&NewGroup {
            definition: d.tag,
            point: center,
            central: None,
            target: None,
            opacity: 0xffff,
            facing: 0,
        })
        .unwrap();
        let mut r = l.random;
        let ground = |_: i32, _: i32| 0;
        let object = |_: ObjectRef| None;
        l.update(1, &world(&object, &ground));
        assert_eq!(l.counts().0, 50);
        r.next_u16();
        let radius = 0xc00 + ((i32::from(r.next_u16()) * 0x801) >> 16);
        let angle = r.next_u16();
        let elevation = ((u32::from(r.next_u16()) * 0x4000) >> 16) as u16;
        let trig = trig_tables();
        let now = 89 * radius / 90;
        let (ia, ie) = (trig_index(angle), trig_index(elevation));
        let flat = (i32::from(trig.cosine[ie]) * now) >> 14;
        let expected = [
            (i32::from(trig.cosine[ia]) * flat) >> 14,
            (i32::from(trig.sine[ia]) * flat) >> 14,
            (i32::from(trig.sine[ie]) * now) >> 14,
        ];
        let first = l
            .projectiles
            .iter()
            .flatten()
            .min_by_key(|p| p.serial)
            .unwrap();
        assert_eq!(first.relative, expected);
        assert_eq!(first.facing, arctangent(expected[0], expected[1]));
        for _ in 0..3 {
            r.next_u16();
        }
        for _ in 0..49 * 7 {
            r.next_u16();
        }
        assert_eq!(r, l.random);
        let Some(LocalPhysics::Converge(ph)) = d.physics else {
            panic!()
        };
        let mut p = *first;
        for _ in 1..92 {
            move_converge(&mut p, &ph, 1);
        }
        assert_eq!((p.clock, p.relative), (90, [0; 3]));
        let out = ConvergePhysics {
            diverge: true,
            ..ph
        };
        let mut p = *first;
        p.clock = 0;
        move_converge(&mut p, &out, 0);
        assert_eq!(p.relative, [0; 3]);
        move_converge(&mut p, &out, 90);
        let flat = (i32::from(trig.cosine[ie]) * radius) >> 14;
        assert_eq!(p.relative[0], (i32::from(trig.cosine[ia]) * flat) >> 14);
    }

    fn myth2_crco() -> LocalDefinition {
        let data = tfl_bytes(
            "00019ac9 636f7277 6372636f 0000 ffff 0065 000b 0100 001a 0032 0000 00000600 \
             00000401 0000005a 00000000 0000 0000 001e 0000 0000 0000 003c 0000 00000000 \
             00000000 0001 0001 0001",
            0x80,
        );
        let phys = tfl_bytes("0004 0000 00000000 0000 0000 0000 4000", 0x40);
        LocalDefinition::parse(FourCC::new(*b"crco"), &data, |id| {
            (id == FourCC::new(*b"crco")).then(|| phys.clone())
        })
        .unwrap()
    }

    #[test]
    fn tfl_physics_type_1_faces_by_its_radial_speed() {
        let ph = OrbitPhysics {
            spin: 0x100,
            radial_speed: -0x1_0000,
            ..OrbitPhysics::default()
        };
        let mut p = Projectile {
            group: 0,
            velocity: [packed(0x1000, 0x100), 0x400, 0],
            period: 0x10000,
            clock: 0,
            rising: 0,
            relative: [0; 3],
            world: None,
            facing: 0,
            scale: 0x100,
            frame: 0,
            tick: 0,
            ticks_per_frame: 1,
            permutation: 0,
            age: 0,
            lifetime: 0,
            serial: 0,
        };
        let mut q = p;
        move_orbit(&mut p, &ph, 1, true);
        assert_eq!(p.facing, 0x1100);
        move_orbit(&mut q, &ph, 1, false);
        assert_eq!(q.facing, 0x9100);
        let mut data = lpgr_with(b"teos");
        data[0x18..0x1a].copy_from_slice(&[0, 4]);
        let made = |flags: u32| {
            let mut data = data.clone();
            data[..4].copy_from_slice(&(0x800 | flags).to_be_bytes());
            let d = LocalDefinition::parse_with_rules(
                FourCC::new(*b"teos"),
                &data,
                tfl_phys,
                Rules::Tfl,
            )
            .unwrap();
            let mut l = LocalProjectiles::with_rules(3, Rules::Tfl);
            l.add_definition(d, sequence());
            l.new_group(&NewGroup {
                definition: d.tag,
                point: [0x8000, 0x8000, 0],
                central: None,
                target: None,
                opacity: 0xffff,
                facing: 0,
            });
            let ground = |_: i32, _: i32| 0;
            let object = |_: ObjectRef| None;
            l.update(1, &world(&object, &ground));
            l.projectiles
                .iter()
                .flatten()
                .map(|p| p.relative)
                .collect::<Vec<_>>()
        };
        assert_eq!(made(0).len(), 4);
        assert_eq!(made(0), made(FLAG_EVEN_ANGLES));
    }

    #[test]
    fn tfl_physics_over_water_take_the_media_at_0() {
        let d = LocalDefinition::parse_with_rules(
            FourCC::new(*b"swar"),
            &lpgr_with(b"swar"),
            tfl_phys,
            Rules::Tfl,
        )
        .unwrap();
        let z = |bits: u8, pixel: bool| {
            let mut l = LocalProjectiles::with_rules(5, Rules::Tfl);
            l.add_definition(d, sequence());
            l.new_group(&NewGroup {
                definition: d.tag,
                point: [0x8000, 0x8000, 0x100],
                central: None,
                target: None,
                opacity: 0xffff,
                facing: 0,
            });
            let ground = |_: i32, _: i32| -0x300;
            let object = |_: ObjectRef| None;
            let cell = move |_: i32, _: i32| (bits, pixel);
            let mut w = world(&object, &ground);
            w.tfl_cell = Some(&cell);
            l.update(1, &w);
            l.projectiles.iter().flatten().next().unwrap().relative[2]
        };
        assert_eq!(z(0, true), 0x200 - 0x300 - 0x100);
        assert_eq!(z(0x08, false), 0x200 - 0x300 - 0x100);
        assert_eq!(z(0x08, true), 0x200 - 0x100);
        assert_eq!(z(0x10, false), 0x200 - 0x100);
        let mut data = lpgr_with(b"teos");
        data[..4].copy_from_slice(&0x804u32.to_be_bytes());
        let d =
            LocalDefinition::parse_with_rules(FourCC::new(*b"teos"), &data, tfl_phys, Rules::Tfl)
                .unwrap();
        let left = |bits: u8, pixel: bool| {
            let mut l = LocalProjectiles::with_rules(5, Rules::Tfl);
            l.add_definition(d, sequence());
            l.new_group(&NewGroup {
                definition: d.tag,
                point: [0x8000, 0x8000, -0x100],
                central: None,
                target: None,
                opacity: 0xffff,
                facing: 0,
            });
            let ground = |_: i32, _: i32| -0x1000;
            let object = |_: ObjectRef| None;
            let cell = move |_: i32, _: i32| (bits, pixel);
            let mut w = world(&object, &ground);
            w.tfl_cell = Some(&cell);
            l.update(1, &w);
            l.counts().0
        };
        assert_eq!(left(0, true), 1);
        assert_eq!(left(0x18, false), 1);
        assert_eq!(left(0x10, true), 0);
    }

    #[test]
    fn delete_group_clears_stale_monster_and_ambient_holders() {
        let mut l = LocalProjectiles::new(7);
        let short_tag = FourCC::new(*b"shrt");
        let long_tag = FourCC::new(*b"long");
        let mut short_def = coef();
        short_def.tag = short_tag;
        short_def.flags = FLAG_ON_DURATION;
        short_def.on_duration = (0, 0);
        short_def.physics = None;
        let mut long_def = coef();
        long_def.tag = long_tag;
        long_def.flags = 0;
        long_def.physics = None;
        l.add_definition(short_def, sequence());
        l.add_definition(long_def, sequence());

        l.start_for_event(&CombatEvent::MonsterLocalGroup {
            tick: 1,
            monster: 42,
            object: 1,
            identifier: 100,
            local_group: short_tag,
        });
        let first_slot = *l.monster_groups.get(&42).unwrap();
        let object = |_: ObjectRef| Some([0x1000, 0x1000, 0]);
        let ground = |_: i32, _: i32| 0;
        let w = world(&object, &ground);
        for _ in 0..4 {
            l.update(1, &w);
        }
        assert!(l.groups[first_slot].is_none());
        assert!(!l.monster_groups.contains_key(&42));

        let reused = l
            .new_group(&NewGroup {
                definition: long_tag,
                point: [0x2000, 0x2000, 0],
                central: None,
                target: None,
                opacity: 0xffff,
                facing: 0,
            })
            .unwrap();
        assert_eq!(reused, first_slot);

        l.start_for_event(&CombatEvent::MonsterLocalGroupOff {
            tick: 5,
            monster: 42,
        });
        assert_ne!(l.groups[reused].unwrap().state, GroupState::Ending);

        let amb = l
            .start_ambient_group(
                AmbientSource {
                    group: short_tag,
                    tries: 1,
                    period: 1,
                },
                [0x4000, 0x4000],
            )
            .unwrap();
        l.media_effect = Some((amb, 0xffff));
        for _ in 0..4 {
            l.update(1, &w);
        }
        assert!(l.media_effect.is_none());
    }

    #[test]
    fn extreme_lpgr_and_physics_values_wrap_without_panic() {
        let mut r = LocalRandom::new(123);
        let orbit = OrbitPhysics {
            spin: 0,
            spin_range: 0,
            spin_acceleration: 0,
            radial_speed: 0,
            rise: 0,
            bob_rate: 1,
            bob_height: i16::MAX,
        };
        let _ = start_orbit(&mut r, (i32::MAX, 0), &orbit, Some(0));

        let mut p = Projectile {
            group: 0,
            velocity: [i32::MIN, i32::MIN, 0],
            period: 0,
            clock: 0,
            rising: 0,
            relative: [100, 100, 0],
            world: None,
            facing: 0,
            scale: 0,
            frame: 0,
            tick: 0,
            ticks_per_frame: 1,
            permutation: 0,
            age: 0,
            lifetime: 0,
            serial: 0,
        };
        let pull = PullPhysics {
            pull: i32::MIN,
            speed_limit: i32::MAX,
            height: 0,
            on_ground: false,
        };
        move_pull(&mut p, &pull, 1, [0; 3], &|_, _| 0);
    }
}
