//! Checkpoint: a respawn point along a course.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Checkpoint", Some("Part"))
    .flags(ClassFlags::CREATABLE)
    .category("Parts")
    .insert_order(30)
    .describe("A part that becomes a player's respawn point when touched.")
    .props(&[PropDesc::new("Order", ValueType::Number, || {
        Value::Number(0.0)
    })
    .editor(Editor::new().drag(NumberUnit::Integer))
    .describe("This checkpoint's place along the course: a lower number never replaces a higher one.")]);
