//! API keys. The full key is shown once, when it is created; only its SHA-256 hash and a
//! short display prefix are stored.

use porchly_core::{Error, Result, limits};
use rusqlite::OptionalExtension;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{DbResult, NOW, Store};

const KEY_PREFIX: &str = "pk_";
/// `pk_` + 8 characters, enough to recognise a key in a list.
const DISPLAY_PREFIX_LEN: usize = 11;
/// Anything longer can't be a Porchly key; reject before hashing.
const MAX_PRESENTED_LEN: usize = 128;

/// A key as shown in lists. Never contains the secret.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyInfo {
    pub id: i64,
    pub name: String,
    /// The first characters of the key, for recognising it (`pk_ab12cd34`).
    pub prefix: String,
    pub can_write: bool,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

impl Store {
    /// Creates a key and returns it with its secret. The secret can't be recovered later.
    pub fn create_key(&mut self, name: &str, can_write: bool) -> Result<(KeyInfo, String)> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::invalid(
                "Give the key a name, so you know later who it's for (for example \"my website\").",
            ));
        }
        if name.chars().count() > limits::MAX_KEY_NAME_LEN || name.chars().any(char::is_control) {
            return Err(Error::invalid(format!(
                "Key names can have at most {} characters and no line breaks.",
                limits::MAX_KEY_NAME_LEN
            )));
        }
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| Error::Internal(format!("no system randomness: {e}")))?;
        let secret = format!("{KEY_PREFIX}{}", data_encoding::BASE32_NOPAD.encode(&bytes).to_ascii_lowercase());
        let prefix: String = secret.chars().take(DISPLAY_PREFIX_LEN).collect();
        self.conn
            .execute(
                &format!("INSERT INTO _porchly_keys (name, prefix, hash, can_write, created_at) VALUES (?1, ?2, ?3, ?4, {NOW})"),
                rusqlite::params![name, prefix, hash(&secret), can_write],
            )
            .db()?;
        let id = self.conn.last_insert_rowid();
        Ok((self.key(id)?, secret))
    }

    pub fn list_keys(&self) -> Result<Vec<KeyInfo>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, prefix, can_write, created_at, revoked_at FROM _porchly_keys ORDER BY id")
            .db()?;
        stmt.query_map([], read_key).db()?.collect::<rusqlite::Result<_>>().db()
    }

    /// Revokes a key. Revoking an already revoked key does nothing.
    pub fn revoke_key(&mut self, id: i64) -> Result<KeyInfo> {
        self.key(id)?;
        self.conn
            .execute(&format!("UPDATE _porchly_keys SET revoked_at = {NOW} WHERE id = ?1 AND revoked_at IS NULL"), [id])
            .db()?;
        self.key(id)
    }

    /// Looks up a presented key. Returns `None` for unknown, malformed or revoked keys.
    pub fn verify_key(&self, presented: &str) -> Result<Option<KeyInfo>> {
        if presented.len() > MAX_PRESENTED_LEN || !presented.starts_with(KEY_PREFIX) {
            return Ok(None);
        }
        self.conn
            .query_row(
                "SELECT id, name, prefix, can_write, created_at, revoked_at FROM _porchly_keys WHERE hash = ?1 AND revoked_at IS NULL",
                [hash(presented)],
                read_key,
            )
            .optional()
            .db()
    }

    pub fn count_active_keys(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM _porchly_keys WHERE revoked_at IS NULL", [], |r| r.get(0))
            .db()?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    fn key(&self, id: i64) -> Result<KeyInfo> {
        self.conn
            .query_row(
                "SELECT id, name, prefix, can_write, created_at, revoked_at FROM _porchly_keys WHERE id = ?1",
                [id],
                read_key,
            )
            .optional()
            .db()?
            .ok_or_else(|| Error::not_found(format!("There's no key with id {id}.")))
    }
}

fn read_key(r: &rusqlite::Row<'_>) -> rusqlite::Result<KeyInfo> {
    Ok(KeyInfo {
        id: r.get(0)?,
        name: r.get(1)?,
        prefix: r.get(2)?,
        can_write: r.get(3)?,
        created_at: r.get(4)?,
        revoked_at: r.get(5)?,
    })
}

fn hash(secret: &str) -> String {
    data_encoding::HEXLOWER.encode(Sha256::digest(secret.as_bytes()).as_slice())
}

#[cfg(test)]
mod tests {
    use crate::test_util::temp_store;

    #[test]
    fn create_verify_revoke() {
        let (_dir, mut s) = temp_store();
        let (info, secret) = s.create_key("website", false).unwrap();
        assert!(secret.starts_with("pk_") && secret.len() == 3 + 52);
        assert!(secret.starts_with(&info.prefix));
        assert!(!info.can_write);
        assert_eq!(s.verify_key(&secret).unwrap().map(|k| k.id), Some(info.id));
        assert_eq!(s.count_active_keys().unwrap(), 1);

        s.revoke_key(info.id).unwrap();
        assert_eq!(s.verify_key(&secret).unwrap(), None);
        assert_eq!(s.count_active_keys().unwrap(), 0);
        assert!(s.revoke_key(info.id).is_ok());
        assert!(s.revoke_key(999).is_err());
    }

    #[test]
    fn secret_is_not_stored() {
        let (_dir, mut s) = temp_store();
        let (_, secret) = s.create_key("k", true).unwrap();
        let stored: String =
            s.conn.query_row("SELECT hash || prefix || name FROM _porchly_keys", [], |r| r.get(0)).unwrap();
        assert!(!stored.contains(&secret));
    }

    #[test]
    fn rejects_bad_keys_and_names() {
        let (_dir, mut s) = temp_store();
        assert_eq!(s.verify_key("").unwrap(), None);
        assert_eq!(s.verify_key("pk_nope").unwrap(), None);
        assert_eq!(s.verify_key(&"pk_x".repeat(1000)).unwrap(), None);
        assert!(s.create_key("  ", false).is_err());
        assert!(s.create_key("a\nb", false).is_err());
        assert!(s.create_key(&"x".repeat(101), false).is_err());
    }

    #[test]
    fn keys_are_unique() {
        let (_dir, mut s) = temp_store();
        let (_, a) = s.create_key("a", false).unwrap();
        let (_, b) = s.create_key("b", false).unwrap();
        assert_ne!(a, b);
    }
}
