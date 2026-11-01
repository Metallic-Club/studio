//! Players: the joined players.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("Players", Some("Instance"))
    .flags(ClassFlags::SERVICE)
    .category("Services")
    .describe("Holds a Player for everyone in the game.");
