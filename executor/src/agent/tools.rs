use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::ai::next_tool_id;
use crate::ai::types::{Message, ToolCall as AiToolCall, ToolResult as AiToolResult};
use crate::governor::verifier::{Diagnostic, Severity};
use crate::parser::ToolCall;
use crate::tools::ToolRegistry;

/// Preview cap for a tool result's `output_preview` in the session log — enough
/// to triage a failure, not the full (possibly huge) output.
const OUTPUT_PREVIEW_CHARS: usize = 500;

pub(super) fn output_preview(content: &str) -> String {
    if content.chars().count() > OUTPUT_PREVIEW_CHARS {
        content.chars().take(OUTPUT_PREVIEW_CHARS).collect()
    } else {
        content.to_string()
    }
}

/// Resolve a tool call's `"path"` argument against the project root. `None` if
/// the call has no string `"path"`.
pub(super) fn resolve_path(tool_call: &ToolCall, project_root: &Path) -> Option<PathBuf> {
    let path = PathBuf::from(tool_call.arguments.get("path").and_then(|v| v.as_str())?);
    Some(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

/// The file an edit-class (`write_file` / `patch`) call targets, resolved against
/// the project root. `None` for non-edit calls or calls missing a `"path"` arg.
pub(super) fn edit_target(tool_call: &ToolCall, project_root: &Path) -> Option<PathBuf> {
    if tool_call.name != "write_file" && tool_call.name != "patch" {
        return None;
    }
    resolve_path(tool_call, project_root)
}

/// The read-before-edit gate (07d). Refuse a `patch` on a file the model has not
/// read this session, or one whose on-disk mtime no longer matches what was read.
/// `None` = allowed. Pure over `working_set` so the mtime-mismatch case is
/// unit-testable without mid-session filesystem hooks. `patch`-only — `write_file`
/// (whole-file create/overwrite) is not gated.
pub(super) fn read_before_edit_refusal(
    tool_call: &ToolCall,
    working_set: &HashMap<PathBuf, SystemTime>,
    project_root: &Path,
) -> Option<String> {
    if tool_call.name != "patch" {
        return None;
    }
    let path = resolve_path(tool_call, project_root)?;
    match working_set.get(&path) {
        None => Some(format!(
            "refusing to patch {}: you have not read it this session. Use read_file on it first.",
            path.display()
        )),
        Some(recorded) => {
            let current = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            match current {
                Some(now) if now == *recorded => None,
                _ => Some(format!(
                    "refusing to patch {}: it changed on disk since you read it. Re-read it with read_file first.",
                    path.display()
                )),
            }
        }
    }
}

/// Record (or refresh) a file's mtime in the working set. Best-effort — a file
/// that can't be stat'd is simply not recorded.
pub(super) fn record_mtime(working_set: &mut HashMap<PathBuf, SystemTime>, path: &Path) {
    if let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) {
        working_set.insert(path.to_path_buf(), modified);
    }
}

/// Stable marker prefix for an evicted (superseded) read result. Used both to
/// build the breadcrumb and to detect an already-evicted result (idempotence).
const SUPERSEDED_PREFIX: &str = "[superseded:";

/// Replace the content of every prior `read_file` tool-result for `edited_path`
/// with a short re-read breadcrumb, because the file's on-disk content changed
/// when the model edited it — the earlier read is now stale and only wastes
/// context. Returns `(reads_evicted, tokens_reclaimed)` where `tokens_reclaimed`
/// is the summed `tokens::count(original) - tokens::count(breadcrumb)` across the
/// evicted reads (the chars/4 estimate, same heuristic as the budget) — the loop
/// uses it to emit a `ReadEvicted` event.
///
/// Safe by construction: the read-before-edit gate already forces a re-read
/// before the next `patch`, so removing stale read content never causes a wrong
/// action. Idempotent: a result already replaced by a breadcrumb (detected via
/// `SUPERSEDED_PREFIX`) is left alone, so re-editing the same file does not
/// re-evict or stack breadcrumbs.
pub(super) fn evict_superseded_reads(
    messages: &mut [Message],
    edited_path: &Path,
    turn: usize,
    project_root: &Path,
) -> (usize, usize) {
    let breadcrumb = format!(
        "{SUPERSEDED_PREFIX} file edited at turn {turn}; this earlier read is stale — \
         re-read with read_file for current content]"
    );
    let breadcrumb_tokens = crate::context::tokens::count(&breadcrumb);
    let mut evicted = 0;
    let mut tokens_reclaimed = 0usize;
    for i in 0..messages.len() {
        // Is messages[i] an assistant `read_file` call whose path == edited_path?
        let matches_read = messages[i]
            .tool_calls
            .as_ref()
            .and_then(|tcs| tcs.first())
            .filter(|tc| tc.name == "read_file")
            .and_then(|tc| serde_json::from_str::<serde_json::Value>(&tc.arguments).ok())
            .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(str::to_string))
            .map(|p| {
                let pb = std::path::PathBuf::from(&p);
                let resolved = if pb.is_absolute() {
                    pb
                } else {
                    project_root.join(pb)
                };
                resolved == edited_path
            })
            .unwrap_or(false);
        if !matches_read {
            continue;
        }
        // The result content is on the next message (the tool message).
        if let Some(next) = messages.get_mut(i + 1)
            && let Some(results) = next.tool_results.as_mut()
            && let Some(r) = results.first_mut()
            && r.tool_name == "read_file"
            && !r.content.starts_with(SUPERSEDED_PREFIX)
        {
            tokens_reclaimed +=
                crate::context::tokens::count(&r.content).saturating_sub(breadcrumb_tokens);
            r.content = breadcrumb.clone();
            evicted += 1;
        }
    }
    (evicted, tokens_reclaimed)
}

/// Render author diagnostics into a retry message the model can act on.
pub(super) fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    let mut out =
        String::from("The verifier found errors you introduced. Fix them and continue:\n");
    for d in diagnostics {
        let col = d.column.map(|c| format!(":{c}")).unwrap_or_default();
        let severity = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
            Severity::Help => "help",
        };
        out.push_str(&format!(
            "- {}:{}{col} {severity}: {}\n",
            d.path.display(),
            d.line,
            d.message,
        ));
    }
    out
}

/// Dispatch a tool call through the registry. Returns `(succeeded, content, metadata)`
/// where `content` is the message fed back to the model and `metadata` is the
/// success-path `ToolResult.metadata` (for per-lever instrumentation). A missing
/// tool or an execution error is a model-visible failure, not an `Err`.
pub(super) async fn dispatch(
    registry: &ToolRegistry,
    tc: &ToolCall,
) -> (bool, String, Option<serde_json::Value>) {
    match registry.get(&tc.name) {
        None => (false, format!("error: unknown tool '{}'", tc.name), None),
        Some(tool) => match tool.execute(tc.arguments.clone()).await {
            Ok(result) => match result.error {
                Some(error) => (false, error, None),
                None => (true, result.output, result.metadata),
            },
            Err(e) => (false, format!("tool execution failed: {e}"), None),
        },
    }
}

pub(super) fn append_tool_exchange(
    messages: &mut Vec<Message>,
    tc: &ToolCall,
    content: &str,
    turn: usize,
) {
    let id = next_tool_id();
    let arguments = serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string());
    messages.push(Message {
        role: "assistant".to_string(),
        content: String::new(),
        tool_calls: Some(vec![AiToolCall {
            id: id.clone(),
            name: tc.name.clone(),
            arguments,
            thought_signature: None,
        }]),
        tool_results: None,
        turn: Some(turn),
    });
    messages.push(Message {
        role: "tool".to_string(),
        content: String::new(),
        tool_calls: None,
        tool_results: Some(vec![AiToolResult {
            tool_call_id: id,
            tool_name: tc.name.clone(),
            content: content.to_string(),
        }]),
        turn: Some(turn),
    });
}

pub(super) fn assistant_text(content: &str, turn: usize) -> Message {
    Message {
        role: "assistant".to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_results: None,
        turn: Some(turn),
    }
}

pub(super) fn user_text(content: &str, turn: usize) -> Message {
    Message {
        role: "user".to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_results: None,
        turn: Some(turn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{Tool, ToolResult};
    use serde_json::json;
    use std::sync::Arc;

    struct StubTool {
        error: Option<String>,
        metadata: Option<serde_json::Value>,
    }

    #[async_trait::async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            "stub"
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn schema(&self) -> serde_json::Value {
            json!({})
        }
        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult, anyhow::Error> {
            Ok(ToolResult {
                output: "ok".to_string(),
                error: self.error.clone(),
                metadata: self.metadata.clone(),
            })
        }
    }

    #[tokio::test]
    async fn dispatch_surfaces_success_metadata() {
        let mut registry = ToolRegistry::new();
        let tool: Arc<dyn Tool> = Arc::new(StubTool {
            error: None,
            metadata: Some(json!({"key": "value"})),
        });
        registry.register(tool);

        let tc = ToolCall {
            name: "stub".to_string(),
            arguments: json!({}),
            origin: crate::parser::Origin::Native,
        };
        let (ok, content, meta) = dispatch(&registry, &tc).await;
        assert!(ok);
        assert_eq!(content, "ok");
        assert_eq!(meta, Some(json!({"key": "value"})));
    }

    #[tokio::test]
    async fn dispatch_returns_none_metadata_on_error() {
        let mut registry = ToolRegistry::new();
        let tool: Arc<dyn Tool> = Arc::new(StubTool {
            error: Some("fail".to_string()),
            metadata: Some(json!({"key": "value"})),
        });
        registry.register(tool);

        let tc = ToolCall {
            name: "stub".to_string(),
            arguments: json!({}),
            origin: crate::parser::Origin::Native,
        };
        let (ok, content, meta) = dispatch(&registry, &tc).await;
        assert!(!ok);
        assert_eq!(content, "fail");
        assert!(meta.is_none(), "error path should return None metadata");
    }

    // ── evict_superseded_reads tests ──────────────────────────────────────

    fn make_read_exchange(path: &str, content: &str) -> Vec<Message> {
        let arguments = serde_json::to_string(&json!({ "path": path })).unwrap();
        vec![
            Message {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(vec![AiToolCall {
                    id: "rcall-1".to_string(),
                    name: "read_file".to_string(),
                    arguments,
                    thought_signature: None,
                }]),
                tool_results: None,
                turn: Some(1),
            },
            Message {
                role: "tool".to_string(),
                content: String::new(),
                tool_calls: None,
                tool_results: Some(vec![AiToolResult {
                    tool_call_id: "rcall-1".to_string(),
                    tool_name: "read_file".to_string(),
                    content: content.to_string(),
                }]),
                turn: Some(1),
            },
        ]
    }

    fn make_bash_exchange(cmd: &str, output: &str) -> Vec<Message> {
        let arguments = serde_json::to_string(&json!({ "command": cmd })).unwrap();
        vec![
            Message {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(vec![AiToolCall {
                    id: "bash-1".to_string(),
                    name: "bash".to_string(),
                    arguments,
                    thought_signature: None,
                }]),
                tool_results: None,
                turn: Some(2),
            },
            Message {
                role: "tool".to_string(),
                content: String::new(),
                tool_calls: None,
                tool_results: Some(vec![AiToolResult {
                    tool_call_id: "bash-1".to_string(),
                    tool_name: "bash".to_string(),
                    content: output.to_string(),
                }]),
                turn: Some(2),
            },
        ]
    }

    #[test]
    fn evict_superseded_reads_replaces_prior_read_of_edited_file() {
        // Content must be longer than the breadcrumb (~96 chars) for tokens_reclaimed > 0
        let original = "fn example() {\n    // This is a longer function that does something interesting and takes up enough\n    // space that evicting it actually reclaims tokens. The original content must be\n    // longer than the breadcrumb itself for tokens_reclaimed to be positive.\n    let x = 42;\n    let y = x * 2;\n    let z = y + 10;\n    println!(\"hello world from example function with value {}\", z);\n    z\n}\nfn helper() {\n    println!(\"helper function output\");\n}\nfn another_function() {\n    // More content to ensure we're well above the breadcrumb length\n    let result = vec![1, 2, 3, 4, 5];\n    result.into_iter().sum::<i32>()\n}\n";
        let mut msgs = make_read_exchange("/r/foo.rs", original);
        let (evicted, tokens) =
            evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 7, Path::new("/r"));
        assert_eq!(evicted, 1);
        assert!(
            tokens > 0,
            "tokens_reclaimed should be positive for content longer than breadcrumb"
        );
        let result = &msgs[1].tool_results.as_ref().unwrap()[0].content;
        assert!(result.starts_with("[superseded:"));
        assert!(!result.contains("example"));
    }

    #[test]
    fn evict_superseded_reads_resolves_relative_read_path() {
        let mut msgs = make_read_exchange("foo.rs", "relative content");
        let (evicted, _) =
            evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 5, Path::new("/r"));
        assert_eq!(evicted, 1);
        let result = &msgs[1].tool_results.as_ref().unwrap()[0].content;
        assert!(result.starts_with("[superseded:"));
    }

    #[test]
    fn evict_superseded_reads_leaves_other_files_untouched() {
        let mut msgs = make_read_exchange("/r/bar.rs", "bar content");
        let (evicted, tokens) =
            evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 5, Path::new("/r"));
        assert_eq!(evicted, 0);
        assert_eq!(tokens, 0);
        let result = &msgs[1].tool_results.as_ref().unwrap()[0].content;
        assert_eq!(result, "bar content");
    }

    #[test]
    fn evict_superseded_reads_ignores_non_read_results() {
        let mut msgs = make_bash_exchange("echo hi", "bash output");
        let (evicted, tokens) =
            evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 5, Path::new("/r"));
        assert_eq!(evicted, 0);
        assert_eq!(tokens, 0);
        let result = &msgs[1].tool_results.as_ref().unwrap()[0].content;
        assert_eq!(result, "bash output");
    }

    #[test]
    fn evict_superseded_reads_is_idempotent() {
        let mut msgs = make_read_exchange("/r/foo.rs", "original content");
        let (evicted1, _) =
            evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 7, Path::new("/r"));
        assert_eq!(evicted1, 1);
        let (evicted2, tokens2) =
            evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 8, Path::new("/r"));
        assert_eq!(evicted2, 0);
        assert_eq!(tokens2, 0);
        let result = &msgs[1].tool_results.as_ref().unwrap()[0].content;
        // Should contain exactly one breadcrumb, not a nested one
        assert!(result.starts_with("[superseded:"));
        assert!(!result.contains("[superseded:[superseded:"));
    }

    #[test]
    fn evict_superseded_reads_evicts_multiple_prior_reads() {
        let mut msgs = Vec::new();
        msgs.extend(make_read_exchange("/r/foo.rs", "first read"));
        msgs.extend(make_read_exchange("/r/foo.rs", "second read"));
        let (evicted, _) =
            evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 10, Path::new("/r"));
        assert_eq!(evicted, 2);
        assert!(
            msgs[1].tool_results.as_ref().unwrap()[0]
                .content
                .starts_with("[superseded:")
        );
        assert!(
            msgs[3].tool_results.as_ref().unwrap()[0]
                .content
                .starts_with("[superseded:")
        );
    }

    #[test]
    fn evict_superseded_reads_breadcrumb_mentions_turn_and_reread() {
        let mut msgs = make_read_exchange("/r/foo.rs", "content");
        let (evicted, _) =
            evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 7, Path::new("/r"));
        assert_eq!(evicted, 1);
        let result = &msgs[1].tool_results.as_ref().unwrap()[0].content;
        assert!(result.contains("turn 7"));
        assert!(result.contains("read_file"));
    }
}
