//! Settings: a few plain choices, kept per project. Each has a safe default, and changes
//! take effect at once (a new port restarts the API).

use std::time::Duration;

use porchly_core::{Error, Result};
use porchly_functions::Service;
use porchly_store::Store;
use serde_json::{Value, json};

use super::{Command, always};
use crate::Ctx;
use crate::net::Net;
use crate::params::Params;

const RATE: &str = "requests_per_second";
/// Saved by earlier versions as a named choice; read when no number is saved.
const OLD_RATE: &str = "rate_limit";
const TIMEOUT: &str = "function_timeout";
const PORT: &str = "port";
const START_API: &str = "start_api";
/// The command parameter for letting AI assistants make tables and fields.
const AI_TABLES: &str = "ai_can_make_tables";

/// Requests per second each app (key) may make.
const MAX_RATE: u32 = 1000;

/// Function time limits, in seconds (0: no limit).
const TIMEOUTS: [u64; 5] = [10, 30, 60, 300, 0];

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "settings.get",
            label: "Settings",
            menu: "Porchly/Settings…",
            shortcut: None,
            params: "none. Returns {requests_per_second, function_timeout: seconds (0 = no limit), port, start_api, ai_can_make_tables, choices}.",
            enabled: always,
            run: get,
        },
        Command {
            id: "settings.set",
            label: "Change settings",
            menu: "",
            shortcut: None,
            params: "Any of: requests_per_second: 1-1000 (per app/key; default 20); function_timeout: 10 | 30 | 60 | 300 | 0 (seconds, 0 = no limit); port: 1024-65535 (restarts the API if it's on); start_api: bool (turn the API on when the app opens); ai_can_make_tables: bool (AI assistants connected over MCP may make tables and fields; default false).",
            enabled: always,
            run: set,
        },
    ]
}

/// Requests per second each app (key) may make.
fn rate(store: &Store) -> u32 {
    let saved = store.setting(RATE).ok().flatten().and_then(|r| r.parse::<u32>().ok());
    let old = || match store.setting(OLD_RATE).ok().flatten().as_deref() {
        Some("gentle") => Some(5),
        Some("busy") => Some(100),
        _ => None,
    };
    saved.or_else(old).filter(|r| (1..=MAX_RATE).contains(r)).unwrap_or(porchly_api::DEFAULT_PER_SECOND)
}

fn timeout_secs(store: &Store) -> u64 {
    store.setting(TIMEOUT).ok().flatten().and_then(|t| t.parse().ok()).filter(|t| TIMEOUTS.contains(t)).unwrap_or(0)
}

/// The port the API uses unless told otherwise.
pub(crate) fn port_setting(store: &Store) -> u16 {
    store
        .setting(PORT)
        .ok()
        .flatten()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1024)
        .unwrap_or(porchly_api::DEFAULT_PORT)
}

fn start_api(store: &Store) -> bool {
    store.setting(START_API).ok().flatten().as_deref() != Some("off")
}

/// Puts the saved settings into effect (at start, and after a change).
pub(crate) fn apply_settings(store: &Store, net: &Net, functions: &Service) {
    net.limits().set_per_second(rate(store));
    functions.set_timeout(match timeout_secs(store) {
        0 => None,
        s => Some(Duration::from_secs(s)),
    });
}

fn get(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    Ok(snapshot(cx.store()))
}

fn snapshot(store: &Store) -> Value {
    json!({
        "requests_per_second": rate(store),
        "function_timeout": timeout_secs(store),
        "port": port_setting(store),
        "start_api": start_api(store),
        "ai_can_make_tables": store.setting(porchly_api::mcp::TABLES_SETTING).ok().flatten().as_deref() == Some("on"),
        "choices": {
            "requests_per_second": { "min": 1, "max": MAX_RATE },
            "function_timeout": TIMEOUTS,
        },
    })
}

fn set(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    // Check everything first, so a bad value changes nothing.
    let rate = match p.opt_scalar_text(RATE)? {
        None => None,
        Some(t) => Some(t.trim().parse::<u32>().ok().filter(|r| (1..=MAX_RATE).contains(r)).ok_or_else(|| {
            Error::invalid(format!("\"requests_per_second\" must be a whole number from 1 to {MAX_RATE}."))
        })?),
    };
    let timeout = match p.opt_scalar_text(TIMEOUT)? {
        None => None,
        Some(t) => Some(t.parse::<u64>().ok().filter(|t| TIMEOUTS.contains(t)).ok_or_else(|| {
            Error::invalid("\"function_timeout\" must be 10, 30, 60 or 300 seconds, or 0 for no limit.")
        })?),
    };
    let port = match p.opt_scalar_text(PORT)? {
        None => None,
        Some(t) => Some(t.parse::<u16>().ok().filter(|p| *p >= 1024).ok_or_else(|| {
            Error::invalid(
                "The port must be a whole number from 1024 to 65535. Lower numbers need administrator rights.",
            )
        })?),
    };
    let start = p.opt_bool(START_API)?;
    let ai_tables = p.opt_bool(AI_TABLES)?;

    let store = cx.store();
    if let Some(r) = rate {
        store.set_setting(RATE, Some(&r.to_string()))?;
        store.set_setting(OLD_RATE, None)?;
    }
    if let Some(t) = timeout {
        store.set_setting(TIMEOUT, Some(&t.to_string()))?;
    }
    let old_port = port_setting(store);
    if let Some(port) = port {
        store.set_setting(PORT, (port != porchly_api::DEFAULT_PORT).then(|| port.to_string()).as_deref())?;
    }
    if let Some(on) = start {
        store.set_setting(START_API, (!on).then_some("off"))?;
    }
    if let Some(on) = ai_tables {
        store.set_setting(porchly_api::mcp::TABLES_SETTING, on.then_some("on"))?;
    }
    let (net, functions) = (cx.net(), cx.functions().clone());
    apply_settings(cx.store(), net, &functions);
    let mut out = snapshot(cx.store());

    // A new port: move the running API to it (the porch light keeps working; it has its own).
    if let Some(port) = port.filter(|p| *p != old_port)
        && cx.net().status().get("running").and_then(Value::as_bool) == Some(true)
    {
        cx.unlock();
        cx.net().stop();
        let shared = cx.shared().clone();
        let status = cx.net().start(&shared, &functions, port, true)?;
        if let Some(o) = out.as_object_mut() {
            o.insert("api".into(), status);
        }
    }
    Ok(out)
}
