//! Myth II sound playback: the engine's eight channels mixed to stereo, with the 3D device's
//! panning, Doppler and reverb, for 16-bit PCM and 32-bit float buffers.

pub mod device3d;
pub mod dsound;
pub mod mixer;
pub mod output;
mod pan_table;
pub mod reverb;
#[cfg(test)]
mod reverb_analysis;
pub mod reverb_presets;

pub use device3d::{Listener3d, Place3d};
pub use output::{SharedAudio, resample_buffer, resample_buffer_with_pitch};
pub use reverb::ReverbPreset;
pub use reverb_presets::REVERB_PRESETS;

pub use mixer::{
    ChannelSpace, Channels, ENGINE_CHANNEL_COUNT, EngineChannelStatus, SoundBuffer,
    SpatialAudioMixer,
};
