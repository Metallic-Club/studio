//! PlayerGui: child of each Player; the player's GUI and LocalScripts live here.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("PlayerGui", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Players")
    .no_insert()
    .describe("Child of each Player; holds the player's GUI and its LocalScripts.");
