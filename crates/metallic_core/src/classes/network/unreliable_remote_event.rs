//! UnreliableRemoteEvent: a channel that may drop messages. Runtime APIs are separate work.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("UnreliableRemoteEvent", Some("BaseRemoteEvent"))
    .flags(ClassFlags::CREATABLE)
    .category("Network")
    .no_insert()
    .describe("Sends messages between the server and clients; a message may be dropped.");
