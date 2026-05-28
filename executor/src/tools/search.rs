// search: regex grep across the project, gitignore-aware.
//
// Backed by the `ignore` crate (same engine ripgrep uses). Returns matches
// with file:line:column context.
//
// Adapted from Rexy: uses Scope for path resolution instead of CWD.

use anyhow::Result;
use async_trait::async_trait;
use ignore::Walk;
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::security::scope::Scope;

use super::registry::{Tool, ToolResult};

#[derive(Deserialize)]
struct SearchArgs {
    pattern: String,
    path: Option<String>,
    max_results: Option<usize>,
    case_insensitive: Option<bool>,
}

struct MatchHit {
    path: String,
    line: usize,
    col: usize,
    line_content: String,
}

pub struct Search {
    scope: Scope,
}

#[async_trait]
impl Tool for Search {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Regex search across files. Gitignore-aware, backed by the same engine ripgrep uses."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern (Rust regex crate syntax)."
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search. Defaults to the project root. Confined to the project root."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Hard cap on total matches. Defaults to 100."
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "If true, regex is compiled case-insensitively. Defaults to false."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<SearchArgs>(args) {
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

        let re = match RegexBuilder::new(&parsed.pattern)
            .case_insensitive(parsed.case_insensitive.unwrap_or(false))
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid regex: {e}")),
                    metadata: None,
                });
            }
        };

        let abs_root = search_root
            .canonicalize()
            .unwrap_or_else(|_| search_root.clone());

        if search_root.is_file() {
            return execute_single_file(&abs_root, &parsed.pattern, &re, max_results);
        }

        let mut hits: Vec<MatchHit> = Vec::new();

        for entry in Walk::new(&abs_root) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }

            let content = match std::fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let rel_path = entry
                .path()
                .strip_prefix(&abs_root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .to_string();

            for (line_idx, line) in content.lines().enumerate() {
                for m in re.find_iter(line) {
                    hits.push(MatchHit {
                        path: rel_path.clone(),
                        line: line_idx + 1,
                        col: m.start() + 1,
                        line_content: line.to_string(),
                    });
                    if hits.len() >= max_results {
                        let truncated = hits.len() == max_results;
                        return Ok(format_output(&hits, &abs_root, &parsed.pattern, truncated));
                    }
                }
            }
        }

        if hits.is_empty() {
            Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "no matches for `{}` in {}",
                    parsed.pattern,
                    abs_root.display()
                )),
                metadata: None,
            })
        } else {
            Ok(format_output(&hits, &abs_root, &parsed.pattern, false))
        }
    }
}

fn execute_single_file(
    path: &std::path::PathBuf,
    pattern: &str,
    re: &regex::Regex,
    max_results: usize,
) -> Result<ToolResult> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!("no matches for `{pattern}` in {}", path.display())),
                metadata: None,
            });
        }
    };

    let mut hits: Vec<MatchHit> = Vec::new();
    let path_str = path.to_string_lossy().to_string();

    for (line_idx, line) in content.lines().enumerate() {
        for m in re.find_iter(line) {
            hits.push(MatchHit {
                path: path_str.clone(),
                line: line_idx + 1,
                col: m.start() + 1,
                line_content: line.to_string(),
            });
            if hits.len() >= max_results {
                return Ok(format_output(&hits, path, pattern, true));
            }
        }
    }

    if hits.is_empty() {
        Ok(ToolResult {
            output: String::new(),
            error: Some(format!("no matches for `{pattern}` in {}", path.display())),
            metadata: None,
        })
    } else {
        Ok(format_output(&hits, path, pattern, false))
    }
}

fn format_output(
    hits: &[MatchHit],
    abs_root: &std::path::Path,
    pattern: &str,
    truncated: bool,
) -> ToolResult {
    let file_set: std::collections::HashSet<&str> = hits.iter().map(|h| h.path.as_str()).collect();
    let file_count = file_set.len();

    let mut output = format!("✓ {} matches in {} files\n\n", hits.len(), file_count);

    let mut current_file = "";
    for hit in hits {
        if hit.path != current_file {
            if !current_file.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("{}:\n", hit.path));
            current_file = &hit.path;
        }
        output.push_str(&format!(
            "  {}:{}  {}\n",
            hit.line, hit.col, hit.line_content
        ));
    }

    if truncated {
        output.push_str(&format!(
            "\n[… truncated at {} matches; narrow your pattern or path to see more …]",
            hits.len()
        ));
    }

    let metadata = json!({
        "path": abs_root.to_string_lossy(),
        "pattern": pattern,
        "matches": hits.len(),
        "files": file_count,
        "truncated": truncated,
    });

    ToolResult {
        output,
        error: None,
        metadata: Some(metadata),
    }
}

pub fn search(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(Search { scope })
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
    async fn finds_single_match_in_one_file() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello world")]);

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "hello",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("✓ 1 matches in 1 files"));
    }

    #[tokio::test]
    async fn finds_matches_across_multiple_files() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("a.txt", "foo\nfoo\n"), ("b.txt", "foo\nfoo\n")],
        );

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "foo",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("✓ 4 matches in 2 files"));
        assert!(result.output.contains("a.txt"));
        assert!(result.output.contains("b.txt"));
    }

    #[tokio::test]
    async fn respects_gitignore() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[
                (".gitignore", "ignored.txt\n"),
                ("keep.txt", "secret here\n"),
                ("ignored.txt", "secret here\n"),
            ],
        );

        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "secret",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("keep.txt"));
        assert!(!result.output.contains("ignored.txt"));
    }

    #[tokio::test]
    async fn reports_no_matches_as_advisory() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "nothing here")]);

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "zzzznotfound",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("no matches for"));
        assert!(err.contains("zzzznotfound"));
    }

    #[tokio::test]
    async fn rejects_invalid_regex() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello")]);

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "[invalid",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("invalid regex"));
    }

    #[tokio::test]
    async fn rejects_missing_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "foo",
                "path": "/nonexistent/path/xyz"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("escapes"));
    }

    #[tokio::test]
    async fn truncates_at_max_results() {
        let dir = tempfile::TempDir::new().unwrap();
        let content = (1..=10)
            .map(|i| format!("match line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        write_files(dir.path(), &[("test.txt", &content)]);

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "match",
                "path": dir.path().to_string_lossy(),
                "max_results": 3
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("truncated at 3 matches"));
        let meta = result.metadata.unwrap();
        assert!(meta["truncated"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn case_insensitive_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "Hello World")]);

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "hello",
                "path": dir.path().to_string_lossy(),
                "case_insensitive": true
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("✓ 1 matches in 1 files"));
    }

    #[tokio::test]
    async fn reports_line_and_column_in_output() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello world")]);

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "world",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("1:7"));
    }

    #[tokio::test]
    async fn rejects_empty_pattern() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello")]);

        let tool = search(make_scope(&dir));
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
    async fn searches_single_file_when_path_is_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello world\nsecond line\n").unwrap();

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "hello",
                "path": file.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("✓ 1 matches in 1 files"));
    }

    #[tokio::test]
    async fn defaults_to_scope_root_when_no_path_given() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(dir.path(), &[("test.txt", "hello world")]);

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "hello"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("✓ 1 matches in 1 files"));
    }
}
