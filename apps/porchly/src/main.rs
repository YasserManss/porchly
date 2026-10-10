//! Porchly, the desktop app: double-click it and you have a database and an API.

// No console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod log;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use porchly_engine::Engine;
use porchly_ui_egui::PorchlyApp;
use serde_json::json;

#[derive(Parser)]
#[command(name = "porchly", version, about = "A database, a live API and a public link, in one app.")]
struct Args {
    /// The project file to open (created if missing). Default: "Porchly data/porchly.db" next to
    /// the app (portable), or Porchly/porchly.db in your home folder if the app's folder can't be
    /// written to.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Add the sample recipes table when the project has no tables.
    #[arg(long)]
    sample: bool,
    /// Don't turn the API on when the app starts.
    #[arg(long)]
    no_api: bool,
    /// Let local tools drive the app over JSON lines on 127.0.0.1:<port> (see docs/control-protocol.md).
    /// The token is printed here; set PORCHLY_CONTROL_TOKEN to choose it.
    #[arg(long)]
    control: Option<u16>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let Some(path) = args.project.clone().or_else(default_project) else {
        return fail(
            "Porchly couldn't find a folder to keep your data in.",
            "Start it with --project <file> to choose where your data lives.",
        );
    };
    let folder = path.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_default();
    if let Err(e) = std::fs::create_dir_all(&folder) {
        return fail(&format!("Porchly couldn't create the folder {}.", folder.display()), &e.to_string());
    }
    let log_path = log::init(&folder.join("logs"));

    let engine = match Engine::open(&path) {
        Ok(e) => e,
        Err(e) => return fail(&format!("Porchly couldn't open {}.", path.display()), &e.to_string()),
    };
    tracing::info!("opened {}", path.display());
    if args.sample {
        let empty = engine
            .execute("table.list", &json!({}))
            .ok()
            .and_then(|v| v.as_array().map(Vec::is_empty))
            .unwrap_or(false);
        if empty {
            let _ = engine.execute("sample.load", &json!({}));
        }
    }
    let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Settings may say not to turn the API on at start.
    let start_api = engine
        .execute("settings.get", &json!({}))
        .ok()
        .and_then(|s| s.get("start_api").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    let mut app = PorchlyApp::new(engine, label);
    app.log_path = log_path.map(|p| p.display().to_string());
    if !args.no_api && start_api {
        app.start_api();
    }

    let control_port = args.control;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Porchly")
            .with_inner_size([1200.0, 760.0])
            .with_icon(std::sync::Arc::new(
                eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/brand/porchly-icon-256.png"))
                    .unwrap_or_default(),
            ))
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Porchly",
        options,
        Box::new(move |cc| {
            PorchlyApp::setup(&cc.egui_ctx);
            if let Some(port) = control_port {
                match app.start_control(port, &cc.egui_ctx) {
                    // The token goes to the terminal only, never to the log.
                    Ok((port, token)) => eprintln!("Control channel on 127.0.0.1:{port}  token: {token}"),
                    Err(e) => {
                        tracing::error!("control channel: {e}");
                        eprintln!("control channel: {e}");
                    }
                }
            }
            Ok(Box::new(Shell(app)))
        }),
    );
    match result {
        Ok(()) => {
            tracing::info!("Porchly closed");
            ExitCode::SUCCESS
        }
        Err(e) => fail(
            "Porchly couldn't open its window.",
            &format!("{e}\n\nThis usually means the graphics driver is missing or too old."),
        ),
    }
}

struct Shell(PorchlyApp);

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Files dropped on the window: CSV files start an import.
        for file in std::mem::take(&mut raw.dropped_files) {
            let path = file.path();
            if !path.as_os_str().is_empty() {
                self.0.file_dropped(path);
            }
        }
    }
}

/// Tells the person what went wrong in a window (there may be no terminal), logs it, and exits.
fn fail(what: &str, detail: &str) -> ExitCode {
    tracing::error!("{what} {detail}");
    eprintln!("{what}\n{detail}");
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Porchly")
        .set_description(format!("{what}\n\n{detail}"))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    ExitCode::FAILURE
}

/// `Porchly/porchly.db` in the person's home folder: no admin rights, easy to find and back up.
/// Where the data lives without `--project`. Porchly is portable: the data sits in a
/// "Porchly data" folder next to the app, so copying the two together (to a USB stick, another
/// computer) takes everything along. When that folder can't be written to (a read-only
/// location, Program Files, macOS running a freshly downloaded app from a hidden copy), the
/// home folder is used. Data from earlier versions in the home folder is kept using.
fn default_project() -> Option<PathBuf> {
    let home = home_project();
    let portable = portable_dir();
    if let Some(dir) = &portable
        && dir.join(PROJECT_FILE).is_file()
    {
        return Some(dir.join(PROJECT_FILE));
    }
    if let Some(h) = home.as_ref().filter(|h| h.is_file()) {
        return Some(h.clone());
    }
    match portable {
        Some(dir) if writable(&dir) => Some(dir.join(PROJECT_FILE)),
        _ => home,
    }
}

const PROJECT_FILE: &str = "porchly.db";

fn home_project() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join("Porchly").join(PROJECT_FILE))
}

/// "Porchly data" next to the app. On macOS that's next to Porchly.app, not inside it.
fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    Some(app_folder(&exe)?.join("Porchly data"))
}

fn app_folder(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let bundle = dir.parent().and_then(Path::parent);
    match bundle {
        Some(b) if dir.ends_with("Contents/MacOS") && b.extension().is_some_and(|e| e == "app") => {
            b.parent().map(Path::to_path_buf)
        }
        _ => Some(dir.to_path_buf()),
    }
}

/// Whether Porchly may keep data in `dir` (made if missing).
fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".porchly-write-test");
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_goes_next_to_the_app() {
        assert_eq!(app_folder(Path::new("/u/Downloads/porchly")), Some(PathBuf::from("/u/Downloads")));
        assert_eq!(
            app_folder(Path::new("/u/Apps/Porchly.app/Contents/MacOS/porchly")),
            Some(PathBuf::from("/u/Apps")),
            "next to the .app, not inside it"
        );
        assert_eq!(app_folder(Path::new("/x/Contents/MacOS/porchly")), Some(PathBuf::from("/x/Contents/MacOS")));
    }

    #[test]
    fn read_only_folders_are_noticed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(writable(&dir.path().join("Porchly data")));
        assert!(!dir.path().join("Porchly data/.porchly-write-test").exists(), "no probe left behind");
        let file = dir.path().join("a-file");
        std::fs::write(&file, "x").unwrap();
        assert!(!writable(&file.join("sub")), "can't make a folder inside a file");
    }
}
