//! Every file under `assets/` must have a row in ATTRIBUTION.md, and every asset the table
//! names must exist.

use std::path::{Path, PathBuf};

use crate::Result;

pub(crate) fn check(root: &Path) -> Result {
    let table =
        std::fs::read_to_string(root.join("ATTRIBUTION.md")).map_err(|e| format!("can't read ATTRIBUTION.md: {e}"))?;
    // Only table rows count: the first cell of each `| `path` | … |` line.
    let listed: Vec<&str> = table
        .lines()
        .filter(|l| l.starts_with('|'))
        .filter_map(|l| l.split('`').nth(1))
        .filter(|s| s.starts_with("assets/"))
        .collect();

    let mut files = Vec::new();
    let dir = root.join("assets");
    if dir.is_dir() {
        walk(&dir, &mut files)?;
    }
    let mut problems = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(root).unwrap_or(f).to_string_lossy().replace('\\', "/");
        if !listed.contains(&rel.as_str()) {
            problems.push(format!("{rel} has no row in ATTRIBUTION.md"));
        }
    }
    for l in &listed {
        if !root.join(l).is_file() {
            problems.push(format!("ATTRIBUTION.md lists {l}, which doesn't exist"));
        }
    }
    if problems.is_empty() {
        println!("assets: ok ({} files)", files.len());
        Ok(())
    } else {
        Err(problems.join("\n  "))
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("can't read {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.file_name().is_some_and(|n| n != ".gitkeep") {
            out.push(path);
        }
    }
    Ok(())
}
