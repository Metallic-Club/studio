//! RunService: the frame loop.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("RunService", Some("Instance"))
    .flags(ClassFlags::SERVICE)
    .category("Services")
    .describe("The frame loop: Heartbeat and whether this is the server or a client.");
