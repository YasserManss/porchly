//! Each function's own output (`console.log`, errors) goes to `logs/functions/<name>.log`
//! next to the project. That's the function talking, not Porchly watching it. Files rotate
//! at 1 MB, keeping one older file.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Logs {
    dir: PathBuf,
}

impl Logs {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Logs { dir: dir.into() }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.log"))
    }

    /// Adds a line to a function's log. Never fails loudly: a log that can't be written
    /// mustn't break the call.
    pub fn write(&self, name: &str, stream: &str, line: &str) {
        let path = self.path(name);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
            let _ = std::fs::rename(&path, self.dir.join(format!("{name}.log.1")));
        }
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let line: String = line.chars().take(8_000).collect();
            let _ = writeln!(f, "{} {stream} {line}", now());
        }
    }

    /// The last `lines` lines of a function's log (oldest first).
    pub fn tail(&self, name: &str, lines: usize) -> Vec<String> {
        let mut out = VecDeque::with_capacity(lines.min(1000));
        for file in [self.dir.join(format!("{name}.log.1")), self.path(name)] {
            if let Ok(text) = std::fs::read_to_string(&file) {
                for line in text.lines() {
                    if out.len() == lines {
                        out.pop_front();
                    }
                    if lines > 0 {
                        out.push_back(line.to_owned());
                    }
                }
            }
        }
        out.into()
    }

    /// Removes a function's logs.
    pub fn clear(&self, name: &str) {
        let _ = std::fs::remove_file(self.path(name));
        let _ = std::fs::remove_file(self.dir.join(format!("{name}.log.1")));
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// The current UTC time as `2026-01-15T14:03:22Z`.
fn now() -> String {
    let secs =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_468), (2026, 1, 15));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn writes_tails_and_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let logs = Logs::new(dir.path().join("logs"));
        assert!(logs.tail("f", 10).is_empty());
        logs.write("f", "out", "one");
        logs.write("f", "err", "two");
        let tail = logs.tail("f", 10);
        assert_eq!(tail.len(), 2);
        assert!(tail[0].ends_with(" out one") && tail[1].ends_with(" err two"), "{tail:?}");
        assert_eq!(logs.tail("f", 1).len(), 1);

        std::fs::write(logs.path("f"), "x".repeat(MAX_LOG_BYTES as usize + 1)).unwrap();
        logs.write("f", "out", "fresh");
        assert!(std::fs::read_to_string(logs.path("f")).unwrap().ends_with(" out fresh\n"));
        assert!(dir.path().join("logs/f.log.1").is_file());
        logs.clear("f");
        assert!(logs.tail("f", 10).is_empty());
    }
}
