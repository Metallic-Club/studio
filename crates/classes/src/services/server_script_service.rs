//! `ServerScriptService`: where server scripts live.

use metallic_core::ClassDesc;

/// The home of server scripts.
pub static CLASS: ClassDesc = ClassDesc::new("ServerScriptService", Some("Instance"))
    .describe("Holds the game's server scripts. Scripts placed here run on the server; players never see them.")
    .service(3)
    .persisted()
    .runs_scripts();
