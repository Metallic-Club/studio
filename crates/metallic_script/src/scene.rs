//! Scene queries the script API needs from the physics: `workspace:Raycast`.
//!
//! The script crate does not know the physics. The host that owns one (the runtime's `Game`)
//! gives the [`ScriptHost`](crate::ScriptHost) a [`SceneQuery`] with
//! [`set_scene_query`](crate::ScriptHost::set_scene_query); without one, `Raycast` finds nothing.

use metallic_core::{InstanceId, Vec3};

/// A ray with a filter, as `workspace:Raycast` builds it from `RaycastParams`.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneRay {
    /// Where the ray starts.
    pub origin: Vec3,
    /// Its direction; the length is the maximum distance in studs.
    pub direction: Vec3,
    /// True: only `parts` can be hit (`Enum.RaycastFilterType.Include`). False: `parts` are
    /// skipped (`Exclude`).
    pub include: bool,
    /// The parts of the filter, already expanded to the descendants of the listed instances.
    pub parts: Vec<InstanceId>,
    /// True: parts with `CanCollide = false` are not hit.
    pub respect_can_collide: bool,
}

/// What a ray hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneHit {
    /// The part that was hit.
    pub instance: InstanceId,
    /// Where.
    pub position: Vec3,
    /// The surface normal, unit length.
    pub normal: Vec3,
    /// Distance from the origin.
    pub distance: f64,
}

/// The physics, as far as scripts may ask it.
pub trait SceneQuery {
    /// The nearest part `ray` hits, if any.
    fn raycast(&self, ray: &SceneRay) -> Option<SceneHit>;
}
