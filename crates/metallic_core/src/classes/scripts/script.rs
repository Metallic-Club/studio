//! Script: runs on the server.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Script", Some("BaseScript"))
    .flags(ClassFlags::CREATABLE)
    .category("Scripts")
    .insert_order(110)
    .describe("A script that runs on the server.");
