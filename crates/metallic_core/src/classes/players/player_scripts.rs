//! PlayerScripts: child of each Player; only its owner sees it.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("PlayerScripts", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Players")
    .describe("Child of each Player; holds the LocalScripts only that player runs.");
