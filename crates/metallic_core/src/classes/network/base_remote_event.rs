//! BaseRemoteEvent: what reliable and unreliable remote events share (#466 moves the members here).
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("BaseRemoteEvent", Some("Instance"))
    .flags(ClassFlags::ABSTRACT)
    .category("Network")
    .icon_like("RemoteEvent")
    .describe("A one-way message channel between the server and clients.");
