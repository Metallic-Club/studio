//! Where the GitHub session is kept between runs: the [`SecretStore`] trait, an in-memory store
//! for tests and the operating system's credential store.

use std::sync::Mutex;

use crate::github_auth::AuthError;

/// Where the GitHub session is kept between runs.
pub trait SecretStore: Send + Sync {
    /// The stored text, or `None` when nothing is stored.
    fn get(&self) -> Result<Option<String>, AuthError>;
    /// Stores (or replaces) the text.
    fn set(&self, value: &str) -> Result<(), AuthError>;
    /// Removes the stored text; removing nothing is fine.
    fn delete(&self) -> Result<(), AuthError>;
}

/// In-memory [`SecretStore`] for tests.
#[derive(Default)]
pub struct MemorySecretStore {
    value: Mutex<Option<String>>,
}

impl MemorySecretStore {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<String>>, AuthError> {
        self.value
            .lock()
            .map_err(|_| AuthError::Store("lock was poisoned".to_string()))
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self) -> Result<Option<String>, AuthError> {
        Ok(self.lock()?.clone())
    }

    fn set(&self, value: &str) -> Result<(), AuthError> {
        *self.lock()? = Some(value.to_string());
        Ok(())
    }

    fn delete(&self) -> Result<(), AuthError> {
        *self.lock()? = None;
        Ok(())
    }
}

/// [`SecretStore`] in the operating system's credential store (Windows Credential Manager), one
/// entry named `service/account`.
pub struct KeyringSecretStore {
    service: String,
    account: String,
}

impl KeyringSecretStore {
    /// Stores under `service` and `account` (Studio: `MetallicStudio`, `github`; the Launcher:
    /// `MetallicLauncher`, `github`).
    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.to_string(),
            account: account.to_string(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, AuthError> {
        keyring::Entry::new(&self.service, &self.account)
            .map_err(|e| AuthError::Store(e.to_string()))
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self) -> Result<Option<String>, AuthError> {
        match self.entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Store(e.to_string())),
        }
    }

    fn set(&self, value: &str) -> Result<(), AuthError> {
        self.entry()?
            .set_password(value)
            .map_err(|e| AuthError::Store(e.to_string()))
    }

    fn delete(&self) -> Result<(), AuthError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::Store(e.to_string())),
        }
    }
}
