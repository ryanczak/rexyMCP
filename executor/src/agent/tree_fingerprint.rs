//! Fingerprint of the project root's working tree: every non-ignored file by
//! relative path, size and mtime. Comparing two fingerprints yields the
//! relative paths that were added, removed or changed between them — the
//! loop's signal that a `bash` call made file progress.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ignore::WalkBuilder;

/// One file's identity in the fingerprint: size and mtime are the content
/// proxies (`touch` counts as a change; accepted by design).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: SystemTime,
}

/// A snapshot of the project root's non-ignored files.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TreeFingerprint {
    entries: HashMap<PathBuf, FileStamp>,
}

/// Best-effort walk of `root`: every non-ignored file's relative path → size +
/// mtime. An entry that cannot be read (or stat'd) is skipped; the walk never
/// returns an error.
///
/// Walk rules, each one a gotcha:
/// - gitignore files are honoured even when `root` is **not** a git repo
///   (`require_git(false)` — `ignore` skips gitignore files outside a repo by
///   default, and tests run in a bare `TempDir`),
/// - hidden files count (`.github/workflows/ci.yml` is a real edit), so
///   `hidden(false)`,
/// - `.git/` is pruned during the walk via `filter_entry` (returning `false`
///   skips the entry *and* its descent), keeping the walk cheap,
/// - the root's `.rexymcp/` is pruned too, whatever the ignore rules say: the
///   loop writes its session log there (and bash spills output there) on
///   every call, so in a repo that does not gitignore it every `bash` call
///   would otherwise read as a tree change and the stall detector would
///   never fire on bash.
pub fn fingerprint(root: &Path) -> TreeFingerprint {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| {
            entry.file_name() != ".git" && !(entry.depth() == 1 && entry.file_name() == ".rexymcp")
        });
    let mut entries = HashMap::new();
    for entry in builder.build() {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        entries.insert(
            rel.to_path_buf(),
            FileStamp {
                size: meta.len(),
                modified,
            },
        );
    }
    TreeFingerprint { entries }
}

impl TreeFingerprint {
    /// Relative paths (sorted) that were added, removed or changed between
    /// `self` (before) and `after`: present in exactly one side, or present in
    /// both with a different size or mtime.
    pub fn changed_paths(&self, after: &TreeFingerprint) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for (path, stamp) in &self.entries {
            match after.entries.get(path) {
                None => out.push(path.clone()),
                Some(other) if other != stamp => out.push(path.clone()),
                Some(_) => {}
            }
        }
        for path in after.entries.keys() {
            if !self.entries.contains_key(path) {
                out.push(path.clone());
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_written_file() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(fingerprint(dir.path()).entries.is_empty());
        fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let before = fingerprint(dir.path());
        fs::write(dir.path().join("new.txt"), "a much longer rewrite").unwrap();
        let after = fingerprint(dir.path());
        assert_eq!(
            before.changed_paths(&after),
            vec![PathBuf::from("new.txt")],
            "a new file and a rewrite to a different size both appear as changes"
        );
    }

    #[test]
    fn detects_deleted_file() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("gone.txt"), "bye").unwrap();
        let before = fingerprint(dir.path());
        fs::remove_file(dir.path().join("gone.txt")).unwrap();
        let after = fingerprint(dir.path());
        assert_eq!(
            before.changed_paths(&after),
            vec![PathBuf::from("gone.txt")]
        );
    }

    #[test]
    fn unchanged_tree_compares_equal() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/b.txt"), "b").unwrap();
        let before = fingerprint(dir.path());
        let after = fingerprint(dir.path());
        assert!(
            before.changed_paths(&after).is_empty(),
            "two walks with no write between them yield no changed paths"
        );
    }

    #[test]
    fn gitignored_write_is_not_a_change() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        let before = fingerprint(dir.path());
        fs::write(dir.path().join("target/out.bin"), "junk").unwrap();
        let after = fingerprint(dir.path());
        assert!(
            before.changed_paths(&after).is_empty(),
            "a write under an ignored path is not a change, even with no .git directory"
        );
    }

    #[test]
    fn dot_git_write_is_not_a_change() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        let before = fingerprint(dir.path());
        fs::write(dir.path().join(".git/objects/ab"), "junk").unwrap();
        let after = fingerprint(dir.path());
        assert!(
            before.changed_paths(&after).is_empty(),
            "a write under .git/ never counts"
        );
    }

    #[test]
    fn rexymcp_state_write_is_not_a_change() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".rexymcp/sessions")).unwrap();
        let before = fingerprint(dir.path());
        fs::write(dir.path().join(".rexymcp/sessions/s.jsonl"), "{}").unwrap();
        let after = fingerprint(dir.path());
        assert!(
            before.changed_paths(&after).is_empty(),
            "the loop's own state under .rexymcp/ never counts, even when not gitignored"
        );
    }

    #[test]
    fn nested_rexymcp_dir_is_a_change() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("fixtures/.rexymcp")).unwrap();
        let before = fingerprint(dir.path());
        fs::write(dir.path().join("fixtures/.rexymcp/f.txt"), "x").unwrap();
        let after = fingerprint(dir.path());
        assert_eq!(
            before.changed_paths(&after),
            vec![PathBuf::from("fixtures/.rexymcp/f.txt")],
            "only the root's .rexymcp/ is pruned"
        );
    }

    #[test]
    fn hidden_file_write_is_a_change() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".github")).unwrap();
        let before = fingerprint(dir.path());
        fs::write(dir.path().join(".github/ci.yml"), "on: push").unwrap();
        let after = fingerprint(dir.path());
        assert_eq!(
            before.changed_paths(&after),
            vec![PathBuf::from(".github/ci.yml")],
            "an edit to a hidden file is progress"
        );
    }
}
