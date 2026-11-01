//! Workspace: the 3D world.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Workspace", Some("Instance"))
    .flags(ClassFlags::SERVICE)
    .category("Services")
    .describe("The 3D world: what is under it is simulated and drawn.")
    .props(&[
        PropDesc::new("FallenPartsDestroyHeight", ValueType::Number, || {
            Value::Number(-500.0)
        })
        .describe("A character that falls below this height dies, in studs."),
    ]);
