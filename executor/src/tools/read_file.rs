// read_file: read a file's contents, optionally a line range.
//
// Adapted from Rexy: uses Scope for path resolution instead of CWD.

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::security::scope::Scope;

use super::registry::{Tool, ToolResult};

/// Maximum lines returned in a single read_file call. Callers needing more must
/// use start_line/end_line to read in slices. Keeps output well below the
/// governor's `runaway_output_bytes` config (default 100 KB) for typical source
/// files.
const MAX_OUTPUT_LINES: usize = 500;

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
    start_line: Option<usize>,
    end_line: Option<usize>,
}

pub struct ReadFile {
    scope: Scope,
}

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file's contents. Returns at most 500 lines; use start_line/end_line to \
         read specific ranges of larger files."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to read. Absolute or relative to the project root."
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-indexed inclusive start line for partial read."
                },
                "end_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-indexed inclusive end line."
                },
                "force": {
                    "type": "boolean",
                    "description": "Set true to force a full re-read even if the file is unchanged since your last read this session (bypasses redundant-read dedupe)."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<ReadFileArgs>(args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid arguments: {e}")),
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

        if !path.exists() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!("file not found: {}", parsed.path)),
                metadata: None,
            });
        }

        if path.is_dir() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!("path is a directory: {}", parsed.path)),
                metadata: None,
            });
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("file is not valid UTF-8: {}", parsed.path)),
                    metadata: None,
                });
            }
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("failed to read {}: {e}", parsed.path)),
                    metadata: None,
                });
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start = parsed.start_line.unwrap_or(1);
        let end = parsed.end_line.unwrap_or(total_lines);

        if start < 1 || end < 1 {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("start_line and end_line must be >= 1".to_string()),
                metadata: None,
            });
        }

        if start > end {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "start_line ({start}) must not exceed end_line ({end})"
                )),
                metadata: None,
            });
        }

        if start > total_lines {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "start_line {start} exceeds file length {total_lines}"
                )),
                metadata: None,
            });
        }

        let end_clamped = end.min(total_lines);
        let slice: Vec<&str> = lines[(start - 1)..end_clamped].to_vec();

        let (output, lines_read, truncated) = if slice.len() > MAX_OUTPUT_LINES {
            let trimmed = &slice[..MAX_OUTPUT_LINES];
            let shown_end = start + MAX_OUTPUT_LINES - 1;
            let output = format!(
                "{}\n[read_file: truncated — file has {} lines; showing lines {}–{}. Re-read with start_line/end_line for other ranges.]",
                trimmed.join("\n"),
                total_lines,
                start,
                shown_end
            );
            (output, MAX_OUTPUT_LINES, true)
        } else {
            (slice.join("\n"), slice.len(), false)
        };

        let abs_path = path.canonicalize().unwrap_or_else(|_| path.clone());

        let metadata = if truncated {
            json!({
                "path": abs_path.to_string_lossy(),
                "bytes": output.len(),
                "lines": total_lines,
                "lines_read": lines_read,
                "truncated": true,
            })
        } else {
            json!({
                "path": abs_path.to_string_lossy(),
                "bytes": output.len(),
                "lines": total_lines,
                "lines_read": lines_read,
            })
        };

        Ok(ToolResult {
            output,
            error: None,
            metadata: Some(metadata),
        })
    }
}

pub fn read_file(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(ReadFile { scope })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_scope(dir: &tempfile::TempDir) -> Scope {
        Scope::new(dir.path()).unwrap()
    }

    #[tokio::test]
    async fn reads_whole_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello\nworld\n").unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": path.to_string_lossy() }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("hello"));
        assert!(result.output.contains("world"));
        let meta = result.metadata.unwrap();
        assert_eq!(meta["lines"], 2);
    }

    #[tokio::test]
    async fn reports_file_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": "nonexistent/file.txt" }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("not found"));
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn reports_directory_as_advisory() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = read_file(make_scope(&dir));
        let result = tool.execute(json!({ "path": "" })).await.unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("directory"));
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn reads_line_range() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("lines.txt");
        fs::write(&path, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 2,
                "end_line": 4
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("line2"));
        assert!(result.output.contains("line3"));
        assert!(result.output.contains("line4"));
        assert!(!result.output.contains("line1"));
        assert!(!result.output.contains("line5"));
        let meta = result.metadata.unwrap();
        assert_eq!(meta["lines_read"], 3);
    }

    #[tokio::test]
    async fn clamps_end_line_past_eof() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("short.txt");
        fs::write(&path, "a\nb\nc\n").unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 1,
                "end_line": 100
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let meta = result.metadata.unwrap();
        assert_eq!(meta["lines_read"], 3);
    }

    #[tokio::test]
    async fn rejects_start_greater_than_end() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello\n").unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 10,
                "end_line": 5
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn rejects_non_utf8() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("binary.bin");
        fs::write(&path, [0xFF, 0xFE, 0xFD]).unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": path.to_string_lossy() }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("UTF-8"));
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn rejects_malformed_args() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "wrong_field": "value" }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("invalid arguments"));
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn rejects_path_outside_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": "../outside/file.txt" }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("escapes"));
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn no_token_count_in_metadata() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello world\n").unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": path.to_string_lossy() }))
            .await
            .unwrap();
        assert!(result.error.is_none());

        let meta = result.metadata.unwrap();
        assert!(
            meta.get("tokens").is_none(),
            "metadata must not contain token count"
        );
        assert!(meta.get("path").is_some());
        assert!(meta.get("bytes").is_some());
        assert!(meta.get("lines").is_some());
        assert!(meta.get("lines_read").is_some());
    }

    #[tokio::test]
    async fn truncates_whole_file_read_at_max_lines() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("large.txt");
        let content = (1..=600)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, &content).unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": path.to_string_lossy() }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("line1"));
        assert!(!result.output.contains("line501"));
        assert!(result.output.contains("[read_file: truncated"));
        assert!(result.output.contains("600"));
        assert!(result.output.contains("500"));
        let meta = result.metadata.unwrap();
        assert_eq!(meta["lines_read"], 500);
        assert_eq!(meta["truncated"], true);
    }

    #[tokio::test]
    async fn truncation_notice_shows_correct_range() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("large.txt");
        let content = (1..=600)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, &content).unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": path.to_string_lossy() }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let output = result.output;
        // The notice should contain "1–500" (en-dash) or "1-500" (hyphen)
        assert!(
            output.contains("1–500") || output.contains("1-500"),
            "truncation notice should show range 1–500, got: {}",
            output
        );
    }

    #[tokio::test]
    async fn small_file_not_truncated() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("small.txt");
        let content = (1..=499)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, &content).unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": path.to_string_lossy() }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(!result.output.contains("[read_file:"));
        let meta = result.metadata.unwrap();
        assert_eq!(meta["lines_read"], 499);
        assert!(meta.get("truncated").is_none());
    }

    #[tokio::test]
    async fn explicit_range_within_cap_not_truncated() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("large.txt");
        let content = (1..=600)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, &content).unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 501,
                "end_line": 600
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("line501"));
        assert!(result.output.contains("line600"));
        assert!(!result.output.contains("[read_file:"));
        let meta = result.metadata.unwrap();
        assert_eq!(meta["lines_read"], 100);
        assert!(meta.get("truncated").is_none());
    }

    #[tokio::test]
    async fn explicit_range_exceeding_cap_is_capped() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("large.txt");
        let content = (1..=600)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, &content).unwrap();

        let tool = read_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "start_line": 1,
                "end_line": 600
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("line1"));
        assert!(!result.output.contains("line501"));
        assert!(result.output.contains("[read_file: truncated"));
        // Notice should show "1–500" not "1–600"
        assert!(
            result.output.contains("1–500") || result.output.contains("1-500"),
            "truncation notice should show 1–500, not 1–600"
        );
        let meta = result.metadata.unwrap();
        assert_eq!(meta["lines_read"], 500);
        assert_eq!(meta["truncated"], true);
    }

    #[tokio::test]
    async fn description_mentions_line_cap() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = read_file(make_scope(&dir));
        assert!(
            tool.description().contains("500 lines"),
            "description should mention '500 lines', got: {}",
            tool.description()
        );
    }
}
