//! Functions: turning the feature on and off, and making, editing and trying functions.
//! See `porchly-functions` and `plan/functions-design.md`.

use std::sync::Arc;

use porchly_api::FnAccess;
use porchly_core::{Error, Result};
use porchly_functions::{Service, Settings};
use porchly_store::Store;
use serde_json::{Value, json};

use super::{Command, always, to_json};
use crate::Ctx;
use crate::params::Params;

/// The project setting that records whether functions are on.
pub(crate) const FUNCTIONS_SETTING: &str = "functions";
/// Most log lines returned at once.
const MAX_LOG_LINES: usize = 2_000;

pub(crate) fn functions_on(store: &Store) -> bool {
    store.setting(FUNCTIONS_SETTING).ok().flatten().as_deref() == Some("on")
}

/// API options carrying the functions service and its internal key.
pub(crate) fn api_options(
    functions: &Arc<Service>,
    limits: &Arc<porchly_api::Limits>,
    stats: &Arc<porchly_api::Stats>,
) -> porchly_api::Options {
    porchly_api::Options {
        functions: Some(functions.clone() as Arc<dyn porchly_api::Functions>),
        internal_token: Some(functions.token().to_owned()),
        public: None,
        limits: Some(limits.clone()),
        stats: Some(stats.clone()),
    }
}

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "functions.status",
            label: "Functions status",
            menu: "",
            shortcut: None,
            params: "none. Returns {enabled, runtime: {ready, version, path, download_size}, download: {done, total} while downloading, folder, api_url}.",
            enabled: always,
            run: status,
        },
        Command {
            id: "functions.enable",
            label: "Turn on Functions",
            menu: "Functions/Turn on Functions…",
            shortcut: None,
            params: "none. Downloads Bun (about 40 MB, once) if it isn't there yet, then turns functions on for this project. Takes a while the first time.",
            enabled: always,
            run: enable,
        },
        Command {
            id: "functions.disable",
            label: "Turn off Functions",
            menu: "Functions/Turn off Functions…",
            shortcut: None,
            params: "remove_runtime?: bool (also delete the downloaded Bun; default false). Functions' code stays in the functions folder.",
            enabled: always,
            run: disable,
        },
        Command {
            id: "function.list",
            label: "List functions",
            menu: "",
            shortcut: None,
            params: "none. Returns [{name, access, file}].",
            enabled: functions_on,
            run: list,
        },
        Command {
            id: "function.create",
            label: "New function",
            menu: "Functions/New function…",
            shortcut: None,
            params: "name: text (lowercase letters, digits, _); template?: \"hello\" | \"tables\" (default hello).",
            enabled: functions_on,
            run: create,
        },
        Command {
            id: "function.get",
            label: "Open function",
            menu: "",
            shortcut: None,
            params: "name: text. Returns {name, code, access, env, file, path}.",
            enabled: functions_on,
            run: get,
        },
        Command {
            id: "function.save",
            label: "Save function",
            menu: "",
            shortcut: Some("Ctrl+S"),
            params: "name: text; code: text. The next call runs the new code.",
            enabled: functions_on,
            run: save,
        },
        Command {
            id: "function.settings",
            label: "Function settings",
            menu: "",
            shortcut: None,
            params: "name: text; access?: \"key\" (needs a key) | \"public\" (anyone, no key); env?: {NAME: value} (replaces all variables).",
            enabled: functions_on,
            run: settings,
        },
        Command {
            id: "function.rename",
            label: "Rename function",
            menu: "",
            shortcut: None,
            params: "name: text; new_name: text. Its address changes to /api/fn/<new_name>.",
            enabled: functions_on,
            run: rename,
        },
        Command {
            id: "function.delete",
            label: "Delete function",
            menu: "",
            shortcut: None,
            params: "name: text. The code is moved to functions/.deleted/, not erased.",
            enabled: functions_on,
            run: delete,
        },
        Command {
            id: "function.logs",
            label: "Function log",
            menu: "",
            shortcut: None,
            params: "name: text; lines?: number (default 200, at most 2000); clear?: bool. Returns {lines, path}.",
            enabled: functions_on,
            run: logs,
        },
        Command {
            id: "function.call",
            label: "Try function",
            menu: "",
            shortcut: None,
            params: "name: text; method?: text (default GET); path?: text (below the function's address); body?: text or JSON. Calls it through the running API. Returns {status, headers, body}.",
            enabled: functions_on,
            run: call,
        },
    ]
}

fn status(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    Ok(cx.functions().status())
}

fn enable(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    cx.functions().folder()?;
    if !cx.net().allowed() && cx.functions().bun().is_none() {
        return Err(Error::invalid("Functions can't be downloaded in this mode."));
    }
    // The download can take minutes: never hold the project while it runs.
    cx.unlock();
    cx.functions().ensure_runtime()?;
    cx.store().set_setting(FUNCTIONS_SETTING, Some("on"))?;
    cx.functions().set_enabled(true);
    Ok(cx.functions().status())
}

fn disable(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let remove = p.bool_or("remove_runtime", false)?;
    cx.store().set_setting(FUNCTIONS_SETTING, None)?;
    cx.functions().set_enabled(false);
    cx.unlock();
    if remove {
        cx.functions().remove_runtime()?;
    }
    Ok(cx.functions().status())
}

fn list(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    to_json(cx.functions().folder()?.list())
}

fn create(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let info = cx.functions().folder()?.create(p.str("name")?, p.opt_str("template")?)?;
    to_json(info)
}

fn get(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let name = p.str("name")?;
    let folder = cx.functions().folder()?;
    let code = folder.code(name)?;
    let settings = folder.settings(name)?;
    let path = folder.entry(name).map(|p| p.display().to_string());
    let info = folder.list().into_iter().find(|f| f.name == name);
    Ok(json!({
        "name": name,
        "code": code,
        "access": settings.access,
        "env": settings.env,
        "file": info.map(|i| i.file),
        "path": path,
    }))
}

fn save(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let name = p.str("name")?;
    cx.functions().folder()?.save(name, p.str("code")?)?;
    Ok(json!({ "saved": name }))
}

fn settings(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let name = p.str("name")?;
    let folder = cx.functions().folder()?;
    let mut settings: Settings = folder.settings(name)?;
    if let Some(access) = p.opt_str("access")? {
        settings.access = match access {
            "key" => FnAccess::Key,
            "public" => FnAccess::Public,
            _ => {
                return Err(Error::invalid("\"access\" must be \"key\" (needs a key) or \"public\" (anyone, no key)."));
            }
        };
    }
    if let Some(env) = p.opt_object("env")? {
        settings.env = env
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => Ok((k.clone(), s.clone())),
                _ => Err(Error::invalid(format!("The value of {k} must be text."))),
            })
            .collect::<Result<_>>()?;
    }
    folder.set_settings(name, &settings)?;
    Ok(json!({ "name": name, "access": settings.access, "env": settings.env }))
}

fn rename(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    to_json(cx.functions().folder()?.rename(p.str("name")?, p.str("new_name")?)?)
}

fn delete(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let kept = cx.functions().folder()?.delete(p.str("name")?)?;
    Ok(json!({ "kept_in": kept.display().to_string() }))
}

fn logs(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let name = p.str("name")?;
    let lines = match p.opt_scalar_text("lines")? {
        None => 200,
        Some(t) => {
            t.parse::<usize>().map_err(|_| Error::invalid("\"lines\" must be a whole number."))?.min(MAX_LOG_LINES)
        }
    };
    porchly_core::Ident::new(name)?;
    let logs = cx.functions().logs()?;
    if p.bool_or("clear", false)? {
        logs.clear(name);
    }
    Ok(json!({ "lines": logs.tail(name, lines), "path": logs.path(name).display().to_string() }))
}

fn call(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let name = p.str("name")?;
    porchly_core::Ident::new(name)?;
    let method = p.opt_str("method")?.unwrap_or("GET");
    let path = p.opt_str("path")?.unwrap_or_default();
    let body = match params.get("body") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(v) => Some(v.to_string()),
    };
    let functions = cx.functions().clone();
    let url = functions
        .api_url()
        .ok_or_else(|| Error::invalid("Turn on the API first: functions are reached through it."))?;
    cx.unlock();
    porchly_functions::call_blocking(&url, functions.token(), name, method, path, body.as_deref())
}
