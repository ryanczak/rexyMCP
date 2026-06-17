# Phase 01: Task-coverage gate in the NoToolCall completion arm

**Milestone:** M21 — Task Coverage Gate
**Status:** todo
**Depends on:** none (M19/M20 done; extends the M19 gate-retry arm)
**Estimated diff:** ~120 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Add a task-coverage check to the `NoToolCall` completion arm of
`execute_phase`, symmetric with M19's gate-retry loop. When tasks are seeded
and any remain incomplete at `NoToolCall` time, inject a named-task list and
loop — preventing `false_completion` on phases where all four gates pass by
construction (docs phases, config-only phases, any phase with no code to fail
fmt/build/lint/test).

## Architecture references

Read before starting:

- `executor/src/agent/mod.rs` — the `NoToolCall` arm (lines 564–639) and the
  gate-retry block (lines 578–627) that this phase extends.
- `executor/src/agent/command.rs` — `gate_failure_feedback` (lines 106–140),
  the model for the new `task_coverage_feedback` helper.
- `executor/src/agent/tasks.rs` — `Task` struct (lines 5–10), `TaskState` via
  `crate::store::sessions::event::TaskState`.
- `executor/src/agent/tests.rs` — `gate_failure_loops_until_gates_pass`
  (lines 4027–4047) and `gate_failure_at_turn_cap_is_budget_exceeded`
  (lines 4049–4066) for the exact test shape to replicate for task coverage.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm `cargo build` and `cargo test` exit 0 (no pre-existing failures).

## Current state

### `executor/src/agent/command.rs` (the helper file)

`gate_failure_feedback` at lines 106–140:

```rust
pub(super) fn gate_failure_feedback(gates: &Gates, outputs: &CommandOutputs) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();
    if gates.fmt == Some(false) {
        sections.push(format!("FORMAT failed:\n{}", outputs.format.as_deref().unwrap_or("(no output captured)")));
    }
    // … build / lint / test …
    if sections.is_empty() { return None; }
    Some(format!(
        "Pre-completion gate check failed — the phase is not done yet. \
         Fix the issues below, then re-emit your completion signal.\n\n{}",
        sections.join("\n\n")
    ))
}
```

Existing imports in `command.rs` (lines 59–63):

```rust
use crate::config::CommandConfig;
use crate::phase::CommandOutputs;
use crate::store::telemetry::Gates;
use super::progress::{EmitCtx, emit_progress};
```

`HashMap` and `TaskState`/`Task` are **not** yet imported here.

### `executor/src/agent/mod.rs` (the loop)

**`seeded` computation (lines 131–136):**

```rust
pub async fn execute_phase(input: &PhaseInput, deps: LoopDeps<'_>) -> Result<PhaseResult> {
    let seeded: Vec<crate::agent::tasks::Task> = if deps.task_tracking {
        tasks::seed_from_spec(&input.phase_doc)
    } else {
        Vec::new()
    };
```

`HashMap` is already imported at line 21:
```rust
use std::collections::{HashMap, HashSet, VecDeque};
```

**Task-metadata block (lines 809–830)** — where the loop already processes
`update_task` results and emits `SessionEvent::TaskUpdate`. `TaskState` is
accessed here via its full path:
```rust
if let Some(meta) = &tool_meta
    && let Some(tu) = meta.get("task_update")
    && let (Some(id), Some(title)) = (
        tu.get("id").and_then(|v| v.as_str()),
        tu.get("title").and_then(|v| v.as_str()),
    )
    && let Some(state) = tu.get("state").and_then(|v| {
        serde_json::from_value::<crate::store::sessions::event::TaskState>(v.clone()).ok()
    })
{
    log_event(&log_handle, &redactor, deps.clock, turns,
        SessionEvent::TaskUpdate { id: id.to_string(), title: title.to_string(), state });
}
```

Note: `TaskState` is `Copy` (see `event.rs` derive), so `state` is still
available after the `SessionEvent::TaskUpdate { ..., state }` move.

**Gate-retry block (lines 578–627)** — the shape this phase replicates, after
which we insert the task-coverage check:

```rust
// (line 578)
if let Some(feedback) = command::gate_failure_feedback(&gates, &command_outputs) {
    log_event(&log_handle, &redactor, deps.clock, turns,
        SessionEvent::Progress {
            turn: turns,
            stage: "gate_retry".to_string(),
            files_changed: vec![],
            message: feedback.clone(),
        });
    messages.push(user_text(&feedback, turns));
    if turns >= deps.max_turns {
        log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
        emit_phase_run(&deps, input, "budget_exceeded", Gates::default(), &metrics, &scorer, turns);
        let artifacts = build_artifacts(&pre_edit_content, deps.project_root,
            log_path.clone(), "budget_exceeded", turns, CommandOutputs::default());
        return Ok(budget_exceeded_result(input, &recent_tool_calls,
            deps.project_root, turns_line(deps.max_turns), artifacts));
    }
    continue;
}
// (line 628) "All configured gates passed — this is a true completion."
```

### `executor/src/agent/tests.rs` — gate-retry test shape (lines 4027–4066)

```rust
#[tokio::test]
async fn gate_failure_loops_until_gates_pass() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let client = MockAiClientScript::new(vec![vec![token("All done.")], vec![token("All done.")]]);
    let budget = Budget::new(1_000_000);
    let commands = all_commands_configured();
    let runner = ScriptedCommandRunner::new(vec![false, false, false, false, true, true, true, true]);
    let mut d = deps(&client, &registry, &budget, 8, dir.path());
    d.commands = &commands;
    d.runner = &runner;
    let result = execute_phase(&input(), d).await.unwrap();
    assert_eq!(result.status, PhaseStatus::Complete);
    assert_eq!(client.calls().len(), 2); // first attempt triggered gate-retry
}

#[tokio::test]
async fn gate_failure_at_turn_cap_is_budget_exceeded() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let client = MockAiClientScript::new(vec![vec![token("All done.")]]);
    let budget = Budget::new(1_000_000);
    let commands = all_commands_configured();
    let runner = ScriptedCommandRunner::new(vec![false, false, false, false]);
    let mut d = deps(&client, &registry, &budget, 1, dir.path());
    d.commands = &commands;
    d.runner = &runner;
    let result = execute_phase(&input(), d).await.unwrap();
    assert_eq!(result.status, PhaseStatus::BudgetExceeded);
}
```

## Spec

### Task 1 — Add `task_coverage_feedback` to `executor/src/agent/command.rs`

**New imports** — add after the existing `use super::progress::{EmitCtx, emit_progress};`
at line 63:

```rust
use crate::store::sessions::event::TaskState;
use std::collections::HashMap;
```

**New helper** — insert after `gate_failure_feedback` (after line 140, before
the private `run_one` function at line 155):

```rust
/// Task-coverage gate symmetric with `gate_failure_feedback`. Returns `Some(msg)`
/// when `seeded` is non-empty and any task's current state is not `Done`; `None`
/// otherwise. The `states` map is kept in sync by the loop as `update_task` calls
/// land — an absent key means the task was never touched (treated as Pending).
pub(super) fn task_coverage_feedback(
    seeded: &[super::tasks::Task],
    states: &HashMap<String, TaskState>,
) -> Option<String> {
    if seeded.is_empty() {
        return None;
    }
    let incomplete: Vec<&super::tasks::Task> = seeded
        .iter()
        .filter(|t| states.get(&t.id) != Some(&TaskState::Done))
        .collect();
    if incomplete.is_empty() {
        return None;
    }
    let list = incomplete
        .iter()
        .map(|t| {
            let label = match states.get(&t.id) {
                Some(TaskState::Active) => "active",
                _ => "pending",
            };
            format!("  Task {} ({}): {}", t.id, t.title, label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "Pre-completion task check: the following spec tasks are not yet marked done:\n{}\n\n\
         Call update_task(id, state=\"done\") for each completed task, \
         then re-signal completion.",
        list
    ))
}
```

### Task 2 — Maintain a task-states shadow map in `execute_phase` (`mod.rs`)

**After line 136** (after `let seeded: Vec<...> = …;`), insert:

```rust
// Task-coverage shadow: tracks live state as update_task calls land.
// All Pending at start; updated in the tool-result block below.
// Only consulted when `task_tracking && !seeded.is_empty()`.
let mut task_states: HashMap<String, crate::store::sessions::event::TaskState> =
    seeded.iter().map(|t| (t.id.clone(), t.state.clone())).collect();
```

**Inside the task-metadata `if let` block** (lines 809–830), add one line
after the `log_event(...)` call and before the closing `}`, to keep the shadow
in sync:

```rust
task_states.insert(id.to_string(), state);
```

`state` is `Copy` so this works after the `SessionEvent::TaskUpdate { ..., state }` use above it.

### Task 3 — Add the task-coverage check in the `NoToolCall` arm (`mod.rs`)

Insert after the closing `}` of the gate-retry block (after line 627, before
the `// All configured gates passed` comment at line 628):

```rust
// Task-coverage gate: if tasks were seeded and any are still incomplete,
// inject named feedback and loop — symmetric with the gate-retry above.
if let Some(feedback) = command::task_coverage_feedback(&seeded, &task_states) {
    log_event(
        &log_handle,
        &redactor,
        deps.clock,
        turns,
        SessionEvent::Progress {
            turn: turns,
            stage: "task_coverage_retry".to_string(),
            files_changed: vec![],
            message: feedback.clone(),
        },
    );
    messages.push(user_text(&feedback, turns));
    if turns >= deps.max_turns {
        log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
        emit_phase_run(
            &deps, input, "budget_exceeded", Gates::default(), &metrics, &scorer, turns,
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
```

### Task 4 — Unit tests for `task_coverage_feedback` in `command.rs`

Add to the `#[cfg(test)] mod tests` block at the bottom of `command.rs`:

```rust
use crate::agent::tasks::Task;
use crate::store::sessions::event::TaskState;
use std::collections::HashMap;

fn task(id: &str, title: &str) -> Task {
    Task { id: id.to_string(), title: title.to_string(), state: TaskState::Pending }
}

#[test]
fn task_coverage_feedback_returns_none_when_no_tasks_seeded() {
    assert!(task_coverage_feedback(&[], &HashMap::new()).is_none());
}

#[test]
fn task_coverage_feedback_returns_none_when_all_tasks_done() {
    let seeded = vec![task("1", "Foo"), task("2", "Bar")];
    let states = HashMap::from([
        ("1".to_string(), TaskState::Done),
        ("2".to_string(), TaskState::Done),
    ]);
    assert!(task_coverage_feedback(&seeded, &states).is_none());
}

#[test]
fn task_coverage_feedback_lists_pending_task_by_id_and_title() {
    let seeded = vec![task("1", "Update the status header")];
    let states = HashMap::new(); // absent = pending
    let msg = task_coverage_feedback(&seeded, &states).expect("should be Some");
    assert!(
        msg.contains("Task 1 (Update the status header): pending"),
        "expected pending task listing, got: {msg}"
    );
}

#[test]
fn task_coverage_feedback_labels_active_task() {
    let seeded = vec![task("3", "Wire the config")];
    let states = HashMap::from([("3".to_string(), TaskState::Active)]);
    let msg = task_coverage_feedback(&seeded, &states).expect("should be Some");
    assert!(
        msg.contains("Task 3 (Wire the config): active"),
        "expected active label, got: {msg}"
    );
}

#[test]
fn task_coverage_feedback_omits_done_tasks_from_list() {
    let seeded = vec![task("1", "Done task"), task("2", "Pending task")];
    let states = HashMap::from([("1".to_string(), TaskState::Done)]);
    let msg = task_coverage_feedback(&seeded, &states).expect("should be Some");
    assert!(!msg.contains("Done task"), "done task must not appear: {msg}");
    assert!(msg.contains("Pending task"), "pending task must appear: {msg}");
}
```

### Task 5 — Integration tests in `executor/src/agent/tests.rs`

Add two tests at the end of the file (after `gate_failure_at_turn_cap_is_budget_exceeded`).

They follow the exact shape of the gate-retry tests but use a phase_doc with a
seeded task and a registry that includes `update_task`. To keep the registry
simple, build it inline.

```rust
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
```

**Pinned negatives:**

- `task_coverage_feedback_returns_none_when_no_tasks_seeded` — the check must
  be a no-op when `seeded.is_empty()`. This covers all pre-existing tests
  where `input().phase_doc = "PHASE"` seeds 0 tasks; **those tests must
  continue to pass unmodified**.
- `task_coverage_feedback_omits_done_tasks_from_list` — a partially-done list
  must only name the incomplete tasks.
- `task_coverage_check_at_turn_cap_is_budget_exceeded` — at the turn cap with
  incomplete tasks, must return `BudgetExceeded`, not `Complete`.

## Acceptance criteria

- [ ] `task_coverage_feedback_returns_none_when_no_tasks_seeded` passes.
- [ ] `task_coverage_feedback_returns_none_when_all_tasks_done` passes.
- [ ] `task_coverage_feedback_lists_pending_task_by_id_and_title` passes.
- [ ] `task_coverage_feedback_labels_active_task` passes.
- [ ] `task_coverage_feedback_omits_done_tasks_from_list` passes.
- [ ] `task_coverage_check_loops_until_all_tasks_done` passes.
- [ ] `task_coverage_check_at_turn_cap_is_budget_exceeded` passes.
- [ ] All pre-existing tests pass unmodified (`cargo test` passes at 807+).
- [ ] `cargo fmt --all --check`, `cargo build`, `cargo clippy` all exit 0.

## Test plan

- `task_coverage_feedback_returns_none_when_no_tasks_seeded` in `command.rs`
  — asserts `None` when `seeded` is empty; the critical backward-compat pin.
- `task_coverage_feedback_returns_none_when_all_tasks_done` in `command.rs`
  — asserts `None` when all tasks are `Done`.
- `task_coverage_feedback_lists_pending_task_by_id_and_title` in `command.rs`
  — asserts the message contains `"Task 1 (Update the status header): pending"`.
- `task_coverage_feedback_labels_active_task` in `command.rs` — asserts
  `"active"` label for a task in `Active` state.
- `task_coverage_feedback_omits_done_tasks_from_list` in `command.rs` —
  asserts done tasks don't appear, pending task does.
- `task_coverage_check_loops_until_all_tasks_done` in `tests.rs` — 3-turn
  sequence: premature complete → coverage retry → update_task → true complete;
  asserts `PhaseStatus::Complete` + 3 model calls.
- `task_coverage_check_at_turn_cap_is_budget_exceeded` in `tests.rs` — 1-turn
  cap with incomplete tasks; asserts `PhaseStatus::BudgetExceeded`.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. The `execute_phase`
function is the library entry point; the two integration tests exercise it end-
to-end against a `TempDir` and `MockAiClientScript`.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `architecture.md` edit.

## Out of scope

- Changing `LoopDeps` struct — no new field, no blast radius.
- Changing the `update_task` tool API or `UpdateTaskTool` internals.
- Changing `build_registry` in `mcp/src/runner.rs`.
- Populating `TierTelemetry.escalation_count` or other telemetry fields — this
  phase wires no telemetry beyond what already exists.
- A `WORKFLOW.md` fold for the no-gate-coverage false_completion class — that
  fold needs user sign-off; it is tracked in the M21 retrospective.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
