//! Inverse kinematics on poses: analytic two-bone IK (arms, legs) and look-at (heads, eyes,
//! turrets). Both work in 2D and 3D and take a weight so they can fade in and out.

use crate::AnimError;
use crate::skeleton::{JointId, Pose, Skeleton, Transform};
use glam::{Quat, Vec3};

/// Settings for [`solve_two_bone`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwoBoneIk {
    /// First joint of the chain (shoulder / hip).
    pub root: JointId,
    /// Middle joint (elbow / knee).
    pub mid: JointId,
    /// End joint (wrist / ankle). Must be a child of `mid`, which must be a child of `root`.
    pub end: JointId,
    /// Model-space point the end joint should reach.
    pub target: Vec3,
    /// Model-space point the middle joint should bend towards (in front of a knee, behind an
    /// elbow). For 2D rigs, put it in the XY plane on the side the joint should bend.
    pub pole: Vec3,
    /// `0` leaves the pose untouched, `1` applies the full solve.
    pub weight: f32,
}

/// Rotation that turns direction `from` onto direction `to` (shortest arc).
fn arc(from: Vec3, to: Vec3) -> Quat {
    let (Some(a), Some(b)) = (from.try_normalize(), to.try_normalize()) else {
        return Quat::IDENTITY;
    };
    Quat::from_rotation_arc(a, b)
}

/// Applies a model-space rotation `delta` to joint `joint` (rotating it about its own origin),
/// converting it into the joint's local rotation. `model` must be the pose's model-space
/// transforms; it is not updated.
fn rotate_joint_model(
    pose: &mut Pose,
    skeleton: &Skeleton,
    model: &[Transform],
    joint: JointId,
    delta: Quat,
) {
    let (Some(local), Some(m)) = (pose.local.get(joint), model.get(joint)) else {
        return;
    };
    let parent_rot = skeleton
        .parent(joint)
        .and_then(|p| model.get(p))
        .map_or(Quat::IDENTITY, |p| p.rotation);
    let new_model_rot = delta * m.rotation;
    let new_local = (parent_rot.inverse() * new_model_rot).normalize();
    let local = Transform {
        rotation: new_local,
        ..*local
    };
    if let Some(slot) = pose.local.get_mut(joint) {
        *slot = local;
    }
}

/// Bends a three-joint chain so the end reaches `target` (or points straight at it when out of
/// reach). Only the rotations of `root` and `mid` change; bone lengths are kept.
///
/// # Errors
/// [`AnimError::InvalidChain`] when the joints are not a parent chain `root -> mid -> end`.
pub fn solve_two_bone(
    skeleton: &Skeleton,
    pose: &mut Pose,
    ik: &TwoBoneIk,
) -> Result<(), AnimError> {
    if skeleton.parent(ik.end) != Some(ik.mid) || skeleton.parent(ik.mid) != Some(ik.root) {
        return Err(AnimError::InvalidChain);
    }
    let w = ik.weight.clamp(0.0, 1.0);
    if w <= 0.0 {
        return Ok(());
    }
    let mut model = Vec::with_capacity(skeleton.len());
    pose.model_space(skeleton, &mut model);
    let (Some(a), Some(b), Some(c)) = (
        model.get(ik.root).copied(),
        model.get(ik.mid).copied(),
        model.get(ik.end).copied(),
    ) else {
        return Err(AnimError::InvalidChain);
    };
    let (a, b, c) = (a.translation, b.translation, c.translation);
    let l1 = (b - a).length();
    let l2 = (c - b).length();
    if l1 < 1e-6 || l2 < 1e-6 {
        return Ok(());
    }
    let to_target = ik.target - a;
    let min_reach = (l1 - l2).abs() + 1e-4;
    let max_reach = l1 + l2 - 1e-4;
    let dist = to_target.length().clamp(min_reach, max_reach);
    let Some(dir) = to_target
        .try_normalize()
        .or_else(|| (c - a).try_normalize())
    else {
        return Ok(());
    };

    // Bend direction: the pole's offset from the root->target line, or the current bend.
    let bend_from = |p: Vec3| {
        let off = p - a;
        (off - dir * off.dot(dir)).try_normalize()
    };
    let bend = bend_from(ik.pole)
        .or_else(|| bend_from(b))
        .unwrap_or_else(|| dir.any_orthonormal_vector());

    // Law of cosines: angle at the root between the target line and the upper bone.
    let cos_a = ((l1 * l1 + dist * dist - l2 * l2) / (2.0 * l1 * dist)).clamp(-1.0, 1.0);
    let sin_a = (1.0 - cos_a * cos_a).max(0.0).sqrt();
    let want_mid = a + (dir * cos_a + bend * sin_a) * l1;
    let want_end = a + dir * dist;

    let q_root = Quat::IDENTITY.slerp(arc(b - a, want_mid - a), w);
    rotate_joint_model(pose, skeleton, &model, ik.root, q_root);
    // Recompute, then aim the lower bone.
    pose.model_space(skeleton, &mut model);
    let (Some(b1), Some(c1)) = (model.get(ik.mid).copied(), model.get(ik.end).copied()) else {
        return Ok(());
    };
    let goal_end = c1.translation.lerp(want_end, w);
    let q_mid = arc(c1.translation - b1.translation, goal_end - b1.translation);
    rotate_joint_model(pose, skeleton, &model, ik.mid, q_mid);
    Ok(())
}

/// Settings for [`look_at`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookAt {
    /// Joint to turn.
    pub joint: JointId,
    /// Model-space point to face.
    pub target: Vec3,
    /// The joint's local axis that should point at the target (e.g. `-Z` for a head facing
    /// forward, `+X` for a 2D bone).
    pub forward: Vec3,
    /// Largest turn away from the animated pose, in radians (`PI` = no limit).
    pub max_angle: f32,
    /// `0` leaves the pose untouched, `1` applies the full turn.
    pub weight: f32,
}

/// Turns one joint so its `forward` axis points at `target`, limited by `max_angle`.
pub fn look_at(skeleton: &Skeleton, pose: &mut Pose, settings: &LookAt) {
    let w = settings.weight.clamp(0.0, 1.0);
    if w <= 0.0 {
        return;
    }
    let mut model = Vec::with_capacity(skeleton.len());
    pose.model_space(skeleton, &mut model);
    let Some(m) = model.get(settings.joint).copied() else {
        return;
    };
    let current = m.rotation * settings.forward;
    let wanted = settings.target - m.translation;
    let full = arc(current, wanted);
    let (axis, angle) = full.to_axis_angle();
    let limited = if angle > settings.max_angle && axis.is_finite() {
        Quat::from_axis_angle(axis, settings.max_angle.max(0.0))
    } else {
        full
    };
    rotate_joint_model(
        pose,
        skeleton,
        &model,
        settings.joint,
        Quat::IDENTITY.slerp(limited, w),
    );
}
