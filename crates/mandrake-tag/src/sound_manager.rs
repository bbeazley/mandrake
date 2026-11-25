use crate::sound::{AmbientSoundTag, PermutationHistory, SoundTag};
use crate::{MarkerEntry, MeshTag, TagArchive};
use mandrake_common::FourCC;
use mandrake_math::engine::{self, arctangent};

pub const WORLD_ONE: i32 = 512;
pub const FULL_VOLUME: i16 = 0x100;
pub const ONE_SHOT_CHANNEL_COUNT: usize = 5;
pub const AMBIENT_CHANNELS: [usize; 2] = [5, 6];
pub const STEREO_CHANNEL: usize = 7;
pub const CHANNEL_COUNT: usize = 8;
pub const CHANNEL_QUEUE_DEPTH: usize = 2;
pub const LISTENER_FORWARD_OFFSET: i32 = 0xc00;
pub const AMBIENT_REPORT_HALF_EXTENT: i32 = 0x6000;
pub const MAXIMUM_AMBIENT_REPORTS: usize = 7;
pub const STEAL_VOLUME_MARGIN: i32 = 0x2a;

pub const SOUND_FLAG_SINGLE_INSTANCE: u32 = 0x1;
pub const SOUND_FLAG_INDEPENDENT_INSTANCES: u32 = 0x2;
pub const DEFAULT_SOUND_PREFERENCE_FLAGS: u16 = 2;
pub const SOUND_PREFERENCE_3D: u16 = 1;
pub const SOUND_PREFERENCE_AMBIENT: u16 = 2;

pub fn sound_preference_flags(file: &[u8]) -> Option<u16> {
    if file.get(0x24..0x2c)? != b"prefsoun" || file.get(0x38..0x3a)? != [0, 1] {
        return None;
    }
    let flags = file.get(0x40..0x42)?;
    Some(u16::from_be_bytes([flags[0], flags[1]]))
}

pub const SOUND_FLAG_ENDS_WITH_OBJECT: u32 = 0x4;
pub const SOUND_FLAG_IGNORES_MEDIA: u32 = 0x8;

pub const AMBIENT_FLAG_NOT_ATTENUATED: u32 = 0x1;
pub const AMBIENT_FLAG_NOT_LOCATED: u32 = 0x2;

pub const REQUEST_FLAG_IGNORE_MEDIA: u16 = 0x1;
pub const REQUEST_FLAG_DETACHED: u16 = 0x2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalRandom {
    pub seed: u32,
}

impl LocalRandom {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }

    pub fn next_u16(&mut self) -> u16 {
        self.seed = self.seed.wrapping_mul(0x19660d).wrapping_add(0x3c6e_f35f);
        (self.seed >> 16) as u16
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct WorldPoint {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl WorldPoint {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn from_marker(marker: &MarkerEntry) -> Self {
        Self::new(marker.x, marker.y, marker.z)
    }
}

pub fn translate_point2d(p: WorldPoint, distance: i32, angle: u16) -> WorldPoint {
    let index = usize::from(angle >> 6);
    WorldPoint {
        x: p.x + (i32::from(engine::COSINE[index]) * distance) / 16384,
        y: p.y + (i32::from(engine::SINE[index]) * distance) / 16384,
        z: p.z,
    }
}

pub fn distance2d(a: WorldPoint, b: WorldPoint) -> i32 {
    engine::distance2d(a.x.wrapping_sub(b.x), a.y.wrapping_sub(b.y))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundListener {
    pub location: WorldPoint,
    pub facing: u16,
}

impl SoundListener {
    pub fn from_observer(observer: WorldPoint, facing: u16) -> Self {
        Self {
            location: translate_point2d(observer, LISTENER_FORWARD_OFFSET, facing),
            facing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DistanceFalloff {
    pub near_volume: i16,
    pub far_volume: i16,
    pub near_distance: i32,
    pub far_distance: i32,
}

impl DistanceFalloff {
    const fn new(near_volume: i16, far_volume: i16, near_distance: i32, far_distance: i32) -> Self {
        Self {
            near_volume,
            far_volume,
            near_distance,
            far_distance,
        }
    }

    pub fn volume_at(&self, distance: i32) -> i16 {
        if distance <= self.near_distance {
            self.near_volume
        } else if distance > self.far_distance {
            self.far_volume
        } else {
            let span = self.far_distance - self.near_distance;
            let delta =
                (self.near_volume as i32 - self.far_volume as i32) * (self.far_distance - distance);
            (delta / span + self.far_volume as i32) as i16
        }
    }
}

pub const DISTANCE_FALLOFF: [DistanceFalloff; 4] = [
    DistanceFalloff::new(0x100, 0, 0x1000, 0x3000),
    DistanceFalloff::new(0x100, 0, 0x1800, 0x4800),
    DistanceFalloff::new(0x100, 0x1c, 0x2000, 0x6000),
    DistanceFalloff::new(0x100, 0, 0x1800, 0x4800),
];

pub const UNDERWATER_DISTANCE_FALLOFF: [DistanceFalloff; 4] = [
    DistanceFalloff::new(0, 0, 0, 0),
    DistanceFalloff::new(0x80, 0, 0, 0x1c00),
    DistanceFalloff::new(0xc0, 0, 0, 0x2800),
    DistanceFalloff::new(0x80, 0, 0, 0x1c00),
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SoundVolumes {
    pub volume: i16,
    pub attenuated: i16,
    pub left: i16,
    pub right: i16,
}

impl SoundVolumes {
    pub fn flat(volume: i16) -> Self {
        Self {
            volume,
            attenuated: volume,
            left: volume,
            right: volume,
        }
    }
}

pub fn stereo_pan(volume: i16, listener: &SoundListener, location: WorldPoint) -> (i16, i16) {
    let dx = listener.location.x - location.x;
    let dy = listener.location.y - location.y;
    if dx == 0 && dy == 0 {
        return (volume, volume);
    }
    let angle = arctangent(dx, dy).wrapping_sub(listener.facing);
    let side = volume as i32 - (volume >> 2) as i32;
    let mut a = angle.wrapping_add(0xc000);
    if a >= 0x8001 {
        a = a.wrapping_neg();
    }
    let a = a as i32;
    if a > 0x4000 {
        let right = volume as i32 - ((a - 0x4000) * side) / 16384;
        (volume, right as i16)
    } else {
        let left = volume as i32 - ((0x4000 - a) * side) / 16384;
        (left as i16, volume)
    }
}

pub fn angle_and_volume_to_stereo_volume(volume: i16, angle: u16) -> (i16, i16) {
    let v = i32::from(volume);
    let v8 = v >> 3;
    let v78 = v - v8;
    let f = i32::from(angle & 0x3fff);
    let towards = |a: i32, b: i32| (b.wrapping_sub(a).wrapping_mul(f) >> 14) + a;
    let (right, left) = match angle >> 14 {
        0 => (towards(v78, v), towards(v78, v8)),
        1 => (v, (v78.wrapping_mul(f) >> 14) + v8),
        2 => (towards(v, v8), v),
        _ => (towards(v8, v78), towards(v, v78)),
    };
    (left as i16, right as i16)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanLaw {
    Myth2,
    Tfl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAllocator {
    Myth2,
    Tfl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbientRules {
    Myth2,
    Tfl,
}

#[derive(Debug, Clone, Copy)]
pub struct SoundRules {
    pub one_shot_channels: usize,
    pub ambient_channels: [usize; 2],
    pub stereo_channel: Option<usize>,
    pub pan: PanLaw,
    pub allocator: ChannelAllocator,
    pub ambient: AmbientRules,
    pub follow_always: bool,
    pub arctangent: fn(i32, i32) -> u16,
    pub distance: fn(WorldPoint, WorldPoint) -> i32,
}

impl SoundRules {
    pub const MYTH2: Self = Self {
        one_shot_channels: ONE_SHOT_CHANNEL_COUNT,
        ambient_channels: AMBIENT_CHANNELS,
        stereo_channel: Some(STEREO_CHANNEL),
        pan: PanLaw::Myth2,
        allocator: ChannelAllocator::Myth2,
        ambient: AmbientRules::Myth2,
        follow_always: false,
        arctangent,
        distance: distance2d,
    };

    pub const TFL: Self = Self {
        one_shot_channels: 4,
        ambient_channels: [4, 5],
        stereo_channel: None,
        pan: PanLaw::Tfl,
        allocator: ChannelAllocator::Tfl,
        ambient: AmbientRules::Tfl,
        follow_always: true,
        arctangent,
        distance: distance2d,
    };

    pub fn pan(&self, volume: i16, listener: &SoundListener, location: WorldPoint) -> (i16, i16) {
        match self.pan {
            PanLaw::Myth2 => stereo_pan(volume, listener, location),
            PanLaw::Tfl => {
                let dx = listener.location.x - location.x;
                let dy = listener.location.y - location.y;
                if dx == 0 && dy == 0 {
                    return (volume, volume);
                }
                let angle = (self.arctangent)(dx, dy).wrapping_sub(listener.facing);
                angle_and_volume_to_stereo_volume(volume, angle)
            }
        }
    }

    pub fn spatialize(
        &self,
        volume: i16,
        falloff: &DistanceFalloff,
        location: Option<WorldPoint>,
        listener: Option<&SoundListener>,
        skip_distance: bool,
    ) -> SoundVolumes {
        let Some(location) = location else {
            return SoundVolumes::flat(volume);
        };
        let Some(listener) = listener else {
            return SoundVolumes {
                volume,
                ..SoundVolumes::default()
            };
        };
        let attenuated = if skip_distance {
            volume
        } else {
            let distance = (self.distance)(location, listener.location);
            ((falloff.volume_at(distance) as i32 * volume as i32) >> 8) as i16
        };
        let (left, right) = self.pan(attenuated, listener, location);
        SoundVolumes {
            volume,
            attenuated,
            left,
            right,
        }
    }
}

impl Default for SoundRules {
    fn default() -> Self {
        Self::MYTH2
    }
}

pub fn spatialize(
    volume: i16,
    falloff: &DistanceFalloff,
    location: Option<WorldPoint>,
    listener: Option<&SoundListener>,
    skip_distance: bool,
) -> SoundVolumes {
    SoundRules::MYTH2.spatialize(volume, falloff, location, listener, skip_distance)
}

pub fn device_gain(attenuated: i16) -> f32 {
    let g = (attenuated as f32 * 0.00390625).clamp(0.0, 1.0) as f64;
    let exponent = ((-(-3.0 - 2.0 * g.powf(0.125))) as f32 / 6.0) as f64;
    g.powf(exponent) as f32
}

pub fn particle_report_volume(volume: i16) -> i16 {
    let quiet = i32::from(0x100i16.wrapping_sub(volume));
    (0x100 - ((quiet * quiet) >> 8)) as i16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelObject {
    pub index: i32,
    pub identifier: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectPlace {
    At(WorldPoint, [i16; 3]),
    Deleted,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStatus {
    Idle,
    Playing,
    Queued,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceSource {
    pub location: Option<WorldPoint>,
    pub velocity: [i16; 3],
}

pub fn reverb_preset(mesh_reverb: i32) -> usize {
    if (1..=21).contains(&mesh_reverb) {
        mesh_reverb as usize
    } else {
        19
    }
}

pub trait SoundChannelBackend {
    fn channel_status(&self, channel: usize) -> ChannelStatus;
    fn stop_channel(&mut self, channel: usize);
    fn set_channel_volume(&mut self, channel: usize, volumes: &SoundVolumes, pitch: i16);
    fn queue_permutation(&mut self, channel: usize, sound: usize, permutation: usize);
    fn set_channel_source(&mut self, _channel: usize, _source: &DeviceSource) {}
}

#[derive(Debug, Clone)]
pub struct SoundDefinition {
    pub id: FourCC,
    pub tag: SoundTag,
    pub history: PermutationHistory,
}

#[derive(Debug, Clone)]
pub struct AmbientDefinition {
    pub id: FourCC,
    pub tag: AmbientSoundTag,
    pub continuous: Option<usize>,
    pub periodic: Option<usize>,
    pub period_counter: i16,
    pub period: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmbientSoundSource {
    pub definition: usize,
    pub location: WorldPoint,
    pub volume: i16,
}

impl AmbientSoundSource {
    pub fn from_marker(definition: usize, marker: &MarkerEntry) -> Self {
        Self {
            definition,
            location: WorldPoint::from_marker(marker),
            volume: if marker.flags & 1 != 0 {
                0
            } else {
                FULL_VOLUME
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AmbientEntry {
    definition: usize,
    volumes: SoundVolumes,
    loudness: i16,
    location: Option<WorldPoint>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ChannelState {
    sound: Option<usize>,
    attenuated: i16,
    loudness: i16,
    volume: i16,
    pitch: i16,
    request_flags: u16,
    location: Option<WorldPoint>,
    object: Option<ChannelObject>,
    velocity: [i16; 3],
    started: u32,
    left: i16,
    right: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OneShotChannel {
    pub sound: usize,
    pub attenuated: i16,
    pub location: Option<WorldPoint>,
    pub object: Option<ChannelObject>,
}

pub struct SoundManager {
    pub sounds: Vec<SoundDefinition>,
    pub ambient_definitions: Vec<AmbientDefinition>,
    pub sources: Vec<AmbientSoundSource>,
    pub stereo_background: Option<usize>,
    pub global_ambient: Option<usize>,
    pub map_size: (i32, i32),
    pub random: LocalRandom,
    pub fire_definition: Option<usize>,
    pub particle_definition: Option<usize>,
    pub particle_volume: i16,
    pub palette_definitions: Vec<usize>,
    pub game_sounds: Vec<usize>,
    pub ambient_sound: bool,
    pub sound_3d: bool,
    pub rules: SoundRules,
    pub time: u32,
    channels: [ChannelState; CHANNEL_COUNT],
    media_query: Option<Box<dyn Fn(WorldPoint) -> bool + Send>>,
}

impl std::fmt::Debug for SoundManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundManager")
            .field("sounds", &self.sounds.len())
            .field("ambient_definitions", &self.ambient_definitions.len())
            .field("sources", &self.sources.len())
            .field("stereo_background", &self.stereo_background)
            .finish()
    }
}

pub type FireReport<'a> =
    &'a dyn Fn(&SoundListener, &AmbientSoundTag, i16) -> Option<(WorldPoint, i16)>;

pub type TagResolver<'a> = dyn FnMut(FourCC, FourCC) -> Option<Vec<u8>> + 'a;

pub fn archive_resolver<'a>(
    archives: &'a [&'a TagArchive],
) -> impl FnMut(FourCC, FourCC) -> Option<Vec<u8>> + 'a {
    move |tag_type, id| {
        archives.iter().find_map(|a| {
            a.find_tag_by_id(tag_type, id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    }
}

impl SoundManager {
    pub fn new(seed: u32) -> Self {
        Self {
            sounds: Vec::new(),
            ambient_definitions: Vec::new(),
            sources: Vec::new(),
            stereo_background: None,
            global_ambient: None,
            map_size: (0, 0),
            random: LocalRandom::new(seed),
            fire_definition: None,
            particle_definition: None,
            particle_volume: 0,
            palette_definitions: Vec::new(),
            game_sounds: Vec::new(),
            ambient_sound: DEFAULT_SOUND_PREFERENCE_FLAGS & SOUND_PREFERENCE_AMBIENT != 0,
            sound_3d: false,
            rules: SoundRules::MYTH2,
            time: 0,
            channels: [ChannelState::default(); CHANNEL_COUNT],
            media_query: None,
        }
    }

    pub fn for_map(
        mesh: &MeshTag,
        stereo_background: Option<FourCC>,
        resolver: &mut TagResolver<'_>,
        seed: u32,
    ) -> Self {
        let mut manager = Self::new(seed);
        manager.map_size = (mesh.cell_width as i32, mesh.cell_height as i32);
        manager.stereo_background =
            stereo_background.and_then(|id| manager.load_ambient_definition(id, resolver));
        for palette in mesh
            .palettes
            .iter()
            .filter(|p| p.marker_type == crate::MarkerPaletteEntry::TYPE_SOUND)
        {
            if let Some(definition) = manager.load_ambient_definition(palette.tag_id, resolver) {
                manager.palette_definitions.push(definition);
            }
        }
        for marker in mesh.sound_markers() {
            let Some(id) = mesh.palette_tag_for_marker(marker) else {
                continue;
            };
            if let Some(definition) = manager.ambient_index(id) {
                manager
                    .sources
                    .push(AmbientSoundSource::from_marker(definition, marker));
            }
        }
        manager
    }

    pub fn set_media_query(&mut self, query: impl Fn(WorldPoint) -> bool + Send + 'static) {
        self.media_query = Some(Box::new(query));
    }

    pub fn sound_index(&self, id: FourCC) -> Option<usize> {
        self.sounds.iter().position(|s| s.id == id)
    }

    pub fn ambient_index(&self, id: FourCC) -> Option<usize> {
        self.ambient_definitions.iter().position(|a| a.id == id)
    }

    pub fn load_sound_definition(
        &mut self,
        id: FourCC,
        resolver: &mut TagResolver<'_>,
    ) -> Option<usize> {
        if let Some(i) = self.sound_index(id) {
            return Some(i);
        }
        let data = resolver(FourCC::new(*b"soun"), id)?;
        let tag = SoundTag::from_bytes(&data).ok()?;
        self.sounds.push(SoundDefinition {
            id,
            tag,
            history: PermutationHistory::default(),
        });
        Some(self.sounds.len() - 1)
    }

    pub fn load_ambient_definition(
        &mut self,
        id: FourCC,
        resolver: &mut TagResolver<'_>,
    ) -> Option<usize> {
        if let Some(i) = self.ambient_index(id) {
            return Some(i);
        }
        let data = resolver(FourCC::new(*b"amso"), id)?;
        let tag = AmbientSoundTag::from_bytes(&data).ok()?;
        let continuous = tag
            .continuous_sound()
            .and_then(|s| self.load_sound_definition(s, resolver));
        let periodic = tag
            .periodic_sound()
            .and_then(|s| self.load_sound_definition(s, resolver));
        self.ambient_definitions.push(AmbientDefinition {
            id,
            tag,
            continuous,
            periodic,
            period_counter: 0,
            period: 0,
        });
        Some(self.ambient_definitions.len() - 1)
    }

    pub fn engine_ambient_index(&self, definition: usize) -> Option<usize> {
        let mut stack = [0usize; 32];
        let mut len = 0usize;
        let mut spill = Vec::new();
        for &d in self
            .stereo_background
            .iter()
            .chain(&self.particle_definition)
            .chain(&self.palette_definitions)
            .chain(&self.fire_definition)
        {
            if d == definition {
                return Some(len + spill.len());
            }
            if stack[..len].contains(&d) || spill.contains(&d) {
                continue;
            }
            if len < stack.len() {
                stack[len] = d;
                len += 1;
            } else {
                spill.push(d);
            }
        }
        None
    }

    pub fn report_sound(&self, definition: usize) -> Option<usize> {
        self.engine_ambient_index(definition)
            .and_then(|i| self.game_sounds.get(i).copied())
            .or(self.ambient_definitions[definition].continuous)
    }

    pub fn report_volumes(
        &mut self,
        definition: usize,
        location: Option<WorldPoint>,
        volume: i16,
        listener: Option<&SoundListener>,
        stereo: bool,
    ) -> Option<SoundVolumes> {
        let mut entries = [None];
        self.report(&mut entries, definition, location, volume, listener, stereo);
        entries[0].map(|e| e.volumes)
    }

    pub fn channel_sound(&self, channel: usize) -> Option<usize> {
        self.channels.get(channel).and_then(|c| c.sound)
    }

    fn falloff(&self, sound: usize, flags: u16, location: Option<WorldPoint>) -> DistanceFalloff {
        let def = &self.sounds[sound].tag;
        let class = (def.loudness.clamp(0, 3)) as usize;
        let underwater = flags & REQUEST_FLAG_IGNORE_MEDIA == 0
            && location.is_some_and(|p| self.media_query.as_ref().is_some_and(|q| q(p)))
            && def.flags & SOUND_FLAG_IGNORES_MEDIA == 0;
        if underwater {
            UNDERWATER_DISTANCE_FALLOFF[class]
        } else {
            DISTANCE_FALLOFF[class]
        }
    }

    fn random_pitch_and_volume(&mut self, sound: usize) -> (i16, i16) {
        let rng = &mut self.random;
        let tag = &self.sounds[sound].tag;
        let pitch = if tag.pitch_delta > 0 {
            tag.pitch_lower_bound as i32 + ((rng.next_u16() as i32 * tag.pitch_delta as i32) >> 16)
        } else {
            tag.pitch_lower_bound as i32
        };
        let volume = if tag.volume_delta > 0 {
            tag.volume_lower_bound as i32
                + ((rng.next_u16() as u32 * tag.volume_delta as u32) >> 16) as i32
        } else {
            tag.volume_lower_bound as i32
        };
        (pitch as i16, volume as i16)
    }

    fn choose_permutation(&mut self, sound: usize) -> Option<usize> {
        let rng = &mut self.random;
        let def = &mut self.sounds[sound];
        def.tag
            .choose_permutation(&mut def.history, || rng.next_u16())
    }

    pub fn play_sound(
        &mut self,
        sound: usize,
        location: Option<WorldPoint>,
        pitch_scale: i16,
        flags: u16,
        listener: Option<&SoundListener>,
        backend: &mut dyn SoundChannelBackend,
    ) -> Option<usize> {
        self.play_object_sound(
            sound,
            location,
            None,
            [0; 3],
            pitch_scale,
            flags,
            listener,
            backend,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn play_object_sound(
        &mut self,
        sound: usize,
        location: Option<WorldPoint>,
        object: Option<ChannelObject>,
        velocity: [i16; 3],
        pitch_scale: i16,
        flags: u16,
        listener: Option<&SoundListener>,
        backend: &mut dyn SoundChannelBackend,
    ) -> Option<usize> {
        if sound >= self.sounds.len() {
            return None;
        }
        let (pitch, volume) = self.random_pitch_and_volume(sound);
        let pitch = ((pitch as i32 * pitch_scale as i32) >> 8) as i16;
        let falloff = self.falloff(sound, flags, location);
        let volumes = self
            .rules
            .spatialize(volume, &falloff, location, listener, false);
        if volumes.attenuated <= 0 {
            self.choose_permutation(sound);
            return None;
        }
        let velocity = if location.is_some() { velocity } else { [0; 3] };
        let channel =
            self.allocate_channel(sound, volumes, pitch, flags, location, velocity, backend)?;
        if flags & REQUEST_FLAG_DETACHED == 0 {
            self.channels[channel].object = object;
        }
        Some(channel)
    }

    #[allow(clippy::too_many_arguments)]
    fn allocate_channel(
        &mut self,
        sound: usize,
        volumes: SoundVolumes,
        pitch: i16,
        request_flags: u16,
        location: Option<WorldPoint>,
        velocity: [i16; 3],
        backend: &mut dyn SoundChannelBackend,
    ) -> Option<usize> {
        let permutation = self.choose_permutation(sound);
        let permutation = permutation.filter(|_| volumes.attenuated > 0)?;
        let loudness = self.sounds[sound].tag.loudness;
        let channel = match self.rules.allocator {
            ChannelAllocator::Myth2 => self.myth2_channel(sound, volumes, backend),
            ChannelAllocator::Tfl => self.tfl_channel(sound, volumes, backend),
        }?;
        if self.channels[channel].sound.is_some() {
            backend.stop_channel(channel);
        }
        self.channels[channel] = ChannelState {
            sound: Some(sound),
            attenuated: volumes.attenuated,
            loudness,
            volume: volumes.volume,
            pitch,
            request_flags,
            location,
            object: None,
            velocity,
            started: self.time,
            left: volumes.left,
            right: volumes.right,
        };
        self.give_source(channel, backend);
        backend.set_channel_volume(channel, &volumes, pitch);
        backend.queue_permutation(channel, sound, permutation);
        Some(channel)
    }

    fn tfl_channel(
        &self,
        sound: usize,
        volumes: SoundVolumes,
        backend: &dyn SoundChannelBackend,
    ) -> Option<usize> {
        let tag = &self.sounds[sound].tag;
        let (loudness, flags) = (tag.loudness, tag.flags);
        let attenuated = i32::from(volumes.attenuated);
        let independent = flags & SOUND_FLAG_INDEPENDENT_INSTANCES != 0;
        let (mut best, mut free): (Option<usize>, bool) = (None, false);
        for ch in 0..self.rules.one_shot_channels {
            let state = &self.channels[ch];
            if state.sound.is_none() || backend.channel_status(ch) == ChannelStatus::Idle {
                free = true;
                best = Some(ch);
                continue;
            }
            let held = i32::from(state.attenuated);
            let same = state.sound == Some(sound);
            let stealable = state.loudness < loudness
                || (state.loudness == loudness && held + STEAL_VOLUME_MARGIN < attenuated);
            if !stealable && !(same && held < attenuated + STEAL_VOLUME_MARGIN) {
                if same && !independent {
                    return None;
                }
                continue;
            }
            if same && !independent {
                if flags & SOUND_FLAG_SINGLE_INSTANCE != 0
                    || self.time < state.started.wrapping_add(2)
                {
                    return None;
                }
                return Some(ch);
            }
            if best.is_none_or(|b| !free && state.attenuated <= self.channels[b].attenuated) {
                best = Some(ch);
            }
        }
        best
    }

    fn myth2_channel(
        &self,
        sound: usize,
        volumes: SoundVolumes,
        backend: &dyn SoundChannelBackend,
    ) -> Option<usize> {
        const FREE: u8 = 1;
        const SAME: u8 = 2;
        const STEALABLE: u8 = 4;

        let loudness = self.sounds[sound].tag.loudness;
        let mut bits = [0u8; ONE_SHOT_CHANNEL_COUNT];
        let (mut any_free, mut any_same) = (false, false);
        for (ch, bits) in bits.iter_mut().enumerate() {
            let state = self.channels[ch];
            let status = backend.channel_status(ch);
            if state.sound.is_none() || status == ChannelStatus::Idle {
                *bits |= FREE;
                any_free = true;
                continue;
            }
            if state.sound == Some(sound) {
                *bits |= SAME;
                any_same = true;
            }
            if status != ChannelStatus::Queued
                && (loudness > state.loudness
                    || (loudness == state.loudness
                        && volumes.attenuated as i32
                            > state.attenuated as i32 + STEAL_VOLUME_MARGIN))
            {
                *bits |= STEALABLE;
            }
        }
        let first = |pred: &dyn Fn(u8) -> bool| bits.iter().position(|b| pred(*b));
        let flags = self.sounds[sound].tag.flags;
        if flags & SOUND_FLAG_INDEPENDENT_INSTANCES != 0 {
            if any_free {
                first(&|b| b & FREE != 0)
            } else if flags & SOUND_FLAG_SINGLE_INSTANCE != 0 {
                first(&|b| b & STEALABLE != 0 && b & SAME == 0)
            } else {
                first(&|b| b & STEALABLE != 0)
            }
        } else if any_same {
            if flags & SOUND_FLAG_SINGLE_INSTANCE != 0 {
                None
            } else {
                first(&|b| b & (SAME | STEALABLE) == SAME | STEALABLE)
            }
        } else if any_free {
            first(&|b| b & FREE != 0)
        } else {
            first(&|b| b & STEALABLE != 0)
        }
    }

    fn stop(&mut self, channel: usize, backend: &mut dyn SoundChannelBackend) {
        if self.channels[channel].sound.take().is_some() {
            backend.stop_channel(channel);
        }
    }

    fn refill(&mut self, channel: usize, backend: &mut dyn SoundChannelBackend) {
        let Some(sound) = self.channels[channel].sound else {
            return;
        };
        for _ in 0..CHANNEL_QUEUE_DEPTH {
            if backend.channel_status(channel) == ChannelStatus::Queued {
                break;
            }
            let Some(permutation) = self.choose_permutation(sound) else {
                break;
            };
            backend.queue_permutation(channel, sound, permutation);
        }
    }

    fn assign_loop(
        &mut self,
        channel: usize,
        sound: usize,
        entry: &AmbientEntry,
        backend: &mut dyn SoundChannelBackend,
    ) {
        self.channels[channel] = ChannelState {
            sound: Some(sound),
            attenuated: entry.volumes.attenuated,
            loudness: entry.loudness,
            location: entry.location,
            started: self.time,
            left: entry.volumes.left,
            right: entry.volumes.right,
            ..ChannelState::default()
        };
        self.give_source(channel, backend);
        backend.set_channel_volume(channel, &entry.volumes, FULL_VOLUME);
    }

    fn give_source(&self, channel: usize, backend: &mut dyn SoundChannelBackend) {
        if self.sound_3d {
            let state = &self.channels[channel];
            backend.set_channel_source(
                channel,
                &DeviceSource {
                    location: state.location,
                    velocity: state.velocity,
                },
            );
        }
    }

    pub fn channel_volumes(&self, channel: usize) -> Option<(usize, SoundVolumes)> {
        let state = self.channels.get(channel)?;
        Some((
            state.sound?,
            SoundVolumes {
                volume: state.volume,
                attenuated: state.attenuated,
                left: state.left,
                right: state.right,
            },
        ))
    }

    pub fn one_shot(&self, channel: usize) -> Option<OneShotChannel> {
        let state = self
            .channels
            .get(channel)
            .filter(|_| channel < self.rules.one_shot_channels)?;
        Some(OneShotChannel {
            sound: state.sound?,
            attenuated: state.attenuated,
            location: state.location,
            object: state.object,
        })
    }

    pub fn follow_objects(
        &mut self,
        place: impl Fn(ChannelObject) -> ObjectPlace,
        backend: &mut dyn SoundChannelBackend,
    ) {
        for channel in 0..self.rules.one_shot_channels {
            let state = self.channels[channel];
            let (Some(sound), Some(object)) = (state.sound, state.object) else {
                continue;
            };
            match place(object) {
                ObjectPlace::At(at, velocity) => {
                    let state = &mut self.channels[channel];
                    state.velocity = velocity;
                    state.location = Some(at);
                }
                ObjectPlace::Unknown => {}
                ObjectPlace::Deleted => {
                    if self.sounds[sound].tag.flags & SOUND_FLAG_ENDS_WITH_OBJECT != 0 {
                        self.stop(channel, backend);
                    } else {
                        self.channels[channel].object = None;
                    }
                }
            }
        }
    }

    fn update_one_shots(
        &mut self,
        listener: Option<&SoundListener>,
        backend: &mut dyn SoundChannelBackend,
    ) {
        for channel in 0..self.rules.one_shot_channels {
            let state = self.channels[channel];
            let (Some(sound), Some(location), Some(_)) = (state.sound, state.location, listener)
            else {
                continue;
            };
            let falloff = self.falloff(sound, state.request_flags, Some(location));
            let volumes =
                self.rules
                    .spatialize(state.volume, &falloff, Some(location), listener, false);
            if volumes.attenuated == 0 && !self.rules.follow_always {
                self.stop(channel, backend);
            } else {
                let held = &mut self.channels[channel];
                held.attenuated = volumes.attenuated;
                (held.left, held.right) = (volumes.left, volumes.right);
                self.give_source(channel, backend);
                backend.set_channel_volume(channel, &volumes, state.pitch);
            }
        }
    }

    pub fn update(
        &mut self,
        ticks: i32,
        listener: Option<&SoundListener>,
        backend: &mut dyn SoundChannelBackend,
    ) {
        self.update_with_fire(ticks, listener, backend, &|_, _, _| None);
    }

    pub fn update_with_fire(
        &mut self,
        ticks: i32,
        listener: Option<&SoundListener>,
        backend: &mut dyn SoundChannelBackend,
        fire: FireReport<'_>,
    ) {
        if ticks <= 0 {
            return;
        }
        self.time = self.time.wrapping_add(ticks as u32);
        for ch in 0..self.rules.one_shot_channels {
            if self.channels[ch].sound.is_some()
                && backend.channel_status(ch) == ChannelStatus::Idle
            {
                self.stop(ch, backend);
            }
        }
        if self.sound_3d || self.rules.follow_always {
            self.update_one_shots(listener, backend);
        }
        self.update_stereo_background(listener, backend);
        self.update_ambient(ticks, listener, backend, fire);
    }

    fn report(
        &mut self,
        entries: &mut [Option<AmbientEntry>],
        definition: usize,
        location: Option<WorldPoint>,
        volume: i16,
        listener: Option<&SoundListener>,
        stereo: bool,
    ) {
        let continuous = self.ambient_definitions[definition].continuous;
        let independent = |m: &Self| {
            continuous
                .is_some_and(|c| m.sounds[c].tag.flags & SOUND_FLAG_INDEPENDENT_INSTANCES != 0)
        };
        let tfl = self.rules.ambient == AmbientRules::Tfl;
        let slot = if tfl {
            entries
                .iter()
                .position(|e| e.is_none_or(|e| e.definition == definition))
        } else if stereo {
            match &entries[0] {
                None => Some(0),
                Some(e) if e.definition == definition && !independent(self) => Some(0),
                Some(_) => None,
            }
        } else if continuous.is_none() {
            entries.iter().position(Option::is_none)
        } else {
            let independent = independent(self);
            entries.iter().position(|e| match e {
                None => true,
                Some(e) => e.definition == definition && !independent,
            })
        };
        let Some(slot) = slot else {
            return;
        };
        let entry = entries[slot].get_or_insert(AmbientEntry {
            definition,
            volumes: SoundVolumes {
                volume,
                ..SoundVolumes::default()
            },
            loudness: 0,
            location,
        });
        let new = if let (true, Some(sound)) = (tfl, continuous) {
            let (_pitch, base) = self.random_pitch_and_volume(sound);
            let falloff = self.falloff(sound, REQUEST_FLAG_IGNORE_MEDIA, location);
            let v = self
                .rules
                .spatialize(base, &falloff, location, listener, false);
            let scale = |x: i16| ((x as i32 * volume as i32) >> 8) as i16;
            SoundVolumes {
                volume: v.volume,
                attenuated: scale(v.attenuated),
                left: scale(v.left),
                right: scale(v.right),
            }
        } else if continuous.is_some() {
            let sound = self
                .report_sound(definition)
                .expect("a definition with a loop has a report sound");
            let (_pitch, base) = self.random_pitch_and_volume(sound);
            if stereo {
                let v = ((base as i32 * volume as i32) >> 8) as i16;
                SoundVolumes::flat(v)
            } else {
                let v = spatialize(base, &DISTANCE_FALLOFF[0], location, listener, true);
                let scale = |x: i16| ((x as i32 * volume as i32) >> 8) as i16;
                SoundVolumes {
                    volume: v.volume,
                    attenuated: scale(v.attenuated),
                    left: scale(v.left),
                    right: scale(v.right),
                }
            }
        } else {
            SoundVolumes::flat(FULL_VOLUME)
        };
        entry.volumes.attenuated = entry.volumes.attenuated.max(new.attenuated);
        entry.volumes.left = entry.volumes.left.max(new.left);
        entry.volumes.right = entry.volumes.right.max(new.right);
    }

    fn finish_loop_entry(&self, entry: &mut Option<AmbientEntry>) {
        let Some(e) = entry else {
            return;
        };
        let Some(c) = self.ambient_definitions[e.definition].continuous else {
            *entry = None;
            return;
        };
        let tag = &self.sounds[c].tag;
        let scale = |x: i16| ((x as i32 * tag.volume_lower_bound as i32) >> 8) as i16;
        e.volumes.attenuated = scale(e.volumes.attenuated);
        if e.volumes.attenuated <= 0 && self.rules.ambient == AmbientRules::Myth2 {
            *entry = None;
            return;
        }
        e.volumes.left = scale(e.volumes.left);
        e.volumes.right = scale(e.volumes.right);
        e.loudness = tag.loudness;
    }

    fn update_stereo_background(
        &mut self,
        listener: Option<&SoundListener>,
        backend: &mut dyn SoundChannelBackend,
    ) {
        let Some(stereo_channel) = self.rules.stereo_channel else {
            return;
        };
        let mut entries = [None];
        if self.ambient_sound {
            if let Some(global) = self.global_ambient {
                self.report(&mut entries, global, None, FULL_VOLUME, listener, true);
            }
            if listener.is_some()
                && let Some(bed) = self.stereo_background
            {
                self.report(&mut entries, bed, None, FULL_VOLUME, listener, true);
            }
        }
        self.finish_loop_entry(&mut entries[0]);
        let Some(entry) = entries[0] else {
            self.stop(stereo_channel, backend);
            return;
        };
        let sound = self.ambient_definitions[entry.definition]
            .continuous
            .expect("finished entries have a continuous sound");
        if self.channels[stereo_channel].sound != Some(sound) {
            self.stop(stereo_channel, backend);
        }
        self.assign_loop(stereo_channel, sound, &entry, backend);
        self.refill(stereo_channel, backend);
    }

    fn update_ambient(
        &mut self,
        ticks: i32,
        listener: Option<&SoundListener>,
        backend: &mut dyn SoundChannelBackend,
        fire: FireReport<'_>,
    ) {
        let tfl = self.rules.ambient == AmbientRules::Tfl;
        let mut all: [Option<AmbientEntry>; MAXIMUM_AMBIENT_REPORTS] =
            [None; MAXIMUM_AMBIENT_REPORTS];
        let (entries, limit) = if tfl {
            (&mut all[..5], usize::MAX)
        } else {
            (&mut all[..], MAXIMUM_AMBIENT_REPORTS)
        };
        let ambient_channels = self.rules.ambient_channels;
        let mut reports = 0;
        if tfl
            && self.rules.stereo_channel.is_none()
            && let Some(global) = self.global_ambient.or(self.stereo_background)
        {
            self.report(entries, global, None, FULL_VOLUME, listener, false);
            reports += 1;
        }
        if let Some(l) = listener {
            if let Some(definition) = self.fire_definition
                && let Some(c) = self.ambient_definitions[definition].continuous
            {
                let tag = &self.sounds[c].tag;
                let (lower, delta) = (tag.volume_lower_bound, tag.volume_delta);
                let volume = if delta > 0 {
                    let r = i32::from(self.random.next_u16());
                    lower.wrapping_add(((i32::from(delta) * r) >> 16) as i16)
                } else {
                    lower
                };
                let amfi = &self.ambient_definitions[definition].tag;
                if let Some((location, volume)) = fire(l, amfi, volume) {
                    self.report(entries, definition, Some(location), volume, listener, false);
                    reports += 1;
                }
            }
            if self.ambient_sound
                && let Some(definition) = self.particle_definition
            {
                let volume = particle_report_volume(self.particle_volume);
                let at = (!tfl).then_some(l.location);
                self.report(entries, definition, at, volume, listener, false);
                reports += 1;
            }
            let sources = if self.ambient_sound {
                self.sources.len()
            } else {
                0
            };
            for i in 0..sources {
                let source = self.sources[i];
                let def = &self.ambient_definitions[source.definition].tag;
                let flags = def.flags;
                let mut volume = source.volume as i32;
                if tfl || flags & AMBIENT_FLAG_NOT_ATTENUATED == 0 {
                    let (sx, sy) = (source.location.x, source.location.y);
                    let (lx, ly) = (l.location.x, l.location.y);
                    if sx < lx - AMBIENT_REPORT_HALF_EXTENT
                        || sx > lx + AMBIENT_REPORT_HALF_EXTENT
                        || sy < ly - AMBIENT_REPORT_HALF_EXTENT
                        || sy > ly + AMBIENT_REPORT_HALF_EXTENT
                    {
                        continue;
                    }
                    let distance = (self.rules.distance)(l.location, source.location);
                    let (inner, outer) = (def.inner_radius as i32, def.outer_radius as i32);
                    if distance >= outer {
                        continue;
                    }
                    if distance >= inner && outer - inner > 0 {
                        volume = (outer - distance) * volume / (outer - inner);
                    }
                    if volume == 0 {
                        continue;
                    }
                }
                if reports >= limit {
                    continue;
                }
                let location =
                    (tfl || flags & AMBIENT_FLAG_NOT_LOCATED == 0).then_some(source.location);
                self.report(
                    entries,
                    source.definition,
                    location,
                    volume as i16,
                    listener,
                    false,
                );
                reports += 1;
            }
        }
        if reports == 0 {
            for ch in ambient_channels {
                self.stop(ch, backend);
            }
            return;
        }

        for slot in entries.iter_mut() {
            let Some(entry) = *slot else {
                continue;
            };
            self.tick_periodic(entry, ticks, listener, backend);
            self.finish_loop_entry(slot);
        }

        loop {
            let mut count = 0;
            let mut quietest: Option<usize> = None;
            for (i, e) in entries.iter().enumerate() {
                let Some(e) = e else {
                    continue;
                };
                count += 1;
                quietest = match quietest {
                    Some(q) => {
                        let m = entries[q].as_ref().unwrap();
                        if e.loudness < m.loudness
                            || (e.loudness == m.loudness
                                && e.volumes.attenuated <= m.volumes.attenuated)
                        {
                            Some(i)
                        } else {
                            Some(q)
                        }
                    }
                    None => Some(i),
                };
            }
            if count <= 2 {
                break;
            }
            entries[quietest.unwrap()] = None;
        }

        let mut matched = [false; 2];
        for slot in entries.iter_mut() {
            let Some(entry) = *slot else {
                continue;
            };
            let sound = self.ambient_definitions[entry.definition]
                .continuous
                .unwrap();
            for (k, ch) in ambient_channels.into_iter().enumerate() {
                if !matched[k] && self.channels[ch].sound == Some(sound) {
                    self.assign_loop(ch, sound, &entry, backend);
                    matched[k] = true;
                    *slot = None;
                    break;
                }
            }
        }
        for (k, ch) in ambient_channels.into_iter().enumerate() {
            if matched[k] {
                continue;
            }
            self.stop(ch, backend);
            if let Some(slot) = entries.iter_mut().find(|e| e.is_some()) {
                let entry = slot.take().unwrap();
                let sound = self.ambient_definitions[entry.definition]
                    .continuous
                    .unwrap();
                self.assign_loop(ch, sound, &entry, backend);
            }
        }
        for ch in ambient_channels {
            self.refill(ch, backend);
        }
    }

    fn tick_periodic(
        &mut self,
        entry: AmbientEntry,
        ticks: i32,
        listener: Option<&SoundListener>,
        backend: &mut dyn SoundChannelBackend,
    ) {
        let definition = entry.definition;
        let Some(periodic) = self.ambient_definitions[definition].periodic else {
            return;
        };
        let def = &mut self.ambient_definitions[definition];
        def.period_counter = def.period_counter.wrapping_add(ticks as i16);
        if def.period_counter <= def.period {
            return;
        }
        if def.period != 0 && self.rules.ambient == AmbientRules::Tfl {
            let angle = self.random.next_u16();
            let (pitch, volume) = self.random_pitch_and_volume(periodic);
            let facing = listener.map_or(0, |l| l.facing);
            let (left, right) =
                angle_and_volume_to_stereo_volume(volume, angle.wrapping_sub(facing));
            let volumes = SoundVolumes {
                volume,
                attenuated: volume,
                left,
                right,
            };
            self.allocate_channel(periodic, volumes, pitch, 0, None, [0; 3], backend);
        } else if def.period != 0 {
            let radius = def.tag.random_sound_radius as i32;
            let offset = if radius > 0 {
                (self.random.next_u16() as i32 * radius) >> 16
            } else {
                0
            };
            let angle = self.random.next_u16();
            let (pitch, volume) = self.random_pitch_and_volume(periodic);
            let origin = entry.location.unwrap_or_default();
            let index = engine::angle_index(angle);
            let mut location = WorldPoint {
                x: origin.x + (i32::from(engine::COSINE[index]) * offset) / 16384,
                y: origin.y + (i32::from(engine::SINE[index]) * offset) / 16384,
                z: origin.z,
            };
            let (w, h) = (self.map_size.0 * WORLD_ONE, self.map_size.1 * WORLD_ONE);
            location.x = if location.x < 0 {
                0
            } else {
                location.x.min(w - 1)
            };
            location.y = if location.y < 0 {
                0
            } else {
                location.y.min(h - 1)
            };
            let falloff = self.falloff(periodic, 0, Some(location));
            let volumes = spatialize(volume, &falloff, Some(location), listener, false);
            self.allocate_channel(periodic, volumes, pitch, 0, Some(location), [0; 3], backend);
        }
        let def = &mut self.ambient_definitions[definition];
        def.period_counter = 0;
        let lower = def.tag.period_lower_bound as i32;
        let delta = def.tag.period_delta as i32;
        let period = if delta > 0 {
            lower + ((self.random.next_u16() as u32 * delta as u32) >> 16) as i32
        } else {
            lower
        };
        self.ambient_definitions[definition].period = period as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound::{SampledSoundHeader, SoundPermutation};

    #[test]
    fn tfl_pan_quarters() {
        let p = angle_and_volume_to_stereo_volume;
        assert_eq!(p(0x100, 0), (0xe0, 0xe0));
        assert_eq!(p(0x100, 0x4000), (0x20, 0x100));
        assert_eq!(p(0x100, 0x8000), (0x100, 0x100));
        assert_eq!(p(0x100, 0xc000), (0x100, 0x20));
        assert_eq!(p(0x100, 0x6000), (0x20 + 0x70, 0x100));
        let l = SoundListener {
            location: WorldPoint::new(0, 0, 0),
            facing: 0,
        };
        let tfl = SoundRules::TFL;
        assert_eq!(
            tfl.pan(0x100, &l, WorldPoint::new(1000, 0, 0)),
            (0x100, 0x100)
        );
        assert_eq!(
            tfl.pan(0x100, &l, WorldPoint::new(-1000, 0, 0)),
            (0xe0, 0xe0)
        );
        assert_eq!(tfl.pan(0x80, &l, WorldPoint::new(0, 0, 0)), (0x80, 0x80));
    }

    #[derive(Default)]
    struct MockBackend {
        queued: [Vec<(usize, usize)>; CHANNEL_COUNT],
        volumes: [Option<(SoundVolumes, i16)>; CHANNEL_COUNT],
        stops: Vec<usize>,
        sources: Vec<(usize, DeviceSource)>,
    }

    impl SoundChannelBackend for MockBackend {
        fn channel_status(&self, channel: usize) -> ChannelStatus {
            match self.queued[channel].len() {
                0 => ChannelStatus::Idle,
                1 => ChannelStatus::Playing,
                _ => ChannelStatus::Queued,
            }
        }
        fn stop_channel(&mut self, channel: usize) {
            self.queued[channel].clear();
            self.stops.push(channel);
        }
        fn set_channel_volume(&mut self, channel: usize, volumes: &SoundVolumes, pitch: i16) {
            self.volumes[channel] = Some((*volumes, pitch));
        }
        fn queue_permutation(&mut self, channel: usize, sound: usize, permutation: usize) {
            self.queued[channel].push((sound, permutation));
        }
        fn set_channel_source(&mut self, channel: usize, source: &DeviceSource) {
            self.sources.push((channel, *source));
        }
    }

    impl MockBackend {
        fn finish(&mut self, channel: usize) {
            self.queued[channel].clear();
        }
    }

    fn sound(flags: u32, loudness: i16, permutations: usize) -> SoundTag {
        SoundTag {
            flags,
            loudness,
            play_fraction: 0xffff,
            external_frequency_modifier: 0x100,
            pitch_lower_bound: 0x100,
            pitch_delta: 0,
            volume_lower_bound: 0x100,
            volume_delta: 0,
            first_subtitle_index: -2,
            subtitle_string_list: None,
            permutations: (0..permutations)
                .map(|i| SoundPermutation {
                    flags: 0,
                    skip_fraction: 0,
                    name: format!("p{i}"),
                    header: SampledSoundHeader {
                        flags: 1,
                        bits_per_sample: 16,
                        format: 1,
                        channels: 1,
                        sample_rate_fixed: 0x5622_0000,
                        packet_count: 0,
                    },
                    adpcm_data: Vec::new(),
                })
                .collect(),
        }
    }

    fn ambient(
        flags: u32,
        inner: i16,
        outer: i16,
        period: (i16, i16),
        continuous: Option<usize>,
        periodic: Option<usize>,
    ) -> AmbientDefinition {
        AmbientDefinition {
            id: FourCC::new(*b"test"),
            tag: AmbientSoundTag {
                flags,
                inner_radius: inner,
                outer_radius: outer,
                period_lower_bound: period.0,
                period_delta: period.1,
                sound_tags: [None; 6],
                random_sound_radius: 0,
            },
            continuous,
            periodic,
            period_counter: 0,
            period: 0,
        }
    }

    fn manager(sounds: Vec<SoundTag>) -> SoundManager {
        let mut m = SoundManager::new(1);
        m.map_size = (96, 96);
        m.sounds = sounds
            .into_iter()
            .map(|tag| SoundDefinition {
                id: FourCC::new(*b"test"),
                tag,
                history: PermutationHistory::default(),
            })
            .collect();
        m
    }

    fn listener_at(x: i32, y: i32, facing: u16) -> SoundListener {
        SoundListener {
            location: WorldPoint::new(x, y, 0),
            facing,
        }
    }

    #[test]
    fn local_random_matches_engine_lcg() {
        let mut r = LocalRandom::new(0);
        assert_eq!(r.next_u16(), 0x3c6e);
        assert_eq!(r.seed, 0x3c6e_f35f);
    }

    mod pre_myth_951 {
        use super::super::WorldPoint;

        const TRIG_TABLE_SIZE: u32 = 1024;

        pub fn trig_entry(index: u32, f: fn(f64) -> f64) -> i32 {
            let a = index as f64 * std::f64::consts::TAU * 0.000_976_562_5;
            let v = f(a);
            if index.is_multiple_of(256) {
                v.round() as i32 * 0x4000
            } else {
                (v * 16384.0 + 0.5) as i32
            }
        }

        pub fn tangent_entry(index: u32) -> i32 {
            let (s, c) = (trig_entry(index, f64::sin), trig_entry(index, f64::cos));
            match (index, c) {
                (0x200, _) => 0,
                (_, 0) => i32::MIN,
                _ => (s << 14) / c,
            }
        }

        pub fn arctangent(x: i32, y: i32) -> u16 {
            if x == 0 {
                return if y < 0 { 0xc000 } else { 0x4000 };
            }
            let t = y.wrapping_shl(14) / x;
            if t == 0 {
                return if x < 0 { 0x8000 } else { 0 };
            }
            let base: u32 = if y > 0 { 0 } else { 512 } + if t < 0 { 256 } else { 0 };
            let tab = |i: u32| tangent_entry((base + i) % TRIG_TABLE_SIZE);
            let (mut lo, mut count) = (1u32, 255u32);
            while count > 0 {
                let step = count >> 1;
                if t > tab(lo + step) {
                    lo += step + 1;
                    count -= step + 1;
                } else {
                    count = step;
                }
            }
            if lo <= 0xff && tab(lo).wrapping_sub(t) > t.wrapping_sub(tab(lo - 1)) {
                lo -= 1;
            }
            (((lo + base) << 6) & 0xffff) as u16
        }

        pub fn distance2d(a: WorldPoint, b: WorldPoint) -> i32 {
            let (dx, dy) = (a.x.wrapping_sub(b.x), a.y.wrapping_sub(b.y));
            let in_range = |v: i32| (-0x7fff..=0x7ffe).contains(&v);
            if !in_range(dx) || !in_range(dy) {
                return 0x7fff;
            }
            let s = (dx * dx) as u32 as u64 + (dy * dy) as u32 as u64;
            let mut r = (s as f64).sqrt() as u64;
            while r * r > s {
                r -= 1;
            }
            while (r + 1) * (r + 1) <= s {
                r += 1;
            }
            if s - r * r > r {
                r += 1;
            }
            (r as i32).min(0x7fff)
        }
    }

    #[test]
    fn engine_tables_match_the_old_trig_entries() {
        for i in 0..1024u32 {
            let k = i as usize;
            assert_eq!(
                pre_myth_951::trig_entry(i, f64::sin),
                i32::from(engine::SINE[k])
            );
            assert_eq!(
                pre_myth_951::trig_entry(i, f64::cos),
                i32::from(engine::COSINE[k])
            );
            assert_eq!(
                pre_myth_951::tangent_entry(i),
                engine::TANGENT[k],
                "tangent {i}"
            );
        }
    }

    #[test]
    fn arctangent_matches_the_old_copy() {
        let check = |x: i32, y: i32| {
            assert_eq!(
                arctangent(x, y),
                pre_myth_951::arctangent(x, y),
                "({x}, {y})"
            );
        };
        for x in -300..=300 {
            for y in -300..=300 {
                check(x, y);
            }
        }
        for i in 0..1024 {
            check(i32::from(engine::COSINE[i]), i32::from(engine::SINE[i]));
        }
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };
        for _ in 0..1_000_000 {
            let (a, b) = (next() as i32, next() as i32);
            check(a, b);
            check(a >> 12, b >> 12);
            check(a >> 20, b >> 20);
        }
        for &x in &[i32::MIN, -1, 0, 1, i32::MAX] {
            for &y in &[i32::MIN, -1, 0, 1, i32::MAX] {
                check(x, y);
            }
        }
    }

    fn check_distance2d(dx: i32, dy: i32) {
        let (p, o) = (WorldPoint::new(dx, dy, 0), WorldPoint::new(0, 0, 0));
        assert_eq!(
            distance2d(p, o),
            pre_myth_951::distance2d(p, o),
            "({dx}, {dy})"
        );
    }

    #[test]
    fn distance2d_matches_the_old_copy() {
        for dx in (-0x7fff..=0x7ffe).step_by(97) {
            for dy in -0x7fff..=0x7ffe {
                check_distance2d(dx, dy);
            }
        }
        for k in -0x7fff..=0x7ffe {
            check_distance2d(k, k);
            check_distance2d(k, -k);
        }
        for &v in &[i32::MIN, -0x8000, -0x7fff, 0x7ffe, 0x7fff, i32::MAX] {
            check_distance2d(v, 0);
            check_distance2d(0, v);
            check_distance2d(v, v);
        }
    }

    #[test]
    #[ignore = "exhaustive, 2^32 pairs"]
    fn distance2d_matches_the_old_copy_everywhere() {
        let threads = std::thread::available_parallelism().map_or(8, |n| n.get()) as i32;
        std::thread::scope(|s| {
            for t in 0..threads {
                s.spawn(move || {
                    for dx in (-0x7fff + t..=0x7ffe).step_by(threads as usize) {
                        for dy in -0x7fff..=0x7ffe {
                            check_distance2d(dx, dy);
                        }
                    }
                });
            }
        });
    }

    #[test]
    fn angles_and_distances() {
        assert_eq!(arctangent(100, 0), 0);
        assert_eq!(arctangent(0, 100), 0x4000);
        assert_eq!(arctangent(-100, 0), 0x8000);
        assert_eq!(arctangent(0, -100), 0xc000);
        assert_eq!(
            distance2d(WorldPoint::new(0, 0, 0), WorldPoint::new(300, 400, 99)),
            500
        );
        assert_eq!(
            distance2d(WorldPoint::new(0, 0, 0), WorldPoint::new(0x8000, 0, 0)),
            0x7fff
        );
        let p = translate_point2d(WorldPoint::new(1000, 1000, 7), 0xc00, 0x4000);
        assert_eq!(p, WorldPoint::new(1000, 1000 + 0xc00, 7));
        let l = SoundListener::from_observer(WorldPoint::new(0, 0, 0), 0);
        assert_eq!(l.location, WorldPoint::new(0xc00, 0, 0));
    }

    #[test]
    fn distance_falloff_matches_table() {
        let class0 = DISTANCE_FALLOFF[0];
        assert_eq!(class0.volume_at(0), 0x100);
        assert_eq!(class0.volume_at(0x1000), 0x100);
        assert_eq!(class0.volume_at(0x2000), 0x80);
        assert_eq!(class0.volume_at(0x3000), 0);
        assert_eq!(class0.volume_at(0x9000), 0);
        assert_eq!(DISTANCE_FALLOFF[2].volume_at(0x7fff), 0x1c);
    }

    #[test]
    fn stereo_pan_follows_engine_convention() {
        let l = listener_at(0, 0, 0);
        assert_eq!(
            stereo_pan(0x100, &l, WorldPoint::new(0, 1000, 0)),
            (0x100, 0x40)
        );
        assert_eq!(
            stereo_pan(0x100, &l, WorldPoint::new(0, -1000, 0)),
            (0x40, 0x100)
        );
        assert_eq!(
            stereo_pan(0x100, &l, WorldPoint::new(1000, 0, 0)),
            (0x100, 0x100)
        );
        assert_eq!(
            stereo_pan(0x100, &l, WorldPoint::new(-1000, 0, 0)),
            (0x100, 0x100)
        );
        assert_eq!(
            stereo_pan(0x100, &l, WorldPoint::new(1000, 1000, 0)),
            (0x100, 0xa0)
        );
        assert_eq!(stereo_pan(0x80, &l, WorldPoint::new(0, 0, 0)), (0x80, 0x80));
    }

    #[test]
    fn spatialize_attenuates_and_pans() {
        let l = listener_at(0, 0, 0);
        let v = spatialize(
            0x100,
            &DISTANCE_FALLOFF[0],
            Some(WorldPoint::new(0, 0x2000, 0)),
            Some(&l),
            false,
        );
        assert_eq!((v.attenuated, v.left, v.right), (0x80, 0x80, 0x20));
        let flat = spatialize(0x90, &DISTANCE_FALLOFF[0], None, Some(&l), false);
        assert_eq!(flat, SoundVolumes::flat(0x90));
        let skip = spatialize(
            0x100,
            &DISTANCE_FALLOFF[0],
            Some(WorldPoint::new(0, 0x9000, 0)),
            Some(&l),
            true,
        );
        assert_eq!(skip.attenuated, 0x100);
    }

    #[test]
    fn continuous_ambient_fades_with_radii_and_takes_a_loop_channel() {
        let mut m = manager(vec![sound(0, 0, 2)]);
        m.ambient_definitions
            .push(ambient(0, 0x2000, 0x3000, (0, 0), Some(0), None));
        m.sources.push(AmbientSoundSource {
            definition: 0,
            location: WorldPoint::new(0x8000, 0x8000, 0),
            volume: FULL_VOLUME,
        });
        let mut b = MockBackend::default();

        let far = listener_at(0x8000 + 0x3000, 0x8000, 0x8000);
        m.update(1, Some(&far), &mut b);
        assert!(b.queued[5].is_empty());

        let mid = listener_at(0x8000 + 0x2800, 0x8000, 0x8000);
        m.update(1, Some(&mid), &mut b);
        assert_eq!(m.channel_sound(5), Some(0));
        assert_eq!(b.queued[5].len(), CHANNEL_QUEUE_DEPTH);
        let (vols, pitch) = b.volumes[5].unwrap();
        assert_eq!(
            (vols.attenuated, vols.left, vols.right, pitch),
            (0x80, 0x80, 0x80, 0x100)
        );

        let near = listener_at(0x8000 + 0x100, 0x8000, 0x8000);
        b.queued[5].pop();
        m.update(1, Some(&near), &mut b);
        assert!(b.stops.is_empty());
        assert_eq!(b.volumes[5].unwrap().0.attenuated, 0x100);
        assert_eq!(b.queued[5].len(), CHANNEL_QUEUE_DEPTH, "refilled");

        m.update(1, Some(&far), &mut b);
        assert_eq!(m.channel_sound(5), None);
        assert!(b.stops.contains(&5));
    }

    #[test]
    fn a_report_reads_the_game_sound_at_its_ambient_index() {
        let mut volumes = vec![
            sound(0, 0, 1),
            sound(0, 0, 1),
            sound(0, 0, 1),
            sound(0, 0, 1),
        ];
        for (tag, lower) in volumes.iter_mut().zip([0x100, 177, 178, 100]) {
            tag.volume_lower_bound = lower;
            tag.volume_delta = 0;
        }
        let mut m = manager(volumes);
        for _ in 0..4 {
            m.ambient_definitions
                .push(ambient(0, 0x1000, 0x2000, (0, 0), Some(0), None));
        }
        m.fire_definition = Some(0);
        m.particle_definition = Some(1);
        m.palette_definitions = vec![3, 2];
        m.stereo_background = Some(3);
        let order = [0, 1, 2, 3].map(|d| m.engine_ambient_index(d));
        assert_eq!(order, [Some(3), Some(1), Some(2), Some(0)]);

        m.game_sounds = vec![1, 2, 3];
        assert_eq!(m.report_sound(3), Some(1));
        assert_eq!(m.report_sound(1), Some(2));
        assert_eq!(m.report_sound(2), Some(3));
        assert_eq!(m.report_sound(0), Some(0), "past the list");
        let l = listener_at(0x4000, 0x4000, 0);
        let got = m.report_volumes(3, Some(l.location), 0x80, Some(&l), true);
        assert_eq!(got.unwrap().attenuated, (177 * 0x80) >> 8);
        let got = m.report_volumes(1, Some(l.location), 43, Some(&l), false);
        assert_eq!(got.unwrap().attenuated, (178 * 43) >> 8);
    }

    #[test]
    fn the_ambient_preference_leaves_only_fire() {
        let mut m = manager(vec![sound(0, 0, 1), sound(0, 1, 1)]);
        for s in [0, 1, 0] {
            m.ambient_definitions
                .push(ambient(0, 0x1000, 0x2000, (0, 0), Some(s), None));
        }
        m.stereo_background = Some(2);
        m.particle_definition = Some(1);
        m.particle_volume = FULL_VOLUME;
        m.sources.push(AmbientSoundSource {
            definition: 0,
            location: WorldPoint::new(0x4000, 0x4000, 0),
            volume: FULL_VOLUME,
        });
        let l = listener_at(0x4000, 0x4000, 0);
        let mut b = MockBackend::default();
        m.update(1, Some(&l), &mut b);
        assert_eq!(m.channel_sound(STEREO_CHANNEL), Some(0));
        assert!(m.channel_sound(5).is_some());

        m.ambient_sound = false;
        m.update(1, Some(&l), &mut b);
        assert_eq!(m.channel_sound(STEREO_CHANNEL), None);
        assert_eq!(m.channel_sound(5), None);
        assert_eq!(m.channel_sound(6), None);

        m.fire_definition = Some(0);
        let fire = |l: &SoundListener, _: &AmbientSoundTag, v: i16| Some((l.location, v));
        m.update_with_fire(1, Some(&l), &mut b, &fire);
        assert_eq!(m.channel_sound(5), Some(0));
    }

    #[test]
    fn only_two_loudest_loops_play() {
        let mut m = manager(vec![sound(0, 0, 1), sound(0, 1, 1), sound(0, 2, 1)]);
        for s in 0..3 {
            m.ambient_definitions
                .push(ambient(0, 0x1000, 0x2000, (0, 0), Some(s), None));
            m.sources.push(AmbientSoundSource {
                definition: s,
                location: WorldPoint::new(0x4000, 0x4000, 0),
                volume: FULL_VOLUME,
            });
        }
        let mut b = MockBackend::default();
        m.update(1, Some(&listener_at(0x4000, 0x4000, 0)), &mut b);
        let mut playing = [m.channel_sound(5).unwrap(), m.channel_sound(6).unwrap()];
        playing.sort();
        assert_eq!(playing, [1, 2], "loudness class 0 loses");
    }

    #[test]
    fn mergeable_sources_share_one_entry() {
        let mut m = manager(vec![sound(0, 0, 1)]);
        m.ambient_definitions
            .push(ambient(0, 0x1000, 0x2000, (0, 0), Some(0), None));
        for x in [0x4000, 0x4800] {
            m.sources.push(AmbientSoundSource {
                definition: 0,
                location: WorldPoint::new(x, 0x4000, 0),
                volume: FULL_VOLUME,
            });
        }
        let mut b = MockBackend::default();
        m.update(1, Some(&listener_at(0x4000, 0x4000, 0)), &mut b);
        assert_eq!(m.channel_sound(5), Some(0));
        assert_eq!(m.channel_sound(6), None);

        m.sounds[0].tag.flags = SOUND_FLAG_INDEPENDENT_INSTANCES;
        m.update(1, Some(&listener_at(0x4000, 0x4000, 0)), &mut b);
        assert_eq!((m.channel_sound(5), m.channel_sound(6)), (Some(0), Some(0)));
    }

    #[test]
    fn periodic_sound_follows_period_timer() {
        let mut m = manager(vec![sound(0, 1, 3)]);
        m.ambient_definitions
            .push(ambient(0, 0x1000, 0x2000, (30, 0), None, Some(0)));
        m.sources.push(AmbientSoundSource {
            definition: 0,
            location: WorldPoint::new(0x4000, 0x4000, 0),
            volume: FULL_VOLUME,
        });
        let l = listener_at(0x4000, 0x4000, 0);
        let mut b = MockBackend::default();
        let mut plays = Vec::new();
        for tick in 1..=100 {
            m.update(1, Some(&l), &mut b);
            if let Some(ch) = (0..ONE_SHOT_CHANNEL_COUNT).find(|&c| !b.queued[c].is_empty()) {
                plays.push(tick);
                b.finish(ch);
            }
        }
        assert_eq!(plays, vec![32, 63, 94]);
        assert!(
            m.channel_sound(5).is_none(),
            "periodic-only sources have no loop"
        );
    }

    #[test]
    fn one_shot_allocation_respects_flags() {
        let mut m = manager(vec![
            sound(SOUND_FLAG_SINGLE_INSTANCE, 1, 2),
            sound(0, 1, 2),
            sound(SOUND_FLAG_INDEPENDENT_INSTANCES, 1, 2),
        ]);
        let l = listener_at(0, 0, 0);
        let mut b = MockBackend::default();
        let at = Some(WorldPoint::new(0x100, 0, 0));
        assert_eq!(
            m.play_sound(0, at, FULL_VOLUME, 0, Some(&l), &mut b),
            Some(0)
        );
        assert_eq!(m.play_sound(0, at, FULL_VOLUME, 0, Some(&l), &mut b), None);
        assert_eq!(
            m.play_sound(1, at, FULL_VOLUME, 0, Some(&l), &mut b),
            Some(1)
        );
        assert_eq!(m.play_sound(1, at, FULL_VOLUME, 0, Some(&l), &mut b), None);
        assert_eq!(
            m.play_sound(2, at, FULL_VOLUME, 0, Some(&l), &mut b),
            Some(2)
        );
        assert_eq!(
            m.play_sound(2, at, FULL_VOLUME, 0, Some(&l), &mut b),
            Some(3)
        );
        b.finish(0);
        m.update(1, Some(&l), &mut b);
        assert_eq!(m.channel_sound(0), None);
        assert_eq!(
            m.play_sound(0, at, FULL_VOLUME, 0, Some(&l), &mut b),
            Some(0)
        );
        let far = Some(WorldPoint::new(0x7000, 0, 0));
        assert_eq!(m.play_sound(1, far, FULL_VOLUME, 0, Some(&l), &mut b), None);
    }

    #[test]
    fn louder_class_steals_a_busy_channel() {
        let mut m = manager(vec![
            sound(SOUND_FLAG_INDEPENDENT_INSTANCES, 0, 1),
            sound(0, 2, 1),
        ]);
        let l = listener_at(0, 0, 0);
        let mut b = MockBackend::default();
        for ch in 0..ONE_SHOT_CHANNEL_COUNT {
            assert_eq!(
                m.play_sound(0, None, FULL_VOLUME, 0, Some(&l), &mut b),
                Some(ch)
            );
        }
        assert_eq!(
            m.play_sound(0, None, FULL_VOLUME, 0, Some(&l), &mut b),
            None
        );
        assert_eq!(
            m.play_sound(1, None, FULL_VOLUME, 0, Some(&l), &mut b),
            Some(0)
        );
        assert_eq!(b.stops, vec![0]);
    }

    #[test]
    fn stereo_background_loops_on_its_channel() {
        let mut m = manager(vec![sound(0, 1, 4)]);
        m.sounds[0].tag.volume_lower_bound = 0x131;
        m.ambient_definitions
            .push(ambient(0, 0x1800, 0x2800, (150, 150), Some(0), None));
        m.stereo_background = Some(0);
        let l = listener_at(0, 0, 0);
        let mut b = MockBackend::default();
        m.update(1, Some(&l), &mut b);
        assert_eq!(m.channel_sound(STEREO_CHANNEL), Some(0));
        let (v, _) = b.volumes[STEREO_CHANNEL].unwrap();
        let expected = ((((0x131 * 0x100) >> 8) * 0x131) >> 8) as i16;
        assert_eq!(
            (v.left, v.right, v.attenuated),
            (expected, expected, expected)
        );
        assert_eq!(b.queued[STEREO_CHANNEL].len(), CHANNEL_QUEUE_DEPTH);
        m.update(1, None, &mut b);
        assert_eq!(m.channel_sound(STEREO_CHANNEL), None);
    }

    #[test]
    fn paused_frames_do_nothing() {
        let mut m = manager(vec![sound(0, 1, 1)]);
        m.ambient_definitions
            .push(ambient(0, 0x1800, 0x2800, (0, 0), Some(0), None));
        m.stereo_background = Some(0);
        let mut b = MockBackend::default();
        m.update(0, Some(&listener_at(0, 0, 0)), &mut b);
        assert_eq!(m.channel_sound(STEREO_CHANNEL), None);
    }

    #[test]
    fn a_stopped_channel_is_free_and_off_keeps_the_starting_gain() {
        let at = WorldPoint::new(0, 0, 0);
        let near = SoundListener {
            location: WorldPoint::new(0x800, 0, 0),
            facing: 0,
        };
        let far = SoundListener {
            location: WorldPoint::new(0x4000, 0, 0),
            facing: 0,
        };
        let mut m = manager(vec![sound(0, 0, 1)]);
        let mut b = MockBackend::default();
        let c = m
            .play_sound(0, Some(at), FULL_VOLUME, 0, Some(&near), &mut b)
            .unwrap();
        m.update(1, Some(&far), &mut b);
        assert_eq!(m.one_shot(c).unwrap().attenuated, FULL_VOLUME);
        m.sound_3d = true;
        m.update(1, Some(&far), &mut b);
        assert_eq!(m.one_shot(c), None);
        assert_eq!(b.stops, vec![c]);
        assert_eq!(b.channel_status(c), ChannelStatus::Idle);
        let c = m
            .play_sound(0, None, FULL_VOLUME, 0, Some(&near), &mut b)
            .unwrap();
        m.update(1, Some(&far), &mut b);
        assert_eq!(m.one_shot(c).unwrap().attenuated, FULL_VOLUME);
    }

    #[test]
    fn the_device_gets_each_source_with_3d_sound_on_only() {
        let object = ChannelObject {
            index: 7,
            identifier: -3,
        };
        let l = SoundListener {
            location: WorldPoint::new(0, 0, 0),
            facing: 0,
        };
        let start = WorldPoint::new(0x100, 0, 0);
        let later = WorldPoint::new(0x100 + 12, -12, 0);
        for on in [false, true] {
            let mut m = manager(vec![sound(0, 0, 1), sound(0, 0, 1)]);
            m.sound_3d = on;
            let mut b = MockBackend::default();
            let c = m
                .play_object_sound(
                    0,
                    Some(start),
                    Some(object),
                    [5, -6, 1],
                    FULL_VOLUME,
                    0,
                    Some(&l),
                    &mut b,
                )
                .unwrap();
            let flat = m
                .play_sound(1, None, FULL_VOLUME, 0, Some(&l), &mut b)
                .unwrap();
            m.follow_objects(|_| ObjectPlace::At(later, [11, -13, 2]), &mut b);
            m.update(1, Some(&l), &mut b);
            if !on {
                assert!(b.sources.is_empty());
                continue;
            }
            let at = |location, velocity| DeviceSource { location, velocity };
            assert_eq!(b.sources[0], (c, at(Some(start), [5, -6, 1])));
            assert_eq!(b.sources[1], (flat, at(None, [0; 3])));
            assert_eq!(
                *b.sources.last().unwrap(),
                (c, at(Some(later), [11, -13, 2]))
            );
            assert_eq!(
                b.volumes[flat].unwrap().0,
                SoundVolumes::flat(b.volumes[flat].unwrap().0.volume)
            );
        }
    }

    #[test]
    fn a_play_keeps_its_own_velocity_until_an_update_copies_the_objects() {
        let object = ChannelObject {
            index: 7,
            identifier: -3,
        };
        let l = SoundListener {
            location: WorldPoint::new(0, 0, 0),
            facing: 0,
        };
        let start = WorldPoint::new(0x100, 0, 0);
        let later = WorldPoint::new(0x110, 0, 0);
        let at = |location, velocity| DeviceSource {
            location: Some(location),
            velocity,
        };
        for (flags, place, want) in [
            (
                REQUEST_FLAG_DETACHED,
                ObjectPlace::At(later, [9, 9, 9]),
                at(start, [5, -6, 1]),
            ),
            (0, ObjectPlace::Deleted, at(start, [5, -6, 1])),
            (0, ObjectPlace::At(later, [9, 9, 9]), at(later, [9, 9, 9])),
        ] {
            let mut m = manager(vec![sound(0, 0, 1)]);
            m.sound_3d = true;
            let mut b = MockBackend::default();
            let c = m
                .play_object_sound(
                    0,
                    Some(start),
                    Some(object),
                    [5, -6, 1],
                    FULL_VOLUME,
                    flags,
                    Some(&l),
                    &mut b,
                )
                .unwrap();
            m.follow_objects(|_| place, &mut b);
            m.update(1, Some(&l), &mut b);
            assert_eq!(*b.sources.last().unwrap(), (c, want), "{flags} {place:?}");
        }
        let mut m = manager(vec![sound(0, 0, 1)]);
        m.sound_3d = true;
        let mut b = MockBackend::default();
        let c = m.play_object_sound(
            0,
            None,
            Some(object),
            [5, -6, 1],
            FULL_VOLUME,
            0,
            Some(&l),
            &mut b,
        );
        let c = c.unwrap();
        assert_eq!(
            b.sources[0],
            (
                c,
                DeviceSource {
                    location: None,
                    velocity: [0; 3]
                }
            )
        );
    }

    #[test]
    fn the_reverb_preset_is_the_meshs_or_19() {
        assert_eq!(reverb_preset(0), 19);
        assert_eq!(reverb_preset(-1), 19);
        assert_eq!(reverb_preset(1), 1);
        assert_eq!(reverb_preset(21), 21);
        assert_eq!(reverb_preset(22), 19);
    }

    #[test]
    fn orphan_keeps_the_last_place_or_ends_with_the_flag() {
        let object = ChannelObject {
            index: 7,
            identifier: -3,
        };
        let l = SoundListener {
            location: WorldPoint::new(0, 0, 0),
            facing: 0,
        };
        let start = WorldPoint::new(0x100, 0, 0);
        let later = WorldPoint::new(0x2000, 0, 0);
        for (flags, ends) in [(0, false), (SOUND_FLAG_ENDS_WITH_OBJECT, true)] {
            let mut m = manager(vec![sound(flags, 0, 1)]);
            m.sound_3d = true;
            let mut b = MockBackend::default();
            let c = m
                .play_object_sound(
                    0,
                    Some(start),
                    Some(object),
                    [0; 3],
                    FULL_VOLUME,
                    0,
                    Some(&l),
                    &mut b,
                )
                .unwrap();
            m.follow_objects(
                |o| {
                    assert_eq!(o, object);
                    ObjectPlace::At(later, [0; 3])
                },
                &mut b,
            );
            m.update(1, Some(&l), &mut b);
            assert_eq!(m.one_shot(c).unwrap().location, Some(later));
            let gain = m.one_shot(c).unwrap().attenuated;
            assert!(gain < FULL_VOLUME);
            m.follow_objects(|_| ObjectPlace::Unknown, &mut b);
            assert_eq!(m.one_shot(c).unwrap().object, Some(object));
            m.follow_objects(|_| ObjectPlace::Deleted, &mut b);
            if ends {
                assert_eq!(m.one_shot(c), None);
            } else {
                let kept = m.one_shot(c).unwrap();
                assert_eq!((kept.object, kept.location), (None, Some(later)));
                m.follow_objects(|_| panic!("no object is left"), &mut b);
                m.update(1, Some(&l), &mut b);
                assert_eq!(m.one_shot(c).unwrap().attenuated, gain);
            }
        }
    }

    #[test]
    fn detached_request_does_not_follow() {
        let l = SoundListener {
            location: WorldPoint::new(0, 0, 0),
            facing: 0,
        };
        let mut m = manager(vec![sound(0, 0, 1)]);
        let mut b = MockBackend::default();
        let object = ChannelObject {
            index: 1,
            identifier: -1,
        };
        let c = m
            .play_object_sound(
                0,
                Some(WorldPoint::new(1, 0, 0)),
                Some(object),
                [0; 3],
                FULL_VOLUME,
                REQUEST_FLAG_DETACHED,
                Some(&l),
                &mut b,
            )
            .unwrap();
        assert_eq!(m.one_shot(c).unwrap().object, None);
    }

    #[test]
    fn device_gain_is_the_backends_curve() {
        assert_eq!(device_gain(0), 0.0);
        assert_eq!(device_gain(FULL_VOLUME), 1.0);
        assert_eq!(device_gain(0x400), 1.0);
        assert_eq!(device_gain(-5), 0.0);
        let half = device_gain(0x80);
        assert!((half - 0.5719).abs() < 0.001, "{half}");
        for (attenuated, gain) in [
            (1, 0.024803),
            (3, 0.04626),
            (113, 0.519441),
            (153, 0.658193),
        ] {
            assert!(
                (device_gain(attenuated) - gain).abs() < 2e-4,
                "{attenuated}"
            );
        }
    }

    #[test]
    fn the_preference_gives_its_flags() {
        let mut file = vec![0u8; 76];
        file[0x24..0x2c].copy_from_slice(b"prefsoun");
        file[0x38..0x3a].copy_from_slice(&[0, 1]);
        file[0x40..0x42].copy_from_slice(&[0, 3]);
        assert_eq!(sound_preference_flags(&file), Some(3));
        file[0x39] = 2;
        assert_eq!(sound_preference_flags(&file), None);
        assert_eq!(sound_preference_flags(&[0; 8]), None);
    }
}
