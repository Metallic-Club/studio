//! Part: the ordinary building block.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Part", Some("BasePart"))
    .flags(ClassFlags::CREATABLE)
    .category("Parts")
    .insert_order(10)
    .describe("The ordinary building block.");
