//! RemoteFunction: a call across the network. Runtime APIs are separate work.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("RemoteFunction", Some("Instance"))
    .flags(ClassFlags::CREATABLE)
    .category("Network")
    .describe("Calls a function across the network and waits for its result.");
