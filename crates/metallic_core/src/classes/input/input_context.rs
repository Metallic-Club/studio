//! InputContext: a set of input actions (docs/design/input-actions.md section 3). A context holds
//! actions, an action holds bindings. Client-only; inert on the server.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("InputContext", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Input")
    .no_insert()
    .describe("A set of input actions that are active together.")
    .props(&[
        PropDesc::new("Enabled", ValueType::Bool, || Value::Bool(true))
            .editor(Editor::section(Section::BEHAVIOUR))
            .describe("Whether these actions are active."),
        PropDesc::new("Priority", ValueType::Number, || Value::Number(0.0))
            .editor(Editor::new().drag(NumberUnit::Integer))
            .describe("A context with a higher number is asked first."),
        PropDesc::new("Sink", ValueType::Bool, || Value::Bool(false))
            .describe("Keeps a control this context uses away from lower contexts."),
        PropDesc::new("RebindGroup", ValueType::String, || {
            Value::String("Game".into())
        })
        .describe(
            "Actions in contexts of the same group are checked against each other for conflicts.",
        ),
    ]);
