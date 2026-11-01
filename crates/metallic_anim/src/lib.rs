//! Animation for the Metallic Engine: one data model for 2D, 3D and everything between.
//!
//! Pure Rust, no GPU and no engine types: the renderer, physics and Studio read poses and
//! property values from here and apply them to the Instance tree themselves.
//!
//! | Piece | What it is |
//! |---|---|
//! | [`easing`] | Roblox's 11 [`EasingStyle`]s x 3 [`EasingDirection`]s. |
//! | [`Curve`] | Keyframes of `f32`, `Vec2`, `Vec3`, `Quat`, [`Color3`] or `bool`, with constant, linear, Bezier (graph-editor handles, [`TangentMode`]s) or eased segments. |
//! | [`Clip`] | Named tracks ([`TrackTarget`]: a joint channel or any object property) plus [`EventMarker`]s and a [`LoopMode`]. |
//! | [`Skeleton`], [`Pose`] | Joint hierarchy (3D joints or 2D bones via [`Bone2dBuilder`]), local transforms, [`JointMask`]s, crossfade and additive blending. |
//! | [`sample_clip`] | Writes a clip into a pose. Joint tracks are offsets from the rest pose (like Roblox `Pose`s). |
//! | [`Animator`] | Plays tracks with fades, weights, speeds, [`AnimationPriority`] layers, masks and additive tracks; reports markers. |
//! | [`StateMachine`] | States, parameters and conditional transitions with blend times; 1D blend states. |
//! | [`Tween`] | Roblox `TweenService` semantics: [`TweenInfo`] time, easing, repeat count, reverses, delay. |
//! | [`ik`] | Two-bone IK and look-at, both weighted. |
//! | [`root_motion_delta`] | Root motion extraction (planar, 2D horizontal or full). |
//! | [`format`] | Text files for clips, skeletons and state machines (RON, one key per line). |
//!
//! ```
//! use metallic_anim::{Clip, Channel, Curve, Keyframe, TrackTarget, JointChannel};
//! let wave = Curve::from_keys(vec![Keyframe::new(0.0, 0.0f32), Keyframe::new(0.5, 1.2), Keyframe::new(1.0, 0.0)])?;
//! let clip = Clip::new("Wave", 1.0)
//!     .with_track(TrackTarget::joint("RightArm", JointChannel::Rotation), Channel::F32(wave));
//! let text = metallic_anim::format::clip_to_text(&clip)?;
//! assert_eq!(metallic_anim::format::clip_from_text(&text)?, clip);
//! # Ok::<(), metallic_anim::AnimError>(())
//! ```
#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod animator;
mod clip;
mod curve;
pub mod easing;
pub mod format;
pub mod ik;
mod sampling;
mod skeleton;
mod state_machine;
mod tween;
mod value;

pub use animator::{
    AnimationPriority, Animator, AnimatorEvent, BlendMode, PlayOptions, TrackHandle,
};
pub use clip::{
    Channel, ChannelValue, Clip, EventMarker, JointChannel, LoopMode, Track, TrackTarget,
};
pub use curve::{Curve, Interpolation, KEY_TIME_EPSILON, Keyframe, Tangent, TangentMode};
pub use easing::{EasingDirection, EasingStyle, ease};
pub use sampling::{
    ClipBinding, RootMotionDelta, RootMotionMode, root_motion_delta, sample_clip, strip_root_motion,
};
pub use skeleton::{Bone2dBuilder, Joint, JointId, JointMask, Pose, Skeleton, Transform};
pub use state_machine::{
    Condition, Motion, MotionSample, Parameter, ParameterValue, State, StateMachine,
    StateMachinePlayer, Transition, blend_motion_samples,
};
pub use tween::{PlaybackState, Tween, TweenInfo, TweenSample};
pub use value::{Animatable, Color3, quat_pow};

/// Re-export of the math library used in the public API.
pub use glam;

/// Errors from this crate.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnimError {
    /// A key or marker time is negative or not a number.
    #[error("invalid key time {0}: times must be finite and >= 0")]
    InvalidKeyTime(f32),
    /// Keys are not in strictly increasing time order.
    #[error("keys are not sorted by time")]
    UnsortedKeys,
    /// A clip setting is invalid (message says which).
    #[error("invalid clip: {0}")]
    InvalidClip(String),
    /// The skeleton is malformed (message says why).
    #[error("invalid skeleton: {0}")]
    InvalidSkeleton(String),
    /// A joint name is not in the skeleton.
    #[error("unknown joint `{0}`")]
    UnknownJoint(String),
    /// IK joints are not a parent chain root -> mid -> end.
    #[error("IK joints must form a chain root -> mid -> end")]
    InvalidChain,
    /// A state machine names a state that does not exist.
    #[error("unknown state `{0}`")]
    UnknownState(String),
    /// A state machine names a clip that was not supplied.
    #[error("unknown clip `{0}`")]
    UnknownClip(String),
    /// A parameter name is not defined.
    #[error("unknown parameter `{0}`")]
    UnknownParameter(String),
    /// A parameter is used as the wrong kind (bool / float / trigger).
    #[error("parameter `{0}` has a different type")]
    ParameterType(String),
    /// The text could not be read or written.
    #[error("animation file format: {0}")]
    Format(String),
    /// The file was written by a newer version.
    #[error("unsupported animation file version {0}")]
    UnsupportedVersion(u32),
}
