//! ServiceProvider: what can hand out services.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ServiceProvider", Some("Instance"))
    .flags(ClassFlags::ABSTRACT)
    .category("Services")
    .describe("Something that hands out services by name.");
