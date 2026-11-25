# Audio and cutscenes (mandrake-audio, mandrake-smacker)

## mandrake-audio: the mixer

`crates/mandrake-audio` is a pure software mixer. It has no I/O and no device backend, and depends only
on `mandrake-common`, `thiserror` and `tracing`. Its entry point is `SharedAudio` (`output.rs`), which
wraps an `Arc<Mutex<AudioEngine>>` together with the device sample rate. The audio callback calls
`SharedAudio::fill_stereo(out)` to fetch interleaved stereo `f32` samples.

### Two models in one mixer (`mixer.rs`, `SpatialAudioMixer`)

1. **Engine channels**, the model the game uses. There are `ENGINE_CHANNEL_COUNT` (8) channels, and
   each channel is a queue of `Arc<SoundBuffer>` played back to back:
   - `SharedAudio::channel_queue` and `channel_queue_resampled` add buffers to a channel.
   - `channel_set_gains` sets the channel's gains. A gain change ramps linearly across one mixed
     block, so it doesn't click.
   - `channel_stop` stops the channel, and `channel_status` returns an `EngineChannelStatus`:
     `Idle`, `Playing` or `Queued`.

   The channel layout belongs to the sound manager in `mandrake_tag::sound_manager`:
   - channels 0 to 4 play one-shots (`ONE_SHOT_CHANNEL_COUNT`);
   - `AMBIENT_CHANNELS` are 5 and 6;
   - `STEREO_CHANNEL` is 7 (music, narration, cutscene sound);
   - each channel holds `CHANNEL_QUEUE_DEPTH` buffers.

   The sound manager decides what plays where and at what volume. The mixer only carries that out.
2. **The legacy spatial model**: `SoundChannel`, `SoundSource`, `Listener`,
   `calculate_spatial_gains` with a linear falloff and constant-power pan, and `trigger_one_shot`.
   Only `--debug-sound`, `SharedAudio::play_one_shot*` and the demo `WebClient` in `mandrake-web` use it.
   Game code should not use it.

`mix` zero-fills the output, adds both models and then the reverb, and clamps the result to [-1, 1].
The mixer has no fixed sample rate: it runs at whatever rate `SharedAudio::new(device_rate, volume)`
was given. Buffers must be at the device rate before they are queued. Use
`resample_buffer`/`resample_buffer_with_pitch` (linear) or `channel_queue_resampled` to convert
them.

### 3D sound (`device3d.rs`) and reverb (`reverb.rs`, `reverb_presets.rs`)

When the player's sound preferences ask for 3D sound, a channel can be placed with
`channel_set_space(ChannelSpace { gain, place: Place3d, send })`. `Place3d` is either `Flat` or
`At { position, velocity }`. Positions and velocities are in cells, with x east, y south and z up.
On every block the mixer pans the channel and applies Doppler pitch, reproducing the original
engine's 3D device:

- **Panning.** `device3d::pan_angles(azimuth, elevation)` interpolates bilinearly in a measured pan
  table (`pan_table.rs`), so it is a lookup, not a formula. This layer applies no distance
  attenuation, because distance gain comes from the sound manager's volumes.
- **Doppler.** `device3d::doppler` uses `SPEED_OF_SOUND` = 343.3 and caps the pitch at
  `MAX_DOPPLER`.
- **Reverb.** A located source with `send` feeds a mono send into `Reverb`, which is set with
  `SharedAudio::set_reverb(Option<&ReverbPreset>)`. `REVERB_PRESETS` is the original game's table of
  26 environment presets. The reverb is built from input shelves, an early-reflection network,
  allpass diffusers and a four-line feedback delay network. Its constants (`reverb::fit`) were
  fitted to measured output.

Stereo buffers play unpanned, and `Place3d::Flat` plays at `FLAT_MONO_GAIN`.

## The output device (`crates/mandrake-client/src/audio_out.rs`)

**Native (cpal).**
- `AudioOutput::open(volume)` tries the default output device first, then each other device in
  turn.
- It takes the device's default config (F32, I16 or U16 samples) and writes stereo into channels 0
  and 1; any further channels get silence.
- The client holds `AudioLink { audio: SharedAudio, stats }`, and there is one output per process.
- If no device opens, the client runs silent. This is never fatal.
- `--mute` opens no device. `--master-volume` sets `SharedAudio::set_master_volume`.

**Web.** There is no cpal. The page calls `set_audio_rate(rate)` and then pumps `mix_audio(out)` (see
[web.md](web.md)).

## How the client feeds sounds

`crates/mandrake-client/src/map_sound.rs` defines `MapSound`, which bridges `mandrake_tag::SoundManager` and
the mixer:

- `MapSound::load(...)` decodes every sound permutation at load time and keeps device-rate copies.
- `NativeBackend` implements `mandrake_tag::SoundChannelBackend`, which carries the sound manager's
  channel calls to `SharedAudio`.
- Without 3D sound, a channel gets left and right gains. With 3D sound, it gets a `ChannelSpace`
  and the mesh's reverb preset.
- The 3D and ambient switches come from the original game's `preferences/sound` file, which is only
  read (`app/sound.rs`, `sound_preference_flags`).

What `MapSound` plays:
- the mesh's stereo background;
- the ambient sound markers;
- fire;
- particle-system ambience;
- unit voices (`load_monster_voices`, `play_monsters_sound`);
- simulation events (`play_sim_sounds`);
- map-action sounds;
- the announcer.

**Per tick** (`app/tick.rs`), the client runs `play_map_action_sounds` and `play_combat_events`. The
latter drains the simulation's combat events and sequence sounds and passes them to
`MapSound::play_sim_sounds`. Local projectile sounds run here too.

**Per frame**, `MapSound::update(polls, &listener, &audio)` runs only when at least one 30 Hz input
poll has passed in this frame. Its argument is the number of **polls**, not game ticks, as in the
original engine. At game speed 0 the ambience therefore still advances. The listener comes from
`MapSound::listener(&camera)`, which is the interpolated render camera, and `SharedAudio` also gets
it through `set_listener`.

**Menus.** `app/menu_sound.rs` defines `MenuSound`. It plays the front end's
`mandrake_session::SoundCue`s on channel 7: `Music`, `Narration`, `Ambient`, `FadeOut { ticks }` and
`Stop`. `MenuSound::update(now_ms)` runs fades and re-queues loops.

**The Fallen Lords.** `tfl_menu_music.rs` defines `TflMusic`, and `tfl_sound.rs` defines
`TflLevelSound`. TFL levels reuse `MapSound` with TFL's sound rules (`SoundRules::TFL`, which has
fewer one-shot channels).

## Cutscenes

### mandrake-smacker

`crates/mandrake-smacker` is a decoder for Smacker video (SMK2 and SMK4) with no dependencies:

- `Smacker::open(&[u8])` opens a file, and `.header()` returns the `Header` (size, `frame_count`,
  `fps()`, `audio_tracks`).
- `.decoder()` returns a `Decoder`. `Decoder::next_frame()` returns `Result<Option<Frame>>`.
- A `Frame` exposes `pixels()` (palette indices), `palette()`, `rgba()` and `audio` (a list of
  `AudioChunk`; `Samples::to_i16`).
- Audio is either raw PCM or Huffman-coded DPCM. Bink audio tracks are flagged but not decoded.
- The modules are `bits` (an LSB-first bit reader), `huffman` and `video` (palette update and 4x4
  block decode).
- Benchmark: `cargo run --release -p mandrake-smacker --example smk-bench -- FILE.smk`.

### Playback (`crates/mandrake-client/src/app/cutscene.rs`)

- `Cutscene::open(game_dir, name, ...)` looks for `<game>/cutscenes/<name>` with the extensions in
  `EXTENSIONS`. Only Smacker files play.
- A decoder thread sends RGBA frames and sound ahead of time (`LEAD_FRAMES`).
- The first audio track is resampled to the device rate and queued on channel 7.
- The `.smk`'s frame rate times the picture.
- Any key or click skips the cutscene.
- The picture is scaled to the window through `InterfaceScaler`, with `cutscene_filter` (FSR1 by
  default; see [configuration.md](configuration.md#interface-interfacedefaulttoml)).
- On the web there are no threads, so cutscenes are skipped.

### HD cutscenes (`app/cutscene_hd.rs`, native only)

A player can provide an upscaled AV1 version of a cutscene at
`$XDG_DATA_HOME/myth2-sourceport/cutscenes-hd/<name>.ivf`. On Windows it goes under
`%LOCALAPPDATA%`.

- **Selection.** `cutscene_hd::choose` uses the file only if it parses as IVF with codec `AV01` and
  has exactly the `.smk`'s frame count. Otherwise the `.smk` plays.
- **Decoding.** rav1d decodes the file on its own thread, and another thread uploads the frames.
  `HdGpu` converts YUV to RGB in a shader.
- **Timing and sound.** The `.smk` still supplies the clock and the sound.
- **Switches.** `--cutscene-hd on|off` and the `cutscene_hd` interface key turn it on or off.
- **Build.** On x86_64 Linux, rav1d is built with its assembly, which needs `nasm`.

No HD files ship. Making them is up to the player.
