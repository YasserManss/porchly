//! Backups and restoring. Once a day Porchly copies the project into `backups/` next to it
//! (the newest [`limits::MAX_DAILY_BACKUPS`] are kept). Together with the snapshots taken
//! before destructive changes, these are the project's restore points.
//!
//! Restoring puts the tables and their rows back as they were. It first saves the present
//! state as a snapshot, so a restore can itself be reversed, and it keeps today's keys and
//! settings: otherwise a key that was turned off since would quietly work again.

use std::path::{Path, PathBuf};
use std::time::Duration;

use porchly_core::{Error, Result, limits};
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, params_from_iter};
use serde::Serialize;

use crate::{DbResult, FORMAT_VERSION, Store, migrate};

/// A copy of the project that can be restored.
#[derive(Debug, Clone, Serialize)]
pub struct RestorePoint {
    /// The file's name; pass it to [`Store::restore`].
    pub name: String,
    /// "backup" (the daily copy) or "snapshot" (before a destructive change, or made by hand).
    pub kind: &'static str,
    /// What it is, in plain words ("Daily backup", "Before deleting the table recipes").
    pub label: String,
    /// When it was made, in this computer's time (`2026-10-11 09:30`).
    pub at: String,
    pub bytes: u64,
    pub path: String,
}

/// What a restore did.
#[derive(Debug, Clone, Serialize)]
pub struct Restored {
    pub name: String,
    /// The snapshot of the state just before the restore.
    pub saved_as: Option<String>,
}

impl Store {
    /// Where daily backups live: `backups/` next to the project (`None` in memory).
    pub fn backup_dir(&self) -> Option<PathBuf> {
        Some(self.path.as_ref()?.parent()?.join("backups"))
    }

    fn stem(&self) -> Option<String> {
        Some(self.path.as_ref()?.file_stem()?.to_string_lossy().into_owned())
    }

    /// Makes today's backup if there isn't one yet, and removes the oldest beyond
    /// [`limits::MAX_DAILY_BACKUPS`]. Returns the new file, or `None` if today's exists
    /// already (or the project is in memory). Cheap to call often.
    pub fn backup_daily(&self) -> Result<Option<PathBuf>> {
        let (Some(dir), Some(stem)) = (self.backup_dir(), self.stem()) else { return Ok(None) };
        let today: String = self.conn.query_row("SELECT date('now', 'localtime')", [], |r| r.get(0)).db()?;
        let path = dir.join(format!("{stem}-{today}.db"));
        if path.exists() {
            return Ok(None);
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Storage(format!("couldn't create the backups folder: {e}")))?;
        // Written under another name first, so a half-made backup is never mistaken for one.
        let part = dir.join(format!("{stem}-{today}.db.part"));
        let _ = std::fs::remove_file(&part);
        let target = part.to_str().ok_or_else(|| Error::Storage("backup path isn't valid text".into()))?;
        self.conn.execute("VACUUM INTO ?1", [target]).db()?;
        std::fs::rename(&part, &path).map_err(|e| Error::Storage(format!("couldn't finish the backup: {e}")))?;

        let mut daily = self.daily_backups(&dir, &stem);
        daily.sort();
        let excess = daily.len().saturating_sub(limits::MAX_DAILY_BACKUPS);
        for old in daily.iter().take(excess) {
            let _ = std::fs::remove_file(old);
        }
        Ok(Some(path))
    }

    /// The daily backup files of this project (`<stem>-YYYY-MM-DD.db`).
    fn daily_backups(&self, dir: &Path, stem: &str) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.strip_prefix(stem)
                        .and_then(|r| r.strip_prefix('-'))
                        .and_then(|r| r.strip_suffix(".db"))
                        .is_some_and(is_date)
                })
            })
            .collect()
    }

    /// Daily backups and snapshots, newest first.
    pub fn restore_points(&self) -> Result<Vec<RestorePoint>> {
        let mut out = Vec::new();
        if let (Some(dir), Some(stem)) = (self.backup_dir(), self.stem()) {
            for path in self.daily_backups(&dir, &stem) {
                out.push(self.point(&path, "backup", "Daily backup".to_owned())?);
            }
        }
        for snap in self.list_snapshots()? {
            let label = snapshot_label(&snap.name);
            out.push(self.point(Path::new(&snap.path), "snapshot", label)?);
        }
        out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.name.cmp(&a.name)));
        Ok(out)
    }

    fn point(&self, path: &Path, kind: &'static str, label: String) -> Result<RestorePoint> {
        let meta = std::fs::metadata(path).map_err(|e| Error::Storage(format!("couldn't read a backup: {e}")))?;
        let secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
            .unwrap_or(0);
        let at: String = self
            .conn
            .query_row("SELECT strftime('%Y-%m-%d %H:%M', ?1, 'unixepoch', 'localtime')", [secs], |r| r.get(0))
            .db()?;
        Ok(RestorePoint {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            kind,
            label,
            at,
            bytes: meta.len(),
            path: path.display().to_string(),
        })
    }

    /// Puts the project's tables and rows back as they are in the restore point `name`
    /// (from [`Store::restore_points`]). The present state is saved as a snapshot first.
    /// Today's keys and settings are kept.
    pub fn restore(&mut self, name: &str) -> Result<Restored> {
        // Only files Porchly listed: `name` is never used as a path.
        let point = self.restore_points()?.into_iter().find(|p| p.name == name).ok_or_else(|| {
            Error::not_found(format!("There's no backup called \"{}\".", name.chars().take(80).collect::<String>()))
        })?;
        let source = Connection::open_with_flags(&point.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| Error::invalid(format!("That backup can't be opened: {e}")))?;
        let version = migrate::current_version(&source).map_err(|_| {
            Error::invalid("That backup is damaged or isn't a Porchly project, so nothing was changed.")
        })?;
        if version == 0 {
            return Err(Error::invalid("That backup is empty, so nothing was changed."));
        }
        if version > FORMAT_VERSION {
            return Err(Error::NewerFormat { found: version, supported: FORMAT_VERSION });
        }

        let saved_as = self.snapshot("before-restore")?;
        let keys = table_rows(
            &self.conn,
            "SELECT id, name, prefix, hash, can_write, created_at, revoked_at FROM _porchly_keys",
        )?;
        let settings = table_rows(&self.conn, "SELECT key, value FROM _porchly_meta WHERE key LIKE 'setting.%'")?;
        {
            let backup = rusqlite::backup::Backup::new(&source, &mut self.conn).db()?;
            backup.run_to_completion(1024, Duration::ZERO, None).db()?;
        }
        drop(source);
        self.conn.pragma_update(None, "journal_mode", "WAL").db()?;
        self.conn.pragma_update(None, "foreign_keys", "ON").db()?;
        migrate::run(&mut self.conn)?;

        let tx = self.conn.transaction().db()?;
        tx.execute("DELETE FROM _porchly_keys", []).db()?;
        for row in &keys {
            tx.execute(
                "INSERT INTO _porchly_keys (id, name, prefix, hash, can_write, created_at, revoked_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params_from_iter(row.iter()),
            )
            .db()?;
        }
        tx.execute("DELETE FROM _porchly_meta WHERE key LIKE 'setting.%'", []).db()?;
        for row in &settings {
            tx.execute("INSERT INTO _porchly_meta (key, value) VALUES (?1, ?2)", params_from_iter(row.iter())).db()?;
        }
        tx.commit().db()?;
        // Recorded changes describe the state that was just replaced.
        self.forget_all_undo()?;
        Ok(Restored {
            name: point.name,
            saved_as: saved_as.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
        })
    }
}

fn table_rows(conn: &Connection, sql: &str) -> Result<Vec<Vec<Value>>> {
    let mut stmt = conn.prepare(sql).db()?;
    let n = stmt.column_count();
    stmt.query_map([], |r| (0..n).map(|i| r.get::<_, Value>(i)).collect::<rusqlite::Result<Vec<_>>>())
        .db()?
        .collect::<rusqlite::Result<_>>()
        .db()
}

/// `2026-10-11`.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// Turns a snapshot's file name (`2026-10-11T09-30-12.345-before-drop-table-recipes.db`)
/// into plain words.
fn snapshot_label(file: &str) -> String {
    let reason = file.strip_suffix(".db").unwrap_or(file);
    // The time stamp is the first 23 characters.
    let reason = reason.get(24..).unwrap_or(reason);
    if let Some(t) = reason.strip_prefix("before-drop-table-") {
        format!("Before deleting the table {t}")
    } else if let Some((table, field)) = reason.strip_prefix("before-drop-field-").and_then(|r| r.split_once('-')) {
        // Names never contain "-", so the first one separates table and field.
        format!("Before deleting the field {field} from {table}")
    } else if let Some((table, field)) = reason.strip_prefix("before-change-field-").and_then(|r| r.split_once('-')) {
        format!("Before changing the type of {field} in {table}")
    } else if reason == "before-restore" {
        "Before restoring a backup".to_owned()
    } else if reason == "manual" {
        "Backup made by hand".to_owned()
    } else {
        format!("Snapshot ({reason})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ListQuery;
    use crate::test_util::temp_store;
    use porchly_core::{FieldKind, Ident};
    use porchly_schema::FieldDef;
    use serde_json::json;

    fn body(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().cloned().unwrap()
    }

    fn with_notes() -> (tempfile::TempDir, Store) {
        let (dir, mut s) = temp_store();
        s.create_table(
            &Ident::new("notes").unwrap(),
            vec![FieldDef::new(Ident::new("text").unwrap(), FieldKind::Text, false)],
        )
        .unwrap();
        s.insert_row("notes", &body(json!({"text": "first"}))).unwrap();
        (dir, s)
    }

    fn texts(s: &Store) -> Vec<String> {
        s.list_rows("notes", &ListQuery::default())
            .unwrap()
            .items
            .iter()
            .map(|r| r["text"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn one_backup_a_day_and_only_the_newest_are_kept() {
        let (dir, s) = with_notes();
        let made = s.backup_daily().unwrap().unwrap();
        assert!(made.starts_with(dir.path().join("backups")));
        assert!(Store::open_existing(&made).is_ok(), "a backup is a working project");
        assert!(s.backup_daily().unwrap().is_none(), "today's exists already");
        // Older days (made up here) are pruned when a new one is added.
        std::fs::remove_file(&made).unwrap();
        for day in 1..=9 {
            std::fs::write(dir.path().join(format!("backups/project-2020-01-0{day}.db")), "old").unwrap();
        }
        std::fs::write(dir.path().join("backups/other-2020-01-01.db"), "not ours").unwrap();
        std::fs::write(dir.path().join("backups/project-notes.db"), "not a daily backup").unwrap();
        s.backup_daily().unwrap().unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.iter().filter(|n| n.starts_with("project-20")).count(), limits::MAX_DAILY_BACKUPS);
        assert!(!names.contains(&"project-2020-01-01.db".to_owned()), "the oldest went");
        assert!(names.contains(&"other-2020-01-01.db".to_owned()) && names.contains(&"project-notes.db".to_owned()));
        assert!(Store::open_in_memory().unwrap().backup_daily().unwrap().is_none());
    }

    #[test]
    fn restoring_brings_the_data_back_and_can_be_reversed() {
        let (_dir, mut s) = with_notes();
        let backup = s.backup_daily().unwrap().unwrap();
        let name = backup.file_name().unwrap().to_str().unwrap().to_owned();
        s.insert_row("notes", &body(json!({"text": "second"}))).unwrap();
        s.drop_table("notes").unwrap();
        assert!(s.table("notes").is_err());

        let points = s.restore_points().unwrap();
        assert!(points.iter().any(|p| p.name == name && p.kind == "backup" && p.label == "Daily backup"));
        assert!(points.iter().any(|p| p.label == "Before deleting the table notes"));

        let done = s.restore(&name).unwrap();
        assert_eq!(texts(&s), ["first"], "as it was when the backup was made");
        s.insert_row("notes", &body(json!({"text": "works after restoring"}))).unwrap();

        // The state just before the restore was saved, so the restore can be reversed.
        let before = done.saved_as.unwrap();
        s.restore(&before).unwrap();
        assert!(s.table("notes").is_err(), "back to after the table was deleted");
        // And the snapshot from the delete itself gets the table back with both rows.
        let snap =
            s.restore_points().unwrap().into_iter().find(|p| p.label == "Before deleting the table notes").unwrap();
        s.restore(&snap.name).unwrap();
        assert_eq!(texts(&s), ["first", "second"]);
    }

    #[test]
    fn restoring_keeps_today_s_keys_and_settings() {
        let (_dir, mut s) = with_notes();
        let (old_key, old_secret) = s.create_key("old", true).unwrap();
        s.set_setting("functions", Some("on")).unwrap();
        let name = s.backup_daily().unwrap().unwrap().file_name().unwrap().to_str().unwrap().to_owned();

        // After the backup: the old key is turned off, a new one is made, a setting changes.
        s.revoke_key(old_key.id).unwrap();
        let (_, new_secret) = s.create_key("new", false).unwrap();
        s.set_setting("functions", None).unwrap();
        s.set_setting("port", Some("12000")).unwrap();
        s.update_row("notes", 1, &body(json!({"text": "changed"}))).unwrap();

        s.restore(&name).unwrap();
        assert_eq!(texts(&s), ["first"]);
        assert!(s.verify_key(&old_secret).unwrap().is_none(), "a key turned off since stays off");
        assert!(s.verify_key(&new_secret).unwrap().is_some(), "a key made since still works");
        assert_eq!(s.setting("functions").unwrap(), None);
        assert_eq!(s.setting("port").unwrap().as_deref(), Some("12000"));
        assert!(s.undo_list(10).unwrap().is_empty(), "old undo steps no longer apply");
    }

    #[test]
    fn only_listed_healthy_backups_can_be_restored() {
        let (dir, mut s) = with_notes();
        let good = s.backup_daily().unwrap().unwrap();
        for name in ["../project.db", "/etc/passwd", "nope.db", ""] {
            assert!(matches!(s.restore(name), Err(Error::NotFound(_))), "{name:?}");
        }
        // A damaged backup is refused before anything changes.
        std::fs::write(dir.path().join("backups/project-2020-01-01.db"), "this is not a database").unwrap();
        assert!(s.restore("project-2020-01-01.db").is_err());
        // So is one from a newer Porchly.
        let newer = dir.path().join("backups/project-2020-01-02.db");
        std::fs::copy(&good, &newer).unwrap();
        {
            let c = Connection::open(&newer).unwrap();
            c.execute("UPDATE _porchly_meta SET value = '999' WHERE key = 'format_version'", []).unwrap();
        }
        assert!(matches!(s.restore("project-2020-01-02.db"), Err(Error::NewerFormat { .. })));
        assert_eq!(texts(&s), ["first"], "nothing was touched");
        assert!(!s.restore_points().unwrap().iter().any(|p| p.label == "Before restoring a backup"));
    }

    #[test]
    fn snapshot_names_become_plain_words() {
        let label = |reason: &str| snapshot_label(&format!("2026-10-11T09-30-12.345-{reason}.db"));
        assert_eq!(label("before-drop-table-recipes"), "Before deleting the table recipes");
        assert_eq!(
            label("before-drop-field-guest_list-phone_number"),
            "Before deleting the field phone_number from guest_list"
        );
        assert_eq!(label("before-change-field-recipes-minutes"), "Before changing the type of minutes in recipes");
        assert_eq!(label("before-restore"), "Before restoring a backup");
        assert_eq!(label("manual"), "Backup made by hand");
        assert_eq!(label("something-else"), "Snapshot (something-else)");
        assert!(is_date("2026-10-11") && !is_date("2026-10-1") && !is_date("notes-2026"));
    }
}
