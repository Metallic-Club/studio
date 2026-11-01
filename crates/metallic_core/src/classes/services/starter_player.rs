//! StarterPlayer: what each player starts with.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("StarterPlayer", Some("Instance"))
    .flags(ClassFlags::SERVICE)
    .category("Services")
    .describe("Holds what each player starts with.");
