//! `Instance`: anything that lives in the game tree. It has a name and a parent, can be found, copied and
//! destroyed, and carries attributes.

use std::sync::Arc;

use metallic_core::{
    ClassDesc, DataModel, Id, MethodCall, MethodDesc, PropFlags, PropMeta, ScriptError, ScriptValue, SignalDesc, Value,
};

use crate::script_args::class_arg;

fn name_get(dm: &DataModel, id: Id) -> Value {
    Value::String(dm.shared_name(id).unwrap_or_else(|_| Arc::from("")))
}

fn name_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    match value {
        Value::String(name) => Ok(dm.set_name(id, name)?),
        other => Err(ScriptError::wrong_type("string", ScriptValue::from(other).type_name())),
    }
}

fn parent_get(dm: &DataModel, id: Id) -> Value {
    Value::Ref(dm.parent(id).ok().flatten())
}

fn parent_set(dm: &mut DataModel, id: Id, value: Value) -> Result<(), ScriptError> {
    match value {
        Value::Ref(parent) => Ok(dm.set_parent(id, parent)?),
        other => Err(ScriptError::wrong_type("Instance", ScriptValue::from(other).type_name())),
    }
}

/// An instance, or nil.
fn instance_or_nil(id: Option<Id>) -> ScriptValue {
    id.map_or(ScriptValue::Nil, ScriptValue::Instance)
}

fn instance_list(ids: impl IntoIterator<Item = Id>) -> ScriptValue {
    ScriptValue::List(ids.into_iter().map(ScriptValue::Instance).collect())
}

fn destroy(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    call.dm.destroy(call.this)?;
    Ok(ScriptValue::Nil)
}

fn clone(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(instance_or_nil(call.dm.clone_tree(call.this)?))
}

fn get_children(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(instance_list(call.dm.children(call.this)?.iter().copied()))
}

fn get_descendants(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(instance_list(call.dm.descendants(call.this)?))
}

fn find_first_child(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let name = call.args.string(0)?;
    let recursive = call.args.opt_bool(1)?.unwrap_or(false);
    Ok(instance_or_nil(call.dm.find_first_child(call.this, &name, recursive)?))
}

fn find_first_child_of_class(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let class = class_arg(call, 0)?;
    Ok(instance_or_nil(call.dm.find_first_child_of_class(call.this, class, false)?))
}

fn find_first_child_which_is_a(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let class = class_arg(call, 0)?;
    let recursive = call.args.opt_bool(1)?.unwrap_or(false);
    Ok(instance_or_nil(
        call.dm.find_first_child_which_is_a(call.this, class, recursive)?,
    ))
}

fn find_first_ancestor(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let name = call.args.string(0)?;
    let mut current = call.dm.parent(call.this)?;
    while let Some(ancestor) = current {
        if call.dm.name(ancestor)? == &*name {
            return Ok(ScriptValue::Instance(ancestor));
        }
        current = call.dm.parent(ancestor)?;
    }
    Ok(ScriptValue::Nil)
}

fn is_descendant_of(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let other = call.args.instance(0)?;
    Ok(call.dm.is_descendant_of(call.this, other)?.into())
}

fn is_ancestor_of(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let other = call.args.instance(0)?;
    Ok(call.dm.is_descendant_of(other, call.this)?.into())
}

fn get_full_name(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    Ok(call.dm.full_name(call.this)?.as_str().into())
}

fn clear_all_children(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    call.dm.destroy_children(call.this)?;
    Ok(ScriptValue::Nil)
}

fn get_attribute(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let name = call.args.string(0)?;
    Ok(call
        .dm
        .attribute(call.this, &name)?
        .cloned()
        .map_or(ScriptValue::Nil, ScriptValue::from))
}

/// The attribute value a script passed, or `None` (remove the attribute) for nil or no value. Instances are passed
/// through so the data model refuses them with its own message.
fn attribute_value(value: Option<&ScriptValue>) -> Result<Option<Value>, ScriptError> {
    let Some(value) = value else {
        return Ok(None);
    };
    Ok(Some(match value {
        ScriptValue::Nil => return Ok(None),
        ScriptValue::Bool(b) => Value::Bool(*b),
        ScriptValue::Number(n) => Value::Number(*n),
        ScriptValue::String(s) => Value::String(Arc::clone(s)),
        ScriptValue::Vector2(v) => Value::Vector2(*v),
        ScriptValue::Vector3(v) => Value::Vector3(*v),
        ScriptValue::CFrame(cf) => Value::CFrame(*cf),
        ScriptValue::Color3(c) => Value::Color3(*c),
        ScriptValue::Enum(item) => Value::Enum(item.clone()),
        ScriptValue::Instance(id) => Value::Ref(Some(*id)),
        ScriptValue::List(_) | ScriptValue::Map(_) | ScriptValue::Tuple(_) => {
            return Err(ScriptError::bad_argument(
                2,
                "an attribute value (boolean, number, string, vector, CFrame, Color3 or EnumItem)",
                value.type_name(),
            ));
        }
    }))
}

fn set_attribute(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let name = call.args.string(0)?;
    let value = attribute_value(call.args.get(1))?;
    call.dm.set_attribute(call.this, &name, value)?;
    Ok(ScriptValue::Nil)
}

fn get_attributes(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let entries = call
        .dm
        .attributes(call.this)?
        .iter()
        .map(|(name, value)| (Arc::clone(name), ScriptValue::from(value.clone())))
        .collect();
    Ok(ScriptValue::Map(entries))
}

static PROPS: &[PropMeta] = &[
    // The scene format stores names and the tree itself, so neither goes through the generic property path.
    PropMeta::string("Name", "Instance")
        .computed(name_get, Some(name_set))
        .flags(PropFlags::NO_SAVE)
        .order(-2)
        .desc("The instance's name. Names do not have to be unique; FindFirstChild returns the first match."),
    PropMeta::reference("Parent", "Instance")
        .computed(parent_get, Some(parent_set))
        .flags(PropFlags::NO_SAVE)
        .order(-1)
        .desc("The instance this one sits under, or nil when it is outside the game. Setting it moves the instance."),
    PropMeta::boolean("Archivable", true)
        .desc("Whether the instance is copied by Clone and saved with the game. Turn it off for temporary objects."),
];

static METHODS: &[MethodDesc] = &[
    MethodDesc::new("Destroy", destroy).describe(
        "Removes the instance and everything under it from the game for good. It cannot be used again afterwards.",
    ),
    MethodDesc::new("Clone", clone).describe(
        "Returns a copy of the instance and everything under it, with no parent. Returns nil when Archivable is off; \
         children with Archivable off are left out.",
    ),
    MethodDesc::new("GetChildren", get_children)
        .describe("Returns a list of the instances directly under this one, in order."),
    MethodDesc::new("GetDescendants", get_descendants).describe(
        "Returns a list of every instance under this one, at any depth, parents before their children.",
    ),
    MethodDesc::new("FindFirstChild", find_first_child).describe(
        "Returns the first child with the given name, or nil. Pass true as the second argument to search every \
         descendant.",
    ),
    MethodDesc::new("FindFirstChildOfClass", find_first_child_of_class).describe(
        "Returns the first child whose class is exactly the given class, or nil. Subclasses do not count; use \
         FindFirstChildWhichIsA for that.",
    ),
    MethodDesc::new("FindFirstChildWhichIsA", find_first_child_which_is_a).describe(
        "Returns the first child whose class is the given class or inherits from it, or nil. Pass true as the \
         second argument to search every descendant.",
    ),
    MethodDesc::new("FindFirstAncestor", find_first_ancestor)
        .describe("Returns the nearest instance above this one with the given name, or nil."),
    MethodDesc::new("IsDescendantOf", is_descendant_of)
        .describe("Returns true if this instance is somewhere under the given instance."),
    MethodDesc::new("IsAncestorOf", is_ancestor_of)
        .describe("Returns true if the given instance is somewhere under this one."),
    MethodDesc::new("GetFullName", get_full_name).describe(
        "Returns the instance's path of names from its service down, separated by dots, such as \
         \"Workspace.Map.Door\".",
    ),
    MethodDesc::new("ClearAllChildren", clear_all_children)
        .describe("Destroys every child of the instance (and everything under them). The instance itself stays."),
    MethodDesc::new("GetAttribute", get_attribute)
        .describe("Returns the value of the attribute with the given name, or nil when it is not set."),
    MethodDesc::new("SetAttribute", set_attribute).describe(
        "Sets an attribute: a custom named value stored on the instance and saved with it. Setting nil removes it. \
         Attributes hold booleans, numbers, strings, vectors, CFrames, colours and enum items, not instances.",
    ),
    MethodDesc::new("GetAttributes", get_attributes)
        .describe("Returns every attribute of the instance as a table of names to values."),
];

static SIGNALS: &[SignalDesc] = &[
    SignalDesc::new("ChildAdded")
        .params(&[("child", "Instance")])
        .describe("Fires after an instance is placed directly under this one."),
    SignalDesc::new("ChildRemoved")
        .params(&[("child", "Instance")])
        .describe("Fires after an instance directly under this one is moved away or destroyed."),
    SignalDesc::new("DescendantAdded")
        .params(&[("descendant", "Instance")])
        .describe("Fires after an instance is placed anywhere under this one, at any depth."),
    SignalDesc::new("DescendantRemoving")
        .params(&[("descendant", "Instance")])
        .describe("Fires just before an instance anywhere under this one is moved away or destroyed."),
    SignalDesc::new("AncestryChanged")
        .params(&[("child", "Instance"), ("parent", "Instance")])
        .describe(
            "Fires when this instance or one of its ancestors gets a new parent, with the instance that moved and \
             its new parent.",
        ),
    SignalDesc::new("AttributeChanged")
        .params(&[("attributeName", "string")])
        .describe("Fires after an attribute is set, changed or removed, with the attribute's name."),
    SignalDesc::new("Destroying").describe("Fires when the instance is about to be destroyed, while it is still intact."),
];

/// Anything that lives in the game tree.
pub static CLASS: ClassDesc = ClassDesc::new("Instance", Some("Object"))
    .describe(
        "The base of everything in the game tree. An instance has a name and a parent, can be searched, copied and \
         destroyed, and can carry custom attributes.",
    )
    .props(PROPS)
    .methods(METHODS)
    .signals(SIGNALS);

#[cfg(test)]
mod tests {
    use metallic_core::{Color3, ScriptValue, Value, Vector3};

    use crate::testing::{call, game, new_in, workspace};

    fn instance(id: metallic_core::Id) -> ScriptValue {
        ScriptValue::Instance(id)
    }

    fn s(text: &str) -> ScriptValue {
        ScriptValue::from(text)
    }

    #[test]
    fn name_parent_and_class_name_are_properties() {
        let mut dm = game();
        let ws = workspace(&dm);
        let folder = new_in(&mut dm, "Folder", ws, "Folder");
        assert_eq!(dm.get_by_name(folder, "Name"), Ok(Value::String("Folder".into())));
        assert_eq!(dm.get_by_name(folder, "ClassName"), Ok(Value::String("Folder".into())));
        assert_eq!(dm.get_by_name(folder, "Parent"), Ok(Value::Ref(Some(ws))));

        dm.set_by_name(folder, "Name", Value::String("Map".into())).unwrap();
        assert_eq!(dm.name(folder), Ok("Map"));
        dm.set_by_name(folder, "Parent", Value::Ref(None)).unwrap();
        assert_eq!(dm.parent(folder), Ok(None));
        assert!(
            dm.set_by_name(folder, "ClassName", Value::String("Model".into()))
                .is_err()
        );
    }

    #[test]
    fn destroy_removes_the_subtree() {
        let mut dm = game();
        let ws = workspace(&dm);
        let folder = new_in(&mut dm, "Folder", ws, "A");
        let inner = new_in(&mut dm, "Folder", folder, "B");
        assert_eq!(call(&mut dm, folder, "Destroy", vec![]), Ok(ScriptValue::Nil));
        assert!(!dm.contains(folder) && !dm.contains(inner));
        assert!(dm.children(ws).unwrap().is_empty());
    }

    #[test]
    fn clone_copies_and_respects_archivable() {
        let mut dm = game();
        let ws = workspace(&dm);
        let folder = new_in(&mut dm, "Folder", ws, "A");
        new_in(&mut dm, "Folder", folder, "B");
        let temp = new_in(&mut dm, "Folder", folder, "Temp");
        dm.set_by_name(temp, "Archivable", Value::Bool(false)).unwrap();

        let ScriptValue::Instance(copy) = call(&mut dm, folder, "Clone", vec![]).unwrap() else {
            panic!("Clone returned no instance");
        };
        assert_ne!(copy, folder);
        assert_eq!(dm.parent(copy), Ok(None));
        assert_eq!(dm.name(copy), Ok("A"));
        let children = dm.children(copy).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(dm.name(children[0]), Ok("B"));

        assert_eq!(call(&mut dm, temp, "Clone", vec![]), Ok(ScriptValue::Nil));
    }

    #[test]
    fn get_children_and_descendants_list_in_order() {
        let mut dm = game();
        let ws = workspace(&dm);
        let a = new_in(&mut dm, "Folder", ws, "A");
        let a1 = new_in(&mut dm, "Folder", a, "A1");
        let b = new_in(&mut dm, "Model", ws, "B");
        assert_eq!(
            call(&mut dm, ws, "GetChildren", vec![]),
            Ok(ScriptValue::List(vec![instance(a), instance(b)]))
        );
        assert_eq!(
            call(&mut dm, ws, "GetDescendants", vec![]),
            Ok(ScriptValue::List(vec![instance(a), instance(a1), instance(b)]))
        );
        assert_eq!(
            call(&mut dm, a1, "GetChildren", vec![]),
            Ok(ScriptValue::List(vec![]))
        );
    }

    #[test]
    fn find_first_child_searches_children_or_descendants() {
        let mut dm = game();
        let ws = workspace(&dm);
        let a = new_in(&mut dm, "Folder", ws, "A");
        let deep = new_in(&mut dm, "Folder", a, "Deep");
        assert_eq!(call(&mut dm, ws, "FindFirstChild", vec![s("A")]), Ok(instance(a)));
        assert_eq!(
            call(&mut dm, ws, "FindFirstChild", vec![s("Deep")]),
            Ok(ScriptValue::Nil)
        );
        assert_eq!(
            call(&mut dm, ws, "FindFirstChild", vec![s("Deep"), true.into()]),
            Ok(instance(deep))
        );
        assert!(call(&mut dm, ws, "FindFirstChild", vec![]).is_err());
    }

    #[test]
    fn class_searches_tell_exact_class_from_inheritance() {
        let mut dm = game();
        let ws = workspace(&dm);
        let folder = new_in(&mut dm, "Folder", ws, "F");
        let model = new_in(&mut dm, "Model", ws, "M");
        let inner = new_in(&mut dm, "Model", folder, "Inner");

        assert_eq!(
            call(&mut dm, ws, "FindFirstChildOfClass", vec![s("Model")]),
            Ok(instance(model))
        );
        assert_eq!(
            call(&mut dm, ws, "FindFirstChildOfClass", vec![s("WorldObject")]),
            Ok(ScriptValue::Nil)
        );
        assert_eq!(
            call(&mut dm, ws, "FindFirstChildWhichIsA", vec![s("WorldObject")]),
            Ok(instance(model))
        );
        assert_eq!(
            call(&mut dm, ws, "FindFirstChildWhichIsA", vec![s("Instance")]),
            Ok(instance(folder))
        );
        assert_eq!(
            call(&mut dm, folder, "FindFirstChildWhichIsA", vec![s("Folder")]),
            Ok(ScriptValue::Nil)
        );
        assert_eq!(
            call(&mut dm, ws, "FindFirstChildWhichIsA", vec![s("Model")]),
            Ok(instance(model))
        );
        // Depth-first: the model inside the folder comes before the folder's next sibling.
        assert_eq!(
            call(&mut dm, ws, "FindFirstChildWhichIsA", vec![s("Model"), true.into()]),
            Ok(instance(inner))
        );

        let unknown = call(&mut dm, ws, "FindFirstChildOfClass", vec![s("Nope")]).unwrap_err();
        assert_eq!(unknown.to_string(), "unknown class 'Nope'");
        let unknown = call(&mut dm, ws, "FindFirstChildWhichIsA", vec![s("Nope")]).unwrap_err();
        assert_eq!(unknown.to_string(), "unknown class 'Nope'");
    }

    #[test]
    fn find_first_ancestor_walks_up() {
        let mut dm = game();
        let ws = workspace(&dm);
        let map = new_in(&mut dm, "Folder", ws, "Map");
        let door = new_in(&mut dm, "Folder", map, "Door");
        let knob = new_in(&mut dm, "Folder", door, "Knob");
        assert_eq!(call(&mut dm, knob, "FindFirstAncestor", vec![s("Map")]), Ok(instance(map)));
        assert_eq!(
            call(&mut dm, knob, "FindFirstAncestor", vec![s("Workspace")]),
            Ok(instance(ws))
        );
        assert_eq!(
            call(&mut dm, knob, "FindFirstAncestor", vec![s("Knob")]),
            Ok(ScriptValue::Nil)
        );
    }

    #[test]
    fn descendant_and_ancestor_checks() {
        let mut dm = game();
        let ws = workspace(&dm);
        let a = new_in(&mut dm, "Folder", ws, "A");
        let b = new_in(&mut dm, "Folder", a, "B");
        assert_eq!(call(&mut dm, b, "IsDescendantOf", vec![instance(ws)]), Ok(true.into()));
        assert_eq!(call(&mut dm, ws, "IsDescendantOf", vec![instance(b)]), Ok(false.into()));
        assert_eq!(call(&mut dm, b, "IsDescendantOf", vec![instance(b)]), Ok(false.into()));
        assert_eq!(call(&mut dm, a, "IsAncestorOf", vec![instance(b)]), Ok(true.into()));
        assert_eq!(call(&mut dm, b, "IsAncestorOf", vec![instance(a)]), Ok(false.into()));
        assert!(call(&mut dm, b, "IsAncestorOf", vec![s("A")]).is_err());
    }

    #[test]
    fn get_full_name_joins_names_from_the_service() {
        let mut dm = game();
        let ws = workspace(&dm);
        let map = new_in(&mut dm, "Folder", ws, "Map");
        let door = new_in(&mut dm, "Model", map, "Door");
        assert_eq!(call(&mut dm, door, "GetFullName", vec![]), Ok(s("Workspace.Map.Door")));
    }

    #[test]
    fn clear_all_children_keeps_the_instance() {
        let mut dm = game();
        let ws = workspace(&dm);
        let map = new_in(&mut dm, "Folder", ws, "Map");
        let a = new_in(&mut dm, "Folder", map, "A");
        let b = new_in(&mut dm, "Folder", a, "B");
        assert_eq!(call(&mut dm, map, "ClearAllChildren", vec![]), Ok(ScriptValue::Nil));
        assert!(dm.contains(map));
        assert!(!dm.contains(a) && !dm.contains(b));
        assert!(dm.children(map).unwrap().is_empty());
    }

    #[test]
    fn attributes_set_get_list_and_remove() {
        let mut dm = game();
        let ws = workspace(&dm);
        let f = new_in(&mut dm, "Folder", ws, "F");
        assert_eq!(call(&mut dm, f, "GetAttribute", vec![s("Health")]), Ok(ScriptValue::Nil));

        call(&mut dm, f, "SetAttribute", vec![s("Health"), 100.0.into()]).unwrap();
        call(&mut dm, f, "SetAttribute", vec![s("Tint"), Color3::new(1.0, 0.0, 0.0).into()]).unwrap();
        call(&mut dm, f, "SetAttribute", vec![s("Spawn"), Vector3::new(0.0, 5.0, 0.0).into()]).unwrap();
        assert_eq!(call(&mut dm, f, "GetAttribute", vec![s("Health")]), Ok(100.0.into()));
        assert_eq!(
            call(&mut dm, f, "GetAttributes", vec![]),
            Ok(ScriptValue::Map(vec![
                ("Health".into(), 100.0.into()),
                ("Tint".into(), Color3::new(1.0, 0.0, 0.0).into()),
                ("Spawn".into(), Vector3::new(0.0, 5.0, 0.0).into()),
            ]))
        );

        call(&mut dm, f, "SetAttribute", vec![s("Health"), ScriptValue::Nil]).unwrap();
        call(&mut dm, f, "SetAttribute", vec![s("Tint")]).unwrap();
        assert_eq!(call(&mut dm, f, "GetAttribute", vec![s("Health")]), Ok(ScriptValue::Nil));
        assert_eq!(
            call(&mut dm, f, "GetAttributes", vec![]),
            Ok(ScriptValue::Map(vec![("Spawn".into(), Vector3::new(0.0, 5.0, 0.0).into())]))
        );

        assert!(call(&mut dm, f, "SetAttribute", vec![s("Owner"), instance(ws)]).is_err());
        let table = call(&mut dm, f, "SetAttribute", vec![s("List"), ScriptValue::List(vec![])]).unwrap_err();
        assert!(table.to_string().starts_with("bad argument #2"), "{table}");
    }
}
