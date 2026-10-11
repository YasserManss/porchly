//! The porch light: a public link to the API through a Cloudflare tunnel. Either a free
//! quick tunnel (a new address each time), or the owner's own tunnel on their own domain (a
//! stable address), whose token is kept in the system keychain.
//! Off at every start; turning it on is always the person's choice.

use std::sync::atomic::Ordering;

use porchly_core::Result;
use serde_json::Value;

use super::{Command, always};
use crate::Ctx;
use crate::params::Params;

/// The project setting that remembers "Allow changes through the link".
pub(crate) const PORCH_LIGHT_CHANGES: &str = "porch_light_changes";
/// The project setting that remembers that a token for the owner's own address was saved
/// (the token itself is in the keychain). Without it, the keychain is never asked.
const OWN_ADDRESS: &str = "porch_light_own";
/// A random id for this project, so its secrets can be found again in the keychain.
const PROJECT_ID: &str = "project_id";
/// The port Porchly's public door uses with a stable address. The owner's Cloudflare tunnel
/// is set up to send visitors to `http://localhost:<this>`.
pub(crate) const PUBLIC_PORT: u16 = 11920;

pub(super) fn commands() -> Vec<Command> {
    vec![
        Command {
            id: "tunnel.status",
            label: "Porch light status",
            menu: "",
            shortcut: None,
            params: "none. Returns {state: off|downloading|connecting|on|failed, url, slow, error, allow_changes, download: {done, total}, own (using your own tunnel), problem (your tunnel is set up wrong in Cloudflare), has_token, token_lasting, service (what your tunnel should send to)}.",
            enabled: always,
            run: status,
        },
        Command {
            id: "tunnel.start",
            label: "Turn on the porch light",
            menu: "Share/Turn on the porch light…",
            shortcut: None,
            params: "allow_changes?: bool (default: as last set, else false); own?: bool (default: true when a token is saved with tunnel.token.set). Gives the API a public https link: your own stable address when own, otherwise a free quick link that changes each time. Downloads cloudflared (about 40 MB) the first time. Returns at once; the link appears in tunnel.status within seconds.",
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
            id: "tunnel.token.set",
            label: "Use your own address",
            menu: "",
            shortcut: None,
            params: "token: text (the token of your own Cloudflare tunnel, or the whole command Cloudflare shows). Kept in this computer's keychain, never in the project or its backups. In Cloudflare, the tunnel's public hostname must send to http://localhost:11920. Returns {has_token, token_lasting, service}.",
            enabled: always,
            run: token_set,
        },
        Command {
            id: "tunnel.token.clear",
            label: "Forget your own address",
            menu: "",
            shortcut: None,
            params: "none. Removes the saved token (and turns the porch light off if it was using it).",
            enabled: always,
            run: token_clear,
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
    full_status(cx)
}

fn with_changes(cx: &Ctx<'_>, mut status: Value) -> Value {
    if let Some(o) = status.as_object_mut() {
        o.insert("allow_changes".into(), Value::Bool(cx.light().allow_changes.load(Ordering::Relaxed)));
        o.insert("service".into(), Value::String(format!("http://localhost:{PUBLIC_PORT}")));
    }
    status
}

/// The status, plus whether a token for the owner's own address was saved.
fn full_status(cx: &mut Ctx<'_>) -> Result<Value> {
    let has_token = cx.store().setting(OWN_ADDRESS)?.is_some();
    let mut status = with_changes(cx, cx.light().tunnel.status());
    if let Some(o) = status.as_object_mut() {
        o.insert("has_token".into(), Value::Bool(has_token));
    }
    Ok(status)
}

/// The name this project's tunnel token has in the keychain.
fn token_name(cx: &mut Ctx<'_>) -> Result<String> {
    let store = cx.store();
    let id = match store.setting(PROJECT_ID)? {
        Some(id) => id,
        None => {
            let mut bytes = [0u8; 12];
            getrandom::fill(&mut bytes)
                .map_err(|e| porchly_core::Error::Internal(format!("no system randomness: {e}")))?;
            let id = data_encoding::HEXLOWER.encode(&bytes);
            store.set_setting(PROJECT_ID, Some(&id))?;
            id
        }
    };
    Ok(format!("porch-light-token:{id}"))
}

/// The saved token. `None` when none was saved. An error when one was saved but this
/// computer's keychain doesn't have it (another computer, or no keychain here).
fn saved_token(cx: &mut Ctx<'_>) -> Result<Option<String>> {
    if cx.store().setting(OWN_ADDRESS)?.is_none() {
        return Ok(None);
    }
    let name = token_name(cx)?;
    cx.unlock();
    match cx.light().secrets().get(&name)? {
        Some(token) => Ok(Some(token)),
        None => Err(porchly_core::Error::invalid(
            "Porchly can't find the token for your own address on this computer. Tokens are kept in each computer's keychain, not in the project. Paste it again (Share), or turn on a temporary link instead.",
        )),
    }
}

fn token_set(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let token = porchly_tunnel::clean_token(p.str("token")?)?;
    let name = token_name(cx)?;
    cx.unlock();
    let secrets = cx.light().secrets();
    secrets.set(&name, &token)?;
    cx.store().set_setting(OWN_ADDRESS, Some("on"))?;
    let mut out = full_status(cx)?;
    if let Some(o) = out.as_object_mut() {
        // False: this computer has no keychain, so the token is only kept until Porchly closes.
        o.insert("token_lasting".into(), Value::Bool(secrets.lasting()));
    }
    Ok(out)
}

fn token_clear(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    let name = token_name(cx)?;
    cx.unlock();
    cx.light().secrets().delete(&name)?;
    cx.store().set_setting(OWN_ADDRESS, None)?;
    cx.unlock();
    if cx.light().tunnel.status().get("own").and_then(Value::as_bool) == Some(true) {
        cx.light().tunnel.stop();
        cx.net().stop_public();
    }
    full_status(cx)
}

fn start(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    if let Some(allow) = p.opt_bool("allow_changes")? {
        set_changes(cx, allow)?;
    }
    // Your own address when a token is saved, unless told otherwise.
    let wanted = p.opt_bool("own")?;
    let token = match wanted {
        Some(false) => None,
        _ => saved_token(cx)?,
    };
    if wanted == Some(true) && token.is_none() {
        return Err(porchly_core::Error::invalid(
            "There's no token for your own address yet. Add it first (Share, or tunnel.token.set).",
        ));
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
    // A different kind of link than the one running: start over.
    let running_own = cx.light().tunnel.status().get("own").and_then(Value::as_bool) == Some(true);
    if cx.light().tunnel.is_running() && running_own != token.is_some() {
        cx.light().tunnel.stop();
    }
    let port = cx.net().start_public(&shared, &functions, &allow, if token.is_some() { PUBLIC_PORT } else { 0 })?;
    if let Err(e) = cx.light().tunnel.start(&program, port, token.as_deref()) {
        cx.net().stop_public();
        return Err(e);
    }
    full_status(cx)
}

fn stop(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    Params::new(params)?;
    cx.light().tunnel.stop();
    cx.net().stop_public();
    full_status(cx)
}

fn allow_changes(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    let allow = p.opt_bool("allow")?.ok_or_else(|| porchly_core::Error::invalid("\"allow\" must be true or false."))?;
    set_changes(cx, allow)?;
    full_status(cx)
}

fn set_changes(cx: &mut Ctx<'_>, allow: bool) -> Result<()> {
    cx.store().set_setting(PORCH_LIGHT_CHANGES, allow.then_some("on"))?;
    cx.light().allow_changes.store(allow, Ordering::Relaxed);
    Ok(())
}
