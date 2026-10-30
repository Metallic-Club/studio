//! Plain data types shared by the engine, the mixer and the spatializer.
//!
//! Field names follow Roblox's `Sound`, `SoundService` and `SoundGroup` properties in snake case
//! (`Volume` -> `volume`, `RollOffMinDistance` -> `roll_off_min_distance`) so the runtime can map
//! Instance properties one to one.

use glam::Vec3;

/// Handle to a loaded sound asset (decoded PCM, or an encoded file that is streamed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(pub(crate) u32);

/// Handle to a sound (a `Sound` instance): one playable voice with its own properties.
///
/// Handles are generational: a removed sound's handle never refers to a later sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Handle to a mixer group (a `SoundGroup`). Five groups always exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupId(pub(crate) u32);

impl GroupId {
    /// The master bus; every other group feeds it, directly or through parents.
    pub const MASTER: GroupId = GroupId(0);
    /// Background music (child of Master).
    pub const MUSIC: GroupId = GroupId(1);
    /// Sound effects in the world; the default group of new sounds (child of Master).
    pub const SFX: GroupId = GroupId(2);
    /// Interface sounds such as clicks (child of Master).
    pub const UI: GroupId = GroupId(3);
    /// Voice chat (child of Master; used from milestone M2).
    pub const VOICE: GroupId = GroupId(4);
    /// The built-in groups in creation order.
    pub const BUILT_IN: [GroupId; 5] =
        [Self::MASTER, Self::MUSIC, Self::SFX, Self::UI, Self::VOICE];

    /// True for the five built-in groups, which cannot be removed.
    pub fn is_built_in(self) -> bool {
        (self.0 as usize) < Self::BUILT_IN.len()
    }
}

/// Handle to an effect inserted on a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectId(pub(crate) u32);

/// How a positional sound's volume falls with distance (Roblox `Enum.RollOffMode`).
///
/// With `d` the listener distance (studs, times `SpatialSettings::rolloff_scale`), `min` and
/// `max` the sound's `RollOffMinDistance`/`RollOffMaxDistance`: every mode gives full volume for
/// `d <= min` and silence for `d >= max`. Between them see [`crate::roll_off_gain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum RollOffMode {
    /// `min / d`: the physical inverse-distance law (default, like Roblox).
    #[default]
    Inverse,
    /// `(max - d) / (max - min)`: a straight line from full volume to silence.
    Linear,
    /// The Linear value squared: quieter mid-range, gentle near `max`.
    LinearSquare,
    /// The lower of Inverse and LinearSquare: inverse up close, tapering smoothly to silence.
    InverseTapered,
}

/// Distance roll-off of one sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RollOff {
    /// Curve shape (`RollOffMode`).
    pub mode: RollOffMode,
    /// Distance in studs up to which the sound plays at full volume (`RollOffMinDistance`).
    pub min_distance: f32,
    /// Distance in studs from which the sound is silent (`RollOffMaxDistance`).
    pub max_distance: f32,
}

impl Default for RollOff {
    fn default() -> Self {
        Self {
            mode: RollOffMode::Inverse,
            min_distance: 10.0,
            max_distance: 10_000.0,
        }
    }
}

/// Where a positional sound is: the world position and velocity of its parent part.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Emitter {
    /// World position in studs.
    pub position: Vec3,
    /// World velocity in studs per second (drives Doppler; zero disables it for this sound).
    pub velocity: Vec3,
}

impl Emitter {
    /// A still emitter at `position`.
    pub fn at(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
        }
    }
}

/// The ears of the local player: normally the camera, updated every frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    /// World position in studs.
    pub position: Vec3,
    /// Look direction (Roblox `CFrame.LookVector`); need not be normalised.
    pub forward: Vec3,
    /// Up direction (`CFrame.UpVector`); need not be exactly orthogonal to `forward`.
    pub up: Vec3,
    /// World velocity in studs per second (Doppler).
    pub velocity: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            forward: Vec3::NEG_Z,
            up: Vec3::Y,
            velocity: Vec3::ZERO,
        }
    }
}

/// World-wide spatial settings (Roblox `SoundService` plus our occlusion and rear-filter knobs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialSettings {
    /// Studs per metre (`DistanceFactor`, default 3.33); sets the speed of sound for Doppler.
    pub distance_factor: f32,
    /// Doppler strength (`DopplerScale`, default 1; 0 disables Doppler).
    pub doppler_scale: f32,
    /// Distance multiplier applied before roll-off (`RolloffScale`, default 1; 2 = fades twice
    /// as fast).
    pub rolloff_scale: f32,
    /// Gain of a fully occluded sound (occlusion 1). Default 0.35 (about -9 dB).
    pub occlusion_gain: f32,
    /// Low-pass cutoff in Hz of a fully occluded sound. Default 1,200 Hz (muffled through a wall).
    pub occlusion_cutoff_hz: f32,
    /// Low-pass cutoff in Hz for a sound directly behind the listener. Default 9,000 Hz: a mild
    /// front/back cue that plain stereo panning lacks.
    pub rear_cutoff_hz: f32,
}

impl Default for SpatialSettings {
    fn default() -> Self {
        Self {
            distance_factor: 3.33,
            doppler_scale: 1.0,
            rolloff_scale: 1.0,
            occlusion_gain: 0.35,
            occlusion_cutoff_hz: 1_200.0,
            rear_cutoff_hz: 9_000.0,
        }
    }
}

/// The properties of one sound, named after Roblox `Sound` properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundProperties {
    /// `Volume`: linear gain, clamped to 0..=10. Default 0.5 like Roblox.
    pub volume: f32,
    /// `PlaybackSpeed`: rate and pitch multiplier, clamped to 0..=20. Default 1.
    pub playback_speed: f32,
    /// `Looped`: restart at the end instead of stopping.
    pub looped: bool,
    /// `Playing`: start playing as soon as the sound is created.
    pub playing: bool,
    /// `TimePosition`: start position in seconds.
    pub time_position: f64,
    /// `RollOffMode`.
    pub roll_off_mode: RollOffMode,
    /// `RollOffMinDistance` in studs. Default 10.
    pub roll_off_min_distance: f32,
    /// `RollOffMaxDistance` in studs. Default 10,000.
    pub roll_off_max_distance: f32,
    /// `SoundGroup`. Default [`GroupId::SFX`] (assistant's call: Roblox sends group-less sounds
    /// straight to the master; we route them through SFX so a player's "effects volume" covers
    /// them).
    pub sound_group: GroupId,
    /// Voice-limit priority (not a Roblox property): when more sounds play than the engine has
    /// voices, the highest priorities are heard. Default 0.
    pub priority: i32,
    /// Position of the sound in the world. `None` = not positional (a `Sound` parented to a
    /// service or a GUI): heard at its volume, unpanned.
    pub emitter: Option<Emitter>,
}

impl Default for SoundProperties {
    fn default() -> Self {
        let roll_off = RollOff::default();
        Self {
            volume: 0.5,
            playback_speed: 1.0,
            looped: false,
            playing: false,
            time_position: 0.0,
            roll_off_mode: roll_off.mode,
            roll_off_min_distance: roll_off.min_distance,
            roll_off_max_distance: roll_off.max_distance,
            sound_group: GroupId::SFX,
            priority: 0,
            emitter: None,
        }
    }
}

impl SoundProperties {
    /// The roll-off part of the properties.
    pub fn roll_off(&self) -> RollOff {
        RollOff {
            mode: self.roll_off_mode,
            min_distance: self.roll_off_min_distance,
            max_distance: self.roll_off_max_distance,
        }
    }
}

/// Something the audio thread reports back; collect with
/// [`AudioEngine::drain_events`](crate::AudioEngine::drain_events) after
/// [`AudioEngine::update`](crate::AudioEngine::update).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundEvent {
    /// The sound reached its end and stopped (`Sound.Ended`).
    Ended(SoundId),
    /// A looped sound wrapped around (`Sound.DidLoop`).
    DidLoop {
        /// The sound that looped.
        sound: SoundId,
        /// Total loops since the sound was created.
        count: u32,
    },
}

/// Mixer load figures, refreshed every mix block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MixStats {
    /// Sounds that are playing and mixed through a real voice.
    pub real_voices: u32,
    /// Sounds that are playing but over the voice limit: time advances, nothing is mixed.
    pub virtual_voices: u32,
    /// Frames rendered since the engine started.
    pub frames_rendered: u64,
}

/// Engine construction settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
    /// Output rate of the headless backend in Hz (a device uses its own rate). Default 48,000.
    pub sample_rate: u32,
    /// Most sounds mixed at once; quieter or lower-priority ones become virtual. Default 64.
    pub max_voices: usize,
    /// Most sounds that may exist at once (playing or not). Default 1,024.
    pub max_sounds: usize,
    /// Most mixer groups including the five built-in ones. Default 32.
    pub max_groups: usize,
    /// Most effects on one group. Default 8.
    pub max_effects_per_group: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            max_voices: 64,
            max_sounds: 1_024,
            max_groups: 32,
            max_effects_per_group: 8,
        }
    }
}
