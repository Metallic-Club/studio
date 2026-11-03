//! `BasePart`: a solid object in the world with a size, a look and physical behaviour. `Part` is the one you create.
//!
//! `CFrame` is the stored transform, in world space. `Position` and `Orientation` are computed views of it: reading
//! them reads the `CFrame`, writing them writes it, and neither is saved.
//!
//! `Size` is clamped on write, per component, into [`MIN_SIZE`]..=[`MAX_SIZE`] metres, so a negative or zero size
//! becomes the smallest one rather than an error. NaN and infinity are refused, like for every property.
//!
//! The `Material` enum lives here because `BasePart` owns it.

use metallic_core::{
    CFrame, ClassDesc, DataModel, EnumDesc, Id, PropFlags, PropMeta, ScriptError, ScriptValue, Value, Vector3, Widget,
};

/// The smallest a part can be along each axis, in metres.
pub const MIN_SIZE: f64 = 0.001;

/// The largest a part can be along each axis, in metres.
pub const MAX_SIZE: f64 = 2048.0;

/// The enums `BasePart` declares.
pub static ENUMS: &[EnumDesc] = &[EnumDesc::new(
    "Material",
    &[
        "Plastic",
        "SmoothPlastic",
        "Metal",
        "Wood",
        "Glass",
        "Neon",
        "Concrete",
        "Grass",
    ],
)
.describe("What a part's surface is made of, which sets how it looks: its texture, shine and glow.")];

/// The part's `CFrame`, or the identity when it cannot be read (a computed getter cannot fail).
fn cframe_of(dm: &DataModel, id: Id) -> CFrame {
    dm.prop_id(id, "CFrame")
        .and_then(|prop| dm.cframe(id, prop))
        .unwrap_or_default()
}

/// Writes the part's `CFrame`.
fn set_cframe(dm: &mut DataModel, id: Id, cframe: CFrame) -> Result<(), ScriptError> {
    let prop = dm.prop_id(id, "CFrame")?;
    Ok(dm.set(id, prop, Value::CFrame(cframe))?)
}

/// A `Vector3` from a value the data model has already type-checked.
fn vector3_of(value: Value) -> Result<Vector3, ScriptError> {
    match value {
        Value::Vector3(v) => Ok(v),
        other => Err(ScriptError::wrong_type("Vector3", ScriptValue::from(other).type_name())),
    }
}

fn position_get(dm: &DataModel, id: Id) -> Value {
    Value::Vector3(cframe_of(dm, id).position())
}

fn position_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let position = vector3_of(value)?;
    let current = cframe_of(dm, id);
    set_cframe(dm, id, CFrame::from_parts(position, current.rotation()))
}

fn orientation_get(dm: &DataModel, id: Id) -> Value {
    Value::Vector3(cframe_of(dm, id).to_orientation_deg())
}

fn orientation_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let angles = vector3_of(value)?;
    let current = cframe_of(dm, id);
    let rotation = CFrame::from_orientation_deg(angles.x, angles.y, angles.z).rotation();
    set_cframe(dm, id, CFrame::from_parts(current.position(), rotation))
}

static PROPS: &[PropMeta] = &[
    PropMeta::cframe("CFrame")
        .transform()
        .group("Transform")
        .order(0)
        .desc("Where the part is and how it is rotated, in world space. Position and Orientation are views of it."),
    PropMeta::vector3("Position", 0.0, 0.0, 0.0)
        .computed(position_get, Some(position_set))
        .derived_from(&["CFrame"])
        .transform()
        .flags(PropFlags::NO_SAVE)
        .group("Transform")
        .order(1)
        .desc("Where the part's centre is, in metres. Setting it moves the part without rotating it."),
    PropMeta::vector3("Orientation", 0.0, 0.0, 0.0)
        .computed(orientation_get, Some(orientation_set))
        .derived_from(&["CFrame"])
        .transform()
        .flags(PropFlags::NO_SAVE)
        .group("Transform")
        .order(2)
        .desc(
            "The part's rotation as angles in degrees about X, Y and Z, applied Z first, then X, then Y. Setting it \
             rotates the part in place.",
        ),
    PropMeta::vector3("Size", 4.0, 1.0, 2.0)
        .range(MIN_SIZE, MAX_SIZE)
        .transform()
        .physics()
        .group("Transform")
        .order(3)
        .desc("How big the part is along X, Y and Z, in metres. Each side is kept between 0.001 and 2048."),
    PropMeta::color3("Color", 0.64, 0.64, 0.64)
        .visual()
        .group("Appearance")
        .order(0)
        .desc("The part's colour."),
    PropMeta::enumeration("Material", "Material", "Plastic")
        .visual()
        .group("Appearance")
        .order(1)
        .desc("What the part looks like it is made of, such as Wood, Metal or Neon."),
    PropMeta::number("Transparency", 0.0)
        .range(0.0, 1.0)
        .widget(Widget::Slider)
        .visual()
        .group("Appearance")
        .order(2)
        .desc("How see-through the part is: 0 is solid, 1 is invisible."),
    PropMeta::boolean("CastShadow", true)
        .visual()
        .group("Appearance")
        .order(3)
        .desc("Whether the part casts a shadow."),
    PropMeta::boolean("Anchored", false)
        .physics()
        .group("Physics")
        .order(0)
        .desc("Whether the part stays where it is. An anchored part is not moved by gravity or collisions."),
    PropMeta::boolean("CanCollide", true)
        .physics()
        .group("Physics")
        .order(1)
        .desc("Whether other parts bump into this one. When off, things pass through it."),
];

/// The base of every solid part.
pub static CLASS: ClassDesc = ClassDesc::new("BasePart", Some("WorldObject"))
    .describe("The base of every solid object in the world: where it is, how big, what it looks like and how it behaves.")
    .components(&["Transform"])
    .props(PROPS);

#[cfg(test)]
mod tests {
    use metallic_core::{CFrame, Color3, EnumItem, Value, Vector3};

    use crate::shared_registry;
    use crate::testing::{game, new_in, workspace};

    fn part() -> (metallic_core::DataModel, metallic_core::Id) {
        let mut dm = game();
        let ws = workspace(&dm);
        let part = new_in(&mut dm, "Part", ws, "Part");
        (dm, part)
    }

    fn vector(dm: &metallic_core::DataModel, id: metallic_core::Id, name: &str) -> Vector3 {
        match dm.get_by_name(id, name) {
            Ok(Value::Vector3(v)) => v,
            other => panic!("{name}: {other:?}"),
        }
    }

    fn cframe(dm: &metallic_core::DataModel, id: metallic_core::Id) -> CFrame {
        match dm.get_by_name(id, "CFrame") {
            Ok(Value::CFrame(cf)) => cf,
            other => panic!("CFrame: {other:?}"),
        }
    }

    #[test]
    fn position_and_cframe_move_together() {
        let (mut dm, part) = part();
        let turned = CFrame::from_orientation_deg(0.0, 90.0, 0.0);
        dm.set_by_name(part, "CFrame", Value::CFrame(turned)).unwrap();

        dm.set_by_name(part, "Position", Value::Vector3(Vector3::new(1.0, 2.0, 3.0)))
            .unwrap();
        let cf = cframe(&dm, part);
        assert_eq!(cf.position(), Vector3::new(1.0, 2.0, 3.0));
        assert!(cf.rotation().abs_diff_eq(turned.rotation(), 1e-6), "rotation is kept");

        let moved = CFrame::new(Vector3::new(-5.0, 0.5, 8.0));
        dm.set_by_name(part, "CFrame", Value::CFrame(moved)).unwrap();
        assert_eq!(vector(&dm, part, "Position"), Vector3::new(-5.0, 0.5, 8.0));
    }

    #[test]
    fn orientation_round_trips_and_keeps_the_position() {
        let (mut dm, part) = part();
        dm.set_by_name(part, "Position", Value::Vector3(Vector3::new(4.0, 5.0, 6.0)))
            .unwrap();
        let angles = Vector3::new(30.0, -45.0, 10.0);
        dm.set_by_name(part, "Orientation", Value::Vector3(angles)).unwrap();
        assert!(vector(&dm, part, "Orientation").abs_diff_eq(angles, 1e-3));
        assert_eq!(vector(&dm, part, "Position"), Vector3::new(4.0, 5.0, 6.0));
        let expected = CFrame::from_orientation_deg(30.0, -45.0, 10.0).rotation();
        assert!(cframe(&dm, part).rotation().abs_diff_eq(expected, 1e-6));
    }

    #[test]
    fn every_transform_write_fires_cframe_position_and_orientation_once() {
        use metallic_core::{Delivery, ListenerKey, ScriptValue};

        let (mut dm, part) = part();
        let changed = dm.signal_id(part, "Changed").unwrap();
        dm.connect(part, changed, ListenerKey(0)).unwrap();
        let names = ["CFrame", "Position", "Orientation"];
        for (listener, name) in (1..).zip(names) {
            let prop = dm.prop_id(part, name).unwrap();
            dm.connect_property_changed(part, prop, ListenerKey(listener)).unwrap();
        }
        let expected: Vec<(u64, Vec<ScriptValue>)> = (1..)
            .zip(names)
            .flat_map(|(listener, name)| [(0, vec![ScriptValue::String(name.into())]), (listener, vec![])])
            .collect();

        let writes = [
            ("CFrame", Value::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 3.0)))),
            ("Position", Value::Vector3(Vector3::new(4.0, 5.0, 6.0))),
            ("Orientation", Value::Vector3(Vector3::new(0.0, 90.0, 0.0))),
        ];
        for (name, value) in writes {
            dm.set_by_name(part, name, value).unwrap();
            let mut out: Vec<Delivery> = Vec::new();
            dm.drain_deliveries(&mut out);
            let got: Vec<_> = out.into_iter().map(|d| (d.listener.0, d.args)).collect();
            assert_eq!(got, expected, "writing {name}");
        }
    }

    #[test]
    fn computed_views_are_not_saved() {
        let registry = shared_registry().unwrap();
        let class = registry.class_id("BasePart").unwrap();
        for name in ["Position", "Orientation"] {
            let prop = registry.prop(class, registry.prop_id(class, name).unwrap()).unwrap();
            assert!(prop.meta.flags.contains(metallic_core::PropFlags::NO_SAVE), "{name}");
            assert!(prop.meta.computed.is_some(), "{name}");
        }
    }

    #[test]
    fn writes_are_clamped_and_nan_is_refused() {
        let (mut dm, part) = part();
        dm.set_by_name(part, "Transparency", Value::Number(2.0)).unwrap();
        assert_eq!(dm.get_by_name(part, "Transparency"), Ok(Value::Number(1.0)));
        dm.set_by_name(part, "Transparency", Value::Number(-1.0)).unwrap();
        assert_eq!(dm.get_by_name(part, "Transparency"), Ok(Value::Number(0.0)));

        dm.set_by_name(part, "Size", Value::Vector3(Vector3::new(-2.0, 0.0, 9000.0)))
            .unwrap();
        let min = super::MIN_SIZE as f32;
        let max = super::MAX_SIZE as f32;
        assert_eq!(vector(&dm, part, "Size"), Vector3::new(min, min, max));

        for (name, value) in [
            ("Transparency", Value::Number(f64::NAN)),
            ("Size", Value::Vector3(Vector3::new(1.0, f32::NAN, 1.0))),
            ("Position", Value::Vector3(Vector3::new(f32::INFINITY, 0.0, 0.0))),
            ("Orientation", Value::Vector3(Vector3::new(0.0, f32::NAN, 0.0))),
        ] {
            assert!(dm.set_by_name(part, name, value).is_err(), "{name}");
        }
        assert_eq!(cframe(&dm, part), CFrame::IDENTITY, "refused writes change nothing");
    }

    #[test]
    fn parts_start_with_their_defaults() {
        let (dm, part) = part();
        assert_eq!(vector(&dm, part, "Size"), Vector3::new(4.0, 1.0, 2.0));
        assert_eq!(
            dm.get_by_name(part, "Color"),
            Ok(Value::Color3(Color3::new(0.64, 0.64, 0.64)))
        );
        assert_eq!(dm.get_by_name(part, "Transparency"), Ok(Value::Number(0.0)));
        assert_eq!(dm.get_by_name(part, "CastShadow"), Ok(Value::Bool(true)));
        assert_eq!(dm.get_by_name(part, "Anchored"), Ok(Value::Bool(false)));
        assert_eq!(dm.get_by_name(part, "CanCollide"), Ok(Value::Bool(true)));
        assert_eq!(
            dm.get_by_name(part, "Material"),
            Ok(Value::Enum(EnumItem::new("Material", "Plastic")))
        );
        assert_eq!(cframe(&dm, part), CFrame::IDENTITY);
    }

    #[test]
    fn material_has_its_items() {
        let registry = shared_registry().unwrap();
        let material = registry.enum_desc("Material").unwrap();
        assert_eq!(
            material.items,
            ["Plastic", "SmoothPlastic", "Metal", "Wood", "Glass", "Neon", "Concrete", "Grass"]
        );
        let (mut dm, part) = part();
        dm.set_by_name(part, "Material", Value::Enum(EnumItem::new("Material", "Neon")))
            .unwrap();
        let unknown = Value::Enum(EnumItem::new("Material", "Cheese"));
        assert!(dm.set_by_name(part, "Material", unknown).is_err());
    }

    #[test]
    fn base_part_is_abstract() {
        let registry = shared_registry().unwrap();
        let class = registry.class_id("BasePart").unwrap();
        assert!(!registry.class(class).unwrap().creatable());
        let mut dm = game();
        assert!(dm.create(class).is_err());
    }
}
