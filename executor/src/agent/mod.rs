//! The core `execute_phase` turn loop (turn-cycle steps 1–5). Drives the local
//! model through budget-bounded turns — chat → drain events → native-or-parsed
//! `ToolCall` → dispatch → score — terminating `complete` (model stops calling
//! tools) or `budget_exceeded` (turn/context exhaustion). Session log, verifier
//! retry, hard-fail detection, and completion artifacts are later sub-phases.

pub mod prompt;
pub mod verify;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use tokio::sync::mpsc;

use crate::ai::AiClient;
use crate::ai::next_tool_id;
use crate::ai::types::{
    AiEvent, Message, ToolCall as AiToolCall, ToolResult as AiToolResult, ToolSchema,
};
use crate::context::budget::Budget;
use crate::context::compactor::compact;
use crate::error::{Error, Result};
use crate::governor::hard_fail::{HardFailSignal, ToolCallSnapshot, evaluate};
use crate::governor::scorer::Scorer;
use crate::governor::verifier::{Baseline, Diagnostic, Severity, VerifierResult};
use crate::parser::{Origin, ParseResult, ToolCall, parse};
use crate::phase::{
    Artifacts, Blocker, Briefing, CommandOutputs, PhaseResult, collect_working_files,
    summarize_attempts,
};
use crate::security::redact::Redactor;
use crate::store::sessions::event::SessionEvent;
use crate::store::sessions::jsonl::{SessionLogHandle, open_session_log, session_log};
use crate::tools::ToolRegistry;
use verify::FileVerifier;

/// Preview cap for a tool result's `output_preview` in the session log — enough
/// to triage a failure, not the full (possibly huge) output.
const OUTPUT_PREVIEW_CHARS: usize = 500;

/// The prompt inputs and verbatim phase metadata the loop assembles into the
/// system prompt and the escalation briefing.
pub struct PhaseInput {
    pub executor_contract: String,
    pub standards: String,
    pub phase_doc: String,
    pub goal: String,
    pub acceptance_criteria: String,
    /// Short phase identifier (e.g. `"phase-07b"`) — used for the `SessionStart`
    /// record and the session-log filename.
    pub phase: String,
}

/// The injected dependencies the loop drives — explicit, no globals. The `clock`
/// is injected (no real `Utc::now()`); session logging reads its `ts` from it.
pub struct LoopDeps<'a> {
    pub client: &'a dyn AiClient,
    pub registry: &'a ToolRegistry,
    pub tools: &'a [ToolSchema],
    pub budget: &'a Budget,
    pub max_turns: usize,
    pub project_root: &'a Path,
    /// Model identifier, for the `SessionStart` record.
    pub model: &'a str,
    /// Caller-provided session id (M5 uses `generate_session_id()`); the loop
    /// never generates it.
    pub session_id: &'a str,
    /// Epoch-millis clock for session-log record timestamps.
    pub clock: &'a dyn Fn() -> u64,
    /// Post-edit verifier (injected so tests need not spawn a real compiler).
    pub verifier: &'a dyn FileVerifier,
}

/// Run the turn cycle until the model stops calling tools (`complete`) or the
/// turn/context budget is exhausted (`budget_exceeded`). Backend/infra failures
/// surface as `Err`; model-visible outcomes (parse failures, unknown/failed
/// tools) are fed back into the conversation and never error.
pub async fn execute_phase(input: &PhaseInput, deps: LoopDeps<'_>) -> Result<PhaseResult> {
    let system = prompt::assemble_system_prompt(
        &input.executor_contract,
        &input.standards,
        &input.phase_doc,
    );
    let tools_opt = if deps.tools.is_empty() {
        None
    } else {
        Some(deps.tools)
    };

    let mut messages: Vec<Message> = Vec::new();
    let mut scorer = Scorer::new();
    let mut recent_tool_calls: VecDeque<ToolCallSnapshot> = VecDeque::new();
    let mut turns: usize = 0;

    // Governor feedback state (07c).
    let mut baseline = Baseline::new();
    let mut baselined_exts: HashSet<String> = HashSet::new();
    let mut recent_verifier_error_counts: Vec<usize> = Vec::new();
    let mut last_author_diagnostics: Vec<Diagnostic> = Vec::new();

    // Step 1 (observability) — open the session log. Best-effort: `.ok()` drops a
    // setup failure on purpose (a non-writable repo must not fail the phase —
    // logging is a side effect that never changes what the loop returns). The
    // composed id puts both phase and session_id in the filename.
    let redactor = Redactor::new();
    let log_dir = deps.project_root.join(".rexymcp").join("sessions");
    let log_handle: Option<SessionLogHandle> =
        open_session_log(&log_dir, &format!("{}-{}", input.phase, deps.session_id)).ok();

    log_event(
        &log_handle,
        &redactor,
        deps.clock,
        0,
        SessionEvent::SessionStart {
            session_id: deps.session_id.to_string(),
            model: deps.model.to_string(),
            phase: input.phase.clone(),
        },
    );
    log_event(
        &log_handle,
        &redactor,
        deps.clock,
        0,
        SessionEvent::Prompt {
            rendered: system.clone(),
        },
    );

    loop {
        // Step 2 — budget: compact on overflow, give up if still over.
        if deps.budget.would_overflow(&system, &messages) {
            compact(&mut messages, deps.budget, &system);
            if deps.budget.would_overflow(&system, &messages) {
                log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
                return Ok(budget_exceeded_result(
                    input,
                    &recent_tool_calls,
                    deps.project_root,
                    "context budget exhausted".to_string(),
                    turns,
                ));
            }
        }

        // Step 3 — call the model and drain its event stream for this turn.
        let (tx, mut rx) = mpsc::unbounded_channel::<AiEvent>();
        deps.client
            .chat(&system, messages.clone(), tx, tools_opt)
            .await
            .map_err(|e| Error::Backend(e.to_string()))?;

        let mut completion = String::new();
        let mut native_call: Option<ToolCall> = None;
        while let Some(event) = rx.recv().await {
            match event {
                AiEvent::Token(s) => completion.push_str(&s),
                AiEvent::ToolCallGeneric { name, args, .. } => {
                    if native_call.is_none() {
                        native_call = Some(ToolCall {
                            name,
                            arguments: args,
                            origin: Origin::Native,
                        });
                    }
                }
                AiEvent::Done(_) => {}
                AiEvent::Error(e) => {
                    log_session_end(&log_handle, &redactor, deps.clock, "error", turns);
                    return Err(Error::Backend(e));
                }
            }
        }
        turns += 1;
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            turns,
            SessionEvent::Completion {
                raw: completion.clone(),
            },
        );

        // Step 4 — turn the output into a ToolCall (native event wins; otherwise
        // run the forgiving text parser).
        let tool_call = if let Some(tc) = native_call {
            tc
        } else {
            match parse(&completion, deps.registry) {
                ParseResult::NoToolCall => {
                    log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
                    return Ok(complete_result(turns));
                }
                ParseResult::Found(tc) => tc,
                ParseResult::Failed(failure) => {
                    log_event(
                        &log_handle,
                        &redactor,
                        deps.clock,
                        turns,
                        SessionEvent::ParseFailed {
                            failure: failure.clone(),
                        },
                    );
                    messages.push(assistant_text(&completion, turns));
                    messages.push(user_text(&failure.feedback, turns));
                    if turns >= deps.max_turns {
                        log_session_end(
                            &log_handle,
                            &redactor,
                            deps.clock,
                            "budget_exceeded",
                            turns,
                        );
                        return Ok(budget_exceeded_result(
                            input,
                            &recent_tool_calls,
                            deps.project_root,
                            turns_line(deps.max_turns),
                            turns,
                        ));
                    }
                    continue;
                }
            }
        };
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            turns,
            SessionEvent::Parsed {
                tool_call: tool_call.clone(),
            },
        );

        // An edit-class call's target path — resolved here (pre-dispatch) so the
        // baseline can be captured *before* the model's edit lands. Otherwise
        // `capture_baseline` would record the model's own new errors as ambient.
        let edit_path = edit_target(&tool_call, deps.project_root);
        if let Some(path) = &edit_path
            && let Some(ext) = path.extension().and_then(|e| e.to_str())
            && !baselined_exts.contains(ext)
        {
            let captured = deps
                .verifier
                .capture_baseline(std::slice::from_ref(path))
                .await;
            baseline.signatures.extend(captured.signatures);
            baselined_exts.insert(ext.to_string());
        }

        // Step 5 — dispatch (native and text share this path) and record.
        let (succeeded, content) = dispatch(deps.registry, &tool_call).await;
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            turns,
            SessionEvent::ToolResult {
                name: tool_call.name.clone(),
                succeeded,
                output_preview: output_preview(&content),
            },
        );
        scorer.record(&tool_call.name, succeeded);
        recent_tool_calls.push_back(ToolCallSnapshot {
            tool: tool_call.name.clone(),
            arguments: tool_call.arguments.clone(),
            succeeded,
        });
        append_tool_exchange(&mut messages, &tool_call, &content, turns);

        // Step 6 — post-edit verify + retry feedback. Only for a successful
        // edit-class call (verifying after a failed edit is noise).
        if succeeded && let Some(path) = &edit_path {
            match deps.verifier.verify(path).await {
                VerifierResult::Checked { diagnostics } => {
                    let (author, _ambient) = baseline.partition(&diagnostics);
                    let author: Vec<Diagnostic> = author.into_iter().cloned().collect();
                    log_event(
                        &log_handle,
                        &redactor,
                        deps.clock,
                        turns,
                        SessionEvent::Verify {
                            diagnostics: author.clone(),
                        },
                    );
                    recent_verifier_error_counts.push(author.len());
                    if author.is_empty() {
                        last_author_diagnostics.clear();
                    } else {
                        messages.push(user_text(&render_diagnostics(&author), turns));
                        last_author_diagnostics = author;
                    }
                }
                VerifierResult::Unsupported => {}
                VerifierResult::Failed(msg) => {
                    messages.push(user_text(&format!("verifier failed: {msg}"), turns));
                }
            }
        }

        // Step 7 — hard-fail detection (repetition / persistent verifier failure /
        // runaway output). Checked before the turn cap so the specific cause wins.
        if let Some(signal) = evaluate(
            &recent_tool_calls,
            &recent_verifier_error_counts,
            Some((&tool_call.name, content.len())),
        ) {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::HardFail {
                    reason: signal.describe(),
                },
            );
            log_session_end(&log_handle, &redactor, deps.clock, "hard_fail", turns);
            return Ok(hard_fail_result(
                input,
                &recent_tool_calls,
                deps.project_root,
                last_author_diagnostics,
                signal,
                turns,
            ));
        }

        // Step 8 — turn cap.
        if turns >= deps.max_turns {
            log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
            return Ok(budget_exceeded_result(
                input,
                &recent_tool_calls,
                deps.project_root,
                turns_line(deps.max_turns),
                turns,
            ));
        }
    }
}

/// Redact an event (round-tripping its JSON through the redactor so every string
/// field is covered) and write it best-effort. A `None` handle (the log failed to
/// open) is a silent no-op — logging never changes loop behavior.
fn log_event(
    handle: &Option<SessionLogHandle>,
    redactor: &Redactor,
    clock: &dyn Fn() -> u64,
    turn: usize,
    event: SessionEvent,
) {
    let Some(handle) = handle else {
        return;
    };
    session_log(handle, clock(), turn, redact_event(redactor, event));
}

fn log_session_end(
    handle: &Option<SessionLogHandle>,
    redactor: &Redactor,
    clock: &dyn Fn() -> u64,
    status: &str,
    turns: usize,
) {
    log_event(
        handle,
        redactor,
        clock,
        turns,
        SessionEvent::SessionEnd {
            status: status.to_string(),
            turns,
        },
    );
}

/// Round-trip an event through the redactor: serialize → redact the JSON →
/// deserialize. This redacts every string the event carries (prompt, completion,
/// tool output, the nested `ParseFailure` / `ToolCall` payloads) in one pass; the
/// `[REDACTED:<kind>]` markers are JSON-safe, so the parse round-trips. On the
/// can't-happen serde failure, fall back to the un-redacted event's structure
/// only after redaction was attempted — but serialization of these types is
/// effectively infallible, so this is a safety net, not a swallow.
fn redact_event(redactor: &Redactor, event: SessionEvent) -> SessionEvent {
    let Ok(json) = serde_json::to_string(&event) else {
        return event;
    };
    let redacted = redactor.redact(&json);
    serde_json::from_str(&redacted).unwrap_or(event)
}

fn output_preview(content: &str) -> String {
    if content.chars().count() > OUTPUT_PREVIEW_CHARS {
        content.chars().take(OUTPUT_PREVIEW_CHARS).collect()
    } else {
        content.to_string()
    }
}

/// The file an edit-class (`write_file` / `patch`) call targets, resolved against
/// the project root. `None` for non-edit calls or calls missing a `"path"` arg.
fn edit_target(tool_call: &ToolCall, project_root: &Path) -> Option<PathBuf> {
    if tool_call.name != "write_file" && tool_call.name != "patch" {
        return None;
    }
    let path = tool_call.arguments.get("path").and_then(|v| v.as_str())?;
    let path = PathBuf::from(path);
    Some(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

/// Render author diagnostics into a retry message the model can act on.
fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
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

fn hard_fail_result(
    input: &PhaseInput,
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
    diagnostics: Vec<Diagnostic>,
    signal: HardFailSignal,
    turns: usize,
) -> PhaseResult {
    let briefing = Briefing {
        goal: input.goal.clone(),
        acceptance_criteria: input.acceptance_criteria.clone(),
        diagnostics,
        working_files: collect_working_files(recent_tool_calls, project_root),
        what_was_tried: summarize_attempts(recent_tool_calls),
        current_blocker: Blocker::HardFail(signal),
        budget_remaining: format!("halted on hard-fail at turn {turns}"),
    };
    PhaseResult::hard_fail(briefing, empty_artifacts("hard_fail", turns))
}

/// Dispatch a tool call through the registry. Returns `(succeeded, content)`
/// where `content` is the message fed back to the model. A missing tool or an
/// execution error is a model-visible failure, not an `Err`.
async fn dispatch(registry: &ToolRegistry, tc: &ToolCall) -> (bool, String) {
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

fn append_tool_exchange(messages: &mut Vec<Message>, tc: &ToolCall, content: &str, turn: usize) {
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

fn assistant_text(content: &str, turn: usize) -> Message {
    Message {
        role: "assistant".to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_results: None,
        turn: Some(turn),
    }
}

fn user_text(content: &str, turn: usize) -> Message {
    Message {
        role: "user".to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_results: None,
        turn: Some(turn),
    }
}

fn turns_line(max_turns: usize) -> String {
    format!("0 of {max_turns} turns remaining")
}

fn empty_artifacts(status: &str, turns: usize) -> Artifacts {
    Artifacts {
        files_changed: Vec::new(),
        diff: String::new(),
        command_outputs: CommandOutputs::default(),
        update_log: format!("Executor run: {status} after {turns} turn(s)."),
    }
}

fn complete_result(turns: usize) -> PhaseResult {
    PhaseResult::complete(empty_artifacts("complete", turns))
}

fn budget_exceeded_result(
    input: &PhaseInput,
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
    budget_remaining: String,
    turns: usize,
) -> PhaseResult {
    let briefing = Briefing {
        goal: input.goal.clone(),
        acceptance_criteria: input.acceptance_criteria.clone(),
        diagnostics: Vec::new(),
        working_files: collect_working_files(recent_tool_calls, project_root),
        what_was_tried: summarize_attempts(recent_tool_calls),
        current_blocker: Blocker::BudgetExceeded,
        budget_remaining,
    };
    PhaseResult::budget_exceeded(briefing, empty_artifacts("budget_exceeded", turns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::testing::MockAiClientScript;
    use crate::ai::types::TokenBreakdown;
    use crate::phase::PhaseStatus;
    use crate::security::scope::Scope;
    use crate::tools::{read_file, write_file};
    use serde_json::json;
    use tempfile::TempDir;

    fn registry_over(scope: Scope) -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(read_file(scope.clone()));
        r.register(write_file(scope));
        r
    }

    const SESSION_ID: &str = "testsid";
    const PHASE_SLUG: &str = "phase-07b";

    fn clock_zero() -> u64 {
        0
    }

    fn input() -> PhaseInput {
        PhaseInput {
            executor_contract: "CONTRACT".to_string(),
            standards: "STANDARDS".to_string(),
            phase_doc: "PHASE".to_string(),
            goal: "make it compile".to_string(),
            acceptance_criteria: "cargo build passes".to_string(),
            phase: PHASE_SLUG.to_string(),
        }
    }

    /// A verifier that is never expected to fire (existing non-edit tests). If an
    /// edit-class call ever reaches it, `verify` returns `Unsupported` so it stays
    /// inert rather than spawning a real compiler.
    struct NoopVerifier;

    #[async_trait::async_trait]
    impl FileVerifier for NoopVerifier {
        async fn verify(&self, _path: &Path) -> VerifierResult {
            VerifierResult::Unsupported
        }
        async fn capture_baseline(&self, _paths: &[PathBuf]) -> Baseline {
            Baseline::new()
        }
    }

    fn deps<'a>(
        client: &'a dyn AiClient,
        registry: &'a ToolRegistry,
        budget: &'a Budget,
        max_turns: usize,
        root: &'a Path,
    ) -> LoopDeps<'a> {
        LoopDeps {
            client,
            registry,
            tools: &[],
            budget,
            max_turns,
            project_root: root,
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier: &NoopVerifier,
        }
    }

    /// The on-disk log path for a run driven by `deps()` over `root`.
    fn log_path(root: &Path) -> std::path::PathBuf {
        root.join(".rexymcp")
            .join("sessions")
            .join(format!("session-{PHASE_SLUG}-{SESSION_ID}.jsonl"))
    }

    fn token(s: &str) -> AiEvent {
        AiEvent::Token(s.to_string())
    }

    fn native(name: &str, args: serde_json::Value) -> AiEvent {
        AiEvent::ToolCallGeneric {
            id: "tc_x".to_string(),
            name: name.to_string(),
            args,
            thought_signature: None,
        }
    }

    #[tokio::test]
    async fn no_tool_call_first_turn_completes_immediately() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("All done, nothing to call.")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        assert!(result.briefing.is_none());
        assert_eq!(client.calls().len(), 1);
    }

    #[tokio::test]
    async fn complete_result_has_no_briefing() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert!(result.briefing.is_none());
    }

    #[tokio::test]
    async fn tool_call_then_no_tool_call_completes() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("now I'm done")],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        assert_eq!(client.calls().len(), 2);
    }

    #[tokio::test]
    async fn native_tool_call_event_dispatches_as_origin_native() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // Always emit a native call; cap at 1 turn so we can inspect the snapshot.
        let client =
            MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        // The native call was dispatched (it succeeded reading the file), recorded
        // in what_was_tried via the snapshot path.
        assert_eq!(result.status, PhaseStatus::BudgetExceeded);
        let briefing = result.briefing.unwrap();
        assert_eq!(briefing.what_was_tried.len(), 1);
        assert!(briefing.what_was_tried[0].one_line.contains("read_file"));
        assert!(briefing.what_was_tried[0].one_line.contains("succeeded"));
    }

    #[tokio::test]
    async fn native_tool_call_skips_text_parser() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // Completion text contains a *different* (malformed-name) call; the native
        // event must win and the text must not be parsed/dispatched.
        let client = MockAiClientScript::new(vec![vec![
            token("{\"name\":\"write_file\",\"arguments\":{}}"),
            native("read_file", json!({ "path": path })),
        ]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        let briefing = result.briefing.unwrap();
        assert_eq!(briefing.what_was_tried.len(), 1);
        assert!(briefing.what_was_tried[0].one_line.contains("read_file"));
    }

    #[tokio::test]
    async fn native_unknown_tool_feeds_failure_not_err() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![
            vec![native("does_not_exist", json!({}))],
            vec![token("giving up")],
        ]);
        let budget = Budget::new(1_000_000);

        // Returns Ok (model-visible failure), reaches Complete on the next turn.
        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        // The unknown-tool feedback was placed in the conversation the second call saw.
        let second_call_messages = &client.calls()[1].messages;
        let has_unknown = second_call_messages.iter().any(|m| {
            m.tool_results
                .as_ref()
                .map(|trs| trs.iter().any(|t| t.content.contains("unknown tool")))
                .unwrap_or(false)
        });
        assert!(
            has_unknown,
            "expected an unknown-tool failure fed back to the model"
        );
    }

    #[tokio::test]
    async fn text_tool_call_is_parsed_and_dispatched() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let hermes = format!(
            "<tool_call>{{\"name\":\"read_file\",\"arguments\":{{\"path\":\"{}\"}}}}</tool_call>",
            path.replace('\\', "\\\\")
        );
        let client = MockAiClientScript::new(vec![vec![token(&hermes)]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        let briefing = result.briefing.unwrap();
        assert_eq!(briefing.what_was_tried.len(), 1);
        assert!(briefing.what_was_tried[0].one_line.contains("read_file"));
    }

    #[tokio::test]
    async fn parse_failure_feeds_feedback_and_continues() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        // First turn: a malformed call (unknown tool name, valid JSON) → Failed.
        // Second turn: plain text → Complete.
        let client = MockAiClientScript::new(vec![
            vec![token("{\"name\":\"nonexistent\",\"arguments\":{}}")],
            vec![token("ok, stopping")],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        // The second call's messages include a user message carrying parser feedback.
        let second = &client.calls()[1].messages;
        let has_user_feedback = second
            .iter()
            .any(|m| m.role == "user" && !m.content.is_empty());
        assert!(
            has_user_feedback,
            "expected parser feedback fed back as a user message"
        );
    }

    #[tokio::test]
    async fn turn_cap_returns_budget_exceeded_with_briefing() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path.clone() }))],
            vec![native("read_file", json!({ "path": path.clone() }))],
            vec![native("read_file", json!({ "path": path }))],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 2, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::BudgetExceeded);
        let briefing = result.briefing.unwrap();
        assert!(matches!(briefing.current_blocker, Blocker::BudgetExceeded));
        assert!(!briefing.what_was_tried.is_empty());
    }

    #[tokio::test]
    async fn budget_briefing_carries_goal_and_attempts() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client =
            MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        let briefing = result.briefing.unwrap();
        assert_eq!(briefing.goal, "make it compile");
        assert!(!briefing.what_was_tried.is_empty());
    }

    #[tokio::test]
    async fn budget_overflow_after_compaction_returns_budget_exceeded() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        // A ceiling of 1 token — the system prompt alone overflows and cannot be
        // compacted away (system is never evicted), so the loop gives up before
        // ever calling the model.
        let client = MockAiClientScript::new(vec![vec![token("unused")]]);
        let budget = Budget::new(1);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::BudgetExceeded);
        assert_eq!(
            client.calls().len(),
            0,
            "must not call the model when over budget"
        );
        assert!(result.briefing.is_some());
    }

    #[tokio::test]
    async fn budget_with_headroom_runs_without_compaction() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
    }

    #[tokio::test]
    async fn tool_outcomes_distinguish_success_and_failure() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("does_not_exist", json!({}))],
            vec![native("read_file", json!({ "path": path }))],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 2, dir.path()))
            .await
            .unwrap();

        let tried = result.briefing.unwrap().what_was_tried;
        assert_eq!(tried.len(), 2);
        assert!(tried[0].one_line.contains("failed"));
        assert!(tried[1].one_line.contains("succeeded"));
    }

    #[tokio::test]
    async fn ai_event_error_propagates_as_err() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![
            token("partial"),
            AiEvent::Error("backend exploded".to_string()),
            AiEvent::Done(TokenBreakdown::default()),
        ]]);
        let budget = Budget::new(1_000_000);

        let result =
            execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path())).await;

        assert!(
            result.is_err(),
            "AiEvent::Error must surface as Err, not a PhaseResult"
        );
    }

    // ── 07b: session log ──────────────────────────────────────────────────

    use crate::store::sessions::jsonl::read_session_log;

    fn records(root: &Path) -> Vec<crate::store::sessions::event::SessionRecord> {
        read_session_log(&log_path(root)).unwrap()
    }

    #[tokio::test]
    async fn creates_log_file_named_with_phase_and_session_id() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let path = log_path(dir.path());
        assert!(path.exists(), "log file not created");
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.contains(PHASE_SLUG) && name.contains(SESSION_ID));
    }

    #[tokio::test]
    async fn logs_session_start_first_then_prompt() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let recs = records(dir.path());
        assert!(matches!(recs[0].event, SessionEvent::SessionStart { .. }));
        assert!(matches!(recs[1].event, SessionEvent::Prompt { .. }));
        match &recs[0].event {
            SessionEvent::SessionStart {
                session_id,
                model,
                phase,
            } => {
                assert_eq!(session_id, SESSION_ID);
                assert_eq!(model, "test-model");
                assert_eq!(phase, PHASE_SLUG);
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn logs_completion_parsed_and_tool_result_for_dispatched_turn() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("done")],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let kinds: Vec<&str> = records(dir.path())
            .iter()
            .map(|r| event_kind(&r.event))
            .collect();
        // SessionStart, Prompt, then turn 1: Completion, Parsed, ToolResult, then
        // turn 2 Completion, then SessionEnd.
        assert_eq!(kinds[0], "session_start");
        assert_eq!(kinds[1], "prompt");
        assert_eq!(kinds[2], "completion");
        assert_eq!(kinds[3], "parsed");
        assert_eq!(kinds[4], "tool_result");
        assert_eq!(*kinds.last().unwrap(), "session_end");
    }

    #[tokio::test]
    async fn logs_parse_failed_for_malformed_turn() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![
            vec![token("{\"name\":\"nonexistent\",\"arguments\":{}}")],
            vec![token("stopping")],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert!(
            records(dir.path())
                .iter()
                .any(|r| matches!(r.event, SessionEvent::ParseFailed { .. })),
            "expected a ParseFailed event"
        );
    }

    #[tokio::test]
    async fn logs_session_end_complete_on_clean_finish() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let recs = records(dir.path());
        match &recs.last().unwrap().event {
            SessionEvent::SessionEnd { status, turns } => {
                assert_eq!(status, "complete");
                assert_eq!(*turns, 1);
            }
            other => panic!("expected SessionEnd, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn logs_session_end_budget_exceeded_on_turn_cap() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path.clone() }))],
            vec![native("read_file", json!({ "path": path }))],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        match &records(dir.path()).last().unwrap().event {
            SessionEvent::SessionEnd { status, .. } => assert_eq!(status, "budget_exceeded"),
            other => panic!("expected SessionEnd, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redacts_secret_in_tool_output_before_writing() {
        const SECRET: &str = "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("creds.txt"), SECRET).unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("creds.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("done")],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let raw = std::fs::read_to_string(log_path(dir.path())).unwrap();
        assert!(!raw.contains(SECRET), "secret leaked into the session log");
        assert!(
            raw.contains("[REDACTED:openai_key]"),
            "redaction marker missing"
        );
    }

    #[tokio::test]
    async fn redacts_secret_in_completion_before_writing() {
        const SECRET: &str = "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client =
            MockAiClientScript::new(vec![vec![token(&format!("here is the key {SECRET} ok"))]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let raw = std::fs::read_to_string(log_path(dir.path())).unwrap();
        assert!(!raw.contains(SECRET), "secret leaked via completion");
        assert!(raw.contains("[REDACTED:openai_key]"));
    }

    #[tokio::test]
    async fn logging_failure_does_not_change_result() {
        let dir = TempDir::new().unwrap();
        // Pre-create `.rexymcp` as a *file* so the sessions dir cannot be created
        // and the log fails to open — the run must still complete normally.
        std::fs::write(dir.path().join(".rexymcp"), "not a dir").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        assert!(!log_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn injected_clock_sets_record_ts() {
        const TS: u64 = 1_717_000_000_000;
        fn clock_fixed() -> u64 {
            TS
        }
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);
        let d = LoopDeps {
            client: &client,
            registry: &registry,
            tools: &[],
            budget: &budget,
            max_turns: 8,
            project_root: dir.path(),
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_fixed,
            verifier: &NoopVerifier,
        };

        execute_phase(&input(), d).await.unwrap();

        let recs = records(dir.path());
        assert!(!recs.is_empty());
        assert!(recs.iter().all(|r| r.ts == TS));
    }

    fn event_kind(event: &SessionEvent) -> &'static str {
        match event {
            SessionEvent::SessionStart { .. } => "session_start",
            SessionEvent::Prompt { .. } => "prompt",
            SessionEvent::Completion { .. } => "completion",
            SessionEvent::Parsed { .. } => "parsed",
            SessionEvent::ParseFailed { .. } => "parse_failed",
            SessionEvent::ToolResult { .. } => "tool_result",
            SessionEvent::Verify { .. } => "verify",
            SessionEvent::HardFail { .. } => "hard_fail",
            SessionEvent::Progress { .. } => "progress",
            SessionEvent::SessionEnd { .. } => "session_end",
        }
    }

    // ── 07c: verifier retry + hard-fail ───────────────────────────────────

    use crate::governor::verifier::{Baseline as Bl, Diagnostic, Severity};
    use std::sync::Mutex;

    /// Verifier mock: pops a scripted `VerifierResult` per `verify` call (an
    /// exhausted script yields `Unsupported`), returns a configured baseline, and
    /// records the paths it was asked to verify.
    struct MockFileVerifier {
        results: Mutex<VecDeque<VerifierResult>>,
        baseline: Bl,
        verified: Mutex<Vec<PathBuf>>,
    }

    impl MockFileVerifier {
        fn new(results: Vec<VerifierResult>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
                baseline: Bl::new(),
                verified: Mutex::new(Vec::new()),
            }
        }

        fn with_baseline(mut self, baseline: Bl) -> Self {
            self.baseline = baseline;
            self
        }

        fn verified_paths(&self) -> Vec<PathBuf> {
            self.verified.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl FileVerifier for MockFileVerifier {
        async fn verify(&self, path: &Path) -> VerifierResult {
            self.verified.lock().unwrap().push(path.to_path_buf());
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(VerifierResult::Unsupported)
        }
        async fn capture_baseline(&self, _paths: &[PathBuf]) -> Bl {
            self.baseline.clone()
        }
    }

    fn diag(message: &str) -> Diagnostic {
        Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 7,
            column: Some(3),
            severity: Severity::Error,
            message: message.to_string(),
            code: Some("E0425".to_string()),
        }
    }

    fn checked(diagnostics: Vec<Diagnostic>) -> VerifierResult {
        VerifierResult::Checked { diagnostics }
    }

    /// A loop run over `dir` driving `client` with an injected `verifier`.
    async fn run_with_verifier(
        dir: &TempDir,
        client: &dyn AiClient,
        verifier: &dyn FileVerifier,
        max_turns: usize,
    ) -> PhaseResult {
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let budget = Budget::new(1_000_000);
        let d = LoopDeps {
            client,
            registry: &registry,
            tools: &[],
            budget: &budget,
            max_turns,
            project_root: dir.path(),
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier,
        };
        execute_phase(&input(), d).await.unwrap()
    }

    /// A native `write_file` call writing `body` to `name` under the temp root.
    fn write_call(dir: &TempDir, name: &str, body: &str) -> AiEvent {
        let path = dir.path().join(name).to_string_lossy().to_string();
        native("write_file", json!({ "path": path, "content": body }))
    }

    #[tokio::test]
    async fn edit_class_call_runs_verifier() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![])]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        assert_eq!(verifier.verified_paths().len(), 1);
        assert!(
            verifier.verified_paths()[0].ends_with("a.rs"),
            "verifier should have run on the edited file"
        );
    }

    #[tokio::test]
    async fn non_edit_call_does_not_run_verifier() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![])]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        assert!(verifier.verified_paths().is_empty());
    }

    #[tokio::test]
    async fn clean_verify_produces_no_retry_message() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![])]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        // The second model call's messages carry no verifier-retry user message.
        let second = &client.calls()[1].messages;
        assert!(
            !second
                .iter()
                .any(|m| m.role == "user" && m.content.contains("verifier found errors")),
            "clean verify must not feed a retry message"
        );
    }

    #[tokio::test]
    async fn author_diagnostics_fed_back_as_retry() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() { bork }")],
            vec![token("ok I'll fix it")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![diag("cannot find value `bork`")])]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        let second = &client.calls()[1].messages;
        assert!(
            second
                .iter()
                .any(|m| m.role == "user" && m.content.contains("cannot find value `bork`")),
            "author diagnostic should be fed back as a retry message"
        );
    }

    #[tokio::test]
    async fn ambient_diagnostics_not_fed_back() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        // The same diagnostic is in the baseline → ambient → must not feed back.
        let ambient = diag("pre-existing error");
        let mut bl = Bl::new();
        bl.record(&ambient);
        let verifier = MockFileVerifier::new(vec![checked(vec![ambient])]).with_baseline(bl);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        let second = &client.calls()[1].messages;
        assert!(
            !second
                .iter()
                .any(|m| m.role == "user" && m.content.contains("pre-existing error")),
            "ambient (baseline) diagnostics must not be fed back"
        );
    }

    #[tokio::test]
    async fn unsupported_verify_is_skipped() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "notes.md", "# hi")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![VerifierResult::Unsupported]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        // No Verify event logged for an unsupported language.
        let has_verify = records(dir.path())
            .iter()
            .any(|r| matches!(r.event, SessionEvent::Verify { .. }));
        assert!(!has_verify, "Unsupported must not log a Verify event");
    }

    #[tokio::test]
    async fn verifier_failed_appends_notice_not_err() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![VerifierResult::Failed("spawn failed".into())]);

        let result = run_with_verifier(&dir, &client, &verifier, 8).await;

        assert_eq!(result.status, PhaseStatus::Complete);
        let second = &client.calls()[1].messages;
        assert!(
            second
                .iter()
                .any(|m| m.content.contains("verifier failed: spawn failed")),
            "a verifier infra failure should append a notice, not error"
        );
    }

    #[tokio::test]
    async fn persistent_verifier_failure_trips_hard_fail() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "v1")],
            vec![write_call(&dir, "a.rs", "v2")],
            vec![write_call(&dir, "a.rs", "v3")],
            vec![token("unreached")],
        ]);
        // Three consecutive Checked-with-author verifier runs.
        let verifier = MockFileVerifier::new(vec![
            checked(vec![diag("err1")]),
            checked(vec![diag("err2")]),
            checked(vec![diag("err3")]),
        ]);

        let result = run_with_verifier(&dir, &client, &verifier, 10).await;

        assert_eq!(result.status, PhaseStatus::HardFail);
        let briefing = result.briefing.unwrap();
        assert!(matches!(
            briefing.current_blocker,
            Blocker::HardFail(HardFailSignal::VerifierFailurePersistent { .. })
        ));
        assert!(
            !briefing.diagnostics.is_empty(),
            "hard-fail briefing must carry the current diagnostics"
        );
    }

    #[tokio::test]
    async fn identical_tool_call_repetition_trips_hard_fail() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let mk = || native("read_file", json!({ "path": path }));
        let client = MockAiClientScript::new(vec![
            vec![mk()],
            vec![mk()],
            vec![mk()],
            vec![token("unreached")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_with_verifier(&dir, &client, &verifier, 10).await;

        assert_eq!(result.status, PhaseStatus::HardFail);
        assert!(matches!(
            result.briefing.unwrap().current_blocker,
            Blocker::HardFail(HardFailSignal::IdenticalToolCallRepetition { .. })
        ));
    }

    #[tokio::test]
    async fn runaway_output_trips_hard_fail() {
        let dir = TempDir::new().unwrap();
        // A file larger than the runaway threshold; reading it overflows the cap.
        let big = "x".repeat(110 * 1024);
        std::fs::write(dir.path().join("big.txt"), &big).unwrap();
        let path = dir.path().join("big.txt").to_string_lossy().to_string();
        let client =
            MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_with_verifier(&dir, &client, &verifier, 10).await;

        assert_eq!(result.status, PhaseStatus::HardFail);
        assert!(matches!(
            result.briefing.unwrap().current_blocker,
            Blocker::HardFail(HardFailSignal::RunawayOutput { .. })
        ));
    }

    #[tokio::test]
    async fn hard_fail_logs_hardfail_then_session_end() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let mk = || native("read_file", json!({ "path": path }));
        let client = MockAiClientScript::new(vec![vec![mk()], vec![mk()], vec![mk()]]);
        let verifier = MockFileVerifier::new(vec![]);

        run_with_verifier(&dir, &client, &verifier, 10).await;

        let kinds: Vec<&str> = records(dir.path())
            .iter()
            .map(|r| event_kind(&r.event))
            .collect();
        let hf = kinds.iter().position(|k| *k == "hard_fail").unwrap();
        let se = kinds.iter().position(|k| *k == "session_end").unwrap();
        assert!(hf < se, "HardFail must be logged before SessionEnd");
        match &records(dir.path()).last().unwrap().event {
            SessionEvent::SessionEnd { status, .. } => assert_eq!(status, "hard_fail"),
            other => panic!("expected SessionEnd, got {other:?}"),
        }
    }
}
