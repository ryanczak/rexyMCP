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

/// The read-before-edit gate (07d, extended M26 for `write_file`). Refuse an
/// edit-class call on a file the model has not read this session, or one whose
/// on-disk mtime no longer matches what was read. `None` = allowed.
///
/// `patch` is always gated. `write_file` is gated **only when it would
/// overwrite an existing file**: a create (target absent on disk) and an
/// append (`append: true`) are allowed unconditionally — neither blind-clobbers
/// content the model never read. The on-disk `metadata`/`exists` stats mirror
/// the mtime branch's existing filesystem touch, so every arm stays
/// `TempDir`-testable.
pub(super) fn read_before_edit_refusal(
    tool_call: &ToolCall,
    working_set: &HashMap<PathBuf, SystemTime>,
    project_root: &Path,
) -> Option<String> {
    let verb = match tool_call.name.as_str() {
        "patch" => "patch",
        "write_file" => "overwrite",
        _ => return None,
    };
    let path = resolve_path(tool_call, project_root)?;
    if tool_call.name == "write_file" {
        let appending = tool_call
            .arguments
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if appending || !path.exists() {
            return None;
        }
    }
    match working_set.get(&path) {
        None => Some(format!(
            "refusing to {verb} {}: you have not read it this session. Use read_file on it first.",
            path.display()
        )),
        Some(recorded) => {
            let current = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            match current {
                Some(now) if now == *recorded => None,
                _ => Some(format!(
                    "refusing to {verb} {}: it changed on disk since you read it. Re-read it with read_file first.",
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

/// Refuse a single-file `git checkout <path>` / `git restore <path>` of a file the
/// executor has edited this session — it would silently discard the model's own
/// uncommitted work. `None` = allowed. `bash`-only; the wholesale forms
/// (`git checkout .` / `git reset --hard` / …) are already blocked by
/// `security::bash_classify`. Pure over `edited` for unit-testability.
pub(super) fn destructive_restore_refusal(
    tool_call: &ToolCall,
    edited: &HashMap<PathBuf, Option<String>>,
    project_root: &Path,
) -> Option<String> {
    if tool_call.name != "bash" {
        return None;
    }
    let command = tool_call
        .arguments
        .get("command")
        .and_then(|v| v.as_str())?;
    for token in restore_path_tokens(command) {
        let resolved = project_root.join(token);
        if edited.contains_key(&resolved) {
            return Some(format!(
                "refusing to run `{command}`: it would discard your uncommitted edits to {} \
                 this session. Do not revert your own work — fix forward from the current \
                 state, and only commit if you need a checkpoint.",
                resolved.display()
            ));
        }
    }
    None
}

/// Path-like argument tokens of a `git checkout` / `git restore` sub-command, across
/// `&&` / `;` / `|`-joined segments. Conservative and NOT a shell parser: it returns
/// every non-flag token after a `checkout`/`restore` subcommand (skipping `-x` flags
/// and a `--` separator marker). Branch names like `main` are harmless — the caller
/// gates on membership in the edited set, which a branch name is never in.
fn restore_path_tokens(command: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for segment in command.split(['&', ';', '|']) {
        let mut toks = segment.split_whitespace();
        // advance to a `git` invocation
        let mut found_git = false;
        let mut in_restore = false;
        for tok in toks.by_ref() {
            if !found_git {
                if tok == "git" {
                    found_git = true;
                }
                continue;
            }
            if !in_restore {
                match tok {
                    "checkout" | "restore" => in_restore = true,
                    // any other subcommand in this segment: stop scanning it
                    _ => break,
                }
                continue;
            }
            // in_restore: collect non-flag, non-`--` tokens as candidate paths
            if tok == "--" || tok.starts_with('-') {
                continue;
            }
            out.push(tok);
        }
    }
    out
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

/// Stable marker prefix for a redundant-read dedupe reference. Used to build the
/// reference and to detect an already-deduped result so a second re-read points
/// at the original live read, never at another reference.
const REDUNDANT_READ_PREFIX: &str = "[already-read:";

/// Find the most recent **live whole-file** `read_file` result for `path` in
/// `messages`. "Live" = the result content is still real — not a phase-04
/// `[superseded:` eviction breadcrumb and not an `[already-read:` reference from a
/// prior dedupe. "Whole-file" = the read call carried no `start_line`/`end_line`
/// (a ranged prior read does not cover the whole file, so it can't stand in for a
/// whole-file re-read). Returns `(turn, content_tokens)` of that prior read —
/// `turn` for the breadcrumb, `content_tokens` (chars/4 estimate, same heuristic
/// as the budget) for the `tokens_saved` calculation — or `None` if no such read
/// is still in context.
///
/// Pure over the slice (no filesystem); the mtime/on-disk check is the caller's
/// (`redundant_read_reference`). Scanning the live `messages` vec *is* the
/// "still in context" test: a read the compactor evicted is removed from
/// `messages`, so it won't be found here.
pub(super) fn last_live_read(
    messages: &[Message],
    path: &Path,
    project_root: &Path,
) -> Option<(usize, usize)> {
    let mut found: Option<(usize, usize)> = None;
    for i in 0..messages.len() {
        // messages[i] must be an assistant whole-file `read_file` call for `path`.
        let args = messages[i]
            .tool_calls
            .as_ref()
            .and_then(|tcs| tcs.first())
            .filter(|tc| tc.name == "read_file")
            .and_then(|tc| serde_json::from_str::<serde_json::Value>(&tc.arguments).ok());
        let Some(args) = args else { continue };
        if args.get("start_line").is_some() || args.get("end_line").is_some() {
            continue; // ranged prior read — does not cover the whole file
        }
        let matches_path = args
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| {
                let pb = std::path::PathBuf::from(p);
                let resolved = if pb.is_absolute() {
                    pb
                } else {
                    project_root.join(pb)
                };
                resolved == path
            })
            .unwrap_or(false);
        if !matches_path {
            continue;
        }
        // The result is on the next (tool) message; it must still be real content.
        if let Some(next) = messages.get(i + 1)
            && let Some(results) = next.tool_results.as_ref()
            && let Some(r) = results.first()
            && r.tool_name == "read_file"
            && !r.content.starts_with(SUPERSEDED_PREFIX)
            && !r.content.starts_with(REDUNDANT_READ_PREFIX)
        {
            let turn = messages[i].turn.unwrap_or(0);
            found = Some((turn, crate::context::tokens::count(&r.content)));
        }
    }
    found
}

/// Decide whether a `read_file` call is a redundant re-read of an unchanged file
/// the model already has in context. Returns `(reference, tokens_saved,
/// prior_turn)` when so — the caller feeds `reference` back as the tool result and
/// emits a `ReadDeduped` event — or `None` to perform the real read.
///
/// Declines (real read) when: the call is ranged (`start_line`/`end_line`) or
/// `force: true` (the model's escape hatches); the file is not in the working set
/// (never read this session); the on-disk mtime no longer matches the recorded
/// one (changed since — incl. untracked `bash` edits); or no live whole-file prior
/// read survives in `messages`. Does its own mtime stat, mirroring
/// `read_before_edit_refusal`, so it is exercised by the loop integration tests.
pub(super) fn redundant_read_reference(
    tool_call: &ToolCall,
    messages: &[Message],
    working_set: &HashMap<PathBuf, SystemTime>,
    project_root: &Path,
) -> Option<(String, usize, usize)> {
    if tool_call.name != "read_file" {
        return None;
    }
    let args = &tool_call.arguments;
    if args.get("start_line").is_some()
        || args.get("end_line").is_some()
        || args.get("force").and_then(|v| v.as_bool()) == Some(true)
    {
        return None;
    }
    let path = resolve_path(tool_call, project_root)?;
    let recorded = working_set.get(&path)?;
    let current = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())?;
    if current != *recorded {
        return None;
    }
    let (prior_turn, prior_tokens) = last_live_read(messages, &path, project_root)?;
    let reference = format!(
        "{REDUNDANT_READ_PREFIX} unchanged since your read at turn {prior_turn}; that content \
         is still above in this conversation — re-read with start_line/end_line or force:true \
         only if you need it again]"
    );
    let tokens_saved = prior_tokens.saturating_sub(crate::context::tokens::count(&reference));
    Some((reference, tokens_saved, prior_turn))
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

    // ── last_live_read tests (pure) ───────────────────────────────────────

    #[test]
    fn last_live_read_finds_prior_whole_file_read() {
        let content = "fn example() {\n    // This is a longer function that does something\n    // interesting and takes up enough space that we can verify tokens > 0.\n    let x = 42;\n    x\n}\n";
        let msgs = make_read_exchange("/r/foo.rs", content);
        let result = last_live_read(&msgs, Path::new("/r/foo.rs"), Path::new("/r"));
        assert!(result.is_some());
        let (turn, tokens) = result.unwrap();
        assert_eq!(turn, 1);
        assert!(
            tokens > 0,
            "tokens should be positive for non-trivial content"
        );
    }

    #[test]
    fn last_live_read_resolves_relative_path() {
        let msgs = make_read_exchange("foo.rs", "relative content");
        let result = last_live_read(&msgs, Path::new("/r/foo.rs"), Path::new("/r"));
        assert!(result.is_some());
    }

    #[test]
    fn last_live_read_skips_superseded_breadcrumb() {
        let content = format!(
            "{SUPERSEDED_PREFIX} file edited at turn 5; this earlier read is stale — \
             re-read with read_file for current content]"
        );
        let msgs = make_read_exchange("/r/foo.rs", &content);
        let result = last_live_read(&msgs, Path::new("/r/foo.rs"), Path::new("/r"));
        assert!(result.is_none(), "superseded breadcrumb is not a live read");
    }

    #[test]
    fn last_live_read_skips_already_read_reference() {
        let content = format!(
            "{REDUNDANT_READ_PREFIX} unchanged since your read at turn 1; that content \
             is still above in this conversation — re-read with start_line/end_line or force:true \
             only if you need it again]"
        );
        let msgs = make_read_exchange("/r/foo.rs", &content);
        let result = last_live_read(&msgs, Path::new("/r/foo.rs"), Path::new("/r"));
        assert!(
            result.is_none(),
            "already-read reference is not a live read"
        );
    }

    #[test]
    fn last_live_read_ignores_ranged_prior_read() {
        // Build a ranged read exchange (start_line present in args)
        let arguments =
            serde_json::to_string(&json!({ "path": "/r/foo.rs", "start_line": 1, "end_line": 10 }))
                .unwrap();
        let msgs = vec![
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
                    content: "ranged content".to_string(),
                }]),
                turn: Some(1),
            },
        ];
        let result = last_live_read(&msgs, Path::new("/r/foo.rs"), Path::new("/r"));
        assert!(
            result.is_none(),
            "a ranged prior read does not cover the whole file"
        );
    }

    #[test]
    fn last_live_read_ignores_other_file() {
        let msgs = make_read_exchange("/r/bar.rs", "bar content");
        let result = last_live_read(&msgs, Path::new("/r/foo.rs"), Path::new("/r"));
        assert!(result.is_none());
    }

    #[test]
    fn last_live_read_returns_latest_of_multiple() {
        let mut msgs = Vec::new();
        // First read at turn 1
        let arguments1 = serde_json::to_string(&json!({ "path": "/r/foo.rs" })).unwrap();
        msgs.push(Message {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![AiToolCall {
                id: "rcall-1".to_string(),
                name: "read_file".to_string(),
                arguments: arguments1,
                thought_signature: None,
            }]),
            tool_results: None,
            turn: Some(1),
        });
        msgs.push(Message {
            role: "tool".to_string(),
            content: String::new(),
            tool_calls: None,
            tool_results: Some(vec![AiToolResult {
                tool_call_id: "rcall-1".to_string(),
                tool_name: "read_file".to_string(),
                content: "first read content".to_string(),
            }]),
            turn: Some(1),
        });
        // Second read at turn 4
        let arguments4 = serde_json::to_string(&json!({ "path": "/r/foo.rs" })).unwrap();
        msgs.push(Message {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![AiToolCall {
                id: "rcall-4".to_string(),
                name: "read_file".to_string(),
                arguments: arguments4,
                thought_signature: None,
            }]),
            tool_results: None,
            turn: Some(4),
        });
        msgs.push(Message {
            role: "tool".to_string(),
            content: String::new(),
            tool_calls: None,
            tool_results: Some(vec![AiToolResult {
                tool_call_id: "rcall-4".to_string(),
                tool_name: "read_file".to_string(),
                content: "second read content".to_string(),
            }]),
            turn: Some(4),
        });

        let result = last_live_read(&msgs, Path::new("/r/foo.rs"), Path::new("/r"));
        assert!(result.is_some());
        let (turn, _) = result.unwrap();
        assert_eq!(
            turn, 4,
            "should return the latest (turn 4), not the first (turn 1)"
        );
    }

    // ── redundant_read_reference tests (TempDir + working_set) ────────────

    fn make_read_call(path: &str) -> ToolCall {
        ToolCall {
            name: "read_file".to_string(),
            arguments: json!({ "path": path }),
            origin: crate::parser::Origin::Native,
        }
    }

    fn make_ranged_read_call(path: &str) -> ToolCall {
        ToolCall {
            name: "read_file".to_string(),
            arguments: json!({ "path": path, "start_line": 1, "end_line": 10 }),
            origin: crate::parser::Origin::Native,
        }
    }

    fn make_force_read_call(path: &str) -> ToolCall {
        ToolCall {
            name: "read_file".to_string(),
            arguments: json!({ "path": path, "force": true }),
            origin: crate::parser::Origin::Native,
        }
    }

    #[test]
    fn redundant_read_reference_dedupes_unchanged_reread() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("foo.rs");
        // Content must be substantially longer than the ~130-char reference string
        // so tokens_saved > 0
        let content = "fn example() {\n    // This is a longer function that does something\n    // interesting and takes up enough space that we can verify tokens > 0.\n    // We need the content to be substantially longer than the dedupe reference\n    // string so that the tokens_saved calculation produces a positive number.\n    // Adding more lines to ensure the content is large enough for the test.\n    let x = 42;\n    let y = x + 1;\n    let z = y * 2;\n    println!(\"result: {}\", z);\n    z\n}\n\nfn another_function() {\n    // Another function to pad the content size well above the reference.\n    let items = vec![1, 2, 3, 4, 5];\n    items.iter().sum::<i32>()\n}\n";
        std::fs::write(&file, content).unwrap();

        let mut working_set = HashMap::new();
        record_mtime(&mut working_set, &file);

        let msgs = make_read_exchange(file.to_string_lossy().as_ref(), content);
        let call = make_read_call(file.to_string_lossy().as_ref());

        let result = redundant_read_reference(&call, &msgs, &working_set, dir.path());
        assert!(result.is_some());
        let (reference, tokens_saved, prior_turn) = result.unwrap();
        assert!(
            reference.starts_with("[already-read:"),
            "reference should start with the dedupe prefix"
        );
        assert!(
            tokens_saved > 0,
            "tokens_saved should be positive for content longer than the reference (got {})",
            tokens_saved
        );
        assert_eq!(prior_turn, 1);
    }

    #[test]
    fn redundant_read_reference_skips_when_mtime_differs() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("foo.rs");
        let content = "some content";
        std::fs::write(&file, content).unwrap();

        let mut working_set = HashMap::new();
        // Deliberately wrong mtime — UNIX_EPOCH cannot equal the file's real mtime
        working_set.insert(file.clone(), std::time::SystemTime::UNIX_EPOCH);

        let msgs = make_read_exchange(file.to_string_lossy().as_ref(), content);
        let call = make_read_call(file.to_string_lossy().as_ref());

        let result = redundant_read_reference(&call, &msgs, &working_set, dir.path());
        assert!(result.is_none(), "mtime mismatch should prevent dedupe");
    }

    #[test]
    fn redundant_read_reference_skips_ranged_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("foo.rs");
        let content = "some content";
        std::fs::write(&file, content).unwrap();

        let mut working_set = HashMap::new();
        record_mtime(&mut working_set, &file);

        let msgs = make_read_exchange(file.to_string_lossy().as_ref(), content);
        let call = make_ranged_read_call(file.to_string_lossy().as_ref());

        let result = redundant_read_reference(&call, &msgs, &working_set, dir.path());
        assert!(result.is_none(), "ranged reads should not be deduped");
    }

    #[test]
    fn redundant_read_reference_skips_force_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("foo.rs");
        let content = "some content";
        std::fs::write(&file, content).unwrap();

        let mut working_set = HashMap::new();
        record_mtime(&mut working_set, &file);

        let msgs = make_read_exchange(file.to_string_lossy().as_ref(), content);
        let call = make_force_read_call(file.to_string_lossy().as_ref());

        let result = redundant_read_reference(&call, &msgs, &working_set, dir.path());
        assert!(result.is_none(), "force:true reads should not be deduped");
    }

    #[test]
    fn redundant_read_reference_none_when_not_in_working_set() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("foo.rs");
        let content = "some content";
        std::fs::write(&file, content).unwrap();

        let working_set: HashMap<PathBuf, SystemTime> = HashMap::new();
        let msgs = make_read_exchange(file.to_string_lossy().as_ref(), content);
        let call = make_read_call(file.to_string_lossy().as_ref());

        let result = redundant_read_reference(&call, &msgs, &working_set, dir.path());
        assert!(
            result.is_none(),
            "first read (not in working set) should not be deduped"
        );
    }

    #[test]
    fn redundant_read_reference_none_when_no_live_prior_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("foo.rs");
        let content = "some content";
        std::fs::write(&file, content).unwrap();

        let mut working_set = HashMap::new();
        record_mtime(&mut working_set, &file);

        // Only prior read is a superseded breadcrumb — nothing live to point at
        let breadcrumb = format!(
            "{SUPERSEDED_PREFIX} file edited at turn 3; this earlier read is stale — \
             re-read with read_file for current content]"
        );
        let msgs = make_read_exchange(file.to_string_lossy().as_ref(), &breadcrumb);
        let call = make_read_call(file.to_string_lossy().as_ref());

        let result = redundant_read_reference(&call, &msgs, &working_set, dir.path());
        assert!(
            result.is_none(),
            "no live prior read → no dedupe (safety gate)"
        );
    }

    // ── destructive_restore_refusal tests ─────────────────────────────────

    fn make_bash_call(command: &str) -> ToolCall {
        ToolCall {
            name: "bash".to_string(),
            arguments: json!({ "command": command }),
            origin: crate::parser::Origin::Native,
        }
    }

    fn make_patch_call() -> ToolCall {
        ToolCall {
            name: "patch".to_string(),
            arguments: json!({ "path": "foo.rs", "old_str": "a", "new_str": "b" }),
            origin: crate::parser::Origin::Native,
        }
    }

    #[test]
    fn refuses_checkout_of_edited_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut edited = HashMap::new();
        let file = dir.path().join("src/x.ts");
        edited.insert(file, Some("old".to_string()));

        let call = make_bash_call("git checkout src/x.ts");
        let result = destructive_restore_refusal(&call, &edited, dir.path());
        assert!(result.is_some());
        let msg = result.unwrap();
        assert!(
            msg.contains("src/x.ts"),
            "message should name the file: {msg}"
        );
    }

    #[test]
    fn refuses_restore_of_edited_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut edited = HashMap::new();
        let file = dir.path().join("src/x.ts");
        edited.insert(file, Some("old".to_string()));

        let call = make_bash_call("git restore src/x.ts");
        let result = destructive_restore_refusal(&call, &edited, dir.path());
        assert!(result.is_some());
    }

    #[test]
    fn refuses_checkout_head_dashdash_form() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut edited = HashMap::new();
        let file = dir.path().join("src/x.ts");
        edited.insert(file, Some("old".to_string()));

        let call = make_bash_call("git checkout HEAD -- src/x.ts");
        let result = destructive_restore_refusal(&call, &edited, dir.path());
        assert!(
            result.is_some(),
            "HEAD and -- should be skipped, src/x.ts should match"
        );
    }

    #[test]
    fn allows_checkout_of_unedited_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut edited = HashMap::new();
        let file = dir.path().join("src/x.ts");
        edited.insert(file, Some("old".to_string()));

        let call = make_bash_call("git checkout src/y.ts");
        let result = destructive_restore_refusal(&call, &edited, dir.path());
        assert!(result.is_none(), "unedited file should be allowed");
    }

    #[test]
    fn allows_branch_switch() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut edited = HashMap::new();
        let file = dir.path().join("src/x.ts");
        edited.insert(file, Some("old".to_string()));

        let call = make_bash_call("git checkout -b feature");
        let result = destructive_restore_refusal(&call, &edited, dir.path());
        assert!(result.is_none(), "branch creation should be allowed");

        let call2 = make_bash_call("git checkout main");
        let result2 = destructive_restore_refusal(&call2, &edited, dir.path());
        assert!(result2.is_none(), "branch switch should be allowed");
    }

    #[test]
    fn ignores_non_bash_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut edited = HashMap::new();
        let file = dir.path().join("src/x.ts");
        edited.insert(file, Some("old".to_string()));

        let call = make_patch_call();
        let result = destructive_restore_refusal(&call, &edited, dir.path());
        assert!(result.is_none(), "non-bash calls should be ignored");
    }

    #[test]
    fn refuses_in_compound_command() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut edited = HashMap::new();
        let file = dir.path().join("src/x.ts");
        edited.insert(file, Some("old".to_string()));

        let call = make_bash_call("npm test && git checkout src/x.ts");
        let result = destructive_restore_refusal(&call, &edited, dir.path());
        assert!(
            result.is_some(),
            "compound command should find the restore segment"
        );
    }

    // ── read_before_edit_refusal: write_file gate tests ───────────────────

    fn make_write_file_call(path: &str, append: bool) -> ToolCall {
        let mut args = serde_json::Map::new();
        args.insert("path".into(), path.into());
        if append {
            args.insert("append".into(), serde_json::Value::Bool(true));
        }
        ToolCall {
            name: "write_file".to_string(),
            arguments: serde_json::Value::Object(args),
            origin: crate::parser::Origin::Native,
        }
    }

    fn make_patch_call_for_path(path: &str) -> ToolCall {
        ToolCall {
            name: "patch".to_string(),
            arguments: json!({ "path": path, "old_str": "x", "new_str": "y" }),
            origin: crate::parser::Origin::Native,
        }
    }

    #[test]
    fn write_file_create_is_allowed_without_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("new.txt");
        let path = file.to_string_lossy().to_string();
        let call = make_write_file_call(&path, false);
        let working_set: HashMap<PathBuf, SystemTime> = HashMap::new();
        let result = read_before_edit_refusal(&call, &working_set, dir.path());
        assert!(
            result.is_none(),
            "create of a new file should be allowed without read"
        );
    }

    #[test]
    fn write_file_append_is_allowed_without_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("existing.txt");
        std::fs::write(&file, "original").unwrap();
        let path = file.to_string_lossy().to_string();
        let call = make_write_file_call(&path, true);
        let working_set: HashMap<PathBuf, SystemTime> = HashMap::new();
        let result = read_before_edit_refusal(&call, &working_set, dir.path());
        assert!(
            result.is_none(),
            "append to an existing file should be allowed without read"
        );
    }

    #[test]
    fn write_file_overwrite_of_unread_existing_file_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("existing.txt");
        std::fs::write(&file, "original").unwrap();
        let path = file.to_string_lossy().to_string();
        let call = make_write_file_call(&path, false);
        let working_set: HashMap<PathBuf, SystemTime> = HashMap::new();
        let result = read_before_edit_refusal(&call, &working_set, dir.path());
        assert!(
            result.is_some(),
            "overwrite of an unread existing file should be refused"
        );
        let msg = result.unwrap();
        assert!(
            msg.contains("overwrite"),
            "refusal message should contain 'overwrite', got: {msg}"
        );
        assert!(
            msg.contains(&*file.file_name().unwrap().to_string_lossy()),
            "refusal message should contain the path"
        );
    }

    #[test]
    fn write_file_overwrite_after_read_is_allowed() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("existing.txt");
        std::fs::write(&file, "original").unwrap();
        let path = file.to_string_lossy().to_string();
        let call = make_write_file_call(&path, false);
        let mut working_set = HashMap::new();
        record_mtime(&mut working_set, &file);
        let result = read_before_edit_refusal(&call, &working_set, dir.path());
        assert!(
            result.is_none(),
            "overwrite after read (with matching mtime) should be allowed"
        );
    }

    #[test]
    fn write_file_overwrite_with_changed_mtime_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("existing.txt");
        std::fs::write(&file, "original").unwrap();
        let path = file.to_string_lossy().to_string();
        let call = make_write_file_call(&path, false);
        let mut working_set = HashMap::new();
        // Deliberately wrong mtime (UNIX_EPOCH) so the file appears changed
        working_set.insert(file.clone(), SystemTime::UNIX_EPOCH);
        let result = read_before_edit_refusal(&call, &working_set, dir.path());
        assert!(
            result.is_some(),
            "overwrite with stale mtime should be refused"
        );
        let msg = result.unwrap();
        assert!(
            msg.contains("changed on disk"),
            "refusal message should contain 'changed on disk', got: {msg}"
        );
    }

    #[test]
    fn patch_of_unread_file_still_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("existing.txt");
        std::fs::write(&file, "original").unwrap();
        let path = file.to_string_lossy().to_string();
        let call = make_patch_call_for_path(&path);
        let working_set: HashMap<PathBuf, SystemTime> = HashMap::new();
        let result = read_before_edit_refusal(&call, &working_set, dir.path());
        assert!(
            result.is_some(),
            "patch of an unread file should still be refused"
        );
        let msg = result.unwrap();
        assert!(
            msg.contains("patch"),
            "refusal message should contain 'patch', got: {msg}"
        );
    }
}
