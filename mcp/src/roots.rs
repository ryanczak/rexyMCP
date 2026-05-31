use std::path::{Path, PathBuf};

/// Result of the corroboration check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Corroboration {
    /// At least one source matched. Names the winning source for the log.
    Matched(MatchedSource),
    /// Sources existed but none matched. The handler turns this into an Err.
    Mismatch {
        repo_path: PathBuf,
        roots: Vec<String>,
        project_dir: Option<PathBuf>,
    },
    /// No sources to check (no roots, no env var). Pass-through; the
    /// handler logs and proceeds.
    NoSources,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchedSource {
    Root { uri: String },
    ProjectDir(PathBuf),
}

/// Pure corroboration. `roots` are raw URIs (`file:///foo/bar`) as advertised
/// by the client; `project_dir` is `CLAUDE_PROJECT_DIR` already read by the
/// caller (None when unset/empty).
pub fn corroborate(
    repo_path: &Path,
    roots: &[String],
    project_dir: Option<&Path>,
) -> Corroboration {
    let has_sources = !roots.is_empty() || project_dir.is_some();

    if !has_sources {
        return Corroboration::NoSources;
    }

    let canonical_repo = match std::fs::canonicalize(repo_path) {
        Ok(p) => p,
        Err(_) => {
            // Nonexistent repo_path is misconfiguration.
            return Corroboration::Mismatch {
                repo_path: repo_path.to_path_buf(),
                roots: roots.to_vec(),
                project_dir: project_dir.map(PathBuf::from),
            };
        }
    };

    for uri in roots {
        if !uri.starts_with("file://") {
            continue;
        }
        let path_str = &uri[7..];
        let root_path = PathBuf::from(path_str);
        let canonical_root = match std::fs::canonicalize(&root_path) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if canonical_repo == canonical_root || canonical_repo.starts_with(&canonical_root) {
            return Corroboration::Matched(MatchedSource::Root { uri: uri.clone() });
        }
    }

    if let Some(pd) = project_dir {
        let canonical_pd = match std::fs::canonicalize(pd) {
            Ok(p) => p,
            Err(_) => {
                return Corroboration::Mismatch {
                    repo_path: repo_path.to_path_buf(),
                    roots: roots.to_vec(),
                    project_dir: Some(pd.to_path_buf()),
                };
            }
        };
        if canonical_repo == canonical_pd || canonical_repo.starts_with(&canonical_pd) {
            return Corroboration::Matched(MatchedSource::ProjectDir(canonical_pd));
        }
    }

    Corroboration::Mismatch {
        repo_path: repo_path.to_path_buf(),
        roots: roots.to_vec(),
        project_dir: project_dir.map(PathBuf::from),
    }
}

/// Format a Mismatch into the error string returned by the tool handler.
pub fn format_mismatch_error(
    repo_path: &Path,
    roots: &[String],
    project_dir: Option<&Path>,
) -> String {
    let roots_str = if roots.is_empty() {
        "none advertised".to_string()
    } else {
        format!("[{}]", roots.join(", "))
    };

    let pd_str = match project_dir {
        Some(p) => p.display().to_string(),
        None => "(unset)".to_string(),
    };

    format!(
        "repo_path {} does not corroborate against any MCP root or CLAUDE_PROJECT_DIR.\n\
         Inspected roots: {}   (or \"none advertised\")\n\
         CLAUDE_PROJECT_DIR: {}\n\
         This usually means the architect passed the wrong repo_path, or the MCP\
         client roots / CLAUDE_PROJECT_DIR are misconfigured. Fix one of those and\
         re-dispatch.",
        repo_path.display(),
        roots_str,
        pd_str,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    fn make_dir(temp: &TempDir, name: &str) -> PathBuf {
        let p = temp.path().join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn repo_path_equals_root() {
        let temp = TempDir::new().unwrap();
        let d = make_dir(&temp, "repo");
        let uri = format!("file://{}", d.display());
        let result = corroborate(&d, &[uri], None);
        assert!(matches!(
            result,
            Corroboration::Matched(MatchedSource::Root { .. })
        ));
    }

    #[test]
    fn repo_path_descendant_of_root() {
        let temp = TempDir::new().unwrap();
        let root = make_dir(&temp, "root");
        let child = make_dir(&temp, "root/nested/repo");
        let uri = format!("file://{}", root.display());
        let result = corroborate(&child, &[uri], None);
        assert!(matches!(
            result,
            Corroboration::Matched(MatchedSource::Root { .. })
        ));
    }

    #[test]
    fn repo_path_equals_project_dir() {
        let temp = TempDir::new().unwrap();
        let d = make_dir(&temp, "project");
        let result = corroborate(&d, &[], Some(&d));
        assert!(matches!(
            result,
            Corroboration::Matched(MatchedSource::ProjectDir(_))
        ));
    }

    #[test]
    fn repo_path_descendant_of_project_dir() {
        let temp = TempDir::new().unwrap();
        let pd = make_dir(&temp, "project");
        let child = make_dir(&temp, "project/sub/repo");
        let result = corroborate(&child, &[], Some(&pd));
        assert!(matches!(
            result,
            Corroboration::Matched(MatchedSource::ProjectDir(_))
        ));
    }

    #[test]
    fn root_matches_before_project_dir() {
        let temp = TempDir::new().unwrap();
        let root = make_dir(&temp, "root");
        let pd = make_dir(&temp, "project");
        let child = make_dir(&temp, "root/sub");
        let uri = format!("file://{}", root.display());
        let result = corroborate(&child, &[uri], Some(&pd));
        assert!(matches!(
            result,
            Corroboration::Matched(MatchedSource::Root { .. })
        ));
    }

    #[test]
    fn first_root_matches_when_multiple() {
        let temp = TempDir::new().unwrap();
        let root1 = make_dir(&temp, "root1");
        let root2 = make_dir(&temp, "root2");
        let child = make_dir(&temp, "root1/sub");
        let uri1 = format!("file://{}", root1.display());
        let uri2 = format!("file://{}", root2.display());
        let result = corroborate(&child, &[uri1.clone(), uri2], None);
        let Corroboration::Matched(MatchedSource::Root { uri }) = result else {
            panic!("expected Matched(Root), got {:?}", result);
        };
        assert_eq!(uri, uri1);
    }

    #[test]
    fn no_sources_returns_no_sources() {
        let result = corroborate(Path::new("/any"), &[], None);
        assert!(matches!(result, Corroboration::NoSources));
    }

    #[test]
    fn mismatch_when_sources_exist_but_none_match() {
        let temp = TempDir::new().unwrap();
        let repo = make_dir(&temp, "repo");
        let other = make_dir(&temp, "other");
        let pd = make_dir(&temp, "project");
        let uri = format!("file://{}", other.display());
        let result = corroborate(&repo, &[uri], Some(&pd));
        assert!(matches!(result, Corroboration::Mismatch { .. }));
    }

    #[test]
    fn file_prefix_stripped() {
        let temp = TempDir::new().unwrap();
        let d = make_dir(&temp, "repo");
        let uri = format!("file://{}", d.display());
        let result = corroborate(&d, &[uri], None);
        assert!(matches!(
            result,
            Corroboration::Matched(MatchedSource::Root { .. })
        ));
    }

    #[test]
    fn non_file_uri_skipped() {
        let temp = TempDir::new().unwrap();
        let d = make_dir(&temp, "repo");
        let result = corroborate(&d, &["http://example.com/foo".to_string()], None);
        assert!(matches!(result, Corroboration::Mismatch { .. }));
    }

    #[test]
    fn url_encoded_does_not_match_unencoded() {
        let temp = TempDir::new().unwrap();
        let d = make_dir(&temp, "repo");
        let uri = "file:///foo%20bar/baz".to_string();
        let result = corroborate(&d, &[uri], None);
        assert!(matches!(result, Corroboration::Mismatch { .. }));
    }

    #[test]
    fn nonexistent_repo_path_returns_mismatch() {
        let result = corroborate(
            Path::new("/nonexistent/path/xyz"),
            &["file:///tmp".to_string()],
            None,
        );
        assert!(matches!(result, Corroboration::Mismatch { .. }));
    }

    #[test]
    fn uncanonicalizable_root_skipped_others_still_checked() {
        let temp = TempDir::new().unwrap();
        let good = make_dir(&temp, "good");
        let child = make_dir(&temp, "good/sub");
        let bad_uri = "file:///nonexistent/bad/path".to_string();
        let good_uri = format!("file://{}", good.display());
        let result = corroborate(&child, &[bad_uri, good_uri], None);
        assert!(matches!(
            result,
            Corroboration::Matched(MatchedSource::Root { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_repo_path_matches_via_canonicalization() {
        let temp = TempDir::new().unwrap();
        let real = make_dir(&temp, "real");
        let link = temp.path().join("link");
        symlink(&real, &link).unwrap();
        let uri = format!("file://{}", real.display());
        let result = corroborate(&link, &[uri], None);
        assert!(matches!(
            result,
            Corroboration::Matched(MatchedSource::Root { .. })
        ));
    }

    #[test]
    fn format_mismatch_error_includes_fix_hint() {
        let err = format_mismatch_error(
            Path::new("/wrong/repo"),
            &["file:///other".to_string()],
            Some(Path::new("/other/project")),
        );
        assert!(err.contains("does not corroborate"));
        assert!(err.contains("[file:///other]"));
        assert!(err.contains("/other/project"));
        assert!(err.contains("Fix one of those and"));
    }

    #[test]
    fn format_mismatch_error_absent_sources() {
        let err = format_mismatch_error(Path::new("/repo"), &[], None);
        assert!(err.contains("none advertised"));
        assert!(err.contains("(unset)"));
    }

    #[test]
    fn format_mismatch_error_lists_each_root_uri() {
        let err = format_mismatch_error(
            Path::new("/repo"),
            &["file:///a".to_string(), "file:///b".to_string()],
            None,
        );
        assert!(err.contains("file:///a"));
        assert!(err.contains("file:///b"));
    }
}
