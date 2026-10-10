//! Downloads the helper programs some features need (Bun for functions, cloudflared for the
//! porch light). Only ever after the person asked for the feature; every download is checked
//! against a SHA-256 pinned in Porchly's source before it is used.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use porchly_core::{Error, Result};
use sha2::{Digest, Sha256};

/// How far a download has got, for progress bars.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

impl Progress {
    pub fn get(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }
}

/// Downloads `url` to `to`, stopping if it grows past `size` bytes, then checks it against
/// `sha256` (lowercase hex). On any failure the partial file is removed. `what` names the
/// download in messages ("Bun").
pub fn download(url: &str, to: &Path, size: u64, sha256: &str, what: &str, progress: &Progress) -> Result<()> {
    progress.total.store(size, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let result = fetch(url, to, size, what, progress).and_then(|()| check(to, sha256, what));
    if result.is_err() {
        let _ = std::fs::remove_file(to);
    }
    result
}

fn fetch(url: &str, to: &Path, size: u64, what: &str, progress: &Progress) -> Result<()> {
    let offline = |e: &dyn std::fmt::Display| {
        Error::invalid(format!(
            "Couldn't download {what}. Check that this computer is connected to the internet, then try again. ({e})"
        ))
    };
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| write_error(dir, &e))?;
    }
    let response = ureq::get(url).call().map_err(|e| offline(&e))?;
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(to).map_err(|e| write_error(to, &e))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| offline(&e))?;
        if n == 0 {
            break;
        }
        done = done.saturating_add(n as u64);
        if done > size {
            return Err(Error::invalid(format!("The {what} download is bigger than expected, so Porchly stopped it.")));
        }
        file.write_all(buf.get(..n).unwrap_or_default()).map_err(|e| write_error(to, &e))?;
        progress.done.store(done, Ordering::Relaxed);
    }
    file.flush().map_err(|e| write_error(to, &e))
}

/// Checks a file against a pinned SHA-256.
pub fn check(path: &Path, sha256: &str, what: &str) -> Result<()> {
    let bytes = std::fs::read(path).map_err(|e| write_error(path, &e))?;
    if sha256_hex(&bytes) == sha256 {
        Ok(())
    } else {
        Err(Error::invalid(format!(
            "The downloaded {what} doesn't match the official release, so Porchly didn't use it. Try again later."
        )))
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(bytes))
}

/// Marks a file as a program that can run (Unix; nothing to do on Windows).
#[cfg(unix)]
pub fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| write_error(path, &e))
}

#[cfg(not(unix))]
pub fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

pub fn write_error(path: &Path, e: &std::io::Error) -> Error {
    Error::invalid(format!("Couldn't write to \"{}\": {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x");
        std::fs::write(&file, b"hello").unwrap();
        let good = sha256_hex(b"hello");
        assert_eq!(good, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        check(&file, &good, "Bun").unwrap();
        assert!(check(&file, &sha256_hex(b"other"), "Bun").unwrap_err().to_string().contains("doesn't match"));
    }

    #[test]
    fn unreachable_addresses_fail_politely() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("sub").join("x");
        // Port 9 on this computer: nothing listens, so this fails fast and offline.
        let err = download("http://127.0.0.1:9/x", &to, 10, &"0".repeat(64), "Bun", &Progress::default()).unwrap_err();
        assert!(err.to_string().contains("Couldn't download Bun"), "{err}");
        assert!(!to.exists(), "no partial file left");
    }
}
