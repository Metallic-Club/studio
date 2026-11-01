//! Folder: a plain container.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Folder", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Base")
    .describe("A plain container for organizing instances.");
