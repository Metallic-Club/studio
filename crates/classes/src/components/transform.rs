//! `Transform`: the component of every class that has its own place in the world (parts, lights, cameras).
//!
//! It brings no properties. Each class declares its own `CFrame`, because the meaning differs: a part's is in world
//! space, a light's is relative to the part it sits in. The component is the name subsystems look for when they need
//! an object's world transform, and the one emitter and mesh components require.

use metallic_core::ComponentDesc;

/// The component that marks an object as placed in the world by its `CFrame`.
pub static COMPONENT: ComponentDesc = ComponentDesc::new("Transform").describe(
    "Places the object in the world from its CFrame property. The renderer, physics and audio read the object's \
     world position and rotation through it.",
);
