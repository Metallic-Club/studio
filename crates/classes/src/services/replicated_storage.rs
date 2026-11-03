//! `ReplicatedStorage`: storage that both the server and players can see.

use metallic_core::ClassDesc;

/// Shared storage for the server and players.
pub static CLASS: ClassDesc = ClassDesc::new("ReplicatedStorage", Some("Instance"))
    .describe(
        "Storage for things both the server and players need, such as shared modules and assets. Nothing here is \
         drawn or run.",
    )
    .service(2)
    .persisted();
