//! LuaSourceContainer: anything holding script source.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("LuaSourceContainer", Some("Instance"))
    .flags(ClassFlags::ABSTRACT)
    .category("Scripts")
    .describe("Anything that holds script source.")
    .props(&[PropDesc::new("Source", ValueType::String, || {
        Value::String(String::new())
    })]);
