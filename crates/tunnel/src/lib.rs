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
    /// True when running the owner's own tunnel (a stable address) rather than a quick one.
    own: bool,
    local_port: u16,
}

#[derive(Debug, Default)]
struct State {
    url: Option<String>,
    connected: bool,
    /// With your own tunnel: the addresses Cloudflare sends here, as (address, where to).
    routes: Vec<(String, String)>,
    /// Whether Cloudflare has told us the tunnel's routes yet.
    routes_known: bool,
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
    /// [`PorchLight::status`]. Without `token`, a free quick tunnel with a new address each
    /// time. With the `token` of the owner's own Cloudflare tunnel, their stable address.
    pub fn start(&self, program: &Path, local_port: u16, token: Option<&str>) -> Result<()> {
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
        cmd.args(["tunnel", "--no-autoupdate", "--config"]).arg(&config).args(["--metrics", "127.0.0.1:0"]);
        match token {
            // In the environment, not on the command line, where other programs could read it.
            Some(token) => cmd.arg("run").env("TUNNEL_TOKEN", token),
            None => cmd.arg("--url").arg(format!("http://127.0.0.1:{local_port}")),
        };
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
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
            let secret = token.map(str::to_owned);
            let spawned = std::thread::Builder::new().name("porchly-porch-light".into()).spawn(move || {
                for line in BufReader::new(err).lines().map_while(std::result::Result::ok) {
                    // The token must never reach the log or an error message.
                    let line = match &secret {
                        Some(t) if line.contains(t.as_str()) => line.replace(t.as_str(), "[token]"),
                        _ => line,
                    };
                    if let Some(log) = &log {
                        write_log(log, &line);
                    }
                    let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
                    if secret.is_some() {
                        if let Some(routes) = find_routes(&line) {
                            s.routes = routes;
                            s.routes_known = true;
                        }
                        // cloudflared keeps retrying a token Cloudflare refuses, forever:
                        // say so instead of "connecting" without end.
                        if !s.connected && token_refused(&line) {
                            s.ended = Some(REFUSED.to_owned());
                        }
                    } else if s.url.is_none() {
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
                let reason = if secret.is_some() { explain_own_end(&s) } else { explain_end(&s) };
                s.ended.get_or_insert(reason);
                s.connected = false;
            });
            if let Err(e) = spawned {
                let _ = child.kill();
                return Err(Error::Internal(format!("couldn't watch the porch light: {e}")));
            }
        }
        *running = Some(Running { child, state, started: Instant::now(), own: token.is_some(), local_port });
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
                if r.own {
                    match (&s.ended, alive, s.connected) {
                        (Some(reason), _, _) => json!({ "state": "failed", "error": reason, "own": true }),
                        (None, false, _) => json!({ "state": "failed", "error": explain_own_end(&s), "own": true }),
                        (None, true, true) => {
                            let (url, problem) = own_address(&s, r.local_port);
                            json!({ "state": "on", "own": true, "url": url, "problem": problem })
                        }
                        (None, true, false) => {
                            json!({ "state": "connecting", "own": true, "slow": elapsed >= SLOW_SECS })
                        }
                    }
                } else {
                    match (&s.ended, alive, s.connected, &s.url) {
                        (Some(reason), _, _, _) => json!({ "state": "failed", "error": reason }),
                        (None, false, _, _) => json!({ "state": "failed", "error": explain_end(&s) }),
                        (None, true, true, Some(url)) => json!({ "state": "on", "url": url }),
                        (None, true, _, url) => {
                            json!({ "state": "connecting", "url": url, "slow": elapsed >= SLOW_SECS })
                        }
                    }
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

/// Reads the routes of the owner's tunnel from cloudflared's "Updated to new configuration"
/// line: each public address and where Cloudflare sends its visitors.
fn find_routes(line: &str) -> Option<Vec<(String, String)>> {
    let start = line.find("config=\"")? + "config=\"".len();
    let rest = line.get(start..)?;
    // The JSON is quoted, with its own quotes escaped; it ends at the last quote on the line.
    let end = rest.rfind('"')?;
    let text = rest.get(..end)?.replace("\\\"", "\"");
    let config: Value = serde_json::from_str(&text).ok()?;
    let routes = config
        .get("ingress")?
        .as_array()?
        .iter()
        .filter_map(|rule| {
            let host = rule.get("hostname")?.as_str()?;
            let service = rule.get("service")?.as_str()?;
            let ok = !host.is_empty()
                && host.len() <= 253
                && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '*');
            ok.then(|| (host.to_owned(), service.chars().take(200).collect()))
        })
        .collect();
    Some(routes)
}

/// Whether a tunnel route's destination is this computer on `port`.
fn points_here(service: &str, port: u16) -> bool {
    let rest = service.strip_prefix("http://").unwrap_or(service).trim_end_matches('/');
    ["localhost", "127.0.0.1", "[::1]"].iter().any(|host| rest == format!("{host}:{port}"))
}

/// With the owner's own tunnel connected: its address, or what's wrong with how it's set up
/// in Cloudflare. The address must lead to Porchly's public door (`port`), which keeps the
/// link read-only unless changes are allowed; any other destination would skip that.
fn own_address(s: &State, port: u16) -> (Option<String>, Option<String>) {
    if !s.routes_known {
        return (None, None);
    }
    if let Some((host, _)) = s.routes.iter().find(|(host, service)| !host.contains('*') && points_here(service, port)) {
        return (Some(format!("https://{host}")), None);
    }
    let problem = match s.routes.first() {
        Some((host, service)) => format!(
            "Your Cloudflare tunnel sends {host} to {service}, not to Porchly's public door. In Cloudflare, set the service of that address to http://localhost:{port}."
        ),
        None => format!(
            "Your Cloudflare tunnel has no public address yet. In Cloudflare, add a public hostname to the tunnel and set its service to http://localhost:{port}."
        ),
    };
    (None, Some(problem))
}

const REFUSED: &str = "Cloudflare didn't accept the token. The tunnel may have been deleted, or the token copied incompletely. Copy it again from your tunnel's page in Cloudflare and paste it into Porchly.";

/// Whether a line of cloudflared's output says Cloudflare refused the tunnel's token.
/// (Seen with cloudflared 2026.10: `ERR Register tunnel error from server side
/// error="Failed to get tunnel"`, repeated while it retries.)
fn token_refused(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.contains("token is not valid")
        || l.contains("invalid tunnel secret")
        || (l.contains("register tunnel error") && (l.contains("failed to get tunnel") || l.contains("unauthorized")))
}

/// Why the owner's own tunnel stopped, in plain words.
fn explain_own_end(s: &State) -> String {
    if s.recent.iter().any(|l| token_refused(l)) {
        REFUSED.into()
    } else if !s.connected {
        "Couldn't reach Cloudflare. Check that this computer is connected to the internet. Some networks (offices, hotels, schools) block this kind of link."
            .into()
    } else {
        "The public link stopped. Turn the porch light on again; if it keeps stopping, the log has details.".into()
    }
}

/// Finds a Cloudflare tunnel token in what someone pasted: the token itself, or the whole
/// command Cloudflare shows (`cloudflared service install eyJh…`). A token is base64 of a
/// small JSON object naming the account (`a`), the tunnel (`t`) and its secret (`s`).
pub fn clean_token(pasted: &str) -> Result<String> {
    let looks_right = |piece: &str| {
        let unpadded = piece.trim_end_matches('=');
        data_encoding::BASE64_NOPAD
            .decode(unpadded.as_bytes())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|v| ["a", "t", "s"].iter().all(|k| v.get(k).is_some_and(Value::is_string)))
    };
    pasted
        .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        .filter(|piece| (40..=2000).contains(&piece.len()))
        .rev()
        .find(|piece| looks_right(piece))
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::invalid(
                "That doesn't look like a Cloudflare tunnel token. In Cloudflare, open your tunnel, copy the token (a long line starting with eyJ), and paste it here.",
            )
        })
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
        assert!(light.start(Path::new("/nonexistent"), 1, None).is_err());
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
        light.start(&program, 1, None).unwrap();
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

    /// A token shaped like Cloudflare's (base64 of {"a","t","s"}), made up for tests.
    fn fake_token() -> String {
        data_encoding::BASE64.encode(br#"{"a":"0123456789abcdef0123456789abcdef","t":"11111111-2222-3333-4444-555555555555","s":"c2VjcmV0LXNlY3JldC1zZWNyZXQ="}"#)
    }

    #[test]
    fn tokens_are_found_in_what_people_paste() {
        let token = fake_token();
        assert_eq!(clean_token(&token).unwrap(), token);
        assert_eq!(clean_token(&format!("  {token}\n")).unwrap(), token);
        let command = format!("sudo cloudflared service install {token}");
        assert_eq!(clean_token(&command).unwrap(), token, "the whole command Cloudflare shows");
        assert_eq!(clean_token(&format!("cloudflared.exe tunnel run --token \"{token}\"")).unwrap(), token);
        for wrong in
            ["", "hello", "eyJhIjoiYiJ9", "pk_abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz", &"A".repeat(200)]
        {
            let err = clean_token(wrong).unwrap_err().to_string();
            assert!(err.contains("doesn't look like a Cloudflare tunnel token"), "{wrong:?}: {err}");
        }
    }

    const CONFIG_LINE: &str = r#"2026-10-11T08:00:01Z INF Updated to new configuration config="{\"ingress\":[{\"hostname\":\"api.example.com\",\"originRequest\":{},\"service\":\"http://localhost:11920\"},{\"originRequest\":{},\"service\":\"http_status:404\"}],\"warp-routing\":{\"enabled\":false}}" version=3"#;

    #[test]
    fn reads_the_address_from_the_tunnel_s_routes() {
        let routes = find_routes(CONFIG_LINE).unwrap();
        assert_eq!(routes, [("api.example.com".to_owned(), "http://localhost:11920".to_owned())]);
        assert!(find_routes("INF Registered tunnel connection").is_none());
        assert!(find_routes(r#"INF Updated to new configuration config="not json" version=1"#).is_none());

        assert!(points_here("http://localhost:11920", 11920) && points_here("http://127.0.0.1:11920/", 11920));
        assert!(!points_here("http://localhost:11911", 11920) && !points_here("http://192.168.1.5:11920", 11920));
        assert!(!points_here("https://localhost:11920", 11920) && !points_here("http://localhost:119200", 11920));

        let mut s = State { routes, routes_known: true, ..State::default() };
        assert_eq!(own_address(&s, 11920), (Some("https://api.example.com".to_owned()), None));
        // Pointing the address at the main API would skip the read-only public door: say so.
        let (url, problem) = own_address(&s, 11921);
        assert!(
            url.is_none() && problem.unwrap().contains("set the service of that address to http://localhost:11921")
        );
        s.routes.clear();
        assert!(own_address(&s, 11920).1.unwrap().contains("no public address yet"));
        assert_eq!(own_address(&State::default(), 11920), (None, None), "not told yet");
    }

    /// A stand-in for cloudflared with a token: says what the real one says, then keeps running.
    #[cfg(unix)]
    fn fake_cloudflared(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-cloudflared");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn wait_for(light: &PorchLight, state: &str) -> Value {
        for _ in 0..200 {
            let status = light.status();
            if status["state"] == state {
                return status;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("never reached {state}: {}", light.status());
    }

    #[cfg(unix)]
    #[test]
    fn your_own_tunnel_gives_a_stable_address_and_the_token_stays_secret() {
        let dir = tempfile::tempdir().unwrap();
        let light = PorchLight::new(Some(dir.path()));
        let token = fake_token();
        let script = format!(
            "echo \"INF Starting tunnel with token $TUNNEL_TOKEN args: $*\" >&2\n\
             echo '{}' >&2\n\
             echo 'INF Registered tunnel connection connIndex=0' >&2\n\
             sleep 30",
            CONFIG_LINE.replace('\'', "")
        );
        let program = fake_cloudflared(dir.path(), &script);
        light.start(&program, 11920, Some(&token)).unwrap();
        let status = wait_for(&light, "on");
        assert_eq!(status["url"], "https://api.example.com");
        assert_eq!((status["own"].as_bool(), status["problem"].is_null()), (Some(true), true));
        let log = std::fs::read_to_string(dir.path().join("logs/porch-light.log")).unwrap();
        assert!(log.contains("with token [token]"), "the program got the token: {log}");
        assert!(!log.contains(&token), "and it never reaches the log");
        assert!(log.contains("args: tunnel --no-autoupdate") && log.contains(" run"), "{log}");
        assert!(!log.contains("--url"), "your own tunnel's routes come from Cloudflare");
        assert!(!format!("{status}").contains(&token));
        light.stop();
        assert_eq!(light.status()["state"], "off");

        // Set up to send visitors somewhere else: on, but with a clear problem and no address.
        light.start(&program, 12345, Some(&token)).unwrap();
        let status = wait_for(&light, "on");
        assert!(status["url"].is_null());
        assert!(status["problem"].as_str().unwrap().contains("http://localhost:12345"));
        light.stop();

        // A token Cloudflare refuses: the real program keeps retrying and never exits.
        let refused = fake_cloudflared(
            dir.path(),
            "echo 'ERR Register tunnel error from server side error=\"Failed to get tunnel\" connIndex=0' >&2\nsleep 30",
        );
        light.start(&refused, 11920, Some(&token)).unwrap();
        let status = wait_for(&light, "failed");
        assert!(status["error"].as_str().unwrap().contains("didn't accept the token"), "{status}");
        light.stop();
        assert!(token_refused("ERR Provided Tunnel token is not valid."));
        assert!(
            !token_refused("ERR Failed to fetch features, default to disable"),
            "an ordinary hiccup isn't a refusal"
        );
    }
}
