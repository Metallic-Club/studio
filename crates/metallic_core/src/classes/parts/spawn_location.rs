//! SpawnLocation: where characters appear.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("SpawnLocation", Some("Part"))
    .flags(ClassFlags::CREATABLE)
    .category("Parts")
    .describe("A part characters spawn on.");
