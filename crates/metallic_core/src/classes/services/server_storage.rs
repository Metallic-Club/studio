//! ServerStorage: private server storage.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ServerStorage", Some("Instance"))
    .flags(ClassFlags::SERVICE.with(ClassFlags::NOT_REPLICATED))
    .category("Services")
    .describe("Instances only the server can see.");
