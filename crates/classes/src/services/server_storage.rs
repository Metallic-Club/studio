//! `ServerStorage`: storage only the server can see.

use metallic_core::ClassDesc;

/// Server-only storage.
pub static CLASS: ClassDesc = ClassDesc::new("ServerStorage", Some("Instance"))
    .describe(
        "Storage only the server can see, such as maps and items to copy into the world later. Nothing here is \
         drawn or run.",
    )
    .service(4)
    .persisted();
