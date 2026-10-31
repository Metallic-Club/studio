//! Clips: named, timed sets of property tracks plus event markers.

use crate::AnimError;
use crate::curve::{Curve, Interpolation, Keyframe, TangentMode};
use crate::value::{Animatable, Color3};
use glam::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Which part of a joint's local transform a track drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JointChannel {
    /// Local position (`Vec3` track; a `Vec2` track sets x and y and keeps z).
    Translation,
    /// Local rotation (`Quat` track; an `F32` track is a 2D angle in radians about +Z).
    Rotation,
    /// Local scale (`Vec3` track; a `Vec2` track sets x and y; an `F32` track scales uniformly).
    Scale,
}

impl JointChannel {
    /// Short label ("position", "rotation", "scale").
    pub fn label(self) -> &'static str {
        match self {
            JointChannel::Translation => "position",
            JointChannel::Rotation => "rotation",
            JointChannel::Scale => "scale",
        }
    }
}

/// What a track animates.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackTarget {
    /// One channel of a skeleton joint (3D) or bone (2D), by joint name.
    Joint {
        /// Joint name, as in the [`Skeleton`](crate::Skeleton).
        joint: String,
        /// Which channel.
        channel: JointChannel,
    },
    /// A named property of an object, for example `Workspace.Door` / `Transparency`.
    /// The engine resolves the path; this crate only samples the value.
    Property {
        /// Dotted path of the object, relative to the animated root.
        path: String,
        /// Property name.
        property: String,
    },
}

impl TrackTarget {
    /// A joint channel target.
    pub fn joint(joint: impl Into<String>, channel: JointChannel) -> Self {
        TrackTarget::Joint {
            joint: joint.into(),
            channel,
        }
    }

    /// A property target.
    pub fn property(path: impl Into<String>, property: impl Into<String>) -> Self {
        TrackTarget::Property {
            path: path.into(),
            property: property.into(),
        }
    }

    /// Human label for editors ("LeftArm rotation", "Door.Transparency").
    pub fn label(&self) -> String {
        match self {
            TrackTarget::Joint { joint, channel } => format!("{joint} {}", channel.label()),
            TrackTarget::Property { path, property } if path.is_empty() => property.clone(),
            TrackTarget::Property { path, property } => format!("{path}.{property}"),
        }
    }
}

/// A curve of one of the supported value types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Channel {
    /// Numbers (also 2D bone angles in radians).
    F32(Curve<f32>),
    /// 2D vectors.
    Vec2(Curve<Vec2>),
    /// 3D vectors.
    Vec3(Curve<Vec3>),
    /// Rotations.
    Quat(Curve<Quat>),
    /// Colours.
    Color3(Curve<Color3>),
    /// On/off switches; always step.
    Bool(Curve<bool>),
}

/// A sampled value of any channel type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChannelValue {
    /// A number.
    F32(f32),
    /// A 2D vector.
    Vec2(Vec2),
    /// A 3D vector.
    Vec3(Vec3),
    /// A rotation.
    Quat(Quat),
    /// A colour.
    Color3(Color3),
    /// A switch.
    Bool(bool),
}

/// Runs `$body` with `$c` bound to the inner `Curve<T>` of a [`Channel`], whatever `T` is.
#[macro_export]
macro_rules! with_curve {
    ($channel:expr, $c:ident => $body:expr) => {
        match $channel {
            $crate::Channel::F32($c) => $body,
            $crate::Channel::Vec2($c) => $body,
            $crate::Channel::Vec3($c) => $body,
            $crate::Channel::Quat($c) => $body,
            $crate::Channel::Color3($c) => $body,
            $crate::Channel::Bool($c) => $body,
        }
    };
}

impl Channel {
    /// Value at `time`, or `None` when the channel has no keys.
    pub fn sample(&self, time: f32) -> Option<ChannelValue> {
        match self {
            Channel::F32(c) => c.sample(time).map(ChannelValue::F32),
            Channel::Vec2(c) => c.sample(time).map(ChannelValue::Vec2),
            Channel::Vec3(c) => c.sample(time).map(ChannelValue::Vec3),
            Channel::Quat(c) => c.sample(time).map(ChannelValue::Quat),
            Channel::Color3(c) => c.sample(time).map(ChannelValue::Color3),
            Channel::Bool(c) => c.sample(time).map(ChannelValue::Bool),
        }
    }

    /// Number of keys.
    pub fn key_count(&self) -> usize {
        with_curve!(self, c => c.len())
    }

    /// Time of key `index`.
    pub fn key_time(&self, index: usize) -> Option<f32> {
        with_curve!(self, c => c.keys().get(index).map(|k| k.time))
    }

    /// All key times, in order.
    pub fn key_times(&self) -> Vec<f32> {
        with_curve!(self, c => c.keys().iter().map(|k| k.time).collect())
    }

    /// Interpolation of key `index`.
    pub fn key_interpolation(&self, index: usize) -> Option<Interpolation> {
        with_curve!(self, c => c.keys().get(index).map(|k| k.interpolation))
    }

    /// Tangent mode of key `index`.
    pub fn key_tangent_mode(&self, index: usize) -> Option<TangentMode> {
        with_curve!(self, c => c.keys().get(index).map(|k| k.tangent_mode))
    }

    /// Sets the interpolation of key `index` (no-op if out of range).
    pub fn set_key_interpolation(&mut self, index: usize, interpolation: Interpolation) {
        with_curve!(self, c => {
            if let Some(k) = c.key_mut(index) {
                k.interpolation = interpolation;
            }
        });
    }

    /// Sets the tangent mode of key `index` and recomputes handles.
    pub fn set_key_tangent_mode(&mut self, index: usize, mode: TangentMode) {
        with_curve!(self, c => {
            if let Some(k) = c.key_mut(index) {
                k.tangent_mode = mode;
            }
            c.recompute_tangents();
        });
    }

    /// Sets key times by index (unknown indices ignored), then sorts. Returns the old->new
    /// index map from [`Curve::sort`]. Negative times are clamped to `0`.
    pub fn set_key_times(&mut self, times: &[(usize, f32)]) -> Vec<usize> {
        with_curve!(self, c => {
            for &(i, t) in times {
                if let Some(k) = c.key_mut(i) {
                    k.time = if t.is_finite() { t.max(0.0) } else { k.time };
                }
            }
            c.sort()
        })
    }

    /// Removes key `index`.
    pub fn remove_key(&mut self, index: usize) {
        with_curve!(self, c => {
            c.remove(index);
        });
    }

    /// Adds a key at `time` holding the curve's current value there (no visible change).
    /// Returns the new key's index, or `None` when the channel is empty or the time is invalid.
    pub fn insert_key_at(&mut self, time: f32) -> Option<usize> {
        fn add<T: Animatable>(c: &mut Curve<T>, time: f32) -> Option<usize> {
            let v = c.sample(time)?;
            c.insert(Keyframe::new(time, v)).ok()
        }
        with_curve!(self, c => add(c, time))
    }

    /// Short type name for editors ("Number", "Vector3", ...).
    pub fn type_label(&self) -> &'static str {
        match self {
            Channel::F32(_) => "Number",
            Channel::Vec2(_) => "Vector2",
            Channel::Vec3(_) => "Vector3",
            Channel::Quat(_) => "Rotation",
            Channel::Color3(_) => "Color3",
            Channel::Bool(_) => "Bool",
        }
    }

    fn validate(&self) -> Result<(), AnimError> {
        with_curve!(self, c => c.validate())
    }
}

/// One animated property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// What it animates.
    pub target: TrackTarget,
    /// The keys.
    pub channel: Channel,
}

impl Track {
    /// A track.
    pub fn new(target: TrackTarget, channel: Channel) -> Self {
        Self { target, channel }
    }
}

/// A named moment in a clip that scripts can listen to (footsteps, hit frames, sounds).
/// Like Roblox's `KeyframeMarker` / `GetMarkerReachedSignal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventMarker {
    /// Time in seconds.
    pub time: f32,
    /// Event name, e.g. `"Footstep"`.
    pub name: String,
    /// Optional text passed to listeners.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
}

/// What happens when playback reaches the end of a clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LoopMode {
    /// Play once and hold the last frame.
    #[default]
    Once,
    /// Jump back to the start.
    Loop,
    /// Play backwards to the start, then forwards again.
    PingPong,
}

/// A named animation: tracks, markers and timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Clip name, e.g. `"Walk"`.
    pub name: String,
    /// Length in seconds. Keys after this are kept but not reached by playback.
    pub duration: f32,
    /// Frames per second used for snapping and display (sampling is continuous).
    #[serde(default = "default_frame_rate")]
    pub frame_rate: f32,
    /// End-of-clip behaviour.
    #[serde(default)]
    pub loop_mode: LoopMode,
    /// The animated properties.
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Named events, sorted by time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<EventMarker>,
}

fn default_frame_rate() -> f32 {
    30.0
}

impl Clip {
    /// An empty clip.
    pub fn new(name: impl Into<String>, duration: f32) -> Self {
        Self {
            name: name.into(),
            duration,
            frame_rate: default_frame_rate(),
            loop_mode: LoopMode::Once,
            tracks: Vec::new(),
            markers: Vec::new(),
        }
    }

    /// Checks durations, frame rate, key order and marker times.
    ///
    /// # Errors
    /// [`AnimError::InvalidClip`], [`AnimError::InvalidKeyTime`] or [`AnimError::UnsortedKeys`].
    pub fn validate(&self) -> Result<(), AnimError> {
        if !(self.duration.is_finite() && self.duration > 0.0) {
            return Err(AnimError::InvalidClip(format!(
                "duration must be > 0, got {}",
                self.duration
            )));
        }
        if !(self.frame_rate.is_finite() && self.frame_rate > 0.0) {
            return Err(AnimError::InvalidClip(format!(
                "frame_rate must be > 0, got {}",
                self.frame_rate
            )));
        }
        for track in &self.tracks {
            track.channel.validate()?;
        }
        for m in &self.markers {
            if !(m.time.is_finite() && m.time >= 0.0) {
                return Err(AnimError::InvalidKeyTime(m.time));
            }
        }
        Ok(())
    }

    /// Adds a track and returns `self`, for building clips in code.
    pub fn with_track(mut self, target: TrackTarget, channel: Channel) -> Self {
        self.tracks.push(Track::new(target, channel));
        self
    }

    /// Adds a marker (kept sorted) and returns `self`.
    pub fn with_marker(mut self, time: f32, name: impl Into<String>) -> Self {
        self.add_marker(EventMarker {
            time,
            name: name.into(),
            value: String::new(),
        });
        self
    }

    /// Adds a marker, keeping markers sorted by time.
    pub fn add_marker(&mut self, marker: EventMarker) {
        let i = self.markers.partition_point(|m| m.time <= marker.time);
        self.markers.insert(i, marker);
    }

    /// Maps playback time (seconds since start, may exceed the duration) to clip time according
    /// to the loop mode.
    pub fn local_time(&self, time: f32) -> f32 {
        let d = self.duration;
        if d <= 0.0 || !time.is_finite() {
            return 0.0;
        }
        match self.loop_mode {
            LoopMode::Once => time.clamp(0.0, d),
            LoopMode::Loop => time.rem_euclid(d),
            LoopMode::PingPong => {
                let p = time.rem_euclid(2.0 * d);
                if p <= d { p } else { 2.0 * d - p }
            }
        }
    }

    /// Snaps a time to the nearest frame.
    pub fn snap_to_frame(&self, time: f32) -> f32 {
        if self.frame_rate <= 0.0 {
            return time;
        }
        (time * self.frame_rate).round() / self.frame_rate
    }

    /// Markers crossed when playback moves forward from `from` (exclusive) to `to` (inclusive),
    /// both in playback time (not wrapped). Handles looping: a marker is reported once per pass.
    /// A marker at time 0 fires when playback starts (call with `from < 0`).
    pub fn markers_crossed(&self, from: f32, to: f32) -> Vec<&EventMarker> {
        let mut out = Vec::new();
        if to <= from || self.duration <= 0.0 || self.markers.is_empty() {
            return out;
        }
        let d = self.duration;
        match self.loop_mode {
            LoopMode::Once => {
                for m in &self.markers {
                    if m.time > from && m.time <= to && m.time <= d {
                        out.push(m);
                    }
                }
            }
            LoopMode::Loop | LoopMode::PingPong => {
                // Walk whole passes; cap the work for huge jumps.
                let first_pass = (from / d).floor().max(0.0) as i64;
                let last_pass = (to / d).floor() as i64;
                let passes = (last_pass - first_pass).clamp(0, 64);
                for p in first_pass..=first_pass + passes {
                    let base = p as f32 * d;
                    for m in &self.markers {
                        let local = if self.loop_mode == LoopMode::PingPong && p.rem_euclid(2) == 1
                        {
                            d - m.time
                        } else {
                            m.time
                        };
                        let t = base + local;
                        if t > from && t <= to {
                            out.push(m);
                        }
                    }
                }
            }
        }
        out
    }

    /// Latest key time over all tracks.
    pub fn last_key_time(&self) -> f32 {
        self.tracks
            .iter()
            .map(|t| with_curve!(&t.channel, c => c.end_time()))
            .fold(0.0, f32::max)
    }

    /// Finds a track by target.
    pub fn track(&self, target: &TrackTarget) -> Option<&Track> {
        self.tracks.iter().find(|t| &t.target == target)
    }

    /// Samples every [`TrackTarget::Property`] track at clip time `time`.
    pub fn sample_properties(&self, time: f32) -> Vec<(&str, &str, ChannelValue)> {
        self.tracks
            .iter()
            .filter_map(|t| match &t.target {
                TrackTarget::Property { path, property } => t
                    .channel
                    .sample(time)
                    .map(|v| (path.as_str(), property.as_str(), v)),
                TrackTarget::Joint { .. } => None,
            })
            .collect()
    }
}
