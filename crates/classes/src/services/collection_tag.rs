//! `CollectionTag` (#809): one CollectionService tag's signals, which `GetInstanceAddedSignal` and
//! `GetInstanceRemovedSignal` return. Made under the service the first time a script asks, named after the tag;
//! see `collection_service`.

use metallic_core::{ClassDesc, Param, SignalDesc, Ty};

/// One tag's signals.
pub static CLASS: ClassDesc = ClassDesc::new(super::collection_service::TAG_SIGNALS, Some("Instance"))
    .describe("The signals of one CollectionService tag, named after it.")
    .hidden_in_explorer()
    .signals(&[
        SignalDesc::new("InstanceAdded")
            .params(&[Param::new("instance", Ty::Class("Instance"))])
            .describe("Fires with an instance that gained the tag, or came into the game with it."),
        SignalDesc::new("InstanceRemoved")
            .params(&[Param::new("instance", Ty::Class("Instance"))])
            .describe("Fires with an instance that lost the tag, or left the game with it."),
    ]);
