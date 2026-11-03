//! `Light`: what every light has in common. The reference example of inheritance in this crate.
//!
//! `Light` declares the shared members once: `Enabled`, `Brightness`, `Color`, `Range`, `Shadows` and `CFrame`.
//! Each concrete light is a small file that names `Light` as its parent and adds only what is its own:
//!
//! | Class              | Adds                        | Component                 |
//! |--------------------|-----------------------------|---------------------------|
//! | `PointLight`       | nothing                     | `PointLightEmitter`       |
//! | `SpotLight`        | `Angle`                     | `SpotLightEmitter`        |
//! | `DirectionalLight` | nothing, and hides `Range`  | `DirectionalLightEmitter` |
//!
//! Inherited properties keep the [`PropId`](metallic_core::PropId) they have in `Light`, so code that reads
//! `Brightness` by id works for every light. Hiding `Range` in `DirectionalLight` keeps its slot, too.
//!
//! # Where a light is
//!
//! A light inside a `BasePart` follows that part: its `CFrame` is relative to the part, and the identity puts it at
//! the part's centre. Anywhere else (in a `Model`, a `Folder` or the `Workspace`) its `CFrame` is in world space.
//! [`world_cframe`] resolves the two cases; the renderer (M3) reads lights through it.

use metallic_core::{CFrame, ClassDesc, DataModel, DmError, Id, PropMeta, Value, Widget};

use crate::base::world_object::is_a_named;

/// The Insert menu group of every concrete light.
pub const CATEGORY: &str = "Lights";

/// The components every light emitter needs: it lights the world from where its `Transform` puts it.
pub const EMITTER_REQUIRES: &[&str] = &["Transform"];

/// The Properties panel group of the members every light shares.
pub const GROUP: &str = "Light";

static PROPS: &[PropMeta] = &[
    PropMeta::boolean("Enabled", true)
        .visual()
        .group(GROUP)
        .order(0)
        .desc("Whether the light shines. Turning it off keeps every other setting."),
    PropMeta::number("Brightness", 1.0)
        .range(0.0, 100.0)
        .widget(Widget::Slider)
        .visual()
        .group(GROUP)
        .order(1)
        .desc("How strong the light is. 1 is an ordinary lamp; 0 gives no light."),
    PropMeta::color3("Color", 1.0, 1.0, 1.0)
        .visual()
        .group(GROUP)
        .order(2)
        .desc("The colour of the light."),
    PropMeta::number("Range", 16.0)
        .range(0.0, 1000.0)
        .visual()
        .group(GROUP)
        .order(3)
        .desc("How far the light reaches, in metres. Past this distance it lights nothing."),
    PropMeta::boolean("Shadows", false)
        .visual()
        .group("Shadows")
        .order(0)
        .desc("Whether objects block this light and cast shadows. Shadows cost performance, so use them sparingly."),
    PropMeta::cframe("CFrame")
        .transform()
        .group("Transform")
        .order(0)
        .desc(
            "Where the light is and which way it points (forward is -Z). Inside a part it is relative to the part, \
             so the light moves with it; anywhere else it is in world space.",
        ),
];

/// The base of every light.
pub static CLASS: ClassDesc = ClassDesc::new("Light", Some("WorldObject"))
    .describe("The base of every light: whether it is on, how bright, what colour, how far it reaches and shadows.")
    .components(&["Transform"])
    .props(PROPS);

/// The part a light follows: its parent, when that is a `BasePart`.
pub fn host_part(dm: &DataModel, light: Id) -> Result<Option<Id>, DmError> {
    match dm.parent(light)? {
        Some(parent) if is_a_named(dm, parent, "BasePart")? => Ok(Some(parent)),
        _ => Ok(None),
    }
}

/// The light's place in the world: its `CFrame` after the part it sits in, if any.
pub fn world_cframe(dm: &DataModel, light: Id) -> Result<CFrame, DmError> {
    let local = dm.cframe(light, dm.prop_id(light, "CFrame")?)?;
    match host_part(dm, light)? {
        Some(part) => Ok(dm.cframe(part, dm.prop_id(part, "CFrame")?)? * local),
        None => Ok(local),
    }
}

/// Moves the light so that its place in the world is `world`, writing its `CFrame` relative to its part if it has
/// one. The part does not move.
pub fn set_world_cframe(dm: &mut DataModel, light: Id, world: CFrame) -> Result<(), DmError> {
    let local = match host_part(dm, light)? {
        Some(part) => dm.cframe(part, dm.prop_id(part, "CFrame")?)?.inverse() * world,
        None => world,
    };
    let prop = dm.prop_id(light, "CFrame")?;
    dm.set(light, prop, Value::CFrame(local))
}

#[cfg(test)]
mod tests {
    use metallic_core::{CFrame, Color3, PropId, Value, Vector3};

    use crate::shared_registry;
    use crate::testing::{game, new_in, workspace};

    const SHARED: [&str; 6] = ["Enabled", "Brightness", "Color", "Range", "Shadows", "CFrame"];

    /// The issue's acceptance test: inheritance gives every light `Light`'s members, under the same ids.
    #[test]
    fn spot_light_inherits_lights_members_and_adds_angle() {
        let registry = shared_registry().unwrap();
        let class = |name| registry.class_id(name).unwrap();
        let (light, spot, point, directional) = (
            class("Light"),
            class("SpotLight"),
            class("PointLight"),
            class("DirectionalLight"),
        );

        for member in ["Enabled", "Brightness", "Color", "Range", "Angle"] {
            assert!(registry.prop_id(spot, member).is_some(), "SpotLight.{member}");
        }
        assert!(registry.prop_id(light, "Angle").is_none(), "Angle is SpotLight's own");
        assert!(registry.prop_id(point, "Angle").is_none());

        for member in SHARED {
            let id = registry.prop_id(light, member).unwrap();
            assert_eq!(registry.prop_id(spot, member), Some(id), "SpotLight.{member}");
            assert_eq!(registry.prop_id(point, member), Some(id), "PointLight.{member}");
            let in_directional = registry.prop(directional, id).unwrap();
            assert_eq!(in_directional.meta.name, member, "DirectionalLight keeps the slot of {member}");
            assert_eq!(in_directional.hidden, member == "Range");
        }
    }

    #[test]
    fn directional_light_hides_range_but_keeps_its_slot() {
        let mut dm = game();
        let ws = workspace(&dm);
        let sun = new_in(&mut dm, "DirectionalLight", ws, "Sun");
        assert!(dm.prop_id(sun, "Range").is_err());
        assert!(dm.get_by_name(sun, "Range").is_err());

        let registry = shared_registry().unwrap();
        let light = registry.class_id("Light").unwrap();
        let range: PropId = registry.prop_id(light, "Range").unwrap();
        assert_eq!(dm.get(sun, range), Ok(Value::Number(16.0)));
    }

    #[test]
    fn lights_start_with_their_defaults() {
        let mut dm = game();
        let ws = workspace(&dm);
        for class in ["PointLight", "SpotLight", "DirectionalLight"] {
            let light = new_in(&mut dm, class, ws, class);
            assert_eq!(dm.get_by_name(light, "Enabled"), Ok(Value::Bool(true)), "{class}");
            assert_eq!(dm.get_by_name(light, "Brightness"), Ok(Value::Number(1.0)), "{class}");
            assert_eq!(
                dm.get_by_name(light, "Color"),
                Ok(Value::Color3(Color3::new(1.0, 1.0, 1.0))),
                "{class}"
            );
            assert_eq!(dm.get_by_name(light, "Shadows"), Ok(Value::Bool(false)), "{class}");
        }
    }

    #[test]
    fn writes_outside_a_range_are_clamped() {
        let mut dm = game();
        let ws = workspace(&dm);
        let spot = new_in(&mut dm, "SpotLight", ws, "Spot");
        for (prop, written, stored) in [
            ("Angle", 500.0, 179.0),
            ("Angle", 0.0, 1.0),
            ("Brightness", -3.0, 0.0),
            ("Brightness", 1e9, 100.0),
            ("Range", 5000.0, 1000.0),
        ] {
            dm.set_by_name(spot, prop, Value::Number(written)).unwrap();
            assert_eq!(dm.get_by_name(spot, prop), Ok(Value::Number(stored)), "{prop} = {written}");
        }
        assert!(dm.set_by_name(spot, "Angle", Value::Number(f64::NAN)).is_err());
    }

    #[test]
    fn every_concrete_light_has_its_emitter() {
        let registry = shared_registry().unwrap();
        for (class, emitter) in [
            ("PointLight", "PointLightEmitter"),
            ("SpotLight", "SpotLightEmitter"),
            ("DirectionalLight", "DirectionalLightEmitter"),
        ] {
            let info = registry.class(registry.class_id(class).unwrap()).unwrap();
            assert!(info.creatable(), "{class}");
            assert!(info.has_component(emitter), "{class}");
            assert!(info.has_component("Transform"), "{class} inherits Transform");
            assert_eq!(registry.component(emitter).unwrap().requires, super::EMITTER_REQUIRES);
        }
        assert!(!registry.class(registry.class_id("Light").unwrap()).unwrap().creatable());
    }

    #[test]
    fn a_light_in_a_part_follows_the_part() {
        let mut dm = game();
        let ws = workspace(&dm);
        let part = new_in(&mut dm, "Part", ws, "Lamp");
        let bulb = new_in(&mut dm, "PointLight", part, "Bulb");
        let lamp_at = CFrame::new(Vector3::new(10.0, 2.0, 0.0));
        dm.set_by_name(part, "CFrame", Value::CFrame(lamp_at)).unwrap();
        let above = CFrame::new(Vector3::new(0.0, 1.0, 0.0));
        dm.set_by_name(bulb, "CFrame", Value::CFrame(above)).unwrap();
        let world = super::world_cframe(&dm, bulb).unwrap();
        assert!(world.position().abs_diff_eq(Vector3::new(10.0, 3.0, 0.0), 1e-5));

        // Outside a part, the same CFrame is in world space.
        dm.set_parent(bulb, Some(ws)).unwrap();
        assert_eq!(super::world_cframe(&dm, bulb), Ok(above));
    }
}
