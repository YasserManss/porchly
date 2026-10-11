use porchly_core::Result;
use serde_json::{Value, json};

use super::{Command, always, to_json};
use crate::Ctx;
use crate::params::{Params, scalar_text};

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "row.list",
            label: "Rows",
            menu: "",
            shortcut: None,
            params: "table: text; limit?: number (max 500); offset?: number; sort?: \"field\", \"-field\" or \"link.field\"; expand?: \"link,link\" (embed linked rows under \"expand\"); search?: text found in any text field (or a row id); where?: {field: value} where value is matched exactly, or \"op.value\" with op = ne, lt, lte, gt, gte, contains, starts, ends, in (\"in.2,4\"), or \"is.empty\" / \"is.notempty\" (null means is.empty)",
            enabled: always,
            run: list,
        },
        Command {
            id: "row.get",
            label: "Row",
            menu: "",
            shortcut: None,
            params: "table: text; id: number; expand?: \"link,link\" (embed linked rows under \"expand\")",
            enabled: always,
            run: get,
        },
        Command {
            id: "row.insert",
            label: "Add row",
            menu: "Row/Add row",
            shortcut: Some("Ctrl+Enter"),
            params: "table: text; values: {field: value}",
            enabled: always,
            run: insert,
        },
        Command {
            id: "row.update",
            label: "Change row",
            menu: "",
            shortcut: None,
            params: "table: text; id: number; values: {field: value}",
            enabled: always,
            run: update,
        },
        Command {
            id: "row.delete",
            label: "Delete row",
            menu: "Row/Delete row",
            shortcut: Some("Ctrl+Backspace"),
            params: "table: text; id: number",
            enabled: always,
            run: delete,
        },
        Command {
            id: "row.insert_many",
            label: "Add rows",
            menu: "",
            shortcut: None,
            params: "table: text; rows: [{field: value}] (at most 1000). All rows are added, or none; an error says which row is wrong. Returns {count, items}.",
            enabled: always,
            run: insert_many,
        },
        Command {
            id: "row.update_many",
            label: "Change rows",
            menu: "",
            shortcut: None,
            params: "table: text; values: {field: value}; and which rows: ids: [number] (at most 100) and/or where: {field: value} (as in row.list). Every matching row is changed. Without ids or where, nothing happens (an error). Returns {updated}.",
            enabled: always,
            run: update_many,
        },
        Command {
            id: "row.delete_many",
            label: "Delete rows",
            menu: "Row/Delete selected rows…",
            shortcut: None,
            params: "table: text; and which rows: ids: [number] (at most 100) and/or where: {field: value} (as in row.list). Every matching row is deleted; links to them are cleared. Without ids or where, nothing happens (an error). Returns {deleted}.",
            enabled: always,
            run: delete_many,
        },
    ]
}

/// Which rows a bulk command means: `ids` and/or `where`, as query pairs.
fn which_rows(p: &Params<'_>) -> Result<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    if let Some(ids) = p.opt_array("ids")? {
        let ids = ids
            .iter()
            .map(|v| v.as_i64().map(|n| n.to_string()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| porchly_core::Error::invalid("\"ids\" must be a list of whole numbers."))?;
        if ids.is_empty() {
            return Err(porchly_core::Error::invalid("\"ids\" is empty: say which rows."));
        }
        pairs.push(("id".to_owned(), format!("in.{}", ids.join(","))));
    }
    if let Some(filters) = p.opt_object("where")? {
        for (k, v) in filters {
            let text = if v.is_null() { "is.empty".to_owned() } else { scalar_text(k, v)? };
            pairs.push((k.clone(), text));
        }
    }
    Ok(pairs)
}

fn insert_many(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let table = p.str("table")?;
    let rows = p
        .opt_array("rows")?
        .ok_or_else(|| porchly_core::Error::invalid("\"rows\" is missing."))?
        .iter()
        .enumerate()
        .map(|(i, r)| {
            r.as_object()
                .cloned()
                .ok_or_else(|| porchly_core::Error::invalid(format!("Row {}: each row must be an object.", i + 1)))
        })
        .collect::<Result<Vec<_>>>()?;
    let items = cx.store().insert_rows_returning(table, &rows)?;
    Ok(json!({ "count": items.len(), "items": to_json(items)? }))
}

fn update_many(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let (table, values) = (p.str("table")?, p.object("values")?);
    let pairs = which_rows(&p)?;
    let store = cx.store();
    let q = store.list_query(table, pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))?;
    Ok(json!({ "updated": store.update_rows(table, &q, values)? }))
}

fn delete_many(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let table = p.str("table")?;
    let pairs = which_rows(&p)?;
    let store = cx.store();
    let q = store.list_query(table, pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))?;
    Ok(json!({ "deleted": store.delete_rows(table, &q)? }))
}

fn list(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let table = cx.store().table(p.str("table")?)?;
    let mut pairs: Vec<(String, String)> = Vec::new();
    for key in ["limit", "offset", "sort", "expand", "search"] {
        if let Some(v) = p.opt_scalar_text(key)? {
            pairs.push((key.to_owned(), v));
        }
    }
    if let Some(filters) = p.opt_object("where")? {
        for (k, v) in filters {
            // null means "has no value"; everything else uses the query-string syntax ("lt.30").
            let text = if v.is_null() { "is.empty".to_owned() } else { scalar_text(k, v)? };
            pairs.push((k.clone(), text));
        }
    }
    let store = cx.store();
    let q = store.list_query(table.name.as_str(), pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))?;
    to_json(store.list_rows(table.name.as_str(), &q)?)
}

fn get(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let (table, id) = (p.str("table")?, p.i64("id")?);
    match p.opt_str("expand")? {
        Some(expand) => to_json(cx.store().get_row_expanded(table, id, expand)?),
        None => to_json(cx.store().get_row(table, id)?),
    }
}

fn insert(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    to_json(cx.store().insert_row(p.str("table")?, p.object("values")?)?)
}

fn update(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    to_json(cx.store().update_row(p.str("table")?, p.i64("id")?, p.object("values")?)?)
}

fn delete(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let id = p.i64("id")?;
    cx.store().delete_row(p.str("table")?, id)?;
    Ok(json!({ "deleted": id }))
}
