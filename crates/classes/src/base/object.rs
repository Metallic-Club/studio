//! `Object`: the root of the class hierarchy. Everything has a `ClassName`, answers `IsA` and fires `Changed`.

use std::sync::Arc;

use metallic_core::{
    ClassDesc, DataModel, Id, MethodCall, MethodDesc, Param, PropFlags, PropMeta, ScriptError, ScriptValue, SignalDesc,
    Ty, Value,
};

use crate::script_args::class_arg;

fn class_name_get(dm: &DataModel, id: Id) -> Value {
    Value::String(Arc::from(dm.class_name(id).unwrap_or_default()))
}

fn is_a(call: &mut MethodCall<'_>) -> Result<ScriptValue, ScriptError> {
    let class = class_arg(call, 0)?;
    Ok(call.dm.is_a(call.this, class)?.into())
}

/// The root of the class hierarchy: what every engine object has in common.
pub static CLASS: ClassDesc = ClassDesc::new("Object", None)
    .describe("The base of every class. Every object knows its class and reports when a property changes.")
    .props(&[PropMeta::string("ClassName", "Object")
        .computed(class_name_get, None)
        .read_only()
        .flags(PropFlags::HIDDEN.union(PropFlags::NO_SAVE))
        .desc("The name of the object's class, such as \"Folder\". It cannot be changed.")])
    .methods(&[MethodDesc::new("IsA", is_a)
        .params(&[Param::new("className", Ty::String)])
        .returns(Ty::Bool)
        .describe(
            "Returns true if the object's class is the given class or inherits from it, such as \
             part:IsA(\"WorldObject\").",
        )])
    .signals(&[SignalDesc::new("Changed")
        .params(&[Param::new("propertyName", Ty::String)])
        .describe("Fires after one of the object's properties changes, with the property's name.")]);
