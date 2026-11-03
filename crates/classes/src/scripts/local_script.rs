//! `LocalScript`: a script that runs on a player's client.

use metallic_core::ClassDesc;

use super::source_container::CATEGORY;

/// A script that runs on clients.
pub static CLASS: ClassDesc = ClassDesc::new("LocalScript", Some("BaseScript"))
    .describe(
        "A script that runs on each player's client, for input, the camera and the user interface. What it \
         changes is seen only by that player.",
    )
    .category(CATEGORY)
    .creatable();
