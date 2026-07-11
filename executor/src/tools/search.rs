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
    context_lines: Option<usize>,
}

struct MatchHit {
    path: String,
    line: usize,
    col: usize,
    line_content: String,
    context_before: Vec<(usize, String)>,
    context_after: Vec<(usize, String)>,
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
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 5,
                    "description": "Number of context lines before and after each match (0–5). Defaults to 0."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let required = ["pattern"];
        let present: Vec<&str> = args
            .as_object()
            .map(|m| {
                required
                    .iter()
                    .copied()
                    .filter(|k| m.contains_key(*k))
                    .collect()
            })
            .unwrap_or_default();
        let parsed = match serde_json::from_value::<SearchArgs>(args) {
            Ok(a) => a,
            Err(_) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(super::registry::missing_args_hint(
                        "search", &required, &present,
                    )),
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

        let ctx = parsed.context_lines.unwrap_or(0).min(5);

        let abs_root = search_root
            .canonicalize()
            .unwrap_or_else(|_| search_root.clone());

        if search_root.is_file() {
            return execute_single_file(&abs_root, &parsed.pattern, &re, max_results, ctx);
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

            let all_lines: Vec<&str> = content.lines().collect();
            for (line_idx, line) in all_lines.iter().enumerate() {
                for m in re.find_iter(line) {
                    let context_before = if ctx > 0 {
                        let before_start = line_idx.saturating_sub(ctx);
                        all_lines[before_start..line_idx]
                            .iter()
                            .enumerate()
                            .map(|(i, l)| (before_start + i + 1, l.to_string()))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let context_after = if ctx > 0 {
                        let after_end = (line_idx + 1 + ctx).min(all_lines.len());
                        all_lines[line_idx + 1..after_end]
                            .iter()
                            .enumerate()
                            .map(|(i, l)| (line_idx + 2 + i, l.to_string()))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    hits.push(MatchHit {
                        path: rel_path.clone(),
                        line: line_idx + 1,
                        col: m.start() + 1,
                        line_content: line.to_string(),
                        context_before,
                        context_after,
                    });
                    if hits.len() >= max_results {
                        let truncated = hits.len() == max_results;
                        return Ok(format_output(
                            &hits,
                            &abs_root,
                            &parsed.pattern,
                            truncated,
                            ctx,
                        ));
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
            Ok(format_output(&hits, &abs_root, &parsed.pattern, false, ctx))
        }
    }
}

fn execute_single_file(
    path: &std::path::PathBuf,
    pattern: &str,
    re: &regex::Regex,
    max_results: usize,
    ctx: usize,
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

    let all_lines: Vec<&str> = content.lines().collect();
    for (line_idx, line) in all_lines.iter().enumerate() {
        for m in re.find_iter(line) {
            let context_before = if ctx > 0 {
                let before_start = line_idx.saturating_sub(ctx);
                all_lines[before_start..line_idx]
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (before_start + i + 1, l.to_string()))
                    .collect()
            } else {
                Vec::new()
            };
            let context_after = if ctx > 0 {
                let after_end = (line_idx + 1 + ctx).min(all_lines.len());
                all_lines[line_idx + 1..after_end]
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (line_idx + 2 + i, l.to_string()))
                    .collect()
            } else {
                Vec::new()
            };
            hits.push(MatchHit {
                path: path_str.clone(),
                line: line_idx + 1,
                col: m.start() + 1,
                line_content: line.to_string(),
                context_before,
                context_after,
            });
            if hits.len() >= max_results {
                return Ok(format_output(&hits, path, pattern, true, ctx));
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
        Ok(format_output(&hits, path, pattern, false, ctx))
    }
}

fn format_output(
    hits: &[MatchHit],
    abs_root: &std::path::Path,
    pattern: &str,
    truncated: bool,
    ctx: usize,
) -> ToolResult {
    let file_set: std::collections::HashSet<&str> = hits.iter().map(|h| h.path.as_str()).collect();
    let file_count = file_set.len();

    let mut output = format!("✓ {} matches in {} files\n\n", hits.len(), file_count);

    let mut current_file = "";
    let mut last_after_end = 0usize;

    for hit in hits {
        if hit.path != current_file {
            if !current_file.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("{}:\n", hit.path));
            current_file = &hit.path;
            last_after_end = 0;
        }

        if ctx > 0 {
            let before_start = hit.context_before.first().map_or(hit.line, |t| t.0);
            // Emit separator if context windows don't overlap
            if last_after_end > 0 && before_start > last_after_end {
                output.push_str("---\n");
            }

            for (line_num, content) in &hit.context_before {
                output.push_str(&format!("   {:>4}  {}\n", line_num, content));
            }
            output.push_str(&format!(
                "> {:>4}:{}  {}\n",
                hit.line, hit.col, hit.line_content
            ));
            for (line_num, content) in &hit.context_after {
                output.push_str(&format!("   {:>4}  {}\n", line_num, content));
            }
            last_after_end = hit.context_after.last().map_or(hit.line, |t| t.0);
        } else {
            output.push_str(&format!(
                "  {}:{}  {}\n",
                hit.line, hit.col, hit.line_content
            ));
        }
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

    #[tokio::test]
    async fn context_lines_zero_output_matches_no_context() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("test.txt", "line one\nline two\nline three\n")],
        );

        let tool = search(make_scope(&dir));
        let result_zero = tool
            .execute(json!({
                "pattern": "two",
                "path": dir.path().to_string_lossy(),
                "context_lines": 0
            }))
            .await
            .unwrap();
        let result_absent = tool
            .execute(json!({
                "pattern": "two",
                "path": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result_zero.error.is_none());
        assert!(result_absent.error.is_none());
        assert_eq!(
            result_zero.output, result_absent.output,
            "context_lines: 0 and absent must produce identical output"
        );
    }

    #[tokio::test]
    async fn context_lines_emits_before_and_after() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[(
                "test.txt",
                "line one\nline two\nMATCH here\nline four\nline five\n",
            )],
        );

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "MATCH",
                "path": dir.path().to_string_lossy(),
                "context_lines": 2
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let out = result.output;
        assert!(out.contains("line one"), "context before: {out}");
        assert!(out.contains("line two"), "context before: {out}");
        assert!(out.contains("MATCH here"), "match line: {out}");
        assert!(out.contains("line four"), "context after: {out}");
        assert!(out.contains("line five"), "context after: {out}");
        assert!(out.contains('>'), "match line has > prefix: {out}");
    }

    #[tokio::test]
    async fn context_lines_capped_at_five() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("test.txt", "line one\nMATCH here\nline three\n")],
        );

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "MATCH",
                "path": dir.path().to_string_lossy(),
                "context_lines": 99
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        // Only 1 before and 1 after available (bounded by file edges)
        let out = result.output;
        assert!(out.contains("line one"), "context before: {out}");
        assert!(out.contains("line three"), "context after: {out}");
    }

    #[tokio::test]
    async fn context_lines_separator_between_nonadjacent_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[(
                "test.txt",
                "A1\nA2\nMATCH_A\nA4\nA5\nA6\nA7\nA8\nA9\nA10\nB1\nB2\nMATCH_B\nB4\nB5\n",
            )],
        );

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "MATCH",
                "path": dir.path().to_string_lossy(),
                "context_lines": 2
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let out = result.output;
        assert!(
            out.contains("---"),
            "separator between non-adjacent hits: {out}"
        );
    }

    #[tokio::test]
    async fn context_lines_no_separator_for_adjacent_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        write_files(
            dir.path(),
            &[("test.txt", "line one\nMATCH_A\nMATCH_B\nline four\n")],
        );

        let tool = search(make_scope(&dir));
        let result = tool
            .execute(json!({
                "pattern": "MATCH",
                "path": dir.path().to_string_lossy(),
                "context_lines": 2
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let out = result.output;
        assert!(
            !out.contains("---"),
            "no separator for adjacent hits whose context windows overlap: {out}"
        );
    }

    #[tokio::test]
    async fn missing_pattern_returns_recovery_hint() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = search(make_scope(&dir));
        let result = tool.execute(json!({})).await.unwrap();

        let err = result.error.as_ref().unwrap();
        assert!(err.contains("pattern"), "should name missing field: {err}");
        assert!(
            !err.contains("invalid arguments: missing field"),
            "no raw serde text: {err}"
        );
    }
}
