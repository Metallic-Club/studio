//! Audio for the Metallic Engine: sounds, mixer groups, effects and 3D spatial audio behind our
//! own interface.
//!
//! - [`AudioEngine`] is what the game thread uses. It mirrors Roblox's `Sound`,
//!   `SoundGroup`, `SoundService` and `SoundEffect` (see [`SoundProperties`], [`GroupId`],
//!   [`SpatialSettings`], [`Effect`]) so the runtime can map Instances to it one to one.
//! - Assets are decoded PCM ([`SoundAsset`]: WAV, OGG Vorbis, FLAC, MP3 through Symphonia) or
//!   streamed while playing ([`DecoderStream`], any [`StreamSource`]).
//! - Mixing ([`MixRenderer`]) runs on the audio thread in fixed 64-frame blocks: resampling and
//!   `PlaybackSpeed`, distance roll-off ([`roll_off_gain`]), equal-power panning, Doppler,
//!   occlusion low-pass, a voice limit with priority-based virtual voices, five built-in
//!   groups (Master, Music, SFX, UI, Voice) plus user groups, and group effects.
//! - Output goes to an [`AudioBackend`]: the headless backend (deterministic offline rendering
//!   for tests and servers, no device needed) or [`DeviceBackend`] (cpal, feature `device`,
//!   on by default).
//!
//! Units: studs and seconds, like the rest of the engine. Volumes are linear (Roblox `Volume`),
//! effect levels are in dB (Roblox `SoundEffect` levels).
#![warn(missing_docs)]

mod asset;
mod backend;
#[cfg(feature = "device")]
mod device;
mod effects;
mod engine;
mod error;
mod mixer;
mod spatial;
mod types;

pub use asset::{DecoderStream, SoundAsset, StreamSource};
pub use backend::AudioBackend;
#[cfg(feature = "device")]
pub use device::DeviceBackend;
pub use effects::{
    CompressorParams, DistortionParams, EchoParams, Effect, EqualizerParams, FilterParams,
    MAX_ECHO_DELAY, ReverbParams, db_to_gain,
};
pub use engine::{AudioEngine, OcclusionHook};
pub use error::AudioError;
pub use mixer::{BLOCK, MixRenderer};
pub use spatial::{
    DOPPLER_RANGE, OPEN_CUTOFF_HZ, SPEED_OF_SOUND_MPS, Spatialized, doppler_factor, listener_local,
    pan_gains, roll_off_gain, spatialize,
};
pub use types::{
    AssetId, EffectId, Emitter, EngineConfig, GroupId, Listener, MixStats, RollOff, RollOffMode,
    SoundEvent, SoundId, SoundProperties, SpatialSettings,
};

/// Re-export of the vector type used in the public API.
/// Spatial vector from the shared workspace math dependency.
pub use glam::Vec3;
