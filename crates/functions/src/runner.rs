//! Runs one call: starts Bun with Porchly's launcher and the function's code on a free
//! 127.0.0.1 port, forwards the one request, streams the response back, then stops Bun.
//! A crash or a hang only ever affects that call.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::http::{HeaderName, HeaderValue, StatusCode, Uri, Version, header};
use axum::response::{IntoResponse, Response};
use http_body::{Frame, SizeHint};
use hyper_util::rt::TokioIo;
use porchly_api::FnCall;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::folder::Folder;
use crate::log::Logs;

/// What the launcher prints once the function is listening.
const READY: &str = "porchly-ready ";
/// Lines of a failed start shown to the caller.
const ERROR_LINES: usize = 20;

/// The launcher: Porchly's only JavaScript. It loads the function and serves it on this
/// computer only (Bun has no flag for that), on a port the system picks.
pub const LAUNCHER: &str = r#"// Porchly starts this for each call to a function. Don't edit: Porchly rewrites it.
import { pathToFileURL } from "node:url";
const fn = (await import(pathToFileURL(process.argv[2]).href)).default;
if (!fn || typeof fn.fetch !== "function") {
  console.error("This function needs: export default { async fetch(request) { ... return new Response(...) } }");
  process.exit(1);
}
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  development: false,
  fetch: (request, server) => fn.fetch(request, server),
  error(e) {
    console.error(e);
    return Response.json({ error: { code: "function_failed", message: "The function failed. Its log in Porchly has the details." } }, { status: 500 });
  },
});
console.log("porchly-ready " + server.port);
"#;

/// Environment variables passed on from Porchly to functions. Nothing else is: functions
/// only see these, Porchly's own variables, and the variables set for them.
const PASSED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "SYSTEMROOT",
    "SystemRoot",
    "WINDIR",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "TZ",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// Everything a call needs.
#[derive(Debug, Clone)]
pub struct Runner {
    pub bun: PathBuf,
    pub launcher: PathBuf,
    pub folder: Folder,
    pub logs: Logs,
    /// Where Bun keeps npm packages it downloads.
    pub cache: PathBuf,
    /// The API's own address, for functions to reach the tables.
    pub porchly_url: String,
    pub token: String,
    /// Stop a function that hasn't answered by then (`None`: no limit).
    pub timeout: Option<std::time::Duration>,
}

impl Runner {
    pub async fn call(self, call: FnCall) -> Response {
        let name = call.name.clone();
        let (timeout, logs) = (self.timeout, self.logs.clone());
        let result = match timeout {
            None => self.run(call).await,
            // Dropping the call when time is up stops Bun (kill on drop).
            Some(limit) => tokio::time::timeout(limit, self.run(call)).await.unwrap_or_else(|_| {
                let message = format!(
                    "The function \"{name}\" didn't answer within {} and was stopped. The time limit is in Porchly's Settings.",
                    describe(limit)
                );
                logs.write(&name, "err", &message);
                Err(Failure { status: StatusCode::GATEWAY_TIMEOUT, code: "function_timeout", message })
            }),
        };
        match result {
            Ok(res) => res,
            Err(failure) => {
                tracing::warn!("function {name}: {}", failure.message);
                let body = json!({ "error": { "code": failure.code, "message": failure.message } });
                (failure.status, axum::Json(body)).into_response()
            }
        }
    }

    async fn run(self, call: FnCall) -> Result<Response, Failure> {
        let FnCall { name, path_and_query, request, caller } = call;
        let entry = self.folder.entry(&name).ok_or_else(|| Failure::not_found(&name))?;
        let settings = self.folder.settings(&name).map_err(|_| Failure::not_found(&name))?;
        let workdir = entry.parent().map(PathBuf::from).unwrap_or_else(|| self.folder.root().to_path_buf());

        let mut cmd = Command::new(&self.bun);
        cmd.arg(&self.launcher)
            .arg(&entry)
            .current_dir(&workdir)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for key in PASSED_ENV {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
        cmd.envs(&settings.env)
            .env("PORCHLY_URL", &self.porchly_url)
            .env("PORCHLY_TOKEN", &self.token)
            .env("PORCHLY_FUNCTION", &name)
            .env("BUN_INSTALL_CACHE_DIR", &self.cache)
            .env("NO_COLOR", "1");
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window flashing up.

        let mut child = cmd.spawn().map_err(|e| Failure {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "functions_unavailable",
            message: format!("Porchly couldn't start Bun to run the function: {e}"),
        })?;

        // Collect the output: the ready line gives the port, everything else goes to the log.
        let recent = Arc::new(Mutex::new(VecDeque::new()));
        let (port_tx, port_rx) = tokio::sync::oneshot::channel::<u16>();
        let mut stdout_task = None;
        if let Some(out) = child.stdout.take() {
            let (logs, name) = (self.logs.clone(), name.clone());
            stdout_task = Some(tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                let mut port_tx = Some(port_tx);
                while let Ok(Some(line)) = lines.next_line().await {
                    if port_tx.is_some()
                        && let Some(port) = line.strip_prefix(READY).and_then(|p| p.trim().parse::<u16>().ok())
                    {
                        if let Some(tx) = port_tx.take() {
                            let _ = tx.send(port);
                        }
                        continue;
                    }
                    logs.write(&name, "out", &line);
                }
            }));
        }
        let mut stderr_task = None;
        if let Some(err) = child.stderr.take() {
            let (logs, name, recent) = (self.logs.clone(), name.clone(), recent.clone());
            stderr_task = Some(tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    logs.write(&name, "err", &line);
                    let mut r = recent.lock().unwrap_or_else(PoisonError::into_inner);
                    if r.len() == ERROR_LINES {
                        r.pop_front();
                    }
                    r.push_back(line);
                }
            }));
        }

        let Ok(port) = port_rx.await else {
            // It stopped without starting to listen: a syntax error, a bad import…
            let _ = child.wait().await;
            for task in [stdout_task, stderr_task].into_iter().flatten() {
                let _ = task.await;
            }
            let lines: Vec<String> = recent.lock().unwrap_or_else(PoisonError::into_inner).iter().cloned().collect();
            let detail = if lines.is_empty() { "It stopped without saying why.".to_owned() } else { lines.join("\n") };
            return Err(Failure {
                status: StatusCode::BAD_GATEWAY,
                code: "function_failed",
                message: format!("The function \"{name}\" couldn't start.\n{detail}"),
            });
        };

        let stream =
            tokio::net::TcpStream::connect(("127.0.0.1", port)).await.map_err(|e| Failure::stopped(&name, &e))?;
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| Failure::stopped(&name, &e))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });

        let (mut parts, body) = request.into_parts();
        parts.uri = path_and_query.parse::<Uri>().map_err(|_| Failure {
            status: StatusCode::BAD_REQUEST,
            code: "invalid",
            message: "The address isn't valid.".into(),
        })?;
        parts.version = Version::HTTP_11;
        strip_hop_by_hop(&mut parts.headers);
        parts.headers.remove(header::AUTHORIZATION);
        let ours: Vec<HeaderName> =
            parts.headers.keys().filter(|k| k.as_str().starts_with("x-porchly-")).cloned().collect();
        for k in ours {
            parts.headers.remove(k);
        }
        if let Some(key) = caller.key_name.as_deref().and_then(|k| HeaderValue::from_str(k).ok()) {
            parts.headers.insert(HeaderName::from_static("x-porchly-key-name"), key);
        }
        parts.headers.insert(
            HeaderName::from_static("x-porchly-key-write"),
            HeaderValue::from_static(if caller.can_write { "true" } else { "false" }),
        );
        let request = axum::http::Request::from_parts(parts, body);

        let response = sender.send_request(request).await.map_err(|e| Failure::stopped(&name, &e))?;
        let (mut parts, incoming) = response.into_parts();
        strip_hop_by_hop(&mut parts.headers);
        // Bun stops when the response has been sent (or the caller goes away).
        Ok(Response::from_parts(parts, Body::new(UntilDone { inner: incoming, _bun: child })))
    }
}

/// "30 seconds", "5 minutes".
fn describe(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    match secs {
        60 => "1 minute".into(),
        s if s > 60 && s % 60 == 0 => format!("{} minutes", s / 60),
        1 => "1 second".into(),
        s => format!("{s} seconds"),
    }
}

/// Headers that belong to one connection, not to the request or response.
fn strip_hop_by_hop(headers: &mut axum::http::HeaderMap) {
    for h in [
        header::CONNECTION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
    ] {
        headers.remove(h);
    }
    headers.remove("keep-alive");
    headers.remove("proxy-connection");
}

/// The response body, holding on to the Bun process until the body is finished or dropped.
struct UntilDone {
    inner: hyper::body::Incoming,
    _bun: Child,
}

impl http_body::Body for UntilDone {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

struct Failure {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl Failure {
    fn not_found(name: &str) -> Self {
        Failure {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: format!("There's no function \"{name}\"."),
        }
    }

    fn stopped(name: &str, e: &dyn std::fmt::Display) -> Self {
        Failure {
            status: StatusCode::BAD_GATEWAY,
            code: "function_failed",
            message: format!("The function \"{name}\" stopped before it answered ({e}). Its log may say why."),
        }
    }
}
