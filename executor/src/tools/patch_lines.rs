// patch_lines: replace a line range in a file.
//
// Adapted from Rexy: uses Scope for path resolution instead of CWD.

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::security::scope::Scope;

use super::registry::{Tool, ToolResult};

#[derive(Deserialize)]
struct PatchLinesArgs {
    path: String,
    start_line: usize,
    end_line: usize,
    new_content: String,
}

pub struct PatchLines {
    scope: Scope,
}

#[async_trait]
impl Tool for PatchLines {
    fn name(&self) -> &str {
        "patch_lines"
    }

    fn description(&self) -> &str {
        "Replace a line range in a file. start_line and end_line are 1-indexed and inclusive. Use for large block replacements where patch old_str matching is fragile."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to patch, confined to the project root."
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-indexed start line (inclusive)."
                },
                "end_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-indexed end line (inclusive). Must be >= start_line."
                },
                "new_content": {
                    "type": "string",
                    "description": "Replacement content. Empty string deletes the line range."
                }
            },
            "required": ["path", "start_line", "end_line", "new_content"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let required = ["path", "start_line", "end_line", "new_content"];
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
        let parsed = match serde_json::from_value::<PatchLinesArgs>(args) {
            Ok(a) => a,
            Err(_) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(super::registry::missing_args_hint(
                        "patch_lines",
                        &required,
                        &present,
                    )),
                    metadata: None,
                });
            }
        };

        let path = match self.scope.resolve(&parsed.path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(e.to_string()),
                    metadata: None,
                });
            }
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("failed to read {}: {e}", parsed.path)),
                    metadata: None,
                });
            }
        };

        let lines: Vec<&str> = content.lines().collect();

        if parsed.start_line < 1 {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("start_line must be >= 1".to_string()),
                metadata: None,
            });
        }

        if parsed.end_line < parsed.start_line {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "end_line ({}) must be >= start_line ({})",
                    parsed.end_line, parsed.start_line
                )),
                metadata: None,
            });
        }

        if parsed.end_line > lines.len() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "end_line ({}) exceeds file length ({} lines)",
                    parsed.end_line,
                    lines.len()
                )),
                metadata: None,
            });
        }

        let n_old = parsed.end_line - parsed.start_line + 1;
        let new_lines: Vec<&str> = parsed.new_content.lines().collect();
        let n_new = new_lines.len();

        let mut result_lines: Vec<&str> = lines[..parsed.start_line - 1].to_vec();
        result_lines.extend(new_lines.iter().copied());
        result_lines.extend_from_slice(&lines[parsed.end_line..]);

        let mut new_content_str = result_lines.join("\n");
        if content.ends_with('\n') {
            new_content_str.push('\n');
        }

        match std::fs::write(&path, &new_content_str) {
            Ok(()) => {}
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("failed to write {}: {e}", parsed.path)),
                    metadata: None,
                });
            }
        }

        let abs_path = path.canonicalize().unwrap_or_else(|_| path.clone());

        let metadata = json!({
            "path": abs_path.to_string_lossy(),
            "start_line": parsed.start_line,
            "end_line": parsed.end_line,
            "lines_removed": n_old,
            "lines_added": n_new,
        });

        Ok(ToolResult {
            output: format!(
                "patched {}:{}-{} ({} lines → {} lines)",
                abs_path.to_string_lossy(),
                parsed.start_line,
                parsed.end_line,
                n_old,
                n_new
            ),
            error: None,
            metadata: Some(metadata),
        })
    }
}

pub fn patch_lines(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(PatchLines { scope })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scope(dir: &tempfile::TempDir) -> Scope {
        Scope::new(dir.path()).unwrap()
    }

    #[tokio::test]
    async fn replaces_middle_lines() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line 1\nline 2\nline 3\nline 4\nline 5\n").unwrap();

        let tool = patch_lines(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 2,
                "end_line": 3,
                "new_content": "replacement A\nreplacement B"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "line 1\nreplacement A\nreplacement B\nline 4\nline 5\n"
        );
    }

    #[tokio::test]
    async fn empty_new_content_deletes_range() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line 1\nline 2\nline 3\nline 4\nline 5\n").unwrap();

        let tool = patch_lines(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 2,
                "end_line": 3,
                "new_content": ""
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "line 1\nline 4\nline 5\n");
    }

    #[tokio::test]
    async fn preserves_trailing_newline() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line 1\nline 2\nline 3\n").unwrap();

        let tool = patch_lines(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 2,
                "end_line": 2,
                "new_content": "replacement"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.ends_with('\n'),
            "trailing newline preserved: {content:?}"
        );
    }

    #[tokio::test]
    async fn rejects_out_of_range() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line 1\nline 2\nline 3\nline 4\nline 5\n").unwrap();

        let tool = patch_lines(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 1,
                "end_line": 10,
                "new_content": "x"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result.error.as_ref().unwrap().contains("exceeds"),
            "error: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn rejects_end_before_start() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line 1\nline 2\nline 3\n").unwrap();

        let tool = patch_lines(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 3,
                "end_line": 2,
                "new_content": "x"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("must be >= start_line"),
            "error: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn missing_fields_returns_recovery_hint() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = patch_lines(make_scope(&dir));
        let result = tool.execute(json!({ "path": "x" })).await.unwrap();

        let err = result.error.as_ref().unwrap();
        assert!(
            err.contains("start_line"),
            "should name missing field: {err}"
        );
        assert!(err.contains("end_line"), "should name missing field: {err}");
        assert!(
            err.contains("new_content"),
            "should name missing field: {err}"
        );
        assert!(err.contains("path"), "should echo supplied field: {err}");
        assert!(
            !err.contains("invalid arguments: missing field"),
            "no raw serde text: {err}"
        );
    }

    #[tokio::test]
    async fn non_object_args_do_not_panic() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = patch_lines(make_scope(&dir));
        let result = tool.execute(json!(5)).await.unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(
            !err.contains("invalid arguments: missing field"),
            "no raw serde text: {err}"
        );
    }
}
