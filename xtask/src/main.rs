//! Project tasks: `cargo xtask <task>`.
//!
//! - `ci`        everything below plus fmt, clippy (-D warnings) and tests; run before every commit
//! - `layers`    crate layering and dependency rules
//! - `assets`    every asset file has a row in ATTRIBUTION.md
//! - `wit-check` the plugin interface (from M2.1)
//! - `plugins`   build the plugins in `plugins/` (from M2.5)

mod assets;
mod layers;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

type Result<T = ()> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let root = root();
    let result = match task.as_str() {
        "ci" => ci(&root),
        "layers" => layers::check(&root),
        "assets" => assets::check(&root),
        "wit-check" => wit_check(&root),
        "plugins" => plugins(&root),
        _ => {
            eprintln!("usage: cargo xtask <ci|layers|assets|wit-check|plugins>");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask {task}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn ci(root: &Path) -> Result {
    step("fmt", || cargo(root, &["fmt", "--all", "--", "--check"]))?;
    step("clippy", || cargo(root, &["clippy", "--workspace", "--all-targets", "--quiet", "--", "-D", "warnings"]))?;
    step("test", || cargo(root, &["test", "--workspace", "--quiet"]))?;
    step("assets", || assets::check(root))?;
    step("layers", || layers::check(root))?;
    step("wit-check", || wit_check(root))?;
    step("plugins", || plugins(root))?;
    println!("ci: all green");
    Ok(())
}

fn step(name: &str, f: impl FnOnce() -> Result) -> Result {
    println!("== {name}");
    f().map_err(|e| format!("{name} failed: {e}"))
}

fn wit_check(root: &Path) -> Result {
    if !root.join("wit").is_dir() {
        println!("skipped: no wit/ yet (arrives in M2.1)");
        return Ok(());
    }
    Err("wit/ exists but the WIT compatibility check isn't written yet; add it with M2.1".into())
}

fn plugins(root: &Path) -> Result {
    if !root.join("plugins").is_dir() {
        println!("skipped: no plugins/ yet (arrives in M2.5)");
        return Ok(());
    }
    Err("plugins/ exists but the plugin build isn't written yet; add it with M2.5".into())
}

pub(crate) fn cargo(root: &Path, args: &[&str]) -> Result {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status =
        Command::new(cargo).args(args).current_dir(root).status().map_err(|e| format!("couldn't run cargo: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("`cargo {}` failed", args.join(" "))) }
}

fn root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(Path::to_path_buf).unwrap_or(manifest)
}
