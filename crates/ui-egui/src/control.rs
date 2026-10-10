//! The control channel: local tools (scripts, tests, AI agents) drive the running app with
//! JSON lines over TCP on 127.0.0.1. Off unless the app is started with `--control <port>`.
//! Every connection must first send the token (`auth`), so other programs and other people on
//! the same computer can't drive it. See `docs/control-protocol.md`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde_json::{Value, json};

/// Longest request line accepted.
const MAX_LINE: u64 = 1024 * 1024;
/// How long a connection waits for the app to answer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// A request waiting for the UI thread, with where to send the answer.
pub(crate) struct Request {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Result<Value, String>>,
}

/// A running control channel.
pub(crate) struct Control {
    pub port: u16,
    pub token: String,
    pub requests: Receiver<Request>,
}

/// Starts listening on 127.0.0.1:`port` (0 picks a free port). `token` is what clients must
/// send first; when `None`, a random one is made.
pub(crate) fn start(port: u16, token: Option<String>, ctx: &egui::Context) -> Result<Control, String> {
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|e| format!("couldn't listen on port {port}: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = match token.filter(|t| t.len() >= 16) {
        Some(t) => t,
        None => random_token()?,
    };
    let (tx, rx) = mpsc::channel::<Request>();
    let ctx = ctx.clone();
    let expected = token.clone();
    std::thread::Builder::new()
        .name("porchly-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, ctx, expected) = (tx.clone(), ctx.clone(), expected.clone());
                let _ = std::thread::Builder::new().name("porchly-control-conn".into()).spawn(move || {
                    if let Err(e) = serve(stream, &tx, &ctx, &expected) {
                        tracing::debug!("control connection ended: {e}");
                    }
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Control { port, token, requests: rx })
}

fn serve(stream: TcpStream, tx: &Sender<Request>, ctx: &egui::Context, token: &str) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut authed = false;
    loop {
        let mut line = String::new();
        let n = (&mut reader).take(MAX_LINE).read_line(&mut line)?;
        if n == 0 {
            return Ok(());
        }
        if !line.ends_with('\n') && n as u64 >= MAX_LINE {
            writeln!(out, "{}", json!({"id": null, "error": {"code": "too_large", "message": "Line too long."}}))?;
            return Ok(());
        }
        let msg: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(e) => {
                writeln!(
                    out,
                    "{}",
                    json!({"id": null, "error": {"code": "invalid", "message": format!("Not JSON: {e}")}})
                )?;
                continue;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or_default().to_owned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        if !authed {
            let given = params.get("token").and_then(Value::as_str).unwrap_or_default();
            if method == "auth" && constant_time_eq(given.as_bytes(), token.as_bytes()) {
                authed = true;
                writeln!(out, "{}", json!({"id": id, "result": {"ok": true}}))?;
                continue;
            }
            writeln!(
                out,
                "{}",
                json!({"id": id, "error": {"code": "unauthorized", "message": "Send {\"method\":\"auth\",\"params\":{\"token\":\"...\"}} first."}})
            )?;
            return Ok(());
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        if tx.send(Request { method, params, reply: reply_tx }).is_err() {
            return Ok(());
        }
        ctx.request_repaint();
        let answer = match reply_rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(result)) => json!({"id": id, "result": result}),
            Ok(Err(message)) => json!({"id": id, "error": {"code": "failed", "message": message}}),
            Err(_) => json!({"id": id, "error": {"code": "timeout", "message": "The app didn't answer in time."}}),
        };
        writeln!(out, "{answer}")?;
    }
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| format!("no system randomness: {e}"))?;
    Ok(data_encoding::HEXLOWER.encode(&bytes))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Encodes a screenshot as PNG: written to `path`, or returned as base64.
pub(crate) fn png(image: &egui::ColorImage, path: Option<&str>) -> Result<Value, String> {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let buffer = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("screenshot has the wrong size")?;
    let mut bytes = Vec::new();
    buffer
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| format!("couldn't encode the screenshot: {e}"))?;
    match path {
        Some(p) => {
            std::fs::write(p, &bytes).map_err(|e| format!("couldn't write {p}: {e}"))?;
            Ok(json!({"path": p, "width": w, "height": h}))
        }
        None => Ok(json!({"png_base64": data_encoding::BASE64.encode(&bytes), "width": w, "height": h})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let a = random_token().unwrap();
        assert_eq!(a.len(), 48);
        assert_ne!(a, random_token().unwrap());
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
