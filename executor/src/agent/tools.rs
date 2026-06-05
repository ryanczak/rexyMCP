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

/// Dispatch a tool call through the registry. Returns `(succeeded, content)`
/// where `content` is the message fed back to the model. A missing tool or an
/// execution error is a model-visible failure, not an `Err`.
pub(super) async fn dispatch(registry: &ToolRegistry, tc: &ToolCall) -> (bool, String) {
    match registry.get(&tc.name) {
        None => (false, format!("error: unknown tool '{}'", tc.name)),
        Some(tool) => match tool.execute(tc.arguments.clone()).await {
            Ok(result) => match result.error {
                Some(error) => (false, error),
                None => (true, result.output),
            },
            Err(e) => (false, format!("tool execution failed: {e}")),
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
