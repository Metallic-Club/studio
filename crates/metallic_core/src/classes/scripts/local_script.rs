//! LocalScript: runs on a client.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("LocalScript", Some("Script"))
    .flags(ClassFlags::CREATABLE)
    .category("Scripts")
    .insert_order(120)
    .describe("A script that runs on a client.");
