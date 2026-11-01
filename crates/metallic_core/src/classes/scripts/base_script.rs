//! BaseScript: source that runs by itself.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("BaseScript", Some("LuaSourceContainer"))
    .flags(ClassFlags::ABSTRACT)
    .category("Scripts")
    .describe("Source that runs by itself while enabled.")
    .props(&[PropDesc::new("Enabled", ValueType::Bool, || {
        Value::Bool(true)
    })]);
