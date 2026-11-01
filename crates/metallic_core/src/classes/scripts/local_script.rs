//! LocalScript: runs on a client.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("LocalScript", Some("Script"))
    .flags(ClassFlags::CREATABLE)
    .category("Scripts")
    .describe("A script that runs on a client.");
