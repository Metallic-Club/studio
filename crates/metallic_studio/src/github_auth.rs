//! GitHub sign-in for Metallic Studio.
//!
//! The code lives in the shared `metallic_update` crate (the Launcher uses it too); this module
//! re-exports it. Studio keeps its session in the Credential Manager entry `MetallicStudio/github`:
//! use [`studio_secret_store`].

pub use metallic_update::github_auth::*;
pub use metallic_update::secret::*;

/// The credential store entry Studio signs in with: `MetallicStudio/github`.
pub fn studio_secret_store() -> KeyringSecretStore {
    KeyringSecretStore::new("MetallicStudio", "github")
}
