//! Registry tests: resolution, lookups, and one test per validation error.

use std::sync::Arc;

use super::*;
use crate::{EnumItem, PropType, ScriptError, ScriptValue, Value};

fn noop(_: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(ScriptValue::Nil)
}

fn other(_: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(ScriptValue::Bool(true))
}

fn build(classes: &[&'static ClassDesc]) -> Result<Registry, RegistryErrors> {
    Registry::build(RegistryInput {
        classes,
        ..RegistryInput::default()
    })
}

fn errors(input: RegistryInput<'_>) -> Vec<RegistryError> {
    Registry::build(input).expect_err("should be rejected").0
}

fn class_errors(classes: &[&'static ClassDesc]) -> Vec<RegistryError> {
    errors(RegistryInput {
        classes,
        ..RegistryInput::default()
    })
}

static ROOT: ClassDesc = ClassDesc::new("Object", None);

// A small hierarchy used by the resolution tests: Object -> Instance -> Light -> SpotLight / DirectionalLight.

static SHAPES: &[EnumDesc] = &[EnumDesc::new("LightKind", &["Point", "Spot", "Sun"]).numbers(&[1, 2, 4])];

static TRANSFORM: ComponentDesc = ComponentDesc::new("Transform").props(&[PropMeta::cframe("CFrame").transform()]);
static EMITTER: ComponentDesc = ComponentDesc::new("LightEmitter")
    .requires(&["Transform"])
    .props(&[PropMeta::boolean("Shadows", false).visual()]);

static OBJECT: ClassDesc = ClassDesc::new("Object", None)
    .props(&[PropMeta::string("ClassName", "").read_only()])
    .signals(&[SignalDesc::new("Changed").params(&[Param::new("property", Ty::String)])]);

static INSTANCE: ClassDesc = ClassDesc::new("Instance", Some("Object"))
    .props(&[
        PropMeta::string("Name", "Instance").aliases(&["name"]),
        PropMeta::boolean("Archivable", true),
    ])
    .methods(&[
        MethodDesc::new("Destroy", noop).returns_nothing(),
        MethodDesc::new("Clone", noop).returns(Ty::Class("Instance")),
    ]);

static LIGHT: ClassDesc = ClassDesc::new("Light", Some("Instance"))
    .components(&["Transform", "LightEmitter"])
    .props(&[
        PropMeta::boolean("Enabled", true),
        PropMeta::number("Brightness", 1.0).range(0.0, 100.0),
        PropMeta::color3("Color", 1.0, 1.0, 1.0),
        PropMeta::number("Range", 16.0),
        PropMeta::enumeration("Kind", "LightKind", "Point"),
    ]);

static SPOT_LIGHT: ClassDesc = ClassDesc::new("SpotLight", Some("Light"))
    .creatable()
    .components(&["Transform"])
    .props(&[PropMeta::number("Angle", 90.0).range(1.0, 179.0)])
    .methods(&[MethodDesc::new("Clone", other).returns(Ty::Class("Instance"))])
    .defaults(&[
        ("Brightness", DefaultValue::Number(2.0)),
        ("Kind", DefaultValue::Enum("Spot")),
    ]);

static DIRECTIONAL: ClassDesc = ClassDesc::new("DirectionalLight", Some("Light"))
    .creatable()
    .hide(&["Range", "Destroy", "Changed"]);

static WORKSPACE: ClassDesc = ClassDesc::new("Workspace", Some("Instance")).service(1).persisted();
static LIGHTING: ClassDesc = ClassDesc::new("Lighting", Some("Instance")).service(1);
static STORAGE: ClassDesc = ClassDesc::new("ServerStorage", Some("Instance")).service(0);

fn lights() -> Registry {
    Registry::build(RegistryInput {
        // Deliberately not in inheritance order.
        classes: &[
            &SPOT_LIGHT,
            &DIRECTIONAL,
            &WORKSPACE,
            &LIGHT,
            &INSTANCE,
            &OBJECT,
            &LIGHTING,
            &STORAGE,
        ],
        components: &[&EMITTER, &TRANSFORM],
        enums: &[SHAPES],
    })
    .expect("valid hierarchy")
}

fn id(reg: &Registry, name: &str) -> ClassId {
    reg.class_id(name).expect("known class")
}

fn names(reg: &Registry, class: &str) -> Vec<&'static str> {
    reg.props(id(reg, class)).iter().map(|p| p.meta.name).collect()
}

#[test]
fn three_level_chain_puts_inherited_properties_first() {
    let reg = lights();
    assert_eq!(names(&reg, "Object"), ["ClassName"]);
    assert_eq!(names(&reg, "Instance"), ["ClassName", "Name", "Archivable"]);
    // Parent's list, then the components this class adds, then its own properties.
    assert_eq!(
        names(&reg, "Light"),
        [
            "ClassName",
            "Name",
            "Archivable",
            "CFrame",
            "Shadows",
            "Enabled",
            "Brightness",
            "Color",
            "Range",
            "Kind"
        ]
    );
    let spot = names(&reg, "SpotLight");
    assert_eq!(spot[..10], names(&reg, "Light")[..]);
    assert_eq!(spot[10..], ["Angle"]);
}

#[test]
fn a_property_keeps_its_id_in_every_descendant() {
    let reg = lights();
    for (prop, declared_on) in [
        ("ClassName", "Object"),
        ("Name", "Instance"),
        ("CFrame", "Light"),
        ("Range", "Light"),
    ] {
        let expected = reg.prop_id(id(&reg, declared_on), prop).expect("declared");
        for class in [
            "Object",
            "Instance",
            "Light",
            "SpotLight",
            "DirectionalLight",
            "Workspace",
        ] {
            let class_id = id(&reg, class);
            if !reg.is_a(class_id, id(&reg, declared_on)) {
                continue;
            }
            // Hidden properties are not found by name, but keep their slot.
            let found = reg.props(class_id).iter().position(|p| p.meta.name == prop);
            assert_eq!(found, Some(expected.index()), "{prop} in {class}");
        }
    }
}

#[test]
fn owners_point_at_the_declaring_class() {
    let reg = lights();
    let spot = id(&reg, "SpotLight");
    let owner = |name| {
        reg.class(
            reg.prop(spot, reg.prop_id(spot, name).expect("prop"))
                .expect("prop")
                .owner,
        )
    };
    assert_eq!(owner("Name").map(ClassInfo::name), Some("Instance"));
    // Component properties belong to the class that added the component.
    assert_eq!(owner("CFrame").map(ClassInfo::name), Some("Light"));
    assert_eq!(owner("Angle").map(ClassInfo::name), Some("SpotLight"));
}

#[test]
fn ids_put_parents_first_and_ignore_input_order() {
    let reg = lights();
    for class in reg.classes() {
        assert_eq!(reg.class(class.id()).map(ClassInfo::name), Some(class.name()));
        if let Some(parent) = class.parent() {
            assert!(parent < class.id());
        }
    }
    assert_eq!(id(&reg, "Object"), ClassId::new(0));
    assert_eq!(id(&reg, "Instance").raw(), 1);
}

#[test]
fn is_a_follows_the_chain() {
    let reg = lights();
    let [object, instance, light, spot, dir, workspace] = [
        "Object",
        "Instance",
        "Light",
        "SpotLight",
        "DirectionalLight",
        "Workspace",
    ]
    .map(|n| id(&reg, n));
    assert!(reg.is_a(spot, spot));
    assert!(reg.is_a(spot, light));
    assert!(reg.is_a(spot, instance));
    assert!(reg.is_a(spot, object));
    assert!(!reg.is_a(light, spot));
    assert!(!reg.is_a(spot, dir));
    assert!(!reg.is_a(workspace, light));
    assert!(!reg.is_a(ClassId::new(999), object));
    assert!(!reg.is_a(spot, ClassId::new(999)));
    let info = reg.class(spot).expect("class");
    assert_eq!(info.depth(), 3);
    assert_eq!(info.ancestors(), [object, instance, light, spot]);
    assert_eq!(info.parent(), Some(light));
}

#[test]
fn defaults_change_only_the_child() {
    let reg = lights();
    let (light, spot) = (id(&reg, "Light"), id(&reg, "SpotLight"));
    let brightness = reg.prop_id(light, "Brightness").expect("prop");
    assert_eq!(
        reg.prop(light, brightness).map(|p| &p.default),
        Some(&Value::Number(1.0))
    );
    assert_eq!(
        reg.prop(spot, brightness).map(|p| &p.default),
        Some(&Value::Number(2.0))
    );
    let kind = reg.prop_id(spot, "Kind").expect("prop");
    let spot_kind = reg.enum_item("LightKind", "Spot").expect("item");
    assert_eq!(reg.prop(spot, kind).map(|p| &p.default), Some(&Value::Enum(spot_kind)));
}

#[test]
fn hide_removes_members_from_lookups_but_keeps_the_slot() {
    let reg = lights();
    let (light, dir) = (id(&reg, "Light"), id(&reg, "DirectionalLight"));
    let range = reg.prop_id(light, "Range").expect("visible on Light");
    assert_eq!(reg.prop_id(dir, "Range"), None);
    let slot = reg.prop(dir, range).expect("slot kept");
    assert!(slot.hidden);
    assert_eq!(slot.meta.name, "Range");
    assert_eq!(reg.props(dir).len(), reg.props(light).len());

    assert!(reg.method(light, "Destroy").is_some());
    assert!(reg.method(dir, "Destroy").is_none());
    assert!(reg.method(dir, "Clone").is_some());
    assert!(reg.signal(dir, "Changed").is_none());
    let info = reg.class(dir).expect("class");
    assert!(info.methods().iter().any(|m| m.desc.name == "Destroy" && m.hidden));
    assert!(info.signals().iter().any(|s| s.desc.name == "Changed" && s.hidden));
}

#[test]
fn a_child_method_replaces_the_parents_in_place() {
    let reg = lights();
    let (instance, spot) = (id(&reg, "Instance"), id(&reg, "SpotLight"));
    let runs = |class, func: MethodFn| std::ptr::fn_addr_eq(reg.method(class, "Clone").expect("method").func, func);
    assert!(runs(instance, noop));
    assert!(runs(spot, other));
    let methods: Vec<_> = reg
        .class(spot)
        .expect("class")
        .methods()
        .iter()
        .map(|m| m.desc.name)
        .collect();
    assert_eq!(methods, ["Destroy", "Clone"]);
    assert_eq!(reg.class(spot).expect("class").methods()[1].owner, spot);
}

#[test]
fn signals_are_inherited() {
    let reg = lights();
    let signal = reg.signal(id(&reg, "SpotLight"), "Changed").expect("inherited");
    assert_eq!(signal.params, Some(&[Param::new("property", Ty::String)][..]));
}

#[test]
fn signal_ids_are_stable_down_the_hierarchy() {
    static BASE: ClassDesc = ClassDesc::new("Base", None).signals(&[SignalDesc::new("A"), SignalDesc::new("B")]);
    static MID: ClassDesc = ClassDesc::new("Mid", Some("Base")).signals(&[SignalDesc::new("C")]);
    // Redeclares `A` (replaced in place) and adds `D`.
    static LEAF: ClassDesc =
        ClassDesc::new("Leaf", Some("Mid")).signals(&[SignalDesc::new("D"), SignalDesc::new("A").describe("again")]);
    let reg = build(&[&LEAF, &MID, &BASE]).expect("valid");
    let (base, mid, leaf) = (id(&reg, "Base"), id(&reg, "Mid"), id(&reg, "Leaf"));
    for name in ["A", "B"] {
        let declared = reg.signal_id(base, name).expect("declared");
        assert_eq!(reg.signal_id(mid, name), Some(declared), "{name} on Mid");
        assert_eq!(reg.signal_id(leaf, name), Some(declared), "{name} on Leaf");
    }
    assert_eq!(reg.signal_id(mid, "C"), reg.signal_id(leaf, "C"));
    let names: Vec<_> = reg
        .class(leaf)
        .expect("class")
        .signals()
        .iter()
        .map(|s| s.desc.name)
        .collect();
    assert_eq!(
        names,
        ["A", "B", "C", "D"],
        "parent's signals first, then the class's new ones"
    );
    let a = reg.signal_id(leaf, "A").expect("A");
    let info = reg.class(leaf).expect("class");
    assert_eq!(info.signal_by_id(a).map(|s| s.desc.description), Some("again"));
    assert_eq!(info.signal_by_id(a).map(|s| s.owner), Some(leaf));
    assert_eq!(reg.signal_id(leaf, "Nope"), None);
    assert_eq!(reg.signal_id(ClassId::new(999), "A"), None);
}

#[test]
fn aliases_find_the_property() {
    let reg = lights();
    let spot = id(&reg, "SpotLight");
    assert_eq!(reg.prop_id(spot, "name"), reg.prop_id(spot, "Name"));
    assert_eq!(reg.prop_id(spot, "Nope"), None);
    assert_eq!(reg.prop_id(ClassId::new(999), "Name"), None);
    assert!(reg.props(ClassId::new(999)).is_empty());
}

#[test]
fn components_are_inherited_once() {
    let reg = lights();
    let spot = reg.class(id(&reg, "SpotLight")).expect("class");
    let components: Vec<_> = spot.components().iter().map(|c| c.name).collect();
    assert_eq!(components, ["Transform", "LightEmitter"]);
    assert!(spot.has_component("LightEmitter"));
    assert!(!spot.has_component("Rigidbody"));
    assert_eq!(reg.component("Transform").map(|c| c.name), Some("Transform"));
    assert!(reg.component("Rigidbody").is_none());
    // Listed by name, whatever the input order (`EMITTER` comes first there).
    let all: Vec<_> = reg.components().iter().map(|c| c.name).collect();
    assert_eq!(all, ["LightEmitter", "Transform"]);
}

#[test]
fn enums_resolve_items_once() {
    let reg = lights();
    let desc = reg.enum_desc("LightKind").expect("enum");
    assert_eq!(desc.number(2), Some(4));
    assert_eq!(
        reg.enum_item("LightKind", "Sun"),
        Some(EnumItem::new("LightKind", "Sun"))
    );
    assert_eq!(reg.enum_item("LightKind", "Moon"), None);
    assert_eq!(reg.enum_item("Material", "Sun"), None);
    assert!(reg.enum_desc("Material").is_none());
    assert_eq!(reg.enums().map(|e| e.name).collect::<Vec<_>>(), ["LightKind"]);
}

#[test]
fn defaults_are_shared_not_copied() {
    let reg = lights();
    let instance = id(&reg, "Instance");
    let name = reg
        .prop(instance, reg.prop_id(instance, "Name").expect("prop"))
        .expect("prop");
    let (Value::String(a), Value::String(b)) = (&name.default, &name.default.clone()) else {
        panic!("Name is a string");
    };
    assert!(Arc::ptr_eq(a, b));
}

#[test]
fn services_come_in_order_then_by_name() {
    let reg = lights();
    let services: Vec<_> = reg
        .services()
        .iter()
        .map(|&s| reg.class(s).map(ClassInfo::name).expect("class"))
        .collect();
    assert_eq!(services, ["ServerStorage", "Lighting", "Workspace"]);
    let workspace = reg.class(id(&reg, "Workspace")).expect("class");
    assert_eq!(workspace.service_order(), Some(1));
    assert!(workspace.persisted());
    assert!(!workspace.runs_scripts() && !workspace.hidden_in_explorer() && !workspace.creatable());
    assert!(reg.class(id(&reg, "SpotLight")).expect("class").creatable());
}

#[test]
fn override_replaces_in_place() {
    static BASE: ClassDesc =
        ClassDesc::new("Base", None).props(&[PropMeta::number("Size", 1.0), PropMeta::int("N", 0)]);
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Base")).props(&[PropMeta::number("Size", 4.0)
        .range(0.0, 10.0)
        .override_inherited()
        .desc("Bigger.")]);
    let reg = build(&[&BASE, &CHILD]).expect("valid");
    let (base, child) = (id(&reg, "Base"), id(&reg, "Child"));
    let size = reg.prop_id(base, "Size").expect("prop");
    assert_eq!(reg.prop_id(child, "Size"), Some(size));
    let prop = reg.prop(child, size).expect("prop");
    assert_eq!(prop.meta.description, "Bigger.");
    assert_eq!(prop.default, Value::Number(4.0));
    assert_eq!(prop.owner, child);
    assert_eq!(names(&reg, "Child"), ["Size", "N"]);
}

fn zero(_: &crate::DataModel, _: crate::Id) -> Value {
    Value::Number(0.0)
}

fn ignore(_: &mut crate::DataModel, _: crate::Id, _: Value) -> Result<(), ScriptError> {
    Ok(())
}

#[test]
fn derived_properties_resolve_into_dependents_per_source() {
    static BASE: ClassDesc = ClassDesc::new("Base", None).components(&["Transform"]).props(&[
        PropMeta::number("Size", 1.0),
        PropMeta::number("Volume", 0.0)
            .computed(zero, None)
            .derived_from(&["Size"]),
        PropMeta::number("Height", 0.0)
            .computed(zero, Some(ignore))
            .derived_from(&["CFrame"]),
        PropMeta::number("Tilt", 0.0)
            .computed(zero, Some(ignore))
            .derived_from(&["CFrame", "CFrame"]),
        PropMeta::number("Plain", 0.0),
    ]);
    // Inherits the table, and adds a dependent of an inherited source (before its own, in property order).
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Base")).props(&[PropMeta::number("Reach", 0.0)
        .computed(zero, None)
        .derived_from(&["Size", "CFrame"])]);
    let reg = Registry::build(RegistryInput {
        classes: &[&BASE, &CHILD],
        components: &[&TRANSFORM],
        ..RegistryInput::default()
    })
    .expect("valid");
    let (base, child) = (id(&reg, "Base"), id(&reg, "Child"));
    let prop = |name| reg.prop_id(child, name).expect("prop");
    let (cframe, size, volume, height, tilt, plain, reach) = (
        prop("CFrame"),
        prop("Size"),
        prop("Volume"),
        prop("Height"),
        prop("Tilt"),
        prop("Plain"),
        prop("Reach"),
    );

    assert_eq!(
        reg.dependents(base, cframe),
        [height, tilt],
        "a repeated source counts once"
    );
    assert_eq!(reg.dependents(base, size), [volume]);
    assert_eq!(reg.dependents(child, cframe), [height, tilt, reach]);
    assert_eq!(reg.dependents(child, size), [volume, reach]);
    for none in [volume, height, plain, PropId::new(999)] {
        assert!(reg.dependents(child, none).is_empty());
    }
    assert!(reg.dependents(ClassId::new(99), size).is_empty());
    // A class that derives nothing has empty tables.
    let lights = lights();
    let spot = id(&lights, "SpotLight");
    for p in 0..lights.props(spot).len() {
        let p = PropId::new(u16::try_from(p).expect("id"));
        assert!(lights.dependents(spot, p).is_empty());
    }
}

#[test]
fn rejects_bad_derived_sources() {
    static BASE: ClassDesc = ClassDesc::new("Base", None).props(&[
        PropMeta::number("Size", 1.0),
        PropMeta::number("Volume", 0.0).computed(zero, None),
    ]);
    static STORED: ClassDesc =
        ClassDesc::new("Stored", Some("Base")).props(&[PropMeta::number("Copy", 0.0).derived_from(&["Size"])]);
    static UNKNOWN: ClassDesc = ClassDesc::new("Unknown", Some("Base")).props(&[PropMeta::number("Area", 0.0)
        .computed(zero, None)
        .derived_from(&["Sise"])]);
    static FROM_COMPUTED: ClassDesc =
        ClassDesc::new("FromComputed", Some("Base")).props(&[PropMeta::number("Weight", 0.0)
            .computed(zero, None)
            .derived_from(&["Volume"])]);
    // A property of a subclass is not a property of the class: `Base` cannot derive from `Grown`'s `Extra`.
    static PARENT: ClassDesc = ClassDesc::new("Parent", Some("Base")).props(&[PropMeta::number("Spread", 0.0)
        .computed(zero, None)
        .derived_from(&["Extra"])]);
    static GROWN: ClassDesc = ClassDesc::new("Grown", Some("Parent")).props(&[PropMeta::number("Extra", 0.0)]);
    let found = class_errors(&[&BASE, &STORED, &UNKNOWN, &FROM_COMPUTED, &PARENT, &GROWN]);
    for expected in [
        RegistryError::DerivedStoredProp {
            class: "Stored",
            prop: "Copy",
        },
        RegistryError::UnknownDerivedSource {
            class: "Unknown",
            prop: "Area",
            from: "Sise",
        },
        RegistryError::DerivedFromComputed {
            class: "FromComputed",
            prop: "Weight",
            from: "Volume",
        },
        RegistryError::UnknownDerivedSource {
            class: "Parent",
            prop: "Spread",
            from: "Extra",
        },
    ] {
        assert!(found.contains(&expected), "{expected:?} missing from {found:?}");
    }
    assert_eq!(found.len(), 4, "{found:?}");
}

#[test]
fn rejects_an_override_that_makes_an_inherited_source_computed() {
    static BASE: ClassDesc = ClassDesc::new("Base", None).props(&[
        PropMeta::number("Size", 1.0),
        PropMeta::number("Volume", 0.0)
            .computed(zero, None)
            .derived_from(&["Size"]),
    ]);
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Base")).props(&[PropMeta::number("Size", 1.0)
        .computed(zero, Some(ignore))
        .override_inherited()]);
    assert_eq!(
        class_errors(&[&BASE, &CHILD]),
        [RegistryError::DerivedFromComputed {
            class: "Child",
            prop: "Volume",
            from: "Size",
        }]
    );
}

#[test]
fn changed_passes_value_is_inherited_and_resolved_per_class() {
    static OBJECT: ClassDesc = ClassDesc::new("Object", None).signals(&[SignalDesc::new("Changed")]);
    // Abstract: it has no `Value` itself, and passes the option down.
    static VALUE_BASE: ClassDesc = ClassDesc::new("ValueBase", Some("Object")).changed_passes_value("Value");
    static NUMBER: ClassDesc = ClassDesc::new("NumberValue", Some("ValueBase"))
        .creatable()
        .props(&[PropMeta::number("Value", 0.0)]);
    static FOLDER: ClassDesc = ClassDesc::new("Folder", Some("Object"))
        .creatable()
        .props(&[PropMeta::number("Value", 0.0)]);
    let reg = build(&[&OBJECT, &VALUE_BASE, &NUMBER, &FOLDER]).expect("valid");
    let class = |name| reg.class(id(&reg, name)).expect("class");
    assert_eq!(class("ValueBase").changed_value(), None);
    let number = class("NumberValue");
    assert_eq!(number.changed_value(), number.prop_id("Value"));
    assert!(number.changed_value().is_some());
    assert_eq!(
        class("Folder").changed_value(),
        None,
        "only classes that declare or inherit it"
    );
    assert_eq!(class("Object").changed_value(), None);
}

#[test]
fn rejects_bad_changed_passes_value() {
    static OBJECT: ClassDesc = ClassDesc::new("Object", None).signals(&[SignalDesc::new("Changed")]);
    static VALUE_BASE: ClassDesc = ClassDesc::new("ValueBase", Some("Object")).changed_passes_value("Value");
    // A creatable subclass that inherits the option without the property.
    static EMPTY: ClassDesc = ClassDesc::new("EmptyValue", Some("ValueBase")).creatable();
    static COMPUTED: ClassDesc = ClassDesc::new("ComputedValue", Some("ValueBase"))
        .creatable()
        .props(&[PropMeta::number("Value", 0.0).computed(zero, None)]);
    static SILENT: ClassDesc = ClassDesc::new("Silent", Some("Object"))
        .creatable()
        .hide(&["Changed"])
        .changed_passes_value("Level")
        .props(&[PropMeta::number("Level", 0.0)]);
    let found = class_errors(&[&OBJECT, &VALUE_BASE, &EMPTY, &COMPUTED, &SILENT]);
    for expected in [
        RegistryError::UnknownChangedValue {
            class: "EmptyValue",
            prop: "Value",
        },
        RegistryError::ChangedValueComputed {
            class: "ComputedValue",
            prop: "Value",
        },
        RegistryError::ChangedValueWithoutSignal {
            class: "Silent",
            prop: "Level",
        },
    ] {
        assert!(found.contains(&expected), "{expected:?} missing from {found:?}");
    }
    assert_eq!(found.len(), 3, "{found:?}");
}

#[test]
fn every_default_kind_converts() {
    static ENUMS: &[EnumDesc] = &[EnumDesc::new("Shape", &["Block", "Ball"])];
    static ALL: ClassDesc = ClassDesc::new("All", None).props(&[
        PropMeta::boolean("A", true),
        PropMeta::int("B", -3),
        PropMeta::number("C", 0.5),
        PropMeta::string("D", "hi"),
        PropMeta::vector2("E", 1.0, 2.0),
        PropMeta::vector3("F", 1.0, 2.0, 3.0),
        PropMeta::new("G", PropType::CFrame, DefaultValue::CFrame(1.0, 2.0, 3.0)),
        PropMeta::color3("H", 0.0, 0.5, 1.0),
        PropMeta::enumeration("I", "Shape", "Ball"),
        PropMeta::reference("J", "All"),
    ]);
    let reg = Registry::build(RegistryInput {
        classes: &[&ALL],
        enums: &[ENUMS],
        ..RegistryInput::default()
    })
    .expect("valid");
    let defaults: Vec<Value> = reg.props(ClassId::new(0)).iter().map(|p| p.default.clone()).collect();
    assert_eq!(
        defaults,
        [
            Value::Bool(true),
            Value::Int(-3),
            Value::Number(0.5),
            Value::String("hi".into()),
            Value::Vector2(crate::Vector2::new(1.0, 2.0)),
            Value::Vector3(crate::Vector3::new(1.0, 2.0, 3.0)),
            Value::CFrame(crate::CFrame::new(crate::Vector3::new(1.0, 2.0, 3.0))),
            Value::Color3(crate::Color3::new(0.0, 0.5, 1.0)),
            Value::Enum(EnumItem::new("Shape", "Ball")),
            Value::Ref(None),
        ]
    );
    for prop in reg.props(ClassId::new(0)) {
        assert!(prop.default.prop_type_matches(&prop.meta.ty), "{}", prop.meta.name);
    }
}

#[test]
fn an_empty_input_builds_an_empty_registry() {
    let reg = Registry::build(RegistryInput::default()).expect("empty is valid");
    assert!(reg.classes().is_empty());
    assert!(reg.services().is_empty());
    assert!(format!("{reg:?}").contains("classes: 0"));
}

// ---- validation: one test per error -------------------------------------------------------------------------------

#[test]
fn rejects_duplicate_class() {
    static AGAIN: ClassDesc = ClassDesc::new("Object", None);
    assert_eq!(
        class_errors(&[&ROOT, &AGAIN]),
        [RegistryError::DuplicateClass { class: "Object" }]
    );
}

#[test]
fn rejects_unknown_parent() {
    static ORPHAN: ClassDesc = ClassDesc::new("Orphan", Some("Nowhere"));
    static GRANDCHILD: ClassDesc = ClassDesc::new("Grandchild", Some("Orphan"));
    // The child of the broken class is not reported again.
    assert_eq!(
        class_errors(&[&ROOT, &ORPHAN, &GRANDCHILD]),
        [RegistryError::UnknownParent {
            class: "Orphan",
            parent: "Nowhere"
        }]
    );
}

#[test]
fn rejects_inheritance_cycle() {
    static A: ClassDesc = ClassDesc::new("A", Some("B"));
    static B: ClassDesc = ClassDesc::new("B", Some("A"));
    static UNDER: ClassDesc = ClassDesc::new("Under", Some("A"));
    assert_eq!(
        class_errors(&[&ROOT, &UNDER, &A, &B]),
        [RegistryError::InheritanceCycle {
            classes: vec!["A", "B", "A"]
        }]
    );
    static SELF: ClassDesc = ClassDesc::new("Selfish", Some("Selfish"));
    assert_eq!(
        class_errors(&[&ROOT, &SELF]),
        [RegistryError::InheritanceCycle {
            classes: vec!["Selfish", "Selfish"]
        }]
    );
}

#[test]
fn rejects_more_than_one_root() {
    static OTHER: ClassDesc = ClassDesc::new("Other", None);
    assert_eq!(
        class_errors(&[&ROOT, &OTHER]),
        [RegistryError::MultipleRoots {
            roots: vec!["Object", "Other"]
        }]
    );
}

#[test]
fn rejects_duplicate_property() {
    static C: ClassDesc = ClassDesc::new("C", None).props(&[PropMeta::number("X", 0.0), PropMeta::boolean("X", false)]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::DuplicateProp {
            owner: Owner::Class("C"),
            prop: "X"
        }]
    );
    static COMP: ComponentDesc =
        ComponentDesc::new("Twice").props(&[PropMeta::number("Y", 0.0), PropMeta::number("Y", 1.0)]);
    let errs = errors(RegistryInput {
        components: &[&COMP],
        ..RegistryInput::default()
    });
    assert_eq!(
        errs,
        [RegistryError::DuplicateProp {
            owner: Owner::Component("Twice"),
            prop: "Y"
        }]
    );
}

#[test]
fn rejects_redeclared_property_without_override() {
    static BASE: ClassDesc = ClassDesc::new("Base", None).props(&[PropMeta::number("Size", 1.0)]);
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Base")).props(&[PropMeta::number("Size", 2.0)]);
    assert_eq!(
        class_errors(&[&BASE, &CHILD]),
        [RegistryError::RedeclaredProp {
            owner: Owner::Class("Child"),
            class: "Child",
            prop: "Size",
            inherited_from: "Base"
        }]
    );
    // A component property clashing with an inherited one.
    static COMP: ComponentDesc = ComponentDesc::new("Sized").props(&[PropMeta::number("Size", 3.0)]);
    static WITH: ClassDesc = ClassDesc::new("With", Some("Base")).components(&["Sized"]);
    let errs = errors(RegistryInput {
        classes: &[&BASE, &WITH],
        components: &[&COMP],
        ..RegistryInput::default()
    });
    assert_eq!(
        errs,
        [RegistryError::RedeclaredProp {
            owner: Owner::Component("Sized"),
            class: "With",
            prop: "Size",
            inherited_from: "Base"
        }]
    );
}

#[test]
fn rejects_bad_overrides() {
    static BASE: ClassDesc = ClassDesc::new("Base", None).props(&[PropMeta::number("Size", 1.0)]);
    static NOTHING: ClassDesc =
        ClassDesc::new("Nothing", Some("Base")).props(&[PropMeta::number("Other", 1.0).override_inherited()]);
    static RETYPED: ClassDesc =
        ClassDesc::new("Retyped", Some("Base")).props(&[PropMeta::int("Size", 1).override_inherited()]);
    assert_eq!(
        class_errors(&[&BASE, &NOTHING, &RETYPED]),
        [
            RegistryError::OverridesNothing {
                owner: Owner::Class("Nothing"),
                class: "Nothing",
                prop: "Other"
            },
            RegistryError::OverrideChangesType {
                owner: Owner::Class("Retyped"),
                prop: "Size",
                expected: PropType::Number,
                found: PropType::Int
            }
        ]
    );
}

#[test]
fn rejects_hiding_or_defaulting_unknown_members() {
    static BASE: ClassDesc = ClassDesc::new("Base", None).props(&[PropMeta::number("Size", 1.0)]);
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Base"))
        .hide(&["Colour"])
        .defaults(&[("Mass", DefaultValue::Number(1.0))]);
    assert_eq!(
        class_errors(&[&BASE, &CHILD]),
        [
            RegistryError::UnknownDefault {
                class: "Child",
                prop: "Mass"
            },
            RegistryError::UnknownHidden {
                class: "Child",
                member: "Colour"
            },
        ]
    );
}

#[test]
fn rejects_default_of_the_wrong_type() {
    static C: ClassDesc =
        ClassDesc::new("C", None).props(&[PropMeta::new("X", PropType::Number, DefaultValue::Int(1))]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::DefaultTypeMismatch {
            owner: Owner::Class("C"),
            prop: "X",
            expected: PropType::Number,
            found: "integer"
        }]
    );
    static BASE: ClassDesc = ClassDesc::new("Base", None).props(&[PropMeta::boolean("On", true)]);
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Base")).defaults(&[("On", DefaultValue::Str("yes"))]);
    assert_eq!(
        class_errors(&[&BASE, &CHILD]),
        [RegistryError::DefaultTypeMismatch {
            owner: Owner::Class("Child"),
            prop: "On",
            expected: PropType::Bool,
            found: "string"
        }]
    );
}

#[test]
fn rejects_unknown_enum_and_enum_item() {
    static ENUMS: &[EnumDesc] = &[EnumDesc::new("Shape", &["Block"])];
    static C: ClassDesc = ClassDesc::new("C", None).props(&[
        PropMeta::enumeration("A", "Material", "Plastic"),
        PropMeta::enumeration("B", "Shape", "Ball"),
    ]);
    let errs = errors(RegistryInput {
        classes: &[&C],
        enums: &[ENUMS],
        ..RegistryInput::default()
    });
    assert_eq!(
        errs,
        [
            RegistryError::UnknownEnum {
                owner: Owner::Class("C"),
                prop: "A",
                enum_type: "Material"
            },
            RegistryError::UnknownEnumItem {
                owner: Owner::Class("C"),
                prop: "B",
                enum_type: "Shape",
                item: "Ball"
            }
        ]
    );
}

#[test]
fn rejects_reference_to_unknown_class() {
    static C: ClassDesc = ClassDesc::new("C", None).props(&[PropMeta::reference("Target", "BasePart")]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::UnknownRefClass {
            owner: Owner::Class("C"),
            prop: "Target",
            class: "BasePart"
        }]
    );
}

#[test]
fn rejects_bad_ranges() {
    static C: ClassDesc = ClassDesc::new("C", None).props(&[
        PropMeta::number("Upside", 5.0).range(10.0, 1.0),
        PropMeta::number("NaN", 5.0).range(f64::NAN, 1.0),
        PropMeta::int("Outside", 50).range(0.0, 10.0),
    ]);
    let errs = class_errors(&[&C]);
    assert_eq!(errs.len(), 3);
    assert_eq!(
        errs[0],
        RegistryError::InvalidRange {
            owner: Owner::Class("C"),
            prop: "Upside",
            min: 10.0,
            max: 1.0
        }
    );
    assert!(matches!(errs[1], RegistryError::InvalidRange { prop: "NaN", .. }));
    assert_eq!(
        errs[2],
        RegistryError::DefaultOutOfRange {
            owner: Owner::Class("C"),
            prop: "Outside",
            default: 50.0,
            min: 0.0,
            max: 10.0
        }
    );
    // `defaults(..)` is checked against the range too.
    static BASE: ClassDesc = ClassDesc::new("Base", None).props(&[PropMeta::number("Angle", 90.0).range(1.0, 179.0)]);
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Base")).defaults(&[("Angle", DefaultValue::Number(200.0))]);
    assert!(matches!(
        class_errors(&[&BASE, &CHILD])[..],
        [RegistryError::DefaultOutOfRange { prop: "Angle", .. }]
    ));
}

#[test]
fn rejects_unknown_component() {
    static C: ClassDesc = ClassDesc::new("C", None).components(&["Rigidbody"]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::UnknownComponent {
            class: "C",
            component: "Rigidbody"
        }]
    );
}

#[test]
fn rejects_unsatisfied_component_requirement() {
    static NEEDS: ComponentDesc = ComponentDesc::new("Needs").requires(&["Transform"]);
    static C: ClassDesc = ClassDesc::new("C", None).components(&["Needs"]);
    let errs = errors(RegistryInput {
        classes: &[&C],
        components: &[&NEEDS, &TRANSFORM],
        ..RegistryInput::default()
    });
    assert_eq!(
        errs,
        [RegistryError::MissingRequiredComponent {
            class: "C",
            component: "Needs",
            requires: "Transform"
        }]
    );
    // Listing the required component later in the same class, or inheriting it, satisfies it.
    static OK: ClassDesc = ClassDesc::new("Ok", None).components(&["Needs", "Transform"]);
    assert!(
        Registry::build(RegistryInput {
            classes: &[&OK],
            components: &[&NEEDS, &TRANSFORM],
            ..RegistryInput::default()
        })
        .is_ok()
    );
    static MISSING: ComponentDesc = ComponentDesc::new("Missing").requires(&["Nowhere"]);
    let errs = errors(RegistryInput {
        components: &[&MISSING],
        ..RegistryInput::default()
    });
    assert_eq!(
        errs,
        [RegistryError::UnknownRequiredComponent {
            component: "Missing",
            requires: "Nowhere"
        }]
    );
}

#[test]
fn rejects_duplicate_component() {
    static AGAIN: ComponentDesc = ComponentDesc::new("Transform");
    let errs = errors(RegistryInput {
        components: &[&TRANSFORM, &AGAIN],
        ..RegistryInput::default()
    });
    assert_eq!(errs, [RegistryError::DuplicateComponent { component: "Transform" }]);
}

#[test]
fn rejects_bad_enums() {
    static A: &[EnumDesc] = &[
        EnumDesc::new("Shape", &["Block", "Ball", "Block"]),
        EnumDesc::new("Key", &["A", "B"]).numbers(&[1]),
    ];
    static B: &[EnumDesc] = &[EnumDesc::new("Shape", &["Wedge"])];
    let errs = errors(RegistryInput {
        enums: &[A, B],
        ..RegistryInput::default()
    });
    assert_eq!(
        errs,
        [
            RegistryError::DuplicateEnumItem {
                name: "Shape",
                item: "Block"
            },
            RegistryError::EnumNumbersMismatch {
                name: "Key",
                items: 2,
                numbers: 1
            },
            RegistryError::DuplicateEnum { name: "Shape" },
        ]
    );
}

#[test]
fn rejects_alias_collision() {
    static C: ClassDesc = ClassDesc::new("C", None).props(&[
        PropMeta::number("Size", 1.0),
        PropMeta::number("Scale", 1.0).aliases(&["Size"]),
    ]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::AliasCollision {
            class: "C",
            alias: "Size",
            prop: "Scale",
            other: "Size"
        }]
    );
}

#[test]
fn rejects_duplicate_methods_and_signals() {
    static C: ClassDesc = ClassDesc::new("C", None)
        .methods(&[
            MethodDesc::new("Go", noop).returns_nothing(),
            MethodDesc::new("Go", other).returns_nothing(),
        ])
        .signals(&[SignalDesc::new("Went"), SignalDesc::new("Went")]);
    assert_eq!(
        class_errors(&[&C]),
        [
            RegistryError::DuplicateMember {
                class: "C",
                kind: MemberKind::Method,
                name: "Go"
            },
            RegistryError::DuplicateMember {
                class: "C",
                kind: MemberKind::Signal,
                name: "Went"
            },
        ]
    );
}

#[test]
fn rejects_members_of_different_kinds_with_one_name() {
    static BASE: ClassDesc = ClassDesc::new("Base", None).props(&[PropMeta::number("Size", 1.0)]);
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Base"))
        .methods(&[MethodDesc::new("Size", noop).returns_nothing()])
        .signals(&[SignalDesc::new("Size")]);
    assert_eq!(
        class_errors(&[&BASE, &CHILD]),
        [
            RegistryError::MemberNameClash {
                class: "Child",
                name: "Size",
                first: MemberKind::Property,
                second: MemberKind::Method
            },
            RegistryError::MemberNameClash {
                class: "Child",
                name: "Size",
                first: MemberKind::Property,
                second: MemberKind::Signal
            },
        ]
    );
    // Hiding the inherited property frees the name.
    static HIDING: ClassDesc = ClassDesc::new("Hiding", Some("Base"))
        .hide(&["Size"])
        .methods(&[MethodDesc::new("Size", noop).returns_nothing()]);
    assert!(build(&[&BASE, &HIDING]).is_ok());
}

#[test]
fn rejects_too_many_classes() {
    let classes = vec![&ROOT; usize::from(u16::MAX) + 1];
    assert_eq!(class_errors(&classes), [RegistryError::TooManyClasses { count: 65536 }]);
}

#[test]
fn rejects_too_many_properties() {
    let props: Vec<PropMeta> = (0..=usize::from(u16::MAX))
        .map(|i| PropMeta::boolean(Box::leak(format!("P{i}").into_boxed_str()), false))
        .collect();
    let class: &'static ClassDesc = Box::leak(Box::new(
        ClassDesc::new("Huge", None).props(Box::leak(props.into_boxed_slice())),
    ));
    assert_eq!(
        class_errors(&[class]),
        [RegistryError::TooManyProps {
            class: "Huge",
            count: 65536
        }]
    );
}

#[test]
fn collects_every_problem_at_once() {
    static ORPHAN: ClassDesc = ClassDesc::new("Orphan", Some("Nowhere"));
    static BAD_PROP: ClassDesc = ClassDesc::new("BadProp", Some("Object")).props(&[PropMeta::reference("R", "Nope")]);
    static BAD_COMPONENT: ClassDesc = ClassDesc::new("BadComponent", Some("Object")).components(&["Nope"]);
    let errs = class_errors(&[&ROOT, &ORPHAN, &BAD_PROP, &BAD_COMPONENT]);
    assert_eq!(errs.len(), 3, "{errs:#?}");
}

// Method and signal signatures.

#[test]
fn signatures_resolve_with_their_types() {
    static SHAPE: &[EnumDesc] = &[EnumDesc::new("Shape", &["Block"])];
    static C: ClassDesc = ClassDesc::new("C", None)
        .methods(&[MethodDesc::new("Find", noop)
            .params(&[
                Param::new("name", Ty::String),
                Param::new("recursive", Ty::Bool).optional(),
                Param::new("rest", Ty::Variadic(&Ty::Any)),
            ])
            .returns(Ty::Tuple(&[
                Ty::Optional(&Ty::Class("C")),
                Ty::Map(&Ty::String, &Ty::Enum("Shape")),
            ]))])
        .signals(&[SignalDesc::new("Moved").params(&[
            Param::new("child", Ty::Class("C")),
            Param::new("parent", Ty::Optional(&Ty::Class("C"))),
        ])]);
    let reg = Registry::build(RegistryInput {
        classes: &[&C],
        enums: &[SHAPE],
        ..RegistryInput::default()
    })
    .expect("valid");
    let class = id(&reg, "C");
    let find = reg.method(class, "Find").expect("method");
    assert_eq!(
        find.returns.map(|t| t.to_string()).as_deref(),
        Some("(C?, {[string]: Enum.Shape})")
    );
    let params: Vec<String> = find
        .params
        .unwrap_or_default()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(params, ["name: string", "recursive: boolean?", "...: any"]);
    assert_eq!(
        reg.signal(class, "Moved").and_then(|s| s.params).map(<[Param]>::len),
        Some(2)
    );
}

#[test]
fn rejects_a_method_without_returns() {
    static C: ClassDesc = ClassDesc::new("C", None).methods(&[MethodDesc::new("Go", noop).params(&[])]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::MissingReturns {
            class: "C",
            method: "Go"
        }]
    );
}

#[test]
fn rejects_an_unknown_class_in_a_type() {
    static C: ClassDesc = ClassDesc::new("C", None)
        .methods(&[MethodDesc::new("Get", noop).returns(Ty::Array(&Ty::Class("Nope")))])
        .signals(&[SignalDesc::new("Hit").params(&[Param::new("part", Ty::Optional(&Ty::Class("Part")))])]);
    assert_eq!(
        class_errors(&[&C]),
        [
            RegistryError::UnknownTypeClass {
                class: "C",
                kind: MemberKind::Method,
                member: "Get",
                name: "Nope"
            },
            RegistryError::UnknownTypeClass {
                class: "C",
                kind: MemberKind::Signal,
                member: "Hit",
                name: "Part"
            },
        ]
    );
}

#[test]
fn rejects_an_unknown_enum_in_a_type() {
    static C: ClassDesc = ClassDesc::new("C", None).methods(&[MethodDesc::new("Set", noop)
        .params(&[Param::new("material", Ty::Enum("Material"))])
        .returns_nothing()]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::UnknownTypeEnum {
            class: "C",
            kind: MemberKind::Method,
            member: "Set",
            enum_type: "Material"
        }]
    );
}

#[test]
fn rejects_a_required_param_after_an_optional_one() {
    static C: ClassDesc =
        ClassDesc::new("C", None)
            .signals(&[SignalDesc::new("Went")
                .params(&[Param::new("from", Ty::String).optional(), Param::new("to", Ty::String)])]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::RequiredParamAfterOptional {
            class: "C",
            kind: MemberKind::Signal,
            member: "Went",
            param: "to"
        }]
    );
    // A variadic after an optional parameter is fine: it can be empty.
    static OK: ClassDesc = ClassDesc::new("C", None).signals(&[SignalDesc::new("Went").params(&[
        Param::new("from", Ty::String).optional(),
        Param::new("rest", Ty::Variadic(&Ty::Any)),
    ])]);
    assert!(build(&[&OK]).is_ok());
}

#[test]
fn rejects_a_variadic_that_is_not_last() {
    static C: ClassDesc = ClassDesc::new("C", None).methods(&[MethodDesc::new("Fire", noop)
        .params(&[Param::new("args", Ty::Variadic(&Ty::Any)), Param::new("last", Ty::Bool)])
        .returns_nothing()]);
    assert_eq!(
        class_errors(&[&C]),
        [RegistryError::VariadicNotLast {
            class: "C",
            kind: MemberKind::Method,
            member: "Fire",
            param: "args"
        }]
    );
}

#[test]
fn rejects_a_variadic_in_a_return_or_inside_a_type() {
    static C: ClassDesc = ClassDesc::new("C", None)
        .methods(&[MethodDesc::new("Get", noop).params(&[]).returns(Ty::Variadic(&Ty::Any))])
        .signals(&[SignalDesc::new("Got").params(&[Param::new("values", Ty::Array(&Ty::Variadic(&Ty::Any)))])]);
    assert_eq!(
        class_errors(&[&C]),
        [
            RegistryError::MisplacedVariadic {
                class: "C",
                kind: MemberKind::Method,
                member: "Get",
                ty: Ty::Variadic(&Ty::Any)
            },
            RegistryError::MisplacedVariadic {
                class: "C",
                kind: MemberKind::Signal,
                member: "Got",
                ty: Ty::Array(&Ty::Variadic(&Ty::Any))
            },
        ]
    );
}

#[test]
fn rejects_a_tuple_outside_a_return() {
    static C: ClassDesc = ClassDesc::new("C", None).methods(&[MethodDesc::new("Get", noop)
        .params(&[Param::new("pair", Ty::Tuple(&[Ty::Bool, Ty::Bool]))])
        .returns(Ty::Optional(&Ty::NOTHING))]);
    assert_eq!(
        class_errors(&[&C]),
        [
            RegistryError::MisplacedTuple {
                class: "C",
                kind: MemberKind::Method,
                member: "Get",
                ty: Ty::Tuple(&[Ty::Bool, Ty::Bool])
            },
            RegistryError::MisplacedTuple {
                class: "C",
                kind: MemberKind::Method,
                member: "Get",
                ty: Ty::Optional(&Ty::NOTHING)
            },
        ]
    );
}

#[test]
fn signature_messages_name_the_member_and_the_fix() {
    let err = RegistryError::MisplacedVariadic {
        class: "Part",
        kind: MemberKind::Method,
        member: "Get",
        ty: Ty::Array(&Ty::Variadic(&Ty::Any)),
    };
    assert_eq!(
        err.to_string(),
        "class `Part` declares method `Get` with type `{...any}`; a variadic (`...T`) can only be the whole type of \
         the last parameter"
    );
    let missing = RegistryError::MissingReturns {
        class: "Part",
        method: "Go",
    };
    assert!(missing.to_string().contains(".returns_nothing()"));
}

#[test]
fn callbacks_resolve_like_methods() {
    static ARGS: &[Param] = &[Param::new("args", Ty::Variadic(&Ty::Any))];
    static BINDABLE: ClassDesc = ClassDesc::new("Bindable", None)
        .methods(&[MethodDesc::new("Invoke", noop)
            .params(ARGS)
            .returns(Ty::Any)
            .invokes_callback("OnInvoke")])
        .callbacks(&[CallbackDesc::new("OnInvoke")
            .params(ARGS)
            .returns(Ty::Any)
            .describe("Runs on Invoke.")]);
    static CHILD: ClassDesc = ClassDesc::new("Child", Some("Bindable"));
    let registry = build(&[&BINDABLE, &CHILD]).expect("builds");
    let child = registry.class_id("Child").expect("Child");
    let on_invoke = registry.callback(child, "OnInvoke").expect("inherited");
    assert_eq!(on_invoke.params, Some(ARGS));
    assert_eq!(on_invoke.description, "Runs on Invoke.");
    assert_eq!(registry.class(child).map(|c| c.callbacks().len()), Some(1));
    assert_eq!(
        registry.method(child, "Invoke").and_then(|m| m.invokes),
        Some("OnInvoke")
    );
    assert!(registry.callback(child, "Invoke").is_none());
    assert_eq!(MemberKind::Callback.to_string(), "callback");
}

#[test]
fn rejects_bad_callbacks() {
    static C: ClassDesc = ClassDesc::new("C", None)
        .props(&[PropMeta::number("Size", 1.0)])
        .methods(&[MethodDesc::new("Call", noop)
            .returns_nothing()
            .invokes_callback("OnCall")])
        .callbacks(&[
            CallbackDesc::new("Size"),
            CallbackDesc::new("OnGo"),
            CallbackDesc::new("OnGo"),
        ]);
    assert_eq!(
        class_errors(&[&C]),
        [
            RegistryError::DuplicateMember {
                class: "C",
                kind: MemberKind::Callback,
                name: "OnGo"
            },
            RegistryError::MemberNameClash {
                class: "C",
                name: "Size",
                first: MemberKind::Property,
                second: MemberKind::Callback
            },
            RegistryError::UnknownInvokedCallback {
                class: "C",
                method: "Call",
                callback: "OnCall"
            },
        ]
    );
    // Hiding the callback a method invokes breaks the method.
    static BASE: ClassDesc = ClassDesc::new("Base", None)
        .methods(&[MethodDesc::new("Call", noop)
            .returns_nothing()
            .invokes_callback("OnCall")])
        .callbacks(&[CallbackDesc::new("OnCall")]);
    static HIDING: ClassDesc = ClassDesc::new("Hiding", Some("Base")).hide(&["OnCall"]);
    assert_eq!(
        class_errors(&[&BASE, &HIDING]),
        [RegistryError::UnknownInvokedCallback {
            class: "Hiding",
            method: "Call",
            callback: "OnCall"
        }]
    );
}
