//! Typed errors of the audio crate.

use crate::{AssetId, EffectId, GroupId, SoundId};

/// Everything that can go wrong in `metallic_audio`.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// The bytes are a known container/codec but could not be decoded.
    #[error("could not decode audio: {0}")]
    Decode(String),
    /// The bytes are not a format we can read (or the codec is not compiled in).
    #[error("unsupported audio format: {0}")]
    Unsupported(String),
    /// Reading a file failed.
    #[error("audio I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The output device failed to open or reported an error.
    #[error("audio device error: {0}")]
    Device(String),
    /// No output device exists (typical on servers and CI machines).
    #[error("no audio output device")]
    NoDevice,
    /// The sound handle does not refer to a live sound.
    #[error("unknown sound {0:?}")]
    UnknownSound(SoundId),
    /// The group handle does not refer to a live group.
    #[error("unknown sound group {0:?}")]
    UnknownGroup(GroupId),
    /// The asset handle does not refer to a loaded asset.
    #[error("unknown sound asset {0:?}")]
    UnknownAsset(AssetId),
    /// The effect handle does not refer to a live effect.
    #[error("unknown sound effect {0:?}")]
    UnknownEffect(EffectId),
    /// A configured capacity (sounds, groups, effects per group) is exhausted.
    #[error("audio limit reached: {0}")]
    Limit(&'static str),
    /// A parameter is out of its valid range or otherwise unusable.
    #[error("invalid audio parameter: {0}")]
    InvalidParameter(String),
    /// The call needs the headless backend (offline rendering) but the engine drives a device.
    #[error("offline rendering needs the headless backend")]
    NotHeadless,
}
