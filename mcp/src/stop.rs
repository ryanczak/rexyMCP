use std::path::{Path, PathBuf};

/// The stop sentinel for a repo: `<repo>/.rexymcp/stop`. Its mere presence means
/// "stop all runs in this repo" (global stop-all; no run-id payload in this phase).
pub fn sentinel_path(repo: &Path) -> PathBuf {
    repo.join(".rexymcp").join("stop")
}

/// Write the sentinel (creating `.rexymcp/` if needed). Content is a human note;
/// only *presence* is load-bearing.
pub fn write_sentinel(repo: &Path) -> std::io::Result<PathBuf> {
    let path = sentinel_path(repo);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, b"stop requested\n")?;
    Ok(path)
}

/// True iff the sentinel exists.
pub fn sentinel_present(repo: &Path) -> bool {
    sentinel_path(repo).exists()
}

/// Remove the sentinel; a missing file is success (idempotent — several watchers
/// may race to clear it).
pub fn clear_sentinel(repo: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(sentinel_path(repo)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_then_present_then_clear_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();

        assert!(!sentinel_present(repo), "should start absent");

        write_sentinel(repo).unwrap();
        assert!(sentinel_present(repo), "should be present after write");

        clear_sentinel(repo).unwrap();
        assert!(!sentinel_present(repo), "should be absent after clear");

        // Second clear is still Ok (idempotent)
        clear_sentinel(repo).unwrap();
        assert!(!sentinel_present(repo), "still absent after second clear");
    }

    #[test]
    fn sentinel_path_is_under_dot_rexymcp() {
        let tmp = TempDir::new().unwrap();
        let p = sentinel_path(tmp.path());
        assert!(
            p.ends_with(".rexymcp/stop") || p.ends_with(".rexymcp\\stop"),
            "sentinel path should end with .rexymcp/stop, got: {}",
            p.display()
        );
    }
}
