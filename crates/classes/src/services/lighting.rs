//! `Lighting`: the sun, the sky and the light that fills the scene.

use metallic_core::{ClassDesc, PropMeta, Widget};

static PROPS: &[PropMeta] = &[
    PropMeta::number("ClockTime", 14.0)
        .range(0.0, 24.0)
        .widget(Widget::Slider)
        .visual()
        .group("Sun")
        .order(0)
        .desc("The time of day in hours, which places the sun: 6 is sunrise, 12 is noon, 18 is sunset."),
    PropMeta::number("Brightness", 2.0)
        .range(0.0, 10.0)
        .visual()
        .group("Sun")
        .order(1)
        .desc("How strong the sunlight is. 0 turns the sun off."),
    PropMeta::color3("SkyColor", 0.53, 0.72, 0.95)
        .visual()
        .group("Sky")
        .order(0)
        .desc("The colour of the sky in the middle of the day."),
    PropMeta::color3("Ambient", 0.5, 0.5, 0.5)
        .visual()
        .group("Sky")
        .order(1)
        .desc("The colour of the light that reaches places the sun does not, such as shadows."),
];

/// The service that controls how the scene is lit.
pub static CLASS: ClassDesc = ClassDesc::new("Lighting", Some("Instance"))
    .describe("Controls how the scene is lit: the time of day, the sun, the sky colour and the ambient light.")
    .service(1)
    .persisted()
    .props(PROPS);
