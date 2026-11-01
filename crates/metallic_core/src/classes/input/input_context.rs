//! InputContext: a set of input actions (docs/design/input-actions.md section 3). A context holds
//! actions, an action holds bindings. Client-only; inert on the server.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("InputContext", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Input")
    .describe("A set of input actions that are active together.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true)),
        PropDesc::new("Priority", ValueType::Number, || Value::Number(0.0)),
        PropDesc::new("Sink", ValueType::Bool, || Value::Bool(false)),
        PropDesc::new("RebindGroup", ValueType::String, || {
            Value::String("Game".into())
        }),
    ]);
