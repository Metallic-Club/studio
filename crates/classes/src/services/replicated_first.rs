//! `ReplicatedFirst`: what a client gets before anything else, as Roblox's: its `LocalScript`s run on each client
//! first, before the rest of the game (for a loading screen), and nothing else in it runs.

use metallic_core::ClassDesc;

/// What reaches clients first.
pub static CLASS: ClassDesc = ClassDesc::new("ReplicatedFirst", Some("Instance"))
    .describe(
        "What each player's client gets before the rest of the game, such as a loading screen. LocalScripts here \
         run on the client first; Scripts here do not run.",
    )
    .service(2)
    .persisted()
    .runs_local_scripts();
