//! A small class set with real descriptors, shared by the data model's unit tests, the integration tests and the
//! benches (each includes this file with `#[path]`).
//!
//! `Object` → `Instance` (computed `Name` and `Parent`, `Archivable`) → `Part` / `Folder`, plus the root `Game`
//! and the services `Workspace` and `Lighting`. `Part` has the properties a renderer and physics read every frame.
//! `Object` and `Instance` declare the built-in signals; `Bindable` has a signal class code fires (`Event`).
//!
//! The perf gate copies this file onto the base branch, so it only uses descriptor API both sides have: signals
//! leave their parameter types undeclared, and there are no methods (a method must declare its signature).
#![allow(dead_code, reason = "each including crate uses a different part of the set")]

use std::sync::Arc;

use metallic_core::{
    CFrame, ClassDesc, ClassId, DataModel, EnumDesc, Id, PropId, PropMeta, Registry, RegistryInput, ScriptError,
    SignalDesc, SignalId, Value,
};

pub(crate) static ENUMS: &[EnumDesc] = &[EnumDesc::new("Material", &["Plastic", "Wood", "Neon", "Metal"])];

fn name_get(dm: &DataModel, id: Id) -> Value {
    Value::String(dm.shared_name(id).unwrap_or_else(|_| Arc::from("")))
}

fn name_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    match value {
        Value::String(name) => Ok(dm.set_name(id, name)?),
        _ => Err(ScriptError::wrong_type("string", "other")),
    }
}

fn parent_get(dm: &DataModel, id: Id) -> Value {
    Value::Ref(dm.parent(id).ok().flatten())
}

fn parent_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    match value {
        Value::Ref(parent) => Ok(dm.set_parent(id, parent)?),
        _ => Err(ScriptError::wrong_type("Instance", "other")),
    }
}

fn position_get(dm: &DataModel, id: Id) -> Value {
    let position = dm
        .prop_id(id, "CFrame")
        .and_then(|p| dm.cframe(id, p))
        .map_or(metallic_core::Vector3::ZERO, CFrame::position);
    Value::Vector3(position)
}

fn position_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    let Value::Vector3(position) = value else {
        return Err(ScriptError::wrong_type("Vector3", "other"));
    };
    let prop = dm.prop_id(id, "CFrame")?;
    let rotation = dm.cframe(id, prop)?.rotation();
    Ok(dm.set(id, prop, Value::CFrame(CFrame::from_parts(position, rotation)))?)
}

fn mass_get(dm: &DataModel, id: Id) -> Value {
    let size = dm
        .prop_id(id, "Size")
        .and_then(|p| dm.vector3(id, p))
        .unwrap_or_default();
    Value::Number(f64::from(size.x * size.y * size.z))
}

pub(crate) static OBJECT: ClassDesc = ClassDesc::new("Object", None).signals(&[SignalDesc::new("Changed")]);

pub(crate) static INSTANCE: ClassDesc = ClassDesc::new("Instance", Some("Object"))
    .props(&[
        PropMeta::string("Name", "Instance").computed(name_get, Some(name_set)),
        PropMeta::reference("Parent", "Instance").computed(parent_get, Some(parent_set)),
        PropMeta::boolean("Archivable", true),
    ])
    .signals(&[
        SignalDesc::new("ChildAdded"),
        SignalDesc::new("ChildRemoved"),
        SignalDesc::new("DescendantAdded"),
        SignalDesc::new("DescendantRemoving"),
        SignalDesc::new("AncestryChanged"),
        SignalDesc::new("AttributeChanged"),
        SignalDesc::new("Destroying"),
    ]);

pub(crate) static PART: ClassDesc = ClassDesc::new("Part", Some("Instance")).creatable().props(&[
    PropMeta::cframe("CFrame").transform().physics(),
    PropMeta::vector3("Size", 4.0, 1.0, 2.0)
        .range(0.05, 2048.0)
        .transform()
        .physics(),
    PropMeta::color3("Color", 0.64, 0.64, 0.64).visual(),
    PropMeta::number("Transparency", 0.0).range(0.0, 1.0).visual(),
    PropMeta::number("Reflectance", 0.0).range(0.0, 1.0).visual(),
    PropMeta::boolean("Anchored", false).physics(),
    PropMeta::boolean("CanCollide", true).physics(),
    PropMeta::enumeration("Material", "Material", "Plastic")
        .visual()
        .physics(),
    PropMeta::int("Layer", 0).range(0.0, 31.0),
    PropMeta::reference("Target", "Part"),
    PropMeta::vector3("Position", 0.0, 0.0, 0.0).computed(position_get, Some(position_set)),
    PropMeta::number("Mass", 0.0).computed(mass_get, None).read_only(),
]);

pub(crate) static FOLDER: ClassDesc = ClassDesc::new("Folder", Some("Instance"))
    .creatable()
    .props(&[PropMeta::string("Note", ""), PropMeta::reference("Link", "Instance")]);

pub(crate) static BINDABLE: ClassDesc = ClassDesc::new("Bindable", Some("Instance"))
    .creatable()
    .signals(&[SignalDesc::new("Event")]);

pub(crate) static GAME: ClassDesc = ClassDesc::new("Game", Some("Instance"));

pub(crate) static WORKSPACE: ClassDesc = ClassDesc::new("Workspace", Some("Instance")).service(1);

pub(crate) static LIGHTING: ClassDesc = ClassDesc::new("Lighting", Some("Instance")).service(2);

/// Every class in the set.
pub(crate) static CLASSES: &[&ClassDesc] = &[
    &OBJECT, &INSTANCE, &PART, &FOLDER, &BINDABLE, &GAME, &WORKSPACE, &LIGHTING,
];

/// The registry of the set plus `extra` classes.
pub(crate) fn registry_with(extra: &[&'static ClassDesc]) -> Arc<Registry> {
    let mut classes: Vec<&'static ClassDesc> = CLASSES.to_vec();
    classes.extend_from_slice(extra);
    let input = RegistryInput {
        classes: &classes,
        enums: &[ENUMS],
        ..RegistryInput::default()
    };
    match Registry::build(input) {
        Ok(registry) => Arc::new(registry),
        Err(errors) => panic!("test classes are invalid: {errors}"),
    }
}

/// The ids tests and benches use, looked up once.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ids {
    pub(crate) game_class: ClassId,
    pub(crate) workspace_class: ClassId,
    pub(crate) lighting_class: ClassId,
    pub(crate) part: ClassId,
    pub(crate) folder: ClassId,
    pub(crate) bindable: ClassId,
    pub(crate) instance: ClassId,
    pub(crate) name: PropId,
    pub(crate) parent: PropId,
    pub(crate) archivable: PropId,
    pub(crate) cframe: PropId,
    pub(crate) size: PropId,
    pub(crate) color: PropId,
    pub(crate) transparency: PropId,
    pub(crate) reflectance: PropId,
    pub(crate) anchored: PropId,
    pub(crate) can_collide: PropId,
    pub(crate) material: PropId,
    pub(crate) layer: PropId,
    pub(crate) target: PropId,
    pub(crate) position: PropId,
    pub(crate) mass: PropId,
    pub(crate) note: PropId,
    pub(crate) link: PropId,
    pub(crate) changed: SignalId,
    pub(crate) child_added: SignalId,
    pub(crate) child_removed: SignalId,
    pub(crate) descendant_added: SignalId,
    pub(crate) descendant_removing: SignalId,
    pub(crate) ancestry_changed: SignalId,
    pub(crate) attribute_changed: SignalId,
    pub(crate) destroying: SignalId,
    /// `Bindable.Event`.
    pub(crate) event: SignalId,
    /// The root.
    pub(crate) game: Id,
    pub(crate) workspace: Id,
    pub(crate) lighting: Id,
}

fn class(registry: &Registry, name: &str) -> ClassId {
    registry.class_id(name).unwrap_or_else(|| panic!("no class {name}"))
}

fn signal(registry: &Registry, class: ClassId, name: &str) -> SignalId {
    registry
        .signal_id(class, name)
        .unwrap_or_else(|| panic!("no signal {name}"))
}

fn prop(registry: &Registry, class: ClassId, name: &str) -> PropId {
    registry
        .prop_id(class, name)
        .unwrap_or_else(|| panic!("no property {name}"))
}

/// A data model over `registry` with the root and both services created.
pub(crate) fn game_with(registry: Arc<Registry>) -> (DataModel, Ids) {
    let r = &*registry;
    let (part, folder) = (class(r, "Part"), class(r, "Folder"));
    let bindable = class(r, "Bindable");
    let (game_class, workspace_class, lighting_class) = (class(r, "Game"), class(r, "Workspace"), class(r, "Lighting"));
    let mut dm = DataModel::new(Arc::clone(&registry));
    let game = dm.create_root(game_class).unwrap_or_else(|e| panic!("{e}"));
    let workspace = dm.create_service(workspace_class).unwrap_or_else(|e| panic!("{e}"));
    let lighting = dm.create_service(lighting_class).unwrap_or_else(|e| panic!("{e}"));
    let ids = Ids {
        game_class,
        workspace_class,
        lighting_class,
        part,
        folder,
        bindable,
        instance: class(r, "Instance"),
        name: prop(r, part, "Name"),
        parent: prop(r, part, "Parent"),
        archivable: prop(r, part, "Archivable"),
        cframe: prop(r, part, "CFrame"),
        size: prop(r, part, "Size"),
        color: prop(r, part, "Color"),
        transparency: prop(r, part, "Transparency"),
        reflectance: prop(r, part, "Reflectance"),
        anchored: prop(r, part, "Anchored"),
        can_collide: prop(r, part, "CanCollide"),
        material: prop(r, part, "Material"),
        layer: prop(r, part, "Layer"),
        target: prop(r, part, "Target"),
        position: prop(r, part, "Position"),
        mass: prop(r, part, "Mass"),
        note: prop(r, folder, "Note"),
        link: prop(r, folder, "Link"),
        changed: signal(r, part, "Changed"),
        child_added: signal(r, part, "ChildAdded"),
        child_removed: signal(r, part, "ChildRemoved"),
        descendant_added: signal(r, part, "DescendantAdded"),
        descendant_removing: signal(r, part, "DescendantRemoving"),
        ancestry_changed: signal(r, part, "AncestryChanged"),
        attribute_changed: signal(r, part, "AttributeChanged"),
        destroying: signal(r, part, "Destroying"),
        event: signal(r, bindable, "Event"),
        game,
        workspace,
        lighting,
    };
    (dm, ids)
}

/// A data model over the set alone, with the root and both services created.
pub(crate) fn game() -> (DataModel, Ids) {
    game_with(registry_with(&[]))
}
