//! Instance: the root of the class tree.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Instance", None)
    .flags(ClassFlags::ABSTRACT)
    .category("Base")
    .describe("The base of every class: a named node in the game tree.")
    // The default Name is the class name: resolution writes it into each class's default table.
    .props(&[PropDesc::new("Name", ValueType::String, || {
        Value::String(String::new())
    })]);
