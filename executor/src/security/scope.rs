// Filesystem scope confinement.
//
// Resolves every requested path to canonical absolute form and checks
// that it stays within the configured target-repo root. Catches `..`
// traversal, absolute paths outside the root, and symlink escapes.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Scope {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeError {
    Escapes { requested: String },
    BadRoot { reason: String },
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopeError::Escapes { requested } => {
                write!(f, "path escapes the project root: {requested}")
            }
            ScopeError::BadRoot { reason } => write!(f, "bad root: {reason}"),
        }
    }
}

impl std::error::Error for ScopeError {}

impl Scope {
    pub fn new(root: &Path) -> Result<Self, ScopeError> {
        if !root.exists() {
            return Err(ScopeError::BadRoot {
                reason: format!("root does not exist: {}", root.display()),
            });
        }
        if !root.is_dir() {
            return Err(ScopeError::BadRoot {
                reason: format!("root is not a directory: {}", root.display()),
            });
        }
        let root = root.canonicalize().map_err(|e| ScopeError::BadRoot {
            reason: format!("cannot canonicalize root: {e}"),
        })?;
        Ok(Self { root })
    }

    pub fn resolve(&self, requested: &str) -> Result<PathBuf, ScopeError> {
        let candidate = if Path::new(requested).is_absolute() {
            PathBuf::from(requested)
        } else {
            self.root.join(requested)
        };

        let confined = confine_to_root(&candidate, &self.root)?;
        Ok(confined)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn confine_to_root(candidate: &Path, root: &Path) -> Result<PathBuf, ScopeError> {
    // Canonicalize the nearest existing ancestor, then append remaining components.
    let (existing_part, remaining) = split_existing_and_remaining(candidate);

    let canonical_base = existing_part
        .canonicalize()
        .map_err(|e| ScopeError::BadRoot {
            reason: format!("cannot canonicalize {}: {e}", existing_part.display()),
        })?;

    if !canonical_base.starts_with(root) {
        return Err(ScopeError::Escapes {
            requested: candidate.to_string_lossy().into_owned(),
        });
    }

    let result = remaining.iter().fold(canonical_base, |acc, c| acc.join(c));
    Ok(result)
}

fn split_existing_and_remaining(path: &Path) -> (PathBuf, PathBuf) {
    let mut current = path.to_path_buf();
    let mut remaining_components: Vec<std::ffi::OsString> = Vec::new();

    while !current.exists() {
        if let Some(parent) = current.parent() {
            if let Some(file_name) = current.file_name() {
                remaining_components.push(file_name.to_os_string());
                current = parent.to_path_buf();
            } else {
                break;
            }
        } else {
            break;
        }
    }

    remaining_components.reverse();
    let remaining: PathBuf = remaining_components.iter().collect();
    (current, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolves_in_root_relative_path() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();

        let scope = Scope::new(dir.path()).unwrap();
        let resolved = scope.resolve("file.txt").unwrap();
        assert!(resolved.starts_with(&scope.root));
        assert_eq!(resolved.file_name().unwrap(), "file.txt");
    }

    #[test]
    fn resolves_in_root_absolute_path() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();

        let scope = Scope::new(dir.path()).unwrap();
        let abs = scope.root().join("file.txt");
        let resolved = scope.resolve(&abs.to_string_lossy()).unwrap();
        assert!(resolved.starts_with(&scope.root));
    }

    #[test]
    fn rejects_dot_dot_escape() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let result = scope.resolve("../escape");
        assert!(matches!(result, Err(ScopeError::Escapes { .. })));
    }

    #[test]
    fn rejects_absolute_path_outside_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let result = scope.resolve("/etc/passwd");
        assert!(matches!(result, Err(ScopeError::Escapes { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();

        let link_path = dir.path().join("escape_link");
        symlink(outside.path(), &link_path).unwrap();

        let scope = Scope::new(dir.path()).unwrap();
        let result = scope.resolve("escape_link/secret.txt");
        assert!(matches!(result, Err(ScopeError::Escapes { .. })));
    }

    #[test]
    fn resolves_nonexistent_leaf_under_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let resolved = scope.resolve("future/file.txt").unwrap();
        assert!(resolved.starts_with(&scope.root));
        assert!(resolved.ends_with("future/file.txt"));
    }

    #[test]
    fn new_on_missing_dir_returns_bad_root() {
        let result = Scope::new(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(matches!(result, Err(ScopeError::BadRoot { .. })));
    }
}
