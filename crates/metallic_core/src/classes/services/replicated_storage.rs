//! ReplicatedStorage: shared storage.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("ReplicatedStorage", Some("Instance"))
    .flags(ClassFlags::SERVICE)
    .category("Services")
    .describe("Instances the server and every client can see, not simulated.");
