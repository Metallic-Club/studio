//! `SpotLight`: a cone of light, like a torch or a stage light.
//!
//! The worked example of `docs/architecture.md` ("Adding a class"). Everything every light has (`Enabled`,
//! `Brightness`, `Color`, `Range`, `Shadows`, `CFrame`) comes from `Light`; this file adds only the cone's `Angle`
//! and names the component that draws it.

use metallic_core::{ClassDesc, ComponentDesc, PropMeta, Widget};

use super::light::{CATEGORY, EMITTER_REQUIRES};

/// The component that lights a cone. Declared here; the renderer implements it.
pub static COMPONENT: ComponentDesc = ComponentDesc::new("SpotLightEmitter")
    .describe("Lights a cone that opens Angle degrees along the light's forward direction (-Z), up to Range metres.")
    .requires(EMITTER_REQUIRES);

static PROPS: &[PropMeta] = &[PropMeta::number("Angle", 90.0)
    .range(1.0, 179.0)
    .widget(Widget::Slider)
    .visual()
    .group("Spotlight")
    .order(0)
    .desc("How wide the cone is: the full angle from one edge to the other, in degrees.")];

/// A cone of light.
pub static CLASS: ClassDesc = ClassDesc::new("SpotLight", Some("Light"))
    .describe("A cone of light shining forward from the light, like a torch or a stage light.")
    .category(CATEGORY)
    .creatable()
    .components(&["SpotLightEmitter"])
    .props(PROPS);
