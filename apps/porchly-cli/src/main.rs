//! `porchly-cli`: serve a project's API and run commands from the terminal.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use porchly_engine::{Engine, registry};
use porchly_store::Store;
use serde_json::Value;

#[derive(Parser)]
#[command(name = "porchly-cli", version, about = "Porchly from the terminal: a database and API in one file.")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve the project's API (only on this computer unless --host says otherwise).
    Serve {
        /// The project file; created if it doesn't exist.
        #[arg(long)]
        project: PathBuf,
        /// Port to listen on. Without it, Porchly uses 11911, or the next free port if that's busy.
        #[arg(long)]
        port: Option<u16>,
        /// Address to listen on. The default only accepts connections from this computer.
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        host: IpAddr,
    },
    /// Run one or more commands, e.g. --cmd 'table.create={"name":"recipes"}'.
    Run {
        /// The project file; created if it doesn't exist.
        #[arg(long)]
        project: PathBuf,
        /// A command id, optionally followed by =JSON parameters. Repeat to run several in order.
        #[arg(long = "cmd", required = true)]
        cmds: Vec<String>,
    },
    /// List commands, optionally only those whose id contains FILTER.
    Commands { filter: Option<String> },
    /// Show a command's parameters.
    Describe { id: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Serve { project, port, host } => serve(&project, host, port),
        Cmd::Run { project, cmds } => run(&project, &cmds),
        Cmd::Commands { filter } => {
            list_commands(filter.as_deref());
            Ok(())
        }
        Cmd::Describe { id } => describe(&id),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn open_store(project: &Path) -> Result<Store, String> {
    let existed = project.exists();
    let store = Store::open(project).map_err(|e| e.to_string())?;
    if !existed {
        eprintln!("Created a new project at {}", project.display());
    }
    Ok(store)
}

fn serve(project: &Path, host: IpAddr, port: Option<u16>) -> Result<(), String> {
    tracing_subscriber::fmt().with_target(false).init();
    let store = open_store(project)?;
    let keys = store.count_active_keys().map_err(|e| e.to_string())?;
    if keys == 0 {
        eprintln!(
            "Note: this project has no keys yet, so nobody can use the API. Create one with:\n  porchly-cli run --project {} --cmd 'api.key.create={{\"name\":\"me\"}}'",
            project.display()
        );
    }
    if !host.is_loopback() {
        eprintln!(
            "Warning: listening on {host}, so other computers that can reach this one can use the API (a key is still required)."
        );
    }
    // Through the engine, so functions run here too and Settings apply.
    let engine = Engine::new(store.into_shared());
    let wanted = engine
        .execute("settings.get", &Value::Null)
        .ok()
        .and_then(|s| s.get("port").and_then(Value::as_u64))
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(porchly_api::DEFAULT_PORT);
    // An explicit --port must be exactly that port; the usual one moves on if it's busy.
    let listener = porchly_api::bind(host, port.unwrap_or(wanted), port.is_some()).map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    if port.is_none() && addr.port() != wanted {
        eprintln!("Port {wanted} is busy, so Porchly is using {} instead.", addr.port());
    }
    engine.api_started(&format!("http://{addr}"));
    let functions = engine.execute("functions.status", &Value::Null).map_err(|e| e.to_string())?;
    if functions.get("enabled").and_then(Value::as_bool) == Some(true) {
        let folder = functions.get("folder").and_then(Value::as_str).unwrap_or("functions/");
        eprintln!("Functions are on: /api/fn/<name> runs the code in {folder}");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("couldn't start the server: {e}"))?;
    runtime
        .block_on(porchly_api::serve_with(engine.store(), listener, engine.api_options(), async {
            let _ = tokio::signal::ctrl_c().await;
        }))
        .map_err(|e| format!("couldn't serve on {addr}: {e}"))
}

fn run(project: &Path, cmds: &[String]) -> Result<(), String> {
    let engine = Engine::new(open_store(project)?.into_shared());
    for spec in cmds {
        let (id, params) = match spec.split_once('=') {
            Some((id, json)) => {
                let params: Value =
                    serde_json::from_str(json).map_err(|e| format!("{id}: the parameters aren't valid JSON: {e}"))?;
                (id.trim(), params)
            }
            None => (spec.trim(), Value::Null),
        };
        let result = engine.execute(id, &params).map_err(|e| format!("{id}: {e}"))?;
        let text = serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?;
        println!("{text}");
    }
    Ok(())
}

fn list_commands(filter: Option<&str>) {
    for c in registry().iter().filter(|c| filter.is_none_or(|f| c.id.contains(f))) {
        println!("{:<18} {}", c.id, c.label);
    }
}

fn describe(id: &str) -> Result<(), String> {
    let c = registry().iter().find(|c| c.id == id).ok_or_else(|| format!("there's no command \"{id}\""))?;
    println!("{}  ({})", c.id, c.label);
    if !c.menu.is_empty() {
        println!("menu:     {}", c.menu);
    }
    if let Some(s) = c.shortcut {
        println!("shortcut: {s}");
    }
    println!("params:   {}", c.params);
    Ok(())
}
