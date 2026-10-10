//! `cloudflared`, Cloudflare's program that opens the tunnel. Porchly doesn't ship it:
//! turning the porch light on the first time downloads the official release once, checks it
//! against the checksum pinned below, and keeps it in `tools/` next to the project.
//! Setting `PORCHLY_CLOUDFLARED` to a `cloudflared` program uses that one instead.

use std::io::Write;
use std::path::{Path, PathBuf};

use porchly_core::{Error, Result};
use porchly_fetch::Progress;

/// The cloudflared release Porchly uses. Porchly never updates it on its own
/// (`--no-autoupdate`); Porchly releases move it forward.
pub const VERSION: &str = "2026.10.0";

/// One downloadable build: a plain program, or a `.tgz` holding one (macOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    pub name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

const ASSETS: &[Asset] = &[
    Asset {
        name: "cloudflared-linux-amd64",
        sha256: "d33ff2d14475178d2012c2c56beba87389ac5ded27649519f198a7d3134a99db",
        size: 40_129_756,
    },
    Asset {
        name: "cloudflared-linux-arm64",
        sha256: "e6422b9d4f72d3194bc5a38676f13667c06666523217b842a877d72a80b5ac08",
        size: 37_687_584,
    },
    Asset {
        name: "cloudflared-darwin-amd64.tgz",
        sha256: "903845b81828c8cb3c5d13d816a2de71c06a3da5785469df8eb0e1b736d92f9f",
        size: 21_741_581,
    },
    Asset {
        name: "cloudflared-darwin-arm64.tgz",
        sha256: "a2f79ff7b9420aa537d74af239f376da170bbabeb529aec416002adac6a72e70",
        size: 19_809_074,
    },
    // Also used on Windows on ARM, which runs x64 programs.
    Asset {
        name: "cloudflared-windows-amd64.exe",
        sha256: "86aee4017b26625cee8484c113558f48effa4cd47f7aa05fcf425604e5d2b23c",
        size: 55_365_048,
    },
];

/// The build for this computer.
pub fn asset_for_this_computer() -> Result<Asset> {
    let name = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "cloudflared-linux-amd64",
        ("linux", "aarch64") => "cloudflared-linux-arm64",
        ("macos", "x86_64") => "cloudflared-darwin-amd64.tgz",
        ("macos", "aarch64") => "cloudflared-darwin-arm64.tgz",
        ("windows", "x86_64" | "aarch64") => "cloudflared-windows-amd64.exe",
        (os, arch) => {
            return Err(Error::invalid(format!(
                "The porch light isn't available on this kind of computer yet ({os} {arch})."
            )));
        }
    };
    ASSETS.iter().find(|a| a.name == name).copied().ok_or_else(|| Error::Internal(format!("no asset {name}")))
}

pub fn download_url(asset: &Asset) -> String {
    format!("https://github.com/cloudflare/cloudflared/releases/download/{VERSION}/{}", asset.name)
}

fn exe_name() -> &'static str {
    if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" }
}

/// Where Porchly keeps its own copy.
pub fn dir(tools: &Path) -> PathBuf {
    tools.join(format!("cloudflared-{VERSION}"))
}

/// The program to run: `PORCHLY_CLOUDFLARED` when set, otherwise Porchly's own copy if it
/// has been downloaded.
pub fn find(tools: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PORCHLY_CLOUDFLARED").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let own = dir(tools).join(exe_name());
    own.is_file().then_some(own)
}

pub fn chosen_by_environment() -> bool {
    std::env::var_os("PORCHLY_CLOUDFLARED").is_some()
}

/// Downloads cloudflared for this computer into `tools`, checks it and puts it in place.
/// Blocks; run it off the UI thread.
pub fn download(tools: &Path, progress: &Progress) -> Result<PathBuf> {
    let asset = asset_for_this_computer()?;
    let part = tools.join(format!("{}.part", asset.name));
    let result =
        porchly_fetch::download(&download_url(&asset), &part, asset.size, asset.sha256, "cloudflared", progress)
            .and_then(|()| install(&part, &asset, tools));
    let _ = std::fs::remove_file(&part);
    result
}

/// Puts a checked download in place: the program itself, or the one inside a `.tgz`.
pub fn install(download: &Path, asset: &Asset, tools: &Path) -> Result<PathBuf> {
    porchly_fetch::check(download, asset.sha256, "cloudflared")?;
    let dir = dir(tools);
    std::fs::create_dir_all(&dir).map_err(|e| porchly_fetch::write_error(&dir, &e))?;
    let target = dir.join(exe_name());
    let part = dir.join(format!("{}.part", exe_name()));
    if asset.name.ends_with(".tgz") {
        let file = std::fs::File::open(download).map_err(|e| porchly_fetch::write_error(download, &e))?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
        let damaged = |e: &dyn std::fmt::Display| {
            Error::invalid(format!("The cloudflared download is damaged ({e}). Try again."))
        };
        let mut found = false;
        for entry in archive.entries().map_err(|e| damaged(&e))? {
            let mut entry = entry.map_err(|e| damaged(&e))?;
            let is_program = entry.path().ok().and_then(|p| p.file_name().map(|n| n == "cloudflared")).unwrap_or(false);
            if is_program && entry.header().entry_type().is_file() {
                let mut out = std::fs::File::create(&part).map_err(|e| porchly_fetch::write_error(&part, &e))?;
                std::io::copy(&mut entry, &mut out).map_err(|e| porchly_fetch::write_error(&part, &e))?;
                out.flush().map_err(|e| porchly_fetch::write_error(&part, &e))?;
                found = true;
                break;
            }
        }
        if !found {
            return Err(Error::invalid("The cloudflared download doesn't contain the program."));
        }
    } else {
        std::fs::copy(download, &part).map_err(|e| porchly_fetch::write_error(&part, &e))?;
    }
    porchly_fetch::make_executable(&part)?;
    std::fs::rename(&part, &target).map_err(|e| porchly_fetch::write_error(&target, &e))?;
    Ok(target)
}

/// Removes Porchly's copy.
pub fn remove(tools: &Path) -> Result<()> {
    let dir = dir(tools);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| porchly_fetch::write_error(&dir, &e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_computer_has_a_build() {
        let asset = asset_for_this_computer().unwrap();
        assert!(download_url(&asset).starts_with("https://github.com/cloudflare/cloudflared/releases/download/"));
        assert_eq!(asset.sha256.len(), 64);
    }

    #[test]
    fn installs_plain_programs_and_tgz() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");

        let plain = dir.path().join("plain");
        std::fs::write(&plain, b"program").unwrap();
        let sha: &'static str = Box::leak(porchly_fetch::sha256_hex(b"program").into_boxed_str());
        let wrong = Asset { name: "cloudflared-linux-amd64", sha256: ASSETS[1].sha256, size: 7 };
        assert!(install(&plain, &wrong, &tools).is_err());
        let exe = install(&plain, &Asset { name: "cloudflared-linux-amd64", sha256: sha, size: 7 }, &tools).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"program");
        remove(&tools).unwrap();

        // A .tgz like the macOS release: the program is inside.
        let tgz = dir.path().join("c.tgz");
        {
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&tgz).unwrap(), flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, "cloudflared", &b"macos"[..]).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let sha: &'static str = Box::leak(porchly_fetch::sha256_hex(&std::fs::read(&tgz).unwrap()).into_boxed_str());
        let exe = install(&tgz, &Asset { name: "cloudflared-darwin-arm64.tgz", sha256: sha, size: 0 }, &tools).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"macos");
    }
}
