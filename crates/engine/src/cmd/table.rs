use porchly_core::{Error, FieldKind, Ident, Result};
use porchly_schema::FieldDef;
use porchly_store::FieldChange;

use serde_json::{Value, json};

use super::{Command, always, to_json};
use crate::Ctx;
use crate::params::Params;

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "table.list",
            label: "Tables",
            menu: "",
            shortcut: None,
            params: "none",
            enabled: always,
            run: list,
        },
        Command {
            id: "table.create",
            label: "New table",
            menu: "Table/New table…",
            shortcut: Some("Ctrl+Shift+N"),
            params: "name: text; fields?: [{name: text, type: text|integer|number|boolean|datetime|link, required?: bool, links_to?: table (for link fields)}]",
            enabled: always,
            run: create,
        },
        Command {
            id: "table.drop",
            label: "Delete table",
            menu: "Table/Delete table…",
            shortcut: None,
            params: "name: text — a snapshot is saved first",
            enabled: always,
            run: drop,
        },
        Command {
            id: "field.add",
            label: "Add field",
            menu: "Table/Add field…",
            shortcut: None,
            params: "table: text; name: text; type: text|integer|number|boolean|datetime|link; required?: bool; links_to?: table (for link fields)",
            enabled: always,
            run: field_add,
        },
        Command {
            id: "field.update",
            label: "Change field",
            menu: "Table/Change field…",
            shortcut: None,
            params: "table: text; name: text (the field to change); new_name?: text; type?: text|integer|number|boolean|datetime|link (existing values are converted, or the change is refused naming the rows that don't fit; a snapshot is saved first); required?: bool; links_to?: table (for link fields)",
            enabled: always,
            run: field_update,
        },
        Command {
            id: "field.drop",
            label: "Delete field",
            menu: "Table/Delete field…",
            shortcut: None,
            params: "table: text; name: text — a snapshot is saved first",
            enabled: always,
            run: field_drop,
        },
    ]
}

fn list(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    to_json(cx.store().list_tables()?)
}

fn create(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let name = Ident::new(p.str("name")?)?;
    let fields = match p.opt_array("fields")? {
        Some(list) => list.iter().map(field_def).collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    to_json(cx.store().create_table(&name, fields)?)
}

fn drop(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let name = p.str("name")?;
    cx.store().drop_table(name)?;
    Ok(json!({ "dropped": name }))
}

fn field_add(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let table = p.str("table")?;
    let def = field_def(params)?;
    to_json(cx.store().add_field(table, def)?)
}

fn field_update(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let change = FieldChange {
        name: p.opt_str("new_name")?.map(Ident::field).transpose()?,
        kind: p.opt_str("type")?.map(str::parse::<FieldKind>).transpose()?,
        required: match p.opt_scalar_text("required")? {
            None => None,
            Some(_) => Some(p.bool_or("required", false)?),
        },
        links_to: p.opt_str("links_to")?.map(Ident::new).transpose()?,
    };
    to_json(cx.store().update_field(p.str("table")?, p.str("name")?, change)?)
}

fn field_drop(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    to_json(cx.store().drop_field(p.str("table")?, p.str("name")?)?)
}

/// Reads `{name, type, required?}` (also accepts `kind` for `type`).
fn field_def(v: &Value) -> Result<FieldDef> {
    let p = Params::new(v)
        .map_err(|_| Error::invalid("Each field must look like {\"name\": \"title\", \"type\": \"text\"}."))?;
    let name = Ident::field(p.str("name")?)?;
    let kind: FieldKind = match (p.opt_str("type")?, p.opt_str("kind")?) {
        (Some(k), _) | (None, Some(k)) => k.parse()?,
        (None, None) => FieldKind::Text,
    };
    let links_to = match p.opt_str("links_to")? {
        Some(t) => Some(Ident::new(t)?),
        None => None,
    };
    Ok(FieldDef { name, kind, required: p.bool_or("required", false)?, links_to })
}
