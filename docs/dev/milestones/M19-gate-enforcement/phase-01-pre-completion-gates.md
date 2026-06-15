# Phase 01: Pre-completion gate enforcement

**Milestone:** M19 — Structural Gate Enforcement
**Status:** todo
**Depends on:** none
**Estimated diff:** ~130 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

When the executor emits a completion signal (no tool call), the runtime already
runs the DoD gate set and records the results — but it does not inspect them
before returning `complete`. This phase closes that gap: if any gate exits
non-zero, the runtime injects the failure output back into the conversation and
loops. The model must fix the issue and re-complete. `false_completion` becomes
structurally impossible through a red gate.

## Architecture references

- `executor/src/agent/mod.rs` — the `execute_phase` turn loop; the
  `ParseResult::NoToolCall` arm is the sole completion path.
- `executor/src/agent/command.rs` — `run_command_set`, `CommandRunner` trait,
  `CommandResult`. The `Gates` and `CommandOutputs` types are already imported
  here (`command.rs:59–61`).
- `executor/src/store/telemetry.rs:27–32` — `Gates` (what `run_command_set`
  already returns).
- `executor/src/phase/result.rs:27–32` — `CommandOutputs`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo build` to confirm the tree is clean before starting.

## Current state

### The completion path (`executor/src/agent/mod.rs:561–583`)

The `ParseResult::NoToolCall` arm has two sub-paths. The first handles
a completion that is *only* a `<think>` block (empty after stripping) — it
`continue`s the loop. The second is the **real** completion path, which this
phase modifies.

The real completion path today (verbatim, lines 561–583):

```rust
log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
// Step 8 — clean completion runs the final command set.
let emit = EmitCtx {
    progress: deps.progress,
    log_handle: &log_handle,
    redactor: &redactor,
    clock: deps.clock,
    pre_edit_content: &pre_edit_content,
    project_root: deps.project_root,
    turn: turns,
};
let (command_outputs, gates) =
    run_command_set(deps.runner, deps.commands, deps.project_root, &emit).await;
emit_phase_run(&deps, input, "complete", gates, &metrics, &scorer, turns);
let artifacts = build_artifacts(
    &pre_edit_content,
    deps.project_root,
    log_path.clone(),
    "complete",
    turns,
    command_outputs,
);
return Ok(PhaseResult::complete(artifacts));
```

**The problem:** `log_session_end("complete")` is on **line 561**, before
`run_command_set` runs. If gates fail and we continue the loop, the session log
already contains a "complete" end record for a phase that did not complete. The
fix moves `log_session_end` to after the gate check.

### `Gates` struct (`executor/src/store/telemetry.rs:27–32`)

```rust
pub struct Gates {
    pub fmt: Option<bool>,
    pub build: Option<bool>,
    pub lint: Option<bool>,
    pub test: Option<bool>,
}
```

`Some(false)` = ran and failed. `Some(true)` = ran and passed. `None` =
command not configured. **Only `Some(false)` is a gate failure.** `None` is
not a failure — treat an unconfigured command as absent, not broken. This
keeps `EMPTY_COMMANDS` / `NoopRunner` tests backward-compatible with no change
in behavior.

### `CommandOutputs` struct (`executor/src/phase/result.rs:27–32`)

```rust
pub struct CommandOutputs {
    pub format: Option<String>,
    pub build: Option<String>,
    pub lint: Option<String>,
    pub test: Option<String>,
}
```

Carries the tail of each command's combined stdout+stderr (already capped at
`MAX_COMMAND_TAIL_CHARS` by `run_command_set`). Include these in the feedback
message so the model sees the actual failure output, not just the gate name.

### Imports: nothing new in either file

- `mod.rs:42` already imports `use crate::store::telemetry::{Gates, GenerationParams};`
- `command.rs:59–61` already imports `CommandConfig`, `CommandOutputs`, `Gates`

Do not add any import lines.

### How feedback enters the conversation (`executor/src/agent/tools.rs:351–358`)

```rust
pub(super) fn user_text(content: &str, turn: usize) -> Message {
    Message {
        role: "user".to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_results: None,
        turn: Some(turn),
    }
}
```

Push the gate-failure feedback via `messages.push(user_text(&feedback, turns))`.
This is the same mechanism already used for parse-failure feedback and verifier
diagnostics — no new infrastructure needed.

### Existing test infrastructure (`executor/src/agent/tests.rs:59–108`)

The existing `NoopRunner` always returns `success: true` and is used with
`EMPTY_COMMANDS` (all fields `None`), so `run_command_set` never calls it.
The new integration tests need configurable per-call outcomes — see the
`ScriptedCommandRunner` in Task 4.

## Spec

### 1. Add `gate_failure_feedback` to `executor/src/agent/command.rs`

Add this `pub(super)` function after `run_command_set` (before `run_post_write_hooks`):

```rust
/// Returns a user-facing feedback message when any configured gate exited
/// non-zero, or `None` when all configured gates passed (or no commands are
/// configured). Unconfigured gates (`None`) are not failures.
pub(super) fn gate_failure_feedback(
    gates: &Gates,
    outputs: &CommandOutputs,
) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();
    if gates.fmt == Some(false) {
        sections.push(format!(
            "FORMAT failed:\n{}",
            outputs.format.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.build == Some(false) {
        sections.push(format!(
            "BUILD failed:\n{}",
            outputs.build.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.lint == Some(false) {
        sections.push(format!(
            "LINT failed:\n{}",
            outputs.lint.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.test == Some(false) {
        sections.push(format!(
            "TEST failed:\n{}",
            outputs.test.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if sections.is_empty() {
        return None;
    }
    Some(format!(
        "Pre-completion gate check failed — the phase is not done yet. \
         Fix the issues below, then re-emit your completion signal.\n\n{}",
        sections.join("\n\n")
    ))
}
```

No new imports — `Gates`, `CommandOutputs` are already in scope in `command.rs`.

### 2. Restructure the completion path in `executor/src/agent/mod.rs`

Replace the block at lines 561–583 (the real-completion sub-path inside
`ParseResult::NoToolCall`, after the empty-think early return) with:

```rust
// Step 8 — run the final gate set BEFORE declaring completion. If any
// gate fails, inject the failure output and continue so the model must
// fix and re-complete. Only log "complete" after all gates pass.
let emit = EmitCtx {
    progress: deps.progress,
    log_handle: &log_handle,
    redactor: &redactor,
    clock: deps.clock,
    pre_edit_content: &pre_edit_content,
    project_root: deps.project_root,
    turn: turns,
};
let (command_outputs, gates) =
    run_command_set(deps.runner, deps.commands, deps.project_root, &emit).await;
if let Some(feedback) = command::gate_failure_feedback(&gates, &command_outputs) {
    log_event(
        &log_handle,
        &redactor,
        deps.clock,
        turns,
        SessionEvent::Progress {
            turn: turns,
            stage: "gate_retry".to_string(),
            files_changed: vec![],
            message: feedback.clone(),
        },
    );
    messages.push(user_text(&feedback, turns));
    if turns >= deps.max_turns {
        log_session_end(
            &log_handle,
            &redactor,
            deps.clock,
            "budget_exceeded",
            turns,
        );
        emit_phase_run(
            &deps,
            input,
            "budget_exceeded",
            Gates::default(),
            &metrics,
            &scorer,
            turns,
        );
        let artifacts = build_artifacts(
            &pre_edit_content,
            deps.project_root,
            log_path.clone(),
            "budget_exceeded",
            turns,
            CommandOutputs::default(),
        );
        return Ok(budget_exceeded_result(
            input,
            &recent_tool_calls,
            deps.project_root,
            turns_line(deps.max_turns),
            artifacts,
        ));
    }
    continue;
}
// All configured gates passed — this is a true completion.
log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
emit_phase_run(&deps, input, "complete", gates, &metrics, &scorer, turns);
let artifacts = build_artifacts(
    &pre_edit_content,
    deps.project_root,
    log_path.clone(),
    "complete",
    turns,
    command_outputs,
);
return Ok(PhaseResult::complete(artifacts));
```

**Key differences from the old block:**
- `log_session_end("complete")` moved to after the gate check (was line 561,
  before `run_command_set`).
- `emit_phase_run` is only called on final terminal paths — not on a
  gate-retry `continue`.
- The gate-retry turn-cap arm mirrors the existing `ParseResult::Failed`
  budget-exceeded arm exactly.

**Termination guarantee:** each gate-retry `continue` re-enters the model-call
step (Step 3), which increments `turns`. The existing `max_turns` check at
Step 9 of the loop (and the inline check above) ensures the loop terminates.
No new backstop counter is needed.

### 3. Unit tests in `executor/src/agent/command.rs`

Add a `#[cfg(test)] mod tests` block at the bottom of `command.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::CommandOutputs;
    use crate::store::telemetry::Gates;

    fn outputs_with(format: &str, build: &str, lint: &str, test: &str) -> CommandOutputs {
        CommandOutputs {
            format: Some(format.to_string()),
            build: Some(build.to_string()),
            lint: Some(lint.to_string()),
            test: Some(test.to_string()),
        }
    }

    #[test]
    fn gate_failure_feedback_returns_none_when_all_pass() {
        let gates = Gates {
            fmt: Some(true),
            build: Some(true),
            lint: Some(true),
            test: Some(true),
        };
        assert!(gate_failure_feedback(&gates, &outputs_with("ok", "ok", "ok", "ok")).is_none());
    }

    #[test]
    fn gate_failure_feedback_returns_none_when_no_commands_configured() {
        // Gates::default() is all None — unconfigured commands are not failures.
        assert!(gate_failure_feedback(&Gates::default(), &CommandOutputs::default()).is_none());
    }

    #[test]
    fn gate_failure_feedback_includes_failing_gates_and_omits_passing() {
        let gates = Gates {
            fmt: Some(false),
            build: Some(false),
            lint: Some(true),
            test: Some(false),
        };
        let outputs = outputs_with("fmt diff here", "build errors", "lint ok", "test failed");
        let msg = gate_failure_feedback(&gates, &outputs).expect("should be Some");
        assert!(msg.contains("FORMAT failed"), "missing FORMAT section");
        assert!(msg.contains("fmt diff here"), "missing FORMAT output");
        assert!(msg.contains("BUILD failed"), "missing BUILD section");
        assert!(msg.contains("build errors"), "missing BUILD output");
        assert!(!msg.contains("LINT"), "LINT should not appear (it passed)");
        assert!(msg.contains("TEST failed"), "missing TEST section");
    }
}
```

### 4. Integration tests in `executor/src/agent/tests.rs`

Add a `ScriptedCommandRunner` to the test module (after `NoopRunner`):

```rust
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
```

Add a helper that builds a `CommandConfig` with all four gates configured:

```rust
fn all_commands_configured() -> CommandConfig {
    CommandConfig {
        format: Some("true".to_string()),
        build: Some("true".to_string()),
        lint: Some("true".to_string()),
        test: Some("true".to_string()),
        lint_fix: None,
    }
}
```

**Note on the script size:** each call to `run_command_set` issues one `run`
call per configured command (format, build, lint, test = 4 calls). A script
of `[false, false, false, false, true, true, true, true]` means: first
completion attempt → all 4 gates fail; second completion attempt → all 4
gates pass.

Add two integration tests:

```rust
#[tokio::test]
async fn gate_failure_loops_until_gates_pass() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    // Two "done" completions: first fails gates, second passes.
    let client = MockAiClientScript::new(vec![
        vec![token("All done.")],
        vec![token("All done.")],
    ]);
    let budget = Budget::new(1_000_000);
    let commands = all_commands_configured();
    // 4 failures then 4 passes.
    let runner = ScriptedCommandRunner::new(vec![
        false, false, false, false,
        true, true, true, true,
    ]);
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
```

**Why this tests the right thing:**
- In `gate_failure_loops_until_gates_pass`: if the gate check were absent and
  the loop returned `complete` unconditionally, `client.calls().len()` would
  be 1. The assertion `== 2` is mutation-resistant against removing the
  gate-retry branch.
- In `gate_failure_at_turn_cap_is_budget_exceeded`: if the loop returned
  `complete` despite failing gates, the status assertion would fail.

**Existing tests are unaffected:** they use `EMPTY_COMMANDS` / `NoopRunner`,
where all `Gates` fields are `None`. `gate_failure_feedback` returns `None`
for all-`None` gates, so behavior is byte-identical to before.

## Acceptance criteria

- [ ] `gate_failure_feedback_returns_none_when_all_pass` passes.
- [ ] `gate_failure_feedback_returns_none_when_no_commands_configured` passes.
- [ ] `gate_failure_feedback_includes_failing_gates_and_omits_passing` passes;
      "LINT" does not appear in the message when lint passed.
- [ ] `gate_failure_loops_until_gates_pass` passes; `client.calls().len() == 2`.
- [ ] `gate_failure_at_turn_cap_is_budget_exceeded` passes.
- [ ] `no_tool_call_first_turn_completes_immediately` still passes (no
      regression: `EMPTY_COMMANDS` → gates all `None` → `gate_failure_feedback`
      returns `None` → behavior unchanged).
- [ ] `cargo fmt --all --check` exits 0.
- [ ] `cargo build` exits 0 with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0.
- [ ] `cargo test` passes (count rises above prior baseline — 5 new tests).

## Test plan

- `gate_failure_feedback_returns_none_when_all_pass` in `executor/src/agent/command.rs`
- `gate_failure_feedback_returns_none_when_no_commands_configured` in `executor/src/agent/command.rs`
- `gate_failure_feedback_includes_failing_gates_and_omits_passing` in `executor/src/agent/command.rs`
- `gate_failure_loops_until_gates_pass` in `executor/src/agent/tests.rs`
- `gate_failure_at_turn_cap_is_budget_exceeded` in `executor/src/agent/tests.rs`

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. The change is
internal to the `execute_phase` loop: no CLI, no MCP tool surface, no config
field changes. Behavior is verified by the integration tests, which drive the
real `execute_phase` function with hermetic mocks.

## Authorizations

None.

## Out of scope

- Adding `clippy::unwrap_used` to workspace `clippy.toml` — related but
  separate concern. Do not touch `clippy.toml` or any workspace config.
- Any change to `PhaseResult`, `CommandOutputs`, `Gates`, or `SessionEvent`
  types. No new `SessionEvent` variant — use `Progress { stage: "gate_retry" }`.
- Any change to `rexymcp.toml` format, `[commands]` section, or any MCP / CLI
  surface.
- Any change to the post-write format hook (`run_post_write_hooks`) or the
  per-edit verifier — those are separate mechanisms that already run during the
  turn cycle.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
