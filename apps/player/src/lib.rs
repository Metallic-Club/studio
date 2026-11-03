//! Metallic Player: runs a game, standalone or on the platform.
//!
//! It is a library so that Studio (Play in Studio) and the Launcher run games through exactly the code that ships.
//! The `metallic_player` binary is a thin wrapper around it.

/// The player's version, as shown in logs and in the platform UI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
