//! Snapshots: full copies of the project taken before destructive changes, kept in
//! `<project>.snapshots/` next to the project file.

use std::path::PathBuf;

use porchly_core::{Error, Result, limits};
use serde::Serialize;

use crate::{DbResult, Store};

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotInfo {
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

impl Store {
    /// Copies the whole project into the snapshot folder. Returns `None` for in-memory
    /// projects. `reason` becomes part of the file name.
    pub fn snapshot(&self, reason: &str) -> Result<Option<PathBuf>> {
        let Some(dir) = self.snapshot_dir() else {
            return Ok(None);
        };
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Storage(format!("couldn't create the snapshot folder: {e}")))?;
        let stamp: String =
            self.conn.query_row("SELECT strftime('%Y-%m-%dT%H-%M-%f', 'now')", [], |r| r.get(0)).db()?;
        let reason: String = reason
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .take(80)
            .collect();
        let path = dir.join(format!("{stamp}-{reason}.db"));
        let target = path.to_str().ok_or_else(|| Error::Storage("snapshot path isn't valid text".into()))?;
        self.conn.execute("VACUUM INTO ?1", [target]).db()?;
        self.prune_snapshots()?;
        Ok(Some(path))
    }

    /// Snapshots, oldest first.
    pub fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>> {
        let Some(dir) = self.snapshot_dir() else {
            return Ok(Vec::new());
        };
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::Storage(format!("couldn't read the snapshot folder: {e}"))),
        };
        let mut out: Vec<SnapshotInfo> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "db"))
            .map(|e| SnapshotInfo {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path().display().to_string(),
                bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn prune_snapshots(&self) -> Result<()> {
        let all = self.list_snapshots()?;
        let excess = all.len().saturating_sub(limits::MAX_SNAPSHOTS);
        for old in all.iter().take(excess) {
            std::fs::remove_file(&old.path)
                .map_err(|e| Error::Storage(format!("couldn't remove an old snapshot: {e}")))?;
        }
        Ok(())
    }

    fn snapshot_dir(&self) -> Option<PathBuf> {
        let path = self.path.as_ref()?;
        let mut name = path.file_name()?.to_os_string();
        name.push(".snapshots");
        Some(path.with_file_name(name))
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;
    use crate::test_util::temp_store;
    use porchly_core::limits;

    #[test]
    fn snapshot_is_a_working_project() {
        let (_dir, s) = temp_store();
        let path = s.snapshot("test reason/../x").unwrap().unwrap();
        assert!(path.file_name().unwrap().to_str().unwrap().ends_with("-test-reason----x.db"));
        assert!(Store::open_existing(&path).is_ok());
    }

    #[test]
    fn keeps_newest_only() {
        let (_dir, s) = temp_store();
        for i in 0..limits::MAX_SNAPSHOTS + 3 {
            s.snapshot(&format!("n{i:03}")).unwrap();
        }
        assert_eq!(s.list_snapshots().unwrap().len(), limits::MAX_SNAPSHOTS);
    }

    #[test]
    fn in_memory_has_no_snapshots() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.snapshot("x").unwrap().is_none());
    }
}
