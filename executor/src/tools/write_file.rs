// write_file: create or overwrite a file. Prefer `patch` for edits.
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
struct WriteFileArgs {
    path: String,
    content: String,
}

pub struct WriteFile {
    scope: Scope,
}

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file with full content. Prefer patch for edits."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to write, confined to the project root. Relative paths resolve under the project root."
                },
                "content": {
                    "type": "string",
                    "description": "Full file content. Overwrites existing files."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<WriteFileArgs>(args) {
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

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "parent directory does not exist: {}",
                    parent.to_string_lossy()
                )),
                metadata: None,
            });
        }

        let existed = path.exists();
        let bytes = parsed.content.len();

        match std::fs::write(&path, &parsed.content) {
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
            "bytes_written": bytes,
            "created": !existed,
            "overwritten": existed,
        });

        Ok(ToolResult {
            output: format!("wrote {bytes} bytes to {}", abs_path.to_string_lossy()),
            error: None,
            metadata: Some(metadata),
        })
    }
}

pub fn write_file(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(WriteFile { scope })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scope(dir: &tempfile::TempDir) -> Scope {
        Scope::new(dir.path()).unwrap()
    }

    #[tokio::test]
    async fn creates_new_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("new.txt");

        let tool = write_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "content": "hello world"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("11 bytes"));
        let meta = result.metadata.unwrap();
        assert!(meta["created"].as_bool().unwrap());
        assert!(!meta["overwritten"].as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn overwrites_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("existing.txt");
        std::fs::write(&path, "old content").unwrap();

        let tool = write_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "content": "new content"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        let meta = result.metadata.unwrap();
        assert!(!meta["created"].as_bool().unwrap());
        assert!(meta["overwritten"].as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
    }

    #[tokio::test]
    async fn reports_missing_parent_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("missing_subdir").join("file.txt");

        let tool = write_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "content": "test"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("parent directory"));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn rejects_malformed_args() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = write_file(make_scope(&dir));
        let result = tool
            .execute(json!({ "path": "/tmp/test.txt" }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("invalid arguments"));
    }

    #[tokio::test]
    async fn scope_escape_returns_advisory_error_and_writes_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = write_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": "../outside.txt",
                "content": "should not exist"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("escapes") || err.contains("outside"));

        let outside = dir.path().parent().unwrap().join("outside.txt");
        assert!(!outside.exists());
    }
}
