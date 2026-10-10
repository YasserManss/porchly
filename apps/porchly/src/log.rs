//! The app's log file: `logs/porchly.log` next to the project, rotated so it never grows
//! without bound. It records what the server did and what went wrong, never keys or row data
//! (the API logs method, path, status and time only).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Size at which the log is rotated.
const MAX_BYTES: u64 = 1024 * 1024;
/// Old logs kept: porchly.log.1 … porchly.log.3.
const KEEP: u32 = 3;

struct Rotating {
    path: PathBuf,
    file: Option<File>,
    written: u64,
}

impl Rotating {
    fn open(path: PathBuf) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Rotating { path, file: Some(file), written })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        for i in (1..KEEP).rev() {
            let from = numbered(&self.path, i);
            if from.exists() {
                std::fs::rename(&from, numbered(&self.path, i + 1))?;
            }
        }
        std::fs::rename(&self.path, numbered(&self.path, 1))?;
        self.file = Some(OpenOptions::new().create(true).append(true).open(&self.path)?);
        self.written = 0;
        Ok(())
    }
}

impl Write for Rotating {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written + buf.len() as u64 > MAX_BYTES {
            // If rotation fails, keep writing to the current file rather than losing the line.
            let _ = self.rotate();
        }
        let Some(file) = self.file.as_mut() else { return Ok(buf.len()) };
        let n = file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), Write::flush)
    }
}

fn numbered(path: &Path, i: u32) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{i}"));
    PathBuf::from(name)
}

/// Starts logging to `dir/porchly.log` and records crashes there. Returns the log file's path,
/// or `None` if the folder can't be written (Porchly then runs without a log file).
pub fn init(dir: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join("porchly.log");
    let writer = Rotating::open(path.clone()).ok()?;
    tracing_subscriber::fmt().with_ansi(false).with_target(false).with_writer(Mutex::new(writer)).try_init().ok()?;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("Porchly hit a bug: {info}");
        default_hook(info);
    }));
    tracing::info!("Porchly {} started", env!("CARGO_PKG_VERSION"));
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_and_keeps_three() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("porchly.log");
        let mut log = Rotating::open(path.clone()).unwrap();
        let line = vec![b'x'; 100 * 1024];
        for _ in 0..60 {
            log.write_all(&line).unwrap();
        }
        assert!(std::fs::metadata(&path).unwrap().len() <= MAX_BYTES);
        for i in 1..=KEEP {
            assert!(numbered(&path, i).exists(), "porchly.log.{i}");
        }
        assert!(!numbered(&path, KEEP + 1).exists());
    }
}
