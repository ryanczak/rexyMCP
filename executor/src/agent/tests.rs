use super::*;
use crate::agent::cancel::{CancelHandle, CancelSignal};
use crate::agent::command::{CommandResult, MAX_COMMAND_TAIL_CHARS, RealCommandRunner};
use crate::ai::AiClient;
use crate::ai::testing::{MockAiClientScript, MockCall};
use crate::ai::types::{AiEvent, Message, TokenBreakdown, ToolSchema};
use crate::phase::{Blocker, PhaseStatus};
use crate::security::scope::Scope;
use crate::store::telemetry::PhaseRun;
use crate::tools::{bash_with_filter, patch, read_file, write_file};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::mpsc::UnboundedSender;

fn registry_over(scope: Scope) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(read_file(scope.clone()));
    r.register(write_file(scope.clone()));
    r.register(patch(scope));
    r
}

const SESSION_ID: &str = "testsid";
const PHASE_SLUG: &str = "phase-07b";

fn clock_zero() -> u64 {
    0
}

fn input() -> PhaseInput {
    PhaseInput {
        standards: "STANDARDS".to_string(),
        phase_doc: "PHASE".to_string(),
        goal: "make it compile".to_string(),
        acceptance_criteria: "cargo build passes".to_string(),
        phase: PHASE_SLUG.to_string(),
        tags: vec!["rust".to_string(), "feature".to_string()],
        phase_doc_path: "docs/dev/milestones/M0-test/phase-01-test.md".to_string(),
        project_id: None,
        milestone_id: None,
        tier: None,
        resumed_task_states: None,
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

/// A command runner for runs with no commands configured (`EMPTY_COMMANDS`),
/// where `run` is never actually reached; returns an empty success if it is.
struct NoopRunner;

#[async_trait::async_trait]
impl CommandRunner for NoopRunner {
    async fn run(&self, _command: &str, _cwd: &Path) -> CommandResult {
        CommandResult {
            output: String::new(),
            success: true,
        }
    }
}

const EMPTY_COMMANDS: CommandConfig = CommandConfig {
    format: None,
    build: None,
    lint: None,
    test: None,
    lint_fix: None,
    format_fix: None,
};

/// A command runner with a scripted sequence of outcomes. Each `run` call pops
/// the next `bool`; returns `success: true` once the script is exhausted.
/// `output` is empty on success and `"gate failed"` on failure.
struct ScriptedCommandRunner {
    script: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<bool>>>,
}

impl ScriptedCommandRunner {
    fn new(outcomes: Vec<bool>) -> Self {
        Self {
            script: std::sync::Arc::new(std::sync::Mutex::new(outcomes.into())),
        }
    }
}

#[async_trait::async_trait]
impl CommandRunner for ScriptedCommandRunner {
    async fn run(&self, _command: &str, _cwd: &Path) -> CommandResult {
        let success = self
            .script
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .unwrap_or(true);
        CommandResult {
            output: if success {
                String::new()
            } else {
                "gate failed".to_string()
            },
            success,
        }
    }
}

fn all_commands_configured() -> CommandConfig {
    CommandConfig {
        format: Some("true".to_string()),
        build: Some("true".to_string()),
        lint: Some("true".to_string()),
        test: Some("true".to_string()),
        lint_fix: None,
        format_fix: None,
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
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams {
            temperature: None,
            seed: None,
        },
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
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
async fn think_only_completion_is_not_complete() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let client = MockAiClientScript::new(vec![
        vec![token("<think>I will read the file</think>\n\n")],
        vec![token("All done.")],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);
    assert_eq!(client.calls().len(), 2);
}

#[tokio::test]
async fn think_only_completion_at_budget_is_budget_exceeded() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let client = MockAiClientScript::new(vec![
        vec![token("<think>plan</think>\n\n")],
        vec![token("<think>still thinking</think>\n")],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 2, dir.path()))
        .await
        .unwrap();

    assert_eq!(result.status, PhaseStatus::BudgetExceeded);
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
async fn completes_without_flipping_status_now_that_gate_is_gone() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let phase_doc = dir.path().join("phase-01-test.md");
    std::fs::write(
        &phase_doc,
        "# Phase 01: Test\n\n**Status:** in-progress\n\n## Update Log\n\n### Update — 2026-01-01 (started)\n",
    )
    .unwrap();
    let inp = PhaseInput {
        phase_doc_path: phase_doc.to_string_lossy().into_owned(),
        ..input()
    };
    let client = MockAiClientScript::new(vec![vec![token("all done")]]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&inp, deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);
    assert_eq!(
        client.calls().len(),
        1,
        "an in-progress doc must not trigger a bookkeeping re-loop"
    );
}

#[tokio::test]
async fn native_tool_call_event_dispatches_as_origin_native() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    // Always emit a native call; cap at 1 turn so we can inspect the snapshot.
    let client = MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
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
    let client = MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
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

// Turn-0 case: backend error before any work stays Err (nothing to preserve).
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

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path())).await;

    assert!(
        result.is_err(),
        "AiEvent::Error must surface as Err, not a PhaseResult"
    );
}

// ── M7-01: backend error degradation ──────────────────────────────────

/// Mock that returns `Err` from `chat()` on a configured call number.
/// Used to exercise the `chat_fut` error path (site A).
struct MockAiClientChatError {
    error_on_call: Arc<Mutex<usize>>,
    events: Arc<Mutex<VecDeque<Vec<AiEvent>>>>,
    calls: Arc<Mutex<Vec<MockCall>>>,
}

impl MockAiClientChatError {
    fn new(events: Vec<Vec<AiEvent>>, error_on_call: usize) -> Self {
        Self {
            error_on_call: Arc::new(Mutex::new(error_on_call)),
            events: Arc::new(Mutex::new(events.into_iter().collect())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl AiClient for MockAiClientChatError {
    async fn chat(
        &self,
        system_prompt: &str,
        messages: Vec<Message>,
        tx: mpsc::UnboundedSender<AiEvent>,
        tools: Option<&[ToolSchema]>,
    ) -> anyhow::Result<()> {
        let call_idx = {
            let mut calls = self.calls.lock().unwrap();
            let idx = calls.len();
            calls.push(MockCall {
                system_prompt: system_prompt.to_string(),
                messages,
                tool_count: tools.map(|t| t.len()).unwrap_or(0),
            });
            idx
        };
        let error_on = *self.error_on_call.lock().unwrap();
        if call_idx == error_on {
            return Err(anyhow::anyhow!("transient backend failure"));
        }
        let events = self.events.lock().unwrap().pop_front();
        if let Some(evts) = events {
            for e in evts {
                let _ = tx.send(e);
            }
        }
        Ok(())
    }
}

#[tokio::test]
async fn backend_error_after_progress_degrades_to_hard_fail() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    // Turn 0: a tool call (read_file) → loop continues with turns=1.
    // Turn 1: chat() returns Err → should degrade to hard_fail.
    let client = MockAiClientChatError::new(
        vec![vec![native("read_file", json!({ "path": path }))]],
        1, // second chat() call (index 1) returns error
    );
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path())).await;

    assert!(
        result.is_ok(),
        "backend error after progress must degrade to Ok(hard_fail), not Err"
    );
    let phase_result = result.unwrap();
    assert_eq!(phase_result.status, PhaseStatus::HardFail);
    assert!(phase_result.briefing.is_some());
    let briefing = phase_result.briefing.unwrap();
    assert!(
        matches!(
            briefing.current_blocker,
            Blocker::HardFail(HardFailSignal::BackendError { .. })
        ),
        "expected BackendError hard-fail signal, got {:?}",
        briefing.current_blocker
    );
}

#[tokio::test]
async fn ai_event_error_after_progress_degrades_to_hard_fail() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    // Turn 0: a tool call (read_file) → loop continues with turns=1.
    // Turn 1: AiEvent::Error in the stream → should degrade to hard_fail.
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![
            token("starting second"),
            AiEvent::Error("mid-phase error".to_string()),
            AiEvent::Done(TokenBreakdown::default()),
        ],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path())).await;

    assert!(
        result.is_ok(),
        "AiEvent::Error after progress must degrade to Ok(hard_fail), not Err"
    );
    let phase_result = result.unwrap();
    assert_eq!(phase_result.status, PhaseStatus::HardFail);
    assert!(phase_result.briefing.is_some());
    let briefing = phase_result.briefing.unwrap();
    assert!(
        matches!(
            briefing.current_blocker,
            Blocker::HardFail(HardFailSignal::BackendError { .. })
        ),
        "expected BackendError hard-fail signal, got {:?}",
        briefing.current_blocker
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

    // Progress and Metrics records are logged unconditionally and interleave
    // with the turn events; filter them out to assert the turn-event sequence.
    let kinds: Vec<&str> = records(dir.path())
        .iter()
        .map(|r| event_kind(&r.event))
        .filter(|k| *k != "progress" && *k != "metrics")
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
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
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
        SessionEvent::Metrics { .. } => "metrics",
        SessionEvent::Compaction { .. } => "compaction",
        SessionEvent::OutputFiltered { .. } => "output_filtered",
        SessionEvent::ReadEvicted { .. } => "read_evicted",
        SessionEvent::ReadDeduped { .. } => "read_deduped",
        SessionEvent::TaskUpdate { .. } => "task_update",
        SessionEvent::NoveltySample { .. } => "novelty_sample",
    }
}

// ── 07c: verifier retry + hard-fail ───────────────────────────────────

use crate::governor::verifier::{Baseline as Bl, Diagnostic, Severity};

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
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
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
async fn loop_surfaces_skipped_verifier_as_advisory() {
    let dir = TempDir::new().unwrap();
    let client = MockAiClientScript::new(vec![
        vec![write_call(&dir, "a.rs", "fn a() {}")],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![VerifierResult::Skipped(
        "cargo not found on PATH — install the Rust toolchain via https://rustup.rs; \
         incremental verification is disabled this run"
            .into(),
    )]);

    let result = run_with_verifier(&dir, &client, &verifier, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    let second = &client.calls()[1].messages;
    assert!(
        second
            .iter()
            .any(|m| m.content.contains("verifier skipped:")),
        "a skipped verifier should append a 'verifier skipped' advisory"
    );
    assert!(
        second
            .iter()
            .any(|m| m.content.contains("cargo not found on PATH")),
        "the advisory must name the missing binary"
    );
}

#[tokio::test]
async fn persistent_verifier_failure_trips_hard_fail() {
    let dir = TempDir::new().unwrap();
    let client = MockAiClientScript::new(vec![
        vec![write_call(&dir, "a.rs", "v1")],
        vec![write_call(&dir, "a.rs", "v2")],
        vec![write_call(&dir, "a.rs", "v3")],
        vec![write_call(&dir, "a.rs", "v4")],
        vec![write_call(&dir, "a.rs", "v5")],
        vec![write_call(&dir, "a.rs", "v6")],
        vec![token("unreached")],
    ]);
    // Six consecutive Checked-with-author verifier runs.
    let verifier = MockFileVerifier::new(vec![
        checked(vec![diag("err1")]),
        checked(vec![diag("err2")]),
        checked(vec![diag("err3")]),
        checked(vec![diag("err4")]),
        checked(vec![diag("err5")]),
        checked(vec![diag("err6")]),
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
    let client = MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_with_verifier(&dir, &client, &verifier, 10).await;

    assert_eq!(result.status, PhaseStatus::HardFail);
    assert!(matches!(
        result.briefing.unwrap().current_blocker,
        Blocker::HardFail(HardFailSignal::RunawayOutput { .. })
    ));
}

// ── M26 phase-07a: oscillation integration test ─────────────────────────

#[tokio::test]
async fn oscillation_across_alternating_reads_trips_hard_fail() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.txt"), "aaa").unwrap();
    std::fs::write(dir.path().join("b.txt"), "bbb").unwrap();
    let path_a = dir.path().join("a.txt").to_string_lossy().to_string();
    let path_b = dir.path().join("b.txt").to_string_lossy().to_string();
    let read_a = || native("read_file", json!({ "path": path_a.clone() }));
    let read_b = || native("read_file", json!({ "path": path_b.clone() }));
    // Alternate A,B,A,B — 4 distinct-turn tool calls, then one trailing read as slack
    let client = MockAiClientScript::new(vec![
        vec![read_a()],
        vec![read_b()],
        vec![read_a()],
        vec![read_b()],
        vec![read_a()], // slack
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    let d = LoopDeps {
        client: &client,
        registry: &registry_over(Scope::new(dir.path()).unwrap()),
        tools: &[],
        budget: &Budget::new(1_000_000),
        max_turns: 10,
        project_root: dir.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock_zero,
        verifier: &verifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig {
            oscillation_window: 4,
            oscillation_distinct_max: 2,
            ..GovernorConfig::default()
        },
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    let result = execute_phase(&input(), d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::HardFail);
    assert!(matches!(
        result.briefing.unwrap().current_blocker,
        Blocker::HardFail(HardFailSignal::Oscillation { .. })
    ));
}

// ── M34: novelty-sample observability integration test ──────────────────

/// Drives a read-only churn over one file, a mutating call, then a second
/// churn — with `novelty_distinct_floor = 0` so the measurement never trips the
/// stall (the run reaches the turn cap instead of hard-failing). Asserts the
/// loop emits exactly **two** `NoveltySample` records: one per churn streak.
/// That single count proves all three behaviors at once — emission (>0), dedup
/// (a 5-read streak collapses to one sample, not one-per-turn), and re-arm (the
/// post-edit streak emits again despite the same `distinct_targets`).
#[tokio::test]
async fn novelty_samples_are_emitted_deduped_and_rearm_after_edit() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.txt"), "aaa").unwrap();
    let path_a = dir.path().join("a.txt").to_string_lossy().to_string();
    let path_b = dir.path().join("b.txt").to_string_lossy().to_string();
    let read_a = || native("read_file", json!({ "path": path_a.clone() }));
    let write_b = || {
        native(
            "write_file",
            json!({ "path": path_b.clone(), "content": "b" }),
        )
    };
    // streak 1 (5 reads) → one sample; edit (re-arm) → no sample; streak 2 (5
    // reads) → one more sample. 11 turns; the turn cap ends the run.
    let client = MockAiClientScript::new(vec![
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![write_b()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    let d = LoopDeps {
        client: &client,
        registry: &registry_over(Scope::new(dir.path()).unwrap()),
        tools: &[],
        budget: &Budget::new(1_000_000),
        max_turns: 11,
        project_root: dir.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock_zero,
        verifier: &verifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig {
            novelty_window: 3,
            novelty_distinct_floor: 0, // distinct >= 1 > 0 → never trips; only measures
            read_only_stall_threshold: 0, // disable the raw backstop
            oscillation_window: 0,     // disable (read/write distinct=2 would trip)
            identical_call_threshold: 50, // 5 consecutive identical reads must not trip
            ..GovernorConfig::default()
        },
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    execute_phase(&input(), d).await.unwrap();

    let samples: Vec<(usize, usize)> = records(dir.path())
        .iter()
        .filter_map(|r| match r.event {
            SessionEvent::NoveltySample {
                window,
                distinct_targets,
            } => Some((window, distinct_targets)),
            _ => None,
        })
        .collect();

    assert_eq!(
        samples,
        vec![(3, 1), (3, 1)],
        "expected exactly two deduped novelty samples (one per churn streak, re-armed by the edit), got {samples:?}"
    );
}

// ── M34 phase-05: advisory-demotion integration tests ───────────────────

/// Shared churn setup: read one file repeatedly so a `novelty_window` of 3
/// collapses to 1 distinct target (≤ `novelty_distinct_floor` = 1 → would trip),
/// with the raw stall + oscillation + identical-repetition detectors disabled so
/// novelty is the only relevant signal. `action` selects advisory vs terminate.
async fn run_low_novelty_churn(action: crate::config::NoveltyAction) -> (TempDir, PhaseResult) {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.txt"), "aaa").unwrap();
    let path_a = dir.path().join("a.txt").to_string_lossy().to_string();
    let read_a = || native("read_file", json!({ "path": path_a.clone() }));
    let client = MockAiClientScript::new(vec![
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
        vec![read_a()],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let d = LoopDeps {
        client: &client,
        registry: &registry_over(Scope::new(dir.path()).unwrap()),
        tools: &[],
        budget: &Budget::new(1_000_000),
        max_turns: 5,
        project_root: dir.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock_zero,
        verifier: &verifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig {
            novelty_window: 3,
            novelty_distinct_floor: 1, // distinct=1 ≤ 1 → the window would trip
            novelty_action: action,
            read_only_stall_threshold: 0, // isolate novelty from the raw backstop
            oscillation_window: 0,
            identical_call_threshold: 50,
            ..GovernorConfig::default()
        },
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    // Await inside the helper so the LoopDeps borrows end before `dir` is returned.
    let result = execute_phase(&input(), d).await.unwrap();
    (dir, result)
}

#[tokio::test]
async fn low_novelty_churn_is_advisory_by_default() {
    let (dir, result) = run_low_novelty_churn(crate::config::NoveltyAction::Advisory).await;

    assert_eq!(
        result.status,
        PhaseStatus::BudgetExceeded,
        "advisory novelty must not hard-fail; the run reaches the turn cap"
    );
    let flagged = records(dir.path()).iter().any(|r| {
        matches!(
            r.event,
            SessionEvent::NoveltySample { distinct_targets, .. } if distinct_targets <= 1
        )
    });
    assert!(
        flagged,
        "the advisory measurement (distinct_targets ≤ floor) is still recorded"
    );
}

#[tokio::test]
async fn low_novelty_churn_hard_fails_when_action_is_terminate() {
    let (_dir, result) = run_low_novelty_churn(crate::config::NoveltyAction::Terminate).await;

    assert_eq!(result.status, PhaseStatus::HardFail);
    assert!(
        matches!(
            result.briefing.unwrap().current_blocker,
            Blocker::HardFail(HardFailSignal::LowNoveltyStall { .. })
        ),
        "terminate mode preserves the pre-demotion LowNoveltyStall hard-fail"
    );
}

// ── M26 phase-07a: cumulative-output-flood integration test ─────────────

#[tokio::test]
async fn cumulative_output_flood_trips_hard_fail() {
    let dir = TempDir::new().unwrap();
    // Three distinct files, each ~400 bytes (distinct args avoid oscillation)
    let content = "x".repeat(400);
    std::fs::write(dir.path().join("f1.txt"), &content).unwrap();
    std::fs::write(dir.path().join("f2.txt"), &content).unwrap();
    std::fs::write(dir.path().join("f3.txt"), &content).unwrap();
    let p1 = dir.path().join("f1.txt").to_string_lossy().to_string();
    let p2 = dir.path().join("f2.txt").to_string_lossy().to_string();
    let p3 = dir.path().join("f3.txt").to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": p1 }))],
        vec![native("read_file", json!({ "path": p2 }))],
        vec![native("read_file", json!({ "path": p3 }))],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    let d = LoopDeps {
        client: &client,
        registry: &registry_over(Scope::new(dir.path()).unwrap()),
        tools: &[],
        budget: &Budget::new(1_000_000),
        max_turns: 10,
        project_root: dir.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock_zero,
        verifier: &verifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig {
            output_window: 3,
            output_window_bytes: 1000,
            ..GovernorConfig::default()
        },
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    let result = execute_phase(&input(), d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::HardFail);
    assert!(matches!(
        result.briefing.unwrap().current_blocker,
        Blocker::HardFail(HardFailSignal::CumulativeOutputFlood { .. })
    ));
}

#[tokio::test]
async fn hard_fail_logs_hardfail_then_session_end() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    let mk = || native("read_file", json!({ "path": path }));
    let client = MockAiClientScript::new(vec![
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
    ]);
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

// ── 07d: read-before-edit ─────────────────────────────────────────────

fn patch_call(path: &str, old: &str, new: &str) -> ToolCall {
    ToolCall {
        name: "patch".to_string(),
        arguments: json!({ "path": path, "old_str": old, "new_str": new }),
        origin: Origin::Native,
    }
}

#[test]
fn gate_allows_non_patch_calls() {
    let root = Path::new("/repo");
    let ws = HashMap::new();
    let write = ToolCall {
        name: "write_file".to_string(),
        arguments: json!({ "path": "a.rs", "content": "x" }),
        origin: Origin::Native,
    };
    let read = ToolCall {
        name: "read_file".to_string(),
        arguments: json!({ "path": "a.rs" }),
        origin: Origin::Native,
    };
    assert!(read_before_edit_refusal(&write, &ws, root).is_none());
    assert!(read_before_edit_refusal(&read, &ws, root).is_none());
}

#[test]
fn gate_refuses_patch_of_unread_file() {
    let root = Path::new("/repo");
    let ws = HashMap::new();
    let call = patch_call("a.rs", "x", "y");
    let refusal = read_before_edit_refusal(&call, &ws, root).expect("should refuse");
    assert!(refusal.contains("have not read"));
}

#[test]
fn gate_allows_patch_of_read_unchanged_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn a() {}").unwrap();
    let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
    let mut ws = HashMap::new();
    ws.insert(file.clone(), mtime);
    let call = patch_call(file.to_str().unwrap(), "a", "b");
    assert!(read_before_edit_refusal(&call, &ws, dir.path()).is_none());
}

#[test]
fn gate_refuses_patch_when_mtime_changed() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn a() {}").unwrap();
    // Working set holds a stale mtime (epoch) — current differs → refuse.
    let mut ws = HashMap::new();
    ws.insert(file.clone(), SystemTime::UNIX_EPOCH);
    let call = patch_call(file.to_str().unwrap(), "a", "b");
    let refusal = read_before_edit_refusal(&call, &ws, dir.path()).expect("should refuse");
    assert!(refusal.contains("changed on disk"));
}

#[tokio::test]
async fn patch_without_prior_read_is_refused() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "original").unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "patch",
            json!({ "path": path, "old_str": "original", "new_str": "edited" }),
        )],
        vec![token("ok")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "original",
        "refused patch must not modify the file"
    );
    let second = &client.calls()[1].messages;
    assert!(
        second.iter().any(|m| m
            .tool_results
            .as_ref()
            .is_some_and(|trs| trs.iter().any(|t| t.content.contains("have not read")))),
        "refusal should be fed back to the model"
    );
}

#[tokio::test]
async fn patch_after_reading_is_allowed() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "original").unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![native(
            "patch",
            json!({ "path": path, "old_str": "original", "new_str": "edited" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "edited",
        "patch after a read should apply"
    );
}

// ── M10 phase-04: superseded-read eviction ────────────────────────────

#[tokio::test]
async fn loop_evicts_prior_read_after_patch() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("foo.txt");
    let original_content = "this is the original file content for eviction testing";
    std::fs::write(&file, original_content).unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![native(
            "patch",
            json!({ "path": path, "old_str": "original", "new_str": "edited" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    // The third model call (index 2) should have the evicted breadcrumb
    // in place of the original read content.
    let third_call_messages = &client.calls()[2].messages;
    let has_breadcrumb = third_call_messages.iter().any(|m| {
        m.tool_results
            .as_ref()
            .is_some_and(|trs| trs.iter().any(|t| t.content.starts_with("[superseded:")))
    });
    assert!(
        has_breadcrumb,
        "third call should contain a superseded breadcrumb"
    );

    // The read_file tool result specifically should NOT contain the original content.
    // (The patch result legitimately echoes the old content in its unified diff;
    // we only care that the read_file slot was evicted, not that the diff is suppressed.)
    let read_result_has_original = third_call_messages.iter().any(|m| {
        m.tool_results.as_ref().is_some_and(|trs| {
            trs.iter()
                .any(|t| t.tool_name == "read_file" && t.content.contains(original_content))
        })
    });
    assert!(
        !read_result_has_original,
        "read_file tool result should NOT contain the original content after eviction"
    );
}

#[tokio::test]
async fn loop_logs_read_evicted_event_after_patch() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("foo.txt");
    std::fs::write(&file, "original content").unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![native(
            "patch",
            json!({ "path": path, "old_str": "original", "new_str": "edited" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    let recs = records(dir.path());
    let has_read_evicted = recs.iter().any(|r| {
        matches!(
            &r.event,
            SessionEvent::ReadEvicted { reads_evicted, .. } if *reads_evicted >= 1
        )
    });
    assert!(
        has_read_evicted,
        "session log should contain a ReadEvicted event with reads_evicted >= 1"
    );
}

#[tokio::test]
async fn loop_does_not_log_read_evicted_without_prior_read() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("foo.txt");
    // No std::fs::write — the file does NOT pre-exist, so write_file is a
    // create (allowed without read). The intent is "a successful write with no
    // prior read evicts nothing."
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "fresh content" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    let recs = records(dir.path());
    let has_read_evicted = recs
        .iter()
        .any(|r| matches!(&r.event, SessionEvent::ReadEvicted { .. }));
    assert!(
        !has_read_evicted,
        "session log should NOT contain a ReadEvicted event when there was no prior read"
    );
}

#[tokio::test]
async fn write_file_without_read_is_allowed() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("new.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "fresh" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "fresh",
        "write_file is not gated by read-before-edit"
    );
}

// ── M26 phase-04: write_file read-before-edit integration tests ────────

#[tokio::test]
async fn write_file_overwrite_of_unread_file_is_refused() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("existing.txt");
    let original = "original content";
    std::fs::write(&file, original).unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "new content" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_with_verifier(&dir, &client, &verifier, 8).await;

    // The run reaches Complete (refusal is model-visible, not a hard_fail)
    assert_eq!(result.status, PhaseStatus::Complete);

    // The file on disk is unchanged (the overwrite was refused)
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        original,
        "the file should not have been overwritten"
    );

    // The refused tool result fed back contains "refusing to overwrite"
    let second_call_messages = &client.calls()[1].messages;
    let has_refusal = second_call_messages.iter().any(|m| {
        m.tool_results.as_ref().is_some_and(|trs| {
            trs.iter()
                .any(|t| t.tool_name == "write_file" && t.content.contains("refusing to overwrite"))
        })
    });
    assert!(
        has_refusal,
        "the refused write_file should contain 'refusing to overwrite'"
    );
}

#[tokio::test]
async fn write_file_after_read_overwrites() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("existing.txt");
    let original = "original content";
    std::fs::write(&file, original).unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![native(
            "write_file",
            json!({ "path": path, "content": "new content" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    // The file on disk is the new content (the read unlocked the overwrite)
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "new content",
        "the file should have been overwritten after read"
    );
}

#[tokio::test]
async fn write_file_append_to_unread_file_is_allowed() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("existing.txt");
    let original = "original content";
    std::fs::write(&file, original).unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": " appended", "append": true }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    // The file on disk is original + appended
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "original content appended",
        "append should have been allowed without read"
    );
}

#[tokio::test]
async fn repeated_refused_patch_trips_hard_fail() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "original").unwrap();
    let path = file.to_string_lossy().to_string();
    let mk = || {
        native(
            "patch",
            json!({ "path": path, "old_str": "original", "new_str": "x" }),
        )
    };
    let client = MockAiClientScript::new(vec![
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_with_verifier(&dir, &client, &verifier, 10).await;

    assert_eq!(result.status, PhaseStatus::HardFail);
    assert!(matches!(
        result.briefing.unwrap().current_blocker,
        Blocker::HardFail(HardFailSignal::IdenticalToolCallRepetition { .. })
    ));
}

// ── 07e: completion artifacts ─────────────────────────────────────────

/// Records which commands ran and returns scripted output; commands named in
/// `failing` return `success: false` (for gate tests).
struct MockCommandRunner {
    ran: Mutex<Vec<String>>,
    output: String,
    failing: HashSet<String>,
}

impl MockCommandRunner {
    fn new(output: &str) -> Self {
        Self {
            ran: Mutex::new(Vec::new()),
            output: output.to_string(),
            failing: HashSet::new(),
        }
    }
    fn failing(mut self, command: &str) -> Self {
        self.failing.insert(command.to_string());
        self
    }
    fn ran(&self) -> Vec<String> {
        self.ran.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl CommandRunner for MockCommandRunner {
    async fn run(&self, command: &str, _cwd: &Path) -> CommandResult {
        self.ran.lock().unwrap().push(command.to_string());
        CommandResult {
            output: self.output.clone(),
            success: !self.failing.contains(command),
        }
    }
}

/// Full run with injectable command runner + command config + telemetry dir.
async fn run_full(
    dir: &TempDir,
    client: &dyn AiClient,
    verifier: &dyn FileVerifier,
    runner: &dyn CommandRunner,
    commands: &CommandConfig,
    telemetry_dir: Option<&Path>,
    max_turns: usize,
) -> PhaseResult {
    run_full_with_context_window(
        dir,
        client,
        verifier,
        runner,
        commands,
        telemetry_dir,
        max_turns,
        None,
    )
    .await
}

/// Same as `run_full` but with an explicit `context_window` value.
#[allow(clippy::too_many_arguments)]
async fn run_full_with_context_window(
    dir: &TempDir,
    client: &dyn AiClient,
    verifier: &dyn FileVerifier,
    runner: &dyn CommandRunner,
    commands: &CommandConfig,
    telemetry_dir: Option<&Path>,
    max_turns: usize,
    context_window: Option<usize>,
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
        commands,
        runner,
        generation_params: GenerationParams::default(),
        telemetry_dir,
        progress: None,
        context_window,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    execute_phase(&input(), d).await.unwrap()
}

#[tokio::test]
async fn diff_and_files_changed_for_edited_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "original\n").unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![native(
            "patch",
            json!({ "path": path, "old_str": "original", "new_str": "edited" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        None,
        8,
    )
    .await;

    assert!(result.diff.contains("-original"), "diff: {}", result.diff);
    assert!(result.diff.contains("+edited"));
    assert_eq!(result.files_changed.len(), 1);
    assert!(result.files_changed[0].path.ends_with("t.txt"));
    assert!(result.files_changed[0].change_summary.contains('+'));
}

#[tokio::test]
async fn new_file_diff_is_all_added() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("new.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "line1\nline2\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        None,
        8,
    )
    .await;

    assert!(result.diff.contains("+line1"));
    assert!(result.diff.contains("+line2"));
    assert!(!result.diff.contains("-line"));
    assert_eq!(result.files_changed.len(), 1);
}

#[tokio::test]
async fn unchanged_file_is_absent_from_files_changed() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "same\n").unwrap();
    let path = file.to_string_lossy().to_string();
    // Prepended read_file so the identical-content overwrite actually executes
    // (the gate would refuse without the read).
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![native(
            "write_file",
            json!({ "path": path, "content": "same\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        None,
        8,
    )
    .await;

    assert!(
        result.files_changed.is_empty(),
        "an unchanged file must not appear in files_changed"
    );
    assert!(result.diff.is_empty());
}

#[tokio::test]
async fn clean_completion_runs_configured_commands() {
    let dir = TempDir::new().unwrap();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok-output");
    let commands = CommandConfig {
        format: None,
        build: Some("cargo build".to_string()),
        lint: None,
        test: Some("cargo test".to_string()),
        lint_fix: None,
        format_fix: None,
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    let ran = runner.ran();
    assert_eq!(
        ran,
        vec!["cargo build".to_string(), "cargo test".to_string()]
    );
    assert_eq!(result.command_outputs.build.as_deref(), Some("ok-output"));
    assert_eq!(result.command_outputs.test.as_deref(), Some("ok-output"));
    assert!(result.command_outputs.format.is_none());
    assert!(result.command_outputs.lint.is_none());
}

#[tokio::test]
async fn command_output_is_tail_capped() {
    let dir = TempDir::new().unwrap();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);
    let big = "y".repeat(MAX_COMMAND_TAIL_CHARS + 500);
    let runner = MockCommandRunner::new(&big);
    let commands = CommandConfig {
        format: None,
        build: Some("b".to_string()),
        lint: None,
        test: None,
        lint_fix: None,
        format_fix: None,
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(
        result
            .command_outputs
            .build
            .as_deref()
            .map(|s| s.chars().count()),
        Some(MAX_COMMAND_TAIL_CHARS)
    );
}

#[tokio::test]
async fn hard_fail_does_not_run_command_set() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    let mk = || native("read_file", json!({ "path": path }));
    let client = MockAiClientScript::new(vec![
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("should-not-run");
    let commands = CommandConfig {
        format: None,
        build: Some("cargo build".to_string()),
        lint: None,
        test: None,
        lint_fix: None,
        format_fix: None,
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 10).await;

    assert_eq!(result.status, PhaseStatus::HardFail);
    assert!(
        runner.ran().is_empty(),
        "command set must not run on hard-fail"
    );
    assert!(result.command_outputs.build.is_none());
}

#[tokio::test]
async fn complete_result_reports_log_path() {
    let dir = TempDir::new().unwrap();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        None,
        8,
    )
    .await;

    assert_eq!(result.log_path, Some(log_path(dir.path())));
}

#[tokio::test]
async fn log_path_is_none_when_log_unopened() {
    let dir = TempDir::new().unwrap();
    // `.rexymcp` as a file → sessions dir can't be created → log doesn't open.
    std::fs::write(dir.path().join(".rexymcp"), "x").unwrap();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        None,
        8,
    )
    .await;

    assert_eq!(result.status, PhaseStatus::Complete);
    assert!(result.log_path.is_none());
}

// ── 08: PhaseRun telemetry ────────────────────────────────────────────

fn read_runs(telem: &Path) -> Vec<PhaseRun> {
    crate::store::telemetry::read(&telem.join("phase_runs.jsonl")).unwrap()
}

#[tokio::test]
async fn run_appends_one_phase_run_line() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    let runs = read_runs(&telem);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "complete");
    assert!(!runs[0].escalated);
}

#[tokio::test]
async fn telemetry_none_dir_is_noop_and_completes() {
    let dir = TempDir::new().unwrap();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        None,
        8,
    )
    .await;

    assert_eq!(result.status, PhaseStatus::Complete);
}

#[tokio::test]
async fn telemetry_write_failure_does_not_change_result() {
    let dir = TempDir::new().unwrap();
    // Telemetry "dir" is actually a file → create_dir_all fails → append errs.
    let telem_file = dir.path().join("telem_is_a_file");
    std::fs::write(&telem_file, "x").unwrap();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    let result = run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem_file),
        8,
    )
    .await;

    assert_eq!(result.status, PhaseStatus::Complete);
}

#[tokio::test]
async fn hard_fail_run_is_escalated() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    let mk = || native("read_file", json!({ "path": path }));
    let client = MockAiClientScript::new(vec![
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        10,
    )
    .await;

    let runs = read_runs(&telem);
    assert_eq!(runs[0].status, "hard_fail");
    assert!(runs[0].escalated);
}

#[tokio::test]
async fn gates_populated_on_complete_from_exit_status() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("out");
    let commands = CommandConfig {
        format: None,
        build: Some("cargo build".to_string()),
        lint: None,
        test: Some("cargo test".to_string()),
        lint_fix: None,
        format_fix: None,
    };

    run_full(
        &dir,
        &client,
        &verifier,
        &runner,
        &commands,
        Some(&telem),
        8,
    )
    .await;

    let gates = read_runs(&telem)[0].gates.clone();
    assert_eq!(gates.build, Some(true));
    assert_eq!(gates.test, Some(true));
    assert_eq!(gates.fmt, None);
    assert_eq!(gates.lint, None);
}

#[tokio::test]
async fn gates_none_on_hard_fail() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    let mk = || native("read_file", json!({ "path": path }));
    let client = MockAiClientScript::new(vec![
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
        vec![mk()],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("out");
    let commands = CommandConfig {
        format: None,
        build: Some("cargo build".to_string()),
        lint: None,
        test: None,
        lint_fix: None,
        format_fix: None,
    };

    run_full(
        &dir,
        &client,
        &verifier,
        &runner,
        &commands,
        Some(&telem),
        10,
    )
    .await;

    let gates = read_runs(&telem)[0].gates.clone();
    assert_eq!(gates.build, None, "no gate should be set on hard-fail");
    assert!(runner.ran().is_empty());
}

#[tokio::test]
async fn tool_success_rate_reflects_scorer() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    // one failure (unknown tool) + one success (read_file), then complete.
    let client = MockAiClientScript::new(vec![
        vec![native("does_not_exist", json!({}))],
        vec![native("read_file", json!({ "path": path }))],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    let rate = read_runs(&telem)[0].tool_success_rate;
    assert!((rate - 0.5).abs() < 1e-9, "expected 0.5, got {rate}");
}

#[tokio::test]
async fn parse_failure_rate_counts_only_parse_attempts() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    // native turn (no parse), then a malformed text turn (parse fail), then a
    // plain-text turn (parse attempt, NoToolCall) → attempts=2, failures=1.
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![token("{\"name\":\"nonexistent\",\"arguments\":{}}")],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    let rate = read_runs(&telem)[0].parse_failure_rate;
    assert!((rate - 0.5).abs() < 1e-9, "expected 0.5, got {rate}");
}

#[tokio::test]
async fn repairs_per_call_counts_repaired_origin() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    // A close-typo tool name the parser fuzzy-repairs → Origin::Repaired.
    let hermes = format!(
        "<tool_call>{{\"name\":\"read_fil\",\"arguments\":{{\"path\":\"{}\"}}}}</tool_call>",
        path
    );
    let client = MockAiClientScript::new(vec![vec![token(&hermes)], vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    assert!(
        read_runs(&telem)[0].repairs_per_call > 0.0,
        "a repaired call should count repairs"
    );
}

#[tokio::test]
async fn verifier_retries_counts_author_failures() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    let client = MockAiClientScript::new(vec![
        vec![write_call(&dir, "a.rs", "bad")],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![checked(vec![diag("err")])]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    assert_eq!(read_runs(&telem)[0].verifier_retries, 1);
}

#[tokio::test]
async fn tokens_accumulate_across_done_events() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    let tb = |n: u32| {
        AiEvent::Done(TokenBreakdown {
            input_tokens: n,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        })
    };
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path })), tb(10)],
        vec![token("done"), tb(5)],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    assert_eq!(read_runs(&telem)[0].tokens.input_tokens, 15);
}

#[tokio::test]
async fn logs_metrics_event_per_turn() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let tb = AiEvent::Done(TokenBreakdown {
        input_tokens: 42,
        output_tokens: 17,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    });
    let completion = AiEvent::Completion {
        finish_reason: Some("stop".into()),
        model: None,
    };
    let client = MockAiClientScript::new(vec![vec![token("done"), tb, completion]]);
    // Real, non-sentinel ceiling so context_pct is non-zero.
    // Must be large enough that the prompt doesn't overflow before turn 1.
    let budget = Budget::new(100_000);

    execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    let recs = records(dir.path());
    let metrics_recs: Vec<_> = recs
        .iter()
        .filter_map(|r| {
            if let SessionEvent::Metrics {
                input_tokens,
                output_tokens,
                context_pct,
                ..
            } = &r.event
            {
                Some((*input_tokens, *output_tokens, *context_pct))
            } else {
                None
            }
        })
        .collect();

    assert!(
        !metrics_recs.is_empty(),
        "expected at least one Metrics record, got {} total records: {:?}",
        recs.len(),
        recs.iter()
            .map(|r| event_kind(&r.event))
            .collect::<Vec<_>>()
    );
    let (in_tok, out_tok, ctx_pct) = metrics_recs.last().unwrap();
    assert_eq!(*in_tok, 42, "input_tokens mismatch");
    assert_eq!(*out_tok, 17, "output_tokens mismatch");
    assert!(
        *ctx_pct > 0.0,
        "context_pct should be > 0 with a real ceiling and non-empty messages"
    );
}

#[tokio::test]
async fn logs_compaction_event_when_budget_overflows() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Tiny budget so overflow fires on turn 0 (system prompt alone is
    // hundreds of tokens). The model is never called.
    let client = MockAiClientScript::new(vec![]);
    let budget = Budget::new(10);

    execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    let recs = records(dir.path());
    let compaction_recs: Vec<_> = recs
        .iter()
        .filter_map(|r| {
            if let SessionEvent::Compaction {
                tokens_before,
                tokens_after,
                ..
            } = &r.event
            {
                Some((*tokens_before, *tokens_after))
            } else {
                None
            }
        })
        .collect();

    assert!(
        !compaction_recs.is_empty(),
        "expected at least one Compaction record, got {} total records: {:?}",
        recs.len(),
        recs.iter()
            .map(|r| event_kind(&r.event))
            .collect::<Vec<_>>()
    );

    let (tokens_before, tokens_after) = compaction_recs.first().unwrap();
    assert!(*tokens_before > 0, "tokens_before should be > 0");
    assert!(
        *tokens_before >= *tokens_after,
        "tokens_before ({}) should be >= tokens_after ({})",
        tokens_before,
        tokens_after
    );
}

#[tokio::test]
async fn wall_clock_zero_under_constant_clock() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    assert_eq!(read_runs(&telem)[0].wall_clock_s, 0.0);
}

// ── 05a: progress callback ────────────────────────────────────────────

use crate::agent::progress::ProgressEvent;

/// Captures progress events into a `Mutex<Vec<ProgressEvent>>` for test
/// inspection. Implements `ProgressCallback` so it can be held by `LoopDeps`
/// without a closure lifetime issue.
struct CaptureCallback {
    events: std::sync::Mutex<Vec<ProgressEvent>>,
}

impl CaptureCallback {
    fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn events(&self) -> Vec<ProgressEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl ProgressCallback for CaptureCallback {
    fn on_progress(&self, event: &ProgressEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
}

/// Helper: build LoopDeps with a progress callback that captures into
/// `capture`. Uses NoopVerifier + NoopRunner + empty commands + no telemetry.
fn deps_with_progress_simple<'a>(
    client: &'a dyn AiClient,
    registry: &'a ToolRegistry,
    budget: &'a Budget,
    max_turns: usize,
    root: &'a Path,
    capture: &'a CaptureCallback,
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
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams {
            temperature: None,
            seed: None,
        },
        telemetry_dir: None,
        progress: Some(capture),
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    }
}

/// Builder for LoopDeps with a progress callback, allowing per-test overrides.
struct DepsBuilder<'a> {
    client: &'a dyn AiClient,
    registry: &'a ToolRegistry,
    budget: &'a Budget,
    max_turns: usize,
    root: &'a Path,
    capture: &'a CaptureCallback,
    verifier: &'a dyn FileVerifier,
    commands: &'a CommandConfig,
    runner: &'a dyn CommandRunner,
    telemetry_dir: Option<&'a Path>,
}

impl<'a> DepsBuilder<'a> {
    fn new(
        client: &'a dyn AiClient,
        registry: &'a ToolRegistry,
        budget: &'a Budget,
        max_turns: usize,
        root: &'a Path,
        capture: &'a CaptureCallback,
    ) -> Self {
        Self {
            client,
            registry,
            budget,
            max_turns,
            root,
            capture,
            verifier: &NoopVerifier,
            commands: &EMPTY_COMMANDS,
            runner: &NoopRunner,
            telemetry_dir: None,
        }
    }
    fn verifier(mut self, v: &'a dyn FileVerifier) -> Self {
        self.verifier = v;
        self
    }
    fn commands(mut self, c: &'a CommandConfig) -> Self {
        self.commands = c;
        self
    }
    fn runner(mut self, r: &'a dyn CommandRunner) -> Self {
        self.runner = r;
        self
    }
    fn build(self) -> LoopDeps<'a> {
        LoopDeps {
            client: self.client,
            registry: self.registry,
            tools: &[],
            budget: self.budget,
            max_turns: self.max_turns,
            project_root: self.root,
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier: self.verifier,
            commands: self.commands,
            runner: self.runner,
            generation_params: GenerationParams {
                temperature: None,
                seed: None,
            },
            telemetry_dir: self.telemetry_dir,
            progress: Some(self.capture),
            context_window: None,
            governor: GovernorConfig::default(),
            task_tracking: true,
            gate_retries: u32::MAX,
            wall_clock_secs: 0,
            cancel: CancelSignal::never(),
        }
    }
}

#[tokio::test]
async fn progress_none_still_logs_progress_records() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let budget = Budget::new(1_000_000);

    execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    // The session-log Progress records are independent of the live
    // callback: `rexymcp status` and Claude's post-return log queries must
    // see liveness even when no live watcher (progress token) is attached.
    let recs = records(dir.path());
    assert!(
        recs.iter()
            .any(|r| matches!(r.event, SessionEvent::Progress { .. })),
        "progress: None must still produce Progress log entries"
    );
}

#[tokio::test]
async fn progress_some_emits_turn_start_and_tool() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    let capture = CaptureCallback::new();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![token("done")],
    ]);
    let budget = Budget::new(1_000_000);

    execute_phase(
        &input(),
        deps_with_progress_simple(&client, &registry, &budget, 8, dir.path(), &capture),
    )
    .await
    .unwrap();

    let events = capture.events();
    let stages: Vec<&str> = events.iter().map(|e| e.stage.as_str()).collect();
    assert!(
        stages.contains(&"turn_start"),
        "expected a turn_start event, got: {:?}",
        stages
    );
    assert!(
        stages.contains(&"tool:read_file"),
        "expected a tool:read_file event, got: {:?}",
        stages
    );
    assert!(
        stages.contains(&"tool:read_file"),
        "expected a tool:read_file event, got: {:?}",
        stages
    );

    // Also check the session log has matching Progress entries.
    let recs = records(dir.path());
    let progress_count = recs
        .iter()
        .filter(|r| matches!(r.event, SessionEvent::Progress { .. }))
        .count();
    assert!(
        progress_count >= events.len(),
        "log should have at least as many Progress entries as callback events"
    );
}

#[tokio::test]
async fn progress_emits_verify_after_edit_class_tool() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let capture = CaptureCallback::new();
    let client = MockAiClientScript::new(vec![
        vec![write_call(&dir, "a.rs", "fn a() {}")],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![checked(vec![])]);
    let budget = Budget::new(1_000_000);

    let d = DepsBuilder::new(&client, &registry, &budget, 8, dir.path(), &capture)
        .verifier(&verifier)
        .build();
    execute_phase(&input(), d).await.unwrap();

    let events = capture.events();
    let stages: Vec<&str> = events.iter().map(|e| e.stage.as_str()).collect();
    assert!(
        stages.contains(&"verify"),
        "expected a verify event after edit-class tool, got: {:?}",
        stages
    );
}

#[tokio::test]
async fn progress_emits_commands_on_clean_completion() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let capture = CaptureCallback::new();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        format: None,
        build: Some("cargo build".to_string()),
        lint: None,
        test: Some("cargo test".to_string()),
        lint_fix: None,
        format_fix: None,
    };
    let budget = Budget::new(1_000_000);

    let d = DepsBuilder::new(&client, &registry, &budget, 8, dir.path(), &capture)
        .verifier(&verifier)
        .commands(&commands)
        .runner(&runner)
        .build();
    execute_phase(&input(), d).await.unwrap();

    let events = capture.events();
    let stages: Vec<&str> = events.iter().map(|e| e.stage.as_str()).collect();
    assert!(
        stages.contains(&"command:build"),
        "expected command:build, got: {:?}",
        stages
    );
    assert!(
        stages.contains(&"command:test"),
        "expected command:test, got: {:?}",
        stages
    );
    assert!(
        !stages.contains(&"command:fmt"),
        "fmt was not configured, must not emit"
    );
}

#[tokio::test]
#[should_panic(expected = "panic in progress callback")]
async fn callback_panic_is_not_caught() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let budget = Budget::new(1_000_000);

    struct PanicCallback;
    impl ProgressCallback for PanicCallback {
        fn on_progress(&self, _event: &ProgressEvent) {
            panic!("panic in progress callback");
        }
    }

    let d = LoopDeps {
        client: &client,
        registry: &registry,
        tools: &[],
        budget: &budget,
        max_turns: 8,
        project_root: dir.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock_zero,
        verifier: &NoopVerifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams {
            temperature: None,
            seed: None,
        },
        telemetry_dir: None,
        progress: Some(&PanicCallback),
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    execute_phase(&input(), d).await.unwrap();
}

#[tokio::test]
async fn progress_independent_of_log_write_failure() {
    let dir = TempDir::new().unwrap();
    // `.rexymcp` as a file → sessions dir can't be created → log doesn't open.
    std::fs::write(dir.path().join(".rexymcp"), "x").unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let capture = CaptureCallback::new();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let budget = Budget::new(1_000_000);

    execute_phase(
        &input(),
        deps_with_progress_simple(&client, &registry, &budget, 8, dir.path(), &capture),
    )
    .await
    .unwrap();

    let events = capture.events();
    assert!(
        !events.is_empty(),
        "callback should still receive events even when log dir is unwritable"
    );
    assert!(
        events.iter().any(|e| e.stage == "turn_start"),
        "expected turn_start event despite log failure"
    );
}

// ── 07b: awaiting_model heartbeat ─────────────────────────────────────

use crate::ai::testing::MockAiClientPending;

use tokio::sync::Notify;

#[tokio::test]
async fn awaiting_model_emitted_before_model_call() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let budget = Budget::new(1_000_000);

    execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    let recs = records(dir.path());
    let awaiting: Vec<_> = recs
        .iter()
        .filter(|r| {
            matches!(
                &r.event,
                SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
            )
        })
        .collect();

    assert!(
        !awaiting.is_empty(),
        "expected at least one awaiting_model Progress record"
    );
    let first_awaiting_idx = recs
        .iter()
        .position(|r| {
            matches!(
                &r.event,
                SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
            )
        })
        .unwrap();
    let completion_idx = recs
        .iter()
        .position(|r| matches!(&r.event, SessionEvent::Completion { .. }))
        .unwrap();
    assert!(
        first_awaiting_idx < completion_idx,
        "awaiting_model must be logged before Completion"
    );
    if let SessionEvent::Progress { turn, .. } = &awaiting[0].event {
        assert_eq!(*turn, 1, "awaiting_model should be for turn 1 (upcoming)");
    }
}

#[tokio::test(start_paused = true)]
async fn heartbeat_reemits_awaiting_model_while_in_flight() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);

    let gate = Arc::new(Notify::new());
    let client = MockAiClientPending::new(vec![token("done")], gate.clone());
    let budget = Budget::new(1_000_000);

    let inp = input();
    let dir_path = dir.path().to_path_buf();
    let dir_path2 = dir_path.clone();

    let handle = tokio::spawn(async move {
        execute_phase(&inp, deps(&client, &registry, &budget, 8, &dir_path)).await
    });

    // Advance time by 3 heartbeat periods, yielding between each so the
    // loop processes the tick and writes its record.
    for _ in 0..3 {
        tokio::time::advance(HEARTBEAT_PERIOD).await;
        tokio::task::yield_now().await;
    }

    // While chat is still in flight, the session log should have
    // 1 pre-call + 3 heartbeat = 4 awaiting_model records.
    let recs_mid = records(&dir_path2);
    let awaiting_mid = recs_mid
        .iter()
        .filter(|r| {
            matches!(
                &r.event,
                SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
            )
        })
        .count();

    gate.notify_one();
    handle.await.unwrap().unwrap();

    assert_eq!(
        awaiting_mid, 4,
        "expected exactly 4 awaiting_model records (1 pre-call + 3 ticks), got {awaiting_mid}"
    );
}

#[tokio::test(start_paused = true)]
async fn heartbeat_stops_when_model_responds() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);

    let gate = Arc::new(Notify::new());
    let client = MockAiClientPending::new(vec![token("done")], gate.clone());
    let budget = Budget::new(1_000_000);

    let inp = input();
    let dir_path = dir.path().to_path_buf();
    let dir_path2 = dir_path.clone();

    let handle = tokio::spawn(async move {
        execute_phase(&inp, deps(&client, &registry, &budget, 8, &dir_path)).await
    });

    // Advance 2 heartbeat periods, yielding between each.
    for _ in 0..2 {
        tokio::time::advance(HEARTBEAT_PERIOD).await;
        tokio::task::yield_now().await;
    }

    let recs_before = records(&dir_path2);
    let count_before = recs_before
        .iter()
        .filter(|r| {
            matches!(
                &r.event,
                SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
            )
        })
        .count();

    // Release the gate so chat resolves.
    gate.notify_one();
    let result = handle.await.unwrap().unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);

    // Advance more time — no new awaiting_model records should appear.
    for _ in 0..3 {
        tokio::time::advance(HEARTBEAT_PERIOD).await;
        tokio::task::yield_now().await;
    }

    let recs_after = records(&dir_path2);
    let count_after = recs_after
        .iter()
        .filter(|r| {
            matches!(
                &r.event,
                SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
            )
        })
        .count();

    assert_eq!(
        count_before, count_after,
        "no new awaiting_model records should appear after chat resolves"
    );
}

// ── 09: Chat-stream provenance (phase-05b) ─────────────────────────

#[tokio::test]
async fn length_finish_rate_is_fraction_of_length_finishes() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");

    // Single turn emitting two Completion events (mock allows arbitrary event sequences)
    let turn1 = vec![
        AiEvent::Completion {
            finish_reason: Some("length".into()),
            model: Some("served-x".into()),
        },
        AiEvent::Completion {
            finish_reason: Some("stop".into()),
            model: None,
        },
        AiEvent::Done(TokenBreakdown::default()),
        AiEvent::Token("done".into()),
    ];
    let client = MockAiClientScript::new(vec![turn1]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    let runs = read_runs(&telem);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].length_finish_rate, Some(0.5));
    assert_eq!(runs[0].served_model, Some("served-x".into()));
}

#[tokio::test]
async fn length_finish_rate_none_when_no_finish_reasons() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");

    // No Completion events at all — only Token + Done
    let turn1 = vec![
        AiEvent::Token("done".into()),
        AiEvent::Done(TokenBreakdown::default()),
    ];
    let client = MockAiClientScript::new(vec![turn1]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    let runs = read_runs(&telem);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].length_finish_rate, None);
    assert_eq!(runs[0].served_model, None);
}

#[tokio::test]
async fn served_model_recorded_from_completion() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");

    let turn1 = vec![
        AiEvent::Completion {
            finish_reason: None,
            model: Some("served-model-v2".into()),
        },
        AiEvent::Done(TokenBreakdown::default()),
        AiEvent::Token("done".into()),
    ];
    let client = MockAiClientScript::new(vec![turn1]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
    )
    .await;

    let runs = read_runs(&telem);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].served_model, Some("served-model-v2".into()));
}

#[tokio::test]
async fn context_window_recorded_from_loop_deps() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");

    let turn1 = vec![
        AiEvent::Completion {
            finish_reason: None,
            model: Some("served-model-v2".into()),
        },
        AiEvent::Done(TokenBreakdown::default()),
        AiEvent::Token("done".into()),
    ];
    let client = MockAiClientScript::new(vec![turn1]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full_with_context_window(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
        Some(262_144),
    )
    .await;

    let runs = read_runs(&telem);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].context_window, Some(262_144));
}

#[tokio::test]
async fn phase_run_context_efficiency_matches_session_log() {
    let dir = TempDir::new().unwrap();
    let telem = dir.path().join("telem");

    let turn1 = vec![
        AiEvent::Completion {
            finish_reason: None,
            model: Some("served-model-v2".into()),
        },
        AiEvent::Done(TokenBreakdown::default()),
        AiEvent::Token("done".into()),
    ];
    let client = MockAiClientScript::new(vec![turn1]);
    let verifier = MockFileVerifier::new(vec![]);

    run_full_with_context_window(
        &dir,
        &client,
        &verifier,
        &NoopRunner,
        &EMPTY_COMMANDS,
        Some(&telem),
        8,
        Some(262_144),
    )
    .await;

    let runs = read_runs(&telem);
    assert_eq!(runs.len(), 1);
    let expected = crate::store::telemetry::aggregate_context_efficiency(&records(dir.path()));
    assert_eq!(runs[0].context_efficiency, expected);
}

// ── M9/phase-01: post-write format hook ─────────────────────────────

#[tokio::test]
async fn format_hook_runs_after_successful_edit() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "hello\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        format_fix: Some("echo fmt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    assert!(
        runner.ran().iter().any(|c| c == "echo fmt"),
        "expected format hook to fire after write_file, got: {:?}",
        runner.ran()
    );
}

#[tokio::test]
async fn format_hook_runs_before_verify() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "hello\n" }),
        )],
        vec![token("done")],
    ]);
    // Verifier that returns Checked with no diagnostics so a "verify" event is emitted.
    let verifier = MockFileVerifier::new(vec![VerifierResult::Checked {
        diagnostics: vec![],
    }]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        format_fix: Some("echo fmt".into()),
        ..EMPTY_COMMANDS
    };
    let capture = CaptureCallback::new();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let budget = Budget::new(1_000_000);
    let d = DepsBuilder::new(&client, &registry, &budget, 8, dir.path(), &capture)
        .verifier(&verifier)
        .commands(&commands)
        .runner(&runner)
        .build();

    let result = execute_phase(&input(), d).await.unwrap();
    assert_eq!(result.status, PhaseStatus::Complete);

    let events = capture.events();
    let stages: Vec<&str> = events.iter().map(|e| e.stage.as_str()).collect();

    // Find the first "format" and the first "verify".
    let format_pos = stages.iter().position(|&s| s == "format");
    let verify_pos = stages.iter().position(|&s| s == "verify");
    assert!(
        format_pos.is_some(),
        "expected a format progress event, got: {:?}",
        stages
    );
    assert!(
        verify_pos.is_some(),
        "expected a verify progress event, got: {:?}",
        stages
    );
    assert!(
        format_pos.unwrap() < verify_pos.unwrap(),
        "format must come before verify, stages: {:?}",
        stages
    );
}

#[tokio::test]
async fn format_hook_skipped_when_no_format_configured() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "hello\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");

    let result = run_full(&dir, &client, &verifier, &runner, &EMPTY_COMMANDS, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    assert!(
        runner.ran().is_empty(),
        "expected no commands when format is None, got: {:?}",
        runner.ran()
    );
}

#[tokio::test]
async fn format_hook_skipped_after_non_edit_call() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "existing").unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        format: Some("echo fmt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    // Final command set runs format once at completion; no hook invocation during read.
    assert_eq!(
        runner.ran().len(),
        1,
        "expected exactly 1 format run (final command set), got: {:?}",
        runner.ran()
    );
}

#[tokio::test]
async fn format_hook_skipped_after_failed_edit() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "original\n").unwrap();
    let path = file.to_string_lossy().to_string();
    // Script a patch without a prior read_file — the read-before-edit gate
    // refuses it (succeeded == false), so the hook should not fire.
    let client = MockAiClientScript::new(vec![
        vec![native(
            "patch",
            json!({ "path": path, "old_str": "original", "new_str": "edited" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        format: Some("echo fmt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    // Final command set runs format once at completion; no hook invocation for the
    // failed patch turn.
    assert_eq!(
        runner.ran().len(),
        1,
        "expected exactly 1 format run (final command set), got: {:?}",
        runner.ran()
    );
}

#[tokio::test]
async fn format_hook_failure_does_not_halt_turn() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "hello\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    // First call (post-write hook) fails — advisory, must not halt.
    // Second call (completion gate) passes — allows completion.
    let runner = ScriptedCommandRunner::new(vec![false, true]);
    let commands = CommandConfig {
        format_fix: Some("fmt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    assert!(
        result.briefing.is_none(),
        "expected no briefing (no hard_fail) on format hook failure"
    );
}

#[tokio::test]
async fn format_hook_runs_on_every_edit_turn() {
    let dir = TempDir::new().unwrap();
    let file1 = dir.path().join("a.txt");
    let file2 = dir.path().join("b.txt");
    let path1 = file1.to_string_lossy().to_string();
    let path2 = file2.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path1, "content": "one\n" }),
        )],
        vec![native(
            "write_file",
            json!({ "path": path2, "content": "two\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        format_fix: Some("echo fmt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    let count = runner.ran().iter().filter(|c| *c == "echo fmt").count();
    assert_eq!(
        count,
        2,
        "expected 2 format_fix hook runs (the completion command set runs format, which is unset), got {}: {:?}",
        count,
        runner.ran()
    );
}

#[tokio::test]
async fn hook_runs_lint_fix_before_format_fix() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "hello\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        lint_fix: Some("echo fix".into()),
        format_fix: Some("echo fmt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    let ran = runner.ran();
    // Hook fires lint_fix then format_fix.
    // Assert the first two invocations are in order: fix before fmt.
    assert!(
        ran.len() >= 2,
        "expected at least 2 runner invocations, got: {:?}",
        ran
    );
    assert_eq!(
        ran[0], "echo fix",
        "lint_fix must run before format_fix, got: {:?}",
        ran
    );
    assert_eq!(
        ran[1], "echo fmt",
        "format must run after lint_fix, got: {:?}",
        ran
    );
}

#[tokio::test]
async fn hook_skips_lint_fix_when_unconfigured() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "hello\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        lint_fix: None,
        format_fix: Some("echo fmt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    let ran = runner.ran();
    assert!(
        !ran.iter().any(|c| c == "echo fix"),
        "lint_fix must not run when unconfigured, got: {:?}",
        ran
    );
    assert!(
        ran.iter().any(|c| c == "echo fmt"),
        "format_fix must still run when lint_fix is None, got: {:?}",
        ran
    );
}

/// Crux test: the hook runs `format_fix` (writing form), not `format` (check form).
#[tokio::test]
async fn hook_runs_format_fix_not_the_check_form() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "hello\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok");
    let commands = CommandConfig {
        format: Some("echo CHECK".into()),
        format_fix: Some("echo FIX".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    let ran = runner.ran();

    // The hook must run format_fix ("echo FIX"), not the check form ("echo CHECK").
    let fix_pos = ran
        .iter()
        .position(|c| c == "echo FIX")
        .expect("hook should have run format_fix (echo FIX)");
    let check_pos = ran
        .iter()
        .position(|c| c == "echo CHECK")
        .expect("completion gate should have run format (echo CHECK)");
    assert!(
        fix_pos < check_pos,
        "format_fix (hook, pos {}) must run before format (gate, pos {})",
        fix_pos,
        check_pos
    );
}

#[tokio::test]
async fn lint_fix_failure_does_not_halt_turn() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("t.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "hello\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = MockCommandRunner::new("ok").failing("bad-fix");
    let commands = CommandConfig {
        lint_fix: Some("bad-fix".into()),
        format: Some("echo fmt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    assert!(
        result.briefing.is_none(),
        "expected no hard_fail on lint_fix failure"
    );
}

#[tokio::test]
async fn loop_emits_output_filtered_event_for_filtered_bash() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(read_file(scope.clone()));
    registry.register(write_file(scope.clone()));
    registry.register(patch(scope.clone()));
    registry.register(bash_with_filter(scope, 30, true));

    // Script: one bash call producing >100 lines, then done.
    let client = MockAiClientScript::new(vec![
        vec![native(
            "bash",
            json!({ "command": "sh -c 'for i in $(seq 1 200); do echo \"line $i\"; done'" }),
        )],
        vec![token("done")],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(
        &input(),
        LoopDeps {
            client: &client,
            registry: &registry,
            tools: &[],
            budget: &budget,
            max_turns: 8,
            project_root: dir.path(),
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier: &NoopVerifier,
            commands: &EMPTY_COMMANDS,
            runner: &NoopRunner,
            generation_params: GenerationParams::default(),
            telemetry_dir: None,
            progress: None,
            context_window: None,
            governor: GovernorConfig::default(),
            task_tracking: true,
            gate_retries: u32::MAX,
            wall_clock_secs: 0,
            cancel: CancelSignal::never(),
        },
    )
    .await
    .unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);

    let recs = records(dir.path());
    let filtered_recs: Vec<_> = recs
        .iter()
        .filter_map(|r| {
            if let SessionEvent::OutputFiltered {
                tokens_before,
                tokens_after,
                filter,
            } = &r.event
            {
                Some((*tokens_before, *tokens_after, filter.clone()))
            } else {
                None
            }
        })
        .collect();

    assert!(
        !filtered_recs.is_empty(),
        "expected at least one OutputFiltered record, got {} total records: {:?}",
        recs.len(),
        recs.iter()
            .map(|r| event_kind(&r.event))
            .collect::<Vec<_>>()
    );

    let (before, after, filter_name) = filtered_recs.first().unwrap();
    assert!(
        *after < *before,
        "tokens_after ({after}) should be less than tokens_before ({before})"
    );
    assert_eq!(filter_name, "generic");
}

// ── M10 phase-06: redundant-read dedupe ───────────────────────────────

#[tokio::test]
async fn loop_dedupes_unchanged_reread() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("foo.txt");
    let content = "this is a sufficiently large file content that we can verify \
                       tokens_saved is positive when the dedupe reference replaces \
                       the full content in the tool result returned to the model on \
                       the second read call of the same unchanged file this session";
    std::fs::write(&file, content).unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        // Turn 1: first read of foo.txt
        vec![native("read_file", json!({ "path": path }))],
        // Turn 2: second read of the same file (unchanged)
        vec![native("read_file", json!({ "path": path }))],
        // Turn 3: done
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    // After the second read (turn 2), the model call at index 2 should
    // contain a read_file tool result starting with [already-read:
    let second_call_messages = &client.calls()[2].messages;
    let has_dedupe_ref = second_call_messages.iter().any(|m| {
        m.tool_results.as_ref().is_some_and(|trs| {
            trs.iter()
                .any(|t| t.tool_name == "read_file" && t.content.starts_with("[already-read:"))
        })
    });
    assert!(
        has_dedupe_ref,
        "second read of unchanged file should return an [already-read: reference"
    );
}

#[tokio::test]
async fn loop_logs_read_deduped_event() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("foo.txt");
    let content = "this is a sufficiently large file content that we can verify \
                       tokens_saved is positive when the dedupe reference replaces \
                       the full content in the tool result returned to the model on \
                       the second read call of the same unchanged file this session";
    std::fs::write(&file, content).unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![native("read_file", json!({ "path": path }))],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    let recs = records(dir.path());
    let has_read_deduped = recs.iter().any(|r| {
        matches!(
            &r.event,
            SessionEvent::ReadDeduped { tokens_saved, .. } if *tokens_saved > 0
        )
    });
    assert!(
        has_read_deduped,
        "session log should contain a ReadDeduped event with tokens_saved > 0"
    );
}

#[tokio::test]
async fn loop_does_not_dedupe_after_edit() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("foo.txt");
    let original_content = "original file content for dedupe after edit testing";
    std::fs::write(&file, original_content).unwrap();
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        // Turn 1: first read
        vec![native("read_file", json!({ "path": path }))],
        // Turn 2: write_file changes the file (mtime changes, prior read superseded)
        vec![native(
            "write_file",
            json!({ "path": path, "content": "new content that is different and long enough" }),
        )],
        // Turn 3: re-read after edit — should NOT be deduped
        vec![native("read_file", json!({ "path": path }))],
        // Turn 4: done
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);

    run_with_verifier(&dir, &client, &verifier, 8).await;

    // After the re-read (turn 3), the model call at index 3 should contain
    // a read_file tool result that does NOT start with [already-read:
    let third_call_messages = &client.calls()[3].messages;
    let read_file_result = third_call_messages.iter().find_map(|m| {
        m.tool_results.as_ref().and_then(|trs| {
            trs.iter()
                .find(|t| t.tool_name == "read_file")
                .map(|t| t.content.as_str())
        })
    });
    assert!(
        read_file_result.is_some(),
        "there should be a read_file tool result in the third call messages"
    );
    let content = read_file_result.unwrap();
    assert!(
        !content.starts_with("[already-read:"),
        "re-read after edit should NOT be deduped (mtime changed + prior read evicted)"
    );

    // No ReadDeduped event should have been logged
    let recs = records(dir.path());
    let has_read_deduped = recs
        .iter()
        .any(|r| matches!(&r.event, SessionEvent::ReadDeduped { .. }));
    assert!(
        !has_read_deduped,
        "no ReadDeduped event should be logged when the file was edited between reads"
    );
}

#[tokio::test]
async fn loop_seeds_task_updates_from_spec() {
    let dir = TempDir::new().unwrap();
    let phase_doc = "## Spec\n\n1. **First task** — do this\n2. **Second task** — do that\n3. **Third** — last\n";
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
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
        clock: &clock_zero,
        verifier: &verifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        ..input()
    };
    let _ = execute_phase(&input, d).await.unwrap();

    let recs = records(dir.path());
    let task_updates: Vec<_> = recs
        .iter()
        .filter(|r| matches!(&r.event, SessionEvent::TaskUpdate { .. }))
        .collect();

    assert_eq!(
        task_updates.len(),
        3,
        "expected exactly 3 task_update records"
    );

    for (i, rec) in task_updates.iter().enumerate() {
        if let SessionEvent::TaskUpdate { state, .. } = &rec.event {
            assert_eq!(
                *state,
                crate::store::sessions::event::TaskState::Pending,
                "task {} should be Pending",
                i
            );
        } else {
            panic!("expected TaskUpdate, got {:?}", rec.event);
        }
    }

    assert_eq!(task_updates[0].turn, 0, "task updates should be at turn 0");
}

#[tokio::test]
async fn loop_emits_no_task_updates_when_spec_absent() {
    let dir = TempDir::new().unwrap();
    let phase_doc = "# No spec here\n\nSome random text.\n";
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let verifier = MockFileVerifier::new(vec![]);

    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
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
        clock: &clock_zero,
        verifier: &verifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        ..input()
    };
    let _ = execute_phase(&input, d).await.unwrap();

    let recs = records(dir.path());
    let task_updates: Vec<_> = recs
        .iter()
        .filter(|r| matches!(&r.event, SessionEvent::TaskUpdate { .. }))
        .collect();

    assert!(
        task_updates.is_empty(),
        "expected zero task_update records when no ## Spec section"
    );
}

// ── 06b: task-tracking gate ─────────────────────────────────────────────

#[tokio::test]
async fn loop_emits_no_task_updates_when_tracking_off() {
    let dir = TempDir::new().unwrap();
    let phase_doc =
        "## Spec\n\n1. **First task** — do this\n2. Second task — do that\n3. **Third** — last\n";
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let _verifier = MockFileVerifier::new(vec![]);

    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let budget = Budget::new(1_000_000);
    let mut d = deps(&client, &registry, &budget, 8, dir.path());
    d.task_tracking = false;

    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        ..input()
    };
    let _ = execute_phase(&input, d).await.unwrap();

    let recs = records(dir.path());
    let task_updates: Vec<_> = recs
        .iter()
        .filter(|r| matches!(&r.event, SessionEvent::TaskUpdate { .. }))
        .collect();

    assert!(
        task_updates.is_empty(),
        "expected zero task_update records when task_tracking is off (got {})",
        task_updates.len()
    );
}

#[tokio::test]
async fn loop_still_seeds_task_updates_when_tracking_on() {
    let dir = TempDir::new().unwrap();
    let phase_doc = "## Spec\n\n1. **First task** — do this\n2. **Second task** — do that\n3. **Third** — last\n";
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let _verifier = MockFileVerifier::new(vec![]);

    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let budget = Budget::new(1_000_000);
    let d = deps(&client, &registry, &budget, 8, dir.path());

    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        ..input()
    };
    let _ = execute_phase(&input, d).await.unwrap();

    let recs = records(dir.path());
    let task_updates: Vec<_> = recs
        .iter()
        .filter(|r| matches!(&r.event, SessionEvent::TaskUpdate { .. }))
        .collect();

    assert_eq!(
        task_updates.len(),
        3,
        "expected exactly 3 task_update records when task_tracking is on"
    );

    for (i, rec) in task_updates.iter().enumerate() {
        if let SessionEvent::TaskUpdate { state, .. } = &rec.event {
            assert_eq!(
                *state,
                crate::store::sessions::event::TaskState::Pending,
                "task {} should be Pending",
                i
            );
        } else {
            panic!("expected TaskUpdate, got {:?}", rec.event);
        }
    }

    assert_eq!(task_updates[0].turn, 0, "task updates should be at turn 0");
}

// ── 06c: model-facing task flips ────────────────────────────────────────

/// Build a registry that includes `update_task` seeded from a spec doc.
fn registry_with_update_task(scope: Scope, tasks: Vec<crate::agent::tasks::Task>) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(read_file(scope.clone()));
    r.register(write_file(scope.clone()));
    r.register(patch(scope.clone()));
    r.register(crate::tools::update_task(tasks));
    r
}

#[tokio::test]
async fn loop_emits_task_update_when_model_flips_task() {
    let dir = TempDir::new().unwrap();
    let phase_doc = "## Spec\n\n1. **First task** — do this\n2. Second task — do that\n";
    let client = MockAiClientScript::new(vec![vec![native(
        "update_task",
        json!({ "id": "1", "state": "active" }),
    )]]);

    let scope = Scope::new(dir.path()).unwrap();
    let tasks = crate::agent::tasks::seed_from_spec(phase_doc);
    let registry = registry_with_update_task(scope, tasks);
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
        clock: &clock_zero,
        verifier: &NoopVerifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        ..input()
    };
    let _ = execute_phase(&input, d).await.unwrap();

    let recs = records(dir.path());
    let active_updates: Vec<_> = recs
        .iter()
        .filter(|r| {
            if let SessionEvent::TaskUpdate { state, .. } = &r.event {
                *state != crate::store::sessions::event::TaskState::Pending
            } else {
                false
            }
        })
        .collect();

    assert_eq!(
        active_updates.len(),
        1,
        "expected exactly one model-driven task_update (active) beyond the turn-0 pending seeds"
    );

    if let SessionEvent::TaskUpdate { id, title, state } = &active_updates[0].event {
        assert_eq!(id, "1");
        assert_eq!(title, "First task");
        assert_eq!(*state, crate::store::sessions::event::TaskState::Active);
    } else {
        panic!("expected TaskUpdate, got {:?}", active_updates[0].event);
    }
}

#[tokio::test]
async fn loop_prompt_omits_task_section_when_tracking_off() {
    let dir = TempDir::new().unwrap();
    let phase_doc = "## Spec\n\n1. **First task** — do this\n2. Second task — do that\n";
    let client = MockAiClientScript::new(vec![vec![token("done")]]);

    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let budget = Budget::new(1_000_000);
    let mut d = deps(&client, &registry, &budget, 8, dir.path());
    d.task_tracking = false;

    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        ..input()
    };
    let _ = execute_phase(&input, d).await.unwrap();

    let recs = records(dir.path());
    let prompt_recs: Vec<_> = recs
        .iter()
        .filter(|r| matches!(&r.event, SessionEvent::Prompt { .. }))
        .collect();

    assert!(
        !prompt_recs.is_empty(),
        "expected at least one Prompt record"
    );
    for rec in prompt_recs {
        if let SessionEvent::Prompt { rendered, .. } = &rec.event {
            assert!(
                !rendered.contains("# Task tracking"),
                "system prompt must not contain '# Task tracking' when task_tracking is off"
            );
        }
    }
}

#[tokio::test]
async fn loop_prompt_includes_task_section_when_tracking_on() {
    let dir = TempDir::new().unwrap();
    let phase_doc = "## Spec\n\n1. **First task** — do this\n2. Second task — do that\n";
    let client = MockAiClientScript::new(vec![vec![token("done")]]);

    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let budget = Budget::new(1_000_000);
    let d = deps(&client, &registry, &budget, 8, dir.path());

    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        ..input()
    };
    let _ = execute_phase(&input, d).await.unwrap();

    let recs = records(dir.path());
    let prompt_recs: Vec<_> = recs
        .iter()
        .filter(|r| matches!(&r.event, SessionEvent::Prompt { .. }))
        .collect();

    assert!(
        !prompt_recs.is_empty(),
        "expected at least one Prompt record"
    );
    let has_task_section = prompt_recs.iter().any(|rec| {
        if let SessionEvent::Prompt { rendered, .. } = &rec.event {
            rendered.contains("# Task tracking") && rendered.contains("First task")
        } else {
            false
        }
    });
    assert!(
        has_task_section,
        "system prompt must contain '# Task tracking' and seeded task titles when task_tracking is on"
    );
}

// ── M14-01: empty-spec warning ─────────────────────────────────────────

#[tokio::test]
async fn mod_emits_progress_warning_when_task_tracking_on_and_no_tasks() {
    use crate::store::sessions::event::SessionEvent;

    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Mock AI responds immediately with a final message (no tool calls)
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let budget = Budget::new(1_000_000);

    let phase_doc =
        "# Phase\n\n## Goal\n\nNo spec items here.\n\n## Acceptance criteria\n\n- [ ] passes\n";
    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        ..input()
    };
    let d = deps(&client, &registry, &budget, 8, dir.path());

    let _ = execute_phase(&input, d).await.unwrap();

    let recs = records(dir.path());
    let warning = recs.iter().find(|r| {
        matches!(
            &r.event,
            SessionEvent::Progress { turn: 0, stage, .. } if stage == "task_seeding"
        )
    });
    assert!(
        warning.is_some(),
        "expected a task_seeding Progress warning at turn 0"
    );
    if let Some(rec) = warning
        && let SessionEvent::Progress { message, .. } = &rec.event
    {
        assert!(!message.is_empty(), "warning message must not be empty");
    }
}

#[tokio::test]
async fn gate_failure_loops_until_gates_pass() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Two "done" completions: first fails gates, second passes.
    let client = MockAiClientScript::new(vec![vec![token("All done.")], vec![token("All done.")]]);
    let budget = Budget::new(1_000_000);
    let commands = all_commands_configured();
    // 4 failures then 4 passes.
    let runner =
        ScriptedCommandRunner::new(vec![false, false, false, false, true, true, true, true]);
    let mut d = deps(&client, &registry, &budget, 8, dir.path());
    d.commands = &commands;
    d.runner = &runner;

    let result = execute_phase(&input(), d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);
    // Two model calls: the first completion triggered a gate-retry turn.
    assert_eq!(client.calls().len(), 2);
}

#[tokio::test]
async fn gate_failure_at_turn_cap_is_budget_exceeded() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let client = MockAiClientScript::new(vec![vec![token("All done.")]]);
    let budget = Budget::new(1_000_000);
    let commands = all_commands_configured();
    // All gates always fail.
    let runner = ScriptedCommandRunner::new(vec![false, false, false, false]);
    let mut d = deps(&client, &registry, &budget, 1, dir.path()); // max_turns = 1
    d.commands = &commands;
    d.runner = &runner;

    let result = execute_phase(&input(), d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::BudgetExceeded);
}

#[tokio::test]
async fn gate_retry_budget_exhaustion_returns_budget_exceeded_before_turn_cap() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Model always returns a completion (no tool calls), triggering gate check each turn.
    // Scripted for 4 productive turns — enough to cover initial + 2 retries; an
    // exhausted script would fall through to the (unrelated) empty-completion
    // recovery path instead of re-running the gate check.
    let client = MockAiClientScript::new(vec![
        vec![token("All done.")],
        vec![token("All done.")],
        vec![token("All done.")],
        vec![token("All done.")],
    ]);
    let budget = Budget::new(1_000_000);
    let commands = all_commands_configured();
    // All gates always fail: 4 commands × enough turns for initial + 2 retries.
    let runner = ScriptedCommandRunner::new(vec![
        false, false, false, false, // turn 1
        false, false, false, false, // retry 1
        false, false, false, false, // retry 2 (budget exhausted)
    ]);
    let mut d = deps(&client, &registry, &budget, 50, dir.path()); // max_turns = 50
    d.commands = &commands;
    d.runner = &runner;
    d.gate_retries = 2;
    d.governor.gate_feedback_repeat_threshold = usize::MAX; // disable A3 hard-fail
    d.governor.empty_completion_threshold = usize::MAX; // disable empty-completion hard-fail

    let result = execute_phase(&input(), d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::BudgetExceeded);
    // With gate_retries=2 and max_turns=50, termination should happen at the
    // retry budget (3 model calls: initial completion + 2 retries), not at the
    // turn cap (50 calls).
    assert!(
        client.calls().len() <= 5,
        "expected ~3 calls but got {}",
        client.calls().len()
    );
}

#[tokio::test]
async fn unlimited_gate_retries_retries_to_turn_cap() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Model always returns a completion (no tool calls), triggering gate check each turn.
    // Scripted for 3 productive turns to match max_turns — an exhausted script
    // would fall through to the empty-completion recovery path instead.
    let client = MockAiClientScript::new(vec![
        vec![token("All done.")],
        vec![token("All done.")],
        vec![token("All done.")],
    ]);
    let budget = Budget::new(1_000_000);
    let commands = all_commands_configured();
    // All gates always fail: 4 commands × enough turns.
    let runner = ScriptedCommandRunner::new(vec![
        false, false, false, false, // turn 1
        false, false, false, false, // turn 2
        false, false, false, false, // turn 3
    ]);
    let mut d = deps(&client, &registry, &budget, 3, dir.path()); // max_turns = 3
    d.commands = &commands;
    d.runner = &runner;
    d.gate_retries = u32::MAX; // unlimited

    let result = execute_phase(&input(), d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::BudgetExceeded);
    // With unlimited gate_retries and max_turns=3, the loop should reach the
    // turn cap (3 model calls), not short-circuit on the retry budget.
    assert!(
        client.calls().len() >= 3,
        "expected >=3 calls but got {}",
        client.calls().len()
    );
}

#[tokio::test]
async fn task_coverage_check_loops_until_all_tasks_done() {
    use crate::tools::update_task as make_update_task;

    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();

    // Phase doc with one seeded task.
    let phase_doc = "## Spec\n\n1. **Foo** — do the thing.\n";
    let seeded_tasks = tasks::seed_from_spec(phase_doc);

    // Registry with update_task so the tool call actually resolves.
    let mut registry = registry_over(scope);
    registry.register(make_update_task(seeded_tasks));

    let commands = all_commands_configured();
    // Turn 1: premature complete (no update_task call).
    // Turn 2: update_task → marks task 1 done.
    // Turn 3: true complete (all tasks done).
    let client = MockAiClientScript::new(vec![
        vec![token("All done.")],
        vec![native("update_task", json!({"id": "1", "state": "done"}))],
        vec![token("All done.")],
    ]);
    let budget = Budget::new(1_000_000);

    let mut inp = input();
    inp.phase_doc = phase_doc.to_string();

    let mut d = deps(&client, &registry, &budget, 8, dir.path());
    d.commands = &commands;
    d.runner = &NoopRunner; // gates always pass

    let result = execute_phase(&inp, d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);
    // Three model calls: premature complete → task coverage retry turn →
    // update_task turn → true complete.
    assert_eq!(client.calls().len(), 3);
}

#[tokio::test]
async fn task_coverage_check_at_turn_cap_is_budget_exceeded() {
    use crate::tools::update_task as make_update_task;

    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();

    let phase_doc = "## Spec\n\n1. **Foo** — do the thing.\n";
    let seeded_tasks = tasks::seed_from_spec(phase_doc);

    let mut registry = registry_over(scope);
    registry.register(make_update_task(seeded_tasks));

    let commands = all_commands_configured();
    // Only one model turn: premature complete at the turn cap.
    let client = MockAiClientScript::new(vec![vec![token("All done.")]]);
    let budget = Budget::new(1_000_000);

    let mut inp = input();
    inp.phase_doc = phase_doc.to_string();

    let mut d = deps(&client, &registry, &budget, 1, dir.path()); // max_turns = 1
    d.commands = &commands;
    d.runner = &NoopRunner;

    let result = execute_phase(&inp, d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::BudgetExceeded);
}

// --- M22 phase-01: empty-completion stall tests ---

#[tokio::test]
async fn empty_completions_hard_fail_at_threshold() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Script 5 empty-completion turns — the governor threshold is 3, so the
    // loop must hard_fail on turn 3 (not burn to the turn cap).
    let client = MockAiClientScript::new(vec![
        vec![token("")],
        vec![token("")],
        vec![token("")],
        vec![token("")], // never reached
        vec![token("")], // never reached
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 20, dir.path()))
        .await
        .unwrap();

    assert_eq!(result.status, PhaseStatus::HardFail);
    // The stall fires on the 3rd empty completion, not the turn cap.
    assert_eq!(client.calls().len(), 3);
}

#[tokio::test]
async fn single_empty_completion_then_recovers_does_not_hard_fail() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let path = dir.path().join("f.txt").to_string_lossy().to_string();
    // Turn 1: empty completion (counter → 1, no stall).
    // Turn 2: real tool call (counter resets to 0).
    // Turn 3: clean text completion → Complete.
    let client = MockAiClientScript::new(vec![
        vec![token("")],
        vec![native("read_file", json!({ "path": path }))],
        vec![token("now I'm done")],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);
    assert_eq!(client.calls().len(), 3);
}

// --- M22 phase-02: stuck gate-feedback stall tests ---

#[tokio::test]
async fn stuck_task_coverage_feedback_hard_fails() {
    use crate::tools::update_task as make_update_task;

    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();

    // Phase doc with one seeded task.
    let phase_doc = "## Spec\n\n1. **Foo** — do the thing.\n";
    let seeded_tasks = tasks::seed_from_spec(phase_doc);

    // Registry with update_task so the tool call actually resolves.
    let mut registry = registry_over(scope);
    registry.register(make_update_task(seeded_tasks));

    let commands = all_commands_configured();
    // The model returns a completion signal on every turn and never calls
    // update_task, so task_coverage_feedback fires identically each turn.
    // With the default gate_feedback_repeat_threshold of 5, the loop must
    // hard_fail at turn 5 (not burn to the turn cap).
    let client = MockAiClientScript::new(vec![
        vec![token("All done.")],
        vec![token("All done.")],
        vec![token("All done.")],
        vec![token("All done.")],
        vec![token("All done.")],
        vec![token("All done.")], // never reached — stall fires at 5
        vec![token("All done.")], // never reached
    ]);
    let budget = Budget::new(1_000_000);

    let mut inp = input();
    inp.phase_doc = phase_doc.to_string();

    let mut d = deps(&client, &registry, &budget, 20, dir.path());
    d.commands = &commands;
    d.runner = &NoopRunner; // gates always pass, so task_coverage_feedback fires

    let result = execute_phase(&inp, d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::HardFail);
    // The stall fires at the threshold (5), not the turn cap (20).
    assert_eq!(client.calls().len(), 5);
}

// ── M22 phase-05: self-revert guard ──────────────────────────────────

#[tokio::test]
async fn self_revert_of_edited_file_is_refused() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("src/flow.ts");
    file.parent().map(|p| std::fs::create_dir_all(p).ok());
    std::fs::write(&file, "original content").unwrap();
    let path = file.to_string_lossy().to_string();

    let scope = Scope::new(dir.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(read_file(scope.clone()));
    registry.register(write_file(scope.clone()));
    registry.register(patch(scope.clone()));
    registry.register(bash_with_filter(scope, 30, true));

    // Turn 1: read_file to unlock the file for write
    // Turn 2: write_file edits the file (puts it in pre_edit_content)
    // Turn 3: bash git checkout of that file → should be refused
    // Turn 4: done
    let client = MockAiClientScript::new(vec![
        vec![native("read_file", json!({ "path": path }))],
        vec![native(
            "write_file",
            json!({ "path": path, "content": "edited content" }),
        )],
        vec![native(
            "bash",
            json!({ "command": "git checkout src/flow.ts" }),
        )],
        vec![token("done")],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(
        &input(),
        LoopDeps {
            client: &client,
            registry: &registry,
            tools: &[],
            budget: &budget,
            max_turns: 8,
            project_root: dir.path(),
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier: &NoopVerifier,
            commands: &EMPTY_COMMANDS,
            runner: &NoopRunner,
            generation_params: GenerationParams::default(),
            telemetry_dir: None,
            progress: None,
            context_window: None,
            governor: GovernorConfig::default(),
            task_tracking: true,
            gate_retries: u32::MAX,
            wall_clock_secs: 0,
            cancel: CancelSignal::never(),
        },
    )
    .await
    .unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);

    // The fourth model call (index 3) should contain the refusal in the bash
    // tool result — the run continues, it's not a hard_fail.
    let fourth_call_messages = &client.calls()[3].messages;
    let has_refusal = fourth_call_messages.iter().any(|m| {
        m.tool_results.as_ref().is_some_and(|trs| {
            trs.iter().any(|t| {
                t.tool_name == "bash"
                    && t.content.contains("refusing to run")
                    && t.content.contains("discard your uncommitted edits")
            })
        })
    });
    assert!(
        has_refusal,
        "the git checkout of an edited file should be refused with a model-visible message"
    );
}

// --- M23 phase-02: truncation-aware empty-completion recovery tests ---

#[tokio::test]
async fn truncated_turn_is_not_treated_as_completion() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Turn 1: real reasoning tokens + finish_reason == "length", no tool call.
    // Before the fix, this would fall through to "declare completion".
    // After the fix, it is re-prompted with truncation feedback.
    // Turn 2: clean text completion → Complete.
    let client = MockAiClientScript::new(vec![
        vec![
            token("I think the answer is 42 because of the following reasoning..."),
            AiEvent::Completion {
                finish_reason: Some("length".into()),
                model: None,
            },
        ],
        vec![token("All done now.")],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    // The run did NOT finish on turn 1 — it was re-prompted and reached turn 2.
    assert_eq!(result.status, PhaseStatus::Complete);
    assert_eq!(client.calls().len(), 2);
}

#[tokio::test]
async fn repeated_truncation_reaches_turn_cap_not_completion() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Every scripted turn is truncated (finish_reason == "length", no tool call).
    // With max_turns = 3, the loop must hit budget_exceeded, not Complete.
    let client = MockAiClientScript::new(vec![
        vec![
            token("reasoning text 1..."),
            AiEvent::Completion {
                finish_reason: Some("length".into()),
                model: None,
            },
        ],
        vec![
            token("reasoning text 2..."),
            AiEvent::Completion {
                finish_reason: Some("length".into()),
                model: None,
            },
        ],
        vec![
            token("reasoning text 3..."),
            AiEvent::Completion {
                finish_reason: Some("length".into()),
                model: None,
            },
        ],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input(), deps(&client, &registry, &budget, 3, dir.path()))
        .await
        .unwrap();

    // Bounded by the turn cap, not a completion.
    assert_eq!(result.status, PhaseStatus::BudgetExceeded);
}

/// End-to-end: the post-write hook's `format_fix` actually rewrites the file on disk
/// via a real subprocess (not a mock).
#[tokio::test]
async fn format_fix_hook_rewrites_file_on_disk() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    let path = file.to_string_lossy().to_string();
    let client = MockAiClientScript::new(vec![
        vec![native(
            "write_file",
            json!({ "path": path, "content": "unformatted\n" }),
        )],
        vec![token("done")],
    ]);
    let verifier = MockFileVerifier::new(vec![]);
    let runner = RealCommandRunner;
    let commands = CommandConfig {
        format_fix: Some("printf 'formatted\\n' > f.txt".into()),
        ..EMPTY_COMMANDS
    };

    let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

    assert_eq!(result.status, PhaseStatus::Complete);
    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        on_disk, "formatted\n",
        "post-write hook's format_fix must rewrite the file on disk, got: {on_disk:?}"
    );
}

/// A deterministic clock that advances 10 seconds per call, so any nonzero
/// `wall_clock_secs` ceiling is crossed after the first loop iteration.
fn advancing_clock() -> impl Fn() -> u64 + Send + Sync {
    let calls = std::sync::atomic::AtomicU64::new(0);
    move || calls.fetch_add(10_000, std::sync::atomic::Ordering::Relaxed)
}

#[tokio::test]
async fn wall_clock_ceiling_trips_budget_exceeded() {
    let dir = tempfile::tempdir().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let registry = registry_over(scope);
    let budget = Budget::new(1_000_000);
    let verifier = MockFileVerifier::new(vec![]);
    let clock = advancing_clock();
    let d = LoopDeps {
        client: &client,
        registry: &registry,
        tools: &[],
        budget: &budget,
        max_turns: 200,
        project_root: dir.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock,
        verifier: &verifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 1,
        cancel: CancelSignal::never(),
    };
    let result = execute_phase(&input(), d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::BudgetExceeded);
    let briefing = result.briefing.unwrap();
    assert!(matches!(briefing.current_blocker, Blocker::BudgetExceeded));
    assert!(
        briefing.budget_remaining.contains("wall-clock"),
        "budget_remaining should mention wall-clock, got: {}",
        briefing.budget_remaining
    );
}

#[tokio::test]
async fn wall_clock_disabled_when_zero_completes() {
    let dir = tempfile::tempdir().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let client = MockAiClientScript::new(vec![vec![token("done")]]);
    let registry = registry_over(scope);
    let budget = Budget::new(1_000_000);
    let verifier = MockFileVerifier::new(vec![]);
    let clock = advancing_clock();
    let d = LoopDeps {
        client: &client,
        registry: &registry,
        tools: &[],
        budget: &budget,
        max_turns: 200,
        project_root: dir.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock,
        verifier: &verifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams::default(),
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: CancelSignal::never(),
    };
    let result = execute_phase(&input(), d).await.unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);
    assert!(result.briefing.is_none());
}

#[tokio::test]
async fn restored_states_override_seeded_pending() {
    use crate::tools::update_task as make_update_task;

    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();

    let phase_doc = "\
## Spec

1. **First task** — do this first
2. **Second task** — do this second
";
    let seeded_tasks = tasks::seed_from_spec(phase_doc);

    // Registry with update_task so the tool call actually resolves.
    let mut registry = registry_over(scope);
    registry.register(make_update_task(seeded_tasks.clone()));

    let input = PhaseInput {
        phase_doc: phase_doc.to_string(),
        resumed_task_states: Some(
            [(
                "1".to_string(),
                crate::store::sessions::event::TaskState::Done,
            )]
            .into_iter()
            .collect(),
        ),
        ..input()
    };
    // Turn 1: mark task 2 as done (task 1 is already done from restore)
    // Turn 2: complete
    let client = MockAiClientScript::new(vec![
        vec![native("update_task", json!({"id": "2", "state": "done"}))],
        vec![token("all done")],
    ]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&input, deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);

    let recs = records(dir.path());
    let task_updates: Vec<_> = recs
        .iter()
        .filter(|r| matches!(&r.event, SessionEvent::TaskUpdate { .. }))
        .collect();

    assert_eq!(
        task_updates.len(),
        3,
        "expected 2 seed + 1 tool-call task_update records"
    );

    // Build last-write-wins map from all task updates.
    let mut states: HashMap<String, crate::store::sessions::event::TaskState> = HashMap::new();
    for rec in &task_updates {
        if let SessionEvent::TaskUpdate { id, state, .. } = &rec.event {
            states.insert(id.clone(), *state);
        }
    }
    // Both tasks should be Done at the end.
    assert_eq!(
        *states.get("1").unwrap(),
        crate::store::sessions::event::TaskState::Done,
        "task 1 should be restored as Done"
    );
    assert_eq!(
        *states.get("2").unwrap(),
        crate::store::sessions::event::TaskState::Done,
        "task 2 should be marked Done by update_task tool call"
    );

    // Verify the seed events (first two) reflect the restored state:
    // task 1 seeded as Done, task 2 seeded as Pending.
    let seed_updates: Vec<_> = task_updates.iter().take(2).collect();
    for rec in &seed_updates {
        if let SessionEvent::TaskUpdate { id, state, .. } = &rec.event {
            if id == "1" {
                assert_eq!(
                    *state,
                    crate::store::sessions::event::TaskState::Done,
                    "seed: task 1 should be restored as Done"
                );
            } else if id == "2" {
                assert_eq!(
                    *state,
                    crate::store::sessions::event::TaskState::Pending,
                    "seed: task 2 should remain Pending"
                );
            }
        }
    }
}

#[tokio::test]
async fn loop_returns_cancelled_when_signal_flipped_between_turns() {
    let root = TempDir::new().unwrap();
    let (handle, signal) = CancelSignal::new();
    // Script: turn 1 writes a file, turn 2 sends nothing (done).
    let script = MockAiClientScript::new(vec![
        vec![
            AiEvent::ToolCallGeneric {
                id: "tc1".to_string(),
                name: "write_file".to_string(),
                args: json!({"path": "foo.txt", "content": "hello\n"}),
                thought_signature: None,
            },
            AiEvent::Done(TokenBreakdown {
                input_tokens: 10,
                output_tokens: 20,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }),
        ],
        vec![AiEvent::Done(TokenBreakdown {
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        })],
    ]);
    let client = Arc::new(script);
    let scope = Scope::new(root.path()).unwrap();
    let budget = Budget::default();
    let deps = LoopDeps {
        client: &*client,
        registry: &registry_over(scope.clone()),
        tools: &[],
        budget: &budget,
        max_turns: 10,
        project_root: root.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock_zero,
        verifier: &NoopVerifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams {
            temperature: None,
            seed: None,
        },
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: signal,
    };
    let input = input();
    // Signal is already flipped — the first top-of-loop check fires immediately.
    handle.cancel();
    let result = execute_phase(&input, deps).await.unwrap();
    assert_eq!(result.status, PhaseStatus::Cancelled);
    assert!(result.cancellation.is_some());
    let c = result.cancellation.as_ref().unwrap();
    assert_eq!(c.stage, "between_turns");
}

struct CancelThenPark {
    handle: CancelHandle,
}

#[async_trait::async_trait]
impl AiClient for CancelThenPark {
    async fn chat(
        &self,
        _system_prompt: &str,
        _messages: Vec<Message>,
        _tx: UnboundedSender<AiEvent>,
        _tools: Option<&[ToolSchema]>,
    ) -> anyhow::Result<()> {
        self.handle.cancel();
        std::future::pending::<()>().await;
        Ok(())
    }
}

#[tokio::test]
async fn loop_returns_cancelled_when_signal_flipped_mid_stream() {
    let root = TempDir::new().unwrap();
    let (handle, signal) = CancelSignal::new();
    let client = CancelThenPark { handle };
    let scope = Scope::new(root.path()).unwrap();
    let budget = Budget::default();
    let deps = LoopDeps {
        client: &client,
        registry: &registry_over(scope),
        tools: &[],
        budget: &budget,
        max_turns: 10,
        project_root: root.path(),
        model: "test-model",
        session_id: SESSION_ID,
        clock: &clock_zero,
        verifier: &NoopVerifier,
        commands: &EMPTY_COMMANDS,
        runner: &NoopRunner,
        generation_params: GenerationParams {
            temperature: None,
            seed: None,
        },
        telemetry_dir: None,
        progress: None,
        context_window: None,
        governor: GovernorConfig::default(),
        task_tracking: true,
        gate_retries: u32::MAX,
        wall_clock_secs: 0,
        cancel: signal,
    };
    let result = execute_phase(&input(), deps).await.unwrap();
    assert_eq!(result.status, PhaseStatus::Cancelled);
    let c = result.cancellation.as_ref().unwrap();
    assert_eq!(c.stage, "awaiting_model");
}
