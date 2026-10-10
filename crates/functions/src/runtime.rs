//! Bun, the program that runs functions. Porchly doesn't ship it: turning Functions on
//! downloads the official release once, checks it against the checksum pinned below, and
//! keeps it in the project folder's `tools/` (no administrator rights, nothing installed).
//! Setting `PORCHLY_BUN` to a `bun` program uses that one instead.

use std::io::Write;
use std::path::{Path, PathBuf};

use porchly_core::{Error, Result};
pub use porchly_fetch::Progress;

/// The Bun release Porchly uses. Porchly releases move it forward; Porchly never updates
/// Bun on its own.
pub const BUN_VERSION: &str = "1.4.2";

/// One downloadable Bun build, with the SHA-256 of its zip from the release's SHASUMS256.txt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    pub name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

const ASSETS: &[Asset] = &[
    Asset {
        name: "bun-linux-x64",
        sha256: "36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913",
        size: 36_646_985,
    },
    Asset {
        name: "bun-linux-x64-baseline",
        sha256: "c678040f14fe0440eb839d37cbd0ce4c051a32da72806ac97de6a6aab6bf728f",
        size: 36_646_949,
    },
    Asset {
        name: "bun-linux-aarch64",
        sha256: "54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7",
        size: 36_602_920,
    },
    Asset {
        name: "bun-linux-x64-musl",
        sha256: "4835eca59d6da70f4674f5642f6e459dcadab773695b2ed9922d131057989742",
        size: 34_101_012,
    },
    Asset {
        name: "bun-linux-x64-musl-baseline",
        sha256: "76e1db84e98f22f78de0a87e309bfbbf297732847f9720db36750646c85c8c18",
        size: 34_100_976,
    },
    Asset {
        name: "bun-linux-aarch64-musl",
        sha256: "71760b6c8ea30623b81a4907cb815d48e2ea266f2e73e751534a44a0607950df",
        size: 33_975_245,
    },
    Asset {
        name: "bun-darwin-aarch64",
        sha256: "90987a3a16d7db556d886ac3d551e7b6d3edf0a1cf43acaed622e8676be1d12f",
        size: 25_377_591,
    },
    Asset {
        name: "bun-darwin-x64",
        sha256: "80520d7e17526308c9185d261679ac6d27798d3803a0e9f7ff9121ab8affb012",
        size: 28_440_507,
    },
    Asset {
        name: "bun-darwin-x64-baseline",
        sha256: "bad5bbd6cf14d0980d115f5954c9ff904df619d5e994d2da1ffccd3f316300b0",
        size: 28_440_543,
    },
    Asset {
        name: "bun-windows-x64",
        sha256: "ce4c17497b2f29712a99d3d53f028de28cd42e3bacb8589599e7f000e49b6405",
        size: 39_807_510,
    },
    Asset {
        name: "bun-windows-x64-baseline",
        sha256: "78c221c2376f79731ccf4e4af0b3bb46d81fefa3296c5abee09ad8a1b21e68c6",
        size: 39_807_490,
    },
    Asset {
        name: "bun-windows-aarch64",
        sha256: "a7a16b876a305fd1029c66dbd27007b4f6112ae896532f675878731a21e50cfd",
        size: 36_330_062,
    },
];

/// The build for this computer: its system, its processor, and for older x64 processors
/// (no AVX2) the "baseline" build, so old laptops work too.
pub fn asset_for_this_computer() -> Result<Asset> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        other => return Err(unsupported(other)),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        other => return Err(unsupported(other)),
    };
    let musl = if cfg!(target_env = "musl") { "-musl" } else { "" };
    let baseline = if arch == "x64" && !has_avx2() { "-baseline" } else { "" };
    let name = format!("bun-{os}-{arch}{musl}{baseline}");
    ASSETS.iter().find(|a| a.name == name).copied().ok_or_else(|| unsupported(&name))
}

#[cfg(target_arch = "x86_64")]
fn has_avx2() -> bool {
    std::arch::is_x86_feature_detected!("avx2")
}

#[cfg(not(target_arch = "x86_64"))]
fn has_avx2() -> bool {
    false
}

fn unsupported(what: &str) -> Error {
    Error::invalid(format!("Functions aren't available on this kind of computer yet ({what})."))
}

/// Where the download from the official release lives.
pub fn download_url(asset: &Asset) -> String {
    format!("https://github.com/oven-sh/bun/releases/download/bun-v{BUN_VERSION}/{}.zip", asset.name)
}

fn exe_name() -> &'static str {
    if cfg!(windows) { "bun.exe" } else { "bun" }
}

/// Where Porchly keeps its own copy of Bun.
pub fn bun_dir(tools: &Path) -> PathBuf {
    tools.join(format!("bun-{BUN_VERSION}"))
}

/// The Bun program to run: `PORCHLY_BUN` when set, otherwise Porchly's own copy if it has
/// been downloaded.
pub fn find(tools: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PORCHLY_BUN").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let own = bun_dir(tools).join(exe_name());
    own.is_file().then_some(own)
}

/// Whether `PORCHLY_BUN` chooses the program (then Porchly never downloads or removes it).
pub fn chosen_by_environment() -> bool {
    std::env::var_os("PORCHLY_BUN").is_some()
}

/// Downloads Bun for this computer into `tools`, checks it and unpacks it. Returns the
/// program's path. Blocks until done; run it off the UI thread.
pub fn download(tools: &Path, progress: &Progress) -> Result<PathBuf> {
    let asset = asset_for_this_computer()?;
    let zip_path = tools.join(format!("{}.zip.part", asset.name));
    let result = porchly_fetch::download(&download_url(&asset), &zip_path, asset.size, asset.sha256, "Bun", progress)
        .and_then(|()| install_zip(&zip_path, &asset, tools));
    let _ = std::fs::remove_file(&zip_path);
    result
}

/// Checks a downloaded zip against the pinned checksum, then unpacks the program into
/// [`bun_dir`]. The program only appears once it is complete.
pub fn install_zip(zip_path: &Path, asset: &Asset, tools: &Path) -> Result<PathBuf> {
    porchly_fetch::check(zip_path, asset.sha256, "Bun")?;
    let bytes = std::fs::read(zip_path).map_err(|e| folder_error(zip_path, &e))?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| Error::invalid(format!("The Bun download is damaged ({e}). Try again.")))?;
    let wanted = format!("{}/{}", asset.name, exe_name());
    let mut entry =
        archive.by_name(&wanted).map_err(|_| Error::invalid(format!("The Bun download doesn't contain {wanted}.")))?;
    let dir = bun_dir(tools);
    std::fs::create_dir_all(&dir).map_err(|e| folder_error(&dir, &e))?;
    let target = dir.join(exe_name());
    let part = dir.join(format!("{}.part", exe_name()));
    {
        let mut out = std::fs::File::create(&part).map_err(|e| folder_error(&part, &e))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| folder_error(&part, &e))?;
        out.flush().map_err(|e| folder_error(&part, &e))?;
    }
    porchly_fetch::make_executable(&part)?;
    std::fs::rename(&part, &target).map_err(|e| folder_error(&target, &e))?;
    Ok(target)
}

/// Removes Porchly's copy of Bun (not one chosen with `PORCHLY_BUN`).
pub fn remove(tools: &Path) -> Result<()> {
    let dir = bun_dir(tools);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| folder_error(&dir, &e))?;
    }
    Ok(())
}

fn folder_error(path: &Path, e: &std::io::Error) -> Error {
    porchly_fetch::write_error(path, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_zip(dir: &Path, asset_name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join("bun.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.add_directory(format!("{asset_name}/"), zip::write::SimpleFileOptions::default()).unwrap();
        zip.start_file(format!("{asset_name}/{}", exe_name()), zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(contents).unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn this_computer_has_a_build() {
        let asset = asset_for_this_computer().unwrap();
        assert!(asset.name.starts_with("bun-"));
        assert!(download_url(&asset).starts_with("https://github.com/oven-sh/bun/releases/download/bun-v"));
        assert_eq!(asset.sha256.len(), 64);
    }

    #[test]
    fn installs_only_a_matching_download() {
        let dir = tempfile::tempdir().unwrap();
        let zip = fake_zip(dir.path(), "bun-test", b"#!/bin/sh\necho 1.4.2\n");
        let sha = porchly_fetch::sha256_hex(&std::fs::read(&zip).unwrap());
        let sha: &'static str = Box::leak(sha.into_boxed_str());
        let tools = dir.path().join("tools");

        let wrong = Asset { name: "bun-test", sha256: ASSETS[0].sha256, size: 0 };
        assert!(install_zip(&zip, &wrong, &tools).unwrap_err().to_string().contains("doesn't match"));
        assert!(find(&tools).is_none() || chosen_by_environment());

        let right = Asset { name: "bun-test", sha256: sha, size: 0 };
        let exe = install_zip(&zip, &right, &tools).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"#!/bin/sh\necho 1.4.2\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o111, 0o111);
        }
        remove(&tools).unwrap();
        assert!(!bun_dir(&tools).exists());
    }

    #[test]
    fn a_zip_without_bun_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let zip = fake_zip(dir.path(), "something-else", b"x");
        let sha = porchly_fetch::sha256_hex(&std::fs::read(&zip).unwrap());
        let asset = Asset { name: "bun-test", sha256: Box::leak(sha.into_boxed_str()), size: 0 };
        assert!(install_zip(&zip, &asset, &dir.path().join("tools")).is_err());
    }
}
