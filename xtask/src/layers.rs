//! Enforces the crate layering in CLAUDE.md: a crate depends only on crates in the same or a
//! lower layer, nothing below the UI layer pulls in GUI libraries, and only the storage
//! crates talk to databases directly.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::Result;

const LAYERS: &[(&str, u8)] = &[
    ("porchly-core", 0),
    ("porchly-schema", 1),
    ("porchly-store", 1),
    ("porchly-fetch", 1),
    ("porchly-api", 2),
    ("porchly-pipeline", 2),
    ("porchly-automate", 2),
    ("porchly-plugin-host", 3),
    ("porchly-connectors", 3),
    ("porchly-tunnel", 3),
    ("porchly-functions", 3),
    ("porchly-engine", 4),
    ("porchly-ui-egui", 5),
    ("porchly-mcp", 5),
    ("porchly-control", 5),
    ("porchly", 6),
    ("porchly-cli", 6),
];

/// Layer where GUI libraries become allowed.
const UI_LAYER: u8 = 5;
const GUI_CRATES: &[&str] = &["egui", "eframe", "egui_extras", "egui-winit", "egui-wgpu", "winit", "rfd"];

/// Database drivers, and the crates allowed to use them.
const DB_CRATES: &[&str] = &["rusqlite", "sqlx", "postgres", "tokio-postgres", "mysql", "mysql_async"];
const DB_OWNERS: &[&str] = &["porchly-store", "porchly-connectors"];

pub(crate) fn check(root: &Path) -> Result {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("couldn't run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("bad cargo metadata: {e}"))?;
    let packages = meta.get("packages").and_then(Value::as_array).ok_or("cargo metadata has no packages")?;

    let mut problems = Vec::new();
    for pkg in packages {
        let name = pkg.get("name").and_then(Value::as_str).unwrap_or_default();
        if name == "xtask" {
            continue;
        }
        let Some(layer) = layer_of(name) else {
            problems.push(format!("{name}: not in the layer table (xtask/src/layers.rs)"));
            continue;
        };
        let deps = pkg.get("dependencies").and_then(Value::as_array).cloned().unwrap_or_default();
        for dep in deps {
            let dep_name = dep.get("name").and_then(Value::as_str).unwrap_or_default();
            if let Some(dep_layer) = layer_of(dep_name)
                && dep_layer > layer
            {
                problems.push(format!("{name} (L{layer}) depends on {dep_name} (L{dep_layer}), a higher layer"));
            }
            if layer < UI_LAYER && GUI_CRATES.contains(&dep_name) {
                problems.push(format!("{name} (L{layer}) depends on GUI crate {dep_name}; only L{UI_LAYER}+ may"));
            }
            if DB_CRATES.contains(&dep_name) && !DB_OWNERS.contains(&name) {
                problems
                    .push(format!("{name} depends on database driver {dep_name}; only {} may", DB_OWNERS.join(", ")));
            }
        }
    }
    if problems.is_empty() {
        println!("layers: ok");
        Ok(())
    } else {
        Err(problems.join("\n  "))
    }
}

fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}
