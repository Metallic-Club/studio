//! `Script`: a script that runs on the server.

use metallic_core::ClassDesc;

use super::source_container::CATEGORY;

/// A script that runs on the server.
pub static CLASS: ClassDesc = ClassDesc::new("Script", Some("BaseScript"))
    .describe(
        "A script that runs on the server, where it can change the game for every player. In a standalone game, \
         which has no separate server, it runs in the single local context.",
    )
    .category(CATEGORY)
    .creatable();
