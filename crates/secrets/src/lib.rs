//! Secrets (tokens for other services) live in the operating system's keychain: Keychain on
//! macOS, Credential Manager on Windows, the Secret Service (GNOME Keyring, KWallet) on
//! Linux. Never in the project file, in files next to it, in logs or in backups.
//!
//! When a computer has no keychain (a server without a desktop, for example), secrets are
//! kept in memory only: they work until Porchly closes and must be entered again next time.
//! [`Secrets::lasting`] says which it is, so the app can tell the person.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use porchly_core::{Error, Result};

/// The name Porchly's entries carry in the keychain.
const SERVICE: &str = "Porchly";

/// A place to keep secrets by name.
pub trait Secrets: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    /// Removing a secret that isn't there is fine.
    fn delete(&self, name: &str) -> Result<()>;
    /// Whether secrets are still there after Porchly closes.
    fn lasting(&self) -> bool;
}

/// The system keychain when this computer has one that works, otherwise memory.
pub fn system() -> Arc<dyn Secrets> {
    if Keychain::works() { Arc::new(Keychain) } else { Arc::new(Memory::default()) }
}

/// The operating system's keychain.
pub struct Keychain;

impl Keychain {
    /// Whether the keychain can be reached and read (it may be missing or locked).
    fn works() -> bool {
        let probe = keyring::Entry::new(SERVICE, "porchly-probe").and_then(|e| match e.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e),
        });
        if let Err(e) = &probe {
            tracing::info!("no usable keychain on this computer ({e}); secrets are kept until Porchly closes");
        }
        probe.is_ok()
    }

    fn entry(name: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, name).map_err(|e| keychain_error(&e))
    }
}

/// Never includes the secret: keyring's errors describe the store, not the value.
fn keychain_error(e: &keyring::Error) -> Error {
    Error::invalid(format!("This computer's keychain couldn't be used: {e}"))
}

impl Secrets for Keychain {
    fn get(&self, name: &str) -> Result<Option<String>> {
        match Self::entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(&e)),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        Self::entry(name)?.set_password(value).map_err(|e| keychain_error(&e))
    }

    fn delete(&self, name: &str) -> Result<()> {
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(&e)),
        }
    }

    fn lasting(&self) -> bool {
        true
    }
}

/// Secrets kept only while Porchly runs (no keychain, previews, tests).
#[derive(Default)]
pub struct Memory {
    values: Mutex<HashMap<String, String>>,
}

impl Secrets for Memory {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.values.lock().unwrap_or_else(PoisonError::into_inner).get(name).cloned())
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner).insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<()> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner).remove(name);
        Ok(())
    }

    fn lasting(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_keeps_secrets_until_dropped() {
        let m = Memory::default();
        assert_eq!(m.get("token").unwrap(), None);
        m.set("token", "s3cret").unwrap();
        assert_eq!(m.get("token").unwrap().as_deref(), Some("s3cret"));
        m.set("token", "newer").unwrap();
        assert_eq!(m.get("token").unwrap().as_deref(), Some("newer"));
        m.delete("token").unwrap();
        m.delete("token").unwrap();
        assert_eq!(m.get("token").unwrap(), None);
        assert!(!m.lasting());
    }

    /// The real keychain, only where one is running and only when asked for
    /// (`PORCHLY_TEST_KEYCHAIN=1`), so the suite never touches anyone's keychain by surprise.
    #[test]
    fn the_system_keychain_round_trips() {
        if std::env::var_os("PORCHLY_TEST_KEYCHAIN").is_none() {
            eprintln!("skipped: set PORCHLY_TEST_KEYCHAIN=1 to test the real keychain");
            return;
        }
        assert!(Keychain::works(), "no usable keychain here");
        let k = Keychain;
        let name = "porchly-test-entry";
        k.delete(name).unwrap();
        assert_eq!(k.get(name).unwrap(), None);
        k.set(name, "a test value, safe to delete").unwrap();
        assert_eq!(k.get(name).unwrap().as_deref(), Some("a test value, safe to delete"));
        k.delete(name).unwrap();
        assert_eq!(k.get(name).unwrap(), None);
        assert!(k.lasting());
    }
}
