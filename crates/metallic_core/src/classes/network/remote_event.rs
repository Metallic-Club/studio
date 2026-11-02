//! RemoteEvent: a reliable message channel.
use crate::classes::prelude::*;

pub static CLASS: ClassDesc = ClassDesc::new("RemoteEvent", Some("BaseRemoteEvent"))
    .flags(ClassFlags::CREATABLE)
    .category("Network")
    .insert_order(310)
    .describe("Sends messages between the server and clients, reliably and in order.");
