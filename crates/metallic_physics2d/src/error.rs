//! Typed errors returned by the 2D physics world.
use crate::desc::{BodyId, JointId};
use std::fmt;

/// Everything that can go wrong when configuring or stepping a [`crate::PhysicsWorld2d`].
///
/// A call that returns an error leaves the world exactly as it was.
#[derive(Debug, Clone, PartialEq)]
pub enum Physics2dError {
    /// `step` was given a `dt` that is NaN, infinite or negative.
    InvalidDt(f64),
    /// A [`crate::WorldConfig`] or [`crate::CharacterConfig`] value is out of range.
    InvalidConfig(&'static str),
    /// A number in a body, collider or joint description is NaN or infinite.
    NonFinite {
        /// The body (or joint's first body) the value belongs to.
        body: BodyId,
        /// Which field was not finite.
        field: &'static str,
    },
    /// A shape cannot be built (zero size, too few points, degenerate polygon...).
    InvalidShape {
        /// The body the shape belongs to.
        body: BodyId,
        /// Why the shape was rejected.
        reason: &'static str,
    },
    /// Two bodies (or a body and a character) share one id.
    DuplicateBody(BodyId),
    /// Two joints share one id.
    DuplicateJoint(JointId),
    /// No body or character with this id exists.
    UnknownBody(BodyId),
    /// A joint names a body that does not exist, or joins a body to itself.
    InvalidJoint {
        /// The joint.
        joint: JointId,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// The operation needs a dynamic body (for example applying an impulse to a static one).
    NotDynamic(BodyId),
}

impl fmt::Display for Physics2dError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Physics2dError::InvalidDt(dt) => {
                write!(f, "time step must be finite and not negative, got {dt}")
            }
            Physics2dError::InvalidConfig(what) => write!(f, "invalid configuration: {what}"),
            Physics2dError::NonFinite { body, field } => {
                write!(f, "body {}: {field} is NaN or infinite", body.0)
            }
            Physics2dError::InvalidShape { body, reason } => {
                write!(f, "body {}: invalid shape: {reason}", body.0)
            }
            Physics2dError::DuplicateBody(id) => write!(f, "body id {} is used twice", id.0),
            Physics2dError::DuplicateJoint(id) => write!(f, "joint id {} is used twice", id.0),
            Physics2dError::UnknownBody(id) => write!(f, "no body with id {}", id.0),
            Physics2dError::InvalidJoint { joint, reason } => {
                write!(f, "joint {}: {reason}", joint.0)
            }
            Physics2dError::NotDynamic(id) => write!(f, "body {} is not dynamic", id.0),
        }
    }
}

impl std::error::Error for Physics2dError {}
