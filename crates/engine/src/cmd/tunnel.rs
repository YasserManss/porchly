//! The porch light: a public link to the API through a Cloudflare quick tunnel.
//! Off at every start; turning it on is always the person's choice.

use std::sync::atomic::Ordering;

use porchly_core::Result;
use serde_json::Value;

use super::{Command, always};
use crate::Ctx;
use crate::params::Params;

/// The project setting that remembers "Allow changes through the link".
pub(crate) const PORCH_LIGHT_CHANGES: &str = "porch_light_changes";

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "tunnel.status",
            label: "Porch light status",
            menu: "",
            shortcut: None,
            params: "none. Returns {state: off|downloading|connecting|on|failed, url, slow, error, allow_changes, download: {done, total}}.",
            enabled: always,
            run: status,
        },
        Command {
            id: "tunnel.start",
            label: "Turn on the porch light",
            menu: "Share/Turn on the porch light…",
            shortcut: None,
            params: "allow_changes?: bool (default: as last set, else false). Gives the API a public https link (Cloudflare quick tunnel). Downloads cloudflared (about 40 MB) the first time. Returns at once; the link appears in tunnel.status within seconds.",
            enabled: always,
            run: start,
        },
        Command {
            id: "tunnel.stop",
            label: "Turn off the porch light",
            menu: "Share/Turn off the porch light",
            shortcut: None,
            params: "none. The public link stops working at once.",
            enabled: always,
            run: stop,
        },
        Command {
            id: "tunnel.allow_changes",
            label: "Allow changes through the public link",
            menu: "",
            shortcut: None,
            params: "allow: bool. When false (the default), requests through the public link can only read, even with a key that may make changes.",
            enabled: always,
            run: allow_changes,
        },
    ]
}

fn status(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    Ok(with_changes(cx, cx.light().tunnel.status()))
}

fn with_changes(cx: &Ctx<'_>, mut status: Value) -> Value {
    if let Some(o) = status.as_object_mut() {
        o.insert("allow_changes".into(), Value::Bool(cx.light().allow_changes.load(Ordering::Relaxed)));
    }
    status
}

fn start(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    if let Some(allow) = p.opt_bool("allow_changes")? {
        set_changes(cx, allow)?;
    }
    cx.unlock();
    // Downloading can take a while; never hold the project meanwhile.
    let program = cx.light().tunnel.ensure_program(cx.net().allowed())?;
    let shared = cx.shared().clone();
    let functions = cx.functions().clone();
    // The public link is a way to reach the API, so the API is on while the light is.
    if cx.net().status().get("running").and_then(Value::as_bool) != Some(true) {
        let port = super::port_setting(cx.store());
        cx.unlock();
        cx.net().start(&shared, &functions, port, false)?;
    }
    let allow = cx.light().allow_changes.clone();
    let port = cx.net().start_public(&shared, &functions, &allow)?;
    if let Err(e) = cx.light().tunnel.start(&program, port) {
        cx.net().stop_public();
        return Err(e);
    }
    Ok(with_changes(cx, cx.light().tunnel.status()))
}

fn stop(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    cx.light().tunnel.stop();
    cx.net().stop_public();
    Ok(with_changes(cx, cx.light().tunnel.status()))
}

fn allow_changes(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let allow = p.opt_bool("allow")?.ok_or_else(|| porchly_core::Error::invalid("\"allow\" must be true or false."))?;
    set_changes(cx, allow)?;
    Ok(with_changes(cx, cx.light().tunnel.status()))
}

fn set_changes(cx: &mut Ctx<'_>, allow: bool) -> Result<()> {
    cx.store().set_setting(PORCH_LIGHT_CHANGES, allow.then_some("on"))?;
    cx.light().allow_changes.store(allow, Ordering::Relaxed);
    Ok(())
}
