//! Credential storage.
//!
//! Secrets (Volcengine AK/SK, user-supplied API keys) go to the OS keyring —
//! Windows Credential Manager on Windows, Keychain on macOS, keyutils on
//! Linux — never to settings.json, logs, or any plaintext cache file.
//!
//! Errors are *sanitized by construction*: `SecretError` carries a fixed
//! kind + a caller-controlled message that never embeds secret values.

use std::collections::HashMap;
use std::sync::Mutex;

/// Fixed error kinds — display strings are static so a secret value can
/// never leak through an error chain.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SecretErrorKind {
    /// OS keyring unavailable (no keychain service, locked session…).
    BackendUnavailable,
    NotFound,
    Io,
}

impl SecretErrorKind {
    pub fn message(self) -> &'static str {
        match self {
            SecretErrorKind::BackendUnavailable => {
                "系统凭据管理器不可用（Windows Credential Manager / 系统钥匙串）"
            }
            SecretErrorKind::NotFound => "凭据不存在",
            SecretErrorKind::Io => "凭据存储读写失败",
        }
    }
}

pub type SecretResult<T> = Result<T, SecretErrorKind>;

/// Injectable abstraction — real impl is the OS keyring, tests use memory.
pub trait SecretStorage: Send + Sync {
    fn set(&self, key: &str, value: &str) -> SecretResult<()>;
    fn get(&self, key: &str) -> SecretResult<String>;
    fn delete(&self, key: &str) -> SecretResult<()>;
    fn backend_name(&self) -> &'static str;
}

/// OS keyring via the `keyring` crate. The entry *user* field is the logical
/// key (e.g. "volcengine_secret_key"); the *service* groups our entries.
pub struct KeyringStorage {
    service: String,
}

impl KeyringStorage {
    pub fn new(service: &str) -> Self {
        Self { service: service.to_string() }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretErrorKind> {
        // Reject weird keys up front; the key itself is not secret material
        // but must be a stable, filesystem-safe identifier.
        if key.is_empty() || key.len() > 120 || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(SecretErrorKind::Io);
        }
        keyring::Entry::new(&self.service, key).map_err(|_| SecretErrorKind::BackendUnavailable)
    }
}

impl SecretStorage for KeyringStorage {
    fn set(&self, key: &str, value: &str) -> SecretResult<()> {
        // Never store an empty value — delete instead so get() reports NotFound.
        if value.is_empty() {
            return self.delete(key);
        }
        let entry = self.entry(key)?;
        entry.set_password(value).map_err(|e| match e {
            keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
                SecretErrorKind::BackendUnavailable
            }
            _ => SecretErrorKind::Io,
        })
    }

    fn get(&self, key: &str) -> SecretResult<String> {
        let entry = self.entry(key)?;
        match entry.get_password() {
            Ok(v) => Ok(v),
            Err(keyring::Error::NoEntry) => Err(SecretErrorKind::NotFound),
            Err(keyring::Error::Ambiguous(_)) => Err(SecretErrorKind::NotFound),
            Err(keyring::Error::NoStorageAccess(_)) | Err(keyring::Error::PlatformFailure(_)) => {
                Err(SecretErrorKind::BackendUnavailable)
            }
            Err(_) => Err(SecretErrorKind::Io),
        }
    }

    fn delete(&self, key: &str) -> SecretResult<()> {
        let entry = self.entry(key)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(keyring::Error::NoStorageAccess(_)) | Err(keyring::Error::PlatformFailure(_)) => {
                Err(SecretErrorKind::BackendUnavailable)
            }
            Err(_) => Err(SecretErrorKind::Io),
        }
    }

    fn backend_name(&self) -> &'static str {
        "OS keyring"
    }
}

/// In-memory storage for tests and for headless dev runs where no keyring
/// daemon exists (providers then degrade to NotConfigured, never plaintext).
pub struct MemoryStorage {
    map: Mutex<HashMap<String, String>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self { map: Mutex::new(HashMap::new()) }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStorage for MemoryStorage {
    fn set(&self, key: &str, value: &str) -> SecretResult<()> {
        if value.is_empty() {
            self.map.lock().unwrap().remove(key);
            return Ok(());
        }
        self.map.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> SecretResult<String> {
        self.map
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or(SecretErrorKind::NotFound)
    }

    fn delete(&self, key: &str) -> SecretResult<()> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "memory (test)"
    }
}

/// Known secret keys. Settings and IPC only ever see these identifiers —
/// secret *values* cross the process boundary once, on save, and then live
/// exclusively in the keyring.
pub const KEY_VOLCENGINE_AK: &str = "volcengine_access_key";
pub const KEY_VOLCENGINE_SK: &str = "volcengine_secret_key";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip_and_delete() {
        let s = MemoryStorage::new();
        assert_eq!(s.get("k"), Err(SecretErrorKind::NotFound));
        s.set("k", "v1").unwrap();
        assert_eq!(s.get("k").unwrap(), "v1");
        s.set("k", "").unwrap(); // empty → delete
        assert_eq!(s.get("k"), Err(SecretErrorKind::NotFound));
        s.delete("missing").unwrap(); // idempotent
    }

    #[test]
    fn keyring_rejects_bad_keys_without_touching_backend() {
        let s = KeyringStorage::new("test-svc");
        assert_eq!(s.set("bad key with spaces", "x"), Err(SecretErrorKind::Io));
        assert_eq!(s.set(&"k".repeat(200), "x"), Err(SecretErrorKind::Io));
    }
}
