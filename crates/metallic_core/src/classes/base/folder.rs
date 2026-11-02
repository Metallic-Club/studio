//! Folder: a plain container.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Folder", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Base")
    .insert_order(210)
    .describe("A plain container for organizing instances.");
