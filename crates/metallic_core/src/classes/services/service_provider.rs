//! ServiceProvider: what can hand out services.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ServiceProvider", Some("Instance"))
    .flags(ClassFlags::ABSTRACT)
    .category("Services")
    .icon_like("Workspace")
    .describe("Something that hands out services by name.");
