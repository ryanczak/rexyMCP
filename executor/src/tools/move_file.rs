// move_file: move or rename a file within the project root.
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
struct MoveFileArgs {
    from: String,
    to: String,
}

pub struct MoveFile {
    scope: Scope,
}

#[async_trait]
impl Tool for MoveFile {
    fn name(&self) -> &str {
        "move_file"
    }

    fn description(&self) -> &str {
        "Move or rename a file within the project root. Both paths are confined to the project root."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "from": {
                    "type": "string",
                    "description": "Source path, confined to the project root."
                },
                "to": {
                    "type": "string",
                    "description": "Destination path, confined to the project root."
                }
            },
            "required": ["from", "to"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<MoveFileArgs>(args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid arguments: {e}")),
                    metadata: None,
                });
            }
        };

        let from_path = match self.scope.resolve(&parsed.from) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(e.to_string()),
                    metadata: None,
                });
            }
        };

        let to_path = match self.scope.resolve(&parsed.to) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(e.to_string()),
                    metadata: None,
                });
            }
        };

        if !from_path.exists() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!("file not found: {}", from_path.display())),
                metadata: None,
            });
        }

        if to_path.exists() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "destination already exists: {} — delete it first if you want to overwrite",
                    to_path.display()
                )),
                metadata: None,
            });
        }

        if let Some(parent) = to_path.parent()
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

        let abs_from = from_path
            .canonicalize()
            .unwrap_or_else(|_| from_path.clone());
        let abs_to = to_path.canonicalize().unwrap_or_else(|_| to_path.clone());

        match std::fs::rename(&from_path, &to_path) {
            Ok(()) => {}
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!(
                        "failed to move {} to {}: {e}",
                        parsed.from, parsed.to
                    )),
                    metadata: None,
                });
            }
        }

        let metadata = json!({
            "from": abs_from.to_string_lossy(),
            "to": abs_to.to_string_lossy(),
        });

        Ok(ToolResult {
            output: format!(
                "moved {} → {}",
                abs_from.to_string_lossy(),
                abs_to.to_string_lossy()
            ),
            error: None,
            metadata: Some(metadata),
        })
    }
}

pub fn move_file(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(MoveFile { scope })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scope(dir: &tempfile::TempDir) -> Scope {
        Scope::new(dir.path()).unwrap()
    }

    #[tokio::test]
    async fn moves_file_to_new_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = dir.path().join("old.txt");
        let to = dir.path().join("new.txt");
        std::fs::write(&from, "content").unwrap();

        let tool = move_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "from": from.to_string_lossy(),
                "to": to.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(!from.exists());
        assert!(to.exists());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "content");
        assert!(result.output.contains("moved"));
    }

    #[tokio::test]
    async fn errors_when_destination_exists() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = dir.path().join("old.txt");
        let to = dir.path().join("existing.txt");
        std::fs::write(&from, "content").unwrap();
        std::fs::write(&to, "other").unwrap();

        let tool = move_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "from": from.to_string_lossy(),
                "to": to.to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(
            result.error.as_ref().unwrap().contains("already exists"),
            "error: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn errors_when_from_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let from = dir.path().join("nonexistent.txt");
        let to = dir.path().join("new.txt");

        let tool = move_file(make_scope(&dir));
        let result = tool
            .execute(json!({
                "from": from.to_string_lossy(),
                "to": to.to_string_lossy()
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
    async fn rejects_from_outside_scope() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let scope = Scope::new(&root).unwrap();
        let tool = move_file(scope);
        let result = tool
            .execute(json!({
                "from": "../outside.txt",
                "to": "inside.txt"
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

    #[tokio::test]
    async fn rejects_to_outside_scope() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let from = root.join("inside.txt");
        std::fs::write(&from, "content").unwrap();

        let scope = Scope::new(&root).unwrap();
        let tool = move_file(scope);
        let result = tool
            .execute(json!({
                "from": "inside.txt",
                "to": "../outside.txt"
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
