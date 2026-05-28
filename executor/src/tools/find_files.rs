// find_files: glob-based file search using the `globset` crate.
// Gitignore-aware via `ignore::Walk`.
//
// Adapted from Rexy: uses Scope for path resolution instead of CWD.

use anyhow::Result;
use async_trait::async_trait;
use globset::Glob;
use ignore::Walk;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::security::scope::Scope;

use super::registry::{Tool, ToolResult};

#[derive(Deserialize)]
struct FindFilesArgs {
    pattern: String,
    path: Option<String>,
    max_results: Option<usize>,
}

pub struct FindFiles {
    scope: Scope,
}

#[async_trait]
impl Tool for FindFiles {
    fn name(&self) -> &str {
        "find_files"
    }

    fn description(&self) -> &str {
        "Glob-based file search. Gitignore-aware, finds files matching a pattern."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (**/*.rs, Cargo.toml, etc.)."
                },
                "path": {
                    "type": "string",
                    "description": "Directory root to search. Defaults to the project root. Confined to the project root."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Hard cap on returned paths. Defaults to 100."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<FindFilesArgs>(args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid arguments: {e}")),
                    metadata: None,
                });
            }
        };

        if parsed.pattern.is_empty() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("pattern must not be empty".to_string()),
                metadata: None,
            });
        }

        let max_results = parsed.max_results.unwrap_or(100);

        if max_results < 1 {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("max_results must be >= 1".to_string()),
                metadata: None,
            });
        }

        let search_root = match self.scope.resolve(parsed.path.as_deref().unwrap_or(".")) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(e.to_string()),
                    metadata: None,
                });
            }
        };

        if !search_root.exists() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "search root does not exist: {}",
                    parsed.path.as_deref().unwrap_or(".")
                )),
                metadata: None,
            });
        }

        if !search_root.is_dir() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "path is not a directory: {}",
                    parsed.path.as_deref().unwrap_or(".")
                )),
                metadata: None,
            });
        }

        let matcher = match Glob::new(&parsed.pattern).map(|g| g.compile_matcher()) {
            Ok(m) => m,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid glob pattern: {e}")),
                    metadata: None,
                });
            }
        };

        let abs_root = search_root
            .canonicalize()
            .unwrap_or_else(|_| search_root.clone());
        let mut matched: Vec<String> = Vec::new();

        for entry in Walk::new(&abs_root) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }

            let rel_path = entry.path().strip_prefix(&abs_root).unwrap_or(entry.path());

            if matcher.is_match(rel_path) {
                matched.push(rel_path.to_string_lossy().to_string());
                if matched.len() >= max_results {
                    return Ok(format_output(&matched, &abs_root, &parsed.pattern, true));
                }
            }
        }

        if matched.is_empty() {
            Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "no files match `{}` in {}",
                    parsed.pattern,
                    abs_root.display()
                )),
                metadata: None,
            })
        } else {
            Ok(format_output(&matched, &abs_root, &parsed.pattern, false))
        }
    }
}

fn format_output(
    matched: &[String],
    abs_root: &std::path::Path,
    pattern: &str,
    truncated: bool,
) -> ToolResult {
    let mut output = format!("✓ {} files match `{pattern}`\n\n", matched.len());

    for path in matched {
        output.push_str(&format!("{path}\n"));
    }

    if truncated {
        output.push_str(&format!(
            "\n[… truncated at {} files; narrow your pattern to see more …]",
            matched.len()
        ));
    }

    let metadata = json!({
        "path": abs_root.to_string_lossy(),
        "pattern": pattern,
        "files": matched.len(),
        "truncated": truncated,
    });

    ToolResult {
        output,
        error: None,
        metadata: Some(metadata),
    }
}

pub fn find_files(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(FindFiles { scope })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scope(dir: &tempfile::TempDir) -> Scope {
        Scope::new(dir.path()).unwrap()
    }

    fn write_files(dir: &std::path::Path, files: &[(&str, &str)]) {
        for (path, content) in files {
            let full = dir.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(full, content).unwrap();
        }
    }

    #[tokio::test]
    async fn finds_files_by_glob() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("a.rs", "// a"), ("b.rs", "// b"), ("c.toml", "[package]")],
        );

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "*.rs",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("a.rs"));
        assert!(result.output.contains("b.rs"));
        assert!(!result.output.contains("c.toml"));
    }

    #[tokio::test]
    async fn finds_files_recursively() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[
                ("src/main.rs", "fn main() {}"),
                ("src/lib.rs", "pub fn lib() {}"),
                ("tests/it.rs", "#[test] fn it() {}"),
            ],
        );

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "**/*.rs",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("src/main.rs"));
        assert!(result.output.contains("src/lib.rs"));
        assert!(result.output.contains("tests/it.rs"));
    }

    #[tokio::test]
    async fn respects_gitignore_for_finds() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[
                (".gitignore", "target/\n"),
                ("src/main.rs", "fn main() {}"),
                ("target/debug/x", "binary"),
            ],
        );

        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "**/*",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("src/main.rs"));
        assert!(!result.output.contains("target/debug/x"));
    }

    #[tokio::test]
    async fn reports_no_matches_as_advisory_finds() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello")]);

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "*.does_not_exist",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("no files match"));
    }

    #[tokio::test]
    async fn rejects_invalid_glob() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello")]);

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "[unterminated",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("invalid glob"));
    }

    #[tokio::test]
    async fn rejects_path_that_is_a_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello").unwrap();

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "*.txt",
                "path": file.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("not a directory"));
    }

    #[tokio::test]
    async fn caps_results_at_max_results() {
        let dir = tempfile::TempDir::new().unwrap();
        let files: Vec<_> = (1..=10)
            .map(|i| (format!("file_{i:02}.rs"), "// content"))
            .collect();
        let files: Vec<_> = files.iter().map(|(n, c)| (n.as_str(), *c)).collect();
        write_files(dir.path(), &files);

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "*.rs",
                "path": dir.path().to_string_lossy(),
                "max_results": 3
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("truncated at 3 files"));
        let meta = result.metadata.unwrap();
        assert!(meta["truncated"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn rejects_empty_pattern() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello")]);

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("pattern must not be empty")
        );
    }

    #[tokio::test]
    async fn rejects_path_outside_root() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello")]);

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "*.txt",
                "path": "../outside"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("escapes"));
    }

    #[tokio::test]
    async fn defaults_to_scope_root_when_no_path_given() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("a.rs", "// a")]);

        let tool = find_files(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "*.rs"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("a.rs"));
    }
}
