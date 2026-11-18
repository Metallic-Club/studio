//! `StarterPlayerScripts`: `StarterPlayer`'s built-in folder of the scripts each player's client runs. Roblox copies
//! them into each player's `PlayerScripts`; with no `Players` yet, its `LocalScript`s run where they are.

use metallic_core::ClassDesc;

/// The scripts each player's client runs.
pub static CLASS: ClassDesc = ClassDesc::new("StarterPlayerScripts", Some("Instance"))
    .describe("LocalScripts here run on each player's client when the game starts.")
    .runs_local_scripts();
