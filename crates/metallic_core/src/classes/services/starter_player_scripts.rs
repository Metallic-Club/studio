//! StarterPlayerScripts: the built-in child of StarterPlayer.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("StarterPlayerScripts", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Services")
    .no_insert()
    .describe("LocalScripts copied to each player's PlayerScripts.");
