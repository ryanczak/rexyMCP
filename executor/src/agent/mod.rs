//! The core `execute_phase` turn loop (turn-cycle steps 1–5). Drives the local
//! model through budget-bounded turns — chat → drain events → native-or-parsed
//! `ToolCall` → dispatch → score — terminating `complete` (model stops calling
//! tools) or `budget_exceeded` (turn/context exhaustion). Session log, verifier
//! retry, hard-fail detection, and completion artifacts are later sub-phases.

pub mod prompt;

use std::collections::VecDeque;
use std::path::Path;

use tokio::sync::mpsc;

use crate::ai::AiClient;
use crate::ai::next_tool_id;
use crate::ai::types::{
    AiEvent, Message, ToolCall as AiToolCall, ToolResult as AiToolResult, ToolSchema,
};
use crate::context::budget::Budget;
use crate::context::compactor::compact;
use crate::error::{Error, Result};
use crate::governor::hard_fail::ToolCallSnapshot;
use crate::governor::scorer::Scorer;
use crate::parser::{Origin, ParseResult, ToolCall, parse};
use crate::phase::{
    Artifacts, Blocker, Briefing, CommandOutputs, PhaseResult, collect_working_files,
    summarize_attempts,
};
use crate::tools::ToolRegistry;

/// The prompt inputs and verbatim phase metadata the loop assembles into the
/// system prompt and the escalation briefing.
pub struct PhaseInput {
    pub executor_contract: String,
    pub standards: String,
    pub phase_doc: String,
    pub goal: String,
    pub acceptance_criteria: String,
}

/// The injected dependencies the loop drives — explicit, no globals, no real
/// clock (the loop here reads no time).
pub struct LoopDeps<'a> {
    pub client: &'a dyn AiClient,
    pub registry: &'a ToolRegistry,
    pub tools: &'a [ToolSchema],
    pub budget: &'a Budget,
    pub max_turns: usize,
    pub project_root: &'a Path,
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

    loop {
        // Step 2 — budget: compact on overflow, give up if still over.
        if deps.budget.would_overflow(&system, &messages) {
            compact(&mut messages, deps.budget, &system);
            if deps.budget.would_overflow(&system, &messages) {
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
                AiEvent::Error(e) => return Err(Error::Backend(e)),
            }
        }
        turns += 1;

        // Step 4 — turn the output into a ToolCall (native event wins; otherwise
        // run the forgiving text parser).
        let tool_call = if let Some(tc) = native_call {
            tc
        } else {
            match parse(&completion, deps.registry) {
                ParseResult::NoToolCall => return Ok(complete_result(turns)),
                ParseResult::Found(tc) => tc,
                ParseResult::Failed(failure) => {
                    messages.push(assistant_text(&completion, turns));
                    messages.push(user_text(&failure.feedback, turns));
                    if turns >= deps.max_turns {
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

        // Step 5 — dispatch (native and text share this path) and record.
        let (succeeded, content) = dispatch(deps.registry, &tool_call).await;
        scorer.record(&tool_call.name, succeeded);
        recent_tool_calls.push_back(ToolCallSnapshot {
            tool: tool_call.name.clone(),
            arguments: tool_call.arguments.clone(),
            succeeded,
        });
        append_tool_exchange(&mut messages, &tool_call, &content, turns);

        // Step 6 — turn cap.
        if turns >= deps.max_turns {
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

    fn input() -> PhaseInput {
        PhaseInput {
            executor_contract: "CONTRACT".to_string(),
            standards: "STANDARDS".to_string(),
            phase_doc: "PHASE".to_string(),
            goal: "make it compile".to_string(),
            acceptance_criteria: "cargo build passes".to_string(),
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
        }
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
}
