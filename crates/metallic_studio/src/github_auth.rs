//! GitHub sign-in for Metallic Studio.
//!
//! The code lives in the shared `metallic_update` crate (the Launcher uses it too); this module
//! re-exports it. Studio keeps its session in the Credential Manager under service
//! `MetallicStudio`, account `github` (Windows shows the entry as `github.MetallicStudio`): use
//! [`studio_secret_store`].

pub use metallic_update::github_auth::*;
pub use metallic_update::secret::*;

/// The credential store entry Studio signs in with: service `MetallicStudio`, account `github`
/// (`github.MetallicStudio` in Windows Credential Manager).
pub fn studio_secret_store() -> KeyringSecretStore {
    KeyringSecretStore::new("MetallicStudio", "github")
}
