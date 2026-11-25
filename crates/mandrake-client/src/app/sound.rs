use crate::app::ClientApp;
use crate::args::Args;
use crate::{audio_out, console, map_sound, media};
use mandrake_common::FourCC;
use mandrake_tag::sound_manager::{ChannelObject, DEFAULT_SOUND_PREFERENCE_FLAGS};
use mandrake_tag::{ModelAnimationTag, TagArchive};
use std::path::Path;
use tracing::{debug, info};

fn load_debug_sound(
    archives: &[Option<&TagArchive>],
    id: &str,
) -> Option<mandrake_audio::SoundBuffer> {
    let bytes: [u8; 4] = id.as_bytes().try_into().ok()?;
    let id = FourCC::new(bytes);
    for archive in archives.iter().flatten() {
        let Some(entry) = archive.find_tag_by_id(FourCC::new(*b"soun"), id) else {
            continue;
        };
        let data = archive.get_tag_data(entry).ok()?;
        let tag = mandrake_tag::SoundTag::from_bytes(&data).ok()?;
        let pcm = tag.decode_permutation(0)?;
        return mandrake_audio::SoundBuffer::from_pcm16_interleaved(
            pcm.channels,
            pcm.sample_rate,
            pcm.samples,
        );
    }
    None
}

const SOUND_ARCHIVE_NAMES: [&str; 2] =
    ["international small install", "international large install"];

fn open_sound_archives(archive_path: &Path) -> Vec<TagArchive> {
    let vfs = mandrake_common::game_vfs();
    let Some(dir) = archive_path.parent() else {
        return Vec::new();
    };
    SOUND_ARCHIVE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .filter(|p| crate::game_files::exists(p))
        .filter_map(|p| TagArchive::open(vfs, &p.to_string_lossy()).ok())
        .collect()
}

pub(crate) fn open_game_sound_archives(archive_path: &Path) -> Vec<TagArchive> {
    if !crate::game_files::exists(archive_path) {
        return Vec::new();
    }
    let vfs = mandrake_common::game_vfs();
    let large = archive_path.parent().map(|d| d.join("large install"));
    std::iter::once(archive_path.to_path_buf())
        .chain(large.filter(|p| crate::game_files::exists(p)))
        .filter_map(|p| TagArchive::open(vfs, &p.to_string_lossy()).ok())
        .chain(open_sound_archives(archive_path))
        .collect()
}

pub(crate) fn game_sound_tag(archives: &[&TagArchive], index: usize) -> Option<FourCC> {
    let list = archives.iter().rev().find_map(|a| {
        a.find_tag_by_id(FourCC(*b"soli"), map_sound::GAME_SOUND_LIST)
            .and_then(|e| a.get_tag_data(e).ok())
    })?;
    let e = list.get(0x10..)?.as_chunks::<8>().0.get(index)?;
    let tag = [e[0], e[1], e[2], e[3]];
    (tag != [0xff; 4] && tag != [0; 4]).then_some(FourCC(tag))
}

pub(crate) fn sound_preference_flags(archive_path: &Path) -> u16 {
    archive_path
        .parent()
        .and_then(Path::parent)
        .and_then(|game| crate::game_files::read(&game.join("preferences/sound")).ok())
        .and_then(|file| mandrake_tag::sound_manager::sound_preference_flags(&file))
        .unwrap_or(DEFAULT_SOUND_PREFERENCE_FLAGS)
}

pub(crate) fn open_audio_device(args: &Args) -> Option<audio_out::AudioOutput> {
    if args.mute {
        eprintln!("[audio] --mute: no audio device opened");
        return None;
    }
    audio_out::AudioOutput::open(crate::settings::sound_volume(args.master_volume))
}

pub(crate) fn load_map_sound(app: &mut ClientApp, args: &Args, ambient_tag: Option<FourCC>) {
    if app.audio.is_some() {
        app.sound_archives = open_sound_archives(&args.archive);
        let archives = sound_archives(
            &app.archive,
            app.large_archive.as_ref(),
            &app.sound_archives,
        );
        let seed = crate::clock::SystemTime::now()
            .duration_since(crate::clock::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let mut map_sound = map_sound::MapSound::load(
            &app.mesh_tag,
            ambient_tag,
            &archives,
            app.camera.distance,
            seed,
        );
        if let Some(ambient) = app
            .particle_field
            .as_ref()
            .and_then(|field| field.ambient_sound())
        {
            map_sound.load_particle_sound(ambient, &archives);
        }
        eprintln!("[audio] map sounds: {}", map_sound.summary());
        let flags = sound_preference_flags(&args.archive);
        map_sound.apply_sound_preferences(flags);
        eprintln!(
            "[audio] sound preference flags {flags}, 3D sound {}",
            if map_sound.manager.sound_3d {
                "on"
            } else {
                "off"
            }
        );
        app.map_sound = Some(map_sound);
        app.load_unit_voices();
        app.load_game_sounds();
    }
}

const DEBUG_SOUND_CHANNEL: usize = 0;

pub(crate) fn play_debug_sound(app: &ClientApp, args: &Args) {
    if let Some(id) = &args.debug_sound {
        match load_debug_sound(&[Some(&app.archive), app.large_archive.as_ref()], id) {
            None => eprintln!(
                "[audio] ERROR: debug sound 'soun/{id}' not found or undecodable in tag archives"
            ),
            Some(buf) => {
                let peak = (0..buf.frame_count())
                    .map(|i| {
                        let (l, r) = buf.get_frame(i);
                        l.abs().max(r.abs())
                    })
                    .fold(0.0f32, f32::max);
                eprintln!(
                    "[audio] debug sound '{id}': {} frames @ {} Hz, {} ch, decoded peak {peak:.3}",
                    buf.frame_count(),
                    buf.sample_rate(),
                    buf.channels() as u8
                );
                if peak == 0.0 {
                    eprintln!("[audio] ERROR: decoded debug sound is silent");
                }
                match &app.audio {
                    None => eprintln!("[audio] ERROR: no output device; debug sound not played"),
                    Some(out) => {
                        out.audio.channel_set_gains(DEBUG_SOUND_CHANNEL, 1.0, 1.0);
                        out.audio.channel_queue(DEBUG_SOUND_CHANNEL, &buf, 1.0);
                        eprintln!("[audio] debug sound queued on channel {DEBUG_SOUND_CHANNEL}");
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            let (audio, stats) = (out.audio.clone(), out.stats.clone());
                            std::thread::spawn(move || {
                                let start = crate::clock::Instant::now();
                                while audio.channel_status(DEBUG_SOUND_CHANNEL)
                                    != mandrake_audio::EngineChannelStatus::Idle
                                    && start.elapsed().as_secs() < 30
                                {
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                                let peak = f32::from_bits(
                                    stats.peak_bits.load(std::sync::atomic::Ordering::Relaxed),
                                );
                                eprintln!(
                                    "[audio] debug sound finished after {:?}: {} callbacks, peak output {peak:.3}",
                                    start.elapsed(),
                                    stats.callbacks.load(std::sync::atomic::Ordering::Relaxed)
                                );
                            });
                        }
                    }
                }
            }
        }
    }
}

fn sound_archives<'a>(
    archive: &'a TagArchive,
    large: Option<&'a TagArchive>,
    extra: &'a [TagArchive],
) -> Vec<&'a TagArchive> {
    std::iter::once(archive).chain(large).chain(extra).collect()
}

fn voice_eligible(alive: bool, unit_team: u8, local_team: i16) -> bool {
    alive && i16::from(unit_team) == local_team
}

impl ClientApp {
    pub(crate) fn load_model_animation_sounds(&mut self) {
        let Some(gpu) = &self.gpu_state else {
            return;
        };
        let Some(map_sound) = self.map_sound.as_mut() else {
            return;
        };
        let archives = sound_archives(
            &self.archive,
            self.large_archive.as_ref(),
            &self.sound_archives,
        );
        let running = self.sim.engine_stop.is_none();
        for anim in gpu
            .model_instances
            .iter()
            .filter_map(|i| i.animation.as_ref())
        {
            for id in anim.definition.sounds.iter().flatten() {
                if map_sound.load_sound(*id, &archives).is_none() {
                    eprintln!("[audio] WARNING: model animation sound 'soun/{id}' not found");
                }
            }
            if anim.definition.flags & ModelAnimationTag::FLAG_RANDOM_START_FRAME != 0
                || (running && !anim.in_sim)
            {
                continue;
            }
            if let Some(index) = anim
                .playback
                .creation_sound(&anim.definition)
                .and_then(|id| map_sound.manager.sound_index(id))
            {
                map_sound.queue_one_shot(index, anim.location);
            }
        }
    }

    fn load_game_sounds(&mut self) {
        let archives = sound_archives(
            &self.archive,
            self.large_archive.as_ref(),
            &self.sound_archives,
        );
        let group = FourCC(*b"soli");
        let list = archives.iter().rev().find_map(|a| {
            a.find_tag_by_id(group, map_sound::GAME_SOUND_LIST)
                .and_then(|e| a.get_tag_data(e).ok())
        });
        if let (Some(m), Some(list)) = (self.map_sound.as_mut(), list) {
            m.load_game_sounds(&list, &archives);
        }
    }

    fn load_unit_voices(&mut self) {
        let Some(map_sound) = self.map_sound.as_mut() else {
            return;
        };
        let archives = sound_archives(
            &self.archive,
            self.large_archive.as_ref(),
            &self.sound_archives,
        );
        let mut archetypes: Vec<FourCC> = self.sim.units.values().map(|u| u.archetype).collect();
        archetypes.sort_unstable_by_key(|a| a.as_u32());
        archetypes.dedup();
        let mut voiced = 0;
        for arch in archetypes {
            let unit = self
                .archive
                .find_tag_by_id(FourCC::new(*b"unit"), arch)
                .or_else(|| self.archive.find_tag(FourCC::new(*b"unit"), arch.as_str()))
                .and_then(|e| mandrake_tag::UnitTag::from_archive(&self.archive, e).ok());
            let Some(unit) = unit else {
                continue;
            };
            for id in map_sound.load_monster_voices(arch, &unit, &archives) {
                eprintln!("[audio] WARNING: unit '{arch}' voice 'soun/{id}' not found");
            }
            if unit.sounds.iter().any(Option::is_some) {
                voiced += 1;
            }
        }
        eprintln!("[audio] unit voices: {voiced} voiced archetype(s)");
        {
            let catalog = &self.sim.engine.catalog;

            let tags = map_sound::SimSoundTags::from_catalog(catalog);
            for id in map_sound.load_sim_sounds(tags, catalog, &archives) {
                eprintln!("[audio] WARNING: combat sound 'soun/{id}' not found");
            }
        }
        if let Some(actions) = self.sim.engine.map_actions.as_ref() {
            for id in map_sound.load_action_sounds(actions, &archives) {
                eprintln!("[audio] WARNING: map action sound 'soun/{id}' not found");
            }
        }
    }

    pub(crate) fn play_map_action_sounds(&mut self) {
        self.update_game_sounds();
        let mut plays = console::take_sound_plays(&mut self.sim);
        if plays.is_empty() {
            return;
        }
        let listener = self
            .map_sound
            .as_ref()
            .map(|s| s.listener(&self.render_camera));
        let world = &self.sim.engine;
        map_sound::resolve_nearest_visible(
            &mut plays,
            world,
            self.minimap.as_ref().map(|m| m.vis.visible.as_slice()),
            listener.as_ref(),
        );
        if let Some(hud) = self.hud.as_mut() {
            hud.print_sound_plays(&plays);
        }
        if let (Some(out), Some(map_sound), Some(listener)) =
            (&self.audio, self.map_sound.as_mut(), listener.as_ref())
        {
            map_sound.play_action_sounds(&plays, world, listener, &out.audio);
        }
    }

    fn update_game_sounds(&mut self) {
        let options = self.sim.game_option_flags();
        let requests = std::mem::take(&mut self.sim.engine.game.game_sound_requests);
        if !requests.is_empty() {
            info!(
                "Game sounds requested at tick {}: {requests:?}",
                self.sim.current_tick
            );
        }
        let (Some(out), Some(map_sound)) = (&self.audio, self.map_sound.as_mut()) else {
            return;
        };
        for id in requests {
            map_sound.game_sounds.queue(u32::from(id), options, false);
        }
        let mut sounds = std::mem::take(&mut map_sound.game_sounds);
        let id = sounds.update(1, |s| map_sound.sound_is_playing(s));
        if let Some(id) = id {
            let index = map_sound
                .game_sound_tags
                .get(usize::from(id))
                .copied()
                .flatten()
                .and_then(|t| map_sound.manager.sound_index(t));
            if let Some(index) = index {
                let listener = map_sound.listener(&self.render_camera);
                map_sound.play_game_sound(index, &listener, &out.audio);
                sounds.started(index);
                info!(
                    "Game sound {id} plays at tick {}: sound index {index}",
                    self.sim.current_tick
                );
            }
            let line = self.hud.as_ref().and_then(|h| {
                let d = h
                    .tags_lookup()
                    .get(FourCC(*b"stli"), map_sound::GAME_SOUND_STRINGS)?;
                mandrake_tag::StringListTag::from_bytes(&d)
                    .ok()?
                    .get(usize::from(id))
                    .map(str::to_string)
            });
            if let (Some(h), Some(line)) = (self.hud.as_mut(), line.filter(|l| l.trim() != "")) {
                info!("Game sound {id} prints: {line}");
                h.print_game_sound_line(&line, media::local_time(self.sim.current_tick));
            }
        }
        map_sound.game_sounds = sounds;
    }

    pub(crate) fn play_combat_events(&mut self) {
        let events = self.sim.drain_combat_events();
        let mut sequences = self.sim.drain_sequence_sounds();
        if let (Some(minimap), world) = (self.minimap.as_ref(), &self.sim.engine) {
            let (cells, options) = (world.mesh_cells, world.game_options);
            sequences.retain(|s| {
                map_sound::sequence_sound_gate(
                    &minimap.vis.visible,
                    cells,
                    options,
                    s.world[0],
                    s.world[1],
                )
            });
        }
        for event in &events {
            debug!("Combat event: {event:?}");
            self.landscape_edits
                .extend(crate::world_view::myth2::landscape_edit(event, &self.sim));
            if let Some(local) = self.local_projectiles.as_mut() {
                local.start_for_event(event);
            }
        }
        let (Some(out), Some(map_sound)) = (&self.audio, self.map_sound.as_mut()) else {
            return;
        };
        if !sequences.is_empty() {
            let archives = sound_archives(
                &self.archive,
                self.large_archive.as_ref(),
                &self.sound_archives,
            );
            for s in &sequences {
                map_sound.load_sequence_sounds(s.collection, &archives);
            }
        }
        let listener = map_sound.listener(&self.render_camera);
        map_sound.play_sim_sounds(&events, &sequences, &listener, &out.audio);
        map_sound.follow_objects(&self.sim, &out.audio);
        map_sound.read_fire(&self.sim);
    }

    pub(crate) fn play_local_projectile_sounds(&mut self) {
        let Some(local) = self.local_projectiles.as_mut() else {
            return;
        };
        let sounds = local.take_sounds();
        if sounds.is_empty() {
            return;
        }
        let (Some(out), Some(map_sound)) = (&self.audio, self.map_sound.as_mut()) else {
            return;
        };
        let archives = sound_archives(
            &self.archive,
            self.large_archive.as_ref(),
            &self.sound_archives,
        );
        let listener = map_sound.listener(&self.render_camera);
        map_sound.play_sequence_sounds(&sounds, &archives, &listener, &out.audio);
    }

    pub(crate) fn play_unit_voices(
        &mut self,
        unit_ids: &[u32],
        kind: mandrake_tag::MonsterSoundType,
    ) {
        let (Some(out), Some(map_sound)) = (&self.audio, self.map_sound.as_mut()) else {
            return;
        };
        let engine = &self.sim.engine;
        let local_team = engine.local_team();
        let monsters: Vec<_> = unit_ids
            .iter()
            .filter_map(|id| Some((*id, self.sim.units.get(id)?)))
            .filter(|(_, u)| voice_eligible(u.is_alive(), u.team, local_team))
            .map(|(id, u)| {
                let at = [
                    u.position.x.to_f32(),
                    u.position.y.to_f32(),
                    u.position.z.to_f32(),
                ];
                let monster = engine
                    .monsters
                    .iter()
                    .find(|m| m.identifier as i64 == id as i64);

                let object = monster.map(|m| ChannelObject {
                    index: m.object.index,
                    identifier: m.object.identifier,
                });
                let velocity = monster.map_or([0; 3], |m| m.object.velocity);
                (u.archetype, map_sound.world_point(at), object, velocity)
            })
            .collect();
        if monsters.is_empty() {
            return;
        }
        let listener = map_sound.listener(&self.render_camera);
        let channels = map_sound.play_monsters_sound(&monsters, kind, &listener, &out.audio);
        info!(
            "Unit voice type {} for {} unit(s): channels {:?}",
            kind.0,
            monsters.len(),
            channels
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_voices_follow_local_team() {
        assert!(voice_eligible(true, 0, 0));
        assert!(!voice_eligible(false, 0, 0));
        assert!(voice_eligible(true, 3, 3));
        assert!(!voice_eligible(true, 0, 3));
        assert!(!voice_eligible(true, 0, -1));
    }
}
