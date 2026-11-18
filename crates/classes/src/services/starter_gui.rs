//! `StarterGui`: the user interface each player starts with, as Roblox's. There is no GUI system yet, so it holds
//! instances and its `LocalScript`s run on each client; nothing in it is drawn.

use metallic_core::ClassDesc;

/// The user interface players start with.
pub static CLASS: ClassDesc = ClassDesc::new("StarterGui", Some("Instance"))
    .describe(
        "The user interface each player starts with. LocalScripts here run on each player's client. (Nothing here \
         is drawn yet.)",
    )
    .service(6)
    .persisted()
    .runs_local_scripts();
