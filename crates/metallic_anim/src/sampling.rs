//! Sampling clips into poses, and root motion.
//!
//! Joint tracks are **offsets from the rest pose**, like Roblox `Pose` CFrames: a translation
//! track adds to the rest position, a rotation track rotates after the rest rotation, a scale
//! track multiplies the rest scale. An empty clip therefore leaves a character in its rest
//! pose, and the same clip works on rigs with different proportions.

use crate::AnimError;
use crate::clip::{Channel, Clip, JointChannel, LoopMode, TrackTarget};
use crate::skeleton::{JointId, Pose, Skeleton, Transform};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
struct BoundTrack {
    track: usize,
    joint: JointId,
    channel: JointChannel,
}

/// A clip's joint tracks resolved to joint indices of one skeleton. Build once, sample often.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipBinding {
    bound: Vec<BoundTrack>,
    animated: Vec<bool>,
    unresolved: Vec<String>,
}

impl ClipBinding {
    /// Resolves the clip's joint tracks against `skeleton`. Tracks naming unknown joints are
    /// skipped and listed in [`ClipBinding::unresolved`] (a rig without a tail can still play a
    /// clip that has one).
    pub fn new(clip: &Clip, skeleton: &Skeleton) -> Self {
        let mut bound = Vec::new();
        let mut unresolved = Vec::new();
        let mut animated = vec![false; skeleton.len()];
        for (i, t) in clip.tracks.iter().enumerate() {
            if let TrackTarget::Joint { joint, channel } = &t.target {
                match skeleton.find(joint) {
                    Some(j) => {
                        bound.push(BoundTrack {
                            track: i,
                            joint: j,
                            channel: *channel,
                        });
                        animated[j] = true;
                    }
                    None => unresolved.push(joint.clone()),
                }
            }
        }
        Self {
            bound,
            animated,
            unresolved,
        }
    }

    /// Like [`ClipBinding::new`] but fails if any joint track names an unknown joint.
    ///
    /// # Errors
    /// [`AnimError::UnknownJoint`] with the first missing name.
    pub fn new_strict(clip: &Clip, skeleton: &Skeleton) -> Result<Self, AnimError> {
        let b = Self::new(clip, skeleton);
        match b.unresolved.first() {
            Some(name) => Err(AnimError::UnknownJoint(name.clone())),
            None => Ok(b),
        }
    }

    /// Joint names that the clip animates but the skeleton lacks.
    pub fn unresolved(&self) -> &[String] {
        &self.unresolved
    }

    /// True when the clip has at least one track on `joint`.
    pub fn animates(&self, joint: JointId) -> bool {
        self.animated.get(joint).copied().unwrap_or(false)
    }
}

/// Writes the clip's joint channels at clip time `time` into `pose`. Joints and channels the
/// clip does not animate keep their current values (call [`Pose::reset_to_rest`] first for a
/// clean sample). `skeleton` supplies rest transforms.
pub fn sample_clip(
    clip: &Clip,
    binding: &ClipBinding,
    skeleton: &Skeleton,
    time: f32,
    pose: &mut Pose,
) {
    let joints = skeleton.joints();
    for b in &binding.bound {
        let (Some(track), Some(joint), Some(local)) = (
            clip.tracks.get(b.track),
            joints.get(b.joint),
            pose.local.get_mut(b.joint),
        ) else {
            continue;
        };
        let rest = &joint.rest;
        match (b.channel, &track.channel) {
            (JointChannel::Translation, Channel::Vec3(c)) => {
                if let Some(v) = c.sample(time) {
                    local.translation = rest.translation + v;
                }
            }
            (JointChannel::Translation, Channel::Vec2(c)) => {
                if let Some(v) = c.sample(time) {
                    local.translation = rest.translation + v.extend(0.0);
                }
            }
            (JointChannel::Rotation, Channel::Quat(c)) => {
                if let Some(q) = c.sample(time) {
                    local.rotation = (rest.rotation * q).normalize();
                }
            }
            (JointChannel::Rotation, Channel::F32(c)) => {
                if let Some(a) = c.sample(time) {
                    local.rotation = (rest.rotation * Quat::from_rotation_z(a)).normalize();
                }
            }
            (JointChannel::Scale, Channel::Vec3(c)) => {
                if let Some(v) = c.sample(time) {
                    local.scale = rest.scale * v;
                }
            }
            (JointChannel::Scale, Channel::Vec2(c)) => {
                if let Some(v) = c.sample(time) {
                    local.scale = rest.scale * v.extend(1.0);
                }
            }
            (JointChannel::Scale, Channel::F32(c)) => {
                if let Some(v) = c.sample(time) {
                    local.scale = rest.scale * v;
                }
            }
            // Type/channel pairs that make no sense (a colour on a rotation) are ignored.
            _ => {}
        }
    }
}

/// Which parts of the root joint's motion move the character instead of the joint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RootMotionMode {
    /// The root joint moves inside the character; the character itself stays put.
    #[default]
    Off,
    /// Ground-plane motion for 3D: X and Z translation plus yaw (rotation about +Y).
    /// Height (Y) stays in the animation so jumps and bobbing still show.
    PlanarXz,
    /// Side-on motion for 2D: X translation only (Y stays in the animation).
    HorizontalX,
    /// All translation and rotation.
    Full,
}

/// Motion of the root joint between two playback times, to apply to the character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RootMotionDelta {
    /// Translation in the character's space.
    pub translation: Vec3,
    /// Rotation in the character's space.
    pub rotation: Quat,
}

fn root_offset(clip: &Clip, joint: &str, t: f32) -> (Vec3, Quat) {
    let mut pos = Vec3::ZERO;
    let mut rot = Quat::IDENTITY;
    for tr in &clip.tracks {
        if let TrackTarget::Joint { joint: j, channel } = &tr.target {
            if j != joint {
                continue;
            }
            match (channel, &tr.channel) {
                (JointChannel::Translation, Channel::Vec3(c)) => pos = c.sample(t).unwrap_or(pos),
                (JointChannel::Translation, Channel::Vec2(c)) => {
                    pos = c.sample(t).map_or(pos, |v| v.extend(0.0));
                }
                (JointChannel::Rotation, Channel::Quat(c)) => rot = c.sample(t).unwrap_or(rot),
                (JointChannel::Rotation, Channel::F32(c)) => {
                    rot = c.sample(t).map_or(rot, Quat::from_rotation_z);
                }
                _ => {}
            }
        }
    }
    (pos, rot)
}

fn mask_motion(mode: RootMotionMode, pos: Vec3, rot: Quat) -> (Vec3, Quat) {
    match mode {
        RootMotionMode::Off => (Vec3::ZERO, Quat::IDENTITY),
        RootMotionMode::PlanarXz => {
            let (yaw, _, _) = rot.to_euler(glam::EulerRot::YXZ);
            (Vec3::new(pos.x, 0.0, pos.z), Quat::from_rotation_y(yaw))
        }
        RootMotionMode::HorizontalX => (Vec3::new(pos.x, 0.0, 0.0), Quat::IDENTITY),
        RootMotionMode::Full => (pos, rot),
    }
}

/// How far the root joint `joint` travels between playback times `from` and `to` (seconds since
/// the clip started, not wrapped), keeping only the parts selected by `mode`.
///
/// For looping clips, every completed loop adds the motion of one whole cycle, so a walk cycle
/// keeps walking forward. Ping-pong clips report the motion of the current direction only.
pub fn root_motion_delta(
    clip: &Clip,
    joint: &str,
    mode: RootMotionMode,
    from: f32,
    to: f32,
) -> RootMotionDelta {
    let local = |t: f32| {
        let (p, r) = root_offset(clip, joint, clip.local_time(t));
        mask_motion(mode, p, r)
    };
    let (p0, r0) = local(from);
    let (p1, r1) = local(to);
    let mut translation = p1 - p0;
    let mut rotation = (r0.inverse() * r1).normalize();
    if clip.loop_mode == LoopMode::Loop && clip.duration > 0.0 {
        let cycles = (to / clip.duration).floor() - (from / clip.duration).floor();
        if cycles != 0.0 {
            let (ps, rs) = mask_motion(
                mode,
                root_offset(clip, joint, 0.0).0,
                root_offset(clip, joint, 0.0).1,
            );
            let (pe, re) = {
                let (p, r) = root_offset(clip, joint, clip.duration);
                mask_motion(mode, p, r)
            };
            translation += (pe - ps) * cycles;
            let cycle_rot = (rs.inverse() * re).normalize();
            rotation = (rotation * crate::value::quat_pow(cycle_rot, cycles)).normalize();
        }
    }
    RootMotionDelta {
        translation,
        rotation,
    }
}

/// Removes the root-motion parts selected by `mode` from `joint` in a sampled pose, so the
/// animation plays in place while [`root_motion_delta`] moves the character.
pub fn strip_root_motion(
    pose: &mut Pose,
    skeleton: &Skeleton,
    joint: JointId,
    mode: RootMotionMode,
) {
    let (Some(local), Some(j)) = (pose.local.get_mut(joint), skeleton.joints().get(joint)) else {
        return;
    };
    let rest: Transform = j.rest;
    match mode {
        RootMotionMode::Off => {}
        RootMotionMode::PlanarXz => {
            local.translation.x = rest.translation.x;
            local.translation.z = rest.translation.z;
            let rel = rest.rotation.inverse() * local.rotation;
            let (yaw, _, _) = rel.to_euler(glam::EulerRot::YXZ);
            local.rotation = (rest.rotation * Quat::from_rotation_y(-yaw) * rel).normalize();
        }
        RootMotionMode::HorizontalX => local.translation.x = rest.translation.x,
        RootMotionMode::Full => {
            local.translation = rest.translation;
            local.rotation = rest.rotation;
        }
    }
}
