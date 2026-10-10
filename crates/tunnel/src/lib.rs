//! The porch light: a public `https://…trycloudflare.com` link to the project's API, through
//! a Cloudflare quick tunnel (no account needed). Off until the person turns it on; off again
//! every time Porchly starts. Only the API is ever behind the link.
//!
//! `cloudflared` makes outgoing connections to Cloudflare (port 7844), so no router settings
//! or administrator rights are needed. Quick tunnels are meant for trying things out: the
//! address changes every time and has no uptime guarantee.

pub mod runtime;

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use porchly_core::{Error, Result};
use porchly_fetch::Progress;
use serde_json::{Value, json};

/// Lines kept to explain a failure.
const RECENT_LINES: usize = 12;
/// After this long without a connection, the status says the network may be blocking it.
const SLOW_SECS: u64 = 30;
/// The porch light's log rotates at this size.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// A small config file of our own, so a `~/.cloudflared/config.yml` on this computer (which
/// breaks quick tunnels) is never read.
const CONFIG: &str = "# Written by Porchly for the porch light. Porchly rewrites it.\nno-autoupdate: true\n";

/// Where things live, and the tunnel while it runs.
pub struct PorchLight {
    /// `tools/` next to the project (`None` for projects in memory).
    tools: Option<PathBuf>,
    log: Option<PathBuf>,
    progress: Progress,
    downloading: AtomicBool,
    download_lock: Mutex<()>,
    running: Mutex<Option<Running>>,
    /// A program to use instead of finding or downloading one (tests).
    program_override: Mutex<Option<PathBuf>>,
}

struct Running {
    child: Child,
    state: Arc<Mutex<State>>,
    started: Instant,
}

#[derive(Debug, Default)]
struct State {
    url: Option<String>,
    connected: bool,
    /// Why it stopped, once it has.
    ended: Option<String>,
    recent: VecDeque<String>,
}

impl PorchLight {
    /// `project_dir` is the folder holding the project file.
    pub fn new(project_dir: Option<&Path>) -> Self {
        PorchLight {
            tools: project_dir.map(|d| d.join("tools")),
            log: project_dir.map(|d| d.join("logs").join("porch-light.log")),
            progress: Progress::default(),
            downloading: AtomicBool::new(false),
            download_lock: Mutex::new(()),
            running: Mutex::new(None),
            program_override: Mutex::new(None),
        }
    }

    fn tools(&self) -> Result<&Path> {
        self.tools.as_deref().ok_or_else(|| {
            Error::invalid("The porch light needs a project saved on disk (this one is only in memory).")
        })
    }

    /// Uses this cloudflared instead of finding or downloading one (tests).
    pub fn use_program(&self, path: Option<PathBuf>) {
        *self.program_override.lock().unwrap_or_else(PoisonError::into_inner) = path;
    }

    pub fn program(&self) -> Option<PathBuf> {
        if let Some(p) = self.program_override.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            return p.is_file().then_some(p);
        }
        runtime::find(self.tools().ok()?)
    }

    /// Downloads cloudflared if it isn't there yet. Blocks; progress shows in [`PorchLight::status`].
    pub fn ensure_program(&self, may_download: bool) -> Result<PathBuf> {
        let _one_at_a_time = self.download_lock.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(p) = self.program() {
            return Ok(p);
        }
        if runtime::chosen_by_environment() {
            return Err(Error::invalid("PORCHLY_CLOUDFLARED doesn't point to a program. Fix it or unset it."));
        }
        if !may_download {
            return Err(Error::invalid("The porch light can't be downloaded in this mode."));
        }
        let tools = self.tools()?.to_path_buf();
        self.downloading.store(true, Ordering::Relaxed);
        let result = runtime::download(&tools, &self.progress);
        self.downloading.store(false, Ordering::Relaxed);
        result
    }

    /// Starts the tunnel to `http://127.0.0.1:<local_port>`. Doesn't wait for the link; watch
    /// [`PorchLight::status`].
    pub fn start(&self, program: &Path, local_port: u16) -> Result<()> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(r) = running.as_mut()
            && r.alive()
        {
            return Ok(());
        }
        let tools = self.tools()?;
        let config = tools.join("porchly-cloudflared.yml");
        std::fs::create_dir_all(tools)
            .and_then(|()| std::fs::write(&config, CONFIG))
            .map_err(|e| porchly_fetch::write_error(&config, &e))?;
        let mut cmd = Command::new(program);
        cmd.args(["tunnel", "--no-autoupdate", "--config"])
            .arg(&config)
            .args(["--metrics", "127.0.0.1:0", "--url"])
            .arg(format!("http://127.0.0.1:{local_port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child =
            cmd.spawn().map_err(|e| Error::invalid(format!("Porchly couldn't start the porch light: {e}")))?;
        let state = Arc::new(Mutex::new(State::default()));
        if let Some(err) = child.stderr.take() {
            let (state, log) = (state.clone(), self.log.clone());
            let spawned = std::thread::Builder::new().name("porchly-porch-light".into()).spawn(move || {
                for line in BufReader::new(err).lines().map_while(std::result::Result::ok) {
                    if let Some(log) = &log {
                        write_log(log, &line);
                    }
                    let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
                    if s.url.is_none() {
                        s.url = find_url(&line);
                    }
                    if line.contains("Registered tunnel connection") {
                        s.connected = true;
                    }
                    if s.recent.len() == RECENT_LINES {
                        s.recent.pop_front();
                    }
                    s.recent.push_back(line);
                }
                let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
                let reason = explain_end(&s);
                s.ended = Some(reason);
                s.connected = false;
            });
            if let Err(e) = spawned {
                let _ = child.kill();
                return Err(Error::Internal(format!("couldn't watch the porch light: {e}")));
            }
        }
        *running = Some(Running { child, state, started: Instant::now() });
        Ok(())
    }

    /// Turns the light off.
    pub fn stop(&self) {
        if let Some(mut r) = self.running.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().unwrap_or_else(PoisonError::into_inner).as_mut().is_some_and(Running::alive)
    }

    /// `state`: "off", "downloading", "connecting", "on" or "failed"; `url` once known;
    /// `slow` when connecting takes long; `error` when it failed.
    pub fn status(&self) -> Value {
        let (done, total) = self.progress.get();
        let downloading = self.downloading.load(Ordering::Relaxed);
        let program = self.program();
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        let base = json!({
            "program_ready": program.is_some(),
            "version": runtime::VERSION,
            "download_size": runtime::asset_for_this_computer().map(|a| a.size).ok(),
        });
        let mut out = match running.as_mut() {
            None if downloading => json!({ "state": "downloading", "download": { "done": done, "total": total } }),
            None => json!({ "state": "off" }),
            Some(r) => {
                let alive = r.alive();
                let s = r.state.lock().unwrap_or_else(PoisonError::into_inner);
                let elapsed = r.started.elapsed().as_secs();
                match (&s.ended, alive, s.connected, &s.url) {
                    (Some(reason), _, _, _) => json!({ "state": "failed", "error": reason }),
                    (None, false, _, _) => json!({ "state": "failed", "error": explain_end(&s) }),
                    (None, true, true, Some(url)) => json!({ "state": "on", "url": url }),
                    (None, true, _, url) => json!({ "state": "connecting", "url": url, "slow": elapsed >= SLOW_SECS }),
                }
            }
        };
        if let (Some(o), Some(b)) = (out.as_object_mut(), base.as_object()) {
            o.extend(b.clone());
        }
        out
    }
}

impl Running {
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for PorchLight {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The quick tunnel's address in a line of cloudflared's output, if there is one.
fn find_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = line.get(start..)?;
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || ":/.-".contains(c))).unwrap_or(rest.len());
    let url = rest.get(..end)?.trim_end_matches(['.', '/']);
    let host = url.strip_prefix("https://")?;
    let ok = host.ends_with(".trycloudflare.com")
        && host != "api.trycloudflare.com"
        && host.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
    ok.then(|| url.to_owned())
}

/// A plain-language reason the tunnel stopped, from what cloudflared said last.
fn explain_end(s: &State) -> String {
    let said = |needle: &str| s.recent.iter().any(|l| l.contains(needle));
    if said("429") || said("Too Many Requests") {
        "Cloudflare is handing out too many free links right now. Try again in a few minutes.".into()
    } else if s.url.is_none() {
        "Couldn't reach Cloudflare. Check that this computer is connected to the internet. Some networks (offices, hotels, schools) block this kind of link."
            .into()
    } else {
        let last = s.recent.iter().rev().find(|l| l.contains(" ERR ")).cloned().unwrap_or_default();
        format!("The public link stopped. {last}").trim().to_owned()
    }
}

fn write_log(path: &Path, line: &str) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", line.chars().take(4_000).collect::<String>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_address() {
        let line = "2026-01-15T11:06:05Z INF |  https://myself-bowling-attend-carpet.trycloudflare.com                                    |";
        assert_eq!(find_url(line).as_deref(), Some("https://myself-bowling-attend-carpet.trycloudflare.com"));
        assert_eq!(find_url("INF Requesting new quick Tunnel on trycloudflare.com..."), None);
        assert_eq!(find_url("see https://api.trycloudflare.com/tunnel"), None);
        assert_eq!(find_url("https://evil.example.com/x.trycloudflare.com"), None);
        assert_eq!(find_url("https://developers.cloudflare.com/x"), None);
    }

    #[test]
    fn explains_failures() {
        let mut s = State::default();
        assert!(explain_end(&s).contains("Couldn't reach Cloudflare"));
        s.recent.push_back("ERR failed to request quick Tunnel: 429 Too Many Requests".into());
        assert!(explain_end(&s).contains("too many free links"));
    }

    #[test]
    fn off_and_in_memory() {
        let light = PorchLight::new(None);
        assert_eq!(light.status()["state"], "off");
        assert!(light.ensure_program(true).is_err() || runtime::chosen_by_environment());
        assert!(light.start(Path::new("/nonexistent"), 1).is_err());
    }

    #[test]
    fn a_program_that_exits_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let light = PorchLight::new(Some(dir.path()));
        // Any program that exits at once stands in for a cloudflared that couldn't connect.
        let program = if cfg!(windows) {
            PathBuf::from("C:\\Windows\\System32\\whoami.exe")
        } else {
            PathBuf::from("/bin/false")
        };
        if !program.is_file() {
            return;
        }
        light.start(&program, 1).unwrap();
        for _ in 0..50 {
            if light.status()["state"] == "failed" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let status = light.status();
        assert_eq!(status["state"], "failed", "{status}");
        assert!(status["error"].as_str().unwrap().contains("Couldn't reach Cloudflare"));
        assert!(dir.path().join("tools/porchly-cloudflared.yml").is_file());
        light.stop();
        assert_eq!(light.status()["state"], "off");
    }
}
