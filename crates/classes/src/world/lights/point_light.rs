//! `PointLight`: light shining from one point in every direction, like a bulb.
//!
//! It adds nothing to `Light`: everything it needs (`Brightness`, `Color`, `Range`, ...) is inherited. What makes it
//! a point light is its component, `PointLightEmitter`, which the renderer implements (M3).

use metallic_core::{ClassDesc, ComponentDesc};

use super::light::{CATEGORY, EMITTER_REQUIRES};

/// The component that lights the world from a point. Declared here; the renderer implements it.
pub static COMPONENT: ComponentDesc = ComponentDesc::new("PointLightEmitter")
    .describe("Lights everything within Range of the light's position, equally in every direction.")
    .requires(EMITTER_REQUIRES);

/// A light shining in every direction from a point.
pub static CLASS: ClassDesc = ClassDesc::new("PointLight", Some("Light"))
    .describe("Light shining in every direction from one point, like a bulb or a torch. It reaches Range metres.")
    .category(CATEGORY)
    .creatable()
    .components(&["PointLightEmitter"]);
