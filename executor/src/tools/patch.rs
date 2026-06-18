// patch: search-and-replace edit. The primary edit primitive — safer than
// full-file rewrites for small models. Uses `similar` for fuzzy match when
// exact match fails.
//
// Adapted from Rexy: uses Scope for path resolution instead of CWD; fs::write
// failure returned as advisory ToolResult error (not ?-bubbled).

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use similar::TextDiff;
use std::sync::Arc;

use crate::security::scope::Scope;

use super::registry::{Tool, ToolResult};

#[derive(Deserialize)]
struct PatchArgs {
    path: String,
    old_str: String,
    new_str: String,
}

pub struct Patch {
    scope: Scope,
}

#[async_trait]
impl Tool for Patch {
    fn name(&self) -> &str {
        "patch"
    }

    fn description(&self) -> &str {
        "Search-and-replace edit. The primary edit primitive — replace an exact \
         substring in a file. Use write_file for create-or-overwrite."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to patch, confined to the project root."
                },
                "old_str": {
                    "type": "string",
                    "description": "Exact substring to replace. Must appear exactly once."
                },
                "new_str": {
                    "type": "string",
                    "description": "Replacement text. May be empty (deletion)."
                }
            },
            "required": ["path", "old_str", "new_str"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let parsed = match serde_json::from_value::<PatchArgs>(args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    output: String::new(),
                    error: Some(format!("invalid arguments: {e}")),
                    metadata: None,
                });
            }
        };

        if parsed.old_str.is_empty() {
            return Ok(ToolResult {
                output: String::new(),
                error: Some("old_str must not be empty".to_string()),
                metadata: None,
            });
        }

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

        let match_count = content.matches(&parsed.old_str).count();

        if parsed.old_str == parsed.new_str {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(noop_hint(&parsed.path, &content, &parsed.old_str)),
                metadata: None,
            });
        }

        match match_count {
            0 => {
                let hint = fuzzy_hint(&parsed.path, &content, &parsed.old_str);
                Ok(ToolResult {
                    output: String::new(),
                    error: Some(hint),
                    metadata: None,
                })
            }
            1 => {
                let bytes_before = content.len();
                let after = content.replacen(&parsed.old_str, &parsed.new_str, 1);
                let bytes_after = after.len();

                if let Err(e) = std::fs::write(&path, &after) {
                    return Ok(ToolResult {
                        output: String::new(),
                        error: Some(format!("failed to write patched file: {e}")),
                        metadata: None,
                    });
                }

                let abs_path = path.canonicalize().unwrap_or_else(|_| path.clone());
                let path_display = abs_path.to_string_lossy();

                let diff = TextDiff::from_lines(&content, &after)
                    .unified_diff()
                    .header(&parsed.path, &parsed.path)
                    .to_string();

                let output = format!("✓ patched {path_display} (1 hunk)\n\n{diff}");

                let metadata = json!({
                    "path": path_display,
                    "bytes_before": bytes_before,
                    "bytes_after": bytes_after,
                    "hunks": 1,
                });

                Ok(ToolResult {
                    output,
                    error: None,
                    metadata: Some(metadata),
                })
            }
            n => Ok(ToolResult {
                output: String::new(),
                error: Some(format!(
                    "old_str matches {n} times in {}; provide more surrounding context to disambiguate",
                    parsed.path
                )),
                metadata: None,
            }),
        }
    }
}

fn noop_hint(path: &str, content: &str, old_str: &str) -> String {
    let preamble =
        "no-op patch: old_str and new_str are identical, so this patch would change nothing.";

    let Some(offset) = content.find(old_str) else {
        return format!(
            "{preamble}\n\nold_str was not found in {path}, so the file does not \
             currently contain this text. Call read_file to see the current content \
             before patching."
        );
    };

    let lines: Vec<&str> = content.lines().collect();
    let start_line = content[..offset].bytes().filter(|&b| b == b'\n').count() + 1;
    let span = old_str.lines().count().max(1);
    let end_line = start_line + span - 1;

    let ctx_start = start_line.saturating_sub(2).max(1);
    let ctx_end = (end_line + 2).min(lines.len());

    let window: String = lines
        .iter()
        .enumerate()
        .filter(|(idx, _)| {
            let lineno = idx + 1;
            lineno >= ctx_start && lineno <= ctx_end
        })
        .map(|(idx, line)| format!("{:>4} | {line}\n", idx + 1))
        .collect();

    let occurrences = content.matches(old_str).count();
    let multiplicity = if occurrences > 1 {
        format!(
            "\n\nNote: this text appears {occurrences} times in {path}. If you meant \
             to remove a duplicate, give old_str a larger window that includes the \
             unique lines around the copy you want to change."
        )
    } else {
        String::new()
    };

    format!(
        "{preamble}\n\nThe file already contains this exact text at \
         {path}:{start_line}-{end_line}:\n{window}\nTo make an edit, old_str and \
         new_str must differ. If the file already has the content you intended, no \
         patch is needed — re-read the file with read_file to confirm the current \
         state, then move on to the next step.{multiplicity}"
    )
}

fn fuzzy_hint(path: &str, content: &str, old_str: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    if total_lines == 0 {
        return format!("0 matches for old_str in {path}");
    }

    let old_line_count = old_str.lines().count();
    let window_size = if old_line_count == 1 {
        5
    } else {
        old_line_count
    };

    if total_lines < window_size {
        return format!("0 matches for old_str in {path}");
    }

    let mut best_ratio: f32 = 0.0;
    let mut best_start: usize = 0;

    for start in 0..=(total_lines - window_size) {
        let window = lines[start..start + window_size].join("\n");
        let ratio = TextDiff::from_chars(old_str, &window).ratio();
        if ratio > best_ratio {
            best_ratio = ratio;
            best_start = start;
        }
    }

    let end = best_start + window_size;
    let window_content = lines[best_start..end].join("\n");

    format!(
        "0 matches for old_str in {path}\n\nClosest window at {path}:{}-{}:\n{window_content}",
        best_start + 1,
        end
    )
}

pub fn patch(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(Patch { scope })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scope(dir: &tempfile::TempDir) -> Scope {
        Scope::new(dir.path()).unwrap()
    }

    #[tokio::test]
    async fn patches_exact_single_match() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "foo bar baz").unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "bar",
                "new_str": "qux"
            }))
            .await
            .unwrap();

        assert!(result.error.is_none());
        assert!(result.output.contains("✓ patched"));
        assert!(result.output.contains("(1 hunk)"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "foo qux baz");
        let meta = result.metadata.unwrap();
        assert_eq!(meta["hunks"], 1);
        assert!(meta["bytes_before"].as_u64().is_some());
        assert!(meta["bytes_after"].as_u64().is_some());
    }

    #[tokio::test]
    async fn success_output_contains_unified_diff() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "foo bar baz").unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "bar",
                "new_str": "qux"
            }))
            .await
            .unwrap();

        assert!(result.output.contains("-foo bar baz"));
        assert!(result.output.contains("+foo qux baz"));
    }

    #[tokio::test]
    async fn preserves_surrounding_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "before\nmiddle\nafter\n").unwrap();

        let tool = patch(make_scope(&dir));
        let _result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "middle",
                "new_str": "new"
            }))
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "before\nnew\nafter\n"
        );
    }

    #[tokio::test]
    async fn reports_zero_matches_with_fuzzy_hint() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(
            &path,
            "use std::path::PathBuf;\n\nfn validate_token(t: &str) -> bool {\n    true\n}\n\nfn main() {}\n",
        )
        .unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "fn verify_token(t: &str) -> bool",
                "new_str": "fn verify_token(t: &str) -> bool { false }"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("0 matches"));
        assert!(err.contains("test.rs"));
        assert!(err.contains("Closest window at"));
    }

    #[tokio::test]
    async fn reports_ambiguous_multiple_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let original = "foo\nfoo\nfoo\n";
        std::fs::write(&path, original).unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "foo",
                "new_str": "bar"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("3"));
        assert!(err.contains("disambiguate"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[tokio::test]
    async fn rejects_empty_old_str() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello").unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "",
                "new_str": "x"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("old_str"));
        assert!(err.contains("empty"));
    }

    #[tokio::test]
    async fn rejects_identical_old_and_new() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello").unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "hello",
                "new_str": "hello"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("no-op"));
    }

    #[tokio::test]
    async fn rejects_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": "nonexistent/file.txt",
                "old_str": "foo",
                "new_str": "bar"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn rejects_directory_as_path() {
        let dir = tempfile::TempDir::new().unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": ".",
                "old_str": "foo",
                "new_str": "bar"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("directory"));
    }

    #[tokio::test]
    async fn rejects_non_utf8_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("binary.bin");
        std::fs::write(&path, [0xFF, 0xFE, 0xFD]).unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "foo",
                "new_str": "bar"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("UTF-8"));
    }

    #[tokio::test]
    async fn rejects_malformed_args() {
        let dir = tempfile::TempDir::new().unwrap();
        let tool = patch(make_scope(&dir));
        let result = tool.execute(json!({ "path": "x" })).await.unwrap();

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("invalid arguments"));
    }

    #[tokio::test]
    async fn does_not_modify_file_on_zero_match() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let original = "hello\n";
        std::fs::write(&path, original).unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "goodbye",
                "new_str": "hi"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[tokio::test]
    async fn scope_escape_returns_advisory_error_and_modifies_nothing() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let scope = Scope::new(&root).unwrap();
        let outside_path = temp.path().join("outside_patch.txt");
        std::fs::write(&outside_path, "original outside content").unwrap();

        let tool = patch(scope);
        let result = tool
            .execute(json!({
                "path": "../outside_patch.txt",
                "old_str": "original outside content",
                "new_str": "hacked"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("escapes") || err.contains("outside"));

        assert_eq!(
            std::fs::read_to_string(&outside_path).unwrap(),
            "original outside content"
        );
    }

    #[tokio::test]
    async fn noop_patch_shows_location_and_context_window() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "fn foo() {}\nfn bar() {}\nfn baz() {}\n").unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "fn bar() {}",
                "new_str": "fn bar() {}"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("no-op"), "error should mention no-op: {err}");
        assert!(
            err.contains(&*path.to_string_lossy()),
            "error should contain file path: {err}"
        );
        assert!(
            err.contains(":2-"),
            "error should contain line reference :2-: {err}"
        );
        assert!(err.contains(" | "), "error should contain gutter: {err}");
        assert!(
            err.contains("fn bar()"),
            "error should contain matched line: {err}"
        );
        assert!(
            err.contains("read_file"),
            "error should suggest read_file: {err}"
        );
    }

    #[tokio::test]
    async fn noop_patch_flags_duplicate_occurrences() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "foo\nbar\nfoo\nbaz\n").unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "foo",
                "new_str": "foo"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(
            err.contains("2 times"),
            "error should mention 2 times: {err}"
        );
    }

    #[tokio::test]
    async fn noop_patch_when_old_str_absent_suggests_read_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world\n").unwrap();

        let tool = patch(make_scope(&dir));
        let result = tool
            .execute(json!({
                "path": path.to_string_lossy(),
                "old_str": "goodbye world",
                "new_str": "goodbye world"
            }))
            .await
            .unwrap();

        assert!(result.error.is_some());
        let err = result.error.as_ref().unwrap();
        assert!(
            err.contains("not found"),
            "error should mention not found: {err}"
        );
        assert!(
            err.contains("read_file"),
            "error should suggest read_file: {err}"
        );
        assert!(
            !err.contains(" | "),
            "error should not contain gutter when text is absent: {err}"
        );
    }
}
