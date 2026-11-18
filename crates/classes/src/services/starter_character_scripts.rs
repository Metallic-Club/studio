//! `StarterCharacterScripts`: `StarterPlayer`'s built-in folder of the scripts Roblox copies into each player's
//! character when it spawns. There are no characters yet, so nothing in it runs.

use metallic_core::ClassDesc;

/// The scripts each character gets.
pub static CLASS: ClassDesc = ClassDesc::new("StarterCharacterScripts", Some("Instance")).describe(
    "Scripts here go into each player's character when it spawns. (There are no characters yet, so nothing here \
     runs.)",
);
