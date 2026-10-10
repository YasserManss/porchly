use porchly_core::Result;

use serde_json::{Value, json};

use super::{Command, always, to_json};
use crate::Ctx;
use crate::params::Params;

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "project.info",
            label: "Project info",
            menu: "File/Project info",
            shortcut: None,
            params: "none",
            enabled: always,
            run: info,
        },
        Command {
            id: "snapshot.create",
            label: "Save a snapshot",
            menu: "File/Save a snapshot",
            shortcut: None,
            params: "reason?: text — becomes part of the file name",
            enabled: always,
            run: snapshot_create,
        },
        Command {
            id: "snapshot.list",
            label: "Snapshots",
            menu: "File/Snapshots…",
            shortcut: None,
            params: "none",
            enabled: always,
            run: snapshot_list,
        },
    ]
}

fn info(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    cx.store().info()
}

fn snapshot_create(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let reason = p.opt_str("reason")?.unwrap_or("manual");
    let path = cx.store().snapshot(reason)?;
    Ok(json!({ "path": path.map(|p| p.display().to_string()) }))
}

fn snapshot_list(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    to_json(cx.store().list_snapshots()?)
}
