//! ServerScriptService: server scripts.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ServerScriptService", Some("Instance"))
    .flags(ClassFlags::SERVICE.with(ClassFlags::NOT_REPLICATED))
    .category("Services")
    .describe("Server scripts; never sent to clients.");
