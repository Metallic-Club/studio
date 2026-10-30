//! The joint hierarchy of a skinned model.
use crate::anim::Transform;
use glam::Mat4;

/// Joints of a skin, sorted so that every parent comes before its children (one pass from the
/// start computes every joint's model-space matrix).
#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    /// Joint names (glTF node names; VRM-humanoid names for the default avatar).
    pub names: Vec<String>,
    /// Parent joint of each joint; `None` for a root joint. Always a smaller index.
    pub parents: Vec<Option<usize>>,
    /// Rest pose: each joint's local transform relative to its parent.
    pub rest: Vec<Transform>,
    /// Inverse bind matrix of each joint (mesh space to joint space at bind time).
    pub inverse_bind: Vec<Mat4>,
    /// Model-space transform of the nodes above the root joints (an armature node, for example).
    pub base: Mat4,
}

impl Skeleton {
    /// Number of joints.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// True when the skeleton has no joints.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Index of the joint called `name`.
    pub fn joint(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
}
