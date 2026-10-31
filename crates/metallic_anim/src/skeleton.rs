//! Skeletons (3D joints and 2D bones), poses, masks and pose blending.
//!
//! One model serves 2D, 3D and everything in between: a 2D bone is a joint whose rotation is
//! about +Z and whose translation has `z = 0` (see [`Transform::from_2d`]). A 2.5D rig simply
//! mixes both.

use crate::AnimError;
use glam::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Translation, rotation and (possibly non-uniform) scale, applied scale first.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// Position.
    pub translation: Vec3,
    /// Orientation.
    pub rotation: Quat,
    /// Scale per axis.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// No translation, rotation or scale.
    pub const IDENTITY: Transform = Transform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// A pure translation.
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    /// Translation and rotation.
    pub fn from_translation_rotation(translation: Vec3, rotation: Quat) -> Self {
        Self {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// A 2D transform: position in the XY plane and an angle in radians about +Z
    /// (counter-clockwise when +Y is up).
    pub fn from_2d(position: Vec2, angle: f32, scale: Vec2) -> Self {
        Self {
            translation: position.extend(0.0),
            rotation: Quat::from_rotation_z(angle),
            scale: scale.extend(1.0),
        }
    }

    /// The 2D view of this transform: XY position, angle about +Z, XY scale.
    pub fn to_2d(&self) -> (Vec2, f32, Vec2) {
        let x_axis = self.rotation * Vec3::X;
        (
            self.translation.truncate(),
            x_axis.y.atan2(x_axis.x),
            self.scale.truncate(),
        )
    }

    /// `self * child`: the child's transform expressed in this transform's parent space.
    /// Exact for uniform scale; for non-uniform scale, shear is dropped (the usual game-engine
    /// approximation).
    pub fn mul_transform(&self, child: &Transform) -> Transform {
        Transform {
            translation: self.translation + self.rotation * (self.scale * child.translation),
            rotation: (self.rotation * child.rotation).normalize(),
            scale: self.scale * child.scale,
        }
    }

    /// Transforms a point.
    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        self.translation + self.rotation * (self.scale * p)
    }

    /// The 4x4 matrix (for renderers).
    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Blend towards `other` by `t`: lerp translation and scale, slerp rotation.
    pub fn lerp(&self, other: &Transform, t: f32) -> Transform {
        Transform {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

/// Index of a joint inside its [`Skeleton`].
pub type JointId = usize;

/// One joint (3D) or bone (2D).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Joint {
    /// Unique name, e.g. `"LeftUpperArm"`.
    pub name: String,
    /// Parent joint index; `None` for a root. Must be smaller than this joint's own index.
    pub parent: Option<JointId>,
    /// Rest (bind) transform relative to the parent.
    pub rest: Transform,
    /// Bone length along the local +X axis for 2D bones / drawing; `0` when unused.
    #[serde(default)]
    pub length: f32,
}

/// A joint hierarchy. Parents always come before their children, so a single forward pass
/// computes model-space transforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SkeletonData", into = "SkeletonData")]
pub struct Skeleton {
    joints: Vec<Joint>,
    by_name: HashMap<String, JointId>,
}

/// Serialised form of a [`Skeleton`]: just the joints (the name index is rebuilt on load).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkeletonData {
    joints: Vec<Joint>,
}

impl TryFrom<SkeletonData> for Skeleton {
    type Error = AnimError;
    fn try_from(data: SkeletonData) -> Result<Self, AnimError> {
        Skeleton::new(data.joints)
    }
}

impl From<Skeleton> for SkeletonData {
    fn from(s: Skeleton) -> Self {
        SkeletonData { joints: s.joints }
    }
}

impl Skeleton {
    /// Builds a skeleton.
    ///
    /// # Errors
    /// [`AnimError::InvalidSkeleton`] if a name repeats or a parent index is not smaller than its
    /// child's index.
    pub fn new(joints: Vec<Joint>) -> Result<Self, AnimError> {
        let mut by_name = HashMap::with_capacity(joints.len());
        for (i, j) in joints.iter().enumerate() {
            if let Some(p) = j.parent
                && p >= i
            {
                return Err(AnimError::InvalidSkeleton(format!(
                    "joint `{}` (index {i}) has parent {p}; parents must come first",
                    j.name
                )));
            }
            if by_name.insert(j.name.clone(), i).is_some() {
                return Err(AnimError::InvalidSkeleton(format!(
                    "joint name `{}` repeats",
                    j.name
                )));
            }
        }
        Ok(Self { joints, by_name })
    }

    /// The joints, parents first.
    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    /// Number of joints.
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    /// True when there are no joints.
    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    /// Joint index by name.
    pub fn find(&self, name: &str) -> Option<JointId> {
        self.by_name.get(name).copied()
    }

    /// Parent of a joint.
    pub fn parent(&self, joint: JointId) -> Option<JointId> {
        self.joints.get(joint).and_then(|j| j.parent)
    }

    /// True when `joint` is `ancestor` or below it.
    pub fn is_descendant_of(&self, mut joint: JointId, ancestor: JointId) -> bool {
        loop {
            if joint == ancestor {
                return true;
            }
            match self.parent(joint) {
                Some(p) => joint = p,
                None => return false,
            }
        }
    }
}

/// Builds a 2D bone rig: each bone is placed at the end of its parent (along the parent's +X)
/// unless an explicit offset is given.
#[derive(Debug, Default)]
pub struct Bone2dBuilder {
    joints: Vec<Joint>,
}

impl Bone2dBuilder {
    /// An empty rig.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a root bone at `position` with `angle` (radians) and `length`. Returns its index.
    pub fn root(&mut self, name: &str, position: Vec2, angle: f32, length: f32) -> JointId {
        self.joints.push(Joint {
            name: name.to_owned(),
            parent: None,
            rest: Transform::from_2d(position, angle, Vec2::ONE),
            length,
        });
        self.joints.len() - 1
    }

    /// Adds a bone at the tip of `parent`, rotated by `angle` relative to it.
    pub fn child(&mut self, name: &str, parent: JointId, angle: f32, length: f32) -> JointId {
        let tip = self.joints.get(parent).map_or(0.0, |p| p.length);
        self.child_at(name, parent, Vec2::new(tip, 0.0), angle, length)
    }

    /// Adds a bone at `offset` in the parent's local space.
    pub fn child_at(
        &mut self,
        name: &str,
        parent: JointId,
        offset: Vec2,
        angle: f32,
        length: f32,
    ) -> JointId {
        self.joints.push(Joint {
            name: name.to_owned(),
            parent: Some(parent),
            rest: Transform::from_2d(offset, angle, Vec2::ONE),
            length,
        });
        self.joints.len() - 1
    }

    /// Finishes the rig.
    ///
    /// # Errors
    /// [`AnimError::InvalidSkeleton`] on repeated names.
    pub fn build(self) -> Result<Skeleton, AnimError> {
        Skeleton::new(self.joints)
    }
}

/// Per-joint weights in `[0, 1]`: which joints an animation layer may touch.
/// For example "upper body only" for a wave that plays while running.
#[derive(Debug, Clone, PartialEq)]
pub struct JointMask {
    weights: Vec<f32>,
}

impl JointMask {
    /// Every joint at weight `w`.
    pub fn uniform(skeleton: &Skeleton, w: f32) -> Self {
        Self {
            weights: vec![w.clamp(0.0, 1.0); skeleton.len()],
        }
    }

    /// `root` and all joints below it at `w`, everything else at `0`.
    ///
    /// # Errors
    /// [`AnimError::UnknownJoint`] if `root` is not in the skeleton.
    pub fn subtree(skeleton: &Skeleton, root: &str, w: f32) -> Result<Self, AnimError> {
        let r = skeleton
            .find(root)
            .ok_or_else(|| AnimError::UnknownJoint(root.to_owned()))?;
        let w = w.clamp(0.0, 1.0);
        let weights = (0..skeleton.len())
            .map(|j| {
                if skeleton.is_descendant_of(j, r) {
                    w
                } else {
                    0.0
                }
            })
            .collect();
        Ok(Self { weights })
    }

    /// Weight of one joint (`0` if out of range).
    pub fn weight(&self, joint: JointId) -> f32 {
        self.weights.get(joint).copied().unwrap_or(0.0)
    }

    /// Sets the weight of one joint.
    pub fn set_weight(&mut self, joint: JointId, w: f32) {
        if let Some(slot) = self.weights.get_mut(joint) {
            *slot = w.clamp(0.0, 1.0);
        }
    }
}

/// Local transforms for every joint of a skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    /// Local transform per joint, same order as the skeleton.
    pub local: Vec<Transform>,
}

impl Pose {
    /// The rest pose.
    pub fn rest(skeleton: &Skeleton) -> Self {
        Self {
            local: skeleton.joints().iter().map(|j| j.rest).collect(),
        }
    }

    /// Resets every joint to rest (reuses the allocation).
    pub fn reset_to_rest(&mut self, skeleton: &Skeleton) {
        self.local.clear();
        self.local.extend(skeleton.joints().iter().map(|j| j.rest));
    }

    /// Model-space transforms (root of the model = identity), written into `out`.
    pub fn model_space(&self, skeleton: &Skeleton, out: &mut Vec<Transform>) {
        out.clear();
        for (i, joint) in skeleton.joints().iter().enumerate() {
            let local = self.local.get(i).copied().unwrap_or(joint.rest);
            let model = match joint.parent.and_then(|p| out.get(p)) {
                Some(parent) => parent.mul_transform(&local),
                None => local,
            };
            out.push(model);
        }
    }

    /// `self = lerp(self, other, w * mask)` per joint.
    pub fn blend_toward(&mut self, other: &Pose, w: f32, mask: Option<&JointMask>) {
        for (i, (a, b)) in self.local.iter_mut().zip(&other.local).enumerate() {
            let jw = w * mask.map_or(1.0, |m| m.weight(i));
            if jw <= 0.0 {
                continue;
            }
            *a = if jw >= 1.0 { *b } else { a.lerp(b, jw) };
        }
    }

    /// Blends two poses into `out`: `lerp(a, b, w * mask)` per joint.
    pub fn blend(a: &Pose, b: &Pose, w: f32, mask: Option<&JointMask>, out: &mut Pose) {
        out.local.clear();
        out.local.extend_from_slice(&a.local);
        out.blend_toward(b, w, mask);
    }

    /// The difference `self - reference` per joint, for additive layers: translation difference,
    /// relative rotation `reference^-1 * self`, and scale ratio.
    pub fn additive_delta(&self, reference: &Pose) -> Pose {
        let local = self
            .local
            .iter()
            .zip(&reference.local)
            .map(|(p, r)| Transform {
                translation: p.translation - r.translation,
                rotation: (r.rotation.inverse() * p.rotation).normalize(),
                scale: p.scale / r.scale.max(Vec3::splat(1e-6)),
            })
            .collect();
        Pose { local }
    }

    /// Adds an additive `delta` (from [`Pose::additive_delta`]) on top of this pose with weight
    /// `w * mask`. Weight `1` adds the full difference; `0.5` half of it.
    pub fn add_additive(&mut self, delta: &Pose, w: f32, mask: Option<&JointMask>) {
        for (i, (p, d)) in self.local.iter_mut().zip(&delta.local).enumerate() {
            let jw = w * mask.map_or(1.0, |m| m.weight(i));
            if jw == 0.0 {
                continue;
            }
            p.translation += d.translation * jw;
            p.rotation = (p.rotation * Quat::IDENTITY.slerp(d.rotation, jw)).normalize();
            p.scale *= Vec3::ONE.lerp(d.scale, jw);
        }
    }
}
