use porchly_core::Result;

use serde_json::{Value, json};

use super::{Command, always, to_json};
use crate::Ctx;
use crate::params::Params;

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "api.key.create",
            label: "New key",
            menu: "Share/New key…",
            shortcut: None,
            params: "name: text — who the key is for; write?: bool (default false: the key can only read)",
            enabled: always,
            run: create,
        },
        Command {
            id: "api.key.list",
            label: "Keys",
            menu: "Share/Keys…",
            shortcut: None,
            params: "none",
            enabled: always,
            run: list,
        },
        Command {
            id: "api.key.revoke",
            label: "Turn off key",
            menu: "",
            shortcut: None,
            params: "id: number",
            enabled: always,
            run: revoke,
        },
    ]
}

fn create(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let (key, secret) = cx.store().create_key(p.str("name")?, p.bool_or("write", false)?)?;
    Ok(json!({
        "key": to_json(key)?,
        "secret": secret,
        "note": "Copy this key now. Porchly only keeps a fingerprint of it, so it can't be shown again.",
    }))
}

fn list(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    to_json(cx.store().list_keys()?)
}

fn revoke(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    to_json(cx.store().revoke_key(p.i64("id")?)?)
}
