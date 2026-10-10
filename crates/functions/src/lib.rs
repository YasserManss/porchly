//! Functions: JavaScript (or TypeScript) behind an API route, run with Bun, one process per
//! call. Off until the owner turns it on; turning it on downloads Bun once. Function code is
//! the owner's and isn't controlled or watched by Porchly (see CLAUDE.md, Plugins).
//! Design: `plan/functions-design.md`.

pub mod folder;
pub mod log;
mod runner;
pub mod runtime;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use axum::response::{IntoResponse, Response};
use porchly_api::{FnAccess, FnCall, FnSummary, Functions};
use porchly_core::{Error, Result};
use serde_json::{Value, json};

pub use folder::{FnInfo, Folder, Settings, TEMPLATES};
pub use log::Logs;
pub use runner::LAUNCHER;
use runner::Runner;

/// Functions for one project. Shared by the engine (commands) and the API server (calls).
pub struct Service {
    /// The folder holding the project file; `None` for projects in memory.
    project_dir: Option<PathBuf>,
    enabled: AtomicBool,
    /// The running API's address, which functions use to reach the tables.
    api_url: Mutex<Option<String>>,
    token: String,
    progress: runtime::Progress,
    downloading: AtomicBool,
    download_lock: Mutex<()>,
    /// A Bun program to use instead of finding or downloading one.
    bun_override: Mutex<Option<PathBuf>>,
    /// Seconds a function may take to answer; 0 means no limit.
    timeout_secs: std::sync::atomic::AtomicU64,
}

impl Service {
    /// `token` is the internal key functions get (see [`porchly_api::Options::internal_token`]).
    pub fn new(project_dir: Option<PathBuf>, enabled: bool, token: String) -> Self {
        Service {
            project_dir,
            enabled: AtomicBool::new(enabled),
            api_url: Mutex::new(None),
            token,
            progress: runtime::Progress::default(),
            downloading: AtomicBool::new(false),
            download_lock: Mutex::new(()),
            bun_override: Mutex::new(None),
            timeout_secs: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// How long a function may take to answer before it's stopped (`None`: no limit).
    pub fn set_timeout(&self, limit: Option<std::time::Duration>) {
        self.timeout_secs.store(limit.map_or(0, |d| d.as_secs().max(1)), Ordering::Relaxed);
    }

    pub fn timeout(&self) -> Option<std::time::Duration> {
        match self.timeout_secs.load(Ordering::Relaxed) {
            0 => None,
            s => Some(std::time::Duration::from_secs(s)),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    /// Tells functions where the API is (when it starts) or that it's gone (`None`).
    pub fn set_api_url(&self, url: Option<String>) {
        *self.api_url.lock().unwrap_or_else(PoisonError::into_inner) = url;
    }

    pub fn api_url(&self) -> Option<String> {
        self.api_url.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn dir(&self) -> Result<&Path> {
        self.project_dir
            .as_deref()
            .ok_or_else(|| Error::invalid("Functions need a project saved on disk (this one is only in memory)."))
    }

    pub fn folder(&self) -> Result<Folder> {
        Ok(Folder::new(self.dir()?.join("functions")))
    }

    pub fn logs(&self) -> Result<Logs> {
        Ok(Logs::new(self.dir()?.join("logs").join("functions")))
    }

    /// Where downloaded helper programs live: `tools/` next to the project.
    pub fn tools(&self) -> Result<PathBuf> {
        Ok(self.dir()?.join("tools"))
    }

    pub fn bun(&self) -> Option<PathBuf> {
        if let Some(p) = self.bun_override.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            return p.is_file().then_some(p);
        }
        runtime::find(&self.tools().ok()?)
    }

    /// Uses this Bun program instead of finding or downloading one (tests, `--bun`).
    pub fn use_bun(&self, path: Option<PathBuf>) {
        *self.bun_override.lock().unwrap_or_else(PoisonError::into_inner) = path;
    }

    /// Downloads Bun if it isn't there yet. Blocks; progress shows in [`Service::status`].
    pub fn ensure_runtime(&self) -> Result<PathBuf> {
        let tools = self.tools()?;
        let _one_at_a_time = self.download_lock.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(bun) = self.bun() {
            return Ok(bun);
        }
        if runtime::chosen_by_environment() {
            return Err(Error::invalid("PORCHLY_BUN doesn't point to a program. Fix it or unset it."));
        }
        self.downloading.store(true, Ordering::Relaxed);
        let result = runtime::download(&tools, &self.progress);
        self.downloading.store(false, Ordering::Relaxed);
        result
    }

    /// Removes Porchly's copy of Bun.
    pub fn remove_runtime(&self) -> Result<()> {
        let _one_at_a_time = self.download_lock.lock().unwrap_or_else(PoisonError::into_inner);
        runtime::remove(&self.tools()?)
    }

    /// Everything the app shows about functions.
    pub fn status(&self) -> Value {
        let (done, total) = self.progress.get();
        let downloading = self.downloading.load(Ordering::Relaxed);
        let bun = self.bun();
        json!({
            "enabled": self.is_enabled(),
            "available": self.project_dir.is_some(),
            "runtime": {
                "ready": bun.is_some(),
                "version": runtime::BUN_VERSION,
                "path": bun.map(|p| p.display().to_string()),
                "from_environment": runtime::chosen_by_environment(),
                "download_size": runtime::asset_for_this_computer().map(|a| a.size).ok(),
            },
            "download": if downloading { json!({ "done": done, "total": total }) } else { Value::Null },
            "folder": self.folder().ok().map(|f| f.root().display().to_string()),
            "api_url": self.api_url(),
        })
    }

    /// The launcher file, written next to Bun (rewritten when Porchly's copy changes).
    fn launcher(&self) -> Result<PathBuf> {
        let tools = self.tools()?;
        let path = tools.join("porchly-launcher.mjs");
        if std::fs::read_to_string(&path).ok().as_deref() != Some(LAUNCHER) {
            std::fs::create_dir_all(&tools)
                .and_then(|()| std::fs::write(&path, LAUNCHER))
                .map_err(|e| Error::invalid(format!("Couldn't write \"{}\": {e}", path.display())))?;
        }
        Ok(path)
    }

    fn runner(&self) -> Result<Runner> {
        if !self.is_enabled() {
            return Err(off());
        }
        let bun = self.bun().ok_or_else(|| {
            Error::invalid("Bun, the helper that runs functions, isn't downloaded. Turn Functions on again in Porchly.")
        })?;
        let porchly_url = self.api_url().unwrap_or_default();
        Ok(Runner {
            bun,
            launcher: self.launcher()?,
            folder: self.folder()?,
            logs: self.logs()?,
            cache: self.tools()?.join("bun-cache"),
            porchly_url,
            token: self.token.clone(),
            timeout: self.timeout(),
        })
    }
}

fn off() -> Error {
    Error::not_found("Functions are turned off in this project.")
}

impl Functions for Service {
    fn access(&self, name: &str) -> Result<FnAccess> {
        if !self.is_enabled() {
            return Err(off());
        }
        let folder = self.folder()?;
        if !folder.exists(name) {
            return Err(Error::not_found(format!(
                "There's no function \"{}\".",
                name.chars().take(60).collect::<String>()
            )));
        }
        Ok(folder.settings(name)?.access)
    }

    fn list(&self) -> Vec<FnSummary> {
        if !self.is_enabled() {
            return Vec::new();
        }
        let Ok(folder) = self.folder() else { return Vec::new() };
        folder.list().into_iter().map(|f| FnSummary { name: f.name, access: f.access }).collect()
    }

    fn call(&self, call: FnCall) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>> {
        match self.runner() {
            Ok(runner) => Box::pin(runner.call(call)),
            Err(e) => {
                let res = porchly_api::ApiError::from(e).into_response();
                Box::pin(async move { res })
            }
        }
    }
}

/// Calls a function through the running API, as an app would, and waits for the answer.
/// For the app's test panel and `function.call`. Returns status, headers and body.
pub fn call_blocking(
    api_url: &str,
    token: &str,
    name: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<Value> {
    let path = path.trim();
    let path =
        if path.is_empty() || path == "/" { String::new() } else { format!("/{}", path.trim_start_matches('/')) };
    let url = format!("{}/api/fn/{name}{path}", api_url.trim_end_matches('/'));
    let method: axum::http::Method =
        method.to_ascii_uppercase().parse().map_err(|_| Error::invalid("That isn't an HTTP method."))?;
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let request = axum::http::Request::builder()
        .method(method)
        .uri(&url)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    let failed = |e: &dyn std::fmt::Display| Error::invalid(format!("Couldn't reach the function: {e}"));
    let request = request.body(body.unwrap_or_default().to_owned()).map_err(|e| failed(&e))?;
    let response = agent.run(request).map_err(|e| failed(&e))?;
    let status = response.status().as_u16();
    let headers: serde_json::Map<String, Value> =
        response.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_owned(), json!(v.to_str().ok()?)))).collect();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(response.into_body().into_reader(), 1024 * 1024), &mut text)
        .map_err(|e| failed(&e))?;
    Ok(json!({ "status": status, "headers": headers, "body": text }))
}
