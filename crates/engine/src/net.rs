//! The servers the engine runs for the desktop app. Both only ever listen on this computer
//! (127.0.0.1): the API itself, and, while the porch light is on, a second copy of it that
//! only the tunnel talks to. That copy is how Porchly knows a request came from the internet:
//! it can't make changes unless the owner allows them, and the functions' key doesn't work.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use porchly_core::{Error, Result};
use porchly_functions::Service;
use porchly_store::Shared;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

pub(crate) struct Net {
    /// False for previews and tests: never open ports.
    allowed: bool,
    state: Mutex<State>,
    /// The rate limit both servers use; Settings change it while they run.
    limits: Arc<porchly_api::Limits>,
    /// Usage numbers, counted by both servers together and kept across restarts of them.
    stats: Arc<porchly_api::Stats>,
}

#[derive(Default)]
struct State {
    runtime: Option<Runtime>,
    server: Option<Server>,
    /// The copy of the API behind the public link.
    public: Option<Server>,
}

struct Server {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    /// Set when the server task ends: `None` while running, the reason afterwards.
    ended: Arc<Mutex<Option<String>>>,
}

impl Net {
    pub(crate) fn new(allowed: bool) -> Self {
        Net {
            allowed,
            state: Mutex::new(State::default()),
            limits: Arc::new(porchly_api::Limits::default()),
            stats: Arc::new(porchly_api::Stats::default()),
        }
    }

    pub(crate) fn stats(&self) -> &Arc<porchly_api::Stats> {
        &self.stats
    }

    pub(crate) fn limits(&self) -> &Arc<porchly_api::Limits> {
        &self.limits
    }

    /// False for previews and tests: no ports, no downloads.
    pub(crate) fn allowed(&self) -> bool {
        self.allowed
    }

    /// Starts the server on 127.0.0.1. Without `exact`, a busy port moves on to the next free one.
    pub(crate) fn start(&self, store: &Shared, functions: &Arc<Service>, port: u16, exact: bool) -> Result<Value> {
        if !self.allowed {
            return Err(Error::invalid("The API can't be turned on in this mode."));
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.server.as_ref().is_some_and(Server::running) {
            return Ok(status_of(state.server.as_ref()));
        }
        let listener = porchly_api::bind(IpAddr::V4(Ipv4Addr::LOCALHOST), port, exact)?;
        let options = crate::cmd::api_options(functions, &self.limits, &self.stats);
        let server = spawn(&mut state, store, listener, options)?;
        functions.set_api_url(Some(format!("http://{}", server.addr)));
        state.server = Some(server);
        Ok(status_of(state.server.as_ref()))
    }

    /// Stops the server. Doesn't wait for requests in flight to finish.
    pub(crate) fn stop(&self) -> Value {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mut server) = state.server.take() {
            server.shut();
        }
        status_of(None)
    }

    pub(crate) fn status(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        status_of(state.server.as_ref())
    }

    /// Starts the copy of the API behind the public link. `port` 0 means any free port (a
    /// quick tunnel is told which); a stable address is set up in Cloudflare to point at one
    /// fixed port, so that one must be exactly `port`. Returns the port.
    pub(crate) fn start_public(
        &self,
        store: &Shared,
        functions: &Arc<Service>,
        allow_changes: &Arc<AtomicBool>,
        port: u16,
    ) -> Result<u16> {
        if !self.allowed {
            return Err(Error::invalid("The porch light can't be turned on in this mode."));
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(s) = state.public.as_ref().filter(|s| s.running() && (port == 0 || s.addr.port() == port)) {
            return Ok(s.addr.port());
        }
        if let Some(mut old) = state.public.take() {
            old.shut();
        }
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|e| {
            if port == 0 {
                Error::invalid(format!("Couldn't open a port for the porch light: {e}"))
            } else {
                Error::invalid(format!(
                    "Port {port}, which your stable address uses, is taken by another program on this computer ({e}). Close that program and try again."
                ))
            }
        })?;
        let options = porchly_api::Options {
            functions: Some(functions.clone() as Arc<dyn porchly_api::Functions>),
            internal_token: None,
            public: Some(allow_changes.clone()),
            limits: Some(self.limits.clone()),
            stats: Some(self.stats.clone()),
        };
        let server = spawn(&mut state, store, listener, options)?;
        let port = server.addr.port();
        state.public = Some(server);
        Ok(port)
    }

    pub(crate) fn stop_public(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mut server) = state.public.take() {
            server.shut();
        }
    }
}

/// Runs a server on the shared runtime (made on first use).
fn spawn(
    state: &mut State,
    store: &Shared,
    listener: std::net::TcpListener,
    options: porchly_api::Options,
) -> Result<Server> {
    let addr = listener.local_addr().map_err(|e| Error::Internal(format!("no local address: {e}")))?;
    if state.runtime.is_none() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("porchly-api")
            .enable_all()
            .build()
            .map_err(|e| Error::Internal(format!("couldn't start the server threads: {e}")))?;
        state.runtime = Some(runtime);
    }
    let Some(runtime) = state.runtime.as_ref() else {
        return Err(Error::Internal("server threads missing".into()));
    };
    let (stop, stopped) = oneshot::channel::<()>();
    let ended = Arc::new(Mutex::new(None));
    let ended_task = ended.clone();
    let store = store.clone();
    runtime.spawn(async move {
        let result = porchly_api::serve_with(store, listener, options, async {
            let _ = stopped.await;
        })
        .await;
        let reason = match result {
            Ok(()) => "stopped".to_owned(),
            Err(e) => format!("the server stopped: {e}"),
        };
        *ended_task.lock().unwrap_or_else(PoisonError::into_inner) = Some(reason);
    });
    Ok(Server { addr, stop: Some(stop), ended })
}

impl Server {
    fn running(&self) -> bool {
        self.ended.lock().unwrap_or_else(PoisonError::into_inner).is_none()
    }

    fn shut(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

fn status_of(server: Option<&Server>) -> Value {
    match server {
        Some(s) if s.running() => json!({
            "running": true,
            "address": s.addr.to_string(),
            "port": s.addr.port(),
            "url": format!("http://{}", s.addr),
            "only_this_computer": s.addr.ip().is_loopback(),
        }),
        Some(s) => json!({
            "running": false,
            "error": s.ended.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        }),
        None => json!({ "running": false }),
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        for server in [state.server.as_mut(), state.public.as_mut()].into_iter().flatten() {
            server.shut();
        }
        // Never block (or panic inside an async context) while shutting down.
        if let Some(runtime) = state.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
