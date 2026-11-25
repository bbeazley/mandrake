use std::collections::HashMap;
use std::sync::Arc;

use mandrake_audio::dsound::DsoundChannel;
use mandrake_audio::{
    ChannelSpace, EngineChannelStatus, Listener3d, Place3d, REVERB_PRESETS, SharedAudio,
    SoundBuffer,
};
use mandrake_common::FourCC;
use mandrake_sim::CombatEvent;
use mandrake_sim::combat::SequenceSound;
use mandrake_sim::engine_rng::EngineWorld;
use mandrake_sim::map_actions::{MapActions, SoundActionPlay, SoundPlace, TYPE_TAG};
use mandrake_sim::projectile_groups::ProjectileCatalog;
use mandrake_tag::sound_manager::{
    AMBIENT_REPORT_HALF_EXTENT, CHANNEL_COUNT, ChannelObject, DeviceSource, FULL_VOLUME,
    ONE_SHOT_CHANNEL_COUNT, REQUEST_FLAG_IGNORE_MEDIA, SOUND_PREFERENCE_3D,
    SOUND_PREFERENCE_AMBIENT, WORLD_ONE, archive_resolver, device_gain, distance2d, reverb_preset,
};
use mandrake_tag::{
    AmbientSoundTag, ChannelStatus, MONSTER_SOUND_SLOTS, MeshTag, MonsterSoundType,
    SoundChannelBackend, SoundListener, SoundManager, SoundVolumes, TagArchive, UnitTag,
    WorldPoint, monster_group_sound,
};
use mandrake_view::camera::Camera;

const FIRE_AMBIENT: FourCC = FourCC::new(*b"amfi");
const FIRE_SOUND_LIST: usize = 0x40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FireSoundCell {
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub fading: Option<(i16, i16)>,
}

pub fn fire_sound_cells(world: &EngineWorld) -> Vec<FireSoundCell> {
    if !world.fire.cells.iter().any(Option::is_some) {
        return Vec::new();
    }
    let mut flame_by_cell: HashMap<(i32, i32), Option<(i16, i16)>> = HashMap::new();
    for g in world.fire.groups.iter().flatten() {
        if g.flame {
            let cell = (g.point[0] >> 9, g.point[1] >> 9);
            flame_by_cell
                .entry(cell)
                .or_insert_with(|| (g.state == 2).then_some((g.fade_out, g.ticks)));
        }
    }
    world
        .fire
        .cells
        .iter()
        .flatten()
        .map(|c| {
            let (cx, cy) = (i32::from(c.x), i32::from(c.y));
            let (x, y) = (cx * 0x200 + 0x100, cy * 0x200 + 0x100);
            FireSoundCell {
                x: c.x,
                y: c.y,
                z: world.mesh_height(x, y, mandrake_tag::MeshHeightMode::Physical) as i16,
                fading: flame_by_cell.get(&(cx, cy)).copied().flatten(),
            }
        })
        .collect()
}

pub fn report_fire_as_sound_source(
    cells: &[FireSoundCell],
    listener: &SoundListener,
    amfi: &AmbientSoundTag,
    volume: i16,
) -> Option<(WorldPoint, i16)> {
    let (lx, ly) = (listener.location.x, listener.location.y);
    let (inner, outer) = (i32::from(amfi.inner_radius), i32::from(amfi.outer_radius));
    let mut list: Vec<(WorldPoint, i16)> = Vec::new();
    for cell in cells {
        let (x, y) = (
            i32::from(cell.x) * 0x200 + 0x100,
            i32::from(cell.y) * 0x200 + 0x100,
        );
        if x < lx - AMBIENT_REPORT_HALF_EXTENT
            || x > lx + AMBIENT_REPORT_HALF_EXTENT
            || y < ly - AMBIENT_REPORT_HALF_EXTENT
            || y > ly + AMBIENT_REPORT_HALF_EXTENT
        {
            continue;
        }
        let point = WorldPoint::new(x, y, i32::from(cell.z));
        let distance = distance2d(listener.location, point);
        if distance >= outer {
            continue;
        }
        let mut v = volume;
        if inner <= distance && outer - inner > 0 {
            v = ((outer - distance) * i32::from(volume) / (outer - inner)) as i16;
        }
        if v == 0 {
            continue;
        }
        if let Some((fade_out, ticks)) = cell.fading {
            let mut scale = 0x100;
            if fade_out != 0 {
                scale -= i32::from(((i32::from(ticks) << 8) / i32::from(fade_out)) as i16);
            }
            v = ((i32::from(v) * scale) >> 8) as i16;
            if v == 0 {
                continue;
            }
        }
        if list.len() < FIRE_SOUND_LIST {
            list.push((point, v));
        }
    }
    mandrake_sim::formations::heap_sort(&mut list, |a, b| i32::from(b.1) - i32::from(a.1));
    list.first().copied()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterVoices {
    pub definition: FourCC,
    pub sounds: [Option<usize>; MONSTER_SOUND_SLOTS],
    pub types: [i8; MONSTER_SOUND_SLOTS],
}

impl MonsterVoices {
    pub fn sound_for(&self, kind: MonsterSoundType) -> Option<usize> {
        let slot = self.types.iter().position(|&t| t == kind.0)?;
        self.sounds[slot]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundRequest {
    pub sound: FourCC,
    pub world: [i32; 3],
    pub flags: u16,
    pub object: Option<ChannelObject>,
    pub velocity: [i16; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionRequest {
    pub sound: FourCC,
    pub world: Option<[i32; 3]>,
    pub flags: u16,
    pub object: Option<ChannelObject>,
    pub velocity: [i16; 3],
}

pub fn action_sound_tags(actions: &MapActions) -> Vec<FourCC> {
    let proc_tag = u32::from_be_bytes(*b"soun");
    let mut all: Vec<FourCC> = (0..actions.actions.len())
        .filter(|&i| actions.actions[i].tag == proc_tag)
        .filter_map(|i| actions.indexed(i, b"soun", TYPE_TAG, 0))
        .filter(|&id| id != -1)
        .map(|id| FourCC((id as u32).to_be_bytes()))
        .collect();
    all.sort_unstable_by_key(|s| s.as_u32());
    all.dedup();
    all
}

pub fn sound_can_see(visible: &[u32], mesh: (i32, i32), x: i32, y: i32) -> bool {
    crate::minimap::visible_cell_bits(visible, mesh, x >> 9, y >> 9) & 2 != 0
}

pub fn sequence_sound_gate(
    visible: &[u32],
    mesh: (i32, i32),
    options: u32,
    x: i32,
    y: i32,
) -> bool {
    if options & mandrake_common::game_options::TFL != 0 {
        return true;
    }
    crate::minimap::visible_cell_bits(visible, mesh, x >> 9, y >> 9) != 0
}

fn monster_location(world: &EngineWorld, m: usize) -> [i32; 3] {
    let (x, y) = world
        .movers
        .get(m)
        .and_then(Option::as_ref)
        .map(|v| (v.x, v.y))
        .unwrap_or(world.monsters[m].position);
    [x, y, world.monsters[m].object.z]
}

pub fn resolve_nearest_visible(
    plays: &mut [SoundActionPlay],
    world: &EngineWorld,
    visible: Option<&[u32]>,
    listener: Option<&SoundListener>,
) {
    for play in plays {
        let SoundPlace::NearestVisible(subjects) = &mut play.place else {
            continue;
        };
        let Some(visible) = visible else {
            tracing::warn!(
                "tick {}: no visibility buffers, soun {:#x} at a subject is not played",
                play.time,
                play.sound
            );
            subjects.clear();
            continue;
        };
        let mut best: Option<(usize, i32)> = None;
        for &m in subjects.iter() {
            let at = monster_location(world, m);
            if !sound_can_see(visible, world.mesh_cells, at[0], at[1]) {
                continue;
            }
            let distance = listener.map_or(0, |l| {
                mandrake_sim::explosions::distance3d(at, [l.location.x, l.location.y, l.location.z])
            });
            if best.is_none_or(|(_, d)| distance < d) {
                best = Some((m, distance));
            }
        }
        *subjects = best.map(|(m, _)| m).into_iter().collect();
    }
}

pub fn action_request(play: &SoundActionPlay, world: &EngineWorld) -> Option<ActionRequest> {
    if play.sound == u32::MAX {
        return None;
    }
    let sound = FourCC(play.sound.to_be_bytes());
    match &play.place {
        SoundPlace::Flat => Some(ActionRequest {
            sound,
            world: None,
            flags: 0,
            object: None,
            velocity: [0; 3],
        }),
        SoundPlace::NearestVisible(subjects) => {
            let m = *subjects.first()?;
            Some(ActionRequest {
                sound,
                world: Some(monster_location(world, m)),
                flags: u16::from(world.monsters[m].object.flags & 0x80 == 0),
                object: Some(ChannelObject {
                    index: world.monsters[m].object.index,
                    identifier: world.monsters[m].object.identifier,
                }),
                velocity: world.monsters[m].object.velocity,
            })
        }
    }
}

fn channel_object(index: i32, identifier: i16) -> Option<ChannelObject> {
    (index >= 0).then_some(ChannelObject { index, identifier })
}

#[derive(Debug, Clone, Default)]
pub struct SimSoundTags {
    flight: HashMap<FourCC, FourCC>,
    groups: Vec<FourCC>,
    sequences: HashMap<FourCC, Vec<[Option<FourCC>; 3]>>,
}

pub fn sequence_sound_tags(collection: &[u8]) -> Option<Vec<[Option<FourCC>; 3]>> {
    const HEADER: usize = 320;
    let be32 = |o: usize| -> Option<usize> {
        let b: [u8; 4] = collection.get(o..o + 4)?.try_into().ok()?;
        Some(i32::from_be_bytes(b).max(0) as usize)
    };
    let tag = |o: usize| -> Option<Option<FourCC>> {
        let b: [u8; 4] = collection.get(o..o + 4)?.try_into().ok()?;
        Some((u32::from_be_bytes(b) != u32::MAX).then_some(FourCC::new(b)))
    };
    let (count, refs) = (be32(0x80)?, be32(0x84)?);
    (0..count)
        .map(|i| {
            let at = HEADER + be32(HEADER + refs + i * 128 + 64)?;
            Some([tag(at + 0x18)?, tag(at + 0x1c)?, tag(at + 0x20)?])
        })
        .collect()
}

impl SimSoundTags {
    pub fn from_catalog(catalog: &ProjectileCatalog) -> Self {
        let mut tags = Self::default();
        for p in catalog.projectiles.values() {
            if let Some(s) = p.flight_sound {
                tags.flight.insert(p.tag_id, s);
            }
        }
        tags.groups = catalog.groups.values().filter_map(|g| g.sound).collect();
        tags
    }

    pub fn add_collection(&mut self, id: FourCC, payload: impl FnOnce() -> Option<Vec<u8>>) {
        if !self.sequences.contains_key(&id)
            && let Some(sounds) = payload().as_deref().and_then(sequence_sound_tags)
        {
            self.sequences.insert(id, sounds);
        }
    }

    fn all(&self, catalog: &ProjectileCatalog) -> Vec<FourCC> {
        let mut all: Vec<FourCC> = self
            .flight
            .values()
            .chain(self.groups.iter())
            .copied()
            .chain(
                self.sequences
                    .values()
                    .flatten()
                    .flatten()
                    .flatten()
                    .copied(),
            )
            .chain(catalog.projectiles.values().filter_map(|p| p.rebound_sound))
            .collect();
        all.sort_unstable_by_key(|s| s.as_u32());
        all.dedup();
        all
    }

    pub fn event_request(&self, event: &CombatEvent) -> Option<SoundRequest> {
        match event {
            CombatEvent::ProjectileCreated {
                definition,
                point,
                object,
                identifier,
                velocity,
                ..
            } => Some(SoundRequest {
                sound: *self.flight.get(definition)?,
                world: *point,
                flags: 0,
                object: channel_object(*object, *identifier),
                velocity: *velocity,
            }),
            CombatEvent::ProjectileGroupSound {
                sound,
                world,
                request_flags,
                object,
                identifier,
                velocity,
                ..
            }
            | CombatEvent::MonsterVoice {
                sound,
                world,
                request_flags,
                object,
                identifier,
                velocity,
                ..
            }
            | CombatEvent::ProjectileRebounded {
                sound: Some(sound),
                world,
                request_flags,
                object,
                identifier,
                velocity,
                ..
            } => Some(SoundRequest {
                sound: *sound,
                world: *world,
                flags: *request_flags,
                object: channel_object(*object, *identifier),
                velocity: *velocity,
            }),
            _ => None,
        }
    }

    pub fn sequence_requests(&self, s: &SequenceSound) -> Vec<SoundRequest> {
        let Some(slots) = self
            .sequences
            .get(&s.collection)
            .and_then(|q| q.get(usize::from(s.sounds.sequence)).or_else(|| q.first()))
        else {
            return Vec::new();
        };
        [
            (s.sounds.first, slots[0]),
            (s.sounds.last, slots[2]),
            (s.sounds.key, slots[1]),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .filter_map(|(_, sound)| {
            Some(SoundRequest {
                sound: sound?,
                world: s.world,
                flags: s.request_flags,
                object: channel_object(s.object, s.identifier),
                velocity: s.velocity,
            })
        })
        .collect()
    }
}

pub struct MapSound {
    pub manager: SoundManager,
    pcm: Vec<Vec<Option<SoundBuffer>>>,
    resampled: HashMap<(usize, usize), Arc<SoundBuffer>>,
    pitch: [i16; CHANNEL_COUNT],
    sources: [DeviceSource; CHANNEL_COUNT],
    pub mesh_reverb: Option<i32>,
    reverb_on: Option<bool>,
    half_extent: (f32, f32),
    observer_distance: f32,
    pending: Vec<(usize, WorldPoint)>,
    fire_cells: Vec<FireSoundCell>,
    monster_voices: HashMap<FourCC, MonsterVoices>,
    sim_tags: SimSoundTags,
    pub game_sounds: GameSounds,
    pub game_sound_tags: Vec<Option<FourCC>>,
    dsound: Option<[DsoundChannel; CHANNEL_COUNT]>,
}

impl MapSound {
    fn from_parts(
        manager: SoundManager,
        mesh_reverb: Option<i32>,
        half_extent: (f32, f32),
        observer_distance: f32,
    ) -> Self {
        let mut sound = Self {
            manager,
            pcm: Vec::new(),
            resampled: HashMap::new(),
            pitch: [FULL_VOLUME; CHANNEL_COUNT],
            sources: [DeviceSource::default(); CHANNEL_COUNT],
            mesh_reverb,
            reverb_on: None,
            half_extent,
            observer_distance,
            pending: Vec::new(),
            fire_cells: Vec::new(),
            monster_voices: HashMap::new(),
            sim_tags: SimSoundTags::default(),
            game_sounds: GameSounds::default(),
            game_sound_tags: Vec::new(),
            dsound: None,
        };
        sound.decode_new_sounds();
        sound
    }

    fn with_backend<R>(
        &mut self,
        audio: &SharedAudio,
        f: impl FnOnce(&mut SoundManager, &mut NativeBackend<'_>) -> R,
    ) -> R {
        let mut backend = NativeBackend {
            audio,
            pcm: &self.pcm,
            resampled: &mut self.resampled,
            pitch: &mut self.pitch,
            sources: &mut self.sources,
            sound_3d: self.manager.sound_3d,
            dsound: self.dsound.as_mut(),
        };
        f(&mut self.manager, &mut backend)
    }

    pub fn load(
        mesh: &MeshTag,
        stereo_background: Option<FourCC>,
        archives: &[&TagArchive],
        observer_distance: f32,
        seed: u32,
    ) -> Self {
        let mut resolver = archive_resolver(archives);
        let mut manager = SoundManager::for_map(mesh, stereo_background, &mut resolver, seed);
        manager.fire_definition = manager.load_ambient_definition(FIRE_AMBIENT, &mut resolver);
        let media_mesh = mesh.clone();
        manager.set_media_query(move |p| point_is_under_media(&media_mesh, p));
        Self::from_parts(
            manager,
            mesh.reverb,
            (mesh.cell_width as f32 * 0.5, mesh.cell_height as f32 * 0.5),
            observer_distance,
        )
    }

    pub fn use_tfl_device(&mut self) {
        self.dsound = Some([DsoundChannel::default(); CHANNEL_COUNT]);
    }

    pub fn with_manager(manager: SoundManager) -> Self {
        Self::from_parts(manager, None, (0.0, 0.0), 0.0)
    }

    pub fn load_sound_with(
        &mut self,
        id: FourCC,
        resolver: &mut mandrake_tag::sound_manager::TagResolver<'_>,
    ) -> Option<usize> {
        let index = self.manager.load_sound_definition(id, resolver);
        self.decode_new_sounds();
        index
    }

    pub fn load_ambient_with(
        &mut self,
        id: FourCC,
        resolver: &mut mandrake_tag::sound_manager::TagResolver<'_>,
    ) -> Option<usize> {
        let index = self.manager.load_ambient_definition(id, resolver);
        self.decode_new_sounds();
        index
    }

    pub fn play_object(
        &mut self,
        sound: usize,
        location: Option<WorldPoint>,
        object: Option<ChannelObject>,
        velocity: [i16; 3],
        listener: &SoundListener,
        audio: &SharedAudio,
    ) -> Option<usize> {
        self.play_at(sound, location, object, velocity, 0, listener, audio)
    }

    pub fn follow_with(
        &mut self,
        place: impl Fn(ChannelObject) -> mandrake_tag::sound_manager::ObjectPlace,
        audio: &SharedAudio,
    ) {
        self.with_backend(audio, |manager, backend| {
            manager.follow_objects(place, backend);
        });
    }

    pub fn sound_is_playing(&self, index: usize) -> bool {
        (0..mandrake_tag::sound_manager::CHANNEL_COUNT)
            .any(|c| self.manager.channel_sound(c) == Some(index))
    }

    pub fn play_game_sound(
        &mut self,
        index: usize,
        listener: &SoundListener,
        audio: &SharedAudio,
    ) -> Option<usize> {
        self.play_at(index, None, None, [0; 3], 0x100, listener, audio)
    }

    pub fn load_game_sounds(&mut self, list: &[u8], archives: &[&TagArchive]) {
        self.game_sound_tags = list
            .get(0x10..)
            .unwrap_or_default()
            .as_chunks::<8>()
            .0
            .iter()
            .map(|e| {
                let tag: [u8; 4] = [e[0], e[1], e[2], e[3]];
                (tag != [0xff; 4] && tag != [0; 4]).then_some(FourCC(tag))
            })
            .collect();
        let mut order = Vec::with_capacity(self.game_sound_tags.len());
        for id in self.game_sound_tags.clone().into_iter().flatten() {
            if let Some(index) = self.load_sound(id, archives)
                && !order.contains(&index)
            {
                order.push(index);
            }
        }
        self.manager.game_sounds = order;
    }

    pub fn load_sound(&mut self, id: FourCC, archives: &[&TagArchive]) -> Option<usize> {
        let mut resolver = archive_resolver(archives);
        let index = self.manager.load_sound_definition(id, &mut resolver);
        self.decode_new_sounds();
        index
    }

    pub fn load_monster_voices(
        &mut self,
        unit_tag: FourCC,
        unit: &UnitTag,
        archives: &[&TagArchive],
    ) -> Vec<FourCC> {
        let mut missing = Vec::new();
        let mut sounds = [None; MONSTER_SOUND_SLOTS];
        for (slot, id) in sounds.iter_mut().zip(unit.sounds) {
            let Some(id) = id else { continue };
            *slot = self.load_sound(id, archives);
            if slot.is_none() {
                missing.push(id);
            }
        }
        self.monster_voices.insert(
            unit_tag,
            MonsterVoices {
                definition: unit.monster_tag_id,
                sounds,
                types: unit.sound_types,
            },
        );
        missing
    }

    pub fn load_sim_sounds(
        &mut self,
        tags: SimSoundTags,
        catalog: &ProjectileCatalog,
        archives: &[&TagArchive],
    ) -> Vec<FourCC> {
        let mut tags = tags;
        let mut sequences = std::mem::take(&mut tags.sequences);
        for (id, sounds) in &mut sequences {
            self.resolve_sequence_sounds(*id, sounds, archives);
        }
        let mut missing = Vec::new();
        for id in tags.all(catalog) {
            if self.load_sound(id, archives).is_none() {
                missing.push(id);
            }
        }
        tags.sequences = sequences;
        self.sim_tags = tags;
        missing
    }

    pub fn load_sequence_sounds(&mut self, id: FourCC, archives: &[&TagArchive]) {
        if self.sim_tags.sequences.contains_key(&id) {
            return;
        }
        let mut tags = std::mem::take(&mut self.sim_tags);
        tags.add_collection(id, || {
            archives.iter().find_map(|a| {
                let e = a.find_tag_by_id(FourCC::new(*b".256"), id)?;
                a.get_tag_data(e).ok()
            })
        });
        if let Some(mut sounds) = tags.sequences.remove(&id) {
            self.resolve_sequence_sounds(id, &mut sounds, archives);
            tags.sequences.insert(id, sounds);
        } else {
            tags.sequences.insert(id, Vec::new());
        }
        self.sim_tags = tags;
    }

    fn resolve_sequence_sounds(
        &mut self,
        id: FourCC,
        sounds: &mut [[Option<FourCC>; 3]],
        archives: &[&TagArchive],
    ) {
        for (sequence, slots) in sounds.iter_mut().enumerate() {
            for slot in slots.iter_mut() {
                let Some(sound) = *slot else { continue };
                if self.load_sound(sound, archives).is_none() {
                    tracing::info!(
                        "failed to load sequence sound {id}:{sequence}:'{sound}' (in no tag file)"
                    );
                    *slot = None;
                }
            }
        }
    }

    pub fn load_action_sounds(
        &mut self,
        actions: &MapActions,
        archives: &[&TagArchive],
    ) -> Vec<FourCC> {
        action_sound_tags(actions)
            .into_iter()
            .filter(|id| self.load_sound(*id, archives).is_none())
            .collect()
    }

    pub fn play_action_sounds(
        &mut self,
        plays: &[SoundActionPlay],
        world: &EngineWorld,
        listener: &SoundListener,
        audio: &SharedAudio,
    ) -> Vec<(FourCC, Option<usize>)> {
        let mut started = Vec::new();
        for play in plays {
            let Some(r) = action_request(play, world) else {
                continue;
            };
            let Some(index) = self.manager.sound_index(r.sound) else {
                tracing::warn!("tick {}: sound {} is not loaded", play.time, r.sound);
                continue;
            };
            let at = r.world.map(|w| WorldPoint::new(w[0], w[1], w[2]));
            let channel = self.play_at(index, at, r.object, r.velocity, r.flags, listener, audio);
            tracing::info!(
                "tick {}: map action {} sound {} at {:?}, listener {:?}, channel {channel:?}",
                play.time,
                play.action,
                r.sound,
                r.world,
                listener.location
            );
            started.push((r.sound, channel));
        }
        started
    }

    pub fn play_sequence_sounds(
        &mut self,
        sequences: &[SequenceSound],
        archives: &[&TagArchive],
        listener: &SoundListener,
        audio: &SharedAudio,
    ) -> Vec<(u32, FourCC, usize)> {
        for s in sequences {
            self.load_sequence_sounds(s.collection, archives);
        }
        let mut started = Vec::new();
        for s in sequences {
            for r in self.sim_tags.sequence_requests(s) {
                let Some(index) = self.manager.sound_index(r.sound) else {
                    tracing::warn!("tick {}: sound {} is not loaded", s.tick, r.sound);
                    continue;
                };
                let at = WorldPoint::new(r.world[0], r.world[1], r.world[2]);
                let channel = self.play_at(
                    index,
                    Some(at),
                    r.object,
                    r.velocity,
                    r.flags,
                    listener,
                    audio,
                );
                if let Some(channel) = channel {
                    started.push((s.tick, r.sound, channel));
                }
            }
        }
        started
    }

    pub fn apply_sound_preferences(&mut self, flags: u16) {
        self.manager.sound_3d = flags & SOUND_PREFERENCE_3D != 0;
        self.manager.ambient_sound = flags & SOUND_PREFERENCE_AMBIENT != 0;
    }

    pub fn play_sim_sounds(
        &mut self,
        events: &[CombatEvent],
        sequences: &[SequenceSound],
        listener: &SoundListener,
        audio: &SharedAudio,
    ) {
        let mut requests: Vec<(u32, SoundRequest)> = events
            .iter()
            .filter_map(|e| Some((e.tick(), self.sim_tags.event_request(e)?)))
            .collect();
        for s in sequences {
            requests.extend(
                self.sim_tags
                    .sequence_requests(s)
                    .into_iter()
                    .map(|r| (s.tick, r)),
            );
        }
        for (tick, r) in requests {
            let Some(index) = self.manager.sound_index(r.sound) else {
                tracing::warn!("tick {tick}: sound {} is not loaded", r.sound);
                continue;
            };
            let at = WorldPoint::new(r.world[0], r.world[1], r.world[2]);
            let channel = self.play_at(
                index,
                Some(at),
                r.object,
                r.velocity,
                r.flags,
                listener,
                audio,
            );
            tracing::debug!(
                "tick {tick}: sound {} at {:?}, listener {:?}, channel {channel:?}",
                r.sound,
                r.world,
                listener.location
            );
        }
    }

    pub fn play_monsters_sound(
        &mut self,
        monsters: &[(FourCC, WorldPoint, Option<ChannelObject>, [i16; 3])],
        kind: MonsterSoundType,
        listener: &SoundListener,
        audio: &SharedAudio,
    ) -> Vec<usize> {
        let monsters: Vec<(MonsterVoices, WorldPoint, Option<ChannelObject>, [i16; 3])> = monsters
            .iter()
            .filter_map(|(tag, at, o, v)| self.monster_voices.get(tag).map(|m| (*m, *at, *o, *v)))
            .collect();
        let definitions: Vec<FourCC> = monsters.iter().map(|(v, ..)| v.definition).collect();
        let (kind, speakers) = monster_group_sound(&definitions, kind);
        let mut channels = Vec::new();
        for i in speakers {
            let (voices, location, object, velocity) = monsters[i];
            if let Some(sound) = voices.sound_for(kind)
                && let Some(channel) = self.play_at(
                    sound,
                    Some(location),
                    object,
                    velocity,
                    REQUEST_FLAG_IGNORE_MEDIA,
                    listener,
                    audio,
                )
            {
                channels.push(channel);
            }
        }
        channels
    }

    pub fn world_point(&self, position: [f32; 3]) -> WorldPoint {
        let col = self.half_extent.0 - position[0];
        let row = position[2] + self.half_extent.1;
        WorldPoint::new(
            (col * WORLD_ONE as f32) as i32,
            (row * WORLD_ONE as f32) as i32,
            (position[1] * WORLD_ONE as f32) as i32,
        )
    }

    pub fn queue_one_shot(&mut self, sound: usize, location: WorldPoint) {
        self.pending.push((sound, location));
    }

    fn decode_new_sounds(&mut self) {
        for def in &self.manager.sounds[self.pcm.len()..] {
            self.pcm.push(
                def.tag
                    .permutations
                    .iter()
                    .map(|p| {
                        let pcm = p.decode();
                        SoundBuffer::from_pcm16_interleaved(
                            pcm.channels,
                            pcm.sample_rate,
                            pcm.samples,
                        )
                    })
                    .collect(),
            );
        }
    }

    pub fn summary(&self) -> String {
        let m = &self.manager;
        let bed = m
            .stereo_background
            .map(|i| m.ambient_definitions[i].id.to_string())
            .unwrap_or_else(|| "none".into());
        let loops = m
            .sources
            .iter()
            .filter(|s| m.ambient_definitions[s.definition].continuous.is_some())
            .count();
        format!(
            "{} sound sources ({} looping), stereo bed '{}', {} amso / {} soun definitions",
            m.sources.len(),
            loops,
            bed,
            m.ambient_definitions.len(),
            m.sounds.len()
        )
    }

    pub fn listener(&self, camera: &Camera) -> SoundListener {
        let mut observer = camera.clone();
        observer.distance = self.observer_distance;
        let location = self.world_point(observer.eye_position());
        let facing = (camera.yaw - std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
        let facing = ((facing / std::f32::consts::TAU) * 65536.0).round() as i64 as u16;
        SoundListener::from_observer(location, facing)
    }

    fn give_device(&mut self, listener: &SoundListener, audio: &SharedAudio) {
        let on = self.manager.sound_3d;
        if on {
            audio.set_listener_3d(device_listener(listener));
        }
        if self.reverb_on != Some(on) {
            self.reverb_on = Some(on);
            let preset = self.mesh_reverb.map(reverb_preset);
            if on && preset.is_none() {
                tracing::warn!("3D sound: the mesh has no header, so no reverb");
            }
            let preset = preset.filter(|_| on).and_then(|i| REVERB_PRESETS.get(i));
            audio.set_reverb(preset);
        }
    }

    pub fn play(
        &mut self,
        sound: usize,
        location: WorldPoint,
        listener: &SoundListener,
        audio: &SharedAudio,
    ) {
        self.play_with_flags(sound, location, 0, listener, audio);
    }

    pub fn play_with_flags(
        &mut self,
        sound: usize,
        location: WorldPoint,
        flags: u16,
        listener: &SoundListener,
        audio: &SharedAudio,
    ) -> Option<usize> {
        self.play_at(sound, Some(location), None, [0; 3], flags, listener, audio)
    }

    #[allow(clippy::too_many_arguments)]
    fn play_at(
        &mut self,
        sound: usize,
        location: Option<WorldPoint>,
        object: Option<ChannelObject>,
        velocity: [i16; 3],
        flags: u16,
        listener: &SoundListener,
        audio: &SharedAudio,
    ) -> Option<usize> {
        self.give_device(listener, audio);
        self.with_backend(audio, |manager, backend| {
            manager.play_object_sound(
                sound,
                location,
                object,
                velocity,
                FULL_VOLUME,
                flags,
                Some(listener),
                backend,
            )
        })
    }

    pub fn update(&mut self, ticks: i32, listener: &SoundListener, audio: &SharedAudio) {
        self.give_device(listener, audio);
        for (sound, location) in std::mem::take(&mut self.pending) {
            self.play(sound, location, listener, audio);
        }
        let before: [_; ONE_SHOT_CHANNEL_COUNT] = std::array::from_fn(|c| {
            (
                self.manager.one_shot(c),
                audio.channel_status(c) == EngineChannelStatus::Idle,
            )
        });
        let cells = std::mem::take(&mut self.fire_cells);
        self.with_backend(audio, |manager, backend| {
            manager.update_with_fire(ticks, Some(listener), backend, &|listener, amfi, volume| {
                report_fire_as_sound_source(&cells, listener, amfi, volume)
            });
        });
        self.fire_cells = cells;
        for (c, (old, idle)) in before.into_iter().enumerate() {
            let Some(old) = old else { continue };
            let id = self.manager.sounds[old.sound].id;
            match self.manager.one_shot(c) {
                None if idle => tracing::debug!("channel {c}: sound {id} finished"),
                None => tracing::debug!(
                    "channel {c}: sound {id} stopped at gain 0, at {:?}, listener {:?}",
                    old.location,
                    listener.location
                ),
                Some(new) if new.attenuated != old.attenuated => tracing::debug!(
                    "channel {c}: sound {id} gain {} -> {}, at {:?}, object {:?}, listener {:?}",
                    old.attenuated,
                    new.attenuated,
                    new.location,
                    new.object.map(|o| o.index),
                    listener.location
                ),
                Some(_) => {}
            }
        }
    }

    pub fn load_particle_sound(&mut self, ambient: FourCC, archives: &[&TagArchive]) {
        let mut resolver = archive_resolver(archives);
        self.manager.particle_definition =
            self.manager.load_ambient_definition(ambient, &mut resolver);
        self.decode_new_sounds();
    }

    pub fn read_fire(&mut self, sim: &mandrake_sim::Simulation) {
        self.fire_cells = fire_sound_cells(&sim.engine);
    }

    pub fn follow_objects(&mut self, sim: &mandrake_sim::Simulation, audio: &SharedAudio) {
        self.with_backend(audio, |manager, backend| {
            manager.follow_objects(|o| sim.sound_object_place(o), backend);
        });
    }
}

fn world_cells(v: [i32; 3]) -> [f32; 3] {
    v.map(|c| c as f32 * (1.0 / WORLD_ONE as f32))
}

const LISTENER_VELOCITY: [f32; 3] = [0.0; 3];

fn device_listener(listener: &SoundListener) -> Listener3d {
    let l = listener.location;
    Listener3d {
        position: world_cells([l.x, l.y, l.z]),
        facing: listener.facing as f32 * (std::f32::consts::TAU / 65536.0),
        velocity: LISTENER_VELOCITY,
    }
}

fn device_place(source: &DeviceSource) -> Place3d {
    match source.location {
        Some(at) => Place3d::At {
            position: world_cells([at.x, at.y, at.z]),
            velocity: world_cells(source.velocity.map(i32::from)),
        },
        None => Place3d::Flat,
    }
}

fn point_is_under_media(mesh: &MeshTag, p: WorldPoint) -> bool {
    if p.x < 0 || p.y < 0 {
        return false;
    }
    let cell = mesh.cell_at((p.x / WORLD_ONE) as usize, (p.y / WORLD_ONE) as usize);
    cell.is_some_and(|c| c.media_height > c.physical_height && p.z < c.media_height as i32)
}

struct NativeBackend<'a> {
    audio: &'a SharedAudio,
    pcm: &'a [Vec<Option<SoundBuffer>>],
    resampled: &'a mut HashMap<(usize, usize), Arc<SoundBuffer>>,
    pitch: &'a mut [i16; CHANNEL_COUNT],
    sources: &'a mut [DeviceSource; CHANNEL_COUNT],
    sound_3d: bool,
    dsound: Option<&'a mut [DsoundChannel; CHANNEL_COUNT]>,
}

impl SoundChannelBackend for NativeBackend<'_> {
    fn channel_status(&self, channel: usize) -> ChannelStatus {
        match self.audio.channel_status(channel) {
            EngineChannelStatus::Idle => ChannelStatus::Idle,
            EngineChannelStatus::Playing => ChannelStatus::Playing,
            EngineChannelStatus::Queued => ChannelStatus::Queued,
        }
    }

    fn stop_channel(&mut self, channel: usize) {
        self.audio.channel_stop(channel);
    }

    fn set_channel_source(&mut self, channel: usize, source: &DeviceSource) {
        if let Some(s) = self.sources.get_mut(channel) {
            *s = *source;
        }
    }

    fn set_channel_volume(&mut self, channel: usize, volumes: &SoundVolumes, pitch: i16) {
        if let Some(p) = self.pitch.get_mut(channel) {
            *p = pitch;
        }
        if let Some(ds) = self.dsound.as_deref_mut().and_then(|d| d.get_mut(channel)) {
            ds.set_volume(i32::from(volumes.attenuated));
            if self.sound_3d {
                let source = self.sources.get(channel).copied().unwrap_or_default();
                self.audio.channel_set_space(
                    channel,
                    ChannelSpace {
                        gain: mandrake_audio::dsound::millibels_to_gain(ds.volume),
                        place: device_place(&source),
                        send: source.location.is_some(),
                        unity_centre: true,
                    },
                );
            } else {
                ds.set_pan(i32::from(volumes.left), i32::from(volumes.right));
                let (left, right) = ds.gains();
                self.audio.channel_set_gains(channel, left, right);
            }
            return;
        }
        if self.sound_3d {
            let source = self.sources.get(channel).copied().unwrap_or_default();
            self.audio.channel_set_space(
                channel,
                ChannelSpace {
                    gain: device_gain(volumes.attenuated),
                    place: device_place(&source),
                    send: source.location.is_some(),
                    unity_centre: false,
                },
            );
            return;
        }
        let device = device_gain(volumes.attenuated);
        let attenuated = volumes.attenuated.max(1) as f32;
        let gain = |v: i16| device * v.max(0) as f32 / attenuated;
        self.audio
            .channel_set_gains(channel, gain(volumes.left), gain(volumes.right));
    }

    fn queue_permutation(&mut self, channel: usize, sound: usize, permutation: usize) {
        let Some(Some(buffer)) = self.pcm.get(sound).and_then(|p| p.get(permutation)) else {
            return;
        };
        let rate = self.audio.device_rate();
        let resampled = self
            .resampled
            .entry((sound, permutation))
            .or_insert_with(|| Arc::new(mandrake_audio::resample_buffer(buffer, rate)))
            .clone();
        let pitch = self.pitch.get(channel).copied().unwrap_or(FULL_VOLUME);
        if pitch == FULL_VOLUME {
            self.audio.channel_queue_resampled(channel, resampled);
        } else {
            self.audio.channel_queue_resampled_pitched(
                channel,
                resampled,
                pitch.max(1) as f32 / FULL_VOLUME as f32,
            );
        }
    }
}

pub const GAME_SOUND_LIST: FourCC = FourCC(*b"inns");
pub const GAME_SOUND_STRINGS: FourCC = FourCC(*b"ings");
const SOLO_GAME_SOUNDS: u32 = 0x78_0180;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameSounds {
    queue: Vec<u16>,
    playing: Option<usize>,
    delay: i32,
}

impl Default for GameSounds {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            playing: None,
            delay: 0x16,
        }
    }
}

impl GameSounds {
    pub fn queue(&mut self, id: u32, options: u32, muted: bool) {
        if muted {
            return;
        }
        if options & mandrake_common::game_options::COOPERATIVE != 0
            && (SOLO_GAME_SOUNDS >> (id & 0x1f)) & 1 == 0
        {
            return;
        }
        let id = id as u16;
        if self.queue.is_empty() && id == 7 {
            self.delay = 0;
        }
        for q in &mut self.queue {
            if *q == id {
                if id == 7 {
                    *q = 8;
                }
                return;
            }
            if *q == 8 && id == 7 {
                return;
            }
        }
        if self.queue.len() < 4 {
            self.queue.push(id);
        }
    }

    pub fn update(&mut self, elapsed: i32, is_playing: impl FnOnce(usize) -> bool) -> Option<u16> {
        match self.playing {
            None => {
                if self.delay < 0x16 {
                    self.delay += elapsed;
                    None
                } else if self.queue.is_empty() {
                    None
                } else {
                    Some(self.queue.remove(0))
                }
            }
            Some(sound) => {
                if !is_playing(sound) {
                    self.delay = 0;
                    self.playing = None;
                }
                None
            }
        }
    }

    pub fn started(&mut self, sound: usize) {
        self.playing = Some(sound);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_tag::SoundVolumes;

    #[test]
    fn tfl_levels_play_at_tfl_15s_gains() {
        use mandrake_audio::dsound::{millibels_to_gain, volume_table};
        let level =
            |dsound: &mut [DsoundChannel; CHANNEL_COUNT], sound_3d: bool, volumes: SoundVolumes| {
                let audio = SharedAudio::new(44100, 1.0);
                let mut resampled = HashMap::new();
                let mut pitch = [FULL_VOLUME; CHANNEL_COUNT];
                let mut sources = [DeviceSource::default(); CHANNEL_COUNT];
                let mut backend = NativeBackend {
                    audio: &audio,
                    pcm: &[],
                    resampled: &mut resampled,
                    pitch: &mut pitch,
                    sources: &mut sources,
                    sound_3d,
                    dsound: Some(dsound),
                };
                backend.set_channel_volume(0, &volumes, FULL_VOLUME);
                audio.channel_queue_resampled(
                    0,
                    Arc::new(SoundBuffer::from_f32_mono(vec![0.5; 4096], 44100)),
                );
                let mut out = vec![0.0f32; 2 * 256];
                audio.fill_stereo(&mut out);
                (out[200] / 0.5, out[201] / 0.5)
            };
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        let mut ds = [DsoundChannel::default(); CHANNEL_COUNT];
        let quiet = millibels_to_gain(-530);
        assert_eq!(volume_table(100), -530);
        let (l, r) = level(&mut ds, false, SoundVolumes::flat(100));
        assert!(near(l, quiet) && near(r, quiet), "{l} {r}");
        assert!(device_gain(100) < quiet * 0.9, "OpenAL's curve is lower");
        let (l, _) = level(&mut ds, false, SoundVolumes::flat(FULL_VOLUME));
        assert!(near(l, quiet), "the refused volume stays: {l}");
        let mut fresh = [DsoundChannel::default(); CHANNEL_COUNT];
        let panned = SoundVolumes {
            volume: FULL_VOLUME,
            attenuated: 150,
            left: 150 / 8,
            right: 150,
        };
        let entry = volume_table(150);
        let (l, r) = level(&mut fresh, false, panned);
        assert!(near(l, millibels_to_gain(entry)), "{l}");
        let pan = volume_table(18 * 0x180 / 150);
        assert_eq!(pan, -1215);
        assert!(near(r, millibels_to_gain(entry + pan)), "{r}");
        let mut three_d = [DsoundChannel::default(); CHANNEL_COUNT];
        let (l, r) = level(&mut three_d, true, SoundVolumes::flat(100));
        assert!(near(l, quiet) && near(r, quiet), "{l} {r}");
    }

    fn empty_mesh(cells: u16) -> MeshTag {
        MeshTag {
            mesh_id: FourCC::new(*b"test"),
            landscape_tag: FourCC::new(*b"test"),
            media_tag: FourCC::new(*b"test"),
            submesh_w: 3,
            submesh_h: 3,
            cell_width: cells,
            cell_height: cells,
            total_cells: 0,
            mesh_offset: 0,
            mesh_size: 0,
            cells: Vec::new(),
            palette_count: 0,
            palette_offset: 0,
            marker_count: 0,
            marker_offset: 0,
            palettes: Vec::new(),
            markers: Vec::new(),
            string_list_tag: None,
            overhead_tag: None,
            pregame_story_tag: None,
            postgame_tag: None,
            next_mesh_id: None,
            landscape_shading: Default::default(),
            reverb: None,
        }
    }

    #[test]
    fn sim_events_ask_for_their_tags_sounds() {
        let mut catalog = ProjectileCatalog::default();
        let mut raw = vec![0u8; 256];
        raw[0x04..0x08].copy_from_slice(b"dwm2");
        raw[0x3c..0x40].copy_from_slice(b"dwbf");
        let dwbl = FourCC::new(*b"dwbl");
        catalog.projectiles.insert(
            dwbl,
            mandrake_tag::ProjectileDefinition::from_bytes(dwbl, &raw).unwrap(),
        );
        let tags = SimSoundTags::from_catalog(&catalog);
        let created = CombatEvent::ProjectileCreated {
            tick: 3,
            definition: dwbl,
            object: 1,
            identifier: -1,
            point: [1, 2, 3],
            facing: 0,
            velocity: [4, -5, 6],
        };
        assert_eq!(
            tags.event_request(&created),
            Some(SoundRequest {
                sound: FourCC::new(*b"dwbf"),
                world: [1, 2, 3],
                flags: 0,
                object: Some(ChannelObject {
                    index: 1,
                    identifier: -1,
                }),
                velocity: [4, -5, 6],
            })
        );
        let hit = CombatEvent::HitReaction {
            tick: 0,
            unit_id: 1,
        };
        assert_eq!(tags.event_request(&hit), None);
    }

    #[test]
    fn a_sequence_plays_first_last_key() {
        let mut c = vec![0u8; 320 + 0x400];
        c[0x80..0x84].copy_from_slice(&1i32.to_be_bytes());
        c[0x84..0x88].copy_from_slice(&0i32.to_be_bytes());
        c[320 + 64..320 + 68].copy_from_slice(&0x100i32.to_be_bytes());
        c[320 + 0x100 + 0x18..320 + 0x100 + 0x1c].copy_from_slice(b"frst");
        c[320 + 0x100 + 0x1c..320 + 0x100 + 0x20].copy_from_slice(b"keyf");
        c[320 + 0x100 + 0x20..320 + 0x100 + 0x24].copy_from_slice(&[0xff; 4]);
        let id = FourCC::new(*b"coll");
        let mut tags = SimSoundTags::default();
        tags.add_collection(id, || Some(c));
        let s = SequenceSound {
            tick: 1,
            object: 1,
            identifier: -1,
            collection: id,
            sounds: mandrake_sim::engine_rng::AnimSounds {
                sequence: 0,
                first: true,
                key: true,
                last: true,
            },
            world: [0; 3],
            request_flags: 1,
            velocity: [0; 3],
        };
        let got: Vec<FourCC> = tags.sequence_requests(&s).iter().map(|r| r.sound).collect();
        assert_eq!(got, [FourCC::new(*b"frst"), FourCC::new(*b"keyf")]);
    }

    #[test]
    fn the_sequence_sound_gate_reads_the_rest_of_the_word() {
        let mesh = (32, 2);
        let at = |cell: i32| (cell * 512 + 5, 512 + 5);
        let gate =
            |visible: &[u32], options, (x, y)| sequence_sound_gate(visible, mesh, options, x, y);
        let mut visible = [0u32; 4];
        assert!(!gate(&visible, 0, at(3)));
        assert!(gate(&visible, 0x20_0000, at(3)));
        assert!(gate(&visible, 0x20_0000, (-1, -1)));
        visible[2] = 1 << (2 * 3);
        assert!(gate(&visible, 0, at(3)));
        assert!(gate(&visible, 0, at(1)), "a later cell of the word");
        assert!(!gate(&visible, 0, at(4)), "an earlier cell of the word");
        assert!(!gate(&visible, 0, at(16)), "the next word");
        assert!(!gate(&visible, 0, (-1, 517)));
        assert!(!gate(&visible, 0, (33 * 512, 517)));
    }

    #[test]
    fn world_point_matches_marker_mapping() {
        let sound = MapSound::load(&empty_mesh(96), None, &[], 10.0, 1);
        let p = sound.world_point([48.0 - 10.0, 0.5, 20.0 - 48.0]);
        assert_eq!(
            p,
            WorldPoint::new(10 * WORLD_ONE, 20 * WORLD_ONE, WORLD_ONE / 2)
        );
    }

    #[test]
    fn listener_maps_client_camera_to_world_units() {
        let mesh = MeshTag {
            mesh_id: FourCC::new(*b"test"),
            landscape_tag: FourCC::new(*b"test"),
            media_tag: FourCC::new(*b"test"),
            submesh_w: 3,
            submesh_h: 3,
            cell_width: 96,
            cell_height: 96,
            total_cells: 0,
            mesh_offset: 0,
            mesh_size: 0,
            cells: Vec::new(),
            palette_count: 0,
            palette_offset: 0,
            marker_count: 0,
            marker_offset: 0,
            palettes: Vec::new(),
            markers: Vec::new(),
            string_list_tag: None,
            overhead_tag: None,
            pregame_story_tag: None,
            postgame_tag: None,
            next_mesh_id: None,
            landscape_shading: Default::default(),
            reverb: None,
        };
        let sound = MapSound::load(&mesh, None, &[], 10.0, 1);
        let camera = Camera::from_myth_observer([0.0, 8.0, 0.0], 0.0, -27f32.to_radians(), 0.0);
        let mut s = sound;
        s.observer_distance = camera.distance;
        let l = s.listener(&camera);
        assert!((l.location.x - (48 + 6) * 512).abs() <= 2, "{:?}", l);
        assert!((l.location.y - 48 * 512).abs() <= 2, "{:?}", l);
        assert_eq!(l.facing, 0, "{:?}", l);
    }
}

#[cfg(test)]
mod game_sound_tests {
    use super::*;

    const SOLO: u32 = 0x200c_40fc;
    const NETWORK: u32 = 0x200c_407c;

    #[test]
    fn the_queue_plays_what_the_engine_played() {
        const FILM: u32 = 0x200c_40bc;
        const ASKED: [u32; 15] = [
            12156, 12356, 12474, 12573, 12717, 12809, 12827, 12827, 12897, 13011, 13140, 13303,
            13852, 13985, 14016,
        ];
        const PLAYED: [(u32, u16); 6] = [
            (12179, 7),
            (12448, 7),
            (12713, 8),
            (13015, 8),
            (13314, 8),
            (13875, 7),
        ];
        let mut s = GameSounds::default();
        let mut got = Vec::new();
        for tick in 12100..14017 {
            for _ in ASKED.iter().filter(|a| **a == tick) {
                s.queue(if tick == 14016 { 6 } else { 7 }, FILM, false);
            }
            let counter = tick + 1;
            let next = PLAYED.iter().find(|p| p.0 > counter).map(|p| p.0);
            let ended = next.is_some_and(|n| counter + 0x17 >= n);
            if let Some(id) = s.update(1, |_| !ended) {
                got.push((counter, id));
                s.started(1);
            }
        }
        assert_eq!(got, PLAYED);
    }

    #[test]
    fn a_solo_game_has_no_end_sound() {
        let mut s = GameSounds::default();
        s.queue(6, SOLO, false);
        s.queue(5, SOLO, false);
        assert_eq!(s.update(1, |_| false), None);
        s.queue(6, NETWORK, false);
        assert_eq!(s.update(1, |_| false), Some(6));
    }

    #[test]
    fn sounds_wait_their_turn_and_casualties_gather() {
        let mut s = GameSounds::default();
        s.queue(7, SOLO, false);
        s.queue(7, SOLO, false);
        s.queue(7, SOLO, false);
        for _ in 0..0x16 {
            assert_eq!(s.update(1, |_| false), None);
        }
        assert_eq!(s.update(1, |_| false), Some(8));
        s.started(3);
        s.queue(0x15, SOLO, false);
        assert_eq!(s.update(1, |_| true), None);
        assert_eq!(s.update(1, |_| false), None);
        for _ in 0..0x16 {
            assert_eq!(s.update(1, |_| false), None);
        }
        assert_eq!(s.update(1, |_| false), Some(0x15));
        let mut full = GameSounds::default();
        for id in [1, 2, 3, 4, 5] {
            full.queue(id, NETWORK, false);
        }
        assert_eq!(full.queue.len(), 4);
        full.queue(1, NETWORK, true);
    }
}
