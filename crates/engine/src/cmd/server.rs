use porchly_core::{Error, Result};
use serde_json::Value;

use super::{Command, always};
use crate::Ctx;
use crate::params::Params;

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "api.start",
            label: "Turn on the API",
            menu: "Share/Turn on the API",
            shortcut: None,
            params: "port?: number (default: the port in Settings, 11911 unless changed; if busy, the next free port is used); exact?: bool (fail instead of moving to another port). Only reachable from this computer.",
            enabled: always,
            run: start,
        },
        Command {
            id: "api.stop",
            label: "Turn off the API",
            menu: "Share/Turn off the API",
            shortcut: None,
            params: "none. Also turns off the porch light.",
            enabled: always,
            run: stop,
        },
        Command {
            id: "api.docs",
            label: "API docs",
            menu: "Share/Open API docs",
            shortcut: None,
            params: "base?: text — the API's address used in examples (default: the running server, or http://127.0.0.1:11911). Returns {html}.",
            enabled: always,
            run: docs,
        },
        Command {
            id: "api.status",
            label: "API status",
            menu: "",
            shortcut: None,
            params: "none",
            enabled: always,
            run: status,
        },
    ]
}

fn start(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let port = match p.opt_scalar_text("port")? {
        None => super::port_setting(cx.store()),
        Some(t) => {
            t.parse::<u16>().map_err(|_| Error::invalid("The port must be a whole number from 1024 to 65535."))?
        }
    };
    let exact = p.bool_or("exact", false)?;
    cx.unlock();
    let shared = cx.shared().clone();
    let functions = cx.functions().clone();
    cx.net().start(&shared, &functions, port, exact)
}

fn stop(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    cx.functions().set_api_url(None);
    // No API, no public link to it.
    cx.light().tunnel.stop();
    cx.net().stop_public();
    Ok(cx.net().stop())
}

fn docs(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let running = cx.net().status().get("url").and_then(Value::as_str).map(str::to_owned);
    let base = match p.opt_str("base")? {
        Some(b) => b.to_owned(),
        None => running.unwrap_or_else(|| format!("http://127.0.0.1:{}", porchly_api::DEFAULT_PORT)),
    };
    let functions = porchly_api::Functions::list(cx.functions().as_ref());
    let tables = cx.store().list_tables()?;
    Ok(serde_json::json!({ "html": porchly_api::docs::html(&tables, &functions, &base) }))
}

fn status(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    Ok(cx.net().status())
}
