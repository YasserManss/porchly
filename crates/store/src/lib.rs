//! A Porchly project: one SQLite file holding the user's tables plus Porchly's own
//! metadata (`_porchly_*` tables). All SQL in Porchly lives in this crate.
//!
//! Values are always bound as parameters. Identifiers can't be bound, so they are only ever
//! spliced in as [`Ident::quoted`], which is safe because [`Ident`] is validated.

mod convert;
mod filter;
mod keys;
mod migrate;
mod rows;
mod snapshot;
mod tables;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use porchly_core::{Error, Result};
use rusqlite::{Connection, OptionalExtension};

pub use filter::{ColumnRef, Filter, Op, Related};
pub use keys::KeyInfo;
pub use migrate::FORMAT_VERSION;
pub use rows::{ListQuery, Page, Row, parse_expand};
pub use snapshot::SnapshotInfo;
pub use tables::FieldChange;

/// SQL expression for "now" as an RFC 3339 UTC timestamp with milliseconds.
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

/// An open project.
pub struct Store {
    conn: Connection,
    path: Option<PathBuf>,
}

/// A store shared between the engine and the HTTP server.
pub type Shared = Arc<Mutex<Store>>;

/// Locks a shared store. A panic while holding the lock can't leave a half-written
/// transaction behind (SQLite rolls it back), so a poisoned lock is safe to recover.
pub fn lock(shared: &Shared) -> MutexGuard<'_, Store> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Store {
    /// Opens the project at `path`, creating it if it doesn't exist.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn =
            Connection::open(path).map_err(|e| Error::invalid(format!("Couldn't open \"{}\": {e}", path.display())))?;
        Self::init(conn, Some(path.to_path_buf()))
    }

    /// Opens an existing project; fails if there is no file at `path`.
    pub fn open_existing(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(Error::not_found(format!("There's no project at \"{}\".", path.display())));
        }
        Self::open(path)
    }

    /// A temporary project in memory (tests and previews). Snapshots are skipped.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().db()?;
        Self::init(conn, None)
    }

    fn init(mut conn: Connection, path: Option<PathBuf>) -> Result<Self> {
        // Touch the file first so "not a database" becomes a friendly error.
        let probe: rusqlite::Result<i64> = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0));
        if probe.is_err() {
            return Err(Error::invalid(
                "This file isn't a Porchly project (or it is damaged). Choose another file or a new file name.",
            ));
        }
        if path.is_some() {
            conn.pragma_update(None, "journal_mode", "WAL").db()?;
        }
        conn.pragma_update(None, "foreign_keys", "ON").db()?;
        conn.pragma_update(None, "busy_timeout", 5000).db()?;
        migrate::run(&mut conn)?;
        Ok(Store { conn, path })
    }

    /// Where the project lives on disk (`None` for in-memory projects).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn into_shared(self) -> Shared {
        Arc::new(Mutex::new(self))
    }

    /// A project setting (stored as `setting.<name>` in `_porchly_meta`), if set.
    pub fn setting(&self, name: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM _porchly_meta WHERE key = ?1", [format!("setting.{name}")], |r| r.get(0))
            .optional()
            .db()
    }

    /// Sets a project setting, or removes it when `value` is `None`.
    pub fn set_setting(&mut self, name: &str, value: Option<&str>) -> Result<()> {
        let key = format!("setting.{name}");
        match value {
            Some(v) => self
                .conn
                .execute("INSERT OR REPLACE INTO _porchly_meta (key, value) VALUES (?1, ?2)", [key.as_str(), v]),
            None => self.conn.execute("DELETE FROM _porchly_meta WHERE key = ?1", [key]),
        }
        .db()?;
        Ok(())
    }

    /// Project facts for display: format version, table count, active key count.
    pub fn info(&self) -> Result<serde_json::Value> {
        let tables: i64 = self.conn.query_row("SELECT count(*) FROM _porchly_tables", [], |r| r.get(0)).db()?;
        let keys = self.count_active_keys()?;
        let created: Option<String> =
            self.conn.query_row("SELECT value FROM _porchly_meta WHERE key = 'created_at'", [], |r| r.get(0)).ok();
        Ok(serde_json::json!({
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "format_version": FORMAT_VERSION,
            "created_at": created,
            "tables": tables,
            "active_keys": keys,
        }))
    }
}

/// Converts SQLite errors into Porchly storage errors.
trait DbResult<T> {
    fn db(self) -> Result<T>;
}

impl<T> DbResult<T> for rusqlite::Result<T> {
    fn db(self) -> Result<T> {
        self.map_err(|e| Error::Storage(e.to_string()))
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    pub fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("project.db")).unwrap();
        (dir, store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        {
            let s = Store::open(&path).unwrap();
            assert_eq!(s.info().unwrap()["tables"], 0);
        }
        assert!(Store::open_existing(&path).is_ok());
        assert!(Store::open_existing(dir.path().join("missing.db")).is_err());
    }

    #[test]
    fn settings() {
        let (_dir, mut s) = test_util::temp_store();
        assert_eq!(s.setting("functions").unwrap(), None);
        s.set_setting("functions", Some("on")).unwrap();
        assert_eq!(s.setting("functions").unwrap().as_deref(), Some("on"));
        s.set_setting("format_version", Some("x")).unwrap();
        let version: String =
            s.conn.query_row("SELECT value FROM _porchly_meta WHERE key = 'format_version'", [], |r| r.get(0)).unwrap();
        assert_eq!(version, FORMAT_VERSION.to_string(), "settings can't touch Porchly's own keys");
        s.set_setting("functions", None).unwrap();
        assert_eq!(s.setting("functions").unwrap(), None);
    }

    #[test]
    fn refuses_non_sqlite_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "hello, this is not a database at all, just some text").unwrap();
        assert!(matches!(Store::open(&path), Err(Error::Invalid(_))));
    }

    #[test]
    fn refuses_foreign_sqlite_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE stuff (x INTEGER)").unwrap();
        }
        assert!(matches!(Store::open(&path), Err(Error::Invalid(_))));
    }

    #[test]
    fn refuses_newer_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        drop(Store::open(&path).unwrap());
        {
            let c = Connection::open(&path).unwrap();
            c.execute("UPDATE _porchly_meta SET value = '999' WHERE key = 'format_version'", []).unwrap();
        }
        assert!(matches!(Store::open(&path), Err(Error::NewerFormat { found: 999, .. })));
    }
}
