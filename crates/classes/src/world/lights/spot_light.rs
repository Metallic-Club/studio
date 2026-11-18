//! `SpotLight`: a cone of light, like a torch or a stage light.
//!
//! The worked example of `docs/architecture.md` ("Adding a class"). Everything every light has (`Enabled`,
//! `Brightness`, `Color`, `Range`, `Shadows`, `CFrame`) comes from `Light`; this file adds only the cone's `Angle`
//! and names the component that draws it.

use metallic_core::{ClassDesc, ComponentDesc, PropFlags, PropMeta, Widget};

use super::light::{CATEGORY, EMITTER_REQUIRES, GROUP, orientation_get, orientation_set};

/// The component that lights a cone. Declared here; the renderer implements it.
pub static COMPONENT: ComponentDesc = ComponentDesc::new("SpotLightEmitter")
    .describe("Lights a cone that opens Angle degrees along the light's forward direction (-Z), up to Range metres.")
    .requires(EMITTER_REQUIRES);

static PROPS: &[PropMeta] = &[
    PropMeta::vector3("Orientation", 0.0, 0.0, 0.0)
        // As a part's (#833): scripts call it Orientation.
        .label("Rotation")
        .computed(orientation_get, Some(orientation_set))
        .derived_from(&["CFrame"])
        .transform()
        .flags(PropFlags::NO_SAVE)
        .group("Transform")
        .order(2)
        .desc(
            "Which way the light points, as angles in degrees about X, Y and Z (applied Z, then X, then Y). Inside a \
             part it is relative to the part.",
        ),
    PropMeta::number("Angle", 90.0)
        .range(1.0, 180.0)
        .widget(Widget::Slider)
        .visual()
        .group(GROUP)
        .order(3)
        .desc("How wide the cone is: the full angle from one edge to the other, in degrees, up to 180 (a half sphere)."),
];

/// A cone of light.
pub static CLASS: ClassDesc = ClassDesc::new("SpotLight", Some("Light"))
    .describe("A cone of light shining forward from the light, like a torch or a stage light.")
    .category(CATEGORY)
    .creatable()
    .components(&["SpotLightEmitter"])
    .props(PROPS);
