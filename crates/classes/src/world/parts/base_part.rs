//! `BasePart`: a solid object in the world with a size, a look and physical behaviour. `Part` is the one you create.
//!
//! `CFrame` is the stored transform, in world space. `Position` and `Orientation` are computed views of it: reading
//! them reads the `CFrame`, writing them writes it, and neither is saved. They find the `CFrame` through a cached
//! id checked on each use rather than by name, because they sit on the script read path (see `cframe_id`).
//!
//! `Size` is clamped on write, per component, into [`MIN_SIZE`]..=[`MAX_SIZE`] metres, so a negative or zero size
//! becomes the smallest one rather than an error. NaN and infinity are refused, like for every property.
//!
//! The `Material` enum lives here because `BasePart` owns it.

use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

use metallic_core::{
    CFrame, ClassDesc, DataModel, DmError, EnumDesc, Id, PropFlags, PropId, PropMeta, ScriptError, ScriptValue, Value,
    Vector3, Widget,
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

/// The name of the stored property `Position` and `Orientation` are views of.
const CFRAME: &str = "CFrame";

/// Above every raw [`PropId`], which is a `u16`: the cache below is empty.
const NO_ID: u32 = u32::MAX;

/// `CFrame`'s declaration, the first of [`PROPS`]. The registry hands it out by reference, unless a subclass
/// overrides it.
static CFRAME_META: &PropMeta = &PROPS[0];

/// The raw [`PropId`] `CFrame` had at its last lookup, or [`NO_ID`]. Only a hint: [`cframe_id`] checks it.
static CFRAME_ID: AtomicU32 = AtomicU32::new(NO_ID);

/// The id of the part's `CFrame`, without hashing its name on every read.
///
/// An inherited property keeps its id in every subclass, so within one registry every part class numbers `CFrame`
/// the same. Another registry can number it differently (tests build their own, with other classes above
/// `BasePart`), so the cached id is used only after checking that it names this class's visible `CFrame`: one index
/// into the class's property table and a pointer compare (a string compare under an override). When the check
/// fails, the name is looked up and the cache refilled. A stale id from another registry or thread is always caught
/// by the check, so relaxed ordering is enough.
fn cframe_id(dm: &DataModel, id: Id) -> Result<PropId, DmError> {
    let class = dm.class_of(id)?;
    if let Ok(raw) = u16::try_from(CFRAME_ID.load(Ordering::Relaxed)) {
        let cached = PropId::new(raw);
        // Comparing the declaration's address first skips the string compare, except under an override.
        let names_cframe = dm.registry().prop(class, cached).is_some_and(|prop| {
            !prop.hidden && (ptr::eq(prop.meta, CFRAME_META) || prop.meta.name == CFRAME)
        });
        if names_cframe {
            return Ok(cached);
        }
    }
    let prop = dm.prop_id(id, CFRAME)?;
    CFRAME_ID.store(u32::from(prop.raw()), Ordering::Relaxed);
    Ok(prop)
}

/// The part's `CFrame` and its id; the identity and `None` when it cannot be read (a computed getter cannot fail).
fn cframe_of(dm: &DataModel, id: Id) -> (CFrame, Option<PropId>) {
    let Ok(prop) = cframe_id(dm, id) else {
        return (CFrame::IDENTITY, None);
    };
    // `get_ref` rather than the typed `cframe`: borrowing and matching here measured about twice as fast.
    match dm.get_ref(id, prop) {
        Ok(Value::CFrame(cframe)) => (*cframe, Some(prop)),
        _ => (CFrame::IDENTITY, Some(prop)),
    }
}

/// Writes the part's `CFrame` through `prop`, the id [`cframe_of`] found, looking it up when there was none.
fn set_cframe(dm: &mut DataModel, id: Id, prop: Option<PropId>, cframe: CFrame) -> Result<(), ScriptError> {
    let prop = match prop {
        Some(prop) => prop,
        None => dm.prop_id(id, CFRAME)?,
    };
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
    Value::Vector3(cframe_of(dm, id).0.position())
}

fn position_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let position = vector3_of(value)?;
    let (current, prop) = cframe_of(dm, id);
    set_cframe(dm, id, prop, CFrame::from_parts(position, current.rotation()))
}

fn orientation_get(dm: &DataModel, id: Id) -> Value {
    Value::Vector3(cframe_of(dm, id).0.to_orientation_deg())
}

fn orientation_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let angles = vector3_of(value)?;
    let (current, prop) = cframe_of(dm, id);
    let rotation = CFrame::from_orientation_deg(angles.x, angles.y, angles.z).rotation();
    set_cframe(dm, id, prop, CFrame::from_parts(current.position(), rotation))
}

static PROPS: &[PropMeta] = &[
    PropMeta::cframe(CFRAME)
        .transform()
        // Position and Rotation are its rows in the Properties panel (#589); scripts still see it.
        .flags(PropFlags::HIDDEN)
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
        // Shown as a model's is (#439, #455); scripts still call it Orientation.
        .label("Rotation")
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
    // Appearance's order leaves 0 to a part's Shape (#589).
    PropMeta::color3("Color", 0.64, 0.64, 0.64)
        .visual()
        .group("Appearance")
        .order(2)
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
        .order(3)
        .desc("How see-through the part is: 0 is solid, 1 is invisible."),
    PropMeta::boolean("CastShadow", true)
        .visual()
        .group("Appearance")
        .order(4)
        .desc("Whether the part casts a shadow."),
    // Collision is declared before Physics so the Properties panel lists it first: groups come in the order a class
    // first names them.
    PropMeta::boolean("CanCollide", true)
        .physics()
        .group("Collision")
        .order(0)
        .desc("Whether other parts bump into this one. When off, things pass through it."),
    // On by default (#589): a part placed in Studio stays where it is put. Scenes from before keep theirs (format 2).
    PropMeta::boolean("Anchored", true)
        .physics()
        .group("Physics")
        .order(0)
        .desc(
            "Whether the part stays where it is. An anchored part is not moved by gravity or collisions. On by \
             default.",
        ),
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
        assert_eq!(dm.get_by_name(part, "Anchored"), Ok(Value::Bool(true)), "on by default (#589)");
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

    /// The `CFrame` id cache: the views must find `CFrame` by checking, never by assuming the id.
    mod cframe_id {
        use std::sync::Arc;

        use metallic_core::{
            CFrame, ClassDesc, DataModel, Id, PropId, PropMeta, Registry, RegistryInput, Value, Vector3,
        };

        use crate::base::{instance, object};
        use crate::components::transform;
        use crate::world::parts::{base_part, part};

        /// A `WorldObject` with two more properties than the real one, so `BasePart`'s `CFrame` lands on another id.
        static WIDE_WORLD_OBJECT: ClassDesc = ClassDesc::new("WorldObject", Some("Instance")).props(&[
            PropMeta::cframe("PivotOffset"),
            PropMeta::number("Padding", 0.0),
            PropMeta::number("MorePadding", 0.0),
        ]);

        /// A class under `Part` that adds properties of its own after the inherited ones.
        static SUB_PART: ClassDesc = ClassDesc::new("SubPart", Some("Part"))
            .creatable()
            .props(&[PropMeta::number("Extra", 0.0)]);

        /// A class under `Part` that overrides `CFrame`, so its declaration is not `BasePart`'s.
        static OVERRIDING_PART: ClassDesc = ClassDesc::new("OverridingPart", Some("Part"))
            .creatable()
            .props(&[PropMeta::cframe("CFrame").override_inherited().transform()]);

        /// A class under `Instance` whose name sorts first, so every class id after it moves.
        static AARDVARK: ClassDesc = ClassDesc::new("Aardvark", Some("Instance")).creatable();

        /// A registry of the part classes and the test classes, with `classes` listed in the given order.
        fn registry_of(classes: &[&'static ClassDesc]) -> Arc<Registry> {
            let enums = [base_part::ENUMS, part::ENUMS];
            Arc::new(
                Registry::build(RegistryInput {
                    classes,
                    components: &[&transform::COMPONENT, &part::COMPONENT],
                    enums: &enums,
                })
                .unwrap_or_else(|e| panic!("{e}")),
            )
        }

        fn create(dm: &mut DataModel, class: &str) -> Id {
            let class = dm.registry().class_id(class).unwrap();
            dm.create(class).unwrap()
        }

        fn cframe_id(dm: &DataModel, id: Id) -> PropId {
            dm.prop_id(id, "CFrame").unwrap()
        }

        fn vector(dm: &DataModel, id: Id, name: &str) -> Vector3 {
            match dm.get_by_name(id, name) {
                Ok(Value::Vector3(v)) => v,
                other => panic!("{name}: {other:?}"),
            }
        }

        fn cframe(dm: &DataModel, id: Id) -> CFrame {
            match dm.get_by_name(id, "CFrame") {
                Ok(Value::CFrame(cf)) => cf,
                other => panic!("CFrame: {other:?}"),
            }
        }

        /// Writes `Position` and `Orientation` on `id` and reads them back, through `CFrame` too.
        fn views_round_trip(dm: &mut DataModel, id: Id, at: Vector3) {
            dm.set_by_name(id, "Position", Value::Vector3(at)).unwrap();
            assert_eq!(cframe(dm, id).position(), at);
            assert_eq!(vector(dm, id, "Position"), at);

            let angles = Vector3::new(10.0, 20.0, 30.0);
            dm.set_by_name(id, "Orientation", Value::Vector3(angles)).unwrap();
            assert!(vector(dm, id, "Orientation").abs_diff_eq(angles, 1e-3));
            assert_eq!(vector(dm, id, "Position"), at, "rotating keeps the position");
            let expected = CFrame::from_orientation_deg(10.0, 20.0, 30.0).rotation();
            assert!(cframe(dm, id).rotation().abs_diff_eq(expected, 1e-6));

            let moved = CFrame::new(at * 2.0);
            dm.set_by_name(id, "CFrame", Value::CFrame(moved)).unwrap();
            assert_eq!(vector(dm, id, "Position"), at * 2.0);
            assert!(vector(dm, id, "Orientation").abs_diff_eq(Vector3::ZERO, 1e-4));
        }

        #[test]
        fn views_work_on_part_and_a_subclass() {
            let registry = registry_of(&[
                &object::CLASS,
                &instance::CLASS,
                &crate::base::world_object::CLASS,
                &base_part::CLASS,
                &part::CLASS,
                &SUB_PART,
            ]);
            let mut dm = DataModel::new(registry);
            let part = create(&mut dm, "Part");
            let sub = create(&mut dm, "SubPart");
            assert_eq!(cframe_id(&dm, part), cframe_id(&dm, sub), "inherited ids are shared");
            views_round_trip(&mut dm, part, Vector3::new(1.0, 2.0, 3.0));
            views_round_trip(&mut dm, sub, Vector3::new(-4.0, 5.0, -6.0));
            assert_eq!(dm.get_by_name(sub, "Extra"), Ok(Value::Number(0.0)));
        }

        #[test]
        fn registries_that_number_cframe_differently_are_not_confused() {
            let usual = registry_of(&[
                &object::CLASS,
                &instance::CLASS,
                &crate::base::world_object::CLASS,
                &base_part::CLASS,
                &part::CLASS,
            ]);
            // Another class order and set: ids are not assumed from either.
            let shifted = registry_of(&[
                &SUB_PART,
                &part::CLASS,
                &AARDVARK,
                &base_part::CLASS,
                &WIDE_WORLD_OBJECT,
                &instance::CLASS,
                &object::CLASS,
            ]);
            let mut a = DataModel::new(usual);
            let mut b = DataModel::new(shifted);
            let part_a = create(&mut a, "Part");
            let part_b = create(&mut b, "Part");
            let sub_b = create(&mut b, "SubPart");
            assert_ne!(cframe_id(&a, part_a), cframe_id(&b, part_b), "the test needs differing ids");
            assert_ne!(a.class_of(part_a).unwrap(), b.class_of(part_b).unwrap());

            // Alternate between the registries so each read finds the other's id cached.
            for step in 1..=3 {
                let at = Vector3::splat(step as f32);
                views_round_trip(&mut a, part_a, at);
                views_round_trip(&mut b, part_b, -at);
                views_round_trip(&mut b, sub_b, at * 3.0);
                assert_eq!(vector(&a, part_a, "Position"), at * 2.0);
                assert_eq!(vector(&b, part_b, "Position"), -at * 2.0);
            }
            // Padding sits where the usual registry keeps `CFrame`; it must never be written as one.
            assert_eq!(b.get_by_name(part_b, "Padding"), Ok(Value::Number(0.0)));
            assert_eq!(b.get_by_name(part_b, "MorePadding"), Ok(Value::Number(0.0)));
        }

        #[test]
        fn views_follow_an_overridden_cframe() {
            let registry = registry_of(&[
                &object::CLASS,
                &instance::CLASS,
                &crate::base::world_object::CLASS,
                &base_part::CLASS,
                &part::CLASS,
                &OVERRIDING_PART,
            ]);
            let mut dm = DataModel::new(registry);
            let part = create(&mut dm, "Part");
            let overriding = create(&mut dm, "OverridingPart");
            assert_eq!(cframe_id(&dm, part), cframe_id(&dm, overriding), "an override keeps the id");
            views_round_trip(&mut dm, overriding, Vector3::new(7.0, 8.0, 9.0));
            views_round_trip(&mut dm, part, Vector3::new(1.0, 1.0, 1.0));
        }
    }
}
