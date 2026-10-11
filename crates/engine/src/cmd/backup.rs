//! Backups, restoring, and undo. See `porchly_store` (`backup.rs`, `undo.rs`).
//! A backup is the project file plus, next to it, a copy of the functions' code
//! (`<backup>.functions/`).

use std::path::{Path, PathBuf};

use porchly_core::Result;
use serde_json::{Value, json};

use super::{Command, always, to_json};
use crate::Ctx;
use crate::params::Params;

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "undo.list",
            label: "Recent changes",
            menu: "",
            shortcut: None,
            params: "limit?: number (default 20). Changes and deletes of rows that can be undone, newest first: [{id, at, summary, rows, undone}]. Kept for 7 days (the last 200).",
            enabled: always,
            run: undo_list,
        },
        Command {
            id: "undo.run",
            label: "Undo",
            menu: "Edit/Undo",
            shortcut: Some("Ctrl+Z"),
            params: "id?: number (default: the newest change not undone yet). Deleted rows come back, changed rows get their old values, cleared links are restored. Returns {id, summary, restored, reverted}.",
            enabled: always,
            run: undo_run,
        },
        Command {
            id: "backup.list",
            label: "Backups",
            menu: "File/Backups…",
            shortcut: None,
            params: "none. Daily backups and snapshots, newest first: [{name, kind, label, at, bytes, path}].",
            enabled: always,
            run: list,
        },
        Command {
            id: "backup.daily",
            label: "Make today's backup",
            menu: "",
            shortcut: None,
            params: "none. Makes today's backup if there isn't one yet (the last 7 days are kept). Returns {made, name}. Porchly runs this by itself.",
            enabled: always,
            run: daily,
        },
        Command {
            id: "backup.create",
            label: "Back up now",
            menu: "File/Back up now",
            shortcut: None,
            params: "none. Saves a copy of the project (and the functions' code) right now. Returns {name}.",
            enabled: always,
            run: create,
        },
        Command {
            id: "backup.restore",
            label: "Restore a backup",
            menu: "",
            shortcut: None,
            params: "name: text (from backup.list). Puts tables, rows and functions' code back as they were then. The present state is saved first (returned as saved_as), and today's keys and settings are kept.",
            enabled: always,
            run: restore,
        },
    ]
}

fn undo_list(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let limit = match p.opt_scalar_text("limit")? {
        None => 20,
        Some(t) => t.parse::<usize>().map_err(|_| porchly_core::Error::invalid("\"limit\" must be a whole number."))?,
    };
    to_json(cx.store().undo_list(limit)?)
}

fn undo_run(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let id = match p.opt_scalar_text("id")? {
        None => None,
        Some(t) => Some(t.parse::<i64>().map_err(|_| porchly_core::Error::invalid("\"id\" must be a whole number."))?),
    };
    to_json(cx.store().undo(id)?)
}

fn list(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    to_json(cx.store().restore_points()?)
}

/// Where the functions' code of the backup file `db` is kept: `<name>.functions/` next to it.
fn code_dir(db: &Path) -> PathBuf {
    db.with_extension("functions")
}

/// Copies the functions' code next to a backup, and clears out copies whose backup is gone.
fn copy_code(cx: &Ctx<'_>, db: &Path) {
    if let Ok(folder) = cx.functions().folder() {
        // A backup of the data is still a backup if the code can't be copied.
        let _ = folder.copy_code_to(&code_dir(db));
    }
    let Some(dir) = db.parent() else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.extension().is_some_and(|e| e == "functions") && !path.with_extension("db").exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn daily(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    let made = cx.store().backup_daily()?;
    cx.unlock();
    if let Some(db) = &made {
        copy_code(cx, db);
    }
    let name = made.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
    Ok(json!({ "made": made.is_some(), "name": name }))
}

fn create(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    let made = cx.store().snapshot("manual")?;
    cx.unlock();
    let Some(db) = made else {
        return Err(porchly_core::Error::invalid("Backups need a project saved on disk (this one is only in memory)."));
    };
    copy_code(cx, &db);
    Ok(json!({ "name": db.file_name().map(|n| n.to_string_lossy().into_owned()) }))
}

fn restore(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let name = p.str("name")?;
    let store = cx.store();
    let point = store.restore_points()?.into_iter().find(|pt| pt.name == name);
    let done = store.restore(name)?;
    // The state saved just before the restore gets the present code, so it can be put back too.
    let saved = done
        .saved_as
        .as_ref()
        .and_then(|saved| store.restore_points().ok()?.into_iter().find(|pt| &pt.name == saved))
        .map(|pt| PathBuf::from(pt.path));
    cx.unlock();
    if let Some(saved) = &saved {
        copy_code(cx, saved);
    }
    let mut functions = 0;
    if let (Some(point), Ok(folder)) = (point, cx.functions().folder()) {
        functions = folder.restore_code_from(&code_dir(Path::new(&point.path)))?;
    }
    // Settings were kept, but put them into effect again in case anything was reset.
    let (net, service) = (cx.net(), cx.functions().clone());
    super::apply_settings(cx.store(), net, &service);
    Ok(json!({ "restored": done.name, "saved_as": done.saved_as, "function_files": functions }))
}
