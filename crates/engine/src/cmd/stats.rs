//! Stats: how much the project is used and how big it is. Usage is counted since Porchly
//! started (see `porchly_api::Stats`); nothing is sent anywhere.

use porchly_core::Result;
use serde_json::{Value, json};

use super::{Command, always};
use crate::Ctx;
use crate::params::Params;

pub(super) fn commands() -> Vec<Command> {
    vec![Command {
        id: "stats.get",
        label: "Stats",
        menu: "Porchly/Stats…",
        shortcut: None,
        params: "none. Returns {seconds (since Porchly started), requests: {total, ok, refused, rate_limited, mistakes, failed, reads, writes, public, mcp}, per_minute: [60 numbers, the current minute last], keys: [{name, requests, last_used_seconds_ago}], tables: [{name, rows, requests}], functions: [{name, calls}], project: {tables, rows, size_bytes, keys, backups, backups_bytes, changes}}.",
        enabled: always,
        run: get,
    }]
}

fn get(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    let mut out = cx.net().stats().snapshot();
    let used = out.get("tables").and_then(Value::as_object).cloned().unwrap_or_default();
    let calls = out.get("functions").and_then(Value::as_object).cloned().unwrap_or_default();
    let functions = cx.functions().clone();

    let store = cx.store();
    let mut rows_total: u64 = 0;
    let mut tables = Vec::new();
    for t in store.list_tables()? {
        let rows = store.count_rows(&t)?;
        rows_total = rows_total.saturating_add(rows);
        let requests = used.get(t.name.as_str()).and_then(Value::as_u64).unwrap_or(0);
        tables.push(json!({ "name": t.name, "rows": rows, "requests": requests }));
    }
    // The project file and what SQLite keeps next to it.
    let size_bytes: u64 = store.path().map_or(0, |p| {
        ["", "-wal"]
            .iter()
            .filter_map(|suffix| {
                let mut name = p.as_os_str().to_owned();
                name.push(suffix);
                std::fs::metadata(name).ok().map(|m| m.len())
            })
            .sum()
    });
    let points = store.restore_points()?;
    let project = json!({
        "tables": tables.len(),
        "rows": rows_total,
        "size_bytes": size_bytes,
        "keys": store.count_active_keys()?,
        "backups": points.len(),
        "backups_bytes": points.iter().map(|p| p.bytes).sum::<u64>(),
        "changes": store.undo_list(1000)?.iter().filter(|c| !c.undone).count(),
    });
    cx.unlock();

    // Every function, with how often it ran (0 for ones not called yet).
    let mut function_list: Vec<Value> = porchly_api::Functions::list(functions.as_ref())
        .into_iter()
        .map(|f| json!({ "name": f.name, "calls": calls.get(&f.name).and_then(Value::as_u64).unwrap_or(0) }))
        .collect();
    function_list.sort_by_key(|f| std::cmp::Reverse(f.get("calls").and_then(Value::as_u64).unwrap_or(0)));
    if let Some(o) = out.as_object_mut() {
        o.insert("tables".into(), Value::Array(tables));
        o.insert("functions".into(), Value::Array(function_list));
        o.insert("project".into(), project);
    }
    Ok(out)
}
