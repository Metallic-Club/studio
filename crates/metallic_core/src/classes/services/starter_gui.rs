//! StarterGui: GUI each player starts with.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("StarterGui", Some("Instance"))
    .flags(ClassFlags::SERVICE)
    .category("Services")
    .describe("GUI copied to each player's PlayerGui.");
