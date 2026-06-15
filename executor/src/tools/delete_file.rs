// delete_file: delete a file within the project root.
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
struct DeleteFileArgs {
    path: String,
}

pub struct DeleteFile {
    scope: Scope,
}

#[async_trait]
impl Tool for DeleteFile {
    fn name(&self) -> &str {
        "delete_file"
    }

    fn description(&self) -> &str {
        "Delete a file. Directories are not permitted; use bash for recursive removal."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to delete, confined to the project root."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<DeleteFileArgs>(args) {
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

        if path.is_dir() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "path is a directory: {} — use bash for directory removal",
                    path.display()
                )),
                metadata: None,
            });
        }

        if !path.exists() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!("file not found: {}", path.display())),
                metadata: None,
            });
        }

        let abs_path = path.canonicalize().unwrap_or_else(|_| path.clone());

        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("failed to delete {}: {e}", parsed.path)),
                    metadata: None,
                });
            }
        }

        let metadata = json!({
            "path": abs_path.to_string_lossy(),
        });

        Ok(ToolResult {
            output: format!("deleted {}", abs_path.to_string_lossy()),
            error: None,
            metadata: Some(metadata),
        })
    }
}

pub fn delete_file(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(DeleteFile { scope })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scope(dir: &tempfile::TempDir) -> Scope {
        Scope::new(dir.path()).unwrap()
    }

    #[tokio::test]
    async fn deletes_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("to_delete.txt");
        std::fs::write(&path, "delete me").unwrap();

        let tool = delete_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(!path.exists());
        assert!(result.output.contains("deleted"));
    }

    #[tokio::test]
    async fn errors_on_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        let tool = delete_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": subdir.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result.error.as_ref().unwrap().contains("directory"),
            "error: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn errors_on_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.txt");

        let tool = delete_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result.error.as_ref().unwrap().contains("not found"),
            "error: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn rejects_path_outside_scope() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let scope = Scope::new(&root).unwrap();
        let tool = delete_file(scope);
        let result = tool
            .execute(json!({
                "path": "../outside.txt"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(
            err.contains("escapes") || err.contains("outside"),
            "error: {err}"
        );
    }
}
