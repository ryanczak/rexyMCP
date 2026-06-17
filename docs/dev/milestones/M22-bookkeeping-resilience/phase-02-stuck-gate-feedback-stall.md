# Phase 02: Stuck gate-feedback stall

**Milestone:** M22 — Bookkeeping-Loop Resilience
**Status:** todo
**Depends on:** phase-01 (both edit the `NoToolCall` arm of `mod.rs`; land 01 first)
**Estimated diff:** ~140 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Terminate a stuck gate-feedback loop. When the **same** gate feedback
(`gate_retry` / `task_coverage_retry` / `bookkeeping_retry`) is re-injected ≥ K
times in a row with no intervening state change, hard_fail with a named signal
instead of looping to the turn cap.

This generalizes the loop-break beyond phase-01: phase-01 fixes the *observed*
147× spiral (which was empty-completion-driven). A3 catches a stuck **non-empty**
completion loop — the model keeps emitting a completion signal, the same gate
keeps failing, and the identical feedback is re-injected unboundedly. Because the
turns carry no tool call, the governor's `IdenticalToolCallRepetition` never sees
them.

## Architecture references

Read before starting:

- `executor/src/agent/mod.rs` — the three sequential gate blocks in the
  `NoToolCall` arm: `gate_failure_feedback` → `task_coverage_feedback` →
  `bookkeeping_feedback`, each logging a `*_retry` progress stage and `continue`ing.
  **If phase-01 has landed**, an empty-completion block now sits above this region;
  locate the gate blocks by the `gate_failure_feedback` anchor text, not a line
  number.
- `executor/src/governor/hard_fail.rs` — `HardFailSignal` + `describe()` + the
  pure checks; A3 adds a sibling variant + pure check (the same shape phase-01
  added `check_empty_completion_stall`).
- `executor/src/config.rs` — `GovernorConfig` + `GovernorConfigOverride`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm `cargo build` and `cargo test` exit 0, and that phase-01 has landed
   (the empty-completion block is present in the `NoToolCall` arm). If phase-01 is
   not yet merged, **file a blocker** — this phase's dependency is unmet.

## Current state

The three gate blocks run sequentially after the gate command set, each as a
self-contained `if let Some(feedback) = …  { log Progress; push feedback; budget
check; continue; }`. The first to return `Some` injects its feedback and loops;
if all three return `None`, the arm proceeds to the true-completion path. The
canonical block (gate-retry) reads:

```rust
if let Some(feedback) = command::gate_failure_feedback(&gates, &command_outputs) {
    log_event(/* … SessionEvent::Progress { stage: "gate_retry", message: feedback.clone() } … */);
    messages.push(user_text(&feedback, turns));
    if turns >= deps.max_turns { /* … budget_exceeded … */ }
    continue;
}
// … task_coverage_feedback block (stage "task_coverage_retry") …
// … bookkeeping_feedback block (stage "bookkeeping_retry") …
// All configured gates passed — this is a true completion.
```

These three feedback functions are **pure** (`command.rs`): `gate_failure_feedback`
builds from `&gates`/`&command_outputs`; `task_coverage_feedback` from
`&seeded`/`&task_states`; `bookkeeping_feedback` re-reads the phase doc from disk.
Calling any of them twice in a turn is cheap and side-effect-free.

Note: `gate_failure_feedback`'s string embeds the command output, so it varies
between runs whose output differs; `task_coverage_feedback` and
`bookkeeping_feedback` are deterministic given state. The stall therefore fires
**only** on a truly byte-identical re-injection — exactly the 143× case — and
will not false-positive on a gate whose output text shifts between attempts. This
conservatism is intended.

## Spec

### Task 1 — Add the `StuckGateFeedback` signal + pure check (`hard_fail.rs`)

Variant (after `EmptyCompletionStall` from phase-01):

```rust
StuckGateFeedback {
    consecutive_count: u32,
},
```

`describe()` arm:

```rust
Self::StuckGateFeedback { consecutive_count } => {
    format!("the same gate feedback was re-injected {consecutive_count} times with no progress")
}
```

Pure check (sibling, not wired into `evaluate`):

```rust
/// Stuck gate-feedback stall: the loop re-injected byte-identical gate feedback
/// (gate-retry / task-coverage / bookkeeping) `consecutive_repeats` times in a row
/// with no intervening state change.
pub fn check_repeated_gate_feedback(
    consecutive_repeats: usize,
    threshold: usize,
) -> Option<HardFailSignal> {
    if consecutive_repeats >= threshold {
        Some(HardFailSignal::StuckGateFeedback {
            consecutive_count: threshold as u32,
        })
    } else {
        None
    }
}
```

### Task 2 — Add `gate_feedback_repeat_threshold` to `GovernorConfig` (`config.rs`)

`pub gate_feedback_repeat_threshold: usize,`, default **5** (a few real fix
attempts are normal; five byte-identical re-injections are a stall). Mirror into
`GovernorConfigOverride` + apply line, like the other thresholds.

### Task 3 — Additive peek-guard above the three gate blocks (`mod.rs`)

**Declare counters** alongside the governor feedback state (near phase-01's
`consecutive_empty_completions`):

```rust
let mut last_gate_feedback: Option<String> = None;
let mut consecutive_gate_repeats: usize = 0;
```

**Insert a new guard block immediately before the existing `gate_failure_feedback`
block** (do **not** modify the three existing blocks). It peeks at whichever gate
feedback will fire this turn — using the same precedence the blocks use — and
tracks repeats:

```rust
// A3 (M22): stuck-gate-feedback stall. Peek at the gate feedback that will fire
// this turn (same precedence as the blocks below); if it is byte-identical to the
// last one and has repeated past the threshold, the loop is stuck — hard_fail
// instead of re-injecting forever. The three blocks below re-evaluate these pure
// fns and inject as before.
let pending_gate_feedback = command::gate_failure_feedback(&gates, &command_outputs)
    .or_else(|| command::task_coverage_feedback(&seeded, &task_states))
    .or_else(|| command::bookkeeping_feedback(std::path::Path::new(&input.phase_doc_path)));
match &pending_gate_feedback {
    Some(fb) => {
        if last_gate_feedback.as_deref() == Some(fb.as_str()) {
            consecutive_gate_repeats += 1;
        } else {
            consecutive_gate_repeats = 1;
            last_gate_feedback = Some(fb.clone());
        }
        if let Some(signal) = crate::governor::hard_fail::check_repeated_gate_feedback(
            consecutive_gate_repeats,
            deps.governor.gate_feedback_repeat_threshold,
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
            emit_phase_run(
                &deps, input, "hard_fail", Gates::default(), &metrics, &scorer, turns,
            );
            let artifacts = build_artifacts(
                &pre_edit_content,
                deps.project_root,
                log_path.clone(),
                "hard_fail",
                turns,
                CommandOutputs::default(),
            );
            return Ok(hard_fail_result(
                input,
                &recent_tool_calls,
                deps.project_root,
                last_author_diagnostics.clone(),
                signal,
                artifacts,
            ));
        }
    }
    None => {
        consecutive_gate_repeats = 0;
        last_gate_feedback = None;
    }
}
```

The three existing gate blocks then run unchanged. (The double-evaluation of the
pure feedback fns — once here, once in the block that fires — is intentional and
cheap; `bookkeeping_feedback`'s extra phase-doc read happens only on completion
attempts, not every turn.)

### Task 4 — Unit tests for `check_repeated_gate_feedback` (`hard_fail.rs`)

- `repeated_gate_feedback_fires_at_threshold` — `check_repeated_gate_feedback(5, 5)`
  returns `Some(StuckGateFeedback { consecutive_count: 5 })`.
- `repeated_gate_feedback_silent_below_threshold` —
  `check_repeated_gate_feedback(4, 5)` returns `None`.
- `describe_stuck_gate_feedback` — the `describe()` string contains
  `"re-injected"` and the count.

### Task 5 — Integration test in `executor/src/agent/tests.rs`

- `stuck_task_coverage_feedback_hard_fails` — seed a task (reuse the M21
  `update_task` registry pattern from `task_coverage_check_loops_until_all_tasks_done`),
  script the client to return a completion signal (`token("All done.")`) on
  **every** turn and **never** call `update_task`, with `max_turns` above the
  threshold and a `NoopRunner` (gates always pass, so `task_coverage_feedback`
  fires identically each turn). Assert `result.status == PhaseStatus::HardFail`
  and the model was called exactly `gate_feedback_repeat_threshold` times (the
  stall fires at the threshold, not the turn cap).

**Pinned negatives:**

- The existing `task_coverage_check_loops_until_all_tasks_done` (3-turn:
  premature → retry → `update_task` → complete) must **pass unmodified** — when
  the model *does* make progress, the feedback changes (the task list shrinks) so
  the repeat counter resets and the stall never fires.
- `gate_failure_loops_until_gates_pass` must pass unmodified — a gate that fails
  then passes is two *different* feedback states (then `None`), so no stall.

## Acceptance criteria

- [ ] `repeated_gate_feedback_fires_at_threshold` passes.
- [ ] `repeated_gate_feedback_silent_below_threshold` passes.
- [ ] `describe_stuck_gate_feedback` passes.
- [ ] `stuck_task_coverage_feedback_hard_fails` passes (hard_fail at threshold).
- [ ] `task_coverage_check_loops_until_all_tasks_done` and
      `gate_failure_loops_until_gates_pass` pass **unmodified**.
- [ ] All pre-existing tests pass unmodified.
- [ ] `cargo fmt --all --check`, `cargo build`, `cargo clippy`, `cargo test` exit 0.

## Test plan

- Three `check_repeated_gate_feedback` unit tests in `hard_fail.rs`.
- One `execute_phase` integration test asserting the stuck-coverage loop
  hard-fails at the threshold, plus the two unmodified progress-path tests as the
  pinned negatives.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. The integration test
exercises the stuck-feedback path end-to-end against `MockAiClientScript`.

## Authorizations

None. Additive config field, no new dependency, no `Cargo.toml`/`architecture.md`
edit.

## Out of scope

- Refactoring the three gate blocks into one — this phase is deliberately additive
  (the peek-guard sits above untouched blocks) to keep the M19/M21 path intact.
- Per-stage repeat thresholds — one shared threshold across all three gate stages.
- Enriching the hard-fail briefing with gate state — useful, but a separate
  concern (the README flags it under "E. Better briefings"); not in this phase.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
