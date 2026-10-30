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
    /// The stored value as bytes (what [`set_secret`](Self::set_secret) wrote), or `None`.
    /// The default reads the text form.
    fn get_secret(&self) -> Result<Option<Vec<u8>>, AuthError> {
        Ok(self.get()?.map(String::into_bytes))
    }
    /// Stores `value` as UTF-8 bytes, which take half the room of the text form on Windows.
    /// More than [`MAX_SECRET_BYTES`] is [`AuthError::TooLarge`]. The default stores text.
    fn set_secret(&self, value: &[u8]) -> Result<(), AuthError> {
        if value.len() > MAX_SECRET_BYTES {
            return Err(AuthError::TooLarge);
        }
        let text = std::str::from_utf8(value).map_err(|e| AuthError::Store(e.to_string()))?;
        self.set(text)
    }
}

/// The most bytes the Windows Credential Manager keeps in one entry (`CRED_MAX_CREDENTIAL_BLOB_SIZE`).
/// The text form (`set`) is stored as UTF-16, so it takes twice the room.
pub const MAX_SECRET_BYTES: usize = 2560;

/// In-memory [`SecretStore`] for tests. It has the same size limit as the Windows Credential
/// Manager: [`MAX_SECRET_BYTES`], counted as UTF-16 for `set` and as bytes for `set_secret`.
#[derive(Default)]
pub struct MemorySecretStore {
    value: Mutex<Option<Vec<u8>>>,
}

impl MemorySecretStore {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<Vec<u8>>>, AuthError> {
        self.value
            .lock()
            .map_err(|_| AuthError::Store("lock was poisoned".to_string()))
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self) -> Result<Option<String>, AuthError> {
        Ok(self
            .lock()?
            .clone()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn set(&self, value: &str) -> Result<(), AuthError> {
        if value.encode_utf16().count() * 2 > MAX_SECRET_BYTES {
            return Err(AuthError::TooLarge);
        }
        *self.lock()? = Some(value.as_bytes().to_vec());
        Ok(())
    }

    fn delete(&self) -> Result<(), AuthError> {
        *self.lock()? = None;
        Ok(())
    }

    fn get_secret(&self) -> Result<Option<Vec<u8>>, AuthError> {
        Ok(self.lock()?.clone())
    }

    fn set_secret(&self, value: &[u8]) -> Result<(), AuthError> {
        if value.len() > MAX_SECRET_BYTES {
            return Err(AuthError::TooLarge);
        }
        *self.lock()? = Some(value.to_vec());
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

    fn get_secret(&self) -> Result<Option<Vec<u8>>, AuthError> {
        match self.entry()?.get_secret() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Store(e.to_string())),
        }
    }

    fn set_secret(&self, value: &[u8]) -> Result<(), AuthError> {
        if value.len() > MAX_SECRET_BYTES {
            return Err(AuthError::TooLarge);
        }
        self.entry()?
            .set_secret(value)
            .map_err(|e| AuthError::Store(e.to_string()))
    }

    fn delete(&self) -> Result<(), AuthError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::Store(e.to_string())),
        }
    }
}
