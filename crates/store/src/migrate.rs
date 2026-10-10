//! Project file migrations. Each entry upgrades the file by one format version, inside a
//! transaction. Never edit a released migration; add a new one and bump `FORMAT_VERSION`.

use porchly_core::{Error, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::{DbResult, NOW};

/// The newest project format this build reads and writes.
pub const FORMAT_VERSION: u32 = 2;

const V1: &str = "
CREATE TABLE _porchly_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE _porchly_tables (
    name       TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);
CREATE TABLE _porchly_fields (
    table_name TEXT NOT NULL REFERENCES _porchly_tables(name) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    required   INTEGER NOT NULL DEFAULT 0,
    position   INTEGER NOT NULL,
    PRIMARY KEY (table_name, name)
);
CREATE TABLE _porchly_keys (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    prefix     TEXT NOT NULL,
    hash       TEXT NOT NULL UNIQUE,
    can_write  INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    revoked_at TEXT
);
";

/// Link fields: the table a `link` field points to.
const V2: &str = "
ALTER TABLE _porchly_fields ADD COLUMN links_to TEXT;
";

const MIGRATIONS: &[(u32, &str)] = &[(1, V1), (2, V2)];

pub(crate) fn run(conn: &mut Connection) -> Result<()> {
    let current = current_version(conn)?;
    if current > FORMAT_VERSION {
        return Err(Error::NewerFormat { found: current, supported: FORMAT_VERSION });
    }
    for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
        let tx = conn.transaction().db()?;
        tx.execute_batch(sql).db()?;
        tx.execute(
            "INSERT OR REPLACE INTO _porchly_meta (key, value) VALUES ('format_version', ?1)",
            [version.to_string()],
        )
        .db()?;
        if *version == 1 {
            tx.execute(&format!("INSERT INTO _porchly_meta (key, value) VALUES ('created_at', {NOW})"), []).db()?;
        }
        tx.commit().db()?;
    }
    Ok(())
}

fn current_version(conn: &Connection) -> Result<u32> {
    let has_meta: bool = conn
        .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '_porchly_meta'", [], |r| {
            r.get::<_, i64>(0)
        })
        .db()?
        > 0;
    if !has_meta {
        let other_tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'",
                [],
                |r| r.get(0),
            )
            .db()?;
        if other_tables > 0 {
            return Err(Error::invalid(
                "This file already holds data that Porchly didn't create, so Porchly won't change it. Choose a new file name for your project.",
            ));
        }
        return Ok(0);
    }
    let value: Option<String> = conn
        .query_row("SELECT value FROM _porchly_meta WHERE key = 'format_version'", [], |r| r.get(0))
        .optional()
        .db()?;
    value
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| Error::invalid("This project file is damaged (its format version is missing)."))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project saved by a version 1 Porchly opens and is upgraded without losing data.
    #[test]
    fn upgrades_a_version_1_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(V1).unwrap();
            c.execute_batch(
                "INSERT INTO _porchly_meta VALUES ('format_version', '1');
                 INSERT INTO _porchly_tables VALUES ('notes', '2026-01-01');
                 INSERT INTO _porchly_fields VALUES ('notes', 'text', 'text', 0, 0);
                 CREATE TABLE notes (id INTEGER PRIMARY KEY AUTOINCREMENT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, text TEXT);
                 INSERT INTO notes (created_at, updated_at, text) VALUES ('x', 'x', 'kept');",
            )
            .unwrap();
        }
        let s = crate::Store::open(&path).unwrap();
        assert_eq!(s.info().unwrap()["format_version"], FORMAT_VERSION);
        let page = s.list_rows("notes", &crate::ListQuery::default()).unwrap();
        assert_eq!(page.items[0]["text"], "kept");
        assert_eq!(s.table("notes").unwrap().fields[0].links_to, None);
    }
}
