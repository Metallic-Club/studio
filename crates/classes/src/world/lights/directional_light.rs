//! `DirectionalLight`: parallel light from far away, like the sun.
//!
//! It lights everything, however far, so it hides the `Range` it inherits from `Light`. Hiding keeps the slot: the
//! property's [`PropId`](metallic_core::PropId) is the same as in every other light, but scripts and Studio no longer
//! see it by name. Only the light's rotation matters; its position does not.

use metallic_core::{ClassDesc, ComponentDesc};

use super::light::{CATEGORY, EMITTER_REQUIRES};

/// The component that lights the whole world from one direction. Declared here; the renderer implements it.
pub static COMPONENT: ComponentDesc = ComponentDesc::new("DirectionalLightEmitter")
    .describe("Lights the whole world with parallel rays along the light's forward direction (-Z), however far.")
    .requires(EMITTER_REQUIRES);

/// Parallel light from one direction.
pub static CLASS: ClassDesc = ClassDesc::new("DirectionalLight", Some("Light"))
    .describe(
        "Parallel light from one direction over the whole world, like the sun. It points along its CFrame's forward \
         direction and has no range.",
    )
    .category(CATEGORY)
    .creatable()
    .hide(&["Range"])
    .components(&["DirectionalLightEmitter"]);
